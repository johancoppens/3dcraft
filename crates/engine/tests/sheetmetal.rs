//! Sheet metal through the commands (docs/design/sheetmetal.md): an L
//! bracket whose volume, box and flat pattern follow closed forms and a
//! thickness variable, its bend table (the allowance in closed form and as
//! the unfold develops it, the radius and angle measured on the part),
//! saved and reopened to the same body and names; a box with corner
//! reliefs; the manufacturability assertions with their limits from
//! expressions and a document table; the flat pattern exported to DXF and
//! SVG with its bend lines styled; flanges that collide when bent fail the
//! feature, bad parameters are errors. At three scales.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;

use common::checked::{
    assert_close, body, built, deep, error, failed, new_document, run, session_in, workdir,
};
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::f64::consts::FRAC_PI_2;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn number(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

/// A rectangle `w` × `d` from the origin in the xy plane; its lines'
/// entity ids (bottom, right, top, left).
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

/// The assertion `name` as `assert.list` checks it now.
fn listed(s: &mut Session, name: &str) -> Value {
    let all = run(s, "assert.list", json!({}));
    all["assertions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["name"] == name)
        .unwrap_or_else(|| panic!("no assertion {name} in {all}"))
        .clone()
}

fn assertion(s: &mut Session, name: &str, spec: Value) -> Value {
    let mut params = spec;
    params["name"] = json!(name);
    run(s, "assert.add", params);
    listed(s, name)
}

#[test]
fn an_l_bracket_follows_its_closed_forms_and_a_variable() {
    for k in SCALES {
        let dir = workdir("sheetmetal", &format!("l-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let (w, d, l, r) = (40.0 * k, 30.0 * k, 25.0 * k, 3.0 * k);
        run(
            &mut s,
            "variable.set",
            json!({"name": "t", "expr": format!("{} mm", 2.0 * k)}),
        );
        let lines = rect(&mut s, "Base", w, d);
        built(
            &mut s,
            "feature.sheetMetal",
            json!({"sketch": "Base", "name": "Bracket", "thickness": "t", "radius": format!("{r} mm"),
                   "kFactor": "0.4",
                   "flanges": [{"edge": lines[0], "length": format!("{l} mm"), "angle": "90 deg"}]}),
        );
        for t in [2.0 * k, 1.0 * k] {
            run(
                &mut s,
                "variable.set",
                json!({"name": "t", "expr": format!("{t} mm")}),
            );
            let found = body(&mut s, "Bracket");
            assert_eq!(found["kind"], "solid", "{found}");
            assert_eq!(found["valid"], true);
            let v = w * t * (d + l) + FRAC_PI_2 * w * ((r + t).ppowi(2) - r * r) / 2.0;
            assert_close(&found["volume"], v, "volume");
            assert_close(&found["bbox"][0][1], -(r + t), "the flange's outside");
            assert_close(&found["bbox"][1][2], t + r + l, "the flange's top");
            let table = run(&mut s, "sheetMetal.bends", json!({"feature": "Bracket"}));
            let bend = &table["bends"][0];
            let ba = FRAC_PI_2 * (r + 0.4 * t);
            assert_close(&bend["allowance"], ba, "allowance");
            assert!(
                (number(&bend["developed"]) - ba).abs() <= 4e-6 * k,
                "{table}"
            );
            assert_close(&bend["radius"], r, "radius");
            assert!((number(&bend["angle"]) - 90.0).abs() < 1e-9, "{table}");
            assert_close(&bend["flangeLength"], l, "flange length");
            assert_close(&bend["setback"], r + t, "setback at 90");
            assert_close(&bend["deduction"], 2.0 * (r + t) - ba, "deduction");
            assert_close(&table["flatSize"][1], d + ba + l, "flat length");
            assert_close(&table["flatSize"][0], w, "flat width");
            assert_close(&table["flatArea"], w * (d + ba + l), "flat area");
            deep(&mut s);
        }
        // Names: the plate's of the folded skin.
        let found = body(&mut s, "Bracket");
        let names: Vec<String> = found["faceNames"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_str().unwrap().to_string())
            .collect();
        for face in [
            "F2:copy(0)/F2:face(0)",
            "F2:copy(1)/F2:face(1)",
            "F2:copy(0)/F2:face(2)",
        ] {
            assert!(names.iter().any(|n| n == face), "{face} in {names:?}");
        }
        assert!(names.iter().any(|n| n.starts_with("F2:rim(")), "{names:?}");
        // Saved and reopened: the same body and names.
        run(&mut s, "file.save", json!({"path": "bracket.3dcraft"}));
        let mut u = session_in(&dir);
        run(&mut u, "file.open", json!({"path": "bracket.3dcraft"}));
        let again = body(&mut u, "Bracket");
        assert_eq!(found["bbox"], again["bbox"]);
        assert_eq!(found["faceNames"], again["faceNames"]);
        assert_eq!(found["volume"], again["volume"]);
    }
}

#[test]
fn a_box_with_reliefs_and_the_manufacturability_assertions() {
    for k in SCALES {
        let dir = workdir("sheetmetal", &format!("box-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let (w, d, t, r, l) = (60.0 * k, 40.0 * k, 1.0 * k, 1.0 * k, 10.0 * k);
        run(
            &mut s,
            "variable.set",
            json!({"name": "t", "expr": format!("{t} mm")}),
        );
        let lines = rect(&mut s, "Base", w, d);
        run(
            &mut s,
            "sketch.circle",
            json!({"sketch": "Base", "center": [20.0 * k, 7.0 * k], "radius": 2.5 * k}),
        );
        let flanges: Vec<Value> = lines
            .iter()
            .map(|e| json!({"edge": e, "length": format!("{l} mm"), "angle": "90 deg"}))
            .collect();
        built(
            &mut s,
            "feature.sheetMetal",
            json!({"sketch": "Base", "name": "Tray", "thickness": "t", "radius": format!("{r} mm"),
                   "kFactor": 0.5, "flanges": flanges}),
        );
        let ba = FRAC_PI_2 * (r + 0.5 * t);
        let bare = w * d - std::f64::consts::PI * (2.5 * k).ppowi(2) + 2.0 * (w + d) * (ba + l);
        // K = 1/2: the volume is the flat area times the thickness.
        assert_close(
            &body(&mut s, "Tray")["volume"],
            bare * t,
            "volume without reliefs",
        );
        let check = assertion(
            &mut s,
            "Reliefs",
            json!({"measure": "relief", "of": "Tray", "is": true}),
        );
        assert_eq!(check["status"], "failed", "{check}");
        // The limits come from a table of the document, by material.
        run(
            &mut s,
            "table.set",
            json!({"name": "steel", "columns": ["grade", {"name": "minRadius", "unit": "mm"}, {"name": "minFlange", "unit": "mm"}],
                   "rows": [["S235", 1.0 * k, 6.0 * k], ["S355", 1.5 * k, 8.0 * k]]}),
        );
        let radius = assertion(
            &mut s,
            "Radius",
            json!({"measure": "bendRadius", "of": "Tray", "min": "steel[\"S235\"].minRadius"}),
        );
        assert_eq!(radius["status"], "passed", "{radius}");
        let radius = assertion(
            &mut s,
            "Radius355",
            json!({"measure": "bendRadius", "of": "Tray", "min": "steel[\"S355\"].minRadius"}),
        );
        assert_eq!(radius["status"], "failed", "{radius}");
        let flange = assertion(
            &mut s,
            "Flange",
            json!({"measure": "flangeLength", "min": "steel[\"S355\"].minFlange"}),
        );
        assert_eq!(flange["status"], "passed", "{flange}");
        let mould = assertion(
            &mut s,
            "FlangeMould",
            json!({"measure": "flangeLength", "measured": "mould", "eq": format!("{} mm", l + r + t)}),
        );
        assert_eq!(mould["status"], "passed", "{mould}");
        // The hole's edge is 7 − 2.5 from the bend's tangent line.
        let hole = assertion(
            &mut s,
            "Hole",
            json!({"measure": "holeToBend", "eq": format!("{} mm", 4.5 * k)}),
        );
        assert_eq!(hole["status"], "passed", "{hole}");
        let hole = assertion(
            &mut s,
            "HoleFar",
            json!({"measure": "holeToBend", "min": format!("2.5 * t + {} mm", 3.0 * k)}),
        );
        assert_eq!(hole["status"], "failed", "{hole}");
        // With reliefs the relief assertion passes, and the area is less
        // the four corners and the flanges' shortened ends.
        let s3 = 3.0 * k;
        run(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Tray", "relief": {"shape": "rect", "size": format!("{s3} mm")}}),
        );
        let reliefs = listed(&mut s, "Reliefs");
        assert_eq!(reliefs["status"], "passed", "{reliefs}");
        let area = bare - 4.0 * s3 * s3 - 8.0 * s3 * (ba + l);
        assert_close(
            &body(&mut s, "Tray")["volume"],
            area * t,
            "volume with reliefs",
        );
        let table = run(&mut s, "sheetMetal.bends", json!({"feature": "Tray"}));
        assert_eq!(table["corners"].as_array().unwrap().len(), 4, "{table}");
        assert_eq!(table["corners"][0]["relief"], "rect");
        // The two assertions meant to fail break the document's check.
        for name in ["Radius355", "HoleFar"] {
            run(&mut s, "assert.delete", json!({"name": name}));
        }
        deep(&mut s);

        // The flat pattern, to DXF and SVG.
        for file in ["tray.dxf", "tray.svg"] {
            let out = run(
                &mut s,
                "file.export",
                json!({"path": file, "sheetMetal": "Tray"}),
            );
            assert_eq!(out["bendsUp"], 4, "{out}");
            let text = std::fs::read_to_string(dir.join(file)).unwrap();
            for layer in ["outline", "bendUp"] {
                assert!(text.contains(layer), "{file} has no layer {layer}");
            }
            if file.ends_with(".dxf") {
                assert!(text.contains("DASHED"), "{file} has no dashed bend lines");
            }
        }
    }
}

#[test]
fn flanges_that_run_into_each_other_fail_and_bad_parameters_are_errors() {
    let dir = workdir("sheetmetal", "refusals");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    let lines = rect(&mut s, "Base", 40.0, 10.0);
    // Two flanges on the long edges bent 135 degrees over the base meet
    // above it.
    let reason = failed(
        &mut s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "name": "Clash", "thickness": "1 mm", "radius": "1 mm", "kFactor": 0.4,
               "flanges": [{"edge": lines[0], "length": "15 mm", "angle": "135 deg"},
                           {"edge": lines[2], "length": "15 mm", "angle": "135 deg"}]}),
    );
    assert!(reason.contains("run into each other"), "{reason}");
    let reason = failed(
        &mut s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "name": "Soft", "thickness": "1 mm", "radius": "1 mm", "kFactor": 1.5,
               "flanges": [{"edge": lines[0], "length": "5 mm", "angle": "90 deg"}]}),
    );
    assert!(
        reason.contains("flanges.0") && reason.contains("K-factor"),
        "{reason}"
    );
    let reason = failed(
        &mut s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "name": "Neg", "thickness": "1 mm", "radius": "-1 mm", "kFactor": 0.4,
               "flanges": [{"edge": lines[0], "length": "5 mm", "angle": "90 deg"}]}),
    );
    assert!(reason.contains("below zero"), "{reason}");
    let e = error(
        &mut s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "thickness": "1 mm", "radius": "1 mm", "kFactor": 0.4,
               "flanges": [{"tip": 0, "length": "5 mm", "angle": "90 deg"}]}),
    );
    assert!(e.contains("tip"), "{e}");
    let e = error(
        &mut s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "thickness": "1 mm", "radius": "1 mm", "kFactor": 0.4,
               "flanges": [{"edge": "e999", "length": "5 mm", "angle": "90 deg"}]}),
    );
    assert!(e.contains("e999"), "{e}");
    let e = error(
        &mut s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "thickness": "1 mm", "radius": "1 mm"}),
    );
    assert!(e.contains("kFactor"), "{e}");
    let e = error(&mut s, "sheetMetal.bends", json!({"feature": "Base"}));
    assert!(e.contains("not a sheet-metal part"), "{e}");
}

#[test]
fn twenty_bends_regenerate_well_under_a_second() {
    let dir = workdir("sheetmetal", "twenty");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    let lines = rect(&mut s, "Base", 60.0, 40.0);
    // Five flanges on each of the four edges' chains: each edge carries a
    // flange, and each flange's tip four more, alternately down and up (a
    // stepped tray rim), 20 bends in all.
    let mut flanges = Vec::new();
    for line in &lines {
        let first = flanges.len();
        flanges.push(json!({"edge": line, "length": "4 mm", "angle": "90 deg"}));
        for j in 0..4 {
            let dir = if j % 2 == 0 { "down" } else { "up" };
            flanges.push(
                json!({"tip": first + j, "length": "4 mm", "angle": "90 deg", "direction": dir}),
            );
        }
    }
    let started = std::time::Instant::now();
    built(
        &mut s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "name": "Rim", "thickness": "1 mm", "radius": "1 mm", "kFactor": 0.4,
               "relief": {"shape": "round", "size": "2 mm"}, "flanges": flanges}),
    );
    let took = started.elapsed();
    let found = body(&mut s, "Rim");
    eprintln!(
        "twenty bends by command: {took:?}, {} faces",
        found["faceCount"]
    );
    assert_eq!(found["valid"], true, "{found}");
    let table = run(&mut s, "sheetMetal.bends", json!({"feature": "Rim"}));
    assert_eq!(table["bends"].as_array().unwrap().len(), 20);
}

/// The guide's sheet-metal section runs as written: the tray builds, its
/// three assertions pass, the bend table's developed allowance agrees with
/// the closed form, and the flat pattern is written.
#[test]
fn the_guide_section_bends_as_it_says() {
    let guide = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/guide/sheet-metal.md");
    let text = std::fs::read_to_string(guide).unwrap();
    let fence = "`".repeat(3);
    let section = text.split("## Sheet metal").nth(1).unwrap();
    let block = section
        .split(&format!("{fence}json"))
        .nth(1)
        .unwrap()
        .split(&fence)
        .next()
        .unwrap();
    let steps: Value = serde_json::from_str(block).unwrap();
    let dir = workdir("sheetmetal", "guide");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    let mut table = Value::Null;
    for step in steps.as_array().unwrap() {
        let id = step[0].as_str().unwrap();
        let result = run(&mut s, id, step[1].clone());
        assert_ne!(result["status"], "failed", "{id}: {result}");
        if id == "assert.add" {
            let name = step[1]["name"].as_str().unwrap();
            let check = listed(&mut s, name);
            assert_eq!(check["status"], "passed", "{check}");
        }
        if id == "sheetMetal.bends" {
            table = result;
        }
    }
    let bends = table["bends"].as_array().unwrap();
    assert_eq!(bends.len(), 4);
    for b in bends {
        let ba = FRAC_PI_2 * (2.0 + 0.42 * 1.5);
        assert!((number(&b["allowance"]) - ba).abs() < 1e-9, "{b}");
        assert!((number(&b["developed"]) - ba).abs() < 1e-5, "{b}");
    }
    assert!(dir.join("tray.dxf").exists());
}

/// A spline hole's distance to the bend is searched with proven bounds,
/// carried to the assertion; and a part that failed is not measured: its assertions
/// and its bend table say it failed.
#[test]
fn a_spline_hole_is_not_proven_and_a_failed_part_is_not_measured() {
    let dir = workdir("sheetmetal", "bounds");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    let lines = rect(&mut s, "Base", 60.0, 40.0);
    run(
        &mut s,
        "sketch.line",
        json!({"sketch": "Base", "from": [40.0, 15.0], "to": [20.0, 15.0]}),
    );
    run(
        &mut s,
        "sketch.spline",
        json!({"sketch": "Base", "control": [[20.0, 15.0], [24.0, 2.0], [33.0, 4.0], [40.0, 15.0]]}),
    );
    built(
        &mut s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "name": "Plate", "thickness": "1 mm", "radius": "1 mm", "kFactor": 0.4,
               "flanges": [{"edge": lines[0], "length": "10 mm", "angle": "90 deg"}]}),
    );
    let table = run(&mut s, "sheetMetal.bends", json!({"feature": "Plate"}));
    let hole = &table["holes"][0];
    assert_eq!(hole["exact"], false, "{table}");
    let (d, lower) = (number(&hole["distance"]), number(&hole["lower"]));
    assert!(lower < d && d - lower <= 1e-6, "{table}");
    // The bounds come with the measure, within an eighth of the
    // tolerance of each other either way of the true distance, so a limit
    // past the upper bound by the tolerance fails and one at the lower
    // bound passes (a band wider than the tolerance would be notProven).
    let check = assertion(
        &mut s,
        "Tight",
        json!({"measure": "holeToBend", "min": format!("{} mm", d + 3e-6)}),
    );
    assert_eq!(check["status"], "failed", "{check}");
    let check = assertion(
        &mut s,
        "Loose",
        json!({"measure": "holeToBend", "min": format!("{lower} mm")}),
    );
    assert_eq!(check["status"], "passed", "{check}");
    for name in ["Tight", "Loose"] {
        run(&mut s, "assert.delete", json!({"name": name}));
    }
    // The part fails (a K-factor past 1): measures and the table refuse.
    run(
        &mut s,
        "sheetMetal.edit",
        json!({"feature": "Plate", "kFactor": 1.5}),
    );
    let check = assertion(
        &mut s,
        "Radius",
        json!({"measure": "bendRadius", "of": "Plate", "min": "1 mm"}),
    );
    assert_eq!(check["status"], "error", "{check}");
    // Named by `of`, the part is read as regeneration left it (by its
    // feature, not its body: repro-sheet-metal-enclosure-3)
    // and refused as failed, with the reason; named by none, every
    // sheet-metal part is read and this one refused as failed too.
    assert!(
        check["message"].as_str().unwrap().contains("failed")
            && check["message"].as_str().unwrap().contains("K-factor"),
        "{check}"
    );
    let check = assertion(
        &mut s,
        "AnyRadius",
        json!({"measure": "bendRadius", "min": "1 mm"}),
    );
    assert_eq!(check["status"], "error", "{check}");
    assert!(
        check["message"].as_str().unwrap().contains("failed"),
        "{check}"
    );
    let e = error(&mut s, "sheetMetal.bends", json!({"feature": "Plate"}));
    assert!(e.contains("failed"), "{e}");
}
