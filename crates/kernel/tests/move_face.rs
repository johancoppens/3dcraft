//! Move face against closed forms (docs/testing.md, oracle 1): every
//! result passes the checker, has the faces, edges and vertices the body
//! had, and the volume and area that follow from the motion, at three
//! scales and in a tilted frame.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_math::Portable;
mod boolean_common;

use boolean_common::{SCALES, assert_close, boxed, mass, rod, solve, tolerance};
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::BooleanOp;
use craft3d_kernel::move_face::{Motion, move_faces};
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

fn shift(x: f64, y: f64, z: f64, k: f64) -> Motion {
    Motion {
        translate: Vec3::new(x * k, y * k, z * k),
        pivot: Vec3::ZERO,
        axis: Vec3::Z,
        angle: 0.0,
    }
}

fn turn(pivot: [f64; 3], axis: Vec3, degrees: f64, k: f64) -> Motion {
    Motion {
        translate: Vec3::ZERO,
        pivot: Vec3::new(pivot[0] * k, pivot[1] * k, pivot[2] * k),
        axis,
        angle: degrees.to_radians(),
    }
}

/// The move, which must succeed, checked again here: the body has what
/// it had.
#[track_caller]
fn moved(body: &Body, faces: &[FaceId], motion: &Motion, k: f64) -> Body {
    let tol = tolerance(k);
    let made = match move_faces(body, faces, motion, &tol) {
        Ok(made) => made,
        Err(e) => panic!("move at scale {k}: {e}"),
    };
    assert_eq!(check(&made.body, &tol), [], "at scale {k}");
    assert_eq!(made.body.faces.len(), body.faces.len());
    assert_eq!(made.body.edges.len(), body.edges.len());
    assert_eq!(made.body.vertices.len(), body.vertices.len());
    made.body
}

#[track_caller]
fn assert_mass(body: &Body, volume: f64, area: f64, k: f64) {
    let m = mass(body);
    assert_close(m.volume, volume * k * k * k, "volume");
    assert_close(m.area, area * k * k, "area");
}

fn refused(body: &Body, faces: &[FaceId], motion: &Motion, k: f64) -> KernelError {
    match move_faces(body, faces, motion, &tolerance(k)) {
        Ok(_) => panic!("a move at scale {k} was made"),
        Err(e) => e,
    }
}

fn block_area(x: f64, y: f64, z: f64) -> f64 {
    2.0 * (x * y + y * z + x * z)
}

#[test]
fn the_top_of_a_block_moved_up_and_down_is_an_offset() {
    for k in SCALES {
        let block = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
        let top = face_at(&block, k, "plane", [2.0, 1.5, 2.0]);
        for d in [0.5, -0.5, 7.0, -1.75] {
            let made = moved(&block, &[top], &shift(0.0, 0.0, d, k), k);
            let h = 2.0 + d;
            assert_mass(&made, 12.0 * h, block_area(4.0, 3.0, h), k);
        }
        // A slanted shift counts by the part along the normal: the top
        // slides along itself.
        let made = moved(&block, &[top], &shift(3.0, -1.0, 1.0, k), k);
        assert_mass(&made, 36.0, block_area(4.0, 3.0, 3.0), k);
        // Down to the floor the block is gone.
        match refused(&block, &[top], &shift(0.0, 0.0, -3.0, k), k) {
            KernelError::FaceOpTooFar { value, largest, .. } => {
                let part = largest / value;
                assert!(part > 0.66 && part < 2.0 / 3.0, "{part}");
            }
            other => panic!("{other}"),
        }
    }
}

#[test]
fn the_top_of_a_block_tilted_about_a_line_across_it_is_a_wedge() {
    for k in SCALES {
        let block = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
        let top = face_at(&block, k, "plane", [2.0, 1.5, 2.0]);
        // About the line along y through the middle of the top: the
        // volume and the sides' areas stay, the top grows.
        for degrees in [10.0, -25.0, 40.0] {
            let made = moved(
                &block,
                &[top],
                &turn([2.0, 1.5, 2.0], Vec3::Y, degrees, k),
                k,
            );
            let cos = degrees.to_radians().pcos();
            assert_mass(&made, 24.0, 12.0 + 12.0 / cos + 16.0 + 12.0, k);
        }
        // About the line along y through one top edge: a wedge whose
        // height at the other side is 2 - 4 tan.
        let degrees: f64 = 20.0;
        let tan = degrees.to_radians().ptan();
        let made = moved(
            &block,
            &[top],
            &turn([0.0, 0.0, 2.0], Vec3::Y, degrees, k),
            k,
        );
        let low = 2.0 - 4.0 * tan;
        let area = 12.0
            + 12.0 / degrees.to_radians().pcos()
            + 2.0 * 4.0 * (2.0 + low) / 2.0
            + 3.0 * 2.0
            + 3.0 * low;
        assert_mass(&made, 3.0 * 4.0 * (2.0 + low) / 2.0, area, k);
        // At 45 degrees about the middle a side shrinks to nothing: what
        // fits is below 45 of the 60 asked.
        match refused(&block, &[top], &turn([2.0, 1.5, 2.0], Vec3::Y, 60.0, k), k) {
            KernelError::FaceOpTooFar {
                value,
                largest,
                edge,
                ..
            } => {
                // The farthest corner, 2 from the line, moves along a
                // chord of its circle; what fits is that part of the turn.
                let chord = 2.0 * 2.0 * (60.0_f64.to_radians() / 2.0).psin();
                assert_close(value, chord * k, "value");
                let part = largest / value;
                assert!(part < 0.75 && part > 0.74, "{part}");
                assert!(edge.is_some());
            }
            other => panic!("{other}"),
        }
    }
}

/// A block of 10 x 10 x 5 with a hole of radius 2 through it.
fn drilled(k: f64) -> Body {
    let block = boxed(k, [0.0, 0.0, 0.0], [10.0, 10.0, 5.0]);
    let drill = rod(k, [5.0, 5.0, -1.0], 2.0, 7.0);
    solve(&block, &drill, BooleanOp::Subtract, k).body
}

fn drilled_mass() -> (f64, f64) {
    let section = 100.0 - PI * 4.0;
    (
        section * 5.0,
        2.0 * section + 40.0 * 5.0 + 2.0 * PI * 2.0 * 5.0,
    )
}

#[test]
fn the_wall_of_a_hole_moved_sideways_moves_the_hole() {
    for k in SCALES {
        let body = drilled(k);
        let wall = face_at(&body, k, "cylinder", [7.0, 5.0, 2.5]);
        for (x, y) in [(1.0, 0.0), (-2.5, 1.5), (0.0, -2.9)] {
            let made = moved(&body, &[wall], &shift(x, y, 0.0, k), k);
            let (volume, area) = drilled_mass();
            assert_mass(&made, volume, area, k);
            // The hole is where it was moved to: its axis is.
            let cylinder = &made.faces[wall.0 as usize].surface;
            let craft3d_geom::Surface::Cylinder(c) = cylinder else {
                panic!("{cylinder:?}")
            };
            let axis = c.frame().origin();
            assert_close(axis.x, (5.0 + x) * k, "axis x");
            assert_close(axis.y, (5.0 + y) * k, "axis y");
        }
        // Up to the side of the block, the hole would leave the face.
        match refused(&body, &[wall], &shift(3.5, 0.0, 0.0, k), k) {
            KernelError::FaceOpTooFar { value, largest, .. } => {
                assert_close(value, 3.5 * k, "value");
                assert!(largest < 3.0 * k && largest > 2.95 * k, "{largest}");
            }
            other => panic!("{other}"),
        }
        // Moving it along its axis changes nothing, and is not refused.
        let made = moved(&body, &[wall], &shift(0.0, 0.0, 1.0, k), k);
        let (volume, area) = drilled_mass();
        assert_mass(&made, volume, area, k);
    }
}

/// A block of 10 x 10 x 5 with a boss of radius 2 and height 3 on it.
fn bossed(k: f64) -> Body {
    let block = boxed(k, [0.0, 0.0, 0.0], [10.0, 10.0, 5.0]);
    let boss = rod(k, [5.0, 5.0, 5.0], 2.0, 3.0);
    solve(&block, &boss, BooleanOp::Union, k).body
}

#[test]
fn a_boss_moved_with_its_top_stands_elsewhere() {
    for k in SCALES {
        let body = bossed(k);
        let wall = face_at(&body, k, "cylinder", [7.0, 5.0, 6.5]);
        let top = face_at(&body, k, "plane", [5.0, 5.0, 8.0]);
        let volume = 500.0 + PI * 4.0 * 3.0;
        let area = block_area(10.0, 10.0, 5.0) + 2.0 * PI * 2.0 * 3.0;
        for (x, y, z) in [(2.0, 1.0, 0.0), (-2.5, 0.0, 1.5), (0.0, 0.0, -2.0)] {
            let made = moved(&body, &[wall, top], &shift(x, y, z, k), k);
            let height = 3.0 + z;
            assert_mass(
                &made,
                500.0 + PI * 4.0 * height,
                block_area(10.0, 10.0, 5.0) + 2.0 * PI * 2.0 * height,
                k,
            );
        }
        // Turned about its own axis it is the same boss.
        let made = moved(
            &body,
            &[wall, top],
            &turn([5.0, 5.0, 0.0], Vec3::Z, 33.0, k),
            k,
        );
        assert_mass(&made, volume, area, k);
        // Off the edge of the block it would hang over the side.
        assert!(matches!(
            refused(&body, &[wall, top], &shift(4.0, 0.0, 0.0, k), k),
            KernelError::FaceOpTooFar { .. }
        ));
    }
}

#[test]
fn a_tilted_cylinder_against_a_flat_face_is_refused_and_not_as_too_far() {
    for k in SCALES {
        let body = drilled(k);
        let wall = face_at(&body, k, "cylinder", [7.0, 5.0, 2.5]);
        match refused(&body, &[wall], &turn([5.0, 5.0, 2.5], Vec3::X, 5.0, k), k) {
            KernelError::FaceOp { faces, reason, .. } => {
                assert!(faces.contains(&wall), "{faces:?}");
                assert!(reason.contains("line or a circle"), "{reason}");
            }
            other => panic!("{other}"),
        }
    }
}

#[test]
fn in_a_tilted_frame_the_wedge_is_the_same() {
    for k in SCALES {
        let block = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
        let top = face_at(&block, k, "plane", [2.0, 1.5, 2.0]);
        let map = Transform::rotation(Vec3::new(1.0, 2.0, 3.0), 0.7)
            .unwrap()
            .then(&Transform::translation(Vec3::new(1.0, -2.0, 0.5) * k));
        let tilted = block.transformed(&map).unwrap();
        let pivot = map.point(Vec3::new(2.0 * k, 1.5 * k, 2.0 * k));
        let motion = Motion {
            translate: map.vector(Vec3::new(0.0, 0.0, 0.5 * k)),
            pivot,
            axis: map.vector(Vec3::Y),
            angle: 15.0_f64.to_radians(),
        };
        let made = moved(&tilted, &[top], &motion, k);
        let cos = 15.0_f64.to_radians().pcos();
        // The turn about the middle keeps the volume; the shift then
        // raises the top by 0.5 along the block's own z, everywhere.
        let h = 2.5;
        assert_mass(
            &made,
            12.0 * h,
            12.0 + 12.0 / cos + 2.0 * 4.0 * h + 2.0 * 3.0 * h,
            k,
        );
    }
}

#[test]
fn what_is_not_a_move_is_an_error() {
    let k = 1.0;
    let block = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
    let top = face_at(&block, k, "plane", [2.0, 1.5, 2.0]);
    let up = shift(0.0, 0.0, 1.0, k);
    assert!(matches!(
        refused(&block, &[], &up, k),
        KernelError::FaceOp { .. }
    ));
    assert!(matches!(
        refused(&block, &[top, top], &up, k),
        KernelError::FaceOp { .. }
    ));
    assert!(matches!(
        refused(&block, &[FaceId(99)], &up, k),
        KernelError::FaceOp { .. }
    ));
    assert!(matches!(
        refused(&block, &[top], &shift(0.0, 0.0, 1e-9, k), k),
        KernelError::FaceOp { .. }
    ));
    for bad in [f64::NAN, f64::INFINITY] {
        assert!(matches!(
            refused(&block, &[top], &shift(bad, 0.0, 0.0, k), k),
            KernelError::InvalidSize { .. }
        ));
        let mut spin = turn([0.0, 0.0, 0.0], Vec3::Z, 10.0, k);
        spin.angle = bad;
        assert!(matches!(
            refused(&block, &[top], &spin, k),
            KernelError::InvalidSize { .. }
        ));
    }
    // A turn about no axis.
    assert!(matches!(
        refused(
            &block,
            &[top],
            &turn([0.0, 0.0, 0.0], Vec3::ZERO, 10.0, k),
            k
        ),
        KernelError::InvalidSize { .. }
    ));
}
