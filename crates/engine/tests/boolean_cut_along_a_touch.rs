//! Random design 122490 through its commands: a block with a washer-shaped
//! groove whose outer cylinder is tangent to the block's side along a line,
//! intersected with a block standing on the plane through that line, and
//! the reduction, the same cut by `feature.planeCut` (docs/design/booleans.md
//! › *A cut along the line where a wall thins to nothing*). The boolean's own
//! check refused what it built (*"loop 2 of face 2 is not a simple closed
//! curve: it runs over loop 3 of the face"*); now it builds one shell, with
//! an edge of zero angle where the side and the groove's wall meet, which
//! passes the deep check, of the volume written without the kernel
//! (`crates/kernel/tests/boolean_cut_along_a_touch.rs` › `oracle`, where the
//! reduction is held at four scales and turned).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::print_stdout
)]

use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};

/// The volumes at scale 1, in closed form: the block above z = 0 less the
/// groove's part of it, a slab 2.5 thick of the region 2.5 ≤ √(x² + z²) ≤
/// 12.5 within the block's section, by ∫ √(R² − z²) dz = (z √(R² − z²) +
/// R² asin(z / R)) / 2 (`oracle::volume` in the kernel's test; the same to
/// 1e-12 by Gauss–Legendre quadrature of the section's width, each circle's
/// square root taken out by z = r sin t, in `plan/seed-122490/oracle.py` in
/// the workshop). The seed's block reaches z = 12.5, the reduction's z = 5.
const SEED: f64 = 3393.895405797801;
const REDUCED: f64 = 1341.316563374929;

/// The seed's own commands, as the random designs made them (scale 1e-3,
/// tolerance 1e-9; `plan/kernel-work-owed/invalid-body-seeds-2` in the
/// workshop).
fn seed_as_made() -> Vec<(&'static str, Value)> {
    vec![
        ("file.new", json!({"tolerance": 1e-09, "units": "mm"})),
        (
            "feature.block",
            json!({"angle": "0 deg", "axis": [0.0, 0.0, 1.0], "name": "N0",
                   "position": [-0.005, -0.0012499999999999998, -0.0025],
                   "size": [0.0175, 0.0175, 0.015]}),
        ),
        (
            "sketch.create",
            json!({"name": "S1", "offset": 0.0, "plane": "xy"}),
        ),
        (
            "sketch.polyline",
            json!({"closed": true, "sketch": "S1", "points": [
                [0.0025, 0.0075], [0.0125, 0.0075], [0.0125, 0.01], [0.0025, 0.01]]}),
        ),
        (
            "feature.revolve",
            json!({"angle": "360 deg", "axis": "y", "cut": "N0", "name": "N1", "sketch": "S1"}),
        ),
        (
            "feature.block",
            json!({"angle": "0 deg", "axis": [0.0, 1.0, 0.0], "name": "N2",
                   "position": [-0.005000000000000001, -0.0012499999999999998, 0.012499999999999999],
                   "size": [0.02, 0.0125, 0.02]}),
        ),
        (
            "feature.boolean",
            json!({"keepTool": false, "name": "N4", "op": "intersect", "target": "N2", "tool": "N1"}),
        ),
    ]
}

/// The block and the groove cut from it, at scale `k` (the document's
/// tolerance a millionth of it), the block from `bottom` to `top`.
fn grooved(k: f64, bottom: f64, top: f64) -> Vec<(&'static str, Value)> {
    vec![
        ("file.new", json!({"units": "mm", "tolerance": 1e-6 * k})),
        (
            "feature.block",
            json!({"angle": "0 deg", "axis": [0.0, 0.0, 1.0], "name": "N0",
                   "position": [-5.0 * k, -1.25 * k, bottom * k],
                   "size": [17.5 * k, 17.5 * k, (top - bottom) * k]}),
        ),
        (
            "sketch.create",
            json!({"name": "S1", "offset": 0.0, "plane": "xy"}),
        ),
        (
            "sketch.polyline",
            json!({"closed": true, "sketch": "S1", "points": [
                [2.5 * k, 7.5 * k], [12.5 * k, 7.5 * k], [12.5 * k, 10.0 * k], [2.5 * k, 10.0 * k]]}),
        ),
        (
            "feature.revolve",
            json!({"angle": "360 deg", "axis": "y", "cut": "N0", "name": "N1", "sketch": "S1"}),
        ),
    ]
}

/// The seed's commands at scale `k`: the block N2 (its z along y) stands on
/// the plane z = 0 and reaches the grooved block's top.
fn seed(k: f64) -> Vec<(&'static str, Value)> {
    let mut steps = grooved(k, -2.5, 12.5);
    steps.push((
        "feature.block",
        json!({"angle": "0 deg", "axis": [0.0, 1.0, 0.0], "name": "N2",
               "position": [-5.0 * k, -1.25 * k, 12.5 * k], "size": [20.0 * k, 12.5 * k, 20.0 * k]}),
    ));
    steps.push((
        "feature.boolean",
        json!({"keepTool": false, "name": "N4", "op": "intersect", "target": "N2", "tool": "N1"}),
    ));
    steps
}

/// The reduction at scale `k`: the grooved block z −5..5 cut by the plane
/// z = 0, keeping what lies above it.
fn reduced(k: f64) -> Vec<(&'static str, Value)> {
    let mut steps = grooved(k, -5.0, 5.0);
    steps.push((
        "feature.planeCut",
        json!({"name": "N4", "target": "N1", "planes": [
            {"normal": [0.0, 0.0, 1.0], "point": [0.0, 0.0, 0.0], "side": "above"}]}),
    ));
    steps
}

/// What is wrong with the result of `steps` at scale `k`, if anything: a
/// step refused, the deep check failed, or the volume further from
/// `expected` (at scale 1) than the tolerance times its area.
fn wrong(label: &str, steps: Vec<(&'static str, Value)>, k: f64, expected: f64) -> Option<String> {
    let mut s = Session::new(Workspace::unrestricted());
    for (id, params) in steps {
        let made = s.execute(id, &params).unwrap();
        if made["status"] == "failed" {
            return Some(format!("{label}: {id} refused: {}", made["error"]));
        }
    }
    let check = s.execute("document.check", &json!({"deep": true})).unwrap();
    if check["ok"] != true {
        return Some(format!("{label}: the deep check failed: {check}"));
    }
    let measured = s
        .execute("document.measure", &json!({"feature": "N4"}))
        .unwrap();
    let (volume, area) = (
        measured["volume"].as_f64().unwrap() / (k * k * k),
        measured["area"].as_f64().unwrap_or(0.0) / (k * k),
    );
    let allowed = 1e-6 * area.max(1.0);
    println!("{label}: volume {volume:.9} against {expected:.9}, allowed {allowed:.1e}");
    ((volume - expected).abs() > allowed)
        .then(|| format!("{label}: volume {volume} where {expected} is written without the kernel"))
}

#[test]
fn seed_122490_builds_the_volume_written_without_the_kernel() {
    let mut found = Vec::new();
    found.extend(wrong("seed as made", seed_as_made(), 1e-3, SEED));
    for k in [1e-3, 1.0, 1e3] {
        found.extend(wrong(&format!("seed k={k}"), seed(k), k, SEED));
        found.extend(wrong(&format!("reduced k={k}"), reduced(k), k, REDUCED));
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
}
