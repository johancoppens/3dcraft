//! The review of the repair of a spar's flat bottom beside a corner of a
//! rib's notch (check/wing-eighth-point, 2026-11-23;
//! docs/design/booleans.md › *A spar's flat bottom beside a corner of a
//! notch*): two cuts of a curve closer together than the tolerance can
//! tell apart, both clear outside one of the two faces, are two cuts.
//!
//! The pair is the repair's own (`boolean_corner_beside.rs`): a prism over
//! a triangle with a corner at the origin, 1.5 thick along z, and a long
//! block turned 25 degrees about the y axis, its bottom the plane `y = d`
//! tolerances, its bottom face beginning `beside` tolerances to the right
//! of the corner at `z = 0`. Here the triangle's two edges may lean to the
//! left (`lean`: x per unit of y), away from the block's face, where the
//! repair's prism has one edge upright.
//!
//! - Where the block's face begins: the nearer of the two places is 1.95
//!   tolerances from the face's edge (refused, as on `main`) or 2.05 (told
//!   apart).
//! - A corner whose edges lean away from the face is nearer the face than
//!   the two places are: both can be clear of it while the corner is not.
//!   Ignored: the union of such a pair is built with its two parts 1.90
//!   tolerances apart, which `main` refused.
//! - A fillet and a chamfer of a body with a loose vertex, whose tool's
//!   boolean is tried at four times the working tolerance first
//!   (`loose_op::boolean_for_tool`) and meets the state there.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod boolean_common;
mod common;

use boolean_common::{mass, verified};
use common::polygon;
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::{BooleanOp, Interference, boolean, interference};
use craft3d_kernel::chamfer::{ChamferEdge, ChamferSize, chamfer};
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::fillet::{FilletEdge, fillet};
use craft3d_kernel::primitives::block;
use craft3d_kernel::profile::detect_profiles;
use craft3d_math::{Frame, Portable, Tolerance, Vec3};
use craft3d_topo::{
    Body, EdgeId, VertexId, check, check_capped, default_loosest, edge_faces, record_vertex,
};

/// The scales the case is held at; the tolerance scales along.
const SCALES: [f64; 4] = [1e-3, 1.0, 1e3, 1e6];

/// The tolerance at scale `k`: the Jig's 0.01 mm at 1.
fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(0.01 * k, 1e-9).unwrap()
}

/// A frame turned in space, at scale `k` (the repair's test's).
fn turned(k: f64) -> Frame {
    Frame::new(
        Vec3::new(13.0 * k, -7.0 * k, 3.0 * k),
        Vec3::new(0.3, -0.5, 0.81),
        Vec3::new(1.0, 0.2, 0.0),
    )
    .unwrap()
}

/// The cosine of the block's turn.
fn cos_turn() -> f64 {
    25.0_f64.to_radians().psin_cos().1
}

/// The prism (the rib) over the triangle with its corner at the origin and
/// its two edges leaning `lean.0` and `lean.1` to the left, and the block
/// (the spar), 6 by `high` by 800, at scale `k` in `frame`: its bottom `d`
/// tolerances over the corner, its face beginning `beside` tolerances to
/// the right of it.
fn pair(
    k: f64,
    frame: &Frame,
    d: f64,
    beside: f64,
    lean: (f64, f64),
    high: f64,
) -> (Body, Body, Tolerance) {
    let tol = tolerance(k);
    let t = tol.linear;
    let h = 5.0 * k;
    let outline = polygon(1, 1, &[[0.0, 0.0], [-lean.0 * h, h], [-lean.1 * h, h]]);
    let profiles = detect_profiles(&outline, &tol).unwrap();
    let rib = extrude(&profiles, frame, 0.0, 1.5 * k, &tol).unwrap().body;
    let (s, c) = 25.0_f64.to_radians().psin_cos();
    let along = frame.vector(s, 0.0, c);
    let across = frame.vector(c, 0.0, -s);
    let start = frame.point(beside * t, d * t, 0.0) - along * (400.0 * k);
    let spar = block(
        &Frame::new(start, along, across).unwrap(),
        6.0 * k,
        high * k,
        800.0 * k,
    )
    .unwrap();
    (rib, spar, tol)
}

#[track_caller]
fn assert_close(actual: f64, expected: f64, slack: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= slack,
        "{what}: {actual} is not {expected} (within {slack})"
    );
}

/// What the three booleans and the interference of the pair say, in a
/// word each, for the table a run prints.
fn said(rib: &Body, spar: &Body, tol: &Tolerance) -> String {
    let word = |r: Result<usize, KernelError>| match r {
        Ok(shells) => format!("built ({shells} shells)"),
        Err(KernelError::EmptyResult { .. }) => "empty".to_string(),
        Err(KernelError::Unsupported { reason, faces, .. }) => {
            format!("refused: {reason} ({} faces)", faces.len())
        }
        Err(e) => format!("error: {e}"),
    };
    let op = |a: &Body, b: &Body, op| word(boolean(a, b, op, tol).map(|r| r.body.shells.len()));
    let contact = match interference(spar, rib, tol) {
        Ok(Interference::Clear) => "clear".to_string(),
        Ok(Interference::Touch { .. }) => "touch".to_string(),
        Ok(Interference::Overlap(_)) => "overlap".to_string(),
        Err(e) => word(Err(e)),
    };
    format!(
        "union {}; rib less spar {}; spar less rib {}; intersect {}; interference {contact}",
        op(spar, rib, BooleanOp::Union),
        op(rib, spar, BooleanOp::Subtract),
        op(spar, rib, BooleanOp::Subtract),
        op(spar, rib, BooleanOp::Intersect),
    )
}

/// The two bodies told apart, as the repair's test has it: their union is
/// both, two shells; each less the other is itself; they have nothing in
/// common, and nothing of one touches the other.
fn told_apart(rib: &Body, spar: &Body, tol: &Tolerance, what: &str) {
    let (v_rib, v_spar) = (mass(rib).volume, mass(spar).volume);
    let slack = |v: f64| 1e-9 * v.abs();
    let both = boolean(spar, rib, BooleanOp::Union, tol)
        .unwrap_or_else(|e| panic!("{what}: the union: {e}"));
    assert_eq!(check(&both.body, tol), [], "{what}: the union");
    verified(spar, rib, BooleanOp::Union, &both.body, tol, what);
    assert_eq!(both.body.shells.len(), 2, "{what}: the union's shells");
    let total = v_rib + v_spar;
    assert_close(mass(&both.body).volume, total, slack(total), what);
    for (target, tool, volume, name) in [(rib, spar, v_rib, "rib"), (spar, rib, v_spar, "spar")] {
        let less = boolean(target, tool, BooleanOp::Subtract, tol)
            .unwrap_or_else(|e| panic!("{what}: the {name} less the other: {e}"));
        assert_eq!(
            check(&less.body, tol),
            [],
            "{what}: the {name} less the other"
        );
        verified(target, tool, BooleanOp::Subtract, &less.body, tol, what);
        assert_eq!(less.body.faces.len(), target.faces.len(), "{what}: {name}");
        assert_close(mass(&less.body).volume, volume, slack(volume), what);
    }
    match boolean(spar, rib, BooleanOp::Intersect, tol) {
        Err(KernelError::EmptyResult { .. }) => {}
        other => panic!("{what}: the intersection is not empty: {other:?}"),
    }
    match interference(spar, rib, tol) {
        Ok(Interference::Clear) => {}
        other => panic!("{what}: the interference is not clear: {other:?}"),
    }
}

/// Where the block's face may begin: the nearer place (W, where the plane
/// crosses the upright edge) 1.95 tolerances from the face's edge, square
/// to it, is within twice the tolerance of the face and every boolean is
/// refused at the edge of tolerance, as on `main`; at 2.05 tolerances the
/// two bodies (then 2.05 tolerances apart) are told apart. The repair's
/// test has 1 tolerance beyond the corner (0.91 square) and 3 (2.72).
#[test]
fn the_face_may_begin_twice_the_tolerance_from_the_nearer_place() {
    let c = cos_turn();
    for &k in &SCALES {
        for name in ["as found", "turned"] {
            let frame = if name == "turned" {
                turned(k)
            } else {
                Frame::WORLD
            };
            for d in [1.06, 1.5] {
                for (gap, apart) in [(1.95, false), (2.05, true)] {
                    let what = format!(
                        "{d} tolerances over the corner, W {gap} tolerances from the face's edge, at {k}, {name}"
                    );
                    let (rib, spar, tol) = pair(k, &frame, d, gap / c, (0.0, 1.0), 3.0);
                    println!("corner_beside_review: {what}: {}", said(&rib, &spar, &tol));
                    if apart {
                        told_apart(&rib, &spar, &tol, &what);
                        continue;
                    }
                    for (a, b, op) in [
                        (&spar, &rib, BooleanOp::Union),
                        (&rib, &spar, BooleanOp::Subtract),
                        (&spar, &rib, BooleanOp::Subtract),
                        (&spar, &rib, BooleanOp::Intersect),
                    ] {
                        match boolean(a, b, op, &tol) {
                            Err(KernelError::Unsupported { reason, faces, .. })
                                if reason.ends_with("(at the edge of tolerance)")
                                    && faces.len() == 2 => {}
                            Err(e) => panic!("{what}: {op:?}: {e}"),
                            Ok(_) => panic!("{what}: {op:?} built"),
                        }
                    }
                }
            }
        }
    }
}

/// A corner whose edges lean away from the block's face (0.5 and 1.5 to
/// the left per unit up), the block's bottom 1.5 tolerances over it and its
/// face beginning 1.55 tolerances beyond it: the plane crosses the two
/// edges 1.5 tolerances apart, at 2.08 and 3.44 tolerances from the face's
/// edge, both clear of the face, while the edge that leans 0.5 passes the
/// block's edge 1.90 tolerances from it (the least distance of the two
/// convex bodies, edge to edge). The parts of their union would be closer
/// together than the tolerance can tell apart: the developer's rule of
/// 2026-10-22 refuses it, and `main` did ("two points closer together…").
/// On b77d1956 the union is built, two shells 1.90 tolerances apart, with
/// the check passing (`craft3d-cli`, the same pair 5 high at 1, as found).
#[test]
#[ignore = "fails on b77d1956, the review's finding: the union is built with its parts 1.90 tolerances apart"]
fn a_corner_that_leans_away_from_the_face_within_twice_the_tolerance_of_it_is_not_united() {
    for &k in &[1.0, 1e3] {
        for name in ["as found", "turned"] {
            let frame = if name == "turned" {
                turned(k)
            } else {
                Frame::WORLD
            };
            let what = format!("at {k}, {name}");
            let (rib, spar, tol) = pair(k, &frame, 1.5, 1.55, (0.5, 1.5), 5.0);
            println!(
                "corner_beside_review: leaning {what}: {}",
                said(&rib, &spar, &tol)
            );
            match boolean(&spar, &rib, BooleanOp::Union, &tol) {
                Err(KernelError::Unsupported { reason, .. })
                    if reason.ends_with("(at the edge of tolerance)") => {}
                Err(KernelError::ZeroThickness { .. }) => {}
                Err(e) => panic!("{what}: the union: {e}"),
                Ok(r) => panic!(
                    "{what}: the union is built, {} shells, its parts 1.90 tolerances apart",
                    r.body.shells.len()
                ),
            }
        }
    }
}

/// Which vertex of the united body is nudged.
#[derive(Clone, Copy, Debug)]
enum Nudged {
    /// The far end of the edge that is blended.
    EndOfTheEdge,
    /// The block's far top corner on its right, away from the edge.
    Elsewhere,
}

/// The vertex of `body` at `p`.
fn vertex_at(body: &Body, p: Vec3, near: f64) -> VertexId {
    let index = (0..body.vertices.len())
        .find(|&v| body.vertices[v].point.distance(p) <= near)
        .expect("a vertex there");
    VertexId(index as u32)
}

/// At 0.01: the rib, a prism over a polygon with the corner at the origin,
/// its upright edge and the slanting one at 45 degrees, and an arm at
/// `2 < y < 4` that reaches into the block (so the two are one solid),
/// united with the block, 6 by 5 by 800 and turned 25 degrees, its bottom
/// 9 tolerances over the corner and its face beginning 37 tolerances beyond
/// it; then one vertex nudged 1.5 tolerances up y and its gap recorded, as
/// reading a STEP file records it (a working tolerance of 1.6875
/// tolerances; four times that, the tool's boolean's first try, puts the
/// corner 1.33 of those under the plane and the two places 1.33 apart).
/// With the bottom face's edge on the corner's side, 800 long.
fn notched(nudged: Nudged) -> (Body, EdgeId, Tolerance) {
    let tol = tolerance(1.0);
    let t = tol.linear;
    let outline = polygon(
        1,
        1,
        &[[0.0, 0.0], [0.0, 2.0], [3.0, 2.0], [3.0, 4.0], [-4.0, 4.0]],
    );
    let profiles = detect_profiles(&outline, &tol).unwrap();
    let rib = extrude(&profiles, &Frame::WORLD, 0.0, 1.5, &tol)
        .unwrap()
        .body;
    let (s, c) = 25.0_f64.to_radians().psin_cos();
    let along = Vec3::new(s, 0.0, c);
    let across = Vec3::new(c, 0.0, -s);
    let start = Vec3::new(37.0 * t, 9.0 * t, 0.0) - along * 400.0;
    let spar = block(&Frame::new(start, along, across).unwrap(), 6.0, 5.0, 800.0).unwrap();
    let mut body = boolean(&spar, &rib, BooleanOp::Union, &tol).unwrap().body;
    assert_eq!(body.shells.len(), 1, "the rib and the spar are one solid");
    let near = 1e-9;
    let (from, to) = (
        vertex_at(&body, start, near),
        vertex_at(&body, start + along * 800.0, near),
    );
    let edge = (0..body.edges.len())
        .find(|&e| {
            let edge = &body.edges[e];
            (edge.start == from && edge.end == to) || (edge.start == to && edge.end == from)
        })
        .map(|e| EdgeId(e as u32))
        .expect("the bottom face's edge on the corner's side");
    let moved = match nudged {
        Nudged::EndOfTheEdge => from,
        Nudged::Elsewhere => {
            vertex_at(&body, start + across * 6.0 + Vec3::new(0.0, 5.0, 0.0), near)
        }
    };
    body.vertices[moved.0 as usize].point += Vec3::new(0.0, 1.5 * t, 0.0);
    let recorded = record_vertex(&mut body, moved, &tol, default_loosest(t)).unwrap();
    assert!(recorded.is_some(), "the nudged vertex is loose");
    assert!(body.is_tolerant());
    (body, edge, tol)
}

/// A fillet of radius 1 and a chamfer of 1 along the 800 long edge of a
/// body with a loose vertex, where four times the working tolerance puts a
/// corner of the rib's notch between one and two of those under the plane
/// of the blended face's neighbour, 0.37 mm beyond it: each is built,
/// valid with its table, and takes away what a straight edge's blend does,
/// `(1 - π/4) r² L` and `d² L / 2`. On `main` (1f196da4) the tool's
/// boolean is refused at four times the working tolerance ("two points
/// closer together…"), and tried again at the working tolerance: with the
/// edge's own far end nudged that is refused too ("two faces between half
/// and twice the tolerance apart"), and the fillet and the chamfer fail;
/// with a vertex elsewhere nudged it is built, and the body is the one
/// built here bit for bit (`craft3d-cli` on both, the body written to STEP:
/// the same file but for its name).
#[test]
fn a_fillet_and_a_chamfer_of_a_loose_body_beside_a_corner_of_a_notch() {
    for nudged in [Nudged::EndOfTheEdge, Nudged::Elsewhere] {
        let (body, edge, tol) = notched(nudged);
        let what = format!("{nudged:?}");
        let before = mass(&body).volume;
        let slack = 1e-9 * before;
        let cap = default_loosest(tol.linear);
        let rounded = fillet(&body, &[FilletEdge { edge, radius: 1.0 }], &tol)
            .unwrap_or_else(|e| panic!("{what}: the fillet: {e}"));
        assert_eq!(
            check_capped(&rounded.body, &tol, cap),
            [],
            "{what}: the fillet"
        );
        let taken = before - mass(&rounded.body).volume;
        assert_close(
            taken,
            (1.0 - std::f64::consts::FRAC_PI_4) * 800.0,
            slack,
            &format!("{what}: what the fillet takes away"),
        );
        let first = edge_faces(&body)[edge.0 as usize][0];
        let cut = chamfer(
            &body,
            &[ChamferEdge {
                edge,
                first,
                size: ChamferSize::Distances {
                    first: 1.0,
                    second: 1.0,
                },
            }],
            &tol,
        )
        .unwrap_or_else(|e| panic!("{what}: the chamfer: {e}"));
        assert_eq!(
            check_capped(&cut.body, &tol, cap),
            [],
            "{what}: the chamfer"
        );
        let taken = before - mass(&cut.body).volume;
        assert_close(
            taken,
            400.0,
            slack,
            &format!("{what}: what the chamfer takes away"),
        );
    }
}
