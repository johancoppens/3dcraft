//! The cost of layer 3.5's solid textures on a real model: a studio
//! frame of the model as it is and with presets that carry a texture on
//! a few of its bodies, on every adapter's readback renderer (the
//! numbers for the dev log; docs/design/rendering.md, *Layer 3.5*).
//!
//! The model is `CRAFT3D_COST_MODEL` (a `.3dcraft` file, e.g. the
//! turbofan or the walker, outside the repository) or, without it, the
//! gear example; the bodies are `CRAFT3D_COST_BODIES` (features, comma
//! separated) or the first four. Needs a graphics adapter (it fails
//! without one, as every GPU test here).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

#[path = "../../gpu/tests/common/mod.rs"]
mod common;

use common::{offscreens, params, perspective};
use craft3d_engine::{Session, Workspace};
use craft3d_gpu::Studio;
use craft3d_math::Vec3;
use craft3d_render::studio::{Look, bundled::environment};
use serde_json::{Value, json};
use std::path::PathBuf;

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn model() -> Session {
    if let Ok(path) = std::env::var("CRAFT3D_COST_MODEL") {
        let path = PathBuf::from(path);
        let dir = path.parent().unwrap().to_path_buf();
        let mut s = Session::new(Workspace {
            read: craft3d_engine::Access::Root(dir),
            write: craft3d_engine::Access::None,
        });
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        run(&mut s, "file.open", json!({"path": name}));
        return s;
    }
    let docs = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/examples");
    let text = std::fs::read_to_string(docs.join("gear.json")).unwrap();
    let script: Value = serde_json::from_str(&text).unwrap();
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    for step in script.as_array().unwrap() {
        let params = step.get(1).cloned().unwrap_or_else(|| json!({}));
        run(&mut s, step[0].as_str().unwrap(), params);
    }
    s
}

#[test]
fn solid_textures_on_a_real_model_cost_this_much() {
    let mut s = model();
    let plain = s.scene(None).unwrap().unwrap();
    let bodies: Vec<String> = match std::env::var("CRAFT3D_COST_BODIES") {
        Ok(list) => list.split(',').map(str::to_string).collect(),
        Err(_) => plain
            .bodies
            .iter()
            .take(4)
            .map(|b| b.feature.clone())
            .collect(),
    };
    let presets = ["carbon", "cast-iron", "oak", "marble", "walnut", "granite"];
    for (k, body) in bodies.iter().enumerate() {
        run(
            &mut s,
            "material.preset",
            json!({"body": body, "preset": presets[k % presets.len()]}),
        );
    }
    let textured = s.scene(None).unwrap().unwrap();
    let triangles = textured.mesh.triangles.len();
    let env = environment("studio").unwrap();
    let p = params(1280, 800, 1);
    let camera = perspective(&textured.mesh, Vec3::new(1.0, -1.4, 0.9), 35.0, &p);
    for mut offscreen in offscreens() {
        let adapter = offscreen.adapter().to_string();
        for (label, scene) in [("as it is", &plain), ("textured", &textured)] {
            for (look_label, look) in [
                ("layer 1", Look::default()),
                (
                    "simpler (WebGL2)",
                    Look {
                        ao: false,
                        shadows: false,
                        ..Look::default()
                    },
                ),
            ] {
                let studio = Studio {
                    env: env.clone(),
                    look,
                };
                let renderer = offscreen.renderer();
                let uploaded = renderer
                    .upload_colored(&scene.mesh, &scene.face_colors)
                    .unwrap();
                renderer
                    .draw_studio(&uploaded, &camera, &p, &studio)
                    .unwrap();
                renderer.read_frame().unwrap();
                let started = std::time::Instant::now();
                for _ in 0..6 {
                    renderer
                        .draw_studio(&uploaded, &camera, &p, &studio)
                        .unwrap();
                    renderer.read_frame().unwrap();
                }
                let ms = started.elapsed().as_secs_f64() * 1000.0 / 6.0;
                eprintln!(
                    "COST readback {adapter}: {triangles} triangles, {} textured bodies, 1280x800, {look_label}, {label}: {ms:.2} ms a picture (with readback)",
                    bodies.len()
                );
            }
        }
    }
}
