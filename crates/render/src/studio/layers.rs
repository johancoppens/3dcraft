//! Layer 3: the layers of a material beyond its colour, metallic and
//! roughness (docs/design/rendering.md, *Layer 3: materials*), as both
//! renderers shade them. [`Optics`] is what a face carries
//! ([`crate::FaceColors::set_optics`]); the functions here are the closed
//! forms the shading uses, each written once and repeated by the GPU's
//! `studio.wgsl`.
//!
//! - **Anisotropy** (Filament, *Physically Based Rendering in Filament*,
//!   § Anisotropic model, after Kulla and Conty 2017 and Heitz 2014): GGX
//!   with `αt = α (1 + a)` along the tangent and `αb = α (1 − a)` across
//!   it, the anisotropic height-correlated visibility; the environment
//!   along Filament's bent reflection, the key light with the anisotropic
//!   lobe instead of its share of the prefiltered environment.
//! - **Clearcoat** (Burley 2012; Kulla and Conty 2017; Filament §
//!   Clear coat model): a dielectric layer (`F0` = 0.04) of its own
//!   roughness over the base; the base is weakened by the coat's
//!   directional albedo `c (F0 A + B)` at the view direction.
//! - **Transmission** (Filament § Transmission and refraction; Walter et
//!   al. 2007 for the rough interface, whose lobe is not traced here): the
//!   share `T` of the diffuse light that passes through instead, tinted by
//!   the base colour and by Beer–Lambert absorption, seeing the picture
//!   without the transmissive faces where the refracted ray leaves the
//!   body (a solid), or straight behind (thin-walled).
//! - **Subsurface** (a wrap profile): the diffuse light of a normal-wrap
//!   kernel `max(0, n·l + w) / (1 + w)²`, which keeps the energy of the
//!   Lambertian one (both integrate to π over the sphere), tinted by the
//!   subsurface colour.

use super::DIELECTRIC_F0;
use craft3d_math::Vec3;
use std::f64::consts::PI;

/// Most distinct [`Optics`] one picture holds (the GPU's material table
/// has a row per one, and 2048 rows is what every device has).
pub const MAX_OPTICS: usize = 1024;
/// The smallest GGX `α` the anisotropic lobe takes along either axis.
pub const MIN_ALPHA: f64 = 0.002;
/// A tangent field gives no direction at a point where its projection on
/// the tangent plane is shorter than this share of its length (a field
/// along the normal, a point on the axis of [`TangentField::Around`]):
/// the point is shaded without anisotropy there.
pub const TANGENT_EPSILON: f64 = 1e-4;
/// The index of refraction of a material without one of its own.
pub const DEFAULT_IOR: f64 = 1.5;
/// The most and the least index of refraction [`Optics`] keeps.
pub const MAX_IOR: f64 = 3.0;
/// The least share of light an absorption colour keeps per channel
/// (linear): a black channel absorbs as much as this.
pub const MIN_ABSORBED: f64 = 1e-4;

/// The direction of a face's anisotropy (its tangent `t`; `b = n × t`),
/// in the model's axes.
#[derive(Clone, Copy, Debug)]
pub enum TangentField {
    /// A fixed axis, projected on the tangent plane at each point (the
    /// `u` direction of a plane, a given brushing direction).
    Axis([f32; 3]),
    /// Around an axis through `origin`: `axis × (p − origin)` (the `u`
    /// direction of a cylinder, cone, sphere or torus: turned parts).
    Around { origin: [f64; 3], axis: [f32; 3] },
}

impl TangentField {
    fn bits(&self) -> (u8, [u64; 3], [u32; 3]) {
        match *self {
            TangentField::Axis(a) => (0, [0; 3], a.map(f32::to_bits)),
            TangentField::Around { origin, axis } => {
                (1, origin.map(f64::to_bits), axis.map(f32::to_bits))
            }
        }
    }

    /// The unit tangent at `p` with the unit normal `n`, or `None` where
    /// the field gives none ([`TANGENT_EPSILON`]).
    pub fn at(&self, p: Vec3, n: Vec3) -> Option<Vec3> {
        let v3 = |a: [f32; 3]| Vec3::new(f64::from(a[0]), f64::from(a[1]), f64::from(a[2]));
        let raw = match *self {
            TangentField::Axis(a) => v3(a),
            TangentField::Around { origin, axis } => {
                v3(axis).cross(p - Vec3::new(origin[0], origin[1], origin[2]))
            }
        };
        let along = raw - n * n.dot(raw);
        let (len, full) = (along.length(), raw.length());
        if !(len.is_finite() && full.is_finite() && len > TANGENT_EPSILON * full) {
            return None;
        }
        along.normalized()
    }
}

/// The layers of a face's material beyond its colour and [`crate::Finish`],
/// in `f32` so that both renderers shade from the same numbers. The
/// default has none of them; [`Optics::sanitized`] brings any value into
/// range.
#[derive(Clone, Copy, Debug)]
pub struct Optics {
    /// −1 to 1: positive widens the lobe along the tangent.
    pub anisotropy: f32,
    pub tangent: TangentField,
    /// The clear coat's strength, 0 to 1, and its roughness.
    pub clearcoat: f32,
    pub clearcoat_roughness: f32,
    /// The share of the diffuse light that passes through, 0 to 1.
    pub transmission: f32,
    /// The index of refraction, 1 to [`MAX_IOR`].
    pub ior: f32,
    /// A thin sheet: the light passes straight through, not refracted.
    pub thin_walled: bool,
    /// The absorption coefficient per channel, per model unit of length
    /// travelled inside ([`absorption_coefficients`]); 0 for clear.
    pub absorption: [f32; 3],
    /// The subsurface wrap `w`, 0 to 1, and the colour of the light that
    /// scatters (the base colour when `None`).
    pub subsurface: f32,
    pub subsurface_color: Option<[u8; 3]>,
}

impl Default for Optics {
    fn default() -> Self {
        Optics {
            anisotropy: 0.0,
            tangent: TangentField::Axis([1.0, 0.0, 0.0]),
            clearcoat: 0.0,
            clearcoat_roughness: 0.0,
            transmission: 0.0,
            ior: DEFAULT_IOR as f32,
            thin_walled: false,
            absorption: [0.0; 3],
            subsurface: 0.0,
            subsurface_color: None,
        }
    }
}

impl PartialEq for Optics {
    fn eq(&self, other: &Self) -> bool {
        let bits = |o: &Optics| {
            (
                o.anisotropy.to_bits(),
                o.tangent.bits(),
                [o.clearcoat, o.clearcoat_roughness, o.transmission, o.ior].map(f32::to_bits),
                o.thin_walled,
                o.absorption.map(f32::to_bits),
                o.subsurface.to_bits(),
                o.subsurface_color,
            )
        };
        bits(self) == bits(other)
    }
}

impl Eq for Optics {}

/// `v` within `lo..=hi`, `fallback` when it is not a number.
fn within(v: f32, lo: f32, hi: f32, fallback: f32) -> f32 {
    if v.is_finite() {
        v.clamp(lo, hi)
    } else {
        fallback
    }
}

impl Optics {
    /// Every value brought into its range; one that is not a number takes
    /// the default's. A field that is not finite becomes the `x` axis.
    pub fn sanitized(self) -> Optics {
        let finite3 = |a: [f32; 3]| a.iter().all(|c| c.is_finite());
        let tangent = match self.tangent {
            TangentField::Axis(a) if finite3(a) => TangentField::Axis(a),
            TangentField::Around { origin, axis }
                if finite3(axis) && origin.iter().all(|c| c.is_finite()) =>
            {
                TangentField::Around { origin, axis }
            }
            _ => TangentField::Axis([1.0, 0.0, 0.0]),
        };
        Optics {
            anisotropy: within(self.anisotropy, -1.0, 1.0, 0.0),
            tangent,
            clearcoat: within(self.clearcoat, 0.0, 1.0, 0.0),
            clearcoat_roughness: within(self.clearcoat_roughness, 0.0, 1.0, 0.0),
            transmission: within(self.transmission, 0.0, 1.0, 0.0),
            ior: within(self.ior, 1.0, MAX_IOR as f32, DEFAULT_IOR as f32),
            thin_walled: self.thin_walled,
            absorption: self.absorption.map(|c| within(c, 0.0, f32::MAX, 0.0)),
            subsurface: within(self.subsurface, 0.0, 1.0, 0.0),
            subsurface_color: self.subsurface_color,
        }
    }

    /// True when it changes nothing: the face is shaded as without it.
    pub fn is_plain(&self) -> bool {
        self.anisotropy == 0.0
            && self.clearcoat == 0.0
            && self.transmission == 0.0
            && self.subsurface == 0.0
    }

    /// True for a face that lets light through: it casts no shadow and is
    /// left out of the picture its transmission sees.
    pub fn is_transmissive(&self) -> bool {
        self.transmission > 0.0
    }
}

/// The absorption coefficients (per unit of length) of a medium that
/// leaves the light the colour `color` (sRGB) after `distance` units:
/// `−ln(c) / distance` per channel with `c` decoded to linear light and
/// at least [`MIN_ABSORBED`]. Zeros for a distance that is not a positive
/// number.
pub fn absorption_coefficients(color: [u8; 3], distance: f64) -> [f32; 3] {
    if !(distance.is_finite() && distance > 0.0) {
        return [0.0; 3];
    }
    color.map(|b| {
        let c = super::tonemap::byte_to_linear(b).max(MIN_ABSORBED);
        // Deliberate narrowing: the coefficient both renderers read.
        (-c.ln() / distance) as f32
    })
}

/// How the wrapped kernel `max(0, t + w) / (1 + w)²` weighs the three
/// bands of the spherical harmonics against the clamped cosine's (whose
/// weights are π, 2π/3 and π/4): `1`, `(1 + 3w/2 − w³/2) / (1 + w)²` and
/// `(1 − w)²`, from `2π ∫ k(t) P_l(t) dt` over `−w ≤ t ≤ 1`.
pub fn wrap_bands(w: f64) -> [f64; 3] {
    let w = w.clamp(0.0, 1.0);
    let s = (1.0 + w) * (1.0 + w);
    [
        1.0,
        (1.0 + 1.5 * w - 0.5 * w * w * w) / s,
        (1.0 - w) * (1.0 - w),
    ]
}

/// The wrapped cosine of a direction at the cosine `nl` to the normal:
/// `max(0, nl + w) / (1 + w)²`.
pub fn wrapped_cosine(nl: f64, w: f64) -> f64 {
    let w = w.clamp(0.0, 1.0);
    (nl + w).max(0.0) / ((1.0 + w) * (1.0 + w))
}

/// The anisotropic roughnesses along and across the tangent for the GGX
/// `α` and anisotropy `a`: `max(α (1 ± a), MIN_ALPHA)`.
pub fn aniso_alphas(alpha: f64, a: f64) -> (f64, f64) {
    (
        (alpha * (1.0 + a)).max(MIN_ALPHA),
        (alpha * (1.0 - a)).max(MIN_ALPHA),
    )
}

/// The anisotropic GGX `D` times the anisotropic height-correlated
/// visibility `V` (Filament): with `t`, `b`, `n` an orthonormal frame,
/// `h` the half vector, `v` towards the viewer and `l` towards the light.
#[allow(clippy::too_many_arguments)]
pub fn aniso_ggx_dv(at: f64, ab: f64, t: Vec3, b: Vec3, n: Vec3, h: Vec3, v: Vec3, l: Vec3) -> f64 {
    let (th, bh, nh) = (t.dot(h), b.dot(h), n.dot(h));
    let k = (th / at) * (th / at) + (bh / ab) * (bh / ab) + nh * nh;
    let d = 1.0 / (PI * at * ab * k * k);
    let (nv, nl) = (n.dot(v).clamp(1e-4, 1.0), n.dot(l).clamp(0.0, 1.0));
    let lv = nl * Vec3::new(at * t.dot(v), ab * b.dot(v), nv).length();
    let ll = nv * Vec3::new(at * t.dot(l), ab * b.dot(l), nl).length();
    let vis = 0.5 / (lv + ll);
    let out = d * vis;
    if out.is_finite() { out.max(0.0) } else { 0.0 }
}

/// Filament's bent reflection for anisotropic image-based light: the
/// normal bent towards the one the stretched lobe sees,
/// `normalize(mix(n, n_a, |a| · saturate(5 r)))`, and `v` reflected about
/// it; `r` is the perceptual roughness.
pub fn bent_reflection(n: Vec3, v: Vec3, t: Vec3, b: Vec3, a: f64, r: f64) -> Vec3 {
    let direction = if a >= 0.0 { b } else { t };
    let across = direction.cross(v);
    let normal = across.cross(direction);
    let bend = a.abs() * (5.0 * r).clamp(0.0, 1.0);
    let bent = (n * (1.0 - bend) + normal * bend).normalized().unwrap_or(n);
    bent * (2.0 * bent.dot(v)) - v
}

/// The direction of the ray `d` (unit, towards the surface) refracted at
/// a surface with the unit normal `n` (facing against `d`) from a medium
/// of index 1 into one of index `ior`; `None` for total internal
/// reflection (which needs `ior < 1`).
pub fn refract(d: Vec3, n: Vec3, ior: f64) -> Option<Vec3> {
    let eta = 1.0 / ior;
    let c = -n.dot(d);
    let k = 1.0 - eta * eta * (1.0 - c * c);
    if !(k.is_finite() && k >= 0.0) {
        return None;
    }
    (d * eta + n * (eta * c - k.sqrt())).normalized()
}

/// The length of the refracted path through a slab that the view ray
/// crosses over `across` units: `across · cos θi / cos θt`, `cos θi = n·v`,
/// `cos θt = −n·r` (the slab's two faces taken as parallel at the point).
pub fn slab_path(across: f64, nv: f64, n: Vec3, refracted: Vec3) -> f64 {
    let cos_t = -n.dot(refracted);
    if !(cos_t.is_finite() && cos_t > 1e-6) {
        return across;
    }
    across * nv / cos_t
}

/// The lateral offset of a ray through a slab of thickness `thickness`
/// and index `ior` at the angle of incidence `theta`:
/// `t sin θ (1 − cos θ / √(n² − sin² θ))` (the textbook closed form; the
/// tests hold the screen-space refraction to it).
pub fn slab_offset(thickness: f64, ior: f64, theta: f64) -> f64 {
    let (s, c) = theta.sin_cos();
    thickness * s * (1.0 - c / (ior * ior - s * s).sqrt())
}

/// The coat's directional albedo `c · (F0 A + B)` at the split sum's
/// `(A, B)` for its roughness, `F0` = 0.04.
pub fn coat_albedo(clearcoat: f64, (a, b): (f64, f64)) -> f64 {
    (clearcoat * (DIELECTRIC_F0 * a + b)).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Midpoint quadrature of `2π ∫ k(t) P_l(t) dt` over `[-1, 1]`.
    fn band_weight(w: f64, l: usize) -> f64 {
        let n = 200_000;
        let mut sum = 0.0;
        for i in 0..n {
            let t = -1.0 + (i as f64 + 0.5) * 2.0 / n as f64;
            let p = match l {
                0 => 1.0,
                1 => t,
                _ => 0.5 * (3.0 * t * t - 1.0),
            };
            sum += wrapped_cosine(t, w) * p * 2.0 / n as f64;
        }
        2.0 * PI * sum
    }

    #[test]
    fn the_wrap_bands_are_the_kernels_harmonics() {
        let lambert = [PI, 2.0 * PI / 3.0, PI / 4.0];
        for w in [0.0, 0.25, 0.5, 1.0] {
            let bands = wrap_bands(w);
            for l in 0..3 {
                let expected = band_weight(w, l) / lambert[l];
                assert!(
                    (bands[l] - expected).abs() < 1e-6,
                    "w {w} band {l}: {} against {expected}",
                    bands[l]
                );
            }
        }
    }

    #[test]
    fn refraction_follows_snell() {
        let n = Vec3::Z;
        let theta: f64 = 0.6;
        let d = Vec3::new(theta.sin(), 0.0, -theta.cos());
        let r = refract(d, n, 1.5).unwrap();
        let sin_t = r.x;
        assert!((theta.sin() - 1.5 * sin_t).abs() < 1e-12);
        assert!(r.z < 0.0);
    }

    #[test]
    fn a_field_along_the_normal_gives_no_tangent() {
        let field = TangentField::Axis([0.0, 0.0, 1.0]);
        assert!(field.at(Vec3::ZERO, Vec3::Z).is_none());
        let around = TangentField::Around {
            origin: [0.0; 3],
            axis: [0.0, 0.0, 1.0],
        };
        let t = around.at(Vec3::new(2.0, 0.0, 0.0), Vec3::X).unwrap();
        assert!((t - Vec3::Y).length() < 1e-12);
        assert!(around.at(Vec3::ZERO, Vec3::X).is_none());
    }

    #[test]
    fn sanitizing_brings_every_value_into_range() {
        let wild = Optics {
            anisotropy: f32::NAN,
            tangent: TangentField::Axis([f32::INFINITY, 0.0, 0.0]),
            clearcoat: 7.0,
            clearcoat_roughness: -1.0,
            transmission: f32::INFINITY,
            ior: 0.2,
            thin_walled: true,
            absorption: [f32::NAN, -3.0, 2.0],
            subsurface: 2.0,
            subsurface_color: None,
        }
        .sanitized();
        assert_eq!(wild.anisotropy, 0.0);
        assert_eq!(wild.clearcoat, 1.0);
        assert_eq!(wild.clearcoat_roughness, 0.0);
        assert_eq!(wild.transmission, 0.0);
        assert_eq!(wild.ior, 1.0);
        assert_eq!(wild.absorption, [0.0, 0.0, 2.0]);
        assert_eq!(wild.subsurface, 1.0);
        assert!(Optics::default().is_plain());
    }
}
