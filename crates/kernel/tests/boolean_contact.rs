//! Booleans of bodies that touch without overlapping, and the merging of
//! what a boolean splits needlessly (docs/testing.md, oracles 1 and 2).
//!
//! The rules under test, per operation, for bodies with no volume in
//! common: the union is both bodies as shells that share nothing, the
//! intersection is empty, the difference is the first body with the
//! counts it had.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod boolean_common;

use boolean_common::*;
use craft3d_kernel::boolean::{
    BooleanOp, BooleanOptions, EdgeSource, FaceSource, Operand, VertexSource, boolean_with,
};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Vec3};
use craft3d_topo::{Body, FaceId, VertexId, genus};
use std::f64::consts::PI;

use BooleanOp::{Intersect, Subtract, Union};

fn body_counts(body: &Body) -> (usize, usize, usize, usize) {
    (
        body.vertices.len(),
        body.edges.len(),
        body.faces.len(),
        body.shells.len(),
    )
}

/// Two bodies that only touch: the three rules, either way round.
#[track_caller]
fn assert_only_touching(a: &Body, b: &Body, k: f64) {
    let (ma, mb) = (mass(a), mass(b));
    let (ca, cb) = (body_counts(a), body_counts(b));
    for (x, y, mx, my, cx, cy) in [(a, b, ma, mb, ca, cb), (b, a, mb, ma, cb, ca)] {
        let both = solve(x, y, Union, k);
        let m = mass(&both.body);
        assert_close(m.volume, mx.volume + my.volume, "volume of the union");
        assert_close(m.area, mx.area + my.area, "area of the union");
        // Nothing is shared and nothing is left of the contact.
        assert_eq!(
            counts(&both),
            (cx.0 + cy.0, cx.1 + cy.1, cx.2 + cy.2, cx.3 + cy.3)
        );
        assert_empty(x, y, Intersect, k);
        let same = solve(x, y, Subtract, k);
        let m = mass(&same.body);
        assert_close(m.volume, mx.volume, "volume of the difference");
        assert_close(m.area, mx.area, "area of the difference");
        assert_eq!(counts(&same), cx);
        assert!(same.faces.iter().all(|f| f.operand == Operand::A));
    }
}

#[test]
fn faces_that_touch_along_a_line_or_at_a_point() {
    for k in SCALES {
        let plate = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
        // A cylinder lying on the plate, within it and sticking out over
        // its edge.
        assert_only_touching(&plate, &log(k, [1.0, 1.5, 2.5], 0.5, 2.0), k);
        assert_only_touching(&plate, &log(k, [3.0, 1.5, 2.5], 0.5, 2.0), k);
        // A cylinder standing against a wall, shorter and longer than it.
        let upright = rod(k, [0.0, 0.0, -2.0], 1.0, 4.0);
        assert_only_touching(&upright, &boxed(k, [1.0, -3.0, -1.0], [2.0, 3.0, 1.0]), k);
        assert_only_touching(&upright, &boxed(k, [1.0, -3.0, -3.0], [2.0, 3.0, 5.0]), k);
        // Kissing cylinders, side by side and one lying across the other.
        assert_only_touching(&upright, &rod(k, [2.0, 0.0, -1.0], 1.0, 4.0), k);
        // A ball on the plate, a ball against a ball, a ball in the
        // hollow between the plate and a lying cylinder is too much to
        // ask of the stand-in; a cone standing on its tip is not.
        assert_only_touching(&plate, &ball(k, [2.0, 1.5, 3.0], 1.0), k);
        assert_only_touching(
            &ball(k, [0.0, 0.0, 0.0], 1.0),
            &ball(k, [1.5, 2.0, 0.0], 1.5),
            k,
        );
        assert_only_touching(&plate, &spike(k, [2.0, 1.5, 3.5], 1.0, -1.5), k);
        // A ring lying on the plate: along a whole circle.
        assert_only_touching(&plate, &ring(k, [2.0, 1.5, 2.25], 0.75, 0.25), k);
        // A ball inside a ring that fits it exactly.
        assert_only_touching(
            &ring(k, [0.0, 0.0, 0.0], 2.0, 0.5),
            &ball(k, [0.0, 0.0, 0.0], 1.5),
            k,
        );
    }
}

#[test]
fn bodies_that_touch_along_an_edge_or_at_a_vertex() {
    for k in SCALES {
        let plate = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
        // Boxes that share one edge, wholly and in part; one vertex; an
        // edge of one in a face of the other; a vertex in a face.
        assert_only_touching(&plate, &boxed(k, [4.0, 3.0, 0.0], [5.0, 4.0, 2.0]), k);
        assert_only_touching(&plate, &boxed(k, [4.0, 3.0, 1.0], [5.0, 4.0, 3.0]), k);
        assert_only_touching(&plate, &boxed(k, [4.0, 3.0, 2.0], [5.0, 4.0, 3.0]), k);
        let turned = Frame::new(
            Vec3::new(2.0, 0.5, 2.0) * k,
            Vec3::new(1.0, 0.0, 1.0),
            Vec3::Y,
        )
        .unwrap();
        let on_edge = primitives::block(&turned, k, 2.0 * k, k).unwrap();
        assert_only_touching(&plate, &on_edge, k);
        let on_tip = spike(k, [2.0, 1.5, 3.5], 1.0, -1.5);
        assert_only_touching(&plate, &on_tip, k);
        // The rim of a cylinder against the plate's edge: a point.
        assert_only_touching(&plate, &rod(k, [5.0, 1.5, 0.5], 1.0, 1.0), k);
    }
}

#[test]
fn a_result_whose_parts_hang_together_by_an_edge() {
    for k in SCALES {
        // Two opposite quarters cut out of a square plate: what is left
        // are two blocks that share one upright edge. Each gets its own.
        let plate = boxed(k, [0.0, 0.0, 0.0], [2.0, 2.0, 1.0]);
        let first = boxed(k, [1.0, -1.0, -1.0], [3.0, 1.0, 2.0]);
        let second = boxed(k, [-1.0, 1.0, -1.0], [1.0, 3.0, 2.0]);
        let ell = solve(&plate, &first, Subtract, k);
        assert_eq!(counts(&ell), (12, 18, 8, 1));
        let two = solve(&ell.body, &second, Subtract, k);
        assert_mass(&two, 2.0, 12.0, k);
        assert_eq!(counts(&two), (16, 24, 12, 2));
        assert_eq!(genus(&two.body), Some(0));
        // The shared corner (1, 1) is there four times: top and bottom
        // of each block.
        let at_corner = two
            .body
            .vertices
            .iter()
            .filter(|v| v.point.distance(Vec3::new(k, k, v.point.z)) < 1e-9 * k)
            .count();
        assert_eq!(at_corner, 4);
        // In one cut, by a cutter that is itself two blocks joined along
        // that edge.
        let cutter = solve(&first, &second, Union, k);
        assert_eq!(cutter.body.shells.len(), 2);
        let again = solve(&plate, &cutter.body, Subtract, k);
        assert_mass(&again, 2.0, 12.0, k);
        assert_eq!(counts(&again), (16, 24, 12, 2));
        // Put back where it came from: the cutter and the two blocks fill
        // each other's gaps (2 + 2 x 12 in volume) and are one body.
        let whole = solve(&two.body, &cutter.body, Union, k);
        assert_close(mass(&whole.body).volume, 26.0 * k * k * k, "volume");
        assert_eq!(whole.body.shells.len(), 1);
    }
}

#[test]
fn a_hole_that_touches_the_side_and_a_cylinder_that_fills_its_block() {
    for k in SCALES {
        // A hole through the plate whose wall touches the plate's side
        // along a line: its rims meet the plate's edges in a point each,
        // tangentially.
        let plate = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
        let drill = rod(k, [1.0, 1.5, -1.0], 1.0, 4.0);
        let holed = solve(&plate, &drill, Subtract, k);
        assert_mass(&holed, 24.0 - 2.0 * PI, 52.0 - 2.0 * PI + 4.0 * PI, k);
        assert_eq!(holed.body.shells.len(), 1);
        let plug = solve(&plate, &drill, Intersect, k);
        assert_mass(&plug, 2.0 * PI, 2.0 * PI + 4.0 * PI, k);
        assert_eq!(counts(&plug), (2, 2, 3, 1));
        let both = solve(&plate, &drill, Union, k);
        assert_mass(
            &both,
            24.0 + 2.0 * PI,
            52.0 - 2.0 * PI + 4.0 * PI + 2.0 * PI,
            k,
        );
        // A cylinder that touches all four sides of its block: taking it
        // out leaves the four corners, each a body of its own.
        let block = boxed(k, [0.0, 0.0, 0.0], [2.0, 2.0, 1.0]);
        let core = rod(k, [1.0, 1.0, -1.0], 1.0, 3.0);
        let corners = solve(&block, &core, Subtract, k);
        assert_mass(&corners, 4.0 - PI, 2.0 * (4.0 - PI) + 8.0 + 2.0 * PI, k);
        assert_eq!(corners.body.shells.len(), 4);
        assert_eq!(counts(&corners), (24, 36, 20, 4));
        let disc = solve(&block, &core, Intersect, k);
        assert_mass(&disc, PI, 2.0 * PI + 2.0 * PI, k);
        assert_eq!(counts(&disc), (2, 2, 3, 1));
    }
}

#[test]
fn what_a_boolean_split_is_merged_again() {
    for k in SCALES {
        let tol = tolerance(k);
        // Two boxes that are flush become a box.
        let a = boxed(k, [0.0, 0.0, 0.0], [2.0, 2.0, 1.0]);
        let b = boxed(k, [2.0, 0.0, 0.0], [5.0, 2.0, 1.0]);
        let whole = primitives::block(&Frame::WORLD, 5.0 * k, 2.0 * k, k).unwrap();
        let joined = solve(&a, &b, Union, k);
        assert_eq!(counts(&joined), (8, 12, 6, 1));
        assert_eq!(counts(&joined), body_counts(&whole));
        let (m, expected) = (mass(&joined.body), mass(&whole));
        assert_close(m.volume, expected.volume, "volume");
        assert_close(m.area, expected.area, "area");
        // The faces that continue from A into B are A's; the far end is
        // B's own. Edges and vertices likewise: A's where A had them.
        let from_a = |i: u32| FaceSource {
            operand: Operand::A,
            face: FaceId(i),
        };
        assert_eq!(
            joined.faces,
            [
                from_a(0),
                from_a(1),
                from_a(2),
                from_a(3),
                from_a(4),
                FaceSource {
                    operand: Operand::B,
                    face: FaceId(5)
                }
            ]
        );
        let of_b = |e: &&EdgeSource| {
            matches!(
                e,
                EdgeSource::Kept {
                    operand: Operand::B,
                    ..
                }
            )
        };
        // The four edges around B's far end; the four long ones are A's.
        assert_eq!(joined.edges.iter().filter(of_b).count(), 4);
        assert!(
            joined
                .edges
                .iter()
                .all(|e| matches!(e, EdgeSource::Kept { .. }))
        );
        assert_eq!(
            joined
                .vertices
                .iter()
                .filter(|v| matches!(
                    v,
                    VertexSource::Kept {
                        operand: Operand::A,
                        ..
                    }
                ))
                .count(),
            4
        );
        // Without the merge the seam stays: the same solid, more of it.
        let plain = BooleanOptions { merge: false };
        let seamed = boolean_with(&a, &b, Union, &tol, &plain).unwrap();
        assert_eq!(counts(&seamed), (12, 20, 10, 1));
        let m = mass(&seamed.body);
        assert_close(m.volume, expected.volume, "volume");
        assert_close(m.area, expected.area, "area");

        // Flush on three sides and overlapping: an L, whose top and
        // bottom are one face each.
        let c = boxed(k, [1.0, 0.0, 0.0], [3.0, 3.0, 1.0]);
        let ell = solve(&a, &c, Union, k);
        assert_mass(&ell, 8.0, 2.0 * 8.0 + 12.0, k);
        assert_eq!(counts(&ell), (12, 18, 8, 1));

        // What was cut apart and is put together again is the body it
        // was: a box, a cylinder, a ball.
        let cutter = boxed(k, [1.0, 1.0, 0.5], [6.0, 5.0, 4.0]);
        for (body, expected) in [
            (boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]), (8, 12, 6, 1)),
            (rod(k, [1.0, 1.0, 0.0], 1.5, 2.0), (2, 2, 3, 1)),
            (ball(k, [1.0, 1.0, 0.5], 1.5), (0, 0, 1, 1)),
            (spike(k, [1.0, 1.0, 0.0], 1.5, 3.0), (1, 1, 2, 1)),
        ] {
            let less = solve(&body, &cutter, Subtract, k);
            let common = solve(&body, &cutter, Intersect, k);
            let again = solve(&less.body, &common.body, Union, k);
            let (m, before) = (mass(&again.body), mass(&body));
            assert_close(m.volume, before.volume, "volume");
            assert_close(m.area, before.area, "area");
            assert_eq!(counts(&again), expected);
        }

        // A cutter that only touches leaves no mark.
        let plate = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 1.0]);
        for toucher in [
            rod(k, [2.0, 1.5, 1.0], 0.5, 3.0),
            boxed(k, [1.0, 1.0, 1.0], [2.0, 2.0, 4.0]),
            boxed(k, [4.0, 1.0, 0.25], [5.0, 2.0, 0.75]),
            boxed(k, [4.0, -1.0, -1.0], [5.0, 4.0, 2.0]),
        ] {
            let same = solve(&plate, &toucher, Subtract, k);
            assert_eq!(counts(&same), (8, 12, 6, 1));
            assert_eq!(same.faces, (0..6).map(from_a).collect::<Vec<FaceSource>>());
            assert!(same.vertices.iter().enumerate().all(|(i, v)| matches!(
                v,
                VertexSource::Kept { operand: Operand::A, vertex: VertexId(n) } if (*n as usize) < 8 && i < 8
            )));
        }
        // A vertex the operand itself had between two edges on one line
        // is the operand's business and stays.
        let mut split = plate.clone();
        {
            // Split the edge from (0,0,0) to (4,0,0) at its middle.
            let edge = split
                .edges
                .iter()
                .position(|e| {
                    let (p, q) = (
                        split.vertices[e.start.0 as usize].point,
                        split.vertices[e.end.0 as usize].point,
                    );
                    p.y == 0.0 && q.y == 0.0 && p.z == 0.0 && q.z == 0.0
                })
                .unwrap();
            let end = split.edges[edge].end;
            let range = split.edges[edge].range;
            let curve = split.edges[edge].curve.clone();
            let middle = split.add_vertex(curve.eval(0.5 * (range.0 + range.1)));
            split.edges[edge].end = middle;
            split.edges[edge].range = (range.0, 0.5 * (range.0 + range.1));
            let rest = split.add_edge(curve, (0.5 * (range.0 + range.1), range.1), middle, end);
            for loop_ in &mut split.loops {
                if let Some(at) = loop_.coedges.iter().position(|c| c.edge.0 as usize == edge) {
                    let reversed = loop_.coedges[at].reversed;
                    let piece = craft3d_topo::Coedge {
                        edge: rest,
                        reversed,
                    };
                    loop_
                        .coedges
                        .insert(if reversed { at } else { at + 1 }, piece);
                }
            }
        }
        assert_eq!(craft3d_topo::check(&split, &tol), []);
        let cut = solve(
            &split,
            &boxed(k, [1.0, 2.0, 0.5], [3.0, 5.0, 4.0]),
            Subtract,
            k,
        );
        assert!(
            cut.body
                .vertices
                .iter()
                .any(|v| v.point.distance(Vec3::new(2.0 * k, 0.0, 0.0)) < 1e-9 * k)
        );
    }
}
