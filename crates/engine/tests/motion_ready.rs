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
use serde_json::json;
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
