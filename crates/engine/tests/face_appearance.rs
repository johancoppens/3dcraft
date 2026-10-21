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
    // Face over body at one feature; a body colour of the plate does not
    // reach the coloured faces, which keep theirs.
    run(
        &mut s,
        "body.appearance",
        json!({"body": "Cut", "color": RED}),
    );
    assert_eq!(coloured(&mut s, "F3", GREEN), [floor]);
    assert_eq!(coloured(&mut s, "F3", BLUE), Vec::<String>::new());
    let faces = scene_faces(&mut s, "F3");
    assert!(
        faces
            .iter()
            .all(|(name, c)| *c == Some(if name == floor { GREEN } else { RED })),
        "{faces:?}"
    );
    // ... and the plate's blue top comes back where the red is cleared.
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
    let mut s = session();
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
    let scene = s.scene(None).unwrap().unwrap();
    assert!(
        scene.problems.iter().any(|p| p.contains(floor)),
        "{:?}",
        scene.problems
    );
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
