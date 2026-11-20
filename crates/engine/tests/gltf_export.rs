//! `file.export` to glTF 2.0 binary (`.glb`), read back by the test kit's
//! reader, written from the specification independently of the writer
//! (`craft3d_testkit::readers::gltf`): the same triangles, box and enclosed
//! volume as the binary STL of the same document at the same quality; the
//! bodies' names, their faces' names and the colours of bodies and faces;
//! metres with +Y up in every unit; the looks glTF cannot hold reported;
//! and what is refused (docs/file-formats.md › glTF).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::run;
use common::{error, session_in, workdir};
use craft3d_engine::Session;
use craft3d_meshio::gltf::srgb_to_linear;
use craft3d_testkit::readers::gltf::{Gltf, GltfNode, GltfPrimitive, read_glb};
use craft3d_testkit::readers::json::Json;
use serde_json::{Value, json};
use std::f64::consts::PI;
use std::path::Path;

const RED: [u8; 3] = [200, 16, 46];
const BLUE: [u8; 3] = [30, 90, 220];
const YELLOW: [u8; 3] = [240, 200, 20];
const GREEN: [u8; 3] = [20, 160, 70];
/// What the pictures draw a body without a colour in.
const GREY: [u8; 3] = [198, 204, 214];

/// Exports `name` with `params` and reads it back.
fn export(s: &mut Session, dir: &Path, name: &str, mut params: Value) -> (Value, Gltf) {
    params["path"] = json!(name);
    let out = run(s, "file.export", params);
    let bytes = std::fs::read(dir.join(name)).unwrap();
    assert_eq!(out["bytes"], json!(bytes.len()), "{out}");
    (out, read_glb(&bytes).unwrap())
}

/// Triangles, box and enclosed volume of a binary STL file.
fn stl_numbers(bytes: &[u8]) -> (usize, [f64; 6], f64) {
    let count = u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize;
    assert_eq!(bytes.len(), 84 + 50 * count);
    let triangles: Vec<[[f64; 3]; 3]> = bytes[84..]
        .chunks(50)
        .map(|record| {
            let f = |i: usize| f64::from(f32::from_le_bytes(record[i..i + 4].try_into().unwrap()));
            let v = |k: usize| [f(k), f(k + 4), f(k + 8)];
            [v(12), v(24), v(36)]
        })
        .collect();
    (count, bbox(&triangles), volume(&triangles))
}

fn bbox(triangles: &[[[f64; 3]; 3]]) -> [f64; 6] {
    let mut b = [f64::INFINITY; 6];
    for slot in &mut b[3..] {
        *slot = f64::NEG_INFINITY;
    }
    for p in triangles.iter().flatten() {
        for (axis, c) in p.iter().enumerate() {
            b[axis] = b[axis].min(*c);
            b[axis + 3] = b[axis + 3].max(*c);
        }
    }
    b
}

fn volume(triangles: &[[[f64; 3]; 3]]) -> f64 {
    triangles
        .iter()
        .map(|[a, b, c]| {
            (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                + a[2] * (b[0] * c[1] - b[1] * c[0]))
                / 6.0
        })
        .sum()
}

/// The triangles of every node in the document's unit and axes: glTF's
/// `(X, Y, Z)` in metres back to `(X, −Z, Y)` over `metres`.
fn triangles(file: &Gltf, metres: f64) -> Vec<[[f64; 3]; 3]> {
    file.nodes
        .iter()
        .flat_map(|n: &GltfNode| n.primitives.iter())
        .flat_map(|p| {
            p.triangles.iter().map(|t| {
                t.map(|k| {
                    let [x, y, z] = p.positions[k];
                    [x / metres, -z / metres, y / metres]
                })
            })
        })
        .collect()
}

/// The string member `key` of an object of `extras`.
fn text<'a>(json: &'a Json, key: &str) -> &'a str {
    json.get(key).and_then(Json::as_str).unwrap()
}

/// The number at a path of keys.
fn number(json: &Json, path: &[&str]) -> f64 {
    json.at(path).and_then(Json::as_f64).unwrap()
}

fn linear(c: [u8; 3]) -> [f64; 4] {
    let [r, g, b] = c.map(srgb_to_linear);
    [r, g, b, 1.0]
}

fn inspect_bodies(s: &mut Session) -> Vec<Value> {
    run(s, "document.inspect", json!({}))["bodies"]
        .as_array()
        .unwrap()
        .clone()
}

/// A block and a pin, exported as glTF and as binary STL at the same
/// mesh (quality coarse, and a tolerance and angle given): read back, the
/// same triangles, the same box and the same volume, to the 32-bit
/// rounding of each file, and the closed forms within the tessellation's
/// bound. Nodes are the bodies, named after their features, and each
/// primitive names its faces as document.inspect does.
#[test]
fn the_triangles_box_and_volume_are_the_stls() {
    let dir = workdir("gltf-export", "stl");
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [10, 20, 30], "name": "Block"}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"position": [40, 0, 0], "radius": 5, "height": 10, "name": "Pin"}),
    );
    for mesh in [
        json!({"quality": "coarse"}),
        json!({"tolerance": 0.02, "angle": 10}),
    ] {
        let (out, file) = export(&mut s, &dir, "model.glb", mesh.clone());
        let mut stl_params = mesh.clone();
        stl_params["path"] = json!("model.stl");
        let stl_out = run(&mut s, "file.export", stl_params);
        let (count, stl_box, stl_volume) =
            stl_numbers(&std::fs::read(dir.join("model.stl")).unwrap());
        let read = triangles(&file, 1e-3);
        let glb_box = bbox(&read);
        let glb_volume = volume(&read);
        println!(
            "{mesh}: triangles glb {} stl {count}; volume glb {glb_volume} stl {stl_volume}; box glb {glb_box:?} stl {stl_box:?}; {} bytes glb, {} stl",
            read.len(),
            out["bytes"],
            stl_out["bytes"]
        );
        assert_eq!(read.len(), count, "{out}");
        assert_eq!(out["triangles"], stl_out["triangles"]);
        assert_eq!(out["tolerance"], stl_out["tolerance"]);
        assert!(
            glb_box
                .iter()
                .zip(&stl_box)
                .all(|(g, t)| (g - t).abs() <= 1e-6 * 50.0),
            "{glb_box:?} {stl_box:?}"
        );
        assert!(
            (glb_volume - stl_volume).abs() <= 1e-6 * stl_volume,
            "{glb_volume} {stl_volume}"
        );
        // The block exactly, the pin short of its cylinder by at most the
        // chordal tolerance over its wall.
        let tolerance = out["tolerance"].as_f64().unwrap();
        let exact = 6000.0 + PI * 25.0 * 10.0;
        let bound = 2.0 * PI * 5.0 * 10.0 * tolerance;
        assert!(
            glb_volume <= exact * (1.0 + 1e-6) && glb_volume >= exact - bound - 1e-6 * exact,
            "{glb_volume} {exact} {bound}"
        );
        for (key, want) in [
            ("bodies", json!(2)),
            ("closed", json!(2)),
            ("faceGroups", json!(9)),
            ("materials", json!(1)),
            ("extensions", json!([])),
            ("leftOut", json!([])),
            ("unit", json!("m")),
            ("documentUnit", json!("mm")),
            ("scale", json!(0.001)),
            ("upAxis", json!("+y")),
        ] {
            assert_eq!(out[key], want, "{key}: {out}");
        }
    }
    let (out, file) = export(&mut s, &dir, "model.glb", json!({"quality": "coarse"}));
    assert_eq!(out["quality"], "coarse");
    assert_eq!(
        file.generator,
        format!("3DCraft {}", env!("CARGO_PKG_VERSION"))
    );
    let bodies = inspect_bodies(&mut s);
    for (node, (name, id)) in file.nodes.iter().zip([("Block", "F1"), ("Pin", "F2")]) {
        assert_eq!(node.name, name);
        assert_eq!(node.mesh.as_deref(), Some(name));
        assert_eq!(node.extras.keys(), ["feature"]);
        assert_eq!(text(&node.extras, "feature"), id);
        assert!(!node.transformed);
        let listed: Vec<String> = bodies.iter().find(|b| b["name"] == name).unwrap()["faceNames"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_str().unwrap().to_string())
            .collect();
        let mut held = Vec::new();
        for p in &node.primitives {
            let face = text(&p.extras, "face");
            for f in p.extras.get("faces").and_then(Json::as_array).unwrap() {
                let f = f.as_u64().unwrap() as usize;
                assert_eq!(listed[f], face);
                held.push(f);
            }
        }
        held.sort_unstable();
        assert_eq!(held, (0..listed.len()).collect::<Vec<_>>(), "{name}");
        // Every material the pictures' grey, no metal, roughness 0.5.
        for p in &node.primitives {
            let m = &file.materials[p.material.unwrap()];
            assert_eq!(m.base_color, linear(GREY));
            assert_eq!((m.metallic, m.roughness, m.double_sided), (0.0, 0.5, false));
        }
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A red plate with a blue top drilled by a yellow drill (the hole's wall
/// is the plate's red), a green metal pin with a clear coat, a glass block
/// that is not thin-walled, a brushed block with subsurface scattering and
/// a block without a colour: every primitive in its face's look; the
/// clear coat, transmission and index of refraction in their extensions;
/// anisotropy, subsurface and the glass's volume reported as left out.
#[test]
fn every_face_has_its_look_and_what_glb_cannot_hold_is_said() {
    let dir = workdir("gltf-export", "looks");
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [40, 30, 10], "name": "Plate"}),
    );
    run(
        &mut s,
        "body.appearance",
        json!({"body": "Plate", "color": RED}),
    );
    run(
        &mut s,
        "face.appearance",
        json!({"faces": ["F1:face(1)"], "color": BLUE}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"position": [20, 15, -5], "radius": 5, "height": 20, "name": "Drill"}),
    );
    run(
        &mut s,
        "body.appearance",
        json!({"body": "Drill", "color": YELLOW}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Plate", "tool": "Drill", "name": "Holed"}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"position": [100, 0, 0], "radius": 4, "height": 12, "name": "Pin"}),
    );
    run(
        &mut s,
        "body.appearance",
        json!({"body": "Pin", "color": GREEN, "metallic": 0.9, "roughness": 0.25,
               "clearcoat": 0.8, "clearcoatRoughness": 0.2}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"size": [5, 5, 5], "position": [150, 0, 0], "name": "Glass"}),
    );
    run(
        &mut s,
        "body.appearance",
        json!({"body": "Glass", "color": [230, 240, 250], "roughness": 0.05,
               "transmission": 0.9, "ior": 1.5}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"size": [5, 5, 5], "position": [200, 0, 0], "name": "Brushed"}),
    );
    run(
        &mut s,
        "body.appearance",
        json!({"body": "Brushed", "color": [180, 180, 185], "metallic": 1,
               "anisotropy": 0.6, "subsurface": 0.3}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"size": [5, 5, 5], "position": [250, 0, 0], "name": "Plain"}),
    );
    let (out, file) = export(&mut s, &dir, "parts.glb", json!({}));
    assert_eq!(
        out["leftOut"],
        json!([
            {"layer": "anisotropy", "bodies": ["Brushed"]},
            {"layer": "subsurface", "bodies": ["Brushed"]},
            {"layer": "volume", "bodies": ["Glass"]},
        ]),
        "{out}"
    );
    let used = [
        "KHR_materials_clearcoat",
        "KHR_materials_ior",
        "KHR_materials_transmission",
    ];
    assert_eq!(out["extensions"], json!(used));
    assert_eq!(file.extensions_used, used);
    assert!(file.extensions_required.is_empty());
    // Red, blue, green, glass, brushed and grey: the drill's yellow is
    // nowhere, its wall being the plate's.
    assert_eq!(out["materials"], 6, "{out}");
    let names: Vec<&str> = file.nodes.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(names, ["Holed", "Pin", "Glass", "Brushed", "Plain"]);
    let material = |p: &GltfPrimitive| &file.materials[p.material.unwrap()];
    let top = f64::from(0.01_f32);
    let mut groups = Vec::new();
    for p in &file.nodes[0].primitives {
        let on_top = p.positions.iter().all(|q| q[1] == top);
        let want = if on_top { BLUE } else { RED };
        assert_eq!(material(p).base_color, linear(want), "{:?}", p.extras);
        groups.push((text(&p.extras, "face").to_string(), on_top));
    }
    // The plate's top is one face, its name the boolean's.
    println!("Holed's face groups (name, on top): {groups:?}");
    assert_eq!(groups.iter().filter(|(_, on_top)| *on_top).count(), 1);
    let pin = material(&file.nodes[1].primitives[0]);
    assert_eq!(pin.base_color, linear(GREEN));
    assert_eq!((pin.metallic, pin.roughness), (0.9, 0.25));
    let coat = "KHR_materials_clearcoat";
    assert_eq!(pin.extensions.keys(), [coat]);
    assert_eq!(number(&pin.extensions, &[coat, "clearcoatFactor"]), 0.8);
    assert_eq!(
        number(&pin.extensions, &[coat, "clearcoatRoughnessFactor"]),
        0.2
    );
    let glass = &material(&file.nodes[2].primitives[0]).extensions;
    assert_eq!(glass.keys(), [used[1], used[2]]);
    assert_eq!(number(glass, &[used[1], "ior"]), 1.5);
    assert_eq!(number(glass, &[used[2], "transmissionFactor"]), 0.9);
    let plain = material(&file.nodes[4].primitives[0]);
    assert_eq!(plain.base_color, linear(GREY));
    assert_eq!((plain.metallic, plain.roughness), (0.0, 0.5));
    // Every primitive of a body one look: all of each body's are.
    for node in &file.nodes[1..] {
        assert!(
            node.primitives
                .iter()
                .all(|p| p.material == node.primitives[0].material)
        );
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// glTF is metres with +Y up whatever the document's unit: a block of
/// 1 × 2 × 3 units stands from (0, 0, −2) to (1, 3, 0) units in metres,
/// its top facing +Y.
#[test]
fn metres_and_y_up_in_every_unit() {
    let dir = workdir("gltf-export", "units");
    for (unit, metres) in [
        ("um", 1e-6),
        ("mm", 1e-3),
        ("cm", 1e-2),
        ("m", 1.0),
        ("in", 0.0254),
        ("ft", 0.3048),
    ] {
        let mut s = session_in(&dir);
        run(&mut s, "file.new", json!({"units": unit}));
        run(
            &mut s,
            "feature.block",
            json!({"size": [1, 2, 3], "name": "B"}),
        );
        let (out, file) = export(&mut s, &dir, "b.glb", json!({}));
        assert_eq!(out["documentUnit"], unit);
        assert_eq!(out["scale"], json!(metres));
        let f = |v: f64| f64::from(v as f32);
        let mut min = [f64::INFINITY; 3];
        let mut max = [f64::NEG_INFINITY; 3];
        for p in &file.nodes[0].primitives {
            for (lo, v) in min.iter_mut().zip(p.min) {
                *lo = lo.min(v);
            }
            for (hi, v) in max.iter_mut().zip(p.max) {
                *hi = hi.max(v);
            }
        }
        assert_eq!(min, [0.0, 0.0, f(-(2.0 * metres))], "{unit}");
        assert_eq!(max, [f(metres), f(3.0 * metres), 0.0], "{unit}");
        let top = file.nodes[0]
            .primitives
            .iter()
            .find(|p| p.positions.iter().all(|q| q[1] == f(3.0 * metres)))
            .unwrap();
        assert!(top.normals.iter().all(|n| *n == [0.0, 1.0, 0.0]), "{unit}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A name with any characters, the format by `format`, hidden bodies left
/// out, and every refusal with its reason.
#[test]
fn names_formats_and_refusals() {
    let dir = workdir("gltf-export", "refusals");
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    let refused = error(&mut s, "file.export", json!({"path": "empty.glb"}));
    assert!(refused.contains("there are no bodies"), "{refused}");
    let name = "Grüße \"A\" \\ ✓ 😀";
    run(
        &mut s,
        "feature.block",
        json!({"size": [2, 2, 2], "name": name}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"size": [1, 1, 1], "position": [5, 0, 0], "name": "Gauge"}),
    );
    run(
        &mut s,
        "feature.visible",
        json!({"feature": "Gauge", "visible": false}),
    );
    // The format by its name, whatever the extension.
    let (out, file) = export(&mut s, &dir, "model.bin", json!({"format": "glb"}));
    assert_eq!(file.nodes.len(), 1);
    assert_eq!(file.nodes[0].name, name);
    assert_eq!(
        out["hiddenLeftOut"],
        json!({"count": 1, "bodies": ["Gauge"]})
    );
    let (_, file) = export(&mut s, &dir, "MODEL.GLB", json!({"bodies": ["Gauge"]}));
    assert_eq!(file.nodes[0].name, "Gauge");
    for (params, why) in [
        (json!({"path": "x.gltf"}), ".glb"),
        (json!({"path": "x.glb", "format": "gltf"}), ".glb"),
        (json!({"path": "x.glb", "sketch": "S"}), "is for a drawing"),
        (json!({"path": "x.glb", "view": "iso"}), "is for a drawing"),
        (
            json!({"path": "x.glb", "quality": "bogus"}),
            "unknown quality",
        ),
        (json!({"path": "x.glb", "angle": 200}), "angle"),
        (json!({"path": "x.glb", "tolerance": -1}), "tolerance"),
        (json!({"path": "x.glb", "bodies": ["Nope"]}), "Nope"),
        (json!({"path": "x.glb", "format": "glbx"}), "glb"),
        // A write that fails: no such folder.
        (json!({"path": "no-such-folder/x.glb"}), "x.glb"),
    ] {
        let message = error(&mut s, "file.export", params.clone());
        assert!(message.contains(why), "{params}: {message}");
    }
    assert!(!dir.join("x.glb").exists() && !dir.join("x.gltf").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Real models: three of the guide's examples (a bracket, a gear and a
/// case) exported at the default quality, read back with every rule of the
/// reader, and against the binary STL of the very same mesh (its tolerance
/// and angle given): the same triangles and volume. Their sizes are
/// printed for the record.
#[test]
fn the_guides_examples_export_and_read_back() {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/examples");
    for name in ["bracket", "gear", "case"] {
        let dir = workdir("gltf-export", name);
        let mut s = session_in(&dir);
        s.execute("file.new", &json!({"units": "mm"})).unwrap();
        let script: Value = serde_json::from_str(
            &std::fs::read_to_string(examples.join(format!("{name}.json"))).unwrap(),
        )
        .unwrap();
        for step in script.as_array().unwrap() {
            let id = step[0].as_str().unwrap();
            let params = step.get(1).cloned().unwrap_or_else(|| json!({}));
            s.execute(id, &params)
                .unwrap_or_else(|e| panic!("{name}: {id}: {e}"));
        }
        let (out, file) = export(&mut s, &dir, "model.glb", json!({}));
        let stl = run(
            &mut s,
            "file.export",
            json!({"path": "model.stl", "tolerance": out["tolerance"], "angle": out["angle"]}),
        );
        let (count, _, stl_volume) = stl_numbers(&std::fs::read(dir.join("model.stl")).unwrap());
        let read = triangles(&file, 1e-3);
        assert_eq!(read.len(), count, "{name}");
        assert!(
            (volume(&read) - stl_volume).abs() <= 1e-6 * stl_volume.abs(),
            "{name}: {} {stl_volume}",
            volume(&read)
        );
        println!(
            "{name}: glb {} bytes, {} triangles, {} bodies, {} face groups, {} materials; stl {} bytes",
            out["bytes"],
            out["triangles"],
            out["bodies"],
            out["faceGroups"],
            out["materials"],
            stl["bytes"]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
