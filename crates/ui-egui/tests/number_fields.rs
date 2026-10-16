//! The number fields over the viewport (See-through, the section's
//! Offset), driven by synthetic input as a person drives them: a drag
//! (plain, with Shift finer, with Ctrl in steps), the arrows at the ends,
//! and typing an expression with units, evaluated with the document's
//! variables; text that is not a number shows why and changes nothing,
//! Escape gives up.

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

fn field(shell: &Shell, name: &str) -> Rect {
    let layout = shell.controls_layout().unwrap();
    match name {
        "see-through" => layout.see_through.unwrap(),
        _ => layout.offset.unwrap(),
    }
}

#[test]
fn a_drag_shift_finer_ctrl_in_steps_and_the_arrows() {
    let (mut session, mut shell, mut window) = started();
    let rect = field(&shell, "see-through");
    let middle = rect.center();
    let camera = shell.state.camera;
    // A drag of a quarter of the width: a quarter of 0 to 0.9.
    window.drag(&mut shell, &mut session, middle, rect.width() * 0.25);
    let t = shell.state.transparency;
    assert!((t - 0.225).abs() < 1e-3, "{t}");
    // With Shift ten times finer: a quarter of the width more is 0.0225.
    window.modifiers = Modifiers::SHIFT;
    window.drag(&mut shell, &mut session, middle, rect.width() * 0.25);
    let finer = shell.state.transparency - t;
    assert!((finer - 0.0225).abs() < 1e-3, "{finer}");
    // With Ctrl on the steps of 5 %.
    window.modifiers = Modifiers::CTRL;
    window.drag(&mut shell, &mut session, middle, rect.width() * 0.1);
    let stepped = shell.state.transparency;
    assert!(
        ((stepped / 0.05).round() * 0.05 - stepped).abs() < 1e-9,
        "{stepped}"
    );
    window.modifiers = Modifiers::NONE;
    // The arrows at the ends: a step down, a step up.
    let before = shell.state.transparency;
    window.click(&mut shell, &mut session, pos2(rect.left() + 6.0, middle.y));
    assert!((shell.state.transparency - (before - 0.05)).abs() < 1e-9);
    window.click(&mut shell, &mut session, pos2(rect.right() - 6.0, middle.y));
    assert!((shell.state.transparency - before).abs() < 1e-9);
    // None of it moved the camera or picked.
    assert_eq!(shell.state.camera, camera);
    assert_eq!(shell.state.picked, None);
}

#[test]
fn typed_expressions_with_units_set_the_value_and_bad_text_says_why() {
    let (mut session, mut shell, mut window) = started();
    // The see-through field: a percentage, an expression.
    let rect = field(&shell, "see-through");
    window.double_click(&mut shell, &mut session, rect.center());
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
    let rect = field(&shell, "offset");
    window.double_click(&mut shell, &mut session, rect.center());
    assert_eq!(shell.typing().unwrap().field, "offset");
    window.type_text(&mut shell, &mut session, "width / 2 - 3 mm");
    window.key(&mut shell, &mut session, Key::Enter);
    assert!(shell.typing().is_none(), "{:?}", shell.typing());
    assert!((shell.state.section.offset - 17.0).abs() < 1e-9);

    // Text that is no number: the field says why and nothing changes.
    let before = shell.state.section.clone();
    window.double_click(&mut shell, &mut session, rect.center());
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
    window.double_click(&mut shell, &mut session, rect.center());
    window.type_text(&mut shell, &mut session, "F");
    window.key(&mut shell, &mut session, Key::F);
    assert_eq!(shell.state.camera, camera);
    window.key(&mut shell, &mut session, Key::Escape);
    assert_eq!(shell.state.picked, None);
}

#[test]
fn the_section_controls_are_dimmed_and_inert_while_it_is_off() {
    let (mut session, mut shell, mut window) = started();
    assert!(!shell.state.section.on);
    let layout = shell.controls_layout().unwrap();
    // The plate and its controls are there, but only the switch answers.
    assert!(layout.section_plate.is_some() && layout.offset.is_some());
    let names: Vec<&str> = shell.section_hits().iter().map(|(n, _)| *n).collect();
    assert_eq!(names, ["toggle"]);
    let before = shell.state.clone();
    for rect in [
        layout.axes.unwrap(),
        layout.offset.unwrap(),
        layout.side.unwrap(),
    ] {
        window.click(&mut shell, &mut session, rect.center());
        window.double_click(&mut shell, &mut session, rect.center());
        window.drag(&mut shell, &mut session, rect.center(), 40.0);
    }
    assert!(shell.typing().is_none());
    assert_eq!(shell.state.section, before.section);
    assert_eq!(shell.state.picked, None);
}
