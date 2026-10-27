//! What the sea-kayak Jig, built natively, found the platform refusing
//! (2026-10-27), each from the steps that made it (`kayak_findings/`):
//!
//! 1. A developable between a curve that inflects and one that does not
//!    (a profile scaled by 1.1 against its plan, so not a generalised
//!    cylinder): the branches of the condition that cross at an inflection
//!    pass each other, and the rulings turned back. Now bridged by a fan.
//! 2. A plate on a developable panel: refused, "the joint reaches past
//!    the faces' surfaces continued", as the panel's surface was continued
//!    across its rulings by as far as the longest one moves. Now each
//!    border is continued far enough past every vertex on it.
//! 5. A 5 m hull's panels: the fit near their ends did not settle (the
//!    walk crept into a side, rulings 2e-6 apart beside rulings 3e-3
//!    apart). Now the path is thinned before the fit; the curvature
//!    measure and its assertion settle.
//! 6. The coaming: a boolean of the hull's cylinders refused ("imprint
//!    lost"); built since 2026-10-27, the ring cut from the hull owed.
//! 7. The hull's panels with a deep check: all built and passing (the
//!    check's finding on TipHalfD was its ray casting, fixed 2026-10-27).
//! 8. The hull's panels unfolded: panels 2 and 3 had end fans 1e-5 mm
//!    long (rail ends that line up with a ruling within the tolerance),
//!    which changed their faces' and edges' names; now no fan within the
//!    tolerance of the strip beside it. The unfolds were refused by a
//!    stretch charged over the cube of the whole 5 m panel, then by the
//!    long edges' developments, integrated across the chines' knots; now
//!    the stretch is the strip's and the edges are integrated span by
//!    span. The deck does not build here (owed).
//! 9. The level for the hull's displacement: a step of the search far
//!    below the keel found nothing, and that was refused ("it may reach
//!    878 below it"), as each trimmed face of the hull, cut from faces
//!    3000 mm long, was bound by its whole surface. Now a trimmed freeform
//!    face is bound by what of its surface lies in its trim.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};
use std::path::PathBuf;

/// Runs the steps of `file`: the session, and what failed (a step that is
/// an error, a deep check that is not ok, an assertion that cannot be
/// measured, a feature that failed).
fn steps(file: &str) -> (Session, Vec<String>) {
    let text = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/kayak_findings")
            .join(file),
    )
    .unwrap();
    let steps: Vec<(String, Value)> = serde_json::from_str(&text).unwrap();
    let mut s = Session::new(Workspace::unrestricted());
    s.execute("file.new", &json!({"units": "mm"})).unwrap();
    let mut failed = Vec::new();
    for (id, params) in &steps {
        match s.execute(id, params) {
            Ok(v) => {
                if id == "document.check" && v["ok"] != true {
                    failed.push(format!("{id}: {v}"));
                }
                if id == "assert.add" && v["status"] != "passed" {
                    failed.push(format!("{id}: {v}"));
                }
            }
            Err(e) => failed.push(format!("{id}: {e}")),
        }
    }
    let document = s.execute("document.inspect", &json!({})).unwrap();
    for f in document["features"].as_array().unwrap() {
        if f["status"] != "ok" {
            failed.push(format!("feature {}: {}", f["name"], f["error"]));
        }
    }
    (s, failed)
}

#[test]
fn a_developable_between_a_curve_that_inflects_and_one_that_does_not_is_built() {
    let (_, failed) = steps("sea_kayak_1.steps.json");
    assert!(failed.is_empty(), "{failed:#?}");
}

#[test]
fn a_plate_on_a_developable_panel_is_built_as_on_the_face_it_lies_on() {
    let (_, failed) = steps("sea_kayak_2.steps.json");
    assert!(failed.is_empty(), "{failed:#?}");
}

#[test]
fn the_panels_of_a_five_metre_hull_are_built_and_measured() {
    let (mut s, failed) = steps("sea_kayak_5.steps.json");
    assert!(failed.is_empty(), "{failed:#?}");
    let measured = s
        .execute("measure.developability", &json!({"of": ["Panel3S"]}))
        .unwrap();
    assert_eq!(measured["developable"], true, "{measured}");
}

#[test]
fn the_coaming_boolean_is_refused_as_owed_not_wrong() {
    // The hull's first intersect lost the ruling in which an end plane
    // cuts a neighbouring cylinder ("imprint lost"), and the tips' first
    // was refused at the edge of tolerance: both were the distance from a
    // segment to a flat box read as its rounding (docs/design/booleans.md,
    // *The sea kayak's tips*, 2026-10-27), and both are built now. Owed:
    // the coaming's ring cut from the hull, refused as a curve that
    // cannot be fitted; nothing else fails, and what is built passes the
    // deep check.
    let (mut s, failed) = steps("sea_kayak_6.steps.json");
    assert_eq!(failed.len(), 2, "{failed:#?}");
    assert!(
        failed[0].starts_with("feature \"RingIn\"") && failed[0].contains("does not support"),
        "{failed:#?}"
    );
    assert!(failed[1].starts_with("feature \"Coaming\""), "{failed:#?}");
    let check = s.execute("document.check", &json!({"deep": true})).unwrap();
    let problems = check["problems"].as_array().unwrap();
    assert!(
        problems
            .iter()
            .all(|p| p["name"] == "RingIn" || p["name"] == "Coaming"),
        "{check}"
    );
}

#[test]
fn the_hull_panels_pass_the_deep_check() {
    // Every feature is built and the deep check finds nothing: its one
    // finding, the intersect of the tips, was the check's own ray casting
    // missing a crossing (docs/design/booleans.md, *The sea kayak's tips*,
    // 2026-10-27; boolean_findings.rs › the_sea_kayak_tips_against_the_deck).
    let (_, failed) = steps("sea_kayak_7.steps.json");
    assert!(failed.is_empty(), "{failed:#?}");
}

#[test]
fn the_level_for_the_hull_displacement_is_found() {
    // A regression of 2026-10-27 (r1-consolidate): the waterline was
    // found at 111.7 mm in 10 steps before nothing found below was
    // checked against each freeform face's whole surface.
    let (mut s, failed) = steps("sea_kayak_9.steps.json");
    assert!(failed.is_empty(), "{failed:#?}");
    let level = s
        .execute(
            "measure.levelFor",
            &json!({"of": "Hull", "volume": "98865474"}),
        )
        .unwrap();
    let offset = level["offset"].as_f64().unwrap();
    assert!((offset - 111.7).abs() < 0.1, "{level}");
    let volume = level["volume"].as_f64().unwrap();
    assert!(
        (volume - 98_865_474.0).abs() <= 1e-9 * 98_865_474.0,
        "{level}"
    );
}

#[test]
fn the_hull_is_cut_round_30_mm() {
    // The boolean refused the hull's cut at 30.28 mm as an invalid body
    // (docs/design/sections-and-levels.md, Open, until 2026-10-27); on
    // main 4d82484c it is measured. Held to the section: the volume below
    // grows by the section's area, (V(30.3) − V(30.2)) / 0.1 within 1e-4
    // of the mean of the sections there.
    let (mut s, failed) = steps("sea_kayak_9.steps.json");
    assert!(failed.is_empty(), "{failed:#?}");
    let below = |s: &mut Session, offset: &str| {
        let m = s
            .execute("measure.below", &json!({"of": "Hull", "offset": offset}))
            .unwrap_or_else(|e| panic!("{offset}: {e}"));
        (
            m["volume"].as_f64().unwrap(),
            m["section"]["area"].as_f64().unwrap(),
        )
    };
    let (low, low_area) = below(&mut s, "30.2 mm");
    let (at, _) = below(&mut s, "30.28 mm");
    let (high, high_area) = below(&mut s, "30.3 mm");
    assert!(low < at && at < high, "{low} {at} {high}");
    let rate = (high - low) / 0.1;
    let mean = 0.5 * (low_area + high_area);
    assert!((rate - mean).abs() <= 1e-4 * mean, "{rate} for {mean}");
}

#[test]
fn the_hull_panels_have_no_fans_where_their_ends_line_up_with_a_ruling() {
    let (mut s, failed) = steps("sea_kayak_8.steps.json");
    // The four hull panels are built and unfold (U0 to U3); only the deck
    // (Panel4S, sheer to ridge) is refused, its condition turning back
    // before the end of a rail (owed: docs/design/developable.md), and
    // its unfold U4 with it.
    for f in &failed {
        assert!(f.contains("Panel4S") || f.contains("U4"), "{failed:#?}");
    }
    assert!(!failed.is_empty(), "the deck was built: update this test");
    for panel in ["Panel0S", "Panel1S", "Panel2S", "Panel3S"] {
        let edges = s
            .execute("document.edges", &json!({"feature": panel}))
            .unwrap();
        let edges = edges["edges"].as_array().unwrap();
        // One face: two rails and the two rulings at their ends.
        assert_eq!(edges.len(), 4, "{panel}: {edges:#?}");
        for e in edges {
            assert!(
                e["length"].as_f64().unwrap() > 10.0,
                "{panel}: a short edge {e}"
            );
            assert_eq!(
                e["faces"],
                json!([format!(
                    "{}:face(0)",
                    e["name"].as_str().unwrap().split(':').next().unwrap()
                )]),
                "{panel}: {e}"
            );
        }
    }
}
