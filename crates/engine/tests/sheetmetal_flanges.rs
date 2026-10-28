//! Inset flanges, flanges with an outline of their own, on a flange's side
//! and on a hole's edge, through the commands
//! (docs/design/sheetmetal.md › *Inset flanges, outlines, sides and
//! holes*): a panel with an inset wall, a tab on the wall's side, an arch
//! drawn in a sketch of its own and a tab bent down from a hole's edge,
//! held to its volume and flat area in closed form at three scales, its
//! bend table, saved and reopened to the same body and names, its flat
//! pattern exported; names that stay when another flange is added; bad
//! parameters as errors and bad flanges failing the part with the reason.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::assert_close;
use common::checked::{body, built, error, failed, new_document, run, session_in, workdir};
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::f64::consts::{FRAC_PI_2, PI};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn lines_of(made: &Value) -> Vec<String> {
    made["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap().to_string())
        .collect()
}

/// The panel's sketches: a 60 × 40 base with a 20 × 15 hole (its lines
/// bottom, right, top, left; the hole's after), and the arch, a 20 × 12
/// rectangle on the base's top edge from x = 20 with a round hole of
/// radius 2.
fn sketches(s: &mut Session, k: f64) -> (Vec<String>, Vec<String>) {
    run(s, "sketch.create", json!({"plane": "xy", "name": "Base"}));
    let base = lines_of(&run(
        s,
        "sketch.rect",
        json!({"sketch": "Base", "from": [0.0, 0.0], "to": [60.0 * k, 40.0 * k]}),
    ));
    let hole = lines_of(&run(
        s,
        "sketch.rect",
        json!({"sketch": "Base", "from": [20.0 * k, 10.0 * k], "to": [40.0 * k, 25.0 * k]}),
    ));
    run(s, "sketch.create", json!({"plane": "xy", "name": "Arch"}));
    run(
        s,
        "sketch.rect",
        json!({"sketch": "Arch", "from": [20.0 * k, 40.0 * k], "to": [40.0 * k, 52.0 * k]}),
    );
    run(
        s,
        "sketch.circle",
        json!({"sketch": "Arch", "center": [30.0 * k, 46.0 * k], "radius": 2.0 * k}),
    );
    (base, hole)
}

fn mm(v: f64) -> String {
    format!("{v} mm")
}

/// The flanges: 0 the wall on the bottom edge, 5 in at both ends, 10
/// long; 1 a tab 6 long on the wall's end side; 2 the arch; 3 a tab 8
/// long bent down from the hole's bottom edge, 2 in at both ends.
fn flanges(k: f64, base: &[String], hole: &[String]) -> Vec<Value> {
    vec![
        json!({"edge": base[0], "length": mm(10.0 * k), "angle": "90 deg",
               "insetStart": mm(5.0 * k), "insetEnd": mm(5.0 * k)}),
        json!({"side": {"flange": 0, "end": "end"}, "length": mm(6.0 * k), "angle": "90 deg"}),
        json!({"edge": base[2], "outline": "Arch", "angle": "90 deg"}),
        json!({"edge": hole[0], "length": mm(8.0 * k), "angle": "90 deg", "direction": "down",
               "insetStart": mm(2.0 * k), "insetEnd": mm(2.0 * k)}),
    ]
}

fn panel(s: &mut Session, k: f64, flanges: Vec<Value>) -> Value {
    built(
        s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "name": "Panel", "thickness": mm(k), "radius": mm(k),
               "kFactor": 0.5, "flanges": flanges}),
    )
}

fn names(s: &mut Session) -> Vec<String> {
    body(s, "Panel")["faceNames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn a_panel_of_every_kind_of_flange_by_its_closed_forms() {
    for k in SCALES {
        let dir = workdir("sheetmetal-flanges", &format!("panel-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let (base, hole) = sketches(&mut s, k);
        panel(&mut s, k, flanges(k, &base, &hole));
        let (t, r) = (k, k);
        let ba = FRAC_PI_2 * (r + 0.5 * t);
        // Bent widths: the wall 50, its side 10 (the wall's length), the
        // arch 20, the hole's tab 16.
        let widths = [50.0 * k, 10.0 * k, 20.0 * k, 16.0 * k];
        let arch = (20.0 * 12.0 - PI * 4.0) * k * k;
        let flats = 60.0 * 40.0 * k * k - 20.0 * 15.0 * k * k
            + 50.0 * 10.0 * k * k
            + 10.0 * 6.0 * k * k
            + arch
            + 16.0 * 8.0 * k * k;
        let strips: f64 = widths.iter().map(|w| w * ba).sum();
        let rings: f64 = widths
            .iter()
            .map(|w| FRAC_PI_2 * w * ((r + t).powi(2) - r * r) / 2.0)
            .sum();
        // At K = 1/2 the volume is the flat area times T.
        let made = body(&mut s, "Panel");
        assert_close(&made["volume"], t * flats + rings, "volume");
        assert_close(&made["volume"], t * (flats + strips), "volume = A T");
        let table = run(&mut s, "sheetMetal.bends", json!({"feature": "Panel"}));
        assert_close(&table["flatArea"], flats + strips, "flat area");
        let on: Vec<&str> = table["bends"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["on"].as_str().unwrap())
            .collect();
        assert_eq!(on[1], "end side of flange 0", "{table}");
        let out = run(
            &mut s,
            "file.export",
            json!({"path": "panel.svg", "sheetMetal": "Panel"}),
        );
        assert_close(&out["flatArea"], flats + strips, "exported flat area");
        run(&mut s, "file.save", json!({"path": "panel.3dcraft"}));
        let mut u = session_in(&dir);
        run(&mut u, "file.open", json!({"path": "panel.3dcraft"}));
        let again = body(&mut u, "Panel");
        assert_eq!(made["volume"], again["volume"]);
        assert_eq!(made["faceNames"], again["faceNames"]);
    }
}

/// Rule 4: a flange added later changes no name the part had, but the
/// rim along the edge it is bent from: adding the tab on the wall's side
/// takes the side's rim (that edge is a bend's tangent line now, no end of
/// the part) and nothing else; adding the arch or the hole's tab, inset on
/// its edge, takes that edge's rim (its pieces are named by the bends at
/// their ends) and nothing else.
#[test]
fn names_stay_when_another_flange_is_added() {
    let k = 1.0;
    let dir = workdir("sheetmetal-flanges", "names");
    let mut s = session_in(&dir);
    new_document(&mut s, k);
    let (base, hole) = sketches(&mut s, k);
    let all = flanges(k, &base, &hole);
    panel(&mut s, k, vec![all[0].clone()]);
    let mut before = names(&mut s);
    for upto in [2, 3, 4] {
        let list: Vec<Value> = all.iter().take(upto).cloned().collect();
        run(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Panel", "flanges": list}),
        );
        let after = names(&mut s);
        let lost: Vec<&String> = before.iter().filter(|n| !after.contains(n)).collect();
        assert!(
            lost.len() <= 1 && lost.iter().all(|n| n.contains("rim(")),
            "adding flange {}: lost {lost:?}",
            upto - 1
        );
        before = after;
    }
}

/// Where each rim of the panel lies: its edges' total length and their
/// length-weighted middle, by the rim's name.
fn rims(s: &mut Session) -> std::collections::BTreeMap<String, [f64; 4]> {
    let edges = run(s, "document.edges", json!({"feature": "Panel"}));
    let mut out = std::collections::BTreeMap::new();
    for e in edges["edges"].as_array().unwrap() {
        let len = e["length"].as_f64().unwrap();
        let mid: Vec<f64> = (0..3)
            .map(|i| 0.5 * (e["start"][i].as_f64().unwrap() + e["end"][i].as_f64().unwrap()))
            .collect();
        for f in e["faces"].as_array().unwrap() {
            let f = f.as_str().unwrap();
            if !f.contains("rim(") {
                continue;
            }
            let a: &mut [f64; 4] = out.entry(f.to_string()).or_insert([0.0; 4]);
            a[0] += len;
            for i in 0..3 {
                a[i + 1] += len * mid[i];
            }
        }
    }
    for a in out.values_mut() {
        for i in 1..4 {
            a[i] /= a[0];
        }
    }
    out
}

/// Every rim name in both lists lies in the same place in both.
fn same_places(
    before: &std::collections::BTreeMap<String, [f64; 4]>,
    after: &std::collections::BTreeMap<String, [f64; 4]>,
    k: f64,
    what: &str,
) {
    for (name, a) in before {
        if let Some(b) = after.get(name) {
            let off = a
                .iter()
                .zip(b)
                .map(|(x, y)| (x - y).abs())
                .fold(0.0, f64::max);
            assert!(
                off <= 1e-6 * k,
                "{what} at scale {k}: {name} moved from {a:?} to {b:?}"
            );
        }
    }
}

/// Rule 4 on one edge: a second inset flange added on the edge the arch
/// sits on, ahead of the arch along it, takes only the rim of the piece
/// it splits; every rim name that survives (the piece from the edge's
/// start, the piece past the arch, every other rim) lies where it lay;
/// deleting the flange again gives back every name in its place.
#[test]
fn names_stay_in_place_when_a_flange_is_added_on_the_same_edge() {
    for k in SCALES {
        let dir = workdir("sheetmetal-flanges", &format!("same-edge-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let (base, hole) = sketches(&mut s, k);
        let mut all = flanges(k, &base, &hole);
        panel(&mut s, k, all.clone());
        let first = rims(&mut s);
        // The top edge runs from x = 60 to x = 0; the arch is at 40..20,
        // the new tab at 55..45, ahead of it.
        all.push(
            json!({"edge": base[2], "length": mm(10.0 * k), "angle": "90 deg",
                        "insetStart": mm(5.0 * k), "insetEnd": mm(45.0 * k)}),
        );
        run(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Panel", "flanges": all}),
        );
        let added = rims(&mut s);
        let lost: Vec<&String> = first.keys().filter(|n| !added.contains_key(*n)).collect();
        assert_eq!(lost.len(), 1, "adding the tab at scale {k}: lost {lost:?}");
        same_places(&first, &added, k, "adding the tab");
        all.pop();
        run(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Panel", "flanges": all}),
        );
        let deleted = rims(&mut s);
        assert_eq!(
            first.keys().collect::<Vec<_>>(),
            deleted.keys().collect::<Vec<_>>(),
            "deleting the tab at scale {k}"
        );
        same_places(&first, &deleted, k, "deleting the tab");
        same_places(&added, &deleted, k, "deleting the tab");
    }
}

#[test]
fn bad_flanges_are_errors_or_fail_the_part() {
    let k = 1.0;
    let dir = workdir("sheetmetal-flanges", "bad");
    let mut s = session_in(&dir);
    new_document(&mut s, k);
    let (base, hole) = sketches(&mut s, k);
    let e = &base[0];
    let make = |flanges: Value| {
        json!({"sketch": "Base", "name": "Bad", "thickness": "1 mm", "radius": "1 mm",
               "kFactor": 0.5, "flanges": flanges})
    };
    for (flanges, why) in [
        (
            json!([{"edge": e, "length": "5 mm", "angle": "90 deg", "tip": 0}]),
            "give one of them",
        ),
        (
            json!([{"edge": e, "length": "5 mm", "angle": "90 deg"},
                   {"side": {"flange": 0, "end": "middle"}, "length": "5 mm", "angle": "90 deg"}]),
            "start or end",
        ),
        (
            json!([{"edge": e, "length": "5 mm", "angle": "90 deg"},
                   {"side": {"flange": 3, "end": "end"}, "length": "5 mm", "angle": "90 deg"}]),
            "earlier flange",
        ),
        (
            json!([{"edge": e, "outline": "Nowhere", "angle": "90 deg"}]),
            "names no sketch",
        ),
        (
            json!([{"edge": e, "outline": "Arch", "length": "5 mm", "angle": "90 deg"}]),
            "give no `length`",
        ),
        (
            json!([{"edge": e, "length": "5 mm", "angle": "90 deg", "insetStart": [1]}]),
            "insetStart",
        ),
    ] {
        let got = error(&mut s, "feature.sheetMetal", make(flanges));
        assert!(got.contains(why), "{why}: {got}");
    }
    for (flanges, why) in [
        (
            json!([{"edge": e, "length": "5 mm", "angle": "90 deg", "insetStart": "40 mm",
                    "insetEnd": "30 mm"}]),
            "leave nothing",
        ),
        (
            json!([{"edge": e, "length": "5 mm", "angle": "90 deg", "insetStart": "-1 mm"}]),
            "inset",
        ),
        (
            json!([{"edge": e, "outline": "Arch", "angle": "90 deg"}]),
            "no line on its edge",
        ),
        (
            json!([{"edge": hole[0], "length": "5 mm", "angle": "90 deg"}]),
            "overlaps itself",
        ),
    ] {
        let got = failed(&mut s, "feature.sheetMetal", make(flanges));
        assert!(got.contains(why), "{why}: {got}");
        run(&mut s, "edit.undo", json!({}));
    }
}

#[test]
fn the_guide_cover_runs_as_it_says() {
    let guide = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/guide/09-features.md");
    let text = std::fs::read_to_string(guide).unwrap();
    let fence = "`".repeat(3);
    let section = text
        .split("**Inset flanges, tabs and outlines.**")
        .nth(1)
        .unwrap();
    let block = section
        .split(&format!("{fence}json"))
        .nth(1)
        .unwrap()
        .split(&fence)
        .next()
        .unwrap();
    let steps: Value = serde_json::from_str(block).unwrap();
    let dir = workdir("sheetmetal-flanges", "guide");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    for step in steps.as_array().unwrap() {
        let id = step[0].as_str().unwrap();
        let result = run(&mut s, id, step[1].clone());
        assert_ne!(result["status"], "failed", "{id}: {result}");
    }
    let table = run(&mut s, "sheetMetal.bends", json!({"feature": "Cover"}));
    let ba = FRAC_PI_2 * (1.0 + 0.42);
    // The wall 80 wide, its side 15, the ear 20, the tab 14; the hole 20 × 25.
    let widths = [80.0, 15.0, 20.0, 14.0];
    let flats = 6000.0 - 500.0 + 80.0 * 15.0 + 15.0 * 10.0 + (300.0 - 9.0 * PI) + 14.0 * 12.0;
    let area = flats + widths.iter().sum::<f64>() * ba;
    assert_close(&table["flatArea"], area, "the cover's flat area");
    assert!(dir.join("cover.svg").exists());
}
