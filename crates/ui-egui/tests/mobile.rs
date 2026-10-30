//! The window on a phone (390 × 844 points) and a tablet (820 × 1180):
//! below `Tokens::narrow_width` the side panel is a drawer behind a
//! button in the menu bar and the controls over the viewport are as
//! large as a finger needs; every control is there, inside the viewport
//! and clear of the others, the section's options included, and the
//! Studio popover fits. Touch: one finger turns the view, two fingers
//! pan, a pinch zooms, a tap selects and a long press opens the "select
//! other" list. Driven by simulated touch events in real frames, as the
//! web and desktop integrations send them (a touch, and pointer events
//! for the first finger).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::{RendererChoice, Shell};
use egui::{Event, PointerButton, Pos2, Rect, TouchDeviceId, TouchId, TouchPhase, pos2, vec2};
use serde_json::{Value, json};

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn part() -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "variable.set",
        json!({ "name": "width", "expr": "60 mm" }),
    );
    run(
        &mut s,
        "variable.publish",
        json!({ "name": "width", "min": "20 mm", "max": "200 mm" }),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": ["width", 40, 30]}),
    );
    s
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

    fn frame(&mut self, shell: &mut Shell, session: &mut Session, events: Vec<Event>) {
        self.time += 1.0 / 60.0;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
        output.textures_delta.clear();
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

    /// One frame of fingers: `(finger, phase, where)`. The first finger
    /// also moves and presses the pointer, as the integrations do.
    fn touch(
        &mut self,
        shell: &mut Shell,
        session: &mut Session,
        fingers: &[(u64, TouchPhase, Pos2)],
    ) {
        let mut events = Vec::new();
        for &(id, phase, pos) in fingers {
            events.push(Event::Touch {
                device_id: TouchDeviceId(1),
                id: TouchId(id),
                phase,
                pos,
                force: None,
            });
            if id == 0 {
                let button = |pressed| Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                };
                match phase {
                    TouchPhase::Start => {
                        events.push(Event::PointerMoved(pos));
                        events.push(button(true));
                    }
                    TouchPhase::Move => events.push(Event::PointerMoved(pos)),
                    TouchPhase::End | TouchPhase::Cancel => {
                        events.push(button(false));
                        events.push(Event::PointerGone);
                    }
                }
            }
        }
        self.frame(shell, session, events);
    }
}

fn opened(width: f32, height: f32) -> (Session, Shell, Window) {
    let mut session = part();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(width, height);
    window.frames(&mut shell, &mut session, 6);
    (session, shell, window)
}

fn set(shell: &mut Shell, session: &mut Session, patch: Value) {
    shell
        .handle_method(session, "ui.set", Some(&patch))
        .unwrap()
        .unwrap();
}

const NAMES: [&str; 10] = [
    "sketches",
    "datums",
    "edges",
    "studio",
    "analysis",
    "see-through",
    "section",
    "fit",
    "iso",
    "projection",
];

/// Every control there, inside the viewport, clear of the other controls
/// and of the plates it is not on, and at least `least` high.
fn every_control_reachable(shell: &Shell, names: &[&str], least: f32, what: &str) {
    let viewport = shell.viewport_rect().unwrap();
    let layout = shell.controls_layout().unwrap();
    let controls = layout.controls();
    for name in names {
        let Some((_, rect)) = controls.iter().find(|(n, _)| n == name) else {
            panic!("{what}: no `{name}` in {controls:?}");
        };
        assert!(viewport.contains_rect(*rect), "{what}: {name} {rect:?}");
        assert!(rect.height() >= least - 0.01, "{what}: {name} {rect:?}");
        for (other, r) in &controls {
            if other != name {
                assert!(
                    !rect.shrink(0.01).intersects(*r),
                    "{what}: {name} over {other}"
                );
            }
        }
    }
    let parts = layout.parts();
    for (k, (a, ra)) in parts.iter().enumerate() {
        assert!(viewport.contains_rect(*ra), "{what}: {a} {ra:?}");
        for (b, rb) in parts.iter().skip(k + 1) {
            assert!(!ra.shrink(0.01).intersects(*rb), "{what}: {a} over {b}");
        }
    }
}

#[test]
fn a_phone_gets_a_drawer_and_controls_a_finger_hits() {
    let (mut session, mut shell, mut window) = opened(390.0, 844.0);
    assert!(shell.narrow());
    let tokens = shell.tokens;
    assert!(tokens.nav_height >= tokens.touch_target);
    // The drawer is shut: the viewport has the window's width.
    assert_eq!(shell.tree_panel_rect(), None);
    let viewport = shell.viewport_rect().unwrap();
    assert!(viewport.width() >= 389.0, "{viewport:?}");
    every_control_reachable(&shell, &NAMES, tokens.touch_target, "phone");
    // The section's options, on a plate of their own when the bar has no
    // room for them.
    set(&mut shell, &mut session, json!({"section": {"on": true}}));
    window.frames(&mut shell, &mut session, 3);
    let mut names = NAMES.to_vec();
    names.extend(["axes", "offset", "side"]);
    every_control_reachable(&shell, &names, tokens.touch_target, "phone, section");
    // A tap on the Section chip switches it off: it is reachable.
    let chip = shell.controls_layout().unwrap().section_toggle.unwrap();
    window.click(&mut shell, &mut session, chip.center());
    assert!(!shell.state.section.on);
    // The Studio popover fits the screen and keeps clear of the controls.
    let chip = shell.controls_layout().unwrap().studio.unwrap();
    window.click(&mut shell, &mut session, chip.center());
    let panel = shell
        .look_hits()
        .iter()
        .find(|(n, _)| n == "panel")
        .map(|(_, r)| *r)
        .expect("the studio popover");
    assert!(viewport.contains_rect(panel), "{panel:?}");
    for (name, part) in shell.controls_layout().unwrap().parts() {
        assert!(!part.intersects(panel), "over the {name}");
    }
    window.click(&mut shell, &mut session, chip.center());
    // The drawer: its button opens it over the viewport, again closes it.
    let button = shell.drawer_button_rect().expect("the drawer's button");
    assert!(button.height() >= tokens.touch_target - 0.01, "{button:?}");
    assert!(button.width() >= tokens.touch_target - 0.01, "{button:?}");
    window.click(&mut shell, &mut session, button.center());
    assert!(shell.state.panels.drawer);
    let drawer = shell.tree_panel_rect().expect("the drawer");
    assert!(
        drawer.width() <= 390.0 * tokens.drawer_share + 0.5,
        "{drawer:?}"
    );
    assert!(viewport.contains_rect(drawer), "{drawer:?} in {viewport:?}");
    // The viewport keeps its size under it.
    assert_eq!(shell.viewport_rect().unwrap(), viewport);
    window.click(&mut shell, &mut session, button.center());
    assert!(!shell.state.panels.drawer);
    assert_eq!(shell.tree_panel_rect(), None);
    // A tap beside the open drawer closes it and selects nothing.
    window.click(&mut shell, &mut session, button.center());
    assert!(shell.tree_panel_rect().is_some());
    let beside = pos2(viewport.right() - 20.0, viewport.center().y);
    window.click(&mut shell, &mut session, beside);
    assert!(!shell.state.panels.drawer);
    assert_eq!(shell.selection().items.len(), 0);
}

#[test]
fn a_tablet_keeps_the_side_panel_and_a_wide_window_the_desktop_layout() {
    let (_session, shell, _window) = opened(820.0, 1180.0);
    assert!(!shell.narrow());
    assert!(shell.tree_panel_rect().is_some());
    assert_eq!(shell.drawer_button_rect(), None);
    every_control_reachable(&shell, &NAMES, shell.tokens.nav_height, "tablet");
    // A window made narrow gets the narrow layout, and back.
    let (mut session, mut shell, mut window) = opened(1280.0, 800.0);
    let wide = shell.tokens;
    window.size = vec2(600.0, 800.0);
    window.frames(&mut shell, &mut session, 3);
    assert!(shell.narrow());
    assert!(shell.drawer_button_rect().is_some());
    window.size = vec2(1280.0, 800.0);
    window.frames(&mut shell, &mut session, 3);
    assert!(!shell.narrow());
    assert_eq!(shell.tokens, wide);
    assert!(shell.tree_panel_rect().is_some());
}

#[test]
fn fingers_turn_pan_zoom_select_and_open_the_list() {
    let (mut session, mut shell, mut window) = opened(390.0, 844.0);
    let middle = shell.viewport_rect().unwrap().center();
    use TouchPhase::{End, Move, Start};
    // One finger turns the view.
    let before = shell.state.camera;
    window.touch(&mut shell, &mut session, &[(0, Start, middle)]);
    for k in 1..=8 {
        let at = middle + vec2(10.0 * k as f32, 0.0);
        window.touch(&mut shell, &mut session, &[(0, Move, at)]);
    }
    window.touch(
        &mut shell,
        &mut session,
        &[(0, End, middle + vec2(80.0, 0.0))],
    );
    window.frames(&mut shell, &mut session, 2);
    let turned = shell.state.camera;
    assert!(
        (turned.yaw - before.yaw).abs() > 5.0,
        "{before:?} {turned:?}"
    );
    assert_eq!(turned.target, before.target);
    // Two fingers spread: a zoom in, without a turn.
    let (a, b) = (middle - vec2(40.0, 0.0), middle + vec2(40.0, 0.0));
    window.touch(&mut shell, &mut session, &[(0, Start, a), (1, Start, b)]);
    for k in 1..=8 {
        let d = vec2(8.0 * k as f32, 0.0);
        window.touch(
            &mut shell,
            &mut session,
            &[(0, Move, a - d), (1, Move, b + d)],
        );
    }
    let d = vec2(64.0, 0.0);
    window.touch(
        &mut shell,
        &mut session,
        &[(0, End, a - d), (1, End, b + d)],
    );
    window.frames(&mut shell, &mut session, 2);
    let zoomed = shell.state.camera;
    assert!(
        zoomed.distance < turned.distance * 0.95,
        "{turned:?} {zoomed:?}"
    );
    assert!(
        (zoomed.yaw - turned.yaw).abs() < 1e-9,
        "{turned:?} {zoomed:?}"
    );
    // Two fingers moved together: a pan, without a turn.
    window.touch(&mut shell, &mut session, &[(0, Start, a), (1, Start, b)]);
    for k in 1..=8 {
        let d = vec2(0.0, 8.0 * k as f32);
        window.touch(
            &mut shell,
            &mut session,
            &[(0, Move, a + d), (1, Move, b + d)],
        );
    }
    let d = vec2(0.0, 64.0);
    window.touch(
        &mut shell,
        &mut session,
        &[(0, End, a + d), (1, End, b + d)],
    );
    window.frames(&mut shell, &mut session, 2);
    let panned = shell.state.camera;
    assert_ne!(panned.target, zoomed.target);
    assert!((panned.yaw - zoomed.yaw).abs() < 1e-9);
    assert!((panned.pitch - zoomed.pitch).abs() < 1e-9);
    // Back to the whole model, then a tap on it selects it.
    shell
        .handle_method(&mut session, "ui.fit", Some(&json!({"view": "iso"})))
        .unwrap()
        .unwrap();
    window.frames(&mut shell, &mut session, 30);
    let middle = shell.viewport_rect().unwrap().center();
    window.touch(&mut shell, &mut session, &[(0, Start, middle)]);
    window.touch(&mut shell, &mut session, &[(0, End, middle)]);
    window.frames(&mut shell, &mut session, 3);
    assert!(
        !shell.selection().items.is_empty(),
        "{:?}",
        shell.selection()
    );
    // A long press opens the "select other" list there.
    window.frames(&mut shell, &mut session, 30);
    window.touch(&mut shell, &mut session, &[(0, Start, middle)]);
    for _ in 0..60 {
        window.touch(&mut shell, &mut session, &[]);
    }
    window.touch(&mut shell, &mut session, &[(0, End, middle)]);
    window.frames(&mut shell, &mut session, 2);
    assert!(shell.other_menu().is_some(), "no list after a long press");
}

/// A pinch whose first finger stays still ends with that finger lifted
/// where it went down, as a tap would: it selects nothing and closes no
/// drawer. A double tap on the background frames the whole model again.
#[test]
fn a_pinch_is_no_tap_and_a_double_tap_frames_everything() {
    use TouchPhase::{End, Move, Start};
    let (mut session, mut shell, mut window) = opened(390.0, 844.0);
    let framed = shell.state.camera;
    let middle = shell.viewport_rect().unwrap().center();
    for drawer in [false, true] {
        set(
            &mut shell,
            &mut session,
            json!({"panels": {"drawer": drawer}}),
        );
        window.frames(&mut shell, &mut session, 10);
        // Finger 0 on the model, still; finger 1 moves away from it.
        let (a, b) = (middle, middle + vec2(60.0, 40.0));
        window.touch(&mut shell, &mut session, &[(0, Start, a)]);
        window.touch(&mut shell, &mut session, &[(0, Move, a), (1, Start, b)]);
        for k in 1..=8 {
            let d = vec2(6.0 * k as f32, 4.0 * k as f32);
            window.touch(&mut shell, &mut session, &[(0, Move, a), (1, Move, b + d)]);
        }
        window.touch(&mut shell, &mut session, &[(1, End, b + vec2(48.0, 32.0))]);
        window.touch(&mut shell, &mut session, &[(0, End, a)]);
        window.frames(&mut shell, &mut session, 3);
        assert!(
            shell.selection().items.is_empty(),
            "drawer {drawer}: {:?}",
            shell.selection()
        );
        assert_eq!(shell.state.panels.drawer, drawer, "the drawer stays");
    }
    set(
        &mut shell,
        &mut session,
        json!({"panels": {"drawer": false}}),
    );
    let zoomed = shell.state.camera;
    assert!(
        zoomed.distance < framed.distance * 0.95,
        "{framed:?} {zoomed:?}"
    );
    // A tap after the pinch is a tap again.
    window.frames(&mut shell, &mut session, 10);
    // A double tap on the background: what F does, the whole model.
    let corner = shell.viewport_rect().unwrap().left_top() + vec2(30.0, 30.0);
    for _ in 0..2 {
        window.touch(&mut shell, &mut session, &[(0, Start, corner)]);
        window.touch(&mut shell, &mut session, &[(0, End, corner)]);
    }
    window.frames(&mut shell, &mut session, 60);
    let fitted = shell.state.camera;
    assert!(
        (fitted.distance - framed.distance).abs() < framed.distance * 0.05,
        "{framed:?} {fitted:?}"
    );
}
