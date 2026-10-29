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
    // Asked to play the motion, the player plays nothing and stops.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"timeline": {"motion": true, "playing": true}}),
    );
    window.frame(&mut shell, &mut session, 0.1);
    window.frame(&mut shell, &mut session, 0.1);
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["timeline"]["playing"], false);
    assert_eq!(got["timeline"]["t"], Value::Null);
}
