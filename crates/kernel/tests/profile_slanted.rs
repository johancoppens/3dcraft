//! Thin slanted polygons are profiles: a clean rectangle at any angle and
//! scale is detected with its closed-form area, in either winding, while
//! polygons that really cross or touch themselves are still refused.
//!
//! The rectangles once failed about once in three hundred: where a short
//! side leaves the tolerance region of a long side, the region's boundary
//! was sampled, and which side of the tolerance such a sample fell on was
//! rounding noise (plan/r1-cleanup.md item 58).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{P, Rng, polygon};
use craft3d_kernel::KernelError;
use craft3d_kernel::profile::detect_profiles;
use craft3d_math::Tolerance;
use std::f64::consts::TAU;

const SCALES: [f64; 4] = [1e-3, 1.0, 1e3, 1e6];

fn uniform(rng: &mut Rng, lo: f64, hi: f64) -> f64 {
    lo + (hi - lo) * rng.unit()
}

/// The polygon's area by the shoelace formula, the oracle.
fn shoelace(points: &[P]) -> f64 {
    let n = points.len();
    (0..n)
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % n]);
            a[0] * b[1] - a[1] * b[0]
        })
        .sum::<f64>()
        .abs()
        / 2.0
}

/// Detects the closed polygon's profile and checks it is the one loop it
/// should be, with the polygon's area.
fn assert_profile(points: &[P], what: &str) {
    let curves = polygon(1, 1, points);
    let profiles = detect_profiles(&curves, &Tolerance::default())
        .unwrap_or_else(|e| panic!("{what}: {e:?}\n{points:?}"));
    assert_eq!(profiles.len(), 1, "{what}");
    assert!(profiles[0].holes.is_empty(), "{what}");
    let (area, expected) = (profiles[0].area(), shoelace(points));
    assert!(
        (area - expected).abs() <= 1e-9 * expected,
        "{what}: area {area}, expected {expected}"
    );
}

/// The rectangle `l × w` centred on `c`, turned by `angle`, its corners in
/// counter-clockwise order (clockwise when `reverse`).
fn rectangle(c: P, l: f64, w: f64, angle: f64, reverse: bool) -> Vec<P> {
    let (s, co) = angle.sin_cos();
    let mut points: Vec<P> = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
        .iter()
        .map(|(x, y)| {
            let (u, v) = (x * l / 2.0, y * w / 2.0);
            [c[0] + u * co - v * s, c[1] + u * s + v * co]
        })
        .collect();
    if reverse {
        points.reverse();
    }
    points
}

#[test]
fn the_sundial_rectangle_is_a_profile() {
    // The corners of plan/repro/repro-sundial-2.json, 44 x 1.2 mm.
    let points = [
        [16.697056638762756, -31.352678659982697],
        [56.860039611356406, -50.52974301317225],
        [56.342980354640936, -51.612632538952624],
        [16.179997382047294, -32.435568185763074],
    ];
    assert_profile(&points, "as drawn");
    let mut reversed = points;
    reversed.reverse();
    assert_profile(&reversed, "reversed");
}

#[test]
fn thin_slanted_rectangles_are_profiles_at_every_scale() {
    let mut rng = Rng(58);
    for k in SCALES {
        for i in 0..2000 {
            let c = [
                uniform(&mut rng, -50.0, 50.0) * k,
                uniform(&mut rng, -50.0, 50.0) * k,
            ];
            let l = uniform(&mut rng, 10.0, 60.0) * k;
            let w = uniform(&mut rng, 0.2, 3.0) * k;
            let angle = uniform(&mut rng, 0.0, TAU);
            let reverse = rng.below(2) == 1;
            let points = rectangle(c, l, w, angle, reverse);
            assert_profile(&points, &format!("scale {k}, rectangle {i}"));
        }
    }
}

/// The shortest distance from `p` to the segment `a b`.
fn to_segment(p: P, a: P, b: P) -> f64 {
    let d = [b[0] - a[0], b[1] - a[1]];
    let l2 = d[0] * d[0] + d[1] * d[1];
    let t = (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / l2).clamp(0.0, 1.0);
    (p[0] - a[0] - t * d[0]).hypot(p[1] - a[1] - t * d[1])
}

#[test]
fn random_star_shaped_polygons_are_profiles_at_every_scale() {
    // Corners at increasing angles about a centre: a simple polygon. Those
    // with a corner nearer than a thousandth of their size to a side it is
    // not on are left out (they are not clearly clear).
    let mut rng = Rng(59);
    for k in SCALES {
        let mut tried = 0;
        while tried < 1000 {
            let n = 3 + rng.below(10) as usize;
            let c = [
                uniform(&mut rng, -50.0, 50.0) * k,
                uniform(&mut rng, -50.0, 50.0) * k,
            ];
            let turn = uniform(&mut rng, 0.0, TAU);
            let mut angles: Vec<f64> = (0..n).map(|_| uniform(&mut rng, 0.0, TAU)).collect();
            angles.sort_by(f64::total_cmp);
            // With a gap of half a turn or more between two corners, the
            // side across it passes the centre and may cross others.
            let wide = (0..n).any(|i| {
                let next = if i + 1 < n {
                    angles[i + 1]
                } else {
                    angles[0] + TAU
                };
                next - angles[i] >= 0.9 * std::f64::consts::PI
            });
            if wide {
                continue;
            }
            let mut points: Vec<P> = angles
                .iter()
                .map(|a| {
                    let r = uniform(&mut rng, 0.05, 1.0) * 40.0 * k;
                    let (s, co) = (a + turn).sin_cos();
                    [c[0] + r * co, c[1] + r * s]
                })
                .collect();
            if rng.below(2) == 1 {
                points.reverse();
            }
            let crowded = (0..n).any(|i| {
                (0..n).any(|j| {
                    let next = (j + 1) % n;
                    i != j && i != next && to_segment(points[i], points[j], points[next]) < 0.04 * k
                })
            });
            if crowded || shoelace(&points) < 1.0 * k * k {
                continue;
            }
            tried += 1;
            assert_profile(&points, &format!("scale {k}, polygon {tried}"));
        }
    }
}

fn assert_crossing(points: &[P], what: &str) {
    let curves = polygon(1, 1, points);
    match detect_profiles(&curves, &Tolerance::default()) {
        Err(KernelError::CrossingCurves { .. }) => {}
        other => panic!("{what}: {other:?}"),
    }
}

#[test]
fn polygons_that_cross_or_touch_themselves_are_still_refused() {
    let mut rng = Rng(60);
    for k in SCALES {
        for i in 0..300 {
            let c = [
                uniform(&mut rng, -50.0, 50.0) * k,
                uniform(&mut rng, -50.0, 50.0) * k,
            ];
            let l = uniform(&mut rng, 10.0, 60.0) * k;
            let w = uniform(&mut rng, 0.2, 3.0) * k;
            let angle = uniform(&mut rng, 0.0, TAU);
            let corners = rectangle(c, l, w, angle, rng.below(2) == 1);
            let (a, b, cc, d) = (corners[0], corners[1], corners[2], corners[3]);
            // A bow tie: the thin rectangle with its last two corners
            // swapped, one of them moved on along the side so that the two
            // halves do not cancel to no area.
            let further = [cc[0] + (b[0] - a[0]) / 2.0, cc[1] + (b[1] - a[1]) / 2.0];
            assert_crossing(&[a, b, d, further], &format!("scale {k}, bow tie {i}"));
            // A corner pushed onto the opposite side: a pentagon whose
            // fifth corner lies on the side from a to b, halfway.
            let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
            assert_crossing(&[a, b, cc, mid, d], &format!("scale {k}, touch {i}"));
            // And pushed through it, by a tenth of the width.
            let inward = [(d[0] - a[0]) / 10.0, (d[1] - a[1]) / 10.0];
            let through = [mid[0] - inward[0], mid[1] - inward[1]];
            assert_crossing(&[a, b, cc, through, d], &format!("scale {k}, through {i}"));
        }
    }
}
