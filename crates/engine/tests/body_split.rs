//! `body.split`: a body of several lumps becomes one body per lump, as
//! split features in the history (docs/decisions.md 2026-10-18; the
//! boolean checklist's item 5). Each split is said by a face of the
//! source's body, never by the order of the shells, so an upstream edit
//! moves a split with its lump or fails it, and never hands it another
//! lump. Held to closed-form volumes and areas per piece that sum to the
//! whole, cavities that stay with their lump, three scales, bodies turned
//! off-axis, analytic and freeform faces, names followed through the
//! splits, save and reopen, undo, and the refusals.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{
    assert_close, assert_solid, body, built, failure, inspect, names_of, new_document, refused,
    run, session, session_in, workdir,
};
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::f64::consts::PI;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn block(s: &mut Session, name: &str, size: [f64; 3], at: [f64; 3], k: f64) {
    built(
        s,
        "feature.block",
        json!({"size": size.map(|x| x * k), "position": at.map(|x| x * k), "name": name}),
    );
}

fn subtract(s: &mut Session, target: &str, tool: &str, name: &str) {
    built(
        s,
        "feature.boolean",
        json!({"op": "subtract", "target": target, "tool": tool, "name": name}),
    );
}

/// A block of 10 (F1) with a slab (F2) through its middle cut out (F3):
/// a lump below (0..4) and one above (6..10).
fn cut_in_two(s: &mut Session, k: f64) {
    block(s, "A", [10.0, 10.0, 10.0], [0.0; 3], k);
    block(s, "Slab", [12.0, 12.0, 2.0], [-1.0, -1.0, 4.0], k);
    subtract(s, "A", "Slab", "Cut");
}

/// The names of the bodies the document has, in history order.
fn bodies(s: &mut Session) -> Vec<String> {
    inspect(s)["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["name"].as_str().unwrap().to_string())
        .collect()
}

/// The split's pieces as the command reports them: `(name, face, number)`.
fn pieces(result: &Value) -> Vec<(String, String, Option<u64>)> {
    result["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| {
            assert_eq!(b["status"], "ok", "{b}");
            (
                b["name"].as_str().unwrap().to_string(),
                b["face"].as_str().unwrap().to_string(),
                b["number"].as_u64(),
            )
        })
        .collect()
}

fn centroid(s: &mut Session, name: &str) -> [f64; 3] {
    let found = run(s, "document.measure", json!({"feature": name}));
    let c = found["centroid"].as_array().unwrap();
    [
        c[0].as_f64().unwrap(),
        c[1].as_f64().unwrap(),
        c[2].as_f64().unwrap(),
    ]
}

#[test]
fn a_cut_that_fell_apart_splits_into_one_body_per_lump() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        cut_in_two(&mut s, k);
        let cut = assert_solid(&mut s, "Cut", 800.0, Some(720.0), k);
        assert_eq!(cut["shells"], 2, "{cut}");
        let result = run(&mut s, "body.split", json!({"source": "Cut"}));
        // Each lump is said by the least name among the faces only it
        // has: the bottom of A for the lower lump, the top for the upper.
        assert_eq!(
            pieces(&result),
            [
                (
                    "Split1".to_string(),
                    "F3:copy(0)/F1:face(0)".to_string(),
                    None
                ),
                (
                    "Split2".to_string(),
                    "F3:copy(0)/F1:face(1)".to_string(),
                    None
                ),
            ],
            "{result}"
        );
        // The source's body went into the splits; the pieces have the
        // volumes and areas of 10 × 10 × 4 blocks and sum to the whole.
        assert_eq!(bodies(&mut s), ["Split1", "Split2"]);
        let lower = assert_solid(&mut s, "Split1", 400.0, Some(360.0), k);
        let upper = assert_solid(&mut s, "Split2", 400.0, Some(360.0), k);
        assert_eq!(lower["shells"], 1);
        assert_eq!(upper["shells"], 1);
        assert_eq!(lower["faces"], 6);
        assert!(centroid(&mut s, "Split1")[2] < centroid(&mut s, "Split2")[2]);
        // The pieces of a face keep its name with the split's prefix, one
        // link for the run of instance 0; the cut faces are the slab's.
        let mut faces = names_of(&lower);
        faces.sort();
        assert_eq!(
            faces,
            [
                "F4:copy(0)/F1:face(0)",
                "F4:copy(0)/F1:face(2)",
                "F4:copy(0)/F1:face(3)",
                "F4:copy(0)/F1:face(4)",
                "F4:copy(0)/F1:face(5)",
                "F4:copy(0)/F3:copy(1)/F2:face(0)",
            ]
        );
        assert!(names_of(&upper).contains(&"F5:copy(0)/F1:face(1)".to_string()));
        assert!(names_of(&upper).contains(&"F5:copy(0)/F3:copy(1)/F2:face(1)".to_string()));
        // One undo step takes both splits back.
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(bodies(&mut s), ["Cut"]);
        assert_solid(&mut s, "Cut", 800.0, Some(720.0), k);
        run(&mut s, "edit.redo", json!({}));
        assert_eq!(bodies(&mut s), ["Split1", "Split2"]);
    }
}

#[test]
fn a_cavity_stays_with_the_lump_that_encloses_it() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        // A block of 10 with a cavity of 4 in its lower half, cut in two
        // above the cavity: the lower lump keeps the cavity.
        block(&mut s, "A", [10.0, 10.0, 10.0], [0.0; 3], k);
        block(&mut s, "Inner", [4.0, 4.0, 4.0], [3.0, 3.0, 2.0], k);
        subtract(&mut s, "A", "Inner", "Hollow");
        block(&mut s, "Slab", [12.0, 12.0, 1.0], [-1.0, -1.0, 8.0], k);
        subtract(&mut s, "Hollow", "Slab", "Cut");
        let cut = assert_solid(&mut s, "Cut", 900.0 - 64.0, None, k);
        assert_eq!(cut["shells"], 3);
        let result = run(&mut s, "body.split", json!({"source": "Cut"}));
        let found = pieces(&result);
        assert_eq!(found.len(), 2, "{result}");
        let lower = assert_solid(
            &mut s,
            "Split1",
            800.0 - 64.0,
            Some(2.0 * 100.0 + 4.0 * 80.0 + 96.0),
            k,
        );
        assert_eq!(lower["shells"], 2, "the cavity is in the lower lump");
        let upper = assert_solid(&mut s, "Split2", 100.0, Some(200.0 + 40.0), k);
        assert_eq!(upper["shells"], 1);
        // The cavity's faces are the inner block's, in the lower lump.
        assert!(
            names_of(&lower).contains(&"F6:copy(0)/F3:copy(1)/F2:face(0)".to_string()),
            "{lower}"
        );
        let check = run(&mut s, "document.check", json!({"deep": true}));
        assert_eq!(check["ok"], true, "{check}");
    }
}

#[test]
fn pieces_of_a_ball_and_a_body_turned_off_axis() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        // A ball of radius 5 with a slab of 2 through its middle cut out:
        // two caps of height 4, each π h² (3R − h) / 3.
        built(
            &mut s,
            "feature.sphere",
            json!({"radius": 5.0 * k, "name": "Ball"}),
        );
        block(&mut s, "Slab", [12.0, 12.0, 2.0], [-6.0, -6.0, -1.0], k);
        subtract(&mut s, "Ball", "Slab", "Cut");
        let cap = PI * 16.0 * (15.0 - 4.0) / 3.0;
        let cap_area = 2.0 * PI * 5.0 * 4.0 + PI * 24.0;
        assert_solid(&mut s, "Cut", 2.0 * cap, Some(2.0 * cap_area), k);
        // Turned about a slanted line and moved: the same two caps.
        built(
            &mut s,
            "feature.transform",
            json!({"source": "Cut", "translate": [3.0 * k, -2.0 * k, 7.0 * k],
                   "angle": "37 deg", "axis": [1, 2, 3], "name": "Turned"}),
        );
        let result = run(&mut s, "body.split", json!({"source": "Turned"}));
        let found = pieces(&result);
        assert_eq!(found.len(), 2, "{result}");
        // The ball's one face has a piece in each cap; the slab's bottom
        // and top faces tell the caps apart.
        assert_eq!(found[0].1, "F4:copy(0)/F3:copy(1)/F2:face(0)");
        assert_eq!(found[1].1, "F4:copy(0)/F3:copy(1)/F2:face(1)");
        assert_solid(&mut s, "Split1", cap, Some(cap_area), k);
        assert_solid(&mut s, "Split2", cap, Some(cap_area), k);
        // Where the caps' centres lie, turned with the body: the cut was
        // symmetric about the origin, so the centres are images of each
        // other in the moved origin.
        let a = centroid(&mut s, "Split1");
        let b = centroid(&mut s, "Split2");
        for (i, shift) in [3.0, -2.0, 7.0].into_iter().enumerate() {
            assert!(
                ((a[i] + b[i]) / 2.0 - shift * k).abs() <= 1e-6 * k,
                "centres {a:?} and {b:?}"
            );
        }
    }
}

#[test]
fn a_ring_cut_in_two_crescents_has_no_face_of_its_own_and_is_numbered() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        // A ring about z, cut by a tube along y that swallows it where
        // it crosses the y axis: two crescents, left and right. Every
        // face (the ring's one wall, the tube's one wall) has a piece in
        // each crescent, so the crescents are told apart only by where
        // they lie.
        built(
            &mut s,
            "feature.torus",
            json!({"major": 10.0 * k, "minor": 2.0 * k, "name": "Ring"}),
        );
        built(
            &mut s,
            "feature.cylinder",
            json!({"radius": 3.0 * k, "height": 40.0 * k, "position": [0, -20.0 * k, 0],
                   "axis": [0, 1, 0], "name": "Tube"}),
        );
        subtract(&mut s, "Ring", "Tube", "Cut");
        let cut = body(&mut s, "Cut");
        assert_eq!(cut["shells"], 2, "{cut}");
        let whole = cut["volume"].as_f64().unwrap();
        assert!(whole < 2.0 * PI * PI * 10.0 * 4.0 * k * k * k && whole > 0.0);
        let result = run(&mut s, "body.split", json!({"source": "Cut"}));
        let found = pieces(&result);
        // Both are said by the ring's one face; the numbers tell them
        // apart, by x: the left crescent is 0, whichever the kernel
        // listed first.
        assert_eq!(found.len(), 2, "{result}");
        let mut numbers: Vec<Option<u64>> = found.iter().map(|p| p.2).collect();
        numbers.sort();
        assert_eq!(numbers, [Some(0), Some(1)], "{result}");
        assert!(
            found.iter().all(|p| p.1 == "F3:copy(0)/F1:face(0)"),
            "{result}"
        );
        let (left, right) = if found[0].2 == Some(0) {
            ("Split1", "Split2")
        } else {
            ("Split2", "Split1")
        };
        // The two are mirror images: half the whole each, as far as the
        // marched curves of the two cuts agree (within 1e-7), and they
        // sum to the whole exactly.
        let check = run(&mut s, "document.check", json!({"deep": true}));
        assert_eq!(check["ok"], true, "{check}");
        let (left, right) = (body(&mut s, left), body(&mut s, right));
        for half in [&left, &right] {
            let volume = half["volume"].as_f64().unwrap();
            assert!(
                (volume - whole / 2.0).abs() <= 1e-7 * whole,
                "{half}: half of {whole}"
            );
        }
        assert!(centroid(&mut s, left["name"].as_str().unwrap())[0] < 0.0);
        assert!(centroid(&mut s, right["name"].as_str().unwrap())[0] > 0.0);
        let summed = left["volume"].as_f64().unwrap() + right["volume"].as_f64().unwrap();
        assert!(
            (summed - whole).abs() <= 1e-7 * whole,
            "the crescents sum to {summed}, the ring is {whole}"
        );
        assert_eq!(
            left["faces"].as_u64().unwrap() + right["faces"].as_u64().unwrap(),
            cut["faces"]
        );
    }
}

#[test]
fn a_reference_to_a_piece_follows_its_lump_through_an_upstream_edit() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        run(
            &mut s,
            "variable.set",
            json!({"name": "at", "expr": format!("{} mm", 4.0 * k)}),
        );
        block(&mut s, "A", [10.0, 10.0, 10.0], [0.0; 3], k);
        built(
            &mut s,
            "feature.block",
            json!({"size": [12.0 * k, 12.0 * k, 2.0 * k], "position": [-k, -k, "at"], "name": "Slab"}),
        );
        subtract(&mut s, "A", "Slab", "Cut");
        run(
            &mut s,
            "body.split",
            json!({"source": "Cut", "names": ["Lower", "Upper"]}),
        );
        // A fillet on the upper piece, by the names the block gave its
        // top and back: the top is in the upper lump only, so it follows
        // there; the back has a piece in each lump, so the short name is
        // ambiguous and the split's own name is given.
        let round = |s: &mut Session| {
            built(
                s,
                "feature.fillet",
                json!({"edges": [["F1:face(1)", "F5:copy(0)/F1:face(3)"]], "radius": k, "name": "Round"}),
            )
        };
        round(&mut s);
        let rounded = 400.0 - (1.0 - PI / 4.0) * 10.0;
        assert_solid(&mut s, "Round", rounded, None, k);
        assert_solid(&mut s, "Lower", 400.0, Some(360.0), k);
        // The slab lower: the lower piece shrinks, the upper grows, and
        // the fillet stays on the upper piece's top back edge.
        run(
            &mut s,
            "variable.set",
            json!({"name": "at", "expr": format!("{} mm", 2.0 * k)}),
        );
        assert_solid(&mut s, "Lower", 200.0, Some(2.0 * 100.0 + 4.0 * 20.0), k);
        assert_solid(&mut s, "Round", 600.0 - (1.0 - PI / 4.0) * 10.0, None, k);
        // The block turned upside down: the face the lower split names
        // (the block's bottom) is on top now, and the split goes with it,
        // never with the place.
        run(
            &mut s,
            "feature.edit",
            json!({"feature": "A", "axis": [0, 0, -1], "position": [0, 10.0 * k, 10.0 * k]}),
        );
        let lower = body(&mut s, "Lower");
        assert!(
            names_of(&lower).contains(&"F4:copy(0)/F1:face(0)".to_string()),
            "{lower}"
        );
        assert!(centroid(&mut s, "Lower")[2] > centroid(&mut s, "Round")[2]);
        assert_solid(&mut s, "Lower", 600.0, None, k);
        assert_solid(&mut s, "Round", 200.0 - (1.0 - PI / 4.0) * 10.0, None, k);
        // A short name of a face with a piece in each lump fails and
        // lists the splits; it does not land on one of them.
        let reason = common::checked::failed(
            &mut s,
            "feature.offsetFace",
            json!({"faces": ["F1:face(2)"], "distance": k, "name": "Wider"}),
        );
        assert!(
            reason.contains("more than one face after the splits Lower, Upper")
                && reason.contains("F4:copy(0)/F1:face(2)")
                && reason.contains("F5:copy(0)/F1:face(2)"),
            "{reason}"
        );
    }
}

#[test]
fn a_name_follows_through_a_split_that_is_not_the_last_and_on() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        cut_in_two(&mut s, k);
        run(
            &mut s,
            "body.split",
            json!({"source": "Cut", "names": ["Lower", "Upper"]}),
        );
        // The lower piece (the first split, not the one the source's
        // body is marked as taken by) goes on into an offset; a name of
        // the block's bottom follows into it and on through the offset,
        // and so does the lower piece's own name of a side.
        built(
            &mut s,
            "feature.offsetFace",
            json!({"faces": ["F4:copy(0)/F1:face(0)"], "distance": k, "name": "Deeper"}),
        );
        assert_solid(&mut s, "Deeper", 500.0, Some(2.0 * 100.0 + 4.0 * 50.0), k);
        built(
            &mut s,
            "feature.chamfer",
            json!({"edges": [["F1:face(0)", "F4:copy(0)/F1:face(2)"]], "distance": k, "name": "Bevel"}),
        );
        assert_solid(&mut s, "Bevel", 500.0 - 0.5 * 10.0, None, k);
        let bevel = inspect(&mut s)["features"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["name"] == "Bevel")
            .unwrap()
            .clone();
        assert_eq!(
            bevel["followed"]["edges"],
            json!([["F6:copy(0)/F1:face(0)", "F6:copy(0)/F1:face(2)"]]),
            "{bevel}"
        );
        assert_eq!(bodies(&mut s), ["Upper", "Bevel"]);
    }
}

#[test]
fn a_split_whose_face_lands_in_several_lumps_fails_rather_than_choosing() {
    let k = 1.0;
    let mut s = session();
    new_document(&mut s, k);
    run(
        &mut s,
        "variable.set",
        json!({"name": "w", "expr": "12 mm"}),
    );
    block(&mut s, "A", [10.0, 10.0, 10.0], [0.0; 3], k);
    built(
        &mut s,
        "feature.block",
        json!({"size": ["w", "w", 2], "position": [-1, -1, 4], "name": "Slab"}),
    );
    subtract(&mut s, "A", "Slab", "Cut");
    run(&mut s, "body.split", json!({"source": "Cut"}));
    assert_solid(&mut s, "Split1", 400.0, None, k);
    // The slab narrowed so it no longer reaches across: one lump, a
    // block with a slot, which holds the bottom and the top both. The
    // first split is that lump, grown; the second is not the piece it
    // was made for and fails rather than being the same lump again.
    run(&mut s, "variable.set", json!({"name": "w", "expr": "6 mm"}));
    let slotted = 1000.0 - 5.0 * 5.0 * 2.0;
    assert_close(
        &body(&mut s, "Split1")["volume"],
        slotted,
        "the first split",
    );
    assert_eq!(bodies(&mut s), ["Split1"]);
    let reason = failure(&mut s, "Split2");
    assert!(
        reason.starts_with("face: the lump of `Cut` that holds the face F3:copy(0)/F1:face(1) is the body of `Split1` already")
            && reason.ends_with("split `Cut` again"),
        "{reason}"
    );
    // The first split suppressed: the second is the lump.
    run(
        &mut s,
        "feature.suppress",
        json!({"feature": "Split1", "suppressed": true}),
    );
    assert_close(
        &body(&mut s, "Split2")["volume"],
        slotted,
        "the second split",
    );
    assert_eq!(bodies(&mut s), ["Split2"]);
    run(
        &mut s,
        "feature.suppress",
        json!({"feature": "Split1", "suppressed": false}),
    );
    assert_eq!(bodies(&mut s), ["Split1"]);
    // The slab turned upright, cutting left from right: the bottom and
    // the top each have a piece in both lumps now, and neither split
    // says which lump it is any more.
    run(
        &mut s,
        "variable.set",
        json!({"name": "w", "expr": "12 mm"}),
    );
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Slab", "size": [2, "w", "w"], "position": [4, -1, -1]}),
    );
    for name in ["Split1", "Split2"] {
        let reason = failure(&mut s, name);
        assert!(
            reason.contains("lie in 2 lumps of `Cut` now") && reason.contains("split `Cut` again"),
            "{name}: {reason}"
        );
    }
    assert_eq!(bodies(&mut s), ["Cut"]);
    // Split again as it stands now: left and right, told by A's sides.
    run(&mut s, "feature.delete", json!({"feature": "Split1"}));
    run(&mut s, "feature.delete", json!({"feature": "Split2"}));
    let result = run(&mut s, "body.split", json!({"source": "Cut"}));
    assert_eq!(
        pieces(&result)
            .into_iter()
            .map(|(_, face, _)| face)
            .collect::<Vec<_>>(),
        ["F3:copy(0)/F1:face(4)", "F3:copy(0)/F1:face(5)"]
    );
}

#[test]
fn a_numbered_split_fails_when_the_tie_is_gone() {
    let k = 1.0;
    let mut s = session();
    new_document(&mut s, k);
    built(
        &mut s,
        "feature.torus",
        json!({"major": 10.0, "minor": 2.0, "name": "Ring"}),
    );
    run(&mut s, "variable.set", json!({"name": "r", "expr": "3 mm"}));
    built(
        &mut s,
        "feature.cylinder",
        json!({"radius": "r", "height": 40.0, "position": [0, -20.0, 0], "axis": [0, 1, 0], "name": "Tube"}),
    );
    subtract(&mut s, "Ring", "Tube", "Cut");
    run(&mut s, "body.split", json!({"source": "Cut"}));
    assert_eq!(bodies(&mut s), ["Split1", "Split2"]);
    // The tube shortened so it crosses the ring once: one lump, a ring
    // opened into a C, so there is nothing left to number.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Tube", "height": 10.0, "position": [0, 5.0, 0]}),
    );
    for name in ["Split1", "Split2"] {
        let reason = failure(&mut s, name);
        assert!(
            reason.starts_with(
                "number: the face F3:copy(0)/F1:face(0) lies in one lump of `Cut` now"
            ),
            "{name}: {reason}"
        );
    }
}

#[test]
fn a_pattern_apart_splits_into_its_instances_and_follows_them() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        run(
            &mut s,
            "variable.set",
            json!({"name": "step", "expr": format!("{} mm", 20.0 * k)}),
        );
        block(&mut s, "A", [10.0, 10.0, 10.0], [0.0; 3], k);
        built(
            &mut s,
            "feature.pattern",
            json!({"source": "A", "count": 3, "step": ["step", 0, 0], "name": "Row"}),
        );
        let result = run(&mut s, "body.split", json!({"source": "Row"}));
        assert_eq!(
            pieces(&result)
                .into_iter()
                .map(|(_, face, number)| (face, number))
                .collect::<Vec<_>>(),
            [
                ("F2:copy(0)/F1:face(0)".to_string(), None),
                ("F2:copy(1)/F1:face(0)".to_string(), None),
                ("F2:copy(2)/F1:face(0)".to_string(), None),
            ]
        );
        for name in ["Split1", "Split2", "Split3"] {
            assert_solid(&mut s, name, 1000.0, Some(600.0), k);
        }
        assert_close(
            &json!(centroid(&mut s, "Split3")[0]),
            45.0 * k,
            "the third instance",
        );
        // The row the other way: the third instance is at the far left
        // now, and its split goes with it.
        run(
            &mut s,
            "variable.set",
            json!({"name": "step", "expr": format!("{} mm", -20.0 * k)}),
        );
        assert_close(
            &json!(centroid(&mut s, "Split3")[0]),
            -35.0 * k,
            "the third instance moved",
        );
        assert_solid(&mut s, "Split3", 1000.0, Some(600.0), k);
    }
}

#[test]
fn a_split_document_saves_and_reopens_to_the_same_bodies() {
    for k in SCALES {
        let dir = workdir("body_split", &format!("round-trip-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        cut_in_two(&mut s, k);
        run(
            &mut s,
            "body.split",
            json!({"source": "Cut", "names": ["Lower", "Upper"]}),
        );
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": [["F1:face(1)", "F5:copy(0)/F1:face(3)"]], "radius": k, "name": "Round"}),
        );
        let before: Vec<Value> = ["Lower", "Round"].iter().map(|n| body(&mut s, n)).collect();
        run(
            &mut s,
            "file.save",
            json!({"path": "split.3dcraft", "overwrite": true}),
        );
        run(&mut s, "file.open", json!({"path": "split.3dcraft"}));
        assert_eq!(bodies(&mut s), ["Lower", "Round"]);
        for (name, was) in ["Lower", "Round"].iter().zip(before) {
            let now = body(&mut s, name);
            for key in [
                "faces",
                "edges",
                "vertices",
                "shells",
                "volume",
                "area",
                "faceNames",
            ] {
                assert_eq!(now[key], was[key], "{name}: {key} after a round trip");
            }
        }
        let check = run(&mut s, "document.check", json!({"deep": true}));
        assert_eq!(check["ok"], true, "{check}");
        // The export has one solid per body.
        let out = run(&mut s, "file.export", json!({"path": "out.step"}));
        assert_eq!(out["solids"], 2, "{out}");
    }
}

#[test]
fn what_body_split_refuses() {
    let mut s = session();
    new_document(&mut s, 1.0);
    block(&mut s, "A", [10.0, 10.0, 10.0], [0.0; 3], 1.0);
    // One lump: nothing to split.
    let reason = refused(&mut s, "body.split", json!({"source": "A"}));
    assert!(
        reason.contains("`A` is one lump; there is nothing to split"),
        "{reason}"
    );
    // A sheet has no lumps.
    built(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "name": "Profile"}),
    );
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Profile", "points": [[0, 0], [4, 0], [4, 4], [0, 4]], "closed": true}),
    );
    built(
        &mut s,
        "feature.fill",
        json!({"sketch": "Profile", "name": "Sheet"}),
    );
    let reason = refused(&mut s, "body.split", json!({"source": "Sheet"}));
    assert!(reason.contains("a sheet has no lumps"), "{reason}");
    // No body at all; a body that went into a later feature.
    let reason = refused(&mut s, "body.split", json!({"source": "Profile"}));
    assert!(reason.contains("`Profile` has no body"), "{reason}");
    block(&mut s, "Slab", [12.0, 12.0, 2.0], [-1.0, -1.0, 4.0], 1.0);
    subtract(&mut s, "A", "Slab", "Cut");
    let reason = refused(&mut s, "body.split", json!({"source": "A"}));
    assert!(
        reason.contains("the body of `A` went into `Cut`; split that feature instead"),
        "{reason}"
    );
    // The names have to fit the lumps.
    let reason = refused(
        &mut s,
        "body.split",
        json!({"source": "Cut", "names": ["One"]}),
    );
    assert!(
        reason.contains("`names` has 1 names for 2 lumps"),
        "{reason}"
    );
    let reason = refused(
        &mut s,
        "body.split",
        json!({"source": "Cut", "names": ["Same", "Same"]}),
    );
    assert!(reason.contains("`names` has `Same` twice"), "{reason}");
    let reason = refused(
        &mut s,
        "body.split",
        json!({"source": "Cut", "names": "One"}),
    );
    assert!(
        reason.contains("`names` must be a list of strings"),
        "{reason}"
    );
    let reason = refused(
        &mut s,
        "body.split",
        json!({"source": "Cut", "names": [1, 2]}),
    );
    assert!(
        reason.contains("`names` must be a list of strings"),
        "{reason}"
    );
    // Bad and missing parameters: an error, never a crash.
    for params in [
        json!({}),
        json!({"source": "Nowhere"}),
        json!({"source": 99}),
        json!({"source": null}),
        json!({"source": [1]}),
        json!({"source": "Cut", "extra": 1}),
        json!([]),
    ] {
        refused(&mut s, "body.split", params);
    }
    // A failed source is refused with its reason; a split of a split is
    // refused like any body of one lump.
    assert_eq!(bodies(&mut s), ["Sheet", "Cut"]);
    run(&mut s, "body.split", json!({"source": "Cut"}));
    let reason = refused(&mut s, "body.split", json!({"source": "Split1"}));
    assert!(reason.contains("`Split1` is one lump"), "{reason}");
    let reason = refused(&mut s, "body.split", json!({"source": "Cut"}));
    assert!(reason.contains("went into"), "{reason}");
    // Nothing is left of the refusals in the document.
    assert_eq!(bodies(&mut s), ["Sheet", "Split1", "Split2"]);
    let check = run(&mut s, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
}
