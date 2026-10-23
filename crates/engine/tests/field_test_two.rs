//! What the second field test (`docs/field-test/2026-10-22/`) found an
//! agent could misread or could not act on, each pinned by the smallest
//! session that showed it: the advice when a body already went into
//! another feature (a feature inserted at the rollback marker), the name
//! of the feature an assertion's body went into, units written in the
//! wrong place or spelled otherwise, an angle's value in radians, a
//! variable defined from itself, negative wall thicknesses, the edges of a
//! fillet found by `madeBy`, the frame of a sketch on a face, the article
//! and number of the sketch messages, and the commands to fix a conflict.
//!
//! The messages are pinned by their words: these are regression tests,
//! not proofs, except where a value is checked against a closed form.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::run;
use common::refused;
use serde_json::{Value, json};

fn session() -> craft3d_engine::Session {
    let mut s = common::session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    s
}

fn failed_of<'a>(result: &'a Value, feature: &str) -> &'a str {
    result["failed"]
        .as_array()
        .and_then(|all| all.iter().find(|f| f["feature"] == feature))
        .and_then(|f| f["error"].as_str())
        .unwrap_or_else(|| panic!("{feature} is not under failed: {result}"))
}

/// A plate with a boss and a hole: the field test's part.
fn plate(s: &mut craft3d_engine::Session) {
    run(s, "variable.set", json!({"name": "t", "expr": "8 mm"}));
    run(
        s,
        "feature.block",
        json!({"size": ["60 mm", "40 mm", "t"], "name": "Plate"}),
    );
    run(
        s,
        "feature.cylinder",
        json!({"radius": "8 mm", "height": "10 mm", "position": ["30 mm", "20 mm", "t"], "name": "Boss"}),
    );
    run(
        s,
        "feature.boolean",
        json!({"op": "union", "target": "Plate", "tool": "Boss", "name": "Bossed"}),
    );
    run(
        s,
        "feature.cylinder",
        json!({"radius": "3 mm", "height": "t + 12 mm", "position": ["30 mm", "20 mm", "-1 mm"], "name": "HoleTool"}),
    );
    run(
        s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Bossed", "tool": "HoleTool", "name": "Drilled"}),
    );
    run(
        s,
        "feature.fillet",
        json!({"edges": [["F1:face(2)", "F1:face(4)"]], "radius": "3 mm", "name": "Round"}),
    );
}

#[test]
fn a_feature_inserted_at_the_marker_says_how_to_rethread_the_ones_after_it() {
    let mut s = session();
    plate(&mut s);
    run(&mut s, "history.rollback", json!({"feature": "Bossed"}));
    run(
        &mut s,
        "feature.chamfer",
        json!({"edges": [["F1:face(1)", "F1:face(3)"]], "distance": "1 mm", "name": "Bevel"}),
    );
    let rolled = run(&mut s, "history.rollback", json!({"feature": null}));
    let error = failed_of(&rolled, "Drilled");
    // The parameter to change and its new value, and no `keep`: a chamfer
    // has none.
    assert!(
        error.contains("build on `Bevel` instead (feature.edit with \"target\": \"Bevel\")"),
        "{error}"
    );
    assert!(!error.contains("keep"), "{error}");
    // The fillet after the marker names faces, not a body: it follows them
    // into the chamfer's body by itself and is not among the failures.
    assert!(
        rolled["failed"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["feature"] != "Round"),
        "{rolled}"
    );
    // Doing what it says repairs the part, the fillet with it.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Drilled", "target": "Bevel"}),
    );
    assert_eq!(run(&mut s, "document.check", json!({}))["ok"], true);
    let round = run(&mut s, "document.inspect", json!({"measures": false}))["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == "Round")
        .cloned()
        .unwrap();
    assert_eq!(round["status"], "ok", "{round}");

    // A transform that took the body: `keep` is offered.
    let mut s = session();
    run(
        &mut s,
        "feature.block",
        json!({"size": [10, 10, 10], "name": "Cube"}),
    );
    run(
        &mut s,
        "feature.transform",
        json!({"source": "Cube", "translate": [20, 0, 0], "name": "Moved"}),
    );
    let mirror = run(
        &mut s,
        "feature.mirror",
        json!({"source": "Cube", "normal": [1, 0, 0], "name": "Mirrored"}),
    );
    let error = mirror["error"].as_str().unwrap();
    assert!(
        error.contains("feature.edit with \"source\": \"Moved\""),
        "{error}"
    );
    assert!(error.contains("or give `Moved` keep: true"), "{error}");
}

#[test]
fn an_assertion_on_a_body_that_went_on_names_the_feature_it_went_into() {
    let mut s = session();
    run(
        &mut s,
        "feature.block",
        json!({"size": [10, 10, 10], "name": "Box"}),
    );
    run(
        &mut s,
        "assert.add",
        json!({"name": "volume", "measure": "volume", "of": "Box", "eq": 1000}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": 2, "height": 12, "position": [5, 5, -1], "name": "Drill"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Box", "tool": "Drill", "name": "Drilled"}),
    );
    let list = run(&mut s, "assert.list", json!({}));
    let message = list["assertions"][0]["message"].as_str().unwrap();
    assert!(
        message.contains("went into F3, `Drilled`; name that feature"),
        "{message}"
    );
}

#[test]
fn a_unit_in_the_wrong_place_or_spelled_otherwise_says_what_to_write() {
    let mut s = session();
    let set = |s: &mut craft3d_engine::Session, expr: &str| {
        run(s, "variable.set", json!({"name": "v", "expr": expr}))
    };
    let after = set(&mut s, "(pi / 4) rad");
    assert!(
        after["error"]
            .as_str()
            .unwrap()
            .contains("multiply by one: `(pi / 4) * 1 rad`"),
        "{after}"
    );
    let inch = set(&mut s, "2 inch");
    assert!(
        inch["error"]
            .as_str()
            .unwrap()
            .contains("`inch` is not a unit: write `in`"),
        "{inch}"
    );
    // A unit read as belonging to a divisor: kept (it is a valid inverse
    // length) with a warning, and refused where a length is needed with
    // the same advice.
    let sixteenth = set(&mut s, "1/16 in");
    assert_eq!(sixteenth["dimension"], "length^-1", "{sixteenth}");
    assert!(
        sixteenth["warning"]
            .as_str()
            .unwrap()
            .contains("write `1 in / 16`"),
        "{sixteenth}"
    );
    let block = run(
        &mut s,
        "feature.block",
        json!({"size": ["1/16 in", 10, 10], "name": "Thin"}),
    );
    let error = block["error"].as_str().unwrap();
    assert!(
        error.contains("is a length^-1, but a length is needed"),
        "{error}"
    );
    assert!(error.contains("write `1 in / 16`"), "{error}");
    // Inverses written as meant are not warned of, and their refusal
    // where a length is needed gives no such advice.
    for meant in ["10 mm / (360 deg)", "1 / (2 mm)"] {
        let result = set(&mut s, meant);
        assert!(result.get("warning").is_none(), "{meant}: {result}");
    }
    let inspected = run(&mut s, "document.inspect", json!({"measures": false}));
    assert!(
        inspected["variables"][0].get("warning").is_none(),
        "{inspected}"
    );
    let thin = run(
        &mut s,
        "feature.block",
        json!({"size": ["v", 10, 10], "name": "Thin2"}),
    );
    let error = thin["error"].as_str().unwrap();
    assert!(
        error.contains("is a length^-1, but a length is needed"),
        "{error}"
    );
    assert!(!error.contains("belongs to the number"), "{error}");
    // Written as meant: 1.5875 mm, built.
    // Written as meant, `v` builds Thin2 (Thin, written inline, still fails).
    set(&mut s, "1 in / 16");
    let measured = run(&mut s, "document.measure", json!({}));
    let volume = measured["volume"].as_f64().unwrap();
    assert!((volume - 25.4 / 16.0 * 100.0).abs() < 1e-9, "{volume}");
}

#[test]
fn an_angle_variable_says_its_value_is_in_radians_and_gives_degrees() {
    let mut s = session();
    let angle = run(
        &mut s,
        "variable.set",
        json!({"name": "a", "expr": "45 deg + 0.1 rad"}),
    );
    assert_eq!(angle["angleUnit"], "rad", "{angle}");
    let value = angle["value"].as_f64().unwrap();
    let degrees = angle["degrees"].as_f64().unwrap();
    assert!((value - (std::f64::consts::FRAC_PI_4 + 0.1)).abs() < 1e-12);
    assert!((degrees - (45.0 + 0.1_f64.to_degrees())).abs() < 1e-9);
    // A length has neither.
    let length = run(
        &mut s,
        "variable.set",
        json!({"name": "w", "expr": "40 mm"}),
    );
    assert!(length.get("angleUnit").is_none() && length.get("degrees").is_none());
}

#[test]
fn a_variable_defined_from_itself_is_refused_with_its_current_value() {
    let mut s = session();
    run(
        &mut s,
        "variable.set",
        json!({"name": "wall", "expr": "2 mm"}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"size": [10, 10, "wall"], "name": "Plate"}),
    );
    let error = refused(
        &mut s,
        "variable.set",
        json!({"name": "wall", "expr": "wall + 1 mm"}),
    );
    assert!(error.contains("uses `wall` itself"), "{error}");
    assert!(error.contains("(it is `2 mm` now)"), "{error}");
    // Nothing changed: the plate is still 2 mm thick.
    let check = run(&mut s, "document.check", json!({}));
    assert_eq!(check["ok"], true, "{check}");
    let volume = run(&mut s, "document.measure", json!({}))["volume"]
        .as_f64()
        .unwrap();
    assert!((volume - 200.0).abs() < 1e-9, "{volume}");
    // A cycle through another variable is kept: both fail with a
    // circular definition, until one is changed.
    run(
        &mut s,
        "variable.set",
        json!({"name": "a", "expr": "b + 1 mm"}),
    );
    let b = run(
        &mut s,
        "variable.set",
        json!({"name": "b", "expr": "a + 1 mm"}),
    );
    assert!(
        b["error"].as_str().unwrap().contains("circular definition"),
        "{b}"
    );
    let inspected = run(&mut s, "document.inspect", json!({"measures": false}));
    for name in ["a", "b"] {
        let variable = inspected["variables"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["name"] == name)
            .unwrap();
        assert!(
            variable["error"]
                .as_str()
                .unwrap()
                .contains("circular definition"),
            "{variable}"
        );
    }
    let b = run(&mut s, "variable.set", json!({"name": "b", "expr": "2 mm"}));
    assert_eq!(b["value"], 2.0, "{b}");
}

#[test]
fn a_negative_wall_says_it_has_to_be_more_than_zero() {
    let mut s = session();
    run(
        &mut s,
        "feature.block",
        json!({"size": [20, 20, 20], "name": "Cube"}),
    );
    let shell = run(
        &mut s,
        "feature.shell",
        json!({"source": "Cube", "thickness": "-2 mm", "open": ["F1:face(1)"]}),
    );
    let error = shell["error"].as_str().unwrap();
    assert!(
        error.contains("has to be more than zero, got -2 mm"),
        "{error}"
    );
    let thicken = run(
        &mut s,
        "feature.thicken",
        json!({"faces": ["F1:face(1)"], "thickness": "-2 mm"}),
    );
    let error = thicken["error"].as_str().unwrap();
    assert!(error.contains("has to be more than zero"), "{error}");
    assert!(error.contains("`side`"), "{error}");
}

#[test]
fn the_edges_of_a_fillet_are_made_by_it() {
    let mut s = session();
    run(
        &mut s,
        "feature.block",
        json!({"size": [20, 20, 20], "name": "Cube"}),
    );
    run(
        &mut s,
        "feature.fillet",
        json!({"edges": [["F1:face(2)", "F1:face(4)"]], "radius": "3 mm", "name": "Round"}),
    );
    // A hole after it, so that the fillet is not the last feature: the
    // edges of the part are then named by the boolean (`F4:between[…]`),
    // and only their faces tell what made them.
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": 2, "height": 22, "position": [10, 10, -1], "name": "Drill"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Round", "tool": "Drill", "name": "Part"}),
    );
    // The fillet's face is bounded by four edges: along the two faces it
    // joins and across the top and the bottom.
    let edges = run(
        &mut s,
        "select.query",
        json!({"kind": "edge", "where": {"madeBy": "Round"}}),
    );
    assert_eq!(edges["count"], 4, "{edges}");
    for item in edges["items"].as_array().unwrap() {
        let name = item["name"].as_str().unwrap();
        assert!(name.contains("F2:fillet(0)"), "{name}");
    }
    // Every edge of the cube is made by the block.
    let all = run(
        &mut s,
        "select.query",
        json!({"kind": "edge", "where": {"madeBy": "Cube"}}),
    );
    let total = run(&mut s, "document.edges", json!({"feature": "Part"}))["count"].clone();
    assert_eq!(all["count"], total, "{all}");
}

#[test]
fn a_sketch_on_a_face_says_where_its_axes_and_normal_point() {
    let mut s = session();
    run(
        &mut s,
        "feature.block",
        json!({"size": [70, 50, 25], "name": "Box"}),
    );
    // The bottom faces down: the sketch's normal is world -Z, and its x
    // and y axes are world axes of the plane, whichever way round.
    let created = run(
        &mut s,
        "sketch.create",
        json!({"name": "Under", "face": "F1:face(0)"}),
    );
    assert_eq!(created["normal"], json!([0.0, 0.0, -1.0]), "{created}");
    let x = &created["frame"]["x"];
    let y = &created["frame"]["y"];
    assert_eq!(x[2], 0.0, "{created}");
    assert_eq!(y[2], 0.0, "{created}");
    let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "Under"}));
    assert_eq!(inspected["frame"], created["frame"]);
    assert_eq!(inspected["normal"], created["normal"]);
    // On the xy plane: the world's own axes.
    let plain = run(
        &mut s,
        "sketch.create",
        json!({"name": "Flat", "plane": "xy"}),
    );
    assert_eq!(
        plain["frame"],
        json!({"origin": [0.0, 0.0, 0.0], "x": [1.0, 0.0, 0.0], "y": [0.0, 1.0, 0.0]})
    );
    assert_eq!(plain["normal"], json!([0.0, 0.0, 1.0]));
}

#[test]
fn sketch_messages_use_the_right_article_and_name_the_commands_to_fix_a_conflict() {
    let mut s = session();
    run(&mut s, "sketch.create", json!({"name": "S", "plane": "xy"}));
    run(
        &mut s,
        "sketch.line",
        json!({"sketch": "S", "from": [0, 0], "to": [10, 0]}),
    );
    let angle = refused(
        &mut s,
        "sketch.constrain",
        json!({"sketch": "S", "type": "angle", "entities": ["e3"], "value": "10 deg"}),
    );
    assert!(
        angle.contains("an angle constraint takes a line and a line, but 1 entity was given"),
        "{angle}"
    );
    run(
        &mut s,
        "sketch.constrain",
        json!({"sketch": "S", "type": "distance", "entities": ["e3"], "value": "10 mm"}),
    );
    let conflict = run(
        &mut s,
        "sketch.constrain",
        json!({"sketch": "S", "type": "distance", "entities": ["e3"], "value": "12 mm"}),
    );
    let error = conflict["error"].as_str().unwrap();
    assert!(
        error.contains("change or delete it (sketch.edit with a new value, or sketch.delete)"),
        "{error}"
    );
    // A literal angle for a length is refused at the command since
    // 2026-10-23 (roadmap R1; it was added and failed the sketch, with the
    // same words).
    let error = refused(
        &mut s,
        "sketch.constrain",
        json!({"sketch": "S", "type": "x", "entities": ["e1"], "value": "10 deg"}),
    );
    assert!(
        error.contains("of an x constraint is an angle, but a length is needed"),
        "{error}"
    );
}
