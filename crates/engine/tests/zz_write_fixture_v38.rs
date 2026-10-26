//! TEMPORARY: writes the format 38 fixture on the Strix and prints it.
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
#[ignore]
fn write_fixture_v38() {
    let dir = workdir("fixture", "v38");
    let steps: Value = serde_json::from_str(include_str!(
        "../../format/tests/fixtures/versions/v38.steps.json"
    ))
    .unwrap();
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    for step in steps.as_array().unwrap() {
        run(&mut s, step[0].as_str().unwrap(), step[1].clone());
    }
    run(&mut s, "file.save", json!({"path": "v38.3dcraft"}));
    let bytes = std::fs::read(dir.join("v38.3dcraft")).unwrap();
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    println!("FIXTURE-HEX-BEGIN");
    for chunk in hex.as_bytes().chunks(4000) {
        println!("{}", std::str::from_utf8(chunk).unwrap());
    }
    println!("FIXTURE-HEX-END");
    let mut t = session_in(&dir);
    run(&mut t, "file.open", json!({"path": "v38.3dcraft"}));
    let inspect = run(&mut t, "document.inspect", json!({}));
    let features: Vec<Value> = inspect["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| json!({"id": f["id"], "name": f["name"], "status": f["status"]}))
        .collect();
    let bodies: Vec<Value> = inspect["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| {
            json!({"feature": b["feature"], "kind": b["kind"], "faces": b["faces"],
                   "edges": b["edges"], "vertices": b["vertices"], "volume": b["volume"],
                   "area": b["area"], "mass": b["mass"], "appearance": b["appearance"],
                   "material": b["material"]})
        })
        .collect();
    let listed = run(&mut t, "param.list", json!({}));
    let parameters: Vec<Value> = listed["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            json!({"name": p["name"], "kind": p["kind"], "expr": p["expr"],
                   "unit": p["unit"], "valid": p["valid"]})
        })
        .collect();
    let expected = json!({
        "format_version": 38,
        "rollback": inspect["rollback"],
        "features": features,
        "bodies": bodies,
        "parameters": parameters,
        "info": listed["info"],
    });
    println!("EXPECTED-BEGIN");
    println!("{}", serde_json::to_string_pretty(&expected).unwrap());
    println!("EXPECTED-END");
    println!("FNV {} {:#x}", bytes.len(), fnv1a(&bytes));
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}
