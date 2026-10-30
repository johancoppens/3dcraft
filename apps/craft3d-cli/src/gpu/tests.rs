//! Oracle 7 (docs/testing.md) for whole commands: the pictures that
//! `view.render` and `view.replay` draw on the headless GPU renderer,
//! each held to the CPU renderer's picture of the same mesh, camera and
//! parameters with the GPU crate's own rules (`crates/gpu/tests/common`
//! for the plain look, the layer-2 rules of `crates/gpu/tests/polish.rs`
//! for the studio look). And, ignored unless asked for, the frames per
//! second at 1080p of both renderers (`fps_at_1080p`).

use super::GpuPictures;
use craft3d_engine::pictures::{PictureRenderer, Shading};
use craft3d_engine::{Access, Session, Workspace};
use craft3d_gpu::{Camera, FaceColors, Frame, Mesh, RenderParams};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[path = "../../../../crates/gpu/tests/common/mod.rs"]
mod common;

/// The layer-2 tolerance of `crates/gpu/tests/polish.rs` (where it is
/// derived and measured): outside the band and the pixels in doubt, at
/// most this share over one level, none over [`MOST_LEVELS`], at most
/// [`MOST_DOUBT`] of the pixels in doubt, a pixel in doubt when more than
/// [`DOUBT_SHARE`] of its light is.
const SHARE_OVER_ONE: f64 = 1.0 / 512.0;
const MOST_LEVELS: u8 = 8;
const DOUBT_SHARE: f32 = 1.0 / 64.0;
const MOST_DOUBT: f64 = 1.0 / 32.0;

/// Per picture: what was compared, and what broke the rules.
type Log = Arc<Mutex<Vec<(String, Option<String>)>>>;

/// The GPU renderer, each picture compared with the CPU's.
struct Checking {
    gpu: GpuPictures,
    log: Log,
}

fn studio_failure(
    mesh: &Mesh,
    camera: &Camera,
    cpu: &Frame,
    doubt: &[f32],
    gpu: &Frame,
) -> Option<String> {
    let band = common::band(mesh, camera, cpu);
    let (mut compared, mut over_one, mut most, mut in_doubt, mut ids) =
        (0usize, 0usize, 0u8, 0usize, 0usize);
    for (i, &in_band) in band.iter().enumerate().take(cpu.ids.len()) {
        if in_band {
            continue;
        }
        if doubt[i] > DOUBT_SHARE {
            in_doubt += 1;
            continue;
        }
        compared += 1;
        if cpu.ids[i] != gpu.ids[i] {
            ids += 1;
        }
        let d = (0..3)
            .map(|k| cpu.rgb[3 * i + k].abs_diff(gpu.rgb[3 * i + k]))
            .max()
            .unwrap();
        if d > 1 {
            over_one += 1;
        }
        most = most.max(d);
    }
    let stats = format!(
        "compared {compared}, over one level {over_one}, most {most}, in doubt {in_doubt}, ids {ids}"
    );
    let all = compared + in_doubt;
    if compared == 0
        || ids != 0
        || over_one as f64 > SHARE_OVER_ONE * compared as f64
        || most > MOST_LEVELS
        || in_doubt as f64 > MOST_DOUBT * all.max(1) as f64
    {
        Some(stats)
    } else {
        None
    }
}

impl PictureRenderer for Checking {
    fn describe(&mut self) -> Result<String, String> {
        self.gpu.describe()
    }

    fn render(
        &mut self,
        mesh: &Mesh,
        colors: &FaceColors,
        camera: &Camera,
        params: &RenderParams,
        shading: Shading<'_>,
        key: Option<u64>,
    ) -> Result<Frame, String> {
        let gpu = self
            .gpu
            .render(mesh, colors, camera, params, shading, key)?;
        let (label, failure) = match shading {
            Shading::Plain => {
                let cpu = craft3d_render::render_colored(mesh, colors, camera, params).unwrap();
                let (parity, _) = common::compare_with(mesh, camera, params, &cpu, &gpu);
                ("plain", parity.failure("plain"))
            }
            Shading::Studio(look, env) => {
                let (cpu, doubt) = craft3d_render::studio::render_studio_doubt(
                    mesh,
                    colors,
                    camera,
                    params,
                    look,
                    env,
                    craft3d_render::studio::polish::TRACE_DOUBT,
                )
                .unwrap();
                ("studio", studio_failure(mesh, camera, &cpu, &doubt, &gpu))
            }
            Shading::Analysed(_) => ("analysed", None),
        };
        self.log.lock().unwrap().push((
            format!("{label} {}x{}", params.width, params.height),
            failure,
        ));
        Ok(gpu)
    }
}

fn workdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("craft3d-cli-gpu-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {}", e.message()))
}

/// A plate with a hole, a glossy ball and a coloured post: flat, curved
/// and freeform faces, colours and finishes.
fn part(s: &mut Session) {
    run(s, "file.new", json!({}));
    run(s, "variable.set", json!({"name": "w", "expr": "20 mm"}));
    run(
        s,
        "feature.block",
        json!({"name": "Plate", "size": ["w", "w", 5]}),
    );
    run(s, "sketch.create", json!({"name": "Hole"}));
    run(
        s,
        "sketch.circle",
        json!({"sketch": "Hole", "center": [10, 10], "radius": 4}),
    );
    run(
        s,
        "feature.extrude",
        json!({"sketch": "Hole", "distance": 10, "cut": "Plate", "name": "Drilled"}),
    );
    run(
        s,
        "feature.sphere",
        json!({"radius": 4, "position": [30, 10, 4], "name": "Ball"}),
    );
    run(
        s,
        "body.appearance",
        json!({"body": "Ball", "color": "#c8102e", "metallic": 1, "roughness": 0.2}),
    );
    run(
        s,
        "feature.cylinder",
        json!({"radius": 2, "height": 12, "position": [5, 25, 0], "name": "Post"}),
    );
    run(
        s,
        "body.appearance",
        json!({"body": "Post", "color": [47, 111, 214], "roughness": 0.6}),
    );
}

#[test]
fn headless_gpu_pictures_match_the_cpu_renderer() {
    let dir = workdir("parity");
    let mut s = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    let log = Arc::new(Mutex::new(Vec::new()));
    s.set_picture_renderer(Box::new(Checking {
        gpu: GpuPictures::default(),
        log: Arc::clone(&log),
    }));
    part(&mut s);
    let renders = [
        json!({"width": 320, "height": 240}),
        json!({"width": 257, "height": 193, "view": "top", "projection": "orthographic"}),
        json!({"width": 320, "height": 240, "look": {}}),
        json!({"width": 320, "height": 240, "look": {"environment": "outdoor", "accumulate": 4}}),
        json!({"width": 320, "height": 240, "look": {"reflections": false, "bloom": 0, "shadows": false}}),
    ];
    for params in renders {
        let result = run(&mut s, "view.render", params.clone());
        assert_eq!(result["renderer"], "gpu", "{params}: {result}");
        assert!(result["adapter"].is_string(), "{result}");
    }
    // The CPU renderer still draws when asked, and says so.
    let cpu = run(
        &mut s,
        "view.render",
        json!({"width": 64, "height": 48, "renderer": "cpu"}),
    );
    assert_eq!(cpu["renderer"], "cpu");
    // A replay of the history and an animation, studio and orbiting.
    let replay = run(
        &mut s,
        "view.replay",
        json!({"dir": "history", "width": 160, "height": 120, "orbit": 90, "look": {"accumulate": 2}}),
    );
    assert_eq!(replay["renderer"], "gpu", "{replay}");
    let animated = run(
        &mut s,
        "view.replay",
        json!({"dir": "sweep", "width": 160, "height": 120, "frames": 4,
               "animate": [{"variable": "w", "from": 14, "to": 26, "unit": "mm"}],
               "camera": {"dolly": 0.8}}),
    );
    assert_eq!(animated["renderer"], "gpu", "{animated}");
    assert_eq!(animated["frames"], 4);

    let log = log.lock().unwrap();
    let failed: Vec<String> = log
        .iter()
        .filter_map(|(label, failure)| failure.as_ref().map(|f| format!("{label}: {f}")))
        .collect();
    eprintln!("{} pictures compared", log.len());
    assert!(log.len() >= 5 + 5 + 4, "{} pictures", log.len());
    assert!(failed.is_empty(), "{}", failed.join("\n"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The frames per second of `view.render` at 1920 × 1080 on both
/// renderers, plain and studio, on a model of realistic size (the
/// benchmark's engine block), and of a GPU replay to PNG files: for the
/// dev log and docs/development.md. `cargo test -p craft3d-cli -- --ignored
/// --nocapture fps_at_1080p`.
#[test]
#[ignore = "a measurement: run it on the Strix with --ignored --nocapture"]
fn fps_at_1080p() {
    let dir = workdir("fps");
    let mut s = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    s.set_picture_renderer(Box::new(GpuPictures::default()));
    run(&mut s, "file.new", json!({}));
    let steps: Value = serde_json::from_str(include_str!(
        "../../../../crates/engine/tests/bench_models/engine-block.steps.json"
    ))
    .unwrap();
    for step in steps.as_array().unwrap() {
        let id = step[0].as_str().unwrap();
        let params = step.get(1).cloned().unwrap_or_else(|| json!({}));
        run(&mut s, id, params);
    }
    let size = json!({"width": 1920, "height": 1080});
    let mut lines = Vec::new();
    for (look_name, look) in [
        ("plain", Value::Null),
        ("studio", json!({})),
        ("studio x16", json!({"accumulate": 16})),
    ] {
        for (renderer, frames) in [
            ("gpu", 6),
            ("cpu", if look_name == "plain" { 3 } else { 1 }),
        ] {
            if renderer == "cpu" && look_name == "studio x16" {
                // The CPU refuses 16 frames at 1080p (2^26 samples).
                continue;
            }
            let mut params = size.clone();
            params["renderer"] = json!(renderer);
            params["path"] = json!("shot.png");
            if !look.is_null() {
                params["look"] = look.clone();
            }
            // The first picture meshes the model and prepares the
            // environment (and the device): not counted.
            let first = run(&mut s, "view.render", params.clone());
            let clock = std::time::Instant::now();
            for _ in 0..frames {
                run(&mut s, "view.render", params.clone());
            }
            let secs = clock.elapsed().as_secs_f64() / f64::from(frames);
            lines.push(format!(
                "1920x1080 {look_name:<10} {renderer}: {:.3} s a frame, {:.2} fps ({} triangles, {})",
                secs,
                1.0 / secs,
                first["triangles"],
                first["adapter"].as_str().unwrap_or("cpu")
            ));
        }
    }
    for look in [Value::Null, json!({})] {
        let mut params = json!({"dir": "orbit", "width": 1920, "height": 1080, "orbit": 30,
                                "hold": 29, "renderer": "gpu"});
        if !look.is_null() {
            params["look"] = look.clone();
        }
        let clock = std::time::Instant::now();
        let replay = run(&mut s, "view.replay", params);
        let secs = clock.elapsed().as_secs_f64();
        let frames = replay["frames"].as_f64().unwrap();
        lines.push(format!(
            "replay 1920x1080 {} gpu, PNG files included: {frames} frames in {secs:.1} s, {:.2} fps",
            if look.is_null() { "plain" } else { "studio x16" },
            frames / secs
        ));
    }
    for line in &lines {
        println!("{line}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
