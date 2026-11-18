//! What a click would select lights up under the pointer (the UX round of
//! 2026-11-18, V7): `ui.pick {x, y, hover: true}` places the pointer
//! without a click, and `ui.get` › `hovered` says what lies there, shaped
//! as `state.picked`: the same face, edge or vertex `ui.pick` gives at
//! that position.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_math::Vec3;
use craft3d_ui_egui::camera::Projector;
use craft3d_ui_egui::{RendererChoice, Shell};
use serde_json::{Value, json};

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

fn refused(shell: &mut Shell, session: &mut Session, method: &str, params: Value) -> String {
    match shell
        .handle_method(session, method, Some(&params))
        .unwrap_or_else(|| panic!("{method} is not a ui method"))
    {
        Ok(value) => panic!("{method} {params} should fail, returned {value}"),
        Err(e) => e,
    }
}

/// A plate 60 × 40 × 8 seen from the iso view.
fn plate() -> (Session, Shell) {
    let mut session = Session::new(Workspace::none());
    run(&mut session, "file.new", json!({"units": "mm"}));
    run(
        &mut session,
        "feature.block",
        json!({"name": "Plate", "size": [60, 40, 8]}),
    );
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    call(&mut shell, &mut session, "ui.fit", json!({"view": "iso"}));
    (session, shell)
}

/// The pixel of the viewport `p` is seen at.
fn pixel_of(shell: &Shell, p: Vec3) -> [f64; 2] {
    let [width, height] = shell.viewport_size();
    let projector = Projector::new(
        &shell.state.camera.to_render(),
        f64::from(width),
        f64::from(height),
    )
    .unwrap();
    let at = projector.project(p).unwrap();
    [at.x, at.y]
}

/// The first of `points` where `ui.pick` gives `kind`, and its answer.
fn where_kind(
    shell: &mut Shell,
    session: &mut Session,
    points: &[Vec3],
    kind: &str,
) -> ([f64; 2], Value) {
    for &p in points {
        let [x, y] = pixel_of(shell, p);
        let picked = call(shell, session, "ui.pick", json!({"x": x, "y": y}));
        if picked["kind"] == kind {
            return ([x, y], picked);
        }
    }
    panic!("no {kind} found at {points:?}");
}

/// `hovered` names what `picked` (a `ui.pick` answer) names.
fn same_thing(hovered: &Value, picked: &Value, context: &str) {
    for key in ["kind", "feature", "name", "face", "faceIndex"] {
        assert_eq!(hovered[key], picked[key], "{context}: {key}");
    }
    for entity in ["edge", "vertex"] {
        for key in ["feature", "body", "name", "index", "faces"] {
            assert_eq!(
                hovered[entity][key], picked[entity][key],
                "{context}: {entity}.{key}"
            );
        }
    }
}

#[test]
fn hovered_is_what_ui_pick_gives_at_the_same_position() {
    let (mut session, mut shell) = plate();
    let get = |shell: &mut Shell, session: &mut Session| {
        call(shell, session, "ui.get", json!({}))["hovered"].clone()
    };
    assert_eq!(get(&mut shell, &mut session), Value::Null, "nothing yet");

    let v = |x, y, z| Vec3::new(x, y, z);
    let corners = [
        v(0.0, 0.0, 8.0),
        v(60.0, 0.0, 8.0),
        v(60.0, 40.0, 8.0),
        v(0.0, 40.0, 8.0),
    ];
    let middles = [
        v(30.0, 0.0, 8.0),
        v(60.0, 20.0, 8.0),
        v(30.0, 40.0, 8.0),
        v(0.0, 20.0, 8.0),
    ];
    let face = where_kind(&mut shell, &mut session, &[v(30.0, 20.0, 8.0)], "face");
    let edge = where_kind(&mut shell, &mut session, &middles, "edge");
    let vertex = where_kind(&mut shell, &mut session, &corners, "vertex");
    let mut serial = 0;
    for (what, ([x, y], picked)) in [("face", face), ("edge", edge), ("vertex", vertex)] {
        let placed = call(
            &mut shell,
            &mut session,
            "ui.pick",
            json!({"x": x, "y": y, "hover": true}),
        );
        let context = format!("the {what} at ({x}, {y}): {placed}");
        // The answer is the pick's, with `hovered` beside it.
        assert_eq!(placed["kind"], what, "{context}");
        same_thing(&placed["hovered"], &picked, &context);
        let hovered = get(&mut shell, &mut session);
        assert_eq!(hovered, placed["hovered"], "{context}");
        serial += 1;
        assert_eq!(hovered["serial"], serial, "{context}");
        assert_eq!(hovered["x"], x, "{context}");
        assert_eq!(hovered["y"], y, "{context}");
        // Placing the pointer is not a click: nothing is picked,
        // highlighted or selected.
        assert_eq!(shell.state.picked, None, "{context}");
        assert_eq!(shell.state.highlight, None, "{context}");
        assert_eq!(shell.state.highlight_edge, None, "{context}");
        assert_eq!(shell.state.highlight_vertex, None, "{context}");
        let selection = call(&mut shell, &mut session, "ui.get", json!({}))["selection"].clone();
        assert_eq!(selection["count"], 0, "{context}: {selection}");
    }

    // A change of the view state keeps it; it is not in the state's JSON
    // and cannot be set.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"notice": "kept"}),
    );
    assert_eq!(get(&mut shell, &mut session)["kind"], "vertex");
    let state = call(&mut shell, &mut session, "ui.get", json!({}))["state"].clone();
    assert!(state.get("hovered").is_none(), "{state}");
    let e = refused(&mut shell, &mut session, "ui.set", json!({"hovered": null}));
    assert!(e.contains("hovered"), "{e}");

    // Outside the model: nothing.
    let placed = call(
        &mut shell,
        &mut session,
        "ui.pick",
        json!({"x": 1.5, "y": 1.5, "hover": true}),
    );
    assert_eq!(placed["hit"], false, "{placed}");
    assert_eq!(placed["hovered"], Value::Null, "{placed}");
    assert_eq!(get(&mut shell, &mut session), Value::Null);
}

#[test]
fn hover_on_a_picture_of_another_size_is_the_viewports_place() {
    let (mut session, mut shell) = plate();
    let [width, height] = shell.viewport_size();
    let [x, y] = pixel_of(&shell, Vec3::new(30.0, 20.0, 8.0));
    let placed = call(
        &mut shell,
        &mut session,
        "ui.pick",
        json!({"x": x * 2.0, "y": y * 2.0, "width": width * 2, "height": height * 2,
               "hover": true}),
    );
    assert_eq!(placed["kind"], "face", "{placed}");
    let hovered = &placed["hovered"];
    // In the viewport's own pixels, as a click is.
    assert!(
        (hovered["x"].as_f64().unwrap() - x).abs() < 1e-9,
        "{hovered}"
    );
    assert!(
        (hovered["y"].as_f64().unwrap() - y).abs() < 1e-9,
        "{hovered}"
    );
}

#[test]
fn hover_and_select_are_not_given_together() {
    let (mut session, mut shell) = plate();
    for params in [
        json!({"x": 10, "y": 10, "hover": true, "select": true}),
        json!({"x": 10, "y": 10, "hover": 1}),
        json!({"x": 10, "y": 10, "hover": true, "shift": true}),
    ] {
        let e = refused(&mut shell, &mut session, "ui.pick", params.clone());
        assert!(e.starts_with("ui.pick"), "{params}: {e}");
    }
    assert_eq!(shell.state.hovered, None);
}
