//! A face of an operand that is itself a sliver: the tiny triangle a
//! plane leaves where it cuts a corner off a block a few tolerances from
//! the vertex, which an earlier boolean made and the next one has to
//! classify whole (the Garabit pier's finding of 2026-10-20: the 107th
//! union of its bodies met a triangular face of 9 to 15 tolerances a
//! side, inradius 3, that an earlier union had left between three bars).
//! Such a face holds no point at the margin from its boundary, and no arc
//! of it is on the other body, so there is nothing to read it by at its
//! boundary; it lies on one side of the other body, and a point of it
//! closer to its boundary decides. The oracles are closed forms (the
//! volume of a block less a corner tetrahedron, united with, less, and in
//! common with a slanted plate through it), the face counts, both orders,
//! the full check of the body and the boolean's own check, at scales from
//! 1e-3 to 1e6.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_math::Portable;
mod boolean_common;

use boolean_common::*;
use craft3d_kernel::boolean::BooleanOp::{self, Intersect, Subtract, Union};
use craft3d_kernel::boolean::boolean;
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Vec3};
use craft3d_topo::{Body, check, mass_properties};

/// The scales these tests run at (the tolerance scales along).
const AT: [f64; 4] = [1e-3, 1.0, 1e3, 1e6];

/// The thickness of the plate, times `k`.
const PLATE: f64 = 0.2;

#[track_caller]
fn assert_volume(body: &Body, expected: f64, k: f64, what: &str) {
    let m = mass_properties(body).unwrap();
    // The volume is exact up to the integrator's bound and rounding.
    let allowance = 1e-9 * expected.abs() + 2.0 * tolerance(k).linear * m.area;
    assert!(
        (m.volume - expected).abs() <= allowance,
        "{what}: volume {} but expected {expected} (allowance {allowance})",
        m.volume
    );
}

/// The boolean, which must succeed; its result checked in full.
#[track_caller]
fn op(a: &Body, b: &Body, op: BooleanOp, k: f64, what: &str) -> Body {
    let tol = tolerance(k);
    let body = match boolean(a, b, op, &tol) {
        Ok(r) => r.body,
        Err(e) => panic!("{what}: {op:?} at scale {k}: {e}"),
    };
    assert_eq!(check(&body, &tol), [], "{what}: {op:?} at scale {k}");
    verified(a, b, op, &body, &tol, what);
    body
}

/// The block beyond the plane that cuts the corner `(1, 1, 1)` off the
/// unit cube at `delta` (times `k`) from the vertex along each edge.
fn corner_cutter(k: f64, delta: f64) -> Body {
    let n = Vec3::new(1.0, 1.0, 1.0).normalized().unwrap();
    let corner = Vec3::new(k, k, k);
    let on_plane = corner - n * (delta * k / 3.0_f64.sqrt());
    let frame = Frame::from_z(on_plane, n).unwrap();
    let origin = on_plane - frame.x() * k - frame.y() * k;
    primitives::block(&frame.at(origin).unwrap(), 2.0 * k, 2.0 * k, 2.0 * k).unwrap()
}

/// The unit cube (times `k`) less its corner at `delta` (times `k`): a
/// body of seven faces, one of them the triangle the cut leaves, whose
/// sides are `delta √2` long.
fn clipped_cube(k: f64, delta: f64) -> Body {
    let cube = boxed(k, [0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
    let what = format!("the corner cut at {delta} (times {k})");
    let body = op(&cube, &corner_cutter(k, delta), Subtract, k, &what);
    assert_eq!(body.faces.len(), 7, "{what}: faces");
    body
}

/// A plate `PLATE` thick (times `k`) whose middle plane is `z − y = −½`
/// (times `k`), reaching from `x = −1` to `x = 2` and well past the cube
/// along its length: it passes through the cube between the lower half
/// of its far side and its bottom, a quarter of the way from the clipped
/// corner, and its box holds the whole cube.
fn plate(k: f64) -> Body {
    let half = 0.5_f64.sqrt();
    let normal = Vec3::new(0.0, -half, half);
    let along = Vec3::new(0.0, half, half);
    // The slab lies between the signed distances `c` and `c + PLATE`
    // from the origin along the normal, about `−½ / √2`.
    let c = -0.5 * half - 0.5 * PLATE;
    let origin = (Vec3::new(-1.0, 0.0, 0.0) - along + normal * c) * k;
    let frame = Frame::new(origin, normal, Vec3::X).unwrap();
    primitives::block(&frame, 3.0 * k, 3.0 * k, PLATE * k).unwrap()
}

/// The volume the plate and the unit cube have in common, times `k³`: the
/// band `|z − y + ½| ≤ h` of the unit square, `h = PLATE / √2`, has the
/// area `h` (the difference of two corner triangles, `((½ + h)² − (½ −
/// h)²) / 2`), over the cube's full width in `x`.
fn common_volume(k: f64) -> f64 {
    PLATE * 0.5_f64.sqrt() * k.ppowi(3)
}

/// A clipped cube whose triangle is 3, 6 or 9 tolerances along each edge
/// from the vertex (sides of 4.2, 8.5 and 12.7 tolerances; inradii of
/// 1.2, 2.4 and 3.7, under the margin of four) united with, less and in
/// common with a plate through the cube that leaves the triangle alone:
/// the triangle is a whole face of one operand that no point at the
/// margin tells, and is read by a point closer to its boundary. Face
/// counts: the plate's band splits four faces of the cube in two and
/// makes a hole in each of its own two large faces.
#[test]
fn a_face_that_is_a_sliver_is_classified_by_a_point_near_its_boundary() {
    for k in AT {
        let tol = tolerance(k).linear;
        for tolerances in [3.0, 6.0, 9.0] {
            let delta = tolerances * tol / k;
            let a = clipped_cube(k, delta);
            let b = plate(k);
            let what = format!("the triangle at {tolerances} tolerances, scale {k}");
            let cube = k.ppowi(3) - (delta * k).ppowi(3) / 6.0;
            let plate = 3.0 * 3.0 * PLATE * k.ppowi(3);
            let common = common_volume(k);
            for (first, second) in [(&a, &b), (&b, &a)] {
                let union = op(first, second, Union, k, &what);
                assert_eq!(union.shells.len(), 1, "{what}: union shells");
                assert_eq!(union.faces.len(), 17, "{what}: union faces");
                assert_volume(&union, cube + plate - common, k, &what);
                assert!(
                    union.faces.iter().any(|f| f.loops.len() == 1
                        && union.loops[f.loops[0].0 as usize].coedges.len() == 3),
                    "{what}: the union keeps the triangle"
                );
            }
            // The plate cuts the cube in two: the corner with the triangle
            // and the rest.
            let less = op(&a, &b, Subtract, k, &what);
            assert_eq!(less.shells.len(), 2, "{what}: cube less plate shells");
            assert_volume(&less, cube - common, k, &what);
            let rest = op(&b, &a, Subtract, k, &what);
            assert_eq!(rest.shells.len(), 1, "{what}: plate less cube shells");
            assert_volume(&rest, plate - common, k, &what);
            let both = op(&a, &b, Intersect, k, &what);
            assert_eq!(both.shells.len(), 1, "{what}: intersection shells");
            assert_eq!(both.faces.len(), 6, "{what}: intersection faces");
            assert_volume(&both, common, k, &what);
        }
    }
}
