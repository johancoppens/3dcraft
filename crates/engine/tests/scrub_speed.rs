//! Timing of scrubbing through the history (`view.scrub`, the feature
//! panel's timeline) on the cutaway turbofan (`bench_models`), as the
//! window does it: the quick first picture of each step on the thread that
//! asks, the display scene beside it. In release only:
//!
//! ```sh
//! cargo test --release -p craft3d-engine --test scrub_speed -- --nocapture
//! ```
//!
//! The first pass meets every step for the first time (its regeneration
//! from the document's cache, and the meshing of the bodies no scene had
//! yet); the second pass comes back to steps seen before, which must cost
//! next to nothing. The bounds are generous (a loaded machine runs three
//! times slower); the printed numbers are the measurement.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{SceneQuality, Session, Workspace};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

/// A step seen before is shown again within this.
const REVISIT: Duration = Duration::from_millis(100);
/// How many of the steps seen last a revisit must find kept.
const RECENT: u64 = 32;
/// The quick first picture of a step seen for the first time.
const FIRST_VISIT: Duration = Duration::from_secs(5);

fn steps(name: &str) -> Vec<(String, Value)> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/bench_models")
        .join(name);
    let text = std::fs::read_to_string(&path).unwrap();
    let list: Vec<(String, Value)> = serde_json::from_str(&text).unwrap();
    list
}

fn stats(label: &str, times: &mut [Duration]) -> Duration {
    times.sort();
    let total: Duration = times.iter().sum();
    let ms = |d: Duration| d.as_secs_f64() * 1e3;
    let median = times.get(times.len() / 2).copied().unwrap_or_default();
    let max = times.last().copied().unwrap_or_default();
    println!(
        "{label:<28} total {:>8.0} ms  median {:>7.1} ms  max {:>7.1} ms",
        ms(total),
        ms(median),
        ms(max)
    );
    max
}

#[test]
#[cfg_attr(debug_assertions, ignore = "timings are for release builds")]
fn scrubbing_the_turbofan_is_quick_and_a_revisit_is_immediate() {
    let mut session = Session::new(Workspace::unrestricted());
    session
        .execute("file.new", &json!({"units": "mm"}))
        .unwrap();
    for (id, params) in steps("turbofan.steps.json") {
        session
            .execute(&id, &params)
            .unwrap_or_else(|e| panic!("{id} {params}: {e}"));
    }
    session.set_refine_elsewhere(true);
    let quality = SceneQuality::display();
    session.scene_with(quality).unwrap().unwrap();
    let count = session.execute("view.scrub", &json!({})).unwrap()["steps"]
        .as_u64()
        .unwrap();
    println!("turbofan: {count} features");
    let (mut first, mut sharp, mut again) = (Vec::new(), Vec::new(), Vec::new());
    for step in 1..=count {
        let start = Instant::now();
        session
            .execute("view.scrub", &json!({"step": step}))
            .unwrap();
        let shown = session.scene_with(quality).unwrap().unwrap();
        first.push(start.elapsed());
        assert!(shown.preview || session.scene_ready(quality));
        if let Some(job) = session.refinement(quality) {
            let start = Instant::now();
            let refined = job.run(&AtomicBool::new(false));
            sharp.push(start.elapsed());
            session.accept_refined(refined);
        }
    }
    // Back over the steps just seen: their display scenes are kept.
    let recent = count.saturating_sub(RECENT).max(1);
    for step in (recent..=count).rev() {
        let start = Instant::now();
        session
            .execute("view.scrub", &json!({"step": step}))
            .unwrap();
        let shown = session.scene_with(quality).unwrap().unwrap();
        again.push(start.elapsed());
        assert!(!shown.preview, "step {step} lost its display scene");
    }
    // Further back, the scenes of the least recently shown steps went to
    // stay within the scene budget: their first picture comes again from
    // the meshes kept, and their regeneration was kept.
    let (mut older, mut rebuilt) = (Vec::new(), 0);
    for step in (1..recent).rev() {
        let start = Instant::now();
        session
            .execute("view.scrub", &json!({"step": step}))
            .unwrap();
        let shown = session.scene_with(quality).unwrap().unwrap();
        older.push(start.elapsed());
        if shown.preview {
            rebuilt += 1;
            if let Some(job) = session.refinement(quality) {
                session.accept_refined(job.run(&AtomicBool::new(false)));
            }
        }
    }
    let first_max = stats("first visit, first picture", &mut first);
    stats("first visit, display scene", &mut sharp);
    let again_max = stats("revisit, recent steps", &mut again);
    let older_max = stats("revisit, older steps", &mut older);
    println!("older steps whose scene had gone: {rebuilt}");
    assert!(older_max < FIRST_VISIT, "older revisit {older_max:?}");
    // Back to the end: the document as it is, its own scene.
    session
        .execute("view.scrub", &json!({"feature": null}))
        .unwrap();
    let start = Instant::now();
    assert!(!session.scene_with(quality).unwrap().unwrap().preview);
    println!("back to the end: {:?}", start.elapsed());
    assert!(first_max < FIRST_VISIT, "first visit {first_max:?}");
    assert!(again_max < REVISIT, "revisit {again_max:?}");
}
