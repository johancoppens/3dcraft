//! A scene rebuilt after an edit meshes only the bodies the edit changed
//! (`crates/engine/src/meshes.rs`), and is the scene a fresh session
//! builds, bit for bit: the same triangles in the same order, at every
//! quality, whether a body's mesh was kept from an earlier state or made
//! now (AGENTS.md rule 5, determinism; docs/design/tessellator.md › *A
//! scene rebuild meshes only what changed*).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Scene, SceneQuality, Session, Workspace};
use serde_json::{Value, json};
use std::sync::Arc;

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// A part of several bodies with curved faces: a drilled flange, and
/// within its box a post, a knob, a row of studs and a ring. Each step is
/// one command, so the history can be rolled back body by body; the
/// later bodies lie within the flange's box, so rolling them back leaves
/// the box, and the quick first picture's tolerance, as they were.
fn script() -> Vec<(&'static str, Value)> {
    vec![
        ("file.new", json!({"units": "mm"})),
        ("sketch.create", json!({"plane": "zx", "name": "Section"})),
        (
            "sketch.polyline",
            json!({"sketch": "Section", "closed": true,
                   "points": [[0, 12], [0, 40], [10, 40], [10, 20], [30, 20], [34, 16], [34, 12]]}),
        ),
        (
            "feature.revolve",
            json!({"sketch": "Section", "axis": "x", "name": "Flange"}),
        ),
        (
            "feature.cylinder",
            json!({"radius": 4, "height": 20, "position": [30, 0, -5], "name": "Bolt"}),
        ),
        (
            "feature.pattern",
            json!({"source": "Bolt", "count": 6, "angle": "60 deg", "axis": [0, 0, 1],
                   "point": [0, 0, 0], "name": "Bolts"}),
        ),
        (
            "feature.boolean",
            json!({"op": "subtract", "target": "Flange", "tool": "Bolts", "name": "Drilled"}),
        ),
        (
            "feature.cylinder",
            json!({"radius": 3, "height": 20, "position": [10, 0, 2], "name": "Post"}),
        ),
        (
            "feature.sphere",
            json!({"radius": 2, "position": [15, 20, 10], "name": "Knob"}),
        ),
        (
            "feature.block",
            json!({"size": [2, 2, 2], "position": [5, -20, 5], "name": "Stud"}),
        ),
        (
            "feature.pattern",
            json!({"source": "Stud", "count": 3, "step": [0, 5, 0], "name": "Studs"}),
        ),
        (
            "feature.torus",
            json!({"major": 8, "minor": 1, "position": [20, 0, 12], "axis": [1, 0, 0], "name": "Ring"}),
        ),
    ]
}

fn fresh(steps: &[(&str, Value)]) -> Session {
    let mut session = Session::new(Workspace::none());
    for (id, params) in steps {
        run(&mut session, id, params.clone());
    }
    session
}

fn scene(session: &mut Session, quality: SceneQuality) -> Arc<Scene> {
    session.scene_with(quality).unwrap().expect("a document")
}

/// The bits of every number of the meshes, in order: equal only when
/// the scenes are the same scene.
fn mesh_bits(out: &mut Vec<u64>, mesh: &craft3d_tess::Mesh) {
    for p in mesh.positions.iter().chain(&mesh.normals) {
        out.extend([p.x.to_bits(), p.y.to_bits(), p.z.to_bits()]);
    }
    out.extend(mesh.triangles.iter().flatten().map(|&k| u64::from(k)));
    out.extend(mesh.face_ids.iter().map(|&f| u64::from(f)));
}

fn bits(scene: &Scene) -> Vec<u64> {
    let mut out = Vec::new();
    mesh_bits(&mut out, &scene.mesh);
    for body in scene.bodies.iter() {
        mesh_bits(&mut out, &body.mesh);
        out.push(u64::from(body.first_face_id));
        for edge in body.edges.iter() {
            for p in edge {
                out.extend([p.x.to_bits(), p.y.to_bits(), p.z.to_bits()]);
            }
        }
    }
    out.push(scene.tolerance.to_bits());
    out.push(scene.max_angle.map_or(0, f64::to_bits));
    out.push(u64::from(scene.coarsened));
    out
}

fn assert_same(cold: &Scene, warm: &Scene, what: &str) {
    assert_eq!(cold.bodies.len(), warm.bodies.len(), "{what}: bodies");
    assert_eq!(cold.problems, warm.problems, "{what}: problems");
    assert!(bits(cold) == bits(warm), "{what}: the scenes differ");
}

/// How many bodies of `now` have the very mesh (the same `Arc`) they had
/// in `before`, by feature, and how many were in `before` with another
/// mesh. (A body that was not in `before`, such as one a rolled-back
/// pattern gives back to its source, is meshed for the first time and
/// counted in neither.)
fn shared(before: &Scene, now: &Scene) -> (usize, usize) {
    let (mut same, mut other) = (0, 0);
    for body in now.bodies.iter() {
        match before.bodies.iter().find(|b| b.feature == body.feature) {
            Some(earlier) if Arc::ptr_eq(&earlier.mesh, &body.mesh) => {
                assert!(Arc::ptr_eq(&earlier.edges, &body.edges));
                same += 1;
            }
            Some(_) => other += 1,
            None => {}
        }
    }
    (same, other)
}

/// The qualities whose tolerance does not follow the model: a tolerance
/// given, a picture's, and the quick first picture's (a fraction of the
/// diagonal of the box around the bodies, which the script keeps). Every
/// body that stays keeps its mesh at these.
fn fixed() -> Vec<SceneQuality> {
    vec![
        SceneQuality::Tolerance(0.05),
        SceneQuality::Picture(0.02),
        SceneQuality::Preview,
    ]
}

/// The qualities whose tolerance is a fraction of the model's size (the
/// side of a square with its area), which every edit moves: the
/// automatic tolerance exactly, so a body that stays is meshed again at
/// the new tolerance; the display rule's on a ladder of rungs 9 % apart,
/// so it stays unless the size has moved to another rung, and the bodies
/// that stay keep their meshes while it does.
fn sized() -> Vec<SceneQuality> {
    vec![SceneQuality::Automatic, SceneQuality::display()]
}

/// Rolling the marker back through the history one feature at a time,
/// then forward again, and editing the last feature: at every quality
/// each scene is the one a fresh session builds, and at every quality
/// whose tolerance stays, every body that stays keeps the very mesh it
/// had.
#[test]
fn rolling_back_and_editing_keep_the_meshes_of_the_bodies_that_stay() {
    let steps = script();
    let mut warm = fresh(&steps);
    let features: Vec<String> = run(&mut warm, "document.inspect", json!({}))["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["id"].as_str().unwrap().to_string())
        .collect();
    assert!(features.len() >= 8);
    let fixed = fixed();
    let all: Vec<SceneQuality> = fixed.iter().chain(&sized()).copied().collect();
    let mut shown: Vec<Arc<Scene>> = all.iter().map(|&q| scene(&mut warm, q)).collect();
    assert!(
        shown[0].bodies.len() >= 5,
        "{} bodies",
        shown[0].bodies.len()
    );

    // Back, one feature at a time, to the third feature.
    let mut edits: Vec<(&str, Value)> = Vec::new();
    let mut stayed = 0;
    let mut display_stayed = 0;
    for id in features.iter().rev().skip(1).take(features.len() - 3) {
        let rollback = ("history.rollback", json!({"feature": id}));
        run(&mut warm, rollback.0, rollback.1.clone());
        edits.push(rollback);
        let mut cold = fresh(&steps);
        for (id, params) in &edits {
            run(&mut cold, id, params.clone());
        }
        for (k, &quality) in all.iter().enumerate() {
            let now = scene(&mut warm, quality);
            assert_same(
                &scene(&mut cold, quality),
                &now,
                &format!("{id} {quality:?}"),
            );
            let display = matches!(quality, SceneQuality::Display { .. });
            if k < fixed.len() || (display && now.tolerance == shown[k].tolerance) {
                // A rollback takes bodies away and changes none.
                let (same, other) = shared(&shown[k], &now);
                // (Rolling back the boolean gives the flange and the
                // bolts back: nothing stays then, and nothing is meshed
                // again.)
                assert!(
                    other == 0,
                    "{id} {quality:?}: a body that stayed was meshed again (tolerance {} -> {})",
                    shown[k].tolerance,
                    now.tolerance,
                );
                stayed += same;
                if display {
                    display_stayed += 1;
                }
            }
            shown[k] = now;
        }
    }
    assert!(stayed >= 3 * fixed.len(), "{stayed} bodies kept their mesh");
    // The display tolerance stays on its rung through most of the steps
    // (the bodies rolled back are small beside the flange).
    assert!(
        display_stayed >= (features.len() - 3) / 2,
        "the display tolerance stayed on its rung through {display_stayed} steps"
    );
    // Forward again in one go: regeneration builds the rolled-back
    // features again, so their bodies are new bodies, meshed as a fresh
    // session meshes them.
    run(&mut warm, "history.rollback", json!({"feature": null}));
    let mut cold = fresh(&steps);
    for (k, &quality) in all.iter().enumerate() {
        let now = scene(&mut warm, quality);
        assert_same(
            &scene(&mut cold, quality),
            &now,
            &format!("forward {quality:?}"),
        );
        shown[k] = now;
    }

    // An edit of the last body: only it is meshed again.
    let edit = ("feature.edit", json!({"feature": "Ring", "minor": 1.5}));
    run(&mut warm, edit.0, edit.1.clone());
    let mut cold = fresh(&steps);
    run(&mut cold, edit.0, edit.1.clone());
    for (k, &quality) in all.iter().enumerate() {
        let now = scene(&mut warm, quality);
        assert_same(
            &scene(&mut cold, quality),
            &now,
            &format!("edit {quality:?}"),
        );
        if k < fixed.len() {
            let (same, other) = shared(&shown[k], &now);
            assert_eq!((same, other), (now.bodies.len() - 1, 1), "edit {quality:?}");
        }
    }
}

/// The meshes made within a budget and without one are the same meshes,
/// so a scene of either kind serves the other: the quick first picture
/// (each body within a budget of its own) and the scene at its tolerance
/// (no budget) share every mesh, whichever is built first, and each is
/// the scene a fresh session builds. (A scene's budget, the display
/// rule's ladder, and no budget: `meshes::tests`.)
#[test]
fn budgeted_and_plain_builds_share_their_meshes() {
    let steps = script();
    let mut warm = fresh(&steps);
    let first = scene(&mut warm, SceneQuality::Preview);
    assert!(!first.coarsened);
    let plain = scene(&mut warm, SceneQuality::Tolerance(first.tolerance));
    let (same, other) = shared(&first, &plain);
    assert_eq!((same, other), (first.bodies.len(), 0));
    let mut cold = fresh(&steps);
    assert_same(
        &scene(&mut cold, SceneQuality::Tolerance(first.tolerance)),
        &plain,
        "the plain scene after the first picture",
    );

    // The other way round.
    let mut warm = fresh(&steps);
    let plain = scene(&mut warm, SceneQuality::Tolerance(first.tolerance));
    let again = scene(&mut warm, SceneQuality::Preview);
    let (same, other) = shared(&plain, &again);
    assert_eq!((same, other), (plain.bodies.len(), 0));
    let mut cold = fresh(&steps);
    assert_same(
        &scene(&mut cold, SceneQuality::Preview),
        &again,
        "the first picture after the plain scene",
    );
}

/// The meshes kept stay within the budget the scenes have, and go with
/// the states that are no longer held.
#[test]
fn the_meshes_kept_stay_within_the_budget() {
    let steps = script();
    let mut session = fresh(&steps);
    scene(&mut session, SceneQuality::display());
    scene(&mut session, SceneQuality::Automatic);
    let bytes = session.mesh_bytes();
    assert!(bytes > 0);
    assert!(
        bytes <= session.scene_bytes(),
        "{bytes} > {}",
        session.scene_bytes()
    );
    session.set_scene_budget(bytes / 2);
    assert!(session.mesh_bytes() <= bytes / 2);
    // Meshed again when asked, within the budget.
    run(
        &mut session,
        "feature.edit",
        json!({"feature": "Ring", "minor": 0.5}),
    );
    scene(&mut session, SceneQuality::Automatic);
    assert!(session.mesh_bytes() <= bytes / 2);
}
