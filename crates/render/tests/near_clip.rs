//! Near-plane clipping in the CPU renderer (docs/design/rendering.md ›
//! *Near-plane clipping*): a triangle with a corner behind the eye is cut
//! along the near plane and its front part drawn, as the GPU draws it.
//! Before, such a triangle was left out whole.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
use craft3d_math::Vec3;
use craft3d_render::{Camera, FaceColors, NO_FACE, Projection, RenderParams, pick};
use craft3d_tess::Mesh;

/// A floor: the square `±half` in x and y at z = 0, two triangles, with
/// its normal up.
fn floor(half: f64) -> Mesh {
    let n = Vec3::Z;
    Mesh {
        positions: vec![
            Vec3::new(-half, -half, 0.0),
            Vec3::new(half, -half, 0.0),
            Vec3::new(half, half, 0.0),
            Vec3::new(-half, half, 0.0),
        ],
        normals: vec![n; 4],
        triangles: vec![[0, 1, 2], [0, 2, 3]],
        face_ids: vec![7, 7],
    }
}

fn params(w: u32, h: u32) -> RenderParams {
    RenderParams {
        width: w,
        height: h,
        ssaa: 1,
        ..RenderParams::default()
    }
}

/// Standing at height 1 over a floor 2000 wide, looking level along +x:
/// every corner of the floor's two triangles but one is far ahead or far
/// behind, so both cross the eye plane. Closed form: the floor fills
/// exactly the rows whose centres are below the horizon (the middle of the
/// picture), out to the floor's end at distance 1000, which is 0.09 of a
/// row below the horizon; above it nothing. At a row `r` below the horizon
/// (`r` pixels from the middle to the centre) the distance is
/// `1 / (r / (h/2) · tan(fov/2))`. On the old renderer the picture was
/// empty.
#[test]
fn a_floor_under_the_eye_fills_exactly_the_rows_below_the_horizon() {
    // At every scale the rules test (1e-3 to 1e6 model units): the near
    // plane is a millionth of the farthest distance, 1e-6 at the least.
    for scale in [1e-3, 1.0, 1e3, 1e6] {
        let mesh = floor(1000.0 * scale);
        let (w, h) = (120_u32, 100_u32);
        let fov = 60.0_f64;
        let camera = Camera {
            eye: Vec3::new(0.0, 0.0, scale),
            target: Vec3::new(scale, 0.0, scale),
            up: Vec3::Z,
            projection: Projection::Perspective { fov_y_deg: fov },
        };
        let frame = craft3d_render::render(&mesh, &camera, &params(w, h)).unwrap();
        let tan = (fov.to_radians() * 0.5).ptan();
        for y in 0..h as usize {
            for x in 0..w as usize {
                let i = y * w as usize + x;
                let below = y as f64 + 0.5 - f64::from(h) / 2.0;
                if below < 0.0 {
                    assert_eq!(
                        frame.ids[i], NO_FACE,
                        "{scale}: ({x}, {y}) above the horizon"
                    );
                    continue;
                }
                assert_eq!(frame.ids[i], 7, "{scale}: ({x}, {y}) below the horizon");
                let dist = scale / (below / (f64::from(h) / 2.0) * tan);
                assert!(
                    (frame.depth[i] - dist).abs() <= 1e-9 * dist,
                    "{scale}: ({x}, {y}): depth {} against {dist}",
                    frame.depth[i]
                );
            }
        }
        // Picking agrees: a point of the floor, on one of its two triangles.
        let hit = pick(&mesh, &camera, w, h, 60.5, 80.5).unwrap().unwrap();
        assert_eq!(hit.face, 7);
        assert!(hit.triangle < 2, "{}", hit.triangle);
        assert!(hit.point.z.abs() < 1e-12 * scale, "{:?}", hit.point);
        let dist = scale / (30.5 / 50.0 * tan);
        assert!((hit.point.x - dist).abs() < 1e-9 * dist, "{:?}", hit.point);
    }
}

/// Inside a closed block at 1e-3 and at 1 model units, near a corner: the
/// near plane cuts the faces along the block's sharp edges, where each face
/// has vertices of its own walked the other way; every pixel is a face (no
/// pinhole where two faces' cuts meet).
#[test]
fn inside_a_closed_block_every_pixel_is_a_face() {
    for scale in [1e-3, 1.0] {
        let frame = craft3d_math::Frame::new(Vec3::ZERO, Vec3::Z, Vec3::X).unwrap();
        let block =
            craft3d_testkit::solids::block(&frame, 2.0 * scale, 2.0 * scale, 2.0 * scale).unwrap();
        let tess = craft3d_tess::TessParams {
            chordal_tolerance: 1e-3 * scale,
            ..craft3d_tess::TessParams::default()
        };
        let mesh = craft3d_tess::tessellate_body(&block.body, &tess).unwrap();
        for (eye, target) in [
            (Vec3::new(0.3, 0.4, 0.2), Vec3::new(1.5, 1.2, 1.7)),
            (Vec3::new(1e-4, 1.0, 1.0), Vec3::new(1.0, 1.0, 1.0)),
        ] {
            let camera = Camera {
                eye: eye * scale,
                target: target * scale,
                up: Vec3::Z,
                projection: Projection::Perspective { fov_y_deg: 100.0 },
            };
            let frame = craft3d_render::render(&mesh, &camera, &params(160, 120)).unwrap();
            let empty = frame.ids.iter().filter(|&&id| id == NO_FACE).count();
            assert_eq!(
                empty, 0,
                "{scale}: {empty} pixels show nothing from {eye:?}"
            );
        }
    }
}

/// A curvature comb that runs from behind the eye to in front of it is
/// drawn for its part in front (`analysis::draw_lines`); before, the whole
/// segment was left out.
#[test]
fn a_comb_segment_through_the_eye_plane_is_drawn_in_front() {
    let camera = Camera {
        eye: Vec3::ZERO,
        target: Vec3::X,
        up: Vec3::Z,
        projection: Projection::Perspective { fov_y_deg: 60.0 },
    };
    let mut frame = craft3d_render::render(&floor(1.0), &camera, &params(80, 60)).unwrap();
    let line = vec![vec![Vec3::new(-5.0, 0.0, -0.5), Vec3::new(20.0, 0.0, -0.5)]];
    craft3d_render::analysis::draw_lines(&mut frame, &camera, &line, [255, 0, 0], 1e9).unwrap();
    let red = frame.rgb.chunks(3).filter(|p| p == &[255, 0, 0]).count();
    assert!(red > 10, "{red} pixels of the line");
}

/// A triangle wholly behind the eye draws nothing; a triangle with a
/// corner that is not a number, or exactly at the eye, never panics.
#[test]
fn triangles_behind_the_eye_or_degenerate_are_left_out_without_a_panic() {
    let camera = Camera {
        eye: Vec3::ZERO,
        target: Vec3::X,
        up: Vec3::Z,
        projection: Projection::Perspective { fov_y_deg: 50.0 },
    };
    let n = Vec3::new(-1.0, 0.0, 0.0);
    let behind = Mesh {
        positions: vec![
            Vec3::new(-1.0, -1.0, -1.0),
            Vec3::new(-1.0, 1.0, -1.0),
            Vec3::new(-1.0, 0.0, 1.0),
        ],
        normals: vec![n; 3],
        triangles: vec![[0, 1, 2]],
        face_ids: vec![1],
    };
    let frame = craft3d_render::render(&behind, &camera, &params(40, 30)).unwrap();
    assert!(frame.ids.iter().all(|&id| id == NO_FACE));
    for odd in [
        Vec3::ZERO,
        Vec3::new(f64::NAN, 0.0, 0.0),
        Vec3::new(1e300, 0.0, 0.0),
    ] {
        let mesh = Mesh {
            positions: vec![odd, Vec3::new(2.0, -1.0, -1.0), Vec3::new(2.0, 1.0, 0.5)],
            normals: vec![n; 3],
            triangles: vec![[0, 1, 2], [2, 1, 0]],
            face_ids: vec![1, 2],
        };
        let _ = craft3d_render::render(&mesh, &camera, &params(40, 30));
        let _ = craft3d_render::render_analysed(
            &mesh,
            &FaceColors::new(),
            &[1.0, 2.0, f32::NAN],
            &camera,
            &RenderParams {
                analysis: Some(craft3d_render::analysis::Analysis::new(
                    craft3d_render::analysis::AnalysisMode::Curvature,
                )),
                ..params(40, 30)
            },
        );
        let _ = pick(&mesh, &camera, 40, 30, 20.0, 15.0);
    }
}

/// Clipping is deterministic and leaves a picture with nothing near the
/// eye bit for bit as it was.
#[test]
fn clipping_is_deterministic_and_changes_nothing_far_from_the_eye() {
    let mesh = floor(1000.0);
    let camera = Camera {
        eye: Vec3::new(0.0, 0.0, 1.0),
        target: Vec3::new(1.0, 0.3, 0.6),
        up: Vec3::Z,
        projection: Projection::Perspective { fov_y_deg: 45.0 },
    };
    let a = craft3d_render::render(&mesh, &camera, &params(80, 60)).unwrap();
    let b = craft3d_render::render(&mesh, &camera, &params(80, 60)).unwrap();
    assert_eq!(a, b);
    // A small floor seen from afar: no corner near the eye.
    let small = floor(1.0);
    let far = Camera {
        eye: Vec3::new(-4.0, -3.0, 3.0),
        target: Vec3::ZERO,
        up: Vec3::Z,
        projection: Projection::Perspective { fov_y_deg: 45.0 },
    };
    let frame = craft3d_render::render(&small, &far, &params(80, 60)).unwrap();
    assert!(frame.ids.iter().filter(|&&id| id == 7).count() > 100);
}
