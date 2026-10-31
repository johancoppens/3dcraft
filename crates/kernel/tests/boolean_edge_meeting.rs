//! Where an edge of one body meets an edge of the other (2026-10-15, the
//! impeller's finding F8: a blade's fillet edge onto the rim of a hub).
//!
//! An edge is only within the tolerance of the curve it stands for. Where
//! an edge of A meets an edge of B, the place is found twice: as A's edge
//! crossing the surface of a face of B, and as B's edge crossing the
//! surface of a face of A. At a shallow angle the two are further apart
//! than points that are merged (half the tolerance), and were refused as
//! "two points closer together than the tolerance can tell apart". Here
//! that is made on purpose: A is a block whose top front edge lies a
//! little off its front face (within the tolerance, as an approximated
//! edge does), and B a block whose edge crosses it, with a face of B at a
//! small angle to A's edge. Each place is within the tolerance of both
//! edges: one vertex. At three scales, the three booleans and their
//! identities.

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_math::Portable;
mod boolean_common;

use boolean_common::*;
use craft3d_geom::{Curve3, Line};
use craft3d_kernel::boolean::BooleanOp::{Intersect, Subtract, Union};
use craft3d_kernel::boolean::boolean;
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Vec3};
use craft3d_topo::{Body, check};

/// The block with the given corners (`i + 2j + 4k` of `primitives::block`)
/// moved by `shift`, and every edge made again through its vertices.
fn moved(body: &Body, corners: &[usize], shift: Vec3) -> Body {
    let mut body = body.clone();
    for &corner in corners {
        body.vertices[corner].point += shift;
    }
    let points: Vec<Vec3> = body.vertices.iter().map(|v| v.point).collect();
    for edge in &mut body.edges {
        let (a, b) = (points[edge.start.0 as usize], points[edge.end.0 as usize]);
        edge.curve = Curve3::Line(Line::through(a, b).unwrap());
        edge.range = (0.0, a.distance(b));
    }
    body
}

/// A: the block `[0, 2]³` with its top front edge (`y = 0`, `z = 2`)
/// moved `off` out of its front face. B: a block whose edge runs through
/// the middle of that edge, into A, with a face at `angle` to it.
fn pair(k: f64, off: f64, angle: f64) -> (Body, Body) {
    let a = primitives::block(&Frame::WORLD, 2.0 * k, 2.0 * k, 2.0 * k).unwrap();
    let a = moved(&a, &[4, 5], Vec3::new(0.0, -off, 0.0));
    let z = Vec3::new(0.0, 1.0, -1.0).normalized().unwrap();
    let across = Vec3::new(0.0, -1.0, -1.0).normalized().unwrap();
    let x = Vec3::X * angle.pcos() + across * angle.psin();
    let middle = Vec3::new(1.0, 0.0, 2.0) * k;
    let frame = Frame::new(middle - z * k, z, x).unwrap();
    let b = primitives::block(&frame, 1.0 * k, 1.0 * k, 2.5 * k).unwrap();
    (a, b)
}

#[test]
fn an_edge_meets_an_edge_at_a_shallow_angle() {
    for k in SCALES {
        let tol = tolerance(k);
        // 0.4 of the tolerance off: the two places 0.65 to 0.85 of it
        // apart.
        for angle in [0.35_f64, 0.39, 0.45] {
            let (a, b) = pair(k, 0.4 * tol.linear, angle);
            assert_eq!(check(&a, &tol), [], "A at {k}");
            let of = |op| match boolean(&a, &b, op, &tol) {
                Ok(result) => {
                    assert_eq!(check(&result.body, &tol), [], "{op:?} at {k}, {angle}");
                    mass(&result.body).volume
                }
                Err(e) => panic!("{op:?} at {k}, angle {angle}: {e}"),
            };
            let (va, vb) = (mass(&a).volume, mass(&b).volume);
            let (all, common, rest) = (of(Union), of(Intersect), of(Subtract));
            assert!(common > 1e-3 * vb, "they overlap: {common}");
            // A's edge is off its face: its volume is known only to the
            // tolerance over its area (it measures 6.7e-8 more than 8).
            let slack = tol.linear * (mass(&a).area + mass(&b).area);
            assert!(
                (all + common - va - vb).abs() <= slack,
                "{all} + {common} != {va} + {vb}"
            );
            assert!(
                (rest + common - va).abs() <= slack,
                "{rest} + {common} != {va}"
            );
        }
    }
}
