//! The adaptive proofs of a clearance and a thickness
//! (docs/design/verification.md › *The proven thickness*): three models on
//! which the true bound held but the proof fell short (the planetary
//! gearbox's planet in its ring, a case shelled to its fillet radius, a
//! clamp with chamfered bore mouths), and closed forms at three scales.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};
use std::time::Instant;

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn new() -> Session {
    let mut s = Session::new(Workspace::unrestricted());
    run(&mut s, "file.new", json!({"units": "mm"}));
    s
}

/// Runs `id` and returns (found, proven, seconds).
fn timed(s: &mut Session, id: &str, params: Value, key: &str, what: &str) -> (f64, f64) {
    let started = Instant::now();
    let out = run(s, id, params);
    let found = out[key].as_f64().unwrap_or_else(|| panic!("{what}: {out}"));
    let proven = out["atLeast"]
        .as_f64()
        .unwrap_or_else(|| panic!("{what}: {out}"));
    eprintln!(
        "{what}: found {found}, proven {proven}, in {:.3} s",
        started.elapsed().as_secs_f64()
    );
    assert!(proven <= found, "{what}: proven {proven} above {found}");
    (found, proven)
}

fn status(s: &mut Session, assertion: Value) -> String {
    let out = run(s, "assert.add", assertion);
    out["status"].as_str().unwrap_or_default().to_string()
}

/// The planetary gearbox as published: planet 1 in the ring, with the
/// backlash its assertion asks for (0.3 to 0.7 of jn).
#[test]
fn the_planet_in_its_ring_is_proven_to_its_backlash() {
    let mut s = Session::new(Workspace::unrestricted());
    let path = format!(
        "{}/tests/proof_fixtures/planetary-gearbox.3dcraftasm",
        env!("CARGO_MANIFEST_DIR")
    );
    run(&mut s, "file.open", json!({"path": path}));
    let started = Instant::now();
    let out = run(
        &mut s,
        "assert.add",
        json!({"name": "x", "measure": "clearance", "from": "Planet1", "to": "Ring", "min": "0.3 * jn", "max": "0.7 * jn"}),
    );
    eprintln!(
        "planet in ring: {out} in {:.3} s",
        started.elapsed().as_secs_f64()
    );
    let got = out["status"].as_str().unwrap_or_default();
    assert_eq!(got, "passed");
}

/// docs/tasks/shelled-case.json's solution, at a wall of `wall` mm.
fn shelled_case(wall: f64) -> Session {
    let task: Value = serde_json::from_str(
        &std::fs::read_to_string(format!(
            "{}/../../docs/tasks/shelled-case.json",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap(),
    )
    .unwrap();
    let mut s = new();
    for step in task["solution"].as_array().unwrap() {
        let (id, params) = (step[0].as_str().unwrap(), step[1].clone());
        if id == "assert.add" {
            continue;
        }
        run(&mut s, id, params);
    }
    run(
        &mut s,
        "variable.set",
        json!({"name": "wall", "expr": format!("{wall} mm")}),
    );
    s
}

#[test]
fn the_shelled_case_is_proven_to_its_wall() {
    let mut s = shelled_case(2.0);
    let (found, proven) = timed(
        &mut s,
        "measure.thickness",
        json!({"of": "Case"}),
        "thickness",
        "shelled case, wall 2",
    );
    assert!((found - 2.0).abs() < 1e-6, "{found}");
    assert!(proven >= 1.5, "proven only {proven}");
    let got = status(
        &mut s,
        json!({"name": "thin", "measure": "thickness", "of": "Case", "min": "1.5 mm"}),
    );
    assert_eq!(got, "passed");
    // Truly thinner: fails.
    let mut s = shelled_case(1.0);
    let got = status(
        &mut s,
        json!({"name": "thin", "measure": "thickness", "of": "Case", "min": "1.5 mm"}),
    );
    assert_eq!(got, "failed");
}

/// The first face of `of` the filter `where` finds.
fn face(s: &mut Session, of: &str, filter: Value) -> String {
    let mut filter = filter;
    filter["in"] = json!(of);
    let found = run(
        s,
        "select.query",
        json!({"kind": "face", "where": filter.clone()}),
    );
    found["items"][0]["name"]
        .as_str()
        .unwrap_or_else(|| panic!("no face {filter}: {found}"))
        .to_string()
}

/// A clamp: a block 80 × 40 × 30, its upright edges rounded r = 8, a bore
/// of 24 through z with both mouths chamfered `chamfer`, and two through
/// holes of 6.6 at x = 12 and x = 68. The thinnest place is from the
/// chamfer's rim to a side: 20 − 12 − chamfer.
fn clamp(chamfer: f64) -> Session {
    let mut s = new();
    run(
        &mut s,
        "feature.block",
        json!({"name": "Blank", "size": [80, 40, 30]}),
    );
    run(
        &mut s,
        "feature.fillet",
        json!({"name": "Rounded", "radius": 8, "edges": [["F1:face(2)", "F1:face(4)"], ["F1:face(2)", "F1:face(5)"], ["F1:face(3)", "F1:face(4)"], ["F1:face(3)", "F1:face(5)"]]}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "BoreTool", "radius": 12, "height": 40, "position": [40, 20, -5]}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"name": "Bored", "op": "subtract", "target": "Rounded", "tool": "BoreTool"}),
    );
    let wall = face(
        &mut s,
        "Bored",
        json!({"surface": "cylinder", "radius": "12 mm"}),
    );
    let top = face(
        &mut s,
        "Bored",
        json!({"surface": "plane", "above": {"z": 29}}),
    );
    let bottom = face(
        &mut s,
        "Bored",
        json!({"surface": "plane", "below": {"z": 1}}),
    );
    run(
        &mut s,
        "feature.chamfer",
        json!({"name": "Eased", "distance": chamfer, "edges": [[top, wall.clone()], [bottom, wall]]}),
    );
    let mut last = "Eased".to_string();
    for (i, x) in [12, 68].into_iter().enumerate() {
        run(
            &mut s,
            "feature.cylinder",
            json!({"name": format!("Drill{i}"), "radius": 3.3, "height": 40, "position": [x, 20, -5]}),
        );
        let next = if i == 1 {
            "Clamp".to_string()
        } else {
            format!("Hole{i}")
        };
        run(
            &mut s,
            "feature.boolean",
            json!({"name": next, "op": "subtract", "target": last, "tool": format!("Drill{i}")}),
        );
        last = next;
    }
    s
}

#[test]
fn the_clamp_is_proven_to_its_thinnest_wall() {
    let mut s = clamp(1.5);
    let (found, proven) = timed(
        &mut s,
        "measure.thickness",
        json!({"of": "Clamp"}),
        "thickness",
        "clamp",
    );
    assert!((found - 6.5).abs() < 1e-5, "{found}");
    assert!(proven >= 3.0, "proven only {proven}");
    let got = status(
        &mut s,
        json!({"name": "thin", "measure": "thickness", "of": "Clamp", "min": "3 mm"}),
    );
    assert_eq!(got, "passed");
    let got = status(
        &mut s,
        json!({"name": "thick", "measure": "thickness", "of": "Clamp", "min": "7 mm"}),
    );
    assert_eq!(got, "failed");
}
