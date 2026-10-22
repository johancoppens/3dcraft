//! `file.export {bodies}`: only the bodies named, to STEP, STL, 3MF and
//! OBJ, with their names and colours; every body without it, as before.
//! Read back by closed form: a block of 10 × 10 × 5 (500), a cylinder of
//! radius 2 and height 5 (20π), a cube of 3 (27).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::run;
use common::{error, session_in};
use craft3d_engine::{Session, Workspace};
use serde_json::json;
use std::f64::consts::PI;
use std::path::Path;

const BLOCK: f64 = 500.0;
const PIN: f64 = 20.0 * PI;

/// Block (red), Pin beside it, Cube suppressed, and Tool taken by a cut
/// out of Slab: four bodies there are (Block, Pin, Slab's cut), two names
/// that have none.
fn parts(s: &mut Session) {
    run(s, "file.new", json!({"units": "mm"}));
    run(
        s,
        "feature.block",
        json!({"size": [10, 10, 5], "name": "Block"}),
    );
    run(
        s,
        "body.appearance",
        json!({"body": "Block", "color": "#c8102e"}),
    );
    run(
        s,
        "feature.cylinder",
        json!({"radius": 2, "height": 5, "position": [20, 0, 0], "name": "Pin"}),
    );
    run(
        s,
        "feature.block",
        json!({"size": [3, 3, 3], "position": [40, 0, 0], "name": "Cube"}),
    );
    run(s, "feature.suppress", json!({"feature": "Cube"}));
    run(
        s,
        "feature.block",
        json!({"size": [10, 10, 2], "position": [60, 0, 0], "name": "Slab"}),
    );
    run(
        s,
        "feature.block",
        json!({"size": [2, 2, 4], "position": [64, 4, -1], "name": "Tool"}),
    );
    run(
        s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Slab", "tool": "Tool", "name": "Cut"}),
    );
}

/// The volume a binary STL encloses, by the divergence theorem.
fn stl_volume(bytes: &[u8]) -> (usize, f64) {
    let count = u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize;
    assert_eq!(bytes.len(), 84 + 50 * count);
    let f = |at: usize| f64::from(f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()));
    let mut volume = 0.0;
    for t in 0..count {
        let base = 84 + 50 * t + 12;
        let v = |i: usize| [f(base + 12 * i), f(base + 12 * i + 4), f(base + 12 * i + 8)];
        let (a, b, c) = (v(0), v(1), v(2));
        volume += (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
            + a[2] * (b[0] * c[1] - b[1] * c[0]))
            / 6.0;
    }
    (count, volume)
}

/// The volume of the STEP file at `path`, imported into a new document.
fn step_volume(path: &Path) -> (usize, f64) {
    let mut reader = Session::new(Workspace::unrestricted());
    reader.execute("file.new", &json!({})).unwrap();
    reader
        .execute("file.import", &json!({"path": path.to_str().unwrap()}))
        .unwrap();
    let inspected = reader.execute("document.inspect", &json!({})).unwrap();
    let bodies = inspected["bodies"].as_array().unwrap().len();
    let measured = reader.execute("document.measure", &json!({})).unwrap();
    (bodies, measured["volume"].as_f64().unwrap())
}

fn close(actual: f64, expected: f64, relative: f64) {
    assert!(
        (actual - expected).abs() <= relative * expected,
        "{actual} is not {expected} within {relative}"
    );
}

#[test]
fn only_the_named_bodies_are_exported_to_every_body_format() {
    let dir = common::workdir("export-bodies", "formats");
    let mut s = session_in(&dir);
    parts(&mut s);

    // STEP: the pin alone, by name, read back exactly.
    let result = run(
        &mut s,
        "file.export",
        json!({"path": "pin.step", "bodies": ["Pin"]}),
    );
    assert_eq!(result["bodies"], 1, "{result}");
    let step = std::fs::read_to_string(dir.join("pin.step")).unwrap();
    assert!(step.contains("'Pin'"), "the body's name");
    assert!(!step.contains("'Block'"));
    let (bodies, volume) = step_volume(&dir.join("pin.step"));
    assert_eq!(bodies, 1);
    close(volume, PIN, 1e-6);
    // The block by its id: its colour comes along.
    let result = run(
        &mut s,
        "file.export",
        json!({"path": "block.step", "bodies": ["F1"]}),
    );
    assert_eq!(result["bodies"], 1, "{result}");
    let step = std::fs::read_to_string(dir.join("block.step")).unwrap();
    assert_eq!(step.matches("=COLOUR_RGB(").count(), 1, "{step}");
    close(step_volume(&dir.join("block.step")).1, BLOCK, 1e-9);
    // Every body without `bodies`, as before: block, pin and the cut slab.
    let every = run(&mut s, "file.export", json!({"path": "all.step"}));
    assert_eq!(every["bodies"], 3, "{every}");
    close(
        step_volume(&dir.join("all.step")).1,
        BLOCK + PIN + 200.0 - 8.0,
        1e-6,
    );

    // STL: the block by id, a number too; its volume exactly (flat faces).
    for given in [json!("F1"), json!(1), json!("Block")] {
        let result = run(
            &mut s,
            "file.export",
            json!({"path": "block.stl", "bodies": [given]}),
        );
        assert_eq!(
            (result["bodies"].clone(), result["closed"].clone()),
            (json!(1), json!(1))
        );
        let (count, volume) = stl_volume(&std::fs::read(dir.join("block.stl")).unwrap());
        assert_eq!(count as u64, result["triangles"].as_u64().unwrap());
        close(volume, BLOCK, 1e-6);
    }
    // Two bodies, named twice and out of order: each once, in history
    // order; the cut slab by the feature that made its body.
    let result = run(
        &mut s,
        "file.export",
        json!({"path": "two.stl", "bodies": ["Cut", "Block", "Cut"]}),
    );
    assert_eq!(result["bodies"], 2, "{result}");
    close(
        stl_volume(&std::fs::read(dir.join("two.stl")).unwrap()).1,
        BLOCK + 192.0,
        1e-6,
    );

    // 3MF: the objects named, the block in its colour.
    let result = run(
        &mut s,
        "file.export",
        json!({"path": "two.3mf", "bodies": ["Pin", "Block"]}),
    );
    assert_eq!(result["bodies"], 2, "{result}");
    let text = String::from_utf8_lossy(&std::fs::read(dir.join("two.3mf")).unwrap()).to_string();
    let names: Vec<&str> = text
        .lines()
        .filter(|l| l.trim_start().starts_with("<object "))
        .map(|l| {
            let at = l.find(" name=\"").unwrap() + 7;
            &l[at..at + l[at..].find('"').unwrap()]
        })
        .collect();
    assert_eq!(names, ["Block", "Pin"]);
    assert!(text.contains("#C8102E"), "the block's colour");

    // OBJ: the pin alone, one object of that name.
    let result = run(
        &mut s,
        "file.export",
        json!({"path": "pin.obj", "bodies": ["Pin"]}),
    );
    assert_eq!(result["bodies"], 1, "{result}");
    let obj = std::fs::read_to_string(dir.join("pin.obj")).unwrap();
    let objects: Vec<&str> = obj.lines().filter(|l| l.starts_with("o ")).collect();
    assert_eq!(objects, ["o Pin"]);
    assert_eq!(
        obj.lines().filter(|l| l.starts_with("f ")).count() as u64,
        result["triangles"].as_u64().unwrap()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bodies_that_are_not_there_are_refused_saying_which_there_are() {
    let dir = common::workdir("export-bodies", "refused");
    let mut s = session_in(&dir);
    parts(&mut s);
    let refused = |s: &mut Session, path: &str, bodies: serde_json::Value| {
        let message = error(s, "file.export", json!({"path": path, "bodies": bodies}));
        assert!(message.starts_with("file.export: "), "{message}");
        message
    };
    for path in ["x.step", "x.stl", "x.3mf", "x.obj"] {
        let empty = refused(&mut s, path, json!([]));
        assert!(empty.contains("`bodies` is empty"), "{empty}");
        assert!(empty.contains("Block (F1), Pin (F2), Cut (F6)"), "{empty}");
        let unknown = refused(&mut s, path, json!(["Pin", "Nope"]));
        assert!(
            unknown.contains("`bodies.1`: there is no feature `Nope`"),
            "{unknown}"
        );
        let suppressed = refused(&mut s, path, json!(["Cube"]));
        assert!(
            suppressed.contains("F3 (Cube) has no body now: it is suppressed"),
            "{suppressed}"
        );
        let taken = refused(&mut s, path, json!(["Tool"]));
        assert!(taken.contains("F6 (Cut) took it"), "{taken}");
        let made_none = refused(&mut s, path, json!([99]));
        assert!(made_none.contains("no feature `99`"), "{made_none}");
        let wrong = refused(&mut s, path, json!("Block"));
        assert!(wrong.contains("must be a list"), "{wrong}");
        let wrong = refused(&mut s, path, json!([true]));
        assert!(wrong.contains("`bodies.0` must be"), "{wrong}");
    }
    // An outline is not a body file.
    let outline = refused(&mut s, "x.dxf", json!(["Block"]));
    assert!(
        outline.contains("`bodies` is for STEP and mesh files"),
        "{outline}"
    );
    // Nothing was written.
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}
