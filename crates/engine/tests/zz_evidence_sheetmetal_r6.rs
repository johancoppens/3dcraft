//! TEMPORARY (sheetmetal-r6): prints renders for the developer as base64
//! in chunks; removed once read.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{new_document, run, session_in, workdir};
use serde_json::{Value, json};

fn print(name: &str, out: &Value) {
    let b64 = out["base64"].as_str().unwrap();
    let chars: Vec<char> = b64.chars().collect();
    for (i, chunk) in chars.chunks(2000).enumerate() {
        println!("EV {name} {i} {}", chunk.iter().collect::<String>());
    }
}

#[test]
fn evidence() {
    let dir = workdir("evidence", "sheetmetal-r6");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    let script = std::fs::read_to_string(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../format/tests/fixtures/versions/v52.steps.json"),
    )
    .unwrap();
    let steps: Value = serde_json::from_str(&script).unwrap();
    for step in steps.as_array().unwrap() {
        run(&mut s, step[0].as_str().unwrap(), step[1].clone());
    }
    for (name, params) in [
        (
            "tray-iso",
            json!({"width": 800, "height": 600, "view": "iso"}),
        ),
        (
            "tray-front",
            json!({"width": 800, "height": 600, "view": "front", "projection": "orthographic"}),
        ),
        (
            "tray-top",
            json!({"width": 800, "height": 600, "view": "top", "projection": "orthographic"}),
        ),
        (
            "tab-notch-zoom",
            json!({"width": 800, "height": 600, "view": "iso",
            "box": [[2.0, -4.0, 0.0], [22.0, 2.0, 14.0]]}),
        ),
    ] {
        let out = run(&mut s, "view.render", params);
        print(name, &out);
    }
    run(
        &mut s,
        "file.export",
        json!({"path": "tray-flat.svg", "sheetMetal": "Tray"}),
    );
    let svg = std::fs::read_to_string(dir.join("tray-flat.svg")).unwrap();
    for (i, line) in svg.lines().enumerate() {
        println!("SVG {i} {line}");
    }
    let table = run(&mut s, "sheetMetal.bends", json!({"feature": "Tray"}));
    println!("EVJSON bends {table}");
    let seq = run(
        &mut s,
        "sheetMetal.bendSequence",
        json!({"feature": "Tray",
        "punch": [[-5.0, 0.0], [5.0, 0.0], [5.0, 100.0], [-5.0, 100.0]],
        "die": [[-15.0, -20.0], [15.0, -20.0], [15.0, 0.0], [-15.0, 0.0]], "maxWork": 200000}),
    );
    println!("EVJSON sequence {seq}");
}
