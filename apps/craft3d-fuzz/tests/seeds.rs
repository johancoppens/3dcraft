//! Random multi-step designs in the gate (docs/testing.md › Random
//! multi-step designs): a fixed set of seeds, run in this process, held to
//! every oracle. The large run, each seed in its own process, with a
//! timeout, a memory cap and the shrinker, is `cargo xtask fuzz`.
//!
//! The seeds are the first ones that passed when the oracle was built;
//! a seed that fails later is a regression. A seed found failing by the
//! large run goes into the findings (`target/fuzz`), not into this list.
//!
//! The design a seed gives changes when the generator draws differently
//! (an expectation change, 2026-10-19): since that date a circular
//! pattern draws one more chance (a screw, `step` along the axis), so
//! seeds 3, 10, 12, 13 and 16 give other designs than before that date
//! (12, 13 and 16 get a screw); all pass. A later failure of one of them
//! is a regression of the new design, not of the old.
//!
//! Since 2026-11-20 a group can be a plane cut, drawn from a stream of its
//! own: a seed without a cut gives the design it gave before, one with a
//! cut gives the same up to the cut (*the cut step takes nothing from the
//! other steps' stream*, below, holds that). Of the first twenty seeds 3,
//! 5, 10, 12, 17, 18 and 19 get one (11 cut groups, 6 built, the others
//! leave nothing) and give other designs from it on; all pass (the build
//! server's fuzz run 44822853676 of f377b7e4, seeds 0 to 4725). With them
//! the twenty seeds ran 130 groups, 105 accepted, 10 booleans, 8 with
//! their identities: the floor of 10 booleans exactly.
//!
//! So on 2026-11-21 the seeds became 0 to 24 (an expectation change in
//! the gate's favour: more designs held to every oracle, the old twenty
//! among them): 162 groups, 133 accepted, 13 booleans, 11 with their
//! identities, 16 cut groups (seeds 22, 23 and 24 add three), 9 built, 3
//! held to their two sides (measured by the review on the build server,
//! Check 44834052527, no finding in any of the 25).

use craft3d_fuzz::{
    Deep, Design, Generator, Options, Setup, Steps, Tally, run_design, run_generated, run_seed,
};
use serde_json::json;
use std::path::PathBuf;

const SEEDS: [u64; 25] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
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
    let mut identities = 0;
    let mut cuts = Steps::default();
    let mut sides = 0;
    let mut below = Tally::default();
    let (mut changed, mut unchanged) = (0, 0);
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
        identities += report.identities_checked;
        if let Some(n) = report.steps.get("planeCut") {
            cuts.groups += n.groups;
            cuts.accepted += n.accepted;
        }
        sides += report.cut_sides.checked;
        below.checked += report.cut_below.checked;
        below.skipped += report.cut_below.skipped;
        changed += report.cuts_changed;
        unchanged += report.cuts_unchanged;
    }
    println!(
        "{groups} groups, {accepted} accepted, {booleans} booleans, {identities} with identities; \
         cuts {cuts:?}, {changed} changed, {unchanged} unchanged, {below:?} held to measure.below, \
         {sides} to their two sides"
    );
    // The gate builds plane cuts too, holds every built one to
    // measure.below and one by one plane to its two sides (16 groups, 9
    // built, 3 sides on 2026-11-21).
    assert!(
        cuts.groups >= 12 && cuts.accepted >= 6 && sides >= 2,
        "{cuts:?}, {sides} cuts by one plane held to their sides"
    );
    assert_eq!(
        changed + unchanged,
        cuts.accepted,
        "every built cut counted"
    );
    assert!(
        below.checked >= 6 && below.checked + below.skipped == cuts.accepted,
        "{below:?} held to measure.below of {} built",
        cuts.accepted
    );
    // The seeds really build something, and booleans among it.
    assert!(groups >= 125, "{groups} groups");
    assert!(accepted * 2 >= groups, "{accepted} of {groups} accepted");
    assert!(booleans >= 10, "{booleans} booleans");
    // Siblings that all refused would switch the identity oracle off.
    assert!(
        identities * 2 >= booleans,
        "identities checked on {identities} of {booleans} booleans"
    );
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

/// The plane cut step draws from a stream of its own: for every gate
/// seed, the design with the step is the design without it, group for
/// group, up to its first cut, and the whole design when it has none.
/// (After a cut the two differ: the cut drew nothing from the other
/// steps' stream, and how much it drew from its own hangs on the
/// document.) The checks do not change a design, so the designs are made
/// with the cheap ones.
#[test]
fn the_cut_step_takes_nothing_from_the_other_steps_stream() {
    let cheap = Options {
        deep: Deep::AtEnd,
        identities: false,
        determinism: false,
        scratch: PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("fuzz-streams"),
    };
    let (mut cut, mut whole) = (0, 0);
    for seed in SEEDS {
        let (with, _) = run_seed(setup(seed), cheap.clone(), &mut |_| {}).expect("runs");
        let generator = Generator::new(setup(seed)).without_plane_cuts();
        let (without, _) =
            run_generated(generator, setup(seed), cheap.clone(), &mut |_| {}).expect("runs");
        assert!(
            without.groups.iter().all(|g| g.step != "planeCut"),
            "seed {seed}: a cut with the step off"
        );
        match with.groups.iter().position(|g| g.step == "planeCut") {
            Some(first) => {
                cut += 1;
                assert_eq!(
                    with.groups.get(..first),
                    without.groups.get(..first),
                    "seed {seed}: the groups before its first cut, at group {first}"
                );
            }
            None => {
                whole += 1;
                assert_eq!(with, without, "seed {seed}: no cut, the same design");
            }
        }
    }
    assert!(
        cut >= 8 && whole >= 10,
        "{cut} seeds with a cut, {whole} without"
    );
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

/// A plane cut group is held to its target: a positive volume, no more
/// than the target had; plane by plane, what `measure.below` gives on the
/// kept side (a point and a normal, a base plane, a cut face; below and
/// above); and by one plane its two sides add up (the other side made on a
/// copy). Built cuts are counted as changed or not; one that leaves
/// nothing is a refusal, counted by its words.
#[test]
fn a_plane_cut_is_held_to_its_target() {
    let cut = |params: serde_json::Value| json!({"step": "planeCut", "commands": [{"command": "feature.planeCut", "params": params}]});
    let design = Design::from_json(&json!({
        "seed": 1, "scale": 1,
        "groups": [
            {"step": "primitive", "commands": [{"command": "feature.block", "params": {"size": [10, 10, 10], "name": "N0"}}]},
            // The block is 0 to 10 every way: 400 below z = 4, kept as a copy.
            cut(json!({"target": "{N0}", "planes": [{"point": [0, 0, 4], "normal": [0, 0, 1]}], "keep": true, "name": "N1"})),
            // Above z = 5 and below x + y = 10: 250.
            cut(json!({"target": "{N0}", "planes": [{"plane": "xy", "offset": 5, "side": "above"}, {"point": [5, 5, 5], "normal": [1, 1, 0]}], "name": "N2"})),
            // Nothing of the 400 lies above z = 20.
            cut(json!({"target": "{N1}", "planes": {"plane": "xy", "offset": 20, "side": "above"}, "name": "N3"})),
            // The 250 lies wholly below z = 50: built, unchanged.
            cut(json!({"target": "{N2}", "planes": [{"plane": "xy", "offset": 50}], "name": "N4"})),
            // The 400's cut face moved in by 1: 300 behind it.
            cut(json!({"target": "{N1}", "planes": [{"plane": "{N1}:cut(1)", "offset": -1}], "name": "N5"})),
        ],
    }))
    .expect("reads");
    let runner = run_design(&design, options()).expect("runs");
    let report = &runner.report;
    assert_eq!(report.finding, None);
    assert_eq!(report.accepted, 5);
    assert_eq!(report.refused.len(), 1);
    assert!(
        report.refused[0].reason.contains("leaves nothing"),
        "{:?}",
        report.refused
    );
    let all = Tally {
        checked: 4,
        skipped: 0,
    };
    assert_eq!(report.cut_volumes, all);
    // Plane by plane: the two planes of N2 too, its first on a copy.
    assert_eq!(report.cut_below, all);
    // N1, N4 and N5 cut by one plane.
    assert_eq!(
        report.cut_sides,
        Tally {
            checked: 3,
            skipped: 0
        }
    );
    assert_eq!((report.cuts_changed, report.cuts_unchanged), (3, 1));
    assert_eq!(
        report.steps.get("planeCut"),
        Some(&Steps {
            groups: 5,
            accepted: 4
        })
    );
    let json = report.to_json();
    assert_eq!(
        json["steps"]["planeCut"],
        json!({"groups": 5, "accepted": 4})
    );
    assert_eq!(json["cuts"]["sides"], json!({"checked": 3, "skipped": 0}));
    assert_eq!(json["cuts"]["below"], json!({"checked": 4, "skipped": 0}));
    assert_eq!(json["cuts"]["changed"], 3);
    assert_eq!(json["cuts"]["unchanged"], 1);
}
