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
//!   standing wall: an order that clears, the second step's least distance
//!   the wall's corner to the throat's flank, `(R + W − H)/√2 + p`.
//! - The sheet rests on the die (review 2026-10-28: the die counted every
//!   piece lying on it as a collision): a channel only 10 wide between its
//!   tangent lines, the other strip flat on the die, clears with a narrow
//!   punch; a tray with flanges on two edges that meet clears its first
//!   bend in either order (the other strip lies on the die along the
//!   bend), its second only with a punch shorter than the edge (the
//!   standing wall at the corner is in the punch's way, as on a press
//!   brake), while a Z as narrow (one wall up, one down) cannot be bent in
//!   either order: the made wall goes into the die.
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
    part(s, k, 40.0, &[(3, h, "up"), (1, h, "up")]);
}

/// A part named `U`: the base `w` × 30 and per flange (edge of the
/// rectangle from its bottom edge on, length, direction) a flange bent 90
/// degrees.
fn part(s: &mut Session, k: f64, w: f64, flanges: &[(usize, f64, &str)]) {
    run(s, "sketch.create", json!({"plane": "xy", "name": "Base"}));
    let made = run(
        s,
        "sketch.rect",
        json!({"sketch": "Base", "from": [0.0, 0.0], "to": [w * k, 30.0 * k]}),
    );
    let lines: Vec<String> = made["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap().to_string())
        .collect();
    let flanges: Vec<Value> = flanges
        .iter()
        .map(|(edge, h, direction)| {
            json!({"edge": lines[*edge], "length": mm(h * k), "angle": "90 deg",
                   "direction": direction})
        })
        .collect();
    built(
        s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "name": "U", "thickness": mm(k), "radius": mm(k),
               "kFactor": 0.5, "flanges": flanges}),
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
const NARROW: [[f64; 2]; 4] = [[-1.0, 0.0], [1.0, 0.0], [1.0, 100.0], [-1.0, 100.0]];
const DIE: [[f64; 2]; 4] = [[-15.0, -20.0], [15.0, -20.0], [15.0, 0.0], [-15.0, 0.0]];

fn sequence(s: &mut Session, k: f64, punch: &[[f64; 2]]) -> Value {
    let out = run(
        s,
        "sheetMetal.bendSequence",
        json!({"feature": "U", "punch": scaled(punch, k), "die": scaled(&DIE, k)}),
    );
    proven_below_least(&out, k);
    out
}

/// Every step's distance proven over the whole stroke is at least the
/// clearance and at most the least distance measured (within rounding: the
/// die's rest proof reckons from heights, not distances), for both tools.
fn proven_below_least(out: &Value, k: f64) {
    let clearance = out["clearance"].as_f64().unwrap();
    assert!((clearance - 1e-6 * k).abs() <= 1e-12 * k, "{out}");
    for step in out["steps"].as_array().unwrap() {
        for tool in [step, &step["die"]] {
            if tool["least"].is_null() {
                continue;
            }
            let least = tool["least"].as_f64().unwrap();
            let proven = tool["proven"].as_f64().unwrap();
            assert!(
                proven >= clearance && proven <= least + 1e-9 * k,
                "proven {proven}, least {least}: {step}"
            );
        }
    }
    assert!(
        out["unchecked"].as_str().unwrap().contains("own strip"),
        "{out}"
    );
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
        let steps = out["steps"].as_array().unwrap();
        assert_eq!(steps.len(), 2, "{out}");
        // The standing wall's inner top corner at the end of the stroke,
        // `((R + W − H)/√2, …)` in the bend's frame, against the throat's
        // flank at x = −p.
        let least = ((a - b) / SQRT_2 + p) * k;
        let got = steps[1]["least"].as_f64().unwrap();
        assert!(
            (got - least).abs() <= 1e-6 * k,
            "scale {k}: gooseneck least {got} against {least}: {out}"
        );
    }
}

#[test]
fn the_sheet_rests_on_the_die_but_does_not_go_into_it() {
    for k in SCALES {
        let dir = workdir("sheetmetal-bendsequence", &format!("rest-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        // A channel 10 wide: at each first bend the other strip lies flat
        // on the die, within its 15 either side of the bend.
        part(&mut s, k, 10.0, &[(3, 5.0, "up"), (1, 5.0, "up")]);
        let out = sequence(&mut s, k, &NARROW);
        assert_eq!(out["feasible"], true, "scale {k}: {out}");
        assert!(out["collisions"].as_array().unwrap().is_empty(), "{out}");
        let first = &out["steps"][0]["die"];
        let lowered = out["clearance"].as_f64().unwrap() + 1e-6 * k;
        // Resting: the lowered die's distance is what it was lowered by.
        assert!(
            (first["least"].as_f64().unwrap() - lowered).abs() <= 1e-9 * k,
            "scale {k}: {first}"
        );
        assert_eq!(first["at"], 0.0, "{first}");

        // A Z as narrow: the made wall stands in the die's way.
        new_document(&mut s, k);
        part(&mut s, k, 10.0, &[(3, 5.0, "up"), (1, 5.0, "down")]);
        let out = sequence(&mut s, k, &NARROW);
        assert_eq!(out["feasible"], false, "scale {k}: {out}");
        let collisions = out["collisions"].as_array().unwrap();
        for (bend, other) in [(1, 0), (0, 1)] {
            assert!(
                !collisions
                    .iter()
                    .any(|c| c["flange"] == bend && c["done"] == json!([])),
                "scale {k}: the first bend {bend} should clear: {out}"
            );
            let c = collisions
                .iter()
                .find(|c| c["flange"] == bend && c["done"] == json!([other]))
                .unwrap_or_else(|| panic!("no collision of bend {bend} after {other}: {out}"));
            assert_eq!(c["tool"], "die", "{c}");
            assert!(
                c["piece"]
                    .as_str()
                    .unwrap()
                    .contains(&format!("flange {other}")),
                "{c}"
            );
        }
    }
}

#[test]
fn a_tray_corner_clears_its_first_bend_and_its_second_with_a_short_punch() {
    for k in SCALES {
        let dir = workdir("sheetmetal-bendsequence", &format!("tray-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        // Flanges on the bottom and the right edge, meeting at (40, 0).
        part(&mut s, k, 40.0, &[(0, 10.0, "up"), (1, 10.0, "up")]);
        let out = sequence(&mut s, k, &NARROW);
        assert_eq!(out["feasible"], false, "scale {k}: {out}");
        let collisions = out["collisions"].as_array().unwrap();
        for (bend, other) in [(1, 0), (0, 1)] {
            assert!(
                !collisions
                    .iter()
                    .any(|c| c["flange"] == bend && c["done"] == json!([])),
                "scale {k}: the first bend {bend} should clear: {out}"
            );
            let c = collisions
                .iter()
                .find(|c| c["flange"] == bend && c["done"] == json!([other]))
                .unwrap_or_else(|| panic!("no collision of bend {bend} after {other}: {out}"));
            assert_eq!(c["tool"], "punch", "{c}");
            assert!(
                c["piece"]
                    .as_str()
                    .unwrap()
                    .contains(&format!("flange {other}")),
                "{c}"
            );
        }
        // Tools 26 long (shorter than either edge, centred on the bend)
        // keep clear of the wall standing beyond the bend's end.
        let out = run(
            &mut s,
            "sheetMetal.bendSequence",
            json!({"feature": "U", "punch": scaled(&NARROW, k), "die": scaled(&DIE, k),
                   "length": 26.0 * k}),
        );
        proven_below_least(&out, k);
        assert_eq!(out["feasible"], true, "scale {k}: {out}");
    }
}

/// A die wholly inside the sheet (a bar a quarter of the thickness thin,
/// 2 long, in the flats under the bend) meets no face of it, yet stands in
/// its material: a collision at the start of the stroke (review
/// 2026-10-28: only points of the pieces inside a tool were looked for).
#[test]
fn a_tool_wholly_inside_the_sheet_collides() {
    for k in SCALES {
        let dir = workdir("sheetmetal-bendsequence", &format!("inside-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        channel(&mut s, k, 20.0);
        let bar = [[-20.0, 0.25], [20.0, 0.25], [20.0, 0.5], [-20.0, 0.5]];
        let out = run(
            &mut s,
            "sheetMetal.bendSequence",
            json!({"feature": "U", "punch": scaled(&STRAIGHT, k), "die": scaled(&bar, k),
                   "length": 2.0 * k}),
        );
        assert_eq!(out["feasible"], false, "scale {k}: {out}");
        let collisions = out["collisions"].as_array().unwrap();
        for bend in [0, 1] {
            let c = collisions
                .iter()
                .find(|c| c["flange"] == bend && c["done"] == json!([]))
                .unwrap_or_else(|| panic!("scale {k}: no collision of bend {bend}: {out}"));
            assert_eq!(c["tool"], "die", "{c}");
            assert_eq!(c["angle"], 0.0, "{c}");
            assert_eq!(c["lower"], 0.0, "{c}");
        }
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
