//! What a feature inserted at the rollback marker re-pointed is kept in
//! the document, so it can be taken back without the undo history and
//! after a save and a load: deleting the inserted feature gives the later
//! features the references they had (the document is the one it was
//! before, its volumes and its features' parameters), and editing it so it
//! takes another body gives back what it took and re-points the features
//! that named the new one, as if it had been inserted so. Each case is
//! checked against the same features built in that order from the start,
//! at three scales.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::run;
use common::{session_in, workdir};
use craft3d_engine::Session;
use serde_json::{Value, json};

type Step = (&'static str, Value);

const SCALES: [f64; 3] = [1e-3, 1.0, 1e3];

fn new(k: f64) -> Session {
    let mut s = common::session();
    run(
        &mut s,
        "file.new",
        json!({"units": "mm", "tolerance": 1e-6 * k}),
    );
    s
}

/// A block with a hole, a fillet on it, a pattern of the result, and a
/// spare block moved up.
fn part(k: f64) -> Vec<Step> {
    vec![
        (
            "feature.block",
            json!({"size": [20.0 * k, 10.0 * k, 6.0 * k], "name": "Base"}),
        ),
        (
            "feature.block",
            json!({"size": [4.0 * k, 4.0 * k, 4.0 * k], "position": [0, 30.0 * k, 0], "name": "Spare"}),
        ),
        (
            "feature.cylinder",
            json!({"radius": 2.0 * k, "height": 10.0 * k, "position": [10.0 * k, 5.0 * k, -2.0 * k], "name": "Tool"}),
        ),
        (
            "feature.boolean",
            json!({"op": "subtract", "target": "Base", "tool": "Tool", "name": "Cut"}),
        ),
        (
            "feature.fillet",
            json!({"edges": [["F1:face(2)", "F1:face(4)"]], "radius": 1.0 * k, "name": "Round"}),
        ),
        (
            "feature.pattern",
            json!({"source": "Round", "count": 2, "step": [30.0 * k, 0, 0], "name": "Row"}),
        ),
        (
            "feature.transform",
            json!({"source": "Spare", "translate": [0, 0, 10.0 * k], "name": "Up"}),
        ),
    ]
}

fn build(s: &mut Session, steps: &[Step]) {
    for (id, params) in steps {
        let r = run(s, id, params.clone());
        assert_ne!(r["status"], "failed", "{id} {params}: {r}");
    }
}

/// Saved and opened again: what follows works from the file alone.
fn round_trip(s: &mut Session, dir: &std::path::Path, name: &str) {
    let path = format!("{name}.3dcraft");
    run(s, "file.save", json!({"path": path, "overwrite": true}));
    let mut opened = session_in(dir);
    run(&mut opened, "file.open", json!({"path": path}));
    *s = opened;
}

/// The features as `document.inspect` lists them: id, name, status and
/// parameters (bodies by feature id), without their measures.
fn features(s: &mut Session) -> Vec<Value> {
    let inspect = run(s, "document.inspect", json!({"measures": false}));
    inspect["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| json!({"id": f["id"], "name": f["name"], "status": f["status"], "params": f["params"]}))
        .collect()
}

/// Each body's name, volume, area and centroid.
fn bodies(s: &mut Session) -> Vec<(String, f64, f64, [f64; 3])> {
    let inspect = run(s, "document.inspect", json!({}));
    inspect["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| {
            let c = &b["centroid"];
            (
                b["name"].as_str().unwrap().to_string(),
                b["volume"].as_f64().unwrap(),
                b["area"].as_f64().unwrap(),
                [0, 1, 2].map(|i| c[i].as_f64().unwrap()),
            )
        })
        .collect()
}

fn same_bodies(ours: &mut Session, theirs: &mut Session, k: f64, what: &str) {
    let (a, b) = (bodies(ours), bodies(theirs));
    assert_eq!(a.len(), b.len(), "{what}: {a:?} vs {b:?}");
    for (x, y) in a.iter().zip(&b) {
        assert_eq!(x.0, y.0, "{what}");
        let near = |p: f64, q: f64, size: f64| (p - q).abs() <= 1e-9 * size;
        assert!(
            near(x.1, y.1, y.1.abs()),
            "{what} {}: volume {x:?} vs {y:?}",
            x.0
        );
        assert!(
            near(x.2, y.2, y.2.abs()),
            "{what} {}: area {x:?} vs {y:?}",
            x.0
        );
        for i in 0..3 {
            assert!(
                near(x.3[i], y.3[i], 100.0 * k),
                "{what} {}: centroid {x:?} vs {y:?}",
                x.0
            );
        }
    }
}

fn forward(s: &mut Session) {
    let rolled = run(s, "history.rollback", json!({"feature": null}));
    assert!(
        rolled["failed"].as_array().is_none_or(Vec::is_empty),
        "{rolled}"
    );
    assert_eq!(run(s, "document.check", json!({}))["ok"], true);
}

/// A chamfer inserted before the subtraction re-points its target;
/// saved, opened again and deleted, the subtraction names the block
/// again: features and bodies are those of the part never changed.
#[test]
fn deleting_an_inserted_feature_gives_the_later_ones_their_bodies_back() {
    for k in SCALES {
        let dir = workdir("repoint_undone", &format!("delete-{k}"));
        let mut s = session_in(&dir);
        run(
            &mut s,
            "file.new",
            json!({"units": "mm", "tolerance": 1e-6 * k}),
        );
        build(&mut s, &part(k));
        let mut original = new(k);
        build(&mut original, &part(k));
        let before = features(&mut original);
        run(&mut s, "history.rollback", json!({"feature": "Tool"}));
        let added = run(
            &mut s,
            "feature.chamfer",
            json!({"edges": [["F1:face(1)", "F1:face(3)"]], "distance": 0.5 * k, "name": "Bevel"}),
        );
        assert_eq!(
            added["repointed"],
            json!([{"feature": "Cut", "param": "target"}]),
            "{added}"
        );
        round_trip(&mut s, &dir, "inserted");
        let deleted = run(&mut s, "feature.delete", json!({"feature": "Bevel"}));
        assert_eq!(
            deleted["restored"],
            json!([{"feature": "Cut", "param": "target", "names": "Base"}]),
            "{deleted}"
        );
        forward(&mut s);
        assert_eq!(features(&mut s), before, "{k}");
        same_bodies(&mut s, &mut original, k, "deleted");
        // And saved again, it is that document still.
        round_trip(&mut s, &dir, "deleted");
        assert_eq!(features(&mut s), before, "{k}");
    }
}

/// A move of the fillet inserted before the pattern, edited to move the
/// spare block instead: the pattern repeats the fillet again, and the
/// move after it, which named the spare block, moves the inserted one;
/// as the same features built in that order. Deleted after a save and a
/// load, the part is the one never changed.
#[test]
fn an_inserted_feature_edited_to_take_another_body_and_then_deleted() {
    for k in SCALES {
        let dir = workdir("repoint_undone", &format!("edit-{k}"));
        let mut s = session_in(&dir);
        run(
            &mut s,
            "file.new",
            json!({"units": "mm", "tolerance": 1e-6 * k}),
        );
        build(&mut s, &part(k));
        let mut original = new(k);
        build(&mut original, &part(k));
        let before = features(&mut original);
        run(&mut s, "history.rollback", json!({"feature": "Round"}));
        let added = run(
            &mut s,
            "feature.transform",
            json!({"source": "Round", "translate": [0, 0, 5.0 * k], "name": "Lift"}),
        );
        assert_eq!(
            added["repointed"],
            json!([{"feature": "Row", "param": "source"}]),
            "{added}"
        );
        round_trip(&mut s, &dir, "inserted");
        let edited = run(
            &mut s,
            "feature.edit",
            json!({"feature": "Lift", "source": "Spare"}),
        );
        assert_eq!(
            edited["restored"],
            json!([{"feature": "Row", "param": "source", "names": "Round"}]),
            "{edited}"
        );
        assert_eq!(
            edited["repointed"],
            json!([{"feature": "Up", "param": "source"}]),
            "{edited}"
        );
        forward(&mut s);
        // Built so from the start: Lift after Round moves Spare, Row
        // repeats Round, Up moves Lift.
        let mut steps = part(k);
        steps[6].1["source"] = json!("Lift");
        steps.insert(
            5,
            (
                "feature.transform",
                json!({"source": "Spare", "translate": [0, 0, 5.0 * k], "name": "Lift"}),
            ),
        );
        let mut straight = new(k);
        build(&mut straight, &steps);
        same_bodies(&mut s, &mut straight, k, "edited");
        // Saved with the marker cleared, opened, and the move deleted.
        round_trip(&mut s, &dir, "edited");
        let deleted = run(&mut s, "feature.delete", json!({"feature": "Lift"}));
        assert_eq!(
            deleted["restored"],
            json!([{"feature": "Up", "param": "source", "names": "Spare"}]),
            "{deleted}"
        );
        assert_eq!(run(&mut s, "document.check", json!({}))["ok"], true);
        assert_eq!(features(&mut s), before, "{k}");
        same_bodies(&mut s, &mut original, k, "deleted");
    }
}

/// An edit of an inserted feature that keeps the body it takes changes
/// none of the references: the pattern still repeats the inserted move,
/// as the same features built in that order.
#[test]
fn an_edit_that_keeps_the_body_re_points_nothing() {
    let k = 1.0;
    let mut s = new(k);
    build(&mut s, &part(k));
    run(&mut s, "history.rollback", json!({"feature": "Round"}));
    run(
        &mut s,
        "feature.transform",
        json!({"source": "Round", "translate": [0, 0, 5], "name": "Lift"}),
    );
    let edited = run(
        &mut s,
        "feature.edit",
        json!({"feature": "Lift", "translate": [0, 0, 7]}),
    );
    assert!(edited.get("repointed").is_none(), "{edited}");
    assert!(edited.get("restored").is_none(), "{edited}");
    forward(&mut s);
    let mut steps = part(k);
    steps[5].1["source"] = json!("Lift");
    steps.insert(
        5,
        (
            "feature.transform",
            json!({"source": "Round", "translate": [0, 0, 7], "name": "Lift"}),
        ),
    );
    let mut straight = new(k);
    build(&mut straight, &steps);
    same_bodies(&mut s, &mut straight, k, "kept");
}
