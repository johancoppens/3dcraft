//! Draft against closed forms (docs/testing.md, oracle 1): a drafted box
//! is a frustum of a pyramid, a drafted rod a frustum of a cone, a
//! drafted hole widens, a drafted rounded box is a rounded frustum; at
//! three scales and in a tilted frame; what is not drafted is refused
//! with the face and the reason.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod boolean_common;

use boolean_common::{SCALES, assert_close, boxed, mass, rod, solve, tolerance};
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::BooleanOp;
use craft3d_kernel::draft::{Draft, draft};
use craft3d_kernel::fillet::{FilletEdge, fillet};
use craft3d_math::{Transform, Vec3};
use craft3d_topo::{Body, EdgeId, FaceId, check};
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

fn edge_at(body: &Body, k: f64, middle: [f64; 3]) -> EdgeId {
    let target = Vec3::new(middle[0] * k, middle[1] * k, middle[2] * k);
    let (index, _) = body
        .edges
        .iter()
        .enumerate()
        .map(|(i, e)| {
            (
                i,
                e.curve.eval(0.5 * (e.range.0 + e.range.1)).distance(target),
            )
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap();
    EdgeId(index as u32)
}

fn up(faces: Vec<FaceId>, height: f64, degrees: f64, k: f64) -> Draft {
    Draft {
        faces,
        point: Vec3::new(0.0, 0.0, height * k),
        pull: Vec3::Z,
        angle: degrees.to_radians(),
    }
}

#[track_caller]
fn drafted(body: &Body, how: &Draft, k: f64) -> Body {
    let tol = tolerance(k);
    let made = match draft(body, how, &tol) {
        Ok(made) => made,
        Err(e) => panic!("draft at scale {k}: {e}"),
    };
    assert_eq!(check(&made.body, &tol), [], "at scale {k}");
    assert_eq!(made.body.faces.len(), body.faces.len());
    assert_eq!(made.body.edges.len(), body.edges.len());
    made.body
}

fn refused(body: &Body, how: &Draft, k: f64) -> KernelError {
    match draft(body, how, &tolerance(k)) {
        Ok(_) => panic!("a draft at scale {k} was made"),
        Err(e) => e,
    }
}

#[track_caller]
fn assert_volume(body: &Body, volume: f64, k: f64) {
    assert_close(mass(body).volume, volume * k * k * k, "volume");
}

/// The volume of a frustum of a square pyramid.
fn frustum(a: f64, b: f64, h: f64) -> f64 {
    h / 3.0 * (a * a + a * b + b * b)
}

fn sides(body: &Body, k: f64, size: [f64; 3]) -> Vec<FaceId> {
    let [x, y, z] = size;
    [
        [x / 2.0, 0.0, z / 2.0],
        [x / 2.0, y, z / 2.0],
        [0.0, y / 2.0, z / 2.0],
        [x, y / 2.0, z / 2.0],
    ]
    .into_iter()
    .map(|p| face_at(body, k, "plane", p))
    .collect()
}

#[test]
fn a_drafted_block_is_a_frustum_of_a_pyramid() {
    for k in SCALES {
        let block = boxed(k, [0.0, 0.0, 0.0], [10.0, 10.0, 5.0]);
        let four = sides(&block, k, [10.0, 10.0, 5.0]);
        for degrees in [3.0_f64, 10.0, 25.0] {
            let tan = degrees.to_radians().tan();
            // From the bottom: narrower at the top.
            let made = drafted(&block, &up(four.clone(), 0.0, degrees, k), k);
            assert_volume(&made, frustum(10.0, 10.0 - 10.0 * tan, 5.0), k);
            // From the top: wider at the bottom.
            let made = drafted(&block, &up(four.clone(), 5.0, degrees, k), k);
            assert_volume(&made, frustum(10.0 + 10.0 * tan, 10.0, 5.0), k);
            // From the middle, and the other way.
            let made = drafted(&block, &up(four.clone(), 2.5, -degrees, k), k);
            assert_volume(
                &made,
                frustum(10.0 - 5.0 * tan, 10.0, 2.5) + frustum(10.0, 10.0 + 5.0 * tan, 2.5),
                k,
            );
        }
        // One side only: a prism on a trapezium.
        let one = drafted(&block, &up(vec![four[3]], 0.0, 20.0, k), k);
        let tan = 20.0_f64.to_radians().tan();
        assert_volume(&one, 10.0 * 5.0 * (10.0 + 10.0 - 5.0 * tan) / 2.0, k);
        // At 45 degrees from the bottom the top is drawn to a point at
        // height 5: what fits is below that.
        match refused(&block, &up(four.clone(), 0.0, 60.0, k), k) {
            KernelError::FaceOpTooFar { value, largest, .. } => {
                let part = largest / value;
                assert!(part < 45.0 / 60.0 && part > 0.74, "{part}");
            }
            other => panic!("{other}"),
        }
    }
}

#[test]
fn a_drafted_rod_is_a_frustum_of_a_cone_and_a_drafted_hole_widens() {
    for k in SCALES {
        let bar = rod(k, [0.0, 0.0, 0.0], 2.0, 6.0);
        let wall = face_at(&bar, k, "cylinder", [2.0, 0.0, 3.0]);
        let cone = |r1: f64, r2: f64, h: f64| PI * h / 3.0 * (r1 * r1 + r1 * r2 + r2 * r2);
        for degrees in [5.0_f64, 15.0] {
            let tan = degrees.to_radians().tan();
            let made = drafted(&bar, &up(vec![wall], 0.0, degrees, k), k);
            assert_volume(&made, cone(2.0, 2.0 - 6.0 * tan, 6.0), k);
            assert_eq!(made.faces[wall.0 as usize].surface.kind(), "cone");
            let made = drafted(&bar, &up(vec![wall], 6.0, degrees, k), k);
            assert_volume(&made, cone(2.0 + 6.0 * tan, 2.0, 6.0), k);
        }
        // A hole through a block, drafted from the bottom: wider at the
        // top.
        let block = boxed(k, [0.0, 0.0, 0.0], [10.0, 10.0, 5.0]);
        let drill = rod(k, [5.0, 5.0, -1.0], 2.0, 7.0);
        let drilled = solve(&block, &drill, BooleanOp::Subtract, k).body;
        let hole = face_at(&drilled, k, "cylinder", [7.0, 5.0, 2.5]);
        let tan = 10.0_f64.to_radians().tan();
        let made = drafted(&drilled, &up(vec![hole], 0.0, 10.0, k), k);
        assert_volume(&made, 500.0 - cone(2.0, 2.0 + 5.0 * tan, 5.0), k);
        // A rod drawn to a point at a third of its height: 2 / tan = 6
        // at 18.43 degrees.
        match refused(&bar, &up(vec![wall], 0.0, 30.0, k), k) {
            KernelError::FaceOpTooFar { value, largest, .. } => {
                let fits = (2.0_f64 / 6.0).atan().to_degrees() / 30.0;
                let part = largest / value;
                assert!(part < fits && part > 0.98 * fits, "{part} {fits}");
            }
            other => panic!("{other}"),
        }
    }
}

#[test]
fn a_rounded_box_drafted_is_a_rounded_frustum() {
    for k in SCALES {
        let (a, b, h, r) = (10.0, 8.0, 5.0, 1.5);
        let block = boxed(k, [0.0, 0.0, 0.0], [a, b, h]);
        let edges: Vec<FilletEdge> = [[0.0, 0.0], [a, 0.0], [0.0, b], [a, b]]
            .into_iter()
            .map(|[x, y]| FilletEdge {
                edge: edge_at(&block, k, [x, y, h / 2.0]),
                radius: r * k,
            })
            .collect();
        let rounded = fillet(&block, &edges, &tolerance(k)).unwrap().body;
        let walls: Vec<FaceId> = rounded
            .faces
            .iter()
            .enumerate()
            .filter(|(_, face)| match &face.surface {
                craft3d_geom::Surface::Plane(p) => p.frame().z().z.abs() < 0.5,
                craft3d_geom::Surface::Cylinder(_) => true,
                _ => false,
            })
            .map(|(i, _)| FaceId(i as u32))
            .collect();
        assert_eq!(walls.len(), 8);
        let degrees = 8.0_f64;
        let t = degrees.to_radians().tan();
        let made = drafted(&rounded, &up(walls, 0.0, degrees, k), k);
        // The section at height z is the rounded rectangle moved in by
        // z tan, its corners of radius r - z tan.
        let volume = a * b * h - (a + b) * t * h * h + 4.0 / 3.0 * t * t * h * h * h
            - (4.0 - PI) * (r * r * h - r * t * h * h + t * t * h * h * h / 3.0);
        assert_volume(&made, volume, k);
    }
}

#[test]
fn in_a_tilted_frame_the_frustum_is_the_same() {
    for k in SCALES {
        let block = boxed(k, [0.0, 0.0, 0.0], [10.0, 10.0, 5.0]);
        let four = sides(&block, k, [10.0, 10.0, 5.0]);
        let map = Transform::rotation(Vec3::new(1.0, 2.0, 3.0), 0.7)
            .unwrap()
            .then(&Transform::translation(Vec3::new(1.0, -2.0, 0.5) * k));
        let tilted = block.transformed(&map).unwrap();
        let how = Draft {
            faces: four,
            point: map.point(Vec3::ZERO),
            pull: map.vector(Vec3::Z),
            angle: 12.0_f64.to_radians(),
        };
        let made = drafted(&tilted, &how, k);
        let tan = 12.0_f64.to_radians().tan();
        assert_volume(&made, frustum(10.0, 10.0 - 10.0 * tan, 5.0), k);
    }
}

#[test]
fn what_is_not_drafted_is_refused_with_the_face() {
    let k = 1.0;
    let block = boxed(k, [0.0, 0.0, 0.0], [10.0, 10.0, 5.0]);
    let top = face_at(&block, k, "plane", [5.0, 5.0, 5.0]);
    let four = sides(&block, k, [10.0, 10.0, 5.0]);
    // The top does not run along the pull.
    match refused(&block, &up(vec![top], 0.0, 5.0, k), k) {
        KernelError::FaceOp { faces, reason, .. } => {
            assert_eq!(faces, [top]);
            assert!(reason.contains("pull direction"), "{reason}");
        }
        other => panic!("{other}"),
    }
    // A pull across the sides: two of them do not run along it.
    let across = Draft {
        pull: Vec3::X,
        ..up(four.clone(), 0.0, 5.0, k)
    };
    assert!(matches!(
        refused(&block, &across, k),
        KernelError::FaceOp { .. }
    ));
    // A ball.
    let ball = boolean_common::ball(k, [0.0, 0.0, 0.0], 2.0);
    assert!(matches!(
        refused(&ball, &up(vec![FaceId(0)], 0.0, 5.0, k), k),
        KernelError::FaceOp { .. }
    ));
    // A rod lying down: its axis is across the pull.
    let lying = boolean_common::log(k, [0.0, 0.0, 0.0], 2.0, 6.0);
    let wall = face_at(&lying, k, "cylinder", [3.0, 0.0, 2.0]);
    match refused(&lying, &up(vec![wall], 0.0, 5.0, k), k) {
        KernelError::FaceOp { reason, .. } => assert!(reason.contains("cylinder"), "{reason}"),
        other => panic!("{other}"),
    }
    // Bad numbers.
    for degrees in [0.0, 90.0, -95.0, f64::NAN] {
        let error = refused(&block, &up(four.clone(), 0.0, degrees, k), k);
        assert!(
            matches!(
                error,
                KernelError::InvalidSize { .. } | KernelError::FaceOp { .. }
            ),
            "{degrees}: {error}"
        );
    }
    let nowhere = Draft {
        pull: Vec3::ZERO,
        ..up(four.clone(), 0.0, 5.0, k)
    };
    assert!(matches!(
        refused(&block, &nowhere, k),
        KernelError::InvalidSize { .. }
    ));
    assert!(matches!(
        refused(&block, &up(Vec::new(), 0.0, 5.0, k), k),
        KernelError::FaceOp { .. }
    ));
}
