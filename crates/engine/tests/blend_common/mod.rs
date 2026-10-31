//! Shared by the tests of the blends (`fillet.rs`, `chamfer.rs`): the
//! bodies they start from and how they find an edge.

// Test support: a failure here is a failed test.
#![allow(
    dead_code,
    unused_imports,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use crate::common::checked::{built, run};
use craft3d_engine::Session;
use craft3d_math::Portable;
use serde_json::{Value, json};

pub const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

// Faces of a block: 0 bottom, 1 top, 2 front (-y), 3 back (+y), 4 left
// (-x), 5 right (+x).
pub const TOP: &str = "F1:face(1)";
pub const FRONT: &str = "F1:face(2)";
pub const LEFT: &str = "F1:face(4)";
pub const RIGHT: &str = "F1:face(5)";
pub const BOTTOM: &str = "F1:face(0)";
pub const BACK: &str = "F1:face(3)";

pub fn point(value: &Value) -> [f64; 3] {
    let list = value.as_array().unwrap();
    [
        list[0].as_f64().unwrap(),
        list[1].as_f64().unwrap(),
        list[2].as_f64().unwrap(),
    ]
}

/// The names of the two faces of the edge of `feature` that `document.edges`
/// lists with its ends nearest `a` and `b` (given for scale 1): what an
/// agent reads to refer to an edge.
pub fn edge_between(
    session: &mut Session,
    feature: &str,
    k: f64,
    a: [f64; 3],
    b: [f64; 3],
) -> Value {
    let listed = run(session, "document.edges", json!({"feature": feature}));
    let far = |p: [f64; 3], q: [f64; 3]| {
        ((p[0] - q[0] * k).ppowi(2) + (p[1] - q[1] * k).ppowi(2) + (p[2] - q[2] * k).ppowi(2))
            .sqrt()
    };
    let (edge, off) = listed["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|edge| {
            let (start, end) = (point(&edge["start"]), point(&edge["end"]));
            let one = far(start, a) + far(end, b);
            let other = far(start, b) + far(end, a);
            (edge, one.min(other))
        })
        .min_by(|x, y| x.1.total_cmp(&y.1))
        .unwrap();
    assert!(off < 1e-6 * k, "no edge of {feature} from {a:?} to {b:?}");
    edge["faces"].clone()
}

pub fn block(session: &mut Session, k: f64) {
    built(
        session,
        "feature.block",
        json!({"name": "Base", "size": [4.0 * k, 3.0 * k, 2.0 * k]}),
    );
}

/// A plate with a hole right through it, a blind hole and a boss.
pub fn drilled_plate(session: &mut Session, k: f64) {
    built(
        session,
        "feature.block",
        json!({"name": "Plate", "size": [8.0 * k, 3.0 * k, 2.0 * k]}),
    );
    let round = |session: &mut Session, name: &str, x: f64, z: f64, height: f64| {
        built(
            session,
            "feature.cylinder",
            json!({"name": name, "position": [x * k, 1.5 * k, z * k], "radius": 0.5 * k, "height": height * k}),
        );
    };
    round(session, "Drill", 1.5, -1.0, 4.0);
    built(
        session,
        "feature.boolean",
        json!({"op": "subtract", "target": "Plate", "tool": "Drill", "name": "Through"}),
    );
    round(session, "Bit", 4.0, 1.0, 2.0);
    built(
        session,
        "feature.boolean",
        json!({"op": "subtract", "target": "Through", "tool": "Bit", "name": "Blind"}),
    );
    round(session, "Stud", 6.5, 2.0, 1.0);
    built(
        session,
        "feature.boolean",
        json!({"op": "union", "target": "Blind", "tool": "Stud", "name": "Part"}),
    );
}

/// A slot: half circles of radius 1 about (0, 0) and (3, 0), 2 high.
pub fn slot(session: &mut Session, k: f64) {
    run(session, "sketch.create", json!({"name": "Outline"}));
    run(
        session,
        "sketch.path",
        json!({"sketch": "Outline", "start": [0.0, -k], "segments": [
            {"line": [3.0 * k, -k]},
            {"arc": [3.0 * k, k], "center": [3.0 * k, 0.0]},
            {"line": [0.0, k]},
            {"arc": "start", "center": [0.0, 0.0]},
        ]}),
    );
    built(
        session,
        "feature.extrude",
        json!({"sketch": "Outline", "distance": 2.0 * k, "name": "Slot"}),
    );
}

/// An L: the outline (0,0) (4,0) (4,1.5) (1.5,1.5) (1.5,3) (0,3), 2 high.
pub fn ell(session: &mut Session, k: f64) {
    run(session, "sketch.create", json!({"name": "Outline"}));
    let points: Vec<[f64; 2]> = [
        [0.0, 0.0],
        [4.0, 0.0],
        [4.0, 1.5],
        [1.5, 1.5],
        [1.5, 3.0],
        [0.0, 3.0],
    ]
    .iter()
    .map(|p: &[f64; 2]| [p[0] * k, p[1] * k])
    .collect();
    run(
        session,
        "sketch.polyline",
        json!({"sketch": "Outline", "closed": true, "points": points}),
    );
    built(
        session,
        "feature.extrude",
        json!({"sketch": "Outline", "distance": 2.0 * k, "name": "Ell"}),
    );
}
