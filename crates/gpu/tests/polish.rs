//! Layer 2 of the studio look on the GPU against the CPU renderer's
//! (`craft3d_render::studio::polish`): each effect alone and all together,
//! on the readback renderer and on the viewport's, determinism, and the
//! cost (docs/design/rendering.md, *The studio look*, *Layer 2*).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
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
use craft3d_render::studio::polish::{Dof, Focus, TRACE_DOUBT};
use craft3d_render::studio::{
    Look, Prepared, bundled::environment, render_studio, render_studio_doubt,
};
use craft3d_tess::{TessParams, tessellate_body};
use craft3d_testkit::{solids, trimmed};
use std::sync::Arc;

/// The tolerance of layer 2, GPU against CPU: outside the band and the
/// pixels in doubt, at most this share of the pixels compared may differ
/// by more than one level, and none by more than [`MOST_LEVELS`].
///
/// The pixels in doubt: a reflected ray that the CPU decides within
/// `polish::TRACE_DOUBT` of a boundary (a depth on the ray against the
/// surface's, the thickness's limit, a sample's border, a face edge-on to
/// the ray) may be decided the other way by the GPU's `f32`, and its sample
/// then shows another surface's light: any number of levels apart, as the
/// band's pixels may be. `render_studio_doubt` gives each pixel the share
/// of its light from such samples (over its samples, its frames and the
/// lens's gather); a pixel whose share passes [`DOUBT_SHARE`] is left out,
/// as the band is, and counted: at most [`MOST_DOUBT`] of the pixels.
///
/// Measured before the pixels in doubt were left out (run 40973956505,
/// 2026-10-30): the readback renderer on its three adapters, three
/// scenes, eight looks, at most 8 of 175 721 pixels over one level for one
/// effect over all scenes, the largest difference 50 levels (reflections,
/// 1 pixel: a hit decided in `f32` against `f64`) and 9 without
/// reflections; the viewport 48 levels. With them left out (run
/// 41000166136, 2026-10-30, the same adapters, scenes and looks): at most
/// 1/6 000 of the pixels compared over one level, at most 6 levels
/// anywhere, every look; the pixels that differed by 8 to 50 levels were
/// all in doubt. The bounds: the share as before (1/512, a margin of 8),
/// 8 levels (a margin of 2 over the 6 measured; looks without a traced
/// ray measured 3). Before the review the bound was 16 levels and 96 for
/// looks that trace a ray and for the viewport.
const SHARE_OVER_ONE: f64 = 1.0 / 512.0;
const MOST_LEVELS: u8 = 8;
/// A share of a pixel's light in doubt above which it is left out: a
/// pixel's bytes are the mean of its samples' over its frames, so a share
/// under 1/64 of samples that may show another surface moves it by at
/// most 255 / 64, about 4 levels (in linear light, under the lens, by
/// about that times the tone curve's slope).
const DOUBT_SHARE: f32 = 1.0 / 64.0;
/// The most pixels in doubt, as a share of those outside the band.
/// Measured at most 1.8 % (the polished look with the lens and 8 frames
/// on the scene of four parts, whose mirror ball and plate reflect the
/// others: a pixel is left out when any of its samples in any frame, or
/// enough of its lens's taps, were in doubt), 0.36 % for reflections
/// alone in one frame; 1/32 holds that with a margin of 1.7.
const MOST_DOUBT: f64 = 1.0 / 32.0;
/// The largest mean share of a pixel's light in doubt: how much of the
/// picture's light the decisions in doubt carry, whatever the frames and
/// the lens spread it over. Measured at most 0.14 %; 1/512 holds that
/// with a margin of 1.4.
const MOST_MEAN_DOUBT: f64 = 1.0 / 512.0;

struct Scene {
    name: &'static str,
    mesh: Mesh,
    colors: FaceColors,
    camera: Camera,
    params: RenderParams,
    env: Arc<Prepared>,
    /// Where the lens focuses.
    focus: Vec3,
    /// The picture's own background instead of the environment.
    plain: bool,
}

/// A part of a scene: a reference solid's mesh, its colour and finish.
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

fn combine(parts: Vec<(Mesh, [u8; 3], Finish)>) -> (Mesh, FaceColors) {
    let mut mesh = Mesh::default();
    let mut colors = FaceColors::new();
    let mut faces = 0_u32;
    for (part, color, finish) in parts {
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
        faces += count;
    }
    (mesh, colors)
}

fn scenes() -> Vec<Scene> {
    let at = |x: f64, y: f64, z: f64| {
        craft3d_math::Frame::new(Vec3::new(x, y, z), Vec3::Z, Vec3::X).unwrap()
    };
    let mirror = Finish::from_factors(Some(1.0), Some(0.08));
    let gloss = Finish::from_factors(Some(0.0), Some(0.15));
    let rough = Finish::from_factors(Some(0.0), Some(0.6));
    let plate = solids::block(&at(0.0, 0.0, 0.0), 4.0, 4.0, 0.2).unwrap();
    let ball = solids::sphere(&at(2.0, 2.0, 0.9), 0.7).unwrap();
    let post = solids::cylinder(&at(0.8, 3.0, 0.2), 0.3, 1.4).unwrap();
    let ring = solids::torus(&at(3.0, 1.0, 0.45), 0.6, 0.22).unwrap();
    let cone = solids::cone(&at(1.0, 1.0, 0.2), 0.5, 1.2).unwrap();
    let (mesh_a, colors_a) = combine(vec![
        part(&plate, [190, 196, 205], mirror),
        part(&ball, [200, 60, 50], rough),
        part(&post, [60, 120, 200], gloss),
    ]);
    let (mesh_b, colors_b) = combine(vec![
        part(&plate, [40, 44, 52], gloss),
        part(&ball, [230, 190, 90], mirror),
        part(&ring, [90, 160, 90], rough),
        part(&cone, [220, 220, 225], gloss),
    ]);
    let (mesh_c, colors_c) = combine(vec![
        part(&ring, [200, 140, 80], mirror),
        part(&cone, [60, 120, 200], gloss),
        part(&plate, [150, 150, 150], rough),
    ]);
    let studio = environment("studio").unwrap();
    let outdoor = environment("outdoor").unwrap();
    let pa = params(320, 240, 1);
    let pb = params(257, 193, 2);
    let pc = params(300, 220, 1);
    vec![
        Scene {
            name: "plate and ball",
            camera: perspective(&mesh_a, Vec3::new(1.0, -1.3, 0.6), 35.0, &pa),
            mesh: mesh_a,
            colors: colors_a,
            params: pa,
            env: studio,
            focus: Vec3::new(2.0, 2.0, 0.9),
            plain: true,
        },
        Scene {
            name: "four parts outdoors",
            camera: perspective(&mesh_b, Vec3::new(-0.8, -1.2, 0.7), 40.0, &pb),
            mesh: mesh_b,
            colors: colors_b,
            params: pb,
            env: outdoor.clone(),
            focus: Vec3::new(3.0, 1.0, 0.45),
            plain: false,
        },
        Scene {
            name: "ring and cone, orthographic",
            camera: orthographic(&mesh_c, Vec3::new(0.9, -1.0, 0.8), &pc),
            mesh: mesh_c,
            colors: colors_c,
            params: pc,
            env: outdoor,
            focus: Vec3::new(1.0, 1.0, 0.8),
            plain: true,
        },
    ]
}

fn dof(focus: Vec3) -> Option<Dof> {
    Some(Dof {
        fstop: 2.8,
        focus: Focus::Point([focus.x, focus.y, focus.z]),
        mm_per_unit: 10.0,
    })
}

/// Each effect alone, then all together.
fn looks(focus: Vec3, plain: bool) -> Vec<(&'static str, Look)> {
    let base = Look {
        plain_background: plain,
        ..Look::default()
    };
    vec![
        ("ssr", Look { ssr: true, ..base }),
        (
            "contact",
            Look {
                contact_shadows: true,
                ..base
            },
        ),
        (
            "specular shadow",
            Look {
                specular_shadow: true,
                ..base
            },
        ),
        (
            "ground reflection",
            Look {
                ground_reflection: 1.0,
                ..base
            },
        ),
        (
            "dof",
            Look {
                dof: dof(focus),
                ..base
            },
        ),
        (
            "bloom",
            Look {
                bloom: 0.25,
                ..base
            },
        ),
        (
            "accumulate 8",
            Look {
                accumulate: 8,
                ..base
            },
        ),
        (
            "all",
            Look {
                dof: dof(focus),
                accumulate: 8,
                plain_background: plain,
                ground_reflection: 1.0,
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
    /// Pixels outside the band left out as in doubt.
    doubt: usize,
    /// The largest difference among them (for the dev log only).
    most_in_doubt: u8,
    /// The sum of the shares in doubt over the pixels outside the band.
    share_sum: f64,
}

impl Stats {
    fn share(&self) -> f64 {
        self.over_one as f64 / self.compared.max(1) as f64
    }

    fn doubt_share(&self) -> f64 {
        self.doubt as f64 / (self.compared + self.doubt).max(1) as f64
    }

    /// The mean share of a pixel's light in doubt.
    fn mean_doubt(&self) -> f64 {
        self.share_sum / (self.compared + self.doubt).max(1) as f64
    }

    fn merge(&mut self, other: Stats) {
        self.compared += other.compared;
        self.over_one += other.over_one;
        self.most = self.most.max(other.most);
        self.id_outside += other.id_outside;
        self.doubt += other.doubt;
        self.most_in_doubt = self.most_in_doubt.max(other.most_in_doubt);
        self.share_sum += other.share_sum;
    }
}

/// Outside the band (`common::band`) and the pixels in doubt (`doubt`,
/// per pixel, from `render_studio_doubt`): every pixel compared, the ids
/// equal, the colours by how much they differ.
fn compare(scene: &Scene, cpu: &Frame, doubt: &[f32], gpu: &Frame) -> Stats {
    let band = band(&scene.mesh, &scene.camera, cpu);
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

/// What of the tolerance `stats` breaks, if anything; the tests gather
/// these and fail at the end, so every measurement is printed.
fn check(failures: &mut Vec<String>, label: &str, stats: &Stats) {
    if stats.id_outside != 0
        || stats.share() > SHARE_OVER_ONE
        || stats.most > MOST_LEVELS
        || stats.doubt_share() > MOST_DOUBT
        || stats.mean_doubt() > MOST_MEAN_DOUBT
    {
        failures.push(format!(
            "{label}: {stats:?}, share {}, in doubt {}",
            stats.share(),
            stats.doubt_share()
        ));
    }
}

/// Margins (in `f32` epsilons) measured besides [`TRACE_DOUBT`], for the
/// dev log: how the pixels in doubt and the largest difference left
/// depend on it.
const SWEEP: [f64; 4] = [32.0, 64.0, 256.0, 512.0];

/// The CPU's picture with its doubt at [`TRACE_DOUBT`] and, for a look
/// that traces a ray, at each margin of [`SWEEP`].
struct Cpu {
    frame: Frame,
    doubt: Vec<f32>,
    sweep: Vec<(f64, Vec<f32>)>,
}

fn cpu_render(scene: &Scene, params: &RenderParams, look: &Look) -> Cpu {
    let run = |margin: f64| {
        render_studio_doubt(
            &scene.mesh,
            &scene.colors,
            &scene.camera,
            params,
            look,
            &scene.env,
            margin,
        )
        .unwrap()
    };
    let (frame, doubt) = run(TRACE_DOUBT);
    let traced = look.ssr || look.ground_reflection > 0.0;
    let sweep = if traced {
        SWEEP.iter().map(|&m| (m, run(m).1)).collect()
    } else {
        Vec::new()
    };
    Cpu {
        frame,
        doubt,
        sweep,
    }
}

/// Prints the sweep of the margins and every pixel compared that differs
/// by more than [`MOST_LEVELS`], with its shares in doubt at each margin.
fn report(label: &str, scene: &Scene, cpu: &Cpu, gpu: &Frame) {
    for (m, doubt) in &cpu.sweep {
        let s = compare(scene, &cpu.frame, doubt, gpu);
        eprintln!(
            "SWEEP {label} | margin {m}: in doubt {} ({:.6}), mean {:.6}, most left {}, most in doubt {}",
            s.doubt,
            s.doubt_share(),
            s.mean_doubt(),
            s.most,
            s.most_in_doubt
        );
    }
    let band = band(&scene.mesh, &scene.camera, &cpu.frame);
    let (w, f) = (cpu.frame.width as usize, &cpu.frame);
    for (i, &in_band) in band.iter().enumerate().take(f.ids.len()) {
        let d = (0..3)
            .map(|k| f.rgb[3 * i + k].abs_diff(gpu.rgb[3 * i + k]))
            .max()
            .unwrap();
        if in_band || cpu.doubt[i] > DOUBT_SHARE || d <= MOST_LEVELS {
            continue;
        }
        let shares: Vec<f32> = cpu.sweep.iter().map(|(_, s)| s[i]).collect();
        eprintln!(
            "OFFENDER {label} | ({}, {}) id {} cpu {:?} gpu {:?} doubt {} sweep {shares:?}",
            i % w,
            i / w,
            f.ids[i],
            &f.rgb[3 * i..3 * i + 3],
            &gpu.rgb[3 * i..3 * i + 3],
            cpu.doubt[i]
        );
    }
}

#[test]
fn layer_two_matches_the_cpu_renderer() {
    let scenes = scenes();
    let mut failures = Vec::new();
    // The CPU's pictures, once for every adapter.
    let cpus: Vec<Vec<Cpu>> = scenes
        .iter()
        .map(|scene| {
            looks(scene.focus, scene.plain)
                .iter()
                .map(|(_, look)| cpu_render(scene, &scene.params, look))
                .collect()
        })
        .collect();
    for (index, mut offscreen) in offscreens().into_iter().enumerate() {
        let first_adapter = index == 0;
        let adapter = offscreen.adapter().to_string();
        let mut per_effect: Vec<(&str, Stats)> = Vec::new();
        for (k, scene) in scenes.iter().enumerate() {
            for (j, (effect, look)) in looks(scene.focus, scene.plain).into_iter().enumerate() {
                let studio = Studio {
                    env: scene.env.clone(),
                    look,
                };
                let measured = &cpus[k][j];
                let (cpu, doubt) = (&measured.frame, &measured.doubt);
                let gpu = offscreen
                    .render_studio(
                        &scene.mesh,
                        &scene.colors,
                        &scene.camera,
                        &scene.params,
                        &studio,
                    )
                    .unwrap();
                let stats = compare(scene, cpu, doubt, &gpu);
                eprintln!(
                    "MEASURE readback {adapter} | {} | {effect}: {stats:?} share {:.6} in doubt {:.6} mean {:.6}",
                    scene.name,
                    stats.share(),
                    stats.doubt_share(),
                    stats.mean_doubt()
                );
                report(
                    &format!("readback {adapter} | {} | {effect}", scene.name),
                    scene,
                    measured,
                    &gpu,
                );
                if effect == "all" {
                    if first_adapter {
                        craft3d_testkit::evidence::print(
                            &format!("polish-cpu-{k}.png"),
                            &cpu.to_png().unwrap(),
                        );
                    }
                    craft3d_testkit::evidence::print(
                        &format!("polish-gpu-{k}-{}.png", adapter_tag(&adapter)),
                        &gpu.to_png().unwrap(),
                    );
                }
                match per_effect.iter_mut().find(|(e, _)| *e == effect) {
                    Some((_, s)) => s.merge(stats),
                    None => per_effect.push((effect, stats)),
                }
                check(
                    &mut failures,
                    &format!("{adapter} {} {effect}", scene.name),
                    &stats,
                );
            }
        }
        for (effect, stats) in &per_effect {
            eprintln!(
                "MEASURE readback {adapter} | all scenes | {effect}: {stats:?} share {:.6} in doubt {:.6}",
                stats.share(),
                stats.doubt_share()
            );
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// A short name of an adapter for a file.
fn adapter_tag(adapter: &str) -> String {
    adapter
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(24)
        .collect()
}

/// The same input gives the same frame bit for bit, also accumulated.
#[test]
fn layer_two_is_deterministic() {
    let scenes = scenes();
    for mut offscreen in offscreens() {
        let adapter = offscreen.adapter().to_string();
        for scene in scenes.iter().take(2) {
            for look in [
                Look {
                    dof: dof(scene.focus),
                    plain_background: scene.plain,
                    ..Look::polished()
                },
                Look {
                    dof: dof(scene.focus),
                    accumulate: 8,
                    plain_background: scene.plain,
                    ..Look::polished()
                },
            ] {
                let studio = Studio {
                    env: scene.env.clone(),
                    look,
                };
                let render = |o: &mut craft3d_gpu::Offscreen| {
                    o.render_studio(
                        &scene.mesh,
                        &scene.colors,
                        &scene.camera,
                        &scene.params,
                        &studio,
                    )
                    .unwrap()
                };
                let first = render(&mut offscreen);
                let again = render(&mut offscreen);
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

/// The viewport's renderer on its deferred path (one sample per pixel):
/// after `m` draws of the same picture, the mean of frames 0 to `m − 1`,
/// against the CPU's accumulation of the same frames at `ssaa` 1; it
/// converges after `Look::accumulate` draws, keeps the picture after
/// that, and starts again when the camera moves.
#[test]
fn the_viewport_accumulates_layer_two_as_the_cpu_renderer() {
    let mut failures = Vec::new();
    for (adapter, device, queue) in devices::devices() {
        let name = adapter.get_info().name;
        let mut onscreen = Onscreen::new(&device, &queue, &adapter).unwrap();
        let lines = onscreen.upload_lines(&[]).unwrap();
        for (k, scene) in scenes().into_iter().enumerate() {
            let params = RenderParams {
                ssaa: 1,
                ..scene.params
            };
            let frames = 8;
            let studio = onscreen.studio_as_drawn(&Studio {
                env: scene.env.clone(),
                look: Look {
                    dof: dof(scene.focus),
                    accumulate: frames,
                    plain_background: scene.plain,
                    ..Look::polished()
                },
            });
            let uploaded = onscreen.upload_colored(&scene.mesh, &scene.colors).unwrap();
            onscreen.set_studio(Some(studio.clone()));
            let draw = |onscreen: &mut Onscreen, camera: &Camera| {
                onscreen
                    .draw(
                        &uploaded,
                        &lines,
                        camera,
                        &params,
                        &overlay(),
                        DrawOptions {
                            bodies: true,
                            capture: true,
                        },
                    )
                    .unwrap();
                onscreen.read_frame().unwrap()
            };
            let s = Scene { params, ..scene };
            let mut last = None;
            for m in 1..=frames {
                let gpu = draw(&mut onscreen, &s.camera);
                assert_eq!(
                    onscreen.converging(),
                    m < frames,
                    "{name} {} draw {m}",
                    s.name
                );
                if [1, 2, frames].contains(&m) {
                    let measured = cpu_render(
                        &s,
                        &params,
                        &Look {
                            accumulate: m,
                            ..studio.look
                        },
                    );
                    let cpu = &measured.frame;
                    let stats = compare(&s, cpu, &measured.doubt, &gpu);
                    eprintln!(
                        "MEASURE viewport {name} | {} | {m} frames: {stats:?} share {:.6} in doubt {:.6} mean {:.6}",
                        s.name,
                        stats.share(),
                        stats.doubt_share(),
                        stats.mean_doubt()
                    );
                    report(
                        &format!("viewport {name} | {} | {m} frames", s.name),
                        &s,
                        &measured,
                        &gpu,
                    );
                    if m == frames && k < 2 {
                        craft3d_testkit::evidence::print(
                            &format!("polish-viewport-cpu-{k}.png"),
                            &cpu.to_png().unwrap(),
                        );
                        craft3d_testkit::evidence::print(
                            &format!("polish-viewport-gpu-{k}-{}.png", adapter_tag(&name)),
                            &gpu.to_png().unwrap(),
                        );
                    }
                    check(
                        &mut failures,
                        &format!("viewport {name} {} {m}", s.name),
                        &stats,
                    );
                }
                last = Some(gpu);
            }
            // Converged: the same picture again.
            let kept = draw(&mut onscreen, &s.camera);
            assert!(!onscreen.converging());
            assert_eq!(Some(&kept), last.as_ref(), "{name} {}", s.name);
            // The camera moves: frame 0 of the new picture, then back.
            let mut moved = s.camera;
            moved.eye = moved.eye + (moved.target - moved.eye) * 0.05;
            let _ = draw(&mut onscreen, &moved);
            assert!(onscreen.converging(), "{name} {}", s.name);
            let first = draw(&mut onscreen, &s.camera);
            let measured = cpu_render(
                &s,
                &params,
                &Look {
                    accumulate: 1,
                    ..studio.look
                },
            );
            let stats = compare(&s, &measured.frame, &measured.doubt, &first);
            eprintln!(
                "MEASURE viewport {name} | {} | restarted: {stats:?} share {:.6} in doubt {:.6}",
                s.name,
                stats.share(),
                stats.doubt_share()
            );
            check(
                &mut failures,
                &format!("viewport {name} {} restarted", s.name),
                &stats,
            );
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// The cost of layer 2 on a model of a thousand faces, as
/// `studio_frames_on_a_thousand_faces`: printed for the dev log.
#[test]
fn layer_two_frames_on_a_thousand_faces() {
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
    let center = mesh.bbox().center().unwrap();
    let colors = FaceColors::new();
    let cases = [
        ("layer 1", Look::default()),
        ("polished", Look::polished()),
        (
            "polished + dof",
            Look {
                dof: dof(center),
                ..Look::polished()
            },
        ),
        (
            "polished, accumulate 16",
            Look {
                accumulate: 16,
                ..Look::polished()
            },
        ),
    ];
    for mut offscreen in offscreens() {
        let adapter = offscreen.adapter().to_string();
        for (label, look) in cases {
            let studio = Studio {
                env: env.clone(),
                look,
            };
            let _ = offscreen
                .render_studio(&mesh, &colors, &camera, &p, &studio)
                .unwrap();
            let renderer = offscreen.renderer();
            let uploaded = renderer.upload_colored(&mesh, &colors).unwrap();
            let started = std::time::Instant::now();
            // One picture of sixteen frames; four of one.
            let pictures = if look.accumulate > 1 { 1 } else { 4 };
            for _ in 0..pictures {
                renderer
                    .draw_studio(&uploaded, &camera, &p, &studio)
                    .unwrap();
                renderer.read_frame().unwrap();
            }
            let ms = started.elapsed().as_secs_f64() * 1000.0 / f64::from(pictures);
            eprintln!(
                "COST readback {adapter}: {faces} faces, {} triangles, 1020x777, {label}: \
                 {ms:.1} ms a picture, {:.1} ms a frame (with readback)",
                mesh.triangles.len(),
                ms / f64::from(look.accumulate.max(1)),
            );
        }
    }
    for (adapter, device, queue) in devices::devices() {
        let name = adapter.get_info().name;
        let mut onscreen = Onscreen::new(&device, &queue, &adapter).unwrap();
        let lines = onscreen.upload_lines(&[]).unwrap();
        let uploaded = onscreen.upload_colored(&mesh, &colors).unwrap();
        for (label, look) in cases {
            onscreen.set_studio(Some(Studio {
                env: env.clone(),
                look,
            }));
            // Fewer on the software rasteriser, which is slow.
            let draws = if name.contains("llvmpipe") {
                5
            } else {
                16_usize
            };
            let mut times = Vec::new();
            // Each draw of a new camera starts again at frame 0; the
            // camera alternates so that every draw draws a frame, except
            // for the accumulated look, whose 16 draws are its frames.
            let other = perspective(&mesh, Vec3::new(1.0, -1.35, 0.9), 35.0, &p);
            for i in 0..draws {
                let cam = if look.accumulate > 1 || i % 2 == 0 {
                    &camera
                } else {
                    &other
                };
                let started = std::time::Instant::now();
                onscreen
                    .draw(
                        &uploaded,
                        &lines,
                        cam,
                        &p,
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
                "COST viewport {name} ({} samples): {} triangles, 1020x777, {label}: {ms:.2} ms a frame",
                onscreen.samples(),
                mesh.triangles.len(),
            );
        }
    }
    let started = std::time::Instant::now();
    let frame = render_studio(&mesh, &colors, &camera, &p, &Look::polished(), &env).unwrap();
    eprintln!("COST cpu: {:?} for the polished frame", started.elapsed());
    craft3d_testkit::evidence::print("polish-grid-cpu.png", &frame.to_png().unwrap());
}
