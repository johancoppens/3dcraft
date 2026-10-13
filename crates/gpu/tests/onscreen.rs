//! The viewport renderer (`Onscreen`): its multisampled picture against
//! the CPU reference renderer, its lines against the rule the CPU viewport
//! hides lines by, and what it refuses.
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
    compare_msaa, compare_msaa_see_through, evidence, orthographic, params, perspective,
    write_evidence,
};
use craft3d_gpu::{
    Camera, DrawOptions, GpuError, Mesh, Onscreen, OverlayParams, Projection, RenderParams, Segment,
};
use craft3d_math::{Frame, Vec3};
use craft3d_tess::{TessParams, tessellate_body, tessellate_patches};
use craft3d_testkit::{solids, teapot::teapot};
use devices::devices;

fn name_of(adapter: &wgpu::Adapter) -> String {
    let info = adapter.get_info();
    format!("{} ({:?})", info.name, info.backend)
}

fn no_overlay() -> OverlayParams {
    OverlayParams {
        slack: 0.0,
        hidden_opacity: 70,
        axis_length: 1.0,
    }
}

fn teapot_mesh() -> Mesh {
    tessellate_patches(&teapot().unwrap(), &TessParams::default()).unwrap()
}

/// A block from (-20, -10, -15) to (20, 10, 15).
fn block_mesh() -> Mesh {
    let frame = Frame::new(Vec3::new(-20.0, -10.0, -15.0), Vec3::Z, Vec3::X).unwrap();
    let block = solids::block(&frame, 40.0, 20.0, 30.0).unwrap();
    tessellate_body(&block.body, &TessParams::default()).unwrap()
}

#[test]
fn the_multisampled_picture_matches_the_cpu_renderer_away_from_the_edges() {
    let teapot = teapot_mesh();
    let block = block_mesh();
    let scenes: Vec<(&str, &Mesh, RenderParams, Camera)> = vec![
        {
            let p = params(640, 400, 1);
            let c = perspective(&teapot, Vec3::new(0.35, -1.0, 0.45), 30.0, &p);
            ("teapot-perspective", &teapot, p, c)
        },
        {
            let p = params(333, 277, 1);
            let c = orthographic(&teapot, Vec3::new(-1.0, 0.4, 0.2), &p);
            ("teapot-orthographic", &teapot, p, c)
        },
        {
            let p = params(480, 320, 1);
            let c = perspective(&block, Vec3::new(1.0, -0.8, 0.6), 40.0, &p);
            ("block-perspective", &block, p, c)
        },
    ];
    let mut log = String::new();
    let mut compared = 0;
    for (adapter, device, queue) in devices() {
        let adapter_name = name_of(&adapter);
        let mut onscreen = Onscreen::new(&device, &queue, &adapter).unwrap();
        println!("{adapter_name}: {} samples", onscreen.samples());
        // One on OpenGL (see `Onscreen::new` and `gl_multisample.rs`),
        // four wherever else this test has run.
        let expected = if adapter.get_info().backend == wgpu::Backend::Gl {
            1
        } else {
            craft3d_gpu::WANTED_SAMPLES
        };
        assert_eq!(onscreen.samples(), expected, "{adapter_name}");
        let lines = onscreen.upload_lines(&[]).unwrap();
        for (name, mesh, params, camera) in &scenes {
            let cpu = craft3d_render::render(mesh, camera, params).unwrap();
            let uploaded = onscreen.upload(mesh).unwrap();
            onscreen
                .draw(
                    &uploaded,
                    &lines,
                    camera,
                    params,
                    &no_overlay(),
                    DrawOptions {
                        bodies: true,
                        capture: true,
                    },
                )
                .unwrap();
            let gpu = onscreen.read_frame().unwrap();
            // The same picture without the capture pass: the capture is
            // only an addition.
            onscreen
                .draw(
                    &uploaded,
                    &lines,
                    camera,
                    params,
                    &no_overlay(),
                    DrawOptions::default(),
                )
                .unwrap();
            assert_eq!(onscreen.read_rgb().unwrap(), gpu.rgb);
            let (parity, band) = compare_msaa(mesh, camera, &cpu, &gpu);
            let label = format!("onscreen {name} on {adapter_name}");
            println!("{label}: {parity:?}");
            log.push_str(&format!("{label}: {parity:?}\n"));
            let path = write_evidence(&format!("onscreen-{name}.png"), &cpu, &gpu, &band);
            println!("evidence: {}", path.display());
            parity.check(&label);
            if onscreen.samples() > 1 {
                // The anti-aliasing shows: edge pixels are blended.
                assert!(
                    parity.color_band > parity.band / 20,
                    "no edge pixel is blended: {label}: {parity:?}"
                );
            }
            compared += 1;
        }
    }
    std::fs::write(evidence("onscreen.txt"), log).unwrap();
    assert!(compared >= scenes.len());
}

/// A block with a cavity and a sphere in the cavity.
fn nested_mesh() -> Mesh {
    let frame = |x, y, z| Frame::new(Vec3::new(x, y, z), Vec3::Z, Vec3::X).unwrap();
    let tess = TessParams {
        chordal_tolerance: 0.01,
        ..TessParams::default()
    };
    let parts = [
        solids::block(&frame(-4.0, -3.0, -2.5), 8.0, 6.0, 5.0).unwrap(),
        solids::block(&frame(-3.5, -2.5, -2.0), 7.0, 5.0, 4.0).unwrap(),
        solids::sphere(&frame(-1.2, 0.3, 0.0), 1.4).unwrap(),
    ];
    let mut out = Mesh::default();
    let mut first_face = 0;
    for part in &parts {
        let part = tessellate_body(&part.body, &tess).unwrap();
        let base = out.positions.len() as u32;
        out.positions.extend(&part.positions);
        out.normals.extend(&part.normals);
        out.triangles
            .extend(part.triangles.iter().map(|t| t.map(|k| k + base)));
        out.face_ids
            .extend(part.face_ids.iter().map(|f| f + first_face));
        first_face += part.face_ids.iter().copied().max().map_or(0, |m| m + 1);
    }
    out
}

/// See-through bodies in the viewport's picture, against the CPU renderer:
/// the teapot, a block and a body inside a hollow body at three opacities,
/// plain and with faces highlighted.
#[test]
fn the_see_through_picture_matches_the_cpu_renderer_away_from_the_edges() {
    let teapot = teapot_mesh();
    let block = block_mesh();
    let nested = nested_mesh();
    let mut scenes: Vec<(String, &Mesh, RenderParams, Camera)> = Vec::new();
    for (k, opacity) in [0.1, 0.5, 0.9].into_iter().enumerate() {
        for (name, mesh, size, toward_eye) in [
            ("teapot", &teapot, (640, 400), Vec3::new(0.35, -1.0, 0.45)),
            ("block", &block, (480, 320), Vec3::new(1.0, -0.8, 0.6)),
            ("nested", &nested, (400, 300), Vec3::new(1.0, -1.6, 0.8)),
        ] {
            let mut p = RenderParams {
                opacity,
                ..params(size.0, size.1, 1)
            };
            let highlighted = (k + name.len()) % 2 == 0;
            if highlighted {
                let faces = mesh.face_ids.iter().copied().max().unwrap() + 1;
                p.highlight = craft3d_gpu::Highlight::new([255, 150, 40], (0..faces).step_by(2)).0;
            }
            let c = if k == 1 {
                orthographic(mesh, toward_eye, &p)
            } else {
                perspective(mesh, toward_eye, 30.0, &p)
            };
            let name = format!(
                "{name}-{opacity}{}",
                if highlighted { "-highlight" } else { "" }
            );
            scenes.push((name, mesh, p, c));
        }
    }
    let mut log = String::new();
    let mut compared = 0;
    // Checked after every scene was drawn, so that a failure leaves the
    // numbers and the pictures of all of them.
    let mut measured = Vec::new();
    for (adapter, device, queue) in devices() {
        let adapter_name = name_of(&adapter);
        let mut onscreen = Onscreen::new(&device, &queue, &adapter).unwrap();
        assert_eq!(onscreen.see_through(), Ok(()), "{adapter_name}");
        let lines = onscreen.upload_lines(&[]).unwrap();
        for (name, mesh, params, camera) in &scenes {
            let cpu = craft3d_render::render(mesh, camera, params).unwrap();
            let uploaded = onscreen.upload(mesh).unwrap();
            let options = DrawOptions {
                bodies: true,
                capture: true,
            };
            onscreen
                .draw(&uploaded, &lines, camera, params, &no_overlay(), options)
                .unwrap();
            let gpu = onscreen.read_frame().unwrap();
            let (parity, band) = compare_msaa_see_through(mesh, camera, params, &cpu, &gpu);
            let label = format!("onscreen see-through {name} on {adapter_name}");
            println!("{label}: {parity:?}");
            log.push_str(&format!("{label}: {parity:?}\n"));
            let backend = format!("{:?}", adapter.get_info().backend).to_lowercase();
            let kind = format!("{:?}", adapter.get_info().device_type).to_lowercase();
            let file = format!("onscreen-seethrough-{name}-{backend}-{kind}.png");
            let path = write_evidence(&file, &cpu, &gpu, &band);
            println!("evidence: {}", path.display());
            measured.push((label.clone(), parity));
            // The opaque picture afterwards is what it was before any
            // see-through one: the layers leave nothing behind.
            let opaque = RenderParams {
                opacity: 1.0,
                ..*params
            };
            let mut fresh = Onscreen::new(&device, &queue, &adapter).unwrap();
            let fresh_mesh = fresh.upload(mesh).unwrap();
            let fresh_lines = fresh.upload_lines(&[]).unwrap();
            fresh
                .draw(
                    &fresh_mesh,
                    &fresh_lines,
                    camera,
                    &opaque,
                    &no_overlay(),
                    options,
                )
                .unwrap();
            onscreen
                .draw(&uploaded, &lines, camera, &opaque, &no_overlay(), options)
                .unwrap();
            assert_eq!(
                onscreen.read_frame().unwrap(),
                fresh.read_frame().unwrap(),
                "{label}"
            );
            compared += 1;
        }
    }
    std::fs::write(evidence("onscreen-seethrough.txt"), log).unwrap();
    for (label, parity) in &measured {
        parity.check(label);
    }
    assert!(compared >= scenes.len());
}

/// The colour of the pixel under (`x`, `y`) of an RGB picture.
fn at(rgb: &[u8], width: u32, x: f64, y: f64) -> [u8; 3] {
    let (px, py) = (x.floor() as usize, y.floor() as usize);
    let i = (py * width as usize + px) * 3;
    [rgb[i], rgb[i + 1], rgb[i + 2]]
}

#[test]
fn lines_are_hidden_behind_a_body_and_dimmed_or_left_out_as_asked() {
    // The block seen from the front (+Y towards the eye, hmm: from -Y),
    // orthographically: it covers x in [-20, 20] and z in [-15, 15] on the
    // screen. Lines along x at z = 0: one behind the block (y = 30), one in
    // front (y = -30), one on its front face (y = -10).
    let mesh = block_mesh();
    let params = params(400, 300, 1);
    let camera = Camera {
        eye: Vec3::new(0.0, -200.0, 0.0),
        target: Vec3::ZERO,
        up: Vec3::Z,
        projection: Projection::Orthographic { half_height: 30.0 },
    };
    let yellow = [255, 196, 64];
    let white = [255, 255, 255];
    let line = |y: f64, z: f64, color: [u8; 3], tested: bool, dimmed: bool| Segment {
        from: Vec3::new(-36.0, y, z),
        to: Vec3::new(36.0, y, z),
        color,
        width: 3.0,
        tested,
        dimmed,
        axis: false,
    };
    let segments = [
        // A sketch line behind the block: dimmed where the block is.
        line(30.0, 0.0, yellow, true, true),
        // An edge behind the block: left out where the block is.
        line(30.0, 8.0, white, true, false),
        // In front: whole.
        line(-30.0, -8.0, yellow, true, true),
        // On the front face: whole (the slack and the neighbourhood).
        line(-10.0, 4.0, white, true, false),
        // Not tested: whole wherever it is.
        line(30.0, -4.0, white, false, false),
        // A dot behind the block, and one beside it.
        Segment {
            from: Vec3::new(0.0, 30.0, 12.0),
            to: Vec3::new(0.0, 30.0, 12.0),
            color: yellow,
            width: 8.0,
            tested: true,
            dimmed: true,
            axis: false,
        },
        Segment {
            from: Vec3::new(30.0, 30.0, 12.0),
            to: Vec3::new(30.0, 30.0, 12.0),
            color: yellow,
            width: 8.0,
            tested: true,
            dimmed: true,
            axis: false,
        },
    ];
    // Model x and z to pixels: 60 model units over 300 pixels.
    let px = |x: f64| 200.0 + x * 5.0;
    let py = |z: f64| 150.0 - z * 5.0;
    for (adapter, device, queue) in devices() {
        // Multisampling is not drawn on OpenGL (see `Onscreen::new`).
        let counts: &[u32] = if adapter.get_info().backend == wgpu::Backend::Gl {
            &[1]
        } else {
            &[1, 4]
        };
        for &samples in counts {
            let mut onscreen = Onscreen::with_samples(&device, &queue, samples).unwrap();
            let uploaded = onscreen.upload(&mesh).unwrap();
            let lines = onscreen.upload_lines(&segments).unwrap();
            assert_eq!(lines.segment_count(), segments.len());
            let overlay = OverlayParams {
                slack: 200.0 * 1e-6,
                hidden_opacity: 70,
                axis_length: 1.0,
            };
            // Depth only: the lines on black show what the test decided.
            onscreen
                .draw(
                    &uploaded,
                    &lines,
                    &camera,
                    &RenderParams {
                        background: [0, 0, 0],
                        ..params
                    },
                    &overlay,
                    DrawOptions {
                        bodies: false,
                        capture: false,
                    },
                )
                .unwrap();
            let rgb = onscreen.read_rgb().unwrap();
            let png = craft3d_render::encode_png(params.width, params.height, &rgb).unwrap();
            std::fs::write(evidence(&format!("onscreen-lines-{samples}.png")), png).unwrap();
            let label = format!("{} with {samples} samples", name_of(&adapter));
            let classify = |x: f64, z: f64| -> &'static str {
                let [_, g, _] = at(&rgb, params.width, px(x) + 0.5, py(z) + 0.5);
                if g > 150 {
                    "full"
                } else if g > 25 {
                    "dimmed"
                } else {
                    "none"
                }
            };
            for x in [-15.0, -5.0, 5.0, 15.0] {
                assert_eq!(
                    classify(x, 0.0),
                    "dimmed",
                    "{label}: sketch behind at x {x}"
                );
                assert_eq!(classify(x, 8.0), "none", "{label}: edge behind at x {x}");
                assert_eq!(classify(x, -8.0), "full", "{label}: in front at x {x}");
                assert_eq!(classify(x, 4.0), "full", "{label}: on the face at x {x}");
                assert_eq!(classify(x, -4.0), "full", "{label}: untested at x {x}");
            }
            for x in [-33.0, -25.0, 25.0, 33.0] {
                assert_eq!(classify(x, 0.0), "full", "{label}: sketch beside at x {x}");
                assert_eq!(classify(x, 8.0), "full", "{label}: edge beside at x {x}");
            }
            assert_eq!(classify(0.0, 12.0), "dimmed", "{label}: dot behind");
            assert_eq!(classify(30.0, 12.0), "full", "{label}: dot beside");
            // Between the lines: nothing.
            assert_eq!(classify(0.0, 2.0), "none", "{label}: between lines");
            // The ends are round: just past an end the line is gone.
            assert_eq!(classify(37.6, -8.0), "none", "{label}: past the end");
        }
    }
}

#[test]
fn what_the_renderer_refuses_and_what_it_takes_in_its_stride() {
    let mesh = block_mesh();
    let params = params(64, 48, 1);
    let camera = perspective(&mesh, Vec3::new(1.0, -1.0, 1.0), 30.0, &params);
    for (adapter, device, queue) in devices() {
        assert!(matches!(
            Onscreen::with_samples(&device, &queue, 3),
            Err(GpuError::Unsupported(_))
        ));
        let mut onscreen = Onscreen::new(&device, &queue, &adapter).unwrap();
        assert!(onscreen.view().is_none() && onscreen.size().is_none());
        assert!(matches!(onscreen.read_rgb(), Err(GpuError::NothingDrawn)));
        let uploaded = onscreen.upload(&mesh).unwrap();
        let lines = onscreen.upload_lines(&[]).unwrap();
        // Supersampling is not what this renderer does.
        assert!(matches!(
            onscreen.draw(
                &uploaded,
                &lines,
                &camera,
                &params_with(&params, 2),
                &no_overlay(),
                DrawOptions::default()
            ),
            Err(GpuError::Unsupported(_))
        ));
        // Sizes the CPU renderer refuses too.
        for (width, height) in [(0, 10), (10, 0), (craft3d_render::MAX_SIDE + 1, 10)] {
            let bad = RenderParams {
                width,
                height,
                ..params
            };
            assert!(matches!(
                onscreen.draw(
                    &uploaded,
                    &lines,
                    &camera,
                    &bad,
                    &no_overlay(),
                    DrawOptions::default()
                ),
                Err(GpuError::Render(_))
            ));
        }
        // Another renderer's mesh or lines.
        let other = Onscreen::new(&device, &queue, &adapter).unwrap();
        let foreign_mesh = other.upload(&mesh).unwrap();
        let foreign_lines = other.upload_lines(&[]).unwrap();
        assert_eq!(
            onscreen
                .draw(
                    &foreign_mesh,
                    &lines,
                    &camera,
                    &params,
                    &no_overlay(),
                    DrawOptions::default()
                )
                .unwrap_err(),
            GpuError::ForeignMesh
        );
        assert_eq!(
            onscreen
                .draw(
                    &uploaded,
                    &foreign_lines,
                    &camera,
                    &params,
                    &no_overlay(),
                    DrawOptions::default()
                )
                .unwrap_err(),
            GpuError::ForeignMesh
        );
        // A degenerate camera is the CPU renderer's error.
        let degenerate = Camera {
            eye: Vec3::ZERO,
            target: Vec3::ZERO,
            up: Vec3::Z,
            projection: Projection::Perspective { fov_y_deg: 30.0 },
        };
        assert!(matches!(
            onscreen.draw(
                &uploaded,
                &lines,
                &degenerate,
                &params,
                &no_overlay(),
                DrawOptions::default()
            ),
            Err(GpuError::Render(_))
        ));
        // Nothing of that counts as drawn.
        assert!(onscreen.view().is_none());
        assert!(matches!(onscreen.read_frame(), Err(GpuError::NothingDrawn)));

        // Hostile lines are left out, not refused; a hostile overlay hides
        // nothing and breaks nothing.
        let hostile = [
            Segment {
                from: Vec3::new(f64::NAN, 0.0, 0.0),
                to: Vec3::X,
                color: [1, 2, 3],
                width: 2.0,
                tested: true,
                dimmed: true,
                axis: false,
            },
            Segment {
                from: Vec3::new(1e300, 0.0, 0.0),
                to: Vec3::new(-1e300, 0.0, 0.0),
                color: [1, 2, 3],
                width: f32::INFINITY,
                tested: true,
                dimmed: false,
                axis: true,
            },
            Segment {
                from: Vec3::ZERO,
                to: Vec3::X,
                color: [255, 255, 255],
                width: -5.0,
                tested: false,
                dimmed: false,
                axis: true,
            },
        ];
        let lines = onscreen.upload_lines(&hostile).unwrap();
        assert_eq!(lines.segment_count(), 1);
        for overlay in [
            OverlayParams {
                slack: f64::NAN,
                hidden_opacity: 255,
                axis_length: f64::INFINITY,
            },
            OverlayParams {
                slack: -1.0,
                hidden_opacity: 0,
                axis_length: f64::NAN,
            },
            OverlayParams {
                slack: 1e300,
                hidden_opacity: 70,
                axis_length: 1e300,
            },
        ] {
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
        }
        assert_eq!(onscreen.size(), Some((64, 48)));
        assert_eq!(onscreen.read_rgb().unwrap().len(), 64 * 48 * 3);
        // The frame needs the capture pass.
        assert!(matches!(onscreen.read_frame(), Err(GpuError::NothingDrawn)));
        onscreen
            .draw(
                &uploaded,
                &lines,
                &camera,
                &params,
                &no_overlay(),
                DrawOptions {
                    bodies: true,
                    capture: true,
                },
            )
            .unwrap();
        let frame = onscreen.read_frame().unwrap();
        assert_eq!((frame.width, frame.height), (64, 48));
        assert!(frame.coverage() > 0.1);

        // Too many segments are refused before anything is allocated.
        let many = vec![hostile[2]; craft3d_gpu::MAX_SEGMENTS + 1];
        assert!(matches!(
            onscreen.upload_lines(&many),
            Err(GpuError::TooManyLines { .. })
        ));
    }
}

fn params_with(params: &RenderParams, ssaa: u32) -> RenderParams {
    RenderParams { ssaa, ..*params }
}
