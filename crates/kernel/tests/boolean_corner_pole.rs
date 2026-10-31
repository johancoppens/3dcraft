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

use craft3d_math::Portable;
mod boolean_common;

use boolean_common::*;
use craft3d_geom::pole::{Border, poles};
use craft3d_geom::{NurbsSurface, Surface};
use craft3d_kernel::boolean::BooleanOp::{Intersect, Subtract, Union};
use craft3d_kernel::boolean::boolean;
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Transform, Vec3};
use craft3d_tess::{SelfCheckParams, TessParams, body_self_intersections, tessellate_body};
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

/// Where the pole is in the parameters: at the end of u (else its
/// start), at the end of v, and the parameters exchanged.
type Corner = (bool, bool, bool);

/// The lune of the sphere of radius `r` about the origin with x and y
/// positive, as a rational patch of bidegree (`degree`, `degree`) with
/// its north pole on the borders `u = 0` and `v = 0`.
fn lune_patch(r: f64, degree: usize) -> Surface {
    lune_patch_at(r, degree, (false, false, false))
}

/// The same with its pole at another corner.
fn lune_patch_at(r: f64, degree: usize, (u_end, v_end, turn): Corner) -> Surface {
    // Homogeneous (x w, y w, z w, w) of the projection, a polynomial of
    // bidegree (2, 2): with X = 1 − s, Y = 1 − t and W = s t,
    // (2 X W, 2 Y W, X² + Y² − W², X² + Y² + W²).
    let f = |a: f64, b: f64| -> [f64; 4] {
        let (a, b) = if turn { (b, a) } else { (a, b) };
        let s = if u_end { 1.0 - a } else { a };
        let t = if v_end { 1.0 - b } else { b };
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
    twin_at(k, (false, false, false))
}

/// The same with the pole at another corner of the patch.
fn twin_at(k: f64, corner: Corner) -> Body {
    let plain = quarter(k);
    let mut twin = plain.clone();
    let patch = lune_patch_at(2.0 * k, 3, corner);
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
        assert!((want.volume - PI * r.ppowi(3) / 3.0).abs() <= 1e-9 * want.volume);
        assert!(
            (got.volume - want.volume).abs() <= 1e-9 * want.volume,
            "{got:?}"
        );
        assert!((got.area - want.area).abs() <= 1e-9 * want.area, "{got:?}");
    }
}

/// A tilt and a shift that leave nothing along an axis.
fn tilt(k: f64) -> Transform {
    Transform::rotation(Vec3::new(0.3, -0.5, 0.8), 0.7)
        .unwrap()
        .then(&Transform::translation(Vec3::new(1.5, -2.0, 0.7) * k))
}

/// A block on a frame through `origin` (in units of the scale): `size`
/// along the frame's axes.
fn block_on(k: f64, origin: Vec3, z: Vec3, x: Vec3, size: [f64; 3]) -> Body {
    let frame = Frame::new(origin * k, z, x).unwrap();
    primitives::block(&frame, size[0] * k, size[1] * k, size[2] * k).unwrap()
}

const NORTH: Vec3 = Vec3::new(0.0, 0.0, 2.0);

/// Tools that cut the lune: away from its poles, around the north pole
/// (the corner of the patch's parameters) and through it, and around the
/// south pole (an ordinary corner of the patch, where the lune's two
/// edges meet).
fn tools(k: f64) -> Vec<(&'static str, Body)> {
    let down = Vec3::new(0.6, 0.5, -0.62).normalized().unwrap();
    let across = Vec3::new(1.0, -1.0, 0.0);
    let leaning = Frame::new(Vec3::ZERO, down, across).unwrap();
    let (lx, ly) = (leaning.x(), leaning.y());
    let edge = Vec3::new(0.2, 0.2, -1.0).normalized().unwrap();
    vec![
        ("away: block", boxed(k, [0.8, 0.7, -0.5], [2.5, 2.5, 0.5])),
        ("away: rod", rod(k, [1.0, 0.9, -3.0], 0.3, 6.0)),
        ("away: ball", ball(k, [1.1, 1.0, 0.9], 0.6)),
        ("away: spike", spike(k, [1.0, 1.1, 0.2], 0.5, 1.5)),
        ("away: ring", ring(k, [1.0, 1.0, 1.0], 0.6, 0.2)),
        (
            "around: block",
            boxed(k, [-0.5, -0.4, 1.5], [0.7, 0.6, 2.6]),
        ),
        ("around: rod", log(k, [-1.0, 0.2, 1.8], 0.5, 3.0)),
        ("around: ball", ball(k, [0.2, 0.3, 1.9], 0.6)),
        ("around: spike", spike(k, [0.1, 0.2, 1.2], 0.7, 1.5)),
        ("around: ring", ring(k, [0.6, 0.1, 1.9], 0.7, 0.3)),
        (
            "through: block face",
            block_on(
                k,
                NORTH - lx * 0.5 - ly * 0.4,
                down,
                across,
                [1.4, 1.2, 0.9],
            ),
        ),
        (
            "through: block edge",
            block_on(
                k,
                NORTH - edge * 0.5,
                Vec3::new(1.0, 0.0, 0.2),
                edge,
                [1.6, 1.1, 1.2],
            ),
        ),
        ("through: ball", ball(k, [0.3, 0.4, 1.4], 0.61_f64.sqrt())),
        ("through: spike", spike(k, [0.3, 0.0, 1.5], 0.45, 1.5)),
        ("south: ball", ball(k, [0.2, 0.3, -1.9], 0.6)),
        (
            "south: block",
            boxed(k, [-0.5, -0.4, -2.6], [0.7, 0.6, -1.5]),
        ),
    ]
}

#[track_caller]
fn near(actual: f64, expected: f64, rel: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= rel * expected.abs().max(actual.abs()),
        "{what}: {actual} is not {expected} (off by {:e})",
        (actual - expected).abs() / expected.abs().max(actual.abs())
    );
}

/// Whether the mesh of a body is closed and encloses its volume, or why
/// not.
fn try_mesh(body: &Body, k: f64) -> Result<(), String> {
    let exact = mass(body);
    let chord = 5e-3 * k;
    let params = TessParams {
        chordal_tolerance: chord,
        ..TessParams::default()
    };
    let mesh = tessellate_body(body, &params).map_err(|e| e.to_string())?;
    let welded = mesh.weld().map_err(|e| format!("{e:?}"))?;
    if !welded.is_closed() {
        return Err("the mesh is not watertight".to_string());
    }
    if (welded.volume() - exact.volume).abs() > exact.area * chord {
        return Err(format!(
            "mesh volume {} against {}",
            welded.volume(),
            exact.volume
        ));
    }
    Ok(())
}

fn freeform_faces(body: &Body) -> usize {
    body.faces
        .iter()
        .filter(|f| matches!(f.surface, Surface::Nurbs(_) | Surface::Bezier(_)))
        .count()
}

#[test]
fn the_twin_meshes_and_has_no_self_intersection() {
    for k in SCALES {
        let twin = twin(k);
        assert_eq!(try_mesh(&twin, k), Ok(()), "the twin at {k}");
        let found =
            body_self_intersections(&twin, &SelfCheckParams::new(tolerance(k).linear)).unwrap();
        assert!(found.is_clean(), "{found}");
    }
}

/// The three booleans of the twin and a tool, held to the same booleans
/// of the analytic quarter ball, and to the identities of sets.
fn agree(tool: &Body, k: f64, tilted: bool, what: &str) -> (f64, f64) {
    agree_at(tool, k, tilted, (false, false, false), what)
}

fn agree_at(tool: &Body, k: f64, tilted: bool, corner: Corner, what: &str) -> (f64, f64) {
    let (mut twin, mut plain, mut tool) = (twin_at(k, corner), quarter(k), tool.clone());
    if tilted {
        twin = twin.transformed(&tilt(k)).unwrap();
        plain = plain.transformed(&tilt(k)).unwrap();
        tool = tool.transformed(&tilt(k)).unwrap();
    }
    let mut worst = (0.0_f64, 0.0_f64);
    let mut got = Vec::new();
    for op in [Union, Intersect, Subtract] {
        let what = format!("{what}, {op:?}");
        let expected = solve(&plain, &tool, op, k);
        let result = match boolean(&twin, &tool, op, &tolerance(k)) {
            Ok(result) => result,
            Err(e) => panic!("{what}: {e}"),
        };
        assert_eq!(check(&result.body, &tolerance(k)), [], "{what}");
        let (want, have) = (mass(&expected.body), mass(&result.body));
        let off = (
            (have.volume - want.volume).abs() / want.volume,
            (have.area - want.area).abs() / want.area,
        );
        worst = (worst.0.max(off.0), worst.1.max(off.1));
        near(have.volume, want.volume, 1e-6, &format!("{what}: volume"));
        near(have.area, want.area, 1e-6, &format!("{what}: area"));
        assert!(
            have.centroid.unwrap().distance(want.centroid.unwrap()) <= 1e-6 * k,
            "{what}: centroid"
        );
        assert_eq!(
            result.body.shells.len(),
            expected.body.shells.len(),
            "{what}: shells"
        );
        if op == Intersect {
            assert!(freeform_faces(&result.body) > 0, "{what}: no freeform face");
        }
        // (The tessellator declines some results whatever surface the
        // lune is on: a torus face with two loops around its tube. Where
        // it meshes the analytic result it must mesh this one.)
        let meshed = try_mesh(&expected.body, k).is_ok();
        if meshed {
            assert_eq!(try_mesh(&result.body, k), Ok(()), "{what}");
        }
        if meshed && k == 1.0 && !tilted {
            let found =
                body_self_intersections(&result.body, &SelfCheckParams::new(tolerance(k).linear))
                    .unwrap();
            assert!(found.is_clean(), "{what}: {found}");
        }
        got.push(have);
    }
    // The identities of sets.
    let (a, b) = (mass(&twin), mass(&tool));
    let (u, i, d) = (&got[0], &got[1], &got[2]);
    near(
        i.volume + d.volume,
        a.volume,
        1e-7,
        &format!("{what}: parts"),
    );
    near(
        u.volume + i.volume,
        a.volume + b.volume,
        1e-7,
        &format!("{what}: union"),
    );
    worst
}

fn lune_against(kind: &str) {
    let mut worst = (0.0_f64, 0.0_f64);
    for k in SCALES {
        for (name, tool) in tools(k) {
            if !name.starts_with(kind) {
                continue;
            }
            for tilted in [false, true] {
                let what = format!("lune, {name} at {k}, tilted {tilted}");
                let off = agree(&tool, k, tilted, &what);
                worst = (worst.0.max(off.0), worst.1.max(off.1));
            }
        }
    }
    println!(
        "worst against the analytic twin: volume {:e}, area {:e}",
        worst.0, worst.1
    );
}

#[test]
fn a_corner_pole_is_cut_away_from_the_pole() {
    lune_against("away");
}

#[test]
fn a_corner_pole_is_cut_around_the_pole() {
    lune_against("around");
}

#[test]
fn a_corner_pole_is_cut_through_the_pole() {
    lune_against("through");
}

#[test]
fn a_corner_pole_is_cut_at_the_far_corner() {
    lune_against("south");
}

/// The pole at each of the other corners of the patch's parameters, and
/// with the parameters exchanged: the same solids.
#[test]
fn a_corner_pole_at_any_corner_is_cut_through_the_pole() {
    let k = 1.0;
    for corner in [
        (true, false, false),
        (false, true, true),
        (true, true, false),
        (true, false, true),
    ] {
        let found: Vec<Border> = poles(&lune_patch_at(2.0, 3, corner))
            .iter()
            .map(|p| p.border)
            .collect();
        assert_eq!(found.len(), 2, "{corner:?}");
        assert_eq!(check(&twin_at(k, corner), &tolerance(k)), []);
        for (name, tool) in tools(k) {
            if !name.starts_with("through") {
                continue;
            }
            agree_at(&tool, k, false, corner, &format!("{corner:?}, {name}"));
        }
    }
}
