//! The memory `motion.check` holds over an assembly's motion
//! (docs/design/motion.md › *Checks over the motion*): each moment places
//! fresh copies of the bodies, and what the checks remember between
//! moments must not keep them alive. Measured as the process's peak
//! resident memory over 9 and then 129 moments of two wheels of 258 faces
//! each: the difference is what more moments cost. In its own test binary,
//! so the process holds nothing else.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

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
            json!([r * a.cos(), r * a.sin()])
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
    check(&mut s, 9);
    let few = peak_kib();
    check(&mut s, 129);
    let many = peak_kib();
    let (Some(few), Some(many)) = (few, many) else {
        eprintln!("motion.check memory: no VmHWM on this system; not measured");
        return;
    };
    let grown = many.saturating_sub(few);
    eprintln!(
        "motion.check memory: peak {} MiB after 9 moments, {} MiB after 129 more (+{} MiB)",
        few / 1024,
        many / 1024,
        grown / 1024
    );
    // Before the answers' keys held their bodies weakly (2026-10-30) the
    // checks kept every moment's two wheels alive, about half a megabyte a
    // moment: 129 moments raised the peak by 60 MiB.
    assert!(
        grown < 24 * 1024,
        "129 moments raised the peak by {} MiB over 9",
        grown / 1024
    );
}
