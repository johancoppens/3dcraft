//! The review of the cut's hatching (the UX round of 2026-11-18, V1), by
//! `review-ux-render`: hostile numbers into `Hatching`, the plane's frame
//! under an orbit and at the changeover of its axis, the same model far
//! from the origin and at other scales against itself at the origin (an
//! oracle that does not use the hatching's own code), the `f32` reduction
//! of the GPU's uniforms emulated with numbers, and what the hatching costs
//! the reference renderer. Findings are printed as `REVIEW` lines.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Portable, Tolerance, Vec3};
use craft3d_render::{
    Camera, Hatch, Hatching, Projection, RenderParams, SECTION_FACE, Section, hatch_distance,
    hatch_frame, render,
};
use craft3d_tess::{Mesh, TessParams, tessellate_body};
use std::time::Instant;

const PAPER: [u8; 3] = [232, 228, 220];
const HATCH: Hatch = Hatch {
    ink: [88, 84, 78],
    spacing_points: 9.0,
    width_points: 1.0,
    height_points: None,
};

fn at(x: f64, y: f64, z: f64) -> Frame {
    Frame::new(Vec3::new(x, y, z), Vec3::Z, Vec3::X).unwrap()
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

/// Across z = 0.1, the half above cut away.
fn cut(hatch: Option<Hatch>) -> Section {
    Section {
        origin: Vec3::new(0.0, 0.0, 0.1),
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

fn camera(target: Vec3, elevation: f64, projection: Projection) -> Camera {
    Camera {
        eye: target + toward(-35.0, elevation) * 14.0,
        target,
        up: Vec3::Z,
        projection,
    }
}

fn hatches() -> Vec<Hatch> {
    let mut out = vec![HATCH];
    for s in [
        0.0,
        -1.0,
        f64::NAN,
        f64::INFINITY,
        f64::MIN_POSITIVE,
        1e-300,
        1e308,
        f64::MAX,
    ] {
        out.push(Hatch {
            spacing_points: s,
            ..HATCH
        });
    }
    for w in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e308, 1e-300] {
        out.push(Hatch {
            width_points: w,
            ..HATCH
        });
    }
    for h in [0.0, -5.0, f64::NAN, f64::INFINITY, 1e-300, 1e300] {
        out.push(Hatch {
            height_points: Some(h),
            ..HATCH
        });
    }
    out
}

fn hostile_cameras() -> Vec<Camera> {
    let ortho = |half_height: f64| Camera {
        eye: Vec3::new(3.0, -4.0, 10.0),
        target: Vec3::ZERO,
        up: Vec3::Z,
        projection: Projection::Orthographic { half_height },
    };
    let persp = |fov_y_deg: f64| Camera {
        projection: Projection::Perspective { fov_y_deg },
        ..ortho(1.0)
    };
    let mut out = Vec::new();
    for half in [
        1.0,
        1e-300,
        1e-12,
        1e12,
        1e300,
        0.0,
        -1.0,
        f64::NAN,
        f64::INFINITY,
    ] {
        out.push(ortho(half));
    }
    for fov in [45.0, 1e-9, 1e-300, 0.0, 179.0, 179.999_999, 180.0, f64::NAN] {
        out.push(persp(fov));
    }
    // At distance 0 from its target, a hair from it, inside a body (the
    // plate's middle), far away, very far, the up nearly along the view.
    out.push(Camera {
        eye: Vec3::ZERO,
        ..persp(45.0)
    });
    out.push(Camera {
        eye: Vec3::new(1e-300, 0.0, 1e-300),
        ..persp(45.0)
    });
    out.push(Camera {
        eye: Vec3::new(0.0, 0.0, 1e-9),
        up: Vec3::Y,
        ..persp(45.0)
    });
    out.push(Camera {
        eye: Vec3::new(1e12, -1e12, 1e12),
        target: Vec3::new(1e12, -1e12, 1e12 - 1.0),
        up: Vec3::Y,
        ..persp(60.0)
    });
    out.push(Camera {
        eye: Vec3::new(0.0, 0.0, 1e308),
        up: Vec3::Y,
        ..persp(60.0)
    });
    out.push(Camera {
        eye: Vec3::new(1e-12, 0.0, 10.0),
        up: Vec3::Z,
        ..persp(60.0)
    });
    out.push(Camera {
        eye: Vec3::new(0.1, 0.2, 0.0),
        target: Vec3::new(3.0, 0.2, 0.0),
        up: Vec3::Z,
        ..persp(90.0)
    });
    out
}

/// NaN, infinity, zero and huge spacings, widths and heights, planes along
/// and nearly along an axis, cameras at distance 0, inside, far, with a
/// field of view of 0 and nearly 180°, pictures 0, 1 and 2³² - 1 pixels
/// high, and points and distances that are not numbers: `Hatching::new`
/// gives a hatching of finite numbers or none, and every coverage, tone
/// and colour of it is a number from 0 to 1. Nothing panics.
#[test]
fn hostile_numbers_never_panic_and_never_give_a_nan() {
    let normals = [
        Vec3::Z,
        -Vec3::X,
        Vec3::new(1e-300, 0.0, 1.0),
        Vec3::new(1.0, 1.0, 1.0).normalized().unwrap(),
        Vec3::new(1.0, 1.0 + 1e-15, 0.3).normalized().unwrap(),
        Vec3::new(1e-17, 1e-17, 1.0),
        Vec3::new(f64::NAN, 0.0, 1.0),
        Vec3::new(f64::INFINITY, 0.0, 0.0),
        Vec3::ZERO,
        Vec3::new(1e300, 1e300, 0.0),
        Vec3::new(1e-310, 0.0, 0.0),
    ];
    let points = [
        Vec3::ZERO,
        Vec3::new(0.3, -0.2, 0.0),
        Vec3::new(1e12, -1e12, 0.0),
        Vec3::new(1e308, 0.0, 0.0),
        Vec3::new(f64::NAN, 0.0, 0.0),
        Vec3::new(f64::INFINITY, 0.0, 0.0),
        Vec3::new(1e-300, 1e-300, 0.0),
    ];
    let distances = [1.0, 0.0, -1.0, 1e-300, 1e300, f64::NAN, f64::INFINITY, 20.0];
    let (mut made, mut none, mut evaluated) = (0_usize, 0_usize, 0_usize);
    for normal in normals {
        for camera in hostile_cameras() {
            for hatch in hatches() {
                for height in [0_u32, 1, 400, u32::MAX] {
                    let Some(h) = Hatching::new(normal, PAPER, &hatch, &camera, height) else {
                        none += 1;
                        continue;
                    };
                    made += 1;
                    let context = format!("{normal:?} {camera:?} {hatch:?} {height}");
                    assert!(
                        h.spacing.is_finite() && h.spacing > 0.0,
                        "{context}: spacing {}",
                        h.spacing
                    );
                    assert!(h.half_width.is_finite(), "{context}: {}", h.half_width);
                    assert!(h.pixel.is_finite() && h.pixel > 0.0, "{context}");
                    assert!((0.0..=1.0).contains(&h.odd_ink), "{context}: {}", h.odd_ink);
                    assert!(h.u.is_finite() && h.v.is_finite(), "{context}");
                    for &p in &points {
                        for &d in &distances {
                            let c = h.coverage(p, d);
                            assert!(
                                (0.0..=1.0).contains(&c),
                                "{context}: coverage {c} at {p:?}, {d}"
                            );
                            let m = h.mean(d);
                            assert!((0.0..=1.0).contains(&m), "{context}: mean {m} at {d}");
                            assert!(!h.across(p, d).is_nan(), "{context}: across at {p:?}, {d}");
                            if let Some(f) = h.footprint(p, d) {
                                assert!(f.is_finite() && f > 0.0, "{context}: footprint {f}");
                            }
                            let color = h.color(p, d);
                            assert_eq!(color, h.color(p, d), "{context}");
                            evaluated += 1;
                        }
                    }
                }
            }
        }
    }
    eprintln!("REVIEW hostile: {made} hatchings made, {none} refused, {evaluated} evaluations");
    assert!(made > 1000 && none > 1000, "{made} {none}");

    // The renderer itself with a hostile hatching, on pictures of 1 × 1
    // and 2 × 1 and with cameras that look nowhere: an answer or an
    // error, never a panic.
    let mesh = drilled_plate();
    for hatch in hatches() {
        for camera in hostile_cameras().into_iter().take(12) {
            for (width, height, ssaa) in [(1, 1, 1), (2, 1, 3), (16, 9, 2)] {
                let params = RenderParams {
                    width,
                    height,
                    ssaa,
                    section: Some(cut(Some(hatch))),
                    ..RenderParams::default()
                };
                let _ = render(&mesh, &camera, &params);
            }
        }
    }
}

/// The plane's frame is a function of its normal alone: an orbit (yaw and
/// pitch turning, slowly, for 720 frames) changes neither the frame nor
/// the spacing, so no line flips between two frames of an orbit. Where the
/// normal turns across a tie of the "most square" axis the frame jumps:
/// the jump is printed (only a change of the plane itself can make it).
#[test]
fn an_orbit_keeps_the_frame_and_only_the_plane_turns_the_lines() {
    let normal = Vec3::new(0.3, -0.8, 0.52).normalized().unwrap();
    let target = Vec3::new(0.4, 0.1, 0.0);
    for projection in [
        Projection::Perspective { fov_y_deg: 40.0 },
        Projection::Orthographic { half_height: 4.0 },
    ] {
        let mut first: Option<Hatching> = None;
        for step in 0..720 {
            let yaw = f64::from(step) * 0.5;
            let pitch = 25.0 + 20.0 * (f64::from(step) * 0.01).psin();
            let camera = Camera {
                eye: target + toward(yaw, pitch) * 20.0,
                target,
                up: Vec3::Z,
                projection,
            };
            let h = Hatching::new(normal, PAPER, &HATCH, &camera, 400).unwrap();
            match first {
                None => first = Some(h),
                Some(f) => {
                    assert_eq!((h.u, h.v), (f.u, f.v), "step {step}");
                    assert_eq!(h.spacing, f.spacing, "step {step}");
                    assert!((h.odd_ink - f.odd_ink).abs() < 1e-9, "step {step}");
                }
            }
        }
    }
    // A normal turning slowly through a tie (|n·Y| = |n·Z| at t = 0.25).
    let direction = |n: Vec3| {
        let (u, v) = hatch_frame(n).unwrap();
        (u + v) * std::f64::consts::FRAC_1_SQRT_2
    };
    let mut worst: (f64, f64) = (0.0, 0.0);
    let mut last: Option<Vec3> = None;
    for k in 0..=5000 {
        let t = f64::from(k) * 1e-4;
        let n = Vec3::new(1.0, t, 0.25).normalized().unwrap();
        let d = direction(n);
        if let Some(previous) = last {
            let turn = previous.angle_to(d).to_degrees();
            if turn > worst.0 {
                worst = (turn, t);
            }
        }
        last = Some(d);
    }
    eprintln!(
        "REVIEW frame: a plane turned by 0.006° per step turns its lines by at most {:.1}° \
         per step, at t = {:.4} (the tie of Y and Z)",
        worst.0, worst.1
    );
}

fn moved(mesh: &Mesh, scale: f64, shift: Vec3) -> Mesh {
    let mut out = mesh.clone();
    for p in &mut out.positions {
        *p = *p * scale + shift;
    }
    out
}

/// The same picture of the model scaled by a power of two and moved by
/// (D, -D, D / 3) against the model at the origin: for the plane z = 0.1
/// the frame is X and Y, `w = (x + y) / √2` does not change under the move,
/// and a power of two scales the spacing and the pixel alike, so in exact
/// arithmetic the two pictures are the same. Compared where both are the
/// cap: how many pixels differ, by how much. Asserted within the claim
/// (scales 2⁻¹⁰ to 2²⁰, 1e7 from the origin); printed beyond it.
#[test]
fn far_from_the_origin_and_at_other_scales_the_cap_is_the_same_picture() {
    let mesh = drilled_plate();
    let target = Vec3::new(0.2, -0.1, 0.1);
    let mut failures = Vec::new();
    for (scale, d) in [
        (1.0, 1e3),
        (1.0, 1e7),
        (1.0, 1e10),
        (1.0, 1e12),
        (1.0 / 1024.0, 0.0),
        (1_048_576.0, 0.0),
        (1.0 / 1024.0, 1e4),
        (1_048_576.0, 1e12),
    ] {
        let shift = Vec3::new(d, -d, d / 3.0);
        let far = moved(&mesh, scale, shift);
        for (name, projection, far_projection) in [
            (
                "ortho",
                Projection::Orthographic { half_height: 3.6 },
                Projection::Orthographic {
                    half_height: 3.6 * scale,
                },
            ),
            (
                "persp",
                Projection::Perspective { fov_y_deg: 35.0 },
                Projection::Perspective { fov_y_deg: 35.0 },
            ),
        ] {
            for (elevation, ssaa) in [(70.0, 1), (24.0, 2), (12.0, 3)] {
                let near_camera = camera(target, elevation, projection);
                let far_camera = Camera {
                    eye: near_camera.eye * scale + shift,
                    target: near_camera.target * scale + shift,
                    up: Vec3::Z,
                    projection: far_projection,
                };
                let params = RenderParams {
                    width: 240,
                    height: 180,
                    ssaa,
                    section: Some(cut(Some(HATCH))),
                    ..RenderParams::default()
                };
                let far_params = RenderParams {
                    section: Some(Section {
                        origin: Vec3::new(0.0, 0.0, 0.1) * scale + shift,
                        ..cut(Some(HATCH))
                    }),
                    ..params
                };
                let a = render(&mesh, &near_camera, &params).unwrap();
                let b = render(&far, &far_camera, &far_params).unwrap();
                let (mut cap, mut ids, mut worst, mut over) = (0, 0, 0_u8, 0);
                for i in 0..a.ids.len() {
                    if a.ids[i] != b.ids[i] {
                        ids += 1;
                        continue;
                    }
                    if a.ids[i] != SECTION_FACE {
                        continue;
                    }
                    cap += 1;
                    let off = (0..3)
                        .map(|c| a.rgb[i * 3 + c].abs_diff(b.rgb[i * 3 + c]))
                        .max()
                        .unwrap();
                    worst = worst.max(off);
                    over += usize::from(off > 2);
                }
                let label = format!(
                    "scale {scale}, {d:e} from the origin, {name}, {elevation}°, ssaa {ssaa}"
                );
                eprintln!(
                    "REVIEW far: {label}: {cap} cap pixels, {over} differ by more than 2 \
                     levels (most {worst}), {ids} pixels of another id"
                );
                assert!(cap > 500, "{label}: {cap} cap pixels");
                let claimed = d <= 1e7 && (1.0 / 1024.0..=1_048_576.0).contains(&scale);
                if claimed && (over > 0 || ids > a.ids.len() / 500) {
                    failures.push(format!("{label}: {over} over 2 levels, {ids} ids"));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// The GPU's reduction (`craft3d_gpu::cut::hatch_uniforms` and
/// `scene.wgsl`'s `section_hatch_color`) emulated with `f32` numbers: `u`
/// and `v` relative to the mesh origin, less a whole number of `2√2`
/// spacings near the target, then the distance to the nearest even and
/// odd line in `f32`, against the CPU's in `f64`, in pixels, over points of
/// the plane within the view. The claim: a model 1e7 from the origin
/// hatches as one at it.
#[test]
fn the_gpus_reduction_before_f32_holds_with_numbers() {
    for d in [0.0, 1e4, 1e7, 1e10, 1e12] {
        let shift = Vec3::new(d * 0.6, -d, d * 0.37);
        let normal = Vec3::new(0.3, -0.8, 0.52).normalized().unwrap();
        let target = shift + Vec3::new(0.2, -0.1, 0.1);
        let origin = shift + Vec3::new(0.05, 0.02, -0.03);
        let camera = Camera {
            eye: target + toward(-35.0, 40.0) * 14.0,
            target,
            up: Vec3::Z,
            projection: Projection::Perspective { fov_y_deg: 35.0 },
        };
        let h = Hatching::new(normal, PAPER, &HATCH, &camera, 400).unwrap();
        let step = 2.0 * h.spacing * std::f64::consts::SQRT_2;
        let reduce = |axis: Vec3| {
            let whole = step * (target.dot(axis) / step).round();
            (origin.dot(axis) - if whole.is_finite() { whole } else { 0.0 }) as f32
        };
        let (uw, vw) = (reduce(h.u), reduce(h.v));
        let f3 = |v: Vec3| [v.x as f32, v.y as f32, v.z as f32];
        let dot3 = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let (uu, vv) = (f3(h.u), f3(h.v));
        let eye = f3(camera.eye - origin);
        let spacing = h.spacing as f32;
        let two = 2.0 * spacing;
        let mut worst = 0.0_f64;
        // Points of the plane through the target, within a few units of it.
        for i in -20..=20 {
            for j in -20..=20 {
                let p = target + h.u * (f64::from(i) * 0.137) + h.v * (f64::from(j) * 0.113);
                let rel_model = f3(p - origin);
                let rel = [
                    rel_model[0] - eye[0],
                    rel_model[1] - eye[1],
                    rel_model[2] - eye[2],
                ];
                let pp = [eye[0] + rel[0], eye[1] + rel[1], eye[2] + rel[2]];
                let u = dot3(pp, uu) + uw;
                let v = dot3(pp, vv) + vw;
                let w = (u + v) * std::f32::consts::FRAC_1_SQRT_2;
                let gpu_even = (w - two * (w / two).round()).abs();
                let gpu_odd = (w - spacing - two * ((w - spacing) / two).round()).abs();
                let w_cpu = (p.dot(h.u) + p.dot(h.v)) * std::f64::consts::FRAC_1_SQRT_2;
                let cpu_even = hatch_distance(w_cpu, 2.0 * h.spacing);
                let cpu_odd = hatch_distance(w_cpu - h.spacing, 2.0 * h.spacing);
                let distance = (p - camera.eye).dot(h.forward);
                let q = h.pixel * distance;
                let off = ((f64::from(gpu_even) - cpu_even).abs())
                    .max((f64::from(gpu_odd) - cpu_odd).abs())
                    / q;
                worst = worst.max(off);
            }
        }
        eprintln!(
            "REVIEW reduction: {d:e} from the origin, spacing {}: the GPU's distance to the \
             nearest line differs from the CPU's by at most {worst:.2e} pixels",
            h.spacing
        );
        if d <= 1e7 {
            assert!(worst < 0.01, "{d:e}: {worst} pixels");
        }
    }
}

/// What the hatching costs the reference renderer: the same picture of the
/// drilled plate cut open, flat paper and hatched, rendered in turn five
/// times each at 800 × 600 with two samples a side (the window's sharp
/// picture); the medians, their ratio and the cost per cap sample. And
/// `Hatching::color` alone, per call.
#[test]
fn what_the_hatching_costs_the_reference_renderer() {
    let mesh = drilled_plate();
    let target = Vec3::new(0.2, -0.1, 0.1);
    let camera = camera(target, 60.0, Projection::Perspective { fov_y_deg: 35.0 });
    let params = |hatch: Option<Hatch>| RenderParams {
        width: 800,
        height: 600,
        ssaa: 2,
        section: Some(cut(hatch)),
        ..RenderParams::default()
    };
    let (flat, hatched) = (params(None), params(Some(HATCH)));
    let _ = render(&mesh, &camera, &flat).unwrap();
    let mut times = [Vec::new(), Vec::new()];
    let mut caps = 0;
    for _ in 0..5 {
        for (k, p) in [&flat, &hatched].into_iter().enumerate() {
            let started = Instant::now();
            let frame = render(&mesh, &camera, p).unwrap();
            times[k].push(started.elapsed().as_secs_f64() * 1000.0);
            caps = frame.ids.iter().filter(|&&id| id == SECTION_FACE).count();
        }
    }
    let median = |v: &mut Vec<f64>| {
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    };
    let (a, b) = (median(&mut times[0]), median(&mut times[1]));
    // Four samples a pixel.
    let per_sample = (b - a) * 1e6 / (caps as f64 * 4.0);
    let h = Hatching::new(Vec3::Z, PAPER, &HATCH, &camera, 600).unwrap();
    let started = Instant::now();
    let mut sum = 0_u64;
    for k in 0..1_000_000 {
        let x = f64::from(k % 1000) * 0.006 - 3.0;
        let y = f64::from(k / 1000) * 0.004 - 2.0;
        let c = h.color(Vec3::new(x, y, 0.1), 14.0);
        sum = sum.wrapping_add(u64::from(c[0]));
    }
    let per_call = started.elapsed().as_secs_f64() * 1e9 / 1e6;
    eprintln!(
        "REVIEW cost: the CPU picture cut open, 800 x 600, ssaa 2, {caps} cap pixels: flat \
         {a:.1} ms, hatched {b:.1} ms (x{:.2}), {per_sample:.1} ns more per cap sample; \
         Hatching::color alone {per_call:.1} ns a call ({sum})",
        b / a
    );
    assert!(b < a * 2.5, "{a} {b}");
}
