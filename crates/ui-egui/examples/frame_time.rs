//! Times whole frames of the shell on the thread that draws them: the
//! first frame, frames while orbiting, the frame after the camera stops
//! (the sharp image) and the frame after an edit.
//!
//! ```sh
//! cargo run --release -p craft3d-ui-egui --example frame_time
//! cargo run --release -p craft3d-ui-egui --example frame_time -- --inline
//! cargo run --release -p craft3d-ui-egui --example frame_time -- --cpu
//! ```
//!
//! `--inline` renders the sharp image on the drawing thread, as a host
//! without threads does; without it the sharp image is rendered on the
//! worker thread, as in the window. `--cpu` renders with the CPU
//! reference renderer; without it the GPU is used when there is one, as
//! in the window.

use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::{RendererChoice, Shell};
use egui::{Event, PointerButton, Pos2, pos2, vec2};
use serde_json::json;
use std::process::ExitCode;
use std::time::{Duration, Instant};

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

struct Window {
    ctx: egui::Context,
    size: egui::Vec2,
    time: f64,
}

impl Window {
    /// One frame; how long the drawing thread was busy with it, in ms.
    fn frame(&mut self, shell: &mut Shell, session: &mut Session, events: Vec<Event>) -> f64 {
        self.time += 1.0 / 60.0;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, self.size)),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let start = Instant::now();
        let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        output.textures_delta.clear();
        elapsed
    }
}

fn summary(label: &str, times: &[f64]) {
    let worst = times.iter().copied().fold(0.0_f64, f64::max);
    let mean = times.iter().sum::<f64>() / times.len().max(1) as f64;
    println!(
        "  {label:<34} mean {mean:>7.1} ms   worst {worst:>7.1} ms   ({} frames)",
        times.len()
    );
}

fn measure(
    large: bool,
    size: [f32; 2],
    inline: bool,
    choice: RendererChoice,
) -> Result<(), String> {
    let mut session = Session::new(Workspace::none());
    build(&mut session, large)?;
    let scene = session
        .scene(None)
        .map_err(|e| e.message().to_string())?
        .ok_or("no document")?;
    println!(
        "{} bodies, {} triangles, window {} x {}, sharp image {}",
        scene.bodies.len(),
        scene.triangles(),
        size[0],
        size[1],
        if inline {
            "on the drawing thread"
        } else {
            "on the worker thread"
        }
    );
    let mut shell = if inline {
        Shell::with_renderer(choice)
    } else {
        Shell::threaded_with(choice)
    };
    println!("renderer: {}", shell.renderer_report().describe());
    let mut window = Window {
        ctx: egui::Context::default(),
        size: vec2(size[0], size[1]),
        time: 0.0,
    };
    let settle = |shell: &mut Shell, window: &mut Window, session: &mut Session| -> Vec<f64> {
        // The frames until the sharp image is on screen. The wait for the
        // worker is not the drawing thread's time: only the frames are.
        let mut times = Vec::new();
        for _ in 0..200 {
            shell.wait_for_sharp(Duration::from_secs(30));
            times.push(window.frame(shell, session, Vec::new()));
            if shell.render_stats().is_some_and(|stats| stats.sharp) {
                break;
            }
        }
        times
    };

    let first = window.frame(&mut shell, &mut session, Vec::new());
    let mut after = settle(&mut shell, &mut window, &mut session);
    after.insert(0, first);
    summary("first frames, until sharp", &after);

    let from = pos2(size[0] * 0.6, size[1] * 0.5);
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
    let mut orbit = Vec::new();
    for step in 1..=40 {
        let at = from + vec2(4.0 * step as f32, 1.5 * step as f32);
        orbit.push(window.frame(&mut shell, &mut session, vec![Event::PointerMoved(at)]));
    }
    summary("orbiting", &orbit);
    let to = from + vec2(160.0, 60.0);
    let release = window.frame(
        &mut shell,
        &mut session,
        vec![Event::PointerButton {
            pos: to,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    let mut stop = settle(&mut shell, &mut window, &mut session);
    stop.insert(0, release);
    summary("after the camera stops, until sharp", &stop);

    session
        .execute("feature.edit", &json!({"feature": "Post", "height": 20}))
        .map_err(|e| e.message().to_string())?;
    let edit = settle(&mut shell, &mut window, &mut session);
    summary("after an edit, until sharp", &edit);
    if let Some(stats) = shell.render_stats() {
        println!(
            "  sharp image: {} x {} ssaa {} on the {}, rendered in {:.1} ms",
            stats.width,
            stats.height,
            stats.ssaa,
            stats.renderer.name(),
            stats.milliseconds.unwrap_or(f64::NAN)
        );
    }
    Ok(())
}

fn main() -> ExitCode {
    let inline = std::env::args().any(|a| a == "--inline");
    let choice = if std::env::args().any(|a| a == "--cpu") {
        RendererChoice::Cpu
    } else {
        RendererChoice::Auto
    };
    let outcome = measure(false, [1280.0, 800.0], inline, choice)
        .and_then(|()| measure(true, [1280.0, 800.0], inline, choice))
        .and_then(|()| measure(true, [2560.0, 1400.0], inline, choice));
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("frame_time: {error}");
            ExitCode::FAILURE
        }
    }
}
