//! Booleans of bodies that touch without overlapping, and the merging of
//! what a boolean splits needlessly (docs/testing.md, oracles 1 and 2).
//!
//! The rules under test, per operation, for bodies with no volume in
//! common: the union is both bodies as shells that share nothing, the
//! intersection is empty, the difference is the first body with the
//! counts it had. Since 2026-10-15 (the developer's decision) a result
//! whose parts touch, along an edge, a line or in a point, is refused as
//! zero thickness (`KernelError::ZeroThickness`); such a union is held to
//! the same rules made without the refusal, and the refusal to a point
//! that lies on two of its parts (`solve_parts`).

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod boolean_common;

use boolean_common::*;
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::{
    BooleanOp, BooleanOptions, EdgeSource, FaceSource, Operand, VertexSource, boolean_with,
    contains_points,
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

/// Two bodies that only touch: the three rules, either way round. Their
/// union is refused as zero thickness (since 2026-10-15: one body whose
/// two parts touch along a line or in a point), at a point that lies on
/// both parts (`solve_parts`), and is held to the rules as made without
/// the refusal.
#[track_caller]
fn assert_only_touching(a: &Body, b: &Body, k: f64) {
    let (ma, mb) = (mass(a), mass(b));
    let (ca, cb) = (body_counts(a), body_counts(b));
    for (x, y, mx, my, cx, cy) in [(a, b, ma, mb, ca, cb), (b, a, mb, ma, cb, ca)] {
        let (both, refused) = solve_parts(x, y, Union, k);
        assert!(refused.is_some(), "the union at scale {k} is not refused");
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
        // Zero thickness along that edge: refused there (since
        // 2026-10-15), and as before when made without the refusal.
        let (two, refused) = solve_parts(&ell.body, &second, Subtract, k);
        let at = refused.unwrap();
        assert!(at.distance(Vec3::new(k, k, at.z)) < 1e-9 * k && at.z > -1e-9 * k);
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
        let (cutter, refused) = solve_parts(&first, &second, Union, k);
        assert!(refused.is_some());
        assert_eq!(cutter.body.shells.len(), 2);
        let (again, refused) = solve_parts(&plate, &cutter.body, Subtract, k);
        assert!(refused.is_some());
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
        // out leaves the four corners, which touch each other along the
        // four lines where it touches the sides: zero thickness, refused
        // (since 2026-10-15), and as before when made without the refusal.
        let block = boxed(k, [0.0, 0.0, 0.0], [2.0, 2.0, 1.0]);
        let core = rod(k, [1.0, 1.0, -1.0], 1.0, 3.0);
        let (corners, refused) = solve_parts(&block, &core, Subtract, k);
        assert!(refused.is_some());
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
        // Every face that went into a merged face is on record: the top
        // is A's top and B's.
        let from_b = |i: u32| FaceSource {
            operand: Operand::B,
            face: FaceId(i),
        };
        assert_eq!(joined.merged.len(), 6);
        assert_eq!(joined.merged[1], [from_a(1), from_b(1)]);
        assert_eq!(joined.merged[4], [from_a(4)]);
        assert_eq!(joined.merged[5], [from_b(5)]);
        assert!(
            joined
                .faces
                .iter()
                .zip(&joined.merged)
                .all(|(source, all)| all.first() == Some(source))
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
        let plain = BooleanOptions {
            merge: false,
            ..BooleanOptions::default()
        };
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

/// The tolerance rule for faces of different kinds, on a ball of radius 2
/// that dips into the top of a block: by less than half the tolerance the
/// two touch; by between half and twice the tolerance the operation
/// refuses; by twice the tolerance or more they overlap, exactly as given,
/// in a cap whose two faces have area 2πRh + π(2Rh − h²).
#[test]
fn a_ball_that_dips_into_a_block_by_about_the_tolerance() {
    for k in SCALES {
        let block = boxed(k, [-4.0, -4.0, -3.0], [4.0, 4.0, 0.0]);
        let dipped = |tolerances: f64| ball(k, [0.5, -0.25, 2.0 - tolerances * 1e-6], 2.0);
        for tolerances in [0.05, 0.3, 0.45] {
            let ball = dipped(tolerances);
            assert_empty(&block, &ball, Intersect, k);
            assert_empty(&ball, &block, Intersect, k);
            // They touch in a point: the union is refused as zero
            // thickness there (since 2026-10-15), and is the two as before
            // when made without the refusal.
            let (both, refused) = solve_parts(&block, &ball, Union, k);
            let at = refused.unwrap();
            assert!(
                at.distance(Vec3::new(0.5, -0.25, 0.0) * k) <= 2e-6 * k,
                "{at:?}"
            );
            assert_eq!(both.body.shells.len(), 2, "{tolerances} at {k}");
            assert_eq!(both.body.edges.len(), 12);
            let less = solve(&block, &ball, Subtract, k);
            assert_mass(&less, 192.0, 224.0, k);
        }
        for tolerances in [0.6, 1.0, 1.7] {
            let ball = dipped(tolerances);
            for (a, b, op) in [
                (&block, &ball, Union),
                (&block, &ball, Intersect),
                (&block, &ball, Subtract),
                (&ball, &block, Subtract),
            ] {
                match craft3d_kernel::boolean::boolean(a, b, op, &tolerance(k)) {
                    Err(KernelError::Unsupported { reason, .. }) => assert!(
                        reason.contains("between half and twice the tolerance"),
                        "{reason}"
                    ),
                    other => panic!("{tolerances} tolerances at {k}: {op:?}: {other:?}"),
                }
            }
        }
        for tolerances in [2.5, 10.0, 1000.0] {
            let ball = dipped(tolerances);
            let h = tolerances * 1e-6;
            let cap = solve(&block, &ball, Intersect, k);
            assert_eq!(cap.body.shells.len(), 1);
            assert_eq!(cap.body.faces.len(), 2);
            let expected = 2.0 * PI * 2.0 * h + PI * (4.0 * h - h * h);
            let area = mass(&cap.body).area / (k * k);
            assert!(
                (area - expected).abs() <= 1e-6 * expected,
                "{tolerances} tolerances at {k}: area {area}, expected {expected}"
            );
            assert_eq!(solve(&block, &ball, Union, k).body.shells.len(), 1);
        }
    }
}

/// A cone given by its apex, the unit axis from the apex into it, its
/// half-angle and its height, as a body and as a membership test with the
/// distance to the nearest of its two faces' surfaces.
struct Pointed {
    apex: Vec3,
    axis: Vec3,
    alpha: f64,
    height: f64,
}

impl Pointed {
    fn body(&self) -> Body {
        let base = self.apex + self.axis * self.height;
        let frame = Frame::from_z(base, -self.axis).unwrap();
        primitives::cone(&frame, self.height * self.alpha.tan(), self.height).unwrap()
    }

    fn volume(&self) -> f64 {
        PI * (self.height * self.alpha.tan()).powi(2) * self.height / 3.0
    }

    /// Inside or not, and how far from the surfaces of its faces.
    fn holds(&self, p: Vec3) -> (bool, f64) {
        let q = p - self.apex;
        let h = q.dot(self.axis);
        let side = (q - self.axis * h).length() * self.alpha.cos() - h * self.alpha.sin();
        (
            h < self.height && h > 0.0 && side < 0.0,
            side.abs().min((h - self.height).abs()),
        )
    }
}

/// Cones that share a ruling and the tangent plane along it, or an apex
/// (`geom::intersect`'s closed forms of 2026-10-14: the ruling, and the
/// conic in which such cones cross besides; the corpus case
/// tangent/cone-cone/002 was refused before, because the touch runs into
/// an apex). The first cone stands on the plane z = 0 of a frame; the
/// second has its apex on the ruling in the frame's xz plane, at
/// `along` of the first's slant height from the first apex (negative:
/// beyond it), and leans away from the first by its own half-angle
/// (`outside`) or towards it.
///
/// From outside they only touch, whatever the apexes: the three rules for
/// bodies with nothing in common. From inside the narrower one leaves the
/// wider through the conic: what is in the result is held, point by
/// point, to membership of the two cones by their own equations, and the
/// volumes to the identities of sets.
#[test]
fn cones_that_share_a_ruling() {
    let tilted = Frame::new(
        Vec3::new(1.0, -2.0, 0.5),
        Vec3::new(0.3, -0.4, 1.0),
        Vec3::new(1.0, 0.2, 0.0),
    )
    .unwrap();
    let mut crossed = 0;
    for k in SCALES {
        for frame in [Frame::WORLD, tilted] {
            let frame = frame.at(frame.origin() * k).unwrap();
            for (alpha, beta, along, tall) in [
                (0.6_f64, 0.5_f64, 0.0, 1.5),
                (0.6, 0.5, 0.25, 1.5),
                (0.6, 0.5, -0.05, 1.5),
                (0.4, 0.9, 0.5, 0.8),
                // The angles of tangent/cone-cone/002 of the corpus.
                (
                    (1.0_f64 / 1.125).atan(),
                    std::f64::consts::FRAC_PI_4,
                    -0.02,
                    1.125,
                ),
            ] {
                let height = 2.0 * k;
                let first = Pointed {
                    apex: frame.point(0.0, 0.0, height),
                    axis: -frame.z(),
                    alpha,
                    height,
                };
                let ruling = frame.vector(alpha.sin(), 0.0, -alpha.cos());
                let slant = height / alpha.cos();
                for outside in [true, false] {
                    let lean = if outside { alpha + beta } else { alpha - beta };
                    let second = Pointed {
                        apex: first.apex + ruling * (along * slant),
                        axis: frame.vector(lean.sin(), 0.0, -lean.cos()),
                        alpha: beta,
                        height: tall * k,
                    };
                    let (a, b) = (first.body(), second.body());
                    assert_close(mass(&a).volume, first.volume(), "the first cone");
                    assert_close(mass(&b).volume, second.volume(), "the second cone");
                    if outside {
                        assert_only_touching(&a, &b, k);
                        continue;
                    }
                    // Seeded points in a box around both, away from the
                    // surfaces.
                    let mut rng = Rng(0x00C0_FFEE ^ (along.to_bits() >> 7));
                    let reach = 3.0 * k;
                    let mut points = Vec::new();
                    while points.len() < 3000 {
                        let p = first.apex - frame.z() * (0.5 * height)
                            + Vec3::new(
                                (2.0 * rng.unit() - 1.0) * reach,
                                (2.0 * rng.unit() - 1.0) * reach,
                                (2.0 * rng.unit() - 1.0) * reach,
                            );
                        let ((in_a, off_a), (in_b, off_b)) = (first.holds(p), second.holds(p));
                        if off_a.min(off_b) > 1e-3 * k {
                            points.push((p, in_a, in_b));
                        }
                    }
                    let places: Vec<Vec3> = points.iter().map(|p| p.0).collect();
                    let mut volumes = Vec::new();
                    type Rule = fn(bool, bool) -> bool;
                    let rules: [(&Body, &Body, BooleanOp, Rule); 4] = [
                        (&a, &b, Intersect, |x, y| x && y),
                        (&a, &b, Union, |x, y| x || y),
                        (&a, &b, Subtract, |x, y| x && !y),
                        (&b, &a, Subtract, |x, y| !x && y),
                    ];
                    for (x, y, op, rule) in rules {
                        let want: Vec<bool> = points.iter().map(|p| rule(p.1, p.2)).collect();
                        // Nothing where no point is expected may still be
                        // a little: the boolean says.
                        if !want.contains(&true)
                            && matches!(
                                boolean_with(x, y, op, &tolerance(k), &BooleanOptions::default()),
                                Err(KernelError::EmptyResult { .. })
                            )
                        {
                            volumes.push(0.0);
                            continue;
                        }
                        // A difference that falls into parts meeting along
                        // the shared ruling or at an apex is refused as
                        // zero thickness (since 2026-10-15); it is held
                        // to the points as made without the refusal.
                        let (result, _) = solve_parts(x, y, op, k);
                        volumes.push(mass(&result.body).volume);
                        let found = contains_points(&result.body, &places, &tolerance(k)).unwrap();
                        let mut decided = 0;
                        for ((p, want), got) in places.iter().zip(want).zip(found) {
                            if let Some(got) = got {
                                assert_eq!(got, want, "{op:?} at {p:?}, scale {k}");
                                decided += 1;
                            }
                        }
                        assert!(decided > 2900, "{decided} points decided");
                    }
                    let [common, union, a_less, b_less] = volumes[..] else {
                        panic!("four results");
                    };
                    let near = |actual: f64, expected: f64, what: &str| {
                        assert!(
                            (actual - expected).abs() <= 1e-7 * expected.abs(),
                            "{what} at scale {k}: {actual} is not {expected}"
                        );
                    };
                    near(
                        common + a_less,
                        first.volume(),
                        "common + first less second",
                    );
                    near(
                        common + b_less,
                        second.volume(),
                        "common + second less first",
                    );
                    near(
                        union + common,
                        first.volume() + second.volume(),
                        "union + common",
                    );
                    crossed += usize::from(b_less > 1e-6 * second.volume() && common > 0.0);
                }
            }
        }
    }
    // The narrower cone did come out of the wider one in some of them.
    assert!(crossed >= 6, "{crossed}");
}

/// A cone standing a tenth of a tolerance over (or into) the top of a block
/// whose edges its rim touches in the plane: the corpus case
/// near/cone-dome/005 with a block for the dome. Under the rim the cone's
/// side, carried on below its base, crosses the edge twice, half a
/// millimetre apart at scale 1 and never more than a twentieth of a
/// tolerance past it between the crossings: one touch, which the root
/// isolation put on either crossing (wherever its search for the nearest
/// point happened to go), so that the edge was cut there and the rim, which
/// touches the edge between them, at the touch, and the pieces did not
/// match. The touch is now where the curve goes deepest.
///
/// The faces under the cone are one surface within the tolerance, facing
/// each other: the union is one body, a cone on a block, with neither face
/// where they overlap (the disc less the segment beyond the block's far
/// edge); the intersection is empty; each difference is the body it
/// starts from.
#[test]
fn a_rim_that_touches_an_edge_a_tenth_of_a_tolerance_over_a_face() {
    let (radius, height) = (0.75_f64, 1.5_f64);
    // The block's top is x in [0.75, 2], y in [0, 1.75], z = 0; the rim's
    // centre is 0.5 from x = 0.75 and touches x = 2 and y = 0.
    let (centre, beyond) = ([1.25, 0.75], 0.5_f64);
    let segment = radius * radius * (beyond / radius).acos()
        - beyond * (radius * radius - beyond * beyond).sqrt();
    let overlap = PI * radius * radius - segment;
    for k in SCALES {
        let block = boxed(k, [0.75, 0.0, -3.0], [2.0, 1.75, 0.0]);
        for gap in [0.1, -0.1, 0.3] {
            let cone = spike(k, [centre[0], centre[1], gap * 1e-6], radius, height);
            let (mb, mc) = (mass(&block), mass(&cone));
            let both = solve(&block, &cone, Union, k);
            assert_eq!(both.body.shells.len(), 1, "gap {gap} at {k}");
            let m = mass(&both.body);
            // Glued across the gap, which is within the tolerance.
            let sum = mb.volume + mc.volume;
            let slack = gap.abs() * 1e-6 * PI * radius * radius * k * k * k + 1e-9 * sum;
            assert!(
                (m.volume - sum).abs() <= slack,
                "gap {gap} at {k}: volume {} instead of {sum}",
                m.volume
            );
            let area = mb.area + mc.area - 2.0 * overlap * k * k;
            assert!(
                (m.area - area).abs() <= 1e-6 * area,
                "gap {gap} at {k}: area {} instead of {area}",
                m.area
            );
            assert_empty(&block, &cone, Intersect, k);
            for (x, y, mx) in [(&block, &cone, mb), (&cone, &block, mc)] {
                let less = solve(x, y, Subtract, k);
                let m = mass(&less.body);
                assert_close(m.volume, mx.volume, "volume of the difference");
                assert_close(m.area, mx.area, "area of the difference");
            }
        }
    }
}

/// A block that lies against a cone along a ruling, with an edge on the
/// ruling that runs on past the apex (the corpus case
/// contact/cone-dome/002 with a block for the dome): its face is the
/// cone's tangent plane along the ruling. The root isolation could not
/// settle the edge against the cone (it lies on it up to the apex and
/// leaves it there, and a cone's distance has no bound on its bend), and
/// refused "a curve that runs along a surface"; the edge now meets the cone
/// at the apex. They only touch: the union is both, the intersection
/// empty, each difference the body it starts from.
#[test]
fn a_block_against_a_cone_along_a_ruling_past_its_apex() {
    let (radius, height) = (0.75_f64, 1.875_f64);
    let slant = radius.hypot(height);
    for k in SCALES {
        let cone = spike(k, [0.0, 0.0, 0.0], radius, height);
        // Along the ruling in the xz plane, from below the rim up past
        // the apex; out of the cone along its normal there.
        let up = Vec3::new(-radius, 0.0, height) * (1.0 / slant);
        let out = Vec3::new(height, 0.0, radius) * (1.0 / slant);
        for (from, length, wide) in [(-0.1, 1.5, 1.0), (0.2, 1.2, 0.6)] {
            let start = Vec3::new(radius, 0.0, 0.0) + up * (from * slant);
            for side in [1.0, -1.0] {
                // The block's x along the normal, z along the ruling (up
                // it, or down it from the top end), so its y is to one
                // side of the ruling or the other.
                let base = if side > 0.0 {
                    start
                } else {
                    start + up * (length * slant)
                };
                let frame = Frame::new(base * k, up * side, out).unwrap();
                let block = craft3d_kernel::primitives::block(
                    &frame,
                    0.5 * k,
                    wide * k,
                    length * slant * k,
                )
                .unwrap();
                let (mb, mc) = (mass(&block), mass(&cone));
                // Along the ruling the two touch: the union is refused as
                // zero thickness (since 2026-10-15) and is both bodies
                // when made without the refusal.
                let (both, refused) = solve_parts(&block, &cone, Union, k);
                assert!(refused.is_some(), "{from} {side} at {k}");
                let m = mass(&both.body);
                assert_close(m.volume, mb.volume + mc.volume, "volume of the union");
                assert_close(m.area, mb.area + mc.area, "area of the union");
                assert_empty(&block, &cone, Intersect, k);
                assert_empty(&cone, &block, Intersect, k);
                for (x, y, mx) in [(&block, &cone, mb), (&cone, &block, mc)] {
                    let less = solve(x, y, Subtract, k);
                    let m = mass(&less.body);
                    assert_close(m.volume, mx.volume, "volume of the difference");
                    assert_close(m.area, mx.area, "area of the difference");
                }
            }
        }
    }
}

/// A rod inside a cone on its axis, its top rim a few tolerances inside
/// the cone's side (the corpus case near/cone-cone/002 has two cone rims
/// ten tolerances apart): a circle about the cone's axis is at one distance
/// from it all round, which the root isolation could not prove (it cut the
/// circle into pieces shorter than that distance and gave up). The rod is
/// inside: the intersection is the rod, the union the cone, the cone less
/// the rod has a cavity.
#[test]
fn a_rim_a_few_tolerances_inside_a_coaxial_cone() {
    let (radius, height) = (2.0_f64, 4.0_f64);
    for k in SCALES {
        let cone = spike(k, [0.0, 0.0, 0.0], radius, height);
        for tolerances in [3.0, 5.0, 40.0] {
            // The rod's top at z = 2, where the cone's radius is 1.
            let r = 1.0 - tolerances * 1e-6 * (radius.hypot(height) / height);
            let rod = rod(k, [0.0, 0.0, 0.5], r, 1.5);
            let (vr, vc) = (mass(&rod).volume, mass(&cone).volume);
            let common = solve(&cone, &rod, Intersect, k);
            assert_close(mass(&common.body).volume, vr, "the rod");
            let both = solve(&cone, &rod, Union, k);
            assert_close(mass(&both.body).volume, vc, "the cone");
            let less = solve(&cone, &rod, Subtract, k);
            assert_close(mass(&less.body).volume, vc - vr, "the cone less the rod");
            assert_eq!(less.body.shells.len(), 2, "{tolerances} at {k}");
            assert_empty(&rod, &cone, Subtract, k);
        }
    }
}

/// The volume a cylinder of radius `rho` and length `length` has in
/// common with the torus of radii `major` and `minor` (centre at the
/// origin, axis z), when the cylinder's axis runs along y through
/// `(major, ·, height)` with its middle at y = 0 and the cylinder lies in
/// the tube's disc `(x − R)² + z² ≤ r²`: at height z the cylinder spans
/// `x ∈ [R − q, R + q]`, `q = sqrt(ρ² − (z − h)²)`, and the torus holds
/// the line along y through `(x, z)` for `|y| ≤ sqrt((R + w)² − x²)`,
/// `w = sqrt(r² − z²)` (the inner bound vanishes since `x ≥ R − w`). The
/// integral across in closed form, `∫ min(L/2, sqrt(a² − x²)) dx`; the
/// one up by Gauss–Legendre in the angle `z = h + ρ sin θ`, on 4000
/// panels (the integrand has kinks where the ends cut in).
fn cylinder_in_torus(major: f64, minor: f64, rho: f64, height: f64, length: f64) -> f64 {
    const NODES: [(f64, f64); 5] = [
        (0.0, 0.568_888_888_888_888_9),
        (-0.538_469_310_105_683_1, 0.478_628_670_499_366_5),
        (0.538_469_310_105_683_1, 0.478_628_670_499_366_5),
        (-0.906_179_845_938_664, 0.236_926_885_056_189_1),
        (0.906_179_845_938_664, 0.236_926_885_056_189_1),
    ];
    let half = 0.5 * length;
    let across = |z: f64| {
        let q = (rho * rho - (z - height) * (z - height)).max(0.0).sqrt();
        let w = (minor * minor - z * z).max(0.0).sqrt();
        let a = major + w;
        let (x1, x2) = (major - q, (major + q).min(a));
        let f = |x: f64| 0.5 * (x * (a * a - x * x).max(0.0).sqrt() + a * a * (x / a).asin());
        let xc = (a * a - half * half).max(0.0).sqrt().clamp(x1, x2);
        2.0 * (half * (xc - x1) + f(x2) - f(xc))
    };
    let panels = 4000;
    let width = PI / f64::from(panels);
    let mut total = 0.0;
    for i in 0..panels {
        let mid = -0.5 * PI + (f64::from(i) + 0.5) * width;
        for (x, weight) in NODES {
            let theta = mid + 0.5 * width * x;
            total += weight * 0.5 * width * across(height + rho * theta.sin()) * rho * theta.cos();
        }
    }
    total
}

/// Fourth-order contact (`docs/roadmap.md` › the boolean checklist, the
/// random runs' seed 2001169): a cylinder inside the tube of a torus, its
/// axis square to the torus', touching the top of the tube. Along its top
/// line the cylinder rises above the tube by `y⁴ / (8 R² r)`, so the two
/// surfaces meet in two loops that leave the point of contact and come
/// back to it, all four of their ends tangent to each other there (a
/// tacnode). The marcher passed through it until 2026-10-15 and the
/// boolean was refused ("stalled", or "two curves leave a point in the
/// same direction and stay together"); now the point is a vertex with
/// four edges, made as graphs near it (`geom`: `intersect/march/tacnode.rs`).
/// Held to the common volume integrated from the closed form above, at
/// three scales, tilted, both ways round, for the two cylinders of the
/// reproducer, a thinner one, and the cylinder lifted out of the tube or
/// sunk into it by three tenths of the tolerance. The slack is the
/// tolerance times the result's area: what moving every face by the
/// tolerance changes.
#[test]
fn a_cylinder_touching_the_top_of_a_torus_tube_from_inside() {
    let tilt = Frame::new(
        Vec3::new(0.2, -0.1, 0.3),
        Vec3::new(0.3, -0.4, 1.0),
        Vec3::new(1.0, 0.2, 0.0),
    )
    .unwrap();
    let (major, minor) = (3.0, 1.0);
    for k in SCALES {
        let tol = tolerance(k);
        for frame in [Frame::WORLD, tilt] {
            let frame = frame.at(frame.origin() * k).unwrap();
            let torus = primitives::torus(&frame, major * k, minor * k).unwrap();
            for (length, radius, lift) in [
                (2.0, 0.5, 0.0),
                (3.0, 0.3, 0.0),
                (8.0, 0.1, 0.0),
                (2.0, 0.5, 0.3),
                (2.0, 0.5, -0.3),
            ] {
                let what = format!("scale {k}, {length} by {radius}, lifted {lift}");
                let height = minor - radius + lift * 1e-6;
                let base = Frame::new(
                    frame.point(major * k, -0.5 * length * k, height * k),
                    frame.y(),
                    frame.x(),
                )
                .unwrap();
                let cylinder = primitives::cylinder(&base, radius * k, length * k).unwrap();
                let common = cylinder_in_torus(major, minor, radius, height, length) * k * k * k;
                let (vt, vc) = (mass(&torus).volume, mass(&cylinder).volume);
                let touch = frame.point(major * k, 0.0, minor * k);
                for (a, b, va, vb) in [(&torus, &cylinder, vt, vc), (&cylinder, &torus, vc, vt)] {
                    for (op, expected) in [
                        (Intersect, common),
                        (Union, va + vb - common),
                        (Subtract, va - common),
                    ] {
                        // The cylinder less the torus is two pieces that
                        // meet in the point of contact: zero thickness,
                        // refused there (since 2026-10-15), held to the
                        // volume when made without the refusal. (The
                        // common part is one piece through that point.)
                        let (result, refused) = solve_parts(a, b, op, k);
                        let parts = op == Subtract && va == vc;
                        assert_eq!(refused.is_some(), parts, "{what}, {op:?}");
                        if let Some(at) = refused {
                            assert!(at.distance(touch) <= 64.0 * tol.linear, "{what}, {op:?}");
                        }
                        let m = mass(&result.body);
                        let slack = tol.linear * m.area;
                        assert!(
                            (m.volume - expected).abs() <= slack,
                            "{what}, {op:?}: volume {} for {expected} (off by {:e}, slack {slack:e})",
                            m.volume,
                            (m.volume - expected).abs()
                        );
                        // The point of contact is a vertex of the result
                        // (where along the arms it lies is ill-conditioned:
                        // the surfaces are within `y⁴` of each other).
                        let nearest = result
                            .body
                            .vertices
                            .iter()
                            .map(|v| v.point.distance(touch))
                            .fold(f64::INFINITY, f64::min);
                        assert!(
                            nearest <= 64.0 * tol.linear,
                            "{what}, {op:?}: the nearest vertex is {nearest:e} from the point of contact"
                        );
                    }
                }
            }
        }
    }
}
