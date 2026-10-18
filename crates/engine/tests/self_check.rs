//! No feature hands on a body that passes through or touches itself
//! (docs/design/regeneration.md › *Never through itself*): what the kernel
//! builds and the checker passes is still refused, with the faces named.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use craft3d_engine::{Access, Session, Workspace};
use serde_json::{Value, json};

fn session() -> Session {
    let mut s = Session::new(Workspace {
        read: Access::None,
        write: Access::None,
    });
    run(&mut s, "file.new", json!({"units": "mm"}));
    s
}

fn feature(session: &mut Session, name: &str) -> Value {
    let document = run(session, "document.inspect", json!({}));
    document["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == name)
        .unwrap_or_else(|| panic!("no feature {name}: {document}"))
        .clone()
}

/// A block 40 x 40 x 10 with a notch cut through it whose point lies
/// `tip` mm in from the front face.
fn notched_plate(tip: f64) -> Session {
    let mut s = session();
    run(
        &mut s,
        "feature.block",
        json!({"size": ["40 mm", "40 mm", "10 mm"], "name": "Plate"}),
    );
    run(&mut s, "sketch.create", json!({"name": "Notch"}));
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Notch", "closed": true,
               "points": [[20, tip], [30, 20], [10, 20]]}),
    );
    run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Notch", "throughAll": true, "symmetric": true,
               "cut": "Plate", "name": "Notched"}),
    );
    s
}

/// The notch's point on the front face: the walls of the notch touch the
/// front along a line, in one shell. The kernel's checker passes it; the
/// boolean refuses it as zero thickness (since 2026-10-17; before, the
/// check of the result did, as a shell that touches itself), naming the
/// faces that meet there. In from the front, or open to it, it is built.
#[test]
fn a_cut_that_leaves_the_part_touching_itself_is_refused() {
    let mut s = notched_plate(0.0);
    let notched = feature(&mut s, "Notched");
    assert_eq!(notched["status"], "failed", "{notched}");
    let why = notched["error"].as_str().unwrap();
    assert!(
        why.starts_with("the result would have zero thickness at (20, 0, "),
        "{why}"
    );
    assert!(why.contains(" and F"), "{why}");
    // A face of the plate and a wall of the notch (the tool).
    assert!(why.contains("F1:face("), "{why}");
    assert!(why.contains(":side("), "{why}");
    let check = run(&mut s, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], false, "{check}");

    for tip in [5.0, -5.0] {
        let mut s = notched_plate(tip);
        let notched = feature(&mut s, "Notched");
        assert_eq!(notched["status"], "ok", "tip {tip}: {notched}");
        let check = run(&mut s, "document.check", json!({"deep": true}));
        assert_eq!(check["ok"], true, "tip {tip}: {check}");
    }
}

/// The hull of a showcase as it was on 2026-10-14: the inner plan ends in
/// the same bow point (200, 0) as the outer one, so the hollowed hull's
/// inner and outer walls touch along the line x = 200, y = 0 from
/// z = 27 to 66: two vertices at one point, two edges along one line,
/// one shell. The checker passed it and no closed mesh could be made of
/// it; the subtraction is now refused, at that line (by the boolean as
/// zero thickness since 2026-10-17, by the check of its result before).
#[test]
fn the_hull_whose_walls_touched_at_the_bow_is_refused() {
    let steps: Vec<(String, Value)> =
        serde_json::from_str(include_str!("self_check/hull_bow.steps.json")).unwrap();
    let mut s = session();
    for (id, params) in &steps {
        run(&mut s, id, params.clone());
    }
    let hull = feature(&mut s, "Hull");
    assert_eq!(hull["status"], "failed", "{hull}");
    let why = hull["error"].as_str().unwrap();
    assert!(
        why.starts_with("the result would have zero thickness at (200, 0, "),
        "{why}"
    );
    let names: Vec<&str> = why
        .split("in a point, at ")
        .nth(1)
        .unwrap()
        .split(", and no solid")
        .next()
        .unwrap()
        .split(" and ")
        .collect();
    assert_eq!(names.len(), 2, "{why}");
    assert_ne!(names[0], names[1], "{why}");
    for name in ["Outer", "Inner"] {
        assert_eq!(feature(&mut s, name)["status"], "ok");
    }
}

/// The checks of booleans, patterns, blends and threads look near what
/// changed; on the ordinary parts they are made for they find nothing,
/// and what they pass `document.check` with `deep` passes too.
#[test]
fn ordinary_parts_pass_the_checks_of_their_features() {
    let mut s = session();
    run(
        &mut s,
        "feature.block",
        json!({"size": ["120 mm", "30 mm", "10 mm"], "name": "Plate"}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": "3 mm", "height": "14 mm", "position": ["10 mm", "10 mm", "-2 mm"],
               "name": "Pin"}),
    );
    run(
        &mut s,
        "feature.pattern",
        json!({"source": "Pin", "count": 10, "step": ["10 mm", "0 mm", "0 mm"], "name": "Pins"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Plate", "tool": "Pins", "name": "Drilled"}),
    );
    // Instances that overlap are joined: the pattern's own check.
    run(
        &mut s,
        "feature.block",
        json!({"size": ["8 mm", "8 mm", "8 mm"], "position": ["0 mm", "40 mm", "0 mm"],
               "name": "Cube"}),
    );
    run(
        &mut s,
        "feature.pattern",
        json!({"source": "Cube", "count": 5, "step": ["6 mm", "0 mm", "0 mm"], "name": "Bar"}),
    );
    for name in ["Drilled", "Bar"] {
        let made = feature(&mut s, name);
        assert_eq!(made["status"], "ok", "{made}");
    }
    let check = run(&mut s, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
}
