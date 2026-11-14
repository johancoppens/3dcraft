//! The presentations (Embed, 2026-11-14; `UiState::presentation`): how
//! much of the window the shell draws, set as state by the page that
//! embeds the web build. `full` is the window as it was; `view` is the 3D
//! view alone, over the whole window, with its view cube and controls;
//! `form` puts a Jig's form beside the view (a half sheet under it in a
//! narrow window), without the menu, the tabs and the status bar. Drawn
//! in real frames, read back from where the shell laid things out.

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::Shell;
use craft3d_ui_egui::state::Presentation;
use egui::{Pos2, Rect, vec2};
use serde_json::{Value, json};

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// A plate with a public width, published and opened as a Jig in use.
fn jig() -> Session {
    let mut s = Session::new(Workspace::memory());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "variable.set",
        json!({"name": "width", "expr": "60 mm"}),
    );
    run(
        &mut s,
        "variable.publish",
        json!({"name": "width", "min": "20 mm", "max": "200 mm"}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": ["width", 40, 30]}),
    );
    run(&mut s, "jig.export", json!({"path": "plate.3dcraftjig"}));
    run(&mut s, "file.close", json!({"discard": true}));
    run(&mut s, "file.open", json!({"path": "plate.3dcraftjig"}));
    assert!(s.document_status().unwrap().jig_in_use);
    s
}

/// Draws `frames` frames of a window of `width` × `height` points.
fn draw(shell: &mut Shell, session: &mut Session, width: f32, height: f32, frames: usize) {
    let ctx = egui::Context::default();
    for frame in 0..frames {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, height))),
            time: Some(frame as f64 / 60.0),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| shell.ui(ui, session));
        output.textures_delta.clear();
    }
}

fn shown(shell: &Shell) -> Vec<String> {
    shell
        .parameter_hits()
        .into_iter()
        .map(|(name, _)| name)
        .collect()
}

#[test]
fn view_is_the_3d_view_alone_and_form_the_form_beside_it() {
    let (width, height) = (1280.0, 800.0);
    // The whole window: menu, tabs, status bar, the side panel with the
    // Jig's form and Open for editing.
    let mut session = jig();
    let mut shell = Shell::new();
    draw(&mut shell, &mut session, width, height, 4);
    let full = shell.viewport_rect().unwrap();
    assert!(full.top() > 0.0 && full.bottom() < height, "{full:?}");
    assert!(full.left() > 0.0, "the side panel: {full:?}");
    assert!(!shell.status_hits().is_empty());
    assert!(shown(&shell).iter().any(|h| h == "Open for editing"));

    // The view alone, over the whole window: no menu, no tabs, no side
    // panel, no status bar; the view cube and the controls stay.
    shell.state.presentation = Presentation::View;
    draw(&mut shell, &mut session, width, height, 4);
    let view = shell.viewport_rect().unwrap();
    assert_eq!(view, Rect::from_min_size(Pos2::ZERO, vec2(width, height)));
    assert!(shell.status_hits().is_empty());
    assert!(shell.tab_hits().is_empty());
    assert!(shell.tree_panel_rect().is_none());
    assert!(shown(&shell).is_empty(), "no form: {:?}", shown(&shell));
    let got = shell
        .handle_method(&mut session, "ui.get", None)
        .unwrap()
        .unwrap();
    assert_eq!(got["state"]["presentation"], "view");
    assert!(got["scene"]["bodies"].as_u64().unwrap() >= 1);

    // The form beside the view, the whole height: the Jig's fields, not
    // Open for editing (the page decides what is opened).
    shell
        .handle_method(
            &mut session,
            "ui.set",
            Some(&json!({"presentation": "form"})),
        )
        .unwrap()
        .unwrap();
    draw(&mut shell, &mut session, width, height, 4);
    let form = shell.viewport_rect().unwrap();
    let panel = shell.tree_panel_rect().unwrap();
    assert_eq!(form.top(), 0.0, "no menu, no tabs: {form:?}");
    assert_eq!(form.bottom(), height, "no status bar: {form:?}");
    assert!(
        (form.left() - panel.right()).abs() < 1.0,
        "{form:?} {panel:?}"
    );
    assert!(shell.status_hits().is_empty());
    assert!(shell.tab_hits().is_empty());
    let fields = shown(&shell);
    assert!(fields.iter().any(|h| h.contains("width")), "{fields:?}");
    assert!(
        !fields.iter().any(|h| h == "Open for editing"),
        "{fields:?}"
    );

    // In a narrow window the form is a sheet under the view.
    draw(&mut shell, &mut session, 390.0, 844.0, 4);
    assert!(shell.parameters_sheet_rect().is_some());
    assert!(shell.drawer_button_rect().is_none(), "no menu bar");
    shell.state.presentation = Presentation::View;
    draw(&mut shell, &mut session, 390.0, 844.0, 4);
    assert!(shell.parameters_sheet_rect().is_none());

    // And back: the window as before.
    shell.state.presentation = Presentation::Full;
    draw(&mut shell, &mut session, width, height, 4);
    assert_eq!(shell.viewport_rect().unwrap(), full);
    assert!(!shell.status_hits().is_empty());
}

#[test]
fn without_the_whole_window_there_is_no_start_screen() {
    let mut session = Session::new(Workspace::none());
    let mut shell = Shell::new();
    draw(&mut shell, &mut session, 1280.0, 800.0, 3);
    assert!(shell.start_shown());
    for presentation in [Presentation::View, Presentation::Form] {
        shell.state.presentation = presentation;
        draw(&mut shell, &mut session, 1280.0, 800.0, 3);
        assert!(!shell.start_shown(), "{presentation:?}");
        let got = shell
            .handle_method(&mut session, "ui.get", None)
            .unwrap()
            .unwrap();
        assert_eq!(got["startScreen"], false);
    }
}
