//! Undo and Redo in the window: Ctrl+Z, Ctrl+Shift+Z and Ctrl+Y ask the
//! host for `edit.undo` and `edit.redo` of the active document, only
//! when there is a step to undo or redo and not while a text field has
//! the keyboard; the Edit menu names the step.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::{EditAction, RendererChoice, Shell};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, vec2};
use serde_json::{Value, json};

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

struct Window {
    ctx: egui::Context,
    time: f64,
}

const CTRL: Modifiers = Modifiers {
    alt: false,
    ctrl: true,
    shift: false,
    mac_cmd: false,
    command: true,
};

impl Window {
    fn frame(&mut self, shell: &mut Shell, session: &mut Session, events: Vec<Event>) {
        self.time += 1.0 / 60.0;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 760.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
        output.textures_delta.clear();
    }

    /// Presses `key` with `modifiers`; returns what the shell asked for.
    fn press(
        &mut self,
        shell: &mut Shell,
        session: &mut Session,
        key: Key,
        modifiers: Modifiers,
    ) -> Option<EditAction> {
        for pressed in [true, false] {
            self.frame(
                shell,
                session,
                vec![
                    Event::ModifiersChanged(modifiers),
                    Event::Key {
                        key,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers,
                    },
                ],
            );
        }
        self.frame(
            shell,
            session,
            vec![Event::ModifiersChanged(Modifiers::NONE)],
        );
        shell.take_edit_action()
    }

    fn double_click(&mut self, shell: &mut Shell, session: &mut Session, at: Pos2) {
        self.frame(shell, session, vec![Event::PointerMoved(at)]);
        self.time += 1.0;
        for _ in 0..2 {
            for pressed in [true, false] {
                self.frame(
                    shell,
                    session,
                    vec![Event::PointerButton {
                        pos: at,
                        button: PointerButton::Primary,
                        pressed,
                        modifiers: Modifiers::NONE,
                    }],
                );
            }
        }
        self.frame(shell, session, Vec::new());
        self.frame(shell, session, Vec::new());
    }
}

fn started(session: &mut Session) -> (Shell, Window) {
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window {
        ctx: egui::Context::default(),
        time: 0.0,
    };
    for _ in 0..4 {
        window.frame(&mut shell, session, Vec::new());
    }
    (shell, window)
}

/// Runs what the shell asked for, as a host does.
fn host_runs(shell: &mut Shell, session: &mut Session, window: &mut Window, action: EditAction) {
    run(session, action.command(), json!({}));
    window.frame(shell, session, Vec::new());
}

#[test]
fn ctrl_z_undoes_and_ctrl_shift_z_or_ctrl_y_redo_the_last_step() {
    let mut session = Session::new(Workspace::none());
    run(&mut session, "file.new", json!({"units": "mm"}));
    run(
        &mut session,
        "feature.block",
        json!({"name": "Plate", "size": [60, 40, 8]}),
    );
    let (mut shell, mut window) = started(&mut session);
    let status = session.document_status().unwrap();
    assert!(status.undo.is_some() && status.redo.is_none(), "{status:?}");
    let menu = EditAction::Undo.menu_text(Some(&status));
    assert!(menu.starts_with("Undo ") && menu.len() > 5, "{menu}");
    assert_eq!(EditAction::Redo.menu_text(Some(&status)), "Redo");

    // Nothing to redo yet: Ctrl+Shift+Z asks for nothing (and is not an
    // Undo either).
    let asked = window.press(&mut shell, &mut session, Key::Z, CTRL | Modifiers::SHIFT);
    assert_eq!(asked, None);

    let asked = window.press(&mut shell, &mut session, Key::Z, CTRL);
    assert_eq!(asked, Some(EditAction::Undo));
    host_runs(&mut shell, &mut session, &mut window, EditAction::Undo);
    let status = session.document_status().unwrap();
    assert!(status.redo.is_some(), "{status:?}");
    assert!(
        session
            .execute("document.inspect", &json!({"measures": false}))
            .unwrap()["features"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    // Nothing left to undo (file.new is not a step): Ctrl+Z asks for
    // nothing.
    if status.undo.is_none() {
        assert_eq!(window.press(&mut shell, &mut session, Key::Z, CTRL), None);
    }

    let asked = window.press(&mut shell, &mut session, Key::Z, CTRL | Modifiers::SHIFT);
    assert_eq!(asked, Some(EditAction::Redo));
    host_runs(&mut shell, &mut session, &mut window, EditAction::Redo);
    assert!(session.document_status().unwrap().redo.is_none());

    // Ctrl+Y redoes too.
    host_runs(&mut shell, &mut session, &mut window, EditAction::Undo);
    let asked = window.press(&mut shell, &mut session, Key::Y, CTRL);
    assert_eq!(asked, Some(EditAction::Redo));
}

#[test]
fn without_a_document_or_a_step_the_keys_ask_for_nothing() {
    let mut session = Session::new(Workspace::none());
    let (mut shell, mut window) = started(&mut session);
    assert_eq!(window.press(&mut shell, &mut session, Key::Z, CTRL), None);
    assert_eq!(window.press(&mut shell, &mut session, Key::Y, CTRL), None);
    run(&mut session, "file.new", json!({"units": "mm"}));
    window.frame(&mut shell, &mut session, Vec::new());
    assert_eq!(session.document_status().unwrap().undo, None);
    assert_eq!(window.press(&mut shell, &mut session, Key::Z, CTRL), None);
    // Z alone is no shortcut.
    run(
        &mut session,
        "feature.block",
        json!({"name": "Plate", "size": [60, 40, 8]}),
    );
    window.frame(&mut shell, &mut session, Vec::new());
    assert_eq!(
        window.press(&mut shell, &mut session, Key::Z, Modifiers::NONE),
        None
    );
}

#[test]
fn ctrl_z_while_typing_into_a_field_is_the_fields_own() {
    let mut session = Session::new(Workspace::none());
    run(&mut session, "file.new", json!({"units": "mm"}));
    run(
        &mut session,
        "feature.block",
        json!({"name": "Plate", "size": [60, 40, 8]}),
    );
    let (mut shell, mut window) = started(&mut session);
    let field = shell.controls_layout().unwrap().see_through.unwrap();
    window.double_click(&mut shell, &mut session, field.center());
    assert!(shell.typing().is_some(), "typing into See-through");
    assert_eq!(window.press(&mut shell, &mut session, Key::Z, CTRL), None);
    assert_eq!(window.press(&mut shell, &mut session, Key::Y, CTRL), None);
    // Nothing was undone.
    assert!(session.document_status().unwrap().undo.is_some());
}
