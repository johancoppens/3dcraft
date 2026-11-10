//! The status bar's agents pill and its popover (2026-11-02): at the
//! bar's right end (decision of 2026-11-02, board s3), opened with a click or `ui.set {"dialogs": {"agents":
//! true}}`, with the same controls as the start screen's "Connect an AI
//! agent" block, and inside the window at 1024 × 700 and on a phone.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::shell::agents::PillKind;
use craft3d_ui_egui::{AgentClient, AgentRoot, AgentsStatus, Shell};
use egui::{Event, Pos2, vec2};
use serde_json::json;

struct Window {
    ctx: egui::Context,
    time: f64,
    size: egui::Vec2,
    /// What the last frame painted.
    shapes: Vec<egui::epaint::ClippedShape>,
}

impl Window {
    fn sized(width: f32, height: f32) -> Self {
        Self {
            ctx: egui::Context::default(),
            time: 0.0,
            size: vec2(width, height),
            shapes: Vec::new(),
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
        self.shapes = output.shapes;
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
}

fn on(clients: Vec<AgentClient>) -> AgentsStatus {
    AgentsStatus {
        available: true,
        allowed: true,
        port: Some(7990),
        connected: clients.len(),
        clients,
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

fn names(hits: &[(String, egui::Rect)]) -> Vec<String> {
    let mut names: Vec<String> = hits.iter().map(|(name, _)| name.clone()).collect();
    names.sort();
    names
}

#[test]
fn the_pill_opens_the_popover_with_the_start_screens_controls() {
    let mut session = Session::new(Workspace::none());
    let mut shell = Shell::new();
    shell.set_agents_status(on(vec![AgentClient {
        name: "Claude Code".to_string(),
        since_ms: 1,
        last_command: Some("feature.edit".to_string()),
        last_ms: 2,
        cwd: None,
        key: Some("MCP bridge (process 1)".to_string()),
    }]));
    let mut window = Window::sized(1400.0, 900.0);
    for _ in 0..3 {
        window.frame(&mut shell, &mut session, Vec::new());
    }
    // The start screen's block: its controls.
    let block: Vec<String> = names(shell.start_hits())
        .into_iter()
        .filter(|name| {
            [
                "agents-switch",
                "change-folder",
                "open-folder",
                "document-folder",
                "copy-command",
                "copy-config",
                "other-clients",
                "disconnect-all",
                // The connected agent's own ✕ (2026-11-04), by its key.
                "disconnect:MCP bridge (process 1)",
            ]
            .contains(&name.as_str())
        })
        .collect();
    assert_eq!(block.len(), 9, "{block:?}");
    // The pill is at the status bar's right end.
    let pill = shell.agents_pill_rect().unwrap();
    assert!(
        pill.right() > 1400.0 - 40.0 && pill.right() <= 1400.0 && pill.bottom() > 850.0,
        "{pill:?}"
    );
    assert_eq!(shell.agents_pill_view().kind, PillKind::Connected);
    assert!(shell.agents_popover_hits().is_empty());
    window.click(&mut shell, &mut session, pill.center());
    assert!(shell.state.dialogs.agents, "a click opens it");
    assert_eq!(names(shell.agents_popover_hits()), block, "the same panel");
    // Every control of the popover lies above the pill and in the window.
    for (name, rect) in shell.agents_popover_hits() {
        assert!(rect.bottom() <= pill.top() + 0.5, "{name}: {rect:?}");
        assert!(
            rect.right() <= 1400.0 && rect.left() >= 0.0,
            "{name}: {rect:?}"
        );
    }
    // ui.get reads it; Escape closes it.
    let got = shell
        .handle_method(&mut session, "ui.get", None)
        .unwrap()
        .unwrap();
    assert_eq!(got["state"]["dialogs"]["agents"], true);
    assert_eq!(got["agents"]["clients"][0]["name"], "Claude Code");
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
    assert!(!shell.state.dialogs.agents);
    // ui.set opens it, for an agent showing the person around.
    shell
        .handle_method(
            &mut session,
            "ui.set",
            Some(&json!({"dialogs": {"agents": true}})),
        )
        .unwrap()
        .unwrap();
    window.frame(&mut shell, &mut session, Vec::new());
    assert!(!shell.agents_popover_hits().is_empty());
    // A click on the switch reaches the host.
    let switch = shell
        .agents_popover_hits()
        .iter()
        .find(|(name, _)| name == "agents-switch")
        .unwrap()
        .1;
    window.click(
        &mut shell,
        &mut session,
        switch.left_center() + vec2(8.0, 0.0),
    );
    assert_eq!(shell.take_agents_switch(), Some(false));
}

#[test]
fn off_the_pill_is_an_invitation_and_the_help_menu_opens_the_popover() {
    let mut session = Session::new(Workspace::none());
    let mut shell = Shell::new();
    shell.set_agents_status(AgentsStatus {
        available: true,
        ..AgentsStatus::default()
    });
    let mut window = Window::sized(1024.0, 700.0);
    for _ in 0..3 {
        window.frame(&mut shell, &mut session, Vec::new());
    }
    let view = shell.agents_pill_view();
    assert_eq!(
        (view.kind, view.label.as_str()),
        (PillKind::Off, "Agents off")
    );
    shell.show_agents_popover(true);
    window.frame(&mut shell, &mut session, Vec::new());
    // Off: the switch, and nothing to copy yet.
    let hits = names(shell.agents_popover_hits());
    assert_eq!(hits, ["agents-switch"]);
}

/// Neither the pill nor the popover runs past the window: at 1024 × 700
/// and on a phone, with a long client name and the popover open.
#[test]
fn the_pill_and_the_popover_fit_small_windows() {
    for (width, height) in [(1024.0, 700.0), (390.0, 844.0)] {
        let mut session = Session::new(Workspace::none());
        session.execute("file.new", &json!({})).unwrap();
        let mut shell = Shell::new();
        shell.set_agents_status(on(vec![AgentClient {
            name: "A very long client name that goes on and on and on".to_string(),
            since_ms: 1,
            last_command: Some("feature.edit".to_string()),
            last_ms: 1_000,
            cwd: None,
            key: None,
        }]));
        // Doing something now: the longest label.
        shell.set_agents_clock(Some(1_200));
        shell.show_agents_popover(true);
        let mut window = Window::sized(width, height);
        for _ in 0..4 {
            window.frame(&mut shell, &mut session, Vec::new());
        }
        assert_eq!(shell.agents_pill_view().kind, PillKind::Active);
        let pill = shell.agents_pill_rect().unwrap();
        // At the right end, at most two fifths of the bar wide (cut with
        // an ellipsis), so it starts past the bar's middle.
        assert!(
            pill.right() <= width + 0.5 && pill.right() > width - 40.0,
            "{width}: {pill:?}"
        );
        assert!(pill.width() <= width * 0.4 + 1.0, "{width}: {pill:?}");
        assert!(!shell.agents_popover_hits().is_empty());
        for (name, rect) in shell.agents_popover_hits() {
            assert!(
                rect.left() >= -0.5 && rect.right() <= width + 0.5,
                "{width}: {name} {rect:?}"
            );
            assert!(rect.top() >= -0.5, "{width}: {name} {rect:?}");
        }
    }
}

/// A question takes no click for a moment after it appears (a click or a
/// double click meant for something else never answers it), offers "no"
/// first, and says folder and file on lines of their own.
#[test]
fn a_question_takes_no_click_at_first_and_offers_no_first() {
    use craft3d_ui_egui::{AgentPrompt, PromptAnswer};
    let mut session = Session::new(Workspace::none());
    session.execute("file.new", &json!({})).unwrap();
    let mut shell = Shell::new();
    let mut status = on(Vec::new());
    status.prompts = vec![AgentPrompt {
        id: 7,
        kind: "write".to_string(),
        client: "Claude Code".to_string(),
        path: "/home/me/out/part.stl".to_string(),
        folder: "/home/me/out".to_string(),
        file_name: "part.stl".to_string(),
        command: Some("file.export".to_string()),
    }];
    shell.set_agents_status(status);
    let mut window = Window::sized(1400.0, 900.0);
    // A few frames (a twentieth of a second): the window has its size.
    for _ in 0..3 {
        window.frame(&mut shell, &mut session, Vec::new());
    }
    let hits = shell.agent_prompt_hits().to_vec();
    let labels: Vec<&str> = hits.iter().map(|(name, _)| name.as_str()).collect();
    // "No" first at the left, the amber one last at the right (board s3).
    assert_eq!(labels, ["Deny", "Allow this folder", "Allow once"]);
    for pair in hits.windows(2) {
        assert!(pair[0].1.left() < pair[1].1.left(), "{hits:?}");
    }
    // At once: the click does nothing.
    let once = hits[2].1.center();
    window.click(&mut shell, &mut session, once);
    assert_eq!(shell.take_agent_prompt_answer(), None, "taken too early");
    // After the moment: it answers.
    for _ in 0..60 {
        window.frame(&mut shell, &mut session, Vec::new());
    }
    assert_eq!(
        shell.agent_prompt_hits()[2].1.center(),
        once,
        "the same place"
    );
    window.click(&mut shell, &mut session, once);
    assert_eq!(
        shell.take_agent_prompt_answer(),
        Some((7, PromptAnswer::Once))
    );
}

/// The question at connect (decision of 2026-11-02): Refuse at the left,
/// Only the work folder, then Allow this folder (amber) at the right;
/// allowing is for this run, or remembered with "Remember for this
/// folder"; refusing keeps the connection (the host's).
#[test]
fn the_question_at_connect_allows_for_the_run_or_remembers() {
    use craft3d_ui_egui::{AgentPrompt, PromptAnswer};
    for (remember, expected) in [(false, PromptAnswer::Session), (true, PromptAnswer::Folder)] {
        let mut session = Session::new(Workspace::none());
        let mut shell = Shell::new();
        let mut status = on(Vec::new());
        status.prompts = vec![AgentPrompt {
            id: 9,
            kind: "connect".to_string(),
            client: "Claude Code".to_string(),
            path: "/home/me/code/pillow-block".to_string(),
            folder: "/home/me/code/pillow-block".to_string(),
            ..AgentPrompt::default()
        }];
        shell.set_agents_status(status);
        let mut window = Window::sized(1024.0, 700.0);
        for _ in 0..60 {
            window.frame(&mut shell, &mut session, Vec::new());
        }
        let hit = |shell: &Shell, name: &str| {
            shell
                .agent_prompt_hits()
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, r)| *r)
                .unwrap_or_else(|| panic!("{name}: {:?}", shell.agent_prompt_hits()))
        };
        let names: Vec<String> = shell
            .agent_prompt_hits()
            .iter()
            .map(|(n, _)| n.clone())
            .collect();
        assert_eq!(
            names,
            [
                "Refuse",
                "Only the work folder",
                "Allow this folder",
                "remember"
            ]
        );
        // Every button inside the window.
        for (name, rect) in shell.agent_prompt_hits() {
            assert!(
                rect.left() >= 0.0 && rect.right() <= 1024.0 && rect.bottom() <= 700.0,
                "{name}: {rect:?}"
            );
        }
        if remember {
            let check = hit(&shell, "remember");
            window.click(
                &mut shell,
                &mut session,
                check.left_center() + vec2(6.0, 0.0),
            );
        }
        let allow = hit(&shell, "Allow this folder").center();
        window.click(&mut shell, &mut session, allow);
        assert_eq!(shell.take_agent_prompt_answer(), Some((9, expected)));
    }
    // Only the work folder refuses the folder only; Refuse disconnects
    // the agent too (the developer's decision of 2026-11-04: until then
    // both answered `deny`).
    let mut session = Session::new(Workspace::none());
    let mut shell = Shell::new();
    let mut status = on(Vec::new());
    status.prompts = vec![AgentPrompt {
        id: 10,
        kind: "connect".to_string(),
        client: "Claude Code".to_string(),
        folder: "/tmp/x".to_string(),
        ..AgentPrompt::default()
    }];
    shell.set_agents_status(status);
    let mut window = Window::sized(1024.0, 700.0);
    for _ in 0..60 {
        window.frame(&mut shell, &mut session, Vec::new());
    }
    let at = |shell: &Shell, name: &str| {
        shell
            .agent_prompt_hits()
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, r)| r.center())
            .unwrap()
    };
    let refuse = at(&shell, "Refuse");
    window.click(&mut shell, &mut session, refuse);
    assert_eq!(
        shell.take_agent_prompt_answer(),
        Some((10, PromptAnswer::Refuse))
    );
    let only = at(&shell, "Only the work folder");
    window.click(&mut shell, &mut session, only);
    assert_eq!(
        shell.take_agent_prompt_answer(),
        Some((10, PromptAnswer::Deny))
    );
}

/// Each connected agent in the agent panel has a ✕ that asks the host to
/// disconnect that one (by the name it authenticated with); one without
/// such a name has none.
#[test]
fn the_cross_of_an_agent_disconnects_that_one() {
    let mut session = Session::new(Workspace::none());
    let mut shell = Shell::new();
    let client = |name: &str, key: Option<&str>| AgentClient {
        name: name.to_string(),
        since_ms: 1,
        last_command: None,
        last_ms: 0,
        cwd: None,
        key: key.map(str::to_string),
    };
    shell.set_agents_status(on(vec![
        client("Claude Code", Some("MCP bridge (process 7)")),
        client("control connection", None),
    ]));
    let mut window = Window::sized(1400.0, 900.0);
    shell
        .handle_method(
            &mut session,
            "ui.set",
            Some(&json!({"dialogs": {"agents": true}})),
        )
        .unwrap()
        .unwrap();
    for _ in 0..3 {
        window.frame(&mut shell, &mut session, Vec::new());
    }
    let crosses: Vec<(String, egui::Rect)> = shell
        .agents_popover_hits()
        .iter()
        .filter(|(n, _)| n.starts_with("disconnect:"))
        .cloned()
        .collect();
    assert_eq!(crosses.len(), 1, "{crosses:?}");
    assert_eq!(crosses[0].0, "disconnect:MCP bridge (process 7)");
    window.click(&mut shell, &mut session, crosses[0].1.center());
    assert_eq!(
        shell.take_agent_disconnect().as_deref(),
        Some("MCP bridge (process 7)")
    );
    assert!(!shell.take_agents_disconnect(), "not all of them");
}

/// `ui.get` over the control channel shows an agent its own entry: not
/// other agents' folders or questions.
#[test]
fn an_agent_sees_its_own_entry_only() {
    use craft3d_ui_egui::AgentPrompt;
    let mut session = Session::new(Workspace::none());
    let mut shell = Shell::new();
    let client = |name: &str| AgentClient {
        name: name.to_string(),
        since_ms: 1,
        last_command: None,
        last_ms: 0,
        cwd: Some(format!("/home/{name}")),
        key: Some(name.to_string()),
    };
    let mut status = on(vec![client("Claude Code"), client("Other")]);
    status.prompts = vec![AgentPrompt {
        id: 1,
        kind: "connect".to_string(),
        client: "Other".to_string(),
        path: "/home/Other".to_string(),
        folder: "/home/Other".to_string(),
        ..AgentPrompt::default()
    }];
    shell.set_agents_status(status);
    let get = |shell: &mut Shell, session: &mut Session| {
        shell
            .handle_method(session, "ui.get", None)
            .unwrap()
            .unwrap()["agents"]
            .clone()
    };
    let all = get(&mut shell, &mut session);
    assert_eq!(
        all["clients"].as_array().unwrap().len(),
        2,
        "the person sees all"
    );
    shell.set_agents_viewer(Some("Claude Code"));
    let own = get(&mut shell, &mut session);
    assert_eq!(own["clients"].as_array().unwrap().len(), 1);
    assert_eq!(own["clients"][0]["name"], "Claude Code");
    assert!(own["prompts"].as_array().unwrap().is_empty());
    shell.set_agents_viewer(None);
}

/// A file an agent opened or wrote stays out of the recent files: opening
/// it from there later would quietly give its folder to agents.
#[test]
fn an_agents_files_stay_out_of_the_recent_files() {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("agents-recent");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = |name: &str| dir.join(name).to_str().unwrap().to_string();
    let mut session = Session::new(Workspace::unrestricted());
    let mut shell = Shell::new();
    session.set_actor(Some("Claude Code"));
    session.execute("file.new", &json!({})).unwrap();
    session
        .execute("file.save", &json!({"path": path("agent.3dcraft")}))
        .unwrap();
    session.set_actor(None);
    shell.sync(&mut session);
    assert!(
        shell.recent_list().paths().is_empty(),
        "{:?}",
        shell.recent_list().paths()
    );
    session.execute("file.new", &json!({})).unwrap();
    session
        .execute("file.save", &json!({"path": path("mine.3dcraft")}))
        .unwrap();
    shell.sync(&mut session);
    let listed = shell.recent_list().paths().to_vec();
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert!(listed[0].ends_with("mine.3dcraft"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The rectangles the last frame painted with some colour, nested ones
/// included.
fn painted_rects(shapes: &[egui::epaint::ClippedShape]) -> Vec<egui::Rect> {
    fn walk(shape: &egui::Shape, out: &mut Vec<egui::Rect>) {
        match shape {
            egui::Shape::Rect(r) if r.fill.a() > 0 => out.push(r.rect),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in shapes {
        walk(&clipped.shape, &mut out);
    }
    out
}

/// The start screen is reachable in full at any height. With the agents'
/// switch on, its "Connect an AI agent" block runs past the foot of a
/// window 900 high (the Copy buttons were cut under the status bar,
/// 2026-11-10): the screen scrolls, says so with its scroll bar at the
/// window's right edge while the pointer is elsewhere, and the wheel
/// brings the block's end above the status bar.
#[test]
fn the_start_screen_scrolls_to_the_end_of_the_agents_block() {
    for (width, height) in [(1400.0, 900.0), (1024.0, 700.0)] {
        let mut session = Session::new(Workspace::none());
        let mut shell = Shell::new();
        shell.set_agents_status(on(Vec::new()));
        let mut window = Window::sized(width, height);
        for _ in 0..3 {
            window.frame(&mut shell, &mut session, Vec::new());
        }
        assert!(shell.start_shown());
        let viewport = shell.viewport_rect().unwrap();
        let end = |shell: &Shell| {
            shell
                .start_hits()
                .iter()
                .filter(|(name, _)| {
                    ["card", "copy-command", "copy-config", "other-clients"]
                        .contains(&name.as_str())
                })
                .map(|(_, rect)| rect.bottom())
                .fold(f32::MIN, f32::max)
        };
        if end(&shell) > viewport.bottom() + 0.5 {
            // The scroll bar: thin and tall, at the right edge, away from
            // the content's column on a wide window.
            let bar = painted_rects(&window.shapes).into_iter().find(|r| {
                r.width() <= 12.0
                    && r.height() >= 30.0
                    && r.right() <= viewport.right()
                    && r.left() >= viewport.right() - 60.0
            });
            assert!(
                bar.is_some(),
                "{width} x {height}: the screen scrolls without a scroll bar in sight"
            );
            let at = viewport.center();
            window.frame(&mut shell, &mut session, vec![Event::PointerMoved(at)]);
            for _ in 0..20 {
                if end(&shell) <= viewport.bottom() + 0.5 {
                    break;
                }
                window.frame(
                    &mut shell,
                    &mut session,
                    vec![Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: vec2(0.0, -200.0),
                        phase: egui::TouchPhase::Move,
                        modifiers: egui::Modifiers::NONE,
                    }],
                );
                for _ in 0..30 {
                    window.frame(&mut shell, &mut session, Vec::new());
                }
            }
        }
        assert!(
            end(&shell) <= viewport.bottom() + 0.5,
            "{width} x {height}: the block ends at {} under the status bar at {}",
            end(&shell),
            viewport.bottom()
        );
    }
}
