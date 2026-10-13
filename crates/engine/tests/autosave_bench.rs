//! Timing of an autosave of a document of some size, and of what a
//! command waits because of it (AGENTS.md rule 12). Ignored by default:
//! `cargo test -p craft3d-engine --test autosave_bench -- --ignored
//! --nocapture`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{RecoveryOptions, Session, Workspace};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// `parts` plates next to each other, each a polygon of `sides` sides with
/// a round hole, extruded by its own variable: three features and one
/// variable per part, as in `regen_bench.rs`.
fn model(parts: usize, sides: usize) -> Session {
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
            "feature.extrude",
            json!({"sketch": sketch, "distance": format!("h{part}"), "name": format!("P{part}")}),
        );
    }
    s
}

fn median(mut times: Vec<f64>) -> (f64, f64) {
    times.sort_by(f64::total_cmp);
    println!("    the five slowest: {:.3?}", &times[times.len() - 5..]);
    (times[times.len() / 2], times[times.len() - 1])
}

#[test]
#[ignore = "a timing, not a check"]
fn time_of_an_autosave_and_what_a_command_waits_for_it() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("autosave-bench");
    let _ = std::fs::remove_dir_all(&dir);
    let (parts, sides) = (150, 24);
    let clock = Instant::now();
    let mut s = model(parts, sides);
    let features = run(&mut s, "file.list", json!({}))["documents"][0]["features"].clone();
    println!(
        "model: {parts} parts, {features} features, built in {:.1} s",
        clock.elapsed().as_secs_f64()
    );

    // The cheapest command there is, without a store: the baseline.
    let time_commands = |s: &mut Session| {
        let times: Vec<f64> = (0..200)
            .map(|i| {
                let clock = Instant::now();
                run(
                    s,
                    "variable.set",
                    json!({"name": "spare", "expr": format!("{i} mm")}),
                );
                clock.elapsed().as_secs_f64() * 1e3
            })
            .collect();
        median(times)
    };
    let (base_median, base_max) = time_commands(&mut s.clone());

    // With a store that writes on its own thread after every change
    // (the shortest interval): every command hands over a snapshot while
    // the thread is busy writing the one before.
    s.enable_recovery(
        RecoveryOptions::new(dir.join("recovery")).with_interval(Duration::from_millis(10)),
    );
    let (with_median, with_max) = time_commands(&mut s);
    let waited = Instant::now();
    while s.recovery_overview().pending > 0 {
        assert!(waited.elapsed() < Duration::from_secs(60));
        std::thread::sleep(Duration::from_millis(2));
    }
    let overview = s.recovery_overview();
    let size = std::fs::read_dir(dir.join("recovery"))
        .unwrap()
        .map(|e| e.unwrap())
        .find(|e| e.file_name().to_str().unwrap().ends_with(".3dcraft"))
        .unwrap()
        .metadata()
        .unwrap()
        .len();
    println!(
        "autosave: {:.1} ms for the last write (serialising, writing, syncing and renaming {size} bytes plus the record), on the store's thread; {} writes during 200 commands",
        overview.last_write_ms.unwrap(),
        overview.writes
    );
    println!(
        "variable.set without a store: median {base_median:.3} ms, max {base_max:.3} ms; with autosave after every change: median {with_median:.3} ms, max {with_max:.3} ms"
    );

    // What the command's own thread does for autosave: noting what
    // changed.
    let times: Vec<f64> = (0..200)
        .map(|_| {
            let clock = Instant::now();
            s.autosave_poll();
            clock.elapsed().as_secs_f64() * 1e6
        })
        .collect();
    let (poll_median, poll_max) = median(times);
    println!("noting what changed (per command): median {poll_median:.2} µs, max {poll_max:.2} µs");
    s.end_recovery();
    assert!(
        std::fs::read_dir(dir.join("recovery"))
            .unwrap()
            .next()
            .is_none()
    );
}
