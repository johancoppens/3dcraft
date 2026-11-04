//! The commands that sweep the motion or solve an assembly, made where the
//! bodies are (`commands_elsewhere.rs`, the page's Web Worker): `view.trace`
//! and `motion.check` sample the motion there, and the assembly edits and
//! reads that report what they solved (`instance.add`, `mate.add`,
//! `variable.set`, `coupling.add`, `assembly.solve`, `assembly.inspect`)
//! solve and place there. Their answers, the page's document and its
//! traces are those of a session that does it all itself, bit for bit,
//! while the page never regenerates, solves or places
//! (`Session::work_here`); a worker that fails leaves an error, never a
//! trace pending for ever.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use common::{session_in, workdir};
use craft3d_engine::{
    SceneQuality, Session, Workspace, measure_checks, run_command, scene_begin, scene_bytes,
    scene_next,
};
use serde_json::{Value, json};
use std::path::Path;

fn page_of(mut s: Session) -> Session {
    s.set_stepwise(true);
    s.set_checks_elsewhere(true);
    s.set_regen_elsewhere(true);
    s.set_commands_elsewhere(true);
    s
}

/// The worker: answers every job of scenes the page has waiting.
fn answer_scenes(page: &mut Session, quality: SceneQuality) {
    let mut jobs = 0;
    while let Some(token) = page.scene_job(quality) {
        jobs += 1;
        assert!(jobs < 100, "the page asks for jobs without end");
        let request = page.scene_request(token).unwrap();
        scene_begin(&request.document, &request.work);
        while let Some(part) = scene_next() {
            page.scene_answered(token, Ok(&part));
        }
        page.scene_done(token);
    }
}

/// The checking worker.
fn answer_checks(page: &mut Session) {
    for _ in 0..20 {
        let Some(token) = page.checks_job() else {
            return;
        };
        let request = page.checks_request(token).unwrap();
        let answer = measure_checks(&request.document, &request.work);
        page.checks_answered(token, Ok(&answer));
    }
}

fn answer_commands(page: &mut Session) -> Vec<craft3d_engine::Answered> {
    let mut answered = Vec::new();
    for token in page.command_jobs() {
        let request = page.command_request(token).unwrap();
        let answer = run_command(&request.document, &request.work);
        answered.extend(page.command_answered(token, Ok(answer)));
    }
    answered
}

fn settle(page: &mut Session) {
    answer_scenes(page, SceneQuality::display());
    answer_checks(page);
}

/// A command asked for as a person or an agent asks for it on the page:
/// its result (from the worker when it was handed over) and whether it
/// was handed over.
fn on_page(page: &mut Session, id: &str, params: &Value) -> (Result<Value, String>, bool) {
    match page.execute_deferrable(id, params) {
        Ok(result) if result["deferred"] == true => {
            let answered = answer_commands(page);
            assert_eq!(answered.len(), 1, "{id}: one answer");
            (answered[0].result.clone(), true)
        }
        other => (other.map_err(|e| e.message().to_string()), false),
    }
}

fn scenes(s: &mut Session) -> [Vec<u8>; 2] {
    [SceneQuality::Preview, SceneQuality::display()]
        .map(|q| scene_bytes(&s.scene_with(q).unwrap().unwrap()))
}

/// A wall and a slider that runs into it for part of the turn, scaled by
/// `k`, with an assertion that they do not overlap.
fn slider(s: &mut Session, k: f64) {
    let mm = |v: f64| format!("{} mm", v * k);
    run(s, "file.new", json!({"units": "mm"}));
    run(s, "variable.set", json!({"name": "t", "expr": "0"}));
    run(
        s,
        "feature.block",
        json!({"name": "Wall", "size": [mm(10.0), mm(10.0), mm(10.0)], "position": [mm(-10.0), "0 mm", "0 mm"]}),
    );
    run(
        s,
        "feature.block",
        json!({"name": "Slider", "size": [mm(10.0), mm(10.0), mm(10.0)],
               "position": [format!("{} + {} * cos(360 deg * t)", mm(15.0), mm(20.0)), "0 mm", "0 mm"]}),
    );
    run(
        s,
        "assert.add",
        json!({"name": "Apart", "measure": "overlap", "of": ["Wall", "Slider"], "is": false}),
    );
}

/// `view.trace` and `motion.check` of a part, at three scales: handed to
/// the worker, the trace drawn on the page and the check's answer are
/// those of a session that samples the motion itself, and the page
/// regenerates nothing. While the trace is made there it is pending on
/// the page, never missing.
#[test]
fn a_trace_and_a_check_over_the_motion_are_made_beside_the_page() {
    for k in [1e-2, 1.0, 1e3] {
        let mut page = page_of(Session::new(Workspace::memory()));
        slider(&mut page, k);
        let mut here = Session::new(Workspace::memory());
        slider(&mut here, k);
        settle(&mut page);
        let work = page.work_here();
        let edges = run(&mut here, "document.edges", json!({"feature": "Slider"}));
        let vertex = edges["vertices"][0]["name"].as_str().unwrap().to_string();
        let trace = json!({"point": vertex, "samples": 17, "name": "Foot", "background": false});
        let deferred = page.execute_deferrable("view.trace", &trace).unwrap();
        assert_eq!(deferred["deferred"], true, "{k}: {deferred}");
        assert_eq!(deferred["pending"], true, "{k}: {deferred}");
        assert!(page.traces_pending(), "{k}");
        let states = page.trace_states();
        assert_eq!(states.len(), 1);
        assert!(states[0].pending && states[0].error.is_none(), "{k}");
        let read = run(&mut page, "view.trace", json!({"name": "Foot"}));
        assert_eq!(read["pending"], true, "{k}: {read}");
        let answered = answer_commands(&mut page);
        assert_eq!(answered.len(), 1);
        let mine = run(&mut here, "view.trace", trace.clone());
        assert_eq!(answered[0].result.as_ref().unwrap(), &mine, "{k}");
        assert!(!page.traces_pending(), "{k}");
        assert_eq!(page.traces(), here.traces(), "{k}");
        assert_eq!(page.trace_states(), here.trace_states(), "{k}");
        assert_eq!(
            run(&mut page, "view.trace", json!({"name": "Foot"})),
            mine,
            "{k}"
        );
        // A trace of the default name, named on the page.
        let unnamed = json!({"point": vertex, "samples": 5, "t": [0.25, 0.75]});
        let (got, handed) = on_page(&mut page, "view.trace", &unnamed);
        assert!(handed);
        let mut asked = unnamed.clone();
        asked["background"] = false.into();
        assert_eq!(got.unwrap(), run(&mut here, "view.trace", asked), "{k}");
        assert_eq!(page.traces(), here.traces(), "{k}");
        // The trace exported from the page: the desktop's answer (this
        // one is a straight line, which has no flat drawing).
        let svg = json!({"path": "foot.svg", "trace": "Foot"});
        let (got, _) = on_page(&mut page, "file.export", &svg);
        let want = here
            .execute("file.export", &svg)
            .map_err(|e| e.message().to_string());
        assert_eq!(got, want, "{k}");
        // The check over the motion: the overlap from t = 0.25 on.
        for check in [
            json!({"samples": 9}),
            json!({"samples": 5, "details": true, "t": [0.2, 0.6]}),
            json!({"measure": {"measure": "clearance", "from": "Wall", "to": "Slider", "min": "1 mm"}, "samples": 7}),
        ] {
            let (got, handed) = on_page(&mut page, "motion.check", &check);
            assert!(handed, "{k} {check}");
            assert_eq!(
                got.unwrap(),
                run(&mut here, "motion.check", check.clone()),
                "{k} {check}"
            );
        }
        // A wrong point is the desktop's error, from there.
        let wrong = json!({"point": "Slider:nowhere", "background": false});
        let (got, _) = on_page(&mut page, "view.trace", &wrong);
        let want = here
            .execute("view.trace", &wrong)
            .map_err(|e| e.message().to_string());
        assert_eq!(got, want, "{k}");
        assert!(got.is_err());
        // Nothing is kept or pending for it, as on the desktop.
        assert_eq!(page.trace_states(), here.trace_states(), "{k}");
        // Requests wrong in themselves: refused alike.
        for (id, params) in [
            ("motion.check", json!({"assertions": ["Nobody"]})),
            ("motion.check", json!({"measure": {"name": "x"}})),
            ("motion.check", json!({"measure": 3})),
            ("motion.check", json!({"assertions": "Apart"})),
            (
                "motion.check",
                json!({"measure": {"measure": "nonsense"}, "samples": 3}),
            ),
        ] {
            let (got, _) = on_page(&mut page, id, &params);
            let want = here
                .execute(id, &params)
                .map_err(|e| e.message().to_string());
            assert_eq!(got, want, "{id} {params} at {k}");
        }
        assert_eq!(page.work_here(), work, "the page regenerated ({k})");
        settle(&mut page);
        assert_eq!(page.work_here(), work, "the page regenerated ({k})");
    }
}

/// A trace handed over whose worker stops, or that comes back for a
/// state the document left, is an error the page says: never pending for
/// ever.
#[test]
fn a_trace_whose_worker_fails_is_an_error_not_pending() {
    let mut page = page_of(Session::new(Workspace::memory()));
    slider(&mut page, 1.0);
    let mut here = Session::new(Workspace::memory());
    slider(&mut here, 1.0);
    settle(&mut page);
    let edges = run(&mut here, "document.edges", json!({"feature": "Slider"}));
    let vertex = edges["vertices"][0]["name"].as_str().unwrap().to_string();
    let deferred = page
        .execute_deferrable("view.trace", &json!({"point": vertex, "name": "A"}))
        .unwrap();
    assert_eq!(deferred["deferred"], true);
    let failed = page.commands_failed("the worker stopped");
    assert_eq!(failed.len(), 1);
    assert!(failed[0].result.is_err());
    assert!(!page.traces_pending());
    let states = page.trace_states();
    assert_eq!(states.len(), 1);
    assert!(!states[0].pending);
    assert!(
        states[0]
            .error
            .as_deref()
            .unwrap()
            .contains("the worker stopped"),
        "{states:?}"
    );
    let read = page.execute("view.trace", &json!({"name": "A"}));
    assert!(read.unwrap_err().message().contains("the worker stopped"));
    // An answer for a state the document left: an error too.
    page.execute_deferrable("view.trace", &json!({"point": vertex, "name": "B"}))
        .unwrap();
    let token = page.command_jobs()[0];
    let request = page.command_request(token).unwrap();
    let answer = run_command(&request.document, &request.work);
    run(
        &mut page,
        "variable.set",
        json!({"name": "spare", "expr": "1"}),
    );
    let answered = page.command_answered(token, Ok(answer)).unwrap();
    assert!(answered.result.is_err());
    assert!(!page.traces_pending());
    assert!(page.traces().is_empty());
    // An answer that does not read: an error, never a panic.
    page.execute_deferrable("view.trace", &json!({"point": vertex, "name": "C"}))
        .unwrap();
    let token = page.command_jobs()[0];
    page.command_request(token).unwrap();
    let forged = craft3d_engine::CommandAnswer {
        answer: json!({"ok": {"name": "C"}, "edited": false, "selection": null, "trace": {"points": ["zz"]}})
            .to_string(),
        ..Default::default()
    };
    let answered = page.command_answered(token, Ok(forged)).unwrap();
    assert!(answered.result.is_err(), "{:?}", answered.result);
    assert!(!page.traces_pending());
    assert!(page.traces().is_empty());
}

/// A bar part on disk with datum axes at its ends.
fn bar_part(s: &mut Session) {
    run(s, "file.new", json!({}));
    run(s, "variable.set", json!({ "name": "len", "expr": "24 mm" }));
    run(
        s,
        "variable.publish",
        json!({ "name": "len", "min": "10 mm", "max": "500 mm" }),
    );
    run(
        s,
        "feature.block",
        json!({ "name": "Bar", "size": ["len", 4, 2] }),
    );
    run(
        s,
        "datum.axis",
        json!({ "name": "End1", "through": [[2, 2, 0], [2, 2, 1]] }),
    );
    run(
        s,
        "datum.axis",
        json!({ "name": "End2", "through": [["len - 2 mm", 2, 0], ["len - 2 mm", 2, 1]] }),
    );
    run(s, "file.save", json!({ "path": "bar.3dcraft" }));
    run(s, "file.close", json!({}));
}

fn bar(name: &str, len: f64, at: [f64; 2], deg: f64) -> Value {
    json!({ "source": "bar.3dcraft", "name": name, "body": "Bar",
            "params": { "len": format!("{} mm", len + 4.0) },
            "position": [at[0], at[1], 0.0], "rotation": [0, 0, deg] })
}

/// A crank-rocker four-bar of bar parts read from files, the crank turned
/// by `t`.
fn four_bar(s: &mut Session) {
    run(s, "assembly.new", json!({}));
    run(s, "instance.add", bar("Ground", 40.0, [-2.0, -2.0], 0.0));
    run(s, "instance.add", bar("Crank", 20.0, [-2.0, -2.0], 0.0));
    run(s, "instance.add", bar("Coupler", 60.0, [18.0, -2.0], 60.0));
    run(s, "instance.add", bar("Rocker", 50.0, [38.0, -2.0], 90.0));
    for (name, a, b) in [
        ("Base", "world/z", "Ground/End1"),
        ("Drive", "Ground/End1", "Crank/End1"),
        ("Elbow", "Crank/End2", "Coupler/End1"),
        ("Knee", "Coupler/End2", "Rocker/End2"),
        ("Hip", "Ground/End2", "Rocker/End1"),
    ] {
        let kind = if name == "Base" { "fixed" } else { "revolute" };
        run(
            s,
            "mate.add",
            json!({ "kind": kind, "a": a, "b": b, "name": name, "flip": true }),
        );
    }
    run(s, "variable.set", json!({ "name": "t", "expr": "0" }));
    run(
        s,
        "variable.set",
        json!({ "name": "Drive_angle", "expr": "360 deg * t" }),
    );
}

/// The four-bar made on the desktop and opened on the page from its file
/// (so the page has not placed it).
fn assembly_pair(dir: &Path) -> (Session, Session) {
    let mut here = session_in(dir);
    bar_part(&mut here);
    four_bar(&mut here);
    run(&mut here, "file.save", json!({"path": "four.3dcraftasm"}));
    let mut page = page_of(session_in(dir));
    run(&mut page, "file.open", json!({"path": "four.3dcraftasm"}));
    (page, here)
}

/// The assembly edits that report what they solved, and the reads that
/// solve: made where the assembly is placed, their answers, the page's
/// document and its undo steps those of a session that solves itself,
/// and the page never places; its scenes and outline after each the
/// desktop's.
#[test]
fn assembly_edits_that_solve_are_made_beside_the_page() {
    let dir = workdir("sweeps-elsewhere", "four-bar-edits");
    let (mut page, mut here) = assembly_pair(&dir);
    settle(&mut page);
    let work = page.work_here();
    for (id, params) in [
        ("assembly.solve", json!({})),
        ("assembly.inspect", json!({"measures": true})),
        ("instance.add", bar("Tail", 30.0, [60.0, 30.0], 0.0)),
        (
            "mate.add",
            json!({"kind": "revolute", "a": "Rocker/End2", "b": "Tail/End1", "name": "Tip", "flip": true}),
        ),
        (
            "variable.set",
            json!({"name": "Drive_angle", "expr": "360 deg * t + 15 deg"}),
        ),
        (
            "variable.set",
            json!({"name": "Tip_angle", "expr": "30 deg", "wait": true}),
        ),
        (
            "mate.add",
            json!({"kind": "revolute", "a": "Nobody/End1", "b": "Tail/End2"}),
        ),
        ("assembly.solve", json!({})),
        ("mate.delete", json!({"name": "Tip"})),
        ("instance.delete", json!({"name": "Tail"})),
        ("assembly.inspect", json!({})),
    ] {
        let want = here
            .execute(id, &params)
            .map_err(|e| e.message().to_string());
        let (mut got, handed) = on_page(&mut page, id, &params);
        // A delete reports nothing solved: made on the page.
        assert!(
            handed || got.is_err() || id.ends_with(".delete"),
            "{id} {params} was not handed over: {got:?}"
        );
        // An edit that does not wait for its regeneration says so where
        // the long work is done in pieces (`deferred`, as for a part:
        // `Session::regeneration_deferred`); the desktop's waits.
        if id == "variable.set"
            && params.get("wait").is_none()
            && let Ok(Value::Object(map)) = &mut got
        {
            assert_eq!(map.remove("deferred"), Some(Value::Bool(true)));
        }
        assert_eq!(got, want, "{id} {params}");
        assert_eq!(
            page.document_bytes(),
            here.document_bytes(),
            "{id} {params}"
        );
        assert_eq!(page.work_here(), work, "{id} placed on the page");
        settle(&mut page);
        assert_eq!(scenes(&mut page), scenes(&mut here), "{id} {params}");
        assert_eq!(page.assembly_outline(), here.assembly_outline(), "{id}");
        assert_eq!(page.work_here(), work, "{id} placed on the page");
    }
    // Undone and redone as on the desktop.
    for undo in ["edit.undo", "edit.undo", "edit.redo"] {
        run(&mut page, undo, json!({}));
        run(&mut here, undo, json!({}));
        assert_eq!(page.document_bytes(), here.document_bytes(), "{undo}");
        settle(&mut page);
        assert_eq!(scenes(&mut page), scenes(&mut here), "{undo}");
    }
    assert_eq!(page.work_here(), work);
}

/// A trace and a check over an assembly's motion, made beside the page.
#[test]
fn a_trace_and_a_check_of_an_assembly_are_made_beside_the_page() {
    let dir = workdir("sweeps-elsewhere", "four-bar-trace");
    let (mut page, mut here) = assembly_pair(&dir);
    run(
        &mut page,
        "assert.add",
        json!({"name": "Apart", "measure": "clearance", "from": "Crank", "to": "Rocker", "min": "1 mm"}),
    );
    run(
        &mut here,
        "assert.add",
        json!({"name": "Apart", "measure": "clearance", "from": "Crank", "to": "Rocker", "min": "1 mm"}),
    );
    settle(&mut page);
    let work = page.work_here();
    let trace =
        json!({"point": "Coupler/End2", "samples": 9, "name": "Elbow", "background": false});
    let (got, handed) = on_page(&mut page, "view.trace", &trace);
    assert!(handed);
    assert_eq!(got.unwrap(), run(&mut here, "view.trace", trace));
    assert_eq!(page.traces(), here.traces());
    let check = json!({"samples": 5});
    let (got, handed) = on_page(&mut page, "motion.check", &check);
    assert!(handed);
    assert_eq!(got.unwrap(), run(&mut here, "motion.check", check));
    // While a moment of the motion is shown too.
    run(&mut page, "view.motion", json!({"t": 0.3}));
    run(&mut here, "view.motion", json!({"t": 0.3}));
    settle(&mut page);
    let check = json!({"samples": 3, "t": [0.1, 0.2]});
    let (got, handed) = on_page(&mut page, "motion.check", &check);
    assert!(handed);
    assert_eq!(got.unwrap(), run(&mut here, "motion.check", check));
    let trace = json!({"point": "Crank/End2", "samples": 5, "name": "Tip", "background": false});
    let (got, handed) = on_page(&mut page, "view.trace", &trace);
    assert!(handed);
    assert_eq!(got.unwrap(), run(&mut here, "view.trace", trace));
    assert_eq!(page.traces(), here.traces());
    // A point named with a feature in an assembly: refused on the page at
    // once, as on the desktop; nothing pending.
    let wrong = json!({"point": "Crank/End2", "feature": "F1", "background": false});
    let (got, handed) = on_page(&mut page, "view.trace", &wrong);
    let want = here
        .execute("view.trace", &wrong)
        .map_err(|e| e.message().to_string());
    assert!(!handed, "{got:?} {want:?}");
    assert_eq!(got, want);
    assert_eq!(page.trace_states(), here.trace_states());
    assert_eq!(page.work_here(), work);
}

/// A source file changed on disk since the page read it, added again: the
/// page takes the bytes the edit read (as the desktop does), so the next
/// placement uses them.
#[test]
fn review_instance_add_of_a_changed_source_takes_its_new_bytes() {
    let dir = workdir("sweeps-elsewhere", "changed-source");
    let (mut page, mut here) = assembly_pair(&dir);
    settle(&mut page);
    // bar.3dcraft saved again, 7 high.
    let mut other = session_in(&dir);
    run(&mut other, "file.new", json!({}));
    run(
        &mut other,
        "variable.set",
        json!({ "name": "len", "expr": "24 mm" }),
    );
    run(
        &mut other,
        "variable.publish",
        json!({ "name": "len", "min": "10 mm", "max": "500 mm" }),
    );
    run(
        &mut other,
        "feature.block",
        json!({ "name": "Bar", "size": ["len", 4, 7] }),
    );
    run(
        &mut other,
        "datum.axis",
        json!({ "name": "End1", "through": [[2, 2, 0], [2, 2, 1]] }),
    );
    run(
        &mut other,
        "datum.axis",
        json!({ "name": "End2", "through": [["len - 2 mm", 2, 0], ["len - 2 mm", 2, 1]] }),
    );
    run(
        &mut other,
        "file.save",
        json!({ "path": "bar.3dcraft", "overwrite": true }),
    );
    let work = page.work_here();
    let add = bar("Tail", 30.0, [60.0, 30.0], 0.0);
    let want = here
        .execute("instance.add", &add)
        .map_err(|e| e.message().to_string());
    let (got, handed) = on_page(&mut page, "instance.add", &add);
    assert!(handed);
    assert_eq!(got, want);
    assert_eq!(page.document_bytes(), here.document_bytes());
    settle(&mut page);
    assert_eq!(scenes(&mut page), scenes(&mut here));
    assert_eq!(page.assembly_outline(), here.assembly_outline());
    assert_eq!(page.work_here(), work);
}

/// A new source file in a folder of its own, added on the page: it goes
/// with the edit to the worker, and is the page's afterwards, so the next
/// placement there has it.
#[test]
fn an_instance_of_a_new_source_in_a_folder_is_added_beside_the_page() {
    let dir = workdir("sweeps-elsewhere", "new-source");
    let (mut page, mut here) = assembly_pair(&dir);
    settle(&mut page);
    std::fs::create_dir_all(dir.join("parts")).unwrap();
    let mut other = session_in(&dir);
    run(&mut other, "file.new", json!({}));
    run(
        &mut other,
        "feature.block",
        json!({ "name": "Rod", "size": [30, 3, 3] }),
    );
    run(
        &mut other,
        "datum.axis",
        json!({ "name": "End1", "through": [[1.5, 1.5, 0], [1.5, 1.5, 1]] }),
    );
    run(
        &mut other,
        "file.save",
        json!({ "path": "parts/rod.3dcraft" }),
    );
    let work = page.work_here();
    for (id, params) in [
        (
            "instance.add",
            json!({"source": "parts/rod.3dcraft", "name": "Rod", "position": [0, 40, 0]}),
        ),
        (
            "mate.add",
            json!({"kind": "revolute", "a": "Rocker/End2", "b": "Rod/End1", "name": "RodPin", "flip": true}),
        ),
        ("assembly.inspect", json!({})),
    ] {
        let want = here
            .execute(id, &params)
            .map_err(|e| e.message().to_string());
        let (got, handed) = on_page(&mut page, id, &params);
        assert!(handed, "{id}");
        assert_eq!(got, want, "{id}");
        assert!(got.is_ok(), "{id}: {got:?}");
        assert_eq!(page.document_bytes(), here.document_bytes(), "{id}");
        settle(&mut page);
        assert_eq!(scenes(&mut page), scenes(&mut here), "{id}");
        assert_eq!(page.assembly_outline(), here.assembly_outline(), "{id}");
    }
    assert_eq!(page.work_here(), work);
}

/// The published planetary gearbox (packed): solved and inspected beside
/// the page.
#[test]
fn the_planetary_gearbox_is_solved_beside_the_page() {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/examples/published/planetary-gearbox.3dcraftasm"
    ))
    .unwrap();
    let open = |s: &mut Session| {
        s.open_bytes("planetary-gearbox.3dcraftasm", &bytes)
            .unwrap();
    };
    let mut page = page_of(Session::new(Workspace::memory()));
    open(&mut page);
    let mut here = Session::new(Workspace::memory());
    open(&mut here);
    answer_scenes(&mut page, SceneQuality::Preview);
    answer_checks(&mut page);
    let work = page.work_here();
    for (id, params) in [
        ("assembly.solve", json!({})),
        ("assembly.inspect", json!({})),
    ] {
        let want = here
            .execute(id, &params)
            .map_err(|e| e.message().to_string());
        let (got, handed) = on_page(&mut page, id, &params);
        assert_eq!(got, want, "{id}");
        assert!(handed, "{id}");
    }
    assert_eq!(page.work_here(), work);
}
