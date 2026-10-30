//! TEMPORARY: writes the format-55 fixture with this build's engine, as
//! `craft3d-cli run --new '{"units": "mm"}' --script v55.steps.json` does,
//! and prints the file (hex) and `document.inspect` of it opened again, to
//! be copied into `crates/format/tests/fixtures/versions/`. Removed once
//! the fixture is in.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use serde_json::{Value, json};

#[test]
fn write_the_v55_fixture() {
    let dir = common::workdir("fixture", "v55");
    // A restricted font for `font.add`: the document records it, keeps no
    // byte of it.
    let fonts = match std::env::var_os("CRAFT3D_FONTS") {
        Some(d) if !d.is_empty() => std::path::PathBuf::from(d),
        _ => std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts"),
    };
    let mut b = std::fs::read(fonts.join("JetBrainsMono-Regular.ttf")).unwrap();
    let n = u16::from_be_bytes([b[4], b[5]]) as usize;
    let os2 = (0..n)
        .find_map(|i| {
            let rec = 12 + 16 * i;
            (&b[rec..rec + 4] == b"OS/2").then(|| {
                u32::from_be_bytes([b[rec + 8], b[rec + 9], b[rec + 10], b[rec + 11]]) as usize
            })
        })
        .unwrap();
    b[os2 + 8..os2 + 10].copy_from_slice(&2u16.to_be_bytes());
    std::fs::write(dir.join("restricted.ttf"), &b).unwrap();
    let steps: Value = serde_json::from_str(
        &std::fs::read_to_string(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../format/tests/fixtures/versions/v55.steps.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let mut s = common::session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    for step in steps.as_array().unwrap() {
        let out = run(&mut s, step[0].as_str().unwrap(), step[1].clone());
        assert!(out.get("error").is_none(), "{step}: {out}");
    }
    run(&mut s, "file.save", json!({"path": "v55.3dcraft"}));
    let bytes = std::fs::read(dir.join("v55.3dcraft")).unwrap();
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    println!("FIXTURE-HEX {hex}");
    run(&mut s, "file.open", json!({"path": "v55.3dcraft"}));
    let inspected = run(&mut s, "document.inspect", json!({}));
    println!("FIXTURE-INSPECT {inspected}");
    assert!(bytes.len() < 60_000, "{} bytes", bytes.len());
}
