//! A picture of the viewport as the window would show it, without a
//! window: the shell runs headlessly (egui with no backend), the image it
//! uploads as the viewport's texture is taken, and the lines it draws
//! over that texture are inked on it. The model is shaded by the renderer
//! the window would use: the GPU when there is one, else the CPU.
//!
//! ```sh
//! cargo run --release -p craft3d-ui-egui --example snapshot -- \
//!     --out ui.png --size 1440x900 --scale 2 --open bracket.3dcraft \
//!     --script '[["ui.set", {"view": "iso"}]]' --renderer auto
//! ```
//!
//! `--size` is the viewport in points, `--scale` the pixels per point
//! (1). `--script` is a JSON list of control-protocol calls
//! `[method, params]`: the `ui.*` methods, and `engine.execute` with
//! `{"command": id, "params": {..}}`. `--renderer cpu` asks for the CPU
//! renderer (what `CRAFT3D_RENDERER=cpu` does for the window). The
//! panels are not in the picture: only the viewport is.

use craft3d_engine::{Access, Session, Workspace};
use craft3d_ui_egui::viewport::{self, Composed};
use craft3d_ui_egui::{RendererChoice, Shell};
use egui::{Pos2, vec2};
use serde_json::Value;
use std::process::ExitCode;
use std::time::Instant;

struct Options {
    out: String,
    size: [f32; 2],
    scale: f32,
    open: Option<String>,
    script: Vec<(String, Value)>,
    renderer: RendererChoice,
}

fn options() -> Result<Options, String> {
    let mut options = Options {
        out: "snapshot.png".to_string(),
        size: [1280.0, 800.0],
        scale: 1.0,
        open: None,
        script: Vec::new(),
        renderer: RendererChoice::Auto,
    };
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| format!("`{flag}` needs a value"))?;
        match flag.as_str() {
            "--out" => options.out = value,
            "--size" => {
                let (w, h) = value
                    .split_once('x')
                    .ok_or("`--size` is WIDTHxHEIGHT, in points")?;
                let side = |text: &str| {
                    text.parse::<f32>()
                        .ok()
                        .filter(|v| (16.0..=8192.0).contains(v))
                        .ok_or("`--size` sides are numbers from 16 to 8192")
                };
                options.size = [side(w)?, side(h)?];
            }
            "--scale" => {
                options.scale = value
                    .parse::<f32>()
                    .ok()
                    .filter(|s| (0.25..=8.0).contains(s))
                    .ok_or("`--scale` is a number from 0.25 to 8")?;
            }
            "--open" => options.open = Some(value),
            "--script" => {
                let calls: Vec<(String, Value)> = serde_json::from_str(&value)
                    .map_err(|e| format!("`--script` is a JSON list of [method, params]: {e}"))?;
                options.script = calls;
            }
            "--renderer" => {
                options.renderer =
                    RendererChoice::parse(&value).map_err(|e| format!("--renderer: {e}"))?;
            }
            other => return Err(format!("unknown flag `{other}`")),
        }
    }
    Ok(options)
}

fn snapshot(options: &Options) -> Result<(), String> {
    // The file is the user's own, read with the user's authority, like
    // one picked in the window's dialog.
    let mut session = Session::new(Workspace {
        read: Access::Unrestricted,
        write: Access::None,
    });
    if let Some(file) = &options.open {
        session
            .execute("file.open", &serde_json::json!({ "path": file }))
            .map_err(|e| e.message().to_string())?;
    }
    let started = Instant::now();
    let mut shell = Shell::with_renderer(options.renderer);
    eprintln!(
        "snapshot: renderer {} ({:.0} ms to open)",
        shell.renderer_report().describe(),
        started.elapsed().as_secs_f64() * 1000.0
    );
    for (method, params) in &options.script {
        let outcome = match method.as_str() {
            "engine.execute" => {
                let command = params
                    .get("command")
                    .and_then(Value::as_str)
                    .ok_or("engine.execute: `command` must be a string")?;
                let empty = Value::Object(Default::default());
                session
                    .execute(command, params.get("params").unwrap_or(&empty))
                    .map_err(|e| e.message().to_string())
            }
            _ => shell
                .handle_method(&mut session, method, Some(params))
                .ok_or_else(|| format!("`{method}` is not a method of the shell"))?,
        };
        outcome.map_err(|e| format!("{method}: {e}"))?;
    }

    // Three frames: the panels are laid out in the first, the viewport
    // has its size from the second, and the image of it is on screen by
    // the third (made where it is drawn: no thread, nothing to wait for).
    let ctx = egui::Context::default();
    // Every whole image uploaded as a texture (egui's font atlas among
    // them): the viewport's is the one of the overlay's size.
    let mut uploads: Vec<(usize, usize, Vec<u8>)> = Vec::new();
    let mut time = 0.0;
    for _ in 0..3 {
        time += 1.0 / 60.0;
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                Pos2::ZERO,
                vec2(options.size[0], options.size[1]),
            )),
            time: Some(time),
            ..Default::default()
        };
        input
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .native_pixels_per_point = Some(options.scale);
        let mut output = ctx.run_ui(input, |ui| shell.ui(ui, &mut session));
        for delta in output
            .textures_delta
            .set
            .values()
            .flat_map(|deltas| deltas.iter())
            .filter(|delta| delta.pos.is_none())
        {
            let egui::ImageData::Color(image) = &delta.image;
            uploads.push((
                image.width(),
                image.height(),
                image
                    .pixels
                    .iter()
                    .flat_map(|p| [p.r(), p.g(), p.b()])
                    .collect(),
            ));
        }
        output.textures_delta.clear();
    }
    if let Some(problem) = shell.problem() {
        eprintln!("snapshot: {problem}");
    }
    let (overlay, [overlay_width, overlay_height]) = shell.overlay();
    let (_, _, rgb) = uploads
        .into_iter()
        .rev()
        .find(|(w, h, _)| (*w, *h) == (overlay_width as usize, overlay_height as usize))
        .ok_or("the shell uploaded no viewport image")?;
    let stats = shell.render_stats().ok_or("nothing was rendered")?;
    let composed = Composed {
        width: overlay_width,
        height: overlay_height,
        rgb,
        overlay: overlay.clone(),
        coverage: 0.0,
        triangles: stats.triangles,
        backend: stats.renderer,
    };
    let picture = viewport::rasterise(composed, &shell.tokens, f64::from(options.scale));
    let png = craft3d_render::encode_png(picture.width, picture.height, &picture.rgb)
        .map_err(|e| e.to_string())?;
    std::fs::write(&options.out, png).map_err(|e| format!("cannot write {}: {e}", options.out))?;
    eprintln!(
        "snapshot: {} ({}x{} pixels, {} rendered {}x{} ssaa {} in {:.1} ms, {} triangles, sharp {})",
        options.out,
        picture.width,
        picture.height,
        stats.renderer.name(),
        stats.width,
        stats.height,
        stats.ssaa,
        stats.milliseconds.unwrap_or(f64::NAN),
        stats.triangles,
        stats.sharp
    );
    Ok(())
}

fn main() -> ExitCode {
    match options().and_then(|options| snapshot(&options)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("snapshot: {error}");
            ExitCode::FAILURE
        }
    }
}
