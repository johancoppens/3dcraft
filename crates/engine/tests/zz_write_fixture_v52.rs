//! TEMPORARY: writes the version-52 fixture as `craft3d-cli run --new
//! '{"units": "mm"}' --script v52.steps.json --out v52.3dcraftasm` does,
//! and prints it (hex, in chunks) and what document.inspect and
//! assembly.inspect say of it opened again where the part's file is not.
//! Removed once the fixture is in.

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
        println!(
            "STEP {} -> {}",
            step[0],
            out.get("status").cloned().unwrap_or(Value::Null)
        );
    }
    run(
        &mut s,
        "file.save",
        json!({"path": "v52.3dcraftasm", "overwrite": true}),
    );
    let bytes = std::fs::read(dir.join("v52.3dcraftasm")).unwrap();
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    chunks("FIXTURE-HEX", &hex);
    // Opened again where the part's file is not: it rebuilds from what is
    // packed.
    std::fs::remove_file(dir.join("bar.3dcraft")).unwrap();
    let mut s = session_in(&dir);
    run(&mut s, "file.open", json!({"path": "v52.3dcraftasm"}));
    let inspect = run(&mut s, "document.inspect", json!({}));
    let out = json!({"features": inspect["features"], "bodies": inspect["bodies"], "rollback": inspect["rollback"]});
    println!("FIXTURE-INSPECT {out}");
    let asm = run(&mut s, "assembly.inspect", json!({"measures": true}));
    println!("FIXTURE-ASSEMBLY {asm}");
}
