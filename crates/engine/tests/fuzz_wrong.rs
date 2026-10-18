//! Designs the random-design fuzzer (`apps/craft3d-fuzz`) found wrong
//! (the ones it found hanging are `fuzz_findings.rs`; both keep their
//! reproducers in `fuzz_findings/`),
//! each replayed from its reproducer as the fuzzer runs it: per group of
//! commands, names in braces resolved to the ids the document gives them;
//! a group that is refused is taken back. Each design must end valid and
//! measurable with nothing touching itself (`document.check` deep finds
//! no problem), and the groups that are refused must be refused for the
//! reason given here.
//!
//! - `5279.json` (shrunk) and `5279-by-hand.json`: a union of two
//!   revolutions that overlap and also touch along a line, accepted, which
//!   `document.check` found touching itself: zero thickness, refused.
//! - `14211.json`: a chamfer on a sphere's small cap that went past the
//!   cap's top and folded through itself, accepted, which could not be
//!   measured: refused.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// `text` with every `{name}` that names a feature replaced by its id.
fn resolved(text: &str, ids: &BTreeMap<String, String>) -> String {
    let mut out = text.to_string();
    for (name, id) in ids {
        out = out.replace(&format!("{{{name}}}"), id);
    }
    out
}

fn mapped(value: &Value, ids: &BTreeMap<String, String>) -> Value {
    match value {
        Value::String(s) => Value::String(resolved(s, ids)),
        Value::Array(items) => Value::Array(items.iter().map(|v| mapped(v, ids)).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), mapped(v, ids)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn ids(session: &mut Session) -> BTreeMap<String, String> {
    let inspect = session.execute("document.inspect", &json!({})).unwrap();
    inspect["features"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| {
            Some((
                f["name"].as_str()?.to_string(),
                f["id"].as_str()?.to_string(),
            ))
        })
        .collect()
}

/// Replays `file`; the reasons of the groups refused, in order.
fn replay(file: &str) -> Vec<String> {
    let text = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fuzz_findings")
            .join(file),
    )
    .unwrap();
    let design: Value = serde_json::from_str(&text).unwrap();
    let scale = design["scale"].as_f64().unwrap();
    let mut session = Session::new(Workspace::unrestricted());
    session
        .execute(
            "file.new",
            &json!({"units": "mm", "tolerance": 1e-6 * scale}),
        )
        .unwrap();
    let mut refusals = Vec::new();
    for group in design["groups"].as_array().unwrap() {
        let before = session.clone();
        let names = ids(&mut session);
        let mut refused = None;
        for command in group["commands"].as_array().unwrap() {
            let id = command["command"].as_str().unwrap();
            let params = mapped(&command["params"], &names);
            match session.execute(id, &params) {
                Ok(result) if result["status"] == "failed" => {
                    refused = Some(result["error"].as_str().unwrap_or_default().to_string());
                }
                Ok(_) => {}
                Err(error) => refused = Some(error.message().to_string()),
            }
            if refused.is_some() {
                break;
            }
        }
        if let Some(reason) = refused {
            assert!(!reason.contains("internal"), "{file}: {reason}");
            refusals.push(reason);
            session = before;
        }
    }
    let check = session
        .execute("document.check", &json!({"deep": true}))
        .unwrap();
    assert_eq!(check["problems"], json!([]), "{file}: {check}");
    refusals
}

#[test]
fn the_fuzzers_findings_end_valid_or_refused_for_their_reason() {
    for (file, reasons) in [
        ("5279.json", &["zero thickness"][..]),
        ("5279-by-hand.json", &["zero thickness"][..]),
        ("14211.json", &["fold through itself"][..]),
    ] {
        let refused = replay(file);
        assert_eq!(refused.len(), reasons.len(), "{file}: {refused:?}");
        for (got, want) in refused.iter().zip(reasons) {
            assert!(got.contains(want), "{file}: {got}");
        }
    }
}
