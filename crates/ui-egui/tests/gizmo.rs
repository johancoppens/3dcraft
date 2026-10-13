//! The orbit gizmo in the corner of the viewport: synthetic clicks and
//! drags on it, and the camera after each. The six standard views are
//! compared with `CameraState::set_view` (the table `ui.set` uses), a drag
//! with `CameraState::orbit` (what a drag in the viewport does), and
//! nothing done on the gizmo or on the buttons under it is a pick.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::state::{CameraState, ProjectionKind};
use craft3d_ui_egui::{RendererChoice, Shell, gizmo};
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
        for _ in 0..3 {
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

/// Where an end of the gizmo is in the window: the shell's own layout
/// (the arm leaves room for the disc at its end).
fn place(shell: &Shell, offset: [f64; 2]) -> Pos2 {
    let rect = shell.gizmo_rect().unwrap();
    let arm = rect.width() * 0.5 - shell.tokens.gizmo_end_radius - 1.0;
    rect.center() + vec2(offset[0] as f32, offset[1] as f32) * arm
}

fn end(shell: &Shell, label: &str) -> gizmo::GizmoEnd {
    *gizmo::ends(&shell.state.camera)
        .iter()
        .find(|e| e.label() == label)
        .unwrap()
}

#[test]
fn the_gizmo_is_in_the_corner_of_the_viewport() {
    let (_session, shell, _window) = started();
    let viewport = shell.viewport_rect().unwrap();
    let rect = shell.gizmo_rect().unwrap();
    assert!(viewport.contains_rect(rect), "{rect:?} in {viewport:?}");
    assert_eq!(rect.width(), shell.tokens.gizmo_size);
    assert_eq!(rect.height(), shell.tokens.gizmo_size);
    // Bottom right, clear of the edge, with the row of buttons under it.
    assert!((viewport.right() - rect.right() - shell.tokens.inset).abs() < 0.5);
    let below = viewport.bottom() - rect.bottom();
    assert!((30.0..50.0).contains(&below), "{below}");
}

/// Evidence for the developer: the viewport's picture with the gizmo inked
/// on it where the shell lays it out, from the same numbers the shell
/// paints from (`gizmo::ends`, the tokens). It is a drawing made here, not
/// the window's own pixels: egui's widgets are not in `ui.screenshot`.
#[test]
fn a_drawing_of_the_gizmo_as_it_is_laid_out() {
    use craft3d_ui_egui::viewport::draw_line;
    let (mut session, mut shell, mut window) = started();
    for (name, yaw, pitch) in [("iso", -45.0, 29.5), ("turned", 150.0, -20.0)] {
        shell
            .handle_method(
                &mut session,
                "ui.set",
                Some(&json!({"camera": {"yaw": yaw, "pitch": pitch}})),
            )
            .unwrap()
            .unwrap();
        window.frame(&mut shell, &mut session, Vec::new());
        let [width, height] = shell.viewport_size();
        let shot = shell
            .handle_method(
                &mut session,
                "ui.screenshot",
                Some(&json!({"width": width, "height": height, "scale": 1.0,
                             "path": null})),
            )
            .unwrap()
            .unwrap();
        assert_eq!(shot["width"], width);
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
        let rect = shell.gizmo_rect().unwrap();
        let tokens = shell.tokens;
        let arm = f64::from(rect.width() * 0.5 - tokens.gizmo_end_radius - 1.0);
        let center = [
            f64::from(rect.center().x - viewport.left()),
            f64::from(rect.center().y - viewport.top()),
        ];
        let ends = gizmo::ends(&shell.state.camera);
        assert_eq!(ends.len(), 6);
        for end in &ends {
            let at = [
                center[0] + end.offset[0] * arm,
                center[1] + end.offset[1] * arm,
            ];
            let ink = tokens.axes[end.axis];
            if end.positive {
                draw_line(&mut rgb, width, height, center, at, 2.0, ink, 255);
                let r = f64::from(tokens.gizmo_end_radius) * 2.0;
                draw_line(&mut rgb, width, height, at, at, r, ink, 255);
            } else {
                let r = f64::from(tokens.gizmo_end_radius) * 1.44;
                draw_line(&mut rgb, width, height, at, at, r, ink, 120);
            }
        }
        let dir =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/ui");
        std::fs::create_dir_all(&dir).unwrap();
        let png = craft3d_render::encode_png(width, height, &rgb).unwrap();
        std::fs::write(dir.join(format!("gizmo-drawing-{name}.png")), png).unwrap();
    }
}

#[test]
fn a_click_on_an_end_of_the_gizmo_gives_that_view_and_the_opposite_one_the_second_time() {
    let (mut session, mut shell, mut window) = started();
    for (label, view, other) in [
        ("X", "right", "left"),
        ("Z", "top", "bottom"),
        ("-Y", "front", "back"),
        ("Y", "back", "front"),
        ("-X", "left", "right"),
        ("-Z", "bottom", "top"),
    ] {
        // From a corner from which that end is in front of the others.
        let positive = !label.starts_with('-');
        let (yaw, pitch) = match (label.trim_start_matches('-'), positive) {
            ("X", true) | ("Y", true) => (40.0, 25.0),
            ("X", false) | ("Y", false) => (-140.0, 25.0),
            (_, true) => (40.0, 35.0),
            (_, false) => (40.0, -35.0),
        };
        shell
            .handle_method(
                &mut session,
                "ui.set",
                Some(&json!({"camera": {"yaw": yaw, "pitch": pitch, "distance": 900.0}})),
            )
            .unwrap()
            .unwrap();
        window.frame(&mut shell, &mut session, Vec::new());
        assert!(!shell.state.camera.auto_fit);
        let target = end(&shell, label);
        assert!(target.toward > 0.0, "{label} faces the viewer from here");
        let at = place(&shell, target.offset);
        window.click(&mut shell, &mut session, at);
        let camera = shell.state.camera;
        let mut expected = CameraState::default();
        expected.set_view(view).unwrap();
        assert_eq!(
            (camera.yaw, camera.pitch),
            (expected.yaw, expected.pitch),
            "{label}"
        );
        assert!(gizmo::faces(&camera, view), "{label}");
        // The model is framed again, as after a view button: nearer than
        // the 900 it was put at, and following the model.
        assert!(
            camera.auto_fit && camera.distance < 600.0,
            "{label}: {camera:?}"
        );
        // The end now faces the viewer, in the middle of the gizmo; a
        // click on it there gives the view from the other side.
        let faced = end(&shell, label);
        assert!(faced.offset[0].hypot(faced.offset[1]) < 1e-9 && faced.toward > 0.999);
        let at = place(&shell, faced.offset);
        window.click(&mut shell, &mut session, at);
        let mut opposite = CameraState::default();
        opposite.set_view(other).unwrap();
        assert_eq!(
            (shell.state.camera.yaw, shell.state.camera.pitch),
            (opposite.yaw, opposite.pitch),
            "{label} again"
        );
        // Nothing of this was a click on the model behind the gizmo.
        assert_eq!(shell.state.picked, None, "{label}");
        assert_eq!(shell.state.highlight, None, "{label}");
    }
}

#[test]
fn a_drag_on_the_gizmo_orbits_as_a_drag_in_the_viewport_does() {
    let (mut session, mut shell, mut window) = started();
    let before = shell.state.camera;
    let rect = shell.gizmo_rect().unwrap();
    let from = rect.center() + vec2(-8.0, 11.0);
    window.frame(&mut shell, &mut session, vec![Event::PointerMoved(from)]);
    window.button(&mut shell, &mut session, from, true);
    let total = vec2(37.0, -14.0);
    for step in 1..=6 {
        let at = from + total * (step as f32 / 6.0);
        window.frame(&mut shell, &mut session, vec![Event::PointerMoved(at)]);
    }
    // The pointer may leave the gizmo while it drags: the drag is its.
    let mut expected = before;
    expected.orbit(f64::from(total.x), f64::from(total.y));
    assert_eq!(shell.state.camera, expected);
    assert_ne!((expected.yaw, expected.pitch), (before.yaw, before.pitch));
    window.button(&mut shell, &mut session, from + total, false);
    for _ in 0..3 {
        window.frame(&mut shell, &mut session, Vec::new());
    }
    assert_eq!(shell.state.camera, expected);
    assert_eq!(
        (expected.target, expected.distance),
        (before.target, before.distance)
    );
    assert_eq!(
        shell.state.picked, None,
        "a drag on the gizmo picks nothing"
    );

    // The same drag in the viewport, away from the gizmo: the same turn.
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

    // A click on the gizmo's backdrop, on no end: nothing, and no pick.
    window.frame(&mut shell, &mut session, Vec::new());
    let camera = shell.state.camera;
    let rect = shell.gizmo_rect().unwrap();
    let ends = gizmo::ends(&camera);
    let arm = f64::from(rect.width() * 0.5 - shell.tokens.gizmo_end_radius - 1.0);
    let free = (0..360)
        .step_by(7)
        .map(|deg| {
            let a = f64::from(deg).to_radians();
            [0.55 * a.cos(), 0.55 * a.sin()]
        })
        .find(|at| {
            ends.iter()
                .all(|e| (e.offset[0] - at[0]).hypot(e.offset[1] - at[1]) * arm > 16.0)
        })
        .expect("a spot of the gizmo between its ends");
    let at = place(&shell, free);
    window.click(&mut shell, &mut session, at);
    assert_eq!(shell.state.camera, camera);
    assert_eq!(shell.state.picked, None);
}

#[test]
fn fit_the_corner_view_and_the_projection_are_still_a_click_away() {
    let (mut session, mut shell, mut window) = started();
    let viewport = shell.viewport_rect().unwrap();
    // The row under the gizmo, from the right: every 6 points.
    let spots: Vec<Pos2> = (4..260)
        .step_by(6)
        .map(|dx| {
            pos2(
                viewport.right() - shell.tokens.inset - dx as f32,
                viewport.bottom() - 20.0,
            )
        })
        .collect();
    let set = |shell: &mut Shell, session: &mut Session, patch: Value| {
        shell
            .handle_method(session, "ui.set", Some(&patch))
            .unwrap()
            .unwrap();
    };
    // A click on `at`; whether `done` holds afterwards. A click that did
    // it was on a button, and so no pick (the spots between the buttons
    // are the viewport's, and a click there is one).
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
    // The projection: some spot of the row toggles it, there and back.
    let toggle = spots
        .iter()
        .copied()
        .find(|&at| {
            press(&mut shell, &mut session, at, &|c| {
                c.projection == ProjectionKind::Orthographic
            })
        })
        .expect("the projection button");
    assert!(press(&mut shell, &mut session, toggle, &|c| {
        c.projection == ProjectionKind::Perspective
    }));
    // The corner view: from another view, some spot gives iso, framed.
    let mut iso = CameraState::default();
    iso.set_view("iso").unwrap();
    let found = spots.iter().copied().any(|at| {
        set(
            &mut shell,
            &mut session,
            json!({"view": "front", "camera": {"distance": 800.0, "projection": "perspective"}}),
        );
        press(&mut shell, &mut session, at, &|c| {
            (c.yaw, c.pitch) == (iso.yaw, iso.pitch) && c.auto_fit
        })
    });
    assert!(found, "the iso button");
    // Fit: some spot frames the model without turning the camera.
    let found = spots.iter().copied().any(|at| {
        set(
            &mut shell,
            &mut session,
            json!({"camera": {"yaw": 20.0, "pitch": 10.0, "distance": 800.0,
                              "projection": "perspective"}}),
        );
        press(&mut shell, &mut session, at, &|c| {
            (c.yaw, c.pitch) == (20.0, 10.0)
                && c.auto_fit
                && c.distance < 600.0
                && c.projection == ProjectionKind::Perspective
        })
    });
    assert!(found, "the fit button");
    // F over the viewport still fits too, and an agent reaches every view
    // through ui.set, as before.
    for (view, ..) in craft3d_ui_egui::state::VIEWS {
        set(&mut shell, &mut session, json!({"view": view}));
        assert!(
            view == "iso" || gizmo::faces(&shell.state.camera, view),
            "{view}"
        );
    }
}

#[test]
fn a_viewport_too_small_for_the_gizmo_has_none_and_nothing_breaks() {
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
        match shell.gizmo_rect() {
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
    assert!(shell.gizmo_rect().is_some());
}
