//! The view cube in the corner of the viewport: synthetic clicks and
//! drags on it, and the camera after each. Its six faces are compared
//! with `CameraState::set_view` (the table `ui.set` uses), the facets of
//! its edges and corners with the direction they stand for, a drag with
//! `CameraState::orbit` (what a drag in the viewport does), the ring
//! around it with the projection, and nothing done on it is a pick.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::state::{CameraState, ProjectionKind};
use craft3d_ui_egui::{RendererChoice, Shell, view_cube};
use egui::{Event, PointerButton, Pos2, pos2, vec2};
use serde_json::{Value, json};

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

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
        json!({"name": "Post", "position": [12, 20, 8], "radius": 6, "height": 30}),
    );
    s
}

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

    fn frame(&mut self, shell: &mut Shell, session: &mut Session, events: Vec<Event>) {
        self.time += 1.0 / 60.0;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, self.size)),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
        output.textures_delta.clear();
    }

    fn button(&mut self, shell: &mut Shell, session: &mut Session, at: Pos2, pressed: bool) {
        self.frame(
            shell,
            session,
            vec![Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }],
        );
    }

    fn click(&mut self, shell: &mut Shell, session: &mut Session, at: Pos2) {
        self.frame(shell, session, vec![Event::PointerMoved(at)]);
        self.button(shell, session, at, true);
        self.button(shell, session, at, false);
        // Long enough for the camera to finish turning (a quarter of a
        // second at 60 frames a second).
        for _ in 0..24 {
            self.frame(shell, session, Vec::new());
        }
    }
}

fn started() -> (Session, Shell, Window) {
    let mut session = part();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1000.0, 700.0);
    for _ in 0..4 {
        window.frame(&mut shell, &mut session, Vec::new());
    }
    (session, shell, window)
}

/// Where a point of the cube's picture is in the window: the shell's own
/// layout (the picture's radius is the place's half side over 1.3).
fn place(shell: &Shell, at: [f64; 2]) -> Pos2 {
    let rect = shell.view_cube_rect().unwrap();
    let arm = rect.width() * 0.5 / 1.3;
    rect.center() + vec2(at[0] as f32, at[1] as f32) * arm
}

fn set(shell: &mut Shell, session: &mut Session, patch: Value) {
    shell
        .handle_method(session, "ui.set", Some(&patch))
        .unwrap()
        .unwrap();
}

#[test]
fn the_cube_is_in_the_corner_of_the_viewport() {
    let (_session, shell, _window) = started();
    let viewport = shell.viewport_rect().unwrap();
    let rect = shell.view_cube_rect().unwrap();
    assert!(viewport.contains_rect(rect), "{rect:?} in {viewport:?}");
    assert_eq!(rect.width(), shell.tokens.view_cube_size);
    assert_eq!(rect.height(), shell.tokens.view_cube_size);
    // Bottom right, clear of the edge, with the row of buttons under it.
    assert!((viewport.right() - rect.right() - shell.tokens.inset).abs() < 0.5);
    let below = viewport.bottom() - rect.bottom();
    assert!((30.0..50.0).contains(&below), "{below}");
}

/// Every one of the 26 facets, in both projections: from a camera that
/// sees it, a click on its middle turns the camera to look from that side,
/// framed; a second click, on the facet now in the middle, gives the
/// other side.
#[test]
fn a_click_on_a_facet_turns_the_camera_to_that_side_and_again_to_the_other() {
    let (mut session, mut shell, mut window) = started();
    let mut clicked = 0;
    for projection in ["perspective", "orthographic"] {
        for side in view_cube::sides() {
            // From a little off that side, so that the facet shows but is
            // not the one faced.
            let (yaw, pitch) = view_cube::angles(side).unwrap();
            let pitch = if pitch > 60.0 {
                pitch - 23.0
            } else {
                pitch + 23.0
            };
            set(
                &mut shell,
                &mut session,
                json!({"camera": {"yaw": yaw + 19.0, "pitch": pitch, "distance": 900.0,
                                  "projection": projection}}),
            );
            window.frame(&mut shell, &mut session, Vec::new());
            assert!(!shell.state.camera.auto_fit);
            let facet = view_cube::facets(&shell.state.camera)
                .into_iter()
                .find(|f| f.side == side)
                .unwrap_or_else(|| panic!("{side:?} is not in sight"));
            let at = place(&shell, facet.center());
            window.click(&mut shell, &mut session, at);
            let camera = shell.state.camera;
            let context = format!("{projection} {side:?}: {camera:?}");
            assert!(view_cube::looks_from(&camera, side), "{context}");
            let (want_yaw, want_pitch) = view_cube::angles(side).unwrap();
            assert!((camera.pitch - want_pitch).abs() < 1e-9, "{context}");
            if side[0] != 0 || side[1] != 0 {
                let off = (camera.yaw - want_yaw + 180.0).rem_euclid(360.0) - 180.0;
                assert!(off.abs() < 1e-9, "{context}");
            }
            // A face is a standard view, as ui.set names it.
            if let Some(name) = view_cube::label_of(side) {
                let mut standard = CameraState::default();
                standard.set_view(name).unwrap();
                assert_eq!((camera.yaw, camera.pitch), (standard.yaw, standard.pitch));
            }
            // Framed: nearer than the 900 it was put at, and following
            // the model again.
            assert!(camera.auto_fit && camera.distance < 600.0, "{context}");
            // The facet now faces the viewer, in the middle of the cube;
            // a click there gives the other side.
            let at = place(&shell, [0.0, 0.0]);
            window.click(&mut shell, &mut session, at);
            assert!(
                view_cube::looks_from(&shell.state.camera, side.map(|c| -c)),
                "{context}, again"
            );
            // Nothing of this was a click on the model behind the cube.
            assert_eq!(shell.state.picked, None, "{context}");
            assert_eq!(shell.state.highlight, None, "{context}");
            clicked += 1;
        }
    }
    assert_eq!(clicked, 52);
}

#[test]
fn the_camera_turns_to_the_view_in_steps_and_another_move_ends_the_turn() {
    let (mut session, mut shell, mut window) = started();
    let from = shell.state.camera;
    let top = view_cube::facets(&from)
        .into_iter()
        .find(|f| f.label() == Some("top"))
        .unwrap();
    let at = place(&shell, top.center());
    window.frame(&mut shell, &mut session, vec![Event::PointerMoved(at)]);
    window.button(&mut shell, &mut session, at, true);
    window.button(&mut shell, &mut session, at, false);
    // On the way: the pitch grows frame by frame towards straight down.
    let mut pitches = vec![shell.state.camera.pitch];
    for _ in 0..8 {
        window.frame(&mut shell, &mut session, Vec::new());
        pitches.push(shell.state.camera.pitch);
    }
    assert!(pitches.windows(2).all(|w| w[0] <= w[1]), "{pitches:?}");
    assert!(
        pitches[4] > from.pitch + 1.0 && pitches[4] < 89.0,
        "{pitches:?}"
    );
    for _ in 0..20 {
        window.frame(&mut shell, &mut session, Vec::new());
    }
    assert_eq!(shell.state.camera.pitch, 90.0);
    let arrived = shell.state.camera;
    // At rest it stays where it is.
    for _ in 0..5 {
        window.frame(&mut shell, &mut session, Vec::new());
    }
    assert_eq!(shell.state.camera, arrived);

    // A turn that ui.set interrupts stops: the camera is what was set.
    let front = view_cube::facets(&shell.state.camera)
        .into_iter()
        .find(|f| f.side == [0, -1, 1])
        .unwrap();
    let at = place(&shell, front.center());
    window.frame(&mut shell, &mut session, vec![Event::PointerMoved(at)]);
    window.button(&mut shell, &mut session, at, true);
    window.button(&mut shell, &mut session, at, false);
    for _ in 0..4 {
        window.frame(&mut shell, &mut session, Vec::new());
    }
    assert_ne!(shell.state.camera.pitch, 90.0, "it is turning");
    set(
        &mut shell,
        &mut session,
        json!({"camera": {"yaw": 10.0, "pitch": 20.0}}),
    );
    for _ in 0..30 {
        window.frame(&mut shell, &mut session, Vec::new());
    }
    assert_eq!(
        (shell.state.camera.yaw, shell.state.camera.pitch),
        (10.0, 20.0)
    );
}

#[test]
fn a_drag_on_the_cube_orbits_as_a_drag_in_the_viewport_does() {
    let (mut session, mut shell, mut window) = started();
    let before = shell.state.camera;
    let rect = shell.view_cube_rect().unwrap();
    let from = rect.center() + vec2(-8.0, 11.0);
    window.frame(&mut shell, &mut session, vec![Event::PointerMoved(from)]);
    window.button(&mut shell, &mut session, from, true);
    let total = vec2(37.0, -14.0);
    for step in 1..=6 {
        let at = from + total * (step as f32 / 6.0);
        window.frame(&mut shell, &mut session, vec![Event::PointerMoved(at)]);
    }
    let mut expected = before;
    expected.orbit(f64::from(total.x), f64::from(total.y));
    assert_eq!(shell.state.camera, expected);
    assert_ne!((expected.yaw, expected.pitch), (before.yaw, before.pitch));
    window.button(&mut shell, &mut session, from + total, false);
    for _ in 0..24 {
        window.frame(&mut shell, &mut session, Vec::new());
    }
    assert_eq!(
        shell.state.camera, expected,
        "a drag is not a click on a facet"
    );
    assert_eq!(shell.state.picked, None, "a drag on the cube picks nothing");

    // The same drag in the viewport, away from the cube: the same turn.
    let viewport = shell.viewport_rect().unwrap();
    let from = viewport.center();
    window.frame(&mut shell, &mut session, vec![Event::PointerMoved(from)]);
    window.button(&mut shell, &mut session, from, true);
    for step in 1..=6 {
        let at = from + total * (step as f32 / 6.0);
        window.frame(&mut shell, &mut session, vec![Event::PointerMoved(at)]);
    }
    window.button(&mut shell, &mut session, from + total, false);
    let mut twice = expected;
    twice.orbit(f64::from(total.x), f64::from(total.y));
    assert_eq!(shell.state.camera, twice);
}

#[test]
fn a_click_on_the_ring_around_the_cube_switches_the_projection() {
    let (mut session, mut shell, mut window) = started();
    assert_eq!(shell.state.camera.projection, ProjectionKind::Perspective);
    let camera = shell.state.camera;
    // On the ring: within the place, beside every facet.
    let ring = [0.0, 1.2];
    assert_eq!(view_cube::facet_at(&camera, ring), None);
    let at = place(&shell, ring);
    assert!(shell.view_cube_rect().unwrap().contains(at));
    window.click(&mut shell, &mut session, at);
    assert_eq!(shell.state.camera.projection, ProjectionKind::Orthographic);
    // Nothing else of the camera moved, and nothing was picked.
    assert_eq!(
        shell.state.camera,
        CameraState {
            projection: ProjectionKind::Orthographic,
            ..camera
        }
    );
    assert_eq!(shell.state.picked, None);
    // The cube is drawn the way the viewport is: its picture changed.
    assert_ne!(
        view_cube::facets(&shell.state.camera),
        view_cube::facets(&camera)
    );
    window.click(&mut shell, &mut session, at);
    assert_eq!(shell.state.camera, camera);
}

#[test]
fn fit_and_the_corner_view_are_still_a_click_away() {
    let (mut session, mut shell, mut window) = started();
    let viewport = shell.viewport_rect().unwrap();
    // The row under the cube, from the right: every 6 points.
    let spots: Vec<Pos2> = (4..120)
        .step_by(6)
        .map(|dx| {
            pos2(
                viewport.right() - shell.tokens.inset - dx as f32,
                viewport.bottom() - 20.0,
            )
        })
        .collect();
    // A click on `at`; whether `done` holds afterwards. A click that did
    // it was on a button, and so no pick.
    let mut press = |shell: &mut Shell,
                     session: &mut Session,
                     at: Pos2,
                     done: &dyn Fn(&CameraState) -> bool| {
        let picked = shell.state.picked.clone();
        window.click(shell, session, at);
        let did = done(&shell.state.camera);
        if did {
            assert_eq!(shell.state.picked, picked, "a click on a button picked");
        }
        did
    };
    let mut iso = CameraState::default();
    iso.set_view("iso").unwrap();
    let found = spots.iter().copied().any(|at| {
        set(
            &mut shell,
            &mut session,
            json!({"view": "front", "camera": {"distance": 800.0}}),
        );
        press(&mut shell, &mut session, at, &|c| {
            (c.yaw, c.pitch) == (iso.yaw, iso.pitch) && c.auto_fit
        })
    });
    assert!(found, "the iso button");
    let found = spots.iter().copied().any(|at| {
        set(
            &mut shell,
            &mut session,
            json!({"camera": {"yaw": 20.0, "pitch": 10.0, "distance": 800.0}}),
        );
        press(&mut shell, &mut session, at, &|c| {
            (c.yaw, c.pitch) == (20.0, 10.0) && c.auto_fit && c.distance < 600.0
        })
    });
    assert!(found, "the fit button");
    // An agent reaches every view and the projection through ui.set, as
    // before.
    for (view, ..) in craft3d_ui_egui::state::VIEWS {
        set(&mut shell, &mut session, json!({"view": view}));
        if let Some(side) = view_cube::sides().find(|&s| view_cube::label_of(s) == Some(view)) {
            assert!(view_cube::looks_from(&shell.state.camera, side), "{view}");
        }
    }
    set(
        &mut shell,
        &mut session,
        json!({"camera": {"projection": "orthographic"}}),
    );
    assert_eq!(shell.state.camera.projection, ProjectionKind::Orthographic);
}

#[test]
fn a_viewport_too_small_for_the_cube_has_none_and_nothing_breaks() {
    let mut session = part();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    for (width, height) in [
        (1000.0, 700.0),
        (260.0, 90.0),
        (30.0, 30.0),
        (1000.0, 700.0),
    ] {
        let mut window = Window::new(width, height);
        for _ in 0..3 {
            window.frame(&mut shell, &mut session, Vec::new());
        }
        let viewport = shell.viewport_rect().unwrap();
        match shell.view_cube_rect() {
            Some(rect) => assert!(viewport.expand(0.5).contains_rect(rect), "{width}x{height}"),
            None => assert!(height < 700.0),
        }
        // Clicks all over it: no panic, whatever is there.
        for (fx, fy) in [(0.99, 0.99), (0.9, 0.8), (0.5, 0.5), (0.95, 0.6)] {
            let at = pos2(
                viewport.left() + viewport.width() * fx,
                viewport.top() + viewport.height() * fy,
            );
            window.click(&mut shell, &mut session, at);
        }
    }
    assert!(shell.view_cube_rect().is_some());
}

/// Evidence for the developer: the viewport's picture with the cube inked
/// on it where the shell lays it out, from the same numbers the shell
/// paints from (`view_cube::facets`, the tokens): outlines only. It is a
/// drawing made here, not the window's own pixels: egui's widgets are not
/// in `ui.screenshot`.
#[test]
fn a_drawing_of_the_cube_as_it_is_laid_out() {
    use craft3d_ui_egui::viewport::draw_line;
    let (mut session, mut shell, mut window) = started();
    for (name, yaw, pitch, projection) in [
        ("iso", -45.0, 29.5, "perspective"),
        ("turned", 150.0, -20.0, "orthographic"),
    ] {
        set(
            &mut shell,
            &mut session,
            json!({"camera": {"yaw": yaw, "pitch": pitch, "projection": projection}}),
        );
        window.frame(&mut shell, &mut session, Vec::new());
        let [width, height] = shell.viewport_size();
        let scene = session.scene_with(shell.scene_quality()).unwrap();
        let picture = craft3d_ui_egui::viewport::picture(
            scene.as_deref(),
            &shell.state.camera,
            craft3d_ui_egui::viewport::Quality {
                width,
                height,
                ssaa: 2,
            },
            &shell.tokens,
            None,
            1.0,
        )
        .unwrap();
        let mut rgb = picture.rgb;
        let viewport = shell.viewport_rect().unwrap();
        let tokens = shell.tokens;
        let facets = view_cube::facets(&shell.state.camera);
        assert!(facets.len() >= 7);
        let at = |p: [f64; 2]| {
            let q = place(&shell, p);
            [
                f64::from(q.x - viewport.left()),
                f64::from(q.y - viewport.top()),
            ]
        };
        for facet in &facets {
            let ink = facet
                .axis()
                .map_or(tokens.text_dim, |axis| tokens.axes[axis]);
            let n = facet.outline.len();
            for k in 0..n {
                let (a, b) = (at(facet.outline[k]), at(facet.outline[(k + 1) % n]));
                draw_line(&mut rgb, width, height, a, b, 1.5, ink, 255);
            }
        }
        for axis in 0..3 {
            let (from, to, _) = view_cube::axis_stick(&shell.state.camera, axis).unwrap();
            draw_line(
                &mut rgb,
                width,
                height,
                at(from),
                at(to),
                2.5,
                tokens.axes[axis],
                255,
            );
        }
        let dir =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/ui");
        std::fs::create_dir_all(&dir).unwrap();
        let png = craft3d_render::encode_png(width, height, &rgb).unwrap();
        std::fs::write(dir.join(format!("view-cube-drawing-{name}.png")), png).unwrap();
    }
}
