//! Where an assembly's outline says each mate's marker is
//! (`OutlineMate::frame`, docs/design/assemblies.md › *Mate markers*): at
//! the mate's own frame in the solve, and nowhere when the mate fails;
//! never at the frame of a flexible subassembly's mate of the same name.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;

use common::plain::run;
use common::{session_in, workdir};
use craft3d_engine::{OutlineMate, Session};
use serde_json::json;

/// A bar 24 × 6 × 3 with datum axes `End1` at x = 3 and `End2` at
/// x = 21, saved at `bar.3dcraft` and left open (document 0).
fn bar(s: &mut Session) {
    run(s, "file.new", json!({}));
    run(
        s,
        "feature.block",
        json!({ "name": "Bar", "size": [24, 6, 3] }),
    );
    run(
        s,
        "datum.axis",
        json!({ "name": "End1", "through": [[3, 3, 0], [3, 3, 1]] }),
    );
    run(
        s,
        "datum.axis",
        json!({ "name": "End2", "through": [[21, 3, 0], [21, 3, 1]] }),
    );
    run(s, "file.save", json!({ "path": "bar.3dcraft" }));
}

fn mate<'a>(mates: &'a [OutlineMate], name: &str) -> &'a OutlineMate {
    mates
        .iter()
        .find(|m| m.name == name)
        .unwrap_or_else(|| panic!("no mate {name}: {mates:?}"))
}

/// A mate of the assembly that fails (its datum is gone from the source)
/// has no frame, though a flexible subassembly in it has a mate of the
/// same name that is solved; before 2026-11-05 the marker took that
/// mate's frame. The subassembly's mate does not appear as the
/// assembly's, and the assembly's other mates keep theirs.
#[test]
fn a_failed_mate_never_takes_a_flexible_subassembly_s_frame() {
    let dir = workdir("mate-frames", "flexible");
    let mut s = session_in(&dir);
    bar(&mut s);
    // The pair: A on the world, B turning about A's End1 (`Drive`).
    run(&mut s, "assembly.new", json!({}));
    for name in ["A", "B"] {
        run(
            &mut s,
            "instance.add",
            json!({ "source": "bar.3dcraft", "name": name, "body": "Bar" }),
        );
    }
    run(
        &mut s,
        "mate.add",
        json!({ "kind": "fixed", "a": "world/z", "b": "A/End1", "name": "Hold" }),
    );
    run(
        &mut s,
        "mate.add",
        json!({ "kind": "revolute", "a": "A/End1", "b": "B/End1", "name": "Drive", "flip": true }),
    );
    run(&mut s, "file.save", json!({ "path": "pair.3dcraftasm" }));
    // The top: the pair flexible, and a bar turning about the pair's B's
    // End2, its own `Drive`.
    run(&mut s, "assembly.new", json!({}));
    run(
        &mut s,
        "instance.add",
        json!({ "source": "pair.3dcraftasm", "name": "Pair", "mode": "flexible" }),
    );
    run(
        &mut s,
        "instance.add",
        json!({ "source": "bar.3dcraft", "name": "Top", "body": "Bar" }),
    );
    run(
        &mut s,
        "mate.add",
        json!({ "kind": "revolute", "a": "Pair/B/End1", "b": "Top/End2", "name": "Drive", "flip": true }),
    );
    let outline = s.assembly_outline().expect("an assembly");
    let drive = mate(&outline.mates, "Drive");
    assert_eq!(drive.status, "ok", "{drive:?}");
    assert_eq!(drive.parts, ["Pair".to_string(), "Top".to_string()]);
    let frame = drive.frame.expect("the solved mate has its frame");
    assert!(frame.origin.iter().all(|c| c.is_finite()));
    // Only the assembly's own mates are listed.
    let names: Vec<&str> = outline.mates.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, ["Drive"]);

    // The bar loses End2: the top's Drive fails; the pair's Drive (on
    // End1) is still solved in the parent.
    run(&mut s, "file.activate", json!({ "document": 0 }));
    run(&mut s, "feature.delete", json!({ "feature": "End2" }));
    run(&mut s, "file.save", json!({ "path": "bar.3dcraft" }));
    run(&mut s, "file.activate", json!({ "document": 2 }));
    run(&mut s, "assembly.reload", json!({}));
    let outline = s.assembly_outline().expect("an assembly");
    let drive = mate(&outline.mates, "Drive");
    assert_eq!(drive.status, "failed", "{drive:?}");
    assert_eq!(drive.frame, None, "a failed mate has no marker: {drive:?}");
}

/// A mate whose datum disappears from its source fails with no frame;
/// the mates that still hold keep theirs.
#[test]
fn a_mate_whose_datum_is_gone_has_no_frame() {
    let dir = workdir("mate-frames", "gone");
    let mut s = session_in(&dir);
    bar(&mut s);
    run(&mut s, "assembly.new", json!({}));
    for name in ["Ground", "Arm"] {
        run(
            &mut s,
            "instance.add",
            json!({ "source": "bar.3dcraft", "name": name, "body": "Bar" }),
        );
    }
    run(
        &mut s,
        "mate.add",
        json!({ "kind": "fixed", "a": "world/z", "b": "Ground/End1", "name": "Base" }),
    );
    run(
        &mut s,
        "mate.add",
        json!({ "kind": "revolute", "a": "Ground/End2", "b": "Arm/End1", "name": "Elbow", "flip": true }),
    );
    let outline = s.assembly_outline().unwrap();
    let elbow = mate(&outline.mates, "Elbow").frame.expect("placed");
    // At the ground's End2, 18 mm from its End1 on the world's z axis.
    let r = elbow.origin[0].phypot(elbow.origin[1]);
    assert!((r - 18.0).abs() < 1e-6, "{elbow:?}");
    run(&mut s, "file.activate", json!({ "document": 0 }));
    run(&mut s, "feature.delete", json!({ "feature": "End2" }));
    run(&mut s, "file.save", json!({ "path": "bar.3dcraft" }));
    run(&mut s, "file.activate", json!({ "document": 1 }));
    run(&mut s, "assembly.reload", json!({}));
    let outline = s.assembly_outline().unwrap();
    let elbow = mate(&outline.mates, "Elbow");
    assert_eq!(elbow.status, "failed", "{elbow:?}");
    assert_eq!(elbow.frame, None);
    assert!(mate(&outline.mates, "Base").frame.is_some());
}
