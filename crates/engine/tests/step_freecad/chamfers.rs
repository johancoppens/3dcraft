//! A third set of documents for oracle 8 (docs/testing.md): chamfered
//! bodies (M9, 2026-10-14). Edges of a block (one with two distances, all
//! twelve with their eight corners), an L with an inside and an outside
//! corner, both rims of a rod, a plate with the mouths of two holes, the
//! floor of a blind one and the foot and crown of a boss, and the rim of
//! a slot picked as a chain; at the scales 1e-3, 1 and 1e6.
//!
//! Per body: ours against a closed form written out here, and FreeCAD's
//! reading of the STEP file against ours (valid, one solid, our volume,
//! area, centre of mass and counts). The table is printed and left in
//! `target/tmp/step-chamfers-files/report.md`.

// `!(x <= bound)` on purpose: a NaN is a problem too.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use super::harness;
use super::{EXACT, Expect, FLAT, Set};
use super::{edge, step};
use craft3d_engine::Session;
use craft3d_math::Portable;
use serde_json::{Value, json};
use std::f64::consts::PI;
use std::path::PathBuf;

type Forms = Result<Vec<(&'static str, f64)>, String>;

/// The volume a triangle in the half-plane (r, z) sweeps in a full turn
/// about the axis (Pappus).
fn ring(corners: [[f64; 2]; 3]) -> f64 {
    let [a, b, c] = corners;
    let area = 0.5 * ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])).abs();
    2.0 * PI * (a[0] + b[0] + c[0]) / 3.0 * area
}

fn block_edge(s: &mut Session, k: f64) -> Forms {
    step(
        s,
        "feature.block",
        json!({"name": "Base", "size": [4.0 * k, 3.0 * k, 2.0 * k]}),
    )?;
    let (d1, d2) = (0.9, 0.3);
    step(
        s,
        "feature.chamfer",
        json!({"edges": [["F1:face(1)", "F1:face(2)"]], "distance": d1 * k, "second": d2 * k, "name": "Bevel"}),
    )?;
    Ok(vec![("Bevel", 24.0 - d1 * d2 / 2.0 * 4.0)])
}

fn block_all(s: &mut Session, k: f64) -> Forms {
    step(
        s,
        "feature.block",
        json!({"name": "Base", "size": [4.0 * k, 3.0 * k, 2.0 * k]}),
    )?;
    let d = 0.25;
    let pairs = [
        (1, 2),
        (1, 3),
        (1, 4),
        (1, 5),
        (0, 2),
        (0, 3),
        (0, 4),
        (0, 5),
        (2, 4),
        (2, 5),
        (3, 4),
        (3, 5),
    ];
    let edges: Vec<Value> = pairs
        .iter()
        .map(|(a, b)| json!([format!("F1:face({a})"), format!("F1:face({b})")]))
        .collect();
    step(
        s,
        "feature.chamfer",
        json!({"edges": edges, "distance": d * k, "name": "Bevel"}),
    )?;
    Ok(vec![(
        "Bevel",
        24.0 - 2.0 * d * d * 9.0 + 16.0 * d * d * d / 3.0,
    )])
}

fn ell(s: &mut Session, k: f64) -> Forms {
    step(s, "sketch.create", json!({"name": "Outline"}))?;
    let points: Vec<[f64; 2]> = [
        [0.0, 0.0],
        [4.0, 0.0],
        [4.0, 1.5],
        [1.5, 1.5],
        [1.5, 3.0],
        [0.0, 3.0],
    ]
    .iter()
    .map(|p: &[f64; 2]| [p[0] * k, p[1] * k])
    .collect();
    step(
        s,
        "sketch.polyline",
        json!({"sketch": "Outline", "closed": true, "points": points}),
    )?;
    step(
        s,
        "feature.extrude",
        json!({"sketch": "Outline", "distance": 2.0 * k, "name": "Ell"}),
    )?;
    let inner = edge(s, "Ell", k, [1.5, 1.5, 0.0], [1.5, 1.5, 2.0])?;
    let outer = edge(s, "Ell", k, [0.0, 0.0, 0.0], [0.0, 0.0, 2.0])?;
    let end = edge(s, "Ell", k, [4.0, 0.0, 2.0], [4.0, 1.5, 2.0])?;
    let (d, angle) = (0.4_f64, 35.0_f64);
    step(
        s,
        "feature.chamfer",
        json!({"edges": [inner, outer, end], "distance": d * k, "angle": format!("{angle} deg"), "name": "Web"}),
    )?;
    // Each a triangle with sides d and d tan(angle) at a right angle: the
    // inside corner gains one 2 long, the outside corner loses one 2
    // long, the top edge of the end of the long arm loses one 1.5 long.
    let section = d * d * angle.to_radians().ptan() / 2.0;
    Ok(vec![(
        "Web",
        16.5 + section * 2.0 - section * 2.0 - section * 1.5,
    )])
}

fn rod(s: &mut Session, k: f64) -> Forms {
    let (r, h) = (1.0, 3.0);
    step(
        s,
        "feature.cylinder",
        json!({"name": "Rod", "radius": r * k, "height": h * k}),
    )?;
    let (d1, d2) = (0.4, 0.25);
    step(
        s,
        "feature.chamfer",
        json!({"edges": [["F1:face(2)", "F1:face(0)"], ["F1:face(1)", "F1:face(0)"]],
               "distance": d1 * k, "second": d2 * k, "name": "Bevel"}),
    )?;
    let removed = 2.0 * ring([[r, h], [r - d1, h], [r, h - d2]]);
    Ok(vec![("Bevel", PI * r * r * h - removed)])
}

fn holes(s: &mut Session, k: f64) -> Forms {
    step(
        s,
        "feature.block",
        json!({"name": "Plate", "size": [8.0 * k, 3.0 * k, 2.0 * k]}),
    )?;
    let mut round = |name: &str, x: f64, z: f64, height: f64| {
        step(
            s,
            "feature.cylinder",
            json!({"name": name, "position": [x * k, 1.5 * k, z * k], "radius": 0.5 * k, "height": height * k}),
        )
    };
    round("Drill", 1.5, -1.0, 4.0)?;
    round("Bit", 4.0, 1.0, 2.0)?;
    round("Stud", 6.5, 2.0, 1.0)?;
    for (op, target, tool, name) in [
        ("subtract", "Plate", "Drill", "Through"),
        ("subtract", "Through", "Bit", "Blind"),
        ("union", "Blind", "Stud", "Part"),
    ] {
        step(
            s,
            "feature.boolean",
            json!({"op": op, "target": target, "tool": tool, "name": name}),
        )?;
    }
    let (r, d) = (0.5, 0.2);
    let mut rims = Vec::new();
    for (x, z) in [
        (1.5, 2.0),
        (1.5, 0.0),
        (4.0, 2.0),
        (4.0, 1.0),
        (6.5, 2.0),
        (6.5, 3.0),
    ] {
        rims.push(edge(s, "Part", k, [x + r, 1.5, z], [x + r, 1.5, z])?);
    }
    step(
        s,
        "feature.chamfer",
        json!({"edges": rims, "distance": d * k, "name": "Bevel"}),
    )?;
    let plain = 48.0 - PI * r * r * 2.0;
    // Three mouths lose the ring outside the wall, the crown the ring
    // inside it; the floor gains the ring inside, the foot the ring
    // outside.
    let outside = ring([[r, 0.0], [r + d, 0.0], [r, d]]);
    let inside = ring([[r, 0.0], [r - d, 0.0], [r, d]]);
    Ok(vec![(
        "Bevel",
        plain - 3.0 * outside - inside + inside + outside,
    )])
}

fn slot(s: &mut Session, k: f64) -> Forms {
    step(s, "sketch.create", json!({"name": "Outline"}))?;
    step(
        s,
        "sketch.path",
        json!({"sketch": "Outline", "start": [0.0, -k], "segments": [
            {"line": [3.0 * k, -k]},
            {"arc": [3.0 * k, k], "center": [3.0 * k, 0.0]},
            {"line": [0.0, k]},
            {"arc": "start", "center": [0.0, 0.0]},
        ]}),
    )?;
    step(
        s,
        "feature.extrude",
        json!({"sketch": "Outline", "distance": 2.0 * k, "name": "Slot"}),
    )?;
    let top = edge(s, "Slot", k, [0.0, -1.0, 2.0], [3.0, -1.0, 2.0])?;
    let bottom = edge(s, "Slot", k, [0.0, -1.0, 0.0], [3.0, -1.0, 0.0])?;
    let d = 0.25;
    step(
        s,
        "feature.chamfer",
        json!({"edges": [top, bottom], "distance": d * k, "chain": true, "name": "Rims"}),
    )?;
    let removed = 2.0 * (2.0 * 3.0 * d * d / 2.0 + ring([[1.0, 0.0], [1.0 - d, 0.0], [1.0, d]]));
    Ok(vec![("Rims", (6.0 + PI) * 2.0 - removed)])
}

#[test]
#[ignore = "needs FreeCAD: set CRAFT3D_FREECAD"]
fn freecad_reads_chamfered_bodies() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("step-chamfers");
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
    let cases: [(&str, Expect, Build); 6] = [
        ("chamfer-block-edge", FLAT, block_edge),
        ("chamfer-block-all", FLAT, block_all),
        ("chamfer-ell", FLAT, ell),
        ("chamfer-rod", EXACT, rod),
        ("chamfer-holes", EXACT, holes),
        ("chamfer-slot", EXACT, slot),
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
                let off = part.volume / (volume * k.ppowi(3)) - 1.0;
                if !(off.abs() <= 1e-9) {
                    bad.push(format!("{file}: {body} is {off:+e} off its closed form"));
                }
            }
        }
    }
    assert!(set.refused.is_empty(), "refused: {:#?}", set.refused);
    assert!(set.unmeasured.is_empty(), "{:#?}", set.unmeasured);
    assert_eq!(set.cases.len(), 18);
    let rows = harness::check(
        "step-chamfers-files",
        "STEP: chamfered bodies, read by FreeCAD",
        &set.cases,
    );
    for row in rows.iter().filter(|r| !r.problems.is_empty()) {
        bad.push(format!("{}: {}", row.file, row.problems.join("; ")));
    }
    assert!(bad.is_empty(), "{bad:#?}");
}
