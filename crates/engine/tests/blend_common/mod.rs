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

/// A block 20 × 30 × 10 (`Base`) with a pocket 3 deep in its top, 10
/// wide from x = 5, from y = 5 on, which ends in a ramp: the plane
/// x − y + z = 0, through the corner (5, 15, 10) of the pocket's rim, with
/// the pocket on the side its normal (1, −1, 1) points to (`Part`). The
/// rim along the wall x = 5 runs from (5, 5, 10), where it ends square
/// against the wall y = 5, to (5, 15, 10), where it ends in the ramp at a
/// slant: 10 long, with the wall below it 3 high. Returns the rim's two
/// faces, the top's first. The pocket is the part of x + z ≥ y ≥ 5 with
/// 5 ≤ x ≤ 15 and 7 ≤ z ≤ 10, 10 · 3 · (10 + 8.5 − 5) = 405, so the
/// part is 6000 − 405 = 5595 (`k` = 1).
pub fn ramp_pocket(session: &mut Session, k: f64) -> Value {
    built(
        session,
        "feature.block",
        json!({"name": "Base", "size": [20.0 * k, 30.0 * k, 10.0 * k]}),
    );
    built(
        session,
        "feature.block",
        json!({"name": "Pocket", "size": [10.0 * k, 25.0 * k, 5.0 * k], "position": [5.0 * k, 5.0 * k, 7.0 * k]}),
    );
    built(
        session,
        "feature.planeCut",
        json!({"name": "Ramp", "target": "Pocket", "planes": [
            {"point": [5.0 * k, 15.0 * k, 10.0 * k], "normal": [1, -1, 1], "side": "above"}
        ]}),
    );
    built(
        session,
        "feature.boolean",
        json!({"op": "subtract", "target": "Base", "tool": "Ramp", "name": "Part"}),
    );
    let rim = json!(["F4:copy(0)/F1:face(1)", "F4:copy(1)/F2:face(4)"]);
    let mut found = edge_between(session, "Part", k, [5.0, 5.0, 10.0], [5.0, 15.0, 10.0]);
    found.as_array_mut().unwrap().sort_by_key(|n| n.to_string());
    assert_eq!(found, rim, "the rim of the pocket");
    rim
}
