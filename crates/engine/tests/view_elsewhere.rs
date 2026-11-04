//! The view made elsewhere (`view_elsewhere.rs`, the page's Web Worker):
//! the moments of a motion and an assembly (at rest and over its motion)
//! are made by the session that regenerates beside the host, from the
//! written-out request alone (the state below the view, an assembly's
//! source files, the moment), and what the host draws, lists and reads
//! is what a session that makes them itself draws, lists and reads, bit
//! for bit; while the host never regenerates, solves or places anything
//! (`Session::work_here`).

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
    scene_next, scene_side_work,
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

/// The worker: answers every job of scenes the page has waiting, part by
/// part, until none waits (a bake goes on there as parts too).
fn answer_scenes(page: &mut Session, quality: SceneQuality) -> usize {
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
    jobs
}

/// The checking worker: answers the page's jobs of checks.
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

/// The commands the page handed over, run where the bodies are.
fn answer_commands(page: &mut Session) -> Vec<craft3d_engine::Answered> {
    let mut answered = Vec::new();
    for token in page.command_jobs() {
        let request = page.command_request(token).unwrap();
        let answer = run_command(&request.document, &request.work);
        answered.extend(page.command_answered(token, Ok(answer)));
    }
    answered
}

fn scenes(s: &mut Session) -> [Vec<u8>; 2] {
    [SceneQuality::Preview, SceneQuality::display()]
        .map(|q| scene_bytes(&s.scene_with(q).unwrap().unwrap()))
}

/// A wall and a slider that runs into it for part of the turn (the
/// slider's left side at 15 + 20 cos(2π t), scaled by `k`).
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
}

/// The moments of a part's motion, with their collisions, at three scales:
/// pending on the page until the worker made them, then the scenes, the
/// collisions and what `view.motion` says are those of a session that
/// makes them itself; the page regenerates nothing.
#[test]
fn moments_of_a_part_are_made_beside_the_page_bit_for_bit() {
    for k in [1e-2, 1.0, 1e3] {
        let mut page = page_of(Session::new(Workspace::memory()));
        slider(&mut page, k);
        let mut here = Session::new(Workspace::memory());
        slider(&mut here, k);
        answer_scenes(&mut page, SceneQuality::display());
        let before = page.work_here();
        for t in [0.0, 0.25, 0.5, 0.731, 1.0] {
            let asked = run(
                &mut page,
                "view.motion",
                json!({"t": t, "collisions": true, "shapes": "ready"}),
            );
            assert_eq!(asked["pending"], true, "{k} {t}");
            assert!(page.motion().unwrap().pending);
            assert!(!page.scene_ready(SceneQuality::Preview));
            let jobs = answer_scenes(&mut page, SceneQuality::display());
            assert_eq!(jobs, 1, "{k} {t}");
            assert!(page.poll_motion(), "the moment came: {k} {t}");
            let mine = run(
                &mut here,
                "view.motion",
                json!({"t": t, "collisions": true}),
            );
            let shown = page.motion().unwrap();
            assert!(!shown.pending && shown.error.is_none());
            assert_eq!(shown.t, t);
            assert_eq!(
                Some(shown),
                here.motion().map(|mut m| {
                    m.pending = false;
                    m
                })
            );
            assert_eq!(scenes(&mut page), scenes(&mut here), "{k} {t}");
            let answered = run(&mut page, "view.motion", json!({}));
            assert_eq!(answered["pending"], false);
            assert_eq!(answered["collisions"], mine["collisions"], "{k} {t}");
            assert_eq!(answered["rebuilt"], mine["rebuilt"], "{k} {t}");
        }
        // At t = 0.5 the slider is 5 x 10 x 10 into the wall.
        run(&mut page, "view.motion", json!({"t": 0.5}));
        answer_scenes(&mut page, SceneQuality::display());
        assert_eq!(
            page.motion().unwrap().collisions,
            vec![("Wall".to_string(), "Slider".to_string())]
        );
        assert_eq!(
            page.work_here(),
            before,
            "the page made a moment itself ({k})"
        );
        // Let go of: the document as it is, its own scenes.
        run(&mut page, "view.motion", json!({"t": null}));
        assert!(page.motion().is_none());
        answer_scenes(&mut page, SceneQuality::display());
        run(&mut here, "view.motion", json!({"t": null}));
        assert_eq!(scenes(&mut page), scenes(&mut here), "{k}");
    }
}

/// A moment asked for while the worker makes another: the late answer is
/// dropped part by part, the newer one shown, and nothing of the older
/// lands on it.
#[test]
fn a_moment_superseded_is_never_shown() {
    let mut page = page_of(Session::new(Workspace::memory()));
    slider(&mut page, 1.0);
    answer_scenes(&mut page, SceneQuality::display());
    run(&mut page, "view.motion", json!({"t": 0.2}));
    let first = page.scene_job(SceneQuality::display()).unwrap();
    let request = page.scene_request(first).unwrap();
    run(&mut page, "view.motion", json!({"t": 0.4}));
    let second = page.scene_job(SceneQuality::display()).unwrap();
    assert_ne!(first, second);
    scene_begin(&request.document, &request.work);
    while let Some(part) = scene_next() {
        assert!(!page.scene_answered(first, Ok(&part)));
    }
    assert!(!page.scene_done(first));
    assert!(page.motion().unwrap().pending);
    answer_scenes(&mut page, SceneQuality::display());
    let mut here = Session::new(Workspace::memory());
    slider(&mut here, 1.0);
    run(&mut here, "view.motion", json!({"t": 0.4}));
    assert_eq!(scenes(&mut page), scenes(&mut here));
}

/// What crosses from the worker is read as hostile: a part of facts that
/// does not read ends the job with errors the page shows, never a panic,
/// and the next moment is asked for again.
#[test]
fn facts_that_do_not_read_are_errors_of_that_moment() {
    let mut page = page_of(Session::new(Workspace::memory()));
    slider(&mut page, 1.0);
    run(&mut page, "view.motion", json!({"t": 0.3}));
    let token = page.scene_job(SceneQuality::display()).unwrap();
    for bad in [
        &b"\x02F"[..],
        b"\x02F\x09\x00\x00\x00{\"x\":",
        b"\x03Fxx",
        b"\x02S",
        b"\x09F{}",
    ] {
        page.scene_answered(token, Ok(bad));
    }
    let shown = page.scene_with(SceneQuality::Preview);
    assert!(shown.is_err(), "the moment's scene says what went wrong");
    // At the same t: not pending any more, and the error said (a player
    // waiting on it would wait for ever).
    let failed = page.motion().unwrap();
    assert!(!failed.pending, "the failed moment is not waited for");
    assert!(failed.error.is_some());
    let answer = run(&mut page, "view.motion", json!({}));
    assert_eq!(answer["pending"], false);
    assert!(answer["error"].is_string(), "{answer}");
    assert!(page.scene_job(SceneQuality::display()).is_none());
    // The other ways a job ends without its facts: the worker stopped,
    // the deadline.
    for (t, end) in [(0.4, 0), (0.45, 1)] {
        run(&mut page, "view.motion", json!({"t": t}));
        let token = page.scene_job(SceneQuality::display()).unwrap();
        if end == 0 {
            page.scene_answered(token, Err("the worker stopped (out of memory)"));
        } else {
            page.scene_failed(token, "it took longer than 30 minutes");
        }
        let motion = page.motion().unwrap();
        assert!(!motion.pending && motion.error.is_some(), "{t}");
        assert!(page.scene_job(SceneQuality::display()).is_none(), "{t}");
    }
    run(&mut page, "view.motion", json!({"t": 0.6}));
    answer_scenes(&mut page, SceneQuality::display());
    assert!(page.scene_with(SceneQuality::Preview).unwrap().is_some());
}

/// A bake asked for on the page is made beside it, a piece at a time
/// after the moment shown, followed as a bake in the background is; the
/// frames are then the moments a session that bakes itself shows.
#[test]
fn a_bake_is_made_beside_the_page() {
    let mut page = page_of(Session::new(Workspace::memory()));
    slider(&mut page, 1.0);
    run(
        &mut page,
        "view.motion",
        json!({"t": 0.0, "collisions": true}),
    );
    answer_scenes(&mut page, SceneQuality::display());
    let asked = run(
        &mut page,
        "motion.bake",
        json!({"frames": 8, "collisions": true, "background": true, "limit": 1}),
    );
    assert_eq!(asked["background"], true);
    assert!(asked["building"].as_u64().unwrap() > 0);
    assert!(page.motion().unwrap().work_left > 0);
    let before = page.work_here();
    answer_scenes(&mut page, SceneQuality::display());
    let view = page.motion().unwrap();
    assert_eq!((view.work_left, view.building), (0, 0));
    assert_eq!(view.baked, Some(8));
    assert!(view.bake_error.is_none());
    let mut here = Session::new(Workspace::memory());
    slider(&mut here, 1.0);
    run(
        &mut here,
        "motion.bake",
        json!({"frames": 8, "collisions": true}),
    );
    // The worker shows the frames from what it baked: once each is
    // meshed, showing them all again meshes nothing (unbaked, a frame
    // shown after others rebuilds the slider, a new body to mesh, as the
    // unbaked t = 0.3 at the end shows).
    let mut baked = scene_side_work().unwrap();
    for pass in 0..2 {
        if pass == 1 {
            baked = scene_side_work().unwrap();
        }
        for k in 0..=8 {
            let t = f64::from(k) / 8.0;
            run(
                &mut page,
                "view.motion",
                json!({"t": t, "collisions": true}),
            );
            answer_scenes(&mut page, SceneQuality::display());
            let mine = run(
                &mut here,
                "view.motion",
                json!({"t": t, "collisions": true}),
            );
            assert_eq!(scenes(&mut page), scenes(&mut here), "{t}");
            let theirs = run(&mut page, "view.motion", json!({}));
            assert_eq!(theirs["collisions"], mine["collisions"], "{t}");
        }
    }
    let again = scene_side_work().unwrap().meshes;
    assert_eq!(again, baked.meshes, "frames meshed again");
    for t in [0.3, 0.7, 0.3] {
        run(
            &mut page,
            "view.motion",
            json!({"t": t, "collisions": true}),
        );
        answer_scenes(&mut page, SceneQuality::display());
    }
    assert!(
        scene_side_work().unwrap().meshes > again + 2,
        "unbaked moments mesh"
    );
    assert_eq!(page.work_here(), before);
    // Cancelled: nothing is asked for any more.
    run(
        &mut page,
        "motion.bake",
        json!({"frames": 16, "background": true}),
    );
    run(&mut page, "motion.bake", json!({"cancel": true}));
    assert_eq!(page.motion().unwrap().work_left, 0);
    assert!(page.scene_job(SceneQuality::display()).is_none());
}

/// A bar part on disk with datum axes at its ends (as the assembly
/// motion tests use it).
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

/// A crank-rocker four-bar of bar parts read from files (crank 20,
/// coupler 60, rocker 50, ground 40), the crank `Drive` turned by `t`,
/// and an assertion that ground and crank do not overlap (they do: the
/// bars lie in one plane, which both sides must measure alike).
fn four_bar(s: &mut Session) {
    run(s, "assembly.new", json!({}));
    let bar = |name: &str, len: f64, at: [f64; 2], deg: f64| {
        json!({ "source": "bar.3dcraft", "name": name, "body": "Bar",
                "params": { "len": format!("{} mm", len + 4.0) },
                "position": [at[0], at[1], 0.0], "rotation": [0, 0, deg] })
    };
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
    run(
        s,
        "assert.add",
        json!({"name": "Apart", "measure": "overlap", "of": ["Ground", "Crank"], "is": false}),
    );
}

fn assembly_pair(dir: &Path) -> (Session, Session) {
    let mut page = page_of(session_in(dir));
    bar_part(&mut page);
    four_bar(&mut page);
    let mut here = session_in(dir);
    four_bar(&mut here);
    (page, here)
}

/// An assembly whose sources are files is placed beside the page from
/// the files the page read (they cross with its state): at rest and over
/// its motion its scenes, its outline and the joints a drag moves are
/// those of a session that places it itself, and the page never places,
/// solves or regenerates.
#[test]
fn an_assembly_is_placed_and_played_beside_the_page() {
    let dir = workdir("view-elsewhere", "four-bar");
    let (mut page, mut here) = assembly_pair(&dir);
    assert!(page.regen_elsewhere());
    let before = page.work_here();
    answer_scenes(&mut page, SceneQuality::display());
    answer_checks(&mut page);
    assert_eq!(scenes(&mut page), scenes(&mut here));
    assert_eq!(page.assembly_outline(), here.assembly_outline());
    assert!(page.assembly_outline().unwrap().mates.len() == 5);
    for feature in ["F1", "F2", "F3", "F4"] {
        assert_eq!(
            page.assembly_joint_of(feature),
            here.assembly_joint_of(feature),
            "{feature}"
        );
    }
    let mine = run(&mut here, "assert.list", json!({}));
    assert_eq!(run(&mut page, "assert.list", json!({})), mine);
    // Measured there, not pending (the bars lie in one plane: they
    // overlap, 192 mm³, on both sides).
    assert_eq!(mine["pending"], 0, "{mine}");
    assert_eq!(mine["assertions"][0]["measured"], true, "{mine}");
    for t in [0.1, 0.35, 0.8] {
        run(
            &mut page,
            "view.motion",
            json!({"t": t, "collisions": true}),
        );
        // While the moment is made, the tree lists the last one.
        assert!(page.assembly_outline().is_some());
        answer_scenes(&mut page, SceneQuality::display());
        run(
            &mut here,
            "view.motion",
            json!({"t": t, "collisions": true}),
        );
        assert_eq!(scenes(&mut page), scenes(&mut here), "{t}");
        assert_eq!(page.assembly_outline(), here.assembly_outline(), "{t}");
        assert_eq!(
            page.assembly_joint_of("F2"),
            here.assembly_joint_of("F2"),
            "{t}"
        );
        let shown = page.motion().unwrap();
        let mut mine = here.motion().unwrap();
        mine.pending = false;
        assert_eq!(shown, mine, "{t}");
        // A read gives what it gives here, wherever it runs (the page
        // placed this assembly at rest itself while it was being made).
        let read = read_deferrable(&mut page, "measure.pose", json!({"instance": "Coupler"}));
        let pose = run(&mut here, "measure.pose", json!({"instance": "Coupler"}));
        assert_eq!(read, pose, "{t}");
    }
    // A joint set by hand (the Parameters tab's slider) at rest.
    run(&mut page, "view.motion", json!({"t": null}));
    run(&mut here, "view.motion", json!({"t": null}));
    run(&mut page, "variable.delete", json!({"name": "Drive_angle"}));
    run(&mut here, "variable.delete", json!({"name": "Drive_angle"}));
    let set = run(
        &mut page,
        "joint.set",
        json!({"mate": "Drive", "value": "30 deg"}),
    );
    assert_eq!(
        (&set["status"], &set["deferred"]),
        (&json!("pending"), &json!(true))
    );
    // Until the worker answers, the tree shows the last one
    // (provisionally), never nothing.
    assert!(page.assembly_outline().is_some());
    // The sources were handed over once: this job names them by digest,
    // a worker started again gets them in full.
    let token = page.scene_job(SceneQuality::display()).unwrap();
    let named = page.scene_request(token).unwrap().document.len();
    page.scene_worker_restarted();
    let full = page.scene_request(token).unwrap().document.len();
    assert!(named < full, "{named} {full}");
    run(
        &mut here,
        "joint.set",
        json!({"mate": "Drive", "value": "30 deg"}),
    );
    answer_scenes(&mut page, SceneQuality::display());
    assert_eq!(scenes(&mut page), scenes(&mut here));
    assert_eq!(page.assembly_outline(), here.assembly_outline());
    // The crank can be dragged now (its joint holds a value); a drag's
    // next value keeps the handle while the worker places it.
    let handle = page.assembly_joint_of("F2");
    assert!(handle.is_some());
    assert_eq!(handle, here.assembly_joint_of("F2"));
    run(
        &mut page,
        "joint.set",
        json!({"mate": "Drive", "value": "40 deg", "merge": true}),
    );
    assert_eq!(page.assembly_joint_of("F2"), handle);
    answer_scenes(&mut page, SceneQuality::display());
    assert_eq!(page.work_here(), before, "the page placed or regenerated");
}

/// A packed assembly (the published planetary gearbox) opened on the
/// page: placed beside it, and its motion played there, bit for bit.
#[test]
fn the_published_planetary_gearbox_plays_beside_the_page() {
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
    let before = page.work_here();
    let quality = SceneQuality::Preview;
    answer_scenes(&mut page, quality);
    assert_eq!(
        scene_bytes(&page.scene_with(quality).unwrap().unwrap()),
        scene_bytes(&here.scene_with(quality).unwrap().unwrap())
    );
    for t in [0.05, 0.3] {
        run(&mut page, "view.motion", json!({"t": t}));
        answer_scenes(&mut page, quality);
        run(&mut here, "view.motion", json!({"t": t}));
        assert_eq!(
            scene_bytes(&page.scene_with(quality).unwrap().unwrap()),
            scene_bytes(&here.scene_with(quality).unwrap().unwrap()),
            "{t}"
        );
        assert_eq!(page.assembly_outline(), here.assembly_outline(), "{t}");
        // Reads the page never placed for: run where it is placed.
        for (id, params) in [
            ("measure.pose", json!({"instance": "Planet1"})),
            ("measure.joint", json!({"mate": "SunOnCarrier"})),
        ] {
            let deferred = page.execute_deferrable(id, &params).unwrap();
            assert_eq!(deferred["deferred"], true, "{id} {t}");
            let answered = answer_commands(&mut page);
            assert_eq!(answered.len(), 1);
            let mine = run(&mut here, id, params);
            assert_eq!(answered[0].result.as_ref().unwrap(), &mine, "{id} {t}");
        }
    }
    assert_eq!(page.work_here(), before);
}

/// A read asked for as a person asks for it: its result, from where the
/// bodies are when it was handed over.
fn read_deferrable(page: &mut Session, id: &str, params: Value) -> Value {
    let result = page.execute_deferrable(id, &params).unwrap();
    if result["deferred"] != true {
        return result;
    }
    let answered = answer_commands(page);
    answered[0].result.clone().unwrap()
}

/// Without the worker (the page regenerates itself again), the page makes
/// the moment and places the assembly itself, as before.
#[test]
fn without_the_worker_the_page_makes_the_view_itself() {
    let mut page = page_of(Session::new(Workspace::memory()));
    slider(&mut page, 1.0);
    page.set_regen_elsewhere(false);
    let answer = run(
        &mut page,
        "view.motion",
        json!({"t": 0.5, "collisions": true}),
    );
    assert!(answer.get("pending").is_none());
    assert_eq!(answer["collisions"].as_array().unwrap().len(), 1);
    assert!(page.scene_job(SceneQuality::display()).is_none());
    let view: Value = serde_json::to_value(page.motion().unwrap().t).unwrap();
    assert_eq!(view, json!(0.5));
}
