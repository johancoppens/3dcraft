//! Where an edit spends its time once a document is open: stepping the
//! rollback marker back through the history, and one `feature.edit`, as
//! the window does them (the command, then the quick first picture, then
//! the display scene). Run in release:
//!
//! ```sh
//! cargo run --release -p craft3d-engine --example step_profile -- model.3dcraft [STEPS] [EDIT_JSON]
//! ```
//!
//! `STEPS` rollback steps from the end of the history (10 by default);
//! `EDIT_JSON` the parameters of a `feature.edit` to time after them (none
//! by default). Per step: the milliseconds of the command (which
//! regenerates), of the first picture and of the display scene, the
//! tolerances of both, and the tessellator's and regeneration's work
//! counters, so that a cache that is meant to save meshing shows as fewer
//! bodies meshed.

use craft3d_engine::{SceneQuality, Session, Workspace};
use craft3d_regen::progress::{Progress, with_sink};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Instant;

/// Milliseconds per phase of the progress reported while `work` ran.
fn phases<R>(work: impl FnOnce() -> R) -> (R, BTreeMap<&'static str, f64>, f64) {
    type Steps = Rc<RefCell<Vec<(Instant, &'static str)>>>;
    let steps: Steps = Rc::new(RefCell::new(Vec::new()));
    let sink = Rc::clone(&steps);
    let start = Instant::now();
    let result = with_sink(
        move |p: &Progress| sink.borrow_mut().push((Instant::now(), p.phase)),
        work,
    );
    let end = Instant::now();
    let steps = steps.borrow();
    let mut by_phase: BTreeMap<&'static str, f64> = BTreeMap::new();
    for (i, (at, phase)) in steps.iter().enumerate() {
        let next = steps.get(i + 1).map_or(end, |(next, _)| *next);
        *by_phase.entry(phase).or_insert(0.0) += next.duration_since(*at).as_secs_f64() * 1e3;
    }
    (
        result,
        by_phase,
        end.duration_since(start).as_secs_f64() * 1e3,
    )
}

fn ms(by_phase: &BTreeMap<&'static str, f64>) -> String {
    by_phase
        .iter()
        .map(|(phase, ms)| format!("{phase} {ms:.0}"))
        .collect::<Vec<_>>()
        .join(", ")
}

struct Counters {
    tess: (u64, u64, u64),
    regen: craft3d_regen::work::RegenWork,
}

fn counters() -> Counters {
    Counters {
        tess: craft3d_tess::tess_work(),
        regen: craft3d_regen::work::work(),
    }
}

fn delta(before: &Counters, after: &Counters) -> String {
    format!(
        "meshed {} bodies {} faces {} triangles; rebuilt {} reused {}",
        after.tess.0 - before.tess.0,
        after.tess.1 - before.tess.1,
        after.tess.2 - before.tess.2,
        after.regen.features_rebuilt - before.regen.features_rebuilt,
        after.regen.features_reused - before.regen.features_reused,
    )
}

/// Runs `command`, then builds the first picture and the display scene,
/// as the window does after a task, and prints what each took.
fn step(
    session: &mut Session,
    label: &str,
    command: impl FnOnce(&mut Session) -> Result<(), String>,
) -> Result<(), String> {
    let before = counters();
    let start = Instant::now();
    let (ran, command_phases, command_ms) = phases(|| command(session));
    ran?;
    let (preview, preview_phases, preview_ms) = phases(|| {
        session
            .scene_with(SceneQuality::Preview)
            .map_err(|e| e.to_string())
    });
    let preview = preview?;
    let (display, display_phases, display_ms) = phases(|| {
        session
            .scene_with(SceneQuality::display())
            .map_err(|e| e.to_string())
    });
    let display = display?;
    let total = start.elapsed().as_secs_f64() * 1e3;
    let after = counters();
    println!(
        "{label}: {total:.0} ms = command {command_ms:.0} [{}] + preview {preview_ms:.0} [{}] + display {display_ms:.0} [{}]",
        ms(&command_phases),
        ms(&preview_phases),
        ms(&display_phases),
    );
    println!(
        "    preview tol {} ({} tri), display tol {} ({} tri, coarsened {}); {}",
        preview.as_ref().map_or(0.0, |s| s.tolerance),
        preview.as_ref().map_or(0, |s| s.triangles()),
        display.as_ref().map_or(0.0, |s| s.tolerance),
        display.as_ref().map_or(0, |s| s.triangles()),
        display.as_ref().is_some_and(|s| s.coarsened),
        delta(&before, &after)
    );
    Ok(())
}

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .ok_or("usage: step_profile FILE.3dcraft [STEPS] [EDIT_JSON]")?;
    let steps: usize = args
        .next()
        .map_or(Ok(10), |s| s.parse().map_err(|e| format!("STEPS: {e}")))?;
    let edit: Option<Value> = args
        .next()
        .map(|s| serde_json::from_str(&s).map_err(|e| format!("EDIT_JSON: {e}")))
        .transpose()?;
    let mut session = Session::new(Workspace::unrestricted());
    step(&mut session, "open", |s| {
        s.execute("file.open", &json!({ "path": path }))
            .map(drop)
            .map_err(|e| e.to_string())
    })?;
    let inspect = session
        .execute("document.inspect", &json!({}))
        .map_err(|e| e.to_string())?;
    let ids: Vec<String> = inspect
        .get("features")
        .and_then(Value::as_array)
        .map(|features| {
            features
                .iter()
                .filter_map(|f| f.get("id").and_then(Value::as_str).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    println!("{} features", ids.len());
    for id in ids.iter().rev().skip(1).take(steps) {
        let id = id.clone();
        step(&mut session, &format!("rollback to {id}"), |s| {
            s.execute("history.rollback", &json!({ "feature": id }))
                .map(drop)
                .map_err(|e| e.to_string())
        })?;
    }
    step(&mut session, "rollback off", |s| {
        s.execute("history.rollback", &json!({ "feature": null }))
            .map(drop)
            .map_err(|e| e.to_string())
    })?;
    if let Some(edit) = edit {
        step(&mut session, "feature.edit", |s| {
            s.execute("feature.edit", &edit)
                .map(drop)
                .map_err(|e| e.to_string())
        })?;
        step(&mut session, "edit.undo", |s| {
            s.execute("edit.undo", &json!({}))
                .map(drop)
                .map_err(|e| e.to_string())
        })?;
    }
    println!("scenes kept: {:.0} MB", session.scene_bytes() as f64 / 1e6);
    Ok(())
}
