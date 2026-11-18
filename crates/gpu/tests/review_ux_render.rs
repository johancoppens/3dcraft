//! The review of the cut's hatching on the GPU (the UX round of 2026-11-18,
//! V1), by `review-ux-render`: cameras chosen to put the cap where the
//! hatching fades into its mean tone (the lines 2 to 3 pixels apart on
//! screen, and under 2) and where the lines at odd multiples are half
//! faded (a zoom between two steps of the spacing), at one to three
//! samples a side, against the CPU renderer within the tolerances of
//! `common/mod.rs`; how many cap pixels differ by how much, hatched and
//! flat; and the model 1e7, 1e10 and 1e12 from the origin against itself
//! at the origin. Findings are printed as `REVIEW` lines.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{Parity, compare_with, offscreens};
use craft3d_gpu::{Camera, Frame, Mesh, Projection, RenderParams, SECTION_FACE, Section};
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::primitives;
use craft3d_math::{Frame as Placement, Portable, Tolerance, Vec3};
use craft3d_render::{Hatch, Hatching};
use craft3d_tess::{TessParams, tessellate_body};

const PAPER: [u8; 3] = [232, 228, 220];
const HATCH: Hatch = Hatch {
    ink: [88, 84, 78],
    spacing_points: 9.0,
    width_points: 1.0,
    height_points: None,
};
const WIDTH: u32 = 256;
const HEIGHT: u32 = 192;

fn at(x: f64, y: f64, z: f64) -> Placement {
    Placement::new(Vec3::new(x, y, z), Vec3::Z, Vec3::X).unwrap()
}

/// A plate 6 × 4 × 1 about the origin with a hole through it.
fn drilled_plate() -> Mesh {
    let plate = primitives::block(&at(-3.0, -2.0, -0.5), 6.0, 4.0, 1.0).unwrap();
    let drill = primitives::cylinder(&at(0.6, 0.3, -2.0), 1.1, 4.0).unwrap();
    let drilled = boolean(&plate, &drill, BooleanOp::Subtract, &Tolerance::default()).unwrap();
    tessellate_body(
        &drilled.body,
        &TessParams {
            chordal_tolerance: 0.004,
            ..TessParams::default()
        },
    )
    .unwrap()
}

/// Across z = 0.1 (moved by `shift`), the half above cut away.
fn cut(shift: Vec3, hatch: Option<Hatch>) -> Section {
    Section {
        origin: Vec3::new(0.0, 0.0, 0.1) + shift,
        normal: Vec3::Z,
        cap: true,
        color: PAPER,
        hatch,
    }
}

fn toward(azimuth: f64, elevation: f64) -> Vec3 {
    let (a, e) = (azimuth.to_radians(), elevation.to_radians());
    Vec3::new(a.pcos() * e.pcos(), a.psin() * e.pcos(), e.psin())
}

const TARGET: Vec3 = Vec3::new(0.2, -0.1, 0.1);

/// A camera on the plate's cut from `elevation` degrees above it, zoomed so
/// that the span the spacing is chosen from is `ratio` times the spacing
/// of a quarter unit (`ratio` 1 to 2: the lines at odd multiples have
/// `Hatching::odd_ink` of `(2 - ratio) / 0.5`, clamped).
fn camera(perspective: bool, ratio: f64, elevation: f64) -> Camera {
    let span = ratio * 0.25;
    // From the azimuth of the lines' normal, (1, 1) / √2: a lower camera
    // brings the lines closer together on screen.
    let direction = toward(45.0, elevation);
    if perspective {
        let tan = 17.5_f64.to_radians().ptan();
        let distance = span * f64::from(HEIGHT) / (18.0 * tan);
        Camera {
            eye: TARGET + direction * distance,
            target: TARGET,
            up: Vec3::Z,
            projection: Projection::Perspective { fov_y_deg: 35.0 },
        }
    } else {
        Camera {
            eye: TARGET + direction * 14.0,
            target: TARGET,
            up: Vec3::Z,
            projection: Projection::Orthographic {
                half_height: span * f64::from(HEIGHT) / 18.0,
            },
        }
    }
}

/// The hatching of `camera` on the cut, and how many pixels apart its
/// lines stand on screen at the target.
fn hatching(camera: &Camera) -> (Hatching, f64) {
    let h = Hatching::new(Vec3::Z, PAPER, &HATCH, camera, HEIGHT).unwrap();
    let distance = (TARGET - camera.eye).dot(h.forward);
    (h, h.across(TARGET, distance))
}

/// The highest elevation (from 85° down, by a quarter of a degree) at
/// which the lines stand at most `across` pixels apart at the target.
fn elevation_for(perspective: bool, ratio: f64, across: f64) -> f64 {
    let mut elevation = 85.0;
    while elevation > 3.0 {
        if hatching(&camera(perspective, ratio, elevation)).1 <= across {
            return elevation;
        }
        elevation -= 0.25;
    }
    panic!("no elevation gives {across} pixels");
}

/// Cap pixels outside the band (both renderers' ids the cap's) by their
/// largest channel difference: 0, 1, 2, 3 or more.
fn cap_differences(cpu: &Frame, gpu: &Frame, band: &[bool]) -> [usize; 4] {
    let mut out = [0; 4];
    for i in 0..cpu.ids.len() {
        if band[i] || cpu.ids[i] != SECTION_FACE || gpu.ids[i] != SECTION_FACE {
            continue;
        }
        let off = (0..3)
            .map(|c| cpu.rgb[i * 3 + c].abs_diff(gpu.rgb[i * 3 + c]))
            .max()
            .unwrap();
        out[usize::from(off).min(3)] += 1;
    }
    out
}

fn summary(parity: &Parity) -> String {
    format!(
        "outside the band: {:?} by level (0, 1, 2, 3+), {} over the tolerance, {:.0} allowed a \
         level; band {} of {}: {} ids, {} colours",
        parity.color_histogram,
        parity.color_outside,
        parity.color_differ_allowed,
        parity.band,
        parity.pixels,
        parity.id_band,
        parity.color_band
    )
}

/// The fade into the mean tone and a half-faded step of the spacing, in
/// perspective and orthographic, at one, two and three samples a side:
/// the GPU's hatched cap against the CPU's within the tolerances of
/// `common/mod.rs`, and the same scene with a flat cap for comparison.
#[test]
fn the_fade_and_a_step_of_the_spacing_match_the_cpu_renderer() {
    let mesh = drilled_plate();
    let mut exercised = (0, 0, 0);
    let mut failures = Vec::new();
    let mut scenes = Vec::new();
    for perspective in [false, true] {
        for ratio in [1.25, 1.75, 1.98] {
            for across in [None, Some(2.5), Some(1.6)] {
                let elevation = across.map_or(70.0, |a| elevation_for(perspective, ratio, a));
                scenes.push((perspective, ratio, elevation));
            }
        }
    }
    for mut offscreen in offscreens() {
        let adapter = offscreen.adapter().clone();
        for (k, &(perspective, ratio, elevation)) in scenes.iter().enumerate() {
            let camera = camera(perspective, ratio, elevation);
            let (h, across) = hatching(&camera);
            if h.odd_ink > 0.2 && h.odd_ink < 0.8 {
                exercised.0 += 1;
            }
            if across > 2.0 && across < 3.0 {
                exercised.1 += 1;
            }
            if across < 2.0 {
                exercised.2 += 1;
            }
            let ssaa = 1 + (k as u32) % 3;
            // The flat cap first: a scene whose flat cap fails is no scene
            // for the hatching (printed, not counted).
            let mut flat_fails = false;
            for hatch in [None, Some(HATCH)] {
                let params = RenderParams {
                    width: WIDTH,
                    height: HEIGHT,
                    ssaa,
                    section: Some(cut(Vec3::ZERO, hatch)),
                    ..RenderParams::default()
                };
                let cpu = craft3d_render::render(&mesh, &camera, &params).unwrap();
                let gpu = offscreen.render(&mesh, &camera, &params).unwrap();
                let (parity, band) = compare_with(&mesh, &camera, &params, &cpu, &gpu);
                let label = format!(
                    "{} ratio {ratio} at {elevation}° (lines {across:.2} px apart, odd ink \
                     {:.2}), ssaa {ssaa}, {} on {adapter}",
                    if perspective { "persp" } else { "ortho" },
                    h.odd_ink,
                    if hatch.is_some() { "hatched" } else { "flat" },
                );
                let caps = cap_differences(&cpu, &gpu, &band);
                eprintln!(
                    "REVIEW parity: {label}: cap pixels outside the band by level {caps:?}; {}",
                    summary(&parity)
                );
                let failure = parity.failure(&label);
                if hatch.is_none() {
                    if caps[1] + caps[2] + caps[3] != 0 {
                        eprintln!("REVIEW parity: {label}: the flat cap is not exact: {caps:?}");
                        flat_fails = true;
                    }
                    if let Some(failure) = failure {
                        eprintln!("REVIEW parity: the flat cap fails, not counted: {failure}");
                        flat_fails = true;
                    }
                } else if let Some(failure) = failure
                    && !flat_fails
                {
                    failures.push(failure);
                }
            }
        }
    }
    eprintln!(
        "REVIEW parity: scenes with the odd lines half faded {}, lines 2 to 3 px apart {}, \
         under 2 px {}",
        exercised.0, exercised.1, exercised.2
    );
    assert!(
        exercised.0 > 0 && exercised.1 > 0 && exercised.2 > 0,
        "{exercised:?}"
    );
    assert!(failures.is_empty(), "{failures:#?}");
}

/// What the hatched cap costs the GPU: the same cut-open picture of the
/// plate at 800 × 600 with two samples a side, flat and hatched in turn,
/// five times each per adapter (the readback included); the medians.
#[test]
fn what_the_hatching_costs_the_gpu() {
    let mesh = drilled_plate();
    let camera = camera(true, 1.5, 60.0);
    for mut offscreen in offscreens() {
        let adapter = offscreen.adapter().clone();
        let params = |hatch: Option<Hatch>| RenderParams {
            width: 800,
            height: 600,
            ssaa: 2,
            section: Some(cut(Vec3::ZERO, hatch)),
            ..RenderParams::default()
        };
        let (flat, hatched) = (params(None), params(Some(HATCH)));
        let _ = offscreen.render(&mesh, &camera, &flat).unwrap();
        let _ = offscreen.render(&mesh, &camera, &hatched).unwrap();
        let mut times = [Vec::new(), Vec::new()];
        for _ in 0..5 {
            for (k, p) in [&flat, &hatched].into_iter().enumerate() {
                let started = std::time::Instant::now();
                let _ = offscreen.render(&mesh, &camera, p).unwrap();
                times[k].push(started.elapsed().as_secs_f64() * 1000.0);
            }
        }
        for t in &mut times {
            t.sort_by(f64::total_cmp);
        }
        eprintln!(
            "REVIEW gpu cost: {adapter}: flat {:.1} ms, hatched {:.1} ms (median of 5)",
            times[0][2], times[1][2]
        );
    }
}

/// The plate moved 1e7, 1e10 and 1e12 from the origin by (D, -D, D / 3),
/// which leaves `w = (x + y) / √2` as it is, so its picture is the one at
/// the origin: the GPU's far picture against the GPU's own at the origin
/// and against the CPU's, on the cap; and the parity of the far scene,
/// hatched and flat. Asserted for 1e7 (the claim), printed beyond.
#[test]
fn far_from_the_origin_the_gpus_hatching_is_the_one_at_the_origin() {
    let mesh = drilled_plate();
    let mut failures = Vec::new();
    for mut offscreen in offscreens() {
        let adapter = offscreen.adapter().clone();
        for (perspective, ratio, elevation, ssaa) in [(false, 1.5, 40.0, 2), (true, 1.75, 30.0, 1)]
        {
            let near_camera = camera(perspective, ratio, elevation);
            let near_params = RenderParams {
                width: WIDTH,
                height: HEIGHT,
                ssaa,
                section: Some(cut(Vec3::ZERO, Some(HATCH))),
                ..RenderParams::default()
            };
            let cpu_near = craft3d_render::render(&mesh, &near_camera, &near_params).unwrap();
            let gpu_near = offscreen.render(&mesh, &near_camera, &near_params).unwrap();
            for d in [1e7, 1e10, 1e12] {
                let shift = Vec3::new(d, -d, d / 3.0);
                let mut far = mesh.clone();
                for p in &mut far.positions {
                    *p = *p + shift;
                }
                let far_camera = Camera {
                    eye: near_camera.eye + shift,
                    target: near_camera.target + shift,
                    ..near_camera
                };
                let label = format!(
                    "{d:e} from the origin, {}, ssaa {ssaa}, on {adapter}",
                    if perspective { "persp" } else { "ortho" }
                );
                let mut parities = Vec::new();
                for hatch in [None, Some(HATCH)] {
                    let params = RenderParams {
                        section: Some(cut(shift, hatch)),
                        ..near_params
                    };
                    let cpu = craft3d_render::render(&far, &far_camera, &params).unwrap();
                    let gpu = offscreen.render(&far, &far_camera, &params).unwrap();
                    let (parity, band) = compare_with(&far, &far_camera, &params, &cpu, &gpu);
                    let caps = cap_differences(&cpu, &gpu, &band);
                    let failure = parity.failure(&label);
                    eprintln!(
                        "REVIEW far parity: {label}, {}: cap by level {caps:?}; {}; {}",
                        if hatch.is_some() { "hatched" } else { "flat" },
                        summary(&parity),
                        failure.as_deref().unwrap_or("passes")
                    );
                    parities.push(failure.is_none());
                    if hatch.is_some() {
                        let no_band = vec![false; gpu.ids.len()];
                        let own = cap_differences(&gpu_near, &gpu, &no_band);
                        let oracle = cap_differences(&cpu_near, &gpu, &no_band);
                        eprintln!(
                            "REVIEW far: {label}: the GPU's cap far against its own at the \
                             origin by level {own:?}, against the CPU's at the origin {oracle:?}"
                        );
                        if d <= 1e7 && own[3] > 2 {
                            failures.push(format!("{label}: against itself {own:?}"));
                        }
                    }
                    // The hatched cap counted where the flat one passes.
                    if d <= 1e7
                        && hatch.is_some()
                        && parities.first() == Some(&true)
                        && let Some(failure) = failure
                    {
                        failures.push(failure);
                    }
                }
                if parities == [true, false] {
                    eprintln!(
                        "REVIEW far: {label}: the hatched cap fails where the flat one passes"
                    );
                }
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
