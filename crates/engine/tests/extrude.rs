//! Extrusions through the commands: a sketch becomes a solid whose volume,
//! area and centroid were computed by hand.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{body, error, run, session, session_in};
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::f64::consts::PI;
use std::path::PathBuf;

fn workdir(name: &str) -> PathBuf {
    common::workdir("extrude", name)
}

/// The sketch is solved to the document's tolerance of 1e-6 mm, so its
/// corners are within a few tolerances of the exact ones. On a part some
/// tens of millimetres across that is a relative error of about 1e-7 in a
/// length, and a small multiple of that in an area or a volume.
fn close(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() <= 1e-6 * expected.abs().max(1.0)
}

fn assert_close(actual: &Value, expected: f64, what: &str) {
    let actual = actual
        .as_f64()
        .unwrap_or_else(|| panic!("{what}: {actual}"));
    assert!(
        close(actual, expected),
        "{what}: {actual}, expected {expected}"
    );
}

fn constrain(session: &mut Session, kind: &str, entities: Value, value: Option<&str>) {
    let mut params = json!({"sketch": "Profile", "type": kind, "entities": entities});
    if let Some(value) = value {
        params["value"] = value.into();
    }
    let result = run(session, "sketch.constrain", params);
    assert_eq!(result["status"], "ok", "{result}");
}

/// The L profile of the phase-1 scenario, fully constrained:
///
/// ```text
///  e6 +--+ e5            points e1..e6, lines e7 (e1-e2) .. e12 (e6-e1)
///     |  |
///     |  +------+ e3     width 40, height 30, thickness 5:
///     |  e4     |        area 40*5 + 5*25 = 325, perimeter 2*(40+30) = 140
///  e1 +---------+ e2
/// ```
fn l_sketch(session: &mut Session, plane: &str, offset: &str) {
    run(session, "file.new", json!({"units": "mm"}));
    for (name, expr) in [
        ("width", "40 mm"),
        ("height", "30 mm"),
        ("thickness", "5 mm"),
        ("depth", "8 mm"),
        ("hole_d", "3 mm"),
    ] {
        run(session, "variable.set", json!({"name": name, "expr": expr}));
    }
    run(
        session,
        "sketch.create",
        json!({"name": "Profile", "plane": plane, "offset": offset}),
    );
    run(
        session,
        "sketch.polyline",
        json!({"sketch": "Profile", "closed": true,
               "points": [[0, 0], [38, 1.5], [41, 6], [4, 4.5], [6.5, 28], [-1, 31]]}),
    );
    constrain(session, "fixed", json!(["e1"]), None);
    for (line, kind) in [
        ("e7", "horizontal"),
        ("e8", "vertical"),
        ("e9", "horizontal"),
        ("e10", "vertical"),
        ("e11", "horizontal"),
        ("e12", "vertical"),
    ] {
        constrain(session, kind, json!([line]), None);
    }
    constrain(session, "distance", json!(["e7"]), Some("width"));
    constrain(session, "distance", json!(["e12"]), Some("height"));
    constrain(session, "distance", json!(["e8"]), Some("thickness"));
    constrain(session, "distance", json!(["e11"]), Some("thickness"));
}

fn assert_bbox(body: &Value, lo: [f64; 3], hi: [f64; 3]) {
    for axis in 0..3 {
        for (corner, expected) in [(0, lo), (1, hi)] {
            let actual = body["bbox"][corner][axis].as_f64().unwrap();
            assert!(
                (actual - expected[axis]).abs() <= 1e-5,
                "bbox {}: {actual}, expected {}",
                body["bbox"],
                expected[axis]
            );
        }
    }
}

#[test]
fn an_l_profile_becomes_a_solid_of_the_predicted_volume() {
    let mut s = session();
    l_sketch(&mut s, "xy", "0");
    let made = run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Profile", "distance": "depth", "name": "Bracket"}),
    );
    assert_eq!(made, json!({"id": "F2", "name": "Bracket", "status": "ok"}));
    let solid = body(&mut s, "Bracket");
    assert_eq!(solid["kind"], "solid");
    // Two caps and six sides; three edges per corner (two cap edges and
    // an upright), twelve corners.
    assert_eq!(
        (&solid["faces"], &solid["edges"], &solid["vertices"]),
        (&json!(8), &json!(18), &json!(12))
    );
    // Volume = area x depth = 325 x 8. Area = two caps + perimeter x depth
    // = 650 + 140 x 8.
    assert_close(&solid["volume"], 2600.0, "volume");
    assert_close(&solid["area"], 1770.0, "area");
    // The L is a 40 x 5 bar centred at (20, 2.5), area 200, and a 5 x 25
    // post centred at (2.5, 17.5), area 125.
    let cx = (200.0 * 20.0 + 125.0 * 2.5) / 325.0;
    let cy = (200.0 * 2.5 + 125.0 * 17.5) / 325.0;
    assert_close(&solid["centroid"][0], cx, "centroid x");
    assert_close(&solid["centroid"][1], cy, "centroid y");
    assert_close(&solid["centroid"][2], 4.0, "centroid z");
    assert_bbox(&solid, [0.0, 0.0, 0.0], [40.0, 30.0, 8.0]);
    let check = run(&mut s, "document.check", json!({}));
    assert_eq!(
        check,
        json!({"ok": true, "bodiesChecked": 1, "problems": []})
    );

    // The part follows its variables.
    run(
        &mut s,
        "variable.set",
        json!({"name": "width", "expr": "60 mm"}),
    );
    run(
        &mut s,
        "variable.set",
        json!({"name": "depth", "expr": "1 cm"}),
    );
    let wider = body(&mut s, "Bracket");
    // Area 60*5 + 5*25 = 425.
    assert_close(&wider["volume"], 4250.0, "volume after the change");
    assert_bbox(&wider, [0.0, 0.0, 0.0], [60.0, 30.0, 10.0]);
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_close(
        &body(&mut s, "Bracket")["volume"],
        2600.0,
        "volume after undo",
    );
}

#[test]
fn circles_inside_a_profile_are_holes_that_follow_the_variables() {
    let mut s = session();
    l_sketch(&mut s, "xy", "0");
    // Two holes in the bar, at a quarter and three quarters of the width,
    // half-way up its thickness.
    for (x, fraction) in [(9.0, "width / 4"), (31.0, "3 * width / 4")] {
        let circle = run(
            &mut s,
            "sketch.circle",
            json!({"sketch": "Profile", "center": [x, 2.0], "radius": 1.0}),
        );
        let (hole, centre) = (circle["entity"].clone(), circle["center"].clone());
        constrain(&mut s, "diameter", json!([hole]), Some("hole_d"));
        let across = run(
            &mut s,
            "sketch.constrain",
            json!({"sketch": "Profile", "type": "distance", "entities": ["e1", centre],
                   "direction": "horizontal", "value": fraction}),
        );
        assert_eq!(across["status"], "ok");
        let up = run(
            &mut s,
            "sketch.constrain",
            json!({"sketch": "Profile", "type": "distance", "entities": ["e1", centre],
                   "direction": "vertical", "value": "thickness / 2"}),
        );
        assert_eq!(
            (&up["status"], &up["dof"]),
            (&json!("ok"), &json!(0)),
            "{up}"
        );
    }
    run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Profile", "distance": "depth"}),
    );
    let solid = body(&mut s, "Extrude1");
    // Each hole adds a wall, two circles and two vertices.
    assert_eq!(
        (&solid["faces"], &solid["edges"], &solid["vertices"]),
        (&json!(10), &json!(22), &json!(16))
    );
    let hole = PI * 1.5 * 1.5;
    assert_close(&solid["volume"], (325.0 - 2.0 * hole) * 8.0, "volume");
    // Caps lose the discs, the walls add their circumference x depth.
    assert_close(
        &solid["area"],
        2.0 * (325.0 - 2.0 * hole) + (140.0 + 2.0 * PI * 3.0) * 8.0,
        "area",
    );
    // Bigger holes in a wider bar: still two of them, still inside.
    run(
        &mut s,
        "variable.set",
        json!({"name": "width", "expr": "80 mm"}),
    );
    run(
        &mut s,
        "variable.set",
        json!({"name": "hole_d", "expr": "4 mm"}),
    );
    let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "Profile"}));
    let centres: Vec<(f64, f64)> = ["e13", "e15"]
        .iter()
        .map(|id| {
            let e = inspected["entities"]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["id"] == *id)
                .unwrap();
            (e["x"].as_f64().unwrap(), e["y"].as_f64().unwrap())
        })
        .collect();
    for ((x, y), expected) in centres.iter().zip([20.0, 60.0]) {
        assert!(
            (x - expected).abs() <= 1e-5 && (y - 2.5).abs() <= 1e-5,
            "({x}, {y})"
        );
    }
    let solid = body(&mut s, "Extrude1");
    // Area 80*5 + 125 = 525, minus two discs of radius 2.
    assert_close(
        &solid["volume"],
        (525.0 - 2.0 * PI * 4.0) * 8.0,
        "volume after",
    );
    // A hole that leaves the material is an error that says so, not a
    // wrong solid: at 6 mm the holes are wider than the 5 mm bar.
    run(
        &mut s,
        "variable.set",
        json!({"name": "hole_d", "expr": "6 mm"}),
    );
    let document = run(&mut s, "document.inspect", json!({}));
    assert_eq!(document["features"][1]["status"], "failed", "{document}");
    // It names a hole (e14 or e16) that runs into a side of the bar.
    let reason = document["features"][1]["error"].as_str().unwrap();
    assert!(
        reason.contains("cross or touch") && (reason.contains("e14") || reason.contains("e16")),
        "{reason}"
    );
    assert_eq!(document["bodies"], json!([]));
}

#[test]
fn direction_and_plane_place_the_solid() {
    // (plane, offset, distance, symmetric, lowest corner, highest corner)
    type Case = (
        &'static str,
        &'static str,
        &'static str,
        bool,
        [f64; 3],
        [f64; 3],
    );
    let cases: [Case; 6] = [
        ("xy", "0", "8", false, [0.0, 0.0, 0.0], [40.0, 30.0, 8.0]),
        ("xy", "0", "-8", false, [0.0, 0.0, -8.0], [40.0, 30.0, 0.0]),
        ("xy", "2", "8", true, [0.0, 0.0, -2.0], [40.0, 30.0, 6.0]),
        ("xy", "2", "-8", true, [0.0, 0.0, -2.0], [40.0, 30.0, 6.0]),
        // On yz the sketch x is world Y and the sketch y is world Z.
        ("yz", "10", "8", false, [10.0, 0.0, 0.0], [18.0, 40.0, 30.0]),
        // On zx the sketch x is world Z and the sketch y is world X.
        ("zx", "-3", "8", false, [0.0, -3.0, 0.0], [30.0, 5.0, 40.0]),
    ];
    for (plane, offset, distance, symmetric, lo, hi) in cases {
        let mut s = session();
        l_sketch(&mut s, plane, offset);
        let made = run(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Profile", "distance": distance, "symmetric": symmetric}),
        );
        assert_eq!(made["status"], "ok", "{plane} {distance}: {made}");
        let solid = body(&mut s, "Extrude1");
        assert_bbox(&solid, lo, hi);
        // Whatever the direction, the solid is not inside out.
        assert_close(&solid["volume"], 2600.0, "volume");
        assert_eq!(run(&mut s, "document.check", json!({}))["ok"], true);
    }
}

#[test]
fn faces_are_named_after_what_made_them() {
    let mut s = session();
    l_sketch(&mut s, "xy", "0");
    run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Profile", "distance": "depth"}),
    );
    let names = |s: &mut Session| -> Vec<String> {
        let mut names: Vec<String> = body(s, "Extrude1")["faceNames"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_str().unwrap().to_string())
            .collect();
        names.sort();
        names
    };
    let before = names(&mut s);
    assert_eq!(
        before,
        [
            "F2:endCap(e7,e8,e9,e10,e11,e12)",
            "F2:side(e10)",
            "F2:side(e11)",
            "F2:side(e12)",
            "F2:side(e7)",
            "F2:side(e8)",
            "F2:side(e9)",
            "F2:startCap(e7,e8,e9,e10,e11,e12)",
        ]
    );
    // A hole drawn afterwards adds a face and renames none.
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Profile", "center": [20, 2.5], "radius": 1}),
    );
    let after = names(&mut s);
    assert_eq!(after.len(), 9);
    assert!(before.iter().all(|name| after.contains(name)), "{after:?}");
    assert!(after.contains(&"F2:side(e14)".to_string()), "{after:?}");
    // Changing a dimension renames nothing either.
    run(
        &mut s,
        "variable.set",
        json!({"name": "width", "expr": "90 mm"}),
    );
    assert_eq!(names(&mut s), after);
}

#[test]
fn a_sketch_that_cannot_be_extruded_says_why() {
    let mut s = session();
    l_sketch(&mut s, "xy", "0");
    run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Profile", "distance": "depth"}),
    );
    let status = |s: &mut Session| -> (String, String) {
        let document = run(s, "document.inspect", json!({}));
        let feature = document["features"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["name"] == "Extrude1")
            .unwrap()
            .clone();
        (
            feature["status"].as_str().unwrap().to_string(),
            feature["error"].as_str().unwrap_or("").to_string(),
        )
    };
    assert_eq!(status(&mut s), ("ok".to_string(), String::new()));
    // (what is done, what undoes it is one edit.undo, words the reason has)
    let breakages: [(&str, Value, &[&str]); 7] = [
        (
            "variable.set",
            json!({"name": "depth", "expr": "0 mm"}),
            &["distance", "zero"],
        ),
        (
            "variable.set",
            json!({"name": "depth", "expr": "45 deg"}),
            &["distance", "length"],
        ),
        (
            "variable.delete",
            json!({"name": "depth", "force": true}),
            &["distance", "depth"],
        ),
        // An open profile: one side of the L is gone, and its neighbours
        // e8 and e10 end in nothing.
        (
            "sketch.delete",
            json!({"sketch": "Profile", "entity": "e9"}),
            &["Profile", "not closed", "e8", "e10"],
        ),
        (
            "feature.suppress",
            json!({"feature": "Profile"}),
            &["Profile", "suppressed"],
        ),
        (
            "feature.move",
            json!({"feature": "Profile", "position": 1}),
            &["Profile", "after"],
        ),
        ("feature.delete", json!({"feature": "Profile"}), &["F1"]),
    ];
    for (command, params, words) in breakages {
        run(&mut s, command, params.clone());
        let (state, reason) = status(&mut s);
        assert_eq!(state, "failed", "{command} {params}");
        for word in words {
            assert!(reason.contains(word), "{command} {params}: {reason}");
        }
        let document = run(&mut s, "document.inspect", json!({}));
        assert_eq!(document["bodies"], json!([]), "{command}");
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(status(&mut s).0, "ok", "after undoing {command}");
    }
    // A sketch whose own constraints conflict fails the extrusion too.
    run(
        &mut s,
        "sketch.constrain",
        json!({"sketch": "Profile", "type": "distance", "entities": ["e9"], "value": "10 mm"}),
    );
    let (state, reason) = status(&mut s);
    assert_eq!(state, "failed");
    assert!(
        reason.contains("Profile") && reason.contains("failed"),
        "{reason}"
    );
    run(&mut s, "edit.undo", json!({}));
    // Construction geometry is not part of a profile: with the bar's
    // lower edge marked as construction the L is open.
    run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "Profile", "entity": "e7", "construction": true}),
    );
    assert_eq!(status(&mut s).0, "failed");
    run(&mut s, "edit.undo", json!({}));
    // And a construction line across the profile changes nothing.
    run(
        &mut s,
        "sketch.line",
        json!({"sketch": "Profile", "from": [-5, 2], "to": [50, 3], "construction": true}),
    );
    assert_eq!(status(&mut s).0, "ok");
    assert_close(&body(&mut s, "Extrude1")["volume"], 2600.0, "volume");
}

#[test]
fn bad_requests_are_refused_and_parameters_can_be_edited() {
    let mut s = session();
    l_sketch(&mut s, "xy", "0");
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": [1, 2, 3]}),
    );
    for (params, expected) in [
        (json!({"distance": 5}), "missing parameter `sketch`"),
        (json!({"sketch": "Profile"}), "missing parameter `distance`"),
        (
            json!({"sketch": "Nope", "distance": 5}),
            "no feature named `Nope`",
        ),
        (json!({"sketch": "Plate", "distance": 5}), "is not a sketch"),
        (
            json!({"sketch": "Profile", "distance": [5]}),
            "`distance` must be an expression",
        ),
        (
            json!({"sketch": "Profile", "distance": 5, "symmetric": "yes"}),
            "`symmetric` must be true or false",
        ),
        (
            json!({"sketch": "Profile", "distance": 5, "taper": 2}),
            "unknown parameter `taper`",
        ),
    ] {
        let message = error(&mut s, "feature.extrude", params.clone());
        assert!(message.contains(expected), "{params}: {message}");
    }
    run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Profile", "distance": 8, "name": "Bracket"}),
    );
    let edited = run(
        &mut s,
        "feature.edit",
        json!({"feature": "Bracket", "distance": "2 * depth", "symmetric": true}),
    );
    assert_eq!(edited["status"], "ok");
    let solid = body(&mut s, "Bracket");
    assert_close(&solid["volume"], 325.0 * 16.0, "volume");
    assert_bbox(&solid, [0.0, 0.0, -8.0], [40.0, 30.0, 8.0]);
    let wrong = error(
        &mut s,
        "feature.edit",
        json!({"feature": "Bracket", "radius": 3}),
    );
    assert!(
        wrong.contains("not a parameter of a extrude") || wrong.contains("extrude"),
        "{wrong}"
    );
    let wrong = error(
        &mut s,
        "feature.edit",
        json!({"feature": "Plate", "distance": 3}),
    );
    assert!(wrong.contains("block"), "{wrong}");
}

#[test]
fn an_extruded_part_is_saved_reopened_rendered_and_exported() {
    let dir = workdir("files");
    let mut s = session_in(&dir);
    l_sketch(&mut s, "xy", "0");
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Profile", "center": [20, 2.5], "radius": 1.5}),
    );
    run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Profile", "distance": "depth", "name": "Bracket"}),
    );
    let before = run(&mut s, "document.measure", json!({}));
    assert_close(&before["volume"], (325.0 - PI * 2.25) * 8.0, "volume");
    run(&mut s, "file.save", json!({"path": "bracket.3dcraft"}));
    run(&mut s, "file.close", json!({}));
    run(&mut s, "file.open", json!({"path": "bracket.3dcraft"}));
    assert_eq!(run(&mut s, "document.measure", json!({})), before);
    let tolerance = 0.01;
    let exported = run(
        &mut s,
        "file.export",
        json!({"path": "bracket.3mf", "tolerance": tolerance}),
    );
    // Watertight: every edge of the mesh is shared by two triangles.
    assert_eq!(
        (&exported["bodies"], &exported["closed"]),
        (&json!(1), &json!(1))
    );
    let rendered = run(
        &mut s,
        "view.render",
        json!({"path": "bracket.png", "width": 640, "height": 480}),
    );
    assert!(rendered["coverage"].as_f64().unwrap() > 0.05, "{rendered}");
    assert!(dir.join("bracket.3mf").metadata().unwrap().len() > 1000);
    assert!(dir.join("bracket.png").metadata().unwrap().len() > 1000);
    let _ = std::fs::remove_dir_all(&dir);
}
