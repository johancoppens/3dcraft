//! `sketch.path`: a chain of lines and arcs through shared corners in one
//! command. The profiles it draws are extruded and measured against their
//! closed forms; each way of giving an arc is checked against the circle
//! worked out by hand.

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
    common::workdir("sketch-path", name)
}

fn entity(inspected: &Value, id: &str) -> Value {
    inspected["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == id)
        .unwrap_or_else(|| panic!("no entity {id}"))
        .clone()
}

/// The solved coordinates of a point of an inspected sketch.
fn xy(inspected: &Value, id: &str) -> (f64, f64) {
    let point = entity(inspected, id);
    (point["x"].as_f64().unwrap(), point["y"].as_f64().unwrap())
}

fn text(value: &Value) -> &str {
    value.as_str().unwrap_or_else(|| panic!("{value}"))
}

/// Nothing in these sketches is driven by a dimension: the points are
/// where the command put them, and the solver has nothing to correct
/// beyond the rounding of the arithmetic that placed the centres.
const EXACT: f64 = 1e-9;

fn assert_at(actual: (f64, f64), expected: (f64, f64), scale: f64, what: &str) {
    assert!(
        (actual.0 - expected.0).abs() <= EXACT * scale
            && (actual.1 - expected.1).abs() <= EXACT * scale,
        "{what}: at {actual:?}, expected {expected:?}"
    );
}

/// A new document with a sketch `P`, the path drawn in it and extruded
/// over `depth` as `Body`. Returns what `sketch.path` answered.
fn drawn_and_extruded(
    s: &mut Session,
    units: &str,
    start: Value,
    segments: Value,
    closed: bool,
    depth: f64,
) -> Value {
    run(s, "file.new", json!({"units": units}));
    run(s, "sketch.create", json!({"name": "P"}));
    let path = run(
        s,
        "sketch.path",
        json!({"sketch": "P", "start": start, "segments": segments, "closed": closed}),
    );
    assert_eq!(path["status"], "ok", "{path}");
    let made = run(
        s,
        "feature.extrude",
        json!({"sketch": "P", "distance": depth, "name": "Body"}),
    );
    assert_eq!(made["status"], "ok", "{made}");
    path
}

fn assert_volume(s: &mut Session, expected: f64, what: &str) -> Value {
    let solid = body(s, "Body");
    assert_eq!(solid["kind"], "solid", "{what}");
    assert_eq!(solid["valid"], true, "{what}");
    let volume = solid["volume"].as_f64().unwrap();
    assert!(
        (volume - expected).abs() <= 1e-9 * expected,
        "{what}: volume {volume}, expected {expected}"
    );
    solid
}

/// Three unit systems and sizes: a part of some tens of millimetres, one
/// of some kilometres in metres, and one of about an inch.
const SCALES: [(&str, f64); 3] = [("mm", 1.0), ("m", 250.0), ("in", 0.03125)];

#[test]
fn a_slot_drawn_as_a_path_has_the_volume_of_a_slot() {
    for (units, k) in SCALES {
        let mut s = session();
        let (l, r, depth) = (30.0 * k, 5.0 * k, 7.0 * k);
        // Two lines and two half circles, each tangent to its neighbours,
        // the last one ending where the path started.
        let path = drawn_and_extruded(
            &mut s,
            units,
            json!([0.0, -r]),
            json!([
                {"line": [l, -r], "tangent": true},
                {"arc": [l, r], "tangent": true},
                {"line": [0.0, r], "tangent": true},
                {"arc": "start", "tangent": true},
            ]),
            false,
            depth,
        );
        assert_eq!(
            path,
            json!({"id": "F1", "name": "P", "status": "ok", "dof": 6,
                   "points": ["e1", "e2", "e4", "e7", "e1"],
                   "entities": ["e3", "e6", "e8", "e10"],
                   "centers": [null, "e5", null, "e9"],
                   "constraints": ["c1", "c2", "c3", "c4"]}),
            "{units}"
        );
        // Area: a rectangle l x 2r and a full circle.
        let solid = assert_volume(&mut s, (2.0 * r * l + PI * r * r) * depth, units);
        // Two caps, two flat sides and two half cylinders.
        assert_eq!(solid["faces"], 6, "{units}");
        // Tangent in four corners leaves a belt around two circles: the
        // centre and radius of each, 6 degrees of freedom, none of the
        // constraints redundant.
        let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "P"}));
        assert_eq!(inspected["dof"], 6);
        for constraint in inspected["constraints"].as_array().unwrap() {
            assert_eq!(constraint["type"], "tangent");
            assert_eq!(constraint["state"], "satisfied", "{constraint}");
        }
        assert_at(xy(&inspected, "e5"), (l, 0.0), k, "first centre");
        assert_at(xy(&inspected, "e9"), (0.0, 0.0), k, "second centre");
    }
}

#[test]
fn a_rounded_rectangle_drawn_as_a_path_has_its_volume() {
    for (units, k) in SCALES {
        let mut s = session();
        let (w, h, r, depth) = (40.0 * k, 24.0 * k, 6.0 * k, 3.0 * k);
        // Counter-clockwise from the bottom edge; each corner is given in
        // another way. The through point of the third is the middle of
        // that corner: at 135 degrees about (r, h - r).
        let d = r * std::f64::consts::FRAC_1_SQRT_2;
        let path = drawn_and_extruded(
            &mut s,
            units,
            json!([r, 0.0]),
            json!([
                {"line": [w - r, 0.0]},
                {"arc": [w, r], "center": [w - r, r]},
                {"line": [w, h - r]},
                {"arc": [w - r, h], "radius": r},
                {"line": [r, h]},
                {"arc": [0.0, h - r], "through": [r - d, h - r + d]},
                {"line": [0.0, r]},
                {"arc": "start", "tangent": true},
            ]),
            false,
            depth,
        );
        assert_eq!(path["entities"].as_array().unwrap().len(), 8);
        assert_eq!(path["constraints"], json!(["c1"]));
        let solid = assert_volume(&mut s, (w * h - (4.0 - PI) * r * r) * depth, units);
        assert_eq!(solid["faces"], 10, "{units}");
        // The four centres are the corners of the rectangle inset by r.
        let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "P"}));
        let centers: Vec<&str> = path["centers"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|c| c.as_str())
            .collect();
        let expected = [(w - r, r), (w - r, h - r), (r, h - r), (r, r)];
        assert_eq!(centers.len(), 4);
        for (id, at) in centers.iter().zip(expected) {
            assert_at(xy(&inspected, id), at, k, id);
        }
    }
}

#[test]
fn d_shapes_drawn_as_paths_have_their_volumes() {
    for (units, k) in SCALES {
        // Half a disc: an arc about the origin and the line that closes it.
        let mut s = session();
        let (r, depth) = (5.0 * k, 2.0 * k);
        let path = drawn_and_extruded(
            &mut s,
            units,
            json!([0.0, -r]),
            json!([{"arc": [0.0, r], "center": [0.0, 0.0]}]),
            true,
            depth,
        );
        assert_eq!(
            (&path["points"], &path["entities"], &path["centers"]),
            (
                &json!(["e1", "e2"]),
                &json!(["e4", "e5"]),
                &json!(["e3", null])
            )
        );
        let solid = assert_volume(&mut s, PI * r * r / 2.0 * depth, units);
        assert_eq!(solid["faces"], 4, "{units}");
        // It bulges towards +x: counter-clockwise from the bottom.
        let hi = solid["bbox"][1][0].as_f64().unwrap();
        let lo = solid["bbox"][0][0].as_f64().unwrap();
        assert!(hi > 0.9 * r && lo > -0.1 * r, "{units}: {}", solid["bbox"]);

        // A shaft with a flat: a disc of radius 5k without the segment
        // beyond the chord at x = -3k, drawn clockwise the long way round
        // from the top of the chord to its bottom.
        let mut s = session();
        drawn_and_extruded(
            &mut s,
            units,
            json!([-3.0 * k, 4.0 * k]),
            json!([{"arc": [-3.0 * k, -4.0 * k], "radius": r, "large": true, "clockwise": true}]),
            true,
            depth,
        );
        // The segment cut off spans the half angle a with cos a = 3/5 and
        // sin a = 4/5: its area is r^2 (a - sin a cos a).
        let a = 0.6_f64.acos();
        let area = PI * r * r - r * r * (a - 0.8 * 0.6);
        let solid = assert_volume(&mut s, area * depth, units);
        let hi = solid["bbox"][1][0].as_f64().unwrap();
        assert!(hi > 0.9 * r, "{units}: {}", solid["bbox"]);
    }
}

/// The centre of an arc, and whether the path walks it counter-clockwise.
type Walked = ((f64, f64), bool);

/// A chain that uses every way of giving an arc, with the circle each one
/// must come out as: its centre, and whether the path walks it
/// counter-clockwise. The corners are on the x axis up to the line.
fn arc_forms() -> (Value, Vec<Option<Walked>>) {
    let segments = json!([
        // About (5, 0), counter-clockwise: under the axis.
        {"arc": [10, 0], "center": [5, 0]},
        // About (15, 0), clockwise: over the axis.
        {"arc": [20, 0], "center": [15, 0], "clockwise": true},
        // Through the top of the circle about (25, 0): clockwise.
        {"arc": [30, 0], "through": [25, 5]},
        // Through the bottom of the circle about (35, 0): counter-clockwise.
        {"arc": [40, 0], "through": [35, -5]},
        // A chord of 6 and a radius of 5: the centres are 4 above or below
        // the middle of the chord. Counter-clockwise the short way round,
        // the centre is on the left, above.
        {"arc": [46, 0], "radius": 5},
        {"arc": [52, 0], "radius": 5, "large": true},
        {"arc": [58, 0], "radius": 5, "clockwise": true},
        {"arc": [64, 0], "radius": 5, "large": true, "clockwise": true},
        {"line": [70, 0]},
        // Leaving along +x and ending up and to the right: a quarter turn
        // to the left about (70, 4).
        {"arc": [74, 4], "tangent": true},
        // Arriving there along +y: a quarter turn to the right about
        // (78, 4), and another one about the same centre.
        {"arc": [78, 8], "tangent": true},
        {"arc": [82, 4], "tangent": true},
    ]);
    let expected = vec![
        Some(((5.0, 0.0), true)),
        Some(((15.0, 0.0), false)),
        Some(((25.0, 0.0), false)),
        Some(((35.0, 0.0), true)),
        Some(((43.0, 4.0), true)),
        Some(((49.0, -4.0), true)),
        Some(((55.0, -4.0), false)),
        Some(((61.0, 4.0), false)),
        None,
        Some(((70.0, 4.0), true)),
        Some(((78.0, 4.0), false)),
        Some(((78.0, 4.0), false)),
    ];
    (segments, expected)
}

#[test]
fn each_way_of_giving_an_arc_gives_the_arc_worked_out_by_hand() {
    let (segments, expected) = arc_forms();
    let corners: Vec<(f64, f64)> = [0.0, 10.0, 20.0, 30.0, 40.0, 46.0, 52.0, 58.0, 64.0, 70.0]
        .iter()
        .map(|x| (*x, 0.0))
        .chain([(74.0, 4.0), (78.0, 8.0), (82.0, 4.0)])
        .collect();
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "sketch.create", json!({"name": "P"}));
    let path = run(
        &mut s,
        "sketch.path",
        json!({"sketch": "P", "start": [0, 0], "segments": segments}),
    );
    // 13 corners and 11 centres are 48 coordinates; each arc keeps its
    // ends equally far from its centre (11) and three are tangent.
    assert_eq!(path["dof"], 48 - 11 - 3);
    assert_eq!(path["constraints"], json!(["c1", "c2", "c3"]));
    let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "P"}));
    for constraint in inspected["constraints"].as_array().unwrap() {
        assert_eq!(constraint["state"], "satisfied", "{constraint}");
    }
    let points = path["points"].as_array().unwrap();
    let entities = path["entities"].as_array().unwrap();
    let centers = path["centers"].as_array().unwrap();
    assert_eq!(
        (points.len(), entities.len(), centers.len()),
        (corners.len(), expected.len(), expected.len())
    );
    for (id, at) in points.iter().zip(&corners) {
        assert_at(xy(&inspected, text(id)), *at, 1.0, text(id));
    }
    for (index, wanted) in expected.iter().enumerate() {
        let drawn = entity(&inspected, text(&entities[index]));
        let (from, to) = (&points[index], &points[index + 1]);
        let Some((center, ccw)) = wanted else {
            assert_eq!(drawn["type"], "line", "segment {index}");
            assert_eq!((&drawn["start"], &drawn["end"]), (from, to));
            assert_eq!(centers[index], Value::Null);
            continue;
        };
        assert_eq!(drawn["type"], "arc", "segment {index}");
        assert_eq!(drawn["center"], centers[index], "segment {index}");
        let found = xy(&inspected, text(&drawn["center"]));
        assert_at(found, *center, 1.0, &format!("centre of segment {index}"));
        // An arc is stored counter-clockwise from its start to its end: a
        // segment walked clockwise has them the other way round.
        let stored = if *ccw { (from, to) } else { (to, from) };
        assert_eq!(
            (&drawn["start"], &drawn["end"]),
            stored,
            "segment {index} goes the wrong way round"
        );
        // Both corners are on the circle.
        let radius = |id: &Value| {
            let (x, y) = xy(&inspected, text(id));
            (x - found.0).hypot(y - found.1)
        };
        assert!(
            (radius(from) - radius(to)).abs() <= EXACT,
            "segment {index}"
        );
        // The tangent arcs are quarter turns of radius 4; the others
        // were all given a radius of 5.
        let wanted_radius = if index >= 9 { 4.0 } else { 5.0 };
        assert!(
            (radius(from) - wanted_radius).abs() <= EXACT,
            "segment {index}: radius {}",
            radius(from)
        );
    }
}

/// What a sketch is, whatever its ids: the lines and arcs in the order
/// they were drawn, with the solved coordinates of their points.
fn shapes(s: &mut Session, sketch: &str) -> Vec<(String, Vec<(f64, f64)>)> {
    let inspected = run(s, "sketch.inspect", json!({"sketch": sketch}));
    inspected["entities"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["type"] != "point")
        .map(|e| {
            let points = ["center", "start", "end"]
                .iter()
                .filter_map(|key| e[*key].as_str())
                .map(|id| xy(&inspected, id))
                .collect();
            (text(&e["type"]).to_string(), points)
        })
        .collect()
}

#[test]
fn a_path_is_what_lines_and_arcs_drawn_one_by_one_are() {
    let (w, h, r) = (40.0, 24.0, 6.0);
    // Counter-clockwise from the bottom edge, with one corner walked
    // clockwise in the path that goes the other way below.
    let corners = [
        ((w - r, 0.0), (w, r), (w - r, r)),
        ((w, h - r), (w - r, h), (w - r, h - r)),
        ((r, h), (0.0, h - r), (r, h - r)),
        ((0.0, r), (r, 0.0), (r, r)),
    ];

    // By hand: every corner id is read back from a response.
    let mut hand = session();
    run(&mut hand, "file.new", json!({"units": "mm"}));
    run(&mut hand, "sketch.create", json!({"name": "P"}));
    let first = run(
        &mut hand,
        "sketch.point",
        json!({"sketch": "P", "at": [r, 0.0]}),
    )["entity"]
        .clone();
    let mut at = first.clone();
    let mut drawn = Vec::new();
    for (index, (line_end, arc_end, center)) in corners.iter().enumerate() {
        let line = run(
            &mut hand,
            "sketch.line",
            json!({"sketch": "P", "from": at, "to": [line_end.0, line_end.1]}),
        );
        let end = if index == 3 {
            first.clone()
        } else {
            json!([arc_end.0, arc_end.1])
        };
        let arc = run(
            &mut hand,
            "sketch.arc",
            json!({"sketch": "P", "center": [center.0, center.1], "start": line["end"], "end": end}),
        );
        at = arc["end"].clone();
        drawn.push(line["entity"].clone());
        drawn.push(arc["entity"].clone());
    }
    let mut last = json!(null);
    for index in 0..drawn.len() {
        let before = &drawn[(index + drawn.len() - 1) % drawn.len()];
        last = run(
            &mut hand,
            "sketch.constrain",
            json!({"sketch": "P", "type": "tangent", "entities": [before, drawn[index]]}),
        );
    }
    run(
        &mut hand,
        "feature.extrude",
        json!({"sketch": "P", "distance": 3, "name": "Body"}),
    );

    // As one path, from the coordinates alone.
    let mut one = session();
    let segments: Vec<Value> = corners
        .iter()
        .enumerate()
        .flat_map(|(index, (line_end, arc_end, center))| {
            let end = if index == 3 {
                json!("start")
            } else {
                json!([arc_end.0, arc_end.1])
            };
            [
                json!({"line": [line_end.0, line_end.1], "tangent": true}),
                json!({"arc": end, "center": [center.0, center.1], "tangent": true}),
            ]
        })
        .collect();
    let path = drawn_and_extruded(&mut one, "mm", json!([r, 0.0]), json!(segments), false, 3.0);

    assert_eq!(path["dof"], last["dof"]);
    assert_eq!(path["constraints"].as_array().unwrap().len(), 8);
    let (by_hand, by_path) = (shapes(&mut hand, "P"), shapes(&mut one, "P"));
    assert_eq!(by_hand.len(), 8);
    assert_eq!(by_hand.len(), by_path.len());
    for (a, b) in by_hand.iter().zip(&by_path) {
        assert_eq!(a.0, b.0);
        assert_eq!(a.1.len(), b.1.len(), "{a:?} {b:?}");
        for (p, q) in a.1.iter().zip(&b.1) {
            assert_at(*p, *q, 1.0, &a.0);
        }
    }
    let (a, b) = (body(&mut hand, "Body"), body(&mut one, "Body"));
    for key in ["kind", "valid", "faces", "edges", "vertices"] {
        assert_eq!(a[key], b[key], "{key}");
    }
    for key in ["volume", "area"] {
        let (x, y) = (a[key].as_f64().unwrap(), b[key].as_f64().unwrap());
        assert!((x - y).abs() <= 1e-9 * x, "{key}: {x} and {y}");
    }
    for axis in 0..3 {
        let (x, y) = (
            a["centroid"][axis].as_f64().unwrap(),
            b["centroid"][axis].as_f64().unwrap(),
        );
        assert!((x - y).abs() <= 1e-9 * w, "centroid: {x} and {y}");
    }
    assert_volume(&mut one, (w * h - (4.0 - PI) * r * r) * 3.0, "by path");

    // The same outline walked the other way round, every arc clockwise,
    // is the same solid.
    let mut back = session();
    let mut reversed = Vec::new();
    for (index, (line_end, arc_end, center)) in corners.iter().enumerate().rev() {
        // The arc of this corner is walked from its end to the line's end,
        // then the line back to where the corner before it ended.
        reversed.push(
            json!({"arc": [line_end.0, line_end.1], "center": [center.0, center.1], "clockwise": true}),
        );
        let _ = arc_end;
        reversed.push(if index == 0 {
            json!({"line": "start"})
        } else {
            let before = corners[index - 1].1;
            json!({"line": [before.0, before.1]})
        });
    }
    drawn_and_extruded(
        &mut back,
        "mm",
        json!([r, 0.0]),
        json!(reversed),
        false,
        3.0,
    );
    assert_volume(&mut back, (w * h - (4.0 - PI) * r * r) * 3.0, "backwards");
}

#[test]
fn a_path_joins_points_that_exist_and_can_be_construction_geometry() {
    let mut s = session();
    run(&mut s, "file.new", json!({}));
    run(&mut s, "sketch.create", json!({"name": "P"}));
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "P", "points": [[0, 0], [10, 0]]}),
    );
    // From one end of the line over the top to the other: a closed D
    // without `closed`, and without a new corner.
    let path = run(
        &mut s,
        "sketch.path",
        json!({"sketch": "P", "start": "e2", "segments": [{"arc": 1, "radius": 5}]}),
    );
    assert_eq!(
        path,
        json!({"id": "F1", "name": "P", "status": "ok", "dof": 5,
               "points": ["e2", "e1"], "entities": ["e5"], "centers": ["e4"],
               "constraints": []})
    );
    let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "P"}));
    assert_at(xy(&inspected, "e4"), (5.0, 0.0), 1.0, "centre");
    let arc = entity(&inspected, "e5");
    assert_eq!((&arc["start"], &arc["end"]), (&json!("e2"), &json!("e1")));
    run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "P", "distance": 2, "name": "Body"}),
    );
    assert_volume(&mut s, PI * 25.0 / 2.0 * 2.0, "half disc on a line");

    // Construction geometry: every segment and the closing line.
    let path = run(
        &mut s,
        "sketch.path",
        json!({"sketch": "P", "start": [20, 0], "construction": true, "closed": true,
               "segments": [{"line": [30, 0]}, {"arc": [30, 10], "through": [35, 5]}]}),
    );
    let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "P"}));
    let entities = path["entities"].as_array().unwrap();
    assert_eq!(entities.len(), 3);
    for id in entities {
        assert_eq!(entity(&inspected, text(id))["construction"], true, "{id}");
    }
    // It adds no profile: the solid is what it was.
    assert_volume(&mut s, PI * 25.0 / 2.0 * 2.0, "with construction geometry");
}

#[test]
fn one_undo_removes_a_path_and_a_saved_one_reopens_as_the_same_body() {
    let dir = workdir("undo-save");
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "sketch.create", json!({"name": "P"}));
    let empty = run(&mut s, "sketch.inspect", json!({"sketch": "P"}));
    // A keyhole: a round head on a straight shank.
    let path = run(
        &mut s,
        "sketch.path",
        json!({"sketch": "P", "start": [-2, 0], "closed": true, "segments": [
            {"line": [2, 0]},
            {"line": [2, 10]},
            {"arc": [-2, 10], "radius": 5, "large": true},
        ]}),
    );
    assert_eq!(path["status"], "ok");
    let drawn = run(&mut s, "sketch.inspect", json!({"sketch": "P"}));
    assert_eq!(drawn["entities"].as_array().unwrap().len(), 9);

    // One step, whatever the number of segments.
    let undone = run(&mut s, "edit.undo", json!({}));
    assert_eq!(undone["undone"], "Add path", "{undone}");
    assert_eq!(run(&mut s, "sketch.inspect", json!({"sketch": "P"})), empty);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(run(&mut s, "sketch.inspect", json!({"sketch": "P"})), drawn);

    run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "P", "distance": 4, "name": "Body"}),
    );
    // The head is a disc of radius 5 without the segment below the chord
    // of 4 at its bottom (half angle a, sin a = 2/5); the shank reaches up
    // to that chord, sqrt(21) under the centre.
    let a = 0.4_f64.asin();
    let below = 21.0_f64.sqrt();
    let head = PI * 25.0 - 25.0 * (a - 0.4 * below / 5.0);
    let area = head + 4.0 * 10.0;
    let before = assert_volume(&mut s, area * 4.0, "keyhole");
    let sketch_before = run(&mut s, "sketch.inspect", json!({"sketch": "P"}));

    run(&mut s, "file.save", json!({"path": "keyhole.3dcraft"}));
    run(&mut s, "file.close", json!({}));
    run(&mut s, "file.open", json!({"path": "keyhole.3dcraft"}));
    assert_eq!(body(&mut s, "Body"), before);
    assert_eq!(
        run(&mut s, "sketch.inspect", json!({"sketch": "P"})),
        sketch_before
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A sketch `P` with a point e1 at the origin, a point e2 at (10, 0) and
/// the line e3 between them, extruded nowhere: something to leave alone.
fn with_a_line(s: &mut Session) {
    run(s, "file.new", json!({"units": "mm"}));
    run(s, "sketch.create", json!({"name": "P"}));
    run(
        s,
        "sketch.polyline",
        json!({"sketch": "P", "points": [[0, 0], [10, 0]]}),
    );
}

/// The number after `before` in `text`, up to the next space, comma or
/// closing bracket.
fn number_after(text: &str, before: &str) -> f64 {
    let at = text
        .find(before)
        .unwrap_or_else(|| panic!("no `{before}` in {text}"))
        + before.len();
    let rest = &text[at..];
    let end = rest.find([' ', ',', ')']).unwrap_or(rest.len());
    rest[..end]
        .parse()
        .unwrap_or_else(|e| panic!("`{}` in {text}: {e}", &rest[..end]))
}

/// An arc given by its centre whose end is not on the circle through its
/// start (an end rounded to a few decimals, as in the turbofan's tube).
/// Given as coordinates and off by up to a percent of the radius, the end
/// is put on the circle, in its direction from the centre: the result
/// says so under `snapped` (the segment, the end as given, where it went
/// and how far), has no warning, and the corner the arc shares with the
/// line before it stays where it was put, as does the arc's end. An end
/// that is a point id (another segment may share it) or one farther off
/// is drawn as given, with a warning that names the segment, says how far
/// off the end is and what the solver does, and gives the end that keeps
/// the start in place; drawn there, the path has neither. An end within
/// the tolerance has neither. At three scales, with the tolerance scaled
/// along.
#[test]
fn an_arc_end_off_its_circle_is_snapped_or_reported_with_the_distance() {
    for k in [1e-3, 1.0, 1e6] {
        let tolerance = 1e-6 * k;
        let (center, radius) = ((72.0 * k, 16.0 * k), 22.0 * k);
        // The point of the circle in the direction (-18, 12.6) from the
        // centre, `off` farther out.
        let toward = (-18.0_f64, 12.6_f64);
        let length = toward.0.hypot(toward.1);
        let end = |off: f64| {
            let reach = radius + off;
            json!([
                center.0 + toward.0 / length * reach,
                center.1 + toward.1 / length * reach
            ])
        };
        let on_circle = {
            let on = end(0.0);
            (on[0].as_f64().unwrap(), on[1].as_f64().unwrap())
        };
        // The arc's end given as `end`; a point id is drawn first, as e1.
        let draw = |s: &mut Session, end: Value, as_point: bool| {
            run(s, "file.new", json!({"tolerance": tolerance}));
            run(s, "sketch.create", json!({"name": "P"}));
            let end = if as_point {
                let made = run(s, "sketch.point", json!({"sketch": "P", "at": end}));
                assert_eq!(made["status"], "ok", "{made}");
                json!("e1")
            } else {
                end
            };
            run(
                s,
                "sketch.path",
                json!({"sketch": "P", "start": [50.0 * k, 10.0 * k], "segments": [
                    {"line": [50.0 * k, 16.0 * k]},
                    {"arc": end, "center": [center.0, center.1], "clockwise": true},
                    {"line": [62.0 * k, 40.0 * k]},
                ]}),
            )
        };
        let mut s = session();

        // Rounding: put on the circle.
        for off in [1e-4 * k, -3e-5 * k, 0.009 * radius] {
            let drawn = draw(&mut s, end(off), false);
            assert_eq!(drawn["status"], "ok", "{drawn}");
            assert!(drawn.get("warnings").is_none(), "{drawn}");
            let snapped = drawn["snapped"].as_array().unwrap();
            assert_eq!(snapped.len(), 1, "{drawn}");
            let snap = &snapped[0];
            assert_eq!(snap["segment"], 1, "{drawn}");
            assert_eq!(snap["given"], end(off), "{drawn}");
            let moved = snap["moved"].as_f64().unwrap();
            assert!(
                (moved - off.abs()).abs() <= 1e-6 * off.abs(),
                "{k}: moved {moved} for {off}"
            );
            let put = (
                snap["end"][0].as_f64().unwrap(),
                snap["end"][1].as_f64().unwrap(),
            );
            assert_at(put, on_circle, k, "the snapped end");
            let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "P"}));
            assert_at(xy(&inspected, "e2"), (50.0 * k, 16.0 * k), k, "the corner");
            let arc_end = drawn["points"][2].as_str().unwrap().to_string();
            assert_at(xy(&inspected, &arc_end), on_circle, k, "the arc's end");
        }

        // A point id, or an end too far off: drawn as given, and warned
        // about.
        for (off, as_point) in [(1e-4 * k, true), (-3e-5 * k, true), (0.02 * radius, false)] {
            let drawn = draw(&mut s, end(off), as_point);
            assert_eq!(drawn["status"], "ok", "{drawn}");
            assert!(drawn.get("snapped").is_none(), "{drawn}");
            let warnings = drawn["warnings"].as_array().unwrap();
            assert_eq!(warnings.len(), 1, "{drawn}");
            let warning = text(&warnings[0]);
            assert!(
                warning.starts_with("segments[1]: the arc's end ("),
                "{warning}"
            );
            assert!(
                warning.contains(if off > 0.0 { " outside " } else { " inside " })
                    && warning.contains("the solver moves the start, the end and the centre"),
                "{warning}"
            );
            let distance = number_after(warning, ") is ");
            assert!(
                (distance - off.abs()).abs() <= 1e-6 * off.abs(),
                "{k}: {distance} for {off}"
            );
            let at = &warning[warning.find("end the arc at (").unwrap()..];
            let on = (number_after(at, "at ("), number_after(at, ", "));
            assert_at(on, on_circle, k, "the end on the circle");

            // Drawn at that end: nothing to report, and the start of the
            // arc stays where the line put it.
            let drawn = draw(&mut s, json!([on.0, on.1]), as_point);
            assert_eq!(drawn["status"], "ok", "{drawn}");
            assert!(drawn.get("warnings").is_none(), "{drawn}");
            assert!(drawn.get("snapped").is_none(), "{drawn}");
            let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "P"}));
            let corner = if as_point { "e3" } else { "e2" };
            assert_at(
                xy(&inspected, corner),
                (50.0 * k, 16.0 * k),
                k,
                "the corner",
            );
        }

        // Within the tolerance: nothing to report.
        let drawn = draw(&mut s, end(0.5 * tolerance), false);
        assert_eq!(drawn["status"], "ok", "{drawn}");
        assert!(drawn.get("warnings").is_none(), "{drawn}");
        assert!(drawn.get("snapped").is_none(), "{drawn}");
    }
}

#[test]
fn bad_paths_are_refused_with_a_reason_and_change_nothing() {
    let mut s = session();
    with_a_line(&mut s);
    let before = run(&mut s, "document.inspect", json!({}));
    let sketch_before = run(&mut s, "sketch.inspect", json!({"sketch": "P"}));
    let line = json!([{"line": [5, 5]}]);
    let many: Vec<Value> = (0..1001).map(|i| json!({"line": [i, 1]})).collect();
    let cases: Vec<(Value, &str)> = vec![
        // The parameters themselves.
        (json!({"start": [0, 0], "segments": line}), "`sketch`"),
        (
            json!({"sketch": "Nope", "start": [0, 0], "segments": line}),
            "Nope",
        ),
        (json!({"sketch": "P", "segments": line}), "`start`"),
        (json!({"sketch": "P", "start": [0, 0]}), "`segments`"),
        (
            json!({"sketch": "P", "start": [0, 0], "segments": []}),
            "at least one segment",
        ),
        (
            json!({"sketch": "P", "start": [0, 0], "segments": "line"}),
            "must be an array",
        ),
        (
            json!({"sketch": "P", "start": [0, 0], "segments": many}),
            "at most 1000",
        ),
        (
            json!({"sketch": "P", "start": [0, 0], "segments": line, "close": true}),
            "unknown parameter `close`",
        ),
        (
            json!({"sketch": "P", "start": [0, 0], "segments": line, "closed": "yes"}),
            "`closed`",
        ),
        (
            json!({"sketch": "P", "start": [0, 0], "segments": line, "construction": 1}),
            "`construction`",
        ),
        (
            json!({"sketch": "P", "start": [0], "segments": line}),
            "`start`",
        ),
        (
            json!({"sketch": "P", "start": "start", "segments": line}),
            "`start`",
        ),
        // Points that do not exist, or are not points.
        (
            json!({"sketch": "P", "start": "e99", "segments": line}),
            "no entity e99",
        ),
        (
            json!({"sketch": "P", "start": "e3", "segments": line}),
            "e3 is a line",
        ),
        (
            json!({"sketch": "P", "start": [0, 5], "segments": [{"line": [5, 5]}, {"line": "e99"}]}),
            "segments[1]: the sketch has no entity e99",
        ),
        (
            json!({"sketch": "P", "start": [0, 5], "segments": [{"arc": [5, 5], "center": "e99"}]}),
            "segments[0]: the sketch has no entity e99",
        ),
        (
            json!({"sketch": "P", "start": [0, 5], "segments": [{"arc": [5, 5], "center": "e3"}]}),
            "segments[0]: e3 is a line",
        ),
        // The shape of a segment.
        (
            json!({"sketch": "P", "start": [0, 5], "segments": [[5, 5]]}),
            "segments[0] must be an object",
        ),
        (
            json!({"sketch": "P", "start": [0, 5], "segments": [{"line": [5, 5]}, {}]}),
            "segments[1] needs `line` or `arc`",
        ),
        (
            json!({"sketch": "P", "start": [0, 5], "segments": [{"line": [5, 5], "arc": [6, 6], "center": [5, 6]}]}),
            "segments[0] has both `line` and `arc`",
        ),
        (
            json!({"sketch": "P", "start": [0, 5], "segments": [{"line": [5, 5], "bulge": 1}]}),
            "segments[0]: unknown key `bulge`",
        ),
        (
            json!({"sketch": "P", "start": [0, 5], "segments": [{"line": [5, 5], "center": [1, 1]}]}),
            "segments[0]: `center` belongs to an arc",
        ),
        (
            json!({"sketch": "P", "start": [0, 5], "segments": [{"line": [5, 5], "clockwise": true}]}),
            "segments[0]: `clockwise` belongs to an arc",
        ),
        (
            json!({"sketch": "P", "start": [0, 5], "segments": [{"arc": [5, 5]}]}),
            "segments[0]: an arc needs",
        ),
        (
            json!({"sketch": "P", "start": [0, 5], "segments": [{"arc": [5, 5], "tangent": false}]}),
            "segments[0]: an arc needs",
        ),
        (
            json!({"sketch": "P", "start": [0, 5], "segments": [{"arc": [5, 5], "center": [2, 5], "radius": 3}]}),
            "segments[0]: give one of",
        ),
        (
            json!({"sketch": "P", "start": [0, 5], "segments": [{"arc": [5, 5], "center": [2.5, 5], "large": true}]}),
            "segments[0]: `large`",
        ),
        (
            json!({"sketch": "P", "start": [0, 5], "segments": [{"arc": [5, 5], "through": [2.5, 7], "clockwise": true}]}),
            "segments[0]: `clockwise`",
        ),
        // Wrong types.
        (
            json!({"sketch": "P", "start": [0, 5], "segments": [{"line": "there"}]}),
            "`segments[0].line` must be an id",
        ),
        (
            json!({"sketch": "P", "start": [0, 5], "segments": [{"line": [5]}]}),
            "`segments[0].line`",
        ),
        (
            json!({"sketch": "P", "start": [0, 5], "segments": [{"line": true}]}),
            "segments[0]: `line` must be the point",
        ),
        (
            json!({"sketch": "P", "start": [0, 5], "segments": [{"line": {"x": 1}}]}),
            "segments[0]: `line` must be the point",
        ),
        (
            json!({"sketch": "P", "start": [0, 5], "segments": [{"arc": [5, 5], "center": [1, "a"]}]}),
            "`segments[0].center`",
        ),
        (
            json!({"sketch": "P", "start": [0, 5], "segments": [{"arc": [5, 5], "through": "e1"}]}),
            "segments[0]: `through` must be two finite numbers",
        ),
        (
            json!({"sketch": "P", "start": [0, 5], "segments": [{"arc": [5, 5], "radius": "5"}]}),
            "segments[0]: `radius` must be a positive number",
        ),
        (
            json!({"sketch": "P", "start": [0, 5], "segments": [{"arc": [5, 5], "radius": 0}]}),
            "segments[0]: `radius` must be a positive number",
        ),
        (
            json!({"sketch": "P", "start": [0, 5], "segments": [{"arc": [5, 5], "radius": -4}]}),
            "segments[0]: `radius` must be a positive number",
        ),
        (
            json!({"sketch": "P", "start": [0, 5], "segments": [{"line": [5, 5]}, {"arc": [9, 9], "tangent": "yes"}]}),
            "segments[1]: `tangent` must be true or false",
        ),
        (
            json!({"sketch": "P", "start": [0, 5], "segments": [{"arc": [5, 5], "radius": 5, "large": 1}]}),
            "segments[0]: `large` must be true or false",
        ),
        // Arcs that do not exist.
        (
            json!({"sketch": "P", "start": [0, 5], "segments": [{"line": [1, 5]}, {"arc": [5, 5], "through": [3, 5]}]}),
            "segments[1]: the `through` point is on the line",
        ),
        (
            json!({"sketch": "P", "start": [0, 5], "segments": [{"arc": [5, 5], "through": [5, 5]}]}),
            "segments[0]: the `through` point is on the line",
        ),
        (
            json!({"sketch": "P", "start": [0, 5], "segments": [{"arc": [5, 5], "radius": 2.4}]}),
            "segments[0]: a radius of 2.4 cannot span the 5",
        ),
        (
            json!({"sketch": "P", "start": [0, 5], "segments": [{"arc": [5, 5], "center": [0, 5]}]}),
            "segments[0]: the arc's centre is on its start",
        ),
        (
            json!({"sketch": "P", "start": [0, 5], "segments": [{"arc": [5, 5], "center": [5, 5]}]}),
            "segments[0]: the arc's centre is on its end",
        ),
        (
            json!({"sketch": "P", "start": "e1", "segments": [{"arc": [5, 5], "center": "e1"}]}),
            "segments[0]: the arc's centre is on its start",
        ),
        (
            json!({"sketch": "P", "start": [0, 5], "segments": [{"arc": [5, 5], "tangent": true}]}),
            "segments[0]: an arc given by `tangent` alone needs a segment before it",
        ),
        (
            json!({"sketch": "P", "start": [0, 5], "segments": [{"line": [5, 5]}, {"arc": [9, 5], "tangent": true}]}),
            "segments[1]: the end is on the line along which",
        ),
        (
            json!({"sketch": "P", "start": [0, 5], "segments": [{"line": [5, 5]}, {"arc": [2, 5], "tangent": true}]}),
            "segments[1]: the end is on the line along which",
        ),
        // Segments without length.
        (
            json!({"sketch": "P", "start": [0, 5], "segments": [{"line": [0, 5]}]}),
            "segments[0]: it ends where it starts",
        ),
        (
            json!({"sketch": "P", "start": "e1", "segments": [{"line": [5, 5]}, {"line": "e2"}, {"arc": "e2", "radius": 4}]}),
            "segments[2]: it ends where it starts",
        ),
        (
            json!({"sketch": "P", "start": [0, 5], "segments": [{"line": "start"}]}),
            "segments[0]: it ends where it starts",
        ),
        (
            json!({"sketch": "P", "start": [0, 5], "segments": [{"line": [5, 5]}, {"line": [5, 5.0000000001]}]}),
            "segments[1]: it ends where it starts",
        ),
        (
            json!({"sketch": "P", "start": [-1e308, 0], "segments": [{"line": [1e308, 0]}]}),
            "segments[0]: its ends are too far apart",
        ),
        // Tangent between two lines, and tangent to nothing.
        (
            json!({"sketch": "P", "start": [0, 5], "segments": [{"line": [5, 5]}, {"line": [9, 9], "tangent": true}]}),
            "segments[1]: a tangent constraint takes",
        ),
        (
            json!({"sketch": "P", "start": [0, 5], "segments": [{"line": [5, 5], "tangent": true}, {"arc": [9, 9], "tangent": true}]}),
            "segments[0]: `tangent` on the first segment is with the last one",
        ),
        // Closing what is closed already.
        (
            json!({"sketch": "P", "start": [0, 5], "closed": true,
                   "segments": [{"line": [5, 5]}, {"line": [5, 9]}, {"line": "start"}]}),
            "the path already ends at its start",
        ),
        (
            json!({"sketch": "P", "start": [0, 5], "closed": true,
                   "segments": [{"line": [5, 5]}, {"line": [5, 9]}, {"line": [0, 5]}]}),
            "`closed` would add a line of no length",
        ),
        (
            json!({"sketch": "P", "start": [0, 5], "closed": true, "segments": [{"line": [5, 5]}]}),
            "`closed` would add a second line on top of the only one",
        ),
    ];
    for (params, expected) in cases {
        let message = error(&mut s, "sketch.path", params.clone());
        assert!(
            message.starts_with("sketch.path: ") && message.contains(expected),
            "{params}: `{message}` should mention `{expected}`"
        );
        // Not a point, not an undo step, not a revision: nothing.
        assert_eq!(
            run(&mut s, "document.inspect", json!({})),
            before,
            "{params}"
        );
    }
    assert_eq!(
        run(&mut s, "sketch.inspect", json!({"sketch": "P"})),
        sketch_before
    );
    // The last undo step is still the line that was there.
    assert_eq!(
        run(&mut s, "edit.undo", json!({}))["undone"],
        "Add polyline"
    );
    // Without a document there is nothing to draw in.
    let mut none = session();
    error(
        &mut none,
        "sketch.path",
        json!({"sketch": "P", "start": [0, 0], "segments": [{"line": [1, 1]}]}),
    );
}

/// `panic_hunt` gives every command adversarial values for its
/// parameters, but not inside a list of segments: this does.
#[test]
fn no_segment_makes_the_command_panic() {
    let mut deep = json!(1);
    for _ in 0..200 {
        deep = json!([deep]);
    }
    let values = vec![
        Value::Null,
        json!(true),
        json!(false),
        json!(0),
        json!(-1),
        json!(1),
        json!(0.5),
        json!(-0.0),
        json!(1e308),
        json!(-1e308),
        json!(5e-324),
        json!(u64::MAX),
        json!(i64::MIN),
        json!(""),
        json!("start"),
        json!("e1"),
        json!("e2"),
        json!("e3"),
        json!("e4"),
        json!("e5"),
        json!("e0"),
        json!("e18446744073709551615"),
        json!("c1"),
        json!("💥"),
        json!("9".repeat(10_000)),
        json!([]),
        json!([1]),
        json!([0, 0]),
        json!([10, 0]),
        json!([5, 0]),
        json!([5, 5]),
        json!([5, 1e-7]),
        json!([5, 1e-300]),
        json!([1e308, 1e308]),
        json!([-1e308, 1e308]),
        json!([1e154, 1e154]),
        json!([5e-324, 5e-324]),
        json!([1, 2, 3]),
        json!(["a", "b"]),
        json!([null, null]),
        json!({}),
        json!({"line": [1, 1]}),
        deep,
    ];
    let mut template = session();
    with_a_line(&mut template);
    // An arc as well: e4 its centre, e5 the arc from e2 to e1.
    run(
        &mut template,
        "sketch.path",
        json!({"sketch": "P", "start": "e2", "segments": [{"arc": "e1", "radius": 5}]}),
    );
    let check = |params: Value| {
        let mut s = template.clone();
        match s.execute("sketch.path", &params) {
            Ok(_) => assert_ne!(s.regeneration_matches_a_full_one(), Some(false), "{params}"),
            Err(e) => assert!(!e.message().contains("internal error"), "{params}: {e}"),
        }
    };
    let keys = [
        "line",
        "arc",
        "center",
        "through",
        "radius",
        "large",
        "clockwise",
        "tangent",
    ];
    let starts = [json!([0, 0]), json!("e1"), json!([1e308, -1e308])];
    for value in &values {
        check(json!({"sketch": "P", "start": value, "segments": [{"line": [3, 4]}]}));
        check(json!({"sketch": "P", "start": [0, 0], "segments": value}));
        for start in &starts {
            for closed in [false, true] {
                // The value as a whole segment, and as each key of one:
                // alone, with a valid line or arc before it, and with
                // every key at once.
                check(json!({"sketch": "P", "start": start, "closed": closed,
                    "segments": [{"line": [3, 4]}, value]}));
                let mut all = serde_json::Map::new();
                for key in keys {
                    all.insert(key.to_string(), value.clone());
                    for with in [
                        json!({}),
                        json!({"line": [7, 1]}),
                        json!({"arc": [7, 1]}),
                        json!({"arc": "start"}),
                        json!({"arc": [7, 1], "tangent": true}),
                    ] {
                        let mut segment = with.as_object().unwrap().clone();
                        segment.insert(key.to_string(), value.clone());
                        check(json!({"sketch": "P", "start": start, "closed": closed,
                            "segments": [segment.clone()]}));
                        check(json!({"sketch": "P", "start": start, "closed": closed,
                            "segments": [{"line": [3, 4]}, segment.clone(), {"line": [-2, 9]}]}));
                        check(json!({"sketch": "P", "start": start, "closed": closed,
                            "segments": [{"arc": [3, 4], "radius": 9, "tangent": true}, segment]}));
                    }
                }
                check(json!({"sketch": "P", "start": start, "closed": closed,
                    "segments": [Value::Object(all)]}));
            }
        }
    }
}
