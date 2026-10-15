//! Thicken against closed forms (docs/testing.md, oracle 1): faces of a
//! solid given a thickness outside, inside or on both sides, at three
//! scales and in a tilted frame; every result passes the checker, and
//! both sides of every face are there.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod boolean_common;

use boolean_common::{SCALES, assert_close, boxed, mass, rod, solve, tolerance};
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::BooleanOp;
use craft3d_kernel::shell::Skin;
use craft3d_kernel::thicken::{Side, thicken};
use craft3d_math::{Transform, Vec3};
use craft3d_topo::{Body, FaceId, check};
use std::f64::consts::PI;

/// The face on a surface of `kind` that passes nearest `point` (given for
/// scale 1) and, of those, has a boundary nearest to it.
fn face_at(body: &Body, k: f64, kind: &str, point: [f64; 3]) -> FaceId {
    let target = Vec3::new(point[0] * k, point[1] * k, point[2] * k);
    let (index, _) = body
        .faces
        .iter()
        .enumerate()
        .filter(|(_, face)| face.surface.kind() == kind)
        .map(|(i, face)| {
            let (u, v) = face.surface.project(target).unwrap();
            let off = face.surface.eval(u, v).distance(target);
            let near = face
                .loops
                .iter()
                .flat_map(|l| &body.loop_(*l).unwrap().coedges)
                .map(|c| {
                    let e = body.edge(c.edge).unwrap();
                    e.curve.eval(0.5 * (e.range.0 + e.range.1)).distance(target)
                })
                .fold(0.0_f64, f64::max);
            (i, off * 1e6 + near)
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap();
    FaceId(index as u32)
}

/// The layer, which must be made, checked again here, with both sides
/// of every face given.
#[track_caller]
fn layer(body: &Body, faces: &[FaceId], t: f64, side: Side, k: f64) -> Body {
    let tol = tolerance(k);
    let made = match thicken(body, faces, t * k, side, &tol) {
        Ok(made) => made,
        Err(e) => panic!("thicken {t} {side:?} at scale {k}: {e}"),
    };
    assert_eq!(check(&made.body, &tol), [], "at scale {k}");
    assert_eq!(made.faces.len(), made.body.faces.len());
    for face in faces {
        assert!(made.faces.contains(&(Skin::Outer, *face)));
        assert!(made.faces.contains(&(Skin::Inner, *face)));
    }
    made.body
}

#[track_caller]
fn assert_mass(body: &Body, volume: f64, area: f64, k: f64) {
    let m = mass(body);
    assert_close(m.volume, volume * k * k * k, "volume");
    assert_close(m.area, area * k * k, "area");
}

fn refused(body: &Body, faces: &[FaceId], t: f64, side: Side, k: f64) -> KernelError {
    match thicken(body, faces, t * k, side, &tolerance(k)) {
        Ok(_) => panic!("a layer of {t} {side:?} at scale {k} was made"),
        Err(e) => e,
    }
}

fn block_area(x: f64, y: f64, z: f64) -> f64 {
    2.0 * (x * y + y * z + x * z)
}

#[test]
fn the_top_of_a_block_thickened_is_a_plate_on_it_under_it_or_through_it() {
    for k in SCALES {
        let block = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
        let top = face_at(&block, k, "plane", [2.0, 1.5, 2.0]);
        for side in [Side::Outside, Side::Inside, Side::Both] {
            for t in [0.5, 1.25] {
                let plate = layer(&block, &[top], t, side, k);
                assert_eq!(plate.faces.len(), 6);
                assert_mass(&plate, 12.0 * t, block_area(4.0, 3.0, t), k);
                // Where it lies.
                let lowest = plate
                    .vertices
                    .iter()
                    .map(|v| v.point.z)
                    .fold(f64::INFINITY, f64::min);
                let expected = match side {
                    Side::Outside => 2.0,
                    Side::Inside => 2.0 - t,
                    Side::Both => 2.0 - t / 2.0,
                };
                assert_close(lowest, expected * k, "bottom of the plate");
            }
        }
        // Inward the block is only 2 high; on both sides 4 fits.
        match refused(&block, &[top], 2.5, Side::Inside, k) {
            KernelError::FaceOpTooFar { largest, .. } => {
                assert!(largest > 1.99 * k && largest < 2.0 * k, "{largest}");
            }
            other => panic!("{other}"),
        }
        match refused(&block, &[top], 5.0, Side::Both, k) {
            KernelError::FaceOpTooFar { value, largest, .. } => {
                assert_close(value, 5.0 * k, "value");
                assert!(largest > 3.99 * k && largest < 4.0 * k, "{largest}");
            }
            other => panic!("{other}"),
        }
    }
}

#[test]
fn two_faces_at_a_corner_give_an_angle_and_the_wall_of_a_rod_a_tube() {
    for k in SCALES {
        let block = boxed(k, [0.0, 0.0, 0.0], [10.0, 10.0, 5.0]);
        let top = face_at(&block, k, "plane", [5.0, 5.0, 5.0]);
        let side = face_at(&block, k, "plane", [10.0, 5.0, 2.5]);
        let t = 1.5;
        // Outside: the block grown at two faces, less the block.
        let angle = layer(&block, &[top, side], t, Side::Outside, k);
        assert_mass(
            &angle,
            (10.0 + t) * 10.0 * (5.0 + t) - 500.0,
            // Top and side outside, their undersides, the two ends.
            (10.0 + t) * 10.0
                + 10.0 * (5.0 + t)
                + 100.0
                + 50.0
                + 2.0 * ((10.0 + t) * (5.0 + t) - 50.0)
                + 10.0 * t
                + 10.0 * t,
            k,
        );
        // Inside: an angle in the material.
        let inner = layer(&block, &[top, side], t, Side::Inside, k);
        assert_close(
            mass(&inner).volume,
            (500.0 - (10.0 - t) * 10.0 * (5.0 - t)) * k * k * k,
            "volume",
        );
        // The wall of a rod: a tube as long as the rod.
        let r = 2.0;
        let h = 6.0;
        let bar = rod(k, [0.0, 0.0, 0.0], r, h);
        let wall = face_at(&bar, k, "cylinder", [r, 0.0, 3.0]);
        for (side, inner, outer) in [
            (Side::Outside, r, r + t),
            (Side::Inside, r - t, r),
            (Side::Both, r - t / 2.0, r + t / 2.0),
        ] {
            let tube = layer(&bar, &[wall], t, side, k);
            assert_mass(
                &tube,
                PI * (outer * outer - inner * inner) * h,
                2.0 * PI * (outer + inner) * h + 2.0 * PI * (outer * outer - inner * inner),
                k,
            );
        }
        // Inward by the radius or more there is no tube.
        assert!(matches!(
            refused(&bar, &[wall], 2.5, Side::Inside, k),
            KernelError::FaceOpTooFar { .. }
        ));
    }
}

/// A block of 10 x 10 x 5 with a hole of radius 2 through it.
fn drilled(k: f64) -> Body {
    let block = boxed(k, [0.0, 0.0, 0.0], [10.0, 10.0, 5.0]);
    let drill = rod(k, [5.0, 5.0, -1.0], 2.0, 7.0);
    solve(&block, &drill, BooleanOp::Subtract, k).body
}

#[test]
fn the_wall_of_a_hole_thickened_is_a_sleeve_in_it_or_round_it() {
    for k in SCALES {
        let body = drilled(k);
        let wall = face_at(&body, k, "cylinder", [7.0, 5.0, 2.5]);
        // Outside the material is into the hole.
        for (side, inner, outer) in [
            (Side::Outside, 1.25, 2.0),
            (Side::Inside, 2.0, 2.75),
            (Side::Both, 1.625, 2.375),
        ] {
            let sleeve = layer(&body, &[wall], 0.75, side, k);
            assert_mass(
                &sleeve,
                PI * (outer * outer - inner * inner) * 5.0,
                2.0 * PI * (outer + inner) * 5.0 + 2.0 * PI * (outer * outer - inner * inner),
                k,
            );
        }
        // A sleeve that would fill the hole.
        assert!(matches!(
            refused(&body, &[wall], 2.5, Side::Outside, k),
            KernelError::FaceOpTooFar { .. }
        ));
    }
}

#[test]
fn in_a_tilted_frame_the_plate_is_the_same() {
    for k in SCALES {
        let block = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
        let top = face_at(&block, k, "plane", [2.0, 1.5, 2.0]);
        let map = Transform::rotation(Vec3::new(1.0, 2.0, 3.0), 0.7)
            .unwrap()
            .then(&Transform::translation(Vec3::new(1.0, -2.0, 0.5) * k));
        let tilted = block.transformed(&map).unwrap();
        let plate = layer(&tilted, &[top], 0.75, Side::Both, k);
        assert_mass(&plate, 12.0 * 0.75, block_area(4.0, 3.0, 0.75), k);
    }
}

#[test]
fn what_is_not_a_thickness_is_an_error() {
    let k = 1.0;
    let block = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
    let top = face_at(&block, k, "plane", [2.0, 1.5, 2.0]);
    for t in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(matches!(
            thicken(&block, &[top], t, Side::Outside, &tolerance(k)),
            Err(KernelError::InvalidSize { .. })
        ));
    }
    assert!(matches!(
        refused(&block, &[], 1.0, Side::Outside, k),
        KernelError::FaceOp { .. }
    ));
    assert!(matches!(
        refused(&block, &[top, top], 1.0, Side::Outside, k),
        KernelError::FaceOp { .. }
    ));
    assert!(matches!(
        refused(&block, &[FaceId(6)], 1.0, Side::Outside, k),
        KernelError::FaceOp { .. }
    ));
    assert!(matches!(
        refused(&block, &[top], 1e-9, Side::Both, k),
        KernelError::FaceOp { .. }
    ));
}
