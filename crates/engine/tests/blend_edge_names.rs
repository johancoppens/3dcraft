//! A fillet or a chamfer takes one edge by its own name, beside the pair
//! of faces it lies between, which means every edge between them
//! (docs/design/names.md › *An edge by its own name*). Two faces can meet
//! in several edges: a pipe sunk into a plate leaves its top along two
//! lines, two merged discs meet in two corners, a loft's end circle is
//! four arcs. The name picks one; the pair still takes all.
//!
//! The oracles are closed forms worked out by hand (the plate and the
//! pipe in the plane across the pipe: the ball's centre, its feet, a
//! polygon less a sector and a circular segment; the two discs: the
//! triangle of the three centres less three sectors) and, on the loft,
//! the fillet by the pair: the same edges by their names build the same
//! body. Each at the scales 1e-3, 1 and 1e6.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;

use common::checked::{
    assert_solid, body, built, error, failed, failure, feature, new_document, run, session,
};
use craft3d_engine::{Access, Session, Workspace};
use serde_json::{Value, json};
use std::f64::consts::PI;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn at(k: f64, p: [f64; 3]) -> Value {
    json!([p[0] * k, p[1] * k, p[2] * k])
}

/// The relative difference between two volumes that should be equal.
#[track_caller]
fn assert_rel(actual: f64, expected: f64, rel: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= rel * expected.abs(),
        "{what}: {actual}, expected {expected} (off by {:e})",
        (actual - expected) / expected
    );
}

fn volume(session: &mut Session, name: &str) -> f64 {
    body(session, name)["volume"].as_f64().unwrap()
}

// ---------------------------------------------------------------------
// The plate and the pipe

/// The top of the plate and the wall of the pipe, as the union names
/// them: the pair both edges lie between.
const TOP: &str = "F3:copy(0)/F1:face(1)";
const WALL: &str = "F3:copy(1)/F2:face(0)";

/// The area of the triangle or polygon through `points`, by the shoelace.
fn polygon(points: &[(f64, f64)]) -> f64 {
    let n = points.len();
    let twice: f64 = (0..n)
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % n]);
            a.0 * b.1 - b.0 * a.1
        })
        .sum();
    0.5 * twice.abs()
}

/// The angle at `o` between `p` and `q`.
fn angle(o: (f64, f64), p: (f64, f64), q: (f64, f64)) -> f64 {
    let (u, v) = ((p.0 - o.0, p.1 - o.1), (q.0 - o.0, q.1 - o.1));
    (u.0 * v.1 - u.1 * v.0).abs().patan2(u.0 * v.0 + u.1 * v.1)
}

/// The plate 60 x 40 x 10 (z 0..10) and the pipe of radius 10 along x
/// (0..60) whose axis is at (y, z) = (20, `zc`), united: in the section
/// across x (y, z) the pipe dips `10 - (zc - 10)` into the plate and
/// leaves its top in the corners at y = 20 -+ half. Its volume, and the
/// volume once the corner at y = 20 + half is rounded with a ball of
/// radius `rho` (by symmetry, the other corner alike): the ball touches
/// the top (centre at z = 10 + rho) and the pipe from outside (centre
/// r + rho from the axis); what it adds is the polygon of the corner, its
/// two feet and its centre, less the sector of the ball between its feet
/// and the segment of the pipe between the corner and its foot there,
/// along the length 60.
fn plate_and_pipe(zc: f64, rho: f64) -> (f64, f64) {
    let r = 10.0;
    let t = zc - 10.0;
    let half = (r * r - t * t).sqrt();
    let dip = r * r * (t / r).pacos() - t * half;
    let v0 = 24000.0 + 60.0 * (PI * r * r - dip);
    let p = (20.0, zc);
    let e = (20.0 + half, 10.0);
    let lift = zc - 10.0 - rho;
    let c = (
        20.0 + ((r + rho) * (r + rho) - lift * lift).sqrt(),
        10.0 + rho,
    );
    let f1 = (c.0, 10.0);
    let f2 = (
        p.0 + r * (c.0 - p.0) / (r + rho),
        p.1 + r * (c.1 - p.1) / (r + rho),
    );
    let sector = 0.5 * rho * rho * angle(c, f1, f2);
    let phi = angle(p, e, f2);
    let segment = 0.5 * r * r * (phi - phi.psin());
    let added = polygon(&[e, f1, c, f2]) - sector - segment;
    (v0, v0 + 60.0 * added)
}

/// The same corner chamfered as 3DCraft measures a chamfer: `a` along the
/// plate from the edge and `b` along the pipe, each at right angles to
/// the edge (an arc of length `b`): the triangle of the corner and the
/// two cut lines, less the segment of the pipe the chord cuts off.
fn plate_and_pipe_chamfered(a: f64, b: f64) -> f64 {
    let (r, zc) = (10.0, 16.0);
    let (v0, _) = plate_and_pipe(zc, 1.0);
    let t = zc - 10.0;
    let half = (r * r - t * t).sqrt();
    let e = (20.0 + half, 10.0);
    let g1 = (e.0 + a, 10.0);
    let turn = (10.0 - zc).patan2(half) + b / r;
    let g2 = (20.0 + r * turn.pcos(), zc + r * turn.psin());
    let phi = b / r;
    let segment = 0.5 * r * r * (phi - phi.psin());
    v0 + 60.0 * (polygon(&[e, g1, g2]) - segment)
}

/// The plate `Plate` (F1), the pipe `Pipe` (F2) with its axis at height
/// `zc`, and their union `Joined` (F3), at scale `k`.
fn plate_with_pipe(session: &mut Session, k: f64, zc: f64) {
    new_document(session, k);
    run(
        session,
        "variable.set",
        json!({"name": "zc", "expr": format!("{} mm", zc * k)}),
    );
    built(
        session,
        "feature.block",
        json!({"size": at(k, [60.0, 40.0, 10.0]), "position": [0, 0, 0], "name": "Plate"}),
    );
    built(
        session,
        "feature.cylinder",
        json!({"radius": 10.0 * k, "height": 60.0 * k,
               "position": [0, 20.0 * k, "zc"], "axis": [1, 0, 0], "name": "Pipe"}),
    );
    built(
        session,
        "feature.boolean",
        json!({"op": "union", "target": "Plate", "tool": "Pipe", "name": "Joined"}),
    );
}

/// The two straight edges of `feature`'s body between the top and the
/// pipe's wall, by their own names: the one at the smaller y first.
fn pipe_edges(session: &mut Session, feature: &str) -> [String; 2] {
    let listed = run(session, "document.edges", json!({"feature": feature}));
    let mut found: Vec<(f64, String)> = listed["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["curve"] == "line")
        .filter(|e| {
            let faces = &e["faces"];
            faces.as_array().unwrap().len() == 2
                && faces
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|f| f.as_str().unwrap().ends_with("F1:face(1)"))
                && faces
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|f| f.as_str().unwrap().ends_with("F2:face(0)"))
        })
        .map(|e| {
            (
                e["start"][1].as_f64().unwrap(),
                e["name"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    let names: Vec<String> = found.into_iter().map(|(_, name)| name).collect();
    names.try_into().unwrap_or_else(|n: Vec<String>| {
        panic!(
            "{feature}: {} edges between the top and the pipe: {n:?}",
            n.len()
        )
    })
}

/// Where the material of the body `name` lies across the pipe: the y of
/// its centroid.
fn centroid_y(session: &mut Session, name: &str) -> f64 {
    body(session, name)["centroid"][1].as_f64().unwrap()
}

#[test]
fn one_of_two_edges_between_a_plate_and_a_pipe_is_rounded_by_its_name() {
    let (v0, rounded) = plate_and_pipe(16.0, 2.0);
    // Re-derived by hand (the BRIEF's numbers): V0 = 40165.7846, and the
    // round adds 60 x 2.250358.
    assert_rel(v0, 40165.7846135291, 1e-12, "the union");
    assert_rel(rounded, 40300.8060669181, 1e-12, "the round");
    for k in SCALES {
        let mut s = session();
        plate_with_pipe(&mut s, k, 16.0);
        let [low, high] = pipe_edges(&mut s, "Joined");
        assert_ne!(low, high);
        // The pair cannot tell them apart: both lie between the top and
        // the wall.
        for name in [&low, &high] {
            let between = run(
                &mut s,
                "document.edges",
                json!({"feature": "Joined", "between": [TOP, WALL]}),
            );
            assert!(
                between["edges"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|e| e["name"] == **name),
                "{between}"
            );
        }
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": [high], "radius": 2.0 * k, "name": "Round"}),
        );
        assert_solid(&mut s, "Round", rounded, None, k);
        // Only the side at y = 28 grew: the union is symmetric about y = 20.
        assert!(centroid_y(&mut s, "Round") > 20.0 * k * (1.0 + 1e-9));
        let names = common::names_of(&body(&mut s, "Round"));
        assert!(names.contains(&"F4:fillet(0)".to_string()), "{names:?}");
        // The other edge by its name: the same volume, the other side.
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Round", "edges": [low]}),
        );
        assert_solid(&mut s, "Round", rounded, None, k);
        assert!(centroid_y(&mut s, "Round") < 20.0 * k * (1.0 - 1e-9));
        // The pair means both edges, as it always did.
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Round", "edges": [[TOP, WALL]]}),
        );
        assert_solid(&mut s, "Round", v0 + 2.0 * (rounded - v0), None, k);
        let names = common::names_of(&body(&mut s, "Round"));
        assert!(names.contains(&"F4:fillet(0,1)".to_string()), "{names:?}");
        // An edge given twice, by its name and in a pair, fails; the
        // parameters show what was given.
        run(
            &mut s,
            "feature.edit",
            json!({"feature": "Round", "edges": [high, [TOP, WALL]]}),
        );
        let error = failure(&mut s, "Round");
        assert!(
            error.contains("edges.1") && error.contains("is the edge of edges.0 too"),
            "{error}"
        );
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Round", "edges": [high]}),
        );
        assert_eq!(feature(&mut s, "Round")["params"]["edges"], json!([high]));
    }
}

#[test]
fn one_edge_chamfered_by_its_name_measures_the_first_size_on_the_face_given() {
    // By hand: 2 along the plate and 2 of arc along the pipe adds
    // 60 x 1.642459; 2 along the plate and 1 along the pipe, and the other
    // way round, less.
    assert_rel(
        plate_and_pipe_chamfered(2.0, 2.0),
        40264.32991667306,
        1e-12,
        "2, 2",
    );
    for k in SCALES {
        let mut s = session();
        plate_with_pipe(&mut s, k, 16.0);
        let [_, high] = pipe_edges(&mut s, "Joined");
        built(
            &mut s,
            "feature.chamfer",
            json!({"edges": [high], "distance": 2.0 * k, "name": "Bevel"}),
        );
        assert_solid(&mut s, "Bevel", plate_and_pipe_chamfered(2.0, 2.0), None, k);
        // Two sizes: the side has to be said.
        run(
            &mut s,
            "feature.edit",
            json!({"feature": "Bevel", "second": 1.0 * k}),
        );
        let error = failure(&mut s, "Bevel");
        assert!(
            error.contains("edges.0")
                && error.contains("give the edge with that face")
                && error.contains(TOP),
            "{error}"
        );
        for (first, a, b) in [(TOP, 2.0, 1.0), (WALL, 1.0, 2.0)] {
            built(
                &mut s,
                "feature.edit",
                json!({"feature": "Bevel", "edges": [[high, first]]}),
            );
            assert_solid(&mut s, "Bevel", plate_and_pipe_chamfered(a, b), None, k);
        }
        // A face that is not one of the edge's.
        run(
            &mut s,
            "feature.edit",
            json!({"feature": "Bevel", "edges": [[high, "F3:copy(0)/F1:face(0)"]]}),
        );
        let error = failure(&mut s, "Bevel");
        assert!(error.contains("is not a face of the edge"), "{error}");
    }
}

#[test]
fn an_edge_name_is_followed_through_the_features_that_take_its_body() {
    let (_, rounded) = plate_and_pipe(16.0, 2.0);
    for k in SCALES {
        // A chamfer of the plate's bottom front edge (1 x 1 along 60: 30
        // less) between the union and the round, which names the edge as
        // the union has it: followed to the chamfer's body by the edge the
        // chamfer kept of it.
        let mut s = session();
        plate_with_pipe(&mut s, k, 16.0);
        let [_, high] = pipe_edges(&mut s, "Joined");
        assert!(high.starts_with("F3:between["), "{high}");
        built(
            &mut s,
            "feature.chamfer",
            json!({"edges": [["F3:copy(0)/F1:face(0)", "F3:copy(0)/F1:face(2)"]],
                   "distance": 1.0 * k, "name": "Bottom"}),
        );
        let [_, there] = pipe_edges(&mut s, "Bottom");
        assert!(there.starts_with("F4:between["), "{there}");
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": [high], "radius": 2.0 * k, "name": "Round"}),
        );
        assert_solid(&mut s, "Round", rounded - 30.0, None, k);
        let state = feature(&mut s, "Round");
        assert_eq!(state["params"]["edges"], json!([high]), "stored as given");
        assert_eq!(
            state["followed"]["edges"],
            json!([there]),
            "built as followed"
        );

        // The same chamfer put in at the rollback marker before a round
        // made by the union's name: the round follows it.
        let mut s = session();
        plate_with_pipe(&mut s, k, 16.0);
        let [_, high] = pipe_edges(&mut s, "Joined");
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": [high], "radius": 2.0 * k, "name": "Round"}),
        );
        assert_solid(&mut s, "Round", rounded, None, k);
        run(&mut s, "history.rollback", json!({"feature": "Joined"}));
        built(
            &mut s,
            "feature.chamfer",
            json!({"edges": [["F3:copy(0)/F1:face(0)", "F3:copy(0)/F1:face(2)"]],
                   "distance": 1.0 * k, "name": "Bottom"}),
        );
        run(&mut s, "history.rollback", json!({"feature": null}));
        assert_solid(&mut s, "Round", rounded - 30.0, None, k);
        run(&mut s, "feature.delete", json!({"feature": "Bottom"}));
        assert_solid(&mut s, "Round", rounded, None, k);
    }
}

#[test]
fn an_edge_name_holds_through_an_edit_upstream_and_fails_when_its_edge_changes() {
    let (_, deeper) = plate_and_pipe(15.0, 2.0);
    let (_, rounded) = plate_and_pipe(16.0, 2.0);
    for k in SCALES {
        let mut s = session();
        plate_with_pipe(&mut s, k, 16.0);
        let [_, high] = pipe_edges(&mut s, "Joined");
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": [high], "radius": 2.0 * k, "name": "Round"}),
        );
        // The pipe sunk 1 deeper: the edges move to y = 20 -+ 8.66, the
        // name stays, the round with it.
        run(
            &mut s,
            "variable.set",
            json!({"name": "zc", "expr": format!("{} mm", 15.0 * k)}),
        );
        assert_solid(&mut s, "Round", deeper, None, k);
        assert!(centroid_y(&mut s, "Round") > 20.0 * k * (1.0 + 1e-9));
        assert_eq!(feature(&mut s, "Round")["params"]["edges"], json!([high]));
        // The pipe starting 5 in: the edge now ends on the pipe's cap,
        // so another edge with other faces near it is there, and the name
        // fails, saying so, rather than landing on an edge it did not name.
        run(
            &mut s,
            "variable.set",
            json!({"name": "zc", "expr": format!("{} mm", 16.0 * k)}),
        );
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Pipe", "position": [5.0 * k, 20.0 * k, "zc"], "height": 55.0 * k}),
        );
        let error = failure(&mut s, "Round");
        assert!(
            error.contains("edges.0") && error.contains("has no edge") && error.contains(&high),
            "{error}"
        );
        // Put back, it holds again.
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Pipe", "position": [0, 20.0 * k, "zc"], "height": 60.0 * k}),
        );
        assert_solid(&mut s, "Round", rounded, None, k);
    }
}

#[test]
fn an_edge_cut_in_two_by_a_later_feature_is_not_one_edge_any_more() {
    let mut s = session();
    plate_with_pipe(&mut s, 1.0, 16.0);
    let [_, high] = pipe_edges(&mut s, "Joined");
    // A notch across the edge at y = 28 cuts it in two.
    built(
        &mut s,
        "feature.block",
        json!({"size": [4, 8, 4], "position": [28, 24, 8], "name": "Notch"}),
    );
    built(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Joined", "tool": "Notch", "name": "Notched"}),
    );
    let error = failed(
        &mut s,
        "feature.fillet",
        json!({"edges": [high], "radius": 1, "name": "Round"}),
    );
    assert!(
        error.contains("edges.0")
            && error.contains("is more than one edge after `Notched`")
            && error.contains("give one of these names"),
        "{error}"
    );
}

#[test]
fn wrong_edges_are_refused_with_the_reason() {
    let mut s = session();
    plate_with_pipe(&mut s, 1.0, 16.0);
    let [low, high] = pipe_edges(&mut s, "Joined");
    for (edges, says) in [
        (json!([TOP]), "is not an edge"),
        (json!([[high, low]]), "is not a face"),
        (json!([high, high]), "is given twice"),
        (json!([[TOP, WALL], [WALL, TOP]]), "is given twice"),
        (json!([[high]]), "holds a list of 1 names"),
        (json!([[TOP, WALL, high]]), "holds a list of 3 names"),
        (json!([1]), "must be a list of edges"),
        (json!([{"edge": high}]), "must be a list of edges"),
        (json!([[1, 2]]), "must be"),
        (json!(["not a name"]), "edges"),
        (json!(high), "must be a list of edges"),
    ] {
        let message = error(
            &mut s,
            "feature.fillet",
            json!({"edges": edges, "radius": 1}),
        );
        assert!(message.contains(says), "{edges}: {message}");
    }
    // Names of no edge the body has, or of no feature: the feature fails
    // with the name; of another body: refused at once.
    for (edges, says) in [
        (
            json!(["F3:between[F3:copy(0)/F1:face(0),F3:copy(0)/F1:face(1)]"]),
            "has no edge",
        ),
        (json!(["F9:edge(0)"]), "F9"),
    ] {
        let result = run(
            &mut s,
            "feature.fillet",
            json!({"edges": edges, "radius": 1, "name": "Wrong"}),
        );
        assert_eq!(result["status"], "failed", "{edges}: {result}");
        let reason = result["error"].as_str().unwrap();
        assert!(reason.contains(says), "{edges}: {reason}");
        run(&mut s, "feature.delete", json!({"feature": "Wrong"}));
    }
    built(
        &mut s,
        "feature.block",
        json!({"size": [1, 1, 1], "position": [100, 0, 0], "name": "Other"}),
    );
    let message = error(
        &mut s,
        "feature.fillet",
        json!({"edges": [high, "Other:edge(0)"], "radius": 1}),
    );
    assert!(message.contains("which are not one body"), "{message}");
}

#[test]
fn a_document_with_edges_by_name_is_saved_and_reopened_to_the_same_body() {
    let dir = common::workdir("edge-names", "saved");
    let mut s = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    plate_with_pipe(&mut s, 1.0, 16.0);
    let [low, high] = pipe_edges(&mut s, "Joined");
    built(
        &mut s,
        "feature.chamfer",
        json!({"edges": [[low, WALL]], "distance": 2, "second": 1, "name": "Bevel"}),
    );
    built(
        &mut s,
        "feature.fillet",
        json!({"edges": [high], "radius": 2, "name": "Round"}),
    );
    let before = body(&mut s, "Round");
    let path = dir.join("pipe.3dcraft");
    run(
        &mut s,
        "file.save",
        json!({"path": path.to_str().unwrap(), "overwrite": true}),
    );
    run(&mut s, "file.open", json!({"path": path.to_str().unwrap()}));
    assert_eq!(body(&mut s, "Round"), before);
    assert_eq!(feature(&mut s, "Round")["params"]["edges"], json!([high]));
    assert_eq!(
        feature(&mut s, "Bevel")["params"]["edges"],
        json!([[low, WALL]])
    );
}

// ---------------------------------------------------------------------
// Two discs merged: an extrusion's own edge names

/// The outline of two discs merged (radius 17.1406 about the origin and
/// 9.6669 about (10.7158, 13.3781), which lies on the first circle), as
/// two arcs closing at A and B, extruded 10, at scale `k`.
fn two_discs(session: &mut Session, k: f64) {
    new_document(session, k);
    let p = |x: f64, y: f64| json!([x * k, y * k]);
    run(
        session,
        "sketch.create",
        json!({"plane": "xy", "name": "Outline"}),
    );
    run(
        session,
        "sketch.path",
        json!({"sketch": "Outline", "start": p(1.7728134670876865, 17.048669520256194),
               "segments": [
                   {"arc": p(16.250324167975673, 5.452243981674495), "center": p(0.0, 0.0),
                    "clockwise": false},
                   {"arc": "start", "center": p(10.715766135516247, 13.378055020402751),
                    "clockwise": false}]}),
    );
    built(
        session,
        "feature.extrude",
        json!({"sketch": "Outline", "distance": 10.0 * k, "name": "Discs"}),
    );
}

/// The union of the two discs and what an R6 round in one of the two
/// concave corners adds, by hand: the union's area is the two discs less
/// their lens; the round's centre Q is R1 + 6 from the origin and R2 + 6
/// from the second centre, and what it fills is the triangle of the
/// three centres, less the triangle of the two centres and the corner,
/// less the three sectors (each disc's from the corner to the round's
/// foot, the round's between its feet).
fn two_discs_rounded() -> (f64, f64) {
    let (a, b, c) = (
        (1.7728134670876865, 17.048669520256194),
        (16.250324167975673, 5.452243981674495),
        (10.715766135516247, 13.378055020402751),
    );
    let o = (0.0, 0.0);
    let dist = |p: (f64, f64), q: (f64, f64)| {
        ((p.0 - q.0) * (p.0 - q.0) + (p.1 - q.1) * (p.1 - q.1)).sqrt()
    };
    let (r1, r2, d) = (dist(a, o), dist(a, c), dist(c, o));
    let a1 = ((d * d + r1 * r1 - r2 * r2) / (2.0 * d * r1)).pacos();
    let a2 = ((d * d + r2 * r2 - r1 * r1) / (2.0 * d * r2)).pacos();
    let lens = r1 * r1 * (a1 - (2.0 * a1).psin() / 2.0) + r2 * r2 * (a2 - (2.0 * a2).psin() / 2.0);
    let union = PI * (r1 * r1 + r2 * r2) - lens;
    let rho = 6.0;
    let (l1, l2) = (r1 + rho, r2 + rho);
    let x = (d * d + l1 * l1 - l2 * l2) / (2.0 * d);
    let h = (l1 * l1 - x * x).sqrt();
    let (ux, uy) = (c.0 / d, c.1 / d);
    let candidates = [
        (x * ux - h * uy, x * uy + h * ux),
        (x * ux + h * uy, x * uy - h * ux),
    ];
    let q = if dist(candidates[0], b) < dist(candidates[1], b) {
        candidates[0]
    } else {
        candidates[1]
    };
    let added = polygon(&[o, q, c])
        - polygon(&[o, b, c])
        - 0.5 * r1 * r1 * angle(o, b, q)
        - 0.5 * r2 * r2 * angle(c, b, q)
        - 0.5 * rho * rho * angle(q, o, c);
    (10.0 * union, 10.0 * (union + added))
}

#[test]
fn one_corner_of_two_merged_discs_is_rounded_by_the_extrusions_own_edge_name() {
    let (v0, rounded) = two_discs_rounded();
    // The same worked out in Python with the same steps; the case's
    // 120000-point shoelace gave the round 1.4755 and 10889.773290.
    assert_rel(v0, 10_875.018_766_864_14, 1e-12, "the discs");
    assert_rel(rounded, 10_889.773_289_948_79, 1e-12, "one corner");
    for k in SCALES {
        let mut s = session();
        two_discs(&mut s, k);
        let listed = run(&mut s, "document.edges", json!({"feature": "Discs"}));
        // The two upright edges, each named by the extrusion after the
        // curves that meet there; the one at B (x = 16.25).
        let upright: Vec<&Value> = listed["edges"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["name"].as_str().unwrap().contains(":lateralEdge("))
            .collect();
        assert_eq!(upright.len(), 2, "{listed}");
        assert_eq!(upright[0]["faces"], upright[1]["faces"]);
        let at_b = upright
            .iter()
            .find(|e| e["start"][0].as_f64().unwrap() > 10.0 * k)
            .unwrap()["name"]
            .as_str()
            .unwrap()
            .to_string();
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": [at_b], "radius": 6.0 * k, "name": "Round"}),
        );
        assert_solid(&mut s, "Round", rounded, None, k);
        // The pair of the two walls: both corners.
        let pair = upright[0]["faces"].clone();
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Round", "edges": [pair]}),
        );
        assert_solid(&mut s, "Round", v0 + 2.0 * (rounded - v0), None, k);
    }
}

// ---------------------------------------------------------------------
// A lofted tube: freeform walls, edges numbered by place

/// Five circles of radius 20 square to an arc of radius 100 turning 60
/// degrees, lofted: a tube whose end circles are four arcs each, between
/// the same cap and wall, at scale `k`.
fn bent_tube(session: &mut Session, k: f64) {
    new_document(session, k);
    for i in 0..5 {
        let turn = f64::from(i) * PI / 12.0;
        let (sin, cos) = (turn.psin(), turn.pcos());
        // The circle's centre on the arc about (0, 100), its plane square
        // to the arc's direction (cos, sin).
        let centre = [100.0 * sin, 100.0 - 100.0 * cos, 0.0];
        let up = [centre[0], centre[1], 1.0];
        let across = [centre[0] + sin, centre[1] - cos, 0.0];
        let plane = format!("Section{i}");
        run(
            session,
            "datum.plane",
            json!({"through": [at(k, centre), at(k, up), at(k, across)], "name": plane}),
        );
        let sketch = format!("Circle{i}");
        run(
            session,
            "sketch.create",
            json!({"plane": plane, "name": sketch}),
        );
        run(
            session,
            "sketch.circle",
            json!({"sketch": sketch, "center": [0, 0], "radius": 20.0 * k}),
        );
    }
    built(
        session,
        "feature.loft",
        json!({"sections": ["Circle0", "Circle1", "Circle2", "Circle3", "Circle4"],
               "name": "Tube"}),
    );
}

#[test]
fn the_arcs_of_a_lofts_end_circle_by_their_names_round_as_the_pair_does() {
    // The torus section 20 about an arc of 100 turning pi/3, less the
    // spandrel (1 - pi/4) 3^2 turned about the tube's axis at its
    // centroid's radius (Pappus): the loft passes through five sections
    // only, so it is held to 5e-4.
    let spandrel = (1.0 - PI / 4.0) * 9.0;
    let centroid = 20.0 - 3.0 * (5.0 / 6.0 - PI / 4.0) / (1.0 - PI / 4.0);
    let expected = PI * 400.0 * 100.0 * PI / 3.0 - 2.0 * PI * centroid * spandrel;
    for k in SCALES {
        let mut s = session();
        bent_tube(&mut s, k);
        let listed = run(&mut s, "document.edges", json!({"feature": "Tube"}));
        // The edges of the start circle: those on the start cap.
        let start: Vec<(Value, String)> = listed["edges"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| {
                e["faces"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|f| f.as_str().unwrap().contains("startCap"))
            })
            .map(|e| (e["faces"].clone(), e["name"].as_str().unwrap().to_string()))
            .collect();
        assert_eq!(start.len(), 4, "{listed}");
        let pair = start[0].0.clone();
        assert!(start.iter().all(|(faces, _)| *faces == pair));
        let names: Vec<String> = start.into_iter().map(|(_, name)| name).collect();
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": names, "radius": 3.0 * k, "name": "Round"}),
        );
        let by_names = volume(&mut s, "Round");
        assert_rel(by_names, expected * k * k * k, 5e-4, "against Pappus");
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Round", "edges": [pair]}),
        );
        let by_pair = volume(&mut s, "Round");
        assert_rel(by_names, by_pair, 1e-9, "the names against the pair");
        common::checked::deep(&mut s);
    }
}
