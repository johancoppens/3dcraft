//! The second review of the repair of a spar's flat bottom beside a corner
//! of a rib's notch (check/wing-eighth-point as amended after its first
//! review, 96e14ddc, 2026-11-24; docs/design/booleans.md › *A spar's flat
//! bottom beside a corner of a notch*).
//!
//! Two cuts of a curve closer together than the tolerance can tell apart
//! are two cuts, not a refusal, where both are clear outside one flat face
//! X of one body and the edges of the other body that carry them (whole),
//! the edges at an end of either that lies within twice the tolerance of
//! X's plane, and the curve between the two places are proven further than
//! twice the tolerance from X (`section.rs` › `clip`, `beside_the_face`,
//! `clear_of_face`). `main` refuses every such pair: "two points closer
//! together than the tolerance can tell apart (at the edge of tolerance)".
//!
//! Two pairs whose union the repair builds with its parts closer together
//! than twice the tolerance, where `main` refuses it (the developer's rule
//! of 2026-10-22 refuses faces of exact bodies between half and twice the
//! tolerance apart). Both tests fail on 96e14ddc: a wrong body each. Since
//! round three (`section.rs` › `clear_of_body`: every face of the first
//! body further than 18 tolerances from both places) both pairs are
//! refused at the two places, as on `main`, and both tests pass.
//!
//! - The proof measures X, not the body X bounds. A rod with a flat along
//!   its underside, turned as the repair's block is, whose round side
//!   bulges a tolerance beyond the flat's edge, beside the corner of the
//!   repair's rib: the rib's upright edge, which carries one of the two
//!   places, passes the round side 1.9 tolerances from it and keeps 2.9
//!   from X.
//! - The edges at an end further than twice the tolerance from X's plane
//!   are left out. A beak of the rib, from the far end of the edge that
//!   carries one place (3.5 tolerances over the plane): the beak's edge
//!   passes the block's edge 1.90 tolerances from it, under the plane, as
//!   the first review's leaning corner did; the plane crosses it 2.45
//!   tolerances from the nearer of the two places.
//!
//! Measured. On 96e14ddc each union is built, two shells, its check
//! passing, with the interference clear and both subtracts the targets, at
//! 1 and 1e3, as found and turned (Check 45288463969; the beak also on
//! Check 45287851413). On `main` (1f196da4, `craft3d-cli`, the same pairs
//! by commands at 1, as found) the interference is unknown and every
//! boolean is refused, "two points closer together than the tolerance can
//! tell apart", naming X and the rib's face `z = 0`. Where no two places
//! are that close (the rod's flat 2.5 tolerances over the corner; the beak
//! without the notch) `main` answers both, interference clear and the
//! union built: a near approach `main` does not see either, hidden there by
//! its refusal of the two places.
//!
//! Each test prints, before it asserts, every pair it builds and the same
//! pair where no two places are that close (`main`'s path, which the repair
//! leaves as it was).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{arc, line, polygon};
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::{BooleanOp, Interference, boolean, interference};
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::primitives::block;
use craft3d_kernel::profile::detect_profiles;
use craft3d_math::{Frame, Portable, Tolerance, Vec3};
use craft3d_topo::{Body, check};

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

/// The scales and frames every pair is built at.
fn settings() -> Vec<(f64, &'static str, Frame)> {
    let mut all = Vec::new();
    for k in [1.0, 1e3] {
        all.push((k, "as found", Frame::WORLD));
        all.push((k, "turned", turned(k)));
    }
    all
}

/// What the three booleans and the interference of the pair say, in a
/// word each, for the table a run prints.
fn said(rib: &Body, other: &Body, tol: &Tolerance) -> String {
    let word = |r: Result<usize, KernelError>| match r {
        Ok(shells) => format!("built ({shells} shells)"),
        Err(KernelError::EmptyResult { .. }) => "empty".to_string(),
        Err(KernelError::Unsupported { reason, faces, .. }) => {
            format!("refused: {reason} ({} faces)", faces.len())
        }
        Err(e) => format!("error: {e}"),
    };
    let op = |a: &Body, b: &Body, op| word(boolean(a, b, op, tol).map(|r| r.body.shells.len()));
    let contact = match interference(other, rib, tol) {
        Ok(Interference::Clear) => "clear".to_string(),
        Ok(Interference::Touch { .. }) => "touch".to_string(),
        Ok(Interference::Overlap(_)) => "overlap".to_string(),
        Err(e) => word(Err(e)),
    };
    format!(
        "union {}; rib less other {}; other less rib {}; intersect {}; interference {contact}",
        op(other, rib, BooleanOp::Union),
        op(rib, other, BooleanOp::Subtract),
        op(other, rib, BooleanOp::Subtract),
        op(other, rib, BooleanOp::Intersect),
    )
}

/// The union of the pair is not built: refused, in whatever words. Built,
/// it is two parts `gap` tolerances apart, which the developer's rule of
/// 2026-10-22 refuses and `main` refused.
fn not_united(rib: &Body, other: &Body, tol: &Tolerance, gap: f64, what: &str) {
    match boolean(other, rib, BooleanOp::Union, tol) {
        Err(e) => println!("corner_beside_review2: {what}: the union is refused: {e}"),
        Ok(r) => panic!(
            "{what}: a wrong body: the union is built, {} shells (its check {}), its two parts {gap:.4} tolerances apart; main refused it (two points closer together than the tolerance can tell apart)",
            r.body.shells.len(),
            if check(&r.body, tol).is_empty() {
                "passes"
            } else {
                "fails"
            },
        ),
    }
}

/// The rib of the repair's test: a prism over the triangle (0, 0), (0, 5),
/// (-5, 5), 1.5 thick along z, at scale `k` in `frame`. The plane `y = d`
/// tolerances crosses its upright edge at W and its slanting one at H, `d`
/// tolerances apart.
fn prism_rib(k: f64, frame: &Frame, tol: &Tolerance) -> Body {
    let h = 5.0 * k;
    let outline = polygon(1, 1, &[[0.0, 0.0], [0.0, h], [-h, h]]);
    let profiles = detect_profiles(&outline, tol).unwrap();
    extrude(&profiles, frame, 0.0, 1.5 * k, tol).unwrap().body
}

/// How far the round side of [`flat_rod`] bulges beyond the edge of its
/// flat, in tolerances: a radius of 200 tolerances, the flat 20 under the
/// axis.
fn bulge() -> f64 {
    200.0 - (200.0_f64 * 200.0 - 20.0 * 20.0).sqrt()
}

/// A rod of radius 2, 20 long, its axis turned 25 degrees about y as the
/// repair's block is, with a flat along its underside in the plane `y = d`
/// tolerances, 20 tolerances under its axis, at scale `k` in `frame`,
/// beside [`prism_rib`]: its round side passes the rib's upright edge `gap`
/// tolerances from it, at the height of the axis and `z = 0`, and recedes
/// from the rib with z; the edge of the flat, X's boundary on that side, is
/// `gap` and [`bulge`] tolerances from W, and meets the rib's face `z = 0`
/// `(gap + bulge) / cos 25°` tolerances beside W. (Not turned, the flat's
/// box keeps more than twice the tolerance from that face's box and the
/// pair is never met, on `main` as here: Check 45287851413.)
fn flat_rod(k: f64, frame: &Frame, tol: &Tolerance, d: f64, gap: f64) -> Body {
    let t = tol.linear;
    let (s, c) = 25.0_f64.to_radians().psin_cos();
    let along = frame.vector(s, 0.0, c);
    let across = frame.vector(c, 0.0, -s);
    let turned = Frame::new(frame.origin(), along, across).unwrap();
    let radius = 2.0 * k;
    let under = 20.0 * t;
    let half = (radius * radius - under * under).sqrt();
    let centre = [radius + gap * t, d * t + under];
    let left = [centre[0] - half, d * t];
    let right = [centre[0] + half, d * t];
    let profile = [line(11, left, right), arc(12, centre, right, left)];
    let profiles = detect_profiles(&profile, tol).unwrap();
    extrude(&profiles, &turned, -10.0 * k, 10.0 * k, tol)
        .unwrap()
        .body
}

/// The proof measures the flat face X, not the body X bounds. The rib's
/// corner 1.5 tolerances under the plane of a rod's flat (the rod turned
/// as the repair's block is), the plane crossing the rib's upright edge at
/// W and its slanting edge at H, 1.5 tolerances apart: both places, the
/// rib's edges and its corner are 2.9 tolerances or more from the flat
/// (proven so by `clear_of_face`), and the two cuts are let through; but
/// the rod's round side, which bulges 1.0025 tolerances beyond the flat's
/// edge 20 tolerances over the flat, passes the rib's upright edge 1.9
/// tolerances from it. No vertex of either is near that place and no edge
/// crosses a face of the other there, so the union's check of zero
/// thickness has nothing to look at. `main` refuses the pair (H and W too
/// near). At 1 and 1e3, as found and turned; printed besides with the plane
/// 2.5 tolerances over the corner, where H and W are not too near (`main`'s
/// path).
#[test]
fn a_carrying_edge_beside_the_round_side_of_a_rod_with_a_flat_is_not_united() {
    let gap = 1.9;
    let mut all = Vec::new();
    for (k, name, frame) in settings() {
        let tol = tolerance(k);
        for d in [1.5, 2.5] {
            let rib = prism_rib(k, &frame, &tol);
            let rod = flat_rod(k, &frame, &tol, d, gap);
            let what = format!(
                "a rod with a flat {d} tolerances over the corner, its round side {gap} tolerances from the rib's upright edge, the flat's edge {:.4} from W, at {k}, {name}",
                gap + bulge()
            );
            println!("corner_beside_review2: {what}: {}", said(&rib, &rod, &tol));
            if d < 2.0 {
                all.push((what, rib, rod, tol));
            }
        }
    }
    for (what, rib, rod, tol) in &all {
        not_united(rib, rod, tol, gap, what);
    }
}

/// The least distance, in tolerances, between the edge that leans `lean`
/// to the left per unit up from a tip `beside` tolerances (along x) left of
/// the block's edge at `z = 0` and `d` tolerances under the block's bottom
/// plane, from the tip to where the plane crosses it, and the line of the
/// block's edge, 25 degrees off z (`boolean_corner_beside.rs` ›
/// `least_gap`).
fn least_gap(lean: f64, d: f64, beside: f64) -> f64 {
    let c = 25.0_f64.to_radians().psin_cos().1;
    let u = ((d - c * c * lean * beside) / (1.0 + c * c * lean * lean)).clamp(0.0, d);
    (c * (lean * u + beside)).phypot(d - u)
}

/// A rib with a notch and a beak, 1.5 thick along z, at scale `k` in
/// `frame` (`t` the tolerance). The notch: the edge from (-5, 5) down to
/// the corner at the origin (45 degrees) and the edge from the corner up to
/// (t, 5t), 0.2 to the right per unit up; the plane `y = 1.5t` crosses them
/// at H and W, 1.8 tolerances apart. From (t, 5t), 3.5 tolerances over that
/// plane, the beak's edge goes down to the beak's tip at (3.5t, 0), leaning
/// 0.5 to the left per unit up as the first review's corner did; the plane
/// crosses it at x = 2.75t, 2.45 tolerances from W. Then down to (-1, -3)
/// and round by (-5, -3). Without the notch (`notch` false) the outline
/// goes from (-5, 5) to (t, 5t) at once, and the plane crosses the beak's
/// edge alone near there.
fn beaked_rib(k: f64, frame: &Frame, tol: &Tolerance, notch: bool) -> Body {
    let t = tol.linear;
    let (far, top, tip) = ([-5.0 * k, 5.0 * k], [t, 5.0 * t], [3.5 * t, 0.0]);
    let (low, back) = ([-k, -3.0 * k], [-5.0 * k, -3.0 * k]);
    let outline = if notch {
        polygon(1, 1, &[far, [0.0, 0.0], top, tip, low, back])
    } else {
        polygon(1, 1, &[far, top, tip, low, back])
    };
    let profiles = detect_profiles(&outline, tol).unwrap();
    extrude(&profiles, frame, 0.0, 1.5 * k, tol).unwrap().body
}

/// The block of the repair's test, 6 by 3 by 800, turned 25 degrees about y
/// as a swept spar is: its bottom the plane `y = d` tolerances, its bottom
/// face beginning `beside` tolerances to the right of the origin at `z = 0`
/// and receding with z.
fn turned_block(k: f64, frame: &Frame, tol: &Tolerance, d: f64, beside: f64) -> Body {
    let t = tol.linear;
    let (s, c) = 25.0_f64.to_radians().psin_cos();
    let along = frame.vector(s, 0.0, c);
    let across = frame.vector(c, 0.0, -s);
    let start = frame.point(beside * t, d * t, 0.0) - along * (400.0 * k);
    block(
        &Frame::new(start, along, across).unwrap(),
        6.0 * k,
        3.0 * k,
        800.0 * k,
    )
    .unwrap()
}

/// The edges at an end further than twice the tolerance from X's plane are
/// left out. The block's bottom 1.5 tolerances over the corner of
/// [`beaked_rib`]'s notch, its face beginning 5.05 tolerances beyond it:
/// H and W, 1.8 tolerances apart, the notch's edges and its corner are 4.2
/// tolerances or more from the block's bottom face (proven so), and the two
/// cuts are let through; the beak's edge leaves the far end of the edge
/// that carries W, 3.5 tolerances over the plane (not within the guard, so
/// not looked at), and under the plane passes the block's edge 1.90
/// tolerances from it (the beak's tip 1.55 tolerances beyond the face's
/// beginning and 1.5 under the plane, as the first review's corner). Its
/// own place is 2.45 tolerances from W and 2.08 from the face's edge, the
/// tip 2.06 from that edge, and no rib face is within twice the tolerance
/// of where the block's edge meets the rib's plane (2.06): the union's
/// check of zero thickness has nothing to look at. `main` refused the pair
/// (H and W too near). At 1 and 1e3, as found and turned; printed besides
/// without the notch (`main`'s path).
#[test]
fn an_edge_from_the_far_end_of_a_carrying_edge_beside_the_edge_of_the_block_is_not_united() {
    let gap = least_gap(0.5, 1.5, 1.55);
    let mut all = Vec::new();
    for (k, name, frame) in settings() {
        let tol = tolerance(k);
        for notch in [true, false] {
            let rib = beaked_rib(k, &frame, &tol, notch);
            let spar = turned_block(k, &frame, &tol, 1.5, 5.05);
            let what = format!(
                "a beak {gap:.4} tolerances from the block's edge, {}, at {k}, {name}",
                if notch {
                    "beside a notch whose places are 1.8 tolerances apart"
                } else {
                    "without the notch"
                }
            );
            println!("corner_beside_review2: {what}: {}", said(&rib, &spar, &tol));
            if notch {
                all.push((what, rib, spar, tol));
            }
        }
    }
    for (what, rib, spar, tol) in &all {
        not_united(rib, spar, tol, gap, what);
    }
}
