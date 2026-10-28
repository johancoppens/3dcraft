//! `NurbsCurve::hull_points`: points whose hull holds a curve over part
//! of its domain (2026-10-28, the boolean's near boxes of faces).

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_geom::NurbsCurve;
use craft3d_math::{Bbox3, Frame, Vec3};

/// A cubic of uneven spans whose last control point is far out.
fn spline(k: f64) -> NurbsCurve {
    NurbsCurve::polynomial(
        3,
        vec![0.0, 0.0, 0.0, 0.0, 0.1, 0.15, 0.6, 1.0, 1.0, 1.0, 1.0],
        vec![
            Vec3::new(0.0, 0.0, 0.0) * k,
            Vec3::new(1.0, 2.0, -1.0) * k,
            Vec3::new(2.0, -1.0, 0.5) * k,
            Vec3::new(3.0, 0.5, 2.0) * k,
            Vec3::new(4.0, 1.0, -0.5) * k,
            Vec3::new(5.0, 0.0, 0.0) * k,
            Vec3::new(1e6, -1e6, 1e6) * k,
        ],
    )
    .unwrap()
}

/// Every sampled point of the curve over `range` within the box of its
/// hull points (to rounding), and the box no larger than `cap` times the
/// sampled one's diagonal.
fn holds(curve: &NurbsCurve, range: (f64, f64), k: f64, cap: f64) {
    let points = curve.hull_points(range).unwrap();
    let held = Bbox3::from_points(points.iter().copied());
    let mut sampled = Bbox3::EMPTY;
    for i in 0..=512 {
        let t = range.0 + (range.1 - range.0) * f64::from(i) / 512.0;
        let p = curve.eval(t);
        assert!(
            held.contains(p, 1e-12 * k),
            "{k} {range:?}: {p:?} outside {held:?}"
        );
        sampled.add(p);
    }
    assert!(
        held.diagonal() <= cap * sampled.diagonal().max(1e-9 * k),
        "{k} {range:?}: {held:?} against {sampled:?}"
    );
}

#[test]
fn the_hull_of_part_of_a_spline_holds_that_part_and_leaves_far_control_points_out() {
    for k in [1e-3, 1.0, 1e6] {
        let c = spline(k);
        // The whole domain: the control points themselves.
        assert_eq!(c.hull_points((0.0, 1.0)).unwrap(), c.control_points());
        assert_eq!(c.hull_points((1.0, -0.5)).unwrap(), c.control_points());
        for range in [
            (0.0, 0.5),
            (0.05, 0.12),
            (0.12, 0.13),
            (0.3, 0.1),
            (0.2, 0.55),
        ] {
            holds(&c, range, k, 8.0);
        }
        // Past the domain on one side only, or not finite: none.
        assert!(c.hull_points((-0.1, 0.5)).is_none());
        assert!(c.hull_points((0.5, f64::NAN)).is_none());
        // One parameter: the point (within rounding).
        let one = c.hull_points((0.3, 0.3)).unwrap();
        assert!(one.iter().all(|p| p.distance(c.eval(0.3)) <= 1e-12 * k));
    }
}

#[test]
fn the_hull_of_part_of_a_rational_circle_holds_that_part() {
    for k in [1e-3, 1.0, 1e6] {
        let frame = Frame::WORLD.at(Vec3::new(1.0, -2.0, 3.0) * k).unwrap();
        let c = NurbsCurve::circle(&frame, 2.0 * k).unwrap();
        let (d0, d1) = c.domain();
        for (a, b) in [(0.0, 0.1), (0.13, 0.42), (0.5, 0.97), (0.01, 0.02)] {
            let range = (d0 + (d1 - d0) * a, d0 + (d1 - d0) * b);
            holds(&c, range, k, 4.0);
        }
    }
}
