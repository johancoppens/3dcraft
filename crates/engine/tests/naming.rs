//! The topological naming suite, version 1 (docs/testing.md): a feature
//! that refers to a face of an earlier body by its persistent name keeps
//! referring to that face through upstream edits, or fails and says the
//! face is gone. It never ends up on another face.
//!
//! The consumer here is a sketch on a face, with a boss extruded from it.
//! The boss's centroid and volume come from the exact body, so they say
//! where the sketch ended up to rounding.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Access, Session, Workspace};
use serde_json::{Value, json};
use std::f64::consts::PI;

fn session() -> Session {
    Session::new(Workspace {
        read: Access::None,
        write: Access::None,
    })
}

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    let result = session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"));
    // Built up edit by edit, the document is what regenerating it from
    // nothing gives (docs/testing.md: incremental = full regeneration).
    assert_ne!(
        session.regeneration_matches_a_full_one(),
        Some(false),
        "after {id} {params}"
    );
    result
}

fn constrain(session: &mut Session, kind: &str, entities: Value, value: Option<&str>) {
    let mut params = json!({"sketch": "Profile", "type": kind, "entities": entities});
    if let Some(value) = value {
        params["value"] = value.into();
    }
    let result = run(session, "sketch.constrain", params);
    assert_eq!(result["status"], "ok", "{result}");
}

const END_CAP: &str = "F2:endCap(e7,e8,e9,e10,e11,e12)";
const START_CAP: &str = "F2:startCap(e7,e8,e9,e10,e11,e12)";

/// The bracket: an L profile (width 40, height 30, thickness 5, corners
/// e1..e6 counter-clockwise from the origin, sides e7..e12) on the xy
/// plane, extruded by `depth` = 8 into the body `Bracket` (feature F2).
///
/// ```text
///  e6 +--+ e5
///     |  |          e7: e1-e2 (bottom)    e10: e4-e5
///     |  +------+ e3   e8: e2-e3 (right end)  e11: e5-e6 (top)
///     |  e4     |   e9: e3-e4              e12: e6-e1 (left)
///  e1 +---------+ e2
/// ```
fn bracket(session: &mut Session) {
    run(session, "file.new", json!({"units": "mm"}));
    for (name, expr) in [
        ("width", "40 mm"),
        ("height", "30 mm"),
        ("thickness", "5 mm"),
        ("depth", "8 mm"),
    ] {
        run(session, "variable.set", json!({"name": name, "expr": expr}));
    }
    run(session, "sketch.create", json!({"name": "Profile"}));
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
    let made = run(
        session,
        "feature.extrude",
        json!({"sketch": "Profile", "distance": "depth", "name": "Bracket"}),
    );
    assert_eq!(made["id"], "F2");
}

/// A sketch `Seat` on `face` with one circle of radius 1 at (x, y) in the
/// sketch's own axes, and a boss of height 4 extruded from it.
fn boss(session: &mut Session, face: &str, offset: f64, x: f64, y: f64) {
    let seat = run(
        session,
        "sketch.create",
        json!({"name": "Seat", "face": face, "offset": offset}),
    );
    assert_eq!(seat["status"], "ok", "{seat}");
    run(
        session,
        "sketch.circle",
        json!({"sketch": "Seat", "center": [x, y], "radius": 1}),
    );
    let made = run(
        session,
        "feature.extrude",
        json!({"sketch": "Seat", "distance": 4, "name": "Boss"}),
    );
    assert_eq!(made["status"], "ok", "{made}");
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

/// The boss is a cylinder of radius 1 and height 4 with its centroid at
/// `expected`: exact up to the sketch solve (1e-6 mm).
fn assert_boss_at(session: &mut Session, expected: [f64; 3]) {
    let state = feature(session, "Boss");
    assert_eq!(state["status"], "ok", "{state}");
    let measured = run(session, "document.measure", json!({"feature": "Boss"}));
    let volume = measured["volume"].as_f64().unwrap();
    assert!((volume - PI * 4.0).abs() <= 1e-9, "volume {volume}");
    for axis in 0..3 {
        let actual = measured["centroid"][axis].as_f64().unwrap();
        assert!(
            (actual - expected[axis]).abs() <= 1e-5,
            "the boss is at {}, expected {expected:?}",
            measured["centroid"]
        );
    }
}

/// The seat failed with a reason that has these words, and took the boss
/// with it: there is no boss anywhere else.
fn assert_seat_lost(session: &mut Session, words: &[&str]) {
    let seat = feature(session, "Seat");
    assert_eq!(seat["status"], "failed", "{seat}");
    let reason = seat["error"].as_str().unwrap();
    for word in words {
        assert!(reason.contains(word), "`{word}` is not in: {reason}");
    }
    let boss = feature(session, "Boss");
    assert_eq!(boss["status"], "failed", "{boss}");
    let document = run(session, "document.inspect", json!({}));
    assert!(
        document["bodies"]
            .as_array()
            .unwrap()
            .iter()
            .all(|b| b["name"] != "Boss"),
        "a failed boss has no body"
    );
}

#[test]
fn a_sketch_on_the_end_cap_has_the_axes_of_the_extruded_sketch() {
    let mut s = session();
    bracket(&mut s);
    boss(&mut s, END_CAP, 0.0, 10.0, 2.5);
    // On the cap at z = 8, rising to 12: the middle is at 10.
    assert_boss_at(&mut s, [10.0, 2.5, 10.0]);
    let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "Seat"}));
    assert_eq!(
        inspected["frame"],
        json!({"origin": [0.0, 0.0, 8.0], "x": [1.0, 0.0, 0.0], "y": [0.0, 1.0, 0.0]})
    );
    assert_eq!(
        inspected["plane"],
        json!({"type": "face", "face": END_CAP, "offset": "0"})
    );
}

#[test]
fn scenario_change_the_extrude_distance() {
    let mut s = session();
    bracket(&mut s);
    boss(&mut s, END_CAP, 0.0, 10.0, 2.5);
    run(
        &mut s,
        "variable.set",
        json!({"name": "depth", "expr": "20 mm"}),
    );
    assert_boss_at(&mut s, [10.0, 2.5, 22.0]);
    // Turned the other way the end cap is at z = -20 and faces down. A
    // sketch on a face is seen from outside the body, so its normal is
    // now -Z; a right-handed frame with x still along world X then has y
    // along world -Y, and the boss drawn at y = 2.5 hangs at world
    // y = -2.5: mirrored, as the face itself is when seen from below.
    run(
        &mut s,
        "variable.set",
        json!({"name": "depth", "expr": "-20 mm"}),
    );
    assert_boss_at(&mut s, [10.0, -2.5, -22.0]);
    // Symmetric: 10 each way, the end cap on top.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Bracket", "symmetric": true}),
    );
    assert_boss_at(&mut s, [10.0, 2.5, 12.0]);
}

#[test]
fn scenario_add_sketch_entities_upstream() {
    let mut s = session();
    bracket(&mut s);
    boss(&mut s, END_CAP, 0.0, 10.0, 2.5);
    // A hole in the bracket: more entities, more faces, another face order.
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Profile", "center": [30, 2.5], "radius": 1.5}),
    );
    assert_boss_at(&mut s, [10.0, 2.5, 10.0]);
    // A second, separate profile in the same sketch, below the L where
    // the wider L of the next step does not reach it: a second shell.
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Profile", "closed": true, "points": [[50, -20], [60, -20], [55, -11]]}),
    );
    assert_eq!(feature(&mut s, "Bracket")["status"], "ok");
    assert_boss_at(&mut s, [10.0, 2.5, 10.0]);
    // And dimensions that move the face's outline but not its origin.
    run(
        &mut s,
        "variable.set",
        json!({"name": "width", "expr": "90 mm"}),
    );
    run(
        &mut s,
        "variable.set",
        json!({"name": "height", "expr": "12 mm"}),
    );
    assert_eq!(feature(&mut s, "Bracket")["status"], "ok");
    assert_boss_at(&mut s, [10.0, 2.5, 10.0]);
}

#[test]
fn scenario_reorder_independent_features() {
    let mut s = session();
    bracket(&mut s);
    boss(&mut s, END_CAP, 0.0, 10.0, 2.5);
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": [1, 2, 3]}),
    );
    for position in [0, 2, 4, 1] {
        run(
            &mut s,
            "feature.move",
            json!({"feature": "Plate", "position": position}),
        );
        assert_boss_at(&mut s, [10.0, 2.5, 10.0]);
    }
    // The seat before the body it sits on: it cannot know the face yet.
    run(
        &mut s,
        "feature.move",
        json!({"feature": "Seat", "position": 0}),
    );
    assert_seat_lost(&mut s, &["Bracket", "comes after", END_CAP]);
    run(&mut s, "edit.undo", json!({}));
    assert_boss_at(&mut s, [10.0, 2.5, 10.0]);
}

#[test]
fn scenario_the_feature_that_made_the_face_goes_away() {
    let mut s = session();
    bracket(&mut s);
    boss(&mut s, END_CAP, 0.0, 10.0, 2.5);
    // A second extrusion of the same sketch, further on: its end cap has
    // the same sources but another feature in its name, and must never be
    // taken for the first.
    run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Profile", "distance": 50, "name": "Decoy"}),
    );
    assert_boss_at(&mut s, [10.0, 2.5, 10.0]);
    for (command, params, words) in [
        (
            "feature.delete",
            json!({"feature": "Bracket"}),
            ["no feature F2", "deleted"],
        ),
        (
            "feature.suppress",
            json!({"feature": "Bracket"}),
            ["Bracket", "suppressed"],
        ),
        ("history.rollback", json!({"feature": "Profile"}), ["", ""]),
        (
            "variable.set",
            json!({"name": "depth", "expr": "nope"}),
            ["Bracket", "failed"],
        ),
    ] {
        run(&mut s, command, params);
        if command == "history.rollback" {
            // Everything after the marker is rolled back, the seat too.
            assert_eq!(feature(&mut s, "Seat")["status"], "rolledBack");
        } else {
            assert_seat_lost(&mut s, &words);
        }
        run(&mut s, "edit.undo", json!({}));
        assert_boss_at(&mut s, [10.0, 2.5, 10.0]);
    }
}

#[test]
fn scenario_the_outline_of_a_cap_changes() {
    // Field test 2026-10-12, finding 1: rounding the corners of a plate
    // renamed its end cap and broke every sketch on it. All caps of one
    // role of one extrusion lie in one plane with one frame (the extruded
    // sketch's), so a sketch on a cap keeps its place whatever happens to
    // the outline the cap was named after.
    let mut s = session();
    bracket(&mut s);
    boss(&mut s, END_CAP, 0.0, 10.0, 2.5);
    // A frame drawn around the L turns the L into a hole of a new outer
    // profile: the end cap is now named after the frame's sides.
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Profile", "closed": true,
               "points": [[-10, -10], [70, -10], [70, 50], [-10, 50]]}),
    );
    assert_eq!(feature(&mut s, "Bracket")["status"], "ok");
    assert_boss_at(&mut s, [10.0, 2.5, 10.0]);
    run(&mut s, "edit.undo", json!({}));
    // A side of the L replaced by two: the cap is named after seven sides.
    run(
        &mut s,
        "sketch.delete",
        json!({"sketch": "Profile", "entity": "e11"}),
    );
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Profile", "points": ["e5", [2.5, 33], "e6"]}),
    );
    assert_eq!(feature(&mut s, "Bracket")["status"], "ok");
    assert_boss_at(&mut s, [10.0, 2.5, 10.0]);
    // The stored name is the user's and is not rewritten.
    let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "Seat"}));
    assert_eq!(inspected["plane"]["face"], END_CAP);
    // Two separate profiles, so two end caps, neither with the old name:
    // still one plane.
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Profile", "closed": true, "points": [[50, -20], [60, -20], [55, -11]]}),
    );
    assert_eq!(feature(&mut s, "Bracket")["status"], "ok");
    assert_boss_at(&mut s, [10.0, 2.5, 10.0]);
    // The plane still follows the extrusion.
    run(
        &mut s,
        "variable.set",
        json!({"name": "depth", "expr": "20 mm"}),
    );
    assert_boss_at(&mut s, [10.0, 2.5, 22.0]);
}

#[test]
fn a_cap_can_be_named_without_its_outline() {
    // `F2:endCap()` is "the plane where F2 ends".
    let mut s = session();
    bracket(&mut s);
    boss(&mut s, "F2:endCap()", 0.0, 10.0, 2.5);
    assert_boss_at(&mut s, [10.0, 2.5, 10.0]);
    run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "Seat", "face": "F2:startCap()"}),
    );
    // Seen from below y is reversed; the boss hangs under z = 0.
    assert_boss_at(&mut s, [10.0, -2.5, -2.0]);
}

#[test]
fn the_looser_match_is_for_caps_of_the_same_feature_and_role_only() {
    let mut s = session();
    bracket(&mut s);
    run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Profile", "distance": 50, "name": "Decoy"}),
    );
    // A cap name with a stale outline never lands on another feature, on
    // the other cap, or on a side.
    boss(&mut s, "F2:endCap(e99)", 0.0, 10.0, 2.5);
    assert_boss_at(&mut s, [10.0, 2.5, 10.0]);
    run(&mut s, "feature.suppress", json!({"feature": "Bracket"}));
    assert_seat_lost(&mut s, &["Bracket", "suppressed"]);
    run(&mut s, "edit.undo", json!({}));
    // A side has a plane of its own: a stale side name points at nothing.
    run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "Seat", "face": "F2:side(e99)"}),
    );
    assert_seat_lost(&mut s, &["no longer has a face", "F2:side(e99)"]);
}

#[test]
fn a_sketch_on_the_start_cap_looks_from_outside() {
    let mut s = session();
    bracket(&mut s);
    // Seen from below, x still runs along world X and y runs against
    // world Y; the boss grows downwards from z = 0.
    boss(&mut s, START_CAP, 0.0, 10.0, -2.5);
    assert_boss_at(&mut s, [10.0, 2.5, -2.0]);
    let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "Seat"}));
    assert_eq!(
        inspected["frame"],
        json!({"origin": [0.0, 0.0, 0.0], "x": [1.0, 0.0, 0.0], "y": [0.0, -1.0, 0.0]})
    );
    run(
        &mut s,
        "variable.set",
        json!({"name": "depth", "expr": "20 mm"}),
    );
    assert_boss_at(&mut s, [10.0, 2.5, -2.0]);
}

#[test]
fn a_sketch_on_a_side_face_follows_the_side() {
    let mut s = session();
    bracket(&mut s);
    // e8 runs from e2 (40, 0) up to e3 (40, 5): the right end of the bar,
    // facing +X. The sketch starts at e2 on the start plane, with x along
    // the side (world +Y) and y along the extrusion (world +Z).
    boss(&mut s, "F2:side(e8)", 0.0, 2.5, 4.0);
    assert_boss_at(&mut s, [42.0, 2.5, 4.0]);
    let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "Seat"}));
    assert_eq!(
        inspected["frame"],
        json!({"origin": [40.0, 0.0, 0.0], "x": [0.0, 1.0, 0.0], "y": [0.0, 0.0, 1.0]})
    );
    // The end of the bar moves out, and the boss with it.
    run(
        &mut s,
        "variable.set",
        json!({"name": "width", "expr": "65 mm"}),
    );
    assert_boss_at(&mut s, [67.0, 2.5, 4.0]);
    // Thicker and deeper: the side grows, its start corner stays.
    run(
        &mut s,
        "variable.set",
        json!({"name": "thickness", "expr": "9 mm"}),
    );
    run(
        &mut s,
        "variable.set",
        json!({"name": "depth", "expr": "30 mm"}),
    );
    assert_boss_at(&mut s, [67.0, 2.5, 4.0]);
    // Symmetric, or turned the other way: the side now starts 15 below
    // the sketch plane, or ends there, but a sketch on it is still
    // measured from the plane of the extruded sketch.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Bracket", "symmetric": true}),
    );
    assert_boss_at(&mut s, [67.0, 2.5, 4.0]);
    run(&mut s, "edit.undo", json!({}));
    run(
        &mut s,
        "variable.set",
        json!({"name": "depth", "expr": "-30 mm"}),
    );
    assert_boss_at(&mut s, [67.0, 2.5, 4.0]);
    run(&mut s, "edit.undo", json!({}));
    // An offset lifts the sketch off the face, along the face's normal.
    run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "Seat", "offset": "width / 65"}),
    );
    assert_boss_at(&mut s, [68.0, 2.5, 4.0]);
    // Moved to the end cap, keeping its offset: 30 + 1 + half of 4.
    run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "Seat", "face": END_CAP}),
    );
    assert_boss_at(&mut s, [2.5, 4.0, 33.0]);
    // And to a world plane.
    run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "Seat", "plane": "yz"}),
    );
    assert_boss_at(&mut s, [3.0, 2.5, 4.0]);
    // The side that carried it is deleted afterwards: nothing breaks,
    // because nothing refers to it any more.
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_boss_at(&mut s, [68.0, 2.5, 4.0]);
    run(
        &mut s,
        "sketch.delete",
        json!({"sketch": "Profile", "entity": "e8"}),
    );
    assert_seat_lost(&mut s, &["Bracket", "failed"]);
}

#[test]
fn a_sketch_on_a_side_face_is_measured_from_the_origin_of_the_extruded_sketch() {
    // Field test 2026-10-12, finding 2: the origin of a side-face sketch
    // was the start of the line that made the face, which moves with
    // upstream dimensions. It is the extruded sketch's origin dropped
    // straight onto the face.
    let mut s = session();
    bracket(&mut s);
    // e10 runs from e4 (5, 5) up to e5 (5, 30): the inner wall of the
    // upright, facing +X. The sketch origin (0, 0) dropped onto x = 5 is
    // (5, 0, 0), not the corner e4.
    boss(&mut s, "F2:side(e10)", 0.0, 20.0, 4.0);
    let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "Seat"}));
    assert_eq!(
        inspected["frame"],
        json!({"origin": [5.0, 0.0, 0.0], "x": [0.0, 1.0, 0.0], "y": [0.0, 0.0, 1.0]})
    );
    assert_boss_at(&mut s, [7.0, 20.0, 4.0]);
    // Thicker: e4 moves to (9, 9). The wall moves out to x = 9 and the
    // boss with it, but it does not climb with the corner.
    run(
        &mut s,
        "variable.set",
        json!({"name": "thickness", "expr": "9 mm"}),
    );
    assert_boss_at(&mut s, [11.0, 20.0, 4.0]);
    // The side replaced by a new line: its own name is gone, and a side
    // has no other face in its plane to fall back on.
    run(
        &mut s,
        "sketch.delete",
        json!({"sketch": "Profile", "entity": "e10"}),
    );
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Profile", "points": ["e4", "e5"]}),
    );
    assert_eq!(feature(&mut s, "Bracket")["status"], "ok");
    assert_seat_lost(&mut s, &["no longer has a face", "F2:side(e10)"]);
}

#[test]
fn the_faces_of_a_primitive_are_named_by_their_number() {
    // A primitive is built in a fixed order, so for it the number of a
    // face says where the face came from: for a block 0 bottom, 1 top,
    // 2 front (-y), 3 back (+y), 4 left (-x), 5 right (+x).
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "variable.set", json!({"name": "t", "expr": "5 mm"}));
    let made = run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": [40, 20, "t"]}),
    );
    assert_eq!(made["id"], "F1");
    let document = run(&mut s, "document.inspect", json!({}));
    assert_eq!(
        document["bodies"][0]["faceNames"],
        json!([
            "F1:face(0)",
            "F1:face(1)",
            "F1:face(2)",
            "F1:face(3)",
            "F1:face(4)",
            "F1:face(5)"
        ])
    );
    // The top: the block's own x and y, at its height.
    boss(&mut s, "F1:face(1)", 0.0, 10.0, 5.0);
    assert_boss_at(&mut s, [10.0, 5.0, 7.0]);
    let frame =
        |s: &mut Session| run(s, "sketch.inspect", json!({"sketch": "Seat"}))["frame"].clone();
    assert_eq!(
        frame(&mut s),
        json!({"origin": [0.0, 0.0, 5.0], "x": [1.0, 0.0, 0.0], "y": [0.0, 1.0, 0.0]})
    );
    // Thicker, and moved: the seat stays on the top.
    run(&mut s, "variable.set", json!({"name": "t", "expr": "8 mm"}));
    assert_boss_at(&mut s, [10.0, 5.0, 10.0]);
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Plate", "position": [100, 0, 0]}),
    );
    assert_boss_at(&mut s, [110.0, 5.0, 10.0]);
    run(&mut s, "edit.undo", json!({}));
    // The right side (+x) and the bottom, each seen from outside.
    for (face, expected) in [
        (
            "F1:face(5)",
            json!({"origin": [40.0, 0.0, 0.0], "x": [0.0, 1.0, 0.0], "y": [0.0, 0.0, 1.0]}),
        ),
        (
            "F1:face(0)",
            json!({"origin": [0.0, 0.0, 0.0], "x": [0.0, 1.0, 0.0], "y": [1.0, 0.0, 0.0]}),
        ),
        (
            "F1:face(4)",
            json!({"origin": [0.0, 0.0, 0.0], "x": [0.0, 0.0, 1.0], "y": [0.0, 1.0, 0.0]}),
        ),
    ] {
        run(
            &mut s,
            "sketch.edit",
            json!({"sketch": "Seat", "face": face}),
        );
        assert_eq!(frame(&mut s), expected, "{face}");
    }
    // A cylinder: 0 the wall, 1 the bottom, 2 the top.
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Post", "radius": 5, "height": 10, "position": [0, 50, 0]}),
    );
    run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "Seat", "face": "F4:face(2)"}),
    );
    assert_seat_lost(&mut s, &["Post", "comes after"]);
    run(
        &mut s,
        "feature.move",
        json!({"feature": "Post", "position": 1}),
    );
    assert_boss_at(&mut s, [10.0, 55.0, 12.0]);
    run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "Seat", "face": "F4:face(1)"}),
    );
    assert_eq!(
        frame(&mut s),
        json!({"origin": [0.0, 50.0, 0.0], "x": [1.0, 0.0, 0.0], "y": [0.0, -1.0, 0.0]})
    );
    run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "Seat", "face": "F4:face(0)"}),
    );
    assert_seat_lost(&mut s, &["cylinder", "flat"]);
}

#[test]
fn names_that_cannot_carry_a_sketch_are_refused_with_a_reason() {
    let mut s = session();
    bracket(&mut s);
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Profile", "center": [30, 2.5], "radius": 1.5}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": [1, 2, 3]}),
    );
    // The request itself is malformed: refused, nothing is added.
    for (face, expected) in [
        (json!("the top"), "is not a name"),
        (json!("F2:roof(e7)"), "the role must be one of"),
        (json!(2), "`face` must be a string"),
    ] {
        let error = s
            .execute("sketch.create", &json!({"face": face}))
            .unwrap_err()
            .to_string();
        assert!(error.contains(expected), "{face}: {error}");
    }
    let both = s
        .execute("sketch.create", &json!({"face": END_CAP, "plane": "xy"}))
        .unwrap_err()
        .to_string();
    assert!(both.contains("either"), "{both}");
    // A well-formed name that cannot carry a sketch is stored and fails,
    // like any feature whose input is not there.
    for (face, words) in [
        ("F2:side(e14)", vec!["cylinder", "flat"]),
        ("F2:lateralEdge(e7,e8)", vec!["is not a face"]),
        ("F2:side(e99)", vec!["no longer has a face", "F2:side(e7)"]),
        (
            "F3:side(e1)",
            vec!["Plate", "no longer has a face", "F3:face(0)"],
        ),
        (
            "F3:face(6)",
            vec!["Plate", "no longer has a face", "F3:face(5)"],
        ),
        ("F3:edge(0)", vec!["is not a face"]),
        ("F1:side(e7)", vec!["Profile", "no names yet"]),
        ("F77:side(e7)", vec!["no feature F77"]),
    ] {
        let made = run(
            &mut s,
            "sketch.create",
            json!({"face": face, "name": "Seat"}),
        );
        assert_eq!(made["status"], "failed", "{face}: {made}");
        let reason = made["error"].as_str().unwrap();
        for word in &words {
            assert!(
                reason.contains(word),
                "{face}: `{word}` is not in: {reason}"
            );
        }
        run(&mut s, "edit.undo", json!({}));
    }
}
