//! A blend of a body that fails the check is refused with the boolean's
//! words for its first operand, *"the first operand of a boolean produced
//! an invalid body"*, as before 2026-11-23: no smaller size mends a body
//! given broken, so no search for what fits runs. Since that day the
//! first tool's boolean's own invalid body is searched
//! (`docs/design/blends.md` › *A tool boolean's invalid body*); the two
//! are told apart by the operation the boolean names, and this holds the
//! body given to its old answer (the review of `check/blend-invalid-tool`).

// Test code: a failure in a helper is a failed test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_kernel::KernelError;
use craft3d_kernel::chamfer::{ChamferEdge, ChamferSize, chamfer};
use craft3d_kernel::fillet::{FilletEdge, fillet};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Tolerance, Transform, Vec3};
use craft3d_topo::{Body, EdgeId, FaceId, check};

fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

/// The edge whose middle is nearest `target`.
fn edge_at(body: &Body, target: Vec3) -> EdgeId {
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

/// The mean of the points where the edges of `face` start.
fn middle_of(body: &Body, face: FaceId) -> Vec3 {
    let face = body.face(face).unwrap();
    let mut sum = Vec3::new(0.0, 0.0, 0.0);
    let mut count = 0.0;
    for l in &face.loops {
        for c in &body.loop_(*l).unwrap().coedges {
            let e = body.edge(c.edge).unwrap();
            sum += e.curve.eval(e.range.0);
            count += 1.0;
        }
    }
    sum * (1.0 / count)
}

/// The faces that use `edge`.
fn faces_of(body: &Body, edge: EdgeId) -> Vec<FaceId> {
    (0..body.faces.len())
        .map(|i| FaceId(i as u32))
        .filter(|&f| {
            body.face(f).unwrap().loops.iter().any(|l| {
                body.loop_(*l)
                    .unwrap()
                    .coedges
                    .iter()
                    .any(|c| c.edge == edge)
            })
        })
        .collect()
}

/// `body` with the plane of its lowest face (by the mean of the points
/// where its edges start) moved `gap` down off its edges.
fn with_lowest_moved(mut body: Body, gap: f64) -> Body {
    let lowest = (0..body.faces.len())
        .map(|i| FaceId(i as u32))
        .min_by(|&a, &b| middle_of(&body, a).z.total_cmp(&middle_of(&body, b).z))
        .unwrap();
    let face = &mut body.faces[lowest.0 as usize];
    face.surface = face
        .surface
        .transformed(&Transform::translation(Vec3::new(0.0, 0.0, -gap)))
        .unwrap();
    body
}

/// A fillet and a chamfer of `edge` of `body`, by `size`.
fn blends(
    body: &Body,
    edge: EdgeId,
    size: f64,
    tol: &Tolerance,
) -> [(&'static str, Result<(), KernelError>); 2] {
    let first = faces_of(body, edge)[0];
    let round = [FilletEdge { edge, radius: size }];
    let cut = [ChamferEdge {
        edge,
        first,
        size: ChamferSize::Distances {
            first: size,
            second: size,
        },
    }];
    [
        ("fillet", fillet(body, &round, tol).map(|_| ())),
        ("chamfer", chamfer(body, &cut, tol).map(|_| ())),
    ]
}

/// A rod of radius 5 k and height 10 k, its floor's plane moved 50
/// tolerances down off its circle, fails the check; a fillet and a chamfer
/// of its top rim (the closed form: a tool cut in by the boolean) are
/// refused as the boolean's first operand's invalid body, at three scales,
/// with no search for what fits; the rod whole is filleted and chamfered.
/// A cube's top edge (a straight edge between planes, built in place, no
/// boolean) is refused too, by the construction in place's own check.
#[test]
fn a_blend_of_a_body_that_fails_the_check_is_refused_as_its_first_operand() {
    for k in [1e-3, 1.0, 1e6] {
        let tol = tolerance(k);
        let rod = primitives::cylinder(&Frame::WORLD, 5.0 * k, 10.0 * k).unwrap();
        let broken = with_lowest_moved(rod.clone(), 50.0 * tol.linear);
        assert!(check(&rod, &tol).is_empty(), "k={k}");
        assert!(!check(&broken, &tol).is_empty(), "k={k}");
        let rim = edge_at(&broken, Vec3::new(-5.0 * k, 0.0, 10.0 * k));
        for (what, got) in blends(&rod, rim, 1.0 * k, &tol) {
            assert!(got.is_ok(), "{what} of the rod whole k={k}: {got:?}");
        }
        for (what, got) in blends(&broken, rim, 1.0 * k, &tol) {
            match got {
                Err(error @ KernelError::InvalidBody { operation, .. }) => {
                    assert_eq!(operation, "the first operand of a boolean", "{what} k={k}");
                    let words = error.to_string();
                    assert!(
                        words.starts_with(
                            "the first operand of a boolean produced an invalid body:"
                        ),
                        "{what} k={k}: {words}"
                    );
                }
                other => panic!("{what} k={k}: {other:?}"),
            }
        }
        let cube = primitives::block(&Frame::WORLD, 10.0 * k, 10.0 * k, 10.0 * k).unwrap();
        let broken = with_lowest_moved(cube, 50.0 * tol.linear);
        assert!(!check(&broken, &tol).is_empty(), "k={k}");
        let edge = edge_at(&broken, Vec3::new(5.0 * k, 0.0, 10.0 * k));
        for (what, got) in blends(&broken, edge, 1.0 * k, &tol) {
            assert!(got.is_err(), "{what} of the cube k={k}: built");
        }
    }
}
