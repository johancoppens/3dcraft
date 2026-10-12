//! CPU reference renderer: the display oracle (docs/architecture.md §7).
//!
//! A software rasteriser with a depth buffer, smooth two-sided shading and an
//! id buffer that says which face is under each pixel. It favours exactness
//! and readability over speed, and is deterministic: single-threaded `f64`
//! arithmetic, triangles drawn in mesh order.

use craft3d_math::{Bbox3, Vec3};
use craft3d_tess::Mesh;
use std::collections::BTreeSet;
use std::fmt;

/// Id-buffer value of a pixel no face covers.
pub const NO_FACE: u32 = u32::MAX;
/// Largest accepted image side, in pixels.
pub const MAX_SIDE: u32 = 8192;
/// Largest accepted supersampling factor per axis.
pub const MAX_SSAA: u32 = 4;
/// Largest accepted number of internal samples (width × height × ssaa²).
pub const MAX_SAMPLES: u64 = 1 << 26;

#[derive(Clone, Debug, PartialEq)]
pub enum RenderError {
    InvalidSize {
        width: u32,
        height: u32,
    },
    InvalidSsaa(u32),
    TooManySamples {
        samples: u64,
    },
    /// Eye and target coincide, the up vector is parallel to the view
    /// direction, or a camera number is not finite or not positive.
    DegenerateCamera,
    /// A triangle refers to a vertex that does not exist, or the per-vertex
    /// and per-triangle arrays have mismatched lengths.
    InvalidMesh {
        triangle: usize,
    },
    Png(String),
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RenderError::InvalidSize { width, height } => write!(
                f,
                "image size {width}x{height} is not within 1..={MAX_SIDE} per side"
            ),
            RenderError::InvalidSsaa(s) => write!(f, "ssaa {s} is not within 1..={MAX_SSAA}"),
            RenderError::TooManySamples { samples } => write!(
                f,
                "{samples} samples exceed the limit of {MAX_SAMPLES}; lower the size or ssaa"
            ),
            RenderError::DegenerateCamera => write!(
                f,
                "camera is degenerate: eye equals target, up is parallel to the view \
                 direction, or the field of view / height is not a positive finite number"
            ),
            RenderError::InvalidMesh { triangle } => {
                write!(f, "mesh is inconsistent at triangle {triangle}")
            }
            RenderError::Png(e) => write!(f, "PNG encoding failed: {e}"),
        }
    }
}

impl std::error::Error for RenderError {}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Projection {
    /// Vertical field of view in degrees, in (0, 180).
    Perspective { fov_y_deg: f64 },
    /// Half of the visible height, in model units.
    Orthographic { half_height: f64 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub eye: Vec3,
    pub target: Vec3,
    pub up: Vec3,
    pub projection: Projection,
}

impl Camera {
    /// A perspective camera that looks at the centre of `bbox` from the
    /// direction `toward_eye`, as close as it can be while every corner of
    /// the box stays in view with a 5 % margin. `None` for an empty box, a
    /// zero direction, an up vector parallel to it, or a bad field of view
    /// or aspect ratio.
    pub fn framing(
        bbox: &Bbox3,
        toward_eye: Vec3,
        up: Vec3,
        fov_y_deg: f64,
        aspect: f64,
    ) -> Option<Camera> {
        let center = bbox.center()?;
        let (lo, hi) = bbox.corners()?;
        let dir = toward_eye.normalized()?;
        let right = up.cross(dir).normalized()?;
        let view_up = dir.cross(right);
        let half_y = (fov_y_deg.to_radians() * 0.5).tan();
        let half_x = half_y * aspect;
        if !(fov_y_deg > 0.0 && fov_y_deg < 180.0 && half_x.is_finite() && half_x > 0.0) {
            return None;
        }
        // A corner at offset `rel` from the centre is at depth
        // `distance - rel·dir` and must satisfy |rel·right| ≤ depth · half_x
        // and |rel·up| ≤ depth · half_y.
        let mut distance = 0.0_f64;
        for x in [lo.x, hi.x] {
            for y in [lo.y, hi.y] {
                for z in [lo.z, hi.z] {
                    let rel = Vec3::new(x, y, z) - center;
                    let along = rel.dot(dir);
                    distance = distance
                        .max(along + rel.dot(right).abs() / half_x)
                        .max(along + rel.dot(view_up).abs() / half_y);
                }
            }
        }
        let distance = (distance * 1.05).max(f64::MIN_POSITIVE);
        distance.is_finite().then_some(Camera {
            eye: center + dir * distance,
            target: center,
            up,
            projection: Projection::Perspective { fov_y_deg },
        })
    }

    /// [`Camera::framing`], with the eye kept clear of `obstacle`: every
    /// corner of that box is in front of the eye, with the same 5 %
    /// margin. For a close-up of a part of a model (`bbox`) from outside
    /// the whole of it (`obstacle`); the renderer does not draw what is
    /// partly behind the eye.
    ///
    /// When the eye has to be further away than a field of view of
    /// `fov_y_deg` puts it, the field of view is narrowed until `bbox`
    /// fills the image again (a box with no size seen from that direction
    /// has nothing to fill it with, and keeps `fov_y_deg`). `None` as for
    /// `framing`.
    pub fn framing_clear_of(
        bbox: &Bbox3,
        obstacle: &Bbox3,
        toward_eye: Vec3,
        up: Vec3,
        fov_y_deg: f64,
        aspect: f64,
    ) -> Option<Camera> {
        let camera = Camera::framing(bbox, toward_eye, up, fov_y_deg, aspect)?;
        let Some((lo, hi)) = obstacle.corners() else {
            return Some(camera);
        };
        let dir = toward_eye.normalized()?;
        let center = camera.target;
        // How far towards the eye the obstacle reaches.
        let mut reach = 0.0_f64;
        for x in [lo.x, hi.x] {
            for y in [lo.y, hi.y] {
                for z in [lo.z, hi.z] {
                    reach = reach.max((Vec3::new(x, y, z) - center).dot(dir));
                }
            }
        }
        let distance = reach * 1.05;
        if !distance.is_finite() {
            return None;
        }
        if distance <= (camera.eye - center).dot(dir) {
            return Some(camera);
        }
        // From there, the half angles `bbox` needs: a corner at offset
        // `rel` is at depth `distance - rel·dir`.
        let right = up.cross(dir).normalized()?;
        let view_up = dir.cross(right);
        let (lo, hi) = bbox.corners()?;
        let mut half_y = 0.0_f64;
        for x in [lo.x, hi.x] {
            for y in [lo.y, hi.y] {
                for z in [lo.z, hi.z] {
                    let rel = Vec3::new(x, y, z) - center;
                    let depth = distance - rel.dot(dir);
                    if depth.is_nan() || depth <= 0.0 {
                        return None;
                    }
                    half_y = half_y
                        .max(rel.dot(view_up).abs() / depth)
                        .max(rel.dot(right).abs() / (depth * aspect));
                }
            }
        }
        let narrowed = if half_y > 0.0 {
            (half_y * 1.05).atan().to_degrees() * 2.0
        } else {
            fov_y_deg
        };
        (narrowed.is_finite() && narrowed > 0.0 && narrowed < 180.0).then_some(Camera {
            eye: center + dir * distance,
            target: center,
            up,
            projection: Projection::Perspective {
                fov_y_deg: narrowed,
            },
        })
    }

    /// An orthographic camera that looks at the centre of `bbox` from the
    /// direction `toward_eye` and shows every corner of the box with a
    /// 5 % margin: lengths across the view are to scale, whatever their
    /// depth. `None` for an empty box, a zero direction, an up vector
    /// parallel to it, a bad aspect ratio, or a box that has no size seen
    /// from that direction.
    pub fn framing_orthographic(
        bbox: &Bbox3,
        toward_eye: Vec3,
        up: Vec3,
        aspect: f64,
    ) -> Option<Camera> {
        let center = bbox.center()?;
        let (lo, hi) = bbox.corners()?;
        let dir = toward_eye.normalized()?;
        let right = up.cross(dir).normalized()?;
        let view_up = dir.cross(right);
        if !(aspect.is_finite() && aspect > 0.0) {
            return None;
        }
        let (mut half_height, mut reach) = (0.0_f64, 0.0_f64);
        for x in [lo.x, hi.x] {
            for y in [lo.y, hi.y] {
                for z in [lo.z, hi.z] {
                    let rel = Vec3::new(x, y, z) - center;
                    half_height = half_height
                        .max(rel.dot(view_up).abs())
                        .max(rel.dot(right).abs() / aspect);
                    reach = reach.max(rel.dot(dir));
                }
            }
        }
        let half_height = half_height * 1.05;
        // In front of the box; how far does not change the picture.
        let distance = reach * 1.05 + half_height;
        (half_height.is_finite() && half_height > 0.0 && distance.is_finite() && distance > 0.0)
            .then_some(Camera {
                eye: center + dir * distance,
                target: center,
                up,
                projection: Projection::Orthographic { half_height },
            })
    }

    /// The same camera zoomed in by `factor` about the centre of the
    /// image: what was a length `l` on the image plane through the target
    /// becomes `factor · l`, and the eye stays where it is, so nothing new
    /// comes between it and the model. A perspective camera narrows its
    /// field of view, an orthographic one its height. A factor below 1
    /// zooms out. `None` for a factor that is not a positive finite
    /// number, or one that leaves no valid camera.
    pub fn zoomed(&self, factor: f64) -> Option<Camera> {
        if !(factor.is_finite() && factor > 0.0) {
            return None;
        }
        let projection = match self.projection {
            Projection::Perspective { fov_y_deg } => {
                if !(fov_y_deg > 0.0 && fov_y_deg < 180.0) {
                    return None;
                }
                let half = (fov_y_deg.to_radians() * 0.5).tan() / factor;
                let fov_y_deg = half.atan().to_degrees() * 2.0;
                if !(fov_y_deg > 0.0 && fov_y_deg < 180.0) {
                    return None;
                }
                Projection::Perspective { fov_y_deg }
            }
            Projection::Orthographic { half_height } => {
                let half_height = half_height / factor;
                if !(half_height.is_finite() && half_height > 0.0) {
                    return None;
                }
                Projection::Orthographic { half_height }
            }
        };
        Some(Camera {
            projection,
            ..*self
        })
    }
}

/// Ambient term of the shading model: `intensity = AMBIENT + DIFFUSE * n·l`.
pub const SHADE_AMBIENT: f64 = 0.2;
/// Diffuse weight of the shading model.
pub const SHADE_DIFFUSE: f64 = 0.8;
/// Weight of the white specular highlight, as a fraction of 255.
pub const SHADE_SPECULAR: f64 = 0.35;
/// Exponent of the specular highlight.
pub const SHADE_SHININESS: i32 = 48;

/// A camera resolved to an orthonormal basis and a projection scale. Both
/// renderers take their projection from here, so there is one definition of
/// where a point lands: with `rel = p - eye` and `dist = rel·forward`, the
/// normalised image coordinates (x right, y up, both in -1..=1) are
/// `rel·right * scale / aspect` and `rel·up * scale`, where `scale` is
/// `1 / (dist * half)` in perspective and `1 / half` in orthographic.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewAxes {
    pub eye: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    pub forward: Vec3,
    pub projection: Projection,
    /// Model units per half image height at distance 1 (perspective) or in
    /// total (orthographic).
    pub half: f64,
}

impl ViewAxes {
    /// Unit vector towards the one light, which is fixed to the camera:
    /// above and to the left of the viewer.
    pub fn light(&self) -> Option<Vec3> {
        (self.up * 0.6 - self.right * 0.45 - self.forward).normalized()
    }
}

impl Camera {
    /// The camera's basis and projection scale, or
    /// [`RenderError::DegenerateCamera`].
    pub fn axes(&self) -> Result<ViewAxes, RenderError> {
        let forward = (self.target - self.eye)
            .normalized()
            .ok_or(RenderError::DegenerateCamera)?;
        let right = forward
            .cross(self.up)
            .normalized()
            .ok_or(RenderError::DegenerateCamera)?;
        let up = right.cross(forward);
        let half = match self.projection {
            Projection::Perspective { fov_y_deg } => {
                if !(fov_y_deg > 0.0 && fov_y_deg < 180.0) {
                    return Err(RenderError::DegenerateCamera);
                }
                (fov_y_deg.to_radians() * 0.5).tan()
            }
            Projection::Orthographic { half_height } => half_height,
        };
        if !(half.is_finite() && half > 0.0 && self.eye.is_finite()) {
            return Err(RenderError::DegenerateCamera);
        }
        Ok(ViewAxes {
            eye: self.eye,
            right,
            up,
            forward,
            projection: self.projection,
            half,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderParams {
    pub width: u32,
    pub height: u32,
    /// Supersampling per axis: 1 is one sample per pixel, 2 is four.
    pub ssaa: u32,
    pub background: [u8; 3],
    pub color: [u8; 3],
}

impl Default for RenderParams {
    fn default() -> Self {
        Self {
            width: 800,
            height: 600,
            ssaa: 2,
            background: [30, 33, 40],
            color: [198, 204, 214],
        }
    }
}

/// A rendered image with its id and depth buffers, all row-major from the
/// top-left pixel.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// 3 bytes per pixel.
    pub rgb: Vec<u8>,
    /// Face id at the pixel's first sample, or [`NO_FACE`].
    pub ids: Vec<u32>,
    /// Distance along the view direction at the pixel's first sample;
    /// infinity where nothing was drawn.
    pub depth: Vec<f64>,
}

impl Frame {
    /// Fraction of pixels covered by any face.
    pub fn coverage(&self) -> f64 {
        if self.ids.is_empty() {
            return 0.0;
        }
        let covered = self.ids.iter().filter(|&&id| id != NO_FACE).count();
        covered as f64 / self.ids.len() as f64
    }

    /// The face ids that are visible somewhere in the image.
    pub fn visible_faces(&self) -> BTreeSet<u32> {
        self.ids
            .iter()
            .copied()
            .filter(|&id| id != NO_FACE)
            .collect()
    }

    /// The id buffer as an image: one stable colour per face, black where
    /// there is none.
    pub fn false_color(&self) -> Vec<u8> {
        self.ids
            .iter()
            .flat_map(|&id| {
                if id == NO_FACE {
                    [0, 0, 0]
                } else {
                    // Knuth's multiplicative hash spreads consecutive ids
                    // over the colour space; the floor keeps them visible.
                    let h = id.wrapping_add(1).wrapping_mul(2_654_435_761);
                    let [a, b, c, _] = h.to_le_bytes();
                    [a | 0x40, b | 0x40, c | 0x40]
                }
            })
            .collect()
    }

    pub fn to_png(&self) -> Result<Vec<u8>, RenderError> {
        encode_png(self.width, self.height, &self.rgb)
    }
}

/// Encodes 8-bit RGB pixels as a PNG file.
pub fn encode_png(width: u32, height: u32, rgb: &[u8]) -> Result<Vec<u8>, RenderError> {
    let expected = u64::from(width) * u64::from(height) * 3;
    if rgb.len() as u64 != expected {
        return Err(RenderError::Png(format!(
            "{} bytes of pixel data for a {width}x{height} image, expected {expected}",
            rgb.len()
        )));
    }
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|e| RenderError::Png(e.to_string()))?;
    writer
        .write_image_data(rgb)
        .map_err(|e| RenderError::Png(e.to_string()))?;
    writer
        .finish()
        .map_err(|e| RenderError::Png(e.to_string()))?;
    Ok(out)
}

/// A vertex after projection.
#[derive(Clone, Copy)]
struct Projected {
    /// Sample coordinates (x right, y down).
    sx: f64,
    sy: f64,
    /// Distance along the view direction.
    dist: f64,
    /// Depth key that is affine in screen space: smaller is nearer.
    key: f64,
    /// 1 / w for perspective-correct interpolation (1 for orthographic).
    inv_w: f64,
}

struct View {
    eye: Vec3,
    right: Vec3,
    up: Vec3,
    forward: Vec3,
    projection: Projection,
    /// Model units per half image height at distance 1 (perspective) or in
    /// total (orthographic).
    half: f64,
    /// Unit vector towards the light ([`ViewAxes::light`]).
    light: Option<Vec3>,
    aspect: f64,
    w: f64,
    h: f64,
}

impl View {
    fn new(camera: &Camera, w: usize, h: usize) -> Result<Self, RenderError> {
        let axes = camera.axes()?;
        Ok(Self {
            eye: axes.eye,
            right: axes.right,
            up: axes.up,
            forward: axes.forward,
            projection: axes.projection,
            half: axes.half,
            light: axes.light(),
            aspect: w as f64 / h as f64,
            w: w as f64,
            h: h as f64,
        })
    }

    /// `None` for a point at or behind the eye plane (perspective) or a
    /// non-finite point.
    fn project(&self, p: Vec3) -> Option<Projected> {
        let rel = p - self.eye;
        let (cx, cy, dist) = (rel.dot(self.right), rel.dot(self.up), rel.dot(self.forward));
        let (scale, key, inv_w) = match self.projection {
            Projection::Perspective { .. } => {
                if dist.is_nan() || dist <= 1e-12 {
                    return None;
                }
                (1.0 / (dist * self.half), -1.0 / dist, 1.0 / dist)
            }
            Projection::Orthographic { .. } => (1.0 / self.half, dist, 1.0),
        };
        let sx = (cx * scale / self.aspect * 0.5 + 0.5) * self.w;
        let sy = (0.5 - cy * scale * 0.5) * self.h;
        (sx.is_finite() && sy.is_finite() && key.is_finite()).then_some(Projected {
            sx,
            sy,
            dist,
            key,
            inv_w,
        })
    }

    /// Unit vector from a surface point towards the viewer.
    fn toward_viewer(&self, p: Vec3) -> Vec3 {
        match self.projection {
            Projection::Perspective { .. } => (self.eye - p).normalized().unwrap_or(-self.forward),
            Projection::Orthographic { .. } => -self.forward,
        }
    }
}

/// Twice the signed area of triangle (a, b, p) in sample space.
fn edge(ax: f64, ay: f64, bx: f64, by: f64, px: f64, py: f64) -> f64 {
    (bx - ax) * (py - ay) - (by - ay) * (px - ax)
}

/// The edge function of `p → q` at a sample, computed so that the two
/// triangles sharing the edge get exactly opposite values: the endpoints are
/// always taken in the same order and the sign is flipped afterwards. A
/// sample is then inside one of the two triangles, or on the edge and inside
/// both, never in neither.
fn shared_edge(p: &Projected, q: &Projected, px: f64, py: f64) -> f64 {
    if (p.sx, p.sy) <= (q.sx, q.sy) {
        edge(p.sx, p.sy, q.sx, q.sy, px, py)
    } else {
        -edge(q.sx, q.sy, p.sx, p.sy, px, py)
    }
}

/// First and last sample index along one axis that a span `[lo, hi]` can
/// touch, or `None` when it misses `0..len` entirely.
fn sample_range(lo: f64, hi: f64, len: usize) -> Option<(usize, usize)> {
    let last = len.checked_sub(1)? as f64;
    if !(lo.is_finite() && hi.is_finite()) || hi < 0.0 || lo > last + 1.0 {
        return None;
    }
    // Clamped to the buffer, so the casts cannot leave 0..len.
    let first = (lo - 0.5).ceil().clamp(0.0, last) as usize;
    let end = (hi - 0.5).floor().clamp(0.0, last) as usize;
    (first <= end).then_some((first, end))
}

fn shade(base: [u8; 3], normal: Vec3, to_viewer: Vec3, view: &View) -> [u8; 3] {
    // Two-sided: sheets are seen from both sides (the inside of the spout).
    let n = if normal.dot(to_viewer) < 0.0 {
        -normal
    } else {
        normal
    };
    // One light fixed to the camera, above and to the left of the viewer.
    let light = view.light.unwrap_or(to_viewer);
    let diffuse = n.dot(light).max(0.0);
    let specular = (light + to_viewer)
        .normalized()
        .map_or(0.0, |half| n.dot(half).max(0.0).powi(SHADE_SHININESS));
    let intensity = SHADE_AMBIENT + SHADE_DIFFUSE * diffuse;
    // Clamped to 0..=255 before the cast.
    base.map(|c| {
        (f64::from(c) * intensity + 255.0 * SHADE_SPECULAR * specular)
            .round()
            .clamp(0.0, 255.0) as u8
    })
}

/// Renders `mesh` as seen from `camera`.
///
/// Triangles with a vertex at or behind the eye plane are skipped (there is
/// no near-plane clipping yet); put the camera outside the model.
pub fn render(mesh: &Mesh, camera: &Camera, params: &RenderParams) -> Result<Frame, RenderError> {
    let (width, height, ssaa) = (params.width, params.height, params.ssaa);
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return Err(RenderError::InvalidSize { width, height });
    }
    if ssaa == 0 || ssaa > MAX_SSAA {
        return Err(RenderError::InvalidSsaa(ssaa));
    }
    let samples = u64::from(width) * u64::from(height) * u64::from(ssaa) * u64::from(ssaa);
    if samples > MAX_SAMPLES {
        return Err(RenderError::TooManySamples { samples });
    }
    if mesh.normals.len() != mesh.positions.len() || mesh.face_ids.len() != mesh.triangles.len() {
        return Err(RenderError::InvalidMesh { triangle: 0 });
    }
    // Bounded by MAX_SIDE * MAX_SSAA and MAX_SAMPLES above.
    let s = ssaa as usize;
    let (w, h) = (width as usize * s, height as usize * s);
    let view = View::new(camera, w, h)?;

    let projected: Vec<Option<Projected>> =
        mesh.positions.iter().map(|&p| view.project(p)).collect();
    let mut color = vec![params.background; w * h];
    let mut ids = vec![NO_FACE; w * h];
    let mut depth_key = vec![f64::INFINITY; w * h];
    let mut depth = vec![f64::INFINITY; w * h];

    for (t, (tri, &face)) in mesh.triangles.iter().zip(&mesh.face_ids).enumerate() {
        let vertex = |k: u32| {
            let k = k as usize;
            match (mesh.positions.get(k), mesh.normals.get(k), projected.get(k)) {
                (Some(&p), Some(&n), Some(&proj)) => Ok((p, n, proj)),
                _ => Err(RenderError::InvalidMesh { triangle: t }),
            }
        };
        let [ia, ib, ic] = *tri;
        let [(pa, na, a), (pb, nb, b), (pc, nc, c)] = [vertex(ia)?, vertex(ib)?, vertex(ic)?];
        let (Some(a), Some(b), Some(c)) = (a, b, c) else {
            continue;
        };
        let area = edge(a.sx, a.sy, b.sx, b.sy, c.sx, c.sy);
        if !area.is_finite() || area == 0.0 {
            continue;
        }
        let Some((x0, x1)) = sample_range(a.sx.min(b.sx).min(c.sx), a.sx.max(b.sx).max(c.sx), w)
        else {
            continue;
        };
        let Some((y0, y1)) = sample_range(a.sy.min(b.sy).min(c.sy), a.sy.max(b.sy).max(c.sy), h)
        else {
            continue;
        };
        let flat_normal = (pb - pa).cross(pc - pa).normalized();
        for y in y0..=y1 {
            let py = y as f64 + 0.5;
            for x in x0..=x1 {
                let px = x as f64 + 0.5;
                // Barycentric weights; dividing by the signed area makes
                // them positive inside for either winding.
                let wa = shared_edge(&b, &c, px, py) / area;
                let wb = shared_edge(&c, &a, px, py) / area;
                let wc = shared_edge(&a, &b, px, py) / area;
                if wa < 0.0 || wb < 0.0 || wc < 0.0 {
                    continue;
                }
                let sum = wa + wb + wc;
                let (wa, wb, wc) = (wa / sum, wb / sum, wc / sum);
                let key = wa * a.key + wb * b.key + wc * c.key;
                let i = y * w + x;
                let Some(slot) = depth_key.get_mut(i) else {
                    continue;
                };
                // Strict: on a tie the triangle drawn first wins, which
                // keeps the result independent of anything but mesh order.
                if key >= *slot {
                    continue;
                }
                *slot = key;
                // Perspective-correct weights for attributes.
                let (qa, qb, qc) = (wa * a.inv_w, wb * b.inv_w, wc * c.inv_w);
                let q = qa + qb + qc;
                let (qa, qb, qc) = (qa / q, qb / q, qc / q);
                let position = pa * qa + pb * qb + pc * qc;
                let normal = (na * qa + nb * qb + nc * qc).normalized().or(flat_normal);
                let to_viewer = view.toward_viewer(position);
                let rgb = normal.map_or(params.color, |n| shade(params.color, n, to_viewer, &view));
                if let (Some(c_slot), Some(id_slot), Some(d_slot)) =
                    (color.get_mut(i), ids.get_mut(i), depth.get_mut(i))
                {
                    *c_slot = rgb;
                    *id_slot = face;
                    *d_slot = wa * a.dist + wb * b.dist + wc * c.dist;
                }
            }
        }
    }

    // Box-filter the colour down; ids and depth come from each pixel's first
    // sample (they cannot be averaged).
    let (ow, oh) = (width as usize, height as usize);
    let mut frame = Frame {
        width,
        height,
        rgb: Vec::with_capacity(ow * oh * 3),
        ids: Vec::with_capacity(ow * oh),
        depth: Vec::with_capacity(ow * oh),
    };
    let per_pixel = (s * s) as u32;
    for oy in 0..oh {
        for ox in 0..ow {
            let mut sum = [0_u32; 3];
            for dy in 0..s {
                let start = (oy * s + dy) * w + ox * s;
                for px in color.get(start..start + s).into_iter().flatten() {
                    for (acc, &c) in sum.iter_mut().zip(px) {
                        *acc += u32::from(c);
                    }
                }
            }
            // A rounded average of u8 values fits in u8.
            frame
                .rgb
                .extend(sum.map(|v| ((v + per_pixel / 2) / per_pixel).min(255) as u8));
            let first = oy * s * w + ox * s;
            frame.ids.push(ids.get(first).copied().unwrap_or(NO_FACE));
            frame
                .depth
                .push(depth.get(first).copied().unwrap_or(f64::INFINITY));
        }
    }
    Ok(frame)
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_tess::{TessParams, tessellate_patches};
    use craft3d_testkit::teapot::{PATCH_COUNT, teapot};

    fn quad(corners: [Vec3; 4], face: u32, mesh: &mut Mesh) {
        let base = mesh.positions.len() as u32;
        mesh.positions.extend(corners);
        mesh.normals.extend([Vec3::ZERO; 4]);
        mesh.triangles.push([base, base + 1, base + 2]);
        mesh.triangles.push([base, base + 2, base + 3]);
        mesh.face_ids.extend([face, face]);
    }

    fn unit_square(z: f64, face: u32, mesh: &mut Mesh) {
        quad(
            [
                Vec3::new(0.0, 0.0, z),
                Vec3::new(1.0, 0.0, z),
                Vec3::new(1.0, 1.0, z),
                Vec3::new(0.0, 1.0, z),
            ],
            face,
            mesh,
        );
    }

    fn top_down(half_height: f64) -> Camera {
        Camera {
            eye: Vec3::new(0.5, 0.5, 5.0),
            target: Vec3::new(0.5, 0.5, 0.0),
            up: Vec3::Y,
            projection: Projection::Orthographic { half_height },
        }
    }

    fn params(width: u32, height: u32, ssaa: u32) -> RenderParams {
        RenderParams {
            width,
            height,
            ssaa,
            ..RenderParams::default()
        }
    }

    #[test]
    fn orthographic_square_covers_exactly_its_area() {
        let mut mesh = Mesh::default();
        unit_square(0.0, 7, &mut mesh);
        // 2 model units over 100 pixels: the unit square is 50 x 50 pixels.
        let frame = render(&mesh, &top_down(1.0), &params(100, 100, 1)).unwrap();
        assert_eq!(frame.ids.iter().filter(|&&id| id == 7).count(), 2500);
        assert_eq!(frame.coverage(), 0.25);
        // x right, y up in the image: the square occupies columns and rows 25..75.
        assert_eq!(frame.ids[25 * 100 + 25], 7);
        assert_eq!(frame.ids[74 * 100 + 74], 7);
        assert_eq!(frame.ids[24 * 100 + 25], NO_FACE);
        assert_eq!(frame.ids[25 * 100 + 75], NO_FACE);
        assert_eq!(frame.depth[50 * 100 + 50], 5.0);
        assert!(frame.depth[0].is_infinite());
        // Lit pixels are brighter than the background, unlit ones are it.
        assert_eq!(&frame.rgb[0..3], &RenderParams::default().background);
        assert!(frame.rgb[(50 * 100 + 50) * 3] > 100);
    }

    #[test]
    fn image_axes_follow_the_camera() {
        // A small square in the +x, +y corner of the view must land top-right.
        let mut mesh = Mesh::default();
        quad(
            [
                Vec3::new(1.0, 1.0, 0.0),
                Vec3::new(1.4, 1.0, 0.0),
                Vec3::new(1.4, 1.4, 0.0),
                Vec3::new(1.0, 1.4, 0.0),
            ],
            1,
            &mut mesh,
        );
        let frame = render(&mesh, &top_down(1.0), &params(100, 100, 1)).unwrap();
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

    #[test]
    fn the_nearer_face_wins_whatever_the_draw_order() {
        for near_first in [true, false] {
            let mut mesh = Mesh::default();
            if near_first {
                unit_square(1.0, 1, &mut mesh);
                unit_square(0.0, 2, &mut mesh);
            } else {
                unit_square(0.0, 2, &mut mesh);
                unit_square(1.0, 1, &mut mesh);
            }
            for projection in [
                Projection::Orthographic { half_height: 1.0 },
                Projection::Perspective { fov_y_deg: 40.0 },
            ] {
                let camera = Camera {
                    projection,
                    ..top_down(1.0)
                };
                let frame = render(&mesh, &camera, &params(64, 64, 1)).unwrap();
                assert_eq!(frame.ids[32 * 64 + 32], 1);
                assert_eq!(frame.depth[32 * 64 + 32], 4.0);
            }
        }
    }

    #[test]
    fn perspective_shrinks_with_distance() {
        let covered = |z: f64| {
            let mut mesh = Mesh::default();
            unit_square(z, 0, &mut mesh);
            let camera = Camera {
                // tan(45°) = 1: at distance d the 200-pixel image shows 2d
                // model units, so the square's edges fall on pixel borders.
                projection: Projection::Perspective { fov_y_deg: 90.0 },
                ..top_down(1.0)
            };
            let frame = render(&mesh, &camera, &params(200, 200, 1)).unwrap();
            frame.ids.iter().filter(|&&id| id == 0).count()
        };
        // Distance 2.5: 40 x 40 pixels. Twice the distance: a quarter of the area.
        assert_eq!(covered(2.5), 1600);
        assert_eq!(covered(0.0), 400);
    }

    #[test]
    fn framing_keeps_the_whole_box_in_view() {
        let bbox = Bbox3::from_points([Vec3::new(-3.0, -2.0, 0.0), Vec3::new(3.5, 2.0, 3.2)]);
        let (lo, hi) = bbox.corners().unwrap();
        for (dir, aspect) in [
            (Vec3::new(0.35, -1.0, 0.45), 1.6),
            (Vec3::new(1.0, 0.2, 0.1), 0.5),
            (Vec3::new(-0.3, 0.4, 2.0), 1.0),
        ] {
            let camera = Camera::framing(&bbox, dir, Vec3::Z, 30.0, aspect).unwrap();
            let (w, h) = ((400.0 * aspect) as usize, 400);
            let view = View::new(&camera, w, h).unwrap();
            let mut touches_margin = false;
            for x in [lo.x, hi.x] {
                for y in [lo.y, hi.y] {
                    for z in [lo.z, hi.z] {
                        let p = view.project(Vec3::new(x, y, z)).unwrap();
                        assert!(p.sx >= 0.0 && p.sx <= w as f64 && p.sy >= 0.0 && p.sy <= h as f64);
                        // Tight: some corner is within 6 % of an image edge.
                        let near =
                            |v: f64, len: usize| v < 0.06 * len as f64 || v > 0.94 * len as f64;
                        touches_margin |= near(p.sx, w) || near(p.sy, h);
                    }
                }
            }
            assert!(touches_margin, "framing from {dir:?} leaves the box small");
        }
        assert!(Camera::framing(&Bbox3::EMPTY, Vec3::X, Vec3::Z, 30.0, 1.0).is_none());
        assert!(Camera::framing(&bbox, Vec3::ZERO, Vec3::Z, 30.0, 1.0).is_none());
        assert!(Camera::framing(&bbox, Vec3::Z, Vec3::Z, 30.0, 1.0).is_none());
        assert!(Camera::framing(&bbox, Vec3::X, Vec3::Z, 0.0, 1.0).is_none());
        assert!(Camera::framing(&bbox, Vec3::X, Vec3::Z, 30.0, f64::NAN).is_none());
    }

    /// Triangles that share an edge must leave no uncovered sample between
    /// them, whatever the coordinates.
    #[test]
    fn a_closed_surface_has_no_pinholes() {
        // A jittered grid that overhangs the view on every side.
        let n = 60_usize;
        let mut seed = 0x2545_F491_4F6C_DD1D_u64;
        let mut jitter = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f64 / (1u64 << 53) as f64 - 0.5
        };
        let mut mesh = Mesh::default();
        for i in 0..=n {
            for j in 0..=n {
                let (x, y) = (
                    i as f64 / n as f64 * 3.0 - 1.0,
                    j as f64 / n as f64 * 3.0 - 1.0,
                );
                let step = 3.0 / n as f64;
                mesh.positions.push(Vec3::new(
                    x + jitter() * step * 0.8,
                    y + jitter() * step * 0.8,
                    jitter(),
                ));
                mesh.normals.push(Vec3::ZERO);
            }
        }
        let side = (n + 1) as u32;
        for i in 0..n as u32 {
            for j in 0..n as u32 {
                let a = i * side + j;
                mesh.triangles.push([a, a + side, a + side + 1]);
                mesh.triangles.push([a, a + side + 1, a + 1]);
                mesh.face_ids.extend([0, 0]);
            }
        }
        for projection in [
            Projection::Orthographic { half_height: 0.5 },
            Projection::Perspective { fov_y_deg: 11.0 },
        ] {
            let camera = Camera {
                projection,
                ..top_down(1.0)
            };
            let frame = render(&mesh, &camera, &params(700, 700, 2)).unwrap();
            let holes = frame.ids.iter().filter(|&&id| id == NO_FACE).count();
            assert_eq!(holes, 0, "{projection:?}");
        }
    }

    #[test]
    fn supersampling_softens_edges_only() {
        let mut mesh = Mesh::default();
        // Offset so the edges fall inside pixels.
        quad(
            [
                Vec3::new(0.013, 0.013, 0.0),
                Vec3::new(0.987, 0.013, 0.0),
                Vec3::new(0.987, 0.987, 0.0),
                Vec3::new(0.013, 0.987, 0.0),
            ],
            0,
            &mut mesh,
        );
        let aliased = render(&mesh, &top_down(1.0), &params(100, 100, 1)).unwrap();
        let smooth = render(&mesh, &top_down(1.0), &params(100, 100, 4)).unwrap();
        let distinct = |f: &Frame| f.rgb.chunks(3).collect::<BTreeSet<_>>().len();
        assert_eq!(distinct(&aliased), 2);
        assert!(distinct(&smooth) > 2);
        let centre = (50 * 100 + 50) * 3;
        assert_eq!(
            aliased.rgb[centre..centre + 3],
            smooth.rgb[centre..centre + 3]
        );
    }

    #[test]
    fn bad_input_is_an_error_not_a_panic() {
        let mut mesh = Mesh::default();
        unit_square(0.0, 0, &mut mesh);
        let cam = top_down(1.0);
        assert!(matches!(
            render(&mesh, &cam, &params(0, 10, 1)),
            Err(RenderError::InvalidSize { .. })
        ));
        assert!(matches!(
            render(&mesh, &cam, &params(MAX_SIDE + 1, 10, 1)),
            Err(RenderError::InvalidSize { .. })
        ));
        assert!(matches!(
            render(&mesh, &cam, &params(10, 10, 0)),
            Err(RenderError::InvalidSsaa(0))
        ));
        assert!(matches!(
            render(&mesh, &cam, &params(10, 10, 9)),
            Err(RenderError::InvalidSsaa(9))
        ));
        assert!(matches!(
            render(&mesh, &cam, &params(MAX_SIDE, MAX_SIDE, 4)),
            Err(RenderError::TooManySamples { .. })
        ));
        for bad in [
            Camera {
                target: cam.eye,
                ..cam
            },
            Camera { up: Vec3::Z, ..cam },
            Camera {
                projection: Projection::Orthographic { half_height: 0.0 },
                ..cam
            },
            Camera {
                projection: Projection::Perspective { fov_y_deg: 180.0 },
                ..cam
            },
            Camera {
                projection: Projection::Perspective {
                    fov_y_deg: f64::NAN,
                },
                ..cam
            },
            Camera {
                eye: Vec3::new(f64::NAN, 0.0, 0.0),
                ..cam
            },
        ] {
            assert_eq!(
                render(&mesh, &bad, &params(10, 10, 1)),
                Err(RenderError::DegenerateCamera)
            );
        }
        let mut broken = mesh.clone();
        broken.triangles[1] = [0, 1, 99];
        assert_eq!(
            render(&broken, &cam, &params(10, 10, 1)),
            Err(RenderError::InvalidMesh { triangle: 1 })
        );
        let mut mismatched = mesh.clone();
        mismatched.normals.pop();
        assert!(matches!(
            render(&mismatched, &cam, &params(10, 10, 1)),
            Err(RenderError::InvalidMesh { .. })
        ));
        // Hostile coordinates and a camera inside the geometry draw nothing or something, never panic.
        let mut wild = Mesh::default();
        quad(
            [
                Vec3::new(-1e300, -1e300, 0.0),
                Vec3::new(1e300, -1e300, 0.0),
                Vec3::new(f64::NAN, 1e300, 0.0),
                Vec3::new(0.0, 0.0, 10.0),
            ],
            0,
            &mut wild,
        );
        for projection in [
            Projection::Orthographic { half_height: 1.0 },
            Projection::Perspective { fov_y_deg: 50.0 },
        ] {
            let camera = Camera { projection, ..cam };
            assert!(render(&wild, &camera, &params(16, 16, 2)).is_ok());
        }
        assert_eq!(
            render(&Mesh::default(), &cam, &params(4, 4, 1))
                .unwrap()
                .coverage(),
            0.0
        );
    }

    #[test]
    fn png_round_trips() {
        let mut mesh = Mesh::default();
        unit_square(0.0, 3, &mut mesh);
        let frame = render(&mesh, &top_down(1.0), &params(40, 30, 2)).unwrap();
        let bytes = frame.to_png().unwrap();
        let decoder = png::Decoder::new(std::io::Cursor::new(&bytes));
        let mut reader = decoder.read_info().unwrap();
        let mut pixels = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut pixels).unwrap();
        assert_eq!((info.width, info.height), (40, 30));
        assert_eq!(info.color_type, png::ColorType::Rgb);
        assert_eq!(&pixels[..info.buffer_size()], &frame.rgb[..]);
        assert!(matches!(
            encode_png(4, 4, &[0; 5]),
            Err(RenderError::Png(_))
        ));
        // The id image has black for no face and one colour for face 3.
        let ids = frame.false_color();
        assert_eq!(ids.len(), frame.rgb.len());
        assert_eq!(ids.chunks(3).collect::<BTreeSet<_>>().len(), 2);
    }

    fn teapot_frame(width: u32, height: u32) -> Frame {
        let mesh = tessellate_patches(&teapot().unwrap(), &TessParams::default()).unwrap();
        let aspect = f64::from(width) / f64::from(height);
        let camera = Camera::framing(
            &mesh.bbox(),
            Vec3::new(0.35, -1.0, 0.45),
            Vec3::Z,
            30.0,
            aspect,
        )
        .unwrap();
        render(&mesh, &camera, &params(width, height, 2)).unwrap()
    }

    #[test]
    fn teapot_renders_whole_and_deterministically() {
        let frame = teapot_frame(320, 240);
        assert_eq!(
            frame,
            teapot_frame(320, 240),
            "two runs must be bit-identical"
        );
        // Framed with a margin: nothing touches the image border.
        let (w, h) = (320, 240);
        for i in 0..w * h {
            let (x, y) = (i % w, i / w);
            if x == 0 || y == 0 || x == w - 1 || y == h - 1 {
                assert_eq!(
                    frame.ids[i], NO_FACE,
                    "teapot touches the border at ({x}, {y})"
                );
            }
        }
        let coverage = frame.coverage();
        assert!((0.15..0.6).contains(&coverage), "coverage {coverage}");
        // From the front-right and above: everything but the far side.
        let visible = frame.visible_faces();
        assert!(visible.iter().all(|&id| (id as usize) < PATCH_COUNT));
        assert!(
            visible.len() >= 20,
            "only {} patches visible",
            visible.len()
        );
        for (name, range) in [("lid", 12..20), ("handle", 24..28), ("spout", 28..32)] {
            assert!(
                range.clone().any(|id| visible.contains(&id)),
                "no {name} patch visible"
            );
        }
        // Spout (+x) is on the right half of the image, handle on the left.
        let mean_x = |ids: std::ops::Range<u32>| {
            let xs: Vec<f64> = (0..w * h)
                .filter(|&i| ids.contains(&frame.ids[i]))
                .map(|i| (i % w) as f64)
                .collect();
            xs.iter().sum::<f64>() / xs.len() as f64
        };
        assert!(mean_x(28..32) > 160.0 && mean_x(24..28) < 160.0);
    }

    /// Where the corners of `bbox` land, as fractions of the half image:
    /// (largest |x|, largest |y|), 1 being the image's edge.
    fn extent(camera: &Camera, bbox: &Bbox3, aspect: f64) -> (f64, f64) {
        let (w, h) = ((400.0 * aspect) as usize, 400);
        let view = View::new(camera, w, h).unwrap();
        let (lo, hi) = bbox.corners().unwrap();
        let (mut ex, mut ey) = (0.0_f64, 0.0_f64);
        for x in [lo.x, hi.x] {
            for y in [lo.y, hi.y] {
                for z in [lo.z, hi.z] {
                    let p = view.project(Vec3::new(x, y, z)).unwrap();
                    ex = ex.max((p.sx / w as f64 * 2.0 - 1.0).abs());
                    ey = ey.max((p.sy / h as f64 * 2.0 - 1.0).abs());
                }
            }
        }
        (ex, ey)
    }

    #[test]
    fn orthographic_framing_shows_the_box_to_scale_with_its_margin() {
        for scale in [1e-3, 1.0, 1e6] {
            let bbox = Bbox3::from_points([
                Vec3::new(-3.0, -2.0, 0.0) * scale,
                Vec3::new(3.5, 2.0, 3.2) * scale,
            ]);
            for (dir, up, aspect) in [
                (Vec3::new(0.35, -1.0, 0.45), Vec3::Z, 1.6),
                (Vec3::new(1.0, 0.2, 0.1), Vec3::Z, 0.5),
                (Vec3::new(0.0, 0.0, 1.0), Vec3::Y, 1.0),
                (Vec3::new(0.0, -1.0, 0.0), Vec3::Z, 2.5),
            ] {
                let camera = Camera::framing_orthographic(&bbox, dir, up, aspect).unwrap();
                assert!(matches!(camera.projection, Projection::Orthographic { .. }));
                // The widest corner is exactly at 1 / 1.05 of the half
                // image, on the axis that decides, and none is outside.
                let (ex, ey) = extent(&camera, &bbox, aspect);
                let widest = ex.max(ey);
                assert!(
                    (widest - 1.0 / 1.05).abs() < 1e-9,
                    "scale {scale}, from {dir:?}: {ex} x {ey}"
                );
                // The eye is in front of every corner.
                let forward = (camera.target - camera.eye).normalized().unwrap();
                let (lo, hi) = bbox.corners().unwrap();
                for corner in [
                    lo,
                    hi,
                    Vec3::new(lo.x, hi.y, lo.z),
                    Vec3::new(hi.x, lo.y, hi.z),
                ] {
                    assert!((corner - camera.eye).dot(forward) > 0.0);
                }
            }
            // Straight from the top, a box of 6.5 by 4 in a square image:
            // the half height is half its width, plus the margin.
            let camera = Camera::framing_orthographic(&bbox, Vec3::Z, Vec3::Y, 1.0).unwrap();
            let Projection::Orthographic { half_height } = camera.projection else {
                panic!("not orthographic");
            };
            assert!((half_height - 3.25 * 1.05 * scale).abs() < 1e-9 * scale);
        }
        let bbox = Bbox3::from_points([Vec3::ZERO, Vec3::new(1.0, 2.0, 3.0)]);
        let none = |bbox: &Bbox3, dir: Vec3, up: Vec3, aspect: f64| {
            Camera::framing_orthographic(bbox, dir, up, aspect).is_none()
        };
        assert!(none(&Bbox3::EMPTY, Vec3::X, Vec3::Z, 1.0));
        assert!(none(&bbox, Vec3::ZERO, Vec3::Z, 1.0));
        assert!(none(&bbox, Vec3::Z, Vec3::Z, 1.0));
        for aspect in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(none(&bbox, Vec3::X, Vec3::Z, aspect));
        }
        // A box that is a line along the view direction shows no size.
        let line = Bbox3::from_points([Vec3::ZERO, Vec3::new(5.0, 0.0, 0.0)]);
        assert!(none(&line, Vec3::X, Vec3::Z, 1.0));
        assert!(!none(&line, Vec3::Y, Vec3::Z, 1.0));
    }

    #[test]
    fn framing_clear_of_keeps_the_eye_outside_the_model_and_the_box_framed() {
        let detail = Bbox3::from_points([Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0)]);
        for (dir, up, aspect) in [
            (Vec3::new(0.0, -1.0, 0.0), Vec3::Z, 1.0),
            (Vec3::new(1.0, -1.0, 0.8), Vec3::Z, 1.5),
            (Vec3::new(0.0, 0.0, 1.0), Vec3::Y, 0.7),
        ] {
            let plain = Camera::framing(&detail, dir, up, 30.0, aspect).unwrap();
            // A model no larger than the detail changes nothing.
            let same = Camera::framing_clear_of(&detail, &detail, dir, up, 30.0, aspect).unwrap();
            assert_eq!(same, plain);
            assert_eq!(
                Camera::framing_clear_of(&detail, &Bbox3::EMPTY, dir, up, 30.0, aspect),
                Some(plain)
            );
            // A model that reaches far towards the eye pushes the eye
            // back beyond it, and the detail still fills the image.
            let unit = dir.normalized().unwrap();
            let mut model = detail;
            model.add(unit * 500.0);
            model.add(unit * -40.0);
            let camera = Camera::framing_clear_of(&detail, &model, dir, up, 30.0, aspect).unwrap();
            assert_eq!(camera.target, plain.target);
            let forward = (camera.target - camera.eye).normalized().unwrap();
            let (lo, hi) = model.corners().unwrap();
            for x in [lo.x, hi.x] {
                for y in [lo.y, hi.y] {
                    for z in [lo.z, hi.z] {
                        let depth = (Vec3::new(x, y, z) - camera.eye).dot(forward);
                        assert!(depth > 0.0, "a corner of the model is behind the eye");
                    }
                }
            }
            let Projection::Perspective { fov_y_deg } = camera.projection else {
                panic!("not perspective");
            };
            assert!(fov_y_deg < 30.0 && fov_y_deg > 0.0, "{fov_y_deg}");
            let (ex, ey) = extent(&camera, &detail, aspect);
            assert!(
                (ex.max(ey) - 1.0 / 1.05).abs() < 1e-9,
                "from {dir:?}: {ex} x {ey}"
            );
        }
        assert!(
            Camera::framing_clear_of(&Bbox3::EMPTY, &detail, Vec3::X, Vec3::Z, 30.0, 1.0).is_none()
        );
        assert!(Camera::framing_clear_of(&detail, &detail, Vec3::Z, Vec3::Z, 30.0, 1.0).is_none());
        // A box that is a line along the view direction has nothing to
        // fill the image with: the field of view stays.
        let line = Bbox3::from_points([Vec3::ZERO, Vec3::new(0.0, 0.0, 2.0)]);
        let mut model = line;
        model.add(Vec3::new(0.0, 0.0, 90.0));
        let camera = Camera::framing_clear_of(&line, &model, Vec3::Z, Vec3::Y, 30.0, 1.0).unwrap();
        assert_eq!(
            camera.projection,
            Projection::Perspective { fov_y_deg: 30.0 }
        );
        assert!(camera.eye.z > 90.0);
        let mut endless = detail;
        endless.add(Vec3::new(f64::MAX, 0.0, 0.0));
        assert!(Camera::framing_clear_of(&detail, &endless, Vec3::X, Vec3::Z, 30.0, 1.0).is_none());
    }

    #[test]
    fn zooming_magnifies_about_the_centre_and_keeps_the_eye() {
        let bbox = Bbox3::from_points([Vec3::new(-3.0, -2.0, 0.0), Vec3::new(3.5, 2.0, 3.2)]);
        let dir = Vec3::new(1.0, -1.0, 0.8);
        let cameras = [
            Camera::framing(&bbox, dir, Vec3::Z, 30.0, 1.5).unwrap(),
            Camera::framing_orthographic(&bbox, dir, Vec3::Z, 1.5).unwrap(),
        ];
        for camera in cameras {
            assert_eq!(camera.zoomed(1.0).map(|c| c.eye), Some(camera.eye));
            let (w, h) = (600, 400);
            let before = View::new(&camera, w, h).unwrap();
            // A point in the image plane through the target, off centre.
            let p = camera.target + before.right * 0.4 + before.up * -0.25;
            let at = |view: &View| {
                let q = view.project(p).unwrap();
                (q.sx - w as f64 / 2.0, q.sy - h as f64 / 2.0)
            };
            let (x0, y0) = at(&before);
            for factor in [0.25, 0.5, 2.0, 7.5, 1000.0] {
                let zoomed = camera.zoomed(factor).unwrap();
                assert_eq!(
                    (zoomed.eye, zoomed.target, zoomed.up),
                    (camera.eye, camera.target, camera.up)
                );
                let (x, y) = at(&View::new(&zoomed, w, h).unwrap());
                assert!(
                    (x - x0 * factor).abs() < 1e-9 * factor.max(1.0)
                        && (y - y0 * factor).abs() < 1e-9 * factor.max(1.0),
                    "zoom {factor}: ({x}, {y}) from ({x0}, {y0})"
                );
            }
            // Twice in a row is the product.
            let twice = camera.zoomed(2.0).unwrap().zoomed(3.0).unwrap();
            let (x, _) = at(&View::new(&twice, w, h).unwrap());
            assert!((x - x0 * 6.0).abs() < 1e-9);
            for bad in [0.0, -2.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
                assert!(camera.zoomed(bad).is_none(), "{bad}");
            }
            // So little zoom that nothing is left of the camera: refused.
            assert!(camera.zoomed(1e-320).is_none());
        }
        let broken = Camera {
            projection: Projection::Perspective { fov_y_deg: 200.0 },
            ..cameras[0]
        };
        assert!(broken.zoomed(2.0).is_none());
    }
}
