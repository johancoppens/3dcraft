//! Notes pinned to the model (2026-11-17; `crates/engine/src/note_cmds.rs`,
//! `crates/doc/src/note.rs`): the person's and the agents' notes at
//! places of a part, end to end through the commands. The oracles are
//! the plate's own geometry (the centre of its top face, the corners and
//! the middles of its edges), the persistent names the notes are pinned
//! by (rule 4), the bodies regenerated with and without the notes (the
//! same, bit for bit), the files written (a saved file keeps the notes, a
//! published Jig and every export holds none of their text) and the undo
//! history.

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

/// A text no file writes unless it carries the notes.
const MARKER: &str = "pin-marker-7Q: 2 mm thicker here";

/// A plate 40 × 20 × `t` (F1, `Plate`, t = 5 mm) and a cylinder of radius
/// 3 through its middle (F2, `Pin`), not yet subtracted.
fn plate(s: &mut Session) {
    run(s, "file.new", json!({"units": "mm"}));
    run(s, "variable.set", json!({"name": "t", "expr": "5 mm"}));
    run(
        s,
        "feature.block",
        json!({"name": "Plate", "size": [40, 20, "t"]}),
    );
    run(
        s,
        "feature.cylinder",
        json!({"name": "Pin", "radius": 3, "height": "t", "position": [20, 10, 0]}),
    );
}

fn notes(s: &mut Session) -> Value {
    run(s, "note.list", json!({}))
}

fn note(s: &mut Session, id: u64) -> Value {
    notes(s)["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == id)
        .unwrap_or_else(|| panic!("no note {id}"))
        .clone()
}

fn point(value: &Value) -> [f64; 3] {
    let p = value["point"].as_array().unwrap();
    [0, 1, 2].map(|k| p[k].as_f64().unwrap())
}

/// Whether `p` is `q` within a millionth of a millimetre.
fn at(p: [f64; 3], q: [f64; 3]) -> bool {
    p.iter().zip(q).all(|(a, b)| (a - b).abs() < 1e-6)
}

fn as_agent(s: &mut Session, id: &str, params: Value) -> Value {
    s.acting(Some("Claude Code"), None, |s| s.execute(id, &params))
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// A command run as the window's own controls run it for the person.
fn as_person(s: &mut Session, id: &str, params: Value) -> Value {
    s.by_person(|s| s.execute(id, &params))
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn xyz(value: &Value) -> [f64; 3] {
    let p = value.as_array().unwrap();
    [0, 1, 2].map(|k| p[k].as_f64().unwrap())
}

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

#[test]
fn notes_are_pinned_listed_resolved_edited_and_deleted() {
    let mut s = session();
    plate(&mut s);
    // On a face by its name, the feature written by its name: at the
    // centre of the top face.
    let top = as_person(
        &mut s,
        "note.add",
        json!({"text": " 2 mm thicker here ", "on": "Plate:face(1)"}),
    );
    assert_eq!(top["id"], 1);
    assert_eq!(top["text"], "2 mm thicker here");
    assert_eq!(top["state"], "open");
    assert_eq!(top["author"], "person");
    assert_eq!(top["on"]["kind"], "face");
    assert_eq!(top["on"]["name"], "F1:face(1)");
    assert_eq!(top["on"]["feature"], "F1");
    assert_eq!(top["on"]["body"], "Plate");
    assert_eq!(top["on"]["origin"]["name"], "Plate");
    assert_eq!(top["attachment"], "attached");
    let [x, y, z] = point(&top);
    assert!(
        (x - 20.0).abs() < 1e-9 && (y - 10.0).abs() < 1e-9 && (z - 5.0).abs() < 1e-9,
        "{top}"
    );
    assert!(
        top["summary"]
            .as_str()
            .unwrap()
            .contains("face F1:face(1) of Plate"),
        "{top}"
    );
    // A point given on it is kept as given.
    let given = run(
        &mut s,
        "note.add",
        json!({"text": "here", "on": "F1:face(1)", "point": [5, 5, 5]}),
    );
    assert_eq!(point(&given), [5.0, 5.0, 5.0]);
    // An edge at its middle, a vertex at itself: on the plate's box, two
    // coordinates at a bound for the edge, three for the corner.
    let at_bounds = |p: [f64; 3]| {
        let bounds = [(0.0, 40.0), (0.0, 20.0), (0.0, 5.0)];
        p.iter()
            .zip(bounds)
            .filter(|(c, (lo, hi))| (**c - lo).abs() < 1e-9 || (**c - hi).abs() < 1e-9)
            .count()
    };
    let edge = run(
        &mut s,
        "note.add",
        json!({"text": "sharp?", "on": "F1:edge(0)"}),
    );
    assert_eq!(edge["on"]["kind"], "edge");
    assert_eq!(at_bounds(point(&edge)), 2, "{edge}");
    let corner = run(
        &mut s,
        "note.add",
        json!({"text": "this corner", "on": "F1:vertex(0)"}),
    );
    assert_eq!(corner["on"]["kind"], "vertex");
    assert_eq!(at_bounds(point(&corner)), 3, "{corner}");
    // A body by its feature: on its surface.
    let body = run(
        &mut s,
        "note.add",
        json!({"text": "in aluminium", "on": "Pin"}),
    );
    assert_eq!(body["on"]["kind"], "body");
    assert_eq!(body["on"]["name"], "Pin");
    let [x, y, z] = point(&body);
    let radial = ((x - 20.0) * (x - 20.0) + (y - 10.0) * (y - 10.0)).sqrt();
    assert!(
        (radial - 3.0).abs() < 0.05 || z.abs() < 1e-9 || (z - 5.0).abs() < 1e-9,
        "{body}"
    );
    // Without on: the one thing selected.
    run(&mut s, "select.set", json!({"names": ["F1:face(2)"]}));
    let selected = run(&mut s, "note.add", json!({"text": "front"}));
    assert_eq!(selected["on"]["name"], "F1:face(2)");
    run(
        &mut s,
        "select.set",
        json!({"names": ["F1:face(2)", "F1:face(3)"]}),
    );
    let e = error(&mut s, "note.add", json!({"text": "which?"}));
    assert!(e.contains("2 faces are selected"), "{e}");
    run(&mut s, "select.set", json!({"mode": "clear"}));
    let e = error(&mut s, "note.add", json!({"text": "where?"}));
    assert!(e.contains("nothing is selected"), "{e}");
    // An agent's note: its question at a place.
    let asked = as_agent(
        &mut s,
        "note.add",
        json!({"text": "This pin touches the plate's middle: is that meant?", "on": "F2:face(0)"}),
    );
    assert_eq!(asked["author"], "agent");
    assert_eq!(asked["agent"], "Claude Code");
    let listed = notes(&mut s);
    assert_eq!(listed["count"], 7);
    assert_eq!(listed["open"], 7);
    assert_eq!(listed["fromAgents"], 1);
    assert_eq!(listed["detached"], 0);
    assert!(
        listed["summary"]
            .as_str()
            .unwrap()
            .starts_with("7 notes: 7 open (1 from an agent), 0 done"),
        "{listed}"
    );
    let inspected = run(&mut s, "document.inspect", json!({"measures": false}));
    assert_eq!(
        inspected["notes"],
        json!({"count": 7, "open": 7, "fromAgents": 1})
    );

    // Resolved by an agent with a reply, which its log shows.
    let done = as_agent(
        &mut s,
        "note.resolve",
        json!({"id": 1, "reply": "Done: t is 7 mm now."}),
    );
    assert_eq!(done["state"], "done");
    assert_eq!(done["done"]["author"], "agent");
    assert_eq!(done["done"]["agent"], "Claude Code");
    assert_eq!(done["done"]["reply"], "Done: t is 7 mm now.");
    let log = run(&mut s, "agent.log", json!({"limit": 1}));
    assert_eq!(log["entries"][0]["command"], "note.resolve");
    assert_eq!(
        log["entries"][0]["message"],
        "Note 1 done: Done: t is 7 mm now."
    );
    let e = error(&mut s, "note.resolve", json!({"id": 1}));
    assert!(e.contains("done already (by Claude Code)"), "{e}");
    let e = error(
        &mut s,
        "note.resolve",
        json!({"id": 1, "done": false, "reply": "x"}),
    );
    assert!(e.contains("done: false alone"), "{e}");
    assert_eq!(
        run(&mut s, "note.list", json!({"state": "done"}))["notes"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        run(&mut s, "note.list", json!({"state": "open"}))["notes"]
            .as_array()
            .unwrap()
            .len(),
        6
    );
    let reopened = run(&mut s, "note.resolve", json!({"id": 1, "done": false}));
    assert_eq!(reopened["state"], "open");
    assert!(reopened.get("done").is_none(), "{reopened}");
    let e = error(&mut s, "note.resolve", json!({"id": 1, "done": false}));
    assert!(e.contains("note 1 is open"), "{e}");
    // Closed over a channel that says no one: unknown, never the person.
    run(&mut s, "note.resolve", json!({"id": 2}));
    assert_eq!(note(&mut s, 2)["done"]["author"], "unknown");
    assert_eq!(note(&mut s, 2)["author"], "unknown");

    // Edited: its text; pinned elsewhere, at the new entity's middle;
    // moved on what it is on.
    let edited = as_agent(
        &mut s,
        "note.edit",
        json!({"id": 3, "text": "round it, 1 mm"}),
    );
    assert_eq!(edited["text"], "round it, 1 mm");
    assert_eq!(edited["author"], "unknown", "who wrote it stays");
    assert_eq!(edited["edited"]["author"], "agent");
    assert_eq!(edited["edited"]["agent"], "Claude Code");
    assert_eq!(edited["on"]["kind"], "edge", "pinned as before");
    let moved = run(&mut s, "note.edit", json!({"id": 3, "on": "F1:face(0)"}));
    assert_eq!(moved["on"]["name"], "F1:face(0)");
    assert!(at(point(&moved), [20.0, 10.0, 0.0]), "{moved}");
    let moved = run(&mut s, "note.edit", json!({"id": 3, "point": [1, 1, 0]}));
    assert_eq!(point(&moved), [1.0, 1.0, 0.0]);
    let e = error(&mut s, "note.edit", json!({"id": 3, "point": [1, 1, 900]}));
    assert!(e.contains("from the face"), "{e}");
    let e = error(&mut s, "note.edit", json!({"id": 3}));
    assert!(e.contains("give what changes"), "{e}");

    // Deleted: one, several, the done ones; ids are never given out again.
    let e = error(&mut s, "note.delete", json!({"id": [4, 99]}));
    assert!(e.contains("there is no note 99"), "{e}");
    assert_eq!(notes(&mut s)["count"], 7, "none went");
    let gone = run(&mut s, "note.delete", json!({"id": [4, 5]}));
    assert_eq!(gone["deleted"], json!([4, 5]));
    let gone = run(&mut s, "note.delete", json!({"state": "done"}));
    assert_eq!(gone["deleted"], json!([2]));
    let gone = run(&mut s, "note.delete", json!({"id": 7}));
    assert_eq!(gone["deleted"], json!([7]));
    let later = run(&mut s, "note.add", json!({"text": "new", "on": "Plate"}));
    assert_eq!(later["id"], 8, "7 was given out already");
    let gone = run(&mut s, "note.delete", json!({"state": "all"}));
    assert_eq!(gone["deleted"], json!([1, 3, 6, 8]));
    assert_eq!(notes(&mut s)["summary"], "no notes");
    let none = run(&mut s, "note.delete", json!({"state": "done"}));
    assert_eq!(none["deleted"], json!([]));
}

#[test]
fn hostile_notes_are_refused_and_change_nothing() {
    let mut s = session();
    plate(&mut s);
    run(
        &mut s,
        "note.add",
        json!({"text": "kept", "on": "F1:face(1)"}),
    );
    let before = s.document_revision();
    let megabyte = "x".repeat(1 << 20);
    for (params, says) in [
        (json!({"on": "F1:face(1)"}), "missing parameter `text`"),
        (json!({"text": "", "on": "F1:face(1)"}), "must not be empty"),
        (
            json!({"text": " \n ", "on": "F1:face(1)"}),
            "must not be empty",
        ),
        (
            json!({"text": megabyte, "on": "F1:face(1)"}),
            "at most 2000 Unicode code points",
        ),
        (
            json!({"text": "a\u{7}b", "on": "F1:face(1)"}),
            "control character U+0007",
        ),
        (json!({"text": "x", "on": "F99:face(0)"}), "F99"),
        (
            json!({"text": "x", "on": "F1:face(99)"}),
            "no body has a face",
        ),
        (json!({"text": "x", "on": "Nothing"}), "Nothing"),
        (
            json!({"text": "x", "on": "F1:face(1)", "point": [null, 0, 0]}),
            "three finite numbers",
        ),
        (
            json!({"text": "x", "on": "F1:face(1)", "point": [1, 2]}),
            "three finite numbers",
        ),
        (
            json!({"text": "x", "on": "F1:face(1)", "point": [1e6, 0, 0]}),
            "from the face",
        ),
        (
            json!({"text": "x", "on": "F1:vertex(0)", "point": [0, 0, 1000]}),
            "from the vertex",
        ),
        (json!({"text": "x", "feature": "Plate"}), "give `on` too"),
        (
            json!({"text": "x", "on": "F1:face(1)", "feature": "Pin"}),
            "no body has a face",
        ),
        (
            json!({"text": "x", "on": "F1:face(1)", "colour": "red"}),
            "unknown parameter",
        ),
    ] {
        let e = error(&mut s, "note.add", params.clone());
        assert!(e.contains(says), "{params}: {e}");
    }
    for (id, params, says) in [
        (
            "note.resolve",
            json!({"id": 9}),
            "there is no note 9; notes: 1",
        ),
        ("note.resolve", json!({"id": 0}), "whole number from 1"),
        ("note.resolve", json!({"id": "1"}), "whole number from 1"),
        (
            "note.resolve",
            json!({"id": 1, "reply": "a\0b"}),
            "reply must not hold",
        ),
        (
            "note.edit",
            json!({"id": 1, "text": ""}),
            "must not be empty",
        ),
        (
            "note.edit",
            json!({"id": 9, "text": "x"}),
            "there is no note 9",
        ),
        (
            "note.edit",
            json!({"id": 1, "point": [1e300, 0, 0]}),
            "from the face",
        ),
        ("note.delete", json!({}), "give `id`"),
        ("note.delete", json!({"id": 1, "state": "all"}), "not both"),
        ("note.delete", json!({"id": []}), "1 to 1000"),
        ("note.delete", json!({"id": [1, -1]}), "a note's id"),
        ("note.delete", json!({"state": "open"}), "done or all"),
        ("note.list", json!({"state": "closed"}), "open, done or all"),
    ] {
        let e = error(&mut s, id, params.clone());
        assert!(e.contains(says), "{id} {params}: {e}");
    }
    assert_eq!(s.document_revision(), before, "nothing changed");
    assert_eq!(notes(&mut s)["count"], 1);
    // In an assembly: not yet.
    run(&mut s, "assembly.new", json!({}));
    let e = error(&mut s, "note.add", json!({"text": "x", "on": "F1:face(1)"}));
    assert!(e.contains("not yet in an assembly"), "{e}");
    assert_eq!(notes(&mut s)["count"], 0);
}

/// Rule 4: a note follows the name it is pinned by. A size changed
/// upstream keeps the face, and the note on it; a boolean that takes the
/// body takes the note along to the face's image; a suppressed feature
/// hides it; a deleted one detaches it, at its point, until an undo
/// brings the face back.
#[test]
fn a_note_stays_on_its_face_through_edits_and_is_detached_when_it_goes() {
    let mut s = session();
    plate(&mut s);
    run(
        &mut s,
        "note.add",
        json!({"text": "thicker", "on": "F1:face(1)"}),
    );
    run(
        &mut s,
        "note.add",
        json!({"text": "smaller hole", "on": "F2:face(0)"}),
    );
    let wall = note(&mut s, 2);
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Plate", "tool": "Pin", "name": "Drilled"}),
    );
    for id in [1, 2] {
        let n = note(&mut s, id);
        assert_eq!(n["attachment"], "attached", "{n}");
        assert_eq!(
            n["now"][0]["feature"], "F3",
            "followed into the boolean: {n}"
        );
        assert_eq!(n["now"][0]["body"], "Drilled");
        assert_eq!(n["now"][0]["pieces"], 1);
    }
    assert!(
        note(&mut s, 1)["now"][0]["name"]
            .as_str()
            .unwrap()
            .ends_with("F1:face(1)"),
        "the top's image"
    );
    // The thickness changed: the same faces, the notes on them.
    run(&mut s, "variable.set", json!({"name": "t", "expr": "7 mm"}));
    for id in [1, 2] {
        let n = note(&mut s, id);
        assert_eq!(n["attachment"], "attached", "{n}");
        assert_eq!(
            n["on"]["name"],
            if id == 1 { "F1:face(1)" } else { "F2:face(0)" }
        );
    }
    assert!(
        at(point(&note(&mut s, 1)), [20.0, 10.0, 5.0]),
        "where it was put"
    );
    // Its marker is on the top as it is now, 7 mm up: the point of the
    // face nearest where it was put, the middle of the plate, which the
    // hole drilled since has taken: so on the hole's rim (radius 3, on a
    // mesh of chords within a thousandth of the plate's size).
    let top = note(&mut s, 1);
    let marker = top["at"].as_array().unwrap();
    let [x, y, z] = [0, 1, 2].map(|k| marker[k].as_f64().unwrap());
    let radial = ((x - 20.0) * (x - 20.0) + (y - 10.0) * (y - 10.0)).sqrt();
    assert!(
        (z - 7.0).abs() < 1e-9 && radial > 3.0 - 0.05 && radial <= 3.0 + 1e-9,
        "{top}"
    );
    // Suppressed: hidden for now.
    run(&mut s, "feature.suppress", json!({"feature": "Pin"}));
    let hidden = note(&mut s, 2);
    assert_eq!(hidden["attachment"], "hidden", "{hidden}");
    assert!(
        hidden["why"].as_str().unwrap().contains("suppressed"),
        "{hidden}"
    );
    run(
        &mut s,
        "feature.suppress",
        json!({"feature": "Pin", "suppressed": false}),
    );
    assert_eq!(note(&mut s, 2)["attachment"], "attached");
    // Deleted: detached, kept at its point, and said so in the list.
    run(&mut s, "feature.delete", json!({"feature": "Pin"}));
    let detached = note(&mut s, 2);
    assert_eq!(detached["attachment"], "detached", "{detached}");
    assert!(
        detached["why"].as_str().unwrap().contains("F2"),
        "{detached}"
    );
    assert_eq!(detached["point"], wall["point"]);
    assert_eq!(detached["text"], "smaller hole");
    assert!(detached.get("at").is_none(), "no marker on what is gone");
    let listed = notes(&mut s);
    assert_eq!(listed["detached"], 1);
    assert!(
        listed["summary"].as_str().unwrap().ends_with("1 detached"),
        "{listed}"
    );
    // Moved anywhere while detached; then the undo of the move and of the
    // deletion pins it to its face again.
    run(
        &mut s,
        "note.edit",
        json!({"id": 2, "point": [100, 100, 100]}),
    );
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(note(&mut s, 2)["point"], wall["point"]);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(note(&mut s, 2)["attachment"], "attached");
}

/// Notes are beside the history: adding one rebuilds no feature, and the
/// document regenerates from nothing to the same bodies, bit for bit,
/// with its notes and without them.
#[test]
fn notes_change_no_body_and_rebuild_nothing() {
    let mut s = session();
    plate(&mut s);
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Plate", "tool": "Pin"}),
    );
    run(&mut s, "document.inspect", json!({}));
    let before = run(&mut s, "document.inspect", json!({}))["bodies"].clone();
    run(&mut s, "note.add", json!({"text": MARKER, "on": "F3"}));
    as_agent(
        &mut s,
        "note.add",
        json!({"text": "why so thin?", "on": "Plate:face(1)"}),
    );
    run(&mut s, "note.resolve", json!({"id": 1, "reply": "kept"}));
    let after = run(&mut s, "document.inspect", json!({}));
    let (rebuilt, reused) = s.last_regeneration().unwrap();
    assert_eq!(rebuilt, 0, "a note rebuilds nothing ({reused} reused)");
    assert_eq!(after["bodies"], before);
    let with = craft3d_format::from_bytes(&s.document_bytes().unwrap()).unwrap();
    assert_eq!(with.notes.items.len(), 2);
    let (without, count) = with.without_notes();
    assert_eq!(count, 2);
    let a = craft3d_regen::regenerate(&with);
    let b = craft3d_regen::regenerate(&without);
    assert_eq!(a.features.len(), b.features.len());
    for (x, y) in a.features.iter().zip(&b.features) {
        assert_eq!(x.id, y.id);
        assert_eq!(x.status, y.status);
        assert_eq!(x.body, y.body, "the body of {}", x.id);
    }
}

/// Each note command is one undo step, and redo brings it back.
#[test]
fn every_note_command_is_one_undo_step() {
    let mut s = session();
    plate(&mut s);
    let steps: [(&str, Value); 5] = [
        ("note.add", json!({"text": "a", "on": "F1:face(1)"})),
        ("note.resolve", json!({"id": 1, "reply": "b"})),
        ("note.resolve", json!({"id": 1, "done": false})),
        (
            "note.edit",
            json!({"id": 1, "text": "c", "point": [1, 1, 5]}),
        ),
        ("note.delete", json!({"id": 1})),
    ];
    let mut states = vec![s.document_bytes().unwrap()];
    for (id, params) in &steps {
        run(&mut s, id, params.clone());
        states.push(s.document_bytes().unwrap());
    }
    for k in (0..steps.len()).rev() {
        run(&mut s, "edit.undo", json!({}));
        assert!(
            s.document_bytes().unwrap() == states[k],
            "undo of {}",
            steps[k].0
        );
    }
    for (k, (id, _)) in steps.iter().enumerate() {
        run(&mut s, "edit.redo", json!({}));
        assert!(s.document_bytes().unwrap() == states[k + 1], "redo of {id}");
    }
}

/// A saved file keeps the notes and opens with them; a published Jig
/// holds none (and the design keeps its own); no export carries their
/// text.
#[test]
fn saving_keeps_the_notes_and_publishing_and_exports_leave_them_out() {
    let dir = workdir("notes", "files");
    let mut s = session_in(&dir);
    plate(&mut s);
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Plate", "tool": "Pin", "name": "Drilled"}),
    );
    run(&mut s, "note.add", json!({"text": MARKER, "on": "F3"}));
    as_agent(
        &mut s,
        "note.add",
        json!({"text": "pin-marker-7Q: is the hole meant to go through?", "on": "F2:face(0)"}),
    );
    as_agent(
        &mut s,
        "note.resolve",
        json!({"id": 1, "reply": "pin-marker-7Q reply"}),
    );
    let listed = notes(&mut s);
    run(&mut s, "file.save", json!({"path": "noted.3dcraft"}));
    let saved = std::fs::read(dir.join("noted.3dcraft")).unwrap();
    assert!(String::from_utf8_lossy(&saved).contains(MARKER));
    let mut t = session_in(&dir);
    run(&mut t, "file.open", json!({"path": "noted.3dcraft"}));
    let reopened = notes(&mut t);
    let strip = |v: &Value| {
        let mut v = v.clone();
        // The summary says the same; the time stamps were written once.
        for n in v["notes"].as_array_mut().unwrap() {
            assert!(n["timeMs"].as_u64().is_some());
        }
        v
    };
    assert_eq!(strip(&reopened), strip(&listed));

    // Published as a Jig: without its notes, the design keeps them.
    run(
        &mut s,
        "variable.publish",
        json!({"name": "t", "label": "Thickness", "min": "2 mm", "max": "20 mm"}),
    );
    let exported = run(&mut s, "jig.export", json!({"path": "noted.3dcraftjig"}));
    assert_eq!(exported["notesLeftOut"], 2, "{exported}");
    let jig = std::fs::read(dir.join("noted.3dcraftjig")).unwrap();
    assert!(!String::from_utf8_lossy(&jig).contains("pin-marker-7Q"));
    let published = craft3d_format::from_bytes(&jig).unwrap();
    assert!(published.notes.is_empty() && published.notes == Default::default());
    assert_eq!(published.features.len(), 3);
    assert_eq!(notes(&mut s)["count"], 2, "the design keeps its notes");

    // No export carries them.
    for (path, extra) in [
        ("noted.step", json!({})),
        ("noted.stl", json!({})),
        ("noted.3mf", json!({})),
        ("noted.obj", json!({})),
        ("noted.dxf", json!({})),
        ("noted.svg", json!({})),
        ("top.dxf", json!({"face": "Plate:face(1)"})),
    ] {
        let mut params = json!({"path": path});
        for (k, v) in extra.as_object().unwrap() {
            params[k] = v.clone();
        }
        run(&mut s, "file.export", params);
        let bytes = std::fs::read(dir.join(path)).unwrap();
        assert!(
            !String::from_utf8_lossy(&bytes).contains("pin-marker-7Q"),
            "{path} carries a note"
        );
    }
}

/// Rule 4 (review-pins R1): a name not proven to stay with its entity (a
/// fillet's face is numbered by its edge's place among the fillet's
/// references) never holds a note. The note is taken and held by place:
/// by the fillet that made the face and its point. The fillet given one
/// edge fewer, the face once numbered 0 is another corner's; the note
/// does not say attached, does not name an entity it cannot prove, and
/// its marker stays on what the fillet made nearest its point.
#[test]
fn a_note_on_an_unproven_name_is_held_by_place() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": [40, 20, 10]}),
    );
    run(
        &mut s,
        "feature.fillet",
        json!({"edges": ["F1:edge(9)", "F1:edge(8)"], "radius": 3, "name": "Round"}),
    );
    let added = run(
        &mut s,
        "note.add",
        json!({"text": "this corner smaller", "on": "F2:fillet(0)"}),
    );
    assert_eq!(added["attachment"], "byPlace", "{added}");
    assert!(added["on"]["name"].is_null(), "{added}");
    assert_eq!(added["on"]["pinnedAs"], "F2:fillet(0)");
    assert_eq!(added["on"]["origin"]["name"], "Round");
    assert!(added.get("now").is_none(), "{added}");
    assert!(
        added["why"].as_str().unwrap().contains("held by `Round`"),
        "{added}"
    );
    let point = point(&added);
    assert!(distance(xyz(&added["at"]), point) < 1e-6, "{added}");
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Round", "edges": ["F1:edge(8)"]}),
    );
    let n = note(&mut s, 1);
    // The fillet's one face left is the other corner, 36 mm off: not the
    // note's place (the coordinator's decision of round 1c): its place is
    // lost, no marker on another corner, no entity named.
    assert_eq!(n["attachment"], "lost", "{n}");
    assert!(n.get("now").is_none(), "{n}");
    assert!(n.get("at").is_none(), "{n}");
    assert_eq!(xyz(&n["point"]), point, "it keeps its point");
    // The fillet deleted: nothing it made is there; detached.
    run(&mut s, "feature.delete", json!({"feature": "Round"}));
    assert_eq!(note(&mut s, 1)["attachment"], "detached");
}

/// Review-pins R2: a face a later feature cut in two keeps the note on
/// both pieces; `now` says there are two and the marker is on the piece
/// nearest the note's point.
#[test]
fn a_face_cut_in_two_keeps_the_note_on_the_piece_it_is_near() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": [40, 20, 10]}),
    );
    run(
        &mut s,
        "note.add",
        json!({"text": "right", "on": "Plate:face(1)", "point": [35, 10, 10]}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Groove", "size": [4, 22, 10], "position": [18, -1, 3]}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Plate", "tool": "Groove"}),
    );
    let n = note(&mut s, 1);
    assert_eq!(n["attachment"], "attached", "{n}");
    assert_eq!(n["now"][0]["pieces"], 2, "{n}");
    assert!(distance(xyz(&n["at"]), [35.0, 10.0, 10.0]) < 1e-6, "{n}");
}

/// Review-pins R3: a vertex stays pinned through a later feature that
/// takes its body and keeps it (a hole far from it), followed by where it
/// lies.
#[test]
fn a_vertex_note_follows_its_corner_into_later_features() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": [40, 20, 5]}),
    );
    run(
        &mut s,
        "note.add",
        json!({"text": "corner", "on": "F1:vertex(0)"}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Hole", "radius": 2, "height": 5, "position": [30, 15, 0]}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Plate", "tool": "Hole", "name": "Holed"}),
    );
    let n = note(&mut s, 1);
    assert_eq!(n["attachment"], "attached", "{n}");
    assert_eq!(n["now"][0]["feature"], "F3", "{n}");
    assert!(distance(xyz(&n["at"]), point(&n)) < 1e-9, "{n}");
}

/// Review-pins R4: a body subtracted as a boolean's tool is no material
/// of the result: a note on it is detached, not attached to the plate.
#[test]
fn a_note_on_a_subtracted_tool_is_detached() {
    let mut s = session();
    plate(&mut s);
    run(
        &mut s,
        "note.add",
        json!({"text": "pin in brass", "on": "Pin"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Plate", "tool": "Pin", "name": "Drilled"}),
    );
    let n = note(&mut s, 1);
    assert_eq!(n["attachment"], "detached", "{n}");
    assert!(n["why"].as_str().unwrap().contains("subtracted"), "{n}");
    // Undone, the pin is its own body again, and the note on it.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(note(&mut s, 1)["attachment"], "attached");
}

/// Review-pins R5: every face of a bundled example takes a note, placed
/// on the face's surface within its bounds (the ship propeller's blades,
/// whose coarse mesh could not be made).
#[test]
fn every_face_of_the_ship_propeller_takes_a_note() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/examples/published");
    let dir = workdir("notes", "propeller");
    std::fs::copy(
        root.join("ship-propeller.3dcraft"),
        dir.join("ship-propeller.3dcraft"),
    )
    .unwrap();
    let mut s = session_in(&dir);
    run(
        &mut s,
        "file.open",
        json!({"path": "ship-propeller.3dcraft"}),
    );
    let inspected = run(&mut s, "document.inspect", json!({"measures": false}));
    let mut placed = 0;
    for body in inspected["bodies"].as_array().unwrap() {
        let feature = body["feature"].as_str().unwrap().to_string();
        for face in body["faceNames"].as_array().unwrap() {
            let face = face.as_str().unwrap();
            let added = s.execute(
                "note.add",
                &json!({"text": "here", "on": face, "feature": feature}),
            );
            let added = added.unwrap_or_else(|e| panic!("{face}: {e}"));
            assert!(added["point"].is_array(), "{added}");
            placed += 1;
        }
    }
    assert!(placed >= 48, "{placed} faces");
}

/// Review-pins 1b-R1 (rule 4): a vertex is followed by its name, never by
/// where something lies. The plate moved 40 mm in −x, or mirrored in its
/// middle plane, brings another corner to where the note's corner was:
/// the note follows its own corner (`F2:copy(0)/F1:vertex(0)`), at its
/// new place.
#[test]
fn a_vertex_note_follows_its_own_corner_through_a_move_and_a_mirror() {
    for (id, params) in [
        (
            "feature.transform",
            json!({"source": "Plate", "translate": [-40, 0, 0]}),
        ),
        (
            "feature.mirror",
            json!({"source": "Plate", "normal": [1, 0, 0], "point": [20, 0, 0]}),
        ),
    ] {
        let mut s = session();
        run(&mut s, "file.new", json!({"units": "mm"}));
        run(
            &mut s,
            "feature.block",
            json!({"name": "Plate", "size": [40, 20, 5]}),
        );
        run(
            &mut s,
            "note.add",
            json!({"text": "corner", "on": "F1:vertex(0)"}),
        );
        let put = point(&note(&mut s, 1));
        run(&mut s, id, params);
        let n = note(&mut s, 1);
        assert_eq!(n["attachment"], "attached", "{id}: {n}");
        for now in n["now"].as_array().unwrap() {
            assert!(
                now["name"].as_str().unwrap().ends_with("F1:vertex(0)"),
                "{id}: another corner: {n}"
            );
        }
        // Where the corner went: moved 40 in −x, or mirrored to x = 40.
        let at = xyz(&n["at"]);
        let want = if id == "feature.transform" {
            [put[0] - 40.0, put[1], put[2]]
        } else {
            [40.0 - put[0], put[1], put[2]]
        };
        assert!(distance(at, want) < 1e-9, "{id}: {n}");
    }
}

/// Review-pins 1b-R2: a note held by place is drawn on what its feature
/// makes only near its point; a fillet given one edge fewer makes nothing
/// there, so the note has no `at`, says its place is lost, and keeps its
/// point.
#[test]
fn a_note_held_by_place_whose_place_is_gone_says_so() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": [40, 20, 10]}),
    );
    run(
        &mut s,
        "feature.fillet",
        json!({"edges": ["F1:edge(9)", "F1:edge(8)"], "radius": 3, "name": "Round"}),
    );
    let added = run(
        &mut s,
        "note.add",
        json!({"text": "this corner smaller", "on": "F2:fillet(0)"}),
    );
    assert_eq!(added["attachment"], "byPlace", "{added}");
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Round", "edges": ["F1:edge(8)"]}),
    );
    let n = note(&mut s, 1);
    let size = (40.0f64 * 40.0 + 20.0 * 20.0 + 10.0 * 10.0).sqrt();
    match n.get("at") {
        Some(at) => assert!(
            distance(xyz(at), point(&n)) <= 0.02 * size + 1e-6,
            "drawn on another corner: {n}"
        ),
        None => {
            assert_eq!(n["attachment"], "lost", "{n}");
            assert!(
                n["why"].as_str().unwrap().contains("its place is gone"),
                "{n}"
            );
        }
    }
    assert_eq!(point(&n), point(&added), "the note keeps its point");
}

/// A note on a sheet of tens of thousands of spans costs what placing on
/// its surface costs, no mesh of it (review-pins 1b: 6.8 s for the first
/// note on a 185 × 185 grid, its mesh).
#[test]
fn a_note_on_a_large_grid_sheet_needs_no_mesh() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    let n = 185;
    let points: Vec<Vec<[f64; 3]>> = (0..n)
        .map(|i| {
            (0..n)
                .map(|j| {
                    let (x, y) = (f64::from(i), f64::from(j));
                    [x, y, 0.3 * f64::from((i * 7 + j * 5) % 11) / 11.0]
                })
                .collect()
        })
        .collect();
    run(
        &mut s,
        "feature.surface.grid",
        json!({"points": points, "name": "G"}),
    );
    run(&mut s, "document.inspect", json!({"measures": false}));
    let started = std::time::Instant::now();
    let added = run(&mut s, "note.add", json!({"text": "sheet", "on": "G"}));
    let took = started.elapsed();
    println!("the first note on a 185 x 185 grid sheet: {took:?}");
    assert!(added["point"].is_array(), "{added}");
    let started = std::time::Instant::now();
    run(&mut s, "note.list", json!({}));
    println!("note.list after it: {:?}", started.elapsed());
}
