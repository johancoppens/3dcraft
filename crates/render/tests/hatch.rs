//! The hatching of a section's cap (`craft3d_render::Hatching`; the UX
//! round of 2026-11-18, V1): lines of ink on paper, a function of the
//! point of the plane in model space. The oracle: a camera orbited about a
//! point of the cap sees that point in the middle pixel of both pictures,
//! and the pixel has the same colour in both, the colour the function
//! gives there; the lines do not swim with the screen.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Portable, Tolerance, Vec3};
use craft3d_render::{
    Camera, Hatch, Hatching, Projection, RenderParams, SECTION_FACE, Section, render,
};
use craft3d_tess::{Mesh, TessParams, tessellate_body};
use std::path::PathBuf;

const PAPER: [u8; 3] = [232, 228, 220];
const HATCH: Hatch = Hatch {
    ink: [88, 84, 78],
    spacing_points: 9.0,
    width_points: 1.0,
    height_points: None,
};

fn at(x: f64, y: f64, z: f64) -> Frame {
    Frame::new(Vec3::new(x, y, z), Vec3::Z, Vec3::X).unwrap()
}

/// A square flange: a plate of 8 × 8, 1 thick on z = 0, with a bore of
/// radius 1.2 and four bolt holes of radius 0.4 on a circle of 2.8.
fn flange() -> Mesh {
    let tolerance = Tolerance::default();
    let mut body = primitives::block(&at(-4.0, -4.0, 0.0), 8.0, 8.0, 1.0).unwrap();
    let mut drills = vec![primitives::cylinder(&at(0.0, 0.0, -1.0), 1.2, 3.0).unwrap()];
    for (x, y) in [(2.8, 0.0), (0.0, 2.8), (-2.8, 0.0), (0.0, -2.8)] {
        drills.push(primitives::cylinder(&at(x, y, -1.0), 0.4, 3.0).unwrap());
    }
    for drill in &drills {
        body = boolean(&body, drill, BooleanOp::Subtract, &tolerance)
            .unwrap()
            .body;
    }
    tessellate_body(
        &body,
        &TessParams {
            chordal_tolerance: 0.004,
            ..TessParams::default()
        },
    )
    .unwrap()
}

/// Across y = 0.3, through two of the bolt holes; the half y < 0.3, where
/// the cameras are, cut away.
fn section() -> Section {
    Section {
        origin: Vec3::new(0.0, 0.3, 0.5),
        normal: -Vec3::Y,
        cap: true,
        color: PAPER,
        hatch: Some(HATCH),
    }
}

/// A camera looking at `target` from 20 units away, 30° above the plane
/// z = 0.5 and `yaw` degrees about the vertical from straight in front
/// (from -y).
fn camera(target: Vec3, yaw: f64, projection: Projection) -> Camera {
    let (yaw, pitch) = (yaw.to_radians(), 30.0_f64.to_radians());
    let toward = Vec3::new(
        yaw.psin() * pitch.pcos(),
        -yaw.pcos() * pitch.pcos(),
        pitch.psin(),
    );
    Camera {
        eye: target + toward * 20.0,
        target,
        up: Vec3::Z,
        projection,
    }
}

fn evidence(name: &str, frame: &craft3d_render::Frame) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/render/hatch");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(name), frame.to_png().unwrap()).unwrap();
}

#[test]
fn a_point_of_the_cap_has_the_same_ink_when_the_camera_orbits_about_it() {
    let mesh = flange();
    // Odd sizes: the camera's target is the middle of the middle pixel.
    let (width, height) = (301_u32, 201_u32);
    let middle = (height as usize / 2) * width as usize + width as usize / 2;
    let params = RenderParams {
        width,
        height,
        ssaa: 1,
        section: Some(section()),
        ..RenderParams::default()
    };
    for (name, projection) in [
        (
            "orthographic",
            Projection::Orthographic { half_height: 2.5 },
        ),
        ("perspective", Projection::Perspective { fov_y_deg: 14.0 }),
    ] {
        // The spacing, the same for both cameras (orbiting keeps the
        // distance to the target): an eighth of a unit, about five pixels.
        let front = camera(Vec3::new(1.8, 0.3, 0.5), 0.0, projection);
        let hatching = Hatching::new(-Vec3::Y, PAPER, &HATCH, &front, height).expect("a hatching");
        assert_eq!(hatching.spacing, 0.125, "{name}");
        // On the plane y = 0.3, U is X and V is Z: w = (x + z) / √2. The
        // points of the cap between the bore and a bolt hole: on a line,
        // half a spacing off it, and between the two.
        let step = hatching.spacing * std::f64::consts::SQRT_2;
        let mut colors = Vec::new();
        for (k, off) in [
            (11, 0.0),
            (11, 0.5),
            (12, 0.1),
            (12, 0.25),
            (13, 0.0),
            (13, 0.75),
        ] {
            let x = (f64::from(k) + off) * step - 0.5;
            let target = Vec3::new(x, 0.3, 0.5);
            let mut seen = Vec::new();
            for yaw in [0.0, 20.0] {
                let camera = camera(target, yaw, projection);
                let frame = render(&mesh, &camera, &params).unwrap();
                if k == 12 && off == 0.1 {
                    evidence(&format!("flange-{name}-{yaw}.png"), &frame);
                }
                assert_eq!(
                    frame.ids[middle], SECTION_FACE,
                    "{name}, {k} + {off}, {yaw}°"
                );
                assert_eq!(
                    Hatching::new(-Vec3::Y, PAPER, &HATCH, &camera, height),
                    Some(hatching),
                    "{name}: the same hatching at {yaw}°"
                );
                seen.push(frame.rgb[middle * 3..middle * 3 + 3].to_vec());
            }
            assert_eq!(seen[0], seen[1], "{name}: the point {target:?}");
            // The colour the function gives at the point.
            let distance = 20.0;
            let expected = hatching.color(target, distance);
            for (channel, want) in seen[0].iter().zip(expected) {
                assert!(
                    channel.abs_diff(want) <= 1,
                    "{name}: {seen:?} for {expected:?}"
                );
            }
            colors.push(seen[0].clone());
        }
        // Not one colour: the lines are there, and paper between them.
        assert!(
            colors.iter().any(|c| c[..] == PAPER[..]),
            "{name}: {colors:?}"
        );
        assert!(colors.iter().any(|c| c[0] < 100), "{name}: {colors:?}");
        assert!(
            colors.iter().any(|c| c[0] > 100 && c[0] < PAPER[0]),
            "{name}: {colors:?}"
        );
    }
}

#[test]
fn a_cap_without_hatching_is_its_paper_and_a_picture_without_a_section_has_no_cap() {
    let mesh = flange();
    let camera = camera(
        Vec3::new(0.0, 0.3, 0.5),
        10.0,
        Projection::Perspective { fov_y_deg: 30.0 },
    );
    // One sample per pixel: a pixel of the cap is the cap's alone.
    let base = RenderParams {
        width: 320,
        height: 240,
        ssaa: 1,
        ..RenderParams::default()
    };
    let flat = render(
        &mesh,
        &camera,
        &RenderParams {
            section: Some(Section {
                hatch: None,
                ..section()
            }),
            ..base
        },
    )
    .unwrap();
    let hatched = render(
        &mesh,
        &camera,
        &RenderParams {
            section: Some(section()),
            ..base
        },
    )
    .unwrap();
    // The same cap, coloured otherwise; nothing else changes.
    assert_eq!(flat.ids, hatched.ids);
    assert_eq!(flat.depth, hatched.depth);
    let mut cap = 0;
    for (i, &id) in flat.ids.iter().enumerate() {
        if id == SECTION_FACE {
            cap += 1;
            assert_eq!(flat.rgb[i * 3..i * 3 + 3], PAPER[..]);
        } else {
            assert_eq!(flat.rgb[i * 3..i * 3 + 3], hatched.rgb[i * 3..i * 3 + 3]);
        }
    }
    assert!(cap > 1_000, "{cap} cap pixels");
    assert_ne!(flat.rgb, hatched.rgb);
    let whole = render(&mesh, &camera, &base).unwrap();
    assert!(whole.ids.iter().all(|&id| id != SECTION_FACE));
}
