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
use craft3d_ui_egui::files::{
    DrawingPlan, DrawingView, ExportCheck, ExportKind, ExportQuality, SheetPlan,
};
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
        drawing: false,
        problem: None,
        ..ExportCheck::default()
    };
    assert_eq!(shell.export_quality(), ExportQuality::Normal);
    frame(&ctx, &mut shell, &mut session, Vec::new());
    shell.ask_export("part.stl".to_string(), check.clone());
    frame(&ctx, &mut shell, &mut session, Vec::new());
    let mut plan = shell.export_dialog().unwrap().plan.clone();
    plan.quality = ExportQuality::Fine;
    shell.answer_export(Some(plan.clone()));
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
        drawing: false,
        problem: None,
        ..ExportCheck::default()
    };
    assert!(shell.ask_export("one.stl".to_string(), check.clone()));
    // A second Export… while it is open does not replace it.
    assert!(!shell.ask_export("two.stl".to_string(), check.clone()));
    assert_eq!(shell.export_dialog().unwrap().path, "one.stl");
    // Another document shown meanwhile: refused, said in the error
    // dialog, nothing to export.
    session.execute("file.new", &json!({})).unwrap();
    frame(&ctx, &mut shell, &mut session, Vec::new());
    let plan = shell.export_dialog().unwrap().plan.clone();
    shell.answer_export(Some(plan.clone()));
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

#[test]
fn a_drawing_of_the_model_asks_its_view_and_remembers_it() {
    let ctx = egui::Context::default();
    let mut session = Session::new(Workspace::memory());
    session.execute("file.new", &json!({})).unwrap();
    session
        .execute("feature.block", &json!({"size": [10, 10, 10]}))
        .unwrap();
    let mut shell = Shell::new();
    frame(&ctx, &mut shell, &mut session, Vec::new());
    // Nothing selected, to DXF: a drawing of the model as seen.
    let check = craft3d_ui_egui::files::export_check(&mut session, "part.dxf");
    assert!(check.drawing && check.asks() && !check.mesh);
    assert!(shell.ask_export("part.dxf".to_string(), check.clone()));
    frame(&ctx, &mut shell, &mut session, Vec::new());
    let mut plan = shell.export_dialog().unwrap().plan.clone();
    assert_eq!(plan.drawing, DrawingPlan::default());
    assert_eq!(plan.drawing.view, DrawingView::Current);
    plan.drawing = DrawingPlan {
        view: DrawingView::Top,
        perspective: false,
        dashed: true,
    };
    shell.answer_export(Some(plan.clone()));
    assert_eq!(
        shell.take_export_answer(),
        Some(("part.dxf".to_string(), plan.clone()))
    );
    // What the file gets, and the same choice offered next time.
    let params = craft3d_ui_egui::files::export_params(&mut session, "part.dxf", &plan).unwrap();
    assert_eq!(
        params,
        vec![
            ("view", json!("top")),
            ("projection", json!("orthographic")),
            ("hidden", json!("dashed")),
        ]
    );
    assert!(shell.ask_export("again.svg".to_string(), check));
    assert_eq!(shell.export_dialog().unwrap().plan.drawing, plan.drawing);
    // Enter exports with what is shown.
    frame(&ctx, &mut shell, &mut session, Vec::new());
    frame(&ctx, &mut shell, &mut session, key(egui::Key::Enter));
    let (path, answered) = shell.take_export_answer().unwrap();
    assert_eq!(path, "again.svg");
    assert_eq!(answered.drawing, plan.drawing);
}

/// A click on the export dialog goes to the dialog, never through it to
/// the model under it.
#[test]
fn a_click_on_the_export_dialog_does_not_reach_the_viewport() {
    let ctx = egui::Context::default();
    let mut session = Session::new(Workspace::memory());
    session
        .execute("file.new", &json!({"units": "mm"}))
        .unwrap();
    session
        .execute(
            "feature.block",
            &json!({"size": [400, 400, 400], "position": [-200, -200, -200]}),
        )
        .unwrap();
    let mut shell = Shell::new();
    let mut time = 0.0;
    let mut step = |shell: &mut Shell, session: &mut Session, events: Vec<Event>| {
        time += 0.05;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(900.0, 600.0))),
            time: Some(time),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| shell.ui(ui, session));
        output.textures_delta.clear();
    };
    for _ in 0..3 {
        step(&mut shell, &mut session, Vec::new());
    }
    let check = craft3d_ui_egui::files::export_check(&mut session, "part.stl");
    assert!(shell.ask_export("part.stl".to_string(), check));
    for _ in 0..3 {
        step(&mut shell, &mut session, Vec::new());
    }
    let dialog = ctx
        .memory(|m| m.area_rect(egui::Id::new("craft3d-export-dialog")))
        .unwrap();
    let at = dialog.left_top() + vec2(dialog.width() * 0.8, 8.0);
    let button = |pressed| Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    step(&mut shell, &mut session, vec![Event::PointerMoved(at)]);
    step(&mut shell, &mut session, vec![button(true)]);
    step(&mut shell, &mut session, vec![button(false)]);
    step(&mut shell, &mut session, Vec::new());
    let selection = session.execute("select.get", &json!({})).unwrap();
    assert_eq!(
        selection["items"],
        json!([]),
        "{dialog:?} {at:?} {selection}"
    );
    assert!(shell.export_dialog().is_some());
    // A DXF export of a document with a named set: a click on "Sheet of
    // set" chooses it (its place in a dialog drawn with the theme).
    session
        .execute(
            "set.define",
            &json!({"name": "Cut sheet", "kind": "face", "where": {"normal": {"along": "z"}}}),
        )
        .unwrap();
    shell.answer_export(None);
    let check = craft3d_ui_egui::files::export_check(&mut session, "part.dxf");
    assert!(shell.ask_export("part.dxf".to_string(), check));
    for _ in 0..3 {
        step(&mut shell, &mut session, Vec::new());
    }
    let dialog = ctx
        .memory(|m| m.area_rect(egui::Id::new("craft3d-export-dialog")))
        .unwrap();
    let at = dialog.left_top() + vec2(134.0, 61.0);
    let button = |pressed| Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    step(&mut shell, &mut session, vec![Event::PointerMoved(at)]);
    step(&mut shell, &mut session, vec![button(true)]);
    step(&mut shell, &mut session, vec![button(false)]);
    step(&mut shell, &mut session, Vec::new());
    let selection = session.execute("select.get", &json!({})).unwrap();
    assert_eq!(
        selection["items"],
        json!([]),
        "{dialog:?} {at:?} {selection}"
    );
    assert_eq!(
        shell.export_dialog().unwrap().plan.sheet.set.as_deref(),
        Some("Cut sheet"),
        "{dialog:?}"
    );
}

/// A DXF or SVG export of a document with named sets asks what goes in:
/// the selection or the view, or a sheet of a set with its width and gap,
/// remembered for the next export (R1 A6).
#[test]
fn a_named_set_is_offered_as_a_sheet_and_remembered() {
    let ctx = egui::Context::default();
    let mut session = Session::new(Workspace::memory());
    session
        .execute("file.new", &json!({"units": "mm"}))
        .unwrap();
    session
        .execute("feature.block", &json!({"size": [10, 10, 3]}))
        .unwrap();
    session
        .execute(
            "set.define",
            &json!({"name": "Cut sheet", "kind": "face", "where": {"normal": {"along": "z"}}}),
        )
        .unwrap();
    let mut shell = Shell::new();
    frame(&ctx, &mut shell, &mut session, Vec::new());
    let check = craft3d_ui_egui::files::export_check(&mut session, "part.dxf");
    assert_eq!(check.sets, vec!["Cut sheet"]);
    assert!(check.asks());
    assert!(shell.ask_export("part.dxf".to_string(), check.clone()));
    for _ in 0..2 {
        frame(&ctx, &mut shell, &mut session, Vec::new());
    }
    // The view is chosen until a set is.
    let mut plan = shell.export_dialog().unwrap().plan.clone();
    assert_eq!(plan.sheet, SheetPlan::default());
    plan.sheet = SheetPlan {
        set: Some("Cut sheet".to_string()),
        width: "300 mm".to_string(),
        gap: "2 mm".to_string(),
    };
    shell.answer_export(Some(plan.clone()));
    let (_, answered) = shell.take_export_answer().unwrap();
    assert_eq!(answered.sheet, plan.sheet);
    assert_eq!(shell.export_sheet(), plan.sheet);
    assert_eq!(
        craft3d_ui_egui::files::export_params(&mut session, "part.dxf", &answered).unwrap(),
        vec![
            ("set", json!("Cut sheet")),
            ("width", json!("300 mm")),
            ("gap", json!("2 mm")),
        ]
    );
    // Offered again, drawn with its fields, and Enter exports it.
    assert!(shell.ask_export("again.svg".to_string(), check));
    for _ in 0..2 {
        frame(&ctx, &mut shell, &mut session, Vec::new());
    }
    assert_eq!(shell.export_dialog().unwrap().plan.sheet, plan.sheet);
    frame(&ctx, &mut shell, &mut session, key(egui::Key::Enter));
    let (path, answered) = shell.take_export_answer().unwrap();
    assert_eq!(path, "again.svg");
    assert_eq!(answered.sheet.set.as_deref(), Some("Cut sheet"));
    // A document without that set: the sheet is not kept.
    session.execute("file.new", &json!({})).unwrap();
    frame(&ctx, &mut shell, &mut session, Vec::new());
    let plain = craft3d_ui_egui::files::export_check(&mut session, "x.dxf");
    assert!(plain.sets.is_empty());
    assert!(shell.ask_export("x.dxf".to_string(), plain));
    frame(&ctx, &mut shell, &mut session, Vec::new());
    frame(&ctx, &mut shell, &mut session, key(egui::Key::Enter));
    let (_, answered) = shell.take_export_answer().unwrap();
    assert_eq!(answered.sheet.set, None);
    // Each document keeps its own: the first one's comes back with it.
    assert_eq!(shell.export_sheet(), SheetPlan::default());
    session
        .execute("file.activate", &json!({"document": 0}))
        .unwrap();
    frame(&ctx, &mut shell, &mut session, Vec::new());
    assert_eq!(shell.export_sheet(), plan.sheet);
}
