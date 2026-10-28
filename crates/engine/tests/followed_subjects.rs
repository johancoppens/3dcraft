//! A subject that follows a body to the newest body that grew from it
//! (`{body, follow}`, docs/design/verification.md › *Subjects*): an
//! assertion on a feature's body keeps measuring when a feature is added
//! on top, while a body named as a plain string keeps its old meaning
//! (exactly that feature's body, an error once it went into another).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use common::{error, session, session_in, workdir};
use craft3d_engine::Session;
use serde_json::{Value, json};

fn listed(s: &mut Session, name: &str) -> Value {
    let list = run(s, "assert.list", json!({}));
    list["assertions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["name"] == name)
        .cloned()
        .unwrap_or_else(|| panic!("no assertion {name}: {list}"))
}

fn close(v: &Value, want: f64, what: &str) {
    let got = v.as_f64().unwrap_or_else(|| panic!("{what}: {v}"));
    assert!(
        (got - want).abs() <= 1e-6 * want.abs().max(1.0),
        "{what}: {got} != {want}"
    );
}

/// A block B (10 × 20 × 30) with assertions on its volume, one exact and
/// one followed, at scale `k`.
fn block(s: &mut Session, k: f64) {
    run(s, "file.new", json!({"units": "mm", "tolerance": 1e-6 * k}));
    run(
        s,
        "feature.block",
        json!({"name": "B", "size": [10.0 * k, 20.0 * k, 30.0 * k]}),
    );
    run(
        s,
        "assert.add",
        json!({"name": "exact", "measure": "volume", "of": "B", "min": "0 mm^3"}),
    );
    run(
        s,
        "assert.add",
        json!({"name": "grown", "measure": "volume", "of": {"body": "B"}, "min": "0 mm^3"}),
    );
}

#[test]
fn a_followed_subject_measures_the_newest_body_and_an_exact_one_keeps_its_meaning() {
    for k in [1e-3, 1.0, 1e3] {
        let mut s = session();
        block(&mut s, k);
        close(
            &listed(&mut s, "grown")["measured"],
            6000.0 * k.powi(3),
            "B alone",
        );
        // A hole through B: the exact subject is in error (its body went
        // into the cut), the followed one measures the cut.
        run(
            &mut s,
            "feature.block",
            json!({"name": "T", "size": [4.0 * k, 4.0 * k, 40.0 * k], "position": [3.0 * k, 8.0 * k, -5.0 * k]}),
        );
        run(
            &mut s,
            "feature.boolean",
            json!({"op": "subtract", "target": "B", "tool": "T", "name": "Cut"}),
        );
        let cut = 5520.0 * k.powi(3);
        let exact = listed(&mut s, "exact");
        assert_eq!(exact["status"], "error", "{exact}");
        assert!(
            exact["message"].as_str().unwrap().contains("follow"),
            "{exact}"
        );
        let grown = listed(&mut s, "grown");
        assert_eq!(grown["status"], "passed", "{grown}");
        close(&grown["measured"], cut, "after the cut");
        // The tool of a subtraction did not grow into the cut.
        run(
            &mut s,
            "assert.add",
            json!({"name": "tool", "measure": "volume", "of": {"body": "T"}, "min": "0 mm^3"}),
        );
        let tool = listed(&mut s, "tool");
        assert_eq!(tool["status"], "error", "{tool}");
        assert!(
            tool["message"].as_str().unwrap().contains("subtraction"),
            "{tool}"
        );
        // A union on top, with B's descendant as the target: followed again;
        // the union's tool grew into it as well.
        run(
            &mut s,
            "feature.block",
            json!({"name": "D", "size": [6.0 * k, 5.0 * k, 10.0 * k], "position": [k, k, 25.0 * k]}),
        );
        run(
            &mut s,
            "assert.add",
            json!({"name": "boss", "measure": "volume", "of": [{"body": "D", "follow": true}], "min": "0 mm^3"}),
        );
        run(
            &mut s,
            "feature.boolean",
            json!({"op": "union", "target": "Cut", "tool": "D", "name": "Join"}),
        );
        let joined = 5670.0 * k.powi(3);
        close(
            &listed(&mut s, "grown")["measured"],
            joined,
            "after the union",
        );
        close(
            &listed(&mut s, "boss")["measured"],
            joined,
            "the union's tool",
        );
        // `follow: false` is the exact meaning.
        run(
            &mut s,
            "assert.edit",
            json!({"name": "grown", "of": {"body": "B", "follow": false}}),
        );
        assert_eq!(listed(&mut s, "grown")["status"], "error");
    }
}

/// The followed subject names its feature by id: a rename keeps it, and
/// it is saved and read back as written.
#[test]
fn a_followed_subject_survives_a_rename_and_a_save() {
    let dir = workdir("followed-subjects", "save");
    let mut s = session_in(&dir);
    block(&mut s, 1.0);
    run(
        &mut s,
        "feature.block",
        json!({"name": "T", "size": [4, 4, 40], "position": [3, 8, -5]}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "B", "tool": "T"}),
    );
    run(
        &mut s,
        "feature.rename",
        json!({"feature": "B", "name": "Base"}),
    );
    close(&listed(&mut s, "grown")["measured"], 5520.0, "renamed");
    run(&mut s, "file.save", json!({"path": "f.3dcraft"}));
    let mut t = session_in(&dir);
    run(&mut t, "file.open", json!({"path": "f.3dcraft"}));
    let back = listed(&mut t, "grown");
    assert_eq!(back["status"], "passed", "{back}");
    close(&back["measured"], 5520.0, "read back");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The measure commands take the same subject; a bad one is an error.
#[test]
fn a_followed_subject_in_a_measure_and_bad_forms() {
    let mut s = session();
    block(&mut s, 1.0);
    run(
        &mut s,
        "feature.transform",
        json!({"source": "B", "translate": [100, 0, 0], "name": "Moved"}),
    );
    error(&mut s, "measure.thickness", json!({"of": "B"}));
    let v = run(&mut s, "measure.thickness", json!({"of": {"body": "B"}}));
    close(&v["thickness"], 10.0, "the moved block");
    for bad in [
        json!({"body": 3}),
        json!({"body": "B", "follow": "yes"}),
        json!({"body": "B", "extra": 1}),
        json!({"body": "nosuch"}),
        json!([{"body": "B"}, 7]),
    ] {
        error(&mut s, "measure.volume", json!({"of": bad}));
    }
}
