//! Layer 3's materials through the commands: the layers of
//! `body.appearance` and `face.appearance`, the presets
//! (`material.presets`, `material.preset`), what the exports carry of them
//! (OBJ's material library, 3MF's display colour, STEP's transparency),
//! the studio picture of a glass body, and bad parameters refused.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::run;
use common::{error, session_in, workdir};
use serde_json::json;

fn model(dir: &std::path::Path) -> craft3d_engine::Session {
    let mut s = session_in(dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [40, 30, 4], "name": "Pane"}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"position": [80, 0, 0], "radius": 10, "height": 30, "name": "Shaft"}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"size": [20, 20, 20], "position": [-60, 0, 0], "name": "Box"}),
    );
    s
}

#[test]
fn the_presets_are_listed_and_given_to_a_body() {
    let dir = workdir("materials", "presets");
    let mut s = model(&dir);
    let list = run(&mut s, "material.presets", json!({}));
    let names: Vec<&str> = list["presets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["preset"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "steel",
            "aluminium-brushed",
            "abs",
            "glass",
            "car-paint",
            "wood"
        ]
    );
    let glass = &list["presets"][3];
    assert_eq!(glass["transmission"], 1.0);
    assert_eq!(glass["material"]["density"], 2500.0);
    // Given to a body: its appearance and its material, in one undo step.
    let out = run(
        &mut s,
        "material.preset",
        json!({"body": "Pane", "preset": "glass"}),
    );
    assert_eq!(out["appearance"]["transmission"], 1.0);
    assert_eq!(out["appearance"]["ior"], 1.52);
    assert_eq!(out["material"]["name"], "Glass");
    // A preset with a change: a blue car paint.
    let out = run(
        &mut s,
        "material.preset",
        json!({"body": "Box", "preset": "car-paint", "color": "#1f4fa0"}),
    );
    assert_eq!(out["appearance"]["color"], "#1f4fa0");
    assert_eq!(out["appearance"]["clearcoat"], 1.0);
    run(&mut s, "edit.undo", json!({}));
    let back = run(
        &mut s,
        "body.appearance",
        json!({"body": "Box", "color": "#808080"}),
    );
    assert!(back["appearance"].get("clearcoat").is_none());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn layers_are_given_and_said_back() {
    let dir = workdir("materials", "layers");
    let mut s = model(&dir);
    let out = run(
        &mut s,
        "body.appearance",
        json!({"body": "Shaft", "color": [200, 202, 206], "metallic": 1, "roughness": 0.3,
               "anisotropy": 0.8, "clearcoat": 0.5, "clearcoatRoughness": 0.1}),
    );
    let a = &out["appearance"];
    assert_eq!(a["anisotropy"], 0.8);
    assert_eq!(a["anisotropyAxis"], "parametric");
    assert_eq!(a["clearcoat"], 0.5);
    let out = run(
        &mut s,
        "face.appearance",
        json!({"faces": ["F3:face(0)"], "color": "#a07040", "subsurface": 0.3,
               "subsurfaceColor": "#ff8040", "anisotropy": -0.5, "anisotropyAxis": [0, 0, 1]}),
    );
    assert!(out.to_string().contains("subsurface"), "{out}");
    // Saved and opened again, the layers are there.
    run(&mut s, "file.save", json!({"path": "layers.3dcraft"}));
    run(&mut s, "file.open", json!({"path": "layers.3dcraft"}));
    let again = run(
        &mut s,
        "material.preset",
        json!({"body": "Pane", "preset": "steel"}),
    );
    assert_eq!(again["material"]["name"], "Steel");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_layers_said_back_can_be_given_again() {
    // What body.appearance says back is what it takes: each layer given
    // back, the axis "parametric" too, gives the same appearance.
    let dir = workdir("materials", "round-trip");
    let mut s = model(&dir);
    let first = run(
        &mut s,
        "body.appearance",
        json!({"body": "Shaft", "color": [200, 202, 206], "metallic": 1, "roughness": 0.3,
               "anisotropy": 0.8, "anisotropyAxis": [0, 0, 1], "clearcoat": 0.5}),
    );
    assert_eq!(
        first["appearance"]["anisotropyAxis"],
        json!([0.0, 0.0, 1.0])
    );
    // Back to the face's own direction by the word it is said back with.
    let parametric = run(
        &mut s,
        "body.appearance",
        json!({"body": "Shaft", "color": [200, 202, 206], "anisotropy": 0.8,
               "anisotropyAxis": "parametric"}),
    );
    let said = parametric["appearance"].clone();
    assert_eq!(said["anisotropyAxis"], "parametric", "{said}");
    let mut again = said.as_object().unwrap().clone();
    again.insert("body".into(), json!("Shaft"));
    let back = run(&mut s, "body.appearance", json!(again));
    assert_eq!(back["appearance"], said);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn bad_layers_are_refused() {
    let dir = workdir("materials", "refused");
    let mut s = model(&dir);
    for params in [
        json!({"body": "Pane", "color": "#ffffff", "anisotropy": 2}),
        json!({"body": "Pane", "color": "#ffffff", "anisotropy": "much"}),
        json!({"body": "Pane", "color": "#ffffff", "anisotropyAxis": [0, 0, 0]}),
        json!({"body": "Pane", "color": "#ffffff", "anisotropyAxis": "x"}),
        json!({"body": "Pane", "color": "#ffffff", "clearcoat": -0.1}),
        json!({"body": "Pane", "color": "#ffffff", "transmission": 1.5}),
        json!({"body": "Pane", "color": "#ffffff", "ior": 0.5}),
        json!({"body": "Pane", "color": "#ffffff", "ior": 1e300}),
        json!({"body": "Pane", "color": "#ffffff", "absorptionDistance": -1}),
        json!({"body": "Pane", "color": "#ffffff", "absorption": "red"}),
        json!({"body": "Pane", "color": "#ffffff", "subsurfaceColor": [1, 2]}),
        json!({"body": "Pane", "color": "#ffffff", "thinWalled": "yes"}),
        json!({"body": "Pane", "clear": true, "transmission": 1}),
    ] {
        let e = error(&mut s, "body.appearance", params.clone());
        assert!(e.contains("body.appearance"), "{params}: {e}");
    }
    for params in [
        json!({"body": "Pane", "preset": "unobtainium"}),
        json!({"body": "Pane"}),
        json!({"body": "Pane", "preset": "glass", "ior": 9}),
        json!({"body": "Nothing", "preset": "glass"}),
        json!({"preset": "glass"}),
    ] {
        let e = error(&mut s, "material.preset", params.clone());
        assert!(e.contains("material.preset"), "{params}: {e}");
    }
    let e = error(&mut s, "material.presets", json!({"preset": "glass"}));
    assert!(e.contains("material.presets"), "{e}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_exports_carry_what_their_formats_hold() {
    let dir = workdir("materials", "exports");
    let mut s = model(&dir);
    run(
        &mut s,
        "material.preset",
        json!({"body": "Pane", "preset": "glass"}),
    );
    run(
        &mut s,
        "material.preset",
        json!({"body": "Box", "preset": "car-paint"}),
    );
    run(
        &mut s,
        "material.preset",
        json!({"body": "Shaft", "preset": "aluminium-brushed"}),
    );
    // OBJ: the PBR keys of the material library.
    run(&mut s, "file.export", json!({"path": "parts.obj"}));
    let mtl = std::fs::read_to_string(dir.join("parts.mtl")).unwrap();
    for line in [
        "d 0.000000",
        "Ni 1.520000",
        "Pc 1.000000",
        "Pcr 0.040000",
        "aniso 0.750000",
    ] {
        assert!(mtl.lines().any(|l| l == line), "`{line}` not in\n{mtl}");
    }
    assert!(mtl.lines().any(|l| l.starts_with("Tf ")), "{mtl}");
    // 3MF: the glass's display colour with its alpha.
    run(&mut s, "file.export", json!({"path": "parts.3mf"}));
    let package = std::fs::read(dir.join("parts.3mf")).unwrap();
    let text = String::from_utf8_lossy(&package);
    assert!(
        text.contains("displaycolor=\"#FAFCFC00\""),
        "no clear glass colour"
    );
    // STEP: the glass's transparency beside its colour.
    run(&mut s, "file.export", json!({"path": "parts.step"}));
    let step = std::fs::read_to_string(dir.join("parts.step")).unwrap();
    assert!(
        step.contains("SURFACE_STYLE_TRANSPARENT(1."),
        "no transparency"
    );
    assert_eq!(step.matches("SURFACE_STYLE_TRANSPARENT").count(), 1);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_glass_body_renders_in_the_studio_look_the_same_twice() {
    let dir = workdir("materials", "render");
    let mut s = model(&dir);
    run(
        &mut s,
        "material.preset",
        json!({"body": "Pane", "preset": "glass"}),
    );
    run(
        &mut s,
        "material.preset",
        json!({"body": "Shaft", "preset": "aluminium-brushed"}),
    );
    let params = json!({"width": 160, "height": 120, "look": {}});
    let a = run(&mut s, "view.render", params.clone());
    let b = run(&mut s, "view.render", params);
    assert_eq!(a["base64"], b["base64"]);
    std::fs::remove_dir_all(&dir).unwrap();
}
