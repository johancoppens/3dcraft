//! **Known wrong, owed** (the second review of 2026-11-14), through the
//! commands: the floor of a round pocket under a spline wall raised 0.02 mm
//! past the wall's lowest point over it. The pocket's rim runs out through
//! the wall there; offset face asks no freeform face whether the moved
//! face went through it, so the feature is built (status ok), and at this
//! depth the mesh the regeneration looks at shows no crossing either:
//! `document.check` with `deep` passes. The body is wrong: it measures
//! less than the same pocket cut to that height by the boolean, which is
//! the body with its rim open through the wall.
//!
//! From 0.3 mm the regeneration's self-check refuses the feature, and
//! short of the wall the floor is built right. The kernel's side of this,
//! with the other kinds of case and three scales:
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
/// with a round pocket of radius 3 from its bottom up to `floor`: the
/// body `Pocketed`.
fn pocketed(floor: f64) -> Session {
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
        json!({"radius": 3.0, "height": floor + 5.0, "position": [17.5, -5.0, 20.0], "axis": [0.0, 1.0, 0.0], "name": "Tool"}),
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

/// How far over the wall's lowest point the floor is built and passes the
/// deep check.
const BUILT_THROUGH_KNOWN: [f64; 1] = [0.02];

#[test]
fn a_pockets_floor_raised_out_through_a_spline_wall_fails() {
    let mut unexpected = Vec::new();
    for out in [-0.3, 0.02, 0.3] {
        let to = lowest() + out;
        let mut s = pocketed(12.0);
        let made = run(
            &mut s,
            "feature.offsetFace",
            json!({"faces": [FLOOR], "distance": -(to - 12.0), "name": "Moved"}),
        );
        let check = run(&mut s, "document.check", json!({"deep": true}));
        let passes = made["status"] == "ok" && check["ok"] == true;
        // The same pocket cut to that height: with its rim open through
        // the wall where the floor is over the wall's lowest point.
        let direct = body(&mut pocketed(to), "Pocketed")["volume"]
            .as_f64()
            .unwrap();
        let same = passes && {
            let volume = body(&mut s, "Moved")["volume"].as_f64().unwrap();
            (volume - direct).abs() <= 1e-9 * direct
        };
        let known = BUILT_THROUGH_KNOWN.contains(&out);
        let what = format!("the floor {out} over the wall's lowest");
        match (passes, same) {
            (true, true) if out < 0.0 => println!("{what}: built right"),
            (true, false) if known => println!("{what}: known wrong, and the deep check passes"),
            (false, _) if out > 0.0 && !known => {
                println!("{what}: fails: {}", made["error"]);
            }
            (false, _) if known => unexpected.push(format!(
                "{what}: fails now ({}): take it off BUILT_THROUGH_KNOWN",
                made["error"]
            )),
            _ => unexpected.push(format!(
                "{what}: status {}, deep check {}, the boolean's volume {}",
                made["status"],
                check["ok"],
                if same { "matched" } else { "not matched" }
            )),
        }
    }
    assert!(unexpected.is_empty(), "{unexpected:#?}");
}
