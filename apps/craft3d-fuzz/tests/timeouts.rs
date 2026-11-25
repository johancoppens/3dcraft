//! Random designs that ran past the fuzzer's minute on `main` ed93bafc
//! (the nightly run 45516571671 and `main`'s own run 45627273373,
//! 2026-11-25; docs/design/booleans.md › *The fuzzer's timeouts of
//! 2026-11-25*), replayed from their reproducers (`timeouts/<seed>.json`,
//! as `craft3d-fuzz check FILE` takes them) with the commands alone (no
//! sibling booleans, the deep check at the end, no save and open). The
//! step that was dear is held to a bound on its work, counted
//! (`craft3d_geom::allowance`), not clocked, and every group to the answer
//! `main` gave, word for word.
//!
//! - 20838232 and 20827917: a fillet's face against a face of the other
//!   body that its border lies on, tangent along it. The march refuses the
//!   pair, and whether the two faces lie on one another over a region
//!   (`boolean::coincide`) was asked of 2^18 samples, each by two winding
//!   numbers: 247.7 million units a boolean (17 s), 904 million (70 s).
//!   No sample can count (within a quarter of the tolerance of the other
//!   face only along the fillet's border, its own boundary), so the answer
//!   is "no evidence" before any winding number, and the boolean refuses
//!   as it did.
//! - 20802370: a shell whose open face is a torus a fillet runs along, as
//!   thick as the torus's tube. The search with the fillets offset as
//!   freeform faces, whose refusal is never said, halved after the whole
//!   failed: 83.6 of the shell's 102.3 million units (14 of 44 s); it is
//!   tried at the whole distance alone now. What is left, the search with
//!   the fillets rebuilt (15.6 million units, 28.6 s: its halving's bound
//!   counts no section of the rebuilt fillets), is owed: a minute in the
//!   test profile, so in the gate's list of ignored tests.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft3d_fuzz::runner::Applied;
use craft3d_fuzz::{Deep, Design, Options, Runner};
use std::path::PathBuf;

/// Each group's work in allowance units, and the report at the end.
fn replay(seed: &str) -> (Vec<u64>, craft3d_fuzz::Report) {
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
        scratch: PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("timeouts"),
    };
    let mut runner = Runner::new(design.setup, options).unwrap();
    let mut work = Vec::with_capacity(design.groups.len());
    for group in &design.groups {
        let (applied, units) = craft3d_geom::allowance::measured(|| runner.apply(group));
        assert!(
            applied != Applied::Found,
            "{seed}: {:?}",
            runner.report.finding
        );
        // (With `--nocapture`, for the records: each group's work.)
        eprintln!("{seed}: group {}: {units} units", work.len());
        work.push(units);
    }
    runner.finish();
    assert!(
        runner.report.finding.is_none(),
        "{seed}: {:?}",
        runner.report.finding
    );
    (work, runner.report)
}

/// The groups `report` refused, as `(group, command, reason)`.
fn refused(report: &craft3d_fuzz::Report) -> Vec<(usize, &str, &str)> {
    report
        .refused
        .iter()
        .map(|r| (r.group, r.command.as_str(), r.reason.as_str()))
        .collect()
}

const TANGENT: &str = "boolean does not support this: the surfaces are tangent or nearly coincident over too large an area to intersect";

/// The cylinder intersected, then united, with the torus less the
/// cylinder filleted along their edge: both refused as on `main`, each in
/// a sixteenth of what it took there (247.7 and 247.9 million units).
#[test]
fn a_cylinder_against_a_fillet_tangent_to_it_is_refused_without_asking_every_sample() {
    let (work, report) = replay("20838232");
    let tangent = format!("{TANGENT} (face 0 of the first body, face 2 of the second body)");
    assert_eq!(
        refused(&report),
        vec![
            (4, "feature.boolean", tangent.as_str()),
            (5, "feature.boolean", tangent.as_str()),
        ]
    );
    assert_eq!((report.groups, report.accepted), (6, 4));
    for group in [4, 5] {
        let units = work.get(group).copied().unwrap();
        assert!(units <= 16_000_000, "group {group}: {units} units");
    }
}

/// A pattern of tori filleted along three of its curves, united with a
/// piece of itself cut off by a plane: refused as on `main`, in under a
/// twenty-fifth of what it took there (904 million units).
#[test]
fn a_filleted_pattern_of_tori_with_its_own_cut_piece_is_refused_without_asking_every_sample() {
    let (work, report) = replay("20827917");
    let tangent = format!("{TANGENT} (face 6 of the first body, face 1 of the second body)");
    assert_eq!(
        refused(&report),
        vec![(5, "feature.boolean", tangent.as_str())]
    );
    assert_eq!((report.groups, report.accepted), (6, 5));
    let units = work.get(5).copied().unwrap();
    assert!(units <= 32_000_000, "{units} units");
}

/// The shell of a filleted union of a ball and a torus, open at the
/// torus, as thick as the torus's tube: refused as on `main`, with what
/// it found to fit, in under a third of what it took there (102.3 million
/// units, 83.6 million of them the second search's halving).
#[test]
#[ignore = "half a minute in release (the first search, owed): cargo test --release -p craft3d-fuzz --test timeouts -- --include-ignored"]
fn the_shell_of_a_filleted_ring_as_thick_as_its_tube_tries_the_second_way_whole_only() {
    let (work, report) = replay("20802370");
    assert_eq!(
        refused(&report),
        vec![
            (
                2,
                "feature.fillet",
                "radius: too large for edges.0 (between F1:face(0) and F1:face(2)): the 5 mm asked does not fit beside the face F1:face(0), nor the radii tried from 1.25 mm up to it; the largest found to fit is just below 1.25 mm",
            ),
            (
                8,
                "feature.shell",
                "thickness: 2.5 mm is too far: the radius of the face would reach zero: the face would vanish (how far it can go before it does was not found: what is said to fit here does, and more may) (the face F6:copy(0)/F5:copy(1)/F4:face(0)); what fits is below 2.47802 mm",
            ),
        ]
    );
    assert_eq!((report.groups, report.accepted), (9, 7));
    let units = work.get(8).copied().unwrap();
    assert!(units <= 32_000_000, "{units} units");
}
