//! A Miura-ori made from sketch patterns, driven by one crease (the
//! Miura Jig's repro 3): one degree of freedom, the kinds pick the branch.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;

use common::plain::run;
use common::{session_in, workdir};
use serde_json::Value;

fn steps(name: &str) -> Vec<(String, Value)> {
    let text = std::fs::read_to_string(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fold_repros")
            .join(name),
    )
    .unwrap();
    let value: Value = serde_json::from_str(&text).unwrap();
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|s| (s[0].as_str().unwrap().to_string(), s[1].clone()))
        .collect()
}

#[test]
fn a_miura_sheet_of_sketch_patterns_driven_by_one_crease_folds() {
    let dir = workdir("fold_miura", "3x12");
    let mut s = session_in(&dir);
    let mut inspect = Value::Null;
    for (id, params) in steps("miura_3x12.steps.json") {
        let out = run(&mut s, &id, params);
        if id == "fold.inspect" {
            inspect = out;
        }
    }
    assert_eq!(inspect["ok"], true, "{}", inspect["error"]);
    // The closed form of the Miura-ori (Tachi 2009; docs/design/origami.md
    // › *Closed forms*): every straight piece by the driving angle, every
    // zigzag piece by 2 atan(tan(rho / 2) / cos gamma).
    let (rho, gamma) = (30f64.to_radians(), 60f64.to_radians());
    let zigzag = (2.0 * ((rho / 2.0).ptan() / gamma.pcos()).patan()).to_degrees();
    let mut pieces = 0;
    for crease in inspect["creases"].as_array().unwrap() {
        let line = crease["line"].as_str().unwrap();
        let want = if ["e21", "e24", "e27", "e30"]
            .iter()
            .any(|z| line.starts_with(z))
        {
            zigzag
        } else {
            30.0
        };
        for piece in crease["pieces"].as_array().unwrap() {
            let got = piece["angle"].as_f64().unwrap().abs();
            assert!((got - want).abs() < 1e-6, "{line}: {got} vs {want}");
            pieces += 1;
        }
    }
    assert!(pieces > 50, "{pieces}");
}
