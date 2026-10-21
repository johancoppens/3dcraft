//! See-through bodies where the comparison of the two depth layers is put
//! to the test: a thin wall, two faces in one plane, a face seen nearly
//! edge-on, layers closer together than the depth buffer resolves, and
//! lines over a see-through picture. Both renderers of the crate (the one
//! that reads frames back, at one sample per texel, and the viewport's,
//! multisampled) against the CPU reference renderer, on every adapter.
//! The general scenes are in `parity.rs` and `onscreen.rs`.
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

use common::{
    compare_msaa_see_through, compare_see_through, evidence, offscreens, params, write_evidence,
};
use craft3d_gpu::{
    Camera, DrawOptions, Frame, GpuError, Highlight, Mesh, NO_FACE, Onscreen, OverlayParams,
    Projection, RenderParams, Segment,
};
use craft3d_math::Vec3;
use devices::devices;

fn quad(mesh: &mut Mesh, corners: [Vec3; 4], face: u32) {
    let base = mesh.positions.len() as u32;
    mesh.positions.extend(corners);
    // No normals: each triangle is shaded from its own.
    mesh.normals.extend([Vec3::ZERO; 4]);
    mesh.triangles.push([base, base + 1, base + 2]);
    mesh.triangles.push([base, base + 2, base + 3]);
    mesh.face_ids.extend([face, face]);
}

/// A square of side `2 * half` in the plane `z = height`, tilted a little
/// about x so that its depth varies over the picture.
fn square(mesh: &mut Mesh, half: f64, height: f64, face: u32) {
    let tilt = 0.2;
    let corner = |x: f64, y: f64| Vec3::new(x, y, height + tilt * y);
    quad(
        mesh,
        [
            corner(-half, -half),
            corner(half, -half),
            corner(half, half),
            corner(-half, half),
        ],
        face,
    );
}

fn view(eye: Vec3, projection: Projection) -> Camera {
    Camera {
        eye,
        target: Vec3::ZERO,
        up: Vec3::Y,
        projection,
    }
}

fn no_overlay() -> OverlayParams {
    OverlayParams {
        slack: 0.0,
        hidden_opacity: 70,
        axis_length: 1.0,
        edges: craft3d_gpu::EdgeLook::default(),
    }
}

/// What the three ways of drawing give for one scene: the CPU frame, then
/// per adapter the frame read back and the viewport's (with its samples).
struct Drawn {
    cpu: Frame,
    gpu: Vec<(String, Frame)>,
}

fn draw(name: &str, mesh: &Mesh, camera: &Camera, params: &RenderParams) -> Drawn {
    let cpu = craft3d_render::render(mesh, camera, params).unwrap();
    let mut gpu = Vec::new();
    for mut offscreen in offscreens() {
        let frame = offscreen.render(mesh, camera, params).unwrap();
        gpu.push((
            format!("{name}, read back on {}", offscreen.adapter()),
            frame,
        ));
    }
    if params.ssaa == 1 {
        for (adapter, device, queue) in devices() {
            let mut onscreen = Onscreen::new(&device, &queue, &adapter).unwrap();
            let uploaded = onscreen.upload(mesh).unwrap();
            let lines = onscreen.upload_lines(&[]).unwrap();
            let options = DrawOptions {
                bodies: true,
                capture: true,
            };
            onscreen
                .draw(&uploaded, &lines, camera, params, &no_overlay(), options)
                .unwrap();
            gpu.push((
                format!(
                    "{name}, viewport with {} samples on {}",
                    onscreen.samples(),
                    adapter.get_info().name
                ),
                onscreen.read_frame().unwrap(),
            ));
        }
    }
    Drawn { cpu, gpu }
}

/// The comparison of `common` (*See-through pictures*) for every way of
/// drawing `mesh`.
fn check(name: &str, mesh: &Mesh, camera: &Camera, params: &RenderParams) -> Drawn {
    let drawn = draw(name, mesh, camera, params);
    for (index, (label, gpu)) in drawn.gpu.iter().enumerate() {
        let band = if label.contains("viewport") {
            let (parity, band) = compare_msaa_see_through(mesh, camera, params, &drawn.cpu, gpu);
            println!("{label}: {parity:?}");
            parity.check(label);
            band
        } else {
            let (parity, band) = compare_see_through(mesh, camera, params, &drawn.cpu, gpu);
            println!("{label}: {parity:?}");
            parity.check(label);
            band
        };
        write_evidence(
            &format!("layers-{name}-{index}.png"),
            &drawn.cpu,
            gpu,
            &band,
        );
    }
    drawn
}

/// One sheet has nothing behind it but the background: nowhere may the
/// sheet show through itself (the front taken for its own "behind", the
/// z-fighting of a surface with itself), at any slope.
///
/// In the first view the pixel centres lie exactly on the diagonal the
/// sheet's two triangles share. The CPU renderer has such a sample inside
/// both triangles, with depth keys that agree only up to rounding; it
/// used to take the sheet for its own "behind" there (8 pixels of this
/// picture came out opaque; measured 2026-10-14). Since the same day the
/// triangles that share an edge are one surface at a sample on it
/// (`craft3d_render::render`, *See-through*), and this view is compared
/// like the others. The GPU's rasteriser gives such a sample to one
/// triangle.
#[test]
fn a_surface_is_never_behind_itself() {
    for (name, eye, target, projection) in [
        (
            "on-the-diagonal",
            Vec3::new(0.0, 0.0, 6.0),
            Vec3::ZERO,
            Projection::Orthographic { half_height: 1.5 },
        ),
        // The same, a third of a pixel aside.
        (
            "head-on",
            Vec3::new(0.0057, 0.0021, 6.0),
            Vec3::new(0.0057, 0.0021, 0.0),
            Projection::Orthographic { half_height: 1.5 },
        ),
        (
            "oblique",
            Vec3::new(2.0, -5.0, 3.0),
            Vec3::ZERO,
            Projection::Perspective { fov_y_deg: 30.0 },
        ),
        // Three degrees above the sheet's plane.
        (
            "grazing",
            Vec3::new(0.3, -6.0, -0.9),
            Vec3::ZERO,
            Projection::Perspective { fov_y_deg: 25.0 },
        ),
    ] {
        let mut mesh = Mesh::default();
        square(&mut mesh, 1.0, 0.0, 4);
        let params = RenderParams {
            opacity: 0.5,
            background: [0, 0, 0],
            ..params(240, 180, 1)
        };
        let camera = Camera {
            eye,
            target,
            up: Vec3::Y,
            projection,
        };
        let scene = format!("sheet-{name}");
        let drawn = check(&scene, &mesh, &camera, &params);
        // Over black at a half, a sheet that showed through itself would
        // be as bright as the opaque sheet: no pixel may be.
        let opaque = RenderParams {
            opacity: 1.0,
            ..params
        };
        let front = craft3d_render::render(&mesh, &camera, &opaque).unwrap();
        let leaked = |frame: &Frame| {
            (0..frame.ids.len())
                .filter(|&i| {
                    front.ids[i] != NO_FACE
                        && u32::from(frame.rgb[i * 3]) * 4 > u32::from(front.rgb[i * 3]) * 3
                })
                .count()
        };
        assert!(front.coverage() > 0.02, "{name}");
        assert_eq!(leaked(&drawn.cpu), 0, "{name}: on the CPU");
        for (label, gpu) in &drawn.gpu {
            assert_eq!(leaked(gpu), 0, "{label}: the sheet shows through itself");
        }
    }
}

/// A wall of two parallel sheets: the back one, highlighted, shows through
/// the front one down to a gap far below anything a model has. `gap` is a
/// fraction of the wall's width.
#[test]
fn a_thin_wall_shows_its_other_side() {
    for (name, gap) in [("1e-2", 1e-2), ("1e-4", 1e-4)] {
        let mut mesh = Mesh::default();
        square(&mut mesh, 1.0, 0.0, 0);
        square(&mut mesh, 1.0, -2.0 * gap, 1);
        // Something else in the picture, so that the depth range is not
        // the wall's own thickness.
        square(&mut mesh, 0.4, -1.5, 2);
        let params = RenderParams {
            opacity: 0.5,
            highlight: Highlight::new([255, 150, 40], [1]).0,
            ..params(240, 180, 1)
        };
        for (kind, camera) in [
            (
                "persp",
                view(
                    Vec3::new(1.0, -2.0, 6.0),
                    Projection::Perspective { fov_y_deg: 30.0 },
                ),
            ),
            (
                "ortho",
                view(
                    Vec3::new(-1.0, 3.0, 6.0),
                    Projection::Orthographic { half_height: 1.6 },
                ),
            ),
        ] {
            let drawn = check(&format!("wall-{name}-{kind}"), &mesh, &camera, &params);
            // The highlighted back shows: the picture has its hue.
            let orange = |frame: &Frame| {
                frame
                    .rgb
                    .chunks(3)
                    .filter(|p| i32::from(p[0]) - i32::from(p[2]) > 40)
                    .count()
            };
            assert!(orange(&drawn.cpu) > 2000, "{name} {kind}");
            for (label, gpu) in &drawn.gpu {
                assert!(
                    orange(gpu) * 100 > orange(&drawn.cpu) * 97,
                    "{label}: the back of the wall does not show"
                );
            }
        }
    }
}

/// Two layers closer together than a 32-bit depth buffer tells apart. The
/// CPU renderer, in `f64`, has the first sheet in front and the second
/// behind it. To the GPU their depths are equal or a rounding apart either
/// way, so a sample is one of three things, each a picture the CPU
/// renderer draws of a scene without one of the sheets: the first sheet
/// over the second (the CPU's picture), the first over what is past both, here the
/// background (a tie: nothing is "behind" at the front's own depth), or the second in
/// front (ordinary z-fighting, as in an opaque picture; it is highlighted
/// here and so not seen through). Nothing else, and no failure; a
/// multisampled pixel may mix them.
#[test]
fn layers_the_depth_buffer_cannot_tell_apart_are_one_or_the_other() {
    let scene = |first: bool, second: bool| {
        let mut mesh = Mesh::default();
        if first {
            square(&mut mesh, 1.0, 0.0, 0);
        }
        if second {
            square(&mut mesh, 1.0, -1e-9, 1);
        }
        mesh
    };
    let params = RenderParams {
        opacity: 0.5,
        highlight: Highlight::new([255, 150, 40], [1]).0,
        ..params(200, 150, 1)
    };
    let camera = view(
        Vec3::new(1.0, -2.0, 6.0),
        Projection::Perspective { fov_y_deg: 30.0 },
    );
    let both = draw("tie", &scene(true, true), &camera, &params);
    let candidates = [
        both.cpu.clone(),
        craft3d_render::render(&scene(true, false), &camera, &params).unwrap(),
        craft3d_render::render(&scene(false, true), &camera, &params).unwrap(),
    ];
    let mut log = String::new();
    for (label, gpu) in &both.gpu {
        let mut counts = [0_usize; 4];
        for i in 0..gpu.ids.len() {
            // Away from the outlines, on the two sheets.
            let (x, y) = (i % 200, i / 200);
            let inside = (1..199).contains(&x)
                && (1..149).contains(&y)
                && [i - 1, i + 1, i - 200, i + 200]
                    .iter()
                    .all(|&n| both.cpu.ids[n] == both.cpu.ids[i]);
            if !inside || both.cpu.ids[i] != 0 {
                continue;
            }
            let pixel = |frame: &Frame| -> [u8; 3] {
                [frame.rgb[i * 3], frame.rgb[i * 3 + 1], frame.rgb[i * 3 + 2]]
            };
            let got = pixel(gpu);
            let near = |other: &Frame| {
                got.iter()
                    .zip(pixel(other))
                    .all(|(p, q)| p.abs_diff(q) <= 1)
            };
            // Between the three, channel by channel: a mix of samples.
            let between = (0..3).all(|c| {
                let levels = candidates.each_ref().map(|frame| pixel(frame)[c]);
                let (lo, hi) = (levels.iter().min().unwrap(), levels.iter().max().unwrap());
                got[c].saturating_add(1) >= *lo && got[c] <= hi.saturating_add(1)
            });
            match candidates.iter().position(near) {
                Some(which) => counts[which] += 1,
                None if between && label.contains("viewport") => counts[3] += 1,
                None => panic!(
                    "{label}: pixel {x}, {y} is {got:?}, not one of {:?}",
                    candidates.each_ref().map(pixel)
                ),
            }
        }
        log.push_str(&format!(
            "{label}: {} pixels show the second sheet behind the first, {} see past both, \
             {} have the second in front, {} mix them\n",
            counts[0], counts[1], counts[2], counts[3]
        ));
        assert!(counts.iter().sum::<usize>() > 5000, "{label}");
    }
    print!("{log}");
    std::fs::write(evidence("layers-tie.txt"), log).unwrap();
}

/// Two faces in one plane, corner for corner: the first is the front, and
/// the second is not behind it (its depth is the front's own), so the
/// background shows, as the CPU renderer has it. With a third sheet
/// further back, that one shows.
#[test]
fn a_face_that_coincides_with_the_front_is_not_behind_it() {
    for with_third in [false, true] {
        let mut mesh = Mesh::default();
        square(&mut mesh, 1.0, 0.0, 7);
        square(&mut mesh, 1.0, 0.0, 3);
        if with_third {
            square(&mut mesh, 1.2, -0.8, 5);
        }
        let params = RenderParams {
            opacity: 0.5,
            highlight: Highlight::new([255, 150, 40], [3]).0,
            ..params(240, 180, 1)
        };
        let camera = view(
            Vec3::new(0.7, -2.0, 5.0),
            Projection::Perspective { fov_y_deg: 35.0 },
        );
        let drawn = check(&format!("coincident-{with_third}"), &mesh, &camera, &params);
        for (label, gpu) in &drawn.gpu {
            // The twin, which is highlighted, is nowhere in the picture.
            let orange = gpu
                .rgb
                .chunks(3)
                .filter(|p| i32::from(p[0]) - i32::from(p[2]) > 40)
                .count();
            assert_eq!(orange, 0, "{label}: the coinciding face shows");
            assert!(gpu.visible_faces().contains(&7), "{label}");
            assert!(!gpu.visible_faces().contains(&3), "{label}");
        }
    }
}

/// A box seen almost along its top face: the face is a few pixels high on
/// screen and its depth runs through most of the range within them.
#[test]
fn a_face_seen_nearly_edge_on_keeps_its_layers() {
    let mut mesh = Mesh::default();
    let corner = |x: f64, y: f64, z: f64| Vec3::new(x, y, z);
    let (a, b) = (1.5, 0.5);
    let faces = [
        // Top and bottom.
        [
            corner(-a, -a, b),
            corner(a, -a, b),
            corner(a, a, b),
            corner(-a, a, b),
        ],
        [
            corner(-a, -a, -b),
            corner(a, -a, -b),
            corner(a, a, -b),
            corner(-a, a, -b),
        ],
        // The four sides.
        [
            corner(-a, -a, -b),
            corner(a, -a, -b),
            corner(a, -a, b),
            corner(-a, -a, b),
        ],
        [
            corner(-a, a, -b),
            corner(a, a, -b),
            corner(a, a, b),
            corner(-a, a, b),
        ],
        [
            corner(-a, -a, -b),
            corner(-a, a, -b),
            corner(-a, a, b),
            corner(-a, -a, b),
        ],
        [
            corner(a, -a, -b),
            corner(a, a, -b),
            corner(a, a, b),
            corner(a, -a, b),
        ],
    ];
    for (face, corners) in faces.into_iter().enumerate() {
        quad(&mut mesh, corners, face as u32);
    }
    for (name, eye, projection) in [
        // Two degrees above the top face's plane.
        (
            "persp",
            Vec3::new(1.0, -9.0, 0.5 + 0.32),
            Projection::Perspective { fov_y_deg: 25.0 },
        ),
        (
            "ortho",
            Vec3::new(-2.0, -9.0, 0.3),
            Projection::Orthographic { half_height: 1.4 },
        ),
    ] {
        let camera = Camera {
            eye,
            target: Vec3::ZERO,
            up: Vec3::Z,
            projection,
        };
        for opacity in [0.1, 0.9] {
            let params = RenderParams {
                opacity,
                ..params(320, 200, 1)
            };
            check(
                &format!("edge-on-{name}-{opacity}"),
                &mesh,
                &camera,
                &params,
            );
        }
    }
}

/// Lines over a see-through picture: drawn on the blended picture, tested
/// against the front's depth as over an opaque one. Hidden and asked to be
/// dimmed (what the viewport asks of a body's edges when the bodies are
/// see-through) a line is blended at the hidden opacity; hidden and not
/// dimmed it is left out; in front it is whole.
#[test]
fn lines_over_a_see_through_picture_are_tested_against_the_front() {
    // A slab seen from -y: it covers x in [-20, 20] and z in [-15, 15].
    let mut mesh = Mesh::default();
    for (y, face) in [(-10.0, 0), (10.0, 1)] {
        quad(
            &mut mesh,
            [
                Vec3::new(-20.0, y, -15.0),
                Vec3::new(20.0, y, -15.0),
                Vec3::new(20.0, y, 15.0),
                Vec3::new(-20.0, y, 15.0),
            ],
            face,
        );
    }
    let params = RenderParams {
        opacity: 0.4,
        ..params(400, 300, 1)
    };
    let camera = Camera {
        eye: Vec3::new(0.0, -200.0, 0.0),
        target: Vec3::ZERO,
        up: Vec3::Z,
        projection: Projection::Orthographic { half_height: 30.0 },
    };
    let ink = [255, 255, 255];
    let line = |y: f64, z: f64, dimmed: bool| Segment {
        from: Vec3::new(-36.0, y, z),
        to: Vec3::new(36.0, y, z),
        color: ink,
        width: 3.0,
        tested: true,
        dimmed,
        axis: false,
        edge: None,
    };
    let segments = [
        // Between the two sheets: hidden by the front one.
        line(0.0, 8.0, true),
        line(0.0, 0.0, false),
        // In front of both.
        line(-30.0, -8.0, false),
    ];
    let px = |x: f64| 200.0 + x * 5.0;
    let py = |z: f64| 150.0 - z * 5.0;
    let cpu = craft3d_render::render(&mesh, &camera, &params).unwrap();
    for (adapter, device, queue) in devices() {
        let mut onscreen = Onscreen::new(&device, &queue, &adapter).unwrap();
        let label = format!(
            "{} with {} samples",
            adapter.get_info().name,
            onscreen.samples()
        );
        let uploaded = onscreen.upload(&mesh).unwrap();
        let overlay = OverlayParams {
            slack: 200.0 * 1e-6,
            hidden_opacity: 70,
            axis_length: 1.0,
            edges: craft3d_gpu::EdgeLook::default(),
        };
        let mut picture = |segments: &[Segment]| {
            let lines = onscreen.upload_lines(segments).unwrap();
            onscreen
                .draw(
                    &uploaded,
                    &lines,
                    &camera,
                    &params,
                    &overlay,
                    DrawOptions::default(),
                )
                .unwrap();
            onscreen.read_rgb().unwrap()
        };
        let plain = picture(&[]);
        let lined = picture(&segments);
        let png = craft3d_render::encode_png(params.width, params.height, &lined).unwrap();
        std::fs::write(evidence("layers-lines.png"), png).unwrap();
        let at = |rgb: &[u8], x: f64, z: f64| {
            let i = ((py(z) as usize) * 400 + px(x) as usize) * 3;
            [rgb[i], rgb[i + 1], rgb[i + 2]]
        };
        for x in [-15.0, -5.0, 5.0, 15.0] {
            // The picture under the lines is the CPU's see-through one.
            let under = at(&plain, x, 4.0);
            let expected = at(&cpu.rgb, x, 4.0);
            assert!(
                under.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 1),
                "{label}: {under:?} for {expected:?}"
            );
            // Hidden and dimmed: the ink at 70 of 255 over the picture.
            let under = at(&plain, x, 8.0);
            let dimmed = at(&lined, x, 8.0);
            for c in 0..3 {
                let blend =
                    f64::from(under[c]) + (f64::from(ink[c]) - f64::from(under[c])) * 70.0 / 255.0;
                assert!(
                    (f64::from(dimmed[c]) - blend).abs() <= 1.5,
                    "{label}: a hidden line at x {x} is {dimmed:?} over {under:?}"
                );
            }
            // Hidden and not dimmed: left out.
            assert_eq!(at(&lined, x, 0.0), at(&plain, x, 0.0), "{label} at x {x}");
            // In front: whole.
            assert_eq!(at(&lined, x, -8.0), ink, "{label} at x {x}");
        }
        // Beside the slab nothing hides a line.
        for x in [-30.0, 30.0] {
            assert_eq!(at(&lined, x, 8.0), ink, "{label} at x {x}");
            assert_eq!(at(&lined, x, 0.0), ink, "{label} at x {x}");
        }
    }
}

/// Opacities that are not opacities, a renderer told it cannot draw
/// see-through bodies, and bodies that are not shaded: an error or the
/// opaque picture, never a panic.
#[test]
fn hostile_opacities_and_a_device_without_see_through_are_handled() {
    let mut mesh = Mesh::default();
    square(&mut mesh, 1.0, 0.0, 0);
    square(&mut mesh, 0.6, -1.0, 1);
    let plain = params(96, 64, 1);
    let camera = view(
        Vec3::new(1.0, -2.0, 6.0),
        Projection::Perspective { fov_y_deg: 30.0 },
    );
    let with = |opacity: f64| RenderParams { opacity, ..plain };
    for (adapter, device, queue) in devices() {
        let name = adapter.get_info().name;
        let mut onscreen = Onscreen::new(&device, &queue, &adapter).unwrap();
        let uploaded = onscreen.upload(&mesh).unwrap();
        let lines = onscreen.upload_lines(&[]).unwrap();
        let picture = |onscreen: &mut Onscreen, params: &RenderParams, bodies: bool| {
            onscreen
                .draw(
                    &uploaded,
                    &lines,
                    &camera,
                    params,
                    &no_overlay(),
                    DrawOptions {
                        bodies,
                        capture: false,
                    },
                )
                .map(|()| onscreen.read_rgb().unwrap())
        };
        let opaque = picture(&mut onscreen, &plain, true).unwrap();
        // Not a number, infinite, above 1: opaque. Below 0: clear.
        for opacity in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1.0, 1.5, 1e300] {
            assert_eq!(
                picture(&mut onscreen, &with(opacity), true).unwrap(),
                opaque,
                "{name}: opacity {opacity}"
            );
        }
        let clear = picture(&mut onscreen, &with(0.0), true).unwrap();
        assert_ne!(clear, opaque, "{name}");
        for opacity in [-0.0, -1.0, -1e300, f64::MIN_POSITIVE / 2.0] {
            assert_eq!(
                picture(&mut onscreen, &with(opacity), true).unwrap(),
                clear,
                "{name}: opacity {opacity}"
            );
        }
        // Bodies that only fill the depth buffer: nothing to see through.
        let unshaded = picture(&mut onscreen, &plain, false).unwrap();
        assert_eq!(picture(&mut onscreen, &with(0.3), false).unwrap(), unshaded);
        // Sizes change between see-through frames, and a size that is
        // none is the CPU renderer's error.
        for (width, height) in [(31, 17), (200, 150), (1, 1)] {
            let resized = RenderParams {
                width,
                height,
                ..with(0.5)
            };
            assert_eq!(
                picture(&mut onscreen, &resized, true).unwrap().len(),
                (width * height * 3) as usize
            );
        }
        for (width, height) in [(0, 10), (10, 0), (0, 0)] {
            let none = RenderParams {
                width,
                height,
                ..with(0.5)
            };
            assert!(matches!(
                picture(&mut onscreen, &none, true),
                Err(GpuError::Render(_))
            ));
        }
        // A device that cannot: refused with the reason, and the opaque
        // picture is still drawn.
        assert_eq!(onscreen.see_through(), Ok(()));
        onscreen.without_see_through();
        assert!(onscreen.see_through().is_err());
        let refused = picture(&mut onscreen, &with(0.5), true).unwrap_err();
        assert!(matches!(refused, GpuError::SeeThrough(_)), "{refused}");
        assert!(refused.to_string().contains("see-through"), "{refused}");
        assert_eq!(picture(&mut onscreen, &plain, true).unwrap(), opaque);
        assert_eq!(
            picture(&mut onscreen, &with(f64::NAN), true).unwrap(),
            opaque
        );
    }
    // The renderer that reads frames back, the same.
    for mut offscreen in offscreens() {
        let opaque = offscreen.render(&mesh, &camera, &plain).unwrap();
        let clear = offscreen.render(&mesh, &camera, &with(0.0)).unwrap();
        assert_ne!(clear, opaque);
        assert_eq!(
            offscreen.render(&mesh, &camera, &with(-5.0)).unwrap(),
            clear
        );
        for (width, height) in [(0, 10), (10, 0)] {
            let none = RenderParams {
                width,
                height,
                ..with(0.5)
            };
            assert!(matches!(
                offscreen.render(&mesh, &camera, &none),
                Err(GpuError::Render(_))
            ));
        }
        offscreen.renderer().without_see_through();
        let refused = offscreen.render(&mesh, &camera, &with(0.5)).unwrap_err();
        assert!(matches!(refused, GpuError::SeeThrough(_)), "{refused}");
        assert_eq!(offscreen.render(&mesh, &camera, &plain).unwrap(), opaque);
    }
}
