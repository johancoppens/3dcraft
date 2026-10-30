//! The moments of an assembly's motion move meshes, they do not make them
//! (docs/design/motion.md › *Assemblies*, › *Costs*): each instance's
//! body is meshed once, as its source, and every moment's scene has that
//! mesh moved by the instance's pose, which a viewer sends to the GPU as
//! a few matrices (`SceneBody::placement`).
//!
//! One test in this file on purpose: it counts the tessellator's work
//! (`craft3d_tess::tess_work`), which every test of a binary shares.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use common::{session_in, workdir};
use craft3d_engine::{SceneQuality, Session};
use serde_json::json;

/// A bar part (a block with two datum axes at its ends) and
/// a four-bar of four instances of it, its crank driven by `360 deg * t`.
fn four_bar(s: &mut Session) {
    run(s, "file.new", json!({}));
    run(s, "variable.set", json!({ "name": "len", "expr": "24 mm" }));
    run(
        s,
        "variable.publish",
        json!({ "name": "len", "min": "10 mm", "max": "500 mm" }),
    );
    run(
        s,
        "feature.block",
        json!({ "name": "Bar", "size": ["len", 4, 2] }),
    );
    run(
        s,
        "datum.axis",
        json!({ "name": "End1", "through": [[2, 2, 0], [2, 2, 1]] }),
    );
    run(
        s,
        "datum.axis",
        json!({ "name": "End2", "through": [["len - 2 mm", 2, 0], ["len - 2 mm", 2, 1]] }),
    );
    run(s, "file.save", json!({ "path": "bar.3dcraft" }));
    run(s, "assembly.new", json!({}));
    for (name, len) in [
        ("Ground", "64 mm"),
        ("Crank", "24 mm"),
        ("Coupler", "64 mm"),
        ("Rocker", "24 mm"),
    ] {
        run(
            s,
            "instance.add",
            json!({ "source": "bar.3dcraft", "name": name, "body": "Bar",
                    "params": { "len": len } }),
        );
    }
    for (name, a, b) in [
        ("Base", "world/z", "Ground/End1"),
        ("Drive", "Ground/End1", "Crank/End1"),
        ("Elbow", "Crank/End2", "Coupler/End1"),
        ("Knee", "Coupler/End2", "Rocker/End2"),
        ("Hip", "Ground/End2", "Rocker/End1"),
    ] {
        let kind = if name == "Base" { "fixed" } else { "revolute" };
        run(
            s,
            "mate.add",
            json!({ "kind": kind, "a": a, "b": b, "name": name, "flip": true }),
        );
    }
    run(s, "variable.set", json!({ "name": "t", "expr": "0" }));
    run(
        s,
        "variable.set",
        json!({ "name": "Drive_angle", "expr": "360 deg * t" }),
    );
}

#[test]
fn the_moments_of_an_assembly_move_meshes_and_make_none() {
    let dir = workdir("motion-meshes", "four-bar");
    let mut s = session_in(&dir);
    four_bar(&mut s);
    // The first moments mesh the sources, once per sampling they are
    // asked at. After that a moment meshes no placed body: only when the
    // quick first picture's tolerance, which follows the box around the
    // bodies (`scene::preview_size`), steps to another rung of its ladder
    // as the linkage moves are the two sources meshed again, once, at it.
    run(&mut s, "view.motion", json!({ "t": 0.0 }));
    let first = s
        .scene_with(SceneQuality::display())
        .unwrap()
        .expect("a scene");
    assert_eq!(first.bodies.len(), 4);
    run(&mut s, "view.motion", json!({ "t": 1.0 / 24.0 }));
    s.scene_with(SceneQuality::display()).unwrap();
    let mut crank_tips = Vec::new();
    let mut meshed_per_moment = Vec::new();
    for k in 1..=12 {
        // Up to t = 0.4: this parallelogram linkage stops at its change
        // point, the bars in one line at t = 0.5.
        let t = f64::from(k) / 30.0;
        let before = craft3d_tess::tess_work().0;
        run(&mut s, "view.motion", json!({ "t": t }));
        let scene = s
            .scene_with(SceneQuality::display())
            .unwrap()
            .expect("a scene");
        meshed_per_moment.push(craft3d_tess::tess_work().0 - before);
        assert_eq!(scene.bodies.len(), 4, "t = {t}");
        for (body, at_rest) in scene.bodies.iter().zip(first.bodies.iter()) {
            let placed = body
                .placement
                .as_ref()
                .unwrap_or_else(|| panic!("t = {t}: {} is not placed", body.name));
            let rest = at_rest.placement.as_ref().unwrap();
            // The same source mesh as at the first moment: shared, not
            // made again.
            assert!(
                std::sync::Arc::ptr_eq(&placed.source, &rest.source),
                "t = {t}: {} has another source mesh",
                body.name
            );
            // The body's mesh is the source's moved by the pose, bit for
            // bit (what the GPU draws from the source and the pose).
            assert_eq!(body.mesh.triangles, placed.source.triangles);
            assert_eq!(body.mesh.face_ids, placed.source.face_ids);
            for (p, q) in body.mesh.positions.iter().zip(&placed.source.positions) {
                assert_eq!(*p, placed.pose.point(*q), "t = {t}: {}", body.name);
            }
            // Its edges and vertices moved with it.
            assert_eq!(body.vertices.len(), at_rest.vertices.len());
        }
        let crank = scene.bodies.iter().find(|b| b.name == "Crank").unwrap();
        crank_tips.push(crank.mesh.bbox().center().unwrap());
    }
    // Placing would mesh the 4 bodies at every moment (48 in all): each
    // moment meshes nothing or the 2 sources once, and few moments do.
    assert!(
        meshed_per_moment.iter().all(|&n| n == 0 || n == 2),
        "{meshed_per_moment:?}"
    );
    assert!(
        meshed_per_moment.iter().filter(|&&n| n > 0).count() <= 3,
        "{meshed_per_moment:?}"
    );
    // The crank went round: its box's centre moved from moment to moment.
    assert!(
        crank_tips.windows(2).all(|w| (w[0] - w[1]).length() > 1e-3),
        "{crank_tips:?}"
    );
}
