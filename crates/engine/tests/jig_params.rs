//! Public parameters, document info and the thumbnail (Jigs, phase 1;
//! docs/decisions.md 2026-10-23, docs/design/document.md › *Public
//! parameters*). The oracles: a block's volume is the product of the
//! parameters that size it; the range, the kind and the dimension of
//! every value are checked before anything changes; a saved Jig opens to
//! the same document, and the same document draws the same thumbnail.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::{assert_close, refused, run, session, session_in, workdir};
use craft3d_engine::Session;
use serde_json::{Value, json};

/// A block `w` × `d` × `h` with `w` at most twice `d`, all public, and
/// an integer, a boolean and a choice that drive nothing.
fn jig(s: &mut Session) {
    run(s, "file.new", json!({"units": "mm"}));
    for (name, expr) in [
        ("w", "40 mm"),
        ("d", "20 mm"),
        ("h", "5 mm"),
        ("n", "3"),
        ("lid", "1"),
        ("style", "2"),
        ("area", "w * d"),
    ] {
        run(s, "variable.set", json!({"name": name, "expr": expr}));
    }
    run(
        s,
        "feature.block",
        json!({"name": "Plate", "size": ["w", "d", "h"]}),
    );
    run(
        s,
        "variable.publish",
        json!({"name": "w", "label": "Width", "group": "Size", "min": "10 mm", "max": "2 * d", "step": "1 mm", "order": 1}),
    );
    run(
        s,
        "variable.publish",
        json!({"name": "d", "label": "Depth", "group": "Size", "min": "5 mm", "max": "100 mm", "order": 2}),
    );
    run(
        s,
        "variable.publish",
        json!({"name": "h", "label": "Thickness", "min": "1 mm", "max": "20 mm", "unit": "in"}),
    );
    run(
        s,
        "variable.publish",
        json!({"name": "n", "kind": "integer", "min": 1, "max": 6, "group": "Holes"}),
    );
    run(
        s,
        "variable.publish",
        json!({"name": "lid", "kind": "boolean", "group": "Holes"}),
    );
    run(
        s,
        "variable.publish",
        json!({"name": "style", "kind": "choice", "choices": ["Plain", "Slotted", "Round"]}),
    );
}

fn volume(s: &mut Session) -> f64 {
    run(s, "document.measure", json!({}))["volume"]
        .as_f64()
        .unwrap()
}

fn param<'a>(list: &'a Value, name: &str) -> &'a Value {
    list["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == name)
        .unwrap_or_else(|| panic!("no parameter {name} in {list}"))
}

#[test]
fn a_jig_lists_its_parameters_in_order_with_ranges_evaluated() {
    let mut s = session();
    jig(&mut s);
    let list = run(&mut s, "param.list", json!({}));
    assert_eq!(list["jig"], true);
    let names: Vec<&str> = list["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    // By order, ties by the variables' order; the derived `area` is not
    // public.
    assert_eq!(names, ["h", "n", "lid", "style", "w", "d"]);
    assert_eq!(list["groups"], json!(["", "Holes", "Size"]));
    let w = param(&list, "w");
    assert_eq!(w["label"], "Width");
    assert_eq!(w["kind"], "number");
    assert_eq!(w["unit"], "mm");
    assert_close(&w["value"], 40.0, "w");
    assert_eq!(w["max"]["expr"], "2 * d");
    assert_close(&w["max"]["value"], 40.0, "max of w");
    assert_close(&w["step"]["value"], 1.0, "step of w");
    assert_eq!(w["atDefault"], true);
    assert_eq!(w["valid"], true);
    // Shown in its own unit: 5 mm in inches.
    let h = param(&list, "h");
    assert_eq!(h["unit"], "in");
    assert_close(&h["value"], 5.0 / 25.4, "h in inches");
    assert_close(&h["max"]["value"], 20.0 / 25.4, "max of h");
    let style = param(&list, "style");
    assert_eq!(style["choices"], json!(["Plain", "Slotted", "Round"]));
    assert_eq!(style["choice"], "Slotted");
    assert_eq!(style["unit"], Value::Null);
    // Without a label, the name.
    assert_eq!(param(&list, "n")["label"], "n");
    // document.inspect carries the description too.
    let inspect = run(&mut s, "document.inspect", json!({"measures": false}));
    let w = inspect["variables"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == "w")
        .unwrap();
    assert_eq!(w["public"]["label"], "Width");
    assert_eq!(w["public"]["kind"], "number");
}

#[test]
fn param_set_applies_a_batch_in_one_step_and_regenerates_once() {
    let mut s = session();
    jig(&mut s);
    assert_close(&json!(volume(&mut s)), 40.0 * 20.0 * 5.0, "start");
    let undo_before =
        run(&mut s, "document.inspect", json!({"measures": false}))["history"]["undo"]
            .as_array()
            .unwrap()
            .len();
    let result = run(
        &mut s,
        "param.set",
        json!({"values": {"d": "30 mm", "w": 50, "h": "0.25 in", "lid": false}}),
    );
    assert_eq!(result["changed"], true);
    assert!(result.get("failed").is_none(), "{result}");
    assert_close(&json!(volume(&mut s)), 50.0 * 30.0 * 6.35, "after");
    let list = run(&mut s, "param.list", json!({}));
    assert_eq!(param(&list, "w")["expr"], "50 mm");
    assert_eq!(param(&list, "lid")["expr"], "0");
    assert_eq!(param(&list, "w")["atDefault"], false);
    let undo = run(&mut s, "document.inspect", json!({"measures": false}))["history"]["undo"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(undo.len(), undo_before + 1);
    run(&mut s, "edit.undo", json!({}));
    assert_close(&json!(volume(&mut s)), 40.0 * 20.0 * 5.0, "undone");
    run(&mut s, "edit.redo", json!({}));

    // A reset goes back to every default in one step.
    run(&mut s, "param.set", json!({"reset": true}));
    assert_close(&json!(volume(&mut s)), 40.0 * 20.0 * 5.0, "reset");
    let list = run(&mut s, "param.list", json!({}));
    assert!(
        list["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["atDefault"] == true),
        "{list}"
    );
    // Setting what is there already changes nothing and adds no step.
    let again = run(&mut s, "param.set", json!({"values": {"w": "40 mm"}}));
    assert_eq!(again["changed"], false);
}

#[test]
fn a_drag_merges_into_one_undo_step() {
    let mut s = session();
    jig(&mut s);
    let steps = |s: &mut Session| {
        run(s, "document.inspect", json!({"measures": false}))["history"]["undo"]
            .as_array()
            .unwrap()
            .len()
    };
    let start = steps(&mut s);
    run(&mut s, "param.set", json!({"values": {"w": 30}}));
    for w in [31, 32, 33] {
        run(
            &mut s,
            "param.set",
            json!({"values": {"w": w}, "merge": true}),
        );
    }
    assert_eq!(steps(&mut s), start + 1);
    // Other parameters do not merge into it.
    run(
        &mut s,
        "param.set",
        json!({"values": {"d": 25}, "merge": true}),
    );
    assert_eq!(steps(&mut s), start + 2);
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_close(&json!(volume(&mut s)), 40.0 * 20.0 * 5.0, "undone drag");
}

#[test]
fn param_set_refuses_what_a_form_would_clamp_and_changes_nothing() {
    let mut s = session();
    jig(&mut s);
    let cases: &[(Value, &str)] = &[
        (
            json!({"w": "50 mm"}),
            "outside its range, from 10 mm to 40 mm (`2 * d`)",
        ),
        (json!({"w": "5 mm"}), "outside its range"),
        (
            json!({"w": "5"}),
            "is a length, and `5` gives a plain number",
        ),
        (json!({"w": "30 deg"}), "is a length"),
        (json!({"w": "d + 1 mm"}), "takes a value, not a formula"),
        (json!({"w": "((("}), "does not read"),
        (json!({"area": "5 mm"}), "`area` is not a public parameter"),
        (json!({"nope": 1}), "not a public parameter"),
        (json!({"n": 2.5}), "not a whole number"),
        (json!({"n": 7}), "outside its range"),
        (json!({"lid": 2}), "not 0 (off) or 1 (on)"),
        (
            json!({"style": 4}),
            "not a choice: a whole number from 1 to 3",
        ),
        (json!({"style": 0}), "not a choice"),
        (json!({"w": true}), "not a boolean"),
        (json!({"w": [1]}), "must be a string"),
        // Moving d would leave w above its range of 2 * d.
        (json!({"d": "15 mm"}), "would make `w` invalid"),
    ];
    let before = run(&mut s, "document.inspect", json!({"measures": false}));
    for (values, expected) in cases {
        let e = refused(&mut s, "param.set", json!({ "values": values }));
        assert!(e.contains(expected), "{values}: {e}");
        assert!(
            e.contains("nothing was changed") || !e.contains("outside"),
            "{e}"
        );
    }
    let after = run(&mut s, "document.inspect", json!({"measures": false}));
    assert_eq!(before["variables"], after["variables"]);
    assert_eq!(before["history"], after["history"]);
    // Both at once is fine.
    run(
        &mut s,
        "param.set",
        json!({"values": {"d": "15 mm", "w": "30 mm"}}),
    );
    assert_close(&json!(volume(&mut s)), 30.0 * 15.0 * 5.0, "both");
    // Bad shapes of the params.
    for params in [
        json!({"values": [1]}),
        json!({"values": {"w": 1}, "reset": "yes"}),
        json!({"other": 1}),
    ] {
        refused(&mut s, "param.set", params);
    }
}

/// With `clamp`, a change that would put another parameter outside a
/// range that depends on it moves that one to the nearest value of its
/// new range, on its step, in the same edit and one undo step, and says
/// so; what is wrong with a value given is still refused.
#[test]
fn param_set_with_clamp_moves_a_dependent_parameter_into_its_new_range() {
    let mut s = session();
    jig(&mut s);
    // w is 40 mm, at most 2 * d; d to 15.3 mm makes that 30.6 mm, and w's
    // step of 1 mm from 10 mm puts it at 30 mm.
    let before = volume(&mut s);
    let result = run(
        &mut s,
        "param.set",
        json!({"values": {"d": "15.3 mm"}, "clamp": true}),
    );
    assert_eq!(result["clamped"], json!([{"name": "w", "expr": "30 mm"}]));
    assert_eq!(result["changed"], true);
    assert_close(&json!(volume(&mut s)), 30.0 * 15.3 * 5.0, "clamped");
    let list = run(&mut s, "param.list", json!({}));
    assert_eq!(param(&list, "w")["expr"], "30 mm");
    assert_eq!(param(&list, "w")["valid"], true);
    // One undo step for both.
    run(&mut s, "edit.undo", json!({}));
    assert_close(&json!(volume(&mut s)), before, "undone");
    // Nothing to clamp: no list.
    let result = run(
        &mut s,
        "param.set",
        json!({"values": {"d": "30 mm"}, "clamp": true}),
    );
    assert!(result.get("clamped").is_none(), "{result}");
    // A value given out of its own range is refused as before.
    let e = refused(
        &mut s,
        "param.set",
        json!({"values": {"w": "500 mm"}, "clamp": true}),
    );
    assert!(e.contains("outside its range"), "{e}");
    // Without clamp the refusal says how to have it clamped.
    let e = refused(&mut s, "param.set", json!({"values": {"d": "6 mm"}}));
    assert!(e.contains("clamp: true"), "{e}");
    refused(
        &mut s,
        "param.set",
        json!({"values": {"d": "6 mm"}, "clamp": "yes"}),
    );
}

/// A drag that starts or stops clamping another parameter on its way
/// stays one undo step: the step is named for the parameter dragged.
#[test]
fn a_drag_across_the_clamp_stays_one_undo_step() {
    let mut s = session();
    jig(&mut s);
    let steps = |s: &mut Session| {
        run(s, "document.inspect", json!({"measures": false}))["history"]["undo"]
            .as_array()
            .unwrap()
            .len()
    };
    let start = steps(&mut s);
    let before = volume(&mut s);
    // w is 40 mm, at most 2 * d: from d = 19 mm on, w is clamped.
    for (i, d) in ["22 mm", "21 mm", "18 mm", "15 mm", "25 mm"]
        .iter()
        .enumerate()
    {
        run(
            &mut s,
            "param.set",
            json!({"values": {"d": d}, "merge": i > 0, "clamp": true}),
        );
    }
    assert_eq!(steps(&mut s), start + 1);
    let list = run(&mut s, "param.list", json!({}));
    assert_eq!(param(&list, "w")["expr"], "30 mm");
    run(&mut s, "edit.undo", json!({}));
    assert_close(&json!(volume(&mut s)), before, "the whole drag undone");
}

#[test]
fn publishing_is_refused_with_a_reason_an_agent_can_act_on() {
    let mut s = session();
    jig(&mut s);
    let cases: &[(Value, &str)] = &[
        (json!({"name": "ghost"}), "there is no variable `ghost`"),
        (json!({"name": "area"}), "is `w * d`, a formula"),
        (json!({"name": "d", "max": "10 mm"}), "outside its range"),
        (json!({"name": "d", "min": "w / 0"}), "does not evaluate"),
        (json!({"name": "d", "min": "nothere"}), "does not evaluate"),
        (json!({"name": "d", "max": "d * 2"}), "uses `d` itself"),
        (
            json!({"name": "d", "min": "5 deg"}),
            "is an angle, and its value a length",
        ),
        (json!({"name": "d", "step": "-1 mm"}), "not positive"),
        (
            json!({"name": "d", "min": "50 mm", "max": "10 mm"}),
            "range is empty",
        ),
        (
            json!({"name": "d", "unit": "deg"}),
            "its unit deg is not one of a length",
        ),
        (json!({"name": "d", "unit": "furlong"}), "is not a unit"),
        (
            json!({"name": "d", "kind": "integer"}),
            "not a whole number",
        ),
        (json!({"name": "d", "kind": "weird"}), "unknown kind"),
        (
            json!({"name": "d", "default": "500 mm"}),
            "its default 500 mm is outside",
        ),
        (json!({"name": "d", "default": "w"}), "is a formula"),
        (json!({"name": "n", "kind": "choice"}), "at least one label"),
        (
            json!({"name": "n", "kind": "choice", "choices": ["a", "b"]}),
            "is not a choice",
        ),
        (
            json!({"name": "n", "kind": "number", "choices": ["a"]}),
            "only a choice has choices",
        ),
        (json!({"name": "n", "choices": "a"}), "list of labels"),
        (
            json!({"name": "n", "order": 1.5}),
            "`order` must be a whole number",
        ),
        (
            json!({"name": "n", "label": "x".repeat(300)}),
            "at most 256 bytes",
        ),
    ];
    let before = run(&mut s, "param.list", json!({}));
    for (params, expected) in cases {
        let e = refused(&mut s, "variable.publish", params.clone());
        assert!(e.contains(expected), "{params}: {e}");
    }
    assert_eq!(run(&mut s, "param.list", json!({})), before);
    // On an empty document there is nothing to publish.
    let mut empty = session();
    run(&mut empty, "file.new", json!({}));
    let e = refused(&mut empty, "variable.publish", json!({"name": "w"}));
    assert!(e.contains("has no variables"), "{e}");
    let e = refused(&mut empty, "param.set", json!({"values": {}}));
    assert!(e.contains("no public parameters"), "{e}");
}

#[test]
fn republishing_changes_only_what_is_given_and_unpublish_keeps_the_value() {
    let mut s = session();
    jig(&mut s);
    run(
        &mut s,
        "variable.publish",
        json!({"name": "w", "description": "Across the plate", "max": null}),
    );
    let w = param(&run(&mut s, "param.list", json!({})), "w").clone();
    assert_eq!(w["label"], "Width");
    assert_eq!(w["description"], "Across the plate");
    assert_eq!(w["max"], Value::Null);
    assert_close(&w["min"]["value"], 10.0, "min kept");
    // A choice that becomes a number loses its choices.
    run(
        &mut s,
        "variable.publish",
        json!({"name": "style", "kind": "number"}),
    );
    let style = param(&run(&mut s, "param.list", json!({})), "style").clone();
    assert!(style.get("choices").is_none(), "{style}");
    let r = run(&mut s, "variable.unpublish", json!({"name": "w"}));
    assert_eq!(r["changed"], true);
    let r = run(&mut s, "variable.unpublish", json!({"name": "w"}));
    assert_eq!(r["changed"], false);
    refused(&mut s, "variable.unpublish", json!({"name": "ghost"}));
    let list = run(&mut s, "param.list", json!({}));
    assert!(
        list["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["name"] != "w")
    );
    assert_close(&json!(volume(&mut s)), 40.0 * 20.0 * 5.0, "value kept");
}

#[test]
fn variable_commands_respect_public_parameters() {
    let mut s = session();
    jig(&mut s);
    // A form can only set a value: a formula is refused while public.
    let e = refused(
        &mut s,
        "variable.set",
        json!({"name": "w", "expr": "2 * d"}),
    );
    assert!(e.contains("variable.unpublish"), "{e}");
    // variable.set is the designer's tool and does not check ranges;
    // document.check says so.
    run(
        &mut s,
        "variable.set",
        json!({"name": "w", "expr": "90 mm"}),
    );
    let check = run(&mut s, "document.check", json!({}));
    assert_eq!(check["ok"], false);
    let problem = check["problems"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["parameter"] == "w")
        .unwrap();
    assert!(
        problem["problem"]
            .as_str()
            .unwrap()
            .contains("outside its range"),
        "{problem}"
    );
    let list = run(&mut s, "param.list", json!({}));
    assert_eq!(param(&list, "w")["valid"], false);
    // A param.set of another parameter is not blocked by w being off
    // already, but one that leaves it off is.
    run(&mut s, "param.set", json!({"values": {"h": "6 mm"}}));
    run(&mut s, "param.set", json!({"values": {"w": "40 mm"}}));
    assert_eq!(run(&mut s, "document.check", json!({}))["ok"], true);
    // The range of w names d: deleting d says so.
    let e = refused(&mut s, "variable.delete", json!({"name": "d"}));
    assert!(e.contains("the parameter `w` (max)"), "{e}");
    // A rename follows into the range and keeps the description.
    run(
        &mut s,
        "variable.rename",
        json!({"name": "d", "to": "depth"}),
    );
    let list = run(&mut s, "param.list", json!({}));
    assert_eq!(param(&list, "w")["max"]["expr"], "2 * depth");
    assert_eq!(param(&list, "depth")["label"], "Depth");
    assert_eq!(param(&list, "w")["valid"], true);
}

#[test]
fn document_info_is_set_cleared_and_shown() {
    let mut s = session();
    jig(&mut s);
    let empty = run(&mut s, "document.info", json!({}));
    assert_eq!(empty["title"], "");
    assert!(
        run(&mut s, "document.inspect", json!({"measures": false}))
            .get("info")
            .is_none()
    );
    let info = run(
        &mut s,
        "document.info",
        json!({"title": "Plate", "author": "Test", "licence": "CC-BY-4.0", "tags": ["plate", "laser"]}),
    );
    assert_eq!(info["title"], "Plate");
    assert_eq!(info["tags"], json!(["plate", "laser"]));
    let inspect = run(&mut s, "document.inspect", json!({"measures": false}));
    assert_eq!(inspect["info"]["licence"], "CC-BY-4.0");
    assert_eq!(
        run(&mut s, "param.list", json!({}))["info"]["author"],
        "Test"
    );
    let info = run(&mut s, "document.info", json!({"tags": null, "author": ""}));
    assert_eq!(info["tags"], json!([]));
    assert_eq!(info["author"], "");
    assert_eq!(info["title"], "Plate");
    for params in [
        json!({"tags": "plate"}),
        json!({"tags": [" padded"]}),
        json!({"title": 3}),
        json!({"title": "x".repeat(2000)}),
        json!({"colour": "red"}),
    ] {
        refused(&mut s, "document.info", params);
    }
}

#[test]
fn a_jig_saves_and_opens_with_its_parameters_info_and_a_thumbnail() {
    let dir = workdir("jig-params", "save");
    let mut s = session_in(&dir);
    jig(&mut s);
    run(
        &mut s,
        "document.info",
        json!({"title": "Plate", "licence": "MIT", "tags": ["plate"]}),
    );
    let saved = run(&mut s, "file.save", json!({"path": "plate.3dcraft"}));
    assert_eq!(saved["thumbnail"], true, "{saved}");
    let before = run(&mut s, "param.list", json!({}));
    let bytes = std::fs::read(dir.join("plate.3dcraft")).unwrap();
    let png = craft3d_format::thumbnail(&bytes).unwrap().unwrap();
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    let decoder = png::Decoder::new(std::io::Cursor::new(&png));
    let reader = decoder.read_info().unwrap();
    assert_eq!((reader.info().width, reader.info().height), (400, 300));
    assert!(png.len() < 64 * 1024, "{} bytes", png.len());

    let mut t = session_in(&dir);
    run(&mut t, "file.open", json!({"path": "plate.3dcraft"}));
    assert_eq!(run(&mut t, "param.list", json!({})), before);
    // Saved again, the same bytes: the thumbnail is deterministic.
    run(&mut t, "file.save", json!({"path": "again.3dcraft"}));
    assert_eq!(std::fs::read(dir.join("again.3dcraft")).unwrap(), bytes);
    // Another value, another picture.
    run(&mut t, "param.set", json!({"values": {"d": "60 mm"}}));
    run(&mut t, "file.save", json!({"path": "deeper.3dcraft"}));
    let other = craft3d_format::thumbnail(&std::fs::read(dir.join("deeper.3dcraft")).unwrap())
        .unwrap()
        .unwrap();
    assert_ne!(other, png);

    // Opted out, and a plain model, have none.
    let r = run(
        &mut t,
        "file.save",
        json!({"path": "bare.3dcraft", "thumbnail": false}),
    );
    assert!(r.get("thumbnail").is_none(), "{r}");
    let bare = std::fs::read(dir.join("bare.3dcraft")).unwrap();
    assert_eq!(craft3d_format::thumbnail(&bare).unwrap(), None);
    assert_eq!(
        craft3d_format::from_bytes(&bare).unwrap(),
        craft3d_format::from_bytes(&std::fs::read(dir.join("deeper.3dcraft")).unwrap()).unwrap()
    );
    let mut plain = session_in(&dir);
    run(&mut plain, "file.new", json!({}));
    run(&mut plain, "feature.block", json!({"size": [1, 2, 3]}));
    let r = run(&mut plain, "file.save", json!({"path": "plain.3dcraft"}));
    assert!(r.get("thumbnail").is_none(), "{r}");
    // Asked for on a document without bodies: saved without, and said why.
    let mut nothing = session_in(&dir);
    run(&mut nothing, "file.new", json!({}));
    run(
        &mut nothing,
        "variable.set",
        json!({"name": "a", "expr": "1 mm"}),
    );
    run(&mut nothing, "variable.publish", json!({"name": "a"}));
    let r = run(
        &mut nothing,
        "file.save",
        json!({"path": "empty-jig.3dcraft"}),
    );
    assert_eq!(r["thumbnail"], false);
    assert!(
        r["thumbnailError"].as_str().unwrap().contains("no bodies"),
        "{r}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn publishing_never_makes_another_parameter_impossible_to_set() {
    let mut s = session();
    jig(&mut s);
    // w's default 40 mm is checked with every parameter at its default:
    // a default of 15 mm for d would put it above 2 * d.
    let e = refused(
        &mut s,
        "variable.publish",
        json!({"name": "d", "default": "15 mm"}),
    );
    assert!(e.contains("would make `w` invalid"), "{e}");
    // A range that stops working is the designer's to mend: the commands
    // that do it say so, and param.set still sets the value, with a
    // warning.
    run(
        &mut s,
        "variable.set",
        json!({"name": "cap", "expr": "100 mm"}),
    );
    run(
        &mut s,
        "variable.publish",
        json!({"name": "d", "max": "cap"}),
    );
    let r = run(
        &mut s,
        "variable.delete",
        json!({"name": "cap", "force": true}),
    );
    let warning = r["parameterWarnings"][0].as_str().unwrap();
    assert!(
        warning.contains("`d` is now invalid") && warning.contains("does not evaluate"),
        "{r}"
    );
    let r = run(&mut s, "param.set", json!({"values": {"d": "30 mm"}}));
    assert!(
        r["warnings"][0]
            .as_str()
            .unwrap()
            .contains("does not evaluate"),
        "{r}"
    );
    assert_close(&json!(volume(&mut s)), 40.0 * 30.0 * 5.0, "set anyway");
    // variable.set says what it put out of range.
    let r = run(
        &mut s,
        "variable.set",
        json!({"name": "w", "expr": "90 mm"}),
    );
    assert!(
        r["parameterWarnings"][0]
            .as_str()
            .unwrap()
            .contains("`w` is now invalid"),
        "{r}"
    );
}

#[test]
fn whole_kinds_take_labels_words_and_store_exact_whole_numbers() {
    let mut s = session();
    jig(&mut s);
    run(
        &mut s,
        "param.set",
        json!({"values": {"n": "4.0000000001", "lid": "false", "style": "Round"}}),
    );
    let list = run(&mut s, "param.list", json!({}));
    assert_eq!(param(&list, "n")["expr"], "4");
    assert_eq!(param(&list, "lid")["expr"], "0");
    assert_eq!(param(&list, "style")["expr"], "3");
    assert_eq!(param(&list, "style")["choice"], "Round");
    run(
        &mut s,
        "param.set",
        json!({"values": {"lid": "TRUE", "n": 5.0}}),
    );
    let list = run(&mut s, "param.list", json!({}));
    assert_eq!(param(&list, "lid")["expr"], "1");
    assert_eq!(param(&list, "n")["expr"], "5");
    let e = refused(&mut s, "param.set", json!({"values": {"style": "Square"}}));
    assert!(
        e.contains("one of its choices (Plain, Slotted, Round)"),
        "{e}"
    );
    let e = refused(&mut s, "param.set", json!({"values": {"lid": "yes"}}));
    assert!(e.contains("give true or false"), "{e}");
    let e = refused(&mut s, "param.set", json!({"values": {"style": [1]}}));
    assert!(e.contains("one of its choices"), "{e}");
}

#[test]
fn a_parameter_of_another_dimension_is_read_and_set_with_its_units() {
    let mut s = session();
    jig(&mut s);
    run(
        &mut s,
        "variable.set",
        json!({"name": "hole", "expr": "400 * (1 mm)^2"}),
    );
    run(
        &mut s,
        "variable.publish",
        json!({"name": "hole", "max": "1000 * (1 mm)^2"}),
    );
    let list = run(&mut s, "param.list", json!({}));
    let hole = param(&list, "hole");
    assert_eq!(hole["unit"], Value::Null);
    assert_eq!(hole["unitText"], "mm^2");
    assert_close(&hole["value"], 400.0, "area");
    let e = refused(&mut s, "param.set", json!({"values": {"hole": 500}}));
    assert!(e.contains("a plain number cannot give"), "{e}");
    run(
        &mut s,
        "param.set",
        json!({"values": {"hole": "500 * (1 mm)^2"}}),
    );
    let e = refused(
        &mut s,
        "param.set",
        json!({"values": {"hole": "5000 * (1 mm)^2"}}),
    );
    assert!(e.contains("outside its range, at most 1000 mm^2"), "{e}");
}

#[test]
fn a_new_drag_never_folds_into_an_earlier_one() {
    let mut s = session();
    jig(&mut s);
    let steps = |s: &mut Session| {
        run(s, "document.inspect", json!({"measures": false}))["history"]["undo"]
            .as_array()
            .unwrap()
            .len()
    };
    // The first drag ends at 30 mm.
    run(&mut s, "param.set", json!({"values": {"w": 35}}));
    run(
        &mut s,
        "param.set",
        json!({"values": {"w": 30}, "merge": true}),
    );
    let after_first = steps(&mut s);
    // The second goes to 25, back to 30 (its step goes), then on to 20.
    run(&mut s, "param.set", json!({"values": {"w": 25}}));
    run(
        &mut s,
        "param.set",
        json!({"values": {"w": 30}, "merge": true}),
    );
    assert_eq!(steps(&mut s), after_first);
    run(
        &mut s,
        "param.set",
        json!({"values": {"w": 20}, "merge": true}),
    );
    assert_eq!(steps(&mut s), after_first + 1);
    run(&mut s, "edit.undo", json!({}));
    assert_close(
        &json!(volume(&mut s)),
        30.0 * 20.0 * 5.0,
        "the first drag kept",
    );
}

#[test]
fn info_url_must_be_a_web_address() {
    let mut s = session();
    jig(&mut s);
    let e = refused(
        &mut s,
        "document.info",
        json!({"url": "javascript:alert(1)"}),
    );
    assert!(e.contains("http://"), "{e}");
    refused(&mut s, "document.info", json!({"title": "a\u{202E}b"}));
    run(
        &mut s,
        "document.info",
        json!({"url": "https://example.org"}),
    );
}

#[test]
fn the_thumbnail_shows_the_document_not_where_the_view_is_scrubbed() {
    let dir = workdir("jig-params", "scrub");
    let mut s = session_in(&dir);
    jig(&mut s);
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Peg", "radius": "3 mm", "height": "20 mm", "position": ["10 mm", "10 mm", "0 mm"]}),
    );
    run(&mut s, "file.save", json!({"path": "plain.3dcraft"}));
    let plain = std::fs::read(dir.join("plain.3dcraft")).unwrap();
    // A fresh session (no cached thumbnail) with the view scrubbed back
    // to before the peg saves the same bytes.
    let mut t = session_in(&dir);
    run(&mut t, "file.open", json!({"path": "plain.3dcraft"}));
    run(&mut t, "view.scrub", json!({"step": 1}));
    let scrubbed = run(&mut t, "view.render", json!({"width": 64, "height": 48}));
    let full = {
        let mut u = session_in(&dir);
        run(&mut u, "file.open", json!({"path": "plain.3dcraft"}));
        run(&mut u, "view.render", json!({"width": 64, "height": 48}))
    };
    // The scrub does show in the view: the pictures differ.
    assert_ne!(scrubbed["base64"], full["base64"]);
    run(&mut t, "file.save", json!({"path": "scrubbed.3dcraft"}));
    assert_eq!(std::fs::read(dir.join("scrubbed.3dcraft")).unwrap(), plain);
    // And the view stays scrubbed.
    let still = run(&mut t, "view.render", json!({"width": 64, "height": 48}));
    assert_eq!(still["base64"], scrubbed["base64"]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// plan/repro/repro-tuned-chimes-1.json: a range bound looks a table up
/// as a variable does (it said "unknown table `scales`").
#[test]
fn a_range_bound_looks_up_a_table() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "table.set",
        json!({"name": "scales", "columns": ["scale", "maxNotes"], "rows": [["A", 8], ["B", 10]]}),
    );
    for (name, expr) in [
        ("scale", "1"),
        ("N", "6"),
        ("probe", "scales[scale].maxNotes"),
    ] {
        run(&mut s, "variable.set", json!({"name": name, "expr": expr}));
    }
    run(
        &mut s,
        "variable.publish",
        json!({"name": "scale", "kind": "choice", "choicesFrom": "scales"}),
    );
    run(
        &mut s,
        "variable.publish",
        json!({"name": "N", "kind": "integer", "min": "3", "max": "scales[scale].maxNotes"}),
    );
    let list = run(&mut s, "param.list", json!({}));
    let n = param(&list, "N");
    assert_eq!(n["valid"], true, "{n}");
    assert_close(&n["max"]["value"], 8.0, "max of N for scale A");
    // The bound follows the choice: 9 notes only on scale B.
    let e = refused(&mut s, "param.set", json!({"values": {"N": 9}}));
    assert!(e.contains("outside its range, from 3 to 8"), "{e}");
    run(
        &mut s,
        "param.set",
        json!({"values": {"scale": "B", "N": 9}}),
    );
    let list = run(&mut s, "param.list", json!({}));
    assert_close(&param(&list, "N")["max"]["value"], 10.0, "max of N for B");
    // Back to A puts N outside: refused, or clamped to the new maximum.
    let e = refused(&mut s, "param.set", json!({"values": {"scale": "A"}}));
    assert!(e.contains("would make `N` invalid"), "{e}");
    let r = run(
        &mut s,
        "param.set",
        json!({"values": {"scale": "A"}, "clamp": true}),
    );
    assert_eq!(r["clamped"], json!([{"name": "N", "expr": "8"}]));
    // A bound that names a table that does not exist says so.
    let e = refused(
        &mut s,
        "variable.publish",
        json!({"name": "N", "max": "nope[scale].maxNotes"}),
    );
    assert!(e.contains("unknown table `nope`"), "{e}");
}

/// A value off its step is refused with the nearest values on it,
/// counted from the minimum (from 0 without one); an end of the range is
/// always allowed (plan/r1-cleanup.md item 60).
#[test]
fn a_value_off_its_step_is_refused_with_the_nearest_on_it() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    for (name, expr) in [("n", "12"), ("t", "2 mm")] {
        run(&mut s, "variable.set", json!({"name": name, "expr": expr}));
    }
    run(
        &mut s,
        "variable.publish",
        json!({"name": "n", "kind": "integer", "max": "21", "step": "2"}),
    );
    run(
        &mut s,
        "variable.publish",
        json!({"name": "t", "min": "1 mm", "max": "2.7 mm", "step": "0.5 mm"}),
    );
    let e = refused(&mut s, "param.set", json!({"values": {"n": 13}}));
    assert!(
        e.contains("13 is not on its step of 2 counted from 0: the nearest values are 12 and 14"),
        "{e}"
    );
    let e = refused(&mut s, "param.set", json!({"values": {"t": "2.3 mm"}}));
    assert!(
        e.contains("2.3 mm is not on its step of 0.5 mm counted from 1 mm: the nearest values are 2 mm and 2.5 mm"),
        "{e}"
    );
    // On the step, and the ends of the range, are taken.
    for (name, value) in [("n", json!(14)), ("n", json!(21)), ("t", json!("2.5 mm"))] {
        run(&mut s, "param.set", json!({"values": {name: value}}));
    }
    run(&mut s, "param.set", json!({"values": {"t": "2.7 mm"}}));
    run(&mut s, "param.set", json!({"values": {"t": "1 mm"}}));
    // Publishing with a value off the step is refused, and so is a
    // default off it.
    run(&mut s, "variable.set", json!({"name": "q", "expr": "7"}));
    let e = refused(
        &mut s,
        "variable.publish",
        json!({"name": "q", "kind": "integer", "step": "2"}),
    );
    assert!(e.contains("not on its step"), "{e}");
    let e = refused(
        &mut s,
        "variable.publish",
        json!({"name": "q", "kind": "integer", "min": "1", "step": "2", "default": "4"}),
    );
    assert!(e.contains("its default 4 is not on its step"), "{e}");
    // A step that moves with the minimum: clamp puts the other parameter
    // back on its step.
    run(&mut s, "variable.set", json!({"name": "lo", "expr": "1"}));
    run(&mut s, "variable.set", json!({"name": "k", "expr": "5"}));
    run(
        &mut s,
        "variable.publish",
        json!({"name": "lo", "kind": "integer", "min": "0", "max": "4"}),
    );
    run(
        &mut s,
        "variable.publish",
        json!({"name": "k", "kind": "integer", "min": "lo", "max": "20", "step": "2"}),
    );
    let e = refused(&mut s, "param.set", json!({"values": {"lo": 2}}));
    assert!(e.contains("would make `k` invalid"), "{e}");
    let r = run(
        &mut s,
        "param.set",
        json!({"values": {"lo": 2}, "clamp": true}),
    );
    let k = r["clamped"][0]["expr"].as_str().unwrap();
    assert!(k == "4" || k == "6", "{r}");
}
