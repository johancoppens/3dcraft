//! The sliders over the viewport (See-through, the section's Offset): the
//! kit's one slider in a row (label, rail, value), driven by synthetic
//! input as a person drives it: a drag of the knob (plain on a fine grid,
//! with Shift ten times finer, with Ctrl in steps), a press on the rail,
//! the arrow keys and the wheel once it has the focus, a double click on
//! the knob for the default, and typing an expression with units into its
//! value, evaluated with the document's variables; text that is not a
//! number shows why and changes nothing, Escape gives up.
//!
//! Expectations changed on 2026-11-04 (the developer's decision of that
//! day: one slider for the whole app, the design's slider with the old
//! number field's precision): a drag starts on the knob (a press on the
//! rail away from it puts the knob there), a plain drag lands on the fine
//! grid of a hundredth of the range (1 % here), the ‹ › at the ends are
//! gone (the arrow keys step instead), and typing begins with a click on
//! the value instead of a double click.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::{RendererChoice, Shell};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, pos2, vec2};
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
        json!({"name": "width", "expr": "40 mm"}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": [60, 40, 8]}),
    );
    s
}

struct Window {
    ctx: egui::Context,
    time: f64,
    modifiers: Modifiers,
}

impl Window {
    fn frame(&mut self, shell: &mut Shell, session: &mut Session, mut events: Vec<Event>) {
        self.time += 1.0 / 60.0;
        events.insert(0, Event::ModifiersChanged(self.modifiers));
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 760.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
        output.textures_delta.clear();
    }

    fn button(&mut self, shell: &mut Shell, session: &mut Session, at: Pos2, pressed: bool) {
        let modifiers = self.modifiers;
        self.frame(
            shell,
            session,
            vec![Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers,
            }],
        );
    }

    fn drag(&mut self, shell: &mut Shell, session: &mut Session, from: Pos2, by: f32) {
        self.frame(shell, session, vec![Event::PointerMoved(from)]);
        self.button(shell, session, from, true);
        for step in 1..=6 {
            let at = from + vec2(by * step as f32 / 6.0, 0.0);
            self.frame(shell, session, vec![Event::PointerMoved(at)]);
        }
        self.button(shell, session, from + vec2(by, 0.0), false);
        self.frame(shell, session, Vec::new());
    }

    fn click(&mut self, shell: &mut Shell, session: &mut Session, at: Pos2) {
        self.frame(shell, session, vec![Event::PointerMoved(at)]);
        self.button(shell, session, at, true);
        self.button(shell, session, at, false);
        self.frame(shell, session, Vec::new());
    }

    fn double_click(&mut self, shell: &mut Shell, session: &mut Session, at: Pos2) {
        self.frame(shell, session, vec![Event::PointerMoved(at)]);
        // Long after any click before, so these two are a double click
        // and not the end of a triple one.
        self.time += 1.0;
        for _ in 0..2 {
            self.button(shell, session, at, true);
            self.button(shell, session, at, false);
        }
        self.frame(shell, session, Vec::new());
        self.frame(shell, session, Vec::new());
    }

    fn key(&mut self, shell: &mut Shell, session: &mut Session, key: Key) {
        let modifiers = self.modifiers;
        for pressed in [true, false] {
            self.frame(
                shell,
                session,
                vec![Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers,
                }],
            );
        }
        self.frame(shell, session, Vec::new());
        self.frame(shell, session, Vec::new());
    }

    fn type_text(&mut self, shell: &mut Shell, session: &mut Session, text: &str) {
        self.frame(shell, session, vec![Event::Text(text.to_string())]);
        self.frame(shell, session, Vec::new());
    }
}

fn started() -> (Session, Shell, Window) {
    let mut session = part();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window {
        ctx: egui::Context::default(),
        time: 0.0,
        modifiers: Modifiers::NONE,
    };
    for _ in 0..4 {
        window.frame(&mut shell, &mut session, Vec::new());
    }
    (session, shell, window)
}

/// The rail of slider `name` (between the knob's centre at either end)
/// and its value, which a click types into.
fn parts(shell: &Shell, name: &str) -> (Rect, Rect) {
    let (_, rail, value) = shell
        .slider_hits()
        .into_iter()
        .find(|(n, _, _)| *n == name)
        .unwrap_or_else(|| panic!("no slider `{name}` in {:?}", shell.slider_hits()));
    (rail, value.unwrap())
}

/// Where the see-through slider's knob is now.
fn knob(shell: &Shell) -> Pos2 {
    let (rail, _) = parts(shell, "see-through");
    let f = (shell.state.transparency / MAX) as f32;
    pos2(rail.left() + rail.width() * f, rail.center().y)
}

const MAX: f64 = craft3d_ui_egui::state::MAX_TRANSPARENCY;

#[test]
fn a_drag_shift_finer_ctrl_in_steps_and_the_keys() {
    let (mut session, mut shell, mut window) = started();
    let (rail, _) = parts(&shell, "see-through");
    let camera = shell.state.camera;
    // A drag of the knob over a quarter of the rail: a quarter of 0 to
    // 0.9, on the fine grid of 1 % (a hundredth of the range, rounded).
    let at = knob(&shell);
    window.drag(&mut shell, &mut session, at, rail.width() * 0.25);
    let t = shell.state.transparency;
    assert!((t - 0.225).abs() <= 0.005 + 1e-9, "{t}");
    assert!(
        ((t / 0.01).round() * 0.01 - t).abs() < 1e-9,
        "on the grid: {t}"
    );
    // With Shift ten times finer: a quarter of the rail more is 0.0225,
    // on a grid ten times finer.
    window.modifiers = Modifiers::SHIFT;
    let at = knob(&shell);
    window.drag(&mut shell, &mut session, at, rail.width() * 0.25);
    let finer = shell.state.transparency - t;
    assert!((finer - 0.0225).abs() <= 0.0005 + 1e-9, "{finer}");
    // With Ctrl on the steps of 5 %.
    window.modifiers = Modifiers::CTRL;
    let at = knob(&shell);
    window.drag(&mut shell, &mut session, at, rail.width() * 0.1);
    let stepped = shell.state.transparency;
    assert!(
        ((stepped / 0.05).round() * 0.05 - stepped).abs() < 1e-9,
        "{stepped}"
    );
    window.modifiers = Modifiers::NONE;
    // Focused by the drag: the arrow keys step it, ten steps with Shift.
    let before = shell.state.transparency;
    window.key(&mut shell, &mut session, Key::ArrowLeft);
    assert!((shell.state.transparency - (before - 0.05)).abs() < 1e-9);
    window.key(&mut shell, &mut session, Key::ArrowRight);
    assert!((shell.state.transparency - before).abs() < 1e-9);
    window.modifiers = Modifiers::SHIFT;
    window.key(&mut shell, &mut session, Key::ArrowLeft);
    window.modifiers = Modifiers::NONE;
    assert!((shell.state.transparency - (before - 0.5).max(0.0)).abs() < 1e-9);
    // A press on the rail away from the knob puts it there.
    window.click(
        &mut shell,
        &mut session,
        pos2(rail.right() + 2.0, rail.center().y),
    );
    assert!((shell.state.transparency - MAX).abs() < 1e-9);
    // A double click on the knob: the default, opaque.
    let at = knob(&shell);
    window.double_click(&mut shell, &mut session, at);
    assert_eq!(shell.state.transparency, 0.0);
    // None of it moved the camera or picked.
    assert_eq!(shell.state.camera, camera);
    assert_eq!(shell.state.picked, None);
}

#[test]
fn typed_expressions_with_units_set_the_value_and_bad_text_says_why() {
    let (mut session, mut shell, mut window) = started();
    // The see-through slider: a percentage, an expression.
    let (_, value) = parts(&shell, "see-through");
    window.click(&mut shell, &mut session, value.center());
    let typing = shell.typing().expect("typing began");
    assert_eq!(typing.field, "see-through");
    assert_eq!(typing.text, "0");
    // The whole text is selected: what is typed replaces it.
    window.type_text(&mut shell, &mut session, "20 + 15 %");
    assert_eq!(shell.typing().unwrap().text, "20 + 15 %");
    window.key(&mut shell, &mut session, Key::Enter);
    assert!(shell.typing().is_none(), "{:?}", shell.typing());
    assert!((shell.state.transparency - 0.35).abs() < 1e-12);

    // The section's offset: on, then a length with the document's
    // variables: 40 mm / 2 - 3 mm.
    let toggle = shell
        .section_hits()
        .iter()
        .find(|(n, _)| *n == "toggle")
        .unwrap()
        .1;
    window.click(&mut shell, &mut session, toggle.center());
    assert!(shell.state.section.on);
    let (rail, value) = parts(&shell, "offset");
    // Enter with the pointer over the slider types too.
    window.frame(
        &mut shell,
        &mut session,
        vec![Event::PointerMoved(rail.center())],
    );
    window.key(&mut shell, &mut session, Key::Enter);
    assert_eq!(shell.typing().unwrap().field, "offset");
    window.type_text(&mut shell, &mut session, "width / 2 - 3 mm");
    window.key(&mut shell, &mut session, Key::Enter);
    assert!(shell.typing().is_none(), "{:?}", shell.typing());
    assert!((shell.state.section.offset - 17.0).abs() < 1e-9);

    // Text that is no number: the field says why and nothing changes.
    let before = shell.state.section.clone();
    window.click(&mut shell, &mut session, value.center());
    window.type_text(&mut shell, &mut session, "width +");
    window.key(&mut shell, &mut session, Key::Enter);
    let typing = shell.typing().expect("still typing");
    assert!(typing.error.is_some(), "{typing:?}");
    assert_eq!(shell.state.section, before);
    // An area is not a length either.
    window.type_text(&mut shell, &mut session, "width * width");
    window.key(&mut shell, &mut session, Key::Enter);
    assert!(
        shell
            .typing()
            .unwrap()
            .error
            .as_deref()
            .unwrap()
            .contains("length")
    );
    assert_eq!(shell.state.section, before);
    // Escape gives up: the value stays.
    window.key(&mut shell, &mut session, Key::Escape);
    assert!(shell.typing().is_none());
    assert_eq!(shell.state.section, before);
    // While typing, F is a letter and not Fit: the camera stays.
    let camera = shell.state.camera;
    window.click(&mut shell, &mut session, value.center());
    window.type_text(&mut shell, &mut session, "F");
    window.key(&mut shell, &mut session, Key::F);
    assert_eq!(shell.state.camera, camera);
    window.key(&mut shell, &mut session, Key::Escape);
    assert_eq!(shell.state.picked, None);
}

#[test]
fn the_section_options_are_not_there_while_it_is_off() {
    let (mut session, mut shell, mut window) = started();
    assert!(!shell.state.section.on);
    let layout = shell.controls_layout().unwrap();
    // Only the toggle: no axis, no offset field, no flip, nothing to type
    // into.
    assert!(layout.section_toggle.is_some());
    assert!(layout.axes.is_none() && layout.offset.is_none() && layout.side.is_none());
    let names: Vec<&str> = shell.section_hits().iter().map(|(n, _)| *n).collect();
    assert_eq!(names, ["toggle"]);
    // On, they are there; off again, gone, and the state keeps the plane.
    let toggle = layout.section_toggle.unwrap().center();
    window.click(&mut shell, &mut session, toggle);
    window.frame(&mut shell, &mut session, Vec::new());
    let on = shell.controls_layout().unwrap();
    assert!(on.axes.is_some() && on.offset.is_some() && on.side.is_some());
    window.click(&mut shell, &mut session, toggle);
    window.frame(&mut shell, &mut session, Vec::new());
    let off = shell.controls_layout().unwrap();
    assert!(off.offset.is_none());
    assert!(!shell.state.section.on && shell.state.section.plane.is_some());
    assert!(shell.typing().is_none());
}

/// Shift pressed in the middle of a drag goes on from the value reached
/// at the finer rate: no jump.
#[test]
fn shift_during_a_drag_does_not_make_the_value_jump() {
    let (mut session, mut shell, mut window) = started();
    let (rail, _) = parts(&shell, "see-through");
    // From the knob (at 0, the rail's left end).
    let from = knob(&shell);
    let w = rail.width();
    window.frame(&mut shell, &mut session, vec![Event::PointerMoved(from)]);
    window.button(&mut shell, &mut session, from, true);
    for step in 1..=4 {
        let at = from + vec2(w * 0.05 * step as f32, 0.0);
        window.frame(&mut shell, &mut session, vec![Event::PointerMoved(at)]);
    }
    let reached = shell.state.transparency;
    assert!((reached - 0.18).abs() <= 0.005 + 1e-9, "{reached}");
    // Shift down, the pointer not moved: the value stays.
    window.modifiers = Modifiers::SHIFT;
    window.frame(&mut shell, &mut session, Vec::new());
    assert!((shell.state.transparency - reached).abs() < 1e-9);
    // A further fifth of the width at a tenth of the rate.
    let at = from + vec2(w * 0.4, 0.0);
    window.frame(&mut shell, &mut session, vec![Event::PointerMoved(at)]);
    let finer = shell.state.transparency - reached;
    assert!((finer - 0.018).abs() <= 0.0005 + 1e-9, "{finer}");
    // Shift up again: still no jump.
    let held = shell.state.transparency;
    window.modifiers = Modifiers::NONE;
    window.frame(&mut shell, &mut session, Vec::new());
    assert!((shell.state.transparency - held).abs() < 1e-9);
    window.button(&mut shell, &mut session, at, false);
}

/// The wheel over a slider that does not have the focus zooms as
/// anywhere in the viewport and leaves the slider; once the slider was
/// clicked it steps it instead, until a press elsewhere.
#[test]
fn the_wheel_steps_only_the_focused_field() {
    let (mut session, mut shell, mut window) = started();
    let at = knob(&shell);
    let wheel = |window: &mut Window, shell: &mut Shell, session: &mut Session| {
        window.frame(shell, session, vec![Event::PointerMoved(at)]);
        window.frame(
            shell,
            session,
            vec![Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: vec2(0.0, 1.0),
                phase: egui::TouchPhase::Move,
                modifiers: Modifiers::NONE,
            }],
        );
        for _ in 0..30 {
            window.frame(shell, session, Vec::new());
        }
    };
    let distance = shell.state.camera.distance;
    wheel(&mut window, &mut shell, &mut session);
    assert_eq!(shell.state.transparency, 0.0, "not focused: no step");
    assert!(shell.state.camera.distance < distance, "the wheel zoomed");
    // Its knob clicked (which leaves the value), then the wheel steps it.
    window.click(&mut shell, &mut session, at);
    assert_eq!(shell.state.transparency, 0.0);
    let distance = shell.state.camera.distance;
    wheel(&mut window, &mut shell, &mut session);
    assert!((shell.state.transparency - 0.05).abs() < 1e-9);
    assert_eq!(shell.state.camera.distance, distance, "no zoom");
    // A press elsewhere (the viewport) takes the focus away.
    let viewport = shell.viewport_rect().unwrap();
    window.click(
        &mut shell,
        &mut session,
        viewport.left_top() + vec2(30.0, 30.0),
    );
    wheel(&mut window, &mut shell, &mut session);
    assert!((shell.state.transparency - 0.05).abs() < 1e-9);
    assert!(shell.state.camera.distance < distance);
}

/// A pasted page is cut at the field's limit and its error repeats only
/// the start of it: nothing grows without bound.
#[test]
fn a_pasted_page_is_cut_and_its_error_is_short() {
    let (mut session, mut shell, mut window) = started();
    let (_, value) = parts(&shell, "see-through");
    window.click(&mut shell, &mut session, value.center());
    let page = "x".repeat(100_000);
    window.type_text(&mut shell, &mut session, &page);
    let typed = shell.typing().unwrap().text.chars().count();
    assert!(
        typed <= craft3d_ui_egui::shell::number_field::MAX_TYPED,
        "{typed}"
    );
    window.key(&mut shell, &mut session, Key::Enter);
    let error = shell.typing().unwrap().error.clone().unwrap();
    assert!(error.chars().count() <= 160, "{error}");
    assert_eq!(shell.state.transparency, 0.0);
}

/// Typing into Offset and then switching the section off (here through
/// `ui.set`, as an agent would): the field is gone, and so is what was
/// being typed into it and its focus.
#[test]
fn switching_the_section_off_ends_typing_into_offset() {
    let (mut session, mut shell, mut window) = started();
    let toggle = shell.controls_layout().unwrap().section_toggle.unwrap();
    window.click(&mut shell, &mut session, toggle.center());
    window.frame(&mut shell, &mut session, Vec::new());
    let (_, offset) = parts(&shell, "offset");
    window.click(&mut shell, &mut session, offset.center());
    assert_eq!(shell.typing().map(|t| t.field), Some("offset"));
    shell
        .handle_method(
            &mut session,
            "ui.set",
            Some(&serde_json::json!({"section": {"on": false}})),
        )
        .unwrap()
        .unwrap();
    window.frame(&mut shell, &mut session, Vec::new());
    window.frame(&mut shell, &mut session, Vec::new());
    assert!(shell.typing().is_none(), "{:?}", shell.typing());
    // The wheel over where the field was does not step anything.
    assert!(!shell.state.section.on);
}
