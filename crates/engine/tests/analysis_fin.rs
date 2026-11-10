//! The curvature map of the guide's G2 fin (docs/guide/features.md,
//! "Curvature-continuous seams") against the exact surface between the
//! vertices: the map interpolates a value per vertex, so the mesh must be
//! fine enough where the curvature changes for what it shows.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{new_document, run, session_in, workdir};
use craft3d_engine::Session;
use craft3d_engine::analysis::{CurvatureKind, refined_map, vertex_values};
use serde_json::{Value, json};

fn fin(s: &mut Session, kind: &str) {
    for (cmd, params) in [
        (
            "feature.cylinder",
            json!({"radius": "20 mm", "height": "30 mm", "name": "Post"}),
        ),
        ("sketch.create", json!({"name": "Seam", "plane": "zx"})),
        (
            "sketch.line",
            json!({"sketch": "Seam", "from": [0, 20], "to": [30, 20]}),
        ),
        (
            "sketch.create",
            json!({"name": "Far", "plane": "yz", "offset": "35 mm"}),
        ),
        (
            "sketch.line",
            json!({"sketch": "Far", "from": [25, 0], "to": [25, 30]}),
        ),
        ("sketch.create", json!({"name": "Bottom"})),
        (
            "sketch.spline",
            json!({"sketch": "Bottom", "through": [[20, 0], [23, 6.5], [27, 13], [35, 25]]}),
        ),
        ("sketch.create", json!({"name": "Top", "offset": "30 mm"})),
        (
            "sketch.spline",
            json!({"sketch": "Top", "through": [[20, 0], [23, 6.5], [27, 13], [35, 25]]}),
        ),
        (
            "feature.surface.network",
            json!({"u": ["Seam", "Far"], "v": ["Bottom", "Top"], "edges": {"uFirst": {"kind": kind, "face": "F1:face(0)"}}, "name": "Fin"}),
        ),
    ] {
        run(s, cmd, params);
    }
}

fn mean_at(s: &mut Session, p: [f64; 3]) -> f64 {
    let out: Value = run(s, "measure.curvature", json!({"of": "Fin", "at": p}));
    out.get("mean")
        .or_else(|| out.get("detail").and_then(|d| d.get("mean")))
        .and_then(Value::as_f64)
        .unwrap_or_else(|| panic!("no mean curvature in {out}"))
}

/// The largest difference, over the fin's triangles, between the map's
/// value at a triangle's centroid (the mean of its corners', as the
/// rasteriser interpolates it) and the exact mean curvature there, as a
/// share of the largest |H| on the fin; with the triangle count.
fn map_error(kind: &str, refined: bool) -> (f64, usize, f64) {
    let mut s = session_in(&workdir("analysis-fin", &format!("{kind}-{refined}")));
    new_document(&mut s, 1.0);
    fin(&mut s, kind);
    let sc = s.scene(None).unwrap().unwrap();
    let (mesh, values) = if refined {
        refined_map(&mut s, &sc, CurvatureKind::Mean).unwrap()
    } else {
        (
            (*sc.mesh).clone(),
            vertex_values(&mut s, &sc, CurvatureKind::Mean).unwrap(),
        )
    };
    assert_eq!(mesh.positions.len(), values.len());
    let fin_body = sc.bodies.iter().find(|b| b.feature != "F1").unwrap();
    let first = fin_body.first_face_id;
    let mesh = &mesh;
    let tris: Vec<[u32; 3]> = mesh
        .triangles
        .iter()
        .zip(&mesh.face_ids)
        .filter(|(_, f)| **f >= first && **f < first + 1)
        .map(|(t, _)| *t)
        .collect();
    let largest = tris
        .iter()
        .flat_map(|t| t.iter())
        .map(|k| f64::from(values[*k as usize]).abs())
        .fold(0.0_f64, f64::max);
    let mut worst = 0.0_f64;
    let mut worst_at = [0.0; 3];
    for t in &tris {
        let c = t
            .iter()
            .map(|k| mesh.positions[*k as usize])
            .fold(craft3d_math::Vec3::ZERO, |a, p| a + p)
            * (1.0 / 3.0);
        let shown: f64 = t
            .iter()
            .map(|k| f64::from(values[*k as usize]).abs())
            .sum::<f64>()
            / 3.0;
        let exact = mean_at(&mut s, [c.x, c.y, c.z]).abs();
        let e = (shown - exact).abs() / largest;
        if e > worst {
            worst = e;
            worst_at = [c.x, c.y, c.z];
        }
    }
    println!(
        "{kind} (refined {refined}): {} triangles on the fin, largest |H| {largest:.5}, worst map error {worst:.4} of it at {worst_at:?}, scene tolerance {}",
        tris.len(),
        sc.tolerance
    );
    (worst, tris.len(), largest)
}

/// The scene's mesh of the G2 fin is too coarse for its map (measured
/// 2026-10-30: 28 triangles, a quarter of the largest |H| off at a
/// centroid); refined for the analysis (`refined_map`), every centroid is
/// within 5 % of it.
#[test]
fn the_fins_map_follows_the_exact_curvature() {
    for kind in ["curvature", "tangent"] {
        let (coarse, _, _) = map_error(kind, false);
        let (fine, triangles, _) = map_error(kind, true);
        println!(
            "{kind}: map error {coarse:.4} on the scene's mesh, {fine:.4} refined ({triangles} triangles)"
        );
        assert!(fine <= 0.05, "{kind}: {fine}");
        assert!(fine <= coarse, "{kind}: {fine} > {coarse}");
    }
}
