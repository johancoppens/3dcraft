//! The sheet through a grid of points against closed forms
//! (docs/design/surfacing.md › *A sheet through a grid of points*): a
//! paraboloid and a bicubic height field sampled on regular grids, the
//! sheet's height compared with the true one between the points as the
//! grid is refined (the error falls about 16 times per halving with chord
//! spacing, the order of chordal cubic interpolation, and about 4 times
//! with Greville spacing, whose parameters are uneven near the ends); the
//! volume under the sheet against the closed-form integral; the deviation
//! at the points within its bound at three scales, far from the origin.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::grid::{Grid, Spacing, interpolate};
use craft3d_math::{Portable, Vec3};

type Height = fn(f64, f64) -> f64;

/// `z = x² + y²` over `[-1, 1]²`, and its integral there.
fn paraboloid(x: f64, y: f64) -> f64 {
    x * x + y * y
}
const PARABOLOID_VOLUME: f64 = 8.0 / 3.0;

/// A bicubic, every term of degree up to 3 in each variable used, and its
/// integral over `[-1, 1]²`: the odd terms vanish, `∫∫ x²y² = 4/9`,
/// `∫∫ 1 = 4`, `∫∫ x² = ∫∫ y² = 4/3`.
fn bicubic(x: f64, y: f64) -> f64 {
    0.5 + 0.3 * x - 0.2 * y + 0.4 * x * x * y * y - 0.25 * x.ppowi(3) * y
        + 0.1 * x * y.ppowi(3)
        + 0.15 * x.ppowi(3) * y.ppowi(3)
        - 0.3 * x * x
        + 0.2 * y * y
}
const BICUBIC_VOLUME: f64 = 0.5 * 4.0 + 0.4 * 4.0 / 9.0 - 0.3 * 4.0 / 3.0 + 0.2 * 4.0 / 3.0;

/// A wave no cubic holds, `z = sin(2x) cos(3y) / 2`.
fn wave(x: f64, y: f64) -> f64 {
    0.5 * (2.0 * x).psin() * (3.0 * y).pcos()
}

/// The `n × n` grid of `f` over `[-1, 1]²`, scaled by `k` and moved by
/// `offset`.
fn sampled(f: Height, n: usize, k: f64, offset: Vec3) -> Vec<Vec3> {
    let mut out = Vec::with_capacity(n * n);
    for r in 0..n {
        for c in 0..n {
            let (x, y) = (
                -1.0 + 2.0 * c as f64 / (n - 1) as f64,
                -1.0 + 2.0 * r as f64 / (n - 1) as f64,
            );
            out.push(Vec3::new(x, y, f(x, y)) * k + offset);
        }
    }
    out
}

/// The largest vertical gap between the sheet and `f` (an upper bound on
/// its distance from the true surface) at 8 places per span each way.
fn vertical_gap(made: &Grid, f: Height) -> f64 {
    let n = 8 * (made.u.len() - 1);
    let mut gap = 0.0_f64;
    for i in 0..=n {
        for j in 0..=n {
            let p = made.surface.eval(i as f64 / n as f64, j as f64 / n as f64);
            gap = gap.max((p.z - f(p.x, p.y)).abs());
        }
    }
    gap
}

/// The volume between the sheet and `z = 0` over its footprint, the
/// surface integral `∫∫ Z (X_u Y_v − X_v Y_u) du dv` (the divergence
/// theorem on the solid under it: its walls are vertical, its floor at
/// zero height), by 6-point Gauss–Legendre quadrature per knot span each
/// way, exact for the polynomial integrand of a bicubic (degree 8 each
/// way).
fn volume_under(made: &Grid) -> f64 {
    let nodes = [
        (-0.932_469_514_203_152, 0.171_324_492_379_170),
        (-0.661_209_386_466_265, 0.360_761_573_048_139),
        (-0.238_619_186_083_197, 0.467_913_934_572_691),
        (0.238_619_186_083_197, 0.467_913_934_572_691),
        (0.661_209_386_466_265, 0.360_761_573_048_139),
        (0.932_469_514_203_152, 0.171_324_492_379_170),
    ];
    let spans = |knots: &[f64]| {
        let mut out: Vec<(f64, f64)> = Vec::new();
        for w in knots.windows(2) {
            if w[1] > w[0] {
                out.push((w[0], w[1]));
            }
        }
        out
    };
    let (su, sv) = (spans(made.surface.knots_u()), spans(made.surface.knots_v()));
    let mut total = 0.0;
    for &(u0, u1) in &su {
        for &(v0, v1) in &sv {
            for &(a, wa) in &nodes {
                for &(b, wb) in &nodes {
                    let u = 0.5 * (u0 + u1) + 0.5 * (u1 - u0) * a;
                    let v = 0.5 * (v0 + v1) + 0.5 * (v1 - v0) * b;
                    let (p, du, dv) = made.surface.derivatives(u, v);
                    let jac = du.x * dv.y - dv.x * du.y;
                    total += wa * wb * 0.25 * (u1 - u0) * (v1 - v0) * p.z * jac;
                }
            }
        }
    }
    total
}

#[test]
fn the_error_between_the_points_falls_with_the_step_as_the_theory_says() {
    let mut lines = Vec::new();
    for (name, f) in [
        ("paraboloid", paraboloid as Height),
        ("bicubic", bicubic as Height),
        ("wave", wave as Height),
    ] {
        for spacing in [Spacing::Greville, Spacing::Chord, Spacing::Even] {
            let mut gaps = Vec::new();
            for n in [9, 17, 33, 65, 129] {
                let pts = sampled(f, n, 1.0, Vec3::ZERO);
                let made = interpolate(&pts, n, n, spacing, 1e-9, 1e-9).unwrap();
                gaps.push((n, vertical_gap(&made, f)));
            }
            let rates: Vec<f64> = gaps.windows(2).map(|w| (w[0].1 / w[1].1).plog2()).collect();
            lines.push(format!("{name} {spacing:?}: {gaps:?}, orders {rates:?}"));
            // Even parameters over a regular grid keep x and y linear, so a
            // field of degree 3 or less each way comes back to rounding.
            if spacing == Spacing::Even && name != "wave" {
                assert!(
                    gaps.iter().all(|(_, g)| *g <= 1e-14),
                    "{}",
                    lines.join("\n")
                );
                continue;
            }
            // The order: chordal cubic interpolation
            // is fourth-order accurate (Floater, IMA J. Numer. Anal. 26,
            // 2006); Greville's parameters, uneven near the ends, give
            // second order there.
            // Over the last three halvings, as the order of averaged
            // chord parameters wanders from one halving to the next.
            let last = (gaps[gaps.len() - 4].1 / gaps[gaps.len() - 1].1).plog2() / 3.0;
            let want = match spacing {
                Spacing::Chord | Spacing::Even => 3.5,
                Spacing::Greville => 1.8,
            };
            assert!(last >= want, "{}", lines.join("\n"));
        }
    }
    println!("{}", lines.join("\n"));
}

#[test]
fn the_volume_under_the_sheet_converges_to_the_integral() {
    let mut lines = Vec::new();
    for (name, f, exact) in [
        ("paraboloid", paraboloid as Height, PARABOLOID_VOLUME),
        ("bicubic", bicubic as Height, BICUBIC_VOLUME),
    ] {
        for spacing in [Spacing::Greville, Spacing::Chord, Spacing::Even] {
            let mut errors = Vec::new();
            for n in [9, 17, 33, 65] {
                let made = interpolate(&sampled(f, n, 1.0, Vec3::ZERO), n, n, spacing, 1e-9, 1e-9)
                    .unwrap();
                errors.push((n, (volume_under(&made) - exact).abs() / exact));
            }
            lines.push(format!(
                "{name} {spacing:?}: relative volume errors {errors:?}"
            ));
            let (_, last) = errors[errors.len() - 1];
            assert!(last < 1e-5, "{}", lines.join("\n"));
            if spacing == Spacing::Even {
                // Exact: the sheet is the field itself.
                assert!(
                    errors.iter().all(|(_, e)| *e <= 1e-13),
                    "{}",
                    lines.join("\n")
                );
                continue;
            }
            assert!(
                errors.windows(2).all(|w| w[1].1 < w[0].1),
                "{}",
                lines.join("\n")
            );
        }
    }
    println!("{}", lines.join("\n"));
}

#[test]
fn every_point_lies_on_the_sheet_within_its_bound_at_every_scale() {
    for k in [1e-3, 1.0, 1e6] {
        for offset in [Vec3::ZERO, Vec3::new(4e3, -7e3, 2e3) * k] {
            // A turned frame: the paraboloid's grid rotated about a slanted
            // axis.
            let axis = Vec3::new(0.3, -0.5, 0.8).normalized().unwrap();
            let (s, c) = 0.7_f64.psin_cos();
            let turn = |p: Vec3| p * c + axis.cross(p) * s + axis * (axis.dot(p) * (1.0 - c));
            let pts: Vec<Vec3> = sampled(paraboloid, 33, k, Vec3::ZERO)
                .into_iter()
                .map(|p| turn(p) + offset)
                .collect();
            for spacing in [Spacing::Greville, Spacing::Chord] {
                let made = interpolate(&pts, 33, 33, spacing, 1e-9 * k, 1e-9).unwrap();
                assert!(
                    made.deviation <= made.bound,
                    "{k} {offset:?} {spacing:?}: {} > {}",
                    made.deviation,
                    made.bound
                );
                println!(
                    "scale {k:e}, offset {:e}, {spacing:?}: deviation {:e}, bound {:e}",
                    offset.length(),
                    made.deviation,
                    made.bound
                );
            }
        }
    }
}
