//! CPU reference renderer: the display oracle (docs/design/rendering.md).
//!
//! A software rasteriser with a depth buffer, smooth two-sided shading and an
//! id buffer that says which face is under each pixel. It favours exactness
//! and readability over speed, and is deterministic: single-threaded `f64`
//! arithmetic, triangles drawn in mesh order.

use craft3d_math::Portable;
pub mod analysis;
mod clip;
pub use clip::{depth_bounds, depth_range};
mod section;
pub mod studio;

pub use section::{Cut, SECTION_FACE, SECTION_MARGIN, SECTION_SNAP, Section, section_bounds};

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
/// How precisely a depth buffer places a surface across the picture, in
/// pixels: a GPU snaps the corners of a triangle to a grid of
/// `2^-subPixelPrecisionBits` of a pixel before it rasterises, and 4 bits
/// is the least Vulkan allows (Direct3D and Metal have 8). A test of a
/// point against a depth buffer allows the surface's change over this
/// distance (the viewport's overlay, on the CPU and on the GPU alike).
pub const SUBPIXEL_GRAIN: f64 = 1.0 / 16.0;

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
    /// A studio still of `frames` frames of `samples` samples each would
    /// pass [`MAX_SAMPLES`] in all.
    TooManyFrames {
        samples: u64,
        frames: u32,
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
    /// An environment of the studio look could not be prepared or used.
    Environment(String),
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
            RenderError::TooManyFrames { samples, frames } => write!(
                f,
                "{frames} frames of {samples} samples exceed the limit of {MAX_SAMPLES} samples \
                 in all; accumulate fewer frames or lower the size or ssaa"
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
            RenderError::Environment(e) => write!(f, "studio environment: {e}"),
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
        let half_y = (fov_y_deg.to_radians() * 0.5).ptan();
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
            (half_y * 1.05).patan().to_degrees() * 2.0
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
                let half = (fov_y_deg.to_radians() * 0.5).ptan() / factor;
                let fov_y_deg = half.patan().to_degrees() * 2.0;
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

/// The studio lighting both renderers shade with, in one place: an
/// ambient term, a key light above and to the left of the viewer and a
/// weaker fill light from the right, both fixed to the camera, so that a
/// face turned from the key light still reads (2026-10-24, the developer's
/// choice; it was one light, ambient 0.2 and diffuse 0.8, and a face turned
/// from it was near black). The intensity of a face is
/// `SHADE_AMBIENT + SHADE_DIFFUSE · n·key + SHADE_FILL · n·fill`, each dot
/// product taken as 0 where it is negative; the white highlight comes from
/// the key light alone. In the iso view a grey block's top is at 0.88, its
/// front at 0.73 and its right side at 0.57.
///
/// The ambient term is a multiple of 1/25 on purpose: a face turned from
/// both lights is shaded with it alone, and a colour byte times `k / 25` is
/// never half-way between two levels, where the GPU rounds to even and the
/// CPU away from zero (see [`SHADE_HIGHLIGHT_AMBIENT`]).
pub const SHADE_AMBIENT: f64 = 0.36;
/// Weight of the key light.
pub const SHADE_DIFFUSE: f64 = 0.64;
/// Weight of the fill light.
pub const SHADE_FILL: f64 = 0.18;
/// Where the key light comes from, in the camera's axes: right, up and
/// towards the viewer (not normalised): above and to the left.
pub const KEY_LIGHT: [f64; 3] = [-0.45, 0.6, 1.0];
/// Where the fill light comes from, in the camera's axes as [`KEY_LIGHT`]:
/// from the right, a little from below, towards the viewer.
pub const FILL_LIGHT: [f64; 3] = [1.0, -0.2, 0.4];
/// Weight of the white specular highlight, as a fraction of 255.
pub const SHADE_SPECULAR: f64 = 0.35;
/// Exponent of the specular highlight.
pub const SHADE_SHININESS: i32 = 48;
/// How much less diffuse light an all-metal face reflects ([`Finish`]):
/// its diffuse weight is `1 − SHADE_METAL_DIFFUSE · metallic` times the
/// plain one, for the key and the fill light alike. `scene.wgsl` has the
/// same number.
pub const SHADE_METAL_DIFFUSE: f64 = 0.3;
/// Ambient term and diffuse weight of a highlighted face ([`Highlight`]):
/// more ambient light than a plain face gets, so that a highlighted face
/// in the shade still shows its colour.
///
/// Three fifths on purpose, as the plain face's ambient term is 9/25:
/// everything turned away from the lights is shaded with the ambient term
/// alone, and a colour byte times a fifth (or `k / 25`) is never half-way
/// between two levels. With
/// 0.55 a byte of 150 is 82.5 there, which the two renderers round
/// differently (the GPU to even), a level apart over the whole shaded side.
pub const SHADE_HIGHLIGHT_AMBIENT: f64 = 0.6;
pub const SHADE_HIGHLIGHT_DIFFUSE: f64 = 0.4;
/// The fill light's weight on a highlighted face: none, its ambient term
/// already keeps it readable in the shade.
pub const SHADE_HIGHLIGHT_FILL: f64 = 0.0;

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
    /// Unit vector towards the key light, which is fixed to the camera:
    /// above and to the left of the viewer ([`KEY_LIGHT`]).
    pub fn light(&self) -> Option<Vec3> {
        self.toward(KEY_LIGHT)
    }

    /// Unit vector towards the fill light, fixed to the camera like the
    /// key light: from the right ([`FILL_LIGHT`]).
    pub fn fill_light(&self) -> Option<Vec3> {
        self.toward(FILL_LIGHT)
    }

    /// A direction given in the camera's axes (right, up, towards the
    /// viewer), in the model's, as a unit vector.
    fn toward(&self, [right, up, back]: [f64; 3]) -> Option<Vec3> {
        (self.right * right + self.up * up - self.forward * back).normalized()
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
                (fov_y_deg.to_radians() * 0.5).ptan()
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

/// Most runs of consecutive face ids a [`Highlight`] holds: what fits the
/// GPU renderer's uniform block.
pub const MAX_HIGHLIGHT_RANGES: usize = 8;

/// The faces drawn in another base colour, so that one face stands out:
/// they are shaded like the others, from `color` instead of
/// [`RenderParams::color`] and with more ambient light
/// ([`SHADE_HIGHLIGHT_AMBIENT`], [`SHADE_HIGHLIGHT_DIFFUSE`]). Both renderers take it from here, so a
/// highlighted face is the same face in both.
///
/// The faces are held as at most [`MAX_HIGHLIGHT_RANGES`] runs of
/// consecutive ids, in ascending order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Highlight {
    pub color: [u8; 3],
    /// First and last id of each run; a run whose first id is above its
    /// last is empty.
    ranges: [[u32; 2]; MAX_HIGHLIGHT_RANGES],
}

impl Default for Highlight {
    fn default() -> Self {
        Self::NONE
    }
}

impl Highlight {
    /// No face.
    pub const NONE: Highlight = Highlight {
        color: [0; 3],
        ranges: [[1, 0]; MAX_HIGHLIGHT_RANGES],
    };

    /// The faces `faces` in `color`, and how many of them were left out
    /// because they make more than [`MAX_HIGHLIGHT_RANGES`] runs: the ones
    /// with the highest ids. [`NO_FACE`] is never a face.
    pub fn new(color: [u8; 3], faces: impl IntoIterator<Item = u32>) -> (Self, usize) {
        let faces: BTreeSet<u32> = faces.into_iter().filter(|&f| f != NO_FACE).collect();
        let mut out = Self {
            color,
            ..Self::NONE
        };
        let mut used = 0_usize;
        let mut left_out = 0_usize;
        for face in faces {
            let last = used.checked_sub(1).and_then(|k| out.ranges.get_mut(k));
            match last {
                Some(run) if run[1].checked_add(1) == Some(face) => run[1] = face,
                _ => match out.ranges.get_mut(used) {
                    Some(run) => {
                        *run = [face, face];
                        used += 1;
                    }
                    None => left_out += 1,
                },
            }
        }
        (out, left_out)
    }

    pub fn contains(&self, face: u32) -> bool {
        self.ranges
            .iter()
            .any(|&[first, last]| first <= face && face <= last)
    }

    pub fn is_empty(&self) -> bool {
        self.ranges().next().is_none()
    }

    /// The runs that hold a face, as `[first, last]`, in ascending order.
    pub fn ranges(&self) -> impl Iterator<Item = [u32; 2]> + '_ {
        self.ranges
            .iter()
            .copied()
            .filter(|&[first, last]| first <= last)
    }

    /// How many faces it holds.
    pub fn len(&self) -> u64 {
        self.ranges()
            .map(|[first, last]| u64::from(last - first) + 1)
            .sum()
    }
}

/// Most face ids a [`FaceColors`] holds a colour for: what a face id
/// compared as `f32` on the GPU keeps exactly.
pub const MAX_FACE_COLORS: usize = 1 << 24;

/// Most shininess a [`Finish`] gives: a roughness of 0.
pub const MAX_SHININESS: u32 = 256;

/// How metal and how rough a face is drawn: the two numbers of a
/// metallic-roughness material, as bytes (0 to 255 for 0 to 1), so that
/// both renderers shade from exactly the same numbers.
///
/// The default (not metal, no roughness of its own) is the plain shading,
/// bit for bit. See [`render`], *Metallic and roughness*.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Finish {
    /// 0 is not metal (the default), 255 all metal.
    pub metallic: u8,
    /// `None`: the plain highlight ([`SHADE_SHININESS`],
    /// [`SHADE_SPECULAR`]); else 0 is a mirror, 255 all rough.
    pub roughness: Option<u8>,
}

impl Finish {
    /// The finish of two factors from 0 to 1 (`None` for not given); a
    /// factor outside 0 to 1 is clamped, one that is not a number counts
    /// as not given.
    pub fn from_factors(metallic: Option<f64>, roughness: Option<f64>) -> Self {
        // Clamped to 0..=255 before the cast.
        let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        Self {
            metallic: metallic.filter(|v| v.is_finite()).map_or(0, byte),
            roughness: roughness.filter(|v| v.is_finite()).map(byte),
        }
    }

    /// True for the plain shading.
    pub fn is_plain(&self) -> bool {
        *self == Self::default()
    }

    /// The exponent of the highlight a roughness gives, or `None` for the
    /// plain one: `2 / r⁴ − 2` (the Phong exponent of a Beckmann
    /// distribution of width `α = r²`), rounded, within 1 to
    /// [`MAX_SHININESS`]. A roughness of about 0.45 gives the plain
    /// exponent, 48.
    pub fn shininess(&self) -> Option<u32> {
        let r = f64::from(self.roughness?) / 255.0;
        let alpha2 = (r * r) * (r * r);
        let floor = 2.0 / f64::from(MAX_SHININESS + 2);
        let n = (2.0 / alpha2.max(floor) - 2.0).round();
        // Within 1..=MAX_SHININESS before the cast.
        Some(n.clamp(1.0, f64::from(MAX_SHININESS)) as u32)
    }
}

/// The base colour of each face that has one of its own (a body's
/// appearance), by face id, with its [`Finish`]; the others are shaded
/// from [`RenderParams::color`]. Both renderers take it from here, so a
/// face has the same colour in both. A highlighted face ([`Highlight`]) is
/// drawn in the highlight's colour, plainly, whatever its own.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FaceColors {
    colors: Vec<Option<([u8; 3], Finish)>>,
    /// Layer 3: the distinct [`studio::layers::Optics`] of the faces, and
    /// per face id its row there plus one (0 for none).
    optics: Vec<studio::layers::Optics>,
    slots: Vec<u16>,
}

impl FaceColors {
    /// No face has a colour of its own.
    pub fn new() -> Self {
        Self::default()
    }

    /// Gives `face` its own colour with the plain finish, or takes it away
    /// (`None`). False, and nothing changes, for a face id at or beyond
    /// [`MAX_FACE_COLORS`].
    pub fn set(&mut self, face: u32, color: Option<[u8; 3]>) -> bool {
        self.set_look(face, color.map(|c| (c, Finish::default())))
    }

    /// [`Self::set`] with a finish.
    pub fn set_look(&mut self, face: u32, look: Option<([u8; 3], Finish)>) -> bool {
        let index = face as usize;
        if index >= MAX_FACE_COLORS {
            return false;
        }
        if index >= self.colors.len() {
            if look.is_none() {
                return true;
            }
            self.colors.resize(index + 1, None);
        }
        if let Some(slot) = self.colors.get_mut(index) {
            *slot = look;
        }
        true
    }

    /// The face's own colour, if it has one.
    pub fn get(&self, face: u32) -> Option<[u8; 3]> {
        self.look(face).map(|(color, _)| color)
    }

    /// The face's own colour and finish, if it has a colour.
    pub fn look(&self, face: u32) -> Option<([u8; 3], Finish)> {
        self.colors.get(face as usize).copied().flatten()
    }

    /// The face's finish: the plain one for a face without a colour.
    pub fn finish(&self, face: u32) -> Finish {
        self.look(face)
            .map(|(_, finish)| finish)
            .unwrap_or_default()
    }

    /// True when no face has a colour of its own.
    pub fn is_empty(&self) -> bool {
        self.colors.iter().all(Option::is_none)
    }

    /// The faces with a colour of their own and its finish, by ascending
    /// id.
    pub fn looks(&self) -> impl Iterator<Item = (u32, ([u8; 3], Finish))> + '_ {
        self.colors
            .iter()
            .enumerate()
            .filter_map(|(face, look)| Some((u32::try_from(face).ok()?, (*look)?)))
    }

    /// Gives `face` the layers of a material beyond its colour and finish
    /// (layer 3, the studio look; sanitized), or takes them away (`None`,
    /// or layers that change nothing). False, and nothing changes, for a
    /// face id at or beyond [`MAX_FACE_COLORS`] or when the picture would
    /// hold more than [`studio::layers::MAX_OPTICS`] distinct ones.
    pub fn set_optics(&mut self, face: u32, optics: Option<studio::layers::Optics>) -> bool {
        let index = face as usize;
        if index >= MAX_FACE_COLORS {
            return false;
        }
        let optics = optics
            .map(studio::layers::Optics::sanitized)
            .filter(|o| !o.is_plain());
        let slot = match optics {
            None => 0,
            Some(optics) => match self.optics.iter().position(|o| *o == optics) {
                Some(at) => at + 1,
                None => {
                    if self.optics.len() >= studio::layers::MAX_OPTICS {
                        return false;
                    }
                    self.optics.push(optics);
                    self.optics.len()
                }
            },
        };
        let Ok(slot) = u16::try_from(slot) else {
            return false;
        };
        if index >= self.slots.len() {
            if slot == 0 {
                return true;
            }
            self.slots.resize(index + 1, 0);
        }
        if let Some(s) = self.slots.get_mut(index) {
            *s = slot;
        }
        true
    }

    /// The face's layers beyond its colour and finish, if it has any.
    pub fn optics(&self, face: u32) -> Option<&studio::layers::Optics> {
        let slot = self.optics_slot(face);
        self.optics.get(usize::from(slot).checked_sub(1)?)
    }

    /// The face's row in [`Self::optics_table`] plus one, 0 for none.
    pub fn optics_slot(&self, face: u32) -> u16 {
        self.slots.get(face as usize).copied().unwrap_or(0)
    }

    /// The distinct layers of the faces, by row.
    pub fn optics_table(&self) -> &[studio::layers::Optics] {
        &self.optics
    }

    /// True when a face that has layers lets light through.
    pub fn has_transmission(&self) -> bool {
        self.optics
            .iter()
            .any(studio::layers::Optics::is_transmissive)
    }

    /// True when `face` lets light through.
    pub fn is_transmissive(&self, face: u32) -> bool {
        self.optics(face)
            .is_some_and(studio::layers::Optics::is_transmissive)
    }

    /// The faces with layers of their own, by ascending id.
    pub fn optics_faces(&self) -> impl Iterator<Item = (u32, &studio::layers::Optics)> + '_ {
        self.slots.iter().enumerate().filter_map(|(face, &slot)| {
            let row = usize::from(slot).checked_sub(1)?;
            Some((u32::try_from(face).ok()?, self.optics.get(row)?))
        })
    }

    /// The faces with a colour of their own, by ascending id.
    pub fn iter(&self) -> impl Iterator<Item = (u32, [u8; 3])> + '_ {
        self.colors
            .iter()
            .enumerate()
            .filter_map(|(face, look)| Some((u32::try_from(face).ok()?, (*look)?.0)))
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
    /// The faces shaded from another base colour.
    pub highlight: Highlight,
    /// How opaque the bodies are, 0 to 1 (see [`render`], *See-through*).
    /// 1, the default, is the opaque picture, bit for bit. A number that
    /// is not one counts as 1; one outside 0 to 1 is clamped.
    pub opacity: f64,
    /// The plane the picture is cut open with (see [`render`], *Section*).
    /// `None`, the default, is the whole picture, bit for bit; so is a
    /// plane that is none ([`Cut::new`]).
    pub section: Option<Section>,
    /// A surface analysis drawn in place of the shading
    /// ([`analysis::Analysis`]); `None`, the default, is the plain
    /// picture, bit for bit. The curvature map's values per vertex come
    /// with the mesh ([`render_analysed`]).
    pub analysis: Option<analysis::Analysis>,
}

impl Default for RenderParams {
    fn default() -> Self {
        Self {
            width: 800,
            height: 600,
            ssaa: 2,
            background: [30, 33, 40],
            color: [198, 204, 214],
            highlight: Highlight::NONE,
            opacity: 1.0,
            section: None,
            analysis: None,
        }
    }
}

impl RenderParams {
    /// [`Self::opacity`] as the renderer uses it: within 0 to 1.
    pub fn opacity(&self) -> f64 {
        if self.opacity.is_finite() {
            self.opacity.clamp(0.0, 1.0)
        } else {
            1.0
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
    /// Distance along the view direction of the surface point under the
    /// pixel's first sample (interpolated perspective-correct, like the
    /// position); infinity where nothing was drawn.
    pub depth: Vec<f64>,
    /// Where the first sample lies in its pixel, along each axis, in
    /// pixels from the pixel's top-left corner: `0.5 / ssaa`, the centre
    /// of the first of the pixel's `ssaa` × `ssaa` samples. `ids` and
    /// `depth` are taken there, so a depth test that reconstructs the
    /// surface between pixels places them there.
    pub sample: f64,
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

/// The distance along the view direction of the point of a triangle under
/// a sample, from the screen-space weights `w` of its corners `p` (which
/// sum to one): interpolated perspective-correct, like the position, so
/// that it is the distance of that very point. In orthographic the weights
/// are the point's own; in perspective the screen weights are divided by
/// the corners' distances first (`Projected::inv_w`), as for any attribute.
fn distance_at(view: &View, w: [f64; 3], p: [&Projected; 3]) -> f64 {
    let [wa, wb, wc] = w;
    let [a, b, c] = p;
    match view.projection {
        Projection::Orthographic { .. } => wa * a.dist + wb * b.dist + wc * c.dist,
        Projection::Perspective { .. } => {
            let (qa, qb, qc) = (wa * a.inv_w, wb * b.inv_w, wc * c.inv_w);
            (qa * a.dist + qb * b.dist + qc * c.dist) / (qa + qb + qc)
        }
    }
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
    /// Unit vector towards the key light ([`ViewAxes::light`]).
    light: Option<Vec3>,
    /// Unit vector towards the fill light ([`ViewAxes::fill_light`]).
    fill: Option<Vec3>,
    aspect: f64,
    w: f64,
    h: f64,
    /// Where a sample's centre lies off the middle of its cell, in
    /// samples (x right, y down): 0 except in the studio look's
    /// accumulated frames (`studio::polish::jitter`).
    jitter: (f64, f64),
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
            fill: axes.fill_light(),
            aspect: w as f64 / h as f64,
            w: w as f64,
            h: h as f64,
            jitter: (0.0, 0.0),
        })
    }

    /// `None` for a point at or behind the eye plane (perspective), a
    /// non-finite point, or one so close to the eye plane that it lands
    /// outside what a float holds.
    fn project(&self, p: Vec3) -> Option<Projected> {
        let rel = p - self.eye;
        let (cx, cy, dist) = (rel.dot(self.right), rel.dot(self.up), rel.dot(self.forward));
        let (scale, key, inv_w) = match self.projection {
            Projection::Perspective { .. } => {
                // Any distance in front of the eye: what is drawn is clipped
                // to the near plane first (`clip`), which may be very close
                // at a small scale (a millionth of the farthest distance).
                // The results are checked to be finite below.
                if dist.is_nan() || dist <= 0.0 {
                    return None;
                }
                (1.0 / (dist * self.half), -1.0 / dist, 1.0 / dist)
            }
            Projection::Orthographic { .. } => (1.0 / self.half, dist, 1.0),
        };
        let sx = (cx * scale / self.aspect * 0.5 + 0.5) * self.w - self.jitter.0;
        let sy = (0.5 - cy * scale * 0.5) * self.h - self.jitter.1;
        (sx.is_finite() && sy.is_finite() && key.is_finite()).then_some(Projected {
            sx,
            sy,
            dist,
            key,
            inv_w,
        })
    }

    /// Where the ray through the position (`px`, `py`) of the picture
    /// (sample coordinates, x right and y down) meets the plane of `cut`,
    /// when the cap can be there: the depth key, the distance along the
    /// view direction and the point. `None` when the ray runs along the
    /// plane, meets it at or behind the eye plane of a perspective camera,
    /// or outside the box the cap stays in, and in front of `near`, the
    /// near plane the triangles are clipped at (`clip`), as the GPU clips
    /// its cap.
    fn cap_at(&self, cut: &Cut, px: f64, py: f64, near: Option<f64>) -> Option<(f64, f64, Vec3)> {
        // The inverse of `project`: the offsets across the view at
        // distance 1 (perspective) or at any distance (orthographic).
        let cx = (px / self.w * 2.0 - 1.0) * self.aspect * self.half;
        let cy = (1.0 - py / self.h * 2.0) * self.half;
        let across = self.right * cx + self.up * cy;
        let (start, direction) = match self.projection {
            Projection::Perspective { .. } => (self.eye, self.forward + across),
            Projection::Orthographic { .. } => (self.eye + across, self.forward),
        };
        // `direction·forward` is 1, so the parameter is the distance.
        let dist = (cut.origin - start).dot(cut.normal) / direction.dot(cut.normal);
        let key = match self.projection {
            Projection::Perspective { .. } => {
                // As `project`: any distance in front of the eye, and not
                // in front of the near plane.
                if dist.is_nan() || dist <= 0.0 || near.is_some_and(|near| dist < near) {
                    return None;
                }
                -1.0 / dist
            }
            Projection::Orthographic { .. } => dist,
        };
        let point = start + direction * dist;
        (key.is_finite() && dist.is_finite() && cut.holds_cap(point)).then_some((key, dist, point))
    }

    /// Unit vector from a surface point towards the viewer.
    fn toward_viewer(&self, p: Vec3) -> Vec3 {
        match self.projection {
            Projection::Perspective { .. } => (self.eye - p).normalized().unwrap_or(-self.forward),
            Projection::Orthographic { .. } => -self.forward,
        }
    }
}

/// The exact derivatives along the samples' x and y of a triangle's
/// perspective-correct normal (before it is made unit) and point at one
/// sample: `corners` the triangle's points and normals, `weights` the
/// sample's perspective-correct weights (summing to 1), `inv_w` the
/// corners' `1 / w` (`ιᵢ`), `q = Σ bᵢ ιᵢ` over the screen weights `bᵢ`,
/// `d_weights` the screen weights' derivatives (constant on the triangle).
/// An attribute `A = Σ bᵢ ιᵢ Aᵢ / q` has `∂A = Σ ∂bᵢ ιᵢ (Aᵢ − A) / q`: the
/// derivative the GPU's `dpdxFine` and `dpdyFine` approximate by a
/// difference over one sample.
fn stripe_slopes(
    corners: [(Vec3, Vec3); 3],
    weights: [f64; 3],
    q: f64,
    inv_w: [f64; 3],
    d_weights: &[[f64; 2]; 3],
) -> analysis::Slopes {
    let mut normal = Vec3::ZERO;
    let mut point = Vec3::ZERO;
    for (&(p, n), &k) in corners.iter().zip(weights.iter()) {
        normal += n * k;
        point += p * k;
    }
    let mut out = analysis::Slopes {
        normal,
        ..analysis::Slopes::default()
    };
    if !(q != 0.0 && q.is_finite()) {
        return out;
    }
    for axis in 0..2 {
        let (mut dn, mut dp) = (Vec3::ZERO, Vec3::ZERO);
        for ((&(p, n), &iw), dw) in corners.iter().zip(inv_w.iter()).zip(d_weights.iter()) {
            let g = dw.get(axis).copied().unwrap_or(0.0) * iw / q;
            dn += (n - normal) * g;
            dp += (p - point) * g;
        }
        if let (Some(n_slot), Some(p_slot)) = (out.d_normal.get_mut(axis), out.d_rel.get_mut(axis))
        {
            *n_slot = dn;
            *p_slot = dp;
        }
    }
    out
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

/// Whether a sample that lies exactly on the edge `p → q` of a triangle
/// whose third corner is `r` belongs to that triangle: the top-left rule
/// of rasterisers. It does when the triangle lies to the right of the edge
/// in the picture, or, for a horizontal edge, below it. The two triangles
/// that share an edge lie on opposite sides of it, so exactly one of them
/// owns a sample on it.
fn owns_edge(p: &Projected, q: &Projected, r: &Projected) -> bool {
    // The ends in one order for both triangles, as in `shared_edge`.
    let (a, b) = if (p.sx, p.sy) <= (q.sx, q.sy) {
        (p, q)
    } else {
        (q, p)
    };
    let inside = edge(a.sx, a.sy, b.sx, b.sy, r.sx, r.sy) > 0.0;
    let (dx, dy) = (b.sx - a.sx, b.sy - a.sy);
    // The edge function one step to the right of the edge is `-dy`, one
    // step below it `dx`.
    if dy != 0.0 {
        inside == (dy < 0.0)
    } else {
        inside == (dx > 0.0)
    }
}

/// Whether a sample with the barycentric weights `weights` (none negative)
/// of the triangle `corners` belongs to it: inside it does, on an edge by
/// [`owns_edge`].
fn owns_sample(weights: [f64; 3], corners: [&Projected; 3]) -> bool {
    let [wa, wb, wc] = weights;
    let [a, b, c] = corners;
    (wa != 0.0 || owns_edge(b, c, a))
        && (wb != 0.0 || owns_edge(c, a, b))
        && (wc != 0.0 || owns_edge(a, b, c))
}

/// Which edge or vertex of the mesh a sample lies exactly on, given its
/// barycentric weights in a triangle with the corners `corners`: a number
/// made of the coordinates of the edge's two ends (in either order) or of
/// the vertex, the same for every triangle that shares it; 0 for a sample
/// inside the triangle.
///
/// The triangles that share an edge are one surface there. A sample on the
/// edge is inside all of them (see [`shared_edge`]), with depth keys that
/// agree only up to rounding, so without this the surface could count as
/// lying behind itself.
fn shared_place(weights: [f64; 3], corners: [Vec3; 3]) -> u64 {
    let [wa, wb, wc] = weights;
    let [pa, pb, pc] = corners;
    let on: &[Vec3] = match (wa == 0.0, wb == 0.0, wc == 0.0) {
        (false, false, false) | (true, true, true) => return 0,
        // On the edge opposite the corner whose weight is zero.
        (true, false, false) => &[pb, pc],
        (false, true, false) => &[pc, pa],
        (false, false, true) => &[pa, pb],
        // At the corner whose weight is not zero.
        (false, true, true) => &[pa],
        (true, false, true) => &[pb],
        (true, true, false) => &[pc],
    };
    let bits = |p: Vec3| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()];
    let mut ends: Vec<[u64; 3]> = on.iter().map(|&p| bits(p)).collect();
    ends.sort_unstable();
    // FNV-1a over the coordinates.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for word in ends.iter().flatten() {
        for byte in word.to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
    hash.max(1)
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

/// `weights` is the ambient term, the key light's weight and the fill
/// light's ([`SHADE_AMBIENT`], [`SHADE_DIFFUSE`], [`SHADE_FILL`]). With the plain
/// [`Finish`] every factor it adds is exactly 1 (or 255 for the tint), so
/// the plain picture is bit for bit what it was before finishes.
fn shade(
    base: [u8; 3],
    weights: (f64, f64, f64),
    finish: Finish,
    normal: Vec3,
    to_viewer: Vec3,
    view: &View,
) -> [u8; 3] {
    // Two-sided: sheets are seen from both sides (the inside of the spout).
    let n = if normal.dot(to_viewer) < 0.0 {
        -normal
    } else {
        normal
    };
    // A key light fixed to the camera, above and to the left of the viewer,
    // and a fill light from the right.
    let light = view.light.unwrap_or(to_viewer);
    let diffuse = n.dot(light).max(0.0);
    let fill = view.fill.map_or(0.0, |f| n.dot(f).max(0.0));
    let metallic = f64::from(finish.metallic) / 255.0;
    // The exponent of the highlight and its strength, relative to the plain one.
    let (shininess, strength) = match finish.shininess().and_then(|e| i32::try_from(e).ok()) {
        Some(exponent) => (
            exponent,
            ((f64::from(exponent) + 2.0) / f64::from(SHADE_SHININESS + 2)).sqrt(),
        ),
        None => (SHADE_SHININESS, 1.0),
    };
    let specular = (light + to_viewer)
        .normalized()
        .map_or(0.0, |half| n.dot(half).max(0.0).ppowi(shininess));
    // A metal reflects less diffusely.
    let intensity = weights.0
        + (weights.1 * diffuse + weights.2 * fill) * (1.0 - SHADE_METAL_DIFFUSE * metallic);
    // Clamped to 0..=255 before the cast.
    base.map(|c| {
        // White on a dielectric, the base colour on a metal.
        let tint = 255.0 * (1.0 - metallic) + f64::from(c) * metallic;
        (f64::from(c) * intensity + tint * SHADE_SPECULAR * strength * specular)
            .round()
            .clamp(0.0, 255.0) as u8
    })
}

/// Renders `mesh` as seen from `camera`.
///
/// In perspective, triangles are clipped to the near plane the GPU
/// renderer uses (the mesh's nearest corner of its box less a margin, at
/// least a millionth of the farthest; `clip`), so a camera close to a
/// surface or inside the model draws what lies in front of it.
///
/// **See-through** ([`RenderParams::opacity`] below 1). Per sample, two
/// surfaces count: the nearest one (the *front*, what the opaque picture
/// shows) and the nearest one that is strictly further away than it (the
/// one *behind*; the background when there is none). The sample's colour
/// is `opacity` times the front's shaded colour plus `1 - opacity` times
/// the shaded colour of the one behind, rounded. Both are the minimum of
/// something over all triangles, so the picture does not depend on the
/// order of the triangles (ties go to the first triangle, as in the opaque
/// picture). The triangles that share an edge or a vertex are one surface
/// at a sample that lies exactly on it: the one that is in front there is
/// the front as ever, and the others are not behind it, although their
/// depth keys differ from its by rounding (a surface is never behind
/// itself). What is behind is drawn opaque: one layer shows through, not
/// every layer, which keeps a cavity readable and the definition exact. A
/// highlighted face in front is not seen through, so that it stands out;
/// behind, it shows in its own colour. The id and depth buffers are the
/// front's: a pick gives the front-most face whatever the opacity.
///
/// **Metallic and roughness** ([`Finish`], per face in [`FaceColors`]). A
/// Blinn-Phong approximation of a metallic-roughness material, with the
/// key and fill lights fixed to the camera ([`SHADE_AMBIENT`]): with `m`
/// the metallic factor, the intensity is `ambient + (key · n·l + fill ·
/// n·f) · (1 − 0.3 m)` (a metal reflects less diffusely), and the
/// highlight, from the key light, `tint · SHADE_SPECULAR · k ·
/// (n·h)^e`, where the tint is white on a dielectric and the base colour
/// on a metal (`255 (1 − m) + base · m`), the exponent `e` comes from the
/// roughness ([`Finish::shininess`]) and `k = √((e + 2) / 50)` makes a
/// smooth face's narrow highlight brighter and a rough face's wide one
/// dimmer (1 at the plain exponent, 48). A face without a finish adds no
/// factor to the plain shading, bit for bit.
///
/// **Section** ([`RenderParams::section`]). A plane cuts the picture open:
/// what lies in the half its normal points to is not drawn. Per sample:
///
/// - *The cut.* Every vertex has a distance from the plane
///   ([`Cut::side`]: positive in the half that is cut away); across a
///   triangle it is interpolated like every other attribute. A sample of
///   a triangle is left out where that is positive. A triangle that lies
///   in the plane (parallel to it, within [`SECTION_SNAP`] of the mesh's
///   size) goes with its body: it is left out when it faces away from the
///   cut half ([`Cut::clips`]).
/// - *The cap.* A body cut open is closed again with a flat face in the
///   plane, in [`Section::color`], unshaded, with [`SECTION_FACE`] as its
///   face id. The plane is inside a body at a sample when the number of
///   surface samples that were left out there is odd: along the ray of the
///   sample these are the surfaces between the plane and the far end of
///   the cut half, and a ray leaves a closed body as often as it enters
///   it. A sample on an edge two triangles share counts for one of them
///   (the top-left rule). The cap is then one more surface at the sample,
///   at the depth where the ray meets the plane: the front where nothing
///   that is kept is nearer (on a tie the triangle wins), the one behind
///   in a see-through picture where it is the nearest surface behind the
///   front. It lies within the box around the mesh ([`SECTION_MARGIN`]).
/// - *What is not closed.* The count is of all triangles together. For a
///   mesh of open sheets it means nothing (ask for no cap:
///   [`Section::cap`]), and where two bodies overlap in space the plane
///   through both shows no cap.
///
/// The id and depth buffers are of what is drawn: a pick on a cut-open
/// model gives what is seen.
pub fn render(mesh: &Mesh, camera: &Camera, params: &RenderParams) -> Result<Frame, RenderError> {
    render_colored(mesh, &FaceColors::new(), camera, params)
}

/// [`render`] with faces in colours of their own: a face with a colour in
/// `colors` is shaded from it instead of [`RenderParams::color`], in the
/// same way (the same light, ambient term and highlight). With no colours
/// it is [`render`], bit for bit.
pub fn render_colored(
    mesh: &Mesh,
    colors: &FaceColors,
    camera: &Camera,
    params: &RenderParams,
) -> Result<Frame, RenderError> {
    render_analysed(mesh, colors, &[], camera, params)
}

/// [`render_colored`] with a value per vertex of `mesh` for the curvature
/// map of [`RenderParams::analysis`] (interpolated across a triangle like
/// the normal; a vertex without one, or a value that is not a number,
/// counts as none). Without an analysis the values are not read.
pub fn render_analysed(
    mesh: &Mesh,
    colors: &FaceColors,
    values: &[f32],
    camera: &Camera,
    params: &RenderParams,
) -> Result<Frame, RenderError> {
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
    // The analysis and the section are placed by the mesh as given; what
    // is drawn is the mesh clipped to the near plane.
    let analysis = match params.analysis {
        Some(a) => Some(analysis::Resolved::new(&a, &camera.axes()?, &mesh.bbox())),
        None => None,
    };
    let cut = params
        .section
        .and_then(|section| Cut::new(&section, &section_bounds(mesh)));
    let bounds = clip::depth_bounds(&section_bounds(mesh), cut.is_some_and(|c| c.cap));
    let near = clip::near_distance(&view, &bounds);
    let clipped = clip::clip_mesh(mesh, values, &view, &bounds)?;
    let (mesh, values) = (&*clipped.mesh, &*clipped.values);

    let projected: Vec<Option<Projected>> =
        mesh.positions.iter().map(|&p| view.project(p)).collect();
    let value_at = |k: u32| values.get(k as usize).map_or(f64::NAN, |&v| f64::from(v));
    let mut color = vec![params.background; w * h];
    let mut ids = vec![NO_FACE; w * h];
    let mut depth_key = vec![f64::INFINITY; w * h];
    let mut depth = vec![f64::INFINITY; w * h];
    let opacity = params.opacity();
    let see_through = opacity < 1.0;
    // Per sample, the depth key and the colour of the surface behind the
    // front one: only for a see-through picture.
    let mut behind: Vec<(f64, [u8; 3])> = if see_through {
        vec![(f64::INFINITY, params.background); w * h]
    } else {
        Vec::new()
    };
    // Per sample of a see-through picture, the edge or vertex of the mesh
    // the front lies exactly on ([`shared_place`]); 0 for none.
    let mut front_on: Vec<u64> = if see_through {
        vec![0; w * h]
    } else {
        Vec::new()
    };
    // The section: every vertex's distance from the plane, and per sample
    // whether the number of surfaces left out there is odd.
    let sides: Vec<f64> = cut.map_or_else(Vec::new, |cut| {
        mesh.positions.iter().map(|&p| cut.side(p)).collect()
    });
    let mut odd: Vec<bool> = if cut.is_some_and(|cut| cut.cap) {
        vec![false; w * h]
    } else {
        Vec::new()
    };

    for (t, (tri, &face)) in mesh.triangles.iter().zip(&mesh.face_ids).enumerate() {
        let vertex = |k: u32| {
            let k = k as usize;
            match (mesh.positions.get(k), mesh.normals.get(k), projected.get(k)) {
                (Some(&p), Some(&n), Some(&proj)) => Ok((p, n, proj)),
                _ => Err(RenderError::InvalidMesh { triangle: t }),
            }
        };
        let side = |k: u32| sides.get(k as usize).copied().unwrap_or(0.0);
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
        // The screen-space barycentric weights' derivatives along x and y
        // (constant over the triangle): for the stripes' footprint.
        let d_weights = [
            [-(c.sy - b.sy) / area, (c.sx - b.sx) / area],
            [-(a.sy - c.sy) / area, (a.sx - c.sx) / area],
            [-(b.sy - a.sy) / area, (b.sx - a.sx) / area],
        ];
        let inv_w = [a.inv_w, b.inv_w, c.inv_w];
        // The section: the corners' distances from the plane, and which
        // way the triangle faces.
        let cut_triangle = cut.map(|cut| {
            let facing = flat_normal.map_or(0.0, |n| n.dot(cut.normal));
            ([side(ia), side(ib), side(ic)], facing)
        });
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
                let on_edge = [wa, wb, wc];
                let sum = wa + wb + wc;
                let (wa, wb, wc) = (wa / sum, wb / sum, wc / sum);
                let key = wa * a.key + wb * b.key + wc * c.key;
                let i = y * w + x;
                if let Some(([sa, sb, sc], facing)) = cut_triangle {
                    // Interpolated like the position: perspective-correct.
                    let (qa, qb, qc) = (wa * a.inv_w, wb * b.inv_w, wc * c.inv_w);
                    let q = qa + qb + qc;
                    let across = (qa * sa + qb * sb + qc * sc) / q;
                    if cut.is_some_and(|cut| cut.clips(across, facing)) {
                        // Left out, and counted once per surface.
                        if let Some(slot) = odd.get_mut(i)
                            && owns_sample(on_edge, [&a, &b, &c])
                        {
                            *slot = !*slot;
                        }
                        continue;
                    }
                }
                let Some(&nearest) = depth_key.get(i) else {
                    continue;
                };
                // Strict: on a tie the triangle drawn first wins, which
                // keeps the result independent of anything but mesh order.
                // Negated, so that a key that is not a number is no front.
                let front = key < nearest;
                // On an edge or a vertex the front lies on too: the same
                // surface as the front, whatever rounding made of its key.
                let place = if see_through {
                    shared_place(on_edge, [pa, pb, pc])
                } else {
                    0
                };
                let same_surface = place != 0 && front_on.get(i) == Some(&place);
                if !front {
                    // Strictly further than the front, and nearer than
                    // what was behind it so far: the new one behind.
                    let nearer_behind = see_through
                        && !same_surface
                        && key > nearest
                        && behind.get(i).is_some_and(|&(second, _)| key < second);
                    if !nearer_behind {
                        continue;
                    }
                }
                // Perspective-correct weights for attributes.
                let (qa, qb, qc) = (wa * a.inv_w, wb * b.inv_w, wc * c.inv_w);
                let q = qa + qb + qc;
                let (qa, qb, qc) = (qa / q, qb / q, qc / q);
                let position = pa * qa + pb * qb + pc * qc;
                let normal = (na * qa + nb * qb + nc * qc).normalized().or(flat_normal);
                let to_viewer = view.toward_viewer(position);
                let (base, light, finish) = if params.highlight.contains(face) {
                    (
                        params.highlight.color,
                        (
                            SHADE_HIGHLIGHT_AMBIENT,
                            SHADE_HIGHLIGHT_DIFFUSE,
                            SHADE_HIGHLIGHT_FILL,
                        ),
                        Finish::default(),
                    )
                } else {
                    let (color, finish) = colors
                        .look(face)
                        .unwrap_or((params.color, Finish::default()));
                    (color, (SHADE_AMBIENT, SHADE_DIFFUSE, SHADE_FILL), finish)
                };
                let rgb = match &analysis {
                    Some(a) => {
                        let value = qa * value_at(ia) + qb * value_at(ib) + qc * value_at(ic);
                        let slopes = a.has_stripes().then(|| {
                            stripe_slopes(
                                [(pa, na), (pb, nb), (pc, nc)],
                                [qa, qb, qc],
                                q,
                                inv_w,
                                &d_weights,
                            )
                        });
                        a.color_at(
                            normal,
                            to_viewer,
                            position - view.eye,
                            value,
                            slopes.as_ref(),
                        )
                    }
                    None => {
                        normal.map_or(base, |n| shade(base, light, finish, n, to_viewer, &view))
                    }
                };
                if !front {
                    if let Some(slot) = behind.get_mut(i) {
                        *slot = (key, rgb);
                    }
                    continue;
                }
                if let (Some(k_slot), Some(c_slot), Some(id_slot), Some(d_slot)) = (
                    depth_key.get_mut(i),
                    color.get_mut(i),
                    ids.get_mut(i),
                    depth.get_mut(i),
                ) {
                    // What was in front is now the one behind, unless it
                    // is this surface's other triangle.
                    if nearest.is_finite()
                        && !same_surface
                        && let Some(slot) = behind.get_mut(i)
                    {
                        *slot = (nearest, *c_slot);
                    }
                    if let Some(slot) = front_on.get_mut(i) {
                        *slot = place;
                    }
                    *k_slot = key;
                    *c_slot = rgb;
                    *id_slot = face;
                    *d_slot = distance_at(&view, [wa, wb, wc], [&a, &b, &c]);
                }
            }
        }
    }

    // The cap of the section: one more surface where the plane is inside a
    // body, drawn after the triangles, so that a triangle wins a tie.
    if let Some(cut) = cut.filter(|cut| cut.cap) {
        for (i, _) in odd.iter().enumerate().filter(|&(_, &inside)| inside) {
            let (px, py) = ((i % w) as f64 + 0.5, (i / w) as f64 + 0.5);
            let Some((key, dist, _)) = view.cap_at(&cut, px, py, near) else {
                continue;
            };
            let (Some(k_slot), Some(c_slot), Some(id_slot), Some(d_slot)) = (
                depth_key.get_mut(i),
                color.get_mut(i),
                ids.get_mut(i),
                depth.get_mut(i),
            ) else {
                continue;
            };
            let nearest = *k_slot;
            if key < nearest {
                if nearest.is_finite()
                    && let Some(slot) = behind.get_mut(i)
                {
                    *slot = (nearest, *c_slot);
                }
                *k_slot = key;
                *c_slot = cut.color;
                *id_slot = SECTION_FACE;
                *d_slot = dist;
                if let Some(slot) = front_on.get_mut(i) {
                    *slot = 0;
                }
            } else if key > nearest
                && let Some(slot) = behind.get_mut(i)
                && key < slot.0
            {
                *slot = (key, cut.color);
            }
        }
    }

    // See-through: the front over the one behind, per sample.
    if see_through {
        for ((front, &(_, back)), &face) in color.iter_mut().zip(&behind).zip(&ids) {
            if face == NO_FACE || params.highlight.contains(face) {
                continue;
            }
            for (channel, under) in front.iter_mut().zip(back) {
                let mixed = f64::from(*channel) * opacity + f64::from(under) * (1.0 - opacity);
                // Between two bytes: within 0..=255.
                *channel = mixed.round().clamp(0.0, 255.0) as u8;
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
        sample: 0.5 / f64::from(ssaa),
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

/// Where model points land in a picture of `width` × `height` pixels seen
/// from a camera: the renderer's own arithmetic for a vertex, for whoever
/// draws or looks for something over the picture (an edge near a
/// position, [`pick`]'s way round).
pub struct Viewpoint {
    view: View,
}

impl Viewpoint {
    /// An error for a picture [`render`] would refuse: its size, a camera
    /// that looks nowhere.
    pub fn new(camera: &Camera, width: u32, height: u32) -> Result<Self, RenderError> {
        if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
            return Err(RenderError::InvalidSize { width, height });
        }
        Ok(Self {
            view: View::new(camera, width as usize, height as usize)?,
        })
    }

    /// The position of `p` in the picture, in pixels (x right, y down from
    /// the top-left corner), and its distance along the view direction.
    /// `None` for a point at or behind the eye plane of a perspective
    /// camera and for one that is not finite.
    pub fn project(&self, p: Vec3) -> Option<[f64; 3]> {
        self.view.project(p).map(|at| [at.sx, at.sy, at.dist])
    }

    /// True for a perspective camera: along a straight line in the picture
    /// it is then the reciprocal of the distance that changes evenly.
    pub fn is_perspective(&self) -> bool {
        matches!(self.view.projection, Projection::Perspective { .. })
    }

    /// Distance of `p` in front of the eye, along the view direction.
    pub fn distance(&self, p: Vec3) -> f64 {
        (p - self.view.eye).dot(self.view.forward)
    }
}

/// What [`pick`] found under a position of the picture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    /// The face id of the triangle, as in [`Frame::ids`].
    pub face: u32,
    /// The index of the triangle in the mesh.
    pub triangle: usize,
    /// The value [`Frame::depth`] holds there: the distance of `point`
    /// along the view direction, by the renderer's own interpolation.
    pub depth: f64,
    /// The point of the triangle under the position, in model coordinates:
    /// the one that projects onto the position.
    pub point: Vec3,
}

/// What [`render`] draws at one sample: the triangle nearest to the viewer
/// under the position (`x`, `y`) of a picture of `width` × `height` pixels
/// seen from `camera`, with x to the right and y down from the top-left
/// corner, so that the centre of pixel (i, j) is (i + 0.5, j + 0.5).
///
/// It is the renderer's own arithmetic for one sample: at a pixel centre
/// the face is the one [`Frame::ids`] holds for that pixel in a picture of
/// the same size with one sample per pixel, triangles are clipped to the
/// near plane as the renderer clips them (the hit names the mesh's
/// triangle the part comes from), and a
/// tie goes to the triangle that comes first in the mesh.
///
/// `Ok(None)` when nothing is under the position, when it is outside the
/// picture, and when it is not a number.
pub fn pick(
    mesh: &Mesh,
    camera: &Camera,
    width: u32,
    height: u32,
    x: f64,
    y: f64,
) -> Result<Option<Hit>, RenderError> {
    pick_cut(mesh, camera, width, height, x, y, None)
}

/// [`pick`] in a picture cut open by `section` ([`render`], *Section*):
/// what is drawn there, never a face that is cut away. The cap is a hit
/// with [`SECTION_FACE`] as its face and no triangle (`usize::MAX`), at
/// the point of the plane under the position; [`cut_faces`] says which
/// faces were cut away in front of it.
pub fn pick_cut(
    mesh: &Mesh,
    camera: &Camera,
    width: u32,
    height: u32,
    x: f64,
    y: f64,
    section: Option<&Section>,
) -> Result<Option<Hit>, RenderError> {
    let Some(sample) = Sample::new(mesh, camera, width, height, x, y, section)? else {
        return Ok(None);
    };
    let mut state = SampleState::default();
    for (t, (tri, &face)) in mesh.triangles.iter().zip(&mesh.face_ids).enumerate() {
        for seen in sample.seen(mesh, t, *tri, face)?.into_iter().flatten() {
            seen.hit(&sample.view, x, y, &mut state, None);
        }
    }
    Ok(state.finish(&sample.view, sample.cut.as_ref(), sample.near, x, y))
}

/// The face ids of the surfaces that a section leaves out at the position
/// (`x`, `y`) of the picture [`pick_cut`] looks at, one per surface, in
/// mesh order: what was cut away along the ray through the position. The
/// plane is inside a body there when an odd number of them are that
/// body's. Empty without a section, outside the picture, and where nothing
/// is cut away.
pub fn cut_faces(
    mesh: &Mesh,
    camera: &Camera,
    width: u32,
    height: u32,
    x: f64,
    y: f64,
    section: Option<&Section>,
) -> Result<Vec<u32>, RenderError> {
    let Some(sample) = Sample::new(mesh, camera, width, height, x, y, section)? else {
        return Ok(Vec::new());
    };
    let mut state = SampleState::default();
    let mut faces = Vec::new();
    for (t, (tri, &face)) in mesh.triangles.iter().zip(&mesh.face_ids).enumerate() {
        for seen in sample.seen(mesh, t, *tri, face)?.into_iter().flatten() {
            seen.hit(&sample.view, x, y, &mut state, Some(&mut faces));
        }
    }
    Ok(faces)
}

/// Every face under the position (`x`, `y`) of the picture [`pick_cut`]
/// looks at, front to back: one hit per face, its nearest point along the
/// ray through the position, as [`pick_cut`] would find it with the
/// faces in front taken away. A section leaves out what it cuts away (its
/// cap is not a face and is not listed). Empty outside the picture; an
/// error as [`pick_cut`]. Ties keep the mesh's order.
pub fn pick_all(
    mesh: &Mesh,
    camera: &Camera,
    width: u32,
    height: u32,
    x: f64,
    y: f64,
    section: Option<&Section>,
) -> Result<Vec<Hit>, RenderError> {
    let Some(sample) = Sample::new(mesh, camera, width, height, x, y, section)? else {
        return Ok(Vec::new());
    };
    let mut state = SampleState {
        every: Some(Vec::new()),
        ..SampleState::default()
    };
    for (t, (tri, &face)) in mesh.triangles.iter().zip(&mesh.face_ids).enumerate() {
        for seen in sample.seen(mesh, t, *tri, face)?.into_iter().flatten() {
            seen.hit(&sample.view, x, y, &mut state, None);
        }
    }
    let mut every = state.every.unwrap_or_default();
    // Stable: a tie keeps the mesh's order, as the renderer's first
    // triangle wins.
    every.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut seen_faces = BTreeSet::new();
    Ok(every
        .into_iter()
        .filter(|(_, hit)| seen_faces.insert(hit.face))
        .map(|(_, hit)| hit)
        .collect())
}

/// The picture one position is looked up in.
struct Sample {
    view: View,
    cut: Option<Cut>,
    /// The near plane the picture clips at (`clip`); `None` in
    /// orthographic.
    near: Option<f64>,
}

impl Sample {
    /// `Ok(None)` for a position outside the picture or one that is not a
    /// number; an error for a picture [`render`] would refuse.
    fn new(
        mesh: &Mesh,
        camera: &Camera,
        width: u32,
        height: u32,
        x: f64,
        y: f64,
        section: Option<&Section>,
    ) -> Result<Option<Self>, RenderError> {
        if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
            return Err(RenderError::InvalidSize { width, height });
        }
        if mesh.normals.len() != mesh.positions.len() || mesh.face_ids.len() != mesh.triangles.len()
        {
            return Err(RenderError::InvalidMesh { triangle: 0 });
        }
        let view = View::new(camera, width as usize, height as usize)?;
        // Negated, so that a position that is not a number is outside too.
        if !(x >= 0.0 && y >= 0.0 && x < view.w && y < view.h) {
            return Ok(None);
        }
        let drawn = section_bounds(mesh);
        let cut = section.and_then(|section| Cut::new(section, &drawn));
        let near = clip::near_distance(
            &view,
            &clip::depth_bounds(&drawn, cut.is_some_and(|c| c.cap)),
        );
        Ok(Some(Self { view, cut, near }))
    }

    /// Triangle `t` as the picture has it: clipped to the near plane as the
    /// renderer clips it, one or two triangles (both named `t`), without
    /// cloning the mesh; `None` for what the renderer does not draw.
    fn seen(
        &self,
        mesh: &Mesh,
        t: usize,
        tri: [u32; 3],
        face: u32,
    ) -> Result<[Option<Seen>; 2], RenderError> {
        let vertex = |k: u32| {
            mesh.positions
                .get(k as usize)
                .copied()
                .ok_or(RenderError::InvalidMesh { triangle: t })
        };
        let [ia, ib, ic] = tri;
        let corners = [vertex(ia)?, vertex(ib)?, vertex(ic)?];
        Ok(seen_clipped(
            &self.view,
            self.near,
            t,
            face,
            corners,
            self.cut.as_ref(),
        ))
    }
}

/// [`Seen::new`] of the parts of a triangle in front of `near`.
fn seen_clipped(
    view: &View,
    near: Option<f64>,
    t: usize,
    face: u32,
    corners: [Vec3; 3],
    cut: Option<&Cut>,
) -> [Option<Seen>; 2] {
    let Some(near) = near else {
        return [Seen::new(view, t, face, corners, cut), None];
    };
    let (parts, n) = clip::clip_corners(corners, view.eye, view.forward, near);
    let mut out = [None, None];
    for (slot, part) in out.iter_mut().zip(parts).take(n) {
        *slot = Seen::new(view, t, face, part, cut);
    }
    out
}

/// What one position of a picture has found so far.
#[derive(Default)]
struct SampleState {
    /// The nearest triangle that is drawn there, with its depth key.
    best: Option<(f64, Hit)>,
    /// Whether the number of surfaces left out there by a section is odd.
    odd: bool,
    /// Every triangle drawn there, with its depth key, when asked for
    /// ([`pick_all`]).
    every: Option<Vec<(f64, Hit)>>,
}

impl SampleState {
    /// The hit, once every triangle was looked at: the nearest triangle,
    /// or the cap of the section where that is nearer.
    fn finish(
        self,
        view: &View,
        cut: Option<&Cut>,
        near: Option<f64>,
        x: f64,
        y: f64,
    ) -> Option<Hit> {
        let triangle = self.best;
        let cap = cut
            .filter(|cut| cut.cap && self.odd)
            .and_then(|cut| view.cap_at(cut, x, y, near));
        match (triangle, cap) {
            // Strict, as in the renderer: the triangle wins a tie.
            (Some((nearest, _)), Some((key, dist, point))) if key < nearest => Some(Hit {
                face: SECTION_FACE,
                triangle: usize::MAX,
                depth: dist,
                point,
            }),
            (None, Some((_, dist, point))) => Some(Hit {
                face: SECTION_FACE,
                triangle: usize::MAX,
                depth: dist,
                point,
            }),
            (triangle, _) => triangle.map(|(_, hit)| hit),
        }
    }
}

/// A triangle of a mesh as a picture has it: its corners in space and
/// where they land.
#[derive(Clone, Copy)]
struct Seen {
    triangle: usize,
    face: u32,
    corners: [Vec3; 3],
    projected: [Projected; 3],
    /// Twice its signed area in the picture.
    area: f64,
    /// In a picture with a section: the section, the corners' distances
    /// from the plane ([`Cut::side`]) and which way the triangle faces.
    cut: Option<(Cut, [f64; 3], f64)>,
}

impl Seen {
    /// `None` for a triangle the renderer does not draw: a corner at or
    /// behind the eye plane (the callers clip to the near plane first, so
    /// only a corner that is not a number), no area in the picture.
    fn new(
        view: &View,
        triangle: usize,
        face: u32,
        corners: [Vec3; 3],
        cut: Option<&Cut>,
    ) -> Option<Self> {
        let [pa, pb, pc] = corners;
        let (a, b, c) = (view.project(pa)?, view.project(pb)?, view.project(pc)?);
        let area = edge(a.sx, a.sy, b.sx, b.sy, c.sx, c.sy);
        let cut = cut.map(|cut| {
            let flat = (pb - pa).cross(pc - pa).normalized();
            (
                *cut,
                corners.map(|p| cut.side(p)),
                flat.map_or(0.0, |n| n.dot(cut.normal)),
            )
        });
        (area.is_finite() && area != 0.0).then_some(Self {
            triangle,
            face,
            corners,
            projected: [a, b, c],
            area,
            cut,
        })
    }

    /// The box around it in the picture: least x, least y, greatest x,
    /// greatest y.
    fn bounds(&self) -> [f64; 4] {
        let [a, b, c] = &self.projected;
        [
            a.sx.min(b.sx).min(c.sx),
            a.sy.min(b.sy).min(c.sy),
            a.sx.max(b.sx).max(c.sx),
            a.sy.max(b.sy).max(c.sy),
        ]
    }

    /// Replaces the best of `state` when this triangle is under (`px`,
    /// `py`) and nearer than what it holds: the renderer's test of one
    /// sample. Where a section leaves the triangle out, that is counted
    /// instead, and its face is added to `left_out`.
    fn hit(
        &self,
        view: &View,
        px: f64,
        py: f64,
        state: &mut SampleState,
        left_out: Option<&mut Vec<u32>>,
    ) {
        let [pa, pb, pc] = self.corners;
        let [a, b, c] = &self.projected;
        // The renderer visits the samples within the triangle's box only.
        let inside =
            |lo: f64, hi: f64, at: f64| lo.is_finite() && hi.is_finite() && lo <= at && at <= hi;
        let [x0, y0, x1, y1] = self.bounds();
        if !inside(x0, x1, px) || !inside(y0, y1, py) {
            return;
        }
        let wa = shared_edge(b, c, px, py) / self.area;
        let wb = shared_edge(c, a, px, py) / self.area;
        let wc = shared_edge(a, b, px, py) / self.area;
        // Negated, so that a weight that is not a number is outside.
        if !(wa >= 0.0 && wb >= 0.0 && wc >= 0.0) {
            return;
        }
        let on_edge = [wa, wb, wc];
        let sum = wa + wb + wc;
        let (wa, wb, wc) = (wa / sum, wb / sum, wc / sum);
        let key = wa * a.key + wb * b.key + wc * c.key;
        let (qa, qb, qc) = (wa * a.inv_w, wb * b.inv_w, wc * c.inv_w);
        let q = qa + qb + qc;
        if let Some((cut, [sa, sb, sc], facing)) = self.cut
            && cut.clips((qa * sa + qb * sb + qc * sc) / q, facing)
        {
            if owns_sample(on_edge, [a, b, c]) {
                state.odd = !state.odd;
                if let Some(faces) = left_out {
                    faces.push(self.face);
                }
            }
            return;
        }
        // Strict, as in the renderer: the first triangle wins a tie.
        let nearer = state
            .best
            .as_ref()
            .is_none_or(|(nearest, _)| key < *nearest);
        if key.is_nan() || (!nearer && state.every.is_none()) {
            return;
        }
        let (qa, qb, qc) = (qa / q, qb / q, qc / q);
        let point = pa * qa + pb * qb + pc * qc;
        if !point.is_finite() {
            return;
        }
        let hit = Hit {
            face: self.face,
            triangle: self.triangle,
            depth: distance_at(view, [wa, wb, wc], [a, b, c]),
            point,
        };
        if let Some(every) = &mut state.every {
            every.push((key, hit));
        }
        if nearer {
            state.best = Some((key, hit));
        }
    }
}

/// The part of a picture within some pixels of a position: the triangles
/// that can be under a position there, so that many positions close
/// together are looked up ([`Self::pick`]) without going through the
/// whole mesh for each.
pub struct Neighbourhood {
    seen: Vec<Seen>,
    /// The box the triangles were kept for: least x, least y, greatest x,
    /// greatest y.
    within: [f64; 4],
    view: View,
    cut: Option<Cut>,
    /// The near plane the picture clips at (`clip`).
    near: Option<f64>,
}

impl Neighbourhood {
    /// The triangles of `mesh` that can be under a position within
    /// `radius` pixels (along x and along y) of (`x`, `y`) in the picture
    /// [`pick`] looks at. Errors as [`pick`]. A position or a radius that
    /// is not a number keeps nothing.
    pub fn new(
        mesh: &Mesh,
        camera: &Camera,
        width: u32,
        height: u32,
        x: f64,
        y: f64,
        radius: f64,
    ) -> Result<Self, RenderError> {
        Self::new_cut(mesh, camera, width, height, x, y, radius, None)
    }

    /// [`Neighbourhood::new`] for a picture cut open by `section`, as
    /// [`pick_cut`] sees it.
    #[allow(clippy::too_many_arguments)]
    pub fn new_cut(
        mesh: &Mesh,
        camera: &Camera,
        width: u32,
        height: u32,
        x: f64,
        y: f64,
        radius: f64,
        section: Option<&Section>,
    ) -> Result<Self, RenderError> {
        if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
            return Err(RenderError::InvalidSize { width, height });
        }
        if mesh.normals.len() != mesh.positions.len() || mesh.face_ids.len() != mesh.triangles.len()
        {
            return Err(RenderError::InvalidMesh { triangle: 0 });
        }
        let view = View::new(camera, width as usize, height as usize)?;
        let drawn = section_bounds(mesh);
        let cut = section.and_then(|section| Cut::new(section, &drawn));
        let near = clip::near_distance(
            &view,
            &clip::depth_bounds(&drawn, cut.is_some_and(|c| c.cap)),
        );
        let within = [x - radius, y - radius, x + radius, y + radius];
        let mut seen = Vec::new();
        let usable = within.iter().all(|c| c.is_finite());
        for (t, (tri, &face)) in mesh.triangles.iter().zip(&mesh.face_ids).enumerate() {
            let vertex = |k: u32| {
                mesh.positions
                    .get(k as usize)
                    .copied()
                    .ok_or(RenderError::InvalidMesh { triangle: t })
            };
            let [ia, ib, ic] = *tri;
            let corners = [vertex(ia)?, vertex(ib)?, vertex(ic)?];
            if !usable {
                continue;
            }
            for triangle in seen_clipped(&view, near, t, face, corners, cut.as_ref())
                .into_iter()
                .flatten()
            {
                let [x0, y0, x1, y1] = triangle.bounds();
                // Negated, so that a box that is not a number is kept out.
                if x0 <= within[2] && x1 >= within[0] && y0 <= within[3] && y1 >= within[1] {
                    seen.push(triangle);
                }
            }
        }
        Ok(Self {
            seen,
            within,
            view,
            cut,
            near,
        })
    }

    /// The section the picture is cut open with, made ready for the mesh.
    pub fn cut(&self) -> Option<&Cut> {
        self.cut.as_ref()
    }

    /// What [`pick`] gives for the position (`x`, `y`): the same triangle,
    /// by the same arithmetic. `None` also for a position outside the
    /// part of the picture this was made for.
    pub fn pick(&self, x: f64, y: f64) -> Option<Hit> {
        let [x0, y0, x1, y1] = self.within;
        let (w, h) = (self.view.w, self.view.h);
        if !(x >= 0.0 && y >= 0.0 && x < w && y < h && x >= x0 && x <= x1 && y >= y0 && y <= y1) {
            return None;
        }
        let mut state = SampleState::default();
        for triangle in &self.seen {
            triangle.hit(&self.view, x, y, &mut state, None);
        }
        state.finish(&self.view, self.cut.as_ref(), self.near, x, y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_tess::{TessParams, tessellate_patches};
    use craft3d_testkit::teapot::{PATCH_COUNT, teapot};

    /// The cap is clipped at the near plane as the triangles are (and as
    /// the GPU clips its cap): with the eye a quarter of the near distance
    /// from the cut plane, the cap's point straight ahead is in front of
    /// the near plane and is not drawn; with no near plane (or further
    /// away) it is.
    #[test]
    fn the_cap_is_clipped_at_the_near_plane() {
        let camera = Camera {
            eye: Vec3::ZERO,
            target: Vec3::X,
            up: Vec3::Z,
            projection: Projection::Perspective { fov_y_deg: 50.0 },
        };
        let view = View::new(&camera, 40, 30).unwrap();
        let bounds = Bbox3::from_points([Vec3::new(-1.0, -1.0, -1.0), Vec3::new(9.0, 1.0, 1.0)]);
        let near = clip::near_distance(&view, &clip::depth_bounds(&bounds, true)).unwrap();
        assert!(near > 0.0 && near < 1e-4, "{near}");
        let cap_at = |x: f64| {
            let section = Section {
                origin: Vec3::new(x, 0.0, 0.0),
                normal: Vec3::new(-1.0, 0.0, 0.0),
                cap: true,
                color: [1, 2, 3],
            };
            let cut = Cut::new(&section, &bounds).unwrap();
            (
                view.cap_at(&cut, 20.0, 15.0, Some(near)),
                view.cap_at(&cut, 20.0, 15.0, None),
            )
        };
        let (clipped, unclipped) = cap_at(0.25 * near);
        assert!(clipped.is_none());
        assert!(unclipped.is_some());
        let (kept, _) = cap_at(4.0 * near);
        let (_, dist, _) = kept.unwrap();
        assert!((dist - 4.0 * near).abs() < 1e-12, "{dist}");
    }

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

    fn teapot_mesh_and_camera(width: u32, height: u32, perspective: bool) -> (Mesh, Camera) {
        let mesh = tessellate_patches(&teapot().unwrap(), &TessParams::default()).unwrap();
        let aspect = f64::from(width) / f64::from(height);
        let toward = Vec3::new(0.35, -1.0, 0.45);
        let camera = if perspective {
            Camera::framing(&mesh.bbox(), toward, Vec3::Z, 30.0, aspect)
        } else {
            Camera::framing_orthographic(&mesh.bbox(), toward, Vec3::Z, aspect)
        }
        .unwrap();
        (mesh, camera)
    }

    #[test]
    fn a_pick_at_a_pixel_centre_is_the_id_buffer_of_that_pixel() {
        for perspective in [true, false] {
            let (width, height) = (96, 72);
            let (mesh, camera) = teapot_mesh_and_camera(width, height, perspective);
            let frame = render(&mesh, &camera, &params(width, height, 1)).unwrap();
            let (mut hits, mut misses) = (0, 0);
            for y in 0..height {
                // Every fifth pixel, shifted from row to row: each pick
                // looks at every triangle.
                for x in (y % 5..width).step_by(5) {
                    let i = (y * width + x) as usize;
                    let hit = pick(
                        &mesh,
                        &camera,
                        width,
                        height,
                        f64::from(x) + 0.5,
                        f64::from(y) + 0.5,
                    )
                    .unwrap();
                    match hit {
                        None => {
                            misses += 1;
                            assert_eq!(frame.ids[i], NO_FACE, "({x}, {y})");
                        }
                        Some(hit) => {
                            hits += 1;
                            // The very same numbers, not close ones.
                            assert_eq!(hit.face, frame.ids[i], "({x}, {y})");
                            assert_eq!(hit.depth, frame.depth[i], "({x}, {y})");
                            assert_eq!(mesh.face_ids[hit.triangle], hit.face);
                            // The point is the one seen at that pixel: it
                            // projects back to the pixel's centre.
                            let view = View::new(&camera, width as usize, height as usize).unwrap();
                            let back = view.project(hit.point).unwrap();
                            assert!(
                                (back.sx - (f64::from(x) + 0.5)).abs() < 1e-6
                                    && (back.sy - (f64::from(y) + 0.5)).abs() < 1e-6,
                                "({x}, {y}): ({}, {})",
                                back.sx,
                                back.sy
                            );
                        }
                    }
                }
            }
            assert!(hits > 150 && misses > 150, "{hits} hits, {misses} misses");
        }
    }

    #[test]
    fn a_pick_gives_the_point_under_the_position() {
        // A unit square at z = 0 seen from above, 100 pixels across the 2
        // units from -0.5 to 1.5: 50 pixels per unit, and y runs the other
        // way.
        let mut mesh = Mesh::default();
        unit_square(0.0, 7, &mut mesh);
        for projection in [
            Projection::Orthographic { half_height: 1.0 },
            Projection::Perspective {
                fov_y_deg: 2.0 * (1.0_f64 / 5.0).patan().to_degrees(),
            },
        ] {
            let camera = Camera {
                projection,
                ..top_down(1.0)
            };
            let hit = pick(&mesh, &camera, 100, 100, 60.0, 40.0).unwrap().unwrap();
            assert_eq!(hit.face, 7);
            assert!(
                (hit.point - Vec3::new(0.7, 0.7, 0.0)).length() < 1e-12,
                "{:?}",
                hit.point
            );
            assert!((hit.depth - 5.0).abs() < 1e-12);
            // Beside the square: nothing.
            assert_eq!(pick(&mesh, &camera, 100, 100, 10.0, 40.0).unwrap(), None);
        }
    }

    #[test]
    fn a_pick_outside_the_picture_or_nowhere_is_nothing_and_never_a_panic() {
        let mut mesh = Mesh::default();
        unit_square(0.0, 0, &mut mesh);
        let camera = top_down(1.0);
        let hostile = [
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            -1.0,
            -1e-9,
            100.0,
            1e300,
            -1e300,
        ];
        for &x in &hostile {
            for &y in hostile.iter().chain(&[50.0]) {
                assert_eq!(pick(&mesh, &camera, 100, 100, x, y).unwrap(), None);
                assert_eq!(pick(&mesh, &camera, 100, 100, y, x).unwrap(), None);
            }
        }
        // The last position inside is still inside: seen this close the
        // square fills the picture.
        let filled = top_down(0.5);
        for (x, y) in [(99.999, 99.999), (0.0, 0.0), (99.999, 0.001)] {
            assert!(pick(&mesh, &filled, 100, 100, x, y).unwrap().is_some());
        }
        assert_eq!(pick(&mesh, &filled, 100, 100, 100.0, 50.0).unwrap(), None);
        assert!(matches!(
            pick(&mesh, &camera, 0, 100, 1.0, 1.0),
            Err(RenderError::InvalidSize { .. })
        ));
        assert!(matches!(
            pick(&mesh, &camera, MAX_SIDE + 1, 100, 1.0, 1.0),
            Err(RenderError::InvalidSize { .. })
        ));
        let nowhere = Camera {
            target: camera.eye,
            ..camera
        };
        assert!(pick(&mesh, &nowhere, 100, 100, 50.0, 50.0).is_err());
        // A mesh that names a vertex it does not have, or holds numbers
        // that are not numbers.
        let mut broken = mesh.clone();
        broken.triangles.push([0, 1, 99]);
        broken.face_ids.push(1);
        assert!(matches!(
            pick(&broken, &camera, 100, 100, 50.0, 50.0),
            Err(RenderError::InvalidMesh { triangle: 2 })
        ));
        let mut unlabelled = mesh.clone();
        unlabelled.face_ids.pop();
        assert!(pick(&unlabelled, &camera, 100, 100, 50.0, 50.0).is_err());
        let mut wild = mesh.clone();
        wild.positions[0] = Vec3::new(f64::NAN, 0.0, 0.0);
        wild.positions[2] = Vec3::new(1e308, -1e308, f64::INFINITY);
        let _ = pick(&wild, &camera, 100, 100, 50.0, 50.0).unwrap();
        assert_eq!(
            pick(&Mesh::default(), &camera, 100, 100, 50.0, 50.0).unwrap(),
            None
        );
    }

    #[test]
    fn a_neighbourhood_answers_as_pick_does_within_its_radius_and_nothing_outside() {
        for perspective in [true, false] {
            let (width, height) = (96, 72);
            let (mesh, camera) = teapot_mesh_and_camera(width, height, perspective);
            let view = Viewpoint::new(&camera, width, height).unwrap();
            assert_eq!(view.is_perspective(), perspective);
            let (mut hits, mut kept, mut all) = (0, 0, 0);
            for (cx, cy, radius) in [(48.3, 36.9, 6.0), (20.0, 30.5, 3.5), (2.0, 70.0, 9.0)] {
                let near =
                    Neighbourhood::new(&mesh, &camera, width, height, cx, cy, radius).unwrap();
                kept += near.seen.len();
                all += mesh.triangles.len();
                for dy in -12..=12 {
                    for dx in -12..=12 {
                        let (x, y) = (cx + f64::from(dx) * 0.77, cy + f64::from(dy) * 0.77);
                        let within = (x - cx).abs() <= radius && (y - cy).abs() <= radius;
                        let whole = pick(&mesh, &camera, width, height, x, y).unwrap();
                        if within {
                            // The very same hit, not a close one.
                            assert_eq!(near.pick(x, y), whole, "({x}, {y})");
                            hits += usize::from(whole.is_some());
                        } else {
                            assert_eq!(near.pick(x, y), None, "({x}, {y})");
                        }
                        // A point that was hit lands where it was hit, at
                        // its own distance.
                        if let Some(hit) = whole {
                            let [sx, sy, dist] = view.project(hit.point).unwrap();
                            assert!((sx - x).abs() < 1e-6 && (sy - y).abs() < 1e-6);
                            assert!((dist - view.distance(hit.point)).abs() < 1e-12);
                        }
                    }
                }
            }
            assert!(hits > 100, "{hits}");
            assert!(kept * 4 < all, "{kept} of {all} triangles were kept");
        }
        // Hostile numbers: nothing kept, nothing found, no panic.
        let mut mesh = Mesh::default();
        unit_square(0.0, 0, &mut mesh);
        let camera = top_down(1.0);
        for bad in [f64::NAN, f64::INFINITY, -1e300] {
            for (x, y, r) in [(bad, 50.0, 4.0), (50.0, bad, 4.0), (50.0, 50.0, bad)] {
                let near = Neighbourhood::new(&mesh, &camera, 100, 100, x, y, r).unwrap();
                assert_eq!(near.pick(50.0, 50.0), None);
                assert_eq!(near.pick(bad, bad), None);
            }
        }
        assert!(Neighbourhood::new(&mesh, &camera, 0, 100, 1.0, 1.0, 1.0).is_err());
        assert!(Viewpoint::new(&camera, 100, MAX_SIDE + 1).is_err());
        let view = Viewpoint::new(&camera, 100, 100).unwrap();
        assert_eq!(view.project(Vec3::new(f64::NAN, 0.0, 0.0)), None);
        let mut broken = mesh.clone();
        broken.triangles.push([0, 1, 99]);
        broken.face_ids.push(1);
        assert!(Neighbourhood::new(&broken, &camera, 100, 100, 50.0, 50.0, 4.0).is_err());
    }

    /// Three unit squares over one another seen from above, in an order.
    fn stack(order: &[(f64, u32)]) -> Mesh {
        let mut mesh = Mesh::default();
        for &(z, face) in order {
            unit_square(z, face, &mut mesh);
        }
        // A normal that leans differently per square, so that each has a
        // shade of its own (a flat stack would be one colour).
        // (By the square's face, not by its place in the mesh.)
        for (k, normal) in mesh.normals.iter_mut().enumerate() {
            let face = order.get(k / 4).map_or(0, |&(_, face)| face);
            let lean = 0.25 * f64::from(face);
            *normal = Vec3::new(lean, 0.3 - lean, 1.0);
        }
        mesh
    }

    #[test]
    fn see_through_is_the_front_over_the_one_behind_whatever_the_order() {
        let levels = [(2.0, 1_u32), (1.0, 2), (0.0, 3)];
        let camera = top_down(1.0);
        let opaque = |mesh: &Mesh| render(mesh, &camera, &params(40, 40, 1)).unwrap();
        let at = |frame: &Frame| -> [u8; 3] {
            let i = (20 * 40 + 20) * 3;
            [frame.rgb[i], frame.rgb[i + 1], frame.rgb[i + 2]]
        };
        // What each square looks like alone, and the top two together.
        let alone: Vec<[u8; 3]> = levels
            .iter()
            .map(|&level| {
                // In the stack's place, so that it has the stack's normal.
                let all = stack(&levels);
                let k = levels.iter().position(|l| *l == level).unwrap();
                let one = Mesh {
                    positions: all.positions[k * 4..k * 4 + 4].to_vec(),
                    normals: all.normals[k * 4..k * 4 + 4].to_vec(),
                    triangles: vec![[0, 1, 2], [0, 2, 3]],
                    face_ids: vec![level.1; 2],
                };
                at(&opaque(&one))
            })
            .collect();
        assert!(alone[0] != alone[1] && alone[1] != alone[2], "{alone:?}");
        let plain = opaque(&stack(&levels));
        assert_eq!(at(&plain), alone[0]);
        for opacity in [0.9, 0.5, 0.25, 0.1, 0.0] {
            let mut first: Option<Frame> = None;
            for order in [
                [0, 1, 2],
                [2, 1, 0],
                [1, 2, 0],
                [1, 0, 2],
                [0, 2, 1],
                [2, 0, 1],
            ] {
                let mesh = stack(&order.map(|k| levels[k]));
                let frame = render(
                    &mesh,
                    &camera,
                    &RenderParams {
                        opacity,
                        ..params(40, 40, 1)
                    },
                )
                .unwrap();
                // The front over the second, by the formula; the third
                // square does not show.
                let expected: Vec<u8> = (0..3)
                    .map(|c| {
                        (f64::from(alone[0][c]) * opacity
                            + f64::from(alone[1][c]) * (1.0 - opacity))
                            .round() as u8
                    })
                    .collect();
                assert_eq!(
                    at(&frame).to_vec(),
                    expected,
                    "opacity {opacity}, {order:?}"
                );
                // The id and the depth are the front's, as in the opaque
                // picture.
                assert_eq!(frame.ids, plain.ids);
                assert_eq!(frame.depth, plain.depth);
                // Beside the squares: the background, untouched.
                assert_eq!(&frame.rgb[0..3], &RenderParams::default().background);
                // And every order gives the same picture, byte for byte.
                match &first {
                    None => first = Some(frame),
                    Some(first) => assert_eq!(first.rgb, frame.rgb, "{order:?}"),
                }
            }
        }
        // One square alone is seen through to the background.
        let mut one = Mesh::default();
        unit_square(0.0, 1, &mut one);
        let solid = at(&opaque(&one));
        let half = render(
            &one,
            &camera,
            &RenderParams {
                opacity: 0.5,
                ..params(40, 40, 1)
            },
        )
        .unwrap();
        let background = RenderParams::default().background;
        for c in 0..3 {
            let mixed = (f64::from(solid[c]) * 0.5 + f64::from(background[c]) * 0.5).round();
            assert_eq!(f64::from(at(&half)[c]), mixed);
        }
        // Two squares at the very same place are one surface: what is
        // behind is the background, not the twin.
        let twins = stack(&[(1.0, 1), (1.0, 2)]);
        let seen = render(
            &twins,
            &camera,
            &RenderParams {
                opacity: 0.5,
                ..params(40, 40, 1)
            },
        )
        .unwrap();
        let front = at(&opaque(&twins));
        for c in 0..3 {
            let mixed = (f64::from(front[c]) * 0.5 + f64::from(background[c]) * 0.5).round();
            assert_eq!(f64::from(at(&seen)[c]), mixed);
        }
    }

    /// A sample exactly on the edge two triangles share is inside both,
    /// with depth keys that differ by rounding. The sheet must not show
    /// through itself there, and what is really behind it must.
    #[test]
    fn a_surface_is_not_behind_itself_on_an_edge_its_triangles_share() {
        // A tilted square of two triangles seen head-on at 240 × 180: the
        // centres of the pixels (60 + k, 149 - k) lie on its diagonal.
        let sheet = |z: f64, face: u32, mesh: &mut Mesh| {
            let corner = |x: f64, y: f64| Vec3::new(x, y, z + 0.2 * y);
            quad(
                [
                    corner(-1.0, -1.0),
                    corner(1.0, -1.0),
                    corner(1.0, 1.0),
                    corner(-1.0, 1.0),
                ],
                face,
                mesh,
            );
        };
        let camera = Camera {
            eye: Vec3::new(0.0, 0.0, 6.0),
            target: Vec3::ZERO,
            up: Vec3::Y,
            projection: Projection::Orthographic { half_height: 1.5 },
        };
        let see_through = RenderParams {
            opacity: 0.5,
            background: [0, 0, 0],
            ..params(240, 180, 1)
        };
        let opaque = RenderParams {
            opacity: 1.0,
            ..see_through
        };
        let mut one = Mesh::default();
        sheet(0.0, 4, &mut one);
        let front = render(&one, &camera, &opaque).unwrap();
        let half = render(&one, &camera, &see_through).unwrap();
        let mut on_the_diagonal = 0;
        for (i, &id) in front.ids.iter().enumerate() {
            if id == NO_FACE {
                continue;
            }
            let (x, y) = (i % 240, i / 240);
            on_the_diagonal += usize::from(x + y == 209);
            for c in 0..3 {
                // Over black at a half: half the front, rounded.
                let expected = (f64::from(front.rgb[i * 3 + c]) * 0.5).round() as u8;
                assert_eq!(
                    half.rgb[i * 3 + c],
                    expected,
                    "the sheet shows through itself at pixel ({x}, {y})"
                );
            }
        }
        assert_eq!(on_the_diagonal, 120, "the view is not on the diagonal");
        // A second sheet behind the first is behind it on the diagonal as
        // everywhere else, in either order of the triangles: every pixel
        // of the first is the blend of the two.
        for first_in_front in [true, false] {
            let mut two = Mesh::default();
            if first_in_front {
                sheet(0.0, 4, &mut two);
                sheet(-1.0, 5, &mut two);
            } else {
                sheet(-1.0, 5, &mut two);
                sheet(0.0, 4, &mut two);
            }
            let both = render(&two, &camera, &see_through).unwrap();
            let clear = RenderParams {
                opacity: 0.0,
                ..see_through
            };
            let behind = render(&two, &camera, &clear).unwrap();
            assert_eq!(render(&two, &camera, &opaque).unwrap().ids, front.ids);
            for (i, _) in front.ids.iter().enumerate().filter(|&(_, &id)| id == 4) {
                for c in 0..3 {
                    let mixed = (f64::from(front.rgb[i * 3 + c]) * 0.5
                        + f64::from(behind.rgb[i * 3 + c]) * 0.5)
                        .round() as u8;
                    assert_eq!(both.rgb[i * 3 + c], mixed, "pixel {i}");
                    assert!(behind.rgb[i * 3 + c] > 0, "nothing behind at pixel {i}");
                }
            }
        }
    }

    #[test]
    fn at_opacity_one_the_picture_is_the_opaque_one_and_a_highlight_is_not_seen_through() {
        for perspective in [true, false] {
            let (mesh, camera) = teapot_mesh_and_camera(96, 72, perspective);
            let plain = render(&mesh, &camera, &params(96, 72, 2)).unwrap();
            // 1, above 1 and what is not a number: the opaque picture.
            for opacity in [1.0, 7.0, f64::NAN, f64::INFINITY] {
                let same = render(
                    &mesh,
                    &camera,
                    &RenderParams {
                        opacity,
                        ..params(96, 72, 2)
                    },
                )
                .unwrap();
                assert_eq!(same, plain, "opacity {opacity}");
            }
            // Below 1 the picture changes where there is a body, and only
            // there; ids and depth stay.
            for opacity in [0.6, -3.0] {
                let seen = render(
                    &mesh,
                    &camera,
                    &RenderParams {
                        opacity,
                        ..params(96, 72, 1)
                    },
                )
                .unwrap();
                let plain = render(&mesh, &camera, &params(96, 72, 1)).unwrap();
                assert_eq!((&seen.ids, &seen.depth), (&plain.ids, &plain.depth));
                let mut changed = 0;
                for (i, &id) in plain.ids.iter().enumerate() {
                    let differs = seen.rgb[i * 3..i * 3 + 3] != plain.rgb[i * 3..i * 3 + 3];
                    assert!(!differs || id != NO_FACE, "the background changed");
                    changed += usize::from(differs);
                }
                assert!(changed > 500, "{changed}");
            }
            // A highlighted face in front keeps its opaque colour.
            let face = plain.ids[plain.ids.len() / 2 + 48];
            assert_ne!(face, NO_FACE);
            let (highlight, _) = Highlight::new([64, 196, 212], [face]);
            let marked = |opacity: f64| {
                render(
                    &mesh,
                    &camera,
                    &RenderParams {
                        opacity,
                        highlight,
                        ..params(96, 72, 1)
                    },
                )
                .unwrap()
            };
            let (solid, seen) = (marked(1.0), marked(0.3));
            let mut kept = 0;
            for (i, &id) in solid.ids.iter().enumerate() {
                if id == face {
                    assert_eq!(seen.rgb[i * 3..i * 3 + 3], solid.rgb[i * 3..i * 3 + 3]);
                    kept += 1;
                }
            }
            assert!(kept > 20, "{kept}");
        }
    }

    #[test]
    fn a_highlight_holds_runs_of_faces_and_says_what_it_left_out() {
        assert!(Highlight::NONE.is_empty());
        assert_eq!(Highlight::default(), Highlight::NONE);
        assert!(!Highlight::NONE.contains(0) && !Highlight::NONE.contains(1));
        let (some, left_out) = Highlight::new([1, 2, 3], [5, 3, 4, 9, NO_FACE, 5]);
        assert_eq!(left_out, 0);
        assert_eq!(some.ranges().collect::<Vec<_>>(), vec![[3, 5], [9, 9]]);
        assert_eq!(some.len(), 4);
        for face in 0..12 {
            assert_eq!(some.contains(face), matches!(face, 3..=5 | 9), "{face}");
        }
        assert!(!some.contains(NO_FACE));
        // Ten separate faces: the first eight are kept.
        let (many, left_out) = Highlight::new([0; 3], (0..10).map(|k| k * 2));
        assert_eq!(left_out, 2);
        assert_eq!(many.len(), MAX_HIGHLIGHT_RANGES as u64);
        assert!(many.contains(14) && !many.contains(16));
        // The largest id that is a face, next to the one that is none.
        let (top, _) = Highlight::new([0; 3], [u32::MAX - 1, u32::MAX]);
        assert!(top.contains(u32::MAX - 1) && !top.contains(u32::MAX));
    }

    #[test]
    fn a_highlighted_face_is_shaded_from_its_own_colour_and_nothing_else_changes() {
        let (width, height) = (160, 120);
        let (mesh, camera) = teapot_mesh_and_camera(width, height, true);
        let plain = render(&mesh, &camera, &params(width, height, 1)).unwrap();
        let face = plain.ids[(60 * width + 80) as usize];
        assert_ne!(face, NO_FACE);
        let (highlight, _) = Highlight::new([255, 140, 0], [face]);
        let marked = render(
            &mesh,
            &camera,
            &RenderParams {
                highlight,
                ..params(width, height, 1)
            },
        )
        .unwrap();
        assert_eq!(marked.ids, plain.ids);
        assert_eq!(marked.depth, plain.depth);
        let (mut changed, mut bluer) = (0, 0);
        for (i, (a, b)) in plain.rgb.chunks(3).zip(marked.rgb.chunks(3)).enumerate() {
            if plain.ids[i] == face {
                // Orange where the body is grey: less blue than red.
                changed += usize::from(a != b);
                bluer += usize::from(b[2] >= b[0]);
            } else {
                assert_eq!(a, b, "pixel {i} is not of the face");
            }
        }
        let pixels = plain.ids.iter().filter(|&&id| id == face).count();
        assert!(pixels > 50, "{pixels}");
        assert_eq!(changed, pixels);
        // Only where the specular highlight saturates is it not orange.
        assert!(bluer * 20 <= pixels, "{bluer} of {pixels}");
        // A highlight of a face the mesh does not have changes nothing.
        let (absent, _) = Highlight::new([255, 0, 0], [1_000_000]);
        let same = render(
            &mesh,
            &camera,
            &RenderParams {
                highlight: absent,
                ..params(width, height, 1)
            },
        )
        .unwrap();
        assert_eq!(same, plain);
    }

    #[test]
    fn a_face_with_a_colour_of_its_own_is_shaded_from_it_and_nothing_else_changes() {
        let (width, height) = (160, 120);
        let (mesh, camera) = teapot_mesh_and_camera(width, height, true);
        let base = params(width, height, 1);
        let plain = render(&mesh, &camera, &base).unwrap();
        // No colours, or the base colour itself: bit for bit the plain picture.
        assert_eq!(
            render_colored(&mesh, &FaceColors::new(), &camera, &base).unwrap(),
            plain
        );
        let face = plain.ids[(60 * width + 80) as usize];
        assert_ne!(face, NO_FACE);
        let mut same = FaceColors::new();
        assert!(same.set(face, Some(base.color)));
        assert_eq!(render_colored(&mesh, &same, &camera, &base).unwrap(), plain);
        // A colour of its own: where the face is, the picture all in that
        // colour; elsewhere the plain one.
        let red = [200, 40, 30];
        let mut colors = FaceColors::new();
        assert!(colors.set(face, Some(red)));
        let colored = render_colored(&mesh, &colors, &camera, &base).unwrap();
        let all_red = render(&mesh, &camera, &RenderParams { color: red, ..base }).unwrap();
        assert_eq!(colored.ids, plain.ids);
        let mut seen = 0;
        for (i, &id) in plain.ids.iter().enumerate() {
            let at = |frame: &Frame| frame.rgb[3 * i..3 * i + 3].to_vec();
            if id == face {
                assert_eq!(at(&colored), at(&all_red), "pixel {i}");
                seen += 1;
            } else {
                assert_eq!(at(&colored), at(&plain), "pixel {i}");
            }
        }
        assert!(seen > 50, "{seen}");
        // A highlight wins over the face's own colour.
        let (highlight, _) = Highlight::new([255, 140, 0], [face]);
        let lit = RenderParams { highlight, ..base };
        assert_eq!(
            render_colored(&mesh, &colors, &camera, &lit).unwrap(),
            render(&mesh, &camera, &lit).unwrap()
        );
    }

    /// The plain shading, written out without the finish's factors
    /// (2026-10-22, the fill light added 2026-10-24). It is the formula of
    /// `shade` itself, so it pins only that a plain finish adds no factor;
    /// the lighting's numbers are pinned by
    /// `a_grey_block_in_the_iso_view_reads_on_every_face`.
    fn shade_before_finishes(
        base: [u8; 3],
        weights: (f64, f64, f64),
        normal: Vec3,
        to_viewer: Vec3,
        view: &View,
    ) -> [u8; 3] {
        let n = if normal.dot(to_viewer) < 0.0 {
            -normal
        } else {
            normal
        };
        let light = view.light.unwrap_or(to_viewer);
        let diffuse = n.dot(light).max(0.0);
        let fill = view.fill.map_or(0.0, |f| n.dot(f).max(0.0));
        let specular = (light + to_viewer)
            .normalized()
            .map_or(0.0, |half| n.dot(half).max(0.0).ppowi(SHADE_SHININESS));
        let intensity = weights.0 + (weights.1 * diffuse + weights.2 * fill);
        base.map(|c| {
            (f64::from(c) * intensity + 255.0 * SHADE_SPECULAR * specular)
                .round()
                .clamp(0.0, 255.0) as u8
        })
    }

    #[test]
    fn the_plain_finish_adds_no_factor_to_the_shading() {
        let (_, camera) = teapot_mesh_and_camera(160, 120, true);
        let view = View::new(&camera, 160, 120).unwrap();
        let to_viewer = Vec3::new(-0.35, 1.0, -0.45).normalized().unwrap();
        let half = (view.light.unwrap() + to_viewer).normalized().unwrap();
        let plain = (SHADE_AMBIENT, SHADE_DIFFUSE, SHADE_FILL);
        let mut compared = 0;
        // Normals all round the sphere, and finely about the highlight.
        for i in 0..=200 {
            for j in 0..=200 {
                let (theta, phi) = (
                    std::f64::consts::PI * f64::from(i) / 200.0,
                    std::f64::consts::TAU * f64::from(j) / 200.0,
                );
                let n = Vec3::new(
                    theta.psin() * phi.pcos(),
                    theta.psin() * phi.psin(),
                    theta.pcos(),
                );
                let near = (half + n * 0.05).normalized().unwrap();
                for normal in [n, near] {
                    for (base, weights) in [
                        ([198, 204, 214], plain),
                        ([200, 40, 30], plain),
                        (
                            [255, 140, 0],
                            (
                                SHADE_HIGHLIGHT_AMBIENT,
                                SHADE_HIGHLIGHT_DIFFUSE,
                                SHADE_HIGHLIGHT_FILL,
                            ),
                        ),
                        ([0, 1, 255], plain),
                    ] {
                        assert_eq!(
                            shade(base, weights, Finish::default(), normal, to_viewer, &view),
                            shade_before_finishes(base, weights, normal, to_viewer, &view),
                            "{normal:?}"
                        );
                        compared += 1;
                    }
                }
            }
        }
        assert_eq!(compared, 201 * 201 * 2 * 4);
    }

    /// The studio lighting (2026-10-24): a grey block seen in the iso view
    /// (from +x, −y, +z, as `view.render` shows it) has three faces of
    /// three clearly different tones, and the darkest, turned from the
    /// key light, is not near black: its relative luminance is 30 to 50 %
    /// of the brightest's (with one light it was 8 %). A face turned from
    /// both lights gets the ambient term alone, a byte times 9/25.
    ///
    /// The pixels are pinned to closed forms worked out apart from the
    /// code (a script with the camera's axes, the two directions and the
    /// weights): intensities top 0.8832, front 0.7311 (no fill: the fill
    /// light is behind it), right 0.5733 (key 0.0707, fill 0.9338), no
    /// highlight on any of the three; times the grey, rounded.
    #[test]
    fn a_grey_block_in_the_iso_view_reads_on_every_face() {
        let camera = Camera {
            eye: Vec3::new(10.0, -10.0, 8.0),
            target: Vec3::new(0.0, 0.0, 0.0),
            up: Vec3::Z,
            projection: Projection::Orthographic { half_height: 5.0 },
        };
        let view = View::new(&camera, 160, 120).unwrap();
        let to_viewer = view.toward_viewer(Vec3::new(0.0, 0.0, 0.0));
        let grey = [221, 217, 210];
        let plain = (SHADE_AMBIENT, SHADE_DIFFUSE, SHADE_FILL);
        let tone = |normal: Vec3| {
            let rgb = shade(grey, plain, Finish::default(), normal, to_viewer, &view);
            relative_luminance(rgb)
        };
        let pixel = |normal: Vec3| shade(grey, plain, Finish::default(), normal, to_viewer, &view);
        assert_eq!(pixel(Vec3::Z), [195, 192, 185]);
        assert_eq!(pixel(-Vec3::Y), [162, 159, 154]);
        assert_eq!(pixel(Vec3::X), [127, 124, 120]);
        let top = tone(Vec3::Z);
        let front = tone(-Vec3::Y);
        let right = tone(Vec3::X);
        assert!(top > front && front > right, "{top} {front} {right}");
        let share = right / top;
        assert!((0.3..=0.5).contains(&share), "{share}");
        assert!(
            front / top < 0.85 && right / front < 0.85,
            "{top} {front} {right}"
        );
        // Turned from both lights: the ambient term alone.
        let away = (view.right * -0.4 - view.up - view.forward * 0.1)
            .normalized()
            .unwrap();
        assert!(away.dot(view.light.unwrap()) < 0.0 && away.dot(view.fill.unwrap()) < 0.0);
        let rgb = shade(grey, plain, Finish::default(), away, to_viewer, &view);
        assert_eq!(rgb, [80, 78, 76]);
    }

    /// The relative luminance of an sRGB colour (WCAG).
    fn relative_luminance(rgb: [u8; 3]) -> f64 {
        let [r, g, b] = rgb.map(|c| {
            let c = f64::from(c) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).ppowf(2.4)
            }
        });
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }

    #[test]
    fn a_finish_turns_roughness_into_a_highlight_exponent() {
        assert!(Finish::default().is_plain());
        assert_eq!(Finish::default().shininess(), None);
        let rough = |r: f64| Finish::from_factors(None, Some(r)).shininess();
        assert_eq!(rough(1.0), Some(1));
        assert_eq!(rough(0.0), Some(MAX_SHININESS));
        // About 0.45 is the plain exponent.
        assert_eq!(rough(114.0 / 255.0), Some(48));
        // Rougher is a lower exponent, all the way.
        let exponents: Vec<u32> = (0..=255_u8)
            .map(|r| {
                Finish {
                    metallic: 0,
                    roughness: Some(r),
                }
                .shininess()
                .unwrap()
            })
            .collect();
        assert!(exponents.windows(2).all(|w| w[0] >= w[1]), "{exponents:?}");
        // Out of range is clamped, not a number is not given.
        assert_eq!(Finish::from_factors(Some(2.0), Some(-1.0)).metallic, 255);
        assert_eq!(
            Finish::from_factors(Some(f64::NAN), Some(f64::INFINITY)),
            Finish::default()
        );
        let mut colors = FaceColors::new();
        let finish = Finish::from_factors(Some(1.0), Some(0.25));
        assert!(colors.set_look(2, Some(([9, 8, 7], finish))));
        assert_eq!(colors.look(2), Some(([9, 8, 7], finish)));
        assert_eq!(colors.get(2), Some([9, 8, 7]));
        assert_eq!(colors.finish(2), finish);
        assert_eq!(colors.finish(1), Finish::default());
    }

    #[test]
    fn metallic_and_roughness_change_only_their_face_and_a_plain_finish_nothing() {
        let (width, height) = (160, 120);
        let (mesh, camera) = teapot_mesh_and_camera(width, height, true);
        let base = params(width, height, 1);
        let plain = render(&mesh, &camera, &base).unwrap();
        let face = plain.ids[(60 * width + 80) as usize];
        assert_ne!(face, NO_FACE);
        let red = [200, 40, 30];
        let look = |finish: Finish| {
            let mut colors = FaceColors::new();
            assert!(colors.set_look(face, Some((red, finish))));
            render_colored(&mesh, &colors, &camera, &base).unwrap()
        };
        let dielectric = look(Finish::default());
        let mut colors = FaceColors::new();
        colors.set(face, Some(red));
        assert_eq!(
            dielectric,
            render_colored(&mesh, &colors, &camera, &base).unwrap()
        );
        // Given the plain exponent, the roughness changes nothing either.
        assert_eq!(
            look(Finish {
                metallic: 0,
                roughness: Some(114)
            }),
            dielectric
        );
        let metal = look(Finish::from_factors(Some(1.0), Some(0.3)));
        let rough = look(Finish::from_factors(None, Some(1.0)));
        let (mut metal_changed, mut rough_changed) = (0, 0);
        // A metal's highlight is in its own colour, not white: never more
        // blue than the red dielectric's.
        for (i, &id) in plain.ids.iter().enumerate() {
            let at = |frame: &Frame| -> [u8; 3] {
                [frame.rgb[3 * i], frame.rgb[3 * i + 1], frame.rgb[3 * i + 2]]
            };
            if id == face {
                metal_changed += usize::from(at(&metal) != at(&dielectric));
                rough_changed += usize::from(at(&rough) != at(&dielectric));
                assert!(at(&metal)[2] <= at(&dielectric)[2], "pixel {i}");
            } else {
                assert_eq!(at(&metal), at(&plain), "pixel {i}");
                assert_eq!(at(&rough), at(&plain), "pixel {i}");
            }
        }
        assert!(metal_changed > 50, "{metal_changed}");
        assert!(rough_changed > 50, "{rough_changed}");
        assert_eq!(metal.ids, plain.ids);
    }

    #[test]
    fn face_colours_hold_what_they_are_given_within_their_cap() {
        let mut colors = FaceColors::new();
        assert!(colors.is_empty());
        assert!(colors.set(3, None));
        assert!(colors.is_empty());
        assert!(colors.set(5, Some([1, 2, 3])));
        assert_eq!(colors.get(5), Some([1, 2, 3]));
        assert_eq!(colors.get(4), None);
        assert_eq!(colors.get(u32::MAX), None);
        assert!(!colors.set(u32::MAX, Some([0; 3])));
        assert!(!colors.set(MAX_FACE_COLORS as u32, Some([0; 3])));
        assert_eq!(colors.iter().collect::<Vec<_>>(), vec![(5, [1, 2, 3])]);
        assert!(colors.set(5, None));
        assert!(colors.is_empty());
    }
}
