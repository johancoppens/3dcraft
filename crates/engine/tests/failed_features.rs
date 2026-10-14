//! A feature that fails (M8.5): every reference a feature can hold is
//! broken upstream in every way a user can break it, and what must hold
//! then is asserted the same way for all of them.
//!
//! - The feature fails with a reason that starts with the parameter, names
//!   the feature it referred to and the name it lost.
//! - What is made from the failed feature fails naming it ("fix it
//!   first"), not with an error of its own.
//! - Every other feature is exactly what it is in the same document
//!   without the failed features (`independent_features_are_untouched`).
//! - `document.inspect` and `document.check` say so; undo and redo restore
//!   exactly; regeneration edit by edit equals a full one (`run`).
//! - Pointing the reference at another entity gives the document that
//!   building it that way from the start gives.
//!
//! The wording of the reasons has no oracle but this file: those
//! assertions are regression tests, not proofs (docs/testing.md). That an
//! independent feature is untouched, that a repair equals a fresh build and
//! that undo restores are compared with a second computation.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Access, Session, Workspace};
use craft3d_testkit::parallel::map_parallel;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    let result = session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"));
    // Built up edit by edit, the document is what regenerating it from
    // nothing gives (docs/testing.md: incremental = full regeneration).
    assert_ne!(
        session.regeneration_matches_a_full_one(),
        Some(false),
        "after {id} {params}"
    );
    result
}

/// A command on a copy that is made to compare with: it has to run, and
/// is not itself held to a full regeneration.
fn only(session: &mut Session, id: &str, params: Value) {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"));
}

/// A command that adds or edits a feature, which has to regenerate.
fn built(session: &mut Session, id: &str, params: Value) -> Value {
    let result = run(session, id, params.clone());
    assert_eq!(result["status"], "ok", "{id} {params}: {result}");
    result
}

fn refused(session: &mut Session, id: &str, params: Value) -> String {
    match session.execute(id, &params) {
        Ok(value) => panic!("{id} {params} should be refused, returned {value}"),
        Err(e) => {
            assert!(!e.message().contains("internal error"), "{id}: {e}");
            e.to_string()
        }
    }
}

fn inspect(session: &mut Session) -> Value {
    run(session, "document.inspect", json!({}))
}

/// What a document is, without what only says how it got there (the undo
/// and redo labels) or where it is kept. The bounding boxes are left out
/// too: `document.inspect` takes them from a mesh whose tolerance follows
/// the size of the whole model, so the box of a curved body changes in
/// the last digits when another body comes or goes. Volume, area,
/// centroid, counts and names are of the body itself.
fn state(session: &mut Session) -> Value {
    let mut document = inspect(session);
    let map = document.as_object_mut().unwrap();
    for key in ["history", "dirty", "path", "document"] {
        map.remove(key);
    }
    for body in map["bodies"].as_array_mut().unwrap() {
        let body = body.as_object_mut().unwrap();
        body.remove("bbox");
        body.remove("bboxTolerance");
    }
    document
}

fn feature<'a>(document: &'a Value, name: &str) -> &'a Value {
    document["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == name || f["id"] == name)
        .unwrap_or_else(|| panic!("no feature {name}"))
}

/// The body of a feature, when it has one that is its own.
fn body_of<'a>(document: &'a Value, name: &str) -> Option<&'a Value> {
    document["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == name)
}

/// The id of a feature, as `F3`.
fn id_of(session: &mut Session, name: &str) -> String {
    let document = inspect(session);
    feature(&document, name)["id"].as_str().unwrap().to_string()
}

/// (status, error) of a feature.
fn status(document: &Value, name: &str) -> (String, String) {
    let entry = feature(document, name);
    (
        entry["status"].as_str().unwrap().to_string(),
        entry["error"].as_str().unwrap_or_default().to_string(),
    )
}

/// The full name of the face of `feature`'s body whose name has `part` in
/// it (`endCap`, `side(e5)`, `face(0)`).
fn face(session: &mut Session, feature: &str, part: &str) -> String {
    let document = inspect(session);
    let body = document["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == feature)
        .unwrap_or_else(|| panic!("no body {feature}"));
    body["faceNames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap())
        .find(|n| n.contains(&format!(":{part}")))
        .unwrap_or_else(|| panic!("{feature} has no face {part}: {body}"))
        .to_string()
}

/// The name of the wall of the hole of a frame of [`base`]: the side of
/// its circle, the last curve of its sketch.
fn hole_wall(session: &mut Session, feature: &str) -> String {
    let document = inspect(session);
    let body = document["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == feature)
        .unwrap_or_else(|| panic!("no body {feature}"));
    body["faceNames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap())
        .rfind(|n| n.contains(":side("))
        .unwrap_or_else(|| panic!("{feature} has no side: {body}"))
        .to_string()
}

/// The name of the vertex of `feature`'s body at `at`.
fn vertex(session: &mut Session, feature: &str, at: [f64; 3]) -> String {
    let offered = run(session, "datum.references", json!({"feature": feature}));
    offered["vertices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| (0..3).all(|i| (v["at"][i].as_f64().unwrap() - at[i]).abs() < 1e-9))
        .unwrap_or_else(|| panic!("{feature} has no vertex at {at:?}: {offered}"))["name"]
        .as_str()
        .unwrap()
        .to_string()
}

/// The model every case starts from: two of everything a feature can
/// refer to (the one a case points at and the one it is repaired with),
/// each with a variable of its own, and bodies nothing refers to.
///
/// Plates are 40 x 20 x 5, the second 30 above the first; rods stand
/// through the first plate; the bar stands through both.
fn base() -> Session {
    static BASE: OnceLock<Session> = OnceLock::new();
    BASE.get_or_init(build_base).clone()
}

fn build_base() -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({}));
    for (name, expr) in [
        ("h", "5 mm"),
        ("h2", "5 mm"),
        ("r", "3 mm"),
        ("r2", "3 mm"),
        ("w", "4 mm"),
        ("lift", "12 mm"),
        ("lift2", "20 mm"),
        ("reach", "30 mm"),
        ("reach2", "30 mm"),
        ("lay", "1 mm"),
        ("lay2", "1 mm"),
        ("dx", "1 mm"),
        ("dx2", "4 mm"),
        ("z0", "0 mm"),
        ("z1", "0 mm"),
        ("t", "3 mm"),
        ("t2", "3 mm"),
    ] {
        run(&mut s, "variable.set", json!({"name": name, "expr": expr}));
    }
    for (sketch, plate, z, height) in [("Base", "Plate", 0, "h"), ("Base2", "Plate2", 30, "h2")] {
        run(
            &mut s,
            "sketch.create",
            json!({"name": sketch, "offset": z}),
        );
        run(
            &mut s,
            "sketch.polyline",
            json!({"sketch": sketch, "closed": true,
                   "points": [[0, 0], [40, 0], [40, 20], [0, 20]]}),
        );
        built(
            &mut s,
            "feature.extrude",
            json!({"sketch": sketch, "distance": height, "name": plate}),
        );
    }
    for (name, x, radius) in [("Rod", 10, "r"), ("Rod2", 30, "r2")] {
        built(
            &mut s,
            "feature.cylinder",
            json!({"radius": radius, "height": 20, "position": [x, 10, -2], "name": name}),
        );
    }
    built(
        &mut s,
        "feature.block",
        json!({"size": ["w", 10, 40], "position": [18, 5, 2], "name": "Bar"}),
    );
    for (name, distance) in [("Shelf", "lift"), ("Shelf2", "lift2")] {
        built(
            &mut s,
            "datum.plane",
            json!({"from": "xy", "distance": distance, "name": name}),
        );
    }
    // Upright axes, to turn about.
    for (name, x, reach) in [("Pole", 5, "reach"), ("Pole2", 35, "reach2")] {
        built(
            &mut s,
            "datum.axis",
            json!({"through": [[x, 5, 0], [x, 5, reach]], "name": name}),
        );
    }
    // Axes in the xy plane, to revolve a sketch on it about.
    for (name, y, lay) in [("Lay", -20, "lay"), ("Lay2", -30, "lay2")] {
        built(
            &mut s,
            "datum.axis",
            json!({"through": [[0, y, 0], [lay, y, 0]], "name": name}),
        );
    }
    for (name, x) in [("Dot", "dx"), ("Dot2", "dx2")] {
        built(
            &mut s,
            "datum.point",
            json!({"at": [x, 2, 3], "name": name}),
        );
    }
    // Sketches to sweep: a disc and a square each, clear of the axes.
    for (name, x, offset) in [("Disc", 70, "z0"), ("Disc2", 90, "z1")] {
        run(
            &mut s,
            "sketch.create",
            json!({"name": name, "offset": offset}),
        );
        run(
            &mut s,
            "sketch.circle",
            json!({"sketch": name, "center": [x, 10], "radius": 4}),
        );
    }
    // A sketch under the plates, to extrude up to them.
    run(
        &mut s,
        "sketch.create",
        json!({"name": "Under", "offset": -10}),
    );
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Under", "center": [5, 15], "radius": 2}),
    );
    // Square frames with a round hole, out of the way: a face that can be
    // deleted (the hole's wall).
    for (sketch, name, x, height) in [
        ("Window", "Frame", 400, "t"),
        ("Window2", "Frame2", 500, "t2"),
    ] {
        run(&mut s, "sketch.create", json!({"name": sketch}));
        run(
            &mut s,
            "sketch.polyline",
            json!({"sketch": sketch, "closed": true,
                   "points": [[x, 0], [x + 20, 0], [x + 20, 20], [x, 20]]}),
        );
        run(
            &mut s,
            "sketch.circle",
            json!({"sketch": sketch, "center": [x + 10, 10], "radius": 4}),
        );
        built(
            &mut s,
            "feature.extrude",
            json!({"sketch": sketch, "distance": height, "name": name}),
        );
    }
    for (name, x) in [("Bystander", 200), ("Spare", 300)] {
        built(
            &mut s,
            "feature.block",
            json!({"size": [1, 2, 3], "position": [x, 0, 0], "name": name}),
        );
    }
    let check = run(&mut s, "document.check", json!({}));
    assert_eq!(check["ok"], true, "{check}");
    s
}

/// The variable whose loss fails an upstream feature of [`base`].
fn variable_of(upstream: &str) -> &'static str {
    match upstream {
        "Plate" => "h",
        "Plate2" => "h2",
        "Rod" => "r",
        "Rod2" => "r2",
        "Bar" => "w",
        "Shelf" => "lift",
        "Shelf2" => "lift2",
        "Pole" => "reach",
        "Pole2" => "reach2",
        "Lay" => "lay",
        "Lay2" => "lay2",
        "Dot" => "dx",
        "Dot2" => "dx2",
        "Disc" => "z0",
        "Disc2" => "z1",
        "Frame" => "t",
        "Frame2" => "t2",
        other => panic!("{other} has no variable"),
    }
}

/// What the feature under test is, which decides what is made from it.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Sketch,
    Body,
    Plane,
    Axis,
    Point,
}

/// One reference of one feature kind.
struct Case {
    /// The feature kind and the reference.
    what: &'static str,
    /// The parameter the reference is, as `document.inspect` shows it
    /// under `params` (`through.0`: the first of the points): what the
    /// reason starts with.
    role: &'static str,
    /// The feature of [`base`] the reference points at, and the one it is
    /// repaired with.
    upstream: &'static str,
    alternative: &'static str,
    /// Adds the feature `D` with its reference at the feature named;
    /// returns what it is and the persistent names it refers to.
    make: fn(&mut Session, &str) -> (Kind, Vec<String>),
    /// Points the reference of `D` at the feature named.
    repair: fn(&mut Session, &str),
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            what: "sketch: plane (a face)",
            role: "plane",
            upstream: "Plate",
            alternative: "Plate2",
            make: |s, u| {
                let top = face(s, u, "endCap");
                run(s, "sketch.create", json!({"name": "D", "face": top}));
                (Kind::Sketch, vec![top])
            },
            repair: |s, u| {
                let top = face(s, u, "endCap");
                built(s, "sketch.edit", json!({"sketch": "D", "face": top}));
            },
        },
        Case {
            what: "sketch: plane (a datum plane)",
            role: "plane",
            upstream: "Shelf",
            alternative: "Shelf2",
            make: |s, u| {
                run(s, "sketch.create", json!({"name": "D", "plane": u}));
                (Kind::Sketch, vec![])
            },
            repair: |s, u| {
                built(s, "sketch.edit", json!({"sketch": "D", "plane": u}));
            },
        },
        Case {
            what: "extrude: sketch",
            role: "sketch",
            upstream: "Disc",
            alternative: "Disc2",
            make: |s, u| {
                built(
                    s,
                    "feature.extrude",
                    json!({"sketch": u, "distance": 6, "name": "D"}),
                );
                (Kind::Body, vec![])
            },
            repair: |s, u| {
                built(s, "feature.edit", json!({"feature": "D", "sketch": u}));
            },
        },
        Case {
            what: "extrude: upTo (a face)",
            role: "upTo",
            upstream: "Plate",
            alternative: "Plate2",
            make: |s, u| {
                let floor = face(s, u, "startCap");
                built(
                    s,
                    "feature.extrude",
                    json!({"sketch": "Under", "upTo": floor, "name": "D"}),
                );
                (Kind::Body, vec![floor])
            },
            repair: |s, u| {
                let floor = face(s, u, "startCap");
                built(s, "feature.edit", json!({"feature": "D", "upTo": floor}));
            },
        },
        Case {
            what: "revolve: sketch",
            role: "sketch",
            upstream: "Disc",
            alternative: "Disc2",
            make: |s, u| {
                built(
                    s,
                    "feature.revolve",
                    json!({"sketch": u, "axis": "y", "angle": "90 deg", "name": "D"}),
                );
                (Kind::Body, vec![])
            },
            repair: |s, u| {
                built(s, "feature.edit", json!({"feature": "D", "sketch": u}));
            },
        },
        Case {
            what: "revolve: axis (a datum axis)",
            role: "axis",
            upstream: "Lay",
            alternative: "Lay2",
            make: |s, u| {
                built(
                    s,
                    "feature.revolve",
                    json!({"sketch": "Disc", "axis": u, "angle": "90 deg", "name": "D"}),
                );
                (Kind::Body, vec![])
            },
            repair: |s, u| {
                built(s, "feature.edit", json!({"feature": "D", "axis": u}));
            },
        },
        Case {
            what: "transform: source",
            role: "source",
            upstream: "Rod",
            alternative: "Rod2",
            make: |s, u| {
                built(
                    s,
                    "feature.transform",
                    json!({"source": u, "translate": [0, 0, 40], "name": "D"}),
                );
                (Kind::Body, vec![])
            },
            repair: |s, u| {
                built(s, "feature.edit", json!({"feature": "D", "source": u}));
            },
        },
        Case {
            what: "mirror: source",
            role: "source",
            upstream: "Rod",
            alternative: "Rod2",
            make: |s, u| {
                built(
                    s,
                    "feature.mirror",
                    json!({"source": u, "normal": [0, 0, 1], "point": [0, 0, -30], "name": "D"}),
                );
                (Kind::Body, vec![])
            },
            repair: |s, u| {
                built(s, "feature.edit", json!({"feature": "D", "source": u}));
            },
        },
        Case {
            what: "mirror: plane (a datum plane)",
            role: "plane",
            upstream: "Shelf",
            alternative: "Shelf2",
            make: |s, u| {
                built(
                    s,
                    "feature.mirror",
                    json!({"source": "Rod", "plane": u, "name": "D"}),
                );
                (Kind::Body, vec![])
            },
            repair: |s, u| {
                built(s, "feature.edit", json!({"feature": "D", "plane": u}));
            },
        },
        Case {
            what: "pattern: source",
            role: "source",
            upstream: "Rod",
            alternative: "Rod2",
            make: |s, u| {
                built(
                    s,
                    "feature.pattern",
                    json!({"source": u, "count": 3, "step": [0, 0, 40], "name": "D"}),
                );
                (Kind::Body, vec![])
            },
            repair: |s, u| {
                built(s, "feature.edit", json!({"feature": "D", "source": u}));
            },
        },
        Case {
            what: "pattern: axis (a datum axis)",
            role: "axis",
            upstream: "Pole",
            alternative: "Pole2",
            make: |s, u| {
                built(
                    s,
                    "feature.pattern",
                    json!({"source": "Rod", "count": 3, "angle": "90 deg", "axis": u, "name": "D"}),
                );
                (Kind::Body, vec![])
            },
            repair: |s, u| {
                built(s, "feature.edit", json!({"feature": "D", "axis": u}));
            },
        },
        Case {
            what: "boolean: target",
            role: "target",
            upstream: "Plate",
            alternative: "Plate2",
            make: |s, u| {
                built(
                    s,
                    "feature.boolean",
                    json!({"op": "subtract", "target": u, "tool": "Bar", "name": "D"}),
                );
                (Kind::Body, vec![])
            },
            repair: |s, u| {
                built(s, "feature.edit", json!({"feature": "D", "target": u}));
            },
        },
        Case {
            what: "boolean: tool",
            role: "tool",
            upstream: "Rod",
            alternative: "Rod2",
            make: |s, u| {
                built(
                    s,
                    "feature.boolean",
                    json!({"op": "subtract", "target": "Plate", "tool": u, "name": "D"}),
                );
                (Kind::Body, vec![])
            },
            repair: |s, u| {
                built(s, "feature.edit", json!({"feature": "D", "tool": u}));
            },
        },
        Case {
            what: "chamfer: edges",
            role: "edges.0",
            upstream: "Plate",
            alternative: "Plate2",
            make: |s, u| {
                let (top, side) = (face(s, u, "endCap"), face(s, u, "side"));
                built(
                    s,
                    "feature.chamfer",
                    json!({"edges": [[top, side]], "distance": 1, "name": "D"}),
                );
                (Kind::Body, vec![top, side])
            },
            repair: |s, u| {
                let (top, side) = (face(s, u, "endCap"), face(s, u, "side"));
                built(
                    s,
                    "feature.edit",
                    json!({"feature": "D", "edges": [[top, side]]}),
                );
            },
        },
        Case {
            what: "fillet: edges",
            role: "edges.0",
            upstream: "Plate",
            alternative: "Plate2",
            make: |s, u| {
                let (top, side) = (face(s, u, "endCap"), face(s, u, "side"));
                built(
                    s,
                    "feature.fillet",
                    json!({"edges": [[top, side]], "radius": 1, "name": "D"}),
                );
                (Kind::Body, vec![top, side])
            },
            repair: |s, u| {
                let (top, side) = (face(s, u, "endCap"), face(s, u, "side"));
                built(
                    s,
                    "feature.edit",
                    json!({"feature": "D", "edges": [[top, side]]}),
                );
            },
        },
        Case {
            what: "offset face: faces",
            role: "faces.0",
            upstream: "Plate",
            alternative: "Plate2",
            make: |s, u| {
                let side = face(s, u, "side");
                built(
                    s,
                    "feature.offsetFace",
                    json!({"faces": [side], "distance": 1, "name": "D"}),
                );
                (Kind::Body, vec![side])
            },
            repair: |s, u| {
                let side = face(s, u, "side");
                built(s, "feature.edit", json!({"feature": "D", "faces": [side]}));
            },
        },
        Case {
            what: "delete face: faces",
            role: "faces.0",
            upstream: "Frame",
            alternative: "Frame2",
            make: |s, u| {
                let wall = hole_wall(s, u);
                built(
                    s,
                    "feature.deleteFace",
                    json!({"faces": [wall], "name": "D"}),
                );
                (Kind::Body, vec![wall])
            },
            repair: |s, u| {
                let wall = hole_wall(s, u);
                built(s, "feature.edit", json!({"feature": "D", "faces": [wall]}));
            },
        },
        Case {
            what: "shell: source",
            role: "source",
            upstream: "Rod",
            alternative: "Rod2",
            make: |s, u| {
                built(
                    s,
                    "feature.shell",
                    json!({"source": u, "thickness": 1, "name": "D"}),
                );
                (Kind::Body, vec![])
            },
            repair: |s, u| {
                built(s, "feature.edit", json!({"feature": "D", "source": u}));
            },
        },
        Case {
            what: "datum plane: from (a face)",
            role: "from",
            upstream: "Plate",
            alternative: "Plate2",
            make: |s, u| {
                let top = face(s, u, "endCap");
                built(
                    s,
                    "datum.plane",
                    json!({"from": top, "distance": 3, "name": "D"}),
                );
                (Kind::Plane, vec![top])
            },
            repair: |s, u| {
                let top = face(s, u, "endCap");
                built(
                    s,
                    "datum.plane",
                    json!({"replace": "D", "from": top, "distance": 3}),
                );
            },
        },
        Case {
            what: "datum plane: from (a datum plane)",
            role: "from",
            upstream: "Shelf",
            alternative: "Shelf2",
            make: |s, u| {
                built(
                    s,
                    "datum.plane",
                    json!({"from": u, "distance": 3, "name": "D"}),
                );
                (Kind::Plane, vec![])
            },
            repair: |s, u| {
                built(
                    s,
                    "datum.plane",
                    json!({"replace": "D", "from": u, "distance": 3}),
                );
            },
        },
        Case {
            what: "datum plane: about (an edge)",
            role: "about",
            upstream: "Plate",
            alternative: "Plate2",
            make: |s, u| {
                let edge = vec![face(s, u, "side(e5)"), face(s, u, "startCap")];
                built(
                    s,
                    "datum.plane",
                    json!({"from": "xy", "about": edge, "angle": "30 deg", "name": "D"}),
                );
                (Kind::Plane, edge)
            },
            repair: |s, u| {
                let edge = vec![face(s, u, "side(e5)"), face(s, u, "startCap")];
                built(
                    s,
                    "datum.plane",
                    json!({"replace": "D", "from": "xy", "about": edge, "angle": "30 deg"}),
                );
            },
        },
        Case {
            what: "datum plane: about (a datum axis)",
            role: "about",
            upstream: "Pole",
            alternative: "Pole2",
            make: |s, u| {
                built(
                    s,
                    "datum.plane",
                    json!({"from": "zx", "about": u, "angle": "30 deg", "name": "D"}),
                );
                (Kind::Plane, vec![])
            },
            repair: |s, u| {
                built(
                    s,
                    "datum.plane",
                    json!({"replace": "D", "from": "zx", "about": u, "angle": "30 deg"}),
                );
            },
        },
        Case {
            what: "datum plane: through (a vertex)",
            role: "through.0",
            upstream: "Plate",
            alternative: "Plate2",
            make: |s, u| {
                let z = if u == "Plate" { 5.0 } else { 35.0 };
                let corner = vertex(s, u, [0.0, 0.0, z]);
                built(
                    s,
                    "datum.plane",
                    json!({"through": [corner, [100, 0, 0], [0, 100, 50]], "name": "D"}),
                );
                (Kind::Plane, vec![corner])
            },
            repair: |s, u| {
                let z = if u == "Plate" { 5.0 } else { 35.0 };
                let corner = vertex(s, u, [0.0, 0.0, z]);
                built(
                    s,
                    "datum.plane",
                    json!({"replace": "D", "through": [corner, [100, 0, 0], [0, 100, 50]]}),
                );
            },
        },
        Case {
            what: "datum plane: through (a datum point)",
            role: "through.0",
            upstream: "Dot",
            alternative: "Dot2",
            make: |s, u| {
                built(
                    s,
                    "datum.plane",
                    json!({"through": [u, [100, 0, 0], [0, 100, 50]], "name": "D"}),
                );
                (Kind::Plane, vec![])
            },
            repair: |s, u| {
                built(
                    s,
                    "datum.plane",
                    json!({"replace": "D", "through": [u, [100, 0, 0], [0, 100, 50]]}),
                );
            },
        },
        Case {
            what: "datum axis: through (a datum point)",
            role: "through.0",
            upstream: "Dot",
            alternative: "Dot2",
            make: |s, u| {
                built(
                    s,
                    "datum.axis",
                    json!({"through": [u, [50, 60, 70]], "name": "D"}),
                );
                (Kind::Axis, vec![])
            },
            repair: |s, u| {
                built(
                    s,
                    "datum.axis",
                    json!({"replace": "D", "through": [u, [50, 60, 70]]}),
                );
            },
        },
        Case {
            what: "datum axis: through (a vertex)",
            role: "through.0",
            upstream: "Plate",
            alternative: "Plate2",
            make: |s, u| {
                let z = if u == "Plate" { 5.0 } else { 35.0 };
                let corner = vertex(s, u, [0.0, 0.0, z]);
                built(
                    s,
                    "datum.axis",
                    json!({"through": [corner, [50, 60, 70]], "name": "D"}),
                );
                (Kind::Axis, vec![corner])
            },
            repair: |s, u| {
                let z = if u == "Plate" { 5.0 } else { 35.0 };
                let corner = vertex(s, u, [0.0, 0.0, z]);
                built(
                    s,
                    "datum.axis",
                    json!({"replace": "D", "through": [corner, [50, 60, 70]]}),
                );
            },
        },
        Case {
            what: "datum axis: edge",
            role: "edge",
            upstream: "Plate",
            alternative: "Plate2",
            make: |s, u| {
                let edge = vec![face(s, u, "side(e5)"), face(s, u, "side(e6)")];
                built(s, "datum.axis", json!({"edge": edge, "name": "D"}));
                (Kind::Axis, edge)
            },
            repair: |s, u| {
                let edge = vec![face(s, u, "side(e5)"), face(s, u, "side(e6)")];
                built(s, "datum.axis", json!({"replace": "D", "edge": edge}));
            },
        },
        Case {
            what: "datum axis: face",
            role: "face",
            upstream: "Rod",
            alternative: "Rod2",
            make: |s, u| {
                let wall = face(s, u, "face(0)");
                built(s, "datum.axis", json!({"face": wall, "name": "D"}));
                (Kind::Axis, vec![wall])
            },
            repair: |s, u| {
                let wall = face(s, u, "face(0)");
                built(s, "datum.axis", json!({"replace": "D", "face": wall}));
            },
        },
        Case {
            what: "datum axis: planes (a datum plane)",
            role: "planes.0",
            upstream: "Shelf",
            alternative: "Shelf2",
            make: |s, u| {
                built(s, "datum.axis", json!({"planes": [u, "zx"], "name": "D"}));
                (Kind::Axis, vec![])
            },
            repair: |s, u| {
                built(
                    s,
                    "datum.axis",
                    json!({"replace": "D", "planes": [u, "zx"]}),
                );
            },
        },
        Case {
            what: "datum axis: planes (a face)",
            role: "planes.0",
            upstream: "Plate",
            alternative: "Plate2",
            make: |s, u| {
                let side = face(s, u, "side(e5)");
                built(
                    s,
                    "datum.axis",
                    json!({"planes": [side, "xy"], "name": "D"}),
                );
                (Kind::Axis, vec![side])
            },
            repair: |s, u| {
                let side = face(s, u, "side(e5)");
                built(
                    s,
                    "datum.axis",
                    json!({"replace": "D", "planes": [side, "xy"]}),
                );
            },
        },
        Case {
            what: "datum point: vertex",
            role: "at",
            upstream: "Plate",
            alternative: "Plate2",
            make: |s, u| {
                let z = if u == "Plate" { 5.0 } else { 35.0 };
                let corner = vertex(s, u, [0.0, 0.0, z]);
                built(s, "datum.point", json!({"vertex": corner, "name": "D"}));
                (Kind::Point, vec![corner])
            },
            repair: |s, u| {
                let z = if u == "Plate" { 5.0 } else { 35.0 };
                let corner = vertex(s, u, [0.0, 0.0, z]);
                built(s, "datum.point", json!({"replace": "D", "vertex": corner}));
            },
        },
        Case {
            what: "datum point: center",
            role: "at",
            upstream: "Rod",
            alternative: "Rod2",
            make: |s, u| {
                let rim = vec![face(s, u, "face(0)"), face(s, u, "face(2)")];
                built(s, "datum.point", json!({"center": rim, "name": "D"}));
                (Kind::Point, rim)
            },
            repair: |s, u| {
                let rim = vec![face(s, u, "face(0)"), face(s, u, "face(2)")];
                built(s, "datum.point", json!({"replace": "D", "center": rim}));
            },
        },
    ]
}

/// Adds the feature `T`, made from `D`.
fn tail(s: &mut Session, kind: Kind) {
    match kind {
        Kind::Sketch => {
            run(
                s,
                "sketch.circle",
                json!({"sketch": "D", "center": [10, 10], "radius": 2}),
            );
            built(
                s,
                "feature.extrude",
                json!({"sketch": "D", "distance": 2, "name": "T"}),
            );
        }
        Kind::Body => {
            built(
                s,
                "feature.transform",
                json!({"source": "D", "translate": [0, 0, 300], "name": "T"}),
            );
        }
        Kind::Plane => {
            built(s, "sketch.create", json!({"plane": "D", "name": "T"}));
        }
        Kind::Axis => {
            built(
                s,
                "feature.pattern",
                json!({"source": "Spare", "count": 2, "angle": "90 deg", "axis": "D", "name": "T"}),
            );
        }
        Kind::Point => {
            built(
                s,
                "datum.axis",
                json!({"through": ["D", [500, 600, 700]], "name": "T"}),
            );
        }
    }
}

/// [`base`] with the case's feature `D` pointing at `upstream`, and `T`
/// made from `D`.
fn model(case: &Case, upstream: &str) -> (Session, Vec<String>) {
    let mut s = base();
    let (kind, names) = (case.make)(&mut s, upstream);
    tail(&mut s, kind);
    let document = inspect(&mut s);
    assert_eq!(document["ok"], true, "{}: {document}", case.what);
    (s, names)
}

fn case_named(what: &str) -> Case {
    cases()
        .into_iter()
        .find(|case| case.what == what)
        .unwrap_or_else(|| panic!("no case {what}"))
}

/// Every feature that did not fail is exactly what it is in the same
/// document without the failed features: its status, its body, what its
/// faces are called, who took its body. Also: nothing that is built is
/// made from a failed feature (taking the failed ones away fails nothing
/// else).
fn independent_features_are_untouched(session: &Session, context: &str) {
    let mut with = session.clone();
    let before = state(&mut with);
    let failed: Vec<String> = before["features"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["status"] == "failed")
        .map(|f| f["id"].as_str().unwrap().to_string())
        .collect();
    // Deleting the feature the rollback marker is on leaves the marker on
    // the one before. Only when every feature up to the marker failed is
    // there none left to carry it, and everything would roll forward:
    // that is a change of its own.
    let features = before["features"].as_array().unwrap();
    if let Some(at) = features.iter().position(|f| f["id"] == before["rollback"])
        && features
            .iter()
            .take(at + 1)
            .all(|f| f["status"] == "failed")
    {
        return;
    }
    let mut without = with.clone();
    for id in &failed {
        only(&mut without, "feature.delete", json!({"feature": id}));
    }
    let after = state(&mut without);
    let kept: Vec<&Value> = before["features"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["status"] != "failed")
        .collect();
    let now: Vec<&Value> = after["features"].as_array().unwrap().iter().collect();
    assert_eq!(kept, now, "{context}: features without the failed ones");
    assert_eq!(
        before["bodies"], after["bodies"],
        "{context}: bodies without the failed features"
    );
}

/// Undoing the last edit and redoing it gives the same document.
fn undo_and_redo_restore(session: &Session, context: &str) {
    let mut copy = session.clone();
    let before = state(&mut copy);
    if copy.execute("edit.undo", &json!({})).is_err() {
        return;
    }
    run(&mut copy, "edit.redo", json!({}));
    assert_eq!(state(&mut copy), before, "{context}: after undo and redo");
}

/// No feature fails without saying why, and no reason is a caught panic.
fn every_failure_has_a_reason(document: &Value, context: &str) {
    let features = document["features"].as_array().unwrap();
    for (at, entry) in features.iter().enumerate() {
        let reason = entry["error"].as_str().unwrap_or_default();
        if entry["status"] == "failed" {
            assert!(reason.len() > 10, "{context}: {entry}");
            assert!(!reason.contains("internal error"), "{context}: {entry}");
        } else {
            assert_eq!(reason, "", "{context}: {entry}");
            assert_eq!(entry.get("causedBy"), None, "{context}: {entry}");
        }
        // A feature that waits for a failed one says which to fix: an
        // earlier feature that failed for a reason of its own.
        let waits = reason.contains("fix it first");
        match entry.get("causedBy") {
            None => assert!(!waits, "{context}: {entry}"),
            Some(root) => {
                assert!(waits, "{context}: {entry}");
                let found = features.iter().position(|f| &f["id"] == root).unwrap();
                assert!(found < at, "{context}: {entry}");
                let root = &features[found];
                assert_eq!(root["status"], "failed", "{context}: {entry}");
                assert_eq!(root.get("causedBy"), None, "{context}: {root}");
            }
        }
    }
}

fn no_findings(wrong: &[String]) {
    assert!(
        wrong.is_empty(),
        "{} findings:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// A way of breaking what a reference points at.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Break {
    Delete,
    Suppress,
    /// Moved to the end of the history: after what refers to it.
    MoveAfter,
    /// Its variable is deleted, so that it fails itself.
    Fail,
}

const BREAKS: [Break; 4] = [
    Break::Delete,
    Break::Suppress,
    Break::MoveAfter,
    Break::Fail,
];

fn apply(s: &mut Session, way: Break, upstream: &str) {
    match way {
        Break::Delete => run(s, "feature.delete", json!({"feature": upstream})),
        Break::Suppress => run(s, "feature.suppress", json!({"feature": upstream})),
        Break::MoveAfter => {
            let last = inspect(s)["features"].as_array().unwrap().len() - 1;
            run(
                s,
                "feature.move",
                json!({"feature": upstream, "position": last}),
            )
        }
        Break::Fail => run(s, "variable.delete", json!({"name": variable_of(upstream)})),
    };
}

/// What the reason of the feature whose reference broke has to say.
fn expected_reason(way: Break, role: &str, upstream: &str, id: &str) -> Vec<String> {
    let mut parts = vec![format!("{role}: ")];
    match way {
        Break::Delete => parts.extend([id.to_string(), "deleted".to_string()]),
        Break::Suppress => parts.extend([format!("`{upstream}`"), "is suppressed".to_string()]),
        Break::MoveAfter => parts.extend([
            format!("`{upstream}`"),
            "comes after".to_string(),
            "feature.move".to_string(),
        ]),
        Break::Fail => parts.extend([format!("`{upstream}`"), "failed; fix it first".to_string()]),
    }
    parts
}

/// With the reference of `D` broken: `D` fails with a reason that starts
/// with the parameter and has `expected` in it, `T` fails naming `D`,
/// nothing else fails (but `also`), the rest is untouched, and the
/// document says so both ways of asking. Returns what does not hold.
fn broken_as_expected(
    s: &mut Session,
    context: &str,
    role: &str,
    expected: &[String],
    also: Option<&str>,
) -> Vec<String> {
    let mut wrong = Vec::new();
    let broken = state(s);
    let (d, reason) = status(&broken, "D");
    if d != "failed" {
        wrong.push(format!("{context}: D is {d}, not failed"));
    }
    for part in expected {
        if !reason.contains(part.as_str()) {
            wrong.push(format!("{context}: no `{part}` in D's reason: {reason}"));
        }
    }
    if !reason.starts_with(&format!("{role}: ")) {
        wrong.push(format!(
            "{context}: D's reason does not start with `{role}: `: {reason}"
        ));
    }
    // What is made from it fails naming it.
    let (t, because) = status(&broken, "T");
    if t != "failed" || !because.contains("`D`") || !because.contains("fix it first") {
        wrong.push(format!("{context}: T is {t}: {because}"));
    }
    for entry in broken["features"].as_array().unwrap() {
        let name = entry["name"].as_str().unwrap();
        let may_fail = name == "D" || name == "T" || also == Some(name);
        if entry["status"] == "failed" && !may_fail {
            wrong.push(format!("{context}: {name} failed too: {}", entry["error"]));
        }
    }
    // Which feature to fix: for `D` the one it refers to when that
    // failed (else `D` is the one), for `T` the same feature.
    let id = |name: &str| feature(&broken, name)["id"].clone();
    let (root_of_d, root_of_t) = match also {
        Some(upstream) => (id(upstream), id(upstream)),
        None => (Value::Null, id("D")),
    };
    for (name, root) in [("D", &root_of_d), ("T", &root_of_t)] {
        let found = &feature(&broken, name)["causedBy"];
        if found != root {
            wrong.push(format!(
                "{context}: {name} is caused by {found}, not {root}"
            ));
        }
    }
    every_failure_has_a_reason(&broken, context);
    independent_features_are_untouched(s, context);
    // The document says so, both ways of asking.
    assert_eq!(broken["ok"], false, "{context}");
    let check = run(s, "document.check", json!({}));
    assert_eq!(check["ok"], false, "{context}");
    let problems = check["problems"].as_array().unwrap();
    for name in ["D", "T"] {
        let (_, reason) = status(&broken, name);
        if !problems.iter().any(|p| {
            p["name"] == name
                && p["problem"] == reason.as_str()
                && p.get("causedBy") == feature(&broken, name).get("causedBy")
        }) {
            wrong.push(format!(
                "{context}: document.check does not list {name}: {check}"
            ));
        }
    }
    wrong
}

/// The audit: every reference, broken in every way. Everything that does
/// not hold is collected, so that one run shows all of it; the reasons are
/// printed (`--nocapture`).
#[test]
fn a_broken_reference_fails_its_feature_with_the_reason_and_nothing_else() {
    let results = map_parallel(cases(), |case| {
        let mut wrong: Vec<String> = Vec::new();
        let mut table: Vec<String> = Vec::new();
        let (mut s, names) = model(&case, case.upstream);
        let upstream_id = id_of(&mut s, case.upstream);
        let intact = state(&mut s);
        for way in BREAKS {
            let context = format!("{} / {way:?}", case.what);
            apply(&mut s, way, case.upstream);
            let broken = state(&mut s);
            table.push(format!("{context}: {}", status(&broken, "D").1));
            let mut expected = expected_reason(way, case.role, case.upstream, &upstream_id);
            // The names it lost.
            expected.extend(names.iter().cloned());
            let also = (way == Break::Fail).then_some(case.upstream);
            wrong.extend(broken_as_expected(
                &mut s, &context, case.role, &expected, also,
            ));
            // Undo and redo restore exactly.
            run(&mut s, "edit.undo", json!({}));
            assert_eq!(state(&mut s), intact, "{context}: after undo");
            run(&mut s, "edit.redo", json!({}));
            assert_eq!(state(&mut s), broken, "{context}: after redo");
            run(&mut s, "edit.undo", json!({}));
        }
        (table, wrong)
    });
    let (table, wrong): (Vec<_>, Vec<_>) = results.into_iter().unzip();
    println!("{}", table.concat().join("\n"));
    no_findings(&wrong.concat());
}

/// Rolling back before what a feature refers to rolls the feature back
/// too: nothing fails, and what comes before the marker is what it is in
/// the document that ends there.
#[test]
fn rolling_back_before_a_reference_rolls_the_feature_back_without_a_failure() {
    map_parallel(cases(), |case| {
        let (mut s, _) = model(&case, case.upstream);
        let intact = state(&mut s);
        let order = names_in_order(&intact);
        let at = order.iter().position(|n| n == case.upstream).unwrap();
        // Before the upstream feature, and on it.
        for marker in [at - 1, at] {
            let context = format!("{} / rolled back to {}", case.what, order[marker]);
            run(
                &mut s,
                "history.rollback",
                json!({"feature": order[marker]}),
            );
            let rolled = state(&mut s);
            assert_eq!(rolled["ok"], true, "{context}: {rolled}");
            rolled_back_is_the_document_that_ends_there(&s, &context);
            let check = run(&mut s, "document.check", json!({}));
            assert_eq!(check["ok"], true, "{context}: {check}");
            run(&mut s, "edit.undo", json!({}));
            assert_eq!(state(&mut s), intact, "{context}: after undo");
            run(&mut s, "edit.redo", json!({}));
            assert_eq!(state(&mut s), rolled, "{context}: after redo");
            run(&mut s, "history.rollback", json!({"feature": null}));
            assert_eq!(state(&mut s), intact, "{context}: rolled forward");
        }
    });
}

/// How many edits can be undone.
fn undo_steps(session: &mut Session) -> usize {
    inspect(session)["history"]["undo"]
        .as_array()
        .unwrap()
        .len()
}

fn names_in_order(document: &Value) -> Vec<String> {
    document["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["name"].as_str().unwrap().to_string())
        .collect()
}

/// With a rollback marker set: the features up to it, and the bodies, are
/// those of the document with everything after the marker deleted; the
/// features after it are rolled back and give no reason.
fn rolled_back_is_the_document_that_ends_there(session: &Session, context: &str) {
    let mut held = session.clone();
    let rolled = state(&mut held);
    let Some(marker) = rolled["rollback"].as_str() else {
        return;
    };
    let features = rolled["features"].as_array().unwrap();
    let at = features.iter().position(|f| f["id"] == marker).unwrap();
    let mut ends_there = held.clone();
    for entry in features.iter().skip(at + 1).rev() {
        only(
            &mut ends_there,
            "feature.delete",
            json!({"feature": entry["id"]}),
        );
        assert_eq!(
            (&entry["status"], &entry["error"]),
            (&json!("rolledBack"), &Value::Null),
            "{context}: {entry}"
        );
    }
    let ends_there = state(&mut ends_there);
    assert_eq!(rolled["bodies"], ends_there["bodies"], "{context}: bodies");
    // A feature that fails says why in terms of the document it is in
    // (what it needs "comes after" it here and is not there at all in the
    // document that ends at the marker): its reason is not compared.
    let without_reason = |entries: &mut dyn Iterator<Item = &Value>| -> Vec<Value> {
        entries
            .map(|entry| {
                let mut entry = entry.clone();
                entry.as_object_mut().unwrap().remove("error");
                entry
            })
            .collect()
    };
    assert_eq!(
        without_reason(&mut features.iter().take(at + 1)),
        without_reason(&mut ends_there["features"].as_array().unwrap().iter()),
        "{context}: the features up to the marker"
    );
}

/// Repair: with what it referred to deleted, suppressed, moved or failed,
/// the reference is pointed at another entity by a command, and the
/// document is then what building it that way from the start gives: the
/// same features, parameters, bodies and names.
#[test]
fn a_repaired_reference_gives_the_document_built_that_way_from_the_start() {
    let wrong = map_parallel(cases(), |case| {
        let mut wrong: Vec<String> = Vec::new();
        for way in BREAKS {
            let context = format!("{} / {way:?}", case.what);
            let (mut repaired, _) = model(&case, case.upstream);
            let intact = state(&mut repaired);
            apply(&mut repaired, way, case.upstream);
            let broken = state(&mut repaired);
            assert_eq!(status(&broken, "D").0, "failed", "{context}");
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                (case.repair)(&mut repaired, case.alternative);
            }));
            if outcome.is_err() {
                wrong.push(format!("{context}: the repair command failed"));
                continue;
            }
            let (mut fresh, _) = model(&case, case.alternative);
            apply(&mut fresh, way, case.upstream);
            let (now, fresh) = (state(&mut repaired), state(&mut fresh));
            if status(&now, "D").0 != "ok" || status(&now, "T").0 != "ok" {
                wrong.push(format!(
                    "{context}: after the repair D is {:?} and T {:?}",
                    status(&now, "D"),
                    status(&now, "T")
                ));
            }
            if now != fresh {
                wrong.push(format!(
                    "{context}: the repaired document is not the one built that way:\n  repaired {}\n  fresh    {}",
                    feature(&now, "D"),
                    feature(&fresh, "D")
                ));
            }
            // The repair is one undo step, and so is the break.
            run(&mut repaired, "edit.undo", json!({}));
            assert_eq!(state(&mut repaired), broken, "{context}: repair undone");
            run(&mut repaired, "edit.undo", json!({}));
            assert_eq!(state(&mut repaired), intact, "{context}: break undone");
            run(&mut repaired, "edit.redo", json!({}));
            run(&mut repaired, "edit.redo", json!({}));
            assert_eq!(state(&mut repaired), now, "{context}: both redone");
        }
        wrong
    });
    no_findings(&wrong.concat());
}

/// The feature a reference points at is still there and builds, but the
/// face, edge or vertex the reference names is not: the plate's first
/// side is drawn again as a new line, so everything named after `e5` is
/// gone and something named after `e9` is there instead. The reason says
/// what the feature no longer has and what it has. A cap is the
/// exception by design: a reference to a cap survives a change of its
/// outline (docs/architecture.md §1.1), so those features are built.
#[test]
fn a_name_that_is_no_longer_there_fails_with_what_is_there_instead() {
    let on_the_plate: Vec<Case> = cases()
        .into_iter()
        .filter(|case| case.upstream == "Plate" && !case.role.starts_with("target"))
        .collect();
    assert_eq!(on_the_plate.len(), 12);
    let wrong = map_parallel(on_the_plate, |case| {
        let (mut s, names) = model(&case, "Plate");
        let intact = state(&mut s);
        let context = format!("{} / redrawn", case.what);
        run(
            &mut s,
            "sketch.delete",
            json!({"sketch": "Base", "entity": "e5"}),
        );
        // In between the profile is open: the plate fails, and so does
        // what refers to it.
        let open = state(&mut s);
        assert_eq!(status(&open, "Plate").0, "failed", "{context}");
        assert_eq!(status(&open, "D").0, "failed", "{context}");
        let line = run(
            &mut s,
            "sketch.line",
            json!({"sketch": "Base", "from": "e1", "to": "e2"}),
        );
        assert_eq!(line["entity"], "e9", "{context}: {line}");
        let redrawn = state(&mut s);
        assert_eq!(status(&redrawn, "Plate").0, "ok", "{context}");
        let lost: Vec<&String> = names.iter().filter(|name| !name.contains("Cap(")).collect();
        let mut wrong = Vec::new();
        if lost.is_empty() {
            // Caps only: the feature is built, where it was.
            for name in ["D", "T"] {
                if status(&redrawn, name).0 != "ok" {
                    wrong.push(format!("{context}: {name} is {:?}", status(&redrawn, name)));
                }
            }
            for name in ["D", "T"] {
                assert_eq!(feature(&redrawn, name), feature(&intact, name), "{context}");
                assert_eq!(body_of(&redrawn, name), body_of(&intact, name), "{context}");
            }
        } else {
            let expected = vec![
                "`Plate` no longer has a".to_string(),
                lost[0].clone(),
                "e9".to_string(),
            ];
            wrong.extend(broken_as_expected(
                &mut s, &context, case.role, &expected, None,
            ));
        }
        run(&mut s, "edit.undo", json!({}));
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(state(&mut s), intact, "{context}: after undo");
        wrong
    });
    no_findings(&wrong.concat());
}

/// A sketch on a side of the plate whose line was drawn again is pointed
/// at the new side, and is then the sketch made on that side from the
/// start.
#[test]
fn a_sketch_is_pointed_at_the_face_that_replaced_the_one_it_lay_on() {
    let redraw = |s: &mut Session| {
        run(
            s,
            "sketch.delete",
            json!({"sketch": "Base", "entity": "e5"}),
        );
        run(
            s,
            "sketch.line",
            json!({"sketch": "Base", "from": "e1", "to": "e2"}),
        );
    };
    let on_a_side = |s: &mut Session, side: &str| {
        let side = face(s, "Plate", side);
        run(s, "sketch.create", json!({"name": "D", "face": side}));
        tail(s, Kind::Sketch);
    };
    let mut repaired = base();
    on_a_side(&mut repaired, "side(e5)");
    redraw(&mut repaired);
    let (now, reason) = status(&inspect(&mut repaired), "D");
    assert_eq!(now, "failed");
    // The reason offers the name to point it at.
    let side = face(&mut repaired, "Plate", "side(e9)");
    assert!(reason.contains(&side), "{reason}");
    built(
        &mut repaired,
        "sketch.edit",
        json!({"sketch": "D", "face": side}),
    );
    // Built that way from the start. The features are added in the same
    // order, so they have the same ids; the plate's sketch is redrawn
    // first here and last there, which is not part of what a document is.
    let mut fresh = base();
    on_a_side(&mut fresh, "side(e6)");
    redraw(&mut fresh);
    built(
        &mut fresh,
        "sketch.edit",
        json!({"sketch": "D", "face": side}),
    );
    assert_eq!(state(&mut repaired), state(&mut fresh));
    let document = inspect(&mut repaired);
    assert_eq!(document["ok"], true, "{document}");
}

/// What a reference points at is still there, but no longer fits what the
/// feature needs of it. The feature fails with the parameter; what is
/// made from it fails naming it; nothing else changes.
#[test]
fn geometry_that_no_longer_fits_fails_the_feature_that_needs_it() {
    type Edit = fn(&mut Session);
    let misfits: Vec<(&str, Edit, &[&str])> = vec![
        (
            // The axis leaves the plane of the sketch.
            "revolve: axis (a datum axis)",
            |s| {
                run(
                    s,
                    "datum.axis",
                    json!({"replace": "Lay", "through": [[0, -20, 0], [1, -20, 5]]}),
                );
            },
            &["`Lay`", "plane"],
        ),
        (
            // The two planes become parallel.
            "datum axis: planes (a datum plane)",
            |s| {
                run(
                    s,
                    "datum.plane",
                    json!({"replace": "Shelf", "from": "zx", "distance": 5}),
                );
            },
            &["parallel"],
        ),
        (
            // The axis is no longer parallel to the plane turned about it.
            "datum plane: about (a datum axis)",
            |s| {
                run(
                    s,
                    "datum.axis",
                    json!({"replace": "Pole", "through": [[5, 5, 0], [5, 9, 30]]}),
                );
            },
            &["parallel"],
        ),
        (
            // The three points come to lie on one line.
            "datum plane: through (a datum point)",
            |s| {
                run(
                    s,
                    "feature.edit",
                    json!({"feature": "Dot", "position": [50, 50, 25]}),
                );
            },
            &["one line"],
        ),
        (
            // The face moves behind the sketch the extrusion starts from.
            "extrude: upTo (a face)",
            |s| {
                run(s, "sketch.edit", json!({"sketch": "Base", "offset": -50}));
            },
            &["startCap", "behind"],
        ),
        (
            // The plate shrinks until the face no longer covers the
            // profile.
            "extrude: upTo (a face)",
            |s| {
                for (point, at) in [("e3", [40, 10]), ("e4", [0, 10])] {
                    run(
                        s,
                        "sketch.edit",
                        json!({"sketch": "Base", "entity": point, "at": at}),
                    );
                }
            },
            &["startCap", "misses the face"],
        ),
    ];
    let wrong = map_parallel(misfits, |(what, edit, expected)| {
        let case = case_named(what);
        let (mut s, _) = model(&case, case.upstream);
        let steps = undo_steps(&mut s);
        let intact = state(&mut s);
        let context = format!("{what} / no longer fits");
        edit(&mut s);
        let role = case.role.split('.').next().unwrap();
        let reason = status(&inspect(&mut s), "D").1;
        println!("{context}: {reason}");
        let expected: Vec<String> = expected.iter().map(ToString::to_string).collect();
        let wrong = broken_as_expected(&mut s, &context, role, &expected, None);
        while undo_steps(&mut s) > steps {
            run(&mut s, "edit.undo", json!({}));
        }
        assert_eq!(state(&mut s), intact, "{context}: after undo");
        wrong
    });
    no_findings(&wrong.concat());
}

/// Every expression of a feature is a reference to the variables in it.
/// A variable that is deleted fails the features that use it, each with
/// its own parameter; defining it again is the repair.
#[test]
fn a_lost_variable_fails_the_features_that_use_it_with_the_parameter() {
    let mut s = base();
    for (name, expr) in [("v", "3 mm"), ("a", "30 deg"), ("n", "3")] {
        run(&mut s, "variable.set", json!({"name": name, "expr": expr}));
    }
    // (feature, the variable it uses, the parameter the variable is in)
    let mut uses: Vec<(&str, &str, &str)> = Vec::new();
    let mut add = |s: &mut Session, id: &str, params: Value, variable, parameter| {
        let name = params["name"].as_str().unwrap().to_string();
        built(s, id, params);
        uses.push((name.leak(), variable, parameter));
    };
    add(
        &mut s,
        "feature.block",
        json!({"size": ["v", 1, 1], "position": [400, 0, 0], "name": "Block"}),
        "v",
        "size.x",
    );
    add(
        &mut s,
        "feature.cylinder",
        json!({"radius": 1, "height": "v", "position": [410, 0, 0], "name": "Cylinder"}),
        "v",
        "height",
    );
    add(
        &mut s,
        "feature.sphere",
        json!({"radius": "v", "position": [420, 0, 0], "name": "Sphere"}),
        "v",
        "radius",
    );
    add(
        &mut s,
        "feature.cone",
        json!({"radius": 1, "height": 2, "position": [430, 0, 0], "angle": "a", "name": "Cone"}),
        "a",
        "angle",
    );
    add(
        &mut s,
        "feature.torus",
        json!({"major": 5, "minor": "v / 3", "position": [450, 0, 0], "name": "Torus"}),
        "v",
        "minor",
    );
    add(
        &mut s,
        "sketch.create",
        json!({"plane": "zx", "offset": "v", "name": "Lifted"}),
        "v",
        "plane.offset",
    );
    add(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Disc", "distance": "2 * v", "name": "Extrusion"}),
        "v",
        "distance",
    );
    add(
        &mut s,
        "feature.revolve",
        json!({"sketch": "Disc2", "axis": "y", "angle": "a", "name": "Revolution"}),
        "a",
        "angle",
    );
    add(
        &mut s,
        "feature.transform",
        json!({"source": "Rod", "translate": [0, 0, "v"], "name": "Moved"}),
        "v",
        "translate.z",
    );
    add(
        &mut s,
        "feature.mirror",
        json!({"source": "Rod2", "normal": [0, 0, 1], "point": [0, 0, "-v"], "name": "Mirrored"}),
        "v",
        "point.z",
    );
    add(
        &mut s,
        "feature.pattern",
        json!({"source": "Bystander", "count": "n", "step": [0, 10, 0], "name": "Row"}),
        "n",
        "count",
    );
    add(
        &mut s,
        "feature.pattern",
        json!({"source": "Spare", "count": 3, "angle": "a", "axis": "Pole", "name": "Circle"}),
        "a",
        "angle",
    );
    add(
        &mut s,
        "datum.plane",
        json!({"from": "Shelf", "distance": "v", "name": "Higher"}),
        "v",
        "distance",
    );
    add(
        &mut s,
        "datum.plane",
        json!({"from": "zx", "about": "Pole", "angle": "a", "name": "Turned"}),
        "a",
        "angle",
    );
    add(
        &mut s,
        "datum.point",
        json!({"at": [0, "v", 0], "name": "Mark"}),
        "v",
        "at.y",
    );
    add(
        &mut s,
        "datum.axis",
        json!({"through": [[0, 0, 0], ["v", 0, 9]], "name": "Line"}),
        "v",
        "through.1.x",
    );
    let intact = state(&mut s);
    assert_eq!(intact["ok"], true, "{intact}");
    for (variable, expr) in [("v", "3 mm"), ("a", "30 deg"), ("n", "3")] {
        run(&mut s, "variable.delete", json!({"name": variable}));
        let broken = state(&mut s);
        for (name, used, parameter) in &uses {
            let (now, reason) = status(&broken, name);
            if *used == variable {
                assert_eq!(now, "failed", "{name} without {variable}");
                assert!(
                    reason.starts_with(&format!("{parameter}: "))
                        && reason.contains(&format!("`{variable}`")),
                    "{name} without {variable}: {reason}"
                );
            } else {
                assert_eq!(feature(&broken, name), feature(&intact, name));
            }
        }
        every_failure_has_a_reason(&broken, variable);
        independent_features_are_untouched(&s, variable);
        // Defined again, every feature and body is what it was. (The
        // variable itself is now the last of the list.)
        run(
            &mut s,
            "variable.set",
            json!({"name": variable, "expr": expr}),
        );
        let again = state(&mut s);
        for key in ["features", "bodies", "ok"] {
            assert_eq!(again[key], intact[key], "{variable} defined again");
        }
    }
}

/// What `feature.edit` refuses when a feature is pointed at something
/// else, and that a refusal changes nothing.
#[test]
fn pointing_a_feature_at_something_it_cannot_be_made_from_is_refused() {
    let try_all = |what: &str, attempts: &[(Value, &str)]| {
        let case = case_named(what);
        let (mut s, _) = model(&case, case.upstream);
        let intact = state(&mut s);
        for (params, expected) in attempts {
            let reason = refused(&mut s, "feature.edit", params.clone());
            assert!(reason.contains(expected), "{params}: {reason}");
            assert_eq!(state(&mut s), intact, "{params}");
        }
    };
    try_all(
        "extrude: sketch",
        &[
            (json!({"feature": "D", "sketch": "Plate"}), "not a sketch"),
            (
                json!({"feature": "D", "sketch": "Nowhere"}),
                "no feature named",
            ),
            (json!({"feature": "D", "sketch": 7.5}), "feature id or name"),
            (
                json!({"feature": "D", "sketch": null}),
                "at least one parameter",
            ),
            (
                json!({"feature": "D", "source": "Rod"}),
                "not a parameter of a extrude",
            ),
            (json!({"feature": "D", "target": "Rod"}), "not a parameter"),
            (json!({"feature": "D", "op": "union"}), "not a parameter"),
        ],
    );
    try_all(
        "transform: source",
        &[
            (
                json!({"feature": "D", "source": "D"}),
                "cannot be made from itself",
            ),
            (json!({"feature": "D", "source": "F999"}), "no feature F999"),
            (json!({"feature": "D", "source": [1, 2]}), "feature id"),
            (json!({"feature": "D", "sketch": "Disc"}), "not a parameter"),
        ],
    );
    try_all(
        "boolean: tool",
        &[
            (json!({"feature": "D", "tool": "Plate"}), "same feature"),
            (json!({"feature": "D", "target": "Rod"}), "same feature"),
            (json!({"feature": "D", "tool": "D"}), "itself"),
            (
                json!({"feature": "D", "op": "xor"}),
                "union, subtract or intersect",
            ),
            (json!({"feature": "D", "op": 3}), "op"),
        ],
    );
    try_all(
        "revolve: axis (a datum axis)",
        &[
            (json!({"feature": "D", "axis": [0, 0, 1]}), "axis"),
            (json!({"feature": "D", "axis": "Shelf"}), "not a datum axis"),
            (json!({"feature": "D", "axis": "Nowhere"}), "axis"),
            (json!({"feature": "D", "sketch": "Lay"}), "not a sketch"),
        ],
    );
}

/// The other things `feature.edit` points a feature at: the operation of
/// a boolean, and the axis of a revolution as one of the sketch's own or
/// a line of it. Each gives the document built that way from the start.
#[test]
fn a_boolean_and_a_revolution_are_edited_into_what_they_could_have_been_built_as() {
    for op in ["union", "intersect"] {
        let mut edited = base();
        built(
            &mut edited,
            "feature.boolean",
            json!({"op": "subtract", "target": "Plate", "tool": "Rod", "name": "D"}),
        );
        built(
            &mut edited,
            "feature.edit",
            json!({"feature": "D", "op": op}),
        );
        let mut fresh = base();
        built(
            &mut fresh,
            "feature.boolean",
            json!({"op": op, "target": "Plate", "tool": "Rod", "name": "D"}),
        );
        assert_eq!(state(&mut edited), state(&mut fresh), "{op}");
    }
    // A sketch with a line of its own to turn about.
    let with_axis = |axis: Value| {
        let mut s = base();
        let line = run(
            &mut s,
            "sketch.line",
            json!({"sketch": "Disc", "from": [0, -5], "to": [10, -5], "construction": true}),
        );
        assert_eq!(line["entity"], "e5", "{line}");
        built(
            &mut s,
            "feature.revolve",
            json!({"sketch": "Disc", "axis": axis, "angle": "90 deg", "name": "D"}),
        );
        s
    };
    for axis in [json!("x"), json!("y"), json!("e5"), json!("Lay2")] {
        let mut edited = with_axis(json!("Lay"));
        built(
            &mut edited,
            "feature.edit",
            json!({"feature": "D", "axis": axis}),
        );
        assert_eq!(
            state(&mut edited),
            state(&mut with_axis(axis.clone())),
            "{axis}"
        );
        // A line that is not in the sketch is the feature's failure to
        // report, as when it is added.
        let failed = run(
            &mut edited,
            "feature.edit",
            json!({"feature": "D", "axis": "e77"}),
        );
        assert_eq!(failed["status"], "failed", "{failed}");
        let reason = failed["error"].as_str().unwrap();
        assert!(
            reason.starts_with("axis: ") && reason.contains("e77"),
            "{reason}"
        );
    }
}

/// A part with references of every kind between a dozen and a half
/// features, for moving them about. No body is wanted by two features, so
/// which features are built depends only on what comes before what.
///
/// A plate with a seat and a boss on its top, a shelf above it with a lid
/// sketched on it and a cover extruded from that, a rod cut out of the
/// plate, a pole along an edge of the plate with pegs in a ring about it,
/// the cover mirrored in the shelf, a post from below up to the plate.
fn workshop_in(workspace: Workspace) -> Session {
    let mut s = Session::new(workspace);
    run(&mut s, "file.new", json!({}));
    for (name, expr) in [("h", "5 mm"), ("lift", "12 mm"), ("n", "3")] {
        run(&mut s, "variable.set", json!({"name": name, "expr": expr}));
    }
    run(&mut s, "sketch.create", json!({"name": "Base"}));
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Base", "closed": true, "points": [[0, 0], [40, 0], [40, 20], [0, 20]]}),
    );
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Base", "distance": "h", "name": "Plate"}),
    );
    let top = face(&mut s, "Plate", "endCap");
    let floor = face(&mut s, "Plate", "startCap");
    let corner = [
        face(&mut s, "Plate", "side(e5)"),
        face(&mut s, "Plate", "side(e6)"),
    ];
    run(
        &mut s,
        "sketch.create",
        json!({"name": "Seat", "face": top}),
    );
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Seat", "center": [10, 10], "radius": 3}),
    );
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Seat", "distance": 4, "name": "Boss"}),
    );
    built(
        &mut s,
        "datum.plane",
        json!({"from": top, "distance": "lift", "name": "Shelf"}),
    );
    run(
        &mut s,
        "sketch.create",
        json!({"name": "Lid", "plane": "Shelf"}),
    );
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Lid", "closed": true, "points": [[0, 0], [10, 0], [10, 10], [0, 10]]}),
    );
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Lid", "distance": 2, "name": "Cover"}),
    );
    built(
        &mut s,
        "feature.cylinder",
        json!({"radius": 2, "height": 20, "position": [30, 10, -2], "name": "Rod"}),
    );
    built(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Plate", "tool": "Rod", "name": "Hole"}),
    );
    built(
        &mut s,
        "datum.axis",
        json!({"edge": corner, "name": "Pole"}),
    );
    built(
        &mut s,
        "feature.block",
        json!({"size": [2, 2, 2], "position": [50, 0, 0], "name": "Peg"}),
    );
    built(
        &mut s,
        "feature.pattern",
        json!({"source": "Peg", "count": "n", "angle": "90 deg", "axis": "Pole", "name": "Ring"}),
    );
    built(
        &mut s,
        "feature.mirror",
        json!({"source": "Cover", "plane": "Shelf", "keep": true, "name": "Twin"}),
    );
    run(
        &mut s,
        "sketch.create",
        json!({"name": "Under", "offset": -10}),
    );
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Under", "center": [5, 15], "radius": 2}),
    );
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Under", "upTo": floor, "name": "Post"}),
    );
    built(
        &mut s,
        "feature.block",
        json!({"size": [1, 2, 3], "position": [200, 0, 0], "name": "Bystander"}),
    );
    let check = run(&mut s, "document.check", json!({}));
    assert_eq!(check["ok"], true, "{check}");
    s
}

fn workshop() -> Session {
    static WORKSHOP: OnceLock<Session> = OnceLock::new();
    WORKSHOP
        .get_or_init(|| workshop_in(Workspace::none()))
        .clone()
}

/// The features of [`workshop`] in the order they are added, each with
/// the features it is made from: written down here from the description
/// of the part, not read from the document.
const WORKSHOP: [(&str, &[&str]); 16] = [
    ("Base", &[]),
    ("Plate", &["Base"]),
    ("Seat", &["Plate"]),
    ("Boss", &["Seat"]),
    ("Shelf", &["Plate"]),
    ("Lid", &["Shelf"]),
    ("Cover", &["Lid"]),
    ("Rod", &[]),
    ("Hole", &["Plate", "Rod"]),
    ("Pole", &["Plate"]),
    ("Peg", &[]),
    ("Ring", &["Peg", "Pole"]),
    ("Twin", &["Cover", "Shelf"]),
    ("Under", &[]),
    ("Post", &["Under", "Plate"]),
    ("Bystander", &[]),
];

/// The features that cannot be built in this order: one that comes before
/// something it is made from, and everything made from such a feature.
fn out_of_order(order: &[String]) -> Vec<&'static str> {
    let mut failed: Vec<&str> = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    for name in order {
        let (name, needs) = WORKSHOP.iter().find(|(n, _)| n == name).unwrap();
        if needs
            .iter()
            .any(|need| !seen.contains(need) || failed.contains(need))
        {
            failed.push(name);
        }
        seen.push(name);
    }
    failed
}

/// `feature.move` takes any position, and the features that are then out
/// of order fail in the documented way: a feature that comes before
/// something it is made from says so (and how to mend it), what is made
/// from a failed feature names it, and everything else is built as
/// before. Every feature of the part is moved to every position.
#[test]
fn a_feature_moved_to_any_position_fails_exactly_what_is_out_of_order() {
    let original = state(&mut workshop());
    assert_eq!(
        names_in_order(&original),
        WORKSHOP.map(|(name, _)| name.to_string())
    );
    let moved = map_parallel((0..WORKSHOP.len()).collect(), |from| {
        let mut s = workshop();
        let intact = state(&mut s);
        let name = WORKSHOP[from].0;
        for to in 0..WORKSHOP.len() {
            let context = format!("{name} to {to}");
            let steps = undo_steps(&mut s);
            let result = run(
                &mut s,
                "feature.move",
                json!({"feature": name, "position": to}),
            );
            let now = state(&mut s);
            let order = names_in_order(&now);
            assert_eq!(order[to], name, "{context}");
            let expected = out_of_order(&order);
            for (feature, needs) in WORKSHOP {
                let (status, reason) = status(&now, feature);
                let fails = expected.contains(&feature);
                assert_eq!(status == "failed", fails, "{context}: {feature}: {reason}");
                if fails {
                    // The reason names a feature it needs: one that comes
                    // after it, or one that failed.
                    let at = order.iter().position(|n| n == feature).unwrap();
                    let said = needs.iter().any(|need| {
                        let later = order.iter().position(|n| n == need).unwrap() > at;
                        reason.contains(&format!("`{need}`"))
                            && if later {
                                reason.contains("comes after") && reason.contains("feature.move")
                            } else {
                                reason.contains("fix it first")
                            }
                    });
                    assert!(said, "{context}: {feature}: {reason}");
                } else {
                    // Built as before: the same body where it still has
                    // one of its own.
                    if let (Some(body), Some(before)) =
                        (body_of(&now, feature), body_of(&original, feature))
                    {
                        assert_eq!(body, before, "{context}: {feature}");
                    }
                    assert_eq!(
                        feature_without_taker(&now, feature),
                        feature_without_taker(&original, feature),
                        "{context}"
                    );
                }
            }
            assert_eq!(result["status"] == "failed", expected.contains(&name));
            every_failure_has_a_reason(&now, &context);
            independent_features_are_untouched(&s, &context);
            if to == from {
                // Where it is already: no change, and nothing to undo.
                assert_eq!(now, intact, "{context}");
                assert_eq!(undo_steps(&mut s), steps, "{context}");
                continue;
            }
            // Undo restores (looked at for a quarter of the moves), and
            // so does moving it back.
            if (from + to) % 4 == 0 {
                run(&mut s, "edit.undo", json!({}));
                assert_eq!(state(&mut s), intact, "{context}: after undo");
                run(&mut s, "edit.redo", json!({}));
                assert_eq!(state(&mut s), now, "{context}: after redo");
            }
            run(
                &mut s,
                "feature.move",
                json!({"feature": name, "position": from}),
            );
            assert_eq!(state(&mut s), intact, "{context}: moved back");
        }
        WORKSHOP.len()
    });
    assert_eq!(moved.iter().sum::<usize>(), 256);
    // Positions past the end are refused.
    let mut s = workshop();
    let reason = refused(
        &mut s,
        "feature.move",
        json!({"feature": "Plate", "position": 16}),
    );
    assert!(reason.contains("position"), "{reason}");
}

/// A feature's entry of `document.inspect` without who took its body:
/// that changes when the taker is not built.
fn feature_without_taker(document: &Value, name: &str) -> Value {
    let mut entry = feature(document, name).clone();
    entry.as_object_mut().unwrap().remove("bodyUsedBy");
    entry
}

/// The rollback marker on every feature of the part, as built and with
/// failed and suppressed features in it: the features up to the marker
/// are those of the document that ends there.
#[test]
fn rolled_back_to_any_feature_the_document_is_the_one_that_ends_there() {
    type Prepare = fn(&mut Session);
    let variants: Vec<(&str, Prepare)> = vec![
        ("as built", |_| {}),
        ("with failures", |s| {
            run(s, "feature.suppress", json!({"feature": "Shelf"}));
            run(s, "feature.delete", json!({"feature": "Rod"}));
            run(
                s,
                "feature.move",
                json!({"feature": "Under", "position": 14}),
            );
        }),
    ];
    map_parallel(variants, |(variant, prepare)| {
        let mut s = workshop();
        prepare(&mut s);
        let whole = state(&mut s);
        for name in names_in_order(&whole) {
            let context = format!("{variant}, rolled back to {name}");
            run(&mut s, "history.rollback", json!({"feature": name}));
            let rolled = state(&mut s);
            assert_eq!(rolled["rollback"], feature(&rolled, &name)["id"]);
            rolled_back_is_the_document_that_ends_there(&s, &context);
            every_failure_has_a_reason(&rolled, &context);
            independent_features_are_untouched(&s, &context);
            run(&mut s, "edit.undo", json!({}));
            run(&mut s, "edit.redo", json!({}));
            assert_eq!(state(&mut s), rolled, "{context}: after undo and redo");
        }
        run(&mut s, "history.rollback", json!({"feature": null}));
        assert_eq!(state(&mut s), whole, "{variant}: rolled forward");
    });
}

fn workdir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("failed-features-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn on_disk(dir: &Path) -> Workspace {
    Workspace {
        read: Access::Root(dir.to_path_buf()),
        write: Access::Root(dir.to_path_buf()),
    }
}

/// Saved and opened again in a session that knows nothing, the document
/// is the same: features, statuses, reasons, bodies, the marker.
fn saved_and_reopened_is_the_same(session: &Session, dir: &Path, context: &str) {
    let mut held = session.clone();
    let before = state(&mut held);
    run(
        &mut held,
        "file.save",
        json!({"path": "copy.3dcraft", "overwrite": true}),
    );
    let mut reopened = Session::new(on_disk(dir));
    run(&mut reopened, "file.open", json!({"path": "copy.3dcraft"}));
    assert_eq!(
        state(&mut reopened),
        before,
        "{context}: saved and reopened"
    );
}

/// A document with failed, suppressed and rolled-back features is the
/// same after saving and opening it: the states, the reasons, the bodies.
/// Rolled forward and repaired after opening, it is what the document
/// that was never saved becomes.
#[test]
fn failed_suppressed_and_rolled_back_features_survive_saving_and_opening() {
    let dir = workdir("saved");
    let mut s = workshop_in(on_disk(&dir));
    run(&mut s, "feature.suppress", json!({"feature": "Peg"}));
    run(&mut s, "feature.delete", json!({"feature": "Rod"}));
    run(&mut s, "variable.delete", json!({"name": "lift"}));
    run(&mut s, "history.rollback", json!({"feature": "Ring"}));
    let saved = state(&mut s);
    let states: Vec<(String, String)> = names_in_order(&saved)
        .iter()
        .map(|name| (name.clone(), status(&saved, name).0))
        .collect();
    let count = |wanted: &str| states.iter().filter(|(_, s)| s == wanted).count();
    // Shelf (its variable), Lid and Cover (the shelf), Hole (its tool),
    // Ring (its source); Peg; Twin, Under, Post, Bystander.
    assert_eq!(
        (count("failed"), count("suppressed"), count("rolledBack")),
        (5, 1, 4),
        "{states:?}"
    );
    assert_eq!(saved["ok"], false);
    run(&mut s, "file.save", json!({"path": "part.3dcraft"}));
    let mut reopened = Session::new(on_disk(&dir));
    run(&mut reopened, "file.open", json!({"path": "part.3dcraft"}));
    assert_eq!(state(&mut reopened), saved);
    let check = |s: &mut Session| run(s, "document.check", json!({}));
    assert_eq!(check(&mut reopened), check(&mut s));
    // The same edits on both: rolled forward, then repaired.
    for session in [&mut s, &mut reopened] {
        run(session, "history.rollback", json!({"feature": null}));
        run(
            session,
            "variable.set",
            json!({"name": "lift", "expr": "12 mm"}),
        );
        run(
            session,
            "feature.suppress",
            json!({"feature": "Peg", "suppressed": false}),
        );
        built(
            session,
            "feature.cylinder",
            json!({"radius": 2, "height": 20, "position": [30, 10, -2], "name": "Rod2"}),
        );
        // The new rod comes last: it is moved before the hole it cuts.
        run(
            session,
            "feature.move",
            json!({"feature": "Rod2", "position": 7}),
        );
        built(
            session,
            "feature.edit",
            json!({"feature": "Hole", "tool": "Rod2"}),
        );
    }
    let repaired = state(&mut s);
    assert_eq!(repaired["ok"], true, "{repaired}");
    assert_eq!(state(&mut reopened), repaired);
    // The repaired part is the part as it was first built, but for the
    // rod, which is another feature now.
    let original = state(&mut workshop());
    for (name, _) in WORKSHOP {
        if name != "Rod" && name != "Hole" {
            assert_eq!(feature(&repaired, name), feature(&original, name));
            assert_eq!(body_of(&repaired, name), body_of(&original, name));
        }
    }
    let measured = |document: &Value| {
        let hole = body_of(document, "Hole").unwrap();
        (
            hole["volume"].clone(),
            hole["area"].clone(),
            hole["bbox"].clone(),
        )
    };
    assert_eq!(measured(&repaired), measured(&original));
}

/// xorshift64*: a seeded sequence, the same on every platform.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len() as u64) as usize]
    }
}

/// One random edit of the [`workshop`]: breaking something (delete,
/// suppress, move, roll back, a variable, a dimension), or pointing a
/// reference at something else, which repairs it or breaks it another
/// way.
fn random_edit(rng: &mut Rng) -> (&'static str, Value) {
    const FEATURES: [&str; 18] = [
        "Base",
        "Plate",
        "Seat",
        "Boss",
        "Shelf",
        "Lid",
        "Cover",
        "Rod",
        "Hole",
        "Pole",
        "Peg",
        "Ring",
        "Twin",
        "Under",
        "Post",
        "Bystander",
        "F17",
        "F18",
    ];
    const SKETCHES: [&str; 5] = ["Base", "Seat", "Lid", "Under", "Plate"];
    const BODIES: [&str; 10] = [
        "Plate",
        "Boss",
        "Cover",
        "Rod",
        "Hole",
        "Peg",
        "Ring",
        "Twin",
        "Post",
        "Bystander",
    ];
    // Faces that are there, faces that are there while their feature is,
    // and faces that never were.
    const FACES: [&str; 14] = [
        "F2:endCap()",
        "F2:startCap()",
        "F2:side(e5)",
        "F2:side(e6)",
        "F2:side(e99)",
        "F4:endCap()",
        "F7:endCap()",
        "F7:side(e5)",
        "F8:face(0)",
        "F8:face(1)",
        "F9:copy(0)/F2:endCap()",
        "F9:copy(1)/F8:face(0)",
        "F13:copy(0)/F7:endCap()",
        "F16:face(1)",
    ];
    let feature = rng.pick(&FEATURES);
    let number = rng.below(60) as f64 / 4.0 - 3.0;
    match rng.below(24) {
        0 => (
            "feature.suppress",
            json!({"feature": feature, "suppressed": rng.below(2) == 0}),
        ),
        1 | 2 => (
            "feature.move",
            json!({"feature": feature, "position": rng.below(17)}),
        ),
        3 => (
            "history.rollback",
            json!({"feature": if rng.below(3) == 0 { Value::Null } else { feature.into() }}),
        ),
        4 | 5 => ("edit.undo", json!({})),
        6 => ("edit.redo", json!({})),
        7 => (
            "feature.delete",
            json!({"feature": if rng.below(3) == 0 { feature } else { "F19" }}),
        ),
        8 => (
            "variable.set",
            json!({"name": rng.pick(&["h", "lift"]),
                   "expr": rng.pick(&["5 mm", "12 mm", "0 mm", "-3 mm", "nope", "1 +", "h * 2"])}),
        ),
        9 => (
            "variable.set",
            json!({"name": "n", "expr": rng.pick(&["3", "2", "0", "h", "2.5"])}),
        ),
        10 => (
            "variable.delete",
            json!({"name": if rng.below(3) == 0 { rng.pick(&["h", "lift", "n"]) } else { "gone" }}),
        ),
        11 => (
            "sketch.edit",
            json!({"sketch": rng.pick(&["Seat", "Lid", "Under"]), "face": rng.pick(&FACES)}),
        ),
        12 => (
            "sketch.edit",
            json!({"sketch": rng.pick(&["Seat", "Lid", "Under"]),
                   "plane": rng.pick(&["Shelf", "xy", "zx", "Pole"])}),
        ),
        13 => (
            "feature.edit",
            json!({"feature": rng.pick(&["Boss", "Cover", "Post"]), "sketch": rng.pick(&SKETCHES)}),
        ),
        14 => {
            let mut params = json!({"feature": "Hole"});
            params[rng.pick(&["target", "tool"])] = rng.pick(&BODIES).into();
            if rng.below(3) == 0 {
                params["op"] = rng.pick(&["union", "subtract", "intersect"]).into();
            }
            ("feature.edit", params)
        }
        15 => (
            "feature.edit",
            json!({"feature": rng.pick(&["Ring", "Twin", "F17"]), "source": rng.pick(&BODIES)}),
        ),
        16 => (
            "feature.edit",
            if rng.below(2) == 0 {
                json!({"feature": "Ring", "axis": if rng.below(2) == 0 { json!("Pole") } else { json!([0, 0, 1]) }})
            } else if rng.below(2) == 0 {
                json!({"feature": "Twin", "plane": rng.pick(&["Shelf", "Pole"])})
            } else {
                json!({"feature": "Twin", "normal": [0, 0, 1], "point": [0, 0, number]})
            },
        ),
        17 => (
            "feature.edit",
            match rng.below(3) {
                0 => json!({"feature": "Post", "upTo": rng.pick(&FACES)}),
                1 => json!({"feature": "Post", "throughAll": true, "reversed": rng.below(2) == 0}),
                _ => json!({"feature": "Post", "distance": number}),
            },
        ),
        18 => (
            "datum.plane",
            json!({"replace": "Shelf", "from": rng.pick(&[
                "xy", "zx", "F2:endCap()", "F2:side(e5)", "F8:face(1)", "F8:face(0)", "F2:side(e99)",
            ]), "distance": rng.pick(&["lift", "2", "h"])}),
        ),
        19 => (
            "datum.axis",
            match rng.below(3) {
                0 => json!({"replace": "Pole", "edge": [rng.pick(&FACES), rng.pick(&FACES)]}),
                1 => json!({"replace": "Pole", "face": rng.pick(&FACES)}),
                _ => json!({"replace": "Pole", "through": [[40, 0, 0], [40, 0, number]]}),
            },
        ),
        20 => (
            "feature.edit",
            json!({"feature": rng.pick(&["Plate", "Boss", "Cover"]),
                   "distance": format!("{number} mm")}),
        ),
        21 => (
            "sketch.edit",
            json!({"sketch": rng.pick(&["Base", "Seat", "Lid", "Under"]),
                   "entity": format!("e{}", 1 + rng.below(4)), "at": [number * 5.0, number]}),
        ),
        22 => (
            "feature.transform",
            json!({"source": rng.pick(&BODIES), "translate": [0, 0, number * 10.0],
                   "keep": rng.below(2) == 0}),
        ),
        _ => (
            "feature.rename",
            json!({"feature": feature, "name": feature}),
        ),
    }
}

/// How many of the steps of the walks were refused, and how many left a
/// failed feature behind: the walk has to be made of edits that happen.
#[derive(Default)]
struct Walked {
    steps: usize,
    refused: usize,
    with_failures: usize,
    repaired: usize,
}

fn walk(seed: u64, steps: usize) -> Walked {
    let dir = workdir(&format!("walk-{seed}"));
    let mut s = workshop_in(on_disk(&dir));
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut walked = Walked::default();
    let mut failing = 0;
    for step in 0..steps {
        let (id, params) = random_edit(&mut rng);
        let context = format!("seed {seed}, step {step}, after {id} {params}");
        walked.steps += 1;
        // A refused command (a feature that was deleted, nothing to undo,
        // a face name that is not a face) is fine; it changes nothing.
        if let Err(error) = s.execute(id, &params) {
            assert!(
                !error.message().contains("internal error"),
                "{context}: {error}"
            );
            walked.refused += 1;
            continue;
        }
        assert_ne!(
            s.regeneration_matches_a_full_one(),
            Some(false),
            "{context}"
        );
        let now = state(&mut s);
        every_failure_has_a_reason(&now, &context);
        let failed = now["features"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| f["status"] == "failed")
            .count();
        walked.with_failures += usize::from(failed > 0);
        walked.repaired += usize::from(failed < failing);
        failing = failed;
        independent_features_are_untouched(&s, &context);
        rolled_back_is_the_document_that_ends_there(&s, &context);
        if step % 4 == 0 {
            undo_and_redo_restore(&s, &context);
        }
        if step % 16 == 0 {
            saved_and_reopened_is_the_same(&s, &dir, &context);
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    walked
}

/// Random deletes, suppressions, moves, rollbacks, edits and repairs of a
/// part of sixteen features with references between them. After every
/// step: what the session holds is what regenerating from nothing gives;
/// no feature fails without a reason; every feature that did not fail is
/// what it is in the document without the failed ones; what is before the
/// rollback marker is the document that ends there. Every few steps: undo
/// and redo restore it, and saved and opened again it is the same.
#[test]
fn a_random_walk_of_breaking_and_repairing_keeps_every_promise() {
    let walks = map_parallel((1..=16u64).collect(), |seed| walk(seed, 160));
    let total = walks.iter().fold(Walked::default(), |sum, w| Walked {
        steps: sum.steps + w.steps,
        refused: sum.refused + w.refused,
        with_failures: sum.with_failures + w.with_failures,
        repaired: sum.repaired + w.repaired,
    });
    println!(
        "{} steps, {} refused, {} left a failed feature, {} had fewer failures than the step before",
        total.steps, total.refused, total.with_failures, total.repaired
    );
    // The walk is made of edits that happen, breaks things and mends them.
    assert!(total.refused * 2 < total.steps);
    assert!(total.with_failures * 4 > total.steps);
    assert!(total.repaired * 50 > total.steps);
}

/// One walk, to look at a failure of the test above:
/// `WALK="seed steps" cargo test -p craft3d-engine --test failed_features one_walk -- --ignored --nocapture`.
#[test]
#[ignore = "reproduces one seed of the random walk"]
fn one_walk() {
    let spec = std::env::var("WALK").expect("WALK=\"seed steps\"");
    let mut parts = spec.split_whitespace().map(|n| n.parse::<u64>().unwrap());
    let (seed, steps) = (parts.next().unwrap(), parts.next().unwrap_or(160));
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    for step in 0..steps {
        let (id, params) = random_edit(&mut rng);
        println!("{step}: {id} {params}");
    }
    walk(seed, steps as usize);
}

/// A reference pointed at something that is itself made from the feature:
/// neither can be built, in either order. The reason says so, and does
/// not send the user to `feature.move`, which would only turn the failure
/// round. Pointing either of the two at something else is the repair.
#[test]
fn a_reference_to_something_made_from_the_feature_itself_says_so() {
    let mut s = workshop();
    let intact = state(&mut s);
    // (the edit that closes the circle, the feature it fails, what that
    // feature now needs, what else fails for it)
    let circles: [(&str, Value, &str, &str, &[&str]); 5] = [
        (
            // The shelf on the cover that is extruded from a sketch on it.
            "datum.plane",
            json!({"replace": "Shelf", "from": "F7:endCap()", "distance": 1}),
            "Shelf",
            "Cover",
            &["Lid", "Cover", "Twin"],
        ),
        (
            // The seat on the boss that is extruded from it.
            "sketch.edit",
            json!({"sketch": "Seat", "face": "F4:endCap()"}),
            "Seat",
            "Boss",
            &["Boss"],
        ),
        (
            // The plate from the sketch that lies on it.
            "feature.edit",
            json!({"feature": "Plate", "sketch": "Seat"}),
            "Plate",
            "Seat",
            &[
                "Seat", "Boss", "Shelf", "Lid", "Cover", "Hole", "Pole", "Ring", "Twin", "Post",
            ],
        ),
        (
            // The pole along the axis of the ring that is turned about it.
            "datum.axis",
            json!({"replace": "Pole", "face": "F12:copy(0)/F11:face(0)"}),
            "Pole",
            "Ring",
            &["Ring"],
        ),
        (
            // The cover's mirror image as what the hole is cut out of, and
            // the cover up to the hole.
            "feature.edit",
            json!({"feature": "Cover", "upTo": "F9:copy(0)/F2:endCap()"}),
            "Cover",
            "Hole",
            &["Twin", "Hole"],
        ),
    ];
    for (id, params, feature, needs, others) in circles {
        let context = format!("{id} {params}");
        if feature == "Cover" {
            run(
                &mut s,
                "feature.edit",
                json!({"feature": "Hole", "target": "Twin"}),
            );
        }
        run(&mut s, id, params);
        let now = state(&mut s);
        let (status_now, reason) = status(&now, feature);
        assert_eq!(status_now, "failed", "{context}");
        assert!(
            reason.contains(&format!("`{needs}`"))
                && reason.contains("is itself made from this feature")
                && !reason.contains("feature.move"),
            "{context}: {reason}"
        );
        for other in others {
            let (status_now, reason) = status(&now, other);
            assert_eq!(status_now, "failed", "{context}: {other}");
            // Made from a failed feature, or another link of the circle.
            assert!(
                reason.contains("fix it first")
                    || reason.contains("is itself made from this feature"),
                "{context}: {other}: {reason}"
            );
        }
        every_failure_has_a_reason(&now, &context);
        independent_features_are_untouched(&s, &context);
        while state(&mut s) != intact {
            run(&mut s, "edit.undo", json!({}));
        }
    }
}

/// Deleting the feature the rollback marker is on leaves what was rolled
/// back rolled back: the marker goes to the feature before.
#[test]
fn deleting_the_feature_at_the_rollback_marker_keeps_the_rest_rolled_back() {
    let mut s = workshop();
    run(&mut s, "history.rollback", json!({"feature": "Cover"}));
    let rolled = state(&mut s);
    let deleted = run(&mut s, "feature.delete", json!({"feature": "Cover"}));
    assert_eq!(deleted["deleted"]["name"], "Cover");
    let now = state(&mut s);
    assert_eq!(now["rollback"], feature(&now, "Lid")["id"]);
    for name in names_in_order(&now) {
        assert_eq!(feature(&now, &name), feature(&rolled, &name), "{name}");
    }
    assert_eq!(now["ok"], true);
    rolled_back_is_the_document_that_ends_there(&s, "the marked feature deleted");
    // A feature added now goes in right after the marker, which moves
    // onto it: it is built, and what was rolled back stays rolled back
    // after it (tests/insert_at_marker.rs holds every command family to
    // this).
    let added = run(
        &mut s,
        "feature.block",
        json!({"size": [1, 1, 1], "name": "Late"}),
    );
    assert_eq!(added["status"], "ok", "{added}");
    let with_late = inspect(&mut s);
    let order = names_in_order(&with_late);
    let lid = order.iter().position(|n| n == "Lid").unwrap();
    assert_eq!(order[lid + 1], "Late", "{order:?}");
    assert_eq!(with_late["rollback"], feature(&with_late, "Late")["id"]);
    assert_eq!(status(&with_late, "Twin").0, "rolledBack");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(state(&mut s), now);
    // Rolled forward, the mirror of the cover has lost its source.
    run(&mut s, "history.rollback", json!({"feature": null}));
    let (twin, reason) = status(&inspect(&mut s), "Twin");
    assert_eq!(twin, "failed");
    assert!(
        reason.starts_with("source: ") && reason.contains("F7"),
        "{reason}"
    );
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(state(&mut s), rolled);
    // The first feature with the marker on it: there is none before it,
    // and the marker is cleared.
    run(&mut s, "history.rollback", json!({"feature": "Base"}));
    run(&mut s, "feature.delete", json!({"feature": "Base"}));
    let now = state(&mut s);
    assert_eq!(now["rollback"], Value::Null);
    assert_eq!(status(&now, "Bystander").0, "ok");
    assert_eq!(status(&now, "Plate").0, "failed");
}

/// A face given by the name of an earlier body is followed through the
/// features that took the body (docs/architecture.md §4.1 › *References
/// that follow a body*). Two ways that can fail are new: a feature on the
/// way took the face away, or made more than one face of it. Either fails
/// the feature with the feature that did, the feature made from it waits
/// for it, nothing else is touched, and the repaired feature is the one
/// built that way from the start.
#[test]
fn a_name_followed_to_a_body_that_lost_it_or_has_it_twice_fails_naming_the_feature_on_the_way() {
    let block = |s: &mut Session| {
        run(s, "file.new", json!({"units": "mm"}));
        built(
            s,
            "feature.block",
            json!({"size": [40, 20, 10], "name": "Base"}),
        );
    };
    type Setup = fn(&mut Session);
    let ways: Vec<(&str, Setup, &[&str])> = vec![
        (
            "taken away",
            |s| {
                built(
                    s,
                    "feature.fillet",
                    json!({"edges": [["F1:face(1)", "F1:face(2)"]], "radius": 2, "name": "Round"}),
                );
                built(
                    s,
                    "feature.deleteFace",
                    json!({"faces": ["F2:fillet(0)"], "name": "Sharp"}),
                );
            },
            &[
                "edges.0: the face F2:fillet(0) is gone",
                "`Sharp`, a deleteFace",
            ],
        ),
        (
            "made twice",
            |s| {
                built(
                    s,
                    "feature.pattern",
                    json!({"source": "Base", "count": 2, "step": [60, 0, 0], "name": "Row"}),
                );
            },
            &[
                "edges.0: the face F1:face(4) is more than one face after `Row`",
                "F2:copy(0)/F1:face(4), F2:copy(1)/F1:face(4)",
            ],
        ),
    ];
    for (way, setup, expected) in ways {
        let mut s = Session::new(Workspace::default());
        block(&mut s);
        setup(&mut s);
        let blend =
            json!({"edges": [["F1:face(4)", "F2:fillet(0)"]], "distance": 1, "name": "Next"});
        let blend = if way == "made twice" {
            json!({"edges": [["F1:face(4)", "F1:face(1)"]], "distance": 1, "name": "Next"})
        } else {
            blend
        };
        let result = run(&mut s, "feature.chamfer", blend);
        assert_eq!(result["status"], "failed", "{way}: {result}");
        run(
            &mut s,
            "feature.shell",
            json!({"source": "Next", "thickness": 1, "name": "After"}),
        );
        let document = state(&mut s);
        let (now, reason) = status(&document, "Next");
        assert_eq!(now, "failed", "{way}");
        for part in expected {
            assert!(reason.contains(part), "{way}: {reason}");
        }
        let (now, reason) = status(&document, "After");
        assert_eq!(now, "failed", "{way}");
        assert!(
            reason.contains("`Next` failed; fix it first"),
            "{way}: {reason}"
        );
        every_failure_has_a_reason(&document, way);
        independent_features_are_untouched(&s, way);
        undo_and_redo_restore(&s, way);
        // Repaired by naming what is there: the edge between the left
        // face and the top, of the last instance for the pattern.
        let edge = if way == "made twice" {
            json!([["F2:copy(1)/F1:face(4)", "F2:copy(1)/F1:face(1)"]])
        } else {
            json!([["F1:face(4)", "F1:face(1)"]])
        };
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Next", "edges": edge}),
        );
        let mut fresh = Session::new(Workspace::default());
        block(&mut fresh);
        setup(&mut fresh);
        built(
            &mut fresh,
            "feature.chamfer",
            json!({"edges": edge, "distance": 1, "name": "Next"}),
        );
        run(
            &mut fresh,
            "feature.shell",
            json!({"source": "Next", "thickness": 1, "name": "After"}),
        );
        assert_eq!(state(&mut s), state(&mut fresh), "{way}");
    }
}
