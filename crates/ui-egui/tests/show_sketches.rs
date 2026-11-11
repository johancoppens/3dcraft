//! `state.show.sketches`: the sketches drawn in the viewport or not, set
//! with `ui.set {"show": {"sketches": false}}` and by the Sketches toggle
//! over the viewport, honoured by the CPU overlay, the GPU's lines and
//! `ui.screenshot`. With the sketches shown the picture is what it was
//! before the toggle existed, bit for bit.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Scene, SceneQuality, Session, Workspace};
use craft3d_ui_egui::state::{CameraState, Show};
use craft3d_ui_egui::viewport::{self, LineKind, Marks};
use craft3d_ui_egui::{Shell, Tokens};
use egui::{Event, PointerButton, Pos2, vec2};
use serde_json::{Value, json};
use std::sync::Arc;

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn call(shell: &mut Shell, session: &mut Session, method: &str, params: Value) -> Value {
    shell
        .handle_method(session, method, Some(&params))
        .unwrap_or_else(|| panic!("{method} is not a ui method"))
        .unwrap_or_else(|e| panic!("{method} {params}: {e}"))
}

/// A plate with a sketch beside it (a triangle of lines on a plane above
/// the plate) and no datums.
fn plate_and_sketch() -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": [60, 40, 8]}),
    );
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "offset": "12 mm", "name": "Outline"}),
    );
    run(
        &mut s,
        "sketch.line",
        json!({"sketch": "Outline", "from": [5, 5], "to": [50, 8]}),
    );
    run(
        &mut s,
        "sketch.line",
        json!({"sketch": "Outline", "from": [50, 8], "to": [20, 35]}),
    );
    s
}

fn screenshot(shell: &mut Shell, session: &mut Session) -> Value {
    call(
        shell,
        session,
        "ui.screenshot",
        json!({"width": 320, "height": 240, "scale": 1.0}),
    )
}

#[test]
fn ui_set_hides_and_shows_the_sketches_and_shown_is_the_picture_of_before() {
    let mut session = plate_and_sketch();
    let mut shell = Shell::new();
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(
        got["state"]["show"],
        json!({"datums": false, "sketches": true, "edges": true, "mates": true})
    );
    let before = screenshot(&mut shell, &mut session);
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"show": {"sketches": false}}),
    );
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(
        got["state"]["show"],
        json!({"datums": false, "sketches": false, "edges": true, "mates": true})
    );
    let hidden = screenshot(&mut shell, &mut session);
    let coverage = |shot: &Value| shot["overlayCoverage"].as_f64().unwrap();
    assert!(
        coverage(&before) > coverage(&hidden),
        "{before} against {hidden}"
    );
    assert_ne!(before["base64"], hidden["base64"]);
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"show": {"sketches": true}}),
    );
    let again = screenshot(&mut shell, &mut session);
    assert_eq!(before["base64"], again["base64"], "bit for bit");
    // The datums' field stays as it was set, and a wrong type is refused.
    let error = shell
        .handle_method(
            &mut session,
            "ui.set",
            Some(&json!({"show": {"sketches": "no"}})),
        )
        .unwrap()
        .unwrap_err();
    assert!(error.contains("ui.set"), "{error}");
}

fn scene(session: &mut Session) -> Arc<Scene> {
    session
        .scene_with(SceneQuality::display())
        .unwrap()
        .unwrap()
}

/// The two drawing paths leave the sketches out when the marks say so,
/// and draw what they drew before otherwise.
#[test]
fn both_drawing_paths_honour_the_toggle() {
    let mut session = plate_and_sketch();
    let scene = scene(&mut session);
    let tokens = Tokens::default();
    let sketch_color = LineKind::Sketch.color(&tokens);
    let shown = Marks::default();
    let hidden = Marks {
        hide_sketches: true,
        ..Marks::default()
    };
    // The GPU's lines.
    let plain = craft3d_ui_egui::direct::segments(Some(&scene), None, &tokens, 1.0);
    let with =
        craft3d_ui_egui::direct::segments_marked(Some(&scene), None, &tokens, 1.0, &shown, false);
    let without =
        craft3d_ui_egui::direct::segments_marked(Some(&scene), None, &tokens, 1.0, &hidden, false);
    assert_eq!(plain, with);
    assert!(with.iter().any(|s| s.color == sketch_color));
    assert!(!without.iter().any(|s| s.color == sketch_color));
    assert!(without.len() < with.len());
    // The CPU overlay, untested against depth.
    let camera = {
        let mut camera = CameraState::default();
        camera.fit(&scene.bbox, 4.0 / 3.0);
        camera
    };
    let lines =
        |marks: &Marks| viewport::overlay_untested(Some(&scene), &camera, 400, 300, None, marks);
    let drawn = lines(&shown);
    let left = lines(&hidden);
    let sketchy = |overlay: &viewport::Overlay| {
        overlay
            .lines
            .iter()
            .filter(|l| l.kind == LineKind::Sketch)
            .count()
            + overlay
                .points
                .iter()
                .filter(|p| p.kind == LineKind::Sketch)
                .count()
    };
    assert!(sketchy(&drawn) > 0);
    assert_eq!(sketchy(&left), 0);
}

/// The Sketches toggle over the viewport flips the same field.
#[test]
fn the_sketches_toggle_in_the_window_flips_the_state() {
    let mut session = plate_and_sketch();
    let mut shell = Shell::new();
    let ctx = egui::Context::default();
    let mut time = 0.0;
    let mut frame = |shell: &mut Shell, session: &mut Session, events: Vec<Event>| {
        time += 1.0 / 60.0;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1024.0, 700.0))),
            time: Some(time),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| shell.ui(ui, session));
        output.textures_delta.clear();
    };
    frame(&mut shell, &mut session, Vec::new());
    let before = shell.state.show;
    let names: Vec<&str> = shell.show_hits().iter().map(|(name, _)| *name).collect();
    assert_eq!(names, ["sketches", "datums", "edges"]);
    let at = shell.show_hits()[0].1.center();
    let press = |pressed| Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    for events in [
        vec![Event::PointerMoved(at)],
        vec![press(true)],
        vec![press(false)],
        Vec::new(),
    ] {
        frame(&mut shell, &mut session, events);
    }
    assert!(!shell.state.show.sketches);
    assert_eq!(
        shell.state.show,
        Show {
            sketches: false,
            ..before
        },
        "only the sketches"
    );
    // The toggle is over the viewport: the click picked nothing.
    assert!(shell.state.picked.is_none());
}
