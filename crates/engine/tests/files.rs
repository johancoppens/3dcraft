//! Listing the open documents and switching between them (field test
//! 2026-10-12, finding 5: earlier documents could only be reached by
//! closing the ones on top).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::{run, session};
use craft3d_engine::{Access, Session, Workspace};
use serde_json::{Value, json};

fn error(session: &mut Session, id: &str, params: Value) -> String {
    session.execute(id, &params).unwrap_err().to_string()
}

#[test]
fn an_empty_session_lists_nothing_and_has_no_active_document() {
    let mut s = session();
    assert_eq!(
        run(&mut s, "file.list", json!({})),
        json!({"active": null, "documents": []})
    );
    let e = error(&mut s, "file.activate", json!({"document": 0}));
    assert!(e.contains("no document is open"), "{e}");
}

#[test]
fn documents_are_listed_in_order_and_commands_act_on_the_active_one() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "A", "size": [1, 2, 3]}),
    );
    run(&mut s, "file.new", json!({"units": "in"}));
    run(&mut s, "variable.set", json!({"name": "w", "expr": "2 in"}));
    run(&mut s, "file.new", json!({}));

    let listed = run(&mut s, "file.list", json!({}));
    assert_eq!(listed["active"], 2);
    assert_eq!(
        listed["documents"],
        json!([
            {"document": 0, "path": null, "dirty": true, "active": false,
             "features": 1, "variables": 0, "units": "mm"},
            {"document": 1, "path": null, "dirty": true, "active": false,
             "features": 0, "variables": 1, "units": "in"},
            {"document": 2, "path": null, "dirty": true, "active": true,
             "features": 0, "variables": 0, "units": "mm"},
        ])
    );

    // Back to the first: the block is there, and an edit lands in it.
    let switched = run(&mut s, "file.activate", json!({"document": 0}));
    assert_eq!(switched["active"], 0);
    assert_eq!(switched["documents"][0]["active"], true);
    assert_eq!(switched["documents"][2]["active"], false);
    let inspected = run(&mut s, "document.inspect", json!({}));
    assert_eq!(inspected["features"][0]["name"], "A");
    run(
        &mut s,
        "feature.block",
        json!({"name": "B", "size": [1, 1, 1]}),
    );
    let listed = run(&mut s, "file.list", json!({}));
    let features: Vec<&Value> = listed["documents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| &d["features"])
        .collect();
    assert_eq!(features, [&json!(2), &json!(0), &json!(0)]);

    // Each document has its own undo history.
    run(&mut s, "file.activate", json!({"document": 1}));
    run(&mut s, "edit.undo", json!({}));
    let listed = run(&mut s, "file.list", json!({}));
    assert_eq!(listed["documents"][1]["variables"], 0);
    assert_eq!(listed["documents"][0]["features"], 2);

    // Closing the middle one moves the last one up; the list says so.
    run(&mut s, "file.close", json!({"discard": true}));
    let listed = run(&mut s, "file.list", json!({}));
    assert_eq!(listed["documents"].as_array().unwrap().len(), 2);
    assert_eq!(listed["documents"][1]["document"], 1);
    assert_eq!(listed["documents"][0]["features"], 2);
    let active = listed["active"].as_u64().unwrap();
    assert_eq!(listed["documents"][active as usize]["active"], true);
}

#[test]
fn a_viewer_scene_follows_the_active_document() {
    let mut s = session();
    run(&mut s, "file.new", json!({}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "A", "size": [1, 2, 3]}),
    );
    run(&mut s, "file.new", json!({}));
    assert_eq!(s.scene(None).unwrap().unwrap().bodies.len(), 0);
    let before = s.scene_revision();
    run(&mut s, "file.activate", json!({"document": 0}));
    assert_ne!(s.scene_revision(), before, "the viewer must redraw");
    assert_eq!(s.scene(None).unwrap().unwrap().bodies.len(), 1);
}

#[test]
fn bad_requests_are_errors_that_change_nothing() {
    let mut s = session();
    run(&mut s, "file.new", json!({}));
    run(&mut s, "file.new", json!({}));
    for (params, words) in [
        (json!({}), vec!["give `document`", "or `path`"]),
        (json!({"document": 2}), vec!["no document 2", "2 are open"]),
        (json!({"document": -1}), vec!["whole number"]),
        (json!({"document": 1.5}), vec!["whole number"]),
        (json!({"document": "1"}), vec!["whole number"]),
        (json!({"document": 1e300}), vec!["whole number"]),
        (
            json!({"path": "a.3dcraft"}),
            vec!["no open document has the path"],
        ),
        (json!({"path": 7}), vec!["`path` must be"]),
        (json!({"document": 0, "path": "a.3dcraft"}), vec!["either"]),
        (json!({"index": 0}), vec!["unknown parameter `index`"]),
    ] {
        let e = error(&mut s, "file.activate", params.clone());
        for word in &words {
            assert!(e.contains(word), "{params}: `{word}` is not in: {e}");
        }
        assert_eq!(run(&mut s, "file.list", json!({}))["active"], 1, "{params}");
    }
    let e = error(&mut s, "file.list", json!({"all": true}));
    assert!(e.contains("takes no parameters"), "{e}");
    let e = error(&mut s, "file.list", json!([1]));
    assert!(e.contains("JSON object"), "{e}");
}

#[test]
fn a_saved_document_is_found_by_its_path() {
    let dir = std::env::temp_dir().join(format!("craft3d-files-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    run(&mut s, "file.new", json!({}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "A", "size": [1, 2, 3]}),
    );
    run(&mut s, "file.save", json!({"path": "a.3dcraft"}));
    run(&mut s, "file.new", json!({}));
    run(&mut s, "file.open", json!({"path": "a.3dcraft"}));
    let listed = run(&mut s, "file.list", json!({}));
    assert_eq!(listed["documents"][0]["path"], "a.3dcraft");
    assert_eq!(listed["documents"][0]["dirty"], false);
    // The same file open twice cannot be told apart by path.
    let e = error(&mut s, "file.activate", json!({"path": "a.3dcraft"}));
    assert!(e.contains("more than once"), "{e}");
    run(&mut s, "file.close", json!({}));
    run(&mut s, "file.activate", json!({"document": 1}));
    let switched = run(&mut s, "file.activate", json!({"path": "a.3dcraft"}));
    assert_eq!(switched["active"], 0);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A document opened by a path relative to the working directory, with
/// unrestricted access (the desktop app opens the file named on its
/// command line that way), keeps the absolute path: a save without a path
/// after the session got a write root reaches the same file, not the
/// relative path again beneath the root (2026-10-15, a window started as
/// `craft3d target/work/x.3dcraft --automation-write-root target/work`
/// saved to `target/work/target/work/x.3dcraft`). Looked up by the path
/// it was opened with, it is still found.
#[test]
fn a_document_opened_by_a_relative_path_keeps_it_absolute() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/tmp/craft3d-files-relative");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("work")).unwrap();
    // Integration tests run in the package's directory.
    let relative = "../../target/tmp/craft3d-files-relative/work/part.3dcraft";
    let mut author = Session::new(Workspace::unrestricted());
    run(&mut author, "file.new", json!({}));
    run(
        &mut author,
        "feature.block",
        json!({"name": "A", "size": [1, 2, 3]}),
    );
    let saved = run(&mut author, "file.save", json!({"path": relative}));
    let file = craft3d_engine::canonical(&dir.join("work/part.3dcraft")).unwrap();
    assert_eq!(saved["path"], file.to_str().unwrap());

    let mut s = Session::new(Workspace {
        read: Access::Unrestricted,
        write: Access::None,
    });
    let opened = run(&mut s, "file.open", json!({"path": relative}));
    assert_eq!(opened["path"], file.to_str().unwrap());
    let listed = run(&mut s, "file.list", json!({}));
    assert_eq!(listed["documents"][0]["path"], file.to_str().unwrap());
    let found = run(&mut s, "file.activate", json!({"path": relative}));
    assert_eq!(found["active"], 0);

    let root = craft3d_engine::canonical(&dir).unwrap();
    s.set_workspace(Workspace {
        read: Access::Root(root.clone()),
        write: Access::Root(root.clone()),
    });
    run(
        &mut s,
        "feature.block",
        json!({"name": "B", "size": [1, 1, 1]}),
    );
    let saved = run(&mut s, "file.save", json!({}));
    assert_eq!(saved["path"], file.to_str().unwrap());
    assert!(
        !root.join("target").exists(),
        "saved beneath the root again"
    );
    let mut check = Session::new(Workspace::unrestricted());
    let reopened = run(
        &mut check,
        "file.open",
        json!({"path": file.to_str().unwrap()}),
    );
    assert_eq!(reopened["features"], 2);
    std::fs::remove_dir_all(&dir).unwrap();
}
