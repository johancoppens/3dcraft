//! `feature.boolean` through the command layer: volumes against closed
//! forms and the identities of docs/testing.md (oracle 3), names through a
//! boolean, and what happens to the operands.

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

/// A plate of `length` x 40 x 8 extruded from sketch `Outline` (F1; sides
/// e5..e8) as `Plate` (F2), with its low corner at the origin.
fn plate() -> Session {
    let mut s = Session::new(Workspace {
        read: Access::None,
        write: Access::None,
    });
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "variable.set",
        json!({"name": "length", "expr": "60 mm"}),
    );
    run(&mut s, "sketch.create", json!({"name": "Outline"}));
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Outline", "closed": true,
               "points": [[0, 0], [60, 0], [60, 40], [0, 40]]}),
    );
    for (kind, entities, value) in [
        ("fixed", json!(["e1"]), None),
        ("horizontal", json!(["e5"]), None),
        ("vertical", json!(["e6"]), None),
        ("horizontal", json!(["e7"]), None),
        ("vertical", json!(["e8"]), None),
        ("distance", json!(["e5"]), Some("length")),
        ("distance", json!(["e6"]), Some("40 mm")),
    ] {
        let mut params = json!({"sketch": "Outline", "type": kind, "entities": entities});
        if let Some(value) = value {
            params["value"] = value.into();
        }
        run(&mut s, "sketch.constrain", params);
    }
    let made = run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Outline", "distance": 8, "name": "Plate"}),
    );
    assert_eq!((&made["id"], &made["status"]), (&json!("F2"), &json!("ok")));
    s
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

fn body(session: &mut Session, name: &str) -> Value {
    let document = run(session, "document.inspect", json!({}));
    document["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == name)
        .unwrap_or_else(|| panic!("{name} has no body: {}", feature(session, name)))
        .clone()
}

fn body_names(session: &mut Session) -> Vec<String> {
    let document = run(session, "document.inspect", json!({}));
    document["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["name"].as_str().unwrap().to_string())
        .collect()
}

fn volume(session: &mut Session, name: &str) -> f64 {
    body(session, name)["volume"].as_f64().unwrap()
}

fn assert_volume(session: &mut Session, name: &str, expected: f64) {
    let actual = volume(session, name);
    assert!(
        (actual - expected).abs() <= 1e-9 * expected,
        "{name}: volume {actual}, expected {expected}"
    );
    let check = run(session, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
}

fn assert_failed(session: &mut Session, name: &str, words: &[&str]) {
    let state = feature(session, name);
    assert_eq!(state["status"], "failed", "{state}");
    let reason = state["error"].as_str().unwrap();
    for word in words {
        assert!(reason.contains(word), "`{word}` is not in: {reason}");
    }
}

fn boolean(session: &mut Session, op: &str, target: &str, tool: &str, name: &str) -> Value {
    run(
        session,
        "feature.boolean",
        json!({"op": op, "target": target, "tool": tool, "name": name}),
    )
}

#[test]
fn a_post_standing_on_a_plate_becomes_one_body() {
    // The model of the first field test: the post's bottom lies exactly
    // in the plate's top.
    let mut s = plate();
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Post", "radius": 6, "height": 30, "position": [30, 20, 8]}),
    );
    let made = boolean(&mut s, "union", "Plate", "Post", "Part");
    assert_eq!(made["status"], "ok", "{made}");
    assert_volume(&mut s, "Part", 19200.0 + PI * 36.0 * 30.0);
    // Both went into it.
    assert_eq!(body_names(&mut s), ["Part"]);
    assert_eq!(feature(&mut s, "Plate")["bodyUsedBy"], "F4");
    assert_eq!(feature(&mut s, "Post")["bodyUsedBy"], "F4");
    // Faces are named after the face they came from: instance 0 is the
    // target, 1 the tool.
    let names = body(&mut s, "Part")["faceNames"].clone();
    let names: Vec<&str> = names
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap())
        .collect();
    for name in [
        "F4:copy(0)/F2:endCap(e5,e6,e7,e8)",
        "F4:copy(0)/F2:startCap(e5,e6,e7,e8)",
        "F4:copy(0)/F2:side(e5)",
        "F4:copy(1)/F3:face(0)",
        "F4:copy(1)/F3:face(2)",
    ] {
        assert!(names.contains(&name), "{name} is not in {names:?}");
    }
    // The post's bottom is inside the part now: it is not a face any more.
    assert!(!names.contains(&"F4:copy(1)/F3:face(1)"), "{names:?}");

    // A sketch on the top of the post, named through the union, and a
    // boss on it: 30 + 8 high, the boss 4 more.
    run(
        &mut s,
        "sketch.create",
        json!({"name": "Seat", "face": "F4:copy(1)/F3:face(2)"}),
    );
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Seat", "center": [0, 0], "radius": 2}),
    );
    run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Seat", "distance": 4, "name": "Boss"}),
    );
    let boss = body(&mut s, "Boss");
    assert!(
        (boss["centroid"][2].as_f64().unwrap() - 40.0).abs() < 1e-9,
        "{boss}"
    );
    // The face that the union swallowed carries nothing.
    run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "Seat", "face": "F4:copy(1)/F3:face(1)"}),
    );
    assert_failed(&mut s, "Seat", &["Part", "no longer has a face"]);
    run(&mut s, "edit.undo", json!({}));
    // The plate gets longer: the part follows.
    run(
        &mut s,
        "variable.set",
        json!({"name": "length", "expr": "80 mm"}),
    );
    assert_volume(&mut s, "Part", 25600.0 + PI * 36.0 * 30.0);
}

#[test]
fn holes_cut_by_subtraction_follow_the_plate() {
    let mut s = plate();
    // A cutter that ends exactly in the plate's faces, and one that
    // sticks out on both sides.
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Drill", "radius": 5, "height": 8, "position": ["length / 4", 20, 0]}),
    );
    boolean(&mut s, "subtract", "Plate", "Drill", "Drilled");
    assert_volume(&mut s, "Drilled", 19200.0 - PI * 25.0 * 8.0);
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Drill2", "radius": 3, "height": 20, "position": ["3 * length / 4", 20, -6]}),
    );
    boolean(&mut s, "subtract", "Drilled", "Drill2", "Part");
    let expected = |length: f64| length * 320.0 - PI * (25.0 + 9.0) * 8.0;
    assert_volume(&mut s, "Part", expected(60.0));
    assert_eq!(body_names(&mut s), ["Part"]);
    // The same plate with the same holes drawn in its sketch: the same
    // volume, area and number of faces.
    let cut = body(&mut s, "Part");
    let mut t = plate();
    for (x, r) in [(15, 5), (45, 3)] {
        run(
            &mut t,
            "sketch.circle",
            json!({"sketch": "Outline", "center": [x, 20], "radius": r}),
        );
    }
    let drawn = body(&mut t, "Plate");
    assert_eq!(cut["faces"], drawn["faces"]);
    for key in ["volume", "area"] {
        let (a, b) = (cut[key].as_f64().unwrap(), drawn[key].as_f64().unwrap());
        assert!((a - b).abs() <= 1e-9 * b, "{key}: {a} vs {b}");
    }
    // Longer: the holes stay at a quarter and three quarters.
    run(
        &mut s,
        "variable.set",
        json!({"name": "length", "expr": "100 mm"}),
    );
    assert_volume(&mut s, "Part", expected(100.0));
    // A name through two booleans.
    let names = cut["faceNames"].as_array().unwrap();
    assert!(
        names.contains(&json!("F6:copy(0)/F2:endCap(e5,e6,e7,e8)")),
        "{names:?}"
    );
    assert!(names.contains(&json!("F6:copy(1)/F5:face(0)")), "{names:?}");
}

/// `document.check` deep holds every boolean feature to what every exact
/// boolean of its target and tool keeps (`kernel::boolean::verify`).
#[test]
fn a_deep_check_verifies_every_boolean() {
    let mut s = plate();
    for (name, x) in [("Drill", "length / 4"), ("Drill2", "3 * length / 4")] {
        run(
            &mut s,
            "feature.cylinder",
            json!({"name": name, "radius": 3, "height": 20, "position": [x, 20, -6]}),
        );
    }
    boolean(&mut s, "subtract", "Plate", "Drill", "Drilled");
    boolean(&mut s, "union", "Drilled", "Drill2", "Part");
    let check = run(&mut s, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
    assert_eq!(check["booleansChecked"], 2, "{check}");
    // Not deep: no boolean is looked at.
    let check = run(&mut s, "document.check", json!({}));
    assert_eq!(check["booleansChecked"], Value::Null, "{check}");
}

#[test]
fn a_pattern_of_cutters_cuts_a_row_of_holes() {
    // What a gear or a bolt circle needs: one tool, repeated, then cut.
    let mut s = plate();
    run(&mut s, "variable.set", json!({"name": "n", "expr": "4"}));
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Drill", "radius": 3, "height": 20, "position": [10, 20, -6]}),
    );
    run(
        &mut s,
        "feature.pattern",
        json!({"source": "Drill", "count": "n", "step": ["(length - 20 mm) / (n - 1)", 0, 0],
               "name": "Drills"}),
    );
    boolean(&mut s, "subtract", "Plate", "Drills", "Part");
    let expected = |length: f64, n: f64| length * 320.0 - n * PI * 9.0 * 8.0;
    assert_volume(&mut s, "Part", expected(60.0, 4.0));
    assert_eq!(body_names(&mut s), ["Part"]);
    // Count and length are variables: the holes follow both.
    run(&mut s, "variable.set", json!({"name": "n", "expr": "6"}));
    assert_volume(&mut s, "Part", expected(60.0, 6.0));
    run(
        &mut s,
        "variable.set",
        json!({"name": "length", "expr": "90 mm"}),
    );
    assert_volume(&mut s, "Part", expected(90.0, 6.0));
    // The wall of the fifth hole, by name.
    let names = body(&mut s, "Part")["faceNames"].clone();
    assert!(
        names
            .as_array()
            .unwrap()
            .contains(&json!("F5:copy(1,4)/F3:face(0)")),
        "{names}"
    );
    // A ring of posts joined to the plate.
    run(&mut s, "feature.delete", json!({"feature": "Part"}));
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Drill", "position": [45, 32, 8], "height": 5}),
    );
    run(&mut s, "feature.delete", json!({"feature": "Drills"}));
    run(
        &mut s,
        "feature.pattern",
        json!({"source": "Drill", "count": 5, "angle": "72 deg", "point": [45, 20, 0],
               "name": "Posts"}),
    );
    boolean(&mut s, "union", "Plate", "Posts", "Part");
    assert_volume(&mut s, "Part", 90.0 * 320.0 + 5.0 * PI * 9.0 * 5.0);
}

#[test]
fn scenario_a_cut_splits_or_removes_a_referenced_face() {
    // The naming suite through booleans: a sketch on the top of a cut
    // plate, then the cut changes under it.
    let mut s = plate();
    run(
        &mut s,
        "feature.block",
        json!({"name": "Cutter", "size": [4, 10, 4], "position": [28, 0, 4]}),
    );
    boolean(&mut s, "subtract", "Plate", "Cutter", "Part");
    // A notch 4 wide, 10 long, 4 deep in the front edge of the top.
    assert_volume(&mut s, "Part", 19200.0 - 160.0);
    run(
        &mut s,
        "sketch.create",
        json!({"name": "Seat", "face": "F4:copy(0)/F2:endCap(e5,e6,e7,e8)"}),
    );
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Seat", "center": [10, 30], "radius": 2}),
    );
    run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Seat", "distance": 4, "name": "Boss"}),
    );
    let boss_at = |s: &mut Session| {
        let boss = body(s, "Boss");
        [0, 1, 2].map(|i| boss["centroid"][i].as_f64().unwrap())
    };
    let near = |a: [f64; 3], b: [f64; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6);
    assert!(
        near(boss_at(&mut s), [10.0, 30.0, 10.0]),
        "{:?}",
        boss_at(&mut s)
    );
    let count = |s: &mut Session| {
        body(s, "Part")["faceNames"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|n| *n == "F4:copy(0)/F2:endCap(e5,e6,e7,e8)")
            .count()
    };
    assert_eq!(count(&mut s), 1);
    // The notch becomes a slot right across: the top is in two pieces
    // with one name. They lie in one plane, so the seat stays put.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Cutter", "size": [4, 40, 4]}),
    );
    assert_volume(&mut s, "Part", 19200.0 - 640.0);
    assert_eq!(count(&mut s), 2);
    assert_eq!(feature(&mut s, "Seat")["status"], "ok");
    assert!(near(boss_at(&mut s), [10.0, 30.0, 10.0]));
    // The cutter takes the whole top layer off: that face is gone, and
    // the seat says so instead of landing on the new top.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Cutter", "size": [80, 60, 4], "position": [-10, -10, 4]}),
    );
    assert!((volume(&mut s, "Part") - 9600.0).abs() < 1e-6);
    assert_eq!(count(&mut s), 0);
    assert_failed(
        &mut s,
        "Seat",
        &["Part", "no longer has a face", "F4:copy(1)/F3:face(0)"],
    );
    assert_failed(&mut s, "Boss", &["Seat"]);
    // The new top is the cutter's bottom, and can be named as such.
    run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "Seat", "face": "F4:copy(1)/F3:face(0)"}),
    );
    assert_eq!(feature(&mut s, "Seat")["status"], "ok");
    let top = boss_at(&mut s)[2];
    assert!(
        (top - 2.0).abs() < 1e-6 || (top - 6.0).abs() < 1e-6,
        "{top}"
    );
}

#[test]
fn an_extrusion_cuts_or_joins_a_body_in_one_command() {
    // The phase 1 scenario: sketch on the top of a plate, cut two holes,
    // change the plate, and everything follows.
    let mut s = plate();
    run(
        &mut s,
        "sketch.create",
        json!({"name": "Holes", "face": "F2:endCap()"}),
    );
    for x in [15, 45] {
        run(
            &mut s,
            "sketch.circle",
            json!({"sketch": "Holes", "center": [x, 20], "radius": 4}),
        );
    }
    let made = run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Holes", "distance": -8, "cut": "Plate", "name": "Drilled"}),
    );
    assert_eq!(made["status"], "ok", "{made}");
    assert_eq!(made["name"], "Drilled");
    assert_eq!(made["sweep"]["status"], "ok", "{made}");
    assert_volume(&mut s, "Drilled", 19200.0 - 2.0 * PI * 16.0 * 8.0);
    assert_eq!(body_names(&mut s), ["Drilled"]);
    // One undo step for both features.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(body_names(&mut s), ["Plate"]);
    run(&mut s, "edit.redo", json!({}));
    // Blind pockets instead: half as deep.
    let sweep = made["sweep"]["name"].as_str().unwrap().to_string();
    run(
        &mut s,
        "feature.edit",
        json!({"feature": sweep, "distance": -4}),
    );
    assert_volume(&mut s, "Drilled", 19200.0 - 2.0 * PI * 16.0 * 4.0);
    // A boss joined on, on the top of the drilled plate.
    let top = format!("{}:copy(0)/F2:endCap()", made["id"].as_str().unwrap());
    run(&mut s, "sketch.create", json!({"name": "Pad", "face": top}));
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Pad", "center": [30, 20], "radius": 5}),
    );
    let joined = run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Pad", "distance": 10, "join": "Drilled", "name": "Part"}),
    );
    assert_eq!(joined["status"], "ok", "{joined}");
    let pockets = 2.0 * PI * 16.0 * 4.0;
    assert_volume(&mut s, "Part", 19200.0 - pockets + PI * 25.0 * 10.0);
    // The plate gets longer: everything follows.
    run(
        &mut s,
        "variable.set",
        json!({"name": "length", "expr": "90 mm"}),
    );
    assert_volume(&mut s, "Part", 28800.0 - pockets + PI * 25.0 * 10.0);
    // Malformed: two targets, or one that is not there. Nothing is added.
    let before = run(&mut s, "document.inspect", json!({}));
    for (params, word) in [
        (
            json!({"sketch": "Pad", "distance": 1, "cut": "Part", "join": "Part"}),
            "not several",
        ),
        (
            json!({"sketch": "Pad", "distance": 1, "cut": "Nope"}),
            "no feature named `Nope`",
        ),
    ] {
        let e = s
            .execute("feature.extrude", &params)
            .unwrap_err()
            .to_string();
        assert!(e.contains(word), "{params}: {e}");
    }
    assert_eq!(run(&mut s, "document.inspect", json!({})), before);
}

#[test]
fn an_edge_is_referred_to_by_the_faces_it_lies_between() {
    let mut s = plate();
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Drill", "radius": 5, "height": 20, "position": [30, 20, -6]}),
    );
    boolean(&mut s, "subtract", "Plate", "Drill", "Part");
    let (top, bottom) = (
        "F4:copy(0)/F2:endCap(e5,e6,e7,e8)",
        "F4:copy(0)/F2:startCap(e5,e6,e7,e8)",
    );
    let (wall, front) = ("F4:copy(1)/F3:face(0)", "F4:copy(0)/F2:side(e5)");
    let between = |s: &mut Session, a: &str, b: &str| {
        run(
            s,
            "document.edges",
            json!({"feature": "Part", "between": [a, b]}),
        )
    };
    // The rim of the hole in the top: a new edge, where the plate's top
    // crosses the drill's wall. A circle of radius 5 at height 8.
    let rim = between(&mut s, top, wall);
    assert!(rim["count"].as_u64().unwrap() >= 1, "{rim}");
    for edge in rim["edges"].as_array().unwrap() {
        assert_eq!(edge["curve"], "circle", "{edge}");
        assert_eq!(edge["start"][2], 8.0, "{edge}");
        let (x, y) = (
            edge["start"][0].as_f64().unwrap() - 30.0,
            edge["start"][1].as_f64().unwrap() - 20.0,
        );
        assert!((x.hypot(y) - 5.0).abs() < 1e-9, "{edge}");
    }
    // The order of the two names does not matter.
    assert_eq!(between(&mut s, wall, top)["edges"], rim["edges"]);
    // The bottom rim is another edge; top and bottom share none.
    assert_eq!(between(&mut s, bottom, wall)["edges"][0]["start"][2], 0.0);
    assert_eq!(between(&mut s, top, bottom)["count"], 0);
    // An edge of the plate that the cut left alone: one line, 60 long.
    let kept = between(&mut s, top, front);
    assert_eq!(kept["count"], 1, "{kept}");
    assert_eq!(kept["edges"][0]["curve"], "line");
    // The plate changes: the same question finds the same edges.
    run(
        &mut s,
        "variable.set",
        json!({"name": "length", "expr": "90 mm"}),
    );
    assert_eq!(between(&mut s, top, wall)["count"], rim["count"]);
    assert_eq!(between(&mut s, top, front)["count"], 1);
    // Every edge of the part lies between two named faces (or is a seam).
    let all = run(&mut s, "document.edges", json!({"feature": "Part"}));
    assert_eq!(all["count"], body(&mut s, "Part")["edges"]);
    for edge in all["edges"].as_array().unwrap() {
        let faces = edge["faces"].as_array().unwrap().len();
        assert!(faces == 1 || faces == 2, "{edge}");
    }
    // What is not there, or not a face, is an error.
    for (params, word) in [
        (
            json!({"feature": "Part", "between": [top, "F4:copy(1)/F3:face(1)"]}),
            "has no face",
        ),
        (
            json!({"feature": "Part", "between": [top]}),
            "two face names",
        ),
        (
            json!({"feature": "Part", "between": [top, 7]}),
            "two face names",
        ),
        (
            json!({"feature": "Part", "between": [top, "roof"]}),
            "is not a name",
        ),
        (
            json!({"feature": "Part", "between": [top, "F2:lateralEdge(e5,e6)"]}),
            "is not a face",
        ),
        (json!({"feature": "Plate"}), "went into F4"),
        (json!({"feature": "Outline"}), "has no body"),
        (json!({}), "missing parameter `feature`"),
    ] {
        let e = s
            .execute("document.edges", &params)
            .unwrap_err()
            .to_string();
        assert!(e.contains(word), "{params}: `{word}` is not in: {e}");
    }
}

#[test]
fn curved_boolean_results_are_meshed_at_every_scale() {
    // Two results the tessellator once refused although they are valid:
    // a ball with a hole through it (a sphere face between two circles),
    // and a ball on the corner of a block in a small document.
    for k in [1e-3_f64, 1.0, 1e3] {
        let mut s = Session::new(Workspace::none());
        run(&mut s, "file.new", json!({"tolerance": 1e-6 * k}));
        run(
            &mut s,
            "feature.sphere",
            json!({"name": "Ball", "radius": 10.0 * k}),
        );
        run(
            &mut s,
            "feature.cylinder",
            json!({"name": "Rod", "radius": 6.0 * k, "height": 30.0 * k,
                   "position": [0, 0, -15.0 * k]}),
        );
        for op in ["union", "subtract"] {
            let made = boolean(&mut s, op, "Ball", "Rod", "Part");
            assert_eq!(made["status"], "ok", "{op} at {k}: {made}");
            let check = run(&mut s, "document.check", json!({"deep": true}));
            assert_eq!(check["ok"], true, "{op} at {k}: {check}");
            let picture = run(&mut s, "view.render", json!({"width": 64, "height": 48}));
            assert!(picture["coverage"].as_f64().unwrap() > 0.0, "{op} at {k}");
            run(&mut s, "edit.undo", json!({}));
        }
        // The ball less the rod through it: a napkin ring, whose volume
        // depends on its height alone: pi h^3 / 6 with h = 2 sqrt(100 - 36).
        boolean(&mut s, "subtract", "Ball", "Rod", "Ring");
        let ring = volume(&mut s, "Ring");
        let expected = PI * (16.0 * k).powi(3) / 6.0;
        assert!(
            (ring - expected).abs() <= 1e-9 * expected,
            "{ring} vs {expected}"
        );

        let mut s = Session::new(Workspace::none());
        run(&mut s, "file.new", json!({"tolerance": 1e-6 * k}));
        run(
            &mut s,
            "feature.block",
            json!({"name": "Block", "size": [40.0 * k, 30.0 * k, 20.0 * k]}),
        );
        run(
            &mut s,
            "feature.sphere",
            json!({"name": "Ball", "radius": 9.0 * k,
                   "position": [40.0 * k, 30.0 * k, 20.0 * k]}),
        );
        let made = boolean(&mut s, "union", "Block", "Ball", "Part");
        assert_eq!(made["status"], "ok", "at {k}: {made}");
        // Seven eighths of the ball stick out of the corner.
        let expected = 24000.0 * k.powi(3) + 7.0 / 8.0 * 4.0 / 3.0 * PI * (9.0 * k).powi(3);
        let part = volume(&mut s, "Part");
        assert!(
            (part - expected).abs() <= 1e-9 * expected,
            "{part} vs {expected}"
        );
        let check = run(&mut s, "document.check", json!({"deep": true}));
        assert_eq!(check["ok"], true, "at {k}: {check}");
        let picture = run(&mut s, "view.render", json!({"width": 64, "height": 48}));
        assert!(picture["coverage"].as_f64().unwrap() > 0.0, "at {k}");
    }
}

#[test]
fn the_three_operations_obey_the_volume_identities() {
    // Two blocks that overlap in a 10 x 10 x 10 corner.
    let build = |op: &str| {
        let mut s = Session::new(Workspace::none());
        run(&mut s, "file.new", json!({}));
        run(
            &mut s,
            "feature.block",
            json!({"name": "A", "size": [30, 20, 10]}),
        );
        run(
            &mut s,
            "feature.block",
            json!({"name": "B", "size": [30, 20, 40], "position": [20, 10, 0]}),
        );
        let made = boolean(&mut s, op, "A", "B", "C");
        assert_eq!(made["status"], "ok", "{op}: {made}");
        let check = run(&mut s, "document.check", json!({}));
        assert_eq!(check["ok"], true, "{op}: {check}");
        volume(&mut s, "C")
    };
    let (a, b) = (6000.0, 24000.0);
    let intersection = build("intersect");
    assert!((intersection - 1000.0).abs() < 1e-9, "{intersection}");
    assert!((build("union") - (a + b - intersection)).abs() < 1e-8);
    assert!((build("subtract") - (a - intersection)).abs() < 1e-8);
}

#[test]
fn operands_are_taken_once_unless_the_tool_is_kept() {
    let mut s = plate();
    run(
        &mut s,
        "feature.block",
        json!({"name": "Other", "size": [60, 40, 8], "position": [0, 0, 20]}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Drill", "radius": 5, "height": 40, "position": [30, 20, -5]}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Plate", "tool": "Drill", "keepTool": true, "name": "First"}),
    );
    boolean(&mut s, "subtract", "Other", "Drill", "Second");
    let drilled = 19200.0 - PI * 25.0 * 8.0;
    assert_volume(&mut s, "First", drilled);
    assert_volume(&mut s, "Second", drilled);
    assert_eq!(body_names(&mut s), ["First", "Second"]);
    // Without keepTool the first one takes the drill.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "First", "keepTool": false}),
    );
    assert_failed(
        &mut s,
        "Second",
        &["tool", "Drill", "already went into `First`"],
    );
    assert_eq!(body_names(&mut s), ["Other", "First"]);
    run(&mut s, "edit.undo", json!({}));
    // What happens to an operand happens to the result.
    run(&mut s, "feature.suppress", json!({"feature": "Drill"}));
    assert_failed(&mut s, "First", &["tool", "Drill", "suppressed"]);
    assert_eq!(body_names(&mut s), ["Plate", "Other"]);
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "feature.delete", json!({"feature": "Plate"}));
    assert_failed(&mut s, "First", &["target", "no feature F2"]);
    run(&mut s, "edit.undo", json!({}));
    assert_volume(&mut s, "First", drilled);
}

#[test]
fn what_cannot_be_combined_fails_with_the_reason() {
    let mut s = plate();
    run(
        &mut s,
        "feature.block",
        json!({"name": "Far", "size": [1, 1, 1], "position": [500, 0, 0]}),
    );
    // Nothing in common.
    boolean(&mut s, "intersect", "Plate", "Far", "Nothing");
    assert_failed(&mut s, "Nothing", &["leaves nothing"]);
    // A failed boolean takes nothing.
    assert_eq!(body_names(&mut s), ["Plate", "Far"]);
    run(&mut s, "feature.delete", json!({"feature": "Nothing"}));
    // A sketch is not a body.
    boolean(&mut s, "union", "Plate", "Outline", "Odd");
    assert_failed(&mut s, "Odd", &["tool", "Outline", "sketch", "no body"]);
    run(&mut s, "feature.delete", json!({"feature": "Odd"}));
    // A sphere: a kind of face the kernel does not combine yet. Valid or
    // an error that says so, never a wrong body.
    run(
        &mut s,
        "feature.sphere",
        json!({"name": "Ball", "radius": 10, "position": [30, 20, 8]}),
    );
    boolean(&mut s, "union", "Plate", "Ball", "Lump");
    let state = feature(&mut s, "Lump");
    if state["status"] == "failed" {
        let reason = state["error"].as_str().unwrap();
        assert!(reason.contains("does not support"), "{reason}");
        assert_eq!(body_names(&mut s), ["Plate", "Far", "Ball"]);
    } else {
        // The ball of radius 10 sits with its centre on the top of the
        // plate, which is 8 thick: half of it stands above, and a cap of
        // height 2 pokes out below, pi h^2 (3 r - h) / 3.
        let half = 2.0 / 3.0 * PI * 1000.0;
        let cap = PI * 4.0 * (30.0 - 2.0) / 3.0;
        assert_volume(&mut s, "Lump", 19200.0 + half + cap);
    }

    // Malformed requests are refused.
    for (params, words) in [
        (
            json!({"target": "Plate", "tool": "Far"}),
            vec!["missing parameter `op`"],
        ),
        (
            json!({"op": "xor", "target": "Plate", "tool": "Far"}),
            vec!["unknown op `xor`"],
        ),
        (
            json!({"op": "union", "tool": "Far"}),
            vec!["missing parameter `target`"],
        ),
        (
            json!({"op": "union", "target": "Plate"}),
            vec!["missing parameter `tool`"],
        ),
        (
            json!({"op": "union", "target": "Plate", "tool": "Plate"}),
            vec!["same feature"],
        ),
        (
            json!({"op": "union", "target": "Plate", "tool": "F2"}),
            vec!["same feature"],
        ),
        (
            json!({"op": "union", "target": "Plate", "tool": "Nope"}),
            vec!["no feature named `Nope`"],
        ),
        (
            json!({"op": 1, "target": "Plate", "tool": "Far"}),
            vec!["`op` must be"],
        ),
        (
            json!({"op": "union", "target": "Plate", "tool": "Far", "keepTool": 1}),
            vec!["keepTool"],
        ),
    ] {
        let e = s
            .execute("feature.boolean", &params)
            .unwrap_err()
            .to_string();
        for word in &words {
            assert!(e.contains(word), "{params}: `{word}` is not in: {e}");
        }
    }
}

#[test]
fn a_part_with_booleans_saves_reopens_and_regenerates_the_same() {
    let dir = std::env::temp_dir().join(format!("craft3d-booleans-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = plate();
    s.set_workspace(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Post", "radius": 6, "height": 30, "position": [30, 20, 8]}),
    );
    boolean(&mut s, "union", "Plate", "Post", "Part");
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Bore", "radius": 3, "height": 50, "position": [30, 20, -5]}),
    );
    boolean(&mut s, "subtract", "Part", "Bore", "Bored");
    assert_volume(
        &mut s,
        "Bored",
        19200.0 + PI * 36.0 * 30.0 - PI * 9.0 * 38.0,
    );
    let before = run(&mut s, "document.inspect", json!({}));
    run(&mut s, "file.save", json!({"path": "part.3dcraft"}));
    run(&mut s, "file.export", json!({"path": "part.3mf"}));
    run(&mut s, "file.open", json!({"path": "part.3dcraft"}));
    let after = run(&mut s, "document.inspect", json!({}));
    assert_eq!(after["bodies"], before["bodies"]);
    assert_eq!(after["features"], before["features"]);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The low and high ends of `name`'s box along `axis` (0 x, 1 y, 2 z).
fn span(session: &mut Session, name: &str, axis: usize) -> (f64, f64) {
    let found = body(session, name);
    (
        found["bbox"][0][axis].as_f64().unwrap(),
        found["bbox"][1][axis].as_f64().unwrap(),
    )
}

/// A pin of radius 1 and length `length` extruded from a sketch on `face`
/// (named `Pin<n>`).
fn pin_on(session: &mut Session, face: &str, n: usize, length: f64) -> String {
    let sketch = format!("Spot{n}");
    let pin = format!("Pin{n}");
    let made = run(
        session,
        "sketch.create",
        json!({"name": sketch, "face": face}),
    );
    assert_eq!(made["status"], "ok", "{made}");
    run(
        session,
        "sketch.circle",
        json!({"sketch": sketch, "center": [0, 0], "radius": 1}),
    );
    let made = run(
        session,
        "feature.extrude",
        json!({"sketch": sketch, "distance": length, "name": pin}),
    );
    assert_eq!(made["status"], "ok", "{made}");
    pin
}

#[test]
fn a_sketch_on_a_face_a_subtraction_left_looks_out_of_the_material() {
    let mut s = plate();
    // A pocket 20 x 20 x 4 in the top (floor at z = 4), and a square hole
    // through the plate.
    run(
        &mut s,
        "feature.block",
        json!({"name": "Cutter", "position": [10, 10, 4], "size": [20, 20, 10]}),
    );
    boolean(&mut s, "subtract", "Plate", "Cutter", "Pocketed");
    run(
        &mut s,
        "feature.block",
        json!({"name": "Punch", "position": [40, 10, -1], "size": [10, 10, 10]}),
    );
    boolean(&mut s, "subtract", "Pocketed", "Punch", "Part");
    assert_volume(&mut s, "Part", 60.0 * 40.0 * 8.0 - 1600.0 - 800.0);
    // A pin on the pocket's floor stands up into the pocket, as one on
    // the plate's top stands up out of it.
    let pin = pin_on(&mut s, "F6:copy(0)/F4:copy(1)/F3:face(0)", 1, 2.0);
    assert_eq!(span(&mut s, &pin, 2), (4.0, 6.0));
    let pin = pin_on(
        &mut s,
        "F6:copy(0)/F4:copy(0)/F2:endCap(e5,e6,e7,e8)",
        2,
        2.0,
    );
    assert_eq!(span(&mut s, &pin, 2), (8.0, 10.0));
    // On the pocket's front wall (y = 10, the material in front of it),
    // into the pocket.
    let pin = pin_on(&mut s, "F6:copy(0)/F4:copy(1)/F3:face(2)", 3, 3.0);
    assert_eq!(span(&mut s, &pin, 1), (10.0, 13.0));
    // On the far wall of the square hole (y = 20, the material behind
    // it), into the hole.
    let pin = pin_on(&mut s, "F6:copy(1)/F5:face(3)", 4, 3.0);
    assert_eq!(span(&mut s, &pin, 1), (17.0, 20.0));
    // A datum plane on the floor lies above it.
    let made = run(
        &mut s,
        "datum.plane",
        json!({"from": "F6:copy(0)/F4:copy(1)/F3:face(0)", "distance": 1, "name": "Above"}),
    );
    assert_eq!(made["status"], "ok", "{made}");
    let pin = {
        run(
            &mut s,
            "sketch.create",
            json!({"name": "OnAbove", "plane": "Above"}),
        );
        run(
            &mut s,
            "sketch.circle",
            json!({"sketch": "OnAbove", "center": [0, 0], "radius": 1}),
        );
        run(
            &mut s,
            "feature.extrude",
            json!({"sketch": "OnAbove", "distance": 1, "name": "PinAbove"}),
        );
        "PinAbove".to_string()
    };
    assert_eq!(span(&mut s, &pin, 2), (5.0, 6.0));
}
