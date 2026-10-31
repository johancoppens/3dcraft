//! How long `Scene::best_view` takes for every feature of a model, at the
//! display quality the window meshes with: what a double click on a row
//! costs before the flight starts.
//!
//! ```sh
//! cargo run --release -p craft3d-engine --example best_view_timing -- model.3dcraft|part.step
//! ```

use craft3d_engine::{Scene, SceneQuality, SelectionItem, SelectionKind, Session, Workspace};
use craft3d_math::Portable;
use craft3d_math::Vec3;
use serde_json::json;
use std::time::Instant;

fn main() -> Result<(), String> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: best_view_timing FILE.3dcraft|FILE.step")?;
    let mut session = Session::new(Workspace::unrestricted());
    let step =
        path.to_ascii_lowercase().ends_with(".step") || path.to_ascii_lowercase().ends_with(".stp");
    if step {
        session
            .execute("file.new", &json!({"units": "mm"}))
            .map_err(|e| e.to_string())?;
        session
            .execute("file.import", &json!({ "path": path }))
            .map_err(|e| e.to_string())?;
    } else {
        session
            .execute("file.open", &json!({ "path": path }))
            .map_err(|e| e.to_string())?;
    }
    let scene: std::sync::Arc<Scene> = session
        .scene_with(SceneQuality::display())
        .map_err(|e| e.to_string())?
        .ok_or("no scene")?;
    let (pitch, yaws) = (29.496_f64.to_radians(), [-45.0_f64, 45.0, 135.0, -135.0]);
    let mut toward = vec![Vec3::new(0.0, 0.0, -1.0)];
    for up in [pitch, -pitch] {
        for yaw in yaws {
            let yaw = yaw.to_radians();
            toward.push(Vec3::new(
                up.pcos() * yaw.pcos(),
                up.pcos() * yaw.psin(),
                up.psin(),
            ));
        }
    }
    let mut times = Vec::new();
    for feature in &scene.features {
        let item = SelectionItem {
            kind: SelectionKind::Feature,
            name: String::new(),
            feature: feature.id.clone(),
        };
        let start = Instant::now();
        let ids = scene.items_face_ids(std::slice::from_ref(&item));
        if ids.is_empty() {
            continue;
        }
        let best = scene.best_view(&ids, &toward, &[]);
        let spent = start.elapsed().as_secs_f64();
        times.push((spent, feature.id.clone(), ids.len(), best));
    }
    times.sort_by(|a, b| b.0.total_cmp(&a.0));
    let total: f64 = times.iter().map(|t| t.0).sum();
    println!(
        "{path}: {} triangles, {} features with faces, mean {:.1} ms, max {:.1} ms",
        scene.triangles(),
        times.len(),
        1e3 * total / (times.len().max(1) as f64),
        1e3 * times.first().map_or(0.0, |t| t.0)
    );
    for (spent, id, faces, best) in times.iter().take(5) {
        println!(
            "  {id}: {faces} faces, {:.1} ms, view {best:?}",
            1e3 * spent
        );
    }
    Ok(())
}
