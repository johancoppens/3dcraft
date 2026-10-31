//! Helices through the commands: `datum.helix`, and `feature.sweep` with
//! a helix as its path.
//!
//! The closed forms: a helix is `turns · √((2πr)² + p²)` long; a profile
//! carried by its screw motion sweeps per radian its area times the speed
//! at which the motion crosses its plane at the profile's centre of area
//! (that speed is affine in the point). In a plane through the axis that
//! is Pappus, `A · 2π x̄ · turns`; square to the helix with its centre on
//! it, `A` times the helix's length (`crates/kernel/tests/helix.rs` has
//! the derivation and the same forms against the kernel alone).
//!
//! The profiles are drawn on the zx plane: its sketch axes are the
//! world's z and x, so a point at sketch (u, v) is at height u and v from
//! the z axis, on the side of +x, where a helix about z given by its
//! radius starts.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;

use common::checked::{
    deep, failure, feature_brief as feature, inspect, refused_with as refused, run,
};
use craft3d_engine::{Access, Session, Workspace};
use serde_json::{Value, json};
use std::f64::consts::{PI, TAU};
use std::path::PathBuf;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn workdir(name: &str) -> PathBuf {
    common::workdir("helix", name)
}

/// A new document whose tolerance follows the size at 1e6.
fn new(k: f64) -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"tolerance": 1e-6 * k.max(1.0)}));
    s
}

fn body(s: &mut Session, name: &str) -> Value {
    let document = inspect(s);
    document["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == name)
        .unwrap_or_else(|| panic!("no body {name}: {}", document["features"]))
        .clone()
}

fn volume(s: &mut Session, name: &str) -> f64 {
    body(s, name)["volume"].as_f64().unwrap()
}

#[track_caller]
fn close(found: f64, expected: f64, relative: f64) {
    assert!(
        (found - expected).abs() <= relative * expected.abs(),
        "found {found}, expected {expected} (off by {:e})",
        (found - expected).abs() / expected.abs()
    );
}

/// A sketch `name` on the zx plane with a circle at height `u`, `v` from
/// the z axis: its centre is e1, the circle e2.
fn wire(s: &mut Session, name: &str, u: f64, v: f64, radius: f64) {
    run(s, "sketch.create", json!({"plane": "zx", "name": name}));
    run(
        s,
        "sketch.circle",
        json!({"sketch": name, "center": [u, v], "radius": radius}),
    );
}

fn helix_length(radius: f64, pitch: f64, turns: f64) -> f64 {
    turns * (TAU * radius).phypot(pitch)
}

#[test]
fn a_helix_says_where_it_lies_and_follows_its_parameters() {
    for k in SCALES {
        let mut s = new(k);
        let made = run(
            &mut s,
            "datum.helix",
            json!({"axis": "z", "radius": 10.0 * k, "pitch": 4.0 * k, "turns": 2.5, "name": "Coil"}),
        );
        assert_eq!(made["status"], "ok", "{made}");
        let datum = &made["datum"];
        assert_eq!(datum["kind"], "helix");
        close(
            datum["length"].as_f64().unwrap(),
            helix_length(10.0 * k, 4.0 * k, 2.5),
            1e-14,
        );
        close(datum["height"].as_f64().unwrap(), 10.0 * k, 1e-15);
        assert_eq!(datum["start"], json!([10.0 * k, 0.0, 0.0]));
        // Two and a half turns: half a turn round from the start, ten up.
        let end: Vec<f64> = datum["end"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect();
        assert!((end[0] + 10.0 * k).abs() < 1e-12 * k, "{end:?}");
        assert!(end[1].abs() < 1e-12 * k, "{end:?}");
        close(end[2], 10.0 * k, 1e-14);
        assert_eq!(datum["leftHanded"], false);
        // Edited: the pitch and the turns, then a height instead.
        let edited = run(
            &mut s,
            "feature.edit",
            json!({"feature": "Coil", "pitch": 2.0 * k, "turns": "4"}),
        );
        assert_eq!(edited["status"], "ok", "{edited}");
        let datum = run(&mut s, "datum.helix", json!({"replace": "Coil", "axis": "z", "radius": 10.0 * k, "pitch": 2.0 * k, "height": 6.0 * k}))["datum"].clone();
        close(datum["turns"].as_f64().unwrap(), 3.0, 1e-15);
        run(
            &mut s,
            "feature.edit",
            json!({"feature": "Coil", "turns": 1, "leftHanded": true}),
        );
        let found = &feature(&mut s, "Coil");
        assert_eq!(found["status"], "ok", "{found}");
        // Undone, the helix is what it was.
        run(&mut s, "edit.undo", json!({}));
        let again = run(
            &mut s,
            "feature.edit",
            json!({"feature": "Coil", "radius": 5.0 * k}),
        );
        assert_eq!(again["status"], "ok", "{again}");
    }
}

#[test]
fn a_helix_through_a_point_about_a_datum_axis_starts_there() {
    let mut s = new(1.0);
    run(
        &mut s,
        "datum.axis",
        json!({"through": [[1, 2, 0], [1, 2, 1]], "name": "Upright"}),
    );
    let made = run(
        &mut s,
        "datum.helix",
        json!({"axis": "Upright", "start": [4, 6, 3], "pitch": "1.5 mm", "height": "12 mm", "leftHanded": true}),
    );
    let datum = &made["datum"];
    assert_eq!(made["status"], "ok", "{made}");
    close(datum["radius"].as_f64().unwrap(), 5.0, 1e-15);
    close(datum["turns"].as_f64().unwrap(), 8.0, 1e-15);
    assert_eq!(datum["origin"], json!([1.0, 2.0, 3.0]));
    assert_eq!(datum["start"], json!([4.0, 6.0, 3.0]));
    assert_eq!(datum["leftHanded"], true);
}

#[test]
fn bad_helices_are_refused_or_fail_with_the_parameter() {
    let mut s = new(1.0);
    for (params, says) in [
        (json!({"radius": 1, "pitch": 1, "turns": 1}), "axis"),
        (json!({"axis": "z", "pitch": 1, "turns": 1}), "radius"),
        (
            json!({"axis": "z", "radius": 1, "start": [1, 0, 0], "pitch": 1, "turns": 1}),
            "either",
        ),
        (json!({"axis": "z", "radius": 1, "turns": 1}), "pitch"),
        (json!({"axis": "z", "radius": 1, "pitch": 1}), "turns"),
        (
            json!({"axis": "z", "radius": 1, "pitch": 1, "turns": 1, "height": 3}),
            "either",
        ),
        (
            json!({"axis": "w", "radius": 1, "pitch": 1, "turns": 1}),
            "axis",
        ),
    ] {
        let error = refused(&mut s, "datum.helix", params.clone());
        assert!(error.contains(says), "{params}: {error}");
    }
    for (name, params, says) in [
        (
            "Flat",
            json!({"radius": 0, "pitch": 1, "turns": 1}),
            "radius",
        ),
        (
            "Still",
            json!({"radius": 1, "pitch": 0, "turns": 1}),
            "pitch",
        ),
        (
            "None",
            json!({"radius": 1, "pitch": 1, "turns": -2}),
            "turns",
        ),
        (
            "Low",
            json!({"radius": 1, "pitch": 1, "height": "0 mm"}),
            "height",
        ),
        (
            "Odd",
            json!({"radius": "1 deg", "pitch": 1, "turns": 1}),
            "radius",
        ),
        (
            "Onaxis",
            json!({"start": [0, 0, 5], "pitch": 1, "turns": 1}),
            "axis",
        ),
    ] {
        let mut params = params;
        params["axis"] = json!("z");
        params["name"] = json!(name);
        run(&mut s, "datum.helix", params);
        let error = failure(&mut s, name);
        assert!(error.contains(says), "{name}: {error}");
    }
}

#[test]
fn a_ring_wire_along_a_helix_follows_pappus_at_every_scale() {
    for k in SCALES {
        let mut s = new(k);
        let (r, p, turns, a) = (10.0 * k, 4.0 * k, 2.5, 1.0 * k);
        run(
            &mut s,
            "datum.helix",
            json!({"axis": "z", "radius": r, "pitch": p, "turns": turns, "name": "Coil"}),
        );
        wire(&mut s, "Wire", 0.0, r, a);
        let made = run(
            &mut s,
            "feature.sweep",
            json!({"sketch": "Wire", "path": "Coil", "name": "Spring"}),
        );
        assert_eq!(made["status"], "ok", "{made}");
        let path = &made["path"];
        close(
            path["length"].as_f64().unwrap(),
            helix_length(r, p, turns),
            1e-14,
        );
        assert!(path["deviation"].as_f64().unwrap() <= 0.5e-6 * k.max(1.0));
        deep(&mut s);
        close(volume(&mut s, "Spring"), PI * a * a * TAU * r * turns, 2e-6);
        let names = body(&mut s, "Spring")["faceNames"].clone();
        let names: Vec<&str> = names
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_str().unwrap())
            .collect();
        assert_eq!(names.len(), 22, "{names:?}");
        assert!(names.contains(&"F3:side(e2)"), "{names:?}");
        assert!(names.contains(&"F3:piece(19)/F3:side(e2)"), "{names:?}");
        assert!(names.contains(&"F3:startCap(e2)"), "{names:?}");
        assert!(names.contains(&"F3:endCap(e2)"), "{names:?}");
    }
}

#[test]
fn a_coil_spring_square_to_its_helix_is_its_wire_times_the_helix_length() {
    let mut s = new(1.0);
    let (r, p, turns, a): (f64, f64, f64, f64) = (10.0, 4.0, 3.0, 1.0);
    run(
        &mut s,
        "datum.helix",
        json!({"axis": "z", "radius": r, "pitch": p, "turns": turns, "name": "Coil"}),
    );
    // Square to the helix at its start: through the start, x away from
    // the axis, the normal along (0, r, p / 2π).
    let k = p / TAU;
    let n = r.phypot(k);
    run(
        &mut s,
        "datum.plane",
        json!({"through": [[r, 0, 0], [r + 1.0, 0, 0], [r, k / n, -r / n]], "name": "Across"}),
    );
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "Across", "name": "Wire"}),
    );
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Wire", "center": [0, 0], "radius": a}),
    );
    let made = run(
        &mut s,
        "feature.sweep",
        json!({"sketch": "Wire", "path": "Coil", "name": "Spring"}),
    );
    assert_eq!(made["status"], "ok", "{made}");
    deep(&mut s);
    close(
        volume(&mut s, "Spring"),
        PI * a * a * helix_length(r, p, turns),
        2e-6,
    );
}

#[test]
fn a_spring_follows_edits_survives_a_file_and_keeps_the_set_identities() {
    let dir = workdir("file");
    let mut s = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    run(&mut s, "file.new", json!({}));
    run(
        &mut s,
        "variable.set",
        json!({"name": "pitch", "expr": "4 mm"}),
    );
    run(
        &mut s,
        "datum.helix",
        json!({"axis": "z", "radius": 10, "pitch": "pitch", "turns": 2, "name": "Coil"}),
    );
    wire(&mut s, "Wire", 0.0, 10.0, 1.0);
    run(
        &mut s,
        "feature.sweep",
        json!({"sketch": "Wire", "path": "Coil", "name": "Spring"}),
    );
    let ring = PI * TAU * 10.0;
    close(volume(&mut s, "Spring"), ring * 2.0, 2e-6);
    // The pitch follows its variable: a pitch is no volume of its own.
    run(
        &mut s,
        "variable.set",
        json!({"name": "pitch", "expr": "3 mm"}),
    );
    close(volume(&mut s, "Spring"), ring * 2.0, 2e-6);
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Coil", "turns": 3}),
    );
    close(volume(&mut s, "Spring"), ring * 3.0, 2e-6);
    run(&mut s, "edit.undo", json!({}));
    close(volume(&mut s, "Spring"), ring * 2.0, 2e-6);
    run(&mut s, "edit.redo", json!({}));
    close(volume(&mut s, "Spring"), ring * 3.0, 2e-6);
    // Too short a pitch for the wire: one turn runs into the next.
    run(
        &mut s,
        "variable.set",
        json!({"name": "pitch", "expr": "1.5 mm"}),
    );
    let error = failure(&mut s, "Spring");
    assert!(error.contains("pitch") && error.contains("Coil"), "{error}");
    run(
        &mut s,
        "variable.set",
        json!({"name": "pitch", "expr": "3 mm"}),
    );
    let before = inspect(&mut s)["bodies"].clone();
    run(&mut s, "file.save", json!({"path": "spring.3dcraft"}));
    run(&mut s, "file.close", json!({}));
    run(&mut s, "file.open", json!({"path": "spring.3dcraft"}));
    assert_eq!(inspect(&mut s)["bodies"], before);

    // With a block through two coils: |A ∪ B| + |A ∩ B| = |A| + |B| and
    // |A − B| = |A| − |A ∩ B|.
    let spring = volume(&mut s, "Spring");
    let other = 6.0 * 30.0 * 4.0;
    let mut volumes = Vec::new();
    for op in ["union", "intersect", "subtract"] {
        run(&mut s, "file.close", json!({"discard": true}));
        run(&mut s, "file.open", json!({"path": "spring.3dcraft"}));
        run(
            &mut s,
            "feature.block",
            json!({"position": [7, -15, 1], "size": [6, 30, 4], "name": "Tool"}),
        );
        let made = run(
            &mut s,
            "feature.boolean",
            json!({"op": op, "target": "Spring", "tool": "Tool", "name": "Result"}),
        );
        assert_eq!(made["status"], "ok", "{op}: {made}");
        deep(&mut s);
        volumes.push(volume(&mut s, "Result"));
    }
    // The boolean meets the spring's B-spline faces in curves within the
    // tolerance (1e-6) of the true ones, so each volume can be off by the
    // tolerance times the area of the cut, which is less than 100.
    let (union, common, difference) = (volumes[0], volumes[1], volumes[2]);
    let slack = 1e-6 * 100.0;
    assert!(
        (union + common - spring - other).abs() < slack,
        "{volumes:?} {spring}"
    );
    assert!(
        (difference - spring + common).abs() < slack,
        "{volumes:?} {spring}"
    );
    assert!(common > 0.0 && common < spring.min(other));
}

#[test]
fn what_cannot_be_swept_along_a_helix_fails_with_the_reason() {
    let mut s = new(1.0);
    run(
        &mut s,
        "datum.helix",
        json!({"axis": "z", "radius": 5, "pitch": 4, "turns": 2, "name": "Coil"}),
    );
    // Round the axis: a circle about it.
    wire(&mut s, "Around", 0.0, 0.0, 6.0);
    run(
        &mut s,
        "feature.sweep",
        json!({"sketch": "Around", "path": "Coil", "name": "A"}),
    );
    let error = failure(&mut s, "A");
    assert!(
        error.starts_with("path: helix `Coil`") && error.contains("axis"),
        "{error}"
    );
    // Not on the plane the helix starts on.
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "name": "Flat", "offset": 1}),
    );
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Flat", "center": [5, 0], "radius": 1}),
    );
    run(
        &mut s,
        "feature.sweep",
        json!({"sketch": "Flat", "path": "Coil", "name": "B"}),
    );
    let error = failure(&mut s, "B");
    assert!(error.contains("starts"), "{error}");
    // A block is no path.
    run(
        &mut s,
        "feature.block",
        json!({"size": [1, 1, 1], "name": "Box"}),
    );
    let error = refused(
        &mut s,
        "feature.sweep",
        json!({"sketch": "Around", "path": "Box"}),
    );
    assert!(error.contains("sketch"), "{error}");
    // A helix is no datum axis.
    let error = refused(
        &mut s,
        "feature.revolve",
        json!({"sketch": "Around", "axis": "Coil"}),
    );
    assert!(
        error.contains("datumHelix") || error.contains("helix"),
        "{error}"
    );
}
