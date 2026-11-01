//! `acrossDegree` on `feature.surface.network` (format 62): the degree each
//! family is interpolated across with, 3 (the default) or 5. Left out it is
//! the cubic network as before; 5 makes another sheet through the same
//! curves; anything else is refused by name, at creation and by
//! `feature.edit`, and nothing crashes.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::{body, run};
use craft3d_engine::Session;
use serde_json::{Value, json};

/// The network of the v62 fixture: two u curves along x, six v curves
/// across them.
fn steps() -> Vec<(String, Value)> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../format/tests/fixtures/versions/v62.steps.json");
    let script: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    script
        .as_array()
        .unwrap()
        .iter()
        .map(|s| (s[0].as_str().unwrap().to_string(), s[1].clone()))
        .collect()
}

/// A session with the curves of the fixture and the network made with
/// `across` (left out for `None`).
fn session(across: Option<Value>) -> Session {
    let mut s = common::session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    for (id, mut params) in steps() {
        if id == "feature.surface.network" {
            match &across {
                Some(a) => params["acrossDegree"] = a.clone(),
                None => {
                    params.as_object_mut().unwrap().remove("acrossDegree");
                }
            }
        }
        run(&mut s, &id, params);
    }
    s
}

#[test]
fn a_network_is_cubic_across_unless_asked_for_quintic() {
    let plain = body(&mut session(None), "Quintic");
    let three = body(&mut session(Some(json!(3))), "Quintic");
    let five = body(&mut session(Some(json!(5))), "Quintic");
    // Left out and 3 are the same sheet.
    assert_eq!(plain["area"], three["area"]);
    assert_eq!(plain["bbox"], three["bbox"]);
    // 5 is another sheet through the same curves.
    let (a3, a5) = (
        three["area"].as_f64().unwrap(),
        five["area"].as_f64().unwrap(),
    );
    assert!((a3 - a5).abs() > 1e-9 * a3, "{a3} {a5}");
}

#[test]
fn an_across_degree_other_than_3_or_5_is_refused() {
    for bad in [
        json!(4),
        json!(1),
        json!(7),
        json!("5"),
        json!(-3),
        json!(5.5),
    ] {
        let mut s = common::session();
        run(&mut s, "file.new", json!({"units": "mm"}));
        let mut last = None;
        for (id, mut params) in steps() {
            if id == "feature.surface.network" {
                params["acrossDegree"] = bad.clone();
                last = Some(s.execute(&id, &params));
            } else {
                run(&mut s, &id, params);
            }
        }
        let err = last.unwrap().expect_err(&format!("{bad} was taken"));
        assert!(
            err.message().contains("acrossDegree"),
            "{bad}: {}",
            err.message()
        );
    }
    // By feature.edit too, and the feature keeps what it had.
    let mut s = session(Some(json!(5)));
    let err = s
        .execute(
            "feature.edit",
            &json!({"feature": "Quintic", "acrossDegree": 4}),
        )
        .expect_err("4 was taken by feature.edit");
    assert!(err.message().contains("acrossDegree"), "{}", err.message());
    let before = body(&mut s, "Quintic");
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Quintic", "acrossDegree": 3}),
    );
    let three = body(&mut s, "Quintic");
    assert_ne!(before["area"], three["area"]);
    assert_eq!(three["area"], body(&mut session(None), "Quintic")["area"]);
    // Null is the default again, as 3 is.
    let mut s = session(Some(json!(5)));
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Quintic", "acrossDegree": null}),
    );
    assert_eq!(body(&mut s, "Quintic")["area"], three["area"]);
}
