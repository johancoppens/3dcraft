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
