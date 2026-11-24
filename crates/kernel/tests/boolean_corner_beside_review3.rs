//! The third review of the repair of a spar's flat bottom beside a corner
//! of a rib's notch (check/wing-eighth-point at 148036c5, its code
//! 5325a00d, 2026-11-24; docs/design/booleans.md › *A spar's flat bottom
//! beside a corner of a notch*).
//!
//! Two cuts of a curve closer together than the tolerance can tell apart
//! are two cuts, not a refusal, where both are clear outside one face X and
//! every face of X's body is proven further than 18 tolerances from both
//! (`section.rs` › `clip`, `clear_of_body`). Near the two places that
//! holds: nothing within 16 tolerances of them is within twice the
//! tolerance of X's body. But the boolean then goes on past the pair that
//! `main` refused, and meets the rest of the two bodies as `main` would
//! have, had it not stopped there: where an approach of the two bodies
//! that `main` does not see lies elsewhere in the same pair (the second
//! review's beak without the notch: an edge passing another body within
//! the band without crossing it or touching it at a vertex, `main`'s own
//! wrong body, owed in the roadmap), a pair `main` refused is now built
//! with it.
//!
//! The pair: the second review's beaked rib and turned block
//! (`boolean_corner_beside_review2.rs`), the beak and the block moved
//! [`SHIFT`] tolerances to the right together, so that the beak passes the
//! block's edge as it did there (1.8987 tolerances from it), while the
//! notch's two places stay where they were, 33 tolerances from the block
//! (past the 18 the repair asks). `main` (1f196da4, `craft3d-cli`, the same
//! pair by commands at 1, as found; the workshop's
//! `plan/wing-eighth-point/review3/`): with the notch, the interference is
//! unknown and the union and both subtracts are refused, "two points closer
//! together than the tolerance can tell apart", naming the block's bottom
//! and the rib's face `z = 0`; without the notch, the interference is clear
//! and every boolean is built (`main`'s blind spot, as in the second
//! review). The first test fails where the union is built: a wrong body
//! that `main` does not give for this pair.
//!
//! The second test takes a face of the first body that is not flat to a
//! told-apart answer (the second review's rod, its round side 37
//! tolerances from the places), which none of the repair's tests does: it
//! shows the second of the four ways a face is proven far
//! (`clear_of_surface`) at work, not only refusing.

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

/// How far the beak and the block are moved to the right of where the
/// second review had them, in tolerances: the block's face then begins 37
/// tolerances beyond the notch's corner, as the wing's spar does.
const SHIFT: f64 = 31.95;

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

/// The union of the pair, in either order, is not built: refused, in
/// whatever words. Built, it is two parts `gap` tolerances apart, which the
/// developer's rule of 2026-10-22 refuses and `main` refused.
fn not_united(rib: &Body, other: &Body, tol: &Tolerance, gap: f64, what: &str) {
    for (a, b, order) in [(other, rib, "block and rib"), (rib, other, "rib and block")] {
        match boolean(a, b, BooleanOp::Union, tol) {
            Err(e) => {
                println!("corner_beside_review3: {what}: the union of {order} is refused: {e}")
            }
            Ok(r) => panic!(
                "{what}: a wrong body: the union of {order} is built, {} shells (its check {}), its parts {gap:.4} tolerances apart; main refused it (two points closer together than the tolerance can tell apart)",
                r.body.shells.len(),
                if check(&r.body, tol).is_empty() {
                    "passes"
                } else {
                    "fails"
                },
            ),
        }
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

/// A rib with a notch and, far beyond it, a beak, 1.5 thick along z, at
/// scale `k` in `frame` (`t` the tolerance). The notch: the edge from
/// (-5, 5) down to the corner at the origin (45 degrees) and the edge from
/// the corner up to (t, 5t), 0.2 to the right per unit up; the plane
/// `y = 1.5t` crosses them at H (-1.5t) and W (0.3t), 1.8 tolerances
/// apart. From (t, 5t) the rib's top runs level, 3.5 tolerances over that
/// plane, to (32.95t, 5t), where the beak's edge goes down to the beak's
/// tip at (35.45t, 0), leaning 0.5 to the left per unit up (the second
/// review's beak moved [`SHIFT`] tolerances to the right); the plane
/// crosses it at x = 34.7t, 34.4 tolerances from W. Then down to (-1, -3)
/// and round by (-5, -3). Without the notch (`notch` false) the outline
/// goes from (-5, 5) to (t, 5t) at once, and the plane crosses the beak's
/// edge alone near the block.
fn far_beaked_rib(k: f64, frame: &Frame, tol: &Tolerance, notch: bool) -> Body {
    let t = tol.linear;
    let (far, top) = ([-5.0 * k, 5.0 * k], [t, 5.0 * t]);
    let (shoulder, tip) = ([(1.0 + SHIFT) * t, 5.0 * t], [(3.5 + SHIFT) * t, 0.0]);
    let (low, back) = ([-k, -3.0 * k], [-5.0 * k, -3.0 * k]);
    let outline = if notch {
        polygon(1, 1, &[far, [0.0, 0.0], top, shoulder, tip, low, back])
    } else {
        polygon(1, 1, &[far, top, shoulder, tip, low, back])
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

/// An approach `main` does not see, elsewhere in a pair `main` refused at
/// two places too near to tell apart. The block's bottom 1.5 tolerances
/// over the corner of [`far_beaked_rib`]'s notch, its face beginning 37
/// tolerances beyond it: H and W, 1.8 tolerances apart, are clear outside
/// the block's bottom face, and every face of the block is further than 18
/// tolerances from both (W is 33.3 tolerances from the bottom face's edge
/// and from the block's side, square to them), so the two cuts are let
/// through (`clear_of_body`). 34 tolerances further on, the beak's edge
/// passes the block's edge 1.8987 tolerances from it, under the plane,
/// with the beak's tip 1.55 tolerances (along x) beside the block's edge
/// and 1.5 under its bottom: as in the second review's pair without the
/// notch, which `main` builds (interference clear, the union two parts
/// 1.90 tolerances apart). `main` refused this pair at H and W. At 1 and
/// 1e3, as found and turned; printed besides without the notch (`main`'s
/// path, which the repair leaves as it was).
#[test]
fn a_beak_far_from_two_places_told_apart_beside_the_edge_of_the_block_is_not_united() {
    let gap = least_gap(0.5, 1.5, 1.55);
    let mut all = Vec::new();
    for (k, name, frame) in settings() {
        let tol = tolerance(k);
        for notch in [true, false] {
            let rib = far_beaked_rib(k, &frame, &tol, notch);
            let spar = turned_block(k, &frame, &tol, 1.5, 5.05 + SHIFT);
            let what = format!(
                "a beak {gap:.4} tolerances from the block's edge, {}, at {k}, {name}",
                if notch {
                    "34 tolerances beyond a notch whose places are 1.8 tolerances apart and 33 from the block"
                } else {
                    "without the notch"
                }
            );
            println!("corner_beside_review3: {what}: {}", said(&rib, &spar, &tol));
            if notch {
                all.push((what, rib, spar, tol));
            }
        }
    }
    for (what, rib, spar, tol) in &all {
        not_united(rib, spar, tol, gap, what);
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

/// The second review's rod (`boolean_corner_beside_review2.rs` ›
/// `flat_rod`): radius 2, 20 long, its axis turned 25 degrees about y as
/// the repair's block is, with a flat along its underside in the plane
/// `y = d` tolerances, 20 tolerances under its axis, at scale `k` in
/// `frame`, beside [`prism_rib`]: its round side passes the rib's upright
/// edge `gap` tolerances from it, at the height of the axis and `z = 0`.
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

/// The second of the four ways a face of the first body is proven far
/// (`clear_of_surface`), taken to an answer: none of the repair's tests
/// tells two places apart beside a first body with a face that is not
/// flat. The second review's rod with its flat 1.5 tolerances over the
/// rib's corner, its round side 37 tolerances from the rib's upright edge
/// rather than 1.9: the flat's edge is 38 tolerances from W, and W is 37.8
/// tolerances from the rod's cylinder by its distance from the axis (H
/// 39.2), past the 31.2 (18 times the square root of 3) that the cylinder
/// must keep from the middle of the cube about the place. `main` refuses
/// the union and both subtracts at H and W (`craft3d-cli`, 1f196da4, the
/// same pair by commands at 1, as found; its interference is clear). Here
/// the union is two shells, its check passing, and the interference clear.
/// At 1 and 1e3, as found and turned.
#[test]
fn a_rod_whose_round_side_is_far_from_two_places_beside_its_flat_is_told_apart() {
    for (k, name, frame) in settings() {
        let tol = tolerance(k);
        let rib = prism_rib(k, &frame, &tol);
        let rod = flat_rod(k, &frame, &tol, 1.5, 37.0);
        let what = format!(
            "a rod with a flat 1.5 tolerances over the corner, its round side 37 tolerances from the rib's upright edge, at {k}, {name}"
        );
        let both = boolean(&rod, &rib, BooleanOp::Union, &tol)
            .unwrap_or_else(|e| panic!("{what}: the union: {e}"));
        assert_eq!(check(&both.body, &tol), [], "{what}: the union");
        assert_eq!(both.body.shells.len(), 2, "{what}: the union's shells");
        match interference(&rod, &rib, &tol) {
            Ok(Interference::Clear) => {}
            other => panic!("{what}: the interference is not clear: {other:?}"),
        }
    }
}
