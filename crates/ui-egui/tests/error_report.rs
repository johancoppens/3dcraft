//! "Copy details" in the status bar: after a command failed it puts a
//! plain-text bug report on the clipboard, and `ui.get` gives the same
//! report to an agent. Without a window (`egui::Context::run_ui` with
//! synthetic clicks).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, VERSION, Workspace};
use craft3d_ui_egui::{CommandReport, Shell};
use egui::{Event, OutputCommand, PointerButton, Pos2, vec2};
use serde_json::json;

struct Window {
    ctx: egui::Context,
    time: f64,
}

impl Window {
    fn frame(
        &mut self,
        shell: &mut Shell,
        session: &mut Session,
        events: Vec<Event>,
    ) -> egui::FullOutput {
        self.time += 1.0 / 60.0;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 700.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
        output.textures_delta.clear();
        output
    }

    /// Clicks at `at`; returns what the clicking frames asked to copy.
    fn click(&mut self, shell: &mut Shell, session: &mut Session, at: Pos2) -> Vec<String> {
        self.frame(shell, session, vec![Event::PointerMoved(at)]);
        let mut copied = Vec::new();
        for pressed in [true, false] {
            let output = self.frame(
                shell,
                session,
                vec![Event::PointerButton {
                    pos: at,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            for command in output.platform_output.commands {
                if let OutputCommand::CopyText(text) = command {
                    copied.push(text);
                }
            }
        }
        copied
    }
}

#[test]
fn copy_details_appears_after_a_failure_and_copies_the_report() {
    let mut session = Session::new(Workspace::memory());
    let mut shell = Shell::new();
    let mut window = Window {
        ctx: egui::Context::default(),
        time: 0.0,
    };
    session.execute("file.new", &json!({})).unwrap();
    session
        .execute("feature.block", &json!({"size": [1, 2, 3]}))
        .unwrap();
    shell.last_command = Some(CommandReport {
        label: "feature.block".into(),
        ok: true,
        text: "{}".into(),
    });
    for _ in 0..3 {
        window.frame(&mut shell, &mut session, Vec::new());
    }
    assert!(
        shell.copy_details_rect().is_none(),
        "no button after a success"
    );

    let params = json!({"size": "a block, please"});
    let error = session.execute("feature.block", &params).unwrap_err();
    shell.last_command = Some(CommandReport {
        label: "feature.block".into(),
        ok: false,
        text: error.message().to_string(),
    });
    for _ in 0..3 {
        window.frame(&mut shell, &mut session, Vec::new());
    }
    let button = shell.copy_details_rect().expect("Copy details is shown");
    let bar = shell
        .status_hits()
        .iter()
        .map(|(_, rect)| *rect)
        .reduce(|a, b| a.union(b))
        .unwrap();
    assert!(button.min.y >= bar.min.y - 4.0, "{button:?} in {bar:?}");
    for (slot, rect) in shell.status_hits() {
        assert!(
            !rect.intersects(button.shrink(1.0)),
            "{slot:?} {rect:?} overlaps {button:?}"
        );
    }

    let copied = window.click(&mut shell, &mut session, button.center());
    assert_eq!(copied.len(), 1, "{copied:?}");
    let text = &copied[0];
    assert!(text.starts_with("3DCraft bug report\n"), "{text}");
    assert!(text.contains(&format!("version: {VERSION}\n")), "{text}");
    assert!(text.contains("command: feature.block\n"), "{text}");
    assert!(text.contains(&format!("params: {params}\n")), "{text}");
    assert!(
        text.contains(&format!("error: {}\n", error.message())),
        "{text}"
    );
    assert!(text.contains("renderer: "), "{text}");

    // The window that shows it, for reading before posting and for copying
    // by hand where a browser refuses the clipboard.
    assert!(shell.error_details_open());
    window.frame(&mut shell, &mut session, Vec::new());
    assert!(shell.error_details_open());

    // The agent's way to the same text.
    let got = shell
        .handle_method(&mut session, "ui.get", None)
        .unwrap()
        .unwrap();
    assert_eq!(got["report"].as_str(), Some(text.as_str()));
}

#[test]
fn a_narrow_status_bar_still_fits_the_button() {
    let mut session = Session::new(Workspace::memory());
    let mut shell = Shell::new();
    let mut window = Window {
        ctx: egui::Context::default(),
        time: 0.0,
    };
    session.execute("no.such.command", &json!({})).unwrap_err();
    shell.last_command = Some(CommandReport {
        label: "no.such.command".into(),
        ok: false,
        text: "x".repeat(500),
    });
    for _ in 0..3 {
        window.frame(&mut shell, &mut session, Vec::new());
    }
    let button = shell.copy_details_rect().expect("Copy details is shown");
    assert!(button.max.x <= 1000.5, "{button:?}");
}
