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

/// The script of an example: a guide's lesson in `docs/examples`, or one
/// of the models kept as tests in `tests/example_models` (the wedge, since
/// 2026-11-10).
fn example(name: &str) -> craft3d_engine::Session {
    let file = format!("{name}.json");
    let lesson = docs().join("examples").join(&file);
    let path = if lesson.is_file() {
        lesson
    } else {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/example_models")
            .join(&file)
    };
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
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
    // Models put beside the tests for the evidence (a Jig, the turbofan),
    // when they are there.
    let input = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/evidence_input");
    for file in ["gear-reducer.3dcraftjig", "turbofan.3dcraft"] {
        if !input.join(file).exists() {
            continue;
        }
        let mut s = common::session_in(&input);
        run(&mut s, "file.open", json!({"path": file}));
        let stem = file.split('.').next().unwrap_or(file);
        for (label, look) in layer_looks() {
            let started = std::time::Instant::now();
            let r = run(
                &mut s,
                "view.render",
                json!({"width": 640, "height": 480, "look": look}),
            );
            eprintln!("{stem} {label}: {:?}", started.elapsed());
            print_png(&format!("{stem}-{label}.png"), &r);
        }
    }
}

/// The looks of layer 2's before and after: plain, layer 1 alone (every
/// effect of layer 2 off), layer 2 (the default studio look), and layer 2
/// with a lens on the model's centre and 16 frames.
fn layer_looks() -> [(&'static str, Value); 4] {
    [
        ("plain", Value::Null),
        (
            "layer1",
            json!({"reflections": false, "contactShadows": false, "specularShadow": false,
                   "groundReflection": 0, "bloom": 0}),
        ),
        ("layer2", json!({})),
        (
            "layer2-dof",
            json!({"dof": {"fstop": 2.8}, "accumulate": 16}),
        ),
    ]
}

#[test]
fn layer_2_before_and_after() {
    for name in ["flange", "gear"] {
        let mut s = example(name);
        let mut pictures = Vec::new();
        for (label, look) in layer_looks() {
            let started = std::time::Instant::now();
            let r = run(
                &mut s,
                "view.render",
                json!({"width": 480, "height": 360, "look": look}),
            );
            eprintln!("{name} {label}: {:?}", started.elapsed());
            print_png(&format!("{name}-{label}.png"), &r);
            pictures.push(r);
        }
        // Layer 2 changes the picture; its look says so.
        assert_ne!(pictures[1]["base64"], pictures[2]["base64"]);
        assert_eq!(pictures[2]["look"]["reflections"], true);
        assert_eq!(pictures[3]["look"]["accumulate"], 16);
        assert_eq!(pictures[3]["look"]["dof"]["focus"], "centre");
    }
    // The metal flange: reflections show.
    let mut s = example("flange");
    let shown = run(&mut s, "view.render", json!({"width": 64, "height": 48}));
    if let Some(name) = shown["bodies"][0]["body"].as_str() {
        let name = name.to_string();
        run(
            &mut s,
            "body.appearance",
            json!({"body": name, "color": "#c8ccd2", "metallic": 1, "roughness": 0.2}),
        );
        for (label, look) in layer_looks() {
            let r = run(
                &mut s,
                "view.render",
                json!({"width": 480, "height": 360, "look": look}),
            );
            print_png(&format!("flange-metal-{label}.png"), &r);
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
        json!({"dof": {"fstop": "wide"}}),
        json!({"dof": {"focus": [1, 2]}}),
        json!({"accumulate": "many"}),
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

/// A still whose frames together would pass the CPU renderer's limit of
/// samples is refused at once, saying what to lower.
#[test]
fn too_many_frames_are_refused() {
    let mut s = example("wedge");
    let started = std::time::Instant::now();
    let e = s
        .execute(
            "view.render",
            &json!({"width": 2000, "height": 2000, "look": {"accumulate": 256}}),
        )
        .unwrap_err();
    assert!(e.to_string().contains("accumulate fewer frames"), "{e}");
    assert!(started.elapsed().as_secs() < 60, "refused before drawing");
}

/// A point `[x, y, z]` within 1e-9 of `want` (the model's box is the
/// B-rep's, to its rounding).
fn at(value: &Value, want: [f64; 3]) {
    let got: Vec<f64> = value
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert!(
        got.len() == 3 && got.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-9),
        "{value} {want:?}"
    );
}

/// `view.render`'s lens on the selection: it focuses on the centre of the
/// session's selection and says so under `lens`; with the selection
/// cleared it focuses on the model's centre and says why; an orthographic
/// view says it blurs nothing.
#[test]
fn view_render_focuses_the_lens_on_the_selection() {
    let mut s = common::session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Near", "size": [10, 10, 10], "position": [40, -25, -5]}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Far", "size": [10, 10, 10], "position": [-60, 15, -5]}),
    );
    let look = json!({"mode": "studio", "dof": {"fstop": 4, "focus": "selection"}});
    let render = |s: &mut craft3d_engine::Session, extra: Value| {
        let mut params = json!({"width": 96, "height": 72, "look": look});
        if let (Some(map), Value::Object(more)) = (params.as_object_mut(), extra) {
            map.extend(more);
        }
        run(s, "view.render", params)
    };
    run(
        &mut s,
        "select.set",
        json!({"items": [{"kind": "body", "feature": "Near"}]}),
    );
    let picture = render(&mut s, json!({}));
    at(&picture["lens"]["focus"], [45.0, -20.0, 0.0]);
    assert_eq!(picture["lens"]["fallback"], Value::Null);
    assert_eq!(picture["lens"]["noBlur"], Value::Null);
    run(
        &mut s,
        "select.set",
        json!({"items": [{"kind": "body", "feature": "Far"}]}),
    );
    let far = render(&mut s, json!({}));
    at(&far["lens"]["focus"], [-55.0, 20.0, 0.0]);
    assert_ne!(far["base64"], picture["base64"], "the focus moved");
    run(&mut s, "select.set", json!({"mode": "clear"}));
    let cleared = render(&mut s, json!({}));
    // The model's box is x -60..50, y -25..25, z -5..5.
    at(&cleared["lens"]["focus"], [-5.0, 0.0, 0.0]);
    let why = cleared["lens"]["fallback"].as_str().unwrap();
    assert!(why.contains("nothing selected"), "{why}");
    // A selection that is not shown is told apart from none.
    run(
        &mut s,
        "select.set",
        json!({"items": [{"kind": "body", "feature": "Near"}]}),
    );
    run(
        &mut s,
        "feature.visible",
        json!({"feature": "Near", "visible": false}),
    );
    let hidden = render(&mut s, json!({}));
    let why = hidden["lens"]["fallback"].as_str().unwrap();
    assert!(why.contains("not shown"), "{why}");
    run(
        &mut s,
        "feature.visible",
        json!({"feature": "Near", "visible": true}),
    );
    let ortho = render(&mut s, json!({"projection": "orthographic"}));
    let why = ortho["lens"]["noBlur"].as_str().unwrap();
    assert!(why.contains("orthographic"), "{why}");
    // An f-number that is not positive is refused.
    let error = common::refused(
        &mut s,
        "view.render",
        json!({"look": {"dof": {"fstop": 0}}}),
    );
    assert!(error.contains("positive f-number"), "{error}");
}
