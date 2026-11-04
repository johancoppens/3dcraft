//! Sweeps along two rails that close into rings, and with the up
//! direction given, through the commands.
//!
//! A circle on the zx plane (sketch axes: the world's z and x) about
//! sketch (0, R) of radius m touches the world's x axis at R − m and
//! R + m; two circles on the xy plane of those radii pass through it
//! there. `feature.sweep` with `rail` carries it round them into a torus:
//! Pappus, `2π R · π m²` and `4π² R m`. Leaning the up direction by `α`
//! from z leans the section back and forth round the ring; with the
//! section's centroid on the chord the volume is
//! `π m² R cos α · 4 K(sin α)` (kernel test `sweep2_ring.rs` derives it),
//! `K` by the arithmetic–geometric mean. Each at three scales, checked
//! deep, its names held through edits, reopened from a file, and cut by
//! a box in half.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{body, deep, failure, feature, inspect, refused_with as refused, run};
use craft3d_engine::{Access, Session, Workspace};
use craft3d_math::Portable;
use serde_json::json;
use std::f64::consts::PI;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

/// Rings of radii `R − m` and `R + m` about the origin on the xy plane
/// and the section between them, all driven by the variables `major` and `minor`.
fn rings(k: f64, major: f64, minor: f64) -> Session {
    let dir = common::workdir("sweep2-ring", &format!("ring-{k}"));
    let mut s = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir),
    });
    run(
        &mut s,
        "file.new",
        json!({"units": "mm", "tolerance": 1e-6 * k}),
    );
    run(
        &mut s,
        "variable.set",
        json!({"name": "major", "expr": format!("{major} mm")}),
    );
    run(
        &mut s,
        "variable.set",
        json!({"name": "minor", "expr": format!("{minor} mm")}),
    );
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "zx", "name": "Section"}),
    );
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Section", "center": [0.0, major], "radius": "minor"}),
    );
    for (name, radius) in [("Inner", "major - minor"), ("Outer", "major + minor")] {
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "xy", "name": name}),
        );
        run(
            &mut s,
            "sketch.circle",
            json!({"sketch": name, "center": [0.0, 0.0], "radius": radius}),
        );
    }
    s
}

fn volume(s: &mut Session, name: &str) -> f64 {
    body(s, name)["volume"].as_f64().unwrap()
}

fn area(s: &mut Session, name: &str) -> f64 {
    body(s, name)["area"].as_f64().unwrap()
}

fn face_names(s: &mut Session, name: &str) -> Vec<String> {
    let mut names: Vec<String> = body(s, name)["faceNames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap().to_string())
        .collect();
    names.sort();
    names
}

#[track_caller]
fn within(found: f64, expected: f64, allowed: f64) {
    assert!(
        (found - expected).abs() <= allowed,
        "found {found}, expected {expected} (off by {:e}, {allowed:e} allowed)",
        (found - expected).abs()
    );
}

fn torus(major: f64, minor: f64) -> f64 {
    2.0 * PI * major * PI * minor * minor
}

#[test]
fn a_circle_round_two_rings_is_a_torus_through_edits_a_file_and_a_box() {
    for k in SCALES {
        let (major, minor) = (10.0 * k, 2.0 * k);
        let t = 1e-6 * k;
        let mut s = rings(k, major, minor);
        // An axis leaning by α from z in the zx plane, for later.
        let alpha: f64 = 0.5;
        let (sin, cos) = alpha.psin_cos();
        run(
            &mut s,
            "datum.axis",
            json!({"through": [[0.0, 0.0, 0.0], [sin * k, 0.0, cos * k]], "name": "Lean"}),
        );
        let made = run(
            &mut s,
            "feature.sweep",
            json!({"sketch": "Section", "path": "Inner", "rail": "Outer", "name": "Ring"}),
        );
        assert_eq!(made["status"], "ok", "{made}");
        assert_eq!(made["path"]["closed"], true, "{made}");
        let deviation = made["path"]["deviation"].as_f64().unwrap();
        assert!(deviation <= t / 2.0, "{made}");
        let a = area(&mut s, "Ring");
        within(volume(&mut s, "Ring"), torus(major, minor), a * t);
        within(a, 4.0 * PI * PI * major * minor, a * 1e-6);
        deep(&mut s);
        // No ends: two half turns of each ring, four quarters of the
        // section each.
        let names = face_names(&mut s, "Ring");
        assert_eq!(names.len(), 8, "{names:?}");
        assert!(names.iter().all(|n| n.contains("side(")), "{names:?}");

        // A fatter section and rings that follow it: the same names.
        let fatter = 3.0 * k;
        run(
            &mut s,
            "variable.set",
            json!({"name": "minor", "expr": format!("{fatter} mm")}),
        );
        assert_eq!(feature(&mut s, "Ring")["status"], "ok");
        assert_eq!(face_names(&mut s, "Ring"), names);
        let a = area(&mut s, "Ring");
        within(volume(&mut s, "Ring"), torus(major, fatter), a * t);
        deep(&mut s);

        // Up along z is the default: the same body. Up leaning by α in
        // the zx plane (a datum axis): the elliptic integral's volume,
        // the same names.
        let before = body(&mut s, "Ring");
        let edited = run(
            &mut s,
            "feature.edit",
            json!({"feature": "Ring", "up": "z"}),
        );
        assert_eq!(edited["status"], "ok", "{edited}");
        assert_eq!(body(&mut s, "Ring")["volume"], before["volume"]);
        let edited = run(
            &mut s,
            "feature.edit",
            json!({"feature": "Ring", "up": "Lean"}),
        );
        assert_eq!(edited["status"], "ok", "{edited}");
        let (mut am, mut gm) = (1.0_f64, cos);
        for _ in 0..40 {
            (am, gm) = (0.5 * (am + gm), (am * gm).sqrt());
        }
        let k_of = PI / (2.0 * am);
        let a = area(&mut s, "Ring");
        within(
            volume(&mut s, "Ring"),
            PI * fatter * fatter * major * cos * 4.0 * k_of,
            a * t,
        );
        assert_eq!(face_names(&mut s, "Ring"), names);
        deep(&mut s);

        // Saved and reopened: the same bodies.
        let bodies = inspect(&mut s)["bodies"].clone();
        run(&mut s, "file.save", json!({"path": "ring.3dcraft"}));
        run(&mut s, "file.close", json!({}));
        run(&mut s, "file.open", json!({"path": "ring.3dcraft"}));
        assert_eq!(inspect(&mut s)["bodies"], bodies);

        // Back to the default up: the torus again.
        let edited = run(
            &mut s,
            "feature.edit",
            json!({"feature": "Ring", "up": null}),
        );
        assert_eq!(edited["status"], "ok", "{edited}");
        let a = area(&mut s, "Ring");
        within(volume(&mut s, "Ring"), torus(major, fatter), a * t);

        // A box takes the half x < 0 away: half the torus.
        let reach = 2.0 * (major + fatter);
        run(
            &mut s,
            "feature.block",
            json!({
                "size": [reach, 2.0 * reach, 4.0 * fatter],
                "position": [-reach, -reach, -2.0 * fatter],
                "name": "Box"
            }),
        );
        let cut = run(
            &mut s,
            "feature.boolean",
            json!({"op": "subtract", "target": "Ring", "tool": "Box", "name": "Half"}),
        );
        assert_eq!(cut["status"], "ok", "{cut}");
        let half = body(&mut s, "Half");
        assert_eq!(half["valid"], true, "{half}");
        let a = half["area"].as_f64().unwrap();
        within(
            half["volume"].as_f64().unwrap(),
            torus(major, fatter) / 2.0,
            a * t,
        );
        deep(&mut s);
    }
}

#[test]
fn what_cannot_be_swept_round_two_rings_fails_with_the_parameter_and_the_reason() {
    let (major, minor) = (10.0, 2.0);
    let mut s = rings(1.0, major, minor);
    run(
        &mut s,
        "feature.sweep",
        json!({"sketch": "Section", "path": "Inner", "rail": "Outer", "name": "Ring"}),
    );
    assert_eq!(feature(&mut s, "Ring")["status"], "ok");
    // Up along the chord where the section is.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Ring", "up": "x"}),
    );
    let why = failure(&mut s, "Ring");
    assert!(why.starts_with("rail: "), "{why}");
    assert!(why.contains("up direction"), "{why}");
    // Up along the rings where the section is: it would lie in their
    // course.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Ring", "up": "y"}),
    );
    let why = failure(&mut s, "Ring");
    assert!(why.contains("up direction"), "{why}");
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Ring", "up": null}),
    );
    assert_eq!(feature(&mut s, "Ring")["status"], "ok");
    // A ring that misses the section.
    run(
        &mut s,
        "variable.set",
        json!({"name": "major", "expr": "11 mm"}),
    );
    let why = failure(&mut s, "Ring");
    assert!(why.contains("outline"), "{why}");
    run(
        &mut s,
        "variable.set",
        json!({"name": "major", "expr": "10 mm"}),
    );
    assert_eq!(feature(&mut s, "Ring")["status"], "ok");
    // A ring and an open rail.
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "name": "Open"}),
    );
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Open", "points": [[12.0, 0.0], [12.0, 10.0]]}),
    );
    let made = run(
        &mut s,
        "feature.sweep",
        json!({"sketch": "Section", "path": "Inner", "rail": "Open", "name": "Mixed"}),
    );
    assert_eq!(made["status"], "failed", "{made}");
    assert!(
        made["error"].as_str().unwrap().contains("both close"),
        "{made}"
    );

    // What the command refuses outright.
    let why = refused(
        &mut s,
        "feature.sweep",
        json!({"sketch": "Section", "path": "Inner", "up": "z"}),
    );
    assert!(why.contains("`up` is for a sweep along two rails"), "{why}");
    for wrong in [json!(7.5), json!([1]), json!({"a": 1}), json!("NoSuch")] {
        let why = refused(
            &mut s,
            "feature.sweep",
            json!({"sketch": "Section", "path": "Inner", "rail": "Outer", "up": wrong}),
        );
        assert!(why.contains("`up`"), "{why}");
    }
}
