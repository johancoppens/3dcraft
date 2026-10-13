//! Shared by the integration tests: adapters, scenes, and the comparison of
//! a GPU frame with the CPU renderer's (oracle 7 in docs/testing.md).
//!
//! # The tolerances, and where they come from
//!
//! The two renderers run the same algorithm on the same samples; they differ
//! in arithmetic. The CPU renderer works in `f64` on exact vertex positions.
//! The GPU works in `f32` and its rasteriser first snaps every projected
//! vertex to a grid of `2^-8` pixel (eight sub-pixel bits: what every desktop
//! driver and lavapipe have; Vulkan's `subPixelPrecisionBits`). Everything
//! below follows from those two facts.
//!
//! **The band.** Snapping moves a triangle's edge by at most one grid step,
//! so the two renderers can give a sample to different triangles only when
//! it lies within `2^-8` pixel of an edge. Between two triangles of one
//! smooth face that changes nothing visible. It matters where the surface
//! under the sample changes: at *feature edges* (the border of a face, an
//! edge without a second triangle, a silhouette edge whose two triangles
//! face opposite ways on screen) and where two surfaces pass through each
//! other, which shows as a change of face id in the CPU frame. The band is
//! every pixel within one pixel (its 3 × 3 neighbourhood) of a projected
//! feature edge or of a change of id in the CPU frame. It is computed from
//! the mesh, the camera and the CPU frame only, never from the GPU frame.
//!
//! **Face ids** outside the band: equal, no tolerance.
//!
//! **Inside the band** the renderers may disagree, but only on samples within
//! `2^-8` pixel of an edge. A line that crosses a pixel passes within `2^-8`
//! pixel of its centre with probability `2 · 2^-8 = 1/128`. That is the
//! limit, [`BAND_MISMATCH`], as a fraction of the band's pixels: the rate if
//! an edge ran through every pixel of the band, where in fact the band is
//! three pixels wide around each edge (so about a third of it is expected
//! from a single edge, and the rest is room for the edges that crowd
//! together at a pole or a corner). A systematic error is far above it: a
//! picture shifted by one pixel changes about a third of the band.
//!
//! **Distance** outside the band: `|gpu - cpu| <= 2^-20 · reach + 2^-7 · G`.
//! `reach` is the largest distance the vertex shader subtracts (eye to mesh
//! centre plus the mesh diagonal): one `f32` rounding is `2^-24` relative,
//! and the path from position to stored distance (subtract, three products,
//! two sums, the rasteriser's interpolation, each on numbers up to `reach`)
//! is bounded by sixteen of them. `G` is the largest difference between the
//! pixel's CPU distance and its eight neighbours', which bounds the gradient
//! of the distance per pixel; a vertex moved by one grid step (`2^-8` pixel)
//! changes the interpolated value by at most the gradient times that step,
//! doubled because the triangle's vertices move independently.
//!
//! **Colour** outside the band: at most [`COLOR_TOLERANCE`] = 1 of 255 per
//! channel, no exceptions. Both renderers round the shaded value to an
//! integer per sample; a value that lies within the `f32` error of a
//! half-integer rounds differently, which is one level. The box filter of
//! supersampling averages such differences and cannot exceed one level.
//! Where the picture itself changes quickly (the specular highlight on a
//! small feature) the snapped vertices add `2^-7 · Gc`, with `Gc` the
//! largest difference to the eight neighbours in the CPU picture, by the
//! argument used for the distance: one more level only where neighbouring
//! pixels differ by 128 levels or more.
//!
//! **How many** colours outside the band may differ by that one level: a
//! sample rounds differently only when its exact value lies within the
//! GPU's error `e` of a half-integer, which has probability `2e`. The error
//! is the `f32` error of the shading (`SHADING_ERROR` levels) plus what a
//! vertex moved by one grid step does to the interpolated colour
//! (`2^-7 · Gc`, as above). The sum of `min(1, 2 · (2^-7 · Gc + 0.004))`
//! over the pixels outside the band is the largest expected number of
//! differing pixels; more than that is a systematic error (a mean that is
//! truncated instead of rounded differs in half of the shaded pixels).

#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_gpu::{
    ADAPTER_HINT, Camera, Frame, Mesh, NO_FACE, Offscreen, Projection, RenderParams,
};
use craft3d_math::Vec3;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Largest colour difference per channel outside the band, in 1/255.
pub const COLOR_TOLERANCE: f64 = 1.0;
/// Largest fraction of the band's pixels whose face id, or whose colour by
/// more than the tolerance, may differ.
pub const BAND_MISMATCH: f64 = 1.0 / 128.0;
/// One sub-pixel grid step of the rasteriser, in pixels.
const SNAP: f64 = 1.0 / 256.0;
/// Bound on the `f32` error of a shaded value, in levels of 255: the
/// specular term `255 · 0.35 · x^48` multiplies the relative error of `x`
/// (a normalised dot product: about eight roundings of `2^-24`) by 48,
/// which is `89 · 48 · 8 · 2^-24 ≈ 0.002`; doubled.
const SHADING_ERROR: f64 = 0.004;
/// Bound on the accumulated `f32` rounding of a distance, relative to the
/// largest number involved: sixteen roundings of `2^-24`.
const F32_PATH: f64 = 16.0 / 16_777_216.0;

/// One renderer per adapter on this machine. Fails, loudly, when there is
/// none: a parity test that silently skips proves nothing.
pub fn offscreens() -> Vec<Offscreen> {
    let adapters = craft3d_gpu::adapters();
    assert!(
        !adapters.is_empty(),
        "no GPU adapter found, so the GPU renderer cannot be tested: {ADAPTER_HINT}"
    );
    adapters
        .iter()
        .map(|adapter| {
            Offscreen::on_adapter(adapter)
                .unwrap_or_else(|e| panic!("cannot use adapter {:?}: {e}", adapter.get_info().name))
        })
        .collect()
}

/// Where the developer looks (docs/testing.md, evidence).
pub fn evidence(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/gpu");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

pub fn params(width: u32, height: u32, ssaa: u32) -> RenderParams {
    RenderParams {
        width,
        height,
        ssaa,
        ..RenderParams::default()
    }
}

/// A perspective camera that frames `mesh` from `toward_eye`.
pub fn perspective(mesh: &Mesh, toward_eye: Vec3, fov_y_deg: f64, p: &RenderParams) -> Camera {
    let aspect = f64::from(p.width) / f64::from(p.height);
    Camera::framing(&mesh.bbox(), toward_eye, Vec3::Z, fov_y_deg, aspect).unwrap()
}

/// An orthographic camera that shows all of `mesh` from `toward_eye`.
pub fn orthographic(mesh: &Mesh, toward_eye: Vec3, p: &RenderParams) -> Camera {
    let bbox = mesh.bbox();
    let (center, diagonal) = (bbox.center().unwrap(), bbox.diagonal());
    let aspect = f64::from(p.width) / f64::from(p.height);
    Camera {
        eye: center + toward_eye.normalized().unwrap() * (diagonal * 2.0),
        target: center,
        up: Vec3::Z,
        projection: Projection::Orthographic {
            half_height: diagonal * 0.55 / aspect.min(1.0),
        },
    }
}

/// Pixel coordinates (x right, y down) and view distance of `p`, from the
/// definition in `craft3d_render::ViewAxes`. `None` at or behind the eye.
fn project(camera: &Camera, p: Vec3, width: f64, height: f64) -> Option<(f64, f64, f64)> {
    let axes = camera.axes().ok()?;
    let rel = p - axes.eye;
    let dist = rel.dot(axes.forward);
    let scale = match axes.projection {
        Projection::Perspective { .. } if dist > 0.0 => 1.0 / (dist * axes.half),
        Projection::Perspective { .. } => return None,
        Projection::Orthographic { .. } => 1.0 / axes.half,
    };
    let aspect = width / height;
    let x = (rel.dot(axes.right) * scale / aspect * 0.5 + 0.5) * width;
    let y = (0.5 - rel.dot(axes.up) * scale * 0.5) * height;
    (x.is_finite() && y.is_finite()).then_some((x, y, dist))
}

/// The coordinates of an edge's two ends, as bits, smaller first.
type EdgeKey = ([u64; 3], [u64; 3]);
/// An edge's two ends and, per triangle that uses it, the face id and the
/// sign of the triangle's area on screen.
type EdgeUse = (Vec3, Vec3, Vec<(u32, i8)>);

/// The band (see the module docs): `true` for a pixel within one pixel of a
/// feature edge of the mesh or of a change of face id in the CPU frame.
pub fn band(mesh: &Mesh, camera: &Camera, cpu: &Frame) -> Vec<bool> {
    let (w, h) = (cpu.width as usize, cpu.height as usize);
    let mut near_edge = vec![false; w * h];
    let mut mark = |x: f64, y: f64| {
        let (cx, cy) = (x.floor() as i64, y.floor() as i64);
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (px, py) = (cx + dx, cy + dy);
                if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h {
                    near_edge[py as usize * w + px as usize] = true;
                }
            }
        }
    };

    // Changes of face id in the CPU frame: face borders that are visible,
    // and the curves where surfaces pass through each other.
    for y in 0..h {
        for x in 0..w {
            let id = cpu.ids[y * w + x];
            let right = x + 1 < w && cpu.ids[y * w + x + 1] != id;
            let below = y + 1 < h && cpu.ids[(y + 1) * w + x] != id;
            if right || below {
                mark(x as f64 + 0.5, y as f64 + 0.5);
                if right {
                    mark(x as f64 + 1.5, y as f64 + 0.5);
                }
                if below {
                    mark(x as f64 + 0.5, y as f64 + 1.5);
                }
            }
        }
    }

    // Feature edges of the mesh. Triangles meet where their corners have
    // the same coordinates, whichever vertex index carries them.
    let key = |p: Vec3| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()];
    let mut edges: BTreeMap<EdgeKey, EdgeUse> = BTreeMap::new();
    for (t, &face) in mesh.face_ids.iter().enumerate() {
        let corners = mesh.corners(t).unwrap();
        let screen = corners.map(|p| project(camera, p, w as f64, h as f64));
        let side = match screen {
            [Some(a), Some(b), Some(c)] => {
                let area = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
                if area > 0.0 {
                    1
                } else if area < 0.0 {
                    -1
                } else {
                    0
                }
            }
            _ => 0,
        };
        for (p, q) in [(0, 1), (1, 2), (2, 0)] {
            let (p, q) = (corners[p], corners[q]);
            let (a, b) = if key(p) <= key(q) { (p, q) } else { (q, p) };
            edges
                .entry((key(a), key(b)))
                .or_insert_with(|| (a, b, Vec::new()))
                .2
                .push((face, side));
        }
    }
    for (a, b, users) in edges.values() {
        let smooth = match users.as_slice() {
            [(face_a, side_a), (face_b, side_b)] => {
                face_a == face_b && side_a == side_b && *side_a != 0
            }
            _ => false,
        };
        if smooth {
            continue;
        }
        let (Some(a), Some(b)) = (
            project(camera, *a, w as f64, h as f64),
            project(camera, *b, w as f64, h as f64),
        ) else {
            continue;
        };
        let length = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        // Half-pixel steps; an edge that long is mostly off screen anyway.
        let steps = ((length * 2.0).ceil() as usize).clamp(1, 8 * (w + h));
        for i in 0..=steps {
            let t = i as f64 / steps as f64;
            mark(a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
        }
    }
    near_edge
}

/// The probability, at most, that a pixel outside the band differs by a
/// level, given the largest difference to its neighbours in the CPU picture
/// (see the module docs).
pub fn differ_probability(color_gradient: f64) -> f64 {
    (2.0 * (2.0 * SNAP * color_gradient + SHADING_ERROR)).min(1.0)
}

/// The colour tolerance of a pixel outside the band, in levels, given the
/// largest difference to its neighbours in the CPU picture.
pub fn color_tolerance(color_gradient: f64) -> f64 {
    COLOR_TOLERANCE + (2.0 * SNAP * color_gradient).floor()
}

/// What the comparison of one scene found.
#[derive(Debug, Default)]
pub struct Parity {
    pub pixels: usize,
    pub covered: usize,
    pub band: usize,
    /// Outside the band: pixels with different face ids.
    pub id_outside: usize,
    /// Outside the band: pixels whose distance differs by more than the
    /// tolerance, and the largest difference as a fraction of it.
    pub depth_outside: usize,
    pub depth_ratio: f64,
    /// Outside the band: pixels by their largest channel difference
    /// (0, 1, 2, 3 or more), and those over their tolerance.
    pub color_histogram: [usize; 4],
    pub color_outside: usize,
    /// Outside the band: the number of pixels expected to differ at all, at
    /// most (the sum of [`differ_probability`]).
    pub color_differ_allowed: f64,
    /// Inside the band: pixels with different ids, and with a colour
    /// difference over the tolerance.
    pub id_band: usize,
    pub color_band: usize,
}

fn neighbours(x: usize, y: usize, w: usize, h: usize) -> impl Iterator<Item = usize> {
    (-1_i64..=1)
        .flat_map(|dy| (-1_i64..=1).map(move |dx| (dx, dy)))
        .filter_map(move |(dx, dy)| {
            let (nx, ny) = (x as i64 + dx, y as i64 + dy);
            (nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h && (dx, dy) != (0, 0))
                .then(|| ny as usize * w + nx as usize)
        })
}

/// Compares a GPU frame with the CPU frame of the same scene.
pub fn compare(mesh: &Mesh, camera: &Camera, cpu: &Frame, gpu: &Frame) -> (Parity, Vec<bool>) {
    assert_eq!((cpu.width, cpu.height), (gpu.width, gpu.height));
    assert_eq!(cpu.rgb.len(), gpu.rgb.len());
    assert_eq!(cpu.ids.len(), gpu.ids.len());
    assert_eq!(cpu.depth.len(), gpu.depth.len());
    let (w, h) = (cpu.width as usize, cpu.height as usize);
    let band = band(mesh, camera, cpu);
    let bbox = mesh.bbox();
    let reach = bbox
        .center()
        .map_or(0.0, |c| (camera.eye - c).length() + bbox.diagonal());
    let mut parity = Parity {
        pixels: w * h,
        ..Parity::default()
    };
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let (id_c, id_g) = (cpu.ids[i], gpu.ids[i]);
            parity.covered += usize::from(id_c != NO_FACE);
            let channel = |frame: &Frame, at: usize, c: usize| f64::from(frame.rgb[at * 3 + c]);
            let color_diff = (0..3)
                .map(|c| (channel(cpu, i, c) - channel(gpu, i, c)).abs())
                .fold(0.0, f64::max);
            if band[i] {
                parity.band += 1;
                parity.id_band += usize::from(id_c != id_g);
                parity.color_band += usize::from(color_diff > COLOR_TOLERANCE);
                continue;
            }
            parity.id_outside += usize::from(id_c != id_g);
            let color_gradient = neighbours(x, y, w, h)
                .flat_map(|n| (0..3).map(move |c| (n, c)))
                .map(|(n, c)| (channel(cpu, i, c) - channel(cpu, n, c)).abs())
                .fold(0.0, f64::max);
            let color_tolerance = COLOR_TOLERANCE + (2.0 * SNAP * color_gradient).floor();
            parity.color_histogram[(color_diff as usize).min(3)] += 1;
            parity.color_differ_allowed += differ_probability(color_gradient);
            parity.color_outside += usize::from(color_diff > color_tolerance);

            let (d_c, d_g) = (cpu.depth[i], gpu.depth[i]);
            if d_c.is_infinite() || d_g.is_infinite() {
                // Nothing drawn: both must say so.
                parity.depth_outside += usize::from(d_c != d_g);
                continue;
            }
            let gradient = neighbours(x, y, w, h)
                .map(|n| (cpu.depth[n] - d_c).abs())
                .filter(|g| g.is_finite())
                .fold(0.0, f64::max);
            let tolerance = F32_PATH * reach + 2.0 * SNAP * gradient;
            let error = (d_c - d_g).abs();
            // NaN on the GPU side must fail too.
            if error.is_nan() || error > tolerance {
                parity.depth_outside += 1;
            }
            parity.depth_ratio = parity.depth_ratio.max(error / tolerance);
        }
    }
    (parity, band)
}

impl Parity {
    /// The assertions of oracle 7.
    pub fn check(&self, scene: &str) {
        let report = format!("{scene}: {self:?}");
        // The comparison must not be hollow: most of the picture, and most
        // of the model in it, is compared strictly.
        assert!(self.covered > self.pixels / 50, "nothing to see: {report}");
        assert!(
            self.band < self.pixels / 2,
            "the band covers half the picture: {report}"
        );
        assert_eq!(
            self.id_outside, 0,
            "face ids differ outside the band: {report}"
        );
        assert_eq!(self.depth_outside, 0, "distances differ: {report}");
        assert_eq!(self.color_outside, 0, "colours differ: {report}");
        let differing: usize = self.color_histogram.iter().skip(1).sum();
        assert!(
            differing as f64 <= self.color_differ_allowed.ceil(),
            "{differing} colours differ by a level where rounding explains at most {}: {report}",
            self.color_differ_allowed.ceil()
        );
        let allowed = (self.band as f64 * BAND_MISMATCH).ceil() as usize;
        assert!(
            self.id_band <= allowed,
            "more than {allowed} face ids differ inside the band: {report}"
        );
        assert!(
            self.color_band <= allowed,
            "more than {allowed} colours differ inside the band: {report}"
        );
    }
}

/// CPU picture, GPU picture and their differences side by side, as a PNG:
/// in the third panel the band is dark blue, a colour difference is green
/// (brighter is larger; one level is already visible), a different face id
/// is red.
pub fn write_evidence(name: &str, cpu: &Frame, gpu: &Frame, band: &[bool]) -> PathBuf {
    let (w, h) = (cpu.width as usize, cpu.height as usize);
    let mut rgb = Vec::with_capacity(w * h * 9);
    for y in 0..h {
        for frame in [cpu, gpu] {
            rgb.extend_from_slice(&frame.rgb[y * w * 3..(y + 1) * w * 3]);
        }
        for x in 0..w {
            let i = y * w + x;
            let diff = (0..3)
                .map(|c| cpu.rgb[i * 3 + c].abs_diff(gpu.rgb[i * 3 + c]))
                .max()
                .unwrap();
            let red = if cpu.ids[i] != gpu.ids[i] { 255 } else { 0 };
            let green = if diff == 0 {
                0
            } else {
                (96 + u32::from(diff) * 16).min(255) as u8
            };
            let blue = if band[i] { 90 } else { 0 };
            rgb.extend([red, green, blue]);
        }
    }
    let path = evidence(name);
    let png = craft3d_render::encode_png(cpu.width * 3, cpu.height, &rgb).unwrap();
    std::fs::write(&path, png).unwrap();
    path
}

// ---------------------------------------------------------------------
// The multisampled picture of the viewport renderer (`Onscreen`).
//
// Multisampling shades once per pixel, at its centre, exactly where the
// CPU renderer takes its one sample for `ssaa = 1`; the samples only
// decide coverage. So a pixel whose samples all lie in one triangle has
// the very colour of the readback renderer, within the tolerance above.
// Two kinds of pixel are different by design:
//
// - **In the band**, at a feature edge, the samples lie on both sides and
//   the resolved colour is a blend: that is the anti-aliasing. Its colour
//   is not compared with the CPU's hard edge; the number of pixels that
//   differ is only measured.
// - **On a seam**, within a pixel of an interior triangle edge of one
//   face, the samples may lie in two triangles, each shaded at the pixel
//   centre by extrapolating its own interpolated normal, and the resolved
//   colour is their coverage-weighted mean. Each fragment is within one
//   pixel of gradient of the centre value, so the pixel is allowed
//   `COLOR_TOLERANCE + Gc` levels, with `Gc` the largest difference to its
//   eight neighbours in the CPU picture.
//
// Face ids and distances come from the renderer's capture pass at one
// sample per pixel and keep the tolerance above everywhere.

/// `true` for a pixel within one pixel of any projected triangle edge:
/// where a multisampled pixel may take its colour from two triangles.
pub fn seams(mesh: &Mesh, camera: &Camera, width: u32, height: u32) -> Vec<bool> {
    let (w, h) = (width as usize, height as usize);
    let mut near = vec![false; w * h];
    let mut mark = |x: f64, y: f64| {
        let (cx, cy) = (x.floor() as i64, y.floor() as i64);
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (px, py) = (cx + dx, cy + dy);
                if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h {
                    near[py as usize * w + px as usize] = true;
                }
            }
        }
    };
    for t in 0..mesh.triangles.len() {
        let corners = mesh.corners(t).unwrap();
        let screen = corners.map(|p| project(camera, p, w as f64, h as f64));
        for (p, q) in [(0, 1), (1, 2), (2, 0)] {
            let (Some(a), Some(b)) = (screen[p], screen[q]) else {
                continue;
            };
            let length = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
            let steps = ((length * 2.0).ceil() as usize).clamp(1, 8 * (w + h));
            for i in 0..=steps {
                let t = i as f64 / steps as f64;
                mark(a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
            }
        }
    }
    near
}

/// What the comparison of a multisampled picture found.
#[derive(Debug, Default)]
pub struct MsaaParity {
    pub pixels: usize,
    pub covered: usize,
    pub band: usize,
    /// Pixels outside the band: on a seam, and not.
    pub seam: usize,
    pub plain: usize,
    /// Outside the band: different face ids, distances over the tolerance
    /// and the largest distance error as a fraction of it.
    pub id_outside: usize,
    pub depth_outside: usize,
    pub depth_ratio: f64,
    /// Plain pixels over the colour tolerance of §7.2, their histogram of
    /// the largest channel difference (0, 1, 2, 3 or more), and how many
    /// of them rounding explains at most.
    pub color_plain_over: usize,
    pub color_plain_histogram: [usize; 4],
    pub color_plain_allowed: f64,
    /// Seam pixels over `COLOR_TOLERANCE + Gc`, and seam pixels that differ
    /// by more than one level (measured, not bounded).
    pub color_seam_over: usize,
    pub color_seam_differ: usize,
    /// Inside the band: different ids (bounded), and colours that differ
    /// by more than one level (the anti-aliasing: measured only).
    pub id_band: usize,
    pub color_band: usize,
}

/// Compares the multisampled picture `gpu` (with ids and distances from
/// the capture pass) with the CPU frame of the same scene at `ssaa = 1`.
pub fn compare_msaa(
    mesh: &Mesh,
    camera: &Camera,
    cpu: &Frame,
    gpu: &Frame,
) -> (MsaaParity, Vec<bool>) {
    assert_eq!((cpu.width, cpu.height), (gpu.width, gpu.height));
    assert_eq!(cpu.rgb.len(), gpu.rgb.len());
    assert_eq!(cpu.ids.len(), gpu.ids.len());
    assert_eq!(cpu.depth.len(), gpu.depth.len());
    let (w, h) = (cpu.width as usize, cpu.height as usize);
    let band = band(mesh, camera, cpu);
    let seams = seams(mesh, camera, cpu.width, cpu.height);
    let bbox = mesh.bbox();
    let reach = bbox
        .center()
        .map_or(0.0, |c| (camera.eye - c).length() + bbox.diagonal());
    let mut out = MsaaParity {
        pixels: w * h,
        ..MsaaParity::default()
    };
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let (id_c, id_g) = (cpu.ids[i], gpu.ids[i]);
            out.covered += usize::from(id_c != NO_FACE);
            let channel = |frame: &Frame, at: usize, c: usize| f64::from(frame.rgb[at * 3 + c]);
            let color_diff = (0..3)
                .map(|c| (channel(cpu, i, c) - channel(gpu, i, c)).abs())
                .fold(0.0, f64::max);
            if band[i] {
                out.band += 1;
                out.id_band += usize::from(id_c != id_g);
                out.color_band += usize::from(color_diff > COLOR_TOLERANCE);
                continue;
            }
            out.id_outside += usize::from(id_c != id_g);
            let color_gradient = neighbours(x, y, w, h)
                .flat_map(|n| (0..3).map(move |c| (n, c)))
                .map(|(n, c)| (channel(cpu, i, c) - channel(cpu, n, c)).abs())
                .fold(0.0, f64::max);
            if seams[i] {
                out.seam += 1;
                out.color_seam_over += usize::from(color_diff > COLOR_TOLERANCE + color_gradient);
                out.color_seam_differ += usize::from(color_diff > COLOR_TOLERANCE);
            } else {
                out.plain += 1;
                let tolerance = COLOR_TOLERANCE + (2.0 * SNAP * color_gradient).floor();
                out.color_plain_histogram[(color_diff as usize).min(3)] += 1;
                out.color_plain_allowed += differ_probability(color_gradient);
                out.color_plain_over += usize::from(color_diff > tolerance);
            }
            let (d_c, d_g) = (cpu.depth[i], gpu.depth[i]);
            if d_c.is_infinite() || d_g.is_infinite() {
                out.depth_outside += usize::from(d_c != d_g);
                continue;
            }
            let gradient = neighbours(x, y, w, h)
                .map(|n| (cpu.depth[n] - d_c).abs())
                .filter(|g| g.is_finite())
                .fold(0.0, f64::max);
            let tolerance = F32_PATH * reach + 2.0 * SNAP * gradient;
            let error = (d_c - d_g).abs();
            if error.is_nan() || error > tolerance {
                out.depth_outside += 1;
            }
            out.depth_ratio = out.depth_ratio.max(error / tolerance);
        }
    }
    (out, band)
}

impl MsaaParity {
    /// The assertions for a multisampled picture (see above).
    pub fn check(&self, scene: &str) {
        let report = format!("{scene}: {self:?}");
        assert!(self.covered > self.pixels / 50, "nothing to see: {report}");
        assert!(
            self.band < self.pixels / 2,
            "the band covers half the picture: {report}"
        );
        assert!(
            self.plain > self.pixels / 4,
            "too little of the picture is compared strictly: {report}"
        );
        assert_eq!(
            self.id_outside, 0,
            "face ids differ outside the band: {report}"
        );
        assert_eq!(self.depth_outside, 0, "distances differ: {report}");
        assert_eq!(
            self.color_plain_over, 0,
            "colours differ away from every edge: {report}"
        );
        let differing: usize = self.color_plain_histogram.iter().skip(1).sum();
        assert!(
            differing as f64 <= self.color_plain_allowed.ceil(),
            "{differing} colours differ by a level where rounding explains at most {}: {report}",
            self.color_plain_allowed.ceil()
        );
        assert_eq!(
            self.color_seam_over, 0,
            "colours on a seam differ by more than a pixel of gradient: {report}"
        );
        let allowed = (self.band as f64 * BAND_MISMATCH).ceil() as usize;
        assert!(
            self.id_band <= allowed,
            "more than {allowed} face ids differ inside the band: {report}"
        );
    }
}
