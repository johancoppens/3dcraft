//! The see-through viewport: `state.transparency` set through `ui.set`
//! and by the slider, the picture it gives (the front surface over the one
//! behind it, by the formula of `craft3d_render::render`), the hidden
//! edges drawn dimmed, picks that still give the front-most face, the
//! window's GPU path drawing a see-through frame itself, and a device that
//! cannot handing it to the CPU renderer.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_gpu::ADAPTER_HINT;
use craft3d_render::{NO_FACE, RenderParams};
use craft3d_ui_egui::backend::Backend;
use craft3d_ui_egui::direct::headless_render_state;
use craft3d_ui_egui::state::MAX_TRANSPARENCY;
use craft3d_ui_egui::viewport::{self, LineKind, Quality};
use craft3d_ui_egui::{RendererChoice, Shell};
use egui::{Event, PointerButton, Pos2, pos2, vec2};
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

/// A box with a blind pocket cut into it from below (a cavity that the
/// opaque picture from above does not show) and a post beside it.
fn part() -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Block", "size": [60, 40, 20]}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Drill", "position": [30, 20, -5], "radius": 10, "height": 17}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Block", "tool": "Drill", "name": "Pocketed"}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Post", "position": [80, 20, 0], "radius": 6, "height": 30}),
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
}

fn shot(shell: &mut Shell, session: &mut Session, width: u32, height: u32) -> Vec<u8> {
    let result = call(
        shell,
        session,
        "ui.screenshot",
        json!({"width": width, "height": height, "scale": 1.0}),
    );
    let png = base64_decode(result["base64"].as_str().unwrap());
    let mut reader = png::Decoder::new(std::io::Cursor::new(&png))
        .read_info()
        .unwrap();
    let mut rgb = vec![0; reader.output_buffer_size()];
    reader.next_frame(&mut rgb).unwrap();
    rgb
}

fn base64_decode(text: &str) -> Vec<u8> {
    let value = |c: u8| -> u32 {
        match c {
            b'A'..=b'Z' => u32::from(c - b'A'),
            b'a'..=b'z' => u32::from(c - b'a') + 26,
            b'0'..=b'9' => u32::from(c - b'0') + 52,
            b'+' => 62,
            b'/' => 63,
            _ => panic!("not base64: {c}"),
        }
    };
    let bytes: Vec<u8> = text.bytes().filter(|&c| c != b'=').collect();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        let mut word = 0_u32;
        for (k, &c) in chunk.iter().enumerate() {
            word |= value(c) << (18 - 6 * k);
        }
        for k in 0..chunk.len().saturating_sub(1) {
            out.push((word >> (16 - 8 * k)) as u8);
        }
    }
    out
}

fn evidence(name: &str, width: u32, height: u32, rgb: &[u8]) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/ui");
    std::fs::create_dir_all(&dir).unwrap();
    let png = craft3d_render::encode_png(width, height, rgb).unwrap();
    std::fs::write(dir.join(name), png).unwrap();
}

#[test]
fn transparency_is_view_state_that_ui_set_sets_and_the_screenshot_shows() {
    let mut session = part();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let tokens = shell.tokens;
    call(&mut shell, &mut session, "ui.fit", json!({"view": "iso"}));
    let (width, height) = (480_u32, 360_u32);
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["state"]["transparency"], 0.0);
    let plain = shot(&mut shell, &mut session, width, height);

    // The frames of the same state, made here: the opaque one for its id
    // buffer, and per opacity the bodies alone, without lines.
    let scene = session.scene_with(shell.scene_quality()).unwrap().unwrap();
    let camera = shell.state.camera;
    let quality = Quality {
        width,
        height,
        ssaa: 1,
    };
    let opaque = viewport::render_bodies(Some(&scene), &camera, quality, &tokens).unwrap();
    let bodies = |opacity: f64| {
        craft3d_render::render(
            &scene.mesh,
            &camera.to_render(),
            &RenderParams {
                opacity,
                ..viewport::render_params(quality, &tokens)
            },
        )
        .unwrap()
    };
    assert_eq!(bodies(1.0), opaque);
    // The pocket is inside: the opaque picture shows no face of the
    // drill, a see-through one has it behind the top and the sides.
    let pocket_faces: Vec<u32> = {
        let body = scene.bodies.iter().find(|b| b.name == "Pocketed").unwrap();
        body.face_names
            .iter()
            .enumerate()
            .filter(|(_, name)| name.contains("copy(1)"))
            .map(|(k, _)| body.first_face_id + k as u32)
            .collect()
    };
    assert_eq!(
        pocket_faces.len(),
        2,
        "the wall and the floor of the pocket"
    );
    assert!(opaque.ids.iter().all(|id| !pocket_faces.contains(id)));

    let mut last = plain.clone();
    for transparency in [0.25, 0.5, 0.75, 0.9] {
        let set = call(
            &mut shell,
            &mut session,
            "ui.set",
            json!({"transparency": transparency}),
        );
        assert_eq!(set["state"]["transparency"], transparency, "{set}");
        assert_eq!(shell.state.transparency, transparency);
        assert_eq!(shell.opacity(), 1.0 - transparency);
        let seen = shot(&mut shell, &mut session, width, height);
        assert_ne!(seen, last, "transparency {transparency}");
        // The screenshot is the oracle's see-through frame with the lines
        // over it: away from every line, pixel for pixel. (The screenshot
        // is supersampled and the frame here is not, so only pixels whose
        // neighbourhood is one face and one colour gradient are compared:
        // inside a flat face seen through to the background.)
        let frame = bodies(1.0 - transparency);
        let overlay = viewport::overlay(Some(&scene), &camera, width, height, None);
        let near_line = |x: f64, y: f64| {
            overlay.lines.iter().any(|l| {
                let (dx, dy) = (l.to[0] - l.from[0], l.to[1] - l.from[1]);
                let l2 = dx * dx + dy * dy;
                let t = if l2 > 0.0 {
                    (((x - l.from[0]) * dx + (y - l.from[1]) * dy) / l2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                (x - (l.from[0] + dx * t)).hypot(y - (l.from[1] + dy * t)) < 4.0
            })
        };
        let (mut compared, mut blended) = (0, 0);
        for y in 2..height - 2 {
            for x in 2..width - 2 {
                let i = (y * width + x) as usize;
                if near_line(f64::from(x) + 0.5, f64::from(y) + 0.5) {
                    continue;
                }
                let px = |rgb: &[u8]| [rgb[i * 3], rgb[i * 3 + 1], rgb[i * 3 + 2]];
                let (f, s) = (px(&frame.rgb), px(&seen));
                let near = (0..3).all(|c| f[c].abs_diff(s[c]) <= 2);
                compared += 1;
                assert!(
                    near || {
                        // A border between two surfaces behind: the
                        // supersampled picture mixes them.
                        let around = [i - 1, i + 1, i - width as usize, i + width as usize];
                        around.iter().any(|&n| {
                            (0..3).any(|c| frame.rgb[n * 3 + c].abs_diff(frame.rgb[i * 3 + c]) > 2)
                        })
                    },
                    "transparency {transparency} at ({x}, {y}): {f:?} in the frame, {s:?} in the screenshot"
                );
                if frame.ids[i] != NO_FACE && px(&opaque.rgb) != f {
                    blended += 1;
                }
            }
        }
        assert!(
            compared > 50_000 && blended > 5_000,
            "{compared}, {blended}"
        );
        // The hidden edges are drawn dimmed: the rim of the pocket, which
        // the opaque picture leaves out.
        let lines = |see_through: bool| {
            viewport::overlay_see_through(
                Some(&scene),
                &camera,
                &opaque,
                None,
                viewport::depth_slack(Backend::Cpu, Some(&scene), &camera),
                &viewport::Marks::default(),
                see_through,
            )
            .lines
        };
        let hidden = lines(true)
            .iter()
            .filter(|l| l.kind == LineKind::Edge && l.hidden)
            .count();
        assert!(hidden > 20, "{hidden} hidden stretches of edges");
        assert!(
            lines(false)
                .iter()
                .all(|l| !(l.kind == LineKind::Edge && l.hidden))
        );
        // A pick gives the front-most face as before: the same answer at
        // every pixel of a grid as when opaque.
        if transparency == 0.5 {
            evidence("see-through-half.png", width, height, &seen);
            for y in (5..height).step_by(23) {
                for x in (5..width).step_by(23) {
                    let at = json!({"x": f64::from(x) + 0.5, "y": f64::from(y) + 0.5,
                                    "width": width, "height": height});
                    let now = call(&mut shell, &mut session, "ui.pick", at.clone());
                    call(
                        &mut shell,
                        &mut session,
                        "ui.set",
                        json!({"transparency": 0}),
                    );
                    let before = call(&mut shell, &mut session, "ui.pick", at);
                    call(
                        &mut shell,
                        &mut session,
                        "ui.set",
                        json!({"transparency": 0.5}),
                    );
                    assert_eq!(now, before, "({x}, {y})");
                    let id = opaque.ids[(y * width + x) as usize];
                    assert_eq!(now.get("face").is_some(), id != NO_FACE, "({x}, {y})");
                }
            }
        }
        if transparency == 0.9 {
            evidence("see-through-most.png", width, height, &seen);
        }
        last = seen;
    }
    evidence("see-through-opaque.png", width, height, &plain);

    // A highlighted face behind the front shows in its colour: the floor
    // of the pocket, seen through the top.
    let body = scene.bodies.iter().find(|b| b.name == "Pocketed").unwrap();
    let floor = body
        .face_names
        .iter()
        .find(|name| name.contains("copy(1)") && name.ends_with("face(2)"))
        .expect("the top of the drill is the floor of the pocket");
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"transparency": 0.6}),
    );
    let before = shot(&mut shell, &mut session, width, height);
    call(
        &mut shell,
        &mut session,
        "ui.highlight",
        json!({"face": floor, "feature": "Pocketed"}),
    );
    let marked = shot(&mut shell, &mut session, width, height);
    let hue = |p: &[u8]| {
        let [r, g, b] = [i32::from(p[0]), i32::from(p[1]), i32::from(p[2])];
        g > r + 25 && b > r + 25
    };
    let tinted = marked.chunks(3).filter(|p| hue(p)).count();
    let tinted_before = before.chunks(3).filter(|p| hue(p)).count();
    assert!(
        tinted > tinted_before + 300,
        "{tinted} pixels are tinted by the highlight behind, {tinted_before} before"
    );
    evidence("see-through-highlight-behind.png", width, height, &marked);
    call(
        &mut shell,
        &mut session,
        "ui.highlight",
        json!({"face": null}),
    );

    // Out of range is clamped, what is not a number is refused, and 0 is
    // the opaque picture again, byte for byte.
    for (given, kept) in [
        (5.0, MAX_TRANSPARENCY),
        (-1.0, 0.0),
        (0.95, MAX_TRANSPARENCY),
    ] {
        call(
            &mut shell,
            &mut session,
            "ui.set",
            json!({"transparency": given}),
        );
        assert_eq!(shell.state.transparency, kept);
    }
    for bad in [json!("half"), json!(null), json!([0.5]), json!({"a": 1})] {
        let state = shell.state.clone();
        let error = shell
            .handle_method(&mut session, "ui.set", Some(&json!({"transparency": bad})))
            .unwrap()
            .unwrap_err();
        assert!(error.starts_with("ui.set"), "{error}");
        assert_eq!(shell.state, state);
    }
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"transparency": 0}),
    );
    assert_eq!(shot(&mut shell, &mut session, width, height), plain);
}

#[test]
fn the_slider_in_the_viewport_sets_the_transparency_and_is_not_the_viewport() {
    let mut session = part();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1000.0, 700.0);
    for _ in 0..4 {
        window.frame(&mut shell, &mut session, Vec::new());
    }
    let viewport_rect = shell.viewport_rect().unwrap();
    let slider = shell.transparency_rect().unwrap();
    assert!(viewport_rect.contains_rect(slider), "{slider:?}");
    // In the bottom row, with the sketches and datums, below the view
    // cube.
    let gizmo = shell.view_cube_rect().unwrap();
    assert!(slider.top() > gizmo.bottom());
    assert_eq!(shell.state.transparency, 0.0);
    let camera = shell.state.camera;
    // A drag from its left end to its right end: opaque to nearly clear,
    // growing on the way.
    let y = slider.center().y;
    let from = pos2(slider.left() + 2.0, y);
    window.frame(&mut shell, &mut session, vec![Event::PointerMoved(from)]);
    window.button(&mut shell, &mut session, from, true);
    let mut seen = vec![shell.state.transparency];
    for step in 1..=8 {
        let at = pos2(from.x + (slider.width() + 6.0) * step as f32 / 8.0, y);
        window.frame(&mut shell, &mut session, vec![Event::PointerMoved(at)]);
        seen.push(shell.state.transparency);
    }
    window.button(
        &mut shell,
        &mut session,
        pos2(slider.right() + 4.0, y),
        false,
    );
    window.frame(&mut shell, &mut session, Vec::new());
    assert!(seen.windows(2).all(|w| w[0] <= w[1]), "{seen:?}");
    assert!(seen.iter().any(|&t| t > 0.2 && t < 0.7), "{seen:?}");
    assert_eq!(shell.state.transparency, MAX_TRANSPARENCY, "{seen:?}");
    // The camera did not turn and nothing was picked.
    assert_eq!(shell.state.camera, camera);
    assert_eq!(shell.state.picked, None);
    // The viewport draws it: the frame on screen is a see-through one.
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["state"]["transparency"], MAX_TRANSPARENCY);
    let overlay = shell.overlay().0.clone();
    assert!(
        overlay
            .lines
            .iter()
            .any(|l| l.kind == LineKind::Edge && l.hidden),
        "the window draws the hidden edges dimmed"
    );
    // Back to the left: opaque.
    let to = pos2(slider.left() - 4.0, y);
    window.frame(
        &mut shell,
        &mut session,
        vec![Event::PointerMoved(pos2(slider.right() - 2.0, y))],
    );
    window.button(
        &mut shell,
        &mut session,
        pos2(slider.right() - 2.0, y),
        true,
    );
    window.frame(&mut shell, &mut session, vec![Event::PointerMoved(to)]);
    window.button(&mut shell, &mut session, to, false);
    window.frame(&mut shell, &mut session, Vec::new());
    assert_eq!(shell.state.transparency, 0.0);
    assert_eq!(shell.state.picked, None);
}

/// The window's device draws the see-through frames itself (two depth
/// layers and a blend, `craft3d_gpu`), so that turning a see-through model
/// is as quick as turning an opaque one; `ui.get` says which renderer drew
/// the frame on screen.
#[test]
fn a_see_through_frame_of_a_gpu_viewport_is_drawn_by_the_gpu() {
    let state = headless_render_state()
        .unwrap_or_else(|e| panic!("the direct path cannot be tested: {e}; {ADAPTER_HINT}"));
    let mut session = part();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    shell.attach_gpu(&state).unwrap();
    let mut window = Window::new(1000.0, 700.0);
    for _ in 0..3 {
        window.frame(&mut shell, &mut session, Vec::new());
    }
    assert!(shell.draws_directly());
    assert_eq!(shell.render_stats().unwrap().renderer, Backend::Gpu);
    let (_, _, direct_plain) = shell.direct_picture().unwrap().unwrap();
    let mut pictures = vec![direct_plain.clone()];
    for transparency in [0.25, 0.5, 0.9] {
        let before = shell.render_count();
        call(
            &mut shell,
            &mut session,
            "ui.set",
            json!({"transparency": transparency}),
        );
        for _ in 0..3 {
            window.frame(&mut shell, &mut session, Vec::new());
        }
        // One frame, on the device, sharp, and no other renderer's.
        assert_eq!(shell.render_count(), before + 1);
        let stats = shell.render_stats().unwrap();
        assert_eq!(stats.renderer, Backend::Gpu, "{stats:?}");
        assert!(stats.sharp);
        assert!(shell.problem().is_none(), "{:?}", shell.problem());
        let got = call(&mut shell, &mut session, "ui.get", json!({}));
        assert_eq!(got["render"]["renderer"], "gpu");
        assert_eq!(got["renderer"]["backend"], "gpu");
        assert_eq!(got["renderer"]["direct"], true);
        assert_eq!(got["state"]["transparency"], transparency);
        let (w, h, picture) = shell.direct_picture().unwrap().unwrap();
        evidence(
            &format!("see-through-direct-{transparency}.png"),
            w,
            h,
            &picture,
        );
        // More see-through, another picture.
        assert!(!pictures.contains(&picture), "transparency {transparency}");
        pictures.push(picture);
    }
    // Turning the model stays on the device, every frame.
    let from = pos2(600.0, 350.0);
    window.frame(&mut shell, &mut session, vec![Event::PointerMoved(from)]);
    window.frame(
        &mut shell,
        &mut session,
        vec![Event::PointerButton {
            pos: from,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    let before = shell.render_count();
    for step in 1..=5 {
        let at = from + vec2(8.0 * step as f32, 3.0 * step as f32);
        window.frame(&mut shell, &mut session, vec![Event::PointerMoved(at)]);
        let moving = shell.render_stats().unwrap();
        assert!(moving.sharp);
        assert_eq!(moving.renderer, Backend::Gpu);
    }
    assert_eq!(shell.render_count(), before + 5);
    window.frame(
        &mut shell,
        &mut session,
        vec![Event::PointerButton {
            pos: from + vec2(40.0, 15.0),
            button: PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    // Opaque again: the opaque picture of the new view, on the device.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"transparency": 0}),
    );
    for _ in 0..3 {
        window.frame(&mut shell, &mut session, Vec::new());
    }
    assert_eq!(shell.render_stats().unwrap().renderer, Backend::Gpu);

    // The renderer that reads frames back draws it too.
    let mut offscreen = craft3d_gpu::Offscreen::new().unwrap();
    let scene = session.scene(None).unwrap().unwrap();
    let camera = shell.state.camera.to_render();
    let [clear, opaque] = [0.5, 1.0].map(|opacity| {
        let params = RenderParams {
            opacity,
            ..RenderParams::default()
        };
        offscreen.render(&scene.mesh, &camera, &params).unwrap()
    });
    assert_ne!(clear.rgb, opaque.rgb);
    assert_eq!(clear.ids, opaque.ids, "a pick gives the front-most face");
}

/// A device that cannot draw see-through bodies refuses such a frame, and
/// the window shows the CPU oracle's for as long as the bodies are
/// see-through, and says so; opaque again, the device draws.
#[test]
fn a_device_without_see_through_hands_such_a_frame_to_the_cpu_renderer() {
    let state = headless_render_state()
        .unwrap_or_else(|e| panic!("the direct path cannot be tested: {e}; {ADAPTER_HINT}"));
    let mut session = part();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    shell.attach_gpu(&state).unwrap();
    shell.without_gpu_see_through();
    let mut window = Window::new(1000.0, 700.0);
    for _ in 0..3 {
        window.frame(&mut shell, &mut session, Vec::new());
    }
    assert!(shell.draws_directly());
    assert_eq!(shell.render_stats().unwrap().renderer, Backend::Gpu);
    let (_, _, direct_plain) = shell.direct_picture().unwrap().unwrap();
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"transparency": 0.5}),
    );
    for _ in 0..4 {
        window.frame(&mut shell, &mut session, Vec::new());
    }
    shell.wait_for_sharp(std::time::Duration::from_secs(20));
    window.frame(&mut shell, &mut session, Vec::new());
    let stats = shell.render_stats().unwrap();
    assert_eq!(stats.renderer, Backend::Cpu, "{stats:?}");
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["render"]["renderer"], "cpu");
    assert!(
        shell.direct_picture().is_none(),
        "the device's picture is not shown"
    );
    assert!(
        shell.draws_directly(),
        "the direct path is kept for opaque frames"
    );
    assert!(shell.problem().is_none(), "{:?}", shell.problem());
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"transparency": 0}),
    );
    for _ in 0..3 {
        window.frame(&mut shell, &mut session, Vec::new());
    }
    assert_eq!(shell.render_stats().unwrap().renderer, Backend::Gpu);
    let (_, _, again) = shell.direct_picture().unwrap().unwrap();
    assert_eq!(
        again, direct_plain,
        "opaque again: the same picture on the device"
    );

    // The renderer that reads frames back, the same: the shell's own
    // renderer hands the frame to the CPU and says which one made it.
    let scene = session.scene(None).unwrap().unwrap();
    let tokens = craft3d_ui_egui::Tokens::default();
    let quality = Quality {
        width: 160,
        height: 120,
        ssaa: 1,
    };
    let mut renderer = craft3d_ui_egui::backend::BodyRenderer::new(RendererChoice::Auto);
    assert_eq!(renderer.backend(), Backend::Gpu, "{ADAPTER_HINT}");
    renderer.set_opacity(0.5);
    let camera = shell.state.camera;
    let (drawn, by) = renderer
        .render(Some(&scene), &camera, quality, &tokens)
        .unwrap();
    assert_eq!(by, Backend::Gpu);
    renderer.without_gpu_see_through();
    let (fallen, by) = renderer
        .render(Some(&scene), &camera, quality, &tokens)
        .unwrap();
    assert_eq!(by, Backend::Cpu);
    assert_eq!(drawn.ids, fallen.ids);
    renderer.set_opacity(1.0);
    let (_, by) = renderer
        .render(Some(&scene), &camera, quality, &tokens)
        .unwrap();
    assert_eq!(by, Backend::Gpu);
}
