//! The search trees of freeform faces (`PatchTree`) are kept on a shelf
//! while an open document's body has their surface, and let go when none
//! has (`craft3d_kernel::release_unused`, run after an edit or a close).
//! Since 2026-11-04 the shelf keeps beside each tree a hash of its key,
//! and the session works out the hashes of the surfaces an open
//! document's bodies hold once per regeneration of it (writing out every
//! freeform surface took some 40 ms per edit of any document while the
//! fuel tank was open); what is kept and what goes must stay the same:
//! after every command the hashes the session kept are those of the
//! bodies open now, worked out afresh, and the shelf holds no tree of a
//! surface outside them. An edit and an undo of the document itself, and
//! a regeneration made again at the same revision with the bodies the
//! budget had let go of, each give it hashes of their own.
//!
//! The shelf is one per process, so this file holds one test.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};

/// What the session kept is what the open bodies hold, and the shelf holds
/// nothing else.
fn kept_as_held(s: &Session, after: &str) {
    let (held, kept) = s.tree_keys_held_and_kept();
    assert_eq!(kept, held, "after {after}: the kept hashes are stale");
    let shelf = craft3d_kernel::shelf_keys();
    assert!(
        shelf.is_subset(&held),
        "after {after}: {} trees on the shelf of surfaces no body holds",
        shelf.difference(&held).count()
    );
}

#[test]
fn the_trees_of_an_open_document_stay_and_those_of_a_closed_one_go() {
    let script: Vec<Value> = serde_json::from_str(include_str!("example_models/mouse.json"))
        .expect("the mouse example is a list of commands");
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    for step in &script {
        let id = step[0].as_str().unwrap();
        let params = step.get(1).cloned().unwrap_or_else(|| json!({}));
        run(&mut s, id, params);
    }
    let (built, _) = craft3d_kernel::shelf_size();
    assert!(built > 0, "the mouse's booleans put trees on the shelf");
    kept_as_held(&s, "the mouse");
    // The mouse itself edited: the trees of its new surfaces stay, those
    // of the surfaces it no longer has go.
    run(
        &mut s,
        "variable.set",
        json!({"name": "height", "expr": "38 mm"}),
    );
    kept_as_held(&s, "an edit of the mouse");
    assert!(
        craft3d_kernel::shelf_size().0 > 0,
        "the edited mouse's trees were let go"
    );
    // And undone: its first surfaces are held again, the edited ones not.
    run(&mut s, "edit.undo", json!({}));
    kept_as_held(&s, "an undo of the mouse");
    // Another document, edited: the mouse is still open, so the trees of
    // its faces stay (its bodies hold their surfaces).
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "feature.block", json!({"size": [10, 10, 10]}));
    run(&mut s, "variable.set", json!({"name": "w", "expr": "3 mm"}));
    let (kept, _) = craft3d_kernel::shelf_size();
    assert!(
        kept > 0,
        "{built} trees built, none kept while the mouse is open"
    );
    let held = kept;
    kept_as_held(&s, "an edit of another document");
    // Again: nothing changed for the mouse, nothing more goes.
    run(&mut s, "variable.set", json!({"name": "w", "expr": "4 mm"}));
    assert_eq!(craft3d_kernel::shelf_size().0, held);
    // The mouse's shell taken from a block far from it, with a budget
    // that keeps no body that went into a later feature: the shell's body
    // is let go of, and its surfaces are not held. A command that needs it
    // derives it again at the same revision: its surfaces are held again.
    s.set_body_budget(0);
    run(&mut s, "file.activate", json!({"document": 0}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [10, 10, 10], "position": [1000, 0, 0], "name": "Far"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Far", "tool": "Shell", "name": "Far less shell"}),
    );
    kept_as_held(&s, "the shell let go of");
    let (without, _) = s.tree_keys_held_and_kept();
    run(&mut s, "datum.references", json!({"feature": "Shell"}));
    kept_as_held(&s, "the shell derived again");
    let (with, _) = s.tree_keys_held_and_kept();
    assert!(
        with.len() > without.len(),
        "the shell derived again holds no more surfaces ({} and {})",
        with.len(),
        without.len()
    );
    // The mouse closed: the next edit lets go of every tree.
    run(&mut s, "file.activate", json!({"document": 0}));
    run(&mut s, "file.close", json!({"discard": true}));
    run(&mut s, "variable.set", json!({"name": "w", "expr": "5 mm"}));
    assert_eq!(craft3d_kernel::shelf_size().0, 0, "{held} trees were kept");
}
