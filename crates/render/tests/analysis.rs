//! The surface analyses of the CPU renderer against closed forms
//! (`craft3d_render::analysis`; docs/design/surfacing.md, stage 1).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
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
/// 16 stripes) widened by the pixel's footprint in phase: the stripes are
/// filtered with a tent that wide each way (`analysis::dark_share`), so an
/// edge is a ramp centred where the edge is. The footprint is the closed
/// form's, `dφ/dy = 2 / cos θ` per unit of height (none across, by the
/// symmetry), and on a ramp the grey is the closed form's square wave
/// averaged with the tent over a pixel up and down (sampled finely), within
/// 0.12 (the normal's 0.005 of a period over the least footprint, 0.05). An
/// edge is where the grey
/// crosses the middle. The picture is mirror symmetric about that vertical.
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
    let (mut checked, mut ramps, mut transitions, mut last) = (0, 0, 0, None);
    for y in 0..h as usize {
        if frame.ids[y * w as usize + x] == NO_FACE {
            continue;
        }
        let wy = (1.0 - (y as f64 + 0.5) / f64::from(h) * 2.0) * half;
        if wy.abs() > 0.95 {
            continue;
        }
        let phase_at = |wy: f64| -2.0 * wy.pasin() / std::f64::consts::TAU * analysis.stripes;
        let phase = phase_at(wy);
        let f = phase - phase.floor();
        let expect = if f < analysis.width {
            colors.dark
        } else {
            colors.light
        };
        let got = pixel(&frame, x, y);
        let is_dark = got[0] < 128;
        if last.is_some_and(|l| l != is_dark) {
            transitions += 1;
        }
        last = Some(is_dark);
        // The pixel's height in model units, and its footprint in phase.
        let size = 2.0 * half / f64::from(h);
        let footprint =
            2.0 / (1.0 - wy * wy).sqrt() * size / std::f64::consts::TAU * analysis.stripes;
        let near_edge = f.min(1.0 - f).min((f - analysis.width).abs()) < 0.02 + footprint;
        if near_edge {
            // The closed form's square wave averaged with the tent over a
            // pixel up and down.
            let n = 800;
            let (mut dark, mut total) = (0.0, 0.0);
            for k in 0..n {
                let t = -1.0 + 2.0 * (f64::from(k) + 0.5) / f64::from(n);
                let weight = 1.0 - t.abs();
                let p = phase_at(wy + t * size);
                total += weight;
                if p - p.floor() < analysis.width {
                    dark += weight;
                }
            }
            let dark = dark / total;
            let share = (f64::from(colors.light[0]) - f64::from(got[0]))
                / (f64::from(colors.light[0]) - f64::from(colors.dark[0]));
            assert!(
                (share - dark).abs() < 0.12,
                "row {y}: dark share {share}, {dark} expected (phase {phase})"
            );
            ramps += 1;
        } else {
            assert_eq!(got, expect, "row {y}: phase {phase}");
            checked += 1;
        }
    }
    assert!(ramps > 20, "{ramps}");
    assert!(checked > 100 && checked + ramps > 150, "{checked} {ramps}");
    // asin 0.95 is 1.25 rad each way: 2 · 2.5 rad / τ · 16 periods, two
    // edges each.
    let expected = (2.0 * 2.0 * 0.95_f64.pasin() / std::f64::consts::TAU * 16.0 * 2.0).floor();
    assert!(
        (f64::from(transitions) - expected).abs() <= 2.0,
        "{transitions} edges, {expected} expected"
    );
    // Mirror symmetric about the vertical through the middle: within 2
    // levels, since a ramp's grey follows the interpolated normal, which
    // is mirror symmetric only as far as the mesh is (one level apart on
    // about 6 % of the pairs); a stripe on one side only is 225 apart.
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
            let (a, b) = (pixel(&frame, l, y), pixel(&frame, r, y));
            if (0..3).any(|c| a[c].abs_diff(b[c]) > 2) {
                differ += 1;
            }
        }
    }
    assert!(differ * 100 <= total, "{differ} of {total} differ");
}

/// The dark share of a pixel: where its red channel lies between the
/// light and the dark colour.
fn dark_share_of(rgb: [u8; 3]) -> f64 {
    let c = AnalysisColors::default();
    (f64::from(c.light[0]) - f64::from(rgb[0])) / (f64::from(c.light[0]) - f64::from(c.dark[0]))
}

/// The mean and the variance of the dark share over the pixels `keep`
/// picks (by their model coordinates across and up, seen orthographically
/// from `eye` with `up`, a half height of 1.1).
fn share_stats(
    mesh: &Mesh,
    analysis: Analysis,
    eye: Vec3,
    up: Vec3,
    side: u32,
    keep: impl Fn(f64, f64) -> bool,
) -> (f64, f64, usize) {
    let half = 1.1;
    let frame = render_analysed(
        mesh,
        &FaceColors::new(),
        &[],
        &ortho(eye, up, half),
        &params(side, side, analysis),
    )
    .unwrap();
    let mut shares = Vec::new();
    for y in 0..side as usize {
        for x in 0..side as usize {
            if frame.ids[y * side as usize + x] == NO_FACE {
                continue;
            }
            let across = ((x as f64 + 0.5) / f64::from(side) * 2.0 - 1.0) * half;
            let upward = (1.0 - (y as f64 + 0.5) / f64::from(side) * 2.0) * half;
            if keep(across, upward) {
                shares.push(dark_share_of(pixel(&frame, x, y)));
            }
        }
    }
    let n = shares.len() as f64;
    let mean = shares.iter().sum::<f64>() / n;
    let var = shares.iter().map(|s| (s - mean) * (s - mean)).sum::<f64>() / n;
    (mean, var, shares.len())
}

/// Stripes finer than a pixel fade to their mean instead of aliasing
/// (docs/design/surfacing.md › Finding 6): zebra on a sphere seen square
/// on, one sample per pixel, its middle (within 0.6 of the centre), the
/// stripes swept from resolved to finer than a pixel. The phase changes by
/// `2 / cos θ · count / τ` per unit, so a pixel of 2.2 / 121 units spans
/// at least 0.006 of a period per stripe of `count`: about 0.02 at 4, 1.5
/// at 256 and 3 at 512. Resolved, the shares are dark or light (variance
/// near `w (1 − w)`); finer than a pixel their mean is `width` and their
/// variance falls (a tent `±f` periods wide is within `w (1 − w) / f²` of
/// the mean, a box `f` wide within `w (1 − w) / f`). Point sampled, the variance stays near `w (1 − w)` at any count.
#[test]
fn stripes_finer_than_a_pixel_fade_to_their_mean() {
    let mesh = mesh_of(
        craft3d_testkit::solids::sphere(&world(), 1.0).unwrap(),
        1e-4,
    );
    let width = 0.3;
    let full = width * (1.0 - width);
    let mut last_var = f64::INFINITY;
    for (stripes, footprint) in [(4.0, 0.024), (64.0, 0.38), (256.0, 1.5), (512.0, 3.0)] {
        let mut a = Analysis::new(AnalysisMode::Zebra);
        a.stripes = stripes;
        a.width = width;
        let (mean, var, n) =
            share_stats(&mesh, a, Vec3::new(0.0, 0.0, 10.0), Vec3::Y, 121, |x, y| {
                x * x + y * y < 0.36
            });
        println!("zebra {stripes}: mean {mean:.4} variance {var:.5} over {n}");
        assert!(n > 3000, "{n}");
        if footprint < 0.1 {
            assert!(var > 0.7 * full, "{stripes}: variance {var} resolved");
        }
        if footprint >= 1.0 {
            assert!((mean - width).abs() < 0.01, "{stripes}: mean {mean}");
            // Every share within `full / f²` of the mean (the tent).
            let bound = full / (footprint * footprint);
            assert!(var < bound * bound, "{stripes}: variance {var}");
        }
        assert!(
            var <= last_var,
            "{stripes}: variance {var} after {last_var}"
        );
        last_var = var;
    }
}

/// Highlight lines toward a vertical face (Finding 6): their spacing
/// shrinks as `|N·M|²` (the normal line meets the light plane further
/// out), so on a sphere seen from the side, where `0.06 < |n_z| < 0.1`
/// (above the grazing bound of 0.05), 24 lines are tens of periods per
/// pixel: the band is their mean, `width` dark, without moiré. Below the
/// bound, where there is no phase, the sample is that mean too: the band
/// across the bound is one grey.
#[test]
fn highlight_lines_toward_a_vertical_face_fade_to_their_mean() {
    let mesh = mesh_of(
        craft3d_testkit::solids::sphere(&world(), 1.0).unwrap(),
        1e-4,
    );
    let mut a = Analysis::new(AnalysisMode::Highlight);
    a.stripes = 24.0;
    a.width = 0.3;
    let (mean, var, n) = share_stats(
        &mesh,
        a,
        Vec3::new(0.0, -10.0, 0.0),
        Vec3::Z,
        241,
        |x, z| x.abs() < 0.5 && (0.06..0.1).contains(&z.abs()),
    );
    println!("highlight near vertical: mean {mean:.4} variance {var:.5} over {n}");
    assert!(n > 400, "{n}");
    assert!((mean - 0.3).abs() < 0.01, "mean {mean}");
    assert!(var < 1e-3, "variance {var}");
    let (mean, var, n) = share_stats(
        &mesh,
        a,
        Vec3::new(0.0, -10.0, 0.0),
        Vec3::Z,
        241,
        |x, z| x.abs() < 0.5 && z.abs() < 0.1,
    );
    println!("highlight across the grazing bound: mean {mean:.4} variance {var:.5} over {n}");
    assert!(n > 800, "{n}");
    assert!((mean - 0.3).abs() < 0.01, "mean {mean}");
    assert!(var < 1e-3, "variance {var}");
}

/// A plane in orthographic zebra has one normal and one view direction, so
/// one phase and no footprint (only the rounding of identical vertex
/// normals): every face of a block is one of the two stripe colours
/// exactly, never a grey (reviewer's finding on a5ffa872: a footprint of
/// rounding, about 10⁻¹⁷, made the tent's second difference draw the
/// mean; on this mesh the footprint comes out exactly zero, so this holds
/// the outcome, and `analysis::tests::dark_share_holds_at_small_footprints`
/// the cause).
#[test]
fn planes_in_orthographic_zebra_are_the_stripe_colours() {
    let block = craft3d_testkit::solids::block(&world(), 2.0, 1.5, 1.0).unwrap();
    let mesh = mesh_of(block, 1e-3);
    let colors = AnalysisColors::default();
    let mut seen = std::collections::BTreeSet::new();
    for (k, eye) in [
        Vec3::new(3.0, -4.0, 5.0),
        Vec3::new(-5.0, -2.0, 3.0),
        Vec3::new(4.0, 3.0, -2.0),
    ]
    .into_iter()
    .enumerate()
    {
        for (stripes, offset) in [(4.0, 0.0), (16.0, 0.37), (64.0, 0.11)] {
            let mut a = Analysis::new(AnalysisMode::Zebra);
            a.stripes = stripes;
            a.offset = offset;
            a.width = 0.3;
            let camera = Camera {
                eye,
                target: Vec3::new(1.0, 0.75, 0.5),
                up: Vec3::Z,
                projection: Projection::Orthographic { half_height: 2.0 },
            };
            let frame = render_analysed(
                &mesh,
                &FaceColors::new(),
                &[],
                &camera,
                &params(160, 120, a),
            )
            .unwrap();
            let mut model = 0;
            for (i, &id) in frame.ids.iter().enumerate() {
                if id == NO_FACE {
                    continue;
                }
                model += 1;
                let rgb = [frame.rgb[3 * i], frame.rgb[3 * i + 1], frame.rgb[3 * i + 2]];
                assert!(
                    rgb == colors.dark || rgb == colors.light,
                    "view {k}, {stripes} stripes: pixel {i} of face {id} is {rgb:?}"
                );
                seen.insert(rgb);
            }
            assert!(model > 1500, "{model}");
        }
    }
    assert_eq!(seen.len(), 2, "both colours appear over the views");
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
    // Both stripe colours appear; the other columns are the greys of the
    // edges' ramps (`analysis::dark_share`).
    let c = AnalysisColors::default();
    assert!(
        stripes.contains(&c.dark) && stripes.contains(&c.light),
        "both stripe colours appear"
    );
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
