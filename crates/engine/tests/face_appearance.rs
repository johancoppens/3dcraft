//! A colour per face (`face.appearance`): kept by the body's feature and
//! the face's persistent name, so it follows the history: upstream edits,
//! the pieces of a face a boolean cut in two, the instances of a pattern;
//! a face that is gone colours nothing and is said; undo and redo, save
//! and reopen, the selection, the pictures and the exports.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::run;
use common::{error, session, session_in, workdir};
use craft3d_engine::Session;
use serde_json::{Value, json};

const BLUE: [u8; 3] = [0x2f, 0x6f, 0xd6];
const GREEN: [u8; 3] = [20, 160, 60];
const RED: [u8; 3] = [200, 16, 46];

fn new_doc(s: &mut Session) {
    run(s, "file.new", json!({"units": "mm"}));
}

/// Per face of the body of `feature` in the scene: its name and colour.
fn scene_faces(s: &mut Session, feature: &str) -> Vec<(String, Option<[u8; 3]>)> {
    let scene = s.scene(None).unwrap().unwrap();
    let body = scene
        .bodies
        .iter()
        .find(|b| b.feature == feature)
        .unwrap_or_else(|| panic!("no body of {feature} in the scene"));
    body.face_names
        .iter()
        .enumerate()
        .map(|(k, name)| {
            (
                name.clone(),
                scene
                    .face_colors
                    .get(body.first_face_id + u32::try_from(k).unwrap()),
            )
        })
        .collect()
}

/// The names of the faces of `feature`'s body that are drawn in `color`.
fn coloured(s: &mut Session, feature: &str, color: [u8; 3]) -> Vec<String> {
    scene_faces(s, feature)
        .into_iter()
        .filter(|(_, c)| *c == Some(color))
        .map(|(name, _)| name)
        .collect()
}

/// A plate (F1, 40 x 30 x 10) with its top (`F1:face(1)`) in blue, and a
/// slot (F2) across it, cut (F3): the top in two pieces.
fn slotted_plate(s: &mut Session) {
    new_doc(s);
    run(
        s,
        "feature.block",
        json!({"size": [40, 30, 10], "name": "Plate"}),
    );
    run(
        s,
        "face.appearance",
        json!({"faces": ["F1:face(1)"], "color": "#2f6fd6"}),
    );
    run(
        s,
        "feature.block",
        json!({"size": [6, 40, 4], "position": [17, -5, 8], "name": "Slot"}),
    );
    run(
        s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Plate", "tool": "Slot", "name": "Cut"}),
    );
}

#[test]
fn a_face_colour_follows_the_pieces_of_a_cut_face_and_upstream_edits() {
    let mut s = session();
    slotted_plate(&mut s);
    // Both pieces of the top, and nothing else.
    let top = "F3:copy(0)/F1:face(1)";
    assert_eq!(coloured(&mut s, "F3", BLUE), [top, top]);
    // The slot's floor (the tool's bottom face, inside out), coloured in
    // the result's body.
    let floor = "F3:copy(1)/F2:face(0)";
    let out = run(
        &mut s,
        "face.appearance",
        json!({"body": "Cut", "faces": [floor], "color": GREEN}),
    );
    assert_eq!(out["faces"][0]["face"], floor);
    assert_eq!(out["faces"][0]["shown"]["color"], "#14a03c");
    assert_eq!(coloured(&mut s, "F3", GREEN), [floor]);
    // Upstream edits keep both on their faces: a bigger plate, a deeper
    // and wider slot.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Plate", "size": [60, 30, 12]}),
    );
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Slot", "size": [9, 40, 7], "position": [20, -5, 8]}),
    );
    assert_eq!(coloured(&mut s, "F3", BLUE), [top, top]);
    assert_eq!(coloured(&mut s, "F3", GREEN), [floor]);
    // A face's own colour wins over a body colour, also one given on a
    // later feature (J-040, 2026-11-10; before it, the red of `Cut` hid
    // the plate's blue top): the coloured faces keep theirs, the rest of
    // the body is red.
    run(
        &mut s,
        "body.appearance",
        json!({"body": "Cut", "color": RED}),
    );
    assert_eq!(coloured(&mut s, "F3", GREEN), [floor]);
    assert_eq!(coloured(&mut s, "F3", BLUE), [top, top]);
    let faces = scene_faces(&mut s, "F3");
    assert!(
        faces.iter().all(|(name, c)| *c
            == Some(if name == floor {
                GREEN
            } else if name == top {
                BLUE
            } else {
                RED
            })),
        "{faces:?}"
    );
    // ... and the plate's blue top stays where the red is cleared.
    run(
        &mut s,
        "body.appearance",
        json!({"body": "Cut", "clear": true}),
    );
    assert_eq!(coloured(&mut s, "F3", BLUE), [top, top]);
    let inspect = run(&mut s, "document.inspect", json!({"measures": false}));
    assert_eq!(
        inspect["faceAppearances"],
        json!([
            {"feature": "F1", "face": "F1:face(1)", "color": "#2f6fd6"},
            {"feature": "F3", "face": floor, "color": "#14a03c"},
        ])
    );
    assert!(inspect.get("lostFaces").is_none());
}

#[test]
fn a_face_that_is_gone_colours_nothing_and_is_said() {
    let dir = workdir("face-appearance", "gone");
    let mut s = session_in(&dir);
    slotted_plate(&mut s);
    let floor = "F3:copy(1)/F2:face(0)";
    run(
        &mut s,
        "face.appearance",
        json!({"body": "Cut", "faces": [floor], "color": GREEN}),
    );
    // The slot cuts through the plate now: it has no floor.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Slot", "size": [6, 40, 20], "position": [17, -5, -5]}),
    );
    assert!(coloured(&mut s, "F3", GREEN).is_empty());
    let inspect = run(&mut s, "document.inspect", json!({"measures": false}));
    let lost = inspect["lostFaces"].as_array().unwrap();
    assert_eq!(lost.len(), 1, "{inspect}");
    assert_eq!(lost[0]["face"], floor);
    assert!(
        lost[0]["notice"]
            .as_str()
            .unwrap()
            .contains("no longer has a face")
    );
    // Said in the window, as a notice: no problem of the picture
    // (expectation changed on 2026-10-26, R1 item 57: it was a problem,
    // and view.render failed on it, so no picture and no Jig thumbnail).
    let scene = s.scene(None).unwrap().unwrap();
    assert!(scene.problems.is_empty(), "{:?}", scene.problems);
    assert!(
        scene.notices.iter().any(|p| p.contains(floor)),
        "{:?}",
        scene.notices
    );
    let check = run(&mut s, "document.check", json!({}));
    assert_eq!(check["ok"], true, "{check}");
    assert_eq!(check["warnings"][0]["kind"], "lostFace", "{check}");
    assert_eq!(check["warnings"][0]["face"], floor, "{check}");
    // A picture is drawn all the same (it was refused).
    run(
        &mut s,
        "view.render",
        json!({"path": "gone.png", "width": 64, "height": 48}),
    );
    assert!(dir.join("gone.png").exists());
    // Undo brings the floor back, and its colour with it.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(coloured(&mut s, "F3", GREEN), [floor]);
    run(&mut s, "edit.redo", json!({}));
    // A lost face's colour can be cleared by its name.
    let out = run(
        &mut s,
        "face.appearance",
        json!({"body": "Cut", "faces": [floor], "clear": true}),
    );
    assert!(out.get("lostFaces").is_none(), "{out}");
    let inspect = run(&mut s, "document.inspect", json!({"measures": false}));
    assert!(inspect.get("faceAppearances").unwrap()[0]["face"] == "F1:face(1)");
    assert!(inspect.get("lostFaces").is_none());
}

#[test]
fn every_instance_of_a_pattern_has_the_seeds_face_colour_unless_given_its_own() {
    let mut s = session();
    new_doc(&mut s);
    run(
        &mut s,
        "feature.block",
        json!({"size": [10, 10, 10], "name": "Seed"}),
    );
    run(
        &mut s,
        "face.appearance",
        json!({"faces": ["F1:face(1)"], "color": BLUE}),
    );
    run(
        &mut s,
        "feature.pattern",
        json!({"source": "Seed", "count": 3, "step": [20, 0, 0], "name": "Row"}),
    );
    let mut tops = coloured(&mut s, "F2", BLUE);
    tops.sort();
    assert_eq!(
        tops,
        [
            "F2:copy(0)/F1:face(1)",
            "F2:copy(1)/F1:face(1)",
            "F2:copy(2)/F1:face(1)"
        ]
    );
    // One instance's top in a colour of its own.
    run(
        &mut s,
        "face.appearance",
        json!({"faces": ["F2:copy(1)/F1:face(1)"], "color": GREEN}),
    );
    assert_eq!(coloured(&mut s, "F2", GREEN), ["F2:copy(1)/F1:face(1)"]);
    assert_eq!(coloured(&mut s, "F2", BLUE).len(), 2);
    // More instances: the new ones take the seed's colour, the one given
    // its own keeps it.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Row", "count": 5}),
    );
    assert_eq!(coloured(&mut s, "F2", BLUE).len(), 4);
    assert_eq!(coloured(&mut s, "F2", GREEN), ["F2:copy(1)/F1:face(1)"]);
    // Moved and copied: the faces keep theirs.
    run(
        &mut s,
        "feature.transform",
        json!({"source": "Row", "translate": [0, 0, 50], "name": "Up"}),
    );
    assert_eq!(coloured(&mut s, "F3", BLUE).len(), 4);
    assert_eq!(
        coloured(&mut s, "F3", GREEN),
        ["F3:copy(0)/F2:copy(1)/F1:face(1)"]
    );
}

#[test]
fn colouring_is_one_undo_step_and_the_selection_can_say_which_faces() {
    let mut s = session();
    new_doc(&mut s);
    run(
        &mut s,
        "feature.block",
        json!({"size": [10, 10, 10], "name": "Box"}),
    );
    run(
        &mut s,
        "face.appearance",
        json!({"faces": ["F1:face(0)", "F1:face(2)"], "color": "#f00"}),
    );
    assert_eq!(coloured(&mut s, "F1", [255, 0, 0]).len(), 2);
    run(&mut s, "edit.undo", json!({}));
    assert!(coloured(&mut s, "F1", [255, 0, 0]).is_empty());
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(coloured(&mut s, "F1", [255, 0, 0]).len(), 2);
    // Select a face (and a body, which a face colour skips), then colour.
    run(
        &mut s,
        "select.set",
        json!({"items": [
            {"kind": "face", "name": "F1:face(5)", "feature": "F1"},
            {"kind": "body", "name": "Box", "feature": "F1"},
        ]}),
    );
    let out = run(
        &mut s,
        "face.appearance",
        json!({"selection": true, "color": "#2f6fd6"}),
    );
    assert_eq!(
        out["faces"],
        json!([{"feature": "F1", "face": "F1:face(5)", "shown": {"color": "#2f6fd6"}}])
    );
    assert_eq!(out["skipped"], json!(["body Box"]));
    assert_eq!(coloured(&mut s, "F1", BLUE), ["F1:face(5)"]);
    // Again with the same colour changes nothing: no undo step.
    let undo = run(&mut s, "document.inspect", json!({"measures": false}))["history"]["undo"]
        .as_array()
        .unwrap()
        .len();
    run(
        &mut s,
        "face.appearance",
        json!({"selection": true, "color": "#2f6fd6"}),
    );
    let again = run(&mut s, "document.inspect", json!({"measures": false}))["history"]["undo"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(undo, again);
    // Clear by the selection too.
    run(
        &mut s,
        "face.appearance",
        json!({"selection": true, "clear": true}),
    );
    assert!(coloured(&mut s, "F1", BLUE).is_empty());
    // Deleting the feature drops its face colours.
    run(&mut s, "feature.delete", json!({"feature": "Box"}));
    let inspect = run(&mut s, "document.inspect", json!({"measures": false}));
    assert!(inspect.get("faceAppearances").is_none(), "{inspect}");
}

#[test]
fn face_colours_persist_through_save_and_reopen() {
    let dir = workdir("face-appearance", "save");
    let mut s = session_in(&dir);
    slotted_plate(&mut s);
    run(
        &mut s,
        "face.appearance",
        json!({"body": "Cut", "faces": ["F3:copy(1)/F2:face(0)"], "color": GREEN, "roughness": 0.3}),
    );
    let before = scene_faces(&mut s, "F3");
    let listed =
        run(&mut s, "document.inspect", json!({"measures": false}))["faceAppearances"].clone();
    run(&mut s, "file.save", json!({"path": "faces.3dcraft"}));
    let first = std::fs::read(dir.join("faces.3dcraft")).unwrap();
    run(&mut s, "file.close", json!({}));
    run(&mut s, "file.open", json!({"path": "faces.3dcraft"}));
    assert_eq!(scene_faces(&mut s, "F3"), before);
    assert_eq!(
        run(&mut s, "document.inspect", json!({"measures": false}))["faceAppearances"],
        listed
    );
    run(&mut s, "file.save", json!({"path": "again.3dcraft"}));
    assert_eq!(std::fs::read(dir.join("again.3dcraft")).unwrap(), first);
}

/// The pixels of a PNG as RGB triples.
fn pixels(png_bytes: &[u8]) -> Vec<[u8; 3]> {
    let decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).unwrap();
    buf[..info.buffer_size()]
        .chunks(3)
        .map(|p| [p[0], p[1], p[2]])
        .collect()
}

fn render(s: &mut Session) -> Vec<[u8; 3]> {
    let out = run(s, "view.render", json!({"width": 160, "height": 120}));
    pixels(&decode_base64(out["base64"].as_str().unwrap()))
}

fn decode_base64(text: &str) -> Vec<u8> {
    let value = |c: u8| -> u32 {
        match c {
            b'A'..=b'Z' => u32::from(c - b'A'),
            b'a'..=b'z' => u32::from(c - b'a') + 26,
            b'0'..=b'9' => u32::from(c - b'0') + 52,
            b'+' => 62,
            b'/' => 63,
            _ => panic!("not base64: {c}"),
        }
    };
    let mut out = Vec::new();
    for chunk in text.as_bytes().chunks(4) {
        let digits: Vec<u32> = chunk
            .iter()
            .filter(|&&c| c != b'=')
            .map(|&c| value(c))
            .collect();
        let word = digits.iter().fold(0, |w, d| (w << 6) | d) << (6 * (4 - digits.len()));
        let bytes = word.to_be_bytes();
        out.extend_from_slice(&bytes[1..digits.len()]);
    }
    out
}

#[test]
fn view_render_draws_a_face_in_its_colour() {
    let mut s = session();
    new_doc(&mut s);
    run(
        &mut s,
        "feature.block",
        json!({"size": [40, 30, 10], "name": "Plate"}),
    );
    let bluish = |p: &[[u8; 3]]| {
        p.iter()
            .filter(|c| i32::from(c[2]) - i32::from(c[0]) > 80)
            .count()
    };
    let plain = render(&mut s);
    assert_eq!(bluish(&plain), 0);
    run(
        &mut s,
        "face.appearance",
        json!({"faces": ["F1:face(1)"], "color": BLUE}),
    );
    let top = render(&mut s);
    assert!(bluish(&top) > 500, "{}", bluish(&top));
    // Only the top: the pixels that changed are the blue ones, and a
    // rim of the top's outline where its edges are blended over it.
    let changed = plain.iter().zip(&top).filter(|(a, b)| a != b).count();
    assert!(
        changed >= bluish(&top) && changed < bluish(&top) * 21 / 20,
        "{changed} changed, {} blue",
        bluish(&top)
    );
}

/// The attribute `name` of an XML tag on one line.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let at = tag.find(&format!(" {name}=\""))? + name.len() + 3;
    Some(tag[at..at + tag[at..].find('"')?].to_string())
}

#[test]
fn a_3mf_and_a_step_export_carry_the_face_colours() {
    let dir = workdir("face-appearance", "export");
    let mut s = session_in(&dir);
    slotted_plate(&mut s);
    run(&mut s, "file.export", json!({"path": "faces.3mf"}));
    let bytes = std::fs::read(dir.join("faces.3mf")).unwrap();
    let text = String::from_utf8_lossy(&bytes).to_string();
    let start = text.find("<model ").unwrap();
    let xml = &text[start..text.find("</model>").unwrap()];
    let bases: Vec<String> = xml
        .lines()
        .filter(|l| l.trim_start().starts_with("<base "))
        .map(|l| attribute(l, "displaycolor").unwrap())
        .collect();
    // The body has no colour: its other faces are in the colour the
    // pictures draw it in, the object's material; the top's triangles
    // have the blue one.
    assert_eq!(bases, ["#C6CCD6", "#2F6FD6"]);
    let objects: Vec<&str> = xml
        .lines()
        .map(str::trim_start)
        .filter(|l| l.starts_with("<object "))
        .collect();
    assert_eq!(objects.len(), 1);
    assert_eq!(attribute(objects[0], "pindex").as_deref(), Some("0"));
    let triangles: Vec<&str> = xml
        .lines()
        .map(str::trim_start)
        .filter(|l| l.starts_with("<triangle "))
        .collect();
    let blue = triangles
        .iter()
        .filter(|t| attribute(t, "p1").as_deref() == Some("1"))
        .count();
    assert_eq!(
        triangles
            .iter()
            .filter(|t| attribute(t, "p1").is_some())
            .count(),
        blue
    );
    // Two pieces of a flat rectangle: at least two triangles each, and
    // far fewer than all of them.
    assert!(
        blue >= 4 && blue < triangles.len() / 2,
        "{blue} of {}",
        triangles.len()
    );
    // STEP: one styled item, on each piece of the top (the solid has no
    // colour), with one colour.
    run(&mut s, "file.export", json!({"path": "faces.step"}));
    let step = std::fs::read_to_string(dir.join("faces.step")).unwrap();
    assert_eq!(step.matches("STYLED_ITEM(").count(), 2, "{step}");
    assert_eq!(step.matches("=COLOUR_RGB(").count(), 1);
    let rgb = step
        .lines()
        .find(|l| l.contains("=COLOUR_RGB("))
        .unwrap()
        .to_string();
    let numbers: Vec<f64> = rgb[rgb.find(",").unwrap() + 1..rgb.rfind(')').unwrap()]
        .split(',')
        .map(|n| n.trim().parse().unwrap())
        .collect();
    let back: Vec<u8> = numbers.iter().map(|x| (x * 255.0).round() as u8).collect();
    assert_eq!(back, BLUE);
    // Every styled item is on an ADVANCED_FACE.
    for line in step.lines().filter(|l| l.contains("STYLED_ITEM(")) {
        let target = line[line.rfind('#').unwrap()..].trim_end_matches(");");
        let entity = step
            .lines()
            .find(|l| l.starts_with(&format!("{target}=")))
            .unwrap();
        assert!(entity.contains("ADVANCED_FACE("), "{entity}");
    }
}

#[test]
fn bad_parameters_are_errors_and_change_nothing() {
    let mut s = session();
    new_doc(&mut s);
    run(
        &mut s,
        "feature.block",
        json!({"size": [10, 10, 10], "name": "Box"}),
    );
    run(&mut s, "sketch.create", json!({"plane": "xy"}));
    let bad: Vec<(Value, &str)> = vec![
        (json!({"color": "#fff"}), "give `faces`"),
        (json!({"faces": [], "color": "#fff"}), "empty"),
        (
            json!({"faces": "F1:face(0)", "color": "#fff"}),
            "list of persistent names",
        ),
        (
            json!({"faces": [7], "color": "#fff"}),
            "list of persistent names",
        ),
        (json!({"faces": ["F1:face(0)"]}), "color"),
        (json!({"faces": ["F1:face(0)"], "color": "blue"}), "color"),
        (
            json!({"faces": ["F1:face(0)"], "color": [1, 2, 300]}),
            "color",
        ),
        (
            json!({"faces": ["F1:face(0)"], "color": "#fff", "metallic": 2}),
            "metallic",
        ),
        (
            json!({"faces": ["F1:face(9)"], "color": "#fff"}),
            "no body has a face",
        ),
        (
            json!({"faces": ["F9:face(0)"], "color": "#fff"}),
            "no body has a face",
        ),
        (
            json!({"faces": ["not a name"], "color": "#fff"}),
            "persistent name",
        ),
        (
            json!({"faces": ["F1:face(0)"], "body": "Nope", "color": "#fff"}),
            "no feature",
        ),
        (
            json!({"faces": ["F1:face(0)"], "body": [1], "color": "#fff"}),
            "body",
        ),
        (
            json!({"faces": ["F1:face(0)"], "body": "Sketch1", "color": "#fff"}),
            "has no body",
        ),
        (
            json!({"faces": ["F1:face(0)"], "clear": true, "color": "#fff"}),
            "leave out",
        ),
        (
            json!({"faces": ["F1:face(9)"], "clear": true}),
            "no body has a face",
        ),
        (
            json!({"selection": true, "color": "#fff"}),
            "selection has no faces",
        ),
        (
            json!({"selection": true, "faces": ["F1:face(0)"], "color": "#fff"}),
            "leave out",
        ),
        (
            json!({"faces": ["F1:face(0)"], "color": "#fff", "extra": 1}),
            "extra",
        ),
    ];
    for (params, why) in bad {
        let message = error(&mut s, "face.appearance", params.clone());
        assert!(message.contains(why), "{params}: {message}");
    }
    let inspect = run(&mut s, "document.inspect", json!({"measures": false}));
    assert!(inspect.get("faceAppearances").is_none());
    // An edge's name is not a face's.
    run(&mut s, "select.set", json!({"names": ["F1"]}));
    let message = error(
        &mut s,
        "face.appearance",
        json!({"selection": true, "color": "#fff"}),
    );
    assert!(message.contains("it holds body"), "{message}");
    // Without a document.
    run(&mut s, "file.close", json!({"discard": true}));
    error(
        &mut s,
        "face.appearance",
        json!({"faces": ["F1:face(0)"], "color": "#fff"}),
    );
}

#[test]
fn a_face_coloured_by_a_name_that_becomes_an_alias_keeps_its_colour() {
    // Three blocks in a row; instance 1's top green. Moved together, the
    // tops merge into one face named after instance 0, with instance 1's
    // name as an alias: the merged top is green (the entry is found by
    // the alias), never silently dropped.
    let mut s = session();
    new_doc(&mut s);
    run(
        &mut s,
        "feature.block",
        json!({"size": [10, 10, 10], "name": "Seed"}),
    );
    run(
        &mut s,
        "feature.pattern",
        json!({"source": "Seed", "count": 3, "step": [20, 0, 0], "name": "Row"}),
    );
    run(
        &mut s,
        "face.appearance",
        json!({"faces": ["F2:copy(1)/F1:face(1)"], "color": GREEN}),
    );
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Row", "step": [10, 0, 0]}),
    );
    assert_eq!(coloured(&mut s, "F2", GREEN), ["F2:copy(0)/F1:face(1)"]);
    let inspect = run(&mut s, "document.inspect", json!({"measures": false}));
    assert!(inspect.get("lostFaces").is_none(), "{inspect}");
    // Several of its names with colours: the face's own name first, then
    // its aliases in instance order. (A name given to face.appearance is
    // read as the face's own name, so instance 2's top is coloured while
    // the tops are apart.)
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Row", "step": [20, 0, 0]}),
    );
    run(
        &mut s,
        "face.appearance",
        json!({"faces": ["F2:copy(2)/F1:face(1)"], "color": RED}),
    );
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Row", "step": [10, 0, 0]}),
    );
    assert_eq!(coloured(&mut s, "F2", GREEN).len(), 1);
    // The merged face's own name, by any of its names.
    run(
        &mut s,
        "face.appearance",
        json!({"body": "Row", "faces": ["F2:copy(2)/F1:face(1)"], "color": BLUE}),
    );
    assert_eq!(coloured(&mut s, "F2", BLUE).len(), 1);
    assert!(coloured(&mut s, "F2", GREEN).is_empty());
    // Apart again: each top its own colour.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Row", "step": [20, 0, 0]}),
    );
    assert_eq!(coloured(&mut s, "F2", BLUE), ["F2:copy(0)/F1:face(1)"]);
    assert_eq!(coloured(&mut s, "F2", GREEN), ["F2:copy(1)/F1:face(1)"]);
    assert_eq!(coloured(&mut s, "F2", RED), ["F2:copy(2)/F1:face(1)"]);
}

/// A face's own colour wins over its body's, on whichever feature of the
/// chain the body's is given (the developer's rule of 2026-11-10, J-040).
/// Before it, the nearest feature won and a body colour on a later
/// feature hid an earlier face colour: this test was
/// `a_body_colour_on_a_later_feature_goes_before_an_earlier_face_colour`
/// and found red on all six faces.
#[test]
fn an_earlier_face_colour_goes_before_a_body_colour_on_a_later_feature() {
    let mut s = session();
    new_doc(&mut s);
    run(
        &mut s,
        "feature.block",
        json!({"size": [10, 10, 10], "name": "Box"}),
    );
    run(
        &mut s,
        "face.appearance",
        json!({"faces": ["F1:face(1)"], "color": BLUE}),
    );
    run(
        &mut s,
        "feature.transform",
        json!({"source": "Box", "translate": [0, 0, 5], "name": "Up"}),
    );
    assert_eq!(coloured(&mut s, "F2", BLUE), ["F2:copy(0)/F1:face(1)"]);
    // The later body's colour on the other five faces; the face keeps its
    // own.
    run(
        &mut s,
        "body.appearance",
        json!({"body": "Up", "color": RED}),
    );
    assert_eq!(coloured(&mut s, "F2", BLUE), ["F2:copy(0)/F1:face(1)"]);
    assert_eq!(coloured(&mut s, "F2", RED).len(), 5);
    // A face colour on that later body goes before the earlier one.
    run(
        &mut s,
        "face.appearance",
        json!({"faces": ["F2:copy(0)/F1:face(1)"], "color": GREEN}),
    );
    assert_eq!(coloured(&mut s, "F2", GREEN), ["F2:copy(0)/F1:face(1)"]);
    assert_eq!(coloured(&mut s, "F2", RED).len(), 5);
}

#[test]
fn a_step_file_brings_its_face_colours_back_as_face_colours() {
    let dir = workdir("face-appearance", "step-import");
    let mut s = session_in(&dir);
    slotted_plate(&mut s);
    run(
        &mut s,
        "face.appearance",
        json!({"body": "Cut", "faces": ["F3:copy(1)/F2:face(0)"], "color": GREEN}),
    );
    let mut before: Vec<Option<[u8; 3]>> = scene_faces(&mut s, "F3")
        .into_iter()
        .map(|(_, c)| c)
        .collect();
    run(&mut s, "file.export", json!({"path": "faces.step"}));
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "file.import", json!({"path": "faces.step"}));
    // The body has no colour of its own (some faces have none in the
    // file); the styled faces keep theirs, by their names in the import.
    let inspect = run(&mut s, "document.inspect", json!({"measures": false}));
    assert!(
        inspect["bodies"][0].get("appearance").is_none(),
        "{inspect}"
    );
    assert_eq!(inspect["faceAppearances"].as_array().unwrap().len(), 3);
    let mut after: Vec<Option<[u8; 3]>> = scene_faces(&mut s, "F1")
        .into_iter()
        .map(|(_, c)| c)
        .collect();
    before.sort();
    after.sort();
    assert_eq!(after, before);
    // One undo step for the import and its colours.
    run(&mut s, "edit.undo", json!({}));
    let inspect = run(&mut s, "document.inspect", json!({"measures": false}));
    assert!(inspect.get("faceAppearances").is_none());
}

/// J-040: a tube whose inner wall is coloured after its extrude, then cut
/// by a port and turned, given a white body colour on the turned body: the
/// inner wall keeps its own colour, in the scene (one resolution for both
/// renderers and the exports) and in a picture; the other faces are white.
/// Without the body colour, and with it on the extrude itself, the same.
#[test]
fn a_face_colour_wins_over_a_body_colour_given_on_a_later_feature() {
    const DARK: [u8; 3] = [0x0c, 0x0c, 0x0e];
    const WHITE: [u8; 3] = [0xff, 0xff, 0xff];
    let evidence =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/j040");
    std::fs::create_dir_all(&evidence).unwrap();
    for (case, body) in [
        ("repro", Some("Moved")),
        ("no-body-colour", None),
        ("same-feature", Some("Tube")),
    ] {
        let mut s = session();
        new_doc(&mut s);
        let steps = json!([
            ["sketch.create", {"plane": "xy", "name": "Ring"}],
            ["sketch.circle", {"sketch": "Ring", "center": ["0 mm", "0 mm"], "radius": "100 mm"}],
            ["sketch.circle", {"sketch": "Ring", "center": ["0 mm", "0 mm"], "radius": "90 mm"}],
            ["feature.extrude", {"sketch": "Ring", "distance": "500 mm", "name": "Tube"}],
            ["face.appearance", {"faces": ["F2:side(e4)"], "color": "#0c0c0e", "metallic": 0, "roughness": 0.8}],
        ]);
        for step in steps.as_array().unwrap() {
            run(&mut s, step[0].as_str().unwrap(), step[1].clone());
        }
        if body == Some("Tube") {
            run(
                &mut s,
                "body.appearance",
                json!({"body": "Tube", "color": "#ffffff"}),
            );
        }
        run(
            &mut s,
            "feature.cylinder",
            json!({"radius": "20 mm", "height": "40 mm", "position": ["0 mm", "80 mm", "250 mm"], "axis": [0, 1, 0], "name": "Port"}),
        );
        run(
            &mut s,
            "feature.boolean",
            json!({"op": "subtract", "target": "Tube", "tool": "Port", "name": "Ported"}),
        );
        run(
            &mut s,
            "feature.transform",
            json!({"source": "Ported", "angle": "30 deg", "axis": [1, 0, 0], "name": "Moved"}),
        );
        if body == Some("Moved") {
            run(
                &mut s,
                "body.appearance",
                json!({"body": "Moved", "color": "#ffffff"}),
            );
        }
        // The turned body is F5's; its inner wall came from F2:side(e4)
        // through the cut and the turn (its pieces share that name).
        let faces = scene_faces(&mut s, "F5");
        let inner: Vec<&(String, Option<[u8; 3]>)> = faces
            .iter()
            .filter(|(name, _)| name.ends_with("F2:side(e4)"))
            .collect();
        assert!(!inner.is_empty(), "{case}: {faces:?}");
        for (name, colour) in &inner {
            assert_eq!(*colour, Some(DARK), "{case}: {name}");
        }
        if body.is_some() {
            for (name, colour) in faces.iter().filter(|(n, _)| !n.ends_with("F2:side(e4)")) {
                assert_eq!(*colour, Some(WHITE), "{case}: {name}");
            }
        }
        let shot = run(
            &mut s,
            "view.render",
            json!({"width": 500, "height": 375, "camera": {"eye": [0, -700, 900], "target": [0, 0, 300], "up": [0, 0, 1], "fovY": 35}, "renderer": "cpu", "look": {"mode": "studio", "accumulate": 1}}),
        );
        if let Some(png) = shot["base64"].as_str() {
            std::fs::write(evidence.join(format!("{case}.png")), decode_base64(png)).unwrap();
        }
    }
}

/// The id of the feature called `name`.
fn feature_id(s: &mut Session, name: &str) -> String {
    let inspect = run(s, "document.inspect", json!({"measures": false}));
    inspect["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == name)
        .and_then(|f| f["id"].as_str())
        .unwrap_or_else(|| panic!("no feature {name}: {inspect}"))
        .to_string()
}

/// The face colours of a body as the scene draws them, in face order.
fn colours_of(s: &mut Session, feature: &str) -> Vec<Option<[u8; 3]>> {
    scene_faces(s, feature)
        .into_iter()
        .map(|(_, c)| c)
        .collect()
}

/// A body's face colours cleared in one step: `face.appearance {body,
/// clear: true}` takes away the colours its faces carry wherever along
/// its history they were given, so a later body colour shows on every
/// face; the answer lists exactly the stored entries; undo brings them
/// back and redo takes them again.
#[test]
fn a_body_s_face_colours_are_cleared_in_one_step() {
    let mut s = session();
    new_doc(&mut s);
    // F1, its top (face 1) blue and its bottom (face 0) green, then cut
    // (F3), moved (F4) and given a red body colour.
    run(
        &mut s,
        "feature.block",
        json!({"size": [40, 30, 10], "name": "Plate"}),
    );
    run(
        &mut s,
        "face.appearance",
        json!({"faces": ["F1:face(1)"], "color": BLUE}),
    );
    run(
        &mut s,
        "face.appearance",
        json!({"faces": ["F1:face(0)"], "color": GREEN}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": 3, "height": 20, "position": [10, 10, -5], "name": "Drill"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Plate", "tool": "Drill", "name": "Cut"}),
    );
    run(
        &mut s,
        "feature.transform",
        json!({"source": "Cut", "translate": [0, 0, 50], "name": "Moved"}),
    );
    run(
        &mut s,
        "body.appearance",
        json!({"body": "Moved", "color": RED}),
    );
    let before = colours_of(&mut s, "F4");
    assert_eq!(before.iter().filter(|c| **c == Some(BLUE)).count(), 1);
    assert_eq!(before.iter().filter(|c| **c == Some(GREEN)).count(), 1);
    let out = run(
        &mut s,
        "face.appearance",
        json!({"body": "Moved", "clear": true}),
    );
    assert_eq!(out["count"], 2, "{out}");
    assert_eq!(out["message"], "Cleared 2 face colours of Moved", "{out}");
    assert!(
        out["cleared"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["feature"] == "F1"),
        "{out}"
    );
    let cleared: Vec<(String, String)> = out["cleared"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["face"].as_str().unwrap().to_string(),
                e["color"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(
        cleared,
        [
            ("F1:face(0)".to_string(), "#14a03c".to_string()),
            ("F1:face(1)".to_string(), "#2f6fd6".to_string()),
        ]
    );
    assert_eq!(out["kept"], json!([]), "{out}");
    assert!(colours_of(&mut s, "F4").iter().all(|c| *c == Some(RED)));
    // One undo step: the colours are back, and redo takes them again.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(colours_of(&mut s, "F4"), before);
    run(&mut s, "edit.redo", json!({}));
    assert!(colours_of(&mut s, "F4").iter().all(|c| *c == Some(RED)));
    // Nothing left to clear is no error.
    let again = run(
        &mut s,
        "face.appearance",
        json!({"body": "Moved", "clear": true}),
    );
    assert_eq!(again["count"], 0, "{again}");
    assert_eq!(again["cleared"], json!([]), "{again}");
    // A hidden body is cleared as a shown one is.
    run(&mut s, "edit.undo", json!({}));
    run(
        &mut s,
        "feature.visible",
        json!({"feature": "Moved", "visible": false}),
    );
    let hidden = run(
        &mut s,
        "face.appearance",
        json!({"body": "Moved", "clear": true}),
    );
    assert_eq!(hidden["count"], 2, "{hidden}");
    run(
        &mut s,
        "feature.visible",
        json!({"feature": "Moved", "visible": true}),
    );
    assert!(colours_of(&mut s, "F4").iter().all(|c| *c == Some(RED)));
}

/// A colour that also reaches a face of another body (a copy kept by a
/// mirror) is kept, and the answer says for which body; the colour given
/// to this body alone is cleared, and the copy keeps its colours.
#[test]
fn clearing_a_body_keeps_the_colours_another_body_shares() {
    let mut s = session();
    new_doc(&mut s);
    run(
        &mut s,
        "feature.block",
        json!({"size": [10, 10, 10], "name": "Part"}),
    );
    run(
        &mut s,
        "face.appearance",
        json!({"faces": ["F1:face(1)"], "color": BLUE}),
    );
    // A copy kept by a mirror (F2), then the part itself moved (F3) and
    // a face of the moved part given green there.
    run(
        &mut s,
        "feature.mirror",
        json!({"source": "Part", "normal": [1, 0, 0], "point": [30, 0, 0], "keep": true, "name": "Twin"}),
    );
    run(
        &mut s,
        "feature.transform",
        json!({"source": "Part", "translate": [0, 0, 30], "name": "Moved"}),
    );
    run(
        &mut s,
        "face.appearance",
        json!({"body": "Moved", "faces": ["F3:copy(0)/F1:face(2)"], "color": GREEN}),
    );
    let twin = colours_of(&mut s, "F2");
    // Inspect tells the two apart: Moved has one colour of its own (the
    // green) and one it shares with Twin; Twin shares that one.
    let inspect = run(&mut s, "document.inspect", json!({"measures": false}));
    let bodies = inspect["bodies"].as_array().unwrap();
    let body = |name: &str| bodies.iter().find(|b| b["name"] == name).unwrap().clone();
    assert_eq!(body("Moved")["faceColors"], 1, "{inspect}");
    assert_eq!(body("Moved")["sharedFaceColors"], 1, "{inspect}");
    assert_eq!(body("Moved")["sharedWith"], json!(["Twin"]), "{inspect}");
    assert!(body("Twin").get("faceColors").is_none(), "{inspect}");
    assert_eq!(body("Twin")["sharedFaceColors"], 1, "{inspect}");
    assert_eq!(body("Twin")["sharedWith"], json!(["Moved"]), "{inspect}");
    assert_eq!(
        twin.iter().filter(|c| **c == Some(BLUE)).count(),
        1,
        "{twin:?}"
    );
    let out = run(
        &mut s,
        "face.appearance",
        json!({"body": "Moved", "clear": true}),
    );
    assert_eq!(out["count"], 1, "{out}");
    assert_eq!(out["cleared"][0]["face"], "F3:copy(0)/F1:face(2)", "{out}");
    assert_eq!(out["cleared"][0]["color"], "#14a03c", "{out}");
    assert_eq!(out["kept"][0]["face"], "F1:face(1)", "{out}");
    assert_eq!(out["kept"][0]["for"], json!(["Twin"]), "{out}");
    assert_eq!(
        out["message"],
        "Cleared 1 face colour of Moved; 1 kept, shared with Twin: clearing it by its name where it was given (the command under kept) takes it from Twin too",
        "{out}"
    );
    // The copy keeps its blue face; the moved part keeps it too (kept),
    // and its green is gone.
    assert_eq!(colours_of(&mut s, "F2"), twin);
    let moved = colours_of(&mut s, "F3");
    assert_eq!(
        moved.iter().filter(|c| **c == Some(BLUE)).count(),
        1,
        "{moved:?}"
    );
    assert!(!moved.contains(&Some(GREEN)), "{moved:?}");
    // Clearing Twin keeps it too: no way out there. The kept entry names
    // the command that does clear it, at the feature it was given on;
    // that command takes the blue from both bodies.
    let twin_out = run(
        &mut s,
        "face.appearance",
        json!({"body": "Twin", "clear": true}),
    );
    assert_eq!(twin_out["count"], 0, "{twin_out}");
    assert_eq!(twin_out["kept"][0]["for"], json!(["Moved"]), "{twin_out}");
    let clear = out["kept"][0]["clear"].clone();
    assert_eq!(
        clear,
        json!({"body": "F1", "faces": ["F1:face(1)"], "clear": true}),
        "{out}"
    );
    let why = out["kept"][0]["why"].as_str().unwrap();
    assert!(why.contains(&format!("face.appearance {clear}")), "{why}");
    assert!(why.contains("from Moved and Twin"), "{why}");
    run(&mut s, "face.appearance", clear);
    assert!(!colours_of(&mut s, "F2").contains(&Some(BLUE)));
    assert!(!colours_of(&mut s, "F3").contains(&Some(BLUE)));
    let inspect = run(&mut s, "document.inspect", json!({"measures": false}));
    assert!(inspect.get("faceAppearances").is_none(), "{inspect}");
}

/// A plate with two blue faces cut through and split in two (`body.split`):
/// both pieces carry both colours, so clearing either keeps both, for the
/// other piece; the commands under `kept` clear them from both pieces.
#[test]
fn a_split_body_s_shared_colours_are_cleared_by_the_kept_commands() {
    let mut s = session();
    new_doc(&mut s);
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": [40, 20, 10]}),
    );
    run(
        &mut s,
        "face.appearance",
        json!({"faces": ["F1:face(1)", "F1:face(2)"], "color": BLUE}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Slot", "size": [4, 40, 20], "position": [18, -10, -5]}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Plate", "tool": "Slot", "name": "Cut"}),
    );
    run(
        &mut s,
        "body.split",
        json!({"source": "Cut", "names": ["Left", "Right"]}),
    );
    let blue = |s: &mut Session, body: &str| {
        colours_of(s, body)
            .iter()
            .filter(|c| **c == Some(BLUE))
            .count()
    };
    let (left, right) = (feature_id(&mut s, "Left"), feature_id(&mut s, "Right"));
    assert!(blue(&mut s, &left) > 0 && blue(&mut s, &right) > 0);
    let out = run(
        &mut s,
        "face.appearance",
        json!({"body": "Left", "clear": true}),
    );
    assert_eq!(out["count"], 0, "{out}");
    let kept = out["kept"].as_array().unwrap();
    assert_eq!(kept.len(), 2, "{out}");
    assert!(kept.iter().all(|k| k["for"] == json!(["Right"])), "{out}");
    assert_eq!(
        out["message"],
        "No face colours of Left to clear; 2 kept, shared with Right: clearing them by their names where they were given (the commands under kept) takes them from Right too",
        "{out}"
    );
    for k in kept {
        let clear = k["clear"].clone();
        assert_eq!(clear["body"], "F1", "{k}");
        assert!(
            k["why"]
                .as_str()
                .unwrap()
                .contains(&format!("face.appearance {clear}")),
            "{k}"
        );
        run(&mut s, "face.appearance", clear);
    }
    assert_eq!(blue(&mut s, &left), 0);
    assert_eq!(blue(&mut s, &right), 0);
    let inspect = run(&mut s, "document.inspect", json!({"measures": false}));
    assert!(inspect.get("faceAppearances").is_none(), "{inspect}");
}

/// A pattern's instances are one body: the seed's face colour, shown on
/// every instance's top, is cleared from all of them (nothing is kept),
/// and the pattern's body colour shows on every face.
#[test]
fn clearing_a_pattern_takes_the_seed_s_colour_from_every_instance() {
    let mut s = session();
    new_doc(&mut s);
    run(
        &mut s,
        "feature.block",
        json!({"name": "Cube", "size": [10, 10, 10]}),
    );
    run(
        &mut s,
        "face.appearance",
        json!({"faces": ["F1:face(1)"], "color": BLUE}),
    );
    run(
        &mut s,
        "feature.pattern",
        json!({"name": "Cubes", "source": "Cube", "count": 3, "step": [20, 0, 0]}),
    );
    run(
        &mut s,
        "body.appearance",
        json!({"body": "Cubes", "color": RED}),
    );
    let before = colours_of(&mut s, "F2");
    assert_eq!(
        before.iter().filter(|c| **c == Some(BLUE)).count(),
        3,
        "{before:?}"
    );
    let out = run(
        &mut s,
        "face.appearance",
        json!({"body": "Cubes", "clear": true}),
    );
    assert_eq!(out["count"], 1, "{out}");
    assert_eq!(out["kept"], json!([]), "{out}");
    assert!(colours_of(&mut s, "F2").iter().all(|c| *c == Some(RED)));
}

/// An imported STEP part with a colour on every face: its face colours
/// cleared in one call, after which the body colour shows on every face.
#[test]
fn an_imported_part_with_a_colour_on_every_face_is_cleared_in_one_call() {
    let dir = workdir("face-appearance", "step-every-face");
    let mut s = session_in(&dir);
    new_doc(&mut s);
    run(
        &mut s,
        "feature.block",
        json!({"size": [10, 10, 10], "name": "Cube"}),
    );
    let colours = [
        "#102030", "#203040", "#304050", "#405060", "#506070", "#607080",
    ];
    for (k, colour) in colours.iter().enumerate() {
        run(
            &mut s,
            "face.appearance",
            json!({"faces": [format!("F1:face({k})")], "color": colour}),
        );
    }
    // A body colour of its own, so the file styles the solid apart from
    // every face (as a CAD part with a colour on every face is written).
    run(
        &mut s,
        "body.appearance",
        json!({"body": "Cube", "color": "#808080"}),
    );
    run(&mut s, "file.export", json!({"path": "cube.step"}));
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "file.import", json!({"path": "cube.step"}));
    let inspect = run(&mut s, "document.inspect", json!({"measures": false}));
    assert_eq!(inspect["faceAppearances"].as_array().unwrap().len(), 6);
    run(
        &mut s,
        "body.appearance",
        json!({"body": "F1", "color": RED}),
    );
    assert!(!colours_of(&mut s, "F1").contains(&Some(RED)));
    let out = run(
        &mut s,
        "face.appearance",
        json!({"body": "F1", "clear": true}),
    );
    assert_eq!(out["count"], 6, "{out}");
    let mut cleared: Vec<String> = out["cleared"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["color"].as_str().unwrap().to_string())
        .collect();
    cleared.sort();
    assert_eq!(cleared, colours);
    assert!(colours_of(&mut s, "F1").iter().all(|c| *c == Some(RED)));
}

/// What clearing a body's face colours refuses, by name, changing
/// nothing: no such feature, a sketch, a body switched off, a body that
/// went into a later feature, `clear` false and `clear` with a colour.
#[test]
fn clearing_a_body_s_face_colours_refuses_what_is_not_a_body() {
    let mut s = session();
    new_doc(&mut s);
    run(&mut s, "variable.set", json!({"name": "on", "expr": "0"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [10, 10, 10], "name": "Part"}),
    );
    run(
        &mut s,
        "face.appearance",
        json!({"faces": ["F1:face(1)"], "color": BLUE}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"size": [5, 5, 5], "position": [20, 0, 0], "name": "Option", "when": "on"}),
    );
    run(
        &mut s,
        "feature.transform",
        json!({"source": "Part", "translate": [0, 0, 30], "name": "Moved"}),
    );
    run(
        &mut s,
        "sketch.create",
        json!({"name": "Drawing", "plane": "xy"}),
    );
    let revision = s.document_revision();
    for (params, words) in [
        (
            json!({"body": "Nobody", "clear": true}),
            "no feature `Nobody`",
        ),
        (
            json!({"body": "Drawing", "clear": true}),
            "is a sketch, not a body",
        ),
        (json!({"body": "Option", "clear": true}), "is inactive"),
        (json!({"body": "Part", "clear": true}), "went into `Moved`"),
        (json!({"body": "Moved", "clear": false}), "give `faces`"),
        (
            json!({"body": "Moved", "clear": true, "color": BLUE}),
            "leave out `color`",
        ),
        (
            json!({"body": ["Moved"], "clear": true}),
            "feature id or name",
        ),
    ] {
        let e = error(&mut s, "face.appearance", params.clone());
        assert!(e.contains(words), "{params}: {e}");
    }
    assert_eq!(s.document_revision(), revision, "nothing changed");
}

/// Two faces of a union coloured apart (one red, one green) that an
/// upstream edit then merges into one face: both entries stay, at the
/// union, by the two names the merged face answers to. Clearing the body
/// of a later transform takes both, so the body's colour shows, not the
/// second one.
#[test]
fn clearing_a_body_takes_every_entry_of_a_merged_face() {
    let mut s = session();
    new_doc(&mut s);
    run(
        &mut s,
        "feature.block",
        json!({"name": "A", "size": [10, 10, 10]}),
    );
    // B's top 1 above A's: two top faces in the union.
    run(
        &mut s,
        "feature.block",
        json!({"name": "B", "size": [10, 10, 10], "position": [10, 0, 1]}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "union", "target": "A", "tool": "B", "name": "Both"}),
    );
    run(
        &mut s,
        "face.appearance",
        json!({"body": "Both", "faces": ["F3:copy(0)/F1:face(1)"], "color": RED}),
    );
    run(
        &mut s,
        "face.appearance",
        json!({"body": "Both", "faces": ["F3:copy(1)/F2:face(1)"], "color": GREEN}),
    );
    run(
        &mut s,
        "feature.transform",
        json!({"source": "Both", "translate": [0, 0, 30], "name": "Moved"}),
    );
    run(
        &mut s,
        "body.appearance",
        json!({"body": "Moved", "color": BLUE}),
    );
    // B lowered: the tops are one plane and merge into one face.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "B", "position": [10, 0, 0]}),
    );
    let inspect = run(&mut s, "document.inspect", json!({"measures": false}));
    assert_eq!(
        inspect["faceAppearances"].as_array().unwrap().len(),
        2,
        "{inspect}"
    );
    let out = run(
        &mut s,
        "face.appearance",
        json!({"body": "Moved", "clear": true}),
    );
    assert_eq!(out["count"], 2, "{out}");
    let inspect = run(&mut s, "document.inspect", json!({"measures": false}));
    assert!(inspect.get("faceAppearances").is_none(), "{inspect}");
    assert!(colours_of(&mut s, "F4").iter().all(|c| *c == Some(BLUE)));
}
