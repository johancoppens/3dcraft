//! Pictures without a section, byte for byte, on the GPU (the review of
//! the UX round of 2026-11-18, by `review-ux-render`): the wgpu renderer's
//! frames of two models, plain, with a highlight and see-through, at one
//! and three samples a side, on every adapter, hashed. The same file runs
//! on main and on the round's tree (whose scene uniform block grew by the
//! hatching's four vectors); the hashes it prints (`BYTES` lines) must be
//! the same on both.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_gpu::{Camera, Highlight, Mesh, Offscreen, Projection, RenderParams};
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_tess::{TessParams, tessellate_body};

fn at(x: f64, y: f64, z: f64) -> Frame {
    Frame::new(Vec3::new(x, y, z), Vec3::Z, Vec3::X).unwrap()
}

fn tess() -> TessParams {
    TessParams {
        chordal_tolerance: 0.004,
        ..TessParams::default()
    }
}

fn models() -> Vec<(&'static str, Mesh)> {
    let plate = primitives::block(&at(-3.0, -2.0, -0.5), 6.0, 4.0, 1.0).unwrap();
    let drill = primitives::cylinder(&at(0.6, 0.3, -2.0), 1.1, 4.0).unwrap();
    let drilled = boolean(&plate, &drill, BooleanOp::Subtract, &Tolerance::default()).unwrap();
    let outer = primitives::block(&at(-3.0, -2.0, -1.5), 6.0, 4.0, 3.0).unwrap();
    let inner = primitives::block(&at(-2.0, -1.0, -1.0), 4.0, 2.0, 2.0).unwrap();
    let hollow = boolean(&outer, &inner, BooleanOp::Subtract, &Tolerance::default()).unwrap();
    vec![
        ("drilled", tessellate_body(&drilled.body, &tess()).unwrap()),
        ("hollow", tessellate_body(&hollow.body, &tess()).unwrap()),
    ]
}

fn fnv(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn frame_hash(frame: &craft3d_gpu::Frame) -> u64 {
    let mut bytes = frame.rgb.clone();
    for id in &frame.ids {
        bytes.extend_from_slice(&id.to_le_bytes());
    }
    for d in &frame.depth {
        bytes.extend_from_slice(&d.to_bits().to_le_bytes());
    }
    fnv(&bytes)
}

#[test]
fn gpu_pictures_without_a_section_byte_for_byte() {
    let adapters = craft3d_gpu::adapters();
    assert!(!adapters.is_empty(), "no adapter");
    for adapter in &adapters {
        let mut offscreen = Offscreen::on_adapter(adapter).unwrap();
        let summary = offscreen.adapter().clone();
        let tag = format!("{}-{}", summary.backend, summary.device_type).to_lowercase();
        for (name, mesh) in models() {
            let bbox = mesh.bbox();
            let center = bbox.center().unwrap();
            let cameras = [
                (
                    "persp",
                    Camera::framing(&bbox, Vec3::new(0.6, -0.9, 0.5), Vec3::Z, 35.0, 4.0 / 3.0)
                        .unwrap(),
                ),
                (
                    "ortho",
                    Camera {
                        eye: center + Vec3::new(-0.4, -0.7, 0.8) * 20.0,
                        target: center,
                        up: Vec3::Z,
                        projection: Projection::Orthographic { half_height: 3.4 },
                    },
                ),
            ];
            for (camera_name, camera) in cameras {
                for ssaa in [1, 3] {
                    for (look, opacity, highlight) in [
                        ("plain", 1.0, Highlight::NONE),
                        (
                            "highlight",
                            1.0,
                            Highlight::new([64, 196, 212], [0_u32, 2, 3]).0,
                        ),
                        ("seethrough", 0.5, Highlight::NONE),
                    ] {
                        let params = RenderParams {
                            width: 320,
                            height: 240,
                            ssaa,
                            opacity,
                            highlight,
                            ..RenderParams::default()
                        };
                        let frame = offscreen.render(&mesh, &camera, &params).unwrap();
                        let again = offscreen.render(&mesh, &camera, &params).unwrap();
                        let (a, b) = (frame_hash(&frame), frame_hash(&again));
                        println!(
                            "BYTES gpu {tag} {name} {camera_name} ssaa{ssaa} {look} {a:016x}{}",
                            if a == b {
                                ""
                            } else {
                                " (not the same when rendered again)"
                            }
                        );
                    }
                }
            }
        }
    }
}
