//! Opt-in: FreeCAD itself, headless, agrees with `tests/data/expected.json`
//! (what `make_fixtures.py` recorded when it made the documents), so the
//! numbers the other tests hold this crate to are FreeCAD's.
//!
//! ```sh
//! CRAFT3D_FREECAD=/path/to/FreeCADCmd cargo test -p craft3d-fcstdread --test freecad -- --ignored
//! ```
//!
//! Part of `cargo xtask freecad`. When opted in, a missing or failing
//! FreeCAD is a test failure, never a skip. FreeCAD runs as a separate
//! program, with a user home of its own; nothing of it is linked.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::process::Command;

#[test]
#[ignore = "needs CRAFT3D_FREECAD"]
fn freecad_measures_the_documents_as_expected_json_says() {
    let freecad = std::env::var("CRAFT3D_FREECAD")
        .expect("set CRAFT3D_FREECAD to a FreeCAD AppImage or FreeCADCmd executable");
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("fcstd-freecad");
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let output = dir.join("verify.report");
    let _ = std::fs::remove_file(&output);
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/verify_fixtures.py");
    let mut command = Command::new(&freecad);
    if freecad.ends_with(".AppImage") {
        command.arg("freecadcmd");
    }
    let run = command
        .arg(&script)
        .env("CRAFT3D_FC_OUT", &output)
        .env("FREECAD_USER_HOME", &home)
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap_or_else(|e| panic!("cannot run {freecad}: {e}"));
    let text = std::fs::read_to_string(&output).unwrap_or_else(|_| {
        panic!(
            "FreeCAD wrote no report ({}):\n{}\n{}",
            run.status,
            String::from_utf8_lossy(&run.stdout),
            String::from_utf8_lossy(&run.stderr)
        )
    });
    let lines: Vec<&str> = text.lines().filter(|l| !l.is_empty()).collect();
    assert!(lines.len() >= 11, "{text}");
    assert!(lines.iter().all(|l| l.starts_with("ok ")), "{text}");
}
