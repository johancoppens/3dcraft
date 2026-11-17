//! A grid surface wider than a table held before 2026-11-17 (64 columns):
//! the Mont Ventoux relief every 87.5 m, 145 heights a row and 113 rows
//! (`grid_timing/ventoux-87.5m.csv`), as a table of heights in metres
//! (docs/design/surfacing.md › *The table form*), made a sheet at 1:70 000
//! with its heights times 2.1 and cut from a block of its footprint; and
//! the same points given one by one. The two sheets are one: the same
//! solid under them, the same heights at the nodes.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::{run, session_in, workdir};
use serde_json::{Value, json};

/// The relief's heights, `rows[r][c]` in metres: 113 rows south to north,
/// 145 heights a row west to east.
fn relief() -> Vec<Vec<f64>> {
    include_str!("grid_timing/ventoux-87.5m.csv")
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| l.split(',').map(|v| v.trim().parse().unwrap()).collect())
        .collect()
}

fn volume_of(inspected: &Value, name: &str) -> f64 {
    inspected["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == name)
        .unwrap()["volume"]
        .as_f64()
        .unwrap()
}

#[test]
fn a_relief_of_145_heights_a_row_is_a_table_a_sheet_and_a_solid() {
    let heights = relief();
    assert_eq!((heights.len(), heights[0].len()), (113, 145));
    let dir = workdir("surface-grid-table", "relief");
    let mut s = session_in(&dir);
    run(
        &mut s,
        "file.new",
        json!({"units": "mm", "tolerance": 0.001}),
    );
    run(
        &mut s,
        "variable.set",
        json!({"name": "vertical_scale", "expr": "2.1 / 70000"}),
    );
    let mut columns = vec![json!("row")];
    columns.extend((1..=145).map(|c| json!({"name": format!("c{c:03}"), "unit": "m"})));
    let rows: Vec<Value> = heights
        .iter()
        .enumerate()
        .map(|(r, row)| {
            let mut cells = vec![json!(r + 1)];
            cells.extend(row.iter().map(|h| json!(h)));
            Value::Array(cells)
        })
        .collect();
    run(
        &mut s,
        "table.set",
        json!({"name": "elevation", "columns": columns, "rows": rows}),
    );
    let made = run(
        &mut s,
        "feature.surface.grid",
        json!({"table": "elevation", "step": ["1.25 mm", "1.25 mm"], "factor": "vertical_scale", "name": "Terrain surface"}),
    );
    assert_eq!(made["status"], "ok", "{made}");
    assert_eq!(made["grid"]["columns"], 145, "{made}");
    assert_eq!(made["grid"]["rows"], 113, "{made}");
    // The same points one by one (heights in millimetres at the scale).
    let scale = 2.1 / 70000.0 * 1000.0;
    let points: Vec<Value> = heights
        .iter()
        .enumerate()
        .map(|(r, row)| {
            Value::Array(
                row.iter()
                    .enumerate()
                    .map(|(c, h)| json!([c as f64 * 1.25, r as f64 * 1.25, h * scale]))
                    .collect(),
            )
        })
        .collect();
    let given = run(
        &mut s,
        "feature.surface.grid",
        json!({"points": points, "spacing": "even", "name": "Given"}),
    );
    assert_eq!(given["status"], "ok", "{given}");
    for (blank, sheet, under) in [
        ("Terrain blank", "Terrain surface", "Terrain"),
        ("Given blank", "Given", "Under given"),
    ] {
        let block = run(
            &mut s,
            "feature.block",
            json!({"size": [180, 140, 100], "position": [0, 0, 0], "name": blank}),
        );
        assert_eq!(block["status"], "ok", "{block}");
        let cut = run(
            &mut s,
            "feature.boolean",
            json!({"op": "intersect", "target": blank, "tool": sheet, "name": under}),
        );
        assert_eq!(cut["status"], "ok", "{cut}");
    }
    // One solid: 776 548 mm³ under the relief (docs/design/surfacing.md ›
    // Findings, the same model from SRTM's tile), within rounding of the
    // heights' two routes to millimetres.
    let inspected = run(&mut s, "document.inspect", json!({}));
    let (table, points) = (
        volume_of(&inspected, "Terrain"),
        volume_of(&inspected, "Under given"),
    );
    assert!(
        (table - points).abs() <= 1e-9 * points,
        "{table} against {points}"
    );
    assert!((table - 776_548.4).abs() < 1.0, "{table}");
    // At the nodes the solid's top is the table's heights.
    for (r, c) in [(0, 0), (56, 72), (84, 72), (112, 144), (7, 131)] {
        let (x, y) = (
            (c as f64 * 1.25).clamp(1e-6, 180.0 - 1e-6),
            (r as f64 * 1.25).clamp(1e-6, 140.0 - 1e-6),
        );
        let hit = run(
            &mut s,
            "measure.ray",
            json!({"from": [x, y, 95.0], "direction": "-z", "of": "Terrain", "limit": 1}),
        );
        let z = hit["hits"][0]["point"][2].as_f64().unwrap();
        assert!(
            (z - heights[r][c] * scale).abs() < 1e-5,
            "at row {r}, column {c}: {z} against {}",
            heights[r][c] * scale
        );
    }
}
