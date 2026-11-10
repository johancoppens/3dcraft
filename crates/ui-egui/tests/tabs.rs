//! The row of tabs above the viewport: one per open document in the order
//! of `file.list`, named after its file, marked when it has unsaved
//! changes; a click asks the host for `file.activate`, the close button
//! for `file.close`, after a question when there is something to lose.
//! The shell only asks: these tests run the commands the host would.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Access, Session, Workspace};
use craft3d_ui_egui::{CloseAnswer, DocAction, Shell, TabPart};
use egui::{Event, PointerButton, Pos2, pos2, vec2};
use serde_json::{Value, json};
use std::path::PathBuf;

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn workdir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("tabs-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Four documents: `gear` saved, `bracket` saved and changed since, a new
/// one nothing was added to, and a new one with a block. The last one is
/// active.
fn four(name: &str) -> Session {
    let dir = workdir(name);
    std::fs::create_dir_all(dir.join("parts")).unwrap();
    let mut s = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir),
    });
    run(&mut s, "file.new", json!({}));
    run(&mut s, "feature.block", json!({"size": [10, 10, 10]}));
    run(&mut s, "file.save", json!({"path": "gear.3dcraft"}));
    run(&mut s, "file.new", json!({}));
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": 4, "height": 9}),
    );
    run(
        &mut s,
        "file.save",
        json!({"path": "parts/bracket.3dcraft"}),
    );
    run(&mut s, "feature.sphere", json!({"radius": 2}));
    run(&mut s, "file.new", json!({}));
    run(&mut s, "file.new", json!({}));
    run(&mut s, "feature.block", json!({"size": [5, 6, 7]}));
    s
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

    fn idle(&mut self, shell: &mut Shell, session: &mut Session, frames: usize) {
        for _ in 0..frames {
            self.frame(shell, session, Vec::new());
        }
    }

    fn click_with(
        &mut self,
        shell: &mut Shell,
        session: &mut Session,
        at: Pos2,
        button: PointerButton,
    ) {
        self.frame(shell, session, vec![Event::PointerMoved(at)]);
        for pressed in [true, false] {
            self.frame(
                shell,
                session,
                vec![Event::PointerButton {
                    pos: at,
                    button,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
    }

    fn click(&mut self, shell: &mut Shell, session: &mut Session, at: Pos2) {
        self.click_with(shell, session, at, PointerButton::Primary);
    }

    fn key(&mut self, shell: &mut Shell, session: &mut Session, key: egui::Key, shift: bool) {
        let modifiers = if shift {
            egui::Modifiers::CTRL | egui::Modifiers::SHIFT
        } else {
            egui::Modifiers::CTRL
        };
        let modifiers = if key == egui::Key::W {
            egui::Modifiers::COMMAND
        } else {
            modifiers
        };
        self.frame(
            shell,
            session,
            vec![Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }],
        );
    }
}

fn hit(shell: &Shell, part: TabPart) -> egui::Rect {
    shell
        .tab_hits()
        .iter()
        .find(|(p, _)| *p == part)
        .map(|(_, rect)| *rect)
        .unwrap_or_else(|| panic!("{part:?} is not drawn: {:?}", shell.tab_hits()))
}

/// Does what the host does with an action: the commands, then a sync.
fn host(shell: &mut Shell, session: &mut Session) -> Option<DocAction> {
    let action = shell.take_doc_action()?;
    match action {
        DocAction::Activate(document) => {
            run(session, "file.activate", json!({ "document": document }));
        }
        DocAction::Close { document, discard } => {
            run(session, "file.activate", json!({ "document": document }));
            run(session, "file.close", json!({ "discard": discard }));
        }
        DocAction::SaveAndClose(document) => {
            run(session, "file.activate", json!({ "document": document }));
            run(session, "file.save", json!({}));
            run(session, "file.close", json!({}));
        }
    }
    shell.sync(session);
    Some(action)
}

fn names(shell: &Shell) -> Vec<String> {
    shell.tabs().iter().map(|tab| tab.name.clone()).collect()
}

#[test]
fn the_tabs_are_the_open_documents_in_order_named_marked_and_in_ui_get() {
    let mut session = four("order");
    let mut shell = Shell::new();
    let mut window = Window::new(1200.0, 700.0);
    window.idle(&mut shell, &mut session, 2);
    assert_eq!(
        names(&shell),
        ["gear", "bracket", "Untitled 1", "Untitled 2"]
    );
    let flags: Vec<(usize, bool, bool, bool)> = shell
        .tabs()
        .iter()
        .map(|tab| (tab.document, tab.dirty, tab.active, tab.asks_to_close))
        .collect();
    assert_eq!(
        flags,
        [
            (0, false, false, false),
            (1, true, false, true),
            // A new document is not on disk, so it counts as changed; with
            // nothing in it, closing it loses nothing.
            (2, true, false, false),
            (3, true, true, true),
        ]
    );
    // Every tab and its close button is drawn, left to right, under the
    // menu bar and above the viewport, the close button inside its tab.
    let viewport = shell.viewport_rect().unwrap();
    let mut right = f32::MIN;
    for document in 0..4 {
        let tab = hit(&shell, TabPart::Tab(document));
        let close = hit(&shell, TabPart::Close(document));
        assert!(tab.left() > right, "{document}: {tab:?}");
        right = tab.right();
        assert!(tab.contains_rect(close), "{tab:?} {close:?}");
        assert!(tab.bottom() <= viewport.top() + 0.5, "{tab:?} {viewport:?}");
        assert!(tab.top() > 10.0, "below the menu bar: {tab:?}");
    }
    // ui.get reads the same.
    let got = shell
        .handle_method(&mut session, "ui.get", Some(&json!({})))
        .unwrap()
        .unwrap();
    assert_eq!(
        got["tabs"][1],
        json!({"document": 1, "name": "bracket", "path": "parts/bracket.3dcraft",
               "dirty": true, "active": false, "asksToClose": true,
               "jigInUse": false, "assembly": false}),
        "{got}"
    );
    assert_eq!(got["tabs"].as_array().unwrap().len(), 4);
    assert_eq!(got["tabs"][3]["active"], true);
    assert_eq!(got["closing"], Value::Null);
    // Saving clears the mark; the untitled ones are numbered in the row.
    run(&mut session, "file.save", json!({"path": "block.3dcraft"}));
    window.idle(&mut shell, &mut session, 1);
    assert_eq!(names(&shell), ["gear", "bracket", "Untitled 1", "block"]);
    assert!(!shell.tabs()[3].dirty);
}

#[test]
fn a_click_on_a_tab_asks_for_file_activate_and_the_highlight_follows() {
    let mut session = four("activate");
    let mut shell = Shell::new();
    let mut window = Window::new(1200.0, 700.0);
    window.idle(&mut shell, &mut session, 2);
    let revision = session.scene_revision();
    let at = hit(&shell, TabPart::Tab(0)).center();
    window.click(&mut shell, &mut session, at);
    // The shell asked; it did not switch.
    assert_eq!(shell.take_doc_action(), Some(DocAction::Activate(0)));
    assert_eq!(session.document_status().unwrap().index, 3);
    assert_eq!(session.scene_revision(), revision);
    // The host runs file.activate: the first tab is the active one now.
    shell.request_doc_action(DocAction::Activate(0));
    host(&mut shell, &mut session);
    window.idle(&mut shell, &mut session, 1);
    let active: Vec<bool> = shell.tabs().iter().map(|tab| tab.active).collect();
    assert_eq!(active, [true, false, false, false]);
    assert_eq!(shell.tree_rows().len(), 1, "the tree is gear's");
    // A click on the active tab asks for nothing.
    let at = hit(&shell, TabPart::Tab(0)).center();
    window.click(&mut shell, &mut session, at);
    assert_eq!(shell.take_doc_action(), None);
    // Ctrl+Tab goes to the next, Ctrl+Shift+Tab to the one before, round.
    window.key(&mut shell, &mut session, egui::Key::Tab, false);
    assert_eq!(shell.take_doc_action(), Some(DocAction::Activate(1)));
    window.key(&mut shell, &mut session, egui::Key::Tab, true);
    assert_eq!(shell.take_doc_action(), Some(DocAction::Activate(3)));
}

#[test]
fn closing_asks_first_only_when_there_is_something_to_lose() {
    let mut session = four("close");
    let mut shell = Shell::new();
    let mut window = Window::new(1200.0, 700.0);
    window.idle(&mut shell, &mut session, 2);

    // Saved and unchanged: closed at once, without a question.
    let at = hit(&shell, TabPart::Close(0)).center();
    window.click(&mut shell, &mut session, at);
    assert_eq!(shell.closing(), None);
    assert_eq!(
        host(&mut shell, &mut session),
        Some(DocAction::Close {
            document: 0,
            discard: false
        })
    );
    window.idle(&mut shell, &mut session, 1);
    assert_eq!(names(&shell), ["bracket", "Untitled 1", "Untitled 2"]);

    // Changed since it was saved: the question, and nothing asked yet.
    let at = hit(&shell, TabPart::Close(0)).center();
    window.click(&mut shell, &mut session, at);
    assert_eq!(shell.take_doc_action(), None);
    let closing = shell.closing().unwrap().clone();
    assert_eq!((closing.document, closing.name.as_str()), (0, "bracket"));
    window.idle(&mut shell, &mut session, 1);
    let got = shell
        .handle_method(&mut session, "ui.get", Some(&json!({})))
        .unwrap()
        .unwrap();
    assert_eq!(got["closing"]["name"], "bracket", "{got}");
    let answers: Vec<CloseAnswer> = shell.closing_hits().iter().map(|(a, _)| *a).collect();
    assert_eq!(answers, CloseAnswer::ALL);
    // While it is asked, the tabs behind it take no clicks: a click beside
    // the question cancels it.
    let behind = hit(&shell, TabPart::Tab(2)).center();
    window.click(&mut shell, &mut session, behind);
    assert_eq!(shell.take_doc_action(), None);
    assert_eq!(shell.closing(), None);
    // Ask again, and Cancel.
    let at = hit(&shell, TabPart::Close(0)).center();
    window.click(&mut shell, &mut session, at);
    assert!(shell.closing().is_some());
    window.idle(&mut shell, &mut session, 1);
    let button = |shell: &Shell, answer: CloseAnswer| {
        shell
            .closing_hits()
            .iter()
            .find(|(a, _)| *a == answer)
            .unwrap()
            .1
            .center()
    };
    let cancel = button(&shell, CloseAnswer::Cancel);
    window.click(&mut shell, &mut session, cancel);
    assert_eq!(shell.closing(), None);
    assert_eq!(shell.take_doc_action(), None);
    assert_eq!(names(&shell).len(), 3);

    // Ask again and don't save: closed with discard.
    window.click(&mut shell, &mut session, at);
    window.idle(&mut shell, &mut session, 1);
    let discard = button(&shell, CloseAnswer::Discard);
    window.click(&mut shell, &mut session, discard);
    assert_eq!(shell.closing(), None);
    assert_eq!(
        host(&mut shell, &mut session),
        Some(DocAction::Close {
            document: 0,
            discard: true
        })
    );
    window.idle(&mut shell, &mut session, 1);
    assert_eq!(names(&shell), ["Untitled 1", "Untitled 2"]);

    // Save: the host saves and closes. (Without a file, the window's host
    // asks for a name first: apps/craft3d.)
    run(&mut session, "file.activate", json!({"document": 1}));
    run(&mut session, "file.save", json!({"path": "kept.3dcraft"}));
    run(&mut session, "feature.sphere", json!({"radius": 1}));
    window.idle(&mut shell, &mut session, 1);
    assert_eq!(names(&shell), ["Untitled 1", "kept"]);
    shell.request_close(1);
    window.idle(&mut shell, &mut session, 1);
    let save = button(&shell, CloseAnswer::Save);
    window.click(&mut shell, &mut session, save);
    assert_eq!(
        host(&mut shell, &mut session),
        Some(DocAction::SaveAndClose(1))
    );
    window.idle(&mut shell, &mut session, 1);
    assert_eq!(names(&shell), ["Untitled 1"]);

    // An empty new document closes without a question, and Ctrl+W closes
    // the active one the same way.
    window.key(&mut shell, &mut session, egui::Key::W, false);
    assert_eq!(
        host(&mut shell, &mut session),
        Some(DocAction::Close {
            document: 0,
            discard: false
        })
    );
    window.idle(&mut shell, &mut session, 1);
    assert!(shell.tabs().is_empty());
    assert!(shell.tab_hits().is_empty(), "no row without documents");
}

/// The question names the document in its body, whole, which wraps, and
/// never in its title, which the dialog cuts at its edge (2026-11-10: the
/// title "Save the changes to "T-slot aluminium profile" before …" was
/// cut); the same for a Jig in use.
#[test]
fn the_question_names_the_document_in_its_body_however_long() {
    use craft3d_ui_egui::shell::closing_words;
    use craft3d_ui_egui::state::Closing;
    let name = "T-slot aluminium profile, 40 x 40 mm, with a name as long as a person types it";
    let closing = |quitting: bool, jig: bool| Closing {
        document: 0,
        name: name.to_string(),
        path: None,
        quitting,
        jig,
    };
    let words = closing_words(&closing(false, false));
    assert_eq!(words.title, "Save the changes before closing?");
    assert_eq!(words.about, format!("\"{name}\" has unsaved changes."));
    assert_eq!(
        words.lost,
        "The changes are lost if you close without saving."
    );
    let words = closing_words(&closing(true, false));
    assert_eq!(words.title, "Save the changes before quitting?");
    assert_eq!(
        words.lost,
        "The changes are lost if you quit without saving."
    );
    let words = closing_words(&closing(false, true));
    assert_eq!(words.title, "Save the parameters as a model?");
    assert_eq!(
        words.about,
        format!("You changed the parameters of \"{name}\".")
    );
    assert_eq!(
        words.lost,
        "The Jig itself is not changed. The values you set are lost if you close without saving them."
    );
}

/// An agent that closes or saves the document while the question is asked
/// takes the question away: it was about a document that is gone.
#[test]
fn the_question_goes_when_its_document_does() {
    let mut session = four("gone");
    let mut shell = Shell::new();
    let mut window = Window::new(1200.0, 700.0);
    window.idle(&mut shell, &mut session, 2);
    shell.request_close(3);
    assert!(shell.closing().is_some());
    window.idle(&mut shell, &mut session, 1);
    run(&mut session, "file.close", json!({"discard": true}));
    window.idle(&mut shell, &mut session, 1);
    assert_eq!(shell.closing(), None);
    assert!(shell.closing_hits().is_empty());
    shell.request_close(1);
    assert!(shell.closing().is_some());
    run(&mut session, "file.activate", json!({"document": 1}));
    run(&mut session, "file.save", json!({}));
    window.idle(&mut shell, &mut session, 1);
    assert_eq!(shell.closing(), None);
    // A tab that does not exist asks for nothing.
    shell.request_close(42);
    shell.request_activate(42);
    assert_eq!(shell.take_doc_action(), None);
    assert_eq!(shell.closing(), None);
}

/// Thirty documents in a narrow window: the tabs shrink to their narrowest
/// and the row scrolls, with the active one in view, and a middle click
/// closes a tab.
#[test]
fn many_documents_shrink_the_tabs_and_scroll_the_active_one_into_view() {
    let mut session = Session::new(Workspace::none());
    for _ in 0..30 {
        run(&mut session, "file.new", json!({}));
    }
    let mut shell = Shell::new();
    let width = 800.0;
    let mut window = Window::new(width, 600.0);
    window.idle(&mut shell, &mut session, 4);
    assert_eq!(shell.tabs().len(), 30);
    let tokens = shell.tokens;
    let widths: Vec<f32> = (0..30)
        .map(|document| hit(&shell, TabPart::Tab(document)).width())
        .collect();
    assert!(
        widths
            .iter()
            .all(|w| (w - tokens.tab_min_width).abs() < 0.5),
        "{widths:?}"
    );
    // The active one, the last, is in the window; the first is scrolled
    // out of it.
    let last = hit(&shell, TabPart::Tab(29));
    assert!(last.left() >= 0.0 && last.right() <= width, "{last:?}");
    assert!(hit(&shell, TabPart::Tab(0)).right() < 0.0);
    // Activating the first scrolls it back into view.
    shell.request_doc_action(DocAction::Activate(0));
    host(&mut shell, &mut session);
    window.idle(&mut shell, &mut session, 4);
    let first = hit(&shell, TabPart::Tab(0));
    assert!(first.left() >= 0.0 && first.right() <= width, "{first:?}");
    assert_eq!(shell.tabs()[0].name, "Untitled 1");
    assert_eq!(shell.tabs()[29].name, "Untitled 30");
    // A middle click on a tab in view closes it.
    let at = hit(&shell, TabPart::Tab(2)).center();
    window.click_with(&mut shell, &mut session, at, PointerButton::Middle);
    assert_eq!(
        shell.take_doc_action(),
        Some(DocAction::Close {
            document: 2,
            discard: false
        })
    );

    // Two documents in a wide window: as wide as the theme allows.
    let mut two = Session::new(Workspace::none());
    run(&mut two, "file.new", json!({}));
    run(&mut two, "file.new", json!({}));
    let mut shell = Shell::new();
    let mut window = Window::new(1600.0, 600.0);
    window.idle(&mut shell, &mut two, 2);
    let w = hit(&shell, TabPart::Tab(0)).width();
    assert!((w - shell.tokens.tab_max_width).abs() < 0.5, "{w}");
}

#[test]
fn the_file_menu_lists_the_documents_and_a_click_activates_one() {
    let mut session = four("menu");
    let mut shell = Shell::new();
    let mut window = Window::new(1200.0, 800.0);
    window.idle(&mut shell, &mut session, 2);
    // Down the open File menu, row by row: the documents come after the
    // file actions and Close, in their order.
    let mut found = Vec::new();
    for row in 0..60 {
        window.click(&mut shell, &mut session, pos2(20.0, 10.0));
        let at = pos2(40.0, 30.0 + 8.0 * row as f32);
        window.click(&mut shell, &mut session, at);
        window.idle(&mut shell, &mut session, 1);
        if let Some(action) = shell.take_doc_action() {
            found.push(action);
        }
        if shell.closing().is_some() {
            // That was Close, on the active document with a block in it.
            assert_eq!(shell.closing().unwrap().document, 3);
            found.push(DocAction::Close {
                document: 3,
                discard: false,
            });
            shell.answer_closing(CloseAnswer::Cancel);
        }
        let _ = shell.take_file_action();
        // Close the menu if it is still open.
        window.frame(
            &mut shell,
            &mut session,
            vec![Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
    }
    found.dedup();
    // The active one (3) is checked and asks for nothing.
    assert_eq!(
        found,
        [
            DocAction::Close {
                document: 3,
                discard: false
            },
            DocAction::Activate(0),
            DocAction::Activate(1),
            DocAction::Activate(2),
        ]
    );
}

/// The marks the tabs draw are in egui's default fonts (see the tree's
/// marks in tests/shell.rs for how that is told).
#[test]
fn the_tab_marks_are_in_the_fonts() {
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
    output.textures_delta.clear();
    let body = egui::FontId::proportional(14.0);
    let glyph = |c: char| {
        ctx.fonts_mut(|f| {
            let galley = f.layout_no_wrap(c.to_string(), body.clone(), egui::Color32::WHITE);
            galley.rows[0].glyphs[0].uv_rect
        })
    };
    let missing = glyph('\u{10FFFD}');
    assert_eq!(glyph('●'), missing);
    for mark in [
        craft3d_ui_egui::shell::UNSAVED_MARK,
        craft3d_ui_egui::shell::CLOSE_MARK,
    ] {
        assert_ne!(glyph(mark), missing, "{mark}");
    }
}

#[test]
fn a_tab_is_named_after_its_file_without_folders_or_extension() {
    use craft3d_ui_egui::shell::tab_name;
    for (path, name) in [
        (Some("/home/me/parts/gear.3dcraft"), Some("gear")),
        (Some(r"C:\parts\gear.3dcraft"), Some("gear")),
        (Some("gear.v2.3dcraft"), Some("gear.v2")),
        (Some("gear"), Some("gear")),
        (Some(".hidden"), Some(".hidden")),
        (Some("dir/"), Some("dir")),
        (Some("a\nb.3dcraft"), Some("ab")),
        (Some(""), None),
        (Some("/"), None),
        (None, None),
    ] {
        assert_eq!(tab_name(path).as_deref(), name, "{path:?}");
    }
    let long = tab_name(Some(&format!("{}.3dcraft", "x".repeat(5000)))).unwrap();
    assert_eq!(long.chars().count(), 120);
}

/// A document without a file is named by its title when it has one (an
/// example from the start screen), and only the others count as
/// `Untitled n`; a file's name goes first.
#[test]
fn a_tab_without_a_file_is_named_by_its_title_else_untitled() {
    use craft3d_engine::DocumentStatus;
    let status = |index: usize, path: Option<&str>, title: Option<&str>| DocumentStatus {
        index,
        id: index as u64 + 1,
        open: 5,
        path: path.map(str::to_string),
        file: None,
        title: title.map(str::to_string),
        assembly: false,
        dirty: false,
        close_asks: false,
        length_unit: "mm",
        angle_unit: "deg",
        jig_in_use: false,
        undo: None,
        redo: None,
    };
    let statuses = [
        status(0, Some("/a/gear.3dcraft"), Some("Gear")),
        status(1, None, Some("Moka pot")),
        status(2, None, None),
        status(3, None, Some(" \n ")),
        status(4, None, Some("Two\nlines")),
        status(5, None, Some("a\u{7}b\r\n\tc")),
    ];
    let tabs = craft3d_ui_egui::doc_tabs(&statuses, Some(1));
    let names: Vec<&str> = tabs.iter().map(|t| t.name.as_str()).collect();
    // A line break or a tab is a space, one for a run of them; another
    // control character is dropped.
    assert_eq!(
        names,
        [
            "gear",
            "Moka pot",
            "Untitled 1",
            "Untitled 2",
            "Two lines",
            "ab c"
        ]
    );
    assert!(tabs[1].active && !tabs[1].marked());
}
