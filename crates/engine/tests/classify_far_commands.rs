//! What a user gets from the booleans of a plate far from the origin, at
//! a size in the millions (a document in micrometres, or in map
//! coordinates): the plate of `crates/kernel/tests/hunt_sheet_plates.rs`
//! case 123, where the point classifier was found wrong (2026-11-14), by
//! the commands a user calls. A rounded rectangle 54.667 by 20.261 with
//! corners of radius 10.1262 (its short sides 0.0086 long), 6.286 thick
//! about its sketch, drawn on a datum plane turned and some 2.6e8 from
//! the origin at scale 1e6 (`datum.plane` through three points,
//! `sketch.arc`, `sketch.line`, `feature.fill`, `feature.plate`), and a
//! ball of radius 3 about the middle of a short side united with it,
//! taken out of it and intersected with it (`feature.sphere`,
//! `feature.boolean`).
//!
//! In that frame the plate's short sides are ruled splines 6.286e6
//! across, where the search for a nearest point ended hundreds of
//! tolerances short (`crates/kernel/src/boolean/freeform.rs` ›
//! `descend`). The three booleans are held to the two volume identities
//! and to the same part at scale 1; so is a plate in the xy plane whose
//! outline has a spline, 8 thick, and a block through that rim.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::{body, new_document, run, session_in, workdir};
use craft3d_engine::Session;
use serde_json::{Value, json};

/// A command that is to build: its result, or its error.
fn step(s: &mut Session, id: &str, params: Value) -> Result<Value, String> {
    let result = s.execute(id, &params).map_err(|e| format!("{id}: {e}"))?;
    if result["status"] == "ok" {
        Ok(result)
    } else {
        Err(format!("{id}: {}", result["error"]))
    }
}

/// The sketch `Outline` filled and plated `thickness` thick on `side`,
/// then `tool` (a command and its parameters, the body named `Tool`)
/// united with the plate, intersected with it and taken out of it, each
/// in a document of its own made by `draw`: the volumes of the plate, the
/// tool and the three results, or the error of the first command that
/// failed.
fn volumes(
    k: f64,
    name: &str,
    draw: &dyn Fn(&mut Session),
    (thickness, side): (f64, &str),
    tool: (&str, Value),
) -> Result<[f64; 5], String> {
    let mut found = [0.0; 5];
    for (index, op) in ["union", "intersect", "subtract"].into_iter().enumerate() {
        let dir = workdir("classify-far", &format!("{name}-{k}-{op}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        draw(&mut s);
        step(
            &mut s,
            "feature.fill",
            json!({"sketch": "Outline", "name": "Sheet"}),
        )?;
        let face = body(&mut s, "Sheet")["faceNames"][0]
            .as_str()
            .unwrap()
            .to_string();
        step(
            &mut s,
            "feature.plate",
            json!({"faces": [face], "thickness": format!("{thickness} mm"), "side": side, "name": "Plate"}),
        )?;
        found[0] = body(&mut s, "Plate")["volume"].as_f64().unwrap();
        step(&mut s, tool.0, tool.1.clone())?;
        found[1] = body(&mut s, "Tool")["volume"].as_f64().unwrap();
        step(
            &mut s,
            "feature.boolean",
            json!({"op": op, "target": "Plate", "tool": "Tool", "name": "Result"}),
        )?;
        let result = body(&mut s, "Result");
        if result["valid"] != true {
            return Err(format!("{op}: the result is not valid: {result}"));
        }
        found[2 + index] = result["volume"].as_f64().unwrap();
    }
    Ok(found)
}

/// The tool passes through the plate's rim, and the three booleans meet
/// the two identities within a billionth of the plate's volume.
fn hold(what: &str, v: &[f64; 5]) {
    let [plate, tool, union, intersection, subtraction] = *v;
    let slack = 1e-9 * plate;
    assert!(
        intersection > 1e-3 * tool && intersection < 0.999 * tool,
        "{what}: the tool is to pass through the rim: {v:?}"
    );
    assert!(
        (union + intersection - plate - tool).abs() <= slack,
        "{what}: union {union:e} + intersection {intersection:e} is not plate {plate:e} + tool {tool:e}"
    );
    assert!(
        (subtraction + intersection - plate).abs() <= slack,
        "{what}: subtraction {subtraction:e} + intersection {intersection:e} is not the plate {plate:e}"
    );
}

/// The volumes at scale 1e6 are those at scale 1 times 1e18.
fn same_at_any_size(small: &[f64; 5], large: &[f64; 5]) {
    for (a, b) in small.iter().zip(large) {
        assert!(
            (b / 1e18 - a).abs() <= 1e-9 * small[0],
            "at scale 1e6 {large:?} against {small:?} at scale 1"
        );
    }
}

/// The plate of the finding and a ball about the middle of a short side,
/// at scale `k`.
fn far_plate(k: f64) -> Result<[f64; 5], String> {
    // The frame of the finding at scale 1e6: the middle of the plate, its
    // long axis, and the direction of its short sides.
    let long = [-0.83477, -0.48978, -0.25157];
    let short = [-0.50006, 0.48315, 0.71868];
    let middle = [-23.899e6, -156.750e6, -162.736e6].map(|c: f64| c * k / 1e6);
    let along = |d: [f64; 3], reach: f64| [0, 1, 2].map(|i| middle[i] + d[i] * reach * k);
    let (w, h, r) = (54.667, 20.261, 10.1262);
    let draw = |s: &mut Session| {
        run(
            s,
            "datum.plane",
            json!({"through": [middle, along(long, 100.0), along(short, 100.0)], "name": "Far"}),
        );
        run(
            s,
            "sketch.create",
            json!({"plane": "Far", "name": "Outline"}),
        );
        let at = |x: f64, y: f64| [x * k, y * k];
        let (x, y) = (0.5 * w, 0.5 * h);
        // Counter-clockwise from the lower right corner: an arc, a line.
        let centres = [
            (x - r, -y + r),
            (x - r, y - r),
            (-x + r, y - r),
            (-x + r, -y + r),
        ];
        let from = [(x - r, -y), (x, y - r), (-x + r, y), (-x, -y + r)];
        let to = [(x, -y + r), (x - r, y), (-x, y - r), (-x + r, -y)];
        for i in 0..4 {
            run(
                s,
                "sketch.arc",
                json!({"sketch": "Outline", "center": at(centres[i].0, centres[i].1),
                    "start": at(from[i].0, from[i].1), "end": at(to[i].0, to[i].1)}),
            );
            let next = from[(i + 1) % 4];
            run(
                s,
                "sketch.line",
                json!({"sketch": "Outline", "from": at(to[i].0, to[i].1), "to": at(next.0, next.1)}),
            );
        }
    };
    // About the middle of the short side at the end of the long axis,
    // within the plate's thickness.
    let ball = json!({"radius": 3.0 * k, "position": along(long, 0.5 * w), "name": "Tool"});
    volumes(
        k,
        "far",
        &draw,
        (6.286 * k, "mid"),
        ("feature.sphere", ball),
    )
}

#[test]
fn the_plate_of_the_finding_and_a_ball_through_its_short_side() {
    let small = far_plate(1.0).unwrap();
    hold("at scale 1", &small);
    let large = far_plate(1e6).unwrap_or_else(|e| panic!("at scale 1e6: {e}"));
    hold("at scale 1e6", &large);
    same_at_any_size(&small, &large);
}

/// A plate in the xy plane with a spline in its outline, 8 thick, and a
/// block through that rim, at scale `k`.
fn spline_plate(k: f64) -> Result<[f64; 5], String> {
    let draw = |s: &mut Session| {
        run(
            s,
            "sketch.create",
            json!({"plane": "xy", "name": "Outline"}),
        );
        let at = |x: f64, y: f64| [x * k, y * k];
        for (from, to) in [
            (at(0.0, 0.0), at(60.0, 0.0)),
            (at(60.0, 0.0), at(60.0, 30.0)),
            (at(0.0, 30.0), at(0.0, 0.0)),
        ] {
            run(
                s,
                "sketch.line",
                json!({"sketch": "Outline", "from": from, "to": to}),
            );
        }
        run(
            s,
            "sketch.spline",
            json!({"sketch": "Outline", "through": [
                at(60.0, 30.0), at(45.0, 36.0), at(30.0, 31.0), at(15.0, 37.0), at(0.0, 30.0)
            ]}),
        );
    };
    let block = json!({
        "size": [10.0 * k, 20.0 * k, 4.0 * k],
        "position": [25.0 * k, 25.0 * k, 2.0 * k],
        "name": "Tool"
    });
    volumes(
        k,
        "spline",
        &draw,
        (8.0 * k, "out"),
        ("feature.block", block),
    )
}

#[test]
fn a_plate_with_a_spline_rim_and_a_block_through_it() {
    let small = spline_plate(1.0).unwrap();
    hold("at scale 1", &small);
    let large = spline_plate(1e6).unwrap_or_else(|e| panic!("at scale 1e6: {e}"));
    hold("at scale 1e6", &large);
    same_at_any_size(&small, &large);
}
