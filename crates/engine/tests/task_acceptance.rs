//! The R1 acceptance (`docs/testing.md` › *The task set*): every task of
//! the set built by an agent over MCP from its description alone. The
//! scripts in `docs/tasks/acceptance/` are what the first such agent sent
//! (`command_run` after `command_run`, 2026-10-22), written without a look
//! at the reference solutions or the expectations; each is scored here as
//! `craft3d-cli score TASK --script FILE` scores it, so the acceptance is
//! repeated at every gate: a change that breaks what an agent built from a
//! description turns this red.
//!
//! Also here: the friction points that agent met and that were fixed in
//! the platform, each held to its fix.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::run;
use craft3d_engine::task_score::score_task;
use serde_json::{Value, json};
use std::path::PathBuf;

fn tasks_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/tasks")
}

fn read(path: &PathBuf) -> Value {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path:?}: {e}"))
}

fn json_stems(dir: &PathBuf) -> Vec<String> {
    let mut stems: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .map(|p| p.file_stem().unwrap().to_str().unwrap().to_string())
        .collect();
    stems.sort();
    stems
}

/// Every task has the agent's candidate, and every candidate passes: every
/// expectation, for the task's values and every variant's, with no step
/// and no feature failed.
#[test]
fn every_task_built_by_an_agent_from_its_description_passes() {
    let tasks = json_stems(&tasks_dir());
    let candidates = json_stems(&tasks_dir().join("acceptance"));
    assert_eq!(
        tasks, candidates,
        "docs/tasks and docs/tasks/acceptance differ"
    );
    let mut failed = Vec::new();
    for id in &tasks {
        let task = read(&tasks_dir().join(format!("{id}.json")));
        let script = read(&tasks_dir().join("acceptance").join(format!("{id}.json")));
        let card = score_task(&task, &script).unwrap_or_else(|e| panic!("{id}: {}", e.message()));
        if card["pass"] != true || card["refusals"] != 0 {
            failed.push(format!("{id}: {card}"));
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

/// The summaries of feature.block, feature.cylinder and feature.cone say
/// how their faces are numbered (an agent naming the wall of a cylinder for
/// a thread had to find it under sketch.create); what they say is true.
#[test]
fn the_documented_face_numbers_of_primitives_are_the_faces() {
    let mut s = common::session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [10, 20, 30], "name": "B"}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": 2, "height": 5, "position": [50, 0, 0], "name": "C"}),
    );
    run(
        &mut s,
        "feature.cone",
        json!({"radius": 2, "height": 5, "position": [80, 0, 0], "name": "K"}),
    );
    let summary = |id: &str| {
        craft3d_engine::Session::commands()
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.summary.to_string())
            .unwrap_or_default()
    };
    assert!(summary("feature.block").contains("0 bottom, 1 top, 2 front, 3 back, 4 left, 5 right"));
    assert!(
        summary("feature.cylinder")
            .contains("F1:face(0) the wall, F1:face(1) the base and F1:face(2) the top")
    );
    assert!(summary("feature.cone").contains("F1:face(0) the side and F1:face(1) the base"));
    let one = |s: &mut craft3d_engine::Session, filter: Value| {
        let found = run(s, "select.query", json!({"kind": "face", "where": filter}));
        assert_eq!(found["count"], 1, "{filter}: {found}");
    };
    for (face, axis) in [
        ("F1:face(0)", "-z"),
        ("F1:face(1)", "z"),
        ("F1:face(2)", "-y"),
        ("F1:face(3)", "y"),
        ("F1:face(4)", "-x"),
        ("F1:face(5)", "x"),
        ("F2:face(1)", "-z"),
        ("F2:face(2)", "z"),
        ("F3:face(1)", "-z"),
    ] {
        one(&mut s, json!({"name": face, "normal": {"along": axis}}));
    }
    one(&mut s, json!({"name": "F2:face(0)", "surface": "cylinder"}));
    one(&mut s, json!({"name": "F3:face(0)", "surface": "cone"}));
}

/// A negative horizontal distance (a point to the left of the origin)
/// is refused with a reason that says what to give instead: the size, the
/// side being the one the sketch is drawn on. That is how the agent
/// pinned the corners of a hexagon. (Until 2026-10-23 the constraint was
/// added and failed the sketch, with the same reason; refused at the
/// command since, roadmap R1, the sketch is never touched.)
#[test]
fn a_negative_distance_says_to_give_its_size_and_draw_the_side() {
    let mut s = common::session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "sketch.create", json!({"name": "S", "plane": "xy"}));
    run(&mut s, "sketch.point", json!({"sketch": "S", "at": [0, 0]}));
    run(
        &mut s,
        "sketch.point",
        json!({"sketch": "S", "at": [-3, 0]}),
    );
    run(
        &mut s,
        "sketch.constrain",
        json!({"sketch": "S", "type": "fixed", "entities": ["e1"]}),
    );
    let revision = run(&mut s, "sketch.inspect", json!({"sketch": "S"}));
    let why = s
        .execute(
            "sketch.constrain",
            &json!({"sketch": "S", "type": "distance", "entities": ["e1", "e2"],
                    "value": "-4 mm", "direction": "horizontal"}),
        )
        .unwrap_err()
        .message()
        .to_string();
    assert!(
        why.contains("a distance cannot be negative") && why.contains("give its size"),
        "{why}"
    );
    assert_eq!(
        run(&mut s, "sketch.inspect", json!({"sketch": "S"})),
        revision
    );
    // The size alone, with the point drawn on the left, puts it on the left.
    let good = run(
        &mut s,
        "sketch.constrain",
        json!({"sketch": "S", "type": "distance", "entities": ["e1", "e2"],
               "value": "4 mm", "direction": "horizontal"}),
    );
    assert_eq!(good["status"], "ok", "{good}");
    let sketch = run(&mut s, "sketch.inspect", json!({"sketch": "S"}));
    let e2 = sketch["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "e2")
        .unwrap()
        .clone();
    assert!((e2["x"].as_f64().unwrap() + 4.0).abs() < 1e-9, "{e2}");
}
