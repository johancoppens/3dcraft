//! `feature.plate` (docs/design/plates.md) driven by commands, at 1e-3, 1
//! and 1e6: a floor and a wall in an L, each a fill of a rectangle, made
//! into plates with every joint; held to the closed-form volumes, to
//! `document.interference` (the two plates touch, on the mitre face or
//! the lap with the closed-form area, and never overlap), to a variable
//! that changes the thickness, to `feature.edit`, to a save and a reopen,
//! and to refusals as errors or failed features.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::*;
use craft3d_engine::Session;
use serde_json::{Value, json};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

/// The L: a floor on xy (`x` 0..a, `y` 0..l) and a wall on yz (`y` 0..l,
/// `z` 0..b), both facing their plane's normal (+z, +x): plates to the
/// side out lie inside the corner. Returns the names of the edges they
/// share (the floor's and the wall's).
fn l_shape(s: &mut Session, a: f64, b: f64, l: f64, k: f64) -> (String, String) {
    run(
        s,
        "variable.set",
        json!({"name": "t", "expr": format!("{} mm", 0.25 * k)}),
    );
    run(
        s,
        "sketch.create",
        json!({"plane": "xy", "name": "FloorSketch"}),
    );
    let floor = run(
        s,
        "sketch.rect",
        json!({"sketch": "FloorSketch", "from": [0.0, 0.0], "to": [a * k, l * k]}),
    );
    built(
        s,
        "feature.fill",
        json!({"sketch": "FloorSketch", "name": "FloorSheet"}),
    );
    run(
        s,
        "sketch.create",
        json!({"plane": "yz", "name": "WallSketch"}),
    );
    let wall = run(
        s,
        "sketch.rect",
        json!({"sketch": "WallSketch", "from": [0.0, 0.0], "to": [l * k, b * k]}),
    );
    built(
        s,
        "feature.fill",
        json!({"sketch": "WallSketch", "name": "WallSheet"}),
    );
    // The floor's fourth line runs along x = 0; the wall's first along
    // z = 0.
    let floor_edge = format!("FloorSheet:startEdge({})", str_of(&floor["lines"][3]));
    let wall_edge = format!("WallSheet:startEdge({})", str_of(&wall["lines"][0]));
    (floor_edge, wall_edge)
}

fn joint(edge: &str, kind: &str, with: &str) -> Value {
    json!({"edge": edge, "kind": kind, "with": with})
}

fn plates(s: &mut Session, edges: &(String, String), floor: &str, wall: &str) {
    built(
        s,
        "feature.plate",
        json!({"faces": ["FloorSheet:startCap()"], "thickness": "t", "side": "out",
               "joints": [joint(&edges.0, floor, "WallSheet:startCap()")], "name": "Floor"}),
    );
    built(
        s,
        "feature.plate",
        json!({"faces": ["WallSheet:startCap()"], "thickness": "t", "side": "out",
               "joints": [joint(&edges.1, wall, "FloorSheet:startCap()")], "name": "Wall"}),
    );
}

fn volume(s: &mut Session, name: &str) -> f64 {
    let found = body(s, name);
    assert_eq!(found["valid"], true, "{found}");
    found["volume"].as_f64().unwrap()
}

#[track_caller]
fn near(actual: f64, expected: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= 1e-9 * expected.abs(),
        "{what}: {actual} is not {expected}"
    );
}

/// What `document.interference` says of the floor and the wall.
fn contact(s: &mut Session) -> Value {
    let found = run(
        s,
        "document.interference",
        json!({"pairs": [["Floor", "Wall"]]}),
    );
    found["pairs"][0].clone()
}

#[test]
fn plates_in_an_l_meet_by_each_joint_without_overlap() {
    let (a, b, l, t) = (3.0, 2.0, 1.5, 0.25);
    for k in SCALES {
        let k3 = k * k * k;
        // Mitre: trapezoids that meet on the bisector, a face of t √2 by l.
        let mut s = session();
        new_document(&mut s, k);
        let edges = l_shape(&mut s, a, b, l, k);
        plates(&mut s, &edges, "mitre", "mitre");
        near(
            volume(&mut s, "Floor"),
            l * (a * t - t * t / 2.0) * k3,
            "mitre floor",
        );
        near(
            volume(&mut s, "Wall"),
            l * (b * t - t * t / 2.0) * k3,
            "mitre wall",
        );
        let c = contact(&mut s);
        assert_eq!(c["status"], "touch", "{c}");
        assert_eq!(c["contact"], "face", "{c}");
        near(
            c["area"].as_f64().unwrap(),
            l * t * 2f64.sqrt() * k * k,
            "mitre face",
        );
        deep(&mut s);
        // Over and under: the floor covers the wall's edge, the lap a
        // face of t by l.
        let mut s = session();
        new_document(&mut s, k);
        let edges = l_shape(&mut s, a, b, l, k);
        plates(&mut s, &edges, "over", "under");
        near(volume(&mut s, "Floor"), l * a * t * k3, "floor over");
        near(volume(&mut s, "Wall"), l * (b - t) * t * k3, "wall under");
        let c = contact(&mut s);
        assert_eq!(c["status"], "touch", "{c}");
        near(c["area"].as_f64().unwrap(), l * t * k * k, "lap face");
        // Touch: the inner edges meet along a line.
        let mut s = session();
        new_document(&mut s, k);
        let edges = l_shape(&mut s, a, b, l, k);
        plates(&mut s, &edges, "touch", "touch");
        near(volume(&mut s, "Floor"), l * (a - t) * t * k3, "touch floor");
        near(volume(&mut s, "Wall"), l * (b - t) * t * k3, "touch wall");
        let c = contact(&mut s);
        assert_eq!(c["status"], "touch", "{c}");
        assert_eq!(c["contact"], "edge", "{c}");
    }
}

#[test]
fn a_plate_follows_its_sheets_through_the_features_that_take_them() {
    // The sheets are moved after they were made; the plates name the
    // faces, the joints' edges and the neighbours by the sheets' own
    // names, which are followed to the moved bodies (rule 4).
    let (a, b, l, t) = (3.0, 2.0, 1.5, 0.25);
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        let edges = l_shape(&mut s, a, b, l, k);
        for sheet in ["FloorSheet", "WallSheet"] {
            built(
                &mut s,
                "feature.transform",
                json!({"source": sheet, "translate": [k, 2.0 * k, 0.5 * k], "name": format!("Moved{sheet}")}),
            );
        }
        plates(&mut s, &edges, "mitre", "mitre");
        let k3 = k * k * k;
        near(
            volume(&mut s, "Floor"),
            l * (a * t - t * t / 2.0) * k3,
            "followed mitre floor",
        );
        near(
            volume(&mut s, "Wall"),
            l * (b * t - t * t / 2.0) * k3,
            "followed mitre wall",
        );
        let c = contact(&mut s);
        assert_eq!(c["status"], "touch", "{c}");
        near(
            c["area"].as_f64().unwrap(),
            l * t * 2f64.sqrt() * k * k,
            "followed mitre face",
        );
    }
}

/// The joints of `name` as built (`followed`), when it has them.
fn built_joints(s: &mut Session, name: &str) -> Option<usize> {
    let found = feature(s, name);
    let followed = &found["followed"];
    let joints = if followed["joints"].is_array() {
        &followed["joints"]
    } else {
        &followed["plate"]["joints"]
    };
    joints.as_array().map(Vec::len)
}

#[test]
fn an_end_lies_on_a_datum_plane_or_on_a_face() {
    let (a, b, l) = (3.0, 2.0, 1.5);
    let t = 0.25;
    let mut s = session();
    new_document(&mut s, 1.0);
    let edges = l_shape(&mut s, a, b, l, 1.0);
    // The floor's edge along x = a: its rectangle's second line (the
    // first four entities are the corners).
    let far = "FloorSheet:startEdge(e6)".to_string();
    assert_eq!(edges.0, "FloorSheet:startEdge(e8)", "the rectangle's ids");
    // A datum plane through that edge, leaning out by 0.1 per unit up.
    built(
        &mut s,
        "datum.plane",
        json!({"through": [[a, 0, 0], [a, l, 0], [a + 0.1, 0, 1]], "name": "Lean"}),
    );
    // The wall's sheet moved (by nothing) before the plate: the face the
    // other end lies on is followed to the moved body.
    built(
        &mut s,
        "feature.transform",
        json!({"source": "WallSheet", "translate": [0, 0, 0], "name": "Placed"}),
    );
    built(
        &mut s,
        "feature.plate",
        json!({"faces": ["FloorSheet:startCap()"], "thickness": "t", "side": "out", "name": "Floor",
               "joints": [{"edge": far, "kind": "onto", "onto": "Lean"},
                          {"edge": edges.0, "kind": "onto", "onto": "WallSheet:startCap()"}]}),
    );
    // The lean adds l ∫0^t 0.1 z dz; the end on the wall's face is square.
    near(
        volume(&mut s, "Floor"),
        l * (a * t + 0.05 * t * t),
        "ends on a datum plane and a face",
    );
    assert_eq!(built_joints(&mut s, "Floor"), Some(2));
    let followed = feature(&mut s, "Floor")["followed"].to_string();
    assert!(
        followed.contains("copy"),
        "the face is followed to the moved sheet: {followed}"
    );
    // The wall's sheet switched off: its face is absent, the joint is left
    // out, and that edge ends square (the same volume here).
    run(&mut s, "feature.suppress", json!({"feature": "WallSheet"}));
    assert_eq!(feature(&mut s, "Floor")["status"], "ok");
    near(
        volume(&mut s, "Floor"),
        l * (a * t + 0.05 * t * t),
        "the joint left out",
    );
    assert_eq!(built_joints(&mut s, "Floor"), Some(1));
    run(
        &mut s,
        "feature.suppress",
        json!({"feature": "WallSheet", "suppressed": false}),
    );
    // Refused with the reason: a surface that runs along the plate, one
    // far from the edge, and onto without its surface (or the other way).
    let reason = failed(
        &mut s,
        "feature.plate",
        json!({"faces": ["FloorSheet:startCap()"], "thickness": "t", "joints": [{"edge": far, "kind": "onto", "onto": "xy"}]}),
    );
    assert!(reason.contains("cuts no end"), "{reason}");
    let reason = failed(
        &mut s,
        "feature.plate",
        json!({"faces": ["FloorSheet:startCap()"], "thickness": "t", "joints": [{"edge": far, "kind": "onto", "onto": "yz"}]}),
    );
    assert!(reason.contains("away from the edge"), "{reason}");
    for params in [
        json!({"faces": ["FloorSheet:startCap()"], "thickness": "t", "joints": [{"edge": far, "kind": "onto"}]}),
        json!({"faces": ["FloorSheet:startCap()"], "thickness": "t", "joints": [{"edge": far, "kind": "square", "onto": "xy"}]}),
    ] {
        let message = refused(&mut s, "feature.plate", params.clone());
        assert!(message.contains("onto"), "{params}: {message}");
    }
}

#[test]
fn a_plate_follows_its_variables_edits_and_reopens() {
    let (a, b, l) = (3.0, 2.0, 1.5);
    let k = 1.0;
    let dir = workdir("plate", "round-trip");
    let mut s = session_in(&dir);
    new_document(&mut s, k);
    let edges = l_shape(&mut s, a, b, l, k);
    plates(&mut s, &edges, "mitre", "mitre");
    run(
        &mut s,
        "variable.set",
        json!({"name": "t", "expr": "0.5 mm"}),
    );
    near(
        volume(&mut s, "Floor"),
        l * (a * 0.5 - 0.125),
        "thicker floor",
    );
    // A square end by editing the joints away.
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "Floor", "joints": [], "side": "mid"}),
    );
    near(
        volume(&mut s, "Floor"),
        l * a * 0.5,
        "square floor in the middle",
    );
    // The faces' names: copies of the sheet's face and rims.
    let floor = body(&mut s, "Floor");
    let names = names_of(&floor);
    assert!(names.iter().any(|n| n.contains("copy(0)/")), "{names:?}");
    assert!(names.iter().any(|n| n.contains("copy(1)/")), "{names:?}");
    assert_eq!(
        names.iter().filter(|n| n.contains(":rim(")).count(),
        4,
        "{names:?}"
    );
    run(&mut s, "file.save", json!({"path": "plates.3dcraft"}));
    let before = (volume(&mut s, "Floor"), volume(&mut s, "Wall"));
    let mut t = session_in(&dir);
    run(&mut t, "file.open", json!({"path": "plates.3dcraft"}));
    assert_eq!((volume(&mut t, "Floor"), volume(&mut t, "Wall")), before);
    assert_eq!(names_of(&body(&mut t, "Floor")), names);
}

#[test]
fn what_a_plate_refuses() {
    let mut s = session();
    new_document(&mut s, 1.0);
    let edges = l_shape(&mut s, 3.0, 2.0, 1.5, 1.0);
    let face = "FloorSheet:startCap()";
    for (params, words) in [
        (json!({"thickness": "t"}), "faces"),
        (json!({"faces": [face]}), "thickness"),
        (
            json!({"faces": [face], "thickness": "t", "side": "up"}),
            "side",
        ),
        (
            json!({"faces": [face], "thickness": "t", "joints": {}}),
            "joints",
        ),
        (
            json!({"faces": [face], "thickness": "t", "joints": [{"kind": "mitre"}]}),
            "edge",
        ),
        (
            json!({"faces": [face], "thickness": "t", "joints": [{"edge": edges.0, "kind": "glue"}]}),
            "kind",
        ),
        (
            json!({"faces": [face], "thickness": "t", "joints": [{"edge": edges.0, "kind": "mitre"}]}),
            "with",
        ),
        (
            json!({"faces": [face], "thickness": "t", "joints": [{"edge": edges.0, "kind": "bevel"}]}),
            "angle",
        ),
        (
            json!({"faces": [face], "thickness": "t", "joints": [{"edge": face, "kind": "square"}]}),
            "not an edge",
        ),
        (
            json!({"faces": [face], "thickness": "t", "joints": [{"edge": edges.0, "kind": "square", "with": face}]}),
            "no neighbour",
        ),
        (
            json!({"faces": [face], "thickness": "t", "joints": [{"edge": edges.0, "kind": "square", "colour": 1}]}),
            "colour",
        ),
        (
            json!({"faces": [face], "thickness": "t", "joints": [{"edge": edges.0, "kind": "square"}, {"edge": edges.0, "kind": "square"}]}),
            "already",
        ),
    ] {
        let message = refused(&mut s, "feature.plate", params.clone());
        assert!(message.contains(words), "{params}: {message}");
    }
    // Accepted, then failed with the reason.
    let reason = failed(
        &mut s,
        "feature.plate",
        json!({"faces": [face], "thickness": "0 mm"}),
    );
    assert!(reason.contains("thickness"), "{reason}");
    // A neighbour the edge does not run along: the wall's far edge.
    let reason = failed(
        &mut s,
        "feature.plate",
        json!({"faces": [face], "thickness": "t", "joints": [joint(&edges.1.replace("WallSheet", "FloorSheet"), "mitre", "WallSheet:startCap()")]}),
    );
    assert!(reason.contains("joints.0"), "{reason}");
    // A bevel past square.
    let reason = failed(
        &mut s,
        "feature.plate",
        json!({"faces": [face], "thickness": "t", "joints": [{"edge": edges.0, "kind": "bevel", "angle": "95 deg"}]}),
    );
    assert!(reason.contains("bevel"), "{reason}");
}
