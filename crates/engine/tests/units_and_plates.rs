//! Mechanical units in outputs and assertions, and the plate thickness of
//! a face as a filter of named sets: the two gaps a Jig author found with
//! a welded beam (plan/repro/repro-welded-beam-1.json and -3.json).
//!
//! A stress is load / area with the units written in: 1000 N on a section
//! of 100 mm² is 10 MPa, shown with the output's label and unit; a unit of
//! another dimension is refused by name, and `max: "235 MPa"` bounds it.
//! A named set of "all plates of 4 mm" follows the plates' thickness
//! variables; a face over a step has no one thickness and never fits.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn refused(session: &mut Session, id: &str, params: Value) -> String {
    match session.execute(id, &params) {
        Ok(value) => panic!("{id} {params} should fail, returned {value}"),
        Err(e) => e.message().to_string(),
    }
}

fn new_session() -> Session {
    let mut s = Session::new(Workspace::unrestricted());
    run(&mut s, "file.new", json!({"units": "mm"}));
    s
}

fn output(session: &mut Session, name: &str) -> Value {
    let listed = run(session, "output.list", json!({}));
    listed["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["name"] == name)
        .cloned()
        .unwrap_or_else(|| panic!("no output {name}: {listed}"))
}

/// The welded-beam repro with units: a bar 10 × 10 mm in section, a load
/// of 1000 N.
fn beam() -> Session {
    let mut s = new_session();
    run(
        &mut s,
        "variable.set",
        json!({"name": "load", "expr": "1000 N"}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Bar", "size": ["100 mm", "10 mm", "10 mm"]}),
    );
    run(
        &mut s,
        "variable.measure",
        json!({"name": "area", "measure": "sectionArea", "of": "Bar", "plane": "yz", "offset": "50 mm"}),
    );
    s
}

#[test]
fn a_stress_is_load_over_area_in_mpa_with_its_label() {
    let mut s = beam();
    // Without a unit of its own: a pressure shows in MPa.
    run(
        &mut s,
        "variable.output",
        json!({"name": "stress", "expr": "load / area"}),
    );
    let plain = output(&mut s, "stress");
    assert_eq!(plain["status"], "known", "{plain}");
    assert_eq!(plain["unit"], "MPa", "{plain}");
    assert_eq!(plain["dimension"], "pressure", "{plain}");
    assert!(
        (plain["value"].as_f64().unwrap() - 10.0).abs() < 1e-9,
        "{plain}"
    );
    assert_eq!(plain["message"], "10 MPa");
    // With a label and a unit of its own.
    run(
        &mut s,
        "variable.output",
        json!({"name": "stress", "expr": "load / area", "label": "Bending stress", "unit": "N/mm²"}),
    );
    let shown = output(&mut s, "stress");
    assert_eq!(shown["label"], "Bending stress");
    assert_eq!(shown["unit"], "N/mm²");
    assert_eq!(shown["text"], "Bending stress 10 N/mm²", "{shown}");
    // Another unit; the label is kept when not given.
    run(
        &mut s,
        "variable.output",
        json!({"name": "stress", "expr": "load / area", "unit": "kPa"}),
    );
    let kpa = output(&mut s, "stress");
    assert_eq!(kpa["text"], "Bending stress 10000 kPa", "{kpa}");
    // param.list shows it too, with its label.
    let listed = run(&mut s, "param.list", json!({}));
    let row = listed["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["name"] == "stress")
        .cloned()
        .unwrap();
    assert_eq!(row["label"], "Bending stress", "{row}");
    // A moment: the load at the end of the 100 mm bar, in N m.
    run(
        &mut s,
        "variable.output",
        json!({"name": "moment", "expr": "load * 100 mm", "label": "Bending moment"}),
    );
    assert_eq!(output(&mut s, "moment")["text"], "Bending moment 100 N m");
    run(
        &mut s,
        "variable.output",
        json!({"name": "moment", "expr": "load * 100 mm", "unit": "kN mm"}),
    );
    assert_eq!(output(&mut s, "moment")["message"], "100 kN mm");
}

#[test]
fn a_unit_of_another_dimension_is_refused_and_one_that_stops_fitting_is_said() {
    let mut s = beam();
    let message = refused(
        &mut s,
        "variable.output",
        json!({"name": "stress", "expr": "load / area", "unit": "mm"}),
    );
    assert!(
        message.contains("a length") && message.contains("a pressure") && message.contains("MPa"),
        "{message}"
    );
    assert!(
        run(&mut s, "output.list", json!({}))["outputs"]
            .as_array()
            .unwrap()
            .iter()
            .all(|o| o["name"] != "stress")
    );
    // A measured output's dimension is its measure's.
    let message = refused(
        &mut s,
        "variable.measure",
        json!({"name": "vol", "measure": "volume", "of": "Bar", "unit": "MPa"}),
    );
    assert!(
        message.contains("a volume") || message.contains("length^3"),
        "{message}"
    );
    for (unit, says) in [
        ("Nm", "not a unit"),
        ("", "empty"),
        ("N/mm/s", "more than one"),
        ("mm^x", "whole power"),
    ] {
        let message = refused(
            &mut s,
            "variable.output",
            json!({"name": "stress", "expr": "load / area", "unit": unit}),
        );
        assert!(message.contains(says), "{unit}: {message}");
    }
    for bad in [json!(3), json!(["MPa"]), json!({"u": 1})] {
        refused(
            &mut s,
            "variable.output",
            json!({"name": "stress", "expr": "load / area", "unit": bad.clone()}),
        );
        refused(
            &mut s,
            "variable.output",
            json!({"name": "stress", "expr": "load / area", "label": bad}),
        );
    }
    // An edit that changes the dimension makes it unknown, with why: the
    // load written without its unit.
    run(
        &mut s,
        "variable.output",
        json!({"name": "stress", "expr": "load / area", "unit": "MPa"}),
    );
    // Refused at once where the dimension is known when it is defined.
    let message = refused(
        &mut s,
        "variable.output",
        json!({"name": "stress", "expr": "area"}),
    );
    assert!(
        message.contains("an area") || message.contains("length^2"),
        "{message}"
    );
    run(
        &mut s,
        "variable.set",
        json!({"name": "load", "expr": "1000"}),
    );
    let unknown = output(&mut s, "stress");
    assert_eq!(unknown["status"], "unknown", "{unknown}");
    assert!(
        unknown["message"].as_str().unwrap().contains("its unit"),
        "{unknown}"
    );
    // null takes the unit away: known again, in mm^-2.
    run(
        &mut s,
        "variable.output",
        json!({"name": "stress", "expr": "load / area", "unit": null}),
    );
    assert_eq!(output(&mut s, "stress")["unit"], "mm^-2");
}

#[test]
fn an_assertion_bounds_a_stress_in_mpa() {
    let mut s = beam();
    run(
        &mut s,
        "variable.output",
        json!({"name": "stress", "expr": "load / area"}),
    );
    run(
        &mut s,
        "assert.add",
        json!({"name": "yield", "measure": "value", "expr": "stress", "max": "235 MPa"}),
    );
    run(
        &mut s,
        "assert.add",
        json!({"name": "too low", "measure": "value", "expr": "stress", "max": "5 N/(1 mm)^2"}),
    );
    run(
        &mut s,
        "assert.add",
        json!({"name": "wrong dimension", "measure": "value", "expr": "stress", "max": "5 mm"}),
    );
    let listed = run(&mut s, "assert.list", json!({}));
    let status = |name: &str| {
        listed["assertions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["name"] == name)
            .cloned()
            .unwrap()
    };
    assert_eq!(status("yield")["status"], "passed", "{listed}");
    assert_eq!(status("too low")["status"], "failed", "{listed}");
    assert_eq!(status("wrong dimension")["status"], "error", "{listed}");
    // A plain-number bound of a stress is in MPa.
    run(
        &mut s,
        "assert.add",
        json!({"name": "plain", "measure": "value", "expr": "stress", "min": 9.5, "max": 10.5}),
    );
    let listed = run(&mut s, "assert.list", json!({}));
    let plain = listed["assertions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["name"] == "plain")
        .cloned()
        .unwrap();
    assert_eq!(plain["status"], "passed", "{plain}");
}

#[test]
fn labels_and_units_of_outputs_survive_a_save() {
    let dir = common::workdir("units", "save");
    let mut s = common::session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "variable.set",
        json!({"name": "load", "expr": "2 kN"}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Bar", "size": ["100 mm", "10 mm", "10 mm"]}),
    );
    run(
        &mut s,
        "variable.output",
        json!({"name": "stress", "expr": "load / (100 * (1 mm)^2)", "label": "Bending stress", "unit": "MPa"}),
    );
    run(
        &mut s,
        "file.save",
        json!({"path": "units.3dcraft", "overwrite": true}),
    );
    run(&mut s, "file.open", json!({"path": "units.3dcraft"}));
    let shown = output(&mut s, "stress");
    assert_eq!(shown["text"], "Bending stress 20 MPa", "{shown}");
}

#[test]
fn a_variable_named_like_a_mechanical_unit_still_works() {
    let mut s = new_session();
    // A count `N` and a thickness `t`, as designs name them.
    run(&mut s, "variable.set", json!({"name": "N", "expr": "12"}));
    run(&mut s, "variable.set", json!({"name": "s", "expr": "3 mm"}));
    run(
        &mut s,
        "variable.set",
        json!({"name": "pull", "expr": "N * 5 N"}),
    );
    run(
        &mut s,
        "variable.output",
        json!({"name": "total", "expr": "pull + N * s * 0 N / (1 mm)"}),
    );
    assert_eq!(output(&mut s, "total")["message"], "60 N");
}

// ------------------------------------------------------------- plates

/// The faces in the named set `name`, as (body, face) names.
fn set_faces(session: &mut Session, name: &str) -> Vec<String> {
    let found = run(session, "select.query", json!({"where": {"set": name}}));
    found["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["body"].as_str().unwrap_or("?").to_string())
        .collect()
}

/// The welded-beam repro: three plates whose thicknesses are variables;
/// "Cut sheet: 4 mm" is a set by thickness, not by plate.
#[test]
fn a_set_by_thickness_follows_the_plate_variables() {
    let mut s = new_session();
    for (name, expr) in [("tA", "4 mm"), ("tB", "4 mm"), ("tC", "3 mm")] {
        run(&mut s, "variable.set", json!({"name": name, "expr": expr}));
    }
    for (name, t, z) in [
        ("A", "tA", "0 mm"),
        ("B", "tB", "50 mm"),
        ("C", "tC", "100 mm"),
    ] {
        run(
            &mut s,
            "feature.block",
            json!({"name": name, "size": ["200 mm", "100 mm", t], "position": ["0 mm", "0 mm", z]}),
        );
    }
    run(
        &mut s,
        "set.define",
        json!({"name": "Cut sheet: 4 mm", "kind": "face", "where": {"normal": {"along": "z"}, "thickness": "4 mm"}}),
    );
    run(
        &mut s,
        "set.define",
        json!({"name": "Like A", "kind": "face", "where": {"normal": {"along": "z"}, "thickness": {"min": "tA - 0.1 mm", "max": "tA + 0.1 mm"}}}),
    );
    assert_eq!(set_faces(&mut s, "Cut sheet: 4 mm"), ["A", "B"]);
    run(
        &mut s,
        "variable.set",
        json!({"name": "tB", "expr": "3 mm"}),
    );
    assert_eq!(set_faces(&mut s, "Cut sheet: 4 mm"), ["A"]);
    run(
        &mut s,
        "variable.set",
        json!({"name": "tC", "expr": "4 mm"}),
    );
    assert_eq!(set_faces(&mut s, "Cut sheet: 4 mm"), ["A", "C"]);
    // An expression of the variables is worked out again: A's own
    // thickness, whatever it is.
    assert_eq!(set_faces(&mut s, "Like A"), ["A", "C"]);
    run(
        &mut s,
        "variable.set",
        json!({"name": "tA", "expr": "3 mm"}),
    );
    assert_eq!(set_faces(&mut s, "Like A"), ["A", "B"]);
    let listed = run(&mut s, "set.list", json!({}));
    assert_eq!(listed["sets"][0]["count"], 1, "{listed}");
    // Every flat face of a plate: its two sides of 3 mm.
    let all = run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"in": "B", "thickness": "3 mm"}}),
    );
    assert_eq!(all["count"], 2, "{all}");
    // The sides, 100 and 200 mm across.
    let across = run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"in": "B", "thickness": {"min": "99 mm", "max": "201 mm"}}}),
    );
    assert_eq!(across["count"], 4, "{across}");
}

/// A plate with a step under half of it: its top face has two
/// thicknesses and fits neither; the bottom faces fit their own. A small
/// boss under another plate, smaller than the spacing of the samples over
/// the face, still takes its top face out. A curved face never fits.
#[test]
fn a_face_over_a_step_has_no_one_thickness() {
    let mut s = new_session();
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": ["100 mm", "50 mm", "4 mm"]}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Step", "size": ["50 mm", "50 mm", "2 mm"], "position": ["50 mm", "0 mm", "-2 mm"]}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "union", "target": "Plate", "tool": "Step", "name": "Stepped"}),
    );
    let fits = |s: &mut Session, along: &str, t: &str| {
        run(
            s,
            "select.query",
            json!({"kind": "face", "where": {"in": "Stepped", "normal": {"along": along}, "thickness": t}}),
        )["count"]
            .as_u64()
            .unwrap()
    };
    assert_eq!(fits(&mut s, "z", "4 mm"), 0);
    assert_eq!(fits(&mut s, "z", "6 mm"), 0);
    // Underneath: 4 mm where there is no step, 6 mm under the step.
    assert_eq!(fits(&mut s, "-z", "4 mm"), 1);
    assert_eq!(fits(&mut s, "-z", "6 mm"), 1);
    // A boss of 3 × 3 mm under a plate of 200 × 100 mm.
    run(
        &mut s,
        "feature.block",
        json!({"name": "Wide", "size": ["200 mm", "100 mm", "4 mm"], "position": ["0 mm", "0 mm", "20 mm"]}),
    );
    assert_eq!(
        run(
            &mut s,
            "select.query",
            json!({"kind": "face", "where": {"in": "Wide", "normal": {"along": "z"}, "thickness": "4 mm"}}),
        )["count"],
        1
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Boss", "size": ["3 mm", "3 mm", "1 mm"], "position": ["77 mm", "41 mm", "19 mm"]}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "union", "target": "Wide", "tool": "Boss", "name": "Bossed"}),
    );
    assert_eq!(
        run(
            &mut s,
            "select.query",
            json!({"kind": "face", "where": {"in": "Bossed", "normal": {"along": "z"}, "thickness": "4 mm"}}),
        )["count"],
        0
    );
    // A cylinder: its flat ends are 10 mm apart, its side never fits.
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Rod", "radius": "5 mm", "height": "10 mm", "position": ["0 mm", "0 mm", "100 mm"]}),
    );
    let rod = |s: &mut Session, surface: &str| {
        run(
            s,
            "select.query",
            json!({"kind": "face", "where": {"in": "Rod", "surface": surface, "thickness": {"min": "0 mm"}}}),
        )["count"]
            .as_u64()
            .unwrap()
    };
    assert_eq!(rod(&mut s, "plane"), 2);
    assert_eq!(rod(&mut s, "cylinder"), 0);
}

#[test]
fn a_thickness_filter_that_does_not_read_is_an_error() {
    let mut s = new_session();
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": ["10 mm", "10 mm", "1 mm"]}),
    );
    for bad in [
        json!("3 deg"),
        json!("nope"),
        json!({"min": "1 mm^2"}),
        json!({"eq": "1 mm", "min": "1 mm"}),
        json!(true),
        json!([1]),
        json!({"tol": "-1 mm", "eq": "1 mm"}),
    ] {
        refused(
            &mut s,
            "select.query",
            json!({"where": {"thickness": bad.clone()}}),
        );
        refused(
            &mut s,
            "set.define",
            json!({"name": "bad", "where": {"thickness": bad}}),
        );
    }
}

/// A measured mass is a mass (since format 56; it was a plain number of
/// kg): shown in kg, bounded by a plain number of kg or by `2 kg`, and a
/// plain number added to it is a unit mismatch until written with `kg`.
#[test]
fn a_measured_mass_is_a_mass_in_kg() {
    let mut s = new_session();
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": ["100 mm", "100 mm", "10 mm"]}),
    );
    run(
        &mut s,
        "body.material",
        json!({"body": "Plate", "name": "Steel", "density": 7850}),
    );
    run(
        &mut s,
        "variable.measure",
        json!({"name": "massKg", "measure": "mass", "of": "Plate"}),
    );
    let mass = output(&mut s, "massKg");
    assert_eq!(mass["status"], "known", "{mass}");
    assert_eq!(mass["dimension"], "mass", "{mass}");
    assert_eq!(mass["unit"], "kg", "{mass}");
    assert!(
        (mass["value"].as_f64().unwrap() - 0.785).abs() < 1e-9,
        "{mass}"
    );
    run(
        &mut s,
        "variable.output",
        json!({"name": "plus", "expr": "massKg + 1"}),
    );
    let plus = output(&mut s, "plus");
    assert_eq!(plus["status"], "unknown", "{plus}");
    run(
        &mut s,
        "variable.output",
        json!({"name": "plus", "expr": "massKg + 1 kg", "unit": "g"}),
    );
    assert_eq!(output(&mut s, "plus")["message"], "1785 g");
    for (name, max) in [
        ("plain", json!(1)),
        ("kg", json!("1 kg")),
        ("g", json!("700 g")),
    ] {
        run(
            &mut s,
            "assert.add",
            json!({"name": name, "measure": "mass", "of": "Plate", "max": max}),
        );
    }
    let listed = run(&mut s, "assert.list", json!({}));
    let status: Vec<&str> = listed["assertions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["status"].as_str().unwrap())
        .collect();
    assert_eq!(status, ["passed", "passed", "failed"], "{listed}");
}

/// A narrow rib under a plate, along its whole width: the grid over the
/// top face can miss it, and its deep corners lie deeper than any line
/// the grid met (the filter samples only vertices no deeper than that);
/// the corners where it meets the plate, at the plate's thickness, are
/// sampled and tell the step, at three scales.
#[test]
fn a_narrow_rib_behind_a_face_is_found_from_where_it_meets_the_plate() {
    for scale in [0.01, 1.0, 1000.0] {
        let mut s = new_session();
        let mm = |v: f64| format!("{} mm", v * scale);
        run(
            &mut s,
            "feature.block",
            json!({"name": "Plate", "size": [mm(100.0), mm(50.0), mm(4.0)]}),
        );
        run(
            &mut s,
            "feature.block",
            json!({"name": "Rib", "size": [mm(0.5), mm(50.0), mm(1.0)], "position": [mm(37.3), mm(0.0), mm(-1.0)]}),
        );
        run(
            &mut s,
            "feature.boolean",
            json!({"op": "union", "target": "Plate", "tool": "Rib", "name": "Ribbed"}),
        );
        let top = run(
            &mut s,
            "select.query",
            json!({"kind": "face", "where": {"in": "Ribbed", "normal": {"along": "z"}, "thickness": {"min": "0 mm"}}}),
        );
        assert_eq!(top["count"], 0, "scale {scale}: {top}");
        // The rib's own bottom is 5 thick, its sides 0.5.
        let bottom = run(
            &mut s,
            "select.query",
            json!({"kind": "face", "where": {"in": "Ribbed", "normal": {"along": "-z"}, "thickness": mm(5.0)}}),
        );
        assert_eq!(bottom["count"], 1, "scale {scale}: {bottom}");
    }
}

/// A narrow groove under a block, across its whole width, behind a boss
/// on its top: every vertex of the groove lies outside the boss's box, so
/// only the points along the groove's edges (which cross the box) tell
/// the step; the boss's top without the groove is 12 thick. Three scales.
#[test]
fn a_groove_whose_ends_lie_outside_the_face_is_found_from_its_edges() {
    for scale in [0.01, 1.0, 1000.0] {
        let mut s = new_session();
        let mm = |v: f64| format!("{} mm", v * scale);
        run(
            &mut s,
            "feature.block",
            json!({"name": "Base", "size": [mm(100.0), mm(50.0), mm(10.0)]}),
        );
        run(
            &mut s,
            "feature.block",
            json!({"name": "Boss", "size": [mm(10.0), mm(10.0), mm(2.0)], "position": [mm(40.0), mm(20.0), mm(10.0)]}),
        );
        run(
            &mut s,
            "feature.boolean",
            json!({"op": "union", "target": "Base", "tool": "Boss", "name": "Bossed"}),
        );
        let top = |s: &mut Session, body: &str| {
            run(
                s,
                "select.query",
                json!({"kind": "face", "where": {"in": body, "normal": {"along": "z"}, "inside": [[mm(39.0), mm(19.0), mm(11.0)], [mm(51.0), mm(31.0), mm(13.0)]], "thickness": {"min": "0 mm"}}}),
            )
        };
        let plain = top(&mut s, "Bossed");
        assert_eq!(plain["count"], 1, "scale {scale}: {plain}");
        run(
            &mut s,
            "feature.block",
            json!({"name": "Cutter", "size": [mm(0.5), mm(50.0), mm(1.0)], "position": [mm(46.4), mm(0.0), mm(0.0)]}),
        );
        run(
            &mut s,
            "feature.boolean",
            json!({"op": "subtract", "target": "Bossed", "tool": "Cutter", "name": "Grooved"}),
        );
        let grooved = top(&mut s, "Grooved");
        assert_eq!(grooved["count"], 0, "scale {scale}: {grooved}");
    }
}

/// The thickness filter's cost on a body of about a thousand faces and
/// four thousand vertices (a plate with 500 bosses under it: some 2500 faces, 4000 vertices): one query
/// over its flat faces, timed (docs/design/selection.md). It must answer
/// within a generous bound in a debug build and find the plate's top face
/// over the bosses has no one thickness.
#[test]
fn the_thickness_filter_on_a_thousand_faces() {
    let mut s = new_session();
    run(
        &mut s,
        "feature.block",
        json!({"name": "Base", "size": ["500 mm", "500 mm", "4 mm"]}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Boss", "size": ["4 mm", "4 mm", "2 mm"], "position": ["10 mm", "10 mm", "-2 mm"]}),
    );
    run(
        &mut s,
        "feature.pattern",
        json!({"source": "Boss", "count": 25, "step": ["20 mm", "0 mm", "0 mm"], "name": "Row"}),
    );
    run(
        &mut s,
        "feature.pattern",
        json!({"source": "Row", "count": 20, "step": ["0 mm", "20 mm", "0 mm"], "name": "Bosses"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "union", "target": "Base", "tool": "Bosses", "name": "Plate"}),
    );
    let inspect = run(&mut s, "document.inspect", json!({"measures": false}));
    let body = inspect["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == "Plate")
        .cloned()
        .unwrap();
    let (faces, vertices) = (
        body["faces"].as_u64().unwrap(),
        body["vertices"].as_u64().unwrap(),
    );
    let started = std::time::Instant::now();
    let found = run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"in": "Plate", "surface": "plane", "thickness": {"min": "0 mm"}}}),
    );
    let took = started.elapsed();
    println!(
        "BENCH thickness filter: {faces} faces, {vertices} vertices, {} fit, {took:?}",
        found["count"]
    );
    assert!(
        faces >= 1000 && vertices >= 4000,
        "{faces} faces, {vertices} vertices"
    );
    // Asked again, the thicknesses of the same body are remembered.
    let started = std::time::Instant::now();
    let again = run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"in": "Plate", "surface": "plane", "thickness": {"min": "0 mm"}}}),
    );
    println!(
        "BENCH thickness filter asked again: {:?}",
        started.elapsed()
    );
    assert_eq!(again, found);
    let top = run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"in": "Plate", "normal": {"along": "z"}, "thickness": {"min": "0 mm"}}}),
    );
    // The bosses' bottoms are 2 mm thick, the top over them is not one.
    assert_eq!(top["count"], 0, "{top}");
    assert!(took.as_secs() < 600, "{took:?}");
}
