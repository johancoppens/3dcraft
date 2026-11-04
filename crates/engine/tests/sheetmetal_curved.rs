//! Curved flanges by command (docs/design/sheetmetal.md › *Curved
//! bends*), at 1e-3, 1 and 1e6: a plate 100 wide whose top is a half
//! circle of radius 50, its arc flanged 10 up at 90 degrees (`R` = 2, `T`
//! = 1, K = 0.4): the volume by Pappus, the bend table's row (the strain
//! bound, the allowance, the tip radius), the flat area, the measures
//! `bendStrain`, `bendRadius` and `flangeLength`, the export's statement
//! of the approximation and its bend line, `maxStrain` refusing it, the
//! names kept when a straight flange is added and when the curved one is
//! edited with its id, saved and reopened; a collar round a round hole;
//! bad curved flanges are errors.

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
use std::f64::consts::{FRAC_PI_2, PI, TAU};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn mm(v: f64) -> String {
    format!("{v} mm")
}

fn number(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

fn names(s: &mut Session, part: &str) -> Vec<String> {
    body(s, part)["faceNames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap().to_string())
        .collect()
}

/// The dome's sketch: three lines and the half circle of radius 50 about
/// (50, 60) on top; returns the lines and the arc.
fn dome(s: &mut Session, k: f64) -> (Vec<String>, String) {
    run(s, "sketch.create", json!({"plane": "xy", "name": "Base"}));
    let mut lines = Vec::new();
    for (a, b) in [
        ([0.0, 0.0], [100.0, 0.0]),
        ([100.0, 0.0], [100.0, 60.0]),
        ([0.0, 60.0], [0.0, 0.0]),
    ] {
        let made = run(
            s,
            "sketch.line",
            json!({"sketch": "Base", "from": [a[0] * k, a[1] * k], "to": [b[0] * k, b[1] * k]}),
        );
        lines.push(made["entity"].as_str().unwrap().to_string());
    }
    let arc = run(
        s,
        "sketch.arc",
        json!({"sketch": "Base", "center": [50.0 * k, 60.0 * k], "start": [100.0 * k, 60.0 * k],
               "end": [0.0, 60.0 * k]}),
    );
    (lines, arc["entity"].as_str().unwrap().to_string())
}

/// `∫ ρ dA` of the section about the arc's axis (Pappus), written out.
fn pappus(a: f64, eps: f64, theta: f64, r: f64, t: f64, l: f64) -> f64 {
    let ro = r + t;
    let area = theta * (ro * ro - r * r) / 2.0 + l * t;
    let mx = (1.0 - theta.pcos()) * (ro.powi(3) - r.powi(3)) / 3.0
        + theta.psin() * l * (ro * ro - r * r) / 2.0
        + theta.pcos() * t * l * l / 2.0;
    a * area + eps * mx
}

#[test]
fn a_curved_flange_by_command() {
    for k in SCALES {
        let dir = workdir("sheetmetal-curved", &format!("dome-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let (lines, arc) = dome(&mut s, k);
        let rim = json!({"edge": arc, "length": mm(10.0 * k), "angle": "90 deg"});
        built(
            &mut s,
            "feature.sheetMetal",
            json!({"sketch": "Base", "name": "Pan", "thickness": mm(k), "radius": mm(2.0 * k),
                   "kFactor": 0.4, "curvedFlanges": [rim.clone()]}),
        );
        let found = body(&mut s, "Pan");
        assert_eq!(found["valid"], true, "scale {k}: {found}");
        let (a, t, r, l) = (50.0 * k, k, 2.0 * k, 10.0 * k);
        let base = (100.0 * 60.0 + 0.5 * PI * 50.0 * 50.0) * k * k;
        let want = base * t + PI * pappus(a, 1.0, FRAC_PI_2, r, t, l);
        let v = number(&found["volume"]);
        assert!(
            (v - want).abs() <= 1e-7 * want,
            "scale {k}: {v} against {want}"
        );
        // The bend table.
        let table = run(&mut s, "sheetMetal.bends", json!({"feature": "Pan"}));
        let row = &table["curvedFlanges"][0];
        assert_eq!(row["id"], 1, "{table}");
        assert_eq!(row["grows"], "out", "{row}");
        assert_eq!(row["strain"]["proven"], true, "{row}");
        let ba = FRAC_PI_2 * (r + 0.4 * t);
        assert!((number(&row["allowance"]) - ba).abs() <= 1e-12 * ba);
        let tip = a + ba + l;
        assert!((number(&row["tipRadius"]) - tip).abs() <= 1e-12 * tip);
        // At 90 degrees the shrink flange's largest compression is the
        // tip's inner fibre: (R − BA − L) / (a + BA + L).
        let least = (r - ba - l) / (a + ba + l);
        let largest = number(&row["strain"]["largest"]);
        assert!((largest + least).abs() <= 1e-10, "{row}");
        let area = base + 0.5 * PI * (tip * tip - a * a);
        assert!(
            (number(&table["flatArea"]) - area).abs() <= 1e-9 * area,
            "scale {k}: {table}"
        );
        // The measures.
        for (measure, max, status) in [
            ("bendStrain", largest + 1e-9, "passed"),
            ("bendStrain", largest - 1e-6, "failed"),
            ("bendRadius", 2.0 * k * 1.000_001, "passed"),
            ("flangeLength", 10.0 * k * 1.000_001, "passed"),
        ] {
            let check = run(
                &mut s,
                "assert.add",
                json!({"name": format!("{measure}{max}"), "measure": measure, "of": "Pan",
                       "max": if measure == "bendStrain" { format!("{max}") } else { mm(max) }}),
            );
            assert_eq!(check["status"], status, "scale {k}: {check}");
        }
        // The export states the approximation and draws the bend line.
        let out = run(
            &mut s,
            "file.export",
            json!({"path": "pan.dxf", "sheetMetal": "Pan"}),
        );
        assert_eq!(out["approximation"]["approximated"], true, "{out}");
        assert_eq!(
            out["approximation"]["curvedFlanges"][0]["strain"]["largest"],
            row["strain"]["largest"]
        );
        assert_eq!(out["bendsUp"], 1, "{out}");
        assert!(
            (number(&out["flatArea"]) - area).abs() <= 1e-9 * area,
            "{out}"
        );
        // A limit below the bound refuses the part, naming it.
        let refused = run(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Pan", "maxStrain": format!("{}", largest * 0.5)}),
        );
        assert_eq!(refused["status"], "failed", "{refused}");
        let why = refused["error"].as_str().unwrap();
        assert!(
            why.contains("curvedFlanges.0") && why.contains("maxStrain"),
            "{why}"
        );
        run(&mut s, "edit.undo", json!({}));
        let ok = run(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Pan", "maxStrain": format!("{}", largest * 1.01)}),
        );
        assert_eq!(ok["status"], "ok", "{ok}");
        // No limit again (null), for the longer rim below.
        let ok = run(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Pan", "maxStrain": null}),
        );
        assert_eq!(ok["status"], "ok", "{ok}");
        // Names: the rim's faces by its id, kept when a flange is added on
        // a straight edge and when the rim is edited with its id.
        let before = names(&mut s, "Pan");
        assert!(
            before.iter().any(|n| n == "F2:copy(0)/F2:face(1)"),
            "{before:?}"
        );
        assert!(
            before.iter().any(|n| n == "F2:copy(1)/F2:face(2)"),
            "{before:?}"
        );
        let wall = json!({"edge": lines[0], "length": mm(15.0 * k), "angle": "90 deg",
                          "direction": "down"});
        let out = run(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Pan", "flanges": [wall]}),
        );
        assert_eq!(out["status"], "ok", "scale {k}: {out}");
        let with_wall = names(&mut s, "Pan");
        let lost: Vec<&String> = before
            .iter()
            .filter(|n| !with_wall.contains(n) && !n.contains("rim("))
            .collect();
        assert!(lost.is_empty(), "scale {k}: lost {lost:?}");
        let curved_faces: Vec<&String> = before
            .iter()
            .filter(|n| n.ends_with("face(1)") || n.ends_with("face(2)"))
            .collect();
        for n in &curved_faces {
            assert!(with_wall.contains(n), "scale {k}: {n} lost with the wall");
        }
        let longer = json!({"edge": arc, "length": mm(12.0 * k), "angle": "90 deg", "id": 1});
        let out = run(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Pan", "curvedFlanges": [longer]}),
        );
        assert_eq!(out["status"], "ok", "scale {k}: {out}");
        let edited = names(&mut s, "Pan");
        assert_eq!(
            edited, with_wall,
            "scale {k}: an edit with its id keeps every name"
        );
        // Saved and opened again: the same part.
        let found = body(&mut s, "Pan");
        run(
            &mut s,
            "file.save",
            json!({"path": "pan.3dcraft", "overwrite": true}),
        );
        run(&mut s, "file.open", json!({"path": "pan.3dcraft"}));
        let again = body(&mut s, "Pan");
        assert_eq!(again["volume"], found["volume"], "scale {k}");
        assert_eq!(again["faceNames"], found["faceNames"], "scale {k}");
    }
}

#[test]
fn a_collar_round_a_round_hole() {
    for k in SCALES {
        let dir = workdir("sheetmetal-curved", &format!("collar-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "xy", "name": "Base"}),
        );
        run(
            &mut s,
            "sketch.rect",
            json!({"sketch": "Base", "from": [0.0, 0.0], "to": [100.0 * k, 60.0 * k]}),
        );
        let hole = run(
            &mut s,
            "sketch.circle",
            json!({"sketch": "Base", "center": [50.0 * k, 30.0 * k], "radius": 15.0 * k}),
        );
        let hole = hole["entity"].as_str().unwrap().to_string();
        built(
            &mut s,
            "feature.sheetMetal",
            json!({"sketch": "Base", "name": "Plate", "thickness": mm(k), "radius": mm(1.5 * k),
                   "kFactor": 0.5,
                   "curvedFlanges": [{"edge": hole, "length": mm(4.0 * k), "angle": "90 deg",
                                      "direction": "down"}]}),
        );
        let found = body(&mut s, "Plate");
        assert_eq!(found["valid"], true, "scale {k}: {found}");
        let (a, t, r, l) = (15.0 * k, k, 1.5 * k, 4.0 * k);
        let base = (100.0 * 60.0 - PI * 15.0 * 15.0) * k * k;
        let want = base * t + TAU * pappus(a, -1.0, FRAC_PI_2, r, t, l);
        let v = number(&found["volume"]);
        assert!(
            (v - want).abs() <= 1e-7 * want,
            "scale {k}: {v} against {want}"
        );
        let table = run(&mut s, "sheetMetal.bends", json!({"feature": "Plate"}));
        let row = &table["curvedFlanges"][0];
        assert_eq!(row["grows"], "in", "{row}");
        // A stretch flange: the tip's inner fibre stretched by
        // −(R − BA − L) / (a − BA − L).
        let ba = FRAC_PI_2 * (r + 0.5 * t);
        let most = -(r - ba - l) / (a - ba - l);
        assert!(
            (number(&row["strain"]["most"]) - most).abs() <= 1e-10,
            "{row}"
        );
        let out = run(
            &mut s,
            "file.export",
            json!({"path": "plate.svg", "sheetMetal": "Plate"}),
        );
        assert_eq!(out["bendsDown"], 1, "{out}");
    }
}

#[test]
fn bad_curved_flanges_are_errors() {
    let dir = workdir("sheetmetal-curved", "bad");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    let (lines, arc) = dome(&mut s, 1.0);
    built(
        &mut s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "name": "Pan", "thickness": "1 mm", "radius": "2 mm",
               "kFactor": 0.5}),
    );
    for (list, why) in [
        (json!("x"), "list of curved flanges"),
        (json!([3]), "is an object"),
        (
            json!([{"edge": arc, "length": "5 mm"}]),
            "needs its `angle`",
        ),
        (
            json!([{"length": "5 mm", "angle": "90 deg"}]),
            "needs its `edge`",
        ),
        (
            json!([{"edge": arc, "length": "5 mm", "angle": "90 deg", "spin": 1}]),
            "unknown key",
        ),
        (
            json!([{"edge": arc, "length": "5 mm", "angle": "90 deg", "direction": "left"}]),
            "up or down",
        ),
        (
            json!([{"edge": arc, "length": "5 mm", "angle": "90 deg", "from": "middle"}]),
            "tangent or mould",
        ),
        (
            json!([{"edge": arc, "length": "5 mm", "angle": "90 deg", "id": 0}]),
            "curvedFlanges.0.id",
        ),
        (
            json!([{"edge": arc, "length": "5 mm", "angle": "90 deg", "id": 9}]),
            "no curved flange of this part",
        ),
        (
            json!([{"edge": "e999", "length": "5 mm", "angle": "90 deg"}]),
            "has no e999",
        ),
        (
            json!([{"edge": lines[0], "length": "5 mm", "angle": "90 deg"}]),
            "not an arc or a circle",
        ),
    ] {
        let got = error(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Pan", "curvedFlanges": list}),
        );
        assert!(got.contains(why), "{got} does not say {why}");
    }
    // The shapes the method refuses fail the part, naming the flange.
    for (length, radius, why) in [
        ("5 mm", "49.5 mm", "reach the axis of its own arc"),
        ("0 mm", "2 mm", "length has to be more"),
    ] {
        let out = run(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Pan", "curvedFlanges": [{"edge": arc, "length": length,
                   "angle": "90 deg", "radius": radius}]}),
        );
        assert_eq!(out["status"], "failed", "{out}");
        let got = out["error"].as_str().unwrap();
        assert!(
            got.contains("curvedFlanges.0") && got.contains(why),
            "{got}"
        );
    }
    // No curved flange: the measure has no value.
    run(
        &mut s,
        "sheetMetal.edit",
        json!({"feature": "Pan", "curvedFlanges": []}),
    );
    let check = run(
        &mut s,
        "assert.add",
        json!({"name": "none", "measure": "bendStrain", "of": "Pan", "max": "0.3"}),
    );
    assert_eq!(check["status"], "error", "{check}");
}
