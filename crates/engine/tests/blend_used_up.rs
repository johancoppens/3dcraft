//! Blends that use a face up exactly, through the commands: the four
//! designs regeneration's self-check refused before 2026-11-13 ("cannot be
//! meshed to see whether it passes through itself … its loops do not
//! bound a region") are built, checked deep (the self-intersection check
//! on a fine mesh included) and measured against closed forms worked out
//! by hand, at three scales, at the origin and far from it. The kernel's
//! side, with tilted frames and a freeform case:
//! `crates/kernel/tests/blend_used_up.rs`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod blend_common;
mod common;

use blend_common::{BACK, BOTTOM, FRONT, LEFT, RIGHT, SCALES, TOP};
use common::checked::{assert_solid, built, failed, names_of, new_document, run, session};
use serde_json::{Value, json};
use std::f64::consts::PI;

/// Where every design is placed: at the origin, and moved far from it.
const PLACES: [[f64; 3]; 2] = [[0.0, 0.0, 0.0], [1000.0, -700.0, 300.0]];

fn at(place: [f64; 3], k: f64) -> Value {
    json!([place[0] * k, place[1] * k, place[2] * k])
}

/// The faces, edges and vertices of a body.
fn counts(body: &Value) -> (u64, u64, u64) {
    (
        body["faces"].as_u64().unwrap(),
        body["edges"].as_u64().unwrap(),
        body["vertices"].as_u64().unwrap(),
    )
}

#[test]
fn the_uprights_of_a_box_rounded_with_half_its_width() {
    // 14 × 6 × 10, the four uprights rounded 3: each 6 wide end is used up
    // and its two rounds meet in a half cylinder. The section is a
    // stadium: 14 × 6 less four spandrels (1 − π/4) 9, its perimeter
    // 2 × 8 + 6π; the area is twice the section plus the perimeter times
    // 10. The faces: top, bottom, the two long sides and the four rounds.
    let section = 84.0 - 4.0 * (1.0 - PI / 4.0) * 9.0;
    let perimeter = 16.0 + 6.0 * PI;
    for k in SCALES {
        for place in PLACES {
            let mut s = session();
            new_document(&mut s, k);
            built(
                &mut s,
                "feature.block",
                json!({"size": [14.0 * k, 6.0 * k, 10.0 * k], "position": at(place, k)}),
            );
            built(
                &mut s,
                "feature.fillet",
                json!({
                    "edges": [[FRONT, LEFT], [FRONT, RIGHT], [BACK, RIGHT], [BACK, LEFT]],
                    "radius": 3.0 * k,
                    "chain": true,
                    "name": "Round",
                }),
            );
            let solid = assert_solid(
                &mut s,
                "Round",
                section * 10.0,
                Some(2.0 * section + perimeter * 10.0),
                k,
            );
            assert_eq!(counts(&solid), (8, 18, 12), "scale {k}, {place:?}");
            let names = names_of(&solid);
            for end in ["F2:copy(0)/F1:face(4)", "F2:copy(0)/F1:face(5)"] {
                assert!(!names.iter().any(|n| n == end), "{end} is left: {names:?}");
            }
            assert_eq!(names.iter().filter(|n| n.contains("fillet")).count(), 4);
        }
    }
}

#[test]
fn both_uprights_of_one_end_rounded_with_half_its_width() {
    // 80 × 20 × 20, the uprights at x = 80 rounded 10: the end is used up.
    // The section is 80 × 20 less two spandrels (1 − π/4) 100, its
    // perimeter 2 × 70 + 20 + 10π. The faces: the five planes left and
    // the two rounds.
    let section = 1600.0 - 2.0 * (1.0 - PI / 4.0) * 100.0;
    let perimeter = 160.0 + 10.0 * PI;
    for k in SCALES {
        for place in PLACES {
            let mut s = session();
            new_document(&mut s, k);
            built(
                &mut s,
                "feature.block",
                json!({"size": [80.0 * k, 20.0 * k, 20.0 * k], "position": at(place, k)}),
            );
            built(
                &mut s,
                "feature.fillet",
                json!({"edges": [[FRONT, RIGHT], [BACK, RIGHT]], "radius": 10.0 * k, "chain": true, "name": "Round"}),
            );
            let solid = assert_solid(
                &mut s,
                "Round",
                section * 20.0,
                Some(2.0 * section + perimeter * 20.0),
                k,
            );
            assert_eq!(counts(&solid), (7, 15, 10), "scale {k}, {place:?}");
            assert!(
                !names_of(&solid)
                    .iter()
                    .any(|n| n == "F2:copy(0)/F1:face(5)")
            );
        }
    }
}

#[test]
fn both_edges_of_a_face_chamfered_with_half_its_height() {
    // 40 × 20 × 5, the top and bottom edges of the front (5 tall)
    // chamfered 2.5: they meet along its middle line and use it up. Each
    // takes 2.5² / 2 × 40 = 125. The section across x: 100 − 6.25, its
    // perimeter 17.5 + 17.5 + 5 + 2 × 2.5 √2. The faces: top, bottom,
    // back, the two ends and the two chamfers.
    let section = 100.0 - 6.25;
    let perimeter = 40.0 + 5.0 * 2.0_f64.sqrt();
    for k in SCALES {
        for place in PLACES {
            let mut s = session();
            new_document(&mut s, k);
            built(
                &mut s,
                "feature.block",
                json!({"size": [40.0 * k, 20.0 * k, 5.0 * k], "position": at(place, k)}),
            );
            built(
                &mut s,
                "feature.chamfer",
                json!({"edges": [[TOP, FRONT], [BOTTOM, FRONT]], "distance": 2.5 * k, "chain": true, "name": "Bevel"}),
            );
            let solid = assert_solid(
                &mut s,
                "Bevel",
                3750.0,
                Some(2.0 * section + perimeter * 40.0),
                k,
            );
            assert_eq!(counts(&solid), (7, 15, 10), "scale {k}, {place:?}");
            assert!(
                !names_of(&solid)
                    .iter()
                    .any(|n| n == "F2:copy(0)/F1:face(2)")
            );
        }
    }
}

#[test]
fn the_inside_bend_of_an_l_rounded_as_long_as_its_inside_faces() {
    // An L of a foot 40 × 20 × 5 and an upright 5 × 20 × 40, joined
    // (7500). Its inside faces run 35 from the bend; a round of 35 in the
    // bend uses both up and meets the legs' ends square. It adds
    // (1 − π/4) 35² along the 20. The section across y: 375 + (1 − π/4)
    // 1225, its perimeter 40 + 5 + 35π/2 + 5 + 40. The faces: the two
    // sides, the bottom, the back of the upright, the foot's end, the
    // upright's top and the round.
    let section = 375.0 + (1.0 - PI / 4.0) * 1225.0;
    let perimeter = 90.0 + 35.0 * PI / 2.0;
    for k in SCALES {
        for place in PLACES {
            let mut s = session();
            new_document(&mut s, k);
            built(
                &mut s,
                "feature.block",
                json!({"size": [40.0 * k, 20.0 * k, 5.0 * k], "position": at(place, k)}),
            );
            built(
                &mut s,
                "feature.block",
                json!({"size": [5.0 * k, 20.0 * k, 40.0 * k], "position": at(place, k)}),
            );
            built(
                &mut s,
                "feature.boolean",
                json!({"op": "union", "target": "F1", "tool": "F2"}),
            );
            built(
                &mut s,
                "feature.fillet",
                json!({
                    "edges": [["F3:copy(0)/F1:face(1)", "F3:copy(1)/F2:face(5)"]],
                    "radius": 35.0 * k,
                    "chain": true,
                    "name": "Bend",
                }),
            );
            let solid = assert_solid(
                &mut s,
                "Bend",
                section * 20.0,
                Some(2.0 * section + perimeter * 20.0),
                k,
            );
            assert_eq!(counts(&solid).0, 7, "scale {k}, {place:?}");
            let names = names_of(&solid);
            for inside in ["F1:face(1)", "F2:face(5)"] {
                assert!(
                    !names.iter().any(|n| n.ends_with(inside)),
                    "{inside} is left: {names:?}"
                );
            }
        }
    }
}

#[test]
fn later_features_on_a_used_up_face_or_a_merged_edge_fail_by_name() {
    // The box of the first test, its two ends used up. A later feature
    // that names an end, by the box's name or by the round's copy of it,
    // fails and names the face, with the faces there are; one that names
    // the edge where the two rounds of an end now meet is refused for what
    // that edge is (the rounds are tangent there). Nothing is retargeted.
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        built(
            &mut s,
            "feature.block",
            json!({"size": [14.0 * k, 6.0 * k, 10.0 * k]}),
        );
        built(
            &mut s,
            "feature.fillet",
            json!({
                "edges": [[FRONT, LEFT], [FRONT, RIGHT], [BACK, RIGHT], [BACK, LEFT]],
                "radius": 3.0 * k,
                "chain": true,
                "name": "Round",
            }),
        );
        let gone = failed(
            &mut s,
            "feature.chamfer",
            json!({"edges": [[TOP, LEFT]], "distance": 0.5 * k, "name": "OnGone"}),
        );
        assert!(gone.contains("F1:face(4) is gone"), "{gone}");
        assert!(gone.contains("F2:fillet(0)"), "{gone}");
        let merged = failed(
            &mut s,
            "feature.fillet",
            json!({"edges": [["F2:fillet(0)", "F2:fillet(3)"]], "radius": 0.5 * k, "name": "OnMerged"}),
        );
        assert!(
            merged.contains("between F2:fillet(0) and F2:fillet(3)"),
            "{merged}"
        );
        assert!(merged.contains("tangent"), "{merged}");
        let sketch = failed(
            &mut s,
            "sketch.create",
            json!({"face": "F2:copy(0)/F1:face(4)", "name": "OnTheEnd"}),
        );
        assert!(
            sketch.contains("no longer has a face F2:copy(0)/F1:face(4)"),
            "{sketch}"
        );
        // The round itself still stands.
        let check = run(&mut s, "document.check", json!({"feature": "Round"}));
        assert_eq!(check["status"], "ok", "{check}");
    }
}
