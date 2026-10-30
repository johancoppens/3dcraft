//! Commands that need the bodies, on a host that has the document
//! regenerated elsewhere and not here: handed to the other side
//! (`run_command`) instead of regenerating on the host's thread, and
//! answered there with what a session that regenerates itself answers
//! (docs/design/engine.md › *Commands where the bodies are*).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{
    Answered, SceneQuality, SectionSpec, Session, Workspace, measure_checks, run_command,
    scene_begin, scene_next,
};
use serde_json::{Value, json};

fn build(session: &mut Session, s: f64) {
    let mm = |v: f64| format!("{} mm", v * s);
    for (id, params) in [
        ("file.new", json!({"units": "mm"})),
        ("variable.set", json!({"name": "w", "expr": mm(40.0)})),
        (
            "feature.block",
            json!({"size": ["w", mm(20.0), mm(5.0)], "name": "A"}),
        ),
        (
            "feature.block",
            json!({"size": [mm(8.0), mm(8.0), mm(8.0)], "position": [mm(60.0), "0 mm", "0 mm"], "name": "B"}),
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
            "datum.plane",
            json!({"from": "xy", "distance": mm(2.0), "name": "Low"}),
        ),
        (
            "feature.sphere",
            json!({"radius": mm(6.0), "position": [mm(0.0), mm(50.0), mm(0.0)], "name": "Ball"}),
        ),
        (
            "datum.plane",
            json!({"from": "xy", "distance": mm(3.0), "name": "Top"}),
        ),
        // Last, a parameter set as the form sets it: on the page the state is
        // then not regenerated there (an edit that needs the bodies runs
        // on the page, regenerating the state before it).
        (
            "variable.publish",
            json!({"name": "w", "min": mm(1.0), "max": mm(1000.0)}),
        ),
        ("param.set", json!({"values": {"w": mm(41.0)}})),
    ] {
        session
            .execute(id, &params)
            .unwrap_or_else(|e| panic!("{id}: {}", e.message()));
    }
}

/// A host as the page is: regenerated, measured and now also running
/// the commands that need the bodies elsewhere.
fn page(s: f64) -> Session {
    let mut session = Session::new(Workspace::memory());
    session.set_stepwise(true);
    session.set_checks_elsewhere(true);
    session.set_regen_elsewhere(true);
    session.set_commands_elsewhere(true);
    build(&mut session, s);
    settle(&mut session);
    assert!(!session.regenerated_here());
    session
}

fn desktop(s: f64) -> Session {
    let mut session = Session::new(Workspace::memory());
    build(&mut session, s);
    session
}

/// Every job of scenes and checks answered by the other side.
fn settle(session: &mut Session) {
    for _ in 0..100 {
        let scenes = session.scene_job(SceneQuality::display());
        if let Some(token) = scenes {
            let request = session.scene_request(token).unwrap();
            scene_begin(&request.document, &request.work);
            while let Some(part) = scene_next() {
                session.scene_answered(token, Ok(&part));
            }
            session.scene_done(token);
        }
        let checks = session.checks_job();
        if let Some(token) = checks {
            let request = session.checks_request(token).unwrap();
            let answer = measure_checks(&request.document, &request.work);
            session.checks_answered(token, Ok(&answer));
        }
        if scenes.is_none() && checks.is_none() {
            return;
        }
    }
    panic!("jobs without end");
}

/// Every command handed over, run on the other side and answered.
fn answer_commands(session: &mut Session) -> Vec<Answered> {
    let mut answered = Vec::new();
    for token in session.command_jobs() {
        let request = session.command_request(token).unwrap();
        let answer = run_command(&request.document, &request.work);
        answered.extend(session.command_answered(token, Ok(answer)));
    }
    assert!(!session.commands_waiting());
    answered
}

/// `id` on the page, its answer once the other side ran it.
fn on_page(session: &mut Session, id: &str, params: &Value) -> (Result<Value, String>, bool) {
    let now = session
        .execute(id, params)
        .map_err(|e| e.message().to_string());
    let Ok(result) = &now else {
        return (now, false);
    };
    if result.get("deferred") != Some(&Value::Bool(true)) {
        return (now, false);
    }
    let token = result["job"].as_u64().unwrap();
    let answered = answer_commands(session);
    assert_eq!(answered.len(), 1, "{id}");
    assert_eq!(answered[0].token, token);
    assert_eq!(answered[0].command, id);
    (answered[0].result.clone(), true)
}

#[test]
fn commands_that_need_the_bodies_run_where_they_are_with_the_same_answers() {
    for s in [1e-2, 1.0, 1e3] {
        let mut page = page(s);
        let mut desktop = desktop(s);
        for (id, params, deferred) in [
            // A click that selects.
            ("select.set", json!({"names": ["B"]}), true),
            (
                "select.query",
                json!({"where": {"kind": "face", "in": "A", "normal": {"along": "z"}}, "then": "add"}),
                true,
            ),
            ("select.get", json!({}), true),
            ("measure.distance", json!({"from": "A", "to": "B"}), true),
            ("measure.thickness", json!({"of": "B"}), true),
            ("measure.below", json!({"of": "A", "plane": "Top"}), true),
            ("file.export", json!({"path": "part.stl"}), true),
            ("file.export", json!({"path": "part.step"}), true),
            // Neither needs the bodies: done here at once.
            ("view.scrub", json!({"step": 2}), false),
            ("view.scrub", json!({"feature": null}), false),
            // A wrong one: the same error.
            (
                "measure.distance",
                json!({"from": "A", "to": "Nowhere"}),
                true,
            ),
        ] {
            let want = desktop
                .execute(id, &params)
                .map_err(|e| e.message().to_string());
            let (got, handed) = on_page(&mut page, id, &params);
            assert_eq!(got, want, "{id} {params} at {s}");
            assert_eq!(handed, deferred, "{id} handed over at {s}");
            assert!(
                !page.regenerated_here(),
                "{id} regenerated on the page at {s}"
            );
            assert_eq!(
                page.selection().items,
                desktop.selection().items,
                "{id} at {s}"
            );
            for name in desktop.file_names() {
                assert_eq!(
                    page.take_file(&name),
                    desktop.take_file(&name),
                    "{name} at {s}"
                );
            }
            assert!(page.file_names().is_empty());
        }
    }
}

/// Clicks that add, in quick succession: the second is handed over
/// before the first is answered, and runs on what the first left, as it
/// does on the desktop.
#[test]
fn clicks_in_quick_succession_add_up() {
    let mut page = page(1.0);
    let mut desktop = desktop(1.0);
    let clicks = [
        json!({"names": ["A"]}),
        json!({"names": ["B"], "mode": "add"}),
        json!({"names": ["Ball"], "mode": "add"}),
    ];
    for click in &clicks {
        desktop.execute("select.set", click).unwrap();
        let now = page.execute("select.set", click).unwrap();
        assert_eq!(now["deferred"], true);
    }
    let answered = answer_commands(&mut page);
    assert_eq!(answered.len(), 3);
    assert!(answered.iter().all(|a| a.result.is_ok()), "{answered:?}");
    assert_eq!(page.selection().items, desktop.selection().items);
    assert_eq!(page.selection().items.len(), 3);
    assert!(!page.regenerated_here());
}

/// A section on a datum plane while the view is scrubbed lies on the
/// plane of the document as it is, found without regenerating on the
/// page: from the scene of that state, kept while the view is scrubbed,
/// or built beside the page first when it is not.
#[test]
fn a_section_on_a_datum_plane_while_scrubbed_never_regenerates_on_the_page() {
    let spec = SectionSpec {
        plane: Some("Top".to_string()),
        offset: 0.5,
        ..SectionSpec::default()
    };
    let mut desktop = desktop(1.0);
    desktop.execute("view.scrub", &json!({"step": 2})).unwrap();
    let scene = desktop
        .scene_with(SceneQuality::display())
        .unwrap()
        .unwrap();
    let want = desktop.section_plane(&scene, &spec).unwrap();
    // Scrubbed after the scene of the document as it is was shown.
    let mut page = page(1.0);
    page.execute("view.scrub", &json!({"step": 2})).unwrap();
    settle(&mut page);
    let scene = page.scene_with(SceneQuality::display()).unwrap().unwrap();
    assert_eq!(page.section_plane(&scene, &spec).unwrap(), want);
    assert!(!page.regenerated_here());
    // Scrubbed with none: the first picture of the document as it is is
    // built first, then the scrubbed state's.
    let mut page = Session::new(Workspace::memory());
    page.set_stepwise(true);
    page.set_checks_elsewhere(true);
    page.set_regen_elsewhere(true);
    page.set_commands_elsewhere(true);
    build(&mut page, 1.0);
    page.execute("view.scrub", &json!({"step": 2})).unwrap();
    settle(&mut page);
    let scene = page.scene_with(SceneQuality::display()).unwrap().unwrap();
    assert_eq!(page.section_plane(&scene, &spec).unwrap(), want);
    assert!(!page.regenerated_here());
}

/// An answer for a state the document left (an edit came between) is an
/// error that says so, and nothing of it is taken; a worker that stops
/// leaves every waiting command an error.
#[test]
fn a_late_answer_is_not_taken_and_a_stopped_worker_leaves_errors() {
    let mut page = page(1.0);
    let done = page
        .execute("select.set", &json!({"names": ["B"]}))
        .unwrap();
    assert_eq!(done["deferred"], true);
    page.execute("variable.set", &json!({"name": "w", "expr": "42 mm"}))
        .unwrap();
    let answered = answer_commands(&mut page);
    let error = answered[0].result.clone().unwrap_err();
    assert!(error.contains("the document changed"), "{error}");
    assert!(page.selection().items.is_empty());
    settle(&mut page);
    page.execute("measure.thickness", &json!({"of": "B"}))
        .unwrap();
    page.execute("measure.thickness", &json!({"of": "Holed"}))
        .unwrap();
    let failed = page.commands_failed("the worker stopped");
    assert_eq!(failed.len(), 2);
    assert!(failed.iter().all(|a| a.result.is_err()));
    assert!(!page.commands_waiting());
    // Hostile answers and requests: errors, never a panic.
    assert!(
        page.command_answered(12345, Ok(Default::default()))
            .is_none()
    );
    assert!(page.command_request(12345).is_err());
    for work in ["", "{}", "{\"command\": 3}", "{\"command\": \"no.such\"}"] {
        let answer = run_command(b"not a document", work);
        assert!(answer.answer.contains("error"), "{work}: {}", answer.answer);
    }
    let token = page
        .execute("measure.thickness", &json!({"of": "B"}))
        .unwrap()["job"]
        .as_u64()
        .unwrap();
    let garbled = craft3d_engine::CommandAnswer {
        answer: "not json".into(),
        files: Vec::new(),
    };
    let answered = page.command_answered(token, Ok(garbled)).unwrap();
    assert!(answered.result.is_err());
}

/// An edit is never handed over: it is the document's, and runs here
/// (one that needs the bodies has its trial undone whole first).
#[test]
fn an_edit_is_never_handed_over() {
    let mut page = page(1.0);
    for (id, params) in [
        (
            "variable.measure",
            json!({"of": "B", "name": "VB", "measure": "volume"}),
        ),
        (
            "assert.add",
            json!({"name": "cube", "measure": "volume", "of": "B", "eq": "512 * (1 mm)^3", "tol": "1e-6 * (1 mm)^3"}),
        ),
        ("feature.block", json!({"size": [1, 1, 1], "name": "C"})),
    ] {
        let revision = page.document_revision();
        let got = page.execute(id, &params).unwrap();
        assert!(got.get("job").is_none(), "{id}: {got}");
        assert_ne!(page.document_revision(), revision, "{id}");
        assert!(!page.commands_waiting());
    }
}
