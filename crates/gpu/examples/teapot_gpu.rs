//! Renders the Utah teapot with the wgpu renderer and reports timings.
//!
//! ```sh
//! cargo run --release -p craft3d-gpu --example teapot_gpu -- --out target/evidence/gpu/teapot.png
//! ```
//!
//! Options: `--out <png>`, `--size <W>x<H>`, `--ssaa <1..4>`,
//! `--tolerance <model units>` (smaller is more triangles), `--frames <n>`
//! (draws to average over), `--cpu <png>` (also render with the CPU
//! reference renderer, for comparison). `WGPU_BACKEND` and
//! `WGPU_ADAPTER_NAME` choose the adapter.

use craft3d_gpu::{Camera, Offscreen, RenderParams};
use craft3d_math::Vec3;
use craft3d_tess::{TessParams, tessellate_patches};
use craft3d_testkit::teapot::teapot;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::Instant;

fn write(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, bytes).map_err(|e| format!("cannot write {}: {e}", path.display()).into())
}

fn ms(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1e3
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut out = PathBuf::from("target/evidence/gpu/teapot.png");
    let mut cpu_out: Option<PathBuf> = None;
    let mut frames = 20_u32;
    let mut params = RenderParams {
        width: 1280,
        height: 800,
        ..RenderParams::default()
    };
    let mut tess = TessParams {
        chordal_tolerance: 0.002,
        ..TessParams::default()
    };

    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let value = args.next().ok_or_else(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--out" => out = PathBuf::from(value),
            "--cpu" => cpu_out = Some(PathBuf::from(value)),
            "--size" => {
                let (w, h) = value
                    .split_once('x')
                    .ok_or_else(|| format!("--size expects WxH, got `{value}`"))?;
                params.width = w.parse().map_err(|e| format!("bad width `{w}`: {e}"))?;
                params.height = h.parse().map_err(|e| format!("bad height `{h}`: {e}"))?;
            }
            "--tolerance" => {
                tess.chordal_tolerance = value
                    .parse()
                    .map_err(|e| format!("bad tolerance `{value}`: {e}"))?;
            }
            "--ssaa" => {
                params.ssaa = value
                    .parse()
                    .map_err(|e| format!("bad ssaa `{value}`: {e}"))?
            }
            "--frames" => {
                frames = value
                    .parse()
                    .map_err(|e| format!("bad frame count `{value}`: {e}"))?
            }
            other => return Err(format!("unknown option `{other}`").into()),
        }
    }

    let mesh = tessellate_patches(&teapot()?, &tess)?;
    let aspect = f64::from(params.width) / f64::from(params.height.max(1));
    let camera = Camera::framing(
        &mesh.bbox(),
        Vec3::new(0.35, -1.0, 0.45),
        Vec3::Z,
        30.0,
        aspect,
    )
    .ok_or("cannot frame an empty model")?;

    let started = Instant::now();
    let mut offscreen = Offscreen::new()?;
    let device_ms = ms(started);
    let adapter = offscreen.adapter().to_string();
    let renderer = offscreen.renderer();

    let started = Instant::now();
    let uploaded = renderer.upload(&mesh)?;
    let upload_ms = ms(started);

    // The first frame allocates the targets and compiles the pipelines.
    let started = Instant::now();
    renderer.draw(&uploaded, &camera, &params)?;
    let frame = renderer.read_frame()?;
    let first_ms = ms(started);

    // Steady state, as a viewport would draw: the camera moves, the mesh
    // stays on the GPU. Each draw is waited for.
    let wait = |renderer: &craft3d_gpu::Renderer| {
        renderer
            .device()
            .poll(wgpu::PollType::wait_indefinitely())
            .map(|_| ())
            .map_err(|e| e.to_string())
    };
    let started = Instant::now();
    for i in 0..frames {
        let swing = f64::from(i) * 0.01;
        let moved = Camera {
            eye: camera.eye + Vec3::new(swing, 0.0, 0.0),
            ..camera
        };
        renderer.draw(&uploaded, &moved, &params)?;
        wait(renderer)?;
    }
    let draw_ms = ms(started) / f64::from(frames.max(1));

    let started = Instant::now();
    for _ in 0..frames {
        renderer.draw(&uploaded, &camera, &params)?;
        renderer.read_frame()?;
    }
    let draw_read_ms = ms(started) / f64::from(frames.max(1));

    write(&out, &frame.to_png()?)?;
    let mut cpu_ms = None;
    if let Some(path) = &cpu_out {
        let started = Instant::now();
        let cpu = craft3d_render::render(&mesh, &camera, &params)?;
        cpu_ms = Some(ms(started));
        write(path, &cpu.to_png()?)?;
    }

    println!(
        "{{\"out\":\"{}\",\"adapter\":\"{adapter}\",\"triangles\":{},\"drawnTriangles\":{},\
         \"size\":[{},{}],\"ssaa\":{},\"coverage\":{:.4},\"visiblePatches\":{},\
         \"deviceMs\":{device_ms:.1},\"uploadMs\":{upload_ms:.1},\"firstFrameMs\":{first_ms:.1},\
         \"drawMs\":{draw_ms:.2},\"drawAndReadMs\":{draw_read_ms:.2},\"cpuMs\":{}}}",
        out.display(),
        mesh.triangles.len(),
        uploaded.triangle_count(),
        frame.width,
        frame.height,
        params.ssaa,
        frame.coverage(),
        frame.visible_faces().len(),
        cpu_ms.map_or("null".to_string(), |v| format!("{v:.1}")),
    );
    Ok(())
}
