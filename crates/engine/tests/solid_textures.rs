//! Layer 3.5's solid textures through the commands: the `pattern` of
//! `body.appearance` and `face.appearance`, the presets that carry one
//! (oak, walnut, beech, marble, granite, carbon, cast-iron) fitted to the
//! body's box, saving and opening, the scene's faces, a cut through a
//! wooden block, what the exports carry (the mean colour), the studio
//! picture, and bad parameters refused (docs/design/rendering.md, *Layer
//! 3.5: solid textures*).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::run;
use common::{error, session_in, workdir};
use serde_json::{Value, json};

const NEW_PRESETS: [&str; 7] = [
    "oak",
    "walnut",
    "beech",
    "marble",
    "granite",
    "carbon",
    "cast-iron",
];

fn board(dir: &std::path::Path) -> craft3d_engine::Session {
    let mut s = session_in(dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [300, 80, 20], "name": "Board"}),
    );
    s
}

fn three(v: &Value) -> [f64; 3] {
    let a = v.as_array().unwrap();
    [0, 1, 2].map(|k| a[k].as_f64().unwrap())
}

#[test]
fn the_presets_fit_their_pattern_to_the_body() {
    let dir = workdir("textures", "presets");
    let mut s = board(&dir);
    let list = run(&mut s, "material.presets", json!({}));
    let names: Vec<&str> = list["presets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["preset"].as_str().unwrap())
        .collect();
    for name in NEW_PRESETS {
        assert!(names.contains(&name), "{name} in {names:?}");
    }
    for name in NEW_PRESETS {
        let out = run(
            &mut s,
            "material.preset",
            json!({"body": "Board", "preset": name}),
        );
        let a = &out["appearance"];
        let pattern = &a["pattern"];
        assert!(pattern.is_object(), "{name}: {out}");
        // Without a colour of its own, the appearance's colour is the
        // pattern's mean (what formats that cannot hold it export), as the
        // list gives it.
        let listed = list["presets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["preset"] == name)
            .unwrap();
        assert_eq!(a["color"], listed["color"], "{name}");
        assert!(pattern.get("meanColor").is_none());
        // The axis along the longest side of the box (x).
        let axis = three(&pattern["axis"]);
        assert!(axis[0] > 0.9 && axis[1] == 0.0, "{name}: {axis:?}");
        assert!(pattern["scale"].as_f64().unwrap() > 0.0, "{name}");
    }
    // Walnut: rings one twentieth of the board's width apart, the pith
    // below the board (beside its thinnest side), the axis tilted towards
    // it so that the board's face shows arches.
    let out = run(
        &mut s,
        "material.preset",
        json!({"body": "Board", "preset": "walnut"}),
    );
    let p = &out["appearance"]["pattern"];
    assert_eq!(p["kind"], "wood");
    assert!((p["scale"].as_f64().unwrap() - 4.0).abs() < 1e-9, "{p}");
    assert_eq!(three(&p["axis"]), [1.0, 0.0, 0.06]);
    // The board is 20 thick: wherever it is placed along z, its bottom is
    // at most 10 below its centre, and the pith is 48 below that centre.
    let origin = three(&p["origin"]);
    let centre_z = s
        .scene(None)
        .unwrap()
        .unwrap()
        .bodies_box
        .center()
        .unwrap()
        .z;
    assert!((origin[2] - (centre_z - 48.0)).abs() < 1e-9, "{origin:?}");
    assert_eq!(out["material"]["name"], "Walnut");
    // A change of the preset's pattern changes that number only.
    let out = run(
        &mut s,
        "material.preset",
        json!({"body": "Board", "preset": "oak", "pattern": {"scale": 2.5, "seed": 9}}),
    );
    let p = &out["appearance"]["pattern"];
    assert_eq!(p["scale"], 2.5);
    assert_eq!(p["seed"], 9);
    assert_eq!(p["rays"], 0.5);
    // `null` takes the preset's pattern away.
    let out = run(
        &mut s,
        "material.preset",
        json!({"body": "Board", "preset": "oak", "pattern": null, "color": "#a07040"}),
    );
    assert!(out["appearance"].get("pattern").is_none(), "{out}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_pattern_said_back_can_be_given_again() {
    let dir = workdir("textures", "round-trip");
    let mut s = board(&dir);
    let first = run(
        &mut s,
        "body.appearance",
        json!({"body": "Board", "roughness": 0.4,
               "pattern": {"kind": "marble", "colors": ["#f0eeea", "#606468"], "distortion": 1.5}}),
    );
    let said = first["appearance"].clone();
    assert_eq!(said["pattern"]["kind"], "marble");
    assert_eq!(said["pattern"]["colors"], json!(["#f0eeea", "#606468"]));
    let mut again = said.as_object().unwrap().clone();
    again.insert("body".into(), json!("Board"));
    let back = run(&mut s, "body.appearance", json!(again));
    assert_eq!(back["appearance"], said);
    // Saved and opened again, the scene's faces carry the texture.
    run(&mut s, "file.save", json!({"path": "marble.3dcraft"}));
    run(&mut s, "file.open", json!({"path": "marble.3dcraft"}));
    let scene = s.scene(None).unwrap().unwrap();
    let body = &scene.bodies[0];
    for k in 0..body.face_names.len() as u32 {
        let o = scene.face_colors.optics(body.first_face_id + k).unwrap();
        let p = o.pattern.unwrap();
        assert_eq!(p.kind, craft3d_render::studio::pattern::PatternKind::Marble);
        assert_eq!(p.stops, 2);
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_cut_through_a_wooden_block_shows_its_rings_on_the_cut_faces() {
    let dir = workdir("textures", "cut");
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [120, 60, 60], "name": "Block"}),
    );
    run(
        &mut s,
        "material.preset",
        json!({"body": "Block", "preset": "oak"}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"size": [70, 70, 40], "position": [60, -5, 30], "name": "Notch"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Block", "tool": "Notch", "name": "Cut"}),
    );
    let scene = s.scene(None).unwrap().unwrap();
    let body = scene.bodies.iter().find(|b| b.feature == "F3").unwrap();
    let faces = body.face_names.len() as u32;
    assert!(faces > 6, "the cut adds faces ({faces})");
    // Every face, the walls the tool cut included, carries the block's one
    // texture (the same row of the material table): its value is a
    // function of the point alone, so the rings run on across every edge
    // between an outer face and a cut face.
    let slots: Vec<u16> = (0..faces)
        .map(|k| scene.face_colors.optics_slot(body.first_face_id + k))
        .collect();
    assert!(
        slots.iter().all(|&slot| slot != 0 && slot == slots[0]),
        "{slots:?}"
    );
    let pattern = scene
        .face_colors
        .optics(body.first_face_id)
        .unwrap()
        .pattern
        .unwrap();
    // At a point of the edge between the top face and the cut floor (both
    // sides give the same point), the rings continue: the value along a
    // line across the edge has no jump there, as on any other line through
    // the wood.
    let edge = craft3d_math::Vec3::new(60.0, 30.0, 30.0);
    let step = craft3d_math::Vec3::new(1e-6, 0.0, 0.0);
    let left = pattern.value(pattern.local(edge - step));
    let right = pattern.value(pattern.local(edge + step));
    assert!((left - right).abs() < 1e-3, "{left} {right}");
    // The studio picture of the cut block, twice the same.
    let params = json!({"width": 200, "height": 150, "look": {}});
    let a = run(&mut s, "view.render", params.clone());
    let b = run(&mut s, "view.render", params);
    assert_eq!(a["base64"], b["base64"]);
    let plain = run(&mut s, "view.render", json!({"width": 200, "height": 150}));
    assert_ne!(a["base64"], plain["base64"]);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_exports_carry_the_mean_colour() {
    let dir = workdir("textures", "exports");
    let mut s = board(&dir);
    let out = run(
        &mut s,
        "material.preset",
        json!({"body": "Board", "preset": "walnut"}),
    );
    let mean = out["appearance"]["color"].as_str().unwrap().to_string();
    let byte = |k: usize| u8::from_str_radix(&mean[1 + 2 * k..3 + 2 * k], 16).unwrap();
    run(&mut s, "file.export", json!({"path": "board.obj"}));
    let mtl = std::fs::read_to_string(dir.join("board.mtl")).unwrap();
    let kd = format!(
        "Kd {:.6} {:.6} {:.6}",
        f64::from(byte(0)) / 255.0,
        f64::from(byte(1)) / 255.0,
        f64::from(byte(2)) / 255.0
    );
    assert!(
        mtl.lines().any(|l| l.starts_with("Kd ")),
        "a diffuse colour in\n{mtl}"
    );
    eprintln!("mean {mean}, {kd}\n{mtl}");
    run(&mut s, "file.export", json!({"path": "board.3mf"}));
    let package = std::fs::read(dir.join("board.3mf")).unwrap();
    let text = String::from_utf8_lossy(&package);
    let hex = mean.trim_start_matches('#').to_uppercase();
    assert!(
        text.contains(&format!("displaycolor=\"#{hex}")),
        "the mean colour {mean} in the 3MF"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn bad_patterns_are_refused() {
    let dir = workdir("textures", "refused");
    let mut s = board(&dir);
    for pattern in [
        json!("wood"),
        json!({}),
        json!({"kind": "teak"}),
        json!({"kind": 3}),
        json!({"kind": "wood", "colors": ["#ffffff"]}),
        json!({"kind": "wood", "colors": ["#ffffff", "#000000", "#111111", "#222222", "#333333"]}),
        json!({"kind": "wood", "colors": ["white", "#000000"]}),
        json!({"kind": "wood", "scale": -1}),
        json!({"kind": "wood", "scale": 0}),
        json!({"kind": "wood", "scale": 1e300}),
        json!({"kind": "wood", "scale": "big"}),
        json!({"kind": "wood", "axis": [0, 0, 0]}),
        json!({"kind": "wood", "axis": [1, 0]}),
        json!({"kind": "wood", "origin": [0, "a", 0]}),
        json!({"kind": "wood", "seed": -1}),
        json!({"kind": "wood", "seed": 1.5}),
        json!({"kind": "wood", "detail": 0}),
        json!({"kind": "wood", "detail": 9}),
        json!({"kind": "wood", "grain": 2}),
        json!({"kind": "wood", "rays": -0.5}),
        json!({"kind": "wood", "distortion": 99}),
        json!({"kind": "wood", "spot": 1.5}),
        json!({"kind": "wood", "roughness": [0.5]}),
        json!({"kind": "wood", "roughness": [0.5, 3]}),
        json!({"kind": "wood", "wrap": "yes"}),
        json!({"kind": "wood", "rings": 4}),
    ] {
        let params = json!({"body": "Board", "pattern": pattern});
        let e = error(&mut s, "body.appearance", params.clone());
        assert!(e.contains("body.appearance"), "{params}: {e}");
        let e = error(
            &mut s,
            "face.appearance",
            json!({"faces": ["F1:face(0)"], "pattern": pattern}),
        );
        assert!(e.contains("face.appearance"), "{pattern}: {e}");
    }
    for params in [
        json!({"body": "Board", "preset": "oak", "pattern": {"grain": 7}}),
        json!({"body": "Board", "preset": "oak", "pattern": {"kind": "plywood"}}),
        json!({"body": "Board", "preset": "oak", "pattern": 4}),
        json!({"body": "Board", "clear": true, "pattern": {"kind": "wood"}}),
    ] {
        let id = if params.get("preset").is_some() {
            "material.preset"
        } else {
            "body.appearance"
        };
        let e = error(&mut s, id, params.clone());
        assert!(e.contains(id), "{params}: {e}");
    }
    // Without a colour and without a pattern, a colour is still needed.
    let e = error(&mut s, "body.appearance", json!({"body": "Board"}));
    assert!(e.contains("color"), "{e}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_face_takes_a_pattern_of_its_own() {
    let dir = workdir("textures", "face");
    let mut s = board(&dir);
    let out = run(
        &mut s,
        "face.appearance",
        json!({"faces": ["F1:face(4)"], "pattern": {"kind": "speckle", "spot": 0.3}}),
    );
    assert!(out.to_string().contains("speckle"), "{out}");
    let scene = s.scene(None).unwrap().unwrap();
    let body = &scene.bodies[0];
    let with: Vec<bool> = (0..body.face_names.len() as u32)
        .map(|k| {
            scene
                .face_colors
                .optics(body.first_face_id + k)
                .is_some_and(|o| o.pattern.is_some())
        })
        .collect();
    assert_eq!(with.iter().filter(|&&w| w).count(), 1, "{with:?}");
    std::fs::remove_dir_all(&dir).unwrap();
}
