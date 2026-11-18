//! The grammar the view's tools share (the UX round of 2026-11-18): the
//! status bar says what a click will do (one hint per state, the file by
//! its name, the selection's words never cut, `ui.get` › `hint`), and
//! Escape steps back once, in one order wherever the pointer is: a field
//! in the view, a list or popover, a joint held by hand, the chosen note,
//! the selection; never the section.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Access, Session, Workspace};
use craft3d_ui_egui::{RendererChoice, Shell, StatusSlot};
use egui::{Event, Pos2, vec2};
use serde_json::{Value, json};
use std::path::PathBuf;

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn call(shell: &mut Shell, session: &mut Session, method: &str, params: Value) -> Value {
    shell
        .handle_method(session, method, Some(&params))
        .unwrap_or_else(|| panic!("{method} is not a ui method"))
        .unwrap_or_else(|e| panic!("{method} {params}: {e}"))
}

/// A block of 60 × 40 × 20 with two holes of radius 3 through it.
fn part(session: &mut Session) {
    run(session, "file.new", json!({"units": "mm"}));
    run(
        session,
        "feature.block",
        json!({"size": [60, 40, 20], "name": "Block"}),
    );
    for (x, name) in [(15, "Left"), (45, "Right")] {
        run(
            session,
            "feature.cylinder",
            json!({"radius": 3, "height": 30, "position": [x, 20, -5], "name": name}),
        );
    }
    run(
        session,
        "feature.boolean",
        json!({"op": "subtract", "target": "Block", "tool": "Left", "name": "Once"}),
    );
    run(
        session,
        "feature.boolean",
        json!({"op": "subtract", "target": "Once", "tool": "Right", "name": "Plate"}),
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

    fn frame(&mut self, shell: &mut Shell, session: &mut Session, events: Vec<Event>) {
        self.time += 1.0 / 60.0;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, self.size)),
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
        for _ in 0..5000 {
            if !shell.measuring() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
            self.frame(shell, session, Vec::new());
        }
    }

    fn key(&mut self, shell: &mut Shell, session: &mut Session, key: egui::Key) {
        self.frame(
            shell,
            session,
            vec![Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        self.frames(shell, session, 2);
    }
}

fn select(session: &mut Session, kind: &str, filter: Value, limit: u64) {
    run(
        session,
        "select.query",
        json!({"kind": kind, "where": filter, "limit": limit, "then": "replace"}),
    );
}

/// V6: one hint for each state the window is in, after the selection's
/// words; `ui.get` says the hint shown; a hint given for a while (a
/// refusal's) takes its place and goes.
#[test]
fn the_status_bar_says_what_a_click_will_do() {
    let mut session = Session::new(Workspace::none());
    part(&mut session);
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1400.0, 900.0);
    window.frames(&mut shell, &mut session, 4);
    let hint = |shell: &Shell| shell.status_hint().map(str::to_string);
    assert_eq!(
        hint(&shell).as_deref(),
        Some("Click a face, an edge or a hole to see its size")
    );
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(
        got["hint"],
        "Click a face, an edge or a hole to see its size"
    );
    select(
        &mut session,
        "face",
        json!({"in": "Plate", "hole": true}),
        1,
    );
    window.frames(&mut shell, &mut session, 3);
    assert_eq!(
        hint(&shell).as_deref(),
        Some("Shift-click another to measure between · N writes a note")
    );
    assert_eq!(
        shell.status_shown(StatusSlot::Selection),
        Some("1 face selected")
    );
    select(
        &mut session,
        "face",
        json!({"in": "Plate", "hole": true}),
        2,
    );
    window.frames(&mut shell, &mut session, 3);
    assert_eq!(hint(&shell).as_deref(), Some("Esc clears"));
    select(&mut session, "face", json!({"in": "Plate"}), 3);
    window.frames(&mut shell, &mut session, 3);
    assert_eq!(hint(&shell).as_deref(), Some("two things give a measure"));
    // A hint given for four seconds takes the bar's place, then goes.
    shell.hint(
        "Select a face, an edge or a vertex first, then press N",
        4.0,
    );
    window.frames(&mut shell, &mut session, 2);
    assert_eq!(
        hint(&shell).as_deref(),
        Some("Select a face, an edge or a vertex first, then press N")
    );
    window.time += 5.0;
    window.frames(&mut shell, &mut session, 2);
    assert_eq!(hint(&shell).as_deref(), Some("two things give a measure"));
    // The pointer on the section's arrow.
    run(&mut session, "select.set", json!({"mode": "clear"}));
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"section": {"on": true, "plane": "yz", "offset": 30}}),
    );
    window.frames(&mut shell, &mut session, 3);
    let [a, b] = shell
        .section_handle()
        .expect("the handle")
        .frame
        .arrow
        .expect("seen side on");
    window.frame(
        &mut shell,
        &mut session,
        vec![Event::PointerMoved(a.lerp(b, 0.5))],
    );
    window.frames(&mut shell, &mut session, 1);
    assert_eq!(
        hint(&shell).as_deref(),
        Some("Drag to move the cut · double-click keeps the other half · Alt: no snapping")
    );
}

/// V6: the file shows by its name, its path is the hover; with two holes
/// selected the bar says *2 faces selected* whole at 1400 and at 1280
/// wide, however long the path; narrower than 800 the selection's words
/// alone.
#[test]
fn the_selection_is_never_cut_and_the_file_shows_by_its_name() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("view-grammar");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut session = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir),
    });
    part(&mut session);
    let long = "a-rather-long-name-of-a-flange-with-two-holes-for-the-bolts-of-the-cover";
    run(
        &mut session,
        "file.save",
        json!({"path": format!("{long}.3dcraft")}),
    );
    select(
        &mut session,
        "face",
        json!({"in": "Plate", "hole": true}),
        2,
    );
    for (width, hint) in [(1400.0, true), (1280.0, true), (780.0, false)] {
        let mut shell = Shell::with_renderer(RendererChoice::Cpu);
        let mut window = Window::new(width, 860.0);
        window.frames(&mut shell, &mut session, 4);
        let slots: Vec<StatusSlot> = shell.status_hits().iter().map(|(s, _)| *s).collect();
        assert!(slots.contains(&StatusSlot::Selection), "{width}: {slots:?}");
        assert_eq!(
            shell.status_shown(StatusSlot::Selection),
            Some("2 faces selected"),
            "{width}"
        );
        assert_eq!(
            slots.contains(&StatusSlot::Hint),
            hint,
            "{width}: {slots:?}"
        );
        if width > 800.0 {
            assert_eq!(
                shell.status_shown(StatusSlot::Path),
                Some(format!("{long}.3dcraft").as_str()),
                "{width}"
            );
        }
    }
}

/// The grammar's Escape: one step back per press, in one order wherever
/// the pointer is, from a state that has a note field with text, the
/// Studio popover, a chosen note and a selection, and the section on:
/// the field gives up the keys and keeps its text, then closes; the
/// popover closes; the note is no longer chosen; the selection is
/// cleared; the section stays on throughout. (A joint held by hand is let
/// go between the popover and the note: an assembly has no notes, so its
/// step is `mate_markers.rs`'s.)
#[test]
fn escape_steps_back_once_in_one_order() {
    let mut session = Session::new(Workspace::none());
    part(&mut session);
    let added = session
        .by_person(|s| {
            s.execute(
                "note.add",
                &json!({"text": "Make it thicker", "on": "Plate"}),
            )
        })
        .unwrap();
    let note = added["id"].as_u64().expect("the note's id");
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1400.0, 900.0);
    window.frames(&mut shell, &mut session, 4);
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"section": {"on": true, "plane": "yz", "offset": 30},
               "look": {"mode": "studio"}, "panels": {"lookOptions": true},
               "notes": {"selected": note}}),
    );
    select(
        &mut session,
        "face",
        json!({"in": "Plate", "hole": true}),
        1,
    );
    window.frames(&mut shell, &mut session, 3);
    // N opens the field on the face; text typed into it.
    window.key(&mut shell, &mut session, egui::Key::N);
    window.frame(
        &mut shell,
        &mut session,
        vec![Event::Text("Thicker".to_string())],
    );
    window.frames(&mut shell, &mut session, 2);
    assert_eq!(
        shell.note_writing().map(|w| w.text.as_str()),
        Some("Thicker")
    );
    assert!(shell.look_panel_open());
    assert_eq!(shell.state.notes.selected, Some(note));
    assert_eq!(shell.selection().items.len(), 1);
    // The pointer away from the view: Escape works wherever it is.
    window.frame(
        &mut shell,
        &mut session,
        vec![Event::PointerMoved(Pos2::new(5.0, 5.0))],
    );
    let state = |shell: &Shell| {
        (
            shell.note_writing().map(|w| w.text.clone()),
            shell.look_panel_open(),
            shell.state.notes.selected,
            shell.selection().items.len(),
            shell.state.section.on,
        )
    };
    let steps = [
        // 1. The field gives up the keys and keeps its text.
        (Some("Thicker".to_string()), true, Some(note), 1, true),
        // 1. Again: it closes.
        (None, true, Some(note), 1, true),
        // 2. The popover.
        (None, false, Some(note), 1, true),
        // 4. The chosen note.
        (None, false, None, 1, true),
        // 5. The selection.
        (None, false, None, 0, true),
        // Nothing left: the section stays on.
        (None, false, None, 0, true),
    ];
    for (k, expected) in steps.into_iter().enumerate() {
        window.key(&mut shell, &mut session, egui::Key::Escape);
        assert_eq!(state(&shell), expected, "Escape {}", k + 1);
    }
}
