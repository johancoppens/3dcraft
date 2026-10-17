//! A fourth set of documents for oracle 8 (docs/testing.md): filleted
//! bodies (M9, 2026-10-14). Edges of a block (one, three with the eighth
//! of a sphere in their corner, all twelve with their eight corners), a
//! hexagonal prism with every edge rounded (corners that are not
//! square), an L with an inside and an outside fillet, both rims of a
//! rod, a plate with the mouths of two holes, the floor of a blind one
//! and the foot of a boss, and the rim of a slot picked as a chain; at
//! the scales 1e-3, 1 and 1e6.
//!
//! Per body: ours against a closed form written out here, and FreeCAD's
//! reading of the STEP file against ours (valid, one solid, our volume,
//! area, centre of mass and counts). The table is printed and left in
//! `target/tmp/step-fillets-files/report.md`.

// `!(x <= bound)` on purpose: a NaN is a problem too.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use super::harness;
use super::{EXACT, Expect, RING, Set};
use super::{edge, step};
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::f64::consts::PI;
use std::path::PathBuf;

type Forms = Result<Vec<(&'static str, f64)>, String>;

/// What a square corner loses to a round of radius 1.
const SPANDREL: f64 = 1.0 - PI / 4.0;

/// What a square corner at distance `at` from the axis, with the round's
/// middle on the side `towards` (+1 away from the axis, -1 towards it),
/// loses to a round of radius `r` in a full turn (Pappus: the square less
/// the quarter disc, each at its own centroid).
fn ring(at: f64, towards: f64, r: f64) -> f64 {
    2.0 * PI
        * (r * r * (at + towards * r / 2.0) - PI / 4.0 * r * r * (at + towards * r)
            + towards * r * r * r / 3.0)
}

fn base(s: &mut Session, k: f64) -> Result<(), String> {
    step(
        s,
        "feature.block",
        json!({"name": "Base", "size": [4.0 * k, 3.0 * k, 2.0 * k]}),
    )?;
    Ok(())
}

fn block_edge(s: &mut Session, k: f64) -> Forms {
    base(s, k)?;
    let r = 0.5;
    step(
        s,
        "feature.fillet",
        json!({"edges": [["F1:face(1)", "F1:face(2)"]], "radius": r * k, "name": "Round"}),
    )?;
    Ok(vec![("Round", 24.0 - SPANDREL * r * r * 4.0)])
}

fn block_three(s: &mut Session, k: f64) -> Forms {
    base(s, k)?;
    let r = 0.5;
    step(
        s,
        "feature.fillet",
        json!({"edges": [["F1:face(1)", "F1:face(2)"], ["F1:face(1)", "F1:face(4)"],
                         ["F1:face(2)", "F1:face(4)"]],
               "radius": r * k, "name": "Round"}),
    )?;
    // Three strips that stop the radius short of the corner, and the
    // corner's cube less an eighth of the ball.
    Ok(vec![(
        "Round",
        24.0 - SPANDREL * r * r * (9.0 - 3.0 * r) - (1.0 - PI / 6.0) * r * r * r,
    )])
}

fn block_all(s: &mut Session, k: f64) -> Forms {
    base(s, k)?;
    let r = 0.25;
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
        "feature.fillet",
        json!({"edges": edges, "radius": r * k, "name": "Round"}),
    )?;
    // The smaller block grown by a ball.
    let (a, b, c) = (4.0 - 2.0 * r, 3.0 - 2.0 * r, 2.0 - 2.0 * r);
    Ok(vec![(
        "Round",
        a * b * c
            + 2.0 * (a * b + a * c + b * c) * r
            + PI * r * r * (a + b + c)
            + 4.0 / 3.0 * PI * r * r * r,
    )])
}

fn hexagon_all(s: &mut Session, k: f64) -> Forms {
    step(s, "sketch.create", json!({"name": "Outline"}))?;
    let points: Vec<[f64; 2]> = (0..6)
        .map(|i| {
            let a = f64::from(i) * PI / 3.0;
            [2.0 * k * a.cos(), 2.0 * k * a.sin()]
        })
        .collect();
    step(
        s,
        "sketch.polyline",
        json!({"sketch": "Outline", "closed": true, "points": points}),
    )?;
    let h = 2.5;
    step(
        s,
        "feature.extrude",
        json!({"sketch": "Outline", "distance": h * k, "name": "Prism"}),
    )?;
    let listed = step(s, "document.edges", json!({"feature": "Prism"}))?;
    let edges: Vec<Value> = listed["edges"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|e| e["faces"].clone())
        .collect();
    if edges.len() != 18 {
        return Err(format!("a hexagonal prism with {} edges", edges.len()));
    }
    let r = 0.3;
    step(
        s,
        "feature.fillet",
        json!({"edges": edges, "radius": r * k, "name": "Round"}),
    )?;
    // The smaller prism grown by a ball (Steiner): its volume, its area
    // times r, the edges' lengths times their outer angles times r^2 / 2,
    // and the ball.
    let apothem = 3.0_f64.sqrt() - r;
    let side = 2.0 * apothem / 3.0_f64.sqrt();
    let tall = h - 2.0 * r;
    let floor = 2.0 * 3.0_f64.sqrt() * apothem * apothem;
    let turn = 0.5 * (12.0 * side * PI / 2.0 + 6.0 * tall * PI / 3.0);
    Ok(vec![(
        "Round",
        floor * tall
            + (2.0 * floor + 6.0 * side * tall) * r
            + turn * r * r
            + 4.0 / 3.0 * PI * r * r * r,
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
    let r = 0.4;
    step(
        s,
        "feature.fillet",
        json!({"edges": [inner, outer, end], "radius": r * k, "name": "Web"}),
    )?;
    // The inside corner gains a spandrel 2 long, the outside corner
    // loses one 2 long, the top edge of the end of the long arm loses one
    // 1.5 long.
    let section = SPANDREL * r * r;
    Ok(vec![(
        "Web",
        16.5 + section * 2.0 - section * 2.0 - section * 1.5,
    )])
}

fn rod(s: &mut Session, k: f64) -> Forms {
    let (rr, h) = (1.0, 3.0);
    step(
        s,
        "feature.cylinder",
        json!({"name": "Rod", "radius": rr * k, "height": h * k}),
    )?;
    let r = 0.25;
    step(
        s,
        "feature.fillet",
        json!({"edges": [["F1:face(2)", "F1:face(0)"], ["F1:face(1)", "F1:face(0)"]],
               "radius": r * k, "name": "Round"}),
    )?;
    Ok(vec![("Round", PI * rr * rr * h - 2.0 * ring(rr, -1.0, r))])
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
    let (rh, r) = (0.5, 0.2);
    let mut rims = Vec::new();
    for (x, z) in [(1.5, 2.0), (1.5, 0.0), (4.0, 2.0), (4.0, 1.0), (6.5, 2.0)] {
        rims.push(edge(s, "Part", k, [x + rh, 1.5, z], [x + rh, 1.5, z])?);
    }
    step(
        s,
        "feature.fillet",
        json!({"edges": rims, "radius": r * k, "name": "Round"}),
    )?;
    let plain = 48.0 - PI * rh * rh * 2.0;
    // Three mouths lose the ring outside the wall; the floor gains the
    // ring inside it, the foot the ring outside.
    let outside = ring(rh, 1.0, r);
    let inside = ring(rh, -1.0, r);
    Ok(vec![("Round", plain - 3.0 * outside + inside + outside)])
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
    let r = 0.25;
    step(
        s,
        "feature.fillet",
        json!({"edges": [top, bottom], "radius": r * k, "chain": true, "name": "Rims"}),
    )?;
    let removed = 2.0 * (2.0 * 3.0 * SPANDREL * r * r + ring(1.0, -1.0, r));
    Ok(vec![("Rims", (6.0 + PI) * 2.0 - removed)])
}

#[test]
#[ignore = "needs FreeCAD: set CRAFT3D_FREECAD"]
fn freecad_reads_filleted_bodies() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("step-fillets");
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
    let cases: [(&str, Expect, Build); 8] = [
        ("fillet-block-edge", EXACT, block_edge),
        ("fillet-block-three", EXACT, block_three),
        ("fillet-block-all", EXACT, block_all),
        ("fillet-hexagon-all", EXACT, hexagon_all),
        ("fillet-ell", EXACT, ell),
        ("fillet-rod", RING, rod),
        ("fillet-holes", RING, holes),
        ("fillet-slot", RING, slot),
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
    assert_eq!(set.cases.len(), 24);
    let rows = harness::check(
        "step-fillets-files",
        "STEP: filleted bodies, read by FreeCAD",
        &set.cases,
    );
    for row in rows.iter().filter(|r| !r.problems.is_empty()) {
        bad.push(format!("{}: {}", row.file, row.problems.join("; ")));
    }
    assert!(bad.is_empty(), "{bad:#?}");
}
