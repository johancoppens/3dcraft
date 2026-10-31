//! A Gordon surface whose middle curve has knots of its own between the
//! crossings (the class-A mouse's crown, 2026-10-31: a spline through
//! more points than there are curves crossing it).

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_geom::NurbsCurve;
use craft3d_geom::network::{Condition, Network, gordon};
use craft3d_geom::refine::{interpolate, interpolate_on};
use craft3d_math::Vec3;

fn v(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3::new(x, y, z)
}

/// The cubic through the points, at chord-length parameters.
fn through(points: &[Vec3]) -> NurbsCurve {
    let mut params = vec![0.0];
    for w in points.windows(2) {
        let last = *params.last().unwrap();
        params.push(last + w[0].distance(w[1]));
    }
    let total = *params.last().unwrap();
    let params: Vec<f64> = params.iter().map(|t| t / total).collect();
    let values: Vec<Vec<f64>> = points.iter().map(|p| vec![p.x, p.y, p.z]).collect();
    let (knots, ctrl) = interpolate(&params, 3, &values).unwrap();
    NurbsCurve::polynomial(3, knots, ctrl.iter().map(|c| v(c[0], c[1], c[2])).collect()).unwrap()
}

#[test]
fn a_curve_with_knots_between_its_crossings_is_made_one_with_its_family() {
    for scale in [1e-3, 1.0, 1e6] {
        let xs = [0.0, 3.0, 5.0, 7.0, 10.0];
        let side = |y: f64| -> Vec<Vec3> {
            xs.iter()
                .map(|&x| v(x, y, 1.0 + 0.3 * (x * 0.3).sin()) * scale)
                .collect()
        };
        let crown_z = |x: f64| 2.0 + 0.5 * (x * 0.31).sin() + 0.02 * x * x;
        // The crown passes through the crossings and through points
        // between them.
        let mut crown_pts: Vec<Vec3> = Vec::new();
        for (k, &x) in xs.iter().enumerate() {
            crown_pts.push(v(x, 0.0, crown_z(x)) * scale);
            if let Some(&next) = xs.get(k + 1)
                && k != 2
            {
                let mid = 0.5 * (x + next);
                crown_pts.push(v(mid, 0.0, crown_z(mid)) * scale);
            }
        }
        let us = vec![
            through(&side(-1.0)),
            through(&crown_pts),
            through(&side(1.0)),
        ];
        let vs: Vec<NurbsCurve> = xs
            .iter()
            .map(|&x| {
                let a = v(x, -1.0, 1.0 + 0.3 * (x * 0.3).sin()) * scale;
                let c = v(x, 0.0, crown_z(x)) * scale;
                let b = v(x, 1.0, 1.0 + 0.3 * (x * 0.3).sin()) * scale;
                through(&[
                    a,
                    (a + c) * 0.5 + v(0.0, 0.0, 0.2) * scale,
                    c,
                    (c + b) * 0.5 + v(0.0, 0.0, 0.2) * scale,
                    b,
                ])
            })
            .collect();
        let network = Network {
            u: &us,
            v: &vs,
            edges: [Condition::Free; 4],
            tolerance: 1e-6 * scale,
            angular: 1e-9,
        };
        let patch = gordon(&network).unwrap_or_else(|e| panic!("at {scale}: {e}"));
        assert!(
            patch.mismatch <= 1e-6 * scale,
            "at {scale}: {}",
            patch.mismatch
        );
    }
}

/// The cubic through `points` as a live sketch spline is fitted: on
/// clamped uniform knots, each point at its Greville abscissa, so the
/// points in the middle sit on knots.
fn greville(points: &[Vec3]) -> NurbsCurve {
    let n = points.len();
    let spans = n - 3;
    let mut knots = vec![0.0; 4];
    for k in 1..spans {
        knots.push(k as f64 / spans as f64);
    }
    knots.extend([1.0; 4]);
    let params: Vec<f64> = (0..n)
        .map(|i| (knots[i + 1] + knots[i + 2] + knots[i + 3]) / 3.0)
        .collect();
    let values: Vec<Vec<f64>> = points.iter().map(|p| vec![p.x, p.y, p.z]).collect();
    let ctrl = interpolate_on(&params, 3, &knots, &values).unwrap();
    NurbsCurve::polynomial(3, knots, ctrl.iter().map(|c| v(c[0], c[1], c[2])).collect()).unwrap()
}

/// The mouse's crown: nine points, three of them where the sections
/// cross it, at Greville abscissae that are knots. The crossing found
/// lies within rounding of the knot, on either side: cut there, the
/// curve kept a span of a rounding's width and joined again had one
/// block of control points more than its family ("the interpolated
/// values must all have the same length", 2026-10-31).
#[test]
fn a_crossing_within_rounding_of_a_knot_is_cut_at_the_knot() {
    for scale in [1e-3, 1.0, 1e6] {
        for hump in [0.36, 0.42, 0.5] {
            let (len, half, h) = (118.0, 31.0, 40.0);
            let stations = [(0.27, 0.48, 0.88), (hump, 0.52, 1.0), (0.66, 0.45, 0.86)];
            let crown: Vec<Vec3> = [
                (0.0, 0.0),
                (0.012, 0.30),
                (0.07, 0.62),
                (stations[0].0, stations[0].2),
                (stations[1].0, stations[1].2),
                (stations[2].0, stations[2].2),
                (0.80, 0.74),
                (0.93, 0.50),
                (1.0, 0.0),
            ]
            .iter()
            .map(|&(x, z)| v(x * len, 0.0, z * h) * scale)
            .collect();
            let seam = |y: f64| -> Vec<Vec3> {
                let mut p = vec![v(0.16 * len, y, 0.0)];
                p.extend(stations.iter().map(|&(x, s, _)| v(x * len, y, s * h)));
                p.push(v(0.86 * len, y, 0.0));
                p.iter().map(|q| *q * scale).collect()
            };
            let us = vec![
                greville(&seam(-half)),
                greville(&crown),
                greville(&seam(half)),
            ];
            let section = |x: f64, s: f64, c: f64| {
                greville(&[
                    v(x * len, -half, s * h) * scale,
                    v(x * len, -0.6 * half, (s + 0.8 * (c - s)) * h) * scale,
                    v(x * len, 0.0, c * h) * scale,
                    v(x * len, 0.6 * half, (s + 0.8 * (c - s)) * h) * scale,
                    v(x * len, half, s * h) * scale,
                ])
            };
            let end = |corner: f64, tip: f64| {
                greville(&[
                    v(corner * len, -half, 0.0) * scale,
                    v((corner + 0.7 * (tip - corner)) * len, -0.7 * half, 0.0) * scale,
                    v(tip * len, 0.0, 0.0) * scale,
                    v((corner + 0.7 * (tip - corner)) * len, 0.7 * half, 0.0) * scale,
                    v(corner * len, half, 0.0) * scale,
                ])
            };
            let mut vs = vec![end(0.16, 0.0)];
            vs.extend(stations.iter().map(|&(x, s, c)| section(x, s, c)));
            vs.push(end(0.86, 1.0));
            let network = Network {
                u: &us,
                v: &vs,
                edges: [Condition::Free; 4],
                tolerance: 1e-6 * scale,
                angular: 1e-9,
            };
            let patch = gordon(&network).unwrap_or_else(|e| panic!("at {scale}, hump {hump}: {e}"));
            assert!(
                patch.mismatch <= 1e-6 * scale,
                "at {scale}: {}",
                patch.mismatch
            );
        }
    }
}
