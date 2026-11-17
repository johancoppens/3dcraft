//! How long a large sheet through a grid of points takes
//! (docs/design/surfacing.md › *A sheet through a grid of points*): a
//! synthetic terrain of 150 × 120 and 400 × 300 points given point by
//! point, through the commands: the feature, meshing (an STL export),
//! measuring, saving, and the process's peak memory after each; and the
//! interpolation alone. And the relief of Mont Ventoux every 87.5 m
//! (`grid_timing/ventoux-87.5m.csv`) and the same terrains cut from a
//! block (*Scale*). Ignored by default; run in release with
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

/// How long a large sheet and the solid under it take through the
/// commands (docs/design/surfacing.md › *Scale*): the 87.5 m relief
/// (145 × 113, as at 1:70 000 with heights times 2.1) given point by point
/// at even and at Greville spacing and as a table of its heights in
/// metres, and the synthetic terrains of 150 × 120 and 400 × 300; each
/// built, cut by a block of its footprint, meshed (an STL of the cut),
/// measured (`document.inspect`) and saved. Prints each command's time,
/// the file's size, the volume and the height of vertical rays at some
/// nodes (to compare runs bit for bit).
#[test]
#[ignore]
fn a_large_sheet_is_cut_meshed_and_measured() {
    let heights = std::sync::Arc::new(relief());
    let ventoux = |heights: std::sync::Arc<Vec<Vec<f64>>>| -> At {
        Box::new(move |r, c| [c as f64 * 1.25, r as f64 * 1.25, heights[r][c] * 0.03])
    };
    // (label, form: points or table, spacing, columns, rows, points)
    let cases: Vec<(&str, &str, &str, usize, usize, At)> = vec![
        (
            "relief 145 x 113 even",
            "points",
            "even",
            145,
            113,
            ventoux(heights.clone()),
        ),
        (
            "relief 145 x 113 even, table",
            "table",
            "even",
            145,
            113,
            ventoux(heights.clone()),
        ),
        (
            "relief 145 x 113 greville",
            "points",
            "greville",
            145,
            113,
            ventoux(heights.clone()),
        ),
        (
            "synthetic 150 x 120",
            "points",
            "even",
            150,
            120,
            synthetic(150),
        ),
        (
            "synthetic 400 x 300",
            "points",
            "even",
            400,
            300,
            synthetic(400),
        ),
    ];
    for (label, form, spacing, columns, rows, at) in cases {
        let dir = workdir("grid-stages", &format!("{columns}x{rows}-{spacing}-{form}"));
        let mut s = session_in(&dir);
        run(
            &mut s,
            "file.new",
            json!({"units": "mm", "tolerance": 0.001}),
        );
        let [x1, y1] = {
            let p = at(rows - 1, columns - 1);
            [p[0], p[1]]
        };
        let timed = |s: &mut craft3d_engine::Session, id: &str, params: Value| {
            let t = Instant::now();
            let result = s.execute(id, &params).map_err(|e| e.to_string());
            println!("  {label}: {id} {:.3} s", t.elapsed().as_secs_f64());
            result
        };
        println!("{label}:");
        let made = if form == "table" {
            run(
                &mut s,
                "variable.set",
                json!({"name": "vertical_scale", "expr": "2.1 / 70000"}),
            );
            let mut names = vec![json!("row")];
            names.extend((1..=columns).map(|c| json!({"name": format!("c{c:03}"), "unit": "m"})));
            let cells: Vec<Value> = heights
                .iter()
                .enumerate()
                .map(|(r, row)| {
                    let mut cells = vec![json!(r + 1)];
                    cells.extend(row.iter().map(|h| json!(h)));
                    Value::Array(cells)
                })
                .collect();
            timed(
                &mut s,
                "table.set",
                json!({"name": "elevation", "columns": names, "rows": cells}),
            )
            .unwrap();
            timed(
                &mut s,
                "feature.surface.grid",
                json!({"table": "elevation", "step": ["1.25 mm", "1.25 mm"], "factor": "vertical_scale", "spacing": spacing, "name": "Sheet"}),
            )
        } else {
            let points: Vec<Value> = (0..rows)
                .map(|r| Value::Array((0..columns).map(|c| json!(at(r, c))).collect()))
                .collect();
            timed(
                &mut s,
                "feature.surface.grid",
                json!({"points": points, "spacing": spacing, "name": "Sheet"}),
            )
        };
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
        let measured = if cut["status"] == "ok" {
            "Under"
        } else {
            "Sheet"
        };
        let mesh = timed(
            &mut s,
            "file.export",
            json!({"path": "mesh.stl", "bodies": [measured]}),
        );
        println!("  {label}: triangles {}", mesh.unwrap()["triangles"]);
        let inspect = timed(&mut s, "document.inspect", json!({})).unwrap();
        for body in inspect["bodies"].as_array().unwrap() {
            println!(
                "  {label}: body {} volume {} area {}",
                body["name"], body["volume"], body["area"]
            );
        }
        timed(&mut s, "file.save", json!({"path": "sheet.3dcraft"})).unwrap();
        let bytes = std::fs::metadata(dir.join("sheet.3dcraft")).unwrap().len();
        println!(
            "  {label}: file {bytes} bytes, {:.1} a point",
            bytes as f64 / (columns * rows) as f64
        );
        if cut["status"] != "ok" {
            continue;
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

/// What a lookup costs in a table of 64 columns and of 1025, the cap
/// since 2026-11-17 (`craft3d_expr::MAX_COLUMNS`): a row by its number,
/// the first column and the last, 100 000 times each. The cap allocates
/// nothing; a table holds and a lookup compares the columns it has.
#[test]
#[ignore]
fn a_lookup_in_a_wide_table() {
    use craft3d_expr::{Cell, Column, Quantity, RowRef, Table};
    for width in [64_usize, 1025] {
        let table = Table {
            name: "t".into(),
            columns: (0..width)
                .map(|c| Column {
                    name: format!("c{c:04}"),
                    unit: None,
                })
                .collect(),
            key: 0,
            rows: std::sync::Arc::new(
                (0..113)
                    .map(|r| (0..width).map(|c| Cell::Number((r * c) as f64)).collect())
                    .collect(),
            ),
        };
        let row = RowRef::Number(Quantity::dimensionless(57.0));
        for column in [format!("c{:04}", 1), format!("c{:04}", width - 1)] {
            let t = Instant::now();
            let mut sum = 0.0;
            for _ in 0..100_000 {
                sum += table.lookup(&row, &column).unwrap().value;
            }
            println!(
                "{width} columns, {column}: {:.3} µs a lookup ({sum})",
                t.elapsed().as_secs_f64() * 10.0
            );
        }
    }
}

/// MEASUREMENT (boolean-spans, 2026-11-17): the synthetic terrain at
/// `columns` × `rows` points cut by a block of its footprint and by a
/// block across its middle (a quarter in from every side), stage by stage
/// (`craft3d_math::clock`, temporary), with the process's peak memory
/// reset between the two (`/proc/self/clear_refs`). A watchdog ends the
/// process after `deadline` seconds a case, printing what the clocks
/// summed so far.
fn cut_stages(columns: usize, rows: usize, deadline: u64) {
    let at = synthetic(columns);
    let [x1, y1] = {
        let p = at(rows - 1, columns - 1);
        [p[0], p[1]]
    };
    let points: Vec<Value> = (0..rows)
        .map(|r| Value::Array((0..columns).map(|c| json!(at(r, c))).collect()))
        .collect();
    for (label, size, position) in [
        ("footprint", [x1, y1, 100.0], [0.0, 0.0, -10.0]),
        (
            "middle",
            [0.5 * x1, 0.5 * y1, 100.0],
            [0.25 * x1, 0.25 * y1, -10.0],
        ),
    ] {
        let _ = std::fs::write("/proc/self/clear_refs", "5");
        let case = format!("{columns} x {rows} {label}");
        let dir = workdir("grid-cut-stages", &format!("{columns}x{rows}-{label}"));
        let mut s = session_in(&dir);
        run(
            &mut s,
            "file.new",
            json!({"units": "mm", "tolerance": 0.001}),
        );
        let t = Instant::now();
        let made = run(
            &mut s,
            "feature.surface.grid",
            json!({"points": points, "name": "Sheet"}),
        );
        println!(
            "{case}: sheet {:.3} s, {} warnings {}",
            t.elapsed().as_secs_f64(),
            made["status"],
            made["warnings"]
        );
        run(
            &mut s,
            "feature.block",
            json!({"size": size, "position": position, "name": "Blank"}),
        );
        let base = peak_mib();
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let watch = {
            let done = done.clone();
            let case = case.clone();
            std::thread::spawn(move || {
                let start = Instant::now();
                while start.elapsed().as_secs() < deadline {
                    if done.load(std::sync::atomic::Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
                println!("{case}: DEADLINE {deadline} s passed; clocks so far:");
                for (stage, seconds, runs) in craft3d_math::clock::peek() {
                    println!("  {case}: {stage:<34} {seconds:9.3} s {runs:>12}");
                }
                println!("{case}: peak {:.0} MiB", peak_mib());
                std::process::exit(0);
            })
        };
        craft3d_math::clock::enable(true);
        let t = Instant::now();
        let cut = s
            .execute(
                "feature.boolean",
                &json!({"op": "intersect", "target": "Blank", "tool": "Sheet", "keepTool": true, "name": "Under"}),
            )
            .map_err(|e| e.to_string());
        let took = t.elapsed().as_secs_f64();
        craft3d_math::clock::enable(false);
        let stages = craft3d_math::clock::take();
        done.store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = watch.join();
        let status = cut.as_ref().map_or_else(
            |e| format!("error {e}"),
            |v| format!("{} {}", v["status"], v["error"]),
        );
        println!(
            "{case}: cut {took:.3} s: {status}; peak {base:.0} MiB before the cut, {:.0} after",
            peak_mib()
        );
        for (stage, seconds, runs) in &stages {
            println!("  {case}: {stage:<34} {seconds:9.3} s {runs:>12}");
        }
        if !cut.as_ref().is_ok_and(|v| v["status"] == "ok") {
            continue;
        }
        let t = Instant::now();
        let inspect = s.execute("document.inspect", &json!({}));
        let took = t.elapsed().as_secs_f64();
        if let Ok(inspect) = inspect {
            for body in inspect["bodies"].as_array().into_iter().flatten() {
                println!(
                    "{case}: inspect {took:.3} s: body {} volume {} area {}",
                    body["name"], body["volume"], body["area"]
                );
            }
        }
        let t = Instant::now();
        let mesh = s.execute(
            "file.export",
            &json!({"path": "mesh.stl", "bodies": ["Under"]}),
        );
        println!(
            "{case}: STL {:.3} s: {}; peak {:.0} MiB",
            t.elapsed().as_secs_f64(),
            mesh.map_or_else(|e| e.to_string(), |m| m["triangles"].to_string()),
            peak_mib()
        );
        // Vertical rays at every 32nd node each way within the block.
        let mut zs = Vec::new();
        for r in (0..rows).step_by(32) {
            for c in (0..columns).step_by(32) {
                let p = at(r, c);
                let (x, y) = (p[0], p[1]);
                if !(x > position[0] && x < position[0] + size[0])
                    || !(y > position[1] && y < position[1] + size[1])
                {
                    continue;
                }
                let hit = s.execute(
                    "measure.ray",
                    &json!({"from": [x, y, 95.0], "direction": "-z", "of": "Under", "limit": 1}),
                );
                let z = hit
                    .ok()
                    .and_then(|h| h["hits"][0]["point"][2].as_f64())
                    .unwrap_or(f64::NAN);
                zs.push(format!("{:.17e}", z - p[2]));
            }
        }
        println!("{case}: ray minus node height {}", zs.join(" "));
    }
}

#[test]
#[ignore]
fn spans_0100() {
    cut_stages(100, 100, 300);
}

#[test]
#[ignore]
fn spans_0184() {
    cut_stages(184, 184, 300);
}

#[test]
#[ignore]
fn spans_0256() {
    cut_stages(256, 256, 300);
}

#[test]
#[ignore]
fn spans_0400x0300() {
    cut_stages(400, 300, 300);
}

#[test]
#[ignore]
fn spans_0512() {
    cut_stages(512, 512, 300);
}
