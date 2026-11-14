//! The published example assemblies the program carries
//! (`docs/examples/published`), pinned: each instance's pose and placed
//! box, each mate's status and value (turns included), the solve's degrees
//! of freedom, error and note, each assertion's status, measure and proven
//! bound at rest, and over a motion of 73 samples each assertion's status,
//! least and greatest value and every sample's status. The golden file was
//! recorded from release/0.1 at 1090fe93 (before the assembly-trust fixes)
//! with the same commands through `craft3d-cli`. A 0.1.x fix (A-030) changes
//! nothing here: every case must match its golden, none is declared.
//!
//! Numbers are held to 1e-6 (poses, boxes, mate values) and 1e-5 (measures,
//! which a search finds: a clearance found moves by 1e-7 when a pose moves
//! by 1e-14); statuses, notes and texts exactly.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use common::session_in;
use serde_json::{Value, json};
use std::path::PathBuf;

fn published() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/examples/published")
}

fn golden() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/published_goldens/published-assemblies.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// The digest of one case, as the golden holds it.
fn digest(file: &str, setup: &[(&str, Value)], motion: bool) -> Value {
    let mut s = session_in(&published());
    run(
        &mut s,
        "file.open",
        json!({ "path": format!("{file}.3dcraftasm") }),
    );
    for (id, params) in setup {
        run(&mut s, id, params.clone());
    }
    let inspect = run(&mut s, "assembly.inspect", json!({ "measures": true }));
    let solve = run(&mut s, "assembly.solve", json!({}));
    let asserts = run(&mut s, "assert.list", json!({}));
    let instances = inspect["instances"].as_array().unwrap();
    let pose = |i: &Value| {
        json!([
            i["name"],
            i["pose"]["origin"],
            i["pose"]["x"],
            i["pose"]["y"],
            i["pose"]["z"]
        ])
    };
    let at_least = |message: &str| -> Value {
        message
            .split("at least ")
            .nth(1)
            .and_then(|rest| {
                let number: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | 'e' | 'E' | '+'))
                    .collect();
                number.parse::<f64>().ok()
            })
            .map_or(Value::Null, |v| json!(v))
    };
    let mut d = json!({
        "poses": instances.iter().filter(|i| !i["pose"].is_null()).map(pose).collect::<Vec<_>>(),
        "boxes": instances.iter().map(|i| {
            let b = &i["measured"]["box"];
            json!([i["name"], b.get(0).cloned().unwrap_or(Value::Null), b.get(1).cloned().unwrap_or(Value::Null)])
        }).collect::<Vec<_>>(),
        "mates": solve["mates"].as_array().unwrap().iter()
            .map(|m| json!([m["name"], m["status"], m["value"]])).collect::<Vec<_>>(),
        "dof": solve["dof"],
        "error": solve["error"],
        "note": solve["note"],
        "asserts": asserts["assertions"].as_array().unwrap().iter().map(|a| {
            json!([a["name"], a["status"], a["measured"], at_least(a["message"].as_str().unwrap_or(""))])
        }).collect::<Vec<_>>(),
    });
    if motion {
        let m = run(
            &mut s,
            "motion.check",
            json!({ "samples": 73, "details": true }),
        );
        d["motion"] = json!({
            "ok": m["ok"],
            "deadPoints": m["deadPoints"].as_array().map_or(0, Vec::len),
            "assertions": m["assertions"].as_array().unwrap().iter().map(|a| {
                json!([
                    a["name"], a["status"], a["min"]["value"], a["max"]["value"],
                    a["samples"].as_array().map(|ss| ss.iter().map(|s| s["status"].clone()).collect::<Vec<_>>())
                        .unwrap_or_default()
                ])
            }).collect::<Vec<_>>(),
        });
    }
    d
}

/// Every difference between `got` and `want` beyond the tolerances, as
/// paths: numbers under `poses`, `boxes` and `mates` to 1e-6, other
/// numbers to 1e-5, everything else exactly.
fn differences(path: &str, got: &Value, want: &Value, out: &mut Vec<String>) {
    match (got, want) {
        (Value::Number(a), Value::Number(b)) => {
            let (a, b) = (a.as_f64().unwrap(), b.as_f64().unwrap());
            let fine = ["poses", "boxes", "mates"]
                .iter()
                .any(|k| path.starts_with(k));
            let tol = if fine { 1e-6 } else { 1e-5 };
            if (a - b).abs() > tol {
                out.push(format!("{path}: {a} (golden {b})"));
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            if a.len() != b.len() {
                out.push(format!("{path}: {} items (golden {})", a.len(), b.len()));
                return;
            }
            for (k, (x, y)) in a.iter().zip(b).enumerate() {
                differences(&format!("{path}[{k}]"), x, y, out);
            }
        }
        (Value::Object(a), Value::Object(b)) => {
            for (k, y) in b {
                differences(
                    &format!("{path}.{k}"),
                    a.get(k).unwrap_or(&Value::Null),
                    y,
                    out,
                );
            }
        }
        (a, b) if a != b => out.push(format!("{path}: {a} (golden {b})")),
        _ => {}
    }
}

/// Checks one case against the golden: no difference at all, or only in
/// the parts `changed` names (by their path's first part).
fn check(file: &str, case: &str, got: &Value, changed: &[&str]) {
    let want = &golden()[file][case];
    assert!(!want.is_null(), "no golden for {file} / {case}");
    let mut out = Vec::new();
    for (k, w) in want.as_object().unwrap() {
        if changed.contains(&k.as_str()) {
            continue;
        }
        differences(k, &got[k], w, &mut out);
    }
    assert!(
        out.is_empty(),
        "{file} / {case} differs from its golden:\n{}",
        out.join("\n")
    );
}

#[test]
fn the_toggle_clamp_is_as_pinned() {
    let got = digest("toggle-clamp", &[], true);
    check("toggle-clamp", "default", &got, &[]);
}

#[test]
fn the_scissor_jack_is_as_pinned() {
    let got = digest("scissor-jack", &[], true);
    check("scissor-jack", "default", &got, &[]);
}

#[test]
fn the_radial_engine_is_as_pinned() {
    let got = digest("radial-engine", &[], true);
    check("radial-engine", "default", &got, &[]);
}

/// The walker at six crank starts: the published range's steps of 60°
/// (as published) and 180°, outside it. With A-030 alone nothing differs,
/// not even at 180°, where the rest solve still lands two legs on their
/// mirrored loop as 0.1.2 did (A-016, which would carry it, is not in 0.1.x).
#[test]
fn the_jansen_walker_is_as_pinned_at_its_crank_starts() {
    for start in [-120, -60, 0, 60, 120, 180] {
        let setup = [(
            "variable.set",
            json!({ "name": "crank_start", "expr": format!("{start} deg") }),
        )];
        let got = digest("jansen-walker", &setup, start == 0 || start == 180);
        check("jansen-walker", &format!("crank_start {start}"), &got, &[]);
    }
}
