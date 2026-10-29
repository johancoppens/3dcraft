//! Bend reliefs and closed corners at the edges of what they cover
//! (docs/design/sheetmetal.md › *Bend reliefs*, *Closed corners*), at
//! 1e-3, 1 and 1e6 (review 2026-10-29):
//!
//! - a tab bent from a hole's edge (a base 100 × 60 with a hole 40 × 20,
//!   the tab 20 wide and 10 long on the hole's bottom edge, R = T = 1,
//!   K = ½): both ends of its bend lie inside the edge, so with bend
//!   reliefs two notches come off the flat area (`r²` each for rect,
//!   `π r²/4` for round), the flat area without them is `6000 − 800 +
//!   20 (BA + 10)`, the volume is the flat area times T, the `relief`
//!   measure passes with them and fails without, and lists each end by
//!   its flange's place (`bend`) and id (`id`);
//! - a closed corner of a wall whose tip carries a tab spanning the tip
//!   to its end at that corner;
//! - a bend relief's size of the wrong kind is an error naming
//!   `bendRelief.size`.

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
use std::f64::consts::{FRAC_PI_2, PI};

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

fn flat_area(s: &mut Session, name: &str) -> f64 {
    number(&run(s, "sheetMetal.bends", json!({"feature": name}))["flatArea"])
}

fn assertion(s: &mut Session, name: &str) -> Value {
    let all = run(s, "assert.list", json!({"details": true}));
    all["assertions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["name"] == name)
        .unwrap_or_else(|| panic!("no assertion {name}: {all}"))
        .clone()
}

#[test]
fn a_tab_on_a_holes_edge_takes_bend_reliefs() {
    for k in SCALES {
        let dir = workdir("sheetmetal-bend-relief-cases", &format!("hole-{k}"));
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
        let hole = lines(&run(
            &mut s,
            "sketch.rect",
            json!({"sketch": "Base", "from": [30.0 * k, 20.0 * k], "to": [70.0 * k, 40.0 * k]}),
        ));
        // The hole's bottom line (y = 20, x from 30 to 70): the tab grows
        // into the hole, its bend 20 long between insets of 10.
        let tab = json!({"edge": hole[0], "length": mm(10.0 * k), "angle": "90 deg",
                         "insetStart": mm(10.0 * k), "insetEnd": mm(10.0 * k)});
        built(
            &mut s,
            "feature.sheetMetal",
            json!({"sketch": "Base", "name": "Plate", "thickness": mm(k), "radius": mm(k),
                   "kFactor": 0.5, "flanges": [tab]}),
        );
        let (t, r, w, l) = (k, k, 20.0 * k, 10.0 * k);
        let ba = FRAC_PI_2 * (r + 0.5 * t);
        let a0 = flat_area(&mut s, "Plate");
        let expected = (6000.0 - 800.0) * k * k + w * (ba + l);
        assert!(
            (a0 - expected).abs() <= 1e-9 * expected,
            "scale {k}: flat area {a0} against {expected}"
        );
        run(
            &mut s,
            "assert.add",
            json!({"name": "Reliefs", "measure": "relief", "of": "Plate", "is": true}),
        );
        let without = assertion(&mut s, "Reliefs");
        assert_eq!(without["status"], "failed", "scale {k}: {without}");
        let table = run(&mut s, "sheetMetal.bends", json!({"feature": "Plate"}));
        let ends = table["bends"][0]["ends"].as_array().unwrap();
        assert_eq!(ends.len(), 2, "scale {k}: {table}");
        assert!(ends.iter().all(|e| e["relief"].is_null()), "{table}");
        let id = table["bends"][0]["id"].clone();
        let listed = without["detail"]["corners"].as_array().unwrap();
        let at_ends: Vec<&Value> = listed.iter().filter(|c| c.get("end").is_some()).collect();
        assert_eq!(at_ends.len(), 2, "{without}");
        for e in at_ends {
            assert_eq!(e["bend"], 0, "{e}");
            assert_eq!(e["id"], id, "{e}");
        }

        for (shape, notch) in [("rect", 1.0), ("round", PI / 4.0)] {
            let rs = 1.5 * k;
            run(
                &mut s,
                "sheetMetal.edit",
                json!({"feature": "Plate", "bendRelief": {"shape": shape, "size": mm(rs)}}),
            );
            let found = body(&mut s, "Plate");
            assert_eq!(found["valid"], true, "scale {k} {shape}: {found}");
            let a = flat_area(&mut s, "Plate");
            let cut = 2.0 * notch * rs * rs;
            assert!(
                (a0 - a - cut).abs() <= 1e-9 * a0,
                "scale {k} {shape}: {a0} - {a} against {cut}"
            );
            // At K = ½ and square ends, the volume is the flat area
            // times T.
            let v = number(&found["volume"]);
            assert!(
                (v - a * t).abs() <= 1e-9 * v,
                "scale {k} {shape}: {v} against {}",
                a * t
            );
            let with = assertion(&mut s, "Reliefs");
            assert_eq!(with["status"], "passed", "scale {k} {shape}: {with}");
        }
    }
}

/// The tray of `sheetmetal_closed_tabs.rs` (60 × 40, walls 10 on every
/// edge, every corner closed by overlap) with one tab on the bottom wall's
/// tip that spans the tip from its start (no inset there) to 36 short of
/// its end: the tab reaches the corner the bottom wall's closed corner
/// runs on past.
#[test]
fn a_closed_corner_whose_tip_flange_spans_the_tips_end() {
    for k in SCALES {
        let dir = workdir("sheetmetal-bend-relief-cases", &format!("tip-end-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "xy", "name": "Base"}),
        );
        let edges = lines(&run(
            &mut s,
            "sketch.rect",
            json!({"sketch": "Base", "from": [0.0, 0.0], "to": [60.0 * k, 40.0 * k]}),
        ));
        let mut flanges: Vec<Value> = edges
            .iter()
            .map(|e| json!({"edge": e, "length": mm(10.0 * k), "angle": "90 deg"}))
            .collect();
        flanges.push(json!({"tip": 0, "length": mm(6.0 * k), "angle": "90 deg",
                            "insetEnd": mm(36.0 * k)}));
        let made = run(
            &mut s,
            "feature.sheetMetal",
            json!({"sketch": "Base", "name": "Tray", "thickness": mm(k), "radius": mm(k),
                   "kFactor": 0.5, "relief": {"shape": "round", "size": mm(3.0 * k)},
                   "corner": {"kind": "overlap", "gap": mm(0.25 * k)}, "flanges": flanges}),
        );
        eprintln!("scale {k}: tip flange to the tip's end: {made}");
        if made["status"] == "ok" {
            let found = body(&mut s, "Tray");
            assert_eq!(found["valid"], true, "scale {k}: {found}");
            let a = flat_area(&mut s, "Tray");
            let v = number(&found["volume"]);
            assert!(
                (v - a * k).abs() <= 1e-9 * v,
                "scale {k}: {v} against {}",
                a * k
            );
            let table = run(&mut s, "sheetMetal.bends", json!({"feature": "Tray"}));
            eprintln!("scale {k}: bends {}", table["bends"][4]);
        } else {
            assert_eq!(made["status"], "failed", "scale {k}: {made}");
            let why = made["error"].as_str().unwrap_or_default();
            assert!(!why.is_empty() && !why.contains("internal"), "{made}");
        }
    }
}

#[test]
fn a_bend_reliefs_size_of_the_wrong_kind_names_it() {
    let k = 1.0;
    let dir = workdir("sheetmetal-bend-relief-cases", "bad-size");
    let mut s = session_in(&dir);
    new_document(&mut s, k);
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "name": "Base"}),
    );
    let edges = lines(&run(
        &mut s,
        "sketch.rect",
        json!({"sketch": "Base", "from": [0.0, 0.0], "to": [60.0, 40.0]}),
    ));
    for size in [json!(true), json!([1]), json!({"x": 1})] {
        let got = error(
            &mut s,
            "feature.sheetMetal",
            json!({"sketch": "Base", "thickness": "1 mm", "radius": "1 mm", "kFactor": 0.5,
                   "bendRelief": {"shape": "round", "size": size},
                   "flanges": [{"edge": edges[0], "length": "10 mm", "angle": "90 deg",
                                "insetStart": "10 mm"}]}),
        );
        assert!(got.contains("bendRelief.size"), "{size}: {got}");
    }
}
