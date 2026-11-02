//! Sheets that meet tangentially along an edge, joined by
//! `feature.boolean {"op": "sew"}` (2026-10-31), through the commands: the
//! crown and a side of the class-A fuel tank (`docs/examples/tank.json`),
//! the side held in curvature (G2) to the crown. A union intersects the
//! two first and refuses (their surfaces touch along the whole edge); the
//! sewing joins them along the edge, the seam is proven G2 on the sewn
//! body, and the document saves and reopens with the operation (format
//! 59). The seams are proven G2 at the default tolerances.

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

/// The sewn sheet's seams are uncut, 887 mm, so their curvature threshold
/// is 1.127e-12 per mm (`angular / L`). Their bounds were 1.35e-12 (right)
/// and 1.70e-12 (left), undecided with the measure's work spent, until
/// 2026-11-02: a half is cut from the piece it came from, not from its
/// parent, and a rounding counts `u = 2⁻⁵³` (`continuity.rs`); now right
/// 1.025e-12 (9 % to spare) and left 1.034e-12 (8 %), in 43 and 45
/// pieces. Counting roundings in `u` rather than `ε` is what carries that
/// margin: with `ε` the left seam stays over the threshold.
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

/// The tank's seams are proven G2 at the default tolerances (1e-6 mm,
/// 1e-9 rad), which it no longer changes: each side is held in curvature
/// to the crown along a 0.69 m seam of many knot spans, and the network
/// refuses a side unless `continuity` proves its seam G2 (curvature
/// threshold `min(angular / L, 2 tol / L²)`, 1.19e-12 per mm). Until
/// 2026-11-01 the measure's rounding allowance on such a seam was 10 to 13
/// times that threshold (the rows along differentiated after the
/// positions were rounded), so the side was refused and the example set
/// 1 µm and 1e-6 rad first; with the rows formed from the stored net's
/// differences, and the shift along the seam between corresponding points
/// counted, the bounds were 1.021e-12 across, 5.7e-13 twist and 1.4e-13
/// along. With the crown quintic (spine and shoulders of degree 5,
/// `acrossDegree` 5, 2026-11-01) the bound across was 1.36e-12,
/// undecided, until the expansion along `b` went on through every break
/// where `b`'s rows are C¹ instead of counting the shift once on each side
/// of it (`continuity.rs` › `Between`): now 1.046e-12 on the left seam and
/// 1.181e-12 on the right (`docs/design/surfacing.md`, the tank's finding
/// 1). The
/// expectation is pinned apart from this test by the closed-form long
/// seam of `geom/tests/network_g2.rs` (G2 proven at half a threshold, G1
/// at 1.5), at three scales.
#[test]
fn the_tank_is_proven_g2_at_the_default_tolerances() {
    let steps = tank_until("Crown and side");
    assert!(
        steps.iter().all(|(id, _)| id != "document.setTolerance"),
        "the tank keeps the default tolerances"
    );
    let mut s = common::session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    for (id, params) in steps {
        run(&mut s, &id, params);
    }
    let doc = inspect(&mut s);
    assert_eq!(doc["tolerance"]["linear"], 1e-6, "{}", doc["tolerance"]);
    assert_eq!(doc["tolerance"]["angular"], 1e-9, "{}", doc["tolerance"]);
    for feature in doc["features"].as_array().unwrap() {
        assert_eq!(feature["status"], "ok", "{feature}");
    }
}

/// The whole tank at the default tolerances: both of its seams, measured
/// on the finished shell through `measure.continuity` (the edges its
/// assertions name), are proven G2, and it is hollow: `feature.shell`
/// builds (the sides free of creases on the C⁴ crown, and the vertices
/// where the tunnel's wall and arch meet a side put on their seam,
/// `tweak::on_seam`), its capacity 24.8 L.
#[test]
fn the_finished_tank_measures_g2_along_both_seams() {
    let steps = tank_until("no such step");
    let mut s = common::session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    let mut edges = Vec::new();
    for (id, params) in steps {
        if id == "assert.add" && params["measure"] == "continuity" {
            edges.push(params.clone());
        }
        run(&mut s, &id, params);
    }
    assert_eq!(edges.len(), 2);
    for assertion in edges {
        let out = run(
            &mut s,
            "measure.continuity",
            json!({"edge": assertion["edge"]}),
        );
        assert_eq!(out["order"], "G2", "{out}");
    }
    let doc = inspect(&mut s);
    for feature in doc["features"].as_array().unwrap() {
        assert_eq!(feature["status"], "ok", "{feature}");
    }
    let capacity = doc["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["name"] == "capacity")
        .unwrap();
    let litres = capacity["value"].as_f64().unwrap();
    assert!((24.0..26.0).contains(&litres), "{capacity}");
}
