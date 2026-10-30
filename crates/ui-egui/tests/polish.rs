//! The last of the UX polish (2026-10-30): one `t` on screen while a
//! motion plays (the player's; no caption, no copy in the status bar), a
//! first section through the middle of the model, off toggles drawn as
//! enabled, the start screen's card inside a phone's screen, the compact
//! player on a phone, the tab overflow and the short status bar there.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Access, Session, Workspace};
use craft3d_ui_egui::{RendererChoice, Shell, StatusSlot, TabPart};
use egui::{Event, PointerButton, Pos2, Rect, pos2, vec2};
use serde_json::{Value, json};
use std::path::PathBuf;

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn part(s: &mut Session) {
    run(s, "file.new", json!({"units": "mm"}));
    run(
        s,
        "feature.block",
        json!({"name": "Plate", "size": [60, 40, 30]}),
    );
}

fn workspace(name: &str) -> Session {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir),
    })
}

/// A four-bar of four instances of one bar part, its crank driven by `t`.
fn four_bar(s: &mut Session) {
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
    run(s, "assembly.new", json!({}));
    for (name, len) in [
        ("Ground", "64 mm"),
        ("Crank", "24 mm"),
        ("Coupler", "64 mm"),
        ("Rocker", "24 mm"),
    ] {
        run(
            s,
            "instance.add",
            json!({ "source": "bar.3dcraft", "name": name, "body": "Bar",
                    "params": { "len": len } }),
        );
    }
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

struct Window {
    ctx: egui::Context,
    size: egui::Vec2,
    time: f64,
}

impl Window {
    fn new(width: f32, height: f32) -> Self {
        Self {
            ctx: egui::Context::default(),
            size: vec2(width, height),
            time: 0.0,
        }
    }

    /// One frame; the texts it drew.
    fn frame(
        &mut self,
        shell: &mut Shell,
        session: &mut Session,
        events: Vec<Event>,
    ) -> Vec<String> {
        self.time += 1.0 / 60.0;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
        output.textures_delta.clear();
        output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) => Some(text.galley.job.text.clone()),
                _ => None,
            })
            .collect()
    }

    fn frames(&mut self, shell: &mut Shell, session: &mut Session, n: usize) {
        for _ in 0..n {
            self.frame(shell, session, Vec::new());
        }
    }

    fn click(&mut self, shell: &mut Shell, session: &mut Session, at: Pos2) {
        let press = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        for events in [
            vec![Event::PointerMoved(at)],
            vec![press(true)],
            vec![press(false)],
            vec![Event::PointerGone],
        ] {
            self.frame(shell, session, events);
        }
        self.frames(shell, session, 3);
    }
}

fn set(shell: &mut Shell, session: &mut Session, patch: Value) {
    shell
        .handle_method(session, "ui.set", Some(&patch))
        .unwrap()
        .unwrap_or_else(|e| panic!("ui.set {patch}: {e}"));
}

/// While a motion plays, exactly one `t` is drawn (the player's), on the
/// desktop and on a phone, and the status bar says `Playing` without one.
#[test]
fn one_t_on_screen_while_a_motion_plays() {
    for (width, height) in [(1280.0, 800.0), (390.0, 844.0)] {
        let mut session = workspace(&format!("one-t-{width}"));
        four_bar(&mut session);
        let mut shell = Shell::with_renderer(RendererChoice::Cpu);
        let mut window = Window::new(width, height);
        window.frames(&mut shell, &mut session, 4);
        set(
            &mut shell,
            &mut session,
            json!({ "timeline": { "motion": true, "playing": true, "loop": true } }),
        );
        let mut checked = 0;
        for k in 0..60 {
            let drawn = window.frame(&mut shell, &mut session, Vec::new());
            if k < 10 {
                continue;
            }
            let ts: Vec<&String> = drawn.iter().filter(|t| t.starts_with("t = ")).collect();
            assert_eq!(ts.len(), 1, "{width}: frame {k}: {ts:?}");
            checked += 1;
            assert_eq!(shell.status_shown(StatusSlot::Command), Some("Playing"));
            assert!(
                shell
                    .status_shown(StatusSlot::Answer)
                    .is_none_or(|a| !a.contains("t =")),
                "{:?}",
                shell.status_shown(StatusSlot::Answer)
            );
        }
        assert!(checked > 40);
        // The caption agents read keeps saying it, as the player does.
        let got = shell
            .handle_method(&mut session, "ui.get", Some(&json!({})))
            .unwrap()
            .unwrap();
        let t = got["timeline"]["t"].as_f64().unwrap();
        assert!(
            got["timeline"]["caption"]
                .as_str()
                .unwrap()
                .starts_with(&format!("t = {t:.3}")),
            "{got:?}"
        );
    }
}

/// A first section switched on without a place cuts across Z through the
/// middle of the model's box; switched off and on again it keeps where
/// it was; an offset given is kept.
#[test]
fn a_first_section_cuts_through_the_middle() {
    let mut session = Session::new(Workspace::none());
    part(&mut session);
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1280.0, 800.0);
    window.frames(&mut shell, &mut session, 3);
    set(&mut shell, &mut session, json!({"section": {"on": true}}));
    assert_eq!(shell.state.section.plane.as_deref(), Some("xy"));
    assert!(
        (shell.state.section.offset - 15.0).abs() < 1e-9,
        "{:?}",
        shell.state.section
    );
    set(
        &mut shell,
        &mut session,
        json!({"section": {"offset": 4.0}}),
    );
    set(&mut shell, &mut session, json!({"section": {"on": false}}));
    set(&mut shell, &mut session, json!({"section": {"on": true}}));
    assert!((shell.state.section.offset - 4.0).abs() < 1e-9);
    // A plane named with no offset: through the middle along its normal.
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    window.frames(&mut shell, &mut session, 3);
    set(
        &mut shell,
        &mut session,
        json!({"section": {"on": true, "plane": "yz"}}),
    );
    assert!(
        (shell.state.section.offset - 30.0).abs() < 1e-9,
        "{:?}",
        shell.state.section
    );
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    window.frames(&mut shell, &mut session, 3);
    set(
        &mut shell,
        &mut session,
        json!({"section": {"on": true, "offset": 2.0}}),
    );
    assert!((shell.state.section.offset - 2.0).abs() < 1e-9);
}

/// An off toggle reads as enabled: its text nearly as bright as the
/// plate's own, far brighter than the dim text of what is disabled.
#[test]
fn off_toggles_read_as_enabled() {
    let tokens = craft3d_ui_egui::Tokens::default();
    let light = |c: [u8; 3]| c.iter().map(|&v| f32::from(v)).sum::<f32>() / 3.0;
    assert!(light(tokens.nav_off_text) > light(tokens.plate_text_dim) + 40.0);
    assert!(light(tokens.nav_off_text) > 0.9 * light(tokens.plate_text));
}

/// On a phone the start screen's card is inside the screen.
#[test]
fn the_start_card_fits_a_phone() {
    for (width, height) in [(390.0, 844.0), (320.0, 640.0), (1280.0, 800.0)] {
        let mut session = Session::new(Workspace::none());
        let mut shell = Shell::with_renderer(RendererChoice::Cpu);
        let mut window = Window::new(width, height);
        window.frames(&mut shell, &mut session, 4);
        let card = shell
            .start_hits()
            .iter()
            .find(|(n, _)| n == "card")
            .map(|(_, r)| *r)
            .expect("the card");
        let viewport = shell.viewport_rect().unwrap();
        assert!(
            card.left() >= viewport.left() - 0.5 && card.right() <= viewport.right() + 0.5,
            "{width}: {card:?} in {viewport:?}"
        );
        for (name, hit) in shell.start_hits() {
            if name != "card" && !name.starts_with("example:") {
                assert!(
                    hit.right() <= viewport.right() + 0.5,
                    "{width}: {name} {hit:?} past {viewport:?}"
                );
            }
        }
    }
}

/// On a phone a document with a history has the compact player above
/// the bottom bar, clear of every control; its play button plays; with
/// the drawer open it makes way.
#[test]
fn a_phone_has_a_compact_player() {
    let mut session = Session::new(Workspace::none());
    part(&mut session);
    run(
        &mut session,
        "feature.cylinder",
        json!({"name": "Post", "position": [12, 20, 30], "radius": 6, "height": 20}),
    );
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(390.0, 844.0);
    window.frames(&mut shell, &mut session, 4);
    let layout = shell.controls_layout().unwrap();
    let plate = layout.player_plate.expect("the compact player");
    let viewport = shell.viewport_rect().unwrap();
    assert!(viewport.contains_rect(plate), "{plate:?}");
    for (name, part) in layout.parts() {
        if name != "player" {
            assert!(!part.intersects(plate), "over the {name}");
        }
    }
    assert!(plate.bottom() <= layout.bar.unwrap().top());
    // The third of the five buttons plays.
    let pad = shell.tokens.nav_padding;
    let side = plate.height() - 2.0 * pad;
    let play = pos2(
        plate.left() + pad + 2.0 * (side + 2.0) + side * 0.5,
        plate.center().y,
    );
    window.click(&mut shell, &mut session, play);
    assert!(shell.state.timeline.playing);
    window.click(&mut shell, &mut session, play);
    assert!(!shell.state.timeline.playing);
    set(
        &mut shell,
        &mut session,
        json!({"panels": {"drawer": true}}),
    );
    window.frames(&mut shell, &mut session, 3);
    assert_eq!(shell.controls_layout().unwrap().player_plate, None);
}

/// On a phone with four documents the tabs that do not fit are listed
/// behind one button, and the status bar keeps to the short texts.
#[test]
fn a_phone_lists_the_tabs_it_has_no_room_for() {
    let mut session = Session::new(Workspace::none());
    for _ in 0..4 {
        part(&mut session);
    }
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(390.0, 844.0);
    window.frames(&mut shell, &mut session, 4);
    assert!(
        shell
            .tab_hits()
            .iter()
            .any(|(part, _)| *part == TabPart::All),
        "{:?}",
        shell.tab_hits()
    );
    let all = shell
        .tab_hits()
        .iter()
        .find(|(part, _)| *part == TabPart::All)
        .map(|(_, r)| *r)
        .unwrap();
    assert!(all.right() <= 390.5, "{all:?}");
    assert_eq!(shell.status_shown(StatusSlot::Path), None);
    assert!(
        shell
            .status_shown(StatusSlot::Render)
            .is_none_or(|text| !text.contains("triangles")),
        "{:?}",
        shell.status_shown(StatusSlot::Render)
    );
    // A wide window has room for them all.
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1280.0, 800.0);
    window.frames(&mut shell, &mut session, 4);
    assert!(
        !shell
            .tab_hits()
            .iter()
            .any(|(part, _)| *part == TabPart::All)
    );
}
