//! A scene uploaded in parts and moved by poses (an assembly's motion,
//! `Renderer::upload_parts`, `Renderer::set_poses`) against the CPU
//! reference renderer drawing the same parts moved on the CPU, on every
//! adapter of this machine: the moved parts are oracle 7's parity
//! (docs/testing.md), with the tolerances of `common/mod.rs`. Near the
//! origin and ten million units away from it (positions are relative to
//! each part's centre, the poses' translations relative to the scene's).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{compare, offscreens, params, perspective, write_evidence};
use craft3d_gpu::{FaceColors, Mesh, MeshPart, drawn_bounds};
use craft3d_math::{Frame, Transform, Vec3};
use craft3d_tess::{TessParams, tessellate_body};
use craft3d_testkit::solids;

/// Three bodies in their own coordinates: a block, a cylinder, a torus.
fn bodies(scale: f64) -> Vec<Mesh> {
    let frame = Frame::new(Vec3::ZERO, Vec3::Z, Vec3::X).unwrap();
    let tess = TessParams {
        chordal_tolerance: 0.01 * scale,
        ..TessParams::default()
    };
    [
        solids::block(&frame, 4.0 * scale, 2.0 * scale, 1.0 * scale).unwrap(),
        solids::cylinder(&frame, 1.0 * scale, 3.0 * scale).unwrap(),
        solids::torus(&frame, 2.0 * scale, 0.5 * scale).unwrap(),
    ]
    .iter()
    .map(|r| tessellate_body(&r.body, &tess).unwrap())
    .collect()
}

/// `mesh` moved by `pose` on the CPU, as the engine moves a placed body's
/// mesh.
fn moved(mesh: &Mesh, pose: &Transform) -> Mesh {
    Mesh {
        positions: mesh.positions.iter().map(|&p| pose.point(p)).collect(),
        normals: mesh.normals.iter().map(|&n| pose.vector(n)).collect(),
        triangles: mesh.triangles.clone(),
        face_ids: mesh.face_ids.clone(),
    }
}

/// The parts moved and joined into one mesh, face ids counting on from
/// `first_faces`.
fn joined(parts: &[Mesh], poses: &[Transform], first_faces: &[u32]) -> Mesh {
    let mut out = Mesh::default();
    for ((part, pose), first) in parts.iter().zip(poses).zip(first_faces) {
        let part = moved(part, pose);
        let base = out.positions.len() as u32;
        out.positions.extend(&part.positions);
        out.normals.extend(&part.normals);
        out.triangles
            .extend(part.triangles.iter().map(|t| t.map(|k| k + base)));
        out.face_ids.extend(part.face_ids.iter().map(|f| f + first));
    }
    out
}

/// The poses of the moment `k`: each part turned about its own axis and
/// shifted by `origin`.
fn poses(k: usize, scale: f64, origin: Vec3) -> Vec<Transform> {
    let angle = 0.4 + 0.7 * k as f64;
    [
        (Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 0.0, 0.0)),
        (Vec3::new(1.0, 1.0, 0.0), Vec3::new(5.0, 0.0, 1.0)),
        (Vec3::new(0.3, -0.2, 1.0), Vec3::new(-2.0, 5.0, -1.0)),
    ]
    .iter()
    .enumerate()
    .map(|(i, (axis, at))| {
        let turn = Transform::rotation(*axis, angle * (i as f64 + 1.0)).unwrap();
        turn.then(&Transform::translation(*at * scale + origin))
    })
    .collect()
}

#[test]
fn parts_moved_by_poses_match_the_cpu_renderer() {
    let mut compared = 0;
    for mut offscreen in offscreens() {
        let adapter = offscreen.adapter().clone();
        for (scale, origin) in [(1.0, Vec3::ZERO), (1.0, Vec3::new(1e7, -2e7, 3e7))] {
            let parts = bodies(scale);
            let firsts: Vec<u32> = {
                let mut next = 0;
                parts
                    .iter()
                    .map(|p| {
                        let first = next;
                        next += p.face_ids.iter().copied().max().map_or(0, |m| m + 1);
                        first
                    })
                    .collect()
            };
            // One face of the torus in a colour of its own: the colours
            // go by the scene's face ids.
            let mut colors = FaceColors::new();
            assert!(colors.set(firsts[2], Some([200, 40, 30])));
            let first_poses = poses(0, scale, origin);
            let at_rest = joined(&parts, &first_poses, &firsts);
            let mesh_parts: Vec<MeshPart<'_>> = parts
                .iter()
                .zip(&first_poses)
                .zip(&firsts)
                .map(|((mesh, pose), first)| MeshPart {
                    mesh,
                    first_face_id: *first,
                    pose: *pose,
                })
                .collect();
            let mut uploaded = offscreen
                .renderer()
                .upload_parts(&mesh_parts, &colors, drawn_bounds(&at_rest))
                .unwrap_or_else(|e| panic!("on {adapter}: {e}"));
            assert_eq!(uploaded.part_count(), 3);
            for k in 0..3 {
                let poses = poses(k, scale, origin);
                let placed = joined(&parts, &poses, &firsts);
                if k > 0 {
                    // Another moment: the poses only.
                    offscreen
                        .renderer()
                        .set_poses(&mut uploaded, &poses, drawn_bounds(&placed))
                        .unwrap();
                }
                let p = params(320, 240, 2);
                let camera = perspective(&placed, Vec3::new(1.0, -1.3, 0.9), 40.0, &p);
                let cpu = craft3d_render::render_colored(&placed, &colors, &camera, &p).unwrap();
                let gpu = offscreen
                    .render_uploaded(&uploaded, &camera, &p)
                    .unwrap_or_else(|e| panic!("on {adapter}: {e}"));
                let (parity, band) = compare(&placed, &camera, &cpu, &gpu);
                let label = format!("parts moment {k} at {origin:?} on {adapter}");
                println!("{label}: {parity:?}");
                if k == 1 && origin == Vec3::ZERO {
                    let file = format!(
                        "parts-{}-{}.png",
                        adapter.backend.to_lowercase(),
                        adapter.device_type.to_lowercase()
                    );
                    println!(
                        "evidence: {}",
                        write_evidence(&file, &cpu, &gpu, &band).display()
                    );
                }
                parity.check(&label);
                compared += 1;
            }
            // A count of poses that is not the count of parts is refused.
            assert!(
                offscreen
                    .renderer()
                    .set_poses(&mut uploaded, &first_poses[..2], drawn_bounds(&at_rest))
                    .is_err()
            );
        }
    }
    assert!(compared > 0);
}

fn part_list<'a>(meshes: [&'a Mesh; 3], at: &[Transform], firsts: &[u32]) -> Vec<MeshPart<'a>> {
    meshes
        .into_iter()
        .zip(at)
        .zip(firsts)
        .map(|((mesh, pose), first)| MeshPart {
            mesh,
            first_face_id: *first,
            pose: *pose,
        })
        .collect()
}

/// Parts taken over from an earlier upload (`upload_parts_from`: another
/// moment in which one part changed shape) draw what a fresh upload of the
/// new parts draws, pixel for pixel.
#[test]
fn parts_taken_over_draw_what_a_fresh_upload_draws() {
    for mut offscreen in offscreens() {
        let adapter = offscreen.adapter().clone();
        let parts = bodies(1.0);
        let other = bodies(1.3);
        let firsts: Vec<u32> = {
            let mut next = 0;
            parts
                .iter()
                .map(|p| {
                    let first = next;
                    next += p.face_ids.iter().copied().max().map_or(0, |m| m + 1);
                    first
                })
                .collect()
        };
        let colors = FaceColors::new();
        let at = poses(1, 1.0, Vec3::ZERO);
        let before = part_list([&parts[0], &parts[1], &parts[2]], &at, &firsts);
        // The cylinder changes shape: the block and the torus stay.
        let after = part_list([&parts[0], &other[1], &parts[2]], &at, &firsts);
        let placed = joined(
            &[parts[0].clone(), other[1].clone(), parts[2].clone()],
            &at,
            &firsts,
        );
        let bounds = drawn_bounds(&placed);
        let renderer = offscreen.renderer();
        let old = renderer.upload_parts(&before, &colors, bounds).unwrap();
        let taken = renderer
            .upload_parts_from(
                &after,
                &colors,
                bounds,
                Some(old),
                &[Some(0), None, Some(2)],
            )
            .unwrap_or_else(|e| panic!("on {adapter}: {e}"));
        let fresh = renderer.upload_parts(&after, &colors, bounds).unwrap();
        assert_eq!(taken.part_count(), 3);
        assert_eq!(taken.triangle_count(), fresh.triangle_count());
        let p = params(320, 240, 2);
        let camera = perspective(&placed, Vec3::new(1.0, -1.3, 0.9), 40.0, &p);
        let a = offscreen.render_uploaded(&taken, &camera, &p).unwrap();
        let b = offscreen.render_uploaded(&fresh, &camera, &p).unwrap();
        assert!(a == b, "on {adapter}: parts taken over draw otherwise");
    }
}
