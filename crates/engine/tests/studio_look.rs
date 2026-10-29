//! `view.render`'s `look`: the studio look of the guide's examples next to
//! the plain one (printed as evidence lines, `craft3d_testkit::evidence`),
//! the plain look unchanged when no look is given, and bad looks refused.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::run;
use serde_json::{Value, json};
use std::path::PathBuf;

fn docs() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs")
}

fn example(name: &str) -> craft3d_engine::Session {
    let text =
        std::fs::read_to_string(docs().join("examples").join(format!("{name}.json"))).unwrap();
    let script: Value = serde_json::from_str(&text).unwrap();
    let mut s = common::session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    for step in script.as_array().unwrap() {
        let id = step[0].as_str().unwrap();
        let params = step.get(1).cloned().unwrap_or_else(|| json!({}));
        run(&mut s, id, params);
    }
    s
}

fn print_png(name: &str, result: &Value) {
    let text = result["base64"].as_str().unwrap();
    let chars: Vec<char> = text.chars().collect();
    for (i, chunk) in chars.chunks(4000).enumerate() {
        let line: String = chunk.iter().collect();
        eprintln!("EV {name} {i} {line}");
    }
}

#[test]
fn the_examples_in_the_studio_look() {
    for name in ["flange", "gear", "bracket"] {
        let mut s = example(name);
        let plain = run(&mut s, "view.render", json!({"width": 480, "height": 360}));
        assert!(plain.get("look").is_none());
        let started = std::time::Instant::now();
        let studio = run(
            &mut s,
            "view.render",
            json!({"width": 480, "height": 360, "look": {}}),
        );
        eprintln!("{name}: studio picture in {:?}", started.elapsed());
        assert_eq!(studio["look"]["mode"], "studio");
        assert_eq!(studio["look"]["environment"], "studio");
        assert_ne!(plain["base64"], studio["base64"]);
        print_png(&format!("{name}-plain.png"), &plain);
        print_png(&format!("{name}-studio.png"), &studio);
        // The plain look given explicitly is the picture without a look.
        let explicit = run(
            &mut s,
            "view.render",
            json!({"width": 480, "height": 360, "look": {"mode": "plain"}}),
        );
        assert_eq!(explicit["base64"], plain["base64"]);
    }
    let mut s = example("gear");
    for (k, look) in [
        json!({"environment": "outdoor"}),
        json!({"environment": "workshop", "rotation": 120}),
        json!({"tonemap": "aces", "exposure": 0.5}),
    ]
    .into_iter()
    .enumerate()
    {
        let r = run(
            &mut s,
            "view.render",
            json!({"width": 480, "height": 360, "look": look}),
        );
        print_png(&format!("gear-look-{k}.png"), &r);
    }
    // A Jig, when one was put beside the tests for the evidence.
    let jig = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/evidence_input");
    if jig.join("gear-reducer.3dcraftjig").exists() {
        let mut s = common::session_in(&jig);
        run(
            &mut s,
            "file.open",
            json!({"path": "gear-reducer.3dcraftjig"}),
        );
        for (label, look) in [("plain", Value::Null), ("studio", json!({}))] {
            let r = run(
                &mut s,
                "view.render",
                json!({"width": 640, "height": 480, "look": look}),
            );
            print_png(&format!("gear-reducer-{label}.png"), &r);
        }
    }
}

#[test]
fn bad_looks_are_refused() {
    let mut s = example("wedge");
    for bad in [
        json!(3),
        json!({"environment": "moon"}),
        json!({"tonemap": "reinhard"}),
        json!({"mode": "photo"}),
        json!({"exposure": "bright"}),
        json!({"light": [1, 2, 3]}),
        json!({"glow": true}),
    ] {
        let e = s
            .execute(
                "view.render",
                &json!({"width": 64, "height": 64, "look": bad}),
            )
            .unwrap_err();
        assert!(e.to_string().contains("look"), "{e}");
    }
    // The ids mode ignores the look: ids are ids.
    let a = run(
        &mut s,
        "view.render",
        json!({"width": 64, "height": 64, "mode": "ids"}),
    );
    let b = run(
        &mut s,
        "view.render",
        json!({"width": 64, "height": 64, "mode": "ids", "look": {}}),
    );
    assert_eq!(a["base64"], b["base64"]);
}
