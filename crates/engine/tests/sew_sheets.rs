//! Sheets that meet tangentially along an edge, joined by
//! `feature.boolean {"op": "sew"}` (2026-10-31), through the commands: the
//! crown and a side of the class-A fuel tank (`docs/examples/tank.json`),
//! the side held in curvature (G2) to the crown. A union intersects the
//! two first and refuses (their surfaces touch along the whole edge); the
//! sewing joins them along the edge, the seam is proven G2 on the sewn
//! body, and the document saves and reopens with the operation (format
//! 59). `document.setTolerance` sets the tolerances the seam is proven to.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use common::plain::{failure, inspect};
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::path::PathBuf;

/// The tank example's steps up to (not including) the step that names
/// `stop`.
fn tank_until(stop: &str) -> Vec<(String, Value)> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/examples/tank.json");
    let script: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut out = Vec::new();
    for step in script.as_array().unwrap() {
        let id = step[0].as_str().unwrap().to_string();
        if step[1]["name"] == stop {
            break;
        }
        out.push((id, step[1].clone()));
    }
    out
}

fn session_with_sides() -> Session {
    let mut s = common::session_in(&common::workdir("sew", "sheets"));
    run(&mut s, "file.new", json!({"units": "mm"}));
    for (id, params) in tank_until("Crown and side") {
        run(&mut s, &id, params);
    }
    s
}

#[test]
fn a_side_held_in_curvature_is_sewn_to_its_crown_where_a_union_refuses() {
    let mut s = session_with_sides();
    // The union intersects the two first, and they touch along the edge.
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "union", "target": "Crown", "tool": "Side L", "name": "United"}),
    );
    let why = failure(&mut s, "United");
    assert!(why.contains("tangent"), "{why}");
    run(&mut s, "edit.undo", json!({}));
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "sew", "target": "Crown", "tool": "Side L", "name": "Sewn"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "sew", "target": "Sewn", "tool": "Side R", "name": "Skin"}),
    );
    let doc = inspect(&mut s);
    let skin = doc["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == "Skin")
        .unwrap()
        .clone();
    assert_eq!(skin["faceNames"].as_array().unwrap().len(), 3, "{skin}");
    // The seams of the sewn sheet are the sides' edges on the crown: G2.
    let edges = run(&mut s, "document.edges", json!({"feature": "Skin"}));
    let seams: Vec<String> = edges["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["faces"].as_array().unwrap().len() == 2)
        .map(|e| e["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(seams.len(), 2, "{edges}");
    for seam in &seams {
        let c = run(&mut s, "measure.continuity", json!({"edge": seam}));
        assert_eq!(c["order"], "G2", "{seam}: {c}");
    }
    // The sewing is saved as such and reopens to the same sheet.
    run(
        &mut s,
        "file.save",
        json!({"path": "sewn.3dcraft", "overwrite": true}),
    );
    run(&mut s, "file.open", json!({"path": "sewn.3dcraft"}));
    let reopened = inspect(&mut s);
    let skin = reopened["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == "Skin")
        .unwrap()
        .clone();
    assert_eq!(skin["faceNames"].as_array().unwrap().len(), 3, "{skin}");
}

#[test]
fn sewing_what_is_not_two_sheets_sharing_an_edge_fails_with_the_reason() {
    let mut s = session_with_sides();
    // The two sides share no edge.
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "sew", "target": "Side L", "tool": "Side R", "name": "Apart"}),
    );
    let why = failure(&mut s, "Apart");
    assert!(why.contains("share no boundary edge"), "{why}");
    run(&mut s, "edit.undo", json!({}));
    // A solid is not sewn.
    run(
        &mut s,
        "feature.block",
        json!({"size": [10, 10, 10], "name": "Block"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "sew", "target": "Block", "tool": "Crown", "name": "Solid"}),
    );
    let why = failure(&mut s, "Solid");
    assert!(why.contains("joins two sheets"), "{why}");
    // An unknown op names sew among the ones there are.
    let err = s
        .execute(
            "feature.boolean",
            &json!({"op": "knit", "target": "Crown", "tool": "Side L"}),
        )
        .unwrap_err()
        .to_string();
    assert!(err.contains("sew"), "{err}");
}

/// The tolerances a seam is proven to: at the default angular tolerance
/// of 1e-9 rad the 0.69 m seam of the tank cannot be proven G2 (the
/// curvature threshold `min(angular / L, 2 tol / L²)` is below what the
/// measure can bound in double precision there), so the side is refused;
/// the example sets 1e-3 mm and 1e-6 rad first.
#[test]
fn the_tank_states_the_tolerances_its_seams_are_proven_to() {
    let steps = tank_until("Crown and side");
    let (first, rest) = steps.split_first().unwrap();
    assert_eq!(first.0, "document.setTolerance");
    let mut s = common::session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    for (id, params) in rest {
        run(&mut s, id, params.clone());
    }
    let why = failure(&mut s, "Side L");
    assert!(why.contains("curvature (G2)"), "{why}");
    let out = run(&mut s, "document.setTolerance", first.1.clone());
    assert_eq!(out["was"]["angular"], 1e-9);
    let doc = inspect(&mut s);
    for feature in doc["features"].as_array().unwrap() {
        assert_eq!(feature["status"], "ok", "{feature}");
    }
}
