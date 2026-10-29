//! A flange on a line of an outlined flange (docs/design/sheetmetal.md ›
//! *Inset flanges, outlines, sides and holes*): a rail whose leg is an
//! outline of its own (a trapezoid, 80 wide at the bend and 60 at its far
//! line, 20 deep) on the base's bottom edge, and a lip bent from the leg's
//! far line, inset 5 at both ends, with and without bend reliefs. Until
//! 2026-10-29 a flange could sit on an outlined flange's tip or side only,
//! which it has not ("not supported"). Held to closed forms at 1e-3, 1 and
//! 1e6: the flat area (base 3000, leg 1400, the strips `w BA`, the lip
//! `50 × 8`, the notches `r²` each), the volume `A T` at K = ½, the names
//! of the part without the lip kept but the rim along the far line; bad
//! lines are errors.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{body, built, error, new_document, run, session_in, workdir};
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::f64::consts::FRAC_PI_2;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn mm(v: f64) -> String {
    format!("{v} mm")
}

fn number(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

fn lines(made: &Value) -> Vec<String> {
    made["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap().to_string())
        .collect()
}

/// The base's bottom edge and the leg's lines (on the edge, the slanted
/// end, the far line, the other end).
fn sketches(s: &mut Session, k: f64) -> (String, Vec<String>) {
    run(s, "sketch.create", json!({"plane": "xy", "name": "Base"}));
    let base = lines(&run(
        s,
        "sketch.rect",
        json!({"sketch": "Base", "from": [0.0, 0.0], "to": [100.0 * k, 30.0 * k]}),
    ));
    run(s, "sketch.create", json!({"plane": "xy", "name": "Leg"}));
    let points: Vec<[f64; 2]> = [[10.0, 0.0], [90.0, 0.0], [80.0, -20.0], [20.0, -20.0]]
        .iter()
        .map(|p| [p[0] * k, p[1] * k])
        .collect();
    let leg = lines(&run(
        s,
        "sketch.polyline",
        json!({"sketch": "Leg", "points": points, "closed": true}),
    ));
    (base[0].clone(), leg)
}

fn rail(s: &mut Session, k: f64, flanges: Vec<Value>, relief: Value) {
    built(
        s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "name": "Rail", "thickness": mm(k), "radius": mm(k),
               "kFactor": 0.5, "flanges": flanges, "bendRelief": relief}),
    );
}

fn names(s: &mut Session) -> Vec<String> {
    body(s, "Rail")["faceNames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn a_lip_on_the_far_line_of_an_outlined_leg() {
    for k in SCALES {
        let dir = workdir("sheetmetal-outline-lines", &format!("rail-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let (edge, leg) = sketches(&mut s, k);
        let leg_flange = json!({"edge": edge, "outline": "Leg", "angle": "90 deg"});
        rail(&mut s, k, vec![leg_flange.clone()], Value::Null);
        let before = names(&mut s);
        let lip = json!({"line": {"flange": 0, "edge": leg[2]}, "length": mm(8.0 * k),
                         "angle": "90 deg", "insetStart": mm(5.0 * k), "insetEnd": mm(5.0 * k)});
        let ba = FRAC_PI_2 * 1.5 * k;
        let plain = 3000.0 * k * k + 1400.0 * k * k + 80.0 * k * ba;
        for (relief, cut) in [
            (Value::Null, 0.0),
            (
                json!({"shape": "rect", "size": mm(1.5 * k)}),
                2.0 * 2.25 * k * k,
            ),
        ] {
            run(
                &mut s,
                "sheetMetal.edit",
                json!({"feature": "Rail", "flanges": [leg_flange.clone(), lip.clone()],
                       "bendRelief": relief}),
            );
            let found = body(&mut s, "Rail");
            assert_eq!(found["valid"], true, "scale {k}: {found}");
            let table = run(&mut s, "sheetMetal.bends", json!({"feature": "Rail"}));
            let a = number(&table["flatArea"]);
            let want = plain + 50.0 * k * ba + 400.0 * k * k - cut;
            assert!(
                (a - want).abs() <= 1e-9 * want,
                "scale {k}: flat area {a} against {want}"
            );
            let v = number(&found["volume"]);
            assert!(
                (v - a * k).abs() <= 1e-9 * v,
                "scale {k}: volume {v} against {}",
                a * k
            );
            let bends = table["bends"].as_array().unwrap();
            assert_eq!(bends[1]["on"], format!("{} of flange 0's outline", leg[2]));
            assert!((number(&bends[1]["length"]) - 50.0 * k).abs() <= 1e-9 * k * 100.0);
            assert_eq!(bends[1]["ends"].as_array().unwrap().len(), 2, "{table}");
            // The leg's names are kept but the rim along its far line.
            let after = names(&mut s);
            let lost: Vec<&String> = before.iter().filter(|n| !after.contains(n)).collect();
            assert!(
                lost.len() <= 1 && lost.iter().all(|n| n.contains("rim(")),
                "scale {k}: lost {lost:?}"
            );
        }
        // Saved and opened again: the same part (the lip's `line` and the
        // bend relief are in the file).
        let kept = body(&mut s, "Rail");
        run(
            &mut s,
            "file.save",
            json!({"path": "rail.3dcraft", "overwrite": true}),
        );
        run(&mut s, "file.open", json!({"path": "rail.3dcraft"}));
        let again = body(&mut s, "Rail");
        assert_eq!(again["volume"], kept["volume"], "scale {k}");
        assert_eq!(again["faceNames"], kept["faceNames"], "scale {k}");
        // The leg's line on the edge is its own bend's tangent line: the
        // part fails with the reason.
        let bad = json!({"line": {"flange": 0, "edge": leg[0]}, "length": mm(8.0 * k),
                         "angle": "90 deg"});
        let out = run(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Rail", "flanges": [leg_flange.clone(), bad]}),
        );
        assert_eq!(out["status"], "failed", "{out}");
        assert!(
            out["error"].as_str().unwrap().contains("tangent line"),
            "{out}"
        );
        run(&mut s, "edit.undo", json!({}));
        // Lines that name nothing a flange can sit on.
        for (on, why) in [
            (json!({"flange": 0, "edge": "e99"}), "has no e99"),
            (json!({"flange": 1, "edge": leg[2]}), "earlier flange"),
            (json!({"flange": 0}), "line is"),
        ] {
            let bad = json!({"line": on, "length": mm(8.0 * k), "angle": "90 deg"});
            let got = error(
                &mut s,
                "sheetMetal.edit",
                json!({"feature": "Rail", "flanges": [leg_flange.clone(), bad]}),
            );
            assert!(got.contains(why), "{on}: {got}");
        }
        // On a flange without an outline.
        let got = error(
            &mut s,
            "feature.sheetMetal",
            json!({"sketch": "Base", "name": "Plain", "thickness": mm(k), "radius": mm(k),
                   "kFactor": 0.5, "flanges": [
                       {"edge": edge, "length": mm(10.0 * k), "angle": "90 deg"},
                       {"line": {"flange": 0, "edge": leg[2]}, "length": mm(8.0 * k),
                        "angle": "90 deg"}]}),
        );
        assert!(got.contains("no outline of its own"), "{got}");
    }
}
