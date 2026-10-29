//! TEMPORARY: writes the version-52 (TEMPORARY number) fixture as `craft3d-cli run --new
//! '{"units": "mm"}' --script v52.steps.json --out v52.3dcraft` does, prints it (hex,
//! in chunks) and what document.inspect says of it opened again, and prints evidence
//! renders (base64, in chunks). Removed once the fixture is in.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use common::{session_in, workdir};
use serde_json::{Value, json};

fn chunks(tag: &str, text: &str) {
    let chars: Vec<char> = text.chars().collect();
    for (i, chunk) in chars.chunks(2000).enumerate() {
        println!("{tag} {i} {}", chunk.iter().collect::<String>());
    }
}

#[test]
fn write_fixture_v52() {
    let dir = workdir("fixture", "v52");
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    let script = std::fs::read_to_string(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../format/tests/fixtures/versions/v52.steps.json"),
    )
    .unwrap();
    let steps: Value = serde_json::from_str(&script).unwrap();
    for step in steps.as_array().unwrap() {
        let out = run(&mut s, step[0].as_str().unwrap(), step[1].clone());
        println!("STEP {} -> {}", step[0], out);
    }
    run(
        &mut s,
        "file.save",
        json!({"path": "v52.3dcraft", "overwrite": true}),
    );
    let bytes = std::fs::read(dir.join("v52.3dcraft")).unwrap();
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    chunks("FIXTURE-HEX", &hex);
    run(&mut s, "file.open", json!({"path": "v52.3dcraft"}));
    let inspect = run(&mut s, "document.inspect", json!({}));
    let out = json!({"features": inspect["features"], "bodies": inspect["bodies"], "rollback": inspect["rollback"], "assertions": inspect["assertions"], "outputs": inspect["outputs"]});
    println!("FIXTURE-INSPECT {out}");
    for (name, params) in [
        (
            "rail-iso.png",
            json!({"width": 800, "height": 600, "view": "iso",
                   "box": [[98.0, -12.0, -2.0], [202.0, 32.0, 30.0]]}),
        ),
        (
            "lip-zoom.png",
            json!({"width": 800, "height": 600, "view": "iso",
                   "box": [[115.0, -2.0, 15.0], [135.0, 12.0, 30.0]]}),
        ),
        (
            "rail-base-notch.png",
            json!({"width": 800, "height": 600, "view": "top", "projection": "orthographic",
                   "box": [[104.0, -4.0, -1.0], [118.0, 6.0, 4.0]]}),
        ),
    ] {
        let r = run(&mut s, "view.render", params);
        chunks(&format!("EV {name}"), r["base64"].as_str().unwrap_or(""));
    }
    run(
        &mut s,
        "file.export",
        json!({"path": "rail-flat.svg", "sheetMetal": "Rail"}),
    );
    let svg = std::fs::read_to_string(dir.join("rail-flat.svg")).unwrap_or_default();
    chunks("EV rail-flat.svg", &svg.replace('\n', "\u{1}"));
    let table = run(&mut s, "sheetMetal.bends", json!({"feature": "Rail"}));
    println!("EVJSON bends {table}");
}
