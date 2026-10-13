//! Reference geometry through the commands: datum planes at a distance,
//! at an angle and through three points, as what a sketch lies on and a
//! body is mirrored in. Every solid made on a datum is measured against a
//! closed form worked out by hand (its volume and area, and where it has
//! to be), at three scales, and checked deep; an upstream edit moves what
//! is placed by a datum; a lost reference fails the datum and what
//! depends on it, and nothing else.

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

type V = [f64; 3];

fn add(a: V, b: V) -> V {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn mul(a: V, s: f64) -> V {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn dot(a: V, b: V) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: V, b: V) -> V {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn unit(a: V) -> V {
    mul(a, 1.0 / dot(a, a).sqrt())
}

fn workdir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("datums-{name}"));
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

fn vector(value: &Value) -> V {
    let list = value.as_array().unwrap_or_else(|| panic!("{value}"));
    [
        list[0].as_f64().unwrap(),
        list[1].as_f64().unwrap(),
        list[2].as_f64().unwrap(),
    ]
}

/// Where a datum lies, as `document.inspect` reports it.
fn datum(session: &mut Session, name: &str) -> Value {
    let found = feature(session, name);
    assert_eq!(found["status"], "ok", "{found}");
    found["datum"].clone()
}

fn assert_close(actual: &Value, expected: f64, relative: f64, what: &str) {
    let actual = actual
        .as_f64()
        .unwrap_or_else(|| panic!("{what}: {actual}"));
    assert!(
        (actual - expected).abs() <= relative * expected.abs(),
        "{what}: {actual}, expected {expected} (off by {:e})",
        (actual - expected) / expected
    );
}

/// Within `slack` of each other, component by component.
fn assert_near(actual: V, expected: V, slack: f64, what: &str) {
    for axis in 0..3 {
        assert!(
            (actual[axis] - expected[axis]).abs() <= slack,
            "{what}: {actual:?}, expected {expected:?}"
        );
    }
}

fn assert_measures(body: &Value, volume: f64, area: f64, relative: f64, what: &str) {
    assert_close(
        &body["volume"],
        volume,
        relative,
        &format!("{what}: volume"),
    );
    assert_close(&body["area"], area, relative, &format!("{what}: area"));
}

/// Every body is valid and none passes through itself.
fn assert_sound(session: &mut Session, what: &str) {
    let check = run(session, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{what}: {check}");
}

/// The box of the corners `origin + x a + y b + n c` for a in {0, w}, b in
/// {0, h}, c in {0, d}: where a box drawn on a plane has to be.
fn box_corners(origin: V, x: V, y: V, n: V, size: V) -> (V, V) {
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for corner in 0..8 {
        let pick = |bit: usize, length: f64| if corner >> bit & 1 == 1 { length } else { 0.0 };
        let p = add(
            origin,
            add(
                mul(x, pick(0, size[0])),
                add(mul(y, pick(1, size[1])), mul(n, pick(2, size[2]))),
            ),
        );
        for axis in 0..3 {
            lo[axis] = lo[axis].min(p[axis]);
            hi[axis] = hi[axis].max(p[axis]);
        }
    }
    (lo, hi)
}

fn assert_bbox(body: &Value, expected: (V, V), slack: f64, what: &str) {
    assert_near(vector(&body["bbox"][0]), expected.0, slack, what);
    assert_near(vector(&body["bbox"][1]), expected.1, slack, what);
}

/// A w x h x d box drawn on `plane` from its origin and extruded along
/// its normal.
fn box_on(session: &mut Session, plane: &str, name: &str, size: V) -> Value {
    let sketch = format!("{name} profile");
    run(
        session,
        "sketch.create",
        json!({"name": sketch, "plane": plane}),
    );
    rectangle(session, &sketch, (0.0, size[0]), (0.0, size[1]));
    built(
        session,
        "feature.extrude",
        json!({"sketch": sketch, "distance": size[2], "name": name}),
    );
    body(session, name)
}

fn box_measures(solid: &Value, size: V, what: &str) {
    let [w, h, d] = size;
    assert_measures(solid, w * h * d, 2.0 * (w * h + w * d + h * d), 1e-11, what);
}

// ------------------------------------------------------------- at a distance

/// A box on a plane beside a base plane: the box's volume and area, and
/// it stands on the plane. The distance follows a variable.
#[test]
fn a_box_on_a_plane_at_a_distance_stands_on_it() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        run(
            &mut s,
            "variable.set",
            json!({"name": "lift", "expr": 7.0 * k}),
        );
        let size = [4.0 * k, 3.0 * k, 5.0 * k];
        let cases: [(&str, V, V, V); 3] = [
            ("xy", [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
            ("yz", [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
            ("zx", [0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ];
        for (base, x, y, n) in cases {
            let plane = format!("Above {base}");
            let made = built(
                &mut s,
                "datum.plane",
                json!({"from": base, "distance": "lift", "name": plane}),
            );
            assert_near(vector(&made["datum"]["origin"]), mul(n, 7.0 * k), 0.0, base);
            assert_near(vector(&made["datum"]["normal"]), n, 0.0, base);
            assert_near(vector(&made["datum"]["x"]), x, 0.0, base);
            let name = format!("Box {base}");
            let solid = box_on(&mut s, &plane, &name, size);
            box_measures(&solid, size, base);
            assert_bbox(
                &solid,
                box_corners(mul(n, 7.0 * k), x, y, n, size),
                1e-9 * k,
                base,
            );
        }
        assert_sound(&mut s, "boxes on offset planes");
        // The variable moves the planes, and the boxes with them; a
        // negative distance is the other side.
        run(
            &mut s,
            "variable.set",
            json!({"name": "lift", "expr": -2.0 * k}),
        );
        for (base, x, y, n) in cases {
            let solid = body(&mut s, &format!("Box {base}"));
            box_measures(&solid, size, base);
            assert_bbox(
                &solid,
                box_corners(mul(n, -2.0 * k), x, y, n, size),
                1e-9 * k,
                base,
            );
        }
        assert_sound(&mut s, "boxes on moved planes");
        // The distance itself is a parameter of the feature.
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Above xy", "distance": "lift + 3 * 1"}),
        );
        assert_eq!(
            feature(&mut s, "Above xy")["params"]["by"]["distance"],
            "lift + 3 * 1"
        );
    }
}

/// A plane beside a face of a body follows the face; a plane beside a
/// datum plane follows the datum.
#[test]
fn a_plane_at_a_distance_from_a_face_follows_the_face() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        run(
            &mut s,
            "variable.set",
            json!({"name": "tall", "expr": 10.0 * k}),
        );
        built(
            &mut s,
            "feature.block",
            json!({"size": [20.0 * k, 20.0 * k, "tall"], "name": "Base"}),
        );
        // 2 above the top of the block, and 3 above that.
        built(
            &mut s,
            "datum.plane",
            json!({"from": "F1:face(1)", "distance": 2.0 * k, "name": "Shelf"}),
        );
        built(
            &mut s,
            "datum.plane",
            json!({"from": "Shelf", "distance": 3.0 * k, "name": "Top shelf"}),
        );
        let lies = datum(&mut s, "Shelf");
        assert_near(vector(&lies["normal"]), [0.0, 0.0, 1.0], 0.0, "normal");
        assert_close(&lies["origin"][2], 12.0 * k, 1e-12, "above the top");
        run(
            &mut s,
            "sketch.create",
            json!({"name": "Round", "plane": "Top shelf"}),
        );
        circle(&mut s, "Round", [5.0 * k, 5.0 * k], 2.0 * k);
        built(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Round", "distance": 4.0 * k, "name": "Post"}),
        );
        let volume = PI * 4.0 * k * k * 4.0 * k;
        let area = 2.0 * PI * 4.0 * k * k + 2.0 * PI * 2.0 * k * 4.0 * k;
        let post = body(&mut s, "Post");
        assert_measures(&post, volume, area, 1e-11, "post");
        assert_close(&post["centroid"][2], 17.0 * k, 1e-11, "centroid");
        assert_sound(&mut s, "post");
        // The block grows: its top moves, the planes with it, the post
        // with them.
        run(
            &mut s,
            "variable.set",
            json!({"name": "tall", "expr": 25.0 * k}),
        );
        assert_close(
            &datum(&mut s, "Shelf")["origin"][2],
            27.0 * k,
            1e-12,
            "moved",
        );
        assert_close(
            &datum(&mut s, "Top shelf")["origin"][2],
            30.0 * k,
            1e-12,
            "moved",
        );
        let post = body(&mut s, "Post");
        assert_measures(&post, volume, area, 1e-11, "moved post");
        assert_close(&post["centroid"][2], 32.0 * k, 1e-11, "moved centroid");
        // Below the bottom: a face's normal points out of the body.
        built(
            &mut s,
            "datum.plane",
            json!({"from": "F1:face(0)", "distance": k, "name": "Under"}),
        );
        let under = datum(&mut s, "Under");
        assert_near(vector(&under["normal"]), [0.0, 0.0, -1.0], 0.0, "out");
        assert_close(&under["origin"][2], -k, 1e-12, "below");
        assert_sound(&mut s, "moved");
    }
}

// --------------------------------------------------------------- at an angle

/// A box on a plane turned about a world axis: the box's volume and area
/// and the bounding box of a box tilted by that angle. The angle follows
/// a variable.
#[test]
fn a_box_on_a_plane_at_an_angle_is_tilted_by_that_angle() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        run(
            &mut s,
            "variable.set",
            json!({"name": "tilt", "expr": "30 deg"}),
        );
        built(
            &mut s,
            "datum.plane",
            json!({"from": "xy", "about": "x", "angle": "tilt", "name": "Ramp"}),
        );
        let size = [4.0 * k, 3.0 * k, 5.0 * k];
        let solid = box_on(&mut s, "Ramp", "Wedge", size);
        // Turned counter-clockwise about +x by a: y goes to (0, cos a,
        // sin a) and the normal to (0, -sin a, cos a).
        let frame = |a: f64| -> (V, V, V) {
            (
                [1.0, 0.0, 0.0],
                [0.0, a.cos(), a.sin()],
                [0.0, -a.sin(), a.cos()],
            )
        };
        let expect = |s: &mut Session, degrees: f64| {
            let (x, y, n) = frame(degrees.to_radians());
            let lies = datum(s, "Ramp");
            assert_near(vector(&lies["normal"]), n, 1e-15, "normal");
            assert_near(vector(&lies["x"]), x, 1e-15, "x");
            assert_near(vector(&lies["origin"]), [0.0; 3], 1e-15 * k, "origin");
            let solid = body(s, "Wedge");
            box_measures(&solid, size, "tilted");
            assert_bbox(
                &solid,
                box_corners([0.0; 3], x, y, n, size),
                1e-9 * k,
                "tilted",
            );
        };
        box_measures(&solid, size, "wedge");
        expect(&mut s, 30.0);
        assert_sound(&mut s, "tilted box");
        for degrees in [-45.0, 90.0, 135.0, 300.0] {
            run(
                &mut s,
                "variable.set",
                json!({"name": "tilt", "expr": format!("{degrees} deg")}),
            );
            expect(&mut s, degrees);
        }
        assert_sound(&mut s, "turned box");
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Ramp", "angle": "10 deg"}),
        );
        expect(&mut s, 10.0);
    }
}

/// A plane turned about an edge of a body goes through that edge: a lid
/// hinged on the top of a block. The edge is the one between two faces.
#[test]
fn a_plane_turned_about_an_edge_is_hinged_on_it() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        run(
            &mut s,
            "variable.set",
            json!({"name": "side", "expr": 10.0 * k}),
        );
        built(
            &mut s,
            "feature.block",
            json!({"size": ["side", 8.0 * k, 6.0 * k], "name": "Base"}),
        );
        // The edge between the top (1) and the right (+x, 5) face: along
        // z cross x = +y, at x = side, z = 6.
        built(
            &mut s,
            "datum.plane",
            json!({"from": "F1:face(1)", "about": ["F1:face(1)", "F1:face(5)"],
                   "angle": "45 deg", "name": "Lid"}),
        );
        let hinge = |s: &mut Session, side: f64| {
            let lies = datum(s, "Lid");
            let root = 0.5_f64.sqrt();
            // z turned counter-clockwise about +y by 45 deg.
            assert_near(vector(&lies["normal"]), [root, 0.0, root], 1e-15, "normal");
            // The plane holds the edge: both of its ends.
            let origin = vector(&lies["origin"]);
            let normal = vector(&lies["normal"]);
            for y in [0.0, 8.0 * k] {
                let off = dot(sub([side, y, 6.0 * k], origin), normal);
                assert!(off.abs() <= 1e-12 * k, "the edge is {off} off the plane");
            }
            lies
        };
        hinge(&mut s, 10.0 * k);
        // A disc drawn on the lid, wherever the lid's origin is: its
        // closed-form volume and area, and its centre half its thickness
        // off the lid.
        run(
            &mut s,
            "sketch.create",
            json!({"name": "Round", "plane": "Lid"}),
        );
        circle(&mut s, "Round", [0.0, 0.0], 3.0 * k);
        built(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Round", "distance": 2.0 * k, "name": "Disc"}),
        );
        let disc = |s: &mut Session, lies: &Value| {
            let disc = body(s, "Disc");
            assert_measures(
                &disc,
                PI * 9.0 * 2.0 * k * k * k,
                2.0 * PI * 9.0 * k * k + 2.0 * PI * 3.0 * 2.0 * k * k,
                1e-11,
                "disc",
            );
            let centre = add(vector(&lies["origin"]), mul(vector(&lies["normal"]), k));
            assert_near(vector(&disc["centroid"]), centre, 1e-9 * k, "centre");
        };
        let lies = hinge(&mut s, 10.0 * k);
        disc(&mut s, &lies);
        assert_sound(&mut s, "disc on the lid");
        // The block gets wider: the edge moves, the lid with it.
        run(
            &mut s,
            "variable.set",
            json!({"name": "side", "expr": 14.0 * k}),
        );
        let lies = hinge(&mut s, 14.0 * k);
        disc(&mut s, &lies);
        // The other way round the edge, the faces in the other order: the
        // same plane seen from its other side turned the other way.
        built(
            &mut s,
            "datum.plane",
            json!({"from": "F1:face(1)", "about": ["F1:face(5)", "F1:face(1)"],
                   "angle": "-45 deg", "name": "Same lid"}),
        );
        let same = datum(&mut s, "Same lid");
        assert_near(
            vector(&same["normal"]),
            vector(&lies["normal"]),
            1e-15,
            "same lid",
        );
        assert_sound(&mut s, "moved lid");
    }
}

// ------------------------------------------------------ through three points

/// A box on the plane through three points: the frame worked out here
/// from the points, the box's volume and area, and where it is.
#[test]
fn a_box_on_a_plane_through_three_points() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        run(
            &mut s,
            "variable.set",
            json!({"name": "reach", "expr": 6.0 * k}),
        );
        built(
            &mut s,
            "datum.plane",
            json!({"through": [["reach", 0, 0], [0, 4.0 * k, 0], [0, 0, 3.0 * k]],
                   "name": "Slant"}),
        );
        let size = [2.0 * k, 3.0 * k, 1.5 * k];
        box_on(&mut s, "Slant", "Chip", size);
        let expect = |s: &mut Session, reach: f64| {
            let (a, b, c) = ([reach, 0.0, 0.0], [0.0, 4.0 * k, 0.0], [0.0, 0.0, 3.0 * k]);
            let x = unit(sub(b, a));
            let n = unit(cross(sub(b, a), sub(c, a)));
            let y = cross(n, x);
            let lies = datum(s, "Slant");
            assert_near(vector(&lies["origin"]), a, 1e-12 * k, "origin");
            assert_near(vector(&lies["normal"]), n, 1e-14, "normal");
            assert_near(vector(&lies["x"]), x, 1e-14, "x");
            let chip = body(s, "Chip");
            box_measures(&chip, size, "chip");
            assert_bbox(&chip, box_corners(a, x, y, n, size), 1e-9 * k, "chip");
            let centre = add(
                a,
                add(
                    mul(x, size[0] / 2.0),
                    add(mul(y, size[1] / 2.0), mul(n, size[2] / 2.0)),
                ),
            );
            assert_near(vector(&chip["centroid"]), centre, 1e-9 * k, "centre");
        };
        expect(&mut s, 6.0 * k);
        assert_sound(&mut s, "chip");
        run(
            &mut s,
            "variable.set",
            json!({"name": "reach", "expr": 9.0 * k}),
        );
        expect(&mut s, 9.0 * k);
        assert_sound(&mut s, "moved chip");
    }
}

/// Three corners of a block give the plane their coordinates give, and
/// the plane follows the block.
#[test]
fn a_plane_through_three_vertices_follows_the_body() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        run(
            &mut s,
            "variable.set",
            json!({"name": "tall", "expr": 3.0 * k}),
        );
        built(
            &mut s,
            "feature.block",
            json!({"size": [6.0 * k, 4.0 * k, "tall"], "name": "Base"}),
        );
        // Corner x + 2 y + 4 z: 1 is (6, 0, 0), 2 is (0, 4, 0), 4 is
        // (0, 0, tall).
        built(
            &mut s,
            "datum.plane",
            json!({"through": ["F1:vertex(1)", "F1:vertex(2)", "F1:vertex(4)"], "name": "Cut"}),
        );
        built(
            &mut s,
            "datum.plane",
            json!({"through": [[6.0 * k, 0, 0], [0, 4.0 * k, 0], [0, 0, "tall"]], "name": "Same"}),
        );
        assert_eq!(datum(&mut s, "Cut"), datum(&mut s, "Same"));
        let n = unit(cross([-6.0, 4.0, 0.0], [-6.0, 0.0, 3.0]));
        assert_near(vector(&datum(&mut s, "Cut")["normal"]), n, 1e-14, "normal");
        run(
            &mut s,
            "variable.set",
            json!({"name": "tall", "expr": 8.0 * k}),
        );
        assert_eq!(datum(&mut s, "Cut"), datum(&mut s, "Same"));
        let n = unit(cross([-6.0, 4.0, 0.0], [-6.0, 0.0, 8.0]));
        assert_near(vector(&datum(&mut s, "Cut")["normal"]), n, 1e-14, "moved");
        // A mix of a vertex and coordinates.
        built(
            &mut s,
            "datum.plane",
            json!({"through": ["F1:vertex(7)", [0, 0, 0], [k, 0, 0]], "name": "Mixed"}),
        );
        let lies = datum(&mut s, "Mixed");
        assert_near(
            vector(&lies["origin"]),
            [6.0 * k, 4.0 * k, 8.0 * k],
            0.0,
            "7",
        );
    }
}

// ----------------------------------------------------------------- a revolve

/// A ring turned from a rectangle on a datum plane: Pappus, and its axis
/// is the datum's own.
#[test]
fn a_revolve_of_a_sketch_on_a_datum_plane_is_a_known_ring() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        // The plane y = 5: sketch x is world z, sketch y is world x.
        built(
            &mut s,
            "datum.plane",
            json!({"from": "zx", "distance": 5.0 * k, "name": "Side"}),
        );
        run(
            &mut s,
            "sketch.create",
            json!({"name": "Section", "plane": "Side"}),
        );
        let (r1, r2, h) = (2.0 * k, 5.0 * k, 3.0 * k);
        rectangle(&mut s, "Section", (r1, r2), (k, k + h));
        built(
            &mut s,
            "feature.revolve",
            json!({"sketch": "Section", "axis": "y", "name": "Ring"}),
        );
        let ring = body(&mut s, "Ring");
        // A tube of rectangular section about the sketch's y axis.
        let volume = PI * (r2 * r2 - r1 * r1) * h;
        let area = 2.0 * PI * (r2 * r2 - r1 * r1) + 2.0 * PI * (r1 + r2) * h;
        assert_measures(&ring, volume, area, 1e-11, "ring");
        // The axis is the line through (0, 5, 0) along world x.
        assert_near(
            vector(&ring["centroid"]),
            [k + h / 2.0, 5.0 * k, 0.0],
            1e-9 * k,
            "centre",
        );
        assert_sound(&mut s, "ring");
        // A quarter of it, on a plane at an angle: a quarter of the
        // volume, by Pappus (area times the way its centre goes).
        built(
            &mut s,
            "datum.plane",
            json!({"from": "zx", "about": "x", "angle": "20 deg", "name": "Leaning"}),
        );
        run(
            &mut s,
            "sketch.create",
            json!({"name": "Leaning section", "plane": "Leaning"}),
        );
        rectangle(&mut s, "Leaning section", (r1, r2), (k, k + h));
        built(
            &mut s,
            "feature.revolve",
            json!({"sketch": "Leaning section", "axis": "y", "angle": "90 deg", "name": "Quarter"}),
        );
        let quarter = body(&mut s, "Quarter");
        let section = (r2 - r1) * h;
        let pappus = section * (PI / 2.0) * (r1 + r2) / 2.0;
        assert_close(&quarter["volume"], pappus, 1e-11, "quarter");
        assert_close(
            &quarter["area"],
            area / 4.0 + 2.0 * section,
            1e-11,
            "quarter area",
        );
        assert_sound(&mut s, "quarter");
    }
}

// ------------------------------------------------------------------ a mirror

/// A body mirrored in a datum plane lies where the reflection puts it.
#[test]
fn a_mirror_in_a_datum_plane_is_the_reflection() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        run(
            &mut s,
            "variable.set",
            json!({"name": "wall", "expr": 9.0 * k}),
        );
        let size = [4.0 * k, 2.0 * k, 3.0 * k];
        let centre = [3.0 * k, 2.0 * k, 2.5 * k];
        built(
            &mut s,
            "feature.block",
            json!({"size": size, "position": [k, k, k], "name": "Part"}),
        );
        // The plane x = wall.
        built(
            &mut s,
            "datum.plane",
            json!({"from": "yz", "distance": "wall", "name": "Wall"}),
        );
        built(
            &mut s,
            "feature.mirror",
            json!({"source": "Part", "plane": "Wall", "keep": true, "name": "Twin"}),
        );
        let reflected = |centre: V, point: V, n: V| -> V {
            sub(centre, mul(n, 2.0 * dot(sub(centre, point), n)))
        };
        let twin = body(&mut s, "Twin");
        box_measures(&twin, size, "twin");
        assert_near(
            vector(&twin["centroid"]),
            reflected(centre, [9.0 * k, 0.0, 0.0], [1.0, 0.0, 0.0]),
            1e-9 * k,
            "twin",
        );
        assert_eq!(inspect(&mut s)["bodies"].as_array().unwrap().len(), 2);
        assert_sound(&mut s, "twin");
        run(
            &mut s,
            "variable.set",
            json!({"name": "wall", "expr": -3.0 * k}),
        );
        assert_near(
            vector(&body(&mut s, "Twin")["centroid"]),
            reflected(centre, [-3.0 * k, 0.0, 0.0], [1.0, 0.0, 0.0]),
            1e-9 * k,
            "moved twin",
        );
        // In a plane at an angle: yz turned 30 deg about z, through the
        // origin, with normal (cos 30, sin 30, 0).
        built(
            &mut s,
            "datum.plane",
            json!({"from": "yz", "about": "z", "angle": "30 deg", "name": "Skew"}),
        );
        built(
            &mut s,
            "feature.mirror",
            json!({"source": "Twin", "plane": "Skew", "name": "Skewed"}),
        );
        let a = 30.0_f64.to_radians();
        let n = [a.cos(), a.sin(), 0.0];
        let twin_centre = reflected(centre, [-3.0 * k, 0.0, 0.0], [1.0, 0.0, 0.0]);
        let skewed = body(&mut s, "Skewed");
        box_measures(&skewed, size, "skewed");
        assert_near(
            vector(&skewed["centroid"]),
            reflected(twin_centre, [0.0; 3], n),
            1e-9 * k,
            "skewed",
        );
        assert_sound(&mut s, "skewed");
        // Back to a plane of its own, and to the datum again.
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Twin", "normal": [0, 1, 0], "point": [0, 5.0 * k, 0]}),
        );
        assert_eq!(feature(&mut s, "Twin")["params"]["plane"], Value::Null);
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Twin", "plane": "Wall"}),
        );
        assert_near(
            vector(&body(&mut s, "Skewed")["centroid"]),
            reflected(twin_centre, [0.0; 3], n),
            1e-9 * k,
            "back",
        );
    }
}

// ------------------------------------------------------------ upstream edits

/// Removing what a datum is placed by fails the datum with the reference
/// it lost, and what depends on the datum with the datum's name; what
/// does not depend on it is still built. Putting the reference right
/// brings everything back as it was.
#[test]
fn a_lost_reference_fails_the_datum_and_what_depends_on_it_and_nothing_else() {
    let mut s = session();
    new_document(&mut s, 1.0);
    built(
        &mut s,
        "feature.block",
        json!({"size": [20, 20, 10], "name": "Base"}),
    );
    built(
        &mut s,
        "feature.block",
        json!({"size": [5, 5, 5], "position": [40, 0, 0], "name": "Other"}),
    );
    built(
        &mut s,
        "datum.plane",
        json!({"from": "F1:face(1)", "distance": 2, "name": "Shelf"}),
    );
    built(
        &mut s,
        "datum.plane",
        json!({"from": "Shelf", "distance": 3, "name": "Top shelf"}),
    );
    run(
        &mut s,
        "sketch.create",
        json!({"name": "Round", "plane": "Top shelf"}),
    );
    circle(&mut s, "Round", [5.0, 5.0], 2.0);
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Round", "distance": 4, "name": "Post"}),
    );
    built(
        &mut s,
        "feature.mirror",
        json!({"source": "Other", "plane": "Shelf", "keep": true, "name": "Twin"}),
    );
    built(
        &mut s,
        "datum.plane",
        json!({"from": "xy", "distance": 1, "name": "Free"}),
    );
    let before = inspect(&mut s);
    run(&mut s, "feature.delete", json!({"feature": "Base"}));
    let status = |s: &mut Session, name: &str| -> (String, String) {
        let found = feature(s, name);
        (
            found["status"].as_str().unwrap().to_string(),
            found["error"].as_str().unwrap_or_default().to_string(),
        )
    };
    let (state, reason) = status(&mut s, "Shelf");
    assert_eq!(state, "failed");
    assert_eq!(
        reason,
        "from: there is no feature F1 for the face F1:face(1) (was it deleted?)"
    );
    assert_eq!(
        status(&mut s, "Top shelf"),
        (
            "failed".to_string(),
            "from: `Shelf` failed; fix it first".to_string()
        )
    );
    assert_eq!(
        status(&mut s, "Round"),
        (
            "failed".to_string(),
            "plane: `Top shelf` failed; fix it first".to_string()
        )
    );
    let (state, reason) = status(&mut s, "Post");
    assert_eq!(state, "failed");
    assert!(reason.contains("`Round` failed"), "{reason}");
    assert_eq!(
        status(&mut s, "Twin"),
        (
            "failed".to_string(),
            "plane: `Shelf` failed; fix it first".to_string()
        )
    );
    // Nothing else: the other block and the free plane are there.
    assert_eq!(status(&mut s, "Other").0, "ok");
    assert_eq!(status(&mut s, "Free").0, "ok");
    assert_eq!(inspect(&mut s)["bodies"].as_array().unwrap().len(), 1);
    let check = run(&mut s, "document.check", json!({}));
    assert_eq!(check["ok"], false);
    // Undo: everything is back as it was.
    run(&mut s, "edit.undo", json!({}));
    let after = inspect(&mut s);
    assert_eq!(after["bodies"], before["bodies"]);
    assert_eq!(after["features"], before["features"]);
    // Repaired by pointing the datum at something else: what depends on
    // it is built again.
    run(&mut s, "feature.delete", json!({"feature": "Base"}));
    built(
        &mut s,
        "datum.plane",
        json!({"replace": "Shelf", "from": "xy", "distance": 12}),
    );
    let repaired = inspect(&mut s);
    assert_eq!(repaired["ok"], true, "{repaired}");
    let post = |document: &Value| {
        document["bodies"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["name"] == "Post")
            .unwrap()
            .clone()
    };
    // The shelf is where it was (12 up), so the post is the same solid.
    // (Not its bbox: that of a round body comes from a mesh whose
    // fineness follows the size of the whole model, which lost a block.)
    for key in ["volume", "area", "centroid", "faceNames"] {
        assert_eq!(post(&repaired)[key], post(&before)[key], "{key}");
    }
    assert_sound(&mut s, "repaired");
}

/// A suppressed or rolled back datum says so to what depends on it.
#[test]
fn a_datum_that_is_switched_off_fails_what_lies_on_it() {
    let mut s = session();
    new_document(&mut s, 1.0);
    built(
        &mut s,
        "datum.plane",
        json!({"from": "xy", "distance": 2, "name": "Shelf"}),
    );
    box_on(&mut s, "Shelf", "Box", [1.0, 2.0, 3.0]);
    run(
        &mut s,
        "feature.suppress",
        json!({"feature": "Shelf", "suppressed": true}),
    );
    let sketch = feature(&mut s, "Box profile");
    assert_eq!(sketch["error"], "plane: `Shelf` is suppressed");
    run(
        &mut s,
        "feature.suppress",
        json!({"feature": "Shelf", "suppressed": false}),
    );
    assert_eq!(inspect(&mut s)["ok"], true);
    // Moved after the sketch that lies on it: the sketch says so.
    run(
        &mut s,
        "feature.move",
        json!({"feature": "Shelf", "position": 2}),
    );
    let sketch = feature(&mut s, "Box profile");
    assert_eq!(
        sketch["error"],
        "plane: `Shelf` comes after this feature in the history; move it before (feature.move)"
    );
    run(
        &mut s,
        "feature.move",
        json!({"feature": "Shelf", "position": 0}),
    );
    assert_eq!(inspect(&mut s)["ok"], true);
}

// --------------------------------------------------------------------- files

/// Every kind of datum plane, a sketch on one and a mirror in one, saved
/// and reopened: the same bodies and the same datums.
#[test]
fn datum_planes_are_saved_and_reopened_as_the_same_bodies() {
    let dir = workdir("files");
    let mut s = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    new_document(&mut s, 1.0);
    run(
        &mut s,
        "variable.set",
        json!({"name": "lift", "expr": "7 mm"}),
    );
    built(
        &mut s,
        "feature.block",
        json!({"size": [20, 20, 10], "name": "Base"}),
    );
    built(
        &mut s,
        "datum.plane",
        json!({"from": "F1:face(1)", "distance": "lift", "name": "Shelf"}),
    );
    built(
        &mut s,
        "datum.plane",
        json!({"from": "Shelf", "about": ["F1:face(1)", "F1:face(5)"], "angle": "25 deg",
               "name": "Lid"}),
    );
    built(
        &mut s,
        "datum.plane",
        json!({"through": ["F1:vertex(7)", [0, 0, "lift"], [30, 0, 0]], "name": "Slant"}),
    );
    built(
        &mut s,
        "datum.plane",
        json!({"from": "zx", "about": "z", "angle": "0.3 rad", "name": "Skew"}),
    );
    box_on(&mut s, "Lid", "Box", [3.0, 4.0, 5.0]);
    box_on(&mut s, "Slant", "Chip", [2.0, 2.0, 1.0]);
    built(
        &mut s,
        "feature.mirror",
        json!({"source": "Chip", "plane": "Skew", "keep": true, "name": "Twin"}),
    );
    assert_sound(&mut s, "all planes");
    let before = inspect(&mut s);
    run(&mut s, "file.save", json!({"path": "planes.3dcraft"}));
    run(&mut s, "file.close", json!({}));
    run(&mut s, "file.open", json!({"path": "planes.3dcraft"}));
    let after = inspect(&mut s);
    assert_eq!(after["bodies"], before["bodies"]);
    assert_eq!(after["features"], before["features"]);
    assert_eq!(before["bodies"].as_array().unwrap().len(), 4);
    assert_eq!(before["ok"], true);
    let text =
        String::from_utf8_lossy(&std::fs::read(dir.join("planes.3dcraft")).unwrap()).to_string();
    for expected in [
        "\"datumPlane\"",
        "\"offset\"",
        "\"angle\"",
        "\"threePoints\"",
        "\"between\"",
    ] {
        assert!(text.contains(expected), "{expected} is not in the file");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------- degenerate

/// What gives no plane is a failed datum with the reason, never a panic
/// and never a plane somewhere else.
#[test]
fn what_gives_no_plane_fails_with_the_reason() {
    let mut s = session();
    new_document(&mut s, 1.0);
    built(
        &mut s,
        "feature.block",
        json!({"size": [20, 20, 10], "name": "Base"}),
    );
    built(
        &mut s,
        "feature.cylinder",
        json!({"radius": 3, "height": 5, "position": [50, 0, 0], "name": "Rod"}),
    );
    let mut count = 0;
    let mut fails = |s: &mut Session, params: Value, expected: &str| {
        count += 1;
        let mut params = params;
        params["name"] = json!(format!("Bad {count}"));
        let reason = failed(s, "datum.plane", params.clone());
        assert!(reason.contains(expected), "{params}: {reason}");
    };
    // Points.
    fails(
        &mut s,
        json!({"through": [[0, 0, 0], [1, 1, 1], [3, 3, 3]]}),
        "the three points are on one line",
    );
    fails(
        &mut s,
        json!({"through": [[0, 0, 0], [1, 1, 1], [1, 1, 1]]}),
        "points 1 and 2 are the same point",
    );
    fails(
        &mut s,
        json!({"through": [[0, 0, 0], [0, 0, 1e-9], [1, 0, 0]]}),
        "points 0 and 1 are the same point",
    );
    fails(
        &mut s,
        json!({"through": [[0, 0, 0], ["1 / 0", 0, 0], [0, 1, 0]]}),
        "through.1.x",
    );
    fails(
        &mut s,
        json!({"through": [[0, 0, 0], ["30 deg", 0, 0], [0, 1, 0]]}),
        "but a length is needed",
    );
    fails(
        &mut s,
        json!({"through": [[0, 0, 0], ["1e200 * 1e200", 0, 0], [0, 1, 0]]}),
        "through.1",
    );
    // Distances and angles that are no number.
    fails(
        &mut s,
        json!({"from": "xy", "distance": "1 / 0"}),
        "distance",
    );
    fails(&mut s, json!({"from": "xy", "distance": "nope"}), "unknown");
    fails(
        &mut s,
        json!({"from": "xy", "distance": "30 deg"}),
        "but a length is needed",
    );
    fails(
        &mut s,
        json!({"from": "xy", "distance": "1e200 * 1e200"}),
        "distance",
    );
    fails(
        &mut s,
        json!({"from": "xy", "about": "x", "angle": "0 / 0"}),
        "angle",
    );
    fails(
        &mut s,
        json!({"from": "xy", "about": "x", "angle": "5 mm"}),
        "but an angle is needed",
    );
    // An axis a plane cannot be turned about.
    fails(
        &mut s,
        json!({"from": "xy", "about": "z", "angle": "10 deg"}),
        "the axis is not parallel to the plane (it makes 90 deg with it)",
    );
    // Names that point at nothing.
    fails(
        &mut s,
        json!({"from": "F1:face(9)", "distance": 1}),
        "`Base` no longer has a face F1:face(9); its faces are: F1:face(0)",
    );
    fails(
        &mut s,
        json!({"from": "F77:face(0)", "distance": 1}),
        "there is no feature F77",
    );
    fails(
        &mut s,
        json!({"from": "F2:face(0)", "distance": 1}),
        "is a cylinder face; a sketch needs a flat one",
    );
    fails(
        &mut s,
        json!({"through": ["F1:vertex(8)", [0, 0, 0], [1, 0, 0]]}),
        "`Base` no longer has a vertex F1:vertex(8); its vertices are: F1:vertex(0)",
    );
    fails(
        &mut s,
        json!({"through": ["F1:face(1)", [0, 0, 0], [1, 0, 0]]}),
        "F1:face(1) is not a vertex",
    );
    // Edges that are not there, or not straight, or not one.
    fails(
        &mut s,
        json!({"from": "xy", "about": ["F1:face(0)", "F1:face(1)"], "angle": "10 deg"}),
        "the faces F1:face(0) and F1:face(1) of `Base` do not meet in an edge",
    );
    fails(
        &mut s,
        json!({"from": "xy", "about": ["F1:face(0)", "F2:face(1)"], "angle": "10 deg"}),
        "are faces of two features (F1 and F2)",
    );
    fails(
        &mut s,
        json!({"from": "xy", "about": ["F2:face(0)", "F2:face(2)"], "angle": "10 deg"}),
        "is a circle, not a straight line",
    );
    fails(
        &mut s,
        json!({"from": "xy", "about": ["F1:face(1)", "F1:face(11)"], "angle": "10 deg"}),
        "`Base` no longer has a face F1:face(11)",
    );
    // An edge that is not parallel to the plane: an upright one.
    fails(
        &mut s,
        json!({"from": "xy", "about": ["F1:face(2)", "F1:face(5)"], "angle": "10 deg"}),
        "the axis is not parallel to the plane",
    );
    // A datum placed by itself, by a later one, and two by each other.
    built(
        &mut s,
        "datum.plane",
        json!({"from": "xy", "distance": 1, "name": "First"}),
    );
    built(
        &mut s,
        "datum.plane",
        json!({"from": "First", "distance": 1, "name": "Second"}),
    );
    let reason = failed(
        &mut s,
        "datum.plane",
        json!({"replace": "First", "from": "First", "distance": 1}),
    );
    assert_eq!(
        reason,
        "from: `First` comes after this feature in the history; move it before (feature.move)"
    );
    let reason = failed(
        &mut s,
        "datum.plane",
        json!({"replace": "First", "from": "Second", "distance": 1}),
    );
    assert!(
        reason.contains("`Second` comes after this feature"),
        "{reason}"
    );
    assert_eq!(
        feature(&mut s, "Second")["error"],
        "from: `First` failed; fix it first"
    );
    // The bodies are untouched by all of it.
    assert_eq!(inspect(&mut s)["bodies"].as_array().unwrap().len(), 2);
    assert_sound_bodies(&mut s);
}

/// The bodies that exist are valid, whatever failed beside them.
fn assert_sound_bodies(session: &mut Session) {
    let check = run(session, "document.check", json!({"deep": true}));
    for problem in check["problems"].as_array().unwrap() {
        assert!(
            problem.get("defect").is_none() && problem["problem"].is_string(),
            "{problem}"
        );
    }
}

/// Parameters that make no sense are refused before anything is stored.
#[test]
fn bad_parameters_are_refused() {
    let mut s = session();
    assert!(
        refused(&mut s, "datum.plane", json!({"from": "xy", "distance": 1})).contains("document")
    );
    new_document(&mut s, 1.0);
    built(
        &mut s,
        "feature.block",
        json!({"size": [20, 20, 10], "name": "Base"}),
    );
    built(
        &mut s,
        "datum.plane",
        json!({"from": "xy", "distance": 1, "name": "Shelf"}),
    );
    let before = inspect(&mut s);
    for (params, expected) in [
        (json!({}), "missing parameter `from`"),
        (json!({"from": "xy"}), "missing parameter `distance`"),
        (
            json!({"from": "xy", "angle": 3}),
            "missing parameter `about`",
        ),
        (
            json!({"from": "xy", "about": "x"}),
            "missing parameter `angle`",
        ),
        (
            json!({"from": "xy", "distance": 1, "about": "x", "angle": 3}),
            "not both",
        ),
        (
            json!({"from": "xy", "distance": 1, "through": [[0, 0, 0], [1, 0, 0], [0, 1, 0]]}),
            "not both",
        ),
        (
            json!({"through": [[0, 0, 0], [1, 0, 0], [0, 1, 0]], "distance": 1}),
            "`distance` goes with `from`",
        ),
        (
            json!({"from": "nowhere", "distance": 1}),
            "there is no plane `nowhere`",
        ),
        (
            json!({"from": "Base", "distance": 1}),
            "`Base` is a block, not a datumPlane",
        ),
        (
            json!({"from": 1, "distance": 1}),
            "`Base` is a block, not a datumPlane",
        ),
        (
            json!({"from": 99, "distance": 1}),
            "there is no feature F99",
        ),
        (
            json!({"from": [1, 2], "distance": 1}),
            "`from` must be a plane",
        ),
        (json!({"from": "F1:side(", "distance": 1}), "is not a name"),
        (
            json!({"from": "F1:vertex(1)", "distance": 1}),
            "is not a face",
        ),
        (
            json!({"from": "xy", "distance": [1]}),
            "`distance` must be an expression",
        ),
        (
            json!({"from": "xy", "about": "w", "angle": 1}),
            "there is no axis `w`",
        ),
        (
            json!({"from": "xy", "about": 7, "angle": 1}),
            "`about` must be an axis",
        ),
        (
            json!({"from": "xy", "about": ["F1:face(1)"], "angle": 1}),
            "two face names",
        ),
        (
            json!({"from": "xy", "about": ["F1:face(1)", "F1:edge(1)"], "angle": 1}),
            "is not a face",
        ),
        (json!({"through": [[0, 0, 0], [1, 0, 0]]}), "three points"),
        (json!({"through": "abc"}), "three points"),
        (
            json!({"through": [[0, 0], [1, 0, 0], [0, 1, 0]]}),
            "three coordinates",
        ),
        (
            json!({"through": [7, [1, 0, 0], [0, 1, 0]]}),
            "a point is [x, y, z]",
        ),
        (
            json!({"through": ["zzz", [1, 0, 0], [0, 1, 0]]}),
            "is not a name",
        ),
        (
            json!({"through": [[0, 0, {}], [1, 0, 0], [0, 1, 0]]}),
            "must be an expression",
        ),
        (
            json!({"from": "xy", "distance": 1, "replace": "Base"}),
            "`Base` is a block, not a datumPlane",
        ),
        (
            json!({"from": "xy", "distance": 1, "replace": "Nope"}),
            "there is no feature named `Nope`",
        ),
        (
            json!({"from": "xy", "distance": 1, "replace": "Shelf", "name": "New"}),
            "`name` names a new datum",
        ),
        (
            json!({"from": "xy", "distance": 1, "name": "Shelf"}),
            "already exists",
        ),
        (
            json!({"from": "xy", "distance": 1, "extra": 1}),
            "unknown parameter `extra`",
        ),
    ] {
        let reason = refused(&mut s, "datum.plane", params.clone());
        assert!(reason.contains(expected), "{params}: {reason}");
    }
    // What takes a datum plane refuses what is not one.
    for (command, params, expected) in [
        (
            "sketch.create",
            json!({"plane": "Base"}),
            "`Base` is a block, not a datum plane",
        ),
        (
            "sketch.create",
            json!({"plane": "Nope"}),
            "unknown plane `Nope`",
        ),
        (
            "feature.mirror",
            json!({"source": "Base", "plane": "Base"}),
            "`Base` is a block, not a datumPlane",
        ),
        (
            "feature.mirror",
            json!({"source": "Base", "plane": "Shelf", "normal": [0, 0, 1]}),
            "not both",
        ),
        (
            "feature.mirror",
            json!({"source": "Base", "plane": "Shelf", "point": [0, 0, 1]}),
            "`point` goes with `normal`",
        ),
        (
            "feature.mirror",
            json!({"source": "Base"}),
            "missing parameter `normal`",
        ),
        (
            "feature.edit",
            json!({"feature": "Shelf", "angle": "3 deg"}),
            "`angle` is not a parameter of a datum plane",
        ),
        (
            "feature.edit",
            json!({"feature": "Base", "plane": "Shelf"}),
            "`plane` is not a parameter of a block",
        ),
    ] {
        let reason = refused(&mut s, command, params.clone());
        assert!(reason.contains(expected), "{command} {params}: {reason}");
    }
    assert_eq!(inspect(&mut s), before, "a refused command changes nothing");
    // A sketch is moved onto a datum plane and back.
    run(&mut s, "sketch.create", json!({"name": "Flat"}));
    run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "Flat", "plane": "Shelf", "offset": 2}),
    );
    let flat = run(&mut s, "sketch.inspect", json!({"sketch": "Flat"}));
    assert_eq!(
        flat["plane"],
        json!({"type": "datum", "plane": 2, "offset": "2"})
    );
    run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "Flat", "plane": "F2"}),
    );
    run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "Flat", "plane": "zx"}),
    );
    let flat = run(&mut s, "sketch.inspect", json!({"sketch": "Flat"}));
    assert_eq!(flat["plane"]["type"], "world");
}
