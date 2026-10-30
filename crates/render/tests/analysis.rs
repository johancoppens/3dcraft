//! The surface analyses of the CPU renderer against closed forms
//! (`craft3d_render::analysis`; docs/design/surfacing.md, stage 1).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Vec3;
use craft3d_render::analysis::{Analysis, AnalysisColors, AnalysisMode};
use craft3d_render::{Camera, FaceColors, NO_FACE, Projection, RenderParams, render_analysed};
use craft3d_tess::{Mesh, TessParams, tessellate_body};

fn world() -> craft3d_math::Frame {
    craft3d_math::Frame::new(Vec3::ZERO, Vec3::Z, Vec3::X).unwrap()
}

fn mesh_of(solid: craft3d_testkit::solids::Reference, tol: f64) -> Mesh {
    let tess = TessParams {
        chordal_tolerance: tol,
        ..TessParams::default()
    };
    tessellate_body(&solid.body, &tess).unwrap()
}

fn params(w: u32, h: u32, analysis: Analysis) -> RenderParams {
    RenderParams {
        width: w,
        height: h,
        ssaa: 1,
        analysis: Some(analysis),
        ..RenderParams::default()
    }
}

fn ortho(eye: Vec3, up: Vec3, half_height: f64) -> Camera {
    Camera {
        eye,
        target: Vec3::ZERO,
        up,
        projection: Projection::Orthographic { half_height },
    }
}

fn pixel(frame: &craft3d_render::Frame, x: usize, y: usize) -> [u8; 3] {
    let i = 3 * (y * frame.width as usize + x);
    [frame.rgb[i], frame.rgb[i + 1], frame.rgb[i + 2]]
}

/// Zebra stripes on a sphere seen square on, with the tubes across the
/// picture: along the vertical through its middle the reflected ray turns
/// twice as fast as the normal, `φ = −2θ` with `θ = asin y` (the module
/// docs of `analysis`), so the stripes are evenly spaced in the angle on
/// the sphere, `count` periods per half turn of the normal. Every pixel
/// agrees with that closed form except within 0.02 of a period of a
/// stripe's edge (the interpolated normal of a mesh within 1e-3 of the
/// sphere is within about 1e-3 rad of the true one, 0.005 of a period at
/// 16 stripes). The picture is mirror symmetric about that vertical.
#[test]
fn zebra_stripes_on_a_sphere_are_evenly_spaced_and_symmetric() {
    let mesh = mesh_of(
        craft3d_testkit::solids::sphere(&world(), 1.0).unwrap(),
        1e-4,
    );
    let analysis = Analysis::new(AnalysisMode::Zebra);
    let (w, h, half) = (221_u32, 221_u32, 1.1);
    let camera = ortho(Vec3::new(0.0, 0.0, 10.0), Vec3::Y, half);
    let frame = render_analysed(
        &mesh,
        &FaceColors::new(),
        &[],
        &camera,
        &params(w, h, analysis),
    )
    .unwrap();
    let colors = AnalysisColors::default();
    let x = (w / 2) as usize;
    let (mut checked, mut transitions, mut last) = (0, 0, None);
    for y in 0..h as usize {
        if frame.ids[y * w as usize + x] == NO_FACE {
            continue;
        }
        let wy = (1.0 - (y as f64 + 0.5) / f64::from(h) * 2.0) * half;
        if wy.abs() > 0.95 {
            continue;
        }
        let theta = wy.asin();
        let phase = -2.0 * theta / std::f64::consts::TAU * analysis.stripes;
        let f = phase - phase.floor();
        let expect = if f < analysis.width {
            colors.dark
        } else {
            colors.light
        };
        let got = pixel(&frame, x, y);
        if last.is_some_and(|l| l != got) {
            transitions += 1;
        }
        last = Some(got);
        let near_edge = f.min(1.0 - f).min((f - analysis.width).abs()) < 0.02;
        if !near_edge {
            assert_eq!(got, expect, "row {y}: phase {phase}");
            checked += 1;
        }
    }
    assert!(checked > 150, "{checked}");
    // asin 0.95 is 1.25 rad each way: 2 · 2.5 rad / τ · 16 periods, two
    // edges each.
    let expected = (2.0 * 2.0 * 0.95_f64.asin() / std::f64::consts::TAU * 16.0 * 2.0).floor();
    assert!(
        (f64::from(transitions) - expected).abs() <= 2.0,
        "{transitions} edges, {expected} expected"
    );
    // Mirror symmetric about the vertical through the middle.
    let mut differ = 0;
    let mut total = 0;
    for y in 0..h as usize {
        for dx in 1..(w as usize / 2) {
            let (l, r) = (x - dx, x + dx);
            if frame.ids[y * w as usize + l] == NO_FACE || frame.ids[y * w as usize + r] == NO_FACE
            {
                continue;
            }
            total += 1;
            if pixel(&frame, l, y) != pixel(&frame, r, y) {
                differ += 1;
            }
        }
    }
    assert!(differ * 100 <= total, "{differ} of {total} differ");
}

/// Zebra stripes on a cylinder seen square to its axis run straight along
/// it: the normal, so the reflected ray, does not change along a ruling,
/// and the interpolation of a ruling's normals does not either. Every
/// column of the side face is one colour.
#[test]
fn zebra_stripes_on_a_cylinder_run_straight_along_the_axis() {
    let mesh = mesh_of(
        craft3d_testkit::solids::cylinder(&world(), 0.5, 2.0).unwrap(),
        1e-3,
    );
    let mut analysis = Analysis::new(AnalysisMode::Zebra);
    analysis.stripes = 24.0;
    analysis.direction = 0.4;
    let (w, h) = (200_u32, 240_u32);
    let camera = Camera {
        eye: Vec3::new(10.0, 0.0, 1.0),
        target: Vec3::new(0.0, 0.0, 1.0),
        up: Vec3::Z,
        projection: Projection::Orthographic { half_height: 1.2 },
    };
    let frame = render_analysed(
        &mesh,
        &FaceColors::new(),
        &[],
        &camera,
        &params(w, h, analysis),
    )
    .unwrap();
    let (w, h) = (w as usize, h as usize);
    let (mut columns, mut stripes) = (0, std::collections::BTreeSet::new());
    for x in 0..w {
        // The middle rows, well inside the side face.
        let colors: Vec<[u8; 3]> = (h / 4..3 * h / 4)
            .filter(|&y| frame.ids[y * w + x] != NO_FACE)
            .map(|y| pixel(&frame, x, y))
            .collect();
        if colors.len() < h / 2 {
            continue;
        }
        columns += 1;
        assert!(
            colors.iter().all(|&c| c == colors[0]),
            "column {x} is not one colour"
        );
        stripes.insert(colors[0]);
    }
    assert!(columns > 80, "{columns}");
    assert_eq!(stripes.len(), 2, "both stripe colours appear");
}

/// Draft on a cylinder whose axis is the pull: the side is square to the
/// pull (too shallow), the top caps face along it (in), the bottom against
/// it (out).
#[test]
fn draft_classes_the_faces_of_a_cylinder() {
    let mesh = mesh_of(
        craft3d_testkit::solids::cylinder(&world(), 0.5, 2.0).unwrap(),
        1e-3,
    );
    let analysis = Analysis::new(AnalysisMode::Draft);
    let camera = Camera {
        eye: Vec3::new(6.0, -6.0, 6.0),
        target: Vec3::new(0.0, 0.0, 1.0),
        up: Vec3::Z,
        projection: Projection::Orthographic { half_height: 1.6 },
    };
    let p = params(160, 160, analysis);
    let frame = render_analysed(&mesh, &FaceColors::new(), &[], &camera, &p).unwrap();
    let mut seen = std::collections::BTreeMap::new();
    for (i, &id) in frame.ids.iter().enumerate() {
        if id == NO_FACE {
            continue;
        }
        let rgb = [frame.rgb[3 * i], frame.rgb[3 * i + 1], frame.rgb[3 * i + 2]];
        // The hue says the class whatever the lighting: the largest
        // channel of the default colours (green in, blue out, red shallow).
        let top = (0..3).max_by_key(|&k| rgb[k]).unwrap();
        let k = ["shallow", "in", "out"][top];
        seen.entry(id)
            .or_insert_with(std::collections::BTreeSet::new)
            .insert(k);
    }
    // Three faces, each one class.
    assert_eq!(seen.len(), 2, "the bottom is not seen from above: {seen:?}");
    assert!(seen.values().all(|k| k.len() == 1), "{seen:?}");
    let classes: std::collections::BTreeSet<_> = seen.values().flatten().copied().collect();
    assert_eq!(
        classes,
        ["in", "shallow"].into_iter().collect(),
        "the top is in and the side too shallow"
    );
}

/// The curvature map colours by the value per vertex: a constant value is
/// one colour of the ramp (lit by the facing only), and a value that is not
/// a number is the neutral colour.
#[test]
fn a_constant_value_is_one_colour_of_the_ramp() {
    let mesh = mesh_of(
        craft3d_testkit::solids::sphere(&world(), 1.0).unwrap(),
        1e-3,
    );
    let mut analysis = Analysis::new(AnalysisMode::Curvature);
    analysis.range = [0.0, 2.0];
    let camera = ortho(Vec3::new(0.0, 0.0, 10.0), Vec3::Y, 1.1);
    let p = params(101, 101, analysis);
    let ones = vec![1.0_f32; mesh.positions.len()];
    let frame = render_analysed(&mesh, &FaceColors::new(), &ones, &camera, &p).unwrap();
    // Square on at the middle: lit fully.
    assert_eq!(pixel(&frame, 50, 50), analysis.colors.ramp[2]);
    let none = vec![f32::NAN; mesh.positions.len()];
    let frame = render_analysed(&mesh, &FaceColors::new(), &none, &camera, &p).unwrap();
    assert_eq!(pixel(&frame, 50, 50), analysis.colors.none);
}

#[test]
fn analysis_pictures_are_deterministic_and_none_is_the_plain_picture() {
    let mesh = mesh_of(
        craft3d_testkit::solids::torus(&world(), 1.0, 0.3).unwrap(),
        1e-3,
    );
    let camera = Camera {
        eye: Vec3::new(3.0, -4.0, 3.0),
        target: Vec3::ZERO,
        up: Vec3::Z,
        projection: Projection::Perspective { fov_y_deg: 35.0 },
    };
    for mode in [
        AnalysisMode::Zebra,
        AnalysisMode::Highlight,
        AnalysisMode::Curvature,
        AnalysisMode::Draft,
    ] {
        let p = params(120, 90, Analysis::new(mode));
        let values: Vec<f32> = mesh.positions.iter().map(|q| q.z as f32).collect();
        let one = render_analysed(&mesh, &FaceColors::new(), &values, &camera, &p).unwrap();
        let two = render_analysed(&mesh, &FaceColors::new(), &values, &camera, &p).unwrap();
        assert_eq!(one, two, "{mode:?}");
    }
    let plain = RenderParams {
        analysis: None,
        ..params(120, 90, Analysis::new(AnalysisMode::Zebra))
    };
    let values: Vec<f32> = mesh.positions.iter().map(|q| q.z as f32).collect();
    assert_eq!(
        render_analysed(&mesh, &FaceColors::new(), &values, &camera, &plain).unwrap(),
        craft3d_render::render(&mesh, &camera, &plain).unwrap()
    );
}
