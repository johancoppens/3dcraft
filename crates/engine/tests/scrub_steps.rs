//! Stepping through the history as the window's history player does it:
//! forward one step at a time from the first feature to the last, then
//! back, on the models of `bench_models`. What each step's regeneration
//! costs, what its first picture and its display scene cost, and that
//! every state shown is the one a regeneration from nothing gives, bit for
//! bit. Also under a small body budget, where the document's cache has let
//! go of the bodies of the early steps (a model whose bodies together take
//! more than the budget). In release only:
//!
//! ```sh
//! cargo test --release -p craft3d-engine --test scrub_steps -- --nocapture --test-threads 1
//! ```

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

fn session(name: &str, budget: Option<usize>) -> Session {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/bench_models")
        .join(name);
    let text = std::fs::read_to_string(&path).unwrap();
    let list: Vec<(String, Value)> = serde_json::from_str(&text).unwrap();
    let mut session = Session::new(Workspace::unrestricted());
    if let Some(bytes) = budget {
        session.set_body_budget(bytes);
    }
    session
        .execute("file.new", &json!({"units": "mm"}))
        .unwrap();
    for (id, params) in list {
        session
            .execute(&id, &params)
            .unwrap_or_else(|e| panic!("{id} {params}: {e}"));
    }
    session.set_refine_elsewhere(true);
    session
}

/// `name` saved as a `.3dcraft` and opened again in a new session, as a
/// model is opened in the window.
fn opened(name: &str) -> Session {
    let mut built = session(name, None);
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("scrub-steps");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name.replace(".steps.json", ".3dcraft"));
    let path = path.to_str().unwrap();
    built.execute("file.save", &json!({"path": path})).unwrap();
    let mut session = Session::new(Workspace::unrestricted());
    session
        .execute("file.open", &json!({"path": path}))
        .unwrap();
    session.set_refine_elsewhere(true);
    session
}

/// What building the scrubbed state's regeneration took.
fn scrubbed_regeneration(session: &mut Session) -> Option<craft3d_engine::ScrubRegeneration> {
    session.scrubbed_regeneration()
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

fn line(label: &str, times: &mut [Duration]) -> Duration {
    times.sort();
    let total: Duration = times.iter().sum();
    let median = times.get(times.len() / 2).copied().unwrap_or_default();
    let max = times.last().copied().unwrap_or_default();
    println!(
        "  {label:<30} total {:>8.0} ms  median {:>7.1} ms  max {:>7.1} ms",
        ms(total),
        ms(median),
        ms(max)
    );
    max
}

/// One pass over `steps`: the regeneration, the first picture and the
/// display scene of each, timed apart; every `check`th state compared
/// with a regeneration from nothing.
fn pass(session: &mut Session, label: &str, steps: &[u64], check: u64) {
    let quality = SceneQuality::display();
    let (mut regen, mut first, mut sharp, mut fresh) = (vec![], vec![], vec![], vec![]);
    let (mut rebuilt, mut rederived, mut kept) = (0, 0, 0);
    let mut tolerances: Vec<f64> = Vec::new();
    for &step in steps {
        let start = Instant::now();
        session
            .execute("view.scrub", &json!({"step": step}))
            .unwrap();
        if let Some(took) = scrubbed_regeneration(session) {
            rebuilt += took.rebuilt;
            rederived += took.rederived;
            kept += usize::from(took.kept);
        }
        regen.push(start.elapsed());
        let start = Instant::now();
        session.scene_with(quality).unwrap().unwrap();
        first.push(start.elapsed());
        if let Some(job) = session.refinement(quality) {
            let start = Instant::now();
            let refined = job.run(&AtomicBool::new(false));
            let took = start.elapsed();
            sharp.push(took);
            session.accept_refined(refined);
            let scene = session.scene_with(quality).unwrap().unwrap();
            let tolerance = scene.sampling().0;
            if tolerances.last() != Some(&tolerance) {
                tolerances.push(tolerance);
            }
            if took > Duration::from_millis(100) {
                println!(
                    "  step {step}: display scene {:.0} ms, tolerance {tolerance:.3e}, {} bodies, {} triangles",
                    ms(took),
                    scene.bodies.len(),
                    scene.triangles()
                );
            }
        }
        if check > 0 && step % check == 0 {
            let start = Instant::now();
            if let Some(same) = session.scrubbed_matches_a_full_one() {
                assert!(
                    same,
                    "{label}: step {step} differs from a fresh regeneration"
                );
                fresh.push(start.elapsed());
            }
        }
    }
    println!(
        "{label}: {} steps, features rebuilt {rebuilt}, bodies derived again {rederived}, states kept {kept}",
        steps.len()
    );
    println!("  display tolerances in turn: {tolerances:?}");
    line("regeneration", &mut regen);
    line("first picture", &mut first);
    line("display scene", &mut sharp);
    if !fresh.is_empty() {
        line("fresh regeneration (sampled)", &mut fresh);
    }
}

fn model(name: &str, budget: Option<usize>) {
    walk(name, budget, session(name, budget));
}

fn walk(name: &str, budget: Option<usize>, mut session: Session) {
    let quality = SceneQuality::display();
    session.scene_with(quality).unwrap().unwrap();
    if let Some(job) = session.refinement(quality) {
        session.accept_refined(job.run(&AtomicBool::new(false)));
    }
    let count = session.execute("view.scrub", &json!({})).unwrap()["steps"]
        .as_u64()
        .unwrap();
    let held = session.bodies_held().unwrap();
    println!(
        "== {name} ({count} features, budget {}, bodies held {} MB, {} let go of)",
        budget.map_or("default".into(), |b| format!("{} MB", b >> 20)),
        held.0 >> 20,
        held.1
    );
    let check = (count / 8).max(1);
    let forward: Vec<u64> = (1..=count).collect();
    pass(&mut session, "forward", &forward, check);
    let back: Vec<u64> = (1..=count).rev().collect();
    pass(&mut session, "back", &back, check);
}

#[test]
#[cfg_attr(debug_assertions, ignore = "timings are for release builds")]
fn stepping_through_the_bench_models() {
    for name in [
        "turbofan.steps.json",
        "engine-block.steps.json",
        "rotor.steps.json",
        "impeller-blades.steps.json",
    ] {
        model(name, None);
    }
    // A model whose bodies take more than the budget, as a big one does.
    model("engine-block.steps.json", Some(8 << 20));
    // Opened from its file, as in the window.
    walk("turbofan.3dcraft", None, opened("turbofan.steps.json"));
    walk("rotor.3dcraft", None, opened("rotor.steps.json"));
}
