//! The review of check/nightly-findings (2026-11-25): what its oracles,
//! run finer and by another route, pin of its fold rule for fillets whose
//! sections run back along the edge.
//!
//! - **Where a fillet's sections start to run back.** The builder's two
//!   oracles (`plan/nightly-findings/scripts/runback1.py`, `runback2.py`)
//!   run at finer radii, and one of the review's own by another route
//!   (`plan/nightly-findings/review/scripts/runback_mine.py`,
//!   `runback_mine2.py`: the ball's centre `c` in the plane through `e(s)`
//!   square to the edge moves along it at `|e'| (1 − κ (c − e) · N)`, so
//!   the sections run back where `κ (c − e) · N` passes 1, the edge's
//!   curvature from the conic's implicit form in the cylinder's base
//!   plane, or from the circle through three of its points on the
//!   cylinder's wall), put the first radius at which they run back at
//!   0.00054418 for 20791978 (none at 0.0005441, from 0.9840 to 0.9880 of
//!   the edge by the cone's angle at 0.0005442, back by 3e-10, a third of
//!   the tolerance) and 0.00045825 for 20821184 (none at 0.0004582, back
//!   by 2.1e-8 at 0.0004584). The kernel's refusal starts between the same
//!   two radii, and does so at the scales 0.1, 1e3 and 1e8 with the
//!   tolerance scaled alike (Check 45633310788).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};

/// A new document at `tolerance`, and `steps` run on it, each built.
fn session(tolerance: f64, steps: &[(&str, Value)]) -> Session {
    let mut s = Session::new(Workspace::unrestricted());
    s.execute("file.new", &json!({"units": "mm", "tolerance": tolerance}))
        .unwrap();
    for (id, params) in steps {
        let made = s.execute(id, params).unwrap();
        assert_ne!(made["status"], "failed", "{id} {params}: {made}");
    }
    s
}

/// Seed 20791978 as shrunk (`nightly_findings_2026_11_25.rs`).
fn seed_20791978() -> Vec<(&'static str, Value)> {
    vec![
        (
            "feature.cone",
            json!({"angle": "0 deg", "axis": [0.01, 0.01, 1.0], "height": 0.01, "name": "N0", "position": [0.01, 0.0025, 0.01], "radius": 0.0075}),
        ),
        (
            "feature.offsetFace",
            json!({"distance": -0.00125, "faces": ["F1:face(1)"], "name": "N1"}),
        ),
        (
            "feature.cylinder",
            json!({"angle": "0 deg", "axis": [0.01, 1.0, 0.01], "height": 0.01, "name": "N3", "position": [0.000_312_499_999_999_999_84, 0.005, 0.015], "radius": 0.01}),
        ),
        (
            "feature.boolean",
            json!({"keepTool": false, "name": "N4", "op": "intersect", "target": "N3", "tool": "N1"}),
        ),
    ]
}

const HYPERBOLA: [&str; 2] = ["F4:copy(0)/F3:face(1)", "F4:copy(1)/F1:face(0)"];

/// Seed 20821184 as shrunk (`nightly_findings_2026_11_25.rs`).
fn seed_20821184() -> Vec<(&'static str, Value)> {
    vec![
        (
            "feature.cylinder",
            json!({"angle": "90 deg", "axis": [1.0, 1.0, 1.0], "height": 0.01, "name": "N0", "position": [0.01, 0.01, 0.0075], "radius": 0.005}),
        ),
        (
            "feature.sphere",
            json!({"name": "N2", "position": [0.015, 0.011_108_439_182_435_162, 0.01], "radius": 0.01}),
        ),
        (
            "feature.torus",
            json!({"angle": "0 deg", "axis": [0.01, 0.01, -1.0], "major": 0.0075, "minor": 0.005, "name": "N3", "position": [0.0075, 0.015, 0.001_464_466_094_067_261_6]}),
        ),
        (
            "feature.boolean",
            json!({"keepTool": false, "name": "N5", "op": "intersect", "target": "N3", "tool": "N0"}),
        ),
        (
            "feature.boolean",
            json!({"keepTool": false, "name": "N6", "op": "subtract", "target": "N2", "tool": "N5"}),
        ),
    ]
}

const RIM: [&str; 2] = ["F5:copy(1)/F3:face(0)", "F5:copy(1,1)/F1:face(0)"];

/// The steps with every length (positions, heights, radii, distances, a
/// torus's radii) times `k`.
fn scaled(steps: &[(&'static str, Value)], k: f64) -> Vec<(&'static str, Value)> {
    steps
        .iter()
        .map(|(id, p)| {
            let mut p = p.clone();
            let map = p.as_object_mut().unwrap();
            for key in ["height", "radius", "distance", "major", "minor"] {
                if let Some(v) = map.get_mut(key) {
                    *v = json!(v.as_f64().unwrap() * k);
                }
            }
            if let Some(v) = map.get_mut("position") {
                let a: Vec<f64> = v
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|x| x.as_f64().unwrap() * k)
                    .collect();
                *v = json!(a);
            }
            (*id, p)
        })
        .collect()
}

/// A fillet of `edge` by `radius` on the body of `steps` at `tolerance`:
/// `None` where it is built and passes the deep check, else the refusal's
/// words (a built body that fails the deep check fails the test).
fn fillet(tolerance: f64, steps: &[(&str, Value)], edge: [&str; 2], radius: f64) -> Option<String> {
    let mut s = session(tolerance, steps);
    let made = s
        .execute(
            "feature.fillet",
            &json!({"chain": false, "edges": [edge], "radius": radius}),
        )
        .unwrap();
    if made["status"] == "failed" {
        return Some(made["error"].as_str().unwrap().to_string());
    }
    let check = s.execute("document.check", &json!({"deep": true})).unwrap();
    assert_eq!(check["ok"], true, "radius {radius}: built, {check}");
    None
}

const RUNS_BACK: &str = "the rolling ball's sections across the edge (each in the plane square to the edge) run back along it";

/// Either side of where the sections first run back, by the three
/// oracles: built and deeply valid below, refused in the words of
/// sections that run back above (never as a bug). At the seeds' own scale
/// (1e-3, tolerance 1e-9) the radii next to the threshold (20791978:
/// 0.0005441 none run back, 0.0005442 back by a third of the tolerance;
/// 20821184: 0.0004582 none, 0.0004584 back by 21 tolerances), and at
/// 0.1, 1e3 and 1e8 times it, with the tolerance alike, radii a little
/// further either side (back by 26 and 74 tolerances above).
#[test]
fn the_refusal_starts_where_the_sections_first_run_back_at_every_scale() {
    for (k, near) in [(1.0, true), (0.1, false), (1e3, false), (1e8, false)] {
        for (name, steps, edge, below, above) in [
            (
                "20791978",
                scaled(&seed_20791978(), k),
                HYPERBOLA,
                0.000_544_1,
                if near { 0.000_544_2 } else { 0.000_544_5 },
            ),
            (
                "20821184",
                scaled(&seed_20821184(), k),
                RIM,
                if near { 0.000_458_2 } else { 0.000_458 },
                if near { 0.000_458_4 } else { 0.000_458_6 },
            ),
        ] {
            let built = fillet(1e-9 * k, &steps, edge, below * k);
            assert_eq!(built, None, "{name} at {k}: radius {below} × {k}");
            let said = fillet(1e-9 * k, &steps, edge, above * k);
            let text = said.unwrap_or_else(|| {
                panic!("{name} at {k}: radius {above} × {k} built, where its sections run back")
            });
            assert!(!text.contains(craft3d_math::BUG), "{name} at {k}: {text}");
            assert!(text.contains(RUNS_BACK), "{name} at {k}: {text}");
        }
    }
}

/// A chamfer of 20821184's rim whose lines across the edge run back along
/// it whole: not built (refused in any words but a bug's). A chamfer's
/// line lies in the plane through `e(s)` square to the edge, so each of
/// its points moves along the edge at `|e'| (1 − κ (P − e) · N)`, and it
/// runs back where `κ (P − e) · N` passes 1; that is largest at a foot.
/// Worked out outside the code (`plan/nightly-findings/review/scripts/
/// chamfer_runback.py`, the feet the distance from `e(s)` in that plane
/// on the torus inside the cylinder and on the cylinder inside the torus,
/// 401 places along the rim): its largest at the torus's foot and at the
/// cylinder's is 0.725 and 0.726 at 0.0013 (built and deeply valid,
/// Check 45633310788), 0.949 and 0.951 at 0.0017, and 1.060 and 1.064 at
/// 0.0019, 1.116 and 1.121 at 0.002, 1.227 and 1.234 at 0.0022, where
/// both feet, and so the whole line, run back over 0.15 to 0.22, 0.13 to
/// 0.24 and 0.11 to 0.27 of the rim (by the cylinder's angle). The two
/// feet pass the line in which the planes meet within 5e-6 of the
/// distance of each other; the kernel refuses these as folding across
/// (`FreeFolds`, Check 45635735135), seen in the thin band at the
/// stretch's ends where one foot is past and the other not yet. The fold
/// along the edge, which `folds` holds fillets to since
/// check/nightly-findings, is not held for a chamfer: where the two feet
/// passed together it is what would (owed: docs/design/blends.md ›
/// *Sections that run back along the edge*).
#[test]
fn seed_20821184_a_chamfer_whose_lines_run_back_whole_is_not_built() {
    for distance in [0.0019, 0.002, 0.0022] {
        let mut s = session(1e-9, &seed_20821184());
        let made = s
            .execute(
                "feature.chamfer",
                &json!({"chain": false, "edges": [RIM], "distance": distance}),
            )
            .unwrap();
        if made["status"] != "failed" {
            let check = s.execute("document.check", &json!({"deep": true})).unwrap();
            panic!(
                "distance {distance}: built, where its lines run back along the rim (the deep check: {check})"
            );
        }
        let text = made["error"].as_str().unwrap();
        assert!(
            !text.contains(craft3d_math::BUG),
            "distance {distance}: {text}"
        );
    }
}
