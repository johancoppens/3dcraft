//! Pictures of booleans with a freeform face, for the developer to look at
//! (`target/evidence/render/freeform/`): the domed block of the testkit
//! (a bicubic roof) with a round hole through its roof, a ball sunk into
//! it, and a block through its walls. Each picture must show the roof.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft3d_geom::Surface;
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_render::{Camera, RenderParams, encode_png, render};
use craft3d_tess::{TessParams, tessellate_body};
use craft3d_testkit::trimmed::domed_block;
use std::path::PathBuf;

fn evidence_dir() -> PathBuf {
    let dir =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/render/freeform");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn the_dome_with_a_hole_a_ball_and_a_block_are_rendered() {
    let tol = Tolerance::default();
    let dome = domed_block(&Frame::WORLD, 4.0, 3.0, 2.0, false)
        .unwrap()
        .body;
    let hole = primitives::cylinder(
        &Frame::new(Vec3::new(2.0, 1.5, -1.0), Vec3::Z, Vec3::X).unwrap(),
        0.5,
        5.0,
    )
    .unwrap();
    let ball = primitives::sphere(
        &Frame::new(Vec3::new(1.0, 2.2, 2.3), Vec3::Z, Vec3::X).unwrap(),
        0.7,
    )
    .unwrap();
    let block = primitives::block(
        &Frame::new(Vec3::new(2.8, -0.5, -0.5), Vec3::Z, Vec3::X).unwrap(),
        1.0,
        4.0,
        1.2,
    )
    .unwrap();
    let scenes = [
        (
            "dome-hole",
            boolean(&dome, &hole, BooleanOp::Subtract, &tol).unwrap(),
        ),
        (
            "dome-ball",
            boolean(&dome, &ball, BooleanOp::Subtract, &tol).unwrap(),
        ),
        (
            "dome-block",
            boolean(&dome, &block, BooleanOp::Union, &tol).unwrap(),
        ),
    ];
    let dir = evidence_dir();
    for (name, result) in &scenes {
        let roof: Vec<u32> = result
            .body
            .faces
            .iter()
            .enumerate()
            .filter(|(_, f)| matches!(f.surface, Surface::Bezier(_)))
            .map(|(i, _)| i as u32)
            .collect();
        assert!(!roof.is_empty(), "{name}: the roof is gone");
        let mesh = tessellate_body(
            &result.body,
            &TessParams {
                chordal_tolerance: 0.002,
                ..TessParams::default()
            },
        )
        .unwrap();
        let params = RenderParams {
            width: 1280,
            height: 800,
            ssaa: 2,
            ..RenderParams::default()
        };
        let camera = Camera::framing(
            &mesh.bbox(),
            Vec3::new(0.6, -1.0, 0.7),
            Vec3::Z,
            30.0,
            f64::from(params.width) / f64::from(params.height),
        )
        .unwrap();
        let frame = render(&mesh, &camera, &params).unwrap();
        std::fs::write(dir.join(format!("{name}.png")), frame.to_png().unwrap()).unwrap();
        std::fs::write(
            dir.join(format!("{name}-ids.png")),
            encode_png(frame.width, frame.height, &frame.false_color()).unwrap(),
        )
        .unwrap();
        let visible = frame.visible_faces();
        assert!(
            roof.iter().any(|id| visible.contains(id)),
            "{name}: the roof is not in the picture"
        );
        assert!(
            frame.coverage() > 0.1,
            "{name}: coverage {}",
            frame.coverage()
        );
        println!(
            "{name}: {} faces, {} triangles, coverage {:.3}, {}",
            result.body.faces.len(),
            mesh.triangles.len(),
            frame.coverage(),
            dir.join(format!("{name}.png")).display()
        );
    }
}
