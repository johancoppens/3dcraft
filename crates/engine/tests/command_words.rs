//! Words as a person or an agent writes them (R1, the second field
//! test): `document.inspect` lists a feature's parameters in the
//! document's own words (`params`: `keep_tool`, `"target": 1`) and, asked
//! with `named: true`, where those differ, in the commands' (`named`:
//! `keepTool`, `"target": "Plate"`), finding 27, `params` staying as it
//! was; and a plain number
//! in `sin`, `cos` or `tan` that looks like degrees is warned about,
//! its meaning (radians) unchanged.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use common::session;
use serde_json::{Value, json};

fn feature(inspect: &Value, name: &str) -> Value {
    inspect["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == name)
        .unwrap()
        .clone()
}

#[test]
fn inspect_names_parameters_as_the_commands_do() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": [40, 20, 5]}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Pin", "radius": 3, "height": 9, "position": [10, 10, -2]}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Plate", "tool": "Pin", "keepTool": true, "name": "Cut"}),
    );
    let inspect = run(
        &mut s,
        "document.inspect",
        json!({"measures": false, "named": true}),
    );
    let cut = feature(&inspect, "Cut");
    // The document's own form, unchanged.
    assert_eq!(cut["params"]["keep_tool"], true, "{cut}");
    assert_eq!(cut["params"]["target"], 1, "{cut}");
    assert_eq!(cut["params"]["tool"], 2, "{cut}");
    // The commands' words.
    let named = &cut["named"];
    assert_eq!(named["keepTool"], true, "{cut}");
    assert_eq!(named["target"], "Plate", "{cut}");
    assert_eq!(named["tool"], "Pin", "{cut}");
    assert!(named.get("keep_tool").is_none(), "{cut}");
    assert_eq!(named["op"], cut["params"]["op"], "{cut}");
    // A feature whose parameters say the same both ways has no `named`.
    let plate = feature(&inspect, "Plate");
    assert!(plate.get("named").is_none(), "{plate}");
    // Asked for only: the output agents read by default is as it was.
    let plain = run(&mut s, "document.inspect", json!({"measures": false}));
    assert!(feature(&plain, "Cut").get("named").is_none(), "{plain}");
}

#[test]
fn a_plain_number_like_degrees_in_sin_or_cos_is_warned_about() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    let set = |s: &mut craft3d_engine::Session, expr: &str| {
        run(s, "variable.set", json!({"name": "k", "expr": expr}))
    };
    let warned = set(&mut s, "10 mm * sin(30)");
    // Radians, as before: the value does not change.
    assert!(
        (warned["value"].as_f64().unwrap() - 10.0 * 30f64.sin()).abs() < 1e-12,
        "{warned}"
    );
    let warning = warned["warning"].as_str().unwrap();
    assert!(
        warning.contains("`sin(30)`") && warning.contains("`sin(30 deg)`"),
        "{warning}"
    );
    assert!(
        set(&mut s, "cos( 45 ) * 1 mm")["warning"]
            .as_str()
            .unwrap()
            .contains("`cos(45 deg)`")
    );
    // Radians as meant, a unit, a variable, another function: no warning.
    for fine in [
        "10 mm * sin(0.5)",
        "10 mm * sin(30 deg)",
        "10 mm * cos(pi / 3)",
        "asin(0.5) * 1 mm / (1 rad)",
        "10 mm * tan(2)",
    ] {
        let result = set(&mut s, fine);
        assert!(result.get("warning").is_none(), "{fine}: {result}");
    }
}
