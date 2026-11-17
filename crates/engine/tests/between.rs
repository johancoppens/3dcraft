//! The relation of two things (`measure.between`, and the assertion
//! measure `between`): what the window shows when two are selected. The
//! oracles are closed forms on a block with two holes, a chamfer and a
//! second block beside it, at three scales: the distance between parallel
//! faces, the angle where two faces meet (90 at a block's edge, 135 at a
//! 45 degree chamfer), the centre distance of two holes (from their walls
//! and from their rims, with its parts along X, Y and Z), an axis and a
//! face, a vertex and a face, two bodies; a relation the two cannot have
//! refused in words, and the check an assertion makes of it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::{run, session};
use craft3d_engine::Session;
use serde_json::{Value, json};

/// A block of 60 × 40 × 20 times `k` with holes of radius 3 along z
/// through (15, 20) and (45, 20), its top's edge along y at x = 60
/// chamfered by 8, and a block of 10 × 40 × 20 at x = 70.
fn part(k: f64) -> Session {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [60.0 * k, 40.0 * k, 20.0 * k], "name": "Block"}),
    );
    for (x, name) in [(15.0, "Left"), (45.0, "Right")] {
        run(
            &mut s,
            "feature.cylinder",
            json!({"radius": 3.0 * k, "height": 30.0 * k, "position": [x * k, 20.0 * k, -5.0 * k], "name": name}),
        );
    }
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Block", "tool": "Left", "name": "Once"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Once", "tool": "Right", "name": "Plate"}),
    );
    run(
        &mut s,
        "feature.chamfer",
        json!({"edges": [["F1:face(1)", "F1:face(5)"]], "distance": 8.0 * k, "name": "Bevel"}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"size": [10.0 * k, 40.0 * k, 20.0 * k], "position": [70.0 * k, 0, 0], "name": "Beside"}),
    );
    s
}

/// The one item of `kind` that `filter` finds, as select.get lists it.
fn item(s: &mut Session, kind: &str, filter: Value) -> Value {
    let found = run(s, "select.query", json!({"kind": kind, "where": filter}));
    assert_eq!(found["count"], 1, "{kind} {filter}: {found}");
    let it = &found["items"][0];
    json!({"kind": it["kind"], "name": it["name"], "feature": it["feature"]})
}

fn between(s: &mut Session, from: &Value, to: &Value) -> Value {
    run(s, "measure.between", json!({"from": from, "to": to}))
}

fn close(a: f64, b: f64, k: f64) -> bool {
    (a - b).abs() <= 1e-9 * k.max(1.0)
}

#[test]
fn the_relation_of_two_things_is_chosen_from_what_they_are_and_measured_exactly() {
    for k in [1e-3, 1.0, 1e3] {
        let mut s = part(k);
        let top = item(
            &mut s,
            "face",
            json!({"in": "Bevel", "normal": {"along": "z"}}),
        );
        let bottom = item(
            &mut s,
            "face",
            json!({"in": "Bevel", "normal": {"along": "-z"}}),
        );
        let left = item(
            &mut s,
            "face",
            json!({"in": "Bevel", "normal": {"along": "-x"}}),
        );
        let slant = item(
            &mut s,
            "face",
            json!({"in": "Bevel", "normal": {"along": [1, 0, 1]}}),
        );
        let side = item(
            &mut s,
            "face",
            json!({"in": "Bevel", "normal": {"along": "x"}}),
        );
        let holes = run(
            &mut s,
            "select.query",
            json!({"kind": "face", "where": {"in": "Bevel", "hole": true}}),
        );
        assert_eq!(holes["count"], 2, "{holes}");
        let hole = |i: usize| {
            let it = &holes["items"][i];
            json!({"kind": "face", "name": it["name"], "feature": it["feature"]})
        };
        // Two parallel faces: the distance between their planes.
        let got = between(&mut s, &top, &bottom);
        assert_eq!(got["relation"], "distance", "{got}");
        assert!(close(got["value"].as_f64().unwrap(), 20.0 * k, k), "{got}");
        assert_eq!(got["parallel"], true);
        assert_eq!(got["words"], "Distance between parallel faces");
        assert!(close(got["parts"][2].as_f64().unwrap().abs(), 20.0 * k, k));
        // Faces at an angle: 90 at the block's edge, 135 at the chamfer.
        let got = between(&mut s, &left, &top);
        assert_eq!(got["relation"], "angle", "{got}");
        assert!(
            (got["degrees"].as_f64().unwrap() - 90.0).abs() < 1e-9,
            "{got}"
        );
        assert!(
            (got["value"].as_f64().unwrap() - std::f64::consts::FRAC_PI_2).abs() < 1e-12,
            "{got}"
        );
        for other in [&top, &side] {
            let got = between(&mut s, &slant, other);
            assert!(
                (got["degrees"].as_f64().unwrap() - 135.0).abs() < 1e-9,
                "{k}: {got}"
            );
            // The corner lies on both planes.
            let at: Vec<f64> = got["at"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_f64().unwrap())
                .collect();
            assert!(close(at[0] + at[2], 72.0 * k, k), "{got}");
        }
        // Two holes: their centre distance, along X only.
        let got = between(&mut s, &hole(0), &hole(1));
        assert_eq!(got["relation"], "centerDistance", "{got}");
        assert_eq!(got["words"], "Centre distance");
        assert!(close(got["value"].as_f64().unwrap(), 30.0 * k, k), "{got}");
        assert_eq!(got["parallel"], true);
        let parts: Vec<f64> = got["parts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect();
        assert!(
            close(parts[0].abs(), 30.0 * k, k)
                && close(parts[1], 0.0, k)
                && close(parts[2], 0.0, k),
            "{got}"
        );
        // From their rims (circles), and a rim to the other's wall.
        let rims = run(
            &mut s,
            "select.query",
            json!({"kind": "edge", "where": {"in": "Bevel", "radius": 3.0 * k, "above": {"z": 19.0 * k}}}),
        );
        assert_eq!(rims["count"], 2, "{rims}");
        let rim = |i: usize| {
            let it = &rims["items"][i];
            json!({"kind": "edge", "name": it["name"], "feature": it["feature"]})
        };
        let got = between(&mut s, &rim(0), &rim(1));
        assert_eq!(got["relation"], "centerDistance", "{got}");
        assert!(close(got["value"].as_f64().unwrap(), 30.0 * k, k), "{got}");
        let got = between(&mut s, &rim(0), &hole(1));
        assert_eq!(got["relation"], "centerDistance", "{got}");
        let d = got["value"].as_f64().unwrap();
        assert!(close(d, 30.0 * k, k) || close(d, 0.0, k), "{got}");
        // A hole's axis and a face parallel to it: the distance between;
        // square to it: the angle.
        let got = between(&mut s, &hole(0), &left);
        assert_eq!(got["relation"], "distance", "{got}");
        let d = got["value"].as_f64().unwrap();
        assert!(close(d, 15.0 * k, k) || close(d, 45.0 * k, k), "{got}");
        let got = between(&mut s, &hole(0), &top);
        assert_eq!(got["relation"], "angle", "{got}");
        assert!(
            (got["degrees"].as_f64().unwrap() - 90.0).abs() < 1e-9,
            "{got}"
        );
        // A vertex and a face: the least distance to the face.
        let corner = item(
            &mut s,
            "vertex",
            json!({"in": "Bevel", "inside": [[-k, -k, -k], [k, k, k]]}),
        );
        let got = between(&mut s, &corner, &side);
        assert_eq!(got["relation"], "leastDistance", "{got}");
        assert!(close(got["value"].as_f64().unwrap(), 60.0 * k, k), "{got}");
        // Two bodies: the clearance between them; a face and a body: the
        // least distance. Both searched for (measure.clearance's and
        // measure.distance's), so found within the document's tolerance.
        let within = |got: &Value, want: f64| {
            let tol = got["tolerance"].as_f64().unwrap();
            (got["value"].as_f64().unwrap() - want).abs() <= tol
        };
        let got = between(&mut s, &json!("Bevel"), &json!("Beside"));
        assert_eq!(got["relation"], "clearance", "{got}");
        assert!(within(&got, 10.0 * k), "{got}");
        let got = between(&mut s, &side, &json!({"kind": "body", "feature": "Beside"}));
        assert_eq!(got["relation"], "leastDistance", "{got}");
        assert!(within(&got, 10.0 * k), "{got}");
        assert!(got["atLeast"].as_f64().unwrap() <= got["value"].as_f64().unwrap());
        assert_eq!(got["lengthUnit"], "mm");
    }
}

#[test]
fn a_relation_the_two_cannot_have_is_refused_and_an_assertion_keeps_one() {
    let mut s = part(1.0);
    let top = item(
        &mut s,
        "face",
        json!({"in": "Bevel", "normal": {"along": "z"}}),
    );
    let left = item(
        &mut s,
        "face",
        json!({"in": "Bevel", "normal": {"along": "-x"}}),
    );
    let refused = |s: &mut Session, params: Value, says: &str| {
        let error = s.execute("measure.between", &params).unwrap_err();
        assert!(error.message().contains(says), "{params}: {error}");
    };
    refused(
        &mut s,
        json!({"from": top, "to": left, "relation": "distance"}),
        "not parallel (90.000 degrees apart)",
    );
    refused(
        &mut s,
        json!({"from": top, "to": left, "relation": "clearance"}),
        "a clearance is between two bodies",
    );
    refused(
        &mut s,
        json!({"from": top, "to": left, "relation": "far"}),
        "`relation` is distance, angle",
    );
    refused(&mut s, json!({"from": top}), "`to` is required");
    refused(
        &mut s,
        json!({"from": top, "to": {"kind": "face", "name": "F1:face(99)", "feature": "Bevel"}}),
        "there is no face",
    );
    refused(
        &mut s,
        json!({"from": top, "to": {"kind": "solid", "name": "x", "feature": "Bevel"}}),
        "a kind is face, edge, vertex or body",
    );
    refused(
        &mut s,
        json!({"from": top, "to": {"kind": "face", "feature": "Bevel"}}),
        "needs the persistent name",
    );
    refused(&mut s, json!({"from": top, "to": 3}), "is an item");
    refused(
        &mut s,
        json!({"from": "Nothing", "to": top}),
        "no feature named",
    );
    // The window's check: the angle now, kept as angleBetween; between
    // is a length, and says so for two that make an angle.
    let added = run(
        &mut s,
        "assert.add",
        json!({"name": "Bevel angle", "measure": "angleBetween", "from": top, "to": left,
               "eq": "90 deg"}),
    );
    assert_eq!(added["status"], "passed", "{added}");
    let wrong = run(
        &mut s,
        "assert.add",
        json!({"name": "Not a length", "measure": "between", "from": top, "to": left,
               "eq": "90 mm"}),
    );
    assert_eq!(wrong["status"], "error", "{wrong}");
    assert!(
        wrong["message"]
            .as_str()
            .unwrap()
            .contains("the measure angleBetween"),
        "{wrong}"
    );
    run(&mut s, "assert.delete", json!({"name": "Not a length"}));
    let one = item(
        &mut s,
        "face",
        json!({"in": "Bevel", "hole": true, "inside": [[0, 0, -10], [30, 40, 30]]}),
    );
    let two = item(
        &mut s,
        "face",
        json!({"in": "Bevel", "hole": true, "inside": [[30, 0, -10], [60, 40, 30]]}),
    );
    let added = run(
        &mut s,
        "assert.add",
        json!({"name": "Holes apart", "measure": "between", "from": one, "to": two,
               "relation": "centerDistance", "eq": "30 mm", "tol": 0.001}),
    );
    assert_eq!(added["status"], "passed", "{added}");
    // The right hole moved: the check fails, saying by how much.
    let edited = run(
        &mut s,
        "feature.edit",
        json!({"feature": "Right", "position": [47, 20, -5]}),
    );
    assert!(edited.is_object());
    let listed = run(&mut s, "assert.list", json!({}));
    let row = listed["assertions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["name"] == "Holes apart")
        .unwrap()
        .clone();
    assert_eq!(row["status"], "failed", "{row}");
    assert!(
        (row["measured"].as_f64().unwrap() - 32.0).abs() < 1e-9,
        "{row}"
    );
    // Undone: passed again.
    run(&mut s, "edit.undo", json!({}));
    let listed = run(&mut s, "assert.list", json!({}));
    assert!(
        listed["assertions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|a| a["status"] == "passed"),
        "{listed}"
    );
}

/// The review of 2026-11-17 (`review-view-measure`): every pair of kinds
/// against closed forms, at three scales, on the part with a ball over it
/// and a cone beside it. Its 18 failures are here: a rim or a ball was
/// measured from its centre under the words "Least distance" (a rim to a
/// side face 15, true 12; to the face it lies on 3, true 0; a ball to the
/// top 25, true 20; a vertex to a rim 61.85, true 59.02), and two straight
/// edges got the centre distance of the lines through them (meeting at a
/// corner 0, their angle 90; skew 20, the edges 28.28 apart).
#[test]
fn every_pair_of_kinds_is_measured_as_the_things_are() {
    for k in [1e-3, 1.0, 1e6] {
        let mut s = part(k);
        run(
            &mut s,
            "feature.sphere",
            json!({"radius": 5.0 * k, "position": [30.0 * k, 20.0 * k, 45.0 * k], "name": "Ball"}),
        );
        run(
            &mut s,
            "feature.cone",
            json!({"radius": 5.0 * k, "height": 10.0 * k, "position": [100.0 * k, 100.0 * k, 0], "name": "Cone"}),
        );
        let b = |x0: f64, y0: f64, z0: f64, x1: f64, y1: f64, z1: f64| {
            json!([[x0 * k, y0 * k, z0 * k], [x1 * k, y1 * k, z1 * k]])
        };
        let mut t = std::collections::BTreeMap::new();
        let mut put = |name: &'static str, s: &mut Session, kind: &str, filter: Value| {
            t.insert(name, item(s, kind, filter));
        };
        put(
            "top",
            &mut s,
            "face",
            json!({"in": "Bevel", "normal": {"along": "z"}}),
        );
        put(
            "bottom",
            &mut s,
            "face",
            json!({"in": "Bevel", "normal": {"along": "-z"}}),
        );
        put(
            "minusx",
            &mut s,
            "face",
            json!({"in": "Bevel", "normal": {"along": "-x"}}),
        );
        put(
            "plusx",
            &mut s,
            "face",
            json!({"in": "Bevel", "normal": {"along": "x"}}),
        );
        put(
            "holeL",
            &mut s,
            "face",
            json!({"in": "Bevel", "hole": true, "inside": b(0.0, 0.0, -10.0, 30.0, 40.0, 30.0)}),
        );
        put(
            "holeR",
            &mut s,
            "face",
            json!({"in": "Bevel", "hole": true, "inside": b(30.0, 0.0, -10.0, 60.0, 40.0, 30.0)}),
        );
        put(
            "besideTop",
            &mut s,
            "face",
            json!({"in": "Beside", "normal": {"along": "z"}}),
        );
        put(
            "ball",
            &mut s,
            "face",
            json!({"in": "Ball", "surface": "sphere"}),
        );
        put(
            "cone",
            &mut s,
            "face",
            json!({"in": "Cone", "surface": "cone"}),
        );
        put(
            "E1",
            &mut s,
            "edge",
            json!({"in": "Bevel", "inside": b(-1.0, -1.0, -1.0, 61.0, 1.0, 1.0)}),
        );
        put(
            "E2",
            &mut s,
            "edge",
            json!({"in": "Bevel", "inside": b(-1.0, -1.0, -1.0, 1.0, 41.0, 1.0)}),
        );
        put(
            "E3",
            &mut s,
            "edge",
            json!({"in": "Bevel", "inside": b(-1.0, 39.0, -1.0, 61.0, 41.0, 1.0)}),
        );
        put(
            "E60",
            &mut s,
            "edge",
            json!({"in": "Bevel", "inside": b(59.0, -1.0, -1.0, 61.0, 41.0, 1.0)}),
        );
        put(
            "F",
            &mut s,
            "edge",
            json!({"in": "Beside", "inside": b(79.0, -1.0, 19.0, 81.0, 41.0, 21.0)}),
        );
        put(
            "rimTop",
            &mut s,
            "edge",
            json!({"in": "Bevel", "radius": 3.0 * k, "inside": b(0.0, 0.0, 19.0, 30.0, 40.0, 21.0)}),
        );
        put(
            "rimBottom",
            &mut s,
            "edge",
            json!({"in": "Bevel", "radius": 3.0 * k, "inside": b(0.0, 0.0, -1.0, 30.0, 40.0, 1.0)}),
        );
        put(
            "V",
            &mut s,
            "vertex",
            json!({"in": "Beside", "inside": b(69.0, -1.0, -1.0, 71.0, 1.0, 1.0)}),
        );
        put(
            "W",
            &mut s,
            "vertex",
            json!({"in": "Beside", "inside": b(79.0, 39.0, 19.0, 81.0, 41.0, 21.0)}),
        );
        t.insert("Bevel", json!("Bevel"));
        t.insert("Beside", json!("Beside"));
        t.insert("BesideBody", json!({"kind": "body", "feature": "Beside"}));
        let root = f64::sqrt;
        let rim = root(3425.0);
        let cases: Vec<(&str, &str, &str, f64)> = vec![
            ("top", "bottom", "distance", 20.0),
            ("minusx", "top", "angle", 90.0),
            ("holeL", "holeR", "centerDistance", 30.0),
            ("rimTop", "rimBottom", "centerDistance", 20.0),
            ("holeL", "ball", "centerDistance", 15.0),
            (
                "holeL",
                "cone",
                "centerDistance",
                root(85.0 * 85.0 + 80.0 * 80.0),
            ),
            ("holeL", "minusx", "distance", 15.0),
            ("holeL", "top", "angle", 90.0),
            ("V", "W", "distance", root(2100.0)),
            ("V", "top", "leastDistance", root(724.0)),
            ("V", "plusx", "leastDistance", 10.0),
            ("V", "E60", "leastDistance", 10.0),
            ("V", "Bevel", "leastDistance", 10.0),
            ("E1", "E3", "centerDistance", 40.0),
            ("E1", "top", "distance", 20.0),
            ("E1", "minusx", "angle", 90.0),
            ("plusx", "BesideBody", "leastDistance", 10.0),
            ("Bevel", "Beside", "clearance", 10.0),
            ("top", "besideTop", "distance", 0.0),
            ("top", "top", "distance", 0.0),
            // The things as they are, never their centres.
            (
                "V",
                "rimTop",
                "leastDistance",
                root((rim - 3.0) * (rim - 3.0) + 400.0),
            ),
            ("rimTop", "minusx", "leastDistance", 12.0),
            ("rimTop", "top", "leastDistance", 0.0),
            ("ball", "top", "leastDistance", 20.0),
            // Two straight edges: meeting, their angle; skew, the edges'
            // least distance.
            ("E1", "E2", "angle", 90.0),
            ("E1", "F", "leastDistance", root(800.0)),
        ];
        for (a, b, relation, want) in cases {
            let got = between(&mut s, &t[a], &t[b]);
            let rel = got["relation"].as_str().unwrap_or("?");
            assert_eq!(rel, relation, "k={k} {a}-{b}: {got}");
            let (value, want) = if rel == "angle" {
                (got["degrees"].as_f64().unwrap(), want)
            } else {
                (got["value"].as_f64().unwrap(), want * k)
            };
            let tol = if rel == "angle" {
                1e-9
            } else {
                got["tolerance"].as_f64().unwrap().max(1e-9 * want.abs())
            };
            assert!(
                (value - want).abs() <= tol,
                "k={k} {a}-{b}: {value} against {want}: {got}"
            );
        }
        // The skew edges' line is drawn between the points of the edges.
        let got = between(&mut s, &t["E1"], &t["F"]);
        let from: Vec<f64> = got["from"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect();
        assert!(
            close(from[0], 60.0 * k, k) && close(from[2], 0.0, k),
            "{got}"
        );
        // Two holes: the line where they open (z = 20), not at mid-depth.
        let got = between(&mut s, &t["holeL"], &t["holeR"]);
        for end in ["from", "to"] {
            let z = got[end][2].as_f64().unwrap();
            assert!(close(z, 20.0 * k, k), "{end}: {got}");
        }
    }
}
