//! TEMPORARY (sheetmetal-r3): prints renders for the developer as base64
//! in chunks; removed once read.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{built, new_document, run, session_in, workdir};
use serde_json::json;

fn print(name: &str, out: &serde_json::Value) {
    let b64 = out["base64"].as_str().unwrap();
    let chars: Vec<char> = b64.chars().collect();
    for (i, chunk) in chars.chunks(2000).enumerate() {
        println!("EV {name} {i} {}", chunk.iter().collect::<String>());
    }
}

#[test]
fn evidence() {
    let dir = workdir("evidence", "sheetmetal-r3");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "name": "Base"}),
    );
    let made = run(
        &mut s,
        "sketch.rect",
        json!({"sketch": "Base", "from": [0.0, 0.0], "to": [40.0, 30.0]}),
    );
    let l = made["lines"].as_array().unwrap();
    built(
        &mut s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "name": "U", "thickness": "1 mm",
        "radius": "1 mm", "kFactor": 0.5, "flanges": [
        {"edge": l[3], "length": "40 mm", "angle": "90 deg"},
        {"edge": l[1], "length": "40 mm", "angle": "90 deg"}]}),
    );
    let out = run(
        &mut s,
        "view.render",
        json!({"width": 640, "height": 480, "view": "front", "projection": "orthographic"}),
    );
    print("u-channel-front", &out);
    let seq = run(
        &mut s,
        "sheetMetal.bendSequence",
        json!({"feature": "U",
        "punch": [[-5.0, 0.0], [5.0, 0.0], [5.0, 100.0], [-5.0, 100.0]],
        "die": [[-15.0, -20.0], [15.0, -20.0], [15.0, 0.0], [-15.0, 0.0]]}),
    );
    println!("EVJSON straight {seq}");
    let seq = run(
        &mut s,
        "sheetMetal.bendSequence",
        json!({"feature": "U",
        "punch": [[-25.0, 0.0], [5.0, 0.0], [5.0, 30.0], [-5.0, 30.0], [-5.0, 150.0], [-25.0, 150.0]],
        "die": [[-15.0, -20.0], [15.0, -20.0], [15.0, 0.0], [-15.0, 0.0]]}),
    );
    println!("EVJSON gooseneck {seq}");
    // The reflex corner's way out: an L-shaped base, both flanges inset at
    // the inner corner.
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "sketch.create", json!({"plane": "xy", "name": "L"}));
    let pts = [
        [0.0, 0.0],
        [40.0, 0.0],
        [40.0, 10.0],
        [10.0, 10.0],
        [10.0, 30.0],
        [0.0, 30.0],
    ];
    let mut ids = Vec::new();
    for i in 0..6 {
        let a = pts[i];
        let b = pts[(i + 1) % 6];
        let made = run(
            &mut s,
            "sketch.line",
            json!({"sketch": "L", "from": a, "to": b}),
        );
        ids.push(made["entity"].clone());
    }
    let refused = run(
        &mut s,
        "feature.sheetMetal",
        json!({"sketch": "L", "name": "Lr", "thickness": "1 mm",
        "radius": "1 mm", "kFactor": 0.5, "flanges": [
        {"edge": ids[2], "length": "5 mm", "angle": "90 deg"},
        {"edge": ids[3], "length": "5 mm", "angle": "90 deg"}]}),
    );
    println!("EVJSON reflex {refused}");
    run(&mut s, "edit.undo", json!({}));
    built(
        &mut s,
        "feature.sheetMetal",
        json!({"sketch": "L", "name": "Lok", "thickness": "1 mm",
        "radius": "1 mm", "kFactor": 0.5, "flanges": [
        {"edge": ids[2], "length": "5 mm", "angle": "90 deg", "insetEnd": "10 mm"},
        {"edge": ids[3], "length": "5 mm", "angle": "90 deg", "insetStart": "3 mm"}]}),
    );
    let out = run(&mut s, "view.render", json!({"width": 640, "height": 480}));
    print("l-inset-iso", &out);
}
