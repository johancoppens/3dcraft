//! How long a large sheet through a grid of points takes
//! (docs/design/surfacing.md › *A sheet through a grid of points*): a
//! synthetic terrain of 150 × 120 and 400 × 300 points given point by
//! point, through the commands: the feature, meshing (an STL export),
//! measuring, saving, and the process's peak memory after each; and the
//! interpolation alone. Ignored by default; run in release with
//! `--ignored --nocapture --test-threads 1` to print the times.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::{run, session_in, workdir};
use craft3d_geom::grid::{Spacing, interpolate};
use craft3d_math::{Portable, Vec3};
use serde_json::{Value, json};
use std::time::Instant;

/// Hills and ridges over `[0, 1]²`, in millimetres for a model 100 mm wide.
fn height(x: f64, y: f64) -> f64 {
    let mut h = 0.0;
    for (cx, cy, r, a) in [
        (0.3, 0.4, 0.15, 18.0),
        (0.7, 0.6, 0.1, 25.0),
        (0.5, 0.2, 0.2, 10.0),
        (0.8, 0.85, 0.07, 12.0),
    ] {
        h += a * (-((x - cx) * (x - cx) + (y - cy) * (y - cy)) / (2.0 * r * r)).pexp();
    }
    h + 2.0 * (9.0 * x + 3.0 * y).psin() * (7.0 * y).pcos()
}

/// The process's peak resident memory so far, in MiB (Linux).
fn peak_mib() -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|kib| kib.parse::<f64>().ok())
        })
        .map_or(f64::NAN, |kib| kib / 1024.0)
}

#[test]
#[ignore]
fn a_large_grid_is_built_meshed_measured_and_saved() {
    for (columns, rows) in [(150_usize, 120_usize), (400, 300)] {
        let step = 100.0 / (columns - 1) as f64;
        let at = |r: usize, c: usize| {
            let (x, y) = (
                c as f64 / (columns - 1) as f64,
                r as f64 / (columns - 1) as f64,
            );
            [c as f64 * step, r as f64 * step, height(x, y)]
        };
        // The interpolation alone, and with the proof that it is regular.
        let pts: Vec<Vec3> = (0..rows)
            .flat_map(|r| (0..columns).map(move |c| (r, c)))
            .map(|(r, c)| {
                let [x, y, z] = at(r, c);
                Vec3::new(x, y, z)
            })
            .collect();
        let t = Instant::now();
        let made = interpolate(&pts, rows, columns, Spacing::Greville, 1e-3, 1e-9).unwrap();
        let alone = t.elapsed();
        assert!(made.deviation <= made.bound);

        let dir = workdir("grid-timing", &format!("{columns}x{rows}"));
        let mut s = session_in(&dir);
        run(
            &mut s,
            "file.new",
            json!({"units": "mm", "tolerance": 0.001}),
        );
        let points: Vec<Value> = (0..rows)
            .map(|r| Value::Array((0..columns).map(|c| json!(at(r, c))).collect()))
            .collect();
        let base = peak_mib();
        let t = Instant::now();
        let made = run(
            &mut s,
            "feature.surface.grid",
            json!({"points": points, "name": "Terrain"}),
        );
        let build = t.elapsed();
        assert_eq!(made["status"], "ok", "{made}");
        let after_build = peak_mib();
        // A command that changes nothing the grid reads: what it costs a
        // document with points given outright (each of their expressions
        // evaluated again for the regeneration's cache).
        let t = Instant::now();
        run(
            &mut s,
            "variable.set",
            json!({"name": "unrelated", "expr": "1"}),
        );
        let unrelated = t.elapsed();
        let t = Instant::now();
        let mesh = run(&mut s, "file.export", json!({"path": "terrain.stl"}));
        let meshed = t.elapsed();
        let after_mesh = peak_mib();
        let t = Instant::now();
        let measured = run(&mut s, "document.measure", json!({}));
        let measure = t.elapsed();
        let t = Instant::now();
        run(&mut s, "file.save", json!({"path": "terrain.3dcraft"}));
        let save = t.elapsed();
        let bytes = std::fs::metadata(dir.join("terrain.3dcraft"))
            .unwrap()
            .len();
        println!(
            "{columns} x {rows}: interpolation and proof {alone:?}; the feature {build:?}; an unrelated variable.set {unrelated:?}; mesh ({} triangles) {meshed:?}; measure (area {}) {measure:?}; save {save:?}, {bytes} bytes, {:.1} a point; peak {base:.0} MiB before, {after_build:.0} after the feature, {after_mesh:.0} after the mesh, {:.0} at the end",
            mesh["triangles"],
            measured["area"],
            bytes as f64 / (columns * rows) as f64,
            peak_mib()
        );
    }
}

/// The heights of the Mont Ventoux relief every 87.5 m, `rows[r][c]` in
/// metres (`grid_timing/ventoux-87.5m.csv`: 113 rows south to north, 145
/// heights a row west to east).
fn relief() -> Vec<Vec<f64>> {
    let text = include_str!("grid_timing/ventoux-87.5m.csv");
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| l.split(',').map(|v| v.trim().parse().unwrap()).collect())
        .collect()
}

/// A point of a grid of `rows` × `columns`: `at(r, c)`.
type At = Box<dyn Fn(usize, usize) -> [f64; 3]>;

/// The synthetic terrain of [`height`] at `columns` points a row, 100 mm
/// wide.
fn synthetic(columns: usize) -> At {
    let step = 100.0 / (columns - 1) as f64;
    Box::new(move |r, c| {
        let (x, y) = (
            c as f64 / (columns - 1) as f64,
            r as f64 / (columns - 1) as f64,
        );
        [c as f64 * step, r as f64 * step, height(x, y)]
    })
}

/// Where the time of a large sheet goes, stage by stage: the 87.5 m relief
/// (145 × 113, given point by point, as at 1:70 000 with heights times
/// 2.1) and the synthetic terrains of 150 × 120 and 400 × 300, each
/// built, cut by a block of its footprint, meshed (an STL of the cut) and
/// measured (`document.inspect`). Prints the stages the clocks of
/// `craft3d_math::clock` summed in each command, the volume and the
/// height of vertical rays at some nodes (to compare runs).
#[test]
#[ignore]
fn stages_of_a_large_grid() {
    let heights = std::sync::Arc::new(relief());
    let ventoux = |heights: std::sync::Arc<Vec<Vec<f64>>>| -> At {
        Box::new(move |r, c| [c as f64 * 1.25, r as f64 * 1.25, heights[r][c] * 0.03])
    };
    let cases: Vec<(&str, &str, usize, usize, At)> = vec![
        (
            "relief 145 x 113 even",
            "even",
            145,
            113,
            ventoux(heights.clone()),
        ),
        (
            "relief 145 x 113 greville",
            "greville",
            145,
            113,
            ventoux(heights),
        ),
        ("synthetic 150 x 120", "even", 150, 120, synthetic(150)),
        ("synthetic 400 x 300", "even", 400, 300, synthetic(400)),
    ];
    for (label, spacing, columns, rows, at) in cases {
        let dir = workdir("grid-stages", &format!("{columns}x{rows}-{spacing}"));
        let mut s = session_in(&dir);
        run(
            &mut s,
            "file.new",
            json!({"units": "mm", "tolerance": 0.001}),
        );
        let points: Vec<Value> = (0..rows)
            .map(|r| Value::Array((0..columns).map(|c| json!(at(r, c))).collect()))
            .collect();
        let [x1, y1] = {
            let p = at(rows - 1, columns - 1);
            [p[0], p[1]]
        };
        let timed = |s: &mut craft3d_engine::Session, id: &str, params: Value| {
            craft3d_math::clock::enable(true);
            let t = Instant::now();
            let result = s.execute(id, &params).map_err(|e| e.to_string());
            let took = t.elapsed().as_secs_f64();
            let stages = craft3d_math::clock::take();
            craft3d_math::clock::enable(false);
            println!("  {label}: {id} {took:.3} s");
            for (stage, seconds, runs) in stages {
                println!("    {stage:<40} {seconds:>9.3} s {runs:>7} runs");
            }
            result
        };
        println!("{label}:");
        let made = timed(
            &mut s,
            "feature.surface.grid",
            json!({"points": points, "spacing": spacing, "name": "Sheet"}),
        );
        assert_eq!(made.as_ref().unwrap()["status"], "ok", "{made:?}");
        let block = timed(
            &mut s,
            "feature.block",
            json!({"size": [x1, y1, 100.0], "position": [0, 0, -10.0], "name": "Blank"}),
        );
        assert_eq!(block.unwrap()["status"], "ok");
        let cut = timed(
            &mut s,
            "feature.boolean",
            json!({"op": "intersect", "target": "Blank", "tool": "Sheet", "keepTool": true, "name": "Under"}),
        )
        .unwrap();
        println!("  {label}: cut status {} {}", cut["status"], cut["error"]);
        if cut["status"] != "ok" {
            let _ = timed(
                &mut s,
                "file.export",
                json!({"path": "sheet.stl", "bodies": ["Sheet"]}),
            );
            if let Ok(inspect) = timed(&mut s, "document.inspect", json!({})) {
                for body in inspect["bodies"].as_array().unwrap() {
                    println!("  {label}: body {} area {}", body["name"], body["area"]);
                }
            }
            continue;
        }
        let mesh = timed(
            &mut s,
            "file.export",
            json!({"path": "under.stl", "bodies": ["Under"]}),
        );
        println!("  {label}: triangles {}", mesh.unwrap()["triangles"]);
        let inspect = timed(&mut s, "document.inspect", json!({})).unwrap();
        for body in inspect["bodies"].as_array().unwrap() {
            println!(
                "  {label}: body {} volume {} area {}",
                body["name"], body["volume"], body["area"]
            );
        }
        // Vertical rays at every 8th node each way.
        let mut zs = Vec::new();
        for r in (0..rows).step_by(8) {
            for c in (0..columns).step_by(8) {
                let p = at(r, c);
                let x = p[0].clamp(1e-6, x1 - 1e-6);
                let y = p[1].clamp(1e-6, y1 - 1e-6);
                let hit = run(
                    &mut s,
                    "measure.ray",
                    json!({"from": [x, y, 95.0], "direction": "-z", "of": "Under", "limit": 1}),
                );
                let z = hit["hits"][0]["point"][2].as_f64().unwrap_or(f64::NAN);
                zs.push(format!("{z:.17e}"));
            }
        }
        println!("  {label}: rays {}", zs.join(" "));
    }
}
