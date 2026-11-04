//! Sweeps along two rails through the commands: a profile on the zx
//! plane, two rails on the xy plane, `feature.sweep` with `rail`, and
//! what the document says of the result.
//!
//! The zx plane's sketch axes are the world's z and x, so a circle about
//! sketch (0, r) of radius r touches the world's origin and (2r, 0, 0):
//! the starts of the rails. Rails from there along +y that close in make
//! a cone; scaled only across, ellipses. Each at three scales against its
//! closed form, checked deep, reopened from a file; the names of its faces
//! held through an edit of a rail.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{body, deep, failure, feature, inspect, refused_with as refused, run};
use craft3d_engine::{Access, Session, Workspace};
use serde_json::json;
use std::f64::consts::PI;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

/// A document whose tolerance fits parts of size `k`, with a circle of
/// radius `r` on the zx plane touching the origin and (2r, 0, 0), and
/// rails from those points along +y to `(x, l)`: rail A ends at
/// `(r - top, l)`, rail B at `(r + top, l)`.
fn horn(k: f64, r: f64, top: f64, l: f64) -> Session {
    let dir = common::workdir("sweep2", &format!("horn-{k}"));
    let mut s = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir),
    });
    run(&mut s, "file.new", json!({"tolerance": 1e-6 * k}));
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "zx", "name": "Section"}),
    );
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Section", "center": [0.0, r], "radius": r}),
    );
    for (name, from, to) in [
        ("RailA", [0.0, 0.0], [r - top, l]),
        ("RailB", [2.0 * r, 0.0], [r + top, l]),
    ] {
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "xy", "name": name}),
        );
        run(
            &mut s,
            "sketch.polyline",
            json!({"sketch": name, "points": [from, to]}),
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

fn cone(l: f64, r0: f64, r1: f64) -> f64 {
    PI * l * (r0 * r0 + r0 * r1 + r1 * r1) / 3.0
}

#[test]
fn a_circle_between_two_rails_is_a_cone_that_follows_an_edit_and_a_file() {
    for k in SCALES {
        let (r, top, l) = (3.0 * k, 1.5 * k, 20.0 * k);
        let t = 1e-6 * k;
        let mut s = horn(k, r, top, l);
        let made = run(
            &mut s,
            "feature.sweep",
            json!({"sketch": "Section", "path": "RailA", "rail": "RailB", "name": "Horn"}),
        );
        assert_eq!(made["status"], "ok", "{made}");
        let deviation = made["path"]["deviation"].as_f64().unwrap();
        assert!(deviation > 0.0 && deviation <= t / 2.0, "{made}");
        let a = area(&mut s, "Horn");
        within(volume(&mut s, "Horn"), cone(l, r, top), a * t);
        deep(&mut s);
        // Four quarters of the circle between the two rails' lines, and
        // the two ends.
        let names = face_names(&mut s, "Horn");
        assert_eq!(names.len(), 6, "{names:?}");
        let sides: Vec<&String> = names.iter().filter(|n| n.contains("side(")).collect();
        assert_eq!(sides.len(), 4, "{names:?}");
        assert!(
            sides.iter().all(|n| n.ends_with("F4:side(e2,e3,e3)")),
            "{names:?}"
        );

        // Edits of the rails: their far ends move in, one after the
        // other. In between the rails differ in length (and the sweep is
        // still built); after both it is the narrower cone. The names of
        // its faces stay throughout.
        let narrower = 0.75 * k;
        run(
            &mut s,
            "sketch.edit",
            json!({"sketch": "RailA", "entity": "e2", "at": [r - narrower, l]}),
        );
        assert_eq!(feature(&mut s, "Horn")["status"], "ok");
        assert_eq!(face_names(&mut s, "Horn"), names);
        run(
            &mut s,
            "sketch.edit",
            json!({"sketch": "RailB", "entity": "e2", "at": [r + narrower, l]}),
        );
        assert_eq!(feature(&mut s, "Horn")["status"], "ok");
        assert_eq!(face_names(&mut s, "Horn"), names);
        let a = area(&mut s, "Horn");
        within(volume(&mut s, "Horn"), cone(l, r, narrower), a * t);
        deep(&mut s);
        for (rail, x) in [("RailA", r - top), ("RailB", r + top)] {
            run(
                &mut s,
                "sketch.edit",
                json!({"sketch": rail, "entity": "e2", "at": [x, l]}),
            );
        }

        // Scaled only across: ellipses of half-axes ρ(y) and r.
        let edited = run(
            &mut s,
            "feature.edit",
            json!({"feature": "Horn", "scale": "width"}),
        );
        assert_eq!(edited["status"], "ok", "{edited}");
        let a = area(&mut s, "Horn");
        within(volume(&mut s, "Horn"), PI * r * l * (r + top) / 2.0, a * t);
        assert_eq!(face_names(&mut s, "Horn"), names);
        deep(&mut s);

        // Saved and reopened: the same bodies.
        let bodies = inspect(&mut s)["bodies"].clone();
        run(&mut s, "file.save", json!({"path": "horn.3dcraft"}));
        run(&mut s, "file.close", json!({}));
        run(&mut s, "file.open", json!({"path": "horn.3dcraft"}));
        assert_eq!(inspect(&mut s)["bodies"], bodies);

        // Without the second rail it is a sweep along rail A, square to it.
        let edited = run(
            &mut s,
            "feature.edit",
            json!({"feature": "Horn", "rail": null}),
        );
        assert_eq!(edited["status"], "failed", "{edited}");
        // A scaling is for two rails only.
        let why = refused(
            &mut s,
            "feature.edit",
            json!({"feature": "Horn", "scale": "width"}),
        );
        assert!(
            why.contains("`scale` is for a sweep along two rails"),
            "{why}"
        );
    }
}

#[test]
fn what_cannot_be_swept_along_two_rails_fails_with_the_parameter_and_the_place() {
    let k = 1.0;
    let (r, top, l) = (3.0, 1.5, 20.0);
    let mut s = horn(k, r, top, l);
    // Rails that cross.
    run(
        &mut s,
        "feature.sweep",
        json!({"sketch": "Section", "path": "RailA", "rail": "RailB", "name": "Horn"}),
    );
    run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "RailB", "entity": "e2", "at": [-4.0, l]}),
    );
    let why = failure(&mut s, "Horn");
    assert!(why.starts_with("rail: "), "{why}");
    assert!(why.contains("meet or cross"), "{why}");
    // Back again: built.
    run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "RailB", "entity": "e2", "at": [r + top, l]}),
    );
    assert_eq!(feature(&mut s, "Horn")["status"], "ok");
    // A rail that does not start on the section.
    run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "RailB", "entity": "e1", "at": [2.0 * r + 1.0, 0.0]}),
    );
    let why = failure(&mut s, "Horn");
    assert!(why.contains("outline of the section"), "{why}");
    run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "RailB", "entity": "e1", "at": [2.0 * r, 0.0]}),
    );
    assert_eq!(feature(&mut s, "Horn")["status"], "ok");

    // What the command refuses outright.
    let why = refused(
        &mut s,
        "feature.sweep",
        json!({"sketch": "Section", "path": "RailA", "scale": "width"}),
    );
    assert!(
        why.contains("`scale` is for a sweep along two rails"),
        "{why}"
    );
    let why = refused(
        &mut s,
        "feature.sweep",
        json!({"sketch": "Section", "path": "RailA", "rail": "RailA"}),
    );
    assert!(why.contains("same sketch"), "{why}");
    let why = refused(
        &mut s,
        "feature.sweep",
        json!({"sketch": "Section", "path": "RailA", "rail": "RailB", "scale": "tall"}),
    );
    assert!(why.contains("\"uniform\" or \"width\""), "{why}");
    for wrong in [json!(7.5), json!([1]), json!({"a": 1}), json!("NoSuch")] {
        let why = refused(
            &mut s,
            "feature.sweep",
            json!({"sketch": "Section", "path": "RailA", "rail": wrong}),
        );
        assert!(!why.is_empty());
    }
}
