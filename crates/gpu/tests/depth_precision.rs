//! The depth buffer's precision with the eye inside a large model: a
//! building-sized scene (50 m of slabs, faces 4 cm apart) seen from a few
//! metres away, where the near plane has to sit right in front of the eye.
//! The GPU must still give every pixel the CPU renderer's face, distance
//! and colour (oracle 7, the tolerances of `common/mod.rs`), and the
//! viewport's lines must not lose their depth test against the very faces
//! they lie on. Before the depth was reversed (`scene.wgsl`) the first
//! test failed on every adapter: 574 face ids wrong outside the band on
//! Vulkan, 833 on OpenGL, 22097 on lavapipe, in stairs along the edges of
//! the slabs.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;
mod devices;

use common::{compare, offscreens, params, write_evidence};
use craft3d_gpu::{Camera, DrawOptions, Mesh, Onscreen, OverlayParams, Projection, Segment};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Vec3};
use craft3d_render::SUBPIXEL_GRAIN;
use craft3d_tess::{TessParams, tessellate_body};
use devices::devices;

/// The mesh of an axis-aligned block from `lo` to `hi`, its faces numbered
/// from `first_face`.
fn block(lo: Vec3, hi: Vec3) -> Mesh {
    let frame = Frame::new(lo, Vec3::Z, Vec3::X).unwrap();
    let size = hi - lo;
    let body = primitives::block(&frame, size.x, size.y, size.z).unwrap();
    tessellate_body(&body, &TessParams::default()).unwrap()
}

/// `parts` in one mesh, each with face ids of its own.
fn join(parts: &[Mesh]) -> Mesh {
    let mut out = Mesh::default();
    let mut faces = 0;
    for part in parts {
        let base = out.positions.len() as u32;
        out.positions.extend(&part.positions);
        out.normals.extend(&part.normals);
        out.triangles
            .extend(part.triangles.iter().map(|t| t.map(|i| i + base)));
        out.face_ids.extend(part.face_ids.iter().map(|f| f + faces));
        faces += part.face_ids.iter().copied().max().map_or(0, |m| m + 1);
    }
    out
}

/// Two storeys of a building, 50 m long, in metres: floor slabs with a
/// 4 cm screed above each, 4 cm off the slab, a row of columns, and the
/// part of the building behind the eye (see [`inside`]).
fn building() -> Mesh {
    let mut parts = Vec::new();
    for storey in 0..2 {
        let z = f64::from(storey) * 3.2;
        // The slab, and the screed 4 cm above it.
        parts.push(block(
            Vec3::new(1.0, -15.0, z),
            Vec3::new(50.0, 15.0, z + 0.3),
        ));
        parts.push(block(
            Vec3::new(1.5, -14.5, z + 0.34),
            Vec3::new(49.5, 14.5, z + 0.38),
        ));
    }
    for k in 0..8 {
        let x = 3.0 + f64::from(k) * 6.0;
        parts.push(block(Vec3::new(x, 2.5, 0.38), Vec3::new(x + 0.3, 2.8, 3.2)));
    }
    // Behind the eye: the box around the model reaches past it.
    parts.push(block(
        Vec3::new(-20.0, -15.0, 0.0),
        Vec3::new(-2.0, 15.0, 3.5),
    ));
    join(&parts)
}

/// The eye 1.7 m above the lower screed, looking along the building and
/// slightly down, with part of the building behind it: inside the box
/// around the model, so the depth range must start right in front of the
/// eye. Every triangle is wholly in front of the eye or wholly behind it,
/// because the CPU renderer does not clip at the eye plane (it skips a
/// triangle that reaches behind it), where the GPU does.
fn inside() -> Camera {
    Camera {
        eye: Vec3::new(0.0, 0.0, 2.1),
        target: Vec3::new(30.0, 0.0, 0.6),
        up: Vec3::Z,
        projection: Projection::Perspective { fov_y_deg: 50.0 },
    }
}

#[test]
fn inside_a_building_every_pixel_matches_the_cpu_renderer() {
    let mesh = building();
    let camera = inside();
    let p = params(400, 300, 1);
    let cpu = craft3d_render::render(&mesh, &camera, &p).unwrap();
    let mut failed = Vec::new();
    for mut offscreen in offscreens() {
        let adapter = offscreen.adapter().clone();
        let gpu = offscreen.render(&mesh, &camera, &p).unwrap();
        let (parity, band) = compare(&mesh, &camera, &cpu, &gpu);
        let file = format!(
            "depth-inside-building-{}-{}.png",
            adapter.backend.to_lowercase(),
            adapter.device_type.to_lowercase()
        );
        write_evidence(&file, &cpu, &gpu, &band);
        println!("{adapter}: {parity:?}");
        let label = format!("inside a building on {adapter}");
        if let Err(why) = std::panic::catch_unwind(|| parity.check(&label)) {
            failed.push(
                why.downcast_ref::<String>()
                    .cloned()
                    .unwrap_or_else(|| label.clone()),
            );
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

/// The overlay as the window draws it on the GPU: the slack of its depth
/// test is the `f32` error of the distance on numbers up to the reach (16
/// roundings of 2^-24 of the eye's distance to the model's centre plus
/// its diagonal: `craft3d_ui_egui::viewport::depth_slack`), and one
/// millionth of the distance to the target. A tenth of a millimetre here,
/// where the faces of the model are centimetres apart.
fn overlay(mesh: &Mesh, camera: &Camera) -> OverlayParams {
    let bbox = mesh.bbox();
    let reach = (camera.eye - bbox.center().unwrap()).length() + bbox.diagonal();
    let slack = reach * 16.0 / 16_777_216.0 + (camera.target - camera.eye).length() * 1e-6;
    assert!(slack < 1e-3, "{slack}");
    OverlayParams {
        slack,
        hidden_opacity: 0,
        axis_length: 1.0,
    }
}

/// The eye just inside the corner of the lower screed, looking along its
/// side, low over it: the edge is seen from a metre to 50 m, at a grazing
/// angle, with nothing in between.
fn along_the_edge() -> Camera {
    Camera {
        eye: Vec3::new(0.5, 13.5, 1.0),
        target: Vec3::new(25.0, 14.5, 0.38),
        up: Vec3::Z,
        projection: Projection::Perspective { fov_y_deg: 50.0 },
    }
}

/// Lines on the lower screed's top face, as the viewport draws a body's
/// edges and a sketch on a face: tested against the depth buffer, lying
/// on the very face under them. One is the side edge of the face, one
/// runs across its middle (like a sketch line, or the edge between two
/// faces in one plane), where all the pixels around it show the face
/// itself. Every pixel a line inks without the test must be inked with it
/// too, with the slack the window gives the test ([`overlay`]). A depth
/// buffer too coarse at this distance puts the face in front of its own
/// lines and they come out dashed. (Only lines that nothing stands in
/// front of: the far side of the face runs behind the columns.) This one
/// passed with the depth not yet reversed as well, because the spread of
/// the nine pixels the test allowed then absorbed the coarse steps on one face;
/// it holds the turning of the reversed depth back into a distance
/// (`depth_test.wgsl`, `surface_at`) to the window's slack.
#[test]
fn inside_a_building_lines_on_the_faces_stay_whole() {
    let mesh = building();
    let camera = along_the_edge();
    let p = params(400, 300, 1);
    let lines = [
        [Vec3::new(1.5, 14.5, 0.38), Vec3::new(49.5, 14.5, 0.38)],
        [Vec3::new(1.5, 9.0, 0.38), Vec3::new(49.5, 9.0, 0.38)],
        [Vec3::new(1.0, 15.0, 0.3), Vec3::new(50.0, 15.0, 0.3)],
        [Vec3::new(1.0, 14.75, 0.3), Vec3::new(50.0, 14.75, 0.3)],
    ];
    let yellow = [255, 196, 64];
    let edges = |tested: bool| -> Vec<Segment> {
        lines
            .iter()
            .map(|&[from, to]| Segment {
                from,
                to,
                color: yellow,
                width: 3.0,
                tested,
                dimmed: false,
                axis: false,
            })
            .collect()
    };
    let mut failed = Vec::new();
    for (adapter, device, queue) in devices() {
        let name = format!(
            "{} ({:?})",
            adapter.get_info().name,
            adapter.get_info().backend
        );
        let mut onscreen = Onscreen::new(&device, &queue, &adapter).unwrap();
        let uploaded = onscreen.upload(&mesh).unwrap();
        let mut inked = Vec::new();
        let mut pictures = Vec::new();
        for tested in [false, true] {
            let lines = onscreen.upload_lines(&edges(tested)).unwrap();
            onscreen
                .draw(
                    &uploaded,
                    &lines,
                    &camera,
                    &p,
                    &overlay(&mesh, &camera),
                    DrawOptions::default(),
                )
                .unwrap();
            let rgb = onscreen.read_rgb().unwrap();
            inked.push(rgb.chunks(3).map(|px| px == yellow).collect::<Vec<bool>>());
            pictures.push(rgb);
        }
        let backend = format!("{:?}", adapter.get_info().backend).to_lowercase();
        let kind = format!("{:?}", adapter.get_info().device_type).to_lowercase();
        let mut picture = Vec::new();
        for y in 0..p.height as usize {
            for row in &pictures {
                picture.extend_from_slice(
                    &row[y * p.width as usize * 3..(y + 1) * p.width as usize * 3],
                );
            }
        }
        let png = craft3d_render::encode_png(p.width * 2, p.height, &picture).unwrap();
        std::fs::write(
            common::evidence(&format!("depth-inside-building-lines-{backend}-{kind}.png")),
            png,
        )
        .unwrap();
        let (all, kept) = (&inked[0], &inked[1]);
        let drawn = all.iter().filter(|&&b| b).count();
        let lost = all.iter().zip(kept).filter(|&(&a, &k)| a && !k).count();
        let w = p.width as usize;
        let at: Vec<String> = all
            .iter()
            .zip(kept)
            .enumerate()
            .filter(|&(_, (&a, &k))| a && !k)
            .map(|(i, _)| format!("({}, {})", i % w, i / w))
            .collect();
        println!(
            "{name}: {drawn} pixels of line, {lost} of them hidden by their own face {}",
            at.join(" ")
        );
        assert!(drawn > 300, "{name}: the lines are not in the picture");
        if lost > 0 {
            failed.push(format!(
                "{name}: {lost} of {drawn} pixels of line are hidden by their own face"
            ));
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

/// The depth buffer as the lines' depth test reads it back
/// (`Onscreen::read_depth`), against the distance the very same fragment
/// wrote to the data target: at every pixel the two agree within the depth
/// test's own allowance (the window's slack, [`overlay`], plus the
/// surface's change over `SUBPIXEL_GRAIN` of a pixel, here taken over the
/// pixel's four neighbours, which covers how the rasteriser places its
/// depth against its attributes). At one sample per pixel, so that the
/// depth read is the centre's, where the capture is (with several, a pixel
/// whose samples straddle two surfaces reads their mean). A depth buffer
/// that loses the key on its way through the backend fails this at the
/// far end of the screed, where the key is a millionth: before the
/// fragment wrote its own depth on OpenGL (`scene_source`), a Radeon 8060S
/// put every pixel 2 to 3 % nearer than its surface (a metre at 43 m) and
/// an Intel MTL up to 0.6 m.
#[test]
fn inside_a_building_the_depth_buffer_holds_the_fragments_distance() {
    let mesh = building();
    let camera = along_the_edge();
    let p = params(400, 300, 1);
    let slack = overlay(&mesh, &camera).slack;
    let mut failed = Vec::new();
    for (adapter, device, queue) in devices() {
        let name = format!(
            "{} ({:?})",
            adapter.get_info().name,
            adapter.get_info().backend
        );
        let mut onscreen = Onscreen::with_samples(&device, &queue, 1).unwrap();
        let uploaded = onscreen.upload(&mesh).unwrap();
        let lines = onscreen.upload_lines(&[]).unwrap();
        onscreen
            .draw(
                &uploaded,
                &lines,
                &camera,
                &p,
                &overlay(&mesh, &camera),
                DrawOptions {
                    capture: true,
                    ..DrawOptions::default()
                },
            )
            .unwrap();
        let frame = onscreen.read_frame().unwrap();
        let depth = onscreen.read_depth().unwrap();
        assert_eq!((depth.width, depth.height), (p.width, p.height));
        let (w, h) = (p.width as usize, p.height as usize);
        let at = |x: usize, y: usize| -> Option<f64> {
            let d = frame.depth[y * w + x];
            d.is_finite().then_some(d)
        };
        let (mut compared, mut over, mut worst) = (0, 0, (0.0f64, 0usize));
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                let i = y * w + x;
                let (Some(written), Some(read)) = (at(x, y), depth.distance(i)) else {
                    continue;
                };
                // The allowance needs the neighbours; at the outline of a
                // face there is none and the depth test hides nothing.
                let neighbours = [at(x - 1, y), at(x + 1, y), at(x, y - 1), at(x, y + 1)];
                let Some(change) = neighbours
                    .iter()
                    .map(|n| n.map(|n| (n - written).abs()))
                    .try_fold(0.0f64, |m, n| n.map(|n| m.max(n)))
                else {
                    continue;
                };
                compared += 1;
                let error = (f64::from(read) - written).abs();
                if error > worst.0 {
                    worst = (error, i);
                }
                if error > slack + change * SUBPIXEL_GRAIN {
                    over += 1;
                }
            }
        }
        println!(
            "{name}: {compared} pixels compared, {over} beyond the allowance, worst {:.3} mm at ({}, {}) where the fragment wrote {:.4} m",
            worst.0 * 1e3,
            worst.1 % w,
            worst.1 / w,
            frame.depth[worst.1]
        );
        assert!(compared > 50_000, "{name}: {compared} pixels compared");
        if over > 0 {
            failed.push(format!(
                "{name}: {over} of {compared} pixels of the depth buffer lie beyond the depth test's allowance from their fragment's distance"
            ));
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}
