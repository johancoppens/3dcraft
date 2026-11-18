//! One body cut open, and no cut, byte for byte (the review of
//! `check/hatch-body`, by `review-ux-render`): the CPU reference
//! renderer's frames of two one-body models, hatched across three planes,
//! opaque and see-through, at one and three samples a side, and without a
//! section, hashed. The same file runs on `check/ux-round-2` and on the
//! hatch's trial tree; the hashes it prints (`BYTES` lines) must be the
//! same on both.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_render::{Camera, Hatch, Projection, RenderParams, Section, render};
use craft3d_tess::{Mesh, TessParams, tessellate_body};

const HATCH: Hatch = Hatch {
    ink: [88, 84, 78],
    spacing_points: 9.0,
    width_points: 1.0,
    height_points: None,
};

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

fn frame_hash(frame: &craft3d_render::Frame) -> u64 {
    let mut bytes = frame.rgb.clone();
    for id in &frame.ids {
        bytes.extend_from_slice(&id.to_le_bytes());
    }
    for d in &frame.depth {
        bytes.extend_from_slice(&d.to_bits().to_le_bytes());
    }
    fnv(&bytes)
}

/// The planes: none, across z near the middle (the half above away),
/// across x, and an oblique one.
fn planes(mesh: &Mesh) -> Vec<(&'static str, Option<Section>)> {
    let (lo, hi) = mesh.bbox().corners().unwrap();
    let mid = (lo + hi) * 0.5;
    let cut = |origin: Vec3, normal: Vec3| {
        Some(Section {
            origin,
            normal,
            cap: true,
            color: [232, 228, 220],
            hatch: Some(HATCH),
        })
    };
    vec![
        ("none", None),
        ("z", cut(mid + Vec3::new(0.0, 0.0, 0.1), Vec3::Z)),
        ("x", cut(mid + Vec3::new(0.4, 0.0, 0.0), -Vec3::X)),
        ("oblique", cut(mid, Vec3::new(0.5, -0.7, 0.6))),
    ]
}

#[test]
fn one_body_cut_open_byte_for_byte() {
    for (name, mesh) in models() {
        let bbox = mesh.bbox();
        let center = bbox.center().unwrap();
        for (plane, section) in planes(&mesh) {
            for (camera_name, camera) in [
                (
                    "persp",
                    Camera::framing(&bbox, Vec3::new(0.6, -0.9, 0.7), Vec3::Z, 35.0, 4.0 / 3.0)
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
            ] {
                for (ssaa, opacity) in [(1, 1.0), (3, 1.0), (2, 0.5)] {
                    let params = RenderParams {
                        width: 320,
                        height: 240,
                        ssaa,
                        opacity,
                        section,
                        ..RenderParams::default()
                    };
                    let frame = render(&mesh, &camera, &params).unwrap();
                    println!(
                        "BYTES cpu {name} {plane} {camera_name} ssaa{ssaa} opacity{opacity} {:016x}",
                        frame_hash(&frame)
                    );
                }
            }
        }
    }
}
