//! A hem pressed flat on the press brake (docs/design/sheetmetal.md ›
//! *Bend sequence* › *A hem pressed flat*), at 1e-3, 1 and 1e6: an L
//! (a base 8 × 30, a wall 20 high up on its right edge, R = T = 1, K = ½)
//! with a hem 8 long bent 180 degrees up on the wall's tip, towards the
//! base. The hem is two strokes: an air bend to 120 degrees with the
//! punch and die, then pressed flat between the flat die under the wall
//! and a pad `2P` wide standing on the hem's flat.
//!
//! - Hem first: the pad meets nothing but the parent lying flat, its
//!   least distance at the end of the stroke the hem's flat's top less
//!   the wall's top, `2(R + T) − T = 2R + T`; the wall then clears.
//! - Wall first: the base stands at `L + R` from the hem's axis (the
//!   base's inside face is `R` short of the wall's flat), so the pad's
//!   side keeps `L + R − P` from it, at every angle (the pad moves only
//!   up and down): 1.5 with `P` = 19.5; with `P` = 25 the pad meets the
//!   base for certain.
//! - The steps list `pre-bend` and `flatten`; bad `hem` objects are
//!   errors.

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
const DIE: [[f64; 2]; 4] = [[-15.0, -20.0], [15.0, -20.0], [15.0, 0.0], [-15.0, 0.0]];

fn pad(half: f64, k: f64) -> Value {
    scaled(&[[-half, 0.0], [half, 0.0], [half, 30.0], [-half, 30.0]], k)
}

/// The L with the hem: flange 0 the wall, flange 1 the hem.
fn hemmed(s: &mut Session, k: f64) {
    run(s, "sketch.create", json!({"plane": "xy", "name": "Base"}));
    let made = run(
        s,
        "sketch.rect",
        json!({"sketch": "Base", "from": [0.0, 0.0], "to": [8.0 * k, 30.0 * k]}),
    );
    let right = made["lines"][1].as_str().unwrap().to_string();
    built(
        s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "name": "L", "thickness": mm(k), "radius": mm(k),
               "kFactor": 0.5,
               "flanges": [
                   {"edge": right, "length": mm(20.0 * k), "angle": "90 deg"},
                   {"tip": 0, "length": mm(8.0 * k), "angle": "180 deg"}]}),
    );
}

fn sequence(s: &mut Session, k: f64, half: f64, order: Value) -> Value {
    let out = run(
        s,
        "sheetMetal.bendSequence",
        json!({"feature": "L", "punch": scaled(&NARROW, k), "die": scaled(&DIE, k),
               "order": order,
               "hem": {"flanges": [1], "preBend": 120, "pad": pad(half, k)}}),
    );
    let clearance = out["clearance"].as_f64().unwrap();
    for step in out["steps"].as_array().unwrap() {
        for tool in ["pad", "die"] {
            let t = if step["stage"] == "flatten" || tool == "die" {
                &step[tool]
            } else {
                step
            };
            if t.is_null() || t["least"].is_null() {
                continue;
            }
            let least = t["least"].as_f64().unwrap();
            let proven = t["proven"].as_f64().unwrap();
            assert!(
                proven >= clearance && proven <= least,
                "proven {proven}, least {least}: {step}"
            );
        }
    }
    out
}

#[test]
fn a_hem_pressed_first_then_the_wall() {
    for k in SCALES {
        let dir = workdir("sheetmetal-hems", &format!("hem-first-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        hemmed(&mut s, k);
        let tol = 1e-6 * k;
        let out = sequence(&mut s, k, 19.5, json!([1, 0]));
        assert_eq!(out["feasible"], true, "scale {k}: {out}");
        let steps = out["steps"].as_array().unwrap();
        assert_eq!(steps.len(), 3, "scale {k}: {out}");
        assert_eq!(steps[0]["stage"], "pre-bend", "{out}");
        assert_eq!(steps[1]["stage"], "flatten", "{out}");
        assert!(steps[2]["stage"].is_null(), "{out}");
        assert_eq!(steps[2]["flange"], 0, "{out}");
        // The pad: least at the end of the stroke, `2R + T` above the
        // wall's top skin.
        let pad = &steps[1]["pad"];
        let least = pad["least"].as_f64().unwrap();
        let exact = 3.0 * k;
        assert!(
            least <= exact + 1e-9 * k && least >= exact - 4.0 * tol,
            "scale {k}: pad {least} against {exact}: {pad}"
        );
        assert!((pad["at"].as_f64().unwrap() - 180.0).abs() < 1e-9, "{pad}");
        // The flat die: the wall lies on it, the lowering apart.
        let die = steps[1]["die"]["least"].as_f64().unwrap();
        let lowered = out["clearance"].as_f64().unwrap() + tol;
        assert!((die - lowered).abs() <= 1e-9 * lowered, "scale {k}: {die}");
        assert!(
            out["unchecked"]
                .as_str()
                .unwrap()
                .contains("hems pressed flat (1)"),
            "{out}"
        );

        // The search finds that order.
        let out = sequence(&mut s, k, 19.5, Value::Null);
        assert_eq!(out["feasible"], true, "scale {k}: {out}");
        assert_eq!(out["order"], json!([1, 0]), "scale {k}: {out}");
    }
}

#[test]
fn a_hem_pressed_after_the_wall_keeps_the_pad_off_the_base() {
    for k in SCALES {
        let dir = workdir("sheetmetal-hems", &format!("wall-first-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        hemmed(&mut s, k);
        let tol = 1e-6 * k;
        let out = sequence(&mut s, k, 19.5, json!([0, 1]));
        assert_eq!(out["feasible"], true, "scale {k}: {out}");
        let flatten = &out["steps"][2];
        assert_eq!(flatten["stage"], "flatten", "{out}");
        let least = flatten["pad"]["least"].as_f64().unwrap();
        let exact = (20.0 + 1.0 - 19.5) * k;
        assert!(
            least <= exact + 1e-9 * k && least >= exact - 4.0 * tol,
            "scale {k}: pad {least} against {exact}: {flatten}"
        );
        // A pad 50 wide reaches past the base's inside face: for certain.
        let out = sequence(&mut s, k, 25.0, json!([0, 1]));
        assert_eq!(out["feasible"], false, "scale {k}: {out}");
        assert_eq!(out["infeasibleProven"], true, "scale {k}: {out}");
        let c = out["collisions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["flange"] == 1)
            .cloned()
            .unwrap_or_else(|| panic!("scale {k}: {out}"));
        assert_eq!(c["tool"], "pad", "{c}");
        assert_eq!(c["piece"], "the base", "{c}");
        assert_eq!(c["certain"], true, "{c}");
        // Hem first, the same pad clears.
        let out = sequence(&mut s, k, 25.0, json!([1, 0]));
        assert_eq!(out["feasible"], true, "scale {k}: {out}");
    }
}

#[test]
fn bad_hems_are_errors() {
    let k = 1.0;
    let dir = workdir("sheetmetal-hems", "bad");
    let mut s = session_in(&dir);
    new_document(&mut s, k);
    hemmed(&mut s, k);
    let ok_pad = pad(5.0, k);
    for (hem, why) in [
        (
            json!({"flanges": [0], "preBend": 120, "pad": ok_pad}),
            "bent 90",
        ),
        (
            json!({"flanges": [7], "preBend": 120, "pad": ok_pad}),
            "index",
        ),
        (
            json!({"flanges": [1], "preBend": 180, "pad": ok_pad}),
            "preBend",
        ),
        (
            json!({"flanges": [1], "preBend": "x", "pad": ok_pad}),
            "preBend",
        ),
        (
            json!({"flanges": [], "preBend": 120, "pad": ok_pad}),
            "flanges",
        ),
        (json!({"flanges": [1], "preBend": 120}), "pad"),
        (
            json!({"flanges": [1], "preBend": 120, "pad": [[0, 0], [1, 1], [1, 0], [0, 1]]}),
            "hem.pad",
        ),
        (
            json!({"flanges": [1], "preBend": 120, "pad": ok_pad, "die": 1}),
            "no `die`",
        ),
        (json!(3), "`hem` is"),
    ] {
        let got = error(
            &mut s,
            "sheetMetal.bendSequence",
            json!({"feature": "L", "punch": scaled(&NARROW, k), "die": scaled(&DIE, k),
                   "hem": hem.clone()}),
        );
        assert!(got.contains(why), "{hem}: {got}");
    }
    // A hem given in a stroke with another flange.
    let got = error(
        &mut s,
        "sheetMetal.bendSequence",
        json!({"feature": "L", "punch": scaled(&NARROW, k), "die": scaled(&DIE, k),
               "segmented": true, "order": [[0, 1]],
               "hem": {"flanges": [1], "preBend": 120, "pad": ok_pad}}),
    );
    assert!(
        got.contains("one line") || got.contains("stroke of its own"),
        "{got}"
    );
}

/// A hem whose flat's far side is a spline: the pad would stand on the
/// spline's control points, above the flat (an upper bound of its highest
/// point, the wrong way for a clearance): refused by reason (review of
/// 47275cf1; before, the flatten stroke was checked with the pad too
/// high).
#[test]
fn a_hem_with_a_spline_outline_is_not_pressed_flat() {
    for k in SCALES {
        let dir = workdir("sheetmetal-hems", &format!("spline-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "xy", "name": "Base"}),
        );
        let made = run(
            &mut s,
            "sketch.rect",
            json!({"sketch": "Base", "from": [0.0, 0.0], "to": [8.0 * k, 30.0 * k]}),
        );
        let right = made["lines"][1].as_str().unwrap().to_string();
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "xy", "name": "Hem"}),
        );
        run(
            &mut s,
            "sketch.polyline",
            json!({"sketch": "Hem", "points": scaled(&[[14.0, 30.0], [8.0, 30.0], [8.0, 0.0], [14.0, 0.0]], k)}),
        );
        run(
            &mut s,
            "sketch.spline",
            json!({"sketch": "Hem", "control": scaled(&[[14.0, 0.0], [18.0, 8.0], [18.0, 22.0], [14.0, 30.0]], k)}),
        );
        built(
            &mut s,
            "feature.sheetMetal",
            json!({"sketch": "Base", "name": "L", "thickness": mm(k), "radius": mm(k),
                   "kFactor": 0.5,
                   "flanges": [{"edge": right, "outline": "Hem", "angle": "180 deg"}]}),
        );
        let got = error(
            &mut s,
            "sheetMetal.bendSequence",
            json!({"feature": "L", "punch": scaled(&NARROW, k), "die": scaled(&DIE, k),
                   "hem": {"flanges": [0], "preBend": 120, "pad": pad(5.0, k)}}),
        );
        assert!(got.contains("spline"), "scale {k}: {got}");
    }
}
