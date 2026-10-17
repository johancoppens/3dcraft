//! Shared by the tests of the face operations that move, tilt and thicken
//! faces: a session, commands that must succeed or fail, and the bodies
//! they start from.

// Test support: a failure here is a failed test.
#![allow(
    dead_code,
    unused_imports,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::Session;
use serde_json::json;
use std::f64::consts::PI;
use std::path::PathBuf;

pub use crate::common::checked::{
    assert_close, assert_solid, body, built, failed, feature, inspect, names_of, new_document,
    refused, run, session,
};

pub const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

pub fn workdir(name: &str) -> PathBuf {
    crate::common::workdir("face-ops", name)
}

pub fn body_names(session: &mut Session) -> Vec<String> {
    inspect(session)["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["name"].as_str().unwrap().to_string())
        .collect()
}

pub fn block_area(x: f64, y: f64, z: f64) -> f64 {
    2.0 * (x * y + y * z + x * z)
}

// Faces of a block: 0 bottom, 1 top, 2 front (-y), 3 back (+y), 4 left
// (-x), 5 right (+x). Of a cylinder: 0 the wall, 1 the bottom, 2 the top.
pub const TOP: &str = "F1:face(1)";

/// `Base`, a block of 10 x 10 x 5 (F1).
pub fn base(session: &mut Session, k: f64) {
    built(
        session,
        "feature.block",
        json!({"name": "Base", "size": [10.0 * k, 10.0 * k, 5.0 * k]}),
    );
}

/// `Base` with a hole of radius 2 through it: `Drilled` (F3).
pub fn drilled(session: &mut Session, k: f64) {
    base(session, k);
    built(
        session,
        "feature.cylinder",
        json!({"name": "Drill", "position": [5.0 * k, 5.0 * k, -k],
               "radius": 2.0 * k, "height": 7.0 * k}),
    );
    built(
        session,
        "feature.boolean",
        json!({"op": "subtract", "target": "Base", "tool": "Drill", "name": "Drilled"}),
    );
}

/// `Base` with a boss of radius 2 and height 3 on it: `Bossed` (F3).
pub fn bossed(session: &mut Session, k: f64) {
    base(session, k);
    built(
        session,
        "feature.cylinder",
        json!({"name": "Stud", "position": [5.0 * k, 5.0 * k, 5.0 * k],
               "radius": 2.0 * k, "height": 3.0 * k}),
    );
    built(
        session,
        "feature.boolean",
        json!({"op": "union", "target": "Base", "tool": "Stud", "name": "Bossed"}),
    );
}

/// The wall of the hole of `Drilled`, of the boss of `Bossed`.
pub const WALL: &str = "F3:copy(1)/F2:face(0)";
/// The top of the boss of `Bossed`.
pub const BOSS_TOP: &str = "F3:copy(1)/F2:face(2)";
/// The top of the block of `Drilled` and `Bossed`.
pub const TOP_OF_3: &str = "F3:copy(0)/F1:face(1)";

pub fn drilled_mass(radius: f64, height: f64) -> (f64, f64) {
    let section = 100.0 - PI * radius * radius;
    (
        section * height,
        2.0 * section + 40.0 * height + 2.0 * PI * radius * height,
    )
}
