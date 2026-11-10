//! The bytes of the STL exports of the examples, the showcase and the
//! benchmark's models, as digests: to compare before and after a change
//! to the tessellator that should leave every mesh it made alone. Not a
//! test of anything on its own account; run it on both commits, in
//! release, and compare the two runs with `diff`:
//!
//! `cargo test --release -p craft3d-engine --test mesh_digests -- --ignored --nocapture`
//!
//! One line per model and quality: `digest <model> <quality> <fnv-1a of
//! the file> <bytes>`, or `failed <model> <quality> <error>`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use craft3d_engine::Session;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// FNV-1a over the bytes.
fn fnv(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// The steps of a script: `[["id", {params}], ...]` or
/// `[{"command": "id", "params": {...}}, ...]`.
fn steps(path: &Path) -> Vec<(String, Value)> {
    let text = std::fs::read_to_string(path).unwrap();
    let value: Value = serde_json::from_str(&text).unwrap();
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|step| match step {
            Value::Array(pair) => (pair[0].as_str().unwrap().to_string(), pair[1].clone()),
            Value::Object(map) => (
                map["command"].as_str().unwrap().to_string(),
                map.get("params").cloned().unwrap_or(json!({})),
            ),
            other => panic!("{path:?}: a step {other}"),
        })
        .collect()
}

fn digests(s: &mut Session, dir: &Path, model: &str) {
    for quality in ["coarse", "normal", "fine"] {
        let file = format!("{model}-{quality}.stl");
        match s.execute("file.export", &json!({"path": file, "quality": quality})) {
            Ok(_) => {
                let bytes = std::fs::read(dir.join(&file)).unwrap();
                println!(
                    "digest {model} {quality} {:016x} {}",
                    fnv(&bytes),
                    bytes.len()
                );
                let _ = std::fs::remove_file(dir.join(&file));
            }
            Err(error) => println!("failed {model} {quality} {}", error.message()),
        }
    }
}

#[test]
#[ignore = "prints digests of STL exports; run in release with --ignored --nocapture"]
fn digests_of_the_exports_of_the_examples_showcase_and_benchmark_models() {
    let dir = common::workdir("mesh-digests", "all");
    let mut scripts: Vec<(String, PathBuf)> = Vec::new();
    for (folder, suffix) in [
        ("docs/examples", ".json"),
        ("crates/engine/tests/example_models", ".json"),
        ("crates/engine/tests/bench_models", ".steps.json"),
    ] {
        let mut found: Vec<PathBuf> = std::fs::read_dir(root().join(folder))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.to_string_lossy().ends_with(suffix))
            .collect();
        found.sort();
        for path in found {
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            scripts.push((name.trim_end_matches(suffix).to_string(), path));
        }
    }
    for (name, path) in scripts {
        let mut s = common::session_in(&dir);
        s.execute("file.new", &json!({"units": "mm"})).unwrap();
        for (id, params) in steps(&path) {
            if id.starts_with("file.") || id.starts_with("view.") || id.starts_with("ui.") {
                continue;
            }
            let _ = s.execute(&id, &params);
        }
        digests(&mut s, &dir, &name);
    }
    let mut showcase: Vec<PathBuf> = std::fs::read_dir(root().join("docs/showcase"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "3dcraft"))
        .collect();
    showcase.sort();
    for path in showcase {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let copy = dir.join(format!("{name}.3dcraft"));
        std::fs::copy(&path, &copy).unwrap();
        let mut s = common::session_in(&dir);
        if let Err(error) = s.execute("file.open", &json!({"path": format!("{name}.3dcraft")})) {
            println!("failed {name} open {}", error.message());
            continue;
        }
        digests(&mut s, &dir, &name);
    }
}
