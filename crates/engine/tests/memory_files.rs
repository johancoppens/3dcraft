//! A session that keeps its files in memory (`Access::Memory`, the
//! browser's workspace): `file.open`, `file.save`, `file.import`,
//! `file.export` and `view.render` with a path read and write the
//! session's own files, and give the same bytes as on disk.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Access, Session, Workspace};
use serde_json::{Value, json};
use std::path::PathBuf;

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn model(session: &mut Session) {
    run(session, "file.new", json!({"units": "mm"}));
    run(
        session,
        "feature.block",
        json!({"size": [40, 20, 10], "name": "Plate"}),
    );
    run(
        session,
        "feature.cylinder",
        json!({"radius": 5, "height": 30, "position": [60, 0, 0], "name": "Pin"}),
    );
    run(
        session,
        "body.appearance",
        json!({"body": "Pin", "color": "#c04020"}),
    );
}

fn on_disk(tag: &str) -> (Session, PathBuf) {
    let dir = std::env::temp_dir().join(format!("craft3d-memory-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let session = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    (session, dir)
}

#[test]
fn exports_in_memory_are_the_bytes_written_to_disk() {
    let mut memory = Session::new(Workspace::memory());
    let (mut disk, dir) = on_disk("export");
    model(&mut memory);
    model(&mut disk);
    for name in ["a.step", "a.stl", "a.3mf", "a.obj"] {
        let wrote = run(&mut memory, "file.export", json!({ "path": name }));
        run(&mut disk, "file.export", json!({ "path": name }));
        let bytes = memory.take_file(name).unwrap_or_else(|| panic!("{name}"));
        assert_eq!(wrote["bytes"].as_u64(), Some(bytes.len() as u64), "{name}");
        let expected = std::fs::read(dir.join(name)).unwrap();
        // STEP carries the time it was written; the rest is the same.
        if name.ends_with(".step") {
            assert_eq!(bytes.len(), expected.len(), "{name}");
            assert!(bytes.starts_with(b"ISO-10303-21;"));
        } else {
            assert_eq!(bytes, expected, "{name}");
        }
    }
    // The OBJ's colours went to a library beside it.
    assert_eq!(memory.file_names(), ["a.mtl"]);
    assert_eq!(
        memory.take_file("a.mtl").unwrap(),
        std::fs::read(dir.join("a.mtl")).unwrap()
    );
    // A picture with a path is a file too.
    run(
        &mut memory,
        "view.render",
        json!({"path": "shot.png", "width": 64, "height": 48}),
    );
    assert!(
        memory
            .take_file("shot.png")
            .unwrap()
            .starts_with(b"\x89PNG")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_document_saved_to_memory_opens_again() {
    let mut first = Session::new(Workspace::memory());
    model(&mut first);
    let saved = run(&mut first, "file.save", json!({"path": "part.3dcraft"}));
    assert_eq!(saved["path"], "part.3dcraft");
    // Saved: a later Save without a path goes to the same name.
    run(&mut first, "file.save", json!({}));
    let bytes = first.take_file("part.3dcraft").unwrap();
    assert!(first.file_names().is_empty());

    let mut second = Session::new(Workspace::memory());
    second.put_file("part.3dcraft", bytes.clone()).unwrap();
    let opened = run(&mut second, "file.open", json!({"path": "part.3dcraft"}));
    assert_eq!(opened["features"], 2);
    // Saving it again gives the same bytes.
    run(&mut second, "file.save", json!({}));
    assert_eq!(second.take_file("part.3dcraft").unwrap(), bytes);
    // A name that is not there, or not a .3dcraft file, is an error.
    let missing = second
        .execute("file.open", &json!({"path": "nothing.3dcraft"}))
        .unwrap_err();
    assert!(missing.message().contains("no file"), "{missing}");
    second
        .put_file("junk.3dcraft", b"not a zip".to_vec())
        .unwrap();
    assert!(
        second
            .execute("file.open", &json!({"path": "junk.3dcraft"}))
            .is_err()
    );
}

#[test]
fn a_step_file_in_memory_is_imported() {
    let mut source = Session::new(Workspace::memory());
    model(&mut source);
    run(&mut source, "file.export", json!({"path": "parts.step"}));
    let step = source.take_file("parts.step").unwrap();

    let mut session = Session::new(Workspace::memory());
    run(&mut session, "file.new", json!({"units": "mm"}));
    session.put_file("parts.step", step).unwrap();
    let imported = run(&mut session, "file.import", json!({"path": "parts.step"}));
    let bodies = imported["bodies"].as_array().map_or(0, Vec::len);
    assert_eq!(bodies, 2, "{imported}");
}

#[test]
fn a_memory_session_touches_no_disk() {
    let mut session = Session::new(Workspace::memory());
    model(&mut session);
    for (id, params) in [
        ("file.export", json!({"path": "/tmp/x.stl"})),
        ("file.export", json!({"path": "../x.stl"})),
        ("file.save", json!({"path": "/tmp/x.3dcraft"})),
        ("file.open", json!({"path": "/etc/passwd"})),
        ("file.import", json!({"path": "../x.step"})),
        ("view.replay", json!({"dir": "frames"})),
    ] {
        assert!(session.execute(id, &params).is_err(), "{id} {params}");
    }
    assert!(session.write_output("shot.png", b"x").is_err());
    assert!(session.file_names().is_empty());
}
