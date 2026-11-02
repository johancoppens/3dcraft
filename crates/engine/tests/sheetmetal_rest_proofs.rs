//! The press brake's rest proof where material of the bend's parent lies
//! past the bend line beside the bend's ends (docs/design/sheetmetal.md ›
//! *Bend sequence* › *Past the bend line*), at 1e-3, 1 and 1e6. As the
//! leg rises both legs turn half the stroke about the bend's axis, so
//! parent material past the tangent line goes down into the die's
//! opening while the rest of the parent lies on the die's shoulder. The
//! proof of the whole piece's lowest point could not hold that (the
//! stroke's first 1/1024 was left not proven); it is proven now by each
//! piece cut at the tangent line, each half kept apart from each convex
//! piece of the die along a direction of its own:
//!
//! - a tray closed at its corner (overlap) bent with one-piece tools that
//!   reach past the bend's end, where the other wall's extension lies
//!   past the bend line: its first bends clear in either order; with a
//!   die whose opening is 1 either side the covered wall's extension
//!   (`R − g` past the line) still clears and the covering wall's (`R +
//!   T` past it) goes into the die's shoulder, a certain collision;
//! - a rack panel with ears past the bend line beside an inset flange:
//!   bendable over a V die whose opening holds the ears, a certain
//!   collision with the base over one narrower than their reach.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{built, new_document, run, session_in, workdir};
use craft3d_engine::Session;
use serde_json::{Value, json};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn mm(v: f64) -> String {
    format!("{v} mm")
}

fn scaled(points: &[[f64; 2]], k: f64) -> Value {
    json!(
        points
            .iter()
            .map(|p| [p[0] * k, p[1] * k])
            .collect::<Vec<_>>()
    )
}

const NARROW: [[f64; 2]; 4] = [[-1.0, 0.0], [1.0, 0.0], [1.0, 100.0], [-1.0, 100.0]];
/// A V die 40 wide, its opening 16 wide and 8 deep.
const WIDE_V: [[f64; 2]; 7] = [
    [-20.0, -20.0],
    [20.0, -20.0],
    [20.0, 0.0],
    [8.0, 0.0],
    [0.0, -8.0],
    [-8.0, 0.0],
    [-20.0, 0.0],
];
/// A V die whose opening is 1 either side of the bend.
const NARROW_V: [[f64; 2]; 7] = [
    [-20.0, -20.0],
    [20.0, -20.0],
    [20.0, 0.0],
    [1.0, 0.0],
    [0.0, -1.0],
    [-1.0, 0.0],
    [-20.0, 0.0],
];

fn sequence(s: &mut Session, k: f64, die: &[[f64; 2]], extra: Value) -> Value {
    let mut params = json!({"feature": "P", "punch": scaled(&NARROW, k), "die": scaled(die, k)});
    if let (Some(p), Some(e)) = (params.as_object_mut(), extra.as_object()) {
        for (key, v) in e {
            p.insert(key.clone(), v.clone());
        }
    }
    let out = run(s, "sheetMetal.bendSequence", params);
    // Every step's distance proven over the whole stroke is at least the
    // clearance and at most the least measured.
    let clearance = out["clearance"].as_f64().unwrap();
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
    out
}

/// The failures of the first bend (nothing made before it).
fn first_failures(out: &Value) -> Vec<Value> {
    out["collisions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["done"] == json!([]))
        .cloned()
        .collect()
}

/// A tray 40 × 30 with walls 10 high on its bottom and right edges, the
/// corner between them closed by overlap (gap 0.25) with a round relief
/// of 3: the right wall's flat runs on past its bend's end down to the
/// bottom wall, past the bottom bend's tangent line by about `R + T`.
fn tray(s: &mut Session, k: f64) {
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
    let flanges: Vec<Value> = [0, 1]
        .iter()
        .map(|&e| json!({"edge": lines[e], "length": mm(10.0 * k), "angle": "90 deg"}))
        .collect();
    built(
        s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "name": "P", "thickness": mm(k), "radius": mm(k),
               "kFactor": 0.5, "relief": {"shape": "round", "size": mm(3.0 * k)},
               "corner": {"kind": "overlap", "gap": mm(0.25 * k)}, "flanges": flanges}),
    );
}

#[test]
fn a_closed_corners_extension_past_the_bend_line_is_proven_in_the_die_opening() {
    for k in SCALES {
        let dir = workdir("sheetmetal-rest-proofs", &format!("tray-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        tray(&mut s, k);
        // One-piece tools reaching past the part: the second bend meets
        // the standing wall (the box corner), but each first bend clears,
        // its whole stroke proven, also its first 1/1024.
        let out = sequence(&mut s, k, &WIDE_V, json!({}));
        let first = first_failures(&out);
        assert!(first.is_empty(), "scale {k}: {first:?}\n{out}");
        assert_eq!(out["feasible"], false, "scale {k}: {out}");
        for c in out["collisions"].as_array().unwrap() {
            assert_eq!(c["tool"], "punch", "scale {k}: {c}");
            assert_eq!(c["certain"], true, "scale {k}: {c}");
        }
        // Each first bend alone, as an order given, clears.
        for b in [0, 1] {
            let out = sequence(&mut s, k, &WIDE_V, json!({"order": [b, 1 - b]}));
            let step = &out["steps"][0];
            assert_eq!(step["flange"], b, "scale {k}: {out}");
            assert!(first_failures(&out).is_empty(), "scale {k}: {out}");
        }
        // With an opening only 1 either side: the bottom wall covers the
        // corner (overlap), so the right wall's extension runs past the
        // bottom bend's line only to the bottom wall's inner skin less the
        // gap, `R − g` = 0.75, inside the opening, and the bottom bend
        // clears first; the bottom wall's own extension runs past the
        // right bend's line to the right wall's outer skin, `R + T` = 2,
        // over the shoulder, and goes into it as the base turns: the right
        // bend first fails for certain, naming the bottom wall.
        let out = sequence(&mut s, k, &NARROW_V, json!({}));
        assert_eq!(out["feasible"], false, "scale {k}: {out}");
        assert_eq!(out["infeasibleProven"], true, "scale {k}: {out}");
        let first = first_failures(&out);
        assert!(
            !first.iter().any(|c| c["flange"] == 0),
            "scale {k}: the bottom bend first should clear: {out}"
        );
        let c = first
            .iter()
            .find(|c| c["flange"] == 1)
            .unwrap_or_else(|| panic!("scale {k}: the right bend first should fail: {out}"));
        assert_eq!(c["tool"], "die", "scale {k}: {c}");
        assert_eq!(c["certain"], true, "scale {k}: {c}");
        assert_eq!(c["piece"], "flange 0", "scale {k}: {c}");
    }
}

/// A rack panel: a base 60 × 35 whose bottom edge runs between two ears
/// 8 wide reaching 5 below it; the flange, 10 long, sits on the edge
/// between them inset 2 from either ear.
fn rack_panel(s: &mut Session, k: f64) {
    let points = [
        [-10.0, -5.0],
        [-2.0, -5.0],
        [-2.0, 0.0],
        [42.0, 0.0],
        [42.0, -5.0],
        [50.0, -5.0],
        [50.0, 30.0],
        [-10.0, 30.0],
    ];
    run(s, "sketch.create", json!({"plane": "xy", "name": "Base"}));
    let made = run(
        s,
        "sketch.polyline",
        json!({"sketch": "Base", "closed": true, "points": scaled(&points, k)}),
    );
    let line = made["lines"][2].as_str().unwrap().to_string();
    built(
        s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "name": "P", "thickness": mm(k), "radius": mm(k),
               "kFactor": 0.5,
               "flanges": [{"edge": line, "length": mm(10.0 * k), "angle": "90 deg",
                            "insetStart": mm(2.0 * k), "insetEnd": mm(2.0 * k)}]}),
    );
}

#[test]
fn ears_past_the_bend_line_are_proven_in_the_die_opening() {
    for k in SCALES {
        let dir = workdir("sheetmetal-rest-proofs", &format!("ears-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        rack_panel(&mut s, k);
        let out = sequence(&mut s, k, &WIDE_V, json!({}));
        assert_eq!(out["feasible"], true, "scale {k}: {out}");
        assert!(
            out["collisions"].as_array().unwrap().is_empty(),
            "scale {k}: {out}"
        );
        // The panel lies on the die at the stroke's start: the least
        // distance to the die lowered by the clearance and the tolerance
        // is that lowering.
        let die = &out["steps"][0]["die"];
        let lowered = out["clearance"].as_f64().unwrap() + 1e-6 * k;
        let least = die["least"].as_f64().unwrap();
        assert!(least <= lowered * (1.0 + 1e-9), "scale {k}: {die}");

        // The ears reach 5 past the bend line, over a shoulder 1 from it:
        // they go into the die as the panel turns, for certain.
        let out = sequence(&mut s, k, &NARROW_V, json!({}));
        assert_eq!(out["feasible"], false, "scale {k}: {out}");
        assert_eq!(out["infeasibleProven"], true, "scale {k}: {out}");
        let c = &out["collisions"][0];
        assert_eq!(c["tool"], "die", "scale {k}: {c}");
        assert_eq!(c["piece"], "the base", "scale {k}: {c}");
        assert_eq!(c["certain"], true, "scale {k}: {c}");
    }
}
