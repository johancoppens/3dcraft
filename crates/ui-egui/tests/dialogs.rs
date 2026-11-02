//! The window's error and export dialogs (2026-10-23): drawn over the
//! window, answered with the keys as with the buttons. The export dialog
//! comes first, before the system's save dialog (2026-10-27): what goes
//! into the file, the format, the options.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::Shell;
use craft3d_ui_egui::files::{
    DrawingPlan, DrawingView, ExportFormat, ExportPlan, ExportQuality, ExportWhat, export_choices,
    export_params,
};
use craft3d_ui_egui::shell::ExportRequest;
use egui::{Event, Pos2, vec2};
use serde_json::{Value, json};

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

/// Runs `file.export` with the parameters the request makes, in the
/// session's memory, and returns the file's bytes.
fn write(session: &mut Session, request: &ExportRequest) -> Vec<u8> {
    let params = export_params(session, &request.name, &request.plan).unwrap();
    let mut call = json!({"path": request.name});
    for (key, value) in params {
        call[key] = value;
    }
    let written = session.execute("file.export", &call);
    assert!(written.is_ok(), "{request:?}: {written:?}");
    session.take_file(&request.name).unwrap()
}

/// A sheet-metal part named `name` from a rectangle in a sketch of its
/// own.
fn part(session: &mut Session, sketch: &str, name: &str, extra: Value) {
    session
        .execute("sketch.create", &json!({"plane": "xy", "name": sketch}))
        .unwrap();
    session
        .execute(
            "sketch.rect",
            &json!({"sketch": sketch, "from": [0, 0], "to": [50, 40]}),
        )
        .unwrap();
    let mut params = json!({"sketch": sketch, "name": name, "thickness": "1 mm", "radius": "1 mm",
        "kFactor": 0.4, "flanges": [{"edge": "e6", "length": "15 mm", "angle": "90 deg"}]});
    if let (Some(map), Some(more)) = (params.as_object_mut(), extra.as_object()) {
        map.extend(more.clone());
    }
    session.execute("feature.sheetMetal", &params).unwrap();
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
fn the_export_dialog_remembers_the_format_and_quality_and_escape_cancels() {
    let ctx = egui::Context::default();
    let mut session = Session::new(Workspace::memory());
    // Without a document there is nothing to export.
    let mut shell = Shell::new();
    let refused = shell.open_export(&mut session).unwrap_err();
    assert!(refused.contains("no document"), "{refused}");
    assert!(shell.export_dialog().is_none() && !shell.state.export.open);
    session.execute("file.new", &json!({})).unwrap();
    session
        .execute("feature.block", &json!({"size": [1, 2, 3]}))
        .unwrap();
    assert_eq!(shell.state.export.quality, ExportQuality::Normal);
    frame(&ctx, &mut shell, &mut session, Vec::new());
    shell.open_export(&mut session).unwrap();
    frame(&ctx, &mut shell, &mut session, Vec::new());
    assert!(shell.state.export.open);
    assert_eq!(shell.state.export.what, ExportWhat::Model);
    assert_eq!(shell.state.export.name, "untitled.step");
    shell.set_export_format(ExportFormat::Stl).unwrap();
    shell.state.export.quality = ExportQuality::Fine;
    frame(&ctx, &mut shell, &mut session, Vec::new());
    shell.answer_export(true);
    assert!(!shell.state.export.open && shell.export_dialog().is_none());
    let request = shell.take_export_answer().unwrap();
    let mut chosen = shell.state.export.clone();
    assert_eq!(chosen.format, ExportFormat::Step, "as before until written");
    chosen.format = ExportFormat::Stl;
    chosen.quality = ExportQuality::Fine;
    chosen.name = "untitled.stl".to_string();
    assert_eq!(
        request,
        ExportRequest {
            name: "untitled.stl".to_string(),
            format: ExportFormat::Stl,
            plan: ExportPlan {
                quality: ExportQuality::Fine,
                ..ExportPlan::default()
            },
            document: shell.viewing(),
            chosen,
        }
    );
    assert_eq!(shell.take_export_answer(), None, "once");
    // Not written (the save dialog cancelled, the export failed): the
    // next dialog is as before.
    shell.open_export(&mut session).unwrap();
    assert_eq!(shell.state.export.format, ExportFormat::Step);
    assert_eq!(shell.state.export.quality, ExportQuality::Normal);
    shell.answer_export(false);
    // A binary STL: a header of 80 bytes, the count, the triangles.
    assert!(write(&mut session, &request).len() > 84);
    shell.export_written(&request);
    // Written: offered next time.
    shell.open_export(&mut session).unwrap();
    assert_eq!(shell.state.export.format, ExportFormat::Stl);
    assert_eq!(shell.state.export.quality, ExportQuality::Fine);
    frame(&ctx, &mut shell, &mut session, Vec::new());
    frame(&ctx, &mut shell, &mut session, key(egui::Key::Escape));
    assert!(shell.export_dialog().is_none() && !shell.state.export.open);
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
    shell.open_export(&mut session).unwrap();
    shell.set_export_format(ExportFormat::Obj).unwrap();
    // A second Export… while it is open does not replace it.
    shell.open_export(&mut session).unwrap();
    assert_eq!(shell.state.export.format, ExportFormat::Obj);
    assert_eq!(shell.state.export.name, "untitled.obj");
    // Another document shown meanwhile: refused, said in the error
    // dialog, nothing to export.
    session.execute("file.new", &json!({})).unwrap();
    frame(&ctx, &mut shell, &mut session, Vec::new());
    shell.answer_export(true);
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
    // The model, to DXF: a drawing of it as seen.
    shell.open_export(&mut session).unwrap();
    shell.set_export_format(ExportFormat::Dxf).unwrap();
    frame(&ctx, &mut shell, &mut session, Vec::new());
    assert_eq!(shell.state.export.drawing, DrawingPlan::default());
    assert_eq!(shell.state.export.drawing.view, DrawingView::Current);
    let drawing = DrawingPlan {
        view: DrawingView::Top,
        perspective: false,
        dashed: true,
    };
    shell.state.export.drawing = drawing;
    shell.answer_export(true);
    let request = shell.take_export_answer().unwrap();
    assert_eq!(request.name, "untitled.dxf");
    // What the file gets, and the same choice offered next time.
    let params = export_params(&mut session, &request.name, &request.plan).unwrap();
    assert_eq!(
        params,
        vec![
            ("view", json!("top")),
            ("projection", json!("orthographic")),
            ("hidden", json!("dashed")),
        ]
    );
    assert!(!write(&mut session, &request).is_empty());
    shell.export_written(&request);
    shell.open_export(&mut session).unwrap();
    assert_eq!(shell.state.export.format, ExportFormat::Dxf);
    shell.set_export_format(ExportFormat::Svg).unwrap();
    assert_eq!(shell.state.export.drawing, drawing);
    // Enter exports with what is shown.
    frame(&ctx, &mut shell, &mut session, Vec::new());
    frame(&ctx, &mut shell, &mut session, key(egui::Key::Enter));
    let request = shell.take_export_answer().unwrap();
    assert_eq!(request.name, "untitled.svg");
    assert_eq!(request.plan.drawing, drawing);
}

/// A click on the export dialog goes to the dialog, never through it to
/// the model under it; a click on a choice of *What* chooses it, and the
/// format follows.
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
    shell.open_export(&mut session).unwrap();
    for _ in 0..3 {
        step(&mut shell, &mut session, Vec::new());
    }
    let dialog = ctx
        .memory(|m| m.area_rect(egui::Id::new("craft3d-export-dialog")))
        .unwrap();
    let at = dialog.left_top() + vec2(dialog.width() * 0.8, 8.0);
    let click = |at: Pos2| {
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        [
            vec![Event::PointerMoved(at)],
            vec![button(true)],
            vec![button(false)],
            Vec::new(),
        ]
    };
    for events in click(at) {
        step(&mut shell, &mut session, events);
    }
    let selection = session.execute("select.get", &json!({})).unwrap();
    assert_eq!(
        selection["items"],
        json!([]),
        "{dialog:?} {at:?} {selection}"
    );
    assert!(shell.export_dialog().is_some());
    // A document with a named set: a click on "Sheet of set" chooses it,
    // wherever the theme draws it, and DXF with it.
    session
        .execute(
            "set.define",
            &json!({"name": "Cut sheet", "kind": "face", "where": {"normal": {"along": "z"}}}),
        )
        .unwrap();
    shell.answer_export(false);
    shell.open_export(&mut session).unwrap();
    for _ in 0..3 {
        step(&mut shell, &mut session, Vec::new());
    }
    // Wherever the dialog draws it (a segment or a row), where it says it
    // drew it.
    let set = ExportWhat::Set("Cut sheet".to_string());
    let at = shell
        .export_what_hits()
        .iter()
        .find(|(what, _)| *what == set)
        .map(|(_, rect)| rect.center())
        .expect("the set is offered");
    let dialog = ctx
        .memory(|m| m.area_rect(egui::Id::new("craft3d-export-dialog")))
        .unwrap();
    assert!(dialog.contains(at), "{at:?} outside {dialog:?}");
    for events in click(at) {
        step(&mut shell, &mut session, events);
    }
    let selection = session.execute("select.get", &json!({})).unwrap();
    assert_eq!(selection["items"], json!([]), "{at:?} {selection}");
    let chosen = (shell.state.export.what == set).then_some(at);
    assert!(chosen.is_some(), "no click chose the set");
    assert_eq!(shell.state.export.format, ExportFormat::Dxf);
    assert_eq!(shell.state.export.name, "untitled.dxf");
    // A sweep down the dialog's body: whatever a click there hits (a
    // label, a control, the margin), it never reaches the model under it.
    for y in (20..240).step_by(10) {
        if shell.export_dialog().is_none() {
            shell.open_export(&mut session).unwrap();
            for _ in 0..3 {
                step(&mut shell, &mut session, Vec::new());
            }
        }
        let dialog = ctx
            .memory(|m| m.area_rect(egui::Id::new("craft3d-export-dialog")))
            .unwrap();
        let at = dialog.left_top() + vec2(60.0, y as f32);
        if !dialog.contains(at) {
            continue;
        }
        for events in click(at) {
            step(&mut shell, &mut session, events);
        }
        let selection = session.execute("select.get", &json!({})).unwrap();
        assert_eq!(selection["items"], json!([]), "{at:?} {selection}");
    }
}

/// A document with named sets offers a sheet of each: with its width and
/// gap, remembered for the next export of that document (R1 A6).
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
    assert_eq!(export_choices(&mut session).sets, vec!["Cut sheet"]);
    shell.open_export(&mut session).unwrap();
    for _ in 0..2 {
        frame(&ctx, &mut shell, &mut session, Vec::new());
    }
    // The model is chosen until a set is; the set only goes to DXF or
    // SVG, so STEP gives way.
    assert_eq!(shell.state.export.what, ExportWhat::Model);
    let sheet = ExportWhat::Set("Cut sheet".to_string());
    shell.set_export_what(sheet.clone()).unwrap();
    assert_eq!(shell.state.export.format, ExportFormat::Dxf);
    assert_eq!(shell.state.export.name, "untitled.dxf");
    shell.state.export.width = "300 mm".to_string();
    shell.state.export.gap = "2 mm".to_string();
    shell.answer_export(true);
    let request = shell.take_export_answer().unwrap();
    assert_eq!(request.plan.what, sheet);
    assert_eq!(shell.export_sheet(), None, "remembered once written");
    assert_eq!(
        export_params(&mut session, &request.name, &request.plan).unwrap(),
        vec![
            ("set", json!("Cut sheet")),
            ("width", json!("300 mm")),
            ("gap", json!("2 mm")),
        ]
    );
    assert!(!write(&mut session, &request).is_empty());
    shell.export_written(&request);
    assert_eq!(
        shell.export_sheet(),
        Some((sheet.clone(), "300 mm".to_string(), "2 mm".to_string()))
    );
    // Offered again, drawn with its fields, and Enter exports it.
    shell.open_export(&mut session).unwrap();
    shell.set_export_format(ExportFormat::Svg).unwrap();
    for _ in 0..2 {
        frame(&ctx, &mut shell, &mut session, Vec::new());
    }
    assert_eq!(shell.state.export.what, sheet);
    assert_eq!(shell.state.export.width, "300 mm");
    frame(&ctx, &mut shell, &mut session, key(egui::Key::Enter));
    let request = shell.take_export_answer().unwrap();
    assert_eq!(request.name, "untitled.svg");
    assert_eq!(request.plan.what, sheet);
    // A document without that set: the model, no width.
    session.execute("file.new", &json!({})).unwrap();
    frame(&ctx, &mut shell, &mut session, Vec::new());
    shell.open_export(&mut session).unwrap();
    assert_eq!(shell.state.export.what, ExportWhat::Model);
    assert_eq!(shell.state.export.width, "");
    frame(&ctx, &mut shell, &mut session, Vec::new());
    frame(&ctx, &mut shell, &mut session, key(egui::Key::Enter));
    let request = shell.take_export_answer().unwrap();
    assert_eq!(request.plan.what, ExportWhat::Model);
    // Each document keeps its own: the first one's comes back with it.
    assert_eq!(shell.export_sheet(), None);
    session
        .execute("file.activate", &json!({"document": 0}))
        .unwrap();
    frame(&ctx, &mut shell, &mut session, Vec::new());
    assert_eq!(shell.export_sheet().map(|s| s.0), Some(sheet));
}

/// The export dialog offers a sheet-metal part's flat pattern: drawn with
/// the choice, answered with it, remembered for the document, and the
/// parameters `file.export {sheetMetal}` (repro-sheet-metal-enclosure-1).
#[test]
fn the_export_dialog_offers_a_flat_pattern() {
    let ctx = egui::Context::default();
    let mut session = Session::new(Workspace::memory());
    session
        .execute("file.new", &json!({"units": "mm"}))
        .unwrap();
    part(&mut session, "S", "Part", json!({}));
    let mut shell = Shell::new();
    frame(&ctx, &mut shell, &mut session, Vec::new());
    shell.open_export(&mut session).unwrap();
    for _ in 0..2 {
        frame(&ctx, &mut shell, &mut session, Vec::new());
    }
    let offered = shell.export_dialog().unwrap().choices.whats();
    let flat = ExportWhat::SheetMetal("Part".to_string());
    assert_eq!(offered, vec![ExportWhat::Model, flat.clone()]);
    assert_eq!(
        shell.state.export.what,
        ExportWhat::Model,
        "the model until a pattern is chosen"
    );
    shell.set_export_what(flat.clone()).unwrap();
    assert_eq!(shell.state.export.format, ExportFormat::Dxf);
    // A pattern is no file of bodies.
    assert!(shell.set_export_format(ExportFormat::Stl).is_err());
    shell.set_export_format(ExportFormat::Svg).unwrap();
    shell.answer_export(true);
    let request = shell.take_export_answer().unwrap();
    assert_eq!(
        export_params(&mut session, &request.name, &request.plan).unwrap(),
        vec![("sheetMetal", json!("Part"))]
    );
    let svg = write(&mut session, &request);
    assert!(String::from_utf8_lossy(&svg).contains("bendUp"));
    shell.export_written(&request);
    // Offered again with it chosen, and Enter exports it.
    assert_eq!(shell.export_sheet().map(|s| s.0), Some(flat.clone()));
    shell.open_export(&mut session).unwrap();
    for _ in 0..2 {
        frame(&ctx, &mut shell, &mut session, Vec::new());
    }
    frame(&ctx, &mut shell, &mut session, key(egui::Key::Enter));
    let request = shell.take_export_answer().unwrap();
    assert_eq!(request.name, "untitled.svg");
    assert_eq!(request.plan.what, flat);
}

/// A part switched off (`when` false) or failed has no flat pattern: the
/// dialog does not offer it, and "every part" is the parts that were
/// built, so the export goes through (review of r1-smgaps).
#[test]
fn a_part_switched_off_is_not_offered_and_every_part_is_the_built_ones() {
    let mut session = Session::new(Workspace::memory());
    session
        .execute("file.new", &json!({"units": "mm"}))
        .unwrap();
    session
        .execute("variable.set", &json!({"name": "lid", "expr": "0"}))
        .unwrap();
    for (sketch, name, extra) in [
        ("S1", "Box", json!({})),
        ("S2", "Lid", json!({"when": "lid"})),
        ("S3", "Tray", json!({})),
        ("S4", "Broken", json!({"kFactor": 1.5})),
    ] {
        part(&mut session, sketch, name, extra);
    }
    let choices = export_choices(&mut session);
    let labels: Vec<String> = choices.patterns.iter().map(|p| p.label()).collect();
    assert_eq!(
        labels,
        vec![
            "Flat pattern of Box",
            "Flat pattern of Tray",
            "Flat patterns of every part"
        ]
    );
    let plan = ExportPlan {
        what: ExportWhat::EveryPart,
        ..ExportPlan::default()
    };
    assert!(choices.offers(&plan.what));
    let params = export_params(&mut session, "parts.svg", &plan).unwrap();
    assert_eq!(params, vec![("sheetMetal", json!(["Box", "Tray"]))]);
    let mut call = json!({"path": "parts.svg"});
    for (key, value) in params {
        call[key] = value;
    }
    let written = session.execute("file.export", &call).unwrap();
    assert_eq!(written["sheetMetal"].as_array().map(Vec::len), Some(2));
    // The lid switched on: offered, and on the sheet.
    session
        .execute("variable.set", &json!({"name": "lid", "expr": "1"}))
        .unwrap();
    let choices = export_choices(&mut session);
    assert_eq!(choices.patterns.len(), 4, "{:?}", choices.patterns);
    assert_eq!(
        export_params(&mut session, "parts.svg", &plan).unwrap(),
        vec![("sheetMetal", json!(["Box", "Lid", "Tray"]))]
    );
}

/// Every kind of export the dialog offers, in every format that holds
/// it: the file is named for the document with the format's extension,
/// the parameters export it, and a name typed without the extension, or
/// with another format's, is named for the format chosen (2026-10-27).
#[test]
fn every_export_kind_is_named_for_its_format_and_exports() {
    let ctx = egui::Context::default();
    let mut session = Session::new(Workspace::memory());
    session
        .execute("file.new", &json!({"units": "mm"}))
        .unwrap();
    part(&mut session, "S1", "Box", json!({}));
    part(&mut session, "S2", "Lid", json!({}));
    session
        .execute(
            "feature.block",
            &json!({"size": [10, 10, 3], "position": [100, 0, 0]}),
        )
        .unwrap();
    session
        .execute("sketch.create", &json!({"name": "Paper"}))
        .unwrap();
    session
        .execute(
            "sketch.polyline",
            &json!({"sketch": "Paper", "points": [[200, 0], [240, 0], [240, 40], [200, 40]], "closed": true}),
        )
        .unwrap();
    let crease = session
        .execute(
            "sketch.line",
            &json!({"sketch": "Paper", "from": [200, 20], "to": [240, 20]}),
        )
        .unwrap();
    session
        .execute(
            "feature.fold",
            &json!({"sketch": "Paper", "name": "Crane", "creases": [
                {"line": crease["entity"], "kind": "valley", "angle": "90 deg"}]}),
        )
        .unwrap();
    session
        .execute(
            "set.define",
            &json!({"name": "Tops", "kind": "face", "where": {"normal": {"along": "z"}}}),
        )
        .unwrap();
    session
        .execute("file.save", &json!({"path": "enclosure.3dcraft"}))
        .unwrap();
    // One flat top face selected (of six that fit): the selection goes to
    // DXF and SVG.
    let top = session
        .execute(
            "select.query",
            &json!({"kind": "face", "where": {"normal": {"along": "z"}}, "limit": 1, "then": "replace"}),
        )
        .unwrap();
    assert_eq!(top["selection"]["count"], 1, "{top}");
    let mut shell = Shell::new();
    frame(&ctx, &mut shell, &mut session, Vec::new());
    shell.open_export(&mut session).unwrap();
    let whats = shell.export_dialog().unwrap().choices.whats();
    assert_eq!(
        whats,
        vec![
            ExportWhat::Model,
            ExportWhat::Selection,
            ExportWhat::Set("Tops".to_string()),
            ExportWhat::SheetMetal("Box".to_string()),
            ExportWhat::SheetMetal("Lid".to_string()),
            ExportWhat::Fold("Crane".to_string()),
            ExportWhat::EveryPart,
        ]
    );
    // The selection first: something is selected and a format holds it.
    assert_eq!(shell.state.export.what, ExportWhat::Selection);
    assert_eq!(shell.state.export.name, "enclosure.dxf");
    shell.answer_export(false);
    let mut exported = 0;
    for what in whats {
        let formats = export_choices(&mut session).formats(&what);
        let wanted = if what == ExportWhat::Model {
            ExportFormat::ALL.to_vec()
        } else {
            vec![ExportFormat::Dxf, ExportFormat::Svg]
        };
        assert_eq!(formats, wanted, "{what:?}");
        for format in formats {
            shell.open_export(&mut session).unwrap();
            shell.set_export_what(what.clone()).unwrap();
            shell.set_export_format(format).unwrap();
            // The model's drawing from a named view.
            shell.state.export.drawing.view = DrawingView::Iso;
            let file = format!("enclosure.{}", format.extension());
            assert_eq!(shell.state.export.name, file, "{what:?}");
            assert_eq!(shell.export_report()["file"], json!(file));
            frame(&ctx, &mut shell, &mut session, Vec::new());
            frame(&ctx, &mut shell, &mut session, key(egui::Key::Enter));
            let request = shell.take_export_answer().unwrap();
            assert_eq!(request.name, file);
            assert_eq!(request.format, format);
            assert_eq!(request.plan.what, what);
            assert!(!write(&mut session, &request).is_empty(), "{request:?}");
            exported += 1;
        }
    }
    assert_eq!(exported, 6 + 6 * 2);
    // Where the dialog names the file (the browser without file
    // dialogs): a name without an extension gets the format's, one with
    // another format's is corrected, any other extension is kept.
    shell.set_export_names_file(true);
    for (typed, format, named) in [
        ("Box final", ExportFormat::Svg, "Box final.svg"),
        ("plate.stl", ExportFormat::Dxf, "plate.dxf"),
        ("plate.DXF", ExportFormat::Dxf, "plate.DXF"),
        ("plate.v2", ExportFormat::Step, "plate.v2.step"),
        ("", ExportFormat::ThreeMf, "untitled.3mf"),
    ] {
        shell.open_export(&mut session).unwrap();
        shell.set_export_what(ExportWhat::Model).unwrap();
        shell.set_export_format(format).unwrap();
        shell.state.export.name = typed.to_string();
        frame(&ctx, &mut shell, &mut session, Vec::new());
        assert_eq!(shell.export_report()["namesFile"], true);
        shell.answer_export(true);
        let request = shell.take_export_answer().unwrap();
        assert_eq!(request.name, named, "{typed:?}");
        assert_eq!(ExportFormat::of_name(&request.name), Some(format));
    }
    // Choosing another format renames the file at once.
    shell.open_export(&mut session).unwrap();
    shell.set_export_what(ExportWhat::Model).unwrap();
    shell.state.export.name = "bracket.step".to_string();
    shell.set_export_format(ExportFormat::Obj).unwrap();
    assert_eq!(shell.state.export.name, "bracket.obj");
    shell
        .set_export_what(ExportWhat::Fold("Crane".to_string()))
        .unwrap();
    assert_eq!(shell.state.export.format, ExportFormat::Dxf);
    assert_eq!(shell.state.export.name, "bracket.dxf");
}

/// The control channel drives the export dialog through `ui.set
/// {"export": …}` and reads it with `ui.get`; what the document does not
/// offer is refused and nothing changes. Exporting stays the person's.
#[test]
fn the_export_dialog_is_driven_by_ui_set_and_read_by_ui_get() {
    let mut session = Session::new(Workspace::memory());
    let mut shell = Shell::new();
    fn call(
        shell: &mut Shell,
        session: &mut Session,
        method: &str,
        params: Value,
    ) -> Result<Value, String> {
        shell.handle_method(session, method, Some(&params)).unwrap()
    }
    // No document: refused, nothing changed (the camera neither).
    let before = shell.state.clone();
    let refused = call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"view": "top", "export": {"open": true}}),
    )
    .unwrap_err();
    assert!(
        refused.contains("ui.set: export: there is no document"),
        "{refused}"
    );
    assert_eq!(shell.state, before);
    session
        .execute("file.new", &json!({"units": "mm"}))
        .unwrap();
    part(&mut session, "S", "Part", json!({}));
    let got = call(&mut shell, &mut session, "ui.get", json!({})).unwrap();
    assert_eq!(got["export"], Value::Null);
    assert_eq!(got["state"]["export"]["open"], false);
    // Opened as the menu opens it.
    let set = call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"export": {"open": true}}),
    )
    .unwrap();
    assert_eq!(set["state"]["export"]["open"], true);
    assert_eq!(set["state"]["export"]["what"], "model");
    let got = call(&mut shell, &mut session, "ui.get", json!({})).unwrap();
    assert_eq!(
        got["export"]["whats"],
        json!([
            {"what": "model", "label": "The model",
             "formats": ["step", "stl", "3mf", "obj", "dxf", "svg"]},
            {"what": {"sheetMetal": "Part"}, "label": "Flat pattern of Part",
             "formats": ["dxf", "svg"]},
        ])
    );
    assert_eq!(got["export"]["file"], "untitled.step");
    // The flat pattern: the format follows; STL is refused for it.
    let set = call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"export": {"what": {"sheetMetal": "Part"}, "width": "600 mm"}}),
    )
    .unwrap();
    assert_eq!(set["state"]["export"]["format"], "dxf");
    assert_eq!(set["state"]["export"]["name"], "untitled.dxf");
    let before = shell.state.clone();
    for refused in [
        json!({"export": {"format": "stl"}}),
        json!({"export": {"what": {"set": "Nope"}}}),
        json!({"export": {"what": "selection"}}),
        json!({"export": {"format": "dwg"}}),
        json!({"export": {"what": {"sheetMetal": 3}}}),
        json!({"export": {"open": "yes"}}),
        json!({"export": {"press": true}}),
    ] {
        let error = call(&mut shell, &mut session, "ui.set", refused.clone()).unwrap_err();
        assert!(error.starts_with("ui.set"), "{refused}: {error}");
        assert_eq!(shell.state, before, "{refused}");
        assert!(shell.export_dialog().is_some(), "{refused}");
    }
    // The options, then closed as Cancel closes it: nothing exported.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"export": {"format": "svg", "quality": "coarse", "drawing": {"dashed": true}}}),
    )
    .unwrap();
    assert_eq!(shell.state.export.format, ExportFormat::Svg);
    assert_eq!(shell.state.export.quality, ExportQuality::Coarse);
    assert!(shell.state.export.drawing.dashed);
    assert_eq!(shell.state.export.width, "600 mm");
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"export": {"open": false}}),
    )
    .unwrap();
    assert!(shell.export_dialog().is_none() && !shell.state.export.open);
    assert_eq!(shell.take_export_answer(), None);
    assert_eq!(shell.last_command.clone().unwrap().text, "cancelled");
    // Cancelled: what it had chosen is not kept.
    assert_eq!(shell.state.export.quality, ExportQuality::Normal);
    assert_eq!(shell.state.export.width, "");
    // Chosen while it is closed: the next opening honours it.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"export": {"what": {"sheetMetal": "Part"}, "width": "500 mm", "name": "lid"}}),
    )
    .unwrap();
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"export": {"open": true}}),
    )
    .unwrap();
    assert_eq!(
        shell.state.export.what,
        ExportWhat::SheetMetal("Part".to_string())
    );
    assert_eq!(shell.state.export.width, "500 mm");
    assert_eq!(shell.state.export.format, ExportFormat::Dxf);
    assert_eq!(shell.state.export.name, "lid.dxf");
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"export": {"open": false}}),
    )
    .unwrap();
    // A request whose export part is refused changes nothing else: the
    // feature it names is not selected.
    let error = call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"selectedFeature": "F2", "export": {"open": true, "what": {"set": "Nope"}}}),
    )
    .unwrap_err();
    assert!(error.contains("not offered"), "{error}");
    assert!(shell.export_dialog().is_none() && !shell.state.export.open);
    let selection = session.execute("select.get", &json!({})).unwrap();
    assert_eq!(selection["items"], json!([]), "{selection}");
    // Opened with its choices in one request; the open dialog takes the
    // keyboard as before.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"export": {"open": true, "what": "model", "format": "3mf", "quality": "coarse"}}),
    )
    .unwrap();
    assert_eq!(shell.state.export.format, ExportFormat::ThreeMf);
    assert_eq!(shell.state.export.name, "untitled.3mf");
    let ctx = egui::Context::default();
    frame(&ctx, &mut shell, &mut session, Vec::new());
    frame(&ctx, &mut shell, &mut session, key(egui::Key::Enter));
    let request = shell.take_export_answer().unwrap();
    assert_eq!(request.name, "untitled.3mf");
    assert_eq!(request.plan.quality, ExportQuality::Coarse);
}
