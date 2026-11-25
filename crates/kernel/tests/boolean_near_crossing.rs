//! A ball and a cylinder inside it that all but touches it from inside:
//! Viviani's figure of eight opened at its crossing point
//! (docs/design/booleans.md › *A stretch given twice beside a crossing
//! point the surfaces are a little apart at*).
//!
//! A ball of radius 2 about the origin and a cylinder of radius
//! `1 − gap` along z through (1, 0, 0), the gap a tenth or a fifth of the
//! tolerance: the cylinder's side passes the ball's face that far inside
//! it at (2, 0, 0), and the two cross in two loops that pass a few tens of
//! tolerances apart there. A ball and a cylinder meet in no closed form,
//! so the boolean asks the march, which reads a crossing point at (2, 0, 0)
//! (the surfaces within the tolerance of each other there) and draws the
//! curves straight to it within its zone. In some boxes the march gave a
//! stretch of the crossing twice, from a seed inside that zone, and the
//! boolean was refused: with the cylinder from z = −2.5 to 0.6, *"imprint
//! lost: a curve in which two faces cross does not divide them; a piece of
//! it is missing"*, and from −1.1 to 0.75 at a fifth of a tolerance,
//! *"two curves leave a point in the same direction and stay together
//! (tangent contact of higher order)"*, for the union, the subtraction and
//! the intersection alike (`main` before 2026-11-25, Check 45664975134).
//!
//! Held to volumes worked out without the kernel: the common volume by
//! the midpoint rule over the cylinder's disc in polar coordinates about
//! its axis (the ball's height there, cut to the cylinder's ends), 2 000 by
//! 4 000 cells, which a rule of twice as many cells in each direction
//! agrees with to 4.4e-8 of the volume; the union, the subtraction and the
//! intersection by the set identities from it, the ball's `4/3 π R³` and
//! the cylinder's `π r² h`. Each result checked, with no defect, and its
//! volume within a millionth of that.

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

/// The volume of the ball of radius `big` about the origin inside the
/// cylinder of radius `small` along z through `(c, 0)` between `zb` and
/// `zt`: the midpoint rule in polar coordinates about the cylinder's axis.
fn common_volume(big: f64, small: f64, c: f64, zb: f64, zt: f64) -> f64 {
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

/// The ball of radius 2 against cylinders of radius `1 − gap` (in
/// tolerances) through (1, 0, 0) from `zb` to `zt`, the vertex of their
/// circles in the direction `seam`, at a tolerance of 1e-3: every boolean
/// built, checked and of the volume the set identities give.
#[test]
fn a_ball_and_a_cylinder_all_but_touching_it_from_inside_are_built() {
    let tol = Tolerance::new(1e-3, 1e-9).unwrap();
    let big = 2.0;
    let ball = primitives::sphere(&Frame::WORLD, big).unwrap();
    let v_ball = 4.0 / 3.0 * PI * big * big * big;
    let (minus_x, plus_y) = (Vec3::new(-1.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0));
    // The settings `main` refused, and one it built (the cylinder through
    // the ball from end to end).
    let cases = [
        (0.1, minus_x, -2.5, 0.6),
        (0.1, plus_y, -2.5, 0.6),
        (0.2, plus_y, -2.5, 0.6),
        (0.2, minus_x, -1.1, 0.75),
        (0.2, plus_y, -1.1, 0.75),
        (0.1, minus_x, -2.5, 2.5),
    ];
    let mut failed = Vec::new();
    for (gap, seam, zb, zt) in cases {
        let small = 1.0 - gap * tol.linear;
        let frame = Frame::new(Vec3::new(1.0, 0.0, zb), Vec3::Z, seam).unwrap();
        let tube = primitives::cylinder(&frame, small, zt - zb).unwrap();
        let v_tube = PI * small * small * (zt - zb);
        let v_common = common_volume(big, small, 1.0, zb, zt);
        for (op, want) in [
            (BooleanOp::Union, v_ball + v_tube - v_common),
            (BooleanOp::Subtract, v_ball - v_common),
            (BooleanOp::Intersect, v_common),
        ] {
            let what = format!(
                "gap {gap} tolerances, seam ({}, {}), cylinder from {zb} to {zt}, {op:?}",
                seam.x, seam.y
            );
            match boolean(&ball, &tube, op, &tol) {
                Err(e) => failed.push(format!("{what}: refused: {e}")),
                Ok(r) => {
                    let defects = check(&r.body, &tol);
                    let got = mass_properties(&r.body).map(|m| m.volume);
                    let off = got
                        .as_ref()
                        .map_or(f64::INFINITY, |g| (g - want).abs() / want);
                    if !defects.is_empty() || !(off <= 1e-6) {
                        failed.push(format!(
                            "{what}: {} defects, volume {got:?} against {want} ({off:e} of it)",
                            defects.len()
                        ));
                    }
                }
            }
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}
