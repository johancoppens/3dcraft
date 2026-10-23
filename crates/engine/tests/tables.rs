//! Tables (R1, formulas: docs/design/document.md › *Tables*): values a
//! model looks up by row (`belt[profile].pitch`), a choice parameter
//! whose labels are a table's keys, and the commands that keep them.
//! The oracle: a block sized by looked-up values has their product as its
//! volume, before and after the table, the row or the names change.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::run;
use common::plain::{refused, session};
use craft3d_engine::Session;
use serde_json::{Value, json};

fn volume(s: &mut Session) -> f64 {
    run(s, "document.measure", json!({}))["volume"]
        .as_f64()
        .unwrap()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * b.abs()
}

/// A plate whose width and thickness come from the row `profile` of the
/// table `belt`; `profile` is a choice parameter offering the belt keys.
fn plate(s: &mut Session) {
    run(s, "file.new", json!({"units": "mm"}));
    let made = run(
        s,
        "table.set",
        json!({"name": "belt", "columns": ["profile", {"name": "width", "unit": "mm"}, {"name": "thick", "unit": "in"}, "teeth"],
               "rows": [["GT2", 6, 0.1, 20], ["HTD3", 9, 0.2, 30], ["T5", 10, 0.25, "n/a"]]}),
    );
    assert_eq!(made["created"], true);
    assert_eq!(made["keys"], json!(["GT2", "HTD3", "T5"]));
    run(s, "variable.set", json!({"name": "profile", "expr": "1"}));
    run(
        s,
        "variable.set",
        json!({"name": "w", "expr": "belt[profile].width"}),
    );
    run(
        s,
        "feature.block",
        json!({"name": "Plate", "size": ["w", "40 mm", "belt[profile].thick"]}),
    );
    run(
        s,
        "variable.publish",
        json!({"name": "profile", "kind": "choice", "choicesFrom": "belt", "label": "Belt"}),
    );
}

#[test]
fn a_lookup_sizes_the_model_and_follows_the_row_and_the_table() {
    let mut s = session();
    plate(&mut s);
    assert!(close(volume(&mut s), 6.0 * 40.0 * 2.54));
    // The choice parameter offers the table's keys and is set by one.
    let list = run(&mut s, "param.list", json!({}));
    let p = &list["parameters"][0];
    assert_eq!(p["choices"], json!(["GT2", "HTD3", "T5"]));
    assert_eq!(p["choicesFrom"], "belt");
    assert_eq!(p["choice"], "GT2");
    run(&mut s, "param.set", json!({"values": {"profile": "HTD3"}}));
    assert!(close(volume(&mut s), 9.0 * 40.0 * 5.08));
    // Editing the table rebuilds what its values reach.
    let set = run(
        &mut s,
        "table.set",
        json!({"name": "belt", "columns": ["profile", {"name": "width", "unit": "mm"}, {"name": "thick", "unit": "in"}, "teeth"],
               "rows": [["GT2", 6, 0.1, 20], ["HTD3", 12, 0.2, 30], ["T5", 10, 0.25, "n/a"]]}),
    );
    assert_eq!(set["created"], false);
    assert!(close(volume(&mut s), 12.0 * 40.0 * 5.08));
    // One undo step each.
    run(&mut s, "edit.undo", json!({}));
    assert!(close(volume(&mut s), 9.0 * 40.0 * 5.08));
    run(&mut s, "edit.redo", json!({}));
    assert!(close(volume(&mut s), 12.0 * 40.0 * 5.08));
    // A row the table does not have, a text where a number is needed:
    // the feature fails and says why, the document stays.
    let failed = run(
        &mut s,
        "variable.set",
        json!({"name": "w", "expr": "belt[profile + 5].width"}),
    );
    let text = failed.to_string();
    assert!(text.contains("there is no row 7"), "{text}");
    let failed = run(
        &mut s,
        "variable.set",
        json!({"name": "w", "expr": "belt[\"T5\"].teeth * 1 mm"}),
    );
    assert!(failed.to_string().contains("holds a text"), "{failed}");
    run(
        &mut s,
        "variable.set",
        json!({"name": "w", "expr": "belt[\"T5\"].width"}),
    );
    assert!(close(volume(&mut s), 10.0 * 40.0 * 5.08));
    // A table shrunk below the choice: the parameter is invalid, not a
    // crash.
    run(
        &mut s,
        "table.set",
        json!({"name": "belt", "columns": ["profile", {"name": "width", "unit": "mm"}, {"name": "thick", "unit": "in"}],
               "rows": [["GT2", 6, 0.1], ["T5", 10, 0.25]]}),
    );
    let list = run(&mut s, "param.list", json!({}));
    assert_eq!(list["parameters"][0]["choices"], json!(["GT2", "T5"]));
    assert_eq!(list["parameters"][0]["valid"], true);
    let check = run(&mut s, "document.check", json!({}));
    assert!(check.is_object());
}

#[test]
fn renames_and_deletes_keep_the_lookups_whole() {
    let mut s = session();
    plate(&mut s);
    let listed = run(&mut s, "table.list", json!({}));
    assert_eq!(listed["tables"][0]["usedBy"].as_array().unwrap().len(), 3);
    let renamed = run(
        &mut s,
        "table.rename",
        json!({"name": "belt", "to": "strap"}),
    );
    assert_eq!(renamed["table"], "strap");
    let inspect = run(&mut s, "document.inspect", json!({}));
    assert!(
        inspect.to_string().contains("strap[profile].width"),
        "{inspect}"
    );
    assert_eq!(
        run(&mut s, "param.list", json!({}))["parameters"][0]["choicesFrom"],
        "strap"
    );
    assert!(close(volume(&mut s), 6.0 * 40.0 * 2.54));
    // A variable rename leaves the table's name and its columns alone.
    run(
        &mut s,
        "variable.rename",
        json!({"name": "w", "to": "width"}),
    );
    assert!(close(volume(&mut s), 6.0 * 40.0 * 2.54));
    // Deleting a used table is refused; a choice's table even with force.
    let e = refused(&mut s, "table.delete", json!({"name": "strap"}));
    assert!(e.contains("choices of parameter `profile`"), "{e}");
    let e = refused(
        &mut s,
        "table.delete",
        json!({"name": "strap", "force": true}),
    );
    assert!(e.contains("unpublish"), "{e}");
    run(&mut s, "variable.unpublish", json!({"name": "profile"}));
    let e = refused(&mut s, "table.delete", json!({"name": "strap"}));
    assert!(
        e.contains("variable `width`") && e.contains("feature F1"),
        "{e}"
    );
    let deleted = run(
        &mut s,
        "table.delete",
        json!({"name": "strap", "force": true}),
    );
    assert_eq!(deleted["failed"][0]["id"], "F1", "{deleted}");
    run(&mut s, "edit.undo", json!({}));
    assert!(close(volume(&mut s), 6.0 * 40.0 * 2.54));
    // Names: a table and a variable do not share one.
    let e = refused(
        &mut s,
        "table.rename",
        json!({"name": "strap", "to": "width"}),
    );
    assert!(e.contains("taken"), "{e}");
    let e = refused(
        &mut s,
        "variable.set",
        json!({"name": "strap", "expr": "1"}),
    );
    assert!(e.contains("table"), "{e}");
}

#[test]
fn a_document_with_tables_saves_and_opens_the_same() {
    let dir = common::plain::workdir("tables", "save");
    let mut s = common::plain::session_in(&dir);
    plate(&mut s);
    run(&mut s, "file.save", json!({"path": "plate.3dcraft"}));
    let mut t = common::plain::session_in(&dir);
    run(&mut t, "file.open", json!({"path": "plate.3dcraft"}));
    assert!(close(volume(&mut t), 6.0 * 40.0 * 2.54));
    let a = run(&mut s, "table.list", json!({"name": "belt"}));
    let b = run(&mut t, "table.list", json!({"name": "belt"}));
    assert_eq!(a, b);
    assert_eq!(b["cells"][2], json!(["T5", 10.0, 0.25, "n/a"]));
}

#[test]
fn bad_tables_are_refused_with_a_reason() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "variable.set", json!({"name": "w", "expr": "1 mm"}));
    let columns = json!(["k", {"name": "x", "unit": "mm"}]);
    for (params, words) in [
        (
            json!({"name": "t", "columns": columns, "rows": [["a", 1], ["a", 2]]}),
            "two rows",
        ),
        (
            json!({"name": "t", "columns": columns, "rows": [["a"]]}),
            "cells",
        ),
        (
            json!({"name": "t", "columns": columns, "rows": [["a", true]]}),
            "number or a text",
        ),
        (
            json!({"name": "t", "columns": columns, "rows": [["a", 1]], "key": "z"}),
            "key",
        ),
        (
            json!({"name": "t", "columns": [{"name": "x", "unit": "inch"}]}),
            "not a unit",
        ),
        (
            json!({"name": "t", "columns": [{"name": "x", "size": 2}]}),
            "no `size`",
        ),
        (json!({"name": "t", "columns": []}), "at least one column"),
        (json!({"name": "t", "columns": 3}), "list of columns"),
        (json!({"name": "w", "columns": columns}), "variable"),
        (
            json!({"name": "mm", "columns": columns}),
            "cannot name a table",
        ),
        (
            json!({"name": "t", "columns": columns, "rows": [["a\"b", 1]]}),
            "printable",
        ),
        (
            json!({"name": "t", "columns": columns, "rows": [[1, 2], [1.0, 3]]}),
            "two rows",
        ),
    ] {
        let e = refused(&mut s, "table.set", params.clone());
        assert!(e.contains(words), "{params}: {e}");
    }
    let many: Vec<Value> = (0..10_001).map(|i| json!([format!("r{i}"), i])).collect();
    let e = refused(
        &mut s,
        "table.set",
        json!({"name": "t", "columns": columns, "rows": many}),
    );
    assert!(e.contains("at most 10000 rows"), "{e}");
    let wide: Vec<String> = (0..65).map(|i| format!("c{i}")).collect();
    let e = refused(&mut s, "table.set", json!({"name": "t", "columns": wide}));
    assert!(e.contains("at most 64 columns"), "{e}");
    for (id, params) in [
        ("table.delete", json!({"name": "nope"})),
        ("table.rename", json!({"name": "nope", "to": "x"})),
        ("table.list", json!({"name": "nope"})),
    ] {
        let e = refused(&mut s, id, params);
        assert!(e.contains("no table named"), "{id}: {e}");
    }
    let e = refused(
        &mut s,
        "variable.publish",
        json!({"name": "w", "kind": "choice", "choicesFrom": "nope"}),
    );
    assert!(e.contains("no table"), "{e}");
}
