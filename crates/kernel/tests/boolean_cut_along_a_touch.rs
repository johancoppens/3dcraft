//! A cut along the line where a wall thins to nothing (random design
//! 122490; docs/design/booleans.md › *A cut along the line where a wall
//! thins to nothing*).
//!
//! A block (x −5..12.5, y −1.25..16.25, z −5..5) less a washer-shaped
//! groove (the rectangle x 2.5..12.5, y 7.5..10 turned once about y): the
//! groove's outer cylinder, radius 12.5, is tangent to the side x = 12.5
//! along the line z = 0, y 7.5..10, a wall that thins to nothing (accepted,
//! 2026-10-18). Cut by the plane z = 0 (a block standing on it, or the
//! plane cut), what is left above is one shell whose crescent between the
//! side and the cylinder thins to a knife edge along that line: the side
//! and the cylinder meet there at no angle. The section cut the side's
//! curve on the plane only where the side's own edges are, so the curve the
//! cylinder meets the plane in lay over its middle between two vertices it
//! did not have, and the cut face's outline and the groove's loop ran over
//! each other there: the boolean's own check refused what it built
//! (*"loop 10 of face 10 is not a simple closed curve: it runs over loop 11
//! of the face"*). A stretch is now cut where another stretch of one of its
//! faces ends on it (`section::junctions`).
//!
//! Every body is held to `topo::check`, the boolean's full check, one shell
//! and its volume written without the kernel ([`oracle`]), within the
//! tolerance times its area, at scales 1e-3 to 1e6 and turned out of the
//! axes; also the seed's own form (the groove tangent to the top as well,
//! the block standing on the cut coplanar with that top), the groove a
//! plain disc, and the half below the plane.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::print_stdout
)]

mod common;

use common::rectangle;
use craft3d_kernel::boolean::BooleanOp::{self, Intersect, Subtract};
use craft3d_kernel::boolean::{BooleanOptions, VerifyOptions, boolean, verify};
use craft3d_kernel::halfspace::{Kept, keep_below};
use craft3d_kernel::primitives::block;
use craft3d_kernel::profile::detect_profiles;
use craft3d_kernel::revolve::revolve;
use craft3d_math::{Frame, Tolerance, Transform, Vec3};
use craft3d_topo::{Body, check, mass_properties};
use std::f64::consts::TAU;

/// The scales the cases are built at; the tolerance is a millionth of
/// the scale.
const SCALES: [f64; 4] = [1e-3, 1.0, 1e3, 1e6];

fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

/// The volumes written without the kernel, at scale 1.
///
/// The block's part between the heights `z0` and `zt` less the groove's:
/// the groove is a slab 2.5 thick (y 7.5..10) of the region
/// `inner ≤ √(x² + z²) ≤ 12.5` within the block's section x −5..12.5, whose
/// width at height z is `s(12.5) + min(5, s(12.5)) − 2 s(inner)` with
/// `s(r) = √(r² − z²)` (the inner term only where |z| < inner, the outer
/// cylinder reaching past the side x = −5 where s(12.5) > 5), integrated in
/// closed form, ∫ √(r² − z²) dz = (z √(r² − z²) + r² asin(z / r)) / 2.
mod oracle {
    use craft3d_math::Portable;

    /// ∫ √(r² − z²) dz from `a` to `b`, both clamped to [−r, r].
    fn int_sqrt(r: f64, a: f64, b: f64) -> f64 {
        let prim = |z: f64| {
            let z = z.clamp(-r, r);
            0.5 * (z * (r * r - z * z).max(0.0).sqrt() + r * r * (z / r).pasin())
        };
        if b <= a { 0.0 } else { prim(b) - prim(a) }
    }

    pub fn volume(z0: f64, zt: f64, inner: f64) -> f64 {
        let past = 131.25_f64.sqrt();
        let overlap = |lo: f64, hi: f64| (zt.min(hi) - z0.max(lo)).max(0.0);
        let outer = int_sqrt(12.5, z0, zt);
        let within_side = 5.0 * overlap(-past, past)
            + int_sqrt(12.5, z0.max(past), zt.max(past))
            + int_sqrt(12.5, z0.min(-past), zt.min(-past));
        let hole = if inner > 0.0 {
            2.0 * int_sqrt(inner, z0.max(-inner), zt.min(inner))
        } else {
            0.0
        };
        17.5 * 17.5 * (zt - z0) - 2.5 * (outer + within_side - hole)
    }

    /// The same by membership: the groove's width at each of `n` heights,
    /// from the section's own inequalities, summed.
    pub fn by_slices(z0: f64, zt: f64, inner: f64, n: u32) -> f64 {
        let h = (zt - z0) / f64::from(n);
        let width = |z: f64| {
            let s = |r: f64| (r * r - z * z).max(0.0).sqrt();
            let (lo, hi) = ((-5.0_f64).max(-s(12.5)), 12.5_f64.min(s(12.5)));
            let hole = if z.abs() < inner { 2.0 * s(inner) } else { 0.0 };
            (hi - lo).max(0.0) - hole
        };
        let groove: f64 = (0..n)
            .map(|i| width(z0 + (f64::from(i) + 0.5) * h))
            .sum::<f64>()
            * h;
        17.5 * 17.5 * (zt - z0) - 2.5 * groove
    }
}

/// The grooved block at scale `k`, the block from `bottom` to `top`, the
/// groove's section from x = `inner` (0: a plain disc) to 12.5.
fn grooved(k: f64, (bottom, top): (f64, f64), inner: f64) -> Body {
    let tol = tolerance(k);
    let blank = block(
        &Frame::WORLD.at(Vec3::new(-5.0, -1.25, bottom) * k).unwrap(),
        17.5 * k,
        17.5 * k,
        (top - bottom) * k,
    )
    .unwrap();
    let profiles = detect_profiles(
        &rectangle(1, 1, (inner * k, 12.5 * k), (7.5 * k, 10.0 * k)),
        &tol,
    )
    .unwrap();
    let groove = revolve(
        &profiles,
        &Frame::WORLD,
        [0.0, 0.0],
        [0.0, 1.0],
        0.0,
        TAU,
        &tol,
    )
    .unwrap()
    .body;
    boolean(&blank, &groove, Subtract, &tol).unwrap().body
}

/// The block x −5..15, y −1.25..18.75, z 0..`top`; in the seed's frame
/// (its z along y, as `feature.block` with axis y places it) when `seed`.
fn standing(k: f64, top: f64, seed: bool) -> Body {
    if seed {
        let frame = Frame::from_z(Vec3::new(-5.0, -1.25, top) * k, Vec3::Y).unwrap();
        block(&frame, 20.0 * k, top * k, 20.0 * k).unwrap()
    } else {
        let frame = Frame::WORLD.at(Vec3::new(-5.0, -1.25, 0.0) * k).unwrap();
        block(&frame, 20.0 * k, 20.0 * k, top * k).unwrap()
    }
}

fn turn() -> Transform {
    Transform::rotation(Vec3::new(1.0, 2.0, 3.0), 0.7).unwrap()
}

/// What is wrong with `body` as the boolean of `a` and `b` (when given) at
/// scale `k`, if anything.
fn wrong_body(
    label: &str,
    body: &Body,
    operands: Option<(&Body, &Body, BooleanOp)>,
    k: f64,
    expected: f64,
) -> Option<String> {
    let tol = tolerance(k);
    let mut problems: Vec<String> = check(body, &tol).iter().map(ToString::to_string).collect();
    if let Some((a, b, op)) = operands {
        match verify(a, b, op, body, &tol, &VerifyOptions::default()) {
            Ok(found) => problems.extend(found.violations.iter().map(ToString::to_string)),
            Err(e) => problems.push(format!("the boolean's check cannot run: {e}")),
        }
    }
    if body.shells.len() != 1 {
        problems.push(format!("{} shells", body.shells.len()));
    }
    if !problems.is_empty() {
        return Some(format!("{label}: built, but {problems:?}"));
    }
    let mass = mass_properties(body).unwrap();
    let volume = mass.volume / (k * k * k);
    let allowed = 1e-6 * mass.area / (k * k);
    println!("{label}: volume {volume:.9} against {expected:.9}, allowed {allowed:.1e}");
    ((volume - expected).abs() > allowed)
        .then(|| format!("{label}: volume {volume} where {expected} is written without the kernel"))
}

fn wrong(
    label: &str,
    (a, b): (&Body, &Body),
    op: BooleanOp,
    k: f64,
    expected: f64,
) -> Option<String> {
    match boolean(a, b, op, &tolerance(k)) {
        Ok(made) => wrong_body(label, &made.body, Some((a, b, op)), k, expected),
        Err(e) => Some(format!("{label}: refused: {e}")),
    }
}

#[test]
fn the_volumes_written_without_the_kernel_agree_with_a_sum_over_slices() {
    for (z0, zt, inner) in [
        (0.0, 5.0, 2.5),
        (0.0, 12.5, 2.5),
        (0.0, 5.0, 0.0),
        (-5.0, 0.0, 2.5),
    ] {
        let (closed, summed) = (
            oracle::volume(z0, zt, inner),
            oracle::by_slices(z0, zt, inner, 400_000),
        );
        assert!(
            (closed - summed).abs() < 1e-6,
            "{z0}..{zt}, inner {inner}: {closed} in closed form, {summed} by slices"
        );
    }
    // The same closed forms in Python, and to 1e-12 by Gauss–Legendre
    // quadrature of the width, each circle's square root taken out by
    // z = r sin t (`plan/seed-122490/oracle.py` in the workshop).
    assert!((oracle::volume(0.0, 5.0, 2.5) - 1341.316563374929).abs() < 1e-9);
    assert!((oracle::volume(0.0, 12.5, 2.5) - 3393.895405797801).abs() < 1e-9);
}

#[test]
fn a_grooved_block_cut_along_the_line_its_groove_touches_the_side_is_one_shell() {
    let expected = oracle::volume(0.0, 5.0, 2.5);
    let mut found = Vec::new();
    for k in SCALES {
        let (cut, groove) = (standing(k, 10.0, false), grooved(k, (-5.0, 5.0), 2.5));
        found.extend(wrong(
            &format!("k={k}"),
            (&cut, &groove),
            Intersect,
            k,
            expected,
        ));
        found.extend(wrong(
            &format!("k={k} the other way"),
            (&groove, &cut),
            Intersect,
            k,
            expected,
        ));
        let (cut, groove) = (
            cut.transformed(&turn()).unwrap(),
            groove.transformed(&turn()).unwrap(),
        );
        found.extend(wrong(
            &format!("k={k} turned"),
            (&cut, &groove),
            Intersect,
            k,
            expected,
        ));
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
}

#[test]
fn the_plane_cut_along_that_line_is_one_shell() {
    let expected = oracle::volume(0.0, 5.0, 2.5);
    let mut found = Vec::new();
    for k in SCALES {
        let groove = grooved(k, (-5.0, 5.0), 2.5);
        // Kept below a plane whose normal is −z: the part above z = 0.
        let plane = Frame::from_z(Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0)).unwrap();
        let label = format!("k={k}");
        match keep_below(&groove, &plane, &tolerance(k), &BooleanOptions::default()) {
            Ok(Kept::Cut(made)) => found.extend(wrong_body(&label, &made.body, None, k, expected)),
            Ok(Kept::Whole) => found.push(format!("{label}: kept whole")),
            Err(e) => found.push(format!("{label}: refused: {e}")),
        }
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
}

#[test]
fn the_seeds_form_the_groove_tangent_to_the_top_too() {
    // The block from z −2.5 to 12.5: the groove's cylinder is tangent to the
    // top z = 12.5 along x = 0 as well, and the block standing on z = 0 (in
    // the seed's frame) reaches that top, its top on the grooved block's.
    let expected = oracle::volume(0.0, 12.5, 2.5);
    let mut found = Vec::new();
    for k in SCALES {
        let (cut, groove) = (standing(k, 12.5, true), grooved(k, (-2.5, 12.5), 2.5));
        found.extend(wrong(
            &format!("k={k}"),
            (&cut, &groove),
            Intersect,
            k,
            expected,
        ));
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
}

#[test]
fn a_plain_disc_and_the_half_below() {
    let mut found = Vec::new();
    for k in SCALES {
        let cut = standing(k, 10.0, false);
        let disc = grooved(k, (-5.0, 5.0), 0.0);
        found.extend(wrong(
            &format!("k={k} disc"),
            (&cut, &disc),
            Intersect,
            k,
            oracle::volume(0.0, 5.0, 0.0),
        ));
        let groove = grooved(k, (-5.0, 5.0), 2.5);
        found.extend(wrong(
            &format!("k={k} below"),
            (&groove, &cut),
            Subtract,
            k,
            oracle::volume(-5.0, 0.0, 2.5),
        ));
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
}
