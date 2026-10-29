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
    let (c, s30) = ((PI / 6.0).cos(), (PI / 6.0).sin());
    let turned: Vec<[f64; 2]> = [[0.0, 0.0], [40.0, 0.0], [40.0, 30.0], [0.0, 30.0]]
        .iter()
        .map(|p| [c * p[0] - s30 * p[1], s30 * p[0] + c * p[1]])
        .collect();
    let hexagon: Vec<[f64; 2]> = (0..6)
        .map(|i| {
            let a = PI / 3.0 * f64::from(i);
            [20.0 * a.cos(), 20.0 * a.sin()]
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
        json!({"name": "x", "measure": "bendable", "of": "U", "punch": punch, "die": die, "order": [0], "is": true}),
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
        // The gooseneck as given needs no turn.
        let out = sequence(&mut s, k, &GOOSENECK);
        assert_eq!(out["steps"][1]["turned"], false, "{out}");
    }
}
