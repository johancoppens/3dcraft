//! Layer 2 of the studio look: screen-space reflections and the ground's
//! reflection (both falling back to the environment), contact shadows,
//! the key's specular shadow, a thin lens's depth of field, bloom and the
//! accumulation of jittered frames (docs/design/rendering.md, *Layer 2*).
//!
//! Every effect works on the buffers of one frame at its samples, in
//! this order: the shading (with the contact shadows and the specular
//! shadow) gives each sample its linear light and its specular weight;
//! the reflections replace the environment in the specular light where a
//! ray finds a surface on the screen; the depth of field gathers the
//! light of a disc; bloom mixes in a blurred pyramid of the light; then
//! the exposure and the tone curve. The GPU renderer runs the same steps
//! on the same buffers (`studio.wgsl`); the numbers it needs and cannot
//! compute alike (the lens's taps) come from here.

use super::Surface;
use crate::{Projection, View};
use craft3d_math::Vec3;
use std::f64::consts::PI;

/// Steps of a reflected ray across the screen (McGuire and Mara 2014:
/// uniform steps in screen space, depth interpolated as `1/z`).
pub const SSR_STEPS: usize = 32;
/// Halvings of the step where a ray first passed behind a surface.
pub const SSR_REFINE: usize = 5;
/// Above this roughness a face reflects only the environment; below, the
/// screen's light is mixed in by `1 − r / SSR_MAX_ROUGHNESS`.
pub const SSR_MAX_ROUGHNESS: f64 = 0.4;
/// How thick a surface on the screen is taken to be, in diagonals of the
/// mesh's box (besides the depth a step covers).
pub const SSR_THICKNESS: f64 = 0.02;
/// The part of the picture's height over which a reflection fades out
/// towards the picture's border.
pub const SSR_EDGE: f64 = 0.05;
/// How far a reflected ray goes, in diagonals of the mesh's box.
pub const SSR_LENGTH: f64 = 1.0;
/// Steps of a contact-shadow ray towards the key light.
pub const CONTACT_STEPS: usize = 12;
/// How far a contact-shadow ray goes, in diagonals of the mesh's box.
pub const CONTACT_LENGTH: f64 = 0.05;
/// How thick an occluder on the screen is taken to be, in diagonals.
pub const CONTACT_THICKNESS: f64 = 0.02;
/// How far behind the screen's surface a contact-shadow ray must be to
/// count as hidden, in samples at that depth (the surface's depth is
/// interpolated between the tessellation's corners; a flat face is
/// exact).
pub const CONTACT_BIAS: f64 = 0.5;
/// The ground's reflection at full strength (`Look::ground_reflection`
/// 1): the Fresnel reflectance of a dielectric (`F0` = 0.04).
pub const GROUND_REFLECTION: f64 = 1.0;
/// Bloom's share of the light in [`super::Look::polished`].
pub const BLOOM_STRENGTH: f64 = 0.04;
/// Levels of bloom's pyramid below the picture.
pub const BLOOM_LEVELS: usize = 6;
/// Rings of the lens's gather (8 i taps on ring i, plus the centre).
pub const DOF_RINGS: usize = 4;
/// The largest blur radius, as a part of the picture's height.
pub const DOF_MAX_COC: f64 = 1.0 / 40.0;
/// The most frames a still picture averages.
pub const MAX_ACCUMULATE: u32 = 256;
/// The golden angle: the lens's taps turn by it from frame to frame.
const GOLDEN_ANGLE: f64 = 2.399_963_229_728_653;

/// A thin lens: its f-number, where it focuses, and the size of a model
/// unit, for the aperture of the 35 mm camera of the same field of view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dof {
    pub fstop: f64,
    pub focus: Focus,
    /// Millimetres per model unit (the document's length unit).
    pub mm_per_unit: f64,
}

/// Where a lens focuses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Focus {
    /// At this distance along the view direction, in model units.
    Distance(f64),
    /// On the plane through this point, square to the view direction.
    Point([f64; 3]),
}

impl Dof {
    /// The aperture's diameter in model units for a perspective camera
    /// with `half` = tan(fov / 2) focused at `focus` (model units): the
    /// camera is a full-frame one (24 mm high) whose lens, focused there,
    /// has that field, so its image distance is `v = 12 mm / half` and its
    /// focal length `f = v zf / (zf + v)` (`1/f = 1/v + 1/zf`); the
    /// aperture is `f / N`. `None` for numbers that are not usable.
    pub fn aperture(&self, half: f64, focus: f64) -> Option<f64> {
        let v = 12.0 / half;
        let zf = focus * self.mm_per_unit;
        let f = v * zf / (zf + v);
        let a = f / self.fstop / self.mm_per_unit;
        (a.is_finite() && a > 0.0).then_some(a)
    }

    /// The focus distance along `forward` from `eye`, if positive.
    pub fn focus_distance(&self, eye: Vec3, forward: Vec3) -> Option<f64> {
        let z = match self.focus {
            Focus::Distance(z) => z,
            Focus::Point([x, y, z]) => (Vec3::new(x, y, z) - eye).dot(forward),
        };
        (z.is_finite() && z > 0.0).then_some(z)
    }
}

/// The thin lens's blur: the radius in samples of the circle of
/// confusion of a point at view distance `z` (infinity for the
/// background), for an aperture `a` (model units), focus distance `zf`,
/// `half` = tan(fov / 2) and a picture `h` samples high.
///
/// The blur's diameter on the sensor is `a · m · |z − zf| / z` with the
/// magnification `m = f / (zf − f)`, and the sensor is `2 m zf half`
/// high, so the diameter is `a |z − zf| / (2 half z zf)` of the picture's
/// height, whatever the focal length: radius `a h |z − zf| / (4 half z
/// zf)` samples.
pub fn coc_radius(a: f64, zf: f64, half: f64, h: f64, z: f64) -> f64 {
    let k = a * h / (4.0 * half * zf);
    let c = if z.is_infinite() {
        k
    } else if z > 0.0 {
        k * (z - zf).abs() / z
    } else {
        f64::INFINITY
    };
    if c.is_nan() { 0.0 } else { c }
}

/// The radical inverse of `i` in `base` (Halton).
pub fn halton(mut i: u32, base: u32) -> f64 {
    let (mut f, mut r) = (1.0, 0.0);
    let b = f64::from(base.max(2));
    while i > 0 {
        f /= b;
        r += f * f64::from(i % base.max(2));
        i /= base.max(2);
    }
    r
}

/// Where the samples of frame `index` lie off their cells' middles, in
/// samples: nowhere for frame 0, then the Halton (2, 3) points.
pub fn jitter(index: u32) -> (f64, f64) {
    if index == 0 {
        (0.0, 0.0)
    } else {
        (halton(index, 2) - 0.5, halton(index, 3) - 0.5)
    }
}

/// The lens's taps of frame `index` for a reach of `reach` samples: the
/// offsets of ring `i` (1 to [`DOF_RINGS`]) lie at `reach (i − 1 + f) /
/// rings`, 8 i of them evenly around and turned by the golden angle per
/// frame, with `f` 1 on frame 0 and the Halton (5) points after, so the
/// frames together cover the disc. As `f32`, the numbers both renderers
/// read.
pub fn dof_taps(reach: f64, index: u32) -> Vec<[f32; 2]> {
    let f = if index == 0 { 1.0 } else { halton(index, 5) };
    let turn = f64::from(index) * GOLDEN_ANGLE;
    let mut out = Vec::with_capacity(4 * DOF_RINGS * (DOF_RINGS + 1));
    for i in 1..=DOF_RINGS {
        let n = 8 * i;
        let r = reach * (i as f64 - 1.0 + f) / DOF_RINGS as f64;
        for j in 0..n {
            let a = 2.0 * PI * j as f64 / n as f64 + turn + 0.5 * i as f64;
            // Deliberate narrowing: both renderers read these numbers.
            out.push([(r * a.cos()) as f32, (r * a.sin()) as f32]);
        }
    }
    out
}

/// The surface of a sample, if any: its view distance.
fn depth(surfaces: &[Surface], w: usize, h: usize, x: f64, y: f64) -> Option<f64> {
    if !(x >= 0.0 && y >= 0.0 && x < w as f64 && y < h as f64) {
        return None;
    }
    let s = surfaces.get(y as usize * w + x as usize)?;
    s.dist.is_finite().then_some(s.dist)
}

/// A ray from the surface point `p` (model axes) along the unit `d`, up to
/// `length`, across the screen: the first sample whose surface it passes
/// behind by at most `thickness` plus the depth of one step, refined by
/// halving, and not the sample `from` it starts at (McGuire and Mara,
/// *Efficient GPU Screen-Space Ray Tracing*, JCGT 2014, with uniform
/// steps). `None` when it leaves the picture or finds nothing.
#[allow(clippy::too_many_arguments)]
pub(crate) fn trace(
    view: &View,
    surfaces: &[Surface],
    (w, h): (usize, usize),
    p: Vec3,
    d: Vec3,
    length: f64,
    thickness: f64,
    from: (usize, usize),
) -> Option<(usize, usize)> {
    let perspective = matches!(view.projection, Projection::Perspective { .. });
    let z0 = (p - view.eye).dot(view.forward);
    let dz = d.dot(view.forward);
    let mut len = length;
    if perspective && dz < 0.0 {
        // Stop the ray short of the eye plane: at a tenth of its depth.
        len = len.min(0.9 * z0 / -dz);
    }
    if !(len > 0.0 && len.is_finite()) {
        return None;
    }
    let a = view.project(p)?;
    let b = view.project(p + d * len)?;
    let depth_at = |u: f64| {
        if perspective {
            1.0 / ((1.0 - u) / a.dist + u / b.dist)
        } else {
            (1.0 - u) * a.dist + u * b.dist
        }
    };
    let at = |u: f64| {
        (
            (a.sx + (b.sx - a.sx) * u).floor(),
            (a.sy + (b.sy - a.sy) * u).floor(),
        )
    };
    let start = (from.0 as f64, from.1 as f64);
    let mut prev = (0.0, a.dist);
    for i in 1..=SSR_STEPS {
        let u = i as f64 / SSR_STEPS as f64;
        let (x, y) = at(u);
        if !(x >= 0.0 && y >= 0.0 && x < w as f64 && y < h as f64) {
            return None;
        }
        let z = depth_at(u);
        if (x, y) != start
            && let Some(zs) = depth(surfaces, w, h, x, y)
            && z >= zs
            && z - zs <= thickness + (z - prev.1).abs()
        {
            let (mut lo, mut hi) = (prev.0, u);
            for _ in 0..SSR_REFINE {
                let mid = 0.5 * (lo + hi);
                let (mx, my) = at(mid);
                if depth(surfaces, w, h, mx, my).is_some_and(|zs| depth_at(mid) >= zs) {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            let (hx, hy) = at(hi);
            if depth(surfaces, w, h, hx, hy).is_none() || (hx, hy) == start {
                return Some((x as usize, y as usize));
            }
            return Some((hx as usize, hy as usize));
        }
        prev = (u, z);
    }
    None
}

/// The contact shadow at the surface point `p` towards the unit `l`: 0
/// when a short ray finds a surface on the screen in front of it, else
/// 1.
#[allow(clippy::too_many_arguments)]
pub(crate) fn contact(
    view: &View,
    surfaces: &[Surface],
    (w, h): (usize, usize),
    p: Vec3,
    l: Vec3,
    length: f64,
    thickness: f64,
    from: (usize, usize),
) -> f64 {
    let start = (from.0 as f64, from.1 as f64);
    for j in 1..=CONTACT_STEPS {
        let t = length * j as f64 / CONTACT_STEPS as f64;
        let Some(q) = view.project(p + l * t) else {
            return 1.0;
        };
        let (x, y) = (q.sx.floor(), q.sy.floor());
        if !(x >= 0.0 && y >= 0.0 && x < w as f64 && y < h as f64) {
            return 1.0;
        }
        if (x, y) == start {
            continue;
        }
        let Some(zs) = depth(surfaces, w, h, x, y) else {
            continue;
        };
        let behind = q.dist - zs;
        if behind > CONTACT_BIAS * view.sample_size(zs) && behind < thickness {
            return 0.0;
        }
    }
    1.0
}

/// The fade of a reflection found at sample `(x, y)` towards the border
/// of a `w` × `h` picture.
pub(crate) fn edge_fade(x: usize, y: usize, w: usize, h: usize) -> f64 {
    let (x, y) = (x as f64 + 0.5, y as f64 + 0.5);
    let near = x.min(w as f64 - x).min(y).min(h as f64 - y);
    (near / (SSR_EDGE * h as f64)).clamp(0.0, 1.0)
}

/// Schlick's Fresnel reflectance of a dielectric (`F0` = 0.04) at the
/// cosine `c`.
pub fn dielectric_fresnel(c: f64) -> f64 {
    let f0 = super::DIELECTRIC_F0;
    f0 + (1.0 - f0) * (1.0 - c.clamp(0.0, 1.0)).powi(5)
}

/// GGX `D` and the height-correlated Smith visibility `V` (Filament) at
/// `α`, the key's specular light per unit irradiance and unit `F`.
pub fn ggx_dv(alpha: f64, nh: f64, nv: f64, nl: f64) -> f64 {
    let a2 = alpha * alpha;
    let k = nh * nh * (a2 - 1.0) + 1.0;
    let d = a2 / (PI * k * k);
    let gv = nl * (nv * nv * (1.0 - a2) + a2).sqrt();
    let gl = nv * (nl * nl * (1.0 - a2) + a2).sqrt();
    let v = 0.5 / (gv + gl);
    let out = d * v;
    if out.is_finite() { out.max(0.0) } else { 0.0 }
}

/// The light of a frame at its samples, as both renderers store it
/// (`f32`): the linear light and, for the reflections, the specular
/// weight with the roughness in the fourth place (a negative roughness
/// for a sample without a surface).
pub(crate) struct Buffers {
    pub w: usize,
    pub h: usize,
    pub light: Vec<[f32; 3]>,
    pub weight: Vec<[f32; 4]>,
}

/// The texel `(x, y)` of a `w` × `h` level, clamped to its border.
fn load(level: &[[f32; 3]], w: usize, h: usize, x: i64, y: i64) -> [f64; 3] {
    let x = x.clamp(0, w as i64 - 1) as usize;
    let y = y.clamp(0, h as i64 - 1) as usize;
    level.get(y * w + x).map_or([0.0; 3], |c| c.map(f64::from))
}

fn add(a: &mut [f64; 3], b: [f64; 3], k: f64) {
    for (x, y) in a.iter_mut().zip(b) {
        *x += y * k;
    }
}

/// One level of bloom's pyramid down (Jimenez, *Next Generation Post
/// Processing in Call of Duty: Advanced Warfare*, SIGGRAPH 2014): 13
/// bilinear taps, each the mean of the 2 × 2 texels around a corner of
/// the source, in five overlapping boxes weighted 1/2 (the middle one)
/// and 1/8. The size halves, rounded up.
pub fn bloom_down(src: &[[f32; 3]], w: usize, h: usize) -> (Vec<[f32; 3]>, usize, usize) {
    let (dw, dh) = (w.div_ceil(2).max(1), h.div_ceil(2).max(1));
    let corner = |cx: i64, cy: i64| {
        let mut c = [0.0; 3];
        for (ox, oy) in [(-1, -1), (0, -1), (-1, 0), (0, 0)] {
            add(&mut c, load(src, w, h, cx + ox, cy + oy), 0.25);
        }
        c
    };
    let mut out = Vec::with_capacity(dw * dh);
    for y in 0..dh as i64 {
        for x in 0..dw as i64 {
            let (cx, cy) = (2 * x + 1, 2 * y + 1);
            let t = |ox: i64, oy: i64| corner(cx + ox, cy + oy);
            let mut c = [0.0; 3];
            for (ox, oy) in [(-1, -1), (1, -1), (-1, 1), (1, 1)] {
                add(&mut c, t(ox, oy), 0.5 / 4.0);
            }
            for (bx, by) in [(-2, -2), (0, -2), (-2, 0), (0, 0)] {
                for (ox, oy) in [(0, 0), (2, 0), (0, 2), (2, 2)] {
                    add(&mut c, t(bx + ox, by + oy), 0.125 / 4.0);
                }
            }
            // Deliberate narrowing: a level is stored as f32.
            out.push(c.map(|v| v as f32));
        }
    }
    (out, dw, dh)
}

/// The coarse level `src` (`w` × `h`) read at the fine level's texel
/// `(x, y)` with a 3 × 3 tent of bilinear taps one coarse texel apart.
pub fn bloom_tent(src: &[[f32; 3]], w: usize, h: usize, x: usize, y: usize) -> [f64; 3] {
    let (u, v) = ((x as f64 + 0.5) * 0.5, (y as f64 + 0.5) * 0.5);
    let bilinear = |u: f64, v: f64| {
        let (fx, fy) = (u - 0.5, v - 0.5);
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let (x0, y0) = (x0 as i64, y0 as i64);
        let mut c = [0.0; 3];
        add(&mut c, load(src, w, h, x0, y0), (1.0 - tx) * (1.0 - ty));
        add(&mut c, load(src, w, h, x0 + 1, y0), tx * (1.0 - ty));
        add(&mut c, load(src, w, h, x0, y0 + 1), (1.0 - tx) * ty);
        add(&mut c, load(src, w, h, x0 + 1, y0 + 1), tx * ty);
        c
    };
    let mut c = [0.0; 3];
    for (oy, wy) in [(-1.0, 1.0), (0.0, 2.0), (1.0, 1.0)] {
        for (ox, wx) in [(-1.0, 1.0), (0.0, 2.0), (1.0, 1.0)] {
            add(&mut c, bilinear(u + ox, v + oy), wx * wy / 16.0);
        }
    }
    c
}

/// Bloom: the light blurred at [`BLOOM_LEVELS`] scales and averaged with
/// itself, `B = (L + up(D1 + up(D2 + …))) / (levels + 1)`, mixed in by
/// `strength`: `(1 − s) L + s B`. Every filter's weights add up to 1, so
/// the light's sum is kept but for what the border's clamping adds.
pub fn bloom(light: &[[f32; 3]], w: usize, h: usize, strength: f64) -> Vec<[f32; 3]> {
    let s = strength.clamp(0.0, 1.0);
    let mut levels = vec![(light.to_vec(), w, h)];
    for _ in 0..BLOOM_LEVELS {
        let Some((src, sw, sh)) = levels.last() else {
            break;
        };
        if *sw <= 1 && *sh <= 1 {
            break;
        }
        let next = bloom_down(src, *sw, *sh);
        levels.push(next);
    }
    let n = levels.len();
    // up_i = D_i + tent(up_{i+1}), from the smallest level up.
    let mut up: Option<(Vec<[f32; 3]>, usize, usize)> = None;
    for (src, sw, sh) in levels.iter().skip(1).rev() {
        let next = match &up {
            None => src.clone(),
            Some((coarse, cw, ch)) => sum_level(src, *sw, *sh, coarse, *cw, *ch),
        };
        up = Some((next, *sw, *sh));
    }
    let mut out = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let l = light.get(y * w + x).map_or([0.0; 3], |c| c.map(f64::from));
            let mut b = l;
            if let Some((coarse, cw, ch)) = &up {
                add(&mut b, bloom_tent(coarse, *cw, *ch, x, y), 1.0);
            }
            let mut c = [0.0; 3];
            for k in 0..3 {
                let (lk, bk) = (l.get(k).copied(), b.get(k).copied());
                if let (Some(o), Some(lk), Some(bk)) = (c.get_mut(k), lk, bk) {
                    *o = (1.0 - s) * lk + s * bk / n as f64;
                }
            }
            out.push(c.map(|v| v as f32));
        }
    }
    out
}

/// `fine + tent(coarse)`, stored as f32.
fn sum_level(
    fine: &[[f32; 3]],
    w: usize,
    h: usize,
    coarse: &[[f32; 3]],
    cw: usize,
    ch: usize,
) -> Vec<[f32; 3]> {
    let mut out = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let mut c = fine.get(y * w + x).map_or([0.0; 3], |c| c.map(f64::from));
            add(&mut c, bloom_tent(coarse, cw, ch, x, y), 1.0);
            out.push(c.map(|v| v as f32));
        }
    }
    out
}

/// The thin lens's depth of field by gathering (a scatter written as a
/// gather): each sample averages the taps whose blur reaches it, a tap
/// at distance `d` weighing `clamp(c − d + 1/2, 0, 1) / max(c, 1/2)²`
/// with `c` the tap's blur radius, or the smaller of its and the
/// sample's when the tap lies behind it (a far surface does not spread
/// over a nearer one). A sample in focus with nothing in front of it
/// keeps its light exactly.
pub fn dof_gather(
    light: &[[f32; 3]],
    depth: &[f64],
    (w, h): (usize, usize),
    coc: impl Fn(f64) -> f64 + Sync,
    taps: &[[f32; 2]],
) -> Vec<[f32; 3]> {
    super::env::par_rows(h, |y| {
        (0..w)
            .map(|x| {
                let i = y * w + x;
                let z0 = depth.get(i).copied().unwrap_or(f64::INFINITY);
                let c0 = coc(z0);
                let l0 = light.get(i).map_or([0.0; 3], |c| c.map(f64::from));
                let w0 = 1.0 / c0.max(0.5).powi(2);
                let mut sum = [0.0; 3];
                add(&mut sum, l0, w0);
                let mut total = w0;
                for &[ox, oy] in taps {
                    let (ox, oy) = (f64::from(ox), f64::from(oy));
                    let (tx, ty) = ((x as f64 + 0.5 + ox).floor(), (y as f64 + 0.5 + oy).floor());
                    if !(tx >= 0.0 && ty >= 0.0 && tx < w as f64 && ty < h as f64) {
                        continue;
                    }
                    let j = ty as usize * w + tx as usize;
                    let zt = depth.get(j).copied().unwrap_or(f64::INFINITY);
                    let ct = coc(zt);
                    let c = if zt > z0 { ct.min(c0) } else { ct };
                    let d = (ox * ox + oy * oy).sqrt();
                    let cover = (c - d + 0.5).clamp(0.0, 1.0);
                    if cover <= 0.0 {
                        continue;
                    }
                    let wt = cover / c.max(0.5).powi(2);
                    add(
                        &mut sum,
                        light.get(j).map_or([0.0; 3], |c| c.map(f64::from)),
                        wt,
                    );
                    total += wt;
                }
                sum.map(|v| (v / total) as f32)
            })
            .collect()
    })
}

/// What the reflections of a frame need: its view, its samples'
/// surfaces and the light.
pub(crate) struct Reflect<'a> {
    pub view: &'a View,
    pub surfaces: &'a [Surface],
    pub frame: &'a super::FrameLight<'a>,
    /// The diagonal of the mesh's box.
    pub diagonal: f64,
}

impl Reflect<'_> {
    /// The reflections of every sample: a glossy surface's specular
    /// environment replaced by the light the ray finds on the screen
    /// (`L + conf · W · (hit − env)`), and the ground's reflection of the
    /// model, the environment where its ray finds nothing.
    pub(crate) fn apply(&self, buffers: &Buffers) -> Vec<[f32; 3]> {
        let (w, h) = (buffers.w, buffers.h);
        let look = &self.frame.look;
        let top = self.frame.top_level();
        let length = SSR_LENGTH * self.diagonal;
        let thickness = SSR_THICKNESS * self.diagonal;
        let hit_light = |(hx, hy): (usize, usize)| {
            buffers
                .light
                .get(hy * w + hx)
                .map_or([0.0; 3], |c| c.map(f64::from))
        };
        let facing = |(hx, hy): (usize, usize), d: Vec3| -> bool {
            let Some(s) = self.surfaces.get(hy * w + hx) else {
                return false;
            };
            let p = self.view.eye + self.view.point_at(hx as f64 + 0.5, hy as f64 + 0.5, s.dist);
            let v = self.view.toward_viewer(p);
            let [nx, ny, nz] = s.normal.map(f64::from);
            let n = Vec3::new(nx, ny, nz).normalized().unwrap_or(v);
            let n = if n.dot(v) < 0.0 { -n } else { n };
            n.dot(d) <= 0.0
        };
        super::env::par_rows(h, |y| {
            (0..w)
                .map(|x| {
                    let i = y * w + x;
                    let l = buffers.light.get(i).copied().unwrap_or([0.0; 3]);
                    let Some(s) = self.surfaces.get(i) else {
                        return l;
                    };
                    let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                    let mut out = l.map(f64::from);
                    if s.dist.is_finite() {
                        let [wr, wg, wb, r] =
                            buffers.weight.get(i).map_or([0.0; 4], |c| c.map(f64::from));
                        if !(look.ssr && (0.0..SSR_MAX_ROUGHNESS).contains(&r)) {
                            return l;
                        }
                        let rel = self.view.point_at(px, py, s.dist);
                        let p = self.view.eye + rel;
                        let v = self.view.toward_viewer(p);
                        let [nx, ny, nz] = s.normal.map(f64::from);
                        let n = Vec3::new(nx, ny, nz).normalized().unwrap_or(v);
                        let n = if n.dot(v) < 0.0 { -n } else { n };
                        let d = n * (2.0 * n.dot(v)) - v;
                        let Some(d) = d.normalized() else {
                            return l;
                        };
                        let Some(hit) = trace(
                            self.view,
                            self.surfaces,
                            (w, h),
                            p,
                            d,
                            length,
                            thickness,
                            (x, y),
                        ) else {
                            return l;
                        };
                        if !facing(hit, d) {
                            return l;
                        }
                        let conf = (1.0 - r / SSR_MAX_ROUGHNESS) * edge_fade(hit.0, hit.1, w, h);
                        let env = self.frame.radiance(d, r * top);
                        let found = hit_light(hit);
                        for (k, wk) in [wr, wg, wb].into_iter().enumerate() {
                            if let (Some(o), Some(e), Some(f)) =
                                (out.get_mut(k), env.get(k), found.get(k))
                            {
                                *o += conf * wk * (f - e);
                            }
                        }
                    } else {
                        // The plain background reflects nothing.
                        let plain = buffers
                            .weight
                            .get(i)
                            .is_some_and(|w| f64::from(w[3]) == super::PLAIN_BACKDROP);
                        let strength = if plain {
                            0.0
                        } else {
                            look.ground_reflection.clamp(0.0, 1.0)
                        };
                        let Some(z) = self.frame.ground.filter(|_| strength > 0.0) else {
                            return l;
                        };
                        let (origin, d) = self.view.ray(px, py);
                        if !(d.z < 0.0 && origin.z > z) {
                            return l;
                        }
                        let t = (z - origin.z) / d.z;
                        if !(t.is_finite() && t > 0.0) {
                            return l;
                        }
                        let g = origin + d * t;
                        let m = Vec3::new(d.x, d.y, -d.z);
                        let f = strength * dielectric_fresnel(-d.z);
                        let found = trace(
                            self.view,
                            self.surfaces,
                            (w, h),
                            g,
                            m,
                            length,
                            thickness,
                            (x, y),
                        )
                        .filter(|&hit| facing(hit, m))
                        .map(|hit| (hit_light(hit), edge_fade(hit.0, hit.1, w, h)));
                        let env = self
                            .frame
                            .radiance(m, super::env::BACKGROUND_ROUGHNESS * top);
                        let (seen, conf) = found.unwrap_or((env, 0.0));
                        for k in 0..3 {
                            if let (Some(o), Some(e), Some(s)) =
                                (out.get_mut(k), env.get(k), seen.get(k))
                            {
                                let refl = e + conf * (s - e);
                                *o += f * (refl - *o);
                            }
                        }
                    }
                    out.map(|v| v as f32)
                })
                .collect()
        })
    }
}

/// A lens over a frame: what [`coc_radius`] needs and the reach of the
/// gather, the most blur any sample of the frame can have.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lens {
    pub aperture: f64,
    pub focus: f64,
    pub half: f64,
    /// The picture's height in samples.
    pub height: f64,
    /// The largest blur radius, in samples ([`DOF_MAX_COC`]).
    pub cap: f64,
    /// The largest blur in the frame: over the depths of the mesh's box
    /// and the background, capped.
    pub reach: f64,
}

impl Lens {
    /// The lens of `dof` for a picture `height` samples high of a mesh
    /// within `bounds`; `None` for an orthographic camera (no lens), a
    /// focus behind the eye, numbers that are not usable, or a blur under
    /// half a sample everywhere.
    pub fn new(
        dof: &Dof,
        camera: &crate::Camera,
        bounds: &craft3d_math::Bbox3,
        height: usize,
    ) -> Option<Lens> {
        let axes = camera.axes().ok()?;
        if !matches!(axes.projection, Projection::Perspective { .. }) {
            return None;
        }
        let half = axes.half;
        let focus = dof.focus_distance(axes.eye, axes.forward)?;
        let aperture = dof.aperture(half, focus)?;
        let h = height as f64;
        let cap = DOF_MAX_COC * h;
        let c = |z: f64| coc_radius(aperture, focus, half, h, z).min(cap);
        let mut reach = c(f64::INFINITY);
        if let Some((lo, hi)) = bounds.corners() {
            for i in 0..8 {
                let p = Vec3::new(
                    if i & 1 == 0 { lo.x } else { hi.x },
                    if i & 2 == 0 { lo.y } else { hi.y },
                    if i & 4 == 0 { lo.z } else { hi.z },
                );
                reach = reach.max(c((p - axes.eye).dot(axes.forward)));
            }
        }
        (reach >= 0.5).then_some(Lens {
            aperture,
            focus,
            half,
            height: h,
            cap,
            reach,
        })
    }

    /// The blur radius in samples at view distance `z`, capped.
    pub fn coc(&self, z: f64) -> f64 {
        coc_radius(self.aperture, self.focus, self.half, self.height, z).min(self.cap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halton_points_are_the_radical_inverses() {
        assert_eq!(halton(1, 2), 0.5);
        assert_eq!(halton(3, 2), 0.75);
        assert!((halton(2, 3) - 2.0 / 3.0).abs() < 1e-15);
        assert_eq!(jitter(0), (0.0, 0.0));
    }

    #[test]
    fn the_blur_is_zero_at_the_focus_and_grows_away_from_it() {
        let (a, zf, half, h) = (2.0, 100.0, 0.5, 400.0);
        assert_eq!(coc_radius(a, zf, half, h, zf), 0.0);
        // a h |z − zf| / (4 half z zf) at z = 200: 2·400·100/(4·0.5·200·100).
        assert!((coc_radius(a, zf, half, h, 200.0) - 2.0).abs() < 1e-12);
        assert!((coc_radius(a, zf, half, h, f64::INFINITY) - 4.0).abs() < 1e-12);
        assert_eq!(coc_radius(a, zf, half, h, -1.0), f64::INFINITY);
    }

    #[test]
    fn the_taps_lie_on_their_rings() {
        let taps = dof_taps(8.0, 0);
        assert_eq!(taps.len(), 8 + 16 + 24 + 32);
        let r = |t: &[f32; 2]| f64::from(t[0]).hypot(f64::from(t[1]));
        assert!(taps.iter().all(|t| r(t) <= 8.0 + 1e-5));
        assert!(taps.iter().any(|t| (r(t) - 8.0).abs() < 1e-5));
    }
}
