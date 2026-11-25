//! The review of the dimple and the caps (2026-11-25; the workshop's
//! `plan/near-dimple/review/REVIEW.md`; docs/design/booleans.md › *Near
//! approaches inside an edge or a face* › *The dimple and the caps*).
//!
//! Two parts whose nearest points lie inside a face of each, 1.5
//! tolerances apart, touch by the rule of 2026-10-15: their union is
//! refused as zero thickness and their interference is not clear.
//!
//! - A ring whose axis leans 8e-10 rad from a plate's normal, less than
//!   the 1e-9 under which `near.rs` › `from_plane` takes the ring's own x
//!   for the part of the plate's normal square to the axis: its pairs then
//!   lie a quarter turn round the ring from the lowest point, 2.4
//!   tolerances higher, and the seeds of the search land there too. (The
//!   section takes such a ring as lying flat: `craft3d_geom`'s
//!   `plane_torus` below the angular tolerance, 1e-9 by default. Leaning
//!   2e-9 rad the section marches and refuses, *"tangent or nearly
//!   coincident over too large an area"*, at 1.5 and at 3 tolerances alike,
//!   so that is no control: Check 45622243924.)
//! - A ring slid on a rod one tolerance off the rod's axis, its inside 1.5
//!   tolerances from the rod on one side and 3.5 on the other (a cylinder
//!   and a torus: no closed form). Every seed lands on the far side, where
//!   the alternating projections settle at once (least across the ring,
//!   greatest round it), so no descent is tried from them.
//! - A ring of radius 30 000 whose axis leans 8e-9 rad from the world's z,
//!   sunk 140 tolerances into a block: `craft3d_topo`'s box of a torus
//!   face reaches `R √(1 − z²) + r` along z (`transform.rs` ›
//!   `face_extent`), which is `r` once the axis's z component rounds to 1,
//!   so the box misses the ring's lowest point by `R sin α` (240
//!   tolerances), and the two bodies' boxes do not meet. On `main` too.
//!
//! All three failed on the review's branch, built as two shells with the
//! interference clear (Check 45622243924), and were ignored as owed; all
//! three pass since 2026-11-25 (`check/near-look-3`; the workshop's
//! `plan/near-look-3/`): `from_plane` takes a ring as flat only to the
//! rounding of the part of the normal square to its axis (1e-12), the
//! search between two faces looks for a way down from every pair where the
//! projections settle (`near.rs` › `escape`), and the box of a torus face
//! reaches `R √(z_j² + z_k²) + r` along each axis. Run with `--nocapture`
//! for the answers.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::{BooleanOp, BooleanResult, Interference, boolean, interference};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Portable, Tolerance, Vec3};
use craft3d_topo::Body;

/// What a boolean said, in a word.
fn said(r: &Result<BooleanResult, KernelError>) -> String {
    match r {
        Ok(r) => format!("built ({} shells)", r.body.shells.len()),
        Err(KernelError::ZeroThickness { .. }) => "zero thickness".to_string(),
        Err(KernelError::Unsupported { reason, .. }) => format!("refused: {reason}"),
        Err(e) => format!("error: {e}"),
    }
}

/// A refusal of the band: too near to tell apart, or zero thickness.
fn refused(r: &Result<BooleanResult, KernelError>) -> bool {
    matches!(
        r,
        Err(KernelError::Unsupported { .. } | KernelError::ZeroThickness { .. })
    )
}

/// The interference in a word.
fn contact(found: &Result<Interference, KernelError>) -> &'static str {
    match found {
        Ok(Interference::Clear) => "clear",
        Ok(Interference::Touch { .. }) => "touch",
        Ok(Interference::Overlap(_)) => "overlap",
        Err(_) => "unknown",
    }
}

/// The near-approach tests' turned frame, at the world's origin: a ring
/// centred there has a box exactly about it, whose middle is the origin.
fn turned() -> Frame {
    Frame::new(
        Vec3::ZERO,
        Vec3::new(0.3, -0.5, 0.81),
        Vec3::new(1.0, 0.2, 0.0),
    )
    .unwrap()
}

/// A ring of radius 3000 and tube 10 centred at the turned frame's origin,
/// its axis leaning `lean` rad from the frame's z towards its −y (the
/// ring's own x is the frame's), over a plate square to the frame's z
/// whose top lies `gap` tolerances under the ring's lowest point: that
/// point is `R sin(lean) + r` under the centre, a quarter turn round from
/// the ring's x (towards the frame's −y). The plate reaches from −3500 to
/// 2900 along the frame's x and from −3200 to 3200 along its y, 10 thick:
/// its box holds the ring's, and its edge at 2900 passes under the tube 75
/// degrees round from the lowest point, `0.74 R sin(lean)` higher. The
/// ring first (A).
fn ring_over_plate(lean: f64, gap: f64, tol: &Tolerance) -> (Body, Body) {
    let f = turned();
    let (major, minor) = (3000.0, 10.0);
    let (s, c) = lean.psin_cos();
    let ring = primitives::torus(
        &Frame::new(Vec3::ZERO, f.vector(0.0, -s, c), f.x()).unwrap(),
        major,
        minor,
    )
    .unwrap();
    let top = -(minor + major * s) - gap * tol.linear;
    let plate = primitives::block(
        &f.at(f.point(-3500.0, -3200.0, top - 10.0)).unwrap(),
        6400.0,
        6400.0,
        10.0,
    )
    .unwrap();
    (ring, plate)
}

/// A ring leaning 8e-10 rad from a plate's normal, its lowest point 1.5
/// tolerances over the plate inside both faces: the closed form's pairs
/// lay where the ring's own x points while `from_plane` took a ring
/// leaning less than 1e-9 as flat, 1.5 + 2.4 tolerances over the plate,
/// and so do the projections from the three seeds (the ring's box middle
/// is its centre, whose projection onto the ring is at its x; the plate's
/// box middle lies along the frame's −x). Refused, by the rule; built
/// before 2026-11-25, the interference clear: the pairs `from_plane` gave
/// missed the isolated pair of the least distance by up to `2 R × 1e-9`.
#[test]
fn a_ring_leaning_less_than_a_billionth_of_a_radian_over_a_plate_in_the_band_is_not_united() {
    let tol = Tolerance::default();
    let (ring, plate) = ring_over_plate(8e-10, 1.5, &tol);
    let union = boolean(&ring, &plate, BooleanOp::Union, &tol);
    let between = interference(&ring, &plate, &tol);
    let answer = format!(
        "the union {}, the interference {}",
        said(&union),
        contact(&between)
    );
    println!("near_dimple_review: a ring leaning 8e-10 rad, 1.5 tolerances over a plate: {answer}");
    assert!(
        refused(&union) && matches!(contact(&between), "touch" | "unknown"),
        "a ring leaning 8e-10 rad, 1.5 tolerances over a plate: {answer}"
    );
}

/// A rod of radius 1 along the world's x from −5 to 5 (its wall's
/// parameter starting at +y), and a ring about it (axis along x, major
/// radius 2) centred one tolerance from the rod's axis towards +y, its
/// inside `1 + 2.5 t` from the ring's axis: 1.5 tolerances from the rod at
/// −y, 3.5 at +y, 2.5 at ±z. The rod first (A).
fn ring_on_rod(tol: &Tolerance) -> (Body, Body) {
    let t = tol.linear;
    let rod = primitives::cylinder(
        &Frame::new(Vec3::new(-5.0, 0.0, 0.0), Vec3::X, Vec3::Y).unwrap(),
        1.0,
        10.0,
    )
    .unwrap();
    let inside = 1.0 + 1.5 * t + t;
    let ring = primitives::torus(
        &Frame::new(Vec3::new(0.0, t, 0.0), Vec3::X, Vec3::Z).unwrap(),
        2.0,
        2.0 - inside,
    )
    .unwrap();
    (rod, ring)
}

/// A ring slid on a rod, one tolerance off its axis, 1.5 tolerances from
/// it at the nearest (a cylinder and a torus, no closed form). The three
/// seeds (the middles of the faces' boxes and of their common part: the
/// rod's axis twice, and the ring's centre) all land on the rod at +y, the
/// far side, where the alternating projections settle in their first step
/// 3.5 tolerances apart: a pair square to both surfaces, least across the
/// ring and greatest round it, and no edge or vertex comes near. Refused,
/// by the rule; before 2026-11-25, when no descent was tried from a
/// settled end, the interference was clear (the rod first, so its wall is
/// the search's first face) and the union, whose first face is the rod's
/// too, built.
#[test]
fn a_ring_slid_on_a_rod_off_its_axis_in_the_band_is_not_united() {
    let tol = Tolerance::new(0.01, 1e-9).unwrap();
    let (rod, ring) = ring_on_rod(&tol);
    let union = boolean(&rod, &ring, BooleanOp::Union, &tol);
    let between = interference(&rod, &ring, &tol);
    let answer = format!(
        "the union {}, the interference {}",
        said(&union),
        contact(&between)
    );
    println!("near_dimple_review: a ring on a rod, 1.5 tolerances at the nearest: {answer}");
    assert!(
        refused(&union) && matches!(contact(&between), "touch" | "unknown"),
        "a ring on a rod, 1.5 tolerances at the nearest: {answer}"
    );
}

/// A ring of radius 30 000 and tube 10 about the origin, its axis leaning
/// 8e-9 rad from the world's z towards −y (its z component rounds to 1),
/// so its lowest point is `(0, −R, −(R sin α + r))`, z = −10.00024; a block
/// 200 by 200 under it whose top is at z = −10.0001. The ring sinks 140
/// tolerances into the block along 200 units: they overlap, so the union
/// is one body (or refused) and the interference an overlap (or unknown).
/// Before 2026-11-25 `craft3d_topo`'s box of the ring ended at z = −10 (`R
/// √(1 − z²) + r`, z = 1), 100 tolerances over the block's top, so the
/// bodies' boxes did not meet: the union was the two bodies as two shells
/// that overlap, and the interference clear (on `main` too).
#[test]
fn a_large_ring_leaning_a_hundred_millionth_of_a_radian_sunk_into_a_block_is_not_left_two_parts() {
    let tol = Tolerance::default();
    let (s, c) = 8e-9_f64.psin_cos();
    let ring = primitives::torus(
        &Frame::new(Vec3::ZERO, Vec3::new(0.0, -s, c), Vec3::X).unwrap(),
        30000.0,
        10.0,
    )
    .unwrap();
    let block = primitives::block(
        &Frame::WORLD.at(Vec3::new(-100.0, -30100.0, -30.0)).unwrap(),
        200.0,
        200.0,
        19.9999,
    )
    .unwrap();
    let union = boolean(&ring, &block, BooleanOp::Union, &tol);
    let between = interference(&ring, &block, &tol);
    let two = union.as_ref().is_ok_and(|r| r.body.shells.len() == 2);
    let answer = format!(
        "the union {}, the interference {}",
        said(&union),
        contact(&between)
    );
    println!("near_dimple_review: a ring sunk 140 tolerances into a block: {answer}");
    assert!(
        !two && contact(&between) != "clear",
        "a ring sunk 140 tolerances into a block: {answer}"
    );
}
