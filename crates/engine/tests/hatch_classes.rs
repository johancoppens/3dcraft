//! A direction of hatching per part (docs/design/rendering.md › *A
//! direction per part*): each body of a scene gets the hatch class of its
//! place among the document's bodies (`craft3d_engine::hatch_class`), the
//! hidden ones counted too, so that parts next to each other in the
//! document read apart on a cut and hiding a part changes no other part's
//! direction; where two bodies of different classes overlap, the cut is
//! cross-hatched and the pick says no one body is cut there.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::{run, session, session_in};
use craft3d_engine::{Scene, Session, hatch_class};
use craft3d_math::Vec3;
use craft3d_render::{Camera, HATCH_CLASSES, Projection, Section};
use serde_json::json;
use std::path::PathBuf;

/// `n` blocks of 10 along x, each touching the next, named `B0`, `B1`, …;
/// with `overlap` the next one starts 4 into the one before.
fn blocks(n: usize, overlap: f64) -> Session {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    for k in 0..n {
        let x = k as f64 * (10.0 - overlap);
        run(
            &mut s,
            "feature.block",
            json!({"size": [10, 10, 10], "position": [x, 0, 0], "name": format!("B{k}")}),
        );
    }
    s
}

/// The scene's bodies by name, with the hatch class of their faces.
fn classes(scene: &Scene) -> Vec<(String, usize)> {
    let classes = scene.face_colors.hatch_classes();
    scene
        .bodies
        .iter()
        .map(|b| {
            let class = classes.class(b.first_face_id);
            // Every face of the body is of its class.
            let faces = b.mesh.face_ids.iter().copied().max().unwrap_or(0);
            assert_eq!(classes.class(b.first_face_id + faces), class, "{}", b.name);
            (b.name.clone(), class)
        })
        .collect()
}

#[test]
fn each_body_is_hatched_by_its_place_and_hiding_one_changes_no_other() {
    let mut s = blocks(6, 0.0);
    let scene = s.scene(None).unwrap().unwrap();
    let got = classes(&scene);
    let want: Vec<(String, usize)> = (0..6).map(|k| (format!("B{k}"), k % 4)).collect();
    assert_eq!(got, want);
    // Neighbours in the document always differ.
    for pair in got.windows(2) {
        assert_ne!(pair[0].1, pair[1].1, "{pair:?}");
    }
    assert_eq!(hatch_class(0), 0);
    assert_eq!(hatch_class(HATCH_CLASSES + 1), 1);
    // Hiding B1 and B3: the others keep their classes.
    run(
        &mut s,
        "feature.visible",
        json!({"feature": ["B1", "B3"], "visible": false}),
    );
    let hidden = s.scene(None).unwrap().unwrap();
    let kept: Vec<(String, usize)> = want
        .iter()
        .filter(|(name, _)| name != "B1" && name != "B3")
        .cloned()
        .collect();
    assert_eq!(classes(&hidden), kept);
    // And the same again when they are shown: the same picture.
    run(
        &mut s,
        "feature.visible",
        json!({"feature": ["B1", "B3"], "visible": true}),
    );
    let again = s.scene(None).unwrap().unwrap();
    assert_eq!(classes(&again), want);
    assert_eq!(again.face_colors, scene.face_colors);
}

/// A class is a place in the document: deleting a part moves the parts
/// after it one place up, so their directions change, and the parts
/// before it keep theirs.
#[test]
fn deleting_a_part_changes_the_directions_of_the_parts_after_it_and_no_other() {
    let mut s = blocks(6, 0.0);
    run(&mut s, "feature.delete", json!({"feature": "B2"}));
    let scene = s.scene(None).unwrap().unwrap();
    let want: Vec<(String, usize)> = [("B0", 0), ("B1", 1), ("B3", 2), ("B4", 3), ("B5", 0)]
        .into_iter()
        .map(|(name, class)| (name.to_string(), class))
        .collect();
    assert_eq!(classes(&scene), want);
}

#[test]
fn a_document_with_one_body_hatches_it_as_it_always_was() {
    let mut s = blocks(1, 0.0);
    let scene = s.scene(None).unwrap().unwrap();
    assert_eq!(classes(&scene), vec![("B0".to_string(), 0)]);
    assert!(scene.face_colors.hatch_classes().is_empty());
}

/// Seen from the front (-y), cut across y at 5 with the front half away:
/// the cap of the row of blocks, and where the pick lands on it.
fn cut_front(scene: &Scene, x: f64) -> (Option<String>, bool) {
    let section = Section {
        origin: Vec3::new(0.0, 5.0, 0.0),
        normal: -Vec3::Y,
        cap: true,
        color: [232, 228, 220],
        hatch: None,
    };
    let camera = Camera {
        eye: Vec3::new(20.0, -200.0, 5.0),
        target: Vec3::new(20.0, 0.0, 5.0),
        up: Vec3::Z,
        projection: Projection::Orthographic { half_height: 15.0 },
    };
    // 10 pixels per unit, x = 0 at pixel 0, z = 5 on row 150.
    let (picked, cap) = scene
        .pick_cut(&camera, 400, 300, x * 10.0, 150.0, Some(&section))
        .unwrap();
    assert!(picked.is_none() || cap.is_none());
    match cap {
        Some(cap) => (cap.feature, true),
        None => (None, false),
    }
}

#[test]
fn an_overlap_of_two_classes_is_a_cap_of_no_one_body_and_of_one_class_a_hole() {
    // B0 to B4 overlap their neighbours by 4: x 0..10, 6..16, 12..22,
    // 18..28, 24..34.
    let mut s = blocks(5, 4.0);
    let scene = s.scene(None).unwrap().unwrap();
    let names: Vec<String> = scene.bodies.iter().map(|b| b.feature.clone()).collect();
    // In B0 alone, then in B1 alone: the cap of that body.
    assert_eq!(cut_front(&scene, 3.0), (Some(names[0].clone()), true));
    assert_eq!(cut_front(&scene, 11.0), (Some(names[1].clone()), true));
    // Where B0 (class 0) and B1 (class 1) overlap: the cap (drawn
    // cross-hatched), of no one body.
    assert_eq!(cut_front(&scene, 8.0), (None, true));
    // A document whose bodies 0 and 4 overlap: both of class 0, they
    // cancel in the count, and the cut shows no cap there (as before
    // hatch classes), the same fault shown as a hole.
    let mut t = session();
    run(&mut t, "file.new", json!({"units": "mm"}));
    for (k, x) in [0.0, 40.0, 60.0, 80.0, 6.0].into_iter().enumerate() {
        run(
            &mut t,
            "feature.block",
            json!({"size": [10, 10, 10], "position": [x, 0, 0], "name": format!("C{k}")}),
        );
    }
    let other = t.scene(None).unwrap().unwrap();
    let got = classes(&other);
    assert_eq!(got[0].1, got[4].1);
    assert_eq!(cut_front(&other, 8.0), (None, false));
    assert!(cut_front(&other, 3.0).1);
}

fn published() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/examples/published")
}

/// The pairs of bodies of `scene` whose boxes touch or overlap, grown by
/// a thousandth of the scene's size: the parts that can be neighbours on
/// a cut (an upper bound: boxes touch more often than parts do).
fn near_pairs(scene: &Scene) -> Vec<(usize, usize)> {
    let boxes: Vec<_> = scene
        .bodies
        .iter()
        .map(|b| b.mesh.bbox().corners().unwrap())
        .collect();
    let (lo, hi) = scene.mesh.bbox().corners().unwrap();
    let grow = (hi - lo).length() * 1e-3;
    let mut pairs = Vec::new();
    for (i, (alo, ahi)) in boxes.iter().enumerate() {
        for (j, (blo, bhi)) in boxes.iter().enumerate().skip(i + 1) {
            let apart = [
                blo.x - ahi.x,
                alo.x - bhi.x,
                blo.y - ahi.y,
                alo.y - bhi.y,
                blo.z - ahi.z,
                alo.z - bhi.z,
            ]
            .into_iter()
            .any(|gap| gap > grow);
            if !apart {
                pairs.push((i, j));
            }
        }
    }
    pairs
}

/// The published assemblies and the hydraulic cylinder: how many of the
/// pairs of parts that can touch share a hatch direction (the cut's
/// thicker outline is what tells those apart). Printed for the record;
/// the classes are the same scene after scene, and parts next to each
/// other in the document never share one.
#[test]
fn the_published_assemblies_say_how_many_neighbours_share_a_direction() {
    let mut report = String::new();
    for file in [
        "toggle-clamp.3dcraftasm",
        "scissor-jack.3dcraftasm",
        "jansen-walker.3dcraftasm",
        "radial-engine.3dcraftasm",
        "hydraulic-cylinder.3dcraft",
    ] {
        let mut s = session_in(&published());
        run(&mut s, "file.open", json!({ "path": file }));
        let scene = s.scene(None).unwrap().unwrap();
        let got = classes(&scene);
        for pair in got.windows(2) {
            assert_ne!(pair[0].1, pair[1].1, "{file}: {pair:?}");
        }
        let pairs = near_pairs(&scene);
        let shared = pairs.iter().filter(|(i, j)| got[*i].1 == got[*j].1).count();
        let line = format!(
            "{file}: {} bodies, {} pairs whose boxes touch, {shared} of them share a direction\n",
            got.len(),
            pairs.len()
        );
        print!("{line}");
        report.push_str(&line);
        // The same classes when the scene is built again.
        let mut again = session_in(&published());
        run(&mut again, "file.open", json!({ "path": file }));
        let rebuilt = again.scene(None).unwrap().unwrap();
        assert_eq!(classes(&rebuilt), got, "{file}");
    }
    craft3d_testkit::evidence::print("hatch-classes-published.txt", report.as_bytes());
}
