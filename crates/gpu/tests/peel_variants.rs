//! The two forms of the multisampled see-through and section stages
//! (`craft3d_gpu::diag::PeelVariant`: masked, and per sample, which a
//! Vulkan driver on Windows gets; docs/design/rendering.md, *Per-sample
//! stages on Vulkan drivers on Windows*) against each other, on every adapter
//! of this machine: see-through, cut open, and both, of a hollow block and
//! a drilled plate. The per-sample picture is held to the CPU renderer by
//! `see_through.rs`, `section.rs`, `onscreen.rs` and `cap_specks.rs` run
//! with `CRAFT3D_PEEL_VARIANT=persample`; here it must draw, and how far it
//! is from the masked one, pixel for pixel, is written to
//! `target/evidence/gpu/peel-variants.txt`. A second test, run by hand,
//! times both forms on a model of a few hundred thousand triangles.
//!
//! These tests need a graphics adapter and fail, with what to install,
//! when there is none; they do not skip.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;
mod devices;

use common::{evidence, params, perspective};
use craft3d_gpu::diag::{PeelSetting, PeelVariant};
use craft3d_gpu::{
    DrawOptions, Frame, Highlight, Mesh, NO_FACE, Onscreen, OverlayParams, RenderParams, Section,
};
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::primitives;
use craft3d_math::{Tolerance, Vec3};
use craft3d_tess::{TessParams, tessellate_body};
use devices::devices;

fn at(x: f64, y: f64, z: f64) -> craft3d_math::Frame {
    craft3d_math::Frame::new(Vec3::new(x, y, z), Vec3::Z, Vec3::X).unwrap()
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

/// A block with a closed cavity in it.
fn hollow_block() -> Mesh {
    let outer = primitives::block(&at(-3.0, -2.0, -1.5), 6.0, 4.0, 3.0).unwrap();
    let inner = primitives::block(&at(-2.0, -1.0, -1.0), 4.0, 2.0, 2.0).unwrap();
    let hollow = boolean(&outer, &inner, BooleanOp::Subtract, &Tolerance::default()).unwrap();
    mesh_of!(&hollow.body, 0.01)
}

/// A plate with a hole drilled through it.
fn drilled_plate() -> Mesh {
    let plate = primitives::block(&at(-3.0, -2.0, -0.5), 6.0, 4.0, 1.0).unwrap();
    let drill = primitives::cylinder(&at(0.6, 0.3, -2.0), 1.1, 4.0).unwrap();
    let drilled = boolean(&plate, &drill, BooleanOp::Subtract, &Tolerance::default()).unwrap();
    mesh_of!(&drilled.body, 0.004)
}

fn no_overlay() -> OverlayParams {
    OverlayParams {
        slack: 0.0,
        hidden_opacity: 70,
        axis_length: 1.0,
        edges: craft3d_gpu::EdgeLook::default(),
    }
}

/// `params` see-through, cut open with a plane through `mesh`, and both.
fn kinds(mesh: &Mesh, base: RenderParams) -> Vec<(&'static str, RenderParams)> {
    let (lo, hi) = mesh.bbox().corners().unwrap();
    let section = Section {
        origin: (lo + hi) * 0.5 + (hi - lo) * 0.07,
        normal: Vec3::new(0.5, -0.7, 0.6),
        cap: true,
        color: [214, 132, 74],
    };
    [
        ("see-through", 0.45, None),
        ("cut", 1.0, Some(section)),
        ("see-through-cut", 0.45, Some(section)),
    ]
    .into_iter()
    .map(|(kind, opacity, section)| {
        (
            kind,
            RenderParams {
                opacity,
                section,
                highlight: Highlight::new([255, 150, 40], [1]).0,
                ..base
            },
        )
    })
    .collect()
}

/// Pixels whose colour, id or distance differ, and the largest difference
/// of a colour channel.
fn differences(masked: &Frame, got: &Frame) -> (usize, u8) {
    assert_eq!((masked.width, masked.height), (got.width, got.height));
    let mut pixels = 0;
    let mut largest = 0_u8;
    for i in 0..masked.ids.len() {
        let (a, b) = (&masked.rgb[i * 3..i * 3 + 3], &got.rgb[i * 3..i * 3 + 3]);
        let channel = a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap();
        if channel > 0
            || masked.ids[i] != got.ids[i]
            || masked.depth[i].to_bits() != got.depth[i].to_bits()
        {
            pixels += 1;
            largest = largest.max(channel);
        }
    }
    (pixels, largest)
}

const TOWARD: Vec3 = Vec3::new(0.8, -0.5, 1.0);

#[test]
fn both_forms_draw_and_how_far_apart_they_are_is_said() {
    let models = [("hollow", hollow_block()), ("drilled", drilled_plate())];
    let options = DrawOptions {
        bodies: true,
        capture: true,
    };
    let mut log = String::new();
    for (adapter, device, queue) in devices() {
        let info = adapter.get_info();
        let adapter_name = format!("{} ({:?})", info.name, info.backend);
        let draw = |variant: PeelVariant| -> (u32, PeelVariant, Vec<(String, Frame)>) {
            let setting = PeelSetting::Fixed(variant);
            let mut onscreen =
                Onscreen::with_peel_setting(&device, &queue, &adapter, setting).unwrap();
            let lines = onscreen.upload_lines(&[]).unwrap();
            let mut frames = Vec::new();
            for (name, mesh) in &models {
                let uploaded = onscreen.upload(mesh).unwrap();
                for (kind, p) in kinds(mesh, params(320, 240, 1)) {
                    let camera = perspective(mesh, TOWARD, 40.0, &p);
                    onscreen
                        .draw(&uploaded, &lines, &camera, &p, &no_overlay(), options)
                        .unwrap_or_else(|e| {
                            panic!("{name} {kind}, {variant} on {adapter_name}: {e}")
                        });
                    frames.push((format!("{name}-{kind}"), onscreen.read_frame().unwrap()));
                }
            }
            (onscreen.samples(), onscreen.peel_variant(), frames)
        };
        let (samples, _, masked) = draw(PeelVariant::Masked);
        let (_, taken, per_sample) = draw(PeelVariant::PerSample);
        for ((scene, expected), (_, got)) in masked.iter().zip(&per_sample) {
            let (pixels, largest) = differences(expected, got);
            let line = format!(
                "{adapter_name}, {samples} samples, {scene}: per sample (taken: {taken}) differs \
                 from masked in {pixels} of {} pixels, by at most {largest} levels",
                expected.ids.len()
            );
            println!("{line}");
            log.push_str(&line);
            log.push('\n');
            if taken == PeelVariant::Masked {
                // A device without per-sample shading draws the masked
                // stages for both.
                assert_eq!(pixels, 0, "{line}");
            }
            let drawn = got.ids.iter().filter(|id| **id != NO_FACE).count();
            assert!(drawn * 20 > got.ids.len(), "{line}: {drawn} pixels drawn");
            // Away from the edges the two are one picture: what differs is
            // a few pixels along silhouettes and the cut.
            assert!(pixels * 50 < got.ids.len(), "{line}");
        }
    }
    std::fs::write(evidence("peel-variants.txt"), log).unwrap();
}

/// What the per-sample stages cost: the see-through, the cut-open and the
/// combined picture of a block with a spherical cavity of about 300 000
/// triangles, 1280 × 800 at the viewport's samples, timed per frame (the
/// draw and the wait for the GPU) in both forms, the median of 20 frames
/// after 3 to warm up. A measurement for docs/design/rendering.md, not a
/// check: run it by hand, in release
/// (`cargo test -p craft3d-gpu --release --test peel_variants -- --ignored --nocapture`).
#[test]
#[ignore = "a measurement, run by hand"]
fn what_the_per_sample_stages_cost() {
    let block = primitives::block(&at(-12.0, -12.0, -12.0), 24.0, 24.0, 24.0).unwrap();
    let ball = primitives::sphere(&at(0.0, 0.0, 0.0), 10.0).unwrap();
    let hollow = boolean(&block, &ball, BooleanOp::Subtract, &Tolerance::default()).unwrap();
    let mesh = mesh_of!(&hollow.body, 0.0004);
    println!("model: {} triangles", mesh.triangles.len());
    let options = DrawOptions {
        bodies: true,
        capture: false,
    };
    for (adapter, device, queue) in devices() {
        let info = adapter.get_info();
        for variant in [PeelVariant::Masked, PeelVariant::PerSample] {
            let setting = PeelSetting::Fixed(variant);
            let mut onscreen =
                Onscreen::with_peel_setting(&device, &queue, &adapter, setting).unwrap();
            let lines = onscreen.upload_lines(&[]).unwrap();
            let uploaded = onscreen.upload(&mesh).unwrap();
            for (kind, p) in kinds(&mesh, params(1280, 800, 1)) {
                let camera = perspective(&mesh, TOWARD, 40.0, &p);
                let mut times = Vec::new();
                for frame in 0..23 {
                    let start = std::time::Instant::now();
                    onscreen
                        .draw(&uploaded, &lines, &camera, &p, &no_overlay(), options)
                        .unwrap();
                    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
                    if frame >= 3 {
                        times.push(start.elapsed().as_secs_f64() * 1000.0);
                    }
                }
                times.sort_by(f64::total_cmp);
                println!(
                    "{} ({:?}), {} samples, {kind}, {} (taken: {}): median {:.2} ms, \
                     fastest {:.2} ms",
                    info.name,
                    info.backend,
                    onscreen.samples(),
                    variant,
                    onscreen.peel_variant(),
                    times[times.len() / 2],
                    times[0]
                );
            }
        }
    }
}
