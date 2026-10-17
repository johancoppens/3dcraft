//! `meet_surfaces` at a cusp: a surface that narrows to a point as the
//! square of the distance (a spline that leaves the axis along it, swept
//! about it), against surfaces that hold the axis through the point.
//! Refused until 2026-10-17 ("touch along a curve"; docs/design/booleans.md
//! › *Cusps*).

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_geom::intersect::{SurfaceMeeting, meet_surfaces};
use craft3d_geom::{Curve3, Cylinder, NurbsSurface, Plane, Surface};
use craft3d_math::{Bbox3, Frame, Tolerance, Vec3};

/// The needle: `ρ = 9 t² − 6 t³`, `y = 6 t`, swept in a whole turn about
/// the y axis (u around, v along the profile, the point at v = 0).
fn needle(k: f64) -> Surface {
    let s = std::f64::consts::FRAC_1_SQRT_2;
    let ring = [
        (1.0, 0.0, 1.0),
        (1.0, 1.0, s),
        (0.0, 1.0, 1.0),
        (-1.0, 1.0, s),
        (-1.0, 0.0, 1.0),
        (-1.0, -1.0, s),
        (0.0, -1.0, 1.0),
        (1.0, -1.0, s),
        (1.0, 0.0, 1.0),
    ];
    let profile = [(0.0, 0.0), (0.0, 2.0), (3.0, 4.0), (3.0, 6.0)];
    let (mut ctrl, mut weights) = (Vec::new(), Vec::new());
    for (x, z, w) in ring {
        for (r, y) in profile {
            ctrl.push(Vec3::new(x * r * k, y * k, z * r * k));
            weights.push(w);
        }
    }
    Surface::Nurbs(
        NurbsSurface::new(
            2,
            3,
            vec![
                0.0, 0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0, 1.0,
            ],
            vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
            9,
            4,
            ctrl,
            weights,
        )
        .unwrap(),
    )
}

/// How far `p` is from the needle: from the circle of its height.
fn off_needle(p: Vec3, k: f64) -> f64 {
    let t = p.y / (6.0 * k);
    let rho = (9.0 * t * t - 6.0 * t * t * t) * k;
    (p.x.hypot(p.z) - rho).abs()
}

fn curves(a: &Surface, b: &Surface, k: f64) -> Vec<(Curve3, (f64, f64), f64)> {
    let tol = Tolerance::new(1e-6 * k, 1e-9).unwrap();
    let within = Bbox3::from_points([
        Vec3::new(-4.0, -1.0, -4.0) * k,
        Vec3::new(4.0, 7.0, 4.0) * k,
    ]);
    match meet_surfaces(a, b, &within, &tol).unwrap_or_else(|e| panic!("{b:?} at {k}: {e}")) {
        SurfaceMeeting::Meets(m) => m
            .curves
            .into_iter()
            .map(|c| (c.curve, c.range, c.deviation))
            .collect(),
        other => panic!("{other:?}"),
    }
}

/// A plane through the axis meets the needle in two meridians that end
/// on the point, to the last bit, and lie within their deviation of both
/// surfaces all along (sampled at 400 places each, also within a
/// thousandth of the point); a cylinder with the axis on its side the
/// same, in a curve from the point around its side of the axis and back.
#[test]
fn faces_that_hold_the_axis_of_a_cusp_meet_it_in_curves_that_end_on_the_point() {
    for k in [1e-3, 1.0, 1e6] {
        let surface = needle(k);
        let tol = 1e-6 * k;
        for normal in [
            Vec3::X,
            Vec3::new(0.5, 0.0, 0.75_f64.sqrt()),
            Vec3::new(0.8, 0.0, -0.6),
        ] {
            let plane =
                Surface::Plane(Plane::new(Frame::new(Vec3::ZERO, normal, Vec3::Y).unwrap()));
            let cylinder = {
                let side = normal.cross(Vec3::Y);
                let frame = Frame::new(side * (2.0 * k), Vec3::Y, normal).unwrap();
                Surface::Cylinder(Cylinder::new(frame, 2.0 * k).unwrap())
            };
            for (other, name, ends) in [(&plane, "plane", 2), (&cylinder, "cylinder", 2)] {
                let found = curves(&surface, other, k);
                let mut on_point = 0;
                for (curve, range, deviation) in &found {
                    assert!(*deviation <= tol, "{name} at {k}: {deviation}");
                    for end in [range.0, range.1] {
                        if curve.eval(end).length() == 0.0 {
                            on_point += 1;
                        }
                    }
                    for i in 0..=400 {
                        let share = f64::from(i) / 400.0;
                        // Dense near the ends, where the point is.
                        let share = 0.5 - 0.5 * (std::f64::consts::PI * share).cos();
                        let p = curve.eval(range.0 + (range.1 - range.0) * share);
                        assert!(off_needle(p, k) <= *deviation, "{name} at {k}: {p:?}");
                        let off = match other {
                            Surface::Plane(_) => p.dot(normal).abs(),
                            _ => {
                                let side = normal.cross(Vec3::Y) * (2.0 * k);
                                let q = p - side;
                                (q.x.hypot(q.z) - 2.0 * k).abs()
                            }
                        };
                        assert!(off <= *deviation, "{name} at {k}: {p:?} {off}");
                    }
                }
                assert_eq!(on_point, ends, "{name} at {k}: {found:?}");
            }
        }
    }
}

/// A plane across the axis near the point, not through it, still cuts
/// its circle there (the chart is moved in short of the cusp only where
/// the other surface passes through the point).
#[test]
fn a_plane_across_a_cusp_near_its_point_cuts_its_circle() {
    for k in [1e-3, 1.0, 1e6] {
        let surface = needle(k);
        let height = 0.003 * k;
        let plane = Surface::Plane(Plane::new(
            Frame::new(Vec3::Y * height, Vec3::Y, Vec3::X).unwrap(),
        ));
        let found = curves(&surface, &plane, k);
        assert_eq!(found.len(), 1, "{k}");
        let (curve, range, _) = &found[0];
        let t: f64 = 0.0005;
        let rho = (9.0 * t * t - 6.0 * t * t * t) * k;
        let p = curve.eval(0.5 * (range.0 + range.1));
        assert!((p.x.hypot(p.z) - rho).abs() <= 1e-6 * k, "{k}: {p:?}");
    }
}
