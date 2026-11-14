//! The spacing of a spline through points (J-034): where it takes its
//! points, and how far it strays from the smooth curve they were taken
//! from, against oracles that do not come from this code: exact
//! reproduction (chord spacing gives points on a line that line), the
//! order of the error as the points double (measured: about 16 times
//! smaller per doubling for chord spacing on any of these curves and
//! spacings, about 4 times for Greville spacing, whose parameters are
//! uneven near the ends), and deviations computed apart from this code,
//! with SciPy's `make_interp_spline` on the textbook parameters and knots
//! (*The NURBS Book*, eqs. 9.4, 9.5, 9.8), its distances by Newton foot
//! points on 20001 even samples of the spline's parameter (on 200001 for
//! the involute); those numbers are quoted where they are used.

#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

use super::*;
use craft3d_doc::SplineSpacing;
use craft3d_math::Portable;

const SPACINGS: [SplineSpacing; 2] = [SplineSpacing::Greville, SplineSpacing::Chord];

fn fit(spacing: SplineSpacing, degree: u32, through: &[P2]) -> Spline2 {
    interpolated(degree, through, false, spacing).unwrap()
}

/// The largest of `distance` over `samples` even steps of the spline's
/// parameter, ends included.
fn worst(spline: &Spline2, samples: u32, distance: impl Fn(P2) -> f64) -> f64 {
    (0..=samples)
        .map(|k| distance(point(spline, f64::from(k) / f64::from(samples)).unwrap()))
        .fold(0.0, f64::max)
}

/// The distance from `q` to the graph `y = f(x)`: Newton on the foot
/// point's condition `(x − qx) + (f(x) − qy) f'(x) = 0`, from `x = qx`.
fn off_graph(q: P2, f: impl Fn(f64) -> [f64; 3]) -> f64 {
    let mut x = q[0];
    for _ in 0..60 {
        let [y, d1, d2] = f(x);
        let g = (x - q[0]) + (y - q[1]) * d1;
        let slope = 1.0 + d1 * d1 + (y - q[1]) * d2;
        x -= g / slope;
    }
    (x - q[0]).phypot(f(x)[0] - q[1])
}

/// `n` places from 0 to 1: even, or in gaps alternating 1 and 3.
fn places(n: usize, uneven: bool) -> Vec<f64> {
    if !uneven {
        return (0..n).map(|i| i as f64 / (n - 1) as f64).collect();
    }
    let gaps: Vec<f64> = (0..n - 1)
        .map(|i| if i % 2 == 0 { 1.0 } else { 3.0 })
        .collect();
    let total: f64 = gaps.iter().sum();
    let mut along = 0.0;
    let mut out = vec![0.0];
    for gap in gaps {
        along += gap;
        out.push(along / total);
    }
    out
}

const R: f64 = 10.0;

/// A quarter circle of radius `R * scale` at the places `s`.
fn quarter(s: &[f64], scale: f64) -> Vec<P2> {
    s.iter()
        .map(|s| {
            let a = std::f64::consts::FRAC_PI_2 * s;
            [R * scale * a.pcos(), R * scale * a.psin()]
        })
        .collect()
}

fn off_circle(radius: f64) -> impl Fn(P2) -> f64 {
    move |q: P2| (q[0].phypot(q[1]) - radius).abs()
}

const RB: f64 = 13.155697;
const ROLL: f64 = 0.745;

fn involute(t: f64) -> P2 {
    [
        RB * (t.pcos() + t * t.psin()),
        RB * (t.psin() - t * t.pcos()),
    ]
}

/// The distance from `q` to the involute from roll 0 to [`ROLL`]: Newton
/// on `(P(t) − q)·P'(t) = 0` from the roll whose radius is `q`'s.
fn off_involute(q: P2) -> f64 {
    let ratio = q[0].phypot(q[1]) / RB;
    let mut t = (ratio * ratio - 1.0).max(0.0).sqrt();
    for _ in 0..60 {
        let p = involute(t);
        let d1 = [RB * t * t.pcos(), RB * t * t.psin()];
        let d2 = [
            RB * (t.pcos() - t * t.psin()),
            RB * (t.psin() + t * t.pcos()),
        ];
        let r = [p[0] - q[0], p[1] - q[1]];
        let g = r[0] * d1[0] + r[1] * d1[1];
        let slope = d1[0] * d1[0] + d1[1] * d1[1] + r[0] * d2[0] + r[1] * d2[1];
        if slope.abs() > 1e-300 {
            t = (t - g / slope).clamp(0.0, ROLL);
        }
    }
    let p = involute(t);
    (p[0] - q[0]).phypot(p[1] - q[1])
}

/// The spline passes through each point at its chord-length parameter
/// (worked out here from the definition: the polygon's length up to the
/// point over the whole), on the knots averaged from those parameters, at
/// every degree, count and scale.
#[test]
fn a_chord_spline_passes_through_its_points_at_their_parameters() {
    for scale in [1e-3, 1.0, 1e6] {
        for degree in 1..=7_usize {
            for count in [degree + 1, degree + 2, 12, 40] {
                let through: Vec<P2> = (0..count)
                    .map(|i| {
                        let t = i as f64;
                        [
                            scale * (t * t * 0.4 + 0.3 * (t * 1.7).psin()),
                            scale * (2.0 * (t * 0.9).pcos() + 0.01 * t * t),
                        ]
                    })
                    .collect();
                let size = through
                    .iter()
                    .map(|p| p[0].abs().max(p[1].abs()))
                    .fold(0.0, f64::max);
                let mut along = vec![0.0];
                for pair in through.windows(2) {
                    let d = (pair[1][0] - pair[0][0]).phypot(pair[1][1] - pair[0][1]);
                    along.push(along.last().unwrap() + d);
                }
                let total = *along.last().unwrap();
                let params: Vec<f64> = along.iter().map(|a| a / total).collect();
                let spline = fit(SplineSpacing::Chord, degree as u32, &through);
                assert_eq!(spline.degree, degree);
                assert_eq!(spline.control.len(), count);
                assert_eq!(spline.control[0], through[0]);
                assert_eq!(spline.control[count - 1], through[count - 1]);
                // p + 1 zeros, the means of p parameters in a row, p + 1 ones.
                assert_eq!(spline.knots.len(), count + degree + 1);
                for (j, knot) in spline.knots.iter().enumerate() {
                    let want = if j <= degree {
                        0.0
                    } else if j >= count {
                        1.0
                    } else {
                        params[j - degree..j].iter().sum::<f64>() / degree as f64
                    };
                    assert!((knot - want).abs() <= 1e-15, "knot {j}");
                }
                let mut off: f64 = 0.0;
                for (u, wanted) in params.iter().zip(&through) {
                    let got = point(&spline, *u).unwrap();
                    off = off.max((got[0] - wanted[0]).phypot(got[1] - wanted[1]));
                }
                assert!(
                    off <= 1e-11 * size,
                    "degree {degree}, {count} points, x{scale}: off by {off:e} of {size}"
                );
            }
        }
    }
}

/// Points on a line, spaced as unevenly as F-153's (0, 7.5, 20, 180,
/// 192.5, 200): with chord spacing the parameter of each is its distance
/// along the line over the whole, a linear function of the parameter that
/// the spline reproduces (interpolation by a spline space that holds the
/// lines is exact on them), so the spline *is* the line, run at even
/// speed: never off it, never back. Greville spacing runs back on it (its
/// parameters do not follow the points).
#[test]
fn chord_spacing_gives_points_on_a_line_that_line_at_even_speed() {
    let along = [0.0, 7.5, 20.0, 180.0, 192.5, 200.0];
    for scale in [1e-3, 1.0, 1e6] {
        let (start, direction) = ([3.0 * scale, -2.0 * scale], [0.6, 0.8]);
        let through: Vec<P2> = along
            .iter()
            .map(|a| {
                [
                    start[0] + scale * a * direction[0],
                    start[1] + scale * a * direction[1],
                ]
            })
            .collect();
        for degree in 1..=5_u32 {
            let spline = fit(SplineSpacing::Chord, degree, &through);
            let length = 200.0 * scale;
            for k in 0..=4000 {
                let u = f64::from(k) / 4000.0;
                let got = point(&spline, u).unwrap();
                let want = [
                    start[0] + length * u * direction[0],
                    start[1] + length * u * direction[1],
                ];
                let off = (got[0] - want[0]).phypot(got[1] - want[1]);
                assert!(
                    off <= 1e-12 * length,
                    "degree {degree} x{scale}: {off:e} at u = {u}"
                );
            }
        }
        // Greville spacing runs back somewhere on the same points.
        let spline = fit(SplineSpacing::Greville, 3, &through);
        let mut back = false;
        let mut before = f64::NEG_INFINITY;
        for k in 0..=4000 {
            let got = point(&spline, f64::from(k) / 4000.0).unwrap();
            let a =
                ((got[0] - start[0]) * direction[0] + (got[1] - start[1]) * direction[1]) / scale;
            back |= a < before;
            before = a;
        }
        assert!(back, "greville x{scale}");
    }
}

/// Nine points evenly spaced in `x` on the parabola `y = x²`: Greville
/// spacing strays 6.4348e-3 from it, chord spacing 1.1566e-3 (SciPy's
/// values, within 1 %).
#[test]
fn a_parabola_is_followed_as_the_textbook_interpolant_follows_it() {
    let parabola = |x: f64| [x * x, 2.0 * x, 2.0];
    let through: Vec<P2> = places(9, false)
        .iter()
        .map(|s| [2.0 * s - 1.0, (2.0 * s - 1.0) * (2.0 * s - 1.0)])
        .collect();
    for (spacing, scipy) in [
        (SplineSpacing::Greville, 6.4348e-3),
        (SplineSpacing::Chord, 1.1566e-3),
    ] {
        let off = worst(&fit(spacing, 3, &through), 20000, |q| {
            off_graph(q, parabola)
        });
        assert!((off - scipy).abs() <= 1e-2 * scipy, "{spacing:?}: {off:e}");
    }
}

/// A quarter circle through evenly spaced points: chord spacing's error
/// falls by about 16 each time the points double (SciPy: 15.9, 16.0,
/// 16.0); Greville spacing's by about 4 (its gaps near the ends are a third
/// and two thirds of the others), 73 times as far off with 9 points and
/// 4700 times with 65. The distance from the circle is exact (radius less
/// R); the values are SciPy's, within 1 %.
#[test]
fn the_order_of_the_error_on_a_circle_shows_what_greville_spacing_costs() {
    let scipy: [(usize, [f64; 2]); 4] = [
        (9, [3.0568e-2, 4.1614e-4]),
        (17, [7.7450e-3, 2.6191e-5]),
        (33, [1.9431e-3, 1.6398e-6]),
        (65, [4.8619e-4, 1.0253e-7]),
    ];
    let mut before: Option<[f64; 2]> = None;
    for (n, expected) in scipy {
        let through = quarter(&places(n, false), 1.0);
        let mut now = [0.0; 2];
        for (i, spacing) in SPACINGS.into_iter().enumerate() {
            now[i] = worst(&fit(spacing, 3, &through), 20000, off_circle(R));
            assert!(
                (now[i] - expected[i]).abs() <= 1e-2 * expected[i],
                "{spacing:?}, {n} points: {:e} against {:e}",
                now[i],
                expected[i]
            );
        }
        if let Some(before) = before {
            let ratio = |i: usize| before[i] / now[i];
            assert!((3.5..=4.5).contains(&ratio(0)), "greville: {}", ratio(0));
            assert!(ratio(1) >= 14.0, "chord: {}", ratio(1));
        }
        before = Some(now);
    }
}

/// The same circle through unevenly spaced points (gaps alternating 1 and
/// 3): chord spacing follows them and its error still falls by about 16
/// per doubling (SciPy: 15.8, 16.0, 16.0); Greville spacing's by 4.
#[test]
fn chord_spacing_keeps_its_order_on_unevenly_spaced_points() {
    let scipy: [(usize, [f64; 2]); 4] = [
        (9, [1.6998e-1, 7.6228e-4]),
        (17, [4.2669e-2, 4.8237e-5]),
        (33, [1.0707e-2, 3.0216e-6]),
        (65, [2.6792e-3, 1.8896e-7]),
    ];
    let mut before: Option<[f64; 2]> = None;
    for (n, expected) in scipy {
        let through = quarter(&places(n, true), 1.0);
        let mut now = [0.0; 2];
        for (i, spacing) in SPACINGS.into_iter().enumerate() {
            now[i] = worst(&fit(spacing, 3, &through), 20000, off_circle(R));
            assert!(
                (now[i] - expected[i]).abs() <= 1e-2 * expected[i],
                "{spacing:?}, {n} points: {:e} against {:e}",
                now[i],
                expected[i]
            );
        }
        if let Some(before) = before {
            assert!(before[1] / now[1] >= 14.0, "chord: {}", before[1] / now[1]);
            assert!(
                before[0] / now[0] <= 4.5,
                "greville: {}",
                before[0] / now[0]
            );
        }
        before = Some(now);
    }
}

/// One period of a sine through 17 points, even and uneven (SciPy's
/// values, within 1 %): chord spacing is closer on both.
#[test]
fn a_sine_is_followed_as_closely_as_the_textbook_interpolant_follows_it() {
    let sine = |x: f64| [x.psin(), x.pcos(), -x.psin()];
    let tau = std::f64::consts::TAU;
    for (uneven, expected) in [
        (false, [2.1011e-3, 6.6058e-4]),
        (true, [4.9672e-2, 2.4352e-3]),
    ] {
        let through: Vec<P2> = places(17, uneven)
            .iter()
            .map(|s| [tau * s, (tau * s).psin()])
            .collect();
        for (i, spacing) in SPACINGS.into_iter().enumerate() {
            let off = worst(&fit(spacing, 3, &through), 20000, |q| off_graph(q, sine));
            assert!(
                (off - expected[i]).abs() <= 1e-2 * expected[i],
                "{spacing:?} uneven {uneven}: {off:e} against {:e}",
                expected[i]
            );
        }
    }
}

/// J-034: eight points of an involute evenly spaced in roll angle, as the
/// planetary gearbox drew its flanks. Greville spacing strays 9.5059e-3
/// from it (as 3DCraft 0.1.0 does: its STEP export gives the same control
/// points, and a ray at roll 0.57 meets it at 15.161712 where the
/// involute is at 15.142773); chord spacing 4.3918e-4 (SciPy's values,
/// within 1 %). Five points: 3.1508e-2 and 2.3031e-3. At three scales
/// the splines scale with the points.
#[test]
fn the_involute_of_j034_is_followed_closely_with_chord_spacing() {
    for (count, expected) in [
        (8_usize, [9.5059e-3, 4.3918e-4]),
        (5, [3.1508e-2, 2.3031e-3]),
    ] {
        let through: Vec<P2> = (0..count)
            .map(|i| involute(ROLL * i as f64 / (count - 1) as f64))
            .collect();
        for (i, spacing) in SPACINGS.into_iter().enumerate() {
            let spline = fit(spacing, 3, &through);
            let off = worst(&spline, 200_000, off_involute);
            assert!(
                (off - expected[i]).abs() <= 1e-2 * expected[i],
                "{spacing:?}, {count} points: {off:e} against {:e}",
                expected[i]
            );
            // Scaled points give the spline scaled.
            for scale in [1e-3, 1e6] {
                let scaled: Vec<P2> = through
                    .iter()
                    .map(|p| [p[0] * scale, p[1] * scale])
                    .collect();
                let big = fit(spacing, 3, &scaled);
                for k in 0..=1000 {
                    let u = f64::from(k) / 1000.0;
                    let (a, b) = (point(&spline, u).unwrap(), point(&big, u).unwrap());
                    let d = (a[0] * scale - b[0]).phypot(a[1] * scale - b[1]);
                    assert!(d <= 1e-12 * RB * 1.5 * scale, "{spacing:?} x{scale}: {d:e}");
                }
            }
        }
    }
}

/// Greville spacing is the fit every live spline had before: bit for bit
/// what [`interpolate`] gives on clamped uniform knots.
#[test]
fn greville_spacing_is_the_fit_as_it_was() {
    let through: Vec<P2> = (0..8)
        .map(|i| involute(ROLL * f64::from(i) / 7.0))
        .collect();
    let spline = fit(SplineSpacing::Greville, 3, &through);
    let pairs: Vec<(f64, f64)> = through.iter().map(|p| (p[0], p[1])).collect();
    let control: Vec<P2> = interpolate(3, &pairs)
        .unwrap()
        .into_iter()
        .map(|(x, y)| [x, y])
        .collect();
    assert_eq!(spline.control, control);
    assert_eq!(spline.knots, craft3d_doc::clamped_uniform_knots(3, 8));
}

/// Chord spacing takes its parameters and knots from the formula fit's own
/// chord lengths and averaged knots.
#[test]
fn chord_spacing_shares_the_formula_fits_parameters_and_knots() {
    let through: Vec<P2> = (0..8)
        .map(|i| involute(ROLL * f64::from(i) / 7.0))
        .collect();
    let (params, knots) = chord_parameters(3, &through).unwrap();
    assert_eq!(params, chord_lengths(&through));
    assert_eq!(knots, averaged_knots(3, &params));
    assert_eq!(fit(SplineSpacing::Chord, 3, &through).knots, knots);
}

/// Closed, a spline with chord spacing is refused (not built yet); with
/// Greville spacing it is the periodic fit, as before.
#[test]
fn a_closed_spline_with_chord_spacing_is_refused() {
    let mut ring: Vec<P2> = (0..7)
        .map(|i| {
            let a = std::f64::consts::TAU * f64::from(i) / 7.0;
            [5.0 * a.pcos() + 0.3 * f64::from(i), 4.0 * a.psin()]
        })
        .collect();
    ring.push(ring[0]);
    let greville = interpolated(3, &ring, true, SplineSpacing::Greville).unwrap();
    let pairs: Vec<(f64, f64)> = ring.iter().map(|p| (p[0], p[1])).collect();
    let control: Vec<P2> = interpolate_closed(3, &pairs)
        .unwrap()
        .into_iter()
        .map(|(x, y)| [x, y])
        .collect();
    assert_eq!(greville.control, control);
    let err = interpolated(3, &ring, true, SplineSpacing::Chord).unwrap_err();
    assert!(
        err.contains("not supported yet") && err.contains("greville"),
        "{err}"
    );
}

/// Two points make a line at degree 1 with either spacing; a cubic needs
/// four. With chord spacing two points in a row at one place, two so close
/// that their leg is at most a billionth of the length (1e-12 apart, which
/// the system could not tell apart), all points at one place, and points
/// that are not finite numbers are refused, the close ones named; never a
/// panic or a spline with numbers that are not finite.
#[test]
fn what_chord_spacing_cannot_fit_is_refused_and_nothing_panics() {
    for spacing in SPACINGS {
        let line = fit(spacing, 1, &[[0.0, 0.0], [3.0, 4.0]]);
        let middle = point(&line, 0.5).unwrap();
        assert!((middle[0] - 1.5).abs() < 1e-15 && (middle[1] - 2.0).abs() < 1e-15);
        assert!(interpolated(3, &[[0.0, 0.0], [3.0, 4.0]], false, spacing).is_err());
        assert!(interpolated(0, &[[0.0, 0.0], [3.0, 4.0]], false, spacing).is_err());
        assert!(interpolated(8, &[[0.0, 0.0]; 12], false, spacing).is_err());
    }
    let chord = |points: &[P2]| interpolated(3, points, false, SplineSpacing::Chord);
    let twice = [[0.0, 0.0], [1.0, 1.0], [1.0, 1.0], [2.0, 0.0], [3.0, 1.0]];
    let err = chord(&twice).unwrap_err();
    assert!(
        err.contains("points 2 and 3") && err.contains("same place"),
        "{err}"
    );
    let close = [
        [0.0, 0.0],
        [1.0, 1.0],
        [1.0 + 1e-12, 1.0],
        [2.0, 0.0],
        [3.0, 1.0],
    ];
    let err = chord(&close).unwrap_err();
    assert!(
        err.contains("points 2 and 3") && err.contains("apart"),
        "{err}"
    );
    // Just above a billionth of the length: fitted, finite.
    let near = [
        [0.0, 0.0],
        [1.0, 1.0],
        [1.0 + 1e-8, 1.0],
        [2.0, 0.0],
        [3.0, 1.0],
    ];
    if let Ok(spline) = chord(&near) {
        assert!(spline.control.iter().flatten().all(|v| v.is_finite()));
    }
    let err = chord(&[[1.0, 2.0]; 5]).unwrap_err();
    assert!(err.contains("no length"), "{err}");
    for bad in [f64::NAN, f64::INFINITY, -f64::INFINITY] {
        let mut points = vec![[0.0, 0.0], [1.0, 0.5], [2.0, 0.0], [3.0, 1.0]];
        points[2][1] = bad;
        assert!(chord(&points).is_err(), "{bad}");
    }
    // Huge but finite: a spline of finite numbers, or a refusal; tiny
    // (subnormal) coordinates likewise.
    for points in [
        vec![[0.0, 0.0], [1e308, 0.0], [-1e308, 1e308], [1e308, 1e308]],
        vec![[0.0, 0.0], [1e-310, 0.0], [2e-310, 1e-310], [3e-310, 0.0]],
        vec![[0.0, 0.0], [1e300, 0.0], [2e300, 1e300], [3e300, 0.0]],
    ] {
        if let Ok(spline) = chord(&points) {
            assert!(spline.control.iter().flatten().all(|v| v.is_finite()));
        }
    }
    let many = vec![[0.0, 0.0]; MAX_THROUGH_POINTS + 1];
    let err = chord(&many).unwrap_err();
    assert!(err.contains("at most"), "{err}");
    // The knots of too few parameters, or of degree 0, are none.
    assert!(averaged_knots(3, &[0.0, 0.5, 1.0]).is_empty());
    assert!(averaged_knots(0, &[0.0, 1.0]).is_empty());
}

/// The same points give the same spline, bit for bit, every time; and a
/// spline through the points in the other order is the same curve run
/// the other way (to rounding).
#[test]
fn a_chord_spline_is_deterministic_and_the_same_run_backwards() {
    let through: Vec<P2> = (0..9)
        .map(|i| {
            let t = f64::from(i);
            [t * t * 0.7 + (t * 0.5).psin(), (t * 1.3).pcos() * 3.0]
        })
        .collect();
    let mut backwards = through.clone();
    backwards.reverse();
    let a = fit(SplineSpacing::Chord, 3, &through);
    assert_eq!(a, fit(SplineSpacing::Chord, 3, &through));
    let b = fit(SplineSpacing::Chord, 3, &backwards);
    for k in 0..=1000 {
        let u = f64::from(k) / 1000.0;
        let (p, q) = (point(&a, u).unwrap(), point(&b, 1.0 - u).unwrap());
        assert!((p[0] - q[0]).phypot(p[1] - q[1]) <= 1e-11, "at {u}");
    }
}
