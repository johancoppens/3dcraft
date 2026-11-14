//! Shells the kernel set's group H found refused, through the commands
//! (2026-11-14): each design as the set's script builds it, the shell
//! against the volume worked out by hand (the closed forms are derived in
//! `crates/kernel/tests/shell_limits.rs`, where they are also held at three
//! scales and turned), checked deep, followed through an edit of the
//! thickness, its new edges and vertices named by their faces, and saved
//! and opened again.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{assert_solid, body, built, deep, new_document, run, session_in};
use craft3d_engine::Session;
use serde_json::{Value, json};

/// 30 tan 1°: how far the drains' floors fall, as the set's scripts give it.
const DRAIN: f64 = 0.5236519478465276;

/// A floor falling `DRAIN` from the rim of `x0..x1` × `y0..y1` at height
/// `z` to its middle: two triangular prisms crossed, as the set builds it.
/// Features `first` to `first + 5`: a plane, a sketch and an extrusion
/// twice (the extrusions `first + 2` and `first + 5`).
fn drain(s: &mut Session, first: u32, (x0, x1, y0, y1, z): (f64, f64, f64, f64, f64)) {
    let (plane, sketch) = (format!("F{first}"), format!("F{}", first + 1));
    built(
        s,
        "datum.plane",
        json!({"through": [[0.0, y0, z], [1.0, y0, z], [0.0, y0, z - 1.0]]}),
    );
    built(s, "sketch.create", json!({"plane": plane}));
    built(
        s,
        "sketch.path",
        json!({"sketch": sketch, "start": [x0, 0.0],
               "segments": [{"line": [x1, 0.0]}, {"line": [(x0 + x1) / 2.0, DRAIN]},
                            {"line": "start"}]}),
    );
    built(
        s,
        "feature.extrude",
        json!({"sketch": sketch, "distance": y1 - y0}),
    );
    let (plane, sketch) = (format!("F{}", first + 3), format!("F{}", first + 4));
    built(
        s,
        "datum.plane",
        json!({"through": [[x0, 0.0, z], [x0, 1.0, z], [x0, 0.0, z + 1.0]]}),
    );
    built(s, "sketch.create", json!({"plane": plane}));
    built(
        s,
        "sketch.path",
        json!({"sketch": sketch, "start": [y0, 0.0],
               "segments": [{"line": [y1, 0.0]}, {"line": [(y0 + y1) / 2.0, -DRAIN]},
                            {"line": "start"}]}),
    );
    built(
        s,
        "feature.extrude",
        json!({"sketch": sketch, "distance": x1 - x0}),
    );
}

/// The vertices of `feature`'s body whose faces are all faces of the
/// hollow (named `<shell>:copy(1)/…`): the faces round each.
fn hollow_vertices(s: &mut Session, feature: &str, shell: &str) -> Vec<Vec<String>> {
    let edges = run(s, "document.edges", json!({"feature": feature}));
    let inner = format!("{shell}:copy(1)/");
    edges["vertices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            v["faces"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| f.as_str().unwrap().to_string())
                .collect::<Vec<String>>()
        })
        .filter(|faces| faces.iter().all(|f| f.starts_with(&inner)))
        .collect()
}

/// Every vertex of the hollow is where three faces meet: the vertices of
/// four faces the solid had there are split.
fn split_round(s: &mut Session, feature: &str, shell: &str) {
    let round = hollow_vertices(s, feature, shell);
    assert!(!round.is_empty());
    for faces in round {
        assert!(faces.len() <= 3, "{faces:?}");
    }
}

/// A session that may read and write in a directory of its own.
fn session(name: &str) -> (Session, std::path::PathBuf) {
    let dir = common::workdir("shell-limits", name);
    (session_in(&dir), dir)
}

/// Saved and opened again, the body is the same.
fn reopened(s: &mut Session, dir: &std::path::Path, name: &str) {
    let before = body(s, name);
    run(s, "file.save", json!({"path": "shell.3dcraft"}));
    let mut again = session_in(dir);
    run(&mut again, "file.open", json!({"path": "shell.3dcraft"}));
    let after = body(&mut again, name);
    assert_eq!(after["volume"], before["volume"], "{name}");
    assert_eq!(after["faceNames"], before["faceNames"], "{name}");
}

/// H-07: a trough whose floor falls 1° to a drain, a tab on one end,
/// shelled 2 open at its top: four bottom corners and the drain, vertices
/// of four convex faces, are split. Then 3.
#[test]
fn a_trough_whose_floor_falls_to_a_drain_is_shelled() {
    let (mut s, dir) = session("trough");
    new_document(&mut s, 1.0);
    built(
        &mut s,
        "feature.block",
        json!({"size": [100.0, 60.0, 40.0], "position": [0, 0, 0]}),
    );
    drain(&mut s, 2, (0.0, 100.0, 0.0, 60.0, 0.0));
    built(
        &mut s,
        "feature.boolean",
        json!({"op": "intersect", "target": "F4", "tool": "F7"}),
    );
    built(
        &mut s,
        "feature.boolean",
        json!({"op": "union", "target": "F1", "tool": "F8"}),
    );
    built(
        &mut s,
        "feature.block",
        json!({"size": [20.0, 20.0, 20.0], "position": [100.0, 20.0, 20.0]}),
    );
    built(
        &mut s,
        "feature.boolean",
        json!({"op": "union", "target": "F9", "tool": "F10"}),
    );
    built(
        &mut s,
        "feature.shell",
        json!({"source": "F11", "thickness": 2.0, "open": ["F11:copy(0)/F1:face(1)"],
               "name": "Trough"}),
    );
    assert_solid(&mut s, "Trough", 37962.27221527102, None, 1.0);
    split_round(&mut s, "Trough", "F12");
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "Trough", "thickness": 3.0}),
    );
    assert_solid(&mut s, "Trough", 55449.822041904146, None, 1.0);
    split_round(&mut s, "Trough", "F12");
    reopened(&mut s, &dir, "Trough");
}

/// H-06: a slab with a pocket whose floor falls 1° to a drain, shelled 3
/// open at its top: the pocket's four bottom corners and its drain,
/// vertices of four concave faces, are split, and the pocket's lining is a
/// lump of its own. Then 5.
#[test]
fn a_pocket_whose_floor_falls_to_a_drain_is_shelled() {
    let (mut s, dir) = session("pocket");
    new_document(&mut s, 1.0);
    built(
        &mut s,
        "feature.block",
        json!({"size": [140.0, 90.0, 50.0], "position": [0, 0, 0]}),
    );
    built(
        &mut s,
        "feature.block",
        json!({"size": [100.0, 60.0, 40.0], "position": [20.0, 15.0, 20.0]}),
    );
    drain(&mut s, 3, (20.0, 120.0, 15.0, 75.0, 20.0));
    built(
        &mut s,
        "feature.boolean",
        json!({"op": "intersect", "target": "F5", "tool": "F8"}),
    );
    built(
        &mut s,
        "feature.boolean",
        json!({"op": "union", "target": "F2", "tool": "F9"}),
    );
    built(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "F1", "tool": "F10"}),
    );
    built(
        &mut s,
        "feature.shell",
        json!({"source": "F11", "thickness": 3.0, "open": ["F11:copy(0)/F1:face(1)"],
               "name": "Pocket"}),
    );
    let made = assert_solid(&mut s, "Pocket", 151815.7900294402, None, 1.0);
    assert_eq!(made["shells"], 2, "{made}");
    split_round(&mut s, "Pocket", "F12");
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "Pocket", "thickness": 5.0}),
    );
    assert_solid(&mut s, "Pocket", 251439.85909941635, None, 1.0);
    reopened(&mut s, &dir, "Pocket");
}

/// H-01: a box and a cylinder whose axis lies in the box's front plane,
/// shelled 1 open at the box's bottom: where the cylinder's lowest and
/// highest lines meet the box's front edge, four faces of both kinds meet;
/// each such vertex becomes two, joined by a line where the front face's
/// offset meets the cylinder's. Then 2.
#[test]
fn a_box_and_a_cylinder_on_its_front_plane_are_shelled() {
    let (mut s, dir) = session("box-and-cylinder");
    new_document(&mut s, 1.0);
    built(
        &mut s,
        "feature.block",
        json!({"size": [47.5, 45.0, 30.0], "position": [0, 0, 0]}),
    );
    built(
        &mut s,
        "feature.cylinder",
        json!({"radius": 7.5, "height": 30.0, "position": [47.5, 0.0, 10.0], "axis": [1.0, 0.0, 0.0]}),
    );
    built(
        &mut s,
        "feature.boolean",
        json!({"op": "union", "target": "F1", "tool": "F2"}),
    );
    built(
        &mut s,
        "feature.shell",
        json!({"source": "F3", "thickness": 1.0, "open": ["F3:copy(0)/F1:face(0)"],
               "name": "Hollow"}),
    );
    assert_solid(&mut s, "Hollow", 8864.598272772455, None, 1.0);
    split_round(&mut s, "Hollow", "F4");
    // The new edge: between the front face's offset and the cylinder's.
    let between = run(
        &mut s,
        "document.edges",
        json!({"feature": "Hollow", "between": ["F4:copy(1)/F3:copy(0)/F1:face(2)",
                                                "F4:copy(1)/F3:copy(1)/F2:face(0)"]}),
    );
    assert_eq!(between["count"], 2, "{between}");
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "Hollow", "thickness": 2.0}),
    );
    assert_solid(&mut s, "Hollow", 16913.528821015258, None, 1.0);
    reopened(&mut s, &dir, "Hollow");
}

/// H-02: a box whose bottom edges are rounded 2.4 (mitred at the corners),
/// shelled 2 open at its bottom: the mitres where the rounds meet are
/// followed to the end. The volume against the closed form within the
/// tolerance over the area (the mitre is a fitted curve).
#[test]
fn a_box_rounded_along_its_bottom_is_shelled_beside_its_rounds() {
    let (mut s, dir) = session("mitred-rounds");
    new_document(&mut s, 1.0);
    built(
        &mut s,
        "feature.block",
        json!({"size": [40.0, 30.0, 10.0], "position": [0, 0, 0]}),
    );
    built(
        &mut s,
        "feature.fillet",
        json!({"edges": [["F1:face(0)", "F1:face(2)"], ["F1:face(0)", "F1:face(5)"],
                         ["F1:face(0)", "F1:face(3)"], ["F1:face(0)", "F1:face(4)"]],
               "radius": 2.4}),
    );
    built(
        &mut s,
        "feature.shell",
        json!({"source": "F2", "thickness": 2.0, "open": ["F2:copy(0)/F1:face(0)"],
               "name": "Lid"}),
    );
    let found: Value = body(&mut s, "Lid");
    let (volume, area) = (
        found["volume"].as_f64().unwrap(),
        found["area"].as_f64().unwrap(),
    );
    let want = 4446.399482908439;
    assert!(
        (volume - want).abs() <= 1e-6 * area,
        "{volume} against {want}"
    );
    deep(&mut s);
    reopened(&mut s, &dir, "Lid");
}
