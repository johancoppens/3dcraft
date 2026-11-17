//! The grid of a grid surface in the window: drawn with the sketches and
//! switched with them (`ui.set {"show": {"sketches": false}}`, the
//! Sketches toggle), said by `ui.get` (`scene.grids`), and tested against
//! the depth of the bodies a lift nearer than it is
//! (`SceneSketch::lift`), by the CPU overlay and in the GPU's segments
//! alike: its lines and points lie on the sheet's exact surface, which
//! the mesh follows only within the scene's tolerance, so without the
//! lift a grid on a hollow sheet would be drawn dimmed where it should
//! show.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{SURFACE_LIFT, Scene, SceneQuality, Session, Workspace};
use craft3d_math::Portable;
use craft3d_ui_egui::state::CameraState;
use craft3d_ui_egui::viewport::{self, LineKind, Marks, Quality};
use craft3d_ui_egui::{Shell, Tokens};
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

/// A bowl of 7 × 6 heights from a table, 10 mm apart: a sheet hollow
/// toward the eye above it, where the mesh's chords lie above the exact
/// surface and a line on that surface below them.
fn bowl() -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    let rows: Vec<Value> = (0..6)
        .map(|r| {
            let mut row = vec![json!(r + 1)];
            for c in 0..7 {
                let (x, y) = (c as f64 - 3.0, r as f64 - 2.5);
                row.push(json!(0.9 * (x * x + y * y)));
            }
            Value::Array(row)
        })
        .collect();
    run(
        &mut s,
        "table.set",
        json!({"name": "bowl", "columns": ["row", "a", "b", "c", "d", "e", "f", "g"], "rows": rows}),
    );
    run(
        &mut s,
        "feature.surface.grid",
        json!({"table": "bowl", "step": ["10 mm", "10 mm"], "factor": "1 mm", "name": "Bowl"}),
    );
    s
}

fn scene(session: &mut Session) -> Arc<Scene> {
    session
        .scene_with(SceneQuality::display())
        .unwrap()
        .unwrap()
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
fn the_grid_is_switched_with_the_sketches_and_ui_get_says_it() {
    let mut session = bowl();
    let mut shell = Shell::new();
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(
        got["scene"]["grids"],
        json!([{"feature": "F1", "name": "Bowl", "rows": 6, "columns": 7, "every": [1, 1], "thinned": false}]),
        "{}",
        got["scene"]
    );
    assert_eq!(got["scene"]["sketches"], 1);
    let shown = screenshot(&mut shell, &mut session);
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"show": {"sketches": false}}),
    );
    let hidden = screenshot(&mut shell, &mut session);
    let coverage = |shot: &Value| shot["overlayCoverage"].as_f64().unwrap();
    assert!(
        coverage(&shown) > coverage(&hidden),
        "{shown} against {hidden}"
    );
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"show": {"sketches": true}}),
    );
    let again = screenshot(&mut shell, &mut session);
    assert_eq!(shown["base64"], again["base64"], "bit for bit");
    // Hidden with its feature, which the document keeps.
    run(
        &mut session,
        "feature.visible",
        json!({"feature": "Bowl", "visible": false}),
    );
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["scene"]["grids"], json!([]));
}

/// From above at a slant, at the display's quality.
fn above(scene: &Scene, size: [u32; 2]) -> CameraState {
    let mut camera = CameraState {
        yaw: 30.0,
        pitch: 55.0,
        ..CameraState::default()
    };
    camera.fit(&scene.bbox, f64::from(size[0]) / f64::from(size[1]));
    camera
}

#[test]
fn the_grids_lines_and_points_lie_on_the_sheet_for_both_paths() {
    let mut session = bowl();
    let scene = scene(&mut session);
    let size = [480, 360];
    let camera = above(&scene, size);
    let quality = Quality {
        width: size[0],
        height: size[1],
        ssaa: 1,
    };
    let frame =
        viewport::render_bodies(Some(&scene), &camera, quality, &Tokens::default()).unwrap();
    // The CPU overlay: every point of the grid in sight, and its lines too
    // (but where a line runs off the sheet's border, past the picture's
    // last sample of it).
    let overlay = viewport::overlay_on(Some(&scene), &camera, &frame, None);
    let points: Vec<_> = overlay
        .points
        .iter()
        .filter(|p| p.kind == LineKind::Sketch)
        .collect();
    assert_eq!(points.len(), 42);
    assert!(points.iter().all(|p| !p.hidden), "{points:?}");
    // The lines are drawn as construction geometry (a guide, not a
    // profile), the points as a sketch's points.
    assert!(!overlay.lines.iter().any(|l| l.kind == LineKind::Sketch));
    let length = |hidden: bool| -> f64 {
        overlay
            .lines
            .iter()
            .filter(|l| l.kind == LineKind::Construction && l.hidden == hidden)
            .map(|l| (l.to[0] - l.from[0]).phypot(l.to[1] - l.from[1]))
            .sum()
    };
    let (seen, behind) = (length(false), length(true));
    assert!(seen > 1000.0, "{seen} pixels of the grid's lines");
    assert!(behind <= 1e-3 * seen, "{behind} of {seen} pixels dimmed");
    // Without the lift (the same lines as a sketch's), the hollow sheet's
    // mesh lies above them: much of the grid would be dimmed.
    let mut unlifted = (*scene).clone();
    for sketch in &mut unlifted.sketches {
        sketch.grid = None;
    }
    let overlay = viewport::overlay_on(Some(&unlifted), &camera, &frame, None);
    let dimmed = overlay
        .points
        .iter()
        .filter(|p| p.kind == LineKind::Sketch && p.hidden)
        .count();
    assert!(dimmed > 0, "without the lift no point was dimmed");
    // The GPU's segments carry the same lift, the bodies' lines none.
    let tokens = Tokens::default();
    let segments = craft3d_ui_egui::direct::segments_marked(
        Some(&scene),
        None,
        &tokens,
        1.0,
        &Marks::default(),
        false,
    );
    let lift = (SURFACE_LIFT * scene.tolerance) as f32;
    let sketchy: Vec<_> = segments
        .iter()
        .filter(|s| {
            s.color == LineKind::Sketch.color(&tokens)
                || s.color == LineKind::Construction.color(&tokens)
        })
        .collect();
    // The rows and columns (from the knots: one chord a span at least)
    // and the 42 points.
    assert!(sketchy.len() > 42 + 6 * 4 + 7 * 3, "{}", sketchy.len());
    assert!(sketchy.iter().all(|s| s.lift == lift), "{lift}");
    assert!(
        segments
            .iter()
            .filter(|s| s.edge.is_some())
            .all(|s| s.lift == 0.0)
    );
}
