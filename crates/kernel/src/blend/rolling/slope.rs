//! How a fillet's section changes along its edge, exactly: what the
//! normal of a fillet face is where its radius varies.
//!
//! **Why.** With one radius the fillet face is a piece of the envelope of
//! its balls, and its normal at a point of a section is the ball's radius
//! there: the refinement holds the fitted surface's normal to that
//! ([`super::radial_kink`]). Where the radius changes along the edge the
//! face is the arcs between the balls' feet, which is not their envelope:
//! along its borders its normal is still the faces' (the border lies on
//! the face and the arc leaves it along the face, see below), but between
//! them it leans away from the ball's radius, by the rate at which the
//! radius changes (in the middle of the arc between planes at a right
//! angle, the tangent of the lean is 0.41 times that rate). So the
//! normal has to be had from the surface itself: the derivative of the
//! section along the edge.
//!
//! **The derivative.** With `s` along the edge, the plane across it at
//! `s` through the edge's point `P(s)` with the unit normal `N(s)`, the
//! ball's centre `c(s)` and radius `r(s)`, and per face `k` the foot
//! `f_k`, the face's unit normal `n_k` there out of the body, and `σ`
//! (−1 at a convex edge, the centre in the material; +1 at a concave
//! one), the construction's own equations are
//!
//! ```text
//! (c − f_k) · n_k = σ r        N · (c − P) = 0
//! ```
//!
//! Their derivatives along `s` are linear in `c′` (a foot moves along its
//! face, square to `n_k`, and `c − f_k` is along `n_k`, square to `n_k′`):
//!
//! ```text
//! n_k · c′ = σ r′              N · c′ = |P′| − N′ · (c − P)
//! ```
//!
//! with `N′ = (P″ − N (N · P″)) / |P′|`. Each foot is `f_k = c − σ r n_k`,
//! and the normal turns as the foot moves, `n_k′ = n_u u′ + n_v v′` (the
//! derivatives of the unit normal from the surface's second derivatives),
//! so the foot's own rate in the face's parameters solves
//!
//! ```text
//! (S_u + σ r n_u) u′ + (S_v + σ r n_v) v′ = c′ − σ r′ n_k
//! ```
//!
//! a vector equation in the face's tangent plane (its right side is
//! square to `n_k` by the first equation). It has no solution where the
//! face curves as tightly as the ball under it, where no ball rolls. The
//! unit vectors from the centre to the feet are `ρ_k = −σ n_k`, the
//! section's rational quadratic has the control points `f_1`,
//! `c + r (ρ_1 + ρ_2) / (1 + ρ_1 · ρ_2)` with the weight
//! `√((1 + ρ_1 · ρ_2) / 2)`, and `f_2`, and the derivative of the surface
//! along `s` at a `v` across it follows by the quotient rule.
//!
//! **Tangent to both faces, for any law.** At a border the surface has
//! two tangents: the arc's, square to the ball's radius there, which is
//! the face's normal, and the border's own, `f_k′`, which lies in the
//! face. Where the two are not parallel they span the face's tangent
//! plane. The refinement measures it on the fitted surface, as for every
//! fillet.

use super::{Section, Solver, radial_kink};
use craft3d_geom::NurbsSurface;
use craft3d_geom::curvature::{curve_second_derivative, second_derivatives};
use craft3d_math::Vec3;

/// A fillet's section with its rate of change along the edge: the centre,
/// the radius, the feet and the unit vectors from the centre to the feet,
/// each with its derivative by `s`.
#[derive(Clone, Copy, Debug)]
pub(super) struct Slope {
    pub centre: (Vec3, Vec3),
    pub radius: (f64, f64),
    pub feet: [(Vec3, Vec3); 2],
    pub rays: [(Vec3, Vec3); 2],
}

impl Solver<'_> {
    /// The rate at which `section` (the ball in the plane at `s`) changes
    /// along the edge: see the module documentation. `None` for a chamfer,
    /// along a chain (whose planes turn into its joints'), where a face
    /// has no normal or curves as tightly as the ball, and where the
    /// second derivatives cannot be had.
    pub(super) fn slope(&self, s: f64, section: &Section) -> Option<Slope> {
        let super::Shape::Ball { radius, convex } = self.shape else {
            return None;
        };
        if self.spine.closed || self.spine.joints.iter().any(Option::is_some) {
            return None;
        }
        let (r, dr) = match self.law {
            Some(law) => law.radius_and_rate(s)?,
            None => (radius, 0.0),
        };
        let sigma = if convex { -1.0 } else { 1.0 };
        let c = section.centre?;
        // The plane across the edge and how it turns.
        let (t0, t1) = self.spine.range;
        let t = t0 + s * (t1 - t0);
        let (point, first) = self.spine.curve.derivatives(t);
        let d1 = first * (t1 - t0);
        let d2 = curve_second_derivative(self.spine.curve, t)? * ((t1 - t0) * (t1 - t0));
        let speed = d1.length();
        if !(speed.is_finite() && speed > 0.0) {
            return None;
        }
        let n = d1 * (1.0 / speed);
        let dn = (d2 - n * n.dot(d2)) * (1.0 / speed);
        // The faces' normals at the feet, and the centre's rate.
        let mut normals = [Vec3::ZERO; 2];
        for (slot, (side, uv)) in normals
            .iter_mut()
            .zip(self.sides.iter().zip(section.params))
        {
            *slot = side.normal(uv)?;
        }
        let [n1, n2] = normals;
        let dc = super::solve3(
            [n1, n2, n],
            [sigma * dr, sigma * dr, speed - dn.dot(c - point)],
        )?;
        // Each foot's rate along its face, and its normal's.
        let mut feet = [(Vec3::ZERO, Vec3::ZERO); 2];
        let mut rays = [(Vec3::ZERO, Vec3::ZERO); 2];
        for k in 0..2 {
            let side = self.sides.get(k)?;
            let (u, v) = *section.params.get(k)?;
            let nk = *normals.get(k)?;
            if !craft3d_geom::curvature::surface_curvature_supported(side.surface) {
                return None;
            }
            let (foot, su, sv) = side.surface.derivatives(u, v);
            let (suu, suv, svv) = second_derivatives(side.surface, u, v);
            let cross = su.cross(sv);
            let area = cross.length();
            if !(area.is_finite() && area > 0.0) {
                return None;
            }
            let m = cross * (1.0 / area);
            // The side's normal is the surface's or its opposite.
            let sign = if m.dot(nk) < 0.0 { -1.0 } else { 1.0 };
            let turn = |a: Vec3| (a - m * m.dot(a)) * (sign / area);
            let nu = turn(suu.cross(sv) + su.cross(suv));
            let nv = turn(suv.cross(sv) + su.cross(svv));
            let (au, av) = (su + nu * (sigma * r), sv + nv * (sigma * r));
            let rhs = dc - nk * (sigma * dr);
            let (a, b, c2, d) = (au.dot(su), av.dot(su), au.dot(sv), av.dot(sv));
            let det = a * d - b * c2;
            let size = (a.abs() + b.abs()) * (c2.abs() + d.abs());
            if !(det.is_finite() && det.abs() > 1e-12 * size) {
                return None;
            }
            let (x, y) = (rhs.dot(su), rhs.dot(sv));
            let du = (x * d - b * y) / det;
            let dv = (a * y - x * c2) / det;
            let dfoot = su * du + sv * dv;
            let dnormal = nu * du + nv * dv;
            if !(dfoot.is_finite() && dnormal.is_finite()) {
                return None;
            }
            *feet.get_mut(k)? = (foot, dfoot);
            *rays.get_mut(k)? = (nk * -sigma, dnormal * -sigma);
        }
        Some(Slope {
            centre: (c, dc),
            radius: (r, dr),
            feet,
            rays,
        })
    }

    /// How far the normal of a fillet's fitted `surface` at `s` turns
    /// away from the normal of the fillet itself along `section` there,
    /// the largest sine at three places across the face: the ball's
    /// radius where the radius is the same all along, the normal of the
    /// arcs between the feet where it varies.
    pub(super) fn kink_across(
        &self,
        surface: &NurbsSurface,
        s: f64,
        section: &Section,
    ) -> Option<f64> {
        if self.law.is_none() {
            return radial_kink(surface, s, section);
        }
        let slope = self.slope(s, section)?;
        let mut kink = 0.0_f64;
        for k in 1..4 {
            let v = f64::from(k) / 4.0;
            let (_, du, dv) = surface.derivatives(s, v);
            let own = du.cross(dv).normalized()?;
            kink = kink.max(own.cross(slope.normal(v)?).length());
        }
        Some(kink)
    }
}

impl Slope {
    /// The point of the fillet's surface at `v` across this section (0 on
    /// the first face, 1 on the second, the section's own rational
    /// parameter), and its derivatives along the edge and across it.
    pub(super) fn surface(&self, v: f64) -> Option<(Vec3, Vec3, Vec3)> {
        let [(f1, df1), (f2, df2)] = self.feet;
        let [(p1, dp1), (p2, dp2)] = self.rays;
        let ((c, dc), (r, dr)) = (self.centre, self.radius);
        // The middle control point and its weight.
        let spread = 1.0 + p1.dot(p2);
        if !(spread > 1e-12) {
            return None;
        }
        let dspread = dp1.dot(p2) + p1.dot(dp2);
        let w = (0.5 * spread).sqrt();
        let dw = dspread / (4.0 * w);
        let sum = p1 + p2;
        let q = c + sum * (r / spread);
        let dq = dc + sum * (dr / spread) + (dp1 + dp2) * (r / spread)
            - sum * (r * dspread / (spread * spread));
        // The homogeneous points, their rates, and the quotient rule.
        let b = [(1.0 - v) * (1.0 - v), 2.0 * v * (1.0 - v), v * v];
        let db = [-2.0 * (1.0 - v), 2.0 * (1.0 - 2.0 * v), 2.0 * v];
        let (h, weights) = ([f1, q * w, f2], [1.0, w, 1.0]);
        let (dh, dweights) = ([df1, q * dw + dq * w, df2], [0.0, dw, 0.0]);
        let (mut a, mut a_s, mut a_v) = (Vec3::ZERO, Vec3::ZERO, Vec3::ZERO);
        let (mut bottom, mut bottom_s, mut bottom_v) = (0.0_f64, 0.0_f64, 0.0_f64);
        for i in 0..3 {
            let (bi, dbi) = (*b.get(i)?, *db.get(i)?);
            let (hi, dhi) = (*h.get(i)?, *dh.get(i)?);
            let (wi, dwi) = (*weights.get(i)?, *dweights.get(i)?);
            a += hi * bi;
            a_s += dhi * bi;
            a_v += hi * dbi;
            bottom += wi * bi;
            bottom_s += dwi * bi;
            bottom_v += wi * dbi;
        }
        if !(bottom > 0.0) {
            return None;
        }
        let point = a * (1.0 / bottom);
        let along = (a_s - point * bottom_s) * (1.0 / bottom);
        let across = (a_v - point * bottom_v) * (1.0 / bottom);
        (point.is_finite() && along.is_finite() && across.is_finite())
            .then_some((point, along, across))
    }

    /// The unit normal of the fillet's surface at `v` across this
    /// section, the way the surface's parameters turn (along the edge
    /// first).
    pub(super) fn normal(&self, v: f64) -> Option<Vec3> {
        let (_, along, across) = self.surface(v)?;
        along.cross(across).normalized()
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Section, Shape, Side, Solver, Spine};
    use crate::blend::law::{Along, Fade, RadiusLaw};
    use craft3d_geom::{
        Circle, Curve3, Cylinder, NurbsCurve, NurbsSurface, Plane, Sphere, Surface,
    };
    use craft3d_math::{Frame, Portable, Vec3};
    use craft3d_topo::EdgeId;

    fn law(radius: f64, start: (f64, f64), end: (f64, f64)) -> RadiusLaw {
        RadiusLaw {
            radius,
            fades: [
                Some(Fade {
                    to: start.0,
                    over: start.1,
                }),
                Some(Fade {
                    to: end.0,
                    over: end.1,
                }),
            ],
        }
    }

    fn solver<'a>(
        curve: &'a Curve3,
        range: (f64, f64),
        sides: [Side<'a>; 2],
        radius: f64,
        law: Option<&'a Along<'a>>,
    ) -> Solver<'a> {
        Solver {
            spine: Spine {
                curve,
                range,
                closed: false,
                joints: [None, None],
                turn: 0.0,
            },
            sides,
            shape: Shape::Ball {
                radius,
                convex: true,
            },
            scale: 10.0,
            moved: None,
            settle: 0.0,
            law,
        }
    }

    /// The section's rate along the edge and across it against central
    /// differences of the construction's own sections, at five places
    /// across; and at the borders the surface's normal is the faces'.
    fn held(solver: &Solver<'_>, seed: &Section, places: &[f64]) {
        let h = 1e-4;
        for &s in places {
            let here = solver.follow(0.5, seed, s, 8).expect("a section");
            let before = solver.follow(s, &here, s - h, 8).expect("a section before");
            let after = solver.follow(s, &here, s + h, 8).expect("a section after");
            let slope = solver.slope(s, &here).expect("its slope");
            for k in 0..=4 {
                let v = f64::from(k) / 4.0;
                let (point, along, across) = slope.surface(v).unwrap();
                assert!(point.distance(here.eval(v).unwrap()) <= 1e-12);
                let numeric = (after.eval(v).unwrap() - before.eval(v).unwrap()) * (0.5 / h);
                assert!(
                    along.distance(numeric) <= 1e-6 * (1.0 + numeric.length()),
                    "s = {s}, v = {v}: {along:?} against {numeric:?}"
                );
                let (lo, hi) = ((v - 1e-5).max(0.0), (v + 1e-5).min(1.0));
                let numeric = (here.eval(hi).unwrap() - here.eval(lo).unwrap()) * (1.0 / (hi - lo));
                assert!(
                    across.distance(numeric) <= 1e-4 * (1.0 + numeric.length()),
                    "s = {s}, v = {v}: {across:?} against {numeric:?}"
                );
            }
            // Tangent to both faces along its borders, for any law.
            for (k, v) in [(0_usize, 0.0), (1, 1.0)] {
                let face = solver.sides[k].normal(here.params[k]).unwrap();
                let own = slope.normal(v).unwrap();
                assert!(
                    own.cross(face).length() <= 1e-9,
                    "s = {s}, border {k}: {}",
                    own.cross(face).length()
                );
            }
            // Between them it is the ball's radius only where the radius
            // does not change.
            let centre = here.centre.unwrap();
            let radial = (slope.surface(0.5).unwrap().0 - centre)
                .normalized()
                .unwrap();
            let lean = slope.normal(0.5).unwrap().cross(radial).length();
            if slope.radius.1 == 0.0 {
                assert!(lean <= 1e-9, "s = {s}: {lean}");
            } else if slope.radius.1.abs() > 0.05 {
                assert!(lean > 1e-3, "s = {s}: {lean}");
            }
        }
    }

    /// A rim: the circle where a plane meets a cylinder, the edge curved
    /// and one face curved along it.
    #[test]
    fn the_slope_along_a_rim_is_the_difference_of_its_sections() {
        let top = Surface::Plane(Plane::new(Frame::WORLD));
        let wall = Surface::Cylinder(Cylinder::new(Frame::WORLD, 10.0).unwrap());
        let rim = Curve3::Circle(Circle::new(Frame::WORLD, 10.0).unwrap());
        let range = (0.3, 1.5);
        let sides = [
            Side {
                surface: &top,
                reversed: false,
            },
            Side {
                surface: &wall,
                reversed: false,
            },
        ];
        let seed = |r: f64| {
            let (sin, cos) = 0.9_f64.psin_cos();
            let centre = Vec3::new((10.0 - r) * cos, (10.0 - r) * sin, -r);
            let feet = [
                Vec3::new(centre.x, centre.y, 0.0),
                Vec3::new(10.0 * cos, 10.0 * sin, -r),
            ];
            Section {
                feet,
                params: [
                    top.project(feet[0]).unwrap(),
                    wall.project(feet[1]).unwrap(),
                ],
                centre: Some(centre),
            }
        };
        // One radius: the normal is the ball's radius.
        let one = solver(&rim, range, sides, 1.0, None);
        held(
            &one,
            &one.section(0.5, &seed(1.0)).unwrap(),
            &[0.1, 0.5, 0.9],
        );
        // A law: 1 along it, from 0.4 over 5 at its start to 1.6 over 4 at
        // its end.
        let along = Along::new(law(1.0, (0.4, 5.0), (1.6, 4.0)), EdgeId(0), &rim, range).unwrap();
        let varying = solver(&rim, range, sides, 1.6, Some(&along));
        let r = along.radius(0.5).unwrap();
        held(
            &varying,
            &varying.section(0.5, &seed(r)).unwrap(),
            &[0.05, 0.2, 0.5, 0.8, 0.95],
        );
    }

    /// Freeform: a spline wall with a bulge (its own normal into the
    /// material, so the side is turned round) under a sphere, along a
    /// spline near where they meet.
    #[test]
    fn the_slope_between_a_sphere_and_a_spline_wall_is_the_difference_of_its_sections() {
        let top = Surface::Sphere(
            Sphere::new(
                Frame::new(Vec3::new(0.0, 0.0, -20.0), Vec3::Z, Vec3::X).unwrap(),
                20.5,
            )
            .unwrap(),
        );
        let bulge = [
            [0.0, 0.1, 0.05, 0.0],
            [0.1, 0.3, 0.2, 0.05],
            [0.05, 0.2, 0.3, 0.1],
            [0.0, 0.05, 0.1, 0.0],
        ];
        let mut ctrl = Vec::new();
        for (i, row) in bulge.iter().enumerate() {
            for (j, y) in row.iter().enumerate() {
                ctrl.push(Vec3::new(-3.0 + 2.0 * i as f64, *y, 1.0 - j as f64));
            }
        }
        let clamped = vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0];
        let wall = Surface::Nurbs(
            NurbsSurface::new(
                3,
                3,
                clamped.clone(),
                clamped.clone(),
                4,
                4,
                ctrl,
                vec![1.0; 16],
            )
            .unwrap(),
        );
        let edge = Curve3::Nurbs(
            NurbsCurve::new(
                3,
                clamped,
                vec![
                    Vec3::new(-2.0, 0.1, 0.4),
                    Vec3::new(-0.7, 0.25, 0.5),
                    Vec3::new(0.7, 0.2, 0.5),
                    Vec3::new(2.0, 0.05, 0.4),
                ],
                vec![1.0; 4],
            )
            .unwrap(),
        );
        let range = (0.0, 1.0);
        let sides = [
            Side {
                surface: &top,
                reversed: false,
            },
            Side {
                surface: &wall,
                reversed: true,
            },
        ];
        let seed = |r: f64| {
            let p = edge.eval(0.5);
            let feet = [p + Vec3::new(0.0, r, 0.0), p + Vec3::new(0.0, 0.0, -r)];
            Section {
                feet,
                params: [
                    top.project(feet[0]).unwrap(),
                    wall.project(feet[1]).unwrap(),
                ],
                centre: Some(p + Vec3::new(0.0, r, -r)),
            }
        };
        let one = solver(&edge, range, sides, 0.5, None);
        held(
            &one,
            &one.section(0.5, &seed(0.5)).unwrap(),
            &[0.2, 0.5, 0.8],
        );
        let along = Along::new(law(0.5, (0.2, 1.5), (0.8, 1.2)), EdgeId(0), &edge, range).unwrap();
        let varying = solver(&edge, range, sides, 0.8, Some(&along));
        let r = along.radius(0.5).unwrap();
        held(
            &varying,
            &varying.section(0.5, &seed(r)).unwrap(),
            &[0.1, 0.3, 0.5, 0.7, 0.9],
        );
    }
}
