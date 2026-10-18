//! Bodies of several lumps and with cavities through the commands (the
//! boolean checklist's item 5): a feature whose result falls apart or
//! leaves a void is one body of several shells (docs/decisions.md, open
//! decision on lumps), measured, checked deep (the shells nest as lumps
//! and cavities), exported to STEP as that many solids and voids, saved
//! and reopened to the same body under the same names, and the features
//! after it work on every lump: a fillet, an offset of faces that have a
//! piece in each lump, a shell. At three scales.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{assert_solid, body, built, new_document, run, session_in, workdir};
use craft3d_engine::Session;
use serde_json::{Value, json};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn block(s: &mut Session, name: &str, size: [f64; 3], at: [f64; 3], k: f64) {
    built(
        s,
        "feature.block",
        json!({"size": size.map(|x| x * k), "position": at.map(|x| x * k), "name": name}),
    );
}

fn boolean(s: &mut Session, op: &str, target: &str, tool: &str, name: &str) {
    built(
        s,
        "feature.boolean",
        json!({"op": op, "target": target, "tool": tool, "name": name}),
    );
}

/// A block of 10 with a slab through its middle cut out: two lumps.
fn split(s: &mut Session, k: f64) {
    block(s, "A", [10.0, 10.0, 10.0], [0.0; 3], k);
    block(s, "Slab", [12.0, 12.0, 2.0], [-1.0, -1.0, 4.0], k);
    boolean(s, "subtract", "A", "Slab", "Cut");
}

/// A block of 10 with a cavity of 4 in its middle.
fn hollow(s: &mut Session, k: f64) {
    block(s, "A", [10.0, 10.0, 10.0], [0.0; 3], k);
    block(s, "Inner", [4.0, 4.0, 4.0], [3.0, 3.0, 3.0], k);
    boolean(s, "subtract", "A", "Inner", "Hollow");
}

/// The STEP export of the document: `(solids, cavities)`.
fn step(s: &mut Session) -> (u64, u64) {
    let out = run(s, "file.export", json!({"path": "out.step"}));
    (
        out["solids"].as_u64().unwrap(),
        out["cavities"].as_u64().unwrap(),
    )
}

/// What the body is saved and reopened: the same counts, volume and
/// names.
fn round_trip(s: &mut Session, name: &str) {
    let before = body(s, name);
    run(
        s,
        "file.save",
        json!({"path": "lumps.3dcraft", "overwrite": true}),
    );
    run(s, "file.open", json!({"path": "lumps.3dcraft"}));
    let after = body(s, name);
    for key in [
        "faces",
        "edges",
        "vertices",
        "shells",
        "volume",
        "faceNames",
    ] {
        assert_eq!(after[key], before[key], "{name}: {key} after a round trip");
    }
}

fn names(body: &Value) -> Vec<String> {
    body["faceNames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn a_body_cut_in_two_is_one_body_of_two_lumps() {
    for k in SCALES {
        let dir = workdir("lumps", &format!("split-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        split(&mut s, k);
        let cut = assert_solid(&mut s, "Cut", 800.0, Some(720.0), k);
        assert_eq!(cut["shells"], 2, "{cut}");
        // The pieces of one face of A, one in each lump, share its name.
        let faces = names(&cut);
        assert_eq!(
            faces
                .iter()
                .filter(|n| *n == "F3:copy(0)/F1:face(2)")
                .count(),
            2,
            "{faces:?}"
        );
        assert_eq!(step(&mut s), (2, 0));
        round_trip(&mut s, "Cut");
        // A face with a piece in each lump moves in both; an edge on a
        // face only one lump has is rounded there.
        built(
            &mut s,
            "feature.offsetFace",
            json!({"faces": ["F3:copy(0)/F1:face(2)"], "distance": k, "name": "Wider"}),
        );
        assert_solid(&mut s, "Wider", 880.0, None, k);
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": [["F3:copy(0)/F1:face(1)", "F3:copy(0)/F1:face(3)"]],
                   "radius": k, "name": "Round"}),
        );
        let round = 880.0 - (1.0 - std::f64::consts::FRAC_PI_4) * 10.0;
        let rounded = assert_solid(&mut s, "Round", round, None, k);
        assert_eq!(rounded["shells"], 2);
        assert_eq!(step(&mut s), (2, 0));
        round_trip(&mut s, "Round");
    }
}

#[test]
fn a_body_with_a_cavity_and_a_lump_inside_it() {
    for k in SCALES {
        let dir = workdir("lumps", &format!("cavity-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        hollow(&mut s, k);
        let found = assert_solid(&mut s, "Hollow", 936.0, Some(696.0), k);
        assert_eq!(found["shells"], 2);
        assert_eq!(step(&mut s), (1, 1));
        round_trip(&mut s, "Hollow");
        // A block in the cavity, clear of its walls.
        block(&mut s, "Core", [2.0, 2.0, 2.0], [4.0, 4.0, 4.0], k);
        boolean(&mut s, "union", "Hollow", "Core", "Nest");
        let nest = assert_solid(&mut s, "Nest", 944.0, Some(720.0), k);
        assert_eq!(nest["shells"], 3);
        assert_eq!(step(&mut s), (2, 1));
        round_trip(&mut s, "Nest");
        // Walls behind every face: the outer lump hollow, the cavity's
        // walls a hollow box round it, in that hollow.
        built(
            &mut s,
            "feature.shell",
            json!({"source": "Nest", "thickness": 0.5 * k, "name": "Walls"}),
        );
        // 1000 − 9³ for the outside, 5³ − 4³ round the cavity, and the
        // core shelled to 2³ − 1³.
        let walls = assert_solid(&mut s, "Walls", 271.0 + 61.0 + 7.0, None, k);
        assert_eq!(walls["shells"], 6);
        assert_eq!(step(&mut s), (3, 3));
        round_trip(&mut s, "Walls");
    }
}

#[test]
fn faces_of_a_cavity_are_moved_and_deleted_by_name() {
    for k in SCALES {
        let mut s = common::checked::session();
        new_document(&mut s, k);
        hollow(&mut s, k);
        let cavity: Vec<String> = (0..6).map(|i| format!("F3:copy(1)/F2:face({i})")).collect();
        built(
            &mut s,
            "feature.offsetFace",
            json!({"faces": cavity, "distance": 0.5 * k, "name": "Smaller"}),
        );
        assert_solid(&mut s, "Smaller", 1000.0 - 27.0, None, k);
        // The cavity's faces by the names they had, followed to the
        // offset's body.
        let cavity: Vec<String> = (0..6).map(|i| format!("F3:copy(1)/F2:face({i})")).collect();
        built(
            &mut s,
            "feature.deleteFace",
            json!({"faces": cavity, "name": "Filled"}),
        );
        let filled = assert_solid(&mut s, "Filled", 1000.0, Some(600.0), k);
        assert_eq!(filled["shells"], 1);
    }
}
