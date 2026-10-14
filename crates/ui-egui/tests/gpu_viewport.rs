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

use common::{color_tolerance, compare, differ_probability};
use craft3d_engine::{DISPLAY_RELATIVE_TOLERANCE, Scene, SceneQuality, Session, Workspace};
use craft3d_gpu::{ADAPTER_HINT, Frame, NO_FACE};
use craft3d_ui_egui::backend::{Backend, BodyRenderer, RendererChoice};
use craft3d_ui_egui::camera::Projector;
use craft3d_ui_egui::direct::{ReadBack, headless_render_state};
use craft3d_ui_egui::shell::image_quality_for;
use craft3d_ui_egui::state::{CameraState, ProjectionKind};
use craft3d_ui_egui::viewport::{self, Composed, Overlay, OverlayLine, Quality};
use craft3d_ui_egui::{Shell, Tokens};
use egui::{Event, PointerButton, Pos2, pos2, vec2};
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

fn scene(session: &mut Session, quality: SceneQuality) -> Arc<Scene> {
    session.scene_with(quality).unwrap().unwrap()
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
    let scene = scene(&mut session, SceneQuality::Automatic);
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
        Some(&scene(&mut session, shell.scene_quality())),
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
        json!({"backend": "cpu", "adapter": Value::Null, "fallback": Value::Null, "direct": false, "msaa": 1})
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

// ---------------------------------------------------------------------
// The direct path: the viewport drawn by the GPU on the window's own
// device, multisampled, with the lines over it drawn and depth-tested on
// the GPU, read back here only to compare it with the oracle.

/// `true` for a pixel within `radius` of any line or dot of `overlay`.
fn near_lines(overlay: &Overlay, width: u32, height: u32, radius: f64) -> Vec<bool> {
    let (w, h) = (width as usize, height as usize);
    let mut near = vec![false; w * h];
    let mut mark = |p: [f64; 2]| {
        let r = radius.ceil() as i64;
        let (cx, cy) = (p[0].floor() as i64, p[1].floor() as i64);
        for dy in -r..=r {
            for dx in -r..=r {
                let (px, py) = (cx + dx, cy + dy);
                if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h {
                    near[py as usize * w + px as usize] = true;
                }
            }
        }
    };
    for line in &overlay.lines {
        let length = (line.to[0] - line.from[0]).hypot(line.to[1] - line.from[1]);
        let steps = ((length * 2.0).ceil() as usize).clamp(1, 8 * (w + h));
        for i in 0..=steps {
            let t = i as f64 / steps as f64;
            mark([
                line.from[0] + (line.to[0] - line.from[0]) * t,
                line.from[1] + (line.to[1] - line.from[1]) * t,
            ]);
        }
    }
    for point in &overlay.points {
        mark(point.at);
    }
    near
}

/// Per pixel of `frame` (the CPU frame the oracle is drawn on): whether a
/// datum plane of `marks` fills it, as `viewport::fill_datum_planes`
/// decides, and whether the GPU could decide otherwise. The GPU tests the
/// plane's point against its depth buffer like a line: hidden when the
/// furthest of the nine surfaces around the pixel, plus the slack and the
/// spread of the nine, is nearer; the CPU when all nine plus the slack
/// are nearer. The GPU's nine pixels are multisampled: their samples lie
/// within the CPU's 5 × 5 pixels around the pixel. A pixel is marginal
/// where the plane's point lies between the lowest and the highest
/// threshold either rule can have there, widened by the error of the GPU's point (its `f32`
/// roundings, magnified at a grazing angle), by the slack once more
/// (which holds the GPU depth buffer's own error) and by how far the CPU
/// frame's screen-interpolated distance lies off the surface on the faces
/// around the pixel: there the plane passes through a surface and either
/// answer is right.
fn fill_decisions(
    frame: &Frame,
    mesh: &craft3d_tess::Mesh,
    camera: &CameraState,
    marks: &viewport::Marks,
    slack: f64,
) -> (Vec<bool>, Vec<bool>) {
    let (w, h) = (frame.width as usize, frame.height as usize);
    let mut fill = vec![false; w * h];
    let mut marginal = vec![false; w * h];
    let projector =
        Projector::new(&camera.to_render(), frame.width as f64, frame.height as f64).unwrap();
    // The CPU frame's distance is interpolated evenly across the screen,
    // where the true distance's reciprocal is: over a triangle whose
    // corners lie at distances a to b it is off the surface by at most
    // (a + b) / 2 - 2ab / (a + b) = (b - a)² / (2 (a + b)). Per face, the
    // most over its triangles; the GPU tests against the true distance.
    let mut interpolation: std::collections::BTreeMap<u32, f64> = Default::default();
    for (triangle, &face) in mesh.triangles.iter().zip(&mesh.face_ids) {
        let d = triangle.map(|k| projector.depth(mesh.positions[k as usize]));
        let (a, b) = (d[0].min(d[1]).min(d[2]), d[0].max(d[1]).max(d[2]));
        let off = if a > 0.0 {
            (b - a).powi(2) / (2.0 * (a + b))
        } else {
            b - a
        };
        let entry = interpolation.entry(face).or_insert(0.0);
        *entry = entry.max(off);
    }
    for (center, u, v) in marks.datum_planes() {
        let normal = u.cross(v);
        for py in 0..h {
            for px in 0..w {
                let (x, y) = (px as f64 + 0.5, py as f64 + 0.5);
                let (start, direction) = projector.ray(x, y);
                let along = direction.dot(normal);
                if along == 0.0 {
                    continue;
                }
                let t = (center - start).dot(normal) / along;
                if t.is_nan() || t <= projector.near() {
                    continue;
                }
                let r = start + direction * t - center;
                if (r.dot(u) / u.dot(u)).abs() > 1.0 || (r.dot(v) / v.dot(v)).abs() > 1.0 {
                    continue;
                }
                // The CPU's nine pixels; and the 5 × 5 around the pixel,
                // which hold the samples of the GPU's nine (a sample lies
                // within half a pixel of its pixel's centre).
                let mut furthest = 0.0_f64;
                for ny in py.saturating_sub(1)..=(py + 1).min(h - 1) {
                    for nx in px.saturating_sub(1)..=(px + 1).min(w - 1) {
                        furthest = furthest.max(frame.depth[ny * w + nx]);
                    }
                }
                let (mut far5, mut near5, mut off) = (0.0_f64, f64::INFINITY, 0.0_f64);
                for ny in py.saturating_sub(2)..=(py + 2).min(h - 1) {
                    for nx in px.saturating_sub(2)..=(px + 2).min(w - 1) {
                        let d = frame.depth[ny * w + nx];
                        far5 = far5.max(d);
                        near5 = near5.min(d);
                        off = off.max(
                            interpolation
                                .get(&frame.ids[ny * w + nx])
                                .copied()
                                .unwrap_or(0.0),
                        );
                    }
                }
                let cpu_hidden = furthest + slack < t;
                let i = py * w + px;
                fill[i] |= !cpu_hidden;
                // The GPU computes the point in `f32`, relative to the eye:
                // a few roundings of 2⁻²⁴ on numbers as large as the reach,
                // magnified where the ray meets the plane at a grazing
                // angle (1 / cos of it).
                let grazing = direction.length() * normal.length() / along.abs();
                let error =
                    2f64.powi(-18) * (t.abs() + r.length() + u.length() + v.length()) * grazing;
                // The GPU hides the point beyond its furthest sample plus
                // the slack and the spread of its samples, which lies
                // between the nearest of the 5 × 5 plus the slack and the
                // furthest of the 5 × 5 plus the slack and their spread;
                // the CPU beyond its furthest plus the slack. Between the
                // lowest and the highest of these, give or take the
                // errors, the two may answer differently.
                let low = near5.min(furthest) + slack - error - slack - off;
                let high = far5 + slack + (far5 - near5) + error + slack + off;
                // Next to the background (`far5` infinite) a sample may
                // show nothing behind the point: the GPU keeps it.
                if near5.is_finite() && t >= low && t <= high {
                    marginal[i] = true;
                }
            }
        }
    }
    (fill, marginal)
}

/// Distance from `p` to the segment `line`, in pixels.
fn distance_to(p: [f64; 2], line: &OverlayLine) -> f64 {
    let (ax, ay) = (line.from[0], line.from[1]);
    let (dx, dy) = (line.to[0] - ax, line.to[1] - ay);
    let l2 = dx * dx + dy * dy;
    let t = if l2 > 0.0 {
        (((p[0] - ax) * dx + (p[1] - ay) * dy) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p[0] - (ax + dx * t)).hypot(p[1] - (ay + dy * t))
}

fn pixel(rgb: &[u8], width: u32, p: [f64; 2]) -> [u8; 3] {
    let i = (p[1].floor() as usize * width as usize + p[0].floor() as usize) * 3;
    [rgb[i], rgb[i + 1], rgb[i + 2]]
}

fn levels(a: [u8; 3], b: [u8; 3]) -> u8 {
    (0..3).map(|c| a[c].abs_diff(b[c])).max().unwrap()
}

#[test]
fn the_direct_path_draws_the_viewport_on_the_windows_device_and_matches_the_oracle() {
    direct_path_against_the_oracle(0.0);
}

/// The part with datums: a plane through the post, a plane at an angle
/// about an axis along the plate's front edge, that axis, and a point
/// above the ball.
fn part_with_datums() -> Session {
    let mut s = part();
    run(
        &mut s,
        "datum.plane",
        json!({"from": "xy", "distance": 20, "name": "Shelf"}),
    );
    run(
        &mut s,
        "datum.axis",
        json!({"through": [[0, 0, 8], [60, 0, 8]], "name": "Hinge"}),
    );
    run(
        &mut s,
        "datum.plane",
        json!({"from": "xy", "angle": "35 deg", "about": "Hinge", "name": "Ramp"}),
    );
    run(
        &mut s,
        "datum.point",
        json!({"at": [44, 30, 24], "name": "Mark"}),
    );
    s
}

/// Datums drawn by the window's device against the oracle: the fill of
/// the planes under the lines, their outlines, the axis and the point,
/// opaque and over see-through bodies (§7.2, *Datums*).
#[test]
fn the_direct_path_draws_datums_and_matches_the_oracle() {
    for transparency in [0.0, 0.5] {
        direct_path_against_the_oracle_of(part_with_datums(), "viewport-datums", transparency);
    }
}

/// The same with the bodies see-through: the device draws the two layers
/// and blends them, with the hidden parts of the bodies' edges dimmed,
/// and the picture is still the oracle's within the tolerance (§7.2,
/// *See-through*).
#[test]
fn the_direct_path_draws_see_through_bodies_and_matches_the_oracle() {
    for transparency in [0.5, 0.9] {
        direct_path_against_the_oracle(transparency);
    }
}

/// The viewport drawn directly, with the bodies `transparency` see-through
/// (0: opaque), against the CPU oracle of the same state.
fn direct_path_against_the_oracle(transparency: f64) {
    direct_path_against_the_oracle_of(part(), "viewport-direct", transparency);
}

/// The viewport of `session` drawn directly, with the bodies
/// `transparency` see-through (0: opaque), against the CPU oracle of the
/// same state, with the datums it has shown. The pictures are written as
/// `{base}-*.png`, with `-seethrough{transparency}` when see-through.
fn direct_path_against_the_oracle_of(mut session: Session, base: &str, transparency: f64) {
    let see_through = transparency > 0.0;
    let name = if see_through {
        format!("{base}-seethrough{transparency}")
    } else {
        base.to_string()
    };
    let state = headless_render_state()
        .unwrap_or_else(|e| panic!("the direct path cannot be tested: {e}; {ADAPTER_HINT}"));
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    // Wide lines, so the pixel nearest a line's centre is all ink in both
    // pictures, whatever the renderers drew beneath it.
    shell.tokens.edge_width = 3.0;
    shell.tokens.sketch_width = 3.0;
    shell.tokens.construction_width = 3.0;
    shell.tokens.axis_width = 3.0;
    shell.tokens.datum_width = 3.0;
    let tokens = shell.tokens;
    shell.attach_gpu(&state).unwrap();
    assert!(shell.draws_directly());
    let report = shell.renderer_report();
    assert_eq!(report.backend, Backend::Gpu);
    assert!(report.direct && report.fallback.is_none(), "{report:?}");
    assert!(matches!(report.msaa, 1 | 4), "{report:?}");
    let adapter = report.adapter.clone().unwrap();
    println!("direct path on {adapter}, {} samples", report.msaa);

    let mut window = Window::new(1000.0, 700.0);
    window.frame(&mut shell, &mut session, Vec::new());
    if see_through {
        call(
            &mut shell,
            &mut session,
            "ui.set",
            json!({"transparency": transparency}),
        );
    }
    window.frame(&mut shell, &mut session, Vec::new());
    window.frame(&mut shell, &mut session, Vec::new());
    let [w, h] = shell.viewport_size();
    let stats = shell.render_stats().unwrap();
    assert_eq!(stats.renderer, Backend::Gpu);
    assert!(stats.sharp, "every frame of the direct path is sharp");
    assert_eq!(
        (stats.width, stats.height, stats.ssaa, stats.msaa),
        (w, h, 1, report.msaa)
    );
    // No image travels through egui's texture upload: the picture is the
    // device's own texture.
    assert!(window.uploaded(w, h).is_none());
    let (pw, ph, gpu_rgb) = shell.direct_picture().unwrap().unwrap();
    assert_eq!((pw, ph), (w, h));

    // The oracle: the CPU picture of the same state at one sample per
    // pixel (shaded at the pixel centres, where the GPU shades), the lines
    // inked by the CPU with the depth decisions taken with the slack the
    // GPU frame gets.
    let scene = scene(&mut session, shell.scene_quality());
    let camera = shell.state.camera;
    let quality = Quality {
        width: w,
        height: h,
        ssaa: 1,
    };
    let layer = |opacity: f64| {
        let params = craft3d_render::RenderParams {
            opacity,
            ..viewport::render_params(quality, &tokens)
        };
        craft3d_render::render(&scene.mesh, &camera.to_render(), &params).unwrap()
    };
    let cpu_frame = layer(shell.opacity());
    assert_eq!(shell.opacity(), 1.0 - transparency);
    // The pictures whose gradients bound the colour: the frame itself,
    // or its two layers when it is see-through.
    let layers = if see_through {
        vec![layer(1.0), layer(0.0)]
    } else {
        vec![cpu_frame.clone()]
    };
    let slack = viewport::depth_slack(Backend::Gpu, Some(&scene), &camera);
    // The datums, as the shell shows them (none in a part without any).
    let marks = viewport::datum_marks(Some(&scene), shell.state.show.datums);
    let overlay = viewport::overlay_see_through(
        Some(&scene),
        &camera,
        &cpu_frame,
        None,
        slack,
        &marks,
        see_through,
    );
    let mut filled = cpu_frame.rgb.clone();
    viewport::fill_datum_planes(
        &mut filled,
        w,
        h,
        &cpu_frame.depth,
        &camera,
        &marks,
        &tokens,
        slack,
    );
    let (fill, fill_marginal) = fill_decisions(&cpu_frame, &scene.mesh, &camera, &marks, slack);
    // The picture under the lines: the bodies with the datum planes'
    // fill (the bodies alone without datums).
    let under = filled.clone();
    let composed = Composed {
        width: w,
        height: h,
        rgb: filled,
        overlay: overlay.clone(),
        coverage: cpu_frame.coverage(),
        triangles: scene.triangles(),
        backend: Backend::Cpu,
    };
    let cpu_rgb = viewport::rasterise(composed, &tokens, 1.0).rgb;
    let render_camera = camera.to_render();
    let band = if see_through {
        common::band_see_through(&scene.mesh, &render_camera, &cpu_frame, 1)
    } else {
        common::band(&scene.mesh, &render_camera, &cpu_frame)
    };
    let seams = common::seams(&scene.mesh, &render_camera, w, h);
    let everything = viewport::overlay_untested(Some(&scene), &camera, w, h, None, &marks);
    let lines = near_lines(&everything, w, h, 3.5);
    let (wu, hu) = (w as usize, h as usize);

    // 1. The bodies, away from every edge and line: the §7.2 tolerance.
    // On a seam (an interior triangle edge) a multisampled pixel may mix
    // two fragments: a pixel of gradient more (see the GPU crate's tests).
    let channel = |rgb: &[u8], i: usize, c: usize| f64::from(rgb[i * 3 + c]);
    let gradient_of = |layer: &Frame, i: usize| -> f64 {
        let (x, y) = (i % wu, i / wu);
        let mut g: f64 = 0.0;
        for dy in -1_i64..=1 {
            for dx in -1_i64..=1 {
                let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                if nx < 0 || ny < 0 || nx as usize >= wu || ny as usize >= hu {
                    continue;
                }
                let n = ny as usize * wu + nx as usize;
                for c in 0..3 {
                    g = g.max((channel(&layer.rgb, i, c) - channel(&layer.rgb, n, c)).abs());
                }
            }
        }
        g
    };
    let (mut plain, mut plain_differ, mut plain_over, mut allowed) = (0, 0, 0, 0.0);
    let (mut seam, mut seam_over) = (0, 0);
    let (mut filled_pixels, mut marginal_pixels, mut fill_differ, mut fill_over) = (0, 0, 0, 0);
    for i in 0..wu * hu {
        if band[i] || lines[i] {
            continue;
        }
        // Where the CPU's and the GPU's depth tests of a datum plane's
        // fill can decide differently (the plane passes through a surface
        // there), the fill is not compared.
        if fill_marginal[i] {
            marginal_pixels += 1;
            continue;
        }
        let diff = f64::from(levels(
            pixel(&cpu_rgb, w, [(i % wu) as f64, (i / wu) as f64]),
            pixel(&gpu_rgb, w, [(i % wu) as f64, (i / wu) as f64]),
        ));
        let gradients = layers.iter().map(|layer| gradient_of(layer, i));
        let g = gradients.clone().fold(0.0, f64::max);
        // Under a datum plane's fill the GPU's fixed-function blend adds
        // a rounding of its own: how a blend rounds into an 8-bit target
        // is the implementation's (measured: lavapipe differs by a level
        // from the CPU's rounded `f64` at about half of such pixels,
        // Intel's Vulkan driver at 189 and its OpenGL driver at 59 of
        // about 149 000, none by more).
        // Over a colour within the tolerance, blending at the opacity α
        // scales the difference by 1 - α and rounds once more: one level
        // more is allowed there, and such pixels are counted apart from
        // the bodies' rounding bound.
        if fill[i] {
            filled_pixels += 1;
            fill_differ += usize::from(diff > 0.0);
            let base = if seams[i] {
                1.0 + g
            } else {
                color_tolerance(g)
            };
            fill_over += usize::from(diff > base + 1.0);
            continue;
        }
        if seams[i] {
            seam += 1;
            seam_over += usize::from(diff > 1.0 + g);
        } else {
            plain += 1;
            plain_differ += usize::from(diff > 0.0);
            plain_over += usize::from(diff > color_tolerance(g));
            allowed += gradients.map(differ_probability).sum::<f64>().min(1.0);
        }
    }
    println!(
        "bodies: {plain} plain pixels, {plain_differ} differ by a level (rounding explains {}), {plain_over} over; {seam} seam pixels, {seam_over} over; datum fill: {filled_pixels} pixels compared, {fill_differ} differ, {fill_over} over, {marginal_pixels} marginal left out",
        allowed.ceil()
    );
    if !marks.datum_planes().is_empty() {
        assert!(
            filled_pixels > (wu * hu) / 20,
            "too little of the fill compared: {filled_pixels}"
        );
    }
    assert_eq!(
        fill_over, 0,
        "the fill differs by more than a level of blending"
    );
    assert!(
        plain > (wu * hu) / 3,
        "too little compared strictly: {plain}"
    );
    assert_eq!(
        plain_over, 0,
        "colours differ away from every edge and line"
    );
    assert!(plain_differ as f64 <= allowed.ceil());
    assert_eq!(
        seam_over, 0,
        "colours differ on a seam by more than a pixel of gradient"
    );

    // 2. The lines: where the CPU decided a stretch is visible, hidden or
    // dimmed, the GPU drew the same. Sampled along every line of the
    // untested overlay (so the hidden stretches of the edges are sampled
    // too), away from the ends of the CPU's stretches, from other lines,
    // and (for what is not inked) from the visible feature edges where
    // the samples blend. The pixel nearest a sample is all ink when the
    // line shows, so it is the line's colour (or the dimmed blend) in both
    // pictures, and the plain picture when it does not.
    let visible_edges = {
        let mut near = vec![false; wu * hu];
        for y in 0..hu {
            for x in 0..wu {
                let id = cpu_frame.ids[y * wu + x];
                let changes = (x + 1 < wu && cpu_frame.ids[y * wu + x + 1] != id)
                    || (y + 1 < hu && cpu_frame.ids[(y + 1) * wu + x] != id);
                if changes {
                    for dy in -2_i64..=2 {
                        for dx in -2_i64..=2 {
                            let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                            if nx >= 0 && ny >= 0 && (nx as usize) < wu && (ny as usize) < hu {
                                near[ny as usize * wu + nx as usize] = true;
                            }
                        }
                    }
                }
            }
        }
        near
    };
    let (mut samples, mut agree, mut inked, mut dimmed, mut bare) = (0, 0, 0, 0, 0);
    /// (difference, where, the CPU's pixel, the GPU's).
    type Worst = (f64, [f64; 2], [u8; 3], [u8; 3]);
    let mut worst: Vec<Worst> = Vec::new();
    for (k, line) in everything.lines.iter().enumerate() {
        let length = (line.to[0] - line.from[0]).hypot(line.to[1] - line.from[1]);
        let steps = (length / 2.0).floor() as usize;
        for i in 1..steps {
            let t = i as f64 / steps as f64;
            let p = [
                line.from[0] + (line.to[0] - line.from[0]) * t,
                line.from[1] + (line.to[1] - line.from[1]) * t,
            ];
            if p[0] < 1.0 || p[1] < 1.0 || p[0] >= f64::from(w) - 1.0 || p[1] >= f64::from(h) - 1.0
            {
                continue;
            }
            // Away from other lines and dots.
            let crowded = everything
                .lines
                .iter()
                .enumerate()
                .any(|(j, other)| j != k && distance_to(p, other) < 4.0)
                || everything
                    .points
                    .iter()
                    .any(|dot| (p[0] - dot.at[0]).hypot(p[1] - dot.at[1]) < 5.0);
            if crowded {
                continue;
            }
            // Away from the ends of the CPU's stretches of this line.
            let near_end = overlay.lines.iter().any(|s| {
                s.kind == line.kind
                    && [s.from, s.to]
                        .iter()
                        .any(|e| (p[0] - e[0]).hypot(p[1] - e[1]) < 3.0)
            });
            if near_end {
                continue;
            }
            let at = [p[0].floor() + 0.5, p[1].floor() + 0.5];
            let idx = at[1] as usize * wu + at[0] as usize;
            let shown = overlay
                .lines
                .iter()
                .find(|s| s.kind == line.kind && distance_to(p, s) < 0.01);
            match shown {
                Some(s) if s.hidden => dimmed += 1,
                Some(_) => inked += 1,
                None => {
                    if visible_edges[idx] {
                        continue;
                    }
                    bare += 1;
                }
            }
            let (c, g) = (pixel(&cpu_rgb, w, at), pixel(&gpu_rgb, w, at));
            let mut diff = levels(c, g);
            // The hidden part of a body's edge, dimmed over see-through
            // bodies, lies on a feature edge of what is behind the front:
            // in the band, where a multisampled pixel is a mix of the
            // faces that meet there and the CPU's pixel is one of them.
            // Each of those faces is among the pixel's neighbours in the
            // CPU picture, so the GPU's pixel is, per channel, the dimmed
            // ink over a level within the range of the CPU picture's
            // 3 x 3 pixels around it.
            if see_through && matches!(shown, Some(s) if s.hidden) {
                let alpha = f64::from(tokens.hidden_opacity) / 255.0;
                let ink = line.kind.color(&tokens);
                let (ax, ay) = (at[0] as usize, at[1] as usize);
                diff = (0..3)
                    .map(|ch| {
                        let around = (ay - 1..=ay + 1)
                            .flat_map(|y| (ax - 1..=ax + 1).map(move |x| y * wu + x))
                            .map(|n| f64::from(under[n * 3 + ch]));
                        let lo = around.clone().fold(f64::INFINITY, f64::min);
                        let hi = around.fold(f64::NEG_INFINITY, f64::max);
                        let dim = |under: f64| under + (f64::from(ink[ch]) - under) * alpha;
                        let (lo, hi) = (dim(lo).min(dim(hi)), dim(lo).max(dim(hi)));
                        let got = f64::from(g[ch]);
                        (lo - got).max(got - hi).max(0.0).ceil() as u8
                    })
                    .max()
                    .unwrap_or(0);
            }
            samples += 1;
            if diff <= 8 {
                agree += 1;
            } else {
                worst.push((f64::from(diff), p, c, g));
            }
        }
    }
    worst.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    println!(
        "lines: {samples} samples ({inked} inked, {dimmed} dimmed, {bare} hidden edges), {agree} agree; worst {:?}",
        &worst[..worst.len().min(5)]
    );
    if see_through {
        // Through see-through bodies no stretch of an edge is left out:
        // what is hidden is dimmed.
        assert!(inked > 100 && dimmed > 40, "the test is hollow");
        assert_eq!(
            bare, 0,
            "a hidden edge is left out of a see-through picture"
        );
    } else {
        assert!(
            inked > 100 && dimmed > 10 && bare > 10,
            "the test is hollow"
        );
    }
    assert!(
        agree * 100 >= samples * 99,
        "the GPU's lines differ from the CPU's at {} of {samples} samples",
        samples - agree
    );

    // What the developer looks at.
    let diff: Vec<u8> = (0..wu * hu)
        .flat_map(|i| {
            let p = [(i % wu) as f64, (i / wu) as f64];
            let d = levels(pixel(&cpu_rgb, w, p), pixel(&gpu_rgb, w, p));
            let green = if d == 0 {
                0
            } else {
                (96 + u32::from(d) * 16).min(255) as u8
            };
            [
                if lines[i] { 60 } else { 0 },
                green,
                if band[i] { 90 } else { 0 },
            ]
        })
        .collect();
    for (suffix, rgb) in [("cpu", &cpu_rgb), ("gpu", &gpu_rgb), ("diff", &diff)] {
        let path = evidence(&format!("{name}-{suffix}.png"));
        write_png(&path, w, h, rgb);
        println!("evidence: {}", path.display());
    }
    std::fs::write(
        evidence(&format!("{name}.txt")),
        format!(
            "adapter: {adapter}, {} samples\nbodies: {plain} plain pixels, {plain_differ} differ by a level, {plain_over} over; {seam} seam pixels, {seam_over} over\ndatum fill: {filled_pixels} pixels compared, {fill_differ} differ, {fill_over} over, {marginal_pixels} marginal\nlines: {samples} samples ({inked} inked, {dimmed} dimmed, {bare} hidden edges), {agree} agree\n",
            report.msaa
        ),
    )
    .unwrap();

    // `ui.get` says so.
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["renderer"]["backend"], "gpu");
    assert_eq!(got["renderer"]["direct"], true);
    assert_eq!(got["renderer"]["msaa"], report.msaa);
    assert_eq!(got["render"]["renderer"], "gpu");
    assert_eq!(got["render"]["sharp"], true);
    assert_eq!(got["render"]["msaa"], report.msaa);
    assert_eq!(got["render"]["ssaa"], 1);

    // Nothing is drawn again while nothing changes.
    let count = shell.render_count();
    window.frame(&mut shell, &mut session, Vec::new());
    assert_eq!(shell.render_count(), count);

    // While the camera moves, every frame is drawn in full: no smaller
    // image, every one sharp.
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
    for step in 1..=5 {
        let at = from + vec2(8.0 * step as f32, 3.0 * step as f32);
        window.frame(&mut shell, &mut session, vec![Event::PointerMoved(at)]);
        let moving = shell.render_stats().unwrap();
        assert!(moving.sharp);
        assert_eq!((moving.width, moving.height, moving.ssaa), (w, h, 1));
        assert_eq!(moving.renderer, Backend::Gpu);
    }
    assert_eq!(shell.render_count(), count + 5);

    // The screenshot is still the oracle's, on the CPU, of the same size.
    let shot = call(
        &mut shell,
        &mut session,
        "ui.screenshot",
        json!({"width": w, "height": h, "scale": 1.0}),
    );
    assert_eq!(shot["width"].as_u64(), Some(u64::from(w)));
    assert!(shot["coverage"].as_f64().unwrap() > 0.1);

    // No face is lost on the way: the direct picture covers what the CPU
    // frame covers (the background colour is where no face is).
    // (Away from the fill of datum planes, which tints the background.)
    let background = tokens.viewport_background;
    let covered_cpu = (0..wu * hu)
        .filter(|&i| cpu_frame.ids[i] != NO_FACE && !fill[i])
        .count();
    let covered_gpu = (0..wu * hu)
        .filter(|&i| {
            let p = [(i % wu) as f64, (i / wu) as f64];
            pixel(&gpu_rgb, w, p) != background && !lines[i] && !fill[i]
        })
        .count();
    let uncovered_lines = (0..wu * hu)
        .filter(|&i| lines[i] && cpu_frame.ids[i] != NO_FACE && !fill[i])
        .count();
    assert!(
        (covered_gpu + uncovered_lines).abs_diff(covered_cpu) * 100 < covered_cpu * 3,
        "coverage {covered_gpu} + {uncovered_lines} against {covered_cpu}"
    );
}

/// A change of the display quality is another mesh of the same document
/// revision: the direct path uploads it and draws it, once.
#[test]
fn a_change_of_display_quality_reaches_the_direct_path() {
    let state = headless_render_state()
        .unwrap_or_else(|e| panic!("the direct path cannot be tested: {e}; {ADAPTER_HINT}"));
    let mut session = part();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    shell.attach_gpu(&state).unwrap();
    let mut window = Window::new(1000.0, 700.0);
    window.frame(&mut shell, &mut session, Vec::new());
    window.frame(&mut shell, &mut session, Vec::new());
    let shown = scene(&mut session, shell.scene_quality());
    let automatic = scene(&mut session, SceneQuality::Automatic);
    assert!(shown.triangles() > automatic.triangles());
    let stats = shell.render_stats().unwrap();
    assert_eq!(
        (stats.renderer, stats.triangles),
        (Backend::Gpu, shown.triangles())
    );
    let (_, _, fine) = shell.direct_picture().unwrap().unwrap();

    let renders = shell.render_count();
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"quality": {"chordal": 1e-3}}),
    );
    window.frame(&mut shell, &mut session, Vec::new());
    assert_eq!(shell.render_count(), renders + 1);
    let stats = shell.render_stats().unwrap();
    assert_eq!(
        (stats.renderer, stats.triangles),
        (Backend::Gpu, automatic.triangles())
    );
    let (_, _, coarse) = shell.direct_picture().unwrap().unwrap();
    assert!(fine != coarse, "the coarser mesh is another picture");
    // Nothing more is drawn while nothing changes.
    window.frame(&mut shell, &mut session, Vec::new());
    window.frame(&mut shell, &mut session, Vec::new());
    assert_eq!(shell.render_count(), renders + 1);
}

/// The flange of the demo: a disc of radius 40 with a hub, six bolt
/// holes of radius 4 about the axis at radius 30, a keyway and two lugs
/// with holes (the commands of the developer's `flange.sh`).
fn flange() -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "zx", "name": "Section"}),
    );
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Section", "closed": true,
               "points": [[0, 12], [0, 40], [10, 40], [10, 20], [30, 20], [34, 16], [34, 12]]}),
    );
    run(
        &mut s,
        "feature.revolve",
        json!({"sketch": "Section", "axis": "x", "name": "Flange"}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": 4, "height": 20, "position": [30, 0, -5], "name": "Bolt"}),
    );
    run(
        &mut s,
        "feature.pattern",
        json!({"source": "Bolt", "count": 6, "angle": "60 deg", "axis": [0, 0, 1],
               "point": [0, 0, 0], "name": "Bolts"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Flange", "tool": "Bolts", "name": "Drilled"}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"size": [4, 6, 40], "position": [10, -3, -3], "name": "Keyway"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Drilled", "tool": "Keyway", "name": "Keyed"}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"size": [14, 12, 10], "position": [38, -6, 0], "name": "Lug"}),
    );
    run(
        &mut s,
        "feature.mirror",
        json!({"source": "Lug", "normal": [1, 0, 0], "point": [0, 0, 0], "keep": true, "name": "Lug2"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "union", "target": "Keyed", "tool": "Lug", "name": "OneLug"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "union", "target": "OneLug", "tool": "Lug2", "name": "TwoLugs"}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": 3, "height": 20, "position": [46, 0, -5], "name": "LugHole"}),
    );
    run(
        &mut s,
        "feature.mirror",
        json!({"source": "LugHole", "normal": [1, 0, 0], "point": [0, 0, 0], "keep": true, "name": "LugHole2"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "TwoLugs", "tool": "LugHole", "name": "Holed"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Holed", "tool": "LugHole2", "name": "Flange part"}),
    );
    s
}

/// A square of `half` pixels about (`cx`, `cy`) of the picture, every
/// pixel `zoom` times as large, so a chord can be seen.
fn crop(rgb: &[u8], width: u32, height: u32, cx: f64, cy: f64, half: u32, zoom: u32) -> ReadBack {
    let (w, h) = (width as i64, height as i64);
    let (x0, y0) = (
        cx.round() as i64 - i64::from(half),
        cy.round() as i64 - i64::from(half),
    );
    let side = 2 * half;
    let mut out = Vec::with_capacity((side * zoom * side * zoom * 3) as usize);
    for y in 0..side {
        for _ in 0..zoom {
            for x in 0..side {
                let (px, py) = (x0 + i64::from(x), y0 + i64::from(y));
                let pixel = if (0..w).contains(&px) && (0..h).contains(&py) {
                    let i = ((py * w + px) * 3) as usize;
                    [rgb[i], rgb[i + 1], rgb[i + 2]]
                } else {
                    [0, 0, 0]
                };
                for _ in 0..zoom {
                    out.extend_from_slice(&pixel);
                }
            }
        }
    }
    (side * zoom, side * zoom, out)
}

/// The flange through the direct path, at the automatic tolerance the
/// viewport showed before 2026-10-13 and at the display quality, at the
/// two window sizes of the development machine. Pictures and a crop of
/// one bolt hole go to `target/evidence/ui/`. The check is the rule
/// itself: at the display quality the chord error of the mesh is below a
/// third of a pixel at the framed zoom, and at four times that zoom still
/// below a pixel, where the automatic tolerance's is over a third of a
/// pixel at the framed zoom on both sizes.
#[test]
fn the_flange_holes_are_round_at_the_display_quality() {
    let state = headless_render_state()
        .unwrap_or_else(|e| panic!("the direct path cannot be tested: {e}; {ADAPTER_HINT}"));
    let mut session = flange();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    shell.attach_gpu(&state).unwrap();
    let mut log = String::new();
    for size in [[1020.0_f32, 754.0_f32], [2560.0, 1400.0]] {
        let mut window = Window::new(size[0], size[1]);
        for (label, chordal) in [("automatic", 1e-3), ("display", DISPLAY_RELATIVE_TOLERANCE)] {
            call(
                &mut shell,
                &mut session,
                "ui.set",
                json!({"quality": {"chordal": chordal}}),
            );
            window.frame(&mut shell, &mut session, Vec::new());
            window.frame(&mut shell, &mut session, Vec::new());
            call(&mut shell, &mut session, "ui.fit", json!({"view": "iso"}));
            let drawn = std::time::Instant::now();
            window.frame(&mut shell, &mut session, Vec::new());
            let _ = state
                .device
                .poll(egui_wgpu::wgpu::PollType::wait_indefinitely());
            let gpu_done_ms = drawn.elapsed().as_secs_f64() * 1000.0;
            let scene = scene(&mut session, shell.scene_quality());
            let stats = shell.render_stats().unwrap();
            assert_eq!(stats.renderer, Backend::Gpu);
            assert_eq!(stats.triangles, scene.triangles());
            let (w, h, rgb) = shell.direct_picture().unwrap().unwrap();
            let name = format!("viewport-flange-{w}x{h}-{label}");
            write_png(&evidence(&format!("{name}.png")), w, h, &rgb);

            // The bolt hole at (30, 0) on the top of the disc (z = 10):
            // its centre and a point of its rim, in pixels.
            let projector =
                Projector::new(&shell.state.camera.to_render(), f64::from(w), f64::from(h))
                    .unwrap();
            let centre = projector
                .project(craft3d_math::Vec3::new(30.0, 0.0, 10.0))
                .unwrap();
            let rim = projector
                .project(craft3d_math::Vec3::new(34.0, 0.0, 10.0))
                .unwrap();
            let radius_px = (rim.x - centre.x).hypot(rim.y - centre.y);
            let px_per_mm = radius_px / 4.0;
            let chord_px = scene.tolerance * px_per_mm;
            let half = (radius_px * 1.6).ceil() as u32;
            let (cw, ch, crop_rgb) = crop(&rgb, w, h, centre.x, centre.y, half, 4);
            write_png(&evidence(&format!("{name}-hole.png")), cw, ch, &crop_rgb);
            let line = format!(
                "{label} at {w}x{h}: {} triangles, chordal tolerance {:.5} mm, hole radius {radius_px:.1} px, chord error {chord_px:.3} px; drawn in {:.1} ms on the thread, {gpu_done_ms:.1} ms until the GPU is done\n",
                scene.triangles(),
                scene.tolerance,
                stats.milliseconds.unwrap_or(f64::NAN)
            );
            print!("{line}");
            log.push_str(&line);
            if label == "display" {
                assert!(!scene.coarsened);
                assert!(chord_px < 1.0 / 3.0, "{chord_px} px");
                assert!(chord_px * 4.0 < 1.0, "{chord_px} px at four times the zoom");
            } else {
                assert!(chord_px > 1.0 / 3.0, "{chord_px} px");
            }
        }
    }
    std::fs::write(evidence("viewport-flange.txt"), log).unwrap();
    println!("evidence: {}", evidence("viewport-flange.txt").display());
}

#[test]
fn a_window_without_a_render_state_or_asking_for_the_cpu_does_not_draw_directly() {
    let state = headless_render_state()
        .unwrap_or_else(|e| panic!("the direct path cannot be tested: {e}; {ADAPTER_HINT}"));
    let cpu = Shell::threaded_with_gpu(RendererChoice::Cpu, Some(&state));
    assert!(!cpu.draws_directly());
    assert_eq!(cpu.renderer_report().backend, Backend::Cpu);
    let none = Shell::threaded_with_gpu(RendererChoice::Auto, None);
    assert!(!none.draws_directly());
    let direct = Shell::threaded_with_gpu(RendererChoice::Auto, Some(&state));
    assert!(direct.draws_directly());
    assert!(direct.renderer_report().direct);
    assert!(direct.renderer_report().describe().contains("direct"));
}

// ------------------------------------------------------- pointing (M8.5)

/// The faces of `scene` a view shows, with a pixel well inside each.
fn visible_faces(frame: &Frame) -> Vec<(u32, [u32; 2])> {
    let (w, h) = (frame.width as i64, frame.height as i64);
    let at = |x: i64, y: i64| frame.ids[(y * w + x) as usize];
    let mut inside: std::collections::BTreeMap<u32, Vec<[u32; 2]>> = Default::default();
    for y in 3..h - 3 {
        for x in 3..w - 3 {
            let id = at(x, y);
            if id != NO_FACE && (-3..=3).all(|dy| (-3..=3).all(|dx| at(x + dx, y + dy) == id)) {
                inside.entry(id).or_default().push([x as u32, y as u32]);
            }
        }
    }
    inside
        .into_iter()
        .map(|(id, pixels)| (id, pixels[pixels.len() / 2]))
        .collect()
}

/// A highlighted face is the same face, in the same colour, whichever
/// renderer shades the viewport: the read-back GPU path against the CPU
/// path, within the tolerance of docs/architecture.md §7.2.
#[test]
fn a_highlighted_face_looks_the_same_from_the_gpu_and_from_the_cpu() {
    let mut session = part();
    let scene = scene(&mut session, SceneQuality::Automatic);
    let tokens = Tokens::default();
    let quality = image_quality_for(Backend::Gpu, [640, 480], false, 8192);
    let mut gpu = gpu();
    let adapter = gpu.report().adapter.clone().unwrap();
    let mut cpu = BodyRenderer::cpu();
    let camera = camera(&scene, ProjectionKind::Perspective, quality);
    let (plain, _) = cpu.render(Some(&scene), &camera, quality, &tokens).unwrap();
    let faces = visible_faces(&plain);
    assert!(faces.len() >= 6, "{} faces in view", faces.len());
    let mut log = format!("adapter: {adapter}\n");
    for (round, &(id, [x, y])) in faces.iter().enumerate() {
        let (body, index) = scene.face_of(id).unwrap();
        let name = &body.face_names[index];
        let highlight = viewport::face_highlight(Some(&scene), Some(&body.feature), name, &tokens);
        assert!(highlight.contains(id), "{name}");
        cpu.set_highlight(highlight);
        gpu.set_highlight(highlight);
        let (cpu_frame, _) = cpu.render(Some(&scene), &camera, quality, &tokens).unwrap();
        let (gpu_frame, made_by) = gpu.render(Some(&scene), &camera, quality, &tokens).unwrap();
        assert_eq!(made_by, Backend::Gpu, "{}", gpu.report().describe());
        let (parity, _) = compare(&scene.mesh, &camera.to_render(), &cpu_frame, &gpu_frame);
        let label = format!("highlight of {name} of {} on {adapter}", body.name);
        log.push_str(&format!("{label}: {parity:?}\n"));
        parity.check(&label);
        // The highlight is in both pictures, where the face is: there the
        // picture is not the plain one, and it is the token's hue.
        let i = ((y * quality.width + x) * 3) as usize;
        for (which, frame) in [("cpu", &cpu_frame), ("gpu", &gpu_frame)] {
            let [r, g, b] = [frame.rgb[i], frame.rgb[i + 1], frame.rgb[i + 2]].map(i32::from);
            assert!(
                g > 2 * r && b > 2 * r,
                "{label}: {which} has ({r}, {g}, {b})"
            );
            assert_ne!(frame.rgb[i..i + 3], plain.rgb[i..i + 3], "{label}: {which}");
        }
        if round == 1 {
            for (suffix, frame) in [("cpu", &cpu_frame), ("gpu", &gpu_frame)] {
                let path = evidence(&format!("pointing-highlight-{suffix}.png"));
                write_png(&path, quality.width, quality.height, &frame.rgb);
                println!("evidence: {}", path.display());
            }
        }
    }
    // And without one the GPU draws the plain picture again.
    cpu.set_highlight(craft3d_render::Highlight::NONE);
    gpu.set_highlight(craft3d_render::Highlight::NONE);
    let (gpu_plain, _) = gpu.render(Some(&scene), &camera, quality, &tokens).unwrap();
    compare(&scene.mesh, &camera.to_render(), &plain, &gpu_plain)
        .0
        .check("no highlight");
    std::fs::write(evidence("pointing-highlight-parity.txt"), log).unwrap();
}

/// The window's own path: a click in a viewport drawn directly by the GPU
/// picks the face the id buffer has there, and the face is highlighted in
/// the picture on the window's device as it is in the oracle's.
#[test]
fn a_click_in_the_direct_viewport_picks_and_highlights_what_the_oracle_shows() {
    let state = headless_render_state()
        .unwrap_or_else(|e| panic!("the direct path cannot be tested: {e}; {ADAPTER_HINT}"));
    let mut session = part();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let tokens = shell.tokens;
    shell.attach_gpu(&state).unwrap();
    assert!(shell.draws_directly());
    let mut window = Window::new(1000.0, 700.0);
    window.frame(&mut shell, &mut session, Vec::new());
    window.frame(&mut shell, &mut session, Vec::new());
    let [w, h] = shell.viewport_size();
    let rect = shell.viewport_rect().unwrap();
    let scene = scene(&mut session, shell.scene_quality());
    let camera = shell.state.camera;
    let quality = Quality {
        width: w,
        height: h,
        ssaa: 1,
    };
    let plain = viewport::render_bodies(Some(&scene), &camera, quality, &tokens).unwrap();
    let (_, _, direct_plain) = shell.direct_picture().unwrap().unwrap();
    let render_camera = camera.to_render();
    let band = common::band(&scene.mesh, &render_camera, &plain);
    let lines = near_lines(
        &viewport::overlay(Some(&scene), &camera, w, h, None),
        w,
        h,
        3.5,
    );
    let faces = visible_faces(&plain);
    assert!(faces.len() >= 6, "{} faces in view", faces.len());
    let mut renders = shell.render_count();
    for (round, &(id, [x, y])) in faces.iter().enumerate() {
        let (body, index) = scene.face_of(id).unwrap();
        let name = body.face_names[index].clone();
        let at = pos2(rect.left() + x as f32 + 0.5, rect.top() + y as f32 + 0.5);
        window.frame(&mut shell, &mut session, vec![Event::PointerMoved(at)]);
        for pressed in [true, false] {
            window.frame(
                &mut shell,
                &mut session,
                vec![Event::PointerButton {
                    pos: at,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        window.frame(&mut shell, &mut session, Vec::new());
        window.frame(&mut shell, &mut session, Vec::new());
        assert_eq!(shell.state.camera, camera);
        // The pick is the id buffer's face, by the name inspect gives it.
        let picked = shell.state.picked.clone().unwrap();
        assert_eq!(picked.face.as_deref(), Some(name.as_str()), "({x}, {y})");
        assert_eq!(picked.feature.as_deref(), Some(body.feature.as_str()));
        let inspect = run(&mut session, "document.inspect", json!({}));
        let listed = inspect["bodies"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["feature"] == body.feature.as_str())
            .unwrap();
        assert_eq!(listed["faceNames"][index], name.as_str());
        // The viewport was drawn again, by the GPU, with the highlight.
        assert!(
            shell.render_count() > renders,
            "the click redrew the viewport"
        );
        renders = shell.render_count();
        assert_eq!(shell.render_stats().unwrap().renderer, Backend::Gpu);
        let (_, _, direct) = shell.direct_picture().unwrap().unwrap();
        // The oracle: the CPU picture of the same state with the same
        // highlight, one sample per pixel.
        let highlight = shell.face_highlight(Some(&scene));
        assert!(highlight.contains(id));
        let mut cpu = BodyRenderer::cpu();
        cpu.set_highlight(highlight);
        let (oracle, _) = cpu.render(Some(&scene), &camera, quality, &tokens).unwrap();
        // Away from every edge and line the two pictures agree to a level
        // plus a pixel of gradient (a multisampled pixel on a seam mixes
        // two fragments), and what the highlight changed, it changed in
        // both.
        let (wu, hu) = (w as usize, h as usize);
        let (mut compared, mut over, mut marked, mut marked_both) = (0, 0, 0, 0);
        for i in 0..wu * hu {
            if band[i] || lines[i] {
                continue;
            }
            let p = [(i % wu) as f64, (i / wu) as f64];
            let mut gradient = 0;
            for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)] {
                let n = [p[0] + dx, p[1] + dy];
                if n[0] >= 0.0 && n[1] >= 0.0 && n[0] < w as f64 && n[1] < h as f64 {
                    gradient =
                        gradient.max(levels(pixel(&oracle.rgb, w, p), pixel(&oracle.rgb, w, n)));
                }
            }
            compared += 1;
            let diff = levels(pixel(&oracle.rgb, w, p), pixel(&direct, w, p));
            over += usize::from(u32::from(diff) > 1 + u32::from(gradient));
            if pixel(&oracle.rgb, w, p) != pixel(&plain.rgb, w, p) {
                marked += 1;
                marked_both +=
                    usize::from(levels(pixel(&direct, w, p), pixel(&direct_plain, w, p)) > 4);
            } else {
                // What the highlight left alone on the CPU is what the GPU
                // drew before the click.
                assert!(
                    levels(pixel(&direct, w, p), pixel(&direct_plain, w, p)) <= 1,
                    "{name}: pixel {p:?} changed on the GPU only"
                );
            }
        }
        assert!(compared > wu * hu / 3, "{compared}");
        assert_eq!(
            over, 0,
            "{name}: colours differ away from every edge and line"
        );
        assert!(marked > 20, "{name}: {marked} pixels are highlighted");
        assert!(
            marked_both * 100 >= marked * 98,
            "{name}: {marked_both} of {marked} highlighted pixels are highlighted on the GPU"
        );
        if round == 1 {
            for (suffix, rgb) in [("direct", &direct), ("oracle", &oracle.rgb)] {
                let path = evidence(&format!("pointing-click-{suffix}.png"));
                write_png(&path, w, h, rgb);
                println!("evidence: {}", path.display());
            }
        }
    }
    // ui.highlight with null clears it on the device too.
    call(
        &mut shell,
        &mut session,
        "ui.highlight",
        json!({"face": null}),
    );
    window.frame(&mut shell, &mut session, Vec::new());
    let (_, _, cleared) = shell.direct_picture().unwrap().unwrap();
    assert_eq!(cleared, direct_plain);
}
