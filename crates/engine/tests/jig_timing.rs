//! Where the time goes when a Jig is opened: reading, regeneration (per
//! feature), the window's scenes, and each assertion. Ignored: it only
//! prints, for comparing two commits on the same machine
//! (`cargo test -p craft3d-engine --release --test jig_timing -- --ignored --nocapture`).
//!
//! The Jigs are read from `CRAFT3D_JIGS_DIR` (the `dist` folder of the
//! Jigs repository), else from `tests/jig_timing/` beside this file;
//! without either the test prints that and passes.

use craft3d_engine::{SceneQuality, Session, Workspace};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

const JIGS: &[&str] = &["welding-table", "solar-cooker", "gear-reducer"];

fn folder() -> Option<PathBuf> {
    std::env::var_os("CRAFT3D_JIGS_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/jig_timing"))
                .filter(|p| p.is_dir())
        })
}

fn ms(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1000.0
}

/// Runs `work`, returning its result and the time each reported step
/// took (until the next step or the end), slowest first.
fn steps<R>(work: impl FnOnce() -> R) -> (R, Vec<(f64, String)>) {
    let log: Rc<RefCell<Vec<(Instant, String)>>> = Rc::default();
    let sink = Rc::clone(&log);
    let out = craft3d_regen::progress::with_sink(
        move |p: &craft3d_regen::progress::Progress| {
            let what = format!(
                "{} {} {}",
                p.phase,
                p.feature.as_deref().unwrap_or(""),
                p.name.as_deref().unwrap_or("")
            );
            sink.borrow_mut().push((Instant::now(), what));
        },
        work,
    );
    let end = Instant::now();
    let log = log.borrow();
    let mut took: Vec<(f64, String)> = log
        .iter()
        .enumerate()
        .map(|(i, (at, what))| {
            let next = log.get(i + 1).map_or(end, |(t, _)| *t);
            ((next - *at).as_secs_f64() * 1000.0, what.clone())
        })
        .collect();
    took.sort_by(|a, b| b.0.total_cmp(&a.0));
    (out, took)
}

fn print_top(label: &str, took: &[(f64, String)]) {
    let total: f64 = took.iter().map(|t| t.0).sum();
    println!(
        "  {label}: {} steps, {total:.0} ms in steps; slowest:",
        took.len()
    );
    for (t, what) in took.iter().take(8) {
        println!("    {t:8.1} ms  {what}");
    }
}

#[test]
#[ignore = "prints timings only"]
fn where_the_time_goes_when_a_jig_opens() {
    let Some(dir) = folder() else {
        println!("no Jigs: set CRAFT3D_JIGS_DIR");
        return;
    };
    for jig in JIGS {
        let path = dir.join(format!("{jig}.3dcraftjig"));
        if !path.is_file() {
            println!("{jig}: not found in {}", dir.display());
            continue;
        }
        println!("== {jig}");
        let path = path.to_string_lossy().to_string();
        let mut session = Session::new(Workspace::unrestricted());
        let start = Instant::now();
        let (opened, took) = steps(|| session.execute("file.open", &json!({ "path": path })));
        assert!(opened.is_ok(), "{opened:?}");
        println!("  file.open: {:.0} ms", ms(start));
        print_top("regeneration in file.open", &took);
        let t = Instant::now();
        let (_, took) = steps(|| session.scene_with(SceneQuality::Preview));
        println!("  preview scene: {:.0} ms", ms(t));
        print_top("preview scene", &took);
        let t = Instant::now();
        let (scene, took) = steps(|| session.scene_with(SceneQuality::display()));
        let triangles: usize = scene
            .ok()
            .flatten()
            .map(|s| s.mesh.triangles.len())
            .unwrap_or(0);
        println!("  display scene: {:.0} ms, {triangles} triangles", ms(t));
        print_top("display scene", &took);
        // Before anything measured them: a copy of the session measures
        // all at once, as the window's thread does.
        let mut together = session.clone();
        // Each assertion alone, on a copy of the session without the
        // others (the regeneration is shared).
        let listed = session
            .execute("assert.list", &json!({ "wait": false }))
            .unwrap_or_default();
        let names: Vec<String> = listed["assertions"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x["name"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let mut all = 0.0;
        for name in &names {
            // A Jig in use is not edited: a model of it is.
            let mut alone = session.clone();
            alone
                .execute("jig.edit", &json!({}))
                .expect("a model of the Jig");
            for other in names.iter().filter(|n| *n != name) {
                alone
                    .execute("assert.delete", &json!({ "name": other }))
                    .expect("the other assertions deleted");
            }
            // Regenerated before the clock starts.
            let _ = alone.scene_with(SceneQuality::Preview);
            let t = Instant::now();
            let checked = alone.execute("assert.list", &json!({ "wait": true }));
            let took = ms(t);
            all += took;
            let status = checked
                .ok()
                .and_then(|c| c["assertions"][0]["status"].as_str().map(str::to_string))
                .unwrap_or_default();
            println!("    assertion {took:8.1} ms  {status:9} {name}");
        }
        println!("  {} assertions: {all:.0} ms in all", names.len());
        let t = Instant::now();
        let _ = together.execute("assert.list", &json!({ "wait": true }));
        println!("  assert.list (all at once): {:.0} ms", ms(t));
        let t = Instant::now();
        let _ = session.execute("document.inspect", &Value::Null);
        println!("  document.inspect: {:.0} ms", ms(t));
        println!("  total: {:.0} ms", ms(start));
    }
}
