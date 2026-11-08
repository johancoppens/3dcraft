//! What an agent reads back from a command: the friction the user guide's
//! writer found before the R1 field test (2026-10-22). A variable in use
//! is not deleted from under its users, an edit says which features it
//! made fail, a refusal says its numbers, lengths are expressions, and
//! every error names its command.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::run;
use common::{error, session, session_in};
use craft3d_engine::Session;
use serde_json::{Value, json};

fn workdir(name: &str) -> std::path::PathBuf {
    common::workdir("agent-results", name)
}

/// A block driven by `w`, `h` (through `d`, a variable of its own), a
/// round on one of its edges of radius `r`, a sketch with a point held at
/// `w`, an assertion that names `w`, and a set whose filter does.
fn part(s: &mut Session) {
    run(s, "file.new", json!({"units": "mm"}));
    run(s, "variable.set", json!({"name": "w", "expr": "40 mm"}));
    run(s, "variable.set", json!({"name": "h", "expr": "10 mm"}));
    run(s, "variable.set", json!({"name": "d", "expr": "h * 2"}));
    run(s, "variable.set", json!({"name": "r", "expr": "2 mm"}));
    run(s, "variable.set", json!({"name": "spare", "expr": "1"}));
    run(
        s,
        "feature.block",
        json!({"size": ["w", "d", "h"], "name": "Block"}),
    );
    run(
        s,
        "feature.fillet",
        json!({"edges": [["F1:face(1)", "F1:face(2)"]], "radius": "r", "name": "Round"}),
    );
    run(s, "sketch.create", json!({"plane": "xy", "name": "Marks"}));
    run(
        s,
        "sketch.point",
        json!({"sketch": "Marks", "at": ["w / 2", 0]}),
    );
    run(
        s,
        "assert.add",
        json!({"name": "wide", "measure": "volume", "of": "Round", "min": "w * h * 1 mm"}),
    );
    run(
        s,
        "set.define",
        json!({"name": "big", "kind": "face", "where": {"area": {"min": "w * h"}}}),
    );
}

fn variables(s: &mut Session) -> Vec<String> {
    run(s, "document.inspect", json!({"measures": false}))["variables"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["name"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn a_variable_in_use_is_not_deleted_and_its_users_are_named() {
    let mut s = session();
    part(&mut s);
    // Every kind of user is named, with the parameters of a feature.
    let refused = error(&mut s, "variable.delete", json!({"name": "w"}));
    assert!(
        refused.starts_with("variable.delete: `w` is still used by "),
        "{refused}"
    );
    for user in [
        "the feature `Block` (F1: size.x)",
        "the sketch `Marks` (F3: ",
        "the assertion `wide`",
        "the set `big`",
        "force: true",
    ] {
        assert!(refused.contains(user), "{user}: {refused}");
    }
    // A variable used by another variable only.
    let refused = error(&mut s, "variable.delete", json!({"name": "h"}));
    assert!(refused.contains("the variable `d`"), "{refused}");
    // Nothing was deleted.
    assert!(variables(&mut s).contains(&"w".to_string()));
    // One nothing uses goes as before.
    let deleted = run(&mut s, "variable.delete", json!({"name": "spare"}));
    assert_eq!(deleted, json!({"deleted": "spare"}));
    // With force it goes, says who used it and which features now fail.
    let forced = run(
        &mut s,
        "variable.delete",
        json!({"name": "r", "force": true}),
    );
    assert_eq!(forced["deleted"], "r");
    assert_eq!(
        forced["usedBy"],
        json!(["the feature `Round` (F2: radius)"])
    );
    assert_eq!(forced["failed"][0]["feature"], "Round", "{forced}");
    assert_eq!(forced["failed"][0]["id"], "F2", "{forced}");
    assert!(
        forced["failed"][0]["error"].as_str().unwrap().contains('r'),
        "{forced}"
    );
    // Undo brings it back, and the round with it.
    run(&mut s, "edit.undo", json!({}));
    assert!(variables(&mut s).contains(&"r".to_string()));
    let check = run(&mut s, "document.check", json!({}));
    assert_eq!(check["ok"], true, "{check}");
    // A wrong type for force is an error, not a crash.
    let wrong = error(
        &mut s,
        "variable.delete",
        json!({"name": "r", "force": "yes"}),
    );
    assert!(wrong.contains("force"), "{wrong}");
}

#[test]
fn an_edit_says_which_features_it_made_fail() {
    let mut s = session();
    part(&mut s);
    // A change that breaks nothing says nothing about failures.
    let fine = run(&mut s, "variable.set", json!({"name": "r", "expr": "3 mm"}));
    assert!(fine.get("failed").is_none(), "{fine}");
    // A round larger than the block is high fails, with its reason.
    let broke = run(
        &mut s,
        "variable.set",
        json!({"name": "r", "expr": "30 mm"}),
    );
    let failed = broke["failed"].as_array().unwrap();
    assert_eq!(failed.len(), 1, "{broke}");
    assert_eq!(failed[0]["feature"], "Round");
    assert_eq!(failed[0]["id"], "F2");
    assert!(
        failed[0]["error"].as_str().unwrap().starts_with("radius"),
        "{broke}"
    );
    // The assertions are still reported beside it.
    assert!(broke.get("assertionSummary").is_some(), "{broke}");
    // Already failing: not made to fail by the next change.
    let again = run(
        &mut s,
        "variable.set",
        json!({"name": "w", "expr": "41 mm"}),
    );
    assert!(again.get("failed").is_none(), "{again}");
    run(&mut s, "variable.set", json!({"name": "r", "expr": "2 mm"}));
    // feature.edit names what failed downstream, not the edited feature
    // (whose own status is in the result).
    let edited = run(
        &mut s,
        "feature.edit",
        json!({"feature": "Block", "size": ["w", "d", "1 mm"]}),
    );
    assert_eq!(edited["status"], "ok", "{edited}");
    assert_eq!(edited["failed"][0]["feature"], "Round", "{edited}");
    // So does deleting. Suppressed, the block's edges are not there for
    // the round, which then has nothing to do and does not fail
    // (docs/design/regeneration.md › Switched off).
    run(&mut s, "edit.undo", json!({}));
    let suppressed = run(&mut s, "feature.suppress", json!({"feature": "Block"}));
    assert!(
        suppressed["failed"]
            .as_array()
            .is_none_or(|failed| failed.iter().all(|f| f["feature"] != "Round")),
        "{suppressed}"
    );
    let round = run(&mut s, "document.inspect", json!({}))["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == "Round")
        .unwrap()
        .clone();
    assert_eq!(round["status"], "ok", "{round}");
    assert_eq!(round["passesNothing"], "`Block` is suppressed", "{round}");
    run(&mut s, "edit.undo", json!({}));
    let deleted = run(&mut s, "feature.delete", json!({"feature": "Block"}));
    assert_eq!(deleted["failed"][0]["feature"], "Round", "{deleted}");
}

#[test]
fn the_round_of_a_rim_too_large_for_a_torus_says_what_fits() {
    // The guide's flange: a disc of radius 40, a round of 20 on its rim.
    // Where the faces are square the ball's centre is 40 - r from the
    // axis: as far as the radius at r = 20, on the axis at r = 40
    // (closed form). The message was garbled until 2026-10-22.
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": "40 mm", "height": "10 mm", "name": "Disc"}),
    );
    let round = run(
        &mut s,
        "feature.fillet",
        json!({"edges": [["F1:face(0)", "F1:face(2)"]], "radius": "20 mm", "name": "Round"}),
    );
    assert_eq!(round["status"], "failed", "{round}");
    let reason = round["error"].as_str().unwrap();
    for words in [
        "edges.0 (between F1:face(0) and F1:face(2))",
        "with a radius of 20 mm",
        "torus that reaches its own axis",
        "every radius from 20 mm up to 40 mm",
        "below 20 mm the ball's centre stays far enough from the axis",
        "exactly 40 mm puts it on the axis",
    ] {
        assert!(reason.contains(words), "{words}: {reason}");
    }
    // What it says fits does (below the disc's height too).
    let fits = run(
        &mut s,
        "feature.edit",
        json!({"feature": "Round", "radius": "9 mm"}),
    );
    assert_eq!(fits["status"], "ok", "{fits}");
}

#[test]
fn the_gap_of_a_sheet_is_an_expression() {
    let dir = workdir("gap");
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "variable.set", json!({"name": "g", "expr": "4 mm"}));
    run(&mut s, "feature.block", json!({"size": [10, 20, 30]}));
    let faces = json!(["F1:face(0)", "F1:face(1)"]);
    let export = |s: &mut Session, gap: Value| {
        run(
            s,
            "file.export",
            json!({"path": "sheet.dxf", "faces": faces, "gap": gap, "width": "1 m"}),
        )
    };
    let by_number = export(&mut s, json!(6));
    let by_expr = export(&mut s, json!("g + 0.2 cm"));
    assert_eq!(by_number["gap"], json!(6.0), "{by_number}");
    assert_eq!(by_expr["gap"], by_number["gap"], "{by_expr}");
    // Of another dimension, or negative: refused, saying which.
    let wrong = error(
        &mut s,
        "file.export",
        json!({"path": "sheet.dxf", "faces": faces, "gap": "5 deg"}),
    );
    assert!(wrong.starts_with("file.export: `gap`: "), "{wrong}");
    let negative = error(
        &mut s,
        "file.export",
        json!({"path": "sheet.dxf", "faces": faces, "gap": "-g"}),
    );
    assert!(
        negative.contains("`gap` must be a length of 0 or more, not -4 mm"),
        "{negative}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn errors_of_paths_name_their_command() {
    let dir = workdir("roots");
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    // An absolute path on this platform (Windows' needs a drive).
    let away = if cfg!(windows) {
        r"C:\x.3dcraft"
    } else {
        "/x.3dcraft"
    };
    let outside = error(&mut s, "file.save", json!({"path": away}));
    assert!(
        outside.starts_with(&format!("file.save: `{away}` is outside the write root")),
        "{outside}"
    );
    let outside = error(&mut s, "file.open", json!({"path": away}));
    assert!(
        outside.starts_with(&format!("file.open: `{away}` is outside the read root")),
        "{outside}"
    );
    let mut sealed = session();
    run(&mut sealed, "file.new", json!({"units": "mm"}));
    let denied = error(&mut sealed, "file.export", json!({"path": "a.step"}));
    assert!(
        denied.starts_with("file.export: this session has no write access"),
        "{denied}"
    );
    // A message that names its command already is not named twice.
    let unknown = error(&mut sealed, "feature.delete", json!({"feature": "Nope"}));
    assert!(unknown.starts_with("feature.delete: "), "{unknown}");
    assert!(
        !unknown.contains("feature.delete: feature.delete"),
        "{unknown}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_sketch_point_says_whether_it_is_held() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "sketch.create", json!({"plane": "xy", "name": "S"}));
    let free = run(&mut s, "sketch.point", json!({"sketch": "S", "at": [0, 0]}));
    assert_eq!(free["held"], false, "{free}");
    assert_eq!(free["constraints"], json!([]), "{free}");
    let held = run(
        &mut s,
        "sketch.point",
        json!({"sketch": "S", "at": [0, "0 mm"]}),
    );
    assert_eq!(held["held"], true, "{held}");
    assert_eq!(held["constraints"].as_array().unwrap().len(), 2, "{held}");
}

/// The names of the features in `failed` of a result.
fn failed_names(result: &Value) -> Vec<String> {
    result
        .get("failed")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .map(|f| f["feature"].as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// Undo and redo do not regenerate: `failed` is measured against the
/// document as it is before the edit, not against the last regeneration
/// of another revision (the review of 2026-10-22: a failure missed after
/// an undo, an unrelated edit blamed for what the undo did). Commands run
/// plainly, so that no check regenerates in between.
#[test]
fn failed_is_measured_against_the_revision_the_edit_starts_from() {
    use common::plain::run;
    let mut s = session();
    part(&mut s);
    let set = |s: &mut Session, name: &str, expr: &str| {
        run(s, "variable.set", json!({"name": name, "expr": expr}))
    };
    let none: Vec<String> = Vec::new();
    assert_eq!(failed_names(&set(&mut s, "r", "30 mm")), ["Round"]);
    run(&mut s, "edit.undo", json!({}));
    // Undone (r is 2 again, nothing regenerated): the same edit breaks
    // the round again, and says so.
    assert_eq!(failed_names(&set(&mut s, "r", "30 mm")), ["Round"]);
    assert_eq!(failed_names(&set(&mut s, "r", "2 mm")), none);
    // Undone to r = 30 (the round fails again, by the undo): an unrelated
    // edit made nothing fail.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(failed_names(&set(&mut s, "spare", "2")), none);
    // Back to r = 2 by undo, to 30 by redo: mended, then broken again.
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(failed_names(&set(&mut s, "r", "3 mm")), none);
    assert_eq!(failed_names(&set(&mut s, "r", "31 mm")), ["Round"]);
}

/// Variables inside the subject of an assertion and in a set's filter
/// are uses; a string that names an axis or a plane is not.
#[test]
fn uses_inside_subject_queries_count_and_axis_names_do_not() {
    let mut s = session();
    part(&mut s);
    run(
        &mut s,
        "variable.set",
        json!({"name": "hr", "expr": "7 mm"}),
    );
    run(
        &mut s,
        "assert.add",
        json!({"name": "no 7 mm holes", "measure": "count",
               "of": {"kind": "face", "where": {"radius": "hr"}}, "eq": "0"}),
    );
    let refused = error(&mut s, "variable.delete", json!({"name": "hr"}));
    assert!(
        refused.contains("the assertion `no 7 mm holes`"),
        "{refused}"
    );
    run(
        &mut s,
        "variable.set",
        json!({"name": "sr", "expr": "2 mm"}),
    );
    run(
        &mut s,
        "set.define",
        json!({"name": "rounds", "kind": ["face"], "where": {"any": [{"radius": "sr"}]}}),
    );
    let refused = error(&mut s, "variable.delete", json!({"name": "sr"}));
    assert!(refused.contains("the set `rounds`"), "{refused}");
    // A variable called x, beside an assertion along the axis x and a
    // set of faces facing x: neither uses it.
    run(&mut s, "variable.set", json!({"name": "x", "expr": "1 mm"}));
    run(
        &mut s,
        "assert.add",
        json!({"name": "long", "measure": "extent", "of": "Round", "axis": "x", "min": "1 mm"}),
    );
    run(
        &mut s,
        "set.define",
        json!({"name": "ends", "kind": "face", "where": {"normal": {"along": "x"}}}),
    );
    let deleted = run(&mut s, "variable.delete", json!({"name": "x"}));
    assert_eq!(deleted, json!({"deleted": "x"}));
}

/// A rectangle 30 mm by `c5` (20 mm) extruded 10 mm, its bottom top edge
/// rounded by 3 mm: features made from a sketch, for sketch edits to
/// break. The points are e1 to e4 from the origin round, the lines e5 to
/// e8 from the bottom one round.
fn profile(s: &mut Session) {
    use common::plain::run;
    run(s, "file.new", json!({"units": "mm"}));
    run(
        s,
        "sketch.create",
        json!({"plane": "xy", "name": "Outline"}),
    );
    let rect = run(
        s,
        "sketch.rect",
        json!({"sketch": "Outline", "from": [0, 0], "to": [30, 20]}),
    );
    assert_eq!(rect["lines"][0], "e5", "{rect}");
    let depth = run(
        s,
        "sketch.constrain",
        json!({"sketch": "Outline", "type": "distance", "entities": ["e2", "e3"], "value": "20 mm"}),
    );
    assert_eq!(depth["constraint"], "c5", "{depth}");
    run(
        s,
        "feature.extrude",
        json!({"sketch": "Outline", "distance": "10 mm", "name": "Plate"}),
    );
    let round = run(
        s,
        "feature.fillet",
        json!({"edges": [["F2:side(e5)", "F2:endCap()"]], "radius": "3 mm", "name": "Round"}),
    );
    assert_eq!(round["status"], "ok", "{round}");
}

/// Every command that changes a sketch says which features made from it
/// the change made fail, as variable.set and feature.edit do (owed since
/// r1-polish, 2026-10-22): an edited dimension, a constraint, an entity
/// added or deleted. Commands run plainly with undo and redo between them,
/// so `failed` is measured against the revision each edit starts from.
#[test]
fn a_sketch_edit_says_which_features_it_made_fail() {
    use common::plain::run;
    let mut s = session();
    profile(&mut s);
    let none: Vec<String> = Vec::new();
    // A dimension that keeps the profile whole breaks nothing.
    let fine = run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "Outline", "constraint": "c5", "value": "25 mm"}),
    );
    assert_eq!(fine["status"], "ok", "{fine}");
    assert_eq!(failed_names(&fine), none, "{fine}");
    // An open profile: the extrusion fails, and the round with it, made
    // from what failed. The sketch carries its own status.
    let opened = run(
        &mut s,
        "sketch.delete",
        json!({"sketch": "Outline", "entity": "e5"}),
    );
    assert_eq!(failed_names(&opened), ["Plate", "Round"], "{opened}");
    assert_eq!(opened["failed"][1]["causedBy"], "F2", "{opened}");
    // Undone (nothing regenerated): drawing a loose point breaks nothing,
    // and deleting the line again breaks both again.
    run(&mut s, "edit.undo", json!({}));
    let point = run(
        &mut s,
        "sketch.point",
        json!({"sketch": "Outline", "at": [50, 50]}),
    );
    assert_eq!(failed_names(&point), none, "{point}");
    run(&mut s, "edit.undo", json!({}));
    let again = run(
        &mut s,
        "sketch.delete",
        json!({"sketch": "Outline", "entity": "e5"}),
    );
    assert_eq!(failed_names(&again), ["Plate", "Round"], "{again}");
    // Closing the gap with a line mends; undone, the next edit finds them
    // failing already and says nothing.
    let mended = run(
        &mut s,
        "sketch.line",
        json!({"sketch": "Outline", "from": "e1", "to": "e2"}),
    );
    assert_eq!(failed_names(&mended), none, "{mended}");
    run(&mut s, "edit.undo", json!({}));
    let still = run(
        &mut s,
        "sketch.point",
        json!({"sketch": "Outline", "at": [60, 60]}),
    );
    assert_eq!(failed_names(&still), none, "{still}");
    // Undo the point and the deletion, redo the deletion: broken by the
    // redo, so a constraint added next made nothing fail.
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.redo", json!({}));
    let constrained = run(
        &mut s,
        "sketch.constrain",
        json!({"sketch": "Outline", "type": "fixed", "entities": ["e1"]}),
    );
    assert_eq!(failed_names(&constrained), none, "{constrained}");
    // Back to the whole profile: a depth too small for the round breaks
    // the round alone.
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    let shallow = run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "Outline", "constraint": "c5", "value": "2 mm"}),
    );
    assert_eq!(shallow["status"], "ok", "{shallow}");
    assert_eq!(failed_names(&shallow), ["Round"], "{shallow}");
}
