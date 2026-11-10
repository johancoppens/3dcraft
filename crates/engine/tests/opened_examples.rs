//! An example the program carries (the window's start screen), opened
//! from its bytes with `Session::open_bytes_titled` (docs/decisions.md,
//! 2026-11-10): clean until it is changed, named by its title, never
//! written back. A part, a Jig in use and an assembly.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{DocumentStatus, Session, Workspace};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn status(s: &Session) -> DocumentStatus {
    s.document_status().unwrap()
}

/// The bytes of a part with one block, as `file.save` writes them.
fn part_bytes() -> Vec<u8> {
    let mut s = Session::new(Workspace::memory());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "feature.block", json!({"size": [40, 20, 5]}));
    run(&mut s, "file.save", json!({"path": "bookend.3dcraft"}));
    s.take_file("bookend.3dcraft").unwrap()
}

/// The bytes of a Jig with one public parameter, as `jig.export` writes
/// them.
fn jig_bytes() -> Vec<u8> {
    let mut s = Session::new(Workspace::memory());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "variable.set",
        json!({"name": "w", "expr": "40 mm"}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": ["w", 20, 5]}),
    );
    run(
        &mut s,
        "variable.publish",
        json!({"name": "w", "min": "10 mm", "max": "90 mm"}),
    );
    run(&mut s, "jig.export", json!({"path": "plate.3dcraftjig"}));
    s.take_file("plate.3dcraftjig").unwrap()
}

/// Opened and only looked at, it is not unsaved work: no unsaved mark, no
/// question on close, its title for its name, no file. The first edit
/// makes it an unsaved document like any other; an undo back to how it
/// opened is clean again, as for a saved file (`OpenDocument::dirty`).
/// Save still asks where; saved, it is named by its file.
#[test]
fn an_opened_example_is_clean_and_named_until_it_is_changed() {
    let bytes = part_bytes();
    let mut s = Session::new(Workspace::memory());
    let opened = s
        .open_bytes_titled("steel-bookend.3dcraft", Some("Steel bookend"), &bytes)
        .unwrap();
    assert_eq!(opened["title"], "Steel bookend", "{opened}");
    let shown = status(&s);
    assert_eq!(shown.title.as_deref(), Some("Steel bookend"));
    assert!(shown.path.is_none() && shown.file.is_none());
    assert!(!shown.dirty && !shown.close_asks && !shown.assembly);
    let listed = run(&mut s, "file.list", json!({}));
    assert_eq!(listed["documents"][0]["title"], "Steel bookend");
    assert_eq!(listed["documents"][0]["dirty"], false);
    // Regenerated and measured: still clean.
    run(&mut s, "document.measure", json!({}));
    assert!(!status(&s).dirty);
    // Closed without a question.
    run(&mut s, "file.close", json!({}));
    assert!(s.document_status().is_none());

    s.open_bytes_titled("steel-bookend.3dcraft", Some("Steel bookend"), &bytes)
        .unwrap();
    run(&mut s, "feature.block", json!({"size": [1, 1, 1]}));
    let edited = status(&s);
    assert!(edited.dirty && edited.close_asks, "{edited:?}");
    assert_eq!(edited.title.as_deref(), Some("Steel bookend"));
    let refused = s.execute("file.close", &json!({})).unwrap_err();
    assert!(refused.to_string().contains("unsaved"), "{refused}");
    run(&mut s, "edit.undo", json!({}));
    let back = status(&s);
    assert!(!back.dirty && !back.close_asks, "{back:?}");
    run(&mut s, "edit.redo", json!({}));
    assert!(status(&s).dirty);
    // Save asks where: nothing is written back to the bytes.
    assert!(s.execute("file.save", &json!({})).is_err());
    assert!(s.file_names().is_empty());
    run(&mut s, "file.save", json!({"path": "my-bookend.3dcraft"}));
    let saved = status(&s);
    assert_eq!(saved.path.as_deref(), Some("my-bookend.3dcraft"));
    assert_eq!(saved.title, None, "named by its file from now on");
    assert!(!saved.dirty);
    let listed = run(&mut s, "file.list", json!({}));
    assert_eq!(listed["documents"][0].get("title"), None);
}

/// A Jig the program carries behaves as one opened from its file: never
/// dirty, and its close asks only once its parameters changed since it
/// was opened (before 2026-11-10 it asked at once, having no saved state
/// to compare with).
#[test]
fn a_jig_from_the_program_asks_only_when_its_parameters_changed() {
    let bytes = jig_bytes();
    let mut s = Session::new(Workspace::memory());
    let opened = s
        .open_bytes_titled("laser-box.3dcraftjig", Some("Laser-cut box"), &bytes)
        .unwrap();
    assert_eq!(opened["jigInUse"], true);
    let shown = status(&s);
    assert!(
        shown.jig_in_use && !shown.dirty && !shown.close_asks,
        "{shown:?}"
    );
    assert_eq!(shown.title.as_deref(), Some("Laser-cut box"));
    run(&mut s, "param.set", json!({"values": {"w": 70}}));
    let set = status(&s);
    assert!(set.jig_in_use && !set.dirty && set.close_asks, "{set:?}");
    let refused = s.execute("file.close", &json!({})).unwrap_err();
    assert!(refused.to_string().contains("Jig"), "{refused}");
    run(&mut s, "edit.undo", json!({}));
    assert!(!status(&s).close_asks);
    // The same as the Jig opened from a file.
    let mut from_file = Session::new(Workspace::memory());
    from_file
        .put_file("plate.3dcraftjig", bytes.clone())
        .unwrap();
    run(
        &mut from_file,
        "file.open",
        json!({"path": "plate.3dcraftjig"}),
    );
    let file_status = status(&from_file);
    assert_eq!(
        (
            file_status.dirty,
            file_status.close_asks,
            file_status.jig_in_use
        ),
        (false, false, true)
    );
    // Saved as a model, under a name of its own.
    run(&mut s, "param.set", json!({"values": {"w": 60}}));
    assert!(s.execute("file.save", &json!({})).is_err());
    run(&mut s, "file.save", json!({"path": "box.3dcraft"}));
    let saved = status(&s);
    assert!(!saved.jig_in_use && saved.title.is_none() && !saved.dirty);
}

/// An assembly the program carries (with its parts packed in it) opens
/// clean and says it is one, so Save proposes a `.3dcraftasm` name; an
/// edit makes it unsaved.
#[test]
fn an_assembly_from_the_program_is_clean_until_an_instance_changes() {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../crates/testkit/data/planetary-gearbox.3dcraftasm"
    ))
    .unwrap();
    let mut s = Session::new(Workspace::memory());
    let opened = s
        .open_bytes_titled(
            "planetary-gearbox.3dcraftasm",
            Some("Planetary gearbox"),
            &bytes,
        )
        .unwrap();
    assert_eq!(opened["assembly"], true);
    let shown = status(&s);
    assert!(
        shown.assembly && !shown.dirty && !shown.close_asks,
        "{shown:?}"
    );
    assert_eq!(shown.title.as_deref(), Some("Planetary gearbox"));
    let inspected = run(&mut s, "assembly.inspect", json!({}));
    assert!(!status(&s).dirty, "solving it changes nothing");
    let first = inspected["instances"][0]["name"]
        .as_str()
        .unwrap()
        .to_string();
    run(
        &mut s,
        "instance.edit",
        json!({"name": first, "visible": false}),
    );
    let edited = status(&s);
    assert!(edited.dirty && edited.close_asks, "{edited:?}");
}

/// A title is one line of at most 120 characters; an empty one is none,
/// and a document opened without one is untitled.
#[test]
fn a_title_is_kept_as_one_short_line() {
    let bytes = part_bytes();
    let mut s = Session::new(Workspace::memory());
    s.open_bytes_titled("a.3dcraft", Some(" \n"), &bytes)
        .unwrap();
    assert_eq!(status(&s).title, None);
    s.open_bytes_titled(
        "b.3dcraft",
        Some(&format!("{}\u{7}", "x".repeat(300))),
        &bytes,
    )
    .unwrap();
    assert_eq!(status(&s).title.map(|t| t.chars().count()), Some(120));
    s.open_bytes("c.3dcraft", &bytes).unwrap();
    let plain = status(&s);
    assert!(plain.title.is_none() && !plain.dirty);
}
