//! The recovery bar and the status bar's word about autosave: what the
//! shell shows of what a crashed 3DCraft left behind, without a window
//! (`egui::Context::run_ui` with synthetic clicks). Recovery itself is
//! tested in the engine (`crates/engine/tests/recovery.rs`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{ManualClock, RecoveryOptions, RecoveryOverview, Session, Workspace};
use craft3d_ui_egui::{RecoveryAction, Shell, autosave_text};
use egui::{Event, PointerButton, Pos2, vec2};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn workdir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("ui-recovery-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// A session that autosaves into `dir`, by a clock that stands still.
fn session(dir: &Path) -> Session {
    let mut s = Session::new(Workspace::none());
    s.enable_recovery(RecoveryOptions::manual(
        dir,
        Arc::new(ManualClock::new(1_800_000_000)),
    ));
    s
}

/// Leaves `count` autosaved documents behind as a crash does: the session
/// is dropped without its clean end.
fn crash(dir: &Path, count: usize) {
    let mut s = session(dir);
    for size in 1..=count {
        run(&mut s, "file.new", json!({}));
        run(&mut s, "feature.block", json!({"size": [size, 2, 3]}));
    }
    s.autosave_now();
}

fn files(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .unwrap()
        .filter(|e| {
            let name = e.as_ref().unwrap().file_name();
            name.to_str().unwrap().ends_with(".3dcraft")
        })
        .count()
}

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

    fn frame(&mut self, shell: &mut Shell, session: &mut Session, events: Vec<Event>) {
        self.time += 1.0 / 60.0;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 700.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
        // No backend to upload the textures to.
        output.textures_delta.clear();
    }

    fn idle(&mut self, shell: &mut Shell, session: &mut Session) {
        for _ in 0..3 {
            self.frame(shell, session, Vec::new());
        }
    }

    /// Clicks the button of the recovery bar that asks for `action`.
    fn click(&mut self, shell: &mut Shell, session: &mut Session, action: &RecoveryAction) {
        let at = shell
            .recovery_hits()
            .iter()
            .find(|(asks, _)| asks == action)
            .unwrap_or_else(|| panic!("no button for {action:?}: {:?}", shell.recovery_hits()))
            .1
            .center();
        self.frame(shell, session, vec![Event::PointerMoved(at)]);
        for pressed in [true, false] {
            self.frame(
                shell,
                session,
                vec![Event::PointerButton {
                    pos: at,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        self.idle(shell, session);
    }
}

fn ui_get(shell: &mut Shell, session: &mut Session) -> Value {
    shell
        .handle_method(session, "ui.get", None)
        .unwrap()
        .unwrap()
}

#[test]
fn nothing_to_recover_shows_no_bar() {
    let dir = workdir("nothing");
    let mut s = session(&dir);
    let mut shell = Shell::new();
    let mut window = Window::new();
    window.idle(&mut shell, &mut s);
    assert!(shell.recovery_hits().is_empty());
    let menu_only = shell.viewport_rect().unwrap().top();
    assert!(menu_only < 40.0, "{menu_only}");
    let got = ui_get(&mut shell, &mut s);
    assert_eq!(got["recovery"]["enabled"], true);
    assert_eq!(got["recovery"]["documents"], json!([]));
    assert_eq!(got["state"]["panels"]["recovery"], true);
    // A session without a recovery directory says so, and shows nothing.
    let mut plain = Session::new(Workspace::none());
    window.idle(&mut shell, &mut plain);
    assert!(shell.recovery_hits().is_empty());
    assert_eq!(ui_get(&mut shell, &mut plain)["recovery"]["enabled"], false);
}

#[test]
fn the_bar_lists_what_was_found_and_recover_opens_it() {
    let dir = workdir("recover");
    crash(&dir, 2);
    let mut s = session(&dir);
    let mut shell = Shell::new();
    let mut window = Window::new();
    window.idle(&mut shell, &mut s);

    // Nothing was opened or removed by starting.
    assert!(!s.has_document());
    assert_eq!(files(&dir), 2);
    let found = shell.recovery().found.clone();
    assert_eq!(found.len(), 2);
    let got = ui_get(&mut shell, &mut s);
    assert_eq!(got["recovery"]["documents"].as_array().unwrap().len(), 2);
    assert_eq!(got["recovery"]["documents"][0]["id"], json!(found[0].id));

    // The bar is between the menu and the viewport, with a Recover and a
    // Discard per document, Discard all and Later.
    let top = shell.viewport_rect().unwrap().top();
    assert!(top > 60.0, "the bar takes room: {top}");
    let asks: Vec<RecoveryAction> = shell
        .recovery_hits()
        .iter()
        .map(|(asks, _)| asks.clone())
        .collect();
    for wanted in [
        RecoveryAction::Later,
        RecoveryAction::DiscardAll,
        RecoveryAction::Open(found[0].id.clone()),
        RecoveryAction::Discard(found[0].id.clone()),
        RecoveryAction::Open(found[1].id.clone()),
        RecoveryAction::Discard(found[1].id.clone()),
    ] {
        assert!(asks.contains(&wanted), "{wanted:?} in {asks:?}");
    }
    assert_eq!(asks.len(), 6);
    for (_, rect) in shell.recovery_hits() {
        assert!(rect.bottom() <= top && rect.top() > 12.0, "{rect:?}");
    }

    // Recover: the document is open, through the command, and the bar
    // lists one less.
    window.click(
        &mut shell,
        &mut s,
        &RecoveryAction::Open(found[1].id.clone()),
    );
    assert!(s.has_document());
    let report = shell.last_command.clone().unwrap();
    assert_eq!((report.label.as_str(), report.ok), ("recovery.open", true));
    assert_eq!(shell.tree_rows().len(), 1);
    let listed = run(&mut s, "file.list", json!({}));
    assert_eq!(listed["documents"][0]["recovered"], true);
    assert_eq!(shell.recovery().found.len(), 1);
    assert_eq!(shell.recovery_hits().len(), 4);

    // The other one: the bar has nothing left to offer and goes.
    window.click(
        &mut shell,
        &mut s,
        &RecoveryAction::Open(found[0].id.clone()),
    );
    assert_eq!(
        run(&mut s, "file.list", json!({}))["documents"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(shell.recovery_hits().is_empty());
    // The viewport begins where it does in a window without the bar that
    // has documents open (the row of their tabs takes room of its own).
    assert_eq!(shell.viewport_rect().unwrap().top(), {
        let mut plain = Session::new(Workspace::none());
        run(&mut plain, "file.new", json!({}));
        let mut other = Shell::new();
        Window::new().idle(&mut other, &mut plain);
        other.viewport_rect().unwrap().top()
    });
    // Their recovery files are this session's now, until they are saved.
    assert_eq!(files(&dir), 2);
}

#[test]
fn discard_asks_twice_and_later_hides_the_bar_without_touching_anything() {
    let dir = workdir("discard");
    crash(&dir, 3);
    let mut s = session(&dir);
    let mut shell = Shell::new();
    let mut window = Window::new();
    window.idle(&mut shell, &mut s);
    let found = shell.recovery().found.clone();
    let first = RecoveryAction::Discard(found[0].id.clone());
    let second = RecoveryAction::Discard(found[1].id.clone());

    // One click arms it and removes nothing.
    window.click(&mut shell, &mut s, &first);
    assert_eq!(shell.discard_armed(), Some(&first));
    assert_eq!(files(&dir), 3);
    assert!(shell.last_command.is_none());
    // A click on another discard arms that one instead.
    window.click(&mut shell, &mut s, &second);
    assert_eq!(shell.discard_armed(), Some(&second));
    assert_eq!(files(&dir), 3);
    // The second click on the same one does it.
    window.click(&mut shell, &mut s, &second);
    assert_eq!(shell.discard_armed(), None);
    assert_eq!(files(&dir), 2);
    let report = shell.last_command.clone().unwrap();
    assert_eq!(
        (report.label.as_str(), report.ok),
        ("recovery.discard", true)
    );
    assert_eq!(shell.recovery().found.len(), 2);
    assert!(!s.has_document());

    // Later: the bar goes, the files stay, and `ui.set` brings it back.
    window.click(&mut shell, &mut s, &RecoveryAction::DiscardAll);
    assert_eq!(shell.discard_armed(), Some(&RecoveryAction::DiscardAll));
    window.click(&mut shell, &mut s, &RecoveryAction::Later);
    assert_eq!(shell.discard_armed(), None);
    assert!(shell.recovery_hits().is_empty());
    assert!(!shell.state.panels.recovery);
    assert_eq!(files(&dir), 2);
    assert_eq!(
        ui_get(&mut shell, &mut s)["recovery"]["documents"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    shell
        .handle_method(
            &mut s,
            "ui.set",
            Some(&json!({"panels": {"recovery": true}})),
        )
        .unwrap()
        .unwrap();
    window.idle(&mut shell, &mut s);
    assert_eq!(shell.recovery_hits().len(), 6);

    // Discard all, twice: everything is gone, and so is the bar.
    window.click(&mut shell, &mut s, &RecoveryAction::DiscardAll);
    assert_eq!(files(&dir), 2);
    window.click(&mut shell, &mut s, &RecoveryAction::DiscardAll);
    assert_eq!(files(&dir), 0);
    assert!(shell.recovery_hits().is_empty());
}

#[test]
fn a_document_that_cannot_be_opened_is_listed_with_recover_disabled() {
    let dir = workdir("damaged");
    crash(&dir, 1);
    let file = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|e| e == "3dcraft"))
        .unwrap();
    std::fs::write(&file, b"not a document").unwrap();
    let mut s = session(&dir);
    let mut shell = Shell::new();
    let mut window = Window::new();
    window.idle(&mut shell, &mut s);
    let found = shell.recovery().found.clone();
    assert!(!found[0].openable);
    // A click on the disabled button does nothing; the command says why.
    window.click(
        &mut shell,
        &mut s,
        &RecoveryAction::Open(found[0].id.clone()),
    );
    assert!(!s.has_document());
    assert!(shell.last_command.is_none());
    shell.recovery_act(&mut s, RecoveryAction::Open(found[0].id.clone()));
    let report = shell.last_command.clone().unwrap();
    assert!(
        !report.ok && report.text.contains("cannot be opened"),
        "{report:?}"
    );
    assert!(file.exists());
}

#[test]
fn the_status_bar_says_when_a_document_was_autosaved_or_why_not() {
    let mut overview = RecoveryOverview::default();
    assert_eq!(autosave_text(&overview, Some(1_000)), None);
    overview.last_autosave = Some(1_000);
    let said = |overview: &RecoveryOverview, now| autosave_text(overview, now).unwrap();
    assert_eq!(
        said(&overview, Some(1_000)),
        ("autosaved just now".to_string(), false)
    );
    assert_eq!(
        said(&overview, Some(1_059)),
        ("autosaved just now".to_string(), false)
    );
    assert_eq!(
        said(&overview, Some(1_060)),
        ("autosaved 1 min ago".to_string(), false)
    );
    assert_eq!(
        said(&overview, Some(1_000 + 3_599)),
        ("autosaved 59 min ago".to_string(), false)
    );
    assert_eq!(
        said(&overview, Some(1_000 + 7_300)),
        ("autosaved 2 h ago".to_string(), false)
    );
    // A clock that went back, or none at all.
    assert_eq!(
        said(&overview, Some(5)),
        ("autosaved just now".to_string(), false)
    );
    assert_eq!(said(&overview, None), ("autosaved".to_string(), false));
    // The problem wins: the last autosave is older than the work.
    overview.problem = Some("cannot write in the recovery directory `/r`: disk full".to_string());
    let (text, problem) = said(&overview, Some(2_000));
    assert!(
        problem && text.starts_with("autosave failed: cannot write"),
        "{text}"
    );

    // In a session: a recovery directory that cannot be made is a problem
    // the shell knows of, `ui.get` shows, and a frame draws.
    let dir = workdir("problem");
    std::fs::write(dir.join("file"), b"in the way").unwrap();
    let mut s = session(&dir.join("file"));
    run(&mut s, "file.new", json!({}));
    run(&mut s, "feature.block", json!({"size": [1, 2, 3]}));
    s.autosave_now();
    let mut shell = Shell::new();
    let mut window = Window::new();
    window.idle(&mut shell, &mut s);
    let (text, problem) = autosave_text(shell.recovery(), Some(0)).unwrap();
    assert!(problem && text.contains("recovery directory"), "{text}");
    let got = ui_get(&mut shell, &mut s);
    assert!(
        got["recovery"]["problem"]
            .as_str()
            .unwrap()
            .contains("recovery directory")
    );
    // The work goes on.
    run(&mut s, "feature.block", json!({"size": [1, 2, 3]}));
    window.idle(&mut shell, &mut s);
    assert_eq!(shell.tree_rows().len(), 2);
}
