//! The offset of freeform surfaces against closed forms: a piece of a
//! sphere and pieces of a torus given as rational NURBS surfaces (exactly
//! those pieces: rational quadratic arcs, *The NURBS Book* §7.3 and §8.5)
//! have as their offsets pieces of the sphere and of the torus with the
//! radius and the tube changed by the distance, point for point in the
//! same parameters. Tilted frames, three scales, both sides; a fold, a
//! distance next to one, and a pole refused.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_geom::offset::{OffsetError, offset, regular};
use craft3d_geom::{NurbsSurface, Surface};
use craft3d_math::{Frame, Vec3};
use std::f64::consts::{FRAC_PI_2, PI};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

/// A rational quadratic arc of the unit circle from `from` to `to`, in
/// pieces of a quarter turn at most: knots, points, weights.
fn arc(from: f64, to: f64) -> (Vec<f64>, Vec<(f64, f64)>, Vec<f64>) {
    let spans = (((to - from) / FRAC_PI_2) - 1e-9).ceil().max(1.0) as usize;
    let step = (to - from) / spans as f64;
    let weight = (0.5 * step).cos();
    let mut knots = vec![0.0; 3];
    let mut points = Vec::new();
    let mut weights = Vec::new();
    for s in 0..spans {
        let a = from + step * s as f64;
        if s == 0 {
            points.push((a.cos(), a.sin()));
            weights.push(1.0);
        }
        let m = a + 0.5 * step;
        points.push((m.cos() / weight, m.sin() / weight));
        weights.push(weight);
        let b = a + step;
        points.push((b.cos(), b.sin()));
        weights.push(1.0);
        if s + 1 < spans {
            knots.extend([(s + 1) as f64 / spans as f64; 2]);
        }
    }
    knots.extend([1.0; 3]);
    (knots, points, weights)
}

/// The surface of revolution about the frame's z of the circle arc in the
/// (ρ, z) half-plane with centre `(rho, 0)` and `radius`, from the angle
/// `low` to `high` across, turned from `from` to `to` about the axis.
fn revolved(
    frame: &Frame,
    rho: f64,
    radius: f64,
    (low, high): (f64, f64),
    (from, to): (f64, f64),
) -> Surface {
    let (knots_u, around, weights_u) = arc(from, to);
    let (knots_v, profile, weights_v) = arc(low, high);
    let mut ctrl = Vec::new();
    let mut weights = Vec::new();
    for (&(cx, cy), wu) in around.iter().zip(&weights_u) {
        for (&(px, pz), wv) in profile.iter().zip(&weights_v) {
            let (pr, z) = (rho + radius * px, radius * pz);
            ctrl.push(frame.point(pr * cx, pr * cy, z));
            weights.push(wu * wv);
        }
    }
    Surface::Nurbs(
        NurbsSurface::new(
            2,
            2,
            knots_u,
            knots_v,
            around.len(),
            profile.len(),
            ctrl,
            weights,
        )
        .unwrap(),
    )
}

fn tilted(k: f64) -> Frame {
    Frame::new(
        Vec3::new(3.0 * k, -1.5 * k, 0.75 * k),
        Vec3::new(0.3, -0.5, 0.8),
        Vec3::new(1.0, 0.4, 0.2),
    )
    .unwrap()
}

/// Every point of `fitted` on an odd grid (not the samples) against
/// `expected`, which gives the exact offset point at `(u, v)`: the
/// largest distance.
fn worst(fitted: &NurbsSurface, expected: impl Fn(f64, f64) -> Vec3) -> f64 {
    let ((u0, u1), (v0, v1)) = fitted.domain();
    let mut worst = 0.0_f64;
    for i in 0..=53 {
        for j in 0..=47 {
            let (u, v) = (
                u0 + (u1 - u0) * f64::from(i) / 53.0,
                v0 + (v1 - v0) * f64::from(j) / 47.0,
            );
            worst = worst.max(fitted.eval(u, v).distance(expected(u, v)));
        }
    }
    worst
}

/// The sign that turns the surface's normal `∂u × ∂v` away from `centre`
/// at the middle of its domain.
fn outward(surface: &Surface, centre: impl Fn(Vec3) -> Vec3) -> f64 {
    let ((u0, u1), (v0, v1)) = surface.domain();
    let (u, v) = (0.5 * (u0 + u1), 0.5 * (v0 + v1));
    let p = surface.eval(u, v);
    if surface.normal(u, v).unwrap().dot(p - centre(p)) > 0.0 {
        1.0
    } else {
        -1.0
    }
}

#[test]
fn the_offset_of_a_piece_of_a_sphere_is_the_piece_of_the_sphere_of_the_radius_moved() {
    for k in SCALES {
        let frame = tilted(k);
        let radius = 2.0 * k;
        let sphere = revolved(&frame, 0.0, radius, (-0.7, 0.9), (0.2, 1.9));
        let centre = frame.origin();
        let sign = outward(&sphere, |_| centre);
        let limit = 0.25e-6 * k;
        for d in [0.6 * k, -0.5 * k, -1.2 * k, 3.0 * k] {
            let fitted =
                offset(&sphere, sign * d, limit).unwrap_or_else(|e| panic!("k {k} d {d}: {e:?}"));
            let off = worst(&fitted, |u, v| {
                let p = sphere.eval(u, v);
                centre + (p - centre) * ((radius + d) / radius)
            });
            assert!(off <= limit, "k {k} d {d}: off by {off}, limit {limit}");
        }
    }
}

#[test]
fn the_offset_of_pieces_of_a_torus_outside_and_inside_is_the_torus_with_its_tube_moved() {
    for k in SCALES {
        let frame = tilted(k);
        let (major, minor) = (3.0 * k, 1.0 * k);
        // The outer side (convex both ways) and the inner side (a saddle).
        for across in [(-1.0, 1.2), (2.0, 4.2)] {
            let torus = revolved(&frame, major, minor, across, (-0.4, 1.3));
            let tube = |p: Vec3| {
                let (x, y, _) = frame.local(p);
                let rho = x.hypot(y);
                frame.point(x / rho * major, y / rho * major, 0.0)
            };
            let sign = outward(&torus, tube);
            let limit = 0.25e-6 * k;
            for d in [0.5 * k, -0.6 * k, 1.5 * k] {
                let fitted = offset(&torus, sign * d, limit)
                    .unwrap_or_else(|e| panic!("k {k} {across:?} d {d}: {e:?}"));
                let off = worst(&fitted, |u, v| {
                    let p = torus.eval(u, v);
                    let c = tube(p);
                    c + (p - c) * ((minor + d) / minor)
                });
                assert!(off <= limit, "k {k} {across:?} d {d}: off by {off}");
            }
        }
    }
}

#[test]
fn an_offset_past_the_radius_of_curvature_folds_and_says_where_and_how_far() {
    for k in SCALES {
        let frame = tilted(k);
        let radius = 2.0 * k;
        let sphere = revolved(&frame, 0.0, radius, (-0.7, 0.9), (0.2, 1.9));
        let centre = frame.origin();
        let inward = -outward(&sphere, |_| centre);
        match regular(&sphere, inward * 2.5 * k) {
            Err(OffsetError::Folds { radius: r, at }) => {
                assert!((r - radius).abs() <= 1e-6 * radius, "k {k}: radius {r}");
                let ((u0, u1), (v0, v1)) = sphere.domain();
                assert!(at.0 >= u0 && at.0 <= u1 && at.1 >= v0 && at.1 <= v1);
            }
            other => panic!("k {k}: {other:?}"),
        }
        // Outward any distance is regular; inward short of the radius too,
        // proven by the bounds.
        regular(&sphere, -inward * 50.0 * k).unwrap();
        regular(&sphere, inward * 1.8 * k).unwrap();
        // The torus folds first across its tube on its inner side, at the
        // tube's radius toward the tube's centre.
        let torus = revolved(&frame, 3.0 * k, 1.0 * k, (2.0, 4.2), (-0.4, 1.3));
        let tube = |p: Vec3| {
            let (x, y, _) = frame.local(p);
            let rho = x.hypot(y);
            frame.point(x / rho * 3.0 * k, y / rho * 3.0 * k, 0.0)
        };
        let inward = -outward(&torus, tube);
        regular(&torus, inward * 0.9 * k).unwrap();
        assert!(matches!(
            regular(&torus, inward * 1.1 * k),
            Err(OffsetError::Folds { .. })
        ));
        assert!(matches!(
            offset(&torus, inward * 1.1 * k, 0.25e-6 * k),
            Err(OffsetError::Folds { .. })
        ));
    }
}

#[test]
fn a_surface_with_a_pole_has_no_offset_there() {
    let frame = tilted(1.0);
    let cap = revolved(&frame, 0.0, 2.0, (0.3, FRAC_PI_2), (0.0, PI));
    assert!(matches!(
        offset(&cap, 0.5, 1e-7),
        Err(OffsetError::NoNormal { .. })
    ));
    // Bad numbers are errors.
    let sphere = revolved(&frame, 0.0, 2.0, (-0.7, 0.9), (0.2, 1.9));
    for (d, limit) in [(f64::NAN, 1e-7), (0.5, 0.0), (0.5, f64::INFINITY)] {
        assert!(matches!(
            offset(&sphere, d, limit),
            Err(OffsetError::Invalid(_))
        ));
    }
}
