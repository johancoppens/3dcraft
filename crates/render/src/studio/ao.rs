//! Ambient occlusion in screen space by horizons: GTAO (Jimenez, Wu,
//! Pesce and Jarabo, *Practical Real-Time Strategies for Accurate
//! Indirect Occlusion*, SIGGRAPH 2016 course). Per sample, a few slices
//! through the view direction; in each, the highest horizon on either
//! side within a radius in model units, clamped to the hemisphere of the
//! normal, and the cosine-weighted visible arc between them in closed
//! form (their equation 10); a sample counts only when it rises above
//! the tangent plane ([`AO_BIAS`]). The mean over the slices, each weighted by
//! the length of the normal projected into it, is the share of the
//! hemisphere's cosine-weighted light that is not blocked. It is divided
//! by the same sum for the slices with nothing in them (the closed form
//! `cos γ + γ sin γ` per slice), so that a flat open surface is exactly 1
//! with any number of slices (the plain mean is 1 only in the limit: 0.999
//! with six slices on a tilted plane).
//!
//! Both renderers run this on their own buffers of normals and view
//! distances, at the samples of the picture, with fixed slice angles and
//! steps (no noise: the result is deterministic).

use craft3d_math::Vec3;
use std::f64::consts::{FRAC_PI_2, PI};

/// Slices per sample and steps per side of a slice.
pub const AO_SLICES: usize = 6;
pub const AO_STEPS: usize = 6;
/// The radius of the search as a share of the radius of the sphere
/// around the mesh.
pub const AO_RADIUS: f64 = 0.12;
/// A sample blocks light only when it rises above the surface's tangent
/// plane by more than this sine (about 3°). The samples are taken at the
/// centres of the samples of the picture, a little off the slice, so on a
/// tilted flat face they would see a horizon a little above the face's
/// own and darken it (0.999 instead of 1 with six slices); a real
/// occluder rises far more.
pub const AO_BIAS: f64 = 0.05;

/// The occlusion of one sample. `point(x, y)` gives the surface point at
/// sample `(x, y)` relative to the eye (`None` for none), `at` is the
/// sample, `p` its point, `n` its unit normal facing the viewer, `v` the
/// unit vector towards the viewer, `right` and `up` the camera's axes,
/// `radius` the search radius in model units and `reach` the same in
/// samples.
#[allow(clippy::too_many_arguments)]
pub fn occlusion(
    point: impl Fn(i64, i64) -> Option<Vec3>,
    at: (i64, i64),
    p: Vec3,
    n: Vec3,
    v: Vec3,
    (right, up): (Vec3, Vec3),
    radius: f64,
    reach: f64,
) -> f64 {
    if !(reach >= 1.0 && radius > 0.0) {
        return 1.0;
    }
    let (mut visible, mut open) = (0.0, 0.0);
    for k in 0..AO_SLICES {
        let phi = PI * (k as f64 + 0.5) / AO_SLICES as f64;
        let (c, s) = (phi.cos(), phi.sin());
        // The slice's direction on the screen (y down) and in space.
        let dir = right * c - up * s;
        let Some(t) = (dir - v * dir.dot(v)).normalized() else {
            continue;
        };
        let projected = v * n.dot(v) + t * n.dot(t);
        let len = projected.length();
        if len <= 1e-9 {
            continue;
        }
        // The side of the normal: +1 at zero (and −0), as the GPU's
        // `select(-1, 1, x >= 0)`.
        let side = if n.dot(t) >= 0.0 { 1.0 } else { -1.0 };
        let gamma = (projected.dot(v) / len).clamp(-1.0, 1.0).acos() * side;
        let mut horizon = [0.0; 2];
        for (side, sign) in [(0, 1.0), (1, -1.0)] {
            let mut highest = -1.0_f64;
            for j in 0..AO_STEPS {
                let step = (j as f64 + 1.0) / AO_STEPS as f64 * reach;
                let x = at.0 as f64 + 0.5 + sign * c * step;
                let y = at.1 as f64 + 0.5 + sign * s * step;
                let Some(q) = point(x.floor() as i64, y.floor() as i64) else {
                    continue;
                };
                let d = q - p;
                let l = d.length();
                if l <= 0.0 || d.dot(n) <= AO_BIAS * l {
                    continue;
                }
                let cos_h = d.dot(v) / l;
                let fall = (1.0 - (l / radius) * (l / radius)).clamp(0.0, 1.0);
                highest = highest.max(-1.0 + (cos_h + 1.0) * fall);
            }
            if let Some(slot) = horizon.get_mut(side) {
                *slot = sign * highest.clamp(-1.0, 1.0).acos();
            }
        }
        let [h2, h1] = horizon;
        let h1 = gamma + (h1 - gamma).max(-FRAC_PI_2);
        let h2 = gamma + (h2 - gamma).min(FRAC_PI_2);
        let (cg, sg) = (gamma.cos(), gamma.sin());
        let arc = |h: f64| 0.25 * (-(2.0 * h - gamma).cos() + cg + 2.0 * h * sg);
        visible += len * (arc(h1) + arc(h2));
        // The same slice with nothing in it: the arcs to the edges of the
        // normal's hemisphere, `cos γ + γ sin γ` in closed form.
        open += len * (cg + gamma * sg);
    }
    if open > 0.0 {
        (visible / open).clamp(0.0, 1.0)
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plane in front of an orthographic camera looking down −z.
    fn plane(normal: Vec3) -> impl Fn(i64, i64) -> Option<Vec3> {
        move |x, y| {
            let (px, py) = (x as f64 + 0.5, -(y as f64 + 0.5));
            // On the plane n·p = n·(0, 0, −10).
            let z = (-10.0 * normal.z - normal.x * px - normal.y * py) / normal.z;
            Some(Vec3::new(px, py, z))
        }
    }

    #[test]
    fn an_open_plane_is_not_occluded() {
        for n in [Vec3::Z, Vec3::new(0.5, 0.2, 1.0), Vec3::new(-0.9, 0.3, 0.4)] {
            let n = n.normalized().unwrap();
            let point = plane(n);
            let p = point(50, 50).unwrap();
            let ao = occlusion(
                &point,
                (50, 50),
                p,
                n,
                Vec3::Z,
                (Vec3::X, Vec3::Y),
                10.0,
                10.0,
            );
            assert!((ao - 1.0).abs() < 1e-6, "{n:?}: {ao}");
        }
    }

    #[test]
    fn the_floor_of_a_narrow_groove_is_occluded() {
        // A floor at z = −10 between walls rising to z = 0 at |x| ≥ 2.
        let point = |x: i64, y: i64| {
            let px = x as f64 + 0.5;
            let z = if px.abs() < 2.0 { -10.0 } else { 0.0 };
            Some(Vec3::new(px, -(y as f64 + 0.5), z))
        };
        let p = point(0, 0).unwrap();
        let ao = occlusion(
            point,
            (0, 0),
            p,
            Vec3::Z,
            Vec3::Z,
            (Vec3::X, Vec3::Y),
            30.0,
            12.0,
        );
        assert!(ao < 0.8, "{ao}");
    }

    /// A floor tilted by 34° towards +x, with a step two units up
    /// (vertically) beyond |x| = 2 on one side. On the side the normal
    /// leans to, the step's top seen from the sample rises into the
    /// normal's hemisphere and blocks cosine-weighted light; on the other
    /// side its horizon, lowered by the distance falloff, stays under the
    /// tangent plane and blocks nothing. A sign error in the normal's angle within
    /// a slice (γ) turns the order round (worked through outside the
    /// code: about 0.84 and 1.0, with the error 1.0 and 0.80).
    #[test]
    fn a_step_on_the_side_the_normal_leans_to_occludes_more() {
        let tilt: f64 = 0.6;
        let n = Vec3::new(tilt.sin(), 0.0, tilt.cos());
        let occluded = |side: f64| {
            let point = move |x: i64, y: i64| {
                let px = x as f64 + 0.5;
                // The floor through (0, 0, −10) with normal n, two units
                // higher beyond |x| = 2 on `side`.
                let floor = -10.0 - tilt.tan() * px;
                let z = if px * side >= 2.0 { floor + 2.0 } else { floor };
                Some(Vec3::new(px, -(y as f64 + 0.5), z))
            };
            let p = point(0, 0).unwrap();
            occlusion(point, (0, 0), p, n, Vec3::Z, (Vec3::X, Vec3::Y), 12.0, 10.0)
        };
        let (towards, away) = (occluded(1.0), occluded(-1.0));
        assert!(towards < away - 0.1, "{towards} {away}");
    }
}
