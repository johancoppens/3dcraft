//! The platform gaps the sheet-metal Jigs found (plan/repro/
//! repro-sheet-metal-enclosure-2..5, repro-rack-2 and -4), each a test that
//! failed on main before its fix: holes in flanges, from the base's sketch
//! and from cut-out sketches switched on and off with `when` (in another
//! plane frame too), cut flat and folded, measured by `holeToBend`, saved
//! and reopened, at three scales; the measures read a part by its feature
//! whatever takes its body later; a part switched off is left out of the
//! measures unless it is named; a name glob with a wildcard feature; and
//! several parts' flat patterns on one sheet.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{assert_close, body, built, error, new_document, run, session_in, workdir};
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::f64::consts::{FRAC_PI_2, PI};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn number(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

/// A rectangle from `from` to `to` in a new sketch on the xy plane; its
/// lines' entity ids (bottom, right, top, left).
fn rect(s: &mut Session, name: &str, from: [f64; 2], to: [f64; 2]) -> Vec<String> {
    run(s, "sketch.create", json!({"plane": "xy", "name": name}));
    let made = run(
        s,
        "sketch.rect",
        json!({"sketch": name, "from": from, "to": to}),
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

/// Holes in a flange (repro-sheet-metal-enclosure-2, repro-rack-4): a
/// 50 × 40 base with a flange on its right edge (K = ½, so the volume is
/// the flat area times the thickness). Its flat starts at x0 = 50 + BA in
/// the pattern; a circle drawn there in the base's sketch is a hole of
/// the flange, a cut-out sketch's circle in the base is a vent switched by
/// `when`, and a circle on a datum plane whose normal points down (axes
/// mirrored) lands on the flange too. Cut flat (the export's circles) and
/// folded (the volume), measured from the bend beside each, the same after
/// a save, at three scales.
#[test]
fn holes_in_flanges_are_cut_flat_and_folded_and_measured() {
    for k in SCALES {
        let dir = workdir("sheetmetal-gaps", &format!("holes-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let (t, r, l) = (1.0 * k, 1.0 * k, 15.0 * k);
        let ba = FRAC_PI_2 * (r + 0.5 * t);
        let x0 = 50.0 * k + ba;
        let lines = rect(&mut s, "Base", [0.0, 0.0], [50.0 * k, 40.0 * k]);
        run(
            &mut s,
            "sketch.circle",
            json!({"sketch": "Base", "center": [x0 + 6.0 * k, 20.0 * k], "radius": 1.7 * k}),
        );
        run(&mut s, "variable.set", json!({"name": "vent", "expr": "1"}));
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "xy", "name": "Vents"}),
        );
        run(
            &mut s,
            "sketch.circle",
            json!({"sketch": "Vents", "center": [25.0 * k, 20.0 * k], "radius": 3.0 * k}),
        );
        run(
            &mut s,
            "feature.when",
            json!({"feature": "Vents", "expr": "vent"}),
        );
        // Origin at the first point, x towards the second (-x), the normal
        // -z: the sketch's axes are the base's mirrored.
        let (px, py) = (x0 + 11.0 * k, 30.0 * k);
        run(
            &mut s,
            "datum.plane",
            json!({"name": "Flip", "through": [[px, py, 0.0], [px - k, py, 0.0], [px, py + 10.0 * k, 0.0]]}),
        );
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "Flip", "name": "Mirror"}),
        );
        run(
            &mut s,
            "sketch.circle",
            json!({"sketch": "Mirror", "center": [0.0, 0.0], "radius": 1.0 * k}),
        );
        built(
            &mut s,
            "feature.sheetMetal",
            json!({"sketch": "Base", "name": "Box", "thickness": format!("{t} mm"),
                   "radius": format!("{r} mm"), "kFactor": 0.5, "cutouts": ["Vents", "Mirror"],
                   "flanges": [{"edge": lines[1], "length": format!("{l} mm"), "angle": "90 deg"}]}),
        );
        let holes = PI * (1.7f64.powi(2) + 3.0f64.powi(2) + 1.0) * k * k;
        let area = 50.0 * k * 40.0 * k + 40.0 * k * (ba + l) - holes;
        assert_close(
            &body(&mut s, "Box")["volume"],
            area * t,
            "volume with holes",
        );
        let table = run(&mut s, "sheetMetal.bends", json!({"feature": "Box"}));
        let found = table["holes"].as_array().unwrap();
        assert_eq!(found.len(), 3, "{table}");
        // The base's vent, 25 − 3 from the bend; the flange's holes, 6 −
        // 1.7 and 11 − 1 from the bend's tangent line on the flange.
        let expect = [(Value::Null, 22.0), (json!(0), 4.3), (json!(0), 10.0)];
        for (hole, (flange, d)) in found.iter().zip(expect) {
            assert_eq!(hole["flange"], flange, "{hole}");
            assert!(
                (number(&hole["distance"]) - d * k).abs() <= 1e-9 * k,
                "{hole}: {d}"
            );
        }
        let check = assertion(
            &mut s,
            "Hole",
            json!({"measure": "holeToBend", "of": "Box", "eq": format!("{} mm", 4.3 * k)}),
        );
        assert_eq!(check["status"], "passed", "{check}");
        // Cut flat: the outline and the three circles.
        let out = run(
            &mut s,
            "file.export",
            json!({"path": "box.dxf", "sheetMetal": "Box"}),
        );
        assert_close(&out["flatArea"], area, "flat area");
        let dxf = std::fs::read_to_string(dir.join("box.dxf")).unwrap();
        assert_eq!(dxf.matches("CIRCLE").count(), 3, "{dxf}");
        // Saved and reopened: the same body.
        let before = body(&mut s, "Box");
        run(&mut s, "file.save", json!({"path": "box.3dcraft"}));
        let mut u = session_in(&dir);
        run(&mut u, "file.open", json!({"path": "box.3dcraft"}));
        let again = body(&mut u, "Box");
        assert_eq!(before["volume"], again["volume"]);
        assert_eq!(before["faceNames"], again["faceNames"]);
        // The vent switched off: no hole in the base.
        run(&mut s, "variable.set", json!({"name": "vent", "expr": "0"}));
        assert_close(
            &body(&mut s, "Box")["volume"],
            (area + PI * 9.0 * k * k) * t,
            "volume without the vent",
        );
        let table = run(&mut s, "sheetMetal.bends", json!({"feature": "Box"}));
        assert_eq!(table["holes"].as_array().unwrap().len(), 2, "{table}");
    }
}

/// A hole that lies in no one flat and not inside the pattern fails the
/// part with the reason; a cut-out sketch off the base's plane is refused
/// by name. (The hole was across the bend's strip until holes across a bend
/// were built, 2026-10-28, `sheetmetal_cuts.rs`; it crosses the strip's end
/// now, past the pattern's edge.)
#[test]
fn holes_across_a_bend_or_off_the_plane_fail_the_part() {
    let dir = workdir("sheetmetal-gaps", "refusals");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    let lines = rect(&mut s, "Base", [0.0, 0.0], [50.0, 40.0]);
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Base", "center": [51.0, 0.0], "radius": 0.5}),
    );
    let made = run(
        &mut s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "name": "Across", "thickness": "1 mm", "radius": "1 mm",
               "kFactor": 0.4, "flanges": [{"edge": lines[1], "length": "15 mm", "angle": "90 deg"}]}),
    );
    assert_eq!(made["status"], "failed", "{made}");
    let why = made["error"].as_str().unwrap();
    assert!(
        why.contains("no one flat") && why.contains("sketch `Base`"),
        "{why}"
    );
    let lines = rect(&mut s, "Base2", [0.0, 100.0], [50.0, 140.0]);
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "yz", "name": "Side"}),
    );
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Side", "center": [120.0, 0.0], "radius": 1.0}),
    );
    let made = run(
        &mut s,
        "feature.sheetMetal",
        json!({"sketch": "Base2", "name": "Off", "thickness": "1 mm", "radius": "1 mm",
               "kFactor": 0.4, "cutouts": ["Side"],
               "flanges": [{"edge": lines[1], "length": "15 mm", "angle": "90 deg"}]}),
    );
    assert_eq!(made["status"], "failed", "{made}");
    let why = made["error"].as_str().unwrap();
    assert!(
        why.contains("cutouts.0") && why.contains("not in the base sketch's plane"),
        "{why}"
    );
    // Bad parameters are errors.
    for bad in [
        json!("Side"),
        json!([5.5]),
        json!(["Nope"]),
        json!(["Base2", "Base2"]),
    ] {
        error(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Off", "cutouts": bad}),
        );
    }
}

/// The repro's base sketch: a 50 × 40 rectangle and a part on its right
/// edge, `name`, with `extra` parameters.
fn part(s: &mut Session, sketch: &str, name: &str, extra: Value) {
    let lines = rect(s, sketch, [0.0, 0.0], [50.0, 40.0]);
    let mut params = json!({"sketch": sketch, "name": name, "thickness": "1 mm", "radius": "1 mm",
        "kFactor": 0.4, "flanges": [{"edge": lines[1], "length": "15 mm", "angle": "90 deg"}]});
    if let (Some(map), Value::Object(more)) = (params.as_object_mut(), extra) {
        map.extend(more);
    }
    run(s, "feature.sheetMetal", params);
}

/// The measures read a part as it was built, by its feature, whatever
/// boolean drills it later (repro-sheet-metal-enclosure-3): before, a part
/// named by `of` was "went into Drilled; name that feature", and Drilled
/// "not a sheet-metal part" with no way on.
#[test]
fn a_part_a_boolean_took_is_measured_by_its_feature() {
    let dir = workdir("sheetmetal-gaps", "consumed");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    part(&mut s, "S", "P", json!({}));
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Drill", "radius": "1.7 mm", "height": "10 mm", "axis": [1, 0, 0],
               "position": ["45 mm", "20 mm", "10 mm"]}),
    );
    built(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "P", "tool": "Drill", "name": "Drilled"}),
    );
    let named = assertion(
        &mut s,
        "Named",
        json!({"measure": "bendRadius", "of": "P", "min": "1 mm"}),
    );
    assert_eq!(named["status"], "passed", "{named}");
    let all = assertion(
        &mut s,
        "All",
        json!({"measure": "bendRadius", "min": "1 mm"}),
    );
    assert_eq!(all["status"], "passed", "{all}");
    let other = assertion(
        &mut s,
        "Other",
        json!({"measure": "bendRadius", "of": "Drilled", "min": "1 mm"}),
    );
    assert_eq!(other["status"], "error", "{other}");
    let why = other["message"].as_str().unwrap();
    assert!(
        why.contains("not a sheet-metal part") && why.contains("`P`"),
        "the error names the part to measure: {why}"
    );
    // The bend table and the export say what took the body.
    let table = run(&mut s, "sheetMetal.bends", json!({"feature": "P"}));
    assert_eq!(table["changedBy"], "Drilled", "{table}");
    let out = run(
        &mut s,
        "file.export",
        json!({"path": "p.dxf", "sheetMetal": "P"}),
    );
    assert_eq!(out["changedBy"], "Drilled", "{out}");
}

/// A part switched off by `when` is left out of the measures; named, it is
/// refused saying so (repro-sheet-metal-enclosure-4, repro-rack-2: before,
/// any part switched off broke every sheet-metal measure).
#[test]
fn a_part_switched_off_is_left_out_of_the_measures() {
    let dir = workdir("sheetmetal-gaps", "off");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    run(&mut s, "variable.set", json!({"name": "on", "expr": "0"}));
    part(&mut s, "S", "Vented", json!({"when": "on == 1"}));
    part(&mut s, "S2", "Plain", json!({"when": "on == 0"}));
    let all = assertion(
        &mut s,
        "All",
        json!({"measure": "bendRadius", "min": "1 mm"}),
    );
    assert_eq!(all["status"], "passed", "{all}");
    let query = assertion(
        &mut s,
        "Query",
        json!({"measure": "bendRadius", "of": {"kind": "feature", "where": {"type": "sheetMetal"}}, "min": "1 mm"}),
    );
    assert_eq!(query["status"], "passed", "{query}");
    let named = assertion(
        &mut s,
        "Named",
        json!({"measure": "bendRadius", "of": "Vented", "min": "1 mm"}),
    );
    assert_eq!(named["status"], "error", "{named}");
    assert!(
        named["message"].as_str().unwrap().contains("switched off"),
        "{named}"
    );
    // A query of the parts that were built: only Plain; inactive, Vented.
    for (status, name) in [
        (json!("ok"), "Plain"),
        (json!(["inactive", "failed"]), "Vented"),
    ] {
        let found = run(
            &mut s,
            "select.query",
            json!({"kind": "feature", "where": {"type": "sheetMetal", "status": status}}),
        );
        assert_eq!(found["count"], 1, "{found}");
        assert_eq!(found["items"][0]["name"], name, "{found}");
    }
    for bad in [json!("off"), json!([]), json!(3)] {
        error(
            &mut s,
            "select.query",
            json!({"kind": "feature", "where": {"status": bad}}),
        );
    }
    // The other way round: the named one measures, Plain is left out.
    run(&mut s, "variable.set", json!({"name": "on", "expr": "1"}));
    assert_eq!(listed(&mut s, "Named")["status"], "passed");
    assert_eq!(listed(&mut s, "All")["status"], "passed");
    // Every part off: said.
    run(
        &mut s,
        "feature.when",
        json!({"feature": "Vented", "expr": "0"}),
    );
    let all = listed(&mut s, "All");
    assert_eq!(all["status"], "error", "{all}");
    assert!(
        all["message"].as_str().unwrap().contains("switched off"),
        "{all}"
    );
}

/// A name glob whose feature is a wildcard, `*:face(0)`, in a query, an
/// assertion and a named set (repro-sheet-metal-enclosure-5: before, every
/// one was refused as "no feature is called `*`"); and a glob that writes
/// a feature by its name (`B:face(*)`).
#[test]
fn a_glob_with_a_wildcard_feature_matches_every_feature() {
    let mut s = common::session();
    new_document(&mut s, 1.0);
    for (name, x) in [("B", 0.0), ("C", 20.0)] {
        run(
            &mut s,
            "feature.block",
            json!({"name": name, "size": ["10 mm", "10 mm", "10 mm"], "position": [x, 0, 0]}),
        );
    }
    let found = run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"name": "*:face(0)"}}),
    );
    assert_eq!(found["count"], 2, "{found}");
    let found = run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"name": "B:face(*)"}}),
    );
    assert_eq!(found["count"], 6, "{found}");
    let found = run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"name": "F?:face(0)"}}),
    );
    assert_eq!(found["count"], 2, "{found}");
    let check = assertion(
        &mut s,
        "Count",
        json!({"measure": "count", "of": {"kind": "face", "where": {"name": "*:face(0)"}}, "eq": "2"}),
    );
    assert_eq!(check["status"], "passed", "{check}");
    run(
        &mut s,
        "set.define",
        json!({"name": "Bottoms", "kind": "face", "where": {"name": "*:face(0)"}}),
    );
    let set = run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"set": "Bottoms"}}),
    );
    assert_eq!(set["count"], 2, "{set}");
    // A name of a feature that is not there is still refused.
    error(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"name": "Nope:face(0)"}}),
    );
}

/// Several parts' flat patterns on one sheet, side by side as a set's
/// faces are laid out: `gap` apart in a row, a new row past `width`.
#[test]
fn several_flat_patterns_go_on_one_sheet() {
    let dir = workdir("sheetmetal-gaps", "sheet");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    part(&mut s, "S", "A", json!({}));
    part(&mut s, "S2", "B", json!({}));
    let one = run(
        &mut s,
        "file.export",
        json!({"path": "a.svg", "sheetMetal": "A"}),
    );
    let out = run(
        &mut s,
        "file.export",
        json!({"path": "both.dxf", "sheetMetal": ["A", "B"], "gap": "4 mm"}),
    );
    let parts = out["sheetMetal"].as_array().unwrap();
    assert_eq!(parts.len(), 2, "{out}");
    assert_eq!(out["rows"], 1, "{out}");
    assert_eq!(out["bendsUp"], 2, "{out}");
    assert_close(
        &out["flatArea"],
        2.0 * number(&one["flatArea"]),
        "flat area",
    );
    let (a, b) = (&parts[0]["bbox"], &parts[1]["bbox"]);
    assert_close(
        &b[0],
        number(&a[2]) + 4.0,
        "the second part a gap past the first",
    );
    let dxf = std::fs::read_to_string(dir.join("both.dxf")).unwrap();
    assert!(dxf.contains("bendUp") && dxf.contains("outline"));
    // A width that holds one part a row: two rows.
    let width = number(&a[2]) - number(&a[0]) + 1.0;
    let out = run(
        &mut s,
        "file.export",
        json!({"path": "rows.svg", "sheetMetal": ["A", "B"], "width": width}),
    );
    assert_eq!(out["rows"], 2, "{out}");
    for bad in [
        json!({"path": "x.svg", "sheetMetal": []}),
        json!({"path": "x.svg", "sheetMetal": ["A", "A"]}),
        json!({"path": "x.svg", "sheetMetal": ["A", "S"]}),
        json!({"path": "x.svg", "sheetMetal": ["A"], "width": "1 mm"}),
        json!({"path": "x.svg", "sheetMetal": "A", "gap": "1 mm"}),
    ] {
        error(&mut s, "file.export", bad);
    }
}
