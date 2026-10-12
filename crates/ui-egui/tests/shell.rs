//! The shell without a window: `egui::Context::run_ui` with synthetic
//! input, on a session driven by commands.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{FeatureState, Session, Workspace};
use craft3d_ui_egui::camera::{ORBIT_DEGREES_PER_POINT, Projector};
use craft3d_ui_egui::state::{CameraState, ProjectionKind};
use craft3d_ui_egui::viewport::{self, LineKind, Quality};
use craft3d_ui_egui::{Shell, Tokens};
use egui::{Event, PointerButton, Pos2, pos2, vec2};
use serde_json::{Value, json};

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// A plate, a post on it, a sketch above them and a feature that fails.
fn part() -> Session {
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
    run(
        &mut s,
        "sketch.create",
        json!({"name": "Outline", "offset": 40}),
    );
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Outline", "closed": true, "points": [[0, 0], [60, 0], [60, 40], [0, 40]]}),
    );
    run(
        &mut s,
        "feature.sphere",
        json!({"name": "Broken", "radius": "missing * 2"}),
    );
    s
}

/// A window of a fixed size, fed one frame at a time.
struct Window {
    ctx: egui::Context,
    size: egui::Vec2,
    time: f64,
}

impl Window {
    fn new(width: f32, height: f32) -> Self {
        Self {
            ctx: egui::Context::default(),
            size: vec2(width, height),
            time: 0.0,
        }
    }

    fn frame(&mut self, shell: &mut Shell, session: &mut Session, events: Vec<Event>) -> usize {
        self.time += 1.0 / 60.0;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, self.size)),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
        // What a backend would upload: how many textures were set.
        let uploads = output.textures_delta.set.len();
        output.textures_delta.clear();
        uploads
    }

    fn idle(&mut self, shell: &mut Shell, session: &mut Session, frames: usize) {
        for _ in 0..frames {
            let _ = self.frame(shell, session, Vec::new());
        }
    }

    fn button(
        &mut self,
        shell: &mut Shell,
        session: &mut Session,
        at: Pos2,
        button: PointerButton,
        pressed: bool,
    ) {
        let _ = self.frame(
            shell,
            session,
            vec![Event::PointerButton {
                pos: at,
                button,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }],
        );
    }

    /// Presses `button` at `from`, moves to `to` in `steps` frames and
    /// lets go. Returns the shell's render statistics while it was held.
    fn drag(
        &mut self,
        shell: &mut Shell,
        session: &mut Session,
        button: PointerButton,
        from: Pos2,
        to: Pos2,
        steps: usize,
    ) -> craft3d_ui_egui::RenderStats {
        let _ = self.frame(shell, session, vec![Event::PointerMoved(from)]);
        self.button(shell, session, from, button, true);
        for step in 1..=steps {
            let at = from + (to - from) * (step as f32 / steps as f32);
            let _ = self.frame(shell, session, vec![Event::PointerMoved(at)]);
        }
        let held = shell.render_stats().unwrap();
        self.button(shell, session, to, button, false);
        held
    }
}

#[test]
fn a_few_frames_show_the_tree_and_render_the_viewport_once() {
    let mut session = part();
    let mut shell = Shell::new();
    let mut window = Window::new(1000.0, 700.0);
    let first = window.frame(&mut shell, &mut session, Vec::new());
    window.idle(&mut shell, &mut session, 4);

    // The feature tree: name, kind, status, and the error of the feature
    // that failed.
    let rows: Vec<(&str, &str, &str, FeatureState, &str)> = shell
        .tree_rows()
        .iter()
        .map(|r| {
            (
                r.id.as_str(),
                r.name.as_str(),
                r.kind,
                r.state,
                r.status.as_str(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            ("F1", "Plate", "block", FeatureState::Ok, "ok"),
            ("F2", "Post", "cylinder", FeatureState::Ok, "ok"),
            // Four free corners, two coordinates each.
            ("F3", "Outline", "sketch", FeatureState::Ok, "8 DOF"),
            ("F4", "Broken", "sphere", FeatureState::Failed, "failed"),
        ]
    );
    let error = shell.tree_rows()[3].error.as_deref().unwrap();
    assert!(error.contains("missing"), "{error}");
    assert!(shell.tree_rows().iter().all(|r| !r.selected));

    // The viewport is the window minus the tree on the left and the status
    // bar at the bottom, and the image has its size in pixels.
    let rect = shell.viewport_rect().unwrap();
    assert!(rect.left() > 150.0 && rect.left() < 450.0, "{rect:?}");
    assert_eq!((rect.top(), rect.right()), (0.0, 1000.0));
    assert!(rect.bottom() < 700.0 && rect.bottom() > 640.0, "{rect:?}");
    let [w, h] = shell.viewport_size();
    assert_eq!(
        (w, h),
        (rect.width().round() as u32, rect.height().round() as u32)
    );

    // The image was uploaded as a texture in a frame that rendered, and is
    // not rendered again while nothing changes.
    assert!(first > 0);
    let rendered = shell.render_count();
    let stats = shell.render_stats().unwrap();
    assert_eq!((stats.width, stats.height, stats.ssaa), (w, h, 2));
    assert!(stats.triangles > 12, "{stats:?}");
    window.idle(&mut shell, &mut session, 10);
    assert_eq!(shell.render_count(), rendered, "idle frames do not render");

    // A command that reads does not render either; one that edits does,
    // once.
    run(&mut session, "document.measure", json!({}));
    window.idle(&mut shell, &mut session, 3);
    assert_eq!(shell.render_count(), rendered);
    run(
        &mut session,
        "feature.edit",
        json!({"feature": "Post", "height": 20}),
    );
    window.idle(&mut shell, &mut session, 3);
    assert_eq!(shell.render_count(), rendered + 1);

    // The camera framed the model by itself: every corner of the scene's
    // box is inside the viewport.
    let scene = session.scene(None).unwrap().unwrap();
    let (lo, hi) = scene.bbox.corners().unwrap();
    let projector =
        Projector::new(&shell.state.camera.to_render(), f64::from(w), f64::from(h)).unwrap();
    for x in [lo.x, hi.x] {
        for y in [lo.y, hi.y] {
            for z in [lo.z, hi.z] {
                let p = projector.project(craft3d_math::Vec3::new(x, y, z)).unwrap();
                assert!((0.0..=f64::from(w)).contains(&p.x), "{p:?}");
                assert!((0.0..=f64::from(h)).contains(&p.y), "{p:?}");
            }
        }
    }
    assert!(shell.problem().is_none());
}

#[test]
fn dragging_with_the_left_button_orbits_by_the_distance_dragged() {
    let mut session = part();
    let mut shell = Shell::new();
    let mut window = Window::new(1000.0, 700.0);
    window.idle(&mut shell, &mut session, 3);
    let before = shell.state.camera;
    let from = shell.viewport_rect().unwrap().center();
    let [w, h] = shell.viewport_size();

    // 50 points to the right and 20 down, in seven uneven steps.
    let held = window.drag(
        &mut shell,
        &mut session,
        PointerButton::Primary,
        from,
        from + vec2(50.0, 20.0),
        7,
    );
    window.idle(&mut shell, &mut session, 3);
    let after = shell.state.camera;
    let (yaw, pitch) = (
        before.yaw - 50.0 * ORBIT_DEGREES_PER_POINT,
        before.pitch + 20.0 * ORBIT_DEGREES_PER_POINT,
    );
    assert_eq!(ORBIT_DEGREES_PER_POINT, 0.4);
    assert!((after.yaw - yaw).abs() < 1e-4, "yaw {} vs {yaw}", after.yaw);
    assert!(
        (after.pitch - pitch).abs() < 1e-4,
        "pitch {} vs {pitch}",
        after.pitch
    );
    // Nothing else moved: an orbit keeps the target and the distance, and
    // the camera keeps following the model.
    assert_eq!(after.target, before.target);
    assert_eq!(after.distance, before.distance);
    assert!(after.auto_fit);

    // While the button was down the image was half the size and not
    // supersampled; at rest it is sharp again.
    assert_eq!((held.width, held.height, held.ssaa), (w / 2, h / 2, 1));
    let rest = shell.render_stats().unwrap();
    assert_eq!((rest.width, rest.height, rest.ssaa), (w, h, 2));

    // The same drag in one step, or in forty, ends in the same place.
    for steps in [1, 40] {
        let mut other = Shell::new();
        let mut window = Window::new(1000.0, 700.0);
        window.idle(&mut other, &mut session, 3);
        window.drag(
            &mut other,
            &mut session,
            PointerButton::Primary,
            from,
            from + vec2(50.0, 20.0),
            steps,
        );
        assert!(
            (other.state.camera.yaw - after.yaw).abs() < 1e-4,
            "{steps} steps"
        );
        assert!(
            (other.state.camera.pitch - after.pitch).abs() < 1e-4,
            "{steps} steps"
        );
    }

    // A drag that starts outside the viewport (on the tree) does nothing.
    let settled = shell.state.camera;
    window.drag(
        &mut shell,
        &mut session,
        PointerButton::Primary,
        pos2(20.0, 300.0),
        pos2(120.0, 350.0),
        5,
    );
    assert_eq!(shell.state.camera, settled);
}

#[test]
fn dragging_with_the_right_button_pans_and_the_wheel_zooms() {
    let mut session = part();
    let mut shell = Shell::new();
    let mut window = Window::new(1000.0, 700.0);
    window.idle(&mut shell, &mut session, 3);
    let before = shell.state.camera;
    let rect = shell.viewport_rect().unwrap();
    let centre = rect.center();

    window.drag(
        &mut shell,
        &mut session,
        PointerButton::Secondary,
        centre,
        centre + vec2(30.0, -12.0),
        4,
    );
    let panned = shell.state.camera;
    // The same as panning the camera it started from by hand.
    let mut expected = before;
    expected.pan(30.0, -12.0, f64::from(rect.height()));
    for (got, want) in panned.target.iter().zip(expected.target) {
        assert!((got - want).abs() < 1e-4 * before.distance, "{panned:?}");
    }
    assert_ne!(panned.target, before.target);
    assert_eq!((panned.yaw, panned.pitch), (before.yaw, before.pitch));
    assert_eq!(panned.distance, before.distance);
    assert!(!panned.auto_fit, "a camera placed by hand stays put");

    // The model no longer pulls the camera along.
    run(
        &mut session,
        "feature.block",
        json!({"name": "Far", "size": [10, 10, 10], "position": [500, 0, 0]}),
    );
    window.idle(&mut shell, &mut session, 2);
    assert_eq!(shell.state.camera, panned);

    // Wheel away from the user: closer. The pointer is over the viewport.
    let _ = window.frame(
        &mut shell,
        &mut session,
        vec![
            Event::PointerMoved(centre),
            Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, 120.0),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    window.idle(&mut shell, &mut session, 30);
    let zoomed = shell.state.camera;
    assert!(
        zoomed.distance < panned.distance * 0.9,
        "{}",
        zoomed.distance
    );
    assert!(
        zoomed.distance > panned.distance * 0.5,
        "{}",
        zoomed.distance
    );
    assert_eq!(zoomed.target, panned.target);

    // F frames everything again, the far block included, and the camera
    // follows the model again.
    let _ = window.frame(
        &mut shell,
        &mut session,
        vec![Event::Key {
            key: egui::Key::F,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    window.idle(&mut shell, &mut session, 2);
    let fitted = shell.state.camera;
    assert!(fitted.auto_fit);
    let scene = session.scene(None).unwrap().unwrap();
    let mut by_hand = CameraState {
        yaw: before.yaw,
        pitch: before.pitch,
        ..CameraState::default()
    };
    let [w, h] = shell.viewport_size();
    by_hand.fit(&scene.bbox, f64::from(w) / f64::from(h));
    assert_eq!(fitted.target, by_hand.target);
    assert_eq!(fitted.distance, by_hand.distance);
    assert!(fitted.target[0] > 200.0, "the far block is in the frame");
}

#[test]
fn clicking_a_row_selects_the_feature_and_its_sketch_is_highlighted() {
    let mut session = part();
    let mut shell = Shell::new();
    let mut window = Window::new(1000.0, 700.0);
    window.idle(&mut shell, &mut session, 3);
    let viewport_left = shell.viewport_rect().unwrap().left();
    assert!(viewport_left > 60.0);

    // Click down the tree until a row takes the selection. Every row is
    // somewhere in the top of the left panel.
    let mut picked = Vec::new();
    let mut at = Vec::new();
    for y in (20..240).step_by(3) {
        let here = pos2(40.0, y as f32);
        let was = shell.state.selected_feature.clone();
        let _ = window.frame(&mut shell, &mut session, vec![Event::PointerMoved(here)]);
        window.button(&mut shell, &mut session, here, PointerButton::Primary, true);
        window.button(
            &mut shell,
            &mut session,
            here,
            PointerButton::Primary,
            false,
        );
        window.idle(&mut shell, &mut session, 1);
        let now = shell.state.selected_feature.clone();
        if now != was
            && let Some(id) = now
            && !picked.contains(&id)
        {
            picked.push(id);
            at.push(here);
        }
    }
    assert_eq!(picked, ["F1", "F2", "F3", "F4"], "rows from top to bottom");

    // Selecting the sketch: its row is marked and its lines change kind.
    let sketch_row = at[2];
    shell.state.selected_feature = None;
    window.idle(&mut shell, &mut session, 1);
    window.button(
        &mut shell,
        &mut session,
        sketch_row,
        PointerButton::Primary,
        true,
    );
    window.button(
        &mut shell,
        &mut session,
        sketch_row,
        PointerButton::Primary,
        false,
    );
    window.idle(&mut shell, &mut session, 1);
    assert_eq!(shell.state.selected_feature.as_deref(), Some("F3"));
    let selected: Vec<bool> = shell.tree_rows().iter().map(|r| r.selected).collect();
    assert_eq!(selected, [false, false, true, false]);
    let [w, h] = shell.viewport_size();
    let scene = session.scene(None).unwrap().unwrap();
    let kinds = |selected: Option<&str>| -> Vec<LineKind> {
        viewport::overlay(Some(&scene), &shell.state.camera, w, h, selected)
            .lines
            .iter()
            .map(|l| l.kind)
            .filter(|k| !matches!(k, LineKind::Axis(_)))
            .collect()
    };
    assert_eq!(kinds(Some("F3")), [LineKind::Selected; 4]);
    assert_eq!(kinds(None), [LineKind::Sketch; 4]);
    assert_eq!(kinds(Some("F1")), [LineKind::Sketch; 4]);

    // Clicking it again clears the selection; so does deleting the feature
    // (by a command: the shell itself changed nothing in the document).
    let revision = session.scene_revision();
    window.button(
        &mut shell,
        &mut session,
        sketch_row,
        PointerButton::Primary,
        true,
    );
    window.button(
        &mut shell,
        &mut session,
        sketch_row,
        PointerButton::Primary,
        false,
    );
    window.idle(&mut shell, &mut session, 1);
    assert_eq!(shell.state.selected_feature, None);
    assert_eq!(session.scene_revision(), revision);
    shell.state.selected_feature = Some("F3".to_string());
    run(
        &mut session,
        "feature.delete",
        json!({"feature": "Outline"}),
    );
    window.idle(&mut shell, &mut session, 1);
    assert_eq!(shell.state.selected_feature, None);
    assert_eq!(shell.tree_rows().len(), 3);
}

#[test]
fn resizing_the_window_draws_coarse_frames_and_then_a_sharp_one() {
    let mut session = part();
    let mut shell = Shell::new();
    let mut window = Window::new(1000.0, 700.0);
    window.idle(&mut shell, &mut session, 3);
    let [w, h] = shell.viewport_size();
    let rest = shell.render_stats().unwrap();
    assert_eq!((rest.width, rest.height, rest.ssaa), (w, h, 2));
    let framed = shell.state.camera;

    // The window gets narrower and a little taller every frame, as when its
    // corner is dragged.
    for step in 1..=5 {
        window.size = vec2(1000.0 - 80.0 * step as f32, 700.0 + 10.0 * step as f32);
        let _ = window.frame(&mut shell, &mut session, Vec::new());
        let [w, h] = shell.viewport_size();
        let stats = shell.render_stats().unwrap();
        assert_eq!((stats.width, stats.height, stats.ssaa), (w / 2, h / 2, 1));
    }
    // It stops: the next frame is the sharp one, and then nothing more is
    // rendered.
    window.idle(&mut shell, &mut session, 1);
    let [w2, h2] = shell.viewport_size();
    assert!(w2 < w && h2 > h);
    let stats = shell.render_stats().unwrap();
    assert_eq!((stats.width, stats.height, stats.ssaa), (w2, h2, 2));
    let rendered = shell.render_count();
    window.idle(&mut shell, &mut session, 5);
    assert_eq!(shell.render_count(), rendered);
    // The model was framed again for the new shape of the viewport: it is
    // tall and narrow now, so the width decides and the camera is further.
    assert!(shell.state.camera.distance > framed.distance * 1.2);
    let mut by_hand = framed;
    let scene = session.scene(None).unwrap().unwrap();
    by_hand.fit(&scene.bbox, f64::from(w2) / f64::from(h2));
    assert_eq!(shell.state.camera, by_hand);
}

#[test]
fn panels_can_be_closed_and_the_viewport_takes_their_place() {
    let mut session = part();
    let mut shell = Shell::new();
    let mut window = Window::new(800.0, 600.0);
    window.idle(&mut shell, &mut session, 3);
    let both = shell.viewport_rect().unwrap();
    assert!(both.left() > 0.0 && both.bottom() < 600.0);

    shell.state.panels.tree = false;
    window.idle(&mut shell, &mut session, 3);
    let no_tree = shell.viewport_rect().unwrap();
    assert_eq!(no_tree.left(), 0.0);
    assert_eq!(no_tree.bottom(), both.bottom());

    shell.state.panels.status = false;
    window.idle(&mut shell, &mut session, 3);
    let neither = shell.viewport_rect().unwrap();
    assert_eq!(
        (
            neither.left(),
            neither.top(),
            neither.right(),
            neither.bottom()
        ),
        (0.0, 0.0, 800.0, 600.0)
    );
    assert_eq!(shell.viewport_size(), [800, 600]);
    // The rows are kept up to date whether or not they are shown.
    assert_eq!(shell.tree_rows().len(), 4);
}

#[test]
fn an_empty_session_and_odd_windows_draw_without_a_picture_or_a_panic() {
    let mut session = Session::new(Workspace::none());
    let mut shell = Shell::new();
    let mut window = Window::new(900.0, 600.0);
    window.idle(&mut shell, &mut session, 3);
    assert!(shell.tree_rows().is_empty());
    assert!(shell.problem().is_none());
    // The mouse still works on nothing.
    let centre = shell.viewport_rect().unwrap().center();
    window.drag(
        &mut shell,
        &mut session,
        PointerButton::Primary,
        centre,
        centre + vec2(10.0, 10.0),
        2,
    );
    // A document without features, then with one.
    run(&mut session, "file.new", json!({}));
    window.idle(&mut shell, &mut session, 2);
    assert!(shell.tree_rows().is_empty());
    run(
        &mut session,
        "feature.torus",
        json!({"major": 20, "minor": 5}),
    );
    window.idle(&mut shell, &mut session, 2);
    assert_eq!(shell.tree_rows().len(), 1);
    run(&mut session, "file.close", json!({"discard": true}));
    window.idle(&mut shell, &mut session, 2);
    assert!(shell.tree_rows().is_empty());

    // Windows of every unreasonable size.
    let mut session = part();
    for (w, h) in [
        (1.0, 1.0),
        (0.0, 0.0),
        (10.0, 3000.0),
        (30000.0, 20.0),
        (300.0, 200.0),
    ] {
        let mut shell = Shell::new();
        let mut window = Window::new(w, h);
        window.idle(&mut shell, &mut session, 3);
        let [pw, ph] = shell.viewport_size();
        assert!(
            (16..=4096).contains(&pw) && (16..=4096).contains(&ph),
            "{w}x{h}"
        );
        assert!(shell.state.camera.validated().is_ok(), "{w}x{h}");
    }
    // A camera in a state the mouse cannot reach still draws.
    let mut shell = Shell::new();
    shell.state.camera = CameraState {
        distance: 1e-9,
        pitch: 90.0,
        projection: ProjectionKind::Orthographic,
        auto_fit: false,
        ..CameraState::default()
    };
    let mut window = Window::new(600.0, 400.0);
    window.idle(&mut shell, &mut session, 3);
}

/// A 20 × 20 square sketched on the XY plane, seen from the top with the
/// camera 100 above its middle: a 400 × 400 image shows 2 · 100 · tan(15°)
/// = 53.59 units, so one unit is 7.464 pixels and the square is 149.28
/// pixels wide, centred on the image.
#[test]
fn a_sketch_square_seen_from_the_top_projects_to_a_square_of_the_expected_size() {
    let mut session = Session::new(Workspace::none());
    run(&mut session, "file.new", json!({}));
    run(&mut session, "sketch.create", json!({"name": "Square"}));
    run(
        &mut session,
        "sketch.polyline",
        json!({"sketch": "Square", "closed": true, "points": [[0, 0], [20, 0], [20, 20], [0, 20]]}),
    );
    run(
        &mut session,
        "sketch.line",
        json!({"sketch": "Square", "from": [0, 0], "to": [20, 20], "construction": true}),
    );
    let scene = session.scene(None).unwrap().unwrap();
    let mut camera = CameraState {
        target: [10.0, 10.0, 0.0],
        distance: 100.0,
        auto_fit: false,
        ..CameraState::default()
    };
    camera.set_view("top").unwrap();

    let overlay = viewport::overlay(Some(&scene), &camera, 400, 400, None);
    let per_unit = 400.0 / (2.0 * 100.0 * 15.0_f64.to_radians().tan());
    let half = 10.0 * per_unit;
    assert!((half - 74.641).abs() < 1e-3, "{half}");
    let (left, right, top, bottom) = (200.0 - half, 200.0 + half, 200.0 - half, 200.0 + half);
    // In drawing order: bottom edge, right edge, top edge, left edge. The
    // sketch's +y is up on the screen, which is towards smaller pixel y.
    let expected = [
        ([left, bottom], [right, bottom]),
        ([right, bottom], [right, top]),
        ([right, top], [left, top]),
        ([left, top], [left, bottom]),
    ];
    let sketch: Vec<_> = overlay
        .lines
        .iter()
        .filter(|l| l.kind == LineKind::Sketch)
        .collect();
    assert_eq!(sketch.len(), 4);
    for (line, (from, to)) in sketch.iter().zip(expected) {
        for (got, want) in line.from.iter().chain(&line.to).zip(from.iter().chain(&to)) {
            assert!((got - want).abs() < 1e-6, "{line:?} vs {from:?} -> {to:?}");
        }
    }
    // The construction diagonal, flagged, from the lower left to the upper
    // right.
    let guides: Vec<_> = overlay
        .lines
        .iter()
        .filter(|l| l.kind == LineKind::Construction)
        .collect();
    assert_eq!(guides.len(), 1);
    assert!((guides[0].from[0] - left).abs() < 1e-6 && (guides[0].from[1] - bottom).abs() < 1e-6);
    assert!((guides[0].to[0] - right).abs() < 1e-6 && (guides[0].to[1] - top).abs() < 1e-6);
    // Six point entities as dots: the four corners, and the two ends of the
    // diagonal, which was drawn from coordinates and so has points of its
    // own on top of two corners.
    assert_eq!(overlay.points.len(), 6);

    // And in the picture: the edges are in the sketch colour, the diagonal
    // in the construction colour, the rest is background.
    let tokens = Tokens::default();
    let quality = Quality {
        width: 400,
        height: 400,
        ssaa: 1,
    };
    let picture = viewport::picture(Some(&scene), &camera, quality, &tokens, None, 1.0).unwrap();
    let pixel = |x: f64, y: f64| {
        let i = (y as usize * 400 + x as usize) * 3;
        [picture.rgb[i], picture.rgb[i + 1], picture.rgb[i + 2]]
    };
    let close = |a: [u8; 3], b: [u8; 3]| a.iter().zip(b).all(|(p, q)| p.abs_diff(q) <= 12);
    for (x, y) in [(200.0, bottom), (right, 240.0), (150.0, top), (left, 160.0)] {
        assert!(
            close(pixel(x, y), tokens.sketch),
            "({x}, {y}): {:?}",
            pixel(x, y)
        );
    }
    // On the diagonal (x + y = 400), away from the corners and the axes at
    // the origin: the pixel whose centre (229.5, 170.5) is on it. The line
    // is one pixel wide, so its neighbours are only tinted.
    assert!(
        close(pixel(229.0, 170.0), tokens.sketch_construction),
        "{:?}",
        pixel(229.0, 170.0)
    );
    for (x, y) in [(200.0, 150.0), (30.0, 30.0), (380.0, 200.0), (250.0, 200.0)] {
        assert_eq!(pixel(x, y), tokens.viewport_background, "({x}, {y})");
    }
    assert_eq!(picture.coverage, 0.0, "no bodies");
    assert!(picture.overlay_coverage > 0.003 && picture.overlay_coverage < 0.05);
}

/// What ties the overlay to the image: a block's silhouette as the CPU
/// renderer draws it is where the projector puts its corners.
#[test]
fn the_overlay_projects_where_the_renderer_draws() {
    let mut session = Session::new(Workspace::none());
    run(&mut session, "file.new", json!({}));
    run(
        &mut session,
        "feature.block",
        json!({"size": [30, 18, 12], "position": [-4, 3, 2]}),
    );
    let scene = session.scene(None).unwrap().unwrap();
    let tokens = Tokens::default();
    for projection in [ProjectionKind::Perspective, ProjectionKind::Orthographic] {
        for (yaw, pitch) in [(-90.0, 90.0), (-45.0, 29.5), (160.0, -40.0), (0.0, 0.0)] {
            let (w, h) = (320_u32, 200_u32);
            let mut camera = CameraState {
                yaw,
                pitch,
                projection,
                ..CameraState::default()
            };
            camera.fit(&scene.bbox, f64::from(w) / f64::from(h));
            let quality = Quality {
                width: w,
                height: h,
                ssaa: 1,
            };
            let frame = viewport::render_bodies(Some(&scene), &camera, quality, &tokens).unwrap();
            // The covered pixels' bounding box, in pixel edges.
            let (mut min, mut max) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
            for (i, id) in frame.ids.iter().enumerate() {
                if *id != craft3d_render::NO_FACE {
                    let (x, y) = ((i % w as usize) as f64, (i / w as usize) as f64);
                    min = [min[0].min(x), min[1].min(y)];
                    max = [max[0].max(x + 1.0), max[1].max(y + 1.0)];
                }
            }
            // The same box from the projected corners.
            let projector =
                Projector::new(&camera.to_render(), f64::from(w), f64::from(h)).unwrap();
            let (lo, hi) = scene.bbox.corners().unwrap();
            let (mut pmin, mut pmax) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
            for x in [lo.x, hi.x] {
                for y in [lo.y, hi.y] {
                    for z in [lo.z, hi.z] {
                        let p = projector.project(craft3d_math::Vec3::new(x, y, z)).unwrap();
                        pmin = [pmin[0].min(p.x), pmin[1].min(p.y)];
                        pmax = [pmax[0].max(p.x), pmax[1].max(p.y)];
                    }
                }
            }
            // A pixel is covered when its centre is inside, so the two
            // agree within one pixel.
            for k in 0..2 {
                assert!(
                    (min[k] - pmin[k]).abs() <= 1.0 && (max[k] - pmax[k]).abs() <= 1.0,
                    "{projection:?} yaw {yaw} pitch {pitch}: image {min:?}..{max:?}, projected {pmin:?}..{pmax:?}"
                );
            }
        }
    }
}
