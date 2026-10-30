//! TEMPORARY: writes the version-58 fixture as `craft3d-cli run --new
//! '{"units": "mm"}' --script v58.steps.json --out v58.3dcraftasm` does,
//! and prints it (hex), what document.inspect says of it opened again with
//! its source files deleted, and how it solves. Removed once the fixture
//! is in.

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
fn write_fixture_v58() {
    let dir = workdir("fixture", "v58");
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    let script = std::fs::read_to_string(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../format/tests/fixtures/versions/v58.steps.json"),
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
        json!({"path": "v58.3dcraftasm", "overwrite": true}),
    );
    let bytes = std::fs::read(dir.join("v58.3dcraftasm")).unwrap();
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    println!("FIXTURE-HEX {hex}");
    std::fs::remove_file(dir.join("bar.3dcraft")).unwrap();
    std::fs::remove_file(dir.join("leg.3dcraftasm")).unwrap();
    run(&mut s, "file.open", json!({"path": "v58.3dcraftasm"}));
    let inspect = run(&mut s, "document.inspect", json!({}));
    let out = json!({"features": inspect["features"], "bodies": inspect["bodies"], "rollback": inspect["rollback"], "assertions": inspect["assertions"], "outputs": inspect["outputs"]});
    println!("FIXTURE-INSPECT {out}");
    let solved = run(&mut s, "assembly.solve", json!({}));
    println!(
        "FIXTURE-SOLVE ok {} dof {} free {}",
        solved["ok"], solved["dof"], solved["free"]
    );
}
