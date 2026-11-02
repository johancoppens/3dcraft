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
        .execute_deferrable(id, params)
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
        let now = page.execute_deferrable("select.set", click).unwrap();
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
        .execute_deferrable("select.set", &json!({"names": ["B"]}))
        .unwrap();
    assert_eq!(done["deferred"], true);
    page.execute("param.set", &json!({"values": {"w": "42 mm"}}))
        .unwrap();
    let answered = answer_commands(&mut page);
    let error = answered[0].result.clone().unwrap_err();
    assert!(error.contains("the document changed"), "{error}");
    assert!(page.selection().items.is_empty());
    settle(&mut page);
    page.execute_deferrable("measure.thickness", &json!({"of": "B"}))
        .unwrap();
    page.execute_deferrable("measure.thickness", &json!({"of": "Holed"}))
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
        .execute_deferrable("measure.thickness", &json!({"of": "B"}))
        .unwrap()["job"]
        .as_u64()
        .unwrap();
    let garbled = craft3d_engine::CommandAnswer {
        answer: "not json".into(),
        files: Vec::new(),
        documents: Vec::new(),
    };
    let answered = page.command_answered(token, Ok(garbled)).unwrap();
    assert!(answered.result.is_err());
}

/// A page that regenerates itself, a piece at a time (no worker): what
/// an edit's result is compared with (its summary says `deferred` where
/// the desktop's says `failed`).
fn alone(s: f64) -> Session {
    let mut session = Session::new(Workspace::memory());
    session.set_stepwise(true);
    build(&mut session, s);
    session
}

/// The name of the face of `body` whose outward normal points along
/// `axis`, as the desktop names it.
fn face(desktop: &mut Session, body: &str, axis: &str) -> String {
    let found = desktop
        .execute(
            "select.query",
            &json!({"kind": "face", "where": {"in": body, "normal": {"along": axis}}}),
        )
        .unwrap();
    found["items"][0]["name"].as_str().unwrap().to_string()
}

/// The undo label of the active document.
fn undo_label(session: &Session) -> Option<String> {
    session.document_status().and_then(|status| status.undo)
}

/// An edit that needs the bodies (a measured variable, an assertion, a
/// fillet on edges named by their faces) is made where they are and
/// applied here from its answer: the same document as the desktop's, one
/// undo step with the desktop's label (undone and redone as any), the
/// result a page that regenerates itself gives, and the page never
/// regenerating. One that needs none runs here at once; one that fails
/// there fails here with the same error and changes nothing.
#[test]
fn an_edit_that_needs_the_bodies_is_made_where_they_are() {
    for s in [1e-2, 1.0, 1e3] {
        let mut page = page(s);
        let mut desktop = desktop(s);
        let mut alone = alone(s);
        let work = page.work_here();
        let top = face(&mut desktop, "B", "z");
        let side = face(&mut desktop, "B", "x");
        let cube = 512.0 * s * s * s;
        for (id, params, handed) in [
            (
                "variable.measure",
                json!({"of": "B", "name": "VB", "measure": "volume"}),
                true,
            ),
            (
                "assert.add",
                json!({"name": "cube", "measure": "volume", "of": "B", "eq": format!("{cube} * (1 mm)^3"), "tol": format!("{} * (1 mm)^3", cube * 1e-6)}),
                true,
            ),
            (
                "feature.fillet",
                json!({"edges": [[top, side]], "radius": format!("{} mm", s), "name": "Round"}),
                true,
            ),
            // A wrong one: the same error, nothing changed.
            (
                "feature.fillet",
                json!({"edges": [["B:nowhere", side]], "radius": format!("{} mm", s)}),
                true,
            ),
            (
                "feature.block",
                json!({"size": [1, 1, 1], "name": "C"}),
                false,
            ),
        ] {
            let want = alone
                .execute(id, &params)
                .map_err(|e| e.message().to_string());
            let _ = desktop.execute(id, &params);
            let (got, deferred) = on_page(&mut page, id, &params);
            assert_eq!(deferred, handed, "{id} handed over at {s}");
            assert_eq!(got, want, "{id} {params} at {s}");
            assert_eq!(
                page.document_bytes(),
                desktop.document_bytes(),
                "{id} at {s}"
            );
            assert_eq!(undo_label(&page), undo_label(&desktop), "{id} at {s}");
            assert_eq!(
                page.work_here(),
                work,
                "{id} regenerated on the page at {s}"
            );
            settle(&mut page);
            assert_eq!(page.work_here(), work, "{id} at {s}");
        }
        // The edits are steps of the page's own history.
        for undo in ["edit.undo", "edit.undo", "edit.redo"] {
            page.execute(undo, &json!({})).unwrap();
            desktop.execute(undo, &json!({})).unwrap();
            assert_eq!(
                page.document_bytes(),
                desktop.document_bytes(),
                "{undo} at {s}"
            );
            settle(&mut page);
        }
        assert_eq!(page.work_here(), work, "at {s}");
    }
}

/// `assert.list` waits for every check: answered on the page from the
/// worker's measures of the state once they are there, handed to the
/// worker that has the bodies before; the same list either way.
#[test]
fn the_assertion_list_never_regenerates_on_the_page() {
    let mut page = page(1.0);
    let mut desktop = desktop(1.0);
    let add = json!({"name": "cube", "measure": "volume", "of": "B", "eq": "512 * (1 mm)^3", "tol": "1e-6 * (1 mm)^3"});
    desktop.execute("assert.add", &add).unwrap();
    on_page(&mut page, "assert.add", &add).0.unwrap();
    let work = page.work_here();
    let list = json!({"details": true});
    let want = desktop
        .execute("assert.list", &list)
        .map_err(|e| e.message().to_string());
    // Before the checks are measured: run where the bodies are.
    let (got, handed) = on_page(&mut page, "assert.list", &list);
    assert!(handed);
    assert_eq!(got, want);
    // After: answered here, at once.
    settle(&mut page);
    let (got, handed) = on_page(&mut page, "assert.list", &list);
    assert!(!handed);
    assert_eq!(got, want);
    assert_eq!(page.work_here(), work);
}

/// An edit whose answer finds the document changed since (an undo came
/// between) is not applied and says so; a worker that stops leaves it an
/// error, and the document as it was.
#[test]
fn an_edit_answered_late_or_never_changes_nothing() {
    let mut page = page(1.0);
    let add = json!({"name": "cube", "measure": "volume", "of": "B", "eq": "512 * (1 mm)^3", "tol": "1e-6 * (1 mm)^3"});
    let before = page.document_bytes();
    let done = page.execute_deferrable("assert.add", &add).unwrap();
    assert_eq!(done["deferred"], true);
    assert!(page.edits_waiting());
    page.execute("param.set", &json!({"values": {"w": "42 mm"}}))
        .unwrap();
    let changed = page.document_bytes();
    let answered = answer_commands(&mut page);
    let error = answered[0].result.clone().unwrap_err();
    assert!(error.contains("the document changed"), "{error}");
    assert!(!answered[0].edited);
    assert_eq!(page.document_bytes(), changed);
    assert!(!page.edits_waiting());
    page.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(page.document_bytes(), before);
    settle(&mut page);
    page.execute_deferrable("assert.add", &add).unwrap();
    let failed = page.commands_failed("the worker stopped");
    assert_eq!(failed.len(), 1);
    assert!(failed[0].result.is_err());
    assert_eq!(page.document_bytes(), before);
    // An answer that claims an edit without its steps, or with steps
    // whose documents do not read: run here instead, never a panic.
    let token = page.execute_deferrable("assert.add", &add).unwrap()["job"]
        .as_u64()
        .unwrap();
    let forged = craft3d_engine::CommandAnswer {
        answer: json!({"ok": null, "edited": true, "selection": null, "steps": [{"label": "x", "key": null, "merge": false}]}).to_string(),
        files: Vec::new(),
        documents: vec![b"not a document".to_vec()],
    };
    let answered = page.command_answered(token, Ok(forged)).unwrap();
    assert!(answered.result.is_ok(), "{:?}", answered.result);
    let mut desktop = desktop(1.0);
    desktop.execute("assert.add", &add).unwrap();
    assert_eq!(page.document_bytes(), desktop.document_bytes());
}

/// What the host asks for itself (the export dialog's choices, the count
/// of what is selected) is answered at once, never handed over: only a
/// command dispatched with `execute_deferrable` is.
#[test]
fn only_what_is_dispatched_is_handed_over() {
    let mut page = page(1.0);
    let got = page
        .execute("measure.thickness", &json!({"of": "B"}))
        .unwrap();
    assert!(got.get("deferred").is_none(), "{got}");
    assert!(!page.commands_waiting());
}

/// An export of the view as the window shows it takes the window's
/// camera with it to where the bodies are.
#[test]
fn an_export_of_the_current_view_keeps_the_window_camera() {
    for s in [1e-2, 1.0, 1e3] {
        let camera = craft3d_render::Camera {
            eye: craft3d_math::Vec3::new(30.0 * s, -80.0 * s, 25.0 * s),
            target: craft3d_math::Vec3::new(20.0 * s, 10.0 * s, 0.0),
            up: craft3d_math::Vec3::new(0.0, 0.0, 1.0),
            projection: craft3d_render::Projection::Orthographic {
                half_height: 60.0 * s,
            },
        };
        let params = json!({"path": "view.svg", "view": "current"});
        let mut desktop = desktop(s);
        desktop
            .execute(
                "param.set",
                &json!({"values": {"w": format!("{} mm", 41.0 * s)}}),
            )
            .ok();
        desktop.set_window_camera(Some(camera));
        let want = desktop
            .execute("file.export", &params)
            .map_err(|e| e.message().to_string());
        let mut page = page(s);
        page.set_window_camera(Some(camera));
        let (got, handed) = on_page(&mut page, "file.export", &params);
        assert!(handed, "at {s}");
        assert_eq!(got, want, "at {s}");
        assert_eq!(
            page.take_file("view.svg"),
            desktop.take_file("view.svg"),
            "at {s}"
        );
        // And the iso view is another picture: the camera mattered.
        let iso = desktop
            .execute("file.export", &json!({"path": "iso.svg", "view": "iso"}))
            .map(|_| desktop.take_file("iso.svg"));
        if let Ok(Some(iso)) = iso {
            assert_ne!(Some(iso), page.take_file("view.svg"));
        }
    }
}

/// Clicks in two tabs: the second tab's click does not follow the first
/// tab's, though both read an empty selection.
#[test]
fn a_click_in_another_tab_does_not_follow_the_first_tabs() {
    let mut page = page(1.0);
    page.execute_deferrable("select.set", &json!({"names": ["B"]}))
        .unwrap();
    build(&mut page, 2.0);
    settle(&mut page);
    let done = page
        .execute_deferrable("select.set", &json!({"names": ["A"], "mode": "add"}))
        .unwrap();
    assert_eq!(done["deferred"], true);
    answer_commands(&mut page);
    let names: Vec<String> = page
        .selection()
        .items
        .iter()
        .map(|i| i.name.clone())
        .collect();
    assert_eq!(names, ["A"]);
}

/// A click whose answer was refused (an edit came between), then an undo
/// back to the state it read: the next click does not follow it.
#[test]
fn a_click_after_a_refused_one_and_an_undo_does_not_follow_it() {
    let mut page = page(1.0);
    page.execute_deferrable("select.set", &json!({"names": ["B"]}))
        .unwrap();
    page.execute("param.set", &json!({"values": {"w": "42 mm"}}))
        .unwrap();
    let refused = answer_commands(&mut page);
    assert!(refused[0].result.is_err());
    page.execute("edit.undo", &json!({})).unwrap();
    settle(&mut page);
    page.execute_deferrable("select.set", &json!({"names": ["Ball"], "mode": "add"}))
        .unwrap();
    answer_commands(&mut page);
    let names: Vec<String> = page
        .selection()
        .items
        .iter()
        .map(|i| i.name.clone())
        .collect();
    assert_eq!(names, ["Ball"]);
}
