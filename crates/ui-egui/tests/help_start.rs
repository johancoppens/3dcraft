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
    // The feature list is put away: the viewport, and the welcome centred
    // in it, take the whole width of the window.
    assert_eq!(shell.viewport_rect().unwrap().left(), 0.0);
    let new = shell
        .start_hits()
        .iter()
        .find(|(name, _)| name == "new")
        .map(|(_, rect)| rect.center().x)
        .unwrap();
    assert!(
        (new - 550.0).abs() < 120.0,
        "the buttons near the middle: {new}"
    );
    // A card for each example.
    let cards = shell
        .start_hits()
        .iter()
        .filter(|(name, _)| name.starts_with("example:"))
        .count();
    assert_eq!(cards, EXAMPLES.len());
    run(&mut session, "file.new", json!({}));
    window.frame(&mut shell, &mut session, Vec::new());
    assert!(!shell.start_shown());
    assert!(
        shell.viewport_rect().unwrap().left() > 100.0,
        "the feature list is back"
    );
    assert!(shell.state.panels.tree, "the panel's setting is unchanged");
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

    // A file that went away is listed as missing, once the window looks
    // again (when the menu opens or the start screen comes up; not every
    // frame).
    std::fs::remove_file(dir.join("a.3dcraft")).unwrap();
    let cached = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(
        cached["recent"][1]["exists"], true,
        "not looked at again yet"
    );
    shell.refresh_recent();
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
fn a_file_saved_beneath_another_root_is_recent_by_its_own_place() {
    let dir = workdir("roots");
    let (read, write) = (dir.join("in"), dir.join("out"));
    std::fs::create_dir_all(&read).unwrap();
    std::fs::create_dir_all(&write).unwrap();
    // The same relative name beneath both roots, a different file each.
    let mut maker = rooted(&read);
    run(&mut maker, "file.new", json!({}));
    run(&mut maker, "file.save", json!({"path": "part.3dcraft"}));
    let mut session = Session::new(Workspace {
        read: Access::Root(read.clone()),
        write: Access::Root(write.clone()),
    });
    let mut shell = Shell::new();
    run(&mut session, "file.new", json!({}));
    run(&mut session, "file.save", json!({"path": "part.3dcraft"}));
    shell.sync(&mut session);
    let recent = shell.recent_files();
    assert_eq!(recent.len(), 1);
    assert_eq!(
        PathBuf::from(&recent[0].path),
        std::path::absolute(write.join("part.3dcraft")).unwrap(),
        "where it was written, not the read root's file of that name"
    );
    run(&mut session, "file.open", json!({"path": "part.3dcraft"}));
    shell.sync(&mut session);
    assert_eq!(
        PathBuf::from(&shell.recent_files()[0].path),
        std::path::absolute(read.join("part.3dcraft")).unwrap()
    );
}

#[test]
fn an_agent_is_told_only_of_recent_files_beneath_its_roots() {
    let dir = workdir("told");
    let mut session = rooted(&dir);
    let mut shell = Shell::new();
    run(&mut session, "file.new", json!({}));
    run(&mut session, "file.save", json!({"path": "mine.3dcraft"}));
    shell.sync(&mut session);
    shell.set_recent_files(RecentFiles::from_json(
        r#"{"recent": ["/somewhere/else/private.3dcraft"]}"#,
    ));
    // The window lists both.
    assert_eq!(shell.recent_files().len(), 2);
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    let told: Vec<&str> = got["recent"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["name"].as_str().unwrap())
        .collect();
    assert_eq!(told, ["mine.3dcraft"]);
    // Without any root nothing is told.
    let mut closed = Session::new(Workspace::none());
    let got = call(&mut shell, &mut closed, "ui.get", json!({}));
    assert_eq!(got["recent"], json!([]));
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

#[test]
fn about_copies_the_version_commit_platform_and_renderer() {
    let shell = Shell::new();
    let info = shell.system_info();
    let lines: Vec<&str> = info.lines().collect();
    assert_eq!(
        lines[0],
        format!("3DCraft {}", craft3d_engine::VERSION).as_str()
    );
    assert!(lines[1].starts_with("commit: "), "{info}");
    assert_eq!(
        lines[2],
        format!("platform: {}", craft3d_engine::platform()).as_str()
    );
    assert!(info.contains("renderer: cpu"), "{info}");
    // The same build lines as the bug report.
    let report = craft3d_engine::bug_report(None, false, &[], None);
    assert!(report.contains(lines[1]) && report.contains(lines[2]));
}
