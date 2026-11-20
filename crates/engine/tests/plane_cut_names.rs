//! The names a plane cut gives, and what an edit does to them
//! (docs/design/plane-cut.md › *Names*; the naming suite of
//! docs/testing.md extended). A face a plane leaves is named by the
//! plane's id, so a fillet, a chamfer, a sketch and a datum placed by it
//! stay where they are when a plane is moved, when one is put between the
//! others, when the list is put in another order and when the target is
//! edited upstream; when the plane is deleted or no longer meets the body
//! they fail with the name, and never land on another face. Every edge
//! and vertex of a cut body has a name no other has.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{
    assert_close, assert_solid, body, built, failed, failure, feature, names_of, new_document,
    refused, run, session,
};
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::collections::BTreeSet;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

const CUT: &str = "feature.planeCut";

/// A plane of id `id` (0: none given) through `at` (times `k`) square to
/// `normal`.
fn plane(k: f64, id: u64, at: [f64; 3], normal: [f64; 3]) -> Value {
    let mut plane = json!({"point": at.map(|x| x * k), "normal": normal});
    if id != 0 {
        plane["id"] = json!(id);
    }
    plane
}

/// The south and the north slope of a roof with eaves at `eaves` and its
/// ridge 8 higher over the middle of a 30 deep block: planes 1 and 2.
fn south(k: f64, eaves: f64) -> Value {
    plane(k, 1, [0.0, 0.0, eaves], [0.0, -8.0, 15.0])
}

fn north(k: f64, eaves: f64) -> Value {
    plane(k, 2, [40.0, 30.0, eaves], [0.0, 8.0, 15.0])
}

/// A third plane, away from the slopes: it takes the bottom corner at
/// (40, 30, 0) off, a tetrahedron with legs of 4. Given with the id
/// `id`, or without one (0).
fn corner(k: f64, id: u64) -> Value {
    plane(k, id, [40.0, 26.0, 0.0], [1.0, 1.0, -1.0])
}

/// A block 40 × 30 × 20 (`Part`, F1) under a roof of two planes (`Roof`,
/// F2), eaves at 8 and the ridge at 16, with what is placed by its names:
/// `Ridge` (F3) rounds the edge between the two slopes, `Eave` (F4)
/// chamfers the edge between the south slope and the front wall, given
/// by that edge's own name, `OnNorth` (F5) is a sketch on the north
/// slope and `Over` (F6) a datum plane 1 above it.
fn roofed(s: &mut Session, k: f64) {
    built(
        s,
        "feature.block",
        json!({"size": [40.0 * k, 30.0 * k, 20.0 * k], "name": "Part"}),
    );
    built(
        s,
        CUT,
        json!({"target": "Part", "planes": [south(k, 8.0), north(k, 8.0)], "name": "Roof"}),
    );
    built(
        s,
        "feature.fillet",
        json!({"edges": [["Roof:cut(1)", "Roof:cut(2)"]], "radius": 2.0 * k, "name": "Ridge"}),
    );
    built(
        s,
        "feature.chamfer",
        json!({
            "edges": ["F2:between[F2:copy(0)/F1:face(2),F2:cut(1)]"],
            "distance": k,
            "name": "Eave",
        }),
    );
    built(
        s,
        "sketch.create",
        json!({"face": "Roof:cut(2)", "name": "OnNorth"}),
    );
    built(
        s,
        "datum.plane",
        json!({"from": "Roof:cut(2)", "distance": k, "name": "Over"}),
    );
}

const PLACED: [&str; 4] = ["Ridge", "Eave", "OnNorth", "Over"];

fn edit_planes(s: &mut Session, planes: Value) -> Value {
    run(
        s,
        "feature.edit",
        json!({"feature": "Roof", "planes": planes}),
    )
}

fn statuses(s: &mut Session) -> Vec<(String, String)> {
    PLACED
        .iter()
        .map(|name| {
            (
                (*name).to_string(),
                feature(s, name)["status"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[track_caller]
fn all_built(s: &mut Session, what: &str) {
    for (name, status) in statuses(s) {
        assert_eq!(status, "ok", "{name} after {what}: {}", feature(s, &name));
    }
    let check = run(s, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{what}: {check}");
}

fn vec3(value: &Value) -> [f64; 3] {
    let v = value.as_array().unwrap_or_else(|| panic!("{value}"));
    [
        v[0].as_f64().unwrap(),
        v[1].as_f64().unwrap(),
        v[2].as_f64().unwrap(),
    ]
}

/// Where the datum `Over` lies: its origin.
fn over(s: &mut Session) -> [f64; 3] {
    vec3(&feature(s, "Over")["datum"]["origin"])
}

/// The names of the faces of the finished body, `Eave` (F4): the roof's
/// faces as copies (`F4:copy(0)/F2:cut(1)`: one link for the way through
/// the round and the chamfer), the round's and the chamfer's own.
fn roof_faces(s: &mut Session) -> BTreeSet<String> {
    names_of(&body(s, "Eave")).into_iter().collect()
}

/// A plane moved, the topology the same: every name stays on its entity,
/// what is placed by them follows.
#[test]
fn a_plane_moved_keeps_every_name() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        roofed(&mut s, k);
        all_built(&mut s, "the roof");
        let faces = roof_faces(&mut s);
        for wanted in [
            "F4:copy(0)/F2:cut(1)",
            "F4:copy(0)/F2:cut(2)",
            "F4:copy(0)/F1:face(2)",
        ] {
            assert!(faces.contains(wanted), "{faces:?}");
        }
        // The top of the block is cut away.
        assert!(!faces.contains("F4:copy(0)/F1:face(1)"), "{faces:?}");
        let was = over(&mut s);
        // The north slope's eaves from 8 to 6: its plane 2 lower along z.
        edit_planes(&mut s, json!([south(k, 8.0), north(k, 6.0)]));
        all_built(&mut s, "the north slope lowered");
        assert_eq!(roof_faces(&mut s), faces);
        let now = over(&mut s);
        // The datum is the plane's own origin moved 1 along its normal:
        // it went down by 2 with the plane.
        assert!(
            (now[2] - (was[2] - 2.0 * k)).abs() <= 1e-9 * k,
            "{was:?} {now:?}"
        );
        assert!((now[0] - was[0]).abs() <= 1e-9 * k && (now[1] - was[1]).abs() <= 1e-9 * k);
    }
}

/// A plane put between the others, and the list in another order: every
/// other plane keeps its id, so every name stays and nothing placed by
/// them moves or fails.
#[test]
fn a_plane_added_in_the_middle_and_another_order_rename_nothing() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        roofed(&mut s, k);
        let faces = roof_faces(&mut s);
        let was = over(&mut s);
        let ridge = body(&mut s, "Eave")["volume"].as_f64().unwrap();
        // A plane given without an id, between the two: it is plane 3.
        let result = edit_planes(&mut s, json!([south(k, 8.0), corner(k, 0), north(k, 8.0)]));
        assert_eq!(result["status"], "ok", "{result}");
        assert_eq!(
            result["planes"],
            json!([
                {"id": 1, "cuts": true, "faces": 1},
                {"id": 3, "cuts": true, "faces": 1},
                {"id": 2, "cuts": true, "faces": 1},
            ])
        );
        all_built(&mut s, "a plane between the slopes");
        let hipped = roof_faces(&mut s);
        assert!(hipped.contains("F4:copy(0)/F2:cut(3)"), "{hipped:?}");
        // Every face that was there is there, under its name: the walls
        // and the bottom the new plane cuts are cut, not cut away.
        for name in &faces {
            assert!(hipped.contains(name), "{name} in {hipped:?}");
        }
        assert_eq!(hipped.len(), faces.len() + 1);
        assert_eq!(over(&mut s), was);
        // The corner it takes off is a tetrahedron with legs of 4.
        let cut = body(&mut s, "Eave")["volume"].as_f64().unwrap();
        assert!(
            (ridge - cut - 64.0 / 6.0 * k * k * k).abs() <= 1e-9 * ridge,
            "{cut} {ridge}"
        );
        // Another order: the same body under the same names.
        edit_planes(&mut s, json!([corner(k, 3), north(k, 8.0), south(k, 8.0)]));
        all_built(&mut s, "the planes in another order");
        assert_eq!(roof_faces(&mut s), hipped);
        assert_eq!(over(&mut s), was);
        assert_close(&body(&mut s, "Eave")["volume"], cut, "volume");
    }
}

/// A plane deleted: what was placed by its face fails with the name, the
/// rest stays, and no later plane takes the name. A plane that no longer
/// meets the body: the same, and moving it back repairs it.
#[test]
fn a_plane_deleted_or_off_the_body_fails_what_was_placed_by_it() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        roofed(&mut s, k);
        // The north slope deleted, another plane added: plane 3, never 2.
        let result = edit_planes(&mut s, json!([south(k, 8.0), corner(k, 0)]));
        assert_eq!(result["status"], "ok", "{result}");
        assert_eq!(result["planes"][1]["id"], 3, "{result}");
        for name in ["Ridge", "OnNorth", "Over"] {
            let reason = failure(&mut s, name);
            assert!(reason.contains("F2:cut(2)"), "{name}: {reason}");
        }
        // The chamfer of the eave is still built, on the roof as it is.
        assert_eq!(feature(&mut s, "Eave")["status"], "ok");
        let left = roof_faces(&mut s);
        assert!(!left.iter().any(|n| n.ends_with("F2:cut(2)")), "{left:?}");
        assert!(left.contains("F4:copy(0)/F2:cut(3)"), "{left:?}");
        // Undone, everything is as it was.
        run(&mut s, "edit.undo", json!({}));
        all_built(&mut s, "the undo");
        // The north slope lifted off the body: it cuts nothing, and what
        // was placed by its face fails by name; back, it is repaired.
        let result = edit_planes(
            &mut s,
            json!([
                south(k, 8.0),
                plane(k, 2, [40.0, 30.0, 100.0], [0.0, 8.0, 15.0])
            ]),
        );
        assert_eq!(
            result["planes"],
            json!([
                {"id": 1, "cuts": true, "faces": 1},
                {"id": 2, "cuts": false, "faces": 0},
            ]),
            "{result}"
        );
        for name in ["Ridge", "OnNorth", "Over"] {
            let reason = failure(&mut s, name);
            assert!(reason.contains("F2:cut(2)"), "{name}: {reason}");
        }
        edit_planes(&mut s, json!([south(k, 8.0), north(k, 8.0)]));
        all_built(&mut s, "the north slope back");
    }
}

/// The target edited upstream: the cut faces keep their names, and what
/// is placed by them stays.
#[test]
fn an_upstream_edit_of_the_target_keeps_the_names() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        roofed(&mut s, k);
        let faces = roof_faces(&mut s);
        let was = over(&mut s);
        // The block longer.
        run(
            &mut s,
            "feature.edit",
            json!({"feature": "Part", "size": [50.0 * k, 30.0 * k, 20.0 * k]}),
        );
        all_built(&mut s, "a longer block");
        assert_eq!(roof_faces(&mut s), faces);
        assert_eq!(over(&mut s), was);
    }
}

/// The names of the edges, and of the vertices, of the body of
/// `feature`, each held to be no other's.
#[track_caller]
fn distinct_names(s: &mut Session, feature: &str, prefix: &str) -> Value {
    let listed = run(s, "document.edges", json!({"feature": feature}));
    assert_eq!(listed["truncated"], false, "{listed}");
    for kind in ["edges", "vertices"] {
        let all: Vec<&str> = listed[kind]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap())
            .collect();
        let distinct: BTreeSet<&str> = all.iter().copied().collect();
        assert_eq!(all.len(), distinct.len(), "{kind}: {all:?}");
        assert!(all.iter().all(|n| n.starts_with(prefix)), "{all:?}");
    }
    listed
}

/// Every edge and every vertex of a cut body has a name no other has,
/// also where a plane cuts the body in two places and where planes meet
/// in a point.
#[test]
fn no_two_edges_and_no_two_vertices_share_a_name() {
    for k in SCALES {
        // A block with a slot through its top: two legs, and a slanted
        // plane across both.
        let mut s = session();
        new_document(&mut s, k);
        built(
            &mut s,
            "feature.block",
            json!({"size": [40.0 * k, 30.0 * k, 20.0 * k], "name": "Part"}),
        );
        built(
            &mut s,
            "feature.block",
            json!({"size": [10.0 * k, 32.0 * k, 12.0 * k], "position": [15.0 * k, -k, 10.0 * k], "name": "Slot"}),
        );
        built(
            &mut s,
            "feature.boolean",
            json!({"op": "subtract", "target": "Part", "tool": "Slot", "name": "Legs"}),
        );
        let result = built(
            &mut s,
            CUT,
            json!({
                "target": "Legs",
                "planes": [plane(k, 0, [0.0, 0.0, 16.0], [0.0, -1.0, 6.0])],
                "name": "Cut",
            }),
        );
        // The plane cuts both legs: two faces of one name.
        assert_eq!(
            result["planes"],
            json!([{"id": 1, "cuts": true, "faces": 2}]),
            "{result}"
        );
        let found = body(&mut s, "Cut");
        assert_eq!(found["valid"], true, "{found}");
        let names = names_of(&found);
        assert_eq!(names.iter().filter(|n| *n == "F4:cut(1)").count(), 2);
        distinct_names(&mut s, "Cut", "F4:");
        // A sketch lies on a face in two pieces: they are one plane.
        built(
            &mut s,
            "sketch.create",
            json!({"face": "Cut:cut(1)", "name": "OnBoth"}),
        );
        let check = run(&mut s, "document.check", json!({"deep": true}));
        assert_eq!(check["ok"], true, "{check}");

        // Three planes through one point given by the same numbers: a
        // three-sided point on a block, its tip one vertex.
        let mut s = session();
        new_document(&mut s, k);
        built(
            &mut s,
            "feature.block",
            json!({"size": [40.0 * k, 30.0 * k, 20.0 * k], "name": "Part"}),
        );
        let apex = [20.0, 15.0, 18.0];
        let result = built(
            &mut s,
            CUT,
            json!({
                "target": "Part",
                "planes": [
                    plane(k, 0, apex, [3.0, 0.0, 4.0]),
                    plane(k, 0, apex, [-3.0, 2.0, 4.0]),
                    plane(k, 0, apex, [-1.0, -3.0, 4.0]),
                ],
                "name": "Point",
            }),
        );
        for report in result["planes"].as_array().unwrap() {
            assert_eq!(report["faces"], 1, "{result}");
        }
        let listed = distinct_names(&mut s, "Point", "F2:");
        let tip = "F2:corner[F2:cut(1),F2:cut(2),F2:cut(3)]";
        let at = listed["vertices"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["name"] == tip)
            .unwrap_or_else(|| panic!("{tip} in {listed}"));
        for (got, want) in vec3(&at["point"]).into_iter().zip(apex) {
            assert!((got - want * k).abs() <= 1e-9 * k, "{at}");
        }
        let check = run(&mut s, "document.check", json!({"deep": true}));
        assert_eq!(check["ok"], true, "{check}");
    }
}

/// A plane in a face of the target: the piece it cuts and the face are
/// one face, under the face's name and answering to the plane's as an
/// alias, by which a sketch and a datum find it.
#[test]
fn a_piece_that_joins_a_face_of_the_target_answers_to_both_names() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        // A step: a block 40 × 30 × 10 with one of 20 × 30 × 10 on it.
        built(
            &mut s,
            "feature.block",
            json!({"size": [40.0 * k, 30.0 * k, 10.0 * k], "name": "Base"}),
        );
        built(
            &mut s,
            "feature.block",
            json!({"size": [20.0 * k, 30.0 * k, 10.0 * k], "position": [0.0, 0.0, 10.0 * k], "name": "Step"}),
        );
        built(
            &mut s,
            "feature.boolean",
            json!({"op": "union", "target": "Base", "tool": "Step", "name": "Stepped"}),
        );
        // Cut at the shoulder: the step goes, the top is one face.
        let result = built(
            &mut s,
            CUT,
            json!({"target": "Stepped", "planes": [plane(k, 0, [0.0, 0.0, 10.0], [0.0, 0.0, 1.0])], "name": "Level"}),
        );
        assert_eq!(
            result["planes"],
            json!([{"id": 1, "cuts": true, "faces": 1}])
        );
        let level = assert_solid(&mut s, "Level", 12_000.0, Some(3800.0), k);
        assert_eq!(level["faces"], 6, "{level}");
        let names = names_of(&level);
        assert!(
            names.contains(&"F4:copy(0)/F1:face(1)".to_string()),
            "{names:?}"
        );
        assert!(!names.contains(&"F4:cut(1)".to_string()), "{names:?}");
        let aliases = level["faceAliases"].to_string();
        assert!(aliases.contains("F4:cut(1)"), "{level}");
        // By the plane's name: the top, whose normal points up.
        let sketch = built(
            &mut s,
            "sketch.create",
            json!({"face": "Level:cut(1)", "name": "OnTop"}),
        );
        let normal = vec3(&sketch["normal"]);
        assert!((normal[2] - 1.0).abs() <= 1e-12, "{normal:?}");
        let datum = built(
            &mut s,
            "datum.plane",
            json!({"from": "Level:cut(1)", "distance": k, "name": "Above"}),
        );
        let origin = vec3(&datum["datum"]["origin"]);
        assert!((origin[2] - 11.0 * k).abs() <= 1e-9 * k, "{origin:?}");
        // The plane a little lower cuts a face of its own: the same name
        // is now the face itself, and what is placed by it follows.
        run(
            &mut s,
            "feature.edit",
            json!({"feature": "Level", "planes": [plane(k, 1, [0.0, 0.0, 9.0], [0.0, 0.0, 1.0])]}),
        );
        let lower = assert_solid(&mut s, "Level", 10_800.0, None, k);
        assert!(
            names_of(&lower).contains(&"F4:cut(1)".to_string()),
            "{lower}"
        );
        assert_eq!(feature(&mut s, "OnTop")["status"], "ok");
        let origin = vec3(&feature(&mut s, "Above")["datum"]["origin"]);
        assert!((origin[2] - 10.0 * k).abs() <= 1e-9 * k, "{origin:?}");
    }
}

/// An edge of a cut body by its own name is taken where every part of
/// the name is proven to stay with its entity: a name by faces with a
/// plane's id is; one that ends in a number by place is refused.
#[test]
fn an_edge_between_cut_faces_is_taken_by_its_own_name() {
    let k = 1.0;
    let mut s = session();
    new_document(&mut s, k);
    built(
        &mut s,
        "feature.block",
        json!({"size": [40.0, 30.0, 20.0], "name": "Part"}),
    );
    built(
        &mut s,
        CUT,
        json!({"target": "Part", "planes": [south(k, 8.0), north(k, 8.0)], "name": "Roof"}),
    );
    let before = body(&mut s, "Roof")["volume"].as_f64().unwrap();
    built(
        &mut s,
        "feature.fillet",
        json!({"edges": ["F2:between[F2:cut(1),F2:cut(2)]"], "radius": 2.0, "name": "Ridge"}),
    );
    let after = body(&mut s, "Ridge")["volume"].as_f64().unwrap();
    assert!(after < before, "{after} {before}");
    // A name with a number by place at its end is not taken.
    let said = refused(
        &mut s,
        "feature.chamfer",
        json!({"edges": ["F2:between[F2:cut(1),F2:cut(2)](0)"], "distance": 1.0}),
    );
    assert!(
        said.contains("F2:between[F2:cut(1),F2:cut(2)](0)"),
        "{said}"
    );
}

/// The review's (2026-11-20): the target pointed at another body. The
/// faces the planes cut keep their names, so the round of the ridge, the
/// sketch and the datum on the north slope stay where they were; the
/// chamfer, given by an edge whose name holds a face of the old target,
/// fails with that name; the old target is a body again.
#[test]
fn another_target_keeps_the_planes_names_and_fails_the_old_targets() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        roofed(&mut s, k);
        let was = over(&mut s);
        // A block like the first, 100 further along x, put before the roof
        // (F7, second in the history).
        built(
            &mut s,
            "feature.block",
            json!({"size": [40.0 * k, 30.0 * k, 20.0 * k], "position": [100.0 * k, 0.0, 0.0], "name": "Other"}),
        );
        run(
            &mut s,
            "feature.move",
            json!({"feature": "Other", "position": 1}),
        );
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Roof", "target": "Other"}),
        );
        for name in ["Ridge", "OnNorth", "Over"] {
            assert_eq!(feature(&mut s, name)["status"], "ok", "{name}");
        }
        assert_eq!(over(&mut s), was);
        let reason = failure(&mut s, "Eave");
        assert!(reason.contains("F2:copy(0)/F1:face(2)"), "{reason}");
        // The roof is the same shape on the other block.
        let faces = names_of(&body(&mut s, "Ridge"));
        for wanted in [
            "F3:copy(0)/F2:cut(1)",
            "F3:copy(0)/F2:cut(2)",
            "F3:copy(0)/F7:face(2)",
        ] {
            assert!(faces.iter().any(|n| n == wanted), "{wanted} in {faces:?}");
        }
        assert!(!faces.iter().any(|n| n.contains("F1:")), "{faces:?}");
        assert_close(
            &body(&mut s, "Part")["volume"],
            24_000.0 * k * k * k,
            "volume",
        );
    }
}

/// The review's (2026-11-20): a plane added that cuts a named face away
/// whole (y ≥ 16 leaves only the north slope). What was placed by the
/// south slope fails with its name; what lies on the north slope stays
/// where it was; the south slope's plane is reported as leaving no face.
#[test]
fn a_plane_that_cuts_a_named_face_away_fails_what_was_placed_by_it() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        roofed(&mut s, k);
        let was = over(&mut s);
        let result = edit_planes(
            &mut s,
            json!([
                south(k, 8.0),
                north(k, 8.0),
                plane(k, 0, [0.0, 16.0, 0.0], [0.0, -1.0, 0.0])
            ]),
        );
        assert_eq!(
            result["planes"],
            json!([
                {"id": 1, "cuts": false, "faces": 0},
                {"id": 2, "cuts": true, "faces": 1},
                {"id": 3, "cuts": true, "faces": 1},
            ]),
            "{result}"
        );
        for name in ["Ridge", "Eave"] {
            let reason = failure(&mut s, name);
            assert!(reason.contains("F2:cut(1)"), "{name}: {reason}");
        }
        assert_eq!(feature(&mut s, "OnNorth")["status"], "ok");
        assert_eq!(over(&mut s), was);
        // 40 × the profile from y = 16 to 30 under the north slope:
        // 40 (8 × 14 + 8/15 × 14² / 2).
        let roof = body(&mut s, "Roof");
        assert_close(
            &roof["volume"],
            40.0 * (112.0 + 8.0 / 15.0 * 98.0) * k * k * k,
            "volume",
        );
    }
}

/// The review's (2026-11-20): a gable across a block with a slot in its
/// top cuts both legs, so each plane leaves two faces of one name and
/// there are two ridges between them. Each ridge is named by the faces
/// near it, so a round takes one ridge by its own name; the pair of faces
/// takes both (the rule for a face in pieces); the bare name of the pair
/// is no edge's, and fails by name.
#[test]
fn two_ridges_between_the_same_planes_are_told_apart_by_the_faces_near_them() {
    use craft3d_math::Portable;
    // What a round of radius 2 takes off a ridge between slopes rising
    // 8 in 15, per unit of its length: r² (tan(φ/2) − φ/2) with φ/2 the
    // slope's angle.
    let rounded = 4.0 * (8.0 / 15.0 - (8.0f64 / 15.0).patan());
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        built(
            &mut s,
            "feature.block",
            json!({"size": [40.0 * k, 30.0 * k, 20.0 * k], "name": "Part"}),
        );
        built(
            &mut s,
            "feature.block",
            json!({"size": [10.0 * k, 32.0 * k, 22.0 * k], "position": [15.0 * k, -k, 10.0 * k], "name": "Slot"}),
        );
        built(
            &mut s,
            "feature.boolean",
            json!({"op": "subtract", "target": "Part", "tool": "Slot", "name": "Legs"}),
        );
        let result = built(
            &mut s,
            CUT,
            json!({"target": "Legs", "planes": [south(k, 12.0), north(k, 12.0)], "name": "Roof"}),
        );
        assert_eq!(
            result["planes"],
            json!([
                {"id": 1, "cuts": true, "faces": 2},
                {"id": 2, "cuts": true, "faces": 2},
            ]),
            "{result}"
        );
        // 19,200 less the slot through the roof's profile above 10:
        // 10 × (30 × 2 + 120).
        let roofed = 17_400.0;
        assert_solid(&mut s, "Roof", roofed, None, k);
        let listed = distinct_names(&mut s, "Roof", "F4:");
        let bare = "F4:between[F4:cut(1),F4:cut(2)]";
        let ridges: Vec<String> = listed["edges"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap().to_string())
            .filter(|n| n.starts_with("F4:between[F4:cut(1),F4:cut(2)|"))
            .collect();
        assert_eq!(ridges.len(), 2, "{listed}");
        // Both by the pair: two ridges of 15.
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": [["Roof:cut(1)", "Roof:cut(2)"]], "radius": 2.0 * k, "name": "Both"}),
        );
        assert_solid(&mut s, "Both", roofed - 30.0 * rounded, None, k);
        run(&mut s, "edit.undo", json!({}));
        // One by its own name.
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": [ridges[0]], "radius": 2.0 * k, "name": "One"}),
        );
        assert_solid(&mut s, "One", roofed - 15.0 * rounded, None, k);
        run(&mut s, "edit.undo", json!({}));
        // The pair's bare name is no edge's.
        let reason = failed(
            &mut s,
            "feature.fillet",
            json!({"edges": [bare], "radius": 2.0 * k, "name": "Bare"}),
        );
        assert!(reason.contains(bare), "{reason}");
    }
}
