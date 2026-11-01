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

/// Plays the motion of the assembly at `CRAFT3D_MOTION_ASM` twice over
/// `CRAFT3D_MOTION_FRAMES` frames (120 by default), as the player does:
/// `view.motion {t}` and the display scene per frame. An assembly whose
/// instances take parameters driven by `t` builds a configuration per
/// frame (docs/design/motion.md › *Configurations over a motion*); the
/// second pass shows what the kept configurations save. Without the
/// variable the test prints that and passes.
#[test]
#[ignore = "prints timings only"]
fn a_motion_through_configurations() {
    let Some(path) = std::env::var_os("CRAFT3D_MOTION_ASM").map(PathBuf::from) else {
        println!("no assembly to time (CRAFT3D_MOTION_ASM)");
        return;
    };
    let frames: u32 = std::env::var("CRAFT3D_MOTION_FRAMES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(120);
    let dir = path.parent().unwrap().to_path_buf();
    let mut s = Session::new(Workspace {
        read: Access::Root(dir),
        write: Access::None,
    });
    let opened = Instant::now();
    s.execute("file.open", &json!({ "path": path.to_string_lossy() }))
        .unwrap();
    s.scene_with(SceneQuality::display()).unwrap().unwrap();
    println!(
        "opened and shown in {:.0} ms",
        opened.elapsed().as_secs_f64() * 1000.0
    );
    if std::env::var_os("CRAFT3D_MOTION_BAKE").is_some() {
        let start = Instant::now();
        let baked = s
            .execute("motion.bake", &json!({ "frames": frames }))
            .unwrap();
        println!(
            "motion.bake in {:.0} ms: {baked}; resident {:.0} MB",
            start.elapsed().as_secs_f64() * 1000.0,
            resident_mb()
        );
    }
    for pass in 1..=2 {
        let mut motion_ms = Vec::new();
        let mut frame_ms = Vec::new();
        let whole = Instant::now();
        for k in 0..frames {
            let t = f64::from(k) / f64::from(frames);
            let start = Instant::now();
            s.execute("view.motion", &json!({ "t": t })).unwrap();
            motion_ms.push(start.elapsed().as_secs_f64() * 1000.0);
            s.scene_with(SceneQuality::display()).unwrap().unwrap();
            frame_ms.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        let stats = |ms: &[f64]| {
            let mut sorted = ms.to_vec();
            sorted.sort_by(f64::total_cmp);
            let mean = sorted.iter().sum::<f64>() / sorted.len().max(1) as f64;
            let median = sorted.get(sorted.len() / 2).copied().unwrap_or(0.0);
            let max = sorted.last().copied().unwrap_or(0.0);
            format!("mean {mean:.1} ms, median {median:.1} ms, max {max:.1} ms")
        };
        println!(
            "pass {pass}: {frames} frames in {:.2} s; view.motion {}; with the scene {}; resident {:.0} MB",
            whole.elapsed().as_secs_f64(),
            stats(&motion_ms),
            stats(&frame_ms),
            resident_mb()
        );
    }
}

/// What one configuration of the part at `CRAFT3D_CONFIG_PART` holds: its
/// bodies' bytes, the bodies no later feature took apart from the rest.
#[test]
#[ignore = "prints sizes only"]
fn the_size_of_a_configuration() {
    let Some(path) = std::env::var_os("CRAFT3D_CONFIG_PART") else {
        println!("no part (CRAFT3D_CONFIG_PART)");
        return;
    };
    let doc = craft3d_format::from_bytes(&std::fs::read(path).unwrap()).unwrap();
    let start = Instant::now();
    let regen = craft3d_regen::regenerate(&doc);
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    let (mut own, mut taken, mut faces) = (0, 0, 0);
    for f in &regen.features {
        if let Some(body) = &f.body {
            let bytes = craft3d_regen::body_bytes(body);
            if f.consumed_by.is_none() {
                own += bytes;
                faces += body.faces.len();
            } else {
                taken += bytes;
            }
        }
    }
    println!(
        "regenerated in {ms:.0} ms; own bodies {} faces, {:.2} MB; bodies taken by later features {:.2} MB",
        faces,
        own as f64 / 1e6,
        taken as f64 / 1e6
    );
    // What 16 regenerations kept take in fact.
    let before = resident_mb();
    let kept: Vec<_> = (0..16).map(|_| craft3d_regen::regenerate(&doc)).collect();
    let each = (resident_mb() - before) / kept.len() as f64;
    println!("resident per regeneration kept: {each:.1} MB");
}

/// The resident memory of this process in MB (Linux; 0 elsewhere).
fn resident_mb() -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|kb| kb.parse::<f64>().ok())
        })
        .map_or(0.0, |kb| kb / 1024.0)
}
