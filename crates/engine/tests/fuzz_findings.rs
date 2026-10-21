//! Designs the random-design fuzzer (`apps/craft3d-fuzz`) found that ran
//! for minutes or did not end (2026-10-17), each replayed from its script
//! (`fuzz_findings/<seed>.json`, as `craft3d-cli run --script` takes it).
//! A hang is as bad as a crash: each has to end within a bound on its
//! work, either with the body built and valid or with the feature refused
//! by a reason that says why. Each replay runs on a thread of its own and
//! fails, rather than hangs, when it does not end in [`HARD_LIMIT`]; each
//! step may add at most [`STEP_WORK`] units to the work count
//! (`craft3d_geom::allowance`); and the outcome of every feature, built or
//! refused, is pinned.
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
//! - The run of 2026-10-20 (docs/design/booleans.md › The fuzzer's
//!   timeouts of 2026-10-20). Patterns and booleans of bodies whose faces
//!   lie near one another (20757788, 20757179, 20830834, 20824278,
//!   20824950): the zero-thickness check looked at every place many times
//!   and at every face whose box (a whole ball) held it, and numeric
//!   intersections asked again were made again. Shells and offsets of
//!   chamfers on cones, balls and tori (20795706, 20814691, 20818576,
//!   20832173, 20833261): refused, after a halving of a dozen tries of
//!   seconds each, now held to `HALVING_WORK`.

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::Duration;

/// How long a replay may run before it fails: far past the slowest (two
/// and a half minutes in release, a few in the test profile), so a test
/// fails here only when a replay no longer ends.
const HARD_LIMIT: Duration = Duration::from_secs(1200);

/// The most work one step may count: a feature's own bounds (a face
/// operation's, at most 400 million here: a shell of 188 edges; a
/// self-check's) and what is not bounded besides, several times over.
const STEP_WORK: u64 = 2_000_000_000;

/// Replays `seed` and holds its features to `pins`: each feature's name
/// with `true` where it is built, `false` where it is refused.
fn replay(seed: &'static str, pins: &'static [(&'static str, bool)]) {
    let (sender, receiver) = std::sync::mpsc::channel();
    let runner = std::thread::Builder::new()
        .name(format!("replay {seed}"))
        .spawn(move || {
            let outcome = std::panic::catch_unwind(|| run(seed));
            let _ = sender.send(outcome);
        })
        .unwrap();
    let outcome = match receiver.recv_timeout(HARD_LIMIT) {
        Ok(Ok(outcome)) => outcome,
        Ok(Err(panic)) => std::panic::resume_unwind(panic),
        Err(_) => panic!("{seed}: the replay did not end within {HARD_LIMIT:?}"),
    };
    let _ = runner.join();
    let want: Vec<(String, bool)> = pins.iter().map(|(n, ok)| ((*n).to_string(), *ok)).collect();
    assert_eq!(outcome, want, "{seed}: the outcomes of its features");
}

/// The steps of `seed`, each within [`STEP_WORK`]; what was built checked
/// deep; the features' outcomes in history order.
fn run(seed: &str) -> Vec<(String, bool)> {
    use craft3d_geom::allowance;
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fuzz_findings")
        .join(format!("{seed}.json"));
    let steps: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let mut session = Session::new(Workspace::unrestricted());
    for (n, step) in steps.iter().enumerate() {
        let (id, params) = (step["command"].as_str().unwrap(), &step["params"]);
        // A command may refuse its params outright; it may not panic (the
        // session turns a panic into an error) or run on.
        let (result, work) = allowance::measured(|| session.execute(id, params));
        assert!(
            work <= STEP_WORK,
            "{seed} step {n} {id}: {work} units of work"
        );
        if let Ok(result) = result
            && result["status"] == "failed"
        {
            let reason = result["error"].as_str().unwrap_or_default();
            assert!(
                !reason.is_empty(),
                "{seed} {id}: refused without a reason: {result}"
            );
        }
    }
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
    let document = session.execute("document.inspect", &json!({})).unwrap();
    document["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["name"].as_str().unwrap().to_string(), f["status"] == "ok"))
        .collect()
}

macro_rules! findings {
    ($($(#[$attr:meta])* $name:ident: $seed:literal => [$($feature:literal: $ok:expr),* $(,)?],)*) => {
        $(
            #[test]
            $(#[$attr])*
            fn $name() {
                replay($seed, &[$(($feature, $ok)),*]);
            }
        )*
    };
}

const OK: bool = true;
const REFUSED: bool = false;

// Six in the gate, one of each kind that ends in seconds in release; the
// rest, a minute or two each in release (a face operation of blend faces
// ends at its bound), with `cargo test --release -p craft3d-engine --test
// fuzz_findings -- --include-ignored`.
findings! {
    chamfer_of_a_sphere_torus_union: "14214" => [
        "N0": OK, "N1": OK, "N2": OK, "N3": OK, "N5": OK, "S6": OK, "Revolve1": OK,
        "N6": OK, "N7": REFUSED,
    ],
    chamfer_of_tori_mirrored: "3016" => [
        "N0": OK, "S1": REFUSED, "Extrude1": REFUSED, "N1": REFUSED, "N2": OK, "N3": OK,
        "N4": OK, "S5": REFUSED, "N5": REFUSED, "N6": OK, "N7": REFUSED, "N8": OK, "N9": OK,
        "N10": OK, "N11": OK,
    ],
    shell_and_its_edit_on_a_filleted_pattern_of_spheres: "5457" => [
        "N0": OK, "N1": OK, "N2": OK, "N3": REFUSED, "N5": OK,
    ],
    pattern_of_ten_thousand_overlapping_spheres: "9611" => ["N0": OK, "N1": OK],
    chain_chamfers_on_a_pattern_of_tori: "12372" => [
        "N0": OK, "N1": OK, "N2": OK, "N3": OK, "N4": OK, "N5": OK,
    ],
    union_after_a_chamfer_of_a_cylinder_and_sphere: "1573" => [
        "N0": OK, "N1": OK, "N2": OK, "N3": OK, "S5": OK, "Extrude1": OK, "N5": OK,
        "N6": OK, "N8": OK,
    ],
    #[ignore = "a minute or two in release: cargo test --release ... -- --include-ignored"]
    offset_of_a_fillet_on_a_revolved_ring: "9601" => [
        "S0": OK, "N0": OK, "N1": OK, "N3": REFUSED,
    ],
    #[ignore = "a minute or two in release: cargo test --release ... -- --include-ignored"]
    shell_of_a_filleted_cylinder_and_sphere: "9534" => [
        "N0": OK, "N1": OK, "N3": OK, "N4": OK, "N5": REFUSED,
    ],
    #[ignore = "a minute or two in release: cargo test --release ... -- --include-ignored"]
    chamfer_of_a_filleted_torus_on_a_cylinder: "4499" => [
        "N0": OK, "N1": OK, "N2": OK, "N4": OK, "N5": OK, "N6": REFUSED,
    ],
    #[ignore = "a minute or two in release: cargo test --release ... -- --include-ignored"]
    offset_of_a_chamfer_between_two_revolved_rings: "1279" => [
        "S0": OK, "N0": OK, "S3": OK, "Revolve1": OK, "N3": OK, "N5": OK, "N8": REFUSED,
    ],
    #[ignore = "a minute or two in release: cargo test --release ... -- --include-ignored"]
    offset_of_a_fillet_on_a_pattern_of_spheres: "14064" => [
        "N0": OK, "N1": OK, "N2": OK, "N7": REFUSED,
    ],
    #[ignore = "a minute or two in release: cargo test --release ... -- --include-ignored"]
    offset_of_a_chamfer_on_a_pattern_of_tori: "5599" => [
        "N0": OK, "N1": OK, "N2": OK, "N3": REFUSED,
    ],
    #[ignore = "a minute or two in release: cargo test --release ... -- --include-ignored"]
    offset_after_a_fillet_on_a_pattern_of_spheres: "7802" => [
        "N0": OK, "N2": OK, "N4": OK, "N5": OK, "N6": OK, "N7": OK, "N9": REFUSED,
    ],
    #[ignore = "a minute or two in release: cargo test --release ... -- --include-ignored"]
    offset_and_shell_of_chamfers_on_a_revolved_ring: "6195" => [
        "S0": OK, "N0": OK, "N1": REFUSED, "N2": OK, "N3": REFUSED, "N5": OK,
        "N6": REFUSED, "N7": REFUSED,
    ],
    #[ignore = "a minute or two in release: cargo test --release ... -- --include-ignored"]
    shell_of_a_filleted_revolved_ring: "12333" => [
        "S0": OK, "N0": OK, "N1": OK, "N2": REFUSED,
    ],
    #[ignore = "a minute or two in release: cargo test --release ... -- --include-ignored"]
    shell_of_a_filleted_block_and_cone: "267" => [
        "N0": OK, "N1": OK, "N2": OK, "N3": OK, "S5": OK, "Extrude1": OK, "N5": OK,
        "N8": OK, "N9": OK, "N10": OK, "N11": REFUSED,
    ],
    #[ignore = "a minute or two in release: cargo test --release ... -- --include-ignored"]
    shell_of_a_chamfered_torus_and_block: "6578" => [
        "N0": OK, "S1": OK, "Extrude1": OK, "N1": OK, "N2": REFUSED, "N3": OK, "N4": OK,
        "S5": OK, "Extrude2": OK, "N5": OK, "N6": REFUSED, "N7": OK,
    ],
    #[ignore = "a minute or two in release: cargo test --release ... -- --include-ignored"]
    chamfer_of_a_cone_torus_union: "14420" => [
        "N1": OK, "N2": OK, "N5": OK, "N7": REFUSED, "N8": REFUSED,
    ],
    #[ignore = "a minute or two in release: cargo test --release ... -- --include-ignored"]
    circular_pattern_of_a_pattern_at_scale_1e3: "6597" => [
        "N2": OK, "N4": OK, "N7": OK,
    ],
    // The run of 2026-10-20: one in the gate (seconds), the others up to
    // two minutes in release.
    pattern_of_four_patterns_of_shelled_balls: "20757788-four" => [
        "N0": OK, "N1": OK, "N2": OK, "N5": OK,
    ],
    #[ignore = "a minute or two in release: cargo test --release ... -- --include-ignored"]
    pattern_of_six_patterns_of_shelled_balls: "20757788" => [
        "N0": OK, "N1": OK, "N2": OK, "N5": OK,
    ],
    #[ignore = "a minute or two in release: cargo test --release ... -- --include-ignored"]
    pattern_of_a_mirrored_pattern_of_rings_at_scale_1e3: "20757179" => [
        "S0": OK, "N0": OK, "N3": OK, "N4": OK, "N5": OK, "S7": OK, "N7": OK, "N8": OK,
    ],
    #[ignore = "a minute or two in release: cargo test --release ... -- --include-ignored"]
    subtractions_of_a_pattern_of_shelled_tubes: "20830834" => [
        "N0": OK, "N1": OK, "N2": OK, "N5": OK, "N8": OK, "N10": OK,
    ],
    #[ignore = "a minute or two in release: cargo test --release ... -- --include-ignored"]
    shell_of_a_pattern_of_rings_at_scale_1e_3: "20824278" => [
        "S3": OK, "N3": OK, "N7": OK, "N9": OK,
    ],
    #[ignore = "a minute or two in release: cargo test --release ... -- --include-ignored"]
    extrude_after_a_pattern_of_rings_at_scale_1e_3: "20824950" => [
        "S0": OK, "N0": OK, "N1": OK, "N2": OK, "N3": OK, "N4": OK, "S5": OK, "N5": OK,
    ],
    #[ignore = "a minute or two in release: cargo test --release ... -- --include-ignored"]
    offset_of_a_chamfer_on_a_torus_in_common_with_a_cone_at_scale_1e3: "20795706" => [
        "N0": OK, "N1": OK, "N3": OK, "N7": OK, "N8": REFUSED,
    ],
    #[ignore = "a minute or two in release: cargo test --release ... -- --include-ignored"]
    shell_of_a_mirrored_pattern_of_a_chamfered_union_at_scale_1e_3: "20814691" => [
        "S0": OK, "N0": OK, "S1": OK, "N1": OK, "N2": OK, "N3": OK, "N4": OK, "N5": OK,
        "N6": REFUSED,
    ],
    #[ignore = "a minute or two in release: cargo test --release ... -- --include-ignored"]
    shell_of_a_chamfered_cone_less_its_mirror_at_scale_1e_3: "20818576" => [
        "N0": OK, "N2": OK, "N3": OK, "N6": OK, "N7": REFUSED,
    ],
    #[ignore = "a minute or two in release: cargo test --release ... -- --include-ignored"]
    shell_of_a_chamfered_torus_in_common_with_balls_at_scale_1e3: "20832173" => [
        "N0": OK, "N1": OK, "N2": OK, "N3": OK, "N5": OK, "N7": REFUSED,
    ],
    #[ignore = "a minute or two in release: cargo test --release ... -- --include-ignored"]
    offset_of_a_chamfer_on_a_ball_in_common_with_a_cylinder_at_scale_1e_3: "20833261" => [
        "N1": OK, "N4": OK, "N5": OK, "N6": OK, "N8": REFUSED,
    ],
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
