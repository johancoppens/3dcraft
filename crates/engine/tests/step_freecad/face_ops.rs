//! A fifth set of documents for oracle 8 (docs/testing.md): the face
//! operations (M9, 2026-10-14). Offset faces (a block's top, a pocket's
//! floor, a hole's wall, a boss's wall, a hexagonal frustum's top) and
//! shells (an open box, a closed hollow block, a cup, a tube, an L open
//! at one end, a rounded tray with a bored boss on the floor of its
//! hollow); at the scales 1e-3, 1 and 1e6.
//!
//! Per body: ours against a closed form written out here, and FreeCAD's
//! reading of the STEP file against ours (valid, the solids, our volume,
//! area, centre of mass and counts). The table is printed and left in
//! `target/tmp/step-face-ops-files/report.md`.

// `!(x <= bound)` on purpose: a NaN is a problem too.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use super::harness;
use super::{EXACT, Expect, RING, Set};
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::f64::consts::PI;
use std::path::PathBuf;

type Forms = Result<Vec<(&'static str, f64)>, String>;

fn step(s: &mut Session, id: &str, params: Value) -> Result<Value, String> {
    let result = s
        .execute(id, &params)
        .map_err(|e| format!("{id} {params}: {e}"))?;
    if result["status"] == "failed" {
        return Err(format!(
            "{id} {params}: {}",
            result["error"].as_str().unwrap_or("failed")
        ));
    }
    Ok(result)
}

fn base(s: &mut Session, k: f64) -> Result<(), String> {
    step(
        s,
        "feature.block",
        json!({"name": "Base", "size": [10.0 * k, 10.0 * k, 5.0 * k]}),
    )?;
    Ok(())
}

fn with(s: &mut Session, k: f64, tool: Value, op: &str) -> Result<(), String> {
    base(s, k)?;
    step(
        s,
        &format!("feature.{}", tool["kind"].as_str().unwrap_or("block")),
        tool["params"].clone(),
    )?;
    step(
        s,
        "feature.boolean",
        json!({"op": op, "target": "Base", "tool": "Tool", "name": "Part"}),
    )?;
    Ok(())
}

fn top(s: &mut Session, k: f64) -> Forms {
    base(s, k)?;
    step(
        s,
        "feature.offsetFace",
        json!({"faces": ["F1:face(1)"], "distance": 1.5 * k, "name": "Moved"}),
    )?;
    Ok(vec![("Moved", 100.0 * 6.5)])
}

fn pocket(s: &mut Session, k: f64) -> Forms {
    let cutter = json!({"kind": "block", "params": {"name": "Tool",
        "position": [3.0 * k, 3.0 * k, 2.0 * k], "size": [4.0 * k, 4.0 * k, 4.0 * k]}});
    with(s, k, cutter, "subtract")?;
    step(
        s,
        "feature.offsetFace",
        json!({"faces": ["F3:copy(1)/F2:face(0)"], "distance": -k, "name": "Moved"}),
    )?;
    Ok(vec![("Moved", 500.0 - 16.0 * 4.0)])
}

fn hole(s: &mut Session, k: f64) -> Forms {
    let drill = json!({"kind": "cylinder", "params": {"name": "Tool",
        "position": [5.0 * k, 5.0 * k, -k], "radius": 2.0 * k, "height": 7.0 * k}});
    with(s, k, drill, "subtract")?;
    step(
        s,
        "feature.offsetFace",
        json!({"faces": ["F3:copy(1)/F2:face(0)"], "distance": -k, "name": "Moved"}),
    )?;
    Ok(vec![("Moved", (100.0 - 9.0 * PI) * 5.0)])
}

fn boss(s: &mut Session, k: f64) -> Forms {
    let stud = json!({"kind": "cylinder", "params": {"name": "Tool",
        "position": [5.0 * k, 5.0 * k, 5.0 * k], "radius": 2.0 * k, "height": 3.0 * k}});
    with(s, k, stud, "union")?;
    step(
        s,
        "feature.offsetFace",
        json!({"faces": ["F3:copy(1)/F2:face(0)", "F3:copy(0)/F1:face(1)"], "distance": 0.5 * k, "name": "Moved"}),
    )?;
    // The block 0.5 taller, the boss 0.5 thicker and 0.5 shorter.
    Ok(vec![("Moved", 100.0 * 5.5 + PI * 2.5 * 2.5 * 2.5)])
}

fn frustum(s: &mut Session, k: f64) -> Forms {
    let hexagon = |r: f64| -> Vec<[f64; 2]> {
        (0..6)
            .map(|i| {
                let a = f64::from(i) * PI / 3.0;
                [r * k * a.cos(), r * k * a.sin()]
            })
            .collect()
    };
    for (name, z, r) in [("Low", 0.0, 3.0), ("High", 2.0, 2.0)] {
        step(
            s,
            "sketch.create",
            json!({"plane": "xy", "offset": z * k, "name": name}),
        )?;
        step(
            s,
            "sketch.polyline",
            json!({"sketch": name, "closed": true, "points": hexagon(r)}),
        )?;
    }
    step(
        s,
        "feature.loft",
        json!({"sections": ["Low", "High"], "ruled": true, "name": "Frustum"}),
    )?;
    let names = step(s, "document.inspect", json!({}))?;
    let cap = names["bodies"][0]["faceNames"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .find(|n| n.contains("endCap"))
        .ok_or("the frustum has no end cap")?
        .to_string();
    step(
        s,
        "feature.offsetFace",
        json!({"faces": [cap], "distance": 2.0 * k, "name": "Moved"}),
    )?;
    let area = |r: f64| 1.5 * 3.0_f64.sqrt() * r * r;
    let (a, b, h) = (area(3.0), area(1.0), 4.0);
    Ok(vec![("Moved", h / 3.0 * (a + b + (a * b).sqrt()))])
}

fn open_box(s: &mut Session, k: f64) -> Forms {
    base(s, k)?;
    step(
        s,
        "feature.shell",
        json!({"source": "Base", "thickness": k, "open": ["F1:face(1)"], "name": "Box"}),
    )?;
    Ok(vec![("Box", 500.0 - 64.0 * 4.0)])
}

fn hollow_block(s: &mut Session, k: f64) -> Forms {
    base(s, k)?;
    step(
        s,
        "feature.shell",
        json!({"source": "Base", "thickness": k, "name": "Hollow"}),
    )?;
    Ok(vec![("Hollow", 500.0 - 64.0 * 3.0)])
}

fn cup(s: &mut Session, k: f64, open: &[&str]) -> Result<f64, String> {
    step(
        s,
        "feature.cylinder",
        json!({"name": "Rod", "radius": 3.0 * k, "height": 5.0 * k}),
    )?;
    step(
        s,
        "feature.shell",
        json!({"source": "Rod", "thickness": 0.5 * k, "open": open, "name": "Cup"}),
    )?;
    Ok(PI * 9.0 * 5.0)
}

fn one_end(s: &mut Session, k: f64) -> Forms {
    let full = cup(s, k, &["F1:face(2)"])?;
    Ok(vec![("Cup", full - PI * 6.25 * 4.5)])
}

fn tube(s: &mut Session, k: f64) -> Forms {
    let full = cup(s, k, &["F1:face(2)", "F1:face(1)"])?;
    Ok(vec![("Cup", full - PI * 6.25 * 5.0)])
}

fn ell(s: &mut Session, k: f64) -> Forms {
    step(s, "sketch.create", json!({"name": "Outline"}))?;
    let points: Vec<[f64; 2]> = [
        [0.0, 0.0],
        [10.0, 0.0],
        [10.0, 4.0],
        [4.0, 4.0],
        [4.0, 10.0],
        [0.0, 10.0],
    ]
    .iter()
    .map(|[x, y]| [x * k, y * k])
    .collect();
    step(
        s,
        "sketch.polyline",
        json!({"sketch": "Outline", "closed": true, "points": points}),
    )?;
    step(
        s,
        "feature.extrude",
        json!({"sketch": "Outline", "distance": 6.0 * k, "name": "Ell"}),
    )?;
    let names = step(s, "document.inspect", json!({}))?;
    let cap = names["bodies"][0]["faceNames"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .find(|n| n.contains("endCap"))
        .ok_or("the L has no end cap")?
        .to_string();
    step(
        s,
        "feature.shell",
        json!({"source": "Ell", "thickness": k, "open": [cap], "name": "Channel"}),
    )?;
    Ok(vec![("Channel", 64.0 * 6.0 - 28.0 * 5.0)])
}

/// The showcase's tray (`docs/showcase/tray.steps.json`), at scale `k`.
fn tray(s: &mut Session, k: f64) -> Forms {
    step(
        s,
        "feature.block",
        json!({"name": "Block", "size": [80.0 * k, 60.0 * k, 40.0 * k]}),
    )?;
    step(
        s,
        "feature.fillet",
        json!({"name": "Rounded", "radius": 10.0 * k, "edges": [
            ["F1:face(2)", "F1:face(4)"], ["F1:face(2)", "F1:face(5)"],
            ["F1:face(3)", "F1:face(4)"], ["F1:face(3)", "F1:face(5)"]]}),
    )?;
    step(
        s,
        "feature.shell",
        json!({"name": "Tray", "source": "Rounded", "thickness": 3.0 * k,
               "open": ["F2:copy(0)/F1:face(1)"]}),
    )?;
    step(
        s,
        "sketch.create",
        json!({"name": "Spot", "face": "F3:copy(1)/F2:copy(0)/F1:face(0)"}),
    )?;
    for radius in [9.0, 4.0] {
        step(
            s,
            "sketch.circle",
            json!({"sketch": "Spot", "center": [30.0 * k, -40.0 * k], "radius": radius * k}),
        )?;
    }
    step(
        s,
        "feature.extrude",
        json!({"sketch": "Spot", "distance": 25.0 * k, "join": "Tray", "name": "Boss"}),
    )?;
    step(
        s,
        "feature.offsetFace",
        json!({"name": "Taller", "faces": ["F6:copy(1)/F5:endCap(e2)"], "distance": 5.0 * k}),
    )?;
    step(
        s,
        "feature.offsetFace",
        json!({"name": "Bored", "faces": ["F7:copy(0)/F6:copy(1)/F5:side(e4)"], "distance": -k}),
    )?;
    let outer = 80.0 * 60.0 - (4.0 - PI) * 100.0;
    let inner = 74.0 * 54.0 - (4.0 - PI) * 49.0;
    Ok(vec![(
        "Bored",
        outer * 40.0 - inner * 37.0 + PI * (81.0 - 25.0) * 30.0,
    )])
}

#[test]
#[ignore = "needs FreeCAD: set CRAFT3D_FREECAD"]
fn freecad_reads_bodies_of_the_face_operations() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("step-face-ops");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut set = Set {
        dir,
        cases: Vec::new(),
        refused: Vec::new(),
        unmeasured: Vec::new(),
        per_body: true,
        meshes: None,
    };
    type Build = fn(&mut Session, f64) -> Forms;
    let cases: [(&str, Expect, Build); 11] = [
        ("offset-top", EXACT, top),
        ("offset-pocket", EXACT, pocket),
        ("offset-hole", RING, hole),
        ("offset-boss", RING, boss),
        ("offset-frustum", EXACT, frustum),
        ("shell-box", EXACT, open_box),
        ("shell-hollow", EXACT, hollow_block),
        ("shell-cup", RING, one_end),
        ("shell-tube", RING, tube),
        ("shell-ell", EXACT, ell),
        ("shell-tray", RING, tray),
    ];
    let mut bad = Vec::new();
    for (k, tag) in [(1e-3, "milli"), (1.0, "one"), (1e6, "mega")] {
        for (name, expect, build) in &cases {
            let file = format!("{tag}-{name}");
            let mut forms = Vec::new();
            let before = set.cases.len();
            set.add(&file, "mm", k, *expect, |s, k| {
                forms = build(s, k)?;
                Ok(())
            });
            if set.cases.len() == before {
                continue;
            }
            let case = &set.cases[before];
            for (body, volume) in forms {
                let Some(part) = case.parts.iter().find(|p| p.name == body) else {
                    bad.push(format!("{file}: no body {body}"));
                    continue;
                };
                let off = part.volume / (volume * k.powi(3)) - 1.0;
                if !(off.abs() <= 1e-9) {
                    bad.push(format!("{file}: {body} is {off:+e} off its closed form"));
                }
            }
        }
    }
    assert!(set.refused.is_empty(), "refused: {:#?}", set.refused);
    assert!(set.unmeasured.is_empty(), "{:#?}", set.unmeasured);
    assert_eq!(set.cases.len(), 33);
    let rows = harness::check(
        "step-face-ops-files",
        "STEP: bodies of the face operations, read by FreeCAD",
        &set.cases,
    );
    for row in rows.iter().filter(|r| !r.problems.is_empty()) {
        bad.push(format!("{}: {}", row.file, row.problems.join("; ")));
    }
    assert!(bad.is_empty(), "{bad:#?}");
}
