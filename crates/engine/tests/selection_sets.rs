//! Selection, slice 2 (decisions B2 of 2026-10-21 and the next list of
//! docs/design/selection.md): named sets kept in the document as queries
//! and computed again at every regeneration; a selection that follows its
//! faces and bodies into later features and keeps what a suppressed or
//! rolled-back feature hides; the context an agent needs to act on an
//! item (its measure and what drives it); and the bugs found live on
//! 2026-10-21. The oracles are closed-form: how many holes a plate was
//! drilled with, the radius it was drilled with, the top of a quadratic
//! Bézier arc.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};
use std::collections::BTreeSet;

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn refused(session: &mut Session, id: &str, params: Value) -> String {
    match session.execute(id, &params) {
        Ok(value) => panic!("{id} {params} should fail, returned {value}"),
        Err(e) => e.message().to_string(),
    }
}

fn names(result: &Value) -> Vec<String> {
    result["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["name"].as_str().unwrap().to_string())
        .collect()
}

fn items(result: &Value) -> Vec<(String, String, String)> {
    result["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            (
                item["kind"].as_str().unwrap().to_string(),
                item["feature"].as_str().unwrap().to_string(),
                item["name"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn item(kind: &str, feature: &str, name: &str) -> (String, String, String) {
    (kind.to_string(), feature.to_string(), name.to_string())
}

/// A plate 80 × 50 × 8 (F1) with `count` holes of 6 mm (F2, a cylinder of
/// radius 3, patterned by F3) and one of 10 mm (F4), drilled by
/// subtracting (F5, F6).
fn plate(count: u32) -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "variable.set",
        json!({"name": "n", "expr": count.to_string()}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": [80, 50, 8]}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Drill", "radius": "3 mm", "height": 20, "position": [10, 10, -5]}),
    );
    run(
        &mut s,
        "feature.pattern",
        json!({"name": "Drills", "source": "Drill", "count": "n", "step": [10, 0, 0]}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Bore", "radius": "5 mm", "height": 20, "position": [40, 35, -5]}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Plate", "tool": "Drills", "name": "Drilled"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Drilled", "tool": "Bore", "name": "Part"}),
    );
    s
}

fn set_count(s: &mut Session, name: &str) -> Value {
    let listed = run(s, "set.list", json!({}));
    listed["sets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|set| set["name"] == name)
        .map(|set| set.get("count").cloned().unwrap_or(set["error"].clone()))
        .unwrap_or(Value::Null)
}

// ------------------------------------------------------------ named sets

/// A set is the query, not what it found: four holes of 6 mm, then six
/// once the pattern has six; it is a filter and a selection, survives a
/// rename of the feature it names (kept by id) and is undone like any edit.
#[test]
fn a_named_set_grows_with_the_design() {
    let mut s = plate(4);
    let defined = run(
        &mut s,
        "set.define",
        json!({"name": "boltholes", "kind": "face", "where": {"hole": true, "radius": "3 mm"}}),
    );
    assert_eq!(defined["count"], 4, "{defined}");
    assert_eq!(defined["kind"], "face");
    // A set naming a feature keeps it by id.
    let row = run(
        &mut s,
        "set.define",
        json!({"name": "row", "kind": "face", "where": {"copyOf": "Drills", "hole": true}}),
    );
    assert_eq!(row["where"]["copyOf"], "F3", "{row}");
    assert_eq!(row["count"], 4);

    run(&mut s, "variable.set", json!({"name": "n", "expr": "6"}));
    assert_eq!(set_count(&mut s, "boltholes"), 6);
    run(
        &mut s,
        "feature.rename",
        json!({"feature": "Drills", "name": "Holes"}),
    );
    assert_eq!(set_count(&mut s, "row"), 6, "a rename changes nothing");

    // As a filter, alone or with others, and as the selection.
    let found = run(
        &mut s,
        "select.query",
        json!({"where": {"set": "boltholes", "above": {"x": 30}}}),
    );
    assert_eq!(found["count"], 3, "the holes at x = 30, 40 and 50: {found}");
    let selected = run(&mut s, "select.set", json!({"set": "boltholes"}));
    assert_eq!(selected["count"], 6);
    let all = run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"hole": true, "radius": "3 mm"}}),
    );
    assert_eq!(names(&selected), names(&all), "in document order");
    let more = run(
        &mut s,
        "select.set",
        json!({"set": "boltholes", "names": ["Part"], "mode": "add"}),
    );
    assert_eq!(more["count"], 7, "{more}");

    // A set may use another; one that uses itself is refused.
    run(
        &mut s,
        "set.define",
        json!({"name": "left", "where": {"set": "boltholes", "below": {"x": 25}}}),
    );
    assert_eq!(set_count(&mut s, "left"), 2);
    let cycle = refused(
        &mut s,
        "set.edit",
        json!({"set": "boltholes", "where": {"set": "left"}}),
    );
    assert!(cycle.contains("uses itself"), "{cycle}");

    // Edit, rename, list in order, remove; each is an undo step.
    let edited = run(
        &mut s,
        "set.edit",
        json!({"set": "boltholes", "where": {"hole": true, "radius": "5 mm"}, "name": "bore"}),
    );
    assert_eq!(edited["name"], "bore");
    assert_eq!(edited["count"], 1);
    assert_eq!(edited["kind"], "face", "kind was kept");
    let listed = run(&mut s, "set.list", json!({}));
    let order: Vec<&str> = listed["sets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|set| set["name"].as_str().unwrap())
        .collect();
    assert_eq!(order, ["bore", "row", "left"]);
    // `left` used `boltholes`, which is now called `bore`: it follows the
    // rename (as a formula follows a renamed variable), so a set defined
    // later under the old name does not change what it holds.
    assert_eq!(listed["sets"][2]["where"]["set"], "bore", "{listed}");
    assert_eq!(set_count(&mut s, "left"), 0, "no 10 mm hole left of x = 25");
    run(
        &mut s,
        "set.define",
        json!({"name": "boltholes", "kind": "face", "where": {"hole": true}}),
    );
    assert_eq!(set_count(&mut s, "left"), 0);
    // A set another uses is not removed from under it.
    let used = refused(&mut s, "set.remove", json!({"set": "bore"}));
    assert!(used.contains("used by `left`"), "{used}");
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(set_count(&mut s, "boltholes"), 6);
    assert_eq!(set_count(&mut s, "left"), 2);
    let inspect = run(&mut s, "document.inspect", json!({"measures": false}));
    assert_eq!(inspect["sets"][0]["name"], "boltholes");
    assert_eq!(inspect["sets"][0]["where"]["radius"], "3 mm");
    let removed = run(&mut s, "set.remove", json!({"set": "row"}));
    assert_eq!(removed["removed"], "row");
    assert_eq!(set_count(&mut s, "row"), Value::Null);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(set_count(&mut s, "row"), 6);

    // A feature a set names is deleted: the set says why it does not read.
    run(&mut s, "feature.delete", json!({"feature": "Part"}));
    run(&mut s, "feature.delete", json!({"feature": "Drilled"}));
    run(&mut s, "feature.delete", json!({"feature": "Holes"}));
    let gone = set_count(&mut s, "row");
    assert!(gone.as_str().is_some_and(|e| e.contains("F3")), "{gone}");
    assert!(refused(&mut s, "select.set", json!({"set": "row"})).starts_with("select.set"));
}

/// Hostile params of the set commands are errors that say what is wrong,
/// and change nothing.
#[test]
fn set_commands_refuse_what_means_nothing() {
    let mut s = plate(2);
    run(
        &mut s,
        "set.define",
        json!({"name": "a", "where": {"hole": true}}),
    );
    let before = run(&mut s, "set.list", json!({}));
    for (id, params, says) in [
        ("set.define", json!({}), "name"),
        ("set.define", json!({"name": ""}), "empty"),
        ("set.define", json!({"name": " a"}), "space"),
        ("set.define", json!({"name": "a"}), "already exists"),
        (
            "set.define",
            json!({"name": "b", "where": {"bogus": 1}}),
            "unknown filter",
        ),
        ("set.define", json!({"name": "b", "kind": "planet"}), "kind"),
        (
            "set.define",
            json!({"name": "b", "where": {"set": "nope"}}),
            "no set",
        ),
        (
            "set.define",
            json!({"name": "b", "where": {"set": "b"}}),
            "no set",
        ),
        (
            "set.define",
            json!({"name": "b", "where": {"in": "Nothing"}}),
            "no feature",
        ),
        ("set.define", json!({"name": "b", "where": 3}), "filter"),
        ("set.define", json!({"name": 3}), "name"),
        ("set.edit", json!({"set": "nope"}), "no set"),
        ("set.edit", json!({"set": "a", "name": ""}), "empty"),
        (
            "set.edit",
            json!({"set": "a", "where": {"radius": "3 deg"}}),
            "length",
        ),
        ("set.remove", json!({"set": "nope"}), "no set"),
        ("set.remove", json!({}), "set"),
        ("set.list", json!({"x": 1}), "x"),
        ("select.set", json!({"set": "nope"}), "no set"),
        ("select.set", json!({"set": 3}), "set"),
        ("select.set", json!({"set": "a", "mode": "clear"}), "clear"),
        ("select.query", json!({"where": {"set": "nope"}}), "no set"),
        ("select.query", json!({"where": {"set": ["a"]}}), "string"),
        ("select.query", json!({"context": "yes"}), "context"),
        ("select.get", json!({"context": 1}), "context"),
    ] {
        let message = refused(&mut s, id, params.clone());
        assert!(message.contains(says), "{id} {params}: {message}");
    }
    assert_eq!(run(&mut s, "set.list", json!({})), before);
    let mut empty = Session::new(Workspace::none());
    for id in ["set.define", "set.edit", "set.remove", "set.list"] {
        assert!(empty.execute(id, &json!({})).is_err());
    }
}

// -------------------------------------------- following, hidden items

/// A face and a body that a later boolean takes are followed to their
/// image there; a face the boolean cuts away drops out and says why;
/// suppressing the boolean brings back the faces as they were, and
/// undoing that follows them again.
#[test]
fn a_selection_follows_its_faces_into_a_later_boolean() {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": [80, 50, 8]}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Drill", "radius": 3, "height": 20, "position": [10, 10, -5]}),
    );
    // The top of the plate, an edge along its top front, the plate, the
    // drill's wall and its bottom (below the plate: cut away).
    let edge = run(
        &mut s,
        "select.query",
        json!({"kind": "edge", "where": {"in": "Plate", "length": 80, "above": {"z": 8}, "below": {"y": 0}}}),
    );
    assert_eq!(edge["count"], 1, "{edge}");
    let edge = edge["items"][0]["name"].as_str().unwrap().to_string();
    run(
        &mut s,
        "select.set",
        json!({"items": [
            {"kind": "face", "name": "F1:face(1)"},
            {"kind": "edge", "name": edge, "feature": "Plate"},
            {"kind": "body", "feature": "Plate"},
            {"kind": "face", "name": "F2:face(0)"},
            {"kind": "face", "name": "F2:face(1)"},
        ]}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Plate", "tool": "Drill", "name": "Cut"}),
    );
    let got = run(&mut s, "select.get", json!({}));
    let now = items(&got);
    assert_eq!(now.len(), 4, "{got}");
    assert_eq!(now[0], item("face", "F3", "F3:copy(0)/F1:face(1)"));
    assert_eq!(now[1].0, "edge");
    assert_eq!(now[1].1, "F3", "{got}");
    assert_eq!(now[2], item("body", "F3", "Cut"));
    assert_eq!(now[3], item("face", "F3", "F3:copy(1)/F2:face(0)"));
    let dropped = got["dropped"].as_array().unwrap();
    assert_eq!(dropped.len(), 1, "{got}");
    assert!(
        dropped[0]
            .as_str()
            .unwrap()
            .starts_with("face F2:face(1): "),
        "{got}"
    );
    // The followed edge is the edge of the plate's top front still: 80 long.
    let followed = run(
        &mut s,
        "select.query",
        json!({"kind": "edge", "where": {"name": now[1].2, "length": 80}}),
    );
    assert_eq!(followed["count"], 1, "{followed}");

    // Suppressed, the boolean passes the plate on unchanged and leaves
    // the drill alone: the plate's faces are followed into its body as
    // before, the drill's are the drill's again (the bottom stays gone:
    // it dropped out).
    run(&mut s, "feature.suppress", json!({"feature": "Cut"}));
    let back = items(&run(&mut s, "select.get", json!({})));
    assert_eq!(back[0], item("face", "F3", "F3:copy(0)/F1:face(1)"));
    assert_eq!(back[1].1, "F3");
    assert_eq!(back[2], item("body", "F3", "Cut"));
    assert_eq!(back[3], item("face", "F2", "F2:face(0)"));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(items(&run(&mut s, "select.get", json!({}))), now);

    // A click on the followed face toggles the entry it follows from.
    run(
        &mut s,
        "select.set",
        json!({"names": ["F3:copy(0)/F1:face(1)"], "mode": "toggle"}),
    );
    assert_eq!(run(&mut s, "select.get", json!({}))["count"], 3);
}

/// A face of a body a pattern takes is followed to the first instance
/// (the body where it was); a body to the pattern's body.
#[test]
fn a_selection_follows_into_a_pattern_to_the_first_instance() {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Fin", "size": [2, 10, 10]}),
    );
    run(
        &mut s,
        "select.set",
        json!({"names": ["F1:face(1)", "Fin"]}),
    );
    run(
        &mut s,
        "feature.pattern",
        json!({"name": "Fins", "source": "Fin", "count": 3, "step": [5, 0, 0]}),
    );
    let got = items(&run(&mut s, "select.get", json!({})));
    assert_eq!(
        got,
        [
            item("face", "F2", "F2:copy(0)/F1:face(1)"),
            item("body", "F2", "Fins")
        ]
    );
    // It is said: the face is the first of three copies, never a silent
    // narrowing.
    let got = run(&mut s, "select.get", json!({}));
    let followed = got["followed"].as_array().unwrap();
    assert_eq!(followed.len(), 2, "{got}");
    assert_eq!(followed[0]["name"], "F2:copy(0)/F1:face(1)");
    assert_eq!(followed[0]["from"]["name"], "F1:face(1)");
    assert_eq!(followed[0]["copies"], 3);
    assert_eq!(followed[0]["pieces"], 1);
}

/// What a suppressed or rolled-back feature hides stays selected, hidden
/// and said, and comes back in its place when the feature does.
#[test]
fn hidden_items_come_back_in_their_place() {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "variable.set",
        json!({"name": "w", "expr": "10 mm"}),
    );
    for (name, x) in [("A", 0), ("B", 20), ("C", 40)] {
        run(
            &mut s,
            "feature.block",
            json!({"name": name, "size": ["w", 10, 10], "position": [x, 0, 0]}),
        );
    }
    run(
        &mut s,
        "select.set",
        json!({"names": ["A", "F2:face(1)", "B", "C"]}),
    );
    let all = items(&run(&mut s, "select.get", json!({})));
    assert_eq!(all.len(), 4);

    run(&mut s, "feature.suppress", json!({"feature": "B"}));
    let got = run(&mut s, "select.get", json!({}));
    assert_eq!(items(&got), [all[0].clone(), all[3].clone()]);
    let hidden = got["hidden"].as_array().unwrap();
    assert_eq!(hidden.len(), 2, "{got}");
    assert_eq!(hidden[0]["name"], "F2:face(1)");
    assert_eq!(hidden[0]["why"], "`B` is suppressed");
    assert_eq!(hidden[1]["kind"], "body");
    assert!(got["dropped"].as_array().unwrap().is_empty());
    assert_eq!(got["summary"], "2 bodies");
    run(
        &mut s,
        "feature.suppress",
        json!({"feature": "B", "suppressed": false}),
    );
    assert_eq!(items(&run(&mut s, "select.get", json!({}))), all);

    run(&mut s, "history.rollback", json!({"feature": "A"}));
    let got = run(&mut s, "select.get", json!({}));
    assert_eq!(items(&got), [all[0].clone()]);
    assert_eq!(got["hidden"][2]["why"], "`C` is rolled back", "{got}");
    run(&mut s, "history.rollback", json!({"feature": null}));
    assert_eq!(items(&run(&mut s, "select.get", json!({}))), all);

    // A failed feature hides its items too, until it builds again.
    run(&mut s, "variable.set", json!({"name": "w", "expr": "0 mm"}));
    let got = run(&mut s, "select.get", json!({}));
    assert!(
        got["hidden"]
            .as_array()
            .unwrap()
            .iter()
            .all(|h| h["why"].as_str().unwrap().contains("failed")),
        "{got}"
    );
    run(
        &mut s,
        "variable.set",
        json!({"name": "w", "expr": "10 mm"}),
    );
    assert_eq!(items(&run(&mut s, "select.get", json!({}))), all);

    // Clearing takes the hidden ones too; a deleted feature's items drop.
    run(&mut s, "feature.suppress", json!({"feature": "C"}));
    run(&mut s, "select.set", json!({"mode": "clear"}));
    run(
        &mut s,
        "feature.suppress",
        json!({"feature": "C", "suppressed": false}),
    );
    assert_eq!(run(&mut s, "select.get", json!({}))["count"], 0);
    run(&mut s, "select.set", json!({"names": ["C"]}));
    run(&mut s, "feature.delete", json!({"feature": "C"}));
    let got = run(&mut s, "select.get", json!({}));
    assert_eq!(got["count"], 0);
    assert_eq!(got["dropped"].as_array().unwrap().len(), 1);
}

// ------------------------------------------------------------- context

/// "Make these holes 6.5 mm" in one step: a hole's context says its radius
/// and axis, the cylinder parameter that sets it with the edit to make,
/// and the pattern instance it is a copy of.
#[test]
fn a_holes_context_says_what_drives_it() {
    let mut s = plate(4);
    let found = run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"hole": true, "radius": "3 mm"}, "context": true}),
    );
    assert_eq!(found["count"], 4);
    for (k, hole) in found["items"].as_array().unwrap().iter().enumerate() {
        assert_eq!(hole["surface"]["type"], "cylinder", "{hole}");
        assert_eq!(hole["surface"]["radius"], 3.0);
        assert_eq!(hole["surface"]["axis"], json!([0.0, 0.0, 1.0]));
        assert_eq!(hole["surface"]["hole"], true);
        let driver = &hole["drivers"][0];
        assert_eq!(driver["feature"], "F2");
        assert_eq!(driver["param"], "radius");
        assert_eq!(driver["expr"], "3 mm");
        assert_eq!(driver["edit"]["command"], "feature.edit");
        // The edit changes the four walls alike: it says so.
        assert_eq!(driver["affects"], 4, "{driver}");
        assert_eq!(driver["alike"], 4);
        assert_eq!(driver["shared"], true);
        assert_eq!(
            hole["copyOf"],
            json!([{"feature": "F3", "name": "Drills", "type": "pattern", "instance": k}]),
            "{hole}"
        );
        assert_eq!(hole["origin"]["type"], "cylinder");
    }
    // Without context the items are as before.
    let plain = run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"hole": true, "radius": "3 mm"}}),
    );
    assert!(plain["items"][0].get("drivers").is_none());

    // The edit the context gives, with the new value.
    let mut edit = found["items"][0]["drivers"][0]["edit"]["params"].clone();
    edit["radius"] = json!("6.5 mm / 2");
    run(&mut s, "feature.edit", edit);
    let made = run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"hole": true, "radius": "3.25 mm"}}),
    );
    assert_eq!(made["count"], 4);

    // select.get gives the same context for what is selected: the top of
    // the plate is set by the block's height (size index 2).
    run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"normal": {"along": "z"}}, "then": "replace"}),
    );
    let got = run(&mut s, "select.get", json!({}));
    let top = &got["items"][0];
    assert_eq!(top["surface"]["type"], "plane");
    assert_eq!(top["surface"]["normal"], json!([0.0, 0.0, 1.0]));
    assert_eq!(top["drivers"][0]["param"], "size");
    assert_eq!(top["drivers"][0]["index"], 2);
    assert_eq!(top["drivers"][0]["expr"], "8");
    assert_eq!(top["drivers"][0]["affects"], 6, "the plate's six faces");
    assert_eq!(top["drivers"][0]["shared"], false);
    assert_eq!(top["body"], "Part");
    let bare = run(&mut s, "select.get", json!({"context": false}));
    assert!(bare["items"][0].get("surface").is_none());
}

/// A hole cut by extruding a sketch circle is driven by the circle's
/// diameter constraint; a circle with none but equal to one is driven by
/// that one, through the equal constraint.
#[test]
fn a_sketched_holes_context_names_its_constraint() {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": [80, 50, 8]}),
    );
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "name": "Holes"}),
    );
    let first = run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Holes", "center": [20, 25], "radius": 4}),
    );
    let second = run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Holes", "center": [60, 25], "radius": 3}),
    );
    let (a, b) = (first["entity"].clone(), second["entity"].clone());
    let diameter = run(
        &mut s,
        "sketch.constrain",
        json!({"sketch": "Holes", "type": "diameter", "entities": [a], "value": "8 mm"}),
    );
    run(
        &mut s,
        "sketch.constrain",
        json!({"sketch": "Holes", "type": "equal", "entities": [a, b]}),
    );
    run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Holes", "distance": 30, "symmetric": true, "cut": "Plate"}),
    );
    let found = run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"hole": true}, "context": true}),
    );
    assert_eq!(found["count"], 2, "{found}");
    for hole in found["items"].as_array().unwrap() {
        assert_eq!(hole["surface"]["radius"], 4.0, "{hole}");
        let driver = &hole["drivers"][0];
        assert_eq!(driver["sketch"], "F2", "{hole}");
        assert_eq!(driver["kind"], "diameter");
        assert_eq!(driver["constraint"], diameter["constraint"]);
        assert_eq!(driver["expr"], "8 mm");
        assert_eq!(driver["edit"]["command"], "sketch.edit");
        // The constraint sizes both circles (one through equal): two walls.
        assert_eq!(driver["affects"], 2, "{driver}");
        assert_eq!(driver["shared"], true);
        assert_eq!(driver["curves"].as_array().unwrap().len(), 2);
    }
    let vias: Vec<&Value> = found["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|hole| &hole["drivers"][0]["via"])
        .collect();
    assert!(
        vias.contains(&&Value::Null) && vias.contains(&&a),
        "{found}"
    );
    // The edit it gives makes both 6.5 mm.
    let mut edit = found["items"][0]["drivers"][0]["edit"]["params"].clone();
    edit["value"] = json!("6.5 mm");
    run(&mut s, "sketch.edit", edit);
    let made = run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"hole": true, "radius": "3.25 mm"}}),
    );
    assert_eq!(made["count"], 2);
}

// ------------------------------------------------------------- bugs

/// The flat bottom of an extrusion with a spline edge is inside a box
/// round it (2026-10-21: the housing's bottom was not, because its box
/// was the hull of the whole spline's control points). The spline is the
/// quadratic Bézier from (40, 30) over (20, 50) to (0, 30): its top is at
/// y = 40 exactly, its control points reach 50.
#[test]
fn a_flat_face_with_a_spline_edge_is_inside_a_box_round_it() {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "sketch.create", json!({"plane": "xy"}));
    let bottom = run(
        &mut s,
        "sketch.line",
        json!({"sketch": "F1", "from": [0, 0], "to": [40, 0]}),
    );
    let right = run(
        &mut s,
        "sketch.line",
        json!({"sketch": "F1", "from": bottom["end"], "to": [40, 30]}),
    );
    let arc = run(
        &mut s,
        "sketch.spline",
        json!({"sketch": "F1", "control": [right["end"], [20, 50], [0, 30]], "degree": 2}),
    );
    run(
        &mut s,
        "sketch.line",
        json!({"sketch": "F1", "from": arc["end"], "to": bottom["start"]}),
    );
    run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "F1", "distance": 10}),
    );
    let within = |s: &mut Session, top: f64| {
        run(
            s,
            "select.query",
            json!({"kind": "face", "where": {"surface": "plane", "inside": [[-0.5, -0.5, -0.1], [40.5, top, 0.1]]}}),
        )["count"]
            .clone()
    };
    assert_eq!(within(&mut s, 40.5), 1, "the bottom, whose top is at 40");
    assert_eq!(within(&mut s, 39.5), 0, "the arc reaches 40");
    let low = run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"normal": {"along": "-z"}, "below": {"y": 40.5}}}),
    );
    assert_eq!(low["count"], 1, "{low}");
    // The spline's own edges too: with the straight edges up the corners
    // at y = 30 below 40.5, without them below 39.5.
    let edges = |s: &mut Session, top: f64| {
        names(&run(
            s,
            "select.query",
            json!({"kind": "edge", "where": {"inside": [[-0.5, 29.5, -0.1], [40.5, top, 10.1]]}}),
        ))
    };
    assert_eq!(
        edges(&mut s, 40.5),
        [
            "F2:startEdge(e8)",
            "F2:endEdge(e8)",
            "F2:lateralEdge(e5,e8)",
            "F2:lateralEdge(e8,e9)"
        ]
    );
    assert_eq!(
        edges(&mut s, 39.5),
        ["F2:lateralEdge(e5,e8)", "F2:lateralEdge(e8,e9)"]
    );
}

/// `file.close` closes the document it is given (2026-10-21: `document`
/// was refused as an unknown param), and keeps the active one active.
#[test]
fn file_close_closes_the_document_it_is_given() {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "First", "size": [1, 1, 1]}),
    );
    run(&mut s, "file.new", json!({}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Second", "size": [1, 1, 1]}),
    );
    let message = refused(&mut s, "file.close", json!({"document": 0}));
    assert!(message.contains("unsaved"), "{message}");
    for bad in [
        json!({"document": 2}),
        json!({"document": -1}),
        json!({"document": "0"}),
    ] {
        assert!(s.execute("file.close", &bad).is_err(), "{bad}");
    }
    let closed = run(
        &mut s,
        "file.close",
        json!({"document": 0, "discard": true}),
    );
    assert_eq!(closed, json!({"open": 1, "active": 0}));
    let inspect = run(&mut s, "document.inspect", json!({"measures": false}));
    assert_eq!(inspect["features"][0]["name"], "Second");
    run(&mut s, "file.close", json!({"discard": true}));
    assert!(s.execute("file.close", &json!({})).is_err());
}

/// `copyOf` through a pattern of a pattern and a mirror of a pattern: the
/// way of a name is walked through each, instance 0 included where the
/// short name leaves it out (121ea40).
#[test]
fn copy_of_walks_patterns_of_patterns_and_mirrored_patterns() {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Seed", "size": [10, 10, 10]}),
    );
    run(
        &mut s,
        "feature.pattern",
        json!({"name": "Row", "source": "Seed", "count": 3, "step": [20, 0, 0]}),
    );
    run(
        &mut s,
        "feature.pattern",
        json!({"name": "Grid", "source": "Row", "count": 2, "step": [0, 20, 0]}),
    );
    let tops = |s: &mut Session, filter: Value| {
        let mut all = vec![json!({"normal": {"along": "z"}})];
        all.push(filter);
        run(s, "select.query", json!({"kind": "face", "where": all}))
    };
    assert_eq!(tops(&mut s, json!({"copyOf": "Grid"}))["count"], 6);
    assert_eq!(tops(&mut s, json!({"copyOf": "Row"}))["count"], 6);
    for k in 0..3 {
        let row = tops(&mut s, json!({"copyOf": "Row", "instance": k}));
        assert_eq!(row["count"], 2, "Row instance {k}: {row}");
        // Instance k of the row lies at x = 20 k, in both rows of the grid.
        let x = 20.0 * f64::from(k);
        let there = tops(
            &mut s,
            json!({"copyOf": "Row", "instance": k, "inside": [[x - 0.1, -1, 9], [x + 10.1, 31, 11]]}),
        );
        assert_eq!(there["count"], 2, "Row instance {k} at x = {x}");
    }
    for k in 0..2 {
        let y = 20.0 * f64::from(k);
        let grid = tops(
            &mut s,
            json!({"copyOf": "Grid", "instance": k, "inside": [[-1, y - 0.1, 9], [51, y + 10.1, 11]]}),
        );
        assert_eq!(grid["count"], 3, "Grid instance {k}: {grid}");
    }
    let one = tops(
        &mut s,
        json!({"all": [{"copyOf": "Grid", "instance": 1}, {"copyOf": "Row", "instance": 0}]}),
    );
    assert_eq!(one["count"], 1, "{one}");
    let one = &one["items"][0];
    assert_eq!(one["feature"], "F3");

    // A mirror of the grid kept beside it, across x = -5.
    run(
        &mut s,
        "feature.mirror",
        json!({"name": "Mirrored", "source": "Grid", "normal": [1, 0, 0], "point": [-5, 0, 0], "keep": true}),
    );
    assert_eq!(tops(&mut s, json!({"copyOf": "Mirrored"}))["count"], 6);
    assert_eq!(
        tops(&mut s, json!({"in": "Mirrored", "copyOf": "Row"}))["count"],
        6
    );
    let mirrored = tops(
        &mut s,
        json!({"in": "Mirrored", "copyOf": "Row", "instance": 2}),
    );
    assert_eq!(mirrored["count"], 2, "{mirrored}");
    let far = tops(
        &mut s,
        json!({"in": "Mirrored", "copyOf": "Row", "instance": 2, "inside": [[-60.1, -1, 9], [-49.9, 31, 11]]}),
    );
    assert_eq!(
        far["count"], 2,
        "instance 2 mirrored to x = -60 .. -50: {far}"
    );
}

// ------------------------------------------------------ review fixes

/// Every edge of a block of three different sides, turned, is driven by
/// the size along which it runs: its length is that size (the review's
/// probe: edge(0) of a 10 × 20 × 30 block runs along y but was given z).
/// A vertex has no driver.
#[test]
fn every_edge_of_a_turned_block_is_driven_by_the_size_it_runs_along() {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Brick", "size": [10, 20, 30], "axis": [1, 1, 0], "angle": "30 deg"}),
    );
    let edges = run(
        &mut s,
        "select.query",
        json!({"kind": "edge", "context": true}),
    );
    assert_eq!(edges["count"], 12, "{edges}");
    let sizes = [10.0, 20.0, 30.0];
    for edge in edges["items"].as_array().unwrap() {
        let length = edge["curve"]["length"].as_f64().unwrap();
        let driver = &edge["drivers"][0];
        assert_eq!(driver["param"], "size", "{edge}");
        let index = usize::try_from(driver["index"].as_u64().unwrap()).unwrap();
        assert!(
            (length - sizes[index]).abs() < 1e-9,
            "{}: {length} long, driven by size {index}",
            edge["name"]
        );
        assert_eq!(driver["affects"], 6);
    }
    let vertices = run(
        &mut s,
        "select.query",
        json!({"kind": "vertex", "context": true}),
    );
    assert_eq!(vertices["count"], 8);
    for vertex in vertices["items"].as_array().unwrap() {
        assert_eq!(vertex["drivers"], json!([]), "{vertex}");
    }
}

/// One bolt hole of the showcase housing is driven by the radius of the
/// cylinder all six were drilled with (a pattern of three, mirrored): the
/// driver says its edit changes six walls alike.
#[test]
fn a_shared_driver_says_how_many_faces_its_edit_changes() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/showcase/housing.3dcraft"
    );
    let mut s = Session::new(Workspace::unrestricted());
    run(&mut s, "file.open", json!({"path": path}));
    let hole = run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"name": "F23:copy(0)/F22:copy(1,1)/F19:face(0)"}, "context": true}),
    );
    let driver = &hole["items"][0]["drivers"][0];
    assert_eq!(driver["feature"], "F19", "{hole}");
    assert_eq!(driver["param"], "radius");
    assert_eq!(driver["alike"], 6, "{driver}");
    assert_eq!(driver["shared"], true);
    // This hole is one of the right-hand row (the tool of DrilledRight),
    // not of the mirrored left-hand one: the way of its name passes the
    // pattern only.
    assert_eq!(
        hole["items"][0]["copyOf"],
        json!([{"feature": "F20", "name": "HolesRight", "type": "pattern", "instance": 1}])
    );
    let mirrored = run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"hole": true, "copyOf": "HolesLeft"}, "context": true}),
    );
    assert_eq!(mirrored["count"], 3);
    let chain = &mirrored["items"][0]["copyOf"];
    assert_eq!(chain[0]["type"], "mirror", "{chain}");
    assert_eq!(chain[1]["type"], "pattern", "{chain}");
}

/// A face whose body a split cuts in pieces is followed into every piece
/// that kept some of it, and the selection says so; an edge a cut parts
/// in two is both halves (the review's probe: only the half in Split1
/// was kept, and nothing was said).
#[test]
fn a_selection_follows_into_every_piece_of_a_split() {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Bar", "size": [30, 10, 10]}),
    );
    let edge = run(
        &mut s,
        "select.query",
        json!({"kind": "edge", "where": {"length": 30, "above": {"z": 10}, "below": {"y": 0}}}),
    );
    let edge = edge["items"][0]["name"].as_str().unwrap().to_string();
    run(&mut s, "select.set", json!({"names": ["F1:face(1)", edge]}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Slab", "size": [2, 20, 20], "position": [14, -5, -5]}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Bar", "tool": "Slab", "name": "Cut"}),
    );
    // The edge is cut in two: both halves, 14 long each.
    let got = run(&mut s, "select.get", json!({}));
    let kinds: Vec<&str> = got["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["face", "edge", "edge"], "{got}");
    for half in &got["items"].as_array().unwrap()[1..] {
        assert_eq!(half["curve"]["length"], 14.0, "{half}");
    }
    run(&mut s, "body.split", json!({"source": "Cut"}));
    let got = run(&mut s, "select.get", json!({}));
    let items = got["items"].as_array().unwrap();
    assert_eq!(items.len(), 4, "a top and an edge in each piece: {got}");
    let pieces: BTreeSet<&str> = items
        .iter()
        .map(|i| i["feature"].as_str().unwrap())
        .collect();
    assert_eq!(pieces.len(), 2, "{got}");
    assert_eq!(items.iter().filter(|i| i["kind"] == "face").count(), 2);
    assert!(got["dropped"].as_array().unwrap().is_empty());
    let followed = got["followed"].as_array().unwrap();
    assert!(
        followed
            .iter()
            .filter(|f| f["kind"] == "face")
            .all(|f| f["pieces"] == 2 && f["from"]["name"] == "F1:face(1)"),
        "{got}"
    );
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(run(&mut s, "select.get", json!({}))["count"], 3);
}

/// A driver whose value is a formula says so; one that is a variable
/// points at the variable, so an agent's edit keeps the design's formulas.
#[test]
fn a_driver_that_is_a_formula_points_at_its_variable() {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "variable.set",
        json!({"name": "bolt_r", "expr": "3 mm"}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Pin", "radius": "bolt_r", "height": 10}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Peg", "radius": "bolt_r * 2", "height": 10, "position": [20, 0, 0]}),
    );
    let walls = run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"surface": "cylinder"}, "context": true}),
    );
    let pin = &walls["items"][0]["drivers"][0];
    assert_eq!(pin["formula"], true, "{pin}");
    assert_eq!(pin["variable"], "bolt_r");
    assert_eq!(
        pin["edit"],
        json!({"command": "variable.set", "params": {"name": "bolt_r", "expr": "3 mm"}})
    );
    let peg = &walls["items"][1]["drivers"][0];
    assert_eq!(peg["formula"], true, "{peg}");
    assert!(peg.get("variable").is_none());
    assert_eq!(peg["edit"]["command"], "feature.edit");
}
