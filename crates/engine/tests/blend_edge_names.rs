//! A fillet or a chamfer takes one edge by its own name, beside the pair
//! of faces it lies between, which means every edge between them
//! (docs/design/names.md › *An edge by its own name*). Two faces can meet
//! in several edges: a pipe sunk into a plate leaves its top along two
//! lines, two merged discs meet in two corners. The name picks one; the
//! pair still takes all. A name is taken only when every part of it is of
//! a kind proven to stay with its entity (`craft3d_doc::identity`); one
//! with anything else (a number at its end that tells its edge apart by
//! place: the arcs of a loft's end circle, the lines of a cylinder halved
//! lengthwise; a blend's face it lists; a split by place its body grew
//! from) is refused, since an edit could move it to another edge.
//!
//! The oracles are closed forms worked out by hand (the plate and the
//! pipe in the plane across the pipe: the ball's centre, its feet, a
//! polygon less a sector and a circular segment; the two discs: the
//! triangle of the three centres less three sectors), and, where a name
//! is followed, the body built by the name it is followed to, or by the
//! pair less the closed form of the other edge's round. At the scales
//! 1e-3, 1 and 1e6 where the geometry allows.

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
    // Worked out by hand the same way: V0 = 40165.7846, and the round adds
    // 60 x 2.250358.
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
    // 60 x 1.642422; 2 along the plate and 1 along the pipe, and the other
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
    let (v0, rounded) = plate_and_pipe(16.0, 2.0);
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

        // A chamfer put in at the marker on the plate's top edge at x = 0
        // behind the pipe, which ends where the rounded edge does: the
        // faces near that edge change, so its name in the chamfer's body is
        // another (no rule rebuilding the name from those faces would find
        // it), and the round follows the edge the kernel kept. That name
        // lists the chamfer's face, a blend's face, which is not proven to
        // stay with its entity, so it is refused. The oracle: in the
        // chamfer's body, the pair of the top and the wall rounds both
        // edges; the other edge's round, which the chamfer does not reach,
        // adds what it adds without it (`rounded - v0`, by symmetry), and
        // what is left is this edge's round, one face fewer.
        let mut direct = session();
        plate_with_pipe(&mut direct, k, 16.0);
        let [_, high] = pipe_edges(&mut direct, "Joined");
        let listed = run(&mut direct, "document.edges", json!({"feature": "Joined"}));
        let behind: Vec<String> = listed["edges"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| {
                e["faces"] == json!([TOP, "F3:copy(0)/F1:face(4)"])
                    && e["start"][1].as_f64().unwrap() > 20.0 * k
            })
            .map(|e| e["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(behind.len(), 1, "{listed}");
        let side = json!({"edges": [behind[0]], "distance": 1.0 * k, "name": "Side"});
        built(&mut direct, "feature.chamfer", side.clone());
        let [_, there] = pipe_edges(&mut direct, "Side");
        // Near it now: the chamfer's face, no longer the plate's back and
        // left faces.
        assert!(
            there.contains("|F4:copy(0)/F1:face(5),F4:chamfer(0)]"),
            "{high} became {there}"
        );
        let message = error(
            &mut direct,
            "feature.fillet",
            json!({"edges": [there], "radius": 2.0 * k, "name": "Round"}),
        );
        assert!(
            message.contains("holds F4:chamfer(0), a blend's face"),
            "{message}"
        );
        built(
            &mut direct,
            "feature.fillet",
            json!({"edges": [[TOP, WALL]], "radius": 2.0 * k, "name": "Round"}),
        );
        let both = body(&mut direct, "Round");
        let mut s = session();
        plate_with_pipe(&mut s, k, 16.0);
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": [high], "radius": 2.0 * k, "name": "Round"}),
        );
        run(&mut s, "history.rollback", json!({"feature": "Joined"}));
        built(&mut s, "feature.chamfer", side);
        run(&mut s, "history.rollback", json!({"feature": null}));
        assert_eq!(feature(&mut s, "Round")["status"], "ok");
        let round = body(&mut s, "Round");
        assert_rel(
            round["volume"].as_f64().unwrap(),
            both["volume"].as_f64().unwrap() - (rounded - v0) * k * k * k,
            1e-9,
            "the round followed through the chamfer",
        );
        assert_eq!(
            round["faceNames"].as_array().unwrap().len() + 1,
            both["faceNames"].as_array().unwrap().len()
        );
        assert!(centroid_y(&mut s, "Round") > 20.0 * k * (1.0 + 1e-9));
        common::checked::deep(&mut s);
    }
}

#[test]
fn an_edge_name_is_followed_the_same_when_the_cache_let_the_bodies_go() {
    // With a budget of nothing, the union's body goes as soon as the
    // chamfer takes it, and with it the names of its edges: following the
    // union's edge name reads them, so they are derived again first, and
    // the round is the one built with every body kept.
    use common::plain::run;
    let (_, rounded) = plate_and_pipe(16.0, 2.0);
    let mut s = Session::new(Workspace::none());
    s.set_body_budget(0);
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [60, 40, 10], "position": [0, 0, 0], "name": "Plate"}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": 10, "height": 60, "position": [0, 20, 16], "axis": [1, 0, 0],
               "name": "Pipe"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "union", "target": "Plate", "tool": "Pipe", "name": "Joined"}),
    );
    let [_, high] = pipe_edges(&mut s, "Joined");
    run(
        &mut s,
        "feature.chamfer",
        json!({"edges": [["F3:copy(0)/F1:face(0)", "F3:copy(0)/F1:face(2)"]],
               "distance": 1, "name": "Bottom"}),
    );
    assert!(s.bodies_held().unwrap().1 > 0, "the union was let go of");
    let made = run(
        &mut s,
        "feature.fillet",
        json!({"edges": [high], "radius": 2, "name": "Round"}),
    );
    assert_eq!(made["status"], "ok", "{made}");
    let measured = run(&mut s, "document.measure", json!({}));
    assert_rel(
        measured["volume"].as_f64().unwrap(),
        rounded - 30.0,
        1e-9,
        "the round",
    );
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

/// A bar 60 x 20 x 10 (`Bar`, F1) cut in two lumps by a gap at x = 28..32
/// (`Gap`, F2; `Cut`, F3), split (`body.split`), the left lump chamfered
/// along its bottom front edge (`Bottom`), and then rounded along the
/// edge where the gap's left face meets the top, named as `Cut` has it,
/// and along its top left edge, named as the left split has it. The
/// round's body, and the cache's count of bodies let go of before it.
fn split_and_rounded(budget: Option<usize>) -> (Value, usize) {
    use common::plain::run;
    let mut s = Session::new(Workspace::none());
    if let Some(bytes) = budget {
        s.set_body_budget(bytes);
    }
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [60, 20, 10], "position": [0, 0, 0], "name": "Bar"}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"size": [4, 30, 20], "position": [28, -5, -5], "name": "Gap"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Bar", "tool": "Gap", "name": "Cut"}),
    );
    // The gap's left face meets the top in the left lump only.
    let listed = run(&mut s, "document.edges", json!({"feature": "Cut"}));
    let gap_top: Vec<String> = listed["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| {
            let faces = e["faces"].to_string();
            faces.contains("F2:face(4)") && faces.contains("F1:face(1)")
        })
        .map(|e| e["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(gap_top.len(), 1, "{listed}");
    let split = run(&mut s, "body.split", json!({"source": "Cut"}));
    let splits: Vec<String> = split["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["name"].as_str().unwrap().to_string())
        .collect();
    let left = splits
        .iter()
        .find(|name| {
            let inspect = run(&mut s, "document.inspect", json!({}));
            inspect["bodies"]
                .as_array()
                .unwrap()
                .iter()
                .any(|b| b["name"] == **name && b["bbox"][1][0].as_f64().unwrap() < 29.0)
        })
        .unwrap()
        .clone();
    let made = run(
        &mut s,
        "feature.chamfer",
        json!({"edges": [[format!("{left}:copy(0)/Bar:face(0)"), format!("{left}:copy(0)/Bar:face(2)")]],
               "distance": 1, "name": "Bottom"}),
    );
    assert_eq!(made["status"], "ok", "{made}");
    let gone = s.bodies_held().unwrap().1;
    let made = run(
        &mut s,
        "feature.fillet",
        json!({"edges": [gap_top[0],
                         [format!("{left}:copy(0)/Bar:face(1)"), format!("{left}:copy(0)/Bar:face(4)")]],
               "radius": 1, "name": "Round"}),
    );
    assert_eq!(made["status"], "ok", "{made}");
    let inspect = run(&mut s, "document.inspect", json!({}));
    let round = inspect["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == "Round")
        .unwrap()
        .clone();
    (round, gone)
}

#[test]
fn an_edge_name_is_followed_into_the_split_that_has_it_whatever_the_cache_let_go() {
    // The pre-split body's edge is in the left split, which the chamfer
    // took; with every body let go of, the splits other than the last and
    // what took them are derived again too, and the command does not
    // refuse the names as of two bodies: the same round as with every
    // body kept (rule 5).
    let (kept, none) = split_and_rounded(None);
    let (lean, gone) = split_and_rounded(Some(0));
    assert_eq!(none, 0);
    assert!(gone > 0, "the cache let bodies go");
    for key in ["volume", "area", "faces", "edges", "faceNames"] {
        assert_eq!(lean[key], kept[key], "{key}");
    }
}

/// The chord a chamfer 1 along the plate and `b` of arc along the pipe
/// makes of the low edge (by symmetry, the high one's) stays outside the
/// pipe up to b = 4.4657, worked out by hand: G1 = (11, 10), G2 on the
/// pipe `b` along it from the corner (12, 10) away from the plate, and the
/// chord outside while (G1 - G2) . n(G2) >= 0, n the pipe's outward
/// normal at G2.
const CHORD_OUTSIDE_UP_TO: f64 = 4.4657;

#[test]
fn a_chamfer_by_name_along_the_pipe_is_the_closed_form_while_its_chord_stays_outside_it() {
    // The low edge chamfered 1 along the plate and `distance` along the
    // pipe toward the other edge between the same faces (the pipe's wall
    // above the plate runs 2 pi 10 - 20 acos(0.6) = 44.29 from one edge to
    // the other): while the chord stays outside the pipe, the closed form
    // (by symmetry the high edge's) and the deep check; at 40 and 55 too
    // large, the largest that fits being said (half a turn of the pipe).
    let mut s = session();
    plate_with_pipe(&mut s, 1.0, 16.0);
    let [low, _] = pipe_edges(&mut s, "Joined");
    for b in [3.0, 4.0] {
        assert!(b < CHORD_OUTSIDE_UP_TO);
        // The one before, when there is one.
        let _ = s.execute("feature.delete", &json!({"feature": "Bevel"}));
        built(
            &mut s,
            "feature.chamfer",
            json!({"edges": [[low, WALL]], "distance": b, "second": 1, "name": "Bevel"}),
        );
        assert_solid(&mut s, "Bevel", plate_and_pipe_chamfered(1.0, b), None, 1.0);
    }
    for b in [40.0, 55.0] {
        run(
            &mut s,
            "feature.edit",
            json!({"feature": "Bevel", "distance": b}),
        );
        let reason = failure(&mut s, "Bevel");
        assert!(
            reason.contains("edges.0")
                && reason.contains("too large")
                && reason.contains("asked fits; one fits just below"),
            "{reason}"
        );
    }
}

#[test]
fn a_concave_chamfer_whose_chord_would_enter_the_pipe_is_too_large() {
    // Past 4.4657 of arc along the pipe the chord from 1 along the plate
    // passes through the pipe; the chamfer, which fills a concave corner,
    // then took material away: 8 gave 40144.61 and 30 gave 31981.21,
    // against the union's 40165.78 (2026-11-13). It should be refused as
    // too large, the largest that fits about 4.4657.
    let mut s = session();
    plate_with_pipe(&mut s, 1.0, 16.0);
    let [low, _] = pipe_edges(&mut s, "Joined");
    for b in [8.0, 30.0] {
        let made = run(
            &mut s,
            "feature.chamfer",
            json!({"edges": [[low, WALL]], "distance": b, "second": 1, "name": "Bevel"}),
        );
        assert_eq!(made["status"], "failed", "{b}: {made}");
        let reason = made["error"].as_str().unwrap();
        assert!(reason.contains("too large"), "{reason}");
        run(&mut s, "feature.delete", json!({"feature": "Bevel"}));
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
    // The same steps, worked out by hand to these digits.
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
fn the_arcs_of_a_lofts_end_circle_numbered_by_place_are_refused_and_the_pair_rounds_them() {
    // The four arcs lie between the same cap and wall, near the same
    // faces: only their place tells them apart (`(0)` to `(3)`), which an
    // edit can move, so no arc is taken alone by its name. The pair takes
    // all four: the torus section 20 about an arc of 100 turning pi/3,
    // less the spandrel (1 - pi/4) 3^2 turned about the tube's axis at its
    // centroid's radius (Pappus); the loft passes through five sections
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
        assert!(
            names.iter().all(|n| n.ends_with(')') && n.contains("](")),
            "numbered: {names:?}"
        );
        let message = error(
            &mut s,
            "feature.fillet",
            json!({"edges": [names[0]], "radius": 3.0 * k, "name": "Round"}),
        );
        assert!(
            message.contains("only by where it lies in space")
                && message.contains("cannot be taken alone yet"),
            "{message}"
        );
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": [pair], "radius": 3.0 * k, "name": "Round"}),
        );
        assert_rel(
            volume(&mut s, "Round"),
            expected * k * k * k,
            5e-4,
            "against Pappus",
        );
        common::checked::deep(&mut s);
    }
}

// ---------------------------------------------------------------------
// Names numbered by place

/// A cylinder r5 h10 about z cut in half lengthwise by a block from x = 1,
/// both turned by the variable `turn` about z: `Half` (F5). Its two
/// straight edges between the cut and the wall, at y = -+ sqrt(24), lie
/// on the same faces near the same faces, and are numbered `(0)` and
/// `(1)` by where they lie.
fn half_cylinder(session: &mut Session) {
    new_document(session, 1.0);
    run(
        session,
        "variable.set",
        json!({"name": "turn", "expr": "0 deg"}),
    );
    built(
        session,
        "feature.cylinder",
        json!({"radius": 5, "height": 10, "position": [0, 0, 0], "axis": [0, 0, 1],
               "name": "Rod"}),
    );
    built(
        session,
        "feature.block",
        json!({"size": [10, 20, 12], "position": [1, -10, -1], "name": "Cutter"}),
    );
    for source in ["Rod", "Cutter"] {
        built(
            session,
            "feature.transform",
            json!({"source": source, "angle": "turn", "name": format!("{source} turned")}),
        );
    }
    built(
        session,
        "feature.boolean",
        json!({"op": "subtract", "target": "Rod turned", "tool": "Cutter turned",
               "name": "Half"}),
    );
}

/// The straight edges of `feature` whose name ends in `(n)`, by `n`.
fn numbered_lines(session: &mut Session, feature: &str) -> Vec<(String, Value)> {
    let listed = run(session, "document.edges", json!({"feature": feature}));
    let mut found: Vec<(String, Value)> = listed["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["curve"] == "line")
        .filter(|e| e["name"].as_str().unwrap().ends_with(')'))
        .filter(|e| e["name"].as_str().unwrap().contains("]("))
        .map(|e| (e["name"].as_str().unwrap().to_string(), e.clone()))
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

/// The volume a feature `name` adds to the body it takes (negative when it
/// takes material away) and the centroid of what it adds or takes: the
/// feature against itself suppressed, which passes that body on.
fn added_by(session: &mut Session, name: &str) -> (f64, [f64; 3]) {
    let moment = |b: &Value| {
        let v = b["volume"].as_f64().unwrap();
        let c = &b["centroid"];
        (v, [0, 1, 2].map(|i| c[i].as_f64().unwrap() * v))
    };
    let state = feature(session, name);
    assert_eq!(state["status"], "ok", "{state}");
    let after = moment(&body(session, name));
    run(session, "feature.suppress", json!({"feature": name}));
    let before = moment(&body(session, name));
    run(
        session,
        "feature.suppress",
        json!({"feature": name, "suppressed": false}),
    );
    let added = after.0 - before.0;
    assert!(added.abs() > 0.0, "{name} changed nothing");
    (added, [0, 1, 2].map(|i| (after.1[i] - before.1[i]) / added))
}

#[test]
fn a_name_that_lists_a_face_numbered_by_place_is_refused_since_an_edit_could_move_it() {
    // (1) The half cylinder's two lines rounded by the pair: the round's
    // faces are `fillet(0,0)` (at y < 0) and `fillet(0,1)`, numbered by
    // where they lie. (2) A slab across the wall, unchanged by a half
    // turn about z, cut from it. (3) The slab's floor meets the wall in two
    // arcs, told apart by the round near each: their names end in no
    // number, but list `fillet(0,0)` or `fillet(0,1)`. (4) Turned half a
    // turn, `fillet(0,0)` is the image of the other round, so the name of
    // the arc at y < 0 names the image of the other arc, at y < 0 again,
    // without any failure. A blend by that name was taken before and
    // would have moved with it; it is refused now, naming that face.
    let arc_at = |s: &mut Session, name: &str| -> f64 {
        let listed = run(s, "document.edges", json!({"feature": "Slotted"}));
        let edge = listed["edges"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["name"] == name)
            .unwrap_or_else(|| panic!("no {name}: {listed}"))
            .clone();
        edge["start"][1].as_f64().unwrap() + edge["end"][1].as_f64().unwrap()
    };
    let mut s = session();
    half_cylinder(&mut s);
    let lines = numbered_lines(&mut s, "Half");
    let faces = lines[0].1["faces"].clone();
    built(
        &mut s,
        "feature.fillet",
        json!({"edges": [faces], "radius": 1, "name": "Round"}),
    );
    built(
        &mut s,
        "feature.block",
        json!({"size": [4, 40, 2], "position": [-2, -20, 4], "name": "Slab"}),
    );
    // Turned with the rest, so every face but the rounds keeps its name on
    // its image.
    built(
        &mut s,
        "feature.transform",
        json!({"source": "Slab", "angle": "turn", "name": "Slab turned"}),
    );
    built(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Round", "tool": "Slab turned", "name": "Slotted"}),
    );
    let listed = run(&mut s, "document.edges", json!({"feature": "Slotted"}));
    let mut arcs: Vec<(f64, String)> = listed["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["curve"] != "line")
        .filter(|e| {
            let faces = e["faces"].to_string();
            faces.contains("F7:face(0)\"") && faces.contains("F1:face(0)\"")
        })
        .map(|e| {
            let y = e["start"][1].as_f64().unwrap() + e["end"][1].as_f64().unwrap();
            (y, e["name"].as_str().unwrap().to_string())
        })
        .collect();
    arcs.sort_by(|a, b| a.0.total_cmp(&b.0));
    assert_eq!(arcs.len(), 2, "{listed}");
    let low = arcs[0].1.clone();
    assert!(
        low.ends_with(']') && low.contains("F6:fillet(0,0)"),
        "the arc at y < 0 lists the round there: {low}"
    );
    assert!(arc_at(&mut s, &low) < 0.0);
    run(
        &mut s,
        "variable.set",
        json!({"name": "turn", "expr": "180 deg"}),
    );
    // The arc that was at y < 0 is at y > 0 now; the name went with the
    // place.
    assert!(arc_at(&mut s, &low) < 0.0, "the name moved with the place");
    run(
        &mut s,
        "variable.set",
        json!({"name": "turn", "expr": "0 deg"}),
    );
    for edges in [json!([low]), json!([[low, "F9:copy(1)/F7:face(0)"]])] {
        let message = error(
            &mut s,
            "feature.fillet",
            json!({"edges": edges, "radius": 0.3, "name": "Arc"}),
        );
        assert!(
            message.contains("edges.0")
                && message.contains("holds F9:copy(0)/F6:fillet(0,0), a blend's face")
                && message.contains("not proven to stay with its entity"),
            "{message}"
        );
    }
    // A face given with an edge is held to the same rule: it says which
    // side a chamfer's first size is on.
    let message = error(
        &mut s,
        "feature.chamfer",
        json!({"edges": [["F1:edge(0)", "F9:copy(0)/F6:fillet(0,0)"]], "distance": 1,
               "second": 2, "name": "Bevel"}),
    );
    assert!(
        message.contains("edges.0")
            && message.contains("holds F6:fillet(0,0), a blend's face")
            && message.contains("as the face its first size is measured along"),
        "{message}"
    );
}

#[test]
fn names_numbered_by_how_they_were_made_follow_their_entity_through_the_edit_that_moves_places() {
    // The edit that moves a place number: the lens drilled through, turned
    // half a turn about its middle. Its two tip edges are numbered by
    // place as edges of the cut (`(0)` at y < 0 whatever the turn), and
    // answer to the extrusion's names of them (`lateralEdge(e6,e4)`, by
    // the sketch's curves): a round by that name follows its edge to
    // y > 0. And a pattern's instance: a round at the foot of pin 1 of
    // two, named by the instance, follows pin 1 when the step turns
    // round and it stands on the other side of pin 0.
    let mut s = session();
    new_document(&mut s, 1.0);
    run(
        &mut s,
        "variable.set",
        json!({"name": "turn", "expr": "0 deg"}),
    );
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "name": "LensProfile"}),
    );
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
    built(
        &mut s,
        "feature.transform",
        json!({"source": "Lens", "angle": "turn", "pivot": [40, 0, 0], "name": "Turned"}),
    );
    built(
        &mut s,
        "feature.cylinder",
        json!({"radius": 0.3, "height": 4, "position": [40, 0, -1], "axis": [0, 0, 1],
               "name": "Drill"}),
    );
    built(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Turned", "tool": "Drill", "name": "Drilled"}),
    );
    let lines = numbered_lines(&mut s, "Drilled");
    let (zero, line) = &lines[0];
    assert!(zero.ends_with("(0)") && line["start"][1].as_f64().unwrap() < 0.0);
    let alias = line["aliases"][0].as_str().unwrap().to_string();
    assert!(alias.ends_with(":lateralEdge(e6,e4)"), "{alias}");
    // Taken: every part of it is of a kind proven to be identity (the
    // fillet refuses it otherwise).
    built(
        &mut s,
        "feature.fillet",
        json!({"edges": [alias], "radius": 0.1, "name": "Tip"}),
    );
    let (taken, at) = added_by(&mut s, "Tip");
    assert!(taken < 0.0 && at[1] < 0.0, "{taken} at {at:?}");
    run(
        &mut s,
        "variable.set",
        json!({"name": "turn", "expr": "180 deg"}),
    );
    let (again, at) = added_by(&mut s, "Tip");
    assert!(at[1] > 0.0, "the round follows its edge to y > 0: {at:?}");
    assert!(
        (again - taken).abs() <= 1e-9 * taken.abs(),
        "{again} {taken}"
    );
    // The place number stays at y < 0: on the image of the other edge.
    run(&mut s, "history.rollback", json!({"feature": "Drilled"}));
    let (zero_now, line_now) = &numbered_lines(&mut s, "Drilled")[0];
    assert_eq!(zero_now, zero);
    assert!(line_now["start"][1].as_f64().unwrap() < 0.0);
    assert!(line_now["aliases"][0] != json!(alias), "{line_now}");
    run(&mut s, "history.rollback", json!({"feature": null}));

    // Two pins, the second `dx` from the first.
    let mut s = session();
    new_document(&mut s, 1.0);
    run(
        &mut s,
        "variable.set",
        json!({"name": "dx", "expr": "20 mm"}),
    );
    built(
        &mut s,
        "feature.block",
        json!({"size": [100, 40, 10], "position": [0, 0, 0], "name": "Plate"}),
    );
    built(
        &mut s,
        "feature.cylinder",
        json!({"radius": 5, "height": 20, "position": [50, 20, 5], "axis": [0, 0, 1],
               "name": "Pin"}),
    );
    built(
        &mut s,
        "feature.pattern",
        json!({"source": "Pin", "count": 2, "step": ["dx", 0, 0], "name": "Pins"}),
    );
    built(
        &mut s,
        "feature.boolean",
        json!({"op": "union", "target": "Plate", "tool": "Pins", "name": "Joined"}),
    );
    let foot = edge_between(&mut s, "Joined", "F1:face(1)", "copy(1,1)/F2:face(0)");
    // Taken: a name by faces without a number, whose faces are copies of
    // a primitive's (the fillet refuses it otherwise).
    built(
        &mut s,
        "feature.fillet",
        json!({"edges": [foot], "radius": 1, "name": "Foot"}),
    );
    let (added, at) = added_by(&mut s, "Foot");
    assert!(
        added > 0.0 && (at[0] - 70.0).abs() < 1.0,
        "{added} at {at:?}"
    );
    run(
        &mut s,
        "variable.set",
        json!({"name": "dx", "expr": "-20 mm"}),
    );
    let (again, at) = added_by(&mut s, "Foot");
    assert!(
        (at[0] - 30.0).abs() < 1.0,
        "the round follows pin 1: {at:?}"
    );
    assert!((again - added).abs() <= 1e-9 * added, "{again} {added}");
}

#[test]
fn an_edge_named_by_its_place_alone_is_refused_since_an_edit_could_move_it() {
    // Taken, the name `(0)` (the line at y = -sqrt(24)) would round, after
    // both bodies are turned half a turn, the image of the other line: the
    // same solid turned, every name still there, no failure (the stated
    // limit of a numbered name). So it is refused, in the command and in
    // a file, saying that no other name tells that edge apart.
    let mut s = session();
    half_cylinder(&mut s);
    let lines = numbered_lines(&mut s, "Half");
    assert_eq!(lines.len(), 2, "{lines:?}");
    let (zero, line) = &lines[0];
    assert!(zero.ends_with("(0)"), "{zero}");
    assert!(
        line["start"][1].as_f64().unwrap() < 0.0 && line["aliases"].is_null(),
        "{line}"
    );
    let faces = line["faces"].clone();
    for edges in [json!([zero]), json!([[zero, faces[0]]])] {
        let message = error(
            &mut s,
            "feature.fillet",
            json!({"edges": edges, "radius": 1, "name": "Round"}),
        );
        assert!(
            message.contains("edges.0")
                && message.contains("only by where it lies in space")
                && message.contains("cannot be taken alone yet")
                && message.contains(faces[0].as_str().unwrap()),
            "{message}"
        );
    }
    // Nor by an edit of a fillet made by the pair, which takes both lines.
    built(
        &mut s,
        "feature.fillet",
        json!({"edges": [faces], "radius": 1, "name": "Round"}),
    );
    let message = error(
        &mut s,
        "feature.edit",
        json!({"feature": "Round", "edges": [zero]}),
    );
    assert!(
        message.contains("only by where it lies in space"),
        "{message}"
    );
    assert_eq!(feature(&mut s, "Round")["status"], "ok");
}

#[test]
fn a_file_with_an_edge_named_by_its_place_alone_fails_the_feature_with_the_reason() {
    use craft3d_doc::{EdgeRef, Expr, FeatureKind, Fillet, PersistentName};
    let dir = common::workdir("edge-names", "numbered");
    let mut s = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    half_cylinder(&mut s);
    let (zero, _) = numbered_lines(&mut s, "Half").remove(0);
    let path = dir.join("half.3dcraft");
    run(
        &mut s,
        "file.save",
        json!({"path": path.to_str().unwrap(), "overwrite": true}),
    );
    // Another program writes a fillet by that name into the file.
    let mut doc = craft3d_format::from_bytes(&std::fs::read(&path).unwrap()).unwrap();
    let edge: PersistentName = zero.parse().unwrap();
    doc.add_feature(
        Some("Round"),
        FeatureKind::Fillet(Fillet {
            edges: vec![EdgeRef::Edge { edge, first: None }],
            radius: Expr::new("1 mm"),
            chain: false,
            fade: Vec::new(),
        }),
    )
    .unwrap();
    std::fs::write(&path, craft3d_format::to_bytes(&doc).unwrap()).unwrap();
    run(&mut s, "file.open", json!({"path": path.to_str().unwrap()}));
    let reason = failure(&mut s, "Round");
    assert!(
        reason.starts_with("edges.0: ")
            && reason.contains("only by where it lies in space")
            && reason.contains("cannot be taken alone yet"),
        "{reason}"
    );
}

#[test]
fn an_edge_numbered_by_place_that_has_another_name_is_refused_with_that_name() {
    // A lens (two arcs sharing both ends, extruded) drilled through its
    // middle: its two straight edges are numbered by place as edges of
    // the cut, and answer to the extrusion's names of them, which say how
    // each was made.
    let mut s = session();
    new_document(&mut s, 1.0);
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "name": "LensProfile"}),
    );
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
    built(
        &mut s,
        "feature.cylinder",
        json!({"radius": 0.3, "height": 4, "position": [40, 0, -1], "axis": [0, 0, 1],
               "name": "Drill"}),
    );
    built(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Lens", "tool": "Drill", "name": "Drilled"}),
    );
    let lines = numbered_lines(&mut s, "Drilled");
    assert_eq!(lines.len(), 2, "{lines:?}");
    for (name, line) in &lines {
        let alias = line["aliases"][0].as_str().unwrap().to_string();
        assert!(alias.contains(":lateralEdge("), "{line}");
        let message = error(
            &mut s,
            "feature.fillet",
            json!({"edges": [name], "radius": 0.1}),
        );
        assert!(
            message.contains("only by where it lies in space")
                && message.contains(&format!("also called \"{alias}\"")),
            "{message}"
        );
        // The other name is taken, and means that edge: the round it
        // makes takes its material on the edge's side of the lens.
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": [alias], "radius": 0.1, "name": "Tip"}),
        );
        let (taken, at) = added_by(&mut s, "Tip");
        let y = line["start"][1].as_f64().unwrap();
        assert!(
            taken < 0.0 && at[1] * y > 0.0,
            "{taken} at {at:?}, edge at y {y}"
        );
        run(&mut s, "feature.delete", json!({"feature": "Tip"}));
    }
}

// ---------------------------------------------------------------------
// Through a blend that uses a face up

/// The edge of the body of `feature` between the faces whose names end in
/// `a` and in `b`: its own name.
fn edge_between(session: &mut Session, feature: &str, a: &str, b: &str) -> String {
    let listed = run(session, "document.edges", json!({"feature": feature}));
    let found: Vec<String> = listed["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| {
            let faces: Vec<&str> = e["faces"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| f.as_str().unwrap())
                .collect();
            faces.len() == 2
                && ((faces[0].ends_with(a) && faces[1].ends_with(b))
                    || (faces[0].ends_with(b) && faces[1].ends_with(a)))
        })
        .map(|e| e["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(found.len(), 1, "{a} / {b}: {listed}");
    found[0].clone()
}

#[test]
fn edge_names_through_a_blend_that_uses_a_face_up_are_followed_or_refused() {
    // A bar 80 x 20 x 20 whose end x = 80 is rounded 10 on one upright
    // (`First`), then 10 on the other (`Second`): the second round uses up
    // what is left of the end, and its two edges along y = 10 (the first
    // round's border and the second's) become one edge between the two
    // rounds, which the kernel reports as made. Named as `First`'s body
    // has them: the first round's border on the end lists that round's
    // face, which is not proven to stay with its entity, and is refused;
    // the end's top edge (taken whole) is gone after `Second`, and says
    // so; the top's edge at x = 0, which the rounds do not touch, is
    // followed to the edge `Second` kept of it and rounded 2 by that
    // name. The two rounds
    // leave the section 80 x 20 less two spandrels (1 - pi/4) 100, along
    // 20; the third takes (1 - pi/4) 4 along 20.
    let spandrel = 1.0 - PI / 4.0;
    let two_rounds = (1600.0 - 2.0 * spandrel * 100.0) * 20.0;
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        built(
            &mut s,
            "feature.block",
            json!({"size": at(k, [80.0, 20.0, 20.0]), "position": [0, 0, 0], "name": "Bar"}),
        );
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": [["F1:face(2)", "F1:face(5)"]], "radius": 10.0 * k, "name": "First"}),
        );
        let border = edge_between(&mut s, "First", "F1:face(5)", "F2:fillet(0)");
        let end_top = edge_between(&mut s, "First", "F1:face(5)", "F1:face(1)");
        let far_top = edge_between(&mut s, "First", "F1:face(4)", "F1:face(1)");
        assert!(far_top.contains(":between["), "{far_top}");
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": [["F2:copy(0)/F1:face(3)", "F2:copy(0)/F1:face(5)"]],
                   "radius": 10.0 * k, "name": "Second"}),
        );
        let second = assert_solid(&mut s, "Second", two_rounds, None, k);
        assert!(
            !common::names_of(&second)
                .iter()
                .any(|n| n.ends_with("F1:face(5)")),
            "the end is used up: {second}"
        );
        // The first round's border lists that round's face, a blend's
        // face: refused as it is given, before it could be followed.
        let message = error(
            &mut s,
            "feature.fillet",
            json!({"edges": [border], "radius": 1.0 * k, "name": "Gone"}),
        );
        assert!(
            message.contains("edges.0") && message.contains("holds F2:fillet(0), a blend's face"),
            "the first round's border: {message}"
        );
        let reason = failed(
            &mut s,
            "feature.fillet",
            json!({"edges": [end_top], "radius": 1.0 * k, "name": "Gone"}),
        );
        assert!(
            reason.contains("edges.0") && reason.contains("is gone") && reason.contains("`Second`"),
            "the end's top edge: {reason}"
        );
        run(&mut s, "feature.delete", json!({"feature": "Gone"}));
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": [far_top], "radius": 2.0 * k, "name": "Third"}),
        );
        assert_solid(&mut s, "Third", two_rounds - spandrel * 4.0 * 20.0, None, k);
        let followed = feature(&mut s, "Third")["followed"]["edges"][0].clone();
        assert!(
            followed.as_str().unwrap().starts_with("F3:between["),
            "{followed}"
        );
    }
}

#[test]
#[ignore = "owed, older than edge names (0.1.0): a face pair that lists a blend face numbered by place moves with the place; the pair path is kept as it was for the files that hold such pairs, docs/design/names.md"]
fn a_pair_that_lists_a_blend_face_numbered_by_place_follows_that_face() {
    // The half cylinder's two lines rounded by the pair (`fillet(0,0)` at
    // y < 0, `fillet(0,1)`): the pair of `fillet(0,0)` and the top names
    // the edge where that round meets the top, at y < 0. After both bodies
    // are turned half a turn, `fillet(0,0)` is the image of the other
    // round, and the same pair names the image of the other edge, at y < 0
    // again, without a failure; a blend given that pair goes with it.
    // Following the round it named, the pair would name the edge at y > 0.
    // (Shown by the edges the pair names: a chamfer of that edge, its end
    // between a round and the cut, is refused by the kernel.)
    let mut s = session();
    half_cylinder(&mut s);
    let lines = numbered_lines(&mut s, "Half");
    let faces = lines[0].1["faces"].clone();
    built(
        &mut s,
        "feature.fillet",
        json!({"edges": [faces], "radius": 1, "name": "Round"}),
    );
    let pair = json!(["F6:fillet(0,0)", "F6:copy(0)/F1:face(2)"]);
    let side = |s: &mut Session| {
        let listed = run(
            s,
            "document.edges",
            json!({"feature": "Round", "between": pair}),
        );
        let edges = listed["edges"].as_array().unwrap();
        assert_eq!(edges.len(), 1, "{listed}");
        edges[0]["start"][1].as_f64().unwrap() + edges[0]["end"][1].as_f64().unwrap()
    };
    assert!(side(&mut s) < 0.0);
    run(
        &mut s,
        "variable.set",
        json!({"name": "turn", "expr": "180 deg"}),
    );
    assert!(side(&mut s) > 0.0, "the pair stayed with the place");
}

// ---------------------------------------------------------------------
// Only names proven to stay with their entity

/// The middle of the edge of `feature`'s body that is called `name` (its
/// own name or an alias).
fn middle_of(session: &mut Session, feature: &str, name: &str) -> [f64; 3] {
    let listed = run(session, "document.edges", json!({"feature": feature}));
    let edge = listed["edges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| {
            e["name"] == name
                || e["aliases"]
                    .as_array()
                    .is_some_and(|aliases| aliases.iter().any(|a| a == name))
        })
        .unwrap_or_else(|| panic!("no {name}: {listed}"))
        .clone();
    [0, 1, 2].map(|i| 0.5 * (edge["start"][i].as_f64().unwrap() + edge["end"][i].as_f64().unwrap()))
}

#[test]
fn a_name_that_lists_a_blend_face_is_refused_since_leaving_out_a_reference_renumbers_it() {
    // A base 100 x 40 x 10 with a slot 10 wide and 5 deep across its top
    // at x 40..50 while `slot` is 1, and a chamfer 1 of two edges of the
    // top by pairs: the slot's wall at x = 40 (reference 0) and the end at
    // x = 0 (reference 1). A blend's face is numbered by its reference's
    // place among those that are there: `chamfer(0)` is the slot's, x
    // 39..40, and the line between the top and it lies at x = 39. With
    // `slot` 0 the cutter is off, reference 0 names a face that is not
    // there and is left out, and `chamfer(0)` is the end's, x 0..1: the
    // same name is the line at x = 1, without any failure. A round by
    // that name would move with it; it is refused, by the command and in
    // a file, naming the blend's face.
    let mut s = session();
    new_document(&mut s, 1.0);
    run(&mut s, "variable.set", json!({"name": "slot", "expr": "1"}));
    built(
        &mut s,
        "feature.block",
        json!({"size": [100, 40, 10], "position": [0, 0, 0], "name": "Base"}),
    );
    built(
        &mut s,
        "feature.block",
        json!({"size": [10, 42, 6], "position": [40, -1, 5], "name": "Cutter", "when": "slot"}),
    );
    built(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Base", "tool": "Cutter", "name": "Slotted"}),
    );
    built(
        &mut s,
        "feature.chamfer",
        json!({"edges": [["F3:copy(1)/F2:face(4)", "F1:face(1)"], ["F1:face(1)", "F1:face(4)"]],
               "distance": 1, "name": "Bevel"}),
    );
    let name = "F4:between[F4:copy(0)/F1:face(1),F4:chamfer(0)]";
    let at = middle_of(&mut s, "Bevel", name);
    assert!(
        (at[0] - 39.0).abs() < 1e-9 && (at[2] - 10.0).abs() < 1e-9,
        "{at:?}"
    );
    run(&mut s, "variable.set", json!({"name": "slot", "expr": "0"}));
    assert_eq!(feature(&mut s, "Bevel")["status"], "ok");
    let at = middle_of(&mut s, "Bevel", name);
    assert!(
        (at[0] - 1.0).abs() < 1e-9 && (at[2] - 10.0).abs() < 1e-9,
        "the name moved to the end's line: {at:?}"
    );
    run(&mut s, "variable.set", json!({"name": "slot", "expr": "1"}));
    let message = error(
        &mut s,
        "feature.fillet",
        json!({"edges": [name], "radius": 0.3, "name": "Round"}),
    );
    assert!(
        message.contains("edges.0")
            && message.contains("holds F4:chamfer(0), a blend's face")
            && message.contains("not proven to stay with its entity"),
        "{message}"
    );
    // The pair of the line's faces is taken as it always was (and moves
    // with the number as it always did: docs/decisions.md, D-open-22).
    built(
        &mut s,
        "feature.fillet",
        json!({"edges": [["F4:copy(0)/F1:face(1)", "F4:chamfer(0)"]], "radius": 0.3,
               "name": "Round"}),
    );
    // A file another program wrote with the round by that name: the
    // feature fails with the same reason at every regeneration.
    let dir = common::workdir("edge-names", "blend-face");
    let path = dir.join("slot.3dcraft");
    let mut s = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    new_document(&mut s, 1.0);
    run(&mut s, "variable.set", json!({"name": "slot", "expr": "1"}));
    for (id, params) in [
        (
            "feature.block",
            json!({"size": [100, 40, 10], "position": [0, 0, 0], "name": "Base"}),
        ),
        (
            "feature.block",
            json!({"size": [10, 42, 6], "position": [40, -1, 5], "name": "Cutter", "when": "slot"}),
        ),
        (
            "feature.boolean",
            json!({"op": "subtract", "target": "Base", "tool": "Cutter", "name": "Slotted"}),
        ),
        (
            "feature.chamfer",
            json!({"edges": [["F3:copy(1)/F2:face(4)", "F1:face(1)"], ["F1:face(1)", "F1:face(4)"]],
                   "distance": 1, "name": "Bevel"}),
        ),
    ] {
        built(&mut s, id, params);
    }
    run(
        &mut s,
        "file.save",
        json!({"path": path.to_str().unwrap(), "overwrite": true}),
    );
    let mut doc = craft3d_format::from_bytes(&std::fs::read(&path).unwrap()).unwrap();
    doc.add_feature(
        Some("Round"),
        craft3d_doc::FeatureKind::Fillet(craft3d_doc::Fillet {
            edges: vec![craft3d_doc::EdgeRef::Edge {
                edge: name.parse().unwrap(),
                first: None,
            }],
            radius: craft3d_doc::Expr::new("0.3 mm"),
            chain: false,
            fade: Vec::new(),
        }),
    )
    .unwrap();
    std::fs::write(&path, craft3d_format::to_bytes(&doc).unwrap()).unwrap();
    run(&mut s, "file.open", json!({"path": path.to_str().unwrap()}));
    for slot in ["1", "0"] {
        run(
            &mut s,
            "variable.set",
            json!({"name": "slot", "expr": slot}),
        );
        let reason = failure(&mut s, "Round");
        assert!(
            reason.starts_with("edges.0: ")
                && reason.contains("holds F4:chamfer(0), a blend's face"),
            "slot {slot}: {reason}"
        );
    }
}

#[test]
fn an_edge_of_a_lump_a_split_chose_by_where_it_lies_is_refused() {
    use craft3d_doc::FeatureKind;
    // A base 100 x 40 x 10 cut through across x 40..50: two lumps, each
    // with an end of its own. Split, and then (as another program may
    // write it) each split told by the top, which both lumps hold pieces
    // of, and its number by where the lumps lie: an edit that moves the
    // lumps past each other gives each split the other lump, and an edge
    // named in it the other edge. So an edge given in that body, written
    // in its name or followed into it, is refused.
    let dir = common::workdir("edge-names", "split-by-place");
    let path = dir.join("lumps.3dcraft");
    let mut s = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    new_document(&mut s, 1.0);
    built(
        &mut s,
        "feature.block",
        json!({"size": [100, 40, 10], "position": [0, 0, 0], "name": "Base"}),
    );
    built(
        &mut s,
        "feature.block",
        json!({"size": [10, 42, 12], "position": [40, -1, -1], "name": "Cutter"}),
    );
    built(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Base", "tool": "Cutter", "name": "Cut"}),
    );
    // The top edge of the end x = 0, as the cut names it.
    let listed = run(&mut s, "document.edges", json!({"feature": "Cut"}));
    let cut_name: Vec<String> = listed["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| {
            e["faces"] == json!(["F3:copy(0)/F1:face(1)", "F3:copy(0)/F1:face(4)"])
                || e["faces"] == json!(["F3:copy(0)/F1:face(4)", "F3:copy(0)/F1:face(1)"])
        })
        .map(|e| e["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(cut_name.len(), 1, "{listed}");
    run(&mut s, "body.split", json!({"source": "Cut"}));
    run(
        &mut s,
        "file.save",
        json!({"path": path.to_str().unwrap(), "overwrite": true}),
    );
    let mut doc = craft3d_format::from_bytes(&std::fs::read(&path).unwrap()).unwrap();
    let mut number = 0;
    for feature in &mut doc.features {
        if let FeatureKind::Split(split) = &mut feature.kind {
            split.face = "F3:copy(0)/F1:face(1)".parse().unwrap();
            split.number = Some(number);
            number += 1;
        }
    }
    assert_eq!(number, 2);
    std::fs::write(&path, craft3d_format::to_bytes(&doc).unwrap()).unwrap();
    run(&mut s, "file.open", json!({"path": path.to_str().unwrap()}));
    // Number 0 is the lump nearer x = 0: the first split, F4.
    let listed = run(&mut s, "document.edges", json!({"feature": "F4"}));
    let end_top: Vec<String> = listed["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| {
            e["faces"] == json!(["F4:copy(0)/F1:face(1)", "F4:copy(0)/F1:face(4)"])
                || e["faces"] == json!(["F4:copy(0)/F1:face(4)", "F4:copy(0)/F1:face(1)"])
        })
        .map(|e| e["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(end_top.len(), 1, "{listed}");
    let message = error(
        &mut s,
        "feature.fillet",
        json!({"edges": [end_top[0]], "radius": 1, "name": "Round"}),
    );
    assert!(
        message.contains("edges.0") && message.contains("grown from the split"),
        "{message}"
    );
    // The same edge as the cut named it, which names no split: taken by
    // the command, and refused when regeneration follows it into the
    // split's body.
    let reason = failed(
        &mut s,
        "feature.fillet",
        json!({"edges": [cut_name[0]], "radius": 1, "name": "Round"}),
    );
    assert!(
        reason.starts_with("edges.0: ") && reason.contains("grown from the split"),
        "{reason}"
    );
}
