//! The memory of a check over the range (`document.sample`, the command
//! behind `craft3d-cli check-range`; docs/design/verification.md ›
//! *Proof over the range*). Each point is built and checked in a session
//! of its own, but what the kernel keeps between operations (the
//! intersections of surfaces, the projections onto them, the meshes of
//! faces, the bodies that passed their check) is kept per process: a
//! check over many points of a freeform part (the spur-gear Jig with a
//! copy meshing with it and an overlap assertion between them, 2026-10-24)
//! grew from 134 MB at one point to 1.2 GB at thirteen, and was killed for
//! its memory on a laptop. What the points leave behind is now let go of:
//! after each point where the host asks for that
//! (`Session::set_lean_sampling`, as check-range does), so a run holds
//! what one point needs; else at the end of the run (a window's).
//!
//! The caches are one per process, so this file holds its tests apart
//! from the others.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};
use std::path::PathBuf;

/// The spur-gear Jig (`3dcraft-jigs/jigs/spur-gear/build.json`, its
/// fifteen parameters published) with a copy of the gear moved to mesh
/// with it and an assertion that they do not overlap.
fn gear_pair() -> Session {
    let text = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/sample_memory/gear-pair.steps.json"),
    )
    .unwrap();
    let steps: Vec<(String, Value)> = serde_json::from_str(&text).unwrap();
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    for (id, params) in steps {
        run(&mut s, &id, params);
    }
    s
}

/// The resident memory of this process and its peak, in MB (Linux only).
fn resident_mb() -> Option<(f64, f64)> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let field = |key: &str| -> Option<f64> {
        let line = status.lines().find(|l| l.starts_with(key))?;
        let kb: f64 = line.split_whitespace().nth(1)?.parse().ok()?;
        Some(kb / 1024.0)
    };
    Some((field("VmRSS:")?, field("VmHWM:")?))
}

/// What the kernel keeps per process: intersections and the projectors
/// and pieces of surfaces, projections, meshes of faces, trees of
/// freeform surfaces.
fn kept() -> [usize; 4] {
    [
        craft3d_geom::intersect::kept(),
        craft3d_geom::kept_projections().0,
        craft3d_tess::kept_meshes().0,
        craft3d_kernel::shelf_size().0,
    ]
}

#[test]
fn a_check_over_the_range_leaves_nothing_the_kernel_kept_behind() {
    // As a window runs it: let go of at the end of the run.
    let mut s = gear_pair();
    let checked = run(&mut s, "document.check", json!({}));
    // Building and checking the pair filled what the kernel keeps.
    let before = kept();
    assert!(before[..3].iter().sum::<usize>() > 0, "{before:?}");
    let result = run(
        &mut s,
        "document.sample",
        json!({"samples": 1, "seed": 2, "corners": false, "choices": false}),
    );
    assert_eq!(result["counts"]["points"], 2, "{result}");
    // The intersections, projections and meshes of the points are gone,
    // and so are those of the open document (they are worked out again,
    // the same, when it needs them); of the trees of freeform surfaces
    // only those of its bodies stay (`Session::release_trees_now`).
    let after = kept();
    assert_eq!(after[..3], [0, 0, 0], "{after:?}");
    assert!(after[3] <= before[3], "{after:?} {before:?}");
    // What is worked out again is what was kept: the open document checks
    // as it did.
    assert_eq!(run(&mut s, "document.check", json!({})), checked);
}

/// The setting `craft3d-cli check-range` runs with (`check_range.rs`,
/// `in_few_arenas`): the C library's allocator keeps two arenas.
const ARENAS: &str = "2";

/// The resident memory over twenty runs of a default and one random point
/// each, every run at another seed, with the allocator as
/// `craft3d-cli check-range` sets it: it must level off, not grow with the
/// points. Measured on the Strix (32 threads, 2026-10-26), in MB after
/// runs 1, 5, 10, 15 and 20: before, 204, 732, 1215, 1634, 1995; with
/// what a point kept let go of, 212, 605, 816, 946, 1140 (the allocator's
/// arenas, one per thread, each kept at its peak); with that and two
/// arenas 150, 395, 344, 455, 482 (each point at its own peak, no
/// growth). Heavy (a minute or two of freeform booleans): run on the
/// Strix with `--ignored --nocapture`.
#[test]
#[ignore = "heavy: a freeform gear pair over 40 points; run on the Strix"]
fn the_memory_of_a_long_check_levels_off() {
    // The allocator reads its setting when the process starts: run this
    // test again in a process that has it.
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    if std::env::var_os("MALLOC_ARENA_MAX").is_none() {
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "the_memory_of_a_long_check_levels_off",
                "--exact",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("MALLOC_ARENA_MAX", ARENAS)
            .output()
            .unwrap();
        println!("{}", String::from_utf8_lossy(&out.stdout));
        println!("{}", String::from_utf8_lossy(&out.stderr));
        assert!(out.status.success());
        return;
    }
    let mut s = gear_pair();
    // As check-range runs: what the kernel keeps goes after every point.
    s.set_lean_sampling(true);
    println!("threads: {}", craft3d_math::available_threads());
    let mut seen = Vec::new();
    for seed in 1..=20_u64 {
        let started = std::time::Instant::now();
        let result = run(
            &mut s,
            "document.sample",
            json!({"samples": 1, "seed": seed, "corners": false, "choices": false, "timeLimitMs": null}),
        );
        assert_eq!(result["counts"]["points"], 2, "{result}");
        let (rss, peak) = resident_mb().unwrap_or((0.0, 0.0));
        println!(
            "seed {seed:2}: {:.1} s, resident {rss:.0} MB, peak {peak:.0} MB, passed {}, failed {}; kept: {:?}",
            started.elapsed().as_secs_f64(),
            result["passed"],
            result["failed"],
            kept(),
        );
        seen.push(rss);
    }
    println!("resident after each run (MB): {seen:.0?}");
    if seen.iter().all(|mb| *mb > 0.0) {
        // Levelled off: the last ten runs hold little more than the
        // first ten did at their highest (482 against 395; before, 1995
        // against 1215, and 1140 against 816 with only the caches let go).
        let early = seen[..10].iter().copied().fold(0.0, f64::max);
        let last = seen[19];
        assert!(
            last <= 1.3 * early,
            "the memory still grows with the points: {seen:.0?}"
        );
    }
}
