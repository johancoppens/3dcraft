//! Datums in the viewport: a plane as a translucent square with its
//! outline, an axis as a line, a point as a dot, in the theme's datum
//! colour, tested against the bodies like a sketch; `state.show.datums`
//! hides them; with none shown the picture is the one without datums, bit
//! for bit. The GPU's picture with datums is compared with this one in
//! `gpu_viewport.rs`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{
    DATUM_AXIS_EXTENT, DATUM_PLANE_EXTENT, SceneDatumShape, SceneQuality, Session, Workspace,
};
use craft3d_render::Highlight;
use craft3d_ui_egui::state::{CameraState, ProjectionKind};
use craft3d_ui_egui::viewport::{self, LineKind, Marks, Quality};
use craft3d_ui_egui::{Shell, Tokens};
use serde_json::{Value, json};
use std::path::PathBuf;

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

/// A plate with a post.
fn plate() -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": [60, 40, 8]}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Post", "position": [15, 20, 8], "radius": 6, "height": 30}),
    );
    s
}

/// The plate with a datum plane through the post, an axis along the
/// plate's long side and a point above it.
fn with_datums() -> Session {
    let mut s = plate();
    run(
        &mut s,
        "datum.plane",
        json!({"from": "xy", "distance": 20, "name": "Shelf"}),
    );
    run(
        &mut s,
        "datum.axis",
        json!({"through": [[0, 0, 0], [60, 0, 0]], "name": "Hinge"}),
    );
    run(
        &mut s,
        "datum.point",
        json!({"at": [45, 30, 25], "name": "Mark"}),
    );
    s
}

fn evidence(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/ui");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn camera(session: &mut Session, view: &str, projection: ProjectionKind) -> CameraState {
    let scene = session
        .scene_with(SceneQuality::Automatic)
        .unwrap()
        .unwrap();
    let mut camera = CameraState {
        projection,
        ..CameraState::default()
    };
    camera.set_view(view).unwrap();
    camera.fit(&scene.bbox, 640.0 / 480.0);
    camera
}

fn picture(session: &mut Session, camera: &CameraState, shown: bool, tokens: &Tokens) -> Vec<u8> {
    let scene = session
        .scene_with(SceneQuality::Automatic)
        .unwrap()
        .unwrap();
    viewport::picture_marked(
        Some(&scene),
        camera,
        Quality {
            width: 640,
            height: 480,
            ssaa: 2,
        },
        tokens,
        None,
        Highlight::NONE,
        &viewport::datum_marks(Some(&scene), shown),
        1.0,
        1.0,
    )
    .unwrap()
    .rgb
}

#[test]
fn the_scene_places_and_sizes_the_datums_that_are_built() {
    let mut s = with_datums();
    let scene = s.scene_with(SceneQuality::Automatic).unwrap().unwrap();
    let names: Vec<&str> = scene.datums.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["Shelf", "Hinge", "Mark"]);
    let size = scene.bbox.diagonal() * DATUM_PLANE_EXTENT;
    let reach = scene.bbox.diagonal() * DATUM_AXIS_EXTENT;
    let centre = scene.bbox.center().unwrap();
    match scene.datums[0].shape {
        SceneDatumShape::Plane { center, u, v } => {
            // The foot of the model's centre on the plane z = 20.
            assert!((center.z - 20.0).abs() < 1e-9);
            assert!((center.x - centre.x).abs() < 1e-9 && (center.y - centre.y).abs() < 1e-9);
            assert!((u.length() - size).abs() < 1e-9 && (v.length() - size).abs() < 1e-9);
            assert!(u.dot(v).abs() < 1e-9 && u.z.abs() < 1e-12 && v.z.abs() < 1e-12);
        }
        other => panic!("{other:?}"),
    }
    match scene.datums[1].shape {
        SceneDatumShape::Axis { from, to } => {
            assert!(from.y.abs() < 1e-9 && from.z.abs() < 1e-9);
            assert!(((to - from).length() - 2.0 * reach).abs() < 1e-9);
            assert!(((from.x + to.x) / 2.0 - centre.x).abs() < 1e-9);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        scene.datums[2].shape,
        SceneDatumShape::Point(craft3d_math::Vec3::new(45.0, 30.0, 25.0))
    );
    // The datums are not in the box the camera frames: a document's
    // pictures do not move when a datum is added.
    let mut bare = plate();
    let without = bare.scene_with(SceneQuality::Automatic).unwrap().unwrap();
    assert_eq!(scene.bbox, without.bbox);

    // Suppressed, rolled back or failed, a datum has no place and is not
    // drawn.
    run(&mut s, "feature.suppress", json!({"feature": "Hinge"}));
    run(&mut s, "history.rollback", json!({"feature": "Shelf"}));
    let scene = s.scene_with(SceneQuality::Automatic).unwrap().unwrap();
    let names: Vec<&str> = scene.datums.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["Shelf"]);
    run(&mut s, "history.rollback", json!({"feature": null}));
    run(
        &mut s,
        "datum.plane",
        json!({"from": "F1:face(99)", "distance": 1, "name": "Lost"}),
    );
    let scene = s.scene_with(SceneQuality::Automatic).unwrap().unwrap();
    let names: Vec<&str> = scene.datums.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["Shelf", "Mark"]);
}

#[test]
fn hidden_or_absent_datums_leave_the_picture_bit_for_bit() {
    let tokens = Tokens::default();
    let mut bare = plate();
    let mut marked = with_datums();
    for (view, projection) in [
        ("iso", ProjectionKind::Perspective),
        ("front", ProjectionKind::Orthographic),
    ] {
        let camera = camera(&mut bare, view, projection);
        let without = picture(&mut bare, &camera, true, &tokens);
        assert_eq!(picture(&mut marked, &camera, false, &tokens), without);
        let shown = picture(&mut marked, &camera, true, &tokens);
        assert_ne!(shown, without, "{view}: the datums show");
    }
}

#[test]
fn the_overlay_draws_the_outline_the_axis_and_the_point() {
    let mut s = with_datums();
    let camera = camera(&mut s, "iso", ProjectionKind::Perspective);
    let scene = s.scene_with(SceneQuality::Automatic).unwrap().unwrap();
    let marks = viewport::datum_marks(Some(&scene), true);
    assert_eq!(marks.datum_lines().len(), 5, "four sides and an axis");
    assert_eq!(marks.datum_points().len(), 1);
    let overlay = viewport::overlay_untested(Some(&scene), &camera, 640, 480, None, &marks);
    let datum_lines = overlay
        .lines
        .iter()
        .filter(|l| l.kind == LineKind::Datum)
        .count();
    assert_eq!(datum_lines, 5);
    assert_eq!(
        overlay
            .points
            .iter()
            .filter(|p| p.kind == LineKind::Datum)
            .count(),
        1
    );
    // Nothing of it without the datums.
    let none = viewport::overlay_untested(Some(&scene), &camera, 640, 480, None, &Marks::default());
    assert!(none.lines.iter().all(|l| l.kind != LineKind::Datum));
    assert_eq!(none.lines.len() + 5, overlay.lines.len());
    // Labels: one per datum, where the anchor projects.
    let labels = viewport::datum_labels(&marks, &camera, 640, 480);
    let names: Vec<&str> = labels.iter().map(|(_, n)| n.as_str()).collect();
    assert_eq!(names, ["Shelf", "Hinge", "Mark"]);
}

/// Seen square on from above, orthographic, the fill of a plane above
/// everything covers its square: (2 · half side)² in model units, times
/// the pixels per unit squared, less the pixels the outline (drawn over
/// it) takes. The colour of a filled pixel over the background is the
/// background blended with the datum colour at the fill opacity.
#[test]
fn the_fill_covers_the_square_in_front_and_is_left_out_behind_a_body() {
    let tokens = Tokens::default();
    let mut s = plate();
    run(
        &mut s,
        "datum.plane",
        json!({"from": "xy", "distance": 50, "name": "Above"}),
    );
    run(
        &mut s,
        "datum.plane",
        json!({"from": "xy", "distance": -10, "name": "Below"}),
    );
    let scene = s.scene_with(SceneQuality::Automatic).unwrap().unwrap();
    let mut camera = CameraState {
        projection: ProjectionKind::Orthographic,
        ..CameraState::default()
    };
    camera.set_view("top").unwrap();
    camera.target = [30.0, 20.0, 0.0];
    camera.distance = 400.0;
    let (w, h) = (800_u32, 800_u32);
    let frame = viewport::render_bodies(
        Some(&scene),
        &camera,
        Quality {
            width: w,
            height: h,
            ssaa: 1,
        },
        &tokens,
    )
    .unwrap();
    let render = camera.to_render();
    let craft3d_render::Projection::Orthographic { half_height } = render.projection else {
        panic!("not orthographic");
    };
    let per_unit = f64::from(h) / (2.0 * half_height);
    let fill = |name: &str| {
        let datum = scene
            .datums
            .iter()
            .find(|d| d.name == name)
            .unwrap()
            .clone();
        let marks = Marks {
            datums: vec![datum],
            ..Marks::default()
        };
        let mut rgb = frame.rgb.clone();
        viewport::fill_datum_planes(
            &mut rgb,
            w,
            h,
            &frame.depth,
            &camera,
            &marks,
            &tokens,
            viewport::depth_slack(viewport_backend(), Some(&scene), &camera),
        );
        rgb
    };
    let changed = |rgb: &[u8]| -> Vec<usize> {
        (0..(w * h) as usize)
            .filter(|&i| rgb[i * 3..i * 3 + 3] != frame.rgb[i * 3..i * 3 + 3])
            .collect()
    };
    let half = scene.bbox.diagonal() * DATUM_PLANE_EXTENT;
    let side = 2.0 * half * per_unit;
    let above = changed(&fill("Above"));
    // Every pixel whose centre is inside the square, give or take the one
    // row and column the edge passes through.
    let expected = side * side;
    assert!(
        (above.len() as f64 - expected).abs() <= 4.0 * side + 4.0,
        "{} pixels against {expected}",
        above.len()
    );
    // Over the background: the blend, rounded.
    let alpha = f64::from(tokens.datum_fill_opacity) / 255.0;
    let blend = |under: u8, ink: u8| {
        (f64::from(under) + (f64::from(ink) - f64::from(under)) * alpha).round() as u8
    };
    let rgb = fill("Above");
    let background = tokens.viewport_background;
    let over_background = above
        .iter()
        .find(|&&i| frame.ids[i] == craft3d_render::NO_FACE)
        .copied()
        .unwrap();
    assert_eq!(
        &rgb[over_background * 3..over_background * 3 + 3],
        &[
            blend(background[0], tokens.datum[0]),
            blend(background[1], tokens.datum[1]),
            blend(background[2], tokens.datum[2])
        ]
    );
    // The plane below the plate: left out where the plate is in front,
    // drawn around it. The test of the lines keeps a point whose own
    // pixel or one of the eight around it shows something at or behind
    // it, so the fill reaches one pixel onto the plate's outline.
    let below = changed(&fill("Below"));
    let wu = w as usize;
    let rim = |i: usize| {
        let (x, y) = ((i % wu) as i64, (i / wu) as i64);
        (-1..=1).any(|dy| {
            (-1..=1).any(|dx| {
                let (nx, ny) = (x + dx, y + dy);
                nx >= 0
                    && ny >= 0
                    && (nx as usize) < wu
                    && (ny as usize) < h as usize
                    && frame.ids[ny as usize * wu + nx as usize] == craft3d_render::NO_FACE
            })
        })
    };
    assert!(below.iter().all(|&i| rim(i)));
    let covered = frame
        .ids
        .iter()
        .enumerate()
        .filter(|&(i, &id)| id != craft3d_render::NO_FACE && !rim(i))
        .count();
    let outline = 2.0 * (60.0 + 40.0) * per_unit;
    assert!(
        (below.len() as f64 - (expected - covered as f64)).abs()
            <= 4.0 * side + 4.0 + 2.0 * outline,
        "{} pixels, expected {}",
        below.len(),
        expected - covered as f64
    );
}

fn viewport_backend() -> craft3d_ui_egui::backend::Backend {
    craft3d_ui_egui::backend::Backend::Cpu
}

/// `ui.set {"show": {"datums": false}}` hides them, and `ui.get` says so;
/// a screenshot changes with it.
#[test]
fn ui_set_hides_and_shows_the_datums() {
    let mut session = with_datums();
    let mut shell = Shell::new();
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["state"]["show"], json!({"datums": true}));
    let shot = |shell: &mut Shell, session: &mut Session| {
        call(
            shell,
            session,
            "ui.screenshot",
            json!({"width": 320, "height": 240, "scale": 1.0}),
        )["overlayCoverage"]
            .as_f64()
            .unwrap()
    };
    let shown = shot(&mut shell, &mut session);
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"show": {"datums": false}}),
    );
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["state"]["show"], json!({"datums": false}));
    let hidden = shot(&mut shell, &mut session);
    assert!(shown > hidden, "{shown} against {hidden}");
    let error = shell
        .handle_method(
            &mut session,
            "ui.set",
            Some(&json!({"show": {"datums": "yes"}})),
        )
        .unwrap()
        .unwrap_err();
    assert!(error.contains("ui.set"), "{error}");
    let error = shell
        .handle_method(
            &mut session,
            "ui.set",
            Some(&json!({"show": {"planes": false}})),
        )
        .unwrap()
        .unwrap_err();
    assert!(error.contains("planes"), "{error}");
}

/// The showcase housing (a datum plane at an angle about a datum axis),
/// as the developer sees it: written where the evidence goes.
#[test]
fn the_housing_shows_its_datums() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/showcase");
    let mut session = Session::new(Workspace {
        read: craft3d_engine::Access::Root(dir),
        write: craft3d_engine::Access::None,
    });
    run(
        &mut session,
        "file.open",
        json!({"path": "housing.3dcraft"}),
    );
    let scene = session
        .scene_with(SceneQuality::Automatic)
        .unwrap()
        .unwrap();
    assert!(!scene.datums.is_empty(), "the housing has datums");
    let tokens = Tokens::default();
    let camera = camera(&mut session, "iso", ProjectionKind::Perspective);
    for (shown, name) in [
        (true, "datums-housing.png"),
        (false, "datums-housing-hidden.png"),
    ] {
        let rgb = picture(&mut session, &camera, shown, &tokens);
        let png = craft3d_render::encode_png(640, 480, &rgb).unwrap();
        std::fs::write(evidence(name), png).unwrap();
    }
}
