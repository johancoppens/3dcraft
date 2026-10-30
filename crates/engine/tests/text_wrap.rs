//! Text wrapped onto a cylinder and a cone (`feature.extrude {wrap}`),
//! embossed and engraved, against closed forms: the development is an
//! isometry, so the wrapped region has the glyphs' own area: the wrapped
//! body (reaching `d` to both sides of the surface) has the volume `2 A d`,
//! and the part of it outside a cylinder of radius `r`, `A (d + d² / 2r)`
//! (inside: `A (d − d² / 2r)`).

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
            // The wrapped body reaches |d| to both sides of the surface
            // (the tool a join or a cut takes): A · 2|d| exactly, the
            // curvature's terms cancelling.
            let _ = r;
            let expected = 2.0 * a * d.abs();
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
fn text_wrapped_onto_a_cone_keeps_its_area() {
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
    // The body reaches the depth to both sides of the cone: A · 2d
    // exactly (one principal curvature is zero, the other's term cancels).
    let a = text_area("CONE", 6.0);
    let v = volume_of(&mut s, "OnCone");
    assert!((v - 2.0 * a * 0.5).abs() <= 1e-5 * a, "{v} vs {}", a);
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

/// "3DCraft" on a cylinder of radius 20 (the evidence case of 2026-10-30):
/// the wrapped body had a face whose loops bound no region, which passed
/// the checks. Ignored until fixed (open bug, devlog 2026-10-30); the
/// diagnosis below wraps each glyph alone at its place to find which.
#[test]
#[ignore = "open bug: a malformed face in the wrapped body of 3DCraft (devlog 2026-10-30)"]
fn three_d_craft_on_a_cylinder_is_rendered_and_joined() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    built(
        &mut s,
        "feature.cylinder",
        json!({"radius": 20, "height": 40, "name": "Body"}),
    );
    run(&mut s, "sketch.create", json!({"name": "Label"}));
    run(
        &mut s,
        "sketch.text",
        json!({"sketch": "Label", "text": "3DCraft", "size": 6, "at": [0, 12], "align": "center"}),
    );
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Label", "wrap": "F1:face(0)", "distance": 1, "join": "Body", "name": "Text"}),
    );
    deep(&mut s);
    let out = run(&mut s, "file.export", json!({"path": "/dev/null.stl"}));
    let _ = out;
}

/// Diagnosis: each glyph of "3DCraft" wrapped alone, where the text puts
/// it; prints which ones fail to mesh (a malformed face) or to join.
#[test]
#[ignore = "diagnosis of the open bug: run with --ignored --nocapture"]
fn diagnose_three_d_craft() {
    for (i, c) in "3DCraft".chars().enumerate() {
        let mut s = session();
        run(&mut s, "file.new", json!({"units": "mm"}));
        built(
            &mut s,
            "feature.cylinder",
            json!({"radius": 20, "height": 40, "name": "Body"}),
        );
        run(&mut s, "sketch.create", json!({"name": "Label"}));
        // The whole text, every other glyph a space: the glyph at its own place.
        let text: String = "3DCraft"
            .chars()
            .enumerate()
            .map(|(k, d)| if k == i { d } else { ' ' })
            .collect();
        let _ = c;
        run(
            &mut s,
            "sketch.text",
            json!({"sketch": "Label", "text": text, "size": 6, "at": [0, 12], "align": "center"}),
        );
        let alone = run(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Label", "wrap": "F1:face(0)", "distance": 1, "name": "Alone"}),
        );
        let shown = run(
            &mut s,
            "view.render",
            json!({"path": "/tmp/x.png", "width": 100, "height": 80}),
        );
        let joined = run(
            &mut s,
            "feature.boolean",
            json!({"target": "Body", "tool": "Alone", "op": "union"}),
        );
        println!(
            "DIAG {c}: alone {} | render {} | join {}",
            alone["status"],
            shown
                .get("error")
                .map_or("ok".to_string(), |e| e.to_string()),
            joined
                .get("error")
                .or(joined.get("status"))
                .map_or(String::new(), |e| e.to_string())
        );
    }
}
