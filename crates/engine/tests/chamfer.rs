//! Chamfers through the commands (M9, the first blend): every body is
//! measured against a closed form worked out by hand, at three scales, and
//! checked deep; the references follow upstream edits and fail by name;
//! the faces a chamfer makes carry names later features build on; a saved
//! document reopens to the same bodies; and what is not built, or does not
//! fit, is refused with the edge, the face and the reason.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Access, Session, Workspace};
use serde_json::{Value, json};
use std::f64::consts::PI;
use std::path::PathBuf;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn workdir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("chamfer-{name}"));
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
    // nothing gives (docs/testing.md: incremental = full regeneration).
    assert_ne!(
        session.regeneration_matches_a_full_one(),
        Some(false),
        "after {id} {params}"
    );
    result
}

/// A command that adds or edits a feature, which has to regenerate.
fn built(session: &mut Session, id: &str, params: Value) -> Value {
    let result = run(session, id, params.clone());
    assert_eq!(result["status"], "ok", "{id} {params}: {result}");
    result
}

/// A command that is accepted but whose feature fails: the reason.
fn failed(session: &mut Session, id: &str, params: Value) -> String {
    let result = run(session, id, params.clone());
    assert_eq!(result["status"], "failed", "{id} {params}: {result}");
    result["error"].as_str().unwrap().to_string()
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

/// A new document in millimetres whose tolerance fits parts of size `k`.
fn new_document(session: &mut Session, k: f64) {
    run(
        session,
        "file.new",
        json!({"units": "mm", "tolerance": 1e-6 * k}),
    );
}

fn inspect(session: &mut Session) -> Value {
    run(session, "document.inspect", json!({}))
}

fn body(session: &mut Session, name: &str) -> Value {
    let document = inspect(session);
    document["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == name)
        .unwrap_or_else(|| panic!("no body {name} in {document}"))
        .clone()
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

#[track_caller]
fn assert_close(actual: &Value, expected: f64, what: &str) {
    let actual = actual
        .as_f64()
        .unwrap_or_else(|| panic!("{what}: {actual}"));
    assert!(
        (actual - expected).abs() <= 1e-9 * expected.abs(),
        "{what}: {actual}, expected {expected} (off by {:e})",
        (actual - expected) / expected
    );
}

/// The body is valid, also to the mesh-level check, and has this volume
/// and area (given for scale 1).
#[track_caller]
fn assert_solid(
    session: &mut Session,
    name: &str,
    volume: f64,
    area: Option<f64>,
    k: f64,
) -> Value {
    let found = body(session, name);
    assert_eq!(found["valid"], true, "{found}");
    assert_close(&found["volume"], volume * k * k * k, "volume");
    if let Some(area) = area {
        assert_close(&found["area"], area * k * k, "area");
    }
    let check = run(session, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
    assert_eq!(check["problems"], json!([]), "{check}");
    found
}

fn names_of(body: &Value) -> Vec<String> {
    body["faceNames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap().to_string())
        .collect()
}

fn point(value: &Value) -> [f64; 3] {
    let list = value.as_array().unwrap();
    [
        list[0].as_f64().unwrap(),
        list[1].as_f64().unwrap(),
        list[2].as_f64().unwrap(),
    ]
}

/// The names of the two faces of the edge of `feature` that `document.edges`
/// lists with its ends nearest `a` and `b` (given for scale 1): what an
/// agent reads to refer to an edge.
fn edge_between(session: &mut Session, feature: &str, k: f64, a: [f64; 3], b: [f64; 3]) -> Value {
    let listed = run(session, "document.edges", json!({"feature": feature}));
    let far = |p: [f64; 3], q: [f64; 3]| {
        ((p[0] - q[0] * k).powi(2) + (p[1] - q[1] * k).powi(2) + (p[2] - q[2] * k).powi(2)).sqrt()
    };
    let (edge, off) = listed["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|edge| {
            let (start, end) = (point(&edge["start"]), point(&edge["end"]));
            let one = far(start, a) + far(end, b);
            let other = far(start, b) + far(end, a);
            (edge, one.min(other))
        })
        .min_by(|x, y| x.1.total_cmp(&y.1))
        .unwrap();
    assert!(off < 1e-6 * k, "no edge of {feature} from {a:?} to {b:?}");
    edge["faces"].clone()
}

fn block(session: &mut Session, k: f64) {
    built(
        session,
        "feature.block",
        json!({"name": "Base", "size": [4.0 * k, 3.0 * k, 2.0 * k]}),
    );
}

// Faces of a block: 0 bottom, 1 top, 2 front (-y), 3 back (+y), 4 left
// (-x), 5 right (+x).
const TOP: &str = "F1:face(1)";
const FRONT: &str = "F1:face(2)";
const LEFT: &str = "F1:face(4)";
const RIGHT: &str = "F1:face(5)";
const BOTTOM: &str = "F1:face(0)";
const BACK: &str = "F1:face(3)";

// ------------------------------------------------------------- one distance

#[test]
fn one_edge_of_a_block_loses_a_prism() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        block(&mut s, k);
        let d = 0.5;
        built(
            &mut s,
            "feature.chamfer",
            json!({"edges": [[TOP, FRONT]], "distance": d * k, "name": "Bevel"}),
        );
        // The top and the front lose a strip of d by 4 each, the two ends
        // a triangle each, and the new face is d * sqrt(2) wide.
        let area = 52.0 - 2.0 * d * 4.0 - d * d + 2.0_f64.sqrt() * d * 4.0;
        let solid = assert_solid(&mut s, "Bevel", 24.0 - d * d / 2.0 * 4.0, Some(area), k);
        assert_eq!(
            (&solid["faces"], &solid["edges"], &solid["vertices"]),
            (&json!(7), &json!(15), &json!(10))
        );
        let mut names = names_of(&solid);
        names.sort();
        assert_eq!(
            names,
            [
                "F2:chamfer(0)",
                "F2:copy(0)/F1:face(0)",
                "F2:copy(0)/F1:face(1)",
                "F2:copy(0)/F1:face(2)",
                "F2:copy(0)/F1:face(3)",
                "F2:copy(0)/F1:face(4)",
                "F2:copy(0)/F1:face(5)",
            ]
        );
        // The block's body went into the chamfer.
        assert_eq!(feature(&mut s, "Base")["bodyUsedBy"], "F2");
        // Every edge and vertex has a name of its own, and those the block
        // had answer to the name they had there.
        let listed = run(&mut s, "document.edges", json!({"feature": "Bevel"}));
        for key in ["edges", "vertices"] {
            let mut names: Vec<&str> = listed[key]
                .as_array()
                .unwrap()
                .iter()
                .map(|e| e["name"].as_str().unwrap())
                .collect();
            let count = names.len();
            names.sort_unstable();
            names.dedup();
            assert_eq!(names.len(), count, "{key}: {listed}");
        }
        let kept = listed["edges"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| {
                e["aliases"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|n| n.as_str().unwrap().contains("F1:edge(")))
            })
            .count();
        assert_eq!(kept, 11, "{listed}");
    }
}

#[test]
fn all_twelve_edges_of_a_block_with_their_eight_corners() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        block(&mut s, k);
        let d = 0.25;
        let pairs = [
            (TOP, FRONT),
            (TOP, BACK),
            (TOP, LEFT),
            (TOP, RIGHT),
            (BOTTOM, FRONT),
            (BOTTOM, BACK),
            (BOTTOM, LEFT),
            (BOTTOM, RIGHT),
            (FRONT, LEFT),
            (FRONT, RIGHT),
            (BACK, LEFT),
            (BACK, RIGHT),
        ];
        let edges: Vec<Value> = pairs.iter().map(|(a, b)| json!([a, b])).collect();
        built(
            &mut s,
            "feature.chamfer",
            json!({"edges": edges, "distance": d * k, "name": "Bevel"}),
        );
        // Twelve prisms, less what two share at each of 24 meetings, plus
        // what three share at each of 8 corners, less the tip each corner
        // face takes off: d^3 (1/3 * 3 - 1/4 + 1/12) per corner.
        let (l, w, h) = (4.0, 3.0, 2.0);
        let volume = l * w * h - 2.0 * d * d * (l + w + h) + 16.0 * d * d * d / 3.0;
        let (a, b, c) = (l - 2.0 * d, w - 2.0 * d, h - 2.0 * d);
        let area = 2.0 * (a * b + a * c + b * c)
            + 4.0 * 2.0_f64.sqrt() * d * (a + b + c)
            + 8.0 * 3.0_f64.sqrt() / 2.0 * d * d;
        let solid = assert_solid(&mut s, "Bevel", volume, Some(area), k);
        assert_eq!(
            (&solid["faces"], &solid["edges"], &solid["vertices"]),
            (&json!(26), &json!(48), &json!(24))
        );
        let names = names_of(&solid);
        let count = |part: &str| names.iter().filter(|n| n.contains(part)).count();
        assert_eq!(count(":chamfer("), 12);
        assert_eq!(count(":chamferCorner("), 8);
        assert_eq!(count(":copy(0)/"), 6);
        // The corner where the first, third and ninth edge meet.
        assert!(
            names.contains(&"F2:chamferCorner(0,2,8)".to_string()),
            "{names:?}"
        );
    }
}

// ----------------------------------------- two distances, distance and angle

/// The vertices `datum.references` lists for `feature`.
fn vertices(session: &mut Session, feature: &str) -> Vec<[f64; 3]> {
    let listed = run(session, "datum.references", json!({"feature": feature}));
    listed["vertices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| point(&v["at"]))
        .collect()
}

#[track_caller]
fn assert_has_vertex(found: &[[f64; 3]], at: [f64; 3], k: f64) {
    let near = found
        .iter()
        .any(|p| (0..3).all(|i| (p[i] - at[i] * k).abs() <= 1e-9 * k));
    assert!(near, "no vertex at {at:?} (times {k}) among {found:?}");
}

#[test]
fn the_first_distance_is_along_the_first_face_of_the_pair() {
    for k in SCALES {
        let (d1, d2) = (0.9, 0.3);
        for (pair, on_top, on_front) in [([TOP, FRONT], d1, d2), ([FRONT, TOP], d2, d1)] {
            let mut s = session();
            new_document(&mut s, k);
            block(&mut s, k);
            built(
                &mut s,
                "feature.chamfer",
                json!({"edges": [pair], "distance": d1 * k, "second": d2 * k, "name": "Bevel"}),
            );
            let area = 52.0 - (d1 + d2) * 4.0 - d1 * d2 + d1.hypot(d2) * 4.0;
            assert_solid(&mut s, "Bevel", 24.0 - d1 * d2 / 2.0 * 4.0, Some(area), k);
            // The cut line on the top is `on_top` from the edge y = 0, the
            // one on the front `on_front` below the top.
            let found = vertices(&mut s, "Bevel");
            assert_has_vertex(&found, [0.0, on_top, 2.0], k);
            assert_has_vertex(&found, [4.0, 0.0, 2.0 - on_front], k);
        }
        // A distance and an angle: 30 degrees between the top and the new
        // face, so the front is cut d1 tan(30 deg) down.
        let mut s = session();
        new_document(&mut s, k);
        block(&mut s, k);
        built(
            &mut s,
            "feature.chamfer",
            json!({"edges": [[TOP, FRONT]], "distance": d1 * k, "angle": "30 deg", "name": "Bevel"}),
        );
        let down = d1 * (PI / 6.0).tan();
        assert_solid(&mut s, "Bevel", 24.0 - d1 * down / 2.0 * 4.0, None, k);
        let found = vertices(&mut s, "Bevel");
        assert_has_vertex(&found, [0.0, d1, 2.0], k);
        assert_has_vertex(&found, [4.0, 0.0, 2.0 - down], k);
        // The same pair the other way round: 30 degrees with the front.
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Bevel", "edges": [[FRONT, TOP]]}),
        );
        let found = vertices(&mut s, "Bevel");
        assert_has_vertex(&found, [0.0, down, 2.0], k);
        assert_has_vertex(&found, [4.0, 0.0, 2.0 - d1], k);
    }
}

// ------------------------------------------------------------ circular edges

/// The volume a triangle in the half-plane (r, z) sweeps in a full turn
/// about the axis (Pappus).
fn ring(corners: [[f64; 2]; 3]) -> f64 {
    let [a, b, c] = corners;
    let area = 0.5 * ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])).abs();
    2.0 * PI * (a[0] + b[0] + c[0]) / 3.0 * area
}

#[test]
fn the_rim_of_a_rod_becomes_a_cone() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        let (r, h) = (1.0, 3.0);
        built(
            &mut s,
            "feature.cylinder",
            json!({"name": "Rod", "radius": r * k, "height": h * k}),
        );
        // A cylinder's faces: 0 the wall, 1 the bottom, 2 the top.
        let (d1, d2) = (0.4, 0.25);
        built(
            &mut s,
            "feature.chamfer",
            json!({"edges": [["F1:face(2)", "F1:face(0)"]], "distance": d1 * k, "second": d2 * k, "name": "Bevel"}),
        );
        // What is left is the rod less a ring; the cone between radius r
        // and r - d1 has the slant hypot(d1, d2).
        let removed = ring([[r, h], [r - d1, h], [r, h - d2]]);
        let area = PI * r * r
            + 2.0 * PI * r * (h - d2)
            + PI * (r - d1) * (r - d1)
            + PI * (2.0 * r - d1) * d1.hypot(d2);
        let solid = assert_solid(&mut s, "Bevel", PI * r * r * h - removed, Some(area), k);
        assert_eq!(solid["faces"], 4);
        // Both ends, one distance: the frustum closed form at each.
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Bevel", "distance": d2 * k,
                   "edges": [["F1:face(2)", "F1:face(0)"], ["F1:face(1)", "F1:face(0)"]]}),
        );
        let removed = 2.0 * PI * d2 * d2 * (r - d2 / 3.0);
        assert_solid(&mut s, "Bevel", PI * r * r * h - removed, None, k);
    }
}

/// A plate with a hole right through it, a blind hole and a boss.
fn drilled_plate(session: &mut Session, k: f64) {
    built(
        session,
        "feature.block",
        json!({"name": "Plate", "size": [8.0 * k, 3.0 * k, 2.0 * k]}),
    );
    let round = |session: &mut Session, name: &str, x: f64, z: f64, height: f64| {
        built(
            session,
            "feature.cylinder",
            json!({"name": name, "position": [x * k, 1.5 * k, z * k], "radius": 0.5 * k, "height": height * k}),
        );
    };
    round(session, "Drill", 1.5, -1.0, 4.0);
    built(
        session,
        "feature.boolean",
        json!({"op": "subtract", "target": "Plate", "tool": "Drill", "name": "Through"}),
    );
    round(session, "Bit", 4.0, 1.0, 2.0);
    built(
        session,
        "feature.boolean",
        json!({"op": "subtract", "target": "Through", "tool": "Bit", "name": "Blind"}),
    );
    round(session, "Stud", 6.5, 2.0, 1.0);
    built(
        session,
        "feature.boolean",
        json!({"op": "union", "target": "Blind", "tool": "Stud", "name": "Part"}),
    );
}

#[test]
fn the_mouth_of_a_hole_loses_material_and_inside_rims_gain_it() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        drilled_plate(&mut s, k);
        let (r, d) = (0.5, 0.2);
        let plain = 48.0 - PI * r * r * 2.0 - PI * r * r * 1.0 + PI * r * r * 1.0;
        assert_solid(&mut s, "Part", plain, None, k);
        // The rims, as document.edges lists them: closed circles.
        let rim = |s: &mut Session, x: f64, z: f64| {
            edge_between(s, "Part", k, [x + r, 1.5, z], [x + r, 1.5, z])
        };
        let mouth = rim(&mut s, 1.5, 2.0);
        let blind_mouth = rim(&mut s, 4.0, 2.0);
        let floor = rim(&mut s, 4.0, 1.0);
        let foot = rim(&mut s, 6.5, 2.0);
        let crown = rim(&mut s, 6.5, 3.0);
        built(
            &mut s,
            "feature.chamfer",
            json!({"edges": [mouth], "distance": d * k, "name": "Bevel"}),
        );
        // The mouth of a hole is an outside edge of the plate: the ring
        // outside the hole's wall goes.
        let outside = ring([[r, 2.0], [r + d, 2.0], [r, 2.0 - d]]);
        let inside = ring([[r, 1.0], [r - d, 1.0], [r, 1.0 + d]]);
        assert_solid(&mut s, "Bevel", plain - outside, None, k);
        // All five at once: two mouths and the crown lose, the floor of
        // the blind hole and the foot of the boss are inside corners and
        // gain.
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Bevel", "edges": [mouth, blind_mouth, floor, foot, crown]}),
        );
        let crown_ring = ring([[r, 3.0], [r - d, 3.0], [r, 3.0 - d]]);
        let foot_ring = ring([[r, 2.0], [r + d, 2.0], [r, 2.0 + d]]);
        let volume = plain - 2.0 * outside - crown_ring + inside + foot_ring;
        let solid = assert_solid(&mut s, "Bevel", volume, None, k);
        let names = names_of(&solid);
        for index in 0..5 {
            assert!(names.contains(&format!("F8:chamfer({index})")), "{names:?}");
        }
    }
}

// ------------------------------------------------------------ inside corners

/// An L: the outline (0,0) (4,0) (4,1.5) (1.5,1.5) (1.5,3) (0,3), 2 high.
fn ell(session: &mut Session, k: f64) {
    run(session, "sketch.create", json!({"name": "Outline"}));
    let points: Vec<[f64; 2]> = [
        [0.0, 0.0],
        [4.0, 0.0],
        [4.0, 1.5],
        [1.5, 1.5],
        [1.5, 3.0],
        [0.0, 3.0],
    ]
    .iter()
    .map(|p: &[f64; 2]| [p[0] * k, p[1] * k])
    .collect();
    run(
        session,
        "sketch.polyline",
        json!({"sketch": "Outline", "closed": true, "points": points}),
    );
    built(
        session,
        "feature.extrude",
        json!({"sketch": "Outline", "distance": 2.0 * k, "name": "Ell"}),
    );
}

#[test]
fn the_inside_corner_of_an_l_gains_a_prism() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        ell(&mut s, k);
        let d = 0.4;
        let inner = edge_between(&mut s, "Ell", k, [1.5, 1.5, 0.0], [1.5, 1.5, 2.0]);
        let ledge = edge_between(&mut s, "Ell", k, [1.5, 1.5, 2.0], [1.5, 3.0, 2.0]);
        let outer = edge_between(&mut s, "Ell", k, [0.0, 0.0, 0.0], [0.0, 0.0, 2.0]);
        built(
            &mut s,
            "feature.chamfer",
            json!({"edges": [inner], "distance": d * k, "name": "Web"}),
        );
        let area = 2.0 * 8.25 + 14.0 * 2.0 - 2.0 * d * 2.0 + d * d + 2.0_f64.sqrt() * d * 2.0;
        let solid = assert_solid(&mut s, "Web", 16.5 + d * d / 2.0 * 2.0, Some(area), k);
        assert_eq!(solid["faces"], 9);
        // An outside edge that ends in the inside corner: it stops against
        // the wall across it, which grows by a triangle and stays one
        // face.
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Web", "edges": [ledge]}),
        );
        let solid = assert_solid(&mut s, "Web", 16.5 - d * d / 2.0 * 1.5, None, k);
        assert_eq!(solid["faces"], 9);
        // The inside corner and an outside corner at once.
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Web", "edges": [inner, outer]}),
        );
        assert_solid(&mut s, "Web", 16.5 + d * d - d * d, None, k);
        // The inside corner and the ledge that ends in it, one gaining and
        // one losing at the same vertex: not built, and refused rather
        // than made into something with a stray face.
        let reason = failed(
            &mut s,
            "feature.edit",
            json!({"feature": "Web", "edges": [inner, ledge]}),
        );
        assert!(reason.starts_with("edges.1 (between "), "{reason}");
        assert!(reason.contains("does not end cleanly"), "{reason}");
    }
}

// ------------------------------------------------------------------- chains

/// A slot: half circles of radius 1 about (0, 0) and (3, 0), 2 high.
fn slot(session: &mut Session, k: f64) {
    run(session, "sketch.create", json!({"name": "Outline"}));
    run(
        session,
        "sketch.path",
        json!({"sketch": "Outline", "start": [0.0, -k], "segments": [
            {"line": [3.0 * k, -k]},
            {"arc": [3.0 * k, k], "center": [3.0 * k, 0.0]},
            {"line": [0.0, k]},
            {"arc": "start", "center": [0.0, 0.0]},
        ]}),
    );
    built(
        session,
        "feature.extrude",
        json!({"sketch": "Outline", "distance": 2.0 * k, "name": "Slot"}),
    );
}

#[test]
fn a_chain_of_lines_and_arcs_is_picked_by_one_of_its_edges() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        slot(&mut s, k);
        let whole = (6.0 + PI) * 2.0;
        assert_solid(&mut s, "Slot", whole, None, k);
        // One straight edge of the top rim, the top face first.
        let pair = edge_between(&mut s, "Slot", k, [0.0, -1.0, 2.0], [3.0, -1.0, 2.0]);
        let names: Vec<&str> = pair
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_str().unwrap())
            .collect();
        let top = names.iter().find(|n| n.contains("endCap")).unwrap();
        let wall = names.iter().find(|n| n.contains("side")).unwrap();
        let (d1, d2) = (0.3, 0.2);
        built(
            &mut s,
            "feature.chamfer",
            json!({"edges": [[top, wall]], "distance": d1 * k, "second": d2 * k, "chain": true, "name": "Rim"}),
        );
        // Two prisms 3 long and a ring in two halves.
        let removed =
            2.0 * 3.0 * d1 * d2 / 2.0 + ring([[1.0, 2.0], [1.0 - d1, 2.0], [1.0, 2.0 - d2]]);
        let solid = assert_solid(&mut s, "Rim", whole - removed, None, k);
        assert_eq!(solid["faces"], 10);
        let names = names_of(&solid);
        for index in 0..4 {
            assert!(
                names.contains(&format!("F3:chamfer(0,{index})")),
                "{names:?}"
            );
        }
        // Without the chain the one edge would stop in the middle of the
        // rim: refused, with the way out.
        let reason = failed(
            &mut s,
            "feature.edit",
            json!({"feature": "Rim", "chain": false}),
        );
        assert!(reason.starts_with("edges.0 (between "), "{reason}");
        assert!(reason.contains("does not end cleanly"), "{reason}");
        assert!(
            reason.contains("chamfer the edges it continues in as well"),
            "{reason}"
        );
    }
}

// ----------------------------------------------------------- what is refused

#[test]
fn a_distance_too_large_names_the_edge_the_face_and_what_fits() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        block(&mut s, k);
        // The top is 3 deep from its front edge; the front 2 high.
        let reason = failed(
            &mut s,
            "feature.chamfer",
            json!({"edges": [[TOP, FRONT]], "distance": 2.5 * k, "name": "Bevel"}),
        );
        assert!(
            reason.starts_with(
                "size.distance: too large for edges.0 (between F1:face(1) and F1:face(2))"
            ),
            "{reason}"
        );
        assert!(
            reason.contains("along the face F1:face(2), leaves that face"),
            "{reason}"
        );
        assert!(
            reason.contains(&format!(
                "the largest distance that fits there is below {} mm",
                2.0 * k
            )),
            "{reason}"
        );
        assert_eq!(inspect(&mut s)["bodies"].as_array().unwrap().len(), 1);
        // The second distance is the one that does not fit.
        let reason = failed(
            &mut s,
            "feature.edit",
            json!({"feature": "Bevel", "distance": 0.5 * k, "second": 2.5 * k}),
        );
        assert!(
            reason.starts_with("size.second: too large for edges.0"),
            "{reason}"
        );
        // Two chamfers that would meet in the middle of the top.
        let reason = failed(
            &mut s,
            "feature.edit",
            json!({"feature": "Bevel", "distance": 1.6 * k, "second": 0.5 * k,
                   "edges": [[TOP, FRONT], [TOP, BACK]]}),
        );
        assert!(reason.contains("too large for edges."), "{reason}");
        assert!(reason.contains("the face F1:face(1)"), "{reason}");
        assert!(
            reason.contains(&format!("below {} mm", 1.4 * k)),
            "{reason}"
        );
        // A size that fits repairs it.
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Bevel", "distance": 1.0 * k, "second": 0.5 * k}),
        );
        assert_solid(&mut s, "Bevel", 24.0 - 2.0 * 0.5 * 4.0 / 2.0, None, k);
    }
}

#[test]
fn bad_parameters_are_errors_and_what_is_not_built_says_its_kind() {
    let mut s = session();
    let k = 1.0;
    // No document.
    refused(
        &mut s,
        "feature.chamfer",
        json!({"edges": [[TOP, FRONT]], "distance": 1}),
    );
    new_document(&mut s, k);
    block(&mut s, k);
    for params in [
        json!({}),
        json!({"distance": 1}),
        json!({"edges": [[TOP, FRONT]]}),
        json!({"edges": [], "distance": 1}),
        json!({"edges": "all", "distance": 1}),
        json!({"edges": [TOP, FRONT], "distance": 1}),
        json!({"edges": [[TOP]], "distance": 1}),
        json!({"edges": [[TOP, FRONT, LEFT]], "distance": 1}),
        json!({"edges": [[TOP, 3]], "distance": 1}),
        json!({"edges": [[TOP, "roof"]], "distance": 1}),
        json!({"edges": [[TOP, "F1:edge(0)"]], "distance": 1}),
        json!({"edges": [[TOP, TOP]], "distance": 1}),
        json!({"edges": [[TOP, FRONT], [FRONT, TOP]], "distance": 1}),
        json!({"edges": [[TOP, "F9:face(0)"]], "distance": 1}),
        json!({"edges": [[TOP, FRONT]], "distance": 1, "second": 1, "angle": 30}),
        json!({"edges": [[TOP, FRONT]], "distance": [1]}),
        json!({"edges": [[TOP, FRONT]], "distance": 1, "chain": "yes"}),
        json!({"edges": [[TOP, FRONT]], "distance": 1, "radius": 2}),
    ] {
        refused(&mut s, "feature.chamfer", params);
    }
    // Accepted, and failed with the parameter.
    for (params, parameter) in [
        (json!({"distance": 0}), "size.distance"),
        (json!({"distance": -1}), "size.distance"),
        (json!({"distance": "1 deg"}), "size.distance"),
        (json!({"distance": "nothing"}), "size.distance"),
        (json!({"distance": 1, "second": 0}), "size.second"),
        (json!({"distance": 1, "angle": 0}), "size.angle"),
        (json!({"distance": 1, "angle": "1 mm"}), "size.angle"),
        (json!({"distance": 1e-9}), "size"),
        // 90 degrees against a square corner leaves no triangle.
        (json!({"distance": 1, "angle": 90}), "edges.0"),
    ] {
        let mut params = params;
        params["edges"] = json!([[TOP, FRONT]]);
        params["name"] = json!("Bevel");
        let reason = failed(&mut s, "feature.chamfer", params.clone());
        assert!(reason.starts_with(parameter), "{params}: {reason}");
        run(&mut s, "feature.delete", json!({"feature": "Bevel"}));
    }
    // Faces that are there and do not meet.
    let reason = failed(
        &mut s,
        "feature.chamfer",
        json!({"edges": [[TOP, BOTTOM]], "distance": 1, "name": "Bevel"}),
    );
    assert!(
        reason.starts_with("edges.0: the faces F1:face(1) and F1:face(0) of `Base` do not meet"),
        "{reason}"
    );
    // A face that is not there.
    let reason = failed(
        &mut s,
        "feature.edit",
        json!({"feature": "Bevel", "edges": [[TOP, "F1:face(9)"]]}),
    );
    assert!(
        reason.starts_with(
            "edges.0: `Base` no longer has a face F1:face(9); its faces are: F1:face(0)"
        ),
        "{reason}"
    );
    // Its own faces; and faces of a feature that is not there.
    let own = feature(&mut s, "Bevel")["id"].as_str().unwrap().to_string();
    let reason = failed(
        &mut s,
        "feature.edit",
        json!({"feature": "Bevel",
               "edges": [[format!("{own}:chamfer(0)"), format!("{own}:copy(0)/F1:face(1)")]]}),
    );
    assert!(
        reason.starts_with("edges: a chamfer cannot be of its own edges"),
        "{reason}"
    );
    let reason = failed(
        &mut s,
        "feature.edit",
        json!({"feature": "Bevel", "edges": [["F99:face(0)", "F99:face(1)"]]}),
    );
    assert!(
        reason.starts_with("edges.0: there is no feature F99 for F99:face(0) and F99:face(1)"),
        "{reason}"
    );
    run(&mut s, "feature.delete", json!({"feature": "Bevel"}));

    // A straight edge between a flat face and a round one, and the edge
    // of a spline wall: not built, and said so.
    let mut s = session();
    new_document(&mut s, k);
    run(&mut s, "sketch.create", json!({"name": "Outline"}));
    run(
        &mut s,
        "sketch.path",
        json!({"sketch": "Outline", "start": [0.0, 0.0], "segments": [
            {"line": [4.0, 0.0]},
            {"line": [4.0, 2.0]},
            {"arc": [0.0, 2.0], "center": [2.0, 2.0]},
            {"line": "start"},
        ]}),
    );
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Outline", "distance": 2.0, "name": "Arch"}),
    );
    let upright = edge_between(&mut s, "Arch", k, [4.0, 2.0, 0.0], [4.0, 2.0, 2.0]);
    let reason = failed(
        &mut s,
        "feature.chamfer",
        json!({"edges": [upright], "distance": 0.2, "name": "Bevel"}),
    );
    assert!(reason.starts_with("edges.0 (between "), "{reason}");
    assert!(reason.contains("tangent along this edge"), "{reason}");

    let mut s = session();
    new_document(&mut s, k);
    run(&mut s, "sketch.create", json!({"name": "Outline"}));
    run(
        &mut s,
        "sketch.spline",
        json!({"sketch": "Outline", "control": [[0.0, 0.0], [1.0, 2.0], [3.0, 2.0], [4.0, 0.0]], "degree": 3}),
    );
    run(
        &mut s,
        "sketch.line",
        json!({"sketch": "Outline", "from": "e4", "to": "e1"}),
    );
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Outline", "distance": 2.0, "name": "Hump"}),
    );
    let hump = body(&mut s, "Hump");
    let names = names_of(&hump);
    let wall = names.iter().find(|n| n.contains("side(e5)")).unwrap();
    let top = names.iter().find(|n| n.contains("endCap")).unwrap();
    // The edge along the spline wall is freeform: its chamfer is a ruled
    // spline surface (kernel::blend's `rolling`). Across the edge the wall
    // is upright and the top flat, so the chamfer cuts away the right
    // triangle with legs d: the point s in from the spline along its
    // normal loses the height d − s. Over the strip along the spline
    // (whose area element is |c'| (1 − κ s) ds dt, κ its curvature) and
    // where the strip is still above the base line y = 0, by Simpson's
    // rule along the spline and in closed form across it.
    let d = 0.2;
    built(
        &mut s,
        "feature.chamfer",
        json!({"edges": [[top, wall]], "distance": d, "name": "Bevel"}),
    );
    {
        // c(t) and its derivatives.
        let c = |t: f64| {
            let x = 3.0 * t * (1.0 - t).powi(2) + 9.0 * t * t * (1.0 - t) + 4.0 * t.powi(3);
            let y = 6.0 * t * (1.0 - t);
            let dx = 3.0 + 6.0 * t - 6.0 * t * t;
            let dy = 6.0 - 12.0 * t;
            let ddx = 6.0 - 12.0 * t;
            let ddy = -12.0;
            (x, y, dx, dy, ddx, ddy)
        };
        let column = |t: f64| {
            let (_, y, dx, dy, ddx, ddy) = c(t);
            let speed = dx.hypot(dy);
            // The normal into the hump (below the arch) and the curvature
            // that shrinks the strip on that side.
            let ny = -dx / speed;
            let kappa = (dy * ddx - dx * ddy) / speed.powi(3);
            let reach = if ny < 0.0 { (y / -ny).min(d) } else { d };
            // ∫ (1 − κ s)(d − s) ds from 0 to reach.
            let m = reach;
            let inner = d * m - m * m / 2.0 - kappa * (d * m * m / 2.0 - m.powi(3) / 3.0);
            speed * inner
        };
        let n = 20_000;
        let h = 1.0 / f64::from(n);
        let cut = ((1..n)
            .map(|i| f64::from(if i % 2 == 1 { 4 } else { 2 }) * column(f64::from(i) * h))
            .sum::<f64>()
            + column(0.0)
            + column(1.0))
            * h
            / 3.0;
        let before = hump["volume"].as_f64().unwrap();
        let after = body(&mut s, "Bevel")["volume"].as_f64().unwrap();
        assert!(
            ((before - after) - cut).abs() <= 1e-7 * cut,
            "the chamfer took {} instead of {cut}",
            before - after
        );
        let check = run(&mut s, "document.check", json!({"deep": true}));
        assert_eq!(check["ok"], true, "{check}");
    }
    // The flat edge of the same body is an edge like any other: the
    // freeform wall is only what its ends run into. The spline is one
    // cubic Bezier piece, x(t) = 3t(1-t)^2 + 9t^2(1-t) + 4t^3 and
    // y(t) = 6t(1-t), so the part is as wide as 4 - 2x at the height y,
    // and the chamfer takes (d - y) of it there: both integrals by
    // Simpson's rule, written out here.
    let flat = names.iter().find(|n| n.contains("side(e6)")).unwrap();
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "Bevel", "edges": [[top, flat]], "distance": d}),
    );
    let x = |t: f64| 3.0 * t * (1.0 - t).powi(2) + 9.0 * t * t * (1.0 - t) + 4.0 * t.powi(3);
    let dx = |t: f64| 3.0 * (1.0 - t).powi(2) + 12.0 * t * (1.0 - t) + 3.0 * t * t;
    let simpson = |f: &dyn Fn(f64) -> f64, from: f64, to: f64| {
        let n = 20_000;
        let h = (to - from) / f64::from(n);
        let inner: f64 = (1..n)
            .map(|i| f64::from(if i % 2 == 1 { 4 } else { 2 }) * f(from + f64::from(i) * h))
            .sum();
        (f(from) + inner + f(to)) * h / 3.0
    };
    let profile = simpson(&|t| 6.0 * t * (1.0 - t) * dx(t), 0.0, 1.0);
    let width = |y: f64| 4.0 - 2.0 * x((1.0 - (1.0 - 2.0 * y / 3.0).sqrt()) / 2.0);
    let removed = simpson(&|y| (d - y) * width(y), 0.0, d);
    let solid = body(&mut s, "Bevel");
    let volume = solid["volume"].as_f64().unwrap();
    let expected = profile * 2.0 - removed;
    assert!(
        (volume - expected).abs() <= 1e-8 * expected,
        "{volume} is not {expected}"
    );
    let check = run(&mut s, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
}

// ----------------------------------------------------- names and the history

#[test]
fn the_chamfer_follows_an_edge_that_moves_and_fails_by_name_when_it_goes() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        run(
            &mut s,
            "variable.set",
            json!({"name": "width", "expr": 4.0 * k}),
        );
        run(
            &mut s,
            "variable.set",
            json!({"name": "bevel", "expr": 0.5 * k}),
        );
        run(
            &mut s,
            "variable.set",
            json!({"name": "reach", "expr": 1.0 * k}),
        );
        built(
            &mut s,
            "feature.block",
            json!({"name": "Base", "size": ["width", 3.0 * k, 2.0 * k]}),
        );
        // A notch in the front, as deep as `reach`, from x = 1 to 2.
        built(
            &mut s,
            "feature.block",
            json!({"name": "Cutter", "position": [k, -k, -k],
                   "size": [1.0 * k, format!("reach + {}", k), 4.0 * k]}),
        );
        let made = built(
            &mut s,
            "feature.boolean",
            json!({"op": "subtract", "target": "Base", "tool": "Cutter", "name": "Notched"}),
        );
        assert_eq!(made["id"], "F3");
        // The edge where the notch's back wall meets the top; the back
        // wall is the cutter's +y face.
        let top = "F3:copy(0)/F1:face(1)";
        let wall = "F3:copy(1)/F2:face(3)";
        built(
            &mut s,
            "feature.chamfer",
            json!({"edges": [[top, wall]], "distance": "bevel", "name": "Bevel"}),
        );
        // A sketch on the chamfer face, so that something is made from it.
        built(
            &mut s,
            "sketch.create",
            json!({"name": "Mark", "face": "F4:chamfer(0)"}),
        );
        let d = 0.5;
        let notched = |width: f64, reach: f64| width * 3.0 * 2.0 - reach * 2.0;
        assert_solid(&mut s, "Bevel", notched(4.0, 1.0) - d * d / 2.0, None, k);
        // Upstream edits that move the edge: the block grows, the notch
        // deepens, the bevel follows a variable.
        run(
            &mut s,
            "variable.set",
            json!({"name": "width", "expr": 6.0 * k}),
        );
        run(
            &mut s,
            "variable.set",
            json!({"name": "reach", "expr": 1.5 * k}),
        );
        run(
            &mut s,
            "variable.set",
            json!({"name": "bevel", "expr": 0.25 * k}),
        );
        assert_eq!(feature(&mut s, "Bevel")["status"], "ok");
        assert_eq!(feature(&mut s, "Mark")["status"], "ok");
        assert_solid(
            &mut s,
            "Bevel",
            notched(6.0, 1.5) - 0.25 * 0.25 / 2.0,
            None,
            k,
        );
        // An edit that removes the edge: the notch goes right through, so
        // its back wall is gone. The chamfer fails with the parameter, the
        // feature and the name it lost; the sketch on it names the
        // chamfer; the notched body is its own again.
        run(
            &mut s,
            "variable.set",
            json!({"name": "reach", "expr": 4.0 * k}),
        );
        let bevel = feature(&mut s, "Bevel");
        assert_eq!(bevel["status"], "failed", "{bevel}");
        let reason = bevel["error"].as_str().unwrap();
        assert!(
            reason.starts_with(
                "edges.0: `Notched` no longer has a face F3:copy(1)/F2:face(3); its faces are: "
            ),
            "{reason}"
        );
        let mark = feature(&mut s, "Mark");
        assert_eq!(mark["status"], "failed", "{mark}");
        assert!(
            mark["error"].as_str().unwrap().contains("`Bevel`"),
            "{mark}"
        );
        assert_eq!(mark["causedBy"], "F4", "{mark}");
        let document = inspect(&mut s);
        let bodies: Vec<&str> = document["bodies"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["name"].as_str().unwrap())
            .collect();
        assert_eq!(bodies, ["Notched"], "{document}");
        // Putting the edge back repairs both.
        run(
            &mut s,
            "variable.set",
            json!({"name": "reach", "expr": 1.5 * k}),
        );
        assert_eq!(feature(&mut s, "Bevel")["status"], "ok");
        assert_eq!(feature(&mut s, "Mark")["status"], "ok");
        // And so does pointing the chamfer at another edge, with the
        // notch through: the edge between the top and the notch's side.
        run(
            &mut s,
            "variable.set",
            json!({"name": "reach", "expr": 4.0 * k}),
        );
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Bevel", "edges": [[top, "F3:copy(1)/F2:face(4)"]]}),
        );
        assert_eq!(feature(&mut s, "Mark")["status"], "ok");
        let split = 6.0 * 3.0 * 2.0 - 3.0 * 2.0;
        assert_solid(&mut s, "Bevel", split - 0.25 * 0.25 / 2.0 * 3.0, None, k);
        // The repaired document is the one built that way from the start.
        let repaired = inspect(&mut s)["bodies"].clone();
        let mut fresh = session();
        new_document(&mut fresh, k);
        run(
            &mut fresh,
            "variable.set",
            json!({"name": "width", "expr": 6.0 * k}),
        );
        run(
            &mut fresh,
            "variable.set",
            json!({"name": "bevel", "expr": 0.25 * k}),
        );
        run(
            &mut fresh,
            "variable.set",
            json!({"name": "reach", "expr": 4.0 * k}),
        );
        built(
            &mut fresh,
            "feature.block",
            json!({"name": "Base", "size": ["width", 3.0 * k, 2.0 * k]}),
        );
        built(
            &mut fresh,
            "feature.block",
            json!({"name": "Cutter", "position": [k, -k, -k],
                   "size": [1.0 * k, format!("reach + {}", k), 4.0 * k]}),
        );
        built(
            &mut fresh,
            "feature.boolean",
            json!({"op": "subtract", "target": "Base", "tool": "Cutter", "name": "Notched"}),
        );
        built(
            &mut fresh,
            "feature.chamfer",
            json!({"edges": [[top, "F3:copy(1)/F2:face(4)"]], "distance": "bevel", "name": "Bevel"}),
        );
        assert_eq!(inspect(&mut fresh)["bodies"], repaired);
    }
}

#[test]
fn a_sketch_on_the_chamfer_face_and_a_chamfer_on_an_edge_of_a_chamfer() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        block(&mut s, k);
        let d = 1.0;
        built(
            &mut s,
            "feature.chamfer",
            json!({"edges": [[TOP, FRONT]], "distance": d * k, "name": "Bevel"}),
        );
        // The face's frame: z out of the body; x along the edge, in the
        // direction of (into the top) x (into the front), which is -x of
        // the block; the origin where the cut line on the first face (the
        // top) starts, at (4, d, 2); so y runs down the face, which is
        // d sqrt(2) wide. A boss at (1, half the width) of the sketch
        // stands at x = 3 in the middle of the face.
        built(
            &mut s,
            "sketch.create",
            json!({"name": "Mark", "face": "F2:chamfer(0)"}),
        );
        let across = d * 2.0_f64.sqrt() / 2.0;
        let (r, tall) = (0.3, 0.5);
        run(
            &mut s,
            "sketch.circle",
            json!({"sketch": "Mark", "center": [1.0 * k, across * k], "radius": r * k}),
        );
        built(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Mark", "distance": tall * k, "join": "Bevel", "name": "Bossed"}),
        );
        let chamfered = 24.0 - d * d / 2.0 * 4.0;
        let boss = PI * r * r * tall;
        let solid = assert_solid(&mut s, "Bossed", chamfered + boss, None, k);
        // Where it stands: the centre of mass of the block, less the
        // prism, plus the boss, whose own centre is a quarter of a unit
        // out of the middle of the face along (0, -1, 1) / sqrt(2).
        let prism = d * d / 2.0 * 4.0;
        let out = tall / 2.0 * 0.5_f64.sqrt();
        let parts = [
            (24.0, [2.0, 1.5, 1.0]),
            (-prism, [2.0, d / 3.0, 2.0 - d / 3.0]),
            (boss, [3.0, d / 2.0 - out, 2.0 - d / 2.0 + out]),
        ];
        let total: f64 = parts.iter().map(|p| p.0).sum();
        let got = point(&solid["centroid"]);
        for axis in 0..3 {
            let want: f64 = parts.iter().map(|(v, c)| v * c[axis]).sum::<f64>() / total;
            assert!(
                (got[axis] - want * k).abs() <= 1e-9 * k,
                "centroid {got:?}, axis {axis} should be {want} times {k}"
            );
        }
        run(&mut s, "edit.undo", json!({}));
        run(&mut s, "edit.undo", json!({}));
        run(&mut s, "edit.undo", json!({}));
        // A second chamfer, on the edge between the first one's face and
        // the top: the two meet at 135 degrees.
        let e = 0.3;
        built(
            &mut s,
            "feature.chamfer",
            json!({"edges": [["F2:chamfer(0)", "F2:copy(0)/F1:face(1)"]], "distance": e * k, "name": "Eased"}),
        );
        let eased = chamfered - e * e / 2.0 * (PI * 0.75).sin() * 4.0;
        let solid = assert_solid(&mut s, "Eased", eased, None, k);
        assert_eq!(solid["faces"], 8);
        let names = names_of(&solid);
        assert!(names.contains(&"F3:chamfer(0)".to_string()), "{names:?}");
        assert!(
            names.contains(&"F3:copy(0)/F2:chamfer(0)".to_string()),
            "{names:?}"
        );
        assert!(
            names.contains(&"F3:copy(0)/F2:copy(0)/F1:face(1)".to_string()),
            "{names:?}"
        );
        // The first chamfer grows: the second follows its edge.
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Bevel", "distance": 1.5 * k}),
        );
        let chamfered = 24.0 - 1.5 * 1.5 / 2.0 * 4.0;
        let eased = chamfered - e * e / 2.0 * (PI * 0.75).sin() * 4.0;
        assert_solid(&mut s, "Eased", eased, None, k);
    }
}

#[test]
fn saved_and_reopened_the_bodies_are_the_same() {
    let dir = workdir("files");
    let mut s = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    let k = 1.0;
    new_document(&mut s, k);
    drilled_plate(&mut s, k);
    let mouth = edge_between(&mut s, "Part", k, [2.0, 1.5, 2.0], [2.0, 1.5, 2.0]);
    let foot = edge_between(&mut s, "Part", k, [7.0, 1.5, 2.0], [7.0, 1.5, 2.0]);
    let long = edge_between(&mut s, "Part", k, [0.0, 0.0, 2.0], [8.0, 0.0, 2.0]);
    let short = edge_between(&mut s, "Part", k, [0.0, 0.0, 2.0], [0.0, 3.0, 2.0]);
    let upright = edge_between(&mut s, "Part", k, [0.0, 0.0, 0.0], [0.0, 0.0, 2.0]);
    built(
        &mut s,
        "feature.chamfer",
        json!({"edges": [mouth, foot], "distance": 0.2, "second": 0.1, "name": "Rims"}),
    );
    // The straight edges are now edges of `Rims`: the same faces, one
    // feature on.
    let on = |pair: &Value| -> Value {
        pair.as_array()
            .unwrap()
            .iter()
            .map(|n| json!(format!("F8:copy(0)/{}", n.as_str().unwrap())))
            .collect()
    };
    built(
        &mut s,
        "feature.chamfer",
        json!({"edges": [on(&long), on(&short), on(&upright)], "distance": 0.3, "angle": "40 deg", "name": "Corner"}),
    );
    let before = inspect(&mut s);
    assert_eq!(before["bodies"].as_array().unwrap().len(), 1);
    // Undo and redo are exact.
    run(&mut s, "edit.undo", json!({}));
    assert_ne!(inspect(&mut s)["bodies"], before["bodies"]);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(inspect(&mut s)["bodies"], before["bodies"]);
    run(&mut s, "file.save", json!({"path": "chamfers.3dcraft"}));
    run(&mut s, "file.close", json!({}));
    run(&mut s, "file.open", json!({"path": "chamfers.3dcraft"}));
    let after = inspect(&mut s);
    assert_eq!(after["bodies"], before["bodies"]);
    assert_eq!(after["features"], before["features"]);
    let check = run(&mut s, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
}
