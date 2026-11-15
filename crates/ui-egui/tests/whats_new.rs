//! What's new (`shell/whats_new.rs`): after an update the start screen
//! shows a card *New in <version>* once, what changes an answer first;
//! Help › What's new shows the notes again, as that card on the start
//! screen and in a window over a document. The card is never in the way:
//! not on a first start, not for the same version or an older one, never
//! over a document, never in a presentation other than the whole window.
//! Drawn in real frames, read back from where the shell laid things out.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::changelog::{Changelog, Seen, Version};
use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::Shell;
use craft3d_ui_egui::shell::WhatsNew;
use craft3d_ui_egui::shell::whats_new::{MENU_ITEM, NEW_TITLE, RESULTS_HINT, RESULTS_TITLE};
use craft3d_ui_egui::state::{Appearance, Presentation};
use egui::{Event, Pos2, Rect, vec2};
use serde_json::{Value, json};

const SAMPLE: &str = "\
## [Unreleased]

### Added

- **A thing not released yet.** It is on its way.

## [0.3.0] - 2026-12-20

### Results that changed

- **A measured volume could be wrong.** `document.measure` reported 10 mm³ *too little*; see [the guide](https://example.org/guide).
- **A clearance could be reported as proven for parts that overlap.** Such a pair is now an overlap.

### Added

- **A fillet takes one edge by its own name.** Give `\"edges\": [\"F3:between[…]\"]`.

### Changed

- **Files are format version 72.**

### Fixed

- **A shell was refused where four faces meet.** It builds now.

## [0.2.1] - 2026-12-05

### Fixed

- **Windows: no console window.** The launcher opens none.

## [0.2.0] - 2026-12-01

### Results that changed

- **A proven distance could be too high.** It can be lower now.

### Added

- **Embed.** The page drives the module.

### Known limitations

- The [known limitations of 0.1.0](https://example.org/v0.1.0) still hold.

## [0.1.0] - 2026-11-11

The first release.
";

fn v(text: &str) -> Version {
    Version::parse(text).unwrap()
}

fn sample() -> Changelog {
    Changelog::parse(SAMPLE).unwrap()
}

/// The sample's notes for someone who had `seen` and now runs 0.3.0.
fn notes(seen: Seen) -> WhatsNew {
    WhatsNew::for_start(&sample(), v("0.3.0"), seen).unwrap()
}

/// The repository's changelog as the next release would carry it: what is
/// not released yet under the next minor version, released today. What a
/// person who last saw 0.1.1 is shown when that version starts.
fn next_release() -> WhatsNew {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../CHANGELOG.md");
    let text = std::fs::read_to_string(path).unwrap();
    let running = v(craft3d_engine::VERSION);
    let next = Version::new(running.major, running.minor + 1, 0);
    let text = text.replacen("## [Unreleased]", &format!("## [{next}] - 2026-11-14"), 1);
    let changelog = Changelog::parse(&text).unwrap();
    let next = changelog.newest().unwrap();
    WhatsNew::for_start(&changelog, next, Seen::Version(v("0.1.1"))).unwrap()
}

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

struct Window {
    ctx: egui::Context,
    time: f64,
    size: egui::Vec2,
}

impl Window {
    fn sized(width: f32, height: f32) -> Self {
        Self {
            ctx: egui::Context::default(),
            time: 0.0,
            size: vec2(width, height),
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

    fn frames(&mut self, shell: &mut Shell, session: &mut Session, count: usize) {
        for _ in 0..count {
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
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        self.frame(shell, session, Vec::new());
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
        self.frame(shell, session, Vec::new());
    }

    /// Opens the Help menu and clicks its item `label`.
    fn help(&mut self, shell: &mut Shell, session: &mut Session, label: &str) {
        let help = shell
            .menu_titles()
            .iter()
            .find(|(title, _)| title == "Help")
            .map(|(_, rect)| *rect)
            .unwrap();
        self.click(shell, session, help.center());
        let item = shell
            .menu_items()
            .iter()
            .find(|(item, _)| item == label)
            .map(|(_, rect)| *rect)
            .unwrap_or_else(|| panic!("no `{label}` in Help: {:?}", shell.menu_items()));
        self.click(shell, session, item.center());
    }
}

fn hit(shell: &Shell, name: &str) -> Option<Rect> {
    shell
        .whats_new_hits()
        .iter()
        .find(|(hit, _)| hit == name)
        .map(|(_, rect)| *rect)
}

fn start_hit(shell: &Shell, name: &str) -> Rect {
    shell
        .start_hits()
        .iter()
        .find(|(hit, _)| hit == name)
        .map(|(_, rect)| *rect)
        .unwrap_or_else(|| panic!("no {name} on the start screen"))
}

fn examples(shell: &Shell) -> Vec<Rect> {
    shell
        .start_hits()
        .iter()
        .filter(|(name, _)| name.starts_with("example:"))
        .map(|(_, rect)| *rect)
        .collect()
}

fn entries(shell: &Shell) -> Vec<(String, Rect)> {
    shell
        .whats_new_hits()
        .iter()
        .filter(|(name, _)| name.starts_with("entry:"))
        .cloned()
        .collect()
}

/// A window on the start screen with `notes`, drawn until it has settled.
fn started(notes: Option<WhatsNew>, width: f32, height: f32) -> (Shell, Session, Window) {
    let mut session = Session::new(Workspace::none());
    let mut shell = Shell::new();
    shell.set_whats_new(notes);
    let mut window = Window::sized(width, height);
    window.frames(&mut shell, &mut session, 4);
    (shell, session, window)
}

#[test]
fn after_an_update_the_start_screen_shows_the_card_with_what_changes_an_answer_first() {
    let offered = notes(Seen::Version(v("0.2.1")));
    assert!(offered.offered);
    assert_eq!(offered.title(), "New in 0.3.0");
    let (mut shell, mut session, mut window) = started(Some(offered), 1100.0, 760.0);
    assert!(shell.start_shown());
    assert_eq!(shell.whats_new_shown(), Some("card"));
    assert!(shell.state.whats_new.card && !shell.state.whats_new.window);
    let card = hit(&shell, "card").unwrap();
    let viewport = shell.viewport_rect().unwrap();
    assert!(viewport.contains_rect(card), "{card:?} in {viewport:?}");
    // Under New and Open, above the examples, as wide as the column.
    let column = start_hit(&shell, "card");
    assert!(card.top() >= start_hit(&shell, "open").bottom());
    let first = examples(&shell)
        .iter()
        .map(Rect::top)
        .fold(f32::MAX, f32::min);
    assert!(
        card.bottom() <= first,
        "{card:?} above the examples at {first}"
    );
    assert!(
        (card.left() - column.left()).abs() < 1.0 && (card.right() - column.right()).abs() < 1.0
    );
    // What changes an answer first, under its own heading; then what is
    // new; then the rest, folded away.
    let results = hit(&shell, "results").unwrap();
    let new = hit(&shell, "new").unwrap();
    let rest = hit(&shell, "rest").unwrap();
    assert!(results.top() < new.top() && new.top() < rest.top());
    assert!(results.bottom() <= new.top() + 0.5 && new.bottom() <= rest.top() + 0.5);
    for part in [results, new, rest, hit(&shell, "close").unwrap()] {
        assert!(card.expand(0.5).contains_rect(part), "{part:?} in {card:?}");
    }
    // The lines: two results and two new things (one of them not
    // released yet); the rest is folded.
    let names: Vec<String> = entries(&shell).into_iter().map(|(name, _)| name).collect();
    assert_eq!(names, ["entry:1:0", "entry:1:1", "entry:0:0", "entry:1:2"]);
    assert!(!shell.state.whats_new.rest);
    // An agent reads the same.
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(
        got["whatsNew"],
        json!({
            "version": "0.3.0", "offered": true, "shown": "card", "low": false,
            "versions": ["unreleased", "0.3.0"],
            "changesAnswers": 2, "new": 2, "rest": 2,
        })
    );
    assert_eq!(
        got["state"]["whatsNew"],
        json!({"card": true, "window": false, "rest": false, "expanded": false})
    );
    // A click on the fold shows the rest, each kind under its name.
    window.click(
        &mut shell,
        &mut session,
        Pos2::new(rest.left() + 60.0, rest.top() + 8.0),
    );
    assert!(shell.state.whats_new.rest);
    assert_eq!(entries(&shell).len(), 6);
    // A click on a title unfolds what the changelog says under it, and
    // again folds it.
    let (_, first) = entries(&shell)[0].clone();
    let below = entries(&shell)[1].1.top();
    window.click(&mut shell, &mut session, first.center());
    let unfolded = entries(&shell)[1].1.top();
    assert!(unfolded > below + 10.0, "{below} then {unfolded}");
    let (_, first) = entries(&shell)[0].clone();
    window.click(&mut shell, &mut session, first.center());
    assert!((entries(&shell)[1].1.top() - below).abs() < 0.5);
    // The × closes it, and the host is told once, to remember.
    assert!(!shell.take_whats_new_closed());
    let close = hit(&shell, "close").unwrap();
    window.click(&mut shell, &mut session, close.center());
    assert!(!shell.state.whats_new.card);
    assert_eq!(shell.whats_new_shown(), None);
    assert!(shell.whats_new_hits().is_empty());
    assert!(shell.take_whats_new_closed());
    assert!(!shell.take_whats_new_closed());
    window.frames(&mut shell, &mut session, 3);
    assert_eq!(shell.whats_new_shown(), None, "it stays closed");
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["whatsNew"]["shown"], Value::Null);
    // The fold is what is left of the state, until it is folded again.
    assert_eq!(
        got["state"]["whatsNew"],
        json!({"card": false, "window": false, "rest": true, "expanded": false})
    );
}

#[test]
fn the_same_version_a_first_start_and_a_downgrade_show_no_card() {
    let (plain, _, _) = started(None, 1100.0, 760.0);
    for seen in [
        Seen::Never,
        Seen::Version(v("0.3.0")),
        Seen::Version(v("0.3.1")),
        Seen::Version(v("9.0.0")),
    ] {
        let known = notes(seen);
        assert!(!known.offered, "{seen:?}");
        let (mut shell, mut session, mut window) = started(Some(known), 1100.0, 760.0);
        assert_eq!(shell.whats_new_shown(), None, "{seen:?}");
        assert!(shell.whats_new_hits().is_empty());
        assert!(shell.state.whats_new.is_closed());
        assert!(!shell.take_whats_new_closed());
        // The start screen is exactly what it is without notes.
        assert_eq!(shell.start_hits(), plain.start_hits(), "{seen:?}");
        // The state reads as it did before there were notes.
        let got = call(&mut shell, &mut session, "ui.get", json!({}));
        assert!(got["state"].get("whatsNew").is_none());
        assert_eq!(got["whatsNew"]["offered"], false);
        // Help has them all the same: this version's own.
        window.help(&mut shell, &mut session, MENU_ITEM);
        assert_eq!(shell.whats_new_shown(), Some("card"), "{seen:?}");
        assert_eq!(got["whatsNew"]["versions"], json!(["unreleased", "0.3.0"]));
    }
    // After an earlier 3DCraft that kept no note of its version:
    // everything since the first release.
    let earlier = notes(Seen::Earlier);
    assert!(earlier.offered);
    let labels: Vec<String> = earlier.sections.iter().map(|s| s.label()).collect();
    assert_eq!(labels, ["Not yet released", "0.3.0", "0.2.1", "0.2.0"]);
    // A program without notes has no item in Help and no card.
    assert!(WhatsNew::for_start(&Changelog::default(), v("0.3.0"), Seen::Earlier).is_none());
    let (mut shell, mut session, mut window) = started(None, 1100.0, 760.0);
    let help = shell
        .menu_titles()
        .iter()
        .find(|(title, _)| title == "Help")
        .map(|(_, rect)| *rect)
        .unwrap();
    window.click(&mut shell, &mut session, help.center());
    assert!(
        shell
            .menu_items()
            .iter()
            .all(|(label, _)| label != MENU_ITEM)
    );
    assert_eq!(
        call(&mut shell, &mut session, "ui.get", json!({}))["whatsNew"],
        Value::Null
    );
}

#[test]
fn several_skipped_versions_are_all_in_the_card_newest_first() {
    let skipped = notes(Seen::Version(v("0.1.0")));
    assert!(skipped.offered);
    assert_eq!(
        skipped.subtitle(),
        "What changed in what is not released yet, 0.3.0, 0.2.1 and 0.2.0."
    );
    // What changes an answer, of every version, newest first.
    let results: Vec<(&str, String)> = skipped
        .results()
        .iter()
        .map(|(_, section, entry)| (entry.title.as_str(), section.label()))
        .collect();
    assert_eq!(
        results,
        [
            ("A measured volume could be wrong.", "0.3.0".to_string()),
            (
                "A clearance could be reported as proven for parts that overlap.",
                "0.3.0".to_string()
            ),
            ("A proven distance could be too high.", "0.2.0".to_string()),
        ]
    );
    let added: Vec<&str> = skipped
        .added()
        .iter()
        .map(|(_, _, entry)| entry.title.as_str())
        .collect();
    assert_eq!(
        added,
        [
            "A thing not released yet.",
            "A fillet takes one edge by its own name.",
            "Embed."
        ]
    );
    // The rest, kind by kind: changed, fixed, the known limitations.
    let rest: Vec<&str> = skipped
        .rest()
        .iter()
        .map(|(_, _, entry)| entry.heading.as_str())
        .collect();
    assert_eq!(rest, ["Changed", "Fixed", "Fixed", "Known limitations"]);
    let (mut shell, mut session, _window) = started(Some(skipped), 1100.0, 760.0);
    let names: Vec<String> = entries(&shell).into_iter().map(|(name, _)| name).collect();
    assert_eq!(
        names,
        [
            "entry:1:0",
            "entry:1:1",
            "entry:3:0",
            "entry:0:0",
            "entry:1:2",
            "entry:3:1"
        ]
    );
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(
        got["whatsNew"]["versions"],
        json!(["unreleased", "0.3.0", "0.2.1", "0.2.0"])
    );
    assert_eq!(got["whatsNew"]["changesAnswers"], 3);
    // One release alone is summed up by its own words, when it has some.
    let one = WhatsNew::for_start(
        &Changelog::parse("## [0.2.0]\n\nOne fix.\n\n### Fixed\n\n- **x.**\n\n## [0.1.0]\n")
            .unwrap(),
        v("0.2.0"),
        Seen::Earlier,
    )
    .unwrap();
    assert_eq!(one.subtitle(), "One fix.");
}

#[test]
fn help_shows_the_card_on_the_start_screen_and_a_window_over_a_document() {
    let (mut shell, mut session, mut window) = started(Some(notes(Seen::Never)), 1100.0, 760.0);
    assert_eq!(shell.whats_new_shown(), None);
    // On the start screen: the card, where it stands after an update.
    window.help(&mut shell, &mut session, MENU_ITEM);
    assert!(shell.state.whats_new.card && !shell.state.whats_new.window);
    assert_eq!(shell.whats_new_shown(), Some("card"));
    let close = hit(&shell, "close").unwrap();
    window.click(&mut shell, &mut session, close.center());
    assert_eq!(shell.whats_new_shown(), None);
    assert!(shell.take_whats_new_closed());
    // Over a document: a window, which the person asked for.
    run(&mut session, "file.new", json!({}));
    run(&mut session, "feature.block", json!({"size": [10, 10, 10]}));
    window.frames(&mut shell, &mut session, 3);
    assert!(!shell.start_shown());
    window.help(&mut shell, &mut session, MENU_ITEM);
    window.frames(&mut shell, &mut session, 2);
    assert!(shell.state.whats_new.window && !shell.state.whats_new.card);
    assert_eq!(shell.whats_new_shown(), Some("window"));
    let screen = Rect::from_min_size(Pos2::ZERO, window.size);
    let shown = hit(&shell, "window").unwrap();
    assert!(screen.contains_rect(shown), "{shown:?}");
    assert!(hit(&shell, "results").is_some() && hit(&shell, "new").is_some());
    // It is something an Escape closes, so a page that embeds the window
    // keeps the key until then.
    assert!(shell.escape_has_work(&window.ctx, &[]));
    window.key(&mut shell, &mut session, egui::Key::Escape);
    assert!(!shell.state.whats_new.window);
    assert_eq!(shell.whats_new_shown(), None);
    assert!(shell.take_whats_new_closed());
    // Its Close button too.
    window.help(&mut shell, &mut session, MENU_ITEM);
    window.frames(&mut shell, &mut session, 2);
    let close = hit(&shell, "close").unwrap();
    window.click(&mut shell, &mut session, close.center());
    assert!(!shell.state.whats_new.window);
    assert!(shell.take_whats_new_closed());
}

#[test]
fn the_card_never_covers_a_document_and_waits_for_the_start_screen() {
    // A file opened from the command line, by an agent or by a test: the
    // window starts on a document.
    let mut session = Session::new(Workspace::none());
    run(&mut session, "file.new", json!({}));
    run(&mut session, "feature.block", json!({"size": [10, 10, 10]}));
    let mut plain = Shell::new();
    let mut window = Window::sized(1100.0, 760.0);
    window.frames(&mut plain, &mut session, 4);
    let mut shell = Shell::new();
    shell.set_whats_new(Some(notes(Seen::Version(v("0.2.1")))));
    let mut window = Window::sized(1100.0, 760.0);
    window.frames(&mut shell, &mut session, 4);
    assert_eq!(shell.whats_new_shown(), None);
    assert!(shell.whats_new_hits().is_empty());
    assert_eq!(shell.viewport_rect(), plain.viewport_rect());
    // Nothing of the window's lies over the document: an Escape has
    // nothing to close.
    assert!(!shell.escape_has_work(&window.ctx, &[]));
    assert!(!shell.take_whats_new_closed(), "nothing was seen yet");
    // A command and a picture go as they always do.
    let before = call(&mut plain, &mut session, "ui.get", json!({}));
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["startScreen"], false);
    assert_eq!(got["features"], before["features"]);
    assert_eq!(got["whatsNew"]["shown"], Value::Null);
    assert_eq!(got["whatsNew"]["offered"], true);
    run(&mut session, "feature.block", json!({"size": [5, 5, 5]}));
    window.frames(&mut shell, &mut session, 2);
    assert_eq!(shell.whats_new_shown(), None);
    // The card waits: it is there when the start screen comes up.
    run(&mut session, "file.close", json!({"discard": true}));
    window.frames(&mut shell, &mut session, 3);
    assert!(shell.start_shown());
    assert_eq!(shell.whats_new_shown(), Some("card"));
}

#[test]
fn a_presentation_other_than_the_whole_window_shows_no_notes() {
    for presentation in [Presentation::View, Presentation::Form] {
        let mut session = Session::new(Workspace::none());
        let mut shell = Shell::new();
        // The web build, where a page may show less than the whole window.
        shell.allow_presentations(true);
        shell.set_whats_new(Some(notes(Seen::Version(v("0.2.1")))));
        assert!(shell.set_presentation(presentation));
        let mut window = Window::sized(1100.0, 760.0);
        window.frames(&mut shell, &mut session, 4);
        assert_eq!(shell.whats_new_shown(), None, "{presentation:?}");
        assert!(shell.whats_new_hits().is_empty());
        assert!(!shell.start_shown());
        // Not as a window either, whatever the state says.
        call(
            &mut shell,
            &mut session,
            "ui.set",
            json!({"whatsNew": {"window": true}}),
        );
        window.frames(&mut shell, &mut session, 3);
        assert_eq!(shell.whats_new_shown(), None, "{presentation:?}");
        assert!(!shell.take_whats_new_closed());
        // Back in the whole window the card is where it was offered.
        call(
            &mut shell,
            &mut session,
            "ui.set",
            json!({"presentation": "full", "whatsNew": {"window": false}}),
        );
        window.frames(&mut shell, &mut session, 3);
        assert_eq!(shell.whats_new_shown(), Some("card"), "{presentation:?}");
    }
}

/// The examples of the first row, by where the start screen drew them.
fn first_row(shell: &Shell) -> Vec<Rect> {
    let cards = examples(shell);
    let top = cards.iter().map(Rect::top).fold(f32::MAX, f32::min);
    cards
        .into_iter()
        .filter(|card| card.top() < top + 1.0)
        .collect()
}

/// What a person who last saw 0.1.1 is shown when the next release starts
/// today, with the repository's own changelog, at the sizes the start
/// screen was fitted to. The card is never in the way of the examples: it
/// and the first row of examples are wholly in sight at every size. Where
/// the window is too low for the whole card and that row (1024 x 700), the
/// card starts low, its title and one line that says how many results may
/// differ; a click on the line unfolds it. Closed, the start screen is the
/// one from before.
#[test]
fn the_card_and_the_first_row_of_examples_are_in_sight_and_the_card_starts_low_in_a_low_window() {
    let real = next_release();
    assert!(real.offered);
    // Everything after 0.1.1, newest first.
    assert!(real.sections.len() >= 2);
    assert!(
        real.sections
            .windows(2)
            .all(|pair| pair[0].version > pair[1].version)
    );
    assert!(real.sections.iter().all(|s| s.version > Some(v("0.1.1"))));
    let (lead, tail) = real.low_line();
    for (width, height, low) in [
        (1024.0, 700.0, Some(true)),
        (1280.0, 800.0, None),
        (1366.0, 768.0, None),
        (1600.0, 1000.0, Some(false)),
        (1920.0, 1080.0, Some(false)),
        (390.0, 844.0, None),
    ] {
        let at = format!("{width} x {height}");
        let (plain, _, _) = started(None, width, height);
        let (mut shell, mut session, mut window) = started(Some(real.clone()), width, height);
        assert_eq!(shell.whats_new_shown(), Some("card"), "{at}");
        if let Some(low) = low {
            assert_eq!(shell.whats_new_low(), low, "{at}");
        }
        let viewport = shell.viewport_rect().unwrap();
        let card = hit(&shell, "card").unwrap();
        let column = start_hit(&shell, "card");
        // The whole card is in the window without scrolling, no wider
        // than the start screen's column, and New and Open stay above it.
        assert!(
            viewport.contains_rect(card),
            "{at}: {card:?} in {viewport:?}"
        );
        assert!(
            card.left() >= column.left() - 0.5 && card.right() <= column.right() + 0.5,
            "{at}: {card:?} in the column {column:?}"
        );
        for name in ["new", "new-assembly", "open"] {
            let button = start_hit(&shell, name);
            assert!(viewport.contains_rect(button), "{at}: {name}");
            assert!(button.bottom() <= card.top(), "{at}: {name} above the card");
        }
        let close = hit(&shell, "close").unwrap();
        assert!(card.contains_rect(close), "{at}");
        // The first row of examples, each card with its name and its
        // summary, is wholly in sight under it.
        let row = first_row(&shell);
        assert!(!row.is_empty(), "{at}");
        println!(
            "{at}: the card {} ({:.0} points high, to {:.0}); the first row of examples to {:.0}; in sight to {:.0}",
            if shell.whats_new_low() {
                "low"
            } else {
                "whole"
            },
            card.height(),
            card.bottom(),
            row.iter().map(Rect::bottom).fold(f32::MIN, f32::max),
            viewport.bottom()
        );
        for example in &row {
            assert!(
                viewport.contains_rect(*example),
                "{at}: the example {example:?} is not wholly in {viewport:?} (low: {})",
                shell.whats_new_low()
            );
            assert!(example.top() >= card.bottom(), "{at}");
        }
        let got = call(&mut shell, &mut session, "ui.get", json!({}));
        assert_eq!(got["whatsNew"]["low"], shell.whats_new_low(), "{at}");
        if shell.whats_new_low() {
            // Low: the title, the ×, and the one line; no list yet.
            let line = hit(&shell, "summary").unwrap();
            assert!(card.contains_rect(line), "{at}");
            assert!(hit(&shell, "list").is_none() && hit(&shell, "results").is_none());
            // (Higher with touch sizes, on a phone.)
            assert!(card.height() < 120.0, "{at}: {card:?}");
            assert!(!shell.state.whats_new.expanded);
            // A click on the line unfolds the card: what changes an
            // answer first, in a list that keeps to its share.
            window.click(&mut shell, &mut session, line.center());
            window.frames(&mut shell, &mut session, 2);
            assert!(shell.state.whats_new.expanded, "{at}");
            assert!(!shell.whats_new_low(), "{at}");
            assert!(hit(&shell, "summary").is_none(), "{at}");
        } else {
            assert!(hit(&shell, "summary").is_none(), "{at}");
        }
        // Whole: what changes an answer is the first thing in its list,
        // which keeps to its share of the height.
        let card = hit(&shell, "card").unwrap();
        let list = hit(&shell, "list").unwrap();
        assert!(viewport.contains_rect(card), "{at}: {card:?}");
        assert!(list.height() <= height * 0.4, "{at}: {list:?}");
        if let (Some(results), Some(new)) = (hit(&shell, "results"), hit(&shell, "new")) {
            assert!(results.top() < new.top(), "{at}");
            assert!(
                (results.top() - list.top()).abs() < 2.0,
                "{at}: {results:?} in {list:?}"
            );
        }
        // Unfolding its rest or an entry does not make it low again.
        call(
            &mut shell,
            &mut session,
            "ui.set",
            json!({"whatsNew": {"rest": true, "expanded": false}}),
        );
        window.frames(&mut shell, &mut session, 3);
        assert!(!shell.whats_new_low(), "{at}: whole while it is shown");
        // Closed, the start screen is exactly the one without notes: its
        // ten examples where they were.
        let close = hit(&shell, "close").unwrap();
        window.click(&mut shell, &mut session, close.center());
        window.frames(&mut shell, &mut session, 3);
        assert_eq!(shell.whats_new_shown(), None, "{at}");
        assert!(!shell.state.whats_new.expanded, "{at}");
        assert_eq!(shell.start_hits(), plain.start_hits(), "{at}");
    }
    // The low card's line, in words: the results first.
    if !real.results().is_empty() {
        assert!(lead.ends_with("may differ from before"), "{lead}");
        assert!(
            lead.starts_with(&real.results().len().to_string()),
            "{lead}"
        );
    }
    assert!(
        tail.contains("new") || tail.contains("changed and fixed"),
        "{tail}"
    );
}

/// The low card's one line, for every shape of notes.
#[test]
fn the_low_cards_line_says_how_many_results_may_differ() {
    let all = notes(Seen::Version(v("0.1.0")));
    assert_eq!(
        all.low_line(),
        (
            "3 results may differ from before".to_string(),
            ", 3 new, 4 changed and fixed".to_string()
        )
    );
    let one = |text: &str| {
        WhatsNew::for_start(&Changelog::parse(text).unwrap(), v("0.2.0"), Seen::Earlier).unwrap()
    };
    assert_eq!(
        one("## [0.2.0]\n\n### Results that changed\n\n- **x.**\n\n## [0.1.0]\n").low_line(),
        ("1 result may differ from before".to_string(), String::new())
    );
    assert_eq!(
        one("## [0.2.0]\n\n### Fixed\n\n- **x.**\n- **y.**\n\n## [0.1.0]\n").low_line(),
        (String::new(), "2 changed and fixed".to_string())
    );
    assert_eq!(
        one("## [0.2.0]\n\n### Added\n\n- **x.**\n\n### Fixed\n\n- **y.**\n\n## [0.1.0]\n")
            .low_line(),
        (String::new(), "1 new, 1 changed and fixed".to_string())
    );
    // Short notes are whole even in a low window: they fit.
    let short = one("## [0.2.0]\n\n### Fixed\n\n- **x.**\n- **y.**\n\n## [0.1.0]\n");
    let (shell, _, _) = started(Some(short), 1024.0, 700.0);
    assert_eq!(shell.whats_new_shown(), Some("card"));
    assert!(!shell.whats_new_low());
    for example in first_row(&shell) {
        assert!(shell.viewport_rect().unwrap().contains_rect(example));
    }
}

/// What a group says of itself in the changelog (the text under its
/// heading that is not an entry) stands under the group's heading on the
/// card: the changelog's own sentence under the results, in place of the
/// window's; under a group of the rest; and for a group that has no entry
/// at all. Nothing under a group is dropped.
#[test]
fn what_a_group_says_of_itself_stands_under_its_heading() {
    let text = "\
## [0.2.0] - 2026-12-01

### Results that changed

What was answered before can differ now. Check it again.

- **A measure was wrong.**

### Fixed

Two fixes of the window.

- **A fix.**

### Known limitations

None that we know of.

## [0.1.0] - 2026-11-11
";
    let said =
        WhatsNew::for_start(&Changelog::parse(text).unwrap(), v("0.2.0"), Seen::Earlier).unwrap();
    assert_eq!(
        said.said(|_| true),
        [
            (
                "Results that changed",
                "What was answered before can differ now. Check it again."
            ),
            ("Fixed", "Two fixes of the window."),
            ("Known limitations", "None that we know of."),
        ]
    );
    // Drawn: with the notes the results' block and the unfolded rest are
    // higher than the same notes without them, by the lines they take.
    let bare = text
        .replace(
            "What was answered before can differ now. Check it again.\n\n",
            "",
        )
        .replace("Two fixes of the window.\n\n", "")
        .replace("### Known limitations\n\nNone that we know of.\n\n", "");
    let without =
        WhatsNew::for_start(&Changelog::parse(&bare).unwrap(), v("0.2.0"), Seen::Earlier).unwrap();
    assert!(without.said(|_| true).is_empty());
    let drawn = |notes: WhatsNew| {
        let mut session = Session::new(Workspace::none());
        let mut shell = Shell::new();
        shell.set_whats_new(Some(notes));
        shell.state.whats_new.rest = true;
        let mut window = Window::sized(1100.0, 900.0);
        window.frames(&mut shell, &mut session, 4);
        assert!(!shell.whats_new_low());
        (
            hit(&shell, "results").unwrap().height(),
            hit(&shell, "rest").unwrap().height(),
        )
    };
    let (results_with, rest_with) = drawn(said);
    let (results_without, rest_without) = drawn(without);
    // The results: the changelog's sentence (one line) stands where the
    // window's own hint (two lines) would; never both.
    assert!(
        results_with < results_without && results_with > results_without - 40.0,
        "{results_with} and {results_without}"
    );
    // The rest: a line under Fixed, and a group of its own with a line.
    assert!(
        rest_with > rest_without + 40.0,
        "{rest_with} and {rest_without}"
    );
    // The repository's own notes carry their sentence under the results.
    let real = next_release();
    if !real.results().is_empty() {
        let own = real.said(|kind| kind == craft3d_engine::changelog::Kind::Results);
        assert_eq!(own.len(), 1, "{own:?}");
        assert!(own[0].1.ends_with("check it again."), "{own:?}");
    }
}

#[test]
fn the_notes_are_drawn_light_and_dark() {
    let mut places = Vec::new();
    for appearance in [Appearance::Light, Appearance::Dark] {
        let mut session = Session::new(Workspace::none());
        let mut shell = Shell::new();
        shell.state.appearance = appearance;
        shell.set_whats_new(Some(next_release()));
        let mut window = Window::sized(1024.0, 700.0);
        window.frames(&mut shell, &mut session, 4);
        assert_eq!(shell.whats_new_shown(), Some("card"), "{appearance:?}");
        assert_eq!(
            call(&mut shell, &mut session, "ui.get", json!({}))["appearance"],
            if appearance == Appearance::Dark {
                "dark"
            } else {
                "light"
            }
        );
        places.push(shell.whats_new_hits().to_vec());
    }
    // The same places in both: only the colours differ.
    assert_eq!(places[0], places[1]);
}

#[test]
fn ui_set_opens_and_closes_the_notes_without_telling_the_host() {
    // Without notes there is nothing to open.
    let (mut shell, mut session, mut window) = started(None, 1100.0, 760.0);
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"whatsNew": {"card": true, "window": true, "rest": true}}),
    );
    window.frames(&mut shell, &mut session, 2);
    assert!(shell.state.whats_new.is_closed());
    assert_eq!(shell.whats_new_shown(), None);
    // With notes: an agent that shows a person around opens the card.
    let (mut shell, mut session, mut window) = started(Some(notes(Seen::Never)), 1100.0, 760.0);
    assert_eq!(shell.whats_new_shown(), None);
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"whatsNew": {"card": true, "rest": true}}),
    );
    window.frames(&mut shell, &mut session, 2);
    assert_eq!(shell.whats_new_shown(), Some("card"));
    assert!(shell.state.whats_new.rest);
    // Asked for both on the start screen, the card is the one shown.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"whatsNew": {"window": true}}),
    );
    window.frames(&mut shell, &mut session, 2);
    assert_eq!(shell.whats_new_shown(), Some("card"));
    assert!(!shell.state.whats_new.window);
    // Closed by the agent, the person has not closed it: nothing is
    // remembered as seen.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"whatsNew": {"card": false, "rest": false}}),
    );
    window.frames(&mut shell, &mut session, 2);
    assert_eq!(shell.whats_new_shown(), None);
    assert!(!shell.take_whats_new_closed());
    assert!(
        call(&mut shell, &mut session, "ui.get", json!({}))["state"]
            .get("whatsNew")
            .is_none()
    );
    // What it does not know is refused, and nothing changes.
    for bad in [
        json!({"whatsNew": {"bogus": true}}),
        json!({"whatsNew": {"card": "yes"}}),
        json!({"whatsNew": 3}),
    ] {
        let before = shell.state.clone();
        let refused = shell
            .handle_method(&mut session, "ui.set", Some(&bad))
            .unwrap();
        assert!(refused.is_err(), "{bad}");
        assert_eq!(shell.state, before, "{bad}");
    }
    // Taking the notes away closes what was open.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"whatsNew": {"card": true}}),
    );
    shell.set_whats_new(None);
    window.frames(&mut shell, &mut session, 2);
    assert!(shell.state.whats_new.is_closed());
    assert_eq!(shell.whats_new_shown(), None);
}

/// What the card says is the changelog's text without its markup, and the
/// headings are plain words.
#[test]
fn what_the_card_says_is_text_a_person_can_read() {
    let sample = notes(Seen::Version(v("0.1.0")));
    let measured = &sample.results()[0];
    assert_eq!(measured.2.title, "A measured volume could be wrong.");
    assert_eq!(
        measured.2.text,
        "document.measure reported 10 mm³ too little; see the guide."
    );
    let real = next_release();
    for lines in [real.results(), real.added(), real.rest()] {
        for (_, _, entry) in lines {
            assert!(!entry.title.is_empty());
            for text in [&entry.title, &entry.text] {
                assert!(!text.contains("**"), "{text}");
                assert!(!text.contains('`'), "{text}");
                assert!(!text.contains("](http"), "{text}");
            }
        }
    }
    assert_eq!(MENU_ITEM, "What's new");
    assert_eq!(RESULTS_TITLE, "Results that may differ from before");
    assert_eq!(NEW_TITLE, "New");
    assert!(RESULTS_HINT.ends_with("check it again."));
}
