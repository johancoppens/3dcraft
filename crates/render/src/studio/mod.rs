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
//!    `1 + F0 (1 / (A + B) − 1)` (Fdez-Agüera, *A Multiple-Scattering
//!    Microfacet Model for Real-Time Image-Based Lighting*, 2019):
//!    - specular: the prefiltered environment along the reflection at
//!      level `r · (levels − 1)`, times `Fs`, the factor, and the
//!      specular occlusion of `ao` (Lagarde and de Rousiers, *Moving
//!      Frostbite to PBR*, 2014);
//!    - diffuse: `base (1 − m)(1 − Fs) / π` times the irradiance of the
//!      environment without its key (spherical harmonics) times `ao`,
//!      plus the key's irradiance times `n·l` times its shadow.
//! 4. Where no surface is: the environment blurred
//!    ([`env::BACKGROUND_ROUGHNESS`]), darkened where the ray meets the
//!    ground under the model by the share of the light the model keeps
//!    from it.
//! 5. Exposure (`2^EV`), the tone curve, the sRGB encoding, rounded to
//!    bytes per sample; the samples of a pixel are averaged as in the
//!    plain picture.

pub mod ao;
pub mod brdf;
pub mod bundled;
pub mod env;
pub mod hdr;
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
use craft3d_math::Vec3;
use shadow::{LightView, ShadowMap};
use std::f64::consts::PI;

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
        }
    }
}

impl Look {
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
}

impl<'a> FrameLight<'a> {
    /// The light of a frame of `mesh`.
    pub fn new(mesh: &Mesh, env: &'a Prepared, look: &Look) -> FrameLight<'a> {
        let bounds = section_bounds(mesh);
        let key = look.key_direction(env).zip(env.key).map(|(d, k)| {
            (
                d,
                k.irradiance.map(f64::from),
                f64::from(k.radius).clamp(0.0, 1.0),
            )
        });
        let shadow = key
            .filter(|_| look.shadows)
            .and_then(|(d, _, _)| LightView::new(&bounds, d))
            .map(|view| ShadowMap::draw(mesh, view));
        FrameLight {
            env,
            look: *look,
            rotation: look.rotation_radians(),
            exposure: look.exposure_scale(),
            key,
            shadow,
            ground: look
                .ground
                .then(|| bounds.corners().map(|(lo, _)| lo.z))
                .flatten(),
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
        let (key_light, key_dot) = match self.key {
            Some((l, e, radius)) => {
                let nl = n.dot(l).max(0.0);
                let vis = match &self.shadow {
                    Some(map) if nl > 0.0 => map.visibility(p, n, radius),
                    _ => 1.0,
                };
                (e, nl * vis)
            }
            None => ([0.0; 3], 0.0),
        };
        let occlusion = (nv + ao).powf((-16.0 * r - 1.0).exp2()) - 1.0 + ao;
        let specular_occlusion = occlusion.clamp(0.0, 1.0);
        let sum_ab = a + b;
        let mut out = [0.0; 3];
        for k in 0..3 {
            let c = base.get(k).copied().unwrap_or(0.0);
            let f0 = DIELECTRIC_F0 * (1.0 - m) + c * m;
            let fs = f0 * a + b;
            let multiple = if sum_ab > 0.0 {
                1.0 + f0 * (1.0 / sum_ab - 1.0)
            } else {
                1.0
            };
            let specular =
                prefiltered.get(k).copied().unwrap_or(0.0) * fs * multiple * specular_occlusion;
            let light = irradiance.get(k).copied().unwrap_or(0.0) * ao
                + key_light.get(k).copied().unwrap_or(0.0) * key_dot;
            let diffuse = c * (1.0 - m) * (1.0 - fs) / PI * light;
            if let Some(o) = out.get_mut(k) {
                *o = diffuse + specular;
            }
        }
        out
    }

    /// The light of the background along the ray from `origin` in the
    /// unit direction `d`: the environment blurred, with the ground's
    /// shadow where the ray meets it.
    pub fn background(&self, origin: Vec3, d: Vec3) -> [f64; 3] {
        let c = self.radiance(d, env::BACKGROUND_ROUGHNESS * self.top_level());
        let factor = self.ground_factor(origin, d);
        c.map(|v| v * factor)
    }

    /// The share of the ground's light the model leaves it where the ray
    /// meets it: `(E + K · l_z · vis) / (E + K · l_z)` with `E` the
    /// luminance of the irradiance without the key on an upward face and
    /// `K` the key's; 1 off the ground.
    pub fn ground_factor(&self, origin: Vec3, d: Vec3) -> f64 {
        let (Some(z), Some(map), Some((l, e, radius))) = (self.ground, &self.shadow, self.key)
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
        let rest = env::luminance(self.irradiance(Vec3::Z));
        let key = env::luminance(e) * lz;
        if !(rest + key > 0.0) || lz <= 0.0 {
            return 1.0;
        }
        let vis = map.visibility(g, Vec3::Z, radius);
        (rest + key * vis) / (rest + key)
    }

    /// Linear light to a byte triple.
    pub fn to_bytes(&self, c: [f64; 3]) -> [u8; 3] {
        let scaled = c.map(|v| v * self.exposure);
        tonemap::display(scaled, self.look.tonemap).map(tonemap::to_byte)
    }
}

/// A sample's surface: its view distance (infinity for none), normal and
/// face.
#[derive(Clone, Copy)]
struct Surface {
    key: f64,
    dist: f64,
    normal: [f32; 3],
    face: u32,
}

/// [`crate::render_colored`] in the studio look, lit by `env`. A
/// see-through or cut-open picture is drawn in the plain look (the
/// studio look of those is still to come).
pub fn render_studio(
    mesh: &Mesh,
    colors: &FaceColors,
    camera: &Camera,
    params: &RenderParams,
    look: &Look,
    env: &Prepared,
) -> Result<Frame, RenderError> {
    if params.opacity() < 1.0 || params.section.is_some() {
        return render_colored(mesh, colors, camera, params);
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
    let view = View::new(camera, w, h)?;
    let surfaces = rasterise(mesh, &view, w, h)?;
    let light = FrameLight::new(mesh, env, look);
    let bounds = section_bounds(mesh);
    let radius = bounds.diagonal() * 0.5 * ao::AO_RADIUS;

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
    let rgb: Vec<[u8; 3]> = env::par_rows(h, |y| {
        (0..w)
            .map(|x| {
                let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                let Some(surface) = surfaces.get(y * w + x) else {
                    return params.background;
                };
                let (origin, ray) = view.ray(px, py);
                if surface.face == NO_FACE || !surface.dist.is_finite() {
                    return light.to_bytes(light.background(origin, ray));
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
                let (base, finish) = if params.highlight.contains(surface.face) {
                    (params.highlight.color, Finish::default())
                } else {
                    colors
                        .look(surface.face)
                        .unwrap_or((params.color, Finish::default()))
                };
                light.to_bytes(light.shade(base, finish, p, n, to_viewer, ao))
            })
            .collect()
    });
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
    for oy in 0..oh {
        for ox in 0..ow {
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
    Ok(frame)
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
                if !(key < slot.key) {
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
