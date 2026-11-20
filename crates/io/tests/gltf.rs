//! glTF export of bodies built on the kernel, read back by a reader written
//! from the specification independently of the writer
//! (`craft3d_testkit::readers::gltf`) and held against the binary STL of the
//! same mesh (triangles, box, enclosed volume) and against closed forms;
//! and pinned byte for byte, with their SHA-256, for four small models
//! (`tests/golden/*.glb`: a block, a cylinder, two bodies with colours, a
//! face on a rational B-spline surface), the files Khronos's glTF-Validator
//! was run on (docs/file-formats.md › glTF).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

#[allow(dead_code)]
#[path = "common/bodies.rs"]
mod bodies;

use craft3d_io::gltf::{GltfBody, Primitives, node};
use craft3d_io::{MeshFormat, export_mesh};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Vec3};
use craft3d_meshio::Unit;
use craft3d_meshio::gltf::{Extra, Extras, Group, PbrMaterial, Scene, srgb_to_linear, write_glb};
use craft3d_tess::{Mesh, TessParams, tessellate_body};
use craft3d_testkit::readers::gltf::{Gltf, GltfNode, read_glb};
use craft3d_testkit::readers::json::Json;
use craft3d_topo::Body;
use std::f64::consts::PI;
use std::path::PathBuf;

const RED: [u8; 3] = [200, 16, 46];
const BLUE: [u8; 3] = [30, 90, 220];
const GREEN: [u8; 3] = [20, 160, 70];
const GLASS: [u8; 3] = [120, 180, 255];

fn params(scale: f64) -> TessParams {
    TessParams {
        chordal_tolerance: 0.05 * scale,
        max_angle: Some(20.0_f64.to_radians()),
        ..TessParams::default()
    }
}

fn names(feature: u32, body: &Body) -> Vec<String> {
    (0..body.faces.len())
        .map(|k| format!("F{feature}:face({k})"))
        .collect()
}

/// A body ready for a scene: its name, feature, mesh, face names and
/// materials.
struct Part {
    name: String,
    feature: u32,
    mesh: Mesh,
    faces: Vec<String>,
    material: u32,
    face_materials: Vec<Option<u32>>,
}

fn part(name: &str, feature: u32, body: &Body, scale: f64, material: u32) -> Part {
    Part {
        name: name.to_string(),
        feature,
        mesh: tessellate_body(body, &params(scale)).unwrap(),
        faces: names(feature, body),
        material,
        face_materials: Vec::new(),
    }
}

fn scene(parts: &[Part], materials: Vec<PbrMaterial>, unit: Unit) -> Scene {
    scene_as(parts, materials, unit, Primitives::Faces, Vec::new())
}

/// The asset's extras `file.export` writes for a document in millimetres.
fn millimetres() -> Extras {
    vec![
        ("documentUnit".into(), Extra::Text("mm".into())),
        ("scale".into(), Extra::Real(0.001)),
        (
            "documentAxes".into(),
            Extra::Texts(vec!["X".into(), "-Z".into(), "Y".into()]),
        ),
    ]
}

fn scene_as(
    parts: &[Part],
    materials: Vec<PbrMaterial>,
    unit: Unit,
    primitives: Primitives,
    asset_extras: Extras,
) -> Scene {
    Scene {
        generator: "3DCraft".to_string(),
        asset_extras,
        unit,
        nodes: parts
            .iter()
            .map(|p| {
                node(GltfBody {
                    name: &p.name,
                    mesh: &p.mesh,
                    face_names: &p.faces,
                    face_materials: &p.face_materials,
                    material: Some(p.material),
                    extras: vec![("feature".into(), Extra::Text(format!("F{}", p.feature)))],
                    primitives,
                })
                .unwrap()
            })
            .collect(),
        materials,
    }
}

fn look(name: &str, color: [u8; 3]) -> PbrMaterial {
    PbrMaterial {
        name: name.to_string(),
        color,
        metallic: 0.0,
        roughness: 0.5,
        ..PbrMaterial::default()
    }
}

/// The four pinned models.
fn fixtures() -> Vec<(&'static str, Vec<Part>, Scene)> {
    let block = primitives::block(&Frame::WORLD, 10.0, 20.0, 30.0).unwrap();
    let cylinder = primitives::cylinder(&Frame::WORLD, 5.0, 10.0).unwrap();
    let a = primitives::block(&Frame::WORLD, 10.0, 10.0, 10.0).unwrap();
    let b = primitives::block(
        &Frame::WORLD.at(Vec3::new(20.0, 0.0, 0.0)).unwrap(),
        5.0,
        5.0,
        5.0,
    )
    .unwrap();
    let round = bodies::quarter_round(&Frame::WORLD, 2.0, 3.0);
    let grey = || vec![look("#c6ccd6", [198, 204, 214])];
    let mut out = Vec::new();
    let parts = vec![part("Block", 1, &block, 1.0, 0)];
    let s = scene(&parts, grey(), Unit::Millimeter);
    out.push(("block.glb", parts, s));
    let parts = vec![part("Cylinder", 1, &cylinder, 1.0, 0)];
    let s = scene(&parts, grey(), Unit::Millimeter);
    out.push(("cylinder.glb", parts, s));
    // A red block with a blue top, and a green metal block with a clear
    // coat beside it.
    let mut first = part("Red", 1, &a, 1.0, 0);
    first.face_materials = vec![None, Some(1)];
    let parts = vec![first, part("Green", 2, &b, 1.0, 2)];
    let materials = vec![
        look("#c8102e", RED),
        look("#1e5adc", BLUE),
        PbrMaterial {
            metallic: 0.9,
            roughness: 0.25,
            clearcoat: Some((1.0, 0.1)),
            ..look("#14a046 metallic 0.9 roughness 0.25 clearcoat 1", GREEN)
        },
    ];
    let s = scene(&parts, materials, Unit::Millimeter);
    out.push(("two-bodies.glb", parts, s));
    // A face on a rational B-spline surface, of glass.
    let parts = vec![part("Round", 1, &round, 1.0, 0)];
    let glass = PbrMaterial {
        roughness: 0.1,
        transmission: Some(0.6),
        ior: Some(1.45),
        ..look("#78b4ff roughness 0.1 transmission 0.6", GLASS)
    };
    let s = scene(&parts, vec![glass], Unit::Millimeter);
    out.push(("quarter-round.glb", parts, s));
    out
}

/// The cylinder and the two bodies a primitive per look, with the asset's
/// extras `file.export` writes: two more pinned files.
fn merged_fixtures() -> Vec<(&'static str, Vec<Part>, Scene)> {
    let mut out = Vec::new();
    for (file, parts, named) in fixtures() {
        let merged = match file {
            "cylinder.glb" => "cylinder-looks.glb",
            "two-bodies.glb" => "two-bodies-looks.glb",
            _ => continue,
        };
        let s = scene_as(
            &parts,
            named.materials,
            Unit::Millimeter,
            Primitives::Looks,
            millimetres(),
        );
        out.push((merged, parts, s));
    }
    out
}

/// The SHA-256 of each pinned file.
const PINNED: [(&str, &str); 6] = [
    (
        "cylinder-looks.glb",
        "dfb76a92827a64d51e5e0cacc25f8cc5c37e67bf5e5695c457ea2225c44c8846",
    ),
    (
        "two-bodies-looks.glb",
        "395734807d982fc0df40d92442380a400e6c1a9c7d3fe16175b74160473c7eae",
    ),
    (
        "block.glb",
        "50523d5bf4200c6404e6c9c5124fcf78e73b8ead11e724cd814e4776d02eb97c",
    ),
    (
        "cylinder.glb",
        "77d755078b6b44fc0b4754a4649cead3c3be99137bbef671045419bd247d5a83",
    ),
    (
        "two-bodies.glb",
        "7dcd155541b935e73b0bb619a0d425c00f88409fa9c490aa05225d1ca7e5509d",
    ),
    (
        "quarter-round.glb",
        "0f99db763e0cff8d179759e0810fc687bdef154337b8d11f8f1d212182fc06d4",
    ),
];

/// The string member `key` of an object of `extras`.
fn text<'a>(json: &'a Json, key: &str) -> &'a str {
    json.get(key).and_then(Json::as_str).unwrap()
}

/// The list of indices at `key`.
fn indices(json: &Json, key: &str) -> Vec<usize> {
    json.get(key)
        .and_then(Json::as_array)
        .unwrap()
        .iter()
        .map(|f| f.as_u64().unwrap() as usize)
        .collect()
}

/// The number at a path of keys.
fn number(json: &Json, path: &[&str]) -> f64 {
    json.at(path).and_then(Json::as_f64).unwrap()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// RFC 4648 base64, for a file to be taken from a test's log.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0_u32, |n, (i, &b)| n | (u32::from(b) << (16 - 8 * i)));
        for k in 0..4 {
            if k <= chunk.len() {
                out.push(ALPHABET[((n >> (18 - 6 * k)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The exact bytes of six small files (four models, two of them also a
/// primitive per look), so that a change of the writer or of the meshes
/// shows up in review. Set `CRAFT3D_UPDATE_GOLDEN=1` to rewrite them after
/// a deliberate change (and the SHA-256 in [`PINNED`]); a failure says what
/// was written, with the bytes in base64 for a file of up to 256 KiB, so
/// the new file can be taken from the log of a run on another machine.
#[test]
fn golden_bytes_of_six_small_files() {
    let mut failures = Vec::new();
    for (file, _, scene) in fixtures().into_iter().chain(merged_fixtures()) {
        let bytes = write_glb(&scene).unwrap();
        assert_eq!(
            write_glb(&scene).unwrap(),
            bytes,
            "{file}: not deterministic"
        );
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/golden")
            .join(file);
        if std::env::var_os("CRAFT3D_UPDATE_GOLDEN").is_some() {
            std::fs::write(&path, &bytes).unwrap();
        }
        let sha = hex(&craft3d_doc::sha256(&bytes));
        let pinned = PINNED.iter().find(|(f, _)| *f == file).unwrap().1;
        let expected = std::fs::read(&path).ok();
        if expected.as_deref() != Some(bytes.as_slice()) || sha != pinned {
            let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(file);
            std::fs::write(&out, &bytes).unwrap();
            let dump = if bytes.len() <= 256 * 1024 {
                format!("\nGOLDEN {file} {sha} {}", base64(&bytes))
            } else {
                String::new()
            };
            failures.push(format!(
                "{file} differs from tests/golden/{file} (SHA-256 pinned {pinned}): written {} bytes, SHA-256 {sha}, in {}{dump}",
                bytes.len(),
                out.display()
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Triangles, box (min x, y, z, max x, y, z) and enclosed volume of a
/// binary STL file, read by its layout: an 80-byte header, the count, 50
/// bytes per triangle.
fn stl_numbers(bytes: &[u8]) -> (usize, [f64; 6], f64) {
    let count = u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize;
    assert_eq!(bytes.len(), 84 + 50 * count);
    let mut triangles = Vec::new();
    for record in bytes[84..].chunks(50) {
        let f = |i: usize| f64::from(f32::from_le_bytes(record[i..i + 4].try_into().unwrap()));
        let v = |k: usize| [f(k), f(k + 4), f(k + 8)];
        triangles.push([v(12), v(24), v(36)]);
    }
    (count, bbox(&triangles), volume(&triangles))
}

fn bbox(triangles: &[[[f64; 3]; 3]]) -> [f64; 6] {
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for p in triangles.iter().flatten() {
        for (axis, c) in p.iter().enumerate() {
            b[axis] = b[axis].min(*c);
            b[axis + 3] = b[axis + 3].max(*c);
        }
    }
    b
}

/// The signed volume by the divergence theorem: positive for triangles
/// that face outward.
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

/// A glTF point (metres, +Y up) in the model's unit and axes: the writer's
/// turn undone, `(X, Y, Z)` to `(X, −Z, Y)`.
fn model_point(p: [f64; 3], metres: f64) -> [f64; 3] {
    [p[0] / metres, -p[2] / metres, p[1] / metres]
}

/// The triangles of a node, in the model's unit and axes.
fn node_triangles(node: &GltfNode, metres: f64) -> Vec<[[f64; 3]; 3]> {
    node.primitives
        .iter()
        .flat_map(|p| {
            p.triangles
                .iter()
                .map(|t| t.map(|k| model_point(p.positions[k], metres)))
        })
        .collect()
}

fn close(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance
}

/// Read back, every fixture agrees with the binary STL of its own mesh:
/// the same triangles, box and volume, to the 32-bit rounding of each
/// file; and with its closed form within the tessellation's bound.
#[test]
fn read_back_agrees_with_stl_and_the_closed_forms() {
    for (file, parts, scene) in fixtures() {
        let read = read_glb(&write_glb(&scene).unwrap()).unwrap();
        assert_eq!(read.generator, "3DCraft");
        assert_eq!(read.nodes.len(), parts.len(), "{file}");
        for (node, part) in read.nodes.iter().zip(&parts) {
            assert_eq!(node.name, part.name);
            assert_eq!(node.mesh.as_deref(), Some(part.name.as_str()));
            assert!(!node.transformed && node.children == 0);
            assert_eq!(node.extras.keys(), ["feature"]);
            assert_eq!(text(&node.extras, "feature"), format!("F{}", part.feature));
            let stl = export_mesh(
                &part.mesh,
                &part.name,
                Unit::Millimeter,
                MeshFormat::StlBinary,
            )
            .unwrap();
            let (count, stl_box, stl_volume) = stl_numbers(&stl);
            let triangles = node_triangles(node, 1e-3);
            let glb_box = bbox(&triangles);
            let glb_volume = volume(&triangles);
            let size = (0..3)
                .map(|k| stl_box[k + 3] - stl_box[k])
                .fold(0.0, f64::max);
            println!(
                "{file} {}: triangles glb {} stl {count}; volume glb {glb_volume} stl {stl_volume}; box glb {glb_box:?} stl {stl_box:?}",
                part.name,
                triangles.len()
            );
            assert_eq!(triangles.len(), count, "{file}");
            assert!(
                glb_box
                    .iter()
                    .zip(&stl_box)
                    .all(|(g, s)| close(*g, *s, 1e-6 * size)),
                "{file}: {glb_box:?} {stl_box:?}"
            );
            assert!(
                close(glb_volume, stl_volume, 1e-6 * stl_volume.abs()),
                "{file}: {glb_volume} {stl_volume}"
            );
            // Every primitive one name of faces, with the faces it holds.
            for p in &node.primitives {
                let face = text(&p.extras, "face");
                let faces = indices(&p.extras, "faces");
                assert!(
                    faces.iter().all(|&f| part.faces[f] == face),
                    "{file}: {face} {faces:?}"
                );
            }
            let mut held: Vec<usize> = node
                .primitives
                .iter()
                .flat_map(|p| indices(&p.extras, "faces"))
                .collect();
            held.sort_unstable();
            assert_eq!(held, (0..part.faces.len()).collect::<Vec<_>>(), "{file}");
        }
        let total: f64 = read
            .nodes
            .iter()
            .map(|n| volume(&node_triangles(n, 1e-3)))
            .sum();
        let (exact, bound) = match file {
            "block.glb" => (6000.0, 0.0),
            // The mesh is inscribed: short of the solid by at most the
            // chordal tolerance over the curved wall's area.
            "cylinder.glb" => (PI * 25.0 * 10.0, 2.0 * PI * 5.0 * 10.0 * 0.05),
            "two-bodies.glb" => (1125.0, 0.0),
            "quarter-round.glb" => (PI * 4.0 * 3.0 / 4.0, PI * 2.0 / 2.0 * 3.0 * 0.05),
            other => panic!("{other}"),
        };
        println!("{file}: volume {total}, closed form {exact}, bound {bound}");
        let rounding = 1e-6 * exact;
        assert!(
            total <= exact + rounding && total >= exact - bound - rounding,
            "{file}: {total} {exact}"
        );
    }
}

/// glTF is metres with +Y up: the block from (0, 0, 0) to (10, 20, 30) mm
/// stands from (0, 0, −0.02) to (0.01, 0.03, 0) m, its top (+Z, face 1 of
/// the kernel's block) faces +Y, and its front (−Y) +Z; the normals are the
/// faces' own, so its edges stay sharp, and the cylinder's wall shades
/// smooth, with the radial normal at every vertex.
#[test]
fn metres_y_up_and_the_normals_of_the_surfaces() {
    let all = fixtures();
    let (_, block_parts, block) = &all[0];
    let read = read_glb(&write_glb(block).unwrap()).unwrap();
    let node = &read.nodes[0];
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for p in &node.primitives {
        for (lo, v) in min.iter_mut().zip(p.min) {
            *lo = lo.min(v);
        }
        for (hi, v) in max.iter_mut().zip(p.max) {
            *hi = hi.max(v);
        }
    }
    let f = |v: f64| f64::from(v as f32);
    assert_eq!(min, [0.0, 0.0, f(-0.02)]);
    assert_eq!(max, [f(0.01), f(0.03), 0.0]);
    for p in &node.primitives {
        // Every normal of a face is its plane's: one per face.
        let first = p.normals[0];
        assert!(p.normals.iter().all(|n| *n == first));
        assert_eq!(first.iter().filter(|c| c.abs() == 1.0).count(), 1);
        // And it faces out of the box.
        let centre: Vec<f64> = (0..3)
            .map(|k| p.positions.iter().map(|q| q[k]).sum::<f64>() / p.positions.len() as f64)
            .collect();
        let middle = [0.005, 0.015, -0.01];
        let out: f64 = (0..3).map(|k| (centre[k] - middle[k]) * first[k]).sum();
        assert!(out > 0.0, "{first:?} at {centre:?}");
    }
    let top = node
        .primitives
        .iter()
        .find(|p| p.positions.iter().all(|q| q[1] == f(0.03)))
        .unwrap();
    assert_eq!(top.normals[0], [0.0, 1.0, 0.0]);
    let face = text(&top.extras, "face");
    let index = block_parts[0].faces.iter().position(|n| n == face).unwrap();
    let kernel_top = block_parts[0]
        .mesh
        .face_ids
        .iter()
        .position(|&f| f as usize == index)
        .unwrap();
    let corner = block_parts[0].mesh.triangles[kernel_top][0];
    assert_eq!(block_parts[0].mesh.positions[corner as usize].z, 30.0);
    let front = node
        .primitives
        .iter()
        .find(|p| p.positions.iter().all(|q| q[2] == 0.0))
        .unwrap();
    assert_eq!(front.normals[0], [0.0, 0.0, 1.0]);

    // The cylinder's wall: radial normals, in glTF's axes.
    let (_, _, cylinder) = &all[1];
    let read = read_glb(&write_glb(cylinder).unwrap()).unwrap();
    let mut walls = 0;
    for p in &read.nodes[0].primitives {
        if p.normals.iter().all(|n| n[1].abs() == 1.0) {
            continue; // a cap
        }
        walls += 1;
        let mut directions = std::collections::BTreeSet::new();
        for (q, n) in p.positions.iter().zip(&p.normals) {
            let [x, y, _] = model_point(*q, 1e-3);
            let r = (x * x + y * y).sqrt();
            let want = [x / r, 0.0, -y / r];
            assert!(
                n.iter().zip(want).all(|(a, b)| close(*a, b, 1e-5)),
                "{n:?} {want:?}"
            );
            directions.insert(((n[0] * 1e4).round() as i64, (n[2] * 1e4).round() as i64));
        }
        // A normal per vertex, not one per triangle: many directions.
        assert!(directions.len() > 8, "smooth: {}", directions.len());
    }
    assert_eq!(walls, 1);
}

/// The colours and finishes of the two bodies: each primitive's material
/// is its face's look, the base colour linear, the clear coat in its
/// extension; glass carries its transmission and index of refraction.
#[test]
fn materials_follow_the_faces_and_carry_their_extensions() {
    let all = fixtures();
    let (_, _, two) = &all[2];
    let read = read_glb(&write_glb(two).unwrap()).unwrap();
    assert_eq!(read.extensions_used, ["KHR_materials_clearcoat"]);
    assert!(read.extensions_required.is_empty());
    let linear = |c: [u8; 3]| {
        let [r, g, b] = c.map(srgb_to_linear);
        [r, g, b, 1.0]
    };
    let colour_of = |p: &craft3d_testkit::readers::gltf::GltfPrimitive| {
        read.materials[p.material.unwrap()].base_color
    };
    let red = &read.nodes[0];
    for p in &red.primitives {
        let want = if text(&p.extras, "face") == "F1:face(1)" {
            BLUE
        } else {
            RED
        };
        assert_eq!(colour_of(p), linear(want), "{:?}", p.extras);
    }
    let green = &read.materials[read.nodes[1].primitives[0].material.unwrap()];
    assert_eq!(green.base_color, linear(GREEN));
    assert_eq!((green.metallic, green.roughness), (0.9, 0.25));
    let coat = "KHR_materials_clearcoat";
    assert_eq!(green.extensions.keys(), [coat]);
    assert_eq!(number(&green.extensions, &[coat, "clearcoatFactor"]), 1.0);
    assert_eq!(
        number(&green.extensions, &[coat, "clearcoatRoughnessFactor"]),
        0.1
    );
    assert!(!green.double_sided);
    let (_, _, round) = &all[3];
    let read = read_glb(&write_glb(round).unwrap()).unwrap();
    let used = ["KHR_materials_ior", "KHR_materials_transmission"];
    assert_eq!(read.extensions_used, used);
    let glass = &read.materials[0].extensions;
    assert_eq!(glass.keys(), used);
    assert_eq!(number(glass, &[used[0], "ior"]), 1.45);
    assert_eq!(number(glass, &[used[1], "transmissionFactor"]), 0.6);
}

/// Names reach the reader as they were, whatever characters they hold
/// (JSON escapes control characters, quotes and backslashes; the rest is
/// UTF-8); a body at the scales of rule 3 reads back with its volume; and
/// the unit is converted to metres.
#[test]
fn any_name_any_scale_and_every_unit() {
    let block = primitives::block(&Frame::WORLD, 1.0, 2.0, 3.0).unwrap();
    let name = "a\u{0}\u{1}\u{1f}\t\n\"q\" \\ é ✓ 😀 \u{7f}\u{2028}";
    let mut p = part(name, 7, &block, 1.0, 0);
    p.faces[0] = format!("{name}:face");
    let s = scene(&[p], vec![look(name, [1, 2, 3])], Unit::Millimeter);
    let read = read_glb(&write_glb(&s).unwrap()).unwrap();
    assert_eq!(read.nodes[0].name, name);
    assert_eq!(read.nodes[0].mesh.as_deref(), Some(name));
    assert_eq!(read.materials[0].name, name);
    assert_eq!(
        text(&read.nodes[0].primitives[0].extras, "face"),
        format!("{name}:face")
    );
    for scale in [1e-3, 1.0, 1e3, 1e6] {
        let body = primitives::block(&Frame::WORLD, scale, 2.0 * scale, 3.0 * scale).unwrap();
        let s = scene(
            &[part("B", 1, &body, scale, 0)],
            vec![look("g", [9, 9, 9])],
            Unit::Millimeter,
        );
        let read = read_glb(&write_glb(&s).unwrap()).unwrap();
        let v = volume(&node_triangles(&read.nodes[0], 1e-3));
        let exact = 6.0 * scale * scale * scale;
        assert!(close(v, exact, 1e-6 * exact), "{scale}: {v} {exact}");
    }
    for (unit, metres) in [
        (Unit::Micron, 1e-6),
        (Unit::Millimeter, 1e-3),
        (Unit::Centimeter, 1e-2),
        (Unit::Meter, 1.0),
        (Unit::Inch, 0.0254),
        (Unit::Foot, 0.3048),
    ] {
        let s = scene(
            &[part("B", 1, &block, 1.0, 0)],
            vec![look("g", [9, 9, 9])],
            unit,
        );
        let read = read_glb(&write_glb(&s).unwrap()).unwrap();
        let top = read.nodes[0]
            .primitives
            .iter()
            .map(|p| p.max[1])
            .fold(f64::MIN, f64::max);
        assert_eq!(top, f64::from((3.0 * metres) as f32), "{unit:?}");
    }
}

/// A group of `n` vertices in millimetres, every one on a triangle: the
/// triangles `(3i, 3i + 1, 3i + 2)` of slivers along x, and one more over
/// the last three vertices when `n` is not a multiple of 3.
fn slivers(n: u32) -> Group {
    let positions = (0..n)
        .map(|k| match k % 3 {
            0 => [f64::from(k / 3), 0.0, 0.0],
            1 => [f64::from(k / 3), 1.0, 0.0],
            _ => [f64::from(k / 3), 0.0, 1.0],
        })
        .collect();
    let mut triangles: Vec<[u32; 3]> = (0..n / 3).map(|i| [3 * i, 3 * i + 1, 3 * i + 2]).collect();
    if !n.is_multiple_of(3) {
        triangles.push([n - 1, n - 2, n - 3]);
    }
    Group {
        extras: Vec::new(),
        material: None,
        positions,
        normals: Vec::new(),
        triangles,
    }
}

/// The switch to 32-bit indices, read back by the independent reader: a
/// group of 65,536 vertices has `u32` indices (5125), one of 65,535 after
/// it `u16` (5123), and both read back with every rule of the reader (the
/// restart value never used, every accessor in its view and aligned, `min`
/// and `max` the data's, unit normals) as the triangles and points given,
/// in metres with +Y up. An empty scene and a node without triangles read
/// back too: no buffer, no mesh.
#[test]
fn indices_past_65535_vertices_read_back_as_32_bits() {
    let groups = vec![slivers(65_536), slivers(65_535)];
    let scene = Scene {
        generator: "3DCraft".to_string(),
        asset_extras: Vec::new(),
        unit: Unit::Millimeter,
        nodes: vec![
            craft3d_meshio::gltf::Node {
                name: "empty".to_string(),
                extras: Vec::new(),
                groups: vec![Group::default()],
            },
            craft3d_meshio::gltf::Node {
                name: "wide".to_string(),
                extras: Vec::new(),
                groups: groups.clone(),
            },
        ],
        materials: Vec::new(),
    };
    let read = read_glb(&write_glb(&scene).unwrap()).unwrap();
    assert_eq!(read.nodes.len(), 2);
    assert!(read.nodes[0].mesh.is_none());
    let primitives = &read.nodes[1].primitives;
    assert_eq!(
        primitives.iter().map(|p| p.index_type).collect::<Vec<_>>(),
        [5125, 5123]
    );
    for (p, group) in primitives.iter().zip(&groups) {
        assert_eq!(p.positions.len(), group.positions.len());
        let given: Vec<[usize; 3]> = group
            .triangles
            .iter()
            .map(|t| t.map(|k| k as usize))
            .collect();
        assert_eq!(p.triangles, given);
        for (q, model) in p.positions.iter().zip(&group.positions) {
            let want = [model[0], model[2], -model[1]];
            assert_eq!(*q, want.map(|c| f64::from((c * 1e-3) as f32) + 0.0));
        }
    }
    let empty = read_glb(&write_glb(&Scene::default()).unwrap()).unwrap();
    assert!(empty.nodes.is_empty() && empty.materials.is_empty());
    assert!(empty.json.get("buffers").is_none());
}

/// A sheet (an open box) is written as it is; the reader takes it, and a
/// material of an open mesh can be drawn from both sides.
#[test]
fn a_sheet_reads_back_open() {
    let sheet = bodies::open_box(2.0);
    let p = part("Tray", 1, &sheet, 1.0, 0);
    let s = scene(
        &[p],
        vec![PbrMaterial {
            double_sided: true,
            ..look("tray", [5, 6, 7])
        }],
        Unit::Millimeter,
    );
    let read: Gltf = read_glb(&write_glb(&s).unwrap()).unwrap();
    assert_eq!(read.nodes[0].primitives.len(), 5);
    assert!(read.materials[0].double_sided);
}

/// The base colour of each triangle of a node, linear, sorted.
fn triangle_colours(read: &Gltf, node: &GltfNode) -> Vec<[f64; 4]> {
    let mut colours: Vec<[f64; 4]> = node
        .primitives
        .iter()
        .flat_map(|p| {
            let colour = read.materials[p.material.unwrap()].base_color;
            std::iter::repeat_n(colour, p.triangles.len())
        })
        .collect();
    colours.sort_by(|p, q| p.partial_cmp(q).unwrap());
    colours
}

/// A primitive per look, read back against a primitive per face group of
/// the same model, for every fixture: the same triangles (as many, the
/// same corners, the same volume), the same materials, every triangle in
/// its colour; as many primitives as the body has looks, none with extras;
/// no more vertices than the named form, a block's corners three vertices
/// each (three normals: the creases stay).
#[test]
fn merged_by_look_reads_back_as_the_named_form() {
    for (file, parts, named) in fixtures() {
        let merged = scene_as(
            &parts,
            named.materials.clone(),
            Unit::Millimeter,
            Primitives::Looks,
            millimetres(),
        );
        let (named_bytes, merged_bytes) = (write_glb(&named).unwrap(), write_glb(&merged).unwrap());
        let (a, b) = (
            read_glb(&named_bytes).unwrap(),
            read_glb(&merged_bytes).unwrap(),
        );
        assert_eq!(a.materials, b.materials, "{file}");
        let mut lines = Vec::new();
        for (x, y) in a.nodes.iter().zip(&b.nodes) {
            let (tx, ty) = (node_triangles(x, 1e-3), node_triangles(y, 1e-3));
            assert_eq!(tx.len(), ty.len(), "{file}");
            assert!(
                close(volume(&tx), volume(&ty), 1e-12 * volume(&tx).abs()),
                "{file}"
            );
            let sorted = |mut t: Vec<[[f64; 3]; 3]>| {
                t.sort_by(|p, q| p.partial_cmp(q).unwrap());
                t
            };
            assert_eq!(sorted(tx), sorted(ty), "{file}: the same triangles");
            assert_eq!(
                triangle_colours(&a, x),
                triangle_colours(&b, y),
                "{file}: every triangle in its colour"
            );
            let looks: std::collections::BTreeSet<Option<usize>> =
                x.primitives.iter().map(|p| p.material).collect();
            assert_eq!(y.primitives.len(), looks.len(), "{file}");
            assert!(y.primitives.iter().all(|p| p.extras == Json::Null));
            let vertices = |n: &GltfNode| {
                n.primitives
                    .iter()
                    .map(|p| p.positions.len())
                    .sum::<usize>()
            };
            assert!(vertices(y) <= vertices(x), "{file}");
            lines.push(format!(
                "{}: {} primitives, {} vertices named; {} primitives, {} vertices merged",
                x.name,
                x.primitives.len(),
                vertices(x),
                y.primitives.len(),
                vertices(y)
            ));
            if file == "block.glb" {
                assert_eq!(vertices(y), 24, "{file}");
            }
        }
        println!(
            "{file}: named {} bytes, merged {} bytes; {}",
            named_bytes.len(),
            merged_bytes.len(),
            lines.join("; ")
        );
    }
}

/// `asset.extras` is all a reader needs to give the document's coordinates
/// back: `scale` and `documentAxes` turn the file's points into the
/// model's, which are the STL's, in `documentUnit`.
#[test]
fn the_asset_says_how_to_turn_the_file_back() {
    let (_, parts, scene) = merged_fixtures().remove(0);
    let read = read_glb(&write_glb(&scene).unwrap()).unwrap();
    let extras = read.json.at(&["asset", "extras"]).unwrap();
    assert_eq!(text(extras, "documentUnit"), "mm");
    let scale = number(extras, &["scale"]);
    let axes: Vec<&str> = extras
        .get("documentAxes")
        .and_then(Json::as_array)
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap())
        .collect();
    assert_eq!(axes, ["X", "-Z", "Y"]);
    // Each of the document's axes from the file's, by the names alone.
    let back = |p: [f64; 3]| {
        let mut model = [0.0; 3];
        for (slot, &axis) in model.iter_mut().zip(&axes) {
            let (sign, name) = match axis.strip_prefix('-') {
                Some(name) => (-1.0, name),
                None => (1.0, axis),
            };
            let at = ["X", "Y", "Z"].iter().position(|a| *a == name).unwrap();
            *slot = sign * p[at] / scale;
        }
        model
    };
    let triangles: Vec<[[f64; 3]; 3]> = read.nodes[0]
        .primitives
        .iter()
        .flat_map(|p| p.triangles.iter().map(|t| t.map(|k| back(p.positions[k]))))
        .collect();
    let stl = export_mesh(&parts[0].mesh, "c", Unit::Millimeter, MeshFormat::StlBinary).unwrap();
    let (count, stl_box, stl_volume) = stl_numbers(&stl);
    assert_eq!(triangles.len(), count);
    assert!(close(volume(&triangles), stl_volume, 1e-6 * stl_volume));
    assert!(
        bbox(&triangles)
            .iter()
            .zip(&stl_box)
            .all(|(g, s)| close(*g, *s, 1e-5))
    );
}
