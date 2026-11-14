//! The memory `motion.check` holds over an assembly's motion
//! (docs/design/motion.md › *Checks over the motion*): each moment places
//! fresh copies of the bodies, and what the checks remember between
//! moments must not keep them alive. Measured as the placed bodies alive
//! during and after runs of 129 and 513 moments of two wheels of 258 faces
//! each with the sample workers, and of 33 and 513 moments on one thread
//! (the process's peak memory before 2026-11-06, see the test). In its own
//! test binary, so the process holds nothing else (the probe below is
//! never run beside the test).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;

use common::plain::run;
use common::{session_in, workdir};
use serde_json::{Value, json};

/// The process's peak resident memory in KiB (`VmHWM`), where the system
/// says it.
fn peak_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find_map(|l| l.strip_prefix("VmHWM:"))
        .and_then(|rest| rest.trim().trim_end_matches("kB").trim().parse().ok())
}

/// A wheel: a star of 256 points extruded 4 thick (258 faces), with datum
/// axes `Hub` (its centre) and `Off` (40 to its left).
fn wheel_part(s: &mut craft3d_engine::Session) {
    run(s, "file.new", json!({}));
    run(s, "sketch.create", json!({ "name": "Profile" }));
    let points: Vec<Value> = (0..256)
        .map(|k| {
            let a = std::f64::consts::TAU * f64::from(k) / 256.0;
            let r = if k % 2 == 0 { 12.0 } else { 10.5 };
            json!([r * a.pcos(), r * a.psin()])
        })
        .collect();
    run(
        s,
        "sketch.polyline",
        json!({ "sketch": "Profile", "closed": true, "points": points }),
    );
    run(
        s,
        "feature.extrude",
        json!({ "sketch": "Profile", "distance": 4, "name": "Wheel" }),
    );
    run(
        s,
        "datum.axis",
        json!({ "name": "Hub", "through": [[0, 0, 0], [0, 0, 1]] }),
    );
    run(
        s,
        "datum.axis",
        json!({ "name": "Off", "through": [[-40, 0, 0], [-40, 0, 1]] }),
    );
    run(s, "file.save", json!({ "path": "wheel.3dcraft" }));
    run(s, "file.close", json!({}));
}

/// The assembly: `Spin`, a wheel turned once about its hub as `t` goes
/// from 0 to 1, and `Still`, a wheel standing 40 beside it, with an
/// assertion that they stay apart and one on the volume of `Spin`.
fn wheels(name: &str) -> craft3d_engine::Session {
    let dir = workdir("motion-memory", name);
    let mut s = session_in(&dir);
    wheel_part(&mut s);
    run(&mut s, "assembly.new", json!({}));
    for name in ["Spin", "Still"] {
        run(
            &mut s,
            "instance.add",
            json!({ "source": "wheel.3dcraft", "name": name, "body": "Wheel" }),
        );
    }
    run(
        &mut s,
        "mate.add",
        json!({ "kind": "revolute", "a": "world/z", "b": "Spin/Hub", "name": "Axle", "flip": true }),
    );
    run(
        &mut s,
        "mate.add",
        json!({ "kind": "fixed", "a": "world/z", "b": "Still/Off", "name": "Stand", "flip": true }),
    );
    run(&mut s, "variable.set", json!({ "name": "t", "expr": "0" }));
    run(
        &mut s,
        "variable.set",
        json!({ "name": "Axle_angle", "expr": "360 deg * t" }),
    );
    run(
        &mut s,
        "assert.add",
        json!({ "name": "apart", "measure": "overlap", "is": false }),
    );
    run(
        &mut s,
        "assert.add",
        json!({ "name": "solid", "measure": "volume", "of": "Spin", "min": "1 mm^3" }),
    );
    s
}

fn check(s: &mut craft3d_engine::Session, samples: u64) {
    let out = run(s, "motion.check", json!({ "samples": samples }));
    assert_eq!(out["ok"], true, "{out}");
}

/// The most placed bodies alive at once in a run of `samples` moments.
fn high_water(s: &mut craft3d_engine::Session, samples: u64) -> usize {
    let _ = craft3d_engine::placed_high_water();
    check(s, samples);
    craft3d_engine::placed_high_water()
}

#[test]
fn more_moments_of_a_motion_check_hold_no_more_bodies() {
    let mut s = wheels("wheels");
    // What is measured (since 2026-11-06): the placed bodies alive, the
    // moments' copies of the two wheels, counted by the engine at every
    // placement (`track_placed_bodies`): the most alive at once during a
    // run and those alive after it. A run that keeps its moments' bodies
    // holds one more for every moment at least; one that does not holds
    // those of the moments in flight, a number that does not grow with the
    // moments. Until 2026-11-05 this test held the process's peak resident
    // memory instead, which stood for the bodies kept while one thread
    // measured the samples. Measured side by side on up to eight threads,
    // the peak measures the allocator's arenas as well: +46 MiB from a run
    // of 9 moments to one of 129 and +5 to +11 MiB from 129 to 513 on a
    // 14-thread laptop, +72 MiB from 129 to 513 in the Gate on the Strix,
    // but +3 MiB from 129 to 513 with MALLOC_ARENA_MAX=2, while the placed
    // bodies alive after each run stayed 2 (the probe of 2026-11-06 in the
    // dev log). The count is what the peak stood for, measured directly;
    // the peak is printed.
    craft3d_engine::track_placed_bodies(true);
    check(&mut s, 9);
    check(&mut s, 129);
    // With the workers, as a `motion.check` runs when the pool is free.
    let at_129 = high_water(&mut s, 129);
    let after_129 = craft3d_engine::live_placed_bodies();
    let few = peak_kib();
    let at_513 = high_water(&mut s, 513);
    let after_513 = craft3d_engine::live_placed_bodies();
    let many = peak_kib();
    // On one thread, as it runs when the pool is busy: the pool held here.
    let held = whole_pool();
    let alone_33 = high_water(&mut s, 33);
    let alone_513 = high_water(&mut s, 513);
    drop(held);
    craft3d_engine::track_placed_bodies(false);
    eprintln!(
        "motion.check memory: placed bodies alive at most {at_129} in a run of 129 moments, {at_513} in one of 513; {after_129} and {after_513} after them; on one thread {alone_33} in 33 moments, {alone_513} in 513; peak {} MiB and {} MiB",
        few.unwrap_or(0) / 1024,
        many.unwrap_or(0) / 1024,
    );
    // With the workers: the moments in flight, whatever their number.
    // The moment being placed, at most one waiting per worker (the
    // channel holds as many as there are workers, seven at most), one
    // being measured per worker and the one it measured before, until the
    // new one takes its place, and the two placements the sweep keeps;
    // each moment one new body (the turning wheel: the standing wheel's
    // copy is the same in every moment, `PlacedCopies`), beside the
    // document's own two. Under 2 x (8 + 8 + 4) = 40, the bound since
    // 2026-11-06, for both runs; kept moments would be over 129 and over
    // 513. Which moments are in flight at once is the scheduler's doing,
    // so the most of a run is a sample: until 2026-11-14 this test asked
    // the run of 513 for no more than the run of 129 plus 2, and failed
    // with 22 against 19 in a gate on a loaded build server; 120 rounds
    // of both there gave 18 to 21 for 129 moments and 19 to 22 for 513,
    // the old comparison failing in one round, and a run of 129 above its
    // run of 513 in five (`probe_high_water_marks`; the dev log of
    // 2026-11-14).
    assert!(
        at_129 <= 2 * (8 + 8 + 4) && at_513 <= 2 * (8 + 8 + 4),
        "{at_129} placed bodies alive at once in 129 moments, {at_513} in 513"
    );
    // Not growing with the moments, asked where the schedule cannot
    // decide it: on one thread each moment is placed, measured and let go
    // of in turn, so the most alive at once is one number for any run
    // (6 here, in each of 300 runs on the build server under load).
    // Whatever keeps a moment's bodies for the rest of a run, one moment
    // in 256 or more often, keeps at least two in the run of 513 and at
    // most one in the run of 33; the comparison it replaces asked for 3
    // more in 513 moments than in 129. Tried (2026-11-14): one moment in
    // 50 kept until the run ends gave 7 here in 33 moments and 16 in 513,
    // and 21 and 28 with the workers, under their bound.
    assert_eq!(
        alone_513, alone_33,
        "on one thread {alone_513} alive at once in 513 moments, {alone_33} in 33"
    );
    assert!(
        after_513 <= after_129 && after_129 <= 4,
        "{after_129} then {after_513} alive after the runs"
    );
}

/// All of the process's pool of threads for heavy work, so that a
/// `motion.check` started meanwhile gets no workers and measures its
/// moments one after the other on the calling thread.
fn whole_pool() -> Vec<craft3d_math::pool::Lease> {
    let all = craft3d_math::pool::SHARED_THREADS;
    let (mut held, mut got) = (Vec::new(), 0);
    let start = std::time::Instant::now();
    while got < all && start.elapsed() < std::time::Duration::from_secs(30) {
        let lease = craft3d_math::pool::lease(all - got);
        got += lease.threads();
        held.push(lease);
        if got < all {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    assert_eq!(got, all, "the pool's threads were not all free within 30 s");
    held
}

/// PROBE (run by hand or on the build server, `-- --ignored --nocapture`;
/// never beside the test above, which shares the process's counts): the
/// high-water marks of runs of 129 and 513 moments with the workers, and
/// of 33 and 513 moments on one thread (the pool held here), round after
/// round (`CRAFT3D_PROBE_ROUNDS`, 40), every other round beside busy
/// threads (`CRAFT3D_PROBE_BUSY`, 16).
#[test]
#[ignore = "probe: the distribution of the high-water marks"]
fn probe_high_water_marks() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let mut s = wheels("probe");
    craft3d_engine::track_placed_bodies(true);
    check(&mut s, 9);
    let mark = |s: &mut craft3d_engine::Session, samples: u64| {
        let _ = craft3d_engine::placed_high_water();
        let start = std::time::Instant::now();
        check(s, samples);
        (
            craft3d_engine::placed_high_water(),
            start.elapsed().as_secs_f64(),
        )
    };
    let number = |name: &str, default: usize| {
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    };
    let (rounds, load) = (
        number("CRAFT3D_PROBE_ROUNDS", 40),
        number("CRAFT3D_PROBE_BUSY", 16),
    );
    let mut rows = Vec::new();
    for round in 0..rounds {
        let busy = if round % 2 == 1 { load } else { 0 };
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let spinners: Vec<_> = (0..busy)
            .map(|_| {
                let stop = std::sync::Arc::clone(&stop);
                std::thread::spawn(move || {
                    let mut x = 0_u64;
                    while !stop.load(Ordering::Relaxed) {
                        x = std::hint::black_box(
                            x.wrapping_mul(6364136223846793005).wrapping_add(1),
                        );
                    }
                })
            })
            .collect();
        let (p129, t129) = mark(&mut s, 129);
        let (p513, t513) = mark(&mut s, 513);
        let held = whole_pool();
        let (s33, _) = mark(&mut s, 33);
        let (s513, ts513) = mark(&mut s, 513);
        drop(held);
        stop.store(true, Ordering::Relaxed);
        for spinner in spinners {
            spinner.join().unwrap();
        }
        let after = craft3d_engine::live_placed_bodies();
        eprintln!(
            "PROBE round {round} busy {busy}: parallel 129 -> {p129} ({t129:.1} s), 513 -> {p513} ({t513:.1} s); one thread 33 -> {s33}, 513 -> {s513} ({ts513:.1} s); alive after {after}"
        );
        rows.push((p129, p513, s33, s513));
    }
    craft3d_engine::track_placed_bodies(false);
    let most = |f: fn(&(usize, usize, usize, usize)) -> usize| rows.iter().map(f).max().unwrap();
    let spread = |f: fn(&(usize, usize, usize, usize)) -> usize| {
        let mut counts = std::collections::BTreeMap::new();
        for row in &rows {
            *counts.entry(f(row)).or_insert(0_usize) += 1;
        }
        format!("{counts:?}")
    };
    eprintln!(
        "PROBE most: parallel 129 {}, 513 {}; one thread 33 {}, 513 {}; 513 over 129 + 2 in {} of {} rounds, 129 over 513 in {}",
        most(|r| r.0),
        most(|r| r.1),
        most(|r| r.2),
        most(|r| r.3),
        rows.iter().filter(|r| r.1 > r.0 + 2).count(),
        rows.len(),
        rows.iter().filter(|r| r.0 > r.1).count(),
    );
    eprintln!(
        "PROBE spread (value: rounds): parallel 129 {}, 513 {}; one thread 33 {}, 513 {}",
        spread(|r| r.0),
        spread(|r| r.1),
        spread(|r| r.2),
        spread(|r| r.3),
    );
}
