//! Designs the random-design fuzzer (`apps/craft3d-fuzz`) found wrong
//! (the ones it found hanging are `fuzz_findings.rs`; both keep their
//! reproducers in `fuzz_findings/`),
//! each replayed from its reproducer as the fuzzer runs it: per group of
//! commands, names in braces resolved to the ids the document gives them;
//! a group that is refused (a command says no, or a feature fails after
//! it) is taken back. Each design must end valid and
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
//! - `17070.json`, `18436.json`, `18647.json`, `23080.json`, `25810.json`
//!   (chamfers) and `28426.json` (an edit of a chamfer's size and of the
//!   shell under it), second run, 2026-10-18: a closed shell (walls of
//!   `t`) whose outer edge is chamfered by `2t`, so that the chamfer's
//!   face passes through the cavity's edge, accepted, which
//!   `document.check` found as shells that meet: zero thickness, refused
//!   (`kernel::boolean::parts_touch`, which regeneration holds every body
//!   to).
//! - `19604.json`: a ball that touches the rim of a cone's base from
//!   outside in a point, united and accepted as two shells that touch:
//!   zero thickness, refused (by the boolean, which looks where an edge
//!   meets the other body's surface; regeneration's `parts_touch` sees
//!   it too, the middle of the rim's edge being the point of contact).
//! - `21635.json`: a mirror of a body with a sphere face whose loop
//!   passes twice through one vertex, which could not be meshed for the
//!   self-check (the seam to the pole started from the pass that does
//!   not open towards it): accepted and valid.
//! - `20754684.json`, `20811107.json` and `20792240.json`, the run of
//!   2026-10-20, each a fault of the check and not of the kernel: a
//!   pattern whose instance has a skin of a cylinder wall thinner than
//!   the mesh's chord, meshed flat in the plane of the face beside it
//!   and taken for an overlap of the two; a groove whose ceiling is
//!   tangent to the top face along a line, where a touch was measured
//!   as a crossing as deep as the body by a walk over the top face's own
//!   triangles; and a mirror of united tori with a bite whose face could
//!   not be meshed, the refinement having run into a needle between a
//!   seam and a chord. All three accepted and valid.

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
        // As the fuzzer: a feature that fails because of the group (an
        // edit upstream of it) refuses the group too.
        if refused.is_none() {
            let inspect = session.execute("document.inspect", &json!({})).unwrap();
            refused = inspect["features"]
                .as_array()
                .unwrap()
                .iter()
                .find(|f| f["status"] == "failed")
                .map(|f| format!("{} failed: {}", f["name"], f["error"]));
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
        ("17070.json", &["zero thickness"][..]),
        ("18436.json", &["zero thickness"][..]),
        ("18647.json", &["zero thickness"][..]),
        ("19604.json", &["zero thickness"][..]),
        ("21635.json", &[][..]),
        ("23080.json", &["zero thickness"][..]),
        ("25810.json", &["zero thickness"][..]),
        ("28426.json", &["zero thickness"][..]),
        ("20754684.json", &[][..]),
        ("20792240.json", &[][..]),
        ("20811107.json", &[][..]),
    ] {
        let refused = replay(file);
        assert_eq!(refused.len(), reasons.len(), "{file}: {refused:?}");
        for (got, want) in refused.iter().zip(reasons) {
            assert!(got.contains(want), "{file}: {got}");
        }
    }
}
