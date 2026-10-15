//! References that follow a body (M9): a chamfer, a fillet, an offset
//! face, a delete face and a shell given faces or edges by the names of an
//! earlier body that went into later features resolve them in the newest
//! body before them, as if the names had been given in full. The oracle
//! of every test is the same document built with the full names, the
//! way it had to be built before (docs/design/names.md › *References
//! that follow a body*).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Access, Session, Workspace};
use serde_json::{Value, json};
use std::path::PathBuf;

fn workdir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("follow-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn session() -> Session {
    Session::new(Workspace {
        read: Access::None,
        write: Access::None,
    })
}

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    let result = session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"));
    // Built up edit by edit, the document is what regenerating it from
    // nothing gives: the cache knows what a followed name depends on.
    assert_ne!(
        session.regeneration_matches_a_full_one(),
        Some(false),
        "after {id} {params}"
    );
    result
}

fn built(session: &mut Session, id: &str, params: Value) -> Value {
    let result = run(session, id, params.clone());
    assert_eq!(result["status"], "ok", "{id} {params}: {result}");
    result
}

fn failed(session: &mut Session, id: &str, params: Value) -> String {
    let result = run(session, id, params.clone());
    assert_eq!(result["status"], "failed", "{id} {params}: {result}");
    result["error"].as_str().unwrap().to_string()
}

fn inspect(session: &mut Session) -> Value {
    run(session, "document.inspect", json!({}))
}

fn bodies(session: &mut Session) -> Value {
    inspect(session)["bodies"].clone()
}

fn feature(session: &mut Session, name: &str) -> Value {
    let document = inspect(session);
    document["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == name)
        .unwrap_or_else(|| panic!("no feature {name} in {document}"))
        .clone()
}

fn steps(session: &mut Session, steps: &[(&str, Value)]) {
    for (id, params) in steps {
        let result = run(session, id, params.clone());
        if result.get("status").is_some() {
            assert_eq!(result["status"], "ok", "{id} {params}: {result}");
        }
    }
}

/// A full name: `name` with the copies of the features in `prefix` in
/// front, outermost first (`["F3", "F2"]` gives `F3:copy(0)/F2:copy(0)/…`).
fn full(prefix: &[&str], name: &str) -> String {
    let mut out = String::new();
    for feature in prefix {
        out.push_str(&format!("{feature}:copy(0)/"));
    }
    out.push_str(name);
    out
}

// ---------------------------------------------------------------------
// The socket head cap screw of the morning of 2026-10-14

fn bolt_blank(session: &mut Session) {
    run(session, "file.new", json!({"units": "mm"}));
    let hex = [
        [4.0, 2.309401],
        [0.0, 4.618802],
        [-4.0, 2.309401],
        [-4.0, -2.309401],
        [0.0, -4.618802],
        [4.0, -2.309401],
    ];
    let list: Vec<(&str, Value)> = vec![
        (
            "feature.cylinder",
            json!({"position": [0, 0, 0], "radius": "8 mm", "height": "10 mm", "name": "Head"}),
        ),
        (
            "feature.cylinder",
            json!({"position": [0, 0, -40], "radius": "5 mm", "height": "40 mm", "name": "Shank"}),
        ),
        (
            "feature.boolean",
            json!({"op": "union", "target": "Head", "tool": "Shank", "name": "Blank"}),
        ),
        (
            "sketch.create",
            json!({"plane": "xy", "offset": "10 mm", "name": "Hex"}),
        ),
        (
            "sketch.line",
            json!({"sketch": "Hex", "from": hex[0], "to": hex[1]}),
        ),
        (
            "sketch.line",
            json!({"sketch": "Hex", "from": "e2", "to": hex[2]}),
        ),
        (
            "sketch.line",
            json!({"sketch": "Hex", "from": "e4", "to": hex[3]}),
        ),
        (
            "sketch.line",
            json!({"sketch": "Hex", "from": "e6", "to": hex[4]}),
        ),
        (
            "sketch.line",
            json!({"sketch": "Hex", "from": "e8", "to": hex[5]}),
        ),
        (
            "sketch.line",
            json!({"sketch": "Hex", "from": "e10", "to": "e1"}),
        ),
        (
            "feature.extrude",
            json!({"sketch": "Hex", "distance": "5 mm", "reversed": true, "cut": "Blank", "name": "Socket"}),
        ),
        ("sketch.create", json!({"plane": "zx", "name": "Cone"})),
        (
            "sketch.line",
            json!({"sketch": "Cone", "from": [5.0, 0], "to": [5.0, 4.0]}),
        ),
        (
            "sketch.line",
            json!({"sketch": "Cone", "from": "e2", "to": [2.6, 0]}),
        ),
        (
            "sketch.line",
            json!({"sketch": "Cone", "from": "e4", "to": "e1"}),
        ),
        (
            "feature.revolve",
            json!({"sketch": "Cone", "axis": "x", "cut": "Socket", "name": "SocketPoint"}),
        ),
    ];
    steps(session, &list);
}

/// The blends of the screw: `(command, name, [a, b], size)`, with the
/// names as the parts were made (`F1` the head, `F2` the shank).
const BOLT_BLENDS: [(&str, &str, [&str; 2], &str); 4] = [
    (
        "feature.fillet",
        "UnderHead",
        ["F1:face(1)", "F2:face(0)"],
        "0.4 mm",
    ),
    (
        "feature.fillet",
        "HeadTop",
        ["F1:face(0)", "F1:face(2)"],
        "0.8 mm",
    ),
    (
        "feature.chamfer",
        "HeadFoot",
        ["F1:face(0)", "F1:face(1)"],
        "0.3 mm",
    ),
    (
        "feature.chamfer",
        "Lead",
        ["F2:face(0)", "F2:face(1)"],
        "1 mm",
    ),
];

/// The copies in front of a face of the head (`F1`) or the shank (`F2`)
/// in the body before blend `index` (F10 is the first blend).
fn bolt_prefix(index: usize, name: &str) -> String {
    let mut out = String::new();
    for blend in (0..index).rev() {
        out.push_str(&format!("F{}:copy(0)/", 10 + blend));
    }
    out.push_str("F9:copy(0)/F6:copy(0)/");
    out.push_str(if name.starts_with("F1:") {
        "F3:copy(0)/"
    } else {
        "F3:copy(1)/"
    });
    out.push_str(name);
    out
}

/// The same name in the short form regeneration names faces by (a copy
/// of a copy is one link, its numbers the instances on the way, zeros at
/// the end left out): written out here from the bolt's history, not by
/// the code under test.
fn bolt_short(index: usize, name: &str) -> String {
    // The outermost copy is the last blend's, or the cut's (F9).
    let head = if index == 0 { 9 } else { 9 + index };
    if name.starts_with("F1:") {
        // The head: instance 0 all the way down.
        format!("F{head}:copy(0)/{name}")
    } else {
        // The shank: instance 0 through the blends, F9 and F6, then the
        // tool of F3.
        format!("F{head}:copy(0)/F3:copy(1)/{name}")
    }
}

fn bolt(session: &mut Session, short: bool) {
    bolt_blank(session);
    for (index, (command, name, [a, b], size)) in BOLT_BLENDS.iter().enumerate() {
        let edge = if short {
            json!([a, b])
        } else {
            json!([bolt_prefix(index, a), bolt_prefix(index, b)])
        };
        let size_key = if *command == "feature.fillet" {
            "radius"
        } else {
            "distance"
        };
        built(
            session,
            command,
            json!({"edges": [edge], size_key: size, "name": name}),
        );
    }
    let check = run(session, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
}

#[test]
fn the_screw_blended_by_the_names_its_parts_were_made_with_is_the_screw_of_full_names() {
    let mut long = session();
    bolt(&mut long, false);
    let mut short = session();
    bolt(&mut short, true);
    // The same body: volume, area, centroid, counts, every face name.
    let expected = bodies(&mut long);
    assert_eq!(bodies(&mut short), expected);
    assert_eq!(expected.as_array().unwrap().len(), 1);
    // Each blend was built from the full names, which it shows; the
    // document keeps the names as given.
    for (index, (_, name, [a, b], _)) in BOLT_BLENDS.iter().enumerate() {
        let given = feature(&mut short, name);
        assert_eq!(given["params"]["edges"], json!([[a, b]]), "{name}");
        assert_eq!(
            given["followed"]["edges"],
            json!([[bolt_short(index, a), bolt_short(index, b)]]),
            "{name}"
        );
        let full = feature(&mut long, name);
        assert_eq!(given["followed"], full["params"], "{name}");
        assert!(full.get("followed").is_none(), "{name}: {full}");
    }
}

#[test]
fn the_screw_saved_reopened_undone_and_redone_is_the_same_screw() {
    let dir = workdir("bolt");
    let mut s = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    bolt(&mut s, true);
    let expected = bodies(&mut s);
    let features = inspect(&mut s)["features"].clone();
    // Undo takes the last chamfer off; redo puts it back the same.
    run(&mut s, "edit.undo", json!({}));
    let before = bodies(&mut s);
    assert_ne!(before, expected);
    assert_eq!(before[0]["name"], "HeadFoot", "{before}");
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(bodies(&mut s), expected);
    let path = dir.join("bolt.3dcraft");
    run(
        &mut s,
        "file.save",
        json!({"path": path.to_str().unwrap(), "overwrite": true}),
    );
    run(&mut s, "file.open", json!({"path": path.to_str().unwrap()}));
    assert_eq!(bodies(&mut s), expected);
    // The names stay as given in the file, and are followed again.
    assert_eq!(inspect(&mut s)["features"], features);
}

#[test]
fn the_edges_of_the_screw_are_listed_with_the_short_names_too() {
    let mut s = session();
    bolt(&mut s, true);
    let listed = run(&mut s, "document.edges", json!({"feature": "Lead"}));
    let edges = listed["edges"].as_array().unwrap();
    assert!(!edges.is_empty());
    let mut seen_fillet = false;
    for edge in edges {
        let (faces, short) = (
            edge["faces"].as_array().unwrap(),
            edge["short"].as_array().unwrap(),
        );
        assert_eq!(faces.len(), short.len(), "{edge}");
        for (long, short) in faces.iter().zip(short) {
            let (long, short) = (long.as_str().unwrap(), short.as_str().unwrap());
            // Every short name is the innermost of the long one: no
            // pattern in this chain makes a copy ambiguous.
            assert!(long.ends_with(short), "{long} / {short}");
            assert!(!short.contains("copy("), "{long} / {short}");
            seen_fillet |= short == "F10:fillet(0)";
        }
    }
    assert!(seen_fillet, "{listed}");
    // A blend given the short names of an edge listed there is the blend
    // given its long names.
    let edge = edges
        .iter()
        .find(|e| {
            e["short"]
                .as_array()
                .unwrap()
                .iter()
                .any(|n| n == "F13:chamfer(0)")
        })
        .unwrap()
        .clone();
    let mut long = session();
    bolt(&mut long, true);
    let mut shorter = session();
    bolt(&mut shorter, true);
    built(
        &mut long,
        "feature.chamfer",
        json!({"edges": [edge["faces"]], "distance": "0.05 mm", "name": "Extra"}),
    );
    built(
        &mut shorter,
        "feature.chamfer",
        json!({"edges": [edge["short"]], "distance": "0.05 mm", "name": "Extra"}),
    );
    assert_eq!(bodies(&mut shorter), bodies(&mut long));
}

// ---------------------------------------------------------------------
// Every kind that takes faces of a body

/// A block of 40 × 20 × 10 (faces: 0 bottom, 1 top, 2 front, 3 back,
/// 4 left, 5 right), its top front edge rounded (`Round`, F2) and its top
/// back edge chamfered (`Bevel`, F3), both by the block's names.
fn rounded_block(session: &mut Session) {
    run(session, "file.new", json!({"units": "mm"}));
    run(
        session,
        "variable.set",
        json!({"name": "w", "expr": "40 mm"}),
    );
    built(
        session,
        "feature.block",
        json!({"size": ["w", 20, 10], "name": "Base"}),
    );
    built(
        session,
        "feature.fillet",
        json!({"edges": [["F1:face(1)", "F1:face(2)"]], "radius": 2, "name": "Round"}),
    );
    built(
        session,
        "feature.chamfer",
        json!({"edges": [["F1:face(1)", "F1:face(3)"]], "distance": 1, "name": "Bevel"}),
    );
}

/// The features to try, each by the short names and by the full ones in
/// the body of `Bevel`.
fn every_kind() -> Vec<(&'static str, Value, Value)> {
    let f1 = |name: &str| full(&["F3", "F2"], name);
    let f2 = |name: &str| full(&["F3"], name);
    vec![
        (
            "feature.fillet",
            json!({"edges": [["F1:face(0)", "F1:face(2)"]], "radius": 1, "name": "Next"}),
            json!({"edges": [[f1("F1:face(0)"), f1("F1:face(2)")]], "radius": 1, "name": "Next"}),
        ),
        (
            "feature.chamfer",
            json!({"edges": [["F1:face(0)", "F1:face(3)"]], "distance": 1, "name": "Next"}),
            json!({"edges": [[f1("F1:face(0)"), f1("F1:face(3)")]], "distance": 1, "name": "Next"}),
        ),
        (
            "feature.offsetFace",
            json!({"faces": ["F1:face(0)"], "distance": 1, "name": "Next"}),
            json!({"faces": [f1("F1:face(0)")], "distance": 1, "name": "Next"}),
        ),
        (
            "feature.deleteFace",
            json!({"faces": ["F2:fillet(0)"], "name": "Next"}),
            json!({"faces": [f2("F2:fillet(0)")], "name": "Next"}),
        ),
        (
            "feature.shell",
            json!({"source": "Base", "thickness": 1, "open": ["F1:face(0)"], "name": "Next"}),
            json!({"source": "Bevel", "thickness": 1, "open": [f1("F1:face(0)")], "name": "Next"}),
        ),
    ]
}

#[test]
fn every_kind_that_takes_faces_follows_the_body_they_were_named_in() {
    for (command, short, long) in every_kind() {
        let mut by_short = session();
        rounded_block(&mut by_short);
        built(&mut by_short, command, short.clone());
        let mut by_long = session();
        rounded_block(&mut by_long);
        built(&mut by_long, command, long.clone());
        let expected = bodies(&mut by_long);
        assert_eq!(bodies(&mut by_short), expected, "{command} {short}");
        assert_eq!(expected.as_array().unwrap().len(), 1, "{command}");
        assert_eq!(
            feature(&mut by_short, "Next")["followed"],
            feature(&mut by_long, "Next")["params"],
            "{command} {short}"
        );
        // An upstream edit: the block wider. Both follow it alike.
        run(
            &mut by_short,
            "variable.set",
            json!({"name": "w", "expr": "55 mm"}),
        );
        run(
            &mut by_long,
            "variable.set",
            json!({"name": "w", "expr": "55 mm"}),
        );
        let wider = bodies(&mut by_long);
        assert_ne!(wider, expected, "{command}");
        assert_eq!(bodies(&mut by_short), wider, "{command} {short}");
        assert_eq!(feature(&mut by_short, "Next")["status"], "ok");
    }
}

#[test]
fn a_feature_inserted_between_is_followed_through_and_a_move_changes_the_chain() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    built(
        &mut s,
        "feature.block",
        json!({"size": [40, 20, 10], "name": "Base"}),
    );
    built(
        &mut s,
        "feature.fillet",
        json!({"edges": [["F1:face(1)", "F1:face(2)"]], "radius": 2, "name": "Round"}),
    );
    built(
        &mut s,
        "feature.fillet",
        json!({"edges": [["F1:face(0)", "F1:face(2)"]], "radius": 1, "name": "Foot"}),
    );
    assert_eq!(
        feature(&mut s, "Foot")["followed"]["edges"],
        json!([["F2:copy(0)/F1:face(0)", "F2:copy(0)/F1:face(2)"]])
    );
    // Rolled back to the round, a chamfer goes in after it: the foot,
    // after the marker, is not built.
    run(&mut s, "history.rollback", json!({"feature": "Round"}));
    built(
        &mut s,
        "feature.chamfer",
        json!({"edges": [["F1:face(1)", "F1:face(3)"]], "distance": 1, "name": "Bevel"}),
    );
    assert_eq!(feature(&mut s, "Foot")["status"], "rolledBack");
    run(&mut s, "history.rollback", json!({"feature": null}));
    // Rolled forward, the foot takes the body of the chamfer: one body
    // with all three blends.
    let foot = feature(&mut s, "Foot");
    assert_eq!(foot["status"], "ok", "{foot}");
    assert_eq!(
        foot["followed"]["edges"],
        json!([["F4:copy(0)/F1:face(0)", "F4:copy(0)/F1:face(2)"]])
    );
    let all = bodies(&mut s);
    assert_eq!(all.as_array().unwrap().len(), 1, "{all}");
    assert_eq!(all[0]["name"], "Foot");
    let names: Vec<String> = all[0]["faceNames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap().to_string())
        .collect();
    for made in ["F2:fillet(0)", "F4:chamfer(0)", "F3:fillet(0)"] {
        assert!(names.iter().any(|n| n.ends_with(made)), "{made}: {names:?}");
    }
    // The same blends built in this order with full names.
    let mut oracle = session();
    run(&mut oracle, "file.new", json!({"units": "mm"}));
    built(
        &mut oracle,
        "feature.block",
        json!({"size": [40, 20, 10], "name": "Base"}),
    );
    built(
        &mut oracle,
        "feature.fillet",
        json!({"edges": [["F1:face(1)", "F1:face(2)"]], "radius": 2, "name": "Round"}),
    );
    built(
        &mut oracle,
        "feature.chamfer",
        json!({"edges": [["F2:copy(0)/F1:face(1)", "F2:copy(0)/F1:face(3)"]], "distance": 1, "name": "Bevel"}),
    );
    built(
        &mut oracle,
        "feature.fillet",
        json!({"edges": [["F3:copy(0)/F2:copy(0)/F1:face(0)", "F3:copy(0)/F2:copy(0)/F1:face(2)"]], "radius": 1, "name": "Foot"}),
    );
    let expected = bodies(&mut oracle);
    for key in ["volume", "area", "centroid", "faces", "edges", "vertices"] {
        assert_eq!(all[0][key], expected[0][key], "{key}");
    }
    // Moved to the end, the chamfer follows the foot, and the foot no
    // longer goes through the chamfer.
    run(
        &mut s,
        "feature.move",
        json!({"feature": "Bevel", "position": 3}),
    );
    assert_eq!(
        feature(&mut s, "Foot")["followed"]["edges"],
        json!([["F2:copy(0)/F1:face(0)", "F2:copy(0)/F1:face(2)"]])
    );
    assert_eq!(
        feature(&mut s, "Bevel")["followed"]["edges"],
        json!([["F3:copy(0)/F1:face(1)", "F3:copy(0)/F1:face(3)"]])
    );
    let moved = bodies(&mut s);
    assert_eq!(moved.as_array().unwrap().len(), 1, "{moved}");
    assert_eq!(moved[0]["name"], "Bevel");
    let (a, b) = (
        moved[0]["volume"].as_f64().unwrap(),
        expected[0]["volume"].as_f64().unwrap(),
    );
    assert!((a - b).abs() <= 1e-9 * b, "{a} {b}");
    // Undo puts the chamfer back before the foot.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(bodies(&mut s)[0]["volume"], expected[0]["volume"]);
    // Suppressed, the chamfer takes no body, and the foot follows the
    // round alone.
    run(&mut s, "feature.suppress", json!({"feature": "Bevel"}));
    assert_eq!(
        feature(&mut s, "Foot")["followed"]["edges"],
        json!([["F2:copy(0)/F1:face(0)", "F2:copy(0)/F1:face(2)"]])
    );
}

#[test]
fn a_face_a_boolean_cut_in_two_is_both_pieces_as_its_full_name_is() {
    let build = |edge: Value| {
        let mut s = session();
        run(&mut s, "file.new", json!({"units": "mm"}));
        built(
            &mut s,
            "feature.block",
            json!({"size": [40, 20, 10], "name": "Base"}),
        );
        built(
            &mut s,
            "feature.block",
            json!({"position": [15, -1, 5], "size": [10, 22, 6], "name": "Slot"}),
        );
        built(
            &mut s,
            "feature.boolean",
            json!({"op": "subtract", "target": "Base", "tool": "Slot", "name": "Cut"}),
        );
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": [edge], "radius": 1, "name": "Round"}),
        );
        s
    };
    let mut short = build(json!(["F1:face(1)", "F1:face(2)"]));
    let mut long = build(json!(["F3:copy(0)/F1:face(1)", "F3:copy(0)/F1:face(2)"]));
    let expected = bodies(&mut long);
    assert_eq!(bodies(&mut short), expected);
    // The top is two faces now, and both of its edges along the front
    // are rounded.
    let names = expected[0]["faceNames"].as_array().unwrap();
    for piece in ["F4:fillet(0,0)", "F4:fillet(0,1)"] {
        assert!(names.iter().any(|n| n == piece), "{piece}: {names:?}");
    }
}

#[test]
fn a_face_a_feature_took_away_fails_naming_that_feature() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    built(
        &mut s,
        "feature.block",
        json!({"size": [40, 20, 10], "name": "Base"}),
    );
    built(
        &mut s,
        "feature.fillet",
        json!({"edges": [["F1:face(1)", "F1:face(2)"]], "radius": 2, "name": "Round"}),
    );
    built(
        &mut s,
        "feature.deleteFace",
        json!({"faces": ["F2:fillet(0)"], "name": "Sharp"}),
    );
    let reason = failed(
        &mut s,
        "feature.chamfer",
        json!({"edges": [["F1:face(4)", "F2:fillet(0)"]], "distance": 1, "name": "Next"}),
    );
    assert!(
        reason.starts_with("edges.0: the face F2:fillet(0) is gone")
            && reason.contains("`Sharp`, a deleteFace"),
        "{reason}"
    );
    // The faces of the block are all there still: the edge the round was
    // is back, and it is followed to.
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "Next", "edges": [["F1:face(1)", "F1:face(2)"]]}),
    );
    // A name the body never had is said as such, not as lost on the way.
    let reason = failed(
        &mut s,
        "feature.fillet",
        json!({"edges": [["F1:face(1)", "F1:face(9)"]], "radius": 1}),
    );
    assert!(
        reason.starts_with("edges.0: `Base` has no face F1:face(9)"),
        "{reason}"
    );
}

#[test]
fn a_face_with_several_copies_is_refused_with_all_of_them() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    built(
        &mut s,
        "feature.block",
        json!({"size": [40, 20, 10], "name": "Base"}),
    );
    built(
        &mut s,
        "feature.pattern",
        json!({"source": "Base", "count": 2, "step": [60, 0, 0], "name": "Row"}),
    );
    let reason = failed(
        &mut s,
        "feature.fillet",
        json!({"edges": [["F1:face(1)", "F1:face(2)"]], "radius": 1, "name": "Round"}),
    );
    assert!(
        reason.starts_with("edges.0: the face F1:face(1) is more than one face after `Row`")
            && reason.contains("F2:copy(0)/F1:face(1), F2:copy(1)/F1:face(1)"),
        "{reason}"
    );
    // One of them, by its name in the pattern, is followed from there on.
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "Round", "edges": [["F2:copy(1)/F1:face(1)", "F2:copy(1)/F1:face(2)"]]}),
    );
    built(
        &mut s,
        "feature.chamfer",
        json!({"edges": [["F2:copy(1)/F1:face(0)", "F2:copy(1)/F1:face(2)"]], "distance": 1, "name": "Bevel"}),
    );
    // Their short names keep the copy of the pattern, and no more.
    let listed = run(&mut s, "document.edges", json!({"feature": "Bevel"}));
    let shorts: Vec<String> = listed["edges"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|e| e["short"].as_array().unwrap().clone())
        .map(|n| n.as_str().unwrap().to_string())
        .collect();
    for name in [
        "F2:copy(0)/F1:face(1)",
        "F2:copy(1)/F1:face(1)",
        "F3:fillet(0)",
        "F4:chamfer(0)",
    ] {
        assert!(shorts.iter().any(|n| n == name), "{name}: {shorts:?}");
    }
    // A shell's skin and hollow both have a copy of every face.
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    built(
        &mut s,
        "feature.block",
        json!({"size": [40, 20, 10], "name": "Base"}),
    );
    built(
        &mut s,
        "feature.shell",
        json!({"source": "Base", "thickness": 1, "open": ["F1:face(1)"], "name": "Box"}),
    );
    let reason = failed(
        &mut s,
        "feature.offsetFace",
        json!({"faces": ["F1:face(2)"], "distance": 1}),
    );
    assert!(
        reason.starts_with("faces.0: the face F1:face(2) is more than one face after `Box`"),
        "{reason}"
    );
}

#[test]
fn faces_of_bodies_that_never_came_together_are_refused_at_once() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    built(
        &mut s,
        "feature.block",
        json!({"size": [40, 20, 10], "name": "Base"}),
    );
    built(
        &mut s,
        "feature.block",
        json!({"position": [60, 0, 0], "size": [10, 10, 10], "name": "Other"}),
    );
    for (command, params) in [
        (
            "feature.fillet",
            json!({"edges": [["F1:face(1)", "F2:face(2)"]], "radius": 1}),
        ),
        (
            "feature.offsetFace",
            json!({"faces": ["F1:face(1)", "F2:face(1)"], "distance": 1}),
        ),
    ] {
        let error = s.execute(command, &params).unwrap_err().to_string();
        assert!(
            error.contains("is a face of `Base` and")
                && error.contains("of `Other`, which are not one body"),
            "{command}: {error}"
        );
    }
    // United, they are one body, and the same faces are accepted.
    built(
        &mut s,
        "feature.boolean",
        json!({"op": "union", "target": "Base", "tool": "Other", "name": "Both"}),
    );
    built(
        &mut s,
        "feature.offsetFace",
        json!({"faces": ["F1:face(1)", "F2:face(1)"], "distance": 1}),
    );
}
