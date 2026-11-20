//! Owed by the boolean (found 2026-11-20 in a hipped roof of 24 planes
//! whose normals were rounded to six decimals; docs/design/booleans.md ›
//! *A plane that passes a vertex by less than a tolerance*): a block
//! whose top passes within about a tolerance of a vertex an earlier
//! intersection made, at a convex corner of a prism.
//!
//! The case, with plain booleans. A prism over a four-cornered outline,
//! 40 high; its corner at (50.4, 0) is the one of the roof's outline. A
//! first block, its top a plane through that corner at height 14 that
//! holds the outline's edge on one side of the corner, is intersected
//! with the prism: the corner's upright edge now ends in a vertex at
//! (50.4, 0, 14). A second block, its top a plane along the outline's
//! edge on the other side of the corner (to the six decimals its normal
//! was given with), passes `t` tolerances over that vertex and is
//! intersected with what the first left. Each block reaches far past the
//! prism on every other side.
//!
//! What each distance answers today is printed by the test
//! (`--ignored --nocapture`) and written under the test below. Two things
//! are owed: a refusal there says "imprint lost: a curve in which two
//! faces cross does not divide them; a piece of it is missing", which
//! names no place, no tolerance and nothing to do; and the band the
//! roadmap gives (half to twice the tolerance refused, as neither one
//! place nor two) is not what happens: 0.7 to 2 tolerances build.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod boolean_common;
mod common;

use boolean_common::tolerance;
use common::polygon;
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::primitives::block;
use craft3d_kernel::profile::detect_profiles;
use craft3d_math::{Frame, Vec3};
use craft3d_topo::{Body, check};

/// The corner, and its neighbours along the outline of the roof.
const BEFORE: [f64; 2] = [44.727, -8.56];
const CORNER: [f64; 2] = [50.4, 0.0];
const AFTER: [f64; 2] = [50.464, 9.658];
/// The eaves' height.
const EAVES: f64 = 14.0;
/// The normals as the roof gave them, to six decimals: of the plane
/// through the edge from the corner on, and of the one through the edge
/// up to it.
const FIRST: [f64; 3] = [0.615_648, -0.004_08, 0.788_011];
const SECOND: [f64; 3] = [0.513_191, -0.340_109, 0.788_011];

/// How far the second plane passes over the vertex, in tolerances.
const OVER: [f64; 13] = [
    0.0, 0.3, 0.45, 0.49, 0.497, 0.51, 0.55, 0.7, 0.9, 1.0, 1.5, 1.9, 2.1,
];
/// And under it.
const UNDER: [f64; 8] = [-0.3, -0.45, -0.55, -0.7, -1.0, -1.5, -1.9, -2.1];

fn prism() -> Body {
    let outline = polygon(1, 1, &[CORNER, AFTER, [0.0, 20.0], BEFORE]);
    let profiles = detect_profiles(&outline, &tolerance(1.0)).unwrap();
    extrude(&profiles, &Frame::WORLD, 0.0, 40.0, &tolerance(1.0))
        .unwrap()
        .body
}

/// A block whose top lies in the plane through `at` square to `normal`
/// and which reaches 150 to every side of `at` in the plane and 150 down
/// from it: past the prism on every side but the top.
fn under(at: Vec3, normal: [f64; 3]) -> Body {
    let plane = Frame::from_z(at, Vec3::new(normal[0], normal[1], normal[2])).unwrap();
    let reach = 150.0;
    let corner = at - plane.x() * reach - plane.y() * reach - plane.z() * reach;
    block(&plane.at(corner).unwrap(), 2.0 * reach, 2.0 * reach, reach).unwrap()
}

/// The prism under the first plane, and the vertex the plane made on the
/// corner's upright edge.
fn first_cut() -> (Body, Vec3) {
    let tol = tolerance(1.0);
    let at = Vec3::new(CORNER[0], CORNER[1], EAVES);
    let cut = boolean(&prism(), &under(at, FIRST), BooleanOp::Intersect, &tol).unwrap();
    assert_eq!(check(&cut.body, &tol), []);
    let vertex = cut
        .body
        .vertices
        .iter()
        .map(|v| v.point)
        .min_by(|a, b| a.distance(at).total_cmp(&b.distance(at)))
        .unwrap();
    assert!(vertex.distance(at) <= 1e-12, "{vertex:?}");
    (cut.body, vertex)
}

/// What the second block answers at `t` tolerances over the vertex: the
/// body, or the refusal.
fn second_cut(body: &Body, vertex: Vec3, t: f64) -> Result<Body, KernelError> {
    let tol = tolerance(1.0);
    let n = Vec3::new(SECOND[0], SECOND[1], SECOND[2])
        .normalized()
        .unwrap();
    let at = vertex + n * (t * tol.linear);
    boolean(body, &under(at, SECOND), BooleanOp::Intersect, &tol).map(|result| result.body)
}

/// How far apart the two nearest vertices of `body` are.
fn nearest_pair(body: &Body) -> f64 {
    let points: Vec<Vec3> = body.vertices.iter().map(|v| v.point).collect();
    let mut nearest = f64::INFINITY;
    for (i, a) in points.iter().enumerate() {
        for b in points.iter().skip(i + 1) {
            nearest = nearest.min(a.distance(*b));
        }
    }
    nearest
}

/// Owed: at every distance the second block either cuts (a body that
/// passes its check) or is refused as at the edge of tolerance, with the
/// faces. Today (2026-11-20, the answers the Check printed, the same on
/// `check/cut-planes` and on `check/contact-border` f93760c7): see
/// docs/design/booleans.md › *A plane that passes a vertex by less than a
/// tolerance*, which lists every distance.
#[test]
#[ignore = "owed: a block's top within about half a tolerance of a vertex is refused as \"imprint lost\", with no place, tolerance or action: run with --ignored --nocapture"]
fn a_block_whose_top_passes_a_vertex_by_less_than_a_tolerance_cuts_or_says_where() {
    let tol = tolerance(1.0);
    let (body, vertex) = first_cut();
    let mut owed = Vec::new();
    for t in OVER.into_iter().chain(UNDER) {
        let said = match second_cut(&body, vertex, t) {
            Ok(cut) => {
                let problems = check(&cut, &tol);
                if problems.is_empty() {
                    format!(
                        "builds ({} faces, {} vertices, the two nearest {:.2} tolerances apart)",
                        cut.faces.len(),
                        cut.vertices.len(),
                        nearest_pair(&cut) / tol.linear
                    )
                } else {
                    owed.push(t);
                    format!("builds a body that fails its check: {problems:?}")
                }
            }
            Err(KernelError::Unsupported { reason, faces, .. })
                if reason.ends_with("(at the edge of tolerance)") && !faces.is_empty() =>
            {
                format!("refused at the edge of tolerance, with its faces: {reason}")
            }
            Err(error) => {
                owed.push(t);
                format!("refused without a place: {error}")
            }
        };
        println!("plane_by_vertex: {t:+.3} tolerances: {said}");
    }
    assert!(owed.is_empty(), "owed at {owed:?} tolerances");
}

/// What holds today and must keep holding: well clear of the vertex, on
/// either side, the second block cuts, and so it does through the vertex
/// itself.
#[test]
fn a_block_whose_top_passes_a_vertex_well_clear_of_it_cuts() {
    let tol = tolerance(1.0);
    let (body, vertex) = first_cut();
    for t in [0.0, 5.0, -5.0, 1000.0, -1000.0] {
        match second_cut(&body, vertex, t) {
            Ok(cut) => assert_eq!(check(&cut, &tol), [], "{t} tolerances"),
            Err(error) => panic!("{t} tolerances: {error}"),
        }
    }
}
