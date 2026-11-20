//! Temporary: writes the fixture of format version 78 with this commit's
//! `craft3d-cli`, as `crates/format/tests/versions.rs` says a fixture is
//! written, and prints the file (as hex) and what the same CLI says of it,
//! for a build server that hands back nothing but its log. Taken out
//! again once the fixture is in.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::PathBuf;
use std::process::{Command, Stdio};

#[test]
#[ignore = "run once by hand, to write the fixture of a new format version"]
fn write_the_fixture_of_format_version_78() {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/format/tests/fixtures/versions");
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("write-fixture-78");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(fixtures.join("v78.steps.json"), dir.join("v78.steps.json")).unwrap();
    let cli = |args: &[&str]| -> String {
        let output = Command::new(env!("CARGO_BIN_EXE_craft3d-cli"))
            .args(args)
            .current_dir(&dir)
            .stdin(Stdio::null())
            .output()
            .expect("the binary runs");
        let stdout = String::from_utf8(output.stdout).unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(output.status.success(), "{args:?}: {stdout}\n{stderr}");
        stdout
    };
    let built = cli(&[
        "run",
        "--new",
        r#"{"units": "mm"}"#,
        "--script",
        "v78.steps.json",
        "--out",
        "v78.3dcraft",
    ]);
    let inspected = cli(&["run", "v78.3dcraft", "--cmd", "document.inspect"]);
    let bytes = std::fs::read(dir.join("v78.3dcraft")).unwrap();
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    println!("FIXTURE-VERSION {}", cli(&["--version"]).trim());
    for line in built.lines() {
        println!("FIXTURE-BUILT {line}");
    }
    println!("FIXTURE-BYTES {}", bytes.len());
    for chunk in hex.as_bytes().chunks(4000) {
        println!("FIXTURE-HEX {}", std::str::from_utf8(chunk).unwrap());
    }
    for line in inspected.lines() {
        // In pieces: a log line has a length it is cut at.
        for chunk in line.as_bytes().chunks(4000) {
            println!("FIXTURE-INSPECT {}", String::from_utf8_lossy(chunk));
        }
        println!("FIXTURE-INSPECT-END");
    }
}
