//! The review of the hatch's direction per body (`check/hatch-body`), by
//! `review-ux-render`, on the reference renderer: hostile runs of classes
//! (face ids beyond the mesh, the last id, the same first face twice,
//! 10 000 runs set backwards) never panic and draw the same twice; and
//! what the classes cost a picture of 400 touching parts. Findings are
//! printed as `REVIEW` lines.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_kernel::primitives;
use craft3d_math::{Frame, Vec3};
use craft3d_render::{
    Camera, FaceColors, Hatch, Projection, RenderParams, SECTION_FACE, Section, render_colored,
};
use craft3d_tess::{Mesh, TessParams, tessellate_body};
use std::time::Instant;

const HATCH: Hatch = Hatch {
    ink: [88, 84, 78],
    spacing_points: 9.0,
    width_points: 1.0,
    height_points: None,
};

fn block(x: f64, y: f64, z: f64, d: f64) -> Mesh {
    let frame = Frame::new(Vec3::new(x, y, z), Vec3::Z, Vec3::X).unwrap();
    let body = primitives::block(&frame, d, d, d).unwrap();
    tessellate_body(
        &body,
        &TessParams {
            chordal_tolerance: 0.01,
            ..TessParams::default()
        },
    )
    .unwrap()
}

fn combine(parts: &[Mesh]) -> (Mesh, Vec<u32>) {
    let mut mesh = Mesh::default();
    let mut firsts = Vec::new();
    let mut faces = 0_u32;
    for part in parts {
        let base = mesh.positions.len() as u32;
        firsts.push(faces);
        mesh.positions.extend(&part.positions);
        mesh.normals.extend(&part.normals);
        mesh.triangles
            .extend(part.triangles.iter().map(|t| t.map(|i| i + base)));
        mesh.face_ids
            .extend(part.face_ids.iter().map(|f| f + faces));
        faces += part.face_ids.iter().copied().max().map_or(0, |m| m + 1);
    }
    (mesh, firsts)
}

/// `n` × `n` blocks of 1 in a grid in x and z, each touching its
/// neighbours, and the first face of each.
fn grid(n: u32) -> (Mesh, Vec<u32>) {
    let parts: Vec<Mesh> = (0..n * n)
        .map(|k| block(f64::from(k % n), 0.0, f64::from(k / n), 1.0))
        .collect();
    combine(&parts)
}

fn picture(n: u32, width: u32, height: u32, ssaa: u32) -> (Camera, RenderParams) {
    let c = f64::from(n) * 0.5;
    let camera = Camera {
        eye: Vec3::new(
            c + 0.3 * f64::from(n),
            -2.0 * f64::from(n),
            c + 0.4 * f64::from(n),
        ),
        target: Vec3::new(c, 0.5, c),
        up: Vec3::Z,
        projection: Projection::Perspective { fov_y_deg: 40.0 },
    };
    let params = RenderParams {
        width,
        height,
        ssaa,
        section: Some(Section {
            origin: Vec3::new(c, 0.5, c),
            normal: -Vec3::Y,
            cap: true,
            color: [232, 228, 220],
            hatch: Some(HATCH),
        }),
        ..RenderParams::default()
    };
    (camera, params)
}

/// Runs of classes no engine makes: before and beyond every face of the
/// mesh, at `u32::MAX`, a class of `usize::MAX`, the same first face set
/// twice, 10 000 runs set from the last to the first: every picture an
/// answer, the same twice, and a cap drawn.
#[test]
fn hostile_runs_of_classes_never_panic() {
    let (mesh, firsts) = grid(4);
    let (camera, params) = picture(4, 160, 120, 2);
    let mut cases: Vec<(&str, FaceColors)> = Vec::new();
    let mut c = FaceColors::new();
    for (k, &f) in firsts.iter().enumerate() {
        c.set_hatch_class(f, k);
    }
    cases.push(("each part its place", c.clone()));
    let mut odd = c.clone();
    odd.set_hatch_class(u32::MAX, usize::MAX);
    odd.set_hatch_class(1_000_000, 3);
    odd.set_hatch_class(firsts[3], 2);
    odd.set_hatch_class(firsts[3], 9);
    odd.set_hatch_class(firsts[5] + 2, 1);
    cases.push(("beyond, at the last id, set twice, mid-part", odd));
    let mut backwards = FaceColors::new();
    let started = Instant::now();
    for k in (0..10_000_u32).rev() {
        assert!(backwards.set_hatch_class(k * 3, k as usize));
    }
    let set_ms = started.elapsed().as_secs_f64() * 1000.0;
    cases.push(("10 000 runs set backwards", backwards));
    for (name, colors) in &cases {
        let a = render_colored(&mesh, colors, &camera, &params).unwrap();
        let b = render_colored(&mesh, colors, &camera, &params).unwrap();
        assert_eq!(a, b, "{name}");
        let caps = a.ids.iter().filter(|&&id| id == SECTION_FACE).count();
        assert!(caps > 100, "{name}: {caps}");
        eprintln!("REVIEW hostile classes: {name}: {caps} cap pixels, the same twice");
    }
    eprintln!("REVIEW hostile classes: 10 000 runs set backwards in {set_ms:.1} ms");
}

/// What the classes cost the reference renderer: 400 touching blocks (20 ×
/// 20) cut through the middle, 800 × 600 with two samples a side, without
/// classes and with each part of the class of its place, five times each
/// in turn; the medians.
#[test]
fn what_the_classes_cost_the_reference_renderer() {
    let (mesh, firsts) = grid(20);
    let (camera, params) = picture(20, 800, 600, 2);
    let none = FaceColors::new();
    let mut classed = FaceColors::new();
    for (k, &f) in firsts.iter().enumerate() {
        classed.set_hatch_class(f, k);
    }
    let _ = render_colored(&mesh, &none, &camera, &params).unwrap();
    let mut times = [Vec::new(), Vec::new()];
    for _ in 0..5 {
        for (k, colors) in [&none, &classed].into_iter().enumerate() {
            let started = Instant::now();
            let _ = render_colored(&mesh, colors, &camera, &params).unwrap();
            times[k].push(started.elapsed().as_secs_f64() * 1000.0);
        }
    }
    for t in &mut times {
        t.sort_by(f64::total_cmp);
    }
    eprintln!(
        "REVIEW cpu cost: 400 parts, {} triangles, 800 x 600 ssaa 2: no classes {:.1} ms, \
         four classes {:.1} ms (medians of 5)",
        mesh.triangles.len(),
        times[0][2],
        times[1][2]
    );
}
