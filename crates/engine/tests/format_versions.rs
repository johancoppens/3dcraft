//! The format promise, regenerated: the file of every format version
//! (`crates/format/tests/fixtures/versions`, written by that version's own
//! build) opens with today's code and regenerates to the bodies that build
//! reported, saving it writes the current version, and the saved file
//! reopens to the same bodies, bit for bit. How the fixtures were written
//! and what keeps them frozen: `crates/format/tests/versions.rs`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use common::{session_in, workdir};
use craft3d_format::FORMAT_VERSION;
use serde_json::{Value, json};
use std::path::PathBuf;

/// Volumes, areas and masses are integrals over the exact B-rep, by
/// quadrature: equal to what the writing build reported to this relative
/// difference. Measured on 2026-10-21: every one agrees to 1e-9 but the
/// area of version 13's fine modelled thread (F6), 1.03e-9 apart (356.32502417
/// against 356.32502380 mm²); counts of faces, edges and vertices agree
/// exactly for every body.
const RELATIVE: f64 = 1e-8;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../format/tests/fixtures/versions")
}

fn expected(version: u32) -> Value {
    let path = fixtures().join(format!("v{version}.expected.json"));
    serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap()
}

fn close(actual: &Value, expected: &Value) -> bool {
    match (actual.as_f64(), expected.as_f64()) {
        (Some(a), Some(e)) => (a - e).abs() <= RELATIVE * e.abs().max(1.0),
        _ => false,
    }
}

/// Every way `inspect` differs from what the writing build reported.
/// `entries` with each `face` written in the canonical form of its name.
fn canonical_faces(entries: &Value) -> Value {
    let mut entries = entries.clone();
    if let Some(list) = entries.as_array_mut() {
        for entry in list {
            let canonical = entry["face"]
                .as_str()
                .and_then(|text| text.parse::<craft3d_doc::PersistentName>().ok())
                .map(|name| name.to_string());
            if let Some(text) = canonical {
                entry["face"] = Value::String(text);
            }
        }
    }
    entries
}

fn differences(version: u32, inspect: &Value, want: &Value) -> Vec<String> {
    let mut found = Vec::new();
    let tag = format!("v{version}");
    if inspect["rollback"] != want["rollback"] {
        found.push(format!(
            "{tag}: rollback {} (was {})",
            inspect["rollback"], want["rollback"]
        ));
    }
    let features = inspect["features"].as_array().unwrap();
    let wanted = want["features"].as_array().unwrap();
    if features.len() != wanted.len() {
        found.push(format!(
            "{tag}: {} features (was {})",
            features.len(),
            wanted.len()
        ));
    }
    for (feature, wanted) in features.iter().zip(wanted) {
        for key in ["id", "name", "status", "dof"] {
            if wanted.get(key).is_some() && feature[key] != wanted[key] {
                found.push(format!(
                    "{tag} {}: {key} {} (was {}){}",
                    wanted["id"],
                    feature[key],
                    wanted[key],
                    feature
                        .get("error")
                        .map(|e| format!(": {e}"))
                        .unwrap_or_default()
                ));
            }
        }
    }
    // The colours given to faces (format 26), as listed, none of them lost.
    if let Some(faces) = want.get("faceAppearances") {
        // A name read in the long form and the short one is the same name
        // (`PersistentName` is canonical once read, 2026-10-22): compare
        // the names as names, the face and its colour exactly as pinned.
        if inspect.get("faceAppearances").map(canonical_faces) != Some(canonical_faces(faces)) {
            found.push(format!(
                "{tag}: faceAppearances {} (were {faces})",
                inspect["faceAppearances"]
            ));
        }
        if let Some(lost) = inspect.get("lostFaces") {
            found.push(format!("{tag}: lostFaces {lost}"));
        }
    }
    // The assertions (format 27), each as it stood: its name, measure,
    // status and what it must be; what was measured within the measure's
    // own rounding (it is said in words).
    if let Some(assertions) = want.get("assertions") {
        let keys = |list: &Value| -> Vec<Value> {
            list.as_array()
                .map(|l| {
                    l.iter()
                        .map(|a| json!([a["name"], a["measure"], a["status"], a["expected"]]))
                        .collect()
                })
                .unwrap_or_default()
        };
        let got = inspect.get("assertions").cloned().unwrap_or(Value::Null);
        if keys(&got) != keys(assertions) {
            found.push(format!("{tag}: assertions {got} (were {assertions})"));
        }
    }
    let bodies = inspect["bodies"].as_array().unwrap();
    let wanted = want["bodies"].as_array().unwrap();
    let ids =
        |list: &[Value]| -> Vec<Value> { list.iter().map(|b| b["feature"].clone()).collect() };
    if ids(bodies) != ids(wanted) {
        found.push(format!(
            "{tag}: bodies of {:?} (were {:?})",
            ids(bodies),
            ids(wanted)
        ));
        return found;
    }
    for (body, wanted) in bodies.iter().zip(wanted) {
        let Some(wanted) = wanted.as_object() else {
            continue;
        };
        for (key, value) in wanted {
            let same = match key.as_str() {
                "volume" | "area" | "mass" => close(&body[key], value),
                _ => &body[key] == value,
            };
            if !same {
                found.push(format!(
                    "{tag} body {}: {key} {} (was {value})",
                    wanted["feature"], body[key]
                ));
            }
        }
    }
    found
}

/// The bodies as `document.inspect` lists them, without what says where
/// the document is.
fn bodies(inspect: &Value) -> Value {
    json!({"features": inspect["features"], "bodies": inspect["bodies"], "rollback": inspect["rollback"]})
}

#[test]
fn the_file_of_every_format_version_regenerates_to_what_its_build_made() {
    let dir = workdir("format-versions", "all");
    let mut found = Vec::new();
    for version in 1..=FORMAT_VERSION {
        let want = expected(version);
        assert_eq!(want["format_version"], version, "v{version}.expected.json");
        let name = format!("v{version}.3dcraft");
        std::fs::copy(fixtures().join(&name), dir.join(&name)).unwrap();
        let mut s = session_in(&dir);
        run(&mut s, "file.open", json!({"path": name}));
        let opened = run(&mut s, "document.inspect", json!({}));
        found.extend(differences(version, &opened, &want));

        // Saved again: the current version, and the same bodies, exactly.
        let again = format!("v{version}-saved.3dcraft");
        run(&mut s, "file.save", json!({"path": again}));
        let text = String::from_utf8_lossy(&std::fs::read(dir.join(&again)).unwrap()).to_string();
        assert!(
            text.contains(&format!("\"format_version\": {FORMAT_VERSION}")),
            "v{version} saved"
        );
        let mut t = session_in(&dir);
        run(&mut t, "file.open", json!({"path": again}));
        let reopened = run(&mut t, "document.inspect", json!({}));
        assert_eq!(
            bodies(&reopened),
            bodies(&opened),
            "v{version}: saved as version {FORMAT_VERSION} and reopened"
        );
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
}
