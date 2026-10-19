//! The bodies that went into later features are kept within a budget
//! (`Session::set_body_budget`, `CRAFT3D_BODY_BUDGET_MB`); beyond it they
//! are let go of and derived again when a command needs them, so a chain
//! of unions built command by command costs the budget plus the
//! document's own bodies, not one body per step
//! (docs/design/regeneration.md).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use craft3d_engine::{Session, Workspace};
use serde_json::json;

/// `n` blocks apart, united one after the other.
fn chain(session: &mut Session, n: usize) {
    for i in 0..n {
        run(
            session,
            "feature.block",
            json!({"name": format!("B{i}"), "size": [10, 10, 10], "position": [20 * i, 0, 0]}),
        );
    }
    let mut target = "B0".to_string();
    for i in 1..n {
        let name = format!("U{i}");
        run(
            session,
            "feature.boolean",
            json!({"op": "union", "target": target, "tool": format!("B{i}"), "name": name}),
        );
        target = name;
    }
}

#[test]
fn within_a_budget_of_nothing_only_the_documents_bodies_are_kept_and_every_command_still_holds() {
    let mut s = Session::new(Workspace::none());
    s.set_body_budget(0);
    run(&mut s, "file.new", json!({}));
    chain(&mut s, 6);
    let (bytes, gone) = s.bodies_held().unwrap();
    assert_eq!((bytes, gone), (0, 10));
    let inspect = run(&mut s, "document.inspect", json!({}));
    assert_eq!(inspect["bodies"].as_array().unwrap().len(), 1);
    // The last union's tool changes: the union is rebuilt from the union
    // before it, which was let go of and is derived again, with
    // everything it is made from.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "B5", "size": [12, 10, 10]}),
    );
    assert_eq!(s.last_regeneration().unwrap().0, 2);
    assert_eq!(s.last_regeneration_rederived().unwrap(), 9);
    assert_eq!(s.bodies_held().unwrap(), (0, 10));
    let measured = run(&mut s, "document.measure", json!({}));
    assert!(
        (measured["volume"].as_f64().unwrap() - 6200.0).abs() < 1e-6,
        "{measured}"
    );
    // A deep check holds every boolean to its operands, also those let
    // go of: it has them derived again, and they are let go of again at
    // the next change.
    let check = run(&mut s, "document.check", json!({"deep": true}));
    assert_eq!(
        (check["ok"].as_bool(), check["booleansChecked"].as_u64()),
        (Some(true), Some(5)),
        "{check}"
    );
    assert_eq!(s.bodies_held().unwrap().1, 0);
    // What a datum can be placed by on a body that went into a later
    // feature is listed from that body.
    let references = run(&mut s, "datum.references", json!({"feature": "U2"}));
    assert_eq!(
        references["vertices"].as_array().unwrap().len(),
        24,
        "{references}"
    );
    // Undone, the next command that reads the document regenerates it,
    // and the bodies derived again for the check are let go of.
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "document.inspect", json!({}));
    assert_eq!(s.bodies_held().unwrap(), (0, 10));
    let references = run(&mut s, "datum.references", json!({"feature": "U2"}));
    assert_eq!(
        references["vertices"].as_array().unwrap().len(),
        24,
        "{references}"
    );
    // That body alone is kept, as asked for.
    let (held, gone) = s.bodies_held().unwrap();
    assert!(held > 0 && gone == 9, "{held} held, {gone} gone");
    // Undo, redo and a deletion in the middle give what a regeneration
    // from nothing gives, body for body.
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(s.regeneration_matches_a_full_one(), Some(true));
    // U3 gone, U4 and U5 fail and U2, B3, B4 and B5 are the document's
    // bodies again, U2 derived again.
    run(&mut s, "feature.delete", json!({"feature": "U3"}));
    let inspect = run(&mut s, "document.inspect", json!({}));
    assert_eq!(inspect["bodies"].as_array().unwrap().len(), 4, "{inspect}");
    assert_eq!(s.regeneration_matches_a_full_one(), Some(true));
}

#[test]
fn a_budget_that_holds_the_last_unions_operands_keeps_an_edit_at_the_end_cheap() {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({}));
    chain(&mut s, 8);
    let (all, gone) = s.bodies_held().unwrap();
    assert_eq!(gone, 0);
    // The tool of the last union and the union before it: about a block
    // and a body of seven blocks, out of the chain's thirty-five.
    let budget = all * 8 / 35 + 1;
    s.set_body_budget(budget);
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "B7", "size": [12, 10, 10]}),
    );
    assert_eq!(s.last_regeneration_rederived().unwrap(), 0);
    let (held, gone) = s.bodies_held().unwrap();
    assert!(held <= budget && gone == 12, "{held} held, {gone} gone");
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "B7", "size": [14, 10, 10]}),
    );
    assert_eq!(
        (
            s.last_regeneration().unwrap().0,
            s.last_regeneration_rederived().unwrap()
        ),
        (2, 0)
    );
    // An edit at the start rebuilds the chain, deriving again the tools
    // that were let go of, once each; the result is the one a
    // regeneration from nothing gives.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "B0", "size": [14, 10, 10]}),
    );
    assert_eq!(
        (
            s.last_regeneration().unwrap().0,
            s.last_regeneration_rederived().unwrap()
        ),
        (8, 6)
    );
    assert_eq!(s.regeneration_matches_a_full_one(), Some(true));
}
