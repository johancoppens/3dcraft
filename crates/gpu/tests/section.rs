//! The section plane on the GPU (`RenderParams::section`) against the CPU
//! reference renderer, on every adapter of this machine: parity under the
//! tolerances of `common/mod.rs` (*Cut-open pictures*), the area of the
//! cap against the closed form of the cross-section, and what happens
//! without a section, with a plane that is none and on a device that
//! draws none.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;
mod devices;

use common::{
    Parity, compare_msaa_with, compare_with, evidence, offscreens, orthographic, params,
    perspective, write_evidence,
};
use craft3d_gpu::{
    Camera, DrawOptions, GpuError, Highlight, Mesh, Onscreen, OverlayParams, Projection,
    RenderParams, SECTION_FACE, Section, Segment,
};
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_tess::{TessParams, tessellate_body, tessellate_patches};
use craft3d_testkit::{solids, teapot::teapot, trimmed};
use devices::devices;
use std::f64::consts::PI;

const CAP: [u8; 3] = [214, 132, 74];

struct Scene {
    name: String,
    mesh: Mesh,
    camera: Camera,
    params: RenderParams,
    evidence: bool,
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

fn at(x: f64, y: f64, z: f64) -> Frame {
    Frame::new(Vec3::new(x, y, z), Vec3::Z, Vec3::X).unwrap()
}

/// A block with a closed cavity in it, made by a boolean.
fn hollow_block() -> Mesh {
    let outer = primitives::block(&at(-3.0, -2.0, -1.5), 6.0, 4.0, 3.0).unwrap();
    let inner = primitives::block(&at(-2.0, -1.0, -1.0), 4.0, 2.0, 2.0).unwrap();
    let hollow = boolean(&outer, &inner, BooleanOp::Subtract, &Tolerance::default()).unwrap();
    mesh_of!(&hollow.body, 0.01)
}

/// A plate with a hole drilled through it by a boolean.
fn drilled_plate() -> Mesh {
    let plate = primitives::block(&at(-3.0, -2.0, -0.5), 6.0, 4.0, 1.0).unwrap();
    let drill = primitives::cylinder(&at(0.6, 0.3, -2.0), 1.1, 4.0).unwrap();
    let drilled = boolean(&plate, &drill, BooleanOp::Subtract, &Tolerance::default()).unwrap();
    mesh_of!(&drilled.body, 0.004)
}

fn teapot_mesh() -> Mesh {
    tessellate_patches(&teapot().unwrap(), &TessParams::default()).unwrap()
}

/// The planes a model is cut with: across each axis at two places of its
/// extent, and an oblique one through its middle. Each as a point and a
/// normal, with a name.
fn planes(mesh: &Mesh) -> Vec<(String, Vec3, Vec3)> {
    let (lo, hi) = mesh.bbox().corners().unwrap();
    let center = (lo + hi) * 0.5;
    let mut out = Vec::new();
    for (name, axis) in [("x", Vec3::X), ("y", Vec3::Y), ("z", Vec3::Z)] {
        for (k, fraction) in [0.31, 0.67].into_iter().enumerate() {
            let point = lo + (hi - lo) * fraction;
            // Cut away the upper part at the first place, the lower part
            // at the second.
            let normal = if k == 0 { axis } else { -axis };
            out.push((format!("{name}{k}"), point, normal));
        }
    }
    out.push((
        "oblique".to_string(),
        center + (hi - lo) * 0.07,
        Vec3::new(0.5, -0.7, 0.6),
    ));
    out
}

fn section(origin: Vec3, normal: Vec3) -> Section {
    Section {
        origin,
        normal,
        cap: true,
        color: CAP,
    }
}

/// A camera on the side of the plane that is cut away, so that the cut is
/// in the picture.
fn looking_in(mesh: &Mesh, normal: Vec3, variant: usize, p: &RenderParams) -> Camera {
    let n = normal.normalized().unwrap();
    let tilt = [
        Vec3::new(0.5, -0.7, 0.45),
        Vec3::new(-0.6, 0.35, 0.7),
        Vec3::new(0.3, 0.8, -0.4),
    ][variant % 3];
    // Towards the eye: mostly along the normal, tilted so that the walls
    // behind the cut show too.
    let toward = n + (tilt - n * tilt.dot(n)) * 0.9;
    if variant % 2 == 1 {
        orthographic(mesh, toward, p)
    } else {
        perspective(mesh, toward, [30.0, 55.0][(variant / 2) % 2], p)
    }
}

/// Every scene of `mesh`: each plane, opaque and see-through, every other
/// one with a highlight.
fn scenes_of(name: &str, mesh: &Mesh, cap: bool, start: usize, out: &mut Vec<Scene>) {
    let faces = mesh.face_ids.iter().copied().max().map_or(0, |m| m + 1);
    for (k, (plane, origin, normal)) in planes(mesh).into_iter().enumerate() {
        for (see_through, opacity) in [(false, 1.0), (true, [0.5, 0.1, 0.9][k % 3])] {
            let variant = start + k + usize::from(see_through);
            let (width, height) = [(280, 210), (233, 177), (190, 250)][variant % 3];
            let mut p = RenderParams {
                opacity,
                section: Some(Section {
                    cap,
                    ..section(origin, normal)
                }),
                ..params(width, height, [1, 2, 3][(variant / 2) % 3])
            };
            if variant.is_multiple_of(2) {
                let picked = (0..faces).filter(|f| f % 3 == (variant as u32) % 3);
                p.highlight = Highlight::new([64, 196, 212], picked).0;
            }
            out.push(Scene {
                name: format!(
                    "section-{name}-{plane}-{}{}",
                    if see_through { "seethrough" } else { "opaque" },
                    if p.highlight.is_empty() {
                        ""
                    } else {
                        "-highlight"
                    }
                ),
                camera: looking_in(mesh, normal, variant, &p),
                mesh: mesh.clone(),
                params: p,
                evidence: k == 6 || (k == 4 && !see_through),
            });
        }
    }
}

/// What a failing comparison leaves behind, before it fails: the CPU
/// picture, the GPU picture and their differences
/// (`failed-<scene>-<backend>-<kind>.png` in the evidence directory), and a
/// text with every pixel outside the band whose colour is two or more
/// levels off, the box around them, and the scene rendered three times
/// more on the same renderer: a frame that comes out right again is a race
/// (in our passes or in the driver), one that is wrong every time is a
/// defect of the algorithm. The text also goes to stderr, so a gate log
/// has it. A rare failure on Mesa's OpenGL in the full gate (16 pixels, 3+
/// levels off, ids right) could not be reproduced outside it; this is what
/// will tell its cause the next time.
fn record_failure(
    scene: &Scene,
    adapter: &craft3d_gpu::AdapterSummary,
    cpu: &craft3d_gpu::Frame,
    gpu: &craft3d_gpu::Frame,
    band: &[bool],
    offscreen: &mut craft3d_gpu::Offscreen,
) {
    let base = format!(
        "failed-{}-{}-{}",
        scene.name,
        adapter.backend.to_lowercase(),
        adapter.device_type.to_lowercase()
    );
    let png = write_evidence(&format!("{base}.png"), cpu, gpu, band);
    let w = cpu.width as usize;
    let mut text = format!(
        "{} on {adapter}: {}x{}, ssaa {}, pictures in {}\n",
        scene.name,
        cpu.width,
        cpu.height,
        scene.params.ssaa,
        png.display()
    );
    let off = |frame: &craft3d_gpu::Frame, i: usize| {
        (0..3)
            .map(|c| cpu.rgb[i * 3 + c].abs_diff(frame.rgb[i * 3 + c]))
            .max()
            .unwrap_or(0)
    };
    let mut bounds: Option<(usize, usize, usize, usize)> = None;
    for i in (0..cpu.ids.len()).filter(|&i| !band[i] && off(gpu, i) >= 2) {
        let (x, y) = (i % w, i / w);
        bounds = Some(bounds.map_or((x, y, x, y), |(x0, y0, x1, y1)| {
            (x0.min(x), y0.min(y), x1.max(x), y1.max(y))
        }));
        text.push_str(&format!(
            "  ({x}, {y}): cpu {:?} id {}, gpu {:?} id {}\n",
            &cpu.rgb[i * 3..i * 3 + 3],
            cpu.ids[i],
            &gpu.rgb[i * 3..i * 3 + 3],
            gpu.ids[i]
        ));
    }
    if let Some((x0, y0, x1, y1)) = bounds {
        text.push_str(&format!("  within x {x0}..={x1}, y {y0}..={y1}\n"));
    }
    for again in 1..=3 {
        match offscreen.render(&scene.mesh, &scene.camera, &scene.params) {
            Ok(next) => {
                let still = (0..cpu.ids.len())
                    .filter(|&i| !band[i] && off(&next, i) >= 2)
                    .count();
                text.push_str(&format!(
                    "  rendered again ({again}): {} the failing frame, {still} pixels off\n",
                    if next == *gpu { "the same as" } else { "not" }
                ));
            }
            Err(e) => text.push_str(&format!("  rendered again ({again}): {e}\n")),
        }
    }
    let _ = std::fs::write(evidence(&format!("{base}.txt")), &text);
    eprintln!("{text}");
}

/// On OpenGL only, a frame that failed is rendered once more, and when that
/// frame passes the same strict comparison its parity stands in for the
/// failed one's. Three gates in a row (2026-10-14) failed on 16 pixels at the
/// right edge of a picture whose colours were random bytes over the
/// background while the face ids were right, and the same scene rendered
/// again came out right every time: Mesa's OpenGL (Intel, under the full
/// gate's load) left part of the colour copy unwritten. Vulkan and lavapipe
/// stay strict, and so does OpenGL for a frame that is wrong again: a defect
/// of the algorithm still fails. The developer agreed to this on 2026-10-14.
fn gl_race_rendered_again(
    scene: &Scene,
    adapter: &craft3d_gpu::AdapterSummary,
    cpu: &craft3d_gpu::Frame,
    offscreen: &mut craft3d_gpu::Offscreen,
) -> Option<Parity> {
    if !adapter.backend.eq_ignore_ascii_case("gl") {
        return None;
    }
    let again = offscreen
        .render(&scene.mesh, &scene.camera, &scene.params)
        .ok()?;
    let (parity, _) = compare_with(&scene.mesh, &scene.camera, &scene.params, cpu, &again);
    (parity.id_outside + parity.depth_outside + parity.color_outside == 0).then_some(parity)
}

/// Renders every scene with both renderers on every adapter and checks
/// parity. Returns the number of comparisons.
fn check(group: &str, scenes: &[Scene]) -> usize {
    let mut compared = 0;
    let mut log = String::new();
    let mut worst = (0_usize, 0_usize, 0.0_f64, 0_usize, 0_usize);
    for mut offscreen in offscreens() {
        let adapter = offscreen.adapter().clone();
        for scene in scenes {
            let cpu = craft3d_render::render(&scene.mesh, &scene.camera, &scene.params).unwrap();
            let gpu = offscreen
                .render(&scene.mesh, &scene.camera, &scene.params)
                .unwrap_or_else(|e| panic!("{} on {adapter}: {e}", scene.name));
            let (mut parity, band) =
                compare_with(&scene.mesh, &scene.camera, &scene.params, &cpu, &gpu);
            let label = format!("{} on {adapter}", scene.name);
            if scene.evidence {
                let file = format!(
                    "{}-{}-{}.png",
                    scene.name,
                    adapter.backend.to_lowercase(),
                    adapter.device_type.to_lowercase()
                );
                write_evidence(&file, &cpu, &gpu, &band);
            }
            // The cap is not shaded: where both frames have it outside the
            // band, the colour of an opaque picture is the cap's own.
            if scene.params.opacity() >= 1.0 {
                for (i, _) in band.iter().enumerate().filter(|&(_, &b)| !b) {
                    if cpu.ids[i] == SECTION_FACE && gpu.ids[i] == SECTION_FACE {
                        assert_eq!(
                            gpu.rgb[i * 3..i * 3 + 3],
                            cpu.rgb[i * 3..i * 3 + 3],
                            "{label}: the cap's colour at pixel {i}"
                        );
                    }
                }
            }
            let caps = cpu.ids.iter().filter(|&&id| id == SECTION_FACE).count();
            log.push_str(&format!("{label}: cap pixels {caps}, {parity:?}\n"));
            if parity.id_outside + parity.depth_outside + parity.color_outside > 0 {
                record_failure(scene, &adapter, &cpu, &gpu, &band, &mut offscreen);
                if let Some(clean) = gl_race_rendered_again(scene, &adapter, &cpu, &mut offscreen) {
                    eprintln!(
                        "{label}: the frame above is counted as rendered again, which matches \
                         ({clean:?}): Mesa's OpenGL left bytes of the colour copy unwritten \
                         (docs/testing.md, *A race in Mesa's OpenGL*)"
                    );
                    parity = clean;
                }
            }
            parity.check(&label);
            let differing: usize = parity.color_histogram.iter().skip(1).sum();
            worst.0 = worst.0.max(differing);
            worst.1 = worst
                .1
                .max(parity.color_histogram[2] + parity.color_histogram[3]);
            worst.2 = worst.2.max(parity.depth_ratio);
            worst.3 = worst.3.max(parity.id_band);
            worst.4 = worst.4.max(parity.color_band);
            compared += 1;
        }
    }
    let summary = format!(
        "{group}: {compared} comparisons; outside the band at most {} pixels of a scene differ \
         by a level and {} by more, the largest distance error is {:.0} % of its tolerance; \
         inside the band at most {} ids and {} colours of a scene differ",
        worst.0,
        worst.1,
        worst.2 * 100.0,
        worst.3,
        worst.4
    );
    println!("{summary}");
    log.push_str(&summary);
    std::fs::write(evidence(&format!("parity-section-{group}.txt")), log).unwrap();
    compared
}

#[test]
fn cut_open_reference_solids_match_the_cpu_renderer() {
    let frame = Frame::new(
        Vec3::new(3.0, -2.0, 5.0),
        Vec3::new(0.3, -0.2, 1.0),
        Vec3::X,
    )
    .unwrap();
    let mut all = solids::all(&frame, 1.0).unwrap();
    all.extend(trimmed::all(&frame, 1.0).unwrap());
    all.push(trimmed::pipe_elbow(&frame, 3.0, 1.0, 1.9).unwrap());
    all.push(trimmed::sphere_octant(&frame, 2.0).unwrap());
    let mut scenes = Vec::new();
    for (index, solid) in all.iter().enumerate() {
        let mesh = mesh_of!(&solid.body, 0.01);
        scenes_of(
            &format!("{}-{index}", solid.name.replace(' ', "_")),
            &mesh,
            true,
            index,
            &mut scenes,
        );
    }
    let compared = check("solids", &scenes);
    assert_eq!(compared, all.len() * 14 * offscreens().len());
}

/// A thousandth, a million times, and ten million units from the origin:
/// for every reference solid one plane, opaque or see-through.
fn scales_scenes() -> Vec<Scene> {
    let near = Vec3::new(3.0, -2.0, 5.0);
    let mut scenes = Vec::new();
    for (round, (scale, origin)) in [
        (1e-3, near * 1e-3),
        (1e6, near * 1e6),
        (1.0, Vec3::new(1e7, -2e7, 3e7)),
    ]
    .into_iter()
    .enumerate()
    {
        let frame = Frame::new(origin, Vec3::new(0.3, -0.2, 1.0), Vec3::X).unwrap();
        let mut all = solids::all(&frame, scale).unwrap();
        all.extend(trimmed::all(&frame, scale).unwrap());
        for (index, solid) in all.iter().enumerate() {
            let mesh = mesh_of!(&solid.body, 0.01 * scale);
            let mut of_solid = Vec::new();
            scenes_of(
                &format!("{}-{index}-round{round}", solid.name.replace(' ', "_")),
                &mesh,
                true,
                index + round,
                &mut of_solid,
            );
            // One plane, opaque, and the next one, see-through.
            let pick = (index + round) % 7 * 2;
            scenes.extend(of_solid.into_iter().skip(pick).take(1));
        }
    }
    scenes
}

#[test]
fn cut_open_solids_at_other_scales_match_the_cpu_renderer() {
    let scenes = scales_scenes();
    assert!(scenes.len() > 30);
    check("scales", &scenes);
}

fn env_count(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// A stress test for races, not run by default: the scenes of the test
/// above, drawn over and over on every adapter by `CRAFT3D_REPEAT_THREADS`
/// threads at once (each with its own devices) for `CRAFT3D_REPEAT_ROUNDS`
/// rounds. Every frame must be the first frame of its scene, bit for bit;
/// a frame that is not is written to the evidence directory
/// (`repeat-<scene>-…png`: the first frame, this one, their differences).
/// The image size changes from one scene to the next, so the targets and
/// the readback buffers are made anew for every frame.
#[test]
#[ignore = "a stress test for races; run it on purpose"]
fn frames_repeat_bit_for_bit_under_load() {
    let threads = env_count("CRAFT3D_REPEAT_THREADS", 4);
    let rounds = env_count("CRAFT3D_REPEAT_ROUNDS", 10);
    let scenes = scales_scenes();
    let repeat = |thread: usize| {
        let mut bad = 0;
        for mut offscreen in offscreens() {
            let adapter = offscreen.adapter().clone();
            let render = |offscreen: &mut craft3d_gpu::Offscreen, s: &Scene| {
                offscreen.render(&s.mesh, &s.camera, &s.params).unwrap()
            };
            let firsts: Vec<_> = scenes.iter().map(|s| render(&mut offscreen, s)).collect();
            for round in 0..rounds {
                for (scene, first) in scenes.iter().zip(&firsts) {
                    let frame = render(&mut offscreen, scene);
                    if frame == *first {
                        continue;
                    }
                    bad += 1;
                    let w = frame.width as usize;
                    let off: Vec<String> = (0..frame.ids.len())
                        .filter(|&i| {
                            frame.rgb[i * 3..i * 3 + 3] != first.rgb[i * 3..i * 3 + 3]
                                || frame.ids[i] != first.ids[i]
                        })
                        .map(|i| format!("({}, {})", i % w, i / w))
                        .collect();
                    let name = format!(
                        "repeat-{}-{}-thread{thread}-round{round}",
                        scene.name,
                        adapter.backend.to_lowercase()
                    );
                    let none = vec![false; frame.ids.len()];
                    write_evidence(&format!("{name}.png"), first, &frame, &none);
                    eprintln!(
                        "{} on {adapter}, thread {thread}, round {round}: {} pixels differ \
                         from the first frame: {}",
                        scene.name,
                        off.len(),
                        off.join(" ")
                    );
                }
            }
            println!(
                "thread {thread} on {adapter}: {} frames",
                rounds * scenes.len()
            );
        }
        bad
    };
    let bad: usize = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|thread| scope.spawn(move || repeat(thread)))
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).sum()
    });
    assert_eq!(bad, 0, "frames that are not the first frame of their scene");
}

#[test]
fn cut_open_models_match_the_cpu_renderer() {
    let mut scenes = Vec::new();
    scenes_of("hollow", &hollow_block(), true, 0, &mut scenes);
    scenes_of("drilled", &drilled_plate(), true, 1, &mut scenes);
    // The teapot is a mesh of open sheets: cut without a cap, as a viewer
    // cuts it, and with one, where the count means nothing but is still
    // the same count in both renderers.
    let teapot = teapot_mesh();
    scenes_of("teapot", &teapot, false, 2, &mut scenes);
    scenes_of("teapot_capped", &teapot, true, 3, &mut scenes);
    let compared = check("models", &scenes);
    assert_eq!(compared, 4 * 14 * offscreens().len());
}

/// An orthographic camera that looks at the plane head-on from the half
/// that is cut away, with `side` model units across the square picture.
fn head_on(mesh: &Mesh, section: &Section, side: f64) -> Camera {
    let bbox = mesh.bbox();
    let normal = section.normal.normalized().unwrap();
    let center = bbox.center().unwrap();
    let up = if normal.cross(Vec3::Z).length() > 1e-6 {
        Vec3::Z
    } else {
        Vec3::Y
    };
    Camera {
        eye: center + normal * (bbox.diagonal() * 3.0),
        target: center,
        up,
        projection: Projection::Orthographic {
            half_height: side * 0.5,
        },
    }
}

#[test]
fn the_caps_area_is_the_cross_sections() {
    let tolerance = 1e-4;
    let block = mesh_of!(
        &primitives::block(&at(-2.0, -1.5, -1.0), 4.0, 3.0, 2.0).unwrap(),
        0.01
    );
    let (r, h) = (1.5, 4.0);
    let cylinder = mesh_of!(
        &primitives::cylinder(&at(0.0, 0.0, -2.0), r, h).unwrap(),
        tolerance
    );
    let ball_radius = 2.0_f64;
    let ball = mesh_of!(
        &primitives::sphere(&at(0.3, -0.2, 0.1), ball_radius).unwrap(),
        tolerance
    );
    let d = 0.9_f64;
    let small = (ball_radius * ball_radius - d * d).sqrt();
    let oblique = Vec3::new(1.0, 2.0, -0.5).normalized().unwrap();
    let hollow = hollow_block();
    // The model, the plane, the area and the outline's length of the
    // cross-section, and the chord error per unit of outline.
    let cases: Vec<(&str, &Mesh, Section, f64, f64, f64)> = vec![
        (
            "block-x",
            &block,
            section(Vec3::X * 0.61, Vec3::X),
            6.0,
            10.0,
            0.0,
        ),
        (
            "block-y",
            &block,
            section(Vec3::Y * -0.7, -Vec3::Y),
            8.0,
            12.0,
            0.0,
        ),
        (
            "block-z",
            &block,
            section(Vec3::Z * 0.013, Vec3::Z),
            12.0,
            14.0,
            0.0,
        ),
        (
            "block-oblique",
            &block,
            section(Vec3::ZERO, Vec3::new(1.0, 0.0, 1.0)),
            3.0 * 8.0_f64.sqrt(),
            2.0 * (3.0 + 8.0_f64.sqrt()),
            0.0,
        ),
        (
            "cylinder-across",
            &cylinder,
            section(Vec3::Z * 0.2, Vec3::Z),
            PI * r * r,
            2.0 * PI * r,
            tolerance,
        ),
        (
            "ball",
            &ball,
            section(Vec3::new(0.3, -0.2, 0.1) + oblique * d, oblique),
            PI * small * small,
            2.0 * PI * small,
            tolerance * ball_radius / small,
        ),
        (
            "hollow-frame",
            &hollow,
            section(Vec3::Z * 0.25, Vec3::Z),
            24.0 - 8.0,
            20.0 + 12.0,
            0.0,
        ),
        (
            "hollow-wall",
            &hollow,
            section(Vec3::Z * 1.25, Vec3::Z),
            24.0,
            20.0,
            0.0,
        ),
    ];
    let pixels = 400_u32;
    let mut log = String::new();
    for mut offscreen in offscreens() {
        let adapter = offscreen.adapter().clone();
        for (name, mesh, cut, area, outline, chord) in &cases {
            let side = mesh.bbox().diagonal() * 1.1;
            let camera = head_on(mesh, cut, side);
            let p = RenderParams {
                section: Some(*cut),
                ..params(pixels, pixels, 1)
            };
            let pixel = side / f64::from(pixels);
            // A pixel on the outline is in or out by where its centre
            // lies: off by at most the pixels the outline passes through.
            let allowed = outline * (pixel * 2.0_f64.sqrt() + chord);
            let cpu = craft3d_render::render(mesh, &camera, &p).unwrap();
            let gpu = offscreen.render(mesh, &camera, &p).unwrap();
            let count = |frame: &craft3d_gpu::Frame| {
                frame.ids.iter().filter(|&&id| id == SECTION_FACE).count() as f64 * pixel * pixel
            };
            let (on_cpu, on_gpu) = (count(&cpu), count(&gpu));
            let line = format!(
                "{name} on {adapter}: closed form {area:.5}, cpu {on_cpu:.5} ({:+.3} %), gpu {on_gpu:.5} ({:+.3} %), allowed {:.3} %",
                (on_cpu - area) / area * 100.0,
                (on_gpu - area) / area * 100.0,
                allowed / area * 100.0
            );
            println!("{line}");
            log.push_str(&line);
            log.push('\n');
            assert!((on_gpu - area).abs() <= allowed, "{line}");
            assert!((on_cpu - area).abs() <= allowed, "{line}");
            // Every cap pixel has the cap's colour, and nothing else has.
            for (i, px) in gpu.rgb.chunks(3).enumerate() {
                assert_eq!(gpu.ids[i] == SECTION_FACE, px == CAP, "{name}: pixel {i}");
            }
        }
    }
    std::fs::write(evidence("section-cap-areas.txt"), log).unwrap();
}

#[test]
fn without_a_section_and_with_a_plane_that_is_none_the_frame_is_the_whole_one() {
    let mesh = hollow_block();
    for mut offscreen in offscreens() {
        let adapter = offscreen.adapter().clone();
        for opacity in [1.0, 0.5] {
            let plain = RenderParams {
                opacity,
                ..params(200, 150, 2)
            };
            let camera = perspective(&mesh, Vec3::new(1.0, -1.3, 0.9), 30.0, &plain);
            let whole = offscreen.render(&mesh, &camera, &plain).unwrap();
            // A cut-open frame in between must leave nothing behind.
            let cut = RenderParams {
                section: Some(section(Vec3::Z * 0.25, Vec3::Z)),
                ..plain
            };
            let opened = offscreen.render(&mesh, &camera, &cut).unwrap();
            assert_ne!(opened, whole, "{adapter}: the section changed nothing");
            assert_eq!(
                offscreen.render(&mesh, &camera, &plain).unwrap(),
                whole,
                "{adapter}: the frame after a cut-open one"
            );
            for (origin, normal) in [
                (Vec3::ZERO, Vec3::ZERO),
                (Vec3::ZERO, Vec3::new(f64::NAN, 0.0, 0.0)),
                (Vec3::new(f64::INFINITY, 0.0, 0.0), Vec3::Z),
                (Vec3::new(0.0, f64::NAN, 0.0), Vec3::Z),
            ] {
                let none = RenderParams {
                    section: Some(section(origin, normal)),
                    ..plain
                };
                assert_eq!(
                    offscreen.render(&mesh, &camera, &none).unwrap(),
                    whole,
                    "{adapter}: a plane that is none"
                );
            }
            // A plane beyond the model keeps all of it, or none of it; the
            // frame with all of it is the whole one within the tolerance
            // of two frames of the same picture (other pipelines drew it).
            let beyond = RenderParams {
                section: Some(section(Vec3::Z * 50.0, Vec3::Z)),
                ..plain
            };
            let kept = offscreen.render(&mesh, &camera, &beyond).unwrap();
            assert_eq!(kept.ids, whole.ids, "{adapter}");
            let gone = RenderParams {
                section: Some(section(Vec3::Z * 50.0, -Vec3::Z)),
                ..plain
            };
            let nothing = offscreen.render(&mesh, &camera, &gone).unwrap();
            assert_eq!(nothing.coverage(), 0.0, "{adapter}");
            // Numbers that are numbers but fit nowhere: an answer or an
            // error, never a panic.
            for (origin, normal) in [
                (Vec3::new(1e300, 0.0, 0.0), Vec3::X),
                (Vec3::ZERO, Vec3::new(1e-300, 0.0, 0.0)),
                (Vec3::new(-1e300, 1e300, 0.0), Vec3::new(1.0, 1.0, 1e-200)),
                (Vec3::new(1e30, 1e30, 1e30), Vec3::new(1.0, 2.0, 3.0)),
            ] {
                let hostile = RenderParams {
                    section: Some(section(origin, normal)),
                    ..plain
                };
                match offscreen.render(&mesh, &camera, &hostile) {
                    Ok(frame) => assert_eq!(frame.width, 200),
                    Err(GpuError::OutOfRange(_)) => {}
                    Err(other) => panic!("{adapter}: {other}"),
                }
            }
        }
        // An empty mesh, cut: the background.
        let p = RenderParams {
            section: Some(section(Vec3::ZERO, Vec3::Z)),
            ..params(64, 48, 1)
        };
        let camera = perspective(&mesh, Vec3::new(1.0, -1.3, 0.9), 30.0, &p);
        let empty = offscreen.render(&Mesh::default(), &camera, &p).unwrap();
        assert_eq!(empty.coverage(), 0.0);
    }
}

#[test]
fn a_renderer_told_it_cannot_refuses_a_section_and_draws_the_whole_model() {
    let mesh = hollow_block();
    for mut offscreen in offscreens() {
        let plain = params(120, 90, 1);
        let camera = perspective(&mesh, Vec3::new(1.0, -1.3, 0.9), 30.0, &plain);
        let whole = offscreen.render(&mesh, &camera, &plain).unwrap();
        offscreen.renderer().without_section();
        let cut = RenderParams {
            section: Some(section(Vec3::Z * 0.25, Vec3::Z)),
            ..plain
        };
        match offscreen.render(&mesh, &camera, &cut) {
            Err(GpuError::Section(why)) => assert!(why.contains("turned off")),
            other => panic!("expected a refusal, got {:?}", other.map(|f| f.width)),
        }
        assert_eq!(offscreen.render(&mesh, &camera, &plain).unwrap(), whole);
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

fn name_of(adapter: &wgpu::Adapter) -> String {
    let info = adapter.get_info();
    format!("{} ({:?})", info.name, info.backend)
}

/// The viewport's renderer (multisampled where the adapter has it) draws
/// a cut-open picture as the CPU renderer does, away from the edges: the
/// hollow block, the drilled plate and the teapot, opaque and see-through.
#[test]
fn the_viewports_cut_open_picture_matches_the_cpu_renderer_away_from_the_edges() {
    let models = [
        ("hollow", hollow_block(), true),
        ("drilled", drilled_plate(), true),
        ("teapot", teapot_mesh(), false),
    ];
    let mut scenes = Vec::new();
    for (index, (name, mesh, cap)) in models.iter().enumerate() {
        let mut all = Vec::new();
        scenes_of(name, mesh, *cap, index, &mut all);
        // Across z, across x and the oblique plane, opaque and seen
        // through; the viewport has one sample of supersampling.
        for scene in all {
            if ["-z0-", "-x1-", "-oblique-"]
                .iter()
                .any(|plane| scene.name.contains(plane))
            {
                let params = RenderParams {
                    width: 420,
                    height: 300,
                    ssaa: 1,
                    ..scene.params
                };
                let camera = looking_in(
                    &scene.mesh,
                    scene.params.section.unwrap().normal,
                    scenes.len(),
                    &params,
                );
                scenes.push(Scene {
                    params,
                    camera,
                    ..scene
                });
            }
        }
    }
    assert_eq!(scenes.len(), 18);
    let mut log = String::new();
    let mut measured = Vec::new();
    for (adapter, device, queue) in devices() {
        let adapter_name = name_of(&adapter);
        let mut onscreen = Onscreen::new(&device, &queue, &adapter).unwrap();
        let lines = onscreen.upload_lines(&[]).unwrap();
        let options = DrawOptions {
            bodies: true,
            capture: true,
        };
        for scene in &scenes {
            let (mesh, camera, params) = (&scene.mesh, &scene.camera, &scene.params);
            let cpu = craft3d_render::render(mesh, camera, params).unwrap();
            let uploaded = onscreen.upload(mesh).unwrap();
            onscreen
                .draw(&uploaded, &lines, camera, params, &no_overlay(), options)
                .unwrap();
            let gpu = onscreen.read_frame().unwrap();
            let (parity, band) = compare_msaa_with(mesh, camera, params, &cpu, &gpu);
            let label = format!(
                "onscreen {} on {adapter_name}, {} samples",
                scene.name,
                onscreen.samples()
            );
            log.push_str(&format!("{label}: {parity:?}\n"));
            let backend = format!("{:?}", adapter.get_info().backend).to_lowercase();
            let kind = format!("{:?}", adapter.get_info().device_type).to_lowercase();
            if scene.name.contains("oblique") {
                let file = format!("onscreen-{}-{backend}-{kind}.png", scene.name);
                write_evidence(&file, &cpu, &gpu, &band);
            }
            measured.push((label.clone(), parity));
            // The whole picture afterwards is what a renderer that never
            // drew a cut-open one gives: the count leaves nothing behind.
            let whole = RenderParams {
                section: None,
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
                    &whole,
                    &no_overlay(),
                    options,
                )
                .unwrap();
            onscreen
                .draw(&uploaded, &lines, camera, &whole, &no_overlay(), options)
                .unwrap();
            assert_eq!(
                onscreen.read_frame().unwrap(),
                fresh.read_frame().unwrap(),
                "{label}"
            );
        }
    }
    std::fs::write(evidence("onscreen-section.txt"), log).unwrap();
    let mut worst = (0_usize, 0_usize, 0_usize);
    for (label, parity) in &measured {
        parity.check(label);
        worst.0 = worst
            .0
            .max(parity.color_plain_histogram.iter().skip(1).sum());
        worst.1 = worst.1.max(parity.color_seam_differ);
        worst.2 = worst.2.max(parity.id_band);
    }
    println!(
        "onscreen: {} comparisons; away from every edge at most {} pixels of a scene differ by a \
         level, on seams at most {} by more than a level, in the band at most {} ids",
        measured.len(),
        worst.0,
        worst.1,
        worst.2
    );
}

/// The colour of the pixel under (`x`, `y`) of an RGB picture.
fn pixel(rgb: &[u8], width: u32, x: f64, y: f64) -> [u8; 3] {
    let i = (y.floor() as usize * width as usize + x.floor() as usize) * 3;
    [rgb[i], rgb[i + 1], rgb[i + 2]]
}

/// The cap is in the depth buffer the lines are tested against: a line
/// behind the cap is hidden, one in the half that is cut away is in front
/// of it, and a line inside the body shows through no cap.
#[test]
fn the_cap_hides_the_lines_behind_it() {
    // A block from (-2, -1.5, -1) to (2, 1.5, 1), seen from +x, cut at
    // x = 0.5 with what is beyond cut away: the cap is the 3 × 2
    // rectangle at x = 0.5.
    let mesh = mesh_of!(
        &primitives::block(&at(-2.0, -1.5, -1.0), 4.0, 3.0, 2.0).unwrap(),
        0.01
    );
    let p = RenderParams {
        section: Some(section(Vec3::X * 0.5, Vec3::X)),
        ..params(400, 300, 1)
    };
    let camera = Camera {
        eye: Vec3::new(20.0, 0.0, 0.0),
        target: Vec3::ZERO,
        up: Vec3::Z,
        projection: Projection::Orthographic { half_height: 2.0 },
    };
    let yellow = [255, 196, 64];
    let line = |x: f64, z: f64| Segment {
        from: Vec3::new(x, -2.4, z),
        to: Vec3::new(x, 2.4, z),
        color: yellow,
        width: 3.0,
        tested: true,
        dimmed: false,
        axis: false,
        edge: None,
    };
    // Inside the body behind the cap, and in the half that is cut away.
    let segments = [line(-0.5, 0.5), line(1.2, -0.5)];
    for (adapter, device, queue) in devices() {
        let adapter_name = name_of(&adapter);
        let mut onscreen = Onscreen::new(&device, &queue, &adapter).unwrap();
        let uploaded = onscreen.upload(&mesh).unwrap();
        let lines = onscreen.upload_lines(&segments).unwrap();
        onscreen
            .draw(
                &uploaded,
                &lines,
                &camera,
                &p,
                &no_overlay(),
                DrawOptions::default(),
            )
            .unwrap();
        let rgb = onscreen.read_rgb().unwrap();
        // 75 pixels per unit: y runs to the left of the picture (the eye is
        // on +x and up is z), the cap covers x 87.5..312.5, y 75..225.
        let on_cap = |z: f64| pixel(&rgb, 400, 200.0, 150.0 - z * 75.0);
        assert_eq!(on_cap(0.0), CAP, "{adapter_name}: the cap");
        assert_eq!(
            on_cap(0.5),
            CAP,
            "{adapter_name}: a line behind the cap is hidden"
        );
        assert_eq!(
            on_cap(-0.5),
            yellow,
            "{adapter_name}: a line before the cap shows"
        );
        // Beside the cap both lines are over the background.
        let beside = |z: f64| pixel(&rgb, 400, 30.0, 150.0 - z * 75.0);
        assert_eq!(beside(0.5), yellow, "{adapter_name}");
        assert_eq!(beside(-0.5), yellow, "{adapter_name}");
        // Without the bodies' colour the depth buffer still holds the cap.
        let only_depth = DrawOptions {
            bodies: false,
            capture: false,
        };
        onscreen
            .draw(&uploaded, &lines, &camera, &p, &no_overlay(), only_depth)
            .unwrap();
        let rgb = onscreen.read_rgb().unwrap();
        let on_cap = |z: f64| pixel(&rgb, 400, 200.0, 150.0 - z * 75.0);
        assert_eq!(on_cap(0.0), p.background, "{adapter_name}");
        assert_eq!(
            on_cap(0.5),
            p.background,
            "{adapter_name}: hidden by the cap"
        );
        assert_eq!(on_cap(-0.5), yellow, "{adapter_name}");
        // A renderer told it cannot refuses, and draws the whole model.
        onscreen.without_section();
        let refused = onscreen.draw(
            &uploaded,
            &lines,
            &camera,
            &p,
            &no_overlay(),
            DrawOptions::default(),
        );
        assert!(
            matches!(refused, Err(GpuError::Section(_))),
            "{adapter_name}"
        );
        let whole = RenderParams { section: None, ..p };
        onscreen
            .draw(
                &uploaded,
                &lines,
                &camera,
                &whole,
                &no_overlay(),
                DrawOptions::default(),
            )
            .unwrap();
    }
}
