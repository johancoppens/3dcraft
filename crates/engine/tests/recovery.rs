//! Autosave and crash recovery (roadmap R1): a session that dies without
//! a clean end leaves its unsaved documents in the recovery directory,
//! and the next session gets them back.
//!
//! A crash is simulated by dropping the session without
//! `Session::end_recovery`: that leaves the recovery files and lets go of
//! the launch's lock, as the end of a process does. (The real thing, a
//! process killed with SIGKILL, is `apps/craft3d-cli/tests/recovery.rs`.)
//! Time is a `ManualClock`: no test waits for an interval.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use craft3d_engine::{Access, ManualClock, RecoveryOptions, Session, Workspace};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const INTERVAL: Duration = Duration::from_secs(30);

fn workdir(name: &str) -> PathBuf {
    common::workdir("recovery", name)
}

/// A session that reads and writes under `dir/work` and autosaves into
/// `dir/recovery`, by a clock the test moves.
fn session(dir: &Path) -> (Session, Arc<ManualClock>) {
    let work = dir.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let mut s = Session::new(Workspace {
        read: Access::Root(work.clone()),
        write: Access::Root(work),
    });
    let clock = Arc::new(ManualClock::new(1_800_000_000));
    s.enable_recovery(
        RecoveryOptions::manual(dir.join("recovery"), clock.clone()).with_interval(INTERVAL),
    );
    (s, clock)
}

fn error(session: &mut Session, id: &str, params: Value) -> String {
    session.execute(id, &params).unwrap_err().to_string()
}

/// The names in the recovery directory, sorted, with each launch's key
/// replaced by `K1`, `K2`, … in order of appearance.
fn listing(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir.join("recovery")) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

fn kinds(dir: &Path) -> Vec<String> {
    let mut kinds: Vec<String> = listing(dir)
        .iter()
        .map(|name| name.rsplit_once('.').unwrap().1.to_string())
        .collect();
    kinds.sort();
    kinds
}

/// A part with a variable, primitives, a constrained sketch and an
/// extrusion of it: enough for a regeneration to have something to get
/// wrong.
fn build_part(s: &mut Session) {
    run(s, "file.new", json!({}));
    run(s, "variable.set", json!({"name": "width", "expr": "40 mm"}));
    run(
        s,
        "feature.block",
        json!({"name": "Plate", "size": ["width", 20, 5]}),
    );
    run(s, "feature.cylinder", json!({"radius": 2, "height": 10}));
    run(s, "sketch.create", json!({"name": "Profile"}));
    run(
        s,
        "sketch.polyline",
        json!({"sketch": "Profile", "closed": true, "points": [[0, 0], [9, 1], [10, 6], [1, 5]]}),
    );
    run(
        s,
        "sketch.constrain",
        json!({"sketch": "Profile", "type": "distance", "entities": ["e5"], "value": "width / 4"}),
    );
    run(
        s,
        "feature.extrude",
        json!({"sketch": "Profile", "distance": "width / 8", "name": "Body"}),
    );
}

/// What a document is, without its undo history: the features, their
/// states, the bodies with their mass properties and face names.
fn inspected(s: &mut Session) -> Value {
    let mut all = run(s, "document.inspect", json!({}));
    all.as_object_mut().unwrap().remove("history").unwrap();
    all
}

fn exported(s: &mut Session, dir: &Path, name: &str) -> Vec<u8> {
    run(s, "file.export", json!({ "path": name }));
    std::fs::read(dir.join("work").join(name)).unwrap()
}

fn saved(s: &mut Session, dir: &Path, name: &str) -> Vec<u8> {
    // A copy of the session, so the original keeps its unsaved changes.
    let mut copy = s.clone();
    run(&mut copy, "file.save", json!({ "path": name }));
    std::fs::read(dir.join("work").join(name)).unwrap()
}

fn documents(listed: &Value) -> &Vec<Value> {
    listed["documents"].as_array().unwrap()
}

#[test]
fn a_change_is_autosaved_after_the_interval_and_not_before() {
    let dir = workdir("interval");
    let (mut s, clock) = session(&dir);
    // An empty, never saved document is not worth a file.
    run(&mut s, "file.new", json!({}));
    clock.advance(INTERVAL * 2);
    s.autosave_poll();
    assert_eq!(listing(&dir), Vec::<String>::new());

    run(&mut s, "feature.block", json!({"size": [1, 2, 3]}));
    let overview = s.recovery_overview();
    assert!(overview.enabled);
    assert_eq!((overview.writes, overview.pending), (0, 1));
    assert_eq!(overview.interval_seconds, 30.0);
    clock.advance(INTERVAL - Duration::from_millis(1));
    s.autosave_poll();
    assert_eq!(listing(&dir), Vec::<String>::new(), "not before");
    assert_eq!(s.recovery_overview().last_autosave, None);

    clock.advance(Duration::from_millis(1));
    s.autosave_poll();
    assert_eq!(kinds(&dir), ["3dcraft", "json", "lock"]);
    let overview = s.recovery_overview();
    assert_eq!((overview.writes, overview.pending), (1, 0));
    assert_eq!(overview.last_autosave, Some(1_800_000_000 + 90));
    assert_eq!(overview.problem, None);

    // The recovery file is the document as `file.save` writes it: the
    // same bytes, nothing of the time or the process in them.
    let name = listing(&dir)
        .into_iter()
        .find(|n| n.ends_with(".3dcraft"))
        .unwrap();
    let autosaved = std::fs::read(dir.join("recovery").join(&name)).unwrap();
    assert_eq!(autosaved, saved(&mut s, &dir, "same.3dcraft"));

    // What varies is in the record.
    let record = name.replace(".3dcraft", ".json");
    let record: Value =
        serde_json::from_slice(&std::fs::read(dir.join("recovery").join(record)).unwrap()).unwrap();
    assert_eq!(record["recovery_version"], 1);
    assert_eq!(record["time"], 1_800_000_000_u64 + 90);
    assert_eq!(record["pid"], std::process::id());
    assert_eq!(record["name"], "untitled");
    assert_eq!(record["path"], Value::Null);
    assert_eq!(record["features"], 1);
    assert_eq!(record["revision"], 1);
    assert_eq!(name.split('-').next(), record["launch"].as_str());
}

#[test]
fn a_burst_of_commands_gives_one_write() {
    let dir = workdir("burst");
    let (mut s, clock) = session(&dir);
    run(&mut s, "file.new", json!({}));
    for i in 1..=10 {
        run(&mut s, "feature.block", json!({"size": [i, 1, 1]}));
        clock.advance(Duration::from_secs(1));
    }
    assert_eq!(s.recovery_overview().writes, 0);
    clock.advance(Duration::from_secs(20));
    s.autosave_poll();
    assert_eq!(s.recovery_overview().writes, 1);
    // The one write holds the last of the burst.
    let record = listing(&dir)
        .into_iter()
        .find(|n| n.ends_with(".json"))
        .unwrap();
    let record: Value =
        serde_json::from_slice(&std::fs::read(dir.join("recovery").join(record)).unwrap()).unwrap();
    assert_eq!(record["features"], 10);

    // Nothing changed: nothing is written again, however long it takes.
    clock.advance(INTERVAL * 10);
    s.autosave_poll();
    assert_eq!(s.recovery_overview().writes, 1);

    // The next burst is one write again, an interval after the last.
    for i in 1..=5 {
        run(&mut s, "feature.block", json!({"size": [1, i, 1]}));
    }
    assert_eq!(
        s.recovery_overview().writes,
        2,
        "long after the last write: at once"
    );
    for i in 1..=5 {
        run(&mut s, "feature.block", json!({"size": [1, 1, i]}));
        clock.advance(Duration::from_secs(1));
    }
    assert_eq!(s.recovery_overview().writes, 2);
    clock.advance(INTERVAL);
    s.autosave_poll();
    assert_eq!(s.recovery_overview().writes, 3);
}

#[test]
fn saving_closing_and_a_clean_end_remove_the_recovery_file() {
    let dir = workdir("removed");
    let (mut s, clock) = session(&dir);
    build_part(&mut s);
    clock.advance(INTERVAL);
    s.autosave_poll();
    assert_eq!(kinds(&dir), ["3dcraft", "json", "lock"]);

    // Saved: the recovery file is older than the real one.
    run(&mut s, "file.save", json!({"path": "part.3dcraft"}));
    assert_eq!(kinds(&dir), ["lock"]);

    // Changed again, written again, saved again, gone again.
    run(&mut s, "feature.block", json!({"size": [1, 1, 1]}));
    clock.advance(INTERVAL);
    s.autosave_poll();
    assert_eq!(kinds(&dir), ["3dcraft", "json", "lock"]);
    run(&mut s, "file.save", json!({}));
    assert_eq!(kinds(&dir), ["lock"]);

    // Closed without saving: the person said to throw the changes away.
    run(&mut s, "feature.block", json!({"size": [1, 1, 1]}));
    clock.advance(INTERVAL);
    s.autosave_poll();
    assert_eq!(kinds(&dir), ["3dcraft", "json", "lock"]);
    run(&mut s, "file.close", json!({"discard": true}));
    assert_eq!(kinds(&dir), ["lock"]);

    // A clean end leaves nothing at all, also of a document with changes.
    build_part(&mut s);
    clock.advance(INTERVAL);
    s.autosave_poll();
    assert_eq!(kinds(&dir), ["3dcraft", "json", "lock"]);
    s.end_recovery();
    assert_eq!(listing(&dir), Vec::<String>::new());
    assert!(!s.recovery_overview().enabled);
    // The session works on without a store.
    run(&mut s, "feature.block", json!({"size": [1, 1, 1]}));
}

#[test]
fn a_dead_sessions_document_is_listed_opened_and_regenerates_to_the_same_bodies() {
    let dir = workdir("recover");
    let (mut first, clock) = session(&dir);
    build_part(&mut first);
    clock.advance(INTERVAL);
    first.autosave_poll();
    let before = inspected(&mut first);
    let step_before = exported(&mut first, &dir, "bodies.step");
    let bytes_before = saved(&mut first, &dir, "before.3dcraft");
    assert!(before["bodies"].as_array().unwrap().len() >= 3, "{before}");
    // The crash: no clean end.
    drop(first);
    assert_eq!(kinds(&dir), ["3dcraft", "json", "lock"]);

    let (mut second, _) = session(&dir);
    // The scan at the start found it; nothing was opened by itself.
    let found = s_found(&second);
    assert_eq!(found.len(), 1);
    assert!(!second.has_document());
    let listed = run(&mut second, "recovery.list", json!({}));
    let documents = documents(&listed);
    assert_eq!(documents.len(), 1, "{listed}");
    let entry = &documents[0];
    assert_eq!(entry["name"], "untitled");
    assert_eq!(entry["path"], Value::Null);
    assert_eq!(entry["time"], 1_800_000_000_u64 + 30);
    assert_eq!(entry["features"], 4);
    assert_eq!(entry["openable"], true);
    assert_eq!(entry["problem"], Value::Null);
    assert_eq!(entry["pid"], std::process::id());
    assert!(entry["size"].as_u64().unwrap() > 100);
    assert!(entry["revision"].as_u64().unwrap() >= 6, "{entry}");
    assert_eq!(listed["running"], 0);

    let id = entry["id"].as_str().unwrap().to_string();
    let opened = run(&mut second, "recovery.open", json!({ "id": id }));
    assert_eq!(opened["recovered"], true);
    assert_eq!(opened["features"], 4);
    assert_eq!(opened["warning"], Value::Null);

    // The same history, and bodies that are the same to the last bit:
    // the document state, the exact geometry as STEP, the file's bytes.
    assert_eq!(inspected(&mut second), before);
    assert_eq!(exported(&mut second, &dir, "bodies.step"), step_before);
    assert_eq!(saved(&mut second, &dir, "after.3dcraft"), bytes_before);
    assert_eq!(second.regeneration_matches_a_full_one(), Some(true));

    // It is an unsaved document, marked as recovered.
    let open = run(&mut second, "file.list", json!({}));
    assert_eq!(open["documents"][0]["dirty"], true);
    assert_eq!(open["documents"][0]["recovered"], true);

    // What is lost: the undo stack, which is in no file. New edits undo.
    let e = error(&mut second, "edit.undo", json!({}));
    assert!(e.contains("nothing to undo"), "{e}");
    run(&mut second, "feature.block", json!({"size": [1, 1, 1]}));
    run(&mut second, "edit.undo", json!({}));
    assert_eq!(inspected(&mut second), before);

    // The recovery file is this session's now: no longer listed, and it
    // stays until the document is saved.
    let listed = run(&mut second, "recovery.list", json!({}));
    assert_eq!(documents_len(&listed), 0, "{listed}");
    assert_eq!(kinds(&dir), ["3dcraft", "json", "lock"]);
    let e = error(&mut second, "recovery.open", json!({ "id": id }));
    assert!(e.contains("no recoverable document"), "{e}");

    // A second crash before it is saved: it is still there.
    drop(second);
    let (mut third, _) = session(&dir);
    let listed = run(&mut third, "recovery.list", json!({}));
    assert_eq!(documents_len(&listed), 1, "{listed}");
    let id = listed["documents"][0]["id"].as_str().unwrap().to_string();
    run(&mut third, "recovery.open", json!({ "id": id }));
    assert_eq!(inspected(&mut third), before);
    run(&mut third, "file.save", json!({"path": "part.3dcraft"}));
    assert_eq!(kinds(&dir), ["lock"]);
    let open = run(&mut third, "file.list", json!({}));
    assert_eq!(open["documents"][0].get("recovered"), None);
    assert_eq!(open["documents"][0]["dirty"], false);
}

fn s_found(s: &Session) -> Vec<craft3d_engine::Recoverable> {
    s.recovery_overview().found
}

fn documents_len(listed: &Value) -> usize {
    documents(listed).len()
}

#[test]
fn two_live_sessions_on_one_directory_do_not_see_each_others_files() {
    let dir = workdir("two-live");
    let (mut a, clock_a) = session(&dir);
    let (mut b, clock_b) = session(&dir);
    build_part(&mut a);
    run(&mut b, "file.new", json!({}));
    run(&mut b, "feature.sphere", json!({"radius": 3}));
    clock_a.advance(INTERVAL);
    clock_b.advance(INTERVAL);
    a.autosave_poll();
    b.autosave_poll();
    assert_eq!(
        kinds(&dir),
        ["3dcraft", "3dcraft", "json", "json", "lock", "lock"]
    );

    for s in [&mut a, &mut b] {
        let listed = run(s, "recovery.list", json!({}));
        assert_eq!(documents_len(&listed), 0, "{listed}");
        assert_eq!(listed["running"], 1);
        // Discarding everything touches nothing of a running launch.
        let discarded = run(s, "recovery.discard", json!({"all": true}));
        assert_eq!(discarded["discarded"], 0);
    }
    assert_eq!(listing(&dir).len(), 6);

    // The id of a running launch's document is refused by name.
    let theirs = listing(&dir)
        .into_iter()
        .filter(|n| n.ends_with(".3dcraft"))
        .map(|n| n.trim_end_matches(".3dcraft").to_string())
        .collect::<Vec<_>>();
    let mut refused = 0;
    for id in &theirs {
        for command in ["recovery.open", "recovery.discard"] {
            let e = error(&mut a, command, json!({ "id": id }));
            assert!(
                e.contains("still running") || e.contains("no recoverable document"),
                "{e}"
            );
            refused += usize::from(e.contains("still running"));
        }
    }
    assert_eq!(refused, 2, "b's document, for both commands");
    assert_eq!(listing(&dir).len(), 6);

    // Once b is gone without a clean end, a sees its document.
    drop(b);
    let listed = run(&mut a, "recovery.list", json!({}));
    assert_eq!(documents_len(&listed), 1, "{listed}");
    assert_eq!(listed["documents"][0]["features"], 1);
    assert_eq!(listed["running"], 0);
}

#[test]
fn damaged_recovery_files_are_listed_with_a_reason_and_never_opened_blindly() {
    let dir = workdir("damaged");
    let recovery = dir.join("recovery");
    // Four documents of one dead launch.
    let (mut dead, clock) = session(&dir);
    for size in 1..=4 {
        run(&mut dead, "file.new", json!({}));
        run(&mut dead, "feature.block", json!({"size": [size, 1, 1]}));
    }
    clock.advance(INTERVAL);
    dead.autosave_poll();
    drop(dead);
    let files: Vec<String> = listing(&dir)
        .into_iter()
        .filter(|n| n.ends_with(".3dcraft"))
        .collect();
    assert_eq!(files.len(), 4);
    let record = |file: &str| recovery.join(file.replace(".3dcraft", ".json"));
    // 1: truncated. 2: its record is gone. 3: its document is gone.
    // 4: its record is not JSON.
    let whole = std::fs::read(recovery.join(&files[0])).unwrap();
    std::fs::write(recovery.join(&files[0]), &whole[..whole.len() / 2]).unwrap();
    std::fs::remove_file(record(&files[1])).unwrap();
    std::fs::remove_file(recovery.join(&files[2])).unwrap();
    std::fs::write(record(&files[3]), b"{\"recovery_version\": 1, \"name\": ").unwrap();
    // And things that are nobody's: a note, an unfinished temporary file
    // of the dead launch, a directory, a record of a launch from the
    // future, garbage with a recovery file's name.
    let key = files[0].split('-').next().unwrap().to_string();
    std::fs::write(recovery.join("notes.txt"), b"mine").unwrap();
    std::fs::write(recovery.join(format!(".{key}-9.3dcraft.77.tmp")), b"half").unwrap();
    std::fs::create_dir(recovery.join("0123456789abcdef-1.3dcraft")).unwrap();
    std::fs::write(
        recovery.join("fedcba9876543210-1.json"),
        br#"{"recovery_version": 99, "name": "later.3dcraft"}"#,
    )
    .unwrap();
    std::fs::write(recovery.join("fedcba9876543210-2.3dcraft"), [0xff_u8; 64]).unwrap();
    std::fs::write(recovery.join("fedcba9876543210-2.json"), [0xff_u8; 64]).unwrap();

    let (mut s, _) = session(&dir);
    let listed = run(&mut s, "recovery.list", json!({}));
    let by_id = |id: &str| {
        documents(&listed)
            .iter()
            .find(|d| d["id"] == id)
            .unwrap_or_else(|| panic!("{id} is not listed: {listed}"))
            .clone()
    };
    let id = |file: &str| file.trim_end_matches(".3dcraft").to_string();
    let truncated = by_id(&id(&files[0]));
    assert_eq!(truncated["openable"], false);
    assert!(
        truncated["problem"]
            .as_str()
            .unwrap()
            .contains("cannot be opened"),
        "{truncated}"
    );
    let no_record = by_id(&id(&files[1]));
    assert_eq!(no_record["openable"], true);
    assert_eq!(no_record["name"], "untitled");
    assert!(
        no_record["problem"]
            .as_str()
            .unwrap()
            .contains("record is missing"),
        "{no_record}"
    );
    let no_document = by_id(&id(&files[2]));
    assert_eq!(no_document["openable"], false);
    assert!(
        no_document["problem"]
            .as_str()
            .unwrap()
            .contains("document file is missing"),
        "{no_document}"
    );
    let bad_record = by_id(&id(&files[3]));
    assert_eq!(bad_record["openable"], true);
    assert!(
        bad_record["problem"]
            .as_str()
            .unwrap()
            .contains("record is damaged"),
        "{bad_record}"
    );
    assert_eq!(by_id("0123456789abcdef-1")["openable"], false);
    assert_eq!(by_id("fedcba9876543210-1")["openable"], false);
    assert_eq!(by_id("fedcba9876543210-2")["openable"], false);
    assert_eq!(documents_len(&listed), 7, "{listed}");
    // The unfinished temporary file of the dead launch was tidied away;
    // what is not ours was left alone.
    assert!(!listing(&dir).iter().any(|n| n.ends_with(".tmp")));
    assert!(recovery.join("notes.txt").exists());

    // What cannot be opened says why, and stays for the person to keep.
    let e = error(&mut s, "recovery.open", json!({"id": id(&files[0])}));
    assert!(e.contains("cannot be opened"), "{e}");
    let e = error(&mut s, "recovery.open", json!({"id": id(&files[2])}));
    assert!(e.contains("no recoverable document"), "{e}");
    assert!(recovery.join(&files[0]).exists());
    assert!(!s.has_document());

    // What can be opened without its record opens as an untitled document.
    let opened = run(&mut s, "recovery.open", json!({"id": id(&files[1])}));
    assert_eq!(opened["path"], Value::Null);
    assert_eq!(opened["features"], 1);
    let opened = run(&mut s, "recovery.open", json!({"id": id(&files[3])}));
    assert_eq!(opened["features"], 1);

    // One is discarded by id, then all the rest; the note stays.
    let discarded = run(&mut s, "recovery.discard", json!({"id": id(&files[0])}));
    assert_eq!(discarded, json!({"discarded": 1, "left": 4}));
    assert!(!recovery.join(&files[0]).exists());
    let e = error(&mut s, "recovery.discard", json!({"id": id(&files[0])}));
    assert!(e.contains("no recoverable document"), "{e}");
    let discarded = run(&mut s, "recovery.discard", json!({"all": true}));
    assert_eq!(
        discarded["left"], 1,
        "the directory cannot be removed as a file"
    );
    assert!(recovery.join("notes.txt").exists());
    // This session's own two adopted documents are still there.
    assert_eq!(
        listing(&dir)
            .iter()
            .filter(|n| n.ends_with(".3dcraft") && !n.starts_with("0123"))
            .count(),
        2
    );
}

#[test]
fn bad_requests_are_errors() {
    let dir = workdir("bad-requests");
    let (mut s, _) = session(&dir);
    for (command, params, words) in [
        ("recovery.list", json!({"all": true}), "unknown parameter"),
        ("recovery.open", json!({}), "missing parameter `id`"),
        ("recovery.open", json!({"id": 7}), "must be a string"),
        (
            "recovery.open",
            json!({"id": ""}),
            "no recoverable document",
        ),
        (
            "recovery.open",
            json!({"id": "../../etc/passwd-1"}),
            "no recoverable document",
        ),
        (
            "recovery.open",
            json!({"id": "0123456789abcdef-1"}),
            "no recoverable document",
        ),
        ("recovery.discard", json!({}), "give `id`"),
        ("recovery.discard", json!({"all": false}), "give `id`"),
        (
            "recovery.discard",
            json!({"id": "0123456789abcdef-1", "all": true}),
            "not both",
        ),
        (
            "recovery.discard",
            json!({"id": "0123456789abcdef-1"}),
            "no recoverable document",
        ),
        ("recovery.discard", json!({"all": "yes"}), "true or false"),
    ] {
        let e = error(&mut s, command, params.clone());
        assert!(e.contains(words), "{command} {params}: {e}");
        assert!(e.starts_with(command), "{e}");
    }
    assert_eq!(listing(&dir), Vec::<String>::new());

    // Without a recovery directory the list is empty and says so; the
    // other two say how to get one.
    let mut plain = Session::new(Workspace::none());
    let listed = run(&mut plain, "recovery.list", json!({}));
    assert_eq!(listed["enabled"], false);
    assert_eq!(listed["documents"], json!([]));
    for command in ["recovery.open", "recovery.discard"] {
        let e = error(&mut plain, command, json!({"id": "0123456789abcdef-1"}));
        assert!(e.contains("no recovery directory"), "{e}");
    }
}

#[cfg(unix)]
#[test]
fn a_directory_that_cannot_be_written_is_a_problem_and_commands_keep_working() {
    use std::os::unix::fs::PermissionsExt;
    let dir = workdir("unwritable");
    let recovery = dir.join("recovery");
    std::fs::create_dir_all(&recovery).unwrap();
    std::fs::set_permissions(&recovery, std::fs::Permissions::from_mode(0o555)).unwrap();
    let writable_anyway = std::fs::write(recovery.join("probe"), b"x").is_ok();
    let (mut s, clock) = session(&dir);
    build_part(&mut s);
    clock.advance(INTERVAL);
    s.autosave_poll();
    if writable_anyway {
        // Run as a user that permissions do not stop: nothing to see.
        std::fs::set_permissions(&recovery, std::fs::Permissions::from_mode(0o755)).unwrap();
        return;
    }
    let overview = s.recovery_overview();
    let problem = overview.problem.clone().unwrap();
    assert!(
        problem.contains("recovery directory") && problem.contains("recovery"),
        "{problem}"
    );
    assert_eq!((overview.writes, overview.pending), (0, 1));
    assert_eq!(overview.last_autosave, None);
    // recovery.list shows it, and commands go on as if nothing happened.
    let listed = run(&mut s, "recovery.list", json!({}));
    assert_eq!(listed["problem"], json!(problem));
    run(&mut s, "feature.block", json!({"size": [1, 1, 1]}));
    run(&mut s, "file.save", json!({"path": "part.3dcraft"}));
    run(&mut s, "feature.block", json!({"size": [2, 1, 1]}));
    assert_eq!(inspected(&mut s)["features"].as_array().unwrap().len(), 6);
    // Not tried again before an interval has passed.
    assert_eq!(s.recovery_overview().writes, 0);

    // The directory becomes writable: the next attempt succeeds and the
    // problem is gone.
    std::fs::set_permissions(&recovery, std::fs::Permissions::from_mode(0o755)).unwrap();
    clock.advance(INTERVAL);
    s.autosave_poll();
    let overview = s.recovery_overview();
    assert_eq!(overview.problem, None);
    assert_eq!((overview.writes, overview.pending), (1, 0));
    assert_eq!(kinds(&dir), ["3dcraft", "json", "lock"]);
}

#[test]
fn a_directory_that_cannot_be_made_is_a_problem_and_commands_keep_working() {
    let dir = workdir("missing");
    // The recovery directory would have to be made inside a file.
    std::fs::write(dir.join("recovery"), b"a file, not a directory").unwrap();
    let (mut s, clock) = session(&dir);
    build_part(&mut s);
    clock.advance(INTERVAL);
    s.autosave_poll();
    let overview = s.recovery_overview();
    let problem = overview.problem.unwrap();
    assert!(problem.contains("recovery directory"), "{problem}");
    assert_eq!(overview.writes, 0);
    run(&mut s, "feature.block", json!({"size": [1, 1, 1]}));
    let listed = run(&mut s, "recovery.list", json!({}));
    assert_eq!(listed["documents"], json!([]));
    assert!(listed["problem"].is_string());
    s.end_recovery();
    assert_eq!(
        std::fs::read(dir.join("recovery")).unwrap(),
        b"a file, not a directory"
    );
}

#[test]
fn after_an_urgent_request_the_last_good_state_is_written_at_once() {
    // What `Session::execute` does after it caught a panic, and what a
    // host does after it caught one outside a command.
    let dir = workdir("urgent");
    let (mut s, _clock) = session(&dir);
    build_part(&mut s);
    assert_eq!(listing(&dir), Vec::<String>::new());
    s.autosave_now();
    assert_eq!(kinds(&dir), ["3dcraft", "json", "lock"]);
    assert_eq!(s.recovery_overview().writes, 1);
    let before = inspected(&mut s);
    drop(s);
    let (mut next, _) = session(&dir);
    let id = s_found(&next)[0].id.clone();
    run(&mut next, "recovery.open", json!({ "id": id }));
    assert_eq!(inspected(&mut next), before);
}

/// A saved part with changes that only the recovery file has, left by a
/// session that died. Returns what the lost session's document was.
fn crash_with_unsaved_changes(dir: &Path) -> Value {
    let (mut s, clock) = session(dir);
    build_part(&mut s);
    run(&mut s, "file.save", json!({"path": "part.3dcraft"}));
    run(&mut s, "file.close", json!({}));
    run(&mut s, "file.open", json!({"path": "part.3dcraft"}));
    run(&mut s, "feature.sphere", json!({"radius": 3}));
    clock.advance(INTERVAL);
    s.autosave_poll();
    inspected(&mut s)
}

#[test]
fn a_recovered_document_is_saved_to_its_original_file_when_that_did_not_change() {
    let dir = workdir("original-same");
    let before = crash_with_unsaved_changes(&dir);
    let original = dir.join("work").join("part.3dcraft");
    let on_disk = std::fs::read(&original).unwrap();

    let (mut s, _) = session(&dir);
    let listed = run(&mut s, "recovery.list", json!({}));
    let entry = &listed["documents"][0];
    assert_eq!(entry["name"], "part.3dcraft");
    assert_eq!(entry["path"], "part.3dcraft");
    let opened = run(&mut s, "recovery.open", json!({"id": entry["id"]}));
    assert_eq!(opened["path"], "part.3dcraft");
    assert_eq!(inspected(&mut s), before);
    // Nothing was written to the original by recovering.
    assert_eq!(std::fs::read(&original).unwrap(), on_disk);

    // It remembers its path: a plain save goes there.
    let saved = run(&mut s, "file.save", json!({}));
    assert_eq!(saved["path"], "part.3dcraft");
    assert_ne!(std::fs::read(&original).unwrap(), on_disk);
    assert_eq!(kinds(&dir), ["lock"]);
    // And it is an ordinary document from then on.
    run(&mut s, "feature.block", json!({"size": [1, 1, 1]}));
    run(&mut s, "file.save", json!({}));
}

#[test]
fn a_recovered_document_is_not_saved_over_an_original_that_changed_since() {
    let dir = workdir("original-changed");
    let before = crash_with_unsaved_changes(&dir);
    let original = dir.join("work").join("part.3dcraft");
    // Somebody else saved another version of the file after the crash.
    let mut other = Session::new(Workspace {
        read: Access::Root(dir.join("work")),
        write: Access::Root(dir.join("work")),
    });
    run(&mut other, "file.new", json!({}));
    run(
        &mut other,
        "feature.torus",
        json!({"major": 10, "minor": 2}),
    );
    run(&mut other, "file.save", json!({"path": "part.3dcraft"}));
    let newer = std::fs::read(&original).unwrap();

    let (mut s, _) = session(&dir);
    let id = s_found(&s)[0].id.clone();
    run(&mut s, "recovery.open", json!({ "id": id }));
    let e = error(&mut s, "file.save", json!({}));
    assert!(
        e.contains("changed on disk") && e.contains(r#"{"overwrite": true}"#),
        "{e}"
    );
    let e = error(&mut s, "file.save", json!({"path": "part.3dcraft"}));
    assert!(e.contains("changed on disk"), "{e}");
    let e = error(&mut s, "file.save", json!({"overwrite": false}));
    assert!(e.contains("changed on disk"), "{e}");
    // Refused: the newer file is untouched, the recovery file is kept and
    // the document is still the recovered one.
    assert_eq!(std::fs::read(&original).unwrap(), newer);
    assert_eq!(kinds(&dir), ["3dcraft", "json", "lock"]);
    assert_eq!(inspected(&mut s), before);

    // Another path needs no confirmation and leaves the newer file alone.
    let mut elsewhere = s.clone();
    run(
        &mut elsewhere,
        "file.save",
        json!({"path": "rescued.3dcraft"}),
    );
    assert_eq!(std::fs::read(&original).unwrap(), newer);

    // Saying so replaces it.
    run(&mut s, "file.save", json!({"overwrite": true}));
    assert_ne!(std::fs::read(&original).unwrap(), newer);
    assert_eq!(kinds(&dir), ["lock"]);
    // The guard is for recovered documents only: afterwards a save is a
    // save.
    run(&mut s, "feature.block", json!({"size": [1, 1, 1]}));
    run(&mut s, "file.save", json!({}));
    let e = error(&mut s, "file.save", json!({"overwrite": "yes"}));
    assert!(e.contains("true or false"), "{e}");
}

#[test]
fn a_recovered_document_whose_original_is_gone_is_saved_there_again() {
    let dir = workdir("original-gone");
    let before = crash_with_unsaved_changes(&dir);
    let original = dir.join("work").join("part.3dcraft");
    std::fs::remove_file(&original).unwrap();
    let (mut s, _) = session(&dir);
    let id = s_found(&s)[0].id.clone();
    run(&mut s, "recovery.open", json!({ "id": id }));
    assert_eq!(inspected(&mut s), before);
    run(&mut s, "file.save", json!({}));
    assert!(original.exists());
}

/// The store's own thread, by the system's clock: a change is on disk a
/// moment later without anyone asking, and a clean end stops the thread.
#[test]
fn the_background_thread_writes_without_being_asked() {
    let dir = workdir("thread");
    let mut s = Session::new(Workspace::none());
    s.enable_recovery(
        RecoveryOptions::new(dir.join("recovery")).with_interval(Duration::from_millis(20)),
    );
    build_part(&mut s);
    let waited = std::time::Instant::now();
    while s.recovery_overview().writes == 0 {
        assert!(
            waited.elapsed() < Duration::from_secs(20),
            "nothing was autosaved: {:?}",
            s.recovery_overview()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    // The thread may be writing a later change now (an atomic write's
    // temporary file, a Gate run of 2026-11-02 saw it): its files once it
    // is done. A temporary file that stays fails here.
    // Done: no temporary file for five intervals of the thread in a row.
    let mut quiet = 0;
    while quiet < 20 && waited.elapsed() < Duration::from_secs(20) {
        quiet = if kinds(&dir).iter().any(|k| k == "tmp") {
            0
        } else {
            quiet + 1
        };
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(kinds(&dir), ["3dcraft", "json", "lock"]);
    let overview = s.recovery_overview();
    assert_eq!(overview.problem, None);
    assert!(overview.last_write_ms.unwrap() >= 0.0);
    s.end_recovery();
    assert_eq!(listing(&dir), Vec::<String>::new());
}

/// A clone of a session is another session: it does not autosave into
/// the original's files, and dropping it ends nothing.
#[test]
fn a_clone_of_a_session_has_no_store() {
    let dir = workdir("clone");
    let (mut s, clock) = session(&dir);
    build_part(&mut s);
    let mut copy = s.clone();
    assert!(!copy.recovery_overview().enabled);
    run(&mut copy, "feature.block", json!({"size": [1, 1, 1]}));
    drop(copy);
    clock.advance(INTERVAL);
    s.autosave_poll();
    assert_eq!(s.recovery_overview().writes, 1);
    assert_eq!(kinds(&dir), ["3dcraft", "json", "lock"]);
}

/// A Jig in use whose parameters were changed is worth a recovery file;
/// it comes back as a model with those values and no file of its own
/// (docs/decisions.md, 2026-10-23).
#[test]
fn a_jig_in_use_comes_back_as_a_model_with_its_values() {
    let dir = workdir("jig-in-use");
    let (mut first, clock) = session(&dir);
    run(&mut first, "file.new", json!({"units": "mm"}));
    run(
        &mut first,
        "variable.set",
        json!({"name": "w", "expr": "40 mm"}),
    );
    run(
        &mut first,
        "feature.block",
        json!({"name": "Plate", "size": ["w", 20, 5]}),
    );
    run(
        &mut first,
        "variable.publish",
        json!({"name": "w", "min": "10 mm", "max": "90 mm"}),
    );
    run(
        &mut first,
        "jig.export",
        json!({"path": "plate.3dcraftjig"}),
    );
    run(&mut first, "file.close", json!({"discard": true}));
    run(&mut first, "file.open", json!({"path": "plate.3dcraftjig"}));
    // Opened and not changed: nothing to keep.
    clock.advance(INTERVAL);
    first.autosave_poll();
    assert!(
        kinds(&dir).iter().all(|k| k != "3dcraft"),
        "{:?}",
        kinds(&dir)
    );
    run(&mut first, "param.set", json!({"values": {"w": 70}}));
    clock.advance(INTERVAL);
    first.autosave_poll();
    drop(first);
    assert!(kinds(&dir).contains(&"3dcraft".to_string()));

    let (mut second, _) = session(&dir);
    let listed = run(&mut second, "recovery.list", json!({}));
    let id = documents(&listed)[0]["id"].as_str().unwrap().to_string();
    let opened = run(&mut second, "recovery.open", json!({ "id": id }));
    assert_eq!(opened["asModel"], true, "{opened}");
    let status = second.document_status().unwrap();
    assert!(!status.jig_in_use && status.path.is_none() && status.dirty);
    let volume = run(&mut second, "document.measure", json!({}))["volume"]
        .as_f64()
        .unwrap();
    assert!((volume - 70.0 * 20.0 * 5.0).abs() < 1e-6, "{volume}");
    // A model: edited as any other.
    run(
        &mut second,
        "variable.set",
        json!({"name": "w", "expr": "50 mm"}),
    );
}

/// An assembly never saved, with an instance, is worth keeping: closing
/// it asks, it is autosaved, and the recovered one reads its sources as
/// it opens, so its instance is placed without an assembly.reload.
#[test]
fn an_unsaved_assembly_asks_is_autosaved_and_comes_back_with_its_sources() {
    let dir = workdir("assembly");
    let (mut first, clock) = session(&dir);
    run(&mut first, "file.new", json!({}));
    run(
        &mut first,
        "feature.block",
        json!({ "name": "Base", "size": [4, 3, 2] }),
    );
    run(&mut first, "file.save", json!({ "path": "block.3dcraft" }));
    run(&mut first, "file.close", json!({}));
    run(&mut first, "assembly.new", json!({}));
    // With an instance it asks.
    run(
        &mut first,
        "instance.add",
        json!({ "source": "block.3dcraft", "name": "A", "position": [10, 0, 0] }),
    );
    let refused = error(&mut first, "file.close", json!({}));
    assert!(refused.contains("discard"), "{refused}");
    clock.advance(INTERVAL);
    first.autosave_poll();
    drop(first);

    let (mut second, _) = session(&dir);
    let listed = run(&mut second, "recovery.list", json!({}));
    let documents = documents(&listed);
    assert_eq!(documents.len(), 1, "{listed}");
    let id = documents[0]["id"].as_str().unwrap().to_string();
    run(&mut second, "recovery.open", json!({ "id": id }));
    let inspected = run(&mut second, "assembly.inspect", json!({ "measures": true }));
    let a = &inspected["instances"][0];
    assert_eq!(a["status"], "ok", "{inspected}");
    let x = a["measured"]["box"][0][0].as_f64().unwrap();
    assert!((x - 10.0).abs() < 1e-6, "{inspected}");
}

/// An example the program carries (the window's start screen,
/// `Session::open_bytes_titled`) is not worth a recovery file while it is
/// only looked at; edited, it is autosaved under its title and comes back
/// under it after a crash. A record written before titles were
/// (2026-11-10) has no `title` and still opens, untitled.
#[test]
fn an_example_is_autosaved_only_once_edited_and_comes_back_under_its_title() {
    let dir = workdir("example");
    let (mut first, clock) = session(&dir);
    run(&mut first, "file.new", json!({}));
    run(&mut first, "feature.block", json!({"size": [30, 10, 8]}));
    run(&mut first, "file.save", json!({"path": "profile.3dcraft"}));
    run(&mut first, "file.close", json!({}));
    let bytes = std::fs::read(dir.join("work").join("profile.3dcraft")).unwrap();
    first
        .open_bytes_titled("t-slot-profile.3dcraft", Some("T-slot profile"), &bytes)
        .unwrap();
    clock.advance(INTERVAL * 2);
    first.autosave_poll();
    assert!(
        kinds(&dir).iter().all(|k| k != "3dcraft"),
        "only looked at: {:?}",
        kinds(&dir)
    );
    assert_eq!(first.recovery_overview().pending, 0);
    run(&mut first, "feature.block", json!({"size": [1, 2, 3]}));
    clock.advance(INTERVAL);
    first.autosave_poll();
    assert_eq!(kinds(&dir), ["3dcraft", "json", "lock"]);
    let record = listing(&dir)
        .into_iter()
        .find(|n| n.ends_with(".json"))
        .unwrap();
    let record_path = dir.join("recovery").join(record);
    let written: Value = serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
    assert_eq!(written["recovery_version"], 1);
    assert_eq!(written["title"], "T-slot profile");
    assert_eq!(written["name"], "T-slot profile");
    assert_eq!(written["path"], Value::Null);
    drop(first);

    let (mut second, clock) = session(&dir);
    let listed = run(&mut second, "recovery.list", json!({}));
    let entry = &documents(&listed)[0];
    assert_eq!(entry["name"], "T-slot profile", "{listed}");
    let id = entry["id"].as_str().unwrap().to_string();
    let opened = run(&mut second, "recovery.open", json!({ "id": id }));
    assert_eq!(opened["title"], "T-slot profile", "{opened}");
    let status = second.document_status().unwrap();
    assert_eq!(status.title.as_deref(), Some("T-slot profile"));
    assert!(status.path.is_none() && status.dirty && status.close_asks);
    let open = run(&mut second, "file.list", json!({}));
    assert_eq!(open["documents"][0]["title"], "T-slot profile");
    assert_eq!(open["documents"][0]["features"], 2);
    // Edited again and crashed again, its record rewritten as a build
    // before titles wrote it: no `title`, named untitled.
    run(&mut second, "feature.block", json!({"size": [2, 2, 2]}));
    clock.advance(INTERVAL);
    second.autosave_poll();
    drop(second);
    let record = listing(&dir)
        .into_iter()
        .find(|n| n.ends_with(".json"))
        .unwrap();
    let record_path = dir.join("recovery").join(record);
    let mut old: Value = serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
    assert_eq!(old["title"], "T-slot profile");
    let map = old.as_object_mut().unwrap();
    map.remove("title");
    map.insert("name".into(), "untitled".into());
    std::fs::write(&record_path, serde_json::to_vec(&old).unwrap()).unwrap();

    let (mut third, _) = session(&dir);
    let listed = run(&mut third, "recovery.list", json!({}));
    let entry = &documents(&listed)[0];
    assert_eq!(entry["name"], "untitled", "{listed}");
    assert_eq!(entry["openable"], true);
    let id = entry["id"].as_str().unwrap().to_string();
    let opened = run(&mut third, "recovery.open", json!({ "id": id }));
    assert_eq!(opened["title"], Value::Null, "{opened}");
    let status = third.document_status().unwrap();
    assert!(status.title.is_none() && status.dirty);
    assert_eq!(
        run(&mut third, "file.list", json!({}))["documents"][0]["features"],
        3
    );
}
