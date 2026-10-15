//! A cut thread at the fine tolerances a viewport asks for: the display
//! rule meshed at 3e-5 of the model's size until 2026-10-14, here about
//! 0.00125 mm (now at 2e-4 with an angle criterion, which is checked
//! too).
//!
//! An M10 thread 24 mm long on a shank with a chamfered end did not come
//! back at 0.0015 mm (a screw with such a thread sat in "meshing" for
//! over thirteen minutes in the app, its engine never answering): the wall of the shank winds around its axis
//! along sixteen turns of the groove, every straight seam tried for it
//! was refused (the seam's end at the loop above came out a rounding
//! error away from that loop's own vertex, and the loop's segments
//! there then crossed it), the face took its chords for too coarse and
//! had them cut round after round, and each round's triangulations cost
//! the square of their points (`crates/tess`, docs/architecture.md, the
//! tessellator's seams and its work budget).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{DISPLAY_MAX_TRIANGLES, SceneQuality, Session, Workspace};
use serde_json::{Value, json};
use std::time::Instant;

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

#[test]
fn a_cut_thread_meshes_at_the_display_tolerance_in_seconds() {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": "5 mm", "height": "40 mm", "name": "Shank"}),
    );
    run(
        &mut s,
        "feature.chamfer",
        json!({"edges": [["F1:face(0)", "F1:face(1)"]], "distance": "1 mm", "name": "Lead"}),
    );
    run(
        &mut s,
        "feature.thread",
        json!({"face": "F1:face(0)", "start": "F1:face(1)", "size": "M10", "length": "24 mm", "modeled": true, "name": "Thread"}),
    );
    // 0.0015 is where it hung; the display rule at 3e-5 asked for a
    // little less here, which happened to escape the rounding.
    let fine = SceneQuality::Display {
        relative: 3e-5,
        max_triangles: DISPLAY_MAX_TRIANGLES,
        max_angle: None,
    };
    for quality in [SceneQuality::Tolerance(0.0015), fine] {
        let start = Instant::now();
        let scene = s.scene_with(quality).unwrap().unwrap();
        let seconds = start.elapsed().as_secs_f64();
        let what = format!(
            "{quality:?}: {} triangles at {} in {seconds:.1} s, coarsened {}: {:?}",
            scene.mesh.triangles.len(),
            scene.tolerance,
            scene.coarsened,
            scene.problems
        );
        assert!(scene.problems.is_empty(), "{what}");
        assert_eq!(scene.bodies.len(), 1, "{what}");
        // At the tolerance asked for: well within the triangle budget.
        assert!(!scene.coarsened && scene.tolerance <= 0.0015, "{what}");
        assert!(
            (50_000..600_000).contains(&scene.mesh.triangles.len()),
            "{what}"
        );
        let welded = scene.mesh.weld().unwrap();
        assert!(welded.is_closed(), "{what}");
        // Seconds in release; a generous bound for a busy machine.
        assert!(seconds < 120.0, "{what}");
        println!("{what}");
    }
    // The display rule as it is now: coarser, with the angle, closed.
    let start = Instant::now();
    let scene = s.scene_with(SceneQuality::display()).unwrap().unwrap();
    let what = format!(
        "display: {} triangles at {} in {:.1} s: {:?}",
        scene.mesh.triangles.len(),
        scene.tolerance,
        start.elapsed().as_secs_f64(),
        scene.problems
    );
    assert!(scene.problems.is_empty(), "{what}");
    assert!(!scene.coarsened, "{what}");
    assert!(scene.mesh.weld().unwrap().is_closed(), "{what}");
    println!("{what}");
}
