//! What the chordal tolerance costs on one model: the scene built without
//! a budget at a row of tolerances, as fractions of the diagonal of the
//! model's box, with its triangles and the time it took. Run in release:
//!
//! ```sh
//! cargo run --release -p craft3d-engine --example tolerance_sweep -- model.3dcraft
//! ```
//!
//! The first scene (the automatic tolerance, as `view.render` picks it)
//! also regenerates and measures the model; its time is printed apart.

use craft3d_engine::{Session, Workspace};
use serde_json::json;
use std::time::Instant;

fn main() -> Result<(), String> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: tolerance_sweep FILE.3dcraft [FRACTION...]")?;
    let fractions: Vec<f64> = std::env::args()
        .skip(2)
        .filter_map(|a| a.parse().ok())
        .collect();
    let fractions = if fractions.is_empty() {
        vec![2e-3, 1e-3, 5e-4, 2e-4, 1e-4, 5e-5, 3e-5]
    } else {
        fractions
    };
    let mut session = Session::new(Workspace::unrestricted());
    session
        .execute("file.open", &json!({ "path": path }))
        .map_err(|e| e.to_string())?;
    let start = Instant::now();
    let automatic = session
        .scene(None)
        .map_err(|e| e.to_string())?
        .ok_or("no document")?;
    println!(
        "automatic (with regeneration and measure): {:.1} ms, tolerance {:.3e}, {} triangles",
        start.elapsed().as_secs_f64() * 1e3,
        automatic.tolerance,
        automatic.triangles()
    );
    let diagonal = automatic.bbox.diagonal();
    println!("box diagonal {diagonal:.4}");
    for fraction in fractions {
        let tolerance = diagonal * fraction;
        let start = Instant::now();
        let scene = session
            .scene(Some(tolerance))
            .map_err(|e| e.to_string())?
            .ok_or("no document")?;
        println!(
            "{fraction:>8.1e} of the diagonal = {tolerance:.3e}: {:>9} triangles in {:>8.1} ms{}",
            scene.triangles(),
            start.elapsed().as_secs_f64() * 1e3,
            if scene.problems.is_empty() {
                String::new()
            } else {
                format!(" ({})", scene.problems.join("; "))
            }
        );
    }
    Ok(())
}
