//! The mass properties of a freeform face with many holes (2026-10-15,
//! the lamp's finding M1: a shade of revolution with tens of cells cut
//! through it could not be measured, "a boundary integral that does not
//! settle", once one face had some ten holes).
//!
//! A domed block (a NURBS roof over a block, `craft3d_testkit`) has thirty
//! upright rods cut through it, roof and floor, at three scales and
//! turned. The roof is then one face with thirty-one loops. Every rod
//! takes the column of the block under the roof over its disc, so the
//! volume left is the block's less thirty integrals of the roof's height
//! over a disc, which the test sums itself (the roof is the height field
//! of its control net: Bernstein polynomials written out). The loops of
//! such a face are integrated each on its own (`topo::mass`), each until
//! two sums in a row agree.

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod boolean_common;

use boolean_common::*;
use craft3d_geom::Surface;
use craft3d_kernel::boolean::BooleanOp::{Subtract, Union};
use craft3d_math::{Transform, Vec3};
use craft3d_testkit::trimmed::domed_block;
use std::f64::consts::PI;

const DX: f64 = 4.0;
const DY: f64 = 3.0;
const HEIGHT: f64 = 2.0;
const RADIUS: f64 = 0.12;

/// The roof's height above `(x, y)`, at scale 1 (the test kit's net).
fn roof_height(x: f64, y: f64) -> f64 {
    let bernstein = |i: usize, t: f64| {
        let s = 1.0 - t;
        match i {
            0 => s * s * s,
            1 => 3.0 * s * s * t,
            2 => 3.0 * s * t * t,
            _ => t * t * t,
        }
    };
    let (u, v) = (x / DX, y / DY);
    let mut z = 0.0;
    for i in 0..4 {
        for j in 0..4 {
            let height = HEIGHT * (1.0 + 0.15 * (((i * 7 + j * 3) % 5) as f64 - 2.0));
            z += bernstein(i, u) * bernstein(j, v) * height;
        }
    }
    z
}

/// The integral of the roof's height over a disc, in polar coordinates
/// by the midpoint rule (the integrand is smooth: far below 1e-8).
fn over_disc(center: (f64, f64), radius: f64) -> f64 {
    const N: usize = 400;
    let (dr, da) = (radius / N as f64, 2.0 * PI / N as f64);
    let mut sum = 0.0;
    for i in 0..N {
        let r = (i as f64 + 0.5) * dr;
        for j in 0..N {
            let a = (j as f64 + 0.5) * da;
            sum += roof_height(center.0 + r * a.cos(), center.1 + r * a.sin()) * r;
        }
    }
    sum * dr * da
}

fn centres() -> Vec<(f64, f64)> {
    let mut all = Vec::new();
    for i in 0..6 {
        for j in 0..5 {
            all.push((0.4 + 0.64 * f64::from(i), 0.35 + 0.57 * f64::from(j)));
        }
    }
    all
}

#[test]
fn a_spline_roof_with_thirty_holes_is_measured() {
    let taken: f64 = centres().iter().map(|&c| over_disc(c, RADIUS)).sum();
    let turn = |k: f64| {
        Transform::rotation_about(
            Vec3::new(1.0, -2.0, 0.5) * k,
            Vec3::new(0.4, 0.7, -0.3),
            0.9,
        )
        .unwrap()
    };
    for k in SCALES {
        let dome = domed_block(
            &craft3d_math::Frame::WORLD,
            DX * k,
            DY * k,
            HEIGHT * k,
            true,
        )
        .unwrap();
        let k3 = k * k * k;
        let mut rods = None;
        for (x, y) in centres() {
            let rod = rod(k, [x, y, -1.0], RADIUS, 5.0);
            rods = Some(match rods {
                None => rod,
                Some(all) => solve(&all, &rod, Union, k).body,
            });
        }
        let rods = rods.unwrap();
        for turned in [false, true] {
            let place = |b: &craft3d_topo::Body| {
                if turned {
                    b.transformed(&turn(k)).unwrap()
                } else {
                    b.clone()
                }
            };
            let rest = solve(&place(&dome.body), &place(&rods), Subtract, k).body;
            let roof = rest
                .faces
                .iter()
                .find(|f| matches!(f.surface, Surface::Nurbs(_)))
                .unwrap();
            assert_eq!(roof.loops.len(), 31, "at {k}");
            let volume = mass(&rest).volume / k3;
            let expected = dome.volume / k3 - taken;
            assert!(
                (volume - expected).abs() <= 1e-7 * expected,
                "at {k}, turned {turned}: {volume} is not {expected}"
            );
        }
    }
}
