//! Bodies that touch along a stretch of a border their freeform faces
//! share (2026-10-27, the sea kayak's touching plates).
//!
//! Two humps extruded from a cubic Bézier and its mirror image stand back
//! to back: their spline walls share the ruling through the point where
//! the curves start, and one hump is taller, so the taller wall's border
//! lies on the other wall's surface only up to the lower hump's height.
//! Touching, they have no volume in common (the intersection is empty,
//! the difference is the target, the overlap assertion passes); moved
//! into each other by half a unit, the intersection is the lens between
//! the two curves times the lower height, in closed form. At 1e-3, 1 and
//! 1e6, with the tolerance scaled, each hump the taller in turn.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;

use common::checked::{body, built, failed, new_document, run, session};
use craft3d_engine::Session;
use serde_json::json;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

/// The hump's curve, `x = 3t + 3t² − 2t³`, `y = 6t(1 − t)` (control
/// points (0, 0), (1, 2), (3, 2), (4, 0)), mirrored by `side`, moved by
/// `shift` along x; the profile closed by its base, extruded by `height`.
fn hump(s: &mut Session, name: &str, side: f64, shift: f64, height: f64, k: f64) {
    let sketch = format!("{name}Sketch");
    run(s, "sketch.create", json!({"name": sketch}));
    let control: Vec<[f64; 2]> = [[0.0, 0.0], [1.0, 2.0], [3.0, 2.0], [4.0, 0.0]]
        .iter()
        .map(|[x, y]| [(side * x + shift) * k, y * k])
        .collect();
    let spline = run(
        s,
        "sketch.spline",
        json!({"sketch": sketch, "control": control, "degree": 3}),
    );
    let ends = &spline["control"];
    run(
        s,
        "sketch.line",
        json!({"sketch": sketch, "from": ends[3], "to": ends[0]}),
    );
    built(
        s,
        "feature.extrude",
        json!({"sketch": sketch, "distance": height * k, "name": name}),
    );
}

/// The area of the hump's profile: `∫ y dx = 4.2`.
const AREA: f64 = 4.2;

/// The lens the two humps share when the mirrored one is moved by `d`
/// along x: between the rising branch `x(t)` and `d − x(t)`, up to where
/// they meet (`x(t*) = d/2`), `∫ (d − 2x) y' dt`, a quartic integrated
/// exactly by three Gauss points.
fn lens(d: f64) -> f64 {
    let x = |t: f64| 3.0 * t + 3.0 * t * t - 2.0 * t.ppowi(3);
    let dx = |t: f64| 3.0 + 6.0 * t - 6.0 * t * t;
    let mut t = 0.1;
    for _ in 0..60 {
        t -= (x(t) - 0.5 * d) / dx(t);
    }
    let f = |s: f64| (d - 2.0 * x(s)) * (6.0 - 12.0 * s);
    let (h, m) = (0.5 * t, 0.5 * t);
    let g = (0.6_f64).sqrt();
    h * (5.0 / 9.0 * f(m - h * g) + 8.0 / 9.0 * f(m) + 5.0 / 9.0 * f(m + h * g))
}

fn close(found: f64, expected: f64, k: f64, what: &str) {
    let slack = 1e-7 * expected.abs().max(k * k * k);
    assert!(
        (found - expected).abs() <= slack,
        "{what} at {k}: {found} against {expected}"
    );
}

#[test]
fn humps_that_touch_along_part_of_a_ruling_share_no_volume() {
    for k in SCALES {
        for (left, right) in [(2.0, 3.0), (3.0, 2.0)] {
            let mut s = session();
            new_document(&mut s, k);
            hump(&mut s, "A", 1.0, 0.0, left, k);
            hump(&mut s, "B", -1.0, 0.0, right, k);
            let what = format!("heights {left} and {right}");
            let reason = failed(
                &mut s,
                "feature.boolean",
                json!({"op": "intersect", "target": "A", "tool": "B", "keepTool": true, "name": "Common"}),
            );
            assert!(
                reason.contains("the result is empty"),
                "{what} at {k}: {reason}"
            );
            run(&mut s, "feature.delete", json!({"feature": "Common"}));
            built(
                &mut s,
                "feature.boolean",
                json!({"op": "subtract", "target": "A", "tool": "B", "keepTool": true, "name": "Rest"}),
            );
            let rest = body(&mut s, "Rest");
            close(
                rest["volume"].as_f64().unwrap(),
                AREA * left * k * k * k,
                k,
                &what,
            );
            let held = run(
                &mut s,
                "assert.add",
                json!({"name": "apart", "measure": "overlap", "of": ["Rest", "B"], "is": false}),
            );
            assert_eq!(held["status"], "passed", "{what} at {k}: {held}");
        }
    }
}

#[test]
fn humps_moved_into_each_other_share_the_lens_between_their_curves() {
    let d = 0.5;
    for k in SCALES {
        for (left, right) in [(2.0, 3.0), (3.0, 2.0)] {
            let mut s = session();
            new_document(&mut s, k);
            hump(&mut s, "A", 1.0, 0.0, left, k);
            hump(&mut s, "B", -1.0, d, right, k);
            built(
                &mut s,
                "feature.boolean",
                json!({"op": "intersect", "target": "A", "tool": "B", "keepTool": true, "name": "Common"}),
            );
            let common = body(&mut s, "Common");
            close(
                common["volume"].as_f64().unwrap(),
                lens(d) * f64::min(left, right) * k * k * k,
                k,
                &format!("heights {left} and {right}"),
            );
        }
    }
}

#[test]
fn the_lens_is_what_the_curves_give() {
    // The oracle itself, against a sum of thin strips across y.
    let d = 0.5;
    let x = |t: f64| 3.0 * t + 3.0 * t * t - 2.0 * t.ppowi(3);
    let y = |t: f64| 6.0 * t * (1.0 - t);
    let n = 200_000;
    let mut area = 0.0;
    for i in 0..n {
        let (a, b) = (f64::from(i) / f64::from(n), f64::from(i + 1) / f64::from(n));
        let (xm, dy) = (0.5 * (x(a) + x(b)), y(b) - y(a));
        if xm < 0.5 * d && dy > 0.0 {
            area += (d - 2.0 * xm) * dy;
        }
    }
    assert!((area - lens(d)).abs() <= 1e-8, "{area} {}", lens(d));
}
