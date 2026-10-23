//! `file.export {quality, angle}` (2026-10-23, the developer found the
//! meshes coarse): an angle criterion beside the automatic chordal
//! tolerance, so a small round stays round on a big part. Oracles: a full
//! circle meshed with no chord turning more than `a` degrees has at least
//! 360 / `a` chords; finer qualities never have fewer triangles; a
//! tolerance given alone makes the mesh it always made.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::run;
use common::{error, session_in, workdir};
use craft3d_engine::Session;
use serde_json::{Value, json};

/// A big plate with a pin of radius 10 far from it: the automatic
/// tolerance (about 1.3) follows the plate and leaves the pin a polygon
/// of seven sides; an angle criterion holds on a radius of more than
/// `tess::ANGLE_FLOOR / angle` tolerances (5.7 at 10 degrees).
fn parts(dir: &std::path::Path) -> Session {
    let mut s = session_in(dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": [1000, 800, 10]}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Pin", "radius": 10, "height": 20, "position": [1200, 0, 0]}),
    );
    s
}

fn export(s: &mut Session, params: Value) -> Value {
    run(s, "file.export", params)
}

/// The vertices of the STL at `path` (binary), on the pin's top cap
/// (z = 20, x near 1200).
fn rim_points(path: &std::path::Path) -> usize {
    let bytes = std::fs::read(path).unwrap();
    let count = u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize;
    let mut points = std::collections::BTreeSet::new();
    for t in 0..count {
        let at = 84 + t * 50 + 12;
        for corner in 0..3 {
            let p = at + corner * 12;
            let f =
                |k: usize| f32::from_le_bytes(bytes[p + k * 4..p + k * 4 + 4].try_into().unwrap());
            let (x, y, z) = (f(0), f(1), f(2));
            if (z - 20.0).abs() < 1e-3 && (x - 1200.0).abs() < 10.5 && (x - 1200.0).hypot(y) > 5.0 {
                points.insert((x.to_bits(), y.to_bits()));
            }
        }
    }
    points.len()
}

#[test]
fn qualities_keep_a_small_pin_round_on_a_big_part() {
    let dir = workdir("export-quality", "pin");
    let mut s = parts(&dir);
    let normal = export(&mut s, json!({"path": "normal.stl"}));
    assert_eq!(normal["quality"], "normal");
    assert_eq!(normal["angle"], 10.0);
    let auto = normal["tolerance"].as_f64().unwrap();
    // The mesh made before 2026-10-23: the same tolerance, no angle.
    let before = export(&mut s, json!({"path": "before.stl", "tolerance": auto}));
    assert_eq!(before["angle"], Value::Null);
    let coarse = export(&mut s, json!({"path": "coarse.stl", "quality": "coarse"}));
    let fine = export(&mut s, json!({"path": "fine.stl", "quality": "fine"}));
    assert_eq!(coarse["angle"], 20.0);
    assert_eq!(fine["angle"], 5.0);
    assert!((coarse["tolerance"].as_f64().unwrap() - 2.0 * auto).abs() < 1e-12 * auto);
    assert!((fine["tolerance"].as_f64().unwrap() - 0.25 * auto).abs() < 1e-12 * auto);
    let triangles = |v: &Value| v["triangles"].as_u64().unwrap();
    assert!(triangles(&before) < triangles(&normal), "{before} {normal}");
    assert!(triangles(&coarse) <= triangles(&normal));
    assert!(triangles(&normal) < triangles(&fine));
    // The pin's rim: before, a few chords (the plate's tolerance is far
    // coarser than the pin); with 10 degrees at least 36.
    let rim = |name: &str| rim_points(&dir.join(name));
    assert!(rim("before.stl") < 36, "{}", rim("before.stl"));
    assert!(rim("normal.stl") >= 36, "{}", rim("normal.stl"));
    assert!(rim("coarse.stl") >= 18, "{}", rim("coarse.stl"));
    assert!(rim("fine.stl") >= 72, "{}", rim("fine.stl"));
    // An angle of its own; 0 is none, as a tolerance alone.
    let own = export(&mut s, json!({"path": "own.stl", "angle": 30}));
    assert_eq!(own["angle"], 30.0);
    assert!(rim("own.stl") >= 12);
    let none = export(
        &mut s,
        json!({"path": "none.stl", "tolerance": auto, "angle": 0}),
    );
    assert_eq!(none["angle"], Value::Null);
    assert_eq!(triangles(&none), triangles(&before));
    // A tolerance with a quality takes the quality's angle.
    let both = export(
        &mut s,
        json!({"path": "both.stl", "tolerance": auto, "quality": "normal"}),
    );
    assert_eq!(triangles(&both), triangles(&normal));
    // The other mesh formats follow the same rule.
    for name in ["normal.3mf", "normal.obj"] {
        let other = export(&mut s, json!({"path": name}));
        assert_eq!(triangles(&other), triangles(&normal), "{name}");
    }
}

#[test]
fn bad_qualities_and_angles_are_refused() {
    let dir = workdir("export-quality", "refused");
    let mut s = parts(&dir);
    for (params, says) in [
        (json!({"path": "a.stl", "quality": "ultra"}), "ultra"),
        (json!({"path": "a.stl", "quality": 3}), "quality"),
        (json!({"path": "a.stl", "angle": -1}), "angle"),
        (json!({"path": "a.stl", "angle": 91}), "angle"),
        (json!({"path": "a.stl", "angle": "ten"}), "angle"),
        (json!({"path": "a.stl", "tolerance": 0}), "positive"),
        (json!({"path": "a.step", "quality": "fine"}), "mesh"),
        (json!({"path": "a.step", "angle": 5}), "mesh"),
        (
            json!({"path": "a.dxf", "quality": "fine", "face": "F1:face(0)"}),
            "mesh",
        ),
    ] {
        let message = error(&mut s, "file.export", params.clone());
        assert!(message.contains(says), "{params}: {message}");
        assert!(!dir.join("a.stl").exists(), "{params}");
    }
    // A null is no value, as for the other parameters.
    let step = export(
        &mut s,
        json!({"path": "a.step", "quality": null, "angle": null, "tolerance": null}),
    );
    assert!(step["bytes"].as_u64().unwrap() > 0);
}
