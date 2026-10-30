//! The studio look: image-based lighting, soft shadows from the key
//! light, ambient occlusion, a ground that catches shadows and a tone
//! curve (docs/design/rendering.md, *The studio look*). The plain look
//! ([`crate::render_colored`]) is untouched by it.
//!
//! Per sample of the picture, as [`render_studio`] does it and the GPU
//! renderer repeats it:
//!
//! 1. The nearest surface, as in the plain picture (the same coverage,
//!    ties and ids), with its normal and view distance.
//! 2. Its ambient occlusion `ao` ([`ao::occlusion`]) when asked for.
//! 3. Its light, linear, from the material of the face: the base colour
//!    (sRGB bytes decoded), metallic `m` and roughness `r` from its
//!    [`crate::Finish`] (a face without a roughness of its own is at
//!    [`DEFAULT_ROUGHNESS`]). With `n` facing the viewer, `v` towards the
//!    viewer, `F0 = 0.04 (1 − m) + base · m`, `(A, B)` the split sum at
//!    `n·v` and `r`, `Fs = F0 A + B` and the multiple-scattering factor
//!    `1 + F0 (1 / (A + B) − 1)` (the energy compensation of Google
//!    Filament's *Physically Based Rendering in Filament*, § Energy loss
//!    in specular reflectance, after Kulla and Conty, *Revisiting
//!    Physically Based Shading at Imageworks*, 2017):
//!    - specular: the prefiltered environment along the reflection at
//!      level `r · (levels − 1)`, times `Fs`, the factor, and the
//!      specular occlusion of `ao` (Lagarde and de Rousiers, *Moving
//!      Frostbite to PBR*, 2014);
//!    - diffuse: `base (1 − m)(1 − Fs) / π` times the irradiance of the
//!      environment without its key (spherical harmonics) times `ao`,
//!      plus the key's irradiance times `n·l` times its shadow.
//! 4. Where no surface is: the environment blurred
//!    ([`env::BACKGROUND_ROUGHNESS`]), or with [`Look::plain_background`]
//!    the picture's own colour, darkened where the ray meets the ground
//!    under the model by the share of the light the model keeps from it.
//! 5. Exposure (`2^EV`), the tone curve, the sRGB encoding, rounded to
//!    bytes per sample; the samples of a pixel are averaged as in the
//!    plain picture.

pub mod ao;
pub mod brdf;
pub mod bundled;
pub mod env;
pub mod hdr;
pub mod polish;
pub mod shadow;
pub mod tonemap;

pub use env::{Key, Level, Prepared};
pub use hdr::{HdrError, HdrImage, decode_hdr};
pub use tonemap::Tonemap;

use crate::{
    Camera, FaceColors, Finish, Frame, MAX_SAMPLES, MAX_SIDE, MAX_SSAA, Mesh, NO_FACE, Projection,
    RenderError, RenderParams, View, edge, render_colored, sample_range, section_bounds,
    shared_edge,
};
use craft3d_math::{Bbox3, Vec3};
pub use shadow::{LightView, ShadowMap};
use std::f64::consts::PI;

/// The roughness that marks a sample of the plain background in the
/// screen-space buffers (−1 marks the environment's).
pub const PLAIN_BACKDROP: f64 = -2.0;
/// The roughness of a face without one of its own.
pub const DEFAULT_ROUGHNESS: f64 = 0.5;
/// The reflectance at normal incidence of a dielectric.
pub const DIELECTRIC_F0: f64 = 0.04;

/// How a studio picture is lit: view state, never document data.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    /// Exposure in stops: the light is multiplied by `2^exposure`.
    pub exposure: f64,
    /// The environment turned about z, in degrees.
    pub rotation: f64,
    /// A ground under the model that catches its shadow.
    pub ground: bool,
    /// Ambient occlusion.
    pub ao: bool,
    /// Shadows from the key light.
    pub shadows: bool,
    pub tonemap: Tonemap,
    /// The key light from a given direction (azimuth from +x towards +y
    /// and elevation, in degrees, in the model's axes) instead of the
    /// environment's brightest one; its strength stays the
    /// environment's.
    pub light: Option<[f64; 2]>,
    /// Layer 2 (`polish`): screen-space reflections, falling back to the
    /// environment.
    pub ssr: bool,
    /// Contact shadows: a short screen-space ray towards the key light.
    pub contact_shadows: bool,
    /// The key's shadow darkens its share of the specular light too.
    pub specular_shadow: bool,
    /// How much the ground reflects the model (0: not at all, 1: a
    /// dielectric's Fresnel reflectance at full strength).
    pub ground_reflection: f64,
    /// The share of the light spread by bloom (0: none).
    pub bloom: f64,
    /// Depth of field of a thin lens (perspective pictures only).
    pub dof: Option<polish::Dof>,
    /// Frames averaged into a still picture (1: one frame).
    pub accumulate: u32,
    /// The background is the picture's own plain colour
    /// (`RenderParams::background`, the theme's) instead of the
    /// environment, which still lights the model; the ground's shadow
    /// darkens it.
    pub plain_background: bool,
}

impl Default for Look {
    fn default() -> Self {
        Look {
            exposure: 0.0,
            rotation: 0.0,
            ground: true,
            ao: true,
            shadows: true,
            tonemap: Tonemap::Agx,
            light: None,
            ssr: false,
            contact_shadows: false,
            specular_shadow: false,
            ground_reflection: 0.0,
            bloom: 0.0,
            dof: None,
            accumulate: 1,
            plain_background: false,
        }
    }
}

impl Look {
    /// The studio look with layer 2's polish on: reflections, contact
    /// shadows, the specular shadow and bloom (no reflecting ground, see
    /// below, and no depth of field: it needs a focus), one frame.
    pub fn polished() -> Look {
        Look {
            ssr: true,
            contact_shadows: true,
            specular_shadow: true,
            // Off: the ground has no edge, and seen at a grazing angle
            // towards the horizon it reflects the sky almost wholly, which
            // lifts the whole background (docs/design/rendering.md, *Layer 2*).
            ground_reflection: 0.0,
            bloom: polish::BLOOM_STRENGTH,
            ..Look::default()
        }
    }

    /// Whether a frame needs the screen-space buffers after shading
    /// (reflections, depth of field, bloom).
    pub fn post(&self) -> bool {
        self.ssr || self.ground_reflection > 0.0 || self.bloom > 0.0 || self.dof.is_some()
    }

    /// The rotation in radians, 0 when it is not a number.
    pub fn rotation_radians(&self) -> f64 {
        if self.rotation.is_finite() {
            self.rotation.rem_euclid(360.0).to_radians()
        } else {
            0.0
        }
    }

    /// `2^exposure`, within 2^±20.
    pub fn exposure_scale(&self) -> f64 {
        if self.exposure.is_finite() {
            self.exposure.clamp(-20.0, 20.0).exp2()
        } else {
            1.0
        }
    }

    /// The unit vector towards the key light in the model's axes, if the
    /// environment has one.
    pub fn key_direction(&self, env: &Prepared) -> Option<Vec3> {
        let key = env.key?;
        if let Some([azimuth, elevation]) = self.light
            && azimuth.is_finite()
            && elevation.is_finite()
        {
            let (a, e) = (
                azimuth.to_radians(),
                elevation.clamp(-90.0, 90.0).to_radians(),
            );
            return Vec3::new(e.cos() * a.cos(), e.cos() * a.sin(), e.sin()).normalized();
        }
        let [x, y, z] = key.direction.map(f64::from);
        rotate_z(Vec3::new(x, y, z), self.rotation_radians()).normalized()
    }
}

/// `d` turned by `angle` radians about z.
pub fn rotate_z(d: Vec3, angle: f64) -> Vec3 {
    let (s, c) = angle.sin_cos();
    Vec3::new(c * d.x - s * d.y, s * d.x + c * d.y, d.z)
}

/// What the shading of a frame needs besides the sample: computed once
/// per frame, the same numbers the GPU's uniform block carries.
pub struct FrameLight<'a> {
    pub env: &'a Prepared,
    pub look: Look,
    pub rotation: f64,
    pub exposure: f64,
    pub key: Option<(Vec3, [f64; 3], f64)>,
    pub shadow: Option<ShadowMap>,
    /// The height of the ground, when there is one.
    pub ground: Option<f64>,
    pub rest_up: f64,
    pub key_luminance: f64,
}

/// The numbers of a frame's light that do not depend on the samples: the
/// key light in the model's axes, the light's view over the mesh's box,
/// the ground's height and the two luminances the ground's shadow is
/// weighed with. The GPU renderer takes them from here too.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightNumbers {
    /// Towards the key light, its irradiance and angular radius.
    pub key: Option<(Vec3, [f64; 3], f64)>,
    /// The light's view, when there are shadows.
    pub view: Option<LightView>,
    pub ground: Option<f64>,
    /// The luminance of the irradiance without the key on an upward face.
    pub rest_up: f64,
    /// The luminance of the key's irradiance.
    pub key_luminance: f64,
}

impl LightNumbers {
    /// The numbers for a mesh within `bounds` (its triangles' corners).
    pub fn new(bounds: &Bbox3, env: &Prepared, look: &Look) -> LightNumbers {
        let key = look.key_direction(env).zip(env.key).map(|(d, k)| {
            (
                d,
                k.irradiance.map(f64::from),
                f64::from(k.radius).clamp(0.0, 1.0),
            )
        });
        let view = key
            .filter(|_| look.shadows)
            .and_then(|(d, _, _)| LightView::new(bounds, d));
        let rotation = look.rotation_radians();
        LightNumbers {
            key,
            view,
            ground: look
                .ground
                .then(|| bounds.corners().map(|(lo, _)| lo.z))
                .flatten(),
            rest_up: env::luminance(env.irradiance(rotate_z(Vec3::Z, -rotation))),
            key_luminance: key.map_or(0.0, |(_, e, _)| env::luminance(e)),
        }
    }
}

impl<'a> FrameLight<'a> {
    /// The light of a frame of `mesh`.
    pub fn new(mesh: &Mesh, env: &'a Prepared, look: &Look) -> FrameLight<'a> {
        let numbers = LightNumbers::new(&section_bounds(mesh), env, look);
        FrameLight {
            env,
            look: *look,
            rotation: look.rotation_radians(),
            exposure: look.exposure_scale(),
            key: numbers.key,
            shadow: numbers.view.map(|view| ShadowMap::draw(mesh, view)),
            ground: numbers.ground,
            rest_up: numbers.rest_up,
            key_luminance: numbers.key_luminance,
        }
    }

    /// The environment along `d` (model axes) at a fractional level.
    pub fn radiance(&self, d: Vec3, lod: f64) -> [f64; 3] {
        env::trilinear(&self.env.levels, d, lod, self.rotation)
    }

    /// The irradiance without the key on a surface of normal `n`.
    pub fn irradiance(&self, n: Vec3) -> [f64; 3] {
        self.env.irradiance(rotate_z(n, -self.rotation))
    }

    fn top_level(&self) -> f64 {
        self.env.levels.len().saturating_sub(1) as f64
    }

    /// The linear light leaving a surface point `p` with unit normal `n`
    /// (facing the viewer) towards the viewer `v`, for a base colour and
    /// finish, with ambient occlusion `ao`.
    pub fn shade(
        &self,
        base: [u8; 3],
        finish: Finish,
        p: Vec3,
        n: Vec3,
        v: Vec3,
        ao: f64,
    ) -> [f64; 3] {
        self.shade_parts(base, finish, p, n, v, ao, 1.0).light
    }

    /// [`Self::shade`] with the key's contact shadow `contact` (1: none),
    /// and the specular weight the reflections need.
    #[allow(clippy::too_many_arguments)]
    pub fn shade_parts(
        &self,
        base: [u8; 3],
        finish: Finish,
        p: Vec3,
        n: Vec3,
        v: Vec3,
        ao: f64,
        contact: f64,
    ) -> Shaded {
        let base = base.map(tonemap::byte_to_linear);
        let m = f64::from(finish.metallic) / 255.0;
        let r = finish
            .roughness
            .map_or(DEFAULT_ROUGHNESS, |r| f64::from(r) / 255.0);
        let nv = n.dot(v).clamp(1e-4, 1.0);
        let reflected = n * (2.0 * n.dot(v)) - v;
        let (a, b) = self.env.split_sum(nv, r);
        let prefiltered = self.radiance(reflected, r * self.top_level());
        let irradiance = self.irradiance(n);
        // The key light's share and its shadow.
        let (key_light, key_dot, key_vis, key_nl) = match self.key {
            Some((l, e, radius)) => {
                let nl = n.dot(l).max(0.0);
                let vis = match &self.shadow {
                    Some(map) if nl > 0.0 => map.visibility(p, n, radius),
                    _ => 1.0,
                } * contact;
                (e, nl * vis, vis, nl)
            }
            None => ([0.0; 3], 0.0, 1.0, 0.0),
        };
        // Lagarde and de Rousiers' specular occlusion, with their
        // roughness α = r² (as Filament).
        let occlusion = (nv + ao).powf((-16.0 * r * r - 1.0).exp2()) - 1.0 + ao;
        let specular_occlusion = occlusion.clamp(0.0, 1.0);
        let sum_ab = a + b;
        let mut out = [0.0; 3];
        let mut weight = [0.0; 3];
        let mut key_specular = [0.0; 3];
        let mut env_specular = [0.0; 3];
        for k in 0..3 {
            let c = base.get(k).copied().unwrap_or(0.0);
            let f0 = DIELECTRIC_F0 * (1.0 - m) + c * m;
            let fs = f0 * a + b;
            let multiple = if sum_ab > 0.0 {
                1.0 + f0 * (1.0 / sum_ab - 1.0)
            } else {
                1.0
            };
            let w = fs * multiple * specular_occlusion;
            let specular =
                prefiltered.get(k).copied().unwrap_or(0.0) * fs * multiple * specular_occlusion;
            let light = irradiance.get(k).copied().unwrap_or(0.0) * ao
                + key_light.get(k).copied().unwrap_or(0.0) * key_dot;
            let diffuse = c * (1.0 - m) * (1.0 - fs) / PI * light;
            if let Some(o) = out.get_mut(k) {
                *o = diffuse;
            }
            if let Some(o) = weight.get_mut(k) {
                *o = w;
            }
            if let Some(o) = env_specular.get_mut(k) {
                *o = specular;
            }
            if let (Some(o), Some(&e), Some((l, _, radius))) =
                (key_specular.get_mut(k), key_light.get(k), self.key)
                && self.look.specular_shadow
                && key_vis < 1.0
                && key_nl > 0.0
            {
                *o = e * key_nl * polish_key_specular(n, v, l, r, radius, f0) * multiple;
            }
        }
        // The specular shadow: the key is part of the prefiltered light;
        // where it is hidden its share of that light is taken out.
        let total = env::luminance(env_specular);
        let share = if total > 0.0 {
            (env::luminance(key_specular) / total).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let keep = 1.0 - (1.0 - key_vis) * share;
        for k in 0..3 {
            if let (Some(o), Some(w)) = (out.get_mut(k), weight.get_mut(k)) {
                *w *= keep;
                *o += env_specular.get(k).copied().unwrap_or(0.0) * keep;
            }
        }
        Shaded {
            light: out,
            weight,
            roughness: r,
        }
    }

    /// The light of the background along the ray from `origin` in the
    /// unit direction `d`: the environment blurred, with the ground's
    /// shadow where the ray meets it.
    pub fn background(&self, origin: Vec3, d: Vec3) -> [f64; 3] {
        let c = self.radiance(d, env::BACKGROUND_ROUGHNESS * self.top_level());
        let factor = self.ground_factor(origin, d);
        c.map(|v| v * factor)
    }

    /// The bytes of the background along the ray from `origin` in the unit
    /// direction `d`: the environment through the tone curve, or, with a
    /// plain background, the colour `plain` darkened by the ground's
    /// shadow in linear light and encoded again (no exposure, no tone
    /// curve: unshadowed, it is `plain` exactly).
    pub fn background_bytes(&self, plain: [u8; 3], origin: Vec3, d: Vec3) -> [u8; 3] {
        if self.look.plain_background {
            let factor = self.ground_factor(origin, d);
            plain.map(|b| {
                tonemap::to_byte(tonemap::srgb_encode(tonemap::byte_to_linear(b) * factor))
            })
        } else {
            self.to_bytes(self.background(origin, d))
        }
    }

    /// The share of the ground's light the model leaves it where the ray
    /// meets it: `(E + K · l_z · vis) / (E + K · l_z)` with `E` the
    /// luminance of the irradiance without the key on an upward face and
    /// `K` the key's; 1 off the ground.
    pub fn ground_factor(&self, origin: Vec3, d: Vec3) -> f64 {
        let (Some(z), Some(map), Some((l, _, radius))) = (self.ground, &self.shadow, self.key)
        else {
            return 1.0;
        };
        if !(d.z < 0.0 && origin.z > z) {
            return 1.0;
        }
        let t = (z - origin.z) / d.z;
        if !(t.is_finite() && t > 0.0) {
            return 1.0;
        }
        let g = origin + d * t;
        let lz = l.z.max(0.0);
        let rest = self.rest_up;
        let key = self.key_luminance * lz;
        // Not above zero, NaN included.
        if (rest + key).partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) || lz <= 0.0 {
            return 1.0;
        }
        let vis = map.visibility(g, Vec3::Z, radius);
        (rest + key * vis) / (rest + key)
    }

    /// A sample's linear light to bytes: through the tone curve, or for
    /// the plain background (roughness [`PLAIN_BACKDROP`]) without it,
    /// `srgb(L · 2^EV)`, which gives back its colour where nothing
    /// changed it.
    pub fn display(&self, c: [f64; 3], roughness: f64) -> [u8; 3] {
        if roughness == PLAIN_BACKDROP {
            c.map(|v| tonemap::to_byte(tonemap::srgb_encode(v * self.exposure)))
        } else {
            self.to_bytes(c)
        }
    }

    /// Linear light to a byte triple.
    pub fn to_bytes(&self, c: [f64; 3]) -> [u8; 3] {
        let scaled = c.map(|v| v * self.exposure);
        tonemap::display(scaled, self.look.tonemap).map(tonemap::to_byte)
    }
}

/// A surface point's light ([`FrameLight::shade_parts`]): the linear
/// light, and the weight of the environment's specular light in it per
/// channel with the roughness, which the reflections replace.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shaded {
    pub light: [f64; 3],
    pub weight: [f64; 3],
    pub roughness: f64,
}

/// The key's specular light per unit irradiance and `n·l` at `F0`: GGX
/// with the lobe widened by half the key's angular radius (as Karis 2013
/// widens it for a sphere light), the height-correlated visibility and
/// Schlick's Fresnel.
fn polish_key_specular(n: Vec3, v: Vec3, l: Vec3, r: f64, radius: f64, f0: f64) -> f64 {
    let Some(hv) = (v + l).normalized() else {
        return 0.0;
    };
    let alpha = (r * r + 0.5 * radius).clamp(0.002, 1.0);
    let (nh, nv, nl, vh) = (
        n.dot(hv).clamp(0.0, 1.0),
        n.dot(v).clamp(1e-4, 1.0),
        n.dot(l).clamp(0.0, 1.0),
        v.dot(hv).clamp(0.0, 1.0),
    );
    let f = f0 + (1.0 - f0) * (1.0 - vh).powi(5);
    polish::ggx_dv(alpha, nh, nv, nl) * f
}

/// A sample's surface: its view distance (infinity for none), normal and
/// face.
#[derive(Clone, Copy)]
pub(crate) struct Surface {
    key: f64,
    dist: f64,
    normal: [f32; 3],
    face: u32,
}

/// [`crate::render_colored`] in the studio look, lit by `env`: the
/// average of [`Look::accumulate`] frames ([`render_studio_frame`]),
/// summed per pixel and rounded once. A see-through or cut-open picture
/// is drawn in the plain look (the studio look of those is still to
/// come). The samples of all frames together may not pass
/// [`crate::MAX_SAMPLES`] ([`RenderError::TooManyFrames`]).
pub fn render_studio(
    mesh: &Mesh,
    colors: &FaceColors,
    camera: &Camera,
    params: &RenderParams,
    look: &Look,
    env: &Prepared,
) -> Result<Frame, RenderError> {
    studio_frames(mesh, colors, camera, params, look, env, false).map(|(frame, _)| frame)
}

/// [`render_studio`] with, per pixel, the share of its light that a
/// decision in doubt may have changed: the mean over its samples and
/// frames of each sample's share, a sample's being 1 where its reflected
/// ray made a decision within [`polish::TRACE_DOUBT`] of its boundary
/// (`polish::Traced`), the share of such samples in the lens's gather
/// where there is a lens, and 0 elsewhere (bloom spreads a sample's light
/// by at most its strength over its pyramid, which is not counted). The
/// GPU makes those decisions in `f32`, so a comparison of the two
/// renderers leaves out the pixels with a large share, as it leaves out
/// the band along silhouettes.
pub fn render_studio_doubt(
    mesh: &Mesh,
    colors: &FaceColors,
    camera: &Camera,
    params: &RenderParams,
    look: &Look,
    env: &Prepared,
) -> Result<(Frame, Vec<f32>), RenderError> {
    studio_frames(mesh, colors, camera, params, look, env, true)
}

fn studio_frames(
    mesh: &Mesh,
    colors: &FaceColors,
    camera: &Camera,
    params: &RenderParams,
    look: &Look,
    env: &Prepared,
    mark: bool,
) -> Result<(Frame, Vec<f32>), RenderError> {
    let plain = || {
        render_colored(mesh, colors, camera, params).map(|frame| {
            let n = frame.ids.len();
            (frame, vec![0.0; n])
        })
    };
    if params.opacity() < 1.0 || params.section.is_some() {
        return plain();
    }
    let frames = look.accumulate.clamp(1, polish::MAX_ACCUMULATE);
    let samples = u64::from(params.width)
        .saturating_mul(u64::from(params.height))
        .saturating_mul(u64::from(params.ssaa))
        .saturating_mul(u64::from(params.ssaa));
    if frames > 1 && samples.saturating_mul(u64::from(frames)) > MAX_SAMPLES {
        return Err(RenderError::TooManyFrames { samples, frames });
    }
    let (mut first, mut doubt) = studio_frame(mesh, colors, camera, params, look, env, 0, mark)?;
    if frames == 1 {
        return Ok((first, doubt));
    }
    let mut sum: Vec<u32> = first.rgb.iter().map(|&b| u32::from(b)).collect();
    for index in 1..frames {
        let (frame, more) = studio_frame(mesh, colors, camera, params, look, env, index, mark)?;
        for (s, &b) in sum.iter_mut().zip(&frame.rgb) {
            *s += u32::from(b);
        }
        for (d, m) in doubt.iter_mut().zip(more) {
            *d += m;
        }
    }
    first.rgb = sum
        .iter()
        .map(|&s| ((s + frames / 2) / frames).min(255) as u8)
        .collect();
    for d in &mut doubt {
        *d /= frames as f32;
    }
    Ok((first, doubt))
}

/// Frame `index` of a studio picture: its samples moved by
/// [`polish::jitter`] and the lens's taps turned for it (frame 0 is the
/// picture without accumulation). Ids and depths are those of the
/// frame's samples.
pub fn render_studio_frame(
    mesh: &Mesh,
    colors: &FaceColors,
    camera: &Camera,
    params: &RenderParams,
    look: &Look,
    env: &Prepared,
    index: u32,
) -> Result<Frame, RenderError> {
    studio_frame(mesh, colors, camera, params, look, env, index, false).map(|(frame, _)| frame)
}

/// [`render_studio_frame`], with each pixel's share in doubt when `mark`
/// (else zeros).
#[allow(clippy::too_many_arguments)]
fn studio_frame(
    mesh: &Mesh,
    colors: &FaceColors,
    camera: &Camera,
    params: &RenderParams,
    look: &Look,
    env: &Prepared,
    index: u32,
    mark: bool,
) -> Result<(Frame, Vec<f32>), RenderError> {
    if params.opacity() < 1.0 || params.section.is_some() {
        return render_colored(mesh, colors, camera, params).map(|frame| {
            let n = frame.ids.len();
            (frame, vec![0.0; n])
        });
    }
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
    let s = ssaa as usize;
    let (w, h) = (width as usize * s, height as usize * s);
    let mut view = View::new(camera, w, h)?;
    view.jitter = polish::jitter(index);
    let surfaces = rasterise(mesh, &view, w, h)?;
    let light = FrameLight::new(mesh, env, look);
    let bounds = section_bounds(mesh);
    let diagonal = bounds.diagonal();
    let radius = diagonal * 0.5 * ao::AO_RADIUS;
    let post = look.post();

    let point = |x: i64, y: i64| -> Option<Vec3> {
        if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
            return None;
        }
        let surface = surfaces.get(y as usize * w + x as usize)?;
        surface
            .dist
            .is_finite()
            .then(|| view.point_at(x as f64 + 0.5, y as f64 + 0.5, surface.dist))
    };
    let shaded: Vec<Shaded> = env::par_rows(h, |y| {
        (0..w)
            .map(|x| {
                let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                let background = |c: [f64; 3]| Shaded {
                    light: c,
                    weight: [0.0; 3],
                    roughness: -1.0,
                };
                let Some(surface) = surfaces.get(y * w + x) else {
                    return background([0.0; 3]);
                };
                let (origin, ray) = view.ray(px, py);
                if surface.face == NO_FACE || !surface.dist.is_finite() {
                    if look.plain_background {
                        // The picture's own colour, displayed without the
                        // tone curve: its light is the colour over the
                        // exposure, marked by the roughness −2. Without
                        // layer 2's post effects it is kept as it is and
                        // shown so, as layer 1 shows it (dividing and
                        // multiplying by the exposure could move a byte).
                        let factor = light.ground_factor(origin, ray);
                        let over = if post { light.exposure } else { 1.0 };
                        let c = params
                            .background
                            .map(|b| tonemap::byte_to_linear(b) * factor / over);
                        return Shaded {
                            light: c,
                            weight: [0.0; 3],
                            roughness: PLAIN_BACKDROP,
                        };
                    }
                    return background(light.background(origin, ray));
                }
                let rel = view.point_at(px, py, surface.dist);
                let p = view.eye + rel;
                let to_viewer = view.toward_viewer(p);
                let [nx, ny, nz] = surface.normal.map(f64::from);
                let n = Vec3::new(nx, ny, nz).normalized().unwrap_or(to_viewer);
                let n = if n.dot(to_viewer) < 0.0 { -n } else { n };
                let ao = if look.ao {
                    let reach = radius / view.sample_size(surface.dist);
                    ao::occlusion(
                        point,
                        (x as i64, y as i64),
                        rel,
                        n,
                        to_viewer,
                        (view.right, view.up),
                        radius,
                        reach.min(h as f64 / 4.0),
                    )
                } else {
                    1.0
                };
                let contact = match light.key {
                    Some((l, _, _)) if look.contact_shadows && n.dot(l) > 0.0 => polish::contact(
                        &view,
                        &surfaces,
                        (w, h),
                        p,
                        l,
                        polish::CONTACT_LENGTH * diagonal,
                        polish::CONTACT_THICKNESS * diagonal,
                        (x, y),
                    ),
                    _ => 1.0,
                };
                let (base, finish) = if params.highlight.contains(surface.face) {
                    (params.highlight.color, Finish::default())
                } else {
                    colors
                        .look(surface.face)
                        .unwrap_or((params.color, Finish::default()))
                };
                light.shade_parts(base, finish, p, n, to_viewer, ao, contact)
            })
            .collect()
    });
    if shaded.len() != w * h {
        return Err(RenderError::Environment(
            "shading the picture failed".to_string(),
        ));
    }
    // Per sample, the share of its light in doubt; empty for none.
    let mut doubt: Vec<[f32; 1]> = Vec::new();
    let rgb: Vec<[u8; 3]> = if post {
        // Deliberate narrowing: the buffers hold f32, as on the GPU.
        let mut buffers = polish::Buffers {
            w,
            h,
            light: shaded.iter().map(|c| c.light.map(|v| v as f32)).collect(),
            weight: shaded
                .iter()
                .map(|c| {
                    let [r, g, b] = c.weight.map(|v| v as f32);
                    [r, g, b, c.roughness as f32]
                })
                .collect(),
        };
        if look.ssr || look.ground_reflection > 0.0 {
            let (lit, unsure) = polish::Reflect {
                view: &view,
                surfaces: &surfaces,
                frame: &light,
                diagonal,
            }
            .apply(&buffers);
            buffers.light = lit;
            if mark && unsure.iter().any(|&d| d) {
                doubt = unsure.iter().map(|&d| [f32::from(u8::from(d))]).collect();
            }
        }
        if let Some(lens) = look
            .dof
            .and_then(|dof| polish::Lens::new(&dof, camera, &bounds, h))
        {
            let depth: Vec<f64> = surfaces.iter().map(|s| s.dist).collect();
            let taps = polish::dof_taps(lens.reach, index);
            let coc = |z: f64| lens.coc(z);
            buffers.light = polish::gather(&buffers.light, &depth, (w, h), &coc, &taps);
            if !doubt.is_empty() {
                doubt = polish::gather(&doubt, &depth, (w, h), &coc, &taps);
            }
        }
        if look.bloom > 0.0 {
            buffers.light = polish::bloom(&buffers.light, w, h, look.bloom);
        }
        buffers
            .light
            .iter()
            .zip(&buffers.weight)
            .map(|(c, w)| light.display(c.map(f64::from), f64::from(w[3])))
            .collect()
    } else {
        shaded
            .iter()
            .map(|c| {
                if c.roughness == PLAIN_BACKDROP {
                    // Layer 1's plain background, exactly.
                    c.light.map(|v| tonemap::to_byte(tonemap::srgb_encode(v)))
                } else {
                    light.display(c.light, c.roughness)
                }
            })
            .collect()
    };
    if rgb.len() != w * h {
        return Err(RenderError::Environment(
            "shading the picture failed".to_string(),
        ));
    }

    let (ow, oh) = (width as usize, height as usize);
    let mut frame = Frame {
        width,
        height,
        rgb: Vec::with_capacity(ow * oh * 3),
        ids: Vec::with_capacity(ow * oh),
        depth: Vec::with_capacity(ow * oh),
        sample: 0.5 / f64::from(ssaa),
    };
    let per_pixel = (s * s) as u32;
    let mut shares = Vec::with_capacity(if mark { ow * oh } else { 0 });
    for oy in 0..oh {
        for ox in 0..ow {
            if mark {
                let mut share = 0.0_f32;
                for dy in 0..s {
                    let start = (oy * s + dy) * w + ox * s;
                    for d in doubt.get(start..start + s).into_iter().flatten() {
                        let [v] = *d;
                        share += v;
                    }
                }
                shares.push(share / per_pixel as f32);
            }
            let mut sum = [0_u32; 3];
            for dy in 0..s {
                let start = (oy * s + dy) * w + ox * s;
                for px in rgb.get(start..start + s).into_iter().flatten() {
                    for (acc, &c) in sum.iter_mut().zip(px) {
                        *acc += u32::from(c);
                    }
                }
            }
            frame
                .rgb
                .extend(sum.map(|v| ((v + per_pixel / 2) / per_pixel).min(255) as u8));
            let first = surfaces.get(oy * s * w + ox * s);
            frame.ids.push(first.map_or(NO_FACE, |f| f.face));
            frame.depth.push(first.map_or(f64::INFINITY, |f| f.dist));
        }
    }
    if !mark {
        shares = vec![0.0; ow * oh];
    }
    Ok((frame, shares))
}

/// The nearest surface per sample, by the plain renderer's rules (the
/// same coverage, depth key and tie rule as [`crate::render_colored`] for
/// an opaque, whole picture).
fn rasterise(mesh: &Mesh, view: &View, w: usize, h: usize) -> Result<Vec<Surface>, RenderError> {
    let projected: Vec<_> = mesh.positions.iter().map(|&p| view.project(p)).collect();
    let mut out = vec![
        Surface {
            key: f64::INFINITY,
            dist: f64::INFINITY,
            normal: [0.0; 3],
            face: NO_FACE,
        };
        w * h
    ];
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
                let wa = shared_edge(&b, &c, px, py) / area;
                let wb = shared_edge(&c, &a, px, py) / area;
                let wc = shared_edge(&a, &b, px, py) / area;
                if wa < 0.0 || wb < 0.0 || wc < 0.0 {
                    continue;
                }
                let sum = wa + wb + wc;
                let (wa, wb, wc) = (wa / sum, wb / sum, wc / sum);
                let key = wa * a.key + wb * b.key + wc * c.key;
                let Some(slot) = out.get_mut(y * w + x) else {
                    continue;
                };
                // Not nearer, NaN included.
                if key.partial_cmp(&slot.key) != Some(std::cmp::Ordering::Less) {
                    continue;
                }
                let (qa, qb, qc) = (wa * a.inv_w, wb * b.inv_w, wc * c.inv_w);
                let q = qa + qb + qc;
                let (qa, qb, qc) = (qa / q, qb / q, qc / q);
                let normal = (na * qa + nb * qb + nc * qc)
                    .normalized()
                    .or(flat_normal)
                    .unwrap_or(Vec3::ZERO);
                *slot = Surface {
                    key,
                    dist: crate::distance_at(view, [wa, wb, wc], [&a, &b, &c]),
                    normal: [normal.x as f32, normal.y as f32, normal.z as f32],
                    face,
                };
            }
        }
    }
    Ok(out)
}

impl View {
    /// The ray through the position `(px, py)` (sample coordinates):
    /// where it starts (the eye, or a point of the eye plane in
    /// orthographic) and its unit direction.
    pub(crate) fn ray(&self, px: f64, py: f64) -> (Vec3, Vec3) {
        let (px, py) = (px + self.jitter.0, py + self.jitter.1);
        let cx = (px / self.w * 2.0 - 1.0) * self.aspect * self.half;
        let cy = (1.0 - py / self.h * 2.0) * self.half;
        let across = self.right * cx + self.up * cy;
        match self.projection {
            Projection::Perspective { .. } => (
                self.eye,
                (self.forward + across).normalized().unwrap_or(self.forward),
            ),
            Projection::Orthographic { .. } => (self.eye + across, self.forward),
        }
    }

    /// The surface point at view distance `dist` under `(px, py)`,
    /// relative to the eye.
    pub(crate) fn point_at(&self, px: f64, py: f64, dist: f64) -> Vec3 {
        let (px, py) = (px + self.jitter.0, py + self.jitter.1);
        let cx = (px / self.w * 2.0 - 1.0) * self.aspect * self.half;
        let cy = (1.0 - py / self.h * 2.0) * self.half;
        let across = self.right * cx + self.up * cy;
        match self.projection {
            Projection::Perspective { .. } => (self.forward + across) * dist,
            Projection::Orthographic { .. } => across + self.forward * dist,
        }
    }

    /// The side of a sample in model units at view distance `dist`.
    pub(crate) fn sample_size(&self, dist: f64) -> f64 {
        match self.projection {
            Projection::Perspective { .. } => 2.0 * self.half * dist / self.h,
            Projection::Orthographic { .. } => 2.0 * self.half / self.h,
        }
    }
}
