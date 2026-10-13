//! Regression tests for defects of `meet_surfaces` found by the booleans.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_geom::intersect::{SurfaceMeeting, meet_surfaces};
use craft3d_geom::{Sphere, Surface};
use craft3d_math::{Bbox3, Frame, Tolerance, Vec3};

fn everywhere() -> Bbox3 {
    Bbox3::from_points([Vec3::new(-9.0, -9.0, -9.0), Vec3::new(9.0, 9.0, 9.0)])
}

#[test]
fn spheres_apart_by_the_tolerance_to_the_last_bit_touch_or_miss_but_do_not_fail() {
    // Found as `near/sphere-sphere/017` of the boolean corpus: radii 1.375
    // and 1, centres 2.375 + 1e-6 apart, where 2.000001 - (-0.375) is a
    // hair over the tolerance in floating point. It was neither "apart"
    // nor "touching" and ended as a circle of radius zero, an error.
    let tol = Tolerance::default();
    let a = Surface::Sphere(
        Sphere::new(
            Frame::WORLD.at(Vec3::new(1.5, -0.375, 1.375)).unwrap(),
            1.375,
        )
        .unwrap(),
    );
    for gap in [0.0, 0.5e-6, 1.0e-6, 1.0000001e-6, 1.5e-6, -1.0e-6] {
        let b = Surface::Sphere(
            Sphere::new(
                Frame::WORLD.at(Vec3::new(1.5, 2.0 + gap, 1.375)).unwrap(),
                1.0,
            )
            .unwrap(),
        );
        match meet_surfaces(&a, &b, &everywhere(), &tol) {
            Ok(SurfaceMeeting::None) => assert!(gap > 0.9e-6, "{gap}"),
            Ok(SurfaceMeeting::Meets(meeting)) => {
                // A touch, or (overlapping by a tolerance) a small circle.
                assert!(meeting.curves.len() + meeting.points.len() == 1, "{gap}");
            }
            other => panic!("{gap}: {other:?}"),
        }
    }
}
