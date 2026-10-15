//! Timing of the scene a viewer draws at several tolerances (AGENTS.md
//! rule 12): what the display quality costs on a flange with six bolt
//! holes and on bodies with a thousand faces. Ignored by default:
//! `cargo test --release -p craft3d-engine --test scene_bench -- --ignored --nocapture`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{SceneQuality, Session, Workspace};
use serde_json::{Value, json};
use std::time::Instant;

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// The flange of the demo: a disc of radius 40 with a hub, six bolt holes
/// of radius 4, a keyway and two lugs with holes.
pub fn flange() -> Session {
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

/// A plate with `n` by `n` round holes, extruded from one sketch: 6 + n²
/// faces, n² of them cylinders.
pub fn drilled(n: usize) -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "sketch.create", json!({"name": "Plate"}));
    let side = 2.0 * n as f64;
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Plate", "closed": true,
               "points": [[0, 0], [side, 0], [side, side], [0, side]]}),
    );
    for i in 0..n {
        for j in 0..n {
            run(
                &mut s,
                "sketch.circle",
                json!({"sketch": "Plate", "center": [1.0 + 2.0 * i as f64, 1.0 + 2.0 * j as f64],
                       "radius": 0.5}),
            );
        }
    }
    let made = run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Plate", "distance": 1, "name": "Drilled"}),
    );
    assert_eq!(made["status"], "ok", "{made}");
    s
}

/// A plate whose outline is a wavy polygon of `sides` sides with a round
/// hole: `sides + 3` faces, nearly all of them flat.
pub fn polygon(sides: usize) -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({}));
    run(&mut s, "sketch.create", json!({"name": "S"}));
    let points: Vec<[f64; 2]> = (0..sides)
        .map(|i| {
            let a = std::f64::consts::TAU * i as f64 / sides as f64;
            let r = 40.0 + 3.0 * (7.0 * a).sin();
            [r * a.cos(), r * a.sin()]
        })
        .collect();
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "S", "closed": true, "points": points}),
    );
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "S", "center": [0, 0], "radius": 10}),
    );
    let made = run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "S", "distance": 5, "name": "P"}),
    );
    assert_eq!(made["status"], "ok", "{made}");
    s
}

/// The processor time this process has used, on all its threads, in
/// milliseconds (Linux: `/proc/self/stat`, in ticks of 10 ms); 0 elsewhere.
/// What a busy machine changes least.
fn cpu_ms() -> f64 {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    // After the command's name, which may hold spaces: fields 14 and 15
    // of the line are the 12th and 13th after the closing bracket.
    let after = stat.rsplit(')').next().unwrap_or_default();
    let ticks: f64 = after
        .split_whitespace()
        .skip(11)
        .take(2)
        .filter_map(|n| n.parse::<f64>().ok())
        .sum();
    ticks * 10.0
}

fn time(label: &str, s: &mut Session, quality: SceneQuality) {
    // A fresh build: the cache is keyed by what is asked for, so ask for
    // something not asked before.
    let (start, cpu) = (Instant::now(), cpu_ms());
    let scene = s.scene_with(quality).unwrap().unwrap();
    let ms = start.elapsed().as_secs_f64() * 1e3;
    let cpu = cpu_ms() - cpu;
    let faces = scene.mesh.face_ids.iter().max().map_or(0, |m| m + 1);
    println!(
        "  {label:<28} tolerance {:>10.5}  {:>9} triangles  {faces:>5} faces  {ms:>8.1} ms  cpu {cpu:>6.0} ms  ({:?})",
        scene.tolerance,
        scene.triangles(),
        scene.problems
    );
}

#[test]
#[ignore = "a timing, not a check"]
fn time_of_the_scene_at_the_display_quality_and_the_automatic_tolerance() {
    let built = Instant::now();
    let mut models: Vec<(&str, Session)> = vec![("flange", flange())];
    println!(
        "flange built in {:.0} ms",
        built.elapsed().as_secs_f64() * 1e3
    );
    let built = Instant::now();
    models.push(("polygon of 1000 sides", polygon(1000)));
    println!(
        "polygon built in {:.0} ms",
        built.elapsed().as_secs_f64() * 1e3
    );
    let built = Instant::now();
    models.push(("plate with 32 x 32 holes", drilled(32)));
    println!(
        "drilled plate built in {:.0} ms",
        built.elapsed().as_secs_f64() * 1e3
    );
    for (name, s) in &mut models {
        println!("{name}:");
        time("automatic", s, SceneQuality::Automatic);
        for relative in [2e-4, 1e-4, 5e-5, 3e-5, 2e-5, 1e-5] {
            time(
                &format!("display {relative:e}, no budget"),
                s,
                SceneQuality::Display {
                    relative,
                    max_triangles: usize::MAX,
                    max_angle: None,
                },
            );
        }
        time("display default", s, SceneQuality::display());
        time(
            "display, budget 200k",
            s,
            SceneQuality::Display {
                relative: craft3d_engine::DISPLAY_RELATIVE_TOLERANCE,
                max_triangles: 200_000,
                max_angle: Some(craft3d_engine::DISPLAY_MAX_ANGLE),
            },
        );
    }
}

/// What the triangle budget costs when a scene is over it: the display
/// ladder on models whose finest mesh is several times the budget. Since
/// 2026-10-13 the body that goes over the budget stops inside the
/// tessellator (`TessParams::max_triangles`); before, it was tessellated
/// in full at every step of the ladder.
#[test]
#[ignore = "a timing, not a check"]
fn time_of_the_display_ladder_under_a_triangle_budget() {
    let mut models: Vec<(&str, Session)> = vec![
        ("flange", flange()),
        ("plate with 32 x 32 holes", drilled(32)),
    ];
    for (name, s) in &mut models {
        println!("{name}:");
        let relative = 1e-6;
        time(
            "display 1e-6, no budget",
            s,
            SceneQuality::Display {
                relative,
                max_triangles: usize::MAX,
                max_angle: None,
            },
        );
        for max_triangles in [400_000, 100_000, 30_000, 10_000] {
            time(
                &format!("display 1e-6, budget {max_triangles}"),
                s,
                SceneQuality::Display {
                    relative,
                    max_triangles,
                    max_angle: None,
                },
            );
        }
    }
}
