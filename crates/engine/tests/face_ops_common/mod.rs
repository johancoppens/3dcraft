//! Shared by the tests of the face operations that move, tilt and thicken
//! faces: a session, commands that must succeed or fail, and the bodies
//! they start from.

// Test support: a failure here is a failed test.
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Access, Session, Workspace};
use serde_json::{Value, json};
use std::f64::consts::PI;
use std::path::PathBuf;

pub const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

pub fn workdir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("face-ops-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

pub fn session() -> Session {
    Session::new(Workspace {
        read: Access::None,
        write: Access::None,
    })
}

pub fn run(session: &mut Session, id: &str, params: Value) -> Value {
    let result = session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"));
    // Built up edit by edit, the document is what regenerating it from
    // nothing gives (docs/testing.md: incremental = full regeneration).
    assert_ne!(
        session.regeneration_matches_a_full_one(),
        Some(false),
        "after {id} {params}"
    );
    result
}

/// A command that adds or edits a feature, which has to regenerate.
pub fn built(session: &mut Session, id: &str, params: Value) -> Value {
    let result = run(session, id, params.clone());
    assert_eq!(result["status"], "ok", "{id} {params}: {result}");
    result
}

/// A command that is accepted but whose feature fails: the reason.
pub fn failed(session: &mut Session, id: &str, params: Value) -> String {
    let result = run(session, id, params.clone());
    assert_eq!(result["status"], "failed", "{id} {params}: {result}");
    result["error"].as_str().unwrap().to_string()
}

pub fn refused(session: &mut Session, id: &str, params: Value) -> String {
    match session.execute(id, &params) {
        Ok(value) => panic!("{id} {params} should be refused, returned {value}"),
        Err(e) => {
            assert!(!e.message().contains("internal error"), "{id}: {e}");
            e.to_string()
        }
    }
}

/// A new document in millimetres whose tolerance fits parts of size `k`.
pub fn new_document(session: &mut Session, k: f64) {
    run(
        session,
        "file.new",
        json!({"units": "mm", "tolerance": 1e-6 * k}),
    );
}

pub fn inspect(session: &mut Session) -> Value {
    run(session, "document.inspect", json!({}))
}

pub fn body(session: &mut Session, name: &str) -> Value {
    let document = inspect(session);
    document["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == name)
        .unwrap_or_else(|| panic!("no body {name} in {document}"))
        .clone()
}

pub fn body_names(session: &mut Session) -> Vec<String> {
    inspect(session)["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["name"].as_str().unwrap().to_string())
        .collect()
}

pub fn feature(session: &mut Session, name: &str) -> Value {
    let document = inspect(session);
    document["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == name)
        .unwrap_or_else(|| panic!("no feature {name} in {document}"))
        .clone()
}

#[track_caller]
pub fn assert_close(actual: &Value, expected: f64, what: &str) {
    let actual = actual
        .as_f64()
        .unwrap_or_else(|| panic!("{what}: {actual}"));
    assert!(
        (actual - expected).abs() <= 1e-9 * expected.abs(),
        "{what}: {actual}, expected {expected} (off by {:e})",
        (actual - expected) / expected
    );
}

/// The body is valid, also to the mesh-level check, and has this volume
/// and area (given for scale 1).
#[track_caller]
pub fn assert_solid(
    session: &mut Session,
    name: &str,
    volume: f64,
    area: Option<f64>,
    k: f64,
) -> Value {
    let found = body(session, name);
    assert_eq!(found["valid"], true, "{found}");
    assert_close(&found["volume"], volume * k * k * k, "volume");
    if let Some(area) = area {
        assert_close(&found["area"], area * k * k, "area");
    }
    let check = run(session, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
    assert_eq!(check["problems"], json!([]), "{check}");
    found
}

pub fn names_of(body: &Value) -> Vec<String> {
    body["faceNames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap().to_string())
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
