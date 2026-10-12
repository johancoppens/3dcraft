//! Timing of an edit on a model of some size (AGENTS.md rule 12). Ignored
//! by default: `cargo test --release -p craft3d-engine --test regen_bench
//! -- --ignored --nocapture`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};
use std::time::Instant;

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// `parts` plates next to each other, each a polygon of `sides` sides with
/// a round hole, extruded by its own variable `h<i>`.
pub fn model(parts: usize, sides: usize) -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({}));
    for part in 0..parts {
        run(
            &mut s,
            "variable.set",
            json!({"name": format!("h{part}"), "expr": "5 mm"}),
        );
        let sketch = format!("S{part}");
        run(&mut s, "sketch.create", json!({"name": sketch}));
        let cx = 100.0 * part as f64;
        let points: Vec<[f64; 2]> = (0..sides)
            .map(|i| {
                let a = std::f64::consts::TAU * i as f64 / sides as f64;
                // A wavy outline, so no two sides are parallel by accident.
                let r = 40.0 + 3.0 * (7.0 * a).sin();
                [cx + r * a.cos(), r * a.sin()]
            })
            .collect();
        run(
            &mut s,
            "sketch.polyline",
            json!({"sketch": sketch, "closed": true, "points": points}),
        );
        run(
            &mut s,
            "sketch.circle",
            json!({"sketch": sketch, "center": [cx, 0], "radius": 10}),
        );
        let made = run(
            &mut s,
            "feature.extrude",
            json!({"sketch": sketch, "distance": format!("h{part}"), "name": format!("P{part}")}),
        );
        assert_eq!(made["status"], "ok", "{made}");
    }
    s
}

#[test]
#[ignore = "a timing, not a check"]
fn time_of_an_edit_that_touches_one_part_of_many() {
    for (parts, sides) in [(10, 200), (40, 200), (10, 1000)] {
        let built = Instant::now();
        let mut s = model(parts, sides);
        let build = built.elapsed();
        let mut times = Vec::new();
        for round in 0..5 {
            let started = Instant::now();
            let last = parts - 1;
            run(
                &mut s,
                "variable.set",
                json!({"name": format!("h{last}"), "expr": format!("{} mm", 6 + round)}),
            );
            let edited = started.elapsed().as_secs_f64() * 1e3;
            if round == 4 {
                println!(
                    "  the edit alone {edited:.1} ms; features rebuilt and reused: {:?}",
                    s.last_regeneration()
                );
            }
            let measured = run(
                &mut s,
                "document.measure",
                json!({"feature": format!("P{last}")}),
            );
            assert!(measured["volume"].as_f64().unwrap() > 0.0);
            times.push(started.elapsed().as_secs_f64() * 1e3);
        }
        times.sort_by(f64::total_cmp);
        println!(
            "{parts} parts of {sides} sides: built in {:.0} ms; one edit of one part: median {:.1} ms (min {:.1}, max {:.1})",
            build.as_secs_f64() * 1e3,
            times[2],
            times[0],
            times[4]
        );
    }
}
