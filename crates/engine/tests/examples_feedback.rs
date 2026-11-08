//! What building the example models showed 3DCraft answering wrongly
//! (3DCRAFT-FEEDBACK.md of the examples repository, F-005 to F-177): each
//! test holds one answer to what was meant, by a value worked out by hand
//! or by a second route through the commands.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::run;
use common::{refused, session, session_in, workdir};
use craft3d_engine::Session;
use serde_json::{Value, json};

fn part() -> Session {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    s
}

fn number(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("{v} is not a number"))
}

fn close(a: f64, b: f64, what: &str) {
    assert!(
        (a - b).abs() <= 1e-9 * b.abs().max(1.0),
        "{what}: {a} and {b}"
    );
}

/// The entry of assertion `name` in `assert.list`.
fn assertion(s: &mut Session, name: &str) -> Value {
    let list = run(s, "assert.list", json!({"details": true}));
    list["assertions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["name"] == name)
        .cloned()
        .unwrap_or_else(|| panic!("no assertion {name}: {list}"))
}

// ------------------------------------------------------- F-021, units

#[test]
fn a_number_with_a_unit_to_a_power_is_refused_where_it_was_taken_silently() {
    let mut s = part();
    for (id, params) in [
        ("variable.set", json!({"name": "a", "expr": "332 mm^2"})),
        (
            "variable.set",
            json!({"name": "a", "expr": "26740000 mm^4"}),
        ),
        ("variable.output", json!({"name": "o", "expr": "332 mm^2"})),
    ] {
        let why = refused(&mut s, id, params);
        assert!(why.contains("* (1 mm)^"), "{why}");
    }
    run(
        &mut s,
        "variable.set",
        json!({"name": "side", "expr": "100 * (1 mm)^2"}),
    );
    run(&mut s, "variable.publish", json!({"name": "side"}));
    let why = refused(&mut s, "param.set", json!({"values": {"side": "332 mm^2"}}));
    assert!(why.contains("* (1 mm)^"), "{why}");
    let why = refused(
        &mut s,
        "variable.publish",
        json!({"name": "side", "default": "332 mm^2"}),
    );
    assert!(why.contains("`default`"), "{why}");
    // What it says to write is the area meant: 332 mm², not 110224.
    let set = run(
        &mut s,
        "variable.set",
        json!({"name": "a", "expr": "332 * (1 mm)^2"}),
    );
    close(number(&set["value"]), 332.0, "the area");
    // A quantity in brackets, a variable and zero mean what they say.
    for expr in ["(2 mm)^2", "a * 1", "0 mm^2", "1 mm^2"] {
        run(&mut s, "variable.set", json!({"name": "b", "expr": expr}));
    }
}

// ------------------------------------------------------- F-177, names

#[test]
fn an_output_may_have_the_name_of_the_column_it_reads() {
    let mut s = part();
    run(
        &mut s,
        "table.set",
        json!({"name": "ipe", "columns": ["profile", {"name": "h", "unit": "mm"}],
               "rows": [["IPE 180", 180], ["IPE 200", 200]]}),
    );
    run(&mut s, "variable.set", json!({"name": "pick", "expr": "2"}));
    let h = run(
        &mut s,
        "variable.output",
        json!({"name": "h", "expr": "ipe[pick].h"}),
    );
    assert_eq!(h["status"], "known", "{h}");
    close(number(&h["value"]), 200.0, "row 2's h");
    // A real cycle is still one.
    run(&mut s, "variable.output", json!({"name": "p", "expr": "1"}));
    run(
        &mut s,
        "variable.output",
        json!({"name": "q", "expr": "p + 1"}),
    );
    let why = refused(
        &mut s,
        "variable.output",
        json!({"name": "p", "expr": "q + 1"}),
    );
    assert!(why.contains("would depend on itself"), "{why}");
}

// ------------------------------------------------------- F-011, F-131

#[test]
fn the_refusals_name_what_does_work() {
    let mut s = part();
    let set = run(
        &mut s,
        "variable.set",
        json!({"name": "a", "expr": "2 ^ (1 / 2)"}),
    );
    let why = set["error"].as_str().unwrap_or_else(|| panic!("{set}"));
    assert!(why.contains("pow(x, y)"), "{why}");
    run(&mut s, "sketch.create", json!({"name": "S"}));
    run(&mut s, "variable.set", json!({"name": "k", "expr": "3 mm"}));
    let why = refused(
        &mut s,
        "sketch.spline",
        json!({"sketch": "S", "through": [[0, 0], ["k", 5], [8, 6], [10, 0]]}),
    );
    assert!(why.contains("live: true"), "{why}");
    // What it points to does take the expression.
    run(
        &mut s,
        "sketch.spline",
        json!({"sketch": "S", "through": [[0, 0], ["k", "5 mm"], [8, 6], [10, 0]], "live": true}),
    );
}

// ------------------------------------------------------- F-168, render

#[test]
fn a_picture_says_the_camera_that_drew_it() {
    let mut s = part();
    run(
        &mut s,
        "feature.block",
        json!({"size": ["10 mm", "10 mm", "10 mm"]}),
    );
    let given = run(
        &mut s,
        "view.render",
        json!({"width": 32, "height": 24, "renderer": "cpu",
               "camera": {"eye": [50, 0, 0], "target": [0, 0, 0], "up": [0, 0, 1],
                          "projection": "orthographic", "height": 40}}),
    );
    assert_eq!(given["projection"], "orthographic", "{}", given["view"]);
    assert_eq!(given["view"], "camera");
    let perspective = run(
        &mut s,
        "view.render",
        json!({"width": 32, "height": 24, "renderer": "cpu",
               "camera": {"eye": [50, 0, 0], "target": [0, 0, 0], "up": [0, 0, 1], "fovY": 30}}),
    );
    assert_eq!(perspective["projection"], "perspective");
    assert_eq!(perspective["view"], "camera");
    let framed = run(
        &mut s,
        "view.render",
        json!({"width": 32, "height": 24, "renderer": "cpu", "view": "top",
               "projection": "orthographic"}),
    );
    assert_eq!(framed["view"], "top");
    assert_eq!(framed["projection"], "orthographic");
}

// ------------------------------------------------------- F-018, inspect

#[test]
fn inspect_can_wait_for_the_assertions_and_refuses_a_bad_wait() {
    let mut s = part();
    run(
        &mut s,
        "feature.block",
        json!({"name": "B", "size": ["10 mm", "10 mm", "10 mm"]}),
    );
    run(
        &mut s,
        "assert.add",
        json!({"name": "v", "measure": "volume", "of": "B", "eq": "1000 * (1 mm)^3"}),
    );
    let inspect = run(&mut s, "document.inspect", json!({"wait": true}));
    assert_eq!(inspect["assertions"][0]["status"], "passed", "{inspect}");
    assert!(inspect.get("whenPending").is_none(), "{inspect}");
    let why = refused(&mut s, "document.inspect", json!({"wait": "soon"}));
    assert!(why.contains("wait"), "{why}");
}

// ---------------------------------------------------- F-133, one name

/// A block's side (+x, face 5) drafted 10 degrees about the plane of its
/// bottom: the normal leans sin 10 degrees towards z.
fn drafted_block(s: &mut Session) {
    run(
        s,
        "feature.block",
        json!({"size": ["40 mm", "40 mm", "30 mm"], "position": ["-20 mm", "-20 mm", "0 mm"],
               "name": "Block"}),
    );
    run(
        s,
        "feature.draft",
        json!({"faces": ["F1:face(2)", "F1:face(3)", "F1:face(4)", "F1:face(5)"],
               "neutral": "F1:face(0)", "angle": "10 deg", "name": "Drafted"}),
    );
}

#[test]
fn a_datum_or_a_sketch_on_an_old_name_says_when_the_face_has_changed() {
    use craft3d_math::Portable;
    let mut s = part();
    drafted_block(&mut s);
    let tilt = 10.0_f64.to_radians();
    // The name keeps meaning the face as it was (docs/decisions.md,
    // references that follow a body), and the result says it has changed.
    let old = run(
        &mut s,
        "datum.plane",
        json!({"from": "F1:face(5)", "distance": "0 mm", "name": "Old"}),
    );
    close(
        number(&old["datum"]["normal"][0]),
        1.0,
        "the face as it was",
    );
    let warning = old["warnings"][0]
        .as_str()
        .unwrap_or_else(|| panic!("{old}"));
    assert!(warning.contains("`F2:copy(0)/F1:face(5)`"), "{warning}");
    // What it points to is the face as it is, and that needs no warning.
    let new = run(
        &mut s,
        "datum.plane",
        json!({"from": "F2:copy(0)/F1:face(5)", "distance": "0 mm", "name": "New"}),
    );
    assert!(new.get("warnings").is_none(), "{new}");
    close(
        number(&new["datum"]["normal"][0]),
        tilt.pcos(),
        "x of the normal",
    );
    close(
        number(&new["datum"]["normal"][2]),
        tilt.psin(),
        "z of the normal",
    );
    // A sketch says the same.
    let sketch = run(
        &mut s,
        "sketch.create",
        json!({"name": "On it", "face": "F1:face(5)"}),
    );
    assert!(
        sketch["warnings"][0]
            .as_str()
            .unwrap()
            .contains("F2:copy(0)"),
        "{sketch}"
    );
    // The top is taken by the draft too, but stays in its plane.
    let top = run(
        &mut s,
        "datum.plane",
        json!({"from": "F1:face(1)", "distance": "0 mm", "name": "Top"}),
    );
    assert!(top.get("warnings").is_none(), "{top}");
    // Redefined where it stands, before the draft: the face has not
    // changed there, and the draft's name would not exist yet.
    let mut early = part();
    run(
        &mut early,
        "feature.block",
        json!({"size": ["40 mm", "40 mm", "30 mm"], "position": ["-20 mm", "-20 mm", "0 mm"],
               "name": "Block"}),
    );
    run(
        &mut early,
        "datum.plane",
        json!({"from": "F1:face(5)", "distance": "0 mm", "name": "Early"}),
    );
    run(
        &mut early,
        "feature.draft",
        json!({"faces": ["F1:face(2)", "F1:face(3)", "F1:face(4)", "F1:face(5)"],
               "neutral": "F1:face(0)", "angle": "10 deg", "name": "Drafted"}),
    );
    let replaced = run(
        &mut early,
        "datum.plane",
        json!({"replace": "Early", "from": "F1:face(5)", "distance": "1 mm"}),
    );
    assert!(replaced.get("warnings").is_none(), "{replaced}");
    // After a shell the name is two faces: no one newest face to name, and
    // it is placed as it always was.
    let mut s = part();
    run(
        &mut s,
        "feature.block",
        json!({"size": ["40 mm", "40 mm", "30 mm"], "name": "Box"}),
    );
    run(
        &mut s,
        "feature.shell",
        json!({"source": "Box", "thickness": "2 mm", "open": ["F1:face(1)"], "name": "Hollow"}),
    );
    let placed = run(
        &mut s,
        "sketch.create",
        json!({"name": "Side", "face": "F1:face(5)"}),
    );
    assert_eq!(placed["status"], "ok", "{placed}");
    assert!(placed.get("warnings").is_none(), "{placed}");
}

// ------------------------------------------------------ F-067, proofs

fn flared_loft(s: &mut Session) {
    let circle = |r: f64| -> Vec<[f64; 2]> {
        let d = r * std::f64::consts::FRAC_1_SQRT_2;
        vec![
            [r, 0.0],
            [d, d],
            [0.0, r],
            [-d, d],
            [-r, 0.0],
            [-d, -d],
            [0.0, -r],
            [d, -d],
        ]
    };
    for (name, offset, r) in [("Low", "0 mm", 10.0), ("High", "30 mm", 30.0)] {
        run(
            s,
            "sketch.create",
            json!({"name": name, "plane": "xy", "offset": offset}),
        );
        run(
            s,
            "sketch.spline",
            json!({"sketch": name, "through": circle(r), "closed": true}),
        );
    }
    run(
        s,
        "feature.loft",
        json!({"sections": ["Low", "High"], "name": "Flared"}),
    );
}

#[test]
fn a_bound_met_only_at_samples_is_not_proven_and_one_missed_there_fails() {
    let mut s = part();
    flared_loft(&mut s);
    let sides = json!({"kind": "face", "where": {"name": "F3:side*"}});
    run(
        &mut s,
        "assert.add",
        json!({"name": "sampled", "measure": "draft", "faces": sides, "direction": "z",
               "min": "-80 deg"}),
    );
    let checked = assertion(&mut s, "sampled");
    assert_eq!(checked["status"], "notProven", "{checked}");
    let message = checked["message"].as_str().unwrap();
    assert!(message.contains("sampled points only"), "{message}");
    // In the degrees the bound is written in.
    assert_eq!(checked["expected"], "≥ -80 deg", "{checked}");
    // Found below the bound at a real place: that is a failure, proven.
    run(
        &mut s,
        "assert.add",
        json!({"name": "missed", "measure": "draft", "faces": sides, "direction": "z",
               "min": "80 deg"}),
    );
    assert_eq!(assertion(&mut s, "missed")["status"], "failed");
    // The least of samples is at most what was found: a bound above it is
    // proven, and one below what was found is not refuted.
    run(
        &mut s,
        "assert.add",
        json!({"name": "above", "measure": "draft", "faces": sides, "direction": "z",
               "max": "80 deg"}),
    );
    assert_eq!(assertion(&mut s, "above")["status"], "passed");
    run(
        &mut s,
        "assert.add",
        json!({"name": "below", "measure": "draft", "faces": sides, "direction": "z",
               "max": "-80 deg"}),
    );
    assert_eq!(assertion(&mut s, "below")["status"], "notProven");
    // A tolerance on an angle reads in degrees too.
    run(
        &mut s,
        "assert.add",
        json!({"name": "about", "measure": "draft", "faces": sides, "direction": "z",
               "eq": "10 deg", "tol": "1 deg"}),
    );
    assert_eq!(assertion(&mut s, "about")["expected"], "10 deg ± 1 deg");
    // An output of the same measure still has its value.
    let output = run(
        &mut s,
        "variable.measure",
        json!({"name": "least", "measure": "draft", "faces": sides, "direction": "z"}),
    );
    assert_eq!(output["status"], "known", "{output}");
}

#[test]
fn curvature_is_proven_on_a_sphere_and_not_on_a_loft() {
    let mut s = part();
    run(
        &mut s,
        "feature.sphere",
        json!({"radius": "10 mm", "name": "Ball"}),
    );
    // K = 1 / 100 everywhere.
    run(
        &mut s,
        "assert.add",
        json!({"name": "ball", "measure": "curvature", "of": "Ball", "max": "0.02 / (1 mm)^2"}),
    );
    assert_eq!(assertion(&mut s, "ball")["status"], "passed");
    flared_loft(&mut s);
    run(
        &mut s,
        "assert.add",
        json!({"name": "loft", "measure": "curvature", "of": "Flared", "max": "100 / (1 mm)^2"}),
    );
    assert_eq!(assertion(&mut s, "loft")["status"], "notProven");
    // The largest of samples is at least what was found: a lower bound on
    // it is proven.
    run(
        &mut s,
        "assert.add",
        json!({"name": "some", "measure": "curvature", "of": "Flared", "min": "0 / (1 mm)^2"}),
    );
    assert_eq!(assertion(&mut s, "some")["status"], "passed");
}

// ------------------------------------------------- F-112, a free centre

fn path_warnings(start: Value, end: Value, center: Value) -> Vec<String> {
    let mut s = part();
    run(&mut s, "sketch.create", json!({"name": "S"}));
    let made = run(
        &mut s,
        "sketch.path",
        json!({"sketch": "S", "start": start,
               "segments": [{"arc": end, "center": center}], "closed": true}),
    );
    made.get("warnings").map_or_else(Vec::new, |w| {
        w.as_array()
            .unwrap()
            .iter()
            .map(|t| t.as_str().unwrap().to_string())
            .collect()
    })
}

#[test]
fn a_plain_centre_among_held_corners_is_warned_about() {
    let warned = path_warnings(
        json!(["10 mm", "0 mm"]),
        json!(["0 mm", "10 mm"]),
        json!([0, 0]),
    );
    assert_eq!(warned.len(), 1, "{warned:?}");
    assert!(warned[0].contains("plain numbers"), "{warned:?}");
    assert!(
        path_warnings(
            json!(["10 mm", "0 mm"]),
            json!(["0 mm", "10 mm"]),
            json!(["0 mm", "0 mm"])
        )
        .is_empty()
    );
    assert!(path_warnings(json!([10, 0]), json!([0, 10]), json!([0, 0])).is_empty());
}

// ---------------------------------------- F-005, F-146, edits keep the rest

#[test]
fn a_body_appearance_keeps_what_a_call_leaves_out() {
    let mut s = part();
    run(
        &mut s,
        "feature.block",
        json!({"name": "B", "size": ["10 mm", "10 mm", "10 mm"]}),
    );
    // No colour yet, and none given: there is nothing to keep.
    let why = refused(
        &mut s,
        "body.appearance",
        json!({"body": "B", "metallic": 1}),
    );
    assert!(why.contains("color"), "{why}");
    run(
        &mut s,
        "body.appearance",
        json!({"body": "B", "color": "#102030", "metallic": 1, "roughness": 0.3}),
    );
    let kept = run(
        &mut s,
        "body.appearance",
        json!({"body": "B", "color": "#405060"}),
    );
    assert_eq!(kept["appearance"]["color"], "#405060", "{kept}");
    assert_eq!(kept["appearance"]["metallic"], 1.0, "{kept}");
    assert_eq!(kept["appearance"]["roughness"], 0.3, "{kept}");
    let without_colour = run(
        &mut s,
        "body.appearance",
        json!({"body": "B", "roughness": 0.7}),
    );
    assert_eq!(without_colour["appearance"]["color"], "#405060");
    assert_eq!(without_colour["appearance"]["roughness"], 0.7);
    // Null takes one field away, clear all of them.
    let taken = run(
        &mut s,
        "body.appearance",
        json!({"body": "B", "metallic": null}),
    );
    assert!(taken["appearance"].get("metallic").is_none(), "{taken}");
    assert_eq!(taken["appearance"]["roughness"], 0.7);
    let cleared = run(
        &mut s,
        "body.appearance",
        json!({"body": "B", "clear": true}),
    );
    assert_eq!(cleared["appearance"], Value::Null, "{cleared}");
}

#[test]
fn an_assertion_edit_changes_only_what_it_names() {
    let mut s = part();
    run(
        &mut s,
        "feature.block",
        json!({"name": "B", "size": ["20 mm", "10 mm", "5 mm"]}),
    );
    run(
        &mut s,
        "assert.add",
        json!({"name": "x", "measure": "extent", "of": "B", "axis": "x",
               "min": "10 mm", "max": "30 mm"}),
    );
    // One bound changed: the measure and the other bound stay.
    run(&mut s, "assert.edit", json!({"name": "x", "max": "25 mm"}));
    let edited = assertion(&mut s, "x");
    assert_eq!(edited["status"], "passed", "{edited}");
    assert_eq!(edited["expected"], "10 mm to 25 mm", "{edited}");
    assert_eq!(edited["measure"]["axis"], "x", "{edited}");
    assert_eq!(edited["measure"]["of"], "F1", "{edited}");
    // One parameter of the measure changed: the rest stays, and the bounds.
    run(&mut s, "assert.edit", json!({"name": "x", "axis": "y"}));
    let turned = assertion(&mut s, "x");
    assert_eq!(turned["measure"]["axis"], "y", "{turned}");
    assert_eq!(turned["measure"]["of"], "F1", "{turned}");
    assert_eq!(turned["expected"], "10 mm to 25 mm", "{turned}");
    // Null takes a bound away; eq in place of bounds replaces them.
    run(&mut s, "assert.edit", json!({"name": "x", "max": null}));
    assert_eq!(assertion(&mut s, "x")["expected"], "≥ 10 mm");
    run(
        &mut s,
        "assert.edit",
        json!({"name": "x", "eq": "10 mm", "tol": "0.1 mm"}),
    );
    assert_eq!(assertion(&mut s, "x")["status"], "passed");
    run(&mut s, "assert.edit", json!({"name": "x", "eq": "11 mm"}));
    let wider = assertion(&mut s, "x");
    assert_eq!(wider["expected"], "11 mm ± 0.1", "{wider}");
    assert_eq!(wider["status"], "failed", "{wider}");
    // Another measure starts afresh and says what it must be.
    let why = refused(
        &mut s,
        "assert.edit",
        json!({"name": "x", "measure": "volume", "of": "B"}),
    );
    assert!(why.contains("must be"), "{why}");
}

// --------------------------------------------------- F-151, one shape

#[test]
fn an_edit_returns_the_summary_under_its_own_key() {
    let mut s = part();
    run(
        &mut s,
        "variable.set",
        json!({"name": "side", "expr": "10 mm"}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "B", "size": ["side", "side", "side"]}),
    );
    run(
        &mut s,
        "assert.add",
        json!({"name": "wide", "measure": "extent", "of": "B", "axis": "x", "eq": "side"}),
    );
    let set = run(
        &mut s,
        "variable.set",
        json!({"name": "side", "expr": "12 mm", "wait": true}),
    );
    assert_eq!(set["assertionSummary"]["passed"], 1, "{set}");
    assert!(set.get("assertions").is_none(), "{set}");
    let check = run(&mut s, "document.check", json!({}));
    assert_eq!(check["assertionSummary"]["passed"], 1, "{check}");
    assert!(check.get("assertions").is_none(), "{check}");
    // The list keeps the key: there it is a list.
    let list = run(&mut s, "assert.list", json!({}));
    assert!(list["assertions"].is_array(), "{list}");
}

// ---------------------------------------------------- F-060, panel caps

/// The name of the face a ray from above meets first on a flat panel of
/// a fold, thick 3 mm on `side`.
fn top_of_panel(side: &str) -> String {
    let mut s = part();
    run(
        &mut s,
        "sketch.create",
        json!({"name": "Board", "plane": "xy"}),
    );
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Board", "closed": true,
               "points": [[0, 0], [100, 0], [100, 40], [0, 40]]}),
    );
    let line = run(
        &mut s,
        "sketch.line",
        json!({"sketch": "Board", "construction": true, "from": [50, 0], "to": [50, 40]}),
    );
    let crease = line["entity"]
        .as_str()
        .or(line["id"].as_str())
        .unwrap()
        .to_string();
    run(
        &mut s,
        "feature.fold",
        json!({"sketch": "Board", "name": "Panels", "fixed": [25, 20],
               "creases": [{"line": crease, "kind": "valley", "angle": "10 deg"}],
               "panels": {"thickness": "3 mm", "clearance": "0.5 mm", "side": side}}),
    );
    let hits = run(
        &mut s,
        "measure.ray",
        json!({"from": [25, 20, 10], "direction": "-z", "of": "Panels"}),
    );
    hits["hits"][0]["face"]
        .as_str()
        .unwrap_or_else(|| panic!("{hits}"))
        .to_string()
}

#[test]
fn a_panels_start_is_the_face_on_the_sheet_as_an_extrusions_is_on_its_sketch() {
    // On the back the panel hangs under the sheet: from above the first
    // face met is the one on the sheet.
    let back = top_of_panel("back");
    assert!(back.contains("startCap"), "{back}");
    // On the front it stands on the sheet: the first face met is the far one.
    let front = top_of_panel("front");
    assert!(front.contains("endCap"), "{front}");
}

// ------------------------------------------------- F-081, print exports

#[test]
fn a_print_format_is_held_to_a_tenth_of_a_millimetre() {
    let dir = workdir("examples-feedback", "print");
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    // A side of the square of its area of about 400 mm: 0.4 mm automatic.
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": "150 mm", "height": "20 mm", "name": "Disc"}),
    );
    let at = |s: &mut Session, params: Value| number(&run(s, "file.export", params)["tolerance"]);
    let automatic = at(&mut s, json!({"path": "disc.obj"}));
    assert!(automatic > 0.2, "{automatic}");
    for path in ["disc.stl", "disc.3mf"] {
        close(at(&mut s, json!({"path": path})), 0.1, path);
        close(
            at(&mut s, json!({"path": path, "quality": "fine"})),
            0.025,
            path,
        );
        // Coarse asks for less, and a tolerance given is what was asked.
        close(
            at(&mut s, json!({"path": path, "quality": "coarse"})),
            2.0 * automatic,
            path,
        );
        close(
            at(&mut s, json!({"path": path, "tolerance": 0.5})),
            0.5,
            path,
        );
    }
    // A part whose automatic tolerance is finer keeps it.
    let dir = workdir("examples-feedback", "print-small");
    let mut small = session_in(&dir);
    run(&mut small, "file.new", json!({"units": "mm"}));
    run(
        &mut small,
        "feature.cylinder",
        json!({"radius": "5 mm", "height": "5 mm", "name": "Pin"}),
    );
    let tiny = at(&mut small, json!({"path": "pin.stl"}));
    close(tiny, at(&mut small, json!({"path": "pin.obj"})), "the pin");
    assert!(tiny < 0.1, "{tiny}");
    // Many metres across it is no print: at most 16 times finer than its
    // own automatic tolerance.
    let dir = workdir("examples-feedback", "print-huge");
    let mut huge = session_in(&dir);
    run(&mut huge, "file.new", json!({"units": "mm"}));
    run(
        &mut huge,
        "feature.cylinder",
        json!({"radius": "5000 mm", "height": "20 mm", "name": "Tank floor"}),
    );
    let plain = at(&mut huge, json!({"path": "huge.obj"}));
    assert!(plain / 16.0 > 0.1, "{plain}");
    close(
        at(&mut huge, json!({"path": "huge.stl"})),
        plain / 16.0,
        "huge",
    );
    // In another unit: a tenth of a millimetre in inches.
    let dir = workdir("examples-feedback", "print-inch");
    let mut inches = session_in(&dir);
    run(&mut inches, "file.new", json!({"units": "in"}));
    run(
        &mut inches,
        "feature.cylinder",
        json!({"radius": "6 in", "height": "1 in", "name": "Disc"}),
    );
    close(
        at(&mut inches, json!({"path": "disc.stl"})),
        0.1 / 25.4,
        "inches",
    );
}
