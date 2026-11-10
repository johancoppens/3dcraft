//! An assertion's condition (`when`, format 46) and lazy expressions over
//! outputs (docs/design/verification.md › *Conditions*): an assertion
//! whose `when` is 0 is inactive, counted as neither passed nor failed nor
//! unproven, in assert.add, assert.list, document.check and an edit's
//! summary; a branch of `if` that is not taken does not need its output,
//! so an output that is unknown there leaves neither an output nor an
//! assertion undecided; and the sheet-metal measures over nothing
//! (`cornerGap` with every corner open, `holeToBend` with no hole beside a
//! bend) are errors that say how a `when` avoids them. From two Jigs'
//! findings (the reproducers repro-sheet-metal-enclosure-7 and
//! repro-rack-4, kept outside the repository).

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

/// The assertion named `name` as `assert.list` gives it.
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

/// The output named `name` as `output.list` gives it.
fn output(s: &mut Session, name: &str) -> Value {
    let list = run(s, "output.list", json!({}));
    list["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["name"] == name)
        .cloned()
        .unwrap_or_else(|| panic!("no output {name}: {list}"))
}

/// A 10 × 20 × 30 block (volume 6000) and a switch `on`.
fn block() -> Session {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "variable.set", json!({"name": "on", "expr": "1"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "B", "size": [10, 20, 30]}),
    );
    s
}

#[test]
fn an_assertion_whose_condition_is_zero_is_inactive_and_counts_as_nothing() {
    let mut s = block();
    // On: measured as any other.
    let added = run(
        &mut s,
        "assert.add",
        json!({"name": "vol", "measure": "volume", "of": "B", "eq": "6000", "when": "on"}),
    );
    assert_eq!(added["status"], "passed", "{added}");
    assert_eq!(added["when"], "on");
    // One that would fail, switched off from the start.
    let off = run(
        &mut s,
        "assert.add",
        json!({"name": "wrong", "measure": "volume", "of": "B", "eq": "1 mm^3", "when": "on == 0"}),
    );
    assert_eq!(off["status"], "inactive", "{off}");
    assert!(off["measured"].is_null(), "{off}");
    assert!(
        off["message"].as_str().unwrap().contains("`on == 0` is 0"),
        "{off}"
    );
    let list = run(&mut s, "assert.list", json!({}));
    assert_eq!(list["count"], 2);
    assert_eq!(list["passed"], 1);
    assert_eq!(list["inactive"], 1);
    assert_eq!(list["failed"], 0);
    assert_eq!(list["notProven"], 0);
    assert_eq!(list["errors"], 0);
    assert_eq!(list["ok"], true, "{list}");
    let check = run(&mut s, "document.check", json!({}));
    assert_eq!(check["ok"], true, "{check}");
    assert!(check["warnings"].as_array().is_none_or(Vec::is_empty));
    // Flipping the switch flips both, and the edit's summary says so.
    let edit = run(
        &mut s,
        "variable.set",
        json!({"name": "on", "expr": "0", "wait": true}),
    );
    let summary = &edit["assertionSummary"];
    assert_eq!(summary["inactive"], 1, "{edit}");
    // The one switched on is measured, and broken.
    assert_eq!(summary["broken"][0]["name"], "wrong", "{edit}");
    assert_eq!(summary["broken"].as_array().unwrap().len(), 1, "{edit}");
    assert_eq!(summary["passed"], 0, "{edit}");
    assert_eq!(listed(&mut s, "vol")["status"], "inactive");
    assert_eq!(listed(&mut s, "wrong")["status"], "failed");
    let check = run(&mut s, "document.check", json!({}));
    assert_eq!(check["ok"], false, "{check}");
    let list = run(&mut s, "assert.list", json!({}));
    assert_eq!(list["ok"], false, "{list}");
    assert_eq!(list["inactive"], 1);
    // Everything off: ok, as with nothing to check.
    run(&mut s, "assert.edit", json!({"name": "wrong", "when": "0"}));
    let list = run(&mut s, "assert.list", json!({}));
    assert_eq!(
        (list["inactive"].clone(), list["ok"].clone()),
        (json!(2), json!(true))
    );
    // assert.edit without when keeps it; null takes it away.
    run(&mut s, "assert.edit", json!({"name": "vol", "eq": "6000"}));
    assert_eq!(listed(&mut s, "vol")["when"], "on");
    let edited = run(&mut s, "assert.edit", json!({"name": "vol", "when": null}));
    assert_eq!(edited["status"], "passed", "{edited}");
    assert!(edited["when"].is_null());
    // Undo brings the condition back.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(listed(&mut s, "vol")["status"], "inactive");
}

#[test]
fn a_condition_follows_a_renamed_variable_and_survives_a_save() {
    let dir = workdir("assertion-when", "save");
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "variable.set", json!({"name": "on", "expr": "0"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "B", "size": [10, 20, 30]}),
    );
    run(
        &mut s,
        "assert.add",
        json!({"name": "vol", "measure": "volume", "of": "B", "eq": "1 mm^3", "when": "on > 0"}),
    );
    run(
        &mut s,
        "variable.rename",
        json!({"name": "on", "to": "lid"}),
    );
    assert_eq!(listed(&mut s, "vol")["when"], "lid > 0");
    // A variable a condition uses is in use.
    let e = error(&mut s, "variable.delete", json!({"name": "lid"}));
    assert!(e.contains("vol"), "{e}");
    run(&mut s, "file.save", json!({"path": "w.3dcraft"}));
    let mut t = session_in(&dir);
    run(&mut t, "file.open", json!({"path": "w.3dcraft"}));
    let back = listed(&mut t, "vol");
    assert_eq!(back["when"], "lid > 0");
    assert_eq!(back["status"], "inactive", "{back}");
    run(&mut t, "variable.set", json!({"name": "lid", "expr": "1"}));
    assert_eq!(listed(&mut t, "vol")["status"], "failed");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_bad_condition_is_refused_or_an_error_never_a_crash() {
    let mut s = block();
    for bad in [json!("1 +"), json!(true), json!([1]), json!({"a": 1})] {
        let e = error(
            &mut s,
            "assert.add",
            json!({"name": "x", "measure": "volume", "eq": "1 mm^3", "when": bad}),
        );
        assert!(e.contains("when"), "{e}");
    }
    // A condition with a unit, or one naming nothing, is the assertion's
    // error (as a feature's fails the feature).
    let with_unit = run(
        &mut s,
        "assert.add",
        json!({"name": "unit", "measure": "volume", "eq": "6000", "when": "2 mm"}),
    );
    assert_eq!(with_unit["status"], "error", "{with_unit}");
    assert!(
        with_unit["message"]
            .as_str()
            .unwrap()
            .contains("plain number"),
        "{with_unit}"
    );
    let unknown = run(
        &mut s,
        "assert.add",
        json!({"name": "typo", "measure": "volume", "eq": "6000", "when": "nosuch"}),
    );
    assert_eq!(unknown["status"], "error", "{unknown}");
    let list = run(&mut s, "assert.list", json!({}));
    assert_eq!(list["errors"], 2);
    assert_eq!(list["ok"], false);
    // The number 0 written as a number.
    let zero = run(
        &mut s,
        "assert.add",
        json!({"name": "zero", "measure": "volume", "eq": "1 mm^3", "when": 0}),
    );
    assert_eq!(zero["status"], "inactive", "{zero}");
}

/// The enclosure Jig's finding: its corner kind is a choice (1 open,
/// 2 butt), and cornerGap has no value while every corner is open.
fn enclosure(corner: u32) -> Session {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "variable.set",
        json!({"name": "corner", "expr": corner.to_string()}),
    );
    run(
        &mut s,
        "variable.set",
        json!({"name": "gap", "expr": "0.5 mm"}),
    );
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "name": "BoxBase"}),
    );
    run(
        &mut s,
        "sketch.rect",
        json!({"sketch": "BoxBase", "from": [0, 0], "to": [120, 80]}),
    );
    run(
        &mut s,
        "feature.sheetMetal",
        json!({"sketch": "BoxBase", "name": "Box", "thickness": "1.5 mm", "radius": "2 mm", "kFactor": 0.42,
            "relief": {"shape": "round", "size": "4 mm"}, "corner": {"kind": "corner", "gap": "gap"},
            "flanges": [{"edge": "e5", "length": "20 mm", "angle": "90 deg"}, {"edge": "e6", "length": "20 mm", "angle": "90 deg"},
                        {"edge": "e7", "length": "20 mm", "angle": "90 deg"}, {"edge": "e8", "length": "20 mm", "angle": "90 deg"}]}),
    );
    run(
        &mut s,
        "variable.measure",
        json!({"name": "cg", "measure": "cornerGap"}),
    );
    run(
        &mut s,
        "variable.output",
        json!({"name": "pick", "expr": "if(corner == 1, gap, cg)"}),
    );
    run(
        &mut s,
        "assert.add",
        json!({"name": "Gap", "measure": "cornerGap", "eq": "gap", "when": "corner != 1"}),
    );
    run(
        &mut s,
        "assert.add",
        json!({"name": "Lazy", "measure": "value", "expr": "if(corner == 1, gap, cg)", "eq": "gap"}),
    );
    run(
        &mut s,
        "assert.add",
        json!({"name": "Picked", "measure": "value", "expr": "pick", "eq": "gap"}),
    );
    run(
        &mut s,
        "assert.add",
        json!({"name": "Bound", "measure": "value", "expr": "gap", "max": "if(corner == 1, 1 mm, cg + 0.1 mm)"}),
    );
    s
}

#[test]
fn a_branch_not_taken_does_not_need_its_unknown_output() {
    let mut s = enclosure(1);
    // Every corner open: the measure has no value, honestly ...
    let cg = output(&mut s, "cg");
    assert_eq!(cg["status"], "unknown", "{cg}");
    let why = cg["message"].as_str().unwrap();
    assert!(why.contains("no corner of `Box` is closed"), "{why}");
    assert!(why.contains("when"), "{why}");
    // ... but nothing that does not need it is undecided.
    let pick = output(&mut s, "pick");
    assert_eq!(pick["status"], "known", "{pick}");
    assert!(
        (pick["value"].as_f64().unwrap() - 0.5).abs() < 1e-12,
        "{pick}"
    );
    assert_eq!(listed(&mut s, "Gap")["status"], "inactive");
    for name in ["Lazy", "Picked", "Bound"] {
        let a = listed(&mut s, name);
        assert_eq!(a["status"], "passed", "{name}: {a}");
    }
    let list = run(&mut s, "assert.list", json!({}));
    assert_eq!(list["ok"], true, "{list}");
    assert_eq!(list["notProven"], 0, "{list}");
    assert_eq!(run(&mut s, "document.check", json!({}))["ok"], true);
    // The branch that needs it, taken: the unknown output undecides what
    // uses it, with its reason.
    run(
        &mut s,
        "variable.output",
        json!({"name": "pick", "expr": "if(corner == 2, gap, cg)"}),
    );
    let pick = output(&mut s, "pick");
    assert_eq!(pick["status"], "unknown", "{pick}");
    assert!(
        pick["message"]
            .as_str()
            .unwrap()
            .contains("it uses `cg`, which is unknown"),
        "{pick}"
    );
    let picked = listed(&mut s, "Picked");
    assert_eq!(picked["status"], "notProven", "{picked}");
    assert!(
        picked["message"].as_str().unwrap().contains("`pick`"),
        "{picked}"
    );
    run(
        &mut s,
        "assert.edit",
        json!({"name": "Lazy", "measure": "value", "expr": "if(corner == 2, gap, cg)"}),
    );
    let lazy = listed(&mut s, "Lazy");
    assert_eq!(lazy["status"], "notProven", "{lazy}");
    assert!(lazy["message"].as_str().unwrap().contains("`cg`"), "{lazy}");
}

#[test]
fn a_closed_corner_switches_the_gap_assertion_on_and_takes_the_other_branch() {
    let mut s = enclosure(1);
    run(
        &mut s,
        "variable.set",
        json!({"name": "corner", "expr": "2"}),
    );
    let cg = output(&mut s, "cg");
    assert_eq!(cg["status"], "known", "{cg}");
    for name in ["Gap", "Lazy", "Picked", "Bound"] {
        let a = listed(&mut s, name);
        assert_eq!(a["status"], "passed", "{name}: {a}");
    }
    let list = run(&mut s, "assert.list", json!({}));
    assert_eq!(
        (list["passed"].clone(), list["inactive"].clone()),
        (json!(4), json!(0))
    );
}

/// The rack Jig's finding: a plate with one flange and a hole cut by a
/// later boolean, which is not a hole of the part as it was built.
#[test]
fn hole_to_bend_without_a_hole_says_why_and_a_condition_switches_it_off() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "variable.set", json!({"name": "vent", "expr": "1"}));
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "name": "A", "offset": 5}),
    );
    run(
        &mut s,
        "sketch.rect",
        json!({"sketch": "A", "from": [10, 10], "to": [110, 60]}),
    );
    run(
        &mut s,
        "feature.sheetMetal",
        json!({"sketch": "A", "name": "Plate", "thickness": "2 mm", "radius": "2 mm", "kFactor": 0.4,
            "flanges": [{"edge": "e5", "length": "15 mm", "angle": "90 deg"}]}),
    );
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "name": "Hole", "offset": 0}),
    );
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Hole", "center": ["60 mm", "40 mm"], "radius": "6 mm"}),
    );
    run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Hole", "distance": "20 mm", "name": "Cutter", "when": "vent"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Plate", "tool": "Cutter", "name": "Vented", "when": "vent"}),
    );
    let a = run(
        &mut s,
        "assert.add",
        json!({"name": "hole to bend", "measure": "holeToBend", "of": "Plate", "max": "100 mm"}),
    );
    assert_eq!(a["status"], "error", "{a}");
    let why = a["message"].as_str().unwrap();
    assert!(why.contains("`Plate` has no holes"), "{why}");
    assert!(why.contains("`Plate` went on into `Vented`"), "{why}");
    assert!(why.contains("cutouts"), "{why}");
    assert!(why.contains("when"), "{why}");
    // Written for a design that may have no such hole: off while it has
    // none (here never, as the hole is not the part's).
    let a = run(
        &mut s,
        "assert.edit",
        json!({"name": "hole to bend", "when": "0"}),
    );
    assert_eq!(a["status"], "inactive", "{a}");
    assert_eq!(run(&mut s, "document.check", json!({}))["ok"], true);
}

/// A condition that needs an unknown output leaves the assertion
/// undecided: notProven with that output's reason, neither an error nor
/// inactive.
#[test]
fn a_condition_that_needs_an_unknown_output_is_not_proven() {
    let mut s = enclosure(1);
    let a = run(
        &mut s,
        "assert.add",
        json!({"name": "When cg", "measure": "value", "expr": "gap", "eq": "gap", "when": "cg > 0 mm"}),
    );
    assert_eq!(a["status"], "notProven", "{a}");
    assert!(a["message"].as_str().unwrap().contains("`cg`"), "{a}");
    // Not needed (the branch is not taken): on, and measured.
    let a = run(
        &mut s,
        "assert.edit",
        json!({"name": "When cg", "when": "if(corner == 1, 1, cg > 0 mm)"}),
    );
    assert_eq!(a["status"], "passed", "{a}");
}

/// An error that is not about the unknown output an earlier evaluation of
/// the same check needed (the `value` expression, read for its dimension)
/// is the assertion's error, not notProven for that output.
#[test]
fn an_unrelated_error_is_not_blamed_on_an_unknown_output() {
    let mut s = enclosure(1);
    let a = run(
        &mut s,
        "assert.add",
        json!({"name": "Typo", "measure": "value", "expr": "cg", "eq": "gap", "when": "nosuch"}),
    );
    assert_eq!(a["status"], "error", "{a}");
    let why = a["message"].as_str().unwrap();
    assert!(why.contains("condition") && why.contains("nosuch"), "{a}");
    // The same assertion with a condition that works out: it is the
    // output it needs that leaves it undecided.
    let a = run(&mut s, "assert.edit", json!({"name": "Typo", "when": "1"}));
    assert_eq!(a["status"], "notProven", "{a}");
    assert!(a["message"].as_str().unwrap().contains("`cg`"), "{a}");
}

/// An edit of the condition alone, with no variable in it, is seen: the
/// condition is part of what an assertion's result is kept by (it was
/// not, and the old result came back).
#[test]
fn editing_only_the_condition_changes_the_status() {
    let mut s = block();
    let a = run(
        &mut s,
        "assert.add",
        json!({"name": "vol", "measure": "volume", "of": "B", "eq": "6000", "when": "1"}),
    );
    assert_eq!(a["status"], "passed", "{a}");
    let a = run(&mut s, "assert.edit", json!({"name": "vol", "when": "0"}));
    assert_eq!(a["status"], "inactive", "{a}");
    assert_eq!(listed(&mut s, "vol")["status"], "inactive");
    let a = run(&mut s, "assert.edit", json!({"name": "vol", "when": "2"}));
    assert_eq!(a["status"], "passed", "{a}");
    let a = run(
        &mut s,
        "assert.edit",
        json!({"name": "vol", "when": "1 mm"}),
    );
    assert_eq!(a["status"], "error", "{a}");
    // document.inspect shows the condition, as assert.list does.
    let inspect = run(&mut s, "document.inspect", json!({"measures": false}));
    assert_eq!(inspect["assertions"][0]["when"], "1 mm", "{inspect}");
}

/// Outputs that use each other lazily are still a cycle: the check is on
/// the names written, so `A`, whose taken branch needs `B`, and `B`, whose
/// untaken branch names `A`, are refused.
#[test]
fn a_lazy_mutual_pair_of_outputs_is_refused_as_a_cycle() {
    let mut s = block();
    run(
        &mut s,
        "variable.output",
        json!({"name": "A", "expr": "1 mm"}),
    );
    run(
        &mut s,
        "variable.output",
        json!({"name": "B", "expr": "if(on == 1, 2 mm, A)"}),
    );
    let e = error(
        &mut s,
        "variable.output",
        json!({"name": "A", "expr": "if(on == 1, B, 0 mm)"}),
    );
    assert!(e.contains("A → B → A") || e.contains("A → B"), "{e}");
}

/// `document.inspect` gives what an assertion measured as `assert.list`
/// does: the number with its unit apart (it gave one line of text).
#[test]
fn inspect_gives_measured_as_a_number_with_its_unit() {
    let mut s = block();
    run(
        &mut s,
        "assert.add",
        json!({"name": "vol", "measure": "volume", "of": "B", "eq": "6000"}),
    );
    run(
        &mut s,
        "assert.add",
        json!({"name": "thru", "measure": "through", "of": "B", "is": false}),
    );
    let inspect = run(&mut s, "document.inspect", json!({"measures": false}));
    let shown = inspect["assertions"].as_array().unwrap();
    for name in ["vol", "thru"] {
        let listed = listed(&mut s, name);
        let inspected = shown.iter().find(|a| a["name"] == name).unwrap();
        assert_eq!(inspected["measured"], listed["measured"], "{inspect}");
        assert_eq!(inspected["unit"], listed["unit"], "{inspect}");
    }
    let vol = shown.iter().find(|a| a["name"] == "vol").unwrap();
    assert!(
        (vol["measured"].as_f64().unwrap() - 6000.0).abs() < 1e-6,
        "{vol}"
    );
    assert_eq!(vol["unit"], "mm^3", "{vol}");
}

/// A `value` whose expression fails for a reason of its own (units that
/// do not add up, a name that is not there) says that error, not that
/// its bounds cannot be read in a dimension the expression does not have.
#[test]
fn a_value_whose_expression_fails_says_its_own_error() {
    let mut s = block();
    for (expr, says) in [("1 mm + 1 mm^2", "`expr`"), ("nosuch + 1", "nosuch")] {
        let a = run(
            &mut s,
            "assert.add",
            json!({"name": "v", "measure": "value", "expr": expr, "eq": "2 mm"}),
        );
        assert_eq!(a["status"], "error", "{a}");
        let why = a["message"].as_str().unwrap();
        assert!(why.contains(says), "{a}");
        assert!(!why.contains("what it must be"), "{a}");
        run(&mut s, "assert.delete", json!({"name": "v"}));
    }
}
