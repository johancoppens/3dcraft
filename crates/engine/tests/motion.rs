//! Motion over the timeline `t` (docs/design/motion.md): `view.motion`,
//! `view.trace` with its export, `motion.check` and `view.replay` over the
//! motion, held to closed forms.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use common::{session, session_in, workdir};
use craft3d_engine::{COLLISION_COLOR, Session};
use serde_json::{Value, json};
use std::f64::consts::TAU;

fn refused(s: &mut Session, id: &str, params: Value, words: &[&str]) {
    let error = s
        .execute(id, &params)
        .expect_err(&format!("{id} {params}"))
        .to_string();
    assert!(!error.contains("internal error"), "{params}: {error}");
    for word in words {
        assert!(
            error.contains(word),
            "{id} {params}: `{error}` lacks `{word}`"
        );
    }
}

fn number(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("{v} is no number"))
}

fn point(v: &Value) -> [f64; 3] {
    let a = v.as_array().unwrap();
    [number(&a[0]), number(&a[1]), number(&a[2])]
}

/// A crank: a 2 mm cube whose corner goes round a circle of radius 15 mm
/// about the z axis as `t` runs from 0 to 1, and a fixed plate beside it.
fn crank(s: &mut Session) {
    run(s, "file.new", json!({}));
    run(s, "variable.set", json!({"name": "t", "expr": "0"}));
    run(s, "variable.set", json!({"name": "R", "expr": "15 mm"}));
    run(
        s,
        "variable.set",
        json!({"name": "crankAngle", "expr": "360 deg * t"}),
    );
    run(
        s,
        "feature.block",
        json!({"name": "Plate", "size": ["10 mm", "10 mm", "2 mm"], "position": ["-40 mm", "-5 mm", "0 mm"]}),
    );
    run(
        s,
        "feature.block",
        json!({"name": "Crank", "size": ["2 mm", "2 mm", "2 mm"],
               "position": ["R * cos(crankAngle)", "R * sin(crankAngle)", "0 mm"]}),
    );
}

#[test]
fn a_crank_traced_is_its_closed_form_circle() {
    let mut s = session();
    crank(&mut s);
    let edges = run(&mut s, "document.edges", json!({"feature": "Crank"}));
    let vertex = edges["vertices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v.get("name").is_some())
        .unwrap()
        .clone();
    let name = vertex["name"].as_str().unwrap().to_string();
    let rest = point(&vertex["point"]);
    let samples = 41;
    let out = run(
        &mut s,
        "view.trace",
        json!({"point": name, "samples": samples, "name": "Foot"}),
    );
    assert_eq!(out["name"], "Foot");
    let points = out["points"].as_array().unwrap();
    assert_eq!(points.len(), samples);
    // The corner at t is its place at t = 0 moved by the block's
    // position: (R cos θ − R, R sin θ, 0), θ = 2π t.
    let r = 15.0;
    for (i, p) in points.iter().enumerate() {
        let t = i as f64 / (samples - 1) as f64;
        let theta = TAU * t;
        let want = [
            rest[0] + r * theta.cos() - r,
            rest[1] + r * theta.sin(),
            rest[2],
        ];
        let got = point(p);
        for k in 0..3 {
            assert!(
                (got[k] - want[k]).abs() < 1e-9,
                "t = {t}: {got:?} is not {want:?}"
            );
        }
    }
    assert_eq!(out["closed"], true);
    // The polyline's length is that of the inscribed 40-gon.
    let chords = 40.0 * 2.0 * r * (TAU / 80.0).sin();
    assert!((number(&out["length"]) - chords).abs() < 1e-8);
    assert_eq!(point(&out["plane"]["normal"]), [0.0, 0.0, 1.0]);
    // The window draws it.
    let traces = s.traces();
    assert_eq!(traces.len(), 1);
    assert_eq!(traces[0].name, "Foot");
    assert_eq!(traces[0].points.len(), samples);
    // The document did not change.
    assert_eq!(s.document_revision(), Some(5));
    // An edit lets go of the trace: it was a path of another document.
    run(
        &mut s,
        "variable.set",
        json!({"name": "R", "expr": "16 mm"}),
    );
    assert!(s.traces().is_empty());
}

#[test]
fn a_sketch_point_traced_follows_its_formulas() {
    let mut s = session();
    run(&mut s, "file.new", json!({}));
    run(&mut s, "variable.set", json!({"name": "t", "expr": "0"}));
    let sketch = run(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "name": "Path"}),
    );
    let sketch = sketch["feature"].as_str().unwrap_or("Path").to_string();
    let p = run(
        &mut s,
        "sketch.point",
        json!({"sketch": "Path", "at": [10, 0]}),
    );
    let e = p["entity"].as_str().unwrap().to_string();
    run(
        &mut s,
        "sketch.constrain",
        json!({"sketch": "Path", "type": "x", "entities": [e], "value": "10 mm * cos(360 deg * t)"}),
    );
    run(
        &mut s,
        "sketch.constrain",
        json!({"sketch": "Path", "type": "y", "entities": [e], "value": "5 mm * sin(360 deg * t)"}),
    );
    let out = run(
        &mut s,
        "view.trace",
        json!({"point": {"sketch": "Path", "entity": e}, "t": [0, 0.5], "samples": 9}),
    );
    assert_eq!(out["name"], "Trace 1");
    for (i, p) in out["points"].as_array().unwrap().iter().enumerate() {
        let theta = TAU * 0.5 * i as f64 / 8.0;
        let got = point(p);
        assert!((got[0] - 10.0 * theta.cos()).abs() < 1e-7, "{got:?}");
        assert!((got[1] - 5.0 * theta.sin()).abs() < 1e-7, "{got:?}");
        assert!(got[2].abs() < 1e-12);
    }
    assert_eq!(out["closed"], false);
    let _ = sketch;
}

#[test]
fn a_trace_is_exported_in_its_plane_to_svg_and_dxf() {
    let dir = workdir("motion", "trace-export");
    let mut s = session_in(&dir);
    crank(&mut s);
    let edges = run(&mut s, "document.edges", json!({"feature": "Crank"}));
    let name = edges["vertices"][0]["name"].as_str().unwrap().to_string();
    run(
        &mut s,
        "view.trace",
        json!({"point": name, "samples": 33, "name": "Foot"}),
    );
    let svg = run(
        &mut s,
        "file.export",
        json!({"path": "foot.svg", "trace": "Foot"}),
    );
    assert_eq!(svg["paths"], 1);
    assert_eq!(svg["closed"], 1);
    assert_eq!(svg["lines"], 32);
    let text = std::fs::read_to_string(dir.join("foot.svg")).unwrap();
    assert!(text.contains("<path"), "{text}");
    let dxf = run(
        &mut s,
        "file.export",
        json!({"path": "foot.dxf", "trace": "Foot"}),
    );
    assert_eq!(dxf["paths"], 1);
    assert!(
        std::fs::read_to_string(dir.join("foot.dxf"))
            .unwrap()
            .contains("ENTITIES")
    );
    refused(
        &mut s,
        "file.export",
        json!({"path": "x.svg", "trace": "Nope"}),
        &["no trace `Nope`", "Foot"],
    );
    refused(
        &mut s,
        "file.export",
        json!({"path": "x.svg", "trace": "Foot", "sketch": "F1"}),
        &["not"],
    );
    refused(
        &mut s,
        "file.export",
        json!({"path": "x.svg", "trace": 3}),
        &["`trace`"],
    );
    refused(
        &mut s,
        "file.export",
        json!({"path": "x.stl", "trace": "Foot"}),
        &["trace"],
    );
}

#[test]
fn showing_a_moment_rebuilds_only_what_t_drives_and_keeps_the_document() {
    let mut s = session();
    crank(&mut s);
    let revision = s.document_revision();
    let out = run(&mut s, "view.motion", json!({"t": 0.25}));
    assert_eq!(out["t"], 0.25);
    assert_eq!(out["rebuilt"], json!(["F2"]));
    assert_eq!(out["timeline"]["value"], 0.0);
    assert_eq!(s.document_revision(), revision);
    assert_eq!(s.motion().unwrap().t, 0.25);
    // The scene shows the crank a quarter turn on.
    let scene = s.scene(None).unwrap().unwrap();
    let crank = scene.bodies.iter().find(|b| b.feature == "F2").unwrap();
    let ys: Vec<f64> = crank.mesh.positions.iter().map(|p| p.y).collect();
    let low = ys.iter().copied().fold(f64::INFINITY, f64::min);
    assert!((low - 15.0).abs() < 1e-4, "{low}");
    // Measures still see the document itself.
    let inspect = run(&mut s, "document.inspect", json!({}));
    assert!(inspect.to_string().contains("\"t\""));
    // Null shows the document again.
    let back = run(&mut s, "view.motion", json!({"t": null}));
    assert_eq!(back["t"], Value::Null);
    assert!(s.motion().is_none());
    // An edit keeps a motion shown and makes it again from the edit.
    run(&mut s, "view.motion", json!({"t": 0.5}));
    run(
        &mut s,
        "variable.set",
        json!({"name": "R", "expr": "20 mm"}),
    );
    assert_eq!(s.motion().unwrap().t, 0.5);
    // Renaming the timeline away lets go of it.
    run(
        &mut s,
        "variable.rename",
        json!({"name": "t", "to": "time"}),
    );
    assert!(s.motion().is_none());
}

#[test]
fn colliding_bodies_are_drawn_red_in_the_scene_and_the_picture() {
    let mut s = session();
    run(&mut s, "file.new", json!({}));
    run(&mut s, "variable.set", json!({"name": "t", "expr": "0"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Wall", "size": ["10 mm", "10 mm", "10 mm"], "position": ["-10 mm", "0 mm", "0 mm"]}),
    );
    // The slider's left side is at 15 mm + 20 mm cos(2π t): into the wall
    // for cos < −0.75.
    run(
        &mut s,
        "feature.block",
        json!({"name": "Slider", "size": ["10 mm", "10 mm", "10 mm"],
               "position": ["15 mm + 20 mm * cos(360 deg * t)", "0 mm", "0 mm"]}),
    );
    let clear = run(&mut s, "view.motion", json!({"t": 0.0, "collisions": true}));
    assert_eq!(clear["collisions"], json!([]));
    let red_faces = |s: &mut Session| {
        let scene = s.scene(None).unwrap().unwrap();
        scene
            .face_colors
            .iter()
            .filter(|(_, c)| *c == COLLISION_COLOR)
            .count()
    };
    assert_eq!(red_faces(&mut s), 0);
    let hit = run(&mut s, "view.motion", json!({"t": 0.5}));
    let pairs = hit["collisions"].as_array().unwrap();
    assert_eq!(pairs.len(), 1);
    assert_eq!(pairs[0]["aName"], "Wall");
    assert_eq!(pairs[0]["bName"], "Slider");
    // At t = 0.5 the slider reaches 5 mm into the wall: 5 x 10 x 10.
    assert!((number(&pairs[0]["volume"]) - 500.0).abs() < 1e-6);
    assert_eq!(red_faces(&mut s), 12);
    assert_eq!(
        s.motion().unwrap().collisions,
        vec![("Wall".to_string(), "Slider".to_string())]
    );
    // The CPU renderer (the oracle) draws them red.
    let picture = run(&mut s, "view.render", json!({"width": 160, "height": 120}));
    let reds = red_pixels(&picture);
    assert!(reds > 500, "{reds} red pixels");
    run(&mut s, "view.motion", json!({"t": 0.0}));
    let picture = run(&mut s, "view.render", json!({"width": 160, "height": 120}));
    assert_eq!(red_pixels(&picture), 0);
    // Switched off, nothing is red.
    run(
        &mut s,
        "view.motion",
        json!({"t": 0.5, "collisions": false}),
    );
    assert_eq!(red_faces(&mut s), 0);
}

fn red_pixels(result: &Value) -> usize {
    let bytes = base64(result["base64"].as_str().unwrap());
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().unwrap();
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).unwrap();
    buffer.truncate(info.buffer_size());
    buffer
        .chunks(3)
        .filter(|p| {
            p[0] > 120
                && u32::from(p[1]) * 2 < u32::from(p[0])
                && u32::from(p[2]) * 2 < u32::from(p[0])
        })
        .count()
}

fn base64(text: &str) -> Vec<u8> {
    let value = |c: u8| match c {
        b'A'..=b'Z' => u32::from(c - b'A'),
        b'a'..=b'z' => u32::from(c - b'a') + 26,
        b'0'..=b'9' => u32::from(c - b'0') + 52,
        b'+' => 62,
        b'/' => 63,
        _ => 0,
    };
    let clean: Vec<u8> = text.bytes().filter(|c| *c != b'=').collect();
    let mut out = Vec::new();
    for chunk in clean.chunks(4) {
        let mut n = 0u32;
        for (i, c) in chunk.iter().enumerate() {
            n |= value(*c) << (18 - 6 * i);
        }
        let bytes = n.to_be_bytes();
        out.extend_from_slice(&bytes[1..chunk.len()]);
    }
    out
}

/// A wall and a slider whose gap is 15 mm + `swing` cos(2π t).
fn swinging(s: &mut Session, swing: &str) {
    run(s, "file.new", json!({}));
    run(s, "variable.set", json!({"name": "t", "expr": "0"}));
    run(
        s,
        "feature.block",
        json!({"name": "Wall", "size": ["10 mm", "10 mm", "10 mm"], "position": ["-10 mm", "0 mm", "0 mm"]}),
    );
    run(
        s,
        "feature.block",
        json!({"name": "Slider", "size": ["10 mm", "10 mm", "10 mm"],
               "position": [format!("15 mm + {swing} * cos(360 deg * t)"), "0 mm", "0 mm"]}),
    );
}

#[test]
fn clearance_over_the_motion_is_its_closed_form() {
    let mut s = session();
    swinging(&mut s, "10 mm");
    run(
        &mut s,
        "assert.add",
        json!({"name": "gap", "measure": "clearance", "from": "Wall", "to": "Slider", "min": "6 mm"}),
    );
    let before = s.document_revision();
    let out = run(
        &mut s,
        "motion.check",
        json!({"samples": 33, "details": true}),
    );
    assert_eq!(s.document_revision(), before);
    assert_eq!(out["ok"], false);
    let gap = &out["assertions"][0];
    assert_eq!(gap["name"], "gap");
    assert_eq!(gap["status"], "failed");
    // 15 + 10 cos(2π t) < 6 first at the sample t = 14/32.
    assert_eq!(gap["firstFailure"]["t"], 14.0 / 32.0);
    // Samples 14 to 18 fail: cos(2π k/32) < −0.9 for k = 14..=18.
    assert_eq!(gap["failed"], 5);
    assert_eq!(gap["min"]["t"], 0.5);
    assert!((number(&gap["min"]["value"]) - 5.0).abs() < 1e-9);
    assert!((number(&gap["max"]["value"]) - 25.0).abs() < 1e-9);
    for sample in gap["samples"].as_array().unwrap() {
        let t = number(&sample["t"]);
        let want = 15.0 + 10.0 * (TAU * t).cos();
        assert!(
            (number(&sample["measured"]) - want).abs() < 1e-9,
            "{sample}"
        );
    }
    // The document's own check is at its own t, and passes.
    let list = run(&mut s, "assert.list", json!({}));
    assert_eq!(list["assertions"][0]["status"], "passed");
}

#[test]
fn the_first_collision_over_the_motion_comes_from_any_measure() {
    let mut s = session();
    swinging(&mut s, "20 mm");
    let out = run(
        &mut s,
        "motion.check",
        json!({"measure": {"measure": "overlap", "of": "Wall", "to": "Slider", "is": false}, "samples": 33}),
    );
    let overlap = &out["assertions"][0];
    assert_eq!(overlap["status"], "failed");
    // 15 + 20 cos(2π t) < 0 first at t = 13/32 (cos 146.25° = −0.83).
    assert_eq!(overlap["firstFailure"]["t"], 13.0 / 32.0);
    // The measure is not kept in the document.
    let list = run(&mut s, "assert.list", json!({}));
    assert_eq!(list["assertions"], json!([]));
    // Over a range clear of it, it passes.
    let early = run(
        &mut s,
        "motion.check",
        json!({"measure": {"measure": "overlap", "of": "Wall", "to": "Slider", "is": false}, "t": [0, 0.3], "samples": 4}),
    );
    assert_eq!(early["ok"], true);
}

#[test]
fn the_motion_replays_as_frames() {
    let dir = workdir("motion", "replay");
    let mut s = session_in(&dir);
    swinging(&mut s, "20 mm");
    let out = run(
        &mut s,
        "view.replay",
        json!({"dir": "frames", "width": 64, "height": 48, "motion": {"frames": 5, "collisions": true}}),
    );
    assert_eq!(out["frames"], 5);
    let frames = out["motion"]["frames"].as_array().unwrap();
    let ts: Vec<f64> = frames.iter().map(|f| number(&f["t"])).collect();
    assert_eq!(ts, vec![0.0, 0.25, 0.5, 0.75, 1.0]);
    let collided: Vec<u64> = frames
        .iter()
        .map(|f| f["collisions"].as_u64().unwrap())
        .collect();
    assert_eq!(collided, vec![0, 0, 1, 0, 0]);
    for k in 0..5 {
        assert!(dir.join(format!("frames/frame-{k:05}.png")).exists());
    }
    let captions = std::fs::read_to_string(dir.join("frames/captions.srt")).unwrap();
    assert!(captions.contains("t = 0.5"), "{captions}");
    refused(
        &mut s,
        "view.replay",
        json!({"dir": "frames", "motion": {}, "from": "F1"}),
        &["`from`", "motion"],
    );
    refused(
        &mut s,
        "view.replay",
        json!({"dir": "frames", "motion": {"frames": 1}}),
        &["frames"],
    );
    refused(
        &mut s,
        "view.replay",
        json!({"dir": "frames", "motion": 3}),
        &["motion"],
    );
}

#[test]
fn without_a_timeline_motion_is_refused_with_how_to_add_one() {
    let mut s = session();
    run(&mut s, "file.new", json!({}));
    run(
        &mut s,
        "feature.block",
        json!({"size": ["1 mm", "1 mm", "1 mm"]}),
    );
    refused(
        &mut s,
        "view.motion",
        json!({"t": 0.5}),
        &["no timeline", "variable.set"],
    );
    refused(
        &mut s,
        "view.trace",
        json!({"point": "x"}),
        &["no timeline"],
    );
    refused(&mut s, "motion.check", json!({}), &["no timeline"]);
    // A thickness called t is not the timeline.
    run(&mut s, "variable.set", json!({"name": "t", "expr": "3 mm"}));
    refused(
        &mut s,
        "view.motion",
        json!({"t": 0.5}),
        &["not the timeline", "rename"],
    );
    // Asking without t says how it stands and changes nothing.
    let out = run(&mut s, "view.motion", json!({}));
    assert_eq!(out["t"], Value::Null);
    assert!(
        out["timeline"]["error"]
            .as_str()
            .unwrap()
            .contains("length")
    );
}

#[test]
fn bad_parameters_are_errors() {
    let mut s = session();
    crank(&mut s);
    for t in [
        json!(-0.1),
        json!(1.5),
        json!("half"),
        json!([0.5]),
        json!(f64::MAX),
    ] {
        refused(&mut s, "view.motion", json!({"t": t}), &["`t`"]);
    }
    refused(
        &mut s,
        "view.motion",
        json!({"collisions": 3}),
        &["collisions"],
    );
    refused(&mut s, "view.motion", json!({"speed": 3}), &["speed"]);
    refused(&mut s, "view.trace", json!({}), &["point"]);
    refused(&mut s, "view.trace", json!({"point": 3}), &["point"]);
    refused(
        &mut s,
        "view.trace",
        json!({"point": "F9:nothing"}),
        &["t = 0"],
    );
    refused(
        &mut s,
        "view.trace",
        json!({"point": {"sketch": "Nope", "entity": "e1"}}),
        &["sketch"],
    );
    refused(
        &mut s,
        "view.trace",
        json!({"point": {"sketch": "Plate"}}),
        &["entity"],
    );
    refused(
        &mut s,
        "view.trace",
        json!({"point": {"sketch": "Plate", "entity": "e1"}}),
        &["not a solved sketch"],
    );
    refused(
        &mut s,
        "view.trace",
        json!({"point": "x", "samples": 1}),
        &["samples"],
    );
    refused(
        &mut s,
        "view.trace",
        json!({"point": "x", "samples": 100000}),
        &["samples"],
    );
    refused(
        &mut s,
        "view.trace",
        json!({"point": "x", "t": [0, 2]}),
        &["`t`"],
    );
    refused(
        &mut s,
        "view.trace",
        json!({"point": "x", "t": "all"}),
        &["`t`"],
    );
    refused(
        &mut s,
        "view.trace",
        json!({"clear": "Nope"}),
        &["no trace"],
    );
    refused(&mut s, "view.trace", json!({"clear": 3}), &["clear"]);
    refused(
        &mut s,
        "view.trace",
        json!({"clear": true, "point": "x"}),
        &["clear"],
    );
    let cleared = run(&mut s, "view.trace", json!({"clear": true}));
    assert_eq!(cleared["removed"], 0);
    refused(&mut s, "motion.check", json!({}), &["no assertions"]);
    refused(
        &mut s,
        "motion.check",
        json!({"assertions": ["nope"]}),
        &["no assertion `nope`"],
    );
    refused(
        &mut s,
        "motion.check",
        json!({"assertions": "nope"}),
        &["list"],
    );
    refused(
        &mut s,
        "motion.check",
        json!({"assertions": [3]}),
        &["list"],
    );
    refused(&mut s, "motion.check", json!({"measure": 3}), &["measure"]);
    refused(
        &mut s,
        "motion.check",
        json!({"measure": {"name": "x", "measure": "volume"}}),
        &["no name"],
    );
    refused(
        &mut s,
        "motion.check",
        json!({"measure": {"measure": "nonsense"}}),
        &["measure"],
    );
    refused(
        &mut s,
        "motion.check",
        json!({"measure": {"measure": "volume", "min": "1 mm^3"}, "samples": 0}),
        &["samples"],
    );
}
