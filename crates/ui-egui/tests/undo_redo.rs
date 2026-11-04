//! Undo and Redo in the window: Ctrl+Z, Ctrl+Shift+Z and Ctrl+Y ask the
//! host for `edit.undo` and `edit.redo` of the active document, queued
//! with that document's id and judged when they run ([`run_edit`]); not
//! while a text field has the keyboard, a modal dialog or the question
//! before closing is open, or the Parameters form drags a value; the
//! Edit menu names the step.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::shell::run_edit;
use craft3d_ui_egui::{EditAction, EditRequest, RendererChoice, Shell};
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
        let asked = shell.take_edit_action();
        assert_eq!(shell.take_edit_action(), None, "one press, one request");
        if let Some(request) = asked {
            assert_eq!(
                Some(request.document),
                session.document_status().map(|s| s.id),
                "for the active document"
            );
        }
        asked.map(|request| request.action)
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
    let document = session.document_status().unwrap().id;
    run_edit(session, EditRequest { action, document }).unwrap();
    window.frame(shell, session, Vec::new());
}

fn plate() -> Session {
    let mut session = Session::new(Workspace::none());
    run(&mut session, "file.new", json!({"units": "mm"}));
    run(
        &mut session,
        "feature.block",
        json!({"name": "Plate", "size": [60, 40, 8]}),
    );
    session
}

#[test]
fn ctrl_z_undoes_and_ctrl_shift_z_or_ctrl_y_redo_the_last_step() {
    let mut session = plate();
    let (mut shell, mut window) = started(&mut session);
    let status = session.document_status().unwrap();
    assert!(status.undo.is_some() && status.redo.is_none(), "{status:?}");
    let menu = EditAction::Undo.menu_text(Some(&status));
    assert!(menu.starts_with("Undo ") && menu.len() > 5, "{menu}");
    assert_eq!(EditAction::Redo.menu_text(Some(&status)), "Redo");

    // Ctrl+Shift+Z is a Redo, never an Undo; with nothing to redo the
    // engine says so when it runs, and nothing changes.
    let asked = window.press(&mut shell, &mut session, Key::Z, CTRL | Modifiers::SHIFT);
    assert_eq!(asked, Some(EditAction::Redo));
    let refused = run_edit(
        &mut session,
        EditRequest {
            action: EditAction::Redo,
            document: status.id,
        },
    )
    .unwrap_err();
    assert!(refused.contains("nothing to redo"), "{refused}");
    assert_eq!(session.document_status().unwrap().undo, status.undo);

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
fn presses_queue_in_order_and_each_runs_on_the_session_as_it_is_then() {
    let mut session = plate();
    run(
        &mut session,
        "feature.block",
        json!({"name": "Post", "size": [10, 10, 30]}),
    );
    let (mut shell, mut window) = started(&mut session);
    let id = session.document_status().unwrap().id;
    // Three presses before the host runs any: none lost to the status
    // of the frame they were pressed in.
    for key in [Key::Z, Key::Z, Key::Y] {
        let modifiers = CTRL;
        for pressed in [true, false] {
            window.frame(
                &mut shell,
                &mut session,
                vec![Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers,
                }],
            );
        }
    }
    let mut asked = Vec::new();
    while let Some(request) = shell.take_edit_action() {
        assert_eq!(request.document, id);
        asked.push(request.action);
        run_edit(&mut session, request).unwrap();
    }
    assert_eq!(
        asked,
        [EditAction::Undo, EditAction::Undo, EditAction::Redo]
    );
    let features = run(&mut session, "document.inspect", json!({"measures": false}))["features"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(features, 1, "two undone, one redone");
    // A request for a document that is no longer the active one is
    // refused, and changes nothing.
    run(&mut session, "file.new", json!({"units": "mm"}));
    run(&mut session, "feature.block", json!({"size": [1, 1, 1]}));
    let other = run_edit(
        &mut session,
        EditRequest {
            action: EditAction::Undo,
            document: id,
        },
    )
    .unwrap_err();
    assert!(other.contains("another document"), "{other}");
    assert!(session.document_status().unwrap().undo.is_some());
    // The queue is bounded.
    for _ in 0..100 {
        shell.request_edit_action(EditAction::Undo, id);
    }
    let mut queued = 0;
    while shell.take_edit_action().is_some() {
        queued += 1;
    }
    assert_eq!(queued, craft3d_ui_egui::shell::MAX_EDIT_QUEUE);
}

#[test]
fn without_a_document_the_keys_ask_for_nothing() {
    let mut session = Session::new(Workspace::none());
    let (mut shell, mut window) = started(&mut session);
    assert_eq!(window.press(&mut shell, &mut session, Key::Z, CTRL), None);
    assert_eq!(window.press(&mut shell, &mut session, Key::Y, CTRL), None);
    let error = run_edit(
        &mut session,
        EditRequest {
            action: EditAction::Undo,
            document: 1,
        },
    )
    .unwrap_err();
    assert!(error.contains("closed"), "{error}");
    run(&mut session, "file.new", json!({"units": "mm"}));
    run(
        &mut session,
        "feature.block",
        json!({"name": "Plate", "size": [60, 40, 8]}),
    );
    window.frame(&mut shell, &mut session, Vec::new());
    // Z alone is no shortcut.
    assert_eq!(
        window.press(&mut shell, &mut session, Key::Z, Modifiers::NONE),
        None
    );
}

#[test]
fn ctrl_z_while_typing_into_a_field_is_the_fields_own() {
    let mut session = plate();
    let (mut shell, mut window) = started(&mut session);
    // Typing begins with a click on the slider's value (2026-11-04: the
    // one slider; the number field took a double click).
    let field = shell
        .slider_hits()
        .into_iter()
        .find(|(n, _, _)| *n == "see-through")
        .and_then(|(_, _, value)| value)
        .unwrap();
    window.double_click(&mut shell, &mut session, field.center());
    assert!(shell.typing().is_some(), "typing into See-through");
    assert_eq!(window.press(&mut shell, &mut session, Key::Z, CTRL), None);
    assert_eq!(window.press(&mut shell, &mut session, Key::Y, CTRL), None);
    // Nothing was undone.
    assert!(session.document_status().unwrap().undo.is_some());
}

#[test]
fn not_under_a_dialog_or_the_question_before_closing() {
    let mut session = plate();
    let (mut shell, mut window) = started(&mut session);
    shell.show_error("file.open", "no such file");
    window.frame(&mut shell, &mut session, Vec::new());
    assert_eq!(window.press(&mut shell, &mut session, Key::Z, CTRL), None);
    // Nor the File keys: Ctrl+N under the error dialog asks for nothing.
    let _ = window.press(&mut shell, &mut session, Key::N, CTRL);
    assert_eq!(shell.take_file_action(), None);
    shell.close_error_dialog();
    window.frame(&mut shell, &mut session, Vec::new());
    // The question before closing the changed document.
    shell.request_close(0);
    window.frame(&mut shell, &mut session, Vec::new());
    window.frame(&mut shell, &mut session, Vec::new());
    assert!(shell.closing().is_some());
    assert_eq!(window.press(&mut shell, &mut session, Key::Z, CTRL), None);
    let _ = window.press(&mut shell, &mut session, Key::S, CTRL);
    assert_eq!(shell.take_file_action(), None);
    assert!(session.document_status().unwrap().undo.is_some());
}

#[test]
fn not_while_the_parameters_form_drags_or_commits_a_value() {
    let mut session = plate();
    run(
        &mut session,
        "variable.set",
        json!({"name": "w", "expr": "40 mm"}),
    );
    run(
        &mut session,
        "variable.publish",
        json!({"name": "w", "min": "10 mm", "max": "90 mm"}),
    );
    let (mut shell, mut window) = started(&mut session);
    // A commit of the form the host runs now (as ui.param on the
    // desktop): the form is busy until it comes back.
    let ask = shell
        .ui_param_ask(&mut session, Some(&json!({"name": "w", "value": 60})))
        .unwrap();
    assert!(shell.parameters_busy());
    assert_eq!(window.press(&mut shell, &mut session, Key::Z, CTRL), None);
    let done = session
        .execute(ask.command.command, &ask.command.params)
        .map_err(|e| e.message().to_string());
    shell.command_done(done);
    window.frame(&mut shell, &mut session, Vec::new());
    assert!(!shell.parameters_busy());
    // Idle again: the keys work.
    assert_eq!(
        window.press(&mut shell, &mut session, Key::Z, CTRL),
        Some(EditAction::Undo)
    );
}
