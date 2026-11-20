//! A hub's bore on its shaft's seat is a touch, also where the keyway's
//! rounded end has a vertex next to the side of the hub's keyway (the gear
//! reducer Jig's keyed shafts, 2026-11-20; design:
//! docs/design/booleans.md › *Contact refused where a curve lies exactly on
//! a face's border*).
//!
//! The bodies are made here, by the commands a Jig uses, in millimetres at
//! the Jig's tolerance of 0.01 mm: a shaft with a keyway cut from a sketch
//! (two lines and two rounded ends), and a hub extruded from its outline
//! with a keyway of the same width, so the rim of the keyway's end in the
//! shaft is tangent to the border of the hub's bore. The end is drawn as
//! two arcs that meet a given number of tolerances off the side of the
//! hub's keyway: the vertex the Jig's shafts have there, where the trace
//! of the end's rim began. `measure.clearance` and `document.interference`
//! answer a touch on the area of the bore, in closed form; between one and
//! four tolerances both were refused before.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_math::Portable;
use serde_json::{Value, json};
use std::f64::consts::PI;

const TOLERANCE: f64 = 0.01;

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// A shaft of radius `seat` along z with a keyway of half width `w` and
/// depth `w` whose ends are rounded about `z = 1` and `z = 44`, and a hub
/// from `z = -3` to `48` on it with a keyway of the same width. With an
/// `offset` the keyway's end at `z = 1` is two arcs that meet that many
/// tolerances off the keyway's side.
fn keyed(seat: f64, w: f64, offset: Option<f64>) -> Session {
    let mut s = Session::new(Workspace::none());
    run(
        &mut s,
        "file.new",
        json!({"units": "mm", "tolerance": TOLERANCE}),
    );
    let (z0, z1) = (1.0, 44.0);
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": seat, "height": 60.0, "position": [0.0, 0.0, -4.0], "name": "Blank"}),
    );
    run(
        &mut s,
        "sketch.create",
        json!({"name": "Slot", "plane": "yz", "offset": seat - w}),
    );
    let mut segments = vec![
        json!({"line": [w, z1]}),
        json!({"arc": [-w, z1], "center": [0.0, z1]}),
        json!({"line": [-w, z0]}),
    ];
    if let Some(e) = offset {
        let away = (1.0 - e * TOLERANCE / w).pacos();
        segments.push(json!({
            "arc": [-w * away.pcos(), z0 - w * away.psin()],
            "center": [0.0, z0],
        }));
    }
    segments.push(json!({"arc": "start", "center": [0.0, z0]}));
    run(
        &mut s,
        "sketch.path",
        json!({"sketch": "Slot", "start": [w, z0], "segments": segments}),
    );
    run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Slot", "distance": w + 1.0, "name": "Cutter"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Blank", "tool": "Cutter", "name": "Shaft"}),
    );
    let x = (seat * seat - w * w).sqrt();
    run(
        &mut s,
        "sketch.create",
        json!({"name": "Hub outline", "plane": "xy", "offset": -3.0}),
    );
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Hub outline", "center": [0.0, 0.0], "radius": seat + 8.0}),
    );
    run(
        &mut s,
        "sketch.path",
        json!({"sketch": "Hub outline", "start": [x, -w], "segments": [
            {"line": [seat + 2.5, -w]},
            {"line": [seat + 2.5, w]},
            {"line": [x, w]},
            {"arc": "start", "center": [0.0, 0.0]},
        ]}),
    );
    run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Hub outline", "distance": 51.0, "name": "Hub"}),
    );
    s
}

#[test]
fn a_hub_on_a_keyed_shaft_touches_whatever_vertex_the_keyways_end_has() {
    // The proportions of the Jig's output and intermediate shafts. The
    // offsets: no vertex, one within the tolerance of the side, the Jig's
    // (1.7 tolerances on the intermediate shaft, 3.1 on the output shaft),
    // and one well clear.
    for (seat, w) in [(12.5, 4.0), (9.0, 3.0)] {
        for offset in [
            None,
            Some(0.4),
            Some(1.4),
            Some(1.7),
            Some(3.0),
            Some(3.1),
            Some(8.0),
        ] {
            let mut s = keyed(seat, w, offset);
            let what = format!("seat {seat}, key {}, offset {offset:?}", 2.0 * w);
            let out = run(
                &mut s,
                "measure.clearance",
                json!({"from": "Hub", "to": "Shaft"}),
            );
            assert_eq!(out["least"]["status"], "touch", "{what}: {out}");
            assert_eq!(out["clearance"].as_f64(), Some(0.0), "{what}: {out}");
            let out = run(
                &mut s,
                "document.interference",
                json!({"pairs": [["Hub", "Shaft"]]}),
            );
            let pair = &out["pairs"][0];
            assert_eq!(pair["status"], "touch", "{what}: {out}");
            assert_eq!(pair["contact"], "face", "{what}: {out}");
            // The bore: the seat's cylinder less the keyway's width, over
            // the hub's length.
            let bore = seat * (2.0 * PI - 2.0 * (w / seat).pasin()) * 51.0;
            let area = pair["area"].as_f64().unwrap();
            assert!(
                (area - bore).abs() <= TOLERANCE * 200.0,
                "{what}: an area of {area}, not {bore}"
            );
        }
    }
}
