//! The viewport renderer as the browser build runs it on WebGL2: wgpu's
//! OpenGL backend on an OpenGL ES 3.0 context (GLSL ES 3.00), with
//! WebGL2's limits, one sample per pixel and the depth written by the
//! fragment, against the CPU reference renderer. Most Linux browsers have
//! WebGPU off, so this is what the web build draws with there.
//!
//! The context is Mesa's, made to be ES 3.0 through its environment
//! (`MESA_GL_VERSION_OVERRIDE` keeps wgpu from the desktop context it
//! prefers, `MESA_GLES_VERSION_OVERRIDE` caps OpenGL ES at 3.0, so the
//! driver reports GLSL ES 3.00 and wgpu translates every shader to it);
//! the environment decides that when the context is made, so the test
//! runs itself again in a child process with it. Mesa is the driver of
//! every Linux machine the tests run on (radeonsi, Intel's iris, or
//! llvmpipe without a GPU); the test fails, with the reason, where no
//! OpenGL ES 3.0 context comes out.
//!
//! Mesa does not apply ES 3.0's rule that a depth texture whose filter is
//! not the nearest texel is incomplete (a browser's WebGL2 does, and reads
//! 0 from it), so this pins the shading language, the limits and the
//! pictures, not that rule: `renderer.rs` › `template_tests` checks that
//! no pass OpenGL runs loads a depth texture.

#![cfg(target_os = "linux")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{
    compare_msaa, compare_msaa_see_through, compare_msaa_with, evidence, orthographic, params,
    perspective, write_evidence,
};
use craft3d_gpu::{
    Camera, DrawOptions, Highlight, Mesh, Onscreen, OverlayParams, Projection, RenderParams,
    Section, Segment,
};
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_tess::{TessParams, tessellate_body, tessellate_patches};
use craft3d_testkit::{solids, teapot::teapot};

/// Marks the child process.
const CHILD: &str = "CRAFT3D_GPU_TEST_WEBGL2";

/// A device with WebGL2's limits on every adapter of the OpenGL backend,
/// each of which must be an OpenGL ES 3.0 context.
fn webgl2_devices() -> Vec<(wgpu::Adapter, wgpu::Device, wgpu::Queue)> {
    let adapters = craft3d_gpu::adapters();
    assert!(
        !adapters.is_empty(),
        "no OpenGL ES 3.0 adapter: wgpu's OpenGL backend found none with \
         MESA_GL_VERSION_OVERRIDE=3.0 MESA_GLES_VERSION_OVERRIDE=3.0 (Mesa's EGL is needed)"
    );
    adapters
        .into_iter()
        .map(|adapter| {
            let info = adapter.get_info();
            println!(
                "adapter: {} ({:?}), {}",
                info.name, info.backend, info.driver_info
            );
            assert_eq!(info.backend, wgpu::Backend::Gl, "{}", info.name);
            assert!(
                info.driver_info.starts_with("OpenGL ES 3.0 "),
                "{}: not an OpenGL ES 3.0 context: {}",
                info.name,
                info.driver_info
            );
            // No compute shaders below OpenGL ES 3.1: what WebGL2 lacks.
            assert!(
                !adapter
                    .get_downlevel_capabilities()
                    .flags
                    .contains(wgpu::DownlevelFlags::COMPUTE_SHADERS),
                "{}",
                info.name
            );
            let (device, queue) = pollster::block_on(
                adapter.request_device(&wgpu::DeviceDescriptor {
                    label: Some("craft3d webgl2 test"),
                    required_features: wgpu::Features::empty(),
                    // What egui-wgpu asks of a WebGL2 device in a browser.
                    required_limits: wgpu::Limits::downlevel_webgl2_defaults()
                        .using_resolution(adapter.limits()),
                    experimental_features: wgpu::ExperimentalFeatures::default(),
                    memory_hints: wgpu::MemoryHints::default(),
                    trace: wgpu::Trace::default(),
                }),
            )
            .unwrap_or_else(|e| panic!("cannot use adapter {:?}: {e}", info.name));
            device.on_uncaptured_error(std::sync::Arc::new(|error: wgpu::Error| {
                panic!("wgpu reported an error outside a scope: {error}");
            }));
            (adapter, device, queue)
        })
        .collect()
}

fn no_overlay() -> OverlayParams {
    OverlayParams {
        slack: 0.0,
        hidden_opacity: 70,
        axis_length: 1.0,
        edges: craft3d_gpu::EdgeLook::default(),
    }
}

fn at(x: f64, y: f64, z: f64) -> Frame {
    Frame::new(Vec3::new(x, y, z), Vec3::Z, Vec3::X).unwrap()
}

fn teapot_mesh() -> Mesh {
    tessellate_patches(&teapot().unwrap(), &TessParams::default()).unwrap()
}

/// A block from (-20, -10, -15) to (20, 10, 15).
fn block_mesh() -> Mesh {
    let block = solids::block(&at(-20.0, -10.0, -15.0), 40.0, 20.0, 30.0).unwrap();
    tessellate_body(&block.body, &TessParams::default()).unwrap()
}

/// A block with a closed cavity in it, made by a boolean.
fn hollow_block() -> Mesh {
    let outer = primitives::block(&at(-3.0, -2.0, -1.5), 6.0, 4.0, 3.0).unwrap();
    let inner = primitives::block(&at(-2.0, -1.0, -1.0), 4.0, 2.0, 2.0).unwrap();
    let hollow = boolean(&outer, &inner, BooleanOp::Subtract, &Tolerance::default()).unwrap();
    tessellate_body(
        &hollow.body,
        &TessParams {
            chordal_tolerance: 0.01,
            ..TessParams::default()
        },
    )
    .unwrap()
}

/// What one comparison is: opaque, see-through or cut open.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Opaque,
    SeeThrough,
    Cut,
}

/// The scenes: opaque pictures, see-through ones at three opacities (every
/// other with faces highlighted) and cut-open ones, opaque and seen
/// through.
fn scenes() -> Vec<(String, Kind, Mesh, RenderParams, Camera)> {
    let teapot = teapot_mesh();
    let block = block_mesh();
    let hollow = hollow_block();
    let mut out = Vec::new();
    for (name, mesh, toward_eye) in [
        ("teapot", &teapot, Vec3::new(0.35, -1.0, 0.45)),
        ("block", &block, Vec3::new(1.0, -0.8, 0.6)),
    ] {
        let p = params(480, 320, 1);
        let camera = perspective(mesh, toward_eye, 30.0, &p);
        out.push((
            format!("{name}-opaque"),
            Kind::Opaque,
            mesh.clone(),
            p,
            camera,
        ));
    }
    for (k, opacity) in [0.1, 0.5, 0.9].into_iter().enumerate() {
        for (name, mesh, toward_eye) in [
            ("teapot", &teapot, Vec3::new(0.35, -1.0, 0.45)),
            ("block", &block, Vec3::new(1.0, -0.8, 0.6)),
            ("hollow", &hollow, Vec3::new(1.0, -1.6, 0.8)),
        ] {
            let mut p = RenderParams {
                opacity,
                ..params(400, 300, 1)
            };
            let highlighted = (k + name.len()) % 2 == 0;
            if highlighted {
                let faces = mesh.face_ids.iter().copied().max().unwrap() + 1;
                p.highlight = Highlight::new([255, 150, 40], (0..faces).step_by(2)).0;
            }
            let camera = if k == 1 {
                orthographic(mesh, toward_eye, &p)
            } else {
                perspective(mesh, toward_eye, 30.0, &p)
            };
            let name = format!(
                "{name}-{opacity}{}",
                if highlighted { "-highlight" } else { "" }
            );
            out.push((name, Kind::SeeThrough, mesh.clone(), p, camera));
        }
    }
    for (plane, origin, normal) in [
        ("z0", Vec3::ZERO, Vec3::Z),
        (
            "oblique",
            Vec3::new(0.3, -0.2, 0.1),
            Vec3::new(0.5, -0.7, 0.6),
        ),
    ] {
        for opacity in [1.0, 0.5] {
            let p = RenderParams {
                opacity,
                section: Some(Section {
                    origin,
                    normal,
                    cap: true,
                    color: [214, 132, 74],
                }),
                ..params(420, 300, 1)
            };
            let n = normal.normalized().unwrap();
            let tilt = Vec3::new(0.5, -0.7, 0.45);
            let camera = perspective(&hollow, n + (tilt - n * tilt.dot(n)) * 0.9, 30.0, &p);
            out.push((
                format!("section-{plane}-{opacity}"),
                Kind::Cut,
                hollow.clone(),
                p,
                camera,
            ));
        }
    }
    out
}

/// Every scene against the CPU renderer under the tolerances of the
/// viewport's other parity tests (`onscreen.rs`, `section.rs`).
fn pictures_match(adapter: &wgpu::Adapter, device: &wgpu::Device, queue: &wgpu::Queue) {
    let mut onscreen = Onscreen::new(device, queue, adapter).unwrap();
    assert_eq!(onscreen.samples(), 1);
    assert_eq!(onscreen.see_through(), Ok(()), "see-through on WebGL2");
    let lines = onscreen.upload_lines(&[]).unwrap();
    let options = DrawOptions {
        bodies: true,
        capture: true,
    };
    let mut log = String::new();
    let mut measured = Vec::new();
    for (name, kind, mesh, params, camera) in &scenes() {
        let cpu = craft3d_render::render(mesh, camera, params).unwrap();
        let uploaded = onscreen.upload(mesh).unwrap();
        onscreen
            .draw(&uploaded, &lines, camera, params, &no_overlay(), options)
            .unwrap();
        let gpu = onscreen.read_frame().unwrap();
        let (parity, band) = match kind {
            Kind::Opaque => compare_msaa(mesh, camera, &cpu, &gpu),
            Kind::SeeThrough => compare_msaa_see_through(mesh, camera, params, &cpu, &gpu),
            Kind::Cut => compare_msaa_with(mesh, camera, params, &cpu, &gpu),
        };
        let label = format!("webgl2 {name}");
        println!("{label}: {parity:?}");
        log.push_str(&format!("{label}: {parity:?}\n"));
        write_evidence(&format!("webgl2-{name}.png"), &cpu, &gpu, &band);
        measured.push((label, parity));
    }
    std::fs::write(evidence("webgl2.txt"), log).unwrap();
    for (label, parity) in &measured {
        parity.check(label);
    }
}

/// The colour of the pixel under (`x`, `y`) of an RGB picture.
fn pixel(rgb: &[u8], width: u32, x: f64, y: f64) -> [u8; 3] {
    let (px, py) = (x.floor() as usize, y.floor() as usize);
    let i = (py * width as usize + px) * 3;
    [rgb[i], rgb[i + 1], rgb[i + 2]]
}

/// The lines over the picture (the overlay's shaders: the distances, the
/// depth test, the edge density): hidden behind the block, dimmed or left
/// out as asked, whole in front of it.
fn lines_are_tested(adapter: &wgpu::Adapter, device: &wgpu::Device, queue: &wgpu::Queue) {
    let mesh = block_mesh();
    let params = RenderParams {
        background: [0, 0, 0],
        ..params(400, 300, 1)
    };
    let line = |y: f64, z: f64, dimmed: bool| Segment {
        from: Vec3::new(-36.0, y, z),
        to: Vec3::new(36.0, y, z),
        color: [255, 196, 64],
        width: 3.0,
        tested: true,
        dimmed,
        axis: false,
        hidden_opacity: None,
        edge: None,
    };
    let segments = [
        line(30.0, 0.0, true),
        line(30.0, 8.0, false),
        line(-30.0, -8.0, true),
        line(-10.0, 4.0, false),
    ];
    let mut onscreen = Onscreen::new(device, queue, adapter).unwrap();
    let uploaded = onscreen.upload(&mesh).unwrap();
    let lines = onscreen.upload_lines(&segments).unwrap();
    // Depth only: the lines on black show what the test decided.
    onscreen
        .draw(
            &uploaded,
            &lines,
            &front_view(),
            &params,
            &overlay(),
            DrawOptions {
                bodies: false,
                capture: false,
            },
        )
        .unwrap();
    let rgb = onscreen.read_rgb().unwrap();
    let png = craft3d_render::encode_png(params.width, params.height, &rgb).unwrap();
    std::fs::write(evidence("webgl2-lines.png"), png).unwrap();
    let classify = |x: f64, z: f64| -> &'static str {
        let [_, g, _] = pixel(&rgb, params.width, px(x) + 0.5, py(z) + 0.5);
        if g > 150 {
            "full"
        } else if g > 25 {
            "dimmed"
        } else {
            "none"
        }
    };
    for x in [-15.0, -5.0, 5.0, 15.0] {
        assert_eq!(classify(x, 0.0), "dimmed", "dimmed behind at x {x}");
        assert_eq!(classify(x, 8.0), "none", "edge behind at x {x}");
        assert_eq!(classify(x, -8.0), "full", "in front at x {x}");
        assert_eq!(classify(x, 4.0), "full", "on the face at x {x}");
    }
    for x in [-33.0, 33.0] {
        assert_eq!(classify(x, 0.0), "full", "beside at x {x}");
        assert_eq!(classify(x, 8.0), "full", "beside at x {x}");
    }
    assert_eq!(classify(0.0, 2.0), "none", "between the lines");
}

/// The block's front, orthographically: x in [-20, 20] and z in [-15, 15]
/// on a picture of 400 × 300 pixels.
fn front_view() -> Camera {
    Camera {
        eye: Vec3::new(0.0, -200.0, 0.0),
        target: Vec3::ZERO,
        up: Vec3::Z,
        projection: Projection::Orthographic { half_height: 30.0 },
    }
}

fn overlay() -> OverlayParams {
    OverlayParams {
        slack: 200.0 * 1e-6,
        hidden_opacity: 70,
        axis_length: 1.0,
        edges: craft3d_gpu::EdgeLook::default(),
    }
}

/// Model x to pixels of [`front_view`].
fn px(x: f64) -> f64 {
    200.0 + x * 5.0
}

/// Model z to pixels of [`front_view`].
fn py(z: f64) -> f64 {
    150.0 - z * 5.0
}

/// Lines over a see-through picture of two sheets: tested against the
/// front's depth (`see_through.rs` has the same on every adapter).
fn lines_over_see_through(adapter: &wgpu::Adapter, device: &wgpu::Device, queue: &wgpu::Queue) {
    let mut mesh = Mesh::default();
    for (y, face) in [(-10.0, 0_u32), (10.0, 1)] {
        let base = mesh.positions.len() as u32;
        mesh.positions.extend([
            Vec3::new(-20.0, y, -15.0),
            Vec3::new(20.0, y, -15.0),
            Vec3::new(20.0, y, 15.0),
            Vec3::new(-20.0, y, 15.0),
        ]);
        mesh.normals.extend([Vec3::ZERO; 4]);
        mesh.triangles.push([base, base + 1, base + 2]);
        mesh.triangles.push([base, base + 2, base + 3]);
        mesh.face_ids.extend([face, face]);
    }
    let params = RenderParams {
        opacity: 0.4,
        ..params(400, 300, 1)
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
        hidden_opacity: None,
        edge: None,
    };
    // Between the sheets, dimmed and left out; in front of both.
    let segments = [
        line(0.0, 8.0, true),
        line(0.0, 0.0, false),
        line(-30.0, -8.0, false),
    ];
    let cpu = craft3d_render::render(&mesh, &front_view(), &params).unwrap();
    let mut onscreen = Onscreen::new(device, queue, adapter).unwrap();
    let uploaded = onscreen.upload(&mesh).unwrap();
    let mut picture = |segments: &[Segment]| {
        let lines = onscreen.upload_lines(segments).unwrap();
        onscreen
            .draw(
                &uploaded,
                &lines,
                &front_view(),
                &params,
                &overlay(),
                DrawOptions::default(),
            )
            .unwrap();
        onscreen.read_rgb().unwrap()
    };
    let plain = picture(&[]);
    let lined = picture(&segments);
    let png = craft3d_render::encode_png(params.width, params.height, &lined).unwrap();
    std::fs::write(evidence("webgl2-lines-see-through.png"), png).unwrap();
    let at = |rgb: &[u8], x: f64, z: f64| pixel(rgb, params.width, px(x), py(z));
    for x in [-15.0, -5.0, 5.0, 15.0] {
        // The picture under the lines is the CPU's see-through one.
        let under = at(&plain, x, 4.0);
        let expected = at(&cpu.rgb, x, 4.0);
        assert!(
            under.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 1),
            "{under:?} for {expected:?} at x {x}"
        );
        // Hidden and dimmed: the ink at 70 of 255 over the picture.
        let under = at(&plain, x, 8.0);
        let dimmed = at(&lined, x, 8.0);
        for c in 0..3 {
            let blend =
                f64::from(under[c]) + (f64::from(ink[c]) - f64::from(under[c])) * 70.0 / 255.0;
            assert!(
                (f64::from(dimmed[c]) - blend).abs() <= 1.5,
                "a hidden line at x {x} is {dimmed:?} over {under:?}"
            );
        }
        // Hidden and not dimmed: left out. In front: whole.
        assert_eq!(at(&lined, x, 0.0), at(&plain, x, 0.0), "at x {x}");
        assert_eq!(at(&lined, x, -8.0), ink, "at x {x}");
    }
    for x in [-30.0, 30.0] {
        assert_eq!(at(&lined, x, 8.0), ink, "beside at x {x}");
    }
}

/// On an OpenGL ES 3.0 context with WebGL2's limits the viewport draws
/// opaque, see-through and cut-open pictures as the CPU renderer does, and
/// its lines are hidden as on every other backend.
#[test]
fn on_opengl_es_3_0_the_viewport_matches_the_cpu_renderer() {
    if std::env::var(CHILD).is_err() {
        let name = "on_opengl_es_3_0_the_viewport_matches_the_cpu_renderer";
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", name, "--nocapture", "--test-threads", "1"])
            .env(CHILD, "1")
            .env("WGPU_BACKEND", "gl")
            .env("MESA_GL_VERSION_OVERRIDE", "3.0")
            .env("MESA_GLES_VERSION_OVERRIDE", "3.0")
            .env_remove("WGPU_ADAPTER_NAME")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        println!("{stdout}");
        // The child ran this very test, and it passed.
        assert!(
            output.status.success() && stdout.contains("1 passed"),
            "the test on OpenGL ES 3.0 failed:\n{stdout}\n{stderr}"
        );
        return;
    }
    for (adapter, device, queue) in webgl2_devices() {
        pictures_match(&adapter, &device, &queue);
        lines_are_tested(&adapter, &device, &queue);
        lines_over_see_through(&adapter, &device, &queue);
    }
}
