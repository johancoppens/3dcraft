//! The search trees of freeform faces (`PatchTree`) are kept on a shelf
//! while an open document's body has their surface, and let go when none
//! has (`craft3d_kernel::release_unused`, run after an edit or a close).
//! Since 2026-11-04 a face's surface is written out only as far as its
//! beginning unless that beginning is the beginning of a tree's key (it
//! took 60 to 100 ms per edit of any document while the fuel tank was
//! open); what is kept and what goes must stay the same.
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

#[test]
fn the_trees_of_an_open_document_stay_and_those_of_a_closed_one_go() {
    let script: Vec<Value> =
        serde_json::from_str(include_str!("../../../docs/examples/mouse.json"))
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
    // Again: nothing changed for the mouse, nothing more goes.
    run(&mut s, "variable.set", json!({"name": "w", "expr": "4 mm"}));
    assert_eq!(craft3d_kernel::shelf_size().0, held);
    // The mouse closed: the next edit lets go of every tree.
    run(&mut s, "file.activate", json!({"document": 0}));
    run(&mut s, "file.close", json!({"discard": true}));
    run(&mut s, "variable.set", json!({"name": "w", "expr": "5 mm"}));
    assert_eq!(craft3d_kernel::shelf_size().0, 0, "{held} trees were kept");
}
