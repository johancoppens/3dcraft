//! The shell without a window: `egui::Context::run_ui` with synthetic
//! input, on a session driven by commands.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{
    AUTOMATIC_RELATIVE_TOLERANCE, DISPLAY_MAX_ANGLE_DEGREES, DISPLAY_MAX_TRIANGLES,
    DISPLAY_RELATIVE_TOLERANCE, FeatureState, SceneQuality, Session, Workspace,
};
use craft3d_math::Portable;
use craft3d_ui_egui::camera::{ORBIT_DEGREES_PER_POINT, Projector};
use craft3d_ui_egui::shell::image_quality;
use craft3d_ui_egui::state::{CameraState, ProjectionKind};
use craft3d_ui_egui::viewport::{self, LineKind, Quality};
use craft3d_ui_egui::{FileAction, Shell, Tokens, window_title};
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

/// The last image of `width` x `height` among `uploads` (width, height and
/// RGB bytes, in the order they went on screen).
fn last_upload(uploads: &[(usize, usize, Vec<u8>)], width: u32, height: u32) -> Option<&[u8]> {
    uploads
        .iter()
        .rev()
        .find(|(w, h, _)| (*w, *h) == (width as usize, height as usize))
        .map(|(_, _, rgb)| rgb.as_slice())
}

/// A window of a fixed size, fed one frame at a time.
struct Window {
    ctx: egui::Context,
    size: egui::Vec2,
    time: f64,
    /// The images the last frame uploaded as whole textures: width,
    /// height and RGB bytes.
    uploads: Vec<(usize, usize, Vec<u8>)>,
}

impl Window {
    fn new(width: f32, height: f32) -> Self {
        Self {
            ctx: egui::Context::default(),
            size: vec2(width, height),
            time: 0.0,
            uploads: Vec::new(),
        }
    }

    /// The image of `width` x `height` the last frame put on screen.
    fn uploaded(&self, width: u32, height: u32) -> Option<&[u8]> {
        last_upload(&self.uploads, width, height)
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
        self.uploads = output
            .textures_delta
            .set
            .values()
            .flat_map(|deltas| deltas.iter())
            .filter(|delta| delta.pos.is_none())
            .map(|delta| match &delta.image {
                egui::ImageData::Color(image) => (
                    image.width(),
                    image.height(),
                    image
                        .pixels
                        .iter()
                        .flat_map(|p| [p.r(), p.g(), p.b()])
                        .collect(),
                ),
            })
            .collect();
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

    // The viewport is the window minus the menu bar and the row of the
    // documents' tabs at the top, the tree on the left and the status bar
    // at the bottom, and the image has its size in pixels.
    let rect = shell.viewport_rect().unwrap();
    assert!(rect.left() > 150.0 && rect.left() < 450.0, "{rect:?}");
    let tabs = shell.tab_hits()[0].1;
    assert!(tabs.top() > 12.0 && tabs.bottom() <= rect.top(), "{tabs:?}");
    assert!(rect.top() > 40.0 && rect.top() < 70.0, "{rect:?}");
    assert_eq!(rect.right(), 1000.0);
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

    // The camera framed the model by itself: every corner of the box
    // around the bodies is inside the viewport (the opening frame, which
    // the camera follows until Fit, holds the bodies drawn and not the
    // sketch above them, 2026-11-11).
    let scene = session.scene_with(shell.scene_quality()).unwrap().unwrap();
    let (lo, hi) = scene.bodies_box.corners().unwrap();
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
    // The zoom is at the pointer (2026-10-16): in the middle of the
    // viewport the point under it is on the line of sight, so the target
    // moves along that line only, towards the model under the pointer.
    assert_eq!((zoomed.yaw, zoomed.pitch), (panned.yaw, panned.pitch));
    let moved = zoomed.target() - panned.target();
    let sideways = moved - panned.toward_eye() * moved.dot(panned.toward_eye());
    assert!(sideways.length() < 1e-6 * panned.distance, "{moved:?}");
    assert!(!zoomed.auto_fit);

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
    let scene = session.scene_with(shell.scene_quality()).unwrap().unwrap();
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
    let scene = session.scene_with(shell.scene_quality()).unwrap().unwrap();
    let kinds = |selected: Option<&str>| -> Vec<LineKind> {
        viewport::overlay(Some(&scene), &shell.state.camera, w, h, selected)
            .lines
            .iter()
            .map(|l| l.kind)
            .filter(|k| !matches!(k, LineKind::Axis(_) | LineKind::Edge))
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
    // The desktop layout at every size here: the narrow one, with its
    // taller bars, is `tests/mobile.rs`'s.
    shell.tokens.narrow_width = 0.0;
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
    // Framed as it opened, on the bodies drawn, as the part without its
    // sketch is: the sketch 2 mm above the post is not in the opening
    // frame (2026-11-11).
    assert!(shell.state.camera.distance > framed.distance * 1.2);
    let mut by_hand = framed;
    let mut bodies_alone = part();
    run(
        &mut bodies_alone,
        "feature.delete",
        json!({"feature": "Outline"}),
    );
    let scene = bodies_alone
        .scene_with(shell.scene_quality())
        .unwrap()
        .unwrap();
    assert!(scene.sketches.is_empty());
    by_hand.fit(&scene.frame_box(), f64::from(w2) / f64::from(h2));
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
    // The menu bar and the row of tabs stay: a row of text and a row of
    // tabs high.
    let menu = both.top();
    assert!(menu > 40.0 && menu < 70.0, "{menu}");

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
        (0.0, menu, 800.0, 600.0)
    );
    assert_eq!(shell.viewport_size(), [800, (600.0 - menu).round() as u32]);
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
    let per_unit = 400.0 / (2.0 * 100.0 * 15.0_f64.to_radians().ptan());
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

/// How long a test waits for the render thread before it fails. The wait
/// ends when the image is there: this is a limit, not a pause.
const LONG: std::time::Duration = std::time::Duration::from_secs(120);

fn direct(
    session: &mut Session,
    shell: &Shell,
    camera: &CameraState,
    moving: bool,
) -> viewport::Composed {
    let scene = session.scene_with(shell.scene_quality()).unwrap();
    viewport::compose(
        scene.as_deref(),
        camera,
        image_quality(shell.viewport_size(), moving, 8192),
        &shell.tokens,
        shell.state.selected_feature.as_deref(),
    )
    .unwrap()
}

#[test]
fn with_a_render_thread_a_preview_is_shown_first_and_the_sharp_image_when_it_is_ready() {
    let mut session = part();
    let mut shell = Shell::threaded();
    assert!(shell.renders_in_background());
    assert!(!Shell::new().renders_in_background());
    let mut window = Window::new(1000.0, 700.0);
    // Two frames: the first lays the panels out, so the viewport has its
    // size from the second on. The sharp image is made on the other thread
    // meanwhile, and one of these frames may take it in already (the
    // second did once in the gate, 2026-11-08, and the frame after the
    // wait then had nothing to upload): what every frame put on screen is
    // kept.
    let mut uploads = Vec::new();
    let _ = window.frame(&mut shell, &mut session, Vec::new());
    uploads.append(&mut window.uploads);
    // The preview, and the sharp image too if the first frame took it in.
    let made_by_the_first_frame = shell.render_count();
    let _ = window.frame(&mut shell, &mut session, Vec::new());
    uploads.append(&mut window.uploads);
    let [w, h] = shell.viewport_size();
    let camera = shell.state.camera;

    // Before the thread's image is taken in, the preview is on screen:
    // half the size, one sample, made on this thread.
    let preview = shell.render_stats().unwrap();
    if !preview.sharp {
        assert_eq!(
            (preview.width, preview.height, preview.ssaa),
            (w / 2, h / 2, 1)
        );
    }
    // The sharp image arrives; the next frame shows it, if no frame did
    // already, and it is exactly the image of this camera.
    assert!(shell.wait_for_sharp(LONG));
    let _ = window.frame(&mut shell, &mut session, Vec::new());
    uploads.append(&mut window.uploads);
    let sharp = shell.render_stats().unwrap();
    assert!(sharp.sharp);
    assert_eq!((sharp.width, sharp.height, sharp.ssaa), (w, h, 2));
    let expected = direct(&mut session, &shell, &camera, false);
    assert_eq!(
        last_upload(&uploads, w, h).unwrap(),
        expected.rgb.as_slice()
    );
    // The preview went on screen first, in every run: the image of half the
    // size before the first one of the full size, and exactly the preview
    // of this camera. The one exception is not a race of the test: a frame
    // hands the backend only the last whole image of a texture, and the
    // first frame has two passes when the interface's fonts arrive
    // (`fonts.rs`), so a sharp image its second pass takes in replaces the
    // preview's upload; that frame then made both images.
    let at = |[uw, uh]: [u32; 2]| {
        uploads
            .iter()
            .position(|(w, h, _)| (*w, *h) == (uw as usize, uh as usize))
    };
    let sharp_at = at([w, h]).unwrap();
    match at([w / 2, h / 2]) {
        Some(preview_at) => {
            assert!(preview_at < sharp_at, "the sharp image came first");
            let preview = direct(&mut session, &shell, &camera, true);
            assert_eq!((preview.width, preview.height), (w / 2, h / 2));
            assert_eq!(uploads[preview_at].2, preview.rgb);
        }
        None => assert!(
            made_by_the_first_frame >= 2,
            "no preview went on screen ({made_by_the_first_frame} image made by the first frame)"
        ),
    }
    // Nothing more is rendered while nothing changes, and there is nothing
    // to wait for.
    let rendered = shell.render_count();
    window.idle(&mut shell, &mut session, 5);
    assert!(shell.wait_for_sharp(std::time::Duration::ZERO));
    assert_eq!(shell.render_count(), rendered);
    assert!(shell.problem().is_none());
}

#[test]
fn a_sharp_image_of_an_old_camera_or_an_old_document_is_never_put_on_screen() {
    let mut session = part();
    let mut shell = Shell::threaded();
    let mut window = Window::new(1000.0, 700.0);
    window.idle(&mut shell, &mut session, 2);
    assert!(shell.wait_for_sharp(LONG));
    let _ = window.frame(&mut shell, &mut session, Vec::new());
    assert!(shell.render_stats().unwrap().sharp);
    let [w, h] = shell.viewport_size();
    let first = shell.state.camera;

    // The camera turns: a preview at once, the sharp image is being made.
    shell.state.camera.yaw += 40.0;
    let turned = shell.state.camera;
    let _ = window.frame(&mut shell, &mut session, Vec::new());
    assert!(!shell.render_stats().unwrap().sharp);
    let preview = direct(&mut session, &shell, &turned, true);
    assert_eq!(
        window.uploaded(preview.width, preview.height).unwrap(),
        preview.rgb.as_slice()
    );
    // That image is finished, but before a frame takes it in the camera
    // turns back and further: it is now the image of an old camera.
    assert!(shell.wait_for_sharp(LONG));
    shell.state.camera.yaw -= 90.0;
    let now = shell.state.camera;
    assert!(now != first && now != turned);
    let _ = window.frame(&mut shell, &mut session, Vec::new());
    let stats = shell.render_stats().unwrap();
    assert!(!stats.sharp, "the stale sharp image was shown: {stats:?}");
    assert!(
        window.uploaded(w, h).is_none(),
        "nothing of the full size went on screen"
    );
    let preview = direct(&mut session, &shell, &now, true);
    assert_eq!(
        window.uploaded(preview.width, preview.height).unwrap(),
        preview.rgb.as_slice(),
        "the preview of the camera as it is now"
    );
    // The image of the camera as it is now follows.
    assert!(shell.wait_for_sharp(LONG));
    let _ = window.frame(&mut shell, &mut session, Vec::new());
    assert!(shell.render_stats().unwrap().sharp);
    let expected = direct(&mut session, &shell, &now, false);
    assert_eq!(window.uploaded(w, h).unwrap(), expected.rgb.as_slice());

    // The same with the document: an edit while its image is being made.
    shell.state.camera.auto_fit = false;
    shell.state.camera.pitch = 10.0;
    let camera = shell.state.camera;
    let _ = window.frame(&mut shell, &mut session, Vec::new());
    assert!(shell.wait_for_sharp(LONG));
    let before_edit = direct(&mut session, &shell, &camera, false);
    run(
        &mut session,
        "feature.edit",
        json!({"feature": "Post", "height": 12}),
    );
    let _ = window.frame(&mut shell, &mut session, Vec::new());
    assert!(!shell.render_stats().unwrap().sharp);
    assert!(window.uploaded(w, h).is_none());
    assert!(shell.wait_for_sharp(LONG));
    let _ = window.frame(&mut shell, &mut session, Vec::new());
    let after_edit = direct(&mut session, &shell, &camera, false);
    assert_ne!(before_edit.rgb, after_edit.rgb, "the edit is visible");
    assert_eq!(window.uploaded(w, h).unwrap(), after_edit.rgb.as_slice());
    assert!(shell.render_stats().unwrap().sharp);
}

#[test]
fn dragging_with_a_render_thread_shows_previews_only_and_ends_sharp() {
    let mut session = part();
    let mut shell = Shell::threaded();
    let mut window = Window::new(1000.0, 700.0);
    window.idle(&mut shell, &mut session, 2);
    assert!(shell.wait_for_sharp(LONG));
    window.idle(&mut shell, &mut session, 1);
    let [w, h] = shell.viewport_size();
    let from = shell.viewport_rect().unwrap().center();
    let held = window.drag(
        &mut shell,
        &mut session,
        PointerButton::Primary,
        from,
        from + vec2(60.0, 10.0),
        6,
    );
    assert!(!held.sharp);
    assert_eq!((held.width, held.height, held.ssaa), (w / 2, h / 2, 1));
    // Let go: the frames after it ask for the sharp image of where the
    // camera stopped. The release frame asks already (the camera no longer
    // moves in it), so the frame after it may take the image in before the
    // wait: what both put on screen is kept, as in
    // `with_a_render_thread_a_preview_is_shown_first_…`.
    let mut uploads = std::mem::take(&mut window.uploads);
    window.idle(&mut shell, &mut session, 1);
    uploads.append(&mut window.uploads);
    assert!(shell.wait_for_sharp(LONG));
    window.idle(&mut shell, &mut session, 1);
    uploads.append(&mut window.uploads);
    let rest = shell.render_stats().unwrap();
    assert!(rest.sharp);
    let camera = shell.state.camera;
    let expected = direct(&mut session, &shell, &camera, false);
    assert_eq!(
        last_upload(&uploads, w, h).unwrap(),
        expected.rgb.as_slice()
    );
}

fn key(key: egui::Key, modifiers: egui::Modifiers) -> Vec<Event> {
    vec![Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }]
}

#[test]
fn the_file_shortcuts_ask_the_host_and_do_nothing_themselves() {
    let mut session = part();
    let mut shell = Shell::new();
    let mut window = Window::new(1000.0, 700.0);
    window.idle(&mut shell, &mut session, 2);
    assert_eq!(shell.take_file_action(), None);
    let before = session.scene_revision();
    let ctrl = egui::Modifiers::COMMAND;
    for (pressed, modifiers, action) in [
        (egui::Key::O, ctrl, FileAction::Open),
        (egui::Key::S, ctrl, FileAction::Save),
        (
            egui::Key::S,
            ctrl | egui::Modifiers::SHIFT,
            FileAction::SaveAs,
        ),
        (egui::Key::N, ctrl, FileAction::New),
        (egui::Key::I, ctrl, FileAction::Import),
        (egui::Key::E, ctrl, FileAction::Export),
    ] {
        let _ = window.frame(&mut shell, &mut session, key(pressed, modifiers));
        assert_eq!(shell.take_file_action(), Some(action), "{action:?}");
        // Taken once.
        assert_eq!(shell.take_file_action(), None);
        assert_eq!(action.shortcut().map(|s| s.logical_key), Some(pressed));
    }
    // The keys alone, or with another modifier, are not shortcuts.
    for (pressed, modifiers) in [
        (egui::Key::S, egui::Modifiers::NONE),
        (egui::Key::O, egui::Modifiers::SHIFT),
        (egui::Key::S, egui::Modifiers::ALT),
        (egui::Key::P, ctrl),
    ] {
        let _ = window.frame(&mut shell, &mut session, key(pressed, modifiers));
        assert_eq!(shell.take_file_action(), None, "{pressed:?} {modifiers:?}");
    }
    // The shell asked; it did not open, save or change anything.
    assert_eq!(session.scene_revision(), before);
    assert!(session.document_status().unwrap().dirty);

    // Without a document there is nothing to save: those two are not
    // asked for; New and Open are.
    let mut empty = Session::new(Workspace::none());
    let mut shell = Shell::new();
    window.idle(&mut shell, &mut empty, 2);
    for (pressed, modifiers, action) in [
        (egui::Key::S, ctrl, None),
        (egui::Key::S, ctrl | egui::Modifiers::SHIFT, None),
        (egui::Key::O, ctrl, Some(FileAction::Open)),
        (egui::Key::N, ctrl, Some(FileAction::New)),
        (egui::Key::E, ctrl, None),
        (egui::Key::I, ctrl, Some(FileAction::Import)),
    ] {
        let _ = window.frame(&mut shell, &mut empty, key(pressed, modifiers));
        assert_eq!(shell.take_file_action(), action, "{pressed:?}");
    }
    // What the menu shows.
    let labels: Vec<&str> = FileAction::ALL.iter().map(|a| a.label()).collect();
    assert_eq!(
        labels,
        [
            "New Part",
            "New Assembly",
            "Open…",
            "Open for editing",
            "Save",
            "Save As…",
            "Import…",
            "Export…",
            "Export as Jig…"
        ]
    );
    // No two entries share a shortcut, and none is Close's (Ctrl+W); the
    // Jig's two entries have none.
    let mut shortcuts: Vec<String> = FileAction::ALL
        .iter()
        .filter_map(|a| a.shortcut())
        .map(|s| format!("{s:?}"))
        .collect();
    assert_eq!(shortcuts.len(), 7, "with New Assembly's Ctrl+Shift+N");
    shortcuts.push(format!(
        "{:?}",
        egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::W)
    ));
    let count = shortcuts.len();
    shortcuts.sort();
    shortcuts.dedup();
    assert_eq!(shortcuts.len(), count, "{shortcuts:?}");
    shell.request_file_action(FileAction::SaveAs);
    assert_eq!(shell.take_file_action(), Some(FileAction::SaveAs));
}

#[test]
fn clicking_file_then_an_entry_asks_for_that_action() {
    let mut session = part();
    let mut shell = Shell::new();
    let mut window = Window::new(1000.0, 700.0);
    window.idle(&mut shell, &mut session, 2);
    let click = |window: &mut Window, shell: &mut Shell, session: &mut Session, at: Pos2| {
        let _ = window.frame(shell, session, vec![Event::PointerMoved(at)]);
        window.button(shell, session, at, PointerButton::Primary, true);
        window.button(shell, session, at, PointerButton::Primary, false);
        window.idle(shell, session, 2);
    };
    // "File" is the first thing in the bar at the top.
    click(&mut window, &mut shell, &mut session, pos2(20.0, 10.0));
    assert_eq!(shell.take_file_action(), None, "the menu is only open");
    let escape = |window: &mut Window, shell: &mut Shell, session: &mut Session| {
        let _ = window.frame(
            shell,
            session,
            key(egui::Key::Escape, egui::Modifiers::NONE),
        );
        window.idle(shell, session, 2);
    };
    escape(&mut window, &mut shell, &mut session);
    // Its entries are below it, a row each: find what each asks for by
    // opening the menu and clicking the rows down it, one at a time (a
    // click anywhere in the menu, a separator too, closes it).
    let mut found = Vec::new();
    for row in 0..32 {
        click(&mut window, &mut shell, &mut session, pos2(20.0, 10.0));
        let at = pos2(40.0, 30.0 + 10.0 * row as f32);
        click(&mut window, &mut shell, &mut session, at);
        if let Some(action) = shell.take_file_action() {
            found.push(action);
        }
        // Close, below the file actions, asks for a tab's action.
        let _ = shell.take_doc_action();
        escape(&mut window, &mut shell, &mut session);
    }
    found.dedup();
    // The Jig's entries are greyed out for a model without public
    // parameters: Open for editing is for a Jig in use, Export as Jig for
    // a model with public parameters.
    let offered: Vec<FileAction> = FileAction::ALL
        .into_iter()
        .filter(|a| !matches!(a, FileAction::OpenForEditing | FileAction::ExportJig))
        .collect();
    assert_eq!(found, offered, "the entries, from the top down");
}

#[test]
fn the_title_names_the_file_and_marks_unsaved_changes() {
    use craft3d_engine::DocumentStatus;
    let status = |path: Option<&str>, dirty: bool| DocumentStatus {
        index: 0,
        id: 1,
        open: 1,
        path: path.map(str::to_string),
        file: None,
        title: None,
        assembly: false,
        dirty,
        close_asks: dirty,
        jig_in_use: false,
        undo: None,
        redo: None,
        length_unit: "mm",
        angle_unit: "deg",
    };
    let title = |path: Option<&str>, dirty: bool| window_title(Some(&status(path, dirty)));
    assert_eq!(window_title(None), "3DCraft");
    assert_eq!(title(None, false), "untitled – 3DCraft");
    assert_eq!(title(None, true), "untitled* – 3DCraft");
    assert_eq!(
        title(Some("/home/me/parts/gear.3dcraft"), false),
        "gear.3dcraft – 3DCraft"
    );
    assert_eq!(
        title(Some("/home/me/parts/gear.3dcraft"), true),
        "gear.3dcraft* – 3DCraft"
    );
    assert_eq!(
        title(Some("work/gear.3dcraft"), true),
        "gear.3dcraft* – 3DCraft"
    );
    assert_eq!(
        title(Some(r"C:\parts\gear.3dcraft"), false),
        "gear.3dcraft – 3DCraft"
    );
    assert_eq!(title(Some("gear.3dcraft"), false), "gear.3dcraft – 3DCraft");
    // Odd paths still give a title, on one line and not endless.
    assert_eq!(title(Some(""), true), "untitled* – 3DCraft");
    assert_eq!(title(Some("/"), false), "untitled – 3DCraft");
    assert_eq!(title(Some("dir/"), false), "dir – 3DCraft");
    assert_eq!(title(Some("a\nb.3dcraft"), false), "ab.3dcraft – 3DCraft");
    let long = title(Some(&"x".repeat(5000)), true);
    assert_eq!(long.chars().count(), 120 + "* – 3DCraft".chars().count());
    // An example from the start screen, without a file: by its title; a
    // file's name goes first.
    let titled = |path: Option<&str>, dirty: bool| DocumentStatus {
        title: Some("Moka pot".to_string()),
        ..status(path, dirty)
    };
    assert_eq!(
        window_title(Some(&titled(None, false))),
        "Moka pot – 3DCraft"
    );
    assert_eq!(
        window_title(Some(&titled(None, true))),
        "Moka pot* – 3DCraft"
    );
    assert_eq!(
        window_title(Some(&titled(Some("/a/pot.3dcraft"), false))),
        "pot.3dcraft – 3DCraft"
    );

    // On a real session it follows the active document. A document that
    // was never saved is not on disk, so the engine counts it as changed
    // from the moment it is made. (Saved documents: apps/craft3d, files.)
    let mut session = Session::new(Workspace::none());
    assert_eq!(window_title(session.document_status().as_ref()), "3DCraft");
    run(&mut session, "file.new", json!({}));
    assert!(session.document_status().unwrap().dirty);
    assert_eq!(
        window_title(session.document_status().as_ref()),
        "untitled* – 3DCraft"
    );
    run(&mut session, "file.close", json!({"discard": true}));
    assert_eq!(window_title(session.document_status().as_ref()), "3DCraft");
}

/// The viewport shows the scene at the display quality (finer than the
/// automatic tolerance `view.render` uses); `ui.set` changes the quality
/// and the picture follows; a quality that is not one is refused and
/// changes nothing; `ui.screenshot` renders the scene the viewport shows.
#[test]
fn the_viewport_asks_for_the_display_quality_and_ui_set_changes_it() {
    let mut session = part();
    // A ball, so that the finest mesh is well over the smallest budget.
    run(
        &mut session,
        "feature.sphere",
        json!({"name": "Ball", "radius": 20, "position": [100, 0, 0]}),
    );
    let mut shell = Shell::new();
    let mut window = Window::new(1000.0, 700.0);
    window.idle(&mut shell, &mut session, 2);
    let automatic = session.scene(None).unwrap().unwrap();
    let shown = shell.sync(&mut session).unwrap();
    assert_eq!(shell.scene_quality(), SceneQuality::display());
    // The display rule's size is the area of its quick first picture's
    // mesh, which is the exact area (the automatic tolerance's size)
    // within that picture's tolerance; the tolerance is on the nearest
    // rung of the engine's ladder, within 4.5 % (2026-10-19).
    let size = automatic.tolerance / AUTOMATIC_RELATIVE_TOLERANCE;
    assert!(
        (shown.tolerance - size * DISPLAY_RELATIVE_TOLERANCE).abs() <= 0.0443 * shown.tolerance
    );
    assert!(!shown.coarsened);
    assert!(shown.triangles() > automatic.triangles());
    assert!(shown.triangles() > 1000);
    assert_eq!(shell.render_stats().unwrap().triangles, shown.triangles());
    let call = |shell: &mut Shell, session: &mut Session, method: &str, params: Value| {
        shell
            .handle_method(session, method, Some(&params))
            .unwrap_or_else(|| panic!("{method} is not a ui method"))
    };
    let got = call(&mut shell, &mut session, "ui.get", json!({})).unwrap();
    assert_eq!(
        got["state"]["quality"],
        json!({"chordal": DISPLAY_RELATIVE_TOLERANCE, "maxTriangles": DISPLAY_MAX_TRIANGLES, "angle": DISPLAY_MAX_ANGLE_DEGREES})
    );
    assert_eq!(got["scene"]["tolerance"], shown.tolerance);
    assert_eq!(got["scene"]["coarsened"], false);
    assert_eq!(got["scene"]["triangles"], shown.triangles());

    // The automatic tolerance as the display quality: the automatic
    // scene's tolerance (within the first picture's measure of the size,
    // on the display rule's ladder: within 4.5 %, 2026-10-19), the scene
    // at that tolerance bit for bit, and a new picture of it.
    let rendered = shell.render_count();
    let set = call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"quality": {"chordal": AUTOMATIC_RELATIVE_TOLERANCE, "angle": 0}}),
    )
    .unwrap();
    assert_eq!(
        set["state"]["quality"],
        json!({"chordal": AUTOMATIC_RELATIVE_TOLERANCE, "maxTriangles": DISPLAY_MAX_TRIANGLES, "angle": 0.0})
    );
    window.idle(&mut shell, &mut session, 2);
    let coarse = shell.sync(&mut session).unwrap();
    assert!((coarse.tolerance - automatic.tolerance).abs() <= 0.0443 * automatic.tolerance);
    assert_eq!(
        coarse.mesh,
        session.scene(Some(coarse.tolerance)).unwrap().unwrap().mesh
    );
    assert!(
        shell.render_count() > rendered,
        "the picture follows the quality"
    );
    assert_eq!(shell.render_stats().unwrap().triangles, coarse.triangles());

    // Only the angle changes: another mesh at the same tolerance, and the
    // picture follows it (the viewer keys what it uploaded by both).
    let rendered = shell.render_count();
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"quality": {"angle": 5}}),
    )
    .unwrap();
    window.idle(&mut shell, &mut session, 2);
    let round = shell.sync(&mut session).unwrap();
    assert_eq!(round.tolerance, coarse.tolerance);
    assert!(round.triangles() > coarse.triangles());
    assert!(
        shell.render_count() > rendered,
        "the picture follows the angle"
    );
    assert_eq!(shell.render_stats().unwrap().triangles, round.triangles());
    let got = call(&mut shell, &mut session, "ui.get", json!({})).unwrap();
    assert_eq!(got["scene"]["angle"].as_f64().map(f64::round), Some(5.0));

    // The smallest budget: coarsened towards the first picture's
    // tolerance, and `ui.get` says so.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"quality": {"chordal": DISPLAY_RELATIVE_TOLERANCE, "maxTriangles": 1000}}),
    )
    .unwrap();
    window.idle(&mut shell, &mut session, 2);
    let fitted = shell.sync(&mut session).unwrap();
    assert!(fitted.coarsened);
    let first = session.scene_with(SceneQuality::Preview).unwrap().unwrap();
    assert!(fitted.tolerance > shown.tolerance && fitted.tolerance <= first.tolerance);
    let got = call(&mut shell, &mut session, "ui.get", json!({})).unwrap();
    assert_eq!(got["scene"]["coarsened"], true);
    assert_eq!(got["state"]["quality"]["maxTriangles"], 1000);

    // Garbage is refused and changes nothing.
    let before = shell.state.clone();
    for bad in [
        json!({"quality": 3}),
        json!({"quality": "fine"}),
        json!({"quality": {"chordal": "fine"}}),
        json!({"quality": {"maxTriangles": -5}}),
        json!({"quality": {"maxTriangles": 2.5}}),
        json!({"quality": {"angle": "round"}}),
        json!({"quality": {"level": 2}}),
        json!({"quality": {"chordal": 1e-4}, "bogus": true}),
    ] {
        let error = call(&mut shell, &mut session, "ui.set", bad.clone()).unwrap_err();
        assert!(error.starts_with("ui.set: "), "{bad}: {error}");
        assert_eq!(shell.state, before, "{bad} changed the state");
    }
    assert!(std::ptr::eq(
        shell.sync(&mut session).unwrap().as_ref(),
        fitted.as_ref()
    ));

    // The screenshot is of the scene the viewport shows.
    let shot = call(
        &mut shell,
        &mut session,
        "ui.screenshot",
        json!({"width": 64, "height": 48}),
    )
    .unwrap();
    assert_eq!(shot["triangles"], fitted.triangles());
}

/// A plate extruded from a sketch with a rod cut out of it, a suppressed
/// block with the rollback marker on it and a block after that; then a
/// side of the plate's outline deleted: the plate fails, and the cut that
/// is made from it fails naming the plate as the cause.
fn upstream_failure() -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "sketch.create", json!({"name": "Base"}));
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Base", "closed": true, "points": [[0, 0], [40, 0], [40, 20], [0, 20]]}),
    );
    run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Base", "distance": 5, "name": "Plate"}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": 2, "height": 20, "position": [30, 10, -2], "name": "Rod"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Plate", "tool": "Rod", "name": "Hole"}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"size": [1, 1, 1], "position": [100, 0, 0], "name": "Spare"}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"size": [1, 1, 1], "position": [120, 0, 0], "name": "After"}),
    );
    run(&mut s, "feature.suppress", json!({"feature": "Spare"}));
    run(&mut s, "history.rollback", json!({"feature": "Spare"}));
    // A side of the outline deleted: the profile is open, the plate fails.
    run(
        &mut s,
        "sketch.delete",
        json!({"sketch": "Base", "entity": "e5"}),
    );
    s
}

#[test]
fn the_tree_shows_failed_suppressed_and_rolled_back_features_and_dims_what_a_failure_skips() {
    let mut session = upstream_failure();
    let inspected = run(&mut session, "document.inspect", json!({}));
    let mut shell = Shell::new();
    let mut window = Window::new(1000.0, 700.0);
    window.idle(&mut shell, &mut session, 3);
    let rows = shell.tree_rows().to_vec();
    let states: Vec<(&str, FeatureState)> =
        rows.iter().map(|r| (r.name.as_str(), r.state)).collect();
    assert_eq!(
        states,
        [
            ("Base", FeatureState::Ok),
            ("Plate", FeatureState::Failed),
            ("Rod", FeatureState::Ok),
            ("Hole", FeatureState::Failed),
            ("Spare", FeatureState::Suppressed),
            ("After", FeatureState::RolledBack),
        ],
        "{inspected}"
    );
    // The tree says what document.inspect says: the reason, and the cause.
    for (row, entry) in rows.iter().zip(inspected["features"].as_array().unwrap()) {
        assert_eq!(
            row.error.as_deref(),
            entry["error"].as_str(),
            "{}",
            row.name
        );
        assert_eq!(
            row.caused_by.as_ref().map(|(id, _)| id.as_str()),
            entry["causedBy"].as_str(),
            "{}",
            row.name
        );
    }
    let hole = &rows[3];
    assert_eq!(
        hole.caused_by,
        Some(("F2".to_string(), "Plate".to_string())),
        "the hole failed because the plate did"
    );
    assert!(rows[1].caused_by.is_none(), "the plate is the one to fix");
    assert_eq!(rows[4].status, "suppressed");
    assert_eq!(rows[5].status, "rolled back");

    // ui.get gives an agent the same.
    let got = shell
        .handle_method(&mut session, "ui.get", None)
        .unwrap()
        .unwrap();
    let features = got["features"].as_array().unwrap();
    let summary: Vec<(String, String, Value)> = features
        .iter()
        .map(|f| {
            (
                f["name"].as_str().unwrap().to_string(),
                f["state"].as_str().unwrap().to_string(),
                f["causedBy"].clone(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("Base".into(), "ok".into(), Value::Null),
            ("Plate".into(), "failed".into(), Value::Null),
            ("Rod".into(), "ok".into(), Value::Null),
            (
                "Hole".into(),
                "failed".into(),
                json!({"id": "F2", "name": "Plate"})
            ),
            ("Spare".into(), "suppressed".into(), Value::Null),
            ("After".into(), "rolledBack".into(), Value::Null),
        ]
    );
    assert!(features[3]["error"].as_str().unwrap().contains("Plate"));

    // The side panel (2026-11-02, the approved design: "a failure is a
    // reason, not a cascade"): the hole, made from the plate, is not
    // rebuilt and is dimmed, with no link of its own; the plate is the
    // failed row, with Show where. A click on the plate's row selects it.
    // (Before, a "caused by" link under the hole led there: replaced with
    // the dimming, which `side_panel_tabs.rs` pins on the housing.)
    use craft3d_ui_egui::shell::sidepanel::features::{failed_itself, not_rebuilt};
    assert!(shell.cause_hits().is_empty());
    assert!(not_rebuilt(&rows[3]));
    assert!(failed_itself(&rows[1]));
    assert!(
        shell
            .side_hits()
            .targets
            .iter()
            .any(|(t, _)| t == "show where:F2")
    );
    let at = shell
        .side_hits()
        .rows
        .iter()
        .find(|(id, _)| id == "F2")
        .unwrap()
        .1
        .center();
    assert!(at.x < shell.viewport_rect().unwrap().left());
    let _ = window.frame(&mut shell, &mut session, vec![Event::PointerMoved(at)]);
    window.button(&mut shell, &mut session, at, PointerButton::Primary, true);
    window.button(&mut shell, &mut session, at, PointerButton::Primary, false);
    window.idle(&mut shell, &mut session, 1);
    assert_eq!(shell.state.selected_feature.as_deref(), Some("F2"));
    let selected: Vec<bool> = shell.tree_rows().iter().map(|r| r.selected).collect();
    assert_eq!(selected, [false, true, false, false, false, false]);
    let got = shell
        .handle_method(&mut session, "ui.get", None)
        .unwrap()
        .unwrap();
    assert_eq!(got["state"]["selectedFeature"], "F2");
    assert_eq!(got["features"][1]["selected"], true);

    // Repaired (the sketch closed again), nothing fails and the link goes.
    run(&mut session, "edit.undo", json!({}));
    window.idle(&mut shell, &mut session, 2);
    assert!(shell.cause_hits().is_empty());
    assert!(shell.tree_rows().iter().all(|r| r.caused_by.is_none()));
    assert_eq!(shell.tree_rows()[1].state, FeatureState::Ok);
}

/// The marks of the tree are glyphs the default fonts have: a missing one
/// would be drawn as the replacement box. (`has_glyph` answers false for
/// anything in the face that holds the replacement glyph, so the glyph
/// each mark is drawn with is compared with the one of a character no
/// font has.)
#[test]
fn the_tree_marks_are_in_the_fonts() {
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
    output.textures_delta.clear();
    let body = egui::FontId::proportional(14.0);
    let glyph = |c: char| {
        ctx.fonts_mut(|f| {
            let galley = f.layout_no_wrap(c.to_string(), body.clone(), egui::Color32::WHITE);
            galley.rows[0].glyphs[0].uv_rect
        })
    };
    let missing = glyph('\u{10FFFD}');
    // The check can tell: U+25CF, the round dot the tree drew before,
    // is not in the default fonts and came out as the box.
    assert_eq!(glyph('●'), missing);
    for mark in craft3d_ui_egui::shell::TREE_MARKS {
        assert_ne!(glyph(mark), missing, "{mark}");
    }
}

/// A feature switched off by its condition is a row of its own state,
/// `inactive`, with its condition: greyed in the window, said by `ui.get`.
/// Its tooltip says what it passes on, from the regeneration (a block
/// makes no body), and so does the row of a fillet that is on but whose
/// edges are not there (it makes no body, and lists what it left out).
#[test]
fn the_tree_shows_an_inactive_feature_with_its_condition() {
    let mut session = part();
    run(
        &mut session,
        "variable.set",
        json!({"name": "lid", "expr": "0"}),
    );
    let lid = run(
        &mut session,
        "feature.block",
        json!({"size": [1, 1, 1], "position": [100, 0, 0], "name": "Lid"}),
    )["id"]
        .as_str()
        .unwrap()
        .to_string();
    run(
        &mut session,
        "feature.fillet",
        json!({"edges": [[format!("{lid}:face(1)"), format!("{lid}:face(2)")]], "radius": 0.1,
               "name": "Soft"}),
    );
    run(
        &mut session,
        "feature.when",
        json!({"feature": "Lid", "expr": "lid"}),
    );
    let mut shell = Shell::new();
    let mut window = Window::new(1000.0, 700.0);
    window.idle(&mut shell, &mut session, 3);
    let rows = shell.tree_rows();
    let row = rows[rows.len() - 2].clone();
    assert_eq!(row.state, FeatureState::Inactive);
    assert_eq!(row.status, "inactive");
    assert_eq!(row.when.as_deref(), Some("lid"));
    assert_eq!(
        row.passes.as_deref(),
        Some("Makes no body: `Lid` is inactive (its condition `lid` is 0).")
    );
    let soft = rows.last().unwrap().clone();
    assert_eq!(soft.state, FeatureState::Ok);
    let passes = soft.passes.clone().unwrap_or_default();
    assert!(
        passes.starts_with("Makes no body: `Lid` is inactive")
            && passes.contains(&format!(
                "Left out, not there: {lid}:face(1) / {lid}:face(2)"
            )),
        "{passes}"
    );
    let got = shell
        .handle_method(&mut session, "ui.get", None)
        .unwrap()
        .unwrap();
    let features = got["features"].as_array().unwrap();
    let last = features[features.len() - 2].clone();
    assert_eq!(last["state"], "inactive", "{last}");
    assert_eq!(last["when"], "lid", "{last}");
    assert_eq!(last["zoomable"], false, "{last}");
    assert_eq!(features.last().unwrap()["passes"], json!(passes));
    run(
        &mut session,
        "variable.set",
        json!({"name": "lid", "expr": "1"}),
    );
    window.idle(&mut shell, &mut session, 2);
    let rows = shell.tree_rows();
    assert_eq!(rows[rows.len() - 2].state, FeatureState::Ok);
    assert_eq!(rows.last().unwrap().passes, None);
}
