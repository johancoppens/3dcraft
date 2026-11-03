//! TEMPORARY: renders the spout of the features chapter and prints the
//! picture (hex, in chunks), so it can be read off the Strix. Removed once
//! the picture is in docs/guide/images.

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

#[test]
fn render_the_spout() {
    let dir = workdir("render", "spout");
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    let text = std::fs::read_to_string(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/guide/09-features.md"),
    )
    .unwrap();
    let start = text
        .find("[\n [\"variable.set\", {\"name\": \"r0\"")
        .unwrap();
    let end = start + text[start..].find("\n```").unwrap();
    let steps: Value = serde_json::from_str(&text[start..end]).unwrap();
    for step in steps.as_array().unwrap() {
        let out = run(&mut s, step[0].as_str().unwrap(), step[1].clone());
        println!("STEP {} -> {}", step[0], out);
    }
    run(
        &mut s,
        "body.appearance",
        json!({"body": "Spout", "color": "#b87333", "metallic": 0.6, "roughness": 0.35}),
    );
    let out = run(
        &mut s,
        "view.render",
        json!({"path": "spout.png", "width": 720, "height": 480, "view": "iso", "renderer": "cpu", "look": {"mode": "studio"}}),
    );
    println!("RENDER {out}");
    let bytes = std::fs::read(dir.join("spout.png")).unwrap();
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let chars: Vec<char> = hex.chars().collect();
    for (i, chunk) in chars.chunks(4000).enumerate() {
        println!("PICTURE-HEX {i} {}", chunk.iter().collect::<String>());
    }
}
