//! The second review of `boolean::near` (check/near-approach at 545a21a2,
//! 2026-11-24; docs/design/booleans.md › *Near approaches inside an edge or
//! a face* › *After the review*).
//!
//! Round two reads two walls of one shell at the place between the two
//! points the look found, as `within_a_shell` reads two faces at a place
//! (2026-10-18): the walls are the faces around each entity but a face
//! around both, joined where two of them share a vertex or an edge near the
//! place or an edge whose wedge reaches it (`Topo::joined_at`). Each test
//! here asks one thing of it:
//!
//! - The first review's boss on a wedge's slope beside its knife edge, at
//!   another slope (10 degrees) and in other places (near the end of the
//!   knife edge, a block turned with its corner to the edge, a hole through
//!   the wedge beside it): the place is the wedge's, accepted, as `main`
//!   builds them. Measured with the look and without it (Check
//!   45362576161, a throwaway with a kernel feature that leaves the look
//!   out, as `main`): every answer of this file the same both ways but the
//!   dimple at the plate's middle (below). At 2 and 30 degrees the boss is
//!   not built on `main` either, for reasons not the look's, so those
//!   slopes are not asked here: at 2 degrees, as found, the section refuses
//!   the union (*"two faces between half and twice the tolerance apart"*)
//!   and builds it turned; at 30 degrees, the foot 2.6 to 3.8 tolerances
//!   from the knife edge, `zero_thickness` refuses it (every block, and the
//!   rod as found), its reading through a third face asking the place to be
//!   further than four tolerances from the edge.
//! - A boss whose foot comes within twice the tolerance of the knife edge
//!   itself (a wedge of 60 degrees): a strip of the slope that narrow
//!   between the foot and the edge, refused, on `main` as on the branch
//!   (the block by the section, as two points too near; the rod as zero
//!   thickness).
//! - A web in the band under a wide dimple (a ball of radius 20 cut from a
//!   plate, its lowest point 1.5 tolerances over the bottom): two faces
//!   only the search between two faces can find, by alternating
//!   projections from three seeds. Where a seed lies under the ball's
//!   centre (the dimple at the plate's middle) the search lands on the
//!   lowest point and the web is refused (`main` built it). Off the middle
//!   the projections between a ball of that size and a plane 1.5
//!   tolerances from it close in on the lowest point by a factor of
//!   `1 − 1.5 t / 20` a step, so after their forty steps they stop about
//!   0.22 units over the plane, further than twice the tolerance, and the
//!   golden-section polish, which starts only from a pair within twice the
//!   tolerance, is not tried: the web is built, as on `main`. A wrong body
//!   by the rule of 2026-10-22: that test fails on the branch (4 of 4 in
//!   the band, Check 45362576161), and is ignored as owed until the
//!   search reaches it (run with `--ignored`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::polygon;
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::{BooleanOp, BooleanResult, boolean};
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::primitives;
use craft3d_kernel::profile::detect_profiles;
use craft3d_math::{Frame, Portable, Tolerance, Vec3};
use craft3d_topo::{Body, check, mass_properties};

/// The tolerance at scale `k`: 0.01 at 1, against bodies of 2 to 20 units
/// (the near-approach tests').
fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(0.01 * k, 1e-9).unwrap()
}

/// The frames the bodies are built in, at scale `k`: the world's, and
/// turned (the near-approach tests' frame).
fn frames(k: f64) -> Vec<(&'static str, Frame)> {
    vec![
        ("as found", Frame::WORLD),
        (
            "turned",
            Frame::new(
                Vec3::new(13.0 * k, -7.0 * k, 3.0 * k),
                Vec3::new(0.3, -0.5, 0.81),
                Vec3::new(1.0, 0.2, 0.0),
            )
            .unwrap(),
        ),
    ]
}

fn volume(body: &Body) -> f64 {
    mass_properties(body).unwrap().volume
}

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

/// Built as one shell and checked; its volume `want` where one is given.
fn one_body(r: &Result<BooleanResult, KernelError>, want: Option<f64>, tol: &Tolerance) -> bool {
    match r {
        Ok(r) => {
            r.body.shells.len() == 1
                && check(&r.body, tol).is_empty()
                && want.is_none_or(|want| (volume(&r.body) - want).abs() <= 1e-6 * want)
        }
        Err(_) => false,
    }
}

/// A wedge 10 long at scale `k` in `frame`: its bottom the plane `y = 0`
/// from `x = 0` to 10, its slope rising from the knife edge along z at the
/// origin at `slope` degrees, its back `x = 10`, from `z = 0` to 10 (the
/// first review's wedge, at any slope).
fn wedge(k: f64, frame: &Frame, tol: &Tolerance, slope: f64) -> Body {
    let rise = slope.to_radians().ptan();
    let outline = polygon(
        1,
        1,
        &[[0.0, 0.0], [10.0 * k, 0.0], [10.0 * k, 10.0 * k * rise]],
    );
    let profiles = detect_profiles(&outline, tol).unwrap();
    extrude(&profiles, frame, 0.0, 10.0 * k, tol).unwrap().body
}

/// The kinds of boss stood on the wedge's slope.
#[derive(Clone, Copy, Debug)]
enum Boss {
    /// A block 2 by 2 by 2, one side facing the knife edge.
    Block,
    /// A rod of radius 1 and height 2, its circle's vertex on its far side.
    Rod,
    /// A block 2 by 2 by 2 turned a quarter of a right angle about the
    /// slope's normal, a corner pointing at the knife edge.
    Turned,
}

/// A boss standing on the slope of the wedge of `slope` degrees, its foot
/// lying on the slope and beginning `d` along the slope from the knife
/// edge, its side nearest the edge (or its corner) at `z` (the first
/// review's boss, at any slope and place). The foot's nearest point to the
/// wedge's bottom is `d sin(slope)` over it, on the slope.
fn boss(k: f64, frame: &Frame, d: f64, kind: Boss, slope: f64, z: f64) -> Body {
    let (s, c) = slope.to_radians().psin_cos();
    let along = frame.vector(c, s, 0.0);
    let out = frame.vector(-s, c, 0.0);
    match kind {
        Boss::Block => {
            // From `z` down to `z - 2`.
            let at = frame.point(d * c, d * s, z);
            primitives::block(
                &Frame::new(at, out, along).unwrap(),
                2.0 * k,
                2.0 * k,
                2.0 * k,
            )
            .unwrap()
        }
        Boss::Rod => {
            let at = frame.point((d + k) * c, (d + k) * s, z);
            primitives::cylinder(&Frame::new(at, out, along).unwrap(), k, 2.0 * k).unwrap()
        }
        Boss::Turned => {
            // Both edges from the corner leave it up the slope, at 45
            // degrees to the knife edge.
            let half = std::f64::consts::FRAC_1_SQRT_2;
            let across = frame.vector(0.0, 0.0, 1.0);
            let x = along * half + across * half;
            let at = frame.point(d * c, d * s, z);
            primitives::block(&Frame::new(at, out, x).unwrap(), 2.0 * k, 2.0 * k, 2.0 * k).unwrap()
        }
    }
}

/// The first review's boss on a wedge's slope beside its knife edge
/// (`boolean_near_approach_review.rs`, 5 degrees), at 10 degrees: the
/// foot's rim 1.3 or 1.9 tolerances over the bottom, which is
/// `1.3 t / sin(slope)` to `1.9 t / sin(slope)` along the slope from the
/// knife edge (7.5 to 10.9 tolerances). The rim lies on the slope, so the
/// place is the wedge's: within its margin `2 t / sin(slope / 2)` (22.9
/// tolerances) of the knife edge, which the slope and the bottom share;
/// accepted by the readings of 2026-10-18 (the look finds the foot's
/// corners and rim 1.3 and 1.9 tolerances over the bottom and reads them
/// joined), and `main` builds every one. One shell, checked, its volume
/// the sum. A block and a rod, at 1 and 1e3, as found and turned. (At 2
/// and 30 degrees `main` refuses some of these for reasons not the look's:
/// see the module.)
#[test]
fn a_boss_on_a_wedges_slope_beside_its_knife_edge_is_united_at_ten_degrees() {
    let mut faults = Vec::new();
    for k in [1.0, 1e3] {
        let tol = tolerance(k);
        let t = tol.linear;
        for (name, frame) in frames(k) {
            let slope = 10.0_f64;
            for kind in [Boss::Block, Boss::Rod] {
                for gap in [1.3, 1.9] {
                    let d = gap * t / slope.to_radians().psin();
                    let a = wedge(k, &frame, &tol, slope);
                    let z = if matches!(kind, Boss::Rod) { 5.0 } else { 4.0 };
                    let b = boss(k, &frame, d, kind, slope, z * k);
                    let want = volume(&a) + volume(&b);
                    let r = boolean(&a, &b, BooleanOp::Union, &tol);
                    let what = format!(
                        "a {kind:?} on a slope of {slope} degrees, its foot {gap} tolerances over the bottom, at {k}, {name}"
                    );
                    println!("near_review2: {what}: the union {}", said(&r));
                    if !one_body(&r, Some(want), &tol) {
                        faults.push(format!("{what}: the union {}", said(&r)));
                    }
                }
            }
        }
    }
    assert!(
        faults.is_empty(),
        "{} wrong answers:\n  {}",
        faults.len(),
        faults.join("\n  ")
    );
}

/// The same boss in other places on wedges of 5 and 10 degrees, its foot
/// 1.3 or 1.9 tolerances over the bottom: a block and a rod near the end
/// of the knife edge (the block from `z = 0.3` to 2.3, the rod about
/// `z = 1.3`), a block turned with a corner pointing at the knife edge
/// (the corner the foot's point nearest the bottom), and a hole of radius
/// 0.5 through the wedge square to its slope, its rim that far from the
/// knife edge (the hole's wall a strip that narrow between the slope and
/// the bottom, in the wedge). Each place is the wedge's; `main` builds
/// them. One shell, checked; the unions' volume the sum, the hole's less
/// than the wedge's. At 1 and 1e3, as found and turned.
#[test]
fn a_boss_or_a_hole_elsewhere_beside_the_knife_edge_is_built() {
    let mut faults = Vec::new();
    for k in [1.0, 1e3] {
        let tol = tolerance(k);
        let t = tol.linear;
        for (name, frame) in frames(k) {
            for slope in [5.0_f64, 10.0] {
                for gap in [1.3, 1.9] {
                    let (s, c) = slope.to_radians().psin_cos();
                    let d = gap * t / s;
                    let a = wedge(k, &frame, &tol, slope);
                    let bosses = [
                        ("block near the end", Boss::Block, 2.3),
                        ("rod near the end", Boss::Rod, 1.3),
                        ("block turned", Boss::Turned, 5.0),
                    ];
                    for (which, kind, z) in bosses {
                        let b = boss(k, &frame, d, kind, slope, z * k);
                        let want = volume(&a) + volume(&b);
                        let r = boolean(&a, &b, BooleanOp::Union, &tol);
                        let what = format!(
                            "a {which} on a slope of {slope} degrees, its foot {gap} tolerances over the bottom, at {k}, {name}"
                        );
                        println!("near_review2: {what}: the union {}", said(&r));
                        if !one_body(&r, Some(want), &tol) {
                            faults.push(format!("{what}: the union {}", said(&r)));
                        }
                    }
                    let out = frame.vector(-s, c, 0.0);
                    let along = frame.vector(c, s, 0.0);
                    let radius = 0.5 * k;
                    let centre = frame.point((d + radius) * c, (d + radius) * s, 5.0 * k);
                    let hole = primitives::cylinder(
                        &Frame::new(centre - out * k, out, along).unwrap(),
                        radius,
                        2.0 * k,
                    )
                    .unwrap();
                    let r = boolean(&a, &hole, BooleanOp::Subtract, &tol);
                    let what = format!(
                        "a hole through a slope of {slope} degrees, its rim {gap} tolerances over the bottom, at {k}, {name}"
                    );
                    println!("near_review2: {what}: the subtraction {}", said(&r));
                    let whole = volume(&a);
                    let right = one_body(&r, None, &tol)
                        && r.as_ref()
                            .is_ok_and(|r| volume(&r.body) < whole && volume(&r.body) > 0.0);
                    if !right {
                        faults.push(format!("{what}: the subtraction {}", said(&r)));
                    }
                }
            }
        }
    }
    assert!(
        faults.is_empty(),
        "{} wrong answers:\n  {}",
        faults.len(),
        faults.join("\n  ")
    );
}

/// A boss on a wedge of 60 degrees whose foot begins 1.5 tolerances along
/// the slope from the knife edge (its rim 1.3 tolerances over the bottom):
/// the foot's near corner (the block) or the rim (the rod) and the knife
/// edge are two points of one shell 1.5 tolerances apart, a strip of the
/// slope that narrow between them, which nothing joins there but the slope
/// itself; refused (2026-10-22), as a corner that near the middle of an
/// edge is: on `main` as on the branch, the block by the section (two
/// points too near), the rod as zero thickness (Check 45362576161). With
/// the rim 3 tolerances over the bottom (the foot 3.5 along the slope from
/// the edge) it is built, one shell, checked, its volume the sum. A block
/// and a rod, at 1 and 1e3, as found and turned.
#[test]
fn a_boss_whose_foot_comes_that_near_the_knife_edge_itself_is_refused() {
    let mut faults = Vec::new();
    let slope = 60.0_f64;
    for k in [1.0, 1e3] {
        let tol = tolerance(k);
        let t = tol.linear;
        for (name, frame) in frames(k) {
            for kind in [Boss::Block, Boss::Rod] {
                for gap in [1.3, 3.0] {
                    let d = gap * t / slope.to_radians().psin();
                    let a = wedge(k, &frame, &tol, slope);
                    let z = if matches!(kind, Boss::Rod) { 5.0 } else { 4.0 };
                    let b = boss(k, &frame, d, kind, slope, z * k);
                    let want = volume(&a) + volume(&b);
                    let r = boolean(&a, &b, BooleanOp::Union, &tol);
                    let what = format!(
                        "a {kind:?} on a slope of {slope} degrees, its foot {:.2} tolerances from the knife edge, at {k}, {name}",
                        d / t
                    );
                    println!("near_review2: {what}: the union {}", said(&r));
                    let right = if gap < 2.0 {
                        refused(&r)
                    } else {
                        one_body(&r, Some(want), &tol)
                    };
                    if !right {
                        faults.push(format!("{what}: the union {}", said(&r)));
                    }
                }
            }
        }
    }
    assert!(
        faults.is_empty(),
        "{} wrong answers:\n  {}",
        faults.len(),
        faults.join("\n  ")
    );
}

/// A plate 10 by 10 by 10 at scale `k` in `frame` less a ball of radius
/// 20 whose lowest point lies `gap` tolerances over the plate's bottom at
/// `(x, 5)`: a dish whose bottom is a web that thin there. The ball takes
/// the whole top, and meets the plate's sides at least 0.11 over the
/// bottom; its poles and its seam lie outside the plate.
fn dish(k: f64, frame: &Frame, tol: &Tolerance, gap: f64, x: f64) -> (Body, Body) {
    let plate = primitives::block(frame, 10.0 * k, 10.0 * k, 10.0 * k).unwrap();
    let centre = frame.point(x * k, 20.0 * k + gap * tol.linear, 5.0 * k);
    let ball = primitives::sphere(&frame.at(centre).unwrap(), 20.0 * k).unwrap();
    (plate, ball)
}

/// The dish's web, `gap` tolerances thick, under the ball's lowest point
/// at `x` (5: the plate's middle; 2: off it): refused in the band (1.5),
/// one shell at 3. Nothing but the ball's face and the plate's bottom comes
/// that near (the sides' arcs stay 0.11 and more over the bottom), so only
/// the search between two faces can see it.
fn dish_answers(x: f64) -> Vec<String> {
    let mut faults = Vec::new();
    for k in [1.0, 1e3] {
        let tol = tolerance(k);
        for (name, frame) in frames(k) {
            for gap in [1.5, 3.0] {
                let (plate, ball) = dish(k, &frame, &tol, gap, x);
                let r = boolean(&plate, &ball, BooleanOp::Subtract, &tol);
                let what = format!(
                    "a web {gap} tolerances thick under a dimple of radius 20 at x = {x}, at {k}, {name}"
                );
                println!("near_review2: {what}: the subtraction {}", said(&r));
                let right = if gap < 2.0 {
                    refused(&r)
                } else {
                    one_body(&r, None, &tol)
                };
                if !right {
                    faults.push(format!("{what}: the subtraction {}", said(&r)));
                }
            }
        }
    }
    faults
}

/// Under the plate's middle a seed of the search lies straight under the
/// ball's centre, so its first projection lands on the lowest point: the
/// web is found and refused as too near to tell apart.
#[test]
fn a_web_in_the_band_under_a_wide_dimple_at_the_plates_middle_is_refused() {
    let faults = dish_answers(5.0);
    assert!(
        faults.is_empty(),
        "{} wrong answers:\n  {}",
        faults.len(),
        faults.join("\n  ")
    );
}

/// Off the middle (the lowest point at `x = 2`) every seed of the search
/// lies about 8.5 degrees round the ball from the lowest point, and the
/// alternating projections between the ball and the bottom close in by
/// `1 − 1.5 t / 20` a step: after forty, still 8.3 degrees round, 0.22
/// over the bottom, so no pair within twice the tolerance is found and the
/// polish is not tried. The web is built, as on `main`: a wrong body by the
/// rule of 2026-10-22.
#[test]
#[ignore = "owed, a wrong body main gives too: the search between two faces does not reach a web in the band under a wide dimple off the plate's middle (docs/roadmap.md); fails today: run with --ignored --nocapture"]
fn a_web_in_the_band_under_a_wide_dimple_off_the_plates_middle_is_refused() {
    let faults = dish_answers(2.0);
    assert!(
        faults.is_empty(),
        "{} wrong answers:\n  {}",
        faults.len(),
        faults.join("\n  ")
    );
}
