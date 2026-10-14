//! Where opening a document spends its time: every step regeneration and
//! meshing report (`craft3d_regen::progress`), timed, for a `.3dcraft`
//! file. Run in release:
//!
//! ```sh
//! cargo run --release -p craft3d-engine --example regen_profile -- model.3dcraft
//! ```
//!
//! Prints one row per step (phase, feature, name, milliseconds), then the
//! totals: opening (which regenerates), the display scene the window asks
//! for, the automatic scene `view.render` asks for, and opening a second
//! document and making the first active again (`file.activate`).

use craft3d_engine::{SceneQuality, Session, Workspace};
use craft3d_regen::progress::{Progress, with_sink};
use serde_json::json;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

type Steps = Rc<RefCell<Vec<(Instant, Progress)>>>;

fn timed<R>(label: &str, work: impl FnOnce() -> R) -> R {
    let steps: Steps = Rc::new(RefCell::new(Vec::new()));
    let sink = Rc::clone(&steps);
    let start = Instant::now();
    let result = with_sink(
        move |p: &Progress| sink.borrow_mut().push((Instant::now(), p.clone())),
        work,
    );
    let end = Instant::now();
    let steps = steps.borrow();
    for (i, (at, step)) in steps.iter().enumerate() {
        let next = steps.get(i + 1).map_or(end, |(next, _)| *next);
        println!(
            "  {:<12} {:<4} {:<24} {:>9.1} ms",
            step.phase,
            step.feature.as_deref().unwrap_or("-"),
            step.name.as_deref().unwrap_or("-"),
            next.duration_since(*at).as_secs_f64() * 1e3
        );
    }
    let before = steps
        .first()
        .map_or(end, |(at, _)| *at)
        .duration_since(start);
    println!(
        "{label}: {:.1} ms ({:.1} ms before the first step)",
        end.duration_since(start).as_secs_f64() * 1e3,
        before.as_secs_f64() * 1e3
    );
    result
}

fn main() -> Result<(), String> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: regen_profile FILE.3dcraft")?;
    let mut session = Session::new(Workspace::unrestricted());
    timed("file.open", || {
        session
            .execute("file.open", &json!({ "path": path }))
            .map_err(|e| e.to_string())
    })?;
    let display = timed("display scene", || {
        session
            .scene_with(SceneQuality::display())
            .map_err(|e| e.to_string())
    })?;
    if let Some(scene) = display {
        // What the window draws, as one number: compared before and after
        // a change that must not change it.
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for p in &scene.mesh.positions {
            p.x.to_bits().hash(&mut hasher);
            p.y.to_bits().hash(&mut hasher);
            p.z.to_bits().hash(&mut hasher);
        }
        for n in &scene.mesh.normals {
            n.x.to_bits().hash(&mut hasher);
            n.y.to_bits().hash(&mut hasher);
            n.z.to_bits().hash(&mut hasher);
        }
        scene.mesh.triangles.hash(&mut hasher);
        scene.mesh.face_ids.hash(&mut hasher);
        scene.tolerance.to_bits().hash(&mut hasher);
        println!(
            "display mesh: {} triangles at tolerance {}, hash {:016x}",
            scene.triangles(),
            scene.tolerance,
            hasher.finish()
        );
    }
    timed("automatic scene", || {
        session
            .scene_with(SceneQuality::Automatic)
            .map_err(|e| e.to_string())
    })?;
    timed("file.new (second document)", || {
        session
            .execute("file.new", &json!({}))
            .map_err(|e| e.to_string())
    })?;
    timed("display scene of the second", || {
        session
            .scene_with(SceneQuality::display())
            .map_err(|e| e.to_string())
    })?;
    timed("file.activate (back to the first)", || {
        session
            .execute("file.activate", &json!({ "document": 0 }))
            .map_err(|e| e.to_string())
    })?;
    timed("display scene after file.activate", || {
        session
            .scene_with(SceneQuality::display())
            .map_err(|e| e.to_string())
    })?;
    Ok(())
}
