//! Through the commands: the floor of a round pocket under a spline wall
//! raised past the wall's lowest point over it, where the pocket's rim
//! runs out through the wall. The feature fails, saying how far the floor
//! can go: offset face asks the spline wall whether the rim went through
//! it. Short of the wall the floor is built, and is the pocket the boolean
//! cuts to that height.
//!
//! Until 2026-11-15 the floor 0.02 mm past the wall's lowest point was
//! built with status ok and passed `document.check` with `deep` (the
//! second review of 2026-11-14; pinned here as `BUILT_THROUGH_KNOWN`,
//! taken off with the repair): no freeform face was asked, and at that
//! depth the mesh the regeneration looks at shows no crossing. The
//! kernel's side of this, with the other kinds of case and three scales:
//! `crates/kernel/tests/face_op_freeform_known.rs`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::{body, built, new_document, run, session};
use craft3d_engine::Session;
use serde_json::json;

/// The height of the wall's spline over x 20.5, where it is lowest over
/// the pocket.
fn lowest() -> f64 {
    let cubic = |t: f64, p: [f64; 4]| {
        let u = 1.0 - t;
        u * u * u * p[0] + 3.0 * u * u * t * p[1] + 3.0 * u * t * t * p[2] + t * t * t * p[3]
    };
    // x falls from 25 to 5 along the spline.
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if cubic(mid, [25.0, 18.0, 12.0, 5.0]) > 20.5 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    cubic(0.5 * (lo + hi), [20.0, 14.0, 27.0, 20.0])
}

/// A block 30 x 20, 40 long, its top between x 5 and x 25 a spline wall,
/// with a round pocket of radius 3 from its bottom up to `floor`, the
/// pocket's seam turned `turn` degrees about its axis: the body
/// `Pocketed`.
fn pocketed(floor: f64, turn: f64) -> Session {
    let mut s = session();
    new_document(&mut s, 1.0);
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "name": "Outline"}),
    );
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Outline", "points": [[5.0, 20.0], [0.0, 20.0], [0.0, 0.0], [30.0, 0.0], [30.0, 20.0], [25.0, 20.0]]}),
    );
    run(
        &mut s,
        "sketch.spline",
        json!({"sketch": "Outline", "control": [[25.0, 20.0], [18.0, 14.0], [12.0, 27.0], [5.0, 20.0]]}),
    );
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Outline", "distance": 40.0, "name": "Prism"}),
    );
    built(
        &mut s,
        "feature.cylinder",
        json!({"radius": 3.0, "height": floor + 5.0, "position": [17.5, -5.0, 20.0], "axis": [0.0, 1.0, 0.0], "angle": format!("{turn} deg"), "name": "Tool"}),
    );
    built(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Prism", "tool": "Tool", "name": "Pocketed"}),
    );
    s
}

/// The pocket's floor: the tool's end cap.
const FLOOR: &str = "F4:copy(1)/F3:face(2)";

#[test]
fn a_pockets_floor_raised_out_through_a_spline_wall_fails() {
    let mut unexpected = Vec::new();
    for turn in [0.0, 90.0, 180.0, 270.0] {
        for out in [-0.3, -0.001, 0.02, 0.3, 1.0] {
            let to = lowest() + out;
            let mut s = pocketed(12.0, turn);
            let made = run(
                &mut s,
                "feature.offsetFace",
                json!({"faces": [FLOOR], "distance": -(to - 12.0), "name": "Moved"}),
            );
            let check = run(&mut s, "document.check", json!({"deep": true}));
            let what = format!("seam turned {turn}, the floor {out} over the wall's lowest");
            if out < 0.0 {
                // The same pocket cut to that height by the boolean.
                let direct = body(&mut pocketed(to, turn), "Pocketed")["volume"]
                    .as_f64()
                    .unwrap();
                let same = made["status"] == "ok" && check["ok"] == true && {
                    let volume = body(&mut s, "Moved")["volume"].as_f64().unwrap();
                    (volume - direct).abs() <= 1e-9 * direct
                };
                if same {
                    println!("{what}: built right");
                } else {
                    unexpected.push(format!(
                        "{what}: status {} ({}), deep check {}",
                        made["status"], made["error"], check["ok"]
                    ));
                }
                continue;
            }
            // Over the wall: fails, for the wall, with how far the floor
            // can go (to the wall's lowest point: 6.3838 mm).
            let error = made["error"].as_str().unwrap_or_default();
            if made["status"] == "failed"
                && error.contains("would pass through another part of the body")
                && error.contains("what fits is above -6.3838 mm")
            {
                println!("{what}: fails: {error}");
            } else {
                unexpected.push(format!(
                    "{what}: status {} ({error}), deep check {}",
                    made["status"], check["ok"]
                ));
            }
        }
    }
    assert!(unexpected.is_empty(), "{unexpected:#?}");
}
