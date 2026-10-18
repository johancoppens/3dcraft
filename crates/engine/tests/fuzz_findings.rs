//! Designs the random-design fuzzer (`apps/craft3d-fuzz`) found that ran
//! for minutes or did not end (2026-10-17), each replayed from its script
//! (`fuzz_findings/<seed>.json`, as `craft3d-cli run --script` takes it).
//! A hang is as bad as a crash: each has to end within a bound on its
//! work, either with the body built and valid or with the feature refused
//! by a reason that says why.
//!
//! - Offset face and shell of blend faces of spheres, tori and revolved
//!   bodies (1279, 5599, 6195, 7802, 9601, 14064; shells 267, 5457,
//!   6578, 9534, 12333): each try of the operation offsets faces of a few
//!   hundred spans and classifies points against them, a second or more
//!   each, and the search for the largest size that fits halves to the
//!   tolerance and widens freeform faces three times per size. Now the
//!   search is held to a work bound (`FACE_OP_WORK`) and refused by name
//!   when it is spent.
//! - Chamfers whose body cannot be meshed for the self-check (3016, 4499,
//!   14214, 14420): a face that cannot be meshed was refined round after
//!   round and the check tried again finer, each pass seconds. Now the
//!   check is held to its own bound (`SELF_CHECK_WORK`).
//! - A pattern of 10 000 overlapping spheres (9611): the instances were
//!   joined one after the other, quadratic in the count, and the
//!   self-check asked every one of 10 000 boxes per triangle. Now they
//!   are joined in pairs and the boxes are asked through a tree.
//! - Slow but ending (1573, 12372, 6597): replayed to keep them so.

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_engine::{Session, Workspace};
use craft3d_testkit::cost::WorkClock;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::Duration;

/// The most work (CPU time of the replaying thread, test profile) one
/// replay may take. Before the fixes the slowest ran for more than ten
/// minutes in release; now the slowest ends in about ten seconds there.
const MOST_WORK: Duration = Duration::from_secs(150);

fn replay(seed: &str) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fuzz_findings")
        .join(format!("{seed}.json"));
    let steps: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let mut session = Session::new(Workspace::unrestricted());
    let clock = WorkClock::start();
    for step in &steps {
        let (id, params) = (step["command"].as_str().unwrap(), &step["params"]);
        // A command may refuse its params outright; it may not panic or
        // run on (the session turns a panic into an error, and the clock
        // below catches the rest).
        if let Ok(result) = session.execute(id, params)
            && result["status"] == "failed"
        {
            let reason = result["error"].as_str().unwrap_or_default();
            assert!(
                !reason.is_empty(),
                "{seed} {id}: refused without a reason: {result}"
            );
        }
    }
    let work = clock.elapsed();
    eprintln!("{seed}: {work:?} of work");
    assert!(work < MOST_WORK, "{seed}: {work:?} of work");
    // What was built is valid; what was not built says why.
    let check = session
        .execute("document.check", &json!({"deep": true}))
        .unwrap();
    for problem in check["problems"].as_array().unwrap() {
        let text = problem["problem"].as_str().unwrap_or_default();
        assert!(
            problem["feature"].is_string() && !text.is_empty(),
            "{seed}: {problem}"
        );
        for bad in [
            "invalid body",
            "cannot be measured",
            "passes through or touches itself",
            "cannot be searched",
        ] {
            assert!(!text.starts_with(bad), "{seed}: {problem}");
        }
    }
    assert_eq!(check["marginal"], 0, "{seed}: {check}");
}

macro_rules! findings {
    ($($(#[$attr:meta])* $name:ident: $seed:literal,)*) => {
        $(
            #[test]
            $(#[$attr])*
            fn $name() {
                replay($seed);
            }
        )*
    };
}

// One of each kind in the gate; the rest, each tens of seconds of work in
// the test profile, with `cargo test --release -p craft3d-engine --test
// fuzz_findings -- --include-ignored` (a minute or two in all).
findings! {
    offset_of_a_fillet_on_a_revolved_ring: "9601",
    shell_of_a_filleted_cylinder_and_sphere: "9534",
    chamfer_of_a_sphere_torus_union: "14214",
    chamfer_of_a_filleted_torus_on_a_cylinder: "4499",
    pattern_of_ten_thousand_overlapping_spheres: "9611",
    chain_chamfers_on_a_pattern_of_tori: "12372",
    union_after_a_chamfer_of_a_cylinder_and_sphere: "1573",
    #[ignore = "tens of seconds in the test profile: run in release"]
    offset_of_a_chamfer_between_two_revolved_rings: "1279",
    #[ignore = "tens of seconds in the test profile: run in release"]
    offset_of_a_fillet_on_a_pattern_of_spheres: "14064",
    #[ignore = "tens of seconds in the test profile: run in release"]
    offset_of_a_chamfer_on_a_pattern_of_tori: "5599",
    #[ignore = "tens of seconds in the test profile: run in release"]
    offset_after_a_fillet_on_a_pattern_of_spheres: "7802",
    #[ignore = "tens of seconds in the test profile: run in release"]
    offset_and_shell_of_chamfers_on_a_revolved_ring: "6195",
    #[ignore = "tens of seconds in the test profile: run in release"]
    shell_of_a_filleted_revolved_ring: "12333",
    #[ignore = "tens of seconds in the test profile: run in release"]
    shell_of_a_filleted_block_and_cone: "267",
    #[ignore = "tens of seconds in the test profile: run in release"]
    shell_and_its_edit_on_a_filleted_pattern_of_spheres: "5457",
    #[ignore = "tens of seconds in the test profile: run in release"]
    shell_of_a_chamfered_torus_and_block: "6578",
    chamfer_of_a_cone_torus_union: "14420",
    chamfer_of_tori_mirrored: "3016",
    #[ignore = "tens of seconds in the test profile: run in release"]
    circular_pattern_of_a_pattern_at_scale_1e3: "6597",
}

/// The instances of a pattern that overlap are joined in pairs, then pairs
/// of pairs: the work grows with `n log n`, not with the square of the
/// count as when each was joined to all before it (a chain of 1000 balls
/// took ten seconds, one of 10 000 more than an hour).
#[test]
fn a_chain_of_overlapping_balls_is_joined_in_less_than_quadratic_work() {
    craft3d_testkit::cost::assert_not_quadratic("a pattern of n balls", (100, 400), |n| {
        let mut s = Session::new(Workspace::unrestricted());
        s.execute("file.new", &json!({"units": "mm"})).unwrap();
        s.execute(
            "feature.sphere",
            &json!({"name": "Ball", "position": [0.0, 0.0, 0.0], "radius": 10.0}),
        )
        .unwrap();
        let made = s
            .execute(
                "feature.pattern",
                &json!({"name": "Chain", "source": "Ball", "count": n, "step": [10.0, 10.0, 10.0]}),
            )
            .unwrap();
        assert_eq!(made["status"], "ok", "{made}");
    });
}
