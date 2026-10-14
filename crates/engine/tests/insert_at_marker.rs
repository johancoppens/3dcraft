//! With the history rolled back (`history.rollback`), a feature added now
//! goes in right after the rollback marker, and the marker moves onto it:
//! what was rolled back stays rolled back, after the new feature. Every
//! family of commands that adds a feature is exercised: a sketch, a
//! primitive, an extrusion (alone and with a boolean at once), a chamfer,
//! a boolean, a datum, a pattern, a mirror, a loft and a sweep.
//!
//! After every step: regeneration edit by edit equals a full one (`run`);
//! undo gives back the document before the step and redo the one after it,
//! exactly; and at the end the document saved and reopened is the same,
//! marker and order included, and adding to the reopened document still
//! inserts at the marker. Rolled forward, the document is the one built in
//! that order without a marker (an independent second construction).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Access, Session, Workspace};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    let result = session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"));
    assert_ne!(
        session.regeneration_matches_a_full_one(),
        Some(false),
        "after {id} {params}"
    );
    result
}

/// What a document is, without how it got there or where it is kept.
/// Bounding boxes are left out: their tolerance follows the size of the
/// whole model (see `failed_features.rs`).
fn state(session: &mut Session) -> Value {
    let mut document = run(session, "document.inspect", json!({}));
    let map = document.as_object_mut().unwrap();
    for key in ["history", "dirty", "path", "document"] {
        map.remove(key);
    }
    for body in map["bodies"].as_array_mut().unwrap() {
        let body = body.as_object_mut().unwrap();
        body.remove("bbox");
        body.remove("bboxTolerance");
    }
    document
}

fn names_in_order(document: &Value) -> Vec<String> {
    document["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["name"].as_str().unwrap().to_string())
        .collect()
}

fn feature<'a>(document: &'a Value, name: &str) -> &'a Value {
    document["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == name)
        .unwrap_or_else(|| panic!("no feature {name}"))
}

fn workdir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("insert-at-marker-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn on_disk(dir: &Path) -> Workspace {
    Workspace {
        read: Access::Root(dir.to_path_buf()),
        write: Access::Root(dir.to_path_buf()),
    }
}

/// One step: the commands that set it up (sketch entities and the like),
/// then the command that adds the feature(s) under test, with the names it
/// adds in the order they land in the history.
struct Step {
    setup: Vec<(&'static str, Value)>,
    add: (&'static str, Value),
    adds: &'static [&'static str],
}

fn square(sketch: &str, from: [f64; 2], to: [f64; 2]) -> (&'static str, Value) {
    (
        "sketch.polyline",
        json!({"sketch": sketch, "closed": true,
               "points": [from, [to[0], from[1]], to, [from[0], to[1]]]}),
    )
}

fn steps() -> Vec<Step> {
    vec![
        // A sketch, and an extrusion of it.
        Step {
            setup: vec![],
            add: (
                "sketch.create",
                json!({"plane": "xy", "offset": -1, "name": "Cutter"}),
            ),
            adds: &["Cutter"],
        },
        Step {
            setup: vec![square("Cutter", [1.0, 1.0], [2.0, 2.0])],
            add: (
                "feature.extrude",
                json!({"sketch": "Cutter", "distance": 5, "name": "Pin"}),
            ),
            adds: &["Pin"],
        },
        // A chamfer on the base block, which was there before the marker.
        Step {
            setup: vec![],
            add: (
                "feature.chamfer",
                json!({"edges": [["F1:face(1)", "F1:face(2)"]], "distance": 0.5, "name": "Bevel"}),
            ),
            adds: &["Bevel"],
        },
        // A boolean between two inserted features.
        Step {
            setup: vec![],
            add: (
                "feature.boolean",
                json!({"op": "subtract", "target": "Bevel", "tool": "Pin", "name": "Holed"}),
            ),
            adds: &["Holed"],
        },
        // An extrusion cut into a body at once: two features in one step,
        // both at the marker, in order.
        Step {
            setup: vec![
                (
                    "sketch.create",
                    json!({"plane": "xy", "offset": 1.5, "name": "Notch"}),
                ),
                square("Notch", [3.0, 0.5], [3.5, 2.5]),
            ],
            add: (
                "feature.extrude",
                json!({"sketch": "Notch", "distance": 1, "cut": "Holed", "name": "Notched"}),
            ),
            adds: &["Extrude1", "Notched"],
        },
        // A datum plane, and a mirror in it.
        Step {
            setup: vec![],
            add: (
                "datum.plane",
                json!({"from": "yz", "distance": -5, "name": "Wall"}),
            ),
            adds: &["Wall"],
        },
        Step {
            setup: vec![],
            add: (
                "feature.mirror",
                json!({"source": "Notched", "plane": "Wall", "keep": true, "name": "Twin"}),
            ),
            adds: &["Twin"],
        },
        // A primitive and a pattern of it.
        Step {
            setup: vec![],
            add: (
                "feature.block",
                json!({"size": [1, 1, 1], "position": [20, 0, 0], "name": "Peg"}),
            ),
            adds: &["Peg"],
        },
        Step {
            setup: vec![],
            add: (
                "feature.pattern",
                json!({"source": "Peg", "count": 3, "step": [3, 0, 0], "name": "Row"}),
            ),
            adds: &["Row"],
        },
        // A loft through two sketches.
        Step {
            setup: vec![
                (
                    "sketch.create",
                    json!({"plane": "xy", "offset": 20, "name": "Low"}),
                ),
                square("Low", [0.0, 0.0], [4.0, 4.0]),
                (
                    "sketch.create",
                    json!({"plane": "xy", "offset": 24, "name": "High"}),
                ),
                square("High", [1.0, 1.0], [3.0, 3.0]),
            ],
            add: (
                "feature.loft",
                json!({"sections": ["Low", "High"], "name": "Tower"}),
            ),
            adds: &["Tower"],
        },
        // A sweep of a circle along a bent path.
        Step {
            setup: vec![
                (
                    "sketch.create",
                    json!({"plane": "zx", "offset": 0, "name": "Section"}),
                ),
                (
                    "sketch.circle",
                    json!({"sketch": "Section", "center": [10, -30], "radius": 1}),
                ),
                (
                    "sketch.create",
                    json!({"plane": "xy", "offset": 10, "name": "Route"}),
                ),
                (
                    "sketch.path",
                    json!({"sketch": "Route", "start": [-30, 0], "segments": [
                        {"line": [-30, 10]},
                        {"arc": [-35, 15], "center": [-35, 10]},
                        {"line": [-47, 15]},
                    ]}),
                ),
            ],
            add: (
                "feature.sweep",
                json!({"sketch": "Section", "path": "Route", "name": "Pipe"}),
            ),
            adds: &["Pipe"],
        },
    ]
}

/// A block (F1) and, after it, a body that stays rolled back throughout.
fn start(workspace: Workspace) -> Session {
    let mut s = Session::new(workspace);
    run(&mut s, "file.new", json!({}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [4, 3, 2], "name": "Base"}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"size": [1, 1, 1], "position": [100, 0, 0], "name": "Later"}),
    );
    s
}

#[test]
fn every_feature_command_inserts_at_the_rollback_marker() {
    let dir = workdir("families");
    let mut s = start(on_disk(&dir));
    run(&mut s, "history.rollback", json!({"feature": "Base"}));
    for step in steps() {
        for (id, params) in &step.setup {
            run(&mut s, id, params.clone());
        }
        let before = state(&mut s);
        // Sketches the setup made went in at the marker too.
        let mut order = names_in_order(&before);
        assert_eq!(order.pop().as_deref(), Some("Later"));
        let (id, params) = &step.add;
        let added = run(&mut s, id, params.clone());
        let context = format!("{id} {params}");
        let after = state(&mut s);
        order.extend(step.adds.iter().map(|n| n.to_string()));
        let mut expected = order.clone();
        expected.push("Later".to_string());
        assert_eq!(names_in_order(&after), expected, "{context}");
        // The marker is on the last feature the step added.
        let last = step.adds.last().unwrap();
        assert_eq!(after["rollback"], feature(&after, last)["id"], "{context}");
        // Everything up to it is built, the rest is rolled back.
        for name in step.adds {
            assert_eq!(feature(&after, name)["status"], "ok", "{context}: {name}");
        }
        assert_eq!(feature(&after, "Later")["status"], "rolledBack");
        if id.starts_with("feature.") || id.starts_with("datum.") {
            assert_eq!(added["status"], "ok", "{context}: {added}");
        }
        assert_eq!(after["ok"], true, "{context}: {after}");
        // Undo gives back the document before the step, redo the one after.
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(state(&mut s), before, "{context}: undone");
        run(&mut s, "edit.redo", json!({}));
        assert_eq!(state(&mut s), after, "{context}: redone");
    }

    // Saved and reopened: order, marker, statuses and bodies are the same.
    let rolled = state(&mut s);
    run(
        &mut s,
        "file.save",
        json!({"path": "inserted.3dcraft", "overwrite": true}),
    );
    let mut reopened = Session::new(on_disk(&dir));
    run(
        &mut reopened,
        "file.open",
        json!({"path": "inserted.3dcraft"}),
    );
    assert_eq!(state(&mut reopened), rolled, "saved and reopened");
    // The reopened document goes on inserting at its marker.
    run(
        &mut reopened,
        "feature.cylinder",
        json!({"radius": 1, "height": 2, "position": [0, 20, 0], "name": "Stud"}),
    );
    let more = state(&mut reopened);
    let names = names_in_order(&more);
    assert_eq!(&names[names.len() - 2..], ["Stud", "Later"]);
    assert_eq!(more["rollback"], feature(&more, "Stud")["id"]);

    // Rolled forward, it is the document built in that order without a
    // marker: the same features with the same statuses and bodies of the
    // same size. (Ids differ: Later was F2 here.)
    run(&mut s, "history.rollback", json!({"feature": null}));
    let forward = state(&mut s);
    let mut straight = start(Workspace::none());
    run(&mut straight, "feature.delete", json!({"feature": "Later"}));
    for step in steps() {
        for (id, params) in &step.setup {
            run(&mut straight, id, params.clone());
        }
        let (id, params) = &step.add;
        run(&mut straight, id, params.clone());
    }
    run(
        &mut straight,
        "feature.block",
        json!({"size": [1, 1, 1], "position": [100, 0, 0], "name": "Later"}),
    );
    let straight = state(&mut straight);
    assert_eq!(names_in_order(&forward), names_in_order(&straight));
    for name in names_in_order(&forward) {
        assert_eq!(
            feature(&forward, &name)["status"],
            feature(&straight, &name)["status"],
            "{name}"
        );
    }
    let measures = |document: &Value| -> Vec<(String, f64, f64)> {
        document["bodies"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| {
                (
                    b["name"].as_str().unwrap().to_string(),
                    b["volume"].as_f64().unwrap(),
                    b["area"].as_f64().unwrap(),
                )
            })
            .collect()
    };
    let (ours, theirs) = (measures(&forward), measures(&straight));
    assert_eq!(ours.len(), theirs.len());
    for ((name, v, a), (other, w, b)) in ours.iter().zip(&theirs) {
        assert_eq!(name, other);
        assert!(
            (v - w).abs() <= 1e-9 * w.abs().max(1.0),
            "{name}: {v} vs {w}"
        );
        assert!(
            (a - b).abs() <= 1e-9 * b.abs().max(1.0),
            "{name}: {a} vs {b}"
        );
    }
    assert_eq!(forward["ok"], true, "{forward}");
}

/// Without a marker a feature is appended, as always.
#[test]
fn without_a_marker_a_feature_is_appended() {
    let mut s = start(Workspace::none());
    run(
        &mut s,
        "feature.block",
        json!({"size": [1, 1, 1], "name": "Last"}),
    );
    let now = state(&mut s);
    assert_eq!(names_in_order(&now), ["Base", "Later", "Last"]);
    assert_eq!(now["rollback"], Value::Null);
}

/// A feature added at the marker that refers to a feature after it (still
/// rolled back) fails as out of order, with a reason, and is not built
/// from something that is not there; rolled forward it stays out of order
/// until it is moved.
#[test]
fn a_feature_at_the_marker_made_from_a_rolled_back_one_is_out_of_order() {
    let mut s = start(Workspace::none());
    run(&mut s, "history.rollback", json!({"feature": "Base"}));
    let added = run(
        &mut s,
        "feature.boolean",
        json!({"op": "union", "target": "Base", "tool": "Later", "name": "Joined"}),
    );
    assert_eq!(added["status"], "failed", "{added}");
    let now = state(&mut s);
    assert_eq!(names_in_order(&now), ["Base", "Joined", "Later"]);
    assert!(
        feature(&now, "Joined")["error"]
            .as_str()
            .unwrap()
            .contains("`Later` comes after this feature"),
        "{now}"
    );
}
