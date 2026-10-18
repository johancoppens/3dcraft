//! Sheets through the commands (the boolean checklist's item 5): a fill
//! makes a sheet from a sketch's profiles, measured (area, no volume),
//! checked deep, exported to STEP as an open shell, saved and reopened;
//! a solid is cut by it both ways (the volumes add up to the solid's, the
//! cut face named after the sheet's face), and the sheet is trimmed by a
//! solid both ways (the areas add up to the sheet's). What a sheet cannot
//! do fails the feature with the reason; bad parameters are errors. At
//! three scales.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{
    assert_close, body, built, deep, error, failed, failure, new_document, run, session_in, workdir,
};
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::f64::consts::PI;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn mm(x: f64) -> String {
    format!("{x} mm")
}

/// A sketch on the plane z = `z` holding a square of side 20 about the
/// z axis with a round hole of radius `hole` (none for 0), and its fill.
fn sheet(s: &mut Session, name: &str, z: f64, hole: f64, k: f64) {
    let sketch = format!("{name} sketch");
    built(
        s,
        "sketch.create",
        json!({"plane": "xy", "offset": mm(z * k), "name": sketch}),
    );
    let c = 10.0 * k;
    run(
        s,
        "sketch.polyline",
        json!({"sketch": sketch, "points": [[-c, -c], [c, -c], [c, c], [-c, c]], "closed": true}),
    );
    if hole > 0.0 {
        run(
            s,
            "sketch.circle",
            json!({"sketch": sketch, "center": [0.0, 0.0], "radius": hole * k}),
        );
    }
    built(s, "feature.fill", json!({"sketch": sketch, "name": name}));
}

fn block(s: &mut Session, name: &str, k: f64) {
    // 4 × 3 × 2 from (−2, −1, 0).
    built(
        s,
        "feature.block",
        json!({"size": [4.0 * k, 3.0 * k, 2.0 * k], "position": [-2.0 * k, -k, 0.0], "name": name}),
    );
}

fn boolean(s: &mut Session, op: &str, target: &str, tool: &str, name: &str) -> Value {
    run(
        s,
        "feature.boolean",
        json!({"op": op, "target": target, "tool": tool, "keepTool": true, "name": name}),
    )
}

fn number(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

#[test]
fn a_fill_is_a_sheet_that_cuts_and_is_trimmed() {
    for k in SCALES {
        let dir = workdir("sheets", &format!("fill-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        sheet(&mut s, "Sheet", 1.5, 1.0, k);
        let found = body(&mut s, "Sheet");
        assert_eq!(found["kind"], "sheet", "{found}");
        assert_eq!(found["valid"], true);
        assert_close(&found["area"], (400.0 - PI) * k * k, "area");
        assert_eq!(number(&found["volume"]), 0.0);
        let names: Vec<&str> = found["faceNames"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_str().unwrap())
            .collect();
        assert_eq!(names.len(), 1);
        assert!(names[0].starts_with("F2:startCap("), "{names:?}");
        deep(&mut s);

        // A block cut by the sheet (the hole is above the block's middle,
        // over x, y in [−1, 1]: the sheet does not reach across it there).
        block(&mut s, "Block", k);
        let refused = boolean(&mut s, "subtract", "Block", "Sheet", "Holed");
        assert_eq!(refused["status"], "failed", "{refused}");
        assert!(
            refused["error"].as_str().unwrap().contains("ends inside"),
            "{refused}"
        );
        run(&mut s, "feature.delete", json!({"feature": "Holed"}));

        // A whole sheet: the block is cut, both ways.
        sheet(&mut s, "Wall", 1.5, 0.0, k);
        block(&mut s, "Other", k);
        built(
            &mut s,
            "feature.boolean",
            json!({"op": "subtract", "target": "Block", "tool": "Wall", "keepTool": true, "name": "Top"}),
        );
        built(
            &mut s,
            "feature.boolean",
            json!({"op": "intersect", "target": "Other", "tool": "Wall", "keepTool": true, "name": "Bottom"}),
        );
        let top = body(&mut s, "Top");
        let bottom = body(&mut s, "Bottom");
        assert_eq!(top["kind"], "solid");
        assert_close(&top["volume"], 6.0 * k * k * k, "volume in front");
        assert_close(&bottom["volume"], 18.0 * k * k * k, "volume behind");
        assert_close(&top["area"], 31.0 * k * k, "area in front");
        // The cut face is named after the sheet's face.
        let cut_faces: Vec<&str> = top["faceNames"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_str().unwrap())
            .filter(|n| n.contains("startCap"))
            .collect();
        assert_eq!(cut_faces.len(), 1, "{top}");
        assert!(cut_faces[0].contains("copy(1)/"), "{cut_faces:?}");
        deep(&mut s);

        // The sheet trimmed by a block both ways: the section is 12.
        block(&mut s, "Cutter", k);
        built(
            &mut s,
            "feature.boolean",
            json!({"op": "intersect", "target": "Wall", "tool": "Cutter", "keepTool": true, "name": "Inside"}),
        );
        let inside = body(&mut s, "Inside");
        assert_eq!(inside["kind"], "sheet");
        assert_close(&inside["area"], 12.0 * k * k, "area inside");
        // A union with a sheet is refused, with the reason.
        let union = boolean(&mut s, "union", "Cutter", "Inside", "Joined");
        assert_eq!(union["status"], "failed", "{union}");
        assert!(
            union["error"]
                .as_str()
                .unwrap()
                .contains("union of a sheet"),
            "{union}"
        );
        run(&mut s, "feature.delete", json!({"feature": "Joined"}));
        deep(&mut s);

        // STEP: the sheets are open shells.
        let out = run(&mut s, "file.export", json!({"path": "sheets.step"}));
        assert_eq!(out["solids"], 3, "{out}");
        let text = std::fs::read_to_string(dir.join("sheets.step")).unwrap();
        assert_eq!(text.matches("OPEN_SHELL(").count(), 2, "{out}");
        assert_eq!(text.matches("SHELL_BASED_SURFACE_MODEL(").count(), 2);

        // Saved and reopened: the same bodies under the same names.
        let before: Vec<Value> = ["Sheet", "Top", "Bottom", "Inside"]
            .iter()
            .map(|n| body(&mut s, n))
            .collect();
        run(
            &mut s,
            "file.save",
            json!({"path": "sheets.3dcraft", "overwrite": true}),
        );
        run(&mut s, "file.open", json!({"path": "sheets.3dcraft"}));
        for (name, before) in ["Sheet", "Top", "Bottom", "Inside"].iter().zip(before) {
            let after = body(&mut s, name);
            for key in [
                "kind",
                "faces",
                "edges",
                "vertices",
                "shells",
                "area",
                "volume",
                "faceNames",
            ] {
                assert_eq!(after[key], before[key], "{name}: {key} after a round trip");
            }
        }
    }
}

#[test]
fn a_fill_that_cannot_be_made_says_why() {
    let dir = workdir("sheets", "refusals");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    built(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "name": "Square"}),
    );
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Square", "points": [[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]], "closed": true}),
    );
    built(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "name": "Open"}),
    );
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Open", "points": [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]}),
    );
    let reason = failed(
        &mut s,
        "feature.fill",
        json!({"sketch": "Open", "name": "F"}),
    );
    assert!(reason.contains("not closed"), "{reason}");
    assert!(failure(&mut s, "F").contains("not closed"));
    // Not a sketch, no sketch, bad parameters: errors, nothing added.
    block(&mut s, "Block", 1.0);
    for params in [
        json!({"sketch": "Block"}),
        json!({"sketch": "Nothing"}),
        json!({}),
        json!({"sketch": 12}),
        json!({"sketch": "Open", "distance": 3}),
        json!({"sketch": ["Open"]}),
        json!({"sketch": "Open", "name": 7}),
        json!(null),
    ] {
        let e = error(&mut s, "feature.fill", params.clone());
        assert!(!e.is_empty(), "{params}");
    }
    // Shell and thicken of a sheet are not built yet, and say so.
    built(
        &mut s,
        "feature.fill",
        json!({"sketch": "Square", "name": "Plate"}),
    );
    let face = body(&mut s, "Plate")["faceNames"][0]
        .as_str()
        .unwrap()
        .to_string();
    let shelled = failed(
        &mut s,
        "feature.shell",
        json!({"source": "Plate", "thickness": "0.1 mm", "name": "Hollow"}),
    );
    assert!(
        shelled.contains("sheet") && shelled.contains("not built yet"),
        "{shelled}"
    );
    run(&mut s, "feature.delete", json!({"feature": "Hollow"}));
    let thickened = failed(
        &mut s,
        "feature.thicken",
        json!({"faces": [face], "thickness": "0.1 mm", "name": "Layer"}),
    );
    assert!(
        thickened.contains("sheet") && thickened.contains("not built yet"),
        "{thickened}"
    );
    run(&mut s, "feature.delete", json!({"feature": "Layer"}));
    // An edit to another sketch: the fill follows it.
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "F", "sketch": "Square"}),
    );
    assert_close(&body(&mut s, "F")["area"], 4.0, "area after the edit");
}
