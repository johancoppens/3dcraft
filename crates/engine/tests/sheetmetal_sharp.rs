//! Sharp bends through the commands (docs/design/sheetmetal.md › *Sharp
//! bends*), at 1e-3, 1 and 1e6: an L with a radius variable set to 0 and
//! back, its volume, box and bend table in closed form (radius 0, the
//! allowance `θ K T`, the setback `T`, the deduction `2 T − θ K T`), its
//! names (the radius going to 0 takes away only the bend's inner skin,
//! and coming back gives every name back), saved and reopened, the flat
//! pattern exported, `bendRadius` measuring 0; a sharp U channel cleared
//! on the press brake by the closed form of the rounded one at `R = 0`;
//! refusals as failed features.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{
    assert_close, body, built, deep, failed, new_document, run, session_in, workdir,
};
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::f64::consts::{FRAC_PI_2, SQRT_2};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn mm(v: f64) -> String {
    format!("{v} mm")
}

fn rect(s: &mut Session, name: &str, w: f64, d: f64) -> Vec<String> {
    run(s, "sketch.create", json!({"plane": "xy", "name": name}));
    let made = run(
        s,
        "sketch.rect",
        json!({"sketch": name, "from": [0.0, 0.0], "to": [w, d]}),
    );
    made["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap().to_string())
        .collect()
}

fn names(found: &Value) -> BTreeSet<String> {
    found["faceNames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn a_sharp_l_by_command_keeps_its_names_and_its_closed_forms() {
    for k in SCALES {
        for (direction, inner) in [
            ("up", "F2:copy(0)/F2:face(1)"),
            ("down", "F2:copy(1)/F2:face(1)"),
        ] {
            let dir = workdir("sheetmetal-sharp", &format!("l-{direction}-{k}"));
            let mut s = session_in(&dir);
            new_document(&mut s, k);
            let (w, d, l, t, r, kf) = (40.0 * k, 30.0 * k, 25.0 * k, 2.0 * k, 3.0 * k, 0.4);
            run(&mut s, "variable.set", json!({"name": "r", "expr": mm(r)}));
            let lines = rect(&mut s, "Base", w, d);
            built(
                &mut s,
                "feature.sheetMetal",
                json!({"sketch": "Base", "name": "Bracket", "thickness": mm(t), "radius": "r",
                       "kFactor": kf,
                       "flanges": [{"edge": lines[0], "length": mm(l), "angle": "90 deg",
                                    "direction": direction}]}),
            );
            let rounded = names(&body(&mut s, "Bracket"));
            run(&mut s, "variable.set", json!({"name": "r", "expr": "0 mm"}));
            let found = body(&mut s, "Bracket");
            assert_eq!(found["valid"], true, "{found}");
            let v = w * t * (d + l) + FRAC_PI_2 * w * t * t / 2.0;
            assert_close(&found["volume"], v, "volume");
            assert_close(&found["bbox"][0][1], -t, "the wall's outside");
            if direction == "up" {
                assert_close(&found["bbox"][1][2], t + l, "the wall's top");
            } else {
                assert_close(&found["bbox"][0][2], -l, "the wall's bottom");
            }
            // Names: only the inner skin of the bend is gone.
            let sharp = names(&found);
            let gone: Vec<&String> = rounded.difference(&sharp).collect();
            assert_eq!(gone, vec![inner], "{sharp:?}");
            assert!(sharp.is_subset(&rounded), "{sharp:?} against {rounded:?}");

            let table = run(&mut s, "sheetMetal.bends", json!({"feature": "Bracket"}));
            let bend = &table["bends"][0];
            let ba = FRAC_PI_2 * kf * t;
            assert_close(&bend["radius"], 0.0, "radius");
            assert_close(&bend["allowance"], ba, "allowance");
            assert!(
                (bend["developed"].as_f64().unwrap() - ba).abs() <= 4e-6 * k,
                "{table}"
            );
            assert_close(&bend["setback"], t, "setback");
            assert_close(&bend["deduction"], 2.0 * t - ba, "deduction");
            assert_close(&table["flatSize"][1], d + ba + l, "flat length");
            assert_close(&table["flatArea"], w * (d + ba + l), "flat area");
            deep(&mut s);

            // bendRadius measures 0: a minimum of 0 passes, any more fails.
            run(
                &mut s,
                "assert.add",
                json!({"name": "R0", "measure": "bendRadius", "of": "Bracket", "min": "0 mm"}),
            );
            run(
                &mut s,
                "assert.add",
                json!({"name": "R1", "measure": "bendRadius", "of": "Bracket", "min": mm(0.5 * k)}),
            );
            let listed = run(&mut s, "assert.list", json!({}));
            let status = |name: &str| {
                listed["assertions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|a| a["name"] == name)
                    .unwrap()["status"]
                    .clone()
            };
            assert_eq!(status("R0"), "passed", "{listed}");
            assert_eq!(status("R1"), "failed", "{listed}");

            // The flat pattern exported, its bend line drawn.
            let out = run(
                &mut s,
                "file.export",
                json!({"path": "sharp.dxf", "sheetMetal": "Bracket"}),
            );
            let key = if direction == "up" {
                "bendsUp"
            } else {
                "bendsDown"
            };
            assert_eq!(out[key], 1, "{out}");

            // Saved and reopened: the same body and names.
            run(&mut s, "file.save", json!({"path": "sharp.3dcraft"}));
            let mut u = session_in(&dir);
            run(&mut u, "file.open", json!({"path": "sharp.3dcraft"}));
            let again = body(&mut u, "Bracket");
            assert_eq!(found["bbox"], again["bbox"]);
            assert_eq!(found["faceNames"], again["faceNames"]);
            assert_eq!(found["volume"], again["volume"]);

            // The radius back: every name back.
            run(&mut s, "variable.set", json!({"name": "r", "expr": mm(r)}));
            assert_eq!(names(&body(&mut s, "Bracket")), rounded);
        }
    }
}

const STRAIGHT: [[f64; 2]; 4] = [[-5.0, 0.0], [5.0, 0.0], [5.0, 100.0], [-5.0, 100.0]];
const DIE: [[f64; 2]; 4] = [[-15.0, -20.0], [15.0, -20.0], [15.0, 0.0], [-15.0, 0.0]];

fn scaled(points: &[[f64; 2]], k: f64) -> Value {
    json!(
        points
            .iter()
            .map(|p| [p[0] * k, p[1] * k])
            .collect::<Vec<_>>()
    )
}

/// A sharp U channel (base 40 wide, walls 20 high, T = 1, K = ½) bent with
/// a straight punch 10 wide clears in the order 0, 1, its second step's
/// least distance the rounded channel's closed form at `R = 0`:
/// `(W − H)/√2 − p` (sheetmetal_bendsequence.rs).
#[test]
fn a_sharp_channel_clears_the_press_brake_by_its_closed_form() {
    for k in SCALES {
        let dir = workdir("sheetmetal-sharp", &format!("brake-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let lines = rect(&mut s, "Base", 40.0 * k, 30.0 * k);
        built(
            &mut s,
            "feature.sheetMetal",
            json!({"sketch": "Base", "name": "U", "thickness": mm(k), "radius": "0 mm",
                   "kFactor": 0.5,
                   "flanges": [{"edge": lines[3], "length": mm(20.0 * k), "angle": "90 deg"},
                               {"edge": lines[1], "length": mm(20.0 * k), "angle": "90 deg"}]}),
        );
        let out = run(
            &mut s,
            "sheetMetal.bendSequence",
            json!({"feature": "U", "punch": scaled(&STRAIGHT, k), "die": scaled(&DIE, k)}),
        );
        assert_eq!(out["feasible"], true, "{out}");
        assert_eq!(out["order"], json!([0, 1]), "{out}");
        let steps = out["steps"].as_array().unwrap();
        let (w, h, p) = (40.0 * k, 20.0 * k, 5.0 * k);
        let least = (w - h) / SQRT_2 - p;
        let got = steps[1]["least"].as_f64().unwrap();
        assert!(
            (got - least).abs() <= 1e-6 * k,
            "scale {k}: least {got} against {least}"
        );
        for step in steps {
            for tool in [step, &step["die"]] {
                if let (Some(l), Some(p)) = (tool["least"].as_f64(), tool["proven"].as_f64()) {
                    assert!(p <= l && p >= 0.0, "{step}");
                }
            }
        }
    }
}

/// What a sharp bend refuses fails the feature with the reason.
#[test]
fn sharp_refusals_fail_the_feature() {
    let dir = workdir("sheetmetal-sharp", "refusals");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    let lines = rect(&mut s, "Base", 40.0, 30.0);
    let reason = failed(
        &mut s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "name": "Hem", "thickness": "1 mm", "radius": "0 mm", "kFactor": 0.4,
               "flanges": [{"edge": lines[0], "length": "5 mm", "angle": "180 deg"}]}),
    );
    assert!(
        reason.contains("flanges.0") && reason.contains("closed hem"),
        "{reason}"
    );
    let reason = failed(
        &mut s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "name": "K0", "thickness": "1 mm", "radius": "0 mm", "kFactor": 0,
               "flanges": [{"edge": lines[0], "length": "5 mm", "angle": "90 deg"}]}),
    );
    assert!(reason.contains("K-factor above 0"), "{reason}");
    let reason = failed(
        &mut s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "name": "Corner", "thickness": "1 mm", "radius": "0 mm", "kFactor": 0.4,
               "flanges": [{"edge": lines[0], "length": "5 mm", "angle": "90 deg"},
                           {"edge": lines[1], "length": "5 mm", "angle": "90 deg"}]}),
    );
    assert!(reason.contains("touch along a line"), "{reason}");
}

/// A later cut through a sharp bend and a pocket in its outer skin are not
/// carried onto the flat pattern, and the export says so (review of
/// 145a3511: they were left out without a word).
#[test]
fn a_later_cut_across_a_sharp_bend_is_listed_not_carried() {
    for k in SCALES {
        for (what, size, position) in [
            ("through", [10.0, 8.0, 6.0], [15.0, -5.0, -1.0]),
            ("pocket", [4.0, 1.3, 1.4], [18.0, -2.5, -0.5]),
        ] {
            let dir = workdir("sheetmetal-sharp", &format!("later-{what}-{k}"));
            let mut s = session_in(&dir);
            new_document(&mut s, k);
            let lines = rect(&mut s, "Base", 40.0 * k, 30.0 * k);
            built(
                &mut s,
                "feature.sheetMetal",
                json!({"sketch": "Base", "name": "L", "thickness": mm(2.0 * k), "radius": "0 mm",
                       "kFactor": 0.4,
                       "flanges": [{"edge": lines[0], "length": mm(25.0 * k), "angle": "90 deg"}]}),
            );
            let scaled = |v: [f64; 3]| [mm(v[0] * k), mm(v[1] * k), mm(v[2] * k)];
            run(
                &mut s,
                "feature.block",
                json!({"name": "Cutter", "size": scaled(size), "position": scaled(position)}),
            );
            built(
                &mut s,
                "feature.boolean",
                json!({"op": "subtract", "target": "L", "tool": "Cutter", "name": "Cut"}),
            );
            let out = run(
                &mut s,
                "file.export",
                json!({"path": "l.dxf", "sheetMetal": "L"}),
            );
            let listed = out["notCarried"].to_string();
            assert!(listed.contains("sharp bend"), "{what} at {k}: {out}");
        }
    }
}

/// A slot straight across a sharp bend by command (holes across a sharp
/// bend, 2026-11-04): the bend in two stretches, the volume `A T` at
/// K = ½ less nothing else (the slot's area on the strip takes `T/(2K) =
/// T` per area), every face named once, the names the same after saving
/// and reopening; a round hole on the tangent line fails the feature with
/// the reason.
#[test]
fn a_slot_straight_across_a_sharp_bend_by_command() {
    for k in SCALES {
        let dir = workdir("sheetmetal-sharp", &format!("slot-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let (w, d, l, t, kf) = (40.0 * k, 30.0 * k, 25.0 * k, 2.0 * k, 0.5);
        let lines = rect(&mut s, "Base", w, d);
        let ba = FRAC_PI_2 * kf * t;
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "xy", "name": "Slot"}),
        );
        run(
            &mut s,
            "sketch.rect",
            json!({"sketch": "Slot", "from": [15.0 * k, -ba - 5.0 * k], "to": [21.0 * k, 3.0 * k]}),
        );
        built(
            &mut s,
            "feature.sheetMetal",
            json!({"sketch": "Base", "name": "L", "thickness": mm(t), "radius": "0 mm",
                   "kFactor": kf, "cutouts": ["Slot"],
                   "flanges": [{"edge": lines[0], "length": mm(l), "angle": "90 deg"}]}),
        );
        let found = body(&mut s, "L");
        assert_eq!(found["valid"], true, "{found}");
        let area = w * (d + ba + l) - 6.0 * k * (8.0 * k + ba);
        assert_close(&found["volume"], area * t, "A T");
        let list = found["faceNames"].as_array().unwrap();
        let unique: BTreeSet<&str> = list.iter().map(|n| n.as_str().unwrap()).collect();
        assert_eq!(unique.len(), list.len(), "{list:?}");
        // The second stretch of the outer skin is named as the bend's
        // further piece.
        assert!(
            unique.iter().any(|n| n.contains("face(4294967297)")),
            "{unique:?}"
        );
        deep(&mut s);
        run(&mut s, "file.save", json!({"path": "slot.3dcraft"}));
        let mut u = session_in(&dir);
        run(&mut u, "file.open", json!({"path": "slot.3dcraft"}));
        let again = body(&mut u, "L");
        assert_eq!(found["faceNames"], again["faceNames"]);
        assert_eq!(found["volume"], again["volume"]);
    }
    let dir = workdir("sheetmetal-sharp", "round-on-the-line");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    let lines = rect(&mut s, "Base", 40.0, 30.0);
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "name": "Hole"}),
    );
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Hole", "center": [20.0, 0.0], "radius": 2.0}),
    );
    let reason = failed(
        &mut s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "name": "L", "thickness": "1 mm", "radius": "0 mm",
               "kFactor": 0.4, "cutouts": ["Hole"],
               "flanges": [{"edge": lines[0], "length": "10 mm", "angle": "90 deg"}]}),
    );
    assert!(reason.contains("straight across"), "{reason}");
}
