//! Regeneration elsewhere (`Session::set_regen_elsewhere`, the page's Web
//! Worker): the host hands the jobs of its scenes over, `scene_begin` and
//! `scene_next` answer them from the written-out request alone, and the
//! scenes the host shows are those a session that builds them itself
//! shows, bit for bit, as are the assertions and outputs measured a state
//! at a time; without the host ever regenerating the state it shows.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{
    AssertionStatus, SceneQuality, SectionSpec, Session, Workspace, measure_checks, scene_begin,
    scene_bytes, scene_next,
};
use serde_json::json;

/// Blocks at scale `s` (mm), a hole, a sphere, a sketch, a datum plane, a
/// coloured body, outputs in two stages and assertions; then `w` set to
/// 41 (scaled) by `param.set`, an edit whose regeneration is left to the
/// host (as the Parameters form sets it).
fn build(session: &mut Session, s: f64) {
    let mm = |v: f64| format!("{} mm", v * s);
    let steps = [
        ("file.new", json!({"units": "mm"})),
        ("variable.set", json!({"name": "w", "expr": mm(40.0)})),
        (
            "variable.publish",
            json!({"name": "w", "min": mm(1.0), "max": mm(1000.0)}),
        ),
        (
            "feature.block",
            json!({"size": ["w", mm(20.0), mm(5.0)], "name": "A"}),
        ),
        (
            "feature.block",
            json!({"size": ["w / 4", "w / 4", "w / 4"], "position": [mm(60.0), "0 mm", "0 mm"], "name": "B"}),
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
            "feature.sphere",
            json!({"radius": mm(6.0), "position": [mm(0.0), mm(50.0), mm(0.0)], "name": "Ball"}),
        ),
        (
            "body.appearance",
            json!({"body": "Ball", "color": "#c04020"}),
        ),
        (
            "datum.plane",
            json!({"from": "xy", "distance": mm(12.0), "name": "Top"}),
        ),
        ("sketch.create", json!({"plane": "xy", "name": "S"})),
        (
            "sketch.circle",
            json!({"sketch": "S", "center": [mm(-30.0), mm(0.0)], "radius": mm(5.0)}),
        ),
        (
            "variable.measure",
            json!({"name": "VB", "measure": "volume", "of": "B", "wait": false}),
        ),
        (
            "variable.output",
            json!({"name": "VB2", "expr": "2 * VB", "wait": false}),
        ),
        (
            "assert.add",
            json!({"name": "cube", "measure": "volume", "of": "B", "eq": "(w / 4)^3", "tol": "1e-6 * (1 mm)^3"}),
        ),
        (
            "assert.add",
            json!({"name": "apart", "measure": "overlap", "is": false}),
        ),
        (
            "assert.add",
            json!({"name": "twice", "measure": "value", "expr": "VB2", "min": "(w / 4)^3"}),
        ),
        ("param.set", json!({"values": {"w": mm(41.0)}})),
    ];
    for (id, params) in steps {
        session
            .execute(id, &params)
            .unwrap_or_else(|e| panic!("{id}: {}", e.message()));
    }
}

/// `w` set as the Parameters form sets it.
fn set_w(session: &mut Session, value: &str) {
    let done = session
        .execute("param.set", &json!({"values": {"w": value}}))
        .unwrap();
    assert!(
        !session.regen_elsewhere() || done["deferred"] == true,
        "{done}"
    );
}

/// A host that regenerates elsewhere.
fn elsewhere() -> Session {
    let mut session = Session::new(Workspace::none());
    session.set_stepwise(true);
    session.set_checks_elsewhere(true);
    session.set_regen_elsewhere(true);
    assert!(session.regen_elsewhere());
    session
}

/// Answers one job of scenes as the worker does; the parts it answered.
fn answer_scenes(session: &mut Session, token: u64) -> usize {
    let request = session.scene_request(token).unwrap();
    assert_eq!(request.token, token);
    scene_begin(&request.document, &request.work);
    let mut parts = 0;
    while let Some(part) = scene_next() {
        assert!(session.scene_answered(token, Ok(&part)));
        parts += 1;
    }
    assert!(session.scene_done(token));
    // Answered: nothing more is taken for it.
    assert!(!session.scene_done(token));
    parts
}

/// Hands every job over and answers it, until none waits.
fn settle(session: &mut Session) {
    for _ in 0..100 {
        assert_eq!(
            session.prepare_some(SceneQuality::display(), || false),
            None,
            "the host does the work itself"
        );
        let scenes = session.scene_job(SceneQuality::display());
        if let Some(token) = scenes {
            // Asked again, the same job keeps its number.
            assert_eq!(session.scene_job(SceneQuality::display()), Some(token));
            assert_eq!(answer_scenes(session, token), 2);
        }
        let checks = session.checks_job();
        if let Some(token) = checks {
            assert_eq!(session.checks_job(), Some(token));
            let request = session.checks_request(token).unwrap();
            let answer = measure_checks(&request.document, &request.work);
            assert!(session.checks_answered(token, Ok(&answer)), "{answer}");
            assert!(!session.checks_answered(token, Ok(&answer)));
        }
        if scenes.is_none() && checks.is_none() {
            return;
        }
    }
    panic!("jobs without end");
}

/// What a session that does everything itself shows.
fn here(s: f64, edit: Option<f64>) -> Session {
    let mut here = Session::new(Workspace::none());
    build(&mut here, s);
    if let Some(w) = edit {
        set_w(&mut here, &format!("{} mm", w * s));
    }
    here.execute("assert.list", &json!({"wait": true})).unwrap();
    here.execute("output.list", &json!({"wait": true})).unwrap();
    here
}

/// The host shows what `want` shows: the scenes bit for bit, the
/// assertions and the outputs.
fn same(session: &mut Session, want: &mut Session, what: &str) {
    for quality in [SceneQuality::Preview, SceneQuality::display()] {
        let got = session.scene_with(quality).unwrap().unwrap();
        let wanted = want.scene_with(quality).unwrap().unwrap();
        assert!(!got.bodies.is_empty(), "{what}");
        assert_eq!(
            scene_bytes(&got),
            scene_bytes(&wanted),
            "{what} {quality:?}"
        );
        assert_eq!(got.revision, session.scene_revision(), "{what}");
    }
    let states = session.assertion_states();
    assert_eq!(states, want.assertion_states(), "{what}");
    assert!(
        states.iter().all(|a| a.status == AssertionStatus::Passed),
        "{what}: {states:?}"
    );
    assert_eq!(session.output_states(), want.output_states(), "{what}");
    // All of it without regenerating the state shown here.
    assert!(!session.regenerated_here(), "{what}");
}

#[test]
fn scenes_built_elsewhere_are_those_built_here_bit_for_bit() {
    for s in [1e-2, 1.0, 1e3] {
        let mut want = here(s, None);
        let mut session = elsewhere();
        build(&mut session, s);
        // Not there yet: pending, and the display waits for the worker.
        assert!(!session.scene_ready(SceneQuality::display()));
        assert!(
            session
                .assertion_states()
                .iter()
                .all(|a| a.status == AssertionStatus::Pending)
        );
        settle(&mut session);
        assert!(session.scene_ready(SceneQuality::display()));
        same(&mut session, &mut want, &format!("scale {s}"));

        // An edit and back: new jobs, the same answers as here.
        for w in [40.0, 41.0] {
            set_w(&mut session, &format!("{} mm", w * s));
            settle(&mut session);
            let mut want = here(s, Some(w));
            same(&mut session, &mut want, &format!("scale {s}, w {w}"));
        }
    }
}

#[test]
fn an_edit_supersedes_the_job_and_a_late_answer_never_lands() {
    let mut session = elsewhere();
    build(&mut session, 1.0);
    let old = session.scene_job(SceneQuality::display()).unwrap();
    let request = session.scene_request(old).unwrap();
    let old_checks = session.checks_job().unwrap();
    let old_checks_request = session.checks_request(old_checks).unwrap();
    set_w(&mut session, "47 mm");
    let new = session.scene_job(SceneQuality::display()).unwrap();
    assert_ne!(new, old);
    assert!(session.scene_request(old).is_err());
    // The old state's answer arrives late: dropped, part by part.
    scene_begin(&request.document, &request.work);
    while let Some(part) = scene_next() {
        assert!(!session.scene_answered(old, Ok(&part)));
    }
    assert!(!session.scene_done(old));
    assert!(!session.scene_ready(SceneQuality::display()));
    let new_checks = session.checks_job().unwrap();
    assert_ne!(new_checks, old_checks);
    let late = measure_checks(&old_checks_request.document, &old_checks_request.work);
    assert!(!session.checks_answered(old_checks, Ok(&late)));
    // The new state is answered as usual.
    settle(&mut session);
    let mut want = here(1.0, Some(47.0));
    same(&mut session, &mut want, "after the edit");
}

#[test]
fn a_worker_that_stops_leaves_errors_and_the_next_state_goes_on() {
    let mut session = elsewhere();
    build(&mut session, 1.0);
    let token = session.scene_job(SceneQuality::display()).unwrap();
    // The quick first picture came, then the worker stopped.
    let request = session.scene_request(token).unwrap();
    scene_begin(&request.document, &request.work);
    let first = scene_next().unwrap();
    assert!(session.scene_answered(token, Ok(&first)));
    assert!(session.scene_answered(token, Err("RuntimeError: unreachable")));
    // Not asked again for that state, and the picture that came stays.
    assert_eq!(session.scene_job(SceneQuality::display()), None);
    assert!(session.scene_with(SceneQuality::Preview).unwrap().is_some());
    let error = session.scene_with(SceneQuality::display()).unwrap_err();
    assert!(error.message().contains("unreachable"), "{error}");
    // A part that is no scene is an error too, never a panic.
    set_w(&mut session, "43 mm");
    let token = session.scene_job(SceneQuality::display()).unwrap();
    // Two scenes that do not read: each an error, the job goes on.
    for junk in [&b"\x00S"[..], b"\x01Sjunk"] {
        assert!(session.scene_answered(token, Ok(junk)));
    }
    assert!(session.scene_with(SceneQuality::Preview).is_err());
    assert!(session.scene_with(SceneQuality::display()).is_err());
    assert!(session.scene_done(token));
    // A part that is no part at all ends the job.
    for (w, junk) in [(45, &b""[..]), (46, b"\x09\x09")] {
        set_w(&mut session, &format!("{w} mm"));
        let token = session.scene_job(SceneQuality::display()).unwrap();
        assert!(session.scene_answered(token, Ok(junk)));
        assert!(!session.scene_answered(token, Ok(junk)));
        assert_eq!(session.scene_job(SceneQuality::display()), None);
        assert!(session.scene_with(SceneQuality::display()).is_err());
    }
    // Given up (it took too long): errors that say so, no bug label.
    set_w(&mut session, "44 mm");
    let token = session.scene_job(SceneQuality::display()).unwrap();
    assert!(session.scene_failed(token, "too long"));
    let error = session.scene_with(SceneQuality::Preview).unwrap_err();
    assert_eq!(error.message(), "not regenerated: too long");
    // The next state goes on as before.
    set_w(&mut session, "41 mm");
    settle(&mut session);
    let mut want = here(1.0, None);
    same(&mut session, &mut want, "after the stop");
}

#[test]
fn without_checks_elsewhere_the_host_regenerates_itself() {
    let mut session = Session::new(Workspace::none());
    session.set_stepwise(true);
    session.set_regen_elsewhere(true);
    assert!(!session.regen_elsewhere());
    build(&mut session, 1.0);
    assert_eq!(session.scene_job(SceneQuality::display()), None);
    let mut steps = 0;
    while session
        .prepare_some(SceneQuality::display(), || false)
        .is_some()
    {
        steps += 1;
        assert!(steps < 1000);
    }
    assert!(session.regenerated_here());
    // Off again: what is not there is built here.
    let mut session = elsewhere();
    build(&mut session, 1.0);
    session.set_regen_elsewhere(false);
    assert_eq!(session.scene_job(SceneQuality::display()), None);
    assert!(!session.regenerated_here());
    // Never stopped, one call does it all (the checks stay elsewhere).
    let _ = session.prepare_some(SceneQuality::display(), || false);
    assert!(session.regenerated_here());
    assert!(session.scene_ready(SceneQuality::display()));
}

/// What a frame reads of the state it shows (the selection, its serial,
/// a section on a datum plane) never regenerates it on the host: not for
/// a document opened beside another whose selection the window held, not
/// after an edit of a document with a selection. A command still checks
/// the selection against the bodies, regenerating as it always did.
#[test]
fn what_a_frame_reads_never_regenerates_on_the_host() {
    let mut session = elsewhere();
    build(&mut session, 1.0);
    settle(&mut session);
    // A click selects: a command, which regenerates here.
    session
        .execute("select.set", &json!({"names": ["B"]}))
        .unwrap();
    assert!(session.regenerated_here());
    assert_eq!(session.selection().items.len(), 1);
    // A second document beside it (another tab, a Jig opened).
    build(&mut session, 2.0);
    settle(&mut session);
    assert!(!session.regenerated_here());
    let serial = session.selection_serial();
    let view = session.selection();
    assert!(view.items.is_empty());
    assert_eq!(view.serial, serial);
    assert!(
        !session.regenerated_here(),
        "reading the selection of a document not regenerated here regenerated it"
    );
    // A selection in it, then an edit: the state shown is new.
    session
        .execute("select.set", &json!({"names": ["Ball"]}))
        .unwrap();
    set_w(&mut session, "83 mm");
    settle(&mut session);
    assert!(!session.regenerated_here());
    let serial = session.selection_serial();
    let view = session.selection();
    assert_eq!(view.items.len(), 1);
    assert_eq!(view.serial, serial);
    assert!(!session.regenerated_here(), "the selection regenerated it");
    // A section on the datum plane: the plane a host that regenerates
    // itself finds, bit for bit, from the scene.
    let scene = session
        .scene_with(SceneQuality::display())
        .unwrap()
        .unwrap();
    let spec = SectionSpec {
        plane: Some("Top".to_string()),
        offset: 1.5,
        ..SectionSpec::default()
    };
    let got = session.section_plane(&scene, &spec).unwrap();
    assert!(!session.regenerated_here(), "the section regenerated it");
    let mut want = here(2.0, Some(41.5));
    let wanted_scene = want.scene_with(SceneQuality::display()).unwrap().unwrap();
    let wanted = want.section_plane(&wanted_scene, &spec).unwrap();
    assert_eq!(got, wanted);
    assert_eq!(format!("{got:?}"), format!("{wanted:?}"));
    // 24 mm up, moved by the offset.
    assert!((got.origin.z - 25.5).abs() < 1e-12, "{got:?}");
    // Not a datum plane of this document: an error, still without
    // regenerating.
    let missing = SectionSpec {
        plane: Some("Nowhere".to_string()),
        ..SectionSpec::default()
    };
    assert!(session.section_plane(&scene, &missing).is_err());
    assert!(!session.regenerated_here());
    // A command checks the selection against the bodies.
    let got = session.execute("select.get", &json!({})).unwrap();
    assert_eq!(got["items"].as_array().map(Vec::len), Some(1), "{got}");
    assert!(session.regenerated_here());
}

/// A state the view is scrubbed to is built elsewhere as a host that
/// builds it itself builds it, bit for bit, and so is the end again.
#[test]
fn a_scrubbed_state_built_elsewhere_is_the_one_built_here() {
    let mut want = here(1.0, None);
    let mut session = elsewhere();
    build(&mut session, 1.0);
    settle(&mut session);
    for step in [
        json!({"step": 2}),
        json!({"step": 4}),
        json!({"step": 6}),
        json!({"feature": null}),
    ] {
        for host in [&mut session, &mut want] {
            host.execute("view.scrub", &step).unwrap();
        }
        settle(&mut session);
        for quality in [SceneQuality::Preview, SceneQuality::display()] {
            let got = session.scene_with(quality).unwrap().unwrap();
            let wanted = want.scene_with(quality).unwrap().unwrap();
            assert_eq!(
                scene_bytes(&got),
                scene_bytes(&wanted),
                "{step} {quality:?}"
            );
        }
    }
}
