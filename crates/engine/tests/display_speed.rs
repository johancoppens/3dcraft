//! Timing targets of what a viewport shows (AGENTS.md rule 12): the quick
//! first picture, the display scene that replaces it, and switching among
//! open documents. In release only (`#[cfg_attr(debug_assertions, ignore)]`):
//!
//! ```sh
//! cargo test --release -p craft3d-engine --test display_speed -- --nocapture
//! ```
//!
//! The models are the showcase's spring and screw (`docs/showcase`), and
//! any `.3dcraft` files in the directory `CRAFT3D_DISPLAY_MODELS` names
//! (the developer's plane and villa); a model that is not there is said
//! and skipped. The bounds are generous (a loaded machine runs three times
//! slower): what the tests catch is a step back by a large factor, the
//! printed table is the measurement.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{SceneQuality, Session, Workspace};
use craft3d_regen::progress::{Progress, with_sink};
use serde_json::json;
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

/// The first picture is meshed within this, after regeneration: the goal
/// is half a second.
const FIRST_PICTURE: Duration = Duration::from_millis(1500);
/// The display scene is built within this.
const SHARP_PICTURE: Duration = Duration::from_secs(20);
/// Making an open document active again shows its scene within this.
const SWITCH: Duration = Duration::from_millis(50);

fn models() -> Vec<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/showcase");
    let mut models = vec![root.join("spring.3dcraft"), root.join("screw.3dcraft")];
    if let Some(dir) = std::env::var_os("CRAFT3D_DISPLAY_MODELS") {
        let mut more: Vec<PathBuf> = std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| p.extension().is_some_and(|e| e == "3dcraft"))
                    .collect()
            })
            .unwrap_or_default();
        more.sort();
        models.extend(more);
    } else {
        println!("CRAFT3D_DISPLAY_MODELS is not set: the showcase's models only");
    }
    models
}

fn open(session: &mut Session, path: &std::path::Path) {
    session
        .execute("file.open", &json!({ "path": path.to_str().unwrap() }))
        .unwrap();
}

/// Times one model: regeneration, the first picture after it, and the
/// display scene built from that picture as a viewer's refinement is.
#[test]
#[cfg_attr(debug_assertions, ignore = "timings are for release builds")]
fn the_first_picture_comes_at_once_and_the_display_scene_soon_after() {
    println!(
        "{:<16} {:>10} {:>10} {:>9} {:>10} {:>9} {:>10}",
        "model", "regen ms", "first ms", "first Δ", "sharp ms", "sharp Δ", "tolerance"
    );
    for path in models() {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        if !path.exists() {
            println!("{name}: not there, skipped");
            continue;
        }
        let mut session = Session::new(Workspace::unrestricted());
        open(&mut session, &path);
        session.set_refine_elsewhere(true);
        // Regeneration ends where the scene's own work starts.
        let measuring: Rc<Cell<Option<Instant>>> = Rc::new(Cell::new(None));
        let mark = Rc::clone(&measuring);
        let start = Instant::now();
        let first = with_sink(
            move |p: &Progress| {
                if p.phase == "measuring" && mark.get().is_none() {
                    mark.set(Some(Instant::now()));
                }
            },
            || session.scene_with(SceneQuality::display()),
        )
        .unwrap()
        .unwrap();
        let end = Instant::now();
        let meshed_from = measuring.get().unwrap_or(start);
        let (regen, meshing) = (meshed_from - start, end - meshed_from);
        assert!(first.preview);
        let job = session.refinement(SceneQuality::display()).unwrap();
        let started = Instant::now();
        let refined = job.run(&AtomicBool::new(false));
        let sharp_time = started.elapsed();
        assert!(session.accept_refined(refined));
        let sharp = session
            .scene_with(SceneQuality::display())
            .unwrap()
            .unwrap();
        assert!(!sharp.preview);
        println!(
            "{name:<16} {:>10.0} {:>10.0} {:>9} {:>10.0} {:>9} {:>10.3e}{}",
            regen.as_secs_f64() * 1e3,
            meshing.as_secs_f64() * 1e3,
            first.triangles(),
            sharp_time.as_secs_f64() * 1e3,
            sharp.triangles(),
            sharp.tolerance,
            if sharp.coarsened { " (coarsened)" } else { "" }
        );
        assert!(meshing < FIRST_PICTURE, "{name}: first picture {meshing:?}");
        assert!(
            sharp_time < SHARP_PICTURE,
            "{name}: display scene {sharp_time:?}"
        );
    }
}

/// Six open documents, each with its display scene: making any of them
/// active again shows the very scene it showed, at once.
#[test]
#[cfg_attr(debug_assertions, ignore = "timings are for release builds")]
fn switching_among_six_open_documents_is_immediate() {
    let mut session = Session::new(Workspace::unrestricted());
    let mut paths: Vec<PathBuf> = models().into_iter().filter(|p| p.exists()).collect();
    let showcase = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/showcase");
    for more in ["gear", "impeller", "vase", "belt", "pipe", "handle"] {
        if paths.len() >= 6 {
            break;
        }
        paths.push(showcase.join(format!("{more}.3dcraft")));
    }
    let mut shown = Vec::new();
    for path in &paths {
        open(&mut session, path);
        shown.push(
            session
                .scene_with(SceneQuality::display())
                .unwrap()
                .unwrap(),
        );
    }
    println!(
        "{} documents, {} MB of scenes",
        paths.len(),
        session.scene_bytes() >> 20
    );
    let mut slowest = Duration::ZERO;
    for round in 0..3 {
        for k in 0..paths.len() {
            let document = (k * 5 + round) % paths.len();
            let start = Instant::now();
            session
                .execute("file.activate", &json!({ "document": document }))
                .unwrap();
            let scene = session
                .scene_with(SceneQuality::display())
                .unwrap()
                .unwrap();
            let took = start.elapsed();
            slowest = slowest.max(took);
            assert!(
                Arc::ptr_eq(&scene.mesh, &shown[document].mesh),
                "document {document} was meshed again"
            );
        }
    }
    println!("slowest switch: {:.2} ms", slowest.as_secs_f64() * 1e3);
    assert!(slowest < SWITCH, "{slowest:?}");
}
