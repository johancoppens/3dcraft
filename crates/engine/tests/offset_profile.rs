//! The offset of the profiles of a sketch (`feature.offsetProfile`),
//! through the commands: a finger edge made by a sketch pattern offset by
//! half a kerf follows the pattern's count with the closed-form area and
//! names that keep their meaning; rectangles at three scales; what is
//! refused; save, reopen, undo; the beam path exported as DXF.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Access, Session, Workspace};
use serde_json::{Value, json};
use std::f64::consts::PI;
use std::path::PathBuf;

fn workdir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("offset-profile-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn session_in(dir: &std::path::Path) -> Session {
    Session::new(Workspace {
        read: Access::Root(dir.to_path_buf()),
        write: Access::Root(dir.to_path_buf()),
    })
}

fn session() -> Session {
    Session::new(Workspace {
        read: Access::None,
        write: Access::None,
    })
}

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
    match session.execute(id, &params) {
        Ok(value) => panic!("{id} {params} should fail, returned {value}"),
        Err(e) => {
            assert!(!e.message().contains("internal error"), "{id}: {e}");
            e.to_string()
        }
    }
}

fn str_of(value: &Value) -> String {
    value
        .as_str()
        .unwrap_or_else(|| panic!("not a string: {value}"))
        .to_string()
}

fn document(session: &mut Session) -> Value {
    run(session, "document.inspect", json!({}))
}

fn feature(session: &mut Session, name: &str) -> Value {
    let document = document(session);
    document["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == name)
        .unwrap_or_else(|| panic!("no feature {name}"))
        .clone()
}

fn status(session: &mut Session, name: &str) -> String {
    let feature = feature(session, name);
    match (&feature["status"], &feature["error"]) {
        (Value::String(s), Value::String(e)) => format!("{s}: {e}"),
        (Value::String(s), _) => s.clone(),
        (other, _) => other.to_string(),
    }
}

fn body(session: &mut Session, name: &str) -> Value {
    let document = document(session);
    document["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == name)
        .unwrap_or_else(|| panic!("no body {name} in {document}"))
        .clone()
}

fn face_names(body: &Value) -> Vec<String> {
    body["faceNames"]
        .as_array()
        .unwrap()
        .iter()
        .map(str_of)
        .collect()
}

fn assert_close(got: f64, want: f64, what: &str) {
    let scale = want.abs().max(1e-300);
    assert!(
        ((got - want) / scale).abs() < 1e-9,
        "{what}: {got} instead of {want}"
    );
}

/// A sketch `Edge`: a plate `length` by `height` whose bottom edge is a
/// chained run of `n` periods of fingers `depth` deep and half a period
/// wide, the period found by the solver (as in `sketch_pattern.rs`).
/// Returns the seed's four lines.
fn finger_sketch(s: &mut Session, length: f64, height: f64, depth: f64) -> Vec<String> {
    run(s, "sketch.create", json!({"name": "Edge"}));
    let p = length / 7.0;
    let run_of = run(
        s,
        "sketch.polyline",
        json!({"sketch": "Edge", "points": [[0, 0], [0, -depth], [p / 2.0, -depth], [p / 2.0, 0], [p, 0]]}),
    );
    let points: Vec<String> = run_of["points"]
        .as_array()
        .unwrap()
        .iter()
        .map(str_of)
        .collect();
    let lines: Vec<String> = run_of["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(str_of)
        .collect();
    let (a, b) = (points[0].clone(), points[4].clone());
    let constrain = |s: &mut Session, kind: &str, entities: Value, value: Option<String>| {
        let mut params = json!({"sketch": "Edge", "type": kind, "entities": entities});
        if let Some(value) = value {
            params["value"] = json!(value);
        }
        run(s, "sketch.constrain", params)
    };
    constrain(s, "fixed", json!([a]), None);
    constrain(s, "vertical", json!([lines[0]]), None);
    constrain(s, "horizontal", json!([lines[1]]), None);
    constrain(s, "vertical", json!([lines[2]]), None);
    constrain(s, "horizontal", json!([lines[3]]), None);
    constrain(s, "distance", json!([lines[0]]), Some(format!("{depth}")));
    constrain(s, "equal", json!([lines[1], lines[3]]), None);
    constrain(s, "horizontal", json!([a, b]), None);
    run(
        s,
        "sketch.pattern",
        json!({"sketch": "Edge", "entities": lines, "count": "n", "chain": [a, b]}),
    );
    let last = format!("{b}[last]");
    let rest = run(
        s,
        "sketch.polyline",
        json!({"sketch": "Edge", "points": [last, [length, height], [0, height], a]}),
    );
    let more: Vec<String> = rest["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(str_of)
        .collect();
    let corner = run(
        s,
        "sketch.point",
        json!({"sketch": "Edge", "at": [length, 0]}),
    );
    let c = str_of(&corner["entity"]);
    constrain(s, "fixed", json!([c]), None);
    constrain(s, "coincident", json!([c, last]), None);
    constrain(s, "vertical", json!([more[0]]), None);
    constrain(s, "horizontal", json!([more[1]]), None);
    constrain(s, "vertical", json!([more[2]]), None);
    lines
}

/// The area of the finger plate offset by `d` with round corners: an
/// orthogonal outline with 2n + 3 convex and 2n − 1 concave corners. A
/// convex corner adds a quarter disc, a concave one takes the d by d
/// square the two strips share.
fn finger_area(length: f64, height: f64, depth: f64, n: f64, d: f64, round: bool) -> f64 {
    let area = length * height + length * depth / 2.0;
    let perimeter = 2.0 * length + 2.0 * height + 2.0 * n * depth;
    let corners = if round {
        (2.0 * n + 3.0) * PI / 4.0 - (2.0 * n - 1.0)
    } else {
        4.0
    };
    area + perimeter * d + corners * d * d
}

#[test]
fn a_finger_edge_offset_by_half_the_kerf_follows_the_count() {
    let (length, height, depth, t) = (120.0, 50.0, 3.0, 3.0);
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "variable.set", json!({"name": "n", "expr": "3"}));
    run(
        &mut s,
        "variable.set",
        json!({"name": "kerf", "expr": "0.2 mm"}),
    );
    let seed = finger_sketch(&mut s, length, height, depth);
    let made = run(
        &mut s,
        "feature.offsetProfile",
        json!({"sketch": "Edge", "distance": "kerf / 2", "name": "Kerf"}),
    );
    assert_eq!(made["status"], "ok", "{made}");
    let offset = str_of(&made["id"]);
    let extruded = run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Kerf", "distance": t, "name": "Panel"}),
    );
    let panel_id = str_of(&extruded["id"]);
    let d = 0.1;
    let mut cap = None;
    for n in [3_u32, 33, 5] {
        run(
            &mut s,
            "variable.set",
            json!({"name": "n", "expr": n.to_string()}),
        );
        assert_eq!(status(&mut s, "Kerf"), "ok", "n = {n}");
        assert_eq!(status(&mut s, "Panel"), "ok", "n = {n}");
        let panel = body(&mut s, "Panel");
        assert_close(
            panel["volume"].as_f64().unwrap(),
            finger_area(length, height, depth, f64::from(n), d, true) * t,
            &format!("n = {n}"),
        );
        let report = feature(&mut s, "Kerf")["offset"].clone();
        assert_eq!(report["loopsBefore"], 1);
        assert_eq!(report["loopsAfter"], 1);
        assert_eq!(report["wholeUpTo"], Value::Null);
        // Names: the kerfed flank of each finger by its origin.
        let names = face_names(&panel);
        for line in &seed {
            let first = format!("{panel_id}:side({offset}:offset({line}))");
            assert!(names.contains(&first), "{first} in {names:?}");
            let last = format!("{panel_id}:side({offset}:offset({line}[{}]))", n - 1);
            assert!(names.contains(&last), "{last} in {names:?}");
            let past = format!("{panel_id}:side({offset}:offset({line}[{n}]))");
            assert!(!names.contains(&past));
        }
        // Each convex corner has its arc, by the two curves in the order
        // the loop runs: the bottom of a finger (seed lines 0 then 1).
        let corner = format!(
            "{panel_id}:side({offset}:offsetCorner({}[2],{}[2]))",
            seed[0], seed[1]
        );
        assert!(names.contains(&corner), "{corner} in {names:?}");
        let caps: Vec<&String> = names.iter().filter(|n| n.contains("Cap")).collect();
        assert_eq!(caps.len(), 2);
        for line in &seed {
            assert!(caps[0].contains(&format!("{line}[*]")), "{caps:?}");
        }
        match &cap {
            None => cap = Some(caps[0].clone()),
            Some(cap) => assert_eq!(cap, caps[0], "the cap keeps its name"),
        }
        // Every name reads back as itself.
        for name in &names {
            let parsed: craft3d_doc::PersistentName = name.parse().unwrap();
            assert_eq!(&parsed.to_string(), name);
        }
    }
    // Sharp corners: the mitre form; kerf 0: the outline itself.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Kerf", "corners": "sharp"}),
    );
    let panel = body(&mut s, "Panel");
    assert_close(
        panel["volume"].as_f64().unwrap(),
        finger_area(length, height, depth, 5.0, d, false) * t,
        "sharp",
    );
    run(&mut s, "variable.set", json!({"name": "kerf", "expr": "0"}));
    let panel = body(&mut s, "Panel");
    assert_close(
        panel["volume"].as_f64().unwrap(),
        finger_area(length, height, depth, 5.0, 0.0, true) * t,
        "kerf 0",
    );
}

#[test]
fn rectangles_with_a_hole_at_three_scales() {
    for scale in [1e-3, 1.0, 1e3] {
        let mut s = session();
        run(&mut s, "file.new", json!({"units": "mm"}));
        run(&mut s, "sketch.create", json!({"name": "Plate"}));
        let (w, h, r) = (10.0 * scale, 6.0 * scale, 1.5 * scale);
        run(
            &mut s,
            "sketch.polyline",
            json!({"sketch": "Plate", "points": [[0, 0], [w, 0], [w, h], [0, h]], "closed": true}),
        );
        run(
            &mut s,
            "sketch.circle",
            json!({"sketch": "Plate", "center": [w / 2.0, h / 2.0], "radius": r}),
        );
        let d = 0.5 * scale;
        run(
            &mut s,
            "feature.offsetProfile",
            json!({"sketch": "Plate", "distance": format!("{}", -d), "name": "Inset"}),
        );
        run(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Inset", "distance": scale, "name": "Body"}),
        );
        let volume = body(&mut s, "Body")["volume"].as_f64().unwrap();
        assert_close(
            volume,
            ((w - 2.0 * d) * (h - 2.0 * d) - PI * (r + d) * (r + d)) * scale,
            &format!("scale {scale}"),
        );
        // Further in: the strips beside the hole go, two regions are
        // left, and the offset says so.
        run(
            &mut s,
            "feature.edit",
            json!({"feature": "Inset", "distance": format!("{}", -1.6 * scale)}),
        );
        assert_eq!(status(&mut s, "Inset"), "ok");
        let report = feature(&mut s, "Inset")["offset"].clone();
        assert_eq!(
            (&report["regionsBefore"], &report["regionsAfter"]),
            (&json!(1), &json!(2)),
            "{report}"
        );
        assert_eq!(
            (&report["loopsBefore"], &report["loopsAfter"]),
            (&json!(2), &json!(2))
        );
        let whole = report["wholeUpTo"].as_f64().unwrap();
        assert!(whole < 0.0 && whole > -1.6 * scale, "{report}");
        // Too far in: nothing is left.
        run(
            &mut s,
            "feature.edit",
            json!({"feature": "Inset", "distance": format!("{}", -3.5 * scale)}),
        );
        let failed = status(&mut s, "Inset");
        assert!(
            failed.starts_with("failed: distance:") && failed.contains("largest distance"),
            "{failed}"
        );
        assert!(status(&mut s, "Body").starts_with("failed"));
    }
}

#[test]
fn an_offset_is_saved_reopened_undone_and_exported() {
    let dir = workdir("files");
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "variable.set", json!({"name": "n", "expr": "4"}));
    finger_sketch(&mut s, 120.0, 50.0, 3.0);
    run(
        &mut s,
        "feature.offsetProfile",
        json!({"sketch": "Edge", "distance": "0.075 mm", "name": "Kerf"}),
    );
    run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Kerf", "distance": 3, "name": "Panel"}),
    );
    let before = document(&mut s);
    run(&mut s, "file.save", json!({"path": "kerf.3dcraft"}));
    let mut again = session_in(&dir);
    run(&mut again, "file.open", json!({"path": "kerf.3dcraft"}));
    let after = document(&mut again);
    assert_eq!(before["bodies"], after["bodies"]);
    // The beam path, as DXF: one closed outline.
    let exported = run(
        &mut s,
        "file.export",
        json!({"path": "kerf.dxf", "sketch": "Kerf"}),
    );
    assert!(dir.join("kerf.dxf").exists(), "{exported}");
    // Undo takes the extrusion, then the offset, away.
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    let features = document(&mut s)["features"].clone();
    assert!(
        features
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["name"] != "Kerf")
    );
}

#[test]
fn bad_params_are_refused() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "sketch.create", json!({"name": "Plate"}));
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Plate", "points": [[0, 0], [10, 0], [10, 6], [0, 6]], "closed": true}),
    );
    run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Plate", "distance": 1, "name": "Body"}),
    );
    for params in [
        json!({"sketch": "Plate"}),
        json!({"sketch": "Plate", "distance": 1, "corners": "square"}),
        json!({"sketch": "Plate", "distance": 1, "corners": 3}),
        json!({"sketch": "Body", "distance": 1}),
        json!({"sketch": "Nothing", "distance": 1}),
        json!({"distance": 1}),
        json!({"sketch": "Plate", "distance": 1, "extra": true}),
    ] {
        error(&mut s, "feature.offsetProfile", params);
    }
    // An offset of an offset, and an offset whose distance is not a
    // length: refused or failed, never a panic.
    run(
        &mut s,
        "feature.offsetProfile",
        json!({"sketch": "Plate", "distance": "1 deg", "name": "Odd"}),
    );
    assert!(status(&mut s, "Odd").starts_with("failed: distance"));
    error(
        &mut s,
        "feature.offsetProfile",
        json!({"sketch": "Odd", "distance": 1}),
    );
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Odd", "distance": "1 mm"}),
    );
    assert_eq!(status(&mut s, "Odd"), "ok");
    // An offset as a loft section or as a sweep's path: refused.
    error(
        &mut s,
        "feature.loft",
        json!({"sections": ["Odd", "Plate"]}),
    );
    error(
        &mut s,
        "feature.sweep",
        json!({"sketch": "Plate", "path": "Odd"}),
    );
}

/// A revolution takes an offset in the place of a sketch: a rectangle
/// beside the axis grown by d with round corners, turned a full turn, is
/// Pappus' volume of the offset area.
#[test]
fn a_revolution_takes_an_offset() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "sketch.create", json!({"name": "Section"}));
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Section", "points": [[2, 0], [4, 0], [4, 3], [2, 3]], "closed": true}),
    );
    run(
        &mut s,
        "feature.offsetProfile",
        json!({"sketch": "Section", "distance": "0.5 mm", "name": "Grown"}),
    );
    run(
        &mut s,
        "feature.revolve",
        json!({"sketch": "Grown", "axis": "y", "name": "Ring"}),
    );
    let area = 6.0 + 10.0 * 0.5 + PI * 0.25;
    let ring = body(&mut s, "Ring");
    assert_close(
        ring["volume"].as_f64().unwrap(),
        2.0 * PI * 3.0 * area,
        "Pappus",
    );
    let names = face_names(&ring);
    assert!(
        names.iter().any(|n| n.contains(":offsetCorner(")),
        "{names:?}"
    );
}

/// A reference to a piece of a region that split follows that region
/// (named by the curves of its loop), fails when the region is not split
/// (the name is not there), never lands on another region, and is back
/// when the split is.
#[test]
fn a_reference_to_a_piece_follows_its_region_or_fails() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "variable.set",
        json!({"name": "d", "expr": "-1.6 mm"}),
    );
    run(&mut s, "sketch.create", json!({"name": "Plate"}));
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Plate", "points": [[-5, -3], [5, -3], [5, 3], [-5, 3]], "closed": true}),
    );
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Plate", "center": [0, 0], "radius": 1.5}),
    );
    run(
        &mut s,
        "feature.offsetProfile",
        json!({"sketch": "Plate", "distance": "d", "name": "Inset"}),
    );
    run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Inset", "distance": 1, "name": "Strips"}),
    );
    let names = face_names(&body(&mut s, "Strips"));
    // The bottom line's two pieces (and the top's, and the hole's) are told
    // apart by the curves of the loop each closes.
    let pieces: Vec<&String> = names.iter().filter(|n| n.contains(":loop(")).collect();
    assert!(pieces.len() >= 6, "{names:?}");
    let piece = pieces[0].clone();
    let curve = piece.rsplit(":offset(").next().unwrap().to_string();
    let twins = pieces.iter().filter(|n| n.ends_with(&curve)).count();
    assert_eq!(twins, 2, "{curve} in {pieces:?}");
    run(
        &mut s,
        "feature.offsetFace",
        json!({"faces": [piece], "distance": "0.05 mm", "name": "Moved"}),
    );
    let moved = body(&mut s, "Moved")["volume"].as_f64().unwrap();
    // Further in, the same region: still there.
    run(
        &mut s,
        "variable.set",
        json!({"name": "d", "expr": "-1.65 mm"}),
    );
    assert_eq!(status(&mut s, "Moved"), "ok");
    // Not split: the name is not there, and the reference fails.
    run(
        &mut s,
        "variable.set",
        json!({"name": "d", "expr": "-0.5 mm"}),
    );
    let failed = status(&mut s, "Moved");
    assert!(failed.starts_with("failed"), "{failed}");
    assert!(failed.contains(&piece), "{failed}");
    // Back: repaired, the same body.
    run(
        &mut s,
        "variable.set",
        json!({"name": "d", "expr": "-1.6 mm"}),
    );
    assert_eq!(status(&mut s, "Moved"), "ok");
    assert_close(
        body(&mut s, "Moved")["volume"].as_f64().unwrap(),
        moved,
        "repaired",
    );
}
