//! Features that make a body out of an earlier feature's body: transform,
//! mirror, pattern. Volumes and centroids come from the exact bodies and
//! are compared with values worked out by hand.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{feature_brief as feature, run};
use craft3d_engine::{Access, Session, Workspace};
use serde_json::{Value, json};
use std::f64::consts::PI;

fn error(session: &mut Session, id: &str, params: Value) -> String {
    session.execute(id, &params).unwrap_err().to_string()
}

/// A bar of 10 x 4 x 2 (volume 80) with its low corner at (x0, y0, 0):
/// sketch `Profile` (F1; points e1..e4, sides e5..e8) and body `Bar` (F2).
fn bar(x0: f64, y0: f64) -> Session {
    let mut s = Session::new(Workspace {
        read: Access::None,
        write: Access::None,
    });
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "sketch.create", json!({"name": "Profile"}));
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Profile", "closed": true,
               "points": [[x0, y0], [x0 + 10.0, y0], [x0 + 10.0, y0 + 4.0], [x0, y0 + 4.0]]}),
    );
    let made = run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Profile", "distance": 2, "name": "Bar"}),
    );
    assert_eq!((&made["id"], &made["status"]), (&json!("F2"), &json!("ok")));
    s
}

fn body(session: &mut Session, name: &str) -> Option<Value> {
    let document = run(session, "document.inspect", json!({}));
    document["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == name)
        .cloned()
}

fn body_names(session: &mut Session) -> Vec<String> {
    let document = run(session, "document.inspect", json!({}));
    document["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["name"].as_str().unwrap().to_string())
        .collect()
}

/// The body `name` has this volume and centroid (relative 1e-9), and the
/// document's bodies are all valid.
fn assert_body(session: &mut Session, name: &str, volume: f64, centroid: [f64; 3]) {
    let state = feature(session, name);
    assert_eq!(state["status"], "ok", "{state}");
    let found = body(session, name).unwrap_or_else(|| panic!("{name} has no body"));
    let actual = found["volume"].as_f64().unwrap();
    assert!(
        (actual - volume).abs() <= 1e-9 * volume.abs(),
        "{name}: volume {actual}, expected {volume}"
    );
    for axis in 0..3 {
        let c = found["centroid"][axis].as_f64().unwrap();
        assert!(
            (c - centroid[axis]).abs() <= 1e-9 * (1.0 + volume.abs().cbrt()),
            "{name}: centroid {}, expected {centroid:?}",
            found["centroid"]
        );
    }
    let check = run(session, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
}

fn assert_failed(session: &mut Session, name: &str, words: &[&str]) {
    let state = feature(session, name);
    assert_eq!(state["status"], "failed", "{state}");
    let reason = state["error"].as_str().unwrap();
    for word in words {
        assert!(reason.contains(word), "`{word}` is not in: {reason}");
    }
    assert!(body(session, name).is_none(), "a failed {name} has no body");
}

#[test]
fn a_moved_body_is_turned_about_the_pivot_and_then_shifted() {
    let mut s = bar(0.0, 0.0);
    assert_body(&mut s, "Bar", 80.0, [5.0, 2.0, 1.0]);
    let made = run(
        &mut s,
        "feature.transform",
        json!({"source": "Bar", "angle": "90 deg", "translate": [3, 0, 0], "name": "Moved"}),
    );
    assert_eq!(made["status"], "ok", "{made}");
    // A quarter turn about Z takes (5, 2, 1) to (-2, 5, 1); then 3 along X.
    assert_body(&mut s, "Moved", 80.0, [1.0, 5.0, 1.0]);
    // The bar's body went into it: one body, and the bar says where.
    assert_eq!(body_names(&mut s), ["Moved"]);
    assert_eq!(feature(&mut s, "Bar")["bodyUsedBy"], "F3");
    assert_eq!(feature(&mut s, "Bar")["status"], "ok");
    // Another pivot and axis: half a turn about the line through
    // (10, 0, 0) along Y takes (x, y, z) to (20 - x, y, -z).
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Moved", "angle": "180 deg", "axis": [0, 5, 0],
               "pivot": ["1 cm", 0, 0], "translate": [0, 0, 0]}),
    );
    assert_body(&mut s, "Moved", 80.0, [15.0, 2.0, -1.0]);
    // No turn at all: the axis is not looked at.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Moved", "angle": 0, "axis": [0, 0, 0], "translate": [0, "1 in", 0]}),
    );
    assert_body(&mut s, "Moved", 80.0, [5.0, 27.4, 1.0]);
    // Every face keeps its name, as a copy.
    let names = body(&mut s, "Moved").unwrap()["faceNames"].clone();
    let names: Vec<&str> = names
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap())
        .collect();
    assert_eq!(names.len(), 6);
    assert!(names.contains(&"F3:copy(0)/F2:side(e5)"), "{names:?}");
    assert!(
        names.contains(&"F3:copy(0)/F2:endCap(e5,e6,e7,e8)"),
        "{names:?}"
    );
    assert!(names.iter().all(|n| n.starts_with("F3:copy(0)/F2:")));
}

#[test]
fn keep_leaves_the_source_its_body() {
    let mut s = bar(0.0, 0.0);
    run(
        &mut s,
        "feature.transform",
        json!({"source": "Bar", "translate": [0, 0, 10], "keep": true, "name": "Copy"}),
    );
    assert_body(&mut s, "Copy", 80.0, [5.0, 2.0, 11.0]);
    assert_body(&mut s, "Bar", 80.0, [5.0, 2.0, 1.0]);
    assert_eq!(body_names(&mut s), ["Bar", "Copy"]);
    assert!(feature(&mut s, "Bar").get("bodyUsedBy").is_none());
    // A second feature can use a body that was kept, but not one that
    // went into another feature.
    run(
        &mut s,
        "feature.mirror",
        json!({"source": "Bar", "normal": [1, 0, 0], "name": "Left"}),
    );
    assert_body(&mut s, "Left", 80.0, [-5.0, 2.0, 1.0]);
    assert_eq!(body_names(&mut s), ["Copy", "Left"]);
    run(
        &mut s,
        "feature.transform",
        json!({"source": "Bar", "translate": [0, 9, 0], "name": "Late"}),
    );
    assert_failed(
        &mut s,
        "Late",
        &["source", "Bar", "already went into `Left`"],
    );
    // Without keep the first one takes it, and the mirror finds it gone.
    run(&mut s, "edit.undo", json!({}));
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Copy", "keep": false}),
    );
    assert_failed(&mut s, "Left", &["Bar", "already went into `Copy`"]);
    assert_eq!(body_names(&mut s), ["Copy"]);
}

#[test]
fn a_mirrored_body_is_a_valid_solid_on_the_other_side() {
    let mut s = bar(20.0, -2.0);
    run(
        &mut s,
        "feature.mirror",
        json!({"source": "Bar", "normal": [1, 0, 0], "name": "Left"}),
    );
    // Volume stays positive: a mirrored body is not inside out.
    assert_body(&mut s, "Left", 80.0, [-25.0, 0.0, 1.0]);
    // A plane that is not through the origin and not along an axis: the
    // plane through (0, 0, 5) with normal (0, 1, 1) swaps y and z about
    // it, (x, y, z) -> (x, 5 - z, 5 - y).
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Left", "normal": [0, 3, 3], "point": [0, 0, 5]}),
    );
    assert_body(&mut s, "Left", 80.0, [25.0, 4.0, 5.0]);
    // Mirrored twice is where it was.
    run(
        &mut s,
        "feature.mirror",
        json!({"source": "Left", "normal": [0, 1, 1], "point": [7, 0, 5], "name": "Back"}),
    );
    assert_body(&mut s, "Back", 80.0, [25.0, 0.0, 1.0]);
    assert_eq!(body_names(&mut s), ["Back"]);
    let names = body(&mut s, "Back").unwrap()["faceNames"].clone();
    assert!(
        names
            .as_array()
            .unwrap()
            .contains(&json!("F4:copy(0)/F2:side(e7)")),
        "{names}"
    );
}

#[test]
fn a_linear_pattern_repeats_the_body_along_the_step() {
    let mut s = bar(0.0, 0.0);
    run(&mut s, "variable.set", json!({"name": "n", "expr": "4"}));
    run(
        &mut s,
        "variable.set",
        json!({"name": "pitch", "expr": "20 mm"}),
    );
    let made = run(
        &mut s,
        "feature.pattern",
        json!({"source": "Bar", "count": "n", "step": ["pitch", 0, "pitch / 4"], "name": "Row"}),
    );
    assert_eq!(made["status"], "ok", "{made}");
    // Instances at 0, 20, 40, 60 along X and 0, 5, 10, 15 along Z.
    assert_body(&mut s, "Row", 320.0, [35.0, 2.0, 8.5]);
    assert_eq!(body_names(&mut s), ["Row"]);
    let row = body(&mut s, "Row").unwrap();
    assert_eq!(
        (&row["faces"], &row["edges"], &row["vertices"]),
        (&json!(24), &json!(48), &json!(32))
    );
    let names = row["faceNames"].as_array().unwrap();
    assert_eq!(names.len(), 24);
    for instance in 0..4 {
        let name = format!("F3:copy({instance})/F2:endCap(e5,e6,e7,e8)");
        assert!(names.contains(&json!(name)), "{name} is not in {names:?}");
    }
    // The count follows its variable; one instance is the source itself.
    run(&mut s, "variable.set", json!({"name": "n", "expr": "7"}));
    assert_body(&mut s, "Row", 560.0, [65.0, 2.0, 16.0]);
    run(&mut s, "variable.set", json!({"name": "n", "expr": "1"}));
    assert_body(&mut s, "Row", 80.0, [5.0, 2.0, 1.0]);
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_body(&mut s, "Row", 320.0, [35.0, 2.0, 8.5]);
}

/// Conditions in expressions (format 31): a pattern's count chosen by
/// `if` from a variable, the count following when the condition flips,
/// and a branch not taken that would divide by zero.
#[test]
fn a_pattern_count_chosen_by_a_condition_follows_its_variable() {
    let mut s = bar(0.0, 0.0);
    run(
        &mut s,
        "variable.set",
        json!({"name": "length", "expr": "150 mm"}),
    );
    run(&mut s, "variable.set", json!({"name": "gaps", "expr": "0"}));
    run(
        &mut s,
        "variable.set",
        json!({"name": "n", "expr": "if(length > 200 mm, 4, if(gaps > 0, 12 / gaps, 2))"}),
    );
    run(
        &mut s,
        "variable.set",
        json!({"name": "pitch", "expr": "clamp(length / 5, 12 mm, 20 mm)"}),
    );
    let made = run(
        &mut s,
        "feature.pattern",
        json!({"source": "Bar", "count": "n", "step": ["pitch", 0, 0], "name": "Row"}),
    );
    assert_eq!(made["status"], "ok", "{made}");
    // Two bars of 80 mm³, 20 mm apart (150 / 5 = 30, clamped to 20).
    assert_body(&mut s, "Row", 160.0, [15.0, 2.0, 1.0]);
    run(
        &mut s,
        "variable.set",
        json!({"name": "length", "expr": "250 mm"}),
    );
    assert_body(&mut s, "Row", 320.0, [35.0, 2.0, 1.0]);
    // The other branch: 12 / 4 = 3 bars, at the pitch clamped from below.
    run(
        &mut s,
        "variable.set",
        json!({"name": "length", "expr": "50 mm"}),
    );
    run(&mut s, "variable.set", json!({"name": "gaps", "expr": "4"}));
    assert_body(&mut s, "Row", 240.0, [17.0, 2.0, 1.0]);
    // The count is directly a condition as well.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Row", "count": "choose(gaps / 2, 5, 2)"}),
    );
    assert_body(&mut s, "Row", 160.0, [11.0, 2.0, 1.0]);
}

#[test]
fn a_circular_pattern_goes_round_the_axis() {
    // The bar from x = 20 to 30, 4 wide about the X axis: far enough from
    // the axis that eight of them do not touch.
    let mut s = bar(20.0, -2.0);
    run(&mut s, "variable.set", json!({"name": "n", "expr": "6"}));
    run(
        &mut s,
        "feature.pattern",
        json!({"source": "Bar", "count": "n", "angle": "360 deg / n", "name": "Ring"}),
    );
    assert_body(&mut s, "Ring", 480.0, [0.0, 0.0, 1.0]);
    assert_eq!(body(&mut s, "Ring").unwrap()["faces"], 36);
    run(&mut s, "variable.set", json!({"name": "n", "expr": "8"}));
    assert_body(&mut s, "Ring", 640.0, [0.0, 0.0, 1.0]);
    // Three of them a quarter turn apart: at 0, 90 and 180 degrees the
    // centroids are (25, 0), (0, 25) and (-25, 0); their mean is (0, 25/3).
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Ring", "count": 3, "angle": "90 deg"}),
    );
    assert_body(&mut s, "Ring", 240.0, [0.0, 25.0 / 3.0, 1.0]);
    // About another axis: the line through (25, 0, 10) along Y. A half
    // turn takes the bar's centroid (25, 0, 1) to (25, 0, 19).
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Ring", "count": 2, "angle": "180 deg",
               "axis": [0, 1, 0], "point": [25, 0, 10]}),
    );
    assert_body(&mut s, "Ring", 160.0, [25.0, 0.0, 10.0]);
}

#[test]
fn a_sketch_lies_on_a_face_of_a_copy() {
    let mut s = bar(20.0, -2.0);
    run(&mut s, "variable.set", json!({"name": "n", "expr": "6"}));
    run(
        &mut s,
        "feature.pattern",
        json!({"source": "Bar", "count": "n", "angle": "360 deg / n", "name": "Ring"}),
    );
    // The top of instance 2, which is turned by 120 degrees: the sketch's
    // axes are the bar's sketch axes turned with it.
    let seat = run(
        &mut s,
        "sketch.create",
        json!({"name": "Seat", "face": "F3:copy(2)/F2:endCap(e5,e6,e7,e8)"}),
    );
    assert_eq!(seat["status"], "ok", "{seat}");
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Seat", "center": [25, 0], "radius": 1}),
    );
    run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Seat", "distance": 4, "name": "Boss"}),
    );
    let third = 2.0 * PI / 3.0;
    assert_body(
        &mut s,
        "Boss",
        4.0 * PI,
        [25.0 * third.cos(), 25.0 * third.sin(), 4.0],
    );
    // More instances: instance 2 is now at 90 degrees, and the boss with it.
    run(&mut s, "variable.set", json!({"name": "n", "expr": "8"}));
    assert_body(&mut s, "Boss", 4.0 * PI, [0.0, 25.0, 4.0]);
    // Fewer than three: instance 2 is gone, and the seat says so.
    run(&mut s, "variable.set", json!({"name": "n", "expr": "2"}));
    assert_failed(&mut s, "Seat", &["Ring", "2 instance(s)", "F3:copy(2)"]);
    assert_failed(&mut s, "Boss", &["Seat"]);
    run(&mut s, "edit.undo", json!({}));
    // The outline of the bar changes: a cap keeps its plane, through the
    // copy as well (`endCap()` is the plane where F2 ends).
    run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "Seat", "face": "F3:copy(2)/F2:endCap()"}),
    );
    assert_body(&mut s, "Boss", 4.0 * PI, [0.0, 25.0, 4.0]);
    // Names that are not faces of the ring.
    for (face, words) in [
        (
            "F3:copy(2)/F2:side(e99)",
            vec!["Ring", "no longer has a face"],
        ),
        (
            "F3:copy(2)/F1:side(e5)",
            vec!["Ring", "no longer has a face"],
        ),
        ("F3:copy(9)/F2:endCap()", vec!["8 instance(s)"]),
        (
            "F2:copy(0)/F1:side(e5)",
            vec!["Bar", "no longer has a face"],
        ),
        ("F3:copy(0)/F2:lateralEdge(e5,e6)", vec!["is not a face"]),
    ] {
        run(
            &mut s,
            "sketch.edit",
            json!({"sketch": "Seat", "face": face}),
        );
        assert_failed(&mut s, "Seat", &words);
        run(&mut s, "edit.undo", json!({}));
    }
    // The face of the bar itself still resolves, though its body went into
    // the ring: the bar's top, where instance 0 is.
    run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "Seat", "face": "F2:endCap()"}),
    );
    assert_body(&mut s, "Boss", 4.0 * PI, [25.0, 0.0, 4.0]);
}

#[test]
fn a_sketch_on_a_mirrored_face_is_seen_from_outside() {
    let mut s = bar(20.0, -2.0);
    run(
        &mut s,
        "feature.mirror",
        json!({"source": "Bar", "normal": [1, 0, 0], "name": "Left"}),
    );
    run(
        &mut s,
        "sketch.create",
        json!({"name": "Seat", "face": "F3:copy(0)/F2:endCap()"}),
    );
    // The top still faces +Z. Its x axis is the mirror image of the bar's
    // (world -X); a right-handed frame then has y along world -Y, so what
    // is drawn at y = 1 lands at world y = -1.
    let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "Seat"}));
    assert_eq!(
        inspected["frame"],
        json!({"origin": [0.0, 0.0, 2.0], "x": [-1.0, 0.0, 0.0], "y": [0.0, -1.0, 0.0]})
    );
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Seat", "center": [25, 1], "radius": 1}),
    );
    run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Seat", "distance": 4, "name": "Boss"}),
    );
    assert_body(&mut s, "Boss", 4.0 * PI, [-25.0, -1.0, 4.0]);
}

#[test]
fn what_happens_to_the_source_happens_to_what_is_made_from_it() {
    let mut s = bar(0.0, 0.0);
    run(
        &mut s,
        "feature.pattern",
        json!({"source": "Bar", "count": 3, "step": [0, 10, 0], "name": "Row"}),
    );
    assert_body(&mut s, "Row", 240.0, [5.0, 12.0, 1.0]);
    // The bar gets longer: so does every instance.
    run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "Profile", "entity": "e2", "at": [30, 0]}),
    );
    run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "Profile", "entity": "e3", "at": [30, 4]}),
    );
    assert_body(&mut s, "Row", 720.0, [15.0, 12.0, 1.0]);
    for (command, params, words) in [
        (
            "feature.suppress",
            json!({"feature": "Bar"}),
            vec!["Bar", "suppressed"],
        ),
        (
            "feature.delete",
            json!({"feature": "Bar"}),
            vec!["no feature F2", "deleted"],
        ),
        (
            "feature.move",
            json!({"feature": "Bar", "position": 2}),
            vec!["Bar", "comes after"],
        ),
        (
            "feature.edit",
            json!({"feature": "Bar", "distance": "nope"}),
            vec!["Bar", "failed"],
        ),
    ] {
        run(&mut s, command, params);
        assert_failed(&mut s, "Row", &words);
        run(&mut s, "edit.undo", json!({}));
        assert_body(&mut s, "Row", 720.0, [15.0, 12.0, 1.0]);
    }
    // A suppressed pattern gives the bar its body back.
    run(&mut s, "feature.suppress", json!({"feature": "Row"}));
    assert_eq!(body_names(&mut s), ["Bar"]);
    assert!(feature(&mut s, "Bar").get("bodyUsedBy").is_none());
    run(&mut s, "edit.undo", json!({}));
    // So does a rolled-back one, and a failed one.
    run(&mut s, "history.rollback", json!({"feature": "Bar"}));
    assert_eq!(body_names(&mut s), ["Bar"]);
    run(&mut s, "edit.undo", json!({}));
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Row", "count": 0}),
    );
    assert_eq!(body_names(&mut s), ["Bar"]);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(body_names(&mut s), ["Row"]);
    // Something that is not a body cannot be a source.
    run(
        &mut s,
        "feature.mirror",
        json!({"source": "Profile", "normal": [0, 0, 1], "name": "Flat"}),
    );
    assert_failed(&mut s, "Flat", &["source", "Profile", "sketch", "no body"]);
}

#[test]
fn parameters_that_make_no_body_are_failures_that_name_the_parameter() {
    let mut s = bar(20.0, -2.0);
    run(
        &mut s,
        "feature.pattern",
        json!({"source": "Bar", "count": 2, "step": [0, 10, 0], "name": "Row"}),
    );
    for (params, words) in [
        (
            json!({"count": 0}),
            vec!["count", "whole number", "at least 1"],
        ),
        (json!({"count": -3}), vec!["count", "at least 1"]),
        (json!({"count": 2.5}), vec!["count", "whole number"]),
        (json!({"count": "1 mm"}), vec!["count", "plain number"]),
        (json!({"count": "nope"}), vec!["count", "nope"]),
        (json!({"count": 1e9}), vec!["count", "at most 10000"]),
        (
            json!({"step": [0, 0, 0]}),
            vec!["step", "on top of each other"],
        ),
        (json!({"step": [0, "30 deg", 0]}), vec!["step.y", "length"]),
        (json!({"step": [0, "1e308 * 1e308", 0]}), vec!["step"]),
    ] {
        run(&mut s, "feature.edit", {
            let mut p = params.clone();
            p["feature"] = "Row".into();
            p
        });
        assert_failed(&mut s, "Row", &words);
        // The bar is still there, whole, and has its body back.
        assert_eq!(body_names(&mut s), ["Bar"]);
        let volume = body(&mut s, "Bar").unwrap()["volume"].as_f64().unwrap();
        assert!((volume - 80.0).abs() < 1e-9, "{volume}");
        run(&mut s, "edit.undo", json!({}));
    }
    run(&mut s, "feature.delete", json!({"feature": "Row"}));
    run(
        &mut s,
        "feature.pattern",
        json!({"source": "Bar", "count": 4, "angle": "90 deg", "name": "Ring"}),
    );
    assert_body(&mut s, "Ring", 320.0, [0.0, 0.0, 1.0]);
    for (params, words) in [
        (json!({"count": 5}), vec!["more than a full turn"]),
        (json!({"angle": 0}), vec!["angle is zero"]),
        (json!({"angle": "3 mm"}), vec!["angle", "angle is needed"]),
        (json!({"axis": [0, 0, 0]}), vec!["axis", "direction"]),
        (
            json!({"axis": [0, "1 mm", 0]}),
            vec!["axis.y", "plain number"],
        ),
        (json!({"point": [0, "1 deg", 0]}), vec!["point.y", "length"]),
    ] {
        run(&mut s, "feature.edit", {
            let mut p = params.clone();
            p["feature"] = "Ring".into();
            p
        });
        assert_failed(&mut s, "Ring", &words);
        run(&mut s, "edit.undo", json!({}));
    }
    run(&mut s, "feature.delete", json!({"feature": "Ring"}));
    run(
        &mut s,
        "feature.mirror",
        json!({"source": "Bar", "normal": [0, 0, 0], "name": "Left"}),
    );
    assert_failed(&mut s, "Left", &["normal", "direction"]);
    run(&mut s, "feature.delete", json!({"feature": "Left"}));
    run(
        &mut s,
        "feature.transform",
        json!({"source": "Bar", "angle": "10 deg", "axis": [0, 0, 0], "name": "Moved"}),
    );
    assert_failed(&mut s, "Moved", &["axis", "direction"]);
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Moved", "axis": [0, 0, 1], "translate": ["1 deg", 0, 0]}),
    );
    assert_failed(&mut s, "Moved", &["translate.x", "length"]);
}

#[test]
fn malformed_requests_are_refused_and_change_nothing() {
    let mut s = bar(0.0, 0.0);
    let before = run(&mut s, "document.inspect", json!({}));
    for (id, params, words) in [
        (
            "feature.transform",
            json!({}),
            vec!["missing parameter `source`"],
        ),
        (
            "feature.transform",
            json!({"source": "Nope"}),
            vec!["no feature named `Nope`", "Bar"],
        ),
        (
            "feature.transform",
            json!({"source": "F9"}),
            vec!["no feature"],
        ),
        ("feature.transform", json!({"source": 2.5}), vec!["source"]),
        (
            "feature.transform",
            json!({"source": "Bar", "translate": [1, 2]}),
            vec!["translate", "three"],
        ),
        (
            "feature.transform",
            json!({"source": "Bar", "keep": "yes"}),
            vec!["keep"],
        ),
        (
            "feature.transform",
            json!({"source": "Bar", "scale": 2}),
            vec!["unknown parameter `scale`"],
        ),
        (
            "feature.mirror",
            json!({"source": "Bar"}),
            vec!["missing parameter `normal`"],
        ),
        (
            "feature.mirror",
            json!({"source": "Bar", "normal": "x"}),
            vec!["normal", "three"],
        ),
        (
            "feature.pattern",
            json!({"source": "Bar", "step": [1, 0, 0]}),
            vec!["missing parameter `count`"],
        ),
        (
            "feature.pattern",
            json!({"source": "Bar", "count": 2}),
            vec!["give `step`", "or `angle`"],
        ),
        (
            "feature.pattern",
            json!({"source": "Bar", "count": 2, "step": [1, 0, 0], "angle": "10 deg"}),
            vec!["one length along the axis"],
        ),
        (
            "feature.pattern",
            json!({"source": "Bar", "count": 2, "step": [1, 0, 0], "axis": [0, 0, 1]}),
            vec!["`axis` belongs to a circular pattern"],
        ),
        (
            "feature.pattern",
            json!({"source": "Bar", "count": [2], "step": [1, 0, 0]}),
            vec!["count"],
        ),
        ("feature.pattern", json!([1]), vec!["JSON object"]),
    ] {
        let e = error(&mut s, id, params.clone());
        for word in &words {
            assert!(e.contains(word), "{id} {params}: `{word}` is not in: {e}");
        }
    }
    assert_eq!(run(&mut s, "document.inspect", json!({})), before);
    // feature.edit: a parameter of another kind is an error that lists
    // the feature's own.
    run(
        &mut s,
        "feature.pattern",
        json!({"source": "Bar", "count": 2, "step": [0, 10, 0], "name": "Row"}),
    );
    let e = error(
        &mut s,
        "feature.edit",
        json!({"feature": "Row", "angle": "10 deg"}),
    );
    assert!(
        e.contains("`angle` is not a parameter of a linear pattern") && e.contains("count, step"),
        "{e}"
    );
    let e = error(
        &mut s,
        "feature.edit",
        json!({"feature": "Row", "keep": true}),
    );
    assert!(e.contains("`keep` is not a parameter"), "{e}");
    let e = error(
        &mut s,
        "feature.edit",
        json!({"feature": "Bar", "translate": [1, 0, 0]}),
    );
    assert!(
        e.contains("`translate` is not a parameter of a extrude"),
        "{e}"
    );
}

#[test]
fn instances_that_touch_or_overlap_are_joined() {
    // Side by side in one body, instances that touch would leave their
    // touching faces inside the material, and instances that overlap would
    // pass through each other: the plain check sees neither. So a pattern
    // joins them, as a union would.
    let mut s = bar(0.0, 0.0);
    run(
        &mut s,
        "feature.pattern",
        json!({"source": "Bar", "count": 3, "step": [4, 1, 0], "name": "Row"}),
    );
    // Three bars of 80, each shifted by (4, 1): neighbours share
    // 6 x 3 x 2 = 36, the first and third 2 x 2 x 2 = 8, and that 8 lies
    // in the second as well. 240 - (36 + 36 + 8) + 8 = 168.
    let found = body(&mut s, "Row").unwrap();
    assert!(
        (found["volume"].as_f64().unwrap() - 168.0).abs() < 1e-9,
        "{found}"
    );
    let deep = run(&mut s, "document.check", json!({"deep": true}));
    assert_eq!(
        (&deep["ok"], &deep["deep"]),
        (&json!(true), &json!(true)),
        "{deep}"
    );
    // Every face is named after an instance and the face it came from.
    // The three tops lie in one plane and have become one face, named
    // after the first instance that has a part in it; the sides stay each
    // instance's own.
    let names = found["faceNames"].as_array().unwrap();
    let top = json!("F3:copy(0)/F2:endCap(e5,e6,e7,e8)");
    assert_eq!(names.iter().filter(|n| **n == top).count(), 1, "{names:?}");
    assert!(
        names.contains(&json!("F3:copy(2)/F2:side(e6)")),
        "{names:?}"
    );
    assert!(
        names
            .iter()
            .all(|n| n.as_str().unwrap().starts_with("F3:copy("))
    );
    // Touching: bars end to end, side by side, stacked. One piece each.
    for (step, centroid) in [
        ([10, 0, 0], [15.0, 2.0, 1.0]),
        ([0, 4, 0], [5.0, 6.0, 1.0]),
        ([0, 0, 2], [5.0, 2.0, 3.0]),
    ] {
        run(
            &mut s,
            "feature.edit",
            json!({"feature": "Row", "step": step}),
        );
        assert_body(&mut s, "Row", 240.0, centroid);
    }
    // Apart: nothing to join, and every edge keeps its name.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Row", "step": [14, 0, 0]}),
    );
    assert_body(&mut s, "Row", 240.0, [19.0, 2.0, 1.0]);
    let edges = run(&mut s, "document.edges", json!({"feature": "Row"}));
    assert!(edges["edges"][0].get("name").is_some(), "{edges}");
    // Boxes that overlap although the bodies do not: bars turned about a
    // far axis, close together.
    run(&mut s, "feature.delete", json!({"feature": "Row"}));
    run(
        &mut s,
        "feature.pattern",
        json!({"source": "Bar", "count": 12, "angle": "30 deg", "point": [-12, 2, 0], "name": "Ring"}),
    );
    assert_body(&mut s, "Ring", 960.0, [-12.0, 2.0, 1.0]);
    let e = s
        .execute("document.check", &json!({"deep": "yes"}))
        .unwrap_err()
        .to_string();
    assert!(e.contains("`deep` must be"), "{e}");
}

#[test]
fn a_chain_of_copies_saves_reopens_and_regenerates_the_same() {
    let dir = std::env::temp_dir().join(format!("craft3d-bodies-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = bar(20.0, -2.0);
    s.set_workspace(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    run(
        &mut s,
        "feature.transform",
        json!({"source": "Bar", "translate": [0, 0, 5], "angle": "30 deg", "name": "Moved"}),
    );
    run(
        &mut s,
        "feature.mirror",
        json!({"source": "Moved", "normal": [0, 0, 1], "keep": true, "name": "Under"}),
    );
    run(
        &mut s,
        "feature.pattern",
        json!({"source": "Under", "count": 5, "angle": "72 deg", "name": "Ring"}),
    );
    // Five bars under the plane z = 0, evenly round the axis.
    assert_body(&mut s, "Ring", 400.0, [0.0, 0.0, -6.0]);
    let half = PI / 6.0;
    assert_body(
        &mut s,
        "Moved",
        80.0,
        [25.0 * half.cos(), 25.0 * half.sin(), 6.0],
    );
    assert_eq!(body_names(&mut s), ["Moved", "Ring"]);
    let names = body(&mut s, "Ring").unwrap()["faceNames"].clone();
    assert!(
        names
            .as_array()
            .unwrap()
            .contains(&json!("F5:copy(4)/F2:side(e6)")),
        "{names}"
    );
    let before = run(&mut s, "document.inspect", json!({}));
    run(&mut s, "file.save", json!({"path": "ring.3dcraft"}));
    run(&mut s, "file.open", json!({"path": "ring.3dcraft"}));
    let after = run(&mut s, "document.inspect", json!({}));
    // Bit for bit: the same bodies from the same recipe.
    assert_eq!(after["bodies"], before["bodies"]);
    assert_eq!(after["features"], before["features"]);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A tread of a spiral stair: a block 800 x 250 x 30 mm (volume 6e6)
/// from radius 100 to 900 mm about the Z axis, its centroid at
/// (500, 0, 15); with `storey` and `n` as variables.
fn tread() -> Session {
    let mut s = Session::new(Workspace {
        read: Access::None,
        write: Access::None,
    });
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "variable.set",
        json!({"name": "storey", "expr": "2.8 m"}),
    );
    run(&mut s, "variable.set", json!({"name": "n", "expr": "28"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": ["0.8 m", "250 mm", "30 mm"], "position": ["100 mm", "-125 mm", 0],
               "name": "Tread"}),
    );
    s
}

/// The mean of the tread centroids of instances 0..count of a screw of
/// `angle` degrees and `rise` mm per instance about Z.
fn stair_centroid(count: u32, angle: f64, rise: f64) -> [f64; 3] {
    let mut sum = [0.0; 3];
    for k in 0..count {
        let (s, c) = (angle * k as f64).to_radians().sin_cos();
        sum[0] += 500.0 * c;
        sum[1] += 500.0 * s;
        sum[2] += 15.0 + rise * k as f64;
    }
    sum.map(|v| v / count as f64)
}

#[test]
fn a_screw_pattern_turns_and_climbs() {
    let mut s = tread();
    let made = run(
        &mut s,
        "feature.pattern",
        json!({"source": "Tread", "count": "n", "angle": "360 deg / n", "step": "storey / n",
               "name": "Stair"}),
    );
    assert_eq!(made["status"], "ok", "{made}");
    // A full turn of 28 treads, 100 mm up each: the centroid is on the
    // axis, halfway up.
    assert_body(&mut s, "Stair", 28.0 * 6e6, [0.0, 0.0, 15.0 + 100.0 * 13.5]);
    let found = body(&mut s, "Stair").unwrap();
    assert_eq!(found["faces"], 28 * 6);
    let names = found["faceNames"].as_array().unwrap();
    for k in 0..28 {
        for face in 0..6 {
            let name = format!("F2:copy({k})/F1:face({face})");
            assert!(names.contains(&json!(name)), "{name} is not in {names:?}");
        }
    }
    // Seven treads of the same stair: the centroid pins the angle, the
    // radius and the rise.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Stair", "count": 7}),
    );
    assert_body(
        &mut s,
        "Stair",
        7.0 * 6e6,
        stair_centroid(7, 360.0 / 28.0, 100.0),
    );
    // The step follows its variables, with units.
    run(
        &mut s,
        "variable.set",
        json!({"name": "storey", "expr": "5.6 m"}),
    );
    assert_body(
        &mut s,
        "Stair",
        7.0 * 6e6,
        stair_centroid(7, 360.0 / 28.0, 200.0),
    );
    // A negative step climbs the other way; a step is edited like the rest.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Stair", "step": "-0.1 m"}),
    );
    assert_body(
        &mut s,
        "Stair",
        7.0 * 6e6,
        stair_centroid(7, 360.0 / 28.0, -100.0),
    );
    // With a step the treads may go round more than once: a quarter turn
    // each, three turns.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Stair", "count": 12, "angle": "90 deg", "step": "200 mm"}),
    );
    assert_body(&mut s, "Stair", 12.0 * 6e6, stair_centroid(12, 90.0, 200.0));
    let params = feature(&mut s, "Stair")["params"].clone();
    assert_eq!(params["layout"]["step"], "200 mm", "{params}");
    // Without one they may not.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Stair", "step": null}),
    );
    assert_failed(&mut s, "Stair", &["more than a full turn"]);
    let params = feature(&mut s, "Stair")["params"].clone();
    assert!(params["layout"]["step"].is_null(), "{params}");
}

#[test]
fn a_screw_with_a_zero_step_is_the_plain_turn_and_comes_back_to_it() {
    let mut s = bar(20.0, -2.0);
    run(
        &mut s,
        "feature.pattern",
        json!({"source": "Bar", "count": 6, "angle": "60 deg", "name": "Ring"}),
    );
    let plain = run(&mut s, "document.inspect", json!({}))["bodies"].clone();
    assert_body(&mut s, "Ring", 480.0, [0.0, 0.0, 1.0]);
    for step in [json!("0 mm"), json!(0), json!("0 * 3 mm")] {
        run(
            &mut s,
            "feature.edit",
            json!({"feature": "Ring", "step": step}),
        );
        let now = run(&mut s, "document.inspect", json!({}))["bodies"].clone();
        assert_eq!(now, plain, "step {step}");
    }
    // A screw, then a plain turn again.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Ring", "step": "4 mm"}),
    );
    assert_body(&mut s, "Ring", 480.0, [0.0, 0.0, 1.0 + 4.0 * 2.5]);
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Ring", "step": null}),
    );
    assert_eq!(run(&mut s, "document.inspect", json!({}))["bodies"], plain);
    // Downstream references follow the instances through every edit: the
    // top of instance 4 keeps its name.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Ring", "step": "4 mm"}),
    );
    let seat = run(
        &mut s,
        "sketch.create",
        json!({"name": "Seat", "face": "F3:copy(4)/F2:endCap(e5,e6,e7,e8)"}),
    );
    assert_eq!(seat["status"], "ok", "{seat}");
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Seat", "center": [25, 0], "radius": 1}),
    );
    run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Seat", "distance": 4, "name": "Boss"}),
    );
    // Instance 4 is turned by 240 degrees and 16 up: the boss stands on
    // its top (z = 18) and its centroid is 2 above.
    let turned = 240f64.to_radians();
    assert_body(
        &mut s,
        "Boss",
        4.0 * PI,
        [25.0 * turned.cos(), 25.0 * turned.sin(), 20.0],
    );
    // A different step moves the boss along: the reference holds.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Ring", "step": "-2 mm"}),
    );
    assert_body(
        &mut s,
        "Boss",
        4.0 * PI,
        [25.0 * turned.cos(), 25.0 * turned.sin(), 2.0 - 8.0 + 2.0],
    );
}

#[test]
fn a_screw_pattern_saves_reopens_and_regenerates_the_same() {
    let dir = std::env::temp_dir().join(format!("craft3d-screw-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = tread();
    s.set_workspace(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    run(
        &mut s,
        "feature.pattern",
        json!({"source": "Tread", "count": "n", "angle": "360 deg / n", "step": "storey / n",
               "name": "Stair"}),
    );
    assert_body(&mut s, "Stair", 28.0 * 6e6, [0.0, 0.0, 15.0 + 100.0 * 13.5]);
    let before = run(&mut s, "document.inspect", json!({}));
    run(&mut s, "file.save", json!({"path": "stair.3dcraft"}));
    run(&mut s, "file.open", json!({"path": "stair.3dcraft"}));
    let after = run(&mut s, "document.inspect", json!({}));
    assert_eq!(after["bodies"], before["bodies"]);
    assert_eq!(after["features"], before["features"]);
    // The step is still an expression of the variables.
    run(&mut s, "variable.set", json!({"name": "n", "expr": "7"}));
    assert_body(
        &mut s,
        "Stair",
        7.0 * 6e6,
        stair_centroid(7, 360.0 / 7.0, 400.0),
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_screw_pattern_refuses_what_it_cannot_make() {
    let mut s = tread();
    // A shift [x, y, z] goes with a linear pattern; a screw's step is a
    // length along the axis.
    let e = error(
        &mut s,
        "feature.pattern",
        json!({"source": "Tread", "count": 4, "angle": "10 deg", "step": [0, 0, "100 mm"]}),
    );
    assert!(e.contains("one length along the axis"), "{e}");
    // One length without an angle: a screw's step, told to give the angle.
    let e = error(
        &mut s,
        "feature.pattern",
        json!({"source": "Tread", "count": 4, "step": "100 mm"}),
    );
    assert!(e.contains("needs `angle`"), "{e}");
    // Not a length: the feature fails and says so.
    run(
        &mut s,
        "feature.pattern",
        json!({"source": "Tread", "count": 4, "angle": "10 deg", "step": "5 deg", "name": "Stair"}),
    );
    assert_failed(&mut s, "Stair", &["step", "length is needed"]);
    // Within the tolerance: neither a climb nor nothing, so refused, not
    // dropped (an exact zero is a plain turn).
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Stair", "step": "1e-7 mm"}),
    );
    assert_failed(
        &mut s,
        "Stair",
        &["step", "within the tolerance", "step: null"],
    );
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Stair", "step": "nope"}),
    );
    assert_failed(&mut s, "Stair", &["step", "nope"]);
    // Wrong types are refused before anything changes.
    for step in [json!([0, 0, 1]), json!(true), json!({"z": 1})] {
        let e = error(
            &mut s,
            "feature.edit",
            json!({"feature": "Stair", "step": step}),
        );
        assert!(e.contains("step"), "{e}");
    }
    // A linear pattern has no step along an axis to edit into a screw.
    run(
        &mut s,
        "feature.pattern",
        json!({"source": "Tread", "count": 3, "step": [0, 0, "100 mm"], "name": "Row"}),
    );
    let e = error(
        &mut s,
        "feature.edit",
        json!({"feature": "Row", "angle": "10 deg"}),
    );
    assert!(e.contains("not a parameter of a linear pattern"), "{e}");
}
