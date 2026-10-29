//! The studio look on the GPU against the CPU renderer's
//! (`craft3d_render::studio`), on every adapter of this machine, and its
//! cost (docs/design/rendering.md, *The studio look*, *Tolerance*).

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
    Camera, DrawOptions, EdgeLook, FaceColors, Frame, Mesh, NO_FACE, Onscreen, OverlayParams,
    RenderParams, Studio,
};
use craft3d_math::Vec3;
use craft3d_render::Finish;
use craft3d_render::studio::{Look, Tonemap, bundled::environment, render_studio};
use craft3d_tess::{TessParams, tessellate_body};
use craft3d_testkit::{solids, trimmed};

/// The studio tolerance (docs/design/rendering.md, *The studio look*):
/// outside the band, at most this share of the covered pixels may differ
/// by more than one level, and none by more than [`MOST_LEVELS`].
const SHARE_OVER_ONE: f64 = 1.0 / 1024.0;
const MOST_LEVELS: u8 = 4;

struct Scene {
    name: String,
    mesh: Mesh,
    colors: FaceColors,
    camera: Camera,
    params: RenderParams,
    look: Look,
}

const PALETTE: [[u8; 3]; 5] = [
    [198, 204, 214],
    [200, 60, 50],
    [60, 120, 200],
    [230, 190, 90],
    [90, 160, 90],
];

fn scenes() -> Vec<Scene> {
    let frame = craft3d_math::Frame::new(
        Vec3::new(3.0, -2.0, 5.0),
        Vec3::new(0.3, -0.2, 1.0),
        Vec3::X,
    )
    .unwrap();
    let mut all = solids::all(&frame, 1.0).unwrap();
    all.extend(trimmed::all(&frame, 1.0).unwrap().into_iter().take(4));
    let looks = [
        Look::default(),
        Look {
            ao: false,
            ..Look::default()
        },
        Look {
            shadows: false,
            tonemap: Tonemap::Aces,
            rotation: 120.0,
            ..Look::default()
        },
        Look {
            exposure: 1.0,
            light: Some([30.0, 50.0]),
            ..Look::default()
        },
    ];
    let mut scenes = Vec::new();
    for (index, solid) in all.iter().enumerate() {
        let mesh = tessellate_body(
            &solid.body,
            &TessParams {
                chordal_tolerance: 0.01,
                ..TessParams::default()
            },
        )
        .unwrap();
        let faces = mesh.face_ids.iter().copied().max().map_or(0, |m| m + 1);
        let mut colors = FaceColors::new();
        for face in 0..faces {
            let k = face as usize + index;
            let finish = Finish::from_factors(
                Some([0.0, 1.0, 0.0][k % 3]),
                Some([0.5, 0.25, 0.85, 0.05][k % 4]),
            );
            colors.set_look(face, Some((PALETTE[k % PALETTE.len()], finish)));
        }
        let params = params(
            [320, 257][index % 2],
            [240, 193][index % 2],
            [1, 2][(index / 2) % 2],
        );
        let toward = [Vec3::new(1.0, -1.3, 0.9), Vec3::new(-0.7, 0.4, 1.2)][index % 2];
        let camera = if index % 3 == 1 {
            orthographic(&mesh, toward, &params)
        } else {
            perspective(&mesh, toward, 35.0, &params)
        };
        scenes.push(Scene {
            name: format!("solid {index} ({})", solid.name),
            mesh,
            colors,
            camera,
            params,
            look: looks[index % looks.len()],
        });
    }
    scenes
}

#[derive(Default, Debug)]
struct Stats {
    covered: usize,
    outside: usize,
    over_one: usize,
    most: u8,
    id_outside: usize,
}

fn compare(scene: &Scene, cpu: &Frame, gpu: &Frame) -> Stats {
    let band = band(&scene.mesh, &scene.camera, cpu);
    let mut stats = Stats::default();
    for (i, &in_band) in band.iter().enumerate().take(cpu.ids.len()) {
        if cpu.ids[i] != NO_FACE {
            stats.covered += 1;
        }
        if in_band {
            continue;
        }
        stats.outside += 1;
        if cpu.ids[i] != gpu.ids[i] {
            stats.id_outside += 1;
        }
        let d = (0..3)
            .map(|k| cpu.rgb[3 * i + k].abs_diff(gpu.rgb[3 * i + k]))
            .max()
            .unwrap();
        if d > 1 {
            stats.over_one += 1;
        }
        stats.most = stats.most.max(d);
    }
    stats
}

#[test]
fn the_studio_look_matches_the_cpu_renderer() {
    let env = environment("studio").unwrap();
    let scenes = scenes();
    for mut offscreen in offscreens() {
        let adapter = offscreen.adapter().to_string();
        for (k, scene) in scenes.iter().enumerate() {
            let studio = Studio {
                env: env.clone(),
                look: scene.look,
            };
            let cpu = render_studio(
                &scene.mesh,
                &scene.colors,
                &scene.camera,
                &scene.params,
                &scene.look,
                &env,
            )
            .unwrap();
            let gpu = offscreen
                .render_studio(
                    &scene.mesh,
                    &scene.colors,
                    &scene.camera,
                    &scene.params,
                    &studio,
                )
                .unwrap();
            let stats = compare(scene, &cpu, &gpu);
            eprintln!("{adapter} {}: {stats:?}", scene.name);
            if k < 2 {
                craft3d_testkit::evidence::print(
                    &format!("studio-cpu-{k}.png"),
                    &cpu.to_png().unwrap(),
                );
                craft3d_testkit::evidence::print(
                    &format!("studio-gpu-{k}.png"),
                    &gpu.to_png().unwrap(),
                );
            }
            assert_eq!(stats.id_outside, 0, "{adapter} {}: ids differ", scene.name);
            assert!(
                stats.over_one as f64 <= SHARE_OVER_ONE * stats.covered.max(1) as f64,
                "{adapter} {}: {stats:?}",
                scene.name
            );
            assert!(
                stats.most <= MOST_LEVELS,
                "{adapter} {}: {stats:?}",
                scene.name
            );
            // The same input gives the same picture.
            let again = offscreen
                .render_studio(
                    &scene.mesh,
                    &scene.colors,
                    &scene.camera,
                    &scene.params,
                    &studio,
                )
                .unwrap();
            assert_eq!(again, gpu, "{adapter} {}", scene.name);
        }
    }
}

/// The cost of a studio frame on a model of a thousand faces: a grid of
/// the reference solids. Printed for the dev log; asserted only to finish.
#[test]
fn studio_frames_on_a_thousand_faces() {
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
    let colors = FaceColors::new();
    for mut offscreen in offscreens() {
        let adapter = offscreen.adapter().to_string();
        for look in [
            Look::default(),
            Look {
                ao: false,
                shadows: false,
                ..Look::default()
            },
        ] {
            let studio = Studio {
                env: env.clone(),
                look,
            };
            // Warm up, then time the frames until the GPU is done.
            let _ = offscreen
                .render_studio(&mesh, &colors, &camera, &p, &studio)
                .unwrap();
            let renderer = offscreen.renderer();
            let uploaded = renderer.upload_colored(&mesh, &colors).unwrap();
            let started = std::time::Instant::now();
            let frames = 10;
            for _ in 0..frames {
                renderer
                    .draw_studio(&uploaded, &camera, &p, &studio)
                    .unwrap();
                renderer.read_frame().unwrap();
            }
            let studio_ms = started.elapsed().as_secs_f64() * 1000.0 / f64::from(frames);
            let started = std::time::Instant::now();
            for _ in 0..frames {
                renderer.draw(&uploaded, &camera, &p).unwrap();
                renderer.read_frame().unwrap();
            }
            let plain_ms = started.elapsed().as_secs_f64() * 1000.0 / f64::from(frames);
            eprintln!(
                "COST {adapter}: {faces} faces, {} triangles, 1020x777: studio (ao {}, shadows {}) \
                 {studio_ms:.1} ms, plain {plain_ms:.1} ms (with readback)",
                mesh.triangles.len(),
                look.ao,
                look.shadows
            );
        }
    }
    // The viewport's renderer: the frame until the GPU is done, nothing
    // read back.
    for (adapter, device, queue) in devices::devices() {
        let name = adapter.get_info().name;
        let mut onscreen = Onscreen::new(&device, &queue, &adapter).unwrap();
        let lines = onscreen.upload_lines(&[]).unwrap();
        let uploaded = onscreen.upload_colored(&mesh, &colors).unwrap();
        let overlay = OverlayParams {
            slack: 0.0,
            hidden_opacity: 70,
            axis_length: 1.0,
            edges: EdgeLook::default(),
        };
        for look in [
            None,
            Some(Look::default()),
            Some(Look {
                ao: false,
                shadows: false,
                ..Look::default()
            }),
        ] {
            onscreen.set_studio(look.map(|look| Studio {
                env: env.clone(),
                look,
            }));
            let mut frame = || {
                onscreen
                    .draw(
                        &uploaded,
                        &lines,
                        &camera,
                        &p,
                        &overlay,
                        DrawOptions::default(),
                    )
                    .unwrap();
                device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            };
            frame();
            let started = std::time::Instant::now();
            for _ in 0..20 {
                frame();
            }
            let ms = started.elapsed().as_secs_f64() * 1000.0 / 20.0;
            eprintln!(
                "COST viewport {name} ({} samples): {} triangles, 1020x777, look {:?}: {ms:.2} ms",
                onscreen.samples(),
                mesh.triangles.len(),
                look.map(|l| (l.ao, l.shadows))
            );
        }
    }
    let started = std::time::Instant::now();
    let frame = render_studio(&mesh, &colors, &camera, &p, &Look::default(), &env).unwrap();
    eprintln!("COST cpu: {:?} for the studio frame", started.elapsed());
    craft3d_testkit::evidence::print("studio-grid-cpu.png", &frame.to_png().unwrap());
}

/// The viewport's renderer (`Onscreen`, multisampled) in the studio look
/// against the CPU renderer at one sample per pixel, under the same
/// tolerance; the first scenes of [`scenes`] at `ssaa` 1.
#[test]
fn the_viewport_draws_the_studio_look_as_the_cpu_renderer() {
    let env = environment("studio").unwrap();
    for (adapter, device, queue) in devices::devices() {
        let name = adapter.get_info().name;
        let mut onscreen = Onscreen::new(&device, &queue, &adapter).unwrap();
        let lines = onscreen.upload_lines(&[]).unwrap();
        for scene in scenes().into_iter().take(6) {
            let params = RenderParams {
                ssaa: 1,
                ..scene.params
            };
            let studio = onscreen.studio_as_drawn(&Studio {
                env: env.clone(),
                look: scene.look,
            });
            let cpu = render_studio(
                &scene.mesh,
                &scene.colors,
                &scene.camera,
                &params,
                &studio.look,
                &env,
            )
            .unwrap();
            let uploaded = onscreen.upload_colored(&scene.mesh, &scene.colors).unwrap();
            onscreen.set_studio(Some(studio));
            onscreen
                .draw(
                    &uploaded,
                    &lines,
                    &scene.camera,
                    &params,
                    &OverlayParams {
                        slack: 0.0,
                        hidden_opacity: 70,
                        axis_length: 1.0,
                        edges: EdgeLook::default(),
                    },
                    DrawOptions {
                        bodies: true,
                        capture: true,
                    },
                )
                .unwrap();
            let gpu = onscreen.read_frame().unwrap();
            let s = Scene { params, ..scene };
            let stats = compare(&s, &cpu, &gpu);
            eprintln!(
                "onscreen {name} {}: {} samples, {stats:?}",
                s.name,
                onscreen.samples()
            );
            assert_eq!(stats.id_outside, 0, "{name} {}", s.name);
            assert!(
                stats.over_one as f64 <= SHARE_OVER_ONE * stats.covered.max(1) as f64,
                "{name} {}: {stats:?}",
                s.name
            );
            assert!(stats.most <= MOST_LEVELS, "{name} {}: {stats:?}", s.name);
        }
    }
}
