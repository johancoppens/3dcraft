//! The review of `check/near-crossing-twice` (2026-11-25), in booleans: a
//! crossing point read where the surfaces are a little apart, met by the
//! union, the subtraction and the intersection (docs/design/booleans.md ›
//! *A stretch given twice beside a crossing point the surfaces are a
//! little apart at*).
//!
//! - A ball of radius 2 about the origin and cylinders of radius
//!   `1 + poke` (`poke` from half a tolerance inside the ball's face to half
//!   a tolerance outside it) along z through (1, 0, 0), the seam of the
//!   cylinder through the point (2, 0, 0) where they all but touch, and a
//!   quarter turn and a half turn away from it, cut at four heights.
//!   Inside, Viviani's figure opened into two loops; outside, into one
//!   curve (the opened figures the branch moved the bits of).
//! - Tori about z (`R/ρ` 1.05, 1.2, 2 and 4) and a block whose face is the
//!   plane `x = R − ρ + δ`, `δ` up to half a tolerance either way: the
//!   plane all but touches the torus at its inner equator, where the
//!   figure of eight in which it crosses it would cross itself. A plane
//!   along a torus's axis meets it in no closed form, so the boolean asks
//!   the march.
//!
//! Each result checked, with no defect, and its volume within a millionth
//! (and 2e-5, `built`) of the set identities on volumes worked out without
//! the kernel; the torus's intersection, two lobes that touch at the point
//! within the tolerance, refused as of zero thickness. The volumes: the
//! ball's common volume with the cylinder by a one-dimensional integral in
//! polar coordinates about the ball's centre (the inner integral along the
//! radius in closed form; the angle by the midpoint rule after a change of
//! variable that takes the square roots out of the ends), and the torus's
//! volume on one side of the plane likewise about its axis. The first is
//! also held against the quadrature of `boolean_near_crossing.rs` (the
//! builder's two-dimensional midpoint rule, copied here), and against
//! Viviani's `(2R³/3)(π − 4/3)` where the cylinder is the ball's half
//! radius and passes through the ball from end to end.
//!
//! Measured (2026-11-25; `main` ed93bafc with these tests, Check
//! 45711338836; the branch, Check 45711343773): of the 96 balls and
//! cylinders, 30 were refused on `main` for all three operations (*"imprint
//! lost"*, *"two curves leave a point in the same direction"*, *"a point
//! too close to a face boundary"*, *"could not be fitted"*); with the
//! branch every one is built. Of the
//! 48 tori and blocks, 12 unions and subtractions were refused on `main`
//! and are built with the branch; every intersection is refused as of zero
//! thickness with the branch (36 on `main`, the other 12 for other
//! reasons). Where both build, 8 settings (24 results) moved their bits,
//! each within 2.6e-7 of the exact volume on both. The builder's quadrature
//! is 4.2e-8 to 8.9e-8 of the volume above the exact one, a twentieth of
//! the millionth it is held to.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
// `!(x <= y)` is deliberate: unlike `x > y` it is also true for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Portable, Tolerance, Vec3};
use craft3d_topo::{check, mass_properties};
use std::f64::consts::{PI, TAU};

/// A line on the test's output that the harness does not hold back.
fn say(line: &str) {
    let _ = std::io::Write::write_all(&mut std::io::stderr(), format!("{line}\n").as_bytes());
}

/// `∫₀^r min(c, √(4 − s²)) s ds`: the ball of radius 2 about the origin,
/// cut at the height `c > 0`, over a disc of radius `r` about its axis
/// (`(1/3)(4 − s²)^{3/2}` is a primitive of `−√(4 − s²) s`).
fn under_cap(r: f64, c: f64) -> f64 {
    let r = r.clamp(0.0, 2.0);
    let rc = (4.0 - c * c).max(0.0).sqrt();
    let cube = |x: f64| {
        let w = (4.0 - x * x).max(0.0);
        w * w.sqrt()
    };
    if r <= rc {
        0.5 * c * r * r
    } else {
        0.5 * c * rc * rc + (cube(rc) - cube(r)) / 3.0
    }
}

/// The volume of the ball of radius 2 about the origin inside the cylinder
/// of radius `rho` along z through (1, 0) between `zb < 0` and `zt > 0`.
/// In polar coordinates about the ball's centre the cylinder's disc is
/// `r² − 2 r cos θ + 1 − ρ² ≤ 0`, between `r± = cos θ ± √(cos² θ − 1 + ρ²)`;
/// the height there is `min(zt, h) + min(−zb, h)`, `h = √(4 − r²)`, whose
/// integral along the radius is `under_cap` twice. Where the disc leaves
/// out the centre (`ρ < 1`) the angle runs over `|θ| < θm`,
/// `cos θm = √(1 − ρ²)`, taken as `θ = −θm cos τ` so that the square root
/// at its ends goes as `τ`; where it holds it, over the whole turn.
fn common_exact(rho: f64, zb: f64, zt: f64) -> f64 {
    let height = |r: f64| under_cap(r, zt) + under_cap(r, -zb);
    let n = 400_000_u32;
    let mut total = 0.0;
    if rho < 1.0 {
        let tm = (1.0 - rho * rho).sqrt().pacos();
        let dtau = PI / f64::from(n);
        for i in 0..n {
            let tau = (f64::from(i) + 0.5) * dtau;
            let (st, ct) = tau.psin_cos();
            let theta = -tm * ct;
            let c = theta.pcos();
            let root = (c * c - 1.0 + rho * rho).max(0.0).sqrt();
            total += (height(c + root) - height(c - root)) * tm * st;
        }
        total * dtau
    } else {
        let dtheta = TAU / f64::from(n);
        for i in 0..n {
            let theta = -PI + (f64::from(i) + 0.5) * dtheta;
            let c = theta.pcos();
            let root = (c * c - 1.0 + rho * rho).max(0.0).sqrt();
            total += height(c + root);
        }
        total * dtheta
    }
}

/// The builder's quadrature (`boolean_near_crossing.rs` › `common_volume`,
/// copied): the ball of radius `big` about the origin inside the cylinder
/// of radius `small` along z through `(c, 0, 0)` between `zb` and `zt`, by
/// the midpoint rule in polar coordinates about the cylinder's axis.
fn common_builders(big: f64, small: f64, c: f64, zb: f64, zt: f64) -> f64 {
    let (ns, nphi) = (2000_u32, 4000_u32);
    let ds = small / f64::from(ns);
    let dphi = TAU / f64::from(nphi);
    let mut total = 0.0;
    for j in 0..nphi {
        let cp = ((f64::from(j) + 0.5) * dphi).pcos();
        let mut ring = 0.0;
        for i in 0..ns {
            let s = (f64::from(i) + 0.5) * ds;
            let h2 = big * big - (c * c + s * s + 2.0 * c * s * cp);
            if h2 <= 0.0 {
                continue;
            }
            let h = h2.sqrt();
            ring += (zt.min(h) - zb.max(-h)).max(0.0) * s;
        }
        total += ring;
    }
    total * ds * dphi
}

/// The volume of the torus about z (major radius `major`, minor `minor`)
/// where `x ≤ x0`: about its axis at the radius `r = R + ρ sin t` the tube
/// is `2ρ cos t` high and `x ≤ x0` over an angle of `2π − 2 acos(x0 / r)`
/// (all of the turn where `r ≤ x0`), so the volume is
/// `∫ 2ρ² cos² t · r · angle dt` over `|t| ≤ π/2`, by the midpoint rule.
fn torus_up_to(major: f64, minor: f64, x0: f64) -> f64 {
    let n = 1_000_000_u32;
    let dt = PI / f64::from(n);
    let mut total = 0.0;
    for i in 0..n {
        let t = -0.5 * PI + (f64::from(i) + 0.5) * dt;
        let (s, c) = t.psin_cos();
        let r = major + minor * s;
        let angle = if x0 >= r {
            TAU
        } else if x0 <= -r {
            0.0
        } else {
            TAU - 2.0 * (x0 / r).pacos()
        };
        total += 2.0 * minor * minor * c * c * r * angle;
    }
    total * dt
}

/// Boolean `op` of `a` and `b` against the volume `want`: `None` where it
/// is built, checked with no defect and within a millionth of it and
/// `2e-5` (twenty cubic tolerances at a tolerance of 1e-3: where the
/// surfaces are half a tolerance apart beside the point they lie within the
/// tolerance of each other over a cross some hundredths of a unit across,
/// over which a face may lie anywhere within the tolerance; on `main` four
/// of those intersections are 1.2e-6 to 2.8e-6 of their volume off, the
/// same bits with the branch); else why not.
fn built(
    a: &craft3d_topo::Body,
    b: &craft3d_topo::Body,
    op: BooleanOp,
    want: f64,
    tol: &Tolerance,
    what: &str,
) -> Option<String> {
    let line = match boolean(a, b, op, tol) {
        Err(e) => format!("refused: {e}"),
        Ok(r) => {
            let defects = check(&r.body, tol);
            let got = mass_properties(&r.body).map(|m| m.volume);
            let off = got
                .as_ref()
                .map_or(f64::INFINITY, |g| (g - want).abs() / want);
            let fine = defects.is_empty() && off * want <= 1e-6 * want + 2e-5;
            let said = format!(
                "{} defects, volume {got:?} against {want} ({off:e} of it)",
                defects.len()
            );
            if fine {
                say(&format!("REVIEW-BOOL {what}, {op:?}: built, {said}"));
                return None;
            }
            said
        }
    };
    say(&format!("REVIEW-BOOL {what}, {op:?}: {line}"));
    Some(format!("{what}, {op:?}: {line}"))
}

/// The ball of radius 2 against cylinders of radius `1 + poke` through
/// (1, 0, 0): the seam towards the point where they all but touch, a
/// quarter turn and a half turn away; cut at four heights.
#[test]
fn a_ball_and_a_cylinder_all_but_touching_it_inside_or_outside_are_built() {
    let tol = Tolerance::new(1e-3, 1e-9).unwrap();
    let ball = primitives::sphere(&Frame::WORLD, 2.0).unwrap();
    let v_ball = 4.0 / 3.0 * PI * 8.0;
    let mut failed = Vec::new();
    for poke in [-0.5, -0.2, -0.1, 0.1, 0.2, 0.5] {
        let rho = 1.0 + poke * tol.linear;
        for (zb, zt) in [(-2.5, 2.5), (-2.5, 0.6), (-1.1, 0.75), (-0.3, 2.5)] {
            let v_common = common_exact(rho, zb, zt);
            for seam in [
                Vec3::X,
                Vec3::Y,
                Vec3::new(-1.0, 0.0, 0.0),
                Vec3::new(0.0, -1.0, 0.0),
            ] {
                let frame = Frame::new(Vec3::new(1.0, 0.0, zb), Vec3::Z, seam).unwrap();
                let tube = primitives::cylinder(&frame, rho, zt - zb).unwrap();
                let v_tube = PI * rho * rho * (zt - zb);
                let what = format!(
                    "ball and cylinder poking {poke} tolerances out, seam ({}, {}), from {zb} to {zt}",
                    seam.x, seam.y
                );
                for (op, want) in [
                    (BooleanOp::Union, v_ball + v_tube - v_common),
                    (BooleanOp::Subtract, v_ball - v_common),
                    (BooleanOp::Intersect, v_common),
                ] {
                    if let Some(why) = built(&ball, &tube, op, want, &tol, &what) {
                        failed.push(why);
                    }
                }
            }
        }
    }
    say(&format!("REVIEW-BOOL-FAILED {}", failed.len()));
    assert!(
        failed.is_empty(),
        "{} not built right:\n{}",
        failed.len(),
        failed.join("\n")
    );
}

/// Tori and a block whose face all but touches the torus at its inner
/// equator, the torus's seam through the point and a quarter turn away.
#[test]
fn a_torus_and_a_block_all_but_touching_it_at_its_inner_equator_are_built() {
    let tol = Tolerance::new(1e-3, 1e-9).unwrap();
    let mut failed = Vec::new();
    for major in [1.05, 1.2, 2.0, 4.0] {
        let minor = 1.0;
        let v_torus = 2.0 * PI * PI * major * minor * minor;
        for delta in [-0.5, -0.2, -0.1, 0.1, 0.2, 0.5] {
            let x0 = major - minor + delta * tol.linear;
            let v_in = torus_up_to(major, minor, x0);
            let reach = major + minor + 1.0;
            let corner = Vec3::new(-reach, -reach, -reach);
            let (dx, dy) = (x0 + reach, 2.0 * reach);
            let block =
                primitives::block(&Frame::new(corner, Vec3::Z, Vec3::X).unwrap(), dx, dy, dy)
                    .unwrap();
            let v_block = dx * dy * dy;
            for seam in [Vec3::X, Vec3::Y] {
                let frame = Frame::new(Vec3::ZERO, Vec3::Z, seam).unwrap();
                let ring = primitives::torus(&frame, major, minor).unwrap();
                let what = format!(
                    "torus {major} and block {delta} tolerances into it, seam ({}, {})",
                    seam.x, seam.y
                );
                for (op, want) in [
                    (BooleanOp::Union, v_torus + v_block - v_in),
                    (BooleanOp::Subtract, v_torus - v_in),
                ] {
                    if let Some(why) = built(&ring, &block, op, want, &tol, &what) {
                        failed.push(why);
                    }
                }
                // Where the plane is `δ` beside the saddle the torus on the
                // axis's side of it is two lobes (`δ < 0`) or two lobes
                // joined by a neck `δ` thick (`δ > 0`): within the
                // tolerance they touch at the point, and the boolean
                // refuses the intersection as of zero thickness there.
                match boolean(&ring, &block, BooleanOp::Intersect, &tol) {
                    Err(e) if e.to_string().contains("zero thickness") => {
                        say(&format!(
                            "REVIEW-BOOL {what}, Intersect: refused as it is to be: {e}"
                        ));
                    }
                    other => {
                        let line = match other {
                            Err(e) => format!("refused: {e}"),
                            Ok(r) => format!(
                                "built, {} defects, volume {:?} (the lobes' {v_in})",
                                check(&r.body, &tol).len(),
                                mass_properties(&r.body).map(|m| m.volume)
                            ),
                        };
                        say(&format!("REVIEW-BOOL {what}, Intersect: {line}"));
                        failed.push(format!("{what}, Intersect: {line}"));
                    }
                }
            }
        }
    }
    say(&format!("REVIEW-BOOL-FAILED {}", failed.len()));
    assert!(
        failed.is_empty(),
        "{} not built right:\n{}",
        failed.len(),
        failed.join("\n")
    );
}

/// The oracles agree: the exact common volume against Viviani's closed
/// form, and the builder's quadrature against the exact volume, for the
/// cylinders of `boolean_near_crossing.rs` and of the test above.
#[test]
fn the_volumes_the_booleans_are_held_to_agree() {
    let viviani = 2.0 * 8.0 / 3.0 * (PI - 4.0 / 3.0);
    let exact = common_exact(1.0, -2.5, 2.5);
    say(&format!(
        "REVIEW-VOLUME Viviani's {viviani}, the exact integral {exact} ({:e} of it)",
        (exact - viviani).abs() / viviani
    ));
    assert!((exact - viviani).abs() <= 1e-12 * viviani);
    let mut worst: f64 = 0.0;
    for poke in [-0.5, -0.2, -0.1, 0.1, 0.2, 0.5] {
        let rho = 1.0 + poke * 1e-3;
        for (zb, zt) in [(-2.5, 2.5), (-2.5, 0.6), (-1.1, 0.75), (-0.3, 2.5)] {
            let ours = common_exact(rho, zb, zt);
            let theirs = common_builders(2.0, rho, 1.0, zb, zt);
            let off = (theirs - ours).abs() / ours;
            worst = worst.max(off);
            say(&format!(
                "REVIEW-VOLUME radius {rho}, from {zb} to {zt}: exact {ours}, the builder's quadrature {theirs} ({off:e} of it)"
            ));
        }
    }
    assert!(
        worst <= 1e-7,
        "the builder's quadrature is {worst:e} off the exact volume"
    );
}
