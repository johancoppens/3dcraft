//! Blends along chains of freeform edges through the commands: the
//! turbofan's core casing, cut open by a block, whose cut edges run from
//! the cone into the torus of the flange's root fillet and on along the
//! flange (built, checked deep, named, refilleted after an edit).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Access, Session, Workspace};
use serde_json::{Value, json};

fn session() -> Session {
    Session::new(Workspace {
        read: Access::None,
        write: Access::None,
    })
}

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn built(session: &mut Session, id: &str, params: Value) -> Value {
    let result = run(session, id, params.clone());
    assert_eq!(result["status"], "ok", "{id} {params}: {result}");
    result
}

fn body(session: &mut Session, name: &str) -> Value {
    let document = run(session, "document.inspect", json!({}));
    document["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == name)
        .unwrap_or_else(|| panic!("no body {name} in {document}"))
        .clone()
}

fn deep_ok(session: &mut Session) {
    let check = run(session, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
}

/// The core casing of the turbofan: the meridian revolved, the root of
/// its flange filleted, a quarter cut away by a block.
fn casing(s: &mut Session) {
    run(s, "file.new", json!({"units": "mm"}));
    run(s, "sketch.create", json!({"plane": "xy", "name": "CoreCaseSk"}));
    run(
        s,
        "sketch.polyline",
        json!({"sketch": "CoreCaseSk", "closed": true, "points": [[47.4316, 76.0], [49.9316, 76.0], [43.3789, 159.0], [55.1513, 159.0], [55.1513, 162.0], [40.6421, 162.0]]}),
    );
    built(
        s,
        "feature.revolve",
        json!({"sketch": "CoreCaseSk", "axis": "y", "name": "CoreCaseRaw"}),
    );
    built(
        s,
        "feature.fillet",
        json!({"edges": [["F2:side(e8)", "F2:side(e9)"]], "radius": 0.6, "name": "CoreCase"}),
    );
    built(
        s,
        "feature.block",
        json!({"size": [200.0, 400.0, 200.0], "position": [0.0, -50.0, 0.0], "name": "CutBox"}),
    );
    built(
        s,
        "feature.boolean",
        json!({"op": "subtract", "target": "CoreCase", "tool": "CutBox", "name": "CoreCaseCut"}),
    );
}

/// Both cut faces' edges along the outside, from the cone through the
/// root fillet to the flange, rounded as two chains in one feature (each
/// given by its edge on the cone, `chain` taking the rest).
#[test]
fn the_cut_edges_of_the_core_casing() {
    let mut s = session();
    casing(&mut s);
    let before = body(&mut s, "CoreCaseCut")["volume"].as_f64().unwrap();
    built(
        &mut s,
        "feature.fillet",
        json!({"edges": [
            ["F5:copy(0)/F2:side(e8)", "F5:copy(1)/F4:face(4)"],
            ["F5:copy(0)/F2:side(e8)", "F5:copy(1)/F4:face(0)"]
        ], "radius": 0.5, "chain": true, "name": "CutRound"}),
    );
    let rounded = body(&mut s, "CutRound");
    assert_eq!(rounded["valid"], true);
    let names: Vec<String> = rounded["faceNames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap().to_string())
        .collect();
    let fillets: Vec<&String> = names.iter().filter(|n| n.starts_with("F6:fillet(")).collect();
    // Three faces per chain: the cone's, the root fillet's, the flange's.
    assert_eq!(fillets.len(), 6, "{names:?}");
    let after = rounded["volume"].as_f64().unwrap();
    // Each chain is about 95 long and takes the corner of a square of the
    // radius less a quarter of the disc along it.
    let corner = 0.5 * 0.5 * (1.0 - std::f64::consts::FRAC_PI_4);
    let removed = before - after;
    assert!(
        removed > 2.0 * 90.0 * corner && removed < 2.0 * 100.0 * corner,
        "removed {removed}"
    );
    deep_ok(&mut s);
    // A smaller radius is a change like any other.
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "CutRound", "radius": 0.3}),
    );
    let smaller = body(&mut s, "CutRound")["volume"].as_f64().unwrap();
    assert!(smaller > after && smaller < before, "{after} -> {smaller}");
    deep_ok(&mut s);
}
