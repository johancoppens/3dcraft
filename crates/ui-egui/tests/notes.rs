//! The notes pinned to the model in the window (`shell/notes.rs`,
//! `shell/sidepanel/notes.rs`): nothing of them for a document without
//! notes; the Notes tab with the open ones counted, an agent's question
//! first; a marker per note in the view, a click on it choosing its note;
//! **N** on a selected face writing a note as the person; the tab's row
//! closing a note through `note.resolve`; the agent pill counting an
//! agent's open questions; a thousand notes thinned to the markers' cap.
//! The pictures (ignored, for a person to look at): light and dark, a
//! chosen note, the field, hundreds of notes, a phone; written to
//! `CRAFT3D_PICS` (else `plan/viewer-tools/pins-pics` of the workshop, or
//! `target/evidence/notes`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::{RendererChoice, Shell, SideTab};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, vec2};
use egui_wgpu::wgpu;
use serde_json::{Value, json};

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn set(shell: &mut Shell, session: &mut Session, params: Value) {
    shell
        .handle_method(session, "ui.set", Some(&params))
        .unwrap()
        .unwrap_or_else(|e| panic!("ui.set {params}: {e}"));
}

fn get(shell: &mut Shell, session: &mut Session) -> Value {
    shell
        .handle_method(session, "ui.get", None)
        .unwrap()
        .unwrap()
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
        self.time += 1.0 / 30.0;
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
        self.frame(shell, session, vec![Event::PointerMoved(at)]);
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
        self.frames(shell, session, 2);
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
    }
}

/// A plate 60 × 40 × 10 (F1) rounded along one top edge (F2, `Round`), a
/// lug beside it (F3) deleted after a note was put on it.
fn plate() -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": [60, 40, 10]}),
    );
    run(
        &mut s,
        "feature.fillet",
        json!({"edges": ["F1:edge(9)"], "radius": 4, "name": "Round"}),
    );
    s
}

/// The plate with its notes: one open (the person's), an agent's
/// question, one done by an agent, one detached, one by place on the
/// fillet.
fn noted() -> Session {
    let mut s = plate();
    s.by_person(|s| {
        s.execute(
            "note.add",
            &json!({"text": "2 mm thicker here", "on": "Plate:face(1)", "point": [40, 25, 10]}),
        )
    })
    .unwrap();
    s.acting(Some("Claude Code"), None, |s| {
        s.execute(
            "note.add",
            &json!({"text": "Should this side be drafted?", "on": "Plate:face(2)"}),
        )
    })
    .unwrap();
    s.by_person(|s| {
        s.execute(
            "note.add",
            &json!({"text": "round this edge", "on": "Plate:face(4)"}),
        )
    })
    .unwrap();
    s.acting(Some("Claude Code"), None, |s| {
        s.execute("note.resolve", &json!({"id": 3, "reply": "Rounded, 4 mm."}))
    })
    .unwrap();
    run(
        &mut s,
        "feature.block",
        json!({"name": "Lug", "size": [10, 10, 10], "position": [60, 15, 0]}),
    );
    s.by_person(|s| {
        s.execute(
            "note.add",
            &json!({"text": "this lug can go", "on": "Lug:face(5)"}),
        )
    })
    .unwrap();
    run(&mut s, "feature.delete", json!({"feature": "Lug"}));
    s.by_person(|s| {
        s.execute(
            "note.add",
            &json!({"text": "a smaller radius here", "on": "F2:fillet(0)"}),
        )
    })
    .unwrap();
    s
}

fn open(window: &mut Window, session: &mut Session) -> Shell {
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    window.frames(&mut shell, session, 6);
    shell
}

fn tabs(shell: &Shell) -> Vec<SideTab> {
    shell.side_hits().tabs.iter().map(|(t, _)| *t).collect()
}

#[test]
fn a_document_without_notes_shows_nothing_of_them() {
    let mut session = plate();
    let mut w = Window::new(1280.0, 800.0);
    let mut shell = open(&mut w, &mut session);
    assert!(
        !tabs(&shell).contains(&SideTab::Notes),
        "{:?}",
        tabs(&shell)
    );
    assert!(shell.note_markers().0.is_empty());
    let state = get(&mut shell, &mut session);
    assert!(state["state"].get("notes").is_none(), "{state}");
    // N with nothing selected opens no field.
    w.key(&mut shell, &mut session, Key::N);
    assert!(shell.note_writing().is_none());
}

#[test]
fn notes_show_in_the_tab_and_the_view_and_a_marker_chooses_its_note() {
    let mut session = noted();
    let mut w = Window::new(1280.0, 800.0);
    let mut shell = open(&mut w, &mut session);
    set(
        &mut shell,
        &mut session,
        json!({"panels": {"sideTab": "notes"}}),
    );
    w.frames(&mut shell, &mut session, 3);
    assert!(tabs(&shell).contains(&SideTab::Notes), "{:?}", tabs(&shell));
    assert_eq!(shell.note_rows().len(), 5);
    let place = |id: u64| {
        shell
            .note_rows()
            .iter()
            .find(|r| r.id == id)
            .unwrap()
            .place
            .clone()
    };
    // The words of place (N3, 2026-11-18: "detached" and "by place: near
    // a face of …" before).
    assert_eq!(place(4), "its face is gone");
    // Held by place: what lies there now, never the name it was pinned by.
    assert!(place(5).starts_with("near a face of "), "{}", place(5));
    let (markers, stood_for) = shell.note_markers();
    assert!(
        !markers.is_empty() && stood_for >= markers.len(),
        "{markers:?}"
    );
    let (id, at) = markers[0];
    w.click(&mut shell, &mut session, at);
    assert_eq!(shell.state.notes.selected, Some(id));
    // The eye hides them; ui.set as well.
    set(
        &mut shell,
        &mut session,
        json!({"notes": {"hideMarkers": true}}),
    );
    w.frames(&mut shell, &mut session, 2);
    assert!(shell.note_markers().0.is_empty());
    let state = get(&mut shell, &mut session);
    assert_eq!(state["state"]["notes"]["hideMarkers"], true, "{state}");
}

#[test]
fn n_on_a_selected_face_writes_a_note_as_the_person() {
    let mut session = plate();
    let mut w = Window::new(1280.0, 800.0);
    let mut shell = open(&mut w, &mut session);
    // The plate's top as the rounded body names it.
    run(
        &mut session,
        "select.set",
        json!({"names": ["F2:copy(0)/F1:face(1)"]}),
    );
    w.frames(&mut shell, &mut session, 2);
    w.key(&mut shell, &mut session, Key::N);
    assert!(shell.note_writing().is_some(), "N opened the field");
    // The Notes tab is offered while a note is written (it may sit under
    // the tab bar's ⋯ here): choosing it holds.
    set(
        &mut shell,
        &mut session,
        json!({"panels": {"sideTab": "notes"}}),
    );
    w.frames(&mut shell, &mut session, 2);
    assert!(shell.note_writing().is_some(), "still writing");
    assert_eq!(
        shell.state.panels.side_tab,
        SideTab::Notes,
        "the tab while writing"
    );
    shell.type_note("2 mm thicker here", true);
    w.frames(&mut shell, &mut session, 3);
    let listed = run(&mut session, "note.list", json!({}));
    assert_eq!(listed["count"], 1, "{listed}");
    assert_eq!(listed["notes"][0]["author"], "person");
    assert_eq!(listed["notes"][0]["text"], "2 mm thicker here");
    // One undo step takes it back.
    run(&mut session, "edit.undo", json!({}));
    assert_eq!(run(&mut session, "note.list", json!({}))["count"], 0);
}

/// *Note here…* in the list under the right mouse button writes a note on
/// the face selected, at the point the right click was on it.
#[test]
fn note_here_writes_a_note_where_the_right_click_was() {
    let mut session = plate();
    let mut w = Window::new(1280.0, 800.0);
    let mut shell = open(&mut w, &mut session);
    run(
        &mut session,
        "select.set",
        json!({"names": ["F2:copy(0)/F1:face(1)"]}),
    );
    w.frames(&mut shell, &mut session, 2);
    // The middle of the view, on the plate's top.
    let panel = shell.side_hits().panel.unwrap();
    let at = Pos2::new((panel.right() + 1280.0) / 2.0, 400.0);
    w.frame(&mut shell, &mut session, vec![Event::PointerMoved(at)]);
    for pressed in [true, false] {
        w.frame(
            &mut shell,
            &mut session,
            vec![Event::PointerButton {
                pos: at,
                button: PointerButton::Secondary,
                pressed,
                modifiers: Modifiers::NONE,
            }],
        );
    }
    w.frames(&mut shell, &mut session, 2);
    let entry = shell.note_here_entry().expect("Note here… offered");
    w.click(&mut shell, &mut session, entry.center());
    let writing = shell.note_writing().expect("the field opened");
    let point = writing.point.expect("the right click's point");
    assert!((point[2] - 10.0).abs() < 1e-6, "on the top: {point:?}");
    shell.type_note("a boss here", true);
    w.frames(&mut shell, &mut session, 3);
    let listed = run(&mut session, "note.list", json!({}));
    let note = &listed["notes"][0];
    assert_eq!(note["author"], "person", "{listed}");
    assert_eq!(note["on"]["kind"], "face", "{listed}");
    for k in 0..3 {
        let got = note["point"][k].as_f64().unwrap();
        assert!((got - point[k]).abs() < 1e-9, "{listed}");
    }
}

fn screenshot(shell: &mut Shell, session: &mut Session) -> Value {
    shell
        .handle_method(
            session,
            "ui.screenshot",
            Some(&json!({"width": 640, "height": 400})),
        )
        .unwrap()
        .unwrap()
}

/// `ui.screenshot` draws the pins as the window does (review-pins: an
/// agent checks the window by this picture): with the pins shown it holds
/// them, hidden it does not, and a document without notes draws none.
#[test]
fn the_screenshot_shows_the_pins() {
    let mut session = noted();
    let mut w = Window::new(1280.0, 800.0);
    let mut shell = open(&mut w, &mut session);
    let shown = screenshot(&mut shell, &mut session);
    assert!(shown["pins"].as_u64().unwrap() >= 3, "{}", shown["pins"]);
    // At the view's own size, the same pins as the window draws.
    let own = shell
        .handle_method(&mut session, "ui.screenshot", Some(&json!({})))
        .unwrap()
        .unwrap();
    assert_eq!(
        own["pins"].as_u64(),
        u64::try_from(shell.note_pins_drawn()).ok(),
        "{}",
        own["pins"]
    );
    set(
        &mut shell,
        &mut session,
        json!({"notes": {"hideMarkers": true}}),
    );
    let hidden = screenshot(&mut shell, &mut session);
    assert_eq!(hidden["pins"], 0);
    assert_ne!(shown["base64"], hidden["base64"]);
    let mut plain = plate();
    let mut w = Window::new(1280.0, 800.0);
    let mut shell = open(&mut w, &mut plain);
    assert_eq!(screenshot(&mut shell, &mut plain)["pins"], 0);
}

/// The field keeps the person's text when `note.add` refuses it, and says
/// why (review-pins: the text was lost).
#[test]
fn a_refused_note_keeps_its_text_in_the_field() {
    let mut session = plate();
    let mut w = Window::new(1280.0, 800.0);
    let mut shell = open(&mut w, &mut session);
    run(
        &mut session,
        "select.set",
        json!({"names": ["F2:copy(0)/F1:face(1)"]}),
    );
    w.frames(&mut shell, &mut session, 2);
    w.key(&mut shell, &mut session, Key::N);
    shell.type_note("a bell\u{7} here", true);
    w.frames(&mut shell, &mut session, 3);
    let writing = shell.note_writing().expect("the field stays open");
    assert_eq!(writing.text, "a bell\u{7} here");
    assert!(
        writing
            .error
            .as_deref()
            .is_some_and(|e| e.contains("control character")),
        "{:?}",
        writing.error
    );
    assert_eq!(run(&mut session, "note.list", json!({}))["count"], 0);
    // Put right, it is added and the field closes.
    shell.type_note("a bell here", true);
    w.frames(&mut shell, &mut session, 3);
    assert!(shell.note_writing().is_none());
    assert_eq!(run(&mut session, "note.list", json!({}))["count"], 1);
}

/// `ui.set` chooses only a note the document has.
#[test]
fn ui_set_refuses_a_note_the_document_does_not_have() {
    let mut session = noted();
    let mut w = Window::new(1280.0, 800.0);
    let mut shell = open(&mut w, &mut session);
    let refused = shell
        .handle_method(
            &mut session,
            "ui.set",
            Some(&json!({"notes": {"selected": 99}})),
        )
        .unwrap()
        .unwrap_err();
    assert!(refused.contains("no note 99"), "{refused}");
    assert_eq!(shell.state.notes.selected, None, "nothing changed");
    set(&mut shell, &mut session, json!({"notes": {"selected": 2}}));
    assert_eq!(shell.state.notes.selected, Some(2));
}

/// Where the side panel drew the target `key` in the last frame.
fn side_target(shell: &Shell, key: &str) -> Option<Rect> {
    shell
        .side_hits()
        .targets
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, r)| *r)
}

/// The tab's field for a note's changed text keeps it when `note.edit`
/// refuses it, and says why (review-pins 2b: not checked).
#[test]
fn a_refused_change_keeps_its_text_in_the_tab() {
    let mut session = noted();
    let mut w = Window::new(1280.0, 800.0);
    let mut shell = open(&mut w, &mut session);
    set(
        &mut shell,
        &mut session,
        json!({"panels": {"sideTab": "notes"}, "notes": {"selected": 1}}),
    );
    w.frames(&mut shell, &mut session, 3);
    let edit = side_target(&shell, "note edit:1").expect("Edit on the chosen note");
    w.click(&mut shell, &mut session, edit.center());
    assert_eq!(shell.note_editing().map(|(id, ..)| id), Some(1));
    shell.type_note_edit("a bell\u{7} here", true);
    w.frames(&mut shell, &mut session, 3);
    let (id, text, error) = shell
        .note_editing()
        .map(|(id, text, error)| (id, text.to_string(), error.map(str::to_string)))
        .expect("the field stays open");
    assert_eq!(id, 1);
    assert_eq!(text, "a bell\u{7} here");
    assert!(
        error
            .as_deref()
            .is_some_and(|e| e.contains("control character")),
        "{error:?}"
    );
    let listed = run(&mut session, "note.list", json!({}));
    assert_eq!(listed["notes"][0]["text"], "2 mm thicker here", "{listed}");
    // Put right, it is kept and the field closes.
    shell.type_note_edit("3 mm thicker here", true);
    w.frames(&mut shell, &mut session, 3);
    assert!(shell.note_editing().is_none());
    let listed = run(&mut session, "note.list", json!({}));
    assert_eq!(listed["notes"][0]["text"], "3 mm thicker here", "{listed}");
}

/// On a short window the selection's card under the Notes tab keeps its
/// buttons within the panel (review-pins 2b: Measure and Copy name ended
/// 3 px below it at 1280 × 460 and 26 px at 390 × 500).
#[test]
fn the_card_under_the_notes_tab_is_within_reach_on_short_windows() {
    for size in [[1280.0_f32, 460.0], [390.0, 500.0]] {
        let mut session = plate();
        for k in 0..30 {
            session
                .by_person(|s| {
                    s.execute(
                        "note.add",
                        &json!({"text": format!("note {k}"), "on": "Plate:face(1)",
                                "point": [5.0 + 1.5 * f64::from(k), 20.0, 10.0]}),
                    )
                })
                .unwrap();
        }
        let mut w = Window::new(size[0], size[1]);
        let mut shell = open(&mut w, &mut session);
        run(
            &mut session,
            "select.set",
            json!({"names": ["F2:copy(0)/F1:face(1)"]}),
        );
        let mut panels = json!({"sideTab": "notes"});
        if size[0] < 500.0 {
            panels["drawer"] = json!(true);
        }
        set(&mut shell, &mut session, json!({ "panels": panels }));
        w.frames(&mut shell, &mut session, 6);
        let panel = shell.side_hits().panel.expect("the side panel");
        let mut seen = 0;
        for key in ["measure", "copy name"] {
            if let Some(r) = side_target(&shell, key) {
                seen += 1;
                assert!(
                    panel.expand(1.0).contains_rect(r),
                    "{size:?}: {key} {r:?} not within {panel:?}"
                );
            }
        }
        assert!(seen > 0, "{size:?}: the card's buttons were drawn");
    }
}

#[test]
fn the_tab_closes_a_note_and_the_pill_counts_an_agents_questions() {
    let mut session = noted();
    let mut w = Window::new(1280.0, 800.0);
    let mut shell = open(&mut w, &mut session);
    // The count first, so a short bar cuts the name and not the count
    // (N1, 2026-11-18: "… · 1 question" at the end before).
    assert!(
        shell
            .agents_pill_view()
            .label
            .starts_with("1 note for you · "),
        "{}",
        shell.agents_pill_view().label
    );
    set(
        &mut shell,
        &mut session,
        json!({"panels": {"sideTab": "notes"}}),
    );
    w.frames(&mut shell, &mut session, 3);
    // The note that waits for the person comes first (For you).
    let rows: Vec<String> = shell
        .side_hits()
        .targets
        .iter()
        .filter(|(k, _)| k.starts_with("note:"))
        .map(|(k, _)| k.clone())
        .collect();
    assert_eq!(rows.first().map(String::as_str), Some("note:2"), "{rows:?}");
    let target = |shell: &Shell, key: &str| {
        shell
            .side_hits()
            .targets
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, r)| r.center())
            .unwrap_or_else(|| panic!("no {key}"))
    };
    let row = target(&shell, "note:2");
    w.click(&mut shell, &mut session, row);
    assert_eq!(shell.state.notes.selected, Some(2));
    let done = target(&shell, "note resolve:2");
    w.click(&mut shell, &mut session, done);
    w.frames(&mut shell, &mut session, 2);
    let listed = run(&mut session, "note.list", json!({"state": "done"}));
    assert!(
        listed["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["id"] == 2 && n["done"]["author"] == "person"),
        "{listed}"
    );
    assert_eq!(shell.notes_for_you(), 0);
}

/// N1 (the UX round of 2026-11-18): a person answers an agent in the
/// window. The tab's groups follow `waitingFor` (note 2, the agent's
/// question, under For you, above the line for the agent; the others
/// under For the agent); the chosen row's field sends `note.reply` as the
/// person, after which the note waits for the agent, stays chosen and
/// the pill counts nothing; Done with text in the field closes it with
/// that text as its reply. `ui.get` › `notes` counts both groups (N2).
#[test]
fn a_reply_in_the_tab_answers_the_agent() {
    let mut session = noted();
    let mut w = Window::new(1280.0, 800.0);
    let mut shell = open(&mut w, &mut session);
    set(
        &mut shell,
        &mut session,
        json!({"panels": {"sideTab": "notes"}, "notes": {"selected": 2}}),
    );
    w.frames(&mut shell, &mut session, 3);
    let state = get(&mut shell, &mut session);
    assert_eq!(
        state["notes"],
        json!({"forYou": 1, "forAgent": 3}),
        "{state}"
    );
    let at =
        |shell: &Shell, key: &str| side_target(shell, key).unwrap_or_else(|| panic!("no {key}"));
    let line = at(&shell, "notes copy");
    assert!(
        at(&shell, "note:2").bottom() <= line.top() + 1.0,
        "For you first"
    );
    assert!(
        at(&shell, "note:1").top() >= line.bottom() - 1.0,
        "For the agent"
    );
    assert!(side_target(&shell, "note reply field:2").is_some());
    assert!(
        side_target(&shell, "note reply field:1").is_none(),
        "the chosen row's only"
    );
    // Edit is offered only on a note the person wrote.
    assert!(side_target(&shell, "note edit:2").is_none());
    shell.type_note_reply("No draft: it is a printed part.", true);
    w.frames(&mut shell, &mut session, 3);
    let listed = run(&mut session, "note.list", json!({}));
    let two = listed["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == 2)
        .unwrap()
        .clone();
    assert_eq!(two["state"], "open", "{two}");
    assert_eq!(two["replies"][0]["author"], "person", "{two}");
    assert_eq!(two["replies"][0]["text"], "No draft: it is a printed part.");
    assert_eq!(two["waitingFor"], "agent", "{two}");
    assert_eq!(shell.notes_for_you(), 0);
    assert_eq!(shell.state.notes.selected, Some(2), "stays chosen");
    assert_eq!(
        shell
            .note_reply_field()
            .map(|(id, text)| (id, text.to_string())),
        Some((2, String::new())),
        "the field emptied"
    );
    assert!(
        !shell.agents_pill_view().label.contains("for you"),
        "{}",
        shell.agents_pill_view().label
    );
    let line = at(&shell, "notes copy");
    assert!(
        at(&shell, "note:2").top() >= line.bottom() - 1.0,
        "moved to For the agent"
    );
    // Done with text in the field: the closing's reply.
    shell.type_note_reply("Closing: kept as it is.", false);
    w.frames(&mut shell, &mut session, 2);
    let done = at(&shell, "note resolve:2");
    w.click(&mut shell, &mut session, done.center());
    w.frames(&mut shell, &mut session, 2);
    let listed = run(&mut session, "note.list", json!({"state": "done"}));
    let two = listed["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == 2)
        .unwrap_or_else(|| panic!("{listed}"))
        .clone();
    assert_eq!(two["done"]["author"], "person", "{two}");
    assert_eq!(two["done"]["reply"], "Closing: kept as it is.", "{two}");
}

/// N1: while a note waits for the person the Notes tab is never behind
/// ⋯: at 260 points it is among the tabs the bar shows.
#[test]
fn with_a_note_for_you_the_notes_tab_is_shown_at_260_points() {
    let mut session = noted();
    let mut w = Window::new(1280.0, 800.0);
    let mut shell = open(&mut w, &mut session);
    set(
        &mut shell,
        &mut session,
        json!({"panels": {"treeWidth": 260}}),
    );
    w.frames(&mut shell, &mut session, 3);
    assert!(shell.notes_for_you() > 0);
    let shown = tabs(&shell);
    assert_eq!(shown.get(1), Some(&SideTab::Notes), "{shown:?}");
}

/// N2: the sentence that sets an agent to work names the document, and
/// its copy button stands under For the agent.
#[test]
fn the_sentence_for_the_agent_names_the_document() {
    let mut session = noted();
    let mut w = Window::new(1280.0, 800.0);
    let mut shell = open(&mut w, &mut session);
    let name = shell
        .tabs()
        .iter()
        .find(|t| t.active)
        .map(|t| t.name.clone())
        .unwrap();
    let text = shell.notes_handover();
    assert_eq!(
        text,
        format!(
            "Work through the open notes of {name}: change the model where each says, and reply on each."
        )
    );
    set(
        &mut shell,
        &mut session,
        json!({"panels": {"sideTab": "notes"}}),
    );
    w.frames(&mut shell, &mut session, 3);
    let copy = side_target(&shell, "notes copy").expect("the copy button");
    w.click(&mut shell, &mut session, copy.center());
    assert!(side_target(&shell, "notes copy").is_some());
}

/// N4: a click on a pin chooses its note and opens the Notes tab on its
/// row (the side panel in sight); the pin under the pointer is known; a
/// click on empty space un-chooses.
#[test]
fn a_click_on_a_pin_opens_its_row_and_one_on_empty_space_unchooses() {
    let mut session = noted();
    let mut w = Window::new(1280.0, 800.0);
    let mut shell = open(&mut w, &mut session);
    assert_eq!(shell.state.panels.side_tab, SideTab::Features);
    let (id, at) = shell.note_markers().0[0];
    w.frame(&mut shell, &mut session, vec![Event::PointerMoved(at)]);
    w.frames(&mut shell, &mut session, 2);
    assert_eq!(shell.note_hovered(), Some(id), "the pin under the pointer");
    w.click(&mut shell, &mut session, at);
    assert_eq!(shell.state.notes.selected, Some(id));
    assert_eq!(shell.state.panels.side_tab, SideTab::Notes);
    w.frames(&mut shell, &mut session, 2);
    assert!(side_target(&shell, &format!("note:{id}")).is_some());
    let panel = shell.side_hits().panel.unwrap();
    w.click(
        &mut shell,
        &mut session,
        Pos2::new(panel.right() + 24.0, 300.0),
    );
    assert_eq!(shell.state.notes.selected, None, "a click on empty space");
}

/// N5: *Note here…* writes on the thing pointed at, at the point under
/// the pointer, whatever was selected, and selects it.
#[test]
fn note_here_writes_on_the_face_pointed_at_not_the_one_selected() {
    let mut session = plate();
    let mut w = Window::new(1280.0, 800.0);
    let mut shell = open(&mut w, &mut session);
    // The bottom selected; the right click on the top.
    run(
        &mut session,
        "select.set",
        json!({"names": ["F2:copy(0)/F1:face(0)"]}),
    );
    w.frames(&mut shell, &mut session, 2);
    let panel = shell.side_hits().panel.unwrap();
    let at = Pos2::new((panel.right() + 1280.0) / 2.0, 400.0);
    w.frame(&mut shell, &mut session, vec![Event::PointerMoved(at)]);
    for pressed in [true, false] {
        w.frame(
            &mut shell,
            &mut session,
            vec![Event::PointerButton {
                pos: at,
                button: PointerButton::Secondary,
                pressed,
                modifiers: Modifiers::NONE,
            }],
        );
    }
    w.frames(&mut shell, &mut session, 2);
    let entry = shell.note_here_entry().expect("Note here… offered");
    w.click(&mut shell, &mut session, entry.center());
    let writing = shell.note_writing().expect("the field opened").clone();
    assert!(writing.on.ends_with("F1:face(1)"), "{:?}", writing.on);
    let point = writing.point.expect("the point under the pointer");
    assert!((point[2] - 10.0).abs() < 1e-6, "on the top: {point:?}");
    let selected: Vec<String> = shell
        .selection()
        .items
        .iter()
        .map(|i| i.name.clone())
        .collect();
    assert_eq!(selected, vec![writing.on.clone()], "selected");
    shell.type_note("a boss here", true);
    w.frames(&mut shell, &mut session, 3);
    let listed = run(&mut session, "note.list", json!({}));
    assert_eq!(listed["notes"][0]["on"]["kind"], "face", "{listed}");
    assert!(
        listed["notes"][0]["on"]["name"]
            .as_str()
            .unwrap()
            .ends_with("F1:face(1)"),
        "{listed}"
    );
}

/// N5: **N** says why it writes nothing: nothing selected, or more than
/// one thing.
#[test]
fn n_says_why_it_cannot_write() {
    use craft3d_ui_egui::shell::notes::{SELECT_FIRST, SELECT_ONE};
    let mut session = plate();
    let mut w = Window::new(1280.0, 800.0);
    let mut shell = open(&mut w, &mut session);
    w.key(&mut shell, &mut session, Key::N);
    w.frames(&mut shell, &mut session, 1);
    assert!(shell.note_writing().is_none());
    assert_eq!(shell.status_hint(), Some(SELECT_FIRST));
    run(
        &mut session,
        "select.set",
        json!({"names": ["F2:copy(0)/F1:face(0)", "F2:copy(0)/F1:face(1)"]}),
    );
    w.frames(&mut shell, &mut session, 2);
    w.key(&mut shell, &mut session, Key::N);
    w.frames(&mut shell, &mut session, 1);
    assert!(shell.note_writing().is_none());
    assert_eq!(shell.status_hint(), Some(SELECT_ONE));
}

/// N6: in a Jig in use **N** says where its notes are written and opens
/// no field; a refused Delete shows its reason under its row, in a
/// person's words, the command's own as the hover.
#[test]
fn in_a_jig_in_use_n_says_why_and_a_refused_delete_shows_in_the_tab() {
    use craft3d_ui_egui::shell::notes::IN_JIG;
    let bytes = noted().document_bytes().expect("the noted document");
    let mut session = Session::new(Workspace::none());
    session
        .open_bytes("plate.3dcraftjig", &bytes)
        .expect("opened as a Jig in use");
    let mut w = Window::new(1280.0, 800.0);
    let mut shell = open(&mut w, &mut session);
    assert!(shell.jig_in_use());
    run(
        &mut session,
        "select.set",
        json!({"names": ["F2:copy(0)/F1:face(1)"]}),
    );
    w.frames(&mut shell, &mut session, 2);
    w.key(&mut shell, &mut session, Key::N);
    w.frames(&mut shell, &mut session, 1);
    assert!(shell.note_writing().is_none(), "no field");
    assert_eq!(shell.status_hint(), Some(IN_JIG));
    set(
        &mut shell,
        &mut session,
        json!({"panels": {"sideTab": "notes"}, "notes": {"selected": 1}}),
    );
    w.frames(&mut shell, &mut session, 3);
    let delete = side_target(&shell, "note delete:1").expect("Delete on the chosen row");
    w.click(&mut shell, &mut session, delete.center());
    w.frames(&mut shell, &mut session, 2);
    let (id, words, why) = shell
        .note_act_error()
        .map(|(id, words, why)| (id, words.to_string(), why.to_string()))
        .expect("the refusal kept");
    assert_eq!(id, 1);
    assert_eq!(words, IN_JIG);
    assert!(why.starts_with("note.delete"), "{why}");
    assert!(
        side_target(&shell, "note refused:1").is_some(),
        "under its row"
    );
    assert_eq!(run(&mut session, "note.list", json!({}))["count"], 5);
}

/// N6: in an assembly **N** says notes are written in a part and opens no
/// field; *Note here…* is in the right-click list, greyed, and opens
/// none.
#[test]
fn in_an_assembly_n_says_why_and_note_here_is_greyed() {
    use craft3d_engine::Access;
    use craft3d_ui_egui::shell::notes::IN_ASSEMBLY;
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("ui-notes-assembly");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut session = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    run(&mut session, "file.new", json!({"units": "mm"}));
    run(
        &mut session,
        "feature.block",
        json!({"name": "Plate", "size": [60, 40, 10]}),
    );
    run(&mut session, "file.save", json!({"path": "plate.3dcraft"}));
    run(&mut session, "assembly.new", json!({}));
    run(
        &mut session,
        "instance.add",
        json!({"source": "plate.3dcraft", "name": "P", "body": "Plate"}),
    );
    let mut w = Window::new(1280.0, 800.0);
    let mut shell = open(&mut w, &mut session);
    w.key(&mut shell, &mut session, Key::N);
    w.frames(&mut shell, &mut session, 1);
    assert!(shell.note_writing().is_none());
    assert_eq!(shell.status_hint(), Some(IN_ASSEMBLY));
    let panel = shell.side_hits().panel.unwrap();
    let at = Pos2::new((panel.right() + 1280.0) / 2.0, 400.0);
    w.frame(&mut shell, &mut session, vec![Event::PointerMoved(at)]);
    for pressed in [true, false] {
        w.frame(
            &mut shell,
            &mut session,
            vec![Event::PointerButton {
                pos: at,
                button: PointerButton::Secondary,
                pressed,
                modifiers: Modifiers::NONE,
            }],
        );
    }
    w.frames(&mut shell, &mut session, 2);
    let entry = shell.note_here_entry().expect("Note here…, greyed");
    w.click(&mut shell, &mut session, entry.center());
    assert!(shell.note_writing().is_none(), "greyed: no field");
}

/// N7: a pin whose place the section cuts away hangs in no air: drawn
/// faint and not clickable; the pin on the kept side is as ever; chosen,
/// the cut-away one is drawn in full.
#[test]
fn a_section_leaves_no_full_pin_in_the_half_it_cuts_away() {
    let mut session = plate();
    for x in [10.0, 50.0] {
        session
            .by_person(|s| {
                s.execute(
                    "note.add",
                    &json!({"text": format!("at {x}"), "on": "Plate:face(1)", "point": [x, 20.0, 10.0]}),
                )
            })
            .unwrap();
    }
    let mut w = Window::new(1280.0, 800.0);
    let mut shell = open(&mut w, &mut session);
    assert_eq!(shell.note_markers().0.len(), 2, "both pins before the cut");
    set(
        &mut shell,
        &mut session,
        json!({"section": {"on": true, "point": [30.0, 20.0, 5.0], "normal": [1.0, 0.0, 0.0]}}),
    );
    w.frames(&mut shell, &mut session, 3);
    let state = get(&mut shell, &mut session);
    let point = state["section"]["point"][0].as_f64().expect("the plane");
    let normal = state["section"]["normal"][0].as_f64().expect("its normal");
    // The note whose place is on the cut-away side of the plane.
    let gone = if (50.0 - point) * normal > 0.0 { 2 } else { 1 };
    let ids: Vec<u64> = shell.note_markers().0.iter().map(|(id, _)| *id).collect();
    assert_eq!(ids, vec![3 - gone], "{state}");
    assert_eq!(shell.note_pins_drawn(), 2, "drawn faint all the same");
    set(
        &mut shell,
        &mut session,
        json!({"notes": {"selected": gone}}),
    );
    w.frames(&mut shell, &mut session, 2);
    assert!(
        shell.note_markers().0.iter().any(|(id, _)| *id == gone),
        "chosen: in full"
    );
}

#[test]
fn a_thousand_notes_draw_within_the_cap() {
    let mut session = plate();
    for k in 0..1000 {
        let x = 1.0 + f64::from(k % 40) * 1.4;
        let y = 1.0 + f64::from(k / 40) * 1.4;
        run(
            &mut session,
            "note.add",
            json!({"text": format!("note {k}"), "on": "F1:face(0)", "point": [x, y, 0]}),
        );
    }
    let mut w = Window::new(1280.0, 800.0);
    let mut shell = open(&mut w, &mut session);
    set(
        &mut shell,
        &mut session,
        json!({"camera": {"pitch": -60.0}}),
    );
    w.frames(&mut shell, &mut session, 3);
    let (markers, stood_for) = shell.note_markers();
    println!(
        "1000 notes: {} markers drawn for {stood_for}",
        markers.len()
    );
    assert!(markers.len() <= craft3d_ui_egui::shell::notes::MAX_MARKERS);
    // A frame with every marker looked up again (a new camera) and one at
    // rest, against the same window without notes.
    let time = |shell: &mut Shell, session: &mut Session, w: &mut Window, turn: bool| {
        let started = std::time::Instant::now();
        for k in 0..5 {
            if turn {
                let yaw = 30.0 + f64::from(k);
                shell
                    .handle_method(session, "ui.set", Some(&json!({"camera": {"yaw": yaw}})))
                    .unwrap()
                    .unwrap();
            }
            w.frame(shell, session, Vec::new());
        }
        started.elapsed().as_secs_f64() / 5.0
    };
    let turning = time(&mut shell, &mut session, &mut w, true);
    let resting = time(&mut shell, &mut session, &mut w, false);
    set(
        &mut shell,
        &mut session,
        json!({"notes": {"hideMarkers": true}}),
    );
    w.frames(&mut shell, &mut session, 2);
    let without = time(&mut shell, &mut session, &mut w, true);
    println!(
        "frame with 1000 notes: {:.1} ms turning, {:.1} ms at rest; without markers {:.1} ms",
        turning * 1e3,
        resting * 1e3,
        without * 1e3
    );
}

/// Paints `jobs` into a texture of `size` pixels and reads it back as RGB.
fn picture(
    state: &egui_wgpu::RenderState,
    jobs: &[egui::ClippedPrimitive],
    size: [u32; 2],
) -> Vec<u8> {
    let mut renderer = state.renderer.write();
    let screen = egui_wgpu::ScreenDescriptor {
        size_in_pixels: size,
        pixels_per_point: 1.0,
    };
    let target = state.device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: state.target_format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = state
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let mut commands =
        renderer.update_buffers(&state.device, &state.queue, &mut encoder, jobs, &screen);
    {
        let mut pass = encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            })
            .forget_lifetime();
        renderer.render(&mut pass, jobs, &screen);
    }
    let row = size[0] * 4;
    let padded = row.div_ceil(256) * 256;
    let buffer = state.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(padded * size[1]),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(size[1]),
            },
        },
        wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
    );
    commands.push(encoder.finish());
    state.queue.submit(commands);
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    state
        .device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let data = slice.get_mapped_range().unwrap();
    let mut rgb = Vec::with_capacity((size[0] * size[1] * 3) as usize);
    let bgra = matches!(
        state.target_format,
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
    );
    for y in 0..size[1] {
        let start = (y * padded) as usize;
        for pixel in data[start..start + row as usize].chunks(4) {
            if bgra {
                rgb.extend([pixel[2], pixel[1], pixel[0]]);
            } else {
                rgb.extend(&pixel[..3]);
            }
        }
    }
    rgb
}

/// A window drawn by egui's own wgpu renderer, for pictures.
struct Shot<'a> {
    state: &'a egui_wgpu::RenderState,
    window: Window,
    jobs: Vec<egui::ClippedPrimitive>,
}

impl Shot<'_> {
    fn frame(&mut self, shell: &mut Shell, session: &mut Session, events: Vec<Event>) {
        self.window.time += 1.0 / 30.0;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.window.size)),
            time: Some(self.window.time),
            events,
            ..Default::default()
        };
        let mut output = self.window.ctx.run_ui(input, |ui| shell.ui(ui, session));
        let mut renderer = self.state.renderer.write();
        for (id, deltas) in &output.textures_delta.set {
            for delta in deltas {
                renderer.update_texture(&self.state.device, &self.state.queue, *id, delta);
            }
        }
        for id in &output.textures_delta.free {
            renderer.free_texture(id);
        }
        output.textures_delta.clear();
        self.jobs = self
            .window
            .ctx
            .tessellate(output.shapes, output.pixels_per_point);
    }

    /// The picture as PNG, and its rows of RGB pixels.
    fn png(&self) -> (Vec<u8>, Vec<u8>) {
        let size = [self.window.size.x as u32, self.window.size.y as u32];
        let rgb = picture(self.state, &self.jobs, size);
        (
            craft3d_render::encode_png(size[0], size[1], &rgb).unwrap(),
            rgb,
        )
    }
}

type Prepare<'p> = &'p dyn Fn(&mut Shell, &mut Session, &mut Shot<'_>);

#[test]
#[ignore = "writes pictures for a person to look at; run with --ignored"]
fn pictures_of_the_notes() {
    use craft3d_ui_egui::direct::headless_render_state;
    let Ok(state) = headless_render_state() else {
        println!("no graphics adapter, no pictures");
        return;
    };
    let dir = std::env::var("CRAFT3D_PICS").map_or_else(
        |_| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/notes")
        },
        std::path::PathBuf::from,
    );
    std::fs::create_dir_all(&dir).unwrap();
    let shoot =
        |name: &str, session: &mut Session, size: [f32; 2], ui_set: Value, prepare: Prepare<'_>| {
            let mut shot = Shot {
                state: &state,
                window: Window::new(size[0], size[1]),
                jobs: Vec::new(),
            };
            let mut shell = Shell::with_renderer(RendererChoice::Cpu);
            for _ in 0..4 {
                shot.frame(&mut shell, session, Vec::new());
            }
            set(&mut shell, session, ui_set);
            for _ in 0..6 {
                shot.frame(&mut shell, session, Vec::new());
            }
            prepare(&mut shell, session, &mut shot);
            for _ in 0..3 {
                shot.frame(&mut shell, session, Vec::new());
            }
            let (png, rgb) = shot.png();
            std::fs::write(dir.join(format!("{name}.png")), &png).unwrap();
            rgb
        };
    let nothing = |_: &mut Shell, _: &mut Session, _: &mut Shot<'_>| {};
    for theme in ["light", "dark"] {
        let mut session = noted();
        shoot(
            &format!("notes-{theme}"),
            &mut session,
            [1400.0, 900.0],
            json!({"appearance": theme, "panels": {"sideTab": "notes"}, "notes": {"selected": 1}}),
            &nothing,
        );
    }
    let mut session = noted();
    shoot(
        "notes-done-open",
        &mut session,
        [1400.0, 900.0],
        json!({"panels": {"sideTab": "notes"}, "notes": {"selected": 2, "doneOpen": true}}),
        &nothing,
    );
    let mut session = plate();
    run(
        &mut session,
        "select.set",
        json!({"names": ["F2:copy(0)/F1:face(1)"]}),
    );
    shoot(
        "notes-writing",
        &mut session,
        [1400.0, 900.0],
        json!({}),
        &|shell, session, shot| {
            for pressed in [true, false] {
                shot.frame(
                    shell,
                    session,
                    vec![Event::Key {
                        key: Key::N,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers: Modifiers::NONE,
                    }],
                );
            }
            shell.type_note("2 mm thicker here", false);
        },
    );
    let mut session = plate();
    for k in 0..300 {
        let x = 2.0 + f64::from(k % 20) * 2.8;
        let y = 2.0 + f64::from(k / 20) * 1.8;
        run(
            &mut session,
            "note.add",
            json!({"text": format!("note {k}"), "on": "F1:face(1)", "point": [x, y, 10]}),
        );
    }
    shoot(
        "notes-hundreds",
        &mut session,
        [1400.0, 900.0],
        json!({"panels": {"sideTab": "notes"}}),
        &nothing,
    );
    let mut session = noted();
    shoot(
        "notes-phone",
        &mut session,
        [390.0, 844.0],
        json!({"panels": {"drawer": true, "sideTab": "notes"}}),
        &nothing,
    );
    // A document without notes draws as it did: the same picture before
    // a note was added and after it was taken away again.
    let mut session = plate();
    let before = shoot(
        "plain-before",
        &mut session,
        [1400.0, 900.0],
        json!({}),
        &nothing,
    );
    run(
        &mut session,
        "note.add",
        json!({"text": "x", "on": "F1:face(1)"}),
    );
    run(&mut session, "note.delete", json!({"state": "all"}));
    let after = shoot(
        "plain-after",
        &mut session,
        [1400.0, 900.0],
        json!({}),
        &nothing,
    );
    // Pixel for pixel, but for the status bar's bottom 32 rows, where the
    // milliseconds the last frame's mesh took differ from run to run.
    let above = (900 - 32) * 1400 * 3;
    assert!(
        before.get(..above) == after.get(..above),
        "a document without notes draws as before"
    );

    // The UX round of 2026-11-18 (plan/viewer-tools/ux/SPEC.md, N1 to
    // N7), named by item.
    // N1: a note in each group, the chosen one's reply field holding text.
    let mut session = noted();
    shoot(
        "N1-tab-groups",
        &mut session,
        [1400.0, 900.0],
        json!({"panels": {"sideTab": "notes"}, "notes": {"selected": 2, "doneOpen": true}}),
        &|shell, _, _| shell.type_note_reply("No draft: it is a printed part.", false),
    );
    // N1: the pill at 480 wide, the count first.
    let mut session = noted();
    shoot(
        "N1-pill-480",
        &mut session,
        [480.0, 860.0],
        json!({}),
        &nothing,
    );
    // N4: pins of each kind, one chosen, one under the pointer (its first
    // line beside it), light and dark.
    for theme in ["light", "dark"] {
        let mut session = noted();
        shoot(
            &format!("N4-pins-{theme}"),
            &mut session,
            [1400.0, 900.0],
            json!({"appearance": theme, "notes": {"selected": 1}}),
            &|shell, session, shot| {
                let other = shell
                    .note_markers()
                    .0
                    .iter()
                    .find(|(id, _)| *id != 1)
                    .map(|(_, at)| *at);
                if let Some(at) = other {
                    shot.frame(shell, session, vec![Event::PointerMoved(at)]);
                    for _ in 0..12 {
                        shot.frame(shell, session, Vec::new());
                    }
                }
            },
        );
    }
    // N5: the list under the right mouse button, Note here… on top.
    let mut session = plate();
    shoot(
        "N5-note-here",
        &mut session,
        [1400.0, 900.0],
        json!({}),
        &|shell, session, shot| {
            let panel = shell.side_hits().panel.map_or(300.0, |p| p.right());
            let at = Pos2::new((panel + 1400.0) / 2.0, 450.0);
            shot.frame(shell, session, vec![Event::PointerMoved(at)]);
            for pressed in [true, false] {
                shot.frame(
                    shell,
                    session,
                    vec![Event::PointerButton {
                        pos: at,
                        button: PointerButton::Secondary,
                        pressed,
                        modifiers: Modifiers::NONE,
                    }],
                );
            }
        },
    );
    // N7: the noted plate cut across X: no full pin in the empty half.
    let mut session = plate();
    for x in [10.0, 30.0, 50.0] {
        session
            .by_person(|s| {
                s.execute(
                    "note.add",
                    &json!({"text": format!("at {x}"), "on": "Plate:face(1)", "point": [x, 20.0, 10.0]}),
                )
            })
            .unwrap();
    }
    shoot(
        "N7-cut",
        &mut session,
        [1400.0, 900.0],
        json!({"section": {"on": true, "point": [40.0, 20.0, 5.0], "normal": [1.0, 0.0, 0.0]}}),
        &nothing,
    );
}
