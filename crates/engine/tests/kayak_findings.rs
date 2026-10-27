//! What the sea-kayak Jig, built natively, found the platform refusing
//! (2026-10-27), each from the steps that made it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};
use std::path::PathBuf;

/// Runs the steps of `file`, printing every step that fails and the
/// status of every feature after.
fn diagnose(file: &str) {
    let text = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/kayak_findings")
            .join(file),
    )
    .unwrap();
    let steps: Vec<(String, Value)> = serde_json::from_str(&text).unwrap();
    let mut s = Session::new(Workspace::unrestricted());
    s.execute("file.new", &json!({"units": "mm"})).unwrap();
    for (id, params) in &steps {
        let t = std::time::Instant::now();
        match s.execute(id, params) {
            Ok(v) => {
                if id.starts_with("measure")
                    || id.starts_with("assert")
                    || id.starts_with("document.check")
                {
                    println!("{file}: {id} -> {v}");
                }
            }
            Err(e) => println!("{file}: ERR {id} {params}: {e}"),
        }
        let dt = t.elapsed().as_secs_f64();
        if dt > 2.0 {
            println!("{file}: {id} took {dt:.1} s");
        }
    }
    let document = s.execute("document.inspect", &json!({})).unwrap();
    for f in document["features"].as_array().unwrap() {
        if f["status"] != "ok" {
            println!("{file}: feature {} {}", f["name"], f);
        }
    }
}

#[test]
fn diagnose_1() {
    diagnose("sea_kayak_1.steps.json");
}
#[test]
fn diagnose_2() {
    diagnose("sea_kayak_2.steps.json");
}
#[test]
fn diagnose_5() {
    diagnose("sea_kayak_5.steps.json");
}
#[test]
fn diagnose_6() {
    diagnose("sea_kayak_6.steps.json");
}
#[test]
fn diagnose_7() {
    diagnose("sea_kayak_7.steps.json");
}
