//! The review of `check/spline-box-domain` (a93e8aea, 2026-11-26): the
//! search by Bézier pieces (`nearest_by_pieces`, what `nearest` falls
//! back on when its stretches run out) over a range past its spline's
//! domain reads the end pieces carried on
//! (`NurbsCurve::continued_pieces`). For a range that starts at or past
//! the domain's end those are cut from the start of the last span, not
//! from where the range starts (`nurbs_continued.rs` › `continued`: `from`
//! is the range's start only while it lies before the domain's end), so
//! the search looks at parameters before the range and may answer one of
//! them: a point of the curve that is not on the edge, at a distance the
//! edge does not come within. On `d835d891` such a range had no pieces
//! and the search answered nothing.

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use super::nearest_by_pieces;
use craft3d_geom::NurbsCurve;
use craft3d_math::Vec3;

/// A cubic of four uneven spans over [0, 1], times `k`; its last span is
/// [0.6, 1].
fn wavy(k: f64) -> NurbsCurve {
    NurbsCurve::polynomial(
        3,
        vec![0.0, 0.0, 0.0, 0.0, 0.2, 0.5, 0.6, 1.0, 1.0, 1.0, 1.0],
        vec![
            Vec3::new(0.0, 0.0, 0.0) * k,
            Vec3::new(1.0, 2.0, -1.0) * k,
            Vec3::new(2.0, -1.0, 0.5) * k,
            Vec3::new(3.0, 0.5, 2.0) * k,
            Vec3::new(4.0, 1.0, -0.5) * k,
            Vec3::new(5.0, -2.0, 0.0) * k,
            Vec3::new(6.0, 0.0, 1.0) * k,
        ],
    )
    .unwrap()
}

/// A rational quarter circle of radius `k` over [0, 1], which stays on its
/// circle carried on.
fn quarter(k: f64) -> NurbsCurve {
    NurbsCurve::new(
        2,
        vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        vec![
            Vec3::new(k, 0.0, 0.0),
            Vec3::new(k, k, 0.0),
            Vec3::new(0.0, k, 0.0),
        ],
        vec![1.0, 0.5_f64.sqrt(), 1.0],
    )
    .unwrap()
}

#[test]
fn the_search_by_pieces_answers_within_a_range_that_starts_at_or_past_the_domains_end() {
    for k in [1e-3, 1.0, 1e6] {
        for (curve, range, at) in [
            (wavy(k), (1.2, 1.5), 0.9),
            (wavy(k), (1.2, 1.5), 1.1),
            (wavy(k), (1.0, 1.4), 0.8),
            (quarter(k), (4.0, 9.0), 0.5),
            (quarter(k), (1.0, 3.0), 0.3),
        ] {
            let p = curve.eval(at);
            let distance = |t: f64| curve.eval(t).distance(p);
            let first = (distance(range.0), range.0);
            if let Some((d, t)) = nearest_by_pieces(&curve, range, p, &distance, first) {
                assert!(
                    (range.0..=range.1).contains(&t),
                    "{k} {range:?}, the point at {at}: answered {t} at {d}, outside the range"
                );
            }
        }
    }
}
