//! The studio look of see-through and cut-open pictures on the GPU
//! against the CPU renderer's (`craft3d_render::studio::render_studio`):
//! a housing over parts seen through, cut open, and both; layer 1's look
//! and the polished one; on the readback renderer and on the viewport's;
//! determinism (docs/design/rendering.md, *See-through and cut-open
//! pictures in the studio look*).
//!
//! The comparison is layer 2's (`polish.rs`), with the band of the
//! picture as the plain look's parity tests take it for a cut-open,
//! see-through picture (`common::band_of`: the silhouettes, the outline
//! of the section, the changes of the face behind the front): outside it
//! and outside the pixels in doubt, the ids equal, at most
//! [`SHARE_OVER_ONE`] of the pixels more than one level apart and none
//! more than [`MOST_LEVELS`].

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;
mod devices;

use common::{band_of, offscreens, orthographic, params, perspective};
use craft3d_gpu::{
    Camera, DrawOptions, EdgeLook, FaceColors, Frame, Mesh, Onscreen, OverlayParams, RenderParams,
    Studio,
};
use craft3d_math::Vec3;
use craft3d_render::studio::layers::{Optics, absorption_coefficients};
use craft3d_render::studio::polish::TRACE_DOUBT;
use craft3d_render::studio::{Look, Prepared, bundled::environment, render_studio_doubt};
use craft3d_render::{Finish, Highlight, SECTION_FACE, Section};
use craft3d_tess::{TessParams, tessellate_body};
use craft3d_testkit::solids;
use std::sync::Arc;

/// The bounds, measured (Strix run 41142710329, 2026-10-31: the readback
/// renderer and the viewport on RADV, radeonsi and llvmpipe; two scenes,
/// the first with a glass ball and highlighted faces, the second with a
/// block standing on the plate; three kinds, two looks): ids equal; at
/// most 2 pixels more than one level apart, 2 levels at most; at most
/// 0.27 % of the pixels in doubt, all of them where two faces in one plane
/// lie behind a see-through front (the block on the plate: 21 to 22 levels
/// apart there, the other face's light), the mean share in doubt as much
/// (0.24 %: such pixels are wholly in doubt). The bounds: 1/4096 over one
/// level, 8 levels (layer 2's), 1/256 in doubt and in the mean share (a
/// margin of 1.4 and 1.6; layer 2's 1/512 for the mean is for traced rays,
/// which leave a pixel partly in doubt), layer 2's threshold for a pixel
/// in doubt (1/64).
const SHARE_OVER_ONE: f64 = 1.0 / 4096.0;
const MOST_LEVELS: u8 = 8;
const DOUBT_SHARE: f32 = 1.0 / 64.0;
const MOST_DOUBT: f64 = 1.0 / 256.0;
const MOST_MEAN_DOUBT: f64 = 1.0 / 256.0;

struct Scene {
    name: &'static str,
    mesh: Mesh,
    colors: FaceColors,
    camera: Camera,
    params: RenderParams,
    env: Arc<Prepared>,
    plain: bool,
}

fn part(solid: &solids::Reference, color: [u8; 3], finish: Finish) -> (Mesh, [u8; 3], Finish) {
    let mesh = tessellate_body(
        &solid.body,
        &TessParams {
            chordal_tolerance: 0.01,
            ..TessParams::default()
        },
    )
    .unwrap();
    (mesh, color, finish)
}

/// The parts in one mesh, each face with its part's colour and finish;
/// and the first face of each part.
fn combine(parts: Vec<(Mesh, [u8; 3], Finish)>) -> (Mesh, FaceColors, Vec<u32>) {
    let mut mesh = Mesh::default();
    let mut colors = FaceColors::new();
    let mut faces = 0_u32;
    let mut firsts = Vec::new();
    for (part, color, finish) in parts {
        let base = mesh.positions.len() as u32;
        let first = faces;
        firsts.push(first);
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
        faces += count;
    }
    (mesh, colors, firsts)
}

/// The three kinds of picture of a scene: seen through, cut open, both.
fn kinds(section: Section) -> [(&'static str, f64, Option<Section>); 3] {
    [
        ("see-through", 0.4, None),
        ("cut open", 1.0, Some(section)),
        ("both", 0.55, Some(section)),
    ]
}

/// A housing over parts (a block around a ball, a ring and a post), and a
/// plate with a ball and a cone, each with its section.
fn scenes() -> Vec<(Scene, Section)> {
    let at = |x: f64, y: f64, z: f64| {
        craft3d_math::Frame::new(Vec3::new(x, y, z), Vec3::Z, Vec3::X).unwrap()
    };
    let gloss = Finish::from_factors(Some(0.0), Some(0.2));
    let rough = Finish::from_factors(Some(0.0), Some(0.6));
    let metal = Finish::from_factors(Some(1.0), Some(0.3));
    let housing = solids::block(&at(0.0, 0.0, 0.0), 4.0, 3.0, 2.0).unwrap();
    let ball = solids::sphere(&at(1.1, 1.5, 1.0), 0.6).unwrap();
    let ring = solids::torus(&at(2.8, 1.5, 1.0), 0.6, 0.2).unwrap();
    let post = solids::cylinder(&at(2.8, 1.5, 0.2), 0.15, 1.6).unwrap();
    let (mesh_a, mut colors_a, firsts) = combine(vec![
        part(&housing, [70, 110, 160], gloss),
        part(&ball, [200, 70, 50], rough),
        part(&ring, [210, 170, 80], metal),
        part(&post, [90, 160, 90], gloss),
    ]);
    // The ball is glass (layer 3's transmission, solid): in a see-through
    // picture it passes the surface behind it as a thin sheet; the post's
    // faces are highlighted: not seen through.
    let ball_faces = firsts[1]..firsts[2];
    for face in ball_faces {
        colors_a.set_optics(
            face,
            Some(Optics {
                transmission: 0.85,
                ior: 1.45,
                absorption: absorption_coefficients([150, 210, 190], 0.6),
                ..Optics::default()
            }),
        );
    }
    let post_faces = firsts[3]..firsts[3] + 3;
    let plate = solids::block(&at(0.0, 0.0, 0.0), 4.0, 4.0, 0.3).unwrap();
    let globe = solids::sphere(&at(2.0, 2.0, 1.0), 0.8).unwrap();
    let cone = solids::cone(&at(1.0, 1.0, 0.3), 0.5, 1.2).unwrap();
    // A block standing on the plate: its bottom face and the plate's top
    // lie in one plane (the surface behind the front is a tie there).
    let block = solids::block(&at(2.9, 0.3, 0.3), 0.8, 0.8, 0.6).unwrap();
    let (mesh_b, colors_b, _) = combine(vec![
        part(&plate, [150, 150, 155], rough),
        part(&globe, [230, 190, 90], gloss),
        part(&cone, [60, 120, 200], metal),
        part(&block, [200, 90, 140], rough),
    ]);
    let pa = RenderParams {
        highlight: Highlight::new([250, 200, 30], post_faces).0,
        ..params(300, 220, 1)
    };
    let pb = params(257, 193, 2);
    vec![
        (
            Scene {
                name: "housing over parts",
                camera: perspective(&mesh_a, Vec3::new(0.6, -1.3, 0.8), 35.0, &pa),
                mesh: mesh_a,
                colors: colors_a,
                params: pa,
                env: environment("outdoor").unwrap(),
                plain: false,
            },
            Section {
                origin: Vec3::new(2.0, 1.4, 1.0),
                normal: Vec3::new(0.0, -1.0, 0.25),
                cap: true,
                color: [200, 90, 60],
            },
        ),
        (
            Scene {
                name: "plate, globe and cone, orthographic",
                camera: orthographic(&mesh_b, Vec3::new(0.9, -1.0, 0.8), &pb),
                mesh: mesh_b,
                colors: colors_b,
                params: pb,
                env: environment("studio").unwrap(),
                plain: true,
            },
            Section {
                origin: Vec3::new(1.6, 2.0, 0.5),
                normal: Vec3::new(0.6, -0.8, 0.0),
                cap: true,
                color: [120, 180, 90],
            },
        ),
    ]
}

fn looks(plain: bool) -> Vec<(&'static str, Look)> {
    vec![
        (
            "layer 1",
            Look {
                plain_background: plain,
                ..Look::default()
            },
        ),
        (
            "polished",
            Look {
                plain_background: plain,
                ..Look::polished()
            },
        ),
    ]
}

#[derive(Default, Debug, Clone, Copy)]
struct Stats {
    compared: usize,
    over_one: usize,
    most: u8,
    id_outside: usize,
    doubt: usize,
    most_in_doubt: u8,
    share_sum: f64,
}

impl Stats {
    fn share(&self) -> f64 {
        self.over_one as f64 / self.compared.max(1) as f64
    }

    fn doubt_share(&self) -> f64 {
        self.doubt as f64 / (self.compared + self.doubt).max(1) as f64
    }

    fn mean_doubt(&self) -> f64 {
        self.share_sum / (self.compared + self.doubt).max(1) as f64
    }
}

fn compare(scene: &Scene, params: &RenderParams, cpu: &Frame, doubt: &[f32], gpu: &Frame) -> Stats {
    let band = band_of(&scene.mesh, &scene.camera, params, cpu);
    let mut stats = Stats::default();
    let diff = |i: usize| {
        (0..3)
            .map(|k| cpu.rgb[3 * i + k].abs_diff(gpu.rgb[3 * i + k]))
            .max()
            .unwrap()
    };
    for (i, &in_band) in band.iter().enumerate().take(cpu.ids.len()) {
        if in_band {
            continue;
        }
        stats.share_sum += f64::from(doubt[i]);
        if doubt[i] > DOUBT_SHARE {
            stats.doubt += 1;
            stats.most_in_doubt = stats.most_in_doubt.max(diff(i));
            continue;
        }
        stats.compared += 1;
        if cpu.ids[i] != gpu.ids[i] {
            stats.id_outside += 1;
        }
        let d = diff(i);
        if d > 1 {
            stats.over_one += 1;
        }
        stats.most = stats.most.max(d);
    }
    stats
}

fn check(failures: &mut Vec<String>, label: &str, stats: &Stats) {
    eprintln!(
        "MEASURE {label}: {stats:?} share {:.6} in doubt {:.6} mean {:.6}",
        stats.share(),
        stats.doubt_share(),
        stats.mean_doubt()
    );
    if stats.id_outside != 0
        || stats.share() > SHARE_OVER_ONE
        || stats.most > MOST_LEVELS
        || stats.doubt_share() > MOST_DOUBT
        || stats.mean_doubt() > MOST_MEAN_DOUBT
    {
        failures.push(format!("{label}: {stats:?}"));
    }
}

fn cpu_render(scene: &Scene, params: &RenderParams, look: &Look) -> (Frame, Vec<f32>) {
    render_studio_doubt(
        &scene.mesh,
        &scene.colors,
        &scene.camera,
        params,
        look,
        &scene.env,
        TRACE_DOUBT,
    )
    .unwrap()
}

fn tag(adapter: &str) -> String {
    adapter
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(24)
        .collect()
}

#[test]
fn see_through_and_cut_open_studio_pictures_match_the_cpu_renderer() {
    let mut failures = Vec::new();
    let scenes = scenes();
    // The CPU's pictures, once for every adapter.
    let mut cpus = Vec::new();
    for (k, (scene, section)) in scenes.iter().enumerate() {
        for (kind, opacity, cut) in kinds(*section) {
            let p = RenderParams {
                opacity,
                section: cut,
                ..scene.params
            };
            for (name, look) in looks(scene.plain) {
                let (frame, doubt) = cpu_render(scene, &p, &look);
                assert!(
                    cut.is_none() || frame.ids.contains(&SECTION_FACE),
                    "{} {kind}: the cap is in sight",
                    scene.name
                );
                if name == "polished" {
                    craft3d_testkit::evidence::print(
                        &format!("studio-cut-cpu-{k}-{}.png", tag(kind)),
                        &frame.to_png().unwrap(),
                    );
                }
                cpus.push((k, kind, p, name, look, frame, doubt));
            }
        }
    }
    for mut offscreen in offscreens() {
        let adapter = offscreen.adapter().to_string();
        for (k, kind, p, name, look, cpu, doubt) in &cpus {
            let scene = &scenes[*k].0;
            let studio = Studio {
                env: scene.env.clone(),
                look: *look,
            };
            let gpu = offscreen
                .render_studio(&scene.mesh, &scene.colors, &scene.camera, p, &studio)
                .unwrap();
            if *name == "polished" {
                craft3d_testkit::evidence::print(
                    &format!("studio-cut-gpu-{k}-{}-{}.png", tag(kind), tag(&adapter)),
                    &gpu.to_png().unwrap(),
                );
            }
            let stats = compare(scene, p, cpu, doubt, &gpu);
            check(
                &mut failures,
                &format!("readback {adapter} | {} | {kind} | {name}", scene.name),
                &stats,
            );
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn see_through_and_cut_open_studio_pictures_are_deterministic() {
    let (scene, section) = scenes().into_iter().next().unwrap();
    let p = RenderParams {
        opacity: 0.5,
        section: Some(section),
        ..scene.params
    };
    for mut offscreen in offscreens() {
        let studio = Studio {
            env: scene.env.clone(),
            look: Look {
                accumulate: 4,
                ..Look::polished()
            },
        };
        let mut render = || {
            offscreen
                .render_studio(&scene.mesh, &scene.colors, &scene.camera, &p, &studio)
                .unwrap()
        };
        let first = render();
        let again = render();
        assert_eq!(first, again);
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

/// The viewport's renderer: a see-through and a cut-open picture take its
/// deferred path (one sample per pixel, frames summed); after
/// `Look::accumulate` draws, the CPU's accumulation of the same frames.
#[test]
fn the_viewport_draws_see_through_and_cut_open_studio_pictures_as_the_cpu() {
    let mut failures = Vec::new();
    let frames = 4;
    for (adapter, device, queue) in devices::devices() {
        let name = adapter.get_info().name;
        let mut onscreen = Onscreen::new(&device, &queue, &adapter).unwrap();
        let lines = onscreen.upload_lines(&[]).unwrap();
        for (k, (scene, section)) in scenes().into_iter().enumerate() {
            let uploaded = onscreen.upload_colored(&scene.mesh, &scene.colors).unwrap();
            for (kind, opacity, cut) in kinds(section) {
                let p = RenderParams {
                    ssaa: 1,
                    opacity,
                    section: cut,
                    ..scene.params
                };
                let studio = onscreen.studio_as_drawn(&Studio {
                    env: scene.env.clone(),
                    look: Look {
                        accumulate: frames,
                        plain_background: scene.plain,
                        ..Look::polished()
                    },
                });
                onscreen.set_studio(Some(studio.clone()));
                let mut gpu = None;
                for _ in 0..frames {
                    onscreen
                        .draw(
                            &uploaded,
                            &lines,
                            &scene.camera,
                            &p,
                            &overlay(),
                            DrawOptions {
                                bodies: true,
                                capture: true,
                            },
                        )
                        .unwrap();
                    gpu = Some(onscreen.read_frame().unwrap());
                }
                assert!(
                    onscreen.studio_failure().is_none(),
                    "{name} {kind}: {:?}",
                    onscreen.studio_failure()
                );
                let gpu = gpu.unwrap();
                let (cpu, doubt) = cpu_render(&scene, &p, &studio.look);
                if k == 0 {
                    craft3d_testkit::evidence::print(
                        &format!("studio-cut-viewport-{}-{}.png", tag(kind), tag(&name)),
                        &gpu.to_png().unwrap(),
                    );
                }
                let stats = compare(&scene, &p, &cpu, &doubt, &gpu);
                check(
                    &mut failures,
                    &format!("viewport {name} | {} | {kind}", scene.name),
                    &stats,
                );
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// The cost on the viewport (rule 12), as `layer_two_frames_on_a_thousand_faces`:
/// the grid of reference solids, about a thousand faces, at 1020 × 777;
/// opaque and whole, seen through, cut open and both, in layer 1's look and
/// the polished one: printed for the dev log.
#[test]
fn see_through_and_cut_open_frames_on_a_thousand_faces() {
    use craft3d_testkit::trimmed;
    let env = environment("studio").unwrap();
    let mut mesh = Mesh::default();
    let mut faces = 0_u32;
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
            mesh.positions.extend(part.positions);
            mesh.normals.extend(part.normals);
            mesh.triangles
                .extend(part.triangles.iter().map(|t| t.map(|i| i + base)));
            mesh.face_ids
                .extend(part.face_ids.iter().map(|f| f + first));
            faces += part.face_ids.iter().copied().max().map_or(0, |m| m + 1);
            if faces >= 1000 {
                break 'grid;
            }
        }
    }
    let p = params(1020, 777, 1);
    let camera = perspective(&mesh, Vec3::new(1.0, -1.4, 0.9), 35.0, &p);
    let other = perspective(&mesh, Vec3::new(1.0, -1.35, 0.9), 35.0, &p);
    let center = mesh.bbox().center().unwrap();
    let section = Section {
        origin: center,
        normal: Vec3::new(0.3, -1.0, 0.0),
        cap: true,
        color: [200, 90, 60],
    };
    let colors = FaceColors::new();
    for (adapter, device, queue) in devices::devices() {
        let name = adapter.get_info().name;
        let mut onscreen = Onscreen::new(&device, &queue, &adapter).unwrap();
        let lines = onscreen.upload_lines(&[]).unwrap();
        let uploaded = onscreen.upload_colored(&mesh, &colors).unwrap();
        for (look_name, look) in [("layer 1", Look::default()), ("polished", Look::polished())] {
            for (kind, opacity, cut) in [
                ("opaque", 1.0, None),
                ("see-through", 0.5, None),
                ("cut open", 1.0, Some(section)),
                ("both", 0.5, Some(section)),
            ] {
                let params = RenderParams {
                    opacity,
                    section: cut,
                    ..p
                };
                onscreen.set_studio(Some(Studio {
                    env: env.clone(),
                    look,
                }));
                let draws = if name.contains("llvmpipe") { 5 } else { 16 };
                let mut times = Vec::new();
                for i in 0..draws {
                    // A new camera every draw: each draw draws a frame.
                    let cam = if i % 2 == 0 { &camera } else { &other };
                    let started = std::time::Instant::now();
                    onscreen
                        .draw(
                            &uploaded,
                            &lines,
                            cam,
                            &params,
                            &overlay(),
                            DrawOptions::default(),
                        )
                        .unwrap();
                    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
                    if i > 0 {
                        times.push(started.elapsed().as_secs_f64() * 1000.0);
                    }
                }
                let ms = times.iter().sum::<f64>() / times.len() as f64;
                eprintln!(
                    "COST viewport {name} ({} samples): {} triangles, 1020x777, {look_name}, {kind}: {ms:.2} ms a frame",
                    onscreen.samples(),
                    mesh.triangles.len(),
                );
            }
        }
    }
}
