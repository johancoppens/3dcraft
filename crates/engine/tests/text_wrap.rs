//! Text wrapped onto a cylinder and a cone (`feature.extrude {wrap}`),
//! embossed and engraved, against closed forms: the development is an
//! isometry, so the wrapped region has the glyphs' own area, and the solid
//! a distance `d` out from a cylinder of radius `r` over it has the volume
//! `A (d + d² / 2r)` (in: `A (d − d² / 2r)`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{built, failed, inspect, run, session};
use craft3d_engine::Session;
use craft3d_font::{FontFile, Metrics, filled_outline};
use serde_json::json;
use std::path::PathBuf;

fn text_area(text: &str, size: f64) -> f64 {
    let dir = match std::env::var_os("CRAFT3D_FONTS") {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts"),
    };
    let bytes = std::fs::read(dir.join("NotoSans-Regular.otf")).unwrap();
    let f = FontFile::parse(&bytes).unwrap();
    let s = size / f.cap_height();
    text.chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| filled_outline(&f, f.glyph(c).unwrap()).unwrap().area() * s * s)
        .sum()
}

fn volume_of(s: &mut Session, name: &str) -> f64 {
    let doc = inspect(s);
    let id = doc["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == name)
        .unwrap()["id"]
        .clone();
    doc["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|b| b["feature"] == id)
        .map(|b| {
            assert_eq!(b["valid"], true, "{b}");
            b["volume"].as_f64().unwrap()
        })
        .sum()
}

fn deep(s: &mut Session) {
    let check = run(s, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
}

fn can(s: &mut Session, k: f64) {
    run(s, "file.new", json!({"units": "mm", "tolerance": 1e-6 * k}));
    built(
        s,
        "feature.cylinder",
        json!({"radius": 20.0 * k, "height": 40.0 * k, "name": "Can"}),
    );
    run(s, "sketch.create", json!({"name": "Label"}));
    run(
        s,
        "sketch.text",
        json!({"sketch": "Label", "text": "HOB", "size": 8.0 * k, "at": [-9.0 * k, 16.0 * k]}),
    );
}

#[test]
fn text_wrapped_onto_a_cylinder_has_the_closed_form_volume() {
    for k in [1e-2, 1.0, 1e2] {
        let (r, a) = (20.0 * k, text_area("HOB", 8.0 * k));
        for d in [1.0 * k, -0.8 * k] {
            let mut s = session();
            can(&mut s, k);
            built(
                &mut s,
                "feature.extrude",
                json!({"sketch": "Label", "wrap": "F1:face(0)", "distance": d, "name": "Wrapped"}),
            );
            let expected = a * (d.abs() + d * d.abs() / (2.0 * r));
            let v = volume_of(&mut s, "Wrapped");
            assert!(
                (v - expected).abs() <= 1e-5 * expected,
                "k {k}, d {d}: {v} vs {expected}"
            );
            deep(&mut s);
        }
    }
}

#[test]
fn embossed_and_engraved_on_a_cylinder() {
    let (r, h) = (20.0, 40.0);
    let can_volume = std::f64::consts::PI * r * r * h;
    let a = text_area("HOB", 8.0);
    // Raised.
    let mut s = session();
    can(&mut s, 1.0);
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Label", "wrap": "F1:face(0)", "distance": 1, "join": "Can", "name": "Raised"}),
    );
    let v = volume_of(&mut s, "Raised");
    let expected = can_volume + a * (1.0 + 1.0 / (2.0 * r));
    assert!((v - expected).abs() <= 1e-5 * expected, "{v} vs {expected}");
    deep(&mut s);
    // Engraved.
    let mut s = session();
    can(&mut s, 1.0);
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Label", "wrap": "F1:face(0)", "distance": -0.5, "cut": "Can", "name": "Cut"}),
    );
    let v = volume_of(&mut s, "Cut");
    let expected = can_volume - a * (0.5 - 0.25 / (2.0 * r));
    assert!((v - expected).abs() <= 1e-5 * expected, "{v} vs {expected}");
    deep(&mut s);
}

#[test]
fn text_wrapped_onto_a_cone_is_within_its_bounds() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    built(
        &mut s,
        "feature.cone",
        json!({"radius": 30, "height": 60, "name": "Cone"}),
    );
    run(&mut s, "sketch.create", json!({"name": "Label"}));
    run(
        &mut s,
        "sketch.text",
        json!({"sketch": "Label", "text": "CONE", "size": 6, "at": [-10, 8]}),
    );
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Label", "wrap": "F1:face(0)", "distance": 0.5, "name": "OnCone"}),
    );
    deep(&mut s);
    // Between the volumes over the widest and the narrowest circle the
    // text reaches (its slant band from 8 − 1 to 8 + 7 along the cone).
    let a = text_area("CONE", 6.0);
    let half = (30.0f64 / 60.0).atan();
    let (d, c) = (0.5, half.cos());
    let radius = |slant: f64| 30.0 - slant * half.sin();
    let low = a * (d + d * d * c / (2.0 * radius(-1.0 + 8.0 - 8.0).max(radius(-1.0))));
    let high = a * (d + d * d * c / (2.0 * radius(16.0)));
    let v = volume_of(&mut s, "OnCone");
    assert!(
        v >= low * (1.0 - 1e-6) && v <= high * (1.0 + 1e-6),
        "{low} ≤ {v} ≤ {high}"
    );
}

#[test]
fn other_surfaces_and_bad_wraps_are_refused_by_name() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    built(
        &mut s,
        "feature.sphere",
        json!({"radius": 20, "name": "Ball"}),
    );
    run(&mut s, "sketch.create", json!({"name": "L"}));
    run(
        &mut s,
        "sketch.text",
        json!({"sketch": "L", "text": "X", "size": 5}),
    );
    let why = failed(
        &mut s,
        "feature.extrude",
        json!({"sketch": "L", "wrap": "F1:face(0)", "distance": 1}),
    );
    assert!(why.contains("not built yet"), "{why}");
    // Too long for the cylinder: past half a turn.
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    built(
        &mut s,
        "feature.cylinder",
        json!({"radius": 2, "height": 10}),
    );
    run(&mut s, "sketch.create", json!({"name": "L"}));
    run(
        &mut s,
        "sketch.text",
        json!({"sketch": "L", "text": "far too long", "size": 3, "at": [0, 2]}),
    );
    let why = failed(
        &mut s,
        "feature.extrude",
        json!({"sketch": "L", "wrap": "F1:face(0)", "distance": 0.2}),
    );
    assert!(why.contains("back of the surface"), "{why}");
    // Parameters that do not go together.
    for params in [
        json!({"sketch": "L", "wrap": "F1:face(0)"}),
        json!({"sketch": "L", "wrap": "F1:face(0)", "distance": 1, "throughAll": true}),
        json!({"sketch": "L", "wrap": "F1:face(0)", "distance": 1, "symmetric": true}),
        json!({"sketch": "L", "wrap": "nonsense", "distance": 1}),
    ] {
        common::checked::error(&mut s, "feature.extrude", params);
    }
}
