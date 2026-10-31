//! The shadow of the key light: a shadow map (Williams, *Casting Curved
//! Shadows on Curved Surfaces*, 1978) seen along the light over the
//! sphere around the mesh, soft by percentage-closer soft shadows
//! (Fernando, *Percentage-Closer Soft Shadows*, NVIDIA 2005): a blocker
//! search gives the mean depth of what is between the point and the
//! light, the light's angular size turns the gap into a penumbra width,
//! and percentage-closer filtering (Reeves, Salesin and Cook 1987) over
//! that width gives the share of the light that reaches the point.
//!
//! Both renderers read their own map with the same arithmetic here; the
//! GPU draws its map with its rasteriser, which may cover a texel along
//! a triangle's border differently ([`super`], *Tolerance*).

use crate::Cut;
use craft3d_math::Portable;
use craft3d_math::{Bbox3, Vec3};
use craft3d_tess::Mesh;

/// Texels per side of the shadow map.
pub const SHADOW_SIZE: usize = 1024;
/// Taps per side of the blocker search and of the filter.
pub const BLOCKER_TAPS: usize = 4;
pub const FILTER_TAPS: usize = 5;
/// Most penumbra, in texels, either side.
pub const MAX_PENUMBRA: f64 = 24.0;
/// The depth bias and the normal offset, in texels of the map.
pub const DEPTH_BIAS: f64 = 2.0;
pub const NORMAL_OFFSET: f64 = 1.5;

/// The light's view: a square over the sphere around the mesh.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightView {
    pub center: Vec3,
    pub radius: f64,
    /// Across the map (right, down) and towards the light.
    pub u: Vec3,
    pub v: Vec3,
    pub w: Vec3,
}

impl LightView {
    /// The view of a light towards `toward` (unit) over `bounds`, or
    /// `None` for an empty box.
    pub fn new(bounds: &Bbox3, toward: Vec3) -> Option<LightView> {
        let center = bounds.center()?;
        let radius = (bounds.diagonal() * 0.5).max(f64::MIN_POSITIVE);
        if !(radius.is_finite() && center.is_finite()) {
            return None;
        }
        let w = toward.normalized()?;
        let up = if w.z.abs() < 0.999 { Vec3::Z } else { Vec3::X };
        let u = up.cross(w).normalized()?;
        let v = w.cross(u);
        Some(LightView {
            center,
            radius,
            u,
            v,
            w,
        })
    }

    /// A point in texels across the map and its depth from the light (0
    /// at the plane through the far side of the sphere facing the light,
    /// 2·radius at the other).
    pub fn project(&self, p: Vec3) -> (f64, f64, f64) {
        let rel = p - self.center;
        let size = SHADOW_SIZE as f64;
        (
            (rel.dot(self.u) / self.radius * 0.5 + 0.5) * size,
            (0.5 - rel.dot(self.v) / self.radius * 0.5) * size,
            self.radius - rel.dot(self.w),
        )
    }

    /// One texel's side in model units.
    pub fn texel(&self) -> f64 {
        2.0 * self.radius / SHADOW_SIZE as f64
    }
}

/// The nearest depth from the light per texel (row-major, infinity where
/// nothing is), as `f32`.
#[derive(Clone, Debug, PartialEq)]
pub struct ShadowMap {
    pub view: LightView,
    pub depth: Vec<f32>,
}

impl ShadowMap {
    /// Draws the mesh into the map: a texel holds the nearest triangle
    /// whose inside holds its centre (the top-left rule on a border).
    pub fn draw(mesh: &Mesh, view: LightView) -> ShadowMap {
        Self::draw_cut(mesh, view, None)
    }

    /// [`Self::draw`] of what a section leaves: a texel of a triangle is
    /// left out where the distance from the plane, interpolated from the
    /// corners (linearly: the light's view is orthographic), says
    /// [`Cut::clips`]; the cap casts no shadow (a point the light reaches
    /// through the cap's plane from the kept half meets a face of the
    /// kept half first).
    pub fn draw_cut(mesh: &Mesh, view: LightView, cut: Option<&Cut>) -> ShadowMap {
        let n = SHADOW_SIZE;
        let mut depth = vec![f32::INFINITY; n * n];
        let projected: Vec<(f64, f64, f64)> =
            mesh.positions.iter().map(|&p| view.project(p)).collect();
        for tri in &mesh.triangles {
            let place = |k: u32| mesh.positions.get(k as usize).copied();
            // The corners' distances from the plane and which way the
            // triangle faces it.
            let cut_triangle = match (cut, place(tri[0]), place(tri[1]), place(tri[2])) {
                (Some(cut), Some(pa), Some(pb), Some(pc)) => {
                    let facing = (pb - pa)
                        .cross(pc - pa)
                        .normalized()
                        .map_or(0.0, |f| f.dot(cut.normal));
                    Some((cut, [cut.side(pa), cut.side(pb), cut.side(pc)], facing))
                }
                _ => None,
            };
            let corner = |k: u32| projected.get(k as usize).copied();
            let (Some(a), Some(b), Some(c)) = (corner(tri[0]), corner(tri[1]), corner(tri[2]))
            else {
                continue;
            };
            let area = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
            if !area.is_finite() || area == 0.0 {
                continue;
            }
            let lo_x = a.0.min(b.0).min(c.0);
            let hi_x = a.0.max(b.0).max(c.0);
            let lo_y = a.1.min(b.1).min(c.1);
            let hi_y = a.1.max(b.1).max(c.1);
            let range = |lo: f64, hi: f64| -> Option<(usize, usize)> {
                let first = (lo - 0.5).ceil().max(0.0);
                let last = (hi - 0.5).floor().min(n as f64 - 1.0);
                // Within 0..n before the casts.
                (first <= last).then_some((first as usize, last as usize))
            };
            let (Some((x0, x1)), Some((y0, y1))) = (range(lo_x, hi_x), range(lo_y, hi_y)) else {
                continue;
            };
            // Oriented so that the inside is positive (the sides follow
            // their corners).
            let (a, b, c, area, cut_triangle) = if area < 0.0 {
                (
                    a,
                    c,
                    b,
                    -area,
                    cut_triangle.map(|(cut, [sa, sb, sc], f)| (cut, [sa, sc, sb], f)),
                )
            } else {
                (a, b, c, area, cut_triangle)
            };
            let edge = |p: (f64, f64, f64), q: (f64, f64, f64), x: f64, y: f64| {
                let e = (q.0 - p.0) * (y - p.1) - (q.1 - p.1) * (x - p.0);
                // Top-left rule in a y-down frame with the inside
                // positive: a border at exactly 0 counts for a top edge
                // (horizontal, the inside below it) or a left one.
                let dy = q.1 - p.1;
                let dx = q.0 - p.0;
                let top_left = (dy == 0.0 && dx < 0.0) || dy > 0.0;
                e > 0.0 || (e == 0.0 && top_left)
            };
            for y in y0..=y1 {
                let py = y as f64 + 0.5;
                for x in x0..=x1 {
                    let px = x as f64 + 0.5;
                    if !(edge(b, c, px, py) && edge(c, a, px, py) && edge(a, b, px, py)) {
                        continue;
                    }
                    let wa = ((c.0 - b.0) * (py - b.1) - (c.1 - b.1) * (px - b.0)) / area;
                    let wb = ((a.0 - c.0) * (py - c.1) - (a.1 - c.1) * (px - c.0)) / area;
                    let wc = 1.0 - wa - wb;
                    if let Some((cut, [sa, sb, sc], facing)) = cut_triangle
                        && cut.clips(wa * sa + wb * sb + wc * sc, facing)
                    {
                        continue;
                    }
                    let d = (wa * a.2 + wb * b.2 + wc * c.2) as f32;
                    if let Some(slot) = depth.get_mut(y * n + x)
                        && d < *slot
                    {
                        *slot = d;
                    }
                }
            }
        }
        ShadowMap { view, depth }
    }

    fn at(&self, x: f64, y: f64) -> f64 {
        let n = SHADOW_SIZE as f64;
        if !(x >= 0.0 && y >= 0.0 && x < n && y < n) {
            return f64::INFINITY;
        }
        // Within 0..n before the casts.
        let i = (y.floor() as usize) * SHADOW_SIZE + x.floor() as usize;
        self.depth.get(i).map_or(f64::INFINITY, |&d| f64::from(d))
    }

    /// The share of the key light that reaches the point `p` with the
    /// unit normal `n` (towards the viewer), for a light of angular
    /// radius `radius`.
    pub fn visibility(&self, p: Vec3, n: Vec3, radius: f64) -> f64 {
        visibility(|x, y| self.at(x, y), &self.view, p, n, radius)
    }
}

/// [`ShadowMap::visibility`] over any map reader: the arithmetic both
/// renderers share.
pub fn visibility(
    at: impl Fn(f64, f64) -> f64,
    view: &LightView,
    p: Vec3,
    n: Vec3,
    radius: f64,
) -> f64 {
    let texel = view.texel();
    let (x, y, d) = view.project(p + n * (NORMAL_OFFSET * texel));
    let bias = DEPTH_BIAS * texel;
    let slope = radius.ptan().max(0.0);
    // The blocker search over the widest penumbra.
    let search = (slope * 2.0 * view.radius / texel).clamp(1.0, MAX_PENUMBRA);
    let offset = |k: usize, taps: usize| (k as f64 + 0.5) / taps as f64 * 2.0 - 1.0;
    let (mut sum, mut count) = (0.0, 0_u32);
    for j in 0..BLOCKER_TAPS {
        for i in 0..BLOCKER_TAPS {
            let s = at(
                x + offset(i, BLOCKER_TAPS) * search,
                y + offset(j, BLOCKER_TAPS) * search,
            );
            if s < d - bias {
                sum += s;
                count += 1;
            }
        }
    }
    if count == 0 {
        return 1.0;
    }
    let blocker = sum / f64::from(count);
    let width = ((d - blocker) * slope / texel).clamp(1.0, MAX_PENUMBRA);
    let mut lit = 0_u32;
    for j in 0..FILTER_TAPS {
        for i in 0..FILTER_TAPS {
            let s = at(
                x + offset(i, FILTER_TAPS) * width,
                y + offset(j, FILTER_TAPS) * width,
            );
            if s >= d - bias {
                lit += 1;
            }
        }
    }
    f64::from(lit) / (FILTER_TAPS * FILTER_TAPS) as f64
}
