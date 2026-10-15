//! What a viewport shows meshes every body: at the display quality (the
//! chordal tolerance of `DISPLAY_RELATIVE_TOLERANCE` and the angle
//! criterion of `DISPLAY_MAX_ANGLE`) and at every coarser tolerance the
//! display rule's ladder may try, closed and the same on any number of
//! threads (docs/design/tessellator.md, *What a viewer draws*).
//!
//! The regression (2026-10-14): a fillet of a few tenths of a millimetre
//! between a cone and a plane of a revolved band (the turbofan's stripes,
//! and the flange roots of its cut-away casings) failed with the angle
//! criterion at every tolerance as "patch 5 needs about 2050 segments per
//! side", so the window drew seven of its bodies not at all. The chords of
//! the circle where the fillet meets the plane bulge beside the fillet,
//! along the surface; points of the fillet nearer to such a chord than its
//! bulge lie beyond it in space, and the refinement walked towards the
//! chord without end.
//!
//! The guard over whole models is for release builds
//! (`#[cfg_attr(debug_assertions, ignore)]`):
//!
//! ```sh
//! cargo test --release -p craft3d-engine --test display_quality -- --nocapture
//! ```
//!
//! It takes the showcase (`docs/showcase`) and any `.3dcraft` files in the
//! directory `CRAFT3D_DISPLAY_MODELS` names (the developer's turbofan).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{DISPLAY_MAX_ANGLE, SceneQuality, Session, Workspace};
use craft3d_tess::{Mesh, TessParams, tessellate_body_and_edges, tessellate_body_with_threads};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::Instant;

fn display(tolerance: f64) -> TessParams {
    TessParams {
        chordal_tolerance: tolerance,
        max_angle: Some(DISPLAY_MAX_ANGLE),
        ..TessParams::default()
    }
}

fn closed(mesh: &Mesh) -> bool {
    mesh.weld().is_ok_and(|welded| welded.is_closed())
}

/// The bodies of the document at `path`, by feature name, regenerated.
fn bodies(path: &Path) -> Vec<(String, std::sync::Arc<craft3d_topo::Body>)> {
    let doc = craft3d_format::load(path).unwrap();
    let regen = craft3d_regen::regenerate(&doc);
    doc.features
        .iter()
        .filter_map(|f| {
            let body = regen.feature(f.id)?.own_body()?;
            Some((f.name.clone(), body.clone()))
        })
        .collect()
}

/// A band revolved about y from a quadrilateral whose inner and outer
/// sides are cones, with the outer cone filleted to both ends (planes) by
/// `radius`: the turbofan's stripe, saved to `dir`.
fn stripe(dir: &Path, radius: f64) -> PathBuf {
    let mut s = Session::new(Workspace::unrestricted());
    let ok = |s: &mut Session, id: &str, params: serde_json::Value| {
        let result = s.execute(id, &params).unwrap();
        assert_ne!(result["status"], "failed", "{id}: {result}");
    };
    ok(
        &mut s,
        "file.new",
        json!({"units": "mm", "tolerance": 1e-6}),
    );
    ok(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "name": "Band"}),
    );
    ok(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Band", "closed": true,
               "points": [[9.5, 18.0], [11.15, 21.0], [12.15, 21.0], [10.5, 18.0]]}),
    );
    ok(
        &mut s,
        "feature.revolve",
        json!({"sketch": "Band", "axis": "y", "name": "Raw"}),
    );
    ok(
        &mut s,
        "feature.fillet",
        json!({"edges": [["F2:side(e6)", "F2:side(e7)"], ["F2:side(e7)", "F2:side(e8)"]],
               "radius": format!("{radius} mm"), "name": "Stripe"}),
    );
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join(format!("stripe-{radius}.3dcraft"));
    ok(&mut s, "file.save", json!({"path": path.to_str().unwrap()}));
    path
}

/// The stripe's fillets mesh with the angle criterion at the tolerance the
/// window ended at (0.51 mm, its quick first picture's), at the display
/// tolerance of the turbofan (0.17 mm) and finer and coarser ones: closed,
/// the same on one thread and on eight. Before the fix every one of them
/// was "patch 5 needs about 2050 segments per side".
#[test]
fn a_thin_fillet_beside_a_plane_meshes_with_the_angle_criterion() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("display-quality");
    for radius in [0.25, 0.5] {
        let path = stripe(&dir, radius);
        let all = bodies(&path);
        let (_, body) = all.iter().find(|(name, _)| name == "Stripe").unwrap();
        for tolerance in [0.01, 0.05, 0.1, 0.17, 0.25, 0.3, 0.4, 0.5, 0.5078, 1.0] {
            let params = display(tolerance);
            let start = Instant::now();
            let mesh = tessellate_body_and_edges(body, &params)
                .and_then(|both| both.mesh)
                .unwrap_or_else(|e| panic!("fillet {radius} at {tolerance}: {e}"));
            let took = start.elapsed();
            assert!(closed(&mesh), "fillet {radius} at {tolerance}: not closed");
            let one = tessellate_body_with_threads(body, &params, 1).unwrap();
            let eight = tessellate_body_with_threads(body, &params, 8).unwrap();
            assert_eq!(one, mesh, "fillet {radius} at {tolerance}: one thread");
            assert_eq!(eight, mesh, "fillet {radius} at {tolerance}: eight threads");
            let plain = tessellate_body_with_threads(
                body,
                &TessParams {
                    max_angle: None,
                    ..params
                },
                0,
            )
            .unwrap();
            println!(
                "fillet {radius} at {tolerance}: {} triangles in {took:?}, {} without the angle",
                mesh.triangles.len(),
                plain.triangles.len()
            );
        }
    }
}

/// The display scene of the stripe draws it, at the display quality.
#[test]
fn the_display_scene_of_the_stripe_has_no_problems() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("display-quality-scene");
    let path = stripe(&dir, 0.25);
    let mut s = Session::new(Workspace::unrestricted());
    s.execute("file.open", &json!({"path": path.to_str().unwrap()}))
        .unwrap();
    let scene = s.scene_with(SceneQuality::display()).unwrap().unwrap();
    assert!(scene.problems.is_empty(), "{:?}", scene.problems);
    assert_eq!(scene.max_angle, Some(DISPLAY_MAX_ANGLE));
    assert!(!scene.bodies.is_empty());
}

fn models() -> Vec<PathBuf> {
    let mut models = Vec::new();
    let showcase = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/showcase");
    let mut dirs = vec![showcase];
    if let Some(dir) = std::env::var_os("CRAFT3D_DISPLAY_MODELS") {
        dirs.push(PathBuf::from(dir));
    } else {
        println!("CRAFT3D_DISPLAY_MODELS is not set: the showcase's models only");
    }
    for dir in dirs {
        let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| p.extension().is_some_and(|e| e == "3dcraft"))
                    .collect()
            })
            .unwrap_or_default();
        found.sort();
        models.extend(found);
    }
    models
}

/// Every body of every model meshes with the angle criterion at the
/// display scene's tolerance and at its quick first picture's (the
/// coarsest the display rule goes), and without it at the export's
/// automatic tolerance and four times finer; every solid closed; and the
/// display scene has no problems. Tessellated directly, not through the scene, which draws a
/// body it cannot mesh with the angle without it.
#[test]
#[cfg_attr(debug_assertions, ignore = "whole models: for release builds")]
fn every_body_of_the_showcase_meshes_at_the_display_quality() {
    let mut failures = Vec::new();
    for path in models() {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let mut s = Session::new(Workspace::unrestricted());
        s.execute("file.open", &json!({"path": path.to_str().unwrap()}))
            .unwrap();
        let first = s.scene_with(SceneQuality::Preview).unwrap().unwrap();
        let start = Instant::now();
        let scene = s.scene_with(SceneQuality::display()).unwrap().unwrap();
        let took = start.elapsed();
        if !scene.problems.is_empty() {
            failures.push(format!("{name}: {:?}", scene.problems));
        }
        let (mut triangles, mut slowest) = (0, (0.0, String::new()));
        let automatic = s.scene_with(SceneQuality::Automatic).unwrap().unwrap();
        for (body_name, body) in bodies(&path) {
            // The display (with the angle) at the display scene's tolerance
            // and at the first picture's; the export (without) at the
            // automatic tolerance and four times finer.
            let display_tolerances = [scene.tolerance, first.tolerance].map(|t| (t, true));
            let export_tolerances =
                [automatic.tolerance, automatic.tolerance / 4.0].map(|t| (t, false));
            for (tolerance, angle) in display_tolerances.into_iter().chain(export_tolerances) {
                let params = if angle {
                    display(tolerance)
                } else {
                    TessParams {
                        chordal_tolerance: tolerance,
                        ..TessParams::default()
                    }
                };
                let what = if angle { "display" } else { "export" };
                let start = Instant::now();
                match tessellate_body_and_edges(&body, &params).and_then(|both| both.mesh) {
                    Ok(mesh) => {
                        let seconds = start.elapsed().as_secs_f64();
                        if seconds > slowest.0 {
                            slowest = (seconds, format!("{body_name} {what} at {tolerance:.3}"));
                        }
                        if angle && tolerance == scene.tolerance {
                            triangles += mesh.triangles.len();
                        }
                        // Every solid meshes closed (the hull did not until
                        // 2026-10-15: its walls touched along the bow).
                        if body.kind == craft3d_topo::BodyKind::Solid && !closed(&mesh) {
                            failures.push(format!(
                                "{name}/{body_name} {what} at {tolerance}: not closed"
                            ));
                        }
                    }
                    Err(e) => {
                        failures.push(format!("{name}/{body_name} {what} at {tolerance}: {e}"))
                    }
                }
            }
        }
        println!(
            "{name:<12} display {:.4} ({}coarsened) {} triangles in {took:?}; meshed directly: \
             {triangles}, slowest body {:.3} s ({})",
            scene.tolerance,
            if scene.coarsened { "" } else { "not " },
            scene.mesh.triangles.len(),
            slowest.0,
            slowest.1
        );
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
