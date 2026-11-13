//! A sketch whose solve from its stored solution fails is not stuck there:
//! the solver tries once more from the circles its held points determine,
//! so a document saved after an edit that failed opens and builds as a new
//! document at its values does (docs/design/document.md › *A document
//! stuck at its last solution*).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use craft3d_engine::Session;
use craft3d_math::Portable;
use serde_json::{Value, json};
use std::path::PathBuf;

fn steps(s: &mut Session, steps: &Value) {
    for step in steps.as_array().unwrap() {
        run(s, step[0].as_str().unwrap(), step[1].clone());
    }
}

/// The points of a sketch as `sketch.inspect` gives them, by id, and its
/// status.
fn points(s: &mut Session, sketch: &str) -> (String, Vec<(String, f64, f64)>) {
    let inspected = run(s, "sketch.inspect", json!({"sketch": sketch}));
    let list = inspected["entities"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["type"] == "point")
        .map(|e| {
            (
                e["id"].as_str().unwrap().to_string(),
                e["x"].as_f64().unwrap(),
                e["y"].as_f64().unwrap(),
            )
        })
        .collect();
    (inspected["status"].as_str().unwrap().to_string(), list)
}

fn point(points: &[(String, f64, f64)], id: &str) -> [f64; 2] {
    let (_, x, y) = points.iter().find(|(p, ..)| p == id).unwrap();
    [*x, *y]
}

fn volume(s: &mut Session, name: &str) -> f64 {
    let doc = run(s, "document.inspect", json!({}));
    let body = doc["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == name)
        .unwrap_or_else(|| panic!("no body {name}: {doc}"));
    body["volume"].as_f64().unwrap()
}

/// The centre of the circle through three points, by Cramer's rule.
fn circumcentre(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> [f64; 2] {
    let d = 2.0 * (a[0] * (b[1] - c[1]) + b[0] * (c[1] - a[1]) + c[0] * (a[1] - b[1]));
    let (a2, b2, c2) = (
        a[0] * a[0] + a[1] * a[1],
        b[0] * b[0] + b[1] * b[1],
        c[0] * c[0] + c[1] * c[1],
    );
    [
        (a2 * (b[1] - c[1]) + b2 * (c[1] - a[1]) + c2 * (a[1] - b[1])) / d,
        (a2 * (c[0] - b[0]) + b2 * (a[0] - c[0]) + c2 * (b[0] - a[0])) / d,
    ]
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/stale_sketches")
}

/// The ids of the cap's and the roll's [centre, start, end, through], as
/// the path of `two-arcs-moved.steps.json` makes them.
const ARCS: [[&str; 4]; 2] = [["e3", "e1", "e2", "e5"], ["e9", "e6", "e8", "e11"]];

/// `stale_sketches/two-arcs-moved.3dcraft` is a document saved by 3DCraft
/// 0.1.0 after `two-arcs-moved.steps.json`: a profile of two arcs drawn
/// through points (a shallow cap and a half-circle roll, every point held
/// by expressions of `t`), built at t = 0 and then given t = 1, which moves
/// most points by more than the arcs' radii, the roll along its own chord.
/// That edit failed ("constraint c7 conflicts"), a failed sketch keeps its
/// last solution, and the file holds the solution at t = 0 with t = 1: it
/// opened failing, and every solve started from there and failed again,
/// while a new document at t = 1 builds. Opened now, it builds: the centres
/// are the circumcentres of the held points, the body is the new
/// document's, and the solution found is stored, so a save and reopen
/// gives the same, bit for bit.
#[test]
fn a_document_saved_stuck_at_its_last_solution_opens_and_builds() {
    let dir = common::workdir("sketch-resolve", "stale");
    std::fs::copy(
        fixtures().join("two-arcs-moved.3dcraft"),
        dir.join("stale.3dcraft"),
    )
    .unwrap();
    // The fixture is stale, or it tests nothing: t is 1, and the sketch
    // holds the solution at t = 0 (the cap's centre e3 where it was then,
    // the roll's end e8 at y = 312.33 + 182), as 0.1.0 saved it.
    let saved = craft3d_format::load(&dir.join("stale.3dcraft")).unwrap();
    let t = saved.variables.iter().find(|v| v.name == "t").unwrap();
    assert_eq!(t.expr.source(), "1");
    let feature = saved.feature_by_name("Profile").unwrap();
    let craft3d_doc::FeatureKind::Sketch(stored) = &feature.kind else {
        panic!("not a sketch");
    };
    let id = |n: u64| craft3d_doc::sketch::EntityId(n);
    assert_eq!(
        stored.point(id(3)).unwrap(),
        (193.65546450939456, 311.74756222827557)
    );
    assert_eq!(stored.point(id(8)).unwrap(), (5.0, 494.3259849173512));

    let mut opened = common::session_in(&dir);
    run(&mut opened, "file.open", json!({"path": "stale.3dcraft"}));
    let (status, here) = points(&mut opened, "Profile");
    assert_eq!(status, "ok");
    // The held points where t = 1 puts them: z = 267.778..., the corners
    // (c0 - h, z) = (15.521, z), (c0, z + dcr) = (25.16, z + 21.42),
    // (5, z + 63), (5, z + 70), (40, z + 70), (40, z), the through points
    // (c0 - 0.75 h, z + dcr / 2) = (17.93075, z + 10.71), (1.5, z + 66.5).
    let z = 267.77805096117135;
    let corners = [
        ("e1", [15.521, z]),
        ("e2", [25.16, z + 21.42]),
        ("e6", [5.0, z + 63.0]),
        ("e8", [5.0, z + 70.0]),
        ("e12", [40.0, z + 70.0]),
        ("e14", [40.0, z]),
    ];
    for (id, at) in corners
        .iter()
        .chain(&[("e5", [17.93075, z + 10.71]), ("e11", [1.5, z + 66.5])])
    {
        let [x, y] = point(&here, id);
        assert!(
            (x - at[0]).abs() <= 1e-9 && (y - at[1]).abs() <= 1e-9,
            "{id} at ({x}, {y}), held at {at:?}"
        );
    }
    // The volume in closed form: 5 times the area of the polygon of the
    // corners (run clockwise: up the left, along the top, down the right,
    // so its inside is on the right of each side) with the cap's segment
    // (its through point is left of the side from e1 to e2: outside) and
    // the roll's half disc of radius 3.5 (its point at x = 1.5 < 5:
    // outside).
    let polygon: f64 = corners
        .iter()
        .zip(corners.iter().cycle().skip(1))
        .map(|((_, a), (_, b))| a[0] * b[1] - b[0] * a[1])
        .sum::<f64>()
        .abs()
        / 2.0;
    let segment = |a: [f64; 2], b: [f64; 2], p: [f64; 2]| {
        let c = circumcentre(a, b, p);
        let r = (a[0] - c[0]).phypot(a[1] - c[1]);
        let chord = (a[0] - b[0]).phypot(a[1] - b[1]);
        let theta = 2.0 * (chord / (2.0 * r)).pasin();
        r * r / 2.0 * (theta - theta.psin())
    };
    let cap = segment([15.521, z], [25.16, z + 21.42], [17.93075, z + 10.71]);
    let roll = std::f64::consts::PI * 3.5 * 3.5 / 2.0;
    let expected = 5.0 * (polygon + cap + roll);
    for [c, a, b, p] in ARCS {
        let [cx, cy] = point(&here, c);
        let [ex, ey] = circumcentre(point(&here, a), point(&here, b), point(&here, p));
        assert!(
            (cx - ex).abs() <= 1e-9 && (cy - ey).abs() <= 1e-9,
            "{c} at ({cx}, {cy}), the circumcentre is ({ex}, {ey})"
        );
    }
    let check = run(&mut opened, "document.check", json!({}));
    assert_eq!(check["ok"], true, "{check}");

    // A new document at t = 1, from the same steps.
    let recipe: Value = serde_json::from_str(
        &std::fs::read_to_string(fixtures().join("two-arcs-moved.steps.json")).unwrap(),
    )
    .unwrap();
    let mut fresh = common::session();
    run(&mut fresh, "file.new", json!({"units": "mm"}));
    let mut at_one = recipe.as_array().unwrap().clone();
    at_one[0] = json!(["variable.set", {"name": "t", "expr": "1"}]);
    steps(&mut fresh, &Value::Array(at_one));
    let (_, there) = points(&mut fresh, "Profile");
    for ((id, x, y), (_, fx, fy)) in here.iter().zip(&there) {
        assert!(
            (x - fx).abs() <= 1e-9 && (y - fy).abs() <= 1e-9,
            "{id}: ({x}, {y}) against a new document's ({fx}, {fy})"
        );
    }
    let (v, fv) = (volume(&mut opened, "Plate"), volume(&mut fresh, "Plate"));
    assert!((v - fv).abs() <= 1e-9 * fv, "volume {v}, new document {fv}");
    assert!(
        (v - expected).abs() <= 1e-9 * expected,
        "volume {v}, closed form {expected}"
    );

    run(
        &mut opened,
        "file.save",
        json!({"path": "saved.3dcraft", "overwrite": true}),
    );
    let mut again = common::session_in(&dir);
    run(&mut again, "file.open", json!({"path": "saved.3dcraft"}));
    assert_eq!(points(&mut again, "Profile").1, here);
    assert_eq!(volume(&mut again, "Plate").to_bits(), v.to_bits());
}

/// The area of the region between a chord of length `c` and an arc of
/// sagitta `b` on it (b > 0): r^2 / 2 (theta - sin theta), theta the angle
/// the chord subtends, by its series where theta is small.
fn segment(c: f64, b: f64) -> f64 {
    let r = (b * b + c * c / 4.0) / (2.0 * b);
    let theta = 2.0 * (c / (2.0 * r)).pasin();
    let cube = theta * theta * theta;
    let less_sin = if theta < 1e-3 {
        cube / 6.0 - cube * theta * theta / 120.0
    } else {
        theta - theta.psin()
    };
    r * r / 2.0 * less_sin
}

/// An arc from (0, 0) to (40 k, 0) through (20 k, bulge) closing a plate
/// 20 k deep below the chord, at three scales (the tolerance the part's
/// own): flattened from a bulge of 6 k to 0.0001 k (the centre two million
/// half-chords away), back to 6 k, across the chord to -3 k and back. The
/// flattening failed with a conflict before; now it builds, with the
/// centre at the closed form (20 k, (b^2 - 400 k^2) / (2 b)) and the volume
/// 5 k (800 k^2 + the segment), as every step with the bulge positive does.
/// Across the chord the one solution of the arc's constraints keeps the
/// way round it was drawn with and runs the long way round its centre,
/// away from its point (a body many times the plate's): it is not taken, and
/// the edit fails as in 0.1.1, with no body; the next edit builds again.
#[test]
fn an_arc_through_a_point_goes_flat_and_across_by_edits_at_three_scales() {
    for k in [1e-3, 1.0, 1e6] {
        let mut s = common::session();
        run(
            &mut s,
            "file.new",
            json!({"units": "mm", "tolerance": 1e-6 * k}),
        );
        let mm = |v: f64| format!("{} mm", v * k);
        steps(
            &mut s,
            &json!([
                ["variable.set", {"name": "span", "expr": mm(40.0)}],
                ["variable.set", {"name": "bulge", "expr": mm(6.0)}],
                ["sketch.create", {"name": "Arc", "plane": "xy"}],
                ["sketch.path", {"sketch": "Arc", "start": ["0 mm", "0 mm"], "segments": [
                    {"arc": ["span", "0 mm"], "through": ["span/2", "bulge"]},
                    {"line": ["span", mm(-20.0)]},
                    {"line": ["0 mm", mm(-20.0)]},
                    {"line": "start"}
                ]}],
                ["feature.extrude", {"sketch": "Arc", "distance": mm(5.0), "name": "Plate"}]
            ]),
        );
        let expected = |b: f64| 5.0 * k * (800.0 * k * k + segment(40.0 * k, b));
        for bulge in [1e-4_f64, 6.0, -3.0, 6.0, 1e-4, -3.0, 6.0] {
            let b = bulge * k;
            let what = format!("scale {k}, bulge {b}");
            let result = run(
                &mut s,
                "variable.set",
                json!({"name": "bulge", "expr": mm(bulge)}),
            );
            if b < 0.0 {
                // Across the chord the one solution runs the long way
                // round, away from the point: not built, the sketch and the
                // plate fail as in 0.1.1, no body.
                let failed = result["failed"].as_array().unwrap();
                assert_eq!(failed.len(), 2, "{what}: {result}");
                let doc = run(&mut s, "document.inspect", json!({}));
                assert!(
                    doc["bodies"].as_array().is_none_or(Vec::is_empty),
                    "{what}: {doc}"
                );
                continue;
            }
            assert!(result.get("failed").is_none(), "{what}: {result}");
            let (status, p) = points(&mut s, "Arc");
            assert_eq!(status, "ok", "{what}");
            // The arc's centre e3, its ends e1 and e2, its through point e5.
            let h = 20.0 * k;
            let r = (b * b + h * h) / (2.0 * b.abs());
            let [cx, cy] = point(&p, "e3");
            let ey = (b * b - h * h) / (2.0 * b);
            assert!(
                (cx - h).abs() <= 1e-9 * r && (cy - ey).abs() <= 1e-9 * r,
                "{what}: centre ({cx}, {cy}), closed form ({h}, {ey})"
            );
            if b > 0.0 {
                let v = volume(&mut s, "Plate");
                let e = expected(b);
                assert!((v - e).abs() <= 1e-9 * e, "{what}: volume {v}, {e}");
            }
        }
    }
}

/// A sketch whose constraints truly conflict keeps its verdict: an arc
/// through three held points, given a radius it cannot have, still fails
/// naming the radius, from a fresh build and after an edit; the last
/// attempt (the centre through the held points) takes a result only when
/// nothing conflicts, so it cannot turn a conflict into a "solved".
#[test]
fn a_real_conflict_is_still_a_conflict() {
    let mut s = common::session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    steps(
        &mut s,
        &json!([
            ["variable.set", {"name": "bulge", "expr": "6 mm"}],
            ["sketch.create", {"name": "Arc", "plane": "xy"}],
            ["sketch.path", {"sketch": "Arc", "start": ["0 mm", "0 mm"], "segments": [
                {"arc": ["40 mm", "0 mm"], "through": ["20 mm", "bulge"]},
                {"line": "start"}
            ]}]
        ]),
    );
    // The circle through (0, 0), (40, 0) and (20, 6) has a radius of
    // 36.33: 50 cannot be.
    let added = run(
        &mut s,
        "sketch.constrain",
        json!({"sketch": "Arc", "type": "radius", "entities": ["e4"], "value": "50 mm"}),
    );
    let id = added["constraint"].as_str().unwrap().to_string();
    for bulge in ["6 mm", "-3 mm", "0.0001 mm", "6 mm"] {
        run(
            &mut s,
            "variable.set",
            json!({"name": "bulge", "expr": bulge}),
        );
        let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "Arc"}));
        assert_eq!(inspected["status"], "failed", "{bulge}: {inspected}");
        let conflicting: Vec<String> = inspected["conflicting"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c.as_str().unwrap().to_string())
            .collect();
        assert!(conflicting.contains(&id), "{bulge}: {inspected}");
    }
}
