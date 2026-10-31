//! The faces of a pattern's seed are named alike in every instance, so a
//! fillet on instance k follows through an edit upstream of the pattern
//! (the Perseverance's finding of 2026-10-21: after a pattern of a
//! chamfered fin, instance 0 was `F4:copy(0)/F2:face(2)` and instance 3
//! `F4:copy(3)/F3:copy(0)/F2:face(2)`, so a chamfer put on the seed later,
//! or taken off, lost every name but instance 0's). The oracle is where
//! the fillet's material lies: on the root of instance k, at its angle
//! about the pattern's axis, before and after every edit, at three
//! scales.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;

use common::checked::{body, built, feature, new_document, run, session};
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::f64::consts::PI;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn at(k: f64, p: [f64; 3]) -> Value {
    json!([p[0] * k, p[1] * k, p[2] * k])
}

/// A rod along x about (y, z) = (0, 95) with eight fins round it, and a
/// fillet `Roots` at the root of instance `instance`, named as instance 0
/// is: F1 the rod, F2 the fin, F3 the pattern, F4 the union, F5 the
/// fillet.
fn finned(session: &mut Session, k: f64, instance: u32) {
    new_document(session, k);
    built(
        session,
        "feature.cylinder",
        json!({"radius": 32.0 * k, "height": 70.0 * k, "position": at(k, [-158.0, 0.0, 95.0]),
               "axis": [1, 0, 0], "name": "Body"}),
    );
    built(
        session,
        "feature.block",
        json!({"size": at(k, [58.0, 1.6, 26.0]), "position": at(k, [-154.0, -0.8, 125.0]),
               "name": "Fin"}),
    );
    built(
        session,
        "feature.pattern",
        json!({"source": "Fin", "count": 8, "angle": "45 deg", "axis": [1, 0, 0],
               "point": at(k, [-150.0, 0.0, 95.0]), "name": "Fins"}),
    );
    built(
        session,
        "feature.boolean",
        json!({"op": "union", "target": "Body", "tool": "Fins", "name": "U"}),
    );
    built(
        session,
        "feature.fillet",
        json!({"edges": [[format!("F3:copy({instance})/F2:face(2)"), "F1:face(0)"]],
               "radius": 0.6 * k, "name": "Roots"}),
    );
}

/// A chamfer of the fin's top edges put in after the fin (F6), the
/// pattern made from it.
fn chamfer_the_seed(session: &mut Session, k: f64) {
    run(session, "history.rollback", json!({"feature": "Fin"}));
    built(
        session,
        "feature.chamfer",
        json!({"edges": [["F2:face(1)", "F2:face(2)"], ["F2:face(1)", "F2:face(3)"],
                         ["F2:face(1)", "F2:face(4)"], ["F2:face(1)", "F2:face(5)"]],
               "distance": 0.5 * k, "name": "FinChamfer"}),
    );
    run(session, "history.rollback", json!({"feature": null}));
    run(
        session,
        "feature.edit",
        json!({"feature": "Fins", "source": "FinChamfer"}),
    );
}

fn volume_and_moment(body: &Value) -> (f64, [f64; 3]) {
    let v = body["volume"].as_f64().unwrap();
    let c = &body["centroid"];
    (v, [0, 1, 2].map(|i| c[i].as_f64().unwrap() * v))
}

/// The angle about the rod's axis (from +z, turning as the pattern does)
/// of the material the fillet `Roots` adds to the union `U`.
fn fillet_angle(session: &mut Session, k: f64, what: &str) -> f64 {
    let state = feature(session, "Roots");
    assert_eq!(state["status"], "ok", "{what}: {state}");
    let after = volume_and_moment(&body(session, "Roots"));
    // Suppressed, the fillet passes the union's body on unchanged.
    run(session, "feature.suppress", json!({"feature": "Roots"}));
    let before = volume_and_moment(&body(session, "Roots"));
    run(
        session,
        "feature.suppress",
        json!({"feature": "Roots", "suppressed": false}),
    );
    let added = after.0 - before.0;
    assert!(added > 0.0, "{what}: the fillet added no material: {added}");
    let y = (after.1[1] - before.1[1]) / added;
    let z = (after.1[2] - before.1[2]) / added - 95.0 * k;
    // Turned by θ about +x, +z goes to (0, −sin θ, cos θ).
    (-y).patan2(z).rem_euclid(2.0 * PI)
}

fn assert_on_instance(session: &mut Session, k: f64, instance: u32, what: &str) {
    let angle = fillet_angle(session, k, what);
    let expected = f64::from(instance) * PI / 4.0;
    let off = (angle - expected + PI).rem_euclid(2.0 * PI) - PI;
    // The fillet runs along the fin's front face, 0.8 off its middle at
    // radius 32: about 1.4° from the instance's angle; the next instance
    // is 45° away.
    assert!(
        off.abs() < 5f64.to_radians(),
        "{what}: the fillet lies at {:.2}°, not on instance {instance} ({:.2}°)",
        angle.to_degrees(),
        expected.to_degrees()
    );
}

fn face_names(session: &mut Session, name: &str) -> Vec<String> {
    body(session, name)["faceNames"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|n| n.as_str().map(str::to_string))
        .collect()
}

#[test]
fn a_fillet_on_an_instance_follows_a_chamfer_put_on_the_seed_and_taken_off() {
    for k in SCALES {
        for instance in [0u32, 3, 6] {
            let what = format!("instance {instance} at {k}");
            let mut session = session();
            finned(&mut session, k, instance);
            assert_on_instance(&mut session, k, instance, &format!("{what}, plain"));
            chamfer_the_seed(&mut session, k);
            assert_on_instance(&mut session, k, instance, &format!("{what}, chamfered"));
            built(
                &mut session,
                "feature.edit",
                json!({"feature": "FinChamfer", "distance": 0.3 * k}),
            );
            assert_on_instance(&mut session, k, instance, &format!("{what}, chamfer 0.3"));
            // The chamfer taken off again, the pattern made from the fin.
            run(
                &mut session,
                "feature.delete",
                json!({"feature": "FinChamfer"}),
            );
            run(
                &mut session,
                "feature.edit",
                json!({"feature": "Fins", "source": "Fin"}),
            );
            assert_on_instance(&mut session, k, instance, &format!("{what}, unchamfered"));
        }
    }
}

/// Every instance names the faces the chamfer kept as instance 0 does:
/// no link for the chamfer in any of them.
#[test]
fn every_instance_of_a_chamfered_seed_is_named_alike() {
    let k = 1.0;
    let mut session = session();
    finned(&mut session, k, 5);
    chamfer_the_seed(&mut session, k);
    // The faces the chamfer kept of the fin's top and sides, through the
    // pattern (F3), the union's tool (F4) and the fillet (F5).
    let names = face_names(&mut session, "Roots");
    for instance in 0..8 {
        for face in 1..6 {
            let way = if instance == 0 {
                "1".to_string()
            } else {
                format!("1,{instance}")
            };
            let name = format!("F5:copy(0)/F4:copy({way})/F2:face({face})");
            assert!(names.contains(&name), "{name} not in {names:?}");
        }
    }
    assert!(!names.iter().any(|n| n.contains("F6:copy(0)")), "{names:?}");
}
