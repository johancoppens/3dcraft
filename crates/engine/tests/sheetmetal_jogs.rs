//! Jogs across a flat by command (docs/design/sheetmetal.md › *Jogs across
//! a flat*): a plate 100 × 40 with a construction line across it at
//! x = 50 and a wall on its far edge, jogged 10 up at 60 degrees. Held at
//! 1e-3, 1 and 1e6: the flat area in closed form, the volume `A T` at
//! K = ½, the bend table's jog bends, the names the jog leaves, saved and
//! opened again to the same part, an edit that keeps the jog's id, the
//! press-brake check taking the jog as formed, and bad jogs as errors.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{body, built, error, new_document, run, session_in, workdir};
use craft3d_engine::Session;
use craft3d_math::Portable;
use serde_json::{Value, json};
use std::f64::consts::{FRAC_PI_2, PI};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn mm(v: f64) -> String {
    format!("{v} mm")
}

fn number(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

fn names(s: &mut Session) -> Vec<String> {
    body(s, "Plate")["faceNames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap().to_string())
        .collect()
}

/// The plate's lines (bottom, right, top, left) and the jog's line.
fn sketch(s: &mut Session, k: f64) -> (Vec<String>, String) {
    run(s, "sketch.create", json!({"plane": "xy", "name": "Base"}));
    let made = run(
        s,
        "sketch.rect",
        json!({"sketch": "Base", "from": [0.0, 0.0], "to": [100.0 * k, 40.0 * k]}),
    );
    let lines = made["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap().to_string())
        .collect();
    let jog = run(
        s,
        "sketch.line",
        json!({"sketch": "Base", "from": [50.0 * k, -5.0 * k], "to": [50.0 * k, 45.0 * k],
               "construction": true}),
    );
    (lines, jog["entity"].as_str().unwrap().to_string())
}

#[test]
fn a_plate_jogged_by_command() {
    for k in SCALES {
        let dir = workdir("sheetmetal-jogs", &format!("plate-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let (lines, line) = sketch(&mut s, k);
        let wall = json!({"edge": lines[1], "length": mm(15.0 * k), "angle": "90 deg"});
        built(
            &mut s,
            "feature.sheetMetal",
            json!({"sketch": "Base", "name": "Plate", "thickness": mm(k), "radius": mm(2.0 * k),
                   "kFactor": 0.5, "flanges": [wall.clone()]}),
        );
        let before = names(&mut s);
        let jog = json!({"line": line, "offset": mm(10.0 * k), "angle": "60 deg"});
        let out = run(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Plate", "jogs": [jog.clone()]}),
        );
        assert_eq!(out["status"], "ok", "scale {k}: {out}");
        let found = body(&mut s, "Plate");
        assert_eq!(found["valid"], true, "scale {k}: {found}");
        let table = run(&mut s, "sheetMetal.bends", json!({"feature": "Plate"}));
        let (t, r, alpha) = (k, 2.0 * k, PI / 3.0);
        let ba = alpha * (r + 0.5 * t);
        let w = (10.0 * k - (2.0 * r + t) * (1.0 - alpha.pcos())) / alpha.psin();
        let wall_ba = FRAC_PI_2 * (r + 0.5 * t);
        let want = 40.0 * k * (100.0 * k + 2.0 * ba + w) + 40.0 * k * (wall_ba + 15.0 * k);
        let a = number(&table["flatArea"]);
        assert!(
            (a - want).abs() <= 1e-9 * want,
            "scale {k}: flat area {a} against {want}"
        );
        let v = number(&found["volume"]);
        assert!(
            (v - a * t).abs() <= 1e-7 * v,
            "scale {k}: volume {v} against {}",
            a * t
        );
        // The bend table: the wall, then the jog's two bends.
        let bends = table["bends"].as_array().unwrap();
        assert_eq!(bends.len(), 3, "{table}");
        assert_eq!(bends[0]["flange"], 0);
        assert_eq!(bends[1]["jog"], 0);
        assert_eq!(bends[1]["flange"], Value::Null);
        assert_eq!(bends[1]["direction"], "up");
        assert_eq!(bends[2]["direction"], "down");
        assert_eq!(bends[1]["id"], 2, "{table}");
        assert_eq!(bends[2]["id"], 3, "{table}");
        for b in &bends[1..] {
            assert!((number(&b["angle"]) - 60.0).abs() <= 1e-6, "{b}");
            assert!((number(&b["allowance"]) - ba).abs() <= 1e-9 * ba, "{b}");
        }
        // Names: every face name of the plate without the jog is kept but
        // the rims along the two edges the line cuts.
        let after = names(&mut s);
        let lost: Vec<&String> = before.iter().filter(|n| !after.contains(n)).collect();
        assert!(
            lost.len() == 2 && lost.iter().all(|n| n.contains("rim(")),
            "scale {k}: lost {lost:?}"
        );
        // Saved and opened again: the same part.
        run(
            &mut s,
            "file.save",
            json!({"path": "plate.3dcraft", "overwrite": true}),
        );
        run(&mut s, "file.open", json!({"path": "plate.3dcraft"}));
        let again = body(&mut s, "Plate");
        assert_eq!(again["volume"], found["volume"], "scale {k}");
        assert_eq!(again["faceNames"], found["faceNames"], "scale {k}");
        // Made with the jog from the start: the same part.
        built(
            &mut s,
            "feature.sheetMetal",
            json!({"sketch": "Base", "name": "Again", "thickness": mm(k), "radius": mm(2.0 * k),
                   "kFactor": 0.5, "flanges": [wall.clone()], "jogs": [jog.clone()]}),
        );
        assert_eq!(
            body(&mut s, "Again")["volume"],
            found["volume"],
            "scale {k}"
        );
        run(&mut s, "edit.undo", json!({}));
        // An edit that sends the jog back without its id keeps it (equal
        // in every field), so its names stay.
        let out = run(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Plate", "jogs": [jog.clone()], "thickness": mm(k)}),
        );
        assert_eq!(out["status"], "ok", "{out}");
        assert_eq!(body(&mut s, "Plate")["faceNames"], found["faceNames"]);
        // The press brake: the jog is formed, the wall the one stroke.
        let check = run(
            &mut s,
            "sheetMetal.bendSequence",
            json!({"feature": "Plate", "punch": [[-1.0 * k, 0.0], [1.0 * k, 0.0], [1.0 * k, 50.0 * k], [-1.0 * k, 50.0 * k]],
                   "die": [[-10.0 * k, 0.0], [10.0 * k, 0.0], [10.0 * k, -10.0 * k], [-10.0 * k, -10.0 * k]]}),
        );
        assert_eq!(check["feasible"], true, "scale {k}: {check}");
        assert_eq!(check["order"], json!([0]), "scale {k}: {check}");
        // Too small an offset for its bends: the part fails, naming the jog.
        let small = json!({"line": line, "offset": mm(1.0 * k), "angle": "60 deg"});
        let out = run(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Plate", "jogs": [small]}),
        );
        assert_eq!(out["status"], "failed", "{out}");
        let why = out["error"].as_str().unwrap();
        assert!(why.contains("jogs.0") && why.contains("web"), "{out}");
    }
}

#[test]
fn bad_jogs_are_errors() {
    let dir = workdir("sheetmetal-jogs", "bad");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    let (lines, line) = sketch(&mut s, 1.0);
    built(
        &mut s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "name": "Plate", "thickness": "1 mm", "radius": "2 mm",
               "kFactor": 0.5}),
    );
    for (jogs, why) in [
        (json!("x"), "list of jogs"),
        (json!([3]), "is an object"),
        (
            json!([{"line": line, "offset": "5 mm"}]),
            "needs its `angle`",
        ),
        (
            json!([{"offset": "5 mm", "angle": "60 deg"}]),
            "needs its `line`",
        ),
        (
            json!([{"line": line, "offset": "5 mm", "angle": "60 deg", "spin": 1}]),
            "unknown key",
        ),
        (
            json!([{"line": line, "offset": "5 mm", "angle": "60 deg", "direction": "left"}]),
            "up or down",
        ),
        (
            json!([{"line": line, "offset": "5 mm", "angle": "60 deg", "id": 0}]),
            "jogs.0.id",
        ),
        (
            json!([{"line": line, "offset": "5 mm", "angle": "60 deg", "id": 7}]),
            "no jog of this part",
        ),
        (
            json!([{"line": "e999", "offset": "5 mm", "angle": "60 deg"}]),
            "has no e999",
        ),
    ] {
        let got = error(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Plate", "jogs": jogs}),
        );
        assert!(got.contains(why), "{got} does not say {why}");
    }
    // A jog on a line that is no line: the sketch's point.
    let point = run(
        &mut s,
        "sketch.point",
        json!({"sketch": "Base", "at": [10.0, 10.0]}),
    );
    let got = error(
        &mut s,
        "sheetMetal.edit",
        json!({"feature": "Plate", "jogs": [{"line": point["entity"], "offset": "5 mm", "angle": "60 deg"}]}),
    );
    assert!(got.contains("not a line"), "{got}");
    let _ = lines;
}
