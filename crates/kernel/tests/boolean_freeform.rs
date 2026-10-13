//! Booleans with a freeform (Bézier or NURBS) face: the domed block of
//! `craft3d_testkit` (a bicubic roof over a block) against blocks, cylinders
//! and balls. The roof's control net is a regular grid, so the roof is the
//! height field `z = h(x / dx, y / dy)` and every volume here has an
//! integral over a rectangle or a disc that a fine quadrature gives to
//! better than the tolerance the results are held to.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod boolean_common;

use boolean_common::*;
use craft3d_geom::Surface;
use craft3d_kernel::boolean::BooleanOp::{Intersect, Subtract, Union};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Vec3};
use craft3d_testkit::trimmed::domed_block;
use craft3d_topo::Body;
use std::f64::consts::PI;

const DX: f64 = 4.0;
const DY: f64 = 3.0;
const HEIGHT: f64 = 2.0;

/// The dome at scale `k`, and its roof height above `(x, y)` (at scale 1).
fn dome(k: f64, nurbs: bool) -> (Body, f64) {
    let reference = domed_block(&Frame::WORLD, DX * k, DY * k, HEIGHT * k, nurbs).unwrap();
    (reference.body, reference.volume / (k * k * k))
}

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

/// The integral of `f(x, y)` over the rectangle, by the midpoint rule on
/// a fine grid (the integrands are smooth: the error is far below 1e-7).
fn over_rectangle(lo: (f64, f64), hi: (f64, f64), f: impl Fn(f64, f64) -> f64) -> f64 {
    const N: usize = 800;
    let (w, h) = ((hi.0 - lo.0) / N as f64, (hi.1 - lo.1) / N as f64);
    let mut sum = 0.0;
    for i in 0..N {
        for j in 0..N {
            sum += f(lo.0 + (i as f64 + 0.5) * w, lo.1 + (j as f64 + 0.5) * h);
        }
    }
    sum * w * h
}

/// The integral over a disc, in polar coordinates.
fn over_disc(center: (f64, f64), radius: f64, f: impl Fn(f64, f64) -> f64) -> f64 {
    const N: usize = 600;
    let (dr, da) = (radius / N as f64, 2.0 * PI / N as f64);
    let mut sum = 0.0;
    for i in 0..N {
        let r = (i as f64 + 0.5) * dr;
        for j in 0..N {
            let a = (j as f64 + 0.5) * da;
            sum += f(center.0 + r * a.cos(), center.1 + r * a.sin()) * r;
        }
    }
    sum * dr * da
}

/// `assert_close` of the common helpers holds results to 1e-9; a quadrature
/// here is good to about 1e-7, a quadrature of something with a square
/// root at its rim to 1e-4.
fn near(actual: f64, expected: f64, rel: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= rel * expected.abs().max(actual.abs()),
        "{what}: {actual} is not {expected}"
    );
}

fn has_patch(body: &Body) -> bool {
    body.faces
        .iter()
        .any(|f| matches!(f.surface, Surface::Bezier(_) | Surface::Nurbs(_)))
}

#[test]
fn a_block_through_the_walls_and_floor_leaves_the_roof_alone() {
    for k in SCALES {
        for nurbs in [false, true] {
            let (dome, volume) = dome(k, nurbs);
            // Through two walls and the floor; well below the roof.
            let block = boxed(k, [1.0, -1.0, -1.0], [3.0, 4.0, 0.5]);
            let overlap = 2.0 * 3.0 * 0.5;
            let united = solve(&dome, &block, Union, k);
            assert_close(
                mass(&united.body).volume / (k * k * k),
                volume + 2.0 * 5.0 * 1.5 - overlap,
                "union volume",
            );
            assert!(has_patch(&united.body));
            let common = solve(&dome, &block, Intersect, k);
            assert_close(
                mass(&common.body).volume / (k * k * k),
                overlap,
                "common volume",
            );
            assert!(!has_patch(&common.body));
            let rest = solve(&dome, &block, Subtract, k);
            assert_close(
                mass(&rest.body).volume / (k * k * k),
                volume - overlap,
                "difference volume",
            );
            assert!(has_patch(&rest.body));
        }
    }
}

#[test]
fn a_block_cuts_through_the_roof() {
    for k in SCALES {
        for nurbs in [false, true] {
            let (dome, volume) = dome(k, nurbs);
            // Its bottom at z = 1 is below the roof everywhere (the roof is
            // 1.4 or higher), its top above it.
            let block = boxed(k, [0.5, 0.5, 1.0], [3.5, 2.5, 3.0]);
            let overlap = over_rectangle((0.5, 0.5), (3.5, 2.5), |x, y| roof_height(x, y) - 1.0);
            let common = solve(&dome, &block, Intersect, k);
            near(
                mass(&common.body).volume / (k * k * k),
                overlap,
                1e-6,
                "common",
            );
            assert!(has_patch(&common.body));
            let rest = solve(&dome, &block, Subtract, k);
            near(
                mass(&rest.body).volume / (k * k * k),
                volume - overlap,
                1e-6,
                "dome less block",
            );
            let united = solve(&dome, &block, Union, k);
            near(
                mass(&united.body).volume / (k * k * k),
                volume + 3.0 * 2.0 * 2.0 - overlap,
                1e-6,
                "union",
            );
            let rest_b = solve(&block, &dome, Subtract, k);
            near(
                mass(&rest_b.body).volume / (k * k * k),
                12.0 - overlap,
                1e-6,
                "block less dome",
            );
            // The kernel's own results agree exactly.
            assert_close(
                mass(&rest.body).volume + mass(&common.body).volume,
                mass(&dome).volume,
                "common and difference make the dome",
            );
        }
    }
}

#[test]
fn a_round_hole_through_the_roof() {
    for k in SCALES {
        let (dome, volume) = dome(k, false);
        let hole = primitives::cylinder(
            &Frame::new(Vec3::new(2.0 * k, 1.5 * k, -k), Vec3::Z, Vec3::X).unwrap(),
            0.5 * k,
            5.0 * k,
        )
        .unwrap();
        let removed = over_disc((2.0, 1.5), 0.5, roof_height);
        let rest = solve(&dome, &hole, Subtract, k);
        near(
            mass(&rest.body).volume / (k * k * k),
            volume - removed,
            1e-6,
            "dome with a hole",
        );
        assert!(has_patch(&rest.body));
        // The roof is one face with a hole in it: two loops.
        let roof = rest
            .body
            .faces
            .iter()
            .find(|f| matches!(f.surface, Surface::Bezier(_)))
            .unwrap();
        assert_eq!(roof.loops.len(), 2);
        let common = solve(&dome, &hole, Intersect, k);
        near(
            mass(&common.body).volume / (k * k * k),
            removed,
            1e-6,
            "the plug",
        );
        assert_close(
            mass(&rest.body).volume + mass(&common.body).volume,
            mass(&dome).volume,
            "plug and rest make the dome",
        );
    }
}

#[test]
fn a_ball_sunk_into_the_roof() {
    let k = 1.0;
    let (dome, volume) = dome(k, false);
    let center = (2.5, 1.0);
    let z0 = roof_height(center.0, center.1);
    let ball = primitives::sphere(
        &Frame::new(Vec3::new(center.0, center.1, z0), Vec3::Z, Vec3::X).unwrap(),
        0.6,
    )
    .unwrap();
    // The part of the ball below the roof: over the disc under the ball,
    // the height of the ball's underside up to the roof.
    let sunk = over_disc(center, 0.6, |x, y| {
        let r2 = (x - center.0).powi(2) + (y - center.1).powi(2);
        let half = (0.36 - r2).max(0.0).sqrt();
        (roof_height(x, y) - (z0 - half)).clamp(0.0, 2.0 * half)
    });
    let common = solve(&dome, &ball, Intersect, k);
    near(mass(&common.body).volume, sunk, 2e-4, "ball in dome");
    let rest = solve(&dome, &ball, Subtract, k);
    let united = solve(&dome, &ball, Union, k);
    let common = mass(&common.body).volume;
    assert_close(mass(&rest.body).volume, volume - common, "dome less ball");
    assert_close(
        mass(&united.body).volume,
        volume + 4.0 * PI / 3.0 * 0.216 - common,
        "dome and ball",
    );
}

/// A block set with its underside in the tangent plane of the roof at a
/// point where the roof is a saddle: the plane touches the roof there and
/// cuts into it on two opposite sides, so the curve in which they meet is
/// two branches that cross at the touch. Until 2026-10-13 a meeting curve
/// through a point where the surfaces are tangent was refused wherever a
/// freeform face was involved, because this kernel and the reference
/// disagreed on such cases; a direct integration of the common volume
/// (below, and in the corpus test) showed the reference to be the one
/// that was off.
///
/// What the block and the dome share, integrated without either: along
/// every vertical line the dome is the stretch from the floor to the
/// roof, the block the stretch between its six planes.
#[test]
fn a_block_on_the_tangent_plane_of_a_saddle_of_the_roof() {
    // The most saddle-like place of the roof's middle, by differences.
    let step = 1e-4;
    let second = |x: f64, y: f64| {
        let h = roof_height;
        (
            (h(x + step, y) - 2.0 * h(x, y) + h(x - step, y)) / (step * step),
            (h(x + step, y + step) - h(x + step, y - step) - h(x - step, y + step)
                + h(x - step, y - step))
                / (4.0 * step * step),
            (h(x, y + step) - 2.0 * h(x, y) + h(x, y - step)) / (step * step),
        )
    };
    let mut at = (0.0, 0.0, 0.0);
    for i in 3..=9 {
        for j in 3..=9 {
            let (x, y) = (DX * f64::from(i) / 12.0, DY * f64::from(j) / 12.0);
            let (xx, xy, yy) = second(x, y);
            let gauss = xx * yy - xy * xy;
            if gauss < at.2 {
                at = (x, y, gauss);
            }
        }
    }
    let (x0, y0, gauss) = at;
    assert!(gauss < -0.01, "the roof has no saddle there: {gauss}");
    let z0 = roof_height(x0, y0);
    let slope = (
        (roof_height(x0 + step, y0) - roof_height(x0 - step, y0)) / (2.0 * step),
        (roof_height(x0, y0 + step) - roof_height(x0, y0 - step)) / (2.0 * step),
    );
    let normal = Vec3::new(-slope.0, -slope.1, 1.0).normalized().unwrap();
    let along = Vec3::new(1.0, 0.0, slope.0).normalized().unwrap();
    // The touch in the middle of the block's underside.
    let (side, tall) = (1.3, 0.8);
    let frame_at = |k: f64| {
        let unit = Frame::new(Vec3::new(x0, y0, z0), normal, along).unwrap();
        let corner = unit.point(-0.5 * side, -0.5 * side, 0.0);
        Frame::new(corner * k, normal, along).unwrap()
    };
    let frame = frame_at(1.0);
    let axes = (frame.x(), frame.y(), frame.z());
    const N: usize = 1600;
    let shared = |n: usize| {
        let mut sum = 0.0;
        for i in 0..n {
            for j in 0..n {
                let (x, y) = (
                    DX * (i as f64 + 0.5) / n as f64,
                    DY * (j as f64 + 0.5) / n as f64,
                );
                let p = frame.local(Vec3::new(x, y, 0.0));
                let (mut low, mut high) = (0.0_f64, roof_height(x, y));
                for (at, along, most) in [
                    (p.0, axes.0.z, side),
                    (p.1, axes.1.z, side),
                    (p.2, axes.2.z, tall),
                ] {
                    // 0 ≤ at + t along ≤ most.
                    let (a, b) = ((0.0 - at) / along, (most - at) / along);
                    low = low.max(a.min(b));
                    high = high.min(a.max(b));
                }
                sum += (high - low).max(0.0);
            }
        }
        sum * DX * DY / (n * n) as f64
    };
    let (coarse, expected) = (shared(N / 2), shared(N));
    assert!(expected > 1e-3, "nothing is shared: {expected}");
    assert!(
        (expected - coarse).abs() < 2e-4 * expected,
        "the integral has not settled: {coarse} then {expected}"
    );
    for k in SCALES {
        for nurbs in [false, true] {
            let (dome, volume) = dome(k, nurbs);
            let block = primitives::block(&frame_at(k), side * k, side * k, tall * k).unwrap();
            let k3 = k * k * k;
            let common = solve(&dome, &block, Intersect, k);
            let common = mass(&common.body).volume / k3;
            near(common, expected, 1e-3, "block and dome");
            let rest = solve(&dome, &block, Subtract, k);
            assert_close(
                mass(&rest.body).volume / k3,
                volume - common,
                "dome less block",
            );
            let united = solve(&dome, &block, Union, k);
            assert_close(
                mass(&united.body).volume / k3,
                volume + side * side * tall - common,
                "dome and block",
            );
            assert!(has_patch(&united.body) && has_patch(&rest.body));
        }
    }
}
