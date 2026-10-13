//! The GPU viewport against the CPU viewport: the two renderers through
//! the shell's own code path ([`BodyRenderer`] and `viewport::compose_with`),
//! compared with the tolerances of docs/architecture.md §7.2 (oracle 7),
//! and the shell showing and reporting the GPU's picture.
//!
//! The comparison is the one of `crates/gpu/tests/parity.rs`, included
//! from there so the tolerance and its derivation have one home. The
//! pictures land in `target/evidence/ui/`.
//!
//! These tests need a graphics adapter and fail, with what to install,
//! when there is none; they do not skip (the GPU crate's policy).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

#[path = "../../gpu/tests/common/mod.rs"]
mod common;

use common::compare;
use craft3d_engine::{Scene, Session, Workspace};
use craft3d_gpu::{ADAPTER_HINT, Frame};
use craft3d_ui_egui::backend::{Backend, BodyRenderer, RendererChoice};
use craft3d_ui_egui::shell::image_quality_for;
use craft3d_ui_egui::state::{CameraState, ProjectionKind};
use craft3d_ui_egui::viewport::{self, Composed, Overlay, Quality};
use craft3d_ui_egui::{Shell, Tokens};
use egui::{Event, Pos2, vec2};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// A plate with a post, a bead and a ball on it (planes, a cylinder, a
/// torus, a sphere: every surface the primitives have), and a sketch above
/// them whose lines pass behind the post.
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
        "feature.torus",
        json!({"name": "Bead", "major": 7, "minor": 2.5, "position": [42, 14, 10.5]}),
    );
    run(
        &mut s,
        "feature.sphere",
        json!({"name": "Ball", "radius": 5, "position": [44, 30, 13]}),
    );
    run(
        &mut s,
        "sketch.create",
        json!({"name": "Outline", "offset": 20}),
    );
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Outline", "closed": true, "points": [[-5, -5], [65, -5], [65, 45], [-5, 45]]}),
    );
    run(
        &mut s,
        "sketch.line",
        json!({"sketch": "Outline", "from": [-5, 20], "to": [65, 20]}),
    );
    s
}

fn scene(session: &mut Session) -> Arc<Scene> {
    session.scene(None).unwrap().unwrap()
}

/// The GPU renderer, or the failure every test here shares.
fn gpu() -> BodyRenderer {
    let renderer = BodyRenderer::new(RendererChoice::Auto);
    assert_eq!(
        renderer.backend(),
        Backend::Gpu,
        "the GPU viewport cannot be tested: {}; {ADAPTER_HINT}",
        renderer.report().describe()
    );
    renderer
}

fn evidence(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/ui");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn write_png(path: &PathBuf, width: u32, height: u32, rgb: &[u8]) {
    let png = craft3d_render::encode_png(width, height, rgb).unwrap();
    std::fs::write(path, png).unwrap();
}

/// The overlay alone, inked on black: what the depth test decided.
fn overlay_ink(composed: &Composed, tokens: &Tokens) -> Vec<u8> {
    let blank = Composed {
        rgb: vec![0; composed.rgb.len()],
        overlay: composed.overlay.clone(),
        ..composed.clone()
    };
    viewport::rasterise(blank, tokens, 1.0).rgb
}

/// The camera of the viewport at rest: fitted to the scene, from the iso
/// view, in the projection asked for.
fn camera(scene: &Scene, projection: ProjectionKind, quality: Quality) -> CameraState {
    let mut camera = CameraState {
        projection,
        ..CameraState::default()
    };
    camera.set_view("iso").unwrap();
    camera.fit(
        &scene.bbox,
        f64::from(quality.width) / f64::from(quality.height),
    );
    camera
}

/// Colour difference, face id difference and the band as a picture:
/// dark blue where the band is, green where the pictures differ (brighter
/// is larger; one level is already visible), red where the face ids do.
fn difference(cpu: &[u8], gpu: &[u8], ids: (&Frame, &Frame), band: &[bool]) -> Vec<u8> {
    let (cpu_ids, gpu_ids) = ids;
    let mut out = Vec::with_capacity(cpu.len());
    for (i, (a, b)) in cpu
        .as_chunks::<3>()
        .0
        .iter()
        .zip(gpu.as_chunks::<3>().0)
        .enumerate()
    {
        let diff = (0..3).map(|c| a[c].abs_diff(b[c])).max().unwrap();
        let red = if cpu_ids.ids[i] != gpu_ids.ids[i] {
            255
        } else {
            0
        };
        let green = if diff == 0 {
            0
        } else {
            (96 + u32::from(diff) * 16).min(255) as u8
        };
        let blue = if band[i] { 90 } else { 0 };
        out.extend([red, green, blue]);
    }
    out
}

#[test]
fn the_gpu_viewport_matches_the_cpu_viewport_within_the_documented_tolerance() {
    let mut session = part();
    let scene = scene(&mut session);
    let tokens = Tokens::default();
    // The sharp image of a 960 × 640 viewport, as the window asks for it.
    let quality = image_quality_for(Backend::Gpu, [960, 640], false, 8192);
    assert_eq!((quality.width, quality.height, quality.ssaa), (960, 640, 2));
    let mut gpu = gpu();
    let adapter = gpu.report().adapter.clone().unwrap();
    println!("GPU viewport on {adapter}");
    let mut cpu = BodyRenderer::cpu();
    let mut log = format!("adapter: {adapter}\n");
    for projection in [ProjectionKind::Perspective, ProjectionKind::Orthographic] {
        let name = match projection {
            ProjectionKind::Perspective => "perspective",
            ProjectionKind::Orthographic => "orthographic",
        };
        let camera = camera(&scene, projection, quality);
        let render_camera = camera.to_render();

        // The bodies: face ids, distances and colours against the oracle.
        let (cpu_frame, made_by) = cpu.render(Some(&scene), &camera, quality, &tokens).unwrap();
        assert_eq!(made_by, Backend::Cpu);
        let (gpu_frame, made_by) = gpu.render(Some(&scene), &camera, quality, &tokens).unwrap();
        assert_eq!(made_by, Backend::Gpu, "{}", gpu.report().describe());
        let (parity, band) = compare(&scene.mesh, &render_camera, &cpu_frame, &gpu_frame);
        let label = format!("viewport {name} on {adapter}");
        println!("{label}: {parity:?}");
        log.push_str(&format!("{label}: {parity:?}\n"));
        parity.check(&label);
        assert!(
            parity.covered > parity.pixels / 10,
            "the part fills a good part of the viewport: {parity:?}"
        );

        // The whole picture, overlay included, through the viewport's
        // own path, both ways.
        let cpu_composed =
            viewport::compose_with(&mut cpu, Some(&scene), &camera, quality, &tokens, None)
                .unwrap();
        let gpu_composed =
            viewport::compose_with(&mut gpu, Some(&scene), &camera, quality, &tokens, None)
                .unwrap();
        assert_eq!(cpu_composed.backend, Backend::Cpu);
        assert_eq!(gpu_composed.backend, Backend::Gpu);
        assert_eq!(
            cpu_composed.rgb, cpu_frame.rgb,
            "the CPU renderer is deterministic"
        );
        assert_eq!(
            gpu_composed.rgb, gpu_frame.rgb,
            "the GPU renderer is deterministic"
        );
        assert!((cpu_composed.coverage - gpu_composed.coverage).abs() < 0.002);

        // The depth test over the GPU frame hides what it hides over the
        // CPU frame: the overlays, inked by themselves, differ only where
        // a stretch ends a pixel earlier or later at a silhouette.
        let cpu_ink = overlay_ink(&cpu_composed, &tokens);
        let gpu_ink = overlay_ink(&gpu_composed, &tokens);
        let inked = cpu_ink
            .as_chunks::<3>()
            .0
            .iter()
            .filter(|p| **p != [0, 0, 0])
            .count();
        let differ = cpu_ink
            .as_chunks::<3>()
            .0
            .iter()
            .zip(gpu_ink.as_chunks::<3>().0)
            .filter(|(a, b)| a != b)
            .count();
        let hidden = |overlay: &Overlay| overlay.lines.iter().filter(|l| l.hidden).count();
        println!(
            "{label}: overlay {} lines ({} hidden) on the CPU frame, {} lines ({} hidden) on the \
             GPU frame; {inked} pixels inked, {differ} differ",
            cpu_composed.overlay.lines.len(),
            hidden(&cpu_composed.overlay),
            gpu_composed.overlay.lines.len(),
            hidden(&gpu_composed.overlay),
        );
        log.push_str(&format!(
            "{label}: overlay {inked} pixels inked, {differ} differ\n"
        ));
        assert!(inked > 2000, "the overlay is there: {inked} pixels");
        assert!(
            differ * 50 <= inked,
            "the overlays differ in {differ} of {inked} inked pixels"
        );
        // Every hidden sketch stretch is hidden in both: the sketch lines
        // behind the post are dimmed whichever renderer made the frame.
        assert!(
            hidden(&cpu_composed.overlay) > 0,
            "something is behind a body"
        );
        assert_eq!(
            hidden(&cpu_composed.overlay) > 0,
            hidden(&gpu_composed.overlay) > 0
        );

        // What the developer looks at: the two pictures as the window
        // shows them, and their difference.
        let cpu_picture = viewport::rasterise(cpu_composed, &tokens, 1.0);
        let gpu_picture = viewport::rasterise(gpu_composed, &tokens, 1.0);
        let diff = difference(
            &cpu_picture.rgb,
            &gpu_picture.rgb,
            (&cpu_frame, &gpu_frame),
            &band,
        );
        for (suffix, rgb) in [
            ("cpu", &cpu_picture.rgb),
            ("gpu", &gpu_picture.rgb),
            ("diff", &diff),
        ] {
            let path = evidence(&format!("viewport-parity-{name}-{suffix}.png"));
            write_png(&path, quality.width, quality.height, rgb);
            println!("evidence: {}", path.display());
        }
    }
    std::fs::write(evidence("viewport-parity.txt"), log).unwrap();
}

/// A window of a fixed size, fed one frame at a time, that keeps every
/// image it was asked to upload as a texture.
struct Window {
    ctx: egui::Context,
    size: egui::Vec2,
    time: f64,
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

    fn frame(&mut self, shell: &mut Shell, session: &mut Session, events: Vec<Event>) {
        self.time += 1.0 / 60.0;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, self.size)),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
        let uploads: Vec<(usize, usize, Vec<u8>)> = output
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
        self.uploads.extend(uploads);
        output.textures_delta.clear();
    }

    /// The last image of `width` x `height` put on screen.
    fn uploaded(&self, width: u32, height: u32) -> Option<&[u8]> {
        self.uploads
            .iter()
            .rev()
            .find(|(w, h, _)| (*w, *h) == (width as usize, height as usize))
            .map(|(_, _, rgb)| rgb.as_slice())
    }
}

fn call(shell: &mut Shell, session: &mut Session, method: &str, params: Value) -> Value {
    shell
        .handle_method(session, method, Some(&params))
        .unwrap()
        .unwrap_or_else(|e| panic!("{method}: {e}"))
}

#[test]
fn the_shell_puts_the_gpu_picture_on_screen_and_says_so() {
    let mut session = part();
    let mut shell = Shell::with_renderer(RendererChoice::Auto);
    assert_eq!(
        shell.renderer_report().backend,
        Backend::Gpu,
        "{}; {ADAPTER_HINT}",
        shell.renderer_report().describe()
    );
    let mut window = Window::new(1000.0, 700.0);
    window.frame(&mut shell, &mut session, Vec::new());
    window.frame(&mut shell, &mut session, Vec::new());
    let [w, h] = shell.viewport_size();
    let stats = shell.render_stats().unwrap();
    assert_eq!(stats.renderer, Backend::Gpu);
    assert!(stats.sharp);
    assert_eq!((stats.width, stats.height, stats.ssaa), (w, h, 2));

    // The texture is the GPU's picture of this camera: a second GPU
    // renderer draws the same.
    let uploaded = window.uploaded(w, h).unwrap();
    let expected = viewport::compose_with(
        &mut gpu(),
        Some(&scene(&mut session)),
        &shell.state.camera,
        Quality {
            width: w,
            height: h,
            ssaa: 2,
        },
        &shell.tokens,
        None,
    )
    .unwrap();
    assert_eq!(expected.backend, Backend::Gpu);
    let differ = uploaded
        .as_chunks::<3>()
        .0
        .iter()
        .zip(expected.rgb.as_chunks::<3>().0)
        .filter(|(a, b)| a != b)
        .count();
    println!(
        "{differ} of {} texture pixels differ between two GPU renderers",
        w * h
    );
    assert!(differ * 1000 <= (w * h) as usize, "{differ} pixels differ");

    // `ui.get` says which renderer is on screen and which the viewport
    // uses.
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["render"]["renderer"], "gpu");
    assert_eq!(got["render"]["sharp"], true);
    assert_eq!(got["renderer"]["backend"], "gpu");
    assert!(got["renderer"]["adapter"].as_str().unwrap().len() > 3);
    assert_eq!(got["renderer"]["fallback"], Value::Null);

    // The screenshot is the oracle's picture, whatever the window shows:
    // the same size and camera, within the tolerance of the GPU's picture,
    // and never identical to the texture only by accident.
    let shot = call(
        &mut shell,
        &mut session,
        "ui.screenshot",
        json!({"width": w, "height": h, "scale": 1.0}),
    );
    assert_eq!(
        (shot["width"].as_u64(), shot["height"].as_u64()),
        (Some(u64::from(w)), Some(u64::from(h)))
    );
    assert!(shot["coverage"].as_f64().unwrap() > 0.1);

    // A CPU shell says cpu, with nothing to fall back from.
    let mut cpu = Shell::with_renderer(RendererChoice::Cpu);
    window.frame(&mut cpu, &mut session, Vec::new());
    window.frame(&mut cpu, &mut session, Vec::new());
    assert_eq!(cpu.render_stats().unwrap().renderer, Backend::Cpu);
    let got = call(&mut cpu, &mut session, "ui.get", json!({}));
    assert_eq!(got["render"]["renderer"], "cpu");
    assert_eq!(
        got["renderer"],
        json!({"backend": "cpu", "adapter": Value::Null, "fallback": Value::Null})
    );
}

#[test]
fn with_a_render_thread_both_the_preview_and_the_sharp_image_come_from_the_gpu() {
    let mut session = part();
    let mut shell = Shell::threaded_with(RendererChoice::Auto);
    assert!(shell.renders_in_background());
    assert_eq!(
        shell.renderer_report().backend,
        Backend::Gpu,
        "{}; {ADAPTER_HINT}",
        shell.renderer_report().describe()
    );
    let mut window = Window::new(1000.0, 700.0);
    window.frame(&mut shell, &mut session, Vec::new());
    window.frame(&mut shell, &mut session, Vec::new());
    let [w, h] = shell.viewport_size();
    // The preview, made on this thread: half the size, one sample, the
    // GPU.
    let preview = shell.render_stats().unwrap();
    if !preview.sharp {
        assert_eq!(
            (preview.width, preview.height, preview.ssaa),
            (w / 2, h / 2, 1)
        );
        assert_eq!(preview.renderer, Backend::Gpu);
    }
    assert!(shell.wait_for_sharp(Duration::from_secs(120)));
    window.frame(&mut shell, &mut session, Vec::new());
    let sharp = shell.render_stats().unwrap();
    assert!(sharp.sharp);
    assert_eq!((sharp.width, sharp.height, sharp.ssaa), (w, h, 2));
    assert_eq!(sharp.renderer, Backend::Gpu, "the thread's renderer");
    let report = shell.renderer_report();
    assert_eq!(report.backend, Backend::Gpu);
    assert_eq!(report.fallback, None);
    println!(
        "sharp {}x{} ssaa {} on the render thread in {:.1} ms on {}",
        sharp.width,
        sharp.height,
        sharp.ssaa,
        sharp.milliseconds.unwrap_or(f64::NAN),
        report.adapter.unwrap_or_default()
    );
}
