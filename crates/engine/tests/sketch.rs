//! Sketches through the commands, as an agent uses them: draw roughly,
//! constrain, read back coordinates that were computed by hand.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{error, run, session, session_in};
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::path::PathBuf;

fn workdir(name: &str) -> PathBuf {
    common::workdir("sketch", name)
}

/// The solver stops when every constraint holds within the document's
/// linear tolerance (1e-6 mm by default). In these well-conditioned
/// sketches a coordinate is then within a few tolerances of the exact
/// value; ten is the bound used.
const NEAR: f64 = 1e-5;

/// The solved coordinates of a point, from `sketch.inspect`.
fn point(session: &mut Session, sketch: &str, id: &str) -> (f64, f64) {
    let inspected = run(session, "sketch.inspect", json!({"sketch": sketch}));
    let entity = inspected["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == id)
        .unwrap_or_else(|| panic!("no entity {id}"));
    (entity["x"].as_f64().unwrap(), entity["y"].as_f64().unwrap())
}

fn assert_points(session: &mut Session, sketch: &str, expected: &[(&str, f64, f64)]) {
    for (id, x, y) in expected {
        let (sx, sy) = point(session, sketch, id);
        assert!(
            (sx - x).abs() <= NEAR && (sy - y).abs() <= NEAR,
            "{id} is at ({sx}, {sy}), expected ({x}, {y})"
        );
    }
}

fn constrain(session: &mut Session, kind: &str, entities: Value, value: Option<&str>) -> Value {
    let mut params = json!({"sketch": "Profile", "type": kind, "entities": entities});
    if let Some(value) = value {
        params["value"] = value.into();
    }
    run(session, "sketch.constrain", params)
}

/// The phase-1 acceptance shape: an L drawn by eye, then constrained.
///
/// ```text
///  e6 +--+ e5            points e1..e6, counter-clockwise from the origin
///     |  |               lines  e7 (e1-e2) .. e12 (e6-e1)
///     |  +------+ e3
///     |  e4     |        width = 40, height = 30, thickness = 5
///  e1 +---------+ e2
/// ```
fn l_profile(session: &mut Session) {
    run(session, "file.new", json!({"units": "mm"}));
    for (name, expr) in [
        ("width", "40 mm"),
        ("height", "30 mm"),
        ("thickness", "5 mm"),
    ] {
        run(session, "variable.set", json!({"name": name, "expr": expr}));
    }
    let created = run(session, "sketch.create", json!({"name": "Profile"}));
    assert_eq!(created["status"], "ok");
    assert_eq!(created["dof"], 0, "an empty sketch has nothing to move");
    let drawn = run(
        session,
        "sketch.polyline",
        json!({"sketch": "Profile", "closed": true,
               "points": [[0, 0], [38, 1.5], [41, 6], [4, 4.5], [6.5, 28], [-1, 31]]}),
    );
    assert_eq!(drawn["points"], json!(["e1", "e2", "e3", "e4", "e5", "e6"]));
    assert_eq!(
        drawn["lines"],
        json!(["e7", "e8", "e9", "e10", "e11", "e12"])
    );
    // Six free points: two coordinates each.
    assert_eq!(drawn["dof"], 12);
    assert_eq!(constrain(session, "fixed", json!(["e1"]), None)["dof"], 10);
    // One equation each, all independent: 10 - 6.
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
    let boxed = run(session, "sketch.inspect", json!({"sketch": "Profile"}));
    assert_eq!(boxed["dof"], 4);
    // What is left is the four lengths below.
    constrain(session, "distance", json!(["e7"]), Some("width"));
    constrain(session, "distance", json!(["e12"]), Some("height"));
    constrain(session, "distance", json!(["e8"]), Some("thickness"));
    let done = constrain(session, "distance", json!(["e11"]), Some("thickness"));
    assert_eq!(done["status"], "ok", "{done}");
    assert_eq!(done["dof"], 0);
    assert_eq!(done["constraint"], "c11");
    assert!(done.get("conflicting").is_none() && done.get("redundant").is_none());
}

const L_40: [(&str, f64, f64); 6] = [
    ("e1", 0.0, 0.0),
    ("e2", 40.0, 0.0),
    ("e3", 40.0, 5.0),
    ("e4", 5.0, 5.0),
    ("e5", 5.0, 30.0),
    ("e6", 0.0, 30.0),
];

#[test]
fn an_l_profile_drawn_by_eye_solves_to_its_dimensions() {
    let mut s = session();
    l_profile(&mut s);
    assert_points(&mut s, "Profile", &L_40);
    let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "Profile"}));
    assert_eq!(inspected["lengthUnit"], "mm");
    assert_eq!(inspected["entities"].as_array().unwrap().len(), 12);
    let constraints = inspected["constraints"].as_array().unwrap();
    assert_eq!(constraints.len(), 11);
    for c in constraints {
        assert_eq!(c["state"], "satisfied", "{c}");
        assert!(c["residual"].as_f64().unwrap() <= 1e-6, "{c}");
    }
    assert_eq!(
        constraints[7],
        json!({"id": "c8", "type": "distance", "entities": ["e7"], "value": "width",
               "state": "satisfied", "residual": constraints[7]["residual"]})
    );
    // The document lists the sketch with its degrees of freedom, and it has
    // no body.
    let document = run(&mut s, "document.inspect", json!({}));
    assert_eq!(document["features"][0]["name"], "Profile");
    assert_eq!(document["features"][0]["status"], "ok");
    assert_eq!(document["features"][0]["dof"], 0);
    assert_eq!(document["bodies"], json!([]));
    assert_eq!(document["ok"], true);
    // The solution is what the document now holds (and saves), not the
    // rough drawing: the next solve starts from it.
    let stored = &document["features"][0]["params"]["entities"];
    for (i, (id, x, y)) in L_40.iter().enumerate() {
        let geometry = &stored[i]["geometry"];
        let (sx, sy) = (
            geometry["x"].as_f64().unwrap(),
            geometry["y"].as_f64().unwrap(),
        );
        assert!(
            (sx - x).abs() <= NEAR && (sy - y).abs() <= NEAR,
            "{id} is stored at ({sx}, {sy})"
        );
    }
}

#[test]
fn changing_a_variable_moves_the_sketch_without_flipping_it() {
    let mut s = session();
    l_profile(&mut s);
    run(
        &mut s,
        "variable.set",
        json!({"name": "width", "expr": "60 mm"}),
    );
    // Only the right-hand side moves: x of e2 and e3.
    assert_points(
        &mut s,
        "Profile",
        &[
            ("e1", 0.0, 0.0),
            ("e2", 60.0, 0.0),
            ("e3", 60.0, 5.0),
            ("e4", 5.0, 5.0),
            ("e5", 5.0, 30.0),
            ("e6", 0.0, 30.0),
        ],
    );
    // A large change in several steps, and back: the L never mirrors.
    for width in [200.0, 6.0, 1e4, 40.0] {
        run(
            &mut s,
            "variable.set",
            json!({"name": "width", "expr": format!("{width} mm")}),
        );
        assert_points(
            &mut s,
            "Profile",
            &[("e2", width, 0.0), ("e3", width, 5.0), ("e5", 5.0, 30.0)],
        );
    }
    // The dimension itself can be replaced by another expression.
    let edited = run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "Profile", "constraint": "c9", "value": "height / 2 + 1 cm"}),
    );
    assert_eq!(
        (&edited["status"], &edited["dof"]),
        (&json!("ok"), &json!(0))
    );
    assert_points(&mut s, "Profile", &[("e5", 5.0, 25.0), ("e6", 0.0, 25.0)]);
    // One undo step per command, the solution included.
    run(&mut s, "edit.undo", json!({}));
    assert_points(&mut s, "Profile", &L_40);
    run(&mut s, "edit.redo", json!({}));
    assert_points(&mut s, "Profile", &[("e6", 0.0, 25.0)]);
}

#[test]
fn a_solved_sketch_does_not_move_when_it_is_solved_again() {
    let mut s = session();
    l_profile(&mut s);
    let before = run(&mut s, "sketch.inspect", json!({"sketch": "Profile"}));
    // An edit that has nothing to do with the sketch re-solves it.
    run(
        &mut s,
        "variable.set",
        json!({"name": "unrelated", "expr": "1"}),
    );
    run(&mut s, "feature.sphere", json!({"radius": 3}));
    let after = run(&mut s, "sketch.inspect", json!({"sketch": "Profile"}));
    assert_eq!(after["entities"], before["entities"], "bit for bit");
}

#[test]
fn a_saved_sketch_reopens_as_it_was() {
    let dir = workdir("save");
    let mut s = session_in(&dir);
    l_profile(&mut s);
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Profile", "center": [20, 2], "radius": 1.25}),
    );
    let before = run(&mut s, "sketch.inspect", json!({"sketch": "Profile"}));
    run(&mut s, "file.save", json!({"path": "l.3dcraft"}));
    run(&mut s, "file.close", json!({}));
    run(&mut s, "file.open", json!({"path": "l.3dcraft"}));
    let after = run(&mut s, "sketch.inspect", json!({"sketch": "Profile"}));
    assert_eq!(after, before);
    // And it is still driven by its variables.
    run(
        &mut s,
        "variable.set",
        json!({"name": "height", "expr": "50 mm"}),
    );
    assert_points(
        &mut s,
        "Profile",
        &[("e5", 5.0, 50.0), ("e6", 0.0, 50.0), ("e3", 40.0, 5.0)],
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn redundant_and_conflicting_constraints_are_named() {
    let mut s = session();
    l_profile(&mut s);
    // The bottom and the inner step are both horizontal already.
    let parallel = constrain(&mut s, "parallel", json!(["e7", "e9"]), None);
    assert_eq!(parallel["status"], "ok", "{parallel}");
    assert_eq!(parallel["redundant"], json!(["c12"]));
    assert_eq!(parallel["dof"], 0);
    assert_points(&mut s, "Profile", &L_40);
    // The inner step e9 is width - thickness = 35 long; 10 cannot hold.
    let clash = constrain(&mut s, "distance", json!(["e9"]), Some("10 mm"));
    assert_eq!(clash["status"], "failed", "{clash}");
    assert_eq!(clash["conflicting"], json!(["c13"]));
    assert!(clash["error"].as_str().unwrap().contains("c13"), "{clash}");
    // The rest of the sketch is still where it belongs.
    assert_points(&mut s, "Profile", &L_40);
    // The verdict is the same however often the sketch is solved again:
    // the solution with the conflict left out is shown, but not stored as
    // the next starting point.
    for _ in 0..3 {
        run(
            &mut s,
            "variable.set",
            json!({"name": "unrelated", "expr": "1"}),
        );
        run(&mut s, "variable.delete", json!({"name": "unrelated"}));
        let again = run(&mut s, "sketch.inspect", json!({"sketch": "Profile"}));
        assert_eq!(again["status"], "failed");
        assert_eq!(again["conflicting"], json!(["c13"]), "{again}");
        assert_eq!(again["error"], clash["error"]);
    }
    assert_points(&mut s, "Profile", &L_40);
    let document = run(&mut s, "document.inspect", json!({}));
    assert_eq!(document["ok"], false);
    let check = run(&mut s, "document.check", json!({}));
    assert_eq!(check["ok"], false);
    assert!(
        check["problems"][0]["problem"]
            .as_str()
            .unwrap()
            .contains("c13")
    );
    // Deleting it heals the sketch.
    let healed = run(
        &mut s,
        "sketch.delete",
        json!({"sketch": "Profile", "constraint": "c13"}),
    );
    assert_eq!(
        (&healed["status"], &healed["deleted"]),
        (&json!("ok"), &json!("c13"))
    );
    assert!(healed.get("conflicting").is_none());
}

/// A dimension set to zero collapses a line that an angle is measured
/// from. The sketch fails, keeps failing the same way, and comes back as
/// it was when the dimension is restored: the collapsed state is never
/// stored as the place to start from.
#[test]
fn a_sketch_broken_by_a_variable_recovers_when_the_variable_is_restored() {
    // Drawn to the right and to the left of the fixed point: a collapsed
    // line has no direction, so only the stored shape can say which side
    // it comes back on.
    for side in [1.0, -1.0] {
        let mut s = session();
        run(&mut s, "file.new", json!({}));
        run(&mut s, "variable.set", json!({"name": "w", "expr": "4 mm"}));
        run(&mut s, "sketch.create", json!({"name": "Profile"}));
        // e1-e2 is the base e4, e2-e3 the upright e5.
        run(
            &mut s,
            "sketch.polyline",
            json!({"sketch": "Profile", "points": [[0, 0], [4.0 * side, 0.3], [4.2 * side, 3]]}),
        );
        constrain(&mut s, "fixed", json!(["e1"]), None);
        constrain(&mut s, "horizontal", json!(["e4"]), None);
        constrain(&mut s, "distance", json!(["e4"]), Some("w"));
        // From the base (pointing +x or -x) to the upright (pointing +y),
        // counter-clockwise: a quarter turn, or three quarters.
        let angle = if side > 0.0 { "90 deg" } else { "270 deg" };
        constrain(&mut s, "angle", json!(["e4", "e5"]), Some(angle));
        let done = constrain(&mut s, "distance", json!(["e5"]), Some("3"));
        assert_eq!(
            (&done["status"], &done["dof"]),
            (&json!("ok"), &json!(0)),
            "{done}"
        );
        let upright = [
            ("e1", 0.0, 0.0),
            ("e2", 4.0 * side, 0.0),
            ("e3", 4.0 * side, 3.0),
        ];
        assert_points(&mut s, "Profile", &upright);

        run(&mut s, "variable.set", json!({"name": "w", "expr": "0 mm"}));
        let broken = run(&mut s, "sketch.inspect", json!({"sketch": "Profile"}));
        assert_eq!(broken["status"], "failed", "{broken}");
        assert_eq!(broken["conflicting"], json!(["c4"]), "{broken}");
        let reason = broken["error"].as_str().unwrap();
        assert!(
            reason.contains("c4") && reason.contains("no direction"),
            "{reason}"
        );
        for _ in 0..3 {
            run(
                &mut s,
                "variable.set",
                json!({"name": "unrelated", "expr": "1"}),
            );
            run(&mut s, "variable.delete", json!({"name": "unrelated"}));
            let again = run(&mut s, "sketch.inspect", json!({"sketch": "Profile"}));
            assert_eq!(again["status"], "failed");
            assert_eq!(again["error"], broken["error"]);
        }
        run(&mut s, "variable.set", json!({"name": "w", "expr": "4 mm"}));
        let healed = run(&mut s, "sketch.inspect", json!({"sketch": "Profile"}));
        assert_eq!(
            (&healed["status"], &healed["dof"]),
            (&json!("ok"), &json!(0)),
            "{healed}"
        );
        assert_points(&mut s, "Profile", &upright);
    }
}

#[test]
fn a_dimension_that_cannot_be_evaluated_fails_the_sketch_and_can_be_fixed() {
    let mut s = session();
    l_profile(&mut s);
    let broken = run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "Profile", "constraint": "c8", "value": "nope * 2"}),
    );
    assert_eq!(broken["status"], "failed");
    let reason = broken["error"].as_str().unwrap();
    assert!(
        reason.contains("c8.value") && reason.contains("nope"),
        "{reason}"
    );
    // An angle where a length is needed, and a negative length.
    for (value, word) in [("30 deg", "length"), ("-5 mm", "c8")] {
        let wrong = run(
            &mut s,
            "sketch.edit",
            json!({"sketch": "Profile", "constraint": "c8", "value": value}),
        );
        assert_eq!(wrong["status"], "failed", "{value}: {wrong}");
        assert!(wrong["error"].as_str().unwrap().contains(word), "{wrong}");
    }
    // The stored coordinates did not move meanwhile.
    assert_points(&mut s, "Profile", &L_40);
    let fixed = run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "Profile", "constraint": "c8", "value": "2 * width"}),
    );
    assert_eq!(fixed["status"], "ok");
    assert_points(&mut s, "Profile", &[("e2", 80.0, 0.0)]);
}

#[test]
fn a_circle_tangent_to_a_line_sits_at_its_radius() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "in"}));
    run(
        &mut s,
        "variable.set",
        json!({"name": "hole_d", "expr": "50.8 mm"}),
    );
    run(&mut s, "sketch.create", json!({"name": "Profile"}));
    let base = run(
        &mut s,
        "sketch.line",
        json!({"sketch": "Profile", "from": [0, 0], "to": [5, 0.25]}),
    );
    assert_eq!(
        (&base["entity"], &base["start"], &base["end"]),
        (&json!("e3"), &json!("e1"), &json!("e2"))
    );
    let circle = run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Profile", "center": [2.2, 0.8], "radius": 0.7}),
    );
    assert_eq!(
        (&circle["entity"], &circle["center"]),
        (&json!("e5"), &json!("e4"))
    );
    // Line: 4 unknowns, circle: 2 for the centre and 1 for the radius.
    assert_eq!(circle["dof"], 7);
    constrain(&mut s, "fixed", json!(["e1"]), None);
    constrain(&mut s, "horizontal", json!(["e3"]), None);
    constrain(&mut s, "distance", json!(["e3"]), Some("5 in"));
    // hole_d is 50.8 mm = 2 in, so the radius is 1 in.
    constrain(&mut s, "diameter", json!(["e5"]), Some("hole_d"));
    constrain(&mut s, "tangent", json!(["e3", "e5"]), None);
    let done = run(
        &mut s,
        "sketch.constrain",
        json!({"sketch": "Profile", "type": "distance", "entities": ["e1", "e4"],
               "direction": "horizontal", "value": "2.5 in"}),
    );
    assert_eq!(
        (&done["status"], &done["dof"]),
        (&json!("ok"), &json!(0)),
        "{done}"
    );
    // The centre was drawn above the line, so it stays above: one radius up.
    assert_points(
        &mut s,
        "Profile",
        &[("e1", 0.0, 0.0), ("e2", 5.0, 0.0), ("e4", 2.5, 1.0)],
    );
    let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "Profile"}));
    let radius = inspected["entities"][4]["radius"].as_f64().unwrap();
    assert!((radius - 1.0).abs() <= NEAR, "{radius}");
    // The solved radius is stored too, not the 0.7 that was drawn.
    let document = run(&mut s, "document.inspect", json!({}));
    let stored = document["features"][0]["params"]["entities"][4]["geometry"]["radius"]
        .as_f64()
        .unwrap();
    assert!((stored - 1.0).abs() <= NEAR, "{stored}");
    assert_eq!(inspected["lengthUnit"], "in");
}

/// A 3-4-5 triangle: the right angle is not asked for, it follows.
#[test]
fn a_triangle_from_three_lengths_keeps_the_side_it_was_drawn_on() {
    for (drawn, side) in [([0.5, 2.0], 1.0), ([0.5, -2.0], -1.0)] {
        let mut s = session();
        run(&mut s, "file.new", json!({}));
        run(&mut s, "sketch.create", json!({"name": "Profile"}));
        // e1, e2, e3 are the corners; e4 = e1-e2, e5 = e2-e3, e6 = e3-e1.
        run(
            &mut s,
            "sketch.polyline",
            json!({"sketch": "Profile", "closed": true, "points": [[0, 0], [3, 0.4], drawn]}),
        );
        constrain(&mut s, "fixed", json!(["e1"]), None);
        constrain(&mut s, "horizontal", json!(["e4"]), None);
        constrain(&mut s, "distance", json!(["e4"]), Some("4"));
        constrain(&mut s, "distance", json!(["e5"]), Some("5"));
        let done = constrain(&mut s, "distance", json!(["e6"]), Some("3"));
        assert_eq!(
            (&done["status"], &done["dof"]),
            (&json!("ok"), &json!(0)),
            "{done}"
        );
        // 3^2 + 4^2 = 5^2: the corner at e1 is square, so e3 is straight
        // above or below e1, on the side it was drawn.
        assert_points(
            &mut s,
            "Profile",
            &[("e1", 0.0, 0.0), ("e2", 4.0, 0.0), ("e3", 0.0, 3.0 * side)],
        );
        // The angle from e4 (pointing +x) to e6 (pointing from e3 to e1,
        // so -y above and +y below), counter-clockwise: 270 or 90 degrees.
        let angle = if side > 0.0 { "270 deg" } else { "90 deg" };
        let extra = constrain(&mut s, "angle", json!(["e4", "e6"]), Some(angle));
        assert_eq!(extra["status"], "ok", "{extra}");
        assert_eq!(extra["redundant"], json!(["c6"]));
    }
}

#[test]
fn a_sketch_lies_on_its_plane() {
    let mut s = session();
    run(&mut s, "file.new", json!({}));
    run(
        &mut s,
        "variable.set",
        json!({"name": "lift", "expr": "7 mm"}),
    );
    for (plane, origin, x, y) in [
        ("xy", [0.0, 0.0, 7.0], [1, 0, 0], [0, 1, 0]),
        ("yz", [7.0, 0.0, 0.0], [0, 1, 0], [0, 0, 1]),
        ("zx", [0.0, 7.0, 0.0], [0, 0, 1], [1, 0, 0]),
    ] {
        let created = run(
            &mut s,
            "sketch.create",
            json!({"plane": plane, "offset": "lift"}),
        );
        let inspected = run(&mut s, "sketch.inspect", json!({"sketch": created["id"]}));
        assert_eq!(
            inspected["frame"],
            json!({"origin": origin, "x": [x[0] as f64, x[1] as f64, x[2] as f64],
                   "y": [y[0] as f64, y[1] as f64, y[2] as f64]}),
            "{plane}"
        );
        assert_eq!(
            inspected["plane"],
            json!({"type": "world", "plane": plane, "offset": "lift"})
        );
    }
    let moved = run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "Sketch1", "plane": "zx", "offset": "-2 * lift"}),
    );
    assert_eq!(moved["status"], "ok");
    let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "Sketch1"}));
    assert_eq!(inspected["frame"]["origin"], json!([0.0, -14.0, 0.0]));
    // An offset that is not a length fails the sketch, and says so.
    let wrong = run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "Sketch1", "offset": "30 deg"}),
    );
    assert_eq!(wrong["status"], "failed");
    assert!(
        wrong["error"].as_str().unwrap().contains("plane.offset"),
        "{wrong}"
    );
}

#[test]
fn entities_can_be_moved_marked_and_deleted() {
    let mut s = session();
    run(&mut s, "file.new", json!({}));
    run(&mut s, "sketch.create", json!({"name": "Profile"}));
    let p = run(
        &mut s,
        "sketch.point",
        json!({"sketch": "Profile", "at": [1, 2]}),
    );
    assert_eq!((&p["entity"], &p["dof"]), (&json!("e1"), &json!(2)));
    run(
        &mut s,
        "sketch.line",
        json!({"sketch": "Profile", "from": "e1", "to": [4, 6]}),
    );
    let arc = run(
        &mut s,
        "sketch.arc",
        json!({"sketch": "Profile", "center": "e1", "start": 2, "end": [-3, 2.5]}),
    );
    assert_eq!(
        arc,
        json!({"id": "F1", "name": "Profile", "status": "ok", "dof": 5,
               "entity": "e5", "center": "e1", "start": "e2", "end": "e4"})
    );
    // Drawn with the start 5 and the end about 4 from the centre. All
    // three points are free, so each may move; what must hold is that
    // both ends are equally far from the centre, somewhere in between.
    let (c, a, b) = (
        point(&mut s, "Profile", "e1"),
        point(&mut s, "Profile", "e2"),
        point(&mut s, "Profile", "e4"),
    );
    let (to_start, to_end) = ((a.0 - c.0).hypot(a.1 - c.1), (b.0 - c.0).hypot(b.1 - c.1));
    assert!((to_start - to_end).abs() <= NEAR, "{to_start} and {to_end}");
    assert!(to_start > 4.0 && to_start < 5.0, "{to_start}");
    run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "Profile", "entity": "e3", "construction": true}),
    );
    let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "Profile"}));
    assert_eq!(inspected["entities"][2]["construction"], true);
    assert_eq!(inspected["entities"][2]["type"], "line");
    constrain(&mut s, "fixed", json!(["e1"]), None);
    let in_use = error(
        &mut s,
        "sketch.delete",
        json!({"sketch": "Profile", "entity": "e1"}),
    );
    assert!(in_use.contains("e3") && in_use.contains("e5"), "{in_use}");
    run(
        &mut s,
        "sketch.delete",
        json!({"sketch": "Profile", "entity": "e5"}),
    );
    run(
        &mut s,
        "sketch.delete",
        json!({"sketch": "Profile", "entity": "e3"}),
    );
    let gone = run(
        &mut s,
        "sketch.delete",
        json!({"sketch": "Profile", "entity": "e1"}),
    );
    assert_eq!(gone["constraintsDeleted"], json!(["c1"]));
    assert_eq!(gone["dof"], 4, "two free points are left");
    // A free point goes where it is put.
    let moved = run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "Profile", "entity": "e2", "at": [-3.5, 9]}),
    );
    assert_eq!(moved["status"], "ok");
    assert_eq!(point(&mut s, "Profile", "e2"), (-3.5, 9.0));
    // Deleting the sketch is deleting the feature.
    run(&mut s, "feature.delete", json!({"feature": "Profile"}));
    error(&mut s, "sketch.inspect", json!({"sketch": "Profile"}));
}

#[test]
fn bad_requests_are_refused_with_a_reason_and_change_nothing() {
    let mut s = session();
    l_profile(&mut s);
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": [1, 2, 3]}),
    );
    let before = run(&mut s, "document.inspect", json!({}));
    let cases: [(&str, Value, &str); 22] = [
        (
            "sketch.point",
            json!({"at": [0, 0]}),
            "missing parameter `sketch`",
        ),
        (
            "sketch.point",
            json!({"sketch": "Nope", "at": [0, 0]}),
            "no feature named `Nope`",
        ),
        (
            "sketch.point",
            json!({"sketch": "Plate", "at": [0, 0]}),
            "is not a sketch",
        ),
        (
            "sketch.point",
            json!({"sketch": "Profile", "at": [0]}),
            "two finite numbers",
        ),
        (
            "sketch.point",
            json!({"sketch": "Profile", "at": [0, true]}),
            "two finite numbers",
        ),
        (
            "sketch.line",
            json!({"sketch": "Profile", "from": "e1"}),
            "missing parameter `to`",
        ),
        (
            "sketch.line",
            json!({"sketch": "Profile", "from": "e1", "to": "e1"}),
            "two different points",
        ),
        (
            "sketch.line",
            json!({"sketch": "Profile", "from": "e1", "to": "e7"}),
            "not a point",
        ),
        (
            "sketch.line",
            json!({"sketch": "Profile", "from": "e1", "to": "e99"}),
            "no entity e99",
        ),
        (
            "sketch.line",
            json!({"sketch": "Profile", "from": "x1", "to": "e2"}),
            "must be an id",
        ),
        (
            "sketch.polyline",
            json!({"sketch": "Profile", "points": [[0, 0]]}),
            "at least 2 points",
        ),
        (
            "sketch.polyline",
            json!({"sketch": "Profile", "points": [[0, 0], [1, 1]], "closed": true}),
            "at least 3 points",
        ),
        (
            "sketch.circle",
            json!({"sketch": "Profile", "center": [0, 0], "radius": 0}),
            "positive",
        ),
        (
            "sketch.circle",
            json!({"sketch": "Profile", "center": [0, 0], "radius": [2]}),
            "finite number",
        ),
        (
            "sketch.arc",
            json!({"sketch": "Profile", "center": "e1", "start": "e1", "end": "e2"}),
            "three different points",
        ),
        (
            "sketch.constrain",
            json!({"sketch": "Profile", "type": "paralel", "entities": ["e7", "e9"]}),
            "unknown constraint type",
        ),
        (
            "sketch.constrain",
            json!({"sketch": "Profile", "type": "parallel", "entities": ["e7", "e1"]}),
            "takes a line and a line",
        ),
        (
            "sketch.constrain",
            json!({"sketch": "Profile", "type": "distance", "entities": ["e7"]}),
            "needs a `value`",
        ),
        (
            "sketch.constrain",
            json!({"sketch": "Profile", "type": "fixed", "entities": ["e1"], "value": 3}),
            "takes no `value`",
        ),
        (
            "sketch.edit",
            json!({"sketch": "Profile", "constraint": "c1", "value": 3}),
            "has no value",
        ),
        (
            "sketch.edit",
            json!({"sketch": "Profile"}),
            "give something to change",
        ),
        (
            "sketch.delete",
            json!({"sketch": "Profile", "entity": "e7", "constraint": "c1"}),
            "either",
        ),
    ];
    for (id, params, expected) in cases {
        let message = error(&mut s, id, params.clone());
        assert!(message.starts_with(id), "{message}");
        assert!(message.contains(expected), "{id} {params}: {message}");
    }
    // A polyline that fails half-way leaves no points behind.
    error(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Profile", "points": [[0, 0], [1, 1], "e99"]}),
    );
    assert_eq!(run(&mut s, "document.inspect", json!({})), before);
    // The other commands know what a sketch is.
    let edit = error(
        &mut s,
        "feature.edit",
        json!({"feature": "Profile", "radius": 1}),
    );
    assert!(edit.contains("sketch.edit"), "{edit}");
    let measure = error(&mut s, "document.measure", json!({"feature": "Profile"}));
    assert!(measure.contains("no body"), "{measure}");
}
