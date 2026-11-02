//! Arcs in the press brake's tools (docs/design/sheetmetal.md › *Bend
//! sequence* › *Arcs in the tools*), at 1e-3, 1 and 1e6: a side of a
//! punch or a die given a point it passes through is that circular arc,
//! exactly, not a polygon drawn round it.
//!
//! - A rod punch of radius `a` about the bend's axis (two corners, both
//!   sides arcs) on the U channel of `sheetmetal_bendsequence.rs` (base
//!   40 between its tangent lines, R = T = 1): a disc about the axis
//!   keeps its distance to every piece the parent carries as the legs
//!   turn about the axis, so the first bend's least distance is the other
//!   strip's near edge, `√(40² + R²) − a`, and the second's the standing
//!   wall's inner face, `40 − R − a`, at every stroke angle; the same rod
//!   as an octagon set flat towards the wall is `a (1 − cos π/8)` further.
//! - A rack panel's ears over a V die whose shoulders are rounded (an arc
//!   tangent to its top and its flank) are proven in the die's opening.
//! - Bad arcs are errors: a point in a line with its side's ends, a
//!   `through` first in the list or after another, one with other keys.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;

use common::checked::{built, error, new_document, run, session_in, workdir};
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::f64::consts::PI;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn mm(v: f64) -> String {
    format!("{v} mm")
}

/// The U channel: base 40 × 30, walls 20 high up on its left and right
/// edges, R = T = 1, K = ½.
fn channel(s: &mut Session, k: f64) {
    run(s, "sketch.create", json!({"plane": "xy", "name": "Base"}));
    let made = run(
        s,
        "sketch.rect",
        json!({"sketch": "Base", "from": [0.0, 0.0], "to": [40.0 * k, 30.0 * k]}),
    );
    let lines: Vec<String> = made["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap().to_string())
        .collect();
    let flanges: Vec<Value> = [3, 1]
        .iter()
        .map(|&e| json!({"edge": lines[e], "length": mm(20.0 * k), "angle": "90 deg"}))
        .collect();
    built(
        s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "name": "U", "thickness": mm(k), "radius": mm(k),
               "kFactor": 0.5, "flanges": flanges}),
    );
}

fn scaled(points: &[[f64; 2]], k: f64) -> Value {
    json!(
        points
            .iter()
            .map(|p| [p[0] * k, p[1] * k])
            .collect::<Vec<_>>()
    )
}

const DIE: [[f64; 2]; 4] = [[-15.0, -20.0], [15.0, -20.0], [15.0, 0.0], [-15.0, 0.0]];

/// The rod: corners at `(±a, 0)`, the sides the half circles through
/// `(0, −a)` and `(0, a)`.
fn rod(a: f64) -> Value {
    json!([[-a, 0.0], {"through": [0.0, -a]}, [a, 0.0], {"through": [0.0, a]}])
}

#[test]
fn a_rod_punch_keeps_its_closed_form_distance() {
    let (r, a) = (1.0, 10.0);
    for k in SCALES {
        let dir = workdir("sheetmetal-tool-arcs", &format!("rod-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        channel(&mut s, k);
        let out = run(
            &mut s,
            "sheetMetal.bendSequence",
            json!({"feature": "U", "punch": rod(a * k), "die": scaled(&DIE, k),
                   "order": [0, 1]}),
        );
        assert_eq!(out["feasible"], true, "scale {k}: {out}");
        let tol = 1e-6 * k;
        let first = (40.0_f64 * 40.0 + r * r).sqrt() - a;
        let second = 40.0 - r - a;
        for (step, exact) in out["steps"].as_array().unwrap().iter().zip([first, second]) {
            let least = step["least"].as_f64().unwrap();
            let proven = step["proven"].as_f64().unwrap();
            // Proven from the mesh less what it can be off by: at most
            // the true distance, within a few tolerances of it.
            assert!(
                least <= exact * k + 1e-9 * k && least >= exact * k - 4.0 * tol,
                "scale {k}: least {least} against {}: {step}",
                exact * k
            );
            let clearance = out["clearance"].as_f64().unwrap();
            assert!(proven <= least && proven >= clearance, "scale {k}: {step}");
        }

        // The same rod as an octagon, a side square to the wall: its
        // flat side stands `a cos(π/8)` from the axis.
        let octagon: Vec<[f64; 2]> = (0..8)
            .map(|i| {
                let t = PI / 8.0 + PI / 4.0 * f64::from(i);
                [a * k * t.pcos(), a * k * t.psin()]
            })
            .collect();
        let out = run(
            &mut s,
            "sheetMetal.bendSequence",
            json!({"feature": "U", "punch": octagon, "die": scaled(&DIE, k),
                   "order": [0, 1]}),
        );
        let least = out["steps"][1]["least"].as_f64().unwrap();
        let exact = (40.0 - r - a * (PI / 8.0).pcos()) * k;
        assert!(
            least <= exact + 1e-9 * k && least >= exact - 4.0 * tol,
            "scale {k}: octagon {least} against {exact}"
        );
    }
}

/// A rack panel (as in `sheetmetal_rest_proofs.rs`): ears 5 past the bend
/// line beside an inset flange.
fn rack_panel(s: &mut Session, k: f64) {
    let points = [
        [-10.0, -5.0],
        [-2.0, -5.0],
        [-2.0, 0.0],
        [42.0, 0.0],
        [42.0, -5.0],
        [50.0, -5.0],
        [50.0, 30.0],
        [-10.0, 30.0],
    ];
    run(s, "sketch.create", json!({"plane": "xy", "name": "Base"}));
    let made = run(
        s,
        "sketch.polyline",
        json!({"sketch": "Base", "closed": true, "points": scaled(&points, k)}),
    );
    let line = made["lines"][2].as_str().unwrap().to_string();
    built(
        s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "name": "U", "thickness": mm(k), "radius": mm(k),
               "kFactor": 0.5,
               "flanges": [{"edge": line, "length": mm(10.0 * k), "angle": "90 deg",
                            "insetStart": mm(2.0 * k), "insetEnd": mm(2.0 * k)}]}),
    );
}

/// A V die 40 wide, its opening 16 wide and 8 deep at 45 degrees, each
/// shoulder rounded by an arc of radius `f` tangent to the top and the
/// flank: the tangent points `f tan(22.5°)` from the corner, the arc
/// through the point of the circle nearest the corner.
fn rounded_v(f: f64, k: f64) -> Value {
    let d = f * (PI / 8.0).ptan();
    let s2 = 0.5_f64.sqrt();
    let centre = [8.0 + d, -f];
    // From the centre towards the corner (8, 0).
    let (vx, vy) = (8.0 - centre[0], -centre[1]);
    let len = vx.phypot(vy);
    let mid = [centre[0] + f * vx / len, centre[1] + f * vy / len];
    let p = |x: f64, y: f64| json!([x * k, y * k]);
    let through = |x: f64, y: f64| json!({"through": [x * k, y * k]});
    json!([
        p(-20.0, -20.0),
        p(20.0, -20.0),
        p(20.0, 0.0),
        p(8.0 + d, 0.0),
        through(mid[0], mid[1]),
        p(8.0 - d * s2, -d * s2),
        p(0.0, -8.0),
        p(-8.0 + d * s2, -d * s2),
        through(-mid[0], mid[1]),
        p(-8.0 - d, 0.0),
        p(-20.0, 0.0)
    ])
}

#[test]
fn ears_clear_a_v_die_with_rounded_shoulders() {
    for k in SCALES {
        let dir = workdir("sheetmetal-tool-arcs", &format!("ears-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        rack_panel(&mut s, k);
        let punch = scaled(&[[-1.0, 0.0], [1.0, 0.0], [1.0, 100.0], [-1.0, 100.0]], k);
        let out = run(
            &mut s,
            "sheetMetal.bendSequence",
            json!({"feature": "U", "punch": punch, "die": rounded_v(2.0, k)}),
        );
        assert_eq!(out["feasible"], true, "scale {k}: {out}");
        assert!(
            out["collisions"].as_array().unwrap().is_empty(),
            "scale {k}: {out}"
        );
    }
}

#[test]
fn bad_arcs_are_errors() {
    let k = 1.0;
    let dir = workdir("sheetmetal-tool-arcs", "bad");
    let mut s = session_in(&dir);
    new_document(&mut s, k);
    channel(&mut s, k);
    let die = scaled(&DIE, k);
    for (punch, why) in [
        // The point in a line with the side's ends.
        (
            json!([[-1, 0], {"through": [0, 0]}, [1, 0], [1, 9], [-1, 9]]),
            "in a line",
        ),
        // First in the list: no side before it.
        (
            json!([{"through": [0, -1]}, [-1, 0], [1, 0], [1, 9], [-1, 9]]),
            "follows a corner",
        ),
        // Two in a row.
        (
            json!([[-1, 0], {"through": [0, -1]}, {"through": [0, -2]}, [1, 0], [1, 9]]),
            "follows a corner",
        ),
        // Other keys.
        (
            json!([[-1, 0], {"through": [0, -1], "radius": 1}, [1, 0], [1, 9], [-1, 9]]),
            "nothing else",
        ),
        (
            json!([[-1, 0], {"through": "x"}, [1, 0], [1, 9]]),
            "at least 3",
        ),
        // Two corners need both sides arcs.
        (
            json!([[-1, 0], {"through": [0, -1]}, [1, 0]]),
            "2 with both",
        ),
    ] {
        let got = error(
            &mut s,
            "sheetMetal.bendSequence",
            json!({"feature": "U", "punch": punch.clone(), "die": die.clone()}),
        );
        assert!(got.contains(why), "{punch}: {got}");
    }
    // As an assertion's tools too.
    run(
        &mut s,
        "assert.add",
        json!({"name": "B", "measure": "bendable", "of": "U",
               "punch": [[-1, 0], {"through": [0, 0]}, [1, 0], [1, 9], [-1, 9]],
               "die": die, "is": true}),
    );
    let listed = run(&mut s, "assert.list", json!({"details": true}));
    let text = listed.to_string();
    assert!(text.contains("in a line"), "{listed}");
}
