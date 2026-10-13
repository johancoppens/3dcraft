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

use craft3d_engine::{Access, Session, Workspace};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::f64::consts::PI;
use std::path::PathBuf;
use std::process::Command;

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

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
    let lean = [0.5, 0.0, (PI / 6.0).cos()];
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
    let ellipse_area = PI * 5.0 * 5.0 / (PI / 6.0).cos();
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
        assert!(near(number("area"), area, 1e-9), "{file}");
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
            } else {
                "Circle:8,Line:10".to_string()
            };
            assert_eq!(*kinds, expected, "{file}");
        }
    }
}
