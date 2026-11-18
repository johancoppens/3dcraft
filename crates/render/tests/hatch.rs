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
        // Near the end of a step: the lines at odd multiples of the
        // spacing have faded part of the way.
        assert!(hatching.odd_ink > 0.0 && hatching.odd_ink < 1.0, "{name}");
        // On the plane y = 0.3, U is X and V is Z: w = (x + z) / √2. The
        // points of the cap between the bore and a bolt hole: on lines at
        // even multiples, on one at an odd multiple, half a spacing off
        // them, on a ramp.
        let step = hatching.spacing * std::f64::consts::SQRT_2;
        let mut colors = Vec::new();
        let mut same = 0;
        for (k, off) in [
            (10, 0.0),
            (10, 0.5),
            (12, 0.1),
            (12, 0.5),
            (13, 0.0),
            (14, 0.0),
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
                // The same lines: only how a line is drawn on screen
                // depends on the camera (its fade where they crowd).
                let h = Hatching::new(-Vec3::Y, PAPER, &HATCH, &camera, height).unwrap();
                assert_eq!(
                    (h.u, h.v, h.spacing),
                    (hatching.u, hatching.v, hatching.spacing),
                    "{name}: the same hatching at {yaw}°"
                );
                assert!((h.odd_ink - hatching.odd_ink).abs() < 1e-9, "{name}");
                // The colour the function gives at the point.
                let got = &frame.rgb[middle * 3..middle * 3 + 3];
                for (channel, want) in got.iter().zip(h.color(target, 20.0)) {
                    assert!(
                        channel.abs_diff(want) <= 1,
                        "{name} {yaw}°: {got:?} for {:?}",
                        h.color(target, 20.0)
                    );
                }
                seen.push((got.to_vec(), h.across(target, 20.0)));
            }
            // Where the lines stand 3 pixels apart or more on screen in
            // both pictures, they are drawn as they are: the same ink.
            if seen.iter().all(|(_, across)| *across >= 3.0) {
                assert_eq!(seen[0].0, seen[1].0, "{name}: the point {target:?}");
                same += 1;
            }
            colors.push(seen[0].0.clone());
        }
        assert!(same >= 3, "{name}: {same} points compared");
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

/// Seen nearly edge on, where the lines would stand closer than 2 pixels
/// apart on screen, the cap is its mean tone, smoothly: no moiré. A
/// picture of the flange's cut from 10° off its plane.
#[test]
fn seen_nearly_edge_on_the_cut_is_its_mean_tone_without_moire() {
    let mesh = flange();
    let (width, height) = (320_u32, 240_u32);
    let target = Vec3::new(0.0, 0.3, 0.5);
    let toward = Vec3::new(1.0, -(10.0_f64.to_radians().ptan()), 0.15);
    let camera = Camera {
        eye: target + toward.normalized().unwrap() * 14.0,
        target,
        up: Vec3::Z,
        projection: Projection::Perspective { fov_y_deg: 30.0 },
    };
    let params = RenderParams {
        width,
        height,
        ssaa: 1,
        section: Some(section()),
        ..RenderParams::default()
    };
    let frame = render(&mesh, &camera, &params).unwrap();
    evidence("flange-nearly-edge-on.png", &frame);
    let h = Hatching::new(-Vec3::Y, PAPER, &HATCH, &camera, height).unwrap();
    let axes = camera.axes().unwrap();
    let (w, hh) = (width as usize, height as usize);
    let aspect = f64::from(width) / f64::from(height);
    // The point of the plane under the middle of pixel (x, y), and its
    // distance along the view.
    let plane_at = |x: usize, y: usize| {
        let cx = ((x as f64 + 0.5) / f64::from(width) * 2.0 - 1.0) * aspect * axes.half;
        let cy = (1.0 - (y as f64 + 0.5) / f64::from(height) * 2.0) * axes.half;
        let ray = axes.forward + axes.right * cx + axes.up * cy;
        let n = -Vec3::Y;
        let t = (Vec3::new(0.0, 0.3, 0.5) - axes.eye).dot(n) / ray.dot(n);
        (axes.eye + ray * t, t)
    };
    let mix = |c: f64| {
        let mut out = [0u8; 3];
        for (k, slot) in out.iter_mut().enumerate() {
            let (p, i) = (f64::from(PAPER[k]), f64::from(HATCH.ink[k]));
            *slot = (p + (i - p) * c).round() as u8;
        }
        out
    };
    let (mut toned, mut drawn) = (0, 0);
    for y in 1..hh - 1 {
        for x in 1..w - 1 {
            let i = y * w + x;
            if (0..9).any(|k| frame.ids[(y + k / 3 - 1) * w + x + k % 3 - 1] != SECTION_FACE) {
                continue;
            }
            let (p, dist) = plane_at(x, y);
            let across = h.across(p, dist);
            let got = &frame.rgb[i * 3..i * 3 + 3];
            if across <= 2.0 {
                toned += 1;
                // The mean tone, and no line: within a level of it, and
                // of the next pixel's.
                let want = mix(h.mean(dist));
                for (a, b) in got.iter().zip(want) {
                    assert!(a.abs_diff(b) <= 1, "({x}, {y}): {got:?} for {want:?}");
                }
                let (q, next_dist) = plane_at(x + 1, y);
                if h.across(q, next_dist) <= 2.0 {
                    let next = &frame.rgb[(i + 1) * 3..(i + 1) * 3 + 3];
                    for (a, b) in got.iter().zip(next) {
                        assert!(a.abs_diff(*b) <= 2, "({x}, {y}): {got:?} beside {next:?}");
                    }
                }
            } else if across >= 3.0 {
                drawn += 1;
            }
        }
    }
    // Both: the lines where they stand apart, the tone where they crowd.
    assert!(toned > 40 && drawn > 40, "{toned} toned, {drawn} drawn");
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
