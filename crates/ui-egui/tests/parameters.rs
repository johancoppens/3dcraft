//! The Parameters tab of a Jig, driven by synthetic input as a person
//! drives it: a Jig opens on it, a drag commits `param.set` as it goes
//! (debounced, one undo step for the whole drag) on the desktop and once
//! on release on the web, typed values are clamped before they are sent
//! and the form says so, a value that breaks the model marks the field and
//! keeps the last good model in the viewport until Back to last good, and
//! Reset all and a boolean's toggle commit at once. The host's part is
//! played here: take the shell's command, run it, hand back the result.

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

/// A plate `w` × `d` × 5 mm with its four vertical edges rounded by `r`,
/// all three public, and a boolean that drives nothing. A radius of half
/// the depth or more makes the fillet fail.
fn jig() -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    for (name, expr) in [("w", "40 mm"), ("d", "20 mm"), ("r", "2 mm"), ("lid", "1")] {
        run(&mut s, "variable.set", json!({"name": name, "expr": expr}));
    }
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": ["w", "d", "5 mm"]}),
    );
    run(
        &mut s,
        "feature.fillet",
        json!({"name": "Round", "radius": "r", "edges": [
            ["F1:face(2)", "F1:face(5)"], ["F1:face(2)", "F1:face(4)"],
            ["F1:face(3)", "F1:face(4)"], ["F1:face(3)", "F1:face(5)"]]}),
    );
    run(
        &mut s,
        "variable.publish",
        json!({"name": "w", "label": "Width", "group": "Size", "min": "20 mm", "max": "100 mm", "step": "1 mm", "order": 1}),
    );
    run(
        &mut s,
        "variable.publish",
        json!({"name": "d", "label": "Depth", "group": "Size", "min": "10 mm", "max": "60 mm", "order": 2}),
    );
    run(
        &mut s,
        "variable.publish",
        json!({"name": "r", "label": "Radius", "description": "Corner radius", "min": "0.5 mm", "max": "30 mm", "order": 3}),
    );
    run(
        &mut s,
        "variable.publish",
        json!({"name": "lid", "label": "Lid", "kind": "boolean", "order": 4}),
    );
    run(
        &mut s,
        "document.info",
        json!({"title": "Plate", "author": "Tests", "licence": "MIT"}),
    );
    s
}

struct Window {
    ctx: egui::Context,
    time: f64,
    /// The commands the shell asked for and the host ran, in order.
    ran: Vec<Value>,
}

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
        // The host, between frames.
        if let Some(asked) = shell.take_command() {
            self.ran.push(asked.params.clone());
            let result = session
                .execute(asked.command, &asked.params)
                .map_err(|e| e.message().to_string());
            shell.command_done(result);
        }
    }

    fn idle(&mut self, shell: &mut Shell, session: &mut Session, frames: usize) {
        for _ in 0..frames {
            self.frame(shell, session, Vec::new());
        }
    }

    fn button(&mut self, shell: &mut Shell, session: &mut Session, at: Pos2, pressed: bool) {
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

    /// A drag of `by` points to the right over `steps` frames.
    fn drag(&mut self, shell: &mut Shell, session: &mut Session, from: Pos2, by: f32, steps: u32) {
        self.frame(shell, session, vec![Event::PointerMoved(from)]);
        self.button(shell, session, from, true);
        for step in 1..=steps {
            let at = from + vec2(by * step as f32 / steps as f32, 0.0);
            self.frame(shell, session, vec![Event::PointerMoved(at)]);
        }
        self.button(shell, session, from + vec2(by, 0.0), false);
        self.idle(shell, session, 4);
    }

    fn click(&mut self, shell: &mut Shell, session: &mut Session, at: Pos2) {
        self.frame(shell, session, vec![Event::PointerMoved(at)]);
        self.button(shell, session, at, true);
        self.button(shell, session, at, false);
        self.idle(shell, session, 4);
    }

    fn key(&mut self, shell: &mut Shell, session: &mut Session, key: Key) {
        for pressed in [true, false] {
            self.frame(
                shell,
                session,
                vec![Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                }],
            );
        }
        self.idle(shell, session, 4);
    }
}

fn started(session: Session) -> (Session, Shell, Window) {
    let mut session = session;
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    shell.set_parameters_single_thread(false);
    let mut window = Window {
        ctx: egui::Context::default(),
        time: 0.0,
        ran: Vec::new(),
    };
    window.idle(&mut shell, &mut session, 4);
    (session, shell, window)
}

fn hit(shell: &Shell, name: &str) -> Rect {
    shell
        .parameter_hits()
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, r)| *r)
        .unwrap_or_else(|| panic!("no `{name}` in {:?}", shell.parameter_hits()))
}

fn form(shell: &mut Shell, session: &mut Session) -> Value {
    shell
        .handle_method(session, "ui.get", None)
        .unwrap()
        .unwrap()["parameters"]
        .clone()
}

fn value(session: &mut Session, name: &str) -> f64 {
    let list = run(session, "param.list", json!({}));
    list["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == name)
        .unwrap()["value"]
        .as_f64()
        .unwrap()
}

#[test]
fn a_jig_opens_on_its_parameters_and_a_plain_document_on_its_features() {
    let (mut session, mut shell, mut window) = started(jig());
    let shown = form(&mut shell, &mut session);
    assert_eq!(shown["jig"], true);
    assert_eq!(shown["tab"], "parameters");
    assert_eq!(shown["count"], 4);
    for name in ["w", "d", "r", "lid", "Reset all"] {
        hit(&shell, name);
    }
    // A plain document has no Parameters tab: the feature list, whatever
    // the state asks for.
    run(&mut session, "file.new", json!({}));
    window.idle(&mut shell, &mut session, 3);
    let plain = form(&mut shell, &mut session);
    assert_eq!(plain["jig"], false);
    assert_eq!(plain["tab"], "features");
    assert!(shell.parameter_hits().is_empty());
    shell
        .handle_method(
            &mut session,
            "ui.set",
            Some(&json!({"parameters": {"open": true}})),
        )
        .unwrap()
        .unwrap();
    window.idle(&mut shell, &mut session, 2);
    assert_eq!(form(&mut shell, &mut session)["tab"], "features");
    assert!(shell.parameter_hits().is_empty());
    // The Jig shows its parameters again, and keeps the Features tab
    // when that was chosen.
    run(&mut session, "file.activate", json!({"document": 0}));
    window.idle(&mut shell, &mut session, 3);
    assert_eq!(form(&mut shell, &mut session)["tab"], "parameters");
    shell
        .handle_method(
            &mut session,
            "ui.set",
            Some(&json!({"parameters": {"open": false}})),
        )
        .unwrap()
        .unwrap();
    window.idle(&mut shell, &mut session, 2);
    assert_eq!(form(&mut shell, &mut session)["tab"], "features");
    run(&mut session, "file.activate", json!({"document": 1}));
    window.idle(&mut shell, &mut session, 2);
    run(&mut session, "file.activate", json!({"document": 0}));
    window.idle(&mut shell, &mut session, 2);
    assert_eq!(form(&mut shell, &mut session)["tab"], "features");
}

#[test]
fn a_drag_commits_as_it_goes_folded_into_one_undo_step() {
    let (mut session, mut shell, mut window) = started(jig());
    let field = hit(&shell, "w");
    let before = value(&mut session, "w");
    // Half a second of dragging to the right: about three commits.
    window.drag(
        &mut shell,
        &mut session,
        pos2(field.center().x, field.center().y),
        field.width() * 0.3,
        30,
    );
    let ran = window.ran.clone();
    assert!(ran.len() >= 2, "commits during the drag: {ran:?}");
    assert_eq!(ran[0]["merge"], false, "the first starts an undo step");
    assert!(
        ran[1..].iter().all(|c| c["merge"] == true),
        "the rest fold into it: {ran:?}"
    );
    let after = value(&mut session, "w");
    assert!(after > before + 10.0, "{before} -> {after}");
    assert_eq!(after, after.round(), "on its step of 1 mm");
    let last = ran.last().unwrap()["values"]["w"].as_f64().unwrap();
    assert_eq!(last, after, "the value at the release is the last sent");
    assert_eq!(form(&mut shell, &mut session)["busy"], false);
    // One undo takes the whole drag back.
    run(&mut session, "edit.undo", json!({}));
    assert_eq!(value(&mut session, "w"), before);
}

#[test]
fn on_the_web_a_drag_commits_once_when_it_ends() {
    let (mut session, mut shell, mut window) = started(jig());
    shell.set_parameters_single_thread(true);
    let field = hit(&shell, "w");
    window.drag(
        &mut shell,
        &mut session,
        field.center(),
        field.width() * 0.3,
        30,
    );
    assert_eq!(window.ran.len(), 1, "{:?}", window.ran);
    assert_eq!(window.ran[0]["merge"], false);
    assert!(value(&mut session, "w") > 50.0);
}

#[test]
fn a_typed_value_out_of_range_is_clamped_and_said() {
    let (mut session, mut shell, mut window) = started(jig());
    let field = hit(&shell, "w");
    // Enter over the field starts typing; the text replaces the value.
    window.frame(
        &mut shell,
        &mut session,
        vec![Event::PointerMoved(field.center())],
    );
    window.key(&mut shell, &mut session, Key::Enter);
    window.frame(
        &mut shell,
        &mut session,
        vec![Event::Text("150".to_string())],
    );
    window.key(&mut shell, &mut session, Key::Enter);
    assert_eq!(window.ran.len(), 1, "{:?}", window.ran);
    assert_eq!(window.ran[0]["values"]["w"], 100.0);
    assert_eq!(value(&mut session, "w"), 100.0);
    let shown = form(&mut shell, &mut session);
    assert_eq!(
        shown["note"],
        json!({"name": "w", "text": "clamped to 100 mm"})
    );
    assert_eq!(shown["trouble"], Value::Null);
}

#[test]
fn a_value_that_breaks_the_model_is_said_over_the_viewport_until_back() {
    let (mut session, mut shell, mut window) = started(jig());
    let good = shell.shown_scene_arc().unwrap().revision;
    // Through the control channel, as the form does it: 15 mm rounds the
    // 20 mm side away.
    let answer = shell
        .handle_method(
            &mut session,
            "ui.param",
            Some(&json!({"name": "r", "value": 15})),
        )
        .unwrap()
        .unwrap();
    assert_eq!(answer["sent"], 15.0);
    assert!(answer["result"]["failed"].is_array(), "{answer}");
    window.idle(&mut shell, &mut session, 3);
    let shown = form(&mut shell, &mut session);
    assert_eq!(shown["trouble"]["name"], "r");
    assert_eq!(shown["trouble"]["broke"], true);
    assert!(
        shown["trouble"]["message"]
            .as_str()
            .unwrap()
            .starts_with("Round:"),
        "{shown}"
    );
    // The viewport shows the document as it is: the failed state, with
    // the banner over it.
    assert_ne!(shell.shown_scene_arc().unwrap().revision, good);
    // Back to last good sets the radius back.
    let back = hit(&shell, "Back to last good");
    window.click(&mut shell, &mut session, back.center());
    // The values, evaluated, and only those that changed.
    assert_eq!(
        window.ran.last().unwrap(),
        &json!({"values": {"r": 2.0}, "clamp": true})
    );
    assert_eq!(value(&mut session, "r"), 2.0);
    let shown = form(&mut shell, &mut session);
    assert_eq!(shown["trouble"], Value::Null, "{shown}");
    // Typed units the form cannot read go to param.set as they are; what
    // it refuses is said at the field, and nothing is held.
    let field = hit(&shell, "r");
    window.frame(
        &mut shell,
        &mut session,
        vec![Event::PointerMoved(field.center())],
    );
    window.key(&mut shell, &mut session, Key::Enter);
    window.frame(
        &mut shell,
        &mut session,
        vec![Event::Text("3 kg".to_string())],
    );
    window.key(&mut shell, &mut session, Key::Enter);
    assert_eq!(window.ran.last().unwrap()["values"]["r"], "3 kg");
    let shown = form(&mut shell, &mut session);
    assert_eq!(shown["trouble"]["name"], "r", "{shown}");
    assert_eq!(shown["trouble"]["broke"], false, "{shown}");
    assert!(
        !shell
            .parameter_hits()
            .iter()
            .any(|(n, _)| n == "Back to last good"),
        "no banner for a refusal"
    );
    assert_eq!(value(&mut session, "r"), 2.0, "nothing changed");
    let answer = shell
        .handle_method(
            &mut session,
            "ui.param",
            Some(&json!({"name": "r", "value": "ten"})),
        )
        .unwrap();
    assert!(answer.is_err(), "{answer:?}");
}

#[test]
fn reset_all_and_a_toggle_commit_at_once() {
    let (mut session, mut shell, mut window) = started(jig());
    run(
        &mut session,
        "param.set",
        json!({"values": {"w": "60 mm", "d": "30 mm"}}),
    );
    window.idle(&mut shell, &mut session, 3);
    let lid = hit(&shell, "lid");
    window.click(
        &mut shell,
        &mut session,
        pos2(lid.left() + 8.0, lid.center().y),
    );
    assert_eq!(window.ran.last().unwrap()["values"]["lid"], 0);
    assert_eq!(value(&mut session, "lid"), 0.0);
    let reset = hit(&shell, "Reset all");
    window.click(&mut shell, &mut session, reset.center());
    assert_eq!(
        window.ran.last().unwrap(),
        &json!({"reset": true, "clamp": true})
    );
    assert_eq!(value(&mut session, "w"), 40.0);
    assert_eq!(value(&mut session, "d"), 20.0);
    assert_eq!(value(&mut session, "lid"), 1.0);
}

/// A value that would put another parameter outside a range that depends
/// on it moves that one into its new range in the same `param.set`
/// (`clamp`), and the note under the field says so: typed in the form and
/// through `ui.param` (A1 of the R1 finishing round: the GT2 pulley's
/// bore when its teeth go down).
#[test]
fn a_value_that_narrows_another_ones_range_clamps_it_and_says_so() {
    let mut s = jig();
    run(
        &mut s,
        "variable.publish",
        json!({"name": "r", "max": "d / 2 - 0.5 mm"}),
    );
    run(&mut s, "param.set", json!({"values": {"r": "8 mm"}}));
    let (mut session, mut shell, mut window) = started(s);
    let field = hit(&shell, "d");
    window.frame(
        &mut shell,
        &mut session,
        vec![Event::PointerMoved(field.center())],
    );
    window.key(&mut shell, &mut session, Key::Enter);
    window.frame(
        &mut shell,
        &mut session,
        vec![Event::Text("10".to_string())],
    );
    window.key(&mut shell, &mut session, Key::Enter);
    assert_eq!(window.ran.len(), 1, "{:?}", window.ran);
    assert_eq!(window.ran[0]["clamp"], true);
    assert_eq!(value(&mut session, "d"), 10.0);
    assert_eq!(value(&mut session, "r"), 4.5);
    let shown = form(&mut shell, &mut session);
    assert_eq!(
        shown["note"],
        json!({"name": "d", "text": "r clamped to 4.5 mm"})
    );
    assert_eq!(shown["trouble"], Value::Null);
    // ui.param: a value clamped itself and another one clamped with it.
    run(
        &mut session,
        "param.set",
        json!({"values": {"d": "40 mm", "r": "15 mm"}}),
    );
    window.idle(&mut shell, &mut session, 2);
    let answer = shell
        .handle_method(
            &mut session,
            "ui.param",
            Some(&json!({"name": "d", "value": 2})),
        )
        .unwrap()
        .unwrap();
    assert_eq!(answer["sent"], 10.0);
    assert_eq!(
        answer["note"], "clamped to 10 mm; r clamped to 4.5 mm",
        "{answer}"
    );
    assert_eq!(
        answer["result"]["clamped"],
        json!([{"name": "r", "expr": "4.5 mm"}])
    );
    assert_eq!(value(&mut session, "r"), 4.5);
    assert_eq!(
        answer["parameters"]["note"],
        json!({"name": "d", "text": "clamped to 10 mm; r clamped to 4.5 mm"})
    );
}

#[test]
fn ui_param_refuses_what_it_cannot_set_and_never_panics() {
    let (mut session, mut shell, _window) = started(jig());
    for params in [
        json!(null),
        json!([1]),
        json!({}),
        json!({"name": 3, "value": 1}),
        json!({"name": "nope", "value": 1}),
        json!({"name": "w"}),
        json!({"name": "w", "value": null}),
        json!({"name": "w", "value": {"x": 1}}),
        json!({"name": "w", "value": 1, "extra": true}),
        json!({"name": "w", "value": 1e308}),
    ] {
        let answer = shell
            .handle_method(&mut session, "ui.param", Some(&params))
            .unwrap();
        // 1e308 is a number: clamped to the range, not refused.
        if params["value"] == json!(1e308) && params.get("extra").is_none() {
            assert_eq!(answer.unwrap()["sent"], 100.0);
        } else {
            assert!(answer.is_err(), "{params}: {answer:?}");
        }
    }
    // Without a document there is nothing to set.
    let mut empty = Session::new(Workspace::none());
    let answer = shell
        .handle_method(
            &mut empty,
            "ui.param",
            Some(&json!({"name": "w", "value": 1})),
        )
        .unwrap();
    assert!(answer.is_err());
}

#[test]
fn a_step_that_changes_nothing_shows_the_documents_value_again() {
    let (mut session, mut shell, mut window) = started(jig());
    run(
        &mut session,
        "param.set",
        json!({"values": {"w": "100 mm"}}),
    );
    window.idle(&mut shell, &mut session, 3);
    // The › at the right end of a field at its maximum: a commit that
    // changes nothing.
    let field = hit(&shell, "w");
    let arrow = pos2(field.right() - 4.0, field.center().y);
    window.click(&mut shell, &mut session, arrow);
    assert_eq!(window.ran.last().unwrap()["values"]["w"], 100.0);
    let shown = form(&mut shell, &mut session);
    assert_eq!(shown["live"], Value::Null, "{shown}");
    assert_eq!(shown["busy"], false);
    assert_eq!(shown["trouble"], Value::Null);
}

#[test]
fn a_result_for_another_document_is_dropped() {
    let (mut session, mut shell, mut window) = started(jig());
    let ask = shell
        .ui_param_ask(&mut session, Some(&json!({"name": "r", "value": 15})))
        .unwrap();
    assert_eq!(
        ask.command.params,
        json!({"values": {"r": 15.0}, "clamp": true})
    );
    // Another document is shown before the result comes back.
    run(&mut session, "file.new", json!({}));
    shell.command_done(Err("param.set: refused".to_string()));
    window.idle(&mut shell, &mut session, 2);
    let shown = form(&mut shell, &mut session);
    assert_eq!(shown["trouble"], Value::Null, "{shown}");
    assert_eq!(shown["busy"], false);
    // And back on the Jig nothing is said either.
    run(&mut session, "file.activate", json!({"document": 0}));
    window.idle(&mut shell, &mut session, 2);
    assert_eq!(form(&mut shell, &mut session)["trouble"], Value::Null);
}

#[test]
fn a_jig_in_use_offers_open_for_editing_and_only_its_parameters_change() {
    let mut session = Session::new(Workspace::memory());
    // The design, published in the session's memory and opened from there.
    let bytes = {
        let mut memory = Session::new(Workspace::memory());
        run(&mut memory, "file.new", json!({"units": "mm"}));
        run(
            &mut memory,
            "variable.set",
            json!({"name": "w", "expr": "40 mm"}),
        );
        run(
            &mut memory,
            "feature.block",
            json!({"size": ["w", "20 mm", "5 mm"]}),
        );
        run(
            &mut memory,
            "variable.publish",
            json!({"name": "w", "min": "20 mm", "max": "100 mm"}),
        );
        run(
            &mut memory,
            "jig.export",
            json!({"path": "plate.3dcraftjig"}),
        );
        memory.take_file("plate.3dcraftjig").unwrap()
    };
    session.put_file("plate.3dcraftjig", bytes).unwrap();
    run(
        &mut session,
        "file.open",
        json!({"path": "plate.3dcraftjig"}),
    );
    let (mut session, mut shell, mut window) = started(session);
    let shown = form(&mut shell, &mut session);
    assert_eq!(shown["inUse"], true, "{shown}");
    assert_eq!(shown["tab"], "parameters");
    // Its parameters change through the form.
    let answer = shell
        .handle_method(
            &mut session,
            "ui.param",
            Some(&json!({"name": "w", "value": 60})),
        )
        .unwrap()
        .unwrap();
    assert_eq!(answer["result"]["changed"], true, "{answer}");
    // Open for editing asks the host for a model of it. The button says
    // what the File menu's item says (sentence case, as the window's
    // other menu items and buttons; 2026-10-23).
    assert_eq!(
        craft3d_ui_egui::FileAction::OpenForEditing.label(),
        "Open for editing"
    );
    let edit = hit(&shell, "Open for editing");
    window.click(&mut shell, &mut session, edit.center());
    assert_eq!(
        shell.take_file_action(),
        Some(craft3d_ui_egui::FileAction::OpenForEditing)
    );
    // Anything but its parameters is refused, to agents too.
    let refused = session
        .execute("variable.set", &json!({"name": "w", "expr": "50 mm"}))
        .unwrap_err();
    assert!(refused.message().contains("this is a Jig"), "{refused}");
}
