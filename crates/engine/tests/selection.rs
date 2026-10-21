//! The selection shared by the window and agents, and selecting by
//! describing (`select.get`, `select.set`, `select.query`; decisions B1
//! and B4 of 2026-10-21). The oracles are closed-form: how many holes a
//! plate was drilled with, which face of a block faces -y and its area,
//! which bodies were placed above a height.

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
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn refused(session: &mut Session, id: &str, params: Value) -> String {
    match session.execute(id, &params) {
        Ok(value) => panic!("{id} {params} should fail, returned {value}"),
        Err(e) => {
            let message = e.message().to_string();
            assert!(
                message.starts_with(id),
                "the error names the command: {message}"
            );
            message
        }
    }
}

fn names(result: &Value) -> Vec<String> {
    result["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["name"].as_str().unwrap().to_string())
        .collect()
}

/// A plate 80 × 50 × 8 with four holes of 6 mm (a pattern of a cylinder
/// of radius 3) and one of 10 mm, drilled by subtracting.
fn plate() -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": [80, 50, 8]}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Drill", "radius": "3 mm", "height": 20, "position": [10, 10, -5]}),
    );
    run(
        &mut s,
        "feature.pattern",
        json!({"name": "Drills", "source": "Drill", "count": 4, "step": [20, 0, 0]}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Bore", "radius": "5 mm", "height": 20, "position": [40, 35, -5]}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Plate", "tool": "Drills", "name": "Drilled"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Drilled", "tool": "Bore", "name": "Part"}),
    );
    s
}

#[test]
fn the_six_mm_holes_are_selected_and_made_six_and_a_half() {
    let mut s = plate();
    let six = json!({"kind": "face", "where": {"hole": true, "radius": "3 mm"}, "then": "replace"});
    let found = run(&mut s, "select.query", six.clone());
    assert_eq!(found["count"], 4, "{found}");
    assert_eq!(found["summary"], "4 faces in Part");
    // Every one was made by the drill the pattern copied.
    for item in found["items"].as_array().unwrap() {
        assert_eq!(item["kind"], "face");
        assert_eq!(item["body"], "Part");
        assert_eq!(item["origin"]["name"], "Drill", "{item}");
    }
    let selected = run(&mut s, "select.get", json!({}));
    assert_eq!(selected["count"], 4);
    assert_eq!(selected["summary"], "4 faces");
    assert_eq!(names(&selected), names(&found), "in document order");
    // The 10 mm hole and the outside of the plate are not holes of 6 mm;
    // the bore is a hole of radius 5.
    let bore = run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"hole": true, "radius": {"min": 4, "max": 6}}}),
    );
    assert_eq!(bore["count"], 1);
    assert_eq!(bore["items"][0]["origin"]["name"], "Bore");
    let not_holes = run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"hole": false}}),
    );
    assert_eq!(not_holes["count"], 6, "the six faces of the plate");

    // Made 6.5 mm by editing what made them: the origin of the faces.
    let origin = found["items"][0]["origin"]["feature"]
        .as_str()
        .unwrap()
        .to_string();
    run(
        &mut s,
        "feature.edit",
        json!({"feature": origin, "radius": "3.25 mm"}),
    );
    // The names stay selected: they name the same faces after the edit.
    let after = run(&mut s, "select.get", json!({}));
    assert_eq!(after["count"], 4, "{after}");
    assert_eq!(after["dropped"], json!([]));
    assert_eq!(names(&after), names(&found));
    assert_eq!(run(&mut s, "select.query", six)["count"], 0);
    let wider = run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"hole": true, "radius": "6.5 mm / 2"}}),
    );
    assert_eq!(names(&wider), names(&found));
    // Each wall's area is the circumference times the plate's thickness.
    let walls = run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"hole": true, "area": {"eq": 2.0 * PI * 3.25 * 8.0, "tol": 1e-6}}}),
    );
    assert_eq!(walls["count"], 4, "{walls}");
}

#[test]
fn the_face_of_a_block_facing_minus_y_is_its_back() {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Cube", "size": [10, 20, 30], "position": [5, 5, 5]}),
    );
    let back = run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"normal": {"along": "-y"}}}),
    );
    assert_eq!(back["count"], 1, "{back}");
    // It lies in the plane y = 5 and has the area 10 × 30.
    let checked = run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"normal": {"along": [0, -1, 0]}, "area": 300, "below": {"y": 5}}}),
    );
    assert_eq!(names(&checked), names(&back));
    // Four faces are square to z, two lie along it.
    let sides = run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"normal": {"perpendicular": "z"}}}),
    );
    assert_eq!(sides["count"], 4);
    let caps = run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"any": [{"normal": {"along": "z"}}, {"normal": {"along": "-z"}}]}}),
    );
    assert_eq!(caps["count"], 2);
    let not_caps = run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"not": {"any": [{"normal": {"along": "z"}}, {"normal": {"along": "-z"}}]}}}),
    );
    assert_eq!(not_caps["count"], 4);
    // Twelve edges, each 10, 20 or 30 long; four of 20.
    let edges = run(
        &mut s,
        "select.query",
        json!({"kind": "edge", "where": {"length": "2 cm"}}),
    );
    assert_eq!(edges["count"], 4, "{edges}");
    let volume = run(
        &mut s,
        "select.query",
        json!({"kind": "body", "where": {"volume": {"min": 5999, "max": 6001}}}),
    );
    assert_eq!(names(&volume), ["Cube"]);
}

#[test]
fn the_bodies_above_a_height_are_coloured_red() {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({}));
    for (name, z) in [("Low", 0), ("Mid", 15), ("High", 30), ("Top", 45)] {
        run(
            &mut s,
            "feature.block",
            json!({"name": name, "size": [10, 10, 10], "position": [0, 0, z]}),
        );
    }
    run(
        &mut s,
        "feature.sphere",
        json!({"name": "Ball", "radius": 4, "position": [30, 0, 40]}),
    );
    let above = run(
        &mut s,
        "select.query",
        json!({"kind": "body", "where": {"above": {"z": 20}}, "then": "replace"}),
    );
    assert_eq!(names(&above), ["High", "Top", "Ball"]);
    assert_eq!(above["selection"]["summary"], "3 bodies");
    for item in above["items"].as_array().unwrap() {
        run(
            &mut s,
            "body.appearance",
            json!({"body": item["feature"], "color": "#ff0000"}),
        );
    }
    let red = run(
        &mut s,
        "select.query",
        json!({"kind": "body", "where": {"color": [250, 5, 0], "within": 10}}),
    );
    assert_eq!(names(&red), ["High", "Top", "Ball"]);
    let plain = run(
        &mut s,
        "select.query",
        json!({"kind": "body", "where": {"color": null}}),
    );
    assert_eq!(names(&plain), ["Low", "Mid"]);
    // A face of a red body is red too; a box finds the low ones.
    let red_faces = run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"color": "#ff0000", "surface": "sphere"}}),
    );
    assert_eq!(red_faces["count"], 1);
    let boxed = run(
        &mut s,
        "select.query",
        json!({"kind": "body", "where": {"inside": [[-1, -1, -1], [11, 11, "26 mm"]]}}),
    );
    assert_eq!(names(&boxed), ["Low", "Mid"]);
    let named = run(
        &mut s,
        "select.query",
        json!({"kind": "body", "where": {"name": "?i*"}}),
    );
    assert_eq!(names(&named), ["Mid", "High"]);
}

#[test]
fn the_selection_is_set_toggled_and_drops_what_is_gone() {
    let mut s = plate();
    let set = run(
        &mut s,
        "select.set",
        json!({"names": ["Part", "Drill", "F1"]}),
    );
    // A body, the drill (which went into the pattern: a feature) and the
    // plate (which went into the boolean).
    let kinds: Vec<&str> = set["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["body", "feature", "feature"]);
    assert_eq!(set["summary"], "1 body, 2 features");
    let face = run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"normal": {"along": "z"}}}),
    )["items"][0]["name"]
        .as_str()
        .unwrap()
        .to_string();
    let toggled = run(
        &mut s,
        "select.set",
        json!({"items": [{"kind": "face", "name": face, "feature": "Part"}, {"name": "Drill"}], "mode": "toggle"}),
    );
    assert_eq!(toggled["summary"], "1 body, 1 face, 1 feature");
    assert_eq!(
        (toggled["added"].as_u64(), toggled["removed"].as_u64()),
        (Some(1), Some(1))
    );
    // Adding what is selected adds nothing; removing takes it out.
    let again = run(
        &mut s,
        "select.set",
        json!({"names": [face], "mode": "add"}),
    );
    assert_eq!(again["added"], 0);
    let removed = run(
        &mut s,
        "select.set",
        json!({"names": ["Part"], "mode": "remove"}),
    );
    assert_eq!(removed["summary"], "1 face, 1 feature");
    // The face goes with the last boolean: it drops out, said once.
    run(&mut s, "feature.delete", json!({"feature": "Part"}));
    let after = run(&mut s, "select.get", json!({}));
    assert_eq!(after["summary"], "1 feature", "{after}");
    let dropped = after["dropped"].as_array().unwrap();
    assert_eq!(dropped.len(), 1, "{after}");
    assert!(dropped[0].as_str().unwrap().starts_with("face "), "{after}");
    // Undoing brings the body back, not the selection: it is not history.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(run(&mut s, "select.get", json!({}))["summary"], "1 feature");
    assert_eq!(
        run(&mut s, "select.set", json!({"mode": "clear"}))["count"],
        0
    );
}

#[test]
fn queries_and_selections_refuse_what_means_nothing() {
    let mut s = plate();
    let before = run(&mut s, "select.set", json!({"names": ["Part"]}));
    for (id, params, says) in [
        (
            "select.set",
            json!({"names": ["Nothing"]}),
            "neither a feature",
        ),
        (
            "select.set",
            json!({"names": ["F9:face(0)"]}),
            "no body has",
        ),
        (
            "select.set",
            json!({"items": [{"kind": "body", "name": "Drill"}]}),
            "went into",
        ),
        (
            "select.set",
            json!({"items": [{"kind": "sketch", "name": "Part"}]}),
            "not a sketch",
        ),
        (
            "select.set",
            json!({"items": [{"kind": "blob", "name": "Part"}]}),
            "`kind`",
        ),
        (
            "select.set",
            json!({"items": [{"name": "Part", "colour": 1}]}),
            "not `colour`",
        ),
        ("select.set", json!({"items": "Part"}), "array"),
        ("select.set", json!({"names": [3]}), "strings"),
        ("select.set", json!({"mode": "add"}), "give `items`"),
        (
            "select.set",
            json!({"mode": "clear", "names": ["Part"]}),
            "takes no items",
        ),
        ("select.set", json!({"mode": "swap"}), "`mode`"),
        ("select.query", json!({"kind": "lump"}), "`kind`"),
        (
            "select.query",
            json!({"where": {"radius": "3 deg"}}),
            "length",
        ),
        (
            "select.query",
            json!({"where": {"area": "3 mm"}}),
            "squared",
        ),
        (
            "select.query",
            json!({"where": {"volume": {"eq": 1, "min": 0}}}),
            "{eq, tol}",
        ),
        (
            "select.query",
            json!({"where": {"normal": {"along": [0, 0, 0]}}}),
            "direction",
        ),
        (
            "select.query",
            json!({"where": {"normal": {"along": "w"}}}),
            "an axis",
        ),
        (
            "select.query",
            json!({"where": {"normal": {"along": "z", "tol": 120}}}),
            "0 to 90",
        ),
        (
            "select.query",
            json!({"where": {"color": "#ff00"}}),
            "#rrggbb",
        ),
        (
            "select.query",
            json!({"where": {"color": [300, 0, 0]}}),
            "#rrggbb",
        ),
        (
            "select.query",
            json!({"where": {"within": 3}}),
            "goes with `color`",
        ),
        (
            "select.query",
            json!({"where": {"madeBy": "Nobody"}}),
            "no feature",
        ),
        (
            "select.query",
            json!({"where": {"inside": [[0, 0, 0]]}}),
            "a box",
        ),
        (
            "select.query",
            json!({"where": {"above": {"z": 1, "x": 2}}}),
            "one of x, y and z",
        ),
        (
            "select.query",
            json!({"where": {"wobble": 1}}),
            "unknown filter",
        ),
        ("select.query", json!({"where": 3}), "an object or an array"),
        ("select.query", json!({"where": {}}), "at least one"),
        (
            "select.query",
            json!({"where": {"all": 3}}),
            "array of filters",
        ),
        ("select.query", json!({"then": "maybe"}), "`then`"),
        ("select.query", json!({"limit": 0}), "`limit`"),
        ("select.query", json!({"limit": 100000}), "`limit`"),
        ("select.query", json!({"surface": "plane"}), "unknown"),
    ] {
        let message = refused(&mut s, id, params.clone());
        assert!(message.contains(says), "{id} {params}: {message}");
    }
    // Nested too deep, and too many filters.
    let mut deep = json!({"kind": "face"});
    for _ in 0..40 {
        deep = json!({"not": deep});
    }
    assert!(refused(&mut s, "select.query", json!({"where": deep})).contains("nested"));
    let many: Vec<Value> = (0..1200).map(|_| json!({"kind": "face"})).collect();
    assert!(refused(&mut s, "select.query", json!({"where": many})).contains("at most"));
    // Nothing of it changed the selection.
    assert_eq!(
        run(&mut s, "select.get", json!({}))["items"],
        before["items"]
    );
    // Without a document, every one of them is refused.
    let mut empty = Session::new(Workspace::none());
    for id in ["select.get", "select.set", "select.query"] {
        assert!(empty.execute(id, &json!({})).is_err());
    }
}

#[test]
fn a_large_result_is_cut_at_its_limit_and_says_so() {
    let mut s = plate();
    let all = run(
        &mut s,
        "select.query",
        json!({"kind": ["face", "edge", "vertex"]}),
    );
    let total = all["count"].as_u64().unwrap();
    assert!(total > 20, "{total}");
    let cut = run(
        &mut s,
        "select.query",
        json!({"kind": ["face", "edge", "vertex"], "limit": 5, "then": "replace"}),
    );
    assert_eq!(cut["count"].as_u64(), Some(total));
    assert_eq!(cut["truncated"], true);
    assert_eq!(cut["items"].as_array().unwrap().len(), 5);
    assert!(
        cut["summary"].as_str().unwrap().contains("the first 5"),
        "{cut}"
    );
    assert_eq!(cut["selection"]["count"], 5);
    // The same query twice gives the same answer, in the same order.
    assert_eq!(
        run(
            &mut s,
            "select.query",
            json!({"kind": ["face", "edge", "vertex"]})
        ),
        all
    );
}

#[test]
fn origins_and_copies_find_what_a_feature_made() {
    let mut s = plate();
    // The faces the pattern's copies left in the part: four walls.
    let copies = run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"copyOf": "Drills"}}),
    );
    assert_eq!(copies["count"], 4, "{copies}");
    let second = run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"copyOf": "Drills", "instance": 1}}),
    );
    assert_eq!(second["count"], 1, "{second}");
    let by_bore = run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"madeBy": "Bore"}}),
    );
    assert_eq!(by_bore["count"], 1);
    // The part is made from everything before it.
    let made = run(
        &mut s,
        "select.query",
        json!({"kind": "body", "where": {"madeBy": "Drill"}}),
    );
    assert_eq!(names(&made), ["Part"]);
    let typed = run(
        &mut s,
        "select.query",
        json!({"kind": "feature", "where": {"type": "boolean"}}),
    );
    assert_eq!(names(&typed), ["Drilled", "Part"]);
    let inside = run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"in": "Part", "surface": ["cylinder", "cone"], "axis": {"along": "-z"}}}),
    );
    assert_eq!(inside["count"], 5);
    let circles = run(
        &mut s,
        "select.query",
        json!({"kind": "edge", "where": {"radius": 5}}),
    );
    assert_eq!(circles["count"], 2, "the bore's two rims: {circles}");
}

/// Two blocks one behind the other, seen from the front: a pick with
/// `all` goes through both, front to back, one hit per face.
#[test]
fn a_pick_through_lists_every_face_along_the_ray_front_to_back() {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Front", "size": [10, 10, 10]}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Back", "size": [10, 10, 10], "position": [0, 20, 0]}),
    );
    let at =
        json!({"x": 400.5, "y": 300.5, "view": "front", "projection": "orthographic", "all": true});
    let picked = run(&mut s, "view.pick", at.clone());
    let hits = picked["hits"].as_array().unwrap();
    let bodies: Vec<&str> = hits.iter().map(|h| h["name"].as_str().unwrap()).collect();
    assert_eq!(bodies, ["Front", "Front", "Back", "Back"], "{picked}");
    // The first is the face the pick gives; depths grow; the front faces
    // of the blocks are at y = 0 and y = 20, their backs at 10 and 30.
    assert_eq!(hits[0]["face"], picked["face"]);
    let ys: Vec<f64> = hits
        .iter()
        .map(|h| h["point"][1].as_f64().unwrap())
        .collect();
    for (y, want) in ys.iter().zip([0.0, 10.0, 20.0, 30.0]) {
        assert!((y - want).abs() < 1e-6, "{ys:?}");
    }
    let depths: Vec<f64> = hits.iter().map(|h| h["depth"].as_f64().unwrap()).collect();
    assert!(depths.windows(2).all(|w| w[0] < w[1]), "{depths:?}");
    // Each hit names its face: the one facing -y of each block first.
    let back_faces = run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"normal": {"along": "-y"}}}),
    );
    assert_eq!(hits[0]["face"], back_faces["items"][0]["name"]);
    assert_eq!(hits[2]["face"], back_faces["items"][1]["name"]);
    // Without all, no list; beside the blocks, an empty one.
    assert!(
        run(&mut s, "view.pick", json!({"x": 400.5, "y": 300.5}))
            .get("hits")
            .is_none()
    );
    let beside = run(
        &mut s,
        "view.pick",
        json!({"x": 2.5, "y": 2.5, "view": "front", "all": true}),
    );
    assert_eq!(beside["hits"], json!([]));
    assert!(
        s.execute("view.pick", &json!({"x": 1, "y": 1, "all": "yes"}))
            .is_err()
    );
}

/// The housing of the showcase (`docs/showcase/housing.3dcraft`): six
/// bolt holes of 7 mm (a pattern of three, mirrored) and two oil holes of
/// 5 mm, by description.
#[test]
fn the_housings_bolt_and_oil_holes_are_found_by_their_size() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/showcase/housing.3dcraft"
    );
    let mut s = Session::new(Workspace::unrestricted());
    run(&mut s, "file.open", json!({"path": path}));
    let bolts = run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"hole": true, "radius": "7 mm / 2"}, "then": "replace"}),
    );
    assert_eq!(bolts["count"], 6, "{bolts}");
    assert_eq!(bolts["summary"], "6 faces in Housing");
    for item in bolts["items"].as_array().unwrap() {
        assert_eq!(item["origin"]["name"], "Hole", "{item}");
    }
    let oil = run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"hole": true, "radius": "2.5 mm", "madeBy": "Oiled"}}),
    );
    assert_eq!(oil["count"], 2, "{oil}");
    // The mirrored three are the copies of the mirror.
    let left = run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"hole": true, "copyOf": "HolesLeft"}}),
    );
    assert_eq!(left["count"], 3, "{left}");
    assert_eq!(run(&mut s, "select.get", json!({}))["count"], 6);
}
