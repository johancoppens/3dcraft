//! The short form of a cap, `F2:startCap()` / `F2:endCap()`: the cap of
//! that role of that feature, whatever outline it was named after. It is
//! accepted wherever a face is named (one rule, `PersistentName::shortens`,
//! used by `Names::face_answers_to` and the selection's lookup), and every
//! feature built with it is, body for body, the one built with the full
//! name.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{error, run, session};
use craft3d_engine::Session;
use serde_json::{Value, json};

/// A 40 x 20 plate 10 thick (`F2`), and a round bar of radius 2 standing on
/// it, extruded from a circle (`F4`), joined (`F5`).
fn plate_and_bar(s: &mut Session) {
    run(s, "file.new", json!({ "units": "mm" }));
    run(s, "sketch.create", json!({ "name": "P" }));
    run(
        s,
        "sketch.rect",
        json!({ "sketch": "P", "from": [0, 0], "to": [40, 20] }),
    );
    run(
        s,
        "feature.extrude",
        json!({ "sketch": "P", "distance": 10, "name": "Plate" }),
    );
    run(
        s,
        "sketch.create",
        json!({ "name": "C", "plane": "xy", "offset": 10 }),
    );
    run(
        s,
        "sketch.circle",
        json!({ "sketch": "C", "center": [20, 10], "radius": 2 }),
    );
    run(
        s,
        "feature.extrude",
        json!({ "sketch": "C", "distance": 12, "name": "Bar" }),
    );
}

/// The whole document as `document.inspect` reports it, without what
/// names the references: the features' status and the bodies' volumes,
/// areas and counts.
fn built(s: &mut Session) -> Value {
    let inspected = run(s, "document.inspect", json!({}));
    let features: Vec<Value> = inspected["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| json!([f["name"], f["status"], f["error"]]))
        .collect();
    let measured = run(s, "document.measure", json!({}));
    json!({ "features": features, "measure": measured })
}

/// Each command run with the short form and, in another document, with the
/// full name: the same bodies.
#[test]
fn short_caps_build_what_the_full_names_build() {
    let cases: [(&str, Value, Value); 5] = [
        (
            "feature.chamfer",
            json!({ "edges": [["F2:endCap()", "F2:side(e5)"]], "distance": 1, "name": "X" }),
            json!({ "edges": [["F2:endCap(e5,e6,e7,e8)", "F2:side(e5)"]], "distance": 1, "name": "X" }),
        ),
        (
            "feature.fillet",
            json!({ "edges": [["F2:startCap()", "F2:side(e6)"]], "radius": 1, "name": "X" }),
            json!({ "edges": [["F2:startCap(e5,e6,e7,e8)", "F2:side(e6)"]], "radius": 1, "name": "X" }),
        ),
        (
            "feature.offsetFace",
            json!({ "faces": ["F2:startCap()"], "distance": 1, "name": "X" }),
            json!({ "faces": ["F2:startCap(e5,e6,e7,e8)"], "distance": 1, "name": "X" }),
        ),
        (
            "feature.thread",
            json!({ "face": "F4:side(e2)", "start": "F4:endCap()", "size": "M4", "length": "6 mm", "name": "X" }),
            json!({ "face": "F4:side(e2)", "start": "F4:endCap(e2)", "size": "M4", "length": "6 mm", "name": "X" }),
        ),
        (
            "datum.plane",
            json!({ "from": "F4:endCap()", "distance": 3, "name": "X" }),
            json!({ "from": "F4:endCap(e2)", "distance": 3, "name": "X" }),
        ),
    ];
    for (id, short, full) in cases {
        let mut a = session();
        plate_and_bar(&mut a);
        let made = run(&mut a, id, short.clone());
        assert_eq!(made["status"], "ok", "{id} {short}: {made}");
        let mut b = session();
        plate_and_bar(&mut b);
        run(&mut b, id, full);
        assert_eq!(built(&mut a), built(&mut b), "{id}");
    }
}

/// The datums, a sketch on the cap of the bar after the bar went into a
/// union, the selection and the colour of a face: each finds the face the
/// full name finds.
#[test]
fn readers_of_face_names_take_the_short_form() {
    let mut s = session();
    plate_and_bar(&mut s);
    let axis = run(
        &mut s,
        "datum.axis",
        json!({ "edge": ["F2:endCap()", "F2:side(e5)"], "name": "A" }),
    );
    assert_eq!(axis["datum"]["origin"], json!([0.0, 0.0, 10.0]), "{axis}");
    let selected = run(&mut s, "select.set", json!({ "names": ["F2:endCap()"] }));
    assert_eq!(
        selected["items"][0]["name"], "F2:endCap(e5,e6,e7,e8)",
        "{selected}"
    );
    let coloured = run(
        &mut s,
        "face.appearance",
        json!({ "faces": ["F4:startCap()"], "color": "#ff0000" }),
    );
    assert_eq!(
        coloured["faces"][0]["face"], "F4:startCap(e2)",
        "{coloured}"
    );
    // Through a union: the copy of the cap, written short.
    let union = run(
        &mut s,
        "feature.boolean",
        json!({ "op": "union", "target": "Plate", "tool": "Bar", "name": "U" }),
    );
    let face = format!("{}:copy(1)/F4:endCap()", union["id"].as_str().unwrap());
    let top = run(
        &mut s,
        "sketch.create",
        json!({ "face": face, "name": "Top" }),
    );
    assert_eq!(top["status"], "ok", "{top}");
    let thread = run(
        &mut s,
        "feature.thread",
        json!({ "face": "F4:side(e2)", "start": "F4:endCap()", "size": "M4", "length": "6 mm" }),
    );
    assert_eq!(thread["status"], "ok", "{thread}");
}

/// A short cap where there is none, or where it means two faces to a
/// reader that wants one, is refused with the faces there are; a feature
/// given one fails with them.
#[test]
fn a_short_cap_that_names_no_face_is_refused() {
    let mut s = session();
    run(&mut s, "file.new", json!({ "units": "mm" }));
    run(
        &mut s,
        "feature.block",
        json!({ "size": [10, 10, 10], "name": "B" }),
    );
    let message = error(&mut s, "select.set", json!({ "names": ["F1:startCap()"] }));
    assert!(message.contains("F1:startCap()"), "{message}");
    let failed = run(
        &mut s,
        "feature.chamfer",
        json!({ "edges": [["F1:endCap()", "F1:face(2)"]], "distance": 1 }),
    );
    assert_eq!(failed["status"], "failed", "{failed}");
    assert!(
        failed["error"].as_str().unwrap().contains("F1:face(0)"),
        "{failed}"
    );
    // Two profiles extruded at once have two end caps: a selection of
    // "the" end cap is refused.
    run(&mut s, "sketch.create", json!({ "name": "Two" }));
    run(
        &mut s,
        "sketch.rect",
        json!({ "sketch": "Two", "from": [20, 0], "to": [25, 5] }),
    );
    run(
        &mut s,
        "sketch.rect",
        json!({ "sketch": "Two", "from": [30, 0], "to": [35, 5] }),
    );
    run(
        &mut s,
        "feature.extrude",
        json!({ "sketch": "Two", "distance": 2, "name": "Pads" }),
    );
    error(&mut s, "select.set", json!({ "names": ["F3:endCap()"] }));
}
