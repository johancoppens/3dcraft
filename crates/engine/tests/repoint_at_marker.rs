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
        json!({"size": [5, 5, 4], "position": [8, 3, 3], "name": "Spare"}),
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
