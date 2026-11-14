//! `NurbsCurve::hull_points`: points whose hull holds a curve over part
//! of its domain (2026-10-28, the boolean's near boxes of faces).

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_geom::{NurbsCurve, hull_points_in, piece_points_over};
use craft3d_math::{Bbox3, Frame, Vec3};

/// A cubic of uneven spans whose last control point is far out.
fn spline(k: f64) -> NurbsCurve {
    NurbsCurve::polynomial(
        3,
        vec![0.0, 0.0, 0.0, 0.0, 0.1, 0.15, 0.6, 1.0, 1.0, 1.0, 1.0],
        vec![
            Vec3::new(0.0, 0.0, 0.0) * k,
            Vec3::new(1.0, 2.0, -1.0) * k,
            Vec3::new(2.0, -1.0, 0.5) * k,
            Vec3::new(3.0, 0.5, 2.0) * k,
            Vec3::new(4.0, 1.0, -0.5) * k,
            Vec3::new(5.0, 0.0, 0.0) * k,
            Vec3::new(1e6, -1e6, 1e6) * k,
        ],
    )
    .unwrap()
}

/// Every sampled point of the curve over `range` within the box of its
/// hull points (to rounding), and the box no larger than `cap` times the
/// sampled one's diagonal.
fn holds(curve: &NurbsCurve, range: (f64, f64), k: f64, cap: f64) {
    let points = curve.hull_points(range).unwrap();
    let held = Bbox3::from_points(points.iter().copied());
    let mut sampled = Bbox3::EMPTY;
    for i in 0..=512 {
        let t = range.0 + (range.1 - range.0) * f64::from(i) / 512.0;
        let p = curve.eval(t);
        assert!(
            held.contains(p, 1e-12 * k),
            "{k} {range:?}: {p:?} outside {held:?}"
        );
        sampled.add(p);
    }
    assert!(
        held.diagonal() <= cap * sampled.diagonal().max(1e-9 * k),
        "{k} {range:?}: {held:?} against {sampled:?}"
    );
}

#[test]
fn the_hull_of_part_of_a_spline_holds_that_part_and_leaves_far_control_points_out() {
    for k in [1e-3, 1.0, 1e6] {
        let c = spline(k);
        // The whole domain: the control points themselves.
        assert_eq!(c.hull_points((0.0, 1.0)).unwrap(), c.control_points());
        assert_eq!(c.hull_points((1.0, -0.5)).unwrap(), c.control_points());
        for range in [
            (0.0, 0.5),
            (0.05, 0.12),
            (0.12, 0.13),
            (0.3, 0.1),
            (0.2, 0.55),
        ] {
            holds(&c, range, k, 8.0);
        }
        // Past the domain on one side only, or not finite: none.
        assert!(c.hull_points((-0.1, 0.5)).is_none());
        assert!(c.hull_points((0.5, f64::NAN)).is_none());
        // One parameter: the point (within rounding).
        let one = c.hull_points((0.3, 0.3)).unwrap();
        assert!(one.iter().all(|p| p.distance(c.eval(0.3)) <= 1e-12 * k));
    }
}

#[test]
fn the_hull_of_part_of_a_rational_circle_holds_that_part() {
    for k in [1e-3, 1.0, 1e6] {
        let frame = Frame::WORLD.at(Vec3::new(1.0, -2.0, 3.0) * k).unwrap();
        let c = NurbsCurve::circle(&frame, 2.0 * k).unwrap();
        let (d0, d1) = c.domain();
        for (a, b) in [(0.0, 0.1), (0.13, 0.42), (0.5, 0.97), (0.01, 0.02)] {
            let range = (d0 + (d1 - d0) * a, d0 + (d1 - d0) * b);
            holds(&c, range, k, 4.0);
        }
    }
}

/// A number in `[0, 1)` from `seed`, which it moves on (a fixed sequence).
fn next(seed: &mut u64) -> f64 {
    *seed = seed
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    (*seed >> 11) as f64 / (1u64 << 53) as f64
}

/// A spline of `degree` made from `seed`: clamped at each end or not, its
/// inner knots of every multiplicity up to the degree plus one (where the
/// curve may jump), evenly or unevenly spaced, rational or not.
fn random_spline(degree: usize, seed: &mut u64, k: f64) -> NurbsCurve {
    let mut knots = Vec::new();
    let mut at = 0.0;
    let first = if next(seed) < 0.6 {
        degree + 1
    } else {
        1 + (next(seed) * degree as f64) as usize
    };
    knots.extend(std::iter::repeat_n(at, first));
    let inner = 1 + (next(seed) * 9.0) as usize;
    for _ in 0..inner {
        at += if next(seed) < 0.5 {
            1.0
        } else {
            0.01 + next(seed)
        };
        let most = if next(seed) < 0.1 { degree + 1 } else { degree };
        let times = 1 + (next(seed) * most as f64) as usize;
        knots.extend(std::iter::repeat_n(at, times.min(most)));
    }
    at += 0.5 + next(seed);
    let last = if next(seed) < 0.6 {
        degree + 1
    } else {
        1 + (next(seed) * degree as f64) as usize
    };
    knots.extend(std::iter::repeat_n(at, last));
    // Enough knots for a domain that is not empty.
    while knots.len() < 2 * degree + 2 {
        at += 1.0;
        knots.push(at);
    }
    let count = knots.len() - degree - 1;
    let ctrl: Vec<Vec3> = (0..count)
        .map(|_| Vec3::new(next(seed) - 0.5, next(seed) - 0.5, next(seed) - 0.5) * k)
        .collect();
    let rational = next(seed) < 0.5;
    let weights: Vec<f64> = (0..count)
        .map(|_| {
            if rational {
                0.3 + 2.7 * next(seed)
            } else {
                1.0
            }
        })
        .collect();
    NurbsCurve::new(degree, knots, ctrl, weights).unwrap()
}

/// The Bézier piece of one span made from that span alone
/// (`NurbsCurve::bezier_piece`) is the piece the decomposition of the whole
/// curve gives for it (`bezier_pieces`), to rounding: on 1500 splines of
/// degrees 1 to 5, rational and not, clamped and not, with inner knots of
/// every multiplicity, at three scales; and `span_at` finds each span from
/// a parameter inside it, the first before the domain and the last past
/// it.
#[test]
fn the_piece_of_one_span_is_the_piece_the_whole_curves_decomposition_gives() {
    let mut seed = 0x5eed_u64;
    let mut made = 0;
    for case in 0..1500 {
        let degree = 1 + case % 5;
        let k = [1e-3, 1.0, 1e6][case % 3];
        let curve = match std::panic::catch_unwind(move || {
            let mut s = seed;
            (random_spline(degree, &mut s, k), s)
        }) {
            Ok((curve, moved)) => {
                seed = moved;
                curve
            }
            // Knots that leave no domain: another.
            Err(_) => {
                seed = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
                continue;
            }
        };
        let whole = curve.bezier_pieces();
        let knots = curve.knots();
        let (d0, d1) = curve.domain();
        let spans: Vec<usize> = (degree..curve.control_points().len())
            .filter(|i| knots[i + 1] > knots[*i] && knots[*i] >= d0 && knots[i + 1] <= d1)
            .collect();
        assert_eq!(spans.len(), whole.len(), "case {case}");
        assert!(!spans.is_empty(), "case {case}");
        for (span, ((lo, hi), points)) in spans.iter().zip(&whole) {
            let ((a, b), one) = curve.bezier_piece(*span).unwrap();
            assert_eq!((a, b), (*lo, *hi), "case {case} span {span}");
            assert_eq!(one.len(), points.len());
            for ((p, w), (q, x)) in one.iter().zip(points) {
                assert!(
                    p.distance(*q) <= 1e-9 * k && (w - x).abs() <= 1e-9 * x,
                    "case {case} span {span}: {p:?} {w} against {q:?} {x}"
                );
            }
            assert_eq!(curve.span_at(0.5 * (lo + hi)), *span, "case {case}");
            assert_eq!(curve.span_at(*lo), *span, "case {case}");
            made += 1;
        }
        assert_eq!(curve.span_at(d0 - 1.0), spans[0], "case {case}");
        assert_eq!(curve.span_at(d1), *spans.last().unwrap(), "case {case}");
        assert_eq!(
            curve.span_at(d1 + 1.0),
            *spans.last().unwrap(),
            "case {case}"
        );
        // A span of no width, and one that is not the curve's, have none.
        for i in 0..knots.len() {
            if !spans.contains(&i) {
                assert!(curve.bezier_piece(i).is_none(), "case {case} span {i}");
            }
        }
    }
    assert!(made > 5000, "{made}");
}

/// A piece cut to a range inside its span gives the points `hull_points_in`
/// gives, bit for bit; cut to a range that runs past the span, points whose
/// box holds the curve continued there, which is the curve past the ends of
/// its domain (as `eval` continues it).
#[test]
fn a_piece_cut_past_its_span_holds_the_curve_continued() {
    let mut seed = 0xc0ffee_u64;
    for case in 0..200 {
        let degree = 1 + case % 5;
        let k = [1e-3, 1.0, 1e6][case % 3];
        // Clamped, so that the first and the last span are the domain's.
        let mut knots = vec![0.0; degree + 1];
        let inner = 1 + case % 4;
        for i in 0..inner {
            knots.push((i + 1) as f64);
        }
        knots.extend(std::iter::repeat_n((inner + 1) as f64, degree + 1));
        let count = knots.len() - degree - 1;
        let ctrl: Vec<Vec3> = (0..count)
            .map(|_| Vec3::new(next(&mut seed), next(&mut seed), next(&mut seed)) * k)
            .collect();
        let weights: Vec<f64> = (0..count)
            .map(|_| {
                if case % 2 == 0 {
                    1.0
                } else {
                    0.8 + 0.4 * next(&mut seed)
                }
            })
            .collect();
        let curve = NurbsCurve::new(degree, knots, ctrl, weights).unwrap();
        let (d0, d1) = curve.domain();
        let first = curve.bezier_piece(curve.span_at(d0)).unwrap();
        let last = curve.bezier_piece(curve.span_at(d1)).unwrap();
        let inside = (d0 + 0.2, d0 + 0.7);
        assert_eq!(
            piece_points_over(&first, inside),
            hull_points_in(std::slice::from_ref(&first), inside),
            "case {case}"
        );
        for (piece, range) in [
            (&first, (d0 - 0.25, d0 + 0.5)),
            (&first, (d0 - 1e-9, d0 + 1.0)),
            (&last, (d1 - 0.5, d1 + 0.25)),
            (&last, (d1 - 1.0, d1 + 1e-9)),
        ] {
            let Some(points) = piece_points_over(piece, range) else {
                // A rational piece continued to where a weight is not
                // positive has no hull: never for a polynomial one.
                assert!(case % 2 == 1, "case {case} {range:?}");
                continue;
            };
            let held = Bbox3::from_points(points.iter().copied());
            for i in 0..=64 {
                let t = range.0 + (range.1 - range.0) * f64::from(i) / 64.0;
                let p = curve.eval(t);
                assert!(
                    held.contains(p, 1e-9 * k),
                    "case {case} {range:?}: {p:?} outside {held:?}"
                );
            }
        }
        // Empty or backward ranges, and one that starts past the span.
        assert!(piece_points_over(&first, (d0 + 0.5, d0 + 0.2)).is_none());
        assert!(piece_points_over(&first, (d0 + 1.0, d0 + 2.0)).is_none());
        assert!(piece_points_over(&first, (f64::NAN, d0 + 0.5)).is_none());
    }
}
