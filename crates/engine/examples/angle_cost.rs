//! What the tessellator's angle criterion costs on one model: every body
//! meshed at one tolerance without the angle and with the display's
//! (`DISPLAY_MAX_ANGLE`), the triangles and the time, per kind of surface
//! and for the faces that cost most. Run in release:
//!
//! ```sh
//! cargo run --release -p craft3d-engine --example angle_cost -- model.3dcraft [TOLERANCE] [BODY]
//! ```
//!
//! Without a tolerance, the display scene's (`SceneQuality::display()`);
//! without a body, every body of the model.

use craft3d_engine::{DISPLAY_MAX_ANGLE, SceneQuality, Session, Workspace};
use craft3d_tess::{Mesh, TessParams, tessellate_body};
use serde_json::json;
use std::collections::BTreeMap;
use std::time::Instant;

fn per_face(mesh: &Mesh, faces: usize) -> Vec<usize> {
    let mut counts = vec![0; faces];
    for &id in &mesh.face_ids {
        if let Some(count) = counts.get_mut(id as usize) {
            *count += 1;
        }
    }
    counts
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let path = args
        .get(1)
        .ok_or("usage: angle_cost FILE.3dcraft [TOLERANCE] [BODY]")?;
    let tolerance = match args.get(2) {
        Some(t) => t.parse::<f64>().map_err(|e| e.to_string())?,
        None => {
            let mut session = Session::new(Workspace::unrestricted());
            session
                .execute("file.open", &json!({ "path": path }))
                .map_err(|e| e.to_string())?;
            session
                .scene_with(SceneQuality::display())
                .map_err(|e| e.to_string())?
                .ok_or("no document")?
                .tolerance
        }
    };
    let only = args.get(3);
    let doc = craft3d_format::load(std::path::Path::new(path)).map_err(|e| e.to_string())?;
    let regen = craft3d_regen::regenerate(&doc);
    let plain = TessParams {
        chordal_tolerance: tolerance,
        ..TessParams::default()
    };
    let angled = TessParams {
        max_angle: Some(DISPLAY_MAX_ANGLE),
        ..plain
    };
    let (mut totals, mut seconds) = ([0usize; 2], [0.0f64; 2]);
    let mut by_kind: BTreeMap<&str, [usize; 2]> = BTreeMap::new();
    let mut faces: Vec<(usize, usize, String)> = Vec::new();
    for feature in &doc.features {
        if only.is_some_and(|name| *name != feature.name) {
            continue;
        }
        let Some(body) = regen.feature(feature.id).and_then(|r| r.own_body()) else {
            continue;
        };
        let mut meshes = Vec::new();
        for (k, params) in [plain, angled].iter().enumerate() {
            let start = Instant::now();
            match tessellate_body(body, params) {
                Ok(mesh) => meshes.push(mesh),
                Err(e) => {
                    println!("{}: {e}", feature.name);
                    break;
                }
            }
            if let (Some(s), Some(t)) = (seconds.get_mut(k), totals.get_mut(k)) {
                *s += start.elapsed().as_secs_f64();
                *t += meshes.last().map_or(0, |m| m.triangles.len());
            }
        }
        let [without, with] = meshes.as_slice() else {
            continue;
        };
        let (without, with) = (
            per_face(without, body.faces.len()),
            per_face(with, body.faces.len()),
        );
        for (i, ((face, a), b)) in body.faces.iter().zip(&without).zip(&with).enumerate() {
            let kind = by_kind.entry(face.surface.kind()).or_default();
            kind[0] += a;
            kind[1] += b;
            let name = format!("{} face {i} ({})", feature.name, face.surface.kind());
            faces.push((*b, *a, name));
        }
    }
    let [without, with] = totals;
    println!(
        "tolerance {tolerance:.4}: {without} triangles in {:.3} s without the angle, {with} in \
         {:.3} s with it ({:.2} times)",
        seconds[0],
        seconds[1],
        with as f64 / without.max(1) as f64
    );
    for (kind, [a, b]) in &by_kind {
        println!("  {kind:<14} {a:>9} {b:>9}");
    }
    faces.sort_by(|x, y| y.0.cmp(&x.0).then(x.2.cmp(&y.2)));
    for (with, without, name) in faces.iter().take(10) {
        println!("  {name}: {without} -> {with}");
    }
    // And the three that cost most of each kind.
    for kind in by_kind.keys() {
        let kind = format!("({kind})");
        for (with, without, name) in faces.iter().filter(|f| f.2.ends_with(&kind)).take(3) {
            println!("  {name}: {without} -> {with}");
        }
    }
    Ok(())
}
