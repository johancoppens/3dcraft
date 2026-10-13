//! `file.export` to DXF and SVG through the command layer: the outline of
//! a sketch and of a flat face, in the document's unit, and what is
//! refused. The writers themselves are tested in `craft3d-drawing`, with
//! readers of their own; FreeCAD reads these files in
//! `drawing_freecad.rs`.

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

fn session(tag: &str) -> (Session, PathBuf) {
    let dir = std::env::temp_dir().join(format!("craft3d-drawing-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let session = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    (session, dir)
}

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn refused(session: &mut Session, params: Value, words: &[&str]) {
    let error = session
        .execute("file.export", &params)
        .expect_err(&params.to_string())
        .to_string();
    assert!(!error.contains("internal error"), "{params}: {error}");
    for word in words {
        assert!(error.contains(word), "{params}: `{error}` lacks `{word}`");
    }
}

fn near(a: f64, b: f64, relative: f64) -> bool {
    (a - b).abs() <= relative * a.abs().max(b.abs())
}

fn number(value: &Value) -> f64 {
    value
        .as_f64()
        .unwrap_or_else(|| panic!("{value} is no number"))
}

fn numbers(value: &Value) -> Vec<f64> {
    value.as_array().unwrap().iter().map(number).collect()
}

/// How often an entity of this kind occurs in a DXF file.
fn entities(dxf: &str, kind: &str) -> usize {
    let section = &dxf[dxf.find("  2\nENTITIES\n").unwrap()..dxf.find("  2\nOBJECTS\n").unwrap()];
    section.matches(&format!("  0\n{kind}\n")).count()
}

/// The value of a header variable of a DXF file.
fn header<'a>(dxf: &'a str, name: &str) -> &'a str {
    let at = dxf.find(&format!("  9\n{name}\n")).unwrap();
    dxf[at..].lines().nth(3).unwrap()
}

fn attribute<'a>(svg: &'a str, name: &str) -> &'a str {
    let key = format!(" {name}=\"");
    let start = svg.find(&key).unwrap_or_else(|| panic!("no {name}")) + key.len();
    &svg[start..start + svg[start..].find('"').unwrap()]
}

/// The plate of the demo, times `k`: 80 × 50 with corners of radius 8, a
/// slot, a keyhole and a D hole, drawn with `sketch.path` and extruded.
fn plate(s: &mut Session, k: f64) {
    let p = |x: f64, y: f64| json!([x * k, y * k]);
    run(s, "sketch.create", json!({"name": "Plate"}));
    let paths = [
        json!({"start": p(8.0, 0.0), "segments": [
            {"line": p(72.0, 0.0), "tangent": true}, {"arc": p(80.0, 8.0), "tangent": true},
            {"line": p(80.0, 42.0), "tangent": true}, {"arc": p(72.0, 50.0), "radius": 8.0 * k, "tangent": true},
            {"line": p(8.0, 50.0), "tangent": true}, {"arc": p(0.0, 42.0), "center": p(8.0, 42.0), "tangent": true},
            {"line": p(0.0, 8.0), "tangent": true}, {"arc": "start", "center": p(8.0, 8.0), "tangent": true}]}),
        json!({"start": p(12.0, 10.0), "segments": [
            {"line": p(32.0, 10.0)}, {"arc": p(32.0, 18.0), "tangent": true},
            {"line": p(12.0, 18.0), "tangent": true}, {"arc": "start", "tangent": true}]}),
        json!({"start": p(58.0, 12.0), "closed": true, "segments": [
            {"line": p(62.0, 12.0)}, {"line": p(62.0, 28.0)},
            {"arc": p(58.0, 28.0), "radius": 7.0 * k, "large": true}]}),
        json!({"start": p(16.0, 30.0), "closed": true, "segments": [
            {"arc": p(16.0, 42.0), "radius": 7.5 * k, "large": true}]}),
    ];
    for mut path in paths {
        path["sketch"] = json!("Plate");
        run(s, "sketch.path", path);
    }
    run(
        s,
        "feature.extrude",
        json!({"sketch": "Plate", "distance": 6.0 * k, "name": "Body"}),
    );
}

/// Closed forms for the plate at `k` = 1: the length of its outline and
/// the area inside it.
fn plate_numbers() -> (f64, f64) {
    // A circle of radius r less the segment a chord c cuts off: the angle
    // it keeps, its arc and its area.
    let keeps = |r: f64, c: f64| TAU - 2.0 * (c / 2.0 / r).asin();
    let segment = |r: f64, c: f64| r * r / 2.0 * (keeps(r, c) - keeps(r, c).sin());
    let length = (2.0 * (64.0 + 34.0) + 16.0 * PI)
        + (40.0 + 8.0 * PI)
        + (4.0 + 16.0 + 16.0 + 7.0 * keeps(7.0, 4.0))
        + (12.0 + 7.5 * keeps(7.5, 12.0));
    let area = (80.0 * 50.0 - (4.0 - PI) * 64.0)
        - (160.0 + 16.0 * PI)
        - (64.0 + segment(7.0, 4.0))
        - segment(7.5, 12.0);
    (length, area)
}

/// The names of the faces of a body whose name has this role in it.
fn face_named(s: &mut Session, role: &str) -> String {
    let inspect = run(s, "document.inspect", json!({}));
    inspect["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|body| body["faceNames"].as_array().unwrap())
        .map(|name| name.as_str().unwrap().to_string())
        .find(|name| name.contains(role))
        .unwrap_or_else(|| panic!("no face with {role}"))
}

#[test]
fn a_sketch_and_the_top_face_of_its_extrusion_give_the_same_outline() {
    let (mut s, dir) = session("plate");
    run(&mut s, "file.new", json!({}));
    plate(&mut s, 1.0);
    let (length, area) = plate_numbers();
    // The kernel's own account of the area: the volume over the height.
    let volume = number(&run(&mut s, "document.measure", json!({}))["volume"]);
    assert!(near(volume / 6.0, area, 1e-12), "{} {area}", volume / 6.0);

    let top = face_named(&mut s, "endCap");
    let bottom = face_named(&mut s, "startCap");
    let mut results = Vec::new();
    for (file, source) in [
        ("sketch", json!({"sketch": "Plate"})),
        ("top", json!({"face": top})),
        ("bottom", json!({"face": bottom, "feature": "Body"})),
    ] {
        for format in ["dxf", "svg"] {
            let mut params = source.clone();
            params["path"] = json!(format!("{file}.{format}"));
            let result = run(&mut s, "file.export", params);
            println!("{result}");
            assert_eq!(result["format"], format);
            assert_eq!(result["unit"], "mm");
            assert_eq!(
                (
                    &result["paths"],
                    &result["closed"],
                    &result["lines"],
                    &result["arcs"]
                ),
                (&json!(4), &json!(4), &json!(10), &json!(8))
            );
            assert_eq!(
                (&result["circles"], &result["ellipses"], &result["splines"]),
                (&json!(0), &json!(0), &json!(0))
            );
            assert!(near(number(&result["length"]), length, 1e-12), "{result}");
            let bbox = numbers(&result["bbox"]);
            // Seen from below, the plate is turned over: y runs the
            // other way.
            let expected = if file == "bottom" {
                [0.0, -50.0, 80.0, 0.0]
            } else {
                [0.0, 0.0, 80.0, 50.0]
            };
            for (got, expected) in bbox.iter().zip(expected) {
                assert!((got - expected).abs() <= 1e-12 * 80.0, "{file}: {bbox:?}");
            }
            let text = std::fs::read_to_string(dir.join(format!("{file}.{format}"))).unwrap();
            assert_eq!(result["bytes"], text.len());
            if format == "dxf" {
                assert_eq!(result["version"], "AC1015");
                assert_eq!(header(&text, "$ACADVER"), "AC1015");
                assert_eq!(header(&text, "$INSUNITS"), "4");
                assert_eq!((entities(&text, "LINE"), entities(&text, "ARC")), (10, 8));
                assert_eq!(entities(&text, "CIRCLE") + entities(&text, "SPLINE"), 0);
            } else {
                assert_eq!(
                    (&result["approximated"], &result["deviation"]),
                    (&json!(0), &json!(0.0))
                );
                assert_eq!(attribute(&text, "width"), "80mm");
                assert_eq!(attribute(&text, "height"), "50mm");
                let y = if file == "bottom" { "0" } else { "-50" };
                assert_eq!(attribute(&text, "viewBox"), format!("0 {y} 80 50"));
                assert_eq!(attribute(&text, "fill"), "none");
                assert_eq!(text.matches(" Z").count(), 4, "four closed subpaths");
                assert!(!text.contains("transform"));
            }
            results.push((file, format, result));
        }
    }
    // Where the face's drawing lies in the model.
    let top = &results[2].2;
    assert_eq!(top["feature"], "F2");
    assert_eq!(top["name"], "Body");
    assert_eq!(top["faces"], 1);
    assert_eq!(numbers(&top["origin"]), [0.0, 0.0, 6.0]);
    assert_eq!(numbers(&top["xAxis"]), [1.0, 0.0, 0.0]);
    assert_eq!(numbers(&top["yAxis"]), [0.0, 1.0, 0.0]);
    let bottom = &results[4].2;
    assert_eq!(numbers(&bottom["origin"]), [0.0, 0.0, 0.0]);
    assert_eq!(numbers(&bottom["yAxis"]), [0.0, -1.0, 0.0]);
    assert_eq!(results[0].2["sketch"], "F1");
    assert_eq!(results[0].2["skipped"], json!([]));
    // The same export twice gives the same bytes.
    let first = std::fs::read(dir.join("top.dxf")).unwrap();
    let again = face_named(&mut s, "endCap");
    run(
        &mut s,
        "file.export",
        json!({"path": "again.dxf", "face": again}),
    );
    assert_eq!(std::fs::read(dir.join("again.dxf")).unwrap(), first);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_unit_and_the_scale_of_the_document_are_those_of_the_file() {
    let (length, _) = plate_numbers();
    for (units, insunits, suffix, factor, k, tolerance) in [
        ("mm", "4", "mm", 1.0, 1.0, 1e-6),
        ("in", "1", "in", 1.0, 1.0, 1e-6),
        ("cm", "5", "cm", 1.0, 0.1, 1e-6),
        ("m", "6", "mm", 1000.0, 1e-3, 1e-9),
        ("um", "13", "mm", 0.001, 1e3, 1e-3),
        ("ft", "2", "in", 12.0, 1.0, 1e-6),
        ("mm", "4", "mm", 1.0, 1e-3, 1e-9),
        ("mm", "4", "mm", 1.0, 1e3, 1e-3),
    ] {
        let (mut s, dir) = session(&format!("units-{units}-{k}"));
        run(
            &mut s,
            "file.new",
            json!({"units": units, "tolerance": tolerance}),
        );
        plate(&mut s, k);
        let top = face_named(&mut s, "endCap");
        for (name, source) in [
            ("sketch", json!({"sketch": "Plate"})),
            ("top", json!({"face": top})),
        ] {
            let mut params = source.clone();
            params["path"] = json!(format!("{name}.dxf"));
            let dxf = run(&mut s, "file.export", params.clone());
            params["path"] = json!(format!("{name}.svg"));
            let svg = run(&mut s, "file.export", params);
            println!(
                "{units} × {k} {name}: length {} (expected {})",
                dxf["length"],
                length * k
            );
            for result in [&dxf, &svg] {
                assert_eq!(result["unit"], units);
                assert!(
                    near(number(&result["length"]), length * k, 1e-11),
                    "{units} {k}: {result}"
                );
                let bbox = numbers(&result["bbox"]);
                for (got, expected) in bbox.iter().zip([0.0, 0.0, 80.0 * k, 50.0 * k]) {
                    assert!(
                        (got - expected).abs() <= 1e-11 * 80.0 * k,
                        "{units} {k}: {bbox:?}"
                    );
                }
                assert_eq!((&result["closed"], &result["arcs"]), (&json!(4), &json!(8)));
            }
            let text = std::fs::read_to_string(dir.join(format!("{name}.dxf"))).unwrap();
            assert_eq!(header(&text, "$INSUNITS"), insunits);
            let text = std::fs::read_to_string(dir.join(format!("{name}.svg"))).unwrap();
            let width = attribute(&text, "width");
            let value: f64 = width.strip_suffix(suffix).unwrap().parse().unwrap();
            assert!(
                near(value, 80.0 * k * factor, 1e-11),
                "{units} {k}: {width}"
            );
            // One user unit is one document unit.
            let view: Vec<f64> = attribute(&text, "viewBox")
                .split(' ')
                .map(|v| v.parse().unwrap())
                .collect();
            assert!(near(view[2], 80.0 * k, 1e-11) && near(view[3], 50.0 * k, 1e-11));
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[test]
fn an_export_leaves_the_document_as_it_was() {
    let (mut s, dir) = session("unchanged");
    run(&mut s, "file.new", json!({}));
    plate(&mut s, 1.0);
    run(&mut s, "file.save", json!({"path": "plate.3dcraft"}));
    let before = (
        run(&mut s, "document.inspect", json!({})),
        run(&mut s, "file.list", json!({})),
    );
    assert_eq!(before.1["documents"][0]["dirty"], false);
    let top = face_named(&mut s, "endCap");
    for params in [
        json!({"path": "a.dxf", "sketch": "Plate"}),
        json!({"path": "a.svg", "sketch": "F1", "construction": true}),
        json!({"path": "b.dxf", "face": top}),
        json!({"path": "b.svg", "face": top, "tolerance": 0.01}),
    ] {
        run(&mut s, "file.export", params);
    }
    // A refused one changes nothing either.
    refused(
        &mut s,
        json!({"path": "c.dxf", "sketch": "Nope"}),
        &["Nope"],
    );
    let after = (
        run(&mut s, "document.inspect", json!({})),
        run(&mut s, "file.list", json!({})),
    );
    assert_eq!(before, after);
    assert!(s.execute("edit.redo", &json!({})).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_curves_of_a_sketch_are_joined_into_chains() {
    let (mut s, dir) = session("chains");
    run(&mut s, "file.new", json!({}));
    run(&mut s, "sketch.create", json!({"name": "S"}));
    let add = |s: &mut Session, id: &str, mut params: Value| {
        params["sketch"] = json!("S");
        run(s, id, params)
    };
    // A rectangle whose corners are shared points: one closed chain.
    add(
        &mut s,
        "sketch.polyline",
        json!({"points": [[0, 0], [10, 0], [10, 5], [0, 5]], "closed": true}),
    );
    // Three lines drawn apart whose ends are at the same places (points
    // of their own, not shared): one open chain of three.
    add(
        &mut s,
        "sketch.line",
        json!({"from": [20, 0], "to": [30, 0]}),
    );
    add(
        &mut s,
        "sketch.line",
        json!({"from": [40, 10], "to": [30, 10]}),
    );
    add(
        &mut s,
        "sketch.line",
        json!({"from": [30, 0], "to": [30, 10]}),
    );
    // A circle, and a half circle closed by a line: two closed paths.
    add(
        &mut s,
        "sketch.circle",
        json!({"center": [5, 2.5], "radius": 1}),
    );
    add(
        &mut s,
        "sketch.arc",
        json!({"center": [50, 0], "start": [55, 0], "end": [45, 0]}),
    );
    add(
        &mut s,
        "sketch.line",
        json!({"from": [45, 0], "to": [55, 0]}),
    );
    // A T: three lines from one point, three open chains.
    for to in [[70, 0], [60, 10], [60, -10]] {
        add(&mut s, "sketch.line", json!({"from": [60, 0], "to": to}));
    }
    // A line without length, and construction geometry.
    add(
        &mut s,
        "sketch.line",
        json!({"from": [80, 0], "to": [80, 0]}),
    );
    add(
        &mut s,
        "sketch.line",
        json!({"from": [-5, 2.5], "to": [15, 2.5], "construction": true}),
    );
    add(
        &mut s,
        "sketch.circle",
        json!({"center": [5, 2.5], "radius": 9, "construction": true}),
    );

    let length = 30.0 + 30.0 + TAU + (5.0 * PI + 10.0) + 30.0;
    let plain = run(
        &mut s,
        "file.export",
        json!({"path": "s.svg", "sketch": "S"}),
    );
    println!("{plain}");
    assert_eq!(
        (
            &plain["paths"],
            &plain["closed"],
            &plain["lines"],
            &plain["arcs"],
            &plain["circles"]
        ),
        (&json!(7), &json!(3), &json!(11), &json!(1), &json!(1))
    );
    assert!(near(number(&plain["length"]), length, 1e-12));
    assert_eq!(plain["skipped"].as_array().unwrap().len(), 1);
    let svg = std::fs::read_to_string(dir.join("s.svg")).unwrap();
    assert_eq!(svg.matches("<path ").count(), 1);
    // The three lines drawn apart are one subpath, in the order drawn
    // and joined head to tail.
    assert!(svg.contains("M20,0 L30,0 L30,-10 L40,-10"), "{svg}");
    assert!(svg.contains("M0,0 L10,0 L10,-5 L0,-5 L0,0 Z"), "{svg}");
    assert!(!svg.contains("layer-construction"));
    let dxf = run(
        &mut s,
        "file.export",
        json!({"path": "s.dxf", "sketch": "S"}),
    );
    let text = std::fs::read_to_string(dir.join("s.dxf")).unwrap();
    assert_eq!(
        (
            entities(&text, "LINE"),
            entities(&text, "ARC"),
            entities(&text, "CIRCLE")
        ),
        (11, 1, 1)
    );
    assert!(near(number(&dxf["length"]), length, 1e-12));
    assert!(!text.contains("\nconstruction\n"));

    // With the construction geometry: a layer of its own, in blue.
    let with = run(
        &mut s,
        "file.export",
        json!({"path": "c.svg", "sketch": "S", "construction": true}),
    );
    assert_eq!(
        (&with["paths"], &with["circles"], &with["lines"]),
        (&json!(9), &json!(2), &json!(12))
    );
    assert!(near(
        number(&with["length"]),
        length + 20.0 + 18.0 * PI,
        1e-12
    ));
    let svg = std::fs::read_to_string(dir.join("c.svg")).unwrap();
    assert_eq!(svg.matches("<path ").count(), 2);
    assert!(svg.contains("id=\"layer-outline\"") && svg.contains("id=\"layer-construction\""));
    assert!(svg.contains("stroke=\"#0000ff\""));
    run(
        &mut s,
        "file.export",
        json!({"path": "c.dxf", "sketch": "S", "construction": true}),
    );
    let text = std::fs::read_to_string(dir.join("c.dxf")).unwrap();
    assert_eq!(
        text.matches("  8\nconstruction\n").count(),
        2,
        "two entities on the layer"
    );
    assert_eq!(
        text.matches("  2\nconstruction\n").count(),
        1,
        "the layer itself"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The length of an ellipse with semi-axes `a` and `b`, by the series in
/// `h = ((a - b) / (a + b))²`.
fn ellipse_length(a: f64, b: f64) -> f64 {
    let h = ((a - b) / (a + b)).powi(2);
    let (mut series, mut term) = (0.0, 1.0_f64);
    for n in 0..80 {
        series += term * term * h.powi(n);
        term *= (0.5 - f64::from(n)) / (f64::from(n) + 1.0);
    }
    PI * (a + b) * series
}

#[test]
fn the_faces_of_primitives_and_a_slanted_cut_keep_their_curves() {
    let (mut s, dir) = session("faces");
    run(&mut s, "file.new", json!({}));
    // A block: its top is a rectangle.
    run(
        &mut s,
        "feature.block",
        json!({"size": [40, 20, 10], "name": "Block"}),
    );
    let top = run(
        &mut s,
        "file.export",
        json!({"path": "block.dxf", "face": "F1:face(1)"}),
    );
    assert_eq!(
        (&top["paths"], &top["closed"], &top["lines"]),
        (&json!(1), &json!(1), &json!(4))
    );
    assert!(near(number(&top["length"]), 120.0, 1e-12));
    // A cylinder: its top is a circle, and its wall is not flat.
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": 5, "height": 30, "position": [100, 0, 0], "name": "Pin"}),
    );
    let disc = run(
        &mut s,
        "file.export",
        json!({"path": "disc.svg", "face": "F2:face(2)"}),
    );
    println!("{disc}");
    assert_eq!((&disc["paths"], &disc["closed"]), (&json!(1), &json!(1)));
    assert!(near(number(&disc["length"]), TAU * 5.0, 1e-12));
    let bbox = numbers(&disc["bbox"]);
    assert!(near(bbox[2] - bbox[0], 10.0, 1e-12) && near(bbox[3] - bbox[1], 10.0, 1e-12));
    let dxf = run(
        &mut s,
        "file.export",
        json!({"path": "disc.dxf", "face": "F2:face(2)"}),
    );
    let text = std::fs::read_to_string(dir.join("disc.dxf")).unwrap();
    let circles = entities(&text, "CIRCLE");
    let arcs = entities(&text, "ARC");
    println!("the disc: {circles} circles, {arcs} arcs; {dxf}");
    assert!(
        circles == 1 || arcs >= 2,
        "a circle or its arcs, never lines"
    );
    assert_eq!(entities(&text, "LINE"), 0);
    refused(
        &mut s,
        json!({"path": "wall.dxf", "face": "F2:face(0)"}),
        &["cylinder face", "flat"],
    );
    assert!(!dir.join("wall.dxf").exists());

    // The cylinder cut by a block whose bottom leans 30°, with one corner
    // on the cylinder's axis: the cut is a quarter of an ellipse with
    // semi-axes 5 / cos 30° and 5, and the two lines to its centre.
    let (a, b) = (5.0 / (PI / 6.0).cos(), 5.0);
    let lean = [0.5, 0.0, (PI / 6.0).cos()];
    run(
        &mut s,
        "feature.block",
        json!({"size": [60, 60, 40], "position": [100, 0, 20], "axis": lean, "name": "Knife"}),
    );
    let cut = run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Pin", "tool": "Knife", "name": "Cut"}),
    );
    assert_eq!(cut["status"], "ok", "{cut}");
    let face = "F4:copy(1)/F3:face(0)";
    let quarter = run(
        &mut s,
        "file.export",
        json!({"path": "quarter.dxf", "face": face}),
    );
    println!("{quarter}");
    assert_eq!(
        (
            &quarter["paths"],
            &quarter["closed"],
            &quarter["lines"],
            &quarter["ellipses"]
        ),
        (&json!(1), &json!(1), &json!(2), &json!(1))
    );
    let expected = a + b + ellipse_length(a, b) / 4.0;
    assert!(
        near(number(&quarter["length"]), expected, 1e-11),
        "{expected}"
    );
    let bbox = numbers(&quarter["bbox"]);
    let (width, height) = (bbox[2] - bbox[0], bbox[3] - bbox[1]);
    assert!(
        near(width.max(height), a, 1e-11) && near(width.min(height), b, 1e-11),
        "{bbox:?}"
    );
    let text = std::fs::read_to_string(dir.join("quarter.dxf")).unwrap();
    assert_eq!(
        (
            entities(&text, "ELLIPSE"),
            entities(&text, "LINE"),
            entities(&text, "SPLINE")
        ),
        (1, 2, 0)
    );
    let svg = run(
        &mut s,
        "file.export",
        json!({"path": "quarter.svg", "face": face}),
    );
    assert_eq!(svg["approximated"], 0);
    let text = std::fs::read_to_string(dir.join("quarter.svg")).unwrap();
    // One arc command with two different radii, and no curve.
    assert_eq!(text.matches(" A").count(), 1, "{text}");
    assert!(!text.contains(" C") && !text.contains(" Q"));

    // The same knife moved so that it takes the whole top: a whole
    // ellipse.
    run(&mut s, "file.new", json!({}));
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": 5, "height": 30, "name": "Pin"}),
    );
    // The block's own x leans up and away: (-cos 30°, 0, sin 30°).
    let corner = [30.0 * lean[2], -30.0, 20.0 - 30.0 * lean[0]];
    run(
        &mut s,
        "feature.block",
        json!({"size": [60, 60, 40], "position": corner, "axis": lean, "name": "Knife"}),
    );
    let cut = run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Pin", "tool": "Knife", "name": "Cut"}),
    );
    assert_eq!(cut["status"], "ok", "{cut}");
    let face = "F3:copy(1)/F2:face(0)";
    let whole = run(
        &mut s,
        "file.export",
        json!({"path": "whole.dxf", "face": face}),
    );
    println!("{whole}");
    assert_eq!(
        (
            &whole["paths"],
            &whole["closed"],
            &whole["lines"],
            &whole["arcs"]
        ),
        (&json!(1), &json!(1), &json!(0), &json!(0))
    );
    assert!(whole["ellipses"].as_u64().unwrap() >= 1);
    assert!(
        near(number(&whole["length"]), ellipse_length(a, b), 1e-11),
        "{whole}"
    );
    let bbox = numbers(&whole["bbox"]);
    let (width, height) = (bbox[2] - bbox[0], bbox[3] - bbox[1]);
    assert!(near(width.max(height), 2.0 * a, 1e-11) && near(width.min(height), 2.0 * b, 1e-11));
    let text = std::fs::read_to_string(dir.join("whole.dxf")).unwrap();
    assert!(entities(&text, "ELLIPSE") >= 1);
    assert_eq!(entities(&text, "LINE") + entities(&text, "SPLINE"), 0);
    let svg = run(
        &mut s,
        "file.export",
        json!({"path": "whole.svg", "face": face}),
    );
    assert_eq!(svg["approximated"], 0);
    assert!(near(number(&svg["length"]), ellipse_length(a, b), 1e-11));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn what_cannot_be_drawn_fails_with_the_reason() {
    let (mut s, dir) = session("refused");
    refused(
        &mut s,
        json!({"path": "a.dxf", "sketch": "Plate"}),
        &["no document"],
    );
    run(&mut s, "file.new", json!({}));
    refused(
        &mut s,
        json!({"path": "a.dxf", "sketch": "Plate"}),
        &["Plate"],
    );
    run(&mut s, "sketch.create", json!({"name": "Empty"}));
    refused(
        &mut s,
        json!({"path": "a.dxf", "sketch": "Empty"}),
        &["no lines, arcs or circles", "construction"],
    );
    run(
        &mut s,
        "sketch.line",
        json!({"sketch": "Empty", "from": [0, 0], "to": [1, 0], "construction": true}),
    );
    refused(
        &mut s,
        json!({"path": "a.svg", "sketch": "Empty"}),
        &["construction"],
    );
    run(
        &mut s,
        "file.export",
        json!({"path": "only.svg", "sketch": "Empty", "construction": true}),
    );
    plate(&mut s, 1.0);
    let top = face_named(&mut s, "endCap");

    // What to draw.
    refused(
        &mut s,
        json!({"path": "a.dxf"}),
        &["give `sketch`", "`face`"],
    );
    refused(
        &mut s,
        json!({"path": "a", "format": "svg"}),
        &["give `sketch`"],
    );
    refused(
        &mut s,
        json!({"path": "a.dxf", "sketch": "Plate", "face": top}),
        &["either a `sketch` or a `face`"],
    );
    refused(
        &mut s,
        json!({"path": "a.dxf", "sketch": "Body"}),
        &["sketch"],
    );
    refused(&mut s, json!({"path": "a.dxf", "sketch": "F99"}), &["F99"]);
    refused(&mut s, json!({"path": "a.dxf", "sketch": 7.5}), &["sketch"]);
    refused(
        &mut s,
        json!({"path": "a.dxf", "sketch": ["Plate"]}),
        &["sketch"],
    );
    refused(
        &mut s,
        json!({"path": "a.dxf", "sketch": "Plate", "feature": "Body"}),
        &["`feature`"],
    );
    refused(
        &mut s,
        json!({"path": "a.dxf", "sketch": "Plate", "construction": "yes"}),
        &["construction"],
    );
    // Faces.
    refused(
        &mut s,
        json!({"path": "a.dxf", "face": "nonsense"}),
        &["`face`"],
    );
    refused(&mut s, json!({"path": "a.dxf", "face": 3}), &["face"]);
    refused(
        &mut s,
        json!({"path": "a.dxf", "face": "F3:endCap(e1)"}),
        &["no body has a face", "faceNames"],
    );
    refused(
        &mut s,
        json!({"path": "a.dxf", "face": "F3:side(e99)", "feature": "Body"}),
        &["`Body` has no face"],
    );
    refused(
        &mut s,
        json!({"path": "a.dxf", "face": top, "feature": "Plate"}),
        &["`Plate` has no body"],
    );
    refused(
        &mut s,
        json!({"path": "a.dxf", "face": top, "feature": "Nope"}),
        &["Nope"],
    );
    refused(
        &mut s,
        json!({"path": "a.dxf", "face": top, "construction": true}),
        &["`construction` is about a sketch"],
    );
    // The side that an arc of the outline swept is a cylinder.
    let inspect = run(&mut s, "document.inspect", json!({}));
    let sides: Vec<&str> = inspect["bodies"][0]["faceNames"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .filter(|name| name.contains("side"))
        .collect();
    let mut round = 0;
    for side in sides {
        match s.execute("file.export", &json!({"path": "side.dxf", "face": side})) {
            Ok(result) => assert_eq!(result["lines"], 4, "{side}"),
            Err(error) => {
                let error = error.to_string();
                assert!(
                    error.contains("cylinder face") && error.contains(side),
                    "{error}"
                );
                round += 1;
            }
        }
    }
    assert_eq!(round, 8, "one round side per arc of the sketch");
    // Options.
    refused(
        &mut s,
        json!({"path": "a.dxf", "sketch": "Plate", "tolerance": 0.1}),
        &["`tolerance` is for SVG"],
    );
    for tolerance in [json!(0), json!(-1), json!("fine")] {
        refused(
            &mut s,
            json!({"path": "a.svg", "sketch": "Plate", "tolerance": tolerance}),
            &["tolerance"],
        );
    }
    refused(
        &mut s,
        json!({"path": "a.dxf", "sketch": "Plate", "format": "dwg"}),
        &["unknown format"],
    );
    refused(
        &mut s,
        json!({"path": "a.dwg", "sketch": "Plate"}),
        &["cannot tell the format", ".dxf"],
    );
    refused(
        &mut s,
        json!({"path": "a.dxf", "sketch": "Plate", "layer": "cut"}),
        &["layer"],
    );
    // The other formats hold bodies, not a sketch or a face.
    for (key, value) in [
        ("sketch", json!("Plate")),
        ("face", json!(top)),
        ("feature", json!("Body")),
        ("construction", json!(true)),
    ] {
        let mut params = json!({"path": "a.stl"});
        params[key] = value;
        refused(&mut s, params.clone(), &[key, ".dxf or .svg"]);
        params["path"] = json!("a.step");
        refused(&mut s, params, &[key, ".dxf or .svg"]);
    }
    // Where to write.
    for path in ["../a.dxf", "/etc/a.dxf", "a/../../b.svg"] {
        refused(&mut s, json!({"path": path, "sketch": "Plate"}), &[path]);
    }
    refused(
        &mut s,
        json!({"path": "", "format": "dxf", "sketch": "Plate"}),
        &["path"],
    );
    // A directory that does not exist, and a file where a directory
    // should be: the write fails, and says so.
    refused(
        &mut s,
        json!({"path": "missing/a.dxf", "sketch": "Plate"}),
        &["cannot", "a.dxf"],
    );
    refused(
        &mut s,
        json!({"path": "only.svg/a.dxf", "sketch": "Plate"}),
        &["cannot", "a.dxf"],
    );
    let left: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name != "only.svg" && name != "side.dxf")
        .collect();
    assert!(left.is_empty(), "nothing else was written: {left:?}");
    // Without a write root nothing is written at all.
    let mut closed = Session::new(Workspace::none());
    run(&mut closed, "file.new", json!({}));
    run(&mut closed, "sketch.create", json!({"name": "S"}));
    run(
        &mut closed,
        "sketch.circle",
        json!({"sketch": "S", "center": [0, 0], "radius": 1}),
    );
    refused(
        &mut closed,
        json!({"path": "a.dxf", "sketch": "S"}),
        &["no write access"],
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_sketch_that_failed_is_not_exported() {
    let (mut s, dir) = session("failed");
    run(&mut s, "file.new", json!({}));
    run(&mut s, "sketch.create", json!({"name": "S"}));
    run(
        &mut s,
        "sketch.line",
        json!({"sketch": "S", "from": [0, 0], "to": [10, 0]}),
    );
    // Two lengths for one line cannot both hold.
    let first = s.execute(
        "sketch.constrain",
        &json!({"sketch": "S", "type": "distance", "entities": ["e1", "e2"], "value": 10}),
    );
    let second = s.execute(
        "sketch.constrain",
        &json!({"sketch": "S", "type": "distance", "entities": ["e1", "e2"], "value": 20}),
    );
    println!("{first:?}\n{second:?}");
    let inspect = run(&mut s, "document.inspect", json!({}));
    let status = inspect["features"][0]["status"].clone();
    println!("status: {status}");
    assert_eq!(status, "failed");
    let error = s
        .execute("file.export", &json!({"path": "s.dxf", "sketch": "S"}))
        .unwrap_err()
        .to_string();
    assert!(error.contains("the sketch `S` failed"), "{error}");
    assert!(error.contains("c2"), "{error}");
    assert!(!dir.join("s.dxf").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}
