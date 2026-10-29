//! An environment prepared for image-based lighting: the prefiltered
//! specular levels, the diffuse irradiance as spherical harmonics and the
//! key light, all computed here at load (docs/design/rendering.md,
//! *The studio look*).
//!
//! Directions are in the model's axes with z up. An environment picture
//! is equirectangular: texel `(x, y)` of a `W × H` picture looks along
//! `φ = ((x + 0.5) / W − 0.5) · 2π` around z from +x and
//! `θ = (y + 0.5) / H · π` from +z.

use super::brdf::{LUT_SIZE, brdf_lut, ggx_d, hammersley, sample_ggx};
use super::hdr::HdrImage;
use crate::RenderError;
use craft3d_math::Vec3;
use std::f64::consts::PI;

/// Number of prefiltered specular levels; level `i` is for roughness
/// `i / (SPEC_LEVELS − 1)`.
pub const SPEC_LEVELS: usize = 6;
/// Width of level 0 (the mirror); every level halves it, the height is
/// half the width.
pub const SPEC_WIDTH: usize = 512;
/// GGX importance samples per texel of a prefiltered level.
pub const PREFILTER_SAMPLES: u32 = 128;
/// The roughness the background is shown at: the environment blurred, so
/// that it reads as a studio and not as a photograph behind the model.
pub const BACKGROUND_ROUGHNESS: f64 = 0.45;
/// The key light is the excess over `KEY_THRESHOLD` times the mean
/// luminance (or a quarter of the peak, if higher) within `KEY_CONE`
/// radians of the brightest direction.
pub const KEY_THRESHOLD: f64 = 4.0;
pub const KEY_CONE: f64 = 0.5;

/// One level of an equirectangular picture, linear RGB as `f32`.
#[derive(Clone, Debug, PartialEq)]
pub struct Level {
    pub width: usize,
    pub height: usize,
    pub texels: Vec<[f32; 3]>,
}

/// The light taken out of the environment as a directional light: what
/// casts the shadows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Key {
    /// Unit vector towards the light, in the environment's own axes
    /// (before [`super::Look::rotation`]).
    pub direction: [f32; 3],
    /// The irradiance it gives a surface facing it, linear RGB.
    pub irradiance: [f32; 3],
    /// Its angular radius in radians: how soft its shadows are.
    pub radius: f32,
}

/// An environment ready for both renderers. Every number either renderer
/// reads is here, as `f32`.
#[derive(Clone, Debug, PartialEq)]
pub struct Prepared {
    /// Prefiltered specular levels, mirror first.
    pub levels: Vec<Level>,
    /// The irradiance of the environment without the key light, as nine
    /// spherical-harmonic coefficients per channel already convolved
    /// with the clamped cosine (Ramamoorthi and Hanrahan 2001): `E(n) =
    /// Σ sh[k] · Y_k(n)` with the real basis of [`sh_basis`].
    pub sh: [[f32; 3]; 9],
    /// The key light, when the environment has a distinct bright region.
    pub key: Option<Key>,
    /// The split-sum table ([`brdf_lut`]).
    pub lut: Vec<[f32; 2]>,
}

/// The direction of the centre of texel `(x, y)` of a `w × h` picture.
pub fn texel_direction(x: usize, y: usize, w: usize, h: usize) -> Vec3 {
    let phi = ((x as f64 + 0.5) / w as f64 - 0.5) * 2.0 * PI;
    let theta = (y as f64 + 0.5) / h as f64 * PI;
    Vec3::new(
        theta.sin() * phi.cos(),
        theta.sin() * phi.sin(),
        theta.cos(),
    )
}

/// The solid angle of a texel of row `y` of a `w × h` picture.
fn texel_solid_angle(y: usize, w: usize, h: usize) -> f64 {
    let theta = (y as f64 + 0.5) / h as f64 * PI;
    (2.0 * PI / w as f64) * (PI / h as f64) * theta.sin()
}

/// Rec. 709 luminance.
pub fn luminance(c: [f64; 3]) -> f64 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// The nine real spherical harmonics of bands 0 to 2 at a unit vector.
pub fn sh_basis(n: Vec3) -> [f64; 9] {
    let (x, y, z) = (n.x, n.y, n.z);
    [
        0.282_094_791,
        0.488_602_512 * y,
        0.488_602_512 * z,
        0.488_602_512 * x,
        1.092_548_431 * x * y,
        1.092_548_431 * y * z,
        0.315_391_565 * (3.0 * z * z - 1.0),
        1.092_548_431 * x * z,
        0.546_274_215 * (x * x - y * y),
    ]
}

fn to64(c: [f32; 3]) -> [f64; 3] {
    c.map(f64::from)
}

impl Level {
    /// The picture bilinearly at the texture coordinates `(u, v)`, both in
    /// 0..1 over the picture: wrapped around in `u`, clamped in `v`.
    pub fn bilinear(&self, u: f64, v: f64) -> [f64; 3] {
        let (w, h) = (self.width as f64, self.height as f64);
        let x = u * w - 0.5;
        let y = (v * h - 0.5).clamp(0.0, h - 1.0);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let wrap = |i: f64| (i.rem_euclid(w) as usize).min(self.width.saturating_sub(1));
        let (xa, xb) = (wrap(x0), wrap(x0 + 1.0));
        let ya = y0 as usize;
        let yb = (ya + 1).min(self.height.saturating_sub(1));
        let at = |x: usize, y: usize| {
            self.texels
                .get(y * self.width + x)
                .map_or([0.0; 3], |&t| to64(t))
        };
        let (a, b, c, d) = (at(xa, ya), at(xb, ya), at(xa, yb), at(xb, yb));
        let mut out = [0.0; 3];
        for (k, o) in out.iter_mut().enumerate() {
            let top = a[k] + (b[k] - a[k]) * fx;
            let bottom = c[k] + (d[k] - c[k]) * fx;
            *o = top + (bottom - top) * fy;
        }
        out
    }

    fn box_down(&self) -> Level {
        let (w, h) = ((self.width / 2).max(1), (self.height / 2).max(1));
        let mut texels = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let mut sum = [0.0_f64; 3];
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let (sx, sy) = (
                        (2 * x + dx).min(self.width - 1),
                        (2 * y + dy).min(self.height - 1),
                    );
                    if let Some(t) = self.texels.get(sy * self.width + sx) {
                        for (s, c) in sum.iter_mut().zip(t) {
                            *s += f64::from(*c);
                        }
                    }
                }
                texels.push(sum.map(|s| (s / 4.0) as f32));
            }
        }
        Level {
            width: w,
            height: h,
            texels,
        }
    }
}

/// The texture coordinates of a direction, turned by `rotation` radians
/// about z (the environment turned the other way).
pub fn direction_uv(d: Vec3, rotation: f64) -> (f64, f64) {
    let u = d.y.atan2(d.x) / (2.0 * PI) + 0.5 - rotation / (2.0 * PI);
    let v = d.z.clamp(-1.0, 1.0).acos() / PI;
    (u, v)
}

/// A list of levels looked up at a fractional level `lod`, linearly
/// between the two levels around it.
pub fn trilinear(levels: &[Level], d: Vec3, lod: f64, rotation: f64) -> [f64; 3] {
    let (u, v) = direction_uv(d, rotation);
    let top = levels.len().saturating_sub(1) as f64;
    let lod = if lod.is_finite() {
        lod.clamp(0.0, top)
    } else {
        0.0
    };
    let l0 = lod.floor();
    let f = lod - l0;
    let i0 = l0 as usize;
    let a = levels.get(i0).map_or([0.0; 3], |l| l.bilinear(u, v));
    if f <= 0.0 {
        return a;
    }
    let b = levels.get(i0 + 1).map_or(a, |l| l.bilinear(u, v));
    [
        a[0] + (b[0] - a[0]) * f,
        a[1] + (b[1] - a[1]) * f,
        a[2] + (b[2] - a[2]) * f,
    ]
}

/// Runs `row` for every row `0..rows` on the machine's threads and
/// returns the rows in order: the same result for any thread count.
pub(crate) fn par_rows<T: Send>(rows: usize, row: impl Fn(usize) -> Vec<T> + Sync) -> Vec<T> {
    let threads = craft3d_math::available_threads()
        .clamp(1, 16)
        .min(rows.max(1));
    if threads <= 1 {
        return (0..rows).flat_map(&row).collect();
    }
    let chunk = rows.div_ceil(threads);
    let row = &row;
    let parts: Vec<Vec<T>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                scope.spawn(move || {
                    let end = ((t + 1) * chunk).min(rows);
                    (t * chunk..end).flat_map(row).collect::<Vec<T>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or_default())
            .collect()
    });
    // A thread that panicked gave an empty part: the rows then do not add
    // up, which the callers check.
    parts.into_iter().flatten().collect()
}

impl Prepared {
    /// Prepares an environment picture: level 0 is the picture resampled
    /// to [`SPEC_WIDTH`] × [`SPEC_WIDTH`]/2 and scaled so that its mean
    /// luminance over the sphere is 1 (so every environment lights a
    /// model alike, and [`super::Look::exposure`] means the same for
    /// all); the key light is taken out of it for the diffuse light
    /// ([`Key`]); the other levels are prefiltered with GGX for their
    /// roughness.
    pub fn new(image: &HdrImage) -> Result<Prepared, RenderError> {
        Self::with_lut(image, brdf_lut())
    }

    /// [`Self::new`] with a split-sum table already computed (it does not
    /// depend on the environment).
    pub fn with_lut(image: &HdrImage, lut: Vec<[f32; 2]>) -> Result<Prepared, RenderError> {
        let bad = |why: &str| RenderError::Environment(why.to_string());
        if image.width == 0
            || image.height == 0
            || image.pixels.len() != image.width.saturating_mul(image.height)
        {
            return Err(bad("the picture has no pixels or the wrong number"));
        }
        if lut.len() != LUT_SIZE * LUT_SIZE {
            return Err(bad("the split-sum table has the wrong size"));
        }
        let source = Level {
            width: image.width,
            height: image.height,
            texels: image
                .pixels
                .iter()
                .map(|p| p.map(|c| if c.is_finite() { c.max(0.0) } else { 0.0 }))
                .collect(),
        };
        // Level 0: K × K bilinear lookups per texel, at the centres of
        // the source texels it covers when the source is K times wider.
        let (w0, h0) = (SPEC_WIDTH, SPEC_WIDTH / 2);
        let k = image.width.div_ceil(w0).max(1);
        let base = par_rows(h0, |y| {
            (0..w0)
                .map(|x| {
                    let mut sum = [0.0; 3];
                    for sy in 0..k {
                        for sx in 0..k {
                            let u = (x as f64 + (sx as f64 + 0.5) / k as f64) / w0 as f64;
                            let v = (y as f64 + (sy as f64 + 0.5) / k as f64) / h0 as f64;
                            let c = source.bilinear(u, v);
                            for (s, c) in sum.iter_mut().zip(c) {
                                *s += c;
                            }
                        }
                    }
                    sum.map(|s| s / (k * k) as f64)
                })
                .collect()
        });
        if base.len() != w0 * h0 {
            return Err(bad("preparing the environment failed"));
        }
        // The mean luminance over the sphere.
        let mut mean = 0.0;
        for (i, c) in base.iter().enumerate() {
            mean += luminance(*c) * texel_solid_angle(i / w0, w0, h0);
        }
        mean /= 4.0 * PI;
        if !(mean.is_finite() && mean > 0.0) {
            return Err(bad("the environment is black or not finite"));
        }
        let scale = 1.0 / mean;
        let mirror = Level {
            width: w0,
            height: h0,
            texels: base.iter().map(|c| c.map(|v| (v * scale) as f32)).collect(),
        };
        // The source chain for filtered importance sampling.
        let mut chain = vec![mirror.clone()];
        while chain.last().is_some_and(|l| l.width > 8) {
            let Some(next) = chain.last().map(Level::box_down) else {
                break;
            };
            chain.push(next);
        }

        let key = extract_key(&mirror, &chain);
        // The diffuse light: the environment without the key.
        let mut sh = [[0.0_f64; 3]; 9];
        for (i, t) in mirror.texels.iter().enumerate() {
            let (x, y) = (i % w0, i / w0);
            let d = texel_direction(x, y, w0, h0);
            let dw = texel_solid_angle(y, w0, h0);
            let mut c = to64(*t);
            if let Some((_, ref excess)) = key {
                if let Some(e) = excess.get(i) {
                    for (c, e) in c.iter_mut().zip(e) {
                        *c -= e;
                    }
                }
            }
            for (coef, y) in sh.iter_mut().zip(sh_basis(d)) {
                for (s, c) in coef.iter_mut().zip(c) {
                    *s += c * y * dw;
                }
            }
        }
        // Convolved with the clamped cosine: π, 2π/3 and π/4 per band.
        let band = [
            PI,
            2.0 * PI / 3.0,
            2.0 * PI / 3.0,
            2.0 * PI / 3.0,
            PI / 4.0,
            PI / 4.0,
            PI / 4.0,
            PI / 4.0,
            PI / 4.0,
        ];
        let sh = std::array::from_fn(|k| {
            let a = band.get(k).copied().unwrap_or(0.0);
            sh.get(k)
                .copied()
                .unwrap_or([0.0; 3])
                .map(|c| (c * a) as f32)
        });

        let mut levels = vec![mirror];
        for i in 1..SPEC_LEVELS {
            let roughness = i as f64 / (SPEC_LEVELS - 1) as f64;
            levels.push(prefilter(&chain, roughness, w0 >> i, h0 >> i)?);
        }
        Ok(Prepared {
            levels,
            sh,
            key: key.map(|(k, _)| k),
            lut,
        })
    }

    /// The irradiance of the environment without its key light on a
    /// surface with the unit normal `n` (in the environment's axes).
    pub fn irradiance(&self, n: Vec3) -> [f64; 3] {
        let y = sh_basis(n);
        let mut out = [0.0; 3];
        for (coef, y) in self.sh.iter().zip(y) {
            for (o, c) in out.iter_mut().zip(coef) {
                *o += f64::from(*c) * y;
            }
        }
        out.map(|v| v.max(0.0))
    }

    /// The split-sum table at `n·v` and roughness, bilinearly between the
    /// texel centres, clamped at the borders.
    pub fn split_sum(&self, n_dot_v: f64, roughness: f64) -> (f64, f64) {
        let size = LUT_SIZE as f64;
        let x = (n_dot_v * size - 0.5).clamp(0.0, size - 1.0);
        let y = (roughness * size - 0.5).clamp(0.0, size - 1.0);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let (xa, ya) = (x0 as usize, y0 as usize);
        let (xb, yb) = ((xa + 1).min(LUT_SIZE - 1), (ya + 1).min(LUT_SIZE - 1));
        let at = |x: usize, y: usize| {
            self.lut
                .get(y * LUT_SIZE + x)
                .map_or([0.0; 2], |t| t.map(f64::from))
        };
        let (a, b, c, d) = (at(xa, ya), at(xb, ya), at(xa, yb), at(xb, yb));
        let lerp2 = |k: usize| {
            let top = a.get(k).copied().unwrap_or(0.0)
                + (b.get(k).copied().unwrap_or(0.0) - a.get(k).copied().unwrap_or(0.0)) * fx;
            let bottom = c.get(k).copied().unwrap_or(0.0)
                + (d.get(k).copied().unwrap_or(0.0) - c.get(k).copied().unwrap_or(0.0)) * fx;
            top + (bottom - top) * fy
        };
        (lerp2(0), lerp2(1))
    }
}

/// Finds the key light: the brightest direction of a coarse level, then
/// the excess over the threshold within [`KEY_CONE`] of it. Returns the
/// key and the excess per texel of `mirror`, or `None` when nothing
/// stands out (an even environment casts no shadow).
fn extract_key(mirror: &Level, chain: &[Level]) -> Option<(Key, Vec<[f64; 3]>)> {
    let coarse = chain
        .iter()
        .find(|l| l.width <= 64)
        .or_else(|| chain.last())?;
    let mut peak = (0.0, Vec3::Z);
    for (i, t) in coarse.texels.iter().enumerate() {
        let lum = luminance(to64(*t));
        if lum > peak.0 {
            peak = (
                lum,
                texel_direction(
                    i % coarse.width,
                    i / coarse.width,
                    coarse.width,
                    coarse.height,
                ),
            );
        }
    }
    let threshold = KEY_THRESHOLD.max(peak.0 * 0.25);
    let (w, h) = (mirror.width, mirror.height);
    let cos_cone = KEY_CONE.cos();
    let mut excess = vec![[0.0; 3]; mirror.texels.len()];
    let (mut total, mut weighted, mut area) = ([0.0_f64; 3], Vec3::ZERO, 0.0);
    for (i, t) in mirror.texels.iter().enumerate() {
        let (x, y) = (i % w, i / w);
        let d = texel_direction(x, y, w, h);
        let c = to64(*t);
        let lum = luminance(c);
        if lum <= threshold || d.dot(peak.1) < cos_cone {
            continue;
        }
        let part = c.map(|v| v * (1.0 - threshold / lum));
        let dw = texel_solid_angle(y, w, h);
        if let Some(slot) = excess.get_mut(i) {
            *slot = part;
        }
        for (s, p) in total.iter_mut().zip(part) {
            *s += p * dw;
        }
        weighted += d * (luminance(part) * dw);
        area += dw;
    }
    let direction = weighted.normalized()?;
    if luminance(total) <= 0.0 {
        return None;
    }
    let radius = (area / PI).sqrt().clamp(0.01, KEY_CONE);
    Some((
        Key {
            direction: [direction.x as f32, direction.y as f32, direction.z as f32],
            irradiance: total.map(|v| v as f32),
            radius: radius as f32,
        },
        excess,
    ))
}

/// One prefiltered level: per texel, the environment seen through a GGX
/// lobe of `roughness` around its direction (`n = v = r`, Karis 2013),
/// with filtered importance sampling (Colbert and Křivánek, *GPU-Based
/// Importance Sampling*, GPU Gems 3, ch. 20): each sample reads the
/// source chain at the level whose texels match the sample's share of
/// the lobe, so few samples give a smooth result.
fn prefilter(chain: &[Level], roughness: f64, w: usize, h: usize) -> Result<Level, RenderError> {
    let (w, h) = (w.max(1), h.max(1));
    let base = chain.first().map_or(1.0, |l| (l.width * l.height) as f64);
    let texel_angle = 4.0 * PI / base;
    let samples: Vec<(f64, f64)> = (0..PREFILTER_SAMPLES)
        .map(|i| hammersley(i, PREFILTER_SAMPLES))
        .collect();
    let rows = par_rows(h, |y| {
        (0..w)
            .map(|x| {
                let n = texel_direction(x, y, w, h);
                let mut sum = [0.0; 3];
                let mut weight = 0.0;
                for &(u, v) in &samples {
                    let hv = sample_ggx(u, v, roughness, n);
                    let nh = n.dot(hv);
                    let l = hv * (2.0 * nh) - n;
                    let nl = n.dot(l);
                    if nl <= 0.0 {
                        continue;
                    }
                    // pdf = D nh / (4 vh) = D / 4 with n = v.
                    let pdf = ggx_d(nh, roughness) / 4.0;
                    let lod = if pdf > 0.0 {
                        let omega_s = 1.0 / (f64::from(PREFILTER_SAMPLES) * pdf);
                        (0.5 * (omega_s / texel_angle).log2() + 1.0).max(0.0)
                    } else {
                        0.0
                    };
                    let c = trilinear(chain, l, lod, 0.0);
                    for (s, c) in sum.iter_mut().zip(c) {
                        *s += c * nl;
                    }
                    weight += nl;
                }
                if weight > 0.0 {
                    sum.map(|s| (s / weight) as f32)
                } else {
                    [0.0; 3]
                }
            })
            .collect()
    });
    if rows.len() != w * h {
        return Err(RenderError::Environment(
            "prefiltering the environment failed".to_string(),
        ));
    }
    Ok(Level {
        width: w,
        height: h,
        texels: rows,
    })
}
