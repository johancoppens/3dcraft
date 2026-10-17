//! Fillets through the commands (M9, the second blend): every body is
//! measured against a closed form worked out by hand, at three scales, and
//! checked deep; the references follow upstream edits and fail by name;
//! the faces a fillet makes carry names later features build on; a saved
//! document reopens to the same bodies; and what is not built, or does not
//! fit, is refused with the edge, the face and the reason.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod blend_common;
mod common;

use blend_common::*;
use common::checked::{
    assert_solid, body, built, failed, feature, inspect, names_of, new_document, refused, run,
    session,
};
use craft3d_engine::{Access, Session, Workspace};
use serde_json::{Value, json};
use std::f64::consts::PI;
use std::path::PathBuf;

fn workdir(name: &str) -> PathBuf {
    common::workdir("fillet", name)
}

/// What a square corner loses to a round of radius 1.
const SPANDREL: f64 = 1.0 - PI / 4.0;

fn names_sorted(body: &Value) -> Vec<String> {
    let mut names = names_of(body);
    names.sort();
    names
}

// ------------------------------------------------------------ straight edges

#[test]
fn one_edge_of_a_block_becomes_a_quarter_cylinder() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        block(&mut s, k);
        let r = 0.5;
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": [[TOP, FRONT]], "radius": r * k, "name": "Round"}),
        );
        // The top and the front lose a strip of r by 4 each, the two ends
        // a spandrel each, and the quarter cylinder is new.
        let area = 52.0 - 2.0 * r * 4.0 - 2.0 * SPANDREL * r * r + PI / 2.0 * r * 4.0;
        let solid = assert_solid(
            &mut s,
            "Round",
            24.0 - SPANDREL * r * r * 4.0,
            Some(area),
            k,
        );
        assert_eq!(
            (&solid["faces"], &solid["edges"], &solid["vertices"]),
            (&json!(7), &json!(15), &json!(10))
        );
        assert_eq!(
            names_sorted(&solid),
            [
                "F2:copy(0)/F1:face(0)",
                "F2:copy(0)/F1:face(1)",
                "F2:copy(0)/F1:face(2)",
                "F2:copy(0)/F1:face(3)",
                "F2:copy(0)/F1:face(4)",
                "F2:copy(0)/F1:face(5)",
                "F2:fillet(0)",
            ]
        );
        // The block's body went into the fillet.
        assert_eq!(feature(&mut s, "Base")["bodyUsedBy"], "F2");
        // The new face is a cylinder whose axis is the radius from both
        // faces: a datum axis on it lies there.
        let axis = built(
            &mut s,
            "datum.axis",
            json!({"face": "F2:fillet(0)", "name": "Axis"}),
        );
        let (origin, direction) = (
            point(&axis["datum"]["origin"]),
            point(&axis["datum"]["direction"]),
        );
        assert!((origin[1] - r * k).abs() <= 1e-9 * k, "{axis}");
        assert!((origin[2] - (2.0 - r) * k).abs() <= 1e-9 * k, "{axis}");
        assert!(direction[0].abs() > 1.0 - 1e-12, "{axis}");
        // Every edge and vertex has a name of its own, and those the block
        // had answer to the name they had there.
        let listed = run(&mut s, "document.edges", json!({"feature": "Round"}));
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
        // The seams are edges between the fillet and its neighbours.
        let seams = listed["edges"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| {
                let name = e["name"].as_str().unwrap();
                name.contains("F2:fillet(0)")
                    && (name.contains("F1:face(1)") || name.contains("F1:face(2)"))
            })
            .count();
        assert_eq!(seams, 2, "{listed}");
    }
}

/// The twelve edges of a block as pairs of its faces.
fn twelve() -> Value {
    json!([
        [TOP, FRONT],
        [TOP, BACK],
        [TOP, LEFT],
        [TOP, RIGHT],
        [BOTTOM, FRONT],
        [BOTTOM, BACK],
        [BOTTOM, LEFT],
        [BOTTOM, RIGHT],
        [FRONT, LEFT],
        [FRONT, RIGHT],
        [BACK, LEFT],
        [BACK, RIGHT],
    ])
}

#[test]
fn all_twelve_edges_of_a_block_with_their_eight_corners() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        block(&mut s, k);
        let r = 0.25;
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": twelve(), "radius": r * k, "name": "Round"}),
        );
        // The smaller block grown by a ball (Steiner).
        let (a, b, c) = (4.0 - 2.0 * r, 3.0 - 2.0 * r, 2.0 - 2.0 * r);
        let face = 2.0 * (a * b + a * c + b * c);
        let volume = a * b * c + face * r + PI * r * r * (a + b + c) + 4.0 / 3.0 * PI * r * r * r;
        let area = face + 2.0 * PI * r * (a + b + c) + 4.0 * PI * r * r;
        let solid = assert_solid(&mut s, "Round", volume, Some(area), k);
        assert_eq!(
            (&solid["faces"], &solid["edges"], &solid["vertices"]),
            (&json!(26), &json!(48), &json!(24))
        );
        let names = names_of(&solid);
        for index in 0..12 {
            assert!(names.contains(&format!("F2:fillet({index})")), "{names:?}");
        }
        let corners: Vec<&String> = names
            .iter()
            .filter(|n| n.starts_with("F2:filletCorner("))
            .collect();
        assert_eq!(corners.len(), 8, "{names:?}");
        // The corner of the top, the front and the left: by the places of
        // those three edges in the list.
        assert!(
            names.contains(&"F2:filletCorner(0,2,8)".to_string()),
            "{names:?}"
        );
        // A smaller radius by a variable: the same closed form.
        run(
            &mut s,
            "variable.set",
            json!({"name": "r", "expr": 0.1 * k}),
        );
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Round", "radius": "r"}),
        );
        let r = 0.1;
        let (a, b, c) = (4.0 - 2.0 * r, 3.0 - 2.0 * r, 2.0 - 2.0 * r);
        let face = 2.0 * (a * b + a * c + b * c);
        let volume = a * b * c + face * r + PI * r * r * (a + b + c) + 4.0 / 3.0 * PI * r * r * r;
        assert_solid(&mut s, "Round", volume, None, k);
    }
}

#[test]
fn one_two_and_three_edges_at_a_corner() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        block(&mut s, k);
        let r = 0.5;
        // Two that meet, the third sharp: they run out into each other.
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": [[TOP, FRONT], [TOP, LEFT]], "radius": r * k, "name": "Round"}),
        );
        let two = 24.0 - SPANDREL * r * r * 7.0 + (5.0 / 3.0 - PI / 2.0) * r * r * r;
        let solid = assert_solid(&mut s, "Round", two, None, k);
        assert_eq!(solid["faces"], 8);
        // Three: an eighth of a sphere in the corner.
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Round", "edges": [[TOP, FRONT], [TOP, LEFT], [FRONT, LEFT]]}),
        );
        let left = PI / 6.0 * r * r * r;
        let three = 24.0 - SPANDREL * r * r * (9.0 - 3.0 * r) - (r * r * r - left);
        let area = 52.0 - 2.0 * r * 9.0 + 3.0 * r * r - 3.0 * SPANDREL * r * r
            + PI / 2.0 * r * (9.0 - 3.0 * r)
            + PI / 2.0 * r * r;
        let solid = assert_solid(&mut s, "Round", three, Some(area), k);
        assert_eq!(
            (&solid["faces"], &solid["edges"], &solid["vertices"]),
            (&json!(10), &json!(21), &json!(13))
        );
        assert!(
            names_of(&solid).contains(&"F2:filletCorner(0,1,2)".to_string()),
            "{solid}"
        );
    }
}

/// A prism on a regular polygon of `sides` sides with circumradius 2.
fn regular_prism(session: &mut Session, k: f64, sides: u32, height: f64) {
    run(session, "sketch.create", json!({"name": "Outline"}));
    let points: Vec<[f64; 2]> = (0..sides)
        .map(|i| {
            let a = f64::from(i) * 2.0 * PI / f64::from(sides);
            [2.0 * k * a.cos(), 2.0 * k * a.sin()]
        })
        .collect();
    run(
        session,
        "sketch.polyline",
        json!({"sketch": "Outline", "closed": true, "points": points}),
    );
    built(
        session,
        "feature.extrude",
        json!({"sketch": "Outline", "distance": height * k, "name": "Prism"}),
    );
}

#[test]
fn edges_between_faces_at_sixty_and_at_a_hundred_and_twenty_degrees() {
    for k in SCALES {
        for (sides, theta, base, side) in [
            (6_u32, 2.0 * PI / 3.0, 6.0 * 3.0_f64.sqrt(), 2.0),
            (3, PI / 3.0, 3.0 * 3.0_f64.sqrt(), 2.0 * 3.0_f64.sqrt()),
        ] {
            let mut s = session();
            new_document(&mut s, k);
            let (h, r) = (2.5, 0.3);
            regular_prism(&mut s, k, sides, h);
            // The upright edge through the first corner.
            let pair = edge_between(&mut s, "Prism", k, [2.0, 0.0, 0.0], [2.0, 0.0, h]);
            built(
                &mut s,
                "feature.fillet",
                json!({"edges": [pair], "radius": r * k, "name": "Round"}),
            );
            // The kite between the two tangent lengths less the sector.
            let half = theta / 2.0;
            let section = r * r * (1.0 / half.tan() - (PI - theta) / 2.0);
            let d = r / half.tan();
            let area = 2.0 * base + f64::from(sides) * side * h - 2.0 * d * h - 2.0 * section
                + (PI - theta) * r * h;
            assert_solid(&mut s, "Round", (base - section) * h, Some(area), k);
            // The axis: on the bisector, r / sin(theta / 2) from the edge.
            let axis = built(
                &mut s,
                "datum.axis",
                json!({"face": "F3:fillet(0)", "name": "Axis"}),
            );
            let origin = point(&axis["datum"]["origin"]);
            assert!(
                (origin[0] - (2.0 - r / half.sin()) * k).abs() <= 1e-9 * k,
                "{axis}"
            );
            assert!(origin[1].abs() <= 1e-9 * k, "{axis}");
        }
    }
}

#[test]
fn the_inside_corner_of_an_l_gains_material() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        ell(&mut s, k);
        let inside = edge_between(&mut s, "Ell", k, [1.5, 1.5, 0.0], [1.5, 1.5, 2.0]);
        let ledge = edge_between(&mut s, "Ell", k, [1.5, 1.5, 2.0], [1.5, 3.0, 2.0]);
        let r = 0.4;
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": [inside], "radius": r * k, "name": "Round"}),
        );
        let area =
            2.0 * 8.25 + 14.0 * 2.0 - 2.0 * r * 2.0 + 2.0 * SPANDREL * r * r + PI / 2.0 * r * 2.0;
        let solid = assert_solid(
            &mut s,
            "Round",
            16.5 + SPANDREL * r * r * 2.0,
            Some(area),
            k,
        );
        assert_eq!(solid["faces"], 9);
        // The top edge of the short wall ends against the other wall, in
        // the inside corner: it stops there.
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Round", "edges": [ledge]}),
        );
        assert_solid(&mut s, "Round", 16.5 - SPANDREL * r * r * 1.5, None, k);
        // Both at once meet in a corner that mixes an outside and an
        // inside fillet: refused.
        let reason = failed(
            &mut s,
            "feature.edit",
            json!({"feature": "Round", "edges": [inside, ledge]}),
        );
        assert!(reason.contains("does not end cleanly"), "{reason}");
    }
}

// ------------------------------------------------------------ circular edges

/// What the corner at `rim` of a section in the half-plane (distance from
/// the axis, height), whose sides leave it along the unit directions `b1`
/// and `b2`, loses to a round of radius `r` in a full turn (Pappus: the
/// kite between the tangent lengths less the sector of the ball, each at
/// its own centroid), and the area of the torus the arc sweeps.
fn ring(rim: [f64; 2], b1: [f64; 2], b2: [f64; 2], r: f64) -> (f64, f64) {
    let theta = (b1[0] * b2[0] + b1[1] * b2[1]).clamp(-1.0, 1.0).acos();
    let d = r / (theta / 2.0).tan();
    let t1 = rim[0] + d * b1[0];
    let t2 = rim[0] + d * b2[0];
    let far = d / (1.0 + theta.cos());
    let centre = [
        rim[0] + far * (b1[0] + b2[0]),
        rim[1] + far * (b1[1] + b2[1]),
    ];
    let kite = 0.5 * d * r * ((rim[0] + t1 + centre[0]) + (rim[0] + t2 + centre[0])) / 3.0;
    let alpha = (PI - theta) / 2.0;
    let to_rim = (rim[0] - centre[0]) / (rim[0] - centre[0]).hypot(rim[1] - centre[1]);
    let sector = r * r * alpha * (centre[0] + to_rim * 2.0 * r * alpha.sin() / (3.0 * alpha));
    let arc_at = centre[0] + to_rim * r * alpha.sin() / alpha;
    (
        2.0 * PI * (kite - sector),
        2.0 * PI * arc_at * 2.0 * alpha * r,
    )
}

#[test]
fn the_rim_of_a_rod_and_the_base_of_a_cone_become_tori() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        let (rr, h, r) = (1.0, 3.0, 0.25);
        built(
            &mut s,
            "feature.cylinder",
            json!({"name": "Rod", "radius": rr * k, "height": h * k}),
        );
        // A cylinder's faces: 0 the wall, 1 the bottom, 2 the top.
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": [["F1:face(2)", "F1:face(0)"]], "radius": r * k, "name": "Round"}),
        );
        let (removed, torus) = ring([rr, h], [-1.0, 0.0], [0.0, -1.0], r);
        let area = PI * rr * rr + 2.0 * PI * rr * (h - r) + PI * (rr - r) * (rr - r) + torus;
        let solid = assert_solid(&mut s, "Round", PI * rr * rr * h - removed, Some(area), k);
        assert_eq!(solid["faces"], 4);
        // Both ends.
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Round",
                   "edges": [["F1:face(2)", "F1:face(0)"], ["F1:face(1)", "F1:face(0)"]]}),
        );
        assert_solid(&mut s, "Round", PI * rr * rr * h - 2.0 * removed, None, k);
        // Half the rod's radius: the torus would reach its own axis.
        let reason = failed(
            &mut s,
            "feature.edit",
            json!({"feature": "Round", "radius": 0.5 * k}),
        );
        assert!(reason.contains("half the rim's"), "{reason}");
        // The base of a cone: the faces meet at atan(h / r).
        let mut s = session();
        new_document(&mut s, k);
        let (rr, h, r) = (1.0, 2.0, 0.2);
        built(
            &mut s,
            "feature.cone",
            json!({"name": "Cone", "radius": rr * k, "height": h * k}),
        );
        let base = edge_between(&mut s, "Cone", k, [rr, 0.0, 0.0], [rr, 0.0, 0.0]);
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": [base], "radius": r * k, "name": "Round"}),
        );
        let slant = rr.hypot(h);
        let (removed, _) = ring([rr, 0.0], [-1.0, 0.0], [-rr / slant, h / slant], r);
        assert_solid(&mut s, "Round", PI * rr * rr * h / 3.0 - removed, None, k);
    }
}

#[test]
fn the_mouth_of_a_hole_loses_material_and_inside_rims_gain_it() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        drilled_plate(&mut s, k);
        let (rh, r) = (0.5, 0.2);
        let part = 48.0 - PI * rh * rh * 2.0 - PI * rh * rh * 1.0 + PI * rh * rh * 1.0;
        assert_solid(&mut s, "Part", part, None, k);
        let mouth = edge_between(&mut s, "Part", k, [2.0, 1.5, 2.0], [2.0, 1.5, 2.0]);
        let floor = edge_between(&mut s, "Part", k, [4.5, 1.5, 1.0], [4.5, 1.5, 1.0]);
        let foot = edge_between(&mut s, "Part", k, [7.0, 1.5, 2.0], [7.0, 1.5, 2.0]);
        let (out, _) = ring([rh, 2.0], [1.0, 0.0], [0.0, -1.0], r);
        let (bottom, _) = ring([rh, 1.0], [-1.0, 0.0], [0.0, 1.0], r);
        let (base, _) = ring([rh, 2.0], [1.0, 0.0], [0.0, 1.0], r);
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": [mouth, floor, foot], "radius": r * k, "name": "Rims"}),
        );
        let solid = assert_solid(&mut s, "Rims", part - out + bottom + base, None, k);
        let names = names_of(&solid);
        for index in 0..3 {
            assert!(names.contains(&format!("F8:fillet({index})")), "{names:?}");
        }
        // A ball that does not fit in the blind hole. What fits is below
        // half the hole's radius: from there on the torus would reach its
        // own axis (until 2026-10-15 the estimate said the hole's radius,
        // while every radius from half of it up was refused that way).
        let reason = failed(
            &mut s,
            "feature.edit",
            json!({"feature": "Rims", "edges": [floor], "radius": 0.6 * k}),
        );
        assert!(
            reason.starts_with("radius: too large for edges.0"),
            "{reason}"
        );
        assert!(
            reason.contains(&format!(
                "the largest radius that fits there is below {} mm",
                0.25 * k
            )),
            "{reason}"
        );
    }
}

#[test]
fn a_chain_of_lines_and_arcs_is_picked_by_one_of_its_edges() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        slot(&mut s, k);
        let whole = (6.0 + PI) * 2.0;
        assert_solid(&mut s, "Slot", whole, None, k);
        // One straight edge of the top rim.
        let pair = edge_between(&mut s, "Slot", k, [0.0, -1.0, 2.0], [3.0, -1.0, 2.0]);
        let r = 0.3;
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": [pair], "radius": r * k, "chain": true, "name": "Rim"}),
        );
        // Two strips 3 long and a ring in two halves; the cylinders and
        // the tori continue each other.
        let (turned, torus) = ring([1.0, 2.0], [-1.0, 0.0], [0.0, -1.0], r);
        let removed = 2.0 * 3.0 * SPANDREL * r * r + turned;
        let area = (6.0 + PI)
            + (3.0 * 2.0 * (1.0 - r) + PI * (1.0 - r) * (1.0 - r))
            + (6.0 + 2.0 * PI) * (2.0 - r)
            + 2.0 * 3.0 * PI / 2.0 * r
            + torus;
        let solid = assert_solid(&mut s, "Rim", whole - removed, Some(area), k);
        assert_eq!(solid["faces"], 10);
        let names = names_of(&solid);
        for index in 0..4 {
            assert!(
                names.contains(&format!("F3:fillet(0,{index})")),
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
            reason.contains("fillet the edges it continues in as well"),
            "{reason}"
        );
    }
}

// ----------------------------------------------------------- what is refused

#[test]
fn a_radius_too_large_names_the_edge_the_face_and_what_fits() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        block(&mut s, k);
        // The top is 3 deep from its front edge; the front 2 high.
        let reason = failed(
            &mut s,
            "feature.fillet",
            json!({"edges": [[TOP, FRONT]], "radius": 2.5 * k, "name": "Round"}),
        );
        assert!(
            reason.starts_with("radius: too large for edges.0 (between F1:face(1) and F1:face(2))"),
            "{reason}"
        );
        assert!(
            reason.contains("touches the face F1:face(2) leaves that face"),
            "{reason}"
        );
        assert!(
            reason.contains(&format!(
                "the largest radius that fits there is below {} mm",
                2.0 * k
            )),
            "{reason}"
        );
        // Never a body: the block is the document's one body still.
        let document = inspect(&mut s);
        let bodies = document["bodies"].as_array().unwrap();
        assert_eq!(bodies.len(), 1);
        assert_eq!(bodies[0]["name"], "Base");
        // Two fillets that would meet in the middle of the top: of one
        // size they fit up to half its depth each (until 2026-10-15 the
        // hint held the other at 1.6 and said 1.4).
        let reason = failed(
            &mut s,
            "feature.edit",
            json!({"feature": "Round", "radius": 1.6 * k,
                   "edges": [[TOP, FRONT], [TOP, BACK]]}),
        );
        assert!(reason.contains("too large for edges."), "{reason}");
        assert!(reason.contains("the face F1:face(1)"), "{reason}");
        assert!(
            reason.contains(&format!("below {} mm", 1.5 * k)),
            "{reason}"
        );
        // A radius that fits repairs it.
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Round", "radius": 1.0 * k}),
        );
        assert_solid(&mut s, "Round", 24.0 - 2.0 * SPANDREL * 4.0, None, k);
    }
}

#[test]
fn bad_parameters_are_errors_and_what_is_not_built_says_its_kind() {
    let mut s = session();
    let k = 1.0;
    // No document.
    refused(
        &mut s,
        "feature.fillet",
        json!({"edges": [[TOP, FRONT]], "radius": 1}),
    );
    new_document(&mut s, k);
    block(&mut s, k);
    for params in [
        json!({}),
        json!({"radius": 1}),
        json!({"edges": [[TOP, FRONT]]}),
        json!({"edges": [], "radius": 1}),
        json!({"edges": "all", "radius": 1}),
        json!({"edges": [TOP, FRONT], "radius": 1}),
        json!({"edges": [[TOP]], "radius": 1}),
        json!({"edges": [[TOP, FRONT, LEFT]], "radius": 1}),
        json!({"edges": [[TOP, 3]], "radius": 1}),
        json!({"edges": [[TOP, "roof"]], "radius": 1}),
        json!({"edges": [[TOP, "F1:edge(0)"]], "radius": 1}),
        json!({"edges": [[TOP, TOP]], "radius": 1}),
        json!({"edges": [[TOP, FRONT], [FRONT, TOP]], "radius": 1}),
        json!({"edges": [[TOP, "F9:face(0)"]], "radius": 1}),
        json!({"edges": [[TOP, FRONT]], "radius": [1]}),
        json!({"edges": [[TOP, FRONT]], "radius": 1, "chain": "yes"}),
        json!({"edges": [[TOP, FRONT]], "radius": 1, "distance": 2}),
    ] {
        refused(&mut s, "feature.fillet", params);
    }
    // Accepted, and failed with the parameter.
    for (params, parameter) in [
        (json!({"radius": 0}), "radius"),
        (json!({"radius": -1}), "radius"),
        (json!({"radius": "1 deg"}), "radius"),
        (json!({"radius": "nothing"}), "radius"),
        (json!({"radius": 1e-9}), "radius"),
    ] {
        let mut params = params;
        params["edges"] = json!([[TOP, FRONT]]);
        params["name"] = json!("Round");
        let reason = failed(&mut s, "feature.fillet", params.clone());
        assert!(reason.starts_with(parameter), "{params}: {reason}");
        run(&mut s, "feature.delete", json!({"feature": "Round"}));
    }
    // Faces that are there and do not meet.
    let reason = failed(
        &mut s,
        "feature.fillet",
        json!({"edges": [[TOP, BOTTOM]], "radius": 1, "name": "Round"}),
    );
    assert!(
        reason.starts_with("edges.0: the faces F1:face(1) and F1:face(0) of `Base` do not meet"),
        "{reason}"
    );
    // A face that is not there.
    let reason = failed(
        &mut s,
        "feature.edit",
        json!({"feature": "Round", "edges": [[TOP, "F1:face(9)"]]}),
    );
    assert!(
        reason.starts_with("edges.0: `Base` no longer has a face F1:face(9); its faces are: "),
        "{reason}"
    );
    run(&mut s, "feature.delete", json!({"feature": "Round"}));
    // An elliptic edge: the mouth of a slanted hole.
    built(
        &mut s,
        "feature.cylinder",
        json!({"name": "Drill", "position": [2.0, 1.5, -1.0], "axis": [0.3, 0.0, 1.0],
               "radius": 0.5, "height": 5}),
    );
    built(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Base", "tool": "Drill", "name": "Cut"}),
    );
    let listed = run(&mut s, "document.edges", json!({"feature": "Cut"}));
    let ellipse = listed["edges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["curve"] == "ellipse")
        .unwrap_or_else(|| panic!("no elliptic edge in {listed}"));
    // Built since 2026-10-14: a ring of spline surface
    // (`blend_freeform.rs` has its oracles).
    built(
        &mut s,
        "feature.fillet",
        json!({"edges": [ellipse["faces"]], "radius": 0.1, "name": "Round"}),
    );
    let check = run(&mut s, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
    // A fillet face is no place for a sketch: refused, with the reason.
    run(&mut s, "feature.delete", json!({"feature": "Round"}));
    run(&mut s, "feature.delete", json!({"feature": "Cut"}));
    built(
        &mut s,
        "feature.fillet",
        json!({"edges": [[TOP, FRONT]], "radius": 0.5, "name": "Round"}),
    );
    let fillet = feature(&mut s, "Round")["id"].as_str().unwrap().to_string();
    let result = s.execute(
        "sketch.create",
        &json!({"name": "Mark", "face": format!("{fillet}:fillet(0)")}),
    );
    match result {
        Ok(value) => assert_eq!(value["status"], "failed", "{value}"),
        Err(error) => assert!(!error.message().contains("internal error"), "{error}"),
    }
}

// --------------------------------------------------------- names and history

#[test]
fn the_fillet_follows_an_edge_that_moves_and_fails_by_name_when_it_goes() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        for (name, value) in [("width", 4.0), ("rounding", 0.5), ("reach", 1.0)] {
            run(
                &mut s,
                "variable.set",
                json!({"name": name, "expr": value * k}),
            );
        }
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
        // wall is the cutter's +y face. It ends against the notch's two
        // side walls, which stand across it.
        let top = "F3:copy(0)/F1:face(1)";
        let wall = "F3:copy(1)/F2:face(3)";
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": [[top, wall]], "radius": "rounding", "name": "Round"}),
        );
        // A sketch on the top, the face next to the fillet, and a boss
        // from it: something made from a face the fillet trimmed.
        built(
            &mut s,
            "sketch.create",
            json!({"name": "Mark", "face": "F4:copy(0)/F3:copy(0)/F1:face(1)"}),
        );
        let notched = |width: f64, reach: f64| width * 3.0 * 2.0 - reach * 2.0;
        // The edge is concave: the wall stands on... no: the notch is cut
        // out, so its back wall and the top meet in an outside edge.
        let r = 0.5;
        assert_solid(
            &mut s,
            "Round",
            notched(4.0, 1.0) - SPANDREL * r * r,
            None,
            k,
        );
        // Upstream edits that move the edge: the block grows, the notch
        // deepens, the radius follows a variable.
        for (name, value) in [("width", 6.0), ("reach", 1.5), ("rounding", 0.25)] {
            run(
                &mut s,
                "variable.set",
                json!({"name": name, "expr": value * k}),
            );
        }
        assert_eq!(feature(&mut s, "Round")["status"], "ok");
        assert_eq!(feature(&mut s, "Mark")["status"], "ok");
        let r = 0.25;
        assert_solid(
            &mut s,
            "Round",
            notched(6.0, 1.5) - SPANDREL * r * r,
            None,
            k,
        );
        // An edit that removes a face: the notch goes right through, so
        // its back wall is gone. The fillet fails with the parameter, the
        // feature and the name it lost; the sketch names the fillet; the
        // notched body is its own again.
        run(
            &mut s,
            "variable.set",
            json!({"name": "reach", "expr": 4.0 * k}),
        );
        let round = feature(&mut s, "Round");
        assert_eq!(round["status"], "failed", "{round}");
        let reason = round["error"].as_str().unwrap();
        assert!(
            reason.starts_with(
                "edges.0: `Notched` no longer has a face F3:copy(1)/F2:face(3); its faces are: "
            ),
            "{reason}"
        );
        let mark = feature(&mut s, "Mark");
        assert_eq!(mark["status"], "failed", "{mark}");
        assert!(
            mark["error"].as_str().unwrap().contains("`Round`"),
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
        assert_eq!(feature(&mut s, "Round")["status"], "ok");
        assert_eq!(feature(&mut s, "Mark")["status"], "ok");
        // And so does pointing the fillet at another edge, with the
        // notch through: the edge between the top and the notch's side.
        run(
            &mut s,
            "variable.set",
            json!({"name": "reach", "expr": 4.0 * k}),
        );
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Round", "edges": [[top, "F3:copy(1)/F2:face(4)"]]}),
        );
        assert_eq!(feature(&mut s, "Mark")["status"], "ok");
        let split = 6.0 * 3.0 * 2.0 - 3.0 * 2.0;
        assert_solid(&mut s, "Round", split - SPANDREL * r * r * 3.0, None, k);
        // The repaired document is the one built that way from the start.
        let repaired = inspect(&mut s)["bodies"].clone();
        let mut fresh = session();
        new_document(&mut fresh, k);
        for (name, value) in [("width", 6.0), ("rounding", 0.25), ("reach", 4.0)] {
            run(
                &mut fresh,
                "variable.set",
                json!({"name": name, "expr": value * k}),
            );
        }
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
            "feature.fillet",
            json!({"edges": [[top, "F3:copy(1)/F2:face(4)"]], "radius": "rounding", "name": "Round"}),
        );
        built(
            &mut fresh,
            "sketch.create",
            json!({"name": "Mark", "face": "F4:copy(0)/F3:copy(0)/F1:face(1)"}),
        );
        assert_eq!(inspect(&mut fresh)["bodies"], repaired);
    }
}

#[test]
fn a_boss_on_the_face_next_to_a_fillet_and_blends_on_neighbouring_edges() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        block(&mut s, k);
        let r = 0.5;
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": [[TOP, FRONT]], "radius": r * k, "name": "Round"}),
        );
        let rounded = 24.0 - SPANDREL * r * r * 4.0;
        // A sketch on the top, which the fillet trimmed: it lies where it
        // lay on the block (the face keeps its frame), and a boss from
        // it joins the body.
        built(
            &mut s,
            "sketch.create",
            json!({"name": "Mark", "face": "F2:copy(0)/F1:face(1)"}),
        );
        let (radius, tall) = (0.3, 0.5);
        run(
            &mut s,
            "sketch.circle",
            json!({"sketch": "Mark", "center": [2.0 * k, 2.0 * k], "radius": radius * k}),
        );
        built(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Mark", "distance": tall * k, "join": "Round", "name": "Bossed"}),
        );
        let solid = assert_solid(
            &mut s,
            "Bossed",
            rounded + PI * radius * radius * tall,
            None,
            k,
        );
        // The boss stands at (2, 2) of the top, as on the block.
        let total = rounded + PI * radius * radius * tall;
        let strip = SPANDREL * r * r * 4.0;
        let x = (24.0 * 2.0 - strip * 2.0 + PI * radius * radius * tall * 2.0) / total;
        assert!(
            (point(&solid["centroid"])[0] - x * k).abs() <= 1e-9 * k,
            "{solid}"
        );
        for _ in 0..3 {
            run(&mut s, "edit.undo", json!({}));
        }
        // A chamfer after the fillet, on the neighbouring edge. The edge
        // between the top and the left now goes on smoothly in the arc
        // where the fillet meets the left, and that in the upright edge
        // between the front and the left: one chain around the rounded
        // corner of the left face. One edge of it alone would stop in the
        // middle of the chain, which is refused with the way out.
        let d = 0.3;
        let neighbour = json!([["F2:copy(0)/F1:face(1)", "F2:copy(0)/F1:face(4)"]]);
        let reason = failed(
            &mut s,
            "feature.chamfer",
            json!({"edges": neighbour, "distance": d * k, "name": "Bevel"}),
        );
        assert!(reason.contains("does not end cleanly"), "{reason}");
        assert!(
            reason.contains("chamfer the edges it continues in as well"),
            "{reason}"
        );
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Bevel", "chain": true}),
        );
        // Two prisms and a quarter of a ring about the fillet's axis,
        // whose section has its centroid a third of d inside the rim.
        let around = |r: f64, section: f64, at: f64| {
            section * ((3.0 - r) + (2.0 - r)) + PI / 2.0 * at * section
        };
        let bevelled = rounded - around(r, d * d / 2.0, r - d / 3.0);
        let solid = assert_solid(&mut s, "Bevel", bevelled, None, k);
        assert_eq!(solid["faces"], 10);
        let names = names_of(&solid);
        for index in 0..3 {
            assert!(
                names.contains(&format!("F3:chamfer(0,{index})")),
                "{names:?}"
            );
        }
        assert!(
            names.contains(&"F3:copy(0)/F2:fillet(0)".to_string()),
            "{names:?}"
        );
        // The fillet grows: the chamfer follows it round.
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Round", "radius": 0.8 * k}),
        );
        assert_eq!(feature(&mut s, "Bevel")["status"], "ok");
        let grown = 24.0 - SPANDREL * 0.8 * 0.8 * 4.0 - around(0.8, d * d / 2.0, 0.8 - d / 3.0);
        assert_solid(&mut s, "Bevel", grown, None, k);
        // A fillet after the fillet, the same way round: two cylinders
        // and a quarter of a torus between them.
        run(&mut s, "feature.delete", json!({"feature": "Bevel"}));
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Round", "radius": r * k}),
        );
        let small = 0.2;
        let reason = failed(
            &mut s,
            "feature.fillet",
            json!({"edges": neighbour, "radius": small * k, "name": "Second"}),
        );
        assert!(
            reason.contains("fillet the edges it continues in as well"),
            "{reason}"
        );
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Second", "chain": true}),
        );
        let (turned, _) = ring([r, 0.0], [-1.0, 0.0], [0.0, 1.0], small);
        let twice = rounded - SPANDREL * small * small * ((3.0 - r) + (2.0 - r)) - turned / 4.0;
        let solid = assert_solid(&mut s, "Second", twice, None, k);
        assert_eq!(solid["faces"], 10);

        // The other way round: a chamfer first, then a fillet on the
        // neighbouring edge, which ends in corners of the part and runs
        // out across the chamfer's end. (The chamfer is the wider of the
        // two here: the estimate of what fits takes the chamfer's slanted
        // end on the front as the end of the front, and refuses a radius
        // above the chamfer's distance as too large, which it is not.)
        let wide = 0.6;
        let mut s = session();
        new_document(&mut s, k);
        block(&mut s, k);
        built(
            &mut s,
            "feature.chamfer",
            json!({"edges": [[TOP, LEFT]], "distance": wide * k, "name": "Bevel"}),
        );
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": [["F2:copy(0)/F1:face(1)", "F2:copy(0)/F1:face(2)"]],
                   "radius": r * k, "name": "Round"}),
        );
        // Both cuts are what their tools take of the block; what both
        // take is counted from their definitions.
        let both = rounded - wide * wide / 2.0 * 3.0 + shared_by_round_and_bevel(r, wide);
        let solid = body(&mut s, "Round");
        let volume = solid["volume"].as_f64().unwrap() / (k * k * k);
        assert!(
            (volume - both).abs() < 1e-7 * both,
            "{volume} is not {both}"
        );
        let check = run(&mut s, "document.check", json!({"deep": true}));
        assert_eq!(check["ok"], true, "{check}");
        let names = names_of(&solid);
        assert!(names.contains(&"F3:fillet(0)".to_string()), "{names:?}");
        assert!(
            names.contains(&"F3:copy(0)/F2:chamfer(0)".to_string()),
            "{names:?}"
        );
    }
}

/// How wide a round of radius `r` is at the depth `y` below the face it
/// starts from: what it takes of the corner there.
fn round_width(r: f64, y: f64) -> f64 {
    if y < r {
        r - (r * r - (r - y) * (r - y)).sqrt()
    } else {
        0.0
    }
}

/// The volume a round of radius `r` and a chamfer of distance `d` on two
/// edges that meet at a square corner both take: along the depth below
/// the face the two edges share, the round's width times the chamfer's
/// (midpoint rule; nothing of the kernel in it).
fn shared_by_round_and_bevel(r: f64, d: f64) -> f64 {
    let n = 2_000_000;
    let top = r.min(d);
    let h = top / f64::from(n);
    (0..n)
        .map(|i| {
            let y = (f64::from(i) + 0.5) * h;
            round_width(r, y) * (d - y).max(0.0) * h
        })
        .sum()
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
        "feature.fillet",
        json!({"edges": [mouth, foot], "radius": 0.2, "name": "Rims"}),
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
        "feature.fillet",
        json!({"edges": [on(&long), on(&short), on(&upright)], "radius": "3 mm / 10", "name": "Corner"}),
    );
    let before = inspect(&mut s);
    assert_eq!(before["bodies"].as_array().unwrap().len(), 1);
    assert!(
        names_of(&before["bodies"][0]).contains(&"F9:filletCorner(0,1,2)".to_string()),
        "{before}"
    );
    // Undo and redo are exact.
    run(&mut s, "edit.undo", json!({}));
    assert_ne!(inspect(&mut s)["bodies"], before["bodies"]);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(inspect(&mut s)["bodies"], before["bodies"]);
    run(&mut s, "file.save", json!({"path": "fillets.3dcraft"}));
    run(&mut s, "file.close", json!({}));
    run(&mut s, "file.open", json!({"path": "fillets.3dcraft"}));
    let after = inspect(&mut s);
    assert_eq!(after["bodies"], before["bodies"]);
    assert_eq!(after["features"], before["features"]);
    let check = run(&mut s, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
}

/// The back top edge of a block rounded by 1.8, then the front top edge by
/// 1.4: the top's flat strip left between the front and the round's
/// tangent line (1.2 deep) is narrower than the radius, so the second
/// fillet's border is carried across that line onto the round, the top
/// goes, and the fillet rolls against the front and the round. The
/// volume against the section's area (Green's theorem over the front, the
/// top, the round and the ball); the names of what is left follow the
/// faces, every edge between two faces of the body.
#[test]
fn a_fillet_whose_border_crosses_onto_an_earlier_round() {
    let (depth, height, length, big, r) = (3.0_f64, 2.0_f64, 4.0_f64, 1.8_f64, 1.4_f64);
    let (cy, cz) = (depth - big, height - big);
    let mz = cz + ((big - r).powi(2) - (r - cy).powi(2)).sqrt();
    let (dy, dz) = (r - cy, mz - cz);
    let len = dy.hypot(dz);
    let touch = [cy + dy / len * big, cz + dz / len * big];
    let angle = |c: [f64; 2], p: [f64; 2]| (p[1] - c[1]).atan2(p[0] - c[0]);
    let line = |p: [f64; 2], q: [f64; 2]| p[0] * q[1] - p[1] * q[0];
    let arc = |c: [f64; 2], rad: f64, a: f64, b: f64| {
        rad * rad * (b - a) + rad * (c[0] * (b.sin() - a.sin()) - c[1] * (b.cos() - a.cos()))
    };
    let twice = line([0.0, mz], [0.0, height])
        + line([0.0, height], [cy, height])
        + arc([cy, cz], big, PI / 2.0, angle([cy, cz], touch))
        + arc([r, mz], r, angle([r, mz], touch), PI);
    let taken = (0.5 * twice).abs();
    // What the first round takes: the block's corner less the quarter
    // disc, along the length.
    let rounded = depth * height * length - (1.0 - PI / 4.0) * big * big * length;
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        block(&mut s, k);
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": [[TOP, BACK]], "radius": big * k, "name": "Round"}),
        );
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": [[TOP, FRONT]], "radius": r * k, "name": "Over"}),
        );
        let solid = assert_solid(&mut s, "Over", rounded - taken * length, None, k);
        // The top is gone: the round's face, the new fillet and four
        // faces of the block are left beside the bottom.
        let names = names_sorted(&solid);
        assert_eq!(
            names,
            [
                "F3:copy(0)/F1:face(0)",
                "F3:copy(0)/F1:face(2)",
                "F3:copy(0)/F1:face(3)",
                "F3:copy(0)/F1:face(4)",
                "F3:copy(0)/F1:face(5)",
                "F3:copy(0)/F2:fillet(0)",
                "F3:fillet(0)",
            ]
        );
        let listed = run(&mut s, "document.edges", json!({"feature": "Over"}));
        let edges = listed["edges"].as_array().unwrap();
        let mut seen = std::collections::BTreeSet::new();
        for edge in edges {
            assert!(seen.insert(edge["name"].to_string()), "{edge}");
            for face in edge["faces"].as_array().unwrap() {
                assert!(
                    names.contains(&face.as_str().unwrap().to_string()),
                    "{edge}"
                );
            }
        }
        // The new fillet meets the front and the round, not the top.
        let between = run(
            &mut s,
            "document.edges",
            json!({"feature": "Over", "between": ["F3:fillet(0)", "F3:copy(0)/F2:fillet(0)"]}),
        );
        assert_eq!(between["edges"].as_array().unwrap().len(), 1, "{between}");
    }
}
