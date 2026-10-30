//! Text in sketches (`sketch.text`, `sketch.textOnCurve`) and fonts
//! (`font.add`, `font.list`), checked against oracles that do not go
//! through the kernel: the glyph outlines' own areas (the font file read
//! by `craft3d-font`), advances and kerning, the tangent of a circle, and
//! closed-form scaling. The built-in fonts are fetched by `cargo xtask
//! fonts` (never committed).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{built, error, failed, inspect, run, session, workdir};
use craft3d_engine::Session;
use craft3d_font::{FontFile, Metrics, filled_outline};
use serde_json::{Value, json};
use std::path::PathBuf;

fn fonts_dir() -> PathBuf {
    match std::env::var_os("CRAFT3D_FONTS") {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts"),
    }
}

fn font_bytes(file: &str) -> Vec<u8> {
    let path = fonts_dir().join(file);
    std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "{}: {e}; fetch the built-in fonts with `cargo xtask fonts`",
            path.display()
        )
    })
}

/// The area a text's glyphs fill, from the font file alone: the sum of the
/// filled outlines of its characters, scaled to a cap height of `size`.
fn text_area(file: &str, text: &str, size: f64) -> f64 {
    let bytes = font_bytes(file);
    let f = FontFile::parse(&bytes).unwrap();
    let s = size / f.cap_height();
    text.chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| filled_outline(&f, f.glyph(c).unwrap()).unwrap().area() * s * s)
        .sum()
}

fn new_document(s: &mut Session, k: f64) {
    run(s, "file.new", json!({"units": "mm", "tolerance": 1e-6 * k}));
}

/// The total volume of the bodies feature `name` made, each valid.
fn volume_of(s: &mut Session, name: &str) -> f64 {
    let doc = inspect(s);
    let id = doc["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == name)
        .unwrap_or_else(|| panic!("no feature {name}"))["id"]
        .clone();
    let bodies: Vec<&Value> = doc["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|b| b["feature"] == id)
        .collect();
    assert!(!bodies.is_empty(), "{name} made no body: {doc}");
    bodies
        .iter()
        .map(|b| {
            assert_eq!(b["valid"], true, "{b}");
            b["volume"].as_f64().unwrap()
        })
        .sum()
}

fn deep_check(s: &mut Session) {
    let check = run(s, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
}

fn text_entity(s: &mut Session, sketch: &str) -> Value {
    let inspected = run(s, "sketch.inspect", json!({ "sketch": sketch }));
    inspected["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["type"] == "text")
        .unwrap_or_else(|| panic!("no text: {inspected}"))
        .clone()
}

#[test]
fn an_extruded_text_is_valid_watertight_and_fills_its_glyphs() {
    for (font, file) in [
        ("sans", "NotoSans-Regular.otf"),
        ("mono", "JetBrainsMono-Regular.ttf"),
    ] {
        let dir = workdir("text", font);
        let mut s = common::checked::session_in(&dir);
        new_document(&mut s, 1.0);
        run(&mut s, "sketch.create", json!({"name": "S"}));
        let made = run(
            &mut s,
            "sketch.text",
            json!({"sketch": "S", "text": "Hola 08&B", "font": font, "size": 10}),
        );
        assert_eq!(made["status"], "ok", "{made}");
        assert_eq!(made["layout"]["text"], "Hola 08&B");
        built(
            &mut s,
            "feature.extrude",
            json!({"sketch": "S", "distance": 2, "name": "Plate"}),
        );
        let expected = text_area(file, "Hola 08&B", 10.0) * 2.0;
        let volume = volume_of(&mut s, "Plate");
        assert!(
            (volume - expected).abs() <= 1e-6 * expected,
            "{font}: volume {volume}, glyph area × depth {expected}"
        );
        deep_check(&mut s);
        // Watertight meshes of it.
        let out = run(&mut s, "file.export", json!({"path": "plate.stl"}));
        assert_eq!(out["closed"], out["bodies"], "{out}");
    }
}

#[test]
fn a_changed_text_regenerates() {
    let mut s = session();
    new_document(&mut s, 1.0);
    run(&mut s, "variable.set", json!({"name": "n", "expr": "12"}));
    run(&mut s, "sketch.create", json!({"name": "S"}));
    run(
        &mut s,
        "sketch.text",
        json!({"sketch": "S", "text": "N{n}", "size": 8}),
    );
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "S", "distance": 1, "name": "Tag"}),
    );
    let before = volume_of(&mut s, "Tag");
    let expected = text_area("NotoSans-Regular.otf", "N12", 8.0);
    assert!(
        (before - expected).abs() <= 1e-6 * expected,
        "{before} vs {expected}"
    );
    run(
        &mut s,
        "variable.set",
        json!({"name": "n", "expr": "3 * 115"}),
    );
    let after = volume_of(&mut s, "Tag");
    let expected = text_area("NotoSans-Regular.otf", "N345", 8.0);
    assert!(
        (after - expected).abs() <= 1e-6 * expected,
        "{after} vs {expected}"
    );
    assert_eq!(text_entity(&mut s, "S")["text"]["layout"]["text"], "N345");
    // An edit of the text itself.
    run(
        &mut s,
        "sketch.textEdit",
        json!({"sketch": "S", "entity": "e2", "text": "M{n / 5}"}),
    );
    assert_eq!(text_entity(&mut s, "S")["text"]["layout"]["text"], "M69");
}

#[test]
fn a_reference_to_a_vanished_glyph_fails_by_name_and_never_retargets() {
    let mut s = session();
    new_document(&mut s, 1.0);
    run(&mut s, "sketch.create", json!({"name": "S"}));
    run(
        &mut s,
        "sketch.text",
        json!({"sketch": "S", "text": "AB", "size": 10}),
    );
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "S", "distance": 2, "name": "Letters"}),
    );
    // The top of the B, a flat face named after the B's contours.
    let names: Vec<String> = inspect(&mut s)["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(common::names_of)
        .collect();
    let top = names
        .iter()
        .find(|n| n.contains("endCap") && n.contains("glyph(1,U+0042)"))
        .unwrap_or_else(|| panic!("no top of the B in {names:?}"))
        .clone();
    built(&mut s, "sketch.create", json!({"name": "OnB", "face": top}));
    // "AB" to "BA": a B is still there, at another place; the reference is
    // to the B at place 1, which is now an A: it fails, by its name.
    run(
        &mut s,
        "sketch.textEdit",
        json!({"sketch": "S", "entity": "e2", "text": "BA"}),
    );
    let state = common::checked::feature(&mut s, "OnB");
    assert_eq!(state["status"], "failed", "{state}");
    let why = state["error"].as_str().unwrap();
    assert!(why.contains("glyph(1,U+0042)"), "{why}");
    // Back to "AB": the same face, found again.
    run(
        &mut s,
        "sketch.textEdit",
        json!({"sketch": "S", "entity": "e2", "text": "AB"}),
    );
    assert_eq!(common::checked::feature(&mut s, "OnB")["status"], "ok");
}

/// The font with `OS/2 fsType` set to `value`.
fn with_fs_type(bytes: &[u8], value: u16) -> Vec<u8> {
    let mut b = bytes.to_vec();
    let n = u16::from_be_bytes([b[4], b[5]]) as usize;
    let os2 = (0..n)
        .find_map(|i| {
            let rec = 12 + 16 * i;
            (&b[rec..rec + 4] == b"OS/2").then(|| {
                u32::from_be_bytes([b[rec + 8], b[rec + 9], b[rec + 10], b[rec + 11]]) as usize
            })
        })
        .unwrap();
    b[os2 + 8..os2 + 10].copy_from_slice(&value.to_be_bytes());
    b
}

#[test]
fn a_font_that_forbids_embedding_is_not_kept() {
    let dir = workdir("text", "restricted");
    let restricted = with_fs_type(&font_bytes("JetBrainsMono-Regular.ttf"), 0x0002);
    std::fs::write(dir.join("restricted.ttf"), &restricted).unwrap();
    let mut s = common::checked::session_in(&dir);
    new_document(&mut s, 1.0);
    let added = run(
        &mut s,
        "font.add",
        json!({"path": "restricted.ttf", "name": "secret"}),
    );
    assert_eq!(added["embedding"], "restricted", "{added}");
    assert_eq!(added["kept"], false);
    assert!(added["warning"].as_str().unwrap().contains("restricted"));
    run(&mut s, "sketch.create", json!({"name": "S"}));
    run(
        &mut s,
        "sketch.text",
        json!({"sketch": "S", "text": "Hi", "font": "secret", "size": 5}),
    );
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "S", "distance": 1, "name": "E"}),
    );
    run(&mut s, "file.save", json!({"path": "restricted.3dcraft"}));
    // The file names the font but holds no bytes of it.
    let file = std::fs::read(dir.join("restricted.3dcraft")).unwrap();
    assert!(file.len() < 100_000, "{} bytes", file.len());
    run(&mut s, "file.open", json!({"path": "restricted.3dcraft"}));
    let list = run(&mut s, "font.list", json!({}));
    let doc_fonts = list["document"].as_array().unwrap();
    assert_eq!(doc_fonts[0]["name"], "secret");
    assert_eq!(doc_fonts[0]["kept"], false);
    assert_eq!(doc_fonts[0]["available"], false);
    let state = common::checked::feature(&mut s, "S");
    assert_eq!(state["status"], "failed", "{state}");
    assert!(
        state["error"]
            .as_str()
            .unwrap()
            .contains("needs font `secret`"),
        "{state}"
    );
    // Given again, it is there for the session.
    run(
        &mut s,
        "font.add",
        json!({"path": "restricted.ttf", "name": "secret"}),
    );
    assert_eq!(common::checked::feature(&mut s, "E")["status"], "ok");
    // A font that may be kept is kept, and reopens the same.
    std::fs::write(
        dir.join("mono.ttf"),
        font_bytes("JetBrainsMono-Regular.ttf"),
    )
    .unwrap();
    let added = run(&mut s, "font.add", json!({"path": "mono.ttf", "name": "m"}));
    assert_eq!(added["kept"], true, "{added}");
}

#[test]
fn text_on_a_circle_keeps_each_glyph_baseline_tangent() {
    let mut s = session();
    new_document(&mut s, 1.0);
    run(&mut s, "sketch.create", json!({"name": "Ring"}));
    let circle = run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Ring", "center": [0, 0], "radius": 30, "construction": true}),
    );
    let id = circle["entity"].clone();
    let r = 30.0;
    for (side, offset) in [("left", 2.0), ("right", 1.5)] {
        let made = run(
            &mut s,
            "sketch.textOnCurve",
            json!({"sketch": "Ring", "curve": id, "text": "AROUND 360", "size": 5, "offset": offset, "side": side, "start": 10}),
        );
        assert_eq!(made["status"], "ok", "{made}");
        let entity = made["entity"].as_str().unwrap().to_string();
        let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "Ring"}));
        let text = inspected["entities"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["id"] == entity.as_str())
            .unwrap()["text"]
            .clone();
        // Left of a counter-clockwise circle is inside it.
        let sign = if side == "left" { -1.0 } else { 1.0 };
        for g in text["glyphs"].as_array().unwrap() {
            let o = [
                g["origin"][0].as_f64().unwrap(),
                g["origin"][1].as_f64().unwrap(),
            ];
            let d = [
                g["direction"][0].as_f64().unwrap(),
                g["direction"][1].as_f64().unwrap(),
            ];
            let half = g["advance"].as_f64().unwrap() / 2.0;
            // The middle of the glyph's baseline.
            let m = [o[0] + d[0] * half, o[1] + d[1] * half];
            let dist = m[0].hypot(m[1]);
            // On the circle grown (left, outside) or shrunk (right, inside)
            // by the offset ...
            assert!((dist - (r + sign * offset)).abs() < 1e-9, "{g}: {dist}");
            // ... and the baseline along the circle's tangent there.
            let radial = [m[0] / dist, m[1] / dist];
            assert!((d[0] * radial[0] + d[1] * radial[1]).abs() < 1e-12, "{g}");
            assert!((d[0].hypot(d[1]) - 1.0).abs() < 1e-12);
        }
    }
    // A glyph on the curve keeps its shape: the area of the closed text
    // set on the circle, extruded, is the glyphs' area.
    run(&mut s, "sketch.create", json!({"name": "Disc"}));
    let c = run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Disc", "center": [0, 0], "radius": 30, "construction": true}),
    );
    run(
        &mut s,
        "sketch.textOnCurve",
        json!({"sketch": "Disc", "curve": c["entity"], "text": "RING", "size": 6}),
    );
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Disc", "distance": 1, "name": "RingText"}),
    );
    let expected = text_area("NotoSans-Regular.otf", "RING", 6.0);
    let v = volume_of(&mut s, "RingText");
    assert!((v - expected).abs() <= 1e-6 * expected, "{v} vs {expected}");
}

#[test]
fn text_scales_from_a_thousandth_to_a_thousand() {
    let base = text_area("NotoSans-Regular.otf", "Ag8", 10.0) * 2.0;
    for k in [1e-3, 1.0, 1e3] {
        let mut s = session();
        new_document(&mut s, k);
        let name = format!("T{}", text.len());
        run(&mut s, "sketch.create", json!({ "name": name }));
        let made = run(
            &mut s,
            "sketch.text",
            json!({"sketch": name, "text": text, "size": size}),
        );
        assert_eq!(made["status"], "failed", "{made}");
    }
    for params in [
        json!({}),
        json!({"path": "nope.ttf"}),
        json!({"builtin": "stroke"}),
        json!({"builtin": "sans", "path": "x"}),
        json!({"builtin": "sans", "name": ""}),
    ] {
        error(&mut s, "font.add", params);
    }
    for params in [
        json!({"sketch": "S", "curve": "e99", "text": "x", "size": 5}),
        json!({"sketch": "S", "text": "x", "size": 5}),
    ] {
        error(&mut s, "sketch.textOnCurve", params);
    }
    error(
        &mut s,
        "sketch.textEdit",
        json!({"sketch": "S", "entity": "e1", "text": "x"}),
    );
    // A text too long for an open curve fails with the reason.
    run(&mut s, "sketch.create", json!({"name": "L"}));
    let line = run(
        &mut s,
        "sketch.line",
        json!({"sketch": "L", "from": [0, 0], "to": [10, 0]}),
    );
    let made = run(
        &mut s,
        "sketch.textOnCurve",
        json!({"sketch": "L", "curve": line["entity"], "text": "far too long for it", "size": 5}),
    );
    assert_eq!(made["status"], "failed", "{made}");
    assert!(
        made["error"].as_str().unwrap().contains("runs off"),
        "{made}"
    );
}

#[test]
fn texts_and_fonts_survive_a_save_and_reopen() {
    let dir = workdir("text", "reopen");
    let mut s = common::checked::session_in(&dir);
    new_document(&mut s, 1.0);
    run(&mut s, "sketch.create", json!({"name": "S"}));
    run(
        &mut s,
        "sketch.text",
        json!({"sketch": "S", "text": "Saved {{1}}", "size": 6, "align": "center", "anchor": "middle", "width": 40, "lineSpacing": 1.2}),
    );
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "S", "distance": 1, "name": "E"}),
    );
    let before = volume_of(&mut s, "E");
    run(&mut s, "file.save", json!({"path": "t.3dcraft"}));
    run(&mut s, "file.open", json!({"path": "t.3dcraft"}));
    let after = volume_of(&mut s, "E");
    assert_eq!(before.to_bits(), after.to_bits());
    assert_eq!(
        text_entity(&mut s, "S")["text"]["layout"]["text"],
        "Saved {1}"
    );
    let list = run(&mut s, "font.list", json!({}));
    assert_eq!(list["document"][0]["name"], "sans");
    assert_eq!(list["document"][0]["kept"], true);
}
