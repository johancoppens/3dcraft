//! Two findings of the random designs that stood in every nightly run
//! for days (Heavy 45516571671 on `main` 22f0bb13, fuzz from seed
//! 20 780 000), reduced and mended (docs/design/blends.md › *Sections
//! that run back along the edge*).
//!
//! - **20791978** and **20821184**, fillets refused as *"internal error
//!   … the fillet face came out not tangent to a face it should run into
//!   smoothly"*. Each edge bends more tightly, somewhere along it, than
//!   the fillet reaches from it: there the rolling ball's sections, each
//!   in the plane square to the edge, run back along the edge, and the
//!   face made of them turns over (the normal at its borders came out
//!   pointing into the face beside it, which the tangency check at the
//!   borders read as a bug; where the turn lay between that check's seven
//!   points the face went on folded, and was stopped only by the check
//!   that the body does not pass through itself, which could not mesh
//!   it). Refused before the face is spliced in, in words that say so,
//!   and since built along the balls' own path, as a chain's fillets are
//!   (`fillet_run_back.rs`, which holds what each takes away or adds to
//!   the volume worked out outside the code). *Oracle*: where the
//!   sections run back was worked out from the
//!   primitives' parameters alone (`plan/nightly-findings/scripts/
//!   runback1.py` and `runback2.py`: in each plane square to the edge the
//!   ball touching both faces, by Newton's method on the faces' own
//!   equations, its feet and its arc's middle, 4000 planes along the edge):
//!   from a radius between 0.000544 and 0.000546 for 20791978 (near the
//!   vertex at the cylinder's wall, where the edge, a hyperbola, turns
//!   most) and between 0.00045 and 0.00046 for 20821184. The radii either
//!   side are held here, built and deeply checked: below by the
//!   construction in place, above along the balls' own path.

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

/// Seed 20791978 as shrunk (at its scale 1e-3, tolerance 1e-9): a cone
/// whose base is moved in by an eighth of its radius, intersected with a
/// cylinder lying across it; the edge where the cylinder's base (a plane
/// 0.0025 from the cone's axis) meets the cone's side is a branch of a
/// hyperbola whose vertex lies near the edge's end at the cylinder's wall.
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

/// The edge of [`seed_20791978`] the finding names: the cylinder's base
/// and the cone's side.
const HYPERBOLA: [&str; 2] = ["F4:copy(0)/F3:face(1)", "F4:copy(1)/F1:face(0)"];

/// Seed 20821184 as shrunk (scale 1e-3, tolerance 1e-9): a ball less the
/// part of a torus inside a cylinder, which leaves a cavity; the cavity's
/// edge where the torus meets the cylinder's wall.
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

/// What a fillet of `edges` by `radius` on the body of `steps` gives: the
/// refusal's words, or `None` when it is built and passes the deep check
/// (a built body that fails it fails the test).
fn fillet(steps: &[(&str, Value)], edges: Value, radius: f64) -> Option<String> {
    let mut s = session(1e-9, steps);
    let made = s
        .execute(
            "feature.fillet",
            &json!({"chain": false, "edges": edges, "radius": radius}),
        )
        .unwrap();
    if made["status"] == "failed" {
        return Some(made["error"].as_str().unwrap().to_string());
    }
    let check = s.execute("document.check", &json!({"deep": true})).unwrap();
    assert_eq!(check["ok"], true, "radius {radius}: built, {check}");
    // A face that folds over itself is a warning, never part of `ok`.
    let folds: Vec<&Value> = check["warnings"]
        .as_array()
        .map(|w| w.iter().filter(|w| w["kind"] == "fold").collect())
        .unwrap_or_default();
    assert!(folds.is_empty(), "radius {radius}: a fold: {folds:?}");
    None
}

/// The finding's own fillet (both edges it named, as the fuzzer asked
/// them) and the edge alone either side of where its sections start to
/// run back (between 0.000544 and 0.000546 by the oracle): built and
/// deeply valid on both sides. An expectation changed on 2026-11-25:
/// until then the radii past that place were refused (*"… run back along
/// it; a fillet whose sections follow the ball's own path there is not
/// built yet: give a smaller radius"*), which was true of the
/// construction in place, whose face folds there; the rolling ball goes
/// on along the curve where the faces moved in by the radius meet, and
/// the fillet it rolls is now built so (`fillet_run_back.rs` holds what it
/// takes away to the volume worked out from the primitives alone, and to
/// the twin `main` builds where another edge's fillet meets this one).
#[test]
fn seed_20791978_a_fillet_whose_sections_run_back_is_built_along_the_balls_path() {
    let steps = seed_20791978();
    let both = json!([
        ["F4:copy(0)/F3:face(0)", "F4:copy(1)/F1:face(1)"],
        HYPERBOLA
    ]);
    assert_eq!(
        fillet(&steps, both, 0.000_625),
        None,
        "the finding's fillet"
    );
    for radius in [0.0005, 0.00054, 0.00055, 0.00056, 0.0006, 0.000_625] {
        let said = fillet(&steps, json!([HYPERBOLA]), radius);
        assert_eq!(said, None, "radius {radius}");
    }
}

/// The finding's fillet of the cavity's rim, and the rim either side of
/// where its sections start to run back (between 0.00045 and 0.00046 by
/// the oracle): built and deeply valid on both sides (an expectation
/// changed on 2026-11-25, as above; until then refused from 0.00046). On
/// `main` before check/nightly-findings, at 0.00046 and 0.00047 the face
/// that ran back passed the tangency check between its points and was
/// refused only because the check that the body does not pass through
/// itself could not mesh it.
#[test]
fn seed_20821184_a_fillet_whose_sections_run_back_is_built_along_the_balls_path() {
    let steps = seed_20821184();
    for radius in [
        0.0004, 0.00045, 0.00046, 0.00047, 0.00048, 0.0005, 0.000_625,
    ] {
        let said = fillet(&steps, json!([RIM]), radius);
        assert_eq!(said, None, "radius {radius}");
    }
}

/// Owed until 2026-11-25: a fillet whose sections follow the ball's own
/// path where the planes square to the edge would make them run back. The
/// rolling ball of these radii touches both faces all along each edge (its
/// centres lie on the curve where the faces moved in by the radius meet:
/// for 20791978 a plane parallel to a cone's axis and the cone, a
/// hyperbola with no cusp), so the fillet exists; it is built along the
/// balls' path since (`fillet_run_back.rs`).
#[test]
fn fillets_whose_planes_square_to_the_edge_run_back_are_built() {
    assert_eq!(
        fillet(&seed_20791978(), json!([HYPERBOLA]), 0.000_625),
        None
    );
    assert_eq!(fillet(&seed_20821184(), json!([RIM]), 0.000_625), None);
}
