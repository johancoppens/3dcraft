//! What measuring a document costs, apart from regenerating it: opens a
//! `.3dcraft` file and regenerates it, then times `document.measure`
//! (every body's area, volume and centroid, integrated exactly) and prints
//! its result's digest, to compare before and after a change that must
//! not change a bit. Run in release:
//!
//! ```sh
//! cargo run --release -p craft3d-engine --example measure_profile -- model.3dcraft
//! ```
//!
//! The CPU time is the process's (all threads), read from `/proc` where
//! there is one.

use craft3d_engine::{Session, Workspace};
use serde_json::json;
use std::time::Instant;

/// The process's CPU time in seconds (user and system), where `/proc`
/// tells it.
fn cpu_seconds() -> Option<f64> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    // The fields after the command's name, which is in parentheses.
    let rest = stat.rsplit_once(')')?.1;
    let fields: Vec<&str> = rest.split_whitespace().collect();
    let (user, system): (f64, f64) = (fields.get(11)?.parse().ok()?, fields.get(12)?.parse().ok()?);
    Some((user + system) / 100.0)
}

fn main() -> Result<(), String> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: measure_profile FILE.3dcraft")?;
    let mut session = Session::new(Workspace::unrestricted());
    let (wall, cpu) = (Instant::now(), cpu_seconds());
    session
        .execute("file.open", &json!({ "path": path }))
        .map_err(|e| e.to_string())?;
    // Regenerated, without measuring: an interference check of no bodies
    // builds every body first.
    session
        .execute("document.interference", &json!({ "bodies": [] }))
        .map_err(|e| e.to_string())?;
    let opened = (wall.elapsed().as_secs_f64(), cpu_seconds());
    let (wall, cpu_before) = (Instant::now(), cpu_seconds());
    let measured = session
        .execute("document.measure", &json!({}))
        .map_err(|e| e.to_string())?;
    let measuring = (wall.elapsed().as_secs_f64(), cpu_seconds());
    let span = |from: Option<f64>, to: Option<f64>| match (from, to) {
        (Some(a), Some(b)) => format!("{:.2} s CPU", b - a),
        _ => "CPU unknown".to_string(),
    };
    println!(
        "open and regenerate: {:.2} s wall, {}",
        opened.0,
        span(cpu, opened.1)
    );
    println!(
        "measure: {:.2} s wall, {}",
        measuring.0,
        span(cpu_before, measuring.1)
    );
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    measured.to_string().hash(&mut hasher);
    println!("measure digest {:016x}", hasher.finish());
    Ok(())
}
