//! Sketch coordinates written as expressions: a point, a corner of a
//! rectangle or a polyline, the centre and end of an arc or path, and a
//! circle's radius are drawn at their value and held there by constraints
//! (`x`, `y`, `radius`) kept in the sketch, so editing a variable moves
//! them. Checked against closed-form volumes and coordinates, across a save
//! and reopen, and against bad parameters.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::assert_close;
use common::checked::{body, error, run, session, session_in};
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::f64::consts::PI;

fn workdir(name: &str) -> std::path::PathBuf {
    common::workdir("sketch-held", name)
}

/// The solved coordinates of a point of sketch `sketch`.
fn xy(s: &mut Session, sketch: &str, id: &str) -> (f64, f64) {
    let inspected = run(s, "sketch.inspect", json!({ "sketch": sketch }));
    let point = inspected["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == id)
        .unwrap_or_else(|| panic!("no entity {id}: {inspected}"))
        .clone();
    (point["x"].as_f64().unwrap(), point["y"].as_f64().unwrap())
}

fn dof(s: &mut Session, sketch: &str) -> Value {
    run(s, "sketch.inspect", json!({ "sketch": sketch }))["dof"].clone()
}

fn assert_xy(actual: (f64, f64), expected: (f64, f64), what: &str) {
    assert!(
        (actual.0 - expected.0).abs() < 1e-9 && (actual.1 - expected.1).abs() < 1e-9,
        "{what}: at {actual:?}, expected {expected:?}"
    );
}

fn set(s: &mut Session, name: &str, expr: &str) {
    run(s, "variable.set", json!({ "name": name, "expr": expr }));
}

/// A plate centred on the origin in x, drawn by one `sketch.rect` with its
/// corners as expressions: fully defined, and it follows width and height.
#[test]
fn a_rectangle_with_expression_corners_follows_the_variables() {
    let mut s = session();
    run(&mut s, "file.new", json!({ "units": "mm" }));
    set(&mut s, "width", "40 mm");
    set(&mut s, "height", "20 mm");
    run(&mut s, "sketch.create", json!({ "name": "P" }));
    let rect = run(
        &mut s,
        "sketch.rect",
        json!({ "sketch": "P", "from": ["-width / 2", 0], "to": ["width / 2", "height"] }),
    );
    // The given corners are held (the 0 beside an expression too), the
    // other two follow by the sides' directions.
    assert_eq!(rect["dof"], json!(0), "{rect}");
    assert_eq!(rect["points"].as_array().unwrap().len(), 4);
    assert_eq!(rect["lines"].as_array().unwrap().len(), 4);
    run(
        &mut s,
        "feature.extrude",
        json!({ "sketch": "P", "distance": 10, "name": "Plate" }),
    );
    assert_close(
        &body(&mut s, "Plate")["volume"],
        40.0 * 20.0 * 10.0,
        "volume",
    );
    let corner = rect["points"][0].as_str().unwrap().to_string();
    assert_xy(xy(&mut s, "P", &corner), (-20.0, 0.0), "from");

    set(&mut s, "width", "60 mm");
    set(&mut s, "height", "15 mm");
    assert_close(
        &body(&mut s, "Plate")["volume"],
        60.0 * 15.0 * 10.0,
        "volume",
    );
    assert_xy(
        xy(&mut s, "P", &corner),
        (-30.0, 0.0),
        "from after the edit",
    );
    let far = rect["points"][2].as_str().unwrap().to_string();
    assert_xy(xy(&mut s, "P", &far), (30.0, 15.0), "to after the edit");
}

/// A point at negative coordinates, a polyline whose corners mix numbers
/// and expressions, and a circle whose radius is an expression.
#[test]
fn points_polylines_and_circles_hold_their_expressions() {
    let mut s = session();
    run(&mut s, "file.new", json!({ "units": "mm" }));
    set(&mut s, "a", "12 mm");
    set(&mut s, "r", "3 mm");
    run(&mut s, "sketch.create", json!({ "name": "S" }));
    let point = run(
        &mut s,
        "sketch.point",
        json!({ "sketch": "S", "at": ["-a", "-a / 4"] }),
    );
    assert_eq!(point["constraints"].as_array().unwrap().len(), 2, "{point}");
    let p = point["entity"].as_str().unwrap().to_string();
    assert_xy(xy(&mut s, "S", &p), (-12.0, -3.0), "point");
    // A number is only where a point is drawn: no constraint.
    let free = run(
        &mut s,
        "sketch.point",
        json!({ "sketch": "S", "at": [5, 5] }),
    );
    assert_eq!(free["constraints"], json!([]));
    // A number beside an expression is held at that number.
    let mixed = run(
        &mut s,
        "sketch.point",
        json!({ "sketch": "S", "at": [7, "a"] }),
    );
    assert_eq!(mixed["constraints"].as_array().unwrap().len(), 2);
    let inspected = run(&mut s, "sketch.inspect", json!({ "sketch": "S" }));
    let kinds: Vec<&str> = inspected["constraints"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["type"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["x", "y", "x", "y"]);
    assert_eq!(inspected["constraints"][0]["value"], "-a");
    assert_eq!(inspected["constraints"][2]["value"], "7");

    set(&mut s, "a", "20 mm");
    assert_xy(xy(&mut s, "S", &p), (-20.0, -5.0), "point after the edit");

    // A triangle and a hole in it: the triangle's corners are expressions.
    run(&mut s, "sketch.create", json!({ "name": "T" }));
    run(
        &mut s,
        "sketch.polyline",
        json!({ "sketch": "T", "points": [[0, 0], ["2 * a", 0], [0, "a"]], "closed": true }),
    );
    let circle = run(
        &mut s,
        "sketch.circle",
        json!({ "sketch": "T", "center": ["a / 2", "a / 4"], "radius": "r" }),
    );
    assert_eq!(
        circle["constraints"].as_array().unwrap().len(),
        1,
        "{circle}"
    );
    run(
        &mut s,
        "feature.extrude",
        json!({ "sketch": "T", "distance": 2, "name": "Tri" }),
    );
    let volume = |a: f64, r: f64| (a * a - PI * r * r) * 2.0;
    assert_close(&body(&mut s, "Tri")["volume"], volume(20.0, 3.0), "volume");
    set(&mut s, "a", "30 mm");
    set(&mut s, "r", "4 mm");
    assert_close(
        &body(&mut s, "Tri")["volume"],
        volume(30.0, 4.0),
        "volume after",
    );
}

/// The corners and centres of an arc, given to `sketch.line`, `sketch.arc`
/// and `sketch.path`, as expressions: a slot whose length and radius are
/// variables.
#[test]
fn paths_and_arcs_take_expressions_for_their_points() {
    let mut s = session();
    run(&mut s, "file.new", json!({ "units": "mm" }));
    set(&mut s, "len", "30 mm");
    set(&mut s, "rd", "5 mm");
    run(&mut s, "sketch.create", json!({ "name": "Slot" }));
    run(
        &mut s,
        "sketch.path",
        json!({
            "sketch": "Slot",
            "start": [0, "-rd"],
            "segments": [
                { "line": ["len", "-rd"] },
                { "arc": ["len", "rd"], "center": ["len", 0] },
                { "line": [0, "rd"] },
                { "arc": "start", "center": [0, "0"] },
            ],
        }),
    );
    assert_eq!(dof(&mut s, "Slot"), json!(0));
    run(
        &mut s,
        "feature.extrude",
        json!({ "sketch": "Slot", "distance": 1, "name": "Bar" }),
    );
    let area = |len: f64, rd: f64| 2.0 * rd * len + PI * rd * rd;
    assert_close(&body(&mut s, "Bar")["volume"], area(30.0, 5.0), "volume");
    set(&mut s, "len", "45 mm");
    set(&mut s, "rd", "7 mm");
    assert_close(
        &body(&mut s, "Bar")["volume"],
        area(45.0, 7.0),
        "volume after",
    );

    // sketch.arc and sketch.line with held points.
    run(&mut s, "sketch.create", json!({ "name": "Q" }));
    let arc = run(
        &mut s,
        "sketch.arc",
        json!({ "sketch": "Q", "center": [0, 0], "start": ["rd", 0], "end": [0, "rd"] }),
    );
    let end = arc["end"].as_str().unwrap().to_string();
    assert_xy(xy(&mut s, "Q", &end), (0.0, 7.0), "arc end");
    let line = run(
        &mut s,
        "sketch.line",
        json!({ "sketch": "Q", "from": end, "to": ["-len", "rd"] }),
    );
    let to = line["end"].as_str().unwrap().to_string();
    assert_xy(xy(&mut s, "Q", &to), (-45.0, 7.0), "line end");
}

/// `x` and `y` are constraints like any other: `sketch.constrain` adds
/// them to an existing point, `sketch.edit` changes their value.
#[test]
fn x_and_y_are_constraints_of_their_own() {
    let mut s = session();
    run(&mut s, "file.new", json!({ "units": "mm" }));
    run(&mut s, "sketch.create", json!({ "name": "S" }));
    let p = run(
        &mut s,
        "sketch.point",
        json!({ "sketch": "S", "at": [1, 1] }),
    )["entity"]
        .as_str()
        .unwrap()
        .to_string();
    let c = run(
        &mut s,
        "sketch.constrain",
        json!({ "sketch": "S", "type": "x", "entities": [p], "value": "-8 mm" }),
    )["constraint"]
        .as_str()
        .unwrap()
        .to_string();
    run(
        &mut s,
        "sketch.constrain",
        json!({ "sketch": "S", "type": "y", "entities": [p], "value": "2.5" }),
    );
    assert_xy(xy(&mut s, "S", &p), (-8.0, 2.5), "constrained");
    assert_eq!(dof(&mut s, "S"), json!(0));
    run(
        &mut s,
        "sketch.edit",
        json!({ "sketch": "S", "constraint": c, "value": "-1 cm" }),
    );
    assert_xy(xy(&mut s, "S", &p), (-10.0, 2.5), "edited");
}

/// Held coordinates are part of the document: a saved file reopens to the
/// same sketch, still following its variables.
#[test]
fn held_coordinates_survive_a_save_and_reopen() {
    let dir = workdir("save");
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({ "units": "mm" }));
    set(&mut s, "w", "10 mm");
    run(&mut s, "sketch.create", json!({ "name": "P" }));
    run(
        &mut s,
        "sketch.rect",
        json!({ "sketch": "P", "from": ["-w", "-w"], "to": ["w", "w / 2"] }),
    );
    run(
        &mut s,
        "feature.extrude",
        json!({ "sketch": "P", "distance": 1, "name": "Plate" }),
    );
    let before = run(&mut s, "sketch.inspect", json!({ "sketch": "P" }));
    run(&mut s, "file.save", json!({ "path": "held.3dcraft" }));
    run(&mut s, "file.close", json!({}));
    run(&mut s, "file.open", json!({ "path": "held.3dcraft" }));
    let after = run(&mut s, "sketch.inspect", json!({ "sketch": "P" }));
    assert_eq!(before["constraints"], after["constraints"]);
    assert_close(&body(&mut s, "Plate")["volume"], 20.0 * 15.0, "volume");
    set(&mut s, "w", "20 mm");
    assert_close(
        &body(&mut s, "Plate")["volume"],
        40.0 * 30.0,
        "volume after",
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Expressions that are not lengths, do not parse or name nothing, and
/// rectangles with no width, are refused with a reason; nothing is added.
#[test]
fn bad_held_coordinates_are_refused() {
    let mut s = session();
    run(&mut s, "file.new", json!({ "units": "mm" }));
    set(&mut s, "w", "10 mm");
    run(&mut s, "sketch.create", json!({ "name": "S" }));
    let cases = [
        ("sketch.point", json!({ "sketch": "S", "at": ["nope", 0] })),
        ("sketch.point", json!({ "sketch": "S", "at": ["2 kg", 0] })),
        ("sketch.point", json!({ "sketch": "S", "at": ["w +", 0] })),
        ("sketch.point", json!({ "sketch": "S", "at": [{}, 0] })),
        ("sketch.point", json!({ "sketch": "S", "at": ["w"] })),
        ("sketch.point", json!({ "sketch": "S", "at": ["1 / 0", 0] })),
        (
            "sketch.point",
            json!({ "sketch": "S", "at": ["x".repeat(5000), 0] }),
        ),
        (
            "sketch.circle",
            json!({ "sketch": "S", "center": [0, 0], "radius": "w * 1 kg" }),
        ),
        (
            "sketch.circle",
            json!({ "sketch": "S", "center": [0, 0], "radius": true }),
        ),
        (
            "sketch.rect",
            json!({ "sketch": "S", "from": [0, 0], "to": ["w", 0] }),
        ),
        (
            "sketch.rect",
            json!({ "sketch": "S", "from": [0, 0], "to": [0, 0] }),
        ),
        ("sketch.rect", json!({ "sketch": "S", "from": [0, 0] })),
        (
            "sketch.rect",
            json!({ "sketch": "S", "from": "e99", "to": [1, 1] }),
        ),
        (
            "sketch.rect",
            json!({ "sketch": "S", "from": "e1[2]", "to": [1, 1] }),
        ),
        (
            "sketch.polyline",
            json!({ "sketch": "S", "points": [[0, 0], ["w", null]] }),
        ),
        (
            "sketch.constrain",
            json!({ "sketch": "S", "type": "x", "entities": ["e1"] }),
        ),
        (
            "sketch.constrain",
            json!({ "sketch": "S", "type": "y", "entities": [], "value": 1 }),
        ),
        (
            "sketch.path",
            json!({ "sketch": "S", "start": ["w", "?"], "segments": [{ "line": [1, 1] }] }),
        ),
        (
            "sketch.spline",
            json!({ "sketch": "S", "through": [[0, 0], ["w", 3], [20, 0], [30, 5]] }),
        ),
    ];
    for (id, params) in cases {
        let message = error(&mut s, id, params.clone());
        assert!(message.starts_with(id), "{id} {params}: {message}");
    }
    let inspected = run(&mut s, "sketch.inspect", json!({ "sketch": "S" }));
    assert_eq!(inspected["entities"], json!([]), "{inspected}");
    assert_eq!(inspected["constraints"], json!([]));
}
