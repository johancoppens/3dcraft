//! What testers and Jig authors writing generated scripts kept running
//! into (R1, 2026-10-25): face and edge names by a feature's name, a bare
//! cap, an axis from expressions, reserved variable names, assertions
//! still pending after an edit, and a condition given with the feature.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{error, run, session};
use craft3d_engine::Session;
use serde_json::{Value, json};

/// A 40 x 20 plate 10 thick extruded from a rectangle (`F2`, "Plate"), a
/// round bar of radius 2 on it extruded from a circle (`F4`, "Bar").
fn plate_and_bar(s: &mut Session) {
    run(s, "file.new", json!({ "units": "mm" }));
    run(s, "sketch.create", json!({ "name": "P" }));
    run(
        s,
        "sketch.rect",
        json!({ "sketch": "P", "from": [0, 0], "to": [40, 20] }),
    );
    run(
        s,
        "feature.extrude",
        json!({ "sketch": "P", "distance": 10, "name": "Plate" }),
    );
    run(
        s,
        "sketch.create",
        json!({ "name": "C", "plane": "xy", "offset": 10 }),
    );
    run(
        s,
        "sketch.circle",
        json!({ "sketch": "C", "center": [20, 10], "radius": 2 }),
    );
    run(
        s,
        "feature.extrude",
        json!({ "sketch": "C", "distance": 12, "name": "Bar" }),
    );
}

fn measured(s: &mut Session) -> Value {
    let inspected = run(s, "document.inspect", json!({}));
    let features: Vec<Value> = inspected["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| json!([f["name"], f["status"], f["error"]]))
        .collect();
    json!({ "features": features, "measure": run(s, "document.measure", json!({})) })
}

fn stored(s: &mut Session, name: &str) -> Value {
    let inspected = run(s, "document.inspect", json!({}));
    inspected["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == name)
        .unwrap()["params"]
        .clone()
}

/// Item 1: a feature's name where its id stands, at the start of a name
/// and inside one; the document stores the ids, so both ways build the
/// same, and a bare `startCap`/`endCap` is the short cap.
#[test]
fn faces_are_named_by_their_features_names() {
    let cases: [(&str, Value, Value); 4] = [
        (
            "feature.fillet",
            json!({ "edges": [["Plate:endCap", "Plate:side(e5)"]], "radius": 1, "name": "X" }),
            json!({ "edges": [["F2:endCap(e5,e6,e7,e8)", "F2:side(e5)"]], "radius": 1, "name": "X" }),
        ),
        (
            "feature.chamfer",
            json!({ "edges": [["Plate:startCap()", "Plate:side(e6)"]], "distance": 1, "name": "X" }),
            json!({ "edges": [["F2:startCap()", "F2:side(e6)"]], "distance": 1, "name": "X" }),
        ),
        (
            "feature.offsetFace",
            json!({ "faces": ["Plate:startCap"], "distance": 1, "name": "X" }),
            json!({ "faces": ["F2:startCap(e5,e6,e7,e8)"], "distance": 1, "name": "X" }),
        ),
        (
            "datum.plane",
            json!({ "from": "Bar:endCap", "distance": 3, "name": "X" }),
            json!({ "from": "F4:endCap(e2)", "distance": 3, "name": "X" }),
        ),
    ];
    for (id, by_name, by_id) in cases {
        let mut a = session();
        plate_and_bar(&mut a);
        let made = run(&mut a, id, by_name.clone());
        assert_eq!(made["status"], "ok", "{id} {by_name}: {made}");
        let mut b = session();
        plate_and_bar(&mut b);
        run(&mut b, id, by_id);
        assert_eq!(measured(&mut a), measured(&mut b), "{id}");
    }
    // What is stored is ids.
    let mut s = session();
    plate_and_bar(&mut s);
    run(
        &mut s,
        "feature.fillet",
        json!({ "edges": [["Plate:endCap", "Plate:side(e5)"]], "radius": 1, "name": "Round" }),
    );
    let edges = stored(&mut s, "Round")["edges"].clone();
    assert_eq!(edges, json!([["F2:endCap()", "F2:side(e5)"]]), "{edges}");
}

/// Inside a name: the copy of the bar's cap in a union, both features by
/// name; a sketch on it and a selection of it.
#[test]
fn nested_names_take_feature_names_too() {
    let mut s = session();
    plate_and_bar(&mut s);
    run(
        &mut s,
        "feature.boolean",
        json!({ "op": "union", "target": "Plate", "tool": "Bar", "name": "Joined" }),
    );
    let top = run(
        &mut s,
        "sketch.create",
        json!({ "face": "Joined:copy(1)/Bar:endCap", "name": "Top" }),
    );
    assert_eq!(top["status"], "ok", "{top}");
    let selected = run(
        &mut s,
        "select.set",
        json!({ "names": ["Joined:copy(1)/Bar:endCap"] }),
    );
    assert_eq!(
        selected["items"][0]["name"], "F5:copy(1)/F4:endCap(e2)",
        "{selected}"
    );
    // A fillet on the joined body, its edge named after the plate's faces
    // as they were before the union (followed to the newest body).
    let round = run(
        &mut s,
        "feature.fillet",
        json!({ "edges": [["Plate:endCap", "Plate:side(e5)"]], "radius": 1 }),
    );
    assert_eq!(round["status"], "ok", "{round}");
}

/// A name a generated script gives in one step is found in the next: the
/// lookup is done when each command runs.
#[test]
fn a_script_names_what_it_made_a_step_before() {
    let mut s = session();
    run(&mut s, "file.new", json!({ "units": "mm" }));
    let steps = [
        (
            "feature.block",
            json!({ "size": [20, 20, 10], "name": "Base" }),
        ),
        (
            "feature.chamfer",
            json!({ "edges": [["Base:face(1)", "Base:face(2)"]], "distance": 1, "name": "Edge" }),
        ),
        (
            "feature.fillet",
            json!({ "edges": [["Edge:copy(0)/Base:face(1)", "Base:face(3)"]], "radius": 1, "name": "Round" }),
        ),
    ];
    for (id, params) in steps {
        let made = run(&mut s, id, params.clone());
        assert_eq!(made["status"], "ok", "{id} {params}: {made}");
    }
}

/// An unknown feature name, one two features share, and a bare cap where
/// the extrusion has two: clear errors, nothing added.
#[test]
fn unknown_or_ambiguous_names_are_refused_clearly() {
    let mut s = session();
    plate_and_bar(&mut s);
    let message = error(
        &mut s,
        "feature.fillet",
        json!({ "edges": [["Plat:endCap", "Plate:side(e5)"]], "radius": 1 }),
    );
    assert!(message.contains("no feature is called `Plat`"), "{message}");
    let message = error(
        &mut s,
        "select.set",
        json!({ "names": ["F2:copy(1)/Nobody:side(e2)"] }),
    );
    assert!(message.contains("`Nobody`"), "{message}");
    // A name is free text: a feature may be called so, and it is kept.
    let named = run(
        &mut s,
        "feature.sphere",
        json!({ "radius": 1, "name": "Plate:face(1)" }),
    );
    assert_eq!(named["name"], "Plate:face(1)", "{named}");
    // Two profiles extruded at once: "the" end cap means two faces.
    run(&mut s, "sketch.create", json!({ "name": "Two" }));
    for x in [50, 60] {
        run(
            &mut s,
            "sketch.rect",
            json!({ "sketch": "Two", "from": [x, 0], "to": [x + 5, 5] }),
        );
    }
    run(
        &mut s,
        "feature.extrude",
        json!({ "sketch": "Two", "distance": 2, "name": "Pads" }),
    );
    error(&mut s, "select.set", json!({ "names": ["Pads:endCap"] }));
    let failed = run(
        &mut s,
        "feature.offsetFace",
        json!({ "faces": ["Pads:endCap"], "distance": 1 }),
    );
    assert_eq!(failed["status"], "failed", "{failed}");
    assert!(
        failed["error"]
            .as_str()
            .unwrap()
            .contains("names the caps of 2 loops"),
        "{failed}"
    );
}

fn undo_steps(s: &mut Session) -> usize {
    run(s, "document.inspect", json!({ "measures": false }))["history"]["undo"]
        .as_array()
        .unwrap()
        .len()
}

/// The selection's items name a face by a feature's name too; and what
/// is no face name (a table's cell, a choice's label, a label) stays as it
/// was written even when it reads like one.
#[test]
fn only_parameters_that_hold_names_are_looked_up() {
    let mut s = session();
    plate_and_bar(&mut s);
    run(
        &mut s,
        "feature.boolean",
        json!({ "op": "union", "target": "Plate", "tool": "Bar", "name": "Joined" }),
    );
    let selected = run(
        &mut s,
        "select.set",
        json!({ "items": [{ "kind": "face", "name": "Joined:copy(1)/Bar:endCap", "feature": "Joined" }] }),
    );
    assert_eq!(
        selected["items"][0]["name"], "F5:copy(1)/F4:endCap(e2)",
        "{selected}"
    );
    let message = error(
        &mut s,
        "select.set",
        json!({ "items": [{ "kind": "face", "name": "Nobody:face(1)" }] }),
    );
    assert!(
        message.contains("no feature is called `Nobody`"),
        "{message}"
    );
    // A table's cells, a choice's labels and a label are text.
    let table = run(
        &mut s,
        "table.set",
        json!({ "name": "t", "columns": ["key", "v"], "rows": [["Plate:face(1)", 1], ["b", 2]] }),
    );
    assert_eq!(table["keys"], json!(["Plate:face(1)", "b"]), "{table}");
    run(
        &mut s,
        "variable.set",
        json!({ "name": "pick", "expr": "1" }),
    );
    run(
        &mut s,
        "variable.publish",
        json!({ "name": "pick", "kind": "choice", "choices": ["Plate:face(1)", "Joined:face(0)"], "label": "Bar:endCap" }),
    );
    let list = run(&mut s, "param.list", json!({}));
    let pick = &list["parameters"][0];
    assert_eq!(
        pick["choices"],
        json!(["Plate:face(1)", "Joined:face(0)"]),
        "{list}"
    );
    assert_eq!(pick["label"], "Bar:endCap", "{list}");
}

/// The command's error shows a name as the script wrote it, not with ids.
#[test]
fn errors_show_names_as_written() {
    let mut s = session();
    plate_and_bar(&mut s);
    // The plate has no side of a curve e99.
    let message = error(
        &mut s,
        "select.set",
        json!({ "names": ["Plate:side(e99)"] }),
    );
    assert!(message.contains("Plate:side(e99)"), "{message}");
    assert!(!message.contains("F2:side(e99)"), "{message}");
}

fn centroid(s: &mut Session) -> [f64; 3] {
    let c = run(s, "document.measure", json!({}))["centroid"].clone();
    [0, 1, 2].map(|i| c[i].as_f64().unwrap())
}

fn near(a: [f64; 3], b: [f64; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9)
}

/// Item 2: an axis from expressions, plain numbers or lengths, follows
/// its variables; one of no length fails the feature with the reason.
#[test]
fn an_axis_follows_expressions() {
    let mut s = session();
    run(&mut s, "file.new", json!({ "units": "mm" }));
    run(
        &mut s,
        "variable.set",
        json!({ "name": "wx", "expr": "30 mm" }),
    );
    run(
        &mut s,
        "variable.set",
        json!({ "name": "wy", "expr": "0 mm" }),
    );
    let made = run(
        &mut s,
        "feature.cylinder",
        json!({ "radius": 1, "height": 10, "axis": ["wx", "wy", 0], "name": "Peg" }),
    );
    assert_eq!(made["status"], "ok", "{made}");
    assert!(
        near(centroid(&mut s), [5.0, 0.0, 0.0]),
        "{:?}",
        centroid(&mut s)
    );
    run(
        &mut s,
        "variable.set",
        json!({ "name": "wx", "expr": "0 mm" }),
    );
    run(
        &mut s,
        "variable.set",
        json!({ "name": "wy", "expr": "2 cm" }),
    );
    assert!(
        near(centroid(&mut s), [0.0, 5.0, 0.0]),
        "{:?}",
        centroid(&mut s)
    );
    // Ratios work as before.
    run(
        &mut s,
        "variable.set",
        json!({ "name": "a", "expr": "90 deg" }),
    );
    run(
        &mut s,
        "feature.edit",
        json!({ "feature": "Peg", "axis": ["cos(a)", "sin(a)", 0] }),
    );
    assert!(
        near(centroid(&mut s), [0.0, 5.0, 0.0]),
        "{:?}",
        centroid(&mut s)
    );
    // A transform's axis too.
    let turned = run(
        &mut s,
        "feature.transform",
        json!({ "source": "Peg", "angle": "90 deg", "axis": ["0 mm", "0 mm", "wy"], "name": "Turned" }),
    );
    assert_eq!(turned["status"], "ok", "{turned}");
    assert!(
        near(centroid(&mut s), [-5.0, 0.0, 0.0]),
        "{:?}",
        centroid(&mut s)
    );
    // No length: the feature fails and says why.
    run(
        &mut s,
        "variable.set",
        json!({ "name": "wy", "expr": "0 mm" }),
    );
    let failed = run(&mut s, "document.inspect", json!({}));
    let turned = failed["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == "Turned")
        .unwrap()
        .clone();
    assert_eq!(turned["status"], "failed", "{turned}");
    assert!(
        turned["error"]
            .as_str()
            .unwrap()
            .contains("which has no length"),
        "{turned}"
    );
    // An angle is no direction component.
    let wrong = run(
        &mut s,
        "feature.block",
        json!({ "size": [1, 1, 1], "axis": ["1 deg", 0, 1] }),
    );
    assert_eq!(wrong["status"], "failed", "{wrong}");
    assert!(
        wrong["error"].as_str().unwrap().contains("axis.x"),
        "{wrong}"
    );
}

/// Item 3: `m` (the metre) and `mod` (the function) cannot name a
/// variable: refused at once, naming the clash and a name to use.
#[test]
fn reserved_variable_names_are_refused_with_the_clash() {
    let mut s = session();
    run(&mut s, "file.new", json!({ "units": "mm" }));
    let metre = error(
        &mut s,
        "variable.set",
        json!({ "name": "m", "expr": "1.5 mm" }),
    );
    assert!(metre.contains("the unit metre"), "{metre}");
    assert!(metre.contains("`module`"), "{metre}");
    let modulo = error(
        &mut s,
        "variable.set",
        json!({ "name": "mod", "expr": "2" }),
    );
    assert!(modulo.contains("the function `mod(...)`"), "{modulo}");
    run(
        &mut s,
        "variable.set",
        json!({ "name": "module", "expr": "1.5 mm" }),
    );
    let renamed = error(
        &mut s,
        "variable.rename",
        json!({ "name": "module", "to": "m" }),
    );
    assert!(renamed.contains("the unit metre"), "{renamed}");
    let inspected = run(&mut s, "document.inspect", json!({}));
    let names: Vec<&str> = inspected["variables"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["module"]);
}

/// Item 4: an edit's result says when assertions are still pending, and
/// `wait: true` gives their final results.
#[test]
fn an_edit_can_wait_for_its_assertions() {
    let mut s = session();
    run(&mut s, "file.new", json!({ "units": "mm" }));
    run(
        &mut s,
        "variable.set",
        json!({ "name": "width", "expr": "10 mm" }),
    );
    run(
        &mut s,
        "variable.publish",
        json!({ "name": "width", "min": "1 mm", "max": "100 mm" }),
    );
    run(
        &mut s,
        "feature.block",
        json!({ "size": ["width", 10, 10], "name": "B" }),
    );
    run(
        &mut s,
        "assert.add",
        json!({ "name": "wide", "measure": "extent", "of": "B", "axis": "x", "eq": "width" }),
    );
    let set = run(
        &mut s,
        "param.set",
        json!({ "values": { "width": "20 mm" }, "wait": true }),
    );
    let assertions = &set["assertionSummary"];
    assert_eq!(assertions["pending"], 0, "{set}");
    assert_eq!(assertions["passed"], 1, "{set}");
    assert!(assertions.get("whenPending").is_none(), "{set}");
    let set = run(
        &mut s,
        "variable.set",
        json!({ "name": "width", "expr": "30 mm", "wait": true }),
    );
    assert_eq!(set["assertionSummary"]["pending"], 0, "{set}");
    assert_eq!(set["assertionSummary"]["passed"], 1, "{set}");
    error(
        &mut s,
        "param.set",
        json!({ "values": { "width": "20 mm" }, "wait": "yes" }),
    );
}

/// Item 5: `when` with the command that adds a feature: switched off from
/// the first regeneration, one undo step, and a feature that could not be
/// built while off never fails.
#[test]
fn a_condition_given_with_the_feature_holds_from_the_start() {
    let mut s = session();
    run(&mut s, "file.new", json!({ "units": "mm" }));
    run(
        &mut s,
        "variable.set",
        json!({ "name": "lid", "expr": "0" }),
    );
    run(
        &mut s,
        "feature.block",
        json!({ "size": [20, 20, 10], "name": "Box" }),
    );
    let steps_before = undo_steps(&mut s);
    let lid = run(
        &mut s,
        "feature.cylinder",
        json!({ "radius": 3, "height": 2, "position": [10, 10, 10], "name": "Lid", "when": "lid" }),
    );
    assert_eq!(lid["status"], "inactive", "{lid}");
    // One undo step, the feature and its condition together.
    assert_eq!(undo_steps(&mut s), steps_before + 1);
    assert_eq!(lid["when"], "lid", "{lid}");
    // A chamfer of the lid's edge: inactive at once, never failed.
    let edge = run(
        &mut s,
        "feature.chamfer",
        json!({ "edges": [["Lid:face(0)", "Lid:face(2)"]], "distance": "0.5 mm", "name": "LidEdge", "when": "lid" }),
    );
    assert_eq!(edge["status"], "inactive", "{edge}");
    assert!(
        edge.get("failed")
            .is_none_or(|f| f.as_array().unwrap().is_empty()),
        "{edge}"
    );
    run(
        &mut s,
        "variable.set",
        json!({ "name": "lid", "expr": "1" }),
    );
    let inspected = run(&mut s, "document.inspect", json!({}));
    for name in ["Lid", "LidEdge"] {
        let feature = inspected["features"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["name"] == name)
            .unwrap();
        assert_eq!(feature["status"], "ok", "{feature}");
    }
    // One undo step takes the feature and its condition away together.
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    let inspected = run(&mut s, "document.inspect", json!({}));
    assert!(
        inspected["features"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["name"] != "LidEdge"),
        "{inspected}"
    );
    // An extrusion with `cut` adds two features; both get the condition.
    run(
        &mut s,
        "sketch.create",
        json!({ "name": "Hole", "plane": "xy", "offset": 10 }),
    );
    run(
        &mut s,
        "sketch.circle",
        json!({ "sketch": "Hole", "center": [5, 5], "radius": 1 }),
    );
    let steps_before = undo_steps(&mut s);
    let cut = run(
        &mut s,
        "feature.extrude",
        json!({ "sketch": "Hole", "distance": 3, "cut": "Box", "name": "Drilled", "when": 0 }),
    );
    assert_eq!(cut["status"], "inactive", "{cut}");
    assert_eq!(cut["sweep"]["status"], "inactive", "{cut}");
    assert_eq!(undo_steps(&mut s), steps_before + 1);
    // null is always on.
    let on = run(
        &mut s,
        "feature.sphere",
        json!({ "radius": 1, "name": "Ball", "when": null }),
    );
    assert_eq!(on["status"], "ok", "{on}");
    assert!(on.get("when").is_none(), "{on}");
}

/// A bad condition is refused before anything is added; a command that
/// adds no feature has no `when`.
#[test]
fn a_bad_condition_is_refused() {
    let mut s = session();
    run(&mut s, "file.new", json!({ "units": "mm" }));
    for bad in [json!("1 +"), json!([1]), json!({ "a": 1 }), json!(true)] {
        let message = error(
            &mut s,
            "feature.block",
            json!({ "size": [1, 1, 1], "when": bad }),
        );
        assert!(message.contains("`when`"), "{bad}: {message}");
    }
    let inspected = run(&mut s, "document.inspect", json!({}));
    assert_eq!(inspected["features"], json!([]), "{inspected}");
    let message = error(&mut s, "sketch.create", json!({ "name": "S", "when": "1" }));
    assert!(message.contains("unknown parameter `when`"), "{message}");
}

/// A command opts in to feature names in its face and edge names by
/// listing the parameters that hold them; every command whose parameters
/// take a face, edge or vertex name, a subject or a filter lists them, and
/// every key listed is one of its parameters.
#[test]
fn every_command_that_takes_names_lists_them() {
    let takes_names = [
        "face name",
        "edge name",
        "vertex name",
        "persistent name",
        "subject",
        "filter",
    ];
    for spec in Session::commands() {
        if takes_names.iter().any(|t| spec.params.contains(t)) {
            assert!(
                !spec.names.is_empty(),
                "{} takes names but lists none",
                spec.id
            );
        }
        if spec.id == "assert.edit" {
            continue;
        }
        for key in spec.names {
            assert!(
                spec.params.contains(&format!("\"{key}\"")),
                "{} lists `{key}`, which it does not take",
                spec.id
            );
        }
    }
}

/// A script that names its features by name builds the same part when a
/// step is added before it (which gives every later feature another id),
/// and when that step is inserted at the rollback marker afterwards
/// (which changes no id that was there): references by name do not shift.
#[test]
fn a_script_by_names_is_stable_when_a_step_is_added_before_it() {
    let script = |s: &mut Session| {
        run(
            s,
            "feature.block",
            json!({ "size": [20, 10, 6], "name": "Cube" }),
        );
        run(
            s,
            "feature.fillet",
            json!({ "edges": [["Cube:face(2)", "Cube:face(4)"]], "radius": 1, "name": "Round" }),
        );
        run(
            s,
            "sketch.create",
            json!({ "face": "Cube:face(1)", "name": "Top" }),
        );
        run(
            s,
            "assert.add",
            json!({ "name": "faces", "measure": "count", "of": {"kind": "face", "where": {"in": "Round"}}, "min": 1 }),
        );
    };
    let outcome = |s: &mut Session| {
        let inspect = run(s, "document.inspect", json!({}));
        let volume = inspect["bodies"][0]["volume"].as_f64().unwrap();
        let faces = run(s, "assert.list", json!({}))["assertions"][0]["measured"].clone();
        (volume, faces, inspect["ok"].clone())
    };
    let mut plain = session();
    run(&mut plain, "file.new", json!({ "units": "mm" }));
    script(&mut plain);
    let want = outcome(&mut plain);
    assert_eq!(want.2, true, "{want:?}");
    // A step added first: every id of the script's features is one more.
    let mut shifted = session();
    run(&mut shifted, "file.new", json!({ "units": "mm" }));
    run(
        &mut shifted,
        "datum.plane",
        json!({ "from": "xy", "distance": 50, "name": "Aside" }),
    );
    script(&mut shifted);
    assert_eq!(outcome(&mut shifted), want);
    // The same step inserted at the marker into the plain build: the
    // stored ids stay as they were, and so does the part.
    run(&mut plain, "history.rollback", json!({ "feature": "Cube" }));
    run(
        &mut plain,
        "datum.plane",
        json!({ "from": "xy", "distance": 50, "name": "Aside" }),
    );
    run(&mut plain, "history.rollback", json!({ "feature": null }));
    assert_eq!(outcome(&mut plain), want);
}
