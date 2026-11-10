//! The surface analyses (zebra, highlight lines, the curvature map, draft)
//! drawn by the GPU renderer against the CPU oracle
//! (`craft3d_render::analysis`; docs/design/surfacing.md, stage 1).
//!
//! Tolerance: outside the band of one pixel around silhouettes and face
//! edges (`common::band`) and around where a face hides part of itself
//! (the CPU frame's distance jumps by more than 2 %), and outside the
//! pixels *in doubt*, every channel within 2 levels and at most 1/1024 of
//! the pixels compared more than 1 level apart. A pixel is in doubt where
//! one of its samples lies within what `f32` can tell of a place where the
//! colour changes faster than the tolerance: where the CPU picture drawn
//! with the stripes shifted by ±[`STRIPE_DOUBT`] (zebra) or
//! ±[`HIGHLIGHT_DOUBT`] (highlight lines) of a period per stripe, with the
//! grazing bound by ±[`GRAZING_DOUBT`], the least draft by
//! ±[`DRAFT_DOUBT`], the normals turned by ±[`NORMAL_DOUBT`] about two
//! axes, or the camera moved by ±[`PAN_DOUBT`] of a sample across and up,
//! is more than 2 levels off the plain one there. The stripes are filtered
//! over each sample's footprint (docs/design/surfacing.md › *Band limit*),
//! so they are a continuous grey and a doubt is a real decision: the
//! point-sampled footprint of zero, the face's own normal, an edge between
//! two triangles where the footprint (each triangle's derivatives) jumps.
//! The doubt may hold at most a share of the model's pixels per scene, so a
//! picture of nothing but edges cannot pass.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;

use common::{band, offscreens, orthographic, params, perspective};
use craft3d_gpu::{Camera, FaceColors, Frame, Mesh, NO_FACE, RenderParams};
use craft3d_math::Vec3;
use craft3d_render::analysis::{Analysis, AnalysisMode};
use craft3d_tess::{TessParams, tessellate_body};

/// The zebra stripes' shift that marks a pixel in doubt, in periods per
/// stripe of the count: the phase `φ / τ · count` is at most `count / 2`,
/// and `4·10⁻⁶ · count` is about 128 roundings (`2^-24`) of that. It
/// relies on the adapter's `atan2` being within about 64 ULP of `φ` (that
/// alone is `1.9·10⁻⁶ · count`), as the adapters tested are (RADV,
/// radeonsi, llvmpipe, Intel); WGSL itself only promises 4096 ULP, which
/// would be `1.2·10⁻⁴ · count`. Since the band limit (2026-10-31) the
/// stripes are a continuous grey, which a shift moves by its slope times
/// the shift (at most `225 / footprint` levels per period), so the shift
/// is what `f32` can be off by, not 10⁻³ of a period, which moved every
/// ramp more than the tolerance (half the pixels in doubt).
const STRIPE_DOUBT: f64 = 4e-6;
/// The highlight lines' shift, in periods per line of the count: their
/// phase is at most about `count / GRAZING · 1.3` periods (the light plane
/// a quarter of the diagonal above the box) and carries a relative error
/// of a few `2^-24` of it: `count · 3·10⁻⁶`, taken six times. How fast it
/// moves with the normal (`stripes / |N·M|²` periods per radian, up to
/// 8000 at the grazing bound with 20 lines) is the normals' turn's
/// ([`NORMAL_DOUBT`]), which holds it where it is that fast and not over
/// the whole picture, as the shift of 10⁻² of a period before the band
/// limit did.
const HIGHLIGHT_DOUBT: f64 = 2e-5;
/// The grazing bound's relative shift: `|N·M|` in `f32` is off by a few
/// `2^-24` of 1, against a bound of 0.05.
const GRAZING_DOUBT: f64 = 1e-4;
/// The normals' turn that marks a pixel in doubt, radians: an interpolated
/// normal in `f32` is off by about 2·10⁻⁷ rad; fifty times that.
const NORMAL_DOUBT: f64 = 1e-5;
/// The camera's move that marks a pixel in doubt, in samples: GPUs snap
/// vertices to a grid of 1/256 of a sample (8 bits of subpixel
/// precision); twice that.
const PAN_DOUBT: f64 = 1.0 / 128.0;
/// The least draft's shift that marks a pixel in doubt, radians: a unit
/// normal in `f32` is off by a few `2^-24`.
const DRAFT_DOUBT: f64 = 1e-4;
/// The shares in doubt measured on 2026-10-30 (the same on every adapter
/// of the Strix: RADV, radeonsi, llvmpipe; the doubt is the CPU's alone),
/// each cap a quarter above: the curvature map 0.0013; draft on the sphere
/// 0.0014, on the cylinder 0.0017. Since the band limit (2026-10-31, a
/// pixel in doubt where a variant is more than 2 levels off, the stripes'
/// shifts what `f32` can be off by, and the camera's move): zebra on the
/// sphere 0.0011 (was 0.0124), on the cylinder 0.0011 (was 0.0116), at
/// 512 stripes 0; highlight lines on the torus 0.0023 (was 0.118), with 20
/// lines 0.0028 (did not hold before); the curvature map 0, draft 0.0008
/// and 0.0009; seen close on a gentle surface (the footprint about 10⁻⁴,
/// the edges nearly sharp, so decisions again) 0.014 (highlight lines) and
/// 0.0085 (zebra). The caps of the stripes are a quarter above, at least
/// 0.001.
const CAP_ZEBRA_SPHERE: f64 = 0.0015;
const CAP_ZEBRA_CYLINDER: f64 = 0.0015;
const CAP_FINE_ZEBRA: f64 = 0.001;
const CAP_HIGHLIGHT: f64 = 0.003;
const CAP_FINE_HIGHLIGHT: f64 = 0.0035;
const CAP_CLOSE: f64 = 0.018;
const CAP_CURVATURE: f64 = 0.002;
const CAP_DRAFT: f64 = 0.0025;

fn mesh_of(solid: craft3d_testkit::solids::Reference) -> Mesh {
    let tess = TessParams {
        chordal_tolerance: 0.002,
        ..TessParams::default()
    };
    tessellate_body(&solid.body, &tess).unwrap()
}

fn world() -> craft3d_math::Frame {
    craft3d_math::Frame::new(Vec3::ZERO, Vec3::Z, Vec3::X).unwrap()
}

fn sphere() -> Mesh {
    mesh_of(craft3d_testkit::solids::sphere(&world(), 1.0).unwrap())
}

fn cylinder() -> Mesh {
    mesh_of(craft3d_testkit::solids::cylinder(&world(), 0.5, 2.0).unwrap())
}

fn torus() -> Mesh {
    mesh_of(craft3d_testkit::solids::torus(&world(), 1.0, 0.35).unwrap())
}

/// A value per vertex standing for a curvature map: the height, so that
/// the ramp runs over the whole model.
fn heights(mesh: &Mesh) -> Vec<f32> {
    mesh.positions.iter().map(|p| p.z as f32).collect()
}

fn with(mode: AnalysisMode, edit: impl Fn(&mut Analysis)) -> RenderParams {
    let mut a = Analysis::new(mode);
    edit(&mut a);
    RenderParams {
        analysis: Some(a),
        ..params(240, 180, 2)
    }
}

/// Whether two pixels are more than the tolerance (2 levels) apart in a
/// channel.
fn apart(x: &[u8], y: &[u8]) -> bool {
    x.iter().zip(y).any(|(a, b)| a.abs_diff(*b) > 2)
}

/// The pixels in doubt (see the module docs).
fn doubt(
    mesh: &Mesh,
    values: &[f32],
    camera: &Camera,
    p: &RenderParams,
) -> (Vec<bool>, Vec<Frame>) {
    let a = p.analysis.unwrap();
    let shifted = |sign: f64| {
        let mut b = a;
        match a.mode {
            AnalysisMode::Zebra => b.offset += sign * STRIPE_DOUBT * a.stripes.round(),
            AnalysisMode::Highlight => {
                b.offset += sign * HIGHLIGHT_DOUBT * a.stripes;
                b.grazing *= 1.0 + sign * GRAZING_DOUBT;
            }
            AnalysisMode::Draft => b.min_draft += sign * DRAFT_DOUBT,
            AnalysisMode::Curvature => {}
        }
        let q = RenderParams {
            analysis: Some(b),
            ..*p
        };
        craft3d_render::render_analysed(mesh, &FaceColors::new(), values, camera, &q).unwrap()
    };
    let (lo, hi) = (shifted(-1.0), shifted(1.0));
    let mut out: Vec<bool> = lo
        .rgb
        .chunks(3)
        .zip(hi.rgb.chunks(3))
        .map(|(x, y)| apart(x, y))
        .collect();
    let mut variants = vec![lo, hi];
    // The normals turned by ±NORMAL_DOUBT about two axes: where a
    // decision is that sensitive to the normal, its `f32` rounding may
    // take it either way.
    let plain =
        craft3d_render::render_analysed(mesh, &FaceColors::new(), values, camera, p).unwrap();
    for (axis, sign) in [(0, 1.0), (0, -1.0), (1, 1.0), (1, -1.0)] {
        let (sin, cos) = (sign * NORMAL_DOUBT).psin_cos();
        let mut turned = mesh.clone();
        for n in &mut turned.normals {
            *n = if axis == 0 {
                Vec3::new(n.x, cos * n.y - sin * n.z, sin * n.y + cos * n.z)
            } else {
                Vec3::new(cos * n.x + sin * n.z, n.y, -sin * n.x + cos * n.z)
            };
        }
        let frame = craft3d_render::render_analysed(&turned, &FaceColors::new(), values, camera, p)
            .unwrap();
        for ((slot, x), y) in out
            .iter_mut()
            .zip(frame.rgb.chunks(3))
            .zip(plain.rgb.chunks(3))
        {
            *slot |= apart(x, y);
        }
        variants.push(frame);
    }
    // The camera moved by ±PAN_DOUBT of a sample across and up: a sample
    // that close to an edge between two triangles of a face may fall in
    // either (the GPU's rasteriser snaps to a subpixel grid), and the
    // stripes' footprint, from each triangle's own derivatives, jumps
    // there.
    let axes = camera.axes().unwrap();
    let samples = f64::from(p.height * p.ssaa);
    let size = match camera.projection {
        craft3d_render::Projection::Perspective { .. } => {
            2.0 * axes.half * (camera.target - camera.eye).length() / samples
        }
        craft3d_render::Projection::Orthographic { .. } => 2.0 * axes.half / samples,
    };
    for along in [axes.right, axes.right * -1.0, axes.up, axes.up * -1.0] {
        let step = along * (PAN_DOUBT * size);
        let moved = Camera {
            eye: camera.eye + step,
            target: camera.target + step,
            ..*camera
        };
        let frame =
            craft3d_render::render_analysed(mesh, &FaceColors::new(), values, &moved, p).unwrap();
        for ((slot, x), y) in out
            .iter_mut()
            .zip(frame.rgb.chunks(3))
            .zip(plain.rgb.chunks(3))
        {
            *slot |= apart(x, y);
        }
        variants.push(frame);
    }
    (out, variants)
}

/// Compares the GPU's picture with the CPU's (see the module docs);
/// `cap` is the largest share of the model's pixels that may be in doubt
/// for this scene, a small margin over the share measured (2026-10-30).
fn check(name: &str, mesh: &Mesh, values: &[f32], camera: &Camera, p: &RenderParams, cap: f64) {
    let cpu = craft3d_render::render_analysed(mesh, &FaceColors::new(), values, camera, p).unwrap();
    let (in_doubt, mut variants) = doubt(mesh, values, camera, p);
    variants.push(cpu.clone());
    let mut near_edge = band(mesh, camera, &cpu);
    mark_occlusion(&cpu, &mut near_edge);
    for mut gpu in offscreens() {
        let adapter = gpu.adapter().name.clone();
        let frame = gpu
            .render_analysed(mesh, &FaceColors::new(), values, camera, p)
            .unwrap();
        let report = compare(&cpu, &frame, &near_edge, &in_doubt, &variants);
        if report.over_one > 0 {
            let png = |f: &Frame| craft3d_render::encode_png(f.width, f.height, &f.rgb).unwrap();
            craft3d_testkit::evidence::print(&format!("{name}-cpu"), &png(&cpu));
            craft3d_testkit::evidence::print(&format!("{name}-gpu"), &png(&frame));
        }
        assert!(report.compared > 0, "{name} on {adapter}: nothing compared");
        assert!(
            report.doubt as f64 <= cap * report.model as f64,
            "{name} on {adapter}: {} of {} pixels in doubt, over {cap}",
            report.doubt,
            report.model
        );
        assert!(
            report.doubt_unmatched as f64 <= report.compared as f64 / 1024.0,
            "{name} on {adapter}: {} pixels in doubt match none of the CPU's pictures ({report:?})",
            report.doubt_unmatched
        );
        assert!(
            report.max <= 2,
            "{name} on {adapter}: a channel {} levels off ({report:?})",
            report.max
        );
        assert!(
            report.over_one as f64 <= report.compared as f64 / 1024.0,
            "{name} on {adapter}: {} pixels over one level ({report:?})",
            report.over_one
        );
        println!(
            "{name} on {adapter}: doubt share {:.4}; {report:?}",
            report.doubt as f64 / report.model as f64
        );
        let png = craft3d_render::encode_png(frame.width, frame.height, &frame.rgb).unwrap();
        std::fs::write(common::evidence(&format!("analysis-{name}.png")), png).unwrap();
    }
}

/// Adds to the band the pixels within one pixel of where the CPU frame's
/// distance jumps by more than 2 % between neighbours: a face hiding
/// another part of itself (the inside of a torus's hole seen over its
/// far side), a silhouette `band` does not know of, since the face is
/// the same on both sides. Which surface a sample there takes is the
/// rasteriser's coverage, as along every silhouette.
fn mark_occlusion(cpu: &Frame, band: &mut [bool]) {
    let (w, h) = (cpu.width as usize, cpu.height as usize);
    let jump = |a: f64, b: f64| {
        a.is_finite() && b.is_finite() && (a - b).abs() > 0.02 * a.abs().min(b.abs())
    };
    let mut marks = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let right = x + 1 < w && jump(cpu.depth[i], cpu.depth[i + 1]);
            let down = y + 1 < h && jump(cpu.depth[i], cpu.depth[i + w]);
            if right || down {
                marks.push((x, y));
            }
        }
    }
    for (x, y) in marks {
        for ny in y.saturating_sub(1)..=(y + 2).min(h - 1) {
            for nx in x.saturating_sub(1)..=(x + 2).min(w - 1) {
                band[ny * w + nx] = true;
            }
        }
    }
}

#[derive(Debug, Default)]
struct Report {
    model: usize,
    doubt: usize,
    compared: usize,
    max: u8,
    over_one: usize,
    /// Pixels in doubt within 2 levels of none of the CPU's pictures
    /// (the plain one and the shifted ones): at most 1/1024 of those
    /// compared, as for pixels more than a level apart (two samples of a
    /// pixel may each go the other way under different shifts: seen on 2
    /// of 1026 pixels in doubt of the highlight lines).
    doubt_unmatched: usize,
    /// The first pixels more than a level apart: x, y, CPU, GPU.
    worst: Vec<(usize, usize, [u8; 3], [u8; 3])>,
}

fn compare(
    cpu: &Frame,
    gpu: &Frame,
    near_edge: &[bool],
    in_doubt: &[bool],
    variants: &[Frame],
) -> Report {
    let mut r = Report::default();
    for i in 0..cpu.ids.len() {
        if cpu.ids[i] == NO_FACE {
            continue;
        }
        r.model += 1;
        if near_edge[i] {
            continue;
        }
        if in_doubt[i] {
            // Compared too: the GPU took one of the ways the CPU's
            // decisions there could go.
            r.doubt += 1;
            let close =
                |f: &Frame| (0..3).all(|c| f.rgb[3 * i + c].abs_diff(gpu.rgb[3 * i + c]) <= 2);
            if !variants.iter().any(close) {
                r.doubt_unmatched += 1;
            }
            continue;
        }
        r.compared += 1;
        let d = (0..3)
            .map(|c| cpu.rgb[3 * i + c].abs_diff(gpu.rgb[3 * i + c]))
            .max()
            .unwrap();
        r.max = r.max.max(d);
        if d > 1 {
            r.over_one += 1;
            if r.worst.len() < 12 {
                let px = |f: &Frame| [f.rgb[3 * i], f.rgb[3 * i + 1], f.rgb[3 * i + 2]];
                let w = cpu.width as usize;
                r.worst.push((i % w, i / w, px(cpu), px(gpu)));
            }
        }
    }
    r
}

#[test]
fn zebra_on_a_sphere_matches_the_cpu() {
    let mesh = sphere();
    let p = with(AnalysisMode::Zebra, |_| {});
    let camera = orthographic(&mesh, Vec3::new(1.0, -2.0, 1.0), &p);
    check("zebra-sphere", &mesh, &[], &camera, &p, CAP_ZEBRA_SPHERE);
}

#[test]
fn zebra_on_a_cylinder_in_perspective_matches_the_cpu() {
    let mesh = cylinder();
    let p = with(AnalysisMode::Zebra, |a| {
        a.stripes = 24.0;
        a.direction = 30f64.to_radians();
        a.width = 0.3;
    });
    let camera = perspective(&mesh, Vec3::new(2.0, -1.0, 0.7), 35.0, &p);
    check(
        "zebra-cylinder",
        &mesh,
        &[],
        &camera,
        &p,
        CAP_ZEBRA_CYLINDER,
    );
}

#[test]
fn highlight_lines_on_a_torus_match_the_cpu() {
    let mesh = torus();
    // Few lines, so that they are resolved; 20, finer than a sample on the
    // far side, are `stripes_finer_than_a_sample_match_the_cpu`.
    let p = with(AnalysisMode::Highlight, |a| a.stripes = 5.0);
    let camera = perspective(&mesh, Vec3::new(1.0, -1.5, 1.2), 35.0, &p);
    check("highlight-torus", &mesh, &[], &camera, &p, CAP_HIGHLIGHT);
}

/// Stripes finer than a sample (Finding 6): before the band limit the
/// two renderers differed by single samples in a moiré that no shift
/// explained (20 highlight lines on this torus: 57 pixels more than a
/// level apart); filtered, the moiré is the stripes' mean on both.
#[test]
fn stripes_finer_than_a_sample_match_the_cpu() {
    let torus = torus();
    let p = with(AnalysisMode::Highlight, |a| a.stripes = 20.0);
    let camera = perspective(&torus, Vec3::new(1.0, -1.5, 1.2), 35.0, &p);
    check(
        "highlight-torus-20",
        &torus,
        &[],
        &camera,
        &p,
        CAP_FINE_HIGHLIGHT,
    );
    let sphere = sphere();
    let p = with(AnalysisMode::Zebra, |a| a.stripes = 512.0);
    let camera = orthographic(&sphere, Vec3::new(1.0, -2.0, 1.0), &p);
    check(
        "zebra-sphere-512",
        &sphere,
        &[],
        &camera,
        &p,
        CAP_FINE_ZEBRA,
    );
}

/// Few stripes seen close on a gently curved surface (the class-A case):
/// a sphere of radius 1 from 0.3 above its pole through a 2° lens, with 4
/// stripes, so that the footprint is about 10⁻⁴ of a period per sample,
/// where the tent's second difference cancelled in `f32` (reviewer's
/// finding on a5ffa872: shares of 0.86 to 0.95 for 1, some samples the
/// other colour). Each stripe edge in turn is put at the middle of the
/// picture (the offset from the phase there); now the point sample away
/// from the edge and the one edge's share at it.
#[test]
fn stripes_seen_close_on_a_gentle_surface_match_the_cpu() {
    let mesh = sphere();
    let target = Vec3::new(0.0, 0.0, 1.0);
    let camera = Camera {
        eye: Vec3::new(0.06, -0.08, 1.3),
        target,
        up: Vec3::Y,
        projection: craft3d_render::Projection::Perspective { fov_y_deg: 2.0 },
    };
    let axes = camera.axes().unwrap();
    let bounds = craft3d_math::Bbox3::from_points(mesh.positions.iter().copied());
    for mode in [AnalysisMode::Highlight, AnalysisMode::Zebra] {
        let mut a = Analysis::new(mode);
        a.stripes = 4.0;
        a.width = 0.3;
        let r = craft3d_render::analysis::Resolved::new(&a, &axes, &bounds);
        let to_viewer = (camera.eye - target).normalized().unwrap();
        let phase = r.phase(target, to_viewer, target - camera.eye).unwrap();
        let fraction = phase - phase.floor();
        for (k, edge) in [0.0, a.width].into_iter().enumerate() {
            let p = with(mode, |b| {
                b.stripes = a.stripes;
                b.width = a.width;
                b.offset = edge - fraction;
            });
            let name = format!("{}-close-{k}", mode.name());
            check(&name, &mesh, &[], &camera, &p, CAP_CLOSE);
        }
    }
}

#[test]
fn a_curvature_map_on_a_torus_matches_the_cpu() {
    let mesh = torus();
    let values = heights(&mesh);
    let p = with(AnalysisMode::Curvature, |a| a.range = [-0.35, 0.35]);
    let camera = perspective(&mesh, Vec3::new(1.0, -1.5, 1.2), 35.0, &p);
    check(
        "curvature-torus",
        &mesh,
        &values,
        &camera,
        &p,
        CAP_CURVATURE,
    );
}

#[test]
fn draft_on_a_cylinder_and_a_sphere_matches_the_cpu() {
    for (name, mesh) in [("draft-sphere", sphere()), ("draft-cylinder", cylinder())] {
        let p = with(AnalysisMode::Draft, |a| {
            a.pull = Vec3::new(0.2, 0.0, 1.0);
            a.min_draft = 5f64.to_radians();
        });
        let camera = perspective(&mesh, Vec3::new(1.0, -2.0, 0.8), 35.0, &p);
        check(name, &mesh, &[], &camera, &p, CAP_DRAFT);
    }
}

#[test]
fn analysis_is_deterministic_on_the_gpu() {
    let mesh = torus();
    let values = heights(&mesh);
    let p = with(AnalysisMode::Curvature, |a| a.range = [-0.35, 0.35]);
    let camera = perspective(&mesh, Vec3::new(1.0, -1.5, 1.2), 35.0, &p);
    for mut gpu in offscreens() {
        let one = gpu
            .render_analysed(&mesh, &FaceColors::new(), &values, &camera, &p)
            .unwrap();
        let two = gpu
            .render_analysed(&mesh, &FaceColors::new(), &values, &camera, &p)
            .unwrap();
        assert_eq!(one.rgb, two.rgb);
        assert_eq!(one.ids, two.ids);
    }
}

#[test]
fn without_an_analysis_the_picture_is_the_plain_one() {
    let mesh = torus();
    let p = params(160, 120, 2);
    let camera = perspective(&mesh, Vec3::new(1.0, -1.5, 1.2), 35.0, &p);
    for mut gpu in offscreens() {
        let plain = gpu.render(&mesh, &camera, &p).unwrap();
        let values = heights(&mesh);
        let with_values = gpu
            .render_analysed(&mesh, &FaceColors::new(), &values, &camera, &p)
            .unwrap();
        assert_eq!(plain, with_values);
    }
}
