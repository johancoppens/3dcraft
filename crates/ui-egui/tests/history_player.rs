//! The history player under the timeline: `ui.set` starts it, it shows
//! every step once its scene is there and for at least a dwell, stops at
//! the end or loops, stops on an edit and on another tab; the keys work
//! over the viewport; the caption names the step. And the Assertions
//! heading of the feature panel, there with a hint while there are none.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::Shell;
use craft3d_ui_egui::shell::NO_ASSERTIONS_HINT;
use egui::{Event, Pos2, vec2};
use serde_json::{Value, json};

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn call(shell: &mut Shell, session: &mut Session, method: &str, params: Value) -> Value {
    shell
        .handle_method(session, method, Some(&params))
        .unwrap()
        .unwrap_or_else(|e| panic!("{method} {params}: {e}"))
}

/// A box, a ball and a rod: three steps.
fn three_steps() -> Session {
    let mut session = Session::new(Workspace::none());
    run(&mut session, "file.new", json!({}));
    run(
        &mut session,
        "feature.block",
        json!({"size": [10, 10, 10], "name": "Box"}),
    );
    run(
        &mut session,
        "feature.sphere",
        json!({"radius": 3, "name": "Ball"}),
    );
    run(
        &mut session,
        "feature.cylinder",
        json!({"radius": 1, "height": 30, "name": "Rod"}),
    );
    session
}

/// Where the pointer rests: the middle of the viewport.
const OVER_VIEWPORT: Pos2 = Pos2::new(700.0, 380.0);

struct Window {
    ctx: egui::Context,
    time: f64,
}

impl Window {
    fn new() -> Self {
        Self {
            ctx: egui::Context::default(),
            time: 0.0,
        }
    }

    fn input(&mut self, seconds: f64, events: Vec<Event>) -> egui::RawInput {
        self.time += seconds;
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 760.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        }
    }

    /// A frame `seconds` after the last; returns the texts it drew.
    fn frame(
        &mut self,
        shell: &mut Shell,
        session: &mut Session,
        seconds: f64,
        events: Vec<Event>,
    ) -> Vec<String> {
        let input = self.input(seconds, events);
        let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
        output.textures_delta.clear();
        texts(&output)
    }

    /// A frame while the session is away on the engine thread.
    fn away(&mut self, shell: &mut Shell, seconds: f64) {
        let input = self.input(seconds, Vec::new());
        let mut output = self.ctx.run_ui(input, |ui| shell.ui_away(ui));
        output.textures_delta.clear();
    }

    fn key(&mut self, shell: &mut Shell, session: &mut Session, key: egui::Key) {
        let events = vec![
            Event::PointerMoved(OVER_VIEWPORT),
            Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            },
        ];
        self.frame(shell, session, 1.0 / 60.0, events);
        // The scrub it asked for runs at the end of that frame; the next
        // one shows it.
        self.frame(shell, session, 1.0 / 60.0, Vec::new());
    }
}

fn texts(output: &egui::FullOutput) -> Vec<String> {
    output
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) => Some(text.galley.job.text.clone()),
            _ => None,
        })
        .collect()
}

fn timeline(shell: &mut Shell, session: &mut Session) -> Value {
    call(shell, session, "ui.get", json!({}))["timeline"].clone()
}

#[test]
fn it_plays_every_step_for_a_dwell_and_stops_at_the_end() {
    let mut session = three_steps();
    let mut shell = Shell::new();
    let mut window = Window::new();
    window.frame(&mut shell, &mut session, 0.0, Vec::new());
    assert_eq!(timeline(&mut shell, &mut session)["shown"], 3);
    assert_eq!(timeline(&mut shell, &mut session)["caption"], json!(null));

    let set = call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"timeline": {"playing": true}}),
    );
    assert_eq!(set["state"]["timeline"]["playing"], true);
    assert_eq!(set["state"]["timeline"]["speed"], 1.0);
    // At the end, play starts from the first step; then every step is
    // shown for at least 0.8 s.
    let mut seen: Vec<(u64, f64)> = Vec::new();
    let mut captions = Vec::new();
    for _ in 0..200 {
        let drawn = window.frame(&mut shell, &mut session, 0.05, Vec::new());
        let t = timeline(&mut shell, &mut session);
        let shown = t["shown"].as_u64().unwrap();
        if seen.last().is_none_or(|(s, _)| *s != shown) {
            seen.push((shown, window.time));
            if let Some(caption) = t["caption"].as_str() {
                captions.push(caption.to_string());
                // The caption is drawn over the viewport (from the next
                // frame on: this one drew the step before).
                let drawn = window.frame(&mut shell, &mut session, 0.0, Vec::new());
                assert!(drawn.iter().any(|text| text == caption), "{drawn:?}");
            }
            let _ = drawn;
        }
        if t["playing"] == false {
            break;
        }
    }
    let steps: Vec<u64> = seen.iter().map(|(s, _)| *s).collect();
    assert_eq!(steps, [1, 2, 3], "{seen:?}");
    for pair in seen.windows(2) {
        assert!(pair[1].1 - pair[0].1 >= 0.8 - 1e-9, "{seen:?}");
    }
    assert_eq!(captions[0], "Step 1/3 · Box · block");
    assert!(captions[1].starts_with("Step 2/3 · Ball"), "{captions:?}");
    let t = timeline(&mut shell, &mut session);
    assert_eq!(t["playing"], false);
    // It ended at the end: the document as it is, not scrubbed.
    assert_eq!(t["scrub"], json!(null));
    assert_eq!(t["caption"], json!(null));
}

#[test]
fn speed_loop_and_bad_settings() {
    let mut session = three_steps();
    let mut shell = Shell::new();
    let mut window = Window::new();
    window.frame(&mut shell, &mut session, 0.0, Vec::new());
    for bad in [
        json!({"timeline": {"speed": 3}}),
        json!({"timeline": {"playing": "yes"}}),
        json!({"timeline": {"rewind": true}}),
    ] {
        let result = shell
            .handle_method(&mut session, "ui.set", Some(&bad))
            .unwrap();
        assert!(result.is_err(), "{bad}");
    }
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"timeline": {"playing": true, "speed": 2, "loop": true, "orbit": true}}),
    );
    let t = timeline(&mut shell, &mut session);
    assert_eq!(
        (&t["playing"], &t["speed"], &t["loop"], &t["orbit"]),
        (&json!(true), &json!(2.0), &json!(true), &json!(true))
    );
    let yaw = |shell: &mut Shell, session: &mut Session| {
        call(shell, session, "ui.get", json!({}))["state"]["camera"]["yaw"]
            .as_f64()
            .unwrap()
    };
    let yaw_before = yaw(&mut shell, &mut session);
    // Twice as fast: 0.4 s a step; with loop it comes round again.
    let mut shown = Vec::new();
    for _ in 0..80 {
        window.frame(&mut shell, &mut session, 0.05, Vec::new());
        let step = timeline(&mut shell, &mut session)["shown"]
            .as_u64()
            .unwrap();
        if shown.last() != Some(&step) {
            shown.push(step);
        }
    }
    assert!(shown.len() >= 6, "{shown:?}");
    assert_eq!(&shown[..5], [1, 2, 3, 1, 2], "{shown:?}");
    assert_eq!(timeline(&mut shell, &mut session)["playing"], true);
    // The orbit turned the camera (4 s at 24 degrees a second).
    let turned = (yaw(&mut shell, &mut session) - yaw_before).rem_euclid(360.0);
    assert!((turned - 96.0).abs() < 2.0, "{turned}");
}

#[test]
fn it_waits_for_the_scene_and_never_skips_a_step() {
    let mut session = three_steps();
    let mut shell = Shell::new();
    let mut window = Window::new();
    window.frame(&mut shell, &mut session, 0.0, Vec::new());
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"timeline": {"playing": true}}),
    );
    // To the first step, not yet past its dwell.
    for _ in 0..15 {
        window.frame(&mut shell, &mut session, 0.05, Vec::new());
    }
    assert_eq!(timeline(&mut shell, &mut session)["shown"], 1);
    // The session is away building (the engine thread) for five seconds:
    // the step after it is asked for, but not shown, and not skipped.
    for _ in 0..100 {
        window.away(&mut shell, 0.05);
    }
    let first = window.frame(&mut shell, &mut session, 0.05, Vec::new());
    let _ = first;
    let t = timeline(&mut shell, &mut session);
    assert_eq!(t["shown"], 2, "{t}");
    // Back: step 2 is shown a whole dwell from now, then step 3.
    let back = window.time;
    let mut next = None;
    for _ in 0..40 {
        window.frame(&mut shell, &mut session, 0.05, Vec::new());
        if timeline(&mut shell, &mut session)["shown"] == 3 {
            next = Some(window.time);
            break;
        }
    }
    let next = next.expect("step 3 came");
    assert!(next - back >= 0.8 - 1e-9, "{}", next - back);
}

#[test]
fn an_edit_a_tab_switch_or_to_end_stops_it() {
    let mut session = three_steps();
    let mut shell = Shell::new();
    let mut window = Window::new();
    window.frame(&mut shell, &mut session, 0.0, Vec::new());
    let play = json!({"timeline": {"playing": true}});
    let playing =
        |shell: &mut Shell, session: &mut Session| timeline(shell, session)["playing"] == true;

    // An edit of the document.
    call(&mut shell, &mut session, "ui.set", play.clone());
    window.frame(&mut shell, &mut session, 0.05, Vec::new());
    window.frame(&mut shell, &mut session, 0.05, Vec::new());
    assert!(playing(&mut shell, &mut session));
    run(
        &mut session,
        "feature.edit",
        json!({"feature": "Ball", "radius": 4}),
    );
    window.frame(&mut shell, &mut session, 0.05, Vec::new());
    assert!(!playing(&mut shell, &mut session));

    // Another tab.
    call(&mut shell, &mut session, "ui.set", play.clone());
    window.frame(&mut shell, &mut session, 0.05, Vec::new());
    assert!(playing(&mut shell, &mut session));
    let first = session.document_status().unwrap().index;
    run(&mut session, "file.new", json!({}));
    window.frame(&mut shell, &mut session, 0.05, Vec::new());
    assert!(!playing(&mut shell, &mut session));
    run(&mut session, "file.activate", json!({"document": first}));
    window.frame(&mut shell, &mut session, 0.05, Vec::new());

    // End (To end), over the viewport.
    call(&mut shell, &mut session, "ui.set", play);
    window.frame(&mut shell, &mut session, 0.05, Vec::new());
    window.frame(&mut shell, &mut session, 0.05, Vec::new());
    assert!(playing(&mut shell, &mut session));
    assert_eq!(timeline(&mut shell, &mut session)["shown"], 1);
    window.key(&mut shell, &mut session, egui::Key::End);
    let t = timeline(&mut shell, &mut session);
    assert_eq!(t["playing"], false);
    assert_eq!(t["scrub"], json!(null));
    assert_eq!(t["shown"], 3);
}

#[test]
fn keys_like_a_video_player_over_the_viewport() {
    let mut session = three_steps();
    let mut shell = Shell::new();
    let mut window = Window::new();
    window.frame(&mut shell, &mut session, 0.0, Vec::new());
    let shown =
        |shell: &mut Shell, session: &mut Session| timeline(shell, session)["shown"].clone();
    window.key(&mut shell, &mut session, egui::Key::ArrowLeft);
    assert_eq!(shown(&mut shell, &mut session), 2);
    window.key(&mut shell, &mut session, egui::Key::Home);
    assert_eq!(shown(&mut shell, &mut session), 1);
    // Not before the first.
    window.key(&mut shell, &mut session, egui::Key::ArrowLeft);
    assert_eq!(shown(&mut shell, &mut session), 1);
    window.key(&mut shell, &mut session, egui::Key::ArrowRight);
    assert_eq!(shown(&mut shell, &mut session), 2);
    assert_eq!(
        timeline(&mut shell, &mut session)["caption"],
        "Step 2/3 · Ball · sphere"
    );
    window.key(&mut shell, &mut session, egui::Key::Space);
    assert_eq!(timeline(&mut shell, &mut session)["playing"], true);
    window.key(&mut shell, &mut session, egui::Key::Space);
    assert_eq!(timeline(&mut shell, &mut session)["playing"], false);
    // Elsewhere (over the menu bar) the keys do nothing.
    let events = vec![
        Event::PointerMoved(Pos2::new(700.0, 5.0)),
        Event::Key {
            key: egui::Key::Home,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        },
    ];
    window.frame(&mut shell, &mut session, 0.02, events);
    window.frame(&mut shell, &mut session, 0.02, Vec::new());
    assert_eq!(shown(&mut shell, &mut session), 2);
    window.key(&mut shell, &mut session, egui::Key::End);
    assert_eq!(timeline(&mut shell, &mut session)["scrub"], json!(null));
}

#[test]
fn no_bare_key_edits_the_document() {
    // With the rollback marker on the second step, the End key shows the
    // end in the view and leaves the marker and the document as they are.
    let mut session = three_steps();
    run(&mut session, "history.rollback", json!({"feature": "Ball"}));
    let mut shell = Shell::new();
    let mut window = Window::new();
    window.frame(&mut shell, &mut session, 0.0, Vec::new());
    let revision = session.document_revision();
    for key in [
        egui::Key::Home,
        egui::Key::ArrowRight,
        egui::Key::ArrowLeft,
        egui::Key::Space,
        egui::Key::Space,
        egui::Key::End,
    ] {
        window.key(&mut shell, &mut session, key);
    }
    assert_eq!(session.document_revision(), revision);
    let t = timeline(&mut shell, &mut session);
    assert_eq!(t["rollback"], 2);
    assert_eq!(t["scrub"], json!(null));
    assert_eq!(t["playing"], false);
}

#[test]
fn keys_do_not_reach_the_player_under_a_dialog() {
    let mut session = three_steps();
    let mut shell = Shell::new();
    let mut window = Window::new();
    window.frame(&mut shell, &mut session, 0.0, Vec::new());
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"dialogs": {"shortcuts": true}}),
    );
    window.key(&mut shell, &mut session, egui::Key::Home);
    assert_eq!(timeline(&mut shell, &mut session)["shown"], 3);
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"dialogs": {"shortcuts": false}}),
    );
    window.key(&mut shell, &mut session, egui::Key::Home);
    assert_eq!(timeline(&mut shell, &mut session)["shown"], 1);
}

#[test]
fn nothing_to_play_is_refused() {
    let play = json!({"timeline": {"playing": true}});
    // No document.
    let mut session = Session::new(Workspace::none());
    let mut shell = Shell::new();
    let refused = shell
        .handle_method(&mut session, "ui.set", Some(&play))
        .unwrap();
    assert!(refused.unwrap_err().contains("no document"));
    assert!(!shell.state.timeline.playing);
    // A document without features.
    run(&mut session, "file.new", json!({}));
    let refused = shell
        .handle_method(&mut session, "ui.set", Some(&play))
        .unwrap();
    assert!(refused.unwrap_err().contains("no features"));
    assert_eq!(timeline(&mut shell, &mut session)["playing"], false);
    // The other settings may be set without one.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"timeline": {"loop": true, "speed": 2}}),
    );
    assert_eq!(timeline(&mut shell, &mut session)["loop"], true);
}

#[test]
fn the_assertions_heading_is_there_with_a_hint_while_there_are_none() {
    let mut session = three_steps();
    let mut shell = Shell::new();
    let mut window = Window::new();
    window.frame(&mut shell, &mut session, 0.0, Vec::new());
    let drawn = window.frame(&mut shell, &mut session, 0.02, Vec::new());
    assert!(drawn.iter().any(|t| t == "Assertions"), "{drawn:?}");
    assert!(drawn.iter().any(|t| t == NO_ASSERTIONS_HINT), "{drawn:?}");
    // With no features yet too.
    let mut empty = Session::new(Workspace::none());
    run(&mut empty, "file.new", json!({}));
    let mut other = Shell::new();
    let mut window = Window::new();
    window.frame(&mut other, &mut empty, 0.0, Vec::new());
    let drawn = window.frame(&mut other, &mut empty, 0.02, Vec::new());
    assert!(drawn.iter().any(|t| t == NO_ASSERTIONS_HINT), "{drawn:?}");
    // An assertion takes the hint's place.
    run(
        &mut session,
        "assert.add",
        json!({"name": "Big enough", "measure": "volume", "min": 1}),
    );
    let mut window = Window::new();
    window.frame(&mut shell, &mut session, 0.0, Vec::new());
    let drawn = window.frame(&mut shell, &mut session, 0.02, Vec::new());
    assert!(drawn.iter().any(|t| t == "Assertions"), "{drawn:?}");
    assert!(drawn.iter().any(|t| t == "Big enough"), "{drawn:?}");
    assert!(!drawn.iter().any(|t| t == NO_ASSERTIONS_HINT), "{drawn:?}");
}
