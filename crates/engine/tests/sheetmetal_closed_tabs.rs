//! Closed corners of a flange that carries flanges
//! (docs/design/sheetmetal.md › *Closed corners*): an enclosure's tray
//! (60 × 40, walls 10 high on all four edges, R = T = 1, K = ½, round
//! reliefs of 3, every corner closed by overlap with a gap of 0.25) with
//! two screw tabs inset on the bottom wall's tip. Until 2026-10-29 such a
//! wall's corners could not be closed ("not supported"). Held to closed
//! forms at 1e-3, 1 and 1e6 against the same tray without the tabs: the
//! flat area grows by `2 w (BA + L)`, the volume by `2 (T w L + θ w ((R +
//! T)² − R²)/2)`, the gap at every corner is still the gap, every face
//! name of the tray without tabs but the tip's rim is kept; a flange on a
//! side its closed corner replaces moves with the corner's end; with bend reliefs
//! the tabs' four notches come off the flat area.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;

use common::checked::{body, built, new_document, run, session_in, workdir};
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

/// The tray's sketch: its lines (bottom, right, top, left).
fn base(s: &mut Session, k: f64) -> Vec<String> {
    run(s, "sketch.create", json!({"plane": "xy", "name": "Base"}));
    let made = run(
        s,
        "sketch.rect",
        json!({"sketch": "Base", "from": [0.0, 0.0], "to": [60.0 * k, 40.0 * k]}),
    );
    made["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap().to_string())
        .collect()
}

/// The walls, and with `tabs` two screw tabs 10 wide and 6 long on the
/// bottom wall's tip (54 long between its reliefs: insets 8 and 36).
fn flanges(k: f64, lines: &[String], tabs: bool) -> Vec<Value> {
    let mut out: Vec<Value> = lines
        .iter()
        .map(|e| json!({"edge": e, "length": mm(10.0 * k), "angle": "90 deg"}))
        .collect();
    if tabs {
        for (a, b) in [(8.0, 36.0), (36.0, 8.0)] {
            out.push(json!({"tip": 0, "length": mm(6.0 * k), "angle": "90 deg",
                            "insetStart": mm(a * k), "insetEnd": mm(b * k)}));
        }
    }
    out
}

fn tray(s: &mut Session, k: f64, flanges: Vec<Value>) {
    built(
        s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "name": "Tray", "thickness": mm(k), "radius": mm(k),
               "kFactor": 0.5, "relief": {"shape": "round", "size": mm(3.0 * k)},
               "corner": {"kind": "overlap", "gap": mm(0.25 * k)}, "flanges": flanges}),
    );
}

fn names(s: &mut Session) -> Vec<String> {
    body(s, "Tray")["faceNames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap().to_string())
        .collect()
}

fn flat_area(s: &mut Session) -> f64 {
    number(&run(s, "sheetMetal.bends", json!({"feature": "Tray"}))["flatArea"])
}

#[test]
fn a_closed_corner_of_a_wall_with_screw_tabs_on_its_tip() {
    for k in SCALES {
        let dir = workdir("sheetmetal-closed-tabs", &format!("tabs-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let lines = base(&mut s, k);
        tray(&mut s, k, flanges(k, &lines, false));
        let plain = body(&mut s, "Tray");
        assert_eq!(plain["valid"], true, "{plain}");
        let (v0, a0) = (number(&plain["volume"]), flat_area(&mut s));
        let before = names(&mut s);

        run(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Tray", "flanges": flanges(k, &lines, true)}),
        );
        let found = body(&mut s, "Tray");
        assert_eq!(found["valid"], true, "scale {k}: {found}");
        let (t, r, w, l) = (k, k, 10.0 * k, 6.0 * k);
        let ba = FRAC_PI_2 * (r + 0.5 * t);
        let a = flat_area(&mut s);
        let grown = 2.0 * w * (ba + l);
        assert!(
            (a - a0 - grown).abs() <= 1e-9 * a0,
            "scale {k}: flat area {a} against {} + {grown}",
            a0
        );
        let ring = FRAC_PI_2 * w * ((r + t).ppowi(2) - r * r) / 2.0;
        let v = number(&found["volume"]);
        let more = 2.0 * (t * w * l + ring);
        assert!(
            (v - v0 - more).abs() <= 1e-9 * v0,
            "scale {k}: volume {v} against {v0} + {more}"
        );
        // At K = ½ and square ends, the volume is the flat area times T.
        assert!(
            (v - a * t).abs() <= 1e-9 * v,
            "scale {k}: {v} against {}",
            a * t
        );
        // The gap at the corners is still the gap.
        run(
            &mut s,
            "assert.add",
            json!({"name": "Gap", "measure": "cornerGap", "of": "Tray", "eq": mm(0.25 * k)}),
        );
        let all = run(&mut s, "assert.list", json!({}));
        let gap = all["assertions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["name"] == "Gap")
            .unwrap()
            .clone();
        assert_eq!(gap["status"], "passed", "scale {k}: {gap}");
        // Every name of the tray without tabs is kept but the rim along
        // the bottom wall's tip, which the tabs split.
        let after = names(&mut s);
        let lost: Vec<&String> = before.iter().filter(|n| !after.contains(n)).collect();
        assert!(
            lost.len() <= 1 && lost.iter().all(|n| n.contains("rim(")),
            "scale {k}: lost {lost:?}"
        );
        // The bend table lists the tabs with their ids and lengths.
        let table = run(&mut s, "sheetMetal.bends", json!({"feature": "Tray"}));
        let bends = table["bends"].as_array().unwrap();
        assert_eq!(bends.len(), 6, "{table}");
        for tab in &bends[4..] {
            assert!((number(&tab["length"]) - w).abs() <= 1e-9 * w, "{tab}");
        }
        for (wall, len) in [(0, 54.0), (1, 34.0), (2, 54.0), (3, 34.0)] {
            let got = number(&bends[wall]["length"]);
            assert!((got - len * k).abs() <= 1e-9 * k * 60.0, "{wall}: {got}");
        }
    }
}

/// A tab on the bottom wall's side at its start, where the corner is
/// closed by overlap with the bottom wall covering: the side moves to the
/// left wall's outside with the tab (until 2026-11-02 refused: the side
/// became the corner's end). The covering swaps nothing in the flat area
/// (each wall's reach is the other's of before), so against the same
/// tray without the tab the flat area grows by the tab's `w (BA + L)`
/// and, at K = ½ with square ends, the volume by `T` times that.
#[test]
fn a_flange_on_a_side_moves_with_its_closed_corners_end() {
    for k in SCALES {
        let dir = workdir("sheetmetal-closed-tabs", &format!("side-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let lines = base(&mut s, k);
        let part = |list: Vec<Value>| {
            json!({"sketch": "Base", "name": "Tray", "thickness": mm(k), "radius": mm(k),
                   "kFactor": 0.5, "relief": {"shape": "round", "size": mm(3.0 * k)},
                   "corner": {"kind": "overlap", "gap": mm(0.25 * k)},
                   "corners": [{"flanges": [3, 0], "kind": "overlap", "over": 0}],
                   "flanges": list})
        };
        built(
            &mut s,
            "feature.sheetMetal",
            part(flanges(k, &lines, false)),
        );
        let plain = body(&mut s, "Tray");
        assert_eq!(plain["valid"], true, "{plain}");
        let (v0, a0) = (number(&plain["volume"]), flat_area(&mut s));
        let mut list = flanges(k, &lines, false);
        list.push(
            json!({"side": {"flange": 0, "end": "start"}, "length": mm(5.0 * k),
                         "angle": "90 deg"}),
        );
        let made = run(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Tray", "flanges": list}),
        );
        assert_ne!(made["status"], "failed", "scale {k}: {made}");
        let tabbed = body(&mut s, "Tray");
        assert_eq!(tabbed["valid"], true, "{tabbed}");
        let grow = 10.0 * k * (FRAC_PI_2 * 1.5 * k + 5.0 * k);
        let a1 = flat_area(&mut s);
        assert!(
            (a1 - a0 - grow).abs() <= 1e-9 * a1,
            "scale {k}: {a1} - {a0}"
        );
        let v1 = number(&tabbed["volume"]);
        assert!(
            (v1 - v0 - k * grow).abs() <= 1e-8 * v1,
            "scale {k}: {v1} - {v0}"
        );
    }
}

#[test]
fn screw_tabs_with_bend_reliefs_at_closed_corners() {
    for k in SCALES {
        let dir = workdir("sheetmetal-closed-tabs", &format!("reliefs-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let lines = base(&mut s, k);
        tray(&mut s, k, flanges(k, &lines, true));
        let a0 = flat_area(&mut s);
        let rs = 1.5 * k;
        run(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Tray", "bendRelief": {"shape": "round", "size": mm(rs)}}),
        );
        let found = body(&mut s, "Tray");
        assert_eq!(found["valid"], true, "scale {k}: {found}");
        let a = flat_area(&mut s);
        let cut = 4.0 * PI * rs * rs / 4.0;
        assert!(
            (a0 - a - cut).abs() <= 1e-9 * a0,
            "scale {k}: {a0} - {a} against {cut}"
        );
        let table = run(&mut s, "sheetMetal.bends", json!({"feature": "Tray"}));
        for tab in &table["bends"].as_array().unwrap()[4..] {
            let ends = tab["ends"].as_array().unwrap();
            assert_eq!(ends.len(), 2, "{tab}");
            assert!(ends.iter().all(|e| e["relief"] == "round"), "{tab}");
        }
        // Every corner and every bend end inside its edge has a relief.
        run(
            &mut s,
            "assert.add",
            json!({"name": "Reliefs", "measure": "relief", "of": "Tray", "is": true}),
        );
        let all = run(&mut s, "assert.list", json!({}));
        let check = all["assertions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["name"] == "Reliefs")
            .unwrap()
            .clone();
        assert_eq!(check["status"], "passed", "scale {k}: {check}");
        // Without them, the measure says so.
        run(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Tray", "bendRelief": null}),
        );
        let all = run(&mut s, "assert.list", json!({}));
        let check = all["assertions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["name"] == "Reliefs")
            .unwrap()
            .clone();
        assert_eq!(check["status"], "failed", "scale {k}: {check}");
    }
}
