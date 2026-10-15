//! Regression tests for defects of `meet_surfaces` found by the booleans.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_geom::intersect::{SurfaceMeeting, meet_surfaces};
use craft3d_geom::{NurbsSurface, Plane, Sphere, Surface};
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

/// The pieces of a curve that a box cuts are held to the tolerance (the
/// lamp's finding B1, 2026-10-15). The outer wall of a lampshade turned
/// from a spline (a rational surface with a pole at its top) and the end
/// cap of a lofted cell meet in a closed curve, which the box (the one
/// the ten cells of the lamp gave this pair, inlined) cuts. The span from
/// the last point inside to the point on the box was a new cubic that
/// was never refined: "an intersection curve could not be fitted within
/// the tolerance" (1.05e-6 where the target is 0.5e-6).
#[test]
fn a_curve_cut_by_the_box_stays_within_the_tolerance_along_the_cut_piece() {
    let tol = Tolerance::default();
    let cap = Surface::Plane(Plane::new(
        Frame::new(
            Vec3::new(-101.30507340959298, -111.29355510700076, 141.5504168692775),
            Vec3::new(0.6691084494494924, 0.7350812312260296, -0.10931361477199582),
            Vec3::new(0.5964115907684737, -0.4433814354168595, 0.6691084494494924),
        )
        .unwrap(),
    ));
    let wall = wall();
    let within = Bbox3::from_points([
        Vec3::new(-168.04316601773542, -168.04316601773542, 104.38877107830017),
        Vec3::new(167.06945264388725, 168.0431660177354, 198.75929163002164),
    ]);
    let met = meet_surfaces(&wall, &cap, &within, &tol).unwrap();
    let SurfaceMeeting::Meets(meeting) = met else {
        panic!("the wall and the cap meet: {met:?}");
    };
    let cut = meeting.curves.iter().filter(|c| !c.closed).count();
    assert!(cut > 0, "the box cuts the curve");
    for curve in &meeting.curves {
        assert!(curve.deviation <= tol.linear, "{}", curve.deviation);
        let (t0, t1) = curve.range;
        for k in 0..=2000 {
            let p = curve.curve.eval(t0 + (t1 - t0) * f64::from(k) / 2000.0);
            assert!(within.contains(p, tol.linear), "{p:?} outside the box");
            for surface in [&wall, &cap] {
                let (u, v) = surface.project(p).unwrap();
                let off = surface.eval(u, v).distance(p);
                assert!(off <= tol.linear, "{off} off a surface at {p:?}");
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn nurbs(
    degree_u: usize,
    degree_v: usize,
    knots_u: &[f64],
    knots_v: &[f64],
    count_u: usize,
    count_v: usize,
    ctrl: &[[f64; 3]],
    weights: &[f64],
) -> Surface {
    Surface::Nurbs(
        NurbsSurface::new(
            degree_u,
            degree_v,
            knots_u.to_vec(),
            knots_v.to_vec(),
            count_u,
            count_v,
            ctrl.iter().map(|p| Vec3::new(p[0], p[1], p[2])).collect(),
            weights.to_vec(),
        )
        .unwrap(),
    )
}

/// The lampshade's outer wall: its profile, a cubic spline from the axis
/// at the top to the rim, turned a whole turn (the kernel's revolve).
// (The weights are written as the kernel computed them: 1/√2 where a
// quarter turn has its middle row.)
#[allow(clippy::approx_constant)]
fn wall() -> Surface {
    nurbs(
        2,
        3,
        &[0.0, 0.0, 0.0, 0.5, 0.5, 1.0, 1.0, 1.0],
        &[0.0, 0.0, 0.0, 0.0, 0.25, 0.5, 0.75, 1.0, 1.0, 1.0, 1.0],
        5,
        7,
        &[
            [0.0, 0.0, 250.0],
            [-60.0, 7.34788079488412e-15, 250.0],
            [-128.0, 1.567547902908612e-14, 238.0],
            [-162.0, 1.9839278146187122e-14, 175.0],
            [-160.0, 1.9594348786357652e-14, 95.0],
            [-134.0, 1.6410267108574532e-14, 35.0],
            [-120.0, 1.469576158976824e-14, 0.0],
            [0.0, 0.0, 250.0],
            [-60.000000000000014, -59.99999999999999, 250.0],
            [-128.00000000000003, -127.99999999999999, 238.0],
            [-162.00000000000003, -161.99999999999997, 175.0],
            [-160.00000000000003, -159.99999999999997, 95.0],
            [-134.00000000000003, -133.99999999999997, 35.0],
            [-120.00000000000003, -119.99999999999999, 0.0],
            [0.0, 0.0, 250.0],
            [-1.1021821192326178e-14, -60.0, 250.0],
            [-2.351321854362918e-14, -128.0, 238.0],
            [-2.975891721928068e-14, -162.0, 175.0],
            [-2.9391523179536473e-14, -160.0, 95.0],
            [-2.4615400662861796e-14, -134.0, 35.0],
            [-2.2043642384652355e-14, -120.0, 0.0],
            [0.0, 0.0, 250.0],
            [59.99999999999998, -60.000000000000014, 250.0],
            [127.99999999999996, -128.00000000000003, 238.0],
            [161.99999999999994, -162.00000000000003, 175.0],
            [159.99999999999994, -160.00000000000003, 95.0],
            [133.99999999999994, -134.00000000000003, 35.0],
            [119.99999999999996, -120.00000000000003, 0.0],
            [0.0, 0.0, 250.0],
            [60.0, -1.469576158976824e-14, 250.0],
            [128.0, -3.135095805817224e-14, 238.0],
            [162.0, -3.9678556292374245e-14, 175.0],
            [160.0, -3.9188697572715304e-14, 95.0],
            [134.0, -3.2820534217149064e-14, 35.0],
            [120.0, -2.939152317953648e-14, 0.0],
        ],
        &[
            1.0,
            1.0,
            1.0,
            1.0,
            1.0,
            1.0,
            1.0,
            0.7071067811865476,
            0.7071067811865476,
            0.7071067811865476,
            0.7071067811865476,
            0.7071067811865476,
            0.7071067811865476,
            0.7071067811865476,
            1.0,
            1.0,
            1.0,
            1.0,
            1.0,
            1.0,
            1.0,
            0.7071067811865476,
            0.7071067811865476,
            0.7071067811865476,
            0.7071067811865476,
            0.7071067811865476,
            0.7071067811865476,
            0.7071067811865476,
            1.0,
            1.0,
            1.0,
            1.0,
            1.0,
            1.0,
            1.0,
        ],
    )
}
