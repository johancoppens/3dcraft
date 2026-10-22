//! The colours of bodies and faces in exported files, read back by
//! readers written from the specifications independently of the writers
//! (`craft3d_testkit::readers`): 3MF base materials, STEP styled items and
//! OBJ with its material library. Every triangle of a 3MF or OBJ file and
//! every face of a STEP file is checked against the colour the document
//! gives it, which is told from where it lies.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::run;
use common::{error, session_in, workdir};
use craft3d_testkit::readers::{
    read_3mf_colors, read_mtl, read_obj, read_step_colours, unit_to_byte,
};
use serde_json::json;
use std::collections::BTreeMap;

const RED: [u8; 3] = [200, 16, 46];
const BLUE: [u8; 3] = [30, 90, 220];
const YELLOW: [u8; 3] = [240, 200, 20];
const GREEN: [u8; 3] = [20, 160, 70];
/// What the pictures draw a body without a colour in.
const GREY: [u8; 3] = [198, 204, 214];

/// A red plate with a blue top, drilled by a yellow drill (the hole's wall
/// is the plate's red), a green metal pin and a block without a colour.
fn model(dir: &std::path::Path) -> craft3d_engine::Session {
    let mut s = session_in(dir);
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
        json!({"body": "Pin", "color": GREEN, "metallic": 0.9, "roughness": 0.25}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"size": [5, 5, 5], "position": [200, 0, 0], "name": "Plain"}),
    );
    s
}

/// The colour the document gives the face a triangle of `body` lies on,
/// told from its corners: the plate's top (all at z = 10) is blue, the
/// rest of the plate red, the pin green, the plain block none.
fn expected(body: &str, corners: [[f64; 3]; 3]) -> Option<[u8; 3]> {
    match body {
        "Holed" if corners.iter().all(|c| (c[2] - 10.0).abs() < 1e-9) => Some(BLUE),
        "Holed" => Some(RED),
        "Pin" => Some(GREEN),
        "Plain" => None,
        other => panic!("an object `{other}` that was not exported"),
    }
}

#[test]
fn the_document_colours_its_faces_as_the_readers_expect() {
    // The scene, which the pictures draw: the plate's top blue and every
    // other face of it red (the hole's wall too), the pin green.
    let dir = workdir("colour-export", "scene");
    let mut s = model(&dir);
    let scene = s.scene(None).unwrap().unwrap();
    let mut seen: BTreeMap<String, Vec<Option<[u8; 3]>>> = BTreeMap::new();
    for body in scene.bodies.iter() {
        let colours = (0..body.face_names.len())
            .map(|k| scene.face_colors.get(body.first_face_id + k as u32))
            .collect();
        seen.insert(body.name.clone(), colours);
    }
    let holed = &seen["Holed"];
    assert_eq!(holed.iter().filter(|c| **c == Some(BLUE)).count(), 1);
    assert!(holed.iter().all(|c| *c == Some(BLUE) || *c == Some(RED)));
    assert!(seen["Pin"].iter().all(|c| *c == Some(GREEN)));
    assert!(seen["Plain"].iter().all(Option::is_none));
    // The pin's finish reaches the pictures.
    let pin = scene.bodies.iter().find(|b| b.name == "Pin").unwrap();
    let finish = scene.face_colors.finish(pin.first_face_id);
    assert_eq!(
        finish,
        craft3d_render::Finish::from_factors(Some(0.9), Some(0.25))
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn every_triangle_of_a_3mf_export_has_its_faces_colour() {
    let dir = workdir("colour-export", "3mf");
    let mut s = model(&dir);
    run(&mut s, "file.export", json!({"path": "parts.3mf"}));
    let objects = read_3mf_colors(&std::fs::read(dir.join("parts.3mf")).unwrap()).unwrap();
    let names: Vec<&str> = objects.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(names, ["Holed", "Pin", "Plain"]);
    let mut counts: BTreeMap<(String, Option<[u8; 3]>), usize> = BTreeMap::new();
    for object in &objects {
        assert_eq!(object.colors.len(), object.triangles.len());
        for (triangle, colour) in object.triangles.iter().zip(&object.colors) {
            let corners = triangle.map(|k| object.vertices[k]);
            assert_eq!(
                *colour,
                expected(&object.name, corners),
                "{} {corners:?}",
                object.name
            );
            *counts.entry((object.name.clone(), *colour)).or_default() += 1;
        }
    }
    // Each colour is there: the blue top, the red rest, the green pin.
    assert!(counts[&("Holed".into(), Some(BLUE))] >= 4, "{counts:?}");
    assert!(counts[&("Holed".into(), Some(RED))] >= 12, "{counts:?}");
    assert!(counts[&("Pin".into(), Some(GREEN))] >= 8, "{counts:?}");
    assert_eq!(counts[&("Plain".into(), None)], 12, "{counts:?}");
    assert_eq!(objects[2].color, None);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn every_face_of_a_step_export_has_its_colour() {
    let dir = workdir("colour-export", "step");
    let mut s = model(&dir);
    run(&mut s, "file.export", json!({"path": "parts.step"}));
    let text = std::fs::read_to_string(dir.join("parts.step")).unwrap();
    let solids = read_step_colours(&text).unwrap();
    let names: Vec<&str> = solids.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Holed", "Pin", "Plain"]);
    let bytes = |c: Option<[f64; 3]>| c.map(|c| c.map(|v| unit_to_byte(v).unwrap()));
    for solid in &solids {
        assert!(!solid.faces.is_empty());
        for face in &solid.faces {
            let top = face.plane.is_some_and(|(at, normal)| {
                (at[2] - 10.0).abs() < 1e-9 && normal == [0.0, 0.0, 1.0]
            });
            let want = match solid.name.as_str() {
                "Holed" if top => Some(BLUE),
                "Holed" => Some(RED),
                "Pin" => Some(GREEN),
                _ => None,
            };
            assert_eq!(bytes(face.colour), want, "{} {face:?}", solid.name);
        }
    }
    // The plate's top is one face, and the hole's wall is a cylinder in
    // the plate's red.
    let holed = &solids[0];
    assert_eq!(
        holed
            .faces
            .iter()
            .filter(|f| bytes(f.colour) == Some(BLUE))
            .count(),
        1
    );
    assert!(
        holed
            .faces
            .iter()
            .any(|f| f.surface == "CYLINDRICAL_SURFACE" && bytes(f.colour) == Some(RED))
    );
    assert_eq!(bytes(holed.colour), Some(RED));
    assert_eq!(bytes(solids[2].colour), None);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_obj_export_writes_its_material_library_and_reads_back_with_every_colour() {
    let dir = workdir("colour-export", "obj");
    let mut s = model(&dir);
    let out = run(&mut s, "file.export", json!({"path": "parts.obj"}));
    assert_eq!(out["mtl"], "parts.mtl");
    let obj = read_obj(&std::fs::read_to_string(dir.join("parts.obj")).unwrap()).unwrap();
    assert_eq!(obj.libraries, ["parts.mtl"]);
    let library = read_mtl(&std::fs::read_to_string(dir.join("parts.mtl")).unwrap()).unwrap();
    let names: Vec<&str> = obj.objects.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(names, ["Holed", "Pin", "Plain"]);
    // The same triangles as the 3MF export's, colour for colour, except
    // that OBJ gives the body without a colour the pictures' grey (a
    // material holds until the next `usemtl`).
    run(&mut s, "file.export", json!({"path": "parts.3mf"}));
    let three = read_3mf_colors(&std::fs::read(dir.join("parts.3mf")).unwrap()).unwrap();
    for (object, other) in obj.objects.iter().zip(&three) {
        assert_eq!(
            object.triangles.len(),
            other.triangles.len(),
            "{}",
            object.name
        );
        for (triangle, material) in object.triangles.iter().zip(&object.materials) {
            let corners = triangle.map(|k| obj.vertices[k]);
            let material = &library[material.as_ref().unwrap()];
            let colour = material.diffuse.unwrap().map(|v| unit_to_byte(v).unwrap());
            let want = expected(&object.name, corners).unwrap_or(GREY);
            assert_eq!(colour, want, "{} {corners:?}", object.name);
            if object.name == "Pin" {
                assert_eq!(material.metallic, Some(0.9));
                assert_eq!(material.roughness, Some(0.25));
            } else {
                assert_eq!((material.metallic, material.roughness), (None, None));
            }
        }
    }
    // One material per appearance: red, blue, green with its finish, grey.
    assert_eq!(library.len(), 4, "{library:?}");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The files in `dir`, by name.
fn files(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn an_obj_export_never_overwrites_its_library_or_a_file_it_was_not_named_for() {
    let dir = workdir("colour-export", "obj-names");
    let mut s = model(&dir);
    // The library would be the OBJ itself: refused, nothing written,
    // whatever the case.
    for path in ["x.mtl", "X.MTL", "y.Mtl"] {
        let said = error(
            &mut s,
            "file.export",
            json!({"path": path, "format": "obj"}),
        );
        assert!(said.contains("its own material library"), "{said}");
    }
    // A name the library cannot take (white space, `#`): refused with a
    // name that can, rather than renamed onto another file's library.
    std::fs::write(dir.join("a_b.mtl"), "not ours").unwrap();
    for path in ["a b.obj", "a#b.obj", "tab\t.obj"] {
        let said = error(&mut s, "file.export", json!({"path": path}));
        assert!(said.contains("white space or `#`"), "{said}");
    }
    assert!(error(&mut s, "file.export", json!({"path": "a b.obj"})).contains("`a_b.obj`"));
    assert_eq!(files(&dir), ["a_b.mtl"]);
    assert_eq!(
        std::fs::read_to_string(dir.join("a_b.mtl")).unwrap(),
        "not ours"
    );
    // Without colours any name will do: no library is needed.
    let mut plain = session_in(&dir);
    run(&mut plain, "file.new", json!({"units": "mm"}));
    run(&mut plain, "feature.block", json!({"size": [1, 1, 1]}));
    let out = run(&mut plain, "file.export", json!({"path": "a b.obj"}));
    assert!(out["mtl"].is_null());
    assert_eq!(files(&dir), ["a b.obj", "a_b.mtl"]);
    // Two OBJ files whose names differ have libraries whose names differ.
    run(&mut s, "file.export", json!({"path": "a_b.obj"}));
    run(&mut s, "file.export", json!({"path": "a-b.obj"}));
    let library = |name: &str| std::fs::read_to_string(dir.join(name)).unwrap();
    assert!(library("a_b.obj").contains("\nmtllib a_b.mtl\n"));
    assert!(library("a-b.obj").contains("\nmtllib a-b.mtl\n"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_failed_obj_write_leaves_its_library_as_it_was() {
    let dir = workdir("colour-export", "obj-fails");
    let mut s = model(&dir);
    // The OBJ's place is a directory: refused before anything is written.
    std::fs::create_dir(dir.join("d.obj")).unwrap();
    let said = error(&mut s, "file.export", json!({"path": "d.obj"}));
    assert!(said.contains("is a directory"), "{said}");
    assert_eq!(files(&dir), ["d.obj"]);
    // An earlier library of that name stays what it was.
    std::fs::write(dir.join("d.mtl"), "earlier").unwrap();
    error(&mut s, "file.export", json!({"path": "d.obj"}));
    assert_eq!(
        std::fs::read_to_string(dir.join("d.mtl")).unwrap(),
        "earlier"
    );
    // So does a library's place that is a directory, and no OBJ is made.
    std::fs::create_dir(dir.join("e.mtl")).unwrap();
    let said = error(&mut s, "file.export", json!({"path": "e.obj"}));
    assert!(said.contains("is a directory"), "{said}");
    assert!(!dir.join("e.obj").exists());
    // In a directory that is not there: refused, nothing written.
    let said = error(&mut s, "file.export", json!({"path": "nowhere/f.obj"}));
    assert!(!said.is_empty());
    assert_eq!(files(&dir), ["d.mtl", "d.obj", "e.mtl"]);
    // The OBJ write fails after the library was written (its temporary
    // file cannot be made: a directory has that name): the library is put
    // back as it was, removed when it was not there.
    for stem in ["g", "h"] {
        let temp = format!(".{stem}.obj.{}.tmp", std::process::id());
        std::fs::create_dir(dir.join(&temp)).unwrap();
    }
    std::fs::write(dir.join("g.mtl"), "earlier").unwrap();
    let said = error(&mut s, "file.export", json!({"path": "g.obj"}));
    assert!(said.contains("temporary file"), "{said}");
    assert_eq!(
        std::fs::read_to_string(dir.join("g.mtl")).unwrap(),
        "earlier"
    );
    error(&mut s, "file.export", json!({"path": "h.obj"}));
    assert!(!dir.join("h.mtl").exists());
    assert!(!dir.join("g.obj").exists() && !dir.join("h.obj").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_obj_export_without_colours_has_no_library() {
    let dir = workdir("colour-export", "plain-obj");
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "feature.block", json!({"size": [1, 2, 3]}));
    let out = run(&mut s, "file.export", json!({"path": "plain.obj"}));
    assert!(out["mtl"].is_null(), "{out}");
    let text = std::fs::read_to_string(dir.join("plain.obj")).unwrap();
    assert!(!text.contains("mtllib") && !text.contains("usemtl"));
    assert!(!dir.join("plain.mtl").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}
