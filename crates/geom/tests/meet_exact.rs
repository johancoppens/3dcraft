//! Surface pairs with a closed-form intersection, against independent
//! formulas, at three scales.

// Test code: a failure in a helper is a failed test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::*;
use craft3d_geom::intersect::{Meeting, SurfaceMeeting, meet_surfaces};
use craft3d_geom::{Curve3, Surface};
use craft3d_math::{Bbox3, Frame, Tolerance, Vec3};
use std::f64::consts::{FRAC_PI_2, TAU};

struct Setup {
    s: f64,
    f: Frame,
    within: Bbox3,
    t: Tolerance,
}

fn setups() -> Vec<Setup> {
    SCALES
        .iter()
        .map(|&s| {
            let f = tilted(s);
            Setup {
                s,
                f,
                within: cube(f.origin(), 40.0 * s),
                t: tol(s),
            }
        })
        .collect()
}

impl Setup {
    fn meet(&self, a: &Surface, b: &Surface) -> SurfaceMeeting {
        let found = meet_surfaces(a, b, &self.within, &self.t).unwrap();
        // The order of the surfaces does not matter, and neither does asking twice.
        assert_eq!(found, meet_surfaces(b, a, &self.within, &self.t).unwrap());
        assert_eq!(found, meet_surfaces(a, b, &self.within, &self.t).unwrap());
        if let SurfaceMeeting::Meets(m) = &found {
            check_meeting(m, a, b, &self.within, &self.t);
        }
        found
    }

    fn curves(&self, a: &Surface, b: &Surface, count: usize) -> Meeting {
        let m = meeting(self.meet(a, b));
        assert_eq!(m.curves.len(), count, "{m:?}");
        assert!(m.points.is_empty());
        assert!(m.curves.iter().all(|c| c.exact));
        m
    }

    fn touch(&self, a: &Surface, b: &Surface, at: Vec3) {
        let m = meeting(self.meet(a, b));
        assert!(m.curves.is_empty(), "{m:?}");
        assert_eq!(m.points.len(), 1);
        assert!(m.points[0].distance(at) <= 2.0 * self.t.linear);
    }

    /// The only curve is a full circle with this centre and radius.
    fn circle(&self, m: &Meeting, index: usize, center: Vec3, radius: f64) {
        let c = &m.curves[index];
        let Curve3::Circle(circle) = &c.curve else {
            panic!("expected a circle, got {c:?}");
        };
        assert!(c.closed && c.range == (0.0, TAU));
        assert!(
            circle.frame().origin().distance(center) <= 1e-9 * self.s,
            "centre {:?} instead of {center:?}",
            circle.frame().origin()
        );
        assert!(
            (circle.radius() - radius).abs() <= 1e-9 * self.s,
            "radius {} instead of {radius}",
            circle.radius()
        );
    }
}

#[test]
fn a_plane_cuts_a_sphere_in_a_circle_or_touches_it() {
    for x in setups() {
        let (s, f) = (x.s, x.f);
        let ball = sphere(f, 3.0 * s);
        let n = f.vector(0.2, 0.5, 1.0).normalized().unwrap();
        let at = |d: f64| plane(Frame::new(f.origin() + n * (d * s), n, f.x()).unwrap());
        let m = x.curves(&at(1.2), &ball, 1);
        x.circle(
            &m,
            0,
            f.origin() + n * (1.2 * s),
            (9.0_f64 - 1.44).sqrt() * s,
        );
        assert!(!m.curves[0].tangent);
        x.touch(&at(3.0), &ball, f.origin() + n * (3.0 * s));
        x.touch(&at(-3.0), &ball, f.origin() - n * (3.0 * s));
        assert_eq!(x.meet(&at(3.1), &ball), SurfaceMeeting::None);
    }
}

#[test]
fn a_plane_cuts_a_cone_in_every_conic() {
    for x in setups() {
        let (s, f) = (x.s, x.f);
        let alpha: f64 = 0.4;
        let k = alpha.tan();
        let c = cone(f, 1.5 * s, alpha);
        let apex = f.point(0.0, 0.0, -1.5 * s / k);

        // Across the axis: a circle whose angle is the cone's u.
        let m = x.curves(&plane(f.at(f.point(s, s, 2.0 * s)).unwrap()), &c, 1);
        x.circle(&m, 0, f.point(0.0, 0.0, 2.0 * s), (1.5 + 2.0 * k) * s);
        let (u, _) = c.project(m.curves[0].curve.eval(1.25)).unwrap();
        assert!((u - 1.25).abs() < 1e-12);
        // Behind the apex: nothing.
        let behind = plane(f.at(f.point(0.0, 0.0, -5.0 * s)).unwrap());
        assert_eq!(x.meet(&behind, &c), SurfaceMeeting::None);

        // Steeper than the rulings: an ellipse of eccentricity
        // cos(plane to axis) / cos(half-angle).
        let lean: f64 = 0.5; // between the plane's normal and the axis
        let through = f.point(0.0, 0.0, 2.0 * s);
        let oblique =
            plane(Frame::new(through, f.vector(lean.sin(), 0.0, lean.cos()), f.y()).unwrap());
        let m = x.curves(&oblique, &c, 1);
        let Curve3::Ellipse(e) = &m.curves[0].curve else {
            panic!("expected an ellipse: {m:?}");
        };
        assert!(m.curves[0].closed);
        let (major, minor) = e.semi_axes();
        let eccentricity = (1.0 - (minor / major).powi(2)).sqrt();
        assert!((eccentricity - lean.sin() / alpha.cos()).abs() < 1e-12);

        // Parallel to a ruling: a parabola, a polynomial quadratic that
        // runs to the box.
        let lean = FRAC_PI_2 - alpha;
        let parallel =
            plane(Frame::new(through, f.vector(lean.sin(), 0.0, lean.cos()), f.y()).unwrap());
        let m = x.curves(&parallel, &c, 1);
        let Curve3::Nurbs(parabola) = &m.curves[0].curve else {
            panic!("expected a NURBS parabola: {m:?}");
        };
        assert!(parabola.degree() == 2 && parabola.weights().iter().all(|w| *w == 1.0));
        let inner = cube(f.origin(), 40.0 * s - 1e-3 * s);
        assert!(!inner.contains(start(&m.curves[0]), 0.0));
        assert!(!inner.contains(end(&m.curves[0]), 0.0));
        assert!(!m.curves[0].closed && !m.curves[0].tangent);
        // The same plane on the far side of the apex misses this nappe.
        let far = plane(
            Frame::new(
                f.point(0.0, 0.0, -9.0 * s),
                f.vector(lean.sin(), 0.0, lean.cos()),
                f.y(),
            )
            .unwrap(),
        );
        assert_eq!(x.meet(&far, &c), SurfaceMeeting::None);

        // Along the axis, beside it: one branch of a hyperbola, a rational
        // quadratic that runs to the box. In the cone's frame the plane is
        // x = 1 and the curve y² = (1.5 + k z)² - 1.
        let beside = plane(Frame::new(f.point(s, 0.0, 0.0), f.x(), f.z()).unwrap());
        let m = x.curves(&beside, &c, 1);
        let Curve3::Nurbs(hyperbola) = &m.curves[0].curve else {
            panic!("expected a NURBS hyperbola: {m:?}");
        };
        assert!(hyperbola.degree() == 2 && hyperbola.weights().iter().any(|w| *w > 1.0));
        assert!(!inner.contains(start(&m.curves[0]), 0.0));
        assert!(!inner.contains(end(&m.curves[0]), 0.0));
        for p in samples(&m.curves[0], 50) {
            let (px, py, pz) = f.local(p);
            assert!((px - s).abs() <= 1e-9 * s);
            assert!((py * py - ((1.5 * s + k * pz).powi(2) - s * s)).abs() <= 1e-9 * s * s * 40.0);
            assert!(pz > -1.5 * s / k);
        }

        // Through the apex: the apex alone, two rulings, or one (tangent).
        let across = plane(Frame::new(apex, f.vector(0.3, 0.0, 1.0), f.y()).unwrap());
        x.touch(&across, &c, apex);
        let along = plane(Frame::new(apex, f.vector(1.0, 0.3, 0.0), f.z()).unwrap());
        let m = x.curves(&along, &c, 2);
        for ray in &m.curves {
            assert!(matches!(ray.curve, Curve3::Line(_)) && !ray.tangent);
            assert!(
                start(ray).distance(apex) <= 1e-9 * s,
                "rays start on the apex"
            );
            assert!(!inner.contains(end(ray), 0.0));
        }
        assert!(end(&m.curves[0]).distance(end(&m.curves[1])) > s);
        let grazing =
            plane(Frame::new(apex, f.vector(alpha.cos(), 0.0, -alpha.sin()), f.y()).unwrap());
        let m = x.curves(&grazing, &c, 1);
        assert!(matches!(m.curves[0].curve, Curve3::Line(_)) && m.curves[0].tangent);
        assert!(start(&m.curves[0]).distance(apex) <= 1e-9 * s);

        // A cone that narrows along its axis.
        let down = cone(f, 1.5 * s, -0.7);
        let m = x.curves(&plane(f.at(f.point(0.0, 0.0, -s)).unwrap()), &down, 1);
        x.circle(&m, 0, f.point(0.0, 0.0, -s), (1.5 + 0.7_f64.tan()) * s);
        let above = plane(f.at(f.point(0.0, 0.0, 3.0 * s)).unwrap());
        assert_eq!(x.meet(&above, &down), SurfaceMeeting::None);
    }
}

#[test]
fn spheres_meet_in_a_circle_or_touch_or_coincide() {
    for x in setups() {
        let (s, f) = (x.s, x.f);
        let (r1, r2) = (3.0 * s, 2.0 * s);
        let toward = f.vector(0.5, -0.2, 0.7).normalized().unwrap();
        let a = sphere(f, r1);
        let other = |d: f64, r: f64| sphere(f.at(f.origin() + toward * (d * s)).unwrap(), r);
        let m = x.curves(&a, &other(4.0, r2), 1);
        // Heron: the circle's radius is the height of the triangle of the
        // two radii over the line of centres.
        let d = 4.0 * s;
        let half = 0.5 * (r1 + r2 + d);
        let area = (half * (half - r1) * (half - r2) * (half - d)).sqrt();
        let along = (d * d + r1 * r1 - r2 * r2) / (2.0 * d);
        x.circle(&m, 0, f.origin() + toward * along, 2.0 * area / d);
        x.touch(&a, &other(5.0, r2), f.origin() + toward * r1);
        x.touch(&a, &other(1.0, r2), f.origin() + toward * r1);
        assert_eq!(x.meet(&a, &other(5.5, r2)), SurfaceMeeting::None);
        assert_eq!(x.meet(&a, &other(0.5, r2)), SurfaceMeeting::None);
        assert_eq!(x.meet(&a, &other(0.0, r2)), SurfaceMeeting::None);
        let turned = sphere(Frame::new(f.origin(), f.x(), f.y()).unwrap(), r1);
        assert_eq!(
            x.meet(&a, &turned),
            SurfaceMeeting::Coincident { same_normal: true }
        );
    }
}

#[test]
fn surfaces_about_one_axis_meet_in_circles() {
    for x in setups() {
        let (s, f) = (x.s, x.f);
        let up = |z: f64| f.point(0.0, 0.0, z * s);
        let ball = sphere(f.at(up(1.0)).unwrap(), 3.0 * s);

        // Cylinder and sphere.
        let m = x.curves(&cylinder(f, 2.0 * s), &ball, 2);
        let w = 5.0_f64.sqrt();
        x.circle(&m, 0, up(1.0 - w), 2.0 * s);
        x.circle(&m, 1, up(1.0 + w), 2.0 * s);
        let (u, _) = cylinder(f, 2.0 * s)
            .project(m.curves[0].curve.eval(2.5))
            .unwrap();
        assert!((u - 2.5).abs() < 1e-12, "the angle is the cylinder's u");
        let m = x.curves(&ball, &cylinder(f, 3.0 * s), 1);
        x.circle(&m, 0, up(1.0), 3.0 * s);
        assert!(m.curves[0].tangent, "the sphere fits the cylinder");
        assert_eq!(x.meet(&ball, &cylinder(f, 3.5 * s)), SurfaceMeeting::None);

        // Cylinder and cone, the cylinder's axis either way round.
        let alpha: f64 = 0.4;
        let k = alpha.tan();
        let c = cone(f, 1.5 * s, alpha);
        for axis in [f, f.reversed_at(up(7.0)).unwrap()] {
            let m = x.curves(&cylinder(axis, 2.5 * s), &c, 1);
            x.circle(&m, 0, up(1.0 / k), 2.5 * s);
        }

        // Cone and a sphere about its apex.
        let apex = up(-1.5 / k);
        let m = x.curves(&c, &sphere(f.at(apex).unwrap(), 4.0 * s), 1);
        x.circle(
            &m,
            0,
            apex + f.z() * (4.0 * s * alpha.cos()),
            4.0 * s * alpha.sin(),
        );

        // Two cones facing each other: ρ = 1.5 + k z and ρ = 1 + k2 (6 - z).
        let k2 = 0.3_f64.tan();
        let facing = cone(f.reversed_at(up(6.0)).unwrap(), s, 0.3);
        let z = (6.0 * k2 - 0.5) / (k + k2);
        let m = x.curves(&c, &facing, 1);
        x.circle(&m, 0, up(z), (1.5 + k * z) * s);
        // Apex to apex: a point.
        let mirrored = cone(f.reversed_at(up(-3.0 / k)).unwrap(), 1.5 * s, alpha);
        x.touch(&c, &mirrored, apex);

        // Torus with cylinders, a sphere and another torus.
        let ring = torus(f, 4.0 * s, s);
        let m = x.curves(&cylinder(f, 4.0 * s), &ring, 2);
        x.circle(&m, 0, up(-1.0), 4.0 * s);
        x.circle(&m, 1, up(1.0), 4.0 * s);
        for r in [3.0, 5.0] {
            let m = x.curves(&cylinder(f, r * s), &ring, 1);
            x.circle(&m, 0, up(0.0), r * s);
            assert!(m.curves[0].tangent);
        }
        assert_eq!(x.meet(&cylinder(f, 2.9 * s), &ring), SurfaceMeeting::None);
        let m = x.curves(&sphere(f, 4.5 * s), &ring, 2);
        let along = (16.0 + 20.25 - 1.0) / 8.0;
        let half = (20.25_f64 - along * along).sqrt();
        for (curve, side) in [(0, -1.0), (1, 1.0)] {
            let c = &m.curves[curve];
            let Curve3::Circle(circle) = &c.curve else {
                panic!()
            };
            assert!((circle.radius() - along * s).abs() <= 1e-9 * s);
            let height = f.local(circle.frame().origin()).2;
            assert!((height.abs() - half * s).abs() <= 1e-9 * s);
            let _ = side;
        }
        let lifted = torus(f.at(up(1.0)).unwrap(), 4.0 * s, s);
        let m = x.curves(&ring, &lifted, 2);
        let w = 0.75_f64.sqrt();
        let mut radii: Vec<f64> = m
            .curves
            .iter()
            .map(|c| match &c.curve {
                Curve3::Circle(circle) => {
                    assert!(circle.frame().origin().distance(up(0.5)) <= 1e-9 * s);
                    circle.radius() / s
                }
                _ => panic!(),
            })
            .collect();
        radii.sort_by(f64::total_cmp);
        assert!((radii[0] - (4.0 - w)).abs() < 1e-9 && (radii[1] - (4.0 + w)).abs() < 1e-9);

        // The same surface placed differently is coincident.
        let same = SurfaceMeeting::Coincident { same_normal: true };
        assert_eq!(
            x.meet(
                &c,
                &cone(f.at(up(2.0)).unwrap(), (1.5 + 2.0 * k) * s, alpha)
            ),
            same
        );
        assert_eq!(
            x.meet(
                &c,
                &cone(f.reversed_at(up(2.0)).unwrap(), (1.5 + 2.0 * k) * s, -alpha)
            ),
            same
        );
        assert_eq!(
            x.meet(
                &ring,
                &torus(f.reversed_at(f.origin()).unwrap(), 4.0 * s, s)
            ),
            same
        );
        assert_eq!(
            x.meet(&ring, &torus(f, 4.0 * s, 0.9 * s)),
            SurfaceMeeting::None
        );
        assert_eq!(x.meet(&c, &cone(f, 1.6 * s, alpha)), SurfaceMeeting::None);
    }
}

#[test]
fn a_plane_cuts_a_torus_across_or_through_its_axis() {
    for x in setups() {
        let (s, f) = (x.s, x.f);
        let ring = torus(f, 4.0 * s, s);
        let level = |z: f64| plane(f.at(f.point(s, -s, z * s)).unwrap());
        let m = x.curves(&level(0.5), &ring, 2);
        let w = 0.75_f64.sqrt();
        x.circle(&m, 0, f.point(0.0, 0.0, 0.5 * s), (4.0 - w) * s);
        x.circle(&m, 1, f.point(0.0, 0.0, 0.5 * s), (4.0 + w) * s);
        let m = x.curves(&level(-1.0), &ring, 1);
        x.circle(&m, 0, f.point(0.0, 0.0, -s), 4.0 * s);
        assert!(m.curves[0].tangent, "the torus lies on the plane");
        assert_eq!(x.meet(&level(1.2), &ring), SurfaceMeeting::None);

        let through = plane(Frame::new(f.point(0.0, 0.0, 3.0 * s), f.x(), f.z()).unwrap());
        let m = x.curves(&through, &ring, 2);
        let mut sides = Vec::new();
        for (k, curve) in m.curves.iter().enumerate() {
            let Curve3::Circle(circle) = &curve.curve else {
                panic!()
            };
            let (cx, cy, cz) = f.local(circle.frame().origin());
            assert!(cx.abs() <= 1e-9 * s && cz.abs() <= 1e-9 * s);
            assert!((cy.abs() - 4.0 * s).abs() <= 1e-9 * s);
            sides.push(cy > 0.0);
            x.circle(&m, k, circle.frame().origin(), s);
            let (_, v) = ring.project(curve.curve.eval(0.7)).unwrap();
            assert!((v - 0.7).abs() < 1e-12, "the angle is the torus' v");
        }
        assert_ne!(sides[0], sides[1]);
    }
}

#[test]
fn equal_cylinders_whose_axes_cross_meet_in_four_half_ellipses() {
    for x in setups() {
        let (s, f) = (x.s, x.f);
        let r = 2.0 * s;
        let a = cylinder(f, r);
        let cross = f.point(0.0, 0.0, s);
        let g = Frame::new(cross, f.vector(1.0, 0.0, 0.5), f.z()).unwrap();
        let b = cylinder(g, r);
        let m = x.curves(&a, &b, 4);
        // The two points where the cylinders are tangent.
        let common = f.z().cross(g.z()).normalized().unwrap();
        let poles = [cross + common * r, cross - common * r];
        let angle = f.z().dot(g.z()).acos();
        let mut majors = Vec::new();
        for curve in &m.curves {
            let Curve3::Ellipse(e) = &curve.curve else {
                panic!("expected ellipses: {curve:?}");
            };
            assert!(!curve.closed && !curve.tangent);
            let (p0, p1) = (start(curve), end(curve));
            assert!(p0.distance(p1) > r, "half an ellipse runs pole to pole");
            for p in [p0, p1] {
                assert!(poles.iter().any(|q| q.distance(p) <= 1e-9 * s));
            }
            assert!((curve.range.1 - curve.range.0 - TAU / 2.0).abs() < 1e-9);
            assert!((e.semi_axes().1 - r).abs() <= 1e-9 * s);
            majors.push(e.semi_axes().0 / r);
        }
        majors.sort_by(f64::total_cmp);
        assert!((majors[0] - 1.0 / (0.5 * angle).cos()).abs() < 1e-9);
        assert!((majors[3] - 1.0 / (0.5 * angle).sin()).abs() < 1e-9);
        // The four halves are different curves.
        for i in 0..4 {
            for j in 0..i {
                let (mi, mj) = (samples(&m.curves[i], 2)[1], samples(&m.curves[j], 2)[1]);
                assert!(mi.distance(mj) > 0.1 * s);
            }
        }
    }
}

#[test]
fn equal_cones_with_parallel_axes_meet_in_a_plane() {
    for x in setups() {
        let (s, f) = (x.s, x.f);
        let a = cone(f, 1.5 * s, 0.4);
        // Shifted along the axis, the second cone is inside the first:
        // nothing (the plane they would meet in cuts the other nappe).
        let lifted = cone(f.at(f.point(0.2 * s, 0.0, 3.0 * s)).unwrap(), 1.5 * s, 0.4);
        assert_eq!(x.meet(&a, &lifted), SurfaceMeeting::None);
        for i in 0..40 {
            let p = lifted.eval(f64::from(i), f64::from(i % 7) * s);
            let (px, py, pz) = f.local(p);
            assert!(
                px.hypot(py) < 1.5 * s + pz * 0.4_f64.tan(),
                "inside the first"
            );
        }
        // Shifted across: one branch of a hyperbola.
        let beside = cone(f.at(f.point(2.0 * s, 0.5 * s, s)).unwrap(), 1.5 * s, 0.4);
        let m = x.curves(&a, &beside, 1);
        assert!(matches!(m.curves[0].curve, Curve3::Nurbs(_)) && !m.curves[0].closed);
        // Turned upside down above it: an ellipse.
        let facing = cone(
            f.reversed_at(f.point(0.3 * s, 0.0, 9.0 * s)).unwrap(),
            1.5 * s,
            0.4,
        );
        let m = x.curves(&a, &facing, 1);
        assert!(matches!(m.curves[0].curve, Curve3::Ellipse(_)) && m.curves[0].closed);
        // Upside down far beside it: the radii add up to less than the
        // distance between the axes at every height.
        let upside = cone(
            f.reversed_at(f.point(12.0 * s, 0.5 * s, 2.0 * s)).unwrap(),
            1.5 * s,
            0.4,
        );
        assert_eq!(x.meet(&a, &upside), SurfaceMeeting::None);
    }
}

#[test]
fn curves_are_clipped_to_the_box() {
    for x in setups() {
        let (s, f) = (x.s, x.f);
        let t = x.t;
        // A line through a box: it ends on the box.
        let a = plane(f);
        let b = plane(Frame::new(f.origin(), f.x(), f.z()).unwrap());
        let small = cube(f.origin(), 2.0 * s);
        let m = meeting(meet_surfaces(&a, &b, &small, &t).unwrap());
        assert_eq!(m.curves.len(), 1);
        check_meeting(&m, &a, &b, &small, &t);
        let shrunk = cube(f.origin(), 2.0 * s - 1e-3 * s);
        assert!(!shrunk.contains(start(&m.curves[0]), 0.0));
        assert!(!shrunk.contains(end(&m.curves[0]), 0.0));
        assert!(start(&m.curves[0]).distance(end(&m.curves[0])) >= 4.0 * s);
        // A box the line misses.
        let away = cube(f.origin() + f.z() * (9.0 * s), s);
        assert_eq!(
            meet_surfaces(&a, &b, &away, &t).unwrap(),
            SurfaceMeeting::None
        );

        // A circle of radius 3 against boxes: every part of the circle in
        // the box is returned, and nothing else.
        let ball = sphere(f, 3.0 * s);
        for (center, half) in [
            (f.point(3.0 * s, 0.0, 0.0), 1.5 * s),
            (f.origin(), 2.5 * s),
            (f.origin(), 2.2 * s),
            (f.point(0.5 * s, 0.4 * s, 0.0), 2.6 * s),
        ] {
            let within = cube(center, half);
            let found = meet_surfaces(&a, &ball, &within, &t).unwrap();
            assert_eq!(found, meet_surfaces(&ball, &a, &within, &t).unwrap());
            let m = match found {
                SurfaceMeeting::Meets(m) => m,
                _ => Meeting {
                    curves: Vec::new(),
                    points: Vec::new(),
                },
            };
            check_meeting(&m, &a, &ball, &within, &t);
            assert!(m.curves.iter().all(|c| !c.closed));
            let mut inside = 0;
            let lines = Lines::of(&m, 400);
            for k in 0..3600 {
                let angle = TAU * f64::from(k) / 3600.0;
                let p = f.point(3.0 * s * angle.cos(), 3.0 * s * angle.sin(), 0.0);
                if cube(center, half - 0.01 * s).contains(p, 0.0) {
                    inside += 1;
                    assert!(lines.distance(p) <= 1e-3 * s, "missing at {angle}");
                }
            }
            let length: f64 = m
                .curves
                .iter()
                .map(|c| (c.range.1 - c.range.0) * 3.0 * s)
                .sum();
            let expected = f64::from(inside) * TAU * 3.0 * s / 3600.0;
            assert!(
                (length - expected).abs() <= 0.2 * s,
                "{length} vs {expected}"
            );
        }
        // Wholly inside: closed.
        let m = meeting(meet_surfaces(&a, &ball, &cube(f.origin(), 4.0 * s), &t).unwrap());
        assert!(m.curves.len() == 1 && m.curves[0].closed);
    }
}

#[test]
fn hostile_input_is_an_error() {
    let f = tilted(1.0);
    let (a, b) = (plane(f), sphere(f, 1.0));
    let within = cube(f.origin(), 5.0);
    let t = tol(1.0);
    assert!(meet_surfaces(&a, &b, &Bbox3::EMPTY, &t).is_err());
    let infinite = Bbox3::from_points([Vec3::ZERO, Vec3::new(f64::INFINITY, 1.0, 1.0)]);
    assert!(meet_surfaces(&a, &b, &infinite, &t).is_err());
    for bad in [f64::NAN, 0.0, -1.0, f64::INFINITY] {
        let linear = Tolerance {
            linear: bad,
            angular: 1e-9,
        };
        let angular = Tolerance {
            linear: 1e-6,
            angular: bad,
        };
        assert!(meet_surfaces(&a, &b, &within, &linear).is_err());
        assert!(meet_surfaces(&a, &b, &within, &angular).is_err());
    }
}
