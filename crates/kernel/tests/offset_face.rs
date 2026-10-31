//! Offset face against closed forms (docs/testing.md, oracle 1): every
//! result passes the checker, has the faces, edges and vertices the body
//! had, and the volume and area that follow from the distance, at three
//! scales.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_math::Portable;
mod boolean_common;

use boolean_common::{SCALES, assert_close, ball, boxed, mass, rod, solve, spike, tolerance};
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::BooleanOp;
use craft3d_kernel::chamfer::{ChamferEdge, ChamferSize, chamfer};
use craft3d_kernel::fillet::{FilletEdge, fillet};
use craft3d_kernel::offset_face::{FaceOffset, offset_faces};
use craft3d_math::Vec3;
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

/// The offset, which must succeed, checked again here: the body has what
/// it had.
#[track_caller]
fn moved(body: &Body, face: FaceId, distance: f64, k: f64) -> Body {
    let tol = tolerance(k);
    let offsets = [FaceOffset {
        face,
        distance: distance * k,
    }];
    let made = match offset_faces(body, &offsets, &tol) {
        Ok(made) => made,
        Err(e) => panic!("offset at scale {k}: {e}"),
    };
    assert_eq!(check(&made.body, &tol), [], "at scale {k}");
    assert_eq!(made.body.faces.len(), body.faces.len());
    assert_eq!(made.body.edges.len(), body.edges.len());
    assert_eq!(made.body.vertices.len(), body.vertices.len());
    assert_eq!(made.faces.len(), body.faces.len());
    made.body
}

#[track_caller]
fn assert_mass(body: &Body, volume: f64, area: f64, k: f64) {
    let m = mass(body);
    assert_close(m.volume, volume * k * k * k, "volume");
    assert_close(m.area, area * k * k, "area");
}

fn refused(body: &Body, face: FaceId, distance: f64, k: f64) -> KernelError {
    let offsets = [FaceOffset {
        face,
        distance: distance * k,
    }];
    match offset_faces(body, &offsets, &tolerance(k)) {
        Ok(_) => panic!("an offset of {distance} at scale {k} was made"),
        Err(e) => e,
    }
}

fn block_area(x: f64, y: f64, z: f64) -> f64 {
    2.0 * (x * y + y * z + x * z)
}

#[test]
fn the_top_of_a_block_moves_out_and_in_and_the_block_has_six_faces() {
    for k in SCALES {
        let block = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
        let top = face_at(&block, k, "plane", [2.0, 1.5, 2.0]);
        for d in [0.5, -0.5, 7.0, -1.75] {
            let made = moved(&block, top, d, k);
            assert_eq!(made.faces.len(), 6);
            let h = 2.0 + d;
            assert_mass(&made, 12.0 * h, block_area(4.0, 3.0, h), k);
        }
    }
}

#[test]
fn a_block_cannot_lose_its_height_and_the_error_says_what_fits() {
    for k in SCALES {
        let block = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
        let top = face_at(&block, k, "plane", [2.0, 1.5, 2.0]);
        for d in [-2.0, -3.0] {
            match refused(&block, top, d, k) {
                KernelError::FaceOpTooFar {
                    edge,
                    value,
                    largest,
                    ..
                } => {
                    // One of the four upright edges is named.
                    let edge = block.edge(edge.unwrap()).unwrap();
                    let (a, b) = (
                        block.vertex(edge.start).unwrap().point,
                        block.vertex(edge.end).unwrap().point,
                    );
                    assert!((a.z - b.z).abs() > k, "{a:?} {b:?}");
                    assert_close(value, -d * k, "value");
                    assert!(largest < 2.0 * k && largest > 1.99 * k, "{largest}");
                }
                other => panic!("{other}"),
            }
        }
    }
}

/// A block of 10 x 10 x 5 with a pocket of 4 x 4 x 3 in its top.
fn pocketed(k: f64) -> Body {
    let block = boxed(k, [0.0, 0.0, 0.0], [10.0, 10.0, 5.0]);
    let pocket = boxed(k, [3.0, 3.0, 2.0], [7.0, 7.0, 6.0]);
    solve(&block, &pocket, BooleanOp::Subtract, k).body
}

#[test]
fn the_floor_of_a_pocket_moves_and_its_walls_follow() {
    for k in SCALES {
        let body = pocketed(k);
        let floor = face_at(&body, k, "plane", [5.0, 5.0, 2.0]);
        for d in [1.0, -1.0, 2.5] {
            let made = moved(&body, floor, d, k);
            let depth = 3.0 - d;
            assert_mass(
                &made,
                500.0 - 16.0 * depth,
                block_area(10.0, 10.0, 5.0) + 16.0 * depth,
                k,
            );
        }
        // Up to the top of the block the pocket is gone, and through the
        // bottom the floor is: neither is a body.
        for d in [3.0, 3.5, -2.0, -4.0] {
            assert!(
                matches!(
                    refused(&body, floor, d, k),
                    KernelError::FaceOpTooFar { .. }
                ),
                "{d}"
            );
        }
    }
}

/// A block of 10 x 10 x 5 with a hole of radius 2 through it.
fn drilled(k: f64) -> Body {
    let block = boxed(k, [0.0, 0.0, 0.0], [10.0, 10.0, 5.0]);
    let drill = rod(k, [5.0, 5.0, -1.0], 2.0, 7.0);
    solve(&block, &drill, BooleanOp::Subtract, k).body
}

fn drilled_mass(radius: f64, height: f64) -> (f64, f64) {
    let section = 100.0 - PI * radius * radius;
    (
        section * height,
        2.0 * section + 40.0 * height + 2.0 * PI * radius * height,
    )
}

#[test]
fn a_hole_through_a_face_that_moves_gets_longer() {
    for k in SCALES {
        let body = drilled(k);
        let top = face_at(&body, k, "plane", [5.0, 5.0, 5.0]);
        for d in [1.0, -2.0] {
            let made = moved(&body, top, d, k);
            let (volume, area) = drilled_mass(2.0, 5.0 + d);
            assert_mass(&made, volume, area, k);
        }
    }
}

#[test]
fn the_wall_of_a_hole_moves_into_the_material_and_out_of_it() {
    for k in SCALES {
        let body = drilled(k);
        let wall = face_at(&body, k, "cylinder", [7.0, 5.0, 2.5]);
        // Outward is away from the material: into the hole, which narrows.
        for d in [-1.0, 1.0, -2.5, 1.5] {
            let made = moved(&body, wall, d, k);
            let (volume, area) = drilled_mass(2.0 - d, 5.0);
            assert_mass(&made, volume, area, k);
        }
        // A hole of no radius, and one wider than the block.
        match refused(&body, wall, 2.0, k) {
            KernelError::FaceOpTooFar { faces, largest, .. } => {
                assert!(faces.contains(&wall));
                assert!(largest > 1.99 * k && largest < 2.0 * k, "{largest}");
            }
            other => panic!("{other}"),
        }
        match refused(&body, wall, -3.5, k) {
            KernelError::FaceOpTooFar { largest, .. } => {
                // The hole reaches the sides of the block at radius 5.
                assert!(largest > 2.99 * k && largest < 3.0 * k, "{largest}");
            }
            other => panic!("{other}"),
        }
    }
}

/// A block of 10 x 10 x 5 with a boss of radius 2 and height 3 on it.
fn bossed(k: f64) -> Body {
    let block = boxed(k, [0.0, 0.0, 0.0], [10.0, 10.0, 5.0]);
    let boss = rod(k, [5.0, 5.0, 5.0], 2.0, 3.0);
    solve(&block, &boss, BooleanOp::Union, k).body
}

#[test]
fn the_wall_of_a_boss_grows_and_shrinks() {
    for k in SCALES {
        let body = bossed(k);
        let wall = face_at(&body, k, "cylinder", [7.0, 5.0, 6.5]);
        for d in [0.5, -1.0, 2.9] {
            let made = moved(&body, wall, d, k);
            let r = 2.0 + d;
            assert_mass(
                &made,
                500.0 + PI * r * r * 3.0,
                block_area(10.0, 10.0, 5.0) + 2.0 * PI * r * 3.0,
                k,
            );
        }
        assert!(matches!(
            refused(&body, wall, 3.5, k),
            KernelError::FaceOpTooFar { .. }
        ));
        assert!(matches!(
            refused(&body, wall, -2.0, k),
            KernelError::FaceOpTooFar { .. }
        ));
    }
}

#[test]
fn the_face_a_boss_stands_on_moves_and_the_boss_gets_shorter() {
    for k in SCALES {
        let body = bossed(k);
        let top = face_at(&body, k, "plane", [1.0, 1.0, 5.0]);
        for d in [1.0, -2.0] {
            let made = moved(&body, top, d, k);
            let (h, b) = (5.0 + d, 3.0 - d);
            assert_mass(
                &made,
                100.0 * h + PI * 4.0 * b,
                block_area(10.0, 10.0, h) + 2.0 * PI * 2.0 * b,
                k,
            );
        }
        // Past the top of the boss there is no boss.
        match refused(&body, top, 4.0, k) {
            KernelError::FaceOpTooFar { largest, .. } => {
                assert!(largest > 2.99 * k && largest < 3.0 * k, "{largest}");
            }
            other => panic!("{other}"),
        }
    }
}

#[test]
fn a_ball_half_sunk_in_a_block_grows_and_a_ball_on_its_own_too() {
    for k in SCALES {
        let block = boxed(k, [0.0, 0.0, 0.0], [10.0, 10.0, 5.0]);
        let body = solve(&block, &ball(k, [5.0, 5.0, 5.0], 2.0), BooleanOp::Union, k).body;
        let dome = face_at(&body, k, "sphere", [5.0, 5.0, 7.0]);
        for d in [0.5, -1.0] {
            let made = moved(&body, dome, d, k);
            let r = 2.0 + d;
            assert_mass(
                &made,
                500.0 + 2.0 / 3.0 * PI * r * r * r,
                block_area(10.0, 10.0, 5.0) + PI * r * r,
                k,
            );
        }
        let alone = ball(k, [1.0, 2.0, 3.0], 2.0);
        let made = moved(&alone, FaceId(0), 1.0, k);
        assert_mass(&made, 36.0 * PI, 36.0 * PI, k);
        assert!(matches!(
            refused(&alone, FaceId(0), -2.0, k),
            KernelError::FaceOpTooFar { .. }
        ));
    }
}

#[test]
fn a_round_hollow_in_a_block_is_widened() {
    for k in SCALES {
        let block = boxed(k, [0.0, 0.0, 0.0], [10.0, 10.0, 5.0]);
        let body = solve(
            &block,
            &ball(k, [5.0, 5.0, 5.0], 2.0),
            BooleanOp::Subtract,
            k,
        )
        .body;
        let bowl = face_at(&body, k, "sphere", [5.0, 5.0, 3.0]);
        // Into the material: the bowl gets wider.
        let made = moved(&body, bowl, -1.0, k);
        let r: f64 = 3.0;
        assert_mass(
            &made,
            500.0 - 2.0 / 3.0 * PI * r.ppowi(3),
            block_area(10.0, 10.0, 5.0) + PI * r * r,
            k,
        );
    }
}

#[test]
fn the_side_of_a_cone_moves_and_keeps_its_angle() {
    for k in SCALES {
        let body = spike(k, [0.0, 0.0, 0.0], 3.0, 4.0);
        let side = face_at(&body, k, "cone", [1.5, 0.0, 2.0]);
        for d in [0.6, -1.2] {
            let made = moved(&body, side, d, k);
            // The slant is 5 for a radius of 3 and a height of 4: the
            // radius at the base changes by d over the cosine, 4/5.
            let r = 3.0 + d * 5.0 / 4.0;
            let h = r * 4.0 / 3.0;
            let slant = r * 5.0 / 3.0;
            assert_mass(&made, PI * r * r * h / 3.0, PI * r * r + PI * r * slant, k);
        }
    }
}

/// A block of 10 x 10 x 5 whose four top edges are chamfered 2 along the
/// top and 3 down the sides: a frustum of a pyramid on a plinth.
fn tapered(k: f64) -> Body {
    let block = boxed(k, [0.0, 0.0, 0.0], [10.0, 10.0, 5.0]);
    let top = face_at(&block, k, "plane", [5.0, 5.0, 5.0]);
    let edges: Vec<ChamferEdge> = [
        [5.0, 0.0, 5.0],
        [5.0, 10.0, 5.0],
        [0.0, 5.0, 5.0],
        [10.0, 5.0, 5.0],
    ]
    .into_iter()
    .map(|middle| ChamferEdge {
        edge: edge_at(&block, k, middle),
        first: top,
        size: ChamferSize::Distances {
            first: 2.0 * k,
            second: 3.0 * k,
        },
    })
    .collect();
    chamfer(&block, &edges, &tolerance(k)).unwrap().body
}

/// The volume of a frustum of a square pyramid.
fn frustum(a: f64, b: f64, h: f64) -> f64 {
    h / 3.0 * (a * a + a * b + b * b)
}

#[test]
fn the_top_of_a_frustum_moves_and_the_slanted_sides_are_continued() {
    for k in SCALES {
        let body = tapered(k);
        assert_eq!(body.faces.len(), 10);
        let top = face_at(&body, k, "plane", [5.0, 5.0, 5.0]);
        // The plinth is 2 high; above it the side of the square shrinks
        // by 4/3 per unit of height, from 10 at height 2.
        let side = |z: f64| 10.0 - (z - 2.0) * 4.0 / 3.0;
        for d in [1.5, -1.0, 4.0, -2.9] {
            let made = moved(&body, top, d, k);
            assert_eq!(made.faces.len(), 10);
            let z = 5.0 + d;
            let volume = 200.0 + frustum(10.0, side(z), z - 2.0);
            // Four trapezia whose slant height is the height times
            // sqrt(13)/3.
            let slant = (z - 2.0) * 13.0_f64.sqrt() / 3.0;
            let area = 100.0 + 80.0 + side(z) * side(z) + 4.0 * 0.5 * (10.0 + side(z)) * slant;
            assert_mass(&made, volume, area, k);
        }
        // The sides meet in a point 7.5 above the plinth: the top is gone
        // 4.5 above where it is. Down, it reaches the plinth after 3.
        for (d, fits) in [(5.0, 4.5), (-3.5, 3.0)] {
            match refused(&body, top, d, k) {
                KernelError::FaceOpTooFar { largest, edge, .. } => {
                    assert!(edge.is_some());
                    assert!(
                        largest > (fits - 0.01) * k && largest < fits * k,
                        "{largest}"
                    );
                }
                other => panic!("{other}"),
            }
        }
    }
}

#[test]
fn a_chamfer_face_moves_and_its_legs_follow() {
    for k in SCALES {
        let block = boxed(k, [0.0, 0.0, 0.0], [10.0, 10.0, 5.0]);
        let edge = edge_at(&block, k, [5.0, 0.0, 5.0]);
        let cut = chamfer(
            &block,
            &[ChamferEdge {
                edge,
                first: faces_of(&block, edge)[0],
                size: ChamferSize::Distances {
                    first: 2.0 * k,
                    second: 2.0 * k,
                },
            }],
            &tolerance(k),
        )
        .unwrap()
        .body;
        assert_eq!(cut.faces.len(), 7);
        let slope = face_at(&cut, k, "plane", [5.0, 1.0, 4.0]);
        for d in [-0.5, 0.75] {
            let made = moved(&cut, slope, d, k);
            assert_eq!(made.faces.len(), 7);
            // The legs of the triangle that is cut off change by the
            // distance times sqrt(2).
            let leg = 2.0 - d * 2.0_f64.sqrt();
            let volume = 500.0 - 10.0 * 0.5 * leg * leg;
            let area = block_area(10.0, 10.0, 5.0) - 2.0 * 10.0 * leg - leg * leg
                + 10.0 * leg * 2.0_f64.sqrt();
            assert_mass(&made, volume, area, k);
        }
        // Outward by sqrt(2) the chamfer is the edge again: no face.
        match refused(&cut, slope, 1.5, k) {
            KernelError::FaceOpTooFar { largest, .. } => {
                let fits = 2.0_f64.sqrt();
                assert!(
                    largest > (fits - 0.01) * k && largest < fits * k,
                    "{largest}"
                );
            }
            other => panic!("{other}"),
        }
    }
}

#[test]
fn a_face_at_a_vertex_where_four_faces_meet_is_refused_with_the_four() {
    for k in SCALES {
        let body = tapered(k);
        // A slanted side: at both its lower corners it meets the other
        // slanted side and two walls of the plinth. Moved, the four no
        // longer share a point: the corner would need an edge it has not.
        let slope = face_at(&body, k, "plane", [5.0, 1.0, 3.5]);
        for d in [-0.5, 0.5] {
            match refused(&body, slope, d, k) {
                KernelError::FaceOp { faces, .. } => {
                    assert_eq!(faces.len(), 4);
                    assert!(faces.contains(&slope));
                }
                other => panic!("{other}"),
            }
        }
    }
}

#[test]
fn several_faces_move_at_once() {
    for k in SCALES {
        let block = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
        let tol = tolerance(k);
        let offsets: Vec<FaceOffset> = (0..6)
            .map(|i| FaceOffset {
                face: FaceId(i),
                distance: 0.5 * k,
            })
            .collect();
        let made = offset_faces(&block, &offsets, &tol).unwrap().body;
        assert_eq!(check(&made, &tol), []);
        assert_mass(&made, 5.0 * 4.0 * 3.0, block_area(5.0, 4.0, 3.0), k);
        // Inward by half the smallest size nothing is left.
        let inward: Vec<FaceOffset> = offsets
            .iter()
            .map(|o| FaceOffset {
                face: o.face,
                distance: -k,
            })
            .collect();
        match offset_faces(&block, &inward, &tol) {
            Err(KernelError::FaceOpTooFar { largest, .. }) => {
                assert!(largest > 0.99 * k && largest < k, "{largest}");
            }
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn a_face_next_to_a_fillet_is_refused_and_not_as_too_far() {
    for k in SCALES {
        let block = boxed(k, [0.0, 0.0, 0.0], [10.0, 10.0, 5.0]);
        let edge = edge_at(&block, k, [5.0, 0.0, 5.0]);
        let rounded = fillet(
            &block,
            &[FilletEdge {
                edge,
                radius: 1.0 * k,
            }],
            &tolerance(k),
        )
        .unwrap()
        .body;
        let top = face_at(&rounded, k, "plane", [5.0, 5.0, 5.0]);
        // The fillet is tangent to the top: its continuation does not
        // meet the moved plane in the line it ends in.
        for d in [0.5, -0.5] {
            match refused(&rounded, top, d, k) {
                KernelError::FaceOp { faces, .. } => assert!(!faces.is_empty()),
                other => panic!("{other}"),
            }
        }
        // The fillet face itself has both neighbours tangent: the same.
        let round = face_at(&rounded, k, "cylinder", [5.0, 0.3, 4.7]);
        assert!(matches!(
            refused(&rounded, round, 0.25, k),
            KernelError::FaceOp { .. }
        ));
    }
}

#[test]
fn what_is_not_a_move_is_an_error() {
    let k = 1.0;
    let block = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
    let tol = tolerance(k);
    let one = |face: u32, distance: f64| {
        offset_faces(
            &block,
            &[FaceOffset {
                face: FaceId(face),
                distance,
            }],
            &tol,
        )
    };
    assert!(matches!(one(99, 1.0), Err(KernelError::FaceOp { .. })));
    assert!(matches!(one(0, 0.0), Err(KernelError::FaceOp { .. })));
    assert!(matches!(one(0, 1e-9), Err(KernelError::FaceOp { .. })));
    assert!(matches!(
        one(0, f64::NAN),
        Err(KernelError::InvalidSize { .. })
    ));
    assert!(matches!(
        one(0, f64::INFINITY),
        Err(KernelError::InvalidSize { .. })
    ));
    assert!(matches!(
        offset_faces(&block, &[], &tol),
        Err(KernelError::FaceOp { .. })
    ));
    let twice = [
        FaceOffset {
            face: FaceId(1),
            distance: 1.0,
        },
        FaceOffset {
            face: FaceId(1),
            distance: 1.0,
        },
    ];
    assert!(matches!(
        offset_faces(&block, &twice, &tol),
        Err(KernelError::FaceOp { .. })
    ));
}
