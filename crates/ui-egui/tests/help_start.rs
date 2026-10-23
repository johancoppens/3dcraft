//! The Help menu's windows, the recent files and the start screen: F1
//! opens the shortcuts, `ui.set` opens About, the window notes the files
//! documents come from and go to, and the start screen is up exactly
//! while no document is open. The examples it offers build.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Access, Session, Workspace};
use craft3d_ui_egui::shell::{EXAMPLES, RecentFiles, run_example};
use craft3d_ui_egui::{FileAction, Shell};
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
        .unwrap()
        .unwrap_or_else(|e| panic!("{method} {params}: {e}"))
}

fn workdir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("help-start-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn rooted(dir: &std::path::Path) -> Session {
    Session::new(Workspace {
        read: Access::Root(dir.to_path_buf()),
        write: Access::Root(dir.to_path_buf()),
    })
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
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 760.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
        output.textures_delta.clear();
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
    }
}

#[test]
fn f1_opens_and_closes_the_shortcuts_and_ui_set_opens_about() {
    let mut session = Session::new(Workspace::none());
    let mut shell = Shell::new();
    let mut window = Window::new();
    window.frame(&mut shell, &mut session, Vec::new());
    assert!(!shell.state.dialogs.shortcuts);
    window.key(&mut shell, &mut session, egui::Key::F1);
    assert!(shell.state.dialogs.shortcuts);
    // Drawn for a few frames, as the window lays itself out.
    for _ in 0..4 {
        window.frame(&mut shell, &mut session, Vec::new());
    }
    assert!(shell.state.dialogs.shortcuts);
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["state"]["dialogs"]["shortcuts"], true);
    window.key(&mut shell, &mut session, egui::Key::F1);
    assert!(!shell.state.dialogs.shortcuts);

    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"dialogs": {"about": true}}),
    );
    assert!(shell.state.dialogs.about);
    // Drawn with a document and without one.
    window.frame(&mut shell, &mut session, Vec::new());
    run(&mut session, "file.new", json!({}));
    window.frame(&mut shell, &mut session, Vec::new());
    assert!(shell.state.dialogs.about, "stays open until closed");
    let refused = shell
        .handle_method(
            &mut session,
            "ui.set",
            Some(&json!({"dialogs": {"about": 3}})),
        )
        .unwrap();
    assert!(refused.is_err());
    let unknown = shell
        .handle_method(
            &mut session,
            "ui.set",
            Some(&json!({"dialogs": {"nope": true}})),
        )
        .unwrap();
    assert!(unknown.is_err());
}

#[test]
fn the_start_screen_is_up_while_no_document_is_open() {
    let mut session = Session::new(Workspace::none());
    let mut shell = Shell::new();
    let mut window = Window::new();
    window.frame(&mut shell, &mut session, Vec::new());
    window.frame(&mut shell, &mut session, Vec::new());
    assert!(shell.start_shown());
    assert_eq!(
        call(&mut shell, &mut session, "ui.get", json!({}))["startScreen"],
        true
    );
    run(&mut session, "file.new", json!({}));
    window.frame(&mut shell, &mut session, Vec::new());
    assert!(!shell.start_shown());
    assert_eq!(
        call(&mut shell, &mut session, "ui.get", json!({}))["startScreen"],
        false
    );
    run(&mut session, "file.close", json!({"discard": true}));
    window.frame(&mut shell, &mut session, Vec::new());
    assert!(shell.start_shown());
}

#[test]
fn a_click_on_new_document_asks_the_host_for_a_new_document() {
    let mut session = Session::new(Workspace::none());
    let mut shell = Shell::new();
    let mut window = Window::new();
    // Twice: the card is centred on the height it had in the frame
    // before.
    window.frame(&mut shell, &mut session, Vec::new());
    window.frame(&mut shell, &mut session, Vec::new());
    let hit = |shell: &Shell, name: &str| {
        shell
            .start_hits()
            .iter()
            .find(|(hit, _)| hit == name)
            .map(|(_, rect)| rect.center())
            .unwrap_or_else(|| panic!("{name} on the start screen"))
    };
    let button = hit(&shell, "new");
    assert!(shell.start_hits().len() >= 2 + EXAMPLES.len());
    for pressed in [true, false] {
        window.frame(
            &mut shell,
            &mut session,
            vec![
                Event::PointerMoved(button),
                Event::PointerButton {
                    pos: button,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    assert_eq!(shell.take_file_action(), Some(FileAction::New));
    // An example's link asks for that example.
    let link = hit(&shell, "example:flange");
    for pressed in [true, false] {
        window.frame(
            &mut shell,
            &mut session,
            vec![
                Event::PointerMoved(link),
                Event::PointerButton {
                    pos: link,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    assert_eq!(shell.take_example().map(|e| e.id), Some("flange"));
}

#[test]
fn opened_and_saved_files_become_recent_and_the_list_survives_a_restart() {
    let dir = workdir("recent");
    let mut session = rooted(&dir);
    let mut shell = Shell::new();
    run(&mut session, "file.new", json!({}));
    run(&mut session, "feature.block", json!({"size": [1, 2, 3]}));
    run(&mut session, "file.save", json!({"path": "a.3dcraft"}));
    shell.sync(&mut session);
    run(&mut session, "file.new", json!({}));
    run(&mut session, "file.save", json!({"path": "b.3dcraft"}));
    shell.sync(&mut session);
    let recent = shell.recent_files();
    let names: Vec<&str> = recent.iter().map(|entry| entry.name.as_str()).collect();
    assert_eq!(names, ["b.3dcraft", "a.3dcraft"]);
    // Kept absolute, beneath the root.
    assert_eq!(
        PathBuf::from(&recent[1].path),
        std::path::absolute(dir.join("a.3dcraft")).unwrap()
    );
    assert!(recent.iter().all(|entry| entry.exists));
    let stored = shell.take_recent_changed().expect("changed");
    assert_eq!(shell.take_recent_changed(), None, "taken");
    // Nothing new: nothing changes.
    shell.sync(&mut session);
    assert_eq!(shell.take_recent_changed(), None);

    // A file that went away is listed as missing.
    std::fs::remove_file(dir.join("a.3dcraft")).unwrap();
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["recent"][0]["name"], "b.3dcraft");
    assert_eq!(got["recent"][1]["exists"], false);

    // A new window starts with the stored list; what it noted itself
    // goes in front.
    let mut again = Shell::new();
    run(&mut session, "file.new", json!({}));
    run(&mut session, "file.save", json!({"path": "c.3dcraft"}));
    again.sync(&mut session);
    again.set_recent_files(RecentFiles::from_json(&stored));
    let names: Vec<String> = again.recent_files().into_iter().map(|e| e.name).collect();
    assert_eq!(names, ["c.3dcraft", "b.3dcraft", "a.3dcraft"]);

    // Choosing one asks the host to open it; Clear empties the list.
    again.clear_recent_files();
    assert!(again.recent_files().is_empty());
    assert_eq!(
        RecentFiles::from_json(&again.take_recent_changed().unwrap()),
        RecentFiles::default()
    );
    assert_eq!(again.take_open_recent(), None);
}

#[test]
fn a_document_in_memory_is_not_recent() {
    let mut session = Session::new(Workspace::memory());
    let mut shell = Shell::new();
    run(&mut session, "file.new", json!({}));
    run(&mut session, "file.save", json!({"path": "part.3dcraft"}));
    shell.sync(&mut session);
    assert!(shell.recent_files().is_empty());
}

#[test]
fn every_example_builds_in_a_new_document_and_passes_its_assertions() {
    let mut session = Session::new(Workspace::none());
    for example in &EXAMPLES {
        let built =
            run_example(&mut session, example).unwrap_or_else(|e| panic!("{}: {e}", example.id));
        assert_eq!(built["example"], example.id);
        let listed = run(&mut session, "assert.list", json!({}));
        assert_eq!(listed["ok"], true, "{}: {listed}", example.id);
        let status = session.document_status().unwrap();
        assert!(status.path.is_none(), "an example is a new document");
    }
    assert_eq!(session.document_statuses().len(), EXAMPLES.len());
}

#[test]
fn a_chosen_example_waits_for_the_host() {
    let mut shell = Shell::new();
    assert!(shell.take_example().is_none());
    shell.choose_example(&EXAMPLES[2]);
    assert_eq!(shell.take_example().map(|e| e.id), Some("case"));
    assert!(shell.take_example().is_none());
}
