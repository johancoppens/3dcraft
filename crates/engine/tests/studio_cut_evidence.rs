//! Pictures for the developer to look at (docs/design/rendering.md,
//! *See-through and cut-open pictures in the studio look*): the cutaway
//! turbofan (`bench_models/turbofan.steps.json`) cut open through its axis
//! in the studio look, and a see-through housing over two gears, drawn by
//! the CPU renderer (the oracle) into `target/evidence/studio-cut/` and
//! printed as evidence lines (`craft3d_testkit::evidence`). Ignored: run
//! with `--ignored --nocapture`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_math::Vec3;
use craft3d_render::studio::{Look, bundled::environment, render_studio};
use craft3d_render::{Camera, Projection, RenderParams, Section};
use serde_json::{Value, json};
use std::path::PathBuf;

fn run(session: &mut Session, id: &str, params: &Value) -> Value {
    session
        .execute(id, params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn evidence(name: &str, png: &[u8]) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/studio-cut");
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join(name), png);
    craft3d_testkit::evidence::print(name, png);
}

/// A camera that sees the whole box from `toward` (from the centre to the
/// eye).
fn camera(lo: Vec3, hi: Vec3, toward: Vec3) -> Camera {
    let centre = (lo + hi) * 0.5;
    let radius = (hi - lo).length() * 0.5;
    let toward = toward.normalized().unwrap();
    Camera {
        eye: centre + toward * (radius * 2.6),
        target: centre,
        up: Vec3::Z,
        projection: Projection::Perspective { fov_y_deg: 35.0 },
    }
}

/// Colours the bodies of the session from a palette, in history order,
/// with their finishes.
fn colour_bodies(session: &mut Session, palette: &[(&str, f64, f64)]) {
    let scene = session.scene(None).unwrap().unwrap();
    let names: Vec<String> = scene.bodies.iter().map(|b| b.name.clone()).collect();
    for (k, name) in names.iter().enumerate() {
        let (color, metallic, roughness) = palette[k % palette.len()];
        run(
            session,
            "body.appearance",
            &json!({"body": name, "color": color, "metallic": metallic, "roughness": roughness}),
        );
    }
}

fn picture(
    session: &mut Session,
    name: &str,
    toward: Vec3,
    opacity: f64,
    section: Option<Section>,
    environment_name: &str,
) {
    let scene = session.scene(None).unwrap().unwrap();
    let (lo, hi) = scene.bodies_box.corners().unwrap();
    let params = RenderParams {
        width: 960,
        height: 640,
        ssaa: 1,
        opacity,
        section,
        background: [235, 233, 229],
        ..RenderParams::default()
    };
    let look = Look {
        accumulate: 8,
        ..Look::polished()
    };
    let env = environment(environment_name).unwrap();
    let frame = render_studio(
        &scene.mesh,
        &scene.face_colors,
        &camera(lo, hi, toward),
        &params,
        &look,
        &env,
    )
    .unwrap();
    evidence(name, &frame.to_png().unwrap());
}

#[test]
#[ignore = "pictures for the developer: run with --ignored --nocapture"]
fn the_turbofan_cut_open_in_the_studio_look() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/bench_models/turbofan.steps.json");
    let steps: Vec<(String, Value)> =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut session = Session::new(Workspace::unrestricted());
    run(&mut session, "file.new", &json!({"units": "mm"}));
    for (id, params) in &steps {
        run(&mut session, id, params);
    }
    colour_bodies(
        &mut session,
        &[
            ("#c8ccd2", 1.0, 0.25),
            ("#3a5f8f", 0.0, 0.35),
            ("#d9a441", 1.0, 0.3),
            ("#b84a3a", 0.0, 0.45),
            ("#6f9a5a", 0.0, 0.4),
            ("#9aa0a8", 1.0, 0.4),
        ],
    );
    let scene = session.scene(None).unwrap().unwrap();
    let (lo, hi) = scene.bodies_box.corners().unwrap();
    let centre = (lo + hi) * 0.5;
    // Through the engine's axis (along y), the upper half cut away.
    let axis_cut = Section {
        origin: centre,
        normal: Vec3::new(0.0, 0.0, 1.0),
        cap: true,
        color: [214, 88, 64],
    };
    let toward = Vec3::new(0.55, -1.0, 0.45);
    picture(
        &mut session,
        "turbofan-whole.png",
        toward,
        1.0,
        None,
        "outdoor",
    );
    picture(
        &mut session,
        "turbofan-cut.png",
        toward,
        1.0,
        Some(axis_cut),
        "outdoor",
    );
    picture(
        &mut session,
        "turbofan-cut-studio.png",
        toward,
        1.0,
        Some(axis_cut),
        "studio",
    );
    picture(
        &mut session,
        "turbofan-see-through.png",
        toward,
        0.45,
        None,
        "outdoor",
    );
}

/// The outline of a spur gear with `teeth` trapezoidal teeth between
/// radii `root` and `tip`.
fn gear_outline(cx: f64, cy: f64, root: f64, tip: f64, teeth: usize) -> Vec<[f64; 2]> {
    let mut points = Vec::new();
    let pitch = std::f64::consts::TAU / teeth as f64;
    for k in 0..teeth {
        let a = k as f64 * pitch;
        for (da, r) in [(0.0, root), (0.18, tip), (0.42, tip), (0.6, root)] {
            let t = a + da * pitch;
            points.push([cx + r * t.cos(), cy + r * t.sin()]);
        }
    }
    points
}

#[test]
#[ignore = "pictures for the developer: run with --ignored --nocapture"]
fn a_see_through_housing_over_gears_in_the_studio_look() {
    let mut session = Session::new(Workspace::unrestricted());
    run(&mut session, "file.new", &json!({"units": "mm"}));
    for (k, (cx, root, tip, teeth)) in [(0.0, 18.0, 21.0, 24_usize), (31.5, 8.0, 11.0, 12)]
        .into_iter()
        .enumerate()
    {
        let sketch = format!("Gear{k}");
        run(
            &mut session,
            "sketch.create",
            &json!({"plane": "xy", "offset": "4 mm", "name": sketch}),
        );
        run(
            &mut session,
            "sketch.polyline",
            &json!({"sketch": sketch, "points": gear_outline(cx, 0.0, root, tip, teeth), "closed": true}),
        );
        run(
            &mut session,
            "feature.extrude",
            &json!({"sketch": sketch, "distance": "8 mm"}),
        );
        run(
            &mut session,
            "feature.cylinder",
            &json!({"radius": "3 mm", "height": "22 mm", "position": [format!("{cx} mm"), "0 mm", "-2 mm"]}),
        );
    }
    // The housing: a closed box around both gears.
    run(
        &mut session,
        "feature.block",
        &json!({"size": ["72 mm", "52 mm", "18 mm"], "position": ["-25 mm", "-26 mm", "0 mm"]}),
    );
    colour_bodies(
        &mut session,
        &[
            ("#d9a441", 1.0, 0.3),
            ("#9aa0a8", 1.0, 0.25),
            ("#b84a3a", 0.0, 0.4),
            ("#9aa0a8", 1.0, 0.25),
            ("#3a5f8f", 0.0, 0.3),
        ],
    );
    let toward = Vec3::new(0.5, -1.0, 0.9);
    picture(
        &mut session,
        "gears-housing.png",
        toward,
        1.0,
        None,
        "studio",
    );
    picture(
        &mut session,
        "gears-see-through.png",
        toward,
        0.4,
        None,
        "studio",
    );
    let scene = session.scene(None).unwrap().unwrap();
    let (lo, hi) = scene.bodies_box.corners().unwrap();
    let cut = Section {
        origin: (lo + hi) * 0.5,
        normal: Vec3::new(0.0, 0.0, 1.0),
        cap: true,
        color: [214, 88, 64],
    };
    picture(
        &mut session,
        "gears-cut.png",
        toward,
        1.0,
        Some(cut),
        "studio",
    );
    picture(
        &mut session,
        "gears-cut-see-through.png",
        toward,
        0.5,
        Some(cut),
        "studio",
    );
}
