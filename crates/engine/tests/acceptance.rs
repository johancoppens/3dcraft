//! The acceptance scenario of phase 1 (docs/roadmap.md), as far as it can
//! be run today: everything but STEP export and the independent program
//! that reads it back.
//!
//! > Create a document. Define variables `width`, `height`, `thickness`,
//! > `hole_d`. Sketch an L-shaped profile constrained by those variables,
//! > extrude it, cut two holes, save as `.3dcraft`. Reopen it, change
//! > `width`, regenerate: the holes follow. Export [...] 3MF.
//! > `document.inspect` reports the volume predicted by hand calculation.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Access, Session, Workspace};
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

fn constrain(s: &mut Session, sketch: &str, kind: &str, entities: Value, extra: Value) {
    let mut params = json!({"sketch": sketch, "type": kind, "entities": entities});
    for (key, value) in extra.as_object().unwrap() {
        params[key] = value.clone();
    }
    let result = run(s, "sketch.constrain", params);
    assert_eq!(result["status"], "ok", "{result}");
}

/// The volume by hand: the L is a bar of width x thickness and an upright
/// of (height - thickness) x thickness, `depth` deep, less two holes.
fn by_hand(width: f64, height: f64, thickness: f64, hole_d: f64, depth: f64) -> f64 {
    (width * thickness + (height - thickness) * thickness) * depth
        - 2.0 * PI * (hole_d / 2.0).powi(2) * depth
}

fn part_volume(s: &mut Session) -> f64 {
    let document = run(s, "document.inspect", json!({}));
    let bodies = document["bodies"].as_array().unwrap();
    assert_eq!(bodies.len(), 1, "one part: {bodies:?}");
    assert_eq!(bodies[0]["name"], "Part");
    let check = run(s, "document.check", json!({}));
    assert_eq!(check["ok"], true, "{check}");
    bodies[0]["volume"].as_f64().unwrap()
}

#[test]
fn the_phase_1_scenario_without_step() {
    let dir = std::env::temp_dir().join(format!("craft3d-acceptance-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    run(&mut s, "file.new", json!({"units": "mm"}));
    for (name, expr) in [
        ("width", "40 mm"),
        ("height", "30 mm"),
        ("thickness", "5 mm"),
        ("hole_d", "3 mm"),
    ] {
        run(&mut s, "variable.set", json!({"name": name, "expr": expr}));
    }

    // The L, drawn roughly and pulled into shape by its constraints:
    // corners e1..e6 counter-clockwise from the origin, sides e7..e12.
    run(&mut s, "sketch.create", json!({"name": "Profile"}));
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Profile", "closed": true,
               "points": [[0, 0], [38, 1.5], [41, 6], [4, 4.5], [6.5, 28], [-1, 31]]}),
    );
    constrain(&mut s, "Profile", "fixed", json!(["e1"]), json!({}));
    for (line, kind) in [
        ("e7", "horizontal"),
        ("e8", "vertical"),
        ("e9", "horizontal"),
        ("e10", "vertical"),
        ("e11", "horizontal"),
        ("e12", "vertical"),
    ] {
        constrain(&mut s, "Profile", kind, json!([line]), json!({}));
    }
    for (line, value) in [
        ("e7", "width"),
        ("e12", "height"),
        ("e8", "thickness"),
        ("e11", "thickness"),
    ] {
        constrain(
            &mut s,
            "Profile",
            "distance",
            json!([line]),
            json!({"value": value}),
        );
    }
    let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "Profile"}));
    assert_eq!(inspected["dof"], 0, "fully constrained: {inspected}");
    let made = run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Profile", "distance": 8, "name": "Bracket"}),
    );
    assert_eq!(made["status"], "ok", "{made}");

    // Two holes on its top, each placed from the corner by the variables:
    // one in the middle of the bar, one in the middle of the upright.
    run(
        &mut s,
        "sketch.create",
        json!({"name": "Holes", "face": "F2:endCap()"}),
    );
    run(
        &mut s,
        "sketch.point",
        json!({"sketch": "Holes", "at": [0, 0]}),
    );
    constrain(&mut s, "Holes", "fixed", json!(["e1"]), json!({}));
    for (at, across, up) in [
        ([18.0, 3.0], "width / 2", "thickness / 2"),
        ([3.0, 17.0], "thickness / 2", "height / 2"),
    ] {
        let circle = run(
            &mut s,
            "sketch.circle",
            json!({"sketch": "Holes", "center": at, "radius": 1}),
        );
        let (center, id) = (circle["center"].clone(), circle["entity"].clone());
        constrain(
            &mut s,
            "Holes",
            "diameter",
            json!([id]),
            json!({"value": "hole_d"}),
        );
        constrain(
            &mut s,
            "Holes",
            "distance",
            json!(["e1", center]),
            json!({"value": across, "direction": "horizontal"}),
        );
        constrain(
            &mut s,
            "Holes",
            "distance",
            json!(["e1", center]),
            json!({"value": up, "direction": "vertical"}),
        );
    }
    let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "Holes"}));
    assert_eq!(inspected["dof"], 0, "{inspected}");
    let cut = run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Holes", "distance": -8, "cut": "Bracket", "name": "Part"}),
    );
    assert_eq!(cut["status"], "ok", "{cut}");
    let volume = part_volume(&mut s);
    let expected = by_hand(40.0, 30.0, 5.0, 3.0, 8.0);
    assert!(
        (volume - expected).abs() <= 1e-9 * expected,
        "{volume} vs {expected}"
    );

    // Save, reopen, change the width: the holes follow.
    run(&mut s, "file.save", json!({"path": "bracket.3dcraft"}));
    run(&mut s, "file.close", json!({}));
    run(&mut s, "file.open", json!({"path": "bracket.3dcraft"}));
    let reopened = part_volume(&mut s);
    assert_eq!(
        reopened.to_bits(),
        volume.to_bits(),
        "bit for bit after reopening"
    );
    run(
        &mut s,
        "variable.set",
        json!({"name": "width", "expr": "64 mm"}),
    );
    let wider = part_volume(&mut s);
    let expected = by_hand(64.0, 30.0, 5.0, 3.0, 8.0);
    assert!(
        (wider - expected).abs() <= 1e-9 * expected,
        "{wider} vs {expected}"
    );
    // The first hole is in the middle of the longer bar now; the second
    // stayed in the upright.
    let holes = run(&mut s, "sketch.inspect", json!({"sketch": "Holes"}));
    let text = holes.to_string();
    assert_eq!(holes["status"], "ok", "{text}");
    let centers: Vec<(f64, f64)> = holes["entities"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["type"] == "point")
        .map(|e| (e["x"].as_f64().unwrap(), e["y"].as_f64().unwrap()))
        .collect();
    let near = |p: (f64, f64), q: (f64, f64)| (p.0 - q.0).abs() < 1e-6 && (p.1 - q.1).abs() < 1e-6;
    assert!(centers.iter().any(|&c| near(c, (32.0, 2.5))), "{centers:?}");
    assert!(centers.iter().any(|&c| near(c, (2.5, 15.0))), "{centers:?}");
    // A bigger hole, a thicker part.
    run(
        &mut s,
        "variable.set",
        json!({"name": "hole_d", "expr": "4 mm"}),
    );
    run(
        &mut s,
        "variable.set",
        json!({"name": "thickness", "expr": "8 mm"}),
    );
    let changed = part_volume(&mut s);
    let expected = by_hand(64.0, 30.0, 8.0, 4.0, 8.0);
    assert!(
        (changed - expected).abs() <= 1e-9 * expected,
        "{changed} vs {expected}"
    );

    // Export: one closed mesh.
    let exported = run(&mut s, "file.export", json!({"path": "bracket.3mf"}));
    assert_eq!(exported["closed"], exported["bodies"], "{exported}");
    assert!(dir.join("bracket.3mf").metadata().unwrap().len() > 1000);
    std::fs::remove_dir_all(&dir).unwrap();
}
