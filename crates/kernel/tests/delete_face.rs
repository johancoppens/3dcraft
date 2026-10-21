//! Delete face against what the body was before the faces were made
//! (docs/testing.md, oracle 1): a block chamfered or filleted and the
//! blend face deleted is the block, a plate drilled, bossed or pocketed
//! and the feature's faces deleted is the plate; volume, area and the
//! counts of faces, edges and vertices agree to 1e-9 at three scales, and
//! every result passes the checker. What is not built is refused with the
//! faces.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod boolean_common;

use boolean_common::{SCALES, assert_close, boxed, log, mass, rod, solve, tolerance};
use craft3d_kernel::boolean::BooleanOp;
use craft3d_kernel::chamfer::{ChamferEdge, ChamferFace, ChamferSize, chamfer};
use craft3d_kernel::delete_face::delete_faces;
use craft3d_kernel::fillet::{FilletEdge, FilletFace, fillet};
use craft3d_math::{Frame, Vec3};
use craft3d_topo::{Body, EdgeId, FaceId, check};
use std::f64::consts::PI;

fn edge_at(body: &Body, k: f64, middle: [f64; 3]) -> EdgeId {
    let target = Vec3::new(middle[0] * k, middle[1] * k, middle[2] * k);
    let (index, _) = body
        .edges
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let p = e.curve.eval(0.5 * (e.range.0 + e.range.1));
            (i, p.distance(target))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap();
    EdgeId(index as u32)
}

fn faces_of(body: &Body, edge: EdgeId) -> Vec<FaceId> {
    (0..body.faces.len() as u32)
        .map(FaceId)
        .filter(|id| {
            body.face(*id)
                .unwrap()
                .loops
                .iter()
                .flat_map(|l| &body.loop_(*l).unwrap().coedges)
                .any(|c| c.edge == edge)
        })
        .collect()
}

/// The faces whose surface passes within the tolerance of `point` (given
/// at scale 1) and are of `kind`.
fn faces_through(body: &Body, k: f64, kind: &str, point: [f64; 3]) -> Vec<FaceId> {
    let target = Vec3::new(point[0] * k, point[1] * k, point[2] * k);
    (0..body.faces.len() as u32)
        .map(FaceId)
        .filter(|id| {
            let face = body.face(*id).unwrap();
            face.surface.kind() == kind && {
                let (u, v) = face.surface.project(target).unwrap();
                face.surface.eval(u, v).distance(target) <= 1e-6 * k
            }
        })
        .collect()
}

fn counts(body: &Body) -> (usize, usize, usize, usize) {
    (
        body.vertices.len(),
        body.edges.len(),
        body.faces.len(),
        body.shells.len(),
    )
}

/// `body` with `faces` deleted, which must succeed, checked and held
/// against `original`: the same volume, area and counts.
#[track_caller]
fn healed_to(body: &Body, faces: &[FaceId], original: &Body, k: f64) -> Body {
    let tol = tolerance(k);
    let made = match delete_faces(body, faces, &tol) {
        Ok(made) => made,
        Err(e) => panic!("delete at scale {k}: {e}"),
    };
    assert_eq!(check(&made.body, &tol), [], "at scale {k}");
    assert_eq!(made.faces.len(), made.body.faces.len());
    assert_eq!(made.edges.len(), made.body.edges.len());
    assert_eq!(made.vertices.len(), made.body.vertices.len());
    assert_eq!(counts(&made.body), counts(original), "at scale {k}");
    let (got, want) = (mass(&made.body), mass(original));
    assert_close(got.volume, want.volume, "volume");
    assert_close(got.area, want.area, "area");
    // No face of the result is one that was deleted.
    for face in &made.faces {
        assert!(!faces.contains(face), "{face:?} is still there");
    }
    made.body
}

#[track_caller]
fn refused(body: &Body, faces: &[FaceId], k: f64) -> String {
    match delete_faces(body, faces, &tolerance(k)) {
        Ok(_) => panic!("deleting {faces:?} at scale {k} was done"),
        Err(e) => e.to_string(),
    }
}

fn filleted(body: &Body, edges: &[EdgeId], r: f64, k: f64) -> (Body, Vec<FaceId>) {
    let request: Vec<FilletEdge> = edges
        .iter()
        .map(|edge| FilletEdge {
            edge: *edge,
            radius: r,
        })
        .collect();
    let tol = tolerance(k);
    let made = fillet(body, &request, &tol).unwrap();
    let blends = made
        .faces
        .iter()
        .enumerate()
        .filter(|(_, f)| matches!(f, FilletFace::Fillet(_)))
        .map(|(i, _)| FaceId(i as u32))
        .collect();
    (made.body, blends)
}

fn chamfered(body: &Body, edge: EdgeId, d: f64, k: f64) -> (Body, FaceId) {
    let first = faces_of(body, edge)[0];
    let made = chamfer(
        body,
        &[ChamferEdge {
            edge,
            first,
            size: ChamferSize::Distances {
                first: d,
                second: d,
            },
        }],
        &tolerance(k),
    )
    .unwrap();
    let (index, _) = made
        .faces
        .iter()
        .enumerate()
        .find(|(_, f)| matches!(f, ChamferFace::Chamfer(_)))
        .unwrap();
    (made.body, FaceId(index as u32))
}

// ------------------------------------------------------- (b) one blend face

#[test]
fn a_fillet_or_a_chamfer_on_an_edge_of_a_block_deleted_is_the_block() {
    for k in SCALES {
        let block = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
        // The top front edge, and a vertical one.
        for middle in [[2.0, 0.0, 2.0], [0.0, 0.0, 1.0]] {
            let edge = edge_at(&block, k, middle);
            let (round, blends) = filleted(&block, &[edge], 0.5 * k, k);
            assert_eq!(blends.len(), 1);
            healed_to(&round, &blends, &block, k);
            let (bevel, face) = chamfered(&block, edge, 0.75 * k, k);
            healed_to(&bevel, &[face], &block, k);
        }
    }
}

#[test]
fn two_fillets_that_do_not_meet_are_deleted_at_once() {
    for k in SCALES {
        let block = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
        let edges = [
            edge_at(&block, k, [2.0, 0.0, 2.0]),
            edge_at(&block, k, [2.0, 3.0, 0.0]),
        ];
        let (round, blends) = filleted(&block, &edges, 0.5 * k, k);
        assert_eq!(blends.len(), 2);
        healed_to(&round, &blends, &block, k);
        // One of them alone leaves the other.
        let one = healed_to_counts(&round, &blends[..1], k);
        assert_eq!(one.faces.len(), 7);
    }
}

fn healed_to_counts(body: &Body, faces: &[FaceId], k: f64) -> Body {
    let made = delete_faces(body, faces, &tolerance(k)).unwrap();
    assert_eq!(check(&made.body, &tolerance(k)), []);
    made.body
}

/// A step: a block with a higher one on its back half. The inside edge
/// where the step's front meets the lower top is concave.
fn step(k: f64) -> Body {
    let low = boxed(k, [0.0, 0.0, 0.0], [4.0, 4.0, 1.0]);
    let high = boxed(k, [0.0, 2.0, 0.0], [4.0, 4.0, 3.0]);
    solve(&low, &high, BooleanOp::Union, k).body
}

#[test]
fn an_inside_fillet_and_an_inside_chamfer_deleted_leave_the_sharp_step() {
    for k in SCALES {
        let stepped = step(k);
        let inside = edge_at(&stepped, k, [2.0, 2.0, 1.0]);
        let (round, blends) = filleted(&stepped, &[inside], 0.5 * k, k);
        // The fillet adds material in the corner.
        assert!(mass(&round).volume > mass(&stepped).volume);
        healed_to(&round, &blends, &stepped, k);
        let (bevel, face) = chamfered(&stepped, inside, 0.5 * k, k);
        healed_to(&bevel, &[face], &stepped, k);
    }
}

#[test]
fn the_round_of_a_rod_s_rim_deleted_is_the_rod() {
    for k in SCALES {
        let bar = rod(k, [0.0, 0.0, 0.0], 2.0, 3.0);
        let rim = edge_at(&bar, k, [-2.0, 0.0, 3.0]);
        let (round, blends) = filleted(&bar, &[rim], 0.5 * k, k);
        let made = healed_to(&round, &blends, &bar, k);
        assert_close(mass(&made).volume, PI * 4.0 * 3.0 * k * k * k, "volume");
        let (bevel, face) = chamfered(&bar, rim, 0.5 * k, k);
        healed_to(&bevel, &[face], &bar, k);
    }
}

/// A plate of 10 x 10 x 2 with a hole of radius 1 through its middle.
fn drilled(k: f64) -> (Body, Body) {
    let plate = boxed(k, [-5.0, -5.0, 0.0], [5.0, 5.0, 2.0]);
    let drill = rod(k, [0.0, 0.0, -1.0], 1.0, 4.0);
    let made = solve(&plate, &drill, BooleanOp::Subtract, k).body;
    (plate, made)
}

#[test]
fn the_round_at_the_mouth_of_a_hole_and_at_the_foot_of_a_boss_deleted() {
    for k in SCALES {
        let (_, plate) = drilled(k);
        let mouth = edge_at(&plate, k, [-1.0, 0.0, 2.0]);
        let (round, blends) = filleted(&plate, &[mouth], 0.25 * k, k);
        healed_to(&round, &blends, &plate, k);
        let (bevel, face) = chamfered(&plate, mouth, 0.25 * k, k);
        healed_to(&bevel, &[face], &plate, k);

        let base = boxed(k, [-5.0, -5.0, 0.0], [5.0, 5.0, 2.0]);
        let stud = rod(k, [0.0, 0.0, 2.0], 1.0, 2.0);
        let bossed = solve(&base, &stud, BooleanOp::Union, k).body;
        let foot = edge_at(&bossed, k, [-1.0, 0.0, 2.0]);
        let (round, blends) = filleted(&bossed, &[foot], 0.25 * k, k);
        healed_to(&round, &blends, &bossed, k);
    }
}

#[test]
fn a_fillet_in_a_tilted_frame_deleted() {
    for k in SCALES {
        let frame = Frame::new(
            Vec3::new(1.0 * k, -2.0 * k, 0.5 * k),
            Vec3::new(1.0, 2.0, 3.0),
            Vec3::new(3.0, -1.0, 0.0),
        )
        .unwrap();
        let block = craft3d_kernel::primitives::block(&frame, 4.0 * k, 3.0 * k, 2.0 * k).unwrap();
        // Every edge in turn.
        for index in [0_u32, 3, 5, 9] {
            let (round, blends) = filleted(&block, &[EdgeId(index)], 0.4 * k, k);
            healed_to(&round, &blends, &block, k);
        }
    }
}

// --------------------------------------------------------- (a) holes filled

#[test]
fn a_hole_through_a_plate_deleted_is_the_plate() {
    for k in SCALES {
        let (plate, made) = drilled(k);
        let wall = faces_through(&made, k, "cylinder", [1.0, 0.0, 1.0]);
        assert_eq!(wall.len(), 1);
        let healed = healed_to(&made, &wall, &plate, k);
        assert_close(mass(&healed).volume, 200.0 * k * k * k, "volume");
    }
}

#[test]
fn a_blind_hole_a_boss_and_a_pocket_deleted_are_the_plate() {
    for k in SCALES {
        let plate = boxed(k, [-5.0, -5.0, 0.0], [5.0, 5.0, 2.0]);
        // A blind hole: the wall and the floor.
        let drill = rod(k, [0.0, 0.0, 1.0], 1.0, 2.0);
        let blind = solve(&plate, &drill, BooleanOp::Subtract, k).body;
        let mut faces = faces_through(&blind, k, "cylinder", [1.0, 0.0, 1.5]);
        faces.extend(faces_through(&blind, k, "plane", [0.0, 0.0, 1.0]));
        assert_eq!(faces.len(), 2);
        healed_to(&blind, &faces, &plate, k);
        // The wall alone leaves a floor that nothing closes.
        let message = refused(&blind, &faces[..1], k);
        assert!(message.contains("delete face"), "{message}");

        // A boss: its wall and top.
        let stud = rod(k, [0.0, 0.0, 2.0], 1.0, 2.0);
        let bossed = solve(&plate, &stud, BooleanOp::Union, k).body;
        let mut faces = faces_through(&bossed, k, "cylinder", [1.0, 0.0, 3.0]);
        faces.extend(faces_through(&bossed, k, "plane", [0.0, 0.0, 4.0]));
        assert_eq!(faces.len(), 2);
        healed_to(&bossed, &faces, &plate, k);
        // Its wall alone: the top cannot reach the plate.
        refused(&bossed, &faces[..1], k);
        // Its top alone: not a hole, not a strip.
        refused(&bossed, &faces[1..], k);

        // A pocket of 4 x 4 x 1: four walls and a floor, in any order.
        let cutter = boxed(k, [-2.0, -2.0, 1.0], [2.0, 2.0, 3.0]);
        let pocketed = solve(&plate, &cutter, BooleanOp::Subtract, k).body;
        let mut faces = Vec::new();
        for p in [
            [0.0, 0.0, 1.0],
            [-2.0, 0.0, 1.5],
            [2.0, 0.0, 1.5],
            [0.0, -2.0, 1.5],
            [0.0, 2.0, 1.5],
        ] {
            faces.extend(faces_through(&pocketed, k, "plane", p));
        }
        faces.reverse();
        assert_eq!(faces.len(), 5);
        healed_to(&pocketed, &faces, &plate, k);
    }
}

#[test]
fn a_cross_hole_in_a_rod_and_a_cavity_deleted() {
    for k in SCALES {
        // A hole across a rod: its wall is bounded by two holes of the
        // rod's round face.
        let bar = rod(k, [0.0, 0.0, 0.0], 2.0, 6.0);
        let across = log(k, [-3.0, 0.0, 3.0], 0.5, 6.0);
        let crossed = solve(&bar, &across, BooleanOp::Subtract, k).body;
        let wall = faces_through(&crossed, k, "cylinder", [0.0, 0.5, 3.0]);
        let wall: Vec<FaceId> = wall
            .into_iter()
            .filter(|f| {
                let face = crossed.face(*f).unwrap();
                matches!(&face.surface, craft3d_geom::Surface::Cylinder(c) if (c.radius() - 0.5 * k).abs() < 1e-9 * k)
            })
            .collect();
        assert!(!wall.is_empty());
        healed_to(&crossed, &wall, &bar, k);

        // A block with a closed cavity: its six faces deleted fill it.
        let block = boxed(k, [0.0, 0.0, 0.0], [4.0, 4.0, 4.0]);
        let inner = boxed(k, [1.0, 1.0, 1.0], [3.0, 3.0, 3.0]);
        let hollow = solve(&block, &inner, BooleanOp::Subtract, k).body;
        assert_eq!(hollow.shells.len(), 2);
        let cavity: Vec<FaceId> = (0..hollow.faces.len() as u32)
            .map(FaceId)
            .filter(|f| {
                let face = hollow.face(*f).unwrap();
                let (u, v) = face
                    .surface
                    .project(Vec3::new(2.0 * k, 2.0 * k, 2.0 * k))
                    .unwrap();
                face.surface
                    .eval(u, v)
                    .distance(Vec3::new(2.0 * k, 2.0 * k, 2.0 * k))
                    < 1.5 * k
            })
            .collect();
        assert_eq!(cavity.len(), 6);
        healed_to(&hollow, &cavity, &block, k);
        // The outside instead leaves the cavity, which is no solid.
        let outside: Vec<FaceId> = (0..hollow.faces.len() as u32)
            .map(FaceId)
            .filter(|f| !cavity.contains(f))
            .collect();
        let message = refused(&hollow, &outside, k);
        assert!(message.contains("no volume"), "{message}");
    }
}

// ------------------------------------------------------------ what is refused

#[test]
fn what_is_not_built_is_refused_with_the_faces() {
    let k = 1.0;
    let block = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
    // A face of a block: its loop is its outline.
    let message = refused(&block, &[FaceId(1)], k);
    assert!(message.contains("cannot be collapsed"), "{message}");
    // Every face, none, one twice, one that is not there.
    let all: Vec<FaceId> = (0..6).map(FaceId).collect();
    assert!(refused(&block, &all, k).contains("no volume"));
    assert!(refused(&block, &[], k).contains("no face"));
    assert!(refused(&block, &[FaceId(1), FaceId(1)], k).contains("twice"));
    assert!(refused(&block, &[FaceId(60)], k).contains("no such face"));

    // Three fillets in a corner: the ball in the corner is no strip, and a
    // fillet next to it ends against it.
    let corner = [
        edge_at(&block, k, [2.0, 0.0, 2.0]),
        edge_at(&block, k, [0.0, 1.5, 2.0]),
        edge_at(&block, k, [0.0, 0.0, 1.0]),
    ];
    let request: Vec<FilletEdge> = corner
        .iter()
        .map(|edge| FilletEdge {
            edge: *edge,
            radius: 0.5,
        })
        .collect();
    let made = fillet(&block, &request, &tolerance(k)).unwrap();
    let ball = made
        .faces
        .iter()
        .position(|f| matches!(f, FilletFace::Corner(_)))
        .unwrap();
    let message = refused(&made.body, &[FaceId(ball as u32)], k);
    assert!(message.contains("cannot close the body"), "{message}");
    let one = made
        .faces
        .iter()
        .position(|f| matches!(f, FilletFace::Fillet(_)))
        .unwrap();
    let message = refused(&made.body, &[FaceId(one as u32)], k);
    assert!(message.contains("tangent"), "{message}");
    // Two fillets that run into each other (a mitre): one alone ends
    // against the other, in a vertex where four faces meet.
    let two = [corner[0], corner[1]];
    let (round, blends) = filleted(&block, &two, 0.5, k);
    let message = refused(&round, &blends[..1], k);
    assert!(message.contains("more than three faces meet"), "{message}");
    // Both at once (since 2026-10-21) are collapsed together: the block
    // (`blends_that_meet_deleted_together_are_the_block`).
    healed_to(&round, &blends, &block, k);
    // The ball without the fillets round it is still refused: the
    // fillets would end against nothing.
    let others: Vec<FaceId> = made
        .faces
        .iter()
        .enumerate()
        .filter(|(_, f)| matches!(f, FilletFace::Fillet(_)))
        .map(|(i, _)| FaceId(i as u32))
        .take(2)
        .collect();
    let mut some = others.clone();
    some.push(FaceId(ball as u32));
    refused(&made.body, &some, k);
}

/// Blends that meet, deleted together (2026-10-21, the general method of
/// a shell as thick as its rounds): every strip's ends and every corner
/// shrink to points, the sides of each strip become one edge, and the
/// faces round them are continued until they meet. Three fillets and
/// the ball in their corner, two fillets in a mitre, the twelve rounds
/// and eight balls of a block rounded all round, three chamfers and the
/// flat corner between them: each time the block, at three scales and
/// turned.
#[test]
fn blends_that_meet_deleted_together_are_the_block() {
    let turn =
        craft3d_math::Transform::rotation(craft3d_math::Vec3::new(1.0, 0.3, -0.6), 0.9).unwrap();
    for k in SCALES {
        for turned in [false, true] {
            let place = |body: Body| {
                if turned {
                    body.transformed(&turn).unwrap()
                } else {
                    body
                }
            };
            let block = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
            let mut all = Vec::new();
            for (x, y) in [(0.0, 0.0), (4.0, 0.0), (0.0, 3.0), (4.0, 3.0)] {
                all.push([x, y, 1.0]);
            }
            for z in [0.0, 2.0] {
                all.extend([[2.0, 0.0, z], [2.0, 3.0, z], [0.0, 1.5, z], [4.0, 1.5, z]]);
            }
            let sets: [Vec<[f64; 3]>; 3] = [
                vec![[2.0, 0.0, 2.0], [0.0, 1.5, 2.0], [0.0, 0.0, 1.0]],
                vec![[2.0, 0.0, 2.0], [0.0, 1.5, 2.0]],
                all,
            ];
            for middles in &sets {
                let edges: Vec<EdgeId> = middles.iter().map(|m| edge_at(&block, k, *m)).collect();
                let request: Vec<FilletEdge> = edges
                    .iter()
                    .map(|edge| FilletEdge {
                        edge: *edge,
                        radius: 0.5 * k,
                    })
                    .collect();
                let made = fillet(&block, &request, &tolerance(k)).unwrap();
                let blends: Vec<FaceId> = made
                    .faces
                    .iter()
                    .enumerate()
                    .filter(|(_, f)| !matches!(f, FilletFace::Kept(_)))
                    .map(|(i, _)| FaceId(i as u32))
                    .collect();
                healed_to(&place(made.body), &blends, &place(block.clone()), k);
            }
            // Three chamfers and the flat triangle in their corner.
            let corner = [[2.0, 0.0, 2.0], [0.0, 1.5, 2.0], [0.0, 0.0, 1.0]];
            let request: Vec<ChamferEdge> = corner
                .iter()
                .map(|m| {
                    let edge = edge_at(&block, k, *m);
                    ChamferEdge {
                        edge,
                        first: faces_of(&block, edge)[0],
                        size: ChamferSize::Distances {
                            first: 0.5 * k,
                            second: 0.5 * k,
                        },
                    }
                })
                .collect();
            let made = chamfer(&block, &request, &tolerance(k)).unwrap();
            let bevels: Vec<FaceId> = made
                .faces
                .iter()
                .enumerate()
                .filter(|(_, f)| !matches!(f, ChamferFace::Kept(_)))
                .map(|(i, _)| FaceId(i as u32))
                .collect();
            healed_to(&place(made.body), &bevels, &place(block.clone()), k);
        }
    }
}

#[test]
fn a_face_between_two_faces_that_never_meet_is_refused() {
    let k = 1.0;
    // A thin wall: its top runs between two parallel faces.
    let wall = boxed(k, [0.0, 0.0, 0.0], [4.0, 0.5, 2.0]);
    let top = faces_through(&wall, k, "plane", [2.0, 0.25, 2.0]);
    let message = refused(&wall, &top, k);
    assert!(message.contains("cannot be collapsed"), "{message}");
}

#[test]
fn a_chamfer_in_a_mitre_or_a_flat_corner_is_refused() {
    let k = 1.0;
    let block = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
    let top_front = edge_at(&block, k, [2.0, 0.0, 2.0]);
    let top_left = edge_at(&block, k, [0.0, 1.5, 2.0]);
    let upright = edge_at(&block, k, [0.0, 0.0, 1.0]);
    let size = ChamferSize::Distances {
        first: 0.5,
        second: 0.5,
    };
    for edges in [
        vec![top_front, top_left],
        vec![top_front, top_left, upright],
    ] {
        let request: Vec<ChamferEdge> = edges
            .iter()
            .map(|edge| ChamferEdge {
                edge: *edge,
                first: faces_of(&block, *edge)[0],
                size,
            })
            .collect();
        let made = chamfer(&block, &request, &tolerance(k)).unwrap();
        for (index, face) in made.faces.iter().enumerate() {
            if !matches!(face, ChamferFace::Kept(_)) {
                let message = refused(&made.body, &[FaceId(index as u32)], k);
                assert!(message.contains("delete face"), "{message}");
            }
        }
    }
}
