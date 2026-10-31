//! Layer 3's light through transmissive faces, in screen space (the
//! approximation of Filament's *Transmission and refraction*, documented
//! in docs/design/rendering.md, *Layer 3: materials*).
//!
//! A sample whose surface lets light through ([`Shaded::transmit`] not
//! zero) adds the light of the picture drawn without the transmissive
//! faces (`behind`) where its ray leaves the body:
//!
//! - thin-walled, or with no back face of a transmissive face behind it
//!   (an open sheet): straight behind, the same sample, unabsorbed;
//! - solid: the view ray refracted into the body at the front surface
//!   ([`layers::refract`]), along it the path a slab with parallel faces
//!   would give between the front surface and the nearest back face under
//!   the sample ([`layers::slab_path`]), its end projected to the sample
//!   it lies in, absorbed by `exp(−σ L)`. Off the picture, the background
//!   along the view ray from that end.
//!
//! The plain background (`Look::plain_background`), which the picture
//! shows as it is, enters as the light the tone curve shows as its colour
//! (`FrameLight::untoned`), off the picture too: clear glass on white
//! stays white instead of being toned a second time.
//!
//! For a slab of parallel faces seen in an orthographic picture this is
//! exact (the ray leaves parallel to the view, so it sees what lies
//! behind its exit point); elsewhere it is the usual screen-space
//! approximation: one transmissive layer (glass behind glass is not
//! seen), the back face assumed parallel to the front, only what is on
//! the picture refracted.

use super::{FrameLight, PLAIN_BACKDROP, Shaded, Surface, layers};
use crate::{FaceColors, NO_FACE, View};
use craft3d_math::Portable;
use craft3d_math::Vec3;

/// What [`pass_through`] needs of the frame.
pub(crate) struct Scene<'a> {
    pub view: &'a View,
    pub colors: &'a FaceColors,
    pub frame: &'a FrameLight<'a>,
    pub size: (usize, usize),
    /// The margin of doubt in `f32` epsilons ([`super::polish::TRACE_DOUBT`]).
    pub margin: f64,
    /// The plain background's colour (linear), when the picture has one.
    pub plain: Option<[f64; 3]>,
}

/// Adds the light through the transmissive samples of `shaded` (their
/// surfaces `surfaces`) from `behind` (the picture without the
/// transmissive faces, whose surfaces are `behind_surfaces`), with
/// `backs` the nearest back faces of the transmissive faces. Per sample,
/// true where the exit fell within the margin of a sample's border with
/// another face across it, which the GPU's `f32` may decide otherwise.
pub(crate) fn pass_through(
    scene: &Scene<'_>,
    shaded: &mut [Shaded],
    surfaces: &[Surface],
    behind: &[Shaded],
    behind_surfaces: &[Surface],
    backs: &[Surface],
) -> Vec<bool> {
    let (w, h) = scene.size;
    let view = scene.view;
    let mut doubt = vec![false; shaded.len()];
    // The light a sample of the picture behind shows through the glass:
    // the plain background (the colour over the exposure, shown as it is)
    // as the light the tone curve shows as that colour, so that it is not
    // toned twice.
    let seen_of = |s: &Shaded| {
        if s.roughness == PLAIN_BACKDROP {
            scene
                .frame
                .untoned(s.light.map(|v| v * scene.frame.exposure))
        } else {
            s.light
        }
    };
    let near = scene.margin * f64::from(f32::EPSILON) * (w.max(h) as f64);
    let face_at = |x: i64, y: i64| -> Option<u32> {
        if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
            return None;
        }
        behind_surfaces
            .get(y as usize * w + x as usize)
            .map(|s| s.face)
    };
    for (i, out) in shaded.iter_mut().enumerate() {
        if out.transmit.iter().all(|&t| t == 0.0) {
            continue;
        }
        let (Some(surface), Some(back)) = (surfaces.get(i), backs.get(i)) else {
            continue;
        };
        let Some(optics) = scene.colors.optics(surface.face) else {
            continue;
        };
        let (x, y) = (i % w, i / w);
        let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
        let rel = view.point_at(px, py, surface.dist);
        let p = view.eye + rel;
        let to_viewer = view.toward_viewer(p);
        let [nx, ny, nz] = surface.normal.map(f64::from);
        let n = Vec3::new(nx, ny, nz).normalized().unwrap_or(to_viewer);
        let n = if n.dot(to_viewer) < 0.0 { -n } else { n };
        let (_, ray) = view.ray(px, py);
        let straight = || behind.get(i).map_or([0.0; 3], seen_of);
        let solid = !optics.thin_walled
            && back.face != NO_FACE
            && back.dist.is_finite()
            && back.key > surface.key;
        let refracted = if solid {
            layers::refract(-to_viewer, n, f64::from(optics.ior))
        } else {
            None
        };
        let (seen, absorbed) = match refracted {
            None => (straight(), [1.0; 3]),
            Some(r) => {
                let across = (view.point_at(px, py, back.dist) - rel).length();
                let nv = n.dot(to_viewer).clamp(1e-4, 1.0);
                let path = layers::slab_path(across, nv, n, r);
                let exit = p + r * path;
                let absorbed = optics
                    .absorption
                    .map(|sigma| (-f64::from(sigma) * path).pexp());
                let at = view
                    .project(exit)
                    .map(|at| (at.sx, at.sy))
                    .filter(|(sx, sy)| sx.is_finite() && sy.is_finite());
                let inside =
                    at.filter(|&(sx, sy)| sx >= 0.0 && sy >= 0.0 && sx < w as f64 && sy < h as f64);
                match inside {
                    None => {
                        if let (Some(d), Some((sx, sy))) = (doubt.get_mut(i), at) {
                            // Just off the picture's border.
                            *d = sx.abs() < near
                                || sy.abs() < near
                                || (sx - w as f64).abs() < near
                                || (sy - h as f64).abs() < near;
                        }
                        let seen = match scene.plain {
                            Some(plain) => {
                                let factor = scene.frame.ground_factor(exit, ray);
                                scene.frame.untoned(plain.map(|v| v * factor))
                            }
                            None => scene.frame.background(exit, ray),
                        };
                        (seen, absorbed)
                    }
                    Some((sx, sy)) => {
                        let (fx, fy) = (sx.floor(), sy.floor());
                        let (ix, iy) = (fx as i64, fy as i64);
                        let here = face_at(ix, iy);
                        let across = |dx: i64, dy: i64| face_at(ix + dx, iy + dy) != here;
                        let unsure = (sx - fx < near && across(-1, 0))
                            || (fx + 1.0 - sx < near && across(1, 0))
                            || (sy - fy < near && across(0, -1))
                            || (fy + 1.0 - sy < near && across(0, 1));
                        if let Some(d) = doubt.get_mut(i) {
                            *d = unsure;
                        }
                        let seen = behind
                            .get(iy as usize * w + ix as usize)
                            .map_or([0.0; 3], seen_of);
                        (seen, absorbed)
                    }
                }
            }
        };
        for ((o, t), (s, a)) in out
            .light
            .iter_mut()
            .zip(out.transmit)
            .zip(seen.iter().zip(absorbed))
        {
            *o += t * a * s;
        }
    }
    doubt
}
