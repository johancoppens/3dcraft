//! The review of `timeouts.rs` (`check/fuzz-timeouts` 963416fb): what it
//! bounds is the count (`craft3d_geom::allowance::measured`), taken around
//! commands whose booleans run outside any bound on the count, where the
//! section meets pairs ahead on helper threads that fill the patch trees'
//! memos, whose hits the count does not charge
//! (`crates/kernel/tests/boolean_ledger.rs`). The ledger
//! (`allowance::measured_asked`) is the measure the kernel calls a
//! function of what is asked alone. The random design 20838232 replayed
//! three times in one process: both, per group, side by side.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft3d_fuzz::runner::Applied;
use craft3d_fuzz::{Deep, Design, Options, Runner};
use std::path::PathBuf;

/// Each group's count and ledger, as `timeouts.rs` replays the design.
fn replay(seed: &str, round: usize) -> Vec<(u64, u64)> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/timeouts")
        .join(format!("{seed}.json"));
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let design = Design::from_json(&value).unwrap();
    let options = Options {
        deep: Deep::AtEnd,
        identities: false,
        determinism: false,
        scratch: PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("timeouts-review-{round}")),
    };
    let mut runner = Runner::new(design.setup, options).unwrap();
    let mut work = Vec::with_capacity(design.groups.len());
    for group in &design.groups {
        let ((applied, count), ledger) = craft3d_geom::allowance::measured_asked(|| {
            craft3d_geom::allowance::measured(|| runner.apply(group))
        });
        assert!(
            applied != Applied::Found,
            "{seed}: {:?}",
            runner.report.finding
        );
        work.push((count, ledger));
    }
    runner.finish();
    assert!(
        runner.report.finding.is_none(),
        "{seed}: {:?}",
        runner.report.finding
    );
    work
}

#[test]
#[ignore = "the review's measurement, printed: cargo test --release -p craft3d-fuzz --test timeouts_review -- --ignored --nocapture"]
fn the_count_and_the_ledger_of_the_fillet_tangent_booleans_replayed() {
    let rounds: Vec<Vec<(u64, u64)>> = (0..3).map(|round| replay("20838232", round)).collect();
    for (round, work) in rounds.iter().enumerate() {
        for (group, (count, ledger)) in work.iter().enumerate() {
            eprintln!("round {round}, group {group}: count {count}, ledger {ledger}");
        }
    }
    let counts_agree = rounds.windows(2).all(|pair| {
        pair.first()
            .zip(pair.get(1))
            .is_some_and(|(a, b)| a.iter().map(|w| w.0).eq(b.iter().map(|w| w.0)))
    });
    let ledgers_agree = rounds.windows(2).all(|pair| {
        pair.first()
            .zip(pair.get(1))
            .is_some_and(|(a, b)| a.iter().map(|w| w.1).eq(b.iter().map(|w| w.1)))
    });
    eprintln!("the counts agree across rounds: {counts_agree}; the ledgers: {ledgers_agree}");
}
