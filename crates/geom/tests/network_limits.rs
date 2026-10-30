//! What a network surface (`network::gordon`) refuses rather than make
//! badly, and what it claims about what it makes: a network whose curves
//! are crossed at different parameters (each re-parametrised curve
//! bringing its own knots) is refused above the size budget before
//! anything that large is made, and the same construction three by three is
//! the plane it lies in; crossings spaced so unevenly that a curve's speed
//! would all but vanish at one are refused; the reported `mismatch` bounds
//! how far the surface passes from its curves (rational curves whose
//! weights differ beyond what one surface can take up are refused, also
//! where the old residual guard let them through at a fine tolerance); a
//! chain joined smoothly within what a tolerance's worth of position
//! allows runs on at one speed.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_geom::network::{
    Condition, MAX_NETWORK_POINTS, Network, NetworkError, Patch, arc, chain, gordon,
};
use craft3d_geom::refine::interpolate;
use craft3d_geom::{NurbsCurve, NurbsSurface, Surface};
use craft3d_math::{Frame, Vec3};
use std::f64::consts::FRAC_PI_2;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];
const FREE: [Condition<'static>; 4] = [Condition::Free; 4];

fn v(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3::new(x, y, z)
}

fn line(a: Vec3, b: Vec3) -> NurbsCurve {
    NurbsCurve::polynomial(1, vec![0.0, 0.0, 1.0, 1.0], vec![a, b]).unwrap()
}

fn spline(points: &[Vec3]) -> NurbsCurve {
    let n = points.len();
    let p = (n - 1).min(3);
    let mut knots = vec![0.0; p + 1];
    for k in 1..n - p {
        knots.push(k as f64 / (n - p) as f64);
    }
    knots.extend(vec![1.0; p + 1]);
    NurbsCurve::polynomial(p, knots, points.to_vec()).unwrap()
}

/// The cubic through the points at the parameters.
fn through(params: &[f64], points: &[Vec3]) -> NurbsCurve {
    let values: Vec<Vec<f64>> = points.iter().map(|p| vec![p.x, p.y, p.z]).collect();
    let (knots, ctrl) = interpolate(params, 3, &values).unwrap();
    NurbsCurve::polynomial(3, knots, ctrl.iter().map(|c| v(c[0], c[1], c[2])).collect()).unwrap()
}

fn network<'a>(u: &'a [NurbsCurve], vv: &'a [NurbsCurve], tol: f64) -> Network<'a> {
    Network {
        u,
        v: vv,
        edges: FREE,
        tolerance: tol,
        angular: 1e-9,
    }
}

fn refused(result: Result<Patch, NetworkError>) -> String {
    match result {
        Err(NetworkError::Refused(why)) => why,
        Err(other) => panic!("expected a refusal, got the error {other}"),
        Ok(p) => panic!(
            "expected a refusal, got a patch of {:?} control points",
            p.surface.counts()
        ),
    }
}

/// A grid of `count` × `count` straight lines across the square of side
/// `side` in z = 0, each a cubic of 30 control points spaced along it by
/// its own warp `t^γ`: every line is crossed at its own parameters.
fn warped_grid(count: usize, side: f64) -> (Vec<NurbsCurve>, Vec<NurbsCurve>) {
    let warp = |k: usize| 0.8 + 0.45 * (k as f64 / (count - 1) as f64);
    let along = |k: usize, at: f64, across: bool| {
        let gamma = warp(k);
        let points: Vec<Vec3> = (0..30)
            .map(|i| {
                let s = side * (f64::from(i) / 29.0).powf(gamma);
                if across { v(at, s, 0.0) } else { v(s, at, 0.0) }
            })
            .collect();
        spline(&points)
    };
    let place = |k: usize| side * k as f64 / (count - 1) as f64;
    let us = (0..count).map(|i| along(i, place(i), false)).collect();
    let vs = (0..count)
        .map(|j| along(count - 1 - j, place(j), true))
        .collect();
    (us, vs)
}

#[test]
fn a_network_too_large_for_the_budget_is_refused_before_it_is_made() {
    for scale in SCALES {
        let (us, vs) = warped_grid(32, scale);
        // Refused from the knots, before the nets (which would take tens
        // of gigabytes): the message names the size it would have had.
        let why = refused(gordon(&network(&us, &vs, 1e-6 * scale)));
        assert!(
            why.contains("control points") && why.contains(&MAX_NETWORK_POINTS.to_string()),
            "{why}"
        );
        let asked: usize = why
            .split(" control points")
            .next()
            .and_then(|head| head.rsplit("need ").next())
            .map(|dims| {
                dims.split(" × ")
                    .map(|n| n.trim().parse::<usize>().unwrap())
                    .product()
            })
            .unwrap();
        assert!(asked > 16 * MAX_NETWORK_POINTS, "{why}");
    }
}

#[test]
fn the_same_network_three_by_three_is_its_plane() {
    for scale in SCALES {
        let (us, vs) = warped_grid(3, scale);
        let patch = gordon(&network(&us, &vs, 1e-6 * scale)).unwrap();
        let (nu, nv) = patch.surface.counts();
        assert!(nu * nv <= MAX_NETWORK_POINTS);
        for i in 0..=16 {
            for j in 0..=16 {
                let p = patch.surface.eval(f64::from(i) / 16.0, f64::from(j) / 16.0);
                assert!(p.z.abs() <= 1e-12 * scale, "{p:?}");
                assert!(p.x >= -1e-9 * scale && p.x <= scale * (1.0 + 1e-9));
            }
        }
        // Every curve on the surface, as `mismatch` says.
        assert!(patch.mismatch <= 1e-9 * scale, "{:e}", patch.mismatch);
    }
}

#[test]
fn crossings_spaced_too_unevenly_are_refused() {
    for scale in SCALES {
        let s = scale;
        // The middle v curve crosses the first u curve 1e-8 of the way
        // along it and the last one half way: the first u curve's speed
        // would fall to about 1e-8 of its mean at that crossing.
        let us = vec![
            line(v(0.0, 0.0, 0.0), v(s, 0.0, 0.0)),
            line(v(0.0, s, 0.0), v(s, s, 0.0)),
        ];
        let vs = vec![
            line(v(0.0, 0.0, 0.0), v(0.0, s, 0.0)),
            line(v(1e-8 * s, 0.0, 0.0), v(0.5 * s, s, 0.0)),
            line(v(s, 0.0, 0.0), v(s, s, 0.0)),
        ];
        let why = refused(gordon(&network(&us, &vs, 1e-9 * s)));
        assert!(why.contains("unevenly"), "{why}");
    }
}

/// The largest distance from curve `curve` to the surface, at 257
/// places along it (each projected onto the surface): what `mismatch`
/// must bound.
fn apart(surface: &NurbsSurface, curve: &NurbsCurve) -> f64 {
    let face = Surface::Nurbs(surface.clone());
    let (a, b) = curve.domain();
    (0..=256)
        .map(|k| {
            let p = curve.eval(a + (b - a) * f64::from(k) / 256.0);
            let (u, w) = face.project(p).unwrap();
            p.distance(face.eval(u, w))
        })
        .fold(0.0, f64::max)
}

#[test]
fn weights_that_differ_by_little_are_held_to_the_tolerance_not_to_a_fixed_residual() {
    let scale = 1e6;
    let r = scale;
    // Two arcs crossed by bulging curves at their ends and middles: the
    // second's middle weight a little off the first's, so that no
    // constants make the crossing weights agree, by a residual of about
    // 1e-10 in their logarithms (below the old fixed guard of 1e-9). The
    // v curves are moved by about that times their length, 1e-4 here: far
    // beyond a tolerance of 1e-6 (1e-12 of the size).
    let quarter = arc(&Frame::WORLD, r, 0.0, FRAC_PI_2).unwrap();
    let lift = v(0.0, 0.0, r);
    let mut weights = quarter.weights().to_vec();
    weights[1] *= 1.0 + 1e-9;
    let other = NurbsCurve::new(
        2,
        quarter.knots().to_vec(),
        quarter.control_points().iter().map(|p| *p + lift).collect(),
        weights,
    )
    .unwrap();
    let us = vec![quarter.clone(), other.clone()];
    let vs: Vec<NurbsCurve> = [0.0, 0.5, 1.0]
        .iter()
        .map(|f| {
            let (p0, p1) = (quarter.eval(*f), other.eval(*f));
            let out = v(p0.x, p0.y, 0.0).normalized().unwrap();
            spline(&[p0, (p0 + p1) * 0.5 + out * (0.3 * r), p1])
        })
        .collect();
    match gordon(&network(&us, &vs, 1e-6)) {
        Err(NetworkError::Refused(why)) => assert!(why.contains("weights"), "{why}"),
        Err(other) => panic!("{other}"),
        Ok(patch) => {
            // Accepted only with a mismatch that bounds the truth.
            for (j, curve) in vs.iter().enumerate() {
                let off = apart(&patch.surface, curve);
                assert!(
                    off <= patch.mismatch,
                    "v curve {j} is {off:e} off, the mismatch says {:e}",
                    patch.mismatch
                );
            }
        }
    }
}

#[test]
fn the_mismatch_bounds_how_far_the_surface_passes_from_its_curves() {
    for scale in SCALES {
        // A u curve nudged off its crossing with the middle v curve by a
        // tenth of the tolerance: the surface goes through the u curves
        // and within `mismatch` of the v curves, all along them.
        let tol = 1e-6 * scale;
        let h = |x: f64, y: f64| 0.3 * (1.3 * x).sin() + 0.2 * (0.8 * y).cos();
        let at = |x: f64, y: f64| v(x, y, h(x, y)) * scale;
        let mut us: Vec<NurbsCurve> = [0.0, 1.5, 3.0]
            .iter()
            .map(|y| {
                spline(
                    &(0..=4)
                        .map(|k| at(0.5 * f64::from(k), *y))
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        // The v curves through the u curves' points where they cross.
        let vs: Vec<NurbsCurve> = [0.0, 0.5, 1.0]
            .iter()
            .map(|x| {
                let pts: Vec<Vec3> = us.iter().map(|c| c.eval(*x)).collect();
                through(
                    &[0.0, 0.25, 0.5, 1.0],
                    &[
                        pts[0],
                        (pts[0] + pts[1]) * 0.5 + v(0.0, 0.0, 0.2) * scale,
                        pts[1],
                        pts[2],
                    ],
                )
            })
            .collect();
        let mut nudged = us[1].control_points().to_vec();
        nudged[2] += v(0.0, 0.0, 0.1 * tol);
        us[1] = NurbsCurve::polynomial(us[1].degree(), us[1].knots().to_vec(), nudged).unwrap();
        let patch = gordon(&network(&us, &vs, tol)).unwrap();
        for curve in &us {
            assert!(apart(&patch.surface, curve) <= 1e-9 * scale);
        }
        for curve in &vs {
            let off = apart(&patch.surface, curve);
            assert!(off <= patch.mismatch, "{off:e} > {:e}", patch.mismatch);
        }
        assert!(patch.mismatch <= tol, "{:e}", patch.mismatch);
    }
}

#[test]
fn a_chain_smooth_within_a_tolerance_of_position_runs_on_at_one_speed() {
    for scale in SCALES {
        let r = scale;
        let quarter = arc(&Frame::WORLD, r, 0.0, FRAC_PI_2).unwrap();
        // The tail turned 1e-7 rad off the arc's tangent at the joint:
        // more than the angular tolerance of 1e-12 given here, less than
        // the 5e-7 a tolerance of 1e-6 makes over the shorter piece.
        let turn: f64 = 1e-7;
        let tail = line(
            v(0.0, r, 0.0),
            v(-2.0 * r * turn.cos(), r + 2.0 * r * turn.sin(), 0.0),
        );
        let joined = chain(&[quarter, tail], 1e-6 * scale, 1e-12).unwrap();
        let mut joint = 0.0;
        for k in 0..=4000 {
            let t = f64::from(k) / 4000.0;
            if joined.eval(t).distance(v(0.0, r, 0.0)) < joined.eval(joint).distance(v(0.0, r, 0.0))
            {
                joint = t;
            }
        }
        let (_, before) = joined.derivatives(joint - 1e-9);
        let (_, after) = joined.derivatives(joint + 1e-9);
        assert!(
            (before.length() - after.length()).abs() <= 1e-5 * before.length(),
            "speeds {} and {}",
            before.length(),
            after.length()
        );
    }
}
