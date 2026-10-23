//! Features switched off, by `feature.suppress` or by a condition
//! (`feature.when`): the features after them go on as if they were not in
//! the history (docs/design/regeneration.md › *Switched off*). A feature
//! that changes a body passes it on unchanged; one that makes a body makes
//! none, and a boolean whose tool is gone passes its target on. Held to the
//! document built without the feature (volumes, body for body) at three
//! scales, to a regeneration from nothing after every command (`checked`),
//! to identical bodies when switched on again, and to clear failures for
//! what only the switched-off feature makes.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{built, feature, inspect, new_document, refused, run, session, workdir};
use craft3d_engine::Session;
use serde_json::{Value, json};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

/// The features of the chain, in order.
const CHAIN: [&str; 7] = ["Plate", "Round", "Bevel", "Drill", "Cut", "Hollow", "Row"];

/// A plate (40 × 20 × 10, times `k`) rounded, bevelled, drilled, hollowed
/// and patterned twice, each feature changing the body of the one before.
/// The feature `skip` is left out, and what follows it is made from the
/// body before it: what switching it off has to give.
fn chain(s: &mut Session, k: f64, skip: Option<&str>) {
    new_document(s, k);
    let on = |name: &str| skip != Some(name);
    let mut last = "Plate".to_string();
    built(
        s,
        "feature.block",
        json!({"size": [40.0 * k, 20.0 * k, 10.0 * k], "name": "Plate"}),
    );
    if on("Round") {
        built(
            s,
            "feature.fillet",
            json!({"edges": [["F1:face(0)", "F1:face(2)"]], "radius": 2.0 * k, "name": "Round"}),
        );
        last = "Round".to_string();
    }
    if on("Bevel") {
        built(
            s,
            "feature.chamfer",
            json!({"edges": [["F1:face(1)", "F1:face(3)"]], "distance": k, "name": "Bevel"}),
        );
        last = "Bevel".to_string();
    }
    built(
        s,
        "feature.cylinder",
        json!({"radius": 3.0 * k, "height": 30.0 * k, "position": [10.0 * k, 10.0 * k, -5.0 * k], "name": "Drill"}),
    );
    if on("Cut") {
        built(
            s,
            "feature.boolean",
            json!({"op": "subtract", "target": last, "tool": "Drill", "name": "Cut"}),
        );
        last = "Cut".to_string();
    }
    if on("Hollow") {
        built(
            s,
            "feature.shell",
            json!({"source": last, "thickness": 0.5 * k, "name": "Hollow"}),
        );
        last = "Hollow".to_string();
    }
    if on("Row") {
        built(
            s,
            "feature.pattern",
            json!({"source": last, "count": 2, "step": [0, 30.0 * k, 0], "name": "Row"}),
        );
    }
}

/// The bodies of the document, each as `(volume, area, faces)`, in
/// history order.
fn bodies(s: &mut Session) -> Vec<(f64, f64, u64)> {
    inspect(s)["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| {
            assert_eq!(b["valid"], true, "{b}");
            (
                b["volume"].as_f64().unwrap(),
                b["area"].as_f64().unwrap(),
                b["faces"].as_u64().unwrap(),
            )
        })
        .collect()
}

fn same_bodies(got: &[(f64, f64, u64)], expected: &[(f64, f64, u64)], context: &str) {
    assert_eq!(got.len(), expected.len(), "{context}: {got:?} {expected:?}");
    for (a, b) in got.iter().zip(expected) {
        assert!(
            (a.0 - b.0).abs() <= 1e-9 * b.0.abs()
                && (a.1 - b.1).abs() <= 1e-9 * b.1.abs()
                && a.2 == b.2,
            "{context}: {got:?} {expected:?}"
        );
    }
}

/// What the document is, without its undo history: the features with
/// their states and the bodies, every measure and name.
fn model(s: &mut Session) -> Value {
    let document = inspect(s);
    json!({"features": document["features"], "bodies": document["bodies"]})
}

fn status(s: &mut Session, name: &str) -> String {
    feature(s, name)["status"].as_str().unwrap().to_string()
}

/// Suppressing any feature of the chain that changes a body gives the
/// bodies of the chain built without it, at three scales; nothing fails;
/// switched on again, the document is what it was, to the bit.
#[test]
fn a_suppressed_feature_passes_its_body_on_as_if_it_were_not_there() {
    for k in SCALES {
        let mut whole = session();
        chain(&mut whole, k, None);
        let intact = model(&mut whole);
        for name in &CHAIN[1..] {
            if *name == "Drill" {
                continue;
            }
            let context = format!("{name} at {k}");
            let mut without = session();
            chain(&mut without, k, Some(name));
            let expected = bodies(&mut without);
            run(&mut whole, "feature.suppress", json!({"feature": name}));
            let state = inspect(&mut whole);
            assert_eq!(state["ok"], true, "{context}: {state}");
            assert_eq!(status(&mut whole, name), "suppressed", "{context}");
            assert!(
                feature(&mut whole, name).get("passesOn").is_some(),
                "{context}: {}",
                feature(&mut whole, name)
            );
            same_bodies(&bodies(&mut whole), &expected, &context);
            run(
                &mut whole,
                "feature.suppress",
                json!({"feature": name, "suppressed": false}),
            );
            assert_eq!(model(&mut whole), intact, "{context}: switched on again");
        }
    }
}

/// A suppressed primitive makes no body: the boolean that drills with it
/// passes its target on; a suppressed target leaves the boolean without a
/// body and the drill alone, and the shell and pattern after it pass
/// nothing on.
#[test]
fn a_suppressed_operand_leaves_the_boolean_as_identity() {
    let mut s = session();
    chain(&mut s, 1.0, None);
    run(&mut s, "feature.suppress", json!({"feature": "Drill"}));
    let cut = feature(&mut s, "Cut");
    assert_eq!(cut["status"], "ok", "{cut}");
    assert_eq!(cut["passesOn"], "F3", "{cut}");
    let mut without = session();
    chain(&mut without, 1.0, Some("Cut"));
    let mut expected = bodies(&mut without);
    // There the drill is a body of its own.
    expected.remove(0);
    same_bodies(&bodies(&mut s), &expected, "drill suppressed");
    run(&mut s, "edit.undo", json!({}));
    // The plate suppressed: nothing passes on but the drill.
    run(&mut s, "feature.suppress", json!({"feature": "Plate"}));
    for name in ["Round", "Bevel"] {
        // A fillet names faces of the plate, which is not there.
        let state = feature(&mut s, name);
        assert_eq!(state["status"], "failed", "{state}");
        assert!(
            state["error"].as_str().unwrap().contains("suppressed"),
            "{state}"
        );
    }
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "feature.suppress", json!({"feature": "Bevel"}));
    run(&mut s, "feature.suppress", json!({"feature": "Round"}));
    run(&mut s, "feature.suppress", json!({"feature": "Plate"}));
    let cut = feature(&mut s, "Cut");
    assert_eq!(cut["status"], "ok", "{cut}");
    assert_eq!(cut["passesNothing"], "`Plate` is suppressed", "{cut}");
    for name in ["Hollow", "Row"] {
        assert_eq!(status(&mut s, name), "ok");
        assert_eq!(
            feature(&mut s, name)["passesNothing"],
            "`Plate` is suppressed"
        );
    }
    let names: Vec<Value> = inspect(&mut s)["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["name"].clone())
        .collect();
    assert_eq!(names, [json!("Drill")]);
}

/// A reference to a face only the switched-off feature makes fails naming
/// it; it never lands on another face. Switched on, it builds again.
#[test]
fn a_face_made_by_a_suppressed_feature_is_not_there() {
    let mut s = session();
    new_document(&mut s, 1.0);
    built(
        &mut s,
        "feature.block",
        json!({"size": [40, 20, 10], "name": "Plate"}),
    );
    built(
        &mut s,
        "feature.chamfer",
        json!({"edges": [["F1:face(0)", "F1:face(2)"]], "distance": 2, "name": "Bevel"}),
    );
    // A sketch on the bevel's face, and a fillet of an edge the plate
    // made, which the bevel keeps.
    run(
        &mut s,
        "sketch.create",
        json!({"name": "OnBevel", "face": "F2:chamfer(0)"}),
    );
    built(
        &mut s,
        "feature.fillet",
        json!({"edges": [["F1:face(1)", "F1:face(3)"]], "radius": 1, "name": "Round"}),
    );
    assert_eq!(status(&mut s, "OnBevel"), "ok");
    let before = model(&mut s);
    let result = run(&mut s, "feature.suppress", json!({"feature": "Bevel"}));
    let failed = result["failed"].as_array().unwrap();
    assert_eq!(failed.len(), 1, "{result}");
    assert_eq!(failed[0]["feature"], "OnBevel", "{result}");
    let reason = failed[0]["error"].as_str().unwrap();
    assert_eq!(
        reason,
        "plane: `Bevel`, which makes the face F2:chamfer(0), is suppressed"
    );
    // The fillet is built on the plate as if the bevel were not there.
    assert_eq!(status(&mut s, "Round"), "ok");
    let mut plain = session();
    new_document(&mut plain, 1.0);
    built(
        &mut plain,
        "feature.block",
        json!({"size": [40, 20, 10], "name": "Plate"}),
    );
    built(
        &mut plain,
        "feature.fillet",
        json!({"edges": [["F1:face(1)", "F1:face(3)"]], "radius": 1, "name": "Round"}),
    );
    same_bodies(&bodies(&mut s), &bodies(&mut plain), "bevel suppressed");
    // Inactive by its condition: the same, said with its own word.
    run(
        &mut s,
        "feature.suppress",
        json!({"feature": "Bevel", "suppressed": false}),
    );
    run(
        &mut s,
        "feature.when",
        json!({"feature": "Bevel", "expr": "0"}),
    );
    assert_eq!(
        feature(&mut s, "OnBevel")["error"],
        "plane: `Bevel`, which makes the face F2:chamfer(0), is inactive"
    );
    run(
        &mut s,
        "feature.when",
        json!({"feature": "Bevel", "expr": null}),
    );
    assert_eq!(model(&mut s), before, "switched on again");
}

/// `when` with a boolean parameter and with a choice: the feature is
/// inactive when the condition is 0, behaves as suppressed, and comes back
/// with the same bodies; undo and redo; a condition that does not evaluate
/// fails the feature with the reason.
#[test]
fn a_condition_switches_a_feature_on_and_off() {
    let mut s = session();
    chain(&mut s, 1.0, None);
    let intact = bodies(&mut s);
    run(&mut s, "variable.set", json!({"name": "lid", "expr": "1"}));
    run(
        &mut s,
        "variable.publish",
        json!({"name": "lid", "label": "Lid", "kind": "boolean"}),
    );
    run(
        &mut s,
        "variable.set",
        json!({"name": "profile", "expr": "1"}),
    );
    run(
        &mut s,
        "variable.publish",
        json!({"name": "profile", "label": "Profile", "kind": "choice", "choices": ["Round", "Square"]}),
    );
    let set = run(
        &mut s,
        "feature.when",
        json!({"feature": "Round", "expr": "profile == 1"}),
    );
    assert_eq!(set["status"], "ok", "{set}");
    assert_eq!(set["when"], "profile == 1", "{set}");
    let set = run(
        &mut s,
        "feature.when",
        json!({"feature": "Cut", "expr": "lid"}),
    );
    assert_eq!(set["status"], "ok", "{set}");
    same_bodies(&bodies(&mut s), &intact, "both on");

    let mut no_round = session();
    chain(&mut no_round, 1.0, Some("Round"));
    run(
        &mut s,
        "param.set",
        json!({"values": {"profile": "Square"}}),
    );
    let round = feature(&mut s, "Round");
    assert_eq!(round["status"], "inactive", "{round}");
    assert_eq!(round["passesOn"], "F1", "{round}");
    same_bodies(&bodies(&mut s), &bodies(&mut no_round), "profile 2");

    let mut no_cut = session();
    chain(&mut no_cut, 1.0, Some("Cut"));
    run(
        &mut s,
        "param.set",
        json!({"values": {"profile": "Round", "lid": false}}),
    );
    assert_eq!(status(&mut s, "Round"), "ok");
    assert_eq!(status(&mut s, "Cut"), "inactive");
    same_bodies(&bodies(&mut s), &bodies(&mut no_cut), "no lid");
    let on = model(&mut s);
    run(&mut s, "param.set", json!({"values": {"lid": true}}));
    same_bodies(&bodies(&mut s), &intact, "lid again");
    // Undo goes back to the state before, exactly.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(model(&mut s), on);
    run(&mut s, "edit.redo", json!({}));
    same_bodies(&bodies(&mut s), &intact, "redone");

    // Suppression wins over a condition that holds.
    run(&mut s, "feature.suppress", json!({"feature": "Cut"}));
    assert_eq!(status(&mut s, "Cut"), "suppressed");
    run(&mut s, "edit.undo", json!({}));

    // A condition that does not evaluate, or has a unit, fails the feature.
    for (expr, words) in [("nothing_here", "nothing_here"), ("3 mm", "plain number")] {
        let result = run(
            &mut s,
            "feature.when",
            json!({"feature": "Cut", "expr": expr}),
        );
        assert_eq!(result["status"], "failed", "{result}");
        let reason = result["error"].as_str().unwrap();
        assert!(
            reason.starts_with("when: ") && reason.contains(words),
            "{reason}"
        );
        run(&mut s, "edit.undo", json!({}));
    }
    // null takes the condition away: one undo step.
    let undo = inspect(&mut s)["history"]["undo"].as_array().unwrap().len();
    let result = run(
        &mut s,
        "feature.when",
        json!({"feature": "Cut", "expr": null}),
    );
    assert!(result.get("when").is_none(), "{result}");
    assert_eq!(
        inspect(&mut s)["history"]["undo"].as_array().unwrap().len(),
        undo + 1
    );
}

/// Renaming a variable or a table rewrites the conditions that use it;
/// the variable is listed as used by the feature's `when`.
#[test]
fn renames_rewrite_conditions() {
    let mut s = session();
    chain(&mut s, 1.0, None);
    run(&mut s, "variable.set", json!({"name": "lid", "expr": "1"}));
    run(
        &mut s,
        "table.set",
        json!({"name": "opts", "columns": ["name", "hole"], "rows": [["a", 1], ["b", 0]]}),
    );
    run(
        &mut s,
        "feature.when",
        json!({"feature": "Cut", "expr": "lid && opts[1].hole"}),
    );
    let refusal = refused(&mut s, "variable.delete", json!({"name": "lid"}));
    assert!(
        refusal.contains("Cut") && refusal.contains("when"),
        "{refusal}"
    );
    run(
        &mut s,
        "variable.rename",
        json!({"name": "lid", "to": "cover"}),
    );
    run(
        &mut s,
        "table.rename",
        json!({"name": "opts", "to": "options"}),
    );
    assert_eq!(feature(&mut s, "Cut")["when"], "cover && options[1].hole");
    assert_eq!(status(&mut s, "Cut"), "ok");
    run(
        &mut s,
        "table.set",
        json!({"name": "options", "columns": ["name", "hole"], "rows": [["a", 0], ["b", 0]]}),
    );
    assert_eq!(status(&mut s, "Cut"), "inactive");
}

/// Saved and opened, a condition and the states it gives are the same.
#[test]
fn a_condition_survives_saving_and_opening() {
    let dir = workdir("switched-off", "save");
    let mut s = common::session_in(&dir);
    chain(&mut s, 1.0, None);
    run(&mut s, "variable.set", json!({"name": "lid", "expr": "0"}));
    run(
        &mut s,
        "feature.when",
        json!({"feature": "Cut", "expr": "lid"}),
    );
    run(&mut s, "feature.suppress", json!({"feature": "Round"}));
    let before = inspect(&mut s);
    run(&mut s, "file.save", json!({"path": "part.3dcraft"}));
    let mut reopened = common::session_in(&dir);
    run(&mut reopened, "file.open", json!({"path": "part.3dcraft"}));
    let after = inspect(&mut reopened);
    assert_eq!(after["features"], before["features"]);
    assert_eq!(after["bodies"], before["bodies"]);
}

/// Bad params are errors, never a crash.
#[test]
fn bad_params_are_refused() {
    let mut s = session();
    chain(&mut s, 1.0, None);
    for params in [
        json!({}),
        json!({"feature": "Cut"}),
        json!({"feature": "Nope", "expr": "1"}),
        json!({"feature": "Cut", "expr": "1 +"}),
        json!({"feature": "Cut", "expr": true}),
        json!({"feature": "Cut", "expr": [1]}),
        json!({"feature": "Cut", "expr": "1", "extra": 1}),
    ] {
        refused(&mut s, "feature.when", params);
    }
}
