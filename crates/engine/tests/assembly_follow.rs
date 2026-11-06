//! An open assembly follows its source files (docs/design/assemblies.md ›
//! *Following the sources*): a source saved in the same session is read
//! again by every open assembly, one changed outside it when the
//! assembly's tab is activated or `assembly.follow` runs; a pinned
//! instance does not follow and says so; a source that changed and
//! cannot be read again keeps the last bodies, with the reason; and
//! nothing is regenerated when nothing changed.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use common::{session_in, workdir};
use craft3d_engine::Session;
use serde_json::{Value, json};

/// A part whose block is `w` × 10 × 10, `w` a variable.
fn knob(s: &mut Session, path: &str, w: &str) {
    run(s, "file.new", json!({}));
    run(s, "variable.set", json!({ "name": "w", "expr": w }));
    run(
        s,
        "feature.block",
        json!({ "name": "Base", "size": ["w", 10, 10] }),
    );
    run(s, "file.save", json!({ "path": path }));
    run(s, "file.close", json!({}));
}

/// An assembly of one instance `K` of `knob.3dcraft`, saved, left open
/// and active.
fn assembly(s: &mut Session) {
    run(s, "assembly.new", json!({}));
    run(
        s,
        "instance.add",
        json!({ "source": "knob.3dcraft", "name": "K" }),
    );
    run(s, "file.save", json!({ "path": "pi.3dcraftasm" }));
}

fn inspect(s: &mut Session) -> Value {
    run(s, "assembly.inspect", json!({ "measures": true }))
}

fn instance<'a>(inspected: &'a Value, name: &str) -> &'a Value {
    inspected["instances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["name"] == name)
        .unwrap_or_else(|| panic!("no instance {name} in {inspected}"))
}

fn volume(s: &mut Session) -> f64 {
    let i = inspect(s);
    instance(&i, "K")["measured"]["volume"].as_f64().unwrap()
}

fn regenerations(s: &mut Session) -> u64 {
    inspect(s)["regenerations"].as_u64().unwrap()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-6 * b.abs().max(1.0)
}

/// The developer's case (2026-11-06): the source opened from the
/// assembly (instance.open), a variable changed and saved there; the
/// assembly's tab shows the new part, read when the file was saved.
#[test]
fn a_source_saved_in_the_same_session_is_followed_by_the_open_assembly() {
    let dir = workdir("assembly-follow", "same-session");
    let mut s = session_in(&dir);
    knob(&mut s, "knob.3dcraft", "40 mm");
    assembly(&mut s);
    assert!(close(volume(&mut s), 4000.0));
    let opened = run(&mut s, "instance.open", json!({ "name": "K" }));
    let asm_doc = 0;
    assert_ne!(opened["document"], json!(asm_doc), "{opened}");
    run(
        &mut s,
        "variable.set",
        json!({ "name": "w", "expr": "50 mm" }),
    );
    let saved = run(&mut s, "file.save", json!({}));
    // Said by the save: the assembly read its source again.
    let followed = saved["followed"].as_array().unwrap();
    assert_eq!(followed.len(), 1, "{saved}");
    assert_eq!(followed[0]["document"], json!(asm_doc), "{saved}");
    assert!(
        followed[0]["changed"][0]
            .as_str()
            .unwrap()
            .ends_with("knob.3dcraft"),
        "{saved}"
    );
    assert!(saved.get("warnings").is_none(), "{saved}");
    run(&mut s, "file.activate", json!({ "document": asm_doc }));
    assert!(close(volume(&mut s), 5000.0), "{}", inspect(&mut s));
    assert!(instance(&inspect(&mut s), "K")["held"].is_null());
    // Not an undo step: the assembly has nothing to undo from it.
    assert_eq!(
        run(&mut s, "file.list", json!({}))["documents"][0]["dirty"],
        false
    );
}

/// A source changed outside the session (another program, a checkout):
/// read again when the assembly's tab is activated, and by
/// assembly.follow; nothing is regenerated when nothing changed.
#[test]
fn a_source_changed_outside_is_followed_on_activation_and_nothing_else_regenerates() {
    let dir = workdir("assembly-follow", "outside");
    let mut s = session_in(&dir);
    knob(&mut s, "knob.3dcraft", "40 mm");
    assembly(&mut s);
    assert!(close(volume(&mut s), 4000.0));
    let before = regenerations(&mut s);
    // Nothing changed: following reads nothing new and builds nothing.
    let followed = run(&mut s, "assembly.follow", json!({}));
    assert_eq!(followed["followed"], json!([]), "{followed}");
    assert_eq!(followed["failed"], 0);
    run(&mut s, "file.new", json!({}));
    run(&mut s, "file.activate", json!({ "document": 0 }));
    assert_eq!(regenerations(&mut s), before);
    // An unrelated part saved: nothing regenerates either.
    run(&mut s, "file.activate", json!({ "document": 1 }));
    run(&mut s, "feature.block", json!({ "size": [1, 1, 1] }));
    let saved = run(&mut s, "file.save", json!({ "path": "other.3dcraft" }));
    assert!(saved.get("followed").is_none(), "{saved}");
    run(&mut s, "file.activate", json!({ "document": 0 }));
    assert_eq!(regenerations(&mut s), before);
    // Another session writes the source: the next activation follows it.
    let mut other = session_in(&dir);
    knob(&mut other, "knob.3dcraft", "60 mm");
    run(&mut s, "file.activate", json!({ "document": 1 }));
    let activated = run(&mut s, "file.activate", json!({ "document": 0 }));
    assert_eq!(activated["followed"][0]["document"], 0, "{activated}");
    assert!(close(volume(&mut s), 6000.0));
    assert_eq!(regenerations(&mut s), before + 1);
    // And assembly.follow, from any tab.
    knob(&mut other, "knob.3dcraft", "70 mm");
    run(&mut s, "file.activate", json!({ "document": 1 }));
    let followed = run(&mut s, "assembly.follow", json!({}));
    assert_eq!(followed["followed"][0]["document"], 0, "{followed}");
    run(&mut s, "file.activate", json!({ "document": 0 }));
    assert!(close(volume(&mut s), 7000.0));
}

/// A pinned instance does not follow its file, and says why.
#[test]
fn a_pinned_instance_does_not_follow_and_says_so() {
    let dir = workdir("assembly-follow", "pinned");
    let mut s = session_in(&dir);
    knob(&mut s, "knob.3dcraft", "40 mm");
    assembly(&mut s);
    run(
        &mut s,
        "instance.edit",
        json!({ "name": "K", "pinned": true }),
    );
    assert!(instance(&inspect(&mut s), "K")["held"].is_null());
    knob(&mut s, "knob.3dcraft", "50 mm");
    run(&mut s, "file.activate", json!({ "document": 0 }));
    assert!(close(volume(&mut s), 4000.0));
    let held = instance(&inspect(&mut s), "K")["held"].clone();
    assert!(held.as_str().unwrap().contains("pinned"), "{held}");
    // Unpinned, it follows.
    run(
        &mut s,
        "instance.edit",
        json!({ "name": "K", "pinned": false }),
    );
    assert!(close(volume(&mut s), 5000.0));
    assert!(instance(&inspect(&mut s), "K")["held"].is_null());
}

/// A source that is gone (or unreadable) keeps the last bodies read,
/// with the reason; back again, it is followed and the reason goes.
#[test]
fn a_source_that_cannot_be_read_again_keeps_the_last_bodies_with_the_reason() {
    let dir = workdir("assembly-follow", "missing");
    let mut s = session_in(&dir);
    knob(&mut s, "knob.3dcraft", "40 mm");
    assembly(&mut s);
    assert!(close(volume(&mut s), 4000.0));
    std::fs::remove_file(dir.join("knob.3dcraft")).unwrap();
    let followed = run(&mut s, "assembly.follow", json!({}));
    assert_eq!(followed["failed"], 1, "{followed}");
    let i = inspect(&mut s);
    assert!(instance(&i, "K")["error"].is_null(), "{i}");
    let held = instance(&i, "K")["held"].as_str().unwrap().to_string();
    assert!(held.contains("could not be read again"), "{held}");
    assert!(close(volume(&mut s), 4000.0));
    // By hand too: the last version stays, the reload says why.
    let reloaded = run(&mut s, "assembly.reload", json!({}));
    assert_eq!(
        reloaded["unread"].as_array().unwrap().len(),
        1,
        "{reloaded}"
    );
    assert!(close(volume(&mut s), 4000.0));
    // The file back, changed: followed, and the reason is gone.
    knob(&mut s, "knob.3dcraft", "30 mm");
    run(&mut s, "file.activate", json!({ "document": 0 }));
    assert!(close(volume(&mut s), 3000.0));
    assert!(instance(&inspect(&mut s), "K")["held"].is_null());
}

/// A subassembly's own source saved in the session is followed by the
/// top assembly open in another tab.
#[test]
fn a_subassembly_source_is_followed_too() {
    let dir = workdir("assembly-follow", "sub");
    let mut s = session_in(&dir);
    knob(&mut s, "knob.3dcraft", "40 mm");
    assembly(&mut s);
    run(&mut s, "file.close", json!({}));
    run(&mut s, "assembly.new", json!({}));
    run(
        &mut s,
        "instance.add",
        json!({ "source": "pi.3dcraftasm", "name": "Sub" }),
    );
    run(&mut s, "file.save", json!({ "path": "top.3dcraftasm" }));
    let i = inspect(&mut s);
    let v0 = instance(&i, "Sub")["measured"]["volume"].as_f64().unwrap();
    assert!(close(v0, 4000.0), "{i}");
    knob(&mut s, "knob.3dcraft", "50 mm");
    run(&mut s, "file.activate", json!({ "document": 0 }));
    let i = inspect(&mut s);
    let v1 = instance(&i, "Sub")["measured"]["volume"].as_f64().unwrap();
    assert!(close(v1, 5000.0), "{i}");
}
