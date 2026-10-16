//! Zero thickness (the developer's decision of 2026-10-15): a result in
//! which two parts of one body touch, along an edge or at a point, is
//! refused, as other modellers refuse it ("Cannot create zero-thickness
//! geometry"); separate bodies may touch. Through the commands, at three
//! scales.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn feature(session: &mut Session, name: &str) -> Value {
    let document = run(session, "document.inspect", json!({}));
    document["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == name)
        .unwrap_or_else(|| panic!("no feature {name}"))
        .clone()
}

fn bodies(session: &mut Session) -> Vec<(String, f64)> {
    let document = run(session, "document.inspect", json!({}));
    document["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| {
            (
                b["name"].as_str().unwrap().to_string(),
                b["volume"].as_f64().unwrap(),
            )
        })
        .collect()
}

/// A block of side `k` at the origin and another at `at` (times `k`).
fn two_blocks(k: f64, at: [f64; 3]) -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"tolerance": 1e-6 * k}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "First", "size": [k, k, k]}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Second", "size": [k, k, k],
               "position": [at[0] * k, at[1] * k, at[2] * k]}),
    );
    s
}

#[test]
fn blocks_that_share_an_edge_or_a_corner_are_not_united() {
    for k in [1e-3_f64, 1.0, 1e3] {
        // Along the upright edge at (k, k), and at the corner (k, k, k).
        for (at, touch) in [
            ([1.0, 1.0, 0.0], [1.0, 1.0, 0.5]),
            ([1.0, 1.0, 1.0], [1.0, 1.0, 1.0]),
        ] {
            let mut s = two_blocks(k, at);
            // Two bodies that touch are fine.
            assert_eq!(bodies(&mut s).len(), 2, "at {k}");
            let made = run(
                &mut s,
                "feature.boolean",
                json!({"op": "union", "target": "First", "tool": "Second", "name": "Both"}),
            );
            assert_eq!(made["status"], "failed", "{at:?} at {k}: {made}");
            let state = feature(&mut s, "Both");
            let reason = state["error"].as_str().unwrap();
            // The reason names where and both faces, and what to do.
            assert!(reason.contains("zero thickness"), "{reason}");
            assert!(
                reason.contains("F1:face(") && reason.contains("F2:face("),
                "{reason}"
            );
            assert!(reason.contains("move them apart"), "{reason}");
            let numbers: Vec<f64> = reason
                .split(['(', ')', ','])
                .filter_map(|t| t.trim().parse::<f64>().ok())
                .collect();
            let at_point = numbers.windows(3).any(|w| {
                (w[0] - touch[0] * k).abs() <= 1e-9 * k
                    && (w[1] - touch[1] * k).abs() <= 1e-9 * k
                    && (0.0..=k * (1.0 + 1e-9)).contains(&w[2])
                    && (touch[2] == 0.5 || (w[2] - touch[2] * k).abs() <= 1e-9 * k)
            });
            assert!(at_point, "{reason}");
            // Moved into each other a little, they are one body; moved
            // apart, the union is two parts that do not touch.
            for (shift, parts) in [(-0.1, 1), (0.1, 2)] {
                let mut s = two_blocks(k, [at[0] + shift, at[1] + shift, at[2]]);
                let made = run(
                    &mut s,
                    "feature.boolean",
                    json!({"op": "union", "target": "First", "tool": "Second", "name": "Both"}),
                );
                assert_eq!(made["status"], "ok", "{shift} at {k}: {made}");
                let inspect = run(&mut s, "document.inspect", json!({}));
                let both = inspect["bodies"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|b| b["name"] == "Both")
                    .unwrap()
                    .clone();
                let volume = both["volume"].as_f64().unwrap();
                let overlap = if parts == 1 {
                    let side = 0.1 * k;
                    side * side * if at[2] == 0.0 { k } else { 0.0 }
                } else {
                    0.0
                };
                let expected = 2.0 * k * k * k - overlap;
                assert!(
                    (volume - expected).abs() <= 1e-9 * expected,
                    "{shift} at {k}: {volume} vs {expected}"
                );
            }
        }
    }
}

/// A rod (radius `k`, height `2k`, its base at z = 0) standing on the top
/// edge of a bar on its edge (a square of side `k` turned 45 degrees,
/// along x from -k/2 to k/2, its top edge on the x axis at z = 0): the
/// base touches the bar along that edge, a line, and nowhere else, so
/// their union is two parts that touch along a line and is refused,
/// while the rod less the bar is the rod. The construction of
/// `step_freecad`'s `rod-on-edge` cases (which only `cargo xtask freecad`
/// runs), here so that the gate sees it.
fn rod_on_an_edge(k: f64) -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"tolerance": 1e-6 * k}));
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": k, "height": 2.0 * k, "name": "Rod"}),
    );
    run(
        &mut s,
        "sketch.create",
        json!({"name": "Diamond", "plane": "yz", "offset": -0.5 * k}),
    );
    let h = std::f64::consts::FRAC_1_SQRT_2;
    let lowest = -2.0 * h;
    let points: Vec<[f64; 2]> = [
        [0.0, lowest],
        [h, lowest + h],
        [0.0, lowest + 2.0 * h],
        [-h, lowest + h],
    ]
    .iter()
    .map(|p| [p[0] * k, p[1] * k])
    .collect();
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Diamond", "closed": true, "points": points}),
    );
    run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Diamond", "distance": k, "name": "Wedge"}),
    );
    s
}

#[test]
fn a_rod_standing_on_the_edge_of_a_bar_is_not_united_with_it() {
    for k in [1e-3_f64, 1.0, 1e6] {
        let mut s = rod_on_an_edge(k);
        // Two bodies that touch along a line are fine.
        let apart = bodies(&mut s);
        assert_eq!(apart.len(), 2, "at {k}: {apart:?}");
        let made = run(
            &mut s,
            "feature.boolean",
            json!({"op": "union", "target": "Rod", "tool": "Wedge", "name": "Result"}),
        );
        assert_eq!(made["status"], "failed", "at {k}: {made}");
        let state = feature(&mut s, "Result");
        let reason = state["error"].as_str().unwrap();
        assert!(reason.contains("zero thickness"), "at {k}: {reason}");
        // Where: on the bar's top edge, inside the rod's base.
        let numbers: Vec<f64> = reason
            .split(['(', ')', ','])
            .filter_map(|t| t.trim().parse::<f64>().ok())
            .collect();
        let on_the_edge = numbers.windows(3).any(|w| {
            w[0].abs() <= 0.5 * k * (1.0 + 1e-9) && w[1].abs() <= 1e-6 * k && w[2].abs() <= 1e-6 * k
        });
        assert!(on_the_edge, "at {k}: {reason}");

        // The rod less the bar is the rod: they share no volume.
        let mut s = rod_on_an_edge(k);
        let made = run(
            &mut s,
            "feature.boolean",
            json!({"op": "subtract", "target": "Rod", "tool": "Wedge", "name": "Result"}),
        );
        assert_eq!(made["status"], "ok", "at {k}: {made}");
        let result = bodies(&mut s);
        let rod = result
            .iter()
            .find(|(name, _)| name == "Result")
            .unwrap_or_else(|| panic!("at {k}: {result:?}"));
        let expected = 2.0 * std::f64::consts::PI * k * k * k;
        assert!(
            (rod.1 - expected).abs() <= 1e-9 * expected,
            "at {k}: {} vs {expected}",
            rod.1
        );
    }
}
