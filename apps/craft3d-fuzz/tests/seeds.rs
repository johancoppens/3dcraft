//! Random multi-step designs in the gate (docs/testing.md › Random
//! multi-step designs): a fixed set of seeds, run in this process, held to
//! every oracle. The large run, each seed in its own process, with a
//! timeout, a memory cap and the shrinker, is `cargo xtask fuzz`.
//!
//! The seeds are the first ones that passed when the oracle was built;
//! a seed that fails later is a regression. A seed found failing by the
//! large run goes into the findings (`target/fuzz`), not into this list.

use craft3d_fuzz::{Deep, Design, Generator, Options, Setup, run_design, run_seed};
use serde_json::json;
use std::path::PathBuf;

const SEEDS: [u64; 20] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
];

fn options() -> Options {
    Options {
        deep: Deep::EveryGroup,
        identities: true,
        determinism: true,
        scratch: PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("fuzz-seeds"),
    }
}

fn setup(seed: u64) -> Setup {
    Setup {
        seed,
        scale: Generator::scale_of(seed),
    }
}

#[test]
fn fixed_seeds_neither_crash_nor_go_wrong() {
    let mut groups = 0;
    let mut accepted = 0;
    let mut booleans = 0;
    for seed in SEEDS {
        let (design, runner) = run_seed(setup(seed), options(), &mut |_| {}).expect("runs");
        let report = &runner.report;
        assert!(
            report.finding.is_none(),
            "seed {seed}: {:?}\nreproducer (craft3d-fuzz check FILE):\n{}",
            report.finding,
            design.to_json()
        );
        groups += report.groups;
        accepted += report.accepted;
        booleans += report.booleans;
    }
    // The seeds really build something, and booleans among it.
    assert!(groups >= 100, "{groups} groups");
    assert!(accepted * 2 >= groups, "{accepted} of {groups} accepted");
    assert!(booleans >= 10, "{booleans} booleans");
}

#[test]
fn a_seed_gives_the_same_design_and_verdict_every_time() {
    for seed in [3, 11] {
        let (a, ra) = run_seed(setup(seed), options(), &mut |_| {}).expect("runs");
        let (b, rb) = run_seed(setup(seed), options(), &mut |_| {}).expect("runs");
        assert_eq!(a, b);
        assert_eq!(ra.report, rb.report);
        assert_eq!(ra.script, rb.script);
        // The design read back from its file runs the same.
        let again = Design::from_json(&a.to_json()).expect("reads");
        let rc = run_design(&again, options()).expect("runs");
        assert_eq!(rc.report.accepted, ra.report.accepted);
        assert_eq!(rc.script, ra.script);
    }
}

#[test]
fn hostile_designs_are_refused_not_crashed() {
    for bad in [
        json!(null),
        json!({"scale": 0, "groups": []}),
        json!({"scale": 1, "groups": 5}),
        json!({"scale": 1, "groups": [{"commands": [{"params": {}}]}]}),
    ] {
        assert!(Design::from_json(&bad).is_err(), "{bad}");
    }
    let design = Design::from_json(&json!({
        "seed": 1, "scale": 1,
        "groups": [
            {"step": "x", "commands": [{"command": "no.such", "params": {}}]},
            {"step": "x", "commands": [{"command": "feature.block", "params": {"size": [-1, "x", 1e308]}}]},
            {"step": "x", "commands": [{"command": "feature.boolean", "params": {"op": "union", "target": "{N9}", "tool": "{"}}]},
            {"step": "x", "commands": [{"command": "feature.block", "params": {"size": [1, 1, 1], "name": "N3"}}]},
            {"step": "x", "commands": [{"command": "feature.fillet", "params": {"edges": [["{N3}:face(0)", "{N3}:face(2)"]], "radius": 0.25}}]},
        ],
    }))
    .expect("reads");
    let runner = run_design(&design, options()).expect("runs");
    assert_eq!(runner.report.finding, None);
    assert_eq!(runner.report.refused.len(), 3);
    assert_eq!(runner.report.accepted, 2);
}
