//! The motion in the window (docs/design/motion.md): a trace drawn as a
//! line over the picture, colliding bodies drawn red, and the player
//! playing the timeline `t` (back and forth, loop, stop at the end), all
//! with the CPU renderer as the oracle (`ui.screenshot`).
//!
//! Each picture is also printed as `EVIDENCE <name> <base64 PNG>` for
//! whoever runs the tests with `--nocapture` and wants to look at it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::Shell;
use egui::{Pos2, vec2};
use serde_json::{Value, json};

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn call(shell: &mut Shell, session: &mut Session, method: &str, params: Value) -> Value {
    shell
        .handle_method(session, method, Some(&params))
        .unwrap()
        .unwrap_or_else(|e| panic!("{method} {params}: {e}"))
}

/// A wall and a crank whose block goes round a circle of 15 mm about the
/// z axis, into the wall near t = 0.5.
fn crank() -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({}));
    run(&mut s, "variable.set", json!({"name": "t", "expr": "0"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Wall", "size": ["4 mm", "30 mm", "6 mm"], "position": ["-18 mm", "-15 mm", "0 mm"]}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Crank", "size": ["4 mm", "4 mm", "4 mm"],
               "position": ["15 mm * cos(360 deg * t) - 2 mm", "15 mm * sin(360 deg * t) - 2 mm", "0 mm"]}),
    );
    s
}

/// The viewport's picture with its overlay, as RGB.
fn screenshot(shell: &mut Shell, session: &mut Session, name: &str) -> Vec<u8> {
    let result = call(
        shell,
        session,
        "ui.screenshot",
        json!({"width": 320, "height": 240, "scale": 1.0}),
    );
    let text = result["base64"].as_str().unwrap();
    println!("EVIDENCE {name} {text}");
    let png = base64_decode(text);
    let mut reader = png::Decoder::new(std::io::Cursor::new(&png))
        .read_info()
        .unwrap();
    let mut rgb = vec![0; reader.output_buffer_size()];
    reader.next_frame(&mut rgb).unwrap();
    rgb
}

fn base64_decode(text: &str) -> Vec<u8> {
    let value = |c: u8| -> u32 {
        match c {
            b'A'..=b'Z' => u32::from(c - b'A'),
            b'a'..=b'z' => u32::from(c - b'a') + 26,
            b'0'..=b'9' => u32::from(c - b'0') + 52,
            b'+' => 62,
            b'/' => 63,
            _ => panic!("not base64: {c}"),
        }
    };
    let bytes: Vec<u8> = text.bytes().filter(|&c| c != b'=').collect();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        let mut word = 0_u32;
        for (k, &c) in chunk.iter().enumerate() {
            word |= value(c) << (18 - 6 * k);
        }
        for k in 0..chunk.len().saturating_sub(1) {
            out.push((word >> (16 - 8 * k)) as u8);
        }
    }
    out
}

/// Pixels near `ink`.
fn count(rgb: &[u8], ink: [u8; 3], within: i32) -> usize {
    rgb.chunks(3)
        .filter(|p| (0..3).all(|k| (i32::from(p[k]) - i32::from(ink[k])).abs() <= within))
        .count()
}

/// Strongly red pixels: a shaded red body.
fn reds(rgb: &[u8]) -> usize {
    rgb.chunks(3)
        .filter(|p| {
            p[0] > 120
                && u32::from(p[1]) * 2 < u32::from(p[0])
                && u32::from(p[2]) * 2 < u32::from(p[0])
        })
        .count()
}

#[test]
fn a_trace_is_drawn_as_a_line_over_the_picture() {
    let mut session = crank();
    let mut shell = Shell::new();
    let before = screenshot(&mut shell, &mut session, "motion-no-trace");
    let trace_ink = craft3d_ui_egui::theme::Tokens::default().trace;
    assert_eq!(count(&before, trace_ink, 24), 0);
    let edges = run(&mut session, "document.edges", json!({"feature": "Crank"}));
    let top = edges["vertices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["point"][2].as_f64() == Some(4.0))
        .unwrap()["name"]
        .as_str()
        .unwrap()
        .to_string();
    run(
        &mut session,
        "view.trace",
        json!({"point": top, "samples": 65}),
    );
    let traced = screenshot(&mut shell, &mut session, "motion-trace");
    let inked = count(&traced, trace_ink, 24);
    // A circle of 15 mm in a picture framed on a part some 40 mm across:
    // hundreds of pixels of line.
    assert!(inked > 200, "{inked} pixels of the trace");
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["timeline"]["traces"], 1);
    run(&mut session, "view.trace", json!({"clear": true}));
    let cleared = screenshot(&mut shell, &mut session, "motion-trace-cleared");
    assert_eq!(cleared, before, "bit for bit");
}

#[test]
fn colliding_bodies_are_drawn_red_by_the_window() {
    let mut session = crank();
    let mut shell = Shell::new();
    run(
        &mut session,
        "view.motion",
        json!({"t": 0.25, "collisions": true}),
    );
    // Only the world's x axis is red while nothing collides.
    let clear = screenshot(&mut shell, &mut session, "motion-clear");
    assert!(reds(&clear) < 100, "{} red pixels", reds(&clear));
    run(&mut session, "view.motion", json!({"t": 0.5}));
    let hit = screenshot(&mut shell, &mut session, "motion-collision");
    assert!(
        reds(&hit) > reds(&clear) + 300,
        "{} red pixels, {} without the collision",
        reds(&hit),
        reds(&clear)
    );
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["timeline"]["t"], 0.5);
    assert_eq!(got["timeline"]["collisions"], json!([["Wall", "Crank"]]));
    assert_eq!(
        got["timeline"]["caption"],
        "t = 0.500 · collision: Wall and Crank"
    );
}

struct Window {
    ctx: egui::Context,
    time: f64,
}

impl Window {
    fn frame(&mut self, shell: &mut Shell, session: &mut Session, seconds: f64) {
        self.time += seconds;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 760.0))),
            time: Some(self.time),
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
        output.textures_delta.clear();
    }

    /// A click at `at`: pressed in one frame, released in the next.
    fn click(&mut self, shell: &mut Shell, session: &mut Session, at: Pos2) {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        for events in [
            vec![egui::Event::PointerMoved(at), button(true)],
            vec![button(false)],
        ] {
            self.time += 0.03;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 760.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
            output.textures_delta.clear();
        }
    }
}

fn shown_t(shell: &mut Shell, session: &mut Session) -> Option<f64> {
    call(shell, session, "ui.get", json!({}))["timeline"]["t"].as_f64()
}

#[test]
fn the_player_plays_the_motion_back_and_forth_and_stops_at_the_end() {
    let mut session = crank();
    let mut shell = Shell::new();
    let mut window = Window {
        ctx: egui::Context::default(),
        time: 0.0,
    };
    window.frame(&mut shell, &mut session, 0.0);
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["timeline"]["hasTimeline"], true);
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"timeline": {"motion": true, "playing": true, "speed": 2}}),
    );
    // At speed 2 the motion takes 2 s from 0 to 1: frames of 0.1 s move
    // t by 0.05 once each moment is shown.
    let mut seen = Vec::new();
    for _ in 0..200 {
        window.frame(&mut shell, &mut session, 0.1);
        if let Some(t) = shown_t(&mut shell, &mut session)
            && seen.last() != Some(&t)
        {
            seen.push(t);
        }
        if call(&mut shell, &mut session, "ui.get", json!({}))["timeline"]["playing"] == false {
            break;
        }
    }
    assert_eq!(seen.first(), Some(&0.0), "{seen:?}");
    assert_eq!(seen.last(), Some(&1.0), "{seen:?}");
    assert!(seen.windows(2).all(|w| w[1] > w[0]), "{seen:?}");
    assert!(seen.len() >= 15, "{seen:?}");
    // The document did not change on the way.
    // file.new, t and two blocks: three edits after the new document.
    assert_eq!(session.document_revision(), Some(3));

    // Back and forth: from 1 back to 0, then it stops (no loop).
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"timeline": {"motion": true, "playing": true, "bounce": true, "speed": 2}}),
    );
    let mut back = Vec::new();
    for _ in 0..300 {
        window.frame(&mut shell, &mut session, 0.1);
        if let Some(t) = shown_t(&mut shell, &mut session)
            && back.last() != Some(&t)
        {
            back.push(t);
        }
        if call(&mut shell, &mut session, "ui.get", json!({}))["timeline"]["playing"] == false {
            break;
        }
    }
    assert!(back.iter().any(|t| *t < 0.5), "{back:?}");
    assert_eq!(back.last(), Some(&0.0), "{back:?}");
    // An edit stops it.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"timeline": {"motion": true, "playing": true, "loop": true}}),
    );
    window.frame(&mut shell, &mut session, 0.1);
    run(
        &mut session,
        "variable.set",
        json!({"name": "t", "expr": "0.5"}),
    );
    window.frame(&mut shell, &mut session, 0.1);
    window.frame(&mut shell, &mut session, 0.1);
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["timeline"]["playing"], false);
}

#[test]
fn without_a_timeline_the_motion_is_not_offered() {
    let mut session = Session::new(Workspace::none());
    run(&mut session, "file.new", json!({}));
    run(
        &mut session,
        "variable.set",
        json!({"name": "t", "expr": "3 mm"}),
    );
    run(
        &mut session,
        "feature.block",
        json!({"size": ["t", "t", "t"]}),
    );
    let mut shell = Shell::new();
    let mut window = Window {
        ctx: egui::Context::default(),
        time: 0.0,
    };
    window.frame(&mut shell, &mut session, 0.0);
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["timeline"]["hasTimeline"], false);
    // Asked to play the motion, the player plays the history instead
    // (since 2026-10-30: the mode follows the document, never a play that
    // does nothing), and no moment of a motion is shown.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"timeline": {"motion": true, "playing": true}}),
    );
    window.frame(&mut shell, &mut session, 0.1);
    window.frame(&mut shell, &mut session, 0.1);
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["timeline"]["playsMotion"], false);
    assert_eq!(got["timeline"]["t"], Value::Null);
}

/// Every thread of the process's pool, held until dropped: a motion's
/// background builder builds nothing meanwhile.
fn freeze_pool() -> Vec<craft3d_math::pool::Lease> {
    let mut held = Vec::new();
    let mut have = 0;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while have < craft3d_math::pool::SHARED_THREADS {
        let lease = craft3d_math::pool::lease(craft3d_math::pool::SHARED_THREADS);
        have += lease.threads();
        held.push(lease);
        assert!(
            std::time::Instant::now() < deadline,
            "the pool never came back"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    held
}

/// A bar part with a published length (`bar.3dcraft`), and an assembly of
/// a crank turned by `360 deg * t` and a bar whose length follows `t`: a
/// configuration of its own at every moment, built in the background
/// while the window plays.
fn stretching(name: &str) -> Session {
    use craft3d_engine::Access;
    let dir =
        std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("ui-motion-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir),
    });
    run(&mut s, "file.new", json!({}));
    run(
        &mut s,
        "variable.set",
        json!({"name": "len", "expr": "24 mm"}),
    );
    run(
        &mut s,
        "variable.publish",
        json!({"name": "len", "min": "10 mm", "max": "500 mm"}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Bar", "size": ["len", 4, 2]}),
    );
    run(
        &mut s,
        "datum.axis",
        json!({"name": "End1", "through": [[2, 2, 0], [2, 2, 1]]}),
    );
    run(&mut s, "file.save", json!({"path": "bar.3dcraft"}));
    run(&mut s, "file.close", json!({}));
    run(&mut s, "assembly.new", json!({}));
    let bar = |name: &str, len: &str, y: f64| {
        json!({"source": "bar.3dcraft", "name": name, "body": "Bar",
               "params": {"len": len}, "position": [0, y, 0]})
    };
    run(&mut s, "instance.add", bar("Ground", "64 mm", 0.0));
    run(&mut s, "instance.add", bar("Crank", "24 mm", 0.0));
    run(
        &mut s,
        "instance.add",
        bar("Belt", "30 mm + 20 mm * t", 20.0),
    );
    run(&mut s, "variable.set", json!({"name": "t", "expr": "0"}));
    for (name, kind, a, b) in [
        ("Base", "fixed", "world/z", "Ground/End1"),
        ("Drive", "revolute", "Ground/End1", "Crank/End1"),
        ("Hold", "fixed", "world/z", "Belt/End1"),
    ] {
        run(
            &mut s,
            "mate.add",
            json!({"kind": kind, "a": a, "b": b, "name": name, "flip": true}),
        );
    }
    run(
        &mut s,
        "variable.set",
        json!({"name": "Drive_angle", "expr": "360 deg * t"}),
    );
    s
}

/// A picture of the window (`ui.screenshot`) and the frames of a
/// frame-exact recording show the exact moment of a playing motion, never
/// the provisional one the viewport may show while the builder works (an
/// independent review of 2026-11-02): the same picture as a window that
/// asked for the moment exactly.
#[test]
fn pictures_and_recorded_frames_of_a_provisional_moment_are_the_exact_one() {
    // The background builder gets no thread: every moment asked for at
    // once stays provisional until something makes it exact.
    let _frozen = freeze_pool();
    let mut window = Window {
        ctx: egui::Context::default(),
        time: 0.0,
    };
    let mut exact = stretching("exact");
    let mut exact_shell = Shell::new();
    window.frame(&mut exact_shell, &mut exact, 0.0);
    run(&mut exact, "view.motion", json!({"t": 0.5}));
    window.frame(&mut exact_shell, &mut exact, 0.0);
    let wanted = screenshot(&mut exact_shell, &mut exact, "motion-exact");

    let mut s = stretching("ready");
    let mut shell = Shell::new();
    window.frame(&mut shell, &mut s, 0.0);
    let answer = run(&mut s, "view.motion", json!({"t": 0.5, "shapes": "ready"}));
    window.frame(&mut shell, &mut s, 0.0);
    if craft3d_math::available_threads() >= 2 {
        assert_eq!(answer["exact"], false, "{answer}");
        // The live viewport shows it provisionally.
        assert!(!s.motion().unwrap().exact);
    }
    assert_eq!(screenshot(&mut shell, &mut s, "motion-ready"), wanted);

    // ui.pick, ui.fit and ui.highlight see the exact moment too: a pixel
    // proven to hit the Belt at the exact moment picks the same there
    // while the viewport shows the moment provisionally.
    let pick = (0..760)
        .step_by(20)
        .flat_map(|y| (0..1100).step_by(20).map(move |x| json!({"x": x, "y": y})))
        .find(|at| {
            call(&mut exact_shell, &mut exact, "ui.pick", at.clone())
                .to_string()
                .contains("Belt")
        })
        .expect("a pixel on the Belt");
    let wanted_pick = call(&mut exact_shell, &mut exact, "ui.pick", pick.clone());
    let provisional = |s: &mut Session| {
        if craft3d_math::available_threads() >= 2 {
            assert!(!s.motion().unwrap().exact, "provisional before the call");
        }
    };
    let mut p = stretching("picked");
    let mut shell = Shell::new();
    window.frame(&mut shell, &mut p, 0.0);
    run(&mut p, "view.motion", json!({"t": 0.5, "shapes": "ready"}));
    provisional(&mut p);
    let got_pick = call(&mut shell, &mut p, "ui.pick", pick);
    assert!(p.motion().unwrap().exact, "ui.pick made the moment exact");
    assert_eq!(got_pick, wanted_pick);
    for (method, params) in [
        ("ui.fit", json!({})),
        ("ui.highlight", json!({"face": "F1:face(0)"})),
    ] {
        let mut h = stretching(&format!("exact-{method}"));
        let mut shell = Shell::new();
        window.frame(&mut shell, &mut h, 0.0);
        run(&mut h, "view.motion", json!({"t": 0.5, "shapes": "ready"}));
        provisional(&mut h);
        // Whether the face is there or not, the scene it is looked up
        // in is the exact moment's.
        let _ = shell.handle_method(&mut h, method, Some(&params));
        assert!(h.motion().unwrap().exact, "{method} made the moment exact");
    }

    // Recorded frame by frame: the viewport is not live, the moment exact.
    let mut r = stretching("recorded");
    let mut shell = Shell::new();
    window.frame(&mut shell, &mut r, 0.0);
    run(&mut r, "view.motion", json!({"t": 0.5, "shapes": "ready"}));
    shell.set_frame_exact(true);
    assert!(shell.exact_pictures());
    window.frame(&mut shell, &mut r, 0.0);
    assert!(r.motion().unwrap().exact);
    assert_eq!(screenshot(&mut shell, &mut r, "motion-recorded"), wanted);
    // And the player asks for exact moments at full quality.
    call(
        &mut shell,
        &mut r,
        "ui.set",
        json!({"timeline": {"motion": true, "playing": true, "speed": 2}}),
    );
    assert!(!matches!(
        shell.scene_quality(),
        craft3d_engine::SceneQuality::Preview
    ));
    for _ in 0..6 {
        window.frame(&mut shell, &mut r, 0.05);
        assert!(r.motion().is_none_or(|m| m.exact));
    }
}

/// A clock for the bake hint that moves `ms` milliseconds each time it is
/// read: every moment's `view.motion` takes `ms` on it.
fn moment_clock(shell: &mut Shell, ms: f64) {
    let now = std::sync::Mutex::new(0.0_f64);
    shell.set_moment_clock(Some(std::sync::Arc::new(move || {
        let mut at = now.lock().unwrap();
        *at += ms;
        *at
    })));
}

/// A motion whose moments are shown provisionally (a configuration of its
/// own at every moment, built in the background as it plays) and quick to
/// make is baked by itself once the player has weighed enough of them, with
/// the hint's progress; a heavy one is offered instead
/// (docs/design/motion.md › *The bake hint*). Its moments take 30 ms,
/// then 150 ms, on the clock the hint times them by.
#[test]
fn a_quick_provisional_motion_bakes_itself_and_a_heavy_one_is_offered() {
    // The background builder gets no thread: the moments stay provisional.
    let frozen = freeze_pool();
    for (cost, name) in [(30.0, "quick"), (150.0, "heavy")] {
        let mut s = stretching(&format!("hint-{name}"));
        let mut shell = Shell::new();
        moment_clock(&mut shell, cost);
        let mut window = Window {
            ctx: egui::Context::default(),
            time: 0.0,
        };
        window.frame(&mut shell, &mut s, 0.0);
        call(
            &mut shell,
            &mut s,
            "ui.set",
            json!({"timeline": {"motion": true, "playing": true, "loop": true}}),
        );
        for _ in 0..30 {
            window.frame(&mut shell, &mut s, 0.03);
        }
        let heavy = call(&mut shell, &mut s, "ui.get", json!({}))["timeline"]["heavy"].clone();
        let ms = heavy["msPerMoment"].as_f64().unwrap();
        assert!((ms - cost).abs() < 1.0, "{name}: {heavy}");
        if craft3d_math::available_threads() < 2 {
            // Without a background, every moment is exact: nothing to say.
            continue;
        }
        assert!(
            heavy["provisional"].as_f64().unwrap() >= 0.5,
            "{name}: {heavy}"
        );
        if name == "quick" {
            assert_eq!(heavy["heavy"], false, "{heavy}");
            assert_eq!(heavy["suggestBake"], false, "{heavy}");
            assert_eq!(heavy["baking"]["auto"], true, "baked by itself: {heavy}");
            assert_eq!(s.motion().unwrap().baked, Some(120));
        } else {
            assert_eq!(heavy["heavy"], true, "{heavy}");
            assert_eq!(heavy["suggestBake"], true, "{heavy}");
            assert_eq!(heavy["baking"], Value::Null, "not baked by itself: {heavy}");
            assert_eq!(s.motion().unwrap().baked, None);
        }
    }
    drop(frozen);
}

/// A bake in the background asked for from the hint goes on to the end
/// while the motion is paused: its bar follows what the builder has left
/// and the hint goes once it is done (review of 2026-11-02, S2: the bar
/// stood still while paused on an exact moment).
#[test]
fn a_bake_in_the_background_finishes_while_paused() {
    if craft3d_math::available_threads() < 2 {
        return;
    }
    let mut s = stretching("hint-paused");
    // As in the window: a bake goes to the background builder.
    s.set_bake_in_background(true);
    let mut shell = Shell::new();
    moment_clock(&mut shell, 150.0);
    let mut window = Window {
        ctx: egui::Context::default(),
        time: 0.0,
    };
    window.frame(&mut shell, &mut s, 0.0);
    call(
        &mut shell,
        &mut s,
        "ui.set",
        json!({"timeline": {"motion": true, "playing": true, "loop": true}}),
    );
    let heavy = |shell: &mut Shell, s: &mut Session| {
        call(shell, s, "ui.get", json!({}))["timeline"]["heavy"].clone()
    };
    for _ in 0..60 {
        window.frame(&mut shell, &mut s, 0.03);
        if heavy(&mut shell, &mut s)["suggestBake"] == true {
            break;
        }
    }
    assert_eq!(heavy(&mut shell, &mut s)["suggestBake"], true);
    // Bake, then pause.
    // A frame more: the panel lays the banner out where it stays.
    window.frame(&mut shell, &mut s, 0.03);
    let at = shell.bake_hint_button().expect("the Bake button").center();
    window.click(&mut shell, &mut s, at);
    assert!(!heavy(&mut shell, &mut s)["baking"].is_null(), "baking");
    call(
        &mut shell,
        &mut s,
        "ui.set",
        json!({"timeline": {"playing": false}}),
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    let mut progress = 0.0;
    loop {
        window.frame(&mut shell, &mut s, 0.03);
        let now = heavy(&mut shell, &mut s);
        if now["baking"].is_null() {
            break;
        }
        let p = now["baking"]["progress"].as_f64().unwrap();
        assert!(p >= progress, "the bar went back: {progress} -> {p}");
        progress = p;
        assert!(
            std::time::Instant::now() < deadline,
            "the bake did not end while paused: {now}"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let motion = s.motion().unwrap();
    assert_eq!(motion.baked, Some(120));
    assert_eq!(motion.building, 0);
    assert_eq!(heavy(&mut shell, &mut s)["suggestBake"], false);
}
