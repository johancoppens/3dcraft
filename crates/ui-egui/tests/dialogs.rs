//! The window's error and export dialogs (2026-10-23): drawn over the
//! window, answered with the keys as with the buttons.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::Shell;
use craft3d_ui_egui::files::{ExportCheck, ExportKind, ExportQuality};
use egui::{Event, Pos2, vec2};
use serde_json::json;

fn frame(ctx: &egui::Context, shell: &mut Shell, session: &mut Session, events: Vec<Event>) {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(900.0, 600.0))),
        events,
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| shell.ui(ui, session));
    output.textures_delta.clear();
}

fn key(key: egui::Key) -> Vec<Event> {
    vec![Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]
}

#[test]
fn the_error_dialog_says_it_plainly_and_closes_with_ok() {
    let ctx = egui::Context::default();
    let mut session = Session::new(Workspace::memory());
    session.execute("file.new", &json!({})).unwrap();
    let mut shell = Shell::new();
    frame(&ctx, &mut shell, &mut session, Vec::new());
    // A cancelled dialog is no error.
    shell.show_error("file.save", "cancelled");
    assert!(shell.error_dialog().is_none());
    shell.show_error("file.save", "file.save: file.save: the disk is full");
    let error = shell.error_dialog().unwrap().clone();
    assert_eq!(error.title, "Could not save");
    assert_eq!(error.message, "The disk is full");
    for _ in 0..2 {
        frame(&ctx, &mut shell, &mut session, Vec::new());
    }
    assert!(shell.error_dialog().is_some(), "it stays until answered");
    frame(&ctx, &mut shell, &mut session, key(egui::Key::Enter));
    assert!(shell.error_dialog().is_none());
}

#[test]
fn the_export_dialog_remembers_the_quality_and_escape_cancels() {
    let ctx = egui::Context::default();
    let mut session = Session::new(Workspace::memory());
    session.execute("file.new", &json!({})).unwrap();
    let mut shell = Shell::new();
    let check = ExportCheck {
        kind: Some(ExportKind::Bodies),
        mesh: true,
        problem: None,
    };
    assert_eq!(shell.export_quality(), ExportQuality::Normal);
    frame(&ctx, &mut shell, &mut session, Vec::new());
    shell.ask_export("part.stl".to_string(), check.clone());
    frame(&ctx, &mut shell, &mut session, Vec::new());
    let mut plan = shell.export_dialog().unwrap().plan;
    plan.quality = ExportQuality::Fine;
    shell.answer_export(Some(plan));
    assert_eq!(
        shell.take_export_answer(),
        Some(("part.stl".to_string(), plan))
    );
    assert_eq!(shell.take_export_answer(), None, "once");
    // Offered next time.
    shell.ask_export("again.stl".to_string(), check);
    assert_eq!(
        shell.export_dialog().unwrap().plan.quality,
        ExportQuality::Fine
    );
    frame(&ctx, &mut shell, &mut session, Vec::new());
    frame(&ctx, &mut shell, &mut session, key(egui::Key::Escape));
    assert!(shell.export_dialog().is_none());
    assert_eq!(shell.take_export_answer(), None);
    let report = shell.last_command.clone().unwrap();
    assert_eq!(
        (report.label.as_str(), report.ok, report.text.as_str()),
        ("file.export", false, "cancelled")
    );
}

#[test]
fn one_export_question_at_a_time_and_only_for_the_document_it_was_asked_for() {
    let ctx = egui::Context::default();
    let mut session = Session::new(Workspace::memory());
    session.execute("file.new", &json!({})).unwrap();
    let mut shell = Shell::new();
    frame(&ctx, &mut shell, &mut session, Vec::new());
    let check = ExportCheck {
        kind: Some(ExportKind::Bodies),
        mesh: true,
        problem: None,
    };
    assert!(shell.ask_export("one.stl".to_string(), check.clone()));
    // A second Export… while it is open does not replace it.
    assert!(!shell.ask_export("two.stl".to_string(), check.clone()));
    assert_eq!(shell.export_dialog().unwrap().path, "one.stl");
    // Another document shown meanwhile: refused, said in the error
    // dialog, nothing to export.
    session.execute("file.new", &json!({})).unwrap();
    frame(&ctx, &mut shell, &mut session, Vec::new());
    let plan = shell.export_dialog().unwrap().plan;
    shell.answer_export(Some(plan));
    assert_eq!(shell.take_export_answer(), None);
    let error = shell.error_dialog().unwrap().clone();
    assert_eq!(error.title, "Could not export");
    assert!(error.message.starts_with("Another document"), "{error:?}");
    // The report is the one of that moment: a later line in the status
    // bar does not change what Copy details copies.
    assert!(
        error.report.contains("another document"),
        "{}",
        error.report
    );
    shell.last_command = Some(craft3d_ui_egui::CommandReport {
        label: "feature.block".to_string(),
        ok: false,
        text: "something else".to_string(),
    });
    assert_eq!(shell.error_dialog().unwrap().report, error.report);
    // Enter answers the dialog, and only the dialog.
    frame(&ctx, &mut shell, &mut session, key(egui::Key::Enter));
    assert!(shell.error_dialog().is_none());
}
