//! The memory `motion.check` holds over an assembly's motion
//! (docs/design/motion.md › *Checks over the motion*): each moment places
//! fresh copies of the bodies, and what the checks remember between
//! moments must not keep them alive. Measured as the placed bodies alive
//! during and after runs of 129 and 513 moments of two wheels of 258 faces
//! each (the process's peak memory before 2026-11-06, see the test). In
//! its own test binary, so the process holds nothing else.

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

#[test]
fn more_moments_of_a_motion_check_hold_no_more_bodies() {
    let dir = workdir("motion-memory", "wheels");
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
    let check = |s: &mut craft3d_engine::Session, samples: u64| {
        let out = run(s, "motion.check", json!({ "samples": samples }));
        assert_eq!(out["ok"], true, "{out}");
    };
    // What is measured (since 2026-11-06): the placed bodies alive, the
    // moments' copies of the two wheels, counted by the engine at every
    // placement (`track_placed_bodies`): the most alive at once during a
    // run and those alive after it. A run that keeps its moments' bodies
    // holds two more for every moment; one that does not holds those of
    // the moments in flight, a number that does not grow with the
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
    let _ = craft3d_engine::placed_high_water();
    check(&mut s, 129);
    let at_129 = craft3d_engine::placed_high_water();
    let after_129 = craft3d_engine::live_placed_bodies();
    let few = peak_kib();
    check(&mut s, 513);
    let at_513 = craft3d_engine::placed_high_water();
    let after_513 = craft3d_engine::live_placed_bodies();
    let many = peak_kib();
    craft3d_engine::track_placed_bodies(false);
    eprintln!(
        "motion.check memory: placed bodies alive at most {at_129} in a run of 129 moments, {at_513} in one of 513; {after_129} and {after_513} after them; peak {} MiB and {} MiB",
        few.unwrap_or(0) / 1024,
        many.unwrap_or(0) / 1024,
    );
    // Two wheels for each moment in flight: at most a group of eight
    // being measured, eight waiting and the two placements a sweep keeps,
    // whatever the number of moments. Kept moments would be 2 x 129 and
    // 2 x 513.
    assert!(
        at_129 <= 2 * (8 + 8 + 4),
        "{at_129} placed bodies alive at once in 129 moments"
    );
    assert!(
        at_513 <= at_129 + 2,
        "{at_513} alive at once in 513 moments, {at_129} in 129"
    );
    assert!(
        after_513 <= after_129 && after_129 <= 4,
        "{after_129} then {after_513} alive after the runs"
    );
}
