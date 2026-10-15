//! `feature.revolve`: volumes and centroids against Pappus and the closed
//! forms of the solids of revolution, through the command layer.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};
use std::f64::consts::PI;

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    let result = session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"));
    assert_ne!(
        session.regeneration_matches_a_full_one(),
        Some(false),
        "after {id} {params}"
    );
    result
}

/// A document with sketch `Profile` (F1) on the xy plane holding a closed
/// polygon through `points`: points e1..en, sides e(n+1)..e(2n).
fn polygon(points: &[[f64; 2]]) -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({}));
    run(&mut s, "sketch.create", json!({"name": "Profile"}));
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Profile", "closed": true, "points": points}),
    );
    s
}

/// The rectangle 10 <= x <= 20, 0 <= y <= 5: area 50, centroid (15, 2.5).
fn rectangle() -> Session {
    polygon(&[[10.0, 0.0], [20.0, 0.0], [20.0, 5.0], [10.0, 5.0]])
}

fn revolve(session: &mut Session, params: Value) -> Value {
    let mut params = params;
    params["sketch"] = "Profile".into();
    params["name"] = "Turned".into();
    run(session, "feature.revolve", params)
}

fn feature(session: &mut Session, name: &str) -> Value {
    let document = run(session, "document.inspect", json!({}));
    document["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == name)
        .unwrap_or_else(|| panic!("no feature {name}"))
        .clone()
}

fn body(session: &mut Session) -> Value {
    let document = run(session, "document.inspect", json!({}));
    document["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == "Turned")
        .unwrap_or_else(|| panic!("no body: {}", document["features"]))
        .clone()
}

fn assert_turned(session: &mut Session, volume: f64, centroid: [f64; 3]) {
    let found = body(session);
    let actual = found["volume"].as_f64().unwrap();
    assert!(
        (actual - volume).abs() <= 1e-9 * volume,
        "volume {actual}, expected {volume}"
    );
    for axis in 0..3 {
        let c = found["centroid"][axis].as_f64().unwrap();
        assert!(
            (c - centroid[axis]).abs() <= 1e-8 * (1.0 + volume.cbrt()),
            "centroid {}, expected {centroid:?}",
            found["centroid"]
        );
    }
    let check = run(session, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
}

fn assert_failed(session: &mut Session, words: &[&str]) {
    let state = feature(session, "Turned");
    assert_eq!(state["status"], "failed", "{state}");
    let reason = state["error"].as_str().unwrap();
    for word in words {
        assert!(reason.contains(word), "`{word}` is not in: {reason}");
    }
}

/// The distance from the axis to the centroid of a ring sector between
/// radii `inner` and `outer` that spans `angle`.
fn sector_centroid(inner: f64, outer: f64, angle: f64) -> f64 {
    let radial = 2.0 / 3.0 * (outer.powi(3) - inner.powi(3)) / (outer.powi(2) - inner.powi(2));
    radial * (angle / 2.0).sin() / (angle / 2.0)
}

#[test]
fn a_full_turn_of_a_rectangle_is_a_ring() {
    let mut s = rectangle();
    let made = revolve(&mut s, json!({"axis": "y"}));
    assert_eq!(made["status"], "ok", "{made}");
    // Pappus: area 50 times the path 2 pi 15 of its centroid.
    assert_turned(&mut s, 1500.0 * PI, [0.0, 2.5, 0.0]);
    // About x the rectangle touches the axis along its bottom: a disc
    // with a bore, pi (5^2) 10 ... the rectangle is 0 <= y <= 5, so the
    // solid is a full cylinder of radius 5 and length 10.
    run(&mut s, "feature.delete", json!({"feature": "Turned"}));
    revolve(&mut s, json!({"axis": "x"}));
    assert_turned(&mut s, 250.0 * PI, [15.0, 0.0, 0.0]);
    // A full turn has no caps: four faces, one per side that is off the
    // axis, each named after its side.
    let names = body(&mut s)["faceNames"].clone();
    let mut names: Vec<&str> = names
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap())
        .collect();
    names.sort_unstable();
    assert_eq!(names, ["F3:side(e6)", "F3:side(e7)", "F3:side(e8)"]);
}

#[test]
fn a_part_turn_goes_counter_clockwise_about_the_axis() {
    let mut s = rectangle();
    revolve(&mut s, json!({"axis": "y", "angle": "90 deg"}));
    // Seen from the tip of +Y, counter-clockwise takes +X towards -Z: the
    // quarter ring lies in x > 0, z < 0, its centroid on the bisector.
    let r = sector_centroid(10.0, 20.0, PI / 2.0);
    let d = r * (PI / 4.0).cos();
    assert_turned(&mut s, 375.0 * PI, [d, 2.5, -d]);
    // The other way.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Turned", "angle": "-90 deg"}),
    );
    assert_turned(&mut s, 375.0 * PI, [d, 2.5, d]);
    // Half to each side of the sketch plane.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Turned", "angle": "90 deg", "symmetric": true}),
    );
    assert_turned(&mut s, 375.0 * PI, [r, 2.5, 0.0]);
    // Three quarters, from a variable.
    run(
        &mut s,
        "variable.set",
        json!({"name": "sweep", "expr": "270 deg"}),
    );
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Turned", "angle": "sweep", "symmetric": false}),
    );
    let r = sector_centroid(10.0, 20.0, 1.5 * PI);
    let bisector = 0.75 * PI;
    assert_turned(
        &mut s,
        1125.0 * PI,
        [r * bisector.cos(), 2.5, -r * bisector.sin()],
    );
    // A part turn has two caps, named like those of an extrusion: the
    // start cap lies in the sketch plane.
    let names = body(&mut s)["faceNames"].clone();
    let names: Vec<&str> = names
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap())
        .collect();
    assert_eq!(names.len(), 6, "{names:?}");
    for name in [
        "F2:startCap(e5,e6,e7,e8)",
        "F2:endCap(e5,e6,e7,e8)",
        "F2:side(e5)",
        "F2:side(e8)",
    ] {
        assert!(names.contains(&name), "{name} is not in {names:?}");
    }
}

#[test]
fn a_line_of_the_sketch_is_an_axis_with_a_direction() {
    let mut s = rectangle();
    // A construction line along the sketch's y axis, upwards, at x = 4:
    // the ring is now between radii 6 and 16.
    run(
        &mut s,
        "sketch.line",
        json!({"sketch": "Profile", "from": [4, -3], "to": [4, 9], "construction": true}),
    );
    revolve(&mut s, json!({"axis": "e11", "angle": "90 deg"}));
    let r = sector_centroid(6.0, 16.0, PI / 2.0);
    let d = r * (PI / 4.0).cos();
    assert_turned(&mut s, 50.0 * 2.0 * PI * 11.0 / 4.0, [4.0 + d, 2.5, -d]);
    // The same line drawn downwards turns the other way.
    run(&mut s, "feature.delete", json!({"feature": "Turned"}));
    run(
        &mut s,
        "sketch.delete",
        json!({"sketch": "Profile", "entity": "e11"}),
    );
    run(
        &mut s,
        "sketch.line",
        json!({"sketch": "Profile", "from": [4, 9], "to": [4, -3], "construction": true}),
    );
    revolve(&mut s, json!({"axis": "e14", "angle": "90 deg"}));
    assert_turned(&mut s, 50.0 * 2.0 * PI * 11.0 / 4.0, [4.0 + d, 2.5, d]);
    // The axis moves with the sketch: at x = 0 the ring is between 10 and
    // 20 again.
    run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "Profile", "entity": "e12", "at": [0, 9]}),
    );
    run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "Profile", "entity": "e13", "at": [0, -3]}),
    );
    let r = sector_centroid(10.0, 20.0, PI / 2.0);
    let d = r * (PI / 4.0).cos();
    assert_turned(&mut s, 375.0 * PI, [d, 2.5, d]);
}

#[test]
fn the_classic_solids_have_their_closed_forms() {
    // A right triangle with a leg on the y axis: a cone of radius 6 and
    // height 9, its centroid a quarter of the height above the base.
    let mut s = polygon(&[[0.0, 0.0], [6.0, 0.0], [0.0, 9.0]]);
    revolve(&mut s, json!({"axis": "y"}));
    assert_turned(&mut s, PI * 36.0 * 9.0 / 3.0, [0.0, 2.25, 0.0]);

    // Half a disc with its diameter on the y axis: a sphere of radius 5.
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({}));
    run(&mut s, "sketch.create", json!({"name": "Profile"}));
    run(
        &mut s,
        "sketch.arc",
        json!({"sketch": "Profile", "center": [0, 2], "start": [0, -3], "end": [0, 7]}),
    );
    run(
        &mut s,
        "sketch.line",
        json!({"sketch": "Profile", "from": "e3", "to": "e2"}),
    );
    revolve(&mut s, json!({"axis": "y"}));
    assert_turned(&mut s, 4.0 / 3.0 * PI * 125.0, [0.0, 2.0, 0.0]);

    // A circle off the axis: a torus, 2 pi^2 R r^2.
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({}));
    run(
        &mut s,
        "sketch.create",
        json!({"name": "Profile", "plane": "yz"}),
    );
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Profile", "center": [12, 1], "radius": 3}),
    );
    // On yz the sketch's x is world Y and its y is world Z: about the
    // sketch's y axis the ring lies round world Z, at height 1.
    revolve(&mut s, json!({"axis": "y"}));
    assert_turned(&mut s, 2.0 * PI * PI * 12.0 * 9.0, [0.0, 0.0, 1.0]);

    // A profile with a hole, half a turn: Pappus with the net area.
    let mut s = rectangle();
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Profile", "center": [15, 2.5], "radius": 1}),
    );
    revolve(&mut s, json!({"axis": "y", "angle": "180 deg"}));
    let volume = (50.0 - PI) * PI * 15.0;
    let found = body(&mut s);
    let actual = found["volume"].as_f64().unwrap();
    assert!(
        (actual - volume).abs() <= 1e-9 * volume,
        "{actual} vs {volume}"
    );
    let check = run(&mut s, "document.check", json!({}));
    assert_eq!(check["ok"], true, "{check}");
}

#[test]
fn what_cannot_be_revolved_fails_with_the_reason() {
    let mut s = rectangle();
    revolve(&mut s, json!({"axis": "y"}));
    for (params, words) in [
        (json!({"angle": 0}), vec!["angle", "other than zero"]),
        (
            json!({"angle": "400 deg"}),
            vec!["angle", "at most a full turn"],
        ),
        (json!({"angle": "5 mm"}), vec!["angle", "angle is needed"]),
        (json!({"angle": "nope"}), vec!["angle", "nope"]),
    ] {
        let mut params = params;
        params["feature"] = "Turned".into();
        run(&mut s, "feature.edit", params);
        assert_failed(&mut s, &words);
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(feature(&mut s, "Turned")["status"], "ok");
    }
    // Slightly over a full turn by rounding is a full turn.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Turned", "angle": "360.0000000001 deg"}),
    );
    assert_turned(&mut s, 1500.0 * PI, [0.0, 2.5, 0.0]);
    run(&mut s, "feature.delete", json!({"feature": "Turned"}));

    // An axis that is not there, is not a line, or has no direction.
    for (axis, words) in [
        ("e99", vec!["axis", "no entity e99"]),
        ("e1", vec!["axis", "e1", "not a line"]),
    ] {
        revolve(&mut s, json!({"axis": axis}));
        assert_failed(&mut s, &words);
        run(&mut s, "feature.delete", json!({"feature": "Turned"}));
    }
    // The profile on both sides of the axis.
    run(
        &mut s,
        "sketch.line",
        json!({"sketch": "Profile", "from": [15, -3], "to": [15, 9], "construction": true}),
    );
    revolve(&mut s, json!({"axis": "e11"}));
    assert_failed(&mut s, &["axis", "Profile", "both sides", "e5"]);
    // An arc whose circle reaches the axis without being centred on it
    // would sweep a spindle torus.
    run(&mut s, "feature.delete", json!({"feature": "Turned"}));
    run(
        &mut s,
        "sketch.delete",
        json!({"sketch": "Profile", "entity": "e6"}),
    );
    run(
        &mut s,
        "sketch.arc",
        json!({"sketch": "Profile", "center": [16, 2.5], "start": "e2", "end": "e3"}),
    );
    revolve(&mut s, json!({"axis": "y"}));
    let state = feature(&mut s, "Turned");
    assert_eq!(
        state["status"], "ok",
        "an arc clear of the axis is a torus: {state}"
    );
    run(&mut s, "feature.delete", json!({"feature": "Turned"}));
    revolve(&mut s, json!({"axis": "e11"}));
    assert_failed(&mut s, &["Profile"]);
    run(&mut s, "feature.delete", json!({"feature": "Turned"}));

    // Malformed requests are refused.
    for (params, words) in [
        (
            json!({"sketch": "Profile"}),
            vec!["missing parameter `axis`"],
        ),
        (
            json!({"sketch": "Profile", "axis": "z"}),
            vec!["`axis` must be"],
        ),
        (
            json!({"sketch": "Profile", "axis": [0, 1]}),
            vec!["`axis` must be"],
        ),
        (
            json!({"sketch": "Profile", "axis": -1}),
            vec!["`axis` must be"],
        ),
        (json!({"axis": "x"}), vec!["missing parameter `sketch`"]),
        (
            json!({"sketch": "Nope", "axis": "x"}),
            vec!["no feature named `Nope`"],
        ),
        (
            json!({"sketch": "Profile", "axis": "x", "angle": [1]}),
            vec!["angle"],
        ),
        (
            json!({"sketch": "Profile", "axis": "x", "taper": 1}),
            vec!["unknown parameter `taper`"],
        ),
    ] {
        let e = s
            .execute("feature.revolve", &params)
            .unwrap_err()
            .to_string();
        for word in &words {
            assert!(e.contains(word), "{params}: `{word}` is not in: {e}");
        }
    }
}

#[test]
fn a_sketch_lies_on_the_cap_of_a_part_turn() {
    let mut s = rectangle();
    revolve(&mut s, json!({"axis": "y", "angle": "90 deg"}));
    // The end cap is the profile turned a quarter round: it lies in the
    // plane x = 0 and faces -X (the way the turn was going).
    let seat = run(
        &mut s,
        "sketch.create",
        json!({"name": "Seat", "face": "F2:endCap()"}),
    );
    assert_eq!(seat["status"], "ok", "{seat}");
    let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "Seat"}));
    let axis = |key: &str| -> [f64; 3] {
        let v = &inspected["frame"][key];
        [0, 1, 2].map(|i| v[i].as_f64().unwrap())
    };
    let (x, y) = (axis("x"), axis("y"));
    let normal = [
        x[1] * y[2] - x[2] * y[1],
        x[2] * y[0] - x[0] * y[2],
        x[0] * y[1] - x[1] * y[0],
    ];
    for (actual, expected) in normal.iter().zip([-1.0, 0.0, 0.0]) {
        assert!((actual - expected).abs() < 1e-12, "normal {normal:?}");
    }
    // The start cap is the profile where it was drawn, facing +Z (against
    // the turn).
    run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "Seat", "face": "F2:startCap()"}),
    );
    let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "Seat"}));
    assert_eq!(inspected["status"], "ok", "{inspected}");
    let z = |key: &str| inspected["frame"][key][2].as_f64().unwrap();
    assert!(z("x").abs() < 1e-12 && z("y").abs() < 1e-12, "{inspected}");
}

/// A spline profile that meets the axis at right angles turns into a dome
/// whose pole is as smooth as a sphere's: the mesh's normals there are
/// the axis, at any size (lamp T1, 2026-10-16: the vertices at the pole
/// had no normal, and the dome was shaded with a starburst there).
#[test]
fn a_revolved_spline_dome_is_shaded_with_the_axis_at_its_pole() {
    for k in [1e-3, 1.0, 1e3] {
        let mut s = Session::new(Workspace::none());
        run(&mut s, "file.new", json!({}));
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "zx", "name": "Profile"}),
        );
        let control =
            [[50.0, 0.0], [50.0, 30.0], [20.0, 40.0], [0.0, 40.0]].map(|[a, b]| [a * k, b * k]);
        run(
            &mut s,
            "sketch.spline",
            json!({"sketch": "Profile", "control": control}),
        );
        run(
            &mut s,
            "sketch.line",
            json!({"sketch": "Profile", "from": "e4", "to": [0, 0]}),
        );
        run(
            &mut s,
            "sketch.line",
            json!({"sketch": "Profile", "from": "e6", "to": "e1"}),
        );
        revolve(&mut s, json!({"axis": "x"}));
        for tolerance in [None, Some(1e-2 * k)] {
            let scene = s.scene(tolerance).unwrap().unwrap();
            let mesh = &scene.mesh;
            let middle = mesh.bbox().center().unwrap();
            let mut at_pole = 0;
            for (p, n) in mesh.positions.iter().zip(&mesh.normals) {
                // The pole: on the axis (the sketch's x axis, which on
                // the zx plane is the world's z), away from the flat end.
                if p.x.abs() > 1e-9 * k || p.y.abs() > 1e-9 * k || p.z.abs() < k {
                    continue;
                }
                at_pole += 1;
                let outward = (p.z - middle.z).signum();
                let angle = n.x.hypot(n.y).atan2(n.z * outward);
                assert!(
                    (n.length() - 1.0).abs() < 1e-9 && angle < 1e-4,
                    "size {k}, tolerance {tolerance:?}: normal {n:?} at the pole {p:?}"
                );
            }
            assert!(at_pole > 0, "size {k}: no vertex at the pole");
        }
    }
}
