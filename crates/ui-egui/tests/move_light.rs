//! The studio look drawn light while the camera moves, where the host asks
//! for it (`Shell::set_light_while_moving`, the browser's page): one frame,
//! no lens, no occlusion, no screen-space passes, at a smaller size on the
//! direct path; the full look again, settling, once the camera has rested
//! for `STILL_DELAY` seconds of input time; never for a picture of the
//! window or a recording; off on the desktop.
//!
//! The direct path's pictures land in `target/evidence/ui/` as
//! `move-light-*.png`. The test of the direct path needs a graphics
//! adapter and fails, with what to install, when there is none (the GPU
//! crate's policy).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_gpu::ADAPTER_HINT;
use craft3d_ui_egui::backend::RendererChoice;
use craft3d_ui_egui::direct::headless_render_state;
use craft3d_ui_egui::{STILL_DELAY, Shell, moving_size};
use egui::{Event, PointerButton, Pos2, vec2};
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
        .unwrap()
        .unwrap_or_else(|e| panic!("{method} {params}: {e}"))
}

/// A plate with a post: something to light and shade.
fn part() -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": ["40 mm", "30 mm", "4 mm"]}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Post", "position": [20, 15, 4], "radius": 5, "height": 12}),
    );
    s
}

/// A window whose clock is the test's: each frame moves it `dt` seconds.
struct Window {
    ctx: egui::Context,
    time: f64,
}

impl Window {
    fn new() -> Self {
        Self {
            ctx: egui::Context::default(),
            time: 0.0,
        }
    }

    fn frame(&mut self, shell: &mut Shell, s: &mut Session, dt: f64, events: Vec<Event>) {
        self.time += dt;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 560.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, s));
        output.textures_delta.clear();
    }

    /// A drag with the primary button from the middle of the viewport by
    /// `by` points, held there (not released), a frame per `dt`.
    fn drag(&mut self, shell: &mut Shell, s: &mut Session, by: egui::Vec2, dt: f64) -> Pos2 {
        let at = shell.viewport_rect().expect("a viewport").center();
        self.frame(shell, s, dt, vec![Event::PointerMoved(at)]);
        self.frame(
            shell,
            s,
            dt,
            vec![Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        for step in 1..=4 {
            let to = at + by * (step as f32 / 4.0);
            self.frame(shell, s, dt, vec![Event::PointerMoved(to)]);
        }
        at
    }

    fn release(&mut self, shell: &mut Shell, s: &mut Session, at: Pos2, dt: f64) {
        self.frame(
            shell,
            s,
            dt,
            vec![
                Event::PointerMoved(at),
                Event::PointerButton {
                    pos: at,
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
}

fn studio(shell: &mut Shell, s: &mut Session) {
    call(
        shell,
        s,
        "ui.set",
        json!({"look": {"mode": "studio", "accumulate": 4,
                        "dof": {"fstop": 1.4, "focus": "centre"}}}),
    );
    shell.prepare_look().unwrap();
}

/// Whether the look the viewport draws with is the light one.
fn draws_light(shell: &Shell) -> bool {
    let look = shell.studio().expect("the studio look, prepared").look;
    look.accumulate == 1 && look.dof.is_none() && !look.ao && !look.ssr && !look.contact_shadows
}

fn interacting(shell: &mut Shell, s: &mut Session) -> bool {
    call(shell, s, "ui.get", json!({}))["viewport"]["interactingLight"]
        .as_bool()
        .expect("viewport.interactingLight")
}

#[test]
fn the_look_is_light_while_the_camera_moves_and_full_once_it_rests() {
    let mut s = part();
    let mut shell = Shell::new();
    shell.set_light_while_moving(true);
    let mut window = Window::new();
    window.frame(&mut shell, &mut s, 0.0, Vec::new());
    studio(&mut shell, &mut s);
    window.frame(&mut shell, &mut s, 0.1, Vec::new());
    assert!(!interacting(&mut shell, &mut s));
    assert!(!draws_light(&shell));
    let still = call(
        &mut shell,
        &mut s,
        "ui.screenshot",
        json!({"width": 200, "height": 150, "scale": 1.0}),
    );

    // A drag turns the camera: light while it lasts, also while the
    // pointer is held still.
    let camera = shell.state.camera;
    let at = window.drag(&mut shell, &mut s, vec2(60.0, 0.0), 1.0 / 60.0);
    assert_ne!(shell.state.camera, camera, "the drag turned nothing");
    assert!(interacting(&mut shell, &mut s));
    assert!(draws_light(&shell));
    window.frame(&mut shell, &mut s, 1.0, Vec::new());
    assert!(draws_light(&shell), "held still mid-drag: still light");

    // ui.screenshot while it moves is the full picture, byte for byte the
    // one of a camera at rest there.
    let moving = call(
        &mut shell,
        &mut s,
        "ui.screenshot",
        json!({"width": 200, "height": 150, "scale": 1.0}),
    );
    assert!(draws_light(&shell), "the screenshot left it light");
    let mut rest = part();
    let mut rest_shell = Shell::new();
    let mut rest_window = Window::new();
    rest_window.frame(&mut rest_shell, &mut rest, 0.0, Vec::new());
    studio(&mut rest_shell, &mut rest);
    rest_shell.state.camera = shell.state.camera;
    rest_window.frame(&mut rest_shell, &mut rest, 0.1, Vec::new());
    let resting = call(
        &mut rest_shell,
        &mut rest,
        "ui.screenshot",
        json!({"width": 200, "height": 150, "scale": 1.0}),
    );
    assert!(
        moving["base64"] == resting["base64"],
        "the screenshot taken while the camera moves is not the full picture"
    );
    assert!(
        moving["base64"] != still["base64"],
        "the camera did not turn"
    );

    // Let go: light for STILL_DELAY more, then the full look.
    window.release(&mut shell, &mut s, at + vec2(60.0, 0.0), 1.0 / 60.0);
    window.frame(&mut shell, &mut s, STILL_DELAY * 0.5, Vec::new());
    assert!(interacting(&mut shell, &mut s), "light until it rested");
    window.frame(&mut shell, &mut s, STILL_DELAY, Vec::new());
    assert!(!interacting(&mut shell, &mut s));
    assert!(!draws_light(&shell));

    // The wheel: light, and full again after the delay.
    window.frame(
        &mut shell,
        &mut s,
        1.0 / 60.0,
        vec![Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, 120.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert!(interacting(&mut shell, &mut s), "the wheel zooms lightly");
    for _ in 0..30 {
        window.frame(&mut shell, &mut s, 0.05, Vec::new());
    }
    assert!(!interacting(&mut shell, &mut s));

    // Recorded: the full look while the camera moves.
    shell.set_recording(true);
    let at = window.drag(&mut shell, &mut s, vec2(0.0, 40.0), 1.0 / 60.0);
    assert!(!interacting(&mut shell, &mut s));
    assert!(!draws_light(&shell));
    shell.set_recording(false);
    assert!(draws_light(&shell), "light again once it is not recorded");
    window.release(&mut shell, &mut s, at + vec2(0.0, 40.0), 1.0 / 60.0);
}

#[test]
fn the_desktop_draws_the_full_look_while_the_camera_moves() {
    let mut s = part();
    let mut shell = Shell::new();
    assert!(!shell.light_while_moving());
    let mut window = Window::new();
    window.frame(&mut shell, &mut s, 0.0, Vec::new());
    studio(&mut shell, &mut s);
    window.frame(&mut shell, &mut s, 0.1, Vec::new());
    window.drag(&mut shell, &mut s, vec2(60.0, 0.0), 1.0 / 60.0);
    assert!(!interacting(&mut shell, &mut s));
    assert!(!draws_light(&shell));
}

#[test]
fn the_smaller_size_while_moving_keeps_the_shape_and_thins_the_lines() {
    assert_eq!(moving_size([1001, 600], 2.0, 1), ([1001, 600], 2.0));
    assert_eq!(moving_size([1001, 600], 2.0, 2), ([501, 300], 1.0));
    assert_eq!(moving_size([1, 1], 1.0, 4), ([1, 1], 0.25));
    assert_eq!(moving_size([0, 3], 1.0, 2), ([1, 2], 0.5));
}

fn evidence(name: &str, width: u32, height: u32, rgb: &[u8]) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/ui");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("move-light-{name}.png"));
    let png = craft3d_render::encode_png(width, height, rgb).unwrap();
    std::fs::write(&path, png).unwrap();
    println!("evidence: {}", path.display());
}

/// The picture the direct path shows once it settled, its frames
/// averaged, the clock standing still: the frames drawn and the picture.
fn settled(
    window: &mut Window,
    shell: &mut Shell,
    s: &mut Session,
    name: &str,
) -> (usize, u32, u32, Vec<u8>) {
    let mut drawn = 0;
    for _ in 0..64 {
        window.frame(shell, s, 0.0, Vec::new());
        drawn += 1;
        if !shell.converging() {
            break;
        }
    }
    assert!(!shell.converging(), "{name}: still averaging");
    let (w, h, rgb) = shell.direct_picture().unwrap().unwrap();
    evidence(name, w, h, &rgb);
    (drawn, w, h, rgb)
}

/// On the window's own device: while the camera moves one frame is drawn
/// at half the size per side; once it rested the full look comes back at
/// the full size and settles to the picture a camera that never moved
/// draws there, pixel for pixel; recorded frame by frame it is the full
/// picture throughout.
#[test]
fn on_the_direct_path_a_moving_camera_is_one_smaller_frame_and_a_resting_one_full() {
    let state = headless_render_state()
        .unwrap_or_else(|e| panic!("the direct path cannot be tested: {e}; {ADAPTER_HINT}"));
    let mut s = part();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    shell.attach_gpu(&state).unwrap();
    shell.set_light_while_moving(true);
    shell.set_moving_divisor(2);
    assert!(shell.draws_directly());
    let mut window = Window::new();
    window.frame(&mut shell, &mut s, 0.0, Vec::new());
    studio(&mut shell, &mut s);
    let (_, full_w, full_h, before) = settled(&mut window, &mut shell, &mut s, "before");

    // Turning: one frame and done, half the size per side.
    let at = window.drag(&mut shell, &mut s, vec2(60.0, 0.0), 1.0 / 60.0);
    // Held still, the clock standing still: still moving, one frame.
    let (drawn, w, h, light) = settled(&mut window, &mut shell, &mut s, "moving");
    assert_eq!(drawn, 1, "a moving camera averages no frames");
    assert_eq!(
        [w, h],
        moving_size([full_w, full_h], 1.0, 2).0,
        "drawn smaller"
    );
    assert!(light.iter().any(|&b| b != light[0]), "a blank picture");

    // Back where it started and let go: once rested the full picture,
    // pixel for pixel the one before the turn.
    window.frame(
        &mut shell,
        &mut s,
        1.0 / 60.0,
        vec![Event::PointerMoved(at)],
    );
    window.release(&mut shell, &mut s, at, 1.0 / 60.0);
    window.frame(&mut shell, &mut s, STILL_DELAY * 2.0, Vec::new());
    assert!(!shell.interacting_light());
    let (drawn, w, h, after) = settled(&mut window, &mut shell, &mut s, "rested");
    assert!(
        drawn >= 3,
        "the rested picture averages its frames: {drawn}"
    );
    assert_eq!([w, h], [full_w, full_h]);
    assert!(after == before, "the rested picture is not the one before");

    // Recorded frame by frame while it turns: the full picture.
    shell.set_frame_exact(true);
    let at = window.drag(&mut shell, &mut s, vec2(60.0, 0.0), 1.0 / 60.0);
    window.frame(
        &mut shell,
        &mut s,
        1.0 / 60.0,
        vec![Event::PointerMoved(at)],
    );
    let (drawn, w, h, recorded) = settled(&mut window, &mut shell, &mut s, "recorded");
    assert!(
        drawn >= 3,
        "the recorded picture averages its frames: {drawn}"
    );
    assert_eq!([w, h], [full_w, full_h]);
    assert!(
        recorded == before,
        "the recorded picture is not the full one"
    );
    shell.set_frame_exact(false);
}
