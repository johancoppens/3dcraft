//! Moments answered at once while configurations are built in the
//! background (`view.motion {shapes: "ready"}`, docs/design/motion.md ›
//! *Shapes in the background*): a provisional moment shows the stale
//! configuration and settles, bit for bit, to the moment a synchronous
//! `view.motion` shows; a bake in the background plays what a bake in the
//! command plays; a baked frame shown again (its solve kept) is the same.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use common::{error, session_in, workdir};
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

/// A bar part with a published length, its body `Bar` 4 wide and 2
/// thick, and datum axes `End1` and `End2` 2 in from its ends.
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

/// A crank turned by `360 deg * t` about a fixed ground bar, and a bar
/// held to the world whose length is `30 mm + 20 mm * t`: a configuration
/// of its own at every moment.
fn stretching(s: &mut Session) {
    run(s, "assembly.new", json!({}));
    let bar = |name: &str, len: &str, y: f64| {
        json!({ "source": "bar.3dcraft", "name": name, "body": "Bar",
                "params": { "len": len }, "position": [0, y, 0] })
    };
    run(s, "instance.add", bar("Ground", "64 mm", 0.0));
    run(s, "instance.add", bar("Crank", "24 mm", 0.0));
    run(s, "instance.add", bar("Belt", "30 mm + 20 mm * t", 20.0));
    run(s, "variable.set", json!({ "name": "t", "expr": "0" }));
    for (name, kind, a, b) in [
        ("Base", "fixed", "world/z", "Ground/End1"),
        ("Drive", "revolute", "Ground/End1", "Crank/End1"),
        ("Hold", "fixed", "world/z", "Belt/End1"),
    ] {
        run(
            s,
            "mate.add",
            json!({ "kind": kind, "a": a, "b": b, "name": name, "flip": true }),
        );
    }
    run(
        s,
        "variable.set",
        json!({ "name": "Drive_angle", "expr": "360 deg * t" }),
    );
}

fn assembly_session(name: &str) -> Session {
    let dir = workdir("motion-ready", name);
    let mut s = session_in(&dir);
    bar_part(&mut s);
    stretching(&mut s);
    // As the window's viewport: provisional moments are shown, and settle
    // by polling.
    s.set_motion_live(true);
    s
}

/// What the view shows now: per body, its name, its triangles and every
/// bit of its mesh's points.
fn shown(s: &mut Session) -> Vec<(String, usize, u64)> {
    let scene = s.scene(None).unwrap().unwrap();
    scene
        .bodies
        .iter()
        .map(|body| {
            let mut digest = 0xcbf2_9ce4_8422_2325u64;
            for p in &body.mesh.positions {
                for v in [p.x, p.y, p.z] {
                    digest = (digest ^ v.to_bits()).wrapping_mul(0x100_0000_01b3);
                }
            }
            (body.name.clone(), body.mesh.positions.len(), digest)
        })
        .collect()
}

/// Polls until the moment shown is exact (as a viewer does every frame).
fn settle(s: &mut Session) {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        s.poll_motion();
        if s.motion().unwrap().exact {
            return;
        }
        assert!(Instant::now() < deadline, "the moment never settled");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn a_provisional_moment_shows_the_stale_shape_and_settles_to_the_exact_one() {
    let mut exact = assembly_session("exact");
    let answer = run(&mut exact, "view.motion", json!({ "t": 0.5 }));
    assert_eq!(answer["exact"], true, "{answer}");
    assert_eq!(answer["stale"], json!([]), "{answer}");
    let wanted = shown(&mut exact);
    let rest = {
        let mut s = assembly_session("rest");
        run(&mut s, "view.motion", json!({ "t": 0 }));
        shown(&mut s)
    };

    let mut s = assembly_session("ready");
    let answer = run(
        &mut s,
        "view.motion",
        json!({ "t": 0.5, "shapes": "ready" }),
    );
    assert_eq!(answer["t"], 0.5, "{answer}");
    if craft3d_math::available_threads() >= 2 {
        // Nothing of the belt at 0.5 is built yet: shown as at rest (its
        // configuration at t = 0), its own being built.
        assert_eq!(answer["exact"], false, "{answer}");
        assert_eq!(answer["stale"], json!(["Belt"]), "{answer}");
        assert!(answer["building"].as_u64().unwrap() >= 1, "{answer}");
        let provisional = shown(&mut s);
        let belt = |bodies: &[(String, usize, u64)]| {
            bodies.iter().find(|b| b.0 == "Belt").cloned().unwrap()
        };
        assert_eq!(belt(&provisional), belt(&rest));
        assert_ne!(belt(&provisional), belt(&wanted));
    }
    settle(&mut s);
    assert_eq!(shown(&mut s), wanted);
    let answer = run(&mut s, "view.motion", json!({}));
    assert_eq!(answer["exact"], true, "{answer}");
    assert_eq!(answer["stale"], json!([]), "{answer}");
    // Asked exactly again, nothing changes.
    let answer = run(&mut s, "view.motion", json!({ "shapes": "exact" }));
    assert_eq!(answer["exact"], true, "{answer}");
    assert_eq!(shown(&mut s), wanted);
}

#[test]
fn a_play_with_ready_shapes_settles_to_the_play_of_exact_moments() {
    const FRAMES: u32 = 8;
    let mut exact = assembly_session("play-exact");
    let mut wanted = Vec::new();
    for k in 0..=FRAMES {
        run(
            &mut exact,
            "view.motion",
            json!({ "t": f64::from(k) / f64::from(FRAMES) }),
        );
        wanted.push(shown(&mut exact));
    }
    let mut s = assembly_session("play-ready");
    let mut got = Vec::new();
    for k in 0..=FRAMES {
        let t = f64::from(k) / f64::from(FRAMES);
        let answer = run(&mut s, "view.motion", json!({ "t": t, "shapes": "ready" }));
        assert_eq!(answer["t"], t, "{answer}");
        // Never an empty picture: every instance is shown, provisional or
        // not.
        assert_eq!(shown(&mut s).len(), 3);
        settle(&mut s);
        got.push(shown(&mut s));
    }
    assert_eq!(got, wanted);
}

#[test]
fn a_bake_in_the_background_plays_what_a_bake_in_the_command_plays() {
    const FRAMES: u32 = 8;
    let mut baked = assembly_session("bake-here");
    let made = run(&mut baked, "motion.bake", json!({ "frames": FRAMES }));
    assert_eq!(made["done"], true, "{made}");
    let mut wanted = Vec::new();
    for pass in 0..2 {
        let mut frames = Vec::new();
        for k in 0..=FRAMES {
            let t = f64::from(k) / f64::from(FRAMES);
            let answer = run(&mut baked, "view.motion", json!({ "t": t }));
            assert_eq!(answer["baked"], FRAMES, "{answer}");
            frames.push(shown(&mut baked));
        }
        // The second pass shows the frames from their kept solves: the
        // same placement.
        if pass == 0 {
            wanted = frames;
        } else {
            assert_eq!(frames, wanted);
        }
    }

    let mut s = assembly_session("bake-behind");
    let queued = run(
        &mut s,
        "motion.bake",
        json!({ "frames": FRAMES, "background": true }),
    );
    if craft3d_math::available_threads() >= 2 {
        assert_eq!(queued["background"], true, "{queued}");
        assert_eq!(queued["queued"], FRAMES + 1, "{queued}");
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            s.poll_motion();
            let answer = run(&mut s, "view.motion", json!({ "t": 0, "shapes": "ready" }));
            if answer["building"] == 0 {
                break;
            }
            assert!(Instant::now() < deadline, "{answer}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    let mut got = Vec::new();
    for k in 0..=FRAMES {
        let t = f64::from(k) / f64::from(FRAMES);
        let answer = run(&mut s, "view.motion", json!({ "t": t, "shapes": "ready" }));
        assert_eq!(answer["baked"], FRAMES, "{answer}");
        // Everything is built: nothing stale, at most the grid to solve.
        assert_eq!(answer["stale"], json!([]), "{answer}");
        settle(&mut s);
        got.push(shown(&mut s));
    }
    assert_eq!(got, wanted);
}

/// The picture `view.render` draws in the session of
/// [`assembly_session`]`(session)`, as bytes.
fn rendered(s: &mut Session, session: &str) -> Vec<u8> {
    run(
        s,
        "view.render",
        json!({ "path": "moment.png", "width": 96, "height": 64 }),
    );
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("motion-ready-{session}"));
    std::fs::read(dir.join("moment.png")).unwrap()
}

/// What an agent, a test or an export sees is the exact moment, also while
/// the window's viewport shows a provisional one: a command's picture, and
/// the scene a picture of the window (a screenshot, a frame-exact
/// recording) is drawn from when the viewport is not live (an independent
/// review of 2026-11-02 found them showing the stale belt).
#[test]
fn a_picture_during_a_provisional_moment_is_the_exact_one() {
    let mut exact = assembly_session("picture-exact");
    exact.set_motion_live(false);
    run(&mut exact, "view.motion", json!({ "t": 0.5 }));
    let wanted = shown(&mut exact);
    let wanted_png = rendered(&mut exact, "picture-exact");

    let mut s = assembly_session("picture-ready");
    let answer = run(
        &mut s,
        "view.motion",
        json!({ "t": 0.5, "shapes": "ready" }),
    );
    if craft3d_math::available_threads() >= 2 {
        assert_eq!(answer["exact"], false, "{answer}");
        assert_ne!(shown(&mut s), wanted, "the viewport shows it provisionally");
    }
    // A command's picture, while the viewport would still be provisional.
    assert_eq!(rendered(&mut s, "picture-ready"), wanted_png);
    // view.motion without shapes answers the exact moment.
    let mut again = assembly_session("picture-again");
    run(
        &mut again,
        "view.motion",
        json!({ "t": 0.5, "shapes": "ready" }),
    );
    let answer = run(&mut again, "view.motion", json!({ "t": 0.5 }));
    assert_eq!(answer["exact"], true, "{answer}");
    assert_eq!(answer["stale"], json!([]), "{answer}");
    // The window's picture: the viewport not live, the scene is exact.
    let mut window = assembly_session("picture-window");
    run(
        &mut window,
        "view.motion",
        json!({ "t": 0.5, "shapes": "ready" }),
    );
    window.set_motion_live(false);
    assert_eq!(shown(&mut window), wanted);
    assert!(window.motion().unwrap().exact);
}

/// An edit or a cancel while the builder works lets go of what it was
/// building for the motion before: the moment settles to the exact one of
/// the edited assembly.
#[test]
fn an_edit_or_a_cancel_while_building_discards_the_stale_work() {
    let edit = |s: &mut Session| {
        run(
            s,
            "variable.set",
            json!({ "name": "Drive_angle", "expr": "180 deg * t" }),
        );
        run(
            s,
            "instance.edit",
            json!({ "name": "Belt", "params": { "len": "40 mm + 20 mm * t" } }),
        );
    };
    let mut exact = assembly_session("edit-exact");
    edit(&mut exact);
    let mut wanted = Vec::new();
    for t in [0.5, 0.75] {
        run(&mut exact, "view.motion", json!({ "t": t }));
        wanted.push(shown(&mut exact));
    }

    let mut s = assembly_session("edit-ready");
    // The background busy with the motion as it was: a bake and a moment.
    run(
        &mut s,
        "motion.bake",
        json!({ "frames": 16, "background": true }),
    );
    run(
        &mut s,
        "view.motion",
        json!({ "t": 0.75, "shapes": "ready" }),
    );
    edit(&mut s);
    let mut got = Vec::new();
    for t in [0.5, 0.75] {
        run(&mut s, "view.motion", json!({ "t": t, "shapes": "ready" }));
        settle(&mut s);
        got.push(shown(&mut s));
    }
    assert_eq!(got, wanted);

    // Cancelled while building: what was dropped is asked for again by
    // the next moment, which settles to the exact one.
    let mut c = assembly_session("cancel-ready");
    edit(&mut c);
    run(
        &mut c,
        "motion.bake",
        json!({ "frames": 16, "background": true }),
    );
    run(
        &mut c,
        "view.motion",
        json!({ "t": 0.75, "shapes": "ready" }),
    );
    let cancelled = run(&mut c, "motion.bake", json!({ "cancel": true }));
    assert!(cancelled.is_object(), "{cancelled}");
    run(
        &mut c,
        "view.motion",
        json!({ "t": 0.5, "shapes": "ready" }),
    );
    settle(&mut c);
    assert_eq!(shown(&mut c), wanted[0]);
}

#[test]
fn bad_shapes_and_background_are_refused() {
    let mut s = assembly_session("bad");
    let why = error(&mut s, "view.motion", json!({ "t": 0.5, "shapes": "fast" }));
    assert!(why.contains("shapes"), "{why}");
    let why = error(&mut s, "view.motion", json!({ "t": 0.5, "shapes": 3 }));
    assert!(why.contains("shapes"), "{why}");
    let why = error(&mut s, "motion.bake", json!({ "background": "yes" }));
    assert!(why.contains("background"), "{why}");
}

/// In a window (`Session::set_bake_in_background`) a trace is worked out
/// by the motion's builder: the command answers at once (pending, the
/// trace not drawn yet), so the window goes on playing (the developer's
/// field test of 2026-11-02: a play stood still while a trace of 641
/// samples built its configurations on the engine thread); once polled in,
/// its points are those the command works out itself.
#[test]
fn a_trace_in_a_window_is_worked_out_in_the_background() {
    let trace = json!({ "point": "Crank/F1:vertex(6)", "samples": 33, "name": "End" });
    let mut here = assembly_session("trace-here");
    let made = run(&mut here, "view.trace", trace.clone());
    let wanted = made["points"].clone();
    assert_eq!(wanted.as_array().unwrap().len(), 33, "{made}");

    let mut s = assembly_session("trace-behind");
    s.set_bake_in_background(true);
    let answer = run(&mut s, "view.trace", trace);
    if craft3d_math::available_threads() < 2 {
        assert_eq!(answer["points"], wanted);
        return;
    }
    assert_eq!(answer["pending"], true, "{answer}");
    assert!(answer["points"].is_null(), "{answer}");
    let deadline = Instant::now() + Duration::from_secs(120);
    while s.traces().is_empty() {
        assert!(
            s.traces_pending() || s.trace_error().is_none(),
            "{:?}",
            s.trace_error()
        );
        s.poll_motion();
        assert!(Instant::now() < deadline, "the trace never came");
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(!s.traces_pending());
    let traces = s.traces();
    let got: Vec<Value> = traces[0]
        .points
        .iter()
        .map(|p| json!([p.x, p.y, p.z]))
        .collect();
    assert_eq!(traces[0].name, "End");
    assert_eq!(Value::Array(got), wanted);
}
