//! Layer 3 of the studio look on the GPU against the CPU renderer's
//! (`craft3d_render::studio::layers`): each layer alone (anisotropy along
//! an axis and around one, the clear coat, subsurface, transmission
//! through a solid and through a thin sheet with absorption) and all
//! together, on the readback renderer and on the viewport's; determinism;
//! and the cost of a frame with glass (docs/design/rendering.md,
//! *Layer 3: materials*).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::needless_range_loop
)]

mod common;
mod devices;

use common::{band, offscreens, orthographic, params, perspective};
use craft3d_gpu::{
    Camera, DrawOptions, EdgeLook, FaceColors, Frame, Mesh, Onscreen, OverlayParams, RenderParams,
    Studio,
};
use craft3d_math::Vec3;
use craft3d_render::Finish;
use craft3d_render::studio::layers::{Optics, TangentField, absorption_coefficients};
use craft3d_render::studio::polish::TRACE_DOUBT;
use craft3d_render::studio::{
    Look, Prepared, bundled::environment, render_studio, render_studio_doubt,
};
use craft3d_tess::{TessParams, tessellate_body};
use craft3d_testkit::{solids, trimmed};
use std::ops::Range;
use std::sync::Arc;

/// The tolerance of layer 3, GPU against CPU, as layer 2's
/// (`tests/polish.rs`): outside the band and the pixels in doubt, at most
/// this share of the pixels compared may differ by more than one level,
/// and none by more than [`MOST_LEVELS`].
///
/// The band of a picture with faces that let light through also holds
/// the band of what they show: where a thin sheet (or a solid without a
/// back face under the sample) shows the picture behind it straight, that
/// picture's band; where a solid refracts, the samples whose ray leaves
/// within one sample of a border between two faces of the picture behind
/// (`render_studio_doubt` with a margin of one sample, [`refraction_band`]).
///
/// Measured: see the dev log entry of 2026-10-30 (render-rl3-gpu) and
/// docs/design/rendering.md, *Layer 3: materials*.
const SHARE_OVER_ONE: f64 = 1.0 / 512.0;
const MOST_LEVELS: u8 = 8;
/// As layer 2's: a share of a pixel's light in doubt above which it is
/// left out, and the most pixels left out so.
const DOUBT_SHARE: f32 = 1.0 / 64.0;
const MOST_DOUBT: f64 = 1.0 / 32.0;

/// The parts of a scene by name.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Part {
    Plate,
    Ball,
    Post,
    Ring,
    Pane,
}

struct Scene {
    name: &'static str,
    mesh: Mesh,
    /// The colours and finishes, without layers.
    colors: FaceColors,
    /// The face ids of each part.
    parts: Vec<(Part, Range<u32>)>,
    camera: Camera,
    params: RenderParams,
    env: Arc<Prepared>,
}

impl Scene {
    fn faces(&self, part: Part) -> Range<u32> {
        self.parts
            .iter()
            .find(|(p, _)| *p == part)
            .map_or(0..0, |(_, r)| r.clone())
    }

    /// The colours with `layers` given to the faces of their parts.
    fn colors_with(&self, layers: &[(Part, Optics)]) -> FaceColors {
        let mut colors = self.colors.clone();
        for (part, optics) in layers {
            for face in self.faces(*part) {
                assert!(colors.set_optics(face, Some(*optics)));
            }
        }
        colors
    }
}

fn tessellated(solid: &solids::Reference) -> Mesh {
    tessellate_body(
        &solid.body,
        &TessParams {
            chordal_tolerance: 0.01,
            ..TessParams::default()
        },
    )
    .unwrap()
}

fn at(x: f64, y: f64, z: f64) -> craft3d_math::Frame {
    craft3d_math::Frame::new(Vec3::new(x, y, z), Vec3::Z, Vec3::X).unwrap()
}

/// The parts, each with its colour and finish, joined into one mesh.
fn combine(
    parts: Vec<(Part, solids::Reference, [u8; 3], Finish)>,
) -> (Mesh, FaceColors, Vec<(Part, Range<u32>)>) {
    let mut mesh = Mesh::default();
    let mut colors = FaceColors::new();
    let mut ranges = Vec::new();
    let mut faces = 0_u32;
    for (name, solid, color, finish) in parts {
        let part = tessellated(&solid);
        let base = mesh.positions.len() as u32;
        let first = faces;
        let count = part.face_ids.iter().copied().max().map_or(0, |m| m + 1);
        mesh.positions.extend(part.positions);
        mesh.normals.extend(part.normals);
        mesh.triangles
            .extend(part.triangles.iter().map(|t| t.map(|i| i + base)));
        mesh.face_ids
            .extend(part.face_ids.iter().map(|f| f + first));
        for face in first..first + count {
            colors.set_look(face, Some((color, finish)));
        }
        ranges.push((name, first..first + count));
        faces += count;
    }
    (mesh, colors, ranges)
}

fn scenes() -> Vec<Scene> {
    let metal = Finish::from_factors(Some(1.0), Some(0.35));
    let gloss = Finish::from_factors(Some(0.0), Some(0.2));
    let rough = Finish::from_factors(Some(0.0), Some(0.6));
    let plate = || solids::block(&at(0.0, 0.0, 0.0), 4.0, 4.0, 0.2).unwrap();
    let ball = || solids::sphere(&at(2.6, 2.6, 0.9), 0.7).unwrap();
    let post = || solids::cylinder(&at(0.9, 3.0, 0.2), 0.35, 1.4).unwrap();
    let ring = || solids::torus(&at(3.0, 1.0, 0.45), 0.6, 0.22).unwrap();
    // A standing pane between the camera and the ball and post.
    let pane = || solids::block(&at(0.6, 1.6, 0.2), 2.6, 0.35, 1.2).unwrap();
    let (mesh_a, colors_a, parts_a) = combine(vec![
        (Part::Plate, plate(), [180, 184, 190], metal),
        (Part::Ball, ball(), [200, 60, 50], rough),
        (Part::Post, post(), [190, 160, 110], metal),
        (Part::Ring, ring(), [90, 160, 90], gloss),
        (Part::Pane, pane(), [235, 240, 238], gloss),
    ]);
    let (mesh_b, colors_b, parts_b) = combine(vec![
        (Part::Plate, plate(), [60, 64, 70], gloss),
        (Part::Ball, ball(), [230, 190, 90], rough),
        (Part::Post, post(), [170, 170, 175], metal),
        (Part::Ring, ring(), [200, 140, 80], metal),
        (Part::Pane, pane(), [225, 235, 245], gloss),
    ]);
    let pa = params(320, 240, 1);
    let pb = params(300, 220, 1);
    vec![
        Scene {
            name: "bench, perspective",
            camera: perspective(&mesh_a, Vec3::new(0.3, -1.3, 0.55), 38.0, &pa),
            mesh: mesh_a,
            colors: colors_a,
            parts: parts_a,
            params: pa,
            env: environment("studio").unwrap(),
        },
        Scene {
            name: "bench outdoors, orthographic",
            camera: orthographic(&mesh_b, Vec3::new(-0.4, -1.0, 0.6), &pb),
            mesh: mesh_b,
            colors: colors_b,
            params: pb,
            parts: parts_b,
            env: environment("outdoor").unwrap(),
        },
    ]
}

fn glass(thin: bool) -> Optics {
    Optics {
        transmission: if thin { 0.9 } else { 1.0 },
        ior: 1.5,
        thin_walled: thin,
        absorption: absorption_coefficients([120, 200, 160], 0.35),
        ..Optics::default()
    }
}

/// Each layer alone, then all together: the layers per part.
fn terms() -> Vec<(&'static str, Vec<(Part, Optics)>)> {
    let axis = |a: f32, axis: [f32; 3]| Optics {
        anisotropy: a,
        tangent: TangentField::Axis(axis),
        ..Optics::default()
    };
    let around = |a: f32, origin: [f64; 3]| Optics {
        anisotropy: a,
        tangent: TangentField::Around {
            origin,
            axis: [0.0, 0.0, 1.0],
        },
        ..Optics::default()
    };
    let coat = |c: f32, r: f32| Optics {
        clearcoat: c,
        clearcoat_roughness: r,
        ..Optics::default()
    };
    let sss = |w: f32, color: Option<[u8; 3]>| Optics {
        subsurface: w,
        subsurface_color: color,
        ..Optics::default()
    };
    vec![
        (
            "anisotropy, axis",
            vec![
                (Part::Plate, axis(0.8, [1.0, 0.0, 0.0])),
                (Part::Ball, axis(-0.6, [0.0, 0.0, 1.0])),
            ],
        ),
        (
            "anisotropy, around",
            vec![
                (Part::Post, around(0.7, [0.9, 3.0, 0.0])),
                (Part::Ring, around(0.9, [3.0, 1.0, 0.45])),
            ],
        ),
        (
            "clearcoat",
            vec![(Part::Ball, coat(1.0, 0.05)), (Part::Plate, coat(0.6, 0.3))],
        ),
        (
            "subsurface",
            vec![
                (Part::Ball, sss(0.8, Some([230, 90, 60]))),
                (Part::Ring, sss(0.5, None)),
            ],
        ),
        ("glass, solid", vec![(Part::Pane, glass(false))]),
        ("glass, thin", vec![(Part::Pane, glass(true))]),
        (
            "all",
            vec![
                (
                    Part::Plate,
                    Optics {
                        clearcoat: 0.5,
                        clearcoat_roughness: 0.2,
                        ..axis(0.6, [1.0, 0.0, 0.0])
                    },
                ),
                (Part::Post, around(0.7, [0.9, 3.0, 0.0])),
                (Part::Ball, sss(0.7, Some([230, 90, 60]))),
                (Part::Ring, coat(1.0, 0.1)),
                (Part::Pane, glass(false)),
            ],
        ),
    ]
}

/// The look of a term: layer 1 with its shadows and occlusion for a layer
/// alone, the polished look (layer 2) with everything together.
fn look_of(term: &str) -> Look {
    if term == "all" {
        Look::polished()
    } else {
        Look::default()
    }
}

/// `mesh` without the faces of `colors` that let light through.
fn opaque(mesh: &Mesh, colors: &FaceColors) -> Mesh {
    let (triangles, face_ids) = mesh
        .triangles
        .iter()
        .zip(&mesh.face_ids)
        .filter(|(_, f)| !colors.is_transmissive(**f))
        .map(|(t, f)| (*t, *f))
        .unzip();
    Mesh {
        positions: mesh.positions.clone(),
        normals: mesh.normals.clone(),
        triangles,
        face_ids,
    }
}

/// The CPU's picture, its doubt at [`TRACE_DOUBT`], and the pixels the
/// comparison leaves out besides: the band of the picture and, with
/// transmission, the band of what it shows ([`refraction_band`]).
struct Cpu {
    frame: Frame,
    doubt: Vec<f32>,
    band: Vec<bool>,
}

/// The pixels of transmissive faces whose light from behind may come from
/// another face of the picture behind on the GPU: those whose refracted
/// ray leaves within one sample of a border between two faces there (the
/// CPU's doubt with a margin of one sample, for the look without the
/// traced effects, whose doubt would mark more), and those that show the
/// picture behind straight within its band.
fn refraction_band(
    scene: &Scene,
    colors: &FaceColors,
    params: &RenderParams,
    look: &Look,
    cpu: &Frame,
) -> Vec<bool> {
    let n = cpu.ids.len();
    if !colors.has_transmission() {
        return vec![false; n];
    }
    let w = (params.width * params.ssaa).max(params.height * params.ssaa);
    let one_sample = 1.0 / (f64::from(f32::EPSILON) * f64::from(w));
    let untraced = Look {
        ssr: false,
        ground_reflection: 0.0,
        dof: None,
        ..*look
    };
    let (_, refracted) = render_studio_doubt(
        &scene.mesh,
        colors,
        &scene.camera,
        params,
        &untraced,
        &scene.env,
        one_sample,
    )
    .unwrap();
    let behind_mesh = opaque(&scene.mesh, colors);
    let behind = render_studio(
        &behind_mesh,
        colors,
        &scene.camera,
        params,
        &Look::default(),
        &scene.env,
    )
    .unwrap();
    let straight = band(&behind_mesh, &scene.camera, &behind);
    (0..n)
        .map(|i| {
            refracted.get(i).is_some_and(|&d| d > 0.0)
                || (colors.is_transmissive(cpu.ids[i]) && straight[i])
        })
        .collect()
}

fn cpu_render(scene: &Scene, colors: &FaceColors, params: &RenderParams, look: &Look) -> Cpu {
    let (frame, doubt) = render_studio_doubt(
        &scene.mesh,
        colors,
        &scene.camera,
        params,
        look,
        &scene.env,
        TRACE_DOUBT,
    )
    .unwrap();
    let mut excluded = band(&scene.mesh, &scene.camera, &frame);
    for (e, r) in excluded
        .iter_mut()
        .zip(refraction_band(scene, colors, params, look, &frame))
    {
        *e |= r;
    }
    let doubt = if doubt.is_empty() {
        vec![0.0; frame.ids.len()]
    } else {
        doubt
    };
    Cpu {
        frame,
        doubt,
        band: excluded,
    }
}

#[derive(Default, Debug, Clone, Copy)]
struct Stats {
    compared: usize,
    over_one: usize,
    most: u8,
    id_outside: usize,
    doubt: usize,
    /// Pixels left out by the band of what transmissive faces show.
    refraction_band: usize,
    /// Pixels of transmissive faces compared, and the largest difference
    /// among them.
    through: usize,
    most_through: u8,
}

impl Stats {
    fn share(&self) -> f64 {
        self.over_one as f64 / self.compared.max(1) as f64
    }

    fn doubt_share(&self) -> f64 {
        self.doubt as f64 / (self.compared + self.doubt).max(1) as f64
    }
}

fn compare(scene: &Scene, colors: &FaceColors, cpu: &Cpu, gpu: &Frame) -> Stats {
    let plain_band = band(&scene.mesh, &scene.camera, &cpu.frame);
    let f = &cpu.frame;
    let mut stats = Stats::default();
    for i in 0..f.ids.len() {
        if cpu.band[i] {
            if !plain_band[i] {
                stats.refraction_band += 1;
            }
            continue;
        }
        if cpu.doubt[i] > DOUBT_SHARE {
            stats.doubt += 1;
            continue;
        }
        stats.compared += 1;
        if f.ids[i] != gpu.ids[i] {
            stats.id_outside += 1;
        }
        let d = (0..3)
            .map(|k| f.rgb[3 * i + k].abs_diff(gpu.rgb[3 * i + k]))
            .max()
            .unwrap();
        if d > 1 {
            stats.over_one += 1;
        }
        stats.most = stats.most.max(d);
        if colors.is_transmissive(f.ids[i]) {
            stats.through += 1;
            stats.most_through = stats.most_through.max(d);
        }
    }
    stats
}

fn check(failures: &mut Vec<String>, label: &str, stats: &Stats) {
    if stats.id_outside != 0
        || stats.share() > SHARE_OVER_ONE
        || stats.most > MOST_LEVELS
        || stats.doubt_share() > MOST_DOUBT
    {
        failures.push(format!(
            "{label}: {stats:?}, share {}, in doubt {}",
            stats.share(),
            stats.doubt_share()
        ));
    }
}

/// Prints every compared pixel over [`MOST_LEVELS`], for the dev log.
fn report(label: &str, cpu: &Cpu, gpu: &Frame) {
    let f = &cpu.frame;
    let w = f.width as usize;
    let mut printed = 0;
    for i in 0..f.ids.len() {
        if cpu.band[i] || cpu.doubt[i] > DOUBT_SHARE {
            continue;
        }
        let d = (0..3)
            .map(|k| f.rgb[3 * i + k].abs_diff(gpu.rgb[3 * i + k]))
            .max()
            .unwrap();
        if d > MOST_LEVELS && printed < 12 {
            printed += 1;
            eprintln!(
                "OFFENDER {label} | ({}, {}) id {} cpu {:?} gpu {:?} doubt {}",
                i % w,
                i / w,
                f.ids[i],
                &f.rgb[3 * i..3 * i + 3],
                &gpu.rgb[3 * i..3 * i + 3],
                cpu.doubt[i]
            );
        }
    }
}

fn adapter_tag(adapter: &str) -> String {
    adapter
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(24)
        .collect()
}

#[test]
fn layer_three_matches_the_cpu_renderer() {
    let scenes = scenes();
    let terms = terms();
    let mut failures = Vec::new();
    let cpus: Vec<Vec<(FaceColors, Cpu)>> = scenes
        .iter()
        .map(|scene| {
            terms
                .iter()
                .map(|(term, layers)| {
                    let colors = scene.colors_with(layers);
                    let cpu = cpu_render(scene, &colors, &scene.params, &look_of(term));
                    (colors, cpu)
                })
                .collect()
        })
        .collect();
    for (index, mut offscreen) in offscreens().into_iter().enumerate() {
        let adapter = offscreen.adapter().to_string();
        for (k, scene) in scenes.iter().enumerate() {
            for (j, (term, _)) in terms.iter().enumerate() {
                let (colors, cpu) = &cpus[k][j];
                let studio = Studio {
                    env: scene.env.clone(),
                    look: look_of(term),
                };
                let gpu = offscreen
                    .render_studio(&scene.mesh, colors, &scene.camera, &scene.params, &studio)
                    .unwrap();
                let stats = compare(scene, colors, cpu, &gpu);
                let label = format!("readback {adapter} | {} | {term}", scene.name);
                eprintln!(
                    "MEASURE {label}: {stats:?} share {:.6} in doubt {:.6}",
                    stats.share(),
                    stats.doubt_share()
                );
                report(&label, cpu, &gpu);
                if term.starts_with("glass") || *term == "all" {
                    let tag = term.replace([',', ' '], "");
                    if index == 0 {
                        craft3d_testkit::evidence::print(
                            &format!("layers-cpu-{k}-{tag}.png"),
                            &cpu.frame.to_png().unwrap(),
                        );
                    }
                    craft3d_testkit::evidence::print(
                        &format!("layers-gpu-{k}-{tag}-{}.png", adapter_tag(&adapter)),
                        &gpu.to_png().unwrap(),
                    );
                }
                check(&mut failures, &label, &stats);
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// The same input gives the same frame bit for bit, with every layer and
/// with glass, also accumulated.
#[test]
fn layer_three_is_deterministic() {
    let scenes = scenes();
    let (_, all) = terms().into_iter().last().unwrap();
    for mut offscreen in offscreens() {
        let adapter = offscreen.adapter().to_string();
        for scene in &scenes {
            let colors = scene.colors_with(&all);
            for look in [
                Look::polished(),
                Look {
                    accumulate: 4,
                    ..Look::polished()
                },
            ] {
                let studio = Studio {
                    env: scene.env.clone(),
                    look,
                };
                let mut render = || {
                    offscreen
                        .render_studio(&scene.mesh, &colors, &scene.camera, &scene.params, &studio)
                        .unwrap()
                };
                let first = render();
                let again = render();
                assert_eq!(first, again, "{adapter} {}", scene.name);
            }
        }
    }
}

fn overlay() -> OverlayParams {
    OverlayParams {
        slack: 0.0,
        hidden_opacity: 70,
        axis_length: 1.0,
        edges: EdgeLook::default(),
    }
}

/// The viewport's renderer (its deferred path, one sample per pixel)
/// against the CPU at `ssaa` 1, every term in the polished look: one
/// frame, then the mean of four.
#[test]
fn the_viewport_draws_layer_three_as_the_cpu_renderer() {
    let mut failures = Vec::new();
    let terms = terms();
    for (adapter, device, queue) in devices::devices() {
        let name = adapter.get_info().name;
        let mut onscreen = Onscreen::new(&device, &queue, &adapter).unwrap();
        let lines = onscreen.upload_lines(&[]).unwrap();
        for scene in scenes() {
            let params = RenderParams {
                ssaa: 1,
                ..scene.params
            };
            for (term, layers) in &terms {
                let colors = scene.colors_with(layers);
                let frames = if *term == "all" { 4 } else { 1 };
                let studio = onscreen.studio_as_drawn(&Studio {
                    env: scene.env.clone(),
                    look: Look {
                        accumulate: frames,
                        ..Look::polished()
                    },
                });
                let uploaded = onscreen.upload_colored(&scene.mesh, &colors).unwrap();
                onscreen.set_studio(Some(studio.clone()));
                let mut gpu = None;
                for _ in 0..frames {
                    onscreen
                        .draw(
                            &uploaded,
                            &lines,
                            &scene.camera,
                            &params,
                            &overlay(),
                            DrawOptions {
                                bodies: true,
                                capture: true,
                            },
                        )
                        .unwrap();
                    gpu = Some(onscreen.read_frame().unwrap());
                }
                let gpu = gpu.unwrap();
                assert!(!onscreen.converging(), "{name} {} {term}", scene.name);
                let cpu = cpu_render(&scene, &colors, &params, &studio.look);
                let stats = compare(&scene, &colors, &cpu, &gpu);
                let label = format!("viewport {name} | {} | {term}", scene.name);
                eprintln!(
                    "MEASURE {label}: {stats:?} share {:.6} in doubt {:.6}",
                    stats.share(),
                    stats.doubt_share()
                );
                report(&label, &cpu, &gpu);
                check(&mut failures, &label, &stats);
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// The cost of a frame with glass on a model of a thousand faces (every
/// seventh part glass), against the same model without: printed for the
/// dev log.
#[test]
fn layer_three_frames_on_a_thousand_faces() {
    let env = environment("studio").unwrap();
    let mut mesh = Mesh::default();
    let mut colors = FaceColors::new();
    let mut faces = 0_u32;
    let mut part_index = 0_usize;
    'grid: for gy in 0..20 {
        for gx in 0..20_usize {
            let origin = Vec3::new(gx as f64 * 5.0, gy as f64 * 5.0, 0.0);
            let frame = craft3d_math::Frame::new(origin, Vec3::Z, Vec3::X).unwrap();
            let mut all = solids::all(&frame, 1.0).unwrap();
            all.extend(trimmed::all(&frame, 1.0).unwrap());
            let solid = &all[(gx + gy * 20) % all.len()];
            let part = tessellate_body(&solid.body, &TessParams::default()).unwrap();
            let base = mesh.positions.len() as u32;
            let first = faces;
            let count = part.face_ids.iter().copied().max().map_or(0, |m| m + 1);
            mesh.positions.extend(part.positions);
            mesh.normals.extend(part.normals);
            mesh.triangles
                .extend(part.triangles.iter().map(|t| t.map(|i| i + base)));
            mesh.face_ids
                .extend(part.face_ids.iter().map(|f| f + first));
            if part_index % 7 == 3 {
                for face in first..first + count {
                    colors.set_optics(face, Some(glass(false)));
                }
            }
            part_index += 1;
            faces += count;
            if faces >= 1000 {
                break 'grid;
            }
        }
    }
    let p = params(1020, 777, 1);
    let camera = perspective(&mesh, Vec3::new(1.0, -1.4, 0.9), 35.0, &p);
    for mut offscreen in offscreens() {
        let adapter = offscreen.adapter().to_string();
        for (label, colors) in [
            ("without glass", FaceColors::new()),
            ("with glass", colors.clone()),
        ] {
            for (look_label, look) in [("layer 1", Look::default()), ("polished", Look::polished())]
            {
                let studio = Studio {
                    env: env.clone(),
                    look,
                };
                let renderer = offscreen.renderer();
                let uploaded = renderer.upload_colored(&mesh, &colors).unwrap();
                renderer
                    .draw_studio(&uploaded, &camera, &p, &studio)
                    .unwrap();
                renderer.read_frame().unwrap();
                let started = std::time::Instant::now();
                for _ in 0..4 {
                    renderer
                        .draw_studio(&uploaded, &camera, &p, &studio)
                        .unwrap();
                    renderer.read_frame().unwrap();
                }
                let ms = started.elapsed().as_secs_f64() * 1000.0 / 4.0;
                eprintln!(
                    "COST readback {adapter}: {faces} faces, 1020x777, {look_label}, {label}: {ms:.1} ms a picture (with readback)"
                );
            }
        }
    }
    let started = std::time::Instant::now();
    let frame = render_studio(&mesh, &colors, &camera, &p, &Look::polished(), &env).unwrap();
    eprintln!(
        "COST cpu: {:?} for the polished frame with glass",
        started.elapsed()
    );
    craft3d_testkit::evidence::print("layers-grid-cpu.png", &frame.to_png().unwrap());
}
