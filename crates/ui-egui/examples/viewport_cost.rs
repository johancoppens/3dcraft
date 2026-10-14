//! Times the viewport's renderers: what one image costs at rest and while
//! the camera moves, for a small and a large model, on the CPU and, where
//! there is an adapter, on the GPU: the direct path of the window (the
//! bodies and the lines drawn on the window's device into the texture
//! that is shown, split into the upload of the mesh and the lines, once
//! per document revision, and the frame until the GPU is done), and the
//! path that reads back, split into the upload, the draw and the readback.
//! The direct path and the CPU are timed with the bodies opaque and
//! see-through (two depth layers and a blend), whole and cut open by a
//! section plane across the middle of the model (a pass that counts what
//! is cut away, and the cap); and what moving the plane costs besides: its
//! outline on the bodies and the lines over the picture, made again.
//!
//! ```sh
//! cargo run --release -p craft3d-ui-egui --example viewport_cost
//! ```

use craft3d_engine::{Scene, Session, Workspace};
use craft3d_gpu::{DrawOptions, Onscreen, OverlayParams};
use craft3d_ui_egui::Tokens;
use craft3d_ui_egui::backend::{Backend, BodyRenderer, RendererChoice};
use craft3d_ui_egui::direct::{direct_size, headless_render_state, segments_marked};
use craft3d_ui_egui::shell::{image_quality, image_quality_for};
use craft3d_ui_egui::state::CameraState;
use craft3d_ui_egui::viewport::{self, AXIS_LENGTH, Marks, Quality, SectionCut, depth_slack};
use egui_wgpu::wgpu;
use serde_json::json;
use std::process::ExitCode;
use std::time::Instant;

fn build(session: &mut Session, large: bool) -> Result<(), String> {
    let mut run = |id: &str, params: serde_json::Value| {
        session
            .execute(id, &params)
            .map(|_| ())
            .map_err(|e| e.message().to_string())
    };
    run("file.new", json!({"units": "mm"}))?;
    run(
        "feature.block",
        json!({"name": "Plate", "size": [60, 40, 8]}),
    )?;
    run(
        "feature.cylinder",
        json!({"name": "Post", "position": [15, 20, 8], "radius": 6, "height": 30}),
    )?;
    if large {
        // Curved bodies all over the plate: tori and spheres.
        for row in 0..6 {
            for column in 0..8 {
                let (x, y) = (5.0 + f64::from(column) * 7.0, 4.0 + f64::from(row) * 6.5);
                if (row + column) % 2 == 0 {
                    run(
                        "feature.torus",
                        json!({"major": 2.2, "minor": 0.8, "position": [x, y, 9.0]}),
                    )?;
                } else {
                    run(
                        "feature.sphere",
                        json!({"radius": 2.5, "position": [x, y, 10.5]}),
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn time(
    label: &str,
    scene: &craft3d_engine::Scene,
    camera: &CameraState,
    quality: Quality,
    opacity: f64,
    cut: Option<&SectionCut>,
) -> Result<(), String> {
    let tokens = Tokens::default();
    let runs = 5;
    let mut best = f64::INFINITY;
    let params = craft3d_render::RenderParams {
        opacity,
        section: cut.map(|cut| cut.section),
        ..viewport::render_params(quality, &tokens)
    };
    for _ in 0..runs {
        let start = Instant::now();
        craft3d_render::render(&scene.mesh, &camera.to_render(), &params)
            .map_err(|e| e.to_string())?;
        best = best.min(start.elapsed().as_secs_f64() * 1000.0);
    }
    println!(
        "  {label:<22} {:>4} x {:<4} ssaa {}  {best:>7.1} ms",
        quality.width, quality.height, quality.ssaa
    );
    Ok(())
}

/// The GPU's cost per image, split into its parts, best of five.
fn time_gpu(
    label: &str,
    offscreen: &mut craft3d_gpu::Offscreen,
    scene: &Scene,
    camera: &CameraState,
    quality: Quality,
) -> Result<(), String> {
    let tokens = Tokens::default();
    let params = viewport::render_params(quality, &tokens);
    let render_camera = camera.to_render();
    let (mut upload, mut draw, mut read) = (f64::INFINITY, f64::INFINITY, f64::INFINITY);
    for _ in 0..5 {
        let ms = |start: Instant| start.elapsed().as_secs_f64() * 1000.0;
        let start = Instant::now();
        let mesh = offscreen
            .renderer()
            .upload(&scene.mesh)
            .map_err(|e| e.to_string())?;
        upload = upload.min(ms(start));
        let start = Instant::now();
        offscreen
            .renderer()
            .draw(&mesh, &render_camera, &params)
            .map_err(|e| e.to_string())?;
        draw = draw.min(ms(start));
        let start = Instant::now();
        let frame = offscreen
            .renderer()
            .read_frame()
            .map_err(|e| e.to_string())?;
        read = read.min(ms(start));
        assert_eq!(frame.width, quality.width);
    }
    println!(
        "  {label:<22} {:>4} x {:<4} ssaa {}  upload {upload:>6.1} ms  draw {draw:>6.1} ms  read back {read:>6.1} ms",
        quality.width, quality.height, quality.ssaa
    );
    Ok(())
}

/// The direct path's cost per frame, split into its parts, best of five:
/// the uploads (once per document revision) and the frame, submitted and
/// then waited for.
#[allow(clippy::too_many_arguments)]
fn time_direct(
    label: &str,
    onscreen: &mut Onscreen,
    device: &wgpu::Device,
    scene: &Scene,
    camera: &CameraState,
    size: [u32; 2],
    opacity: f64,
    cut: Option<&SectionCut>,
) -> Result<(), String> {
    let tokens = Tokens::default();
    let [width, height] = direct_size(size, device.limits().max_texture_dimension_2d);
    let marks = Marks {
        cut: cut.cloned(),
        ..Marks::default()
    };
    let params = craft3d_render::RenderParams {
        width,
        height,
        ssaa: 1,
        background: tokens.viewport_background,
        color: tokens.body,
        highlight: craft3d_render::Highlight::NONE,
        opacity,
        section: marks.section(),
    };
    let overlay = OverlayParams {
        slack: depth_slack(Backend::Gpu, Some(scene), camera),
        hidden_opacity: tokens.hidden_opacity,
        axis_length: camera.distance * AXIS_LENGTH,
    };
    let render_camera = camera.to_render();
    let (mut upload, mut submit, mut done) = (f64::INFINITY, f64::INFINITY, f64::INFINITY);
    let mut segment_count = 0;
    for _ in 0..5 {
        let ms = |start: Instant| start.elapsed().as_secs_f64() * 1000.0;
        let start = Instant::now();
        let mesh = onscreen.upload(&scene.mesh).map_err(|e| e.to_string())?;
        let lines = onscreen
            .upload_lines(&segments_marked(
                Some(scene),
                None,
                &tokens,
                1.0,
                &marks,
                opacity < 1.0,
            ))
            .map_err(|e| e.to_string())?;
        segment_count = lines.segment_count();
        upload = upload.min(ms(start));
        let start = Instant::now();
        onscreen
            .draw(
                &mesh,
                &lines,
                &render_camera,
                &params,
                &overlay,
                DrawOptions::default(),
            )
            .map_err(|e| e.to_string())?;
        submit = submit.min(ms(start));
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| e.to_string())?;
        done = done.min(ms(start));
    }
    println!(
        "  {label:<22} {width:>4} x {height:<4} msaa {}  upload {upload:>6.1} ms ({segment_count} segments)  submit {submit:>6.1} ms  GPU done {done:>6.1} ms",
        onscreen.samples()
    );
    Ok(())
}

/// What the viewport's own path costs with either renderer: shading and
/// the overlay, as the window asks for it.
fn time_path(
    label: &str,
    renderer: &mut BodyRenderer,
    scene: &Scene,
    camera: &CameraState,
    quality: Quality,
) -> Result<(), String> {
    let tokens = Tokens::default();
    let mut best = f64::INFINITY;
    for _ in 0..5 {
        let start = Instant::now();
        viewport::compose_with(renderer, Some(scene), camera, quality, &tokens, None)?;
        best = best.min(start.elapsed().as_secs_f64() * 1000.0);
    }
    println!(
        "  {label:<22} {:>4} x {:<4} ssaa {}  {best:>7.1} ms on the {}",
        quality.width,
        quality.height,
        quality.ssaa,
        renderer.backend().name()
    );
    Ok(())
}

fn measure(large: bool, tolerance: Option<f64>) -> Result<(), String> {
    let mut session = Session::new(Workspace::none());
    build(&mut session, large)?;
    let start = Instant::now();
    let scene = session
        .scene(tolerance)
        .map_err(|e| e.message().to_string())?
        .ok_or("no document")?;
    let scene_ms = start.elapsed().as_secs_f64() * 1000.0;
    println!(
        "{} bodies, {} triangles (tolerance {:.4}): scene built in {scene_ms:.1} ms",
        scene.bodies.len(),
        scene.triangles(),
        scene.tolerance
    );
    // A section across z through the middle of the model, as the window
    // makes it: the plane for the renderers, and its outline on the
    // bodies, which is what moving the plane costs besides the frame.
    let (lo, hi) = scene.bbox.corners().ok_or("an empty scene")?;
    let tokens = Tokens::default();
    let section = craft3d_render::Section {
        origin: (lo + hi) * 0.5,
        normal: craft3d_math::Vec3::Z,
        cap: true,
        color: tokens.section_cap,
    };
    let mut outline_ms = f64::INFINITY;
    let mut cut = None;
    for _ in 0..5 {
        let start = Instant::now();
        cut = SectionCut::new(&scene, section);
        outline_ms = outline_ms.min(start.elapsed().as_secs_f64() * 1000.0);
    }
    let cut = cut.ok_or("no section")?;
    let start = Instant::now();
    let moved = segments_marked(
        Some(&scene),
        None,
        &tokens,
        1.0,
        &Marks {
            cut: Some(cut.clone()),
            ..Marks::default()
        },
        false,
    );
    println!(
        "section across z: its outline ({} segments) in {outline_ms:.1} ms, the lines over the picture ({} segments) in {:.1} ms: once per move of the plane",
        cut.outline.len(),
        moved.len(),
        start.elapsed().as_secs_f64() * 1000.0
    );
    for size in [[1020, 777], [1652, 993], [2560, 1400]] {
        let mut camera = CameraState::default();
        camera.fit(&scene.bbox, f64::from(size[0]) / f64::from(size[1]));
        time(
            "at rest",
            &scene,
            &camera,
            image_quality(size, false, 8192),
            1.0,
            None,
        )?;
        time(
            "at rest, see-through",
            &scene,
            &camera,
            image_quality(size, false, 8192),
            0.5,
            None,
        )?;
        time(
            "at rest, section",
            &scene,
            &camera,
            image_quality(size, false, 8192),
            1.0,
            Some(&cut),
        )?;
        time(
            "section, see-through",
            &scene,
            &camera,
            image_quality(size, false, 8192),
            0.5,
            Some(&cut),
        )?;
        time(
            "while moving",
            &scene,
            &camera,
            image_quality(size, true, 8192),
            1.0,
            None,
        )?;
        time(
            "full size, 4 samples",
            &scene,
            &camera,
            Quality {
                width: size[0],
                height: size[1],
                ssaa: 2,
            },
            1.0,
            None,
        )?;
    }
    match headless_render_state() {
        Ok(state) => {
            let mut onscreen = Onscreen::new(&state.device, &state.queue, &state.adapter)
                .map_err(|e| e.to_string())?;
            println!(
                "direct path on {} ({:?})",
                state.adapter.get_info().name,
                state.adapter.get_info().backend
            );
            for size in [[1020, 777], [1280, 800], [2560, 1400]] {
                let mut camera = CameraState::default();
                camera.fit(&scene.bbox, f64::from(size[0]) / f64::from(size[1]));
                for (label, opacity, cut) in [
                    ("every frame", 1.0, None),
                    ("see-through", 0.5, None),
                    ("section", 1.0, Some(&cut)),
                    ("section, see-through", 0.5, Some(&cut)),
                ] {
                    time_direct(
                        label,
                        &mut onscreen,
                        &state.device,
                        &scene,
                        &camera,
                        size,
                        opacity,
                        cut,
                    )?;
                }
            }
        }
        Err(why) => println!("no direct path: {why}"),
    }
    let mut gpu = BodyRenderer::new(RendererChoice::Auto);
    println!("readback path, {}", gpu.report().describe());
    let mut cpu = BodyRenderer::cpu();
    for size in [[1020, 777], [2560, 1400]] {
        let mut camera = CameraState::default();
        camera.fit(&scene.bbox, f64::from(size[0]) / f64::from(size[1]));
        for (backend, renderer) in [(Backend::Cpu, &mut cpu), (Backend::Gpu, &mut gpu)] {
            if renderer.backend() != backend {
                continue;
            }
            for moving in [false, true] {
                let quality = image_quality_for(backend, size, moving, 8192);
                let label = if moving { "while moving" } else { "at rest" };
                time_path(label, renderer, &scene, &camera, quality)?;
            }
        }
    }
    if gpu.backend() == Backend::Gpu {
        let mut offscreen = craft3d_gpu::Offscreen::new().map_err(|e| e.to_string())?;
        println!("GPU parts on {}", offscreen.adapter());
        for size in [[1020, 777], [2560, 1400]] {
            let mut camera = CameraState::default();
            camera.fit(&scene.bbox, f64::from(size[0]) / f64::from(size[1]));
            for moving in [false, true] {
                let quality = image_quality_for(Backend::Gpu, size, moving, 8192);
                let label = if moving { "while moving" } else { "at rest" };
                time_gpu(label, &mut offscreen, &scene, &camera, quality)?;
            }
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let outcome = measure(false, None)
        .and_then(|()| measure(true, None))
        .and_then(|()| measure(true, Some(0.002)));
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("viewport_cost: {error}");
            ExitCode::FAILURE
        }
    }
}
