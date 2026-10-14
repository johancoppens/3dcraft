//! Booleans with a freeform face whose patch closes into one point on two
//! adjacent sides: a pole at a corner of its parameters, where both
//! parameters stall.
//!
//! The oracle is an analytic twin. A quarter of a ball (x and y positive)
//! has a lune for its curved face, from the north pole to the south pole
//! between two meridians. Its twin has that lune on a rational patch made
//! by inverse stereographic projection from the north pole of the quadrant
//! `X = (1 − s) / (s t)`, `Y = (1 − t) / (s t)`: the borders `s = 0` and
//! `t = 0` go to infinity, which is the north pole, the borders `s = 1`
//! and `t = 1` are the meridians, and the corner `(1, 1)` is the south
//! pole, an ordinary corner. The patch is the lune exactly; a boolean of
//! the one must give what the same boolean of the other gives.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod boolean_common;

use boolean_common::*;
use craft3d_geom::pole::{Border, poles};
use craft3d_geom::{NurbsSurface, Surface};
use craft3d_kernel::boolean::BooleanOp::{Intersect, Subtract, Union};
use craft3d_kernel::boolean::boolean;
use craft3d_math::Vec3;
use craft3d_topo::{Body, check};
use std::f64::consts::PI;

/// A polynomial of bidegree `(n, n)` in Bernstein form, as a grid of
/// coefficients `[i][j]`, raised by one degree both ways.
fn elevated(c: &[Vec<[f64; 4]>]) -> Vec<Vec<[f64; 4]>> {
    let n = c.len() - 1;
    let m = n + 1;
    let mix = |a: [f64; 4], b: [f64; 4], wa: f64, wb: f64| -> [f64; 4] {
        [
            wa * a[0] + wb * b[0],
            wa * a[1] + wb * b[1],
            wa * a[2] + wb * b[2],
            wa * a[3] + wb * b[3],
        ]
    };
    // Along i.
    let mut rows = vec![vec![[0.0; 4]; n + 1]; m + 1];
    for (i, row) in rows.iter_mut().enumerate() {
        for (j, out) in row.iter_mut().enumerate() {
            let a = i as f64 / m as f64;
            let low = if i > 0 { c[i - 1][j] } else { [0.0; 4] };
            let high = if i <= n { c[i][j] } else { [0.0; 4] };
            *out = mix(low, high, a, 1.0 - a);
        }
    }
    // Along j.
    let mut out = vec![vec![[0.0; 4]; m + 1]; m + 1];
    for i in 0..=m {
        for j in 0..=m {
            let a = j as f64 / m as f64;
            let low = if j > 0 { rows[i][j - 1] } else { [0.0; 4] };
            let high = if j <= n { rows[i][j] } else { [0.0; 4] };
            out[i][j] = mix(low, high, a, 1.0 - a);
        }
    }
    out
}

/// The lune of the sphere of radius `r` about the origin with x and y
/// positive, as a rational patch of bidegree (`degree`, `degree`) with
/// its north pole on the borders `u = 0` and `v = 0`.
fn lune_patch(r: f64, degree: usize) -> Surface {
    // Homogeneous (x w, y w, z w, w) of the projection, a polynomial of
    // bidegree (2, 2): with X = 1 − s, Y = 1 − t and W = s t,
    // (2 X W, 2 Y W, X² + Y² − W², X² + Y² + W²).
    let f = |s: f64, t: f64| -> [f64; 4] {
        let (x, y, w) = (1.0 - s, 1.0 - t, s * t);
        let q = x * x + y * y;
        [r * 2.0 * x * w, r * 2.0 * y * w, r * (q - w * w), q + w * w]
    };
    // Bernstein coefficients of degree 2 from the values at 0, ½, 1:
    // c₀ = f(0), c₂ = f(1), c₁ = 2 f(½) − (f(0) + f(1)) / 2, both ways.
    let at = [0.0, 0.5, 1.0];
    let values: Vec<Vec<[f64; 4]>> = at
        .iter()
        .map(|&s| at.iter().map(|&t| f(s, t)).collect())
        .collect();
    let solve = |v: [[f64; 4]; 3]| -> [[f64; 4]; 3] {
        let mut c = [[0.0; 4]; 3];
        for k in 0..4 {
            c[0][k] = v[0][k];
            c[2][k] = v[2][k];
            c[1][k] = 2.0 * v[1][k] - 0.5 * (v[0][k] + v[2][k]);
        }
        c
    };
    let mut half = vec![vec![[0.0; 4]; 3]; 3];
    for i in 0..3 {
        let c = solve([values[i][0], values[i][1], values[i][2]]);
        half[i] = c.to_vec();
    }
    let mut grid = vec![vec![[0.0; 4]; 3]; 3];
    for j in 0..3 {
        let c = solve([half[0][j], half[1][j], half[2][j]]);
        for i in 0..3 {
            grid[i][j] = c[i];
        }
    }
    while grid.len() - 1 < degree {
        grid = elevated(&grid);
    }
    let n = grid.len() - 1;
    let mut points = Vec::new();
    let mut weights = Vec::new();
    for row in &grid {
        for h in row {
            assert!(h[3] > 0.0, "a weight that is not positive: {h:?}");
            points.push(Vec3::new(h[0] / h[3], h[1] / h[3], h[2] / h[3]));
            weights.push(h[3]);
        }
    }
    let knots: Vec<f64> = std::iter::repeat_n(0.0, n + 1)
        .chain(std::iter::repeat_n(1.0, n + 1))
        .collect();
    Surface::Nurbs(
        NurbsSurface::new(n, n, knots.clone(), knots, n + 1, n + 1, points, weights).unwrap(),
    )
}

/// The quarter ball of radius 2 with x and y positive, analytic.
fn quarter(k: f64) -> Body {
    let ball = ball(k, [0.0, 0.0, 0.0], 2.0);
    let block = boxed(k, [0.0, 0.0, -3.0], [3.0, 3.0, 3.0]);
    solve(&ball, &block, Intersect, k).body
}

/// The same quarter ball with its lune on the patch with a corner pole.
fn twin(k: f64) -> Body {
    let plain = quarter(k);
    let mut twin = plain.clone();
    let patch = lune_patch(2.0 * k, 3);
    let mut replaced = 0;
    for (index, face) in plain.faces.iter().enumerate() {
        if !matches!(face.surface, Surface::Sphere { .. }) {
            continue;
        }
        let at = patch.eval(0.4, 0.7);
        let (su, sv) = face.surface.project(at).unwrap();
        assert!(face.surface.eval(su, sv).distance(at) <= 1e-12 * k);
        let same = patch
            .normal(0.4, 0.7)
            .unwrap()
            .dot(face.surface.normal(su, sv).unwrap())
            > 0.0;
        twin.faces[index].surface = patch.clone();
        twin.faces[index].reversed = face.reversed == same;
        replaced += 1;
    }
    assert_eq!(replaced, 1);
    twin
}

#[test]
fn the_lune_patch_has_a_pole_on_two_adjacent_borders() {
    for k in SCALES {
        let patch = lune_patch(2.0 * k, 3);
        let found: Vec<Border> = poles(&patch).iter().map(|p| p.border).collect();
        assert_eq!(found, [Border::UStart, Border::VStart]);
        for (s, t) in [(0.3, 0.6), (0.9, 0.1), (0.5, 0.5), (1.0, 1.0)] {
            let p = patch.eval(s, t);
            assert!((p.length() - 2.0 * k).abs() <= 1e-12 * k, "{p:?}");
            assert!(p.x >= -1e-15 * k && p.y >= -1e-15 * k);
        }
        assert!(patch.eval(1.0, 1.0).distance(Vec3::new(0.0, 0.0, -2.0 * k)) <= 1e-12 * k);
        let twin = twin(k);
        assert_eq!(check(&twin, &tolerance(k)), []);
        let (want, got) = (mass(&quarter(k)), mass(&twin));
        let r = 2.0 * k;
        assert!((want.volume - PI * r.powi(3) / 3.0).abs() <= 1e-9 * want.volume);
        assert!((got.volume - want.volume).abs() <= 1e-9 * want.volume, "{got:?}");
        assert!((got.area - want.area).abs() <= 1e-9 * want.area, "{got:?}");
    }
}

#[test]
fn probe_booleans_with_the_corner_pole() {
    let k = 1.0;
    let twin = twin(k);
    let plain = quarter(k);
    let tool = ball(k, [0.5, 0.6, 1.8], 0.5);
    for op in [Union, Intersect, Subtract] {
        let want = solve(&plain, &tool, op, k);
        match boolean(&twin, &tool, op, &tolerance(k)) {
            Ok(got) => {
                assert_eq!(check(&got.body, &tolerance(k)), []);
                let (w, g) = (mass(&want.body), mass(&got.body));
                eprintln!("{op:?}: {} vs {} / {} vs {}", g.volume, w.volume, g.area, w.area);
            }
            Err(e) => eprintln!("{op:?}: {e}"),
        }
    }
}
