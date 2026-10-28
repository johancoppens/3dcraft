//! `sheetMetal.bendSequence` (docs/design/sheetmetal.md › *Bend
//! sequence*), held to closed forms at 1e-3, 1 and 1e6: a U channel
//! (base 40 wide between its tangent lines, walls up at both sides, inner
//! radius R = 1, T = 1, K = ½) bent with a straight punch 10 wide.
//!
//! - Walls 20 high: both orders clear; the second wall's least distance is
//!   the first wall's inner top corner to the punch's flank at the end of
//!   the stroke, where both legs rise 45 degrees: `(R + W − H) / √2 − p`.
//! - Walls 40 high: the first wall's top swings into the punch before the
//!   second bend reaches 90 degrees (at `2 (atan2(R + W, H) − asin(p /
//!   √((R + W)² + H²)))`, 81.4 degrees), in either order: no order, the
//!   collisions named by bend and stage.
//! - The same walls with a gooseneck punch, its throat open towards the
//!   standing wall: an order that clears.
//! - Bad parameters are errors.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{built, error, new_document, run, session_in, workdir};
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::f64::consts::SQRT_2;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn mm(v: f64) -> String {
    format!("{v} mm")
}

/// The U channel: the base 40 × 30, walls `h` high on its left and right
/// edges.
fn channel(s: &mut Session, k: f64, h: f64) {
    run(s, "sketch.create", json!({"plane": "xy", "name": "Base"}));
    let made = run(
        s,
        "sketch.rect",
        json!({"sketch": "Base", "from": [0.0, 0.0], "to": [40.0 * k, 30.0 * k]}),
    );
    let lines: Vec<String> = made["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap().to_string())
        .collect();
    built(
        s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "name": "U", "thickness": mm(k), "radius": mm(k),
               "kFactor": 0.5, "flanges": [
                   {"edge": lines[3], "length": mm(h * k), "angle": "90 deg"},
                   {"edge": lines[1], "length": mm(h * k), "angle": "90 deg"}]}),
    );
}

fn scaled(points: &[[f64; 2]], k: f64) -> Value {
    json!(
        points
            .iter()
            .map(|p| [p[0] * k, p[1] * k])
            .collect::<Vec<_>>()
    )
}

const STRAIGHT: [[f64; 2]; 4] = [[-5.0, 0.0], [5.0, 0.0], [5.0, 100.0], [-5.0, 100.0]];
const GOOSENECK: [[f64; 2]; 6] = [
    [-25.0, 0.0],
    [5.0, 0.0],
    [5.0, 30.0],
    [-5.0, 30.0],
    [-5.0, 150.0],
    [-25.0, 150.0],
];
const DIE: [[f64; 2]; 4] = [[-15.0, -20.0], [15.0, -20.0], [15.0, 0.0], [-15.0, 0.0]];

fn sequence(s: &mut Session, k: f64, punch: &[[f64; 2]]) -> Value {
    run(
        s,
        "sheetMetal.bendSequence",
        json!({"feature": "U", "punch": scaled(punch, k), "die": scaled(&DIE, k)}),
    )
}

#[test]
fn short_walls_clear_by_the_closed_form() {
    for k in SCALES {
        let dir = workdir("sheetmetal-bendsequence", &format!("short-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        channel(&mut s, k, 20.0);
        let out = sequence(&mut s, k, &STRAIGHT);
        assert_eq!(out["feasible"], true, "{out}");
        assert_eq!(out["order"], json!([0, 1]), "{out}");
        let steps = out["steps"].as_array().unwrap();
        assert_eq!(steps[1]["done"], json!([0]), "{out}");
        let (r, w, h, p) = (1.0 * k, 40.0 * k, 20.0 * k, 5.0 * k);
        let least = (r + w - h) / SQRT_2 - p;
        let got = steps[1]["least"].as_f64().unwrap();
        assert!(
            (got - least).abs() <= 1e-6 * k,
            "scale {k}: least {got} against {least}"
        );
        assert!(
            (steps[1]["at"].as_f64().unwrap() - 90.0).abs() < 1e-9,
            "{out}"
        );
        assert!(steps[0]["least"].as_f64().unwrap() >= least, "{out}");
        assert!(out["collisions"].as_array().unwrap().is_empty(), "{out}");
    }
}

#[test]
fn tall_walls_hit_a_straight_punch_in_either_order_and_clear_a_gooseneck() {
    for k in SCALES {
        let dir = workdir("sheetmetal-bendsequence", &format!("tall-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        channel(&mut s, k, 40.0);
        let out = sequence(&mut s, k, &STRAIGHT);
        assert_eq!(out["feasible"], false, "{out}");
        assert_eq!(out["order"], Value::Null, "{out}");
        let (a, b, p) = (41.0_f64, 40.0_f64, 5.0_f64);
        let first = 2.0 * (a.atan2(b) - (p / a.hypot(b)).asin());
        let collisions = out["collisions"].as_array().unwrap();
        for (bend, other) in [(1, 0), (0, 1)] {
            let c = collisions
                .iter()
                .find(|c| c["flange"] == bend && c["done"] == json!([other]))
                .unwrap_or_else(|| panic!("no collision of bend {bend} after {other}: {out}"));
            assert_eq!(c["tool"], "punch", "{c}");
            assert_eq!(c["piece"], format!("flange {other}"), "{c}");
            let angle = c["angle"].as_f64().unwrap();
            assert!(
                angle >= first.to_degrees() - 1e-6 && angle <= 90.0 + 1e-9,
                "scale {k}: collision at {angle} degrees, first possible {}",
                first.to_degrees()
            );
        }
        let out = sequence(&mut s, k, &GOOSENECK);
        assert_eq!(out["feasible"], true, "{out}");
        assert_eq!(out["steps"].as_array().unwrap().len(), 2, "{out}");
    }
}

#[test]
fn bad_parameters_are_errors() {
    let k = 1.0;
    let dir = workdir("sheetmetal-bendsequence", "bad");
    let mut s = session_in(&dir);
    new_document(&mut s, k);
    channel(&mut s, k, 20.0);
    let die = scaled(&DIE, k);
    let punch = scaled(&STRAIGHT, k);
    for (params, why) in [
        (json!({"feature": "U", "die": die}), "punch"),
        (
            json!({"feature": "U", "punch": [[0, 0], [1, 0]], "die": die}),
            "at least 3",
        ),
        (
            json!({"feature": "U", "punch": [[0, 0], [1, "x"], [0, 1]], "die": die}),
            "at least 3",
        ),
        (
            json!({"feature": "U", "punch": [[0, 0], [1, 1], [1, 0], [0, 1]], "die": die}),
            "punch",
        ),
        (
            json!({"feature": "U", "punch": punch, "die": die, "clearance": 0}),
            "clearance",
        ),
        (
            json!({"feature": "U", "punch": punch, "die": die, "length": -1}),
            "length",
        ),
        (
            json!({"feature": "Base", "punch": punch, "die": die}),
            "sheet-metal",
        ),
        (
            json!({"feature": "Nowhere", "punch": punch, "die": die}),
            "Nowhere",
        ),
        (
            json!({"feature": "U", "punch": punch, "die": die, "order": [0]}),
            "unknown parameter",
        ),
    ] {
        let got = error(&mut s, "sheetMetal.bendSequence", params.clone());
        assert!(got.contains(why), "{params}: {got}");
    }
}
