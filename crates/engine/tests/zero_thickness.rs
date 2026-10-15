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
