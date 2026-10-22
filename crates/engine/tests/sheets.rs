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

/// A fill in the plane of a face of the part: the piece on that face
/// counts as inside the part (decided 2026-10-18). A plate with a block on
/// it, and a fill in the plane of the plate's top: the part is cut into
/// the block (in front, subtract) and the plate (behind, intersect), the
/// volumes adding up; trimmed by the part the fill keeps the plate's top
/// with the block's footprint (12), and the rest for subtract (388). At
/// three scales, checked deep.
#[test]
fn a_fill_on_a_face_of_the_part_cuts_and_is_trimmed_there() {
    for k in SCALES {
        let dir = workdir("sheets", &format!("on-face-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        // Three copies of the part: a plate of 4 × 3 × 1 from (−2, −1, 0)
        // with a block of 1 × 1 × 1 on its top.
        for part in ["Part", "PartB", "PartC"] {
            let (plate, block) = (format!("{part} plate"), format!("{part} block"));
            built(
                &mut s,
                "feature.block",
                json!({"size": [4.0 * k, 3.0 * k, k], "position": [-2.0 * k, -k, 0.0], "name": plate}),
            );
            built(
                &mut s,
                "feature.block",
                json!({"size": [k, k, k], "position": [-0.5 * k, 0.0, k], "name": block}),
            );
            built(
                &mut s,
                "feature.boolean",
                json!({"op": "union", "target": plate, "tool": block, "name": part}),
            );
            assert_close(&body(&mut s, part)["volume"], 13.0 * k * k * k, "the part");
        }
        sheet(&mut s, "Wall", 1.0, 0.0, k);
        sheet(&mut s, "WallB", 1.0, 0.0, k);
        built(
            &mut s,
            "feature.boolean",
            json!({"op": "subtract", "target": "Part", "tool": "Wall", "keepTool": true, "name": "Above"}),
        );
        built(
            &mut s,
            "feature.boolean",
            json!({"op": "intersect", "target": "PartB", "tool": "Wall", "keepTool": true, "name": "Below"}),
        );
        let above = body(&mut s, "Above");
        let below = body(&mut s, "Below");
        assert_eq!(above["kind"], "solid");
        assert_eq!(below["kind"], "solid");
        assert_close(&above["volume"], k * k * k, "the block above the fill");
        assert_close(
            &below["volume"],
            12.0 * k * k * k,
            "the plate behind the fill",
        );
        assert_close(&above["area"], 6.0 * k * k, "the block's area");
        assert_close(&below["area"], 38.0 * k * k, "the plate's area");
        // The block's new bottom is the fill's face; the plate keeps its
        // own top, whole.
        let caps = |found: &Value| {
            found["faceNames"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|n| n.as_str().unwrap().contains("startCap"))
                .count()
        };
        assert_eq!(caps(&above), 1, "{above}");
        assert_eq!(caps(&below), 0, "{below}");
        assert_eq!(below["faces"], 6, "{below}");
        deep(&mut s);

        // Trimmed by the part: the top with the footprint, and the rest.
        built(
            &mut s,
            "feature.boolean",
            json!({"op": "intersect", "target": "Wall", "tool": "PartC", "keepTool": true, "name": "Inside"}),
        );
        built(
            &mut s,
            "feature.boolean",
            json!({"op": "subtract", "target": "WallB", "tool": "PartC", "keepTool": true, "name": "Outside"}),
        );
        let inside = body(&mut s, "Inside");
        let outside = body(&mut s, "Outside");
        assert_eq!(inside["kind"], "sheet");
        assert_eq!(outside["kind"], "sheet");
        assert_close(&inside["area"], 12.0 * k * k, "inside the part");
        assert_close(&outside["area"], 388.0 * k * k, "outside the part");
        assert_eq!(inside["faces"], 1, "{inside}");
        deep(&mut s);
    }
}

/// A fill of the rectangle from `lo` to `hi` (times `k`) in a sketch on
/// the world plane `plane` (xy: normal z; yz: sketch axes y and z, normal
/// x) moved by `offset` along its normal.
fn rectangle_sheet(
    s: &mut Session,
    name: &str,
    (plane, offset): (&str, f64),
    lo: [f64; 2],
    hi: [f64; 2],
    k: f64,
) {
    let sketch = format!("{name} sketch");
    built(
        s,
        "sketch.create",
        json!({"plane": plane, "offset": mm(offset * k), "name": sketch}),
    );
    let ([x0, y0], [x1, y1]) = (lo.map(|v| v * k), hi.map(|v| v * k));
    run(
        s,
        "sketch.polyline",
        json!({"sketch": sketch, "points": [[x0, y0], [x1, y0], [x1, y1], [x0, y1]], "closed": true}),
    );
    built(s, "feature.fill", json!({"sketch": sketch, "name": name}));
}

/// Two sheets (decided 2026-10-22): a plate of 20 × 20 cut by a wall at
/// x = 5 facing +x keeps 5 × 20 in front (subtract) and 15 × 20 behind
/// (intersect), sheets whose faces are named after the plate's; two
/// halves of a plate are sewn by a union into the whole plate, one face;
/// a union of sheets that cross and a wall that ends inside the plate
/// fail with the reason; saved and reopened the bodies are the same. At
/// three scales, checked deep.
#[test]
fn a_sheet_cuts_a_sheet_and_two_halves_are_sewn() {
    for k in SCALES {
        let dir = workdir("sheets", &format!("two-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        for name in ["Plate", "PlateB"] {
            rectangle_sheet(&mut s, name, ("xy", 0.0), [-10.0, -10.0], [10.0, 10.0], k);
        }
        rectangle_sheet(&mut s, "Wall", ("yz", 5.0), [-30.0, -30.0], [30.0, 30.0], k);
        built(
            &mut s,
            "feature.boolean",
            json!({"op": "subtract", "target": "Plate", "tool": "Wall", "keepTool": true, "name": "Front"}),
        );
        built(
            &mut s,
            "feature.boolean",
            json!({"op": "intersect", "target": "PlateB", "tool": "Wall", "keepTool": true, "name": "Behind"}),
        );
        let front = body(&mut s, "Front");
        let behind = body(&mut s, "Behind");
        for (found, area) in [(&front, 100.0), (&behind, 300.0)] {
            assert_eq!(found["kind"], "sheet", "{found}");
            assert_eq!(found["valid"], true, "{found}");
            assert_eq!(found["faces"], 1, "{found}");
            assert_eq!(found["edges"], 4, "{found}");
            assert_close(&found["area"], area * k * k, "a sheet cut by a sheet");
            assert_eq!(number(&found["volume"]), 0.0);
            let names: Vec<&str> = found["faceNames"]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n.as_str().unwrap())
                .collect();
            assert!(
                names[0].contains("copy(0)/") && names[0].contains("startCap"),
                "the plate's face: {names:?}"
            );
        }
        deep(&mut s);

        // Two halves sewn: the whole plate, one face of four edges.
        rectangle_sheet(&mut s, "Left", ("xy", 0.0), [-10.0, -10.0], [0.0, 10.0], k);
        rectangle_sheet(&mut s, "Right", ("xy", 0.0), [0.0, -10.0], [10.0, 10.0], k);
        built(
            &mut s,
            "feature.boolean",
            json!({"op": "union", "target": "Left", "tool": "Right", "name": "Whole"}),
        );
        let whole = body(&mut s, "Whole");
        assert_eq!(whole["kind"], "sheet", "{whole}");
        assert_close(&whole["area"], 400.0 * k * k, "two halves");
        assert_eq!(whole["faces"], 1, "{whole}");
        assert_eq!(whole["edges"], 4, "{whole}");
        assert_eq!(whole["shells"], 1, "{whole}");
        deep(&mut s);

        // A union of sheets that cross, a wall that ends inside the plate.
        let crossed = boolean(&mut s, "union", "Whole", "Wall", "Crossed");
        assert_eq!(crossed["status"], "failed", "{crossed}");
        assert!(
            crossed["error"].as_str().unwrap().contains("inside a face"),
            "{crossed}"
        );
        run(&mut s, "feature.delete", json!({"feature": "Crossed"}));
        rectangle_sheet(&mut s, "Short", ("yz", 5.0), [-30.0, -30.0], [0.0, 30.0], k);
        let short = boolean(&mut s, "subtract", "Whole", "Short", "Halfway");
        assert_eq!(short["status"], "failed", "{short}");
        assert!(
            short["error"].as_str().unwrap().contains("ends inside"),
            "{short}"
        );
        run(&mut s, "feature.delete", json!({"feature": "Halfway"}));
        deep(&mut s);

        // Saved and reopened: the same bodies under the same names.
        let names = ["Front", "Behind", "Whole"];
        let before: Vec<Value> = names.iter().map(|n| body(&mut s, n)).collect();
        run(
            &mut s,
            "file.save",
            json!({"path": "two.3dcraft", "overwrite": true}),
        );
        run(&mut s, "file.open", json!({"path": "two.3dcraft"}));
        for (name, before) in names.iter().zip(before) {
            let after = body(&mut s, name);
            for key in ["kind", "faces", "edges", "area", "faceNames"] {
                assert_eq!(after[key], before[key], "{name}: {key} after a round trip");
            }
        }
    }
}

/// The six faces of the cube [−1, 1]³ (times `k`) as fills of squares of
/// side 2 on datum planes through the faces' centres, facing out (or in),
/// sewn by unions one at a time: facing out the last union closes the
/// sheet into a solid, the cube (volume 8, area 24), checked deep and
/// cut by a block like any solid; facing in the last union fails with
/// the reason. At three scales.
#[test]
fn six_sheets_sewn_into_a_cube_are_a_solid() {
    let x = [1.0, 0.0, 0.0];
    let y = [0.0, 1.0, 0.0];
    let z = [0.0, 0.0, 1.0];
    let neg = |v: [f64; 3]| v.map(|c| -c);
    // (centre, u, v) with u × v the outward normal.
    let faces = [
        (z, x, y),
        (neg(z), y, x),
        (x, y, z),
        (neg(x), z, y),
        (y, z, x),
        (neg(y), x, z),
    ];
    for k in SCALES {
        for out in [true, false] {
            let dir = workdir("sheets", &format!("cube-{k}-{out}"));
            let mut s = session_in(&dir);
            new_document(&mut s, k);
            for (i, &(c, u, v)) in faces.iter().enumerate() {
                let (u, v) = if out { (u, v) } else { (v, u) };
                let point = |d: [f64; 3]| [0, 1, 2].map(|j| mm((c[j] + d[j]) * k));
                let plane = format!("Plane {i}");
                built(
                    &mut s,
                    "datum.plane",
                    json!({"through": [point([0.0; 3]), point(u), point(v)], "name": plane}),
                );
                let sketch = format!("Face {i} sketch");
                built(
                    &mut s,
                    "sketch.create",
                    json!({"plane": plane, "name": sketch}),
                );
                run(
                    &mut s,
                    "sketch.polyline",
                    json!({"sketch": sketch, "points": [[-k, -k], [k, -k], [k, k], [-k, k]], "closed": true}),
                );
                built(
                    &mut s,
                    "feature.fill",
                    json!({"sketch": sketch, "name": format!("Face {i}")}),
                );
            }
            let mut sewn = "Face 0".to_string();
            for i in 1..6 {
                let name = format!("Sewn {i}");
                let op = json!({"op": "union", "target": sewn, "tool": format!("Face {i}"), "name": name});
                if i < 5 || out {
                    built(&mut s, "feature.boolean", op);
                    sewn = name;
                } else {
                    let reason = failed(&mut s, "feature.boolean", op);
                    assert!(reason.contains("faces inward"), "{reason}");
                    run(&mut s, "feature.delete", json!({"feature": name}));
                }
            }
            let found = body(&mut s, &sewn);
            if out {
                assert_eq!(found["kind"], "solid", "{found}");
                assert_eq!(found["valid"], true, "{found}");
                assert_close(&found["volume"], 8.0 * k * k * k, "the sewn cube");
                assert_close(&found["area"], 24.0 * k * k, "the sewn cube");
                assert_eq!(found["faces"], 6, "{found}");
                // Cut by a block like any solid: an eighth taken away.
                built(
                    &mut s,
                    "feature.block",
                    json!({"size": [2.0 * k, 2.0 * k, 2.0 * k], "name": "Corner"}),
                );
                built(
                    &mut s,
                    "feature.boolean",
                    json!({"op": "subtract", "target": sewn, "tool": "Corner", "name": "Less"}),
                );
                assert_close(
                    &body(&mut s, "Less")["volume"],
                    7.0 * k * k * k,
                    "less a corner",
                );
            } else {
                assert_eq!(found["kind"], "sheet", "{found}");
            }
            deep(&mut s);
        }
    }
}
