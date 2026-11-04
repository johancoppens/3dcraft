//! Oracle 8 (docs/testing.md) for flat outlines: FreeCAD imports the DXF
//! and SVG files `file.export` writes for a sketch and for faces, and
//! measures what it got: the length of the edges, their bounding box, how
//! many closed wires they join into and the area inside.
//!
//! Opt-in, because it needs FreeCAD:
//!
//! ```sh
//! CRAFT3D_FREECAD=/path/to/FreeCAD.AppImage cargo test -p craft3d-engine --test drawing_freecad -- --ignored --nocapture
//! ```
//!
//! The script is the one of `craft3d-drawing`
//! (`crates/drawing/tests/drawing_report.py`), whose own FreeCAD test
//! covers the writers: units, scales, ellipses and splines.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;

use common::plain::run;
use craft3d_engine::{Access, Session, Workspace};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::f64::consts::PI;
use std::path::PathBuf;
use std::process::Command;

/// FreeCAD's report per file of `dir`: key → value.
fn freecad_reads(dir: &std::path::Path) -> BTreeMap<String, BTreeMap<String, String>> {
    let freecad = std::env::var("CRAFT3D_FREECAD").expect(
        "set CRAFT3D_FREECAD to a FreeCAD AppImage or FreeCADCmd executable to run these tests",
    );
    let output = dir.join("report.txt");
    let script =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../drawing/tests/drawing_report.py");
    let mut command = Command::new(&freecad);
    if freecad.ends_with(".AppImage") {
        command.arg("freecadcmd");
    }
    let run = command
        .arg(&script)
        .env("CRAFT3D_FC_IN", dir)
        .env("CRAFT3D_FC_OUT", &output)
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap_or_else(|e| panic!("cannot run {freecad}: {e}"));
    let text = std::fs::read_to_string(&output).unwrap_or_else(|_| {
        panic!(
            "FreeCAD wrote no report ({}):\n{}\n{}",
            run.status,
            String::from_utf8_lossy(&run.stdout),
            String::from_utf8_lossy(&run.stderr)
        )
    });
    text.lines()
        .map(|line| {
            let fields: BTreeMap<String, String> = line
                .split('\t')
                .filter_map(|f| f.split_once('='))
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            if let Some(error) = fields.get("error") {
                panic!("FreeCAD could not read {}: {error}", fields["file"]);
            }
            (fields["file"].clone(), fields)
        })
        .collect()
}

fn near(a: f64, b: f64, relative: f64) -> bool {
    (a - b).abs() <= relative * a.abs().max(b.abs())
}

/// The plate of the demo: 80 × 50 with corners of radius 8, a slot, a
/// keyhole and a D hole, 6 thick.
fn plate(s: &mut Session) {
    run(s, "sketch.create", json!({"name": "Plate"}));
    let paths = [
        json!({"start": [8, 0], "segments": [
            {"line": [72, 0], "tangent": true}, {"arc": [80, 8], "tangent": true},
            {"line": [80, 42], "tangent": true}, {"arc": [72, 50], "radius": 8, "tangent": true},
            {"line": [8, 50], "tangent": true}, {"arc": [0, 42], "center": [8, 42], "tangent": true},
            {"line": [0, 8], "tangent": true}, {"arc": "start", "center": [8, 8], "tangent": true}]}),
        json!({"start": [12, 10], "segments": [
            {"line": [32, 10]}, {"arc": [32, 18], "tangent": true},
            {"line": [12, 18], "tangent": true}, {"arc": "start", "tangent": true}]}),
        json!({"start": [58, 12], "closed": true, "segments": [
            {"line": [62, 12]}, {"line": [62, 28]}, {"arc": [58, 28], "radius": 7, "large": true}]}),
        json!({"start": [16, 30], "closed": true, "segments": [
            {"arc": [16, 42], "radius": 7.5, "large": true}]}),
    ];
    for mut path in paths {
        path["sketch"] = json!("Plate");
        run(s, "sketch.path", path);
    }
    run(
        s,
        "feature.extrude",
        json!({"sketch": "Plate", "distance": 6, "name": "Body"}),
    );
}

#[test]
#[ignore = "needs FreeCAD: set CRAFT3D_FREECAD and run with --ignored"]
fn freecad_reads_the_outline_of_a_sketch_and_of_faces() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("drawing-freecad");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let workspace = || Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    };
    // (file, our result, the area inside by a closed form or the
    // kernel's own measure, millimetres per unit, whether FreeCAD's DXF
    // reader can be held to it).
    let mut cases: Vec<(String, Value, f64, f64)> = Vec::new();
    for (units, to_mm) in [("mm", 1.0), ("in", 25.4)] {
        let mut s = Session::new(workspace());
        run(&mut s, "file.new", json!({"units": units}));
        plate(&mut s);
        // The area of the plate's top, from the solid: volume over height.
        let area = run(&mut s, "document.measure", json!({}))["volume"]
            .as_f64()
            .unwrap()
            / 6.0;
        let inspect = run(&mut s, "document.inspect", json!({}));
        let face = |role: &str| -> String {
            inspect["bodies"][0]["faceNames"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .find(|name| name.contains(role))
                .unwrap()
                .to_string()
        };
        for (name, source) in [
            ("sketch", json!({"sketch": "Plate"})),
            ("top", json!({"face": face("endCap")})),
            ("bottom", json!({"face": face("startCap")})),
        ] {
            for format in ["dxf", "svg"] {
                let file = format!("plate-{units}-{name}.{format}");
                let mut params = source.clone();
                params["path"] = json!(file);
                let result = run(&mut s, "file.export", params);
                cases.push((file, result, area, to_mm));
            }
        }
        // The top and the bottom side by side on one sheet, each on a
        // layer of its own. FreeCAD's area is the largest closed wire less
        // all others: one plate's outline less the other's and both
        // plates' holes.
        let outline = 80.0 * 50.0 - (4.0 - PI) * 64.0;
        let holes = outline - area;
        for format in ["dxf", "svg"] {
            let file = format!("sheet-{units}.{format}");
            let result = run(
                &mut s,
                "file.export",
                json!({"path": file, "faces": [face("endCap"), face("startCap")], "gap": 5}),
            );
            assert_eq!(result["rows"], 1, "{result}");
            cases.push((file, result, -2.0 * holes, to_mm));
        }
    }
    // A cylinder of radius 5 cut on a slant of 30°: an ellipse with
    // semi-axes 5 / cos 30° and 5, whole.
    let mut s = Session::new(workspace());
    run(&mut s, "file.new", json!({}));
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": 5, "height": 30, "name": "Pin"}),
    );
    let lean = [0.5, 0.0, (PI / 6.0).pcos()];
    let corner = [30.0 * lean[2], -30.0, 20.0 - 30.0 * lean[0]];
    run(
        &mut s,
        "feature.block",
        json!({"size": [60, 60, 40], "position": corner, "axis": lean, "name": "Knife"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Pin", "tool": "Knife", "name": "Cut"}),
    );
    let ellipse_area = PI * 5.0 * 5.0 / (PI / 6.0).pcos();
    for format in ["dxf", "svg"] {
        let file = format!("slant.{format}");
        let result = run(
            &mut s,
            "file.export",
            json!({"path": file, "face": "F3:copy(1)/F2:face(0)"}),
        );
        assert_eq!(result["ellipses"], 1, "{result}");
        cases.push((file, result, ellipse_area, 1.0));
    }

    let reports = freecad_reads(&dir);
    assert_eq!(reports.len(), cases.len());
    for (file, ours, area, to_mm) in &cases {
        let report = &reports[file];
        let number = |key: &str| -> f64 { report[key].parse().unwrap() };
        let length = ours["length"].as_f64().unwrap() * to_mm;
        let area = area * to_mm * to_mm;
        println!(
            "{file}: length {} (ours {length}), area {} (expected {area}), bbox {}, {} edges ({}), {} closed of {} wires",
            report["length"],
            report["area"],
            report["bbox"],
            report["edges"],
            report["kinds"],
            report["closed"],
            report["wires"]
        );
        assert!(near(number("length"), length, 1e-9), "{file}");
        assert!(near(number("area"), area, 1e-9), "{file}: {area}");
        // As many closed wires as we have closed paths, and nothing left
        // open.
        assert_eq!(report["closed"], ours["closed"].to_string(), "{file}");
        assert_eq!(report["wires"], ours["paths"].to_string(), "{file}");
        let bbox: Vec<f64> = report["bbox"]
            .split(' ')
            .map(|v| v.parse().unwrap())
            .collect();
        let size = (bbox[2] - bbox[0]).max(bbox[3] - bbox[1]);
        for (got, expected) in bbox.iter().zip(ours["bbox"].as_array().unwrap()) {
            let expected = expected.as_f64().unwrap() * to_mm;
            assert!((got - expected).abs() <= 1e-9 * size, "{file}: {bbox:?}");
        }
        if file.ends_with(".dxf") && *to_mm == 1.0 {
            // Every curve arrived as what it is.
            let kinds = &report["kinds"];
            let expected = if file.starts_with("slant") {
                "Ellipse:1".to_string()
            } else if file.starts_with("sheet") {
                "Circle:16,Line:20".to_string()
            } else {
                "Circle:8,Line:10".to_string()
            };
            assert_eq!(*kinds, expected, "{file}");
        }
    }
}

/// The length of the elliptic arc of semi-axes `a` and `a ratio` from
/// parameter `t0` to `t1`, by Simpson's rule on 4000 panels.
fn elliptic_arc(a: f64, ratio: f64, t0: f64, t1: f64) -> f64 {
    let b = a * ratio;
    let speed = |t: f64| (a * t.psin()).phypot(b * t.pcos());
    let n = 4000;
    let h = (t1 - t0) / f64::from(n);
    let mut sum = speed(t0) + speed(t1);
    for i in 1..n {
        sum += speed(t0 + h * f64::from(i)) * if i % 2 == 1 { 4.0 } else { 2.0 };
    }
    sum * h / 3.0
}

/// The ELLIPSE entities of a DXF file that are arcs: (a, ratio, start,
/// end).
fn elliptic_arcs(dxf: &str) -> Vec<(f64, f64, f64, f64)> {
    let start = dxf.find("  2\nENTITIES\n").unwrap();
    let mut out = Vec::new();
    for entity in dxf[start..].split("  0\n").skip(1) {
        if !entity.starts_with("ELLIPSE\n") {
            continue;
        }
        let groups: Vec<&str> = entity.lines().collect();
        let value = |code: &str| -> f64 {
            let at = groups.iter().position(|g| g.trim() == code).unwrap();
            groups[at + 1].trim().parse().unwrap()
        };
        let (t0, t1) = (value("41"), value("42"));
        if (t1 - t0 - std::f64::consts::TAU).abs() > 1e-12 {
            out.push((value("11").phypot(value("21")), value("40"), t0, t1));
        }
    }
    out
}

/// The model as seen (`view_export.rs`): FreeCAD reads the visible,
/// silhouette and dashed hidden lines of a drilled block in iso, from the
/// front and from the top, and a D of a cubic and a line extruded, from
/// the top and in iso: lines, circles, ellipses, elliptic arcs and
/// splines (since 2026-10-28; polylines before), as long as ours and in
/// the same box. FreeCAD's DXF reader (1.1) draws every ELLIPSE whole,
/// whatever its start and end parameters (groups 41 and 42; see the
/// writers' own FreeCAD test): from a DXF with elliptic arcs it gets each
/// arc's whole ellipse, so its length is ours with every elliptic arc
/// made whole, and its box is not compared. The SVG it reads as written.
#[test]
#[ignore = "needs FreeCAD: set CRAFT3D_FREECAD and run with --ignored"]
fn freecad_reads_a_projected_view() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("drawing-freecad-view");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [40, 30, 10], "name": "Block"}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": 5, "height": 20, "position": [20, 15, -5], "name": "Drill"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Block", "tool": "Drill", "name": "Hole"}),
    );
    let mut cases = Vec::new();
    for (view, hidden) in [("iso", "dashed"), ("front", "dashed"), ("top", "omit")] {
        for format in ["dxf", "svg"] {
            let file = format!("{view}.{format}");
            let result = run(
                &mut s,
                "file.export",
                json!({"path": file, "view": view, "hidden": hidden}),
            );
            cases.push((file, result));
        }
    }
    // A spline edge: the D of the sketch tests, extruded.
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "sketch.create", json!({"name": "D"}));
    run(
        &mut s,
        "sketch.spline",
        json!({"sketch": "D", "degree": 3, "control": [[0, 0], [10, 0], [10, 10], [0, 10]]}),
    );
    run(
        &mut s,
        "sketch.line",
        json!({"sketch": "D", "from": [0, 10], "to": [0, 0]}),
    );
    run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "D", "distance": 5}),
    );
    for (view, hidden) in [("top", "omit"), ("iso", "dashed")] {
        for format in ["dxf", "svg"] {
            let file = format!("d-{view}.{format}");
            let result = run(
                &mut s,
                "file.export",
                json!({"path": file, "view": view, "hidden": hidden}),
            );
            cases.push((file, result));
        }
    }
    let reports = freecad_reads(&dir);
    assert_eq!(reports.len(), cases.len());
    for (file, ours) in &cases {
        let report = &reports[file];
        let number = |key: &str| -> f64 { report[key].parse().unwrap() };
        let mut length = ours["length"].as_f64().unwrap();
        let mut whole = false;
        if file.ends_with(".dxf") {
            let text = std::fs::read_to_string(dir.join(file)).unwrap();
            for (a, ratio, t0, t1) in elliptic_arcs(&text) {
                let full = elliptic_arc(a, ratio, 0.0, std::f64::consts::TAU);
                length += full - elliptic_arc(a, ratio, t0, t1);
                whole = true;
            }
        }
        println!(
            "{file}: length {} (ours {length}), bbox {} (ours {}), {} edges ({})",
            report["length"], report["bbox"], ours["bbox"], report["edges"], report["kinds"]
        );
        assert!(near(number("length"), length, 1e-9), "{file}");
        if whole {
            continue;
        }
        let bbox: Vec<f64> = report["bbox"]
            .split(' ')
            .map(|v| v.parse().unwrap())
            .collect();
        let size = (bbox[2] - bbox[0]).max(bbox[3] - bbox[1]);
        // Our box of a spline is the box of its Bézier pieces' control
        // points (`Drawing::bbox`), which holds the curve: FreeCAD's tight
        // box lies inside it.
        let splined = report["kinds"].contains("BSpline") || report["kinds"].contains("Bezier");
        for (k, (got, expected)) in bbox
            .iter()
            .zip(ours["bbox"].as_array().unwrap())
            .enumerate()
        {
            let expected = expected.as_f64().unwrap();
            if splined {
                let inside = if k < 2 {
                    *got >= expected - 1e-9 * size
                } else {
                    *got <= expected + 1e-9 * size
                };
                assert!(inside, "{file}: {bbox:?}");
            } else {
                assert!((got - expected).abs() <= 1e-9 * size, "{file}: {bbox:?}");
            }
        }
    }
}

/// A fold's crease pattern (`file.export` with `fold`): FreeCAD reads the
/// outline and the mountain (dash-dot) and valley (dashed) lines of a
/// waterbomb base as straight edges, as long as ours and in the same box.
#[test]
#[ignore = "needs FreeCAD: set CRAFT3D_FREECAD and run with --ignored"]
fn freecad_reads_a_crease_pattern() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("drawing-freecad-fold");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    run(&mut s, "file.new", json!({"units": "mm"}));
    let c = 50.0;
    run(&mut s, "sketch.create", json!({"name": "Paper"}));
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Paper", "points": [[-c, -c], [c, -c], [c, c], [-c, c]], "closed": true}),
    );
    let mut lines = Vec::new();
    for (from, to) in [
        ([-c, 0.0], [c, 0.0]),
        ([-c, -c], [c, c]),
        ([c, -c], [-c, c]),
    ] {
        let made = run(
            &mut s,
            "sketch.line",
            json!({"sketch": "Paper", "from": from, "to": to}),
        );
        lines.push(made["entity"].clone());
    }
    run(
        &mut s,
        "feature.fold",
        json!({"sketch": "Paper", "name": "Waterbomb", "creases": [
            {"line": lines[0], "kind": "mountain", "angle": "36.03846874550981 deg"},
            {"line": lines[1], "kind": "valley", "angle": "49.40777113928209 deg"},
            {"line": lines[2], "kind": "valley"}]}),
    );
    let mut cases = Vec::new();
    for format in ["dxf", "svg"] {
        let file = format!("waterbomb.{format}");
        let result = run(
            &mut s,
            "file.export",
            json!({"path": file, "fold": "Waterbomb"}),
        );
        cases.push((file, result));
    }
    let reports = freecad_reads(&dir);
    assert_eq!(reports.len(), cases.len());
    let length = 8.0 * c + 2.0 * c + 4.0 * 2f64.sqrt() * c;
    for (file, ours) in &cases {
        let report = &reports[file];
        let number = |key: &str| -> f64 { report[key].parse().unwrap() };
        println!(
            "{file}: length {} (ours {}), bbox {}, {} edges ({})",
            report["length"], ours["length"], report["bbox"], report["edges"], report["kinds"]
        );
        assert!(
            near(ours["length"].as_f64().unwrap(), length, 1e-12),
            "{file}"
        );
        assert!(near(number("length"), length, 1e-9), "{file}");
        let bbox: Vec<f64> = report["bbox"]
            .split(' ')
            .map(|v| v.parse().unwrap())
            .collect();
        for (got, expected) in bbox.iter().zip([-c, -c, c, c]) {
            assert!((got - expected).abs() <= 1e-9 * c, "{file}: {bbox:?}");
        }
        if file.ends_with(".dxf") {
            assert_eq!(report["kinds"], "Line:7", "{file}");
        }
    }
}

/// A sheet-metal part's flat pattern (`file.export` with `sheetMetal`):
/// FreeCAD reads the outline of a tray with rect reliefs and its bend
/// lines, as long as ours and in the box of the closed form.
#[test]
#[ignore = "needs FreeCAD: set CRAFT3D_FREECAD and run with --ignored"]
fn freecad_reads_a_flat_pattern() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("drawing-freecad-sheetmetal");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "name": "Base"}),
    );
    let made = run(
        &mut s,
        "sketch.rect",
        json!({"sketch": "Base", "from": [0, 0], "to": [120, 80]}),
    );
    let flanges: Vec<Value> = made["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| json!({"edge": e, "length": "20 mm", "angle": "90 deg"}))
        .collect();
    run(
        &mut s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "name": "Tray", "thickness": "1.5 mm", "radius": "2 mm",
               "kFactor": 0.42, "relief": {"shape": "rect", "size": "4 mm"}, "flanges": flanges}),
    );
    let ba = 0.5 * PI * (2.0 + 0.42 * 1.5);
    let reach = ba + 20.0;
    let mut cases = Vec::new();
    for format in ["dxf", "svg"] {
        let file = format!("tray.{format}");
        let result = run(
            &mut s,
            "file.export",
            json!({"path": file, "sheetMetal": "Tray"}),
        );
        cases.push((file, result));
    }
    let reports = freecad_reads(&dir);
    assert_eq!(reports.len(), cases.len());
    for (file, ours) in &cases {
        let report = &reports[file];
        let number = |key: &str| -> f64 { report[key].parse().unwrap() };
        println!(
            "{file}: length {} (ours {}), bbox {}, {} edges ({})",
            report["length"], ours["length"], report["bbox"], report["edges"], report["kinds"]
        );
        assert!(
            near(number("length"), ours["length"].as_f64().unwrap(), 1e-9),
            "{file}"
        );
        let bbox: Vec<f64> = report["bbox"]
            .split(' ')
            .map(|v| v.parse().unwrap())
            .collect();
        for (got, expected) in bbox
            .iter()
            .zip([-reach, -reach, 120.0 + reach, 80.0 + reach])
        {
            assert!((got - expected).abs() <= 1e-9 * 120.0, "{file}: {bbox:?}");
        }
    }
}

/// The mini-PC chassis example's three flat patterns on one sheet
/// (`docs/examples/chassis.json`: louvres bent out of slots, holes across
/// the lid's bends, a hem, a jog, closed mitre corners): FreeCAD reads the
/// DXF and the SVG with our length, our box, and as many closed wires as
/// the export writes chains.
#[test]
#[ignore = "needs FreeCAD: set CRAFT3D_FREECAD and run with --ignored"]
fn freecad_reads_the_chassis_flat_patterns_on_one_sheet() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("drawing-freecad-chassis");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    run(&mut s, "file.new", json!({"units": "mm"}));
    let script: Value = serde_json::from_str(
        &std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/examples/chassis.json"),
        )
        .unwrap(),
    )
    .unwrap();
    for step in script.as_array().unwrap() {
        let id = step[0].as_str().unwrap();
        // The checks are the example's own test's; here only the parts.
        if id == "assert.add" || id == "document.check" {
            continue;
        }
        run(&mut s, id, step[1].clone());
    }
    let parts = json!(["Tray", "Lid", "Drive bracket"]);
    let mut cases = Vec::new();
    for format in ["dxf", "svg"] {
        let file = format!("chassis.{format}");
        let result = run(
            &mut s,
            "file.export",
            json!({"path": file, "sheetMetal": parts}),
        );
        cases.push((file, result));
    }
    let reports = freecad_reads(&dir);
    assert_eq!(reports.len(), cases.len());
    for (file, ours) in &cases {
        let report = &reports[file];
        let number = |key: &str| -> f64 { report[key].parse().unwrap() };
        println!(
            "{file}: length {} (ours {}), bbox {} (ours {}), wires {} (ours {})",
            report["length"],
            ours["length"],
            report["bbox"],
            ours["bbox"],
            report.get("closed").map_or("?", String::as_str),
            ours["closed"]
        );
        assert!(
            near(number("length"), ours["length"].as_f64().unwrap(), 1e-9),
            "{file}"
        );
        let bbox: Vec<f64> = report["bbox"]
            .split(' ')
            .map(|v| v.parse().unwrap())
            .collect();
        let want: Vec<f64> = ours["bbox"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect();
        let size = want[2] - want[0];
        for (got, expected) in bbox.iter().zip(&want) {
            assert!(
                (got - expected).abs() <= 1e-9 * size,
                "{file}: {bbox:?} against {want:?}"
            );
        }
        if let Some(closed) = report.get("closed") {
            assert_eq!(
                closed.parse::<u64>().unwrap(),
                ours["closed"].as_u64().unwrap(),
                "{file}"
            );
        }
    }
}

/// The outline of a glyph with holes, from a text in each built-in font
/// and in the single-stroke font: FreeCAD reads the DXF and SVG of the
/// sketch, its wires the glyph's contours, its area the glyph's (outer
/// less holes) as the kernel measures the extruded glyph.
#[test]
#[ignore = "needs FreeCAD: set CRAFT3D_FREECAD and run with --ignored"]
fn freecad_reads_the_outline_of_text() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("drawing-freecad-text");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let workspace = || Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    };
    let mut expected: BTreeMap<String, (f64, usize)> = BTreeMap::new();
    for (font, glyph, contours) in [("sans", "B", 3), ("mono", "B", 3), ("sans", "O", 2)] {
        let mut s = Session::new(workspace());
        run(&mut s, "file.new", json!({}));
        run(&mut s, "sketch.create", json!({"name": "Glyph"}));
        run(
            &mut s,
            "sketch.text",
            json!({"sketch": "Glyph", "text": glyph, "font": font, "size": 20}),
        );
        run(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Glyph", "distance": 1, "name": "G"}),
        );
        let area = run(&mut s, "document.measure", json!({}))["volume"]
            .as_f64()
            .unwrap();
        for format in ["dxf", "svg"] {
            let file = format!("{font}-{glyph}.{format}");
            run(
                &mut s,
                "file.export",
                json!({"path": file, "sketch": "Glyph"}),
            );
            expected.insert(file, (area, contours));
        }
    }
    // The stroke font: open strokes, as many wires as strokes (an I and
    // an L: one stroke each).
    let mut s = Session::new(workspace());
    run(&mut s, "file.new", json!({}));
    run(&mut s, "sketch.create", json!({"name": "Strokes"}));
    run(
        &mut s,
        "sketch.text",
        json!({"sketch": "Strokes", "text": "IL", "font": "stroke", "size": 20}),
    );
    for format in ["dxf", "svg"] {
        let file = format!("stroke-IL.{format}");
        run(
            &mut s,
            "file.export",
            json!({"path": file, "sketch": "Strokes"}),
        );
        expected.insert(file, (0.0, 2));
    }
    let report = freecad_reads(&dir);
    for (file, (area, wires)) in &expected {
        let got = report
            .get(file)
            .unwrap_or_else(|| panic!("FreeCAD did not read {file}"));
        let got_wires: usize = got["wires"].parse().unwrap();
        assert_eq!(got_wires, *wires, "{file}: {got:?}");
        // Each Bézier piece is a SPLINE of its own in the DXF.
        if *area > 0.0 {
            let got_area: f64 = got["area"].parse().unwrap();
            assert!(
                near(got_area, *area, 1e-3),
                "{file}: area {got_area}, ours {area}"
            );
        }
    }
}

/// The cut parts of `cut.export` with a kerf (docs/design/cutting.md):
/// a 100 × 50 plate with a round hole and a slot and a 60 × 40 plate,
/// side by side, 0.2 off the parts. FreeCAD reads the DXF and the SVG
/// with our length, the sheet's box (the kerf around both parts), and
/// four closed wires (two outlines, two holes).
#[test]
#[ignore = "needs FreeCAD: set CRAFT3D_FREECAD and run with --ignored"]
fn freecad_reads_cut_parts_with_a_kerf() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("drawing-freecad-cut");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "sketch.create", json!({"plane": "xy", "name": "P"}));
    run(
        &mut s,
        "sketch.rect",
        json!({"sketch": "P", "from": [0, 0], "to": [100, 50]}),
    );
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "P", "center": [20, 25], "radius": 5}),
    );
    run(
        &mut s,
        "sketch.path",
        json!({"sketch": "P", "start": [50, 21], "segments": [
            {"line": [80, 21]}, {"arc": [80, 29], "tangent": true},
            {"line": [50, 29], "tangent": true}, {"arc": "start", "tangent": true}]}),
    );
    run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "P", "distance": 6, "name": "Holed"}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"size": [60, 40, 6], "position": [0, 200, 0], "name": "Small"}),
    );
    let k = 0.2;
    let length = (300.0 + 2.0 * PI * k)
        + 2.0 * PI * (5.0 - k)
        + (60.0 + 2.0 * PI * (4.0 - k))
        + (200.0 + 2.0 * PI * k);
    let mut cases = Vec::new();
    for format in ["dxf", "svg"] {
        let file = format!("cut.{format}");
        let result = run(
            &mut s,
            "cut.export",
            json!({"path": file, "kerf": format!("{k} mm"), "gap": "5 mm"}),
        );
        assert!(
            near(result["length"].as_f64().unwrap(), length, 1e-12),
            "{result}"
        );
        cases.push((file, result));
    }
    let reports = freecad_reads(&dir);
    assert_eq!(reports.len(), cases.len());
    // Both parts placed by their kerfed boxes from (0, 0), 5 apart.
    let want = [
        0.0,
        0.0,
        100.0 + 2.0 * k + 5.0 + 60.0 + 2.0 * k,
        50.0 + 2.0 * k,
    ];
    for (file, ours) in &cases {
        let report = &reports[file];
        let number = |key: &str| -> f64 { report[key].parse().unwrap() };
        println!(
            "{file}: length {} (ours {}), bbox {}, wires {}, {} edges ({})",
            report["length"],
            ours["length"],
            report["bbox"],
            report.get("closed").map_or("?", String::as_str),
            report["edges"],
            report["kinds"]
        );
        assert!(near(number("length"), length, 1e-9), "{file}");
        let bbox: Vec<f64> = report["bbox"]
            .split(' ')
            .map(|v| v.parse().unwrap())
            .collect();
        for (got, expected) in bbox.iter().zip(want) {
            assert!((got - expected).abs() <= 1e-9 * 200.0, "{file}: {bbox:?}");
        }
        if let Some(closed) = report.get("closed") {
            assert_eq!(closed.parse::<u64>().unwrap(), 4, "{file}");
        }
    }
}
