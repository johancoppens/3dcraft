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
//!   bend), its second only with a punch shorter than the edge or with
//!   segmented tooling (the standing wall at the corner is in the way of
//!   the default tools, which reach past the part, as on a press brake;
//!   a saved `bendable` without `length` keeps that meaning), while a Z as narrow (one wall up, one down) cannot be bent in
//!   either order: the made wall goes into the die.
//! - Bad parameters are errors.
//! - Every length (the tools' points, `length`, `clearance`) is a number
//!   in the document's unit or a length expression, in a document in
//!   millimetres as in one in inches.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;

use common::checked::{built, error, new_document, run, session_in, workdir};
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::f64::consts::{PI, SQRT_2};

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
/// clearance and at most the least distance measured, for both tools.
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
                proven >= clearance && proven <= least,
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
        let first = 2.0 * (a.patan2(b) - (p / a.phypot(b)).pasin());
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
        // Resting: the lowered die lies the lowering away; what is proven
        // is at least the clearance and no more than that.
        let least = first["least"].as_f64().unwrap();
        assert!(
            least >= out["clearance"].as_f64().unwrap() && least <= lowered * (1.0 + 1e-9),
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
        // Segmented tooling, asked for: each bend's tools as long as the
        // bend (40), ending where it does, short of the wall standing
        // beyond it. Never the default: a tool longer than its bend on
        // the brake would meet that wall (the default fails closed).
        let out = run(
            &mut s,
            "sheetMetal.bendSequence",
            json!({"feature": "U", "punch": scaled(&NARROW, k), "die": scaled(&DIE, k),
                   "segmented": true}),
        );
        proven_below_least(&out, k);
        assert_eq!(out["feasible"], true, "scale {k}: {out}");
        assert!(
            out["unchecked"]
                .as_str()
                .unwrap()
                .contains("segmented tooling assumed"),
            "{out}"
        );
    }
}

/// A `bendable` assertion without `length`, saved and opened again,
/// keeps its meaning: the tools reach past the whole part (as they did
/// when `bendable` was made), so the tray's second bend meets the wall
/// standing at the corner, in either order: it fails. Only `segmented`
/// asks for tools as long as each bend, which clear it (review
/// 2026-10-29: per-bend lengths as the default changed what saved
/// assertions said).
#[test]
fn a_saved_bendable_without_length_keeps_its_meaning() {
    let k = 1.0;
    let dir = workdir("sheetmetal-bendsequence", "bendable-saved");
    let mut s = session_in(&dir);
    new_document(&mut s, k);
    part(&mut s, k, 40.0, &[(0, 10.0, "up"), (1, 10.0, "up")]);
    let (punch, die) = (scaled(&NARROW, k), scaled(&DIE, k));
    let whole = bendable(
        &mut s,
        "whole",
        json!({"of": "U", "punch": punch, "die": die, "is": true}),
    );
    assert_eq!(whole["status"], "failed", "{whole}");
    assert!(
        whole["detail"]["unchecked"]
            .as_str()
            .unwrap()
            .contains("one-piece tools"),
        "{whole}"
    );
    let segmented = bendable(
        &mut s,
        "segmented",
        json!({"of": "U", "punch": punch, "die": die, "segmented": true, "is": true}),
    );
    assert_eq!(segmented["status"], "passed", "{segmented}");
    run(&mut s, "file.save", json!({"path": "tray.3dcraft"}));
    run(&mut s, "file.open", json!({"path": "tray.3dcraft"}));
    let listed = run(&mut s, "assert.list", json!({"details": true}));
    let status = |name: &str| {
        listed["assertions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["name"] == name)
            .unwrap_or_else(|| panic!("no assertion {name}: {listed}"))["status"]
            .clone()
    };
    assert_eq!(status("whole"), "failed", "{listed}");
    assert_eq!(status("segmented"), "passed", "{listed}");
    // `segmented` is a yes or no.
    let bad = bendable(
        &mut s,
        "bad",
        json!({"of": "U", "punch": punch, "die": die, "segmented": 1, "is": true}),
    );
    assert_eq!(bad["status"], "error", "{bad}");
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

/// A part named `U` whose base is the closed polygon `points` (scaled by
/// `k`), a flange `h` high bent 90 degrees up on its side `edge`.
fn polygon_part(s: &mut Session, k: f64, points: &[[f64; 2]], edge: usize, h: f64) {
    run(s, "sketch.create", json!({"plane": "xy", "name": "Base"}));
    let made = run(
        s,
        "sketch.polyline",
        json!({"sketch": "Base", "closed": true, "points": scaled(points, k)}),
    );
    let line = made["lines"][edge].as_str().unwrap().to_string();
    built(
        s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "name": "U", "thickness": mm(k), "radius": mm(k),
               "kFactor": 0.5,
               "flanges": [{"edge": line, "length": mm(h * k), "angle": "90 deg"}]}),
    );
}

/// A flange on a side that does not run along the sketch's axes (review
/// 2026-10-29: the flats' hull was their box in the pattern's axes, whose
/// corners stood past a slanted bend line and below the die's top as soon
/// as the stroke began, so every such part was notProven): a triangle, a
/// rectangle turned 30 degrees and a hexagon, each one bend that clears
/// with the base lying on the die.
#[test]
fn a_flange_on_a_slanted_edge_is_proven_lying_on_the_die() {
    let (c, s30) = ((PI / 6.0).pcos(), (PI / 6.0).psin());
    let turned: Vec<[f64; 2]> = [[0.0, 0.0], [40.0, 0.0], [40.0, 30.0], [0.0, 30.0]]
        .iter()
        .map(|p| [c * p[0] - s30 * p[1], s30 * p[0] + c * p[1]])
        .collect();
    let hexagon: Vec<[f64; 2]> = (0..6)
        .map(|i| {
            let a = PI / 3.0 * f64::from(i);
            [20.0 * a.pcos(), 20.0 * a.psin()]
        })
        .collect();
    let shapes: [(&str, &[[f64; 2]]); 3] = [
        ("triangle", &[[0.0, 0.0], [40.0, 0.0], [20.0, 30.0]]),
        ("turned", &turned),
        ("hexagon", &hexagon),
    ];
    for k in SCALES {
        for (name, points) in shapes {
            let dir = workdir("sheetmetal-bendsequence", &format!("slanted-{name}-{k}"));
            let mut s = session_in(&dir);
            new_document(&mut s, k);
            polygon_part(&mut s, k, points, 1, 10.0);
            let out = sequence(&mut s, k, &STRAIGHT);
            assert_eq!(out["feasible"], true, "{name}, scale {k}: {out}");
            assert_eq!(out["order"], json!([0]), "{name}, scale {k}: {out}");
            assert!(
                out["collisions"].as_array().unwrap().is_empty(),
                "{name}, scale {k}: {out}"
            );
            // The base lies on the die: proven within the lowering.
            let die = &out["steps"][0]["die"];
            let lowered = out["clearance"].as_f64().unwrap() + 1e-6 * k;
            let least = die["least"].as_f64().unwrap();
            assert!(least <= lowered * (1.0 + 1e-9), "{name}, scale {k}: {die}");
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
        // A coordinate is a length expression (as for `bendable`): a name
        // that is not defined is named.
        (
            json!({"feature": "U", "punch": [[0, 0], [1, "nowhere"], [0, 1]], "die": die}),
            "nowhere",
        ),
        (
            json!({"feature": "U", "punch": [[0, 0], [1, "5 deg"], [0, 1]], "die": die}),
            "length",
        ),
        (
            json!({"feature": "U", "punch": [[0, 0], [1, true], [0, 1]], "die": die}),
            "punch",
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
            json!({"feature": "U", "punch": punch, "die": die, "length": "-1 mm"}),
            "length",
        ),
        (
            json!({"feature": "U", "punch": punch, "die": die, "length": [30, "2 deg"]}),
            "flange 1",
        ),
        (
            json!({"feature": "U", "punch": punch, "die": die, "clearance": "0 mm"}),
            "clearance",
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
            json!({"feature": "U", "punch": punch, "die": die, "orders": [0]}),
            "unknown parameter",
        ),
    ] {
        let got = error(&mut s, "sheetMetal.bendSequence", params.clone());
        assert!(got.contains(why), "{params}: {got}");
    }
}

/// Every length of the command is read as every length param is, a
/// number in the document's unit or a length expression, as `bendable`
/// reads it (2026-11-10: `"length": "60 mm"` was refused, "must be more
/// than 0", while `bendable` took it): the U channel with walls 20 high,
/// built in millimetres in a document in millimetres and in one in
/// inches, checked with the tools, `length` and `clearance` given in
/// millimetres, clears the same way in both, every length the same once
/// in one unit, and `bendable` given the same words agrees.
#[test]
fn every_length_is_a_length_expression_in_millimetres_and_in_inches() {
    let in_mm = |points: &[[f64; 2]]| {
        json!(
            points
                .iter()
                .map(|p| [mm(p[0]), mm(p[1])])
                .collect::<Vec<_>>()
        )
    };
    let mut leasts = Vec::new();
    for (units, per_mm) in [("mm", 1.0), ("in", 1.0 / 25.4)] {
        let dir = workdir("sheetmetal-bendsequence", &format!("units-{units}"));
        let mut s = session_in(&dir);
        // The same tolerance in both, a millionth of a millimetre.
        run(
            &mut s,
            "file.new",
            json!({"units": units, "tolerance": 1e-6 * per_mm}),
        );
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "xy", "name": "Base"}),
        );
        let made = run(
            &mut s,
            "sketch.rect",
            json!({"sketch": "Base", "from": [mm(0.0), mm(0.0)], "to": [mm(40.0), mm(30.0)]}),
        );
        let line = |i: usize| made["lines"][i].clone();
        built(
            &mut s,
            "feature.sheetMetal",
            json!({"sketch": "Base", "name": "U", "thickness": mm(1.0), "radius": mm(1.0),
                   "kFactor": 0.5, "flanges": [
                       {"edge": line(3), "length": mm(20.0), "angle": "90 deg", "direction": "up"},
                       {"edge": line(1), "length": mm(20.0), "angle": "90 deg", "direction": "up"}]}),
        );
        let tools = json!({"feature": "U", "punch": in_mm(&STRAIGHT), "die": in_mm(&DIE),
                           "length": "60 mm", "segmented": true, "clearance": "0.001 mm"});
        let out = run(&mut s, "sheetMetal.bendSequence", tools.clone());
        assert_eq!(out["feasible"], true, "{units}: {out}");
        assert_eq!(out["order"], json!([0, 1]), "{units}: {out}");
        let near = |got: &Value, want: f64| {
            let got = got.as_f64().unwrap();
            assert!(
                (got - want).abs() <= 1e-9 * want.abs(),
                "{units}: {got} against {want}: {out}"
            );
        };
        near(&out["clearance"], 0.001 * per_mm);
        let steps = out["steps"].as_array().unwrap();
        for step in steps {
            for segment in step["segments"].as_array().unwrap() {
                near(segment, 60.0 * per_mm);
            }
        }
        leasts.push(steps[1]["least"].as_f64().unwrap() / per_mm);
        // A length per flange, as words too.
        let mut per_flange = tools.clone();
        per_flange["length"] = json!(["60 mm", null]);
        let out = run(&mut s, "sheetMetal.bendSequence", per_flange);
        assert_eq!(out["feasible"], true, "{units}: {out}");
        // The measure, given the same words, says the same.
        let mut asked = tools;
        asked["of"] = json!("U");
        asked["is"] = json!(true);
        if let Some(map) = asked.as_object_mut() {
            map.remove("feature");
        }
        let check = bendable(&mut s, "same tools", asked);
        assert_eq!(check["status"], "passed", "{units}: {check}");
        assert_eq!(
            check["detail"]["parts"][0]["order"],
            json!([0, 1]),
            "{check}"
        );
    }
    assert!(
        (leasts[0] - leasts[1]).abs() <= 1e-6,
        "the same in millimetres: {leasts:?}"
    );
}

/// The assertion of the press-brake check, `bendable` (as an output too).
fn bendable(s: &mut Session, name: &str, params: Value) -> Value {
    let mut p = params;
    p["name"] = json!(name);
    p["measure"] = json!("bendable");
    run(s, "assert.add", p);
    let listed = run(s, "assert.list", json!({"details": true}));
    listed["assertions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["name"] == name)
        .cloned()
        .unwrap_or_else(|| panic!("no assertion {name}: {listed}"))
}

/// `bendable` agrees with the closed forms of the U channel at three
/// scales: walls 20 high are bendable with the straight punch, walls 40
/// high collide in either order (no order, for certain: the command says
/// `infeasibleProven`), and clear the gooseneck; the punch's points are
/// looked up in a tool table; a search cut short by its work is not
/// proven, never passed, and its output unknown.
#[test]
fn bendable_follows_the_closed_forms_of_the_u_channel() {
    for k in SCALES {
        let dir = workdir("sheetmetal-bendsequence", &format!("bendable-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        channel(&mut s, k, 20.0);
        let (punch, die) = (scaled(&STRAIGHT, k), scaled(&DIE, k));
        let short = bendable(
            &mut s,
            "short",
            json!({"of": "U", "punch": punch, "die": die, "is": true}),
        );
        assert_eq!(short["status"], "passed", "scale {k}: {short}");
        assert_eq!(
            short["detail"]["parts"][0]["order"],
            json!([0, 1]),
            "{short}"
        );
        assert_eq!(
            short["detail"]["parts"][0]["verdict"], "bendable",
            "{short}"
        );

        // The punch from a tool table: its half width and height by row.
        run(
            &mut s,
            "table.set",
            json!({"name": "punches", "columns": ["tool", {"name": "half", "unit": "mm"}, {"name": "h", "unit": "mm"}],
                   "rows": [["straight", 5.0 * k, 100.0 * k], ["wide", 30.0 * k, 100.0 * k]]}),
        );
        run(&mut s, "variable.set", json!({"name": "tool", "expr": "1"}));
        let from_table = json!([
            ["-punches[tool].half", "0 mm"],
            ["punches[tool].half", "0 mm"],
            ["punches[tool].half", "punches[tool].h"],
            ["-punches[tool].half", "punches[tool].h"]
        ]);
        let tabled = bendable(
            &mut s,
            "tabled",
            json!({"of": "U", "punch": from_table, "die": die, "is": true}),
        );
        assert_eq!(tabled["status"], "passed", "scale {k}: {tabled}");
        // A punch 60 wide fills the channel between the walls once one
        // stands: the second bend meets it.
        run(&mut s, "variable.set", json!({"name": "tool", "expr": "2"}));
        let tabled = bendable(
            &mut s,
            "tabled wide",
            json!({"of": "U", "punch": from_table, "die": die, "is": true}),
        );
        assert_eq!(tabled["status"], "failed", "scale {k}: {tabled}");
        // Renaming the table renames its lookups inside the measure.
        run(
            &mut s,
            "table.rename",
            json!({"name": "punches", "to": "tools"}),
        );
        let listed = run(&mut s, "assert.list", json!({}));
        let stored = listed["assertions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["name"] == "tabled")
            .unwrap()["measure"]
            .to_string();
        assert!(stored.contains("tools[tool].half"), "{stored}");
        assert!(!stored.contains("punches"), "{stored}");

        // Out of work: not proven, and an output of it unknown.
        let cut = bendable(
            &mut s,
            "cut short",
            json!({"of": "U", "punch": punch, "die": die, "maxWork": 1, "is": true}),
        );
        assert_eq!(cut["status"], "notProven", "scale {k}: {cut}");
        assert!(
            cut["message"].as_str().unwrap().contains("distances"),
            "{cut}"
        );
        run(
            &mut s,
            "variable.measure",
            json!({"name": "canBend", "measure": "bendable", "of": "U", "punch": punch, "die": die, "maxWork": 1}),
        );
        let listed = run(&mut s, "output.list", json!({}));
        assert_eq!(listed["outputs"][0]["status"], "unknown", "{listed}");

        // Walls 40 high: no order, for certain.
        new_document(&mut s, k);
        channel(&mut s, k, 40.0);
        let tall = bendable(
            &mut s,
            "tall",
            json!({"of": "U", "punch": punch, "die": die, "is": true}),
        );
        assert_eq!(tall["status"], "failed", "scale {k}: {tall}");
        assert_eq!(tall["detail"]["parts"][0]["verdict"], "collides", "{tall}");
        assert!(
            tall["message"]
                .as_str()
                .unwrap()
                .contains("punch meets flange"),
            "{tall}"
        );
        let out = sequence(&mut s, k, &STRAIGHT);
        assert_eq!(out["infeasibleProven"], true, "{out}");
        assert!(
            out["collisions"]
                .as_array()
                .unwrap()
                .iter()
                .all(|c| c["certain"] == true),
            "{out}"
        );
        let goose = bendable(
            &mut s,
            "gooseneck",
            json!({"of": "U", "punch": scaled(&GOOSENECK, k), "die": die, "is": true}),
        );
        assert_eq!(goose["status"], "passed", "scale {k}: {goose}");
        // As an output: 1 for yes.
        run(
            &mut s,
            "variable.measure",
            json!({"name": "canBend", "measure": "bendable", "of": "U", "punch": scaled(&GOOSENECK, k), "die": die}),
        );
        let listed = run(&mut s, "output.list", json!({}));
        assert_eq!(listed["outputs"][0]["value"], 1.0, "{listed}");
    }
}

#[test]
fn bendable_with_bad_parameters_is_an_error_not_a_crash() {
    let k = 1.0;
    let dir = workdir("sheetmetal-bendsequence", "bendable-bad");
    let mut s = session_in(&dir);
    new_document(&mut s, k);
    channel(&mut s, k, 20.0);
    let (punch, die) = (scaled(&STRAIGHT, k), scaled(&DIE, k));
    for (params, why) in [
        (json!({"of": "U", "die": die}), "punch"),
        (
            json!({"of": "U", "punch": [[0, 0], [1, 0]], "die": die}),
            "at least 3",
        ),
        (
            json!({"of": "U", "punch": [[0, 0], [1, "nowhere"], [0, 1]], "die": die}),
            "nowhere",
        ),
        (
            json!({"of": "U", "punch": [[0, 0], [1, "5 deg"], [0, 1]], "die": die}),
            "length",
        ),
        (
            json!({"of": "U", "punch": [[0, 0], [1, 1], [1, 0], [0, 1]], "die": die}),
            "punch",
        ),
        (
            json!({"of": "U", "punch": punch, "die": die, "clearance": 0}),
            "clearance",
        ),
        (
            json!({"of": "U", "punch": punch, "die": die, "length": "-1 mm"}),
            "length",
        ),
        (
            json!({"of": "U", "punch": punch, "die": die, "maxWork": 0}),
            "maxWork",
        ),
        (
            json!({"of": "U", "punch": punch, "die": die, "maxWork": 1e9}),
            "maxWork",
        ),
        (json!({"of": "Base", "punch": punch, "die": die}), "Base"),
        (
            json!({"of": "Nowhere", "punch": punch, "die": die}),
            "Nowhere",
        ),
    ] {
        let got = bendable(&mut s, "bad", {
            let mut p = params.clone();
            p["is"] = json!(true);
            p
        });
        assert_eq!(got["status"], "error", "{params}: {got}");
        let message = got["message"].as_str().unwrap();
        assert!(message.contains(why), "{params}: {message}");
        assert!(!message.contains("internal error"), "{params}: {message}");
        run(&mut s, "assert.delete", json!({"name": "bad"}));
    }
    // A bound where yes or no is asked, a key it does not take: refused.
    for params in [
        json!({"name": "x", "measure": "bendable", "of": "U", "punch": punch, "die": die, "min": 1}),
        // `order` is the measure's since 2026-10-31 (an order given to
        // check); a key it does not take is still refused.
        json!({"name": "x", "measure": "bendable", "of": "U", "punch": punch, "die": die, "sequence": [0], "is": true}),
    ] {
        assert!(s.execute("assert.add", &params).is_err(), "{params}");
    }
}

/// What `bendable` costs on a part of four flanges (a tray 60 × 40 with
/// walls 10 high on every edge, tools 30 long): the search's work and
/// time, printed for the design note; an assertion measures it again
/// only when what it depends on changes.
#[test]
fn the_cost_of_bendable_on_a_tray_of_four_flanges() {
    let k = 1.0;
    let dir = workdir("sheetmetal-bendsequence", "bendable-cost");
    let mut s = session_in(&dir);
    new_document(&mut s, k);
    part(
        &mut s,
        k,
        60.0,
        &[
            (0, 10.0, "up"),
            (1, 10.0, "up"),
            (2, 10.0, "up"),
            (3, 10.0, "up"),
        ],
    );
    for length in [30.0, 200.0] {
        let started = std::time::Instant::now();
        let got = bendable(
            &mut s,
            &format!("tray {length}"),
            json!({"of": "U", "punch": scaled(&NARROW, k), "die": scaled(&DIE, k), "length": length, "is": true}),
        );
        let ms = started.elapsed().as_millis();
        eprintln!(
            "bendable on a four-flange tray, tools {length} long: {} in {ms} ms, work {}: {}",
            got["status"], got["detail"]["work"], got["message"]
        );
        assert!(
            ["passed", "failed", "notProven"].contains(&got["status"].as_str().unwrap()),
            "{got}"
        );
        assert!(got["detail"]["work"].as_u64().unwrap() <= 20_000, "{got}");
    }
}

/// The gooseneck mirrored (`x → −x`), its throat open the other way: as
/// the tools stand, the standing wall of the U channel 40 high meets it in
/// either order (both walls lie on the same side of their bend's frame),
/// but a press brake lets the part be turned end for end between bends,
/// which for tools extruded symmetrically along the bend is the tools
/// mirrored in `x`. So the part is bendable, the second step turned, with
/// the closed form of the gooseneck as given (review 2026-10-29: the
/// measure said no, for certain).
#[test]
fn a_mirrored_gooseneck_clears_with_the_part_turned_end_for_end() {
    let mirrored: Vec<[f64; 2]> = GOOSENECK.iter().map(|p| [-p[0], p[1]]).collect();
    for k in SCALES {
        let dir = workdir("sheetmetal-bendsequence", &format!("turned-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        channel(&mut s, k, 40.0);
        let goose = bendable(
            &mut s,
            "mirrored gooseneck",
            json!({"of": "U", "punch": scaled(&mirrored, k), "die": scaled(&DIE, k), "is": true}),
        );
        assert_ne!(goose["status"], "failed", "scale {k}: {goose}");
        assert_eq!(goose["status"], "passed", "scale {k}: {goose}");
        let out = sequence(&mut s, k, &mirrored);
        assert_eq!(out["feasible"], true, "{out}");
        let steps = out["steps"].as_array().unwrap();
        assert_eq!(steps.len(), 2, "{out}");
        assert_eq!(steps[1]["turned"], true, "{out}");
        let (a, b, p) = (41.0_f64, 40.0_f64, 5.0_f64);
        let least = ((a - b) / SQRT_2 + p) * k;
        let got = steps[1]["least"].as_f64().unwrap();
        assert!(
            (got - least).abs() <= 1e-6 * k,
            "scale {k}: turned gooseneck least {got} against {least}: {out}"
        );
        // A step that clears turned is not a collision: none is listed.
        assert_eq!(out["collisions"], json!([]), "scale {k}: {out}");
        // Mirrored with its points reversed (the same polygon, as the
        // check mirrors it): the same result.
        let reversed: Vec<[f64; 2]> = mirrored.iter().rev().copied().collect();
        let again = sequence(&mut s, k, &reversed);
        assert_eq!(again["steps"][1]["turned"], true, "{again}");
        // The gooseneck as given needs no turn.
        let out = sequence(&mut s, k, &GOOSENECK);
        let steps = out["steps"].as_array().unwrap();
        assert!(steps.iter().all(|st| st["turned"] == false), "{out}");
        // A symmetric punch is never tried turned.
        let out = sequence(&mut s, k, &STRAIGHT);
        assert_eq!(out["feasible"], false, "{out}");
        let collisions = out["collisions"].as_array().unwrap();
        assert!(!collisions.is_empty(), "{out}");
        assert!(collisions.iter().all(|c| c["turned"] == false), "{out}");
    }
}

/// A tray 180 × 160 with walls 34 high on its four edges and lips 8 wide
/// on two of them: six bends, a gooseneck. The whole search (no order
/// clears: the lips close over the punch) measures some 33 000 distances,
/// more than the old default cap of 20000, so with the default of 1 000 000
/// it now ends with its answer (finding 20, 2026-11-05: a chassis of 7
/// bends ran out at 20000; a box of 7 bends segmented takes some 676 000
/// as a whole search, which the default holds). A cap
/// too small says about how much the whole search would take, and that
/// estimate is not below what it took.
#[test]
fn a_search_over_six_bends_fits_the_default_work_and_a_short_one_says_how_much_more() {
    let dir = workdir("sheetmetal-bendsequence", "six-bends");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "name": "Base"}),
    );
    let made = run(
        &mut s,
        "sketch.rect",
        json!({"sketch": "Base", "from": [0.0, 0.0], "to": [180.0, 160.0]}),
    );
    let lines: Vec<String> = made["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap().to_string())
        .collect();
    let mut flanges: Vec<Value> = lines
        .iter()
        .map(|edge| {
            json!({"edge": edge, "length": "34 mm", "angle": "90 deg",
                   "insetStart": "2 mm", "insetEnd": "2 mm"})
        })
        .collect();
    for wall in 0..2 {
        flanges.push(json!({"tip": wall, "length": "8 mm", "angle": "90 deg",
                            "insetStart": "12 mm", "insetEnd": "12 mm"}));
    }
    built(
        &mut s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "name": "Tray", "thickness": "1 mm", "radius": "1 mm",
               "kFactor": 0.4, "flanges": flanges}),
    );
    let params =
        json!({"feature": "Tray", "punch": scaled(&GOOSENECK, 1.0), "die": scaled(&DIE, 1.0)});
    let whole = run(&mut s, "sheetMetal.bendSequence", params.clone());
    let work = whole["work"].as_u64().unwrap();
    assert!(work > 20_000, "{work}");
    let mut short = params;
    short["maxWork"] = json!(5000);
    let got = error(&mut s, "sheetMetal.bendSequence", short);
    assert!(
        got.contains("more than 5000") && got.contains("maxWork"),
        "{got}"
    );
    let estimate: f64 = got
        .split("the whole search may need up to about ")
        .nth(1)
        .and_then(|rest| rest.split(';').next())
        .and_then(|n| n.trim().parse().ok())
        .unwrap_or_else(|| panic!("no estimate: {got}"));
    assert!(estimate >= work as f64, "{estimate} < {work}: {got}");
}
