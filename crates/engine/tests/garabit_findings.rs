//! What the Garabit valley showcase found (2026-10-21), each from the
//! steps that made it (`garabit_findings/` beside this file, or below):
//!
//! 1. No command made a freeform sheet: a loft of open sections is one
//!    now, and a terrain sheet cuts a block (subtract keeps the side the
//!    sheet faces: the chain's direction × the loft's).
//! 2. A loft took at most 64 sections; now as many as its spline through
//!    them takes (256).
//! 3. Faces between half and twice the tolerance apart are refused, by
//!    design: the refusal now names the faces, the tolerance and what to
//!    do.
//! 4. Pier 2's diagonal: a crack between a girder and the leg's web at
//!    1.19°, cut short by the diagonal, was refused as zero thickness.
//! 5. A pit whose bottom is at the height of the lake beside it: the
//!    meeting of the pit's bottom and the terrain was asked in a box that
//!    reached 22 m past the pit, into the rippled lake.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};
use std::path::PathBuf;

fn run(session: &mut Session, id: &str, params: &Value) -> Value {
    session
        .execute(id, params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn session(tolerance: f64) -> Session {
    let mut s = Session::new(Workspace::unrestricted());
    run(
        &mut s,
        "file.new",
        &json!({"units": "m", "tolerance": tolerance}),
    );
    s
}

/// The steps of `file` (a JSON array of `[id, params]`).
fn steps(file: &str) -> Vec<(String, Value)> {
    let text = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/garabit_findings")
            .join(file),
    )
    .unwrap();
    serde_json::from_str(&text).unwrap()
}

/// `value` with every length times `k`: numbers (the points of sketches)
/// and texts such as "12.5 m".
fn scaled(value: &Value, k: f64) -> Value {
    match value {
        Value::Number(n) => json!(n.as_f64().unwrap() * k),
        Value::String(text) => match text.strip_suffix(" m").map(str::parse::<f64>) {
            Some(Ok(x)) => json!(format!("{} m", x * k)),
            _ => value.clone(),
        },
        Value::Array(items) => Value::Array(items.iter().map(|v| scaled(v, k)).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, v)| (key.clone(), scaled(v, k)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn feature(session: &mut Session, name: &str) -> Value {
    let document = run(session, "document.inspect", &json!({}));
    document["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == name)
        .unwrap_or_else(|| panic!("no feature {name}: {document}"))
        .clone()
}

/// The measure of the body of feature `name`.
fn measured(session: &mut Session, name: &str) -> Value {
    let all = run(session, "document.measure", &json!({}));
    all["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == name)
        .unwrap_or_else(|| panic!("no body {name}: {all}"))
        .clone()
}

fn volume(session: &mut Session, name: &str) -> f64 {
    measured(session, name)["volume"].as_f64().unwrap()
}

#[test]
fn an_open_loft_is_a_sheet_that_cuts_a_block() {
    let mut s = session(1e-6);
    for (x, name, control) in [
        (
            "0 m",
            "S0",
            json!([[-50, 10], [-20, 25], [0, 5], [20, 30], [50, 12]]),
        ),
        (
            "40 m",
            "S1",
            json!([[-50, 14], [-20, 20], [0, 8], [20, 26], [50, 10]]),
        ),
        (
            "80 m",
            "S2",
            json!([[-50, 12], [-20, 18], [0, 9], [20, 22], [50, 11]]),
        ),
    ] {
        run(
            &mut s,
            "sketch.create",
            &json!({"plane": "yz", "offset": x, "name": name}),
        );
        run(
            &mut s,
            "sketch.spline",
            &json!({"sketch": name, "control": control}),
        );
    }
    run(
        &mut s,
        "feature.loft",
        &json!({"sections": ["S0", "S1", "S2"], "name": "Sheet"}),
    );
    assert_eq!(feature(&mut s, "Sheet")["status"], "ok");
    // A sheet: no volume, no centroid.
    let sheet = measured(&mut s, "Sheet");
    assert_eq!(sheet["volume"], 0.0);
    assert!(sheet["centroid"].is_null());
    for (op, name) in [("subtract", "Below"), ("intersect", "Above")] {
        run(
            &mut s,
            "feature.block",
            &json!({"size": ["60 m", "80 m", "40 m"], "position": ["10 m", "-40 m", "-5 m"], "name": format!("{name}Block")}),
        );
        run(
            &mut s,
            "feature.boolean",
            &json!({"op": op, "target": format!("{name}Block"), "tool": "Sheet", "keepTool": true, "name": name}),
        );
        assert_eq!(feature(&mut s, name)["status"], "ok");
    }
    let (below, above) = (volume(&mut s, "Below"), volume(&mut s, "Above"));
    let whole = 60.0 * 80.0 * 40.0;
    assert!(
        (below + above - whole).abs() <= 1e-9 * whole,
        "{below} + {above} is not {whole}"
    );
    let height = |s: &mut Session, name: &str| measured(s, name)["centroid"][2].as_f64().unwrap();
    assert!(height(&mut s, "Below") < height(&mut s, "Above"));
    let check = run(&mut s, "document.check", &json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
}

#[test]
fn a_loft_takes_as_many_sections_as_its_spline_does() {
    assert_eq!(
        craft3d_doc::MAX_LOFT_SECTIONS,
        craft3d_kernel::loft::MAX_SECTIONS
    );
    assert_eq!(
        craft3d_kernel::loft::MAX_SECTIONS,
        craft3d_geom::refine::MAX_INTERPOLATED
    );
    let mut s = session(1e-6);
    let names: Vec<String> = (0..=craft3d_doc::MAX_LOFT_SECTIONS)
        .map(|i| format!("S{i}"))
        .collect();
    let error = s
        .execute("feature.loft", &json!({"sections": names}))
        .unwrap_err()
        .to_string();
    assert!(error.contains("at most 256, got 257"), "{error}");
}

#[test]
fn faces_at_the_edge_of_tolerance_are_refused_with_what_to_do() {
    let mut s = session(1e-6);
    run(
        &mut s,
        "feature.block",
        &json!({"size": ["10 m", "1 m", "1 m"], "name": "A"}),
    );
    run(
        &mut s,
        "feature.block",
        &json!({"size": ["10 m", "1 m", "1 m"], "position": ["9 m", "0.0000015 m", "0.2 m"], "name": "B"}),
    );
    let made = run(
        &mut s,
        "feature.boolean",
        &json!({"op": "union", "target": "A", "tool": "B", "keepTool": true, "name": "AB"}),
    );
    let error = made["error"].as_str().unwrap_or_default().to_string();
    assert_eq!(made["status"], "failed", "{made}");
    assert!(error.contains("at the edge of tolerance"), "{error}");
    assert!(error.contains("tolerance of 1e-6"), "{error}");
    assert!(error.contains("onto each other within half"), "{error}");
    // Within half the tolerance, or twice it apart: united.
    for (y, name) in [("0.0000003 m", "C"), ("0.000003 m", "D")] {
        run(
            &mut s,
            "feature.block",
            &json!({"size": ["10 m", "1 m", "1 m"], "name": format!("A{name}0")}),
        );
        run(
            &mut s,
            "feature.block",
            &json!({"size": ["10 m", "1 m", "1 m"], "position": ["9 m", y, "0.2 m"], "name": name}),
        );
        run(
            &mut s,
            "feature.boolean",
            &json!({"op": "union", "target": format!("A{name}0"), "tool": name, "name": format!("A{name}")}),
        );
        assert_eq!(feature(&mut s, &format!("A{name}"))["status"], "ok");
    }
}

#[test]
fn a_crack_cut_short_by_a_diagonal_unites() {
    let mut s = session(1e-6);
    for (id, params) in steps("pier2_min.steps.json") {
        run(&mut s, &id, &params);
    }
    for (target, tool, name) in [("P2_Leg3", "P2_G19", "U0"), ("U0", "P2_S55", "U1")] {
        run(
            &mut s,
            "feature.boolean",
            &json!({"op": "union", "target": target, "tool": tool, "name": name}),
        );
        assert_eq!(feature(&mut s, name)["status"], "ok", "{name}");
    }
    let check = run(&mut s, "document.check", &json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
}

#[test]
fn a_pit_at_the_height_of_the_lake_beside_it_cuts_the_terrain() {
    // The terrain's loft (64 sections), without the steps after it.
    let all = steps("terrain.steps.json");
    let till = all.iter().position(|(id, _)| id == "feature.loft").unwrap();
    for k in [1e-3, 1.0, 1e3] {
        let mut s = session(1e-6 * k);
        for (id, params) in all.iter().take(till + 1) {
            run(&mut s, id, &scaled(params, k));
        }
        assert_eq!(feature(&mut s, "Terrain")["status"], "ok");
        let whole = volume(&mut s, "Terrain");
        run(
            &mut s,
            "feature.transform",
            &json!({"source": "Terrain", "keep": true, "translate": ["0 m", "0 m", "0 m"], "name": "Again"}),
        );
        for (op, name, target) in [
            ("subtract", "Cut", "Terrain"),
            ("intersect", "Common", "Again"),
        ] {
            run(
                &mut s,
                "feature.block",
                &scaled(
                    &json!({"size": ["30 m", "20 m", "40 m"], "position": ["-104 m", "-10 m", "735 m"], "name": format!("{name}Pit")}),
                    k,
                ),
            );
            run(
                &mut s,
                "feature.boolean",
                &json!({"op": op, "target": target, "tool": format!("{name}Pit"), "name": name}),
            );
            assert_eq!(feature(&mut s, name)["status"], "ok", "{name} at {k}");
        }
        let (cut, common) = (volume(&mut s, "Cut"), volume(&mut s, "Common"));
        assert!(common > 0.0, "the pit reaches into the hill at {k}");
        assert!(
            (cut + common - whole).abs() <= 1e-9 * whole,
            "at {k}: {cut} + {common} is not {whole}"
        );
    }
}
