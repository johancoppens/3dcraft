//! Oracle 7 (docs/testing.md): the GPU renderer against the CPU reference
//! renderer, on every adapter of this machine. The tolerances and their
//! derivation are in `common/mod.rs`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{
    compare, compare_see_through, compare_with, evidence, frame_difference, offscreens,
    orthographic, params, perspective, write_evidence,
};
use craft3d_gpu::{Camera, FaceColors, Frame, Mesh, NO_FACE, Projection, RenderParams};
use craft3d_math::Vec3;
use craft3d_tess::{TessParams, tessellate_body, tessellate_patches};
use craft3d_testkit::{solids, teapot::teapot, trimmed};

struct Scene {
    name: String,
    mesh: Mesh,
    camera: Camera,
    params: RenderParams,
    /// Faces in colours of their own (a body's appearance).
    colors: FaceColors,
    /// Leave pictures for the developer.
    evidence: bool,
}

/// Renders every scene with both renderers on every adapter and checks
/// parity. Returns the number of comparisons.
fn check(scenes: &[Scene]) -> usize {
    let mut compared = 0;
    let mut log = String::new();
    for mut offscreen in offscreens() {
        let adapter = offscreen.adapter().clone();
        for scene in scenes {
            let cpu = craft3d_render::render_colored(
                &scene.mesh,
                &scene.colors,
                &scene.camera,
                &scene.params,
            )
            .unwrap();
            let gpu = offscreen
                .render_colored(&scene.mesh, &scene.colors, &scene.camera, &scene.params)
                .unwrap_or_else(|e| panic!("{} on {adapter}: {e}", scene.name));
            let (parity, band) = compare(&scene.mesh, &scene.camera, &cpu, &gpu);
            let label = format!("{} on {adapter}", scene.name);
            if scene.evidence {
                let file = format!(
                    "{}-{}-{}.png",
                    scene.name,
                    adapter.backend.to_lowercase(),
                    adapter.device_type.to_lowercase()
                );
                let path = write_evidence(&file, &cpu, &gpu, &band);
                println!("evidence: {}", path.display());
            }
            log.push_str(&format!("{label}: {parity:?}\n"));
            println!("{label}: {parity:?}");
            parity.check(&label);
            compared += 1;
        }
    }
    // What was measured, next to the pictures.
    if let Some(first) = scenes.first() {
        let group = first.name.split('-').next().unwrap_or("scenes");
        std::fs::write(evidence(&format!("parity-{group}.txt")), log).unwrap();
    }
    compared
}

fn teapot_mesh() -> Mesh {
    tessellate_patches(&teapot().unwrap(), &TessParams::default()).unwrap()
}

/// The usual view of the teapot: from the front-right and above.
const TEAPOT_VIEW: Vec3 = Vec3::new(0.35, -1.0, 0.45);

#[test]
fn the_teapot_matches_the_cpu_renderer() {
    let mesh = teapot_mesh();
    let mut scenes = Vec::new();
    // Sizes with and without row padding in the readback, every
    // supersampling factor, both projections.
    for (name, size, perspective_fov, toward_eye, show) in [
        (
            "teapot-perspective",
            (640, 400, 2),
            Some(30.0),
            TEAPOT_VIEW,
            true,
        ),
        (
            "teapot-large",
            (1280, 800, 1),
            Some(30.0),
            TEAPOT_VIEW,
            false,
        ),
        (
            "teapot-orthographic",
            (320, 240, 1),
            None,
            TEAPOT_VIEW,
            true,
        ),
        (
            "teapot-wide-angle",
            (333, 77, 3),
            Some(70.0),
            Vec3::new(-1.0, 0.4, 0.2),
            false,
        ),
        (
            "teapot-small",
            (64, 64, 4),
            None,
            Vec3::new(0.2, 0.1, 1.0),
            false,
        ),
        (
            "teapot-from-below",
            (200, 300, 2),
            Some(45.0),
            Vec3::new(-0.5, -0.5, -1.0),
            false,
        ),
    ] {
        let params = params(size.0, size.1, size.2);
        let camera = match perspective_fov {
            Some(fov) => perspective(&mesh, toward_eye, fov, &params),
            None => orthographic(&mesh, toward_eye, &params),
        };
        scenes.push(Scene {
            name: name.to_string(),
            mesh: mesh.clone(),
            camera,
            params,
            colors: FaceColors::new(),
            evidence: show,
        });
    }
    assert_eq!(check(&scenes), scenes.len() * offscreens().len());
}

/// Every reference solid of the test kit, tilted, at three scales and once
/// ten million units away from the origin (where `f32` coordinates would be
/// a unit apart: the renderer must work relative to the model), with the
/// projection, the image size and the supersampling varied along the way.
fn solid_scenes() -> Vec<Scene> {
    let mut scenes = Vec::new();
    let near = Vec3::new(3.0, -2.0, 5.0);
    let far_away = Vec3::new(1e7, -2e7, 3e7);
    for (round, (scale, origin)) in [
        (1.0, near),
        (1e-3, near * 1e-3),
        (1e6, near * 1e6),
        (1.0, far_away),
    ]
    .into_iter()
    .enumerate()
    {
        let frame = craft3d_math::Frame::new(origin, Vec3::new(0.3, -0.2, 1.0), Vec3::X).unwrap();
        let mut all = solids::all(&frame, scale).unwrap();
        all.extend(trimmed::all(&frame, scale).unwrap());
        all.push(trimmed::pipe_elbow(&frame, 3.0 * scale, 1.0 * scale, 1.9).unwrap());
        all.push(trimmed::sphere_octant(&frame, 2.0 * scale).unwrap());
        for (index, solid) in all.iter().enumerate() {
            // Each solid is seen with a different combination per scale.
            let variant = index + round;
            let tess = TessParams {
                chordal_tolerance: 0.01 * scale,
                ..TessParams::default()
            };
            let mesh = tessellate_body(&solid.body, &tess).unwrap();
            let (width, height) = [(320, 240), (257, 193), (180, 260)][variant % 3];
            let params = params(width, height, [1, 2, 3][(variant / 3) % 3]);
            let toward_eye = [
                Vec3::new(1.0, -1.3, 0.9),
                Vec3::new(-0.7, 0.4, 1.2),
                Vec3::new(0.2, 1.0, -0.6),
            ][(variant / 2) % 3];
            let orthographic_view = variant % 2 == 1;
            let camera = if orthographic_view {
                orthographic(&mesh, toward_eye, &params)
            } else {
                perspective(&mesh, toward_eye, [30.0, 60.0][(variant / 4) % 2], &params)
            };
            scenes.push(Scene {
                name: format!(
                    "solids-{}-{index}-round{round}-scale{scale:e}-{}",
                    solid.name.replace(' ', "_"),
                    if orthographic_view { "ortho" } else { "persp" }
                ),
                mesh,
                camera,
                params,
                colors: FaceColors::new(),
                evidence: round == 0 && [0, 5, 6, 12, 14].contains(&index),
            });
        }
    }
    scenes
}

#[test]
fn the_reference_solids_match_the_cpu_renderer() {
    let scenes = solid_scenes();
    assert_eq!(scenes.len(), 64);
    assert_eq!(check(&scenes), scenes.len() * offscreens().len());
}

/// A camera close to a surface or inside the model (docs/design/
/// rendering.md › *Near-plane clipping*): triangles that cross the near
/// plane are clipped by both renderers. Before the CPU renderer clipped,
/// it left out every triangle with a corner behind the eye, and showed
/// what was behind those triangles instead.
fn close_scenes() -> Vec<Scene> {
    let world = craft3d_math::Frame::new(Vec3::ZERO, Vec3::Z, Vec3::X).unwrap();
    let tess = TessParams {
        chordal_tolerance: 0.002,
        ..TessParams::default()
    };
    let sphere = tessellate_body(&solids::sphere(&world, 1.0).unwrap().body, &tess).unwrap();
    let block =
        tessellate_body(&solids::block(&world, 2.0, 2.0, 2.0).unwrap().body, &tess).unwrap();
    let cameras = [
        // Skimming the sphere just above its pole, looking along it.
        (
            "skim",
            &sphere,
            Vec3::new(0.0, 0.0, 1.02),
            Vec3::new(1.0, 0.1, 0.85),
            60.0,
        ),
        // Inside the sphere, looking at its wall.
        (
            "inside",
            &sphere,
            Vec3::new(0.1, 0.2, 0.0),
            Vec3::new(1.0, 0.3, 0.2),
            70.0,
        ),
        // Inside a block, near a corner.
        (
            "corner",
            &block,
            Vec3::new(0.3, 0.4, 0.2),
            Vec3::new(1.5, 1.2, 1.7),
            80.0,
        ),
    ];
    cameras
        .into_iter()
        .enumerate()
        .map(|(k, (name, mesh, eye, target, fov))| {
            let params = params([240, 201, 160][k], [180, 151, 200][k], [1, 2, 3][k]);
            Scene {
                name: format!("close-{name}"),
                mesh: mesh.clone(),
                camera: Camera {
                    eye,
                    target,
                    up: Vec3::Z,
                    projection: Projection::Perspective { fov_y_deg: fov },
                },
                params,
                colors: FaceColors::new(),
                evidence: true,
            }
        })
        .collect()
}

#[test]
fn a_camera_close_to_or_inside_the_model_matches_the_cpu_renderer() {
    let scenes = close_scenes();
    assert_eq!(check(&scenes), scenes.len() * offscreens().len());
}

/// The first round of the reference solids with faces highlighted: every
/// other face, and in the next scene one run of three, so that the runs of
/// the uniform block are all used and are used in pairs.
fn highlighted_scenes() -> Vec<Scene> {
    solid_scenes()
        .into_iter()
        .take(16)
        .enumerate()
        .map(|(index, mut scene)| {
            let faces = scene
                .mesh
                .face_ids
                .iter()
                .copied()
                .max()
                .map_or(0, |m| m + 1);
            let chosen: Vec<u32> = if index % 2 == 0 {
                (0..faces).step_by(2).collect()
            } else {
                (faces / 2..faces).take(3).collect()
            };
            let (highlight, _) = craft3d_gpu::Highlight::new([255, 150, 40], chosen);
            assert!(!highlight.is_empty(), "{}", scene.name);
            scene.params.highlight = highlight;
            scene.name = scene.name.replacen("solids-", "highlight-", 1);
            scene.evidence = [0, 5, 6, 12].contains(&index);
            scene
        })
        .collect()
}

#[test]
fn highlighted_faces_match_the_cpu_renderer() {
    let scenes = highlighted_scenes();
    assert_eq!(scenes.len(), 16);
    // The highlight is in the picture at all: the CPU frame has pixels of
    // the highlight's hue (red well above blue) that the plain one lacks.
    // (Three faces in a row may all be on the far side; every other face
    // never is.)
    let mut shown = 0;
    for (index, scene) in scenes.iter().enumerate() {
        let orange = |params: &RenderParams| {
            craft3d_render::render(&scene.mesh, &scene.camera, params)
                .unwrap()
                .rgb
                .chunks(3)
                .filter(|p| i32::from(p[0]) - i32::from(p[2]) > 60)
                .count()
        };
        let plain = RenderParams {
            highlight: craft3d_gpu::Highlight::NONE,
            ..scene.params
        };
        assert_eq!(orange(&plain), 0, "{}", scene.name);
        let marked = orange(&scene.params);
        assert!(marked > 20 || index % 2 == 1, "{}", scene.name);
        shown += usize::from(marked > 20);
    }
    assert!(shown >= 12, "{shown} of 16 scenes show their highlight");
    assert_eq!(check(&scenes), scenes.len() * offscreens().len());
}

/// A face id the shader cannot compare exactly is refused, not drawn wrong.
#[test]
fn a_highlight_above_the_ids_a_float_holds_is_refused() {
    let mesh = teapot_mesh();
    let params = params(64, 48, 1);
    let camera = perspective(&mesh, Vec3::new(1.0, -1.0, 0.8), 30.0, &params);
    for mut offscreen in offscreens() {
        let (highlight, _) = craft3d_gpu::Highlight::new([255, 0, 0], [(1 << 24) - 1]);
        let fine = RenderParams {
            highlight,
            ..params
        };
        offscreen.render(&mesh, &camera, &fine).unwrap();
        let (highlight, _) = craft3d_gpu::Highlight::new([255, 0, 0], [3, 1 << 24]);
        let refused = RenderParams {
            highlight,
            ..params
        };
        let error = offscreen.render(&mesh, &camera, &refused).unwrap_err();
        assert!(error.to_string().contains("highlighted faces"), "{error}");
        // And the renderer goes on.
        offscreen.render(&mesh, &camera, &params).unwrap();
    }
}

/// [`check`] for see-through scenes: the CPU frame once per scene, the
/// comparison of `common`'s *See-through pictures*. Returns the number of
/// comparisons.
fn check_see_through(scenes: &[Scene]) -> usize {
    let mut compared = 0;
    let mut log = String::new();
    let mut renderers = offscreens();
    for scene in scenes {
        assert!(scene.params.opacity() < 1.0, "{}", scene.name);
        let cpu = craft3d_render::render_colored(
            &scene.mesh,
            &scene.colors,
            &scene.camera,
            &scene.params,
        )
        .unwrap();
        // The scene is one where the opacity shows: the picture is not the
        // opaque one. (Highlighted faces are not seen through, and they
        // may be all that faces the eye.)
        let opaque = RenderParams {
            opacity: 1.0,
            ..scene.params
        };
        let front =
            craft3d_render::render_colored(&scene.mesh, &scene.colors, &scene.camera, &opaque)
                .unwrap();
        let changed = cpu
            .rgb
            .chunks(3)
            .zip(front.rgb.chunks(3))
            .filter(|(a, b)| a != b)
            .count();
        assert!(
            changed > cpu.ids.len() / 100 || !scene.params.highlight.is_empty(),
            "{}: only {changed} pixels are seen through",
            scene.name
        );
        for offscreen in &mut renderers {
            let adapter = offscreen.adapter().clone();
            let gpu = offscreen
                .render_colored(&scene.mesh, &scene.colors, &scene.camera, &scene.params)
                .unwrap_or_else(|e| panic!("{} on {adapter}: {e}", scene.name));
            let (parity, band) =
                compare_see_through(&scene.mesh, &scene.camera, &scene.params, &cpu, &gpu);
            let label = format!("{} on {adapter}", scene.name);
            if scene.evidence {
                let file = format!(
                    "{}-{}-{}.png",
                    scene.name,
                    adapter.backend.to_lowercase(),
                    adapter.device_type.to_lowercase()
                );
                let path = write_evidence(&file, &cpu, &gpu, &band);
                println!("evidence: {}", path.display());
            }
            log.push_str(&format!("{label}: {parity:?}\n"));
            println!("{label}: {parity:?}");
            parity.check(&label);
            compared += 1;
        }
    }
    if let Some(first) = scenes.first() {
        let group = first.name.split('-').next().unwrap_or("scenes");
        std::fs::write(evidence(&format!("parity-{group}.txt")), log).unwrap();
    }
    compared
}

/// The opacities of the see-through scenes: nearly clear, half, nearly
/// opaque.
const OPACITIES: [f64; 3] = [0.1, 0.5, 0.9];

/// Every other face, or one run of three, highlighted (as
/// [`highlighted_scenes`]).
fn highlight_some(scene: &mut Scene, every_other: bool) {
    let faces = scene
        .mesh
        .face_ids
        .iter()
        .copied()
        .max()
        .map_or(0, |m| m + 1);
    let chosen: Vec<u32> = if every_other {
        (0..faces).step_by(2).collect()
    } else {
        (faces / 2..faces).take(3).collect()
    };
    scene.params.highlight = craft3d_gpu::Highlight::new([255, 150, 40], chosen).0;
}

/// The reference solids seen through: the first round at every opacity,
/// the other rounds (a thousandth, a million times, ten million units
/// away) at one each, every third scene with faces highlighted.
#[test]
fn see_through_reference_solids_match_the_cpu_renderer() {
    let mut scenes = Vec::new();
    for (index, scene) in solid_scenes().into_iter().enumerate() {
        let opacities: &[f64] = if index < 16 {
            &OPACITIES
        } else {
            &[OPACITIES[index % 3]]
        };
        for &opacity in opacities {
            let mut seen = Scene {
                name: scene
                    .name
                    .replacen("solids-", &format!("seethrough{opacity}-"), 1),
                mesh: scene.mesh.clone(),
                camera: scene.camera,
                params: RenderParams {
                    opacity,
                    ..scene.params
                },
                colors: scene.colors.clone(),
                evidence: opacity == 0.5 && [0, 5, 6, 12, 14].contains(&index),
            };
            if scenes.len() % 3 == 2 {
                highlight_some(&mut seen, scenes.len() % 2 == 0);
                seen.name.push_str("-highlight");
            }
            scenes.push(seen);
        }
    }
    assert_eq!(scenes.len(), 96);
    assert_eq!(
        check_see_through(&scenes),
        scenes.len() * offscreens().len()
    );
}

fn merged(parts: &[Mesh]) -> Mesh {
    let mut out = Mesh::default();
    let mut first_face = 0;
    for part in parts {
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

/// A block with a cavity (a second block inside it) and a sphere and a
/// cone in the cavity: four layers along most rays.
fn nested_mesh() -> Mesh {
    let frame = |x, y, z| craft3d_math::Frame::new(Vec3::new(x, y, z), Vec3::Z, Vec3::X).unwrap();
    let tess = TessParams {
        chordal_tolerance: 0.01,
        ..TessParams::default()
    };
    let mesh =
        |solid: &craft3d_testkit::solids::Reference| tessellate_body(&solid.body, &tess).unwrap();
    merged(&[
        mesh(&solids::block(&frame(-4.0, -3.0, -2.5), 8.0, 6.0, 5.0).unwrap()),
        mesh(&solids::block(&frame(-3.5, -2.5, -2.0), 7.0, 5.0, 4.0).unwrap()),
        mesh(&solids::sphere(&frame(-1.2, 0.3, 0.0), 1.4).unwrap()),
        mesh(&solids::cone(&frame(1.6, -0.8, -1.5), 1.0, 2.6).unwrap()),
    ])
}

/// The teapot, a plate with a hole drilled through it and a body inside a
/// hollow body, seen through at every opacity, plain and with a highlight.
#[test]
fn see_through_models_match_the_cpu_renderer() {
    let frame = craft3d_math::Frame::new(Vec3::ZERO, Vec3::new(0.2, 0.1, 1.0), Vec3::X).unwrap();
    let tess = TessParams {
        chordal_tolerance: 0.005,
        ..TessParams::default()
    };
    let plate = tessellate_body(
        &trimmed::plate_with_round_hole(&frame, 6.0, 4.0, 1.0, 1.2)
            .unwrap()
            .body,
        &tess,
    )
    .unwrap();
    let models = [
        (
            "teapot",
            teapot_mesh(),
            TEAPOT_VIEW,
            Vec3::new(-1.0, 0.4, 0.2),
        ),
        (
            "plate",
            plate,
            Vec3::new(0.9, -1.4, 1.0),
            Vec3::new(-0.3, 0.2, -1.0),
        ),
        (
            "nested",
            nested_mesh(),
            Vec3::new(1.0, -1.6, 0.8),
            Vec3::new(-1.0, 0.2, 0.1),
        ),
    ];
    let mut scenes = Vec::new();
    for (name, mesh, first_view, second_view) in &models {
        for (k, &opacity) in OPACITIES.iter().enumerate() {
            for highlighted in [false, true] {
                // The views, sizes and projections alternate.
                let variant = k + usize::from(highlighted);
                let params = RenderParams {
                    opacity,
                    ..[
                        params(400, 300, 1),
                        params(320, 240, 2),
                        params(233, 177, 3),
                    ][variant % 3]
                };
                let toward_eye = if variant % 2 == 0 {
                    *first_view
                } else {
                    *second_view
                };
                let camera = if variant % 2 == 0 {
                    perspective(mesh, toward_eye, 35.0, &params)
                } else {
                    orthographic(mesh, toward_eye, &params)
                };
                let mut scene = Scene {
                    name: format!(
                        "seethroughmodels-{name}-{opacity}{}",
                        if highlighted { "-highlight" } else { "" }
                    ),
                    mesh: mesh.clone(),
                    camera,
                    params,
                    colors: FaceColors::new(),
                    evidence: opacity == 0.5,
                };
                if highlighted {
                    highlight_some(&mut scene, true);
                }
                scenes.push(scene);
            }
        }
    }
    assert_eq!(scenes.len(), 18);
    assert_eq!(
        check_see_through(&scenes),
        scenes.len() * offscreens().len()
    );
}

/// At opacity 1, and at an opacity that is not a number or is above 1, the
/// picture is the opaque one, bit for bit, also right after a see-through
/// picture on the same renderer.
#[test]
fn at_opacity_one_the_frame_is_the_opaque_one_bit_for_bit() {
    let mesh = nested_mesh();
    for ssaa in [1, 2] {
        let plain = params(200, 150, ssaa);
        let camera = perspective(&mesh, Vec3::new(1.0, -1.6, 0.8), 35.0, &plain);
        for mut offscreen in offscreens() {
            let opaque = offscreen.render(&mesh, &camera, &plain).unwrap();
            let clear = RenderParams {
                opacity: 0.3,
                ..plain
            };
            assert_ne!(offscreen.render(&mesh, &camera, &clear).unwrap(), opaque);
            for opacity in [1.0, 7.0, f64::NAN, f64::INFINITY] {
                let same = RenderParams { opacity, ..plain };
                assert_eq!(
                    offscreen.render(&mesh, &camera, &same).unwrap(),
                    opaque,
                    "opacity {opacity} on {}",
                    offscreen.adapter()
                );
            }
            // Below 0 is 0: the layer behind alone.
            let [below, zero] = [-3.0, 0.0].map(|opacity| {
                let params = RenderParams { opacity, ..plain };
                offscreen.render(&mesh, &camera, &params).unwrap()
            });
            assert_eq!(below, zero);
            assert_ne!(zero, opaque);
        }
    }
}

fn quad(mesh: &mut Mesh, corners: [Vec3; 4], normal: Vec3, face: u32) {
    let base = mesh.positions.len() as u32;
    mesh.positions.extend(corners);
    mesh.normals.extend([normal; 4]);
    mesh.triangles.push([base, base + 1, base + 2]);
    mesh.triangles.push([base, base + 2, base + 3]);
    mesh.face_ids.extend([face, face]);
}

/// Hand-made meshes for what the solids do not have: sheets that pass
/// through each other, vertices without a normal, coincident faces, face
/// ids that use all 32 bits, and one triangle much larger than the view.
#[test]
fn awkward_meshes_match_the_cpu_renderer() {
    let mut crossing = Mesh::default();
    quad(
        &mut crossing,
        [
            Vec3::new(-1.0, -1.0, -0.6),
            Vec3::new(1.0, -1.0, -0.6),
            Vec3::new(1.0, 1.0, 0.6),
            Vec3::new(-1.0, 1.0, 0.6),
        ],
        Vec3::ZERO,
        0,
    );
    quad(
        &mut crossing,
        [
            Vec3::new(-1.0, -1.0, 0.5),
            Vec3::new(1.0, -1.0, 0.5),
            Vec3::new(1.0, 1.0, -0.7),
            Vec3::new(-1.0, 1.0, -0.7),
        ],
        Vec3::ZERO,
        u32::MAX - 1,
    );
    quad(
        &mut crossing,
        [
            Vec3::new(-0.2, -1.3, -1.0),
            Vec3::new(0.3, -1.3, -1.0),
            Vec3::new(0.3, 1.2, 1.0),
            Vec3::new(-0.2, 1.2, 1.0),
        ],
        Vec3::new(1.0, 0.2, 0.1),
        0x8000_0001,
    );

    // The same square twice: the one drawn first must win everywhere.
    let mut coincident = Mesh::default();
    for face in [7, 3] {
        quad(
            &mut coincident,
            [
                Vec3::new(-1.0, -0.8, 0.1),
                Vec3::new(0.9, -1.0, 0.0),
                Vec3::new(1.0, 0.7, 0.3),
                Vec3::new(-0.8, 1.0, -0.2),
            ],
            Vec3::Z,
            face,
        );
    }

    // Far larger than the view: clipped on every side by the GPU.
    let mut huge = Mesh::default();
    quad(
        &mut huge,
        [
            Vec3::new(-40.0, -30.0, 0.0),
            Vec3::new(50.0, -35.0, 2.0),
            Vec3::new(45.0, 38.0, 0.5),
            Vec3::new(-42.0, 33.0, -1.5),
        ],
        Vec3::new(0.1, -0.2, 1.0),
        11,
    );
    quad(
        &mut huge,
        [
            Vec3::new(-0.5, -0.5, 1.0),
            Vec3::new(0.5, -0.5, 1.0),
            Vec3::new(0.5, 0.5, 1.5),
            Vec3::new(-0.5, 0.5, 1.5),
        ],
        Vec3::ZERO,
        12,
    );

    let view = |eye: Vec3, projection| Camera {
        eye,
        target: Vec3::ZERO,
        up: Vec3::Z,
        projection,
    };
    // Straight down, so that no corner of the huge quad is behind the eye
    // plane: there the CPU renderer drops the triangle and the GPU clips it
    // (a documented difference, not what this scene is about).
    let down = |projection| Camera {
        eye: Vec3::new(0.0, 0.0, 6.0),
        target: Vec3::ZERO,
        up: Vec3::new(0.2, 1.0, 0.0),
        projection,
    };
    let scenes = [
        (
            "awkward-crossing-persp",
            &crossing,
            view(
                Vec3::new(2.0, -5.0, 3.0),
                Projection::Perspective { fov_y_deg: 35.0 },
            ),
            params(300, 220, 2),
        ),
        (
            "awkward-crossing-ortho",
            &crossing,
            view(
                Vec3::new(-3.0, -4.0, 2.5),
                Projection::Orthographic { half_height: 1.7 },
            ),
            params(301, 219, 1),
        ),
        (
            "awkward-coincident",
            &coincident,
            view(
                Vec3::new(0.7, -2.0, 4.0),
                Projection::Perspective { fov_y_deg: 40.0 },
            ),
            params(240, 240, 1),
        ),
        (
            "awkward-huge-persp",
            &huge,
            down(Projection::Perspective { fov_y_deg: 50.0 }),
            params(256, 192, 2),
        ),
        (
            "awkward-huge-ortho",
            &huge,
            down(Projection::Orthographic { half_height: 1.5 }),
            params(256, 192, 1),
        ),
    ]
    .map(|(name, mesh, camera, params)| Scene {
        name: name.to_string(),
        mesh: mesh.clone(),
        camera,
        params,
        colors: FaceColors::new(),
        evidence: name == "awkward-crossing-persp",
    });
    check(&scenes);

    // The ids survive untouched, all 32 bits of them, and the first of two
    // coincident faces is the one that shows.
    for mut offscreen in offscreens() {
        let [crossing_scene, _, coincident_scene, ..] = &scenes;
        let frame = offscreen
            .render(
                &crossing_scene.mesh,
                &crossing_scene.camera,
                &crossing_scene.params,
            )
            .unwrap();
        assert_eq!(
            frame.visible_faces().into_iter().collect::<Vec<_>>(),
            [0, 0x8000_0001, u32::MAX - 1]
        );
        let frame = offscreen
            .render(
                &coincident_scene.mesh,
                &coincident_scene.camera,
                &coincident_scene.params,
            )
            .unwrap();
        assert_eq!(frame.visible_faces().into_iter().collect::<Vec<_>>(), [7]);
    }
}

/// Exact expectations that do not need the CPU renderer: the same numbers
/// as `craft3d-render`'s own unit tests assert.
#[test]
fn an_orthographic_square_covers_exactly_its_pixels() {
    let mut mesh = Mesh::default();
    // Off the pixel grid by more than the rasteriser's sub-pixel step, so
    // no sample lies on an edge.
    quad(
        &mut mesh,
        [
            Vec3::new(-0.003, -0.003, 0.0),
            Vec3::new(1.003, -0.003, 0.0),
            Vec3::new(1.003, 1.003, 0.0),
            Vec3::new(-0.003, 1.003, 0.0),
        ],
        Vec3::ZERO,
        7,
    );
    let camera = Camera {
        eye: Vec3::new(0.5, 0.5, 5.0),
        target: Vec3::new(0.5, 0.5, 0.0),
        up: Vec3::Y,
        projection: Projection::Orthographic { half_height: 1.0 },
    };
    for mut offscreen in offscreens() {
        // 2 model units over 100 pixels: the square's edges are at pixel
        // 24.85 and 75.15, so it covers the 50 × 50 pixels whose centres
        // are in columns and rows 25..75.
        let frame = offscreen
            .render(&mesh, &camera, &params(100, 100, 1))
            .unwrap();
        assert_eq!(frame.ids.iter().filter(|&&id| id == 7).count(), 2500);
        assert_eq!(frame.ids[25 * 100 + 25], 7);
        assert_eq!(frame.ids[74 * 100 + 74], 7);
        assert_eq!(frame.ids[24 * 100 + 25], NO_FACE);
        assert_eq!(frame.ids[25 * 100 + 75], NO_FACE);
        assert_eq!(frame.depth[50 * 100 + 50], 5.0);
        assert!(frame.depth[0].is_infinite());
        assert_eq!(&frame.rgb[0..3], &RenderParams::default().background);
        // A corner in +x, +y of the view is top-right in the image.
        let mut corner = Mesh::default();
        quad(
            &mut corner,
            [
                Vec3::new(1.003, 1.003, 0.0),
                Vec3::new(1.397, 1.003, 0.0),
                Vec3::new(1.397, 1.397, 0.0),
                Vec3::new(1.003, 1.397, 0.0),
            ],
            Vec3::ZERO,
            1,
        );
        let frame = offscreen
            .render(&corner, &camera, &params(100, 100, 1))
            .unwrap();
        let hits: Vec<(usize, usize)> = (0..10_000)
            .filter(|&i| frame.ids[i] == 1)
            .map(|i| (i % 100, i / 100))
            .collect();
        assert_eq!(hits.len(), 400);
        assert!(
            hits.iter()
                .all(|&(x, y)| (75..95).contains(&x) && (5..25).contains(&y))
        );
    }
}

fn teapot_frame(offscreen: &mut craft3d_gpu::Offscreen, mesh: &Mesh) -> Frame {
    let params = params(480, 360, 2);
    let camera = perspective(mesh, TEAPOT_VIEW, 30.0, &params);
    offscreen.render(mesh, &camera, &params).unwrap()
}

/// Rule 5: the same input gives the same frame, bit for bit, on the same
/// adapter: again on one device, on a new device, after another picture of
/// the same size (whose contents are then in every target the teapot's
/// passes reuse) and after one of another size. Every comparison runs and
/// the test fails with all that differed; each difference leaves its
/// frames under `target/evidence/gpu/same-frame-*` ([`frame_difference`]).
#[test]
fn the_same_input_gives_the_same_frame() {
    let mesh = teapot_mesh();
    let mut first_run = offscreens();
    let mut second_run = offscreens();
    let mut differences = Vec::new();
    for (index, (a, b)) in first_run.iter_mut().zip(&mut second_run).enumerate() {
        assert_eq!(a.adapter(), b.adapter());
        let frame = teapot_frame(a, &mesh);
        assert!(frame.coverage() > 0.15, "{}", a.adapter());
        let adapter = a.adapter().to_string();
        let mut check = |what: &str, got: Frame| {
            let name = format!("same-frame-{index}-{}", what.replace(' ', "-"));
            if let Some(report) = frame_difference(&name, &frame, &got) {
                differences.push(format!("{what}, {adapter}: {report}"));
            }
        };
        check("same device", teapot_frame(a, &mesh));
        check("new device", teapot_frame(b, &mesh));
        // Another picture of the same size in between: the targets are
        // kept and hold its pixels when the teapot is drawn again.
        let same_size = params(480, 360, 2);
        let camera = orthographic(&mesh, Vec3::new(-0.3, 1.0, -0.6), &same_size);
        let dirty = a.render(&mesh, &camera, &same_size).unwrap();
        assert!(
            dirty.rgb != frame.rgb && dirty.ids != frame.ids,
            "{adapter}"
        );
        check("after another picture", teapot_frame(a, &mesh));
        // And one of another size: the targets are made again.
        let other = params(100, 50, 1);
        let camera = orthographic(&mesh, Vec3::X, &other);
        a.render(&mesh, &camera, &other).unwrap();
        check("after another size", teapot_frame(a, &mesh));
    }
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}

/// The passes over the triangles drawn in two steps (depth first, then
/// the colour of the first fragment at that depth, once per sample;
/// `Step` in `renderer.rs`, the way around the Radeon's lost writes) give
/// the frame of one step, bit for bit: the teapot in perspective and
/// orthographic, opaque, seen through, cut open with a cap and both, and
/// the same square drawn twice, where the one drawn first must stay.
///
/// Three claims per view and adapter:
///
/// 1. The two-step frame, which production draws, matches the CPU
///    renderer (oracle 7) within the parity tolerances of `common`.
/// 2. The one-step frame matches the CPU renderer as well, and the
///    two-step frame is nowhere further from it ([`Parity::worse_than`]).
/// 3. The two frames are the same bit for bit, except where the one-step
///    frame lost a write: on the Radeon 8060S (Mesa, RADV and radeonsi)
///    one of several writes of a sample in one step is now and then lost
///    (about one teapot in 250, more seen through; docs/design/rendering.md,
///    *The Radeon loses one of several writes of a sample*), which is why
///    production draws in two steps. A lost write does not repeat at the
///    same pixel from one frame to the next, so a pixel where the frames
///    differ is the one-step frame's lost write when the one-step frame
///    drawn again ([`ONE_STEP_REDRAWS`] times at most) gives the two-step
///    value there; it is reported, with its evidence, and does not fail.
///    A difference that the one-step frame repeats every time is the
///    two-step frame's: a lost write there, or a tie rule that is not the
///    one step's. It fails. So does a one-step frame that is out of
///    parity with the CPU renderer other than by such lost writes.
#[test]
fn two_steps_give_the_one_step_frame() {
    let teapot = teapot_mesh();
    let mut coincident = Mesh::default();
    for face in [7, 3] {
        quad(
            &mut coincident,
            [
                Vec3::new(-1.0, -0.8, 0.1),
                Vec3::new(0.9, -1.0, 0.0),
                Vec3::new(1.0, 0.7, 0.3),
                Vec3::new(-0.8, 1.0, -0.2),
            ],
            Vec3::ZERO,
            face,
        );
    }
    let size = params(480, 360, 2);
    let see_through = RenderParams {
        opacity: 0.5,
        ..size
    };
    let cut_open = RenderParams {
        section: Some(craft3d_render::Section {
            origin: teapot.bbox().center().unwrap(),
            normal: Vec3::new(0.3, -1.0, 0.2),
            cap: true,
            color: [200, 60, 40],
        }),
        ..size
    };
    let both = RenderParams {
        opacity: 0.5,
        ..cut_open
    };
    let in_perspective = perspective(&teapot, TEAPOT_VIEW, 30.0, &size);
    let square = orthographic(&coincident, Vec3::new(0.2, -0.3, 1.0), &size);
    let views = [
        ("teapot perspective", &teapot, in_perspective, &size),
        (
            "teapot orthographic",
            &teapot,
            orthographic(&teapot, TEAPOT_VIEW, &size),
            &size,
        ),
        ("teapot see-through", &teapot, in_perspective, &see_through),
        ("teapot cut open", &teapot, in_perspective, &cut_open),
        (
            "teapot cut open see-through",
            &teapot,
            in_perspective,
            &both,
        ),
        ("coincident", &coincident, square, &size),
        ("coincident see-through", &coincident, square, &see_through),
    ];
    // The oracle's frames, once per view.
    let cpu: Vec<Frame> = views
        .iter()
        .map(|(_, mesh, camera, params)| craft3d_render::render(mesh, camera, params).unwrap())
        .collect();
    let mut failures = Vec::new();
    let mut two_steps = 0;
    for (index, adapter) in craft3d_gpu::adapters().iter().enumerate() {
        let mut one = craft3d_gpu::Offscreen::on_adapter(adapter).unwrap();
        let mut two = craft3d_gpu::Offscreen::on_adapter(adapter).unwrap();
        let name = one.adapter().to_string();
        one.renderer().in_one_step().unwrap();
        assert!(!one.renderer().in_two_steps());
        if !two.renderer().in_two_steps() {
            println!("{name}: draws in one step only");
            continue;
        }
        two_steps += 1;
        for ((what, mesh, camera, params), cpu) in views.iter().zip(&cpu) {
            let label = format!("{what}, {name}");
            let expected = one.render(mesh, camera, params).unwrap();
            let got = two.render(mesh, camera, params).unwrap();
            assert!(expected.coverage() > 0.05, "{label}");
            if *what == "coincident" {
                assert!(got.ids.iter().all(|&id| id != 3), "{label}");
            }
            // 1. Production's frame against the oracle, strictly.
            let (parity_two, _) = compare_with(mesh, camera, params, cpu, &got);
            println!("{label}, two steps: {parity_two:?}");
            if let Some(failure) = parity_two.failure(&format!("{label}, two steps")) {
                failures.push(failure);
            }
            // 3. Bit for bit, but for the one-step frame's lost writes.
            let evidence = format!("two-steps-{index}-{}", what.replace(' ', "-"));
            let Some(report) = frame_difference(&evidence, &expected, &got) else {
                continue;
            };
            let differs = differing_pixels(&expected, &got);
            let mut kept = differs.clone();
            for _ in 0..ONE_STEP_REDRAWS {
                if kept.is_empty() {
                    break;
                }
                let again = one.render(mesh, camera, params).unwrap();
                kept.retain(|&i| !same_pixel(&again, &got, i));
            }
            let lost = differs.len() - kept.len();
            let same_size = (expected.width, expected.height) == (got.width, got.height)
                && expected.ids.len() == got.ids.len();
            if !kept.is_empty() || !same_size || expected.sample.to_bits() != got.sample.to_bits() {
                failures.push(format!(
                    "{label}: the two-step frame differs from the one-step frame at {} \
                     pixels the one-step frame gives every time it is drawn; {report}",
                    kept.len()
                ));
                continue;
            }
            println!(
                "{label}: the one-step frame lost {lost} writes (the Radeon's lost writes, \
                 docs/design/rendering.md; production draws in two steps); {report}"
            );
            // 2. Where the frames are the same, the one-step frame has the
            // two-step frame's parity. Here they differ only by the
            // one-step frame's lost writes: it is compared with the oracle
            // as drawn, and the two-step frame must be nowhere further
            // from it; that the lost writes take it out of parity is
            // reported, not failed (they are the driver's, as above).
            let (parity_one, _) = compare_with(mesh, camera, params, cpu, &expected);
            println!("{label}, one step: {parity_one:?}");
            let worse = parity_two.worse_than(&parity_one);
            if !worse.is_empty() {
                failures.push(format!(
                    "{label}: the two-step frame is further from the CPU frame than the \
                     one-step frame: {}",
                    worse.join(", ")
                ));
            }
            if let Some(failure) = parity_one.failure(&format!("{label}, one step")) {
                println!(
                    "{label}: the lost writes take the one-step frame out of parity: {failure}"
                );
            }
        }
    }
    println!("{two_steps} adapters draw in two steps");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// How often [`two_steps_give_the_one_step_frame`] draws the one-step
/// frame again where it differs from the two-step frame. A lost write was
/// seen in at most one teapot of about 140 (see-through, on Vulkan), so
/// that it repeats at one pixel in all of three frames drawn again is
/// below one in a million.
const ONE_STEP_REDRAWS: usize = 3;

/// The pixels where two frames of one size differ: colour, face id or the
/// bits of the distance.
fn differing_pixels(a: &Frame, b: &Frame) -> Vec<usize> {
    (0..a.ids.len().min(b.ids.len()))
        .filter(|&i| !same_pixel(a, b, i))
        .collect()
}

/// Whether two frames of one size agree at pixel `i`, bit for bit.
fn same_pixel(a: &Frame, b: &Frame, i: usize) -> bool {
    a.rgb.get(i * 3..i * 3 + 3) == b.rgb.get(i * 3..i * 3 + 3)
        && a.ids.get(i) == b.ids.get(i)
        && a.depth.get(i).map(|d| d.to_bits()) == b.depth.get(i).map(|d| d.to_bits())
}

/// [`the_same_input_gives_the_same_frame`] many times over in one process:
/// the teapot drawn many times by threads on every adapter at once, each
/// frame compared with the thread's first, for a difference too rare for
/// one run of the test to meet (docs/design/rendering.md, *A frame that
/// differed once*). Not part of the gate: `cargo test -p craft3d-gpu
/// --release --test parity -- --ignored --nocapture soak`, or the task
/// `gpu-soak` of the Heavy workflow on the Strix. Set by the environment:
/// `CRAFT3D_GPU_SOAK` frames per thread (default 2000),
/// `CRAFT3D_GPU_SOAK_THREADS` per adapter (default 2: the gate runs the
/// parity tests in parallel on the shared GPU too), and
/// `CRAFT3D_GPU_SOAK_ONE_STEP=1` to draw the opaque scene in one step, as
/// before the Radeon's lost writes were known (`Renderer::in_one_step`),
/// and `CRAFT3D_GPU_SOAK_SCENE` `see-through` (opacity 0.5) or `section`
/// (cut open through the middle, with a cap) for the other passes;
/// `WGPU_ADAPTER_NAME` and `WGPU_BACKEND` choose the adapters.
#[test]
#[ignore = "minutes; on demand and in the Heavy workflow"]
fn soak_the_same_input_gives_the_same_frame() {
    let number = |name: &str, default: usize| -> usize {
        std::env::var(name)
            .ok()
            .and_then(|n| n.parse().ok())
            .unwrap_or(default)
    };
    let frames = number("CRAFT3D_GPU_SOAK", 2000);
    let threads = number("CRAFT3D_GPU_SOAK_THREADS", 2).max(1);
    let one_step = std::env::var("CRAFT3D_GPU_SOAK_ONE_STEP").is_ok_and(|v| v == "1");
    let mesh = teapot_mesh();
    let adapters = offscreens().len();
    // All at once, each thread with a device of its own.
    let differences: Vec<String> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..adapters * threads)
            .map(|worker| {
                let mesh = &mesh;
                scope.spawn(move || soak(worker, worker / threads, frames, one_step, mesh))
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| worker.join().unwrap())
            .collect()
    });
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}

/// The soak's frame: the teapot of [`teapot_frame`], opaque, seen through
/// or cut open (`CRAFT3D_GPU_SOAK_SCENE`).
fn soak_frame(offscreen: &mut craft3d_gpu::Offscreen, mesh: &Mesh) -> Frame {
    let mut params = params(480, 360, 2);
    match std::env::var("CRAFT3D_GPU_SOAK_SCENE").as_deref() {
        Ok("see-through") => params.opacity = 0.5,
        Ok("section") => {
            params.section = Some(craft3d_render::Section {
                origin: mesh.bbox().center().unwrap(),
                normal: Vec3::new(0.3, -1.0, 0.2),
                cap: true,
                color: [200, 60, 40],
            });
        }
        _ => {}
    }
    let camera = perspective(mesh, TEAPOT_VIEW, 30.0, &params);
    offscreen.render(mesh, &camera, &params).unwrap()
}

/// One worker of [`soak_the_same_input_gives_the_same_frame`]: `frames`
/// teapots on a device of its own on adapter `index`, each compared with
/// the first; what differed, with the evidence of the first ten.
fn soak(worker: usize, index: usize, frames: usize, one_step: bool, mesh: &Mesh) -> Vec<String> {
    let adapter = craft3d_gpu::adapters().into_iter().nth(index).unwrap();
    let mut offscreen = craft3d_gpu::Offscreen::on_adapter(&adapter).unwrap();
    if one_step {
        offscreen.renderer().in_one_step().unwrap();
    }
    let scene = std::env::var("CRAFT3D_GPU_SOAK_SCENE").unwrap_or_else(|_| "opaque".to_string());
    let steps = match (scene.as_str(), offscreen.renderer().in_two_steps()) {
        ("see-through" | "section", _) => scene.clone(),
        (_, true) => "opaque, two steps".to_string(),
        (_, false) => "opaque, one step".to_string(),
    };
    let started = std::time::Instant::now();
    let frame = soak_frame(&mut offscreen, mesh);
    let mut differences = Vec::new();
    let mut differing = 0;
    for k in 1..frames {
        let got = soak_frame(&mut offscreen, mesh);
        if got == frame {
            continue;
        }
        differing += 1;
        // The evidence of the first few; the rest are counted.
        if differing <= 10 {
            let name = format!("same-frame-soak-{worker}-{k}");
            let report = frame_difference(&name, &frame, &got).unwrap_or_default();
            differences.push(format!("frame {k}, {}: {report}", offscreen.adapter()));
        }
    }
    if differing > 10 {
        differences.push(format!(
            "{}: {differing} frames differ in all",
            offscreen.adapter()
        ));
    }
    println!(
        "worker {worker}, {} ({steps}): {differing} of {} frames differ from the first ({:.1} s)",
        offscreen.adapter(),
        frames.saturating_sub(1),
        started.elapsed().as_secs_f64()
    );
    differences
}

/// The adapters the tests above ran on, for the test log and the evidence
/// directory. Fails when there is none.
#[test]
fn an_adapter_is_available() {
    let found: Vec<String> = offscreens()
        .iter()
        .map(|o| o.adapter().to_string())
        .collect();
    println!("adapters: {found:#?}");
    std::fs::write(evidence("adapters.txt"), found.join("\n") + "\n").unwrap();
    // The default choice is one of them.
    let default = craft3d_gpu::Offscreen::new().unwrap();
    assert!(found.contains(&default.adapter().to_string()));
}

/// A palette of body colours, light and dark, saturated and grey.
const PALETTE: [[u8; 3]; 6] = [
    [230, 230, 225],
    [40, 40, 45],
    [200, 40, 30],
    [181, 140, 60],
    [60, 120, 200],
    [128, 128, 128],
];

/// The first round of the reference solids with their faces in colours
/// of their own: every face its own in one scene, every other face (the
/// rest in the base colour) in the next, opaque and seen through.
fn colored_scenes() -> Vec<Scene> {
    let mut scenes = Vec::new();
    for (index, mut scene) in solid_scenes().into_iter().take(16).enumerate() {
        let faces = scene
            .mesh
            .face_ids
            .iter()
            .copied()
            .max()
            .map_or(0, |m| m + 1);
        let mut colors = FaceColors::new();
        for face in 0..faces {
            if index % 2 == 0 || face % 2 == 0 {
                assert!(colors.set(face, Some(PALETTE[(face as usize + index) % PALETTE.len()])));
            }
        }
        scene.colors = colors;
        scene.name = scene.name.replacen("solids-", "colored-", 1);
        scene.evidence = [0, 5, 6, 12].contains(&index);
        if index % 4 == 3 {
            highlight_some(&mut scene, false);
            scene.name.push_str("-highlight");
        }
        scenes.push(scene);
    }
    scenes
}

/// What `face.appearance` makes: a body in one colour with one face in a
/// colour of its own (the face's colour before its body's), opaque and
/// seen through, for a few reference solids.
#[test]
fn one_face_in_its_own_colour_on_a_coloured_body_matches_the_cpu_renderer() {
    let mut scenes = Vec::new();
    for (index, mut scene) in solid_scenes().into_iter().take(8).enumerate() {
        let faces = scene
            .mesh
            .face_ids
            .iter()
            .copied()
            .max()
            .map_or(0, |m| m + 1);
        let body = PALETTE[index % PALETTE.len()];
        let own = [0x2f, 0x6f, 0xd6];
        let mut colors = FaceColors::new();
        for face in 0..faces {
            let color = if face == (index as u32) % faces.max(1) {
                own
            } else {
                body
            };
            assert!(colors.set(face, Some(color)));
        }
        scene.colors = colors;
        scene.name = scene.name.replacen("solids-", "face-colour-", 1);
        scene.evidence = index == 0;
        if index % 2 == 1 {
            scene.params.opacity = OPACITIES[index % 3];
            scene.name.push_str("-seethrough");
        }
        scenes.push(scene);
    }
    let (opaque, seen): (Vec<Scene>, Vec<Scene>) = scenes
        .into_iter()
        .partition(|scene| scene.params.opacity >= 1.0);
    assert_eq!(check(&opaque), opaque.len() * offscreens().len());
    assert_eq!(check_see_through(&seen), seen.len() * offscreens().len());
}

/// Metallic and roughness (`Finish`): the reference solids with their
/// faces in finishes from a mirror to all rough, dielectric and metal,
/// opaque and seen through; a face without a finish beside them.
#[test]
fn faces_with_metallic_and_roughness_match_the_cpu_renderer() {
    use craft3d_render::Finish;
    const FINISHES: [(Option<f64>, Option<f64>); 6] = [
        (Some(1.0), Some(0.0)),
        (Some(1.0), Some(0.35)),
        (Some(0.5), Some(0.6)),
        (None, Some(0.1)),
        (None, Some(1.0)),
        (None, None),
    ];
    let mut scenes = Vec::new();
    for (index, mut scene) in solid_scenes().into_iter().take(8).enumerate() {
        let faces = scene
            .mesh
            .face_ids
            .iter()
            .copied()
            .max()
            .map_or(0, |m| m + 1);
        let mut colors = FaceColors::new();
        for face in 0..faces {
            let k = face as usize + index;
            let (metallic, roughness) = FINISHES[k % FINISHES.len()];
            let look = (
                PALETTE[k % PALETTE.len()],
                Finish::from_factors(metallic, roughness),
            );
            assert!(colors.set_look(face, Some(look)));
        }
        // The finishes are in the picture (a solid of one face may have
        // drawn the plain one).
        if (0..faces).any(|face| !colors.finish(face).is_plain()) {
            let mut plain = FaceColors::new();
            for face in 0..faces {
                plain.set(face, colors.get(face));
            }
            let render = |colors: &FaceColors| {
                craft3d_render::render_colored(&scene.mesh, colors, &scene.camera, &scene.params)
                    .unwrap()
                    .rgb
            };
            assert!(render(&plain) != render(&colors), "{}", scene.name);
        }
        scene.colors = colors;
        scene.name = scene.name.replacen("solids-", "finish-", 1);
        scene.evidence = index < 2;
        if index % 2 == 1 {
            scene.params.opacity = OPACITIES[index % 3];
            scene.name.push_str("-seethrough");
        }
        scenes.push(scene);
    }
    let (opaque, seen): (Vec<Scene>, Vec<Scene>) = scenes
        .into_iter()
        .partition(|scene| scene.params.opacity >= 1.0);
    assert_eq!(check(&opaque), opaque.len() * offscreens().len());
    assert_eq!(check_see_through(&seen), seen.len() * offscreens().len());
}

#[test]
fn faces_in_colours_of_their_own_match_the_cpu_renderer() {
    let scenes = colored_scenes();
    // The colours are in the picture: the CPU frame differs from the one
    // without them.
    for scene in &scenes {
        let plain = craft3d_render::render(&scene.mesh, &scene.camera, &scene.params).unwrap();
        let colored = craft3d_render::render_colored(
            &scene.mesh,
            &scene.colors,
            &scene.camera,
            &scene.params,
        )
        .unwrap();
        // (A highlight may cover every face there is.)
        if scene.params.highlight.is_empty() {
            assert_ne!(plain.rgb, colored.rgb, "{}", scene.name);
        }
    }
    assert_eq!(check(&scenes), scenes.len() * offscreens().len());
    let seen: Vec<Scene> = scenes
        .into_iter()
        .enumerate()
        .map(|(index, mut scene)| {
            scene.params.opacity = OPACITIES[index % 3];
            scene.name = scene.name.replacen("colored-", "colored-seethrough-", 1);
            scene
        })
        .collect();
    assert_eq!(check_see_through(&seen), seen.len() * offscreens().len());
}
