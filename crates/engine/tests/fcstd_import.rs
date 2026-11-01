//! FreeCAD documents through `file.import`: one feature per final
//! object's solid, named after its label, coloured, saved with the
//! document and regenerated from its bytes (docs/design/step-import.md ›
//! FreeCAD documents). The documents are FreeCAD's own output
//! (`crates/fcstdread/tests/data/make_fixtures.py`); their measures are
//! held to FreeCAD's in `craft3d-fcstdread`'s tests.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use common::{refused, session_in, workdir};
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::path::Path;

fn fixture(dir: &Path, name: &str) {
    let from = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../fcstdread/tests/data")
        .join(name);
    std::fs::copy(from, dir.join(name)).unwrap();
}

fn bodies(s: &mut Session) -> Vec<Value> {
    let inspected = run(s, "document.inspect", json!({}));
    inspected["bodies"].as_array().unwrap().clone()
}

#[test]
fn a_freecad_document_imports_its_final_objects_with_their_colours() {
    let dir = workdir("fcstd-import", "multi");
    fixture(&dir, "multi.FCStd");
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    let result = run(&mut s, "file.import", json!({"path": "multi.FCStd"}));
    let made = result["bodies"].as_array().unwrap();
    let names: Vec<&str> = made.iter().map(|b| b["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        ["Red block", "Green", "Ball", "Cone", "Ring"],
        "{result}"
    );
    for body in made {
        assert_eq!(body["status"], "ok", "{body}");
        assert_eq!(body["import"]["fileUnit"], "millimetre");
    }
    assert_eq!(made[0]["import"]["color"], "#ff0000");
    assert_eq!(made[1]["import"]["color"], "#00ff00");
    assert!(result["skipped"].to_string().contains("Ghost"), "{result}");
    let all = bodies(&mut s);
    assert_eq!(all.len(), 5);
    assert!(all[0].to_string().contains("#ff0000"), "{}", all[0]);
    // Exact surfaces: the ball's volume is a sphere's.
    let ball = all[2]["volume"].as_f64().unwrap();
    let exact = 4.0 / 3.0 * std::f64::consts::PI * 64.0;
    assert!((ball - exact).abs() < 1e-9 * exact, "{ball}");

    // One undo step; saved and reopened without the file: the same.
    run(&mut s, "edit.undo", json!({}));
    assert!(bodies(&mut s).is_empty());
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(bodies(&mut s), all);
    run(&mut s, "file.save", json!({"path": "multi.3dcraft"}));
    std::fs::remove_file(dir.join("multi.FCStd")).unwrap();
    let mut t = session_in(&dir);
    run(&mut t, "file.open", json!({"path": "multi.3dcraft"}));
    assert_eq!(bodies(&mut t), all);
}

#[test]
fn a_partdesign_body_and_a_cut_import_as_one_body_each() {
    let dir = workdir("fcstd-import", "body");
    for name in ["body.FCStd", "cut.FCStd", "fillet.FCStd", "spline.FCStd"] {
        fixture(&dir, name);
    }
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    for (file, labels) in [
        ("body.FCStd", vec!["Padded body"]),
        ("cut.FCStd", vec!["Plate with hole"]),
        ("fillet.FCStd", vec!["Rounded block"]),
        ("spline.FCStd", vec!["Vase", "Blob"]),
    ] {
        let result = run(&mut s, "file.import", json!({"path": file}));
        let made = result["bodies"].as_array().unwrap();
        let names: Vec<&str> = made.iter().map(|b| b["name"].as_str().unwrap()).collect();
        assert_eq!(names, labels, "{result}");
        assert!(made.iter().all(|b| b["status"] == "ok"), "{result}");
    }
    // A boolean on an imported body works as on any other.
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": 2, "height": 40, "position": [5, 5, -5], "name": "Drill"}),
    );
    let cut = run(
        &mut s,
        "feature.boolean",
        json!({"target": "Plate with hole", "tool": "Drill", "op": "subtract"}),
    );
    assert_eq!(cut["status"], "ok", "{cut}");
}

#[test]
fn a_damaged_document_is_refused_and_changes_nothing() {
    let dir = workdir("fcstd-import", "damaged");
    let bytes = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../fcstdread/tests/data/box.FCStd"),
    )
    .unwrap();
    std::fs::write(dir.join("cut.FCStd"), &bytes[..bytes.len() / 2]).unwrap();
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    let error = refused(&mut s, "file.import", json!({"path": "cut.FCStd"}));
    assert!(error.contains("cut.FCStd"), "{error}");
    assert!(bodies(&mut s).is_empty());
}
