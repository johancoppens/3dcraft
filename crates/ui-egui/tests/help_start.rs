//! The Help menu's windows, the recent files and the start screen: F1
//! opens the shortcuts, `ui.set` opens About, the window notes the files
//! documents come from and go to, and the start screen is up exactly
//! while no document is open. Its examples are listed and opened by
//! `ui.example` too (each one is opened by `bundled_examples.rs`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Access, Session, Workspace};
use craft3d_ui_egui::shell::{EXAMPLES, EXAMPLES_URL, ExampleSection, External, RecentFiles};
use craft3d_ui_egui::{FileAction, Shell, StatusSlot, TabPart};
use egui::{Event, Pos2, vec2};
use serde_json::{Value, json};
use std::path::PathBuf;

/// The examples of one section of the start screen.
fn in_section(section: ExampleSection) -> usize {
    EXAMPLES.iter().filter(|e| e.section == section).count()
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
    size: egui::Vec2,
}

impl Window {
    fn new() -> Self {
        Self::sized(1100.0, 760.0)
    }

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
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, self.size)),
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
    // in it, take the whole width of the window. The content is a column
    // in the middle (board f1), its header and buttons at its left.
    assert_eq!(shell.viewport_rect().unwrap().left(), 0.0);
    let hit = |name: &str| {
        shell
            .start_hits()
            .iter()
            .find(|(hit, _)| hit == name)
            .map(|(_, rect)| *rect)
            .unwrap()
    };
    let column = hit("card");
    assert!(
        (column.center().x - 550.0).abs() < 2.0,
        "the column in the middle: {column:?}"
    );
    let new = hit("new");
    assert!(
        (new.left() - column.left()).abs() < 2.0,
        "New part at the column's left: {new:?} in {column:?}"
    );
    // A card for each example of the section shown (the parts first),
    // and the sections to choose from.
    let cards = shell
        .start_hits()
        .iter()
        .filter(|(name, _)| name.starts_with("example:"))
        .count();
    assert_eq!(cards, in_section(ExampleSection::Parts));
    for section in ExampleSection::ALL {
        let hit = format!("section:{}", section.word());
        assert!(
            shell.start_hits().iter().any(|(name, _)| *name == hit),
            "{hit}"
        );
    }
    // The first two are marked for a newcomer, and More examples is there.
    assert!(cards >= 2 && EXAMPLES[0].start_here() && EXAMPLES[1].start_here());
    assert!(
        shell
            .start_hits()
            .iter()
            .any(|(name, _)| name == "more-examples")
    );
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
fn every_section_fits_on_the_start_screen_at_common_window_sizes() {
    assert_eq!(in_section(ExampleSection::Parts), 10);
    for section in ExampleSection::ALL {
        let count = in_section(section);
        assert!((1..=10).contains(&count), "{section:?}: {count}");
        for (width, height) in [
            (1024.0, 700.0),
            (1280.0, 800.0),
            (1366.0, 768.0),
            (1600.0, 1000.0),
        ] {
            let mut session = Session::new(Workspace::none());
            let mut shell = Shell::new();
            shell.show_section(section);
            let mut window = Window::sized(width, height);
            for _ in 0..3 {
                window.frame(&mut shell, &mut session, Vec::new());
            }
            let viewport = shell.viewport_rect().unwrap();
            let cards: Vec<egui::Rect> = shell
                .start_hits()
                .iter()
                .filter(|(name, _)| name.starts_with("example:"))
                .map(|(_, rect)| *rect)
                .collect();
            assert_eq!(cards.len(), count, "{section:?} {width} x {height}");
            // At most two rows of five, every card inside the viewport:
            // nothing to scroll to.
            let mut rows: Vec<f32> = cards.iter().map(|r| r.top().round()).collect();
            rows.dedup();
            assert_eq!(
                rows.len(),
                count.div_ceil(5),
                "{section:?} {width} x {height}: rows at {rows:?}"
            );
            for card in &cards {
                assert!(
                    viewport.contains_rect(*card),
                    "{section:?} {width} x {height}: card {card:?} outside {viewport:?}"
                );
            }
            // A full row fills the column: no empty room at its right.
            if count >= 5 {
                let column = shell
                    .start_hits()
                    .iter()
                    .find(|(name, _)| name == "card")
                    .map(|(_, rect)| *rect)
                    .unwrap();
                let right = cards.iter().map(|c| c.right()).fold(f32::MIN, f32::max);
                assert!(
                    (column.right() - right).abs() < 2.0,
                    "{section:?} {width} x {height}: the row ends at {right}, the column at {}",
                    column.right()
                );
            }
        }
    }
}

#[test]
fn on_a_phone_the_cards_stay_within_the_width() {
    for section in ExampleSection::ALL {
        let mut session = Session::new(Workspace::none());
        let mut shell = Shell::new();
        shell.show_section(section);
        let mut window = Window::sized(390.0, 844.0);
        for _ in 0..3 {
            window.frame(&mut shell, &mut session, Vec::new());
        }
        let viewport = shell.viewport_rect().unwrap();
        let cards: Vec<egui::Rect> = shell
            .start_hits()
            .iter()
            .filter(|(name, _)| name.starts_with("example:"))
            .map(|(_, rect)| *rect)
            .collect();
        assert!(!cards.is_empty(), "{section:?}");
        for card in &cards {
            assert!(
                card.left() >= viewport.left() - 0.5 && card.right() <= viewport.right() + 0.5,
                "{section:?}: card {card:?} wider than {viewport:?}"
            );
        }
    }
}

/// The phone's start screen (board m1): the top bar names 3DCraft (its
/// menu, the tab row's All, offers New and Open), and the status bar the
/// program and its version instead of "no document".
#[test]
fn a_phone_start_screen_has_the_top_bar_and_the_version() {
    let mut session = Session::new(Workspace::none());
    let mut shell = Shell::new();
    let mut window = Window::sized(390.0, 844.0);
    for _ in 0..3 {
        window.frame(&mut shell, &mut session, Vec::new());
    }
    assert!(
        shell
            .tab_hits()
            .iter()
            .any(|(part, _)| *part == TabPart::All),
        "{:?}",
        shell.tab_hits()
    );
    let units = shell.status_shown(StatusSlot::Units).unwrap_or_default();
    assert!(units.starts_with("3DCraft "), "{units}");
    // A window keeps "no document".
    let mut shell = Shell::new();
    let mut window = Window::sized(1280.0, 800.0);
    for _ in 0..3 {
        window.frame(&mut shell, &mut session, Vec::new());
    }
    assert_eq!(shell.status_shown(StatusSlot::Units), Some("no document"));
}

#[test]
fn a_click_on_a_section_shows_its_cards() {
    let mut session = Session::new(Workspace::none());
    let mut shell = Shell::new();
    let mut window = Window::sized(1280.0, 800.0);
    for _ in 0..3 {
        window.frame(&mut shell, &mut session, Vec::new());
    }
    assert_eq!(shell.start_section(), ExampleSection::Parts);
    let at = shell
        .start_hits()
        .iter()
        .find(|(name, _)| name == "section:assemblies")
        .map(|(_, rect)| rect.center())
        .unwrap();
    for pressed in [true, false] {
        window.frame(
            &mut shell,
            &mut session,
            vec![egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }],
        );
    }
    window.frame(&mut shell, &mut session, Vec::new());
    assert_eq!(shell.start_section(), ExampleSection::Assemblies);
    let cards: Vec<&str> = shell
        .start_hits()
        .iter()
        .filter_map(|(name, _)| name.strip_prefix("example:"))
        .collect();
    let assemblies: Vec<&str> = EXAMPLES
        .iter()
        .filter(|e| e.section == ExampleSection::Assemblies)
        .map(|e| e.id)
        .collect();
    assert_eq!(cards, assemblies);
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
    assert!(shell.start_hits().len() >= 2 + in_section(ExampleSection::Parts));
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
    // An example's card asks for that example.
    let link = hit(&shell, "example:steel-bookend");
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
    assert_eq!(shell.take_example().map(|e| e.id), Some("steel-bookend"));
    // More examples asks the host to open the galleries in the browser, at
    // the part of the section shown (the website's anchors).
    assert_eq!(EXAMPLES_URL, "https://3dcraft.dev/examples/");
    for (section, url) in [
        (
            ExampleSection::Parts,
            "https://3dcraft.dev/examples/#examples",
        ),
        (ExampleSection::Jigs, "https://3dcraft.dev/examples/#jigs"),
        (
            ExampleSection::Assemblies,
            "https://3dcraft.dev/examples/#assemblies",
        ),
    ] {
        assert_eq!(section.more_url(), url);
        shell.show_section(section);
        window.frame(&mut shell, &mut session, Vec::new());
        let more = hit(&shell, "more-examples");
        for pressed in [true, false] {
            window.frame(
                &mut shell,
                &mut session,
                vec![
                    Event::PointerMoved(more),
                    Event::PointerButton {
                        pos: more,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert_eq!(shell.take_external(), Some(External::Url(url.to_string())));
    }
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
fn a_chosen_example_waits_for_the_host() {
    let mut shell = Shell::new();
    assert!(shell.take_example().is_none());
    let jig = EXAMPLES.iter().find(|e| e.id == "laser-box").unwrap();
    shell.choose_example(jig);
    assert_eq!(shell.take_example().map(|e| e.id), Some("laser-box"));
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

/// `ui.example` lists the start screen's examples, shows a section and
/// opens one as a click does; what it cannot do it refuses and says why.
#[test]
fn ui_example_lists_shows_and_opens_the_examples() {
    let mut session = Session::new(Workspace::none());
    let mut shell = Shell::new();
    let mut window = Window::new();
    window.frame(&mut shell, &mut session, Vec::new());
    window.frame(&mut shell, &mut session, Vec::new());
    let listed = call(&mut shell, &mut session, "ui.example", json!({}));
    assert_eq!(listed["shown"], true);
    assert_eq!(listed["section"], "parts");
    assert_eq!(
        listed["moreExamples"],
        "https://3dcraft.dev/examples/#examples"
    );
    assert_eq!(
        call(&mut shell, &mut session, "ui.get", json!({}))["startSection"],
        "parts"
    );
    let examples = listed["examples"].as_array().unwrap();
    assert_eq!(examples.len(), EXAMPLES.len());
    let ids: Vec<&str> = examples.iter().map(|e| e["id"].as_str().unwrap()).collect();
    let expected: Vec<&str> = EXAMPLES.iter().map(|e| e.id).collect();
    assert_eq!(ids, expected);
    assert_eq!(examples[0]["kind"], "part");
    assert_eq!(examples[0]["startHere"], true);
    // As its repository measured it, in hundredths of a second.
    let seconds = examples[0]["seconds"].as_f64().unwrap();
    assert_eq!(seconds, (seconds * 100.0).round() / 100.0, "{seconds}");
    assert!(examples[0]["file"].as_str().unwrap().ends_with(".3dcraft"));
    for e in examples {
        let kind = e["kind"].as_str().unwrap();
        let section = e["section"].as_str().unwrap();
        assert_eq!(
            (kind, section),
            match kind {
                "part" => ("part", "parts"),
                "jig" => ("jig", "jigs"),
                _ => ("assembly", "assemblies"),
            },
            "{e}"
        );
    }
    // A section, as a click on its tab.
    let shown = call(
        &mut shell,
        &mut session,
        "ui.example",
        json!({"section": "jigs"}),
    );
    assert_eq!(shown["section"], "jigs");
    assert_eq!(shown["moreExamples"], "https://3dcraft.dev/examples/#jigs");
    assert_eq!(shell.start_section(), ExampleSection::Jigs);
    // ui.get says which section shows.
    assert_eq!(
        call(&mut shell, &mut session, "ui.get", json!({}))["startSection"],
        "jigs"
    );
    window.frame(&mut shell, &mut session, Vec::new());
    let cards = shell
        .start_hits()
        .iter()
        .filter(|(name, _)| name.starts_with("example:"))
        .count();
    assert_eq!(cards, in_section(ExampleSection::Jigs));
    // An example, as a click on its card: the host opens it.
    let chosen = call(
        &mut shell,
        &mut session,
        "ui.example",
        json!({"id": "radial-engine"}),
    );
    assert_eq!(chosen["chosen"]["id"], "radial-engine");
    assert!(
        !shell.state.timeline.motion,
        "the player follows once it is open"
    );
    assert_eq!(shell.take_example().map(|e| e.id), Some("radial-engine"));
    // Refused, and nothing chosen.
    for params in [
        json!({"id": "nope"}),
        json!({"id": 3}),
        json!({"section": "tools"}),
        json!({"id": "moka-pot", "section": "parts"}),
        json!({"what": 1}),
        json!([1]),
    ] {
        let refused = shell
            .handle_method(&mut session, "ui.example", Some(&params))
            .unwrap();
        assert!(refused.is_err(), "{params}");
        assert!(shell.take_example().is_none(), "{params}");
    }
    let unknown = shell
        .handle_method(&mut session, "ui.example", Some(&json!({"id": "nope"})))
        .unwrap()
        .unwrap_err();
    assert!(unknown.contains("moka-pot"), "it lists the ids: {unknown}");
}

/// An example from the start screen that was only opened is no unsaved
/// work (docs/decisions.md, 2026-11-10): its tab carries its title and no
/// unsaved mark, the window's title names it, closing it or the window
/// asks nothing, and it is no recent file. The first edit marks it, and
/// then closing it asks, by its title. An assembly's player is put on its
/// motion only once it is open.
#[test]
fn an_opened_example_is_named_on_its_tab_and_asks_nothing_until_changed() {
    use craft3d_ui_egui::shell::run_example;
    use craft3d_ui_egui::{DocAction, window_title};
    let mut session = Session::new(Workspace::memory());
    let mut shell = Shell::new();
    let mut window = Window::new();
    window.frame(&mut shell, &mut session, Vec::new());
    let profile = EXAMPLES.iter().find(|e| e.id == "t-slot-profile").unwrap();
    shell.choose_example(profile);
    let example = shell.take_example().unwrap();
    let opened = run_example(&mut session, example);
    shell.example_opened(&opened);
    assert!(opened.is_ok(), "{opened:?}");
    assert!(!shell.state.timeline.motion, "a part plays its history");
    window.frame(&mut shell, &mut session, Vec::new());
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    let tab = &got["tabs"][0];
    assert_eq!(tab["name"], profile.title, "{tab}");
    assert_eq!(tab["dirty"], false, "{tab}");
    assert_eq!(tab["asksToClose"], false, "{tab}");
    assert_eq!(tab["path"], Value::Null);
    assert_eq!(got["document"]["title"], profile.title);
    assert_eq!(got["document"]["dirty"], false);
    let status = session.document_status().unwrap();
    assert_eq!(
        window_title(Some(&status)),
        format!("{} – 3DCraft", profile.title)
    );
    assert!(shell.recent_files().is_empty(), "no file of its own");
    // Closing its tab, and the window, ask nothing.
    shell.request_close(0);
    assert!(shell.closing().is_none());
    assert_eq!(
        shell.take_doc_action(),
        Some(DocAction::Close {
            document: 0,
            discard: false
        })
    );
    assert!(shell.request_window_close(), "nothing to ask about");

    // Edited: marked, and the questions come, by its title.
    let mut session = Session::new(Workspace::memory());
    let mut shell = Shell::new();
    let opened = run_example(&mut session, profile);
    shell.example_opened(&opened);
    run(&mut session, "feature.block", json!({"size": [1, 1, 1]}));
    window.frame(&mut shell, &mut session, Vec::new());
    let tab = &call(&mut shell, &mut session, "ui.get", json!({}))["tabs"][0];
    assert_eq!(tab["name"], profile.title);
    assert_eq!(
        (tab["dirty"].clone(), tab["asksToClose"].clone()),
        (json!(true), json!(true))
    );
    let status = session.document_status().unwrap();
    assert_eq!(
        window_title(Some(&status)),
        format!("{}* – 3DCraft", profile.title)
    );
    shell.request_close(0);
    let closing = shell.closing().unwrap();
    assert_eq!(closing.name, profile.title);
    assert!(closing.path.is_none() && !closing.jig);
    shell.answer_closing(craft3d_ui_egui::CloseAnswer::Cancel);
    assert!(!shell.request_window_close(), "it asks first");

    // An assembly: on its motion once it is open.
    let mut session = Session::new(Workspace::memory());
    let mut shell = Shell::new();
    let clamp = EXAMPLES.iter().find(|e| e.id == "toggle-clamp").unwrap();
    shell.choose_example(clamp);
    assert!(!shell.state.timeline.motion);
    let example = shell.take_example().unwrap();
    let opened = run_example(&mut session, example);
    shell.example_opened(&opened);
    assert!(
        shell.state.timeline.motion,
        "an assembly opens on its motion"
    );
    let status = session.document_status().unwrap();
    assert!(status.assembly && !status.dirty && !status.close_asks);
    assert_eq!(status.title.as_deref(), Some(clamp.title));
}
