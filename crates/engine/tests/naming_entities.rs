//! The topological naming suite for edges and vertices
//! (docs/testing.md): every edge and every vertex of every body has a
//! persistent name of its own, the name stays on the same edge or corner
//! through upstream edits that keep the topology, and a reference to one
//! that an edit removes fails with the name it lost. It never ends up on
//! another edge or corner.
//!
//! The consumer here is a datum point at a vertex (`datum.point`), whose
//! position says where the name ended up; edges are followed by the
//! faces, the curve and the end points `document.edges` lists for a name.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{built, new_document, run, session};
use craft3d_engine::{Access, Session, Workspace};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

type V = [f64; 3];

fn workdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "craft3d-naming-entities-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A command that only asks: it changes nothing, so there is nothing to
/// compare with a full regeneration.
fn ask(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn vector(value: &Value) -> V {
    let list = value.as_array().unwrap_or_else(|| panic!("{value}"));
    [
        list[0].as_f64().unwrap(),
        list[1].as_f64().unwrap(),
        list[2].as_f64().unwrap(),
    ]
}

fn distance(a: V, b: V) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

fn texts(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|list| {
            list.iter()
                .map(|v| v.as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn body(session: &mut Session, name: &str) -> Value {
    let document = ask(session, "document.inspect", json!({}));
    document["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == name)
        .unwrap_or_else(|| panic!("no body {name} in {}", document["features"]))
        .clone()
}

fn feature(session: &mut Session, name: &str) -> Value {
    let document = ask(session, "document.inspect", json!({}));
    document["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == name)
        .unwrap_or_else(|| panic!("no feature {name}"))
        .clone()
}

fn id_of(session: &mut Session, name: &str) -> String {
    feature(session, name)["id"].as_str().unwrap().to_string()
}

/// A vertex as `datum.references` lists it.
#[derive(Clone, Debug, PartialEq)]
struct Corner {
    name: String,
    aliases: Vec<String>,
    at: V,
}

/// An edge as `document.edges` lists it.
#[derive(Clone, Debug, PartialEq)]
struct Rim {
    name: String,
    aliases: Vec<String>,
    /// The names of the faces on its sides, ascending as text.
    faces: Vec<String>,
    curve: String,
    start: V,
    end: V,
}

fn corners(session: &mut Session, feature: &str) -> Vec<Corner> {
    let listed = ask(session, "datum.references", json!({"feature": feature}));
    assert_eq!(listed["truncated"], false);
    listed["vertices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| Corner {
            name: v["name"].as_str().unwrap().to_string(),
            aliases: texts(&v["aliases"]),
            at: vector(&v["at"]),
        })
        .collect()
}

fn rims(session: &mut Session, feature: &str) -> Vec<Rim> {
    let listed = ask(session, "document.edges", json!({"feature": feature}));
    let edges = listed["edges"].as_array().unwrap();
    assert_eq!(listed["count"], edges.len());
    edges
        .iter()
        .map(|e| {
            let mut faces = texts(&e["faces"]);
            faces.sort();
            Rim {
                name: e["name"]
                    .as_str()
                    .unwrap_or_else(|| panic!("{feature}: an edge without a name: {e}"))
                    .to_string(),
                aliases: texts(&e["aliases"]),
                faces,
                curve: e["curve"].as_str().unwrap().to_string(),
                start: vector(&e["start"]),
                end: vector(&e["end"]),
            }
        })
        .collect()
}

/// Every name the entities answer to, each mapped to the index of its
/// entity; panics when two entities answer to one name.
fn index_names<'a>(
    what: &str,
    feature: &str,
    names: impl Iterator<Item = (usize, &'a String)>,
) -> BTreeMap<String, usize> {
    let mut seen = BTreeMap::new();
    for (index, name) in names {
        if let Some(other) = seen.insert(name.clone(), index) {
            panic!("{feature}: {what} {other} and {index} are both called {name}");
        }
    }
    seen
}

/// Where the datum point `Probe` lies when it is put on the vertex
/// `name`, or why it does not lie anywhere.
fn probe(session: &mut Session, name: &str) -> Result<V, String> {
    let there = ask(session, "document.inspect", json!({}))["features"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["name"] == "Probe");
    let mut params = json!({"vertex": name});
    params[if there { "replace" } else { "name" }] = "Probe".into();
    let result = run(session, "datum.point", params);
    match result["status"].as_str() {
        Some("ok") => Ok(vector(&result["datum"]["at"])),
        _ => Err(result["error"].as_str().unwrap().to_string()),
    }
}

/// `F7:corner[a,b,c]` as `F7:corner[c,b,a]`, for a name that lists
/// faces and nothing after them.
fn faces_reversed(name: &str) -> Option<String> {
    let (head, rest) = name.split_once(":corner[")?;
    let listed = rest.strip_suffix(']')?;
    if head.contains('/') || listed.contains('|') {
        return None;
    }
    // Cut at the commas between names, not at those inside `(e1,e2)`.
    let (mut faces, mut depth, mut from) = (Vec::new(), 0, 0);
    for (at, c) in listed.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                faces.push(&listed[from..at]);
                from = at + 1;
            }
            _ => {}
        }
    }
    faces.push(&listed[from..]);
    faces.reverse();
    Some(format!("{head}:corner[{}]", faces.join(",")))
}

/// Completeness and uniqueness: the body of `feature` has as many names
/// as it has edges and vertices, no two edges answer to one name and no
/// two vertices do, an edge named by its faces lists the faces it lies
/// between, and a datum point at every vertex name (aliases too) lies on
/// that vertex. Returns the vertices and edges.
fn assert_named(session: &mut Session, feature: &str) -> (Vec<Corner>, Vec<Rim>) {
    let solid = body(session, feature);
    let (corners, rims) = (corners(session, feature), rims(session, feature));
    assert_eq!(solid["vertices"], corners.len(), "{feature}: vertices");
    assert_eq!(solid["edges"], rims.len(), "{feature}: edges");
    let face_names: BTreeSet<String> = texts(&solid["faceNames"]).into_iter().collect();
    let all = |own: &'_ String, aliases: &'_ Vec<String>| {
        std::iter::once(own.clone())
            .chain(aliases.iter().cloned())
            .collect::<Vec<_>>()
    };
    let vertex_names: Vec<(usize, String)> = corners
        .iter()
        .enumerate()
        .flat_map(|(i, c)| all(&c.name, &c.aliases).into_iter().map(move |n| (i, n)))
        .collect();
    index_names(
        "vertices",
        feature,
        vertex_names.iter().map(|(i, n)| (*i, n)),
    );
    let edge_names: Vec<(usize, String)> = rims
        .iter()
        .enumerate()
        .flat_map(|(i, r)| all(&r.name, &r.aliases).into_iter().map(move |n| (i, n)))
        .collect();
    index_names("edges", feature, edge_names.iter().map(|(i, n)| (*i, n)));
    for rim in &rims {
        for face in &rim.faces {
            assert!(face_names.contains(face), "{feature}: {face}");
        }
        // Named by its faces, an edge lists exactly those on its sides:
        // twice the same for a seam.
        // (A copy of such an edge lists the faces of the body it is a
        // copy from.)
        let own = rim.name.split_once(":between[");
        if let Some(rest) = own
            .filter(|(made_by, _)| !made_by.contains('/'))
            .map(|(_, rest)| rest)
        {
            let listed = rest.split(['|', ']']).next().unwrap();
            let mut sides = rim.faces.clone();
            if sides.len() == 1 {
                sides.push(sides[0].clone());
            }
            let either = [sides.join(","), format!("{},{}", sides[1], sides[0])];
            assert!(
                either.iter().any(|order| order == listed),
                "{feature}: {}",
                rim.name
            );
        }
    }
    for (index, name) in &vertex_names {
        let at = probe(session, name).unwrap_or_else(|e| panic!("{feature}: {name}: {e}"));
        assert_eq!(at, corners[*index].at, "{feature}: {name}");
        // Typed with its faces the other way round it is the same name.
        if let Some(typed) = faces_reversed(name) {
            let at = probe(session, &typed).unwrap_or_else(|e| panic!("{feature}: {typed}: {e}"));
            assert_eq!(at, corners[*index].at, "{feature}: {typed}");
        }
    }
    // The edges of a name between two faces are among those between them.
    for rim in &rims {
        let pair = match rim.faces.as_slice() {
            [one] => [one.clone(), one.clone()],
            [a, b] => [a.clone(), b.clone()],
            other => panic!("{feature}: {} lies on {other:?}", rim.name),
        };
        let between = ask(
            session,
            "document.edges",
            json!({"feature": feature, "between": pair}),
        );
        assert!(
            between["edges"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["name"] == rim.name.as_str()),
            "{feature}: {}",
            rim.name
        );
    }
    if !corners.is_empty() {
        run(session, "feature.delete", json!({"feature": "Probe"}));
    }
    (corners, rims)
}

fn sketch(session: &mut Session, name: &str, plane: &str, offset: f64) {
    run(
        session,
        "sketch.create",
        json!({"name": name, "plane": plane, "offset": offset}),
    );
}

fn rectangle(session: &mut Session, sketch: &str, x: (f64, f64), y: (f64, f64)) {
    run(
        session,
        "sketch.polyline",
        json!({"sketch": sketch, "closed": true,
               "points": [[x.0, y.0], [x.1, y.0], [x.1, y.1], [x.0, y.1]]}),
    );
}

fn circle(session: &mut Session, sketch: &str, center: [f64; 2], radius: f64) {
    run(
        session,
        "sketch.circle",
        json!({"sketch": sketch, "center": center, "radius": radius}),
    );
}

fn block(session: &mut Session, name: &str, position: V, size: V) {
    built(
        session,
        "feature.block",
        json!({"size": size, "position": position, "name": name}),
    );
}

fn boolean(session: &mut Session, op: &str, target: &str, tool: &str, name: &str) -> Value {
    built(
        session,
        "feature.boolean",
        json!({"op": op, "target": target, "tool": tool, "name": name}),
    )
}

/// Prints what the entities of a body are called, for the record of a
/// test run with `CRAFT3D_SHOW_NAMES=1` and `--nocapture` (or the names
/// of the one body the variable names).
fn show(feature: &str, corners: &[Corner], rims: &[Rim]) {
    match std::env::var("CRAFT3D_SHOW_NAMES") {
        Ok(which) if which == "1" || which == feature => {}
        _ => return,
    }
    println!(
        "== {feature}: {} vertices, {} edges",
        corners.len(),
        rims.len()
    );
    for corner in corners {
        println!(
            "  v {} {:?} at {:?}",
            corner.name, corner.aliases, corner.at
        );
    }
    for rim in rims {
        println!("  e {} {:?} {}", rim.name, rim.aliases, rim.curve);
    }
}

fn named_and_shown(session: &mut Session, feature: &str) -> (Vec<Corner>, Vec<Rim>) {
    let (corners, rims) = assert_named(session, feature);
    show(feature, &corners, &rims);
    (corners, rims)
}

// ---------------------------------------------------------------------
// (a) Completeness and uniqueness.
// ---------------------------------------------------------------------

#[test]
fn the_primitives_name_every_edge_and_vertex() {
    let mut s = session();
    new_document(&mut s, 1.0);
    block(&mut s, "Block", [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
    built(
        &mut s,
        "feature.cylinder",
        json!({"radius": 2, "height": 5, "position": [10, 0, 0], "name": "Cylinder"}),
    );
    built(
        &mut s,
        "feature.sphere",
        json!({"radius": 2, "position": [20, 0, 0], "name": "Sphere"}),
    );
    built(
        &mut s,
        "feature.cone",
        json!({"radius": 2, "height": 5, "position": [30, 0, 0], "name": "Cone"}),
    );
    built(
        &mut s,
        "feature.torus",
        json!({"major": 5, "minor": 1, "position": [45, 0, 0], "name": "Torus"}),
    );
    for name in ["Block", "Cylinder", "Sphere", "Cone", "Torus"] {
        named_and_shown(&mut s, name);
    }
}

type P = [f64; 2];

/// Two splines that share both ends: a leaf.
const UPPER: [P; 6] = [
    [0.0, 0.0],
    [1.0, 3.0],
    [4.0, 4.0],
    [7.0, 2.5],
    [9.0, 2.0],
    [10.0, 0.0],
];
const LOWER: [P; 4] = [[0.0, 0.0], [3.0, -2.0], [8.0, -1.5], [10.0, 0.0]];

fn scaled<const N: usize>(points: &[P; N], k: f64) -> Vec<P> {
    points.iter().map(|p| [p[0] * k, p[1] * k]).collect()
}

/// The sweeps of the suite, each from a sketch of its own, at scale `k`:
/// their names.
fn sweeps(s: &mut Session, k: f64) -> Vec<&'static str> {
    // A bar: a rectangle extruded.
    sketch(s, "BarProfile", "xy", 0.0);
    rectangle(s, "BarProfile", (0.0, 4.0 * k), (0.0, 3.0 * k));
    built(
        s,
        "feature.extrude",
        json!({"sketch": "BarProfile", "distance": 7.0 * k, "name": "Bar"}),
    );
    // A plate with a round hole, symmetric about its sketch plane.
    sketch(s, "PlateProfile", "yz", 20.0 * k);
    rectangle(s, "PlateProfile", (0.0, 8.0 * k), (0.0, 6.0 * k));
    circle(s, "PlateProfile", [4.0 * k, 3.0 * k], 1.5 * k);
    built(
        s,
        "feature.extrude",
        json!({"sketch": "PlateProfile", "distance": 2.0 * k, "symmetric": true,
               "name": "Plate"}),
    );
    // A lens: two arcs that share both ends, so two walls that meet in
    // two edges.
    sketch(s, "LensProfile", "xy", 0.0);
    let (a, b) = ([40.0 * k, -3.0 * k], [40.0 * k, 3.0 * k]);
    run(
        s,
        "sketch.arc",
        json!({"sketch": "LensProfile", "center": [36.0 * k, 0.0], "start": a, "end": b}),
    );
    run(
        s,
        "sketch.arc",
        json!({"sketch": "LensProfile", "center": [44.0 * k, 0.0], "start": "e3", "end": "e2"}),
    );
    built(
        s,
        "feature.extrude",
        json!({"sketch": "LensProfile", "distance": 2.0 * k, "name": "Lens"}),
    );
    // A leaf: two splines that share both ends.
    sketch(s, "LeafProfile", "zx", 0.0);
    let (upper, lower) = (scaled(&UPPER, k), scaled(&LOWER, k));
    let first = run(
        s,
        "sketch.spline",
        json!({"sketch": "LeafProfile", "control": upper}),
    );
    let control = texts(&first["control"]);
    run(
        s,
        "sketch.spline",
        json!({"sketch": "LeafProfile", "degree": 2,
               "control": [control[0], lower[1], lower[2], control[5]]}),
    );
    built(
        s,
        "feature.extrude",
        json!({"sketch": "LeafProfile", "distance": 1.5 * k, "name": "Leaf"}),
    );
    // A ring, a quarter of one, and a bowl whose profile lies on the axis.
    sketch(s, "RingProfile", "xy", 60.0 * k);
    rectangle(s, "RingProfile", (10.0 * k, 20.0 * k), (0.0, 5.0 * k));
    built(
        s,
        "feature.revolve",
        json!({"sketch": "RingProfile", "axis": "y", "name": "Ring"}),
    );
    sketch(s, "QuarterProfile", "xy", 120.0 * k);
    rectangle(s, "QuarterProfile", (10.0 * k, 20.0 * k), (0.0, 5.0 * k));
    built(
        s,
        "feature.revolve",
        json!({"sketch": "QuarterProfile", "axis": "y", "angle": "90 deg", "name": "Quarter"}),
    );
    sketch(s, "BowlProfile", "xy", 180.0 * k);
    run(
        s,
        "sketch.polyline",
        json!({"sketch": "BowlProfile", "closed": true,
               "points": [[0, 0], [6.0 * k, 0], [3.0 * k, 4.0 * k], [0, 4.0 * k]]}),
    );
    built(
        s,
        "feature.revolve",
        json!({"sketch": "BowlProfile", "axis": "y", "name": "Bowl"}),
    );
    // A post that ends on the tilted floor of a roof.
    built(
        s,
        "feature.cylinder",
        json!({"radius": 20.0 * k, "height": 3.0 * k, "position": [0, 100.0 * k, 7.0 * k],
               "axis": [0.4_f64.sin(), 0, 0.4_f64.cos()], "name": "Roof"}),
    );
    let roof = id_of(s, "Roof");
    sketch(s, "PostProfile", "xy", 0.0);
    rectangle(s, "PostProfile", (-2.0 * k, 2.0 * k), (98.0 * k, 102.0 * k));
    built(
        s,
        "feature.extrude",
        json!({"sketch": "PostProfile", "upTo": format!("{roof}:face(1)"), "name": "Post"}),
    );
    vec![
        "Bar", "Plate", "Lens", "Leaf", "Ring", "Quarter", "Bowl", "Post",
    ]
}

#[test]
fn the_sweeps_name_every_edge_and_vertex() {
    let mut s = session();
    new_document(&mut s, 1.0);
    for name in sweeps(&mut s, 1.0) {
        named_and_shown(&mut s, name);
    }
}

fn cylinder(session: &mut Session, name: &str, position: V, radius: f64, height: f64, axis: V) {
    built(
        session,
        "feature.cylinder",
        json!({"radius": radius, "height": height, "position": position, "axis": axis,
               "name": name}),
    );
}

/// One boolean each, at scale `k`: their names.
///
/// - `Joint`: two flush blocks united into one box (merged faces).
/// - `Step`: two overlapping blocks of different heights united.
/// - `Drilled`: a block with a round hole through it.
/// - `Slotted`: a block with a slot across its top, which cuts the top
///   and two sides in two.
/// - `Notched`: a block with a round notch across one of its edges: the
///   edge enters and leaves the cylinder.
/// - `Rounded`: what a block and a cylinder have in common.
/// - `Pocketed`: a block with a pocket whose tool was flush with its top
///   and one side (coincident faces).
/// - `Halved`: a cylinder cut in half lengthwise, and `Grooved`: a plate
///   with a ring half sunk into its top taken away. Both have edges and
///   corners that no face tells apart, which are numbered.
fn booleans(s: &mut Session, k: f64) -> Vec<&'static str> {
    let at = |x: f64, y: f64, z: f64| [x * k, y * k, z * k];
    block(s, "JointA", at(0.0, 0.0, 0.0), at(40.0, 20.0, 5.0));
    block(s, "JointB", at(40.0, 0.0, 0.0), at(30.0, 20.0, 5.0));
    boolean(s, "union", "JointA", "JointB", "Joint");

    block(s, "StepA", at(0.0, 100.0, 0.0), at(40.0, 20.0, 5.0));
    block(s, "StepB", at(30.0, 100.0, 0.0), at(30.0, 20.0, 9.0));
    boolean(s, "union", "StepA", "StepB", "Step");

    block(s, "DrilledA", at(0.0, 200.0, 0.0), at(40.0, 20.0, 5.0));
    cylinder(
        s,
        "DrilledB",
        at(15.0, 210.0, -1.0),
        4.0 * k,
        7.0 * k,
        [0.0, 0.0, 1.0],
    );
    boolean(s, "subtract", "DrilledA", "DrilledB", "Drilled");

    block(s, "SlottedA", at(0.0, 300.0, 0.0), at(40.0, 20.0, 5.0));
    block(s, "SlottedB", at(10.0, 299.0, 3.0), at(6.0, 22.0, 5.0));
    boolean(s, "subtract", "SlottedA", "SlottedB", "Slotted");

    block(s, "NotchedA", at(0.0, 400.0, 0.0), at(40.0, 20.0, 5.0));
    cylinder(
        s,
        "NotchedB",
        at(20.0, 399.0, 5.0),
        3.0 * k,
        22.0 * k,
        [0.0, 1.0, 0.0],
    );
    boolean(s, "subtract", "NotchedA", "NotchedB", "Notched");

    block(s, "RoundedA", at(0.0, 500.0, 0.0), at(40.0, 20.0, 5.0));
    cylinder(
        s,
        "RoundedB",
        at(0.0, 500.0, -1.0),
        12.0 * k,
        7.0 * k,
        [0.0, 0.0, 1.0],
    );
    boolean(s, "intersect", "RoundedA", "RoundedB", "Rounded");

    block(s, "PocketedA", at(0.0, 600.0, 0.0), at(40.0, 20.0, 5.0));
    block(s, "PocketedB", at(30.0, 605.0, 2.0), at(10.0, 10.0, 3.0));
    boolean(s, "subtract", "PocketedA", "PocketedB", "Pocketed");

    cylinder(
        s,
        "HalvedA",
        at(0.0, 700.0, 0.0),
        5.0 * k,
        10.0 * k,
        [0.0, 0.0, 1.0],
    );
    block(s, "HalvedB", at(1.0, 690.0, -1.0), at(10.0, 20.0, 12.0));
    boolean(s, "subtract", "HalvedA", "HalvedB", "Halved");

    block(s, "GroovedA", at(-20.0, 780.0, 0.0), at(40.0, 40.0, 5.0));
    built(
        s,
        "feature.torus",
        json!({"major": 10.0 * k, "minor": 2.0 * k, "position": at(0.0, 800.0, 5.0),
               "name": "GroovedB"}),
    );
    boolean(s, "subtract", "GroovedA", "GroovedB", "Grooved");
    vec![
        "Joint", "Step", "Drilled", "Slotted", "Notched", "Rounded", "Pocketed", "Halved",
        "Grooved",
    ]
}

#[test]
fn one_boolean_names_every_edge_and_vertex() {
    let mut s = session();
    new_document(&mut s, 1.0);
    for name in booleans(&mut s, 1.0) {
        let (corners, rims) = named_and_shown(&mut s, name);
        // Only the two bodies with entities no face tells apart have
        // numbered names: everywhere else the faces say which is which.
        let numbered = corners
            .iter()
            .map(|c| &c.name)
            .chain(rims.iter().map(|r| &r.name))
            .filter(|n| n.ends_with(')'))
            .count();
        let expected = match name {
            "Halved" => 6,
            "Grooved" => 4,
            _ => 0,
        };
        assert_eq!(numbered, expected, "{name}");
    }
}

/// Copies and patterns of bodies with and without booleans in them, at
/// scale `k`: their names.
///
/// - `Moved`: `Drilled` turned and shifted. `Mirrored`: `Slotted` in a
///   mirror.
/// - `Row`: three instances of `Notched` that stay apart.
/// - `Wheel`: four blocks round an axis that stay apart.
/// - `Rail`: three blocks end to end, joined into one box (merged faces).
/// - `Stair`: three blocks that overlap, each a step up from the last.
/// - `Cross`: four bars round an axis that overlap in the middle.
fn copies(s: &mut Session, k: f64) -> Vec<&'static str> {
    let at = |x: f64, y: f64, z: f64| [x * k, y * k, z * k];
    booleans(s, k);
    built(
        s,
        "feature.transform",
        json!({"source": "Drilled", "translate": at(0.0, 0.0, 50.0), "angle": "30 deg",
               "axis": [1, 1, 0], "pivot": at(0.0, 200.0, 0.0), "name": "Moved"}),
    );
    built(
        s,
        "feature.mirror",
        json!({"source": "Slotted", "normal": [0, 0, 1], "point": at(0.0, 0.0, -10.0),
               "name": "Mirrored"}),
    );
    built(
        s,
        "feature.pattern",
        json!({"source": "Notched", "count": 3, "step": at(0.0, 0.0, 20.0), "name": "Row"}),
    );
    block(s, "Spoke", at(110.0, -2.0, 0.0), at(10.0, 4.0, 2.0));
    built(
        s,
        "feature.pattern",
        json!({"source": "Spoke", "count": 4, "angle": "90 deg", "point": at(100.0, 0.0, 0.0),
               "name": "Wheel"}),
    );
    block(s, "Sleeper", at(200.0, 0.0, 0.0), at(10.0, 4.0, 2.0));
    built(
        s,
        "feature.pattern",
        json!({"source": "Sleeper", "count": 3, "step": at(10.0, 0.0, 0.0), "name": "Rail"}),
    );
    block(s, "Tread", at(300.0, 0.0, 0.0), at(10.0, 4.0, 2.0));
    built(
        s,
        "feature.pattern",
        json!({"source": "Tread", "count": 3, "step": at(6.0, 0.0, 1.0), "name": "Stair"}),
    );
    block(s, "Arm", at(399.0, -1.0, 0.0), at(10.0, 2.0, 2.0));
    built(
        s,
        "feature.pattern",
        json!({"source": "Arm", "count": 4, "angle": "90 deg", "point": at(400.0, 0.0, 0.0),
               "name": "Cross"}),
    );
    vec![
        "Moved", "Mirrored", "Row", "Wheel", "Rail", "Stair", "Cross",
    ]
}

#[test]
fn copies_and_patterns_name_every_edge_and_vertex() {
    let mut s = session();
    new_document(&mut s, 1.0);
    for name in copies(&mut s, 1.0) {
        let (corners, rims) = named_and_shown(&mut s, name);
        // A copy of a body a boolean made keeps what the entities
        // answered to there, as copies.
        if matches!(name, "Moved" | "Mirrored" | "Row") {
            assert!(
                corners.iter().any(|c| !c.aliases.is_empty())
                    && rims.iter().any(|r| !r.aliases.is_empty()),
                "{name}"
            );
        }
    }
    // The rail is one box: its eight corners are corners of the first and
    // of the last sleeper, and answer to those names.
    let rail = id_of(&mut s, "Rail");
    let sleeper = id_of(&mut s, "Sleeper");
    let corners = corners(&mut s, "Rail");
    assert_eq!(corners.len(), 8);
    for (instance, vertex, at) in [
        (0, 0, [200.0, 0.0, 0.0]),
        (0, 6, [200.0, 4.0, 2.0]),
        (2, 1, [230.0, 0.0, 0.0]),
        (2, 7, [230.0, 4.0, 2.0]),
    ] {
        let alias = format!("{rail}:copy({instance})/{sleeper}:vertex({vertex})");
        let found = corners
            .iter()
            .find(|c| c.aliases.contains(&alias))
            .unwrap_or_else(|| panic!("{alias} in {corners:?}"));
        assert_eq!(found.at, at, "{alias}");
        assert!(
            found.name.starts_with(&format!("{rail}:corner[")),
            "{found:?}"
        );
    }
}

/// The part the stability scenarios edit, at scale `k`: a block `Base`
/// (`w` x `d` x `h`) with a hole of radius `r` through it at `hx`
/// (`Holed`), a slot across its top from `sx`, `sw` wide and `sd` deep,
/// that starts at `sy` and runs out through the back (`Slit`), and a boss of radius 2
/// and height 3 standing on the top at x = 5 (`Part`): three booleans in
/// a row, the last with coincident faces.
fn part(s: &mut Session, k: f64) {
    for (name, value) in [
        ("w", 40.0),
        ("d", 20.0),
        ("h", 5.0),
        ("r", 4.0),
        ("hx", 15.0),
        ("sx", 28.0),
        ("sw", 6.0),
        ("sd", 2.0),
        ("sy", -1.0),
        ("u", 1.0),
    ] {
        run(
            s,
            "variable.set",
            json!({"name": name, "expr": format!("{} mm", value * k)}),
        );
    }
    built(
        s,
        "feature.block",
        json!({"size": ["w", "d", "h"], "name": "Base"}),
    );
    built(
        s,
        "feature.cylinder",
        json!({"radius": "r", "height": "h + 2 * u", "position": ["hx", "d / 2", "-u"],
               "name": "Hole"}),
    );
    boolean(s, "subtract", "Base", "Hole", "Holed");
    built(
        s,
        "feature.block",
        json!({"size": ["sw", "d + 2 * u", "sd + u"], "position": ["sx", "sy", "h - sd"],
               "name": "Slot"}),
    );
    boolean(s, "subtract", "Holed", "Slot", "Slit");
    built(
        s,
        "feature.cylinder",
        json!({"radius": "2 * u", "height": "3 * u", "position": ["5 * u", "d / 2", "h"],
               "name": "Boss"}),
    );
    boolean(s, "union", "Slit", "Boss", "Part");
}

/// The names of the faces of the part, by what they are.
struct PartFaces {
    top: String,
    bottom: String,
    front: String,
    right: String,
    hole: String,
    slot_left: String,
    slot_floor: String,
    boss_wall: String,
    boss_top: String,
    /// The prefix of a name the base block gave: `F9:copy(0)/F1:`.
    base: String,
}

fn part_faces(s: &mut Session) -> PartFaces {
    let ids: Vec<String> = ["Base", "Hole", "Holed", "Slot", "Slit", "Boss", "Part"]
        .iter()
        .map(|name| id_of(s, name))
        .collect();
    let [base, hole, holed, slot, slit, boss, part] = ids.as_slice() else {
        panic!()
    };
    // In the short form: a row of copies that are each instance 0 is one
    // link (Part's target is Slit, whose target is Holed, whose target is
    // Base).
    let of_base = format!("{part}:copy(0)/{base}:");
    let of_hole = format!("{part}:copy(0)/{holed}:copy(1)/{hole}:");
    let of_slot = format!("{part}:copy(0)/{slit}:copy(1)/{slot}:");
    let of_boss = format!("{part}:copy(1)/{boss}:");
    PartFaces {
        bottom: format!("{of_base}face(0)"),
        top: format!("{of_base}face(1)"),
        front: format!("{of_base}face(2)"),
        right: format!("{of_base}face(5)"),
        hole: format!("{of_hole}face(0)"),
        slot_left: format!("{of_slot}face(4)"),
        slot_floor: format!("{of_slot}face(0)"),
        boss_wall: format!("{of_boss}face(0)"),
        boss_top: format!("{of_boss}face(2)"),
        base: of_base,
    }
}

/// `F9:corner[a,b,c]` for the faces given in any order.
fn corner_name(s: &mut Session, faces: &[&String]) -> String {
    let part = id_of(s, "Part");
    let mut faces: Vec<&str> = faces.iter().map(|f| f.as_str()).collect();
    // The order names are listed in: by feature, then by what follows;
    // for the names of this part that is the order of their texts. (A
    // name typed in another order is read as the same name: see
    // `assert_named`.)
    faces.sort();
    format!("{part}:corner[{}]", faces.join(","))
}

/// What the suite knows of a body beyond the names: per name of an edge
/// its curve, the faces on its sides and the names of the vertices at
/// its ends; per name of a vertex the edges that end in it. Two bodies
/// with one such record have every name on the same edge or corner, as
/// far as the faces and the way the entities hang together can tell.
fn structure(s: &mut Session, feature: &str) -> BTreeMap<String, String> {
    let (corners, rims) = (corners(s, feature), rims(s, feature));
    let vertex_at = |p: V| {
        corners
            .iter()
            .find(|c| c.at == p)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| panic!("{feature}: no vertex at {p:?}"))
    };
    let mut record = BTreeMap::new();
    let mut ending: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for rim in &rims {
        let mut ends = [vertex_at(rim.start), vertex_at(rim.end)];
        ends.sort();
        for end in &ends {
            ending
                .entry(end.clone())
                .or_default()
                .push(rim.name.clone());
        }
        record.insert(
            rim.name.clone(),
            format!(
                "{} between {:?} from {} to {}, also {:?}",
                rim.curve, rim.faces, ends[0], ends[1], rim.aliases
            ),
        );
    }
    for corner in &corners {
        let mut edges = ending.remove(&corner.name).unwrap_or_default();
        edges.sort();
        record.insert(
            corner.name.clone(),
            format!("vertex of {edges:?}, also {:?}", corner.aliases),
        );
    }
    record
}

/// Fails with the names that are on something else, or gone, or new.
fn assert_same_structure(
    after: &BTreeMap<String, String>,
    before: &BTreeMap<String, String>,
    what: &str,
) {
    let mut differences = Vec::new();
    for (name, record) in before {
        match after.get(name) {
            None => differences.push(format!("gone: {name}\n   was {record}")),
            Some(now) if now != record => {
                differences.push(format!("moved: {name}\n   was {record}\n   is  {now}"));
            }
            Some(_) => {}
        }
    }
    for name in after.keys().filter(|name| !before.contains_key(*name)) {
        differences.push(format!("new: {name}\n   is  {}", after[name]));
    }
    assert!(
        differences.is_empty(),
        "{what}: {} names differ\n{}",
        differences.len(),
        differences.join("\n")
    );
}

fn set(s: &mut Session, name: &str, value: f64) {
    run(
        s,
        "variable.set",
        json!({"name": name, "expr": format!("{value} mm")}),
    );
    let status = feature(s, "Part")["status"].clone();
    assert_eq!(status, "ok", "after {name} = {value}");
}

#[test]
fn two_and_three_booleans_name_every_edge_and_vertex() {
    // Each stage is looked at while its body is still its own.
    for stage in ["Holed", "Slit", "Part"] {
        let mut s = session();
        new_document(&mut s, 1.0);
        part(&mut s, 1.0);
        let last = ["Boss", "Part", "Slot", "Slit"];
        let keep = match stage {
            "Holed" => 0,
            "Slit" => 2,
            _ => 4,
        };
        for name in last.iter().take(4 - keep) {
            run(&mut s, "feature.delete", json!({"feature": name}));
        }
        // Deleting from the end leaves the stage as the last body.
        let (corners, rims) = named_and_shown(&mut s, stage);
        let expected = match stage {
            "Holed" => (10, 14),
            "Slit" => (18, 26),
            _ => (20, 28),
        };
        assert_eq!((corners.len(), rims.len()), expected, "{stage}");
    }
}

// ---------------------------------------------------------------------
// (b) Stability: an edit that changes sizes and not the topology leaves
// every name on the same edge or corner.
// ---------------------------------------------------------------------

/// The corners of the part whose place a closed form gives, as (name,
/// position) for the values of the variables.
fn known_corners(s: &mut Session, v: &BTreeMap<&str, f64>) -> Vec<(String, V)> {
    let f = part_faces(s);
    let (w, d, h) = (v["w"], v["d"], v["h"]);
    let (r, hx, sx, sd) = (v["r"], v["hx"], v["sx"], v["sd"]);
    let u = v["u"];
    let boss = id_of(s, "Boss");
    let part = id_of(s, "Part");
    vec![
        // What the base block had keeps its name there.
        (format!("{}vertex(7)", f.base), [w, d, h]),
        (format!("{}vertex(0)", f.base), [0.0, 0.0, 0.0]),
        (
            corner_name(s, &[&f.bottom, &f.front, &f.right]),
            [w, 0.0, 0.0],
        ),
        // Where the slot's left wall leaves the top at the front, and
        // where it meets its floor there.
        (
            corner_name(s, &[&f.top, &f.front, &f.slot_left]),
            [sx, 0.0, h],
        ),
        (
            corner_name(s, &[&f.front, &f.slot_left, &f.slot_floor]),
            [sx, 0.0, h - sd],
        ),
        // The one vertex of each rim of the hole, and of the boss.
        (corner_name(s, &[&f.top, &f.hole]), [hx + r, d / 2.0, h]),
        (
            corner_name(s, &[&f.bottom, &f.hole]),
            [hx + r, d / 2.0, 0.0],
        ),
        (
            corner_name(s, &[&f.top, &f.boss_wall]),
            [7.0 * u, d / 2.0, h],
        ),
        (
            corner_name(s, &[&f.boss_wall, &f.boss_top]),
            [7.0 * u, d / 2.0, h + 3.0 * u],
        ),
        (
            format!("{part}:copy(1)/{boss}:vertex(1)"),
            [7.0 * u, d / 2.0, h + 3.0 * u],
        ),
    ]
}

fn assert_known_corners(s: &mut Session, v: &BTreeMap<&str, f64>, slack: f64, what: &str) {
    for (index, (name, expected)) in known_corners(s, v).into_iter().enumerate() {
        let datum = feature(s, &format!("Mark {index}"));
        assert_eq!(datum["status"], "ok", "{what}: {name}: {datum}");
        let at = vector(&datum["datum"]["at"]);
        assert!(
            distance(at, expected) <= slack,
            "{what}: {name} is at {at:?}, not at {expected:?}"
        );
    }
}

fn part_values(k: f64) -> BTreeMap<&'static str, f64> {
    [
        ("w", 40.0),
        ("d", 20.0),
        ("h", 5.0),
        ("r", 4.0),
        ("hx", 15.0),
        ("sx", 28.0),
        ("sw", 6.0),
        ("sd", 2.0),
        ("sy", -1.0),
        ("u", 1.0),
    ]
    .into_iter()
    .map(|(name, value)| (name, value * k))
    .collect()
}

/// The part with a datum point `Mark i` on each of its known corners.
fn marked_part(s: &mut Session, k: f64) -> BTreeMap<&'static str, f64> {
    new_document(s, k);
    part(s, k);
    let values = part_values(k);
    for (index, (name, _)) in known_corners(s, &values).into_iter().enumerate() {
        built(
            s,
            "datum.point",
            json!({"vertex": name, "name": format!("Mark {index}")}),
        );
    }
    assert_known_corners(s, &values, 0.0, "as built");
    values
}

#[test]
fn an_edit_that_keeps_the_topology_keeps_every_name_on_its_entity() {
    for k in [1e-3, 1.0, 1e6] {
        let mut s = session();
        let mut values = marked_part(&mut s, k);
        let before = structure(&mut s, "Part");
        assert_eq!(before.len(), 20 + 28);
        // A size of the base, the radius and the place of the hole, the
        // place and the depth of the slot: the tools still cut the faces
        // they cut.
        for (name, value) in [
            ("w", 55.0),
            ("d", 26.0),
            ("h", 7.0),
            ("r", 3.0),
            ("hx", 12.5),
            ("sx", 33.0),
            ("sd", 3.5),
            ("sw", 4.0),
            ("w", 47.0),
        ] {
            let value = value * k;
            set(&mut s, name, value);
            values.insert(name, value);
            let what = format!("x{k}: {name} = {value}");
            assert_same_structure(&structure(&mut s, "Part"), &before, &what);
            assert_known_corners(&mut s, &values, 1e-9 * k, &what);
        }
        // And back: the same again, to the last bit of every position.
        for (name, value) in part_values(k) {
            set(&mut s, name, value);
            values.insert(name, value);
        }
        assert_same_structure(&structure(&mut s, "Part"), &before, "back");
        assert_known_corners(&mut s, &values, 0.0, "back");
    }
}

// ---------------------------------------------------------------------
// (c) An edit that removes an entity makes the references to it fail,
// with the name lost, and leaves the others.
// ---------------------------------------------------------------------

/// The marks that are not placed, with the reason.
fn lost_marks(s: &mut Session) -> BTreeMap<usize, String> {
    let document = ask(s, "document.inspect", json!({}));
    document["features"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| {
            let index: usize = f["name"].as_str()?.strip_prefix("Mark ")?.parse().ok()?;
            match f["status"].as_str()? {
                "ok" => None,
                _ => Some((index, f["error"].as_str()?.to_string())),
            }
        })
        .collect()
}

#[test]
fn an_edit_that_removes_an_entity_fails_the_references_to_it_and_no_others() {
    let mut s = session();
    let mut values = marked_part(&mut s, 1.0);
    let names: Vec<String> = known_corners(&mut s, &values)
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert!(lost_marks(&mut s).is_empty());

    // The slot no longer reaches the front: the two corners where its
    // left wall met the front are gone, and nothing else.
    set(&mut s, "sy", 3.0);
    let lost = lost_marks(&mut s);
    assert_eq!(lost.keys().copied().collect::<Vec<_>>(), [3, 4]);
    for (index, reason) in &lost {
        assert!(
            reason.starts_with(&format!(
                "at: `Part` no longer has a vertex {}; its vertices are: ",
                names[*index]
            )),
            "{reason}"
        );
    }
    // The others are where they were, and the body is sound.
    values.insert("sy", 3.0);
    for (index, (name, expected)) in known_corners(&mut s, &values).into_iter().enumerate() {
        if lost.contains_key(&index) {
            continue;
        }
        let at = vector(&feature(&mut s, &format!("Mark {index}"))["datum"]["at"]);
        assert_eq!(at, expected, "{name}");
    }
    assert_named(&mut s, "Part");
    // Put back, the references are found again where they were.
    set(&mut s, "sy", -1.0);
    values.insert("sy", -1.0);
    assert!(lost_marks(&mut s).is_empty());
    assert_known_corners(&mut s, &values, 0.0, "the slot is back");

    // The slot takes the far corner of the base away: the corner is
    // gone under the name the base gave it and under its own.
    set(&mut s, "sx", 36.0);
    let lost = lost_marks(&mut s);
    assert_eq!(
        lost.keys().copied().collect::<Vec<_>>(),
        [0],
        "{lost:?}: the corner (w, d, h)"
    );
    assert!(lost[&0].contains(&names[0]), "{lost:?}");
    set(&mut s, "sx", 28.0);
    assert!(lost_marks(&mut s).is_empty());

    // The feature that made the hole is switched off: what lay on the
    // hole is gone with the name lost, and the rest of the part's
    // corners are found.
    run(&mut s, "feature.suppress", json!({"feature": "Hole"}));
    let lost = lost_marks(&mut s);
    // (Suppressed, the hole passes the base on unchanged: since
    // 2026-10-23 the part is built without it.)
    assert_eq!(
        lost.keys().copied().collect::<Vec<_>>(),
        [5, 6],
        "{lost:?}: the corners on the hole's wall"
    );
    for (mark, reason) in &lost {
        assert!(reason.contains(&names[*mark]), "{reason}");
    }
    run(
        &mut s,
        "feature.suppress",
        json!({"feature": "Hole", "suppressed": false}),
    );
    assert!(lost_marks(&mut s).is_empty());
    assert_known_corners(&mut s, &values, 0.0, "the hole is back");
}

// ---------------------------------------------------------------------
// (d) Names do not depend on the scale, on the order of unrelated
// features, or on what else runs.
// ---------------------------------------------------------------------

/// Every name of every body of the suite's models at scale `k`, with
/// what the suite knows of the entity it is on.
fn all_structures(k: f64) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut found = BTreeMap::new();
    let mut s = session();
    new_document(&mut s, k);
    for name in sweeps(&mut s, k) {
        found.insert(name.to_string(), structure(&mut s, name));
    }
    let mut s = session();
    new_document(&mut s, k);
    for name in copies(&mut s, k) {
        found.insert(name.to_string(), structure(&mut s, name));
    }
    let mut s = session();
    new_document(&mut s, k);
    for name in booleans(&mut s, k) {
        found.insert(name.to_string(), structure(&mut s, name));
    }
    let mut s = session();
    new_document(&mut s, k);
    part(&mut s, k);
    found.insert("Part".to_string(), structure(&mut s, "Part"));
    found
}

#[test]
fn names_do_not_depend_on_the_scale() {
    let unit = all_structures(1.0);
    assert_eq!(unit.len(), 8 + 7 + 9 + 1);
    for k in [1e-3, 1e6] {
        let scaled = all_structures(k);
        for (name, record) in &unit {
            assert_same_structure(&scaled[name], record, &format!("{name} at x{k}"));
        }
    }
}

#[test]
fn names_do_not_depend_on_what_else_runs_or_on_unrelated_features() {
    let alone = all_structures(1.0);
    // The same models built by four threads at once.
    let threads: Vec<_> = (0..4)
        .map(|_| std::thread::spawn(|| all_structures(1.0)))
        .collect();
    for thread in threads {
        assert_eq!(thread.join().unwrap(), alone);
    }
    // An unrelated feature anywhere in the history changes no name.
    let mut s = session();
    new_document(&mut s, 1.0);
    part(&mut s, 1.0);
    let before = structure(&mut s, "Part");
    assert_eq!(before, alone["Part"]);
    built(
        &mut s,
        "feature.sphere",
        json!({"radius": 3, "position": [0, 0, 100], "name": "Other"}),
    );
    block(&mut s, "Another", [0.0, 0.0, 200.0], [1.0, 2.0, 3.0]);
    for position in [0, 1, 3, 5, 7, 8] {
        run(
            &mut s,
            "feature.move",
            json!({"feature": "Other", "position": position}),
        );
        run(
            &mut s,
            "feature.move",
            json!({"feature": "Another", "position": 8 - position}),
        );
        assert_same_structure(
            &structure(&mut s, "Part"),
            &before,
            &format!("at {position}"),
        );
    }
}

// ---------------------------------------------------------------------
// (e) Saved and reopened, the names are the same and so are the
// references by them.
// ---------------------------------------------------------------------

#[test]
fn saved_and_reopened_the_names_are_the_same() {
    let dir = workdir("saved");
    let mut s = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    let values = marked_part(&mut s, 1.0);
    let before = structure(&mut s, "Part");
    let marks = |s: &mut Session| -> Vec<Value> {
        (0..10)
            .map(|index| feature(s, &format!("Mark {index}")))
            .collect()
    };
    let placed = marks(&mut s);
    let path = dir.join("part.3dcraft");
    run(&mut s, "file.save", json!({"path": path.to_str().unwrap()}));
    run(&mut s, "file.close", json!({}));
    run(&mut s, "file.open", json!({"path": path.to_str().unwrap()}));
    assert_eq!(structure(&mut s, "Part"), before);
    assert_eq!(marks(&mut s), placed);
    assert_known_corners(&mut s, &values, 0.0, "reopened");
    // The file holds the names as the document does: written again it
    // is the same file.
    let again = dir.join("again.3dcraft");
    run(
        &mut s,
        "file.save",
        json!({"path": again.to_str().unwrap()}),
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        std::fs::read(&again).unwrap()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------
// Where a name cannot be stable, and what happens then.
// ---------------------------------------------------------------------

/// A cylinder cut in half lengthwise by a block, both turned by `turn`
/// about the z axis first: the flat cut meets the wall in two straight
/// edges that no face tells apart.
fn half_cylinder(s: &mut Session) {
    run(s, "variable.set", json!({"name": "turn", "expr": "0 deg"}));
    cylinder(s, "Round", [0.0, 0.0, 0.0], 5.0, 10.0, [0.0, 0.0, 1.0]);
    block(s, "Cutter", [1.0, -10.0, -1.0], [10.0, 20.0, 12.0]);
    for source in ["Round", "Cutter"] {
        built(
            s,
            "feature.transform",
            json!({"source": source, "angle": "turn", "name": format!("{source} turned")}),
        );
    }
    boolean(s, "subtract", "Round turned", "Cutter turned", "Half");
}

#[test]
fn edges_and_corners_no_face_tells_apart_are_numbered_and_the_number_can_move() {
    let mut s = session();
    new_document(&mut s, 1.0);
    half_cylinder(&mut s);
    let (before, rims) = assert_named(&mut s, "Half");
    show("Half", &before, &rims);
    // Two straight edges between the cut and the wall, and at each end of
    // them two corners on the same three faces: those are numbered, in
    // the order of where they lie (by x, then y, then z).
    let numbered = |name: &str| name.ends_with("(0)") || name.ends_with("(1)");
    assert_eq!(before.iter().filter(|c| numbered(&c.name)).count(), 4);
    let tied: Vec<&Rim> = rims.iter().filter(|r| numbered(&r.name)).collect();
    assert_eq!(tied.len(), 2, "{tied:?}");
    assert!(
        tied.iter()
            .all(|r| r.curve == "line" && r.aliases.is_empty())
    );
    let y = (25.0_f64 - 1.0).sqrt();
    for corner in &before {
        let expected = if corner.name.ends_with("(0)") { -y } else { y };
        assert!(
            // The two x are a rounding error apart, either way round: it
            // is y that orders them.
            (corner.at[1] - expected).abs() < 1e-9 && (corner.at[0] - 1.0).abs() < 1e-9,
            "{corner:?}"
        );
    }

    // The limit: turn both bodies half a turn about z. It is the same
    // solid, turned; every name is still there, but each numbered name is
    // now on the image of the *other* one of its pair. Nothing fails.
    run(
        &mut s,
        "variable.set",
        json!({"name": "turn", "expr": "180 deg"}),
    );
    let (after, _) = assert_named(&mut s, "Half");
    assert_eq!(after.len(), before.len());
    for corner in &before {
        let turned = [-corner.at[0], -corner.at[1], corner.at[2]];
        let same_name = after.iter().find(|c| c.name == corner.name).unwrap();
        assert!(
            distance(same_name.at, turned) > 9.0,
            "{}: the number stayed on the turned corner",
            corner.name
        );
        let other = match corner.name.strip_suffix("(0)") {
            Some(stem) => format!("{stem}(1)"),
            None => format!("{}(0)", corner.name.strip_suffix("(1)").unwrap()),
        };
        let swapped = after.iter().find(|c| c.name == other).unwrap();
        assert!(distance(swapped.at, turned) < 1e-9, "{}", corner.name);
    }
}

#[test]
fn a_second_edge_between_the_same_faces_takes_the_bare_name_away() {
    let mut s = session();
    new_document(&mut s, 1.0);
    run(
        &mut s,
        "variable.set",
        json!({"name": "sy", "expr": "5 mm"}),
    );
    block(&mut s, "Base", [0.0, 0.0, 0.0], [40.0, 20.0, 5.0]);
    built(
        &mut s,
        "feature.block",
        json!({"size": [6, 30, 5], "position": [10, "sy", 3], "name": "Slot"}),
    );
    boolean(&mut s, "subtract", "Base", "Slot", "Part");
    let (base, part) = (id_of(&mut s, "Base"), id_of(&mut s, "Part"));
    let top = format!("{part}:copy(0)/{base}:face(1)");
    let front = format!("{part}:copy(0)/{base}:face(2)");
    let bare = format!("{part}:between[{top},{front}]");
    let kept = format!("{part}:copy(0)/{base}:edge(4)");
    // The slot stops short of the front: one edge between the top and the
    // front, which is also the edge the base had there.
    let rims = assert_named(&mut s, "Part").1;
    let edge = rims.iter().find(|r| r.name == bare).expect("the edge");
    assert_eq!(edge.aliases, std::slice::from_ref(&kept));
    assert_eq!((edge.start[0] - edge.end[0]).abs(), 40.0);

    // The slot runs out through the front: two edges between the top and
    // the front. Neither is "the" edge: the bare name and the name the
    // base gave are gone, and each piece says which faces are near it.
    run(
        &mut s,
        "variable.set",
        json!({"name": "sy", "expr": "-1 mm"}),
    );
    let rims = assert_named(&mut s, "Part").1;
    assert!(
        rims.iter()
            .all(|r| r.name != bare && !r.aliases.contains(&kept))
    );
    let pieces: Vec<&Rim> = rims
        .iter()
        .filter(|r| {
            r.name
                .starts_with(&format!("{part}:between[{top},{front}|"))
        })
        .collect();
    assert_eq!(pieces.len(), 2, "{pieces:?}");
    let left = format!("{part}:copy(0)/{base}:face(4)");
    let right = format!("{part}:copy(0)/{base}:face(5)");
    for piece in &pieces {
        // The piece at the left end of the front names the left face.
        let at_left = piece.start[0].min(piece.end[0]) == 0.0;
        assert_eq!(piece.name.contains(&left), at_left, "{piece:?}");
        assert_eq!(piece.name.contains(&right), !at_left, "{piece:?}");
        assert!(!piece.name.ends_with(')'), "no number: {}", piece.name);
    }
    // Back again, the edge has its names back.
    run(
        &mut s,
        "variable.set",
        json!({"name": "sy", "expr": "5 mm"}),
    );
    let rims = assert_named(&mut s, "Part").1;
    assert_eq!(
        rims.iter().find(|r| r.name == bare).unwrap().aliases,
        [kept]
    );
}

#[test]
fn the_two_circles_of_a_groove_are_numbered() {
    let mut s = session();
    new_document(&mut s, 1.0);
    // A ring half sunk into a plate leaves a groove: the top is cut into
    // a disc and the rest, which share the top's name, and the groove
    // meets them in two circles that no face tells apart.
    block(&mut s, "Plate", [-20.0, -20.0, 0.0], [40.0, 40.0, 5.0]);
    built(
        &mut s,
        "feature.torus",
        json!({"major": 10, "minor": 2, "position": [0, 0, 5], "name": "Ring"}),
    );
    boolean(&mut s, "subtract", "Plate", "Ring", "Grooved");
    let (corners, rims) = named_and_shown(&mut s, "Grooved");
    let circles: Vec<&Rim> = rims.iter().filter(|r| r.curve == "circle").collect();
    assert_eq!(circles.len(), 2);
    let pair = "F3:between[F3:copy(0)/F1:face(1),F3:copy(1)/F2:face(0)]";
    let mut names: Vec<&str> = circles.iter().map(|r| r.name.as_str()).collect();
    names.sort();
    assert_eq!(names, [format!("{pair}(0)"), format!("{pair}(1)")]);
    // Each circle has one vertex, numbered the same way: by where it is.
    let on_rims: Vec<&Corner> = corners.iter().filter(|c| c.aliases.is_empty()).collect();
    assert_eq!(on_rims.len(), 2);
    for corner in on_rims {
        let inner = corner.at[0] == 8.0;
        assert!(inner || corner.at[0] == 12.0, "{corner:?}");
        assert_eq!(corner.name.ends_with("(0)"), inner, "{corner:?}");
    }
}

#[test]
fn a_numbered_edge_an_operand_had_answers_to_the_name_it_had_there() {
    let mut s = session();
    new_document(&mut s, 1.0);
    // A lens: two walls that meet in two straight edges, with a hole
    // drilled through its middle. No face tells the two edges apart, nor
    // the corners at their ends; the sweep that made them does.
    sketch(&mut s, "LensProfile", "xy", 0.0);
    run(
        &mut s,
        "sketch.arc",
        json!({"sketch": "LensProfile", "center": [36, 0], "start": [40, -3], "end": [40, 3]}),
    );
    run(
        &mut s,
        "sketch.arc",
        json!({"sketch": "LensProfile", "center": [44, 0], "start": "e3", "end": "e2"}),
    );
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "LensProfile", "distance": 2, "name": "Lens"}),
    );
    cylinder(
        &mut s,
        "Drill",
        [40.0, 0.0, -1.0],
        0.3,
        4.0,
        [0.0, 0.0, 1.0],
    );
    boolean(&mut s, "subtract", "Lens", "Drill", "Drilled");
    let (lens, drilled) = (id_of(&mut s, "Lens"), id_of(&mut s, "Drilled"));
    let (corners, rims) = named_and_shown(&mut s, "Drilled");
    for (alias, y) in [("e6,e4", -3.0), ("e4,e6", 3.0)] {
        let kept = format!("{drilled}:copy(0)/{lens}:lateralEdge({alias})");
        let edge = rims
            .iter()
            .find(|r| r.aliases == [kept.clone()])
            .unwrap_or_else(|| panic!("{kept}"));
        assert_eq!((edge.start[1], edge.end[1]), (y, y));
        // By its faces it is one of two, told apart by a number only.
        let number = if y < 0.0 { "(0)" } else { "(1)" };
        assert!(edge.name.ends_with(number), "{edge:?}");
        for (role, z) in [("startVertex", 0.0), ("endVertex", 2.0)] {
            let kept = format!("{drilled}:copy(0)/{lens}:{role}({alias})");
            let corner = corners
                .iter()
                .find(|c| c.aliases == [kept.clone()])
                .unwrap_or_else(|| panic!("{kept}"));
            assert_eq!(corner.at, [40.0, y, z]);
            assert!(corner.name.ends_with(number), "{corner:?}");
        }
    }
}
