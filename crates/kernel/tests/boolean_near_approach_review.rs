//! The review of `boolean::near` (check/near-approach at 59ec3f42,
//! 2026-11-24; docs/design/booleans.md › *Near approaches inside an edge or
//! a face*).
//!
//! The look refuses two walls of one shell between half and twice the
//! tolerance apart, except where an edge joins a vertex of the one to a
//! vertex of the other (`hop`), which its texts call a sliver beside a
//! vertex that the wedge readings of 2026-10-18 accept. The readings
//! themselves are not asked: `hop` is read from the topology alone. Two
//! consequences, each a test here:
//!
//! - A wedge that thins to nothing (the developer's decision of
//!   2026-10-18, accepted; `within_a_shell` › `wedged` reads it) with a
//!   boss standing on its slope beside the knife edge: the rim of the
//!   boss lies on the slope, so it is as near the other face of the wedge
//!   as the slope is there, but it is not joined to that face by an edge.
//!   The look refuses the union as too near to tell apart, though every
//!   point of the rim is a point of the wedge's own slope.
//! - A web one and a half tolerances thick between a plate's bottom and a
//!   channel cut across the whole plate (a large cylinder): the channel's
//!   face and the bottom are joined at the plate's sides by edges 65
//!   tolerances long, so the pair hops and the web is built, five units
//!   from any vertex. The same web under a hole that does not reach the
//!   sides is refused. The developer's rule of 2026-10-22 refuses both.
//!
//! And one that holds: far from the origin the band is read as near it.

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

/// The frames the pairs are built in, at scale `k`: the world's, turned
/// (the near-approach tests' frame), and turned far from the origin.
fn frames(k: f64) -> Vec<(&'static str, Frame)> {
    let z = Vec3::new(0.3, -0.5, 0.81);
    let x = Vec3::new(1.0, 0.2, 0.0);
    vec![
        ("as found", Frame::WORLD),
        (
            "turned",
            Frame::new(Vec3::new(13.0 * k, -7.0 * k, 3.0 * k), z, x).unwrap(),
        ),
        (
            "turned, 1e5 away",
            Frame::new(Vec3::new(1e5 * k, -3e4 * k, 2e4 * k), z, x).unwrap(),
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

/// Built as one shell, checked, its volume `want`.
fn one_body(r: &Result<BooleanResult, KernelError>, want: f64, tol: &Tolerance) -> bool {
    match r {
        Ok(r) => {
            r.body.shells.len() == 1
                && check(&r.body, tol).is_empty()
                && (volume(&r.body) - want).abs() <= 1e-6 * want
        }
        Err(_) => false,
    }
}

/// The slope of the wedge, in degrees.
const SLOPE: f64 = 5.0;

/// A wedge 10 by 10 at scale `k` in `frame`: its bottom the plane `y = 0`
/// from `x = 0` to 10, its slope rising from the knife edge along z at the
/// origin at [`SLOPE`] degrees, its back `x = 10`, from `z = 0` to 10.
fn wedge(k: f64, frame: &Frame, tol: &Tolerance) -> Body {
    let rise = SLOPE.to_radians().ptan();
    let outline = polygon(
        1,
        1,
        &[[0.0, 0.0], [10.0 * k, 0.0], [10.0 * k, 10.0 * k * rise]],
    );
    let profiles = detect_profiles(&outline, tol).unwrap();
    extrude(&profiles, frame, 0.0, 10.0 * k, tol).unwrap().body
}

/// A boss standing on the wedge's slope, its foot lying on the slope and
/// beginning `d` along the slope from the knife edge: a block 2 by 2 by 2
/// from `z = 2` to 4, or a rod of radius 1 and height 2 about `z = 5`
/// whose circle's vertex is on its far side. The foot's nearest point to
/// the wedge's bottom is `d sin(SLOPE)` over it, inside the slope.
fn boss(k: f64, frame: &Frame, d: f64, block: bool) -> Body {
    let (s, c) = SLOPE.to_radians().psin_cos();
    let along = frame.vector(c, s, 0.0);
    let out = frame.vector(-s, c, 0.0);
    if block {
        let at = frame.point(d * c, d * s, 4.0 * k);
        primitives::block(
            &Frame::new(at, out, along).unwrap(),
            2.0 * k,
            2.0 * k,
            2.0 * k,
        )
        .unwrap()
    } else {
        let at = frame.point((d + k) * c, (d + k) * s, 5.0 * k);
        primitives::cylinder(&Frame::new(at, out, along).unwrap(), k, 2.0 * k).unwrap()
    }
}

/// A wedge that thins to nothing at its knife edge, and a boss on its
/// slope beside that edge (see the module): the rim of the boss's foot
/// lies on the slope `gap` tolerances over the bottom, which is what the
/// slope itself is there. The faces of the wedge meet at the knife edge
/// at 5 degrees, so they stay within twice the tolerance of each other
/// for `2 t / sin 5°`, 0.23 units at 1 (`wedged` reads them joined for
/// twice that); the rim begins 0.15 units (at 1.3 tolerances) along the
/// slope. The union is the wedge and the boss, one shell, as `main`
/// builds it; at 3 tolerances it is built in any case.
#[test]
fn a_boss_on_a_wedges_slope_beside_its_knife_edge_is_united() {
    let mut faults = Vec::new();
    for k in [1.0, 1e3] {
        let tol = tolerance(k);
        let t = tol.linear;
        for (name, frame) in frames(k) {
            for block in [true, false] {
                for gap in [1.3, 1.9, 3.0] {
                    let d = gap * t / SLOPE.to_radians().psin_cos().0;
                    let a = wedge(k, &frame, &tol);
                    let b = boss(k, &frame, d, block);
                    let want = volume(&a) + volume(&b);
                    let r = boolean(&a, &b, BooleanOp::Union, &tol);
                    let what = format!(
                        "a {} on the slope, its foot {gap} tolerances over the bottom, at {k}, {name}",
                        if block { "block" } else { "rod" }
                    );
                    println!("near_review: {what}: the union {}", said(&r));
                    if !one_body(&r, want, &tol) {
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

/// A plate 10 by 10 by 10 at scale `k` in `frame`, less a cylinder along z
/// whose lowest line runs `gap` tolerances over the plate's bottom: a
/// channel of radius 20 across the whole plate (its face meets both sides
/// `x = 0` and `x = 10`, 0.65 over the bottom), or a hole of radius 1
/// about `x = 5` that reaches neither side. The cylinder's vertex is at
/// its top, outside the plate.
fn web(k: f64, frame: &Frame, tol: &Tolerance, gap: f64, channel: bool) -> (Body, Body) {
    let t = tol.linear;
    let radius = if channel { 20.0 * k } else { k };
    let plate = primitives::block(frame, 10.0 * k, 10.0 * k, 10.0 * k).unwrap();
    let base = frame.point(5.0 * k, radius + gap * t, -k);
    let axis = frame.vector(0.0, 0.0, 1.0);
    let up = frame.vector(0.0, 1.0, 0.0);
    let cutter =
        primitives::cylinder(&Frame::new(base, axis, up).unwrap(), radius, 12.0 * k).unwrap();
    (plate, cutter)
}

/// A web `gap` tolerances thick between the plate's bottom and the
/// cylinder cut from it (see [`web`]). Within twice the tolerance it is
/// two walls of one shell in the band, five units from any vertex: refused
/// (the developer's rule of 2026-10-22, as the look refuses it under the
/// hole). Under the channel the look reads the pair as a sliver beside a
/// vertex, the channel's face and the bottom being joined by the plate's
/// upright edges at its sides (0.65 long), and the web is built, as on
/// `main`. At 3 tolerances both are built.
#[test]
fn a_web_in_the_band_under_a_channel_is_refused_as_under_a_hole() {
    let mut faults = Vec::new();
    for k in [1.0, 1e3] {
        let tol = tolerance(k);
        for (name, frame) in frames(k) {
            for channel in [false, true] {
                for gap in [1.5, 3.0] {
                    let (plate, cutter) = web(k, &frame, &tol, gap, channel);
                    let r = boolean(&plate, &cutter, BooleanOp::Subtract, &tol);
                    let what = format!(
                        "a web {gap} tolerances thick under a {}, at {k}, {name}",
                        if channel { "channel" } else { "hole" }
                    );
                    println!("near_review: {what}: the subtraction {}", said(&r));
                    let wrong = if gap < 2.0 {
                        !refused(&r)
                    } else {
                        !matches!(&r, Ok(r) if r.body.shells.len() == 1
                            && check(&r.body, &tol).is_empty())
                    };
                    if wrong {
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

/// Far from the origin the band is read as near it: two balls (radii 3
/// and 2) and two rods crossing square to each other (radius 2), `gap`
/// tolerances apart, a million units from the origin at 1e-3, 1 and 1e6
/// (the rods' vertices on their far sides). In the band the union is
/// refused (zero thickness, or a refusal of the section's own); at 2.5
/// tolerances it is two parts.
#[test]
fn far_from_the_origin_the_band_is_read_as_near_it() {
    let mut faults = Vec::new();
    for k in [1e-3, 1.0, 1e6] {
        let tol = tolerance(k);
        let t = tol.linear;
        let far = Frame::new(
            Vec3::new(1e6 * k, -4e5 * k, 7e5 * k),
            Vec3::new(0.3, -0.5, 0.81),
            Vec3::new(1.0, 0.2, 0.0),
        )
        .unwrap();
        for gap in [1.1, 1.9, 2.5] {
            let balls = (
                primitives::sphere(&far, 3.0 * k).unwrap(),
                primitives::sphere(
                    &far.at(far.point(5.0 * k + gap * t, 0.0, 0.0)).unwrap(),
                    2.0 * k,
                )
                .unwrap(),
            );
            let rods = (
                primitives::cylinder(
                    &Frame::new(
                        far.point(-10.0 * k, 0.0, 0.0),
                        far.vector(1.0, 0.0, 0.0),
                        far.vector(0.0, 0.0, -1.0),
                    )
                    .unwrap(),
                    2.0 * k,
                    20.0 * k,
                )
                .unwrap(),
                primitives::cylinder(
                    &Frame::new(
                        far.point(0.0, -10.0 * k, 4.0 * k + gap * t),
                        far.vector(0.0, 1.0, 0.0),
                        far.vector(0.0, 0.0, 1.0),
                    )
                    .unwrap(),
                    2.0 * k,
                    20.0 * k,
                )
                .unwrap(),
            );
            for (pair, (a, b)) in [("two balls", balls), ("two rods across", rods)] {
                let r = boolean(&a, &b, BooleanOp::Union, &tol);
                let what = format!("{pair} {gap} tolerances apart, 1e6 away, at {k}");
                println!("near_review: {what}: the union {}", said(&r));
                let right = if gap < 2.0 {
                    refused(&r)
                } else {
                    matches!(&r, Ok(r) if r.body.shells.len() == 2)
                };
                if !right {
                    faults.push(format!("{what}: the union {}", said(&r)));
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
