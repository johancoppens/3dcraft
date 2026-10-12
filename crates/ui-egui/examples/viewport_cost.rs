//! Times the CPU viewport: what one image costs at rest and while the
//! camera moves, for a small and a large model.
//!
//! ```sh
//! cargo run --release -p craft3d-ui-egui --example viewport_cost
//! ```

use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::Tokens;
use craft3d_ui_egui::shell::image_quality;
use craft3d_ui_egui::state::CameraState;
use craft3d_ui_egui::viewport::{self, Quality};
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
) -> Result<(), String> {
    let tokens = Tokens::default();
    let runs = 5;
    let mut best = f64::INFINITY;
    for _ in 0..runs {
        let start = Instant::now();
        viewport::render_bodies(Some(scene), camera, quality, &tokens)?;
        best = best.min(start.elapsed().as_secs_f64() * 1000.0);
    }
    println!(
        "  {label:<22} {:>4} x {:<4} ssaa {}  {best:>7.1} ms",
        quality.width, quality.height, quality.ssaa
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
    for size in [[1020, 777], [1652, 993], [2560, 1400]] {
        let mut camera = CameraState::default();
        camera.fit(&scene.bbox, f64::from(size[0]) / f64::from(size[1]));
        time("at rest", &scene, &camera, image_quality(size, false, 8192))?;
        time(
            "while moving",
            &scene,
            &camera,
            image_quality(size, true, 8192),
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
        )?;
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
