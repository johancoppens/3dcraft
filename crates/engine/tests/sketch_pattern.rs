//! Patterns inside a sketch (`sketch.pattern`): rows, grids, chained runs
//! and rings of seed entities, made as rigid images of the solved seed at
//! every regeneration. The profiles are extruded and measured against
//! their closed forms at three scales; counts change by a variable in one
//! regeneration; names follow the instance's origin (`e7[3]`), a cap's
//! name holds a whole run as `e7[*]`, and a reference to an instance that
//! is gone fails instead of moving to another.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Access, Session, Workspace};
use serde_json::{Value, json};
use std::f64::consts::{PI, TAU};
use std::path::PathBuf;

fn workdir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("sketch-pattern-{name}"));
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
    // Built up edit by edit, the document is what regenerating it from
    // nothing gives (docs/testing.md: incremental = full regeneration).
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

fn feature_status(session: &mut Session, name: &str) -> String {
    let document = document(session);
    let feature = document["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == name)
        .unwrap_or_else(|| panic!("no feature {name}"))
        .clone();
    match (&feature["status"], &feature["error"]) {
        (Value::String(s), Value::String(e)) => format!("{s}: {e}"),
        (Value::String(s), _) => s.clone(),
        (other, _) => other.to_string(),
    }
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

/// A plate `width` by `height` of thickness `thick` with a row of `count`
/// holes of radius `r`, the first at (`first`, height / 2), `pitch` apart:
/// a sketch `Plate` with the outline, a circle and a pattern of it, and
/// an extrusion `Body`. Returns the circle's id.
fn hole_row(
    s: &mut Session,
    [width, height, thick, r, first, pitch]: [f64; 6],
    count: &str,
) -> String {
    run(s, "sketch.create", json!({"name": "Plate"}));
    run(
        s,
        "sketch.polyline",
        json!({"sketch": "Plate", "points": [[0, 0], [width, 0], [width, height], [0, height]], "closed": true}),
    );
    let circle = run(
        s,
        "sketch.circle",
        json!({"sketch": "Plate", "center": [first, height / 2.0], "radius": r}),
    );
    let circle = str_of(&circle["entity"]);
    let made = run(
        s,
        "sketch.pattern",
        json!({"sketch": "Plate", "entities": [circle], "count": count, "step": [format!("{pitch}"), 0]}),
    );
    assert_eq!(made["pattern"], "p1", "{made}");
    run(
        s,
        "feature.extrude",
        json!({"sketch": "Plate", "distance": format!("{thick}"), "name": "Body"}),
    );
    circle
}

#[test]
fn a_row_and_a_grid_of_holes_are_their_closed_forms_at_three_scales() {
    for k in [1e-3, 1.0, 1e3] {
        let mut s = session();
        run(&mut s, "file.new", json!({"units": "mm"}));
        run(&mut s, "variable.set", json!({"name": "n", "expr": "7"}));
        let (w, h, t, r) = (100.0 * k, 30.0 * k, 2.0 * k, 2.0 * k);
        let circle = hole_row(&mut s, [w, h, t, r, 10.0 * k, 12.0 * k], "n");
        let plate = body(&mut s, "Body");
        assert_close(
            plate["volume"].as_f64().unwrap(),
            (w * h - 7.0 * PI * r * r) * t,
            &format!("row at {k}"),
        );
        // Four sides, two caps, the seed's wall and six instances' walls.
        assert_eq!(plate["faces"], 4 + 2 + 7, "{k}");
        let names = face_names(&plate);
        assert!(names.contains(&format!("F2:side({circle})")), "{names:?}");
        for i in 1..7 {
            assert!(
                names.contains(&format!("F2:side({circle}[{i}])")),
                "{names:?}"
            );
        }

        // Rows: the same seed in a grid of 7 by 3, the rows 6 apart.
        run(
            &mut s,
            "sketch.pattern",
            json!({"sketch": "Plate", "replace": "p1", "step": [format!("{}", 12.0 * k), 0], "count2": 3, "step2": [0, format!("{}", -6.0 * k)]}),
        );
        // The rows go down from the middle: 15, 9 and 3, holes of radius 2.
        let plate = body(&mut s, "Body");
        assert_close(
            plate["volume"].as_f64().unwrap(),
            (w * h - 21.0 * PI * r * r) * t,
            &format!("grid at {k}"),
        );
        let names = face_names(&plate);
        assert!(
            names.contains(&format!("F2:side({circle}[6,2])")),
            "{names:?}"
        );
        assert!(names.contains(&format!("F2:side({circle})")), "{names:?}");
    }
}

/// A 70 × 70 perforated sheet made by one pattern: 4900 holes and the
/// outline, more curves than profile detection took before it had a broad
/// phase (4096; roadmap M5). The volume is the closed form.
#[test]
fn a_sheet_of_70_by_70_patterned_holes_extrudes() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    let (pitch, r, t) = (10.0, 3.0, 2.0);
    let side = 71.0 * pitch;
    run(&mut s, "sketch.create", json!({"name": "Plate"}));
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Plate", "points": [[0, 0], [side, 0], [side, side], [0, side]], "closed": true}),
    );
    let circle = run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Plate", "center": [pitch, pitch], "radius": r}),
    );
    let circle = str_of(&circle["entity"]);
    run(
        &mut s,
        "sketch.pattern",
        json!({"sketch": "Plate", "entities": [circle], "count": 70, "step": [pitch, 0], "count2": 70, "step2": [0, pitch]}),
    );
    run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Plate", "distance": t, "name": "Body"}),
    );
    let plate = body(&mut s, "Body");
    assert_close(
        plate["volume"].as_f64().unwrap(),
        (side * side - 4900.0 * PI * r * r) * t,
        "70 × 70 holes",
    );
    assert_eq!(plate["faces"], 4 + 2 + 4900);
}

/// A plate `length` wide and `height` high whose bottom edge is a chained
/// run of `n` periods of fingers `depth` deep, half a period wide: the
/// seed is one period from A (fixed at the origin) to B, and the far corner
/// C, fixed at (`length`, 0), is coincident with B in the last instance,
/// so the solver finds the period. Returns the ids of A, B and the seed's
/// four lines.
fn finger_plate(s: &mut Session, length: f64, height: f64, depth: f64, n: &str) -> Vec<String> {
    run(s, "variable.set", json!({"name": "n", "expr": n}));
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
    let made = run(
        s,
        "sketch.pattern",
        json!({"sketch": "Edge", "entities": lines, "count": "n", "chain": [a, b]}),
    );
    assert_eq!(made["status"], "ok", "{made}");
    // The rest of the outline, from the end of the run round to A.
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
    let tied = constrain(s, "coincident", json!([c, last]), None);
    assert_eq!(tied["status"], "ok", "{tied}");
    constrain(s, "vertical", json!([more[0]]), None);
    constrain(s, "horizontal", json!([more[1]]), None);
    let closed = constrain(s, "vertical", json!([more[2]]), None);
    assert!(closed["status"] == "ok", "{closed}");
    run(
        s,
        "feature.extrude",
        json!({"sketch": "Edge", "distance": 3, "name": "Panel"}),
    );
    let mut out = vec![a, b];
    out.extend(lines);
    out
}

#[test]
fn a_finger_edge_finds_its_period_and_follows_its_count_in_one_regeneration() {
    let (length, height, depth, t) = (120.0, 50.0, 3.0, 3.0);
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    let ids = finger_plate(&mut s, length, height, depth, "3");
    let (b, seed) = (ids[1].clone(), ids[2..].to_vec());
    let mut cap = None;
    for n in [3_u32, 33, 5, 1, 2, 40, 3] {
        run(
            &mut s,
            "variable.set",
            json!({"name": "n", "expr": n.to_string()}),
        );
        assert_eq!(feature_status(&mut s, "Edge"), "ok", "n = {n}");
        assert_eq!(feature_status(&mut s, "Panel"), "ok", "n = {n}");
        // Fingers half a period wide and `depth` deep: area L H + L d / 2.
        let panel = body(&mut s, "Panel");
        assert_close(
            panel["volume"].as_f64().unwrap(),
            (length * height + length * depth / 2.0) * t,
            &format!("n = {n}"),
        );
        // The solver found the period: B = A + (C - A) / n.
        let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "Edge"}));
        let end = inspected["entities"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["id"] == b.as_str())
            .unwrap()
            .clone();
        assert_close(end["x"].as_f64().unwrap(), length / f64::from(n), "period");
        assert_eq!(inspected["patterns"][0]["instances"], n);
        // Names follow the instance's origin; the caps name the run once.
        let names = face_names(&panel);
        for line in &seed {
            assert!(names.contains(&format!("F2:side({line})")), "{names:?}");
            let last = format!("F2:side({line}[{}])", n - 1);
            if n > 1 {
                assert!(names.contains(&last), "{last} in {names:?}");
            }
            assert!(!names.contains(&format!("F2:side({line}[{n}])")));
        }
        let caps: Vec<&String> = names.iter().filter(|n| n.contains("Cap")).collect();
        assert_eq!(caps.len(), 2);
        // Also at one instance: the seed alone is the whole run.
        for line in &seed {
            assert!(caps[0].contains(&format!("{line}[*]")), "{caps:?}");
        }
        match &cap {
            None => cap = Some(caps[0].clone()),
            Some(cap) => assert_eq!(cap, caps[0], "the cap keeps its name"),
        }
    }
}

/// The outline of a patterned sketch, exported for a laser cutter, has
/// its instances: one closed path whose length is the closed form, the
/// same as the outline of the extruded panel's cap.
#[test]
fn a_patterned_outline_is_exported_with_its_instances() {
    let dir = workdir("export");
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    let (length, height, depth) = (120.0, 50.0, 3.0);
    finger_plate(&mut s, length, height, depth, "9");
    let cap = face_names(&body(&mut s, "Panel"))
        .into_iter()
        .find(|n| n.contains("endCap"))
        .unwrap();
    for (file, source) in [
        ("sketch", json!({"sketch": "Edge"})),
        ("cap", json!({"face": cap})),
    ] {
        for format in ["dxf", "svg"] {
            let mut params = source.clone();
            params["path"] = json!(format!("{file}.{format}"));
            let result = run(&mut s, "file.export", params);
            assert_eq!(
                (&result["paths"], &result["closed"], &result["lines"]),
                (&json!(1), &json!(1), &json!(4 * 9 + 3)),
                "{source} {result}"
            );
            assert_close(
                result["length"].as_f64().unwrap(),
                2.0 * length + 2.0 * height + 2.0 * 9.0 * depth,
                &format!("{source} {format}"),
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Separate instances (a row of discs) are each their own profile: the
/// seed's cap is named after the seed at every count, 1 included.
#[test]
fn the_cap_of_a_separate_seed_keeps_its_name_at_every_count() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "variable.set", json!({"name": "n", "expr": "1"}));
    run(&mut s, "sketch.create", json!({"name": "Discs"}));
    let disc = run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Discs", "center": [0, 0], "radius": 1}),
    );
    let disc = str_of(&disc["entity"]);
    run(
        &mut s,
        "sketch.pattern",
        json!({"sketch": "Discs", "entities": [disc], "count": "n", "step": [5, 0]}),
    );
    run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Discs", "distance": 2, "name": "Pins"}),
    );
    let cap = format!("F2:endCap({disc})");
    for n in [1, 2, 1, 3, 1] {
        run(
            &mut s,
            "variable.set",
            json!({"name": "n", "expr": n.to_string()}),
        );
        let names = face_names(&body(&mut s, "Pins"));
        assert!(names.contains(&cap), "n = {n}: {names:?}");
        if n > 1 {
            assert!(
                names.contains(&format!("F2:endCap({disc}[1])")),
                "{names:?}"
            );
        }
        assert!(!names.iter().any(|n| n.contains("[*]")), "{names:?}");
    }
}

/// A ring of runs (`tooth` from `a` to `b` about the origin, fixed `a` at
/// radius `r`, the tip fixed) on a plane `offset` up. Returns the ids of
/// `a` and `b`.
fn ring(s: &mut Session, name: &str, offset: f64, r: f64, angle: &str) -> (String, String) {
    run(s, "sketch.create", json!({"name": name, "offset": offset}));
    let o = str_of(&run(s, "sketch.point", json!({"sketch": name, "at": [0, 0]}))["entity"]);
    let tooth = run(
        s,
        "sketch.polyline",
        json!({"sketch": name, "points": [[r, 0], [1.1 * r, 0.2 * r], [0.85 * r, 0.5 * r]]}),
    );
    let points: Vec<String> = tooth["points"]
        .as_array()
        .unwrap()
        .iter()
        .map(str_of)
        .collect();
    let lines: Vec<String> = tooth["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(str_of)
        .collect();
    for point in [&o, &points[0], &points[1]] {
        run(
            s,
            "sketch.constrain",
            json!({"sketch": name, "type": "fixed", "entities": [point]}),
        );
    }
    run(
        s,
        "sketch.pattern",
        json!({"sketch": name, "entities": lines, "count": "teeth", "angle": angle, "center": o, "chain": [points[0], points[2]]}),
    );
    (points[0].clone(), points[2].clone())
}

/// A ring whose turns make a whole turn within the angular tolerance but
/// whose last joint misses the first by more than the linear tolerance
/// does not close, and its image points are where its curves are: the
/// last joint as the turns put it.
#[test]
fn a_ring_that_misses_its_start_does_not_close_and_its_images_say_so() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "variable.set",
        json!({"name": "teeth", "expr": "12"}),
    );
    let r = 1e4;
    // 5e-11 rad too much per tooth: 6e-10 rad in all, within the angular
    // tolerance (1e-9); 6e-6 mm at the rim, beyond the linear one (1e-6).
    let extra = 5e-11_f64.to_degrees();
    let (a, b) = ring(&mut s, "Ring", 0.0, r, &format!("30 deg + {extra:e} deg"));
    run(
        &mut s,
        "sketch.constrain",
        json!({"sketch": "Ring", "type": "fixed", "entities": [format!("{b}[last]")]}),
    );
    let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "Ring"}));
    assert_eq!(inspected["patterns"][0]["closes"], false, "{inspected}");
    let image = inspected["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["ref"] == format!("{b}[last]"))
        .unwrap()
        .clone();
    let (x, y) = (image["x"].as_f64().unwrap(), image["y"].as_f64().unwrap());
    let turn = 12.0 * (30.0_f64.to_radians() + 5e-11);
    let (wx, wy) = (r * turn.cos(), r * turn.sin());
    assert!((x - wx).hypot(y - wy) < 1e-9 * r, "{x} {y} vs {wx} {wy}");
    assert!((x - r).hypot(y) > 1e-6, "not taken to be the start {a}");
}

/// The caps of a loft through two rings of runs name each run once, at
/// every count.
#[test]
fn the_caps_of_a_loft_through_patterned_sections_name_the_run_once() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "variable.set",
        json!({"name": "teeth", "expr": "6"}),
    );
    ring(&mut s, "Low", 0.0, 10.0, "360 deg / teeth");
    ring(&mut s, "High", 8.0, 10.0, "360 deg / teeth");
    run(
        &mut s,
        "feature.loft",
        json!({"sections": ["Low", "High"], "name": "Crown"}),
    );
    let mut caps = None;
    for n in [6, 9, 3, 6] {
        run(
            &mut s,
            "variable.set",
            json!({"name": "teeth", "expr": n.to_string()}),
        );
        assert_eq!(feature_status(&mut s, "Crown"), "ok", "{n}");
        let names: Vec<String> = face_names(&body(&mut s, "Crown"))
            .into_iter()
            .filter(|n| n.contains("Cap"))
            .collect();
        assert_eq!(names.len(), 2, "{names:?}");
        assert!(names.iter().all(|n| n.contains("[*]")), "{names:?}");
        match &caps {
            None => caps = Some(names),
            Some(caps) => assert_eq!(caps, &names, "{n}"),
        }
    }
}

#[test]
fn a_ring_of_teeth_closes_and_is_its_closed_form() {
    for k in [1e-3, 1.0, 1e3] {
        let (r, h) = (20.0 * k, 4.0 * k);
        let mut s = session();
        run(&mut s, "file.new", json!({"units": "mm"}));
        run(
            &mut s,
            "variable.set",
            json!({"name": "teeth", "expr": "12"}),
        );
        run(&mut s, "sketch.create", json!({"name": "Gear"}));
        let centre = run(
            &mut s,
            "sketch.point",
            json!({"sketch": "Gear", "at": [0, 0]}),
        );
        let o = str_of(&centre["entity"]);
        let tooth = run(
            &mut s,
            "sketch.polyline",
            json!({"sketch": "Gear", "points": [[r, 0], [r + h, 0.1 * r], [0.85 * r, 0.5 * r]]}),
        );
        let points: Vec<String> = tooth["points"]
            .as_array()
            .unwrap()
            .iter()
            .map(str_of)
            .collect();
        let lines: Vec<String> = tooth["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(str_of)
            .collect();
        let (a, tip, b) = (points[0].clone(), points[1].clone(), points[2].clone());
        let axis = run(
            &mut s,
            "sketch.line",
            json!({"sketch": "Gear", "from": o, "to": tip, "construction": true}),
        );
        let axis = str_of(&axis["entity"]);
        for (kind, entities, value) in [
            ("fixed", json!([o]), None),
            ("fixed", json!([a]), None),
            ("distance", json!([axis]), Some(format!("{}", r + h))),
        ] {
            let mut params = json!({"sketch": "Gear", "type": kind, "entities": entities});
            if let Some(value) = value {
                params["value"] = json!(value);
            }
            run(&mut s, "sketch.constrain", params);
        }
        let ring = run(
            &mut s,
            "sketch.pattern",
            json!({"sketch": "Gear", "entities": lines, "count": "teeth", "angle": "360 deg / teeth", "center": o, "chain": [a, b]}),
        );
        assert_eq!(ring["status"], "ok", "{ring}");
        run(
            &mut s,
            "sketch.constrain",
            json!({"sketch": "Gear", "type": "symmetric", "entities": [a, b, axis]}),
        );
        run(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Gear", "distance": format!("{}", 5.0 * k), "name": "Wheel"}),
        );
        for n in [12_u32, 7, 30] {
            run(
                &mut s,
                "variable.set",
                json!({"name": "teeth", "expr": n.to_string()}),
            );
            assert_eq!(feature_status(&mut s, "Wheel"), "ok", "{n} at {k}");
            let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "Gear"}));
            assert_eq!(inspected["patterns"][0]["closes"], true, "{inspected}");
            assert_eq!(inspected["patterns"][0]["chainState"], "satisfied");
            // Each tooth is the triangles O A T and O T B: r (r + h) sin(θ/2).
            let theta = TAU / f64::from(n);
            let area = f64::from(n) * r * (r + h) * (theta / 2.0).sin();
            let wheel = body(&mut s, "Wheel");
            assert_close(
                wheel["volume"].as_f64().unwrap(),
                area * 5.0 * k,
                &format!("{n} teeth at {k}"),
            );
            assert_eq!(wheel["faces"], 2 * n + 2);
        }
    }
}

#[test]
fn a_reference_to_an_instance_that_is_gone_fails_and_comes_back() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "variable.set",
        json!({"name": "slots", "expr": "5"}),
    );
    run(&mut s, "sketch.create", json!({"name": "Plate"}));
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Plate", "points": [[0, 0], [100, 0], [100, 30], [0, 30]], "closed": true}),
    );
    let slot = run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Plate", "points": [[5, 10], [10, 10], [10, 20], [5, 20]], "closed": true}),
    );
    let lines: Vec<String> = slot["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(str_of)
        .collect();
    run(
        &mut s,
        "sketch.pattern",
        json!({"sketch": "Plate", "entities": lines, "count": "slots", "step": ["15 mm", 0]}),
    );
    run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Plate", "distance": 4, "name": "Body"}),
    );
    // The far wall of slot 3 (x = 55) carries a sketch.
    let wall = format!("F2:side({}[3])", lines[1]);
    assert!(face_names(&body(&mut s, "Body")).contains(&wall));
    run(
        &mut s,
        "sketch.create",
        json!({"name": "OnWall", "face": wall}),
    );
    let frame = run(&mut s, "sketch.inspect", json!({"sketch": "OnWall"}))["frame"].clone();
    assert_close(
        frame["origin"][0].as_f64().unwrap(),
        55.0,
        "on the wall of slot 3",
    );

    // Two slots: the wall is gone and the sketch says so; it is not moved
    // to another slot's wall.
    run(
        &mut s,
        "variable.set",
        json!({"name": "slots", "expr": "2"}),
    );
    assert_eq!(feature_status(&mut s, "Body"), "ok");
    let failed = feature_status(&mut s, "OnWall");
    assert!(failed.contains(&wall), "{failed}");
    assert!(failed.contains("no longer has a face"), "{failed}");
    // Back to five: the same wall, the same place.
    run(
        &mut s,
        "variable.set",
        json!({"name": "slots", "expr": "5"}),
    );
    assert_eq!(feature_status(&mut s, "OnWall"), "ok");
    let again = run(&mut s, "sketch.inspect", json!({"sketch": "OnWall"}))["frame"].clone();
    assert_eq!(again, frame);
}

#[test]
fn an_image_point_past_the_count_is_missing_and_last_follows_the_count() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "variable.set", json!({"name": "n", "expr": "5"}));
    run(&mut s, "sketch.create", json!({"name": "S"}));
    let p = run(&mut s, "sketch.point", json!({"sketch": "S", "at": [0, 0]}));
    let p = str_of(&p["entity"]);
    let q = run(&mut s, "sketch.point", json!({"sketch": "S", "at": [3, 0]}));
    let q = str_of(&q["entity"]);
    let dot = run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "S", "center": p, "radius": 1}),
    );
    let dot = str_of(&dot["entity"]);
    run(
        &mut s,
        "sketch.constrain",
        json!({"sketch": "S", "type": "fixed", "entities": [p]}),
    );
    run(
        &mut s,
        "sketch.pattern",
        json!({"sketch": "S", "entities": [dot], "count": "n", "step": [10, 0]}),
    );
    // q on instance 4 of p: (40, 0).
    let tied = run(
        &mut s,
        "sketch.constrain",
        json!({"sketch": "S", "type": "coincident", "entities": [q, format!("{p}[4]")]}),
    );
    let c = str_of(&tied["constraint"]);
    let where_q = |s: &mut Session| {
        let inspected = run(s, "sketch.inspect", json!({"sketch": "S"}));
        let entry = inspected["entities"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["id"] == q.as_str())
            .unwrap()
            .clone();
        (entry["x"].as_f64().unwrap(), inspected)
    };
    assert_close(where_q(&mut s).0, 40.0, "q on e[4]");
    // An index past the count is refused when it is given.
    let refused = error(
        &mut s,
        "sketch.constrain",
        json!({"sketch": "S", "type": "fixed", "entities": [format!("{p}[9]")]}),
    );
    assert!(
        refused.contains("is not there") && refused.contains("5 instances"),
        "{refused}"
    );

    // Three instances: e[4] is missing; its constraint is left out and the
    // sketch fails naming it; q is not put on another instance.
    run(&mut s, "variable.set", json!({"name": "n", "expr": "3"}));
    let (x, inspected) = where_q(&mut s);
    assert_close(x, 40.0, "q stays");
    let state = inspected["constraints"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == c.as_str())
        .unwrap()["state"]
        .clone();
    assert_eq!(state, "missing");
    let failed = feature_status(&mut s, "S");
    assert!(
        failed.contains(&format!("{p}[4]")) && failed.contains("3 instances"),
        "{failed}"
    );
    run(&mut s, "variable.set", json!({"name": "n", "expr": "5"}));
    assert_eq!(feature_status(&mut s, "S"), "ok");

    // `last` follows the count.
    run(
        &mut s,
        "sketch.delete",
        json!({"sketch": "S", "constraint": c}),
    );
    run(
        &mut s,
        "sketch.constrain",
        json!({"sketch": "S", "type": "coincident", "entities": [q, format!("{p}[last]")]}),
    );
    for n in [5, 2, 8] {
        run(
            &mut s,
            "variable.set",
            json!({"name": "n", "expr": n.to_string()}),
        );
        assert_close(where_q(&mut s).0, 10.0 * f64::from(n - 1), "q on e[last]");
    }
    // The same image twice is one entity.
    let before = run(&mut s, "sketch.inspect", json!({"sketch": "S"}))["entities"]
        .as_array()
        .unwrap()
        .len();
    run(
        &mut s,
        "sketch.constrain",
        json!({"sketch": "S", "type": "horizontal", "entities": [q, format!("{p}[last]")]}),
    );
    let after = run(&mut s, "sketch.inspect", json!({"sketch": "S"}))["entities"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(before, after);

    // Deleting the pattern takes its image points and their constraints.
    let gone = run(
        &mut s,
        "sketch.delete",
        json!({"sketch": "S", "pattern": "p1"}),
    );
    // e[4] (its constraint went before) and e[last].
    assert_eq!(gone["imagesDeleted"].as_array().unwrap().len(), 2, "{gone}");
    assert_eq!(
        gone["constraintsDeleted"].as_array().unwrap().len(),
        2,
        "{gone}"
    );
    assert_eq!(feature_status(&mut s, "S"), "ok");
    let left = run(&mut s, "sketch.inspect", json!({"sketch": "S"}));
    assert!(left.get("patterns").is_none(), "{left}");
}

#[test]
fn deleting_a_seed_entity_shrinks_or_removes_its_pattern() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "sketch.create", json!({"name": "S"}));
    let a = run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "S", "center": [0, 0], "radius": 1}),
    );
    let b = run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "S", "center": [0, 5], "radius": 1}),
    );
    let (a, b) = (str_of(&a["entity"]), str_of(&b["entity"]));
    run(
        &mut s,
        "sketch.pattern",
        json!({"sketch": "S", "entities": [a, b], "count": 3, "step": [4, 0]}),
    );
    let gone = run(&mut s, "sketch.delete", json!({"sketch": "S", "entity": a}));
    assert!(gone.get("patternsDeleted").is_none(), "{gone}");
    let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "S"}));
    assert_eq!(inspected["patterns"][0]["seed"], json!([b]));
    let gone = run(&mut s, "sketch.delete", json!({"sketch": "S", "entity": b}));
    assert_eq!(gone["patternsDeleted"], json!(["p1"]), "{gone}");
}

#[test]
fn the_same_document_gives_the_same_bits_and_survives_save_undo_and_redo() {
    let dir = workdir("bits");
    let build = |s: &mut Session| {
        run(s, "file.new", json!({"units": "mm"}));
        finger_plate(s, 97.0, 41.0, 2.5, "11");
        body(s, "Panel")
    };
    let mut first = session_in(&dir);
    let mut second = session_in(&dir);
    let one = build(&mut first);
    let two = build(&mut second);
    assert_eq!(one, two);
    assert_eq!(
        one["volume"].as_f64().unwrap().to_bits(),
        two["volume"].as_f64().unwrap().to_bits()
    );
    let sketch = run(&mut first, "sketch.inspect", json!({"sketch": "Edge"}));

    // Saved and opened again: the same sketch and body, format 18.
    run(&mut first, "file.save", json!({"path": "fingers.3dcraft"}));
    run(&mut first, "file.close", json!({}));
    run(&mut first, "file.open", json!({"path": "fingers.3dcraft"}));
    assert_eq!(body(&mut first, "Panel"), one);
    assert_eq!(
        run(&mut first, "sketch.inspect", json!({"sketch": "Edge"})),
        sketch
    );

    // Undo and redo of a pattern edit restore it exactly.
    run(
        &mut first,
        "sketch.pattern",
        json!({"sketch": "Edge", "replace": "p1", "count": "n + 2"}),
    );
    assert_ne!(body(&mut first, "Panel"), one);
    run(&mut first, "edit.undo", json!({}));
    assert_eq!(body(&mut first, "Panel"), one);
    assert_eq!(
        run(&mut first, "sketch.inspect", json!({"sketch": "Edge"})),
        sketch
    );
    run(&mut first, "edit.redo", json!({}));
    let inspected = run(&mut first, "sketch.inspect", json!({"sketch": "Edge"}));
    assert_eq!(inspected["patterns"][0]["instances"], 13);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bad_pattern_params_are_refused_with_a_reason() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "sketch.create", json!({"name": "S"}));
    let dot = run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "S", "center": [0, 0], "radius": 1}),
    );
    let dot = str_of(&dot["entity"]);
    let centre = str_of(
        &dot.replace('e', "")
            .parse::<u64>()
            .map(|n| json!(format!("e{}", n - 1)))
            .unwrap(),
    );
    let refused = |s: &mut Session, params: Value, want: &str| {
        let mut full = json!({"sketch": "S"});
        for (k, v) in params.as_object().unwrap() {
            full[k] = v.clone();
        }
        let message = error(s, "sketch.pattern", full.clone());
        assert!(message.contains(want), "{full}: {message}");
    };
    let seed = json!([dot]);
    refused(&mut s, json!({"count": 3, "step": [1, 0]}), "entities");
    refused(
        &mut s,
        json!({"entities": [], "count": 3, "step": [1, 0]}),
        "at least one",
    );
    refused(
        &mut s,
        json!({"entities": ["e99"], "count": 3, "step": [1, 0]}),
        "e99",
    );
    refused(&mut s, json!({"entities": seed, "step": [1, 0]}), "count");
    refused(&mut s, json!({"entities": seed, "count": 3}), "give `step`");
    refused(
        &mut s,
        json!({"entities": seed, "count": 3, "step": [1]}),
        "two expressions",
    );
    refused(
        &mut s,
        json!({"entities": seed, "count": 3, "angle": 30}),
        "center",
    );
    refused(
        &mut s,
        json!({"entities": seed, "count": 3, "step": [1, 0], "angle": 30, "center": [0, 0]}),
        "not both",
    );
    refused(
        &mut s,
        json!({"entities": seed, "count": 3, "chain": [centre, "e1"]}),
        "twice",
    );
    refused(
        &mut s,
        json!({"entities": seed, "count": 3, "chain": ["e1", "e7"]}),
        "e7",
    );
    refused(
        &mut s,
        json!({"entities": seed, "count": 3, "step": [1, 0], "count2": 2}),
        "step2",
    );
    refused(
        &mut s,
        json!({"entities": seed, "count": 3, "step": [1, 0], "replace": "p9"}),
        "no pattern p9",
    );
    refused(
        &mut s,
        json!({"entities": seed, "count": 3, "step": [1, 0], "replace": "x"}),
        "replace",
    );

    // Counts and steps that do not evaluate to a pattern fail the sketch
    // with the reason; nothing panics.
    for (count, step, want) in [
        ("0", "1", "at least 1"),
        ("-1", "1", "at least 1"),
        ("2.5", "1", "whole number"),
        ("1e12", "1", "at most"),
        ("5 mm", "1", "plain number"),
        ("nan", "1", "count"),
        ("3", "0", "zero"),
        ("3", "1 deg", "length"),
    ] {
        let made = s.execute(
            "sketch.pattern",
            &json!({"sketch": "S", "entities": [dot], "count": count, "step": [step, 0]}),
        );
        match made {
            Ok(result) => {
                let status = format!("{} {}", result["status"], result["error"]);
                assert!(status.contains(want), "{count} {step}: {status}");
                run(&mut s, "edit.undo", json!({}));
            }
            Err(e) => assert!(e.to_string().contains(want), "{count} {step}: {e}"),
        }
    }

    // Image points that cannot be.
    run(
        &mut s,
        "sketch.pattern",
        json!({"sketch": "S", "entities": [dot], "count": 3, "step": [5, 0]}),
    );
    for (text, want) in [
        (format!("{centre}[99]"), "is not there"),
        (format!("{centre}[-1]"), "not an image point"),
        (format!("{centre}[last"), "not an image point"),
        (format!("{centre}[1,2,3]"), "not an image point"),
        (format!("{centre}[1,2]"), "one number"),
        ("e77[1]".to_string(), "e77"),
        (format!("{dot}[1]"), "circle"),
    ] {
        let message = error(
            &mut s,
            "sketch.constrain",
            json!({"sketch": "S", "type": "fixed", "entities": [text]}),
        );
        assert!(message.contains(want), "{text}: {message}");
    }
    // A constraint between images of one pattern only.
    let message = error(
        &mut s,
        "sketch.constrain",
        json!({"sketch": "S", "type": "coincident", "entities": [format!("{centre}[1]"), format!("{centre}[2]")]}),
    );
    assert!(message.contains("not supported yet"), "{message}");
    // A circle on an image point.
    let message = error(
        &mut s,
        "sketch.circle",
        json!({"sketch": "S", "center": format!("{centre}[1]"), "radius": 1}),
    );
    assert!(message.contains("not supported yet"), "{message}");
    // A seed in two patterns.
    let message = error(
        &mut s,
        "sketch.pattern",
        json!({"sketch": "S", "entities": [dot], "count": 2, "step": [0, 5]}),
    );
    assert!(message.contains("seed of p1"), "{message}");
    assert_eq!(feature_status(&mut s, "S"), "ok");
}

/// A row of 400 holes: the time to solve the sketch and extrude it, and to
/// change the count by one. Run in release with `--ignored --nocapture`;
/// the numbers are in the dev log and docs/design/document.md.
#[test]
#[ignore = "a timing, for release builds"]
fn a_row_of_400_holes_is_quick() {
    use std::time::{Duration, Instant};
    // Each command by itself, without the comparison with a full
    // regeneration that `run` makes.
    let timed = |s: &mut Session, id: &str, params: Value| -> (Value, Duration) {
        let started = Instant::now();
        let result = s.execute(id, &params).unwrap();
        (result, started.elapsed())
    };
    // The baseline: the plate with one hole.
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "sketch.create", json!({"name": "Plate"}));
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Plate", "points": [[0, 0], [4020, 0], [4020, 20], [0, 20]], "closed": true}),
    );
    let (_, circle) = timed(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Plate", "center": [10, 10], "radius": 3}),
    );
    let (_, one) = timed(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Plate", "distance": 2, "name": "Body"}),
    );
    println!("1 hole: circle {circle:?}, extrude {one:?}");

    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "variable.set", json!({"name": "n", "expr": "400"}));
    run(&mut s, "sketch.create", json!({"name": "Plate"}));
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Plate", "points": [[0, 0], [4020, 0], [4020, 20], [0, 20]], "closed": true}),
    );
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Plate", "center": [10, 10], "radius": 3}),
    );
    let (_, pattern) = timed(
        &mut s,
        "sketch.pattern",
        json!({"sketch": "Plate", "entities": ["e10"], "count": "n", "step": [10, 0]}),
    );
    let (_, extrude) = timed(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Plate", "distance": 2, "name": "Body"}),
    );
    let built = pattern + extrude;
    println!("400 holes: pattern {pattern:?}, extrude {extrude:?}");
    let plate = body(&mut s, "Body");
    assert_close(
        plate["volume"].as_f64().unwrap(),
        (4020.0 * 20.0 - 400.0 * PI * 9.0) * 2.0,
        "400 holes",
    );
    let started = Instant::now();
    s.execute("variable.set", &json!({"name": "n", "expr": "401"}))
        .unwrap();
    let changed = started.elapsed();
    let plate = body(&mut s, "Body");
    assert_close(
        plate["volume"].as_f64().unwrap(),
        (4020.0 * 20.0 - 401.0 * PI * 9.0) * 2.0,
        "401 holes",
    );
    println!("400 holes: pattern and extrude {built:?}; 400 -> 401 by one command {changed:?}");

    // A finger edge of 400 periods: 1603 curves, one profile.
    let mut f = session();
    run(&mut f, "file.new", json!({"units": "mm"}));
    finger_plate(&mut f, 4000.0, 50.0, 3.0, "3");
    let (_, fingers) = timed(&mut f, "variable.set", json!({"name": "n", "expr": "400"}));
    let panel = body(&mut f, "Panel");
    assert_close(
        panel["volume"].as_f64().unwrap(),
        (4000.0 * 50.0 + 4000.0 * 3.0 / 2.0) * 3.0,
        "400 fingers",
    );
    assert_eq!(panel["faces"], 4 * 400 + 3 + 2);
    let (_, again) = timed(&mut f, "variable.set", json!({"name": "n", "expr": "401"}));
    let panel = body(&mut f, "Panel");
    assert_eq!(panel["faces"], 4 * 401 + 3 + 2);
    println!(
        "400 fingers (1603 curves): 3 -> 400 by one command {fingers:?}, 400 -> 401 {again:?}"
    );
}
