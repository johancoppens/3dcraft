//! The review of the UX round's notes in the window (review-ux-notes,
//! 2026-11-18, on `check/ux-round` 5d8452a7): the reply field when a reply
//! is refused, typed while the camera turns, or on a note an agent deletes;
//! `ui.pick` with `secondary` from every side; what a thousand long notes
//! cost the Notes tab and the pins. The tests marked `ignore` are the
//! review's findings: they fail on 5d8452a7 and pass once the finding is
//! mended (`cargo test -p craft3d-ui-egui --test review_ux_notes --
//! --ignored`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::{RendererChoice, Shell, SideTab};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, vec2};
use serde_json::{Value, json};

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn as_agent(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .acting(Some("Claude Code"), None, |s| s.execute(id, &params))
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn as_person(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .by_person(|s| s.execute(id, &params))
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

fn pick(shell: &mut Shell, session: &mut Session, params: Value) -> Result<Value, String> {
    shell
        .handle_method(session, "ui.pick", Some(&params))
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

    fn input(&mut self, events: Vec<Event>) -> egui::RawInput {
        self.time += 1.0 / 30.0;
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            time: Some(self.time),
            events,
            ..Default::default()
        }
    }

    fn frame(&mut self, shell: &mut Shell, session: &mut Session, events: Vec<Event>) {
        let input = self.input(events);
        let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
        output.textures_delta.clear();
    }

    /// A frame while the session is away, working on the engine thread
    /// (a long command of an agent's): what is asked of it waits.
    fn frame_away(&mut self, shell: &mut Shell, events: Vec<Event>) {
        let input = self.input(events);
        let mut output = self.ctx.run_ui(input, |ui| shell.ui_away(ui));
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
}

fn key(key: Key, pressed: bool) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: Modifiers::NONE,
    }
}

/// A plate 60 × 40 × 10 (F1) rounded along one top edge (F2, `Round`).
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

/// The plate with the person's note (1), an agent's question (2) and the
/// person's note on another face (3).
fn noted() -> Session {
    let mut s = plate();
    as_person(
        &mut s,
        "note.add",
        json!({"text": "2 mm thicker here", "on": "Plate:face(1)", "point": [40, 25, 10]}),
    );
    as_agent(
        &mut s,
        "note.add",
        json!({"text": "Should this side be drafted?", "on": "Plate:face(2)"}),
    );
    as_person(
        &mut s,
        "note.add",
        json!({"text": "round this edge", "on": "Plate:face(4)"}),
    );
    s
}

fn open(window: &mut Window, session: &mut Session) -> Shell {
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    window.frames(&mut shell, session, 6);
    shell
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

fn replies(session: &mut Session, id: u64) -> Vec<String> {
    let listed = run(session, "note.list", json!({}));
    listed["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == id)
        .map(|n| {
            n["replies"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r["text"].as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// The Notes tab open on note `id`, chosen.
fn on_note(shell: &mut Shell, session: &mut Session, w: &mut Window, id: u64) {
    set(
        shell,
        session,
        json!({"panels": {"sideTab": "notes"}, "notes": {"selected": id}}),
    );
    w.frames(shell, session, 3);
}

/// A refused reply keeps its text in the field and says why under the row,
/// in a person's words, the command's own as the hover; Done with that
/// text then closes the note with it (a closing's reply is not one of the
/// 20).
#[test]
fn a_refused_reply_keeps_its_text_and_says_why() {
    let mut session = noted();
    for k in 0..19 {
        run(
            &mut session,
            "note.reply",
            json!({"id": 2, "text": format!("r{k}")}),
        );
    }
    as_agent(&mut session, "note.reply", json!({"id": 2, "text": "r19"}));
    let mut w = Window::new(1280.0, 800.0);
    let mut shell = open(&mut w, &mut session);
    on_note(&mut shell, &mut session, &mut w, 2);
    shell.type_note_reply("One more answer.", true);
    w.frames(&mut shell, &mut session, 3);
    assert_eq!(
        shell
            .note_reply_field()
            .map(|(id, text)| (id, text.to_string())),
        Some((2, "One more answer.".to_string())),
        "the text kept"
    );
    let (id, words, why) = shell
        .note_act_error()
        .map(|(id, words, why)| (id, words.to_string(), why.to_string()))
        .expect("the refusal said");
    assert_eq!(id, 2);
    // With what to do instead (ux-c after the review: a refusal gives the
    // next step; the sentence as the review read it, then the step).
    assert_eq!(
        words,
        "This note has 20 replies, as many as a note takes: close it with Done, or write a new note to go on."
    );
    assert!(why.starts_with("note.reply"), "{why}");
    assert!(
        side_target(&shell, "note refused:2").is_some(),
        "under its row"
    );
    assert_eq!(replies(&mut session, 2).len(), 20);
    // Done with the text in the field: the closing's reply.
    let done = side_target(&shell, "note resolve:2").expect("Done");
    w.click(&mut shell, &mut session, done.center());
    w.frames(&mut shell, &mut session, 2);
    let listed = run(&mut session, "note.list", json!({"state": "done"}));
    let two = listed["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == 2)
        .expect("closed")
        .clone();
    assert_eq!(two["done"]["reply"], "One more answer.", "{two}");
    assert_eq!(two["done"]["author"], "person");
    assert_eq!(
        shell.note_reply_field().map(|(_, text)| text.to_string()),
        Some(String::new()),
        "the field emptied"
    );
}

/// Typed in the reply field while the camera turns under it (an agent
/// turning the view): every letter goes to the field (an N among them
/// opens no note's field), Enter sends the reply once and leaves the
/// camera where it was turned to; the groups, the pill and `ui.get` ›
/// `notes` then count the note as the agent's, as `note.list` does.
#[test]
fn a_reply_typed_while_the_camera_turns() {
    let mut session = noted();
    let mut w = Window::new(1280.0, 800.0);
    let mut shell = open(&mut w, &mut session);
    on_note(&mut shell, &mut session, &mut w, 2);
    let field = side_target(&shell, "note reply field:2").expect("the field");
    w.click(&mut shell, &mut session, field.center());
    // Whatever flight a choice started is over (FLY_SECONDS).
    w.frames(&mut shell, &mut session, 12);
    let typed = [
        vec![
            key(Key::N, true),
            Event::Text("N".into()),
            key(Key::N, false),
        ],
        vec![Event::Text("o draft".into())],
        vec![Event::Text(": it is printed.".into())],
    ];
    let mut yaw = 30.0;
    for events in typed {
        yaw += 7.0;
        set(
            &mut shell,
            &mut session,
            json!({"camera": {"yaw": yaw, "autoFit": false}}),
        );
        w.frame(&mut shell, &mut session, events);
    }
    yaw += 7.0;
    set(
        &mut shell,
        &mut session,
        json!({"camera": {"yaw": yaw, "autoFit": false}}),
    );
    w.frame(&mut shell, &mut session, vec![key(Key::Enter, true)]);
    w.frame(&mut shell, &mut session, vec![key(Key::Enter, false)]);
    w.frames(&mut shell, &mut session, 3);
    assert!(shell.note_writing().is_none(), "N went to the field");
    assert_eq!(
        replies(&mut session, 2),
        ["No draft: it is printed."],
        "sent once, whole"
    );
    let state = get(&mut shell, &mut session);
    let turned = state["state"]["camera"]["yaw"].as_f64().unwrap();
    assert!((turned - yaw).abs() < 1e-9, "{turned} {yaw}");
    // Counted alike everywhere.
    let listed = run(&mut session, "note.list", json!({}));
    assert_eq!(state["notes"]["forYou"], listed["forYou"], "{state}");
    assert_eq!(state["notes"]["forAgent"], listed["forAgent"]);
    assert_eq!(
        shell.notes_for_you() as u64,
        listed["forYou"].as_u64().unwrap()
    );
    assert!(
        !shell.agents_pill_view().label.contains("for you"),
        "{}",
        shell.agents_pill_view().label
    );
}

/// A note an agent deletes while the person writes a reply to it: the row
/// and its half-written reply go, no other note is given the text; a reply
/// sent but not yet run when the agent deletes the note is refused and
/// lands on no other note. Nothing panics.
#[test]
fn a_note_deleted_while_its_reply_is_written() {
    let mut session = noted();
    let mut w = Window::new(1280.0, 800.0);
    let mut shell = open(&mut w, &mut session);
    on_note(&mut shell, &mut session, &mut w, 2);
    shell.type_note_reply("half an answer", false);
    w.frames(&mut shell, &mut session, 1);
    as_agent(&mut session, "note.delete", json!({"id": 2}));
    w.frames(&mut shell, &mut session, 3);
    assert_eq!(shell.state.notes.selected, None);
    assert!(shell.note_reply_field().is_none(), "the text went with it");
    on_note(&mut shell, &mut session, &mut w, 1);
    assert!(
        shell
            .note_reply_field()
            .is_none_or(|(id, text)| id == 1 && text.is_empty()),
        "{:?}",
        shell.note_reply_field()
    );
    // Sent, and the note deleted before the window runs it.
    shell.type_note_reply("for note one", true);
    as_agent(&mut session, "note.delete", json!({"id": 1}));
    w.frames(&mut shell, &mut session, 3);
    let listed = run(&mut session, "note.list", json!({}));
    for n in listed["notes"].as_array().unwrap() {
        assert_eq!(n["replies"], json!([]), "{n}");
    }
    assert_eq!(shell.state.notes.selected, None);
    println!("refused: {:?}", shell.note_act_error());
}

/// `ui.pick` with `secondary` from every side: on empty space it answers
/// an empty list and opens none; in a picture of another size it is the
/// same place of the view; Escape closes the list it opened; on a pin; in
/// an assembly and in a Jig in use the entry is greyed and opens no field;
/// every hostile parameter refused with a sentence; the document, the
/// selection and the last click as they were.
#[test]
fn ui_pick_secondary_from_every_side() {
    let mut session = noted();
    let mut w = Window::new(1280.0, 800.0);
    let mut shell = open(&mut w, &mut session);
    let [vw, vh] = shell.viewport_size();
    let (x, y) = (f64::from(vw) / 2.0, f64::from(vh) / 2.0);
    let before = session.document_bytes();
    let picked = get(&mut shell, &mut session)["state"]["picked"].clone();

    // Empty space, a corner of the view.
    let empty = pick(
        &mut shell,
        &mut session,
        json!({"x": 1, "y": 1, "secondary": true}),
    )
    .expect("a right click on nothing");
    assert_eq!(empty["list"], json!([]), "{empty}");
    w.frames(&mut shell, &mut session, 2);
    assert!(shell.note_here_entry().is_none() && shell.other_menu().is_none());

    // The middle of a picture of 32 × 32 pixels (16 is the least side):
    // the middle of the view.
    let small = pick(
        &mut shell,
        &mut session,
        json!({"x": 16.0, "y": 16.0, "width": 32, "height": 32, "secondary": true}),
    )
    .expect("the middle");
    let listed = small["list"].as_array().unwrap().len();
    assert!(listed > 0, "{small}");
    let middle = pick(
        &mut shell,
        &mut session,
        json!({"x": x, "y": y, "secondary": true, "all": true}),
    )
    .expect("the middle");
    assert_eq!(middle["list"], small["list"], "{middle}");
    assert!(middle["hits"].as_array().is_some_and(|h| !h.is_empty()));
    w.frames(&mut shell, &mut session, 2);
    assert!(shell.note_here_entry().is_some(), "Note here… on top");
    w.frame(&mut shell, &mut session, vec![key(Key::Escape, true)]);
    w.frame(&mut shell, &mut session, vec![key(Key::Escape, false)]);
    w.frames(&mut shell, &mut session, 2);
    assert!(shell.other_menu().is_none(), "Escape closes the list");

    // On a pin.
    let (id, head) = shell.note_markers().0[0];
    let rect = shell.viewport_rect().expect("the view");
    let px = f64::from(head.x - rect.left()) * f64::from(vw) / f64::from(rect.width());
    let py = f64::from(head.y - rect.top()) * f64::from(vh) / f64::from(rect.height());
    let on_pin = pick(
        &mut shell,
        &mut session,
        json!({"x": px, "y": py, "secondary": true}),
    )
    .expect("a right click on a pin");
    w.frames(&mut shell, &mut session, 2);
    println!(
        "right click on pin {id}: {on_pin}; Note here… offered: {}",
        shell.note_here_entry().is_some()
    );
    assert!(shell.note_writing().is_none());

    // Hostile.
    for params in [
        json!({"x": x, "y": y, "secondary": true, "radius": -1}),
        json!({"x": x, "y": y, "secondary": true, "radius": 1e300}),
        json!({"x": x, "y": y, "secondary": true, "width": 0}),
        json!({"x": 1.0, "y": 1.0, "secondary": true, "width": 2, "height": 2}),
        json!({"x": x, "y": y, "secondary": true, "width": -5}),
        json!({"x": x, "y": y, "secondary": true, "width": 1.5}),
        json!({"x": x, "y": y, "secondary": true, "height": 4_294_967_296_u64}),
        json!({"x": x, "y": y, "secondary": "true"}),
        json!({"x": x, "y": y, "secondary": true, "button": 2}),
        json!({"x": [x], "y": y, "secondary": true}),
        json!({"x": x, "secondary": true}),
        json!({"x": -1e-9, "y": y, "secondary": true}),
        json!({"x": x, "y": 1e300, "secondary": true}),
        json!({"x": x, "y": y, "secondary": true, "shift": true}),
        json!([x, y]),
    ] {
        let e = match pick(&mut shell, &mut session, params.clone()) {
            Ok(answer) => panic!("{params} should be refused: {answer}"),
            Err(e) => e,
        };
        assert!(e.starts_with("ui.pick"), "{params}: {e}");
    }
    w.frames(&mut shell, &mut session, 2);
    assert!(session.document_bytes() == before, "the document as it was");
    assert!(shell.selection().items.is_empty(), "nothing selected");
    assert_eq!(
        get(&mut shell, &mut session)["state"]["picked"],
        picked,
        "the last click as it was"
    );

    // A Jig in use: the entry greyed with its reason, no field.
    let bytes = noted().document_bytes().unwrap();
    let mut jig = Session::new(Workspace::none());
    jig.open_bytes("plate.3dcraftjig", &bytes).unwrap();
    let mut w = Window::new(1280.0, 800.0);
    let mut shell = open(&mut w, &mut jig);
    pick(
        &mut shell,
        &mut jig,
        json!({"x": x, "y": y, "secondary": true}),
    )
    .expect("a right click in a Jig in use");
    w.frames(&mut shell, &mut jig, 2);
    let entry = shell.note_here_entry().expect("Note here…, greyed");
    w.click(&mut shell, &mut jig, entry.center());
    assert!(shell.note_writing().is_none(), "greyed: no field");
}

/// A thousand notes of some 1400 characters each (1.4 MB of text, within
/// the 2 MiB a document holds), half of them waiting for the person, the
/// first with 20 replies and chosen: what a frame costs with the Notes tab
/// shown against the Features tab, at rest and with the camera turning,
/// after a reply (the list read again), and zoomed far in and far out,
/// where the pins stay within their cap. Printed, for the review; the only
/// assertions are the caps.
#[test]
fn a_thousand_long_notes_in_the_tab_and_the_view() {
    use craft3d_ui_egui::shell::notes::MAX_MARKERS;
    let mut session = plate();
    let words = "lorem ipsum dolor sit amet ".repeat(52);
    for k in 0..1000u32 {
        let x = 1.0 + f64::from(k % 40) * 1.4;
        let y = 1.0 + f64::from(k / 40) * 1.4;
        let params =
            json!({"text": format!("note {k}: {words}"), "on": "F1:face(0)", "point": [x, y, 0]});
        if k % 2 == 0 {
            as_agent(&mut session, "note.add", params);
        } else {
            as_person(&mut session, "note.add", params);
        }
    }
    for k in 0..20 {
        let params =
            json!({"id": 1, "text": format!("reply {k}: {}", "a few words more ".repeat(5))});
        if k % 2 == 0 {
            as_person(&mut session, "note.reply", params);
        } else {
            as_agent(&mut session, "note.reply", params);
        }
    }
    let listed = run(&mut session, "note.list", json!({"state": "open"}));
    println!(
        "1000 notes: {} for you, {} for the agent",
        listed["forYou"], listed["forAgent"]
    );
    let mut w = Window::new(1280.0, 800.0);
    let mut shell = open(&mut w, &mut session);
    set(
        &mut shell,
        &mut session,
        json!({"camera": {"pitch": -60.0}}),
    );
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
        started.elapsed().as_secs_f64() * 1e3 / 5.0
    };
    w.frames(&mut shell, &mut session, 3);
    let features = (
        time(&mut shell, &mut session, &mut w, false),
        time(&mut shell, &mut session, &mut w, true),
    );
    on_note(&mut shell, &mut session, &mut w, 1);
    assert_eq!(shell.state.panels.side_tab, SideTab::Notes);
    let first = std::time::Instant::now();
    w.frames(&mut shell, &mut session, 1);
    let first = first.elapsed().as_secs_f64() * 1e3;
    let tab = (
        time(&mut shell, &mut session, &mut w, false),
        time(&mut shell, &mut session, &mut w, true),
    );
    // A reply: the document changes and the list is read again.
    let started = std::time::Instant::now();
    as_agent(
        &mut session,
        "note.reply",
        json!({"id": 3, "text": "Which side?"}),
    );
    let command = started.elapsed().as_secs_f64() * 1e3;
    let started = std::time::Instant::now();
    w.frames(&mut shell, &mut session, 1);
    let reread = started.elapsed().as_secs_f64() * 1e3;
    println!(
        "a frame with 1000 long notes: Features tab {:.1} ms at rest, {:.1} ms turning; Notes tab {:.1} ms at rest, {:.1} ms turning (its first frame {first:.1} ms); a reply {command:.1} ms and the frame after it {reread:.1} ms",
        features.0, features.1, tab.0, tab.1
    );
    assert!(shell.note_pins_drawn() <= MAX_MARKERS);
    // Far in and far out: the pins within their cap, the frame as cheap.
    for distance in [1e-6, 1e-2, 1e3, 1e9] {
        set(
            &mut shell,
            &mut session,
            json!({"camera": {"distance": distance, "autoFit": false}}),
        );
        let started = std::time::Instant::now();
        w.frames(&mut shell, &mut session, 3);
        let ms = started.elapsed().as_secs_f64() * 1e3 / 3.0;
        let drawn = shell.note_pins_drawn();
        println!("distance {distance:e}: {drawn} pins, {ms:.1} ms a frame");
        assert!(drawn <= MAX_MARKERS, "{drawn} pins at {distance}");
    }
}

/// Finding 3 (should): Enter pressed twice in the reply field while the
/// session is away (an agent's long command runs on the engine thread)
/// sends the reply twice. The field keeps its text until the answer
/// (`sidepanel/notes.rs` 240-247) and `send_note_reply` (183-190) sends
/// again whatever is waiting, so the person who sees no answer and presses
/// Enter again gives the same reply twice. One reply per text sent; the
/// field could say it is sending.
#[test]
fn enter_twice_while_the_engine_works_sends_one_reply() {
    let mut session = noted();
    let mut w = Window::new(1280.0, 800.0);
    let mut shell = open(&mut w, &mut session);
    on_note(&mut shell, &mut session, &mut w, 2);
    let field = side_target(&shell, "note reply field:2").expect("the field");
    w.click(&mut shell, &mut session, field.center());
    w.frame(
        &mut shell,
        &mut session,
        vec![Event::Text("Yes, 9 mm.".into())],
    );
    for _ in 0..2 {
        w.frame_away(&mut shell, vec![key(Key::Enter, true)]);
        w.frame_away(&mut shell, vec![key(Key::Enter, false)]);
        w.frame_away(&mut shell, Vec::new());
    }
    w.frames(&mut shell, &mut session, 3);
    assert_eq!(replies(&mut session, 2), ["Yes, 9 mm."], "sent once");
}
