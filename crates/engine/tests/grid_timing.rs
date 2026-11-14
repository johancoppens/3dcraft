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
