//! What a moment of an assembly's motion costs, as the window asks for it
//! while a motion plays: `view.motion {t}`, then the display scene of the
//! moment. Ignored: it only prints, for comparing two commits on the same
//! machine
//! (`cargo test -p craft3d-engine --release --test motion_timing -- --ignored --nocapture`).
//!
//! The assemblies are read from `CRAFT3D_JIGS_DIR` (the `dist` folder of
//! the Jigs repository), else from `tests/motion_timing/` beside this
//! file; without either the test prints that and passes.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft3d_engine::{Access, SceneQuality, Session, Workspace};
use serde_json::json;
use std::path::PathBuf;
use std::time::Instant;

const ASSEMBLIES: &[&str] = &["strandbeest-walker"];

fn folder() -> Option<PathBuf> {
    std::env::var_os("CRAFT3D_JIGS_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/motion_timing"))
                .filter(|p| p.is_dir())
        })
}

#[test]
#[ignore = "prints timings only"]
fn a_moment_of_an_assembly_motion() {
    let Some(dir) = folder() else {
        println!("no assemblies to time (CRAFT3D_JIGS_DIR or tests/motion_timing/)");
        return;
    };
    for name in ASSEMBLIES {
        let path = dir.join(format!("{name}.3dcraftasm"));
        if !path.is_file() {
            println!("{name}: not found in {}", dir.display());
            continue;
        }
        let mut s = Session::new(Workspace {
            read: Access::Root(dir.clone()),
            write: Access::None,
        });
        let opened = Instant::now();
        s.execute("file.open", &json!({ "path": path.to_string_lossy() }))
            .unwrap();
        let scene = s.scene_with(SceneQuality::display()).unwrap().unwrap();
        println!(
            "{name}: opened and shown in {:.0} ms, {} bodies, {} triangles",
            opened.elapsed().as_secs_f64() * 1000.0,
            scene.bodies.len(),
            scene.triangles()
        );
        // As a script at 10 moments a second does it: 60 moments, 0 to 1.
        let mut frames = Vec::new();
        let mut motion_ms = Vec::new();
        let (meshed, ..) = craft3d_tess::tess_work();
        let mut placed = 0;
        for k in 0..=60 {
            let t = f64::from(k) / 60.0;
            let start = Instant::now();
            s.execute("view.motion", &json!({ "t": t })).unwrap();
            motion_ms.push(start.elapsed().as_secs_f64() * 1000.0);
            let scene = s.scene_with(SceneQuality::display()).unwrap().unwrap();
            frames.push(start.elapsed().as_secs_f64() * 1000.0);
            placed = scene
                .bodies
                .iter()
                .filter(|b| b.placement.is_some())
                .count();
        }
        let (meshed_after, ..) = craft3d_tess::tess_work();
        let stats = |ms: &[f64]| {
            let mut sorted = ms.to_vec();
            sorted.sort_by(f64::total_cmp);
            let mean = sorted.iter().sum::<f64>() / sorted.len().max(1) as f64;
            let median = sorted.get(sorted.len() / 2).copied().unwrap_or(0.0);
            let max = sorted.last().copied().unwrap_or(0.0);
            format!("mean {mean:.1} ms, median {median:.1} ms, max {max:.1} ms")
        };
        println!(
            "{name}: a moment (view.motion + display scene): {}",
            stats(&frames)
        );
        println!("{name}: of which view.motion: {}", stats(&motion_ms));
        println!(
            "{name}: bodies meshed over 61 moments: {}, bodies placed by a pose: {placed} of {}",
            meshed_after - meshed,
            scene.bodies.len()
        );
    }
}
