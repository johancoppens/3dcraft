//! The kernel's shelf of search trees for freeform surfaces lets go of the
//! trees of surfaces no open document holds (the developer's decision of
//! 2026-10-15): closing a document with a freeform model empties it, and
//! the same model built again gives the same body, bit for bit.
//!
//! The shelf is one per process, so this file holds one test: no other
//! test in its process puts trees on it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// A loft from a square to a circle (its sides freeform) with a rod cut
/// out of it, in a new document: the volume and area of the result.
fn freeform_part(s: &mut Session) -> (f64, f64) {
    run(s, "file.new", json!({"units": "mm"}));
    run(s, "sketch.create", json!({"plane": "xy", "name": "Low"}));
    run(
        s,
        "sketch.polyline",
        json!({"sketch": "Low", "closed": true,
               "points": [[-10, -10], [10, -10], [10, 10], [-10, 10]]}),
    );
    run(
        s,
        "sketch.create",
        json!({"plane": "xy", "offset": 20, "name": "High"}),
    );
    run(
        s,
        "sketch.circle",
        json!({"sketch": "High", "center": [0, 0], "radius": 6}),
    );
    let made = run(
        s,
        "feature.loft",
        json!({"sections": ["Low", "High"], "name": "Spout"}),
    );
    assert_eq!(made["status"], "ok", "{made}");
    // A rod through one of the freeform sides.
    run(
        s,
        "feature.cylinder",
        json!({"name": "Rod", "radius": 3, "height": 40,
               "position": [-20, 0, 8], "axis": [1, 0, 0]}),
    );
    let made = run(
        s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Spout", "tool": "Rod", "name": "Drilled"}),
    );
    assert_eq!(made["status"], "ok", "{made}");
    let inspect = run(s, "document.inspect", json!({}));
    let body = inspect["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == "Drilled")
        .unwrap()
        .clone();
    (
        body["volume"].as_f64().unwrap(),
        body["area"].as_f64().unwrap(),
    )
}

#[test]
fn closing_a_freeform_model_empties_the_shelf_and_changes_no_result() {
    let mut s = Session::new(Workspace::none());
    let first = freeform_part(&mut s);
    let (trees, bytes) = craft3d_kernel::shelf_size();
    assert!(trees > 0 && bytes > 0, "{trees} trees, {bytes} bytes");
    // A second document, plain, does not let go of the first's trees.
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "feature.block", json!({"size": [5, 5, 5]}));
    assert_eq!(craft3d_kernel::shelf_size().0, trees);
    run(&mut s, "file.close", json!({"discard": true}));
    assert_eq!(craft3d_kernel::shelf_size().0, trees);
    // Closing the freeform model lets go of all of them.
    run(&mut s, "file.close", json!({"discard": true}));
    assert_eq!(craft3d_kernel::shelf_size(), (0, 0));
    // Built again: the trees again, and the same body to the bit.
    let again = freeform_part(&mut s);
    assert_eq!(first.0.to_bits(), again.0.to_bits());
    assert_eq!(first.1.to_bits(), again.1.to_bits());
    assert_eq!(craft3d_kernel::shelf_size().0, trees);
    // Deleting the boolean and the loft lets go of the trees of the
    // surfaces only they held.
    run(&mut s, "feature.delete", json!({"feature": "Drilled"}));
    run(&mut s, "feature.delete", json!({"feature": "Spout"}));
    run(&mut s, "document.inspect", json!({}));
    assert_eq!(craft3d_kernel::shelf_size(), (0, 0));
}
