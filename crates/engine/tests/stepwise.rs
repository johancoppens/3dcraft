//! A host with one thread (the page) gets a document ready a piece at a
//! time (`Session::prepare_some`): the regeneration a feature at a time,
//! the first picture and the display mesh a body at a time, the
//! assertions one at a time; and it ends with exactly what a session that
//! does it all at once shows.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{SceneQuality, Session, Workspace};
use serde_json::{Value, json};

/// Plates at scale `s` (in mm), joined, with a hole cut from the first,
/// and assertions on them.
fn build(session: &mut Session, s: f64) {
    let mm = |v: f64| format!("{} mm", v * s);
    let steps = [
        ("file.new", json!({"units": "mm"})),
        ("variable.set", json!({"name": "w", "expr": mm(40.0)})),
        (
            "feature.block",
            json!({"size": ["w", mm(20.0), mm(5.0)], "name": "A"}),
        ),
        (
            "feature.block",
            json!({"size": [mm(10.0), mm(10.0), mm(10.0)], "position": [mm(60.0), "0 mm", "0 mm"], "name": "B"}),
        ),
        (
            "feature.block",
            json!({"size": [mm(10.0), mm(10.0), mm(10.0)], "position": [mm(80.0), "0 mm", "0 mm"], "name": "C"}),
        ),
        (
            "feature.cylinder",
            json!({"radius": mm(2.0), "height": mm(20.0), "position": [mm(10.0), mm(10.0), mm(-5.0)], "name": "Pin"}),
        ),
        (
            "feature.boolean",
            json!({"op": "subtract", "target": "A", "tool": "Pin", "name": "Holed"}),
        ),
        (
            "assert.add",
            json!({"name": "two blocks", "measure": "volume", "of": ["B", "C"], "eq": format!("2 * ({})^3", mm(10.0)), "tol": "1e-6 * (1 mm)^3"}),
        ),
        (
            "assert.add",
            json!({"name": "apart", "measure": "overlap", "is": false}),
        ),
        (
            "assert.add",
            json!({"name": "plate", "measure": "volume", "of": "Holed", "min": format!("{} * (1 mm)^3", s * s * s)}),
        ),
    ];
    for (id, params) in steps {
        session
            .execute(id, &params)
            .unwrap_or_else(|e| panic!("{id}: {}", e.message()));
    }
}

/// The scene a viewer shows, as numbers that must be the same bits.
fn picture(session: &mut Session) -> Vec<u64> {
    let scene = session
        .scene_with(SceneQuality::display())
        .unwrap()
        .unwrap();
    assert!(!scene.preview, "the display scene");
    let mut bits: Vec<u64> = scene
        .mesh
        .positions
        .iter()
        .flat_map(|p| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()])
        .collect();
    bits.extend(scene.mesh.triangles.iter().flatten().map(|&i| u64::from(i)));
    bits.push(scene.tolerance.to_bits());
    bits
}

fn states(session: &mut Session) -> Value {
    session
        .execute("assert.list", &json!({ "wait": true }))
        .unwrap()["assertions"]
        .clone()
}

#[test]
fn a_piece_at_a_time_ends_as_all_at_once() {
    for s in [1e-2, 1.0, 1e3] {
        let mut whole = Session::new(Workspace::none());
        build(&mut whole, s);
        let want_picture = picture(&mut whole);
        let want_states = states(&mut whole);
        assert!(
            want_states
                .as_array()
                .unwrap()
                .iter()
                .all(|a| a["status"] == "passed"),
            "{want_states}"
        );

        let mut session = Session::new(Workspace::none());
        session.set_stepwise(true);
        build(&mut session, s);
        // An edit and back, so that everything is to be done again from
        // what is cached: here from a new parameter value.
        session
            .execute(
                "variable.set",
                &json!({"name": "w", "expr": format!("{} mm", 41.0 * s)}),
            )
            .unwrap();
        session
            .execute(
                "variable.set",
                &json!({"name": "w", "expr": format!("{} mm", 40.0 * s)}),
            )
            .unwrap();
        let mut seen: Vec<(String, usize, usize)> = Vec::new();
        let mut calls = 0;
        // Stop as soon as allowed: one feature, body or assertion a call.
        while let Some(p) = session.prepare_some(SceneQuality::display(), || true) {
            calls += 1;
            assert!(calls < 100, "no progress: {seen:?}");
            seen.push((p.phase.to_string(), p.step, p.steps));
        }
        let phases: Vec<&str> = seen.iter().map(|(p, ..)| p.as_str()).collect();
        assert!(phases.contains(&"checking"), "{seen:?} at scale {s}");
        // The assertions one at a time, of 3 (those whose bodies the edit
        // left as they were are not measured again).
        let checking: Vec<(usize, usize)> = seen
            .iter()
            .filter(|(p, ..)| p == "checking")
            .map(|(_, a, b)| (*a, *b))
            .collect();
        assert!(
            checking
                .windows(2)
                .all(|w| w[1].0 == w[0].0 + 1 && w[1].1 == 3)
                && checking.iter().all(|&(done, of)| of == 3 && done < 3),
            "{seen:?}"
        );
        // Phases in order, never back.
        let order = ["regenerating", "meshing", "refining", "checking"];
        let ranks: Vec<usize> = phases
            .iter()
            .map(|p| order.iter().position(|o| o == p).unwrap())
            .collect();
        assert!(ranks.windows(2).all(|w| w[0] <= w[1]), "{seen:?}");
        assert_eq!(picture(&mut session), want_picture, "scale {s}");
        assert_eq!(states(&mut session), want_states, "scale {s}");
    }
}

/// Opening a file regenerates it a feature at a time: each call builds
/// one more (counted from 1) and says which one is next.
#[test]
fn a_file_opened_is_regenerated_a_feature_at_a_time() {
    let dir = std::env::temp_dir().join(format!("craft3d-stepwise-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("plates.3dcraft").to_string_lossy().to_string();
    let mut maker = Session::new(Workspace::unrestricted());
    build(&mut maker, 1.0);
    maker
        .execute("file.save", &json!({ "path": path }))
        .unwrap();
    let mut session = Session::new(Workspace::unrestricted());
    session.set_stepwise(true);
    session
        .execute("file.open", &json!({ "path": path }))
        .unwrap();
    let mut regenerating = Vec::new();
    while let Some(p) = session.prepare_some(SceneQuality::display(), || true) {
        if p.phase == "regenerating" {
            regenerating.push((p.step, p.steps, p.name.clone().unwrap_or_default()));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        regenerating,
        [
            (1, 5, "B".to_string()),
            (2, 5, "C".to_string()),
            (3, 5, "Pin".to_string()),
            (4, 5, "Holed".to_string()),
        ]
    );
    // Nothing left, and a session that does it all at once agrees.
    assert!(
        session
            .prepare_some(SceneQuality::display(), || true)
            .is_none()
    );
    assert_eq!(picture(&mut session), picture(&mut maker));
}

/// Without a stop that says so, it is all done in one call.
#[test]
fn a_stop_that_never_stops_does_it_all_in_one_call() {
    let mut session = Session::new(Workspace::none());
    session.set_stepwise(true);
    build(&mut session, 1.0);
    session
        .execute("variable.set", &json!({"name": "w", "expr": "50 mm"}))
        .unwrap();
    assert!(
        session
            .prepare_some(SceneQuality::display(), || false)
            .is_none()
    );
    assert!(
        session
            .assertion_states()
            .iter()
            .all(|a| format!("{:?}", a.status) == "Passed")
    );
}

/// Commands between the pieces of a regeneration: a parameter set and
/// undone, the document saved and exported, then the rest a piece at a
/// time. The features (names, statuses, bodies) and the picture are what
/// a session that opens the same file and does it all at once has; also
/// with the rollback marker on a feature in the middle.
#[test]
fn commands_between_the_pieces_change_nothing_that_is_built() {
    let dir = std::env::temp_dir().join(format!("craft3d-stepwise-cmds-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (variant, marker) in [("plain", None), ("rolled", Some("Pin"))] {
        let path = dir
            .join(format!("{variant}.3dcraft"))
            .to_string_lossy()
            .to_string();
        let again = dir
            .join(format!("{variant}-again.3dcraft"))
            .to_string_lossy()
            .to_string();
        let stl = dir
            .join(format!("{variant}.stl"))
            .to_string_lossy()
            .to_string();
        let mut maker = Session::new(Workspace::unrestricted());
        build(&mut maker, 1.0);
        maker
            .execute(
                "variable.publish",
                &json!({"name": "w", "min": "10 mm", "max": "90 mm"}),
            )
            .unwrap();
        if let Some(feature) = marker {
            maker
                .execute("history.rollback", &json!({ "feature": feature }))
                .unwrap();
        }
        maker
            .execute("file.save", &json!({ "path": path }))
            .unwrap();

        let mut session = Session::new(Workspace::unrestricted());
        session.set_stepwise(true);
        session
            .execute("file.open", &json!({ "path": path }))
            .unwrap();
        let piece = |session: &mut Session| {
            session
                .prepare_some(SceneQuality::display(), || true)
                .map(|p| p.phase)
        };
        assert_eq!(piece(&mut session), Some("regenerating"), "{variant}");
        session
            .execute("param.set", &json!({"values": {"w": 60}}))
            .unwrap();
        let _ = piece(&mut session);
        session.execute("edit.undo", &json!({})).unwrap();
        let _ = piece(&mut session);
        session
            .execute("file.save", &json!({ "path": again }))
            .unwrap();
        let _ = piece(&mut session);
        session
            .execute("file.export", &json!({ "path": stl }))
            .unwrap();
        let mut calls = 0;
        while piece(&mut session).is_some() {
            calls += 1;
            assert!(calls < 100, "{variant}: no progress");
        }

        let mut fresh = Session::new(Workspace::unrestricted());
        fresh
            .execute("file.open", &json!({ "path": path }))
            .unwrap();
        let features = |s: &mut Session| {
            s.execute("document.inspect", &json!({})).unwrap()["features"].clone()
        };
        let got = features(&mut session);
        assert_eq!(got, features(&mut fresh), "{variant}");
        if marker.is_some() {
            assert!(got.to_string().contains("rolledBack"), "{variant}: {got}");
        }
        assert_eq!(picture(&mut session), picture(&mut fresh), "{variant}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
