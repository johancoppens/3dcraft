//! The bodies that went into later features are kept within a budget
//! (`Session::set_body_budget`, `CRAFT3D_BODY_BUDGET_MB`); beyond it they
//! are let go of and derived again when a command needs them, so a chain
//! of unions built command by command costs the budget plus the
//! document's own bodies, not one body per step
//! (docs/design/regeneration.md).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};

/// `n` blocks apart, united one after the other.
fn chain(session: &mut Session, n: usize) {
    for i in 0..n {
        run(
            session,
            "feature.block",
            json!({"name": format!("B{i}"), "size": [10, 10, 10], "position": [20 * i, 0, 0]}),
        );
    }
    let mut target = "B0".to_string();
    for i in 1..n {
        let name = format!("U{i}");
        run(
            session,
            "feature.boolean",
            json!({"op": "union", "target": target, "tool": format!("B{i}"), "name": name}),
        );
        target = name;
    }
}

#[test]
fn within_a_budget_of_nothing_only_the_documents_bodies_are_kept_and_every_command_still_holds() {
    let mut s = Session::new(Workspace::none());
    s.set_body_budget(0);
    run(&mut s, "file.new", json!({}));
    chain(&mut s, 6);
    let (bytes, gone) = s.bodies_held().unwrap();
    assert_eq!((bytes, gone), (0, 10));
    let inspect = run(&mut s, "document.inspect", json!({}));
    assert_eq!(inspect["bodies"].as_array().unwrap().len(), 1);
    // The last union's tool changes: the union is rebuilt from the union
    // before it, which was let go of and is derived again, with
    // everything it is made from.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "B5", "size": [12, 10, 10]}),
    );
    assert_eq!(s.last_regeneration().unwrap().0, 2);
    assert_eq!(s.last_regeneration_rederived().unwrap(), 9);
    assert_eq!(s.bodies_held().unwrap(), (0, 10));
    let measured = run(&mut s, "document.measure", json!({}));
    assert!(
        (measured["volume"].as_f64().unwrap() - 6200.0).abs() < 1e-6,
        "{measured}"
    );
    // A deep check holds every boolean to its operands, also those let
    // go of: it has them derived again, and they are let go of again at
    // the next change.
    let check = run(&mut s, "document.check", json!({"deep": true}));
    assert_eq!(
        (check["ok"].as_bool(), check["booleansChecked"].as_u64()),
        (Some(true), Some(5)),
        "{check}"
    );
    assert_eq!(s.bodies_held().unwrap().1, 0);
    // What a datum can be placed by on a body that went into a later
    // feature is listed from that body.
    let references = run(&mut s, "datum.references", json!({"feature": "U2"}));
    assert_eq!(
        references["vertices"].as_array().unwrap().len(),
        24,
        "{references}"
    );
    // Undone, the next command that reads the document regenerates it,
    // and the bodies derived again for the check are let go of.
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "document.inspect", json!({}));
    assert_eq!(s.bodies_held().unwrap(), (0, 10));
    let references = run(&mut s, "datum.references", json!({"feature": "U2"}));
    assert_eq!(
        references["vertices"].as_array().unwrap().len(),
        24,
        "{references}"
    );
    // That body alone is kept, as asked for.
    let (held, gone) = s.bodies_held().unwrap();
    assert!(held > 0 && gone == 9, "{held} held, {gone} gone");
    // Undo, redo and a deletion in the middle give what a regeneration
    // from nothing gives, body for body.
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(s.regeneration_matches_a_full_one(), Some(true));
    // U3 gone, U4 and U5 fail and U2, B3, B4 and B5 are the document's
    // bodies again, U2 derived again.
    run(&mut s, "feature.delete", json!({"feature": "U3"}));
    let inspect = run(&mut s, "document.inspect", json!({}));
    assert_eq!(inspect["bodies"].as_array().unwrap().len(), 4, "{inspect}");
    assert_eq!(s.regeneration_matches_a_full_one(), Some(true));
}

#[test]
fn a_budget_that_holds_the_last_unions_operands_keeps_an_edit_at_the_end_cheap() {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({}));
    chain(&mut s, 8);
    let (all, gone) = s.bodies_held().unwrap();
    assert_eq!(gone, 0);
    // The tool of the last union and the union before it: about a block
    // and a body of seven blocks, out of the chain's thirty-five.
    let budget = all * 8 / 35 + 1;
    s.set_body_budget(budget);
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "B7", "size": [12, 10, 10]}),
    );
    assert_eq!(s.last_regeneration_rederived().unwrap(), 0);
    let (held, gone) = s.bodies_held().unwrap();
    assert!(held <= budget && gone == 12, "{held} held, {gone} gone");
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "B7", "size": [14, 10, 10]}),
    );
    assert_eq!(
        (
            s.last_regeneration().unwrap().0,
            s.last_regeneration_rederived().unwrap()
        ),
        (2, 0)
    );
    // An edit at the start rebuilds the chain, deriving again the tools
    // that were let go of, once each; the result is the one a
    // regeneration from nothing gives.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "B0", "size": [14, 10, 10]}),
    );
    assert_eq!(
        (
            s.last_regeneration().unwrap().0,
            s.last_regeneration_rederived().unwrap()
        ),
        (8, 6)
    );
    assert_eq!(s.regeneration_matches_a_full_one(), Some(true));
}

/// The same commands on sessions with a budget of nothing, of forty
/// kilobytes, of a hundred and fifty, and without one: every result,
/// every error and the full-regeneration oracle agree, and the first
/// session lets bodies go and derives them again. A union that keeps
/// its tool, a transform that keeps its source (both taken later), a
/// pattern of that transform, datums at vertices of kept and taken
/// bodies, blends, sketches on faces, an extrusion up to a face, a
/// failing boolean, edits at the start, undo, suppress, delete and roll
/// back (the reviewer's probe of 2026-10-19).
#[test]
fn every_command_gives_the_same_result_whatever_the_budget() {
    let script: Vec<(&str, Value)> = vec![
        ("file.new", json!({})),
        (
            "feature.block",
            json!({"size":[10,10,10], "position":[0,0,0], "name":"B0"}),
        ),
        (
            "feature.block",
            json!({"size":[10,10,10], "position":[8,0,0], "name":"B1"}),
        ),
        (
            "feature.block",
            json!({"size":[10,10,10], "position":[16,0,0], "name":"B2"}),
        ),
        (
            "feature.block",
            json!({"size":[10,10,10], "position":[24,0,0], "name":"B3"}),
        ),
        (
            "feature.block",
            json!({"size":[10,10,10], "position":[32,0,0], "name":"B4"}),
        ),
        (
            "feature.block",
            json!({"size":[10,10,10], "position":[40,0,0], "name":"B5"}),
        ),
        (
            "feature.boolean",
            json!({"op":"union", "target":"B0", "tool":"B1", "name":"U1"}),
        ),
        (
            "feature.boolean",
            json!({"op":"union", "target":"U1", "tool":"B2", "name":"U2"}),
        ),
        (
            "feature.boolean",
            json!({"op":"union", "target":"U2", "tool":"B3", "name":"U3"}),
        ),
        (
            "feature.boolean",
            json!({"op":"union", "target":"U3", "tool":"B4", "name":"U4"}),
        ),
        (
            "feature.transform",
            json!({"source":"B5", "translate":[0,30,0], "keep":true, "name":"T"}),
        ),
        (
            "feature.boolean",
            json!({"op":"union", "target":"U4", "tool":"B5", "name":"U5"}),
        ),
        (
            "feature.pattern",
            json!({"source":"T", "step":[0,15,0], "count":3, "name":"PT"}),
        ),
        ("datum.point", json!({"vertex":"F1:vertex(4)", "name":"P"})),
        ("datum.point", json!({"vertex":"F2:vertex(7)", "name":"Q"})),
        (
            "datum.point",
            json!({"vertex":"F11:copy(0)/F6:vertex(7)", "name":"R"}),
        ),
        (
            "feature.chamfer",
            json!({"edges":[["F1:face(1)","F1:face(2)"]], "distance":2, "name":"C"}),
        ),
        (
            "feature.fillet",
            json!({"edges":[["F3:face(1)","F3:face(3)"]], "radius":1, "name":"FL"}),
        ),
        ("sketch.create", json!({"face":"F1:face(2)", "name":"S"})),
        (
            "sketch.circle",
            json!({"sketch":"S", "center":[5,5], "radius":2}),
        ),
        (
            "feature.extrude",
            json!({"sketch":"S", "distance":5, "reversed":true, "name":"E"}),
        ),
        (
            "sketch.create",
            json!({"plane":"xy", "offset":-5, "name":"S2"}),
        ),
        (
            "sketch.circle",
            json!({"sketch":"S2", "center":[20,5], "radius":1}),
        ),
        (
            "feature.extrude",
            json!({"sketch":"S2", "upTo":"F3:face(1)", "name":"E2"}),
        ),
        (
            "feature.boolean",
            json!({"op":"union", "target":"FL", "tool":"B1", "name":"BAD"}),
        ),
        (
            "feature.boolean",
            json!({"op":"subtract", "target":"FL", "tool":"E", "name":"CUT"}),
        ),
        (
            "feature.block",
            json!({"size":[10,10,10], "position":[0,60,0], "name":"B6"}),
        ),
        (
            "feature.block",
            json!({"size":[10,10,10], "position":[8,60,0], "name":"B7"}),
        ),
        (
            "feature.boolean",
            json!({"op":"union", "target":"B6", "tool":"B7", "keepTool":true, "name":"KT"}),
        ),
        (
            "feature.transform",
            json!({"source":"B7", "translate":[0,20,0], "name":"K2"}),
        ),
        (
            "feature.transform",
            json!({"source":"KT", "translate":[0,40,0], "name":"K3"}),
        ),
        ("datum.references", json!({"feature":"KT"})),
        ("datum.point", json!({"vertex":"F1:vertex(0)", "name":"P0"})),
        ("document.inspect", json!({})),
        ("document.measure", json!({})),
        ("document.check", json!({"deep":true})),
        ("document.check", json!({})),
        ("datum.references", json!({"feature":"B0"})),
        ("datum.references", json!({"feature":"U2"})),
        ("datum.references", json!({"feature":"T"})),
        ("datum.references", json!({"feature":"PT"})),
        ("document.edges", json!({"feature":"CUT"})),
        ("feature.edit", json!({"feature":"B0", "size":[12,10,10]})),
        ("document.inspect", json!({})),
        ("document.measure", json!({})),
        ("document.check", json!({"deep":true})),
        ("datum.references", json!({"feature":"B0"})),
        ("datum.references", json!({"feature":"U2"})),
        ("feature.edit", json!({"feature":"B2", "position":[16,1,0]})),
        ("document.inspect", json!({})),
        ("document.measure", json!({})),
        ("edit.undo", json!({})),
        ("edit.undo", json!({})),
        ("document.inspect", json!({})),
        ("document.measure", json!({})),
        (
            "feature.suppress",
            json!({"feature":"U3", "suppressed":true}),
        ),
        ("document.inspect", json!({})),
        ("document.measure", json!({})),
        (
            "feature.suppress",
            json!({"feature":"U3", "suppressed":false}),
        ),
        ("document.inspect", json!({})),
        ("document.measure", json!({})),
        ("feature.delete", json!({"feature":"U3"})),
        ("document.inspect", json!({})),
        ("document.measure", json!({})),
        ("edit.undo", json!({})),
        ("document.inspect", json!({})),
        ("document.check", json!({"deep":true})),
        ("datum.references", json!({"feature":"U4"})),
        ("history.rollback", json!({"feature":"U3"})),
        ("document.inspect", json!({})),
        ("document.measure", json!({})),
        ("datum.references", json!({"feature":"U2"})),
        ("history.rollback", json!({"feature":null})),
        ("document.inspect", json!({})),
        ("document.measure", json!({})),
        ("feature.edit", json!({"feature":"T", "translate":[0,31,0]})),
        ("datum.references", json!({"feature":"PT"})),
        ("document.measure", json!({})),
        ("feature.edit", json!({"feature":"B5", "size":[10,10,11]})),
        ("datum.references", json!({"feature":"PT"})),
        ("datum.references", json!({"feature":"T"})),
        ("document.measure", json!({})),
    ];
    let mut sessions: Vec<Session> = [0usize, 40_000, 150_000, usize::MAX]
        .into_iter()
        .map(|bytes| {
            let mut s = Session::new(Workspace::none());
            s.set_body_budget(bytes);
            s
        })
        .collect();
    let (mut most_gone, mut rederived) = (0usize, 0usize);
    for (n, (id, params)) in script.iter().enumerate() {
        let outputs: Vec<String> = sessions
            .iter_mut()
            .map(|s| match s.execute(id, params) {
                Ok(value) => serde_json::to_string(&value).unwrap(),
                Err(e) => format!("ERR {}", e.message()),
            })
            .collect();
        for (which, output) in outputs.iter().enumerate() {
            assert_eq!(
                output, &outputs[3],
                "step {n} {id} {params}: session {which} differs from the unlimited one"
            );
        }
        most_gone = most_gone.max(sessions[0].bodies_held().unwrap().1);
        rederived += sessions[0].last_regeneration_rederived().unwrap();
    }
    for s in &mut sessions {
        assert_eq!(s.regeneration_matches_a_full_one(), Some(true));
    }
    assert!(
        most_gone >= 10 && rederived >= 10,
        "{most_gone} gone at most, {rederived} derived again"
    );
}
