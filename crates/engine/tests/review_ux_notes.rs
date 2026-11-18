//! The review of the UX round's notes (review-ux-notes, 2026-11-18, on
//! `check/ux-round` 5d8452a7): `note.reply` from every side (an open, a
//! done, a detached and a lost note, one that does not exist; a person, a
//! named, an unnamed and a "you" agent, a script), its caps (2000 code
//! points, 20 replies, 2 MiB of note text a document) and the text it
//! keeps; whose turn it is after each kind of word, the same in
//! `note.list` and `Session::notes_json`; a file loaded with more than the
//! caps allow; a published Jig without the thread. The tests marked
//! `ignore` are the review's findings: they fail on 5d8452a7 and pass once
//! the finding is mended (`cargo test -p craft3d-engine --test
//! review_ux_notes -- --ignored`).

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
use craft3d_engine::agent_log::UNNAMED_AGENT;
use serde_json::{Value, json};

/// The most bytes of note text a document holds (`MAX_NOTES_BYTES`).
const NOTES_BYTES: usize = 2 * 1024 * 1024;

/// A plate 40 × 20 × 10 (F1, `Plate`) rounded along two top edges (F2,
/// `Round`) and a lug beside it (F3, `Lug`).
fn plate(s: &mut Session) {
    run(s, "file.new", json!({"units": "mm"}));
    run(
        s,
        "feature.block",
        json!({"name": "Plate", "size": [40, 20, 10]}),
    );
    run(
        s,
        "feature.fillet",
        json!({"edges": ["F1:edge(9)", "F1:edge(8)"], "radius": 3, "name": "Round"}),
    );
    run(
        s,
        "feature.block",
        json!({"name": "Lug", "size": [10, 10, 10], "position": [60, 5, 0]}),
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

fn by(s: &mut Session, agent: Option<&str>, id: &str, params: Value) -> Result<Value, String> {
    match agent {
        Some(name) => s.acting(Some(name), None, |s| s.execute(id, &params)),
        None => s.by_person(|s| s.execute(id, &params)),
    }
    .map_err(|e| e.to_string())
}

fn person(s: &mut Session, id: &str, params: Value) -> Value {
    by(s, None, id, params.clone()).unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn agent(s: &mut Session, name: &str, id: &str, params: Value) -> Value {
    by(s, Some(name), id, params.clone()).unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// The texts of a note's replies, in order.
fn texts(note: &Value) -> Vec<String> {
    note["replies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["text"].as_str().unwrap().to_string())
        .collect()
}

/// A clock step: the next command's time is later than the last one's
/// (`timeMs` is in milliseconds).
fn tick() {
    std::thread::sleep(std::time::Duration::from_millis(3));
}

/// `note.reply` on every kind of note, by every kind of author: an open
/// note, a done one (it stays done, its closing kept), a detached one and
/// one whose place is lost (their attachment as it was); by the person, a
/// named agent, an agent without a name, an agent that calls itself "you"
/// and a script; one undo step each, "Reply to note N"; a note that does
/// not exist names the notes there are.
#[test]
fn note_reply_on_every_kind_of_note_by_every_kind_of_author() {
    let mut s = session();
    plate(&mut s);
    person(
        &mut s,
        "note.add",
        json!({"text": "2 mm thicker here", "on": "Plate:face(1)"}),
    );
    agent(
        &mut s,
        "Claude Code",
        "note.add",
        json!({"text": "Should this side be drafted?", "on": "Plate:face(2)"}),
    );
    person(
        &mut s,
        "note.resolve",
        json!({"id": 2, "reply": "No draft."}),
    );
    person(
        &mut s,
        "note.add",
        json!({"text": "this lug can go", "on": "Lug:face(5)"}),
    );
    person(
        &mut s,
        "note.add",
        json!({"text": "this round smaller", "on": "F2:fillet(0)"}),
    );
    run(&mut s, "feature.delete", json!({"feature": "Lug"}));
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Round", "edges": ["F1:edge(8)"]}),
    );
    assert_eq!(note(&mut s, 3)["attachment"], "detached");
    assert_eq!(note(&mut s, 4)["attachment"], "lost");

    // An open note, by the person: still open, the agent's turn.
    let open = person(&mut s, "note.reply", json!({"id": 1, "text": "Or 3 mm?"}));
    assert_eq!(open["state"], "open", "{open}");
    assert_eq!(open["replies"][0]["author"], "person");
    assert!(open["replies"][0]["agent"].is_null());
    assert_eq!(open["waitingFor"], "agent");
    assert_eq!(
        run(&mut s, "edit.undo", json!({}))["undone"],
        "Reply to note 1"
    );
    assert_eq!(note(&mut s, 1)["replies"], json!([]));
    assert_eq!(
        run(&mut s, "edit.redo", json!({}))["redone"],
        "Reply to note 1"
    );
    assert_eq!(texts(&note(&mut s, 1)), ["Or 3 mm?"]);

    // A done note, by a named agent: done as it was, its closing kept.
    let done = agent(
        &mut s,
        "Claude Code",
        "note.reply",
        json!({"id": 2, "text": "Understood."}),
    );
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["done"]["author"], "person");
    assert_eq!(done["done"]["reply"], "No draft.");
    assert_eq!(done["replies"][0]["agent"], "Claude Code");
    assert!(done["waitingFor"].is_null(), "{done}");

    // A detached note, by an agent without a name; a lost one, by an agent
    // that calls itself "you": answered all the same, where they are as
    // they were, and an agent's word, so the person's turn.
    let detached = agent(
        &mut s,
        UNNAMED_AGENT,
        "note.reply",
        json!({"id": 3, "text": "It is gone already."}),
    );
    assert_eq!(detached["replies"][0]["author"], "agent", "{detached}");
    assert!(detached["replies"][0]["agent"].is_null(), "{detached}");
    assert_eq!(detached["waitingFor"], "you");
    assert_eq!(note(&mut s, 3)["attachment"], "detached");
    let lost = agent(
        &mut s,
        "you",
        "note.reply",
        json!({"id": 4, "text": "Which corner?"}),
    );
    assert_eq!(lost["replies"][0]["author"], "agent", "{lost}");
    assert_eq!(lost["replies"][0]["agent"], "you");
    assert_eq!(lost["waitingFor"], "you");
    assert_eq!(note(&mut s, 4)["attachment"], "lost");

    // A script (no one): unknown, the agent's turn.
    let script = run(
        &mut s,
        "note.reply",
        json!({"id": 4, "text": "the left one"}),
    );
    assert_eq!(script["replies"][1]["author"], "unknown", "{script}");
    assert_eq!(script["waitingFor"], "agent");

    // A note that does not exist: refused, naming the notes there are,
    // nothing changed.
    let before = s.document_bytes().unwrap();
    let e = error(&mut s, "note.reply", json!({"id": 99, "text": "x"}));
    assert!(e.starts_with("note.reply"), "{e}");
    assert!(e.contains("there is no note 99"), "{e}");
    assert!(e.contains("1, 2, 3, 4"), "{e}");
    assert!(s.document_bytes().unwrap() == before);

    // A Jig in use changes only through its parameters.
    let mut jig = session();
    jig.open_bytes("plate.3dcraftjig", &before).unwrap();
    let e = error(&mut jig, "note.reply", json!({"id": 1, "text": "x"}));
    assert!(e.starts_with("note.reply"), "{e}");
    println!("note.reply in a Jig in use: {e}");
}

/// The text a reply keeps and the ones it refuses: 2000 code points of
/// four bytes each and 2000 with spaces trimmed off are taken, 2001 are
/// not; empty and blank, and every control character but a line break, are
/// refused with what is wrong; CR LF and the line separator become a line
/// break, a tab a space; right-to-left text is kept as written, the
/// bidirectional formatting characters left out (a reply of nothing else
/// is empty); a joined emoji kept whole. Every refusal starts with the
/// command's name and changes nothing.
#[test]
fn the_text_of_a_reply() {
    let mut s = session();
    plate(&mut s);
    person(
        &mut s,
        "note.add",
        json!({"text": "here", "on": "Plate:face(1)"}),
    );
    let kept = |s: &mut Session, text: &str| -> String {
        let answer = run(s, "note.reply", json!({"id": 1, "text": text}));
        let replies = answer["replies"].as_array().unwrap();
        replies.last().unwrap()["text"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let wide = "\u{1F600}".repeat(2000);
    assert_eq!(kept(&mut s, &wide), wide, "2000 code points, 8000 bytes");
    let padded = format!("  {}  \n", "x".repeat(2000));
    assert_eq!(kept(&mut s, &padded), "x".repeat(2000));
    assert_eq!(kept(&mut s, "a\r\nb\rc\u{2028}d\te"), "a\nb\nc\nd e");
    let hebrew = "\u{5E9}\u{5DC}\u{5D5}\u{5DD}, 7 mm";
    assert_eq!(kept(&mut s, hebrew), hebrew, "right to left, as written");
    let arabic = "\u{645}\u{631}\u{62D}\u{628}\u{627} 9 mm";
    assert_eq!(kept(&mut s, arabic), arabic);
    assert_eq!(
        kept(&mut s, "\u{202E}7 mm\u{202C} \u{200F}ok\u{2066}"),
        "7 mm ok",
        "no bidirectional formatting"
    );
    let mechanic = "\u{1F469}\u{200D}\u{1F527}";
    assert_eq!(kept(&mut s, mechanic), mechanic, "a joined emoji whole");
    assert_eq!(note(&mut s, 1)["replies"].as_array().unwrap().len(), 7);

    let before = s.document_bytes().unwrap();
    let refused = |s: &mut Session, text: &str, says: &str| {
        let e = error(s, "note.reply", json!({"id": 1, "text": text}));
        assert!(e.starts_with("note.reply: "), "{e}");
        assert!(e.contains(says), "{text:?}: {e}");
        println!("{text:.20?}: {e}");
    };
    refused(&mut s, &"y".repeat(2001), "at most 2000");
    refused(&mut s, &"y".repeat(2001), "2001");
    refused(&mut s, &"\u{1F600}".repeat(2001), "at most 2000");
    refused(&mut s, &"y".repeat(100_000), "at most 2000");
    refused(&mut s, "", "empty");
    refused(&mut s, " \n\t \u{2028} ", "empty");
    refused(&mut s, "\u{202E}\u{200F}", "empty");
    for c in ['\u{0}', '\u{7}', '\u{1B}', '\u{7F}', '\u{85}', '\u{9F}'] {
        refused(
            &mut s,
            &format!("a{c}b"),
            &format!("U+{:04X}", u32::from(c)),
        );
    }
    assert!(s.document_bytes().unwrap() == before, "nothing changed");
}

/// The caps: the 20th reply is taken and the 21st refused, a done note's
/// closing not among them; a reply that would take the document's notes
/// past 2 MiB of text is refused with how to make room, and one that fits
/// exactly is taken.
#[test]
fn the_caps_on_replies() {
    let mut s = session();
    plate(&mut s);
    person(
        &mut s,
        "note.add",
        json!({"text": "here", "on": "Plate:face(1)"}),
    );
    agent(
        &mut s,
        "Claude Code",
        "note.resolve",
        json!({"id": 1, "reply": "Done."}),
    );
    for k in 1..=20 {
        run(
            &mut s,
            "note.reply",
            json!({"id": 1, "text": format!("reply {k}")}),
        );
    }
    let before = s.document_bytes().unwrap();
    let e = error(&mut s, "note.reply", json!({"id": 1, "text": "21"}));
    assert!(e.contains("20 replies"), "{e}");
    println!("the 21st reply: {e}");
    assert!(s.document_bytes().unwrap() == before);
    let full = note(&mut s, 1);
    assert_eq!(full["replies"].as_array().unwrap().len(), 20);
    assert_eq!(full["done"]["reply"], "Done.");

    // Notes of 8000 bytes each until the next is refused, then a reply
    // one byte too long, one that fits exactly, and one more byte.
    let mut s = session();
    plate(&mut s);
    let big = "\u{1F600}".repeat(2000);
    let mut added = 0;
    let refusal = loop {
        match s.execute("note.add", &json!({"text": big, "on": "Plate:face(1)"})) {
            Ok(_) => added += 1,
            Err(e) => break e.to_string(),
        }
        assert!(added <= 1000, "the cap of 2 MiB never came");
    };
    assert!(refusal.contains("2097152"), "{refusal}");
    // Written by a script: no author's name among the bytes.
    let now: usize = notes(&mut s)["notes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["text"].as_str().unwrap().len())
        .sum();
    let room = NOTES_BYTES - now;
    assert!(
        room > 0 && room < 8000,
        "{room} bytes left after {added} notes"
    );
    // Replies of 2000 bytes, each on a note of its own, until at most
    // 2000 are left.
    let mut left = room;
    let mut id = 1u64;
    while left > 2000 {
        run(
            &mut s,
            "note.reply",
            json!({"id": id, "text": "z".repeat(2000)}),
        );
        left -= 2000;
        id += 1;
    }
    // One byte more than is left, in at most 2000 code points: refused,
    // saying how to make room; nothing changed.
    let mid = s.document_bytes().unwrap();
    let over = format!("\u{E9}{}", "q".repeat(left - 1));
    assert_eq!(over.len(), left + 1);
    let e = error(&mut s, "note.reply", json!({"id": id, "text": over}));
    assert!(e.starts_with("note.reply: "), "{e}");
    assert!(e.contains("2097152") && e.contains("note.delete"), "{e}");
    println!("past 2 MiB: {e}");
    assert!(s.document_bytes().unwrap() == mid, "nothing changed");
    // Exactly what is left: taken; then not one byte more.
    run(
        &mut s,
        "note.reply",
        json!({"id": id, "text": "q".repeat(left)}),
    );
    let e = error(&mut s, "note.reply", json!({"id": id + 1, "text": "w"}));
    assert!(e.contains("2097152"), "{e}");
    // Undone, the room is back.
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "note.reply", json!({"id": id + 1, "text": "w"}));
}

/// Whose turn it is, after each kind of word, and the same everywhere it
/// is said: `note.list`'s `waitingFor`, `forYou` and `forAgent` (which
/// count the open notes, every one in one of them), and
/// `Session::notes_json` (what the web page shows), note for note.
#[test]
fn whose_turn_it_is_after_each_kind_of_word() {
    let mut s = session();
    plate(&mut s);
    let turn = |s: &mut Session, id: u64| note(s, id)["waitingFor"].clone();
    // 1: the person's note waits for an agent; an agent's question back
    // for the person; the person's answer for an agent again.
    person(
        &mut s,
        "note.add",
        json!({"text": "1", "on": "Plate:face(1)"}),
    );
    assert_eq!(turn(&mut s, 1), "agent");
    tick();
    agent(
        &mut s,
        "Claude Code",
        "note.reply",
        json!({"id": 1, "text": "?"}),
    );
    assert_eq!(turn(&mut s, 1), "you");
    tick();
    person(&mut s, "note.reply", json!({"id": 1, "text": "!"}));
    assert_eq!(turn(&mut s, 1), "agent");
    // 2: an agent's note waits for the person; its text changed by the
    // person after that, for an agent.
    agent(
        &mut s,
        "Claude Code",
        "note.add",
        json!({"text": "2", "on": "Plate:face(1)"}),
    );
    assert_eq!(turn(&mut s, 2), "you");
    tick();
    person(&mut s, "note.edit", json!({"id": 2, "text": "2, changed"}));
    assert_eq!(turn(&mut s, 2), "agent");
    // 3: a script's note waits for an agent; an unnamed agent's for the
    // person; one that calls itself "you" is an agent all the same.
    run(
        &mut s,
        "note.add",
        json!({"text": "3", "on": "Plate:face(1)"}),
    );
    assert_eq!(turn(&mut s, 3), "agent");
    agent(
        &mut s,
        UNNAMED_AGENT,
        "note.add",
        json!({"text": "4", "on": "Plate:face(1)"}),
    );
    assert_eq!(turn(&mut s, 4), "you");
    agent(
        &mut s,
        "you",
        "note.add",
        json!({"text": "5", "on": "Plate:face(1)"}),
    );
    assert_eq!(turn(&mut s, 5), "you");
    // 6: done: nobody's turn.
    person(
        &mut s,
        "note.add",
        json!({"text": "6", "on": "Plate:face(1)"}),
    );
    agent(
        &mut s,
        "Claude Code",
        "note.resolve",
        json!({"id": 6, "reply": "Done."}),
    );
    assert!(turn(&mut s, 6).is_null());

    let listed = notes(&mut s);
    let list = listed["notes"].as_array().unwrap();
    let count = |word: &str| list.iter().filter(|n| n["waitingFor"] == word).count();
    assert_eq!(listed["forYou"], count("you"), "{listed}");
    assert_eq!(listed["forAgent"], count("agent"), "{listed}");
    assert_eq!(
        count("you") + count("agent"),
        listed["open"].as_u64().unwrap() as usize
    );
    assert_eq!((count("you"), count("agent")), (2, 3), "{listed}");
    // The web page's list (no places): the same turns, the same threads.
    let kept = s.notes_json();
    assert_eq!(kept["forYou"], listed["forYou"]);
    assert_eq!(kept["forAgent"], listed["forAgent"]);
    for (a, b) in kept["notes"].as_array().unwrap().iter().zip(list) {
        assert_eq!(a["waitingFor"], b["waitingFor"], "{a}");
        assert_eq!(a["replies"], b["replies"], "{a}");
    }
}

/// A file with more than the caps allow, written by hand: an open note
/// with 21 replies, and notes of more than 2 MiB of text together. Each is
/// refused on opening with what is wrong and where, the session keeps the
/// document it had and goes on; 20 replies on a note never closed are
/// read (format 77).
#[test]
fn a_file_over_the_caps_is_refused_and_the_session_goes_on() {
    let dir = workdir("review-ux-notes", "caps");
    let mut s = session_in(&dir);
    plate(&mut s);
    person(
        &mut s,
        "note.add",
        json!({"text": "a", "on": "Plate:face(1)"}),
    );
    for k in 0..20 {
        run(
            &mut s,
            "note.reply",
            json!({"id": 1, "text": format!("r{k}")}),
        );
    }
    let bytes = s.document_bytes().unwrap();
    let manifest = |bytes: &[u8]| -> Value {
        let entries = craft3d_zipstore::read_stored(bytes).unwrap();
        let entry = entries
            .iter()
            .find(|e| e.name == craft3d_format::MANIFEST_NAME)
            .unwrap();
        serde_json::from_slice(entry.data).unwrap()
    };
    let pack = |m: &Value| -> Vec<u8> {
        let text = serde_json::to_vec(m).unwrap();
        craft3d_zipstore::write_stored(&[(craft3d_format::MANIFEST_NAME, text.as_slice())]).unwrap()
    };
    let m = manifest(&bytes);
    assert_eq!(m["format_version"], 77);
    let read = craft3d_format::from_bytes(&pack(&m)).unwrap();
    assert_eq!(read.notes.items[0].later.len(), 20);
    assert!(read.notes.items[0].done.is_none());

    // 21 replies on the open note.
    let mut over = m.clone();
    let later = over["document"]["notes"]["items"][0]["later"]
        .as_array_mut()
        .unwrap();
    let again = later[0].clone();
    later.push(again);
    let e = craft3d_format::from_bytes(&pack(&over))
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("note 1") && e.contains("more than 20 replies"),
        "{e}"
    );
    std::fs::write(dir.join("over.3dcraft"), pack(&over)).unwrap();
    let e = error(&mut s, "file.open", json!({"path": "over.3dcraft"}));
    assert!(e.contains("more than 20 replies"), "{e}");
    println!("21 replies: {e}");

    // More than 2 MiB together: 300 notes of 2000 code points of four
    // bytes (2.4 MB).
    let mut heavy = m.clone();
    let items = heavy["document"]["notes"]["items"].as_array_mut().unwrap();
    let mut first = items[0].clone();
    first["later"] = json!([]);
    first["text"] = json!("\u{1F600}".repeat(2000));
    items.clear();
    for id in 1..=300u64 {
        let mut n = first.clone();
        n["id"] = json!(id);
        items.push(n);
    }
    heavy["document"]["notes"]["next"] = json!(301);
    let e = craft3d_format::from_bytes(&pack(&heavy))
        .unwrap_err()
        .to_string();
    assert!(e.contains("bytes of text together"), "{e}");
    std::fs::write(dir.join("heavy.3dcraft"), pack(&heavy)).unwrap();
    let e = error(&mut s, "file.open", json!({"path": "heavy.3dcraft"}));
    assert!(e.contains("2097152"), "{e}");
    println!("2.4 MB of notes: {e}");

    // The session still holds the plate and its note, and goes on.
    assert_eq!(note(&mut s, 1)["replies"].as_array().unwrap().len(), 20);
    person(
        &mut s,
        "note.add",
        json!({"text": "b", "on": "Plate:face(1)"}),
    );
    run(&mut s, "note.reply", json!({"id": 2, "text": "c"}));
}

/// A published Jig carries no note and no reply; the design keeps both.
#[test]
fn a_published_jig_carries_no_reply() {
    let dir = workdir("review-ux-notes", "jig");
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "variable.set", json!({"name": "t", "expr": "5 mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": [40, 20, "t"]}),
    );
    run(
        &mut s,
        "variable.publish",
        json!({"name": "t", "label": "Thickness", "min": "2 mm", "max": "20 mm"}),
    );
    person(
        &mut s,
        "note.add",
        json!({"text": "rux-text-9", "on": "Plate:face(1)"}),
    );
    agent(
        &mut s,
        "Claude Code",
        "note.reply",
        json!({"id": 1, "text": "rux-reply-9"}),
    );
    run(&mut s, "jig.export", json!({"path": "noted.3dcraftjig"}));
    let jig = std::fs::read(dir.join("noted.3dcraftjig")).unwrap();
    let text = String::from_utf8_lossy(&jig);
    assert!(!text.contains("rux-text-9") && !text.contains("rux-reply-9"));
    assert!(craft3d_format::from_bytes(&jig).unwrap().notes.is_empty());
    assert_eq!(texts(&note(&mut s, 1)), ["rux-reply-9"]);
}

/// Finding 1 (must): opened again, a note's turn goes to the wrong side
/// when a reply came after its closing. The closing's reply is pushed
/// after the replies that came later than it (`Document::resolve_note`,
/// `crates/doc/src/note.rs` 623-633), and `Note::last_word` takes the last
/// of the list, not the latest: the person who answered a done note and
/// opened it again finds it under For you, and an agent that works through
/// the notes that wait for it (`waitingFor: agent`) skips it. The thread
/// also reads in another order than it did while the note was done (the
/// window's `NoteRow::thread` puts the closing at its time).
#[test]
fn opened_again_the_turn_follows_the_latest_word() {
    let mut s = session();
    plate(&mut s);
    // The person asks; the agent closes it with what it did; the person
    // answers on the done note and opens it again: the agent's turn.
    person(
        &mut s,
        "note.add",
        json!({"text": "2 mm thicker here", "on": "Plate:face(1)"}),
    );
    tick();
    agent(
        &mut s,
        "Claude Code",
        "note.resolve",
        json!({"id": 1, "reply": "Made it 7 mm."}),
    );
    tick();
    person(
        &mut s,
        "note.reply",
        json!({"id": 1, "text": "The other flange too, please."}),
    );
    tick();
    let reopened = person(&mut s, "note.resolve", json!({"id": 1, "done": false}));
    let times: Vec<u64> = reopened["replies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["timeMs"].as_u64().unwrap())
        .collect();
    assert!(
        times.windows(2).all(|w| w[0] <= w[1]),
        "the thread in the order it was written: {reopened}"
    );
    assert_eq!(
        reopened["waitingFor"], "agent",
        "the person spoke last: {reopened}"
    );
    // The other way about: the agent's question closed by the person, the
    // agent's answer on the done note, the agent opens it again: the
    // person's turn.
    agent(
        &mut s,
        "Claude Code",
        "note.add",
        json!({"text": "Draft this side?", "on": "Plate:face(2)"}),
    );
    tick();
    person(&mut s, "note.resolve", json!({"id": 2, "reply": "No."}));
    tick();
    agent(
        &mut s,
        "Claude Code",
        "note.reply",
        json!({"id": 2, "text": "Then how is it released from the mould?"}),
    );
    tick();
    let asked = agent(
        &mut s,
        "Claude Code",
        "note.resolve",
        json!({"id": 2, "done": false}),
    );
    assert_eq!(asked["waitingFor"], "you", "the agent asked last: {asked}");
}

/// Finding 2 (should): opened again, a note with 20 replies loses the
/// reply it was closed with, silently (`crates/doc/src/note.rs` 626: "while
/// it has room for one"). Either it is kept or Open again is refused with
/// why; the text is never dropped without a word.
#[test]
fn opened_again_a_full_note_keeps_its_closing_reply() {
    let mut s = session();
    plate(&mut s);
    person(
        &mut s,
        "note.add",
        json!({"text": "here", "on": "Plate:face(1)"}),
    );
    agent(
        &mut s,
        "Claude Code",
        "note.resolve",
        json!({"id": 1, "reply": "What I did, in one sentence."}),
    );
    for k in 0..20 {
        run(
            &mut s,
            "note.reply",
            json!({"id": 1, "text": format!("r{k}")}),
        );
    }
    match by(
        &mut s,
        None,
        "note.resolve",
        json!({"id": 1, "done": false}),
    ) {
        Ok(reopened) => assert!(
            texts(&reopened)
                .iter()
                .any(|t| t == "What I did, in one sentence."),
            "the closing's reply is gone: {reopened}"
        ),
        Err(e) => println!("refused, with why: {e}"),
    }
}
