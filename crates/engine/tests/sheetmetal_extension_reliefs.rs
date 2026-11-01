//! Bend reliefs cut into a closed corner's extension, by commands
//! (docs/design/sheetmetal.md › *Bend reliefs*), at 1e-3, 1 and 1e6: the
//! spot the sheet-metal enclosure Jig stopped its return flanges short of.
//!
//! The enclosure's base (the Jig's defaults: inside 160 × 120 × 50, T = 1,
//! R = 1, K = 0.42): a floor `2a × 2b` (a = 79, b = 59), side walls 47
//! high, front and back lips 10, round corner reliefs of `s = 2.5`, every
//! corner closed by overlap with the walls over the lips, and on each
//! wall's tip a return 11.4 wide with an outline of its own holding two
//! screw holes, spanning the whole tip (`retY = b − s`, as the Jig draws
//! it for open corners). The closed corners run the walls' tips on past
//! the returns' ends, so the four ends lie inside the extended tips. Until
//! 2026-10-31 a bend relief there was refused ("not built (owed)"), and
//! the Jig stopped its returns `s + 1` shorter at each end. Now:
//!
//! - with round bend reliefs of `s` it is built and valid, the four
//!   notches (`π s²/4` each) come off the flat area, the volume is the
//!   flats' area times T plus what K ≠ ½ moves (held against the part
//!   without bend reliefs: the notches are flat, so the volume drops by
//!   `π s² T`), every face name (rims included) kept with only the
//!   notches' rims added, the `relief` measure fails without and passes with them,
//!   and the bend table lists both ends of each return with the relief;
//! - with tear bend reliefs of `s` (a circle reaching `2s = 5` along the
//!   extension, which runs on 4.5 past the return) the feature fails,
//!   naming the return, before any body is made;
//! - the press-brake check (`sheetMetal.bendSequence`) still proves a part
//!   with such a relief bendable: a wall, a lip closed against it and a
//!   return on the wall's tip, with segmented tooling.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{body, built, failure, names_of, new_document, run, session_in, workdir};
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::f64::consts::PI;

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

/// The enclosure's base at scale `k`, without bend reliefs.
fn enclosure(s: &mut Session, k: f64) {
    let (a, b, wall, lip, ret, rs) = (79.0, 59.0, 47.0, 10.0, 11.4, 2.5);
    let (ret_x, ret_y) = (a + wall, b - rs);
    let ys = ret_y - 8.0;
    run(s, "sketch.create", json!({"plane": "xy", "name": "Floor"}));
    let edges = lines(&run(
        s,
        "sketch.rect",
        json!({"sketch": "Floor", "from": [-a * k, -b * k], "to": [a * k, b * k]}),
    ));
    for (name, side) in [("ReturnRight", 1.0), ("ReturnLeft", -1.0)] {
        run(s, "sketch.create", json!({"plane": "xy", "name": name}));
        run(
            s,
            "sketch.rect",
            json!({"sketch": name, "from": [side * ret_x * k, -ret_y * k],
                   "to": [side * (ret_x + ret) * k, ret_y * k]}),
        );
        for y in [ys, -ys] {
            run(
                s,
                "sketch.circle",
                json!({"sketch": name, "center": [side * (ret_x + ret / 2.0) * k, y * k],
                       "radius": 1.7 * k}),
            );
        }
    }
    // Lines: bottom, right, top, left.
    let flange = |e: &String, l: f64| json!({"edge": e, "length": mm(l * k), "angle": "90 deg"});
    built(
        s,
        "feature.sheetMetal",
        json!({"sketch": "Floor", "name": "Base", "thickness": mm(k), "radius": mm(k),
               "kFactor": 0.42, "relief": {"shape": "round", "size": mm(rs * k)},
               "corner": {"kind": "overlap", "gap": mm(0.5 * k)},
               "corners": [{"flanges": [0, 2], "kind": "overlap", "over": 0},
                           {"flanges": [0, 3], "kind": "overlap", "over": 0},
                           {"flanges": [1, 2], "kind": "overlap", "over": 1},
                           {"flanges": [1, 3], "kind": "overlap", "over": 1}],
               "flanges": [flange(&edges[1], wall), flange(&edges[3], wall),
                           flange(&edges[0], lip), flange(&edges[2], lip),
                           {"tip": 0, "outline": "ReturnRight", "angle": "90 deg"},
                           {"tip": 1, "outline": "ReturnLeft", "angle": "90 deg"}]}),
    );
}

#[test]
fn the_enclosures_returns_span_the_tip_with_bend_reliefs() {
    for k in SCALES {
        let dir = workdir("sheetmetal-extension-reliefs", &format!("enclosure-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        enclosure(&mut s, k);
        let rs = 2.5 * k;
        let plain = body(&mut s, "Base");
        assert_eq!(plain["valid"], true, "scale {k}: {plain}");
        let (a0, v0) = (flat_area(&mut s, "Base"), number(&plain["volume"]));
        let names0 = names_of(&plain);
        run(
            &mut s,
            "assert.add",
            json!({"name": "Reliefs", "measure": "relief", "of": "Base", "is": true}),
        );
        assert_eq!(
            assertion(&mut s, "Reliefs")["status"],
            "failed",
            "scale {k}"
        );

        built(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Base", "bendRelief": {"shape": "round", "size": mm(rs)}}),
        );
        let found = body(&mut s, "Base");
        assert_eq!(found["valid"], true, "scale {k}: {found}");
        // Every face name of the part without the reliefs, rims included,
        // is kept; the reliefs add only the notches' rims.
        let names1 = names_of(&found);
        let lost: Vec<&String> = names0.iter().filter(|n| !names1.contains(n)).collect();
        assert!(lost.is_empty(), "scale {k}: lost {lost:?}");
        let added: Vec<&String> = names1.iter().filter(|n| !names0.contains(n)).collect();
        assert!(!added.is_empty(), "scale {k}: no notch rims");
        assert!(
            added.iter().all(|n| n.contains("rim(")),
            "scale {k}: added {added:?}"
        );
        let a = flat_area(&mut s, "Base");
        let notches = PI * rs * rs;
        assert!(
            (a0 - a - notches).abs() <= 1e-9 * a0,
            "scale {k}: {a0} - {a} against {notches}"
        );
        let v = number(&found["volume"]);
        assert!(
            (v0 - v - notches * k).abs() <= 1e-8 * v0,
            "scale {k}: {v0} - {v} against {}",
            notches * k
        );
        let with = assertion(&mut s, "Reliefs");
        assert_eq!(with["status"], "passed", "scale {k}: {with}");
        let table = run(&mut s, "sheetMetal.bends", json!({"feature": "Base"}));
        for b in [4, 5] {
            let ends = table["bends"][b]["ends"].as_array().unwrap();
            assert_eq!(ends.len(), 2, "scale {k}: {table}");
            assert!(ends.iter().all(|e| !e["relief"].is_null()), "{table}");
        }

        // A tear of the same size reaches 5 along the extension, which runs
        // on 4.5 past the return: refused, naming the return.
        let made = run(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Base", "bendRelief": {"shape": "tear", "size": mm(rs)}}),
        );
        assert_eq!(made["status"], "failed", "scale {k}: {made}");
        let why = failure(&mut s, "Base");
        assert!(
            why.contains("flanges.4") || why.contains("flanges.5"),
            "scale {k}: {why}"
        );
        assert!(
            why.contains("tear")
                && (why.contains("cuts across") || why.contains("of the extension beside it")),
            "scale {k}: {why}"
        );
    }
}

/// A tray corner: a wall on the bottom edge of a 60 × 40 floor, a lip on
/// the left edge closed against it by overlap (the wall over), a return on
/// the wall's tip spanning it, round reliefs.
#[test]
fn a_part_with_a_relief_in_an_extension_is_proven_bendable() {
    let punch = [[-4.0, 0.0], [4.0, 0.0], [4.0, 60.0], [-4.0, 60.0]];
    let die = [[-15.0, -20.0], [15.0, -20.0], [15.0, 0.0], [-15.0, 0.0]];
    let scaled = |poly: &[[f64; 2]], k: f64| -> Value {
        json!(
            poly.iter()
                .map(|p| [p[0] * k, p[1] * k])
                .collect::<Vec<_>>()
        )
    };
    for k in SCALES {
        let dir = workdir("sheetmetal-extension-reliefs", &format!("press-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "xy", "name": "Floor"}),
        );
        let edges = lines(&run(
            &mut s,
            "sketch.rect",
            json!({"sketch": "Floor", "from": [0.0, 0.0], "to": [60.0 * k, 40.0 * k]}),
        ));
        built(
            &mut s,
            "feature.sheetMetal",
            json!({"sketch": "Floor", "name": "Corner", "thickness": mm(k), "radius": mm(k),
                   "kFactor": 0.5, "relief": {"shape": "round", "size": mm(3.0 * k)},
                   "bendRelief": {"shape": "round", "size": mm(k)},
                   "corners": [{"flanges": [0, 1], "kind": "overlap", "gap": mm(0.25 * k),
                                "over": 0}],
                   "flanges": [{"edge": edges[0], "length": mm(20.0 * k), "angle": "90 deg"},
                               {"edge": edges[3], "length": mm(10.0 * k), "angle": "90 deg"},
                               {"tip": 0, "length": mm(8.0 * k), "angle": "90 deg"}]}),
        );
        let found = body(&mut s, "Corner");
        assert_eq!(found["valid"], true, "scale {k}: {found}");
        let table = run(&mut s, "sheetMetal.bends", json!({"feature": "Corner"}));
        let ends = table["bends"][2]["ends"].as_array().unwrap();
        assert_eq!(ends.len(), 1, "scale {k}: {table}");
        assert!(!ends[0]["relief"].is_null(), "{table}");
        let out = run(
            &mut s,
            "sheetMetal.bendSequence",
            json!({"feature": "Corner", "punch": scaled(&punch, k), "die": scaled(&die, k),
                   "segmented": true}),
        );
        assert_eq!(out["feasible"], true, "scale {k}: {out}");
        assert_eq!(
            out["steps"].as_array().unwrap().len(),
            3,
            "scale {k}: {out}"
        );
    }
}
