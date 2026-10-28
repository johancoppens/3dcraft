//! A sketch on a face with its x axis turned to a world direction
//! (`sketch.create {face, xAxis}`): the bottom of a block has x along
//! world Y of its own; with `xAxis: [1, 0, 0]` it has x along world X, the
//! normal kept and the frame right-handed; saved and read back; a sketch
//! without it keeps the face's own axes, as every file before it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::run;
use common::{error, session_in, workdir};
use serde_json::{Value, json};

fn vec3(v: &Value) -> [f64; 3] {
    let a = v.as_array().unwrap();
    [0, 1, 2].map(|i| a[i].as_f64().unwrap())
}

fn near(a: [f64; 3], b: [f64; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-12)
}

/// The face of block `F1` whose outward normal is `normal`.
fn face_with_normal(s: &mut craft3d_engine::Session, normal: [f64; 3]) -> String {
    for i in 0..6 {
        let face = format!("F1:face({i})");
        let made = run(s, "sketch.create", json!({"face": face, "name": "Probe"}));
        let n = vec3(&made["normal"]);
        run(s, "feature.delete", json!({"feature": "Probe"}));
        if near(n, normal) {
            return face;
        }
    }
    panic!("no face with normal {normal:?}");
}

#[test]
fn a_sketch_on_the_bottom_can_have_x_along_world_x() {
    let dir = workdir("sketch-x-axis", "save");
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [10, 20, 30], "name": "B"}),
    );
    let bottom = face_with_normal(&mut s, [0.0, 0.0, -1.0]);
    let own = run(
        &mut s,
        "sketch.create",
        json!({"face": bottom, "name": "Own"}),
    );
    let turned = run(
        &mut s,
        "sketch.create",
        json!({"face": bottom, "xAxis": [1, 0, 0], "name": "Turned"}),
    );
    assert_eq!(turned["status"], "ok", "{turned}");
    let (x, y, n) = (
        vec3(&turned["frame"]["x"]),
        vec3(&turned["frame"]["y"]),
        vec3(&turned["normal"]),
    );
    assert!(near(x, [1.0, 0.0, 0.0]), "{turned}");
    assert!(near(y, [0.0, -1.0, 0.0]), "{turned}");
    assert!(near(n, [0.0, 0.0, -1.0]), "{turned}");
    // The origin and the normal are the face's own.
    assert_eq!(own["frame"]["origin"], turned["frame"]["origin"]);
    assert_eq!(own["normal"], turned["normal"]);
    // A direction not in the face's plane is projected onto it.
    let tilted = run(
        &mut s,
        "sketch.create",
        json!({"face": bottom, "xAxis": [2, 0, 5], "name": "Tilted"}),
    );
    assert!(
        near(vec3(&tilted["frame"]["x"]), [1.0, 0.0, 0.0]),
        "{tilted}"
    );
    // Saved and read back.
    run(&mut s, "file.save", json!({"path": "x.3dcraft"}));
    let mut t = session_in(&dir);
    run(&mut t, "file.open", json!({"path": "x.3dcraft"}));
    let back = run(&mut t, "sketch.inspect", json!({"sketch": "Turned"}));
    assert_eq!(back["frame"], turned["frame"], "{back}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_bad_x_axis_is_refused_or_fails_the_sketch() {
    let mut s = common::session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [10, 20, 30], "name": "B"}),
    );
    let bottom = face_with_normal(&mut s, [0.0, 0.0, -1.0]);
    for bad in [
        json!([1, 0]),
        json!("x"),
        json!([1, 0, null]),
        json!([1, 0, ""]),
    ] {
        error(
            &mut s,
            "sketch.create",
            json!({"face": bottom, "xAxis": bad}),
        );
    }
    error(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "xAxis": [1, 0, 0]}),
    );
    // Along the normal: the sketch is kept and fails, saying why.
    let along = run(
        &mut s,
        "sketch.create",
        json!({"face": bottom, "xAxis": [0, 0, 3], "name": "Along"}),
    );
    assert_eq!(along["status"], "failed", "{along}");
    assert!(along.to_string().contains("normal"), "{along}");
}
