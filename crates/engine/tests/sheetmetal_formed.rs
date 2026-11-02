//! Formed louvres by command (docs/design/sheetmetal.md › *Formed
//! louvres*), at 1e-3, 1 and 1e6: a plate 100 × 60 with a wall 15 high on
//! its right edge (`R` = 2, `T` = 1, K = ½, so the flat area times `T` is
//! the volume), and a formed louvre lanced from (30, 20) to (70, 20), its
//! hood 8 wide and 3 high behind the lance (its left), closed: its volume
//! `A T − F T + V_hood` in closed form, the bend table's louvre, the
//! measure `louvreStretch`, the pattern's lance and its `form` layer, the
//! names kept when the louvre or a flange is added, saved and reopened;
//! bad louvres are errors and a misplaced one fails the part by reason.

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

/// The plate's lines and the lance.
fn sketch(s: &mut Session, k: f64) -> (Vec<String>, String) {
    run(s, "sketch.create", json!({"plane": "xy", "name": "Base"}));
    let made = run(
        s,
        "sketch.rect",
        json!({"sketch": "Base", "from": [0.0, 0.0], "to": [100.0 * k, 60.0 * k]}),
    );
    let lines = made["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap().to_string())
        .collect();
    let lance = run(
        s,
        "sketch.line",
        json!({"sketch": "Base", "from": [30.0 * k, 20.0 * k], "to": [70.0 * k, 20.0 * k],
               "construction": true}),
    );
    (lines, lance["entity"].as_str().unwrap().to_string())
}

/// The hood's closed forms for width `w`, height `h`, thickness `t` and
/// middle `l` (closed ends): its section's radius and turn, its volume
/// (`2αρ T L` and the two quarter turns, `π ∫ x dA`), the footprint's
/// area `L w + π w²/2` and the stretch `2αρ/w − 1`.
fn hood(w: f64, h: f64, t: f64, l: f64) -> (f64, f64, f64) {
    let rho = (w * w + h * h) / (4.0 * h);
    let alpha = 2.0 * (h / w).patan();
    let c = alpha.pcos();
    let cube = |x: f64| x * x * x;
    let first = (cube(rho + t) - cube(rho)) / 3.0 * (1.0 - c);
    let second = w * alpha * (rho * rho - (rho - t) * (rho - t)) / 2.0
        + (cube(rho) - cube(rho - t)) / 3.0 * (c - 1.0);
    let volume = 2.0 * alpha * rho * t * l + PI * (first + second);
    let footprint = l * w + PI * w * w / 2.0;
    (volume, footprint, 2.0 * alpha * rho / w - 1.0)
}

#[test]
fn a_formed_louvre_by_command() {
    for k in SCALES {
        let dir = workdir("sheetmetal-formed", &format!("plate-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let (lines, lance) = sketch(&mut s, k);
        let wall = json!({"edge": lines[1], "length": mm(15.0 * k), "angle": "90 deg"});
        built(
            &mut s,
            "feature.sheetMetal",
            json!({"sketch": "Base", "name": "Plate", "thickness": mm(k), "radius": mm(2.0 * k),
                   "kFactor": 0.5, "flanges": [wall.clone()]}),
        );
        let before = names(&mut s);
        let plain = number(&body(&mut s, "Plate")["volume"]);
        let louvre = json!({"line": lance, "width": mm(8.0 * k), "height": mm(3.0 * k)});
        let out = run(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Plate", "formedLouvres": [louvre.clone()]}),
        );
        assert_eq!(out["status"], "ok", "scale {k}: {out}");
        let found = body(&mut s, "Plate");
        assert_eq!(found["valid"], true, "scale {k}: {found}");
        let t = k;
        let (w, h, l) = (8.0 * k, 3.0 * k, 40.0 * k - 16.0 * k);
        let (v_hood, footprint, stretch) = hood(w, h, t, l);
        // The flat pattern is the plate's: the lance takes no area.
        let table = run(&mut s, "sheetMetal.bends", json!({"feature": "Plate"}));
        let ba = FRAC_PI_2 * (2.0 * k + 0.5 * t);
        let area = 100.0 * k * 60.0 * k + 60.0 * k * (ba + 15.0 * k);
        let a = number(&table["flatArea"]);
        assert!(
            (a - area).abs() <= 1e-9 * area,
            "scale {k}: {a} against {area}"
        );
        assert!(
            (plain - area * t).abs() <= 1e-8 * plain,
            "scale {k}: plain {plain}"
        );
        let v = number(&found["volume"]);
        let want = area * t - footprint * t + v_hood;
        assert!(
            (v - want).abs() <= 1e-8 * want,
            "scale {k}: volume {v} against {want}"
        );
        let listed = &table["formedLouvres"][0];
        assert_eq!(listed["id"], 2, "{table}");
        assert_eq!(listed["flat"], 0, "{table}");
        assert_eq!(listed["ends"], "closed", "{table}");
        assert!(
            (number(&listed["stretch"]) - stretch).abs() <= 1e-12,
            "{listed}"
        );
        assert!((number(&listed["footprintArea"]) - footprint).abs() <= 1e-12 * footprint);
        assert!((number(&listed["hoodVolume"]) - v_hood).abs() <= 1e-12 * v_hood);
        // Names: every name of the plate without the louvre is kept, the
        // louvre's are its id's (`face(15, 2, …)`).
        let after = names(&mut s);
        let lost: Vec<&String> = before.iter().filter(|n| !after.contains(n)).collect();
        assert!(lost.is_empty(), "scale {k}: lost {lost:?}");
        let added: Vec<&String> = after.iter().filter(|n| !before.contains(n)).collect();
        assert!(!added.is_empty(), "scale {k}: no hood faces");
        assert!(
            added.iter().all(|n| n.contains("face(15,2,")),
            "scale {k}: {added:?}"
        );
        // A flange added on another edge renames none of the louvre's.
        let lip = json!({"edge": lines[3], "length": mm(10.0 * k), "angle": "90 deg"});
        let out = run(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Plate", "flanges": [wall.clone(), lip], "formedLouvres": [louvre.clone()]}),
        );
        assert_eq!(out["status"], "ok", "scale {k}: {out}");
        let with_lip = names(&mut s);
        for n in &added {
            assert!(with_lip.contains(n), "scale {k}: {n} lost with the lip");
        }
        run(&mut s, "edit.undo", json!({}));
        // The stretch as a measure, held to a limit.
        let check = run(
            &mut s,
            "assert.add",
            json!({"name": "stretch", "measure": "louvreStretch", "of": "Plate",
                   "max": format!("{}", stretch + 1e-9)}),
        );
        assert_eq!(check["status"], "passed", "scale {k}: {check}");
        let check = run(
            &mut s,
            "assert.add",
            json!({"name": "too much", "measure": "louvreStretch", "of": "Plate",
                   "max": format!("{}", stretch - 1e-6)}),
        );
        assert_eq!(check["status"], "failed", "scale {k}: {check}");
        // The flat pattern: the lance cut, the footprint on `form`.
        let out = run(
            &mut s,
            "file.export",
            json!({"path": "plate.dxf", "sheetMetal": "Plate"}),
        );
        assert_eq!(out["formedLouvres"], 1, "{out}");
        let text = std::fs::read_to_string(dir.join("plate.dxf")).unwrap();
        assert!(text.contains("form"), "no layer form");
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
        // Down and open: built, its volume in closed form (open: the
        // section along the whole lance, the footprint `ℓ w`).
        let open = json!({"line": lance, "width": mm(8.0 * k), "height": mm(3.0 * k),
                          "ends": "open", "direction": "down"});
        let out = run(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Plate", "formedLouvres": [open]}),
        );
        assert_eq!(out["status"], "ok", "scale {k}: {out}");
        let rho = (w * w + h * h) / (4.0 * h);
        let alpha = 2.0 * (h / w).patan();
        let v_open = 2.0 * alpha * rho * t * 40.0 * k;
        let v = number(&body(&mut s, "Plate")["volume"]);
        let want = area * t - 40.0 * k * w * t + v_open;
        assert!(
            (v - want).abs() <= 1e-8 * want,
            "scale {k}: open {v} against {want}"
        );
        // Too wide: its footprint reaches past the plate's edge (45 back
        // from a lance 40 from the edge): the part fails by reason.
        let wide = json!({"line": lance, "width": mm(45.0 * k), "height": mm(3.0 * k),
                          "ends": "open"});
        let out = run(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Plate", "formedLouvres": [wide]}),
        );
        assert_eq!(out["status"], "failed", "{out}");
        let why = out["error"].as_str().unwrap();
        assert!(why.contains("formedLouvres.0"), "{out}");
    }
}

#[test]
fn bad_formed_louvres_are_errors() {
    let dir = workdir("sheetmetal-formed", "bad");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    let (_, lance) = sketch(&mut s, 1.0);
    built(
        &mut s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "name": "Plate", "thickness": "1 mm", "radius": "2 mm",
               "kFactor": 0.5}),
    );
    for (louvres, why) in [
        (json!("x"), "list of formed louvres"),
        (json!([3]), "is an object"),
        (
            json!([{"line": lance, "width": "5 mm"}]),
            "needs its `height`",
        ),
        (
            json!([{"width": "5 mm", "height": "2 mm"}]),
            "needs its `line`",
        ),
        (
            json!([{"line": lance, "width": "5 mm", "height": "2 mm", "spin": 1}]),
            "unknown key",
        ),
        (
            json!([{"line": lance, "width": "5 mm", "height": "2 mm", "ends": "half"}]),
            "closed or open",
        ),
        (
            json!([{"line": lance, "width": "5 mm", "height": "2 mm", "direction": "left"}]),
            "up or down",
        ),
        (
            json!([{"line": lance, "width": "5 mm", "height": "2 mm", "id": 0}]),
            "formedLouvres.0.id",
        ),
        (
            json!([{"line": lance, "width": "5 mm", "height": "2 mm", "id": 9}]),
            "no formed louvre of this part",
        ),
        (
            json!([{"line": "e999", "width": "5 mm", "height": "2 mm"}]),
            "has no e999",
        ),
    ] {
        let got = error(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Plate", "formedLouvres": louvres}),
        );
        assert!(got.contains(why), "{got} does not say {why}");
    }
    // The shapes the method refuses fail the part, naming the louvre.
    for (w, h, ends, why) in [
        ("5 mm", "0.5 mm", "closed", "more than the thickness"),
        ("5 mm", "6 mm", "closed", "less than its width"),
        ("45 mm", "3 mm", "open", "lies in no one flat"),
        ("25 mm", "3 mm", "closed", "no straight middle"),
    ] {
        let out = run(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Plate", "formedLouvres": [{"line": lance, "width": w, "height": h, "ends": ends}]}),
        );
        assert_eq!(out["status"], "failed", "{out}");
        let got = out["error"].as_str().unwrap();
        assert!(
            got.contains("formedLouvres.0") && got.contains(why),
            "{got}"
        );
    }
    // No louvre: the measure has no value.
    run(
        &mut s,
        "sheetMetal.edit",
        json!({"feature": "Plate", "formedLouvres": []}),
    );
    let check = run(
        &mut s,
        "assert.add",
        json!({"name": "none", "measure": "louvreStretch", "of": "Plate", "max": "0.3"}),
    );
    assert_eq!(check["status"], "error", "{check}");
}
