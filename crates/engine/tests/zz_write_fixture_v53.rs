//! TEMPORARY: writes the version-53 (TEMPORARY number) fixture as
//! `craft3d-cli run --new '{"units": "mm"}' --script v53.steps.json --out
//! v53.3dcraftasm` does, and prints it (hex) and what document.inspect says
//! of it opened again; and the PLACEHOLDER fixtures of versions 51 and 52
//! (the v50 fixture with its version raised, until the branches that take
//! those numbers bring theirs). Removed once the fixtures are in.

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

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn write_fixture_v53() {
    let dir = workdir("fixture", "v53");
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    let versions = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../format/tests/fixtures/versions");
    let script = std::fs::read_to_string(versions.join("v53.steps.json")).unwrap();
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
        json!({"path": "v53.3dcraftasm", "overwrite": true}),
    );
    let bytes = std::fs::read(dir.join("v53.3dcraftasm")).unwrap();
    println!("FIXTURE-HEX v53 {}", hex(&bytes));
    // Opened again where the part's file is not: it rebuilds from what is
    // packed.
    std::fs::remove_file(dir.join("bar.3dcraft")).unwrap();
    let mut s = session_in(&dir);
    run(&mut s, "file.open", json!({"path": "v53.3dcraftasm"}));
    let inspect = run(&mut s, "document.inspect", json!({}));
    let out = json!({"features": inspect["features"], "bodies": inspect["bodies"], "rollback": inspect["rollback"]});
    println!("FIXTURE-INSPECT v53 {out}");
    let asm = run(&mut s, "assembly.inspect", json!({"measures": true}));
    println!("FIXTURE-ASSEMBLY v53 {asm}");
    // The placeholders.
    let v50 = std::fs::read(versions.join("v50.3dcraft")).unwrap();
    let entries = craft3d_zipstore::read_stored(&v50).unwrap();
    for version in [51u32, 52] {
        let mut owned: Vec<(String, Vec<u8>)> = Vec::new();
        for e in &entries {
            let data = if e.name == "manifest.json" {
                let text = std::str::from_utf8(e.data).unwrap();
                assert!(text.contains("\"format_version\": 50"));
                text.replacen(
                    "\"format_version\": 50",
                    &format!("\"format_version\": {version}"),
                    1,
                )
                .into_bytes()
            } else {
                e.data.to_vec()
            };
            owned.push((e.name.to_string(), data));
        }
        let list: Vec<(&str, &[u8])> = owned
            .iter()
            .map(|(n, d)| (n.as_str(), d.as_slice()))
            .collect();
        let bytes = craft3d_zipstore::write_stored(&list).unwrap();
        println!("FIXTURE-HEX v{version} {}", hex(&bytes));
    }
}
