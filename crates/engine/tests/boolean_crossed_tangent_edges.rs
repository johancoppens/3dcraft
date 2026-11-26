//! Random design 121704 through its commands: half a torus whose rims are
//! filleted, less a copy moved along its axis, the two end caps in one
//! plane, each bounded by its fillet's tangent edge, and the two edges
//! crossing there (docs/design/booleans.md › *Two tangent edges that cross
//! on a shared cap*). The boolean's own check refused what it built
//! (*"edge 10 is used 1 times"*); now it builds one shell, which passes the
//! deep check, of the volume written without the kernel
//! (`crates/kernel/tests/boolean_crossed_tangent_edges.rs` › `oracle`,
//! where the reduction, the end rim alone, is held at four scales and
//! turned).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::print_stdout
)]

use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};

/// The volume of the result at scale 1 with both rims filleted (the seed)
/// and with the end rim alone (its reduction), written without the kernel
/// (`oracle::less(2.0)` and `oracle::less(1.0)` in the kernel's test; the
/// same to twelve digits in `plan/seed-121704/tools/oracle2.py` in the
/// workshop, where the fillet's own volume agrees with its flux).
const SEED: f64 = 4187.470524706867;
const REDUCED: f64 = 4206.825286588981;

/// The seed's commands at scale `k` (the document's tolerance a millionth
/// of it), with both rims filleted or the end rim alone.
fn steps(k: f64, both_rims: bool) -> Vec<(&'static str, Value)> {
    let mut edges = vec![json!(["F2:side(e2)", "F2:endCap(e2)"])];
    if both_rims {
        edges.insert(0, json!(["F2:side(e2)", "F2:startCap(e2)"]));
    }
    vec![
        ("file.new", json!({"units": "mm", "tolerance": 1e-6 * k})),
        (
            "sketch.create",
            json!({"name": "S0", "offset": -10.0 * k, "plane": "xy"}),
        ),
        (
            "sketch.circle",
            json!({"center": [12.5 * k, 2.5 * k], "radius": 7.5 * k, "sketch": "S0"}),
        ),
        (
            "feature.revolve",
            json!({"angle": "180 deg", "axis": "y", "name": "N0", "sketch": "S0"}),
        ),
        (
            "feature.fillet",
            json!({"chain": false, "edges": edges, "name": "N1", "radius": 2.5 * k}),
        ),
        (
            "feature.transform",
            json!({"keep": true, "name": "N2", "source": "N1", "translate": [0.0, 7.5 * k, 0.0]}),
        ),
        (
            "feature.boolean",
            json!({"keepTool": true, "name": "N4", "op": "subtract", "target": "N1", "tool": "N2"}),
        ),
    ]
}

/// What is wrong with the seed (or its reduction) at scale `k`, if
/// anything: a step refused, the deep check failed, or the result's volume
/// further from `expected` (at scale 1) than the tolerance times its area.
fn wrong(label: &str, k: f64, both_rims: bool, expected: f64) -> Option<String> {
    let mut s = Session::new(Workspace::unrestricted());
    for (id, params) in steps(k, both_rims) {
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
fn seed_121704_builds_the_volume_written_without_the_kernel() {
    let mut found = Vec::new();
    for k in [1.0, 1e-3, 1e3] {
        found.extend(wrong(&format!("seed k={k}"), k, true, SEED));
        found.extend(wrong(&format!("reduced k={k}"), k, false, REDUCED));
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
}
