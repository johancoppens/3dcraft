//! The section cap sample by sample: with several samples per pixel the
//! count that decides the cap (`cut.rs`, *The count*) is per sample, so
//! the cap of the window's renderer has no lone samples missing in a flat
//! stretch of it (2026-10-19, docs/design/rendering.md › *Specks in the
//! GPU's cap*), and at one sample per pixel the cap is the CPU renderer's
//! at every pixel, also where the sample lies exactly on a shared edge.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;
mod devices;

use common::{params, perspective};
use craft3d_gpu::{
    Camera, DrawOptions, Mesh, Onscreen, OverlayParams, Projection, RenderParams, SECTION_FACE,
    Section,
};
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_tess::{TessParams, tessellate_body};
use devices::devices;

const CAP: [u8; 3] = [214, 132, 74];

fn at(x: f64, y: f64, z: f64) -> Frame {
    Frame::new(Vec3::new(x, y, z), Vec3::Z, Vec3::X).unwrap()
}

macro_rules! mesh_of {
    ($body:expr, $tolerance:expr $(,)?) => {
        tessellate_body(
            $body,
            &TessParams {
                chordal_tolerance: $tolerance,
                ..TessParams::default()
            },
        )
        .unwrap()
    };
}

/// The part of `crates/ui-egui/tests/section.rs`: a block with a ball
/// taken out of its middle. Cut through the ball's centre, the removed
/// half of the cavity's wall is seen edge-on along its silhouette, in
/// sliver triangles whose distance from the plane changes sign within a
/// fraction of a pixel of their edges.
fn hollow() -> Mesh {
    let block = primitives::block(&at(0.0, 0.0, 0.0), 60.0, 40.0, 20.0).unwrap();
    let ball = primitives::sphere(&at(30.0, 20.0, 10.0), 7.0).unwrap();
    let body = boolean(&block, &ball, BooleanOp::Subtract, &Tolerance::default()).unwrap();
    mesh_of!(&body.body, 0.004)
}

fn section(origin: Vec3, normal: Vec3) -> Section {
    Section {
        origin,
        normal,
        cap: true,
        color: CAP,
    }
}

fn overlay() -> OverlayParams {
    OverlayParams {
        slack: 1e-5,
        hidden_opacity: 0,
        axis_length: 1.0,
        edges: craft3d_gpu::EdgeLook::default(),
    }
}

/// Pixels where `oracle` has the cap on the pixel and two all around
/// (clear of the cut's outline) and `picture` does not: the rule of
/// `crates/ui-egui/tests/section.rs`.
fn specks(w: usize, h: usize, oracle: &[bool], picture: &[bool]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for y in 2..h - 2 {
        for x in 2..w - 2 {
            let all = (-2_i64..=2).all(|dy| {
                (-2_i64..=2)
                    .all(|dx| oracle[(y as i64 + dy) as usize * w + (x as i64 + dx) as usize])
            });
            if all && !picture[y * w + x] {
                out.push((x, y));
            }
        }
    }
    out
}

/// The cut-open views of [`hollow`] this is pinned on: before the count
/// was per sample, the Intel MTL (Vulkan, four samples) and lavapipe
/// missed one to five samples of the cap in 22 of these 24 views (30 in
/// all, blended with the cavity's wall behind), 4 to 8 pixels from the
/// cut's outline, at the same pixels on both.
fn views() -> Vec<(String, Section, Vec3, (u32, u32))> {
    let centre = Vec3::new(30.0, 20.0, 10.0);
    let mut out = Vec::new();
    for (name, normal) in [("z", Vec3::Z), ("x", Vec3::X)] {
        for (k, toward) in [
            Vec3::new(0.5, -0.7, 0.6),
            Vec3::new(0.3, -0.9, 0.5),
            Vec3::new(0.7, -0.4, 0.65),
            Vec3::new(1.0, -1.0, 1.0),
        ]
        .into_iter()
        .enumerate()
        {
            for size in [(740, 654), (800, 600), (1000, 700)] {
                out.push((
                    format!("{name} view {k} {}x{}", size.0, size.1),
                    section(centre, normal),
                    toward,
                    size,
                ));
            }
        }
    }
    out
}

/// The window's renderer (several samples where the adapter has them)
/// against the CPU oracle at one sample: no pixel of a flat stretch of
/// the cap lacks the cap's colour.
#[test]
fn the_caps_samples_are_counted_at_their_own_point() {
    let mesh = hollow();
    let mut failed = Vec::new();
    for (adapter, device, queue) in devices() {
        let name = format!(
            "{} ({:?})",
            adapter.get_info().name,
            adapter.get_info().backend
        );
        let mut onscreen = Onscreen::new(&device, &queue, &adapter).unwrap();
        let uploaded = onscreen.upload(&mesh).unwrap();
        let lines = onscreen.upload_lines(&[]).unwrap();
        let mut total = 0;
        for (view, section, toward, (w, h)) in views() {
            let p = RenderParams {
                section: Some(section),
                ..params(w, h, 1)
            };
            let camera = perspective(&mesh, toward, 40.0, &p);
            let cpu = craft3d_render::render(&mesh, &camera, &p).unwrap();
            onscreen
                .draw(
                    &uploaded,
                    &lines,
                    &camera,
                    &p,
                    &overlay(),
                    DrawOptions::default(),
                )
                .unwrap();
            let rgb = onscreen.read_rgb().unwrap();
            let oracle: Vec<bool> = cpu.ids.iter().map(|&id| id == SECTION_FACE).collect();
            let picture: Vec<bool> = rgb.chunks(3).map(|px| px == CAP).collect();
            let found = specks(w as usize, h as usize, &oracle, &picture);
            if !found.is_empty() {
                let at: Vec<String> = found
                    .iter()
                    .map(|&(x, y)| {
                        let i = (y * w as usize + x) * 3;
                        format!("({x}, {y}) {:?}", &rgb[i..i + 3])
                    })
                    .collect();
                failed.push(format!(
                    "{name}: {view}: {} pixels of the cap missing: {}",
                    found.len(),
                    at.join(" ")
                ));
            }
            total += oracle.iter().filter(|&&c| c).count();
        }
        println!("{name}: {total} cap pixels over {} views", views().len());
        assert!(total > 100_000, "{name}: {total} cap pixels");
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

/// At one sample per pixel, four times the resolution (so that the
/// standard sample positions of a multisampled pixel are pixel centres
/// here), the GPU's cap is the CPU renderer's pixel for pixel, clear of
/// the cut's outline, in either direction.
#[test]
fn at_one_sample_the_cap_is_the_cpu_renderers_pixel_for_pixel() {
    let mesh = hollow();
    let mut failed = Vec::new();
    let chosen: Vec<_> = views()
        .into_iter()
        .filter(|(view, ..)| view == "z view 1 740x654" || view == "x view 0 800x600")
        .collect();
    assert_eq!(chosen.len(), 2);
    for (adapter, device, queue) in devices() {
        let name = format!(
            "{} ({:?})",
            adapter.get_info().name,
            adapter.get_info().backend
        );
        let mut onscreen = Onscreen::with_samples(&device, &queue, 1).unwrap();
        let uploaded = onscreen.upload(&mesh).unwrap();
        let lines = onscreen.upload_lines(&[]).unwrap();
        for (view, section, toward, (w, h)) in &chosen {
            let p = RenderParams {
                section: Some(*section),
                ..params(*w, *h, 1)
            };
            let camera = perspective(&mesh, *toward, 40.0, &p);
            let p4 = RenderParams {
                width: w * 4,
                height: h * 4,
                ..p
            };
            let cpu = craft3d_render::render(&mesh, &camera, &p4).unwrap();
            onscreen
                .draw(
                    &uploaded,
                    &lines,
                    &camera,
                    &p4,
                    &overlay(),
                    DrawOptions {
                        capture: true,
                        ..DrawOptions::default()
                    },
                )
                .unwrap();
            let gpu = onscreen.read_frame().unwrap();
            let oracle: Vec<bool> = cpu.ids.iter().map(|&id| id == SECTION_FACE).collect();
            let picture: Vec<bool> = gpu.ids.iter().map(|&id| id == SECTION_FACE).collect();
            let (w4, h4) = (*w as usize * 4, *h as usize * 4);
            let missing = specks(w4, h4, &oracle, &picture);
            let extra = specks(w4, h4, &picture, &oracle);
            println!(
                "{name}: {view} at {w4}x{h4}: {} cap pixels, {} missing, {} extra",
                oracle.iter().filter(|&&c| c).count(),
                missing.len(),
                extra.len()
            );
            if !missing.is_empty() || !extra.is_empty() {
                failed.push(format!(
                    "{name}: {view}: {} pixels of the cap missing, {} extra: {:?} {:?}",
                    missing.len(),
                    extra.len(),
                    &missing[..missing.len().min(8)],
                    &extra[..extra.len().min(8)]
                ));
            }
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

/// A sample exactly on an edge two triangles share is counted once: a
/// block seen straight from above, framed so that the diagonal of its top
/// face runs through the centres of the pixels along it, cut through its
/// middle. The cap is whole on both renderers at one sample per pixel,
/// where the sample is the centre; with the window's renderer (several
/// samples where the adapter has them) the cap is whole too, though its
/// samples lie off the diagonal.
#[test]
fn a_sample_on_a_shared_edge_is_counted_once() {
    // A 10 × 10 × 4 block; the picture 200 × 200 pixels of a 20 × 20
    // view, so that the block's corners land on pixel corners 50 pixels
    // from the picture's centre and its diagonal passes through the
    // centres of the pixels (i + 0.5, i + 0.5) along it.
    let block = primitives::block(&at(-5.0, -5.0, -2.0), 10.0, 10.0, 4.0).unwrap();
    let mesh = mesh_of!(&block, 0.01);
    let camera = Camera {
        eye: Vec3::new(0.0, 0.0, 30.0),
        target: Vec3::ZERO,
        up: Vec3::Y,
        projection: Projection::Orthographic { half_height: 10.0 },
    };
    let p = RenderParams {
        section: Some(section(Vec3::ZERO, Vec3::Z)),
        ..params(200, 200, 1)
    };
    let cpu = craft3d_render::render(&mesh, &camera, &p).unwrap();
    // The cap over the whole block but its outline.
    let inside = |x: usize, y: usize| (52..148).contains(&x) && (52..148).contains(&y);
    let cap_pixels = cpu.ids.iter().filter(|&&id| id == SECTION_FACE).count();
    assert!(cap_pixels > 9_000, "{cap_pixels}");
    for y in 0..200 {
        for x in 0..200 {
            if inside(x, y) {
                assert_eq!(cpu.ids[y * 200 + x], SECTION_FACE, "CPU at {x}, {y}");
            }
        }
    }
    for (adapter, device, queue) in devices() {
        let name = format!(
            "{} ({:?})",
            adapter.get_info().name,
            adapter.get_info().backend
        );
        // With one sample, and with the window's renderer (several where
        // the adapter has them).
        for samples in ["one", "the window's"] {
            let mut onscreen = if samples == "one" {
                Onscreen::with_samples(&device, &queue, 1).unwrap()
            } else {
                Onscreen::new(&device, &queue, &adapter).unwrap()
            };
            let uploaded = onscreen.upload(&mesh).unwrap();
            let lines = onscreen.upload_lines(&[]).unwrap();
            onscreen
                .draw(
                    &uploaded,
                    &lines,
                    &camera,
                    &p,
                    &overlay(),
                    DrawOptions::default(),
                )
                .unwrap();
            let rgb = onscreen.read_rgb().unwrap();
            let mut missing = Vec::new();
            for y in 0..200 {
                for x in 0..200 {
                    let i = (y * 200 + x) * 3;
                    if inside(x, y) && rgb[i..i + 3] != CAP {
                        missing.push((x, y, [rgb[i], rgb[i + 1], rgb[i + 2]]));
                    }
                }
            }
            assert!(
                missing.is_empty(),
                "{name}, {samples} samples: {} pixels of the cap missing: {:?}",
                missing.len(),
                &missing[..missing.len().min(10)]
            );
        }
    }
}
