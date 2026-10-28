//! A feature inserted at the rollback marker that takes a body: the
//! features after it that named that body (a boolean's target or tool,
//! the source of a transform, mirror, pattern, shell or split) are pointed
//! at the new feature, so the history goes on with the body it changed
//! (`Document::repoint_after`). Each case is checked against the same
//! features built in that order from the start, at three scales.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::run;
use craft3d_engine::Session;
use serde_json::{Value, json};

type Step = (&'static str, Value);

fn new(k: f64) -> Session {
    let mut s = common::session();
    run(
        &mut s,
        "file.new",
        json!({"units": "mm", "tolerance": 1e-6 * k}),
    );
    s
}

/// A block with a hole, a fillet on it and a pattern of the result.
fn part(k: f64) -> Vec<Step> {
    vec![
        (
            "feature.block",
            json!({"size": [20.0 * k, 10.0 * k, 6.0 * k], "name": "Base"}),
        ),
        (
            "feature.cylinder",
            json!({"radius": 2.0 * k, "height": 10.0 * k, "position": [10.0 * k, 5.0 * k, -2.0 * k], "name": "Tool"}),
        ),
        (
            "feature.boolean",
            json!({"op": "subtract", "target": "Base", "tool": "Tool", "name": "Cut"}),
        ),
        (
            "feature.fillet",
            json!({"edges": [["F1:face(2)", "F1:face(4)"]], "radius": 1.0 * k, "name": "Round"}),
        ),
        (
            "feature.pattern",
            json!({"source": "Round", "count": 2, "step": [30.0 * k, 0, 0], "name": "Row"}),
        ),
    ]
}

fn build(s: &mut Session, steps: &[Step]) {
    for (id, params) in steps {
        let r = run(s, id, params.clone());
        assert_ne!(r["status"], "failed", "{id} {params}: {r}");
    }
}

fn volume(s: &mut Session) -> f64 {
    run(s, "document.measure", json!({}))["volume"]
        .as_f64()
        .unwrap()
}

/// Inserts `inserted` after the feature `at` and rolls forward; checks
/// the parameters re-pointed, that nothing fails, and that the volume is
/// that of the part built in that order.
fn insert_and_compare(k: f64, at: &str, inserted: Step, repointed: Value) {
    let mut s = new(k);
    build(&mut s, &part(k));
    run(&mut s, "history.rollback", json!({"feature": at}));
    let added = run(&mut s, inserted.0, inserted.1.clone());
    assert_eq!(added["status"], "ok", "{added}");
    assert_eq!(added["repointed"], repointed, "{added}");
    let forward = run(&mut s, "history.rollback", json!({"feature": null}));
    assert!(
        forward["failed"].as_array().is_none_or(Vec::is_empty),
        "{forward}"
    );
    assert_eq!(run(&mut s, "document.check", json!({}))["ok"], true);

    // The same features, in that order, from the start: the later ones
    // written to name the inserted feature.
    let name = inserted.1["name"].as_str().unwrap().to_string();
    let mut steps = Vec::new();
    for (id, params) in part(k) {
        let mut params = params;
        for item in repointed.as_array().unwrap() {
            if params["name"] == item["feature"] {
                params[item["param"].as_str().unwrap()] = json!(name);
            }
        }
        let here = params["name"] == at;
        steps.push((id, params));
        if here {
            steps.push(inserted.clone());
        }
    }
    let mut straight = new(k);
    build(&mut straight, &steps);
    let (ours, theirs) = (volume(&mut s), volume(&mut straight));
    assert!(
        (ours - theirs).abs() <= 1e-9 * theirs.abs(),
        "{k}: {ours} vs {theirs}"
    );
    // Undone, the later features name the old body again.
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "history.rollback", json!({"feature": null}));
    let mut original = new(k);
    build(&mut original, &part(k));
    let (back, before) = (volume(&mut s), volume(&mut original));
    assert!(
        (back - before).abs() <= 1e-9 * before.abs(),
        "{back} vs {before}"
    );
}

/// A chamfer inserted before the subtraction: the subtraction's target is
/// the chamfer; the fillet after it follows its faces by itself.
#[test]
fn a_blend_inserted_before_a_boolean_and_a_blend() {
    for k in [1e-3, 1.0, 1e3] {
        insert_and_compare(
            k,
            "Tool",
            (
                "feature.chamfer",
                json!({"edges": [["F1:face(1)", "F1:face(3)"]], "distance": 0.5 * k, "name": "Bevel"}),
            ),
            json!([{"feature": "Cut", "param": "target"}]),
        );
    }
}

/// A move inserted before the pattern: the pattern repeats the moved body.
#[test]
fn a_transform_inserted_before_a_pattern() {
    for k in [1e-3, 1.0, 1e3] {
        insert_and_compare(
            k,
            "Round",
            (
                "feature.transform",
                json!({"source": "Round", "translate": [0, 0, 5.0 * k], "name": "Lift"}),
            ),
            json!([{"feature": "Row", "param": "source"}]),
        );
    }
}

/// A union inserted before the subtraction, of the block and a boss: the
/// subtraction cuts the union.
#[test]
fn a_union_inserted_before_a_subtraction() {
    let k = 1.0;
    let mut s = new(k);
    build(&mut s, &part(k));
    run(&mut s, "history.rollback", json!({"feature": "Tool"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [4, 4, 4], "position": [1, 1, 5], "name": "Boss"}),
    );
    let joined = run(
        &mut s,
        "feature.boolean",
        json!({"op": "union", "target": "Base", "tool": "Boss", "name": "Joined"}),
    );
    assert_eq!(
        joined["repointed"],
        json!([{"feature": "Cut", "param": "target"}]),
        "{joined}"
    );
    run(&mut s, "history.rollback", json!({"feature": null}));
    assert_eq!(run(&mut s, "document.check", json!({}))["ok"], true);
    // Base 1200 - the hole (pi 2^2 6) + the boss above the block (4 4 3),
    // less what the fillet takes, twice: compared with the same features
    // built in that order.
    let mut straight = new(k);
    let mut steps = part(k);
    steps[2].1["target"] = json!("Joined");
    steps.insert(
        2,
        (
            "feature.boolean",
            json!({"op": "union", "target": "Base", "tool": "Boss", "name": "Joined"}),
        ),
    );
    steps.insert(
        2,
        (
            "feature.block",
            json!({"size": [4, 4, 4], "position": [1, 1, 5], "name": "Boss"}),
        ),
    );
    build(&mut straight, &steps);
    let (ours, theirs) = (volume(&mut s), volume(&mut straight));
    assert!((ours - theirs).abs() <= 1e-9 * theirs, "{ours} vs {theirs}");
}

/// The tool of a subtraction inserted at the marker does not go on in its
/// result: a later feature that used that tool is not pointed at the
/// subtraction, and fails with the advice.
#[test]
fn a_subtraction_inserted_does_not_take_over_its_tool() {
    let mut s = new(1.0);
    build(&mut s, &part(1.0));
    run(&mut s, "history.rollback", json!({"feature": "Tool"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [5, 5, 4], "position": [9, 4, 3], "name": "Spare"}),
    );
    let early = run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Spare", "tool": "Tool", "name": "Early"}),
    );
    assert!(early.get("repointed").is_none(), "{early}");
    run(&mut s, "history.rollback", json!({"feature": null}));
    let inspect = run(&mut s, "document.inspect", json!({"measures": false}));
    let features = inspect["features"].as_array().unwrap();
    let of = |name: &str| features.iter().find(|f| f["name"] == name).unwrap().clone();
    assert_eq!(of("Early")["status"], "ok", "{inspect}");
    let cut = of("Cut");
    assert_eq!(cut["status"], "failed", "{cut}");
    assert!(
        cut.to_string().contains("already went into `Early`"),
        "{cut}"
    );
}

/// A re-pointed document is saved and read back as it is: the later
/// features name the inserted one, and it builds the same volume.
#[test]
fn a_re_pointed_document_survives_a_save() {
    let dir = common::workdir("repoint-at-marker", "save");
    let mut s = common::session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    build(&mut s, &part(1.0));
    run(&mut s, "history.rollback", json!({"feature": "Tool"}));
    let bevel = run(
        &mut s,
        "feature.chamfer",
        json!({"edges": [["F1:face(1)", "F1:face(3)"]], "distance": 0.5, "name": "Bevel"}),
    );
    assert_eq!(
        bevel["repointed"],
        json!([{"feature": "Cut", "param": "target"}])
    );
    run(&mut s, "history.rollback", json!({"feature": null}));
    let want = volume(&mut s);
    run(&mut s, "file.save", json!({"path": "r.3dcraft"}));
    let mut t = common::session_in(&dir);
    run(&mut t, "file.open", json!({"path": "r.3dcraft"}));
    assert_eq!(run(&mut t, "document.check", json!({}))["ok"], true);
    let got = volume(&mut t);
    assert!((got - want).abs() <= 1e-9 * want, "{got} vs {want}");
    let inspect = run(&mut t, "document.inspect", json!({"measures": false}));
    let cut = inspect["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == "Cut")
        .unwrap()
        .clone();
    let bevel_id = inspect["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == "Bevel")
        .unwrap()["id"]
        .clone();
    let bevel_number: u64 = bevel_id.as_str().unwrap()[1..].parse().unwrap();
    assert_eq!(cut["params"]["target"], json!(bevel_number), "{cut}");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A union inserted before a subtraction whose tool it joins: the tool is
/// re-pointed to the union, which holds the other body too, and the entry
/// says so. Only the command that re-pointed lists it: an edit of the
/// inserted feature afterwards lists nothing.
#[test]
fn a_tool_re_pointed_to_a_larger_body_is_warned_of_once() {
    let mut s = new(1.0);
    build(&mut s, &part(1.0));
    run(
        &mut s,
        "feature.block",
        json!({"size": [4, 4, 4], "position": [30, 1, 1], "name": "Spare"}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"size": [2, 2, 2], "position": [31, 2, 3], "name": "E"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Spare", "tool": "E", "name": "Notch"}),
    );
    run(&mut s, "history.rollback", json!({"feature": "E"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [1, 1, 1], "position": [31.5, 2.5, 4.5], "name": "A"}),
    );
    let joined = run(
        &mut s,
        "feature.boolean",
        json!({"op": "union", "target": "A", "tool": "E", "name": "U"}),
    );
    let listed = joined["repointed"].as_array().unwrap();
    assert_eq!(listed.len(), 1, "{joined}");
    assert_eq!(listed[0]["feature"], "Notch");
    assert_eq!(listed[0]["param"], "tool");
    let warning = listed[0]["warning"].as_str().unwrap();
    assert!(
        warning.contains("`A`") && warning.contains("more"),
        "{warning}"
    );
    // An edit of U while the marker is still on it re-points nothing.
    let edited = run(
        &mut s,
        "feature.edit",
        json!({"feature": "U", "op": "union"}),
    );
    assert!(edited.get("repointed").is_none(), "{edited}");
}
