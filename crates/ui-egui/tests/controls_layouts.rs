//! The controls over the viewport: in a window of 1920 × 1080 and one of
//! 1280 × 800, with the section off and on, every control is drawn inside
//! the viewport, and a synthetic click on each does what the control is
//! for.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::state::ProjectionKind;
use craft3d_ui_egui::{RendererChoice, Shell};
use egui::{Event, PointerButton, Pos2, Rect, vec2};
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
    s
}

struct Window {
    ctx: egui::Context,
    size: egui::Vec2,
    time: f64,
}

impl Window {
    fn frame(&mut self, shell: &mut Shell, session: &mut Session, events: Vec<Event>) {
        self.time += 1.0 / 60.0;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
        output.textures_delta.clear();
    }

    fn click(&mut self, shell: &mut Shell, session: &mut Session, at: Pos2) {
        self.frame(shell, session, vec![Event::PointerMoved(at)]);
        for pressed in [true, false] {
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
        // Away again, so the next frame has no hover left over.
        self.frame(shell, session, vec![Event::PointerGone]);
        self.frame(shell, session, Vec::new());
    }
}

fn hit(hits: &[(&'static str, Rect)], name: &str) -> Pos2 {
    hits.iter()
        .find(|(n, _)| *n == name)
        .unwrap_or_else(|| panic!("no control `{name}` in {hits:?}"))
        .1
        .center()
}

fn set(shell: &mut Shell, session: &mut Session, patch: Value) {
    shell
        .handle_method(session, "ui.set", Some(&patch))
        .unwrap()
        .unwrap();
}

/// At both sizes: everything inside the viewport, and each control does
/// its job when clicked.
#[test]
fn every_control_is_reachable() {
    {
        for (width, height) in [(1920.0, 1080.0), (1280.0, 800.0)] {
            let mut session = part();
            let mut shell = Shell::with_renderer(RendererChoice::Cpu);
            let mut window = Window {
                ctx: egui::Context::default(),
                size: vec2(width, height),
                time: 0.0,
            };
            for _ in 0..4 {
                window.frame(&mut shell, &mut session, Vec::new());
            }
            let label = format!("{width}x{height}");
            let viewport = shell.viewport_rect().unwrap();
            let check = |shell: &Shell, on: bool| {
                let layout = shell.controls_layout().unwrap();
                let mut names: Vec<&str> = Vec::new();
                for (name, rect) in layout.parts().into_iter().chain(layout.controls()) {
                    assert!(
                        viewport.contains_rect(rect),
                        "{label}: {name} {rect:?} outside {viewport:?}"
                    );
                    names.push(name);
                }
                let mut expected = vec![
                    "sketches",
                    "datums",
                    "see-through",
                    "section",
                    "fit",
                    "iso",
                    "projection",
                    "cube",
                    "triad",
                ];
                if on {
                    expected.extend(["axes", "offset", "side"]);
                }
                for name in expected {
                    assert!(
                        names.contains(&name),
                        "{label}, section {on}: no {name} in {names:?}"
                    );
                }
                assert_eq!(
                    layout.axes.is_some(),
                    on,
                    "{label}: the options only while on"
                );
            };
            check(&shell, false);
            // The display toggles.
            let before = shell.state.show;
            let at = hit(shell.show_hits(), "sketches");
            window.click(&mut shell, &mut session, at);
            assert_eq!(shell.state.show.sketches, !before.sketches, "{label}");
            let at = hit(shell.show_hits(), "datums");
            window.click(&mut shell, &mut session, at);
            assert_eq!(shell.state.show.datums, !before.datums, "{label}");
            // The projection, Iso and Fit.
            let at = hit(shell.camera_hits(), "orthographic");
            window.click(&mut shell, &mut session, at);
            assert_eq!(
                shell.state.camera.projection,
                ProjectionKind::Orthographic,
                "{label}"
            );
            let at = hit(shell.camera_hits(), "perspective");
            window.click(&mut shell, &mut session, at);
            assert_eq!(
                shell.state.camera.projection,
                ProjectionKind::Perspective,
                "{label}"
            );
            set(&mut shell, &mut session, json!({"view": "top"}));
            let at = hit(shell.camera_hits(), "iso");
            window.click(&mut shell, &mut session, at);
            let iso = shell.state.camera;
            assert!((iso.yaw + 45.0).abs() < 1e-9, "{label}: {iso:?}");
            set(
                &mut shell,
                &mut session,
                json!({"camera": {"distance": 5000.0}}),
            );
            let at = hit(shell.camera_hits(), "fit");
            window.click(&mut shell, &mut session, at);
            assert!(shell.state.camera.distance < 5000.0, "{label}");
            // None of those clicks was a pick in the viewport.
            assert_eq!(shell.state.picked, None, "{label}");
            // The section: on, its options, an axis, and the side.
            let at = hit(shell.section_hits(), "toggle");
            window.click(&mut shell, &mut session, at);
            assert!(shell.state.section.on, "{label}");
            window.frame(&mut shell, &mut session, Vec::new());
            check(&shell, true);
            let at = hit(shell.section_hits(), "x");
            window.click(&mut shell, &mut session, at);
            assert_eq!(shell.state.section.plane.as_deref(), Some("yz"), "{label}");
            let at = hit(shell.section_hits(), "flip");
            window.click(&mut shell, &mut session, at);
            assert!(shell.state.section.flip, "{label}");
            assert_eq!(shell.state.picked, None, "{label}");
            assert!(shell.section_hits().iter().any(|(n, _)| *n == "offset"));
            // And off again: only the toggle is left to click.
            let at = hit(shell.section_hits(), "toggle");
            window.click(&mut shell, &mut session, at);
            assert!(!shell.state.section.on, "{label}");
            window.frame(&mut shell, &mut session, Vec::new());
            check(&shell, false);
            assert_eq!(
                shell.section_hits().len(),
                1,
                "{label}: {:?}",
                shell.section_hits()
            );
        }
    }
}

/// `ui.screenshot {window: true}` is the desktop window's: anywhere else
/// it is refused with the reason, and a `window` that is not a boolean is
/// an error too.
#[test]
fn a_window_picture_needs_the_window() {
    let mut session = part();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let error = shell
        .handle_method(
            &mut session,
            "ui.screenshot",
            Some(&json!({"window": true})),
        )
        .unwrap()
        .unwrap_err();
    assert!(error.contains("only the window"), "{error}");
    let error = shell
        .handle_method(&mut session, "ui.screenshot", Some(&json!({"window": 3})))
        .unwrap()
        .unwrap_err();
    assert!(error.contains("boolean"), "{error}");
    // The request is recognised as one, with its path checked.
    let line = r#"{"id": 1, "method": "ui.screenshot", "params": {"window": true}}"#;
    assert_eq!(
        Shell::window_picture_request(&session, line),
        Some(Ok(None))
    );
    let line =
        r#"{"id": 1, "method": "ui.screenshot", "params": {"window": true, "path": "a.png"}}"#;
    assert!(matches!(
        Shell::window_picture_request(&session, line),
        Some(Err(_))
    ));
    let line = r#"{"id": 1, "method": "ui.screenshot", "params": {"window": true, "width": 3}}"#;
    assert!(matches!(
        Shell::window_picture_request(&session, line),
        Some(Err(_))
    ));
    let line = r#"{"id": 1, "method": "ui.screenshot", "params": {}}"#;
    assert_eq!(Shell::window_picture_request(&session, line), None);
    // A picture of a frame is a PNG of its size.
    let image = egui::ColorImage::new([4, 3], vec![egui::Color32::from_rgb(10, 20, 30); 12]);
    let result = shell.window_picture(&mut session, &image, None).unwrap();
    assert_eq!(
        (result["width"].clone(), result["height"].clone()),
        (json!(4), json!(3))
    );
    assert_eq!(result["mime"], "image/png");
}

/// A click on a plate between its controls (its padding, the gap with a
/// divider) is not a pick in the viewport; a click just beside the plate
/// is.
#[test]
fn the_plates_take_their_clicks_and_beside_them_is_the_viewport() {
    let mut session = part();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window {
        ctx: egui::Context::default(),
        size: vec2(1280.0, 800.0),
        time: 0.0,
    };
    for _ in 0..4 {
        window.frame(&mut shell, &mut session, Vec::new());
    }
    let layout = shell.controls_layout().unwrap();
    let pad = shell.tokens.nav_padding;
    let (bar, camera) = (layout.bar.unwrap(), layout.camera_plate.unwrap());
    let (see, section) = (layout.see_through.unwrap(), layout.section_toggle.unwrap());
    let (iso, projection) = (layout.iso.unwrap(), layout.projection.unwrap());
    let on_plates = [
        Pos2::new(bar.left() + pad * 0.5, bar.center().y),
        Pos2::new(bar.center().x, bar.top() + pad * 0.5),
        Pos2::new((see.right() + section.left()) * 0.5, bar.center().y),
        Pos2::new(camera.right() - pad * 0.5, camera.center().y),
        Pos2::new((iso.right() + projection.left()) * 0.5, camera.center().y),
    ];
    let before = shell.state.clone();
    for at in on_plates {
        window.click(&mut shell, &mut session, at);
        assert_eq!(shell.state.picked, None, "{at:?} picked");
        assert_eq!(shell.state.show, before.show);
        assert_eq!(shell.state.section, before.section);
        assert_eq!(shell.state.camera.projection, before.camera.projection);
    }
    let beside = Pos2::new(bar.left() - 6.0, bar.center().y);
    window.click(&mut shell, &mut session, beside);
    assert!(
        shell.state.picked.is_some(),
        "beside the bar is the viewport"
    );
}
