//! The UI overhaul's phase 2b, part A (plan of 2026-11-02, boards s3, s4,
//! s6, f1, f2, w3, w4): the status bar with the agents pill at its right
//! end, the start screen, the menus with the new View menu and the
//! appearance switch, the documents' tabs and the dialogs' one layout.
//! Everything stays inside the window at 1400 × 900, 1024 × 700, 480 and
//! 390 points wide.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::shell::FileAction;
use craft3d_ui_egui::{AgentClient, AgentRoot, AgentsStatus, Shell};
use egui::{Event, Pos2, Rect, vec2};
use serde_json::json;

const SIZES: [(f32, f32); 4] = [
    (1400.0, 900.0),
    (1024.0, 700.0),
    (480.0, 860.0),
    (390.0, 844.0),
];

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
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        self.frame(shell, session, vec![button(true)]);
        self.frame(shell, session, vec![button(false)]);
        self.frame(shell, session, Vec::new());
    }

    fn inside(&self, rect: Rect) -> bool {
        rect.left() >= -0.5
            && rect.top() >= -0.5
            && rect.right() <= self.size.x + 0.5
            && rect.bottom() <= self.size.y + 0.5
    }
}

fn connected() -> AgentsStatus {
    AgentsStatus {
        available: true,
        allowed: true,
        port: Some(7990),
        connected: 1,
        clients: vec![AgentClient {
            name: "Claude Code".to_string(),
            since_ms: 1,
            last_command: Some("feature.fillet".to_string()),
            last_ms: 1_000,
            cwd: None,
        }],
        work_folder: Some("/home/me/Documents/3DCraft".to_string()),
        document_folder: true,
        roots: vec![AgentRoot {
            path: "/home/me/Documents/3DCraft".to_string(),
            from: "chosen".to_string(),
            read: true,
            write: true,
        }],
        home: Some("/home/me".to_string()),
        ..AgentsStatus::default()
    }
}

fn call(
    shell: &mut Shell,
    session: &mut Session,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    shell
        .handle_method(session, method, Some(&params))
        .unwrap()
        .unwrap()
}

/// The pill is the status bar's last item, at its right end, after every
/// other item, and inside the window at every size; working, it says
/// what the agent does.
#[test]
fn the_pill_ends_the_status_bar_at_every_size() {
    for (width, height) in SIZES {
        let mut session = Session::new(Workspace::none());
        session.execute("file.new", &json!({})).unwrap();
        session
            .execute("feature.block", &json!({"size": [10, 10, 10]}))
            .unwrap();
        let mut shell = Shell::new();
        shell.set_agents_status(connected());
        shell.set_agents_clock(Some(1_200));
        let mut window = Window::sized(width, height);
        window.frames(&mut shell, &mut session, 4);
        let pill = shell.agents_pill_rect().expect("the pill is drawn");
        assert!(window.inside(pill), "{width}: {pill:?}");
        assert!(
            pill.right() > width - 24.0,
            "{width}: at the right end: {pill:?}"
        );
        for (slot, rect) in shell.status_hits() {
            assert!(
                rect.right() <= pill.left() + 0.5,
                "{width}: {slot:?} {rect:?} runs into the pill {pill:?}"
            );
        }
        assert_eq!(
            shell.agents_pill_view().label,
            "Claude Code · feature.fillet"
        );
    }
}

/// The start screen (board f1): New part, New assembly and Open…, the
/// sections, the cards, Recent and the Connect block, all inside the
/// window at every size; New assembly asks the host for an assembly.
#[test]
fn the_start_screen_fits_and_new_assembly_asks_for_one() {
    for (width, height) in SIZES {
        let mut session = Session::new(Workspace::none());
        let mut shell = Shell::new();
        shell.set_agents_status(connected());
        let mut window = Window::sized(width, height);
        window.frames(&mut shell, &mut session, 4);
        assert!(shell.start_shown());
        let viewport = shell.viewport_rect().unwrap();
        let hits = shell.start_hits().to_vec();
        for name in [
            "new",
            "new-assembly",
            "open",
            "section:parts",
            "section:jigs",
        ] {
            let rect = hits
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, r)| *r)
                .unwrap_or_else(|| panic!("{width}: no {name}"));
            assert!(
                rect.left() >= viewport.left() - 0.5 && rect.right() <= viewport.right() + 0.5,
                "{width}: {name} {rect:?} beside {viewport:?}"
            );
        }
        // Nothing runs past the sides (the screen scrolls down when it
        // is taller than the window, never sideways).
        for (name, rect) in &hits {
            assert!(
                rect.left() >= viewport.left() - 0.5 && rect.right() <= viewport.right() + 0.5,
                "{width}: {name} {rect:?} beside {viewport:?}"
            );
        }
        if width >= 1024.0 {
            // Two rows of five cards, within the window's height.
            let cards: Vec<Rect> = hits
                .iter()
                .filter(|(n, _)| n.starts_with("example:"))
                .map(|(_, r)| *r)
                .collect();
            assert_eq!(cards.len(), 10, "{width}");
            for card in &cards {
                assert!(viewport.contains_rect(*card), "{width}: {card:?}");
            }
        }
        let new = hits.iter().find(|(n, _)| n == "new-assembly").unwrap().1;
        window.click(&mut shell, &mut session, new.center());
        assert_eq!(
            shell.take_file_action(),
            Some(FileAction::NewAssembly),
            "{width}"
        );
    }
}

/// View › Appearance and the side panel and status bar toggles are the
/// state's fields: the menu sets them, ui.get reads them, ui.set sets
/// them; the panels' colours follow at the next frame.
#[test]
fn the_view_menu_and_the_appearance_switch() {
    let mut session = Session::new(Workspace::none());
    session.execute("file.new", &json!({})).unwrap();
    let mut shell = Shell::new();
    let mut window = Window::sized(1400.0, 900.0);
    window.frames(&mut shell, &mut session, 3);
    let titles: Vec<String> = shell.menu_titles().iter().map(|(t, _)| t.clone()).collect();
    assert_eq!(titles, ["File", "Edit", "View", "Help"]);
    let open_view = |window: &mut Window, shell: &mut Shell, session: &mut Session| {
        let view = shell
            .menu_titles()
            .iter()
            .find(|(t, _)| t == "View")
            .unwrap()
            .1;
        window.click(shell, session, view.center());
        shell.menu_items().to_vec()
    };
    let items = open_view(&mut window, &mut shell, &mut session);
    let labels: Vec<&str> = items.iter().map(|(l, _)| l.as_str()).collect();
    assert_eq!(
        labels,
        [
            "Side panel",
            "Status bar",
            "System",
            "Light",
            "Dark",
            "Full screen"
        ]
    );
    for (label, rect) in &items {
        assert!(window.inside(*rect), "{label}: {rect:?}");
    }
    // Dark, from the menu.
    let dark = items.iter().find(|(l, _)| l == "Dark").unwrap().1;
    window.click(&mut shell, &mut session, dark.center());
    window.frames(&mut shell, &mut session, 2);
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["state"]["appearance"], "dark");
    assert_eq!(got["appearance"], "dark");
    assert!(shell.dark());
    // Light, through ui.set.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"appearance": "light"}),
    );
    window.frames(&mut shell, &mut session, 2);
    assert!(!shell.dark());
    assert_eq!(
        call(&mut shell, &mut session, "ui.get", json!({}))["appearance"],
        "light"
    );
    // The status bar, put away from the menu.
    let items = open_view(&mut window, &mut shell, &mut session);
    let status = items.iter().find(|(l, _)| l == "Status bar").unwrap().1;
    window.click(&mut shell, &mut session, status.center());
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["state"]["panels"]["status"], false);
    window.frames(&mut shell, &mut session, 2);
    assert!(shell.agents_pill_rect().is_none(), "no status bar, no pill");
    // Ctrl+B puts the side panel away and brings it back.
    let ctrl_b = |pressed| Event::Key {
        key: egui::Key::B,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    };
    assert!(shell.state.panels.tree);
    window.frame(&mut shell, &mut session, vec![ctrl_b(true), ctrl_b(false)]);
    assert!(!shell.state.panels.tree);
    window.frame(&mut shell, &mut session, vec![ctrl_b(true), ctrl_b(false)]);
    assert!(shell.state.panels.tree);
}

/// File › New Part and New Assembly carry the documents' line icons and
/// their keys; Open recent is a submenu.
#[test]
fn the_file_menu_offers_both_new_documents() {
    let mut session = Session::new(Workspace::none());
    let mut shell = Shell::new();
    let mut window = Window::sized(1024.0, 700.0);
    window.frames(&mut shell, &mut session, 3);
    let file = shell.menu_titles()[0].1;
    window.click(&mut shell, &mut session, file.center());
    let labels: Vec<String> = shell.menu_items().iter().map(|(l, _)| l.clone()).collect();
    assert_eq!(&labels[..3], ["New Part", "New Assembly", "Open…"]);
    let assembly = shell.menu_items()[1].1;
    window.click(&mut shell, &mut session, assembly.center());
    assert_eq!(shell.take_file_action(), Some(FileAction::NewAssembly));
    assert_eq!(
        FileAction::NewAssembly
            .shortcut()
            .map(|s| s.format(&egui::ModifierNames::NAMES, false)),
        Some("Ctrl+Shift+N".to_string())
    );
}

/// Every dialog has one layout (board s6): the amber button is the last
/// at the right of its footer, and the dialog is inside the window.
#[test]
fn the_dialogs_put_the_amber_button_last() {
    let mut session = Session::new(Workspace::unrestricted());
    session.execute("file.new", &json!({})).unwrap();
    session
        .execute("feature.block", &json!({"size": [10, 10, 10]}))
        .unwrap();
    let mut shell = Shell::new();
    for (width, height) in [(1024.0, 700.0), (390.0, 844.0)] {
        let mut window = Window::sized(width, height);
        window.frames(&mut shell, &mut session, 3);
        shell.sync(&mut session);
        // The question before closing: Save (amber) right of Cancel, right
        // of Don't Save.
        let document = shell.tabs()[0].document;
        shell.request_close(document);
        window.frames(&mut shell, &mut session, 3);
        let hits = shell.closing_hits().to_vec();
        assert_eq!(hits.len(), 3, "{width}");
        let save = hits[0].1;
        let discard = hits[1].1;
        let cancel = hits[2].1;
        assert!(
            discard.right() < cancel.left() && cancel.right() < save.left() + 0.5,
            "{width}: {hits:?}"
        );
        for (_, rect) in &hits {
            assert!(window.inside(*rect), "{width}: {rect:?}");
        }
        shell.answer_closing(craft3d_ui_egui::CloseAnswer::Cancel);
        window.frames(&mut shell, &mut session, 2);
    }
}

/// A tab says its document's kind with a line icon (the plan's middle
/// way): a Jig in use, an assembly, a part.
#[test]
fn a_tab_says_its_kind() {
    use craft3d_ui_egui::kit::Glyph;
    use craft3d_ui_egui::shell::tab_glyph;
    let tab = |path: Option<&str>, jig: bool, assembly: bool| craft3d_ui_egui::DocTab {
        path: path.map(str::to_string),
        jig_in_use: jig,
        assembly,
        ..craft3d_ui_egui::DocTab::default()
    };
    assert_eq!(
        tab_glyph(&tab(Some("a/b.3dcraftjig"), true, false)),
        Glyph::Jig
    );
    assert_eq!(
        tab_glyph(&tab(Some("a/b.3dcraftasm"), false, true)),
        Glyph::Assembly
    );
    assert_eq!(tab_glyph(&tab(None, false, true)), Glyph::Assembly);
    assert_eq!(
        tab_glyph(&tab(Some("a/b.3dcraft"), false, false)),
        Glyph::Part
    );
    assert_eq!(tab_glyph(&tab(None, false, false)), Glyph::Part);
    // An assembly made in the window is known as one in its tab.
    let mut session = Session::new(Workspace::none());
    session.execute("assembly.new", &json!({})).unwrap();
    let mut shell = Shell::new();
    let mut window = Window::sized(1024.0, 700.0);
    window.frames(&mut shell, &mut session, 3);
    assert!(shell.tabs()[0].assembly, "{:?}", shell.tabs());
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["tabs"][0]["assembly"], true);
}

/// A part with three steps: its history plays.
fn three_steps() -> Session {
    let mut session = Session::new(Workspace::none());
    session.execute("file.new", &json!({})).unwrap();
    session
        .execute(
            "feature.block",
            &json!({"size": [10, 10, 10], "name": "Box"}),
        )
        .unwrap();
    session
        .execute("feature.sphere", &json!({"radius": 3, "name": "Ball"}))
        .unwrap();
    session
        .execute(
            "feature.cylinder",
            &json!({"radius": 1, "height": 30, "name": "Rod"}),
        )
        .unwrap();
    session
}

/// The planetary gearbox assembly: its motion plays.
fn gearbox() -> Session {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/examples/published/planetary-gearbox.3dcraftasm");
    let mut session = Session::new(Workspace::unrestricted());
    session
        .execute("file.open", &json!({"path": path.to_str().unwrap()}))
        .unwrap();
    session
}

/// The player is one row (board s8) that fits where it shows: in the side
/// panel at 1400 × 900 and 1024 × 700, and at 480 and 390 points both in
/// the drawer and on the strip over the viewport, for a history and for
/// an assembly's motion. Its parts are inside the window and the panel,
/// and none overlaps another.
#[test]
fn the_player_fits_at_every_size() {
    for (name, make) in [
        ("history", three_steps as fn() -> Session),
        ("motion", gearbox as fn() -> Session),
    ] {
        for (width, height) in SIZES {
            for drawer in [false, true] {
                let narrow = width < 800.0;
                if drawer && !narrow {
                    continue;
                }
                let mut session = make();
                let mut shell = Shell::new();
                let mut window = Window::sized(width, height);
                window.frames(&mut shell, &mut session, 3);
                if drawer {
                    call(
                        &mut shell,
                        &mut session,
                        "ui.set",
                        json!({"panels": {"drawer": true}}),
                    );
                }
                // The drawer slides in over a tenth of a second.
                window.frames(&mut shell, &mut session, 10);
                let hits = shell.player_hits().to_vec();
                let what = format!("{name} {width}×{height} drawer {drawer}");
                assert!(!hits.is_empty(), "{what}: no player");
                let parts: Vec<&str> = hits.iter().map(|(p, _)| *p).collect();
                if name == "history" || !narrow || drawer {
                    for part in ["back", "play", "forward", "position"] {
                        assert!(parts.contains(&part), "{what}: no {part} in {parts:?}");
                    }
                } else {
                    // The motion's strip over the viewport: play, slider, t.
                    for part in ["play", "slider", "position"] {
                        assert!(parts.contains(&part), "{what}: no {part} in {parts:?}");
                    }
                }
                if !narrow {
                    for part in ["speed", "loop", "orbit"] {
                        assert!(parts.contains(&part), "{what}: no {part} in {parts:?}");
                    }
                }
                // No ⏮ or ⏭ (decision 5).
                assert!(!parts.contains(&"first") && !parts.contains(&"last"));
                let panel = shell.tree_panel_rect();
                for (part, rect) in &hits {
                    assert!(window.inside(*rect), "{what}: {part} {rect:?} outside");
                    if let (Some(panel), true) = (panel, !narrow || drawer) {
                        assert!(
                            panel.expand(0.5).contains_rect(*rect),
                            "{what}: {part} {rect:?} outside the panel {panel:?}"
                        );
                    }
                }
                for (i, (a, ra)) in hits.iter().enumerate() {
                    for (b, rb) in hits.iter().skip(i + 1) {
                        assert!(
                            !ra.shrink(0.5).intersects(rb.shrink(0.5)),
                            "{what}: {a} {ra:?} over {b} {rb:?}"
                        );
                    }
                }
            }
        }
    }
}

/// The Agent tab reads the document's log (what an agent did, newest
/// first, with its intent) and the feature list marks what an agent made
/// with ✦ (the document's provenance, `madeBy`); the person's own
/// features carry no mark.
#[test]
fn the_agent_tab_and_the_agents_mark() {
    let mut session = Session::new(Workspace::none());
    session.execute("file.new", &json!({})).unwrap();
    session
        .execute(
            "feature.block",
            &json!({"size": [10, 10, 10], "name": "Mine"}),
        )
        .unwrap();
    session.set_actor(Some("Claude Code"));
    session.set_intent(Some("Round the base for handling"));
    session
        .execute("feature.sphere", &json!({"radius": 3, "name": "Theirs"}))
        .unwrap();
    session.set_intent(None);
    session.set_actor(None);
    let mut shell = Shell::new();
    shell.set_agents_status(connected());
    let mut window = Window::sized(1400.0, 900.0);
    window.frames(&mut shell, &mut session, 3);
    let made: Vec<(String, Option<String>)> = shell
        .tree_rows()
        .iter()
        .map(|r| (r.name.clone(), r.made_by.clone()))
        .collect();
    assert_eq!(
        made,
        [
            ("Mine".to_string(), None),
            ("Theirs".to_string(), Some("Claude Code".to_string()))
        ]
    );
    // The tab is offered (in the bar or behind ⋯), and shows the entry.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"panels": {"sideTab": "agent"}}),
    );
    window.frames(&mut shell, &mut session, 3);
    let rows = shell.side_hits().agent_rows.clone();
    assert_eq!(rows.len(), 1, "one command by the agent");
    let panel = shell.side_hits().panel.unwrap();
    for row in &rows {
        assert!(
            panel.expand(0.5).contains_rect(*row),
            "{row:?} in {panel:?}"
        );
    }
    // Agents off and no log: no tab.
    let mut plain = Session::new(Workspace::none());
    plain.execute("file.new", &json!({})).unwrap();
    let mut shell = Shell::new();
    let mut window = Window::sized(1400.0, 900.0);
    window.frames(&mut shell, &mut plain, 3);
    call(
        &mut shell,
        &mut plain,
        "ui.set",
        json!({"panels": {"sideTab": "agent"}}),
    );
    window.frames(&mut shell, &mut plain, 3);
    assert!(shell.side_hits().agent_rows.is_empty());
    assert_ne!(shell.state.panels.side_tab, craft3d_ui_egui::SideTab::Agent);
}

/// An assembly's mates and checks keep their statuses in view: a name is
/// cut to leave its status room, inside the panel, at 1400, 1024, 480 and
/// 390 points (the drawer on the narrow ones).
#[test]
fn an_assemblys_mates_and_checks_stay_inside() {
    for (width, height) in SIZES {
        let mut session = gearbox();
        // A check whose name is far wider than the panel.
        session
            .execute(
                "assert.add",
                &json!({"name": format!("A check with a name far wider than the panel {}", "x".repeat(60)),
                        "measure": "volume", "min": 0}),
            )
            .unwrap();
        let mut shell = Shell::new();
        let mut window = Window::sized(width, height);
        window.frames(&mut shell, &mut session, 3);
        if width < 800.0 {
            call(
                &mut shell,
                &mut session,
                "ui.set",
                json!({"panels": {"drawer": true}}),
            );
        }
        window.frames(&mut shell, &mut session, 10);
        assert!(!shell.assertion_rows().is_empty(), "the gearbox has checks");
        let panel = shell.side_hits().panel.expect("the side panel");
        let statuses = shell.side_hits().statuses.clone();
        assert!(
            statuses.len() > shell.assertion_rows().len(),
            "{width}: mates and checks: {statuses:?}"
        );
        // Side to side inside the panel (the list scrolls up and down).
        for status in &statuses {
            assert!(
                status.left() >= panel.left() - 0.5 && status.right() <= panel.right() + 0.5,
                "{width}: {status:?} past the panel's sides {panel:?}"
            );
            assert!(
                status.width() >= 8.0,
                "{width}: a status with no room: {status:?}"
            );
        }
        assert!(window.inside(panel), "{width}: {panel:?}");
    }
}

/// A Jig in use on a phone: its parameters are a half sheet under the
/// model (board m8); on a 768-point tablet the side panel docks beside it
/// (t2) and there is no drawer.
#[test]
fn a_jig_on_a_phone_and_a_tablet() {
    let open = || {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/examples/published/planetary-gearbox.3dcraftjig");
        let mut session = Session::new(Workspace::unrestricted());
        session
            .execute("file.open", &json!({"path": path.to_str().unwrap()}))
            .unwrap();
        session
    };
    let mut session = open();
    let mut shell = Shell::new();
    let mut window = Window::sized(390.0, 844.0);
    window.frames(&mut shell, &mut session, 4);
    let sheet = shell.parameters_sheet_rect().expect("the half sheet");
    let viewport = shell.viewport_rect().unwrap();
    assert!(
        viewport.expand(0.5).contains_rect(sheet),
        "{sheet:?} in {viewport:?}"
    );
    assert!(
        (sheet.bottom() - viewport.bottom()).abs() < 0.5,
        "{sheet:?}"
    );
    assert!(
        sheet.height() < viewport.height() * 0.75,
        "the model stays in view above: {sheet:?}"
    );
    // Its form is the Parameters tab's: Reset all is there.
    let reset = shell
        .parameter_hits()
        .iter()
        .find(|(n, _)| n == "Reset all")
        .map(|(_, r)| *r)
        .expect("Reset all");
    assert!(sheet.contains_rect(reset), "{reset:?} in {sheet:?}");
    // A tablet: docked, no drawer and no sheet.
    let mut session = open();
    let mut shell = Shell::new();
    let mut window = Window::sized(768.0, 1024.0);
    window.frames(&mut shell, &mut session, 4);
    assert!(shell.narrow());
    assert!(shell.parameters_sheet_rect().is_none());
    assert!(
        shell.drawer_button_rect().is_none(),
        "no drawer beside a docked panel"
    );
    let panel = shell.tree_panel_rect().expect("the docked panel");
    assert!(panel.left() < 1.0 && panel.width() >= 200.0, "{panel:?}");
    let viewport = shell.viewport_rect().unwrap();
    assert!(
        viewport.left() >= panel.right() - 0.5,
        "{viewport:?} beside {panel:?}"
    );
}

/// On a phone the questions about access are a bottom sheet with the
/// answers one under the other, the amber one first (board m7).
#[test]
fn a_question_on_a_phone_is_a_sheet() {
    use craft3d_ui_egui::AgentPrompt;
    let mut session = Session::new(Workspace::none());
    session.execute("file.new", &json!({})).unwrap();
    let mut shell = Shell::new();
    let mut status = connected();
    status.prompts = vec![AgentPrompt {
        id: 4,
        kind: "write".to_string(),
        client: "Claude Code".to_string(),
        path: "/home/me/Downloads/housing.step".to_string(),
        folder: "/home/me/Downloads".to_string(),
        file_name: "housing.step".to_string(),
        command: Some("file.export".to_string()),
    }];
    shell.set_agents_status(status);
    let mut window = Window::sized(390.0, 844.0);
    window.frames(&mut shell, &mut session, 30);
    let hits = shell.agent_prompt_hits().to_vec();
    let names: Vec<&str> = hits.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["Allow once", "Allow this folder", "Deny"]);
    for pair in hits.windows(2) {
        assert!(
            pair[0].1.bottom() <= pair[1].1.top() + 0.5,
            "stacked: {hits:?}"
        );
    }
    for (name, rect) in &hits {
        assert!(window.inside(*rect), "{name}: {rect:?}");
        assert!(
            rect.width() > 390.0 * 0.7,
            "{name} spans the sheet: {rect:?}"
        );
    }
    let once = hits[0].1.center();
    window.click(&mut shell, &mut session, once);
    assert_eq!(
        shell.take_agent_prompt_answer(),
        Some((4, craft3d_ui_egui::PromptAnswer::Once))
    );
}
