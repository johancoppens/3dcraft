//! The performance benchmark of the gate (docs/testing.md, oracle
//! "work counts"): fixed models built command by command, edited, measured,
//! checked and meshed, with the **work** each took counted, not the time.
//! A count is the same on an idle machine and on one at load 55, so a
//! slowdown fails the gate and a busy machine never does.
//!
//! What is counted (each from a process-wide counter of its crate, read
//! before and after a model):
//!
//! - regeneration (`craft3d_regen::work`): regenerations, features rebuilt
//!   and reused, entries of what features are made from compared, sketches
//!   solved;
//! - the kernel (`craft3d_kernel::work`): booleans, pairs of faces a
//!   boolean intersected, the faces and edges its broad phase handed on to
//!   an exact test (which grow with the product of two counts where a
//!   broad phase is lost), patch trees of freeform surfaces built, blend
//!   cross-sections solved;
//! - mass properties (`craft3d_topo::mass_work`): faces integrated and the
//!   evaluations of the integrand that took;
//! - the tessellator (`craft3d_tess::tess_work`): bodies meshed, their
//!   faces, triangles.
//!
//! Each count may not exceed its baseline in `bench_models/baseline.txt`
//! by more than [`MARGIN`]. A count that grows past it is a slowdown (or
//! more work done on purpose); a count far below it is a gain to lock in.
//! To accept new counts, deliberately:
//!
//! ```sh
//! CRAFT3D_BENCH_UPDATE=1 cargo test -p craft3d-engine --test benchmark
//! ```
//!
//! and log it in the dev log as an expectation change (docs/testing.md ›
//! Changing an expectation), with the counts before and after and why.
//!
//! The CPU time of each model is printed beside the counts, for
//! information only: it never fails the test.
//!
//! The models are copies of showcase scripts (`target/work/*`, which are
//! not in the repository), pinned here so that the counts mean the same
//! thing from run to run:
//!
//! - `turbofan.steps.json`: the cutaway turbofan, whole;
//! - `engine-block.steps.json`: the engine block, whole;
//! - `impeller-blades.steps.json`: the impeller's first 82 steps (the
//!   hub with its chamfers and fillets, one lofted blade cut by the
//!   shroud, patterned six times and filleted; the whole wheel takes a
//!   minute in release, too long for the gate);
//! - `rotor.steps.json`: the showcase's wind turbine rotor (`docs/showcase`),
//!   blades lofted smoothly through several spline sections, the gate's
//!   loft through many sections (a loft made slower shows here);
//! - holes: a plate with [`HOLES`] holes cut one after another, command by
//!   command, generated here (as `examples/regen_rows.rs holes` does).
//!
//! Run one model, with the counts printed:
//! `BENCH_MODEL=turbofan cargo test -p craft3d-engine --test benchmark -- --nocapture`
//! (a model run alone can count fewer patch trees than in the full run,
//! which shares their shelf; only the full run is held to the baseline).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Instant;

/// How far a count may rise above its baseline before the test fails.
const MARGIN: f64 = 0.10;

/// Holes in the plate of the holes model.
const HOLES: usize = 250;

type Counts = BTreeMap<String, u64>;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/bench_models")
}

/// Every counter, as it stands now.
fn snapshot() -> Counts {
    let regen = craft3d_regen::work::work();
    let kernel = craft3d_kernel::work::work();
    let (faces_integrated, mass_evaluations) = craft3d_topo::mass_work();
    let (meshes, faces_meshed, triangles) = craft3d_tess::tess_work();
    [
        ("regenerations", regen.regenerations),
        ("features_rebuilt", regen.features_rebuilt),
        ("features_reused", regen.features_reused),
        ("context_entries", regen.context_entries),
        ("sketch_solves", regen.sketch_solves),
        ("booleans", kernel.booleans),
        ("boolean_face_pairs", kernel.face_pairs),
        ("boolean_face_candidates", kernel.face_candidates),
        ("boolean_edge_candidates", kernel.edge_candidates),
        ("patch_trees", kernel.patch_trees),
        ("blend_sections", kernel.blend_sections),
        ("mass_faces", faces_integrated),
        ("mass_evaluations", mass_evaluations),
        ("meshes", meshes),
        ("mesh_faces", faces_meshed),
        ("triangles", triangles),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_string(), value))
    .collect()
}

/// CPU time of the whole process (all its threads, also those that have
/// ended), from `/proc/self/stat`; `None` where there is no such file.
/// Clock ticks are taken as the usual 100 a second.
fn process_cpu_seconds() -> Option<f64> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    // Fields after the command name, which is in parentheses.
    let rest = &stat[stat.rfind(')')? + 2..];
    let fields: Vec<&str> = rest.split_whitespace().collect();
    // utime and stime are fields 14 and 15 of the whole line, 12 and 13
    // counted from the state (field 3).
    let utime: f64 = fields.get(11)?.parse().ok()?;
    let stime: f64 = fields.get(12)?.parse().ok()?;
    Some((utime + stime) / 100.0)
}

struct Model {
    name: &'static str,
    steps: Vec<(String, Value)>,
    /// Edits after the build, each a command and its parameters.
    edits: Vec<(String, Value)>,
}

fn steps_file(file: &str) -> Vec<(String, Value)> {
    let path = dir().join(file);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path:?}: {e}"))
}

fn edit(id: &str, params: Value) -> (String, Value) {
    (id.to_string(), params)
}

/// The plate with `count` holes, cut one after another (each cut from the
/// body the one before left), as an agent builds it command by command.
fn holes(count: usize) -> Model {
    let mut steps = vec![edit("variable.set", json!({"name": "r", "expr": "2 mm"}))];
    let columns = count.div_ceil(2).max(5);
    let length = 10.0 * columns as f64;
    steps.push(edit(
        "feature.block",
        json!({"size": [length, 40, 5], "name": "Plate"}),
    ));
    let mut last = "Plate".to_string();
    for hole in 0..count {
        let sketch = format!("S{hole}");
        steps.push(edit("sketch.create", json!({ "name": sketch })));
        let (x, y) = (
            5.0 + 10.0 * (hole / 2) as f64,
            10.0 + 20.0 * (hole % 2) as f64,
        );
        steps.push(edit(
            "sketch.circle",
            json!({"sketch": sketch, "center": [x, y], "radius": 2}),
        ));
        if hole + 1 == count {
            steps.push(edit(
                "sketch.constrain",
                json!({"sketch": sketch, "type": "diameter", "entities": ["e2"], "value": "2 * r"}),
            ));
        }
        let name = format!("H{hole}");
        steps.push(edit(
            "feature.extrude",
            json!({"sketch": sketch, "distance": 15, "symmetric": true, "cut": last, "name": name}),
        ));
        last = name;
    }
    Model {
        name: "holes",
        steps,
        edits: vec![
            edit("variable.set", json!({"name": "r", "expr": "1.5 mm"})),
            edit(
                "feature.edit",
                json!({"feature": "Plate", "size": [length + 10.0, 40, 5]}),
            ),
        ],
    }
}

fn models() -> Vec<Model> {
    vec![
        Model {
            name: "turbofan",
            steps: steps_file("turbofan.steps.json"),
            edits: vec![edit(
                "variable.set",
                json!({"name": "mixFingers", "expr": "14"}),
            )],
        },
        Model {
            name: "engine-block",
            steps: steps_file("engine-block.steps.json"),
            edits: vec![edit(
                "variable.set",
                json!({"name": "chSmall", "expr": "0.6 mm"}),
            )],
        },
        Model {
            name: "impeller",
            steps: steps_file("impeller-blades.steps.json"),
            edits: vec![edit(
                "variable.set",
                json!({"name": "filletSeat", "expr": "0.2 mm"}),
            )],
        },
        Model {
            name: "rotor",
            steps: steps_file("rotor.steps.json"),
            edits: Vec::new(),
        },
        holes(HOLES),
    ]
}

fn run(session: &mut Session, id: &str, params: &Value, model: &str) -> Value {
    session
        .execute(id, params)
        .unwrap_or_else(|e| panic!("{model}: {id} {params}: {e}"))
}

/// Builds, edits, measures, checks and meshes `model`; the work it took
/// and the CPU seconds.
fn measure(model: &Model) -> (Counts, Option<f64>) {
    let before = snapshot();
    let cpu = process_cpu_seconds();
    let mut session = Session::new(Workspace::unrestricted());
    run(
        &mut session,
        "file.new",
        &json!({"units": "mm"}),
        model.name,
    );
    for (id, params) in model.steps.iter().chain(&model.edits) {
        run(&mut session, id, params, model.name);
    }
    // What the model cannot have (a body that cannot be measured, a
    // feature that failed) is counted, not a failure here: the count is
    // held exactly, so one more refusal fails the test, and one fewer is
    // a gain to lock in.
    let mut refusals = 0u64;
    if let Err(e) = session.execute("document.measure", &json!({})) {
        println!("{}: document.measure: {e}", model.name);
        refusals += 1;
    }
    let check = run(&mut session, "document.check", &json!({}), model.name);
    let problems = check
        .get("problems")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for problem in &problems {
        println!("{}: document.check: {problem}", model.name);
    }
    let tmp = scratch();
    let stl = tmp.join(format!("{}.stl", model.name));
    if let Err(e) = session.execute("file.export", &json!({"path": stl.to_string_lossy()})) {
        println!("{}: file.export: {e}", model.name);
        refusals += 1;
    }
    let after = snapshot();
    let mut counts: Counts = after
        .iter()
        .map(|(name, value)| {
            let was = before.get(name).copied().unwrap_or(0);
            (name.clone(), value.saturating_sub(was))
        })
        .collect();
    counts.insert("refusals".to_string(), refusals);
    counts.insert("check_problems".to_string(), problems.len() as u64);
    counts.insert(
        "commands".to_string(),
        (model.steps.len() + model.edits.len()) as u64,
    );
    let spent = cpu.zip(process_cpu_seconds()).map(|(a, b)| b - a);
    (counts, spent)
}

/// Where the meshes and the counts of the last run go:
/// `$CARGO_TARGET_TMPDIR/benchmark`, as the test is run (by cargo or by
/// `cargo xtask ci`), else as it was built.
fn scratch() -> PathBuf {
    let tmp = std::env::var_os("CARGO_TARGET_TMPDIR")
        .map_or_else(|| PathBuf::from(env!("CARGO_TARGET_TMPDIR")), PathBuf::from)
        .join("benchmark");
    std::fs::create_dir_all(&tmp).unwrap();
    tmp
}

fn baseline_path() -> PathBuf {
    dir().join("baseline.txt")
}

/// `model counter value` per line; `#` starts a comment.
fn read_baseline() -> BTreeMap<(String, String), u64> {
    let text = std::fs::read_to_string(baseline_path()).unwrap_or_default();
    text.lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let model = words.next()?.to_string();
            let counter = words.next()?.to_string();
            let value = words.next()?.parse().ok()?;
            Some(((model, counter), value))
        })
        .collect()
}

fn write_baseline(all: &[(&str, Counts)]) {
    let mut text = String::from(
        "# Work counts of crates/engine/tests/benchmark.rs: model, counter, count.\n\
         # Written by CRAFT3D_BENCH_UPDATE=1 cargo test -p craft3d-engine --test benchmark;\n\
         # every change is an expectation change, logged in the dev log.\n",
    );
    for (model, counts) in all {
        for (counter, value) in counts {
            let _ = writeln!(text, "{model} {counter} {value}");
        }
    }
    std::fs::write(baseline_path(), text).unwrap();
}

/// Counts held exactly: what the models cannot have.
const EXACT: [&str; 2] = ["refusals", "check_problems"];

/// The most `counter` may be: its baseline and [`MARGIN`] more, rounded
/// up; the baseline itself for the [`EXACT`] ones.
fn allowed(counter: &str, baseline: u64) -> u64 {
    if EXACT.contains(&counter) {
        baseline
    } else {
        (baseline as f64 * (1.0 + MARGIN)).ceil() as u64
    }
}

#[test]
fn work_stays_within_the_baseline() {
    let only = std::env::var("BENCH_MODEL").ok();
    let update = std::env::var_os("CRAFT3D_BENCH_UPDATE").is_some();
    assert!(
        !(update && only.is_some()),
        "update the baseline from the full run, without BENCH_MODEL"
    );
    let baseline = read_baseline();
    let mut all: Vec<(&str, Counts)> = Vec::new();
    let mut report = String::new();
    let mut over = Vec::new();
    let mut under = Vec::new();
    for model in models() {
        if only.as_deref().is_some_and(|name| name != model.name) {
            continue;
        }
        let started = Instant::now();
        let (counts, cpu) = measure(&model);
        let _ = writeln!(
            report,
            "{}: {:.1} s wall, {} CPU (information only)",
            model.name,
            started.elapsed().as_secs_f64(),
            cpu.map_or("unknown".to_string(), |s| format!("{s:.1} s"))
        );
        for (counter, &value) in &counts {
            let base = baseline
                .get(&(model.name.to_string(), counter.clone()))
                .copied();
            let verdict = match base {
                None => {
                    over.push(format!("{} {counter}: {value}, no baseline", model.name));
                    "NO BASELINE".to_string()
                }
                Some(base) if value > allowed(counter, base) => {
                    over.push(format!(
                        "{} {counter}: {value}, baseline {base}, at most {}",
                        model.name,
                        allowed(counter, base)
                    ));
                    "OVER".to_string()
                }
                Some(base)
                    if (value as f64) < base as f64 * (1.0 - MARGIN)
                        || (EXACT.contains(&counter.as_str()) && value < base) =>
                {
                    under.push(format!("{} {counter}: {value} < {base}", model.name));
                    "below".to_string()
                }
                Some(_) => String::new(),
            };
            let _ = writeln!(
                report,
                "  {counter:<20} {value:>12}  baseline {:>12}  {verdict}",
                base.map_or("-".to_string(), |b| b.to_string())
            );
        }
        all.push((model.name, counts));
    }
    println!("{report}");
    // For `cargo xtask health`: the counts of the last run.
    let summary: String = all
        .iter()
        .flat_map(|(model, counts)| {
            counts
                .iter()
                .map(move |(counter, value)| format!("{model} {counter} {value}\n"))
        })
        .collect();
    let _ = std::fs::write(scratch().join("last-counts.txt"), summary);
    if update {
        write_baseline(&all);
        println!(
            "baseline written to {}; log the change in the dev log as an expectation change",
            baseline_path().display()
        );
        return;
    }
    if !under.is_empty() {
        println!(
            "less work than the baseline by more than {:.0} % (a gain: lock it in with \
             CRAFT3D_BENCH_UPDATE=1):\n  {}",
            MARGIN * 100.0,
            under.join("\n  ")
        );
    }
    assert!(
        over.is_empty(),
        "more work than the baseline allows ({:.0} % margin):\n  {}\n\n{report}\n\
         A slowdown: find what does more work. If the extra work is meant, update the baseline \
         deliberately with\n  CRAFT3D_BENCH_UPDATE=1 cargo test -p craft3d-engine --test benchmark\n\
         and log it in the dev log as an expectation change (docs/testing.md).",
        MARGIN * 100.0,
        over.join("\n  ")
    );
}
