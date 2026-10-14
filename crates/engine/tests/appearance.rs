//! A colour and a material per body: the commands, how the colour follows
//! a body through booleans and moves, persistence, the pictures and the
//! exports that show it, and the mass and inertia a density gives.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::needless_range_loop
)]

use craft3d_engine::{Access, Session, Workspace};
use serde_json::{Value, json};
use std::f64::consts::PI;

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    let result = session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"));
    assert_ne!(
        session.regeneration_matches_a_full_one(),
        Some(false),
        "after {id} {params}"
    );
    result
}

fn error(session: &mut Session, id: &str, params: Value) -> String {
    session.execute(id, &params).unwrap_err().to_string()
}

fn session_in(dir: Option<&std::path::Path>) -> Session {
    let access = || match dir {
        Some(dir) => Access::Root(dir.to_path_buf()),
        None => Access::None,
    };
    let mut s = Session::new(Workspace {
        read: access(),
        write: access(),
    });
    run(&mut s, "file.new", json!({"units": "mm"}));
    s
}

const RED: [u8; 3] = [200, 16, 46];
const BLUE: [u8; 3] = [30, 90, 220];

/// A plate (F1, 40 x 30 x 10) and a pin (F2, a cylinder through it).
fn plate_and_pin(s: &mut Session) {
    run(
        s,
        "feature.block",
        json!({"size": [40, 30, 10], "name": "Plate"}),
    );
    run(
        s,
        "feature.cylinder",
        json!({"position": [20, 15, -5], "radius": 5, "height": 20, "name": "Pin"}),
    );
}

fn inspect_body(s: &mut Session, name: &str) -> Value {
    let document = run(s, "document.inspect", json!({}));
    document["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == name)
        .unwrap_or_else(|| panic!("no body {name}: {document}"))
        .clone()
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
                scene.face_colors.get(body.first_face_id + k as u32),
            )
        })
        .collect()
}

#[test]
fn a_colour_is_set_reported_and_cleared() {
    let mut s = session_in(None);
    plate_and_pin(&mut s);
    let out = run(
        &mut s,
        "body.appearance",
        json!({"body": "Plate", "color": "#c8102e", "roughness": 0.4}),
    );
    assert_eq!(
        out["appearance"],
        json!({"color": "#c8102e", "roughness": 0.4})
    );
    assert_eq!(
        inspect_body(&mut s, "Plate")["appearance"],
        json!({"color": "#c8102e", "roughness": 0.4})
    );
    assert!(inspect_body(&mut s, "Pin").get("appearance").is_none());
    // [r, g, b] and #rgb work too.
    run(
        &mut s,
        "body.appearance",
        json!({"body": "F2", "color": [30, 90, 220]}),
    );
    assert_eq!(
        inspect_body(&mut s, "Pin")["appearance"]["color"],
        "#1e5adc"
    );
    run(
        &mut s,
        "body.appearance",
        json!({"body": 2, "color": "#fff"}),
    );
    assert_eq!(
        inspect_body(&mut s, "Pin")["appearance"]["color"],
        "#ffffff"
    );
    // Undo and redo, like any edit.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(
        inspect_body(&mut s, "Pin")["appearance"]["color"],
        "#1e5adc"
    );
    run(&mut s, "edit.redo", json!({}));
    run(
        &mut s,
        "body.appearance",
        json!({"body": "Pin", "clear": true}),
    );
    assert!(inspect_body(&mut s, "Pin").get("appearance").is_none());
    // A change of parameters keeps the colour: it belongs to the feature.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Plate", "size": [50, 30, 10]}),
    );
    assert_eq!(
        inspect_body(&mut s, "Plate")["appearance"]["color"],
        "#c8102e"
    );
}

#[test]
fn bad_parameters_are_errors_and_change_nothing() {
    let mut s = session_in(None);
    plate_and_pin(&mut s);
    run(&mut s, "sketch.create", json!({"name": "Sk"}));
    let before = run(&mut s, "document.inspect", json!({}));
    let cases = [
        json!({}),
        json!({"body": "Plate"}),
        json!({"body": "Nope", "color": "#ffffff"}),
        json!({"body": 99, "color": "#ffffff"}),
        json!({"body": "Sk", "color": "#ffffff"}),
        json!({"body": "Plate", "color": "red"}),
        json!({"body": "Plate", "color": "#12"}),
        json!({"body": "Plate", "color": "#gg0000"}),
        json!({"body": "Plate", "color": "#ffffffff"}),
        json!({"body": "Plate", "color": [1, 2]}),
        json!({"body": "Plate", "color": [300, 0, 0]}),
        json!({"body": "Plate", "color": [-1, 0, 0]}),
        json!({"body": "Plate", "color": [0.5, 0, 0]}),
        json!({"body": "Plate", "color": null}),
        json!({"body": "Plate", "color": "#ffffff", "metallic": 2}),
        json!({"body": "Plate", "color": "#ffffff", "roughness": -0.1}),
        json!({"body": "Plate", "color": "#ffffff", "metallic": "shiny"}),
        json!({"body": "Plate", "color": "#ffffff", "shine": 1}),
        json!({"body": "Plate", "clear": true, "color": "#ffffff"}),
        json!({"body": "Plate", "clear": "yes"}),
        json!({"body": [1], "color": "#ffffff"}),
    ];
    for params in cases {
        let e = error(&mut s, "body.appearance", params.clone());
        assert!(e.starts_with("body.appearance"), "{params}: {e}");
    }
    let materials = [
        json!({"body": "Plate"}),
        json!({"body": "Plate", "name": "Steel"}),
        json!({"body": "Plate", "name": "Steel", "density": 0}),
        json!({"body": "Plate", "name": "Steel", "density": -7850}),
        json!({"body": "Plate", "name": "Steel", "density": "abc"}),
        json!({"body": "Plate", "name": "Steel", "density": "5 furlongs"}),
        json!({"body": "Plate", "name": "Steel", "density": "g/cm3"}),
        json!({"body": "Plate", "name": "Steel", "density": [1]}),
        json!({"body": "Plate", "name": "", "density": 1}),
        json!({"body": "Plate", "name": " x", "density": 1}),
        json!({"body": "Plate", "name": "a\nb", "density": 1}),
        json!({"body": "Sk", "name": "Steel", "density": 1}),
        json!({"body": "Plate", "clear": true, "density": 1}),
    ];
    for params in materials {
        let e = error(&mut s, "body.material", params.clone());
        assert!(e.starts_with("body.material"), "{params}: {e}");
    }
    let after = run(&mut s, "document.inspect", json!({}));
    assert_eq!(before["bodies"], after["bodies"]);
    assert_eq!(before["history"], after["history"]);
}

#[test]
fn a_boolean_keeps_the_targets_colour_and_the_tools_faces_keep_the_tools() {
    let mut s = session_in(None);
    plate_and_pin(&mut s);
    run(
        &mut s,
        "body.appearance",
        json!({"body": "Plate", "color": RED}),
    );
    run(
        &mut s,
        "body.appearance",
        json!({"body": "Pin", "color": BLUE}),
    );
    let cut = run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Plate", "tool": "Pin", "name": "Holed"}),
    );
    assert_eq!(cut["status"], "ok");
    assert_eq!(
        inspect_body(&mut s, "Holed")["appearance"]["color"],
        "#c8102e"
    );
    // The wall of the hole came from the pin, the rest from the plate.
    let faces = scene_faces(&mut s, "F3");
    assert_eq!(faces.len(), 7);
    for (name, color) in &faces {
        let want = if name.starts_with("F3:copy(1)") {
            BLUE
        } else {
            RED
        };
        assert_eq!(*color, Some(want), "{name}");
    }
    assert_eq!(faces.iter().filter(|(_, c)| *c == Some(BLUE)).count(), 1);
    // A tool without a colour of its own: its faces take the body's.
    run(
        &mut s,
        "body.appearance",
        json!({"body": "Pin", "clear": true}),
    );
    assert!(
        scene_faces(&mut s, "F3")
            .iter()
            .all(|(_, c)| *c == Some(RED))
    );
    // A colour on the boolean colours the whole body.
    run(
        &mut s,
        "body.appearance",
        json!({"body": "Pin", "color": BLUE}),
    );
    run(
        &mut s,
        "body.appearance",
        json!({"body": "Holed", "color": "#00ff00"}),
    );
    assert!(
        scene_faces(&mut s, "F3")
            .iter()
            .all(|(_, c)| *c == Some([0, 255, 0]))
    );
    // A move and a further union carry the colours along: the moved
    // body's faces keep theirs, the union's tool (a block) its own.
    run(
        &mut s,
        "body.appearance",
        json!({"body": "Holed", "clear": true}),
    );
    run(
        &mut s,
        "feature.transform",
        json!({"source": "Holed", "translate": [0, 0, 100], "name": "Moved"}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"size": [10, 10, 10], "position": [-5, -5, 105], "name": "Lug"}),
    );
    run(
        &mut s,
        "body.appearance",
        json!({"body": "Lug", "color": "#ffd700"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "union", "target": "Moved", "tool": "Lug", "name": "Joined"}),
    );
    let faces = scene_faces(&mut s, "F6");
    let count = |c: [u8; 3]| faces.iter().filter(|(_, x)| *x == Some(c)).count();
    assert_eq!(count(BLUE), 1, "{faces:?}");
    assert!(count([255, 215, 0]) >= 3, "{faces:?}");
    assert!(count(RED) >= 5, "{faces:?}");
    assert_eq!(faces.iter().filter(|(_, c)| c.is_none()).count(), 0);
}

#[test]
fn colours_persist_through_save_and_load() {
    let dir = std::env::temp_dir().join(format!("craft3d-appearance-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = session_in(Some(&dir));
    plate_and_pin(&mut s);
    run(
        &mut s,
        "body.appearance",
        json!({"body": "Plate", "color": RED, "metallic": 0.9}),
    );
    run(
        &mut s,
        "body.material",
        json!({"body": "Pin", "name": "Brass", "density": "8.5 g/cm3"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Plate", "tool": "Pin", "keepTool": true}),
    );
    let before = scene_faces(&mut s, "F3");
    run(&mut s, "file.save", json!({"path": "coloured.3dcraft"}));
    run(&mut s, "file.close", json!({}));
    run(&mut s, "file.open", json!({"path": "coloured.3dcraft"}));
    assert_eq!(
        inspect_body(&mut s, "Boolean1")["appearance"],
        json!({"color": "#c8102e", "metallic": 0.9})
    );
    let pin = inspect_body(&mut s, "Pin");
    assert_eq!(pin["material"], json!({"name": "Brass", "density": 8500.0}));
    assert_eq!(scene_faces(&mut s, "F3"), before);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// `inertia` of an inspect or measure entry, as numbers.
fn tensor(entry: &Value) -> [[f64; 3]; 3] {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = entry["inertia"][i][j].as_f64().unwrap();
        }
    }
    out
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * b.abs().max(1e-300)
}

#[test]
fn a_density_gives_the_mass_and_the_inertia_tensor_in_closed_form() {
    for (unit, per_metre) in [("mm", 1e3_f64), ("m", 1.0), ("in", 1.0 / 0.0254)] {
        let mut s = Session::new(Workspace {
            read: Access::None,
            write: Access::None,
        });
        run(&mut s, "file.new", json!({"units": unit}));
        let (a, b, c) = (2.0, 3.0, 5.0);
        run(
            &mut s,
            "feature.block",
            json!({"size": [a, b, c], "position": [1, 2, 3], "name": "Block"}),
        );
        let (r, h) = (1.5, 4.0);
        run(
            &mut s,
            "feature.cylinder",
            json!({"position": [10, 0, 0], "radius": r, "height": h, "name": "Can"}),
        );
        run(
            &mut s,
            "feature.sphere",
            json!({"position": [-10, 0, 0], "radius": 2.5, "name": "Ball"}),
        );
        let density = 7850.0;
        for name in ["Block", "Can", "Ball"] {
            run(
                &mut s,
                "body.material",
                json!({"body": name, "name": "Steel", "density": density}),
            );
        }
        // Mass in kg: the volume in m³ times the density.
        let kg = |volume: f64| volume / per_metre.powi(3) * density;
        let measure = run(&mut s, "document.measure", json!({}));
        let entry = |name: &str| {
            measure["bodies"]
                .as_array()
                .unwrap()
                .iter()
                .find(|b| b["name"] == name)
                .unwrap()
                .clone()
        };
        let block = entry("Block");
        let m = kg(a * b * c);
        assert!(close(block["mass"].as_f64().unwrap(), m), "{unit}: {block}");
        assert_eq!(block["inertiaUnit"], format!("kg {unit}^2"));
        let t = tensor(&block);
        assert!(close(t[0][0], m * (b * b + c * c) / 12.0), "{unit}: {t:?}");
        assert!(close(t[1][1], m * (a * a + c * c) / 12.0), "{unit}: {t:?}");
        assert!(close(t[2][2], m * (a * a + b * b) / 12.0), "{unit}: {t:?}");
        assert!(t[0][1].abs() < 1e-12 * t[0][0], "{unit}: {t:?}");

        let can = entry("Can");
        let m = kg(PI * r * r * h);
        assert!(close(can["mass"].as_f64().unwrap(), m), "{unit}: {can}");
        let t = tensor(&can);
        assert!(
            close(t[0][0], m * (3.0 * r * r + h * h) / 12.0),
            "{unit}: {t:?}"
        );
        assert!(close(t[2][2], m * r * r / 2.0), "{unit}: {t:?}");

        let ball = entry("Ball");
        let m = kg(4.0 / 3.0 * PI * 2.5_f64.powi(3));
        assert!(close(ball["mass"].as_f64().unwrap(), m), "{unit}: {ball}");
        let t = tensor(&ball);
        for k in 0..3 {
            assert!(close(t[k][k], 0.4 * m * 2.5 * 2.5), "{unit}: {t:?}");
        }
        let total: f64 = ["Block", "Can", "Ball"]
            .iter()
            .map(|n| entry(n)["mass"].as_f64().unwrap())
            .sum();
        assert!(close(measure["mass"].as_f64().unwrap(), total));
        // document.inspect says the same.
        assert_eq!(inspect_body(&mut s, "Block")["mass"], block["mass"]);
    }
}

#[test]
fn a_material_follows_the_body_through_a_boolean_and_the_total_needs_every_body() {
    let mut s = session_in(None);
    plate_and_pin(&mut s);
    run(
        &mut s,
        "body.material",
        json!({"body": "Plate", "name": "Aluminium", "density": 2700}),
    );
    let measure = run(&mut s, "document.measure", json!({}));
    // The pin has no material: no total.
    assert!(measure.get("mass").is_none(), "{measure}");
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Plate", "tool": "Pin"}),
    );
    let holed = inspect_body(&mut s, "Boolean1");
    assert_eq!(holed["material"]["from"], "F1");
    let volume = 40.0 * 30.0 * 10.0 - PI * 25.0 * 10.0;
    assert!(
        close(holed["mass"].as_f64().unwrap(), volume * 1e-9 * 2700.0),
        "{holed}"
    );
    let measure = run(&mut s, "document.measure", json!({}));
    assert!(close(
        measure["mass"].as_f64().unwrap(),
        volume * 1e-9 * 2700.0
    ));
    // Clearing it takes the mass away.
    run(
        &mut s,
        "body.material",
        json!({"body": "Plate", "clear": true}),
    );
    assert!(inspect_body(&mut s, "Boolean1").get("mass").is_none());
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
    let b64 = out["base64"].as_str().unwrap();
    pixels(&decode_base64(b64))
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
fn view_render_shows_the_colours() {
    let mut s = session_in(None);
    plate_and_pin(&mut s);
    let plain = render(&mut s);
    let reddish = |p: &[[u8; 3]]| p.iter().filter(|c| c[0] as i32 - c[2] as i32 > 60).count();
    let bluish = |p: &[[u8; 3]]| p.iter().filter(|c| c[2] as i32 - c[0] as i32 > 60).count();
    assert_eq!(reddish(&plain), 0);
    run(
        &mut s,
        "body.appearance",
        json!({"body": "Plate", "color": RED}),
    );
    run(
        &mut s,
        "body.appearance",
        json!({"body": "Pin", "color": BLUE}),
    );
    let colored = render(&mut s);
    assert!(reddish(&colored) > 1000, "{}", reddish(&colored));
    assert!(bluish(&colored) > 100, "{}", bluish(&colored));
    // The background is untouched.
    assert_eq!(colored[0], plain[0]);
}

#[test]
fn a_3mf_and_a_step_export_carry_the_colours() {
    let dir = std::env::temp_dir().join(format!("craft3d-appearance-3mf-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = session_in(Some(&dir));
    plate_and_pin(&mut s);
    run(
        &mut s,
        "feature.block",
        json!({"size": [5, 5, 5], "position": [100, 0, 0], "name": "Plain"}),
    );
    run(
        &mut s,
        "body.appearance",
        json!({"body": "Plate", "color": RED}),
    );
    run(
        &mut s,
        "body.appearance",
        json!({"body": "Pin", "color": BLUE}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Plate", "tool": "Pin", "keepTool": true}),
    );
    run(&mut s, "file.export", json!({"path": "coloured.3mf"}));
    let bytes = std::fs::read(dir.join("coloured.3mf")).unwrap();
    // The model part, read back: stored entries in a ZIP.
    let text = String::from_utf8_lossy(&bytes).to_string();
    let start = text.find("<model ").unwrap();
    let xml = &text[start..text.find("</model>").unwrap()];
    let attribute = |tag: &str, name: &str| -> Option<String> {
        let at = tag.find(&format!(" {name}=\""))? + name.len() + 3;
        Some(tag[at..at + tag[at..].find('"')?].to_string())
    };
    let bases: Vec<String> = xml
        .lines()
        .filter(|l| l.trim_start().starts_with("<base "))
        .map(|l| attribute(l, "displaycolor").unwrap())
        .collect();
    assert_eq!(bases, ["#1E5ADC", "#C8102E"]);
    // Objects in history order: the pin (kept), the plain block, the cut
    // plate; each with its material, and the cut plate's hole wall blue.
    let mut objects: Vec<(String, Option<String>, usize)> = Vec::new();
    for line in xml.lines().map(str::trim_start) {
        if line.starts_with("<object ") {
            objects.push((
                attribute(line, "name").unwrap(),
                attribute(line, "pindex"),
                0,
            ));
        } else if line.starts_with("<triangle ") && attribute(line, "p1").as_deref() == Some("0") {
            objects.last_mut().unwrap().2 += 1;
        }
    }
    assert_eq!(objects.len(), 3, "{objects:?}");
    assert_eq!(objects[0], ("Pin".to_string(), Some("0".to_string()), 0));
    assert_eq!(objects[1], ("Plain".to_string(), None, 0));
    assert_eq!(objects[2].0, "Boolean1");
    assert_eq!(objects[2].1.as_deref(), Some("1"));
    assert!(objects[2].2 >= 8, "the wall of the hole: {objects:?}");
    // STEP: the pin's solid, the cut plate's solid and the wall of its
    // hole (its own colour) are styled; the plain block is not.
    run(&mut s, "file.export", json!({"path": "coloured.step"}));
    let step = std::fs::read_to_string(dir.join("coloured.step")).unwrap();
    assert_eq!(step.matches("STYLED_ITEM(").count(), 3, "{step}");
    assert_eq!(step.matches("=COLOUR_RGB(").count(), 3);
    std::fs::remove_dir_all(&dir).unwrap();
}
