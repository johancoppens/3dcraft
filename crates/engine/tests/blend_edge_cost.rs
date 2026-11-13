//! What a blend at the end of a long chain costs when the cache lets the
//! bodies go (rule 12): a plate with `HOLES` holes cut one after another,
//! a body budget of nothing, and a fillet of the plate's top front edge
//! added and then edited, given as the pair of its faces, as the name the
//! block gave it (followed through every cut: the cut bodies' edge names
//! are derived again first), and as the name the last cut gives it
//! (nothing followed). Ignored: run in release, one scenario per process,
//! for the time each command takes and the process's peak memory
//! (`cargo test --release -p craft3d-engine --test blend_edge_cost --
//! --ignored --nocapture <scenario>`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};
use std::time::Instant;

/// How many holes the plate gets.
const HOLES: usize = 150;

/// The process's peak resident memory in MB (Linux), or nothing.
fn peak_mb() -> Option<f64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("VmHWM:"))?;
    let kb: f64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024.0)
}

/// The plate (F1) and its holes cut one after another, the last cut
/// `Cut{HOLES - 1}`, with nothing kept that a later feature took.
fn plate_with_holes(s: &mut Session) -> Value {
    s.set_body_budget(0);
    run(s, "file.new", json!({"units": "mm"}));
    let length = 10.0 * HOLES as f64 + 20.0;
    run(
        s,
        "feature.block",
        json!({"size": [length, 40, 10], "position": [0, 0, 0], "name": "Plate"}),
    );
    let listed = run(s, "document.edges", json!({"feature": "Plate"}));
    let mut target = "Plate".to_string();
    for i in 0..HOLES {
        run(
            s,
            "feature.cylinder",
            json!({"radius": 2, "height": 20, "position": [10.0 + 10.0 * i as f64, 20, -5],
                   "axis": [0, 0, 1], "name": format!("Hole{i}")}),
        );
        let cut = format!("Cut{i}");
        run(
            s,
            "feature.boolean",
            json!({"op": "subtract", "target": target, "tool": format!("Hole{i}"), "name": cut}),
        );
        target = cut;
    }
    listed
}

/// The edge between the faces named `a` and `b` of `listed`: its name.
fn edge(listed: &Value, a: &str, b: &str) -> String {
    listed["edges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["faces"] == json!([a, b]) || e["faces"] == json!([b, a]))
        .unwrap_or_else(|| panic!("no edge {a} / {b}: {listed}"))["name"]
        .as_str()
        .unwrap()
        .to_string()
}

fn measure(scenario: &str, edges: Value, s: &mut Session) {
    let start = Instant::now();
    let made = run(
        s,
        "feature.fillet",
        json!({"edges": edges, "radius": 1, "name": "Round"}),
    );
    let add = start.elapsed().as_secs_f64() * 1e3;
    assert_eq!(made["status"], "ok", "{made}");
    let start = Instant::now();
    run(s, "feature.edit", json!({"feature": "Round", "radius": 2}));
    let edit = start.elapsed().as_secs_f64() * 1e3;
    let measured = run(s, "document.measure", json!({}));
    println!(
        "COST {scenario}: holes {HOLES}, add {add:.0} ms, edit {edit:.0} ms, peak {} MB, rederived {:?}, volume {}",
        peak_mb().map_or("n/a".to_string(), |mb| format!("{mb:.0}")),
        s.last_regeneration_rederived(),
        measured["volume"]
    );
}

#[test]
#[ignore = "a measurement: run in release, one scenario per process"]
fn cost_by_pair() {
    let mut s = Session::new(Workspace::none());
    plate_with_holes(&mut s);
    measure(
        "by the pair of the plate's faces",
        json!([["F1:face(1)", "F1:face(2)"]]),
        &mut s,
    );
}

#[test]
#[ignore = "a measurement: run in release, one scenario per process"]
fn cost_by_the_plates_name() {
    let mut s = Session::new(Workspace::none());
    let listed = plate_with_holes(&mut s);
    let name = edge(&listed, "F1:face(1)", "F1:face(2)");
    measure("by the name the plate gave the edge", json!([name]), &mut s);
}

#[test]
#[ignore = "a measurement: run in release, one scenario per process"]
fn cost_by_the_last_cuts_name() {
    let mut s = Session::new(Workspace::none());
    plate_with_holes(&mut s);
    let last = format!("Cut{}", HOLES - 1);
    let listed = run(&mut s, "document.edges", json!({"feature": last}));
    let edges = listed["edges"].as_array().unwrap();
    let name = edges
        .iter()
        .find(|e| {
            let faces = e["faces"].to_string();
            faces.contains("F1:face(1)\"") && faces.contains("F1:face(2)\"")
        })
        .unwrap_or_else(|| panic!("{listed}"))["name"]
        .as_str()
        .unwrap()
        .to_string();
    measure(
        "by the name the last cut gives the edge",
        json!([name]),
        &mut s,
    );
}
