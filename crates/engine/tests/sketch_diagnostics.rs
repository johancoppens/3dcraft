//! What a sketch says about itself beyond its statuses (R1, the second
//! field test, 2026-10-22): which constraints a conflicting one
//! contradicts, a line the solver shrank to a point under two directions,
//! degrees of freedom left as a warning of `document.check`, and literal
//! values a constraint can never take refused at the command.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use common::{refused, session};
use craft3d_engine::Session;
use serde_json::{Value, json};

fn constrain(s: &mut Session, kind: &str, entities: Value, value: Option<&str>) -> Value {
    let mut params = json!({"sketch": "S", "type": kind, "entities": entities});
    if let Some(value) = value {
        params["value"] = json!(value);
    }
    run(s, "sketch.constrain", params)
}

/// A sketch `S` with a line e3 from e1 to e2, e1 fixed (c1).
fn line_sketch(s: &mut Session) {
    run(s, "file.new", json!({"units": "mm"}));
    run(s, "sketch.create", json!({"name": "S", "plane": "xy"}));
    let line = run(
        s,
        "sketch.line",
        json!({"sketch": "S", "from": [0, 0], "to": [8, 3]}),
    );
    assert_eq!(line["entity"], "e3");
    constrain(s, "fixed", json!(["e1"]), None);
}

#[test]
fn a_conflicting_constraint_names_the_ones_it_contradicts() {
    let mut s = session();
    line_sketch(&mut s);
    // c2 horizontal; c3 the length 10; c4 vertical contradicts c2 (the
    // line would have no length) and c3 together.
    constrain(&mut s, "horizontal", json!(["e3"]), None);
    constrain(&mut s, "distance", json!(["e3"]), Some("10 mm"));
    let bad = constrain(&mut s, "vertical", json!(["e3"]), None);
    assert_eq!(bad["conflicting"], json!(["c4"]), "{bad}");
    assert_eq!(
        bad["conflicts"],
        json!([{"constraint": "c4", "contradicts": ["c2", "c3"]}]),
        "{bad}"
    );
    run(&mut s, "edit.undo", json!({}));

    // Two lengths: the second contradicts the first only; the fixed
    // point and the direction take no part.
    let bad = constrain(&mut s, "distance", json!(["e3"]), Some("12 mm"));
    assert_eq!(bad["conflicting"], json!(["c4"]), "{bad}");
    assert_eq!(
        bad["conflicts"],
        json!([{"constraint": "c4", "contradicts": ["c3"]}]),
        "{bad}"
    );
    // Finding them leaves the document alone: one undo takes the
    // conflict away, and the sketch is fine again.
    run(&mut s, "edit.undo", json!({}));
    let check = run(&mut s, "document.check", json!({}));
    assert_eq!(check["ok"], true, "{check}");
    // Without a conflict there is nothing to name.
    let fine = constrain(&mut s, "distance", json!(["e3"]), Some("10 mm"));
    assert_eq!(fine["redundant"], json!(["c4"]), "{fine}");
    assert!(fine.get("conflicts").is_none(), "{fine}");
}

#[test]
fn horizontal_and_vertical_on_one_line_are_reported_as_a_contradiction() {
    let mut s = session();
    line_sketch(&mut s);
    constrain(&mut s, "horizontal", json!(["e3"]), None);
    let both = constrain(&mut s, "vertical", json!(["e3"]), None);
    // The solve is as it was (both hold on a point), and the result says
    // what happened to the line.
    assert_eq!(both["status"], "ok", "{both}");
    assert_eq!(both["collapsed"][0]["line"], "e3", "{both}");
    assert_eq!(
        both["collapsed"][0]["constraints"],
        json!(["c2", "c3"]),
        "{both}"
    );
    let check = run(&mut s, "document.check", json!({}));
    assert_eq!(check["ok"], false, "{check}");
    let problem = &check["problems"][0];
    assert_eq!(problem["feature"], "F1", "{check}");
    assert_eq!(problem["line"], "e3", "{check}");
    assert!(
        problem["problem"]
            .as_str()
            .unwrap()
            .contains("has shrunk to a point"),
        "{check}"
    );
    // A line of its own length with one direction is fine.
    run(&mut s, "edit.undo", json!({}));
    let one = constrain(&mut s, "distance", json!(["e3"]), Some("5 mm"));
    assert!(one.get("collapsed").is_none(), "{one}");
}

#[test]
fn an_under_constrained_sketch_is_a_warning_of_document_check() {
    let mut s = session();
    line_sketch(&mut s);
    // e1 fixed: the other end is free (2 degrees of freedom).
    let check = run(&mut s, "document.check", json!({}));
    assert_eq!(check["ok"], true, "{check}");
    let warnings = check["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 1, "{check}");
    assert_eq!(warnings[0]["feature"], "F1");
    assert_eq!(warnings[0]["dof"], 2);
    assert!(
        warnings[0]["warning"]
            .as_str()
            .unwrap()
            .contains("2 degrees of freedom left"),
        "{check}"
    );
    // Fully constrained: no warning.
    constrain(&mut s, "horizontal", json!(["e3"]), None);
    constrain(&mut s, "distance", json!(["e3"]), Some("10 mm"));
    let check = run(&mut s, "document.check", json!({}));
    assert!(check.get("warnings").is_none(), "{check}");
}

#[test]
fn literal_values_a_constraint_can_never_take_are_refused() {
    let mut s = session();
    line_sketch(&mut s);
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "S", "center": [20, 0], "radius": 2}),
    );
    run(
        &mut s,
        "sketch.line",
        json!({"sketch": "S", "from": [0, 10], "to": [8, 13]}),
    );
    let before = run(&mut s, "sketch.inspect", json!({"sketch": "S"}));
    for (kind, entities, value, says) in [
        ("distance", json!(["e3"]), "-4 mm", "cannot be negative"),
        ("radius", json!(["e5"]), "-2", "cannot be negative"),
        ("diameter", json!(["e5"]), "-1 cm", "cannot be negative"),
        ("distance", json!(["e3"]), "30 deg", "a length is needed"),
        ("angle", json!(["e3", "e8"]), "5 mm", "an angle is needed"),
        ("x", json!(["e2"]), "90 deg", "a length is needed"),
    ] {
        let error = refused(
            &mut s,
            "sketch.constrain",
            json!({"sketch": "S", "type": kind, "entities": entities, "value": value}),
        );
        assert!(error.contains(says), "{kind} {value}: {error}");
    }
    // Nothing was added.
    assert_eq!(
        run(&mut s, "sketch.inspect", json!({"sketch": "S"})),
        before
    );
    // Values that can be: a negative coordinate, an angle as a plain
    // number (degrees) or with its unit, zero.
    constrain(&mut s, "x", json!(["e2"]), Some("-3 mm"));
    constrain(&mut s, "angle", json!(["e3", "e8"]), Some("0"));
    let length = constrain(&mut s, "distance", json!(["e3"]), Some("4 mm"));
    let c = length["constraint"].as_str().unwrap().to_string();
    // An edit to a value it can never take is refused as well.
    let error = refused(
        &mut s,
        "sketch.edit",
        json!({"sketch": "S", "constraint": c, "value": "-4 mm"}),
    );
    assert!(error.contains("cannot be negative"), "{error}");
    // A value from variables is left to the regeneration: the variable
    // may still change.
    run(
        &mut s,
        "variable.set",
        json!({"name": "d", "expr": "-2 mm"}),
    );
    let later = constrain(&mut s, "radius", json!(["e5"]), Some("d"));
    assert_eq!(later["status"], "failed", "{later}");
}
