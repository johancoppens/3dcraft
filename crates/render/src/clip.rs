//! Near-plane clipping for the CPU renderer's perspective pictures
//! (docs/design/rendering.md › *Near-plane clipping*).
//!
//! A triangle with a corner closer to the eye than the near plane is cut
//! along that plane (Sutherland–Hodgman against `dist ≥ near`, `dist` the
//! distance along the view direction, which is the clip-space `w`), so
//! that what lies in front of the plane is drawn as the GPU draws it,
//! instead of the whole triangle being left out. The near plane is the
//! GPU's, from the one function both renderers call ([`depth_range`] of
//! [`depth_bounds`]): the nearest corner of the drawn triangles' box (with
//! the cap's margin when a section draws one) less a margin, and at least a
//! millionth of the farthest. A camera outside that box, further than the
//! margin, clips nothing; one within the margin or inside clips at the same
//! distance on both renderers.
//!
//! A new corner is the point `t` of the way along a triangle's edge, and
//! every attribute there is the same interpolation of the edge's ends:
//! along a straight edge in space the perspective-correct interpolation of
//! an attribute is the linear one, so the normal, the point and the value
//! of the analysis are exact. The ends are taken in the order of their
//! positions' bits, so the two triangles that share an edge get the very
//! same point, also where a body's faces meet along a sharp edge with
//! vertices of their own (the same positions under other indices, walked
//! the other way): the picture stays watertight, and the see-through rule
//! that knows a shared edge by its points' bits still knows it. The
//! clipped mesh is built in the order of the triangles, with a `BTreeMap`
//! for the shared edges: the same mesh gives the same clipped mesh, bit
//! for bit.

use crate::{Projection, RenderError, View};
use craft3d_math::{Bbox3, Vec3};
use craft3d_tess::Mesh;
use std::borrow::Cow;
use std::collections::BTreeMap;

/// The box the depth range is fitted to: the box of the drawn triangles
/// (`section_bounds`: those with every corner finite) and, when a section
/// draws its cap, that box grown by the cap's margin (the cap may reach
/// into it). Both renderers call this.
pub fn depth_bounds(drawn: &Bbox3, cap: bool) -> Bbox3 {
    let mut bounds = *drawn;
    if cap && let Some((lo, hi)) = drawn.corners() {
        let margin = Vec3::new(1.0, 1.0, 1.0) * (drawn.diagonal() * crate::SECTION_MARGIN);
        if margin.is_finite() {
            bounds.add(lo - margin);
            bounds.add(hi + margin);
        }
    }
    bounds
}

/// Near and far distance along the view direction `forward` from `eye`
/// that enclose `bounds` with a margin: the GPU's depth buffer is fitted
/// to them, and in perspective both renderers clip at the near one. With
/// the eye inside the box the near distance is a millionth of the far one.
/// For an empty box, or one wholly behind the eye, any valid range
/// (`0.5..1`): nothing there is drawn.
pub fn depth_range(eye: Vec3, forward: Vec3, bounds: &Bbox3, perspective: bool) -> (f64, f64) {
    const EMPTY: (f64, f64) = (0.5, 1.0);
    let Some((lo, hi)) = bounds.corners() else {
        return EMPTY;
    };
    let (mut nearest, mut farthest) = (f64::INFINITY, f64::NEG_INFINITY);
    for x in [lo.x, hi.x] {
        for y in [lo.y, hi.y] {
            for z in [lo.z, hi.z] {
                let dist = (Vec3::new(x, y, z) - eye).dot(forward);
                nearest = nearest.min(dist);
                farthest = farthest.max(dist);
            }
        }
    }
    if !(nearest.is_finite() && farthest.is_finite()) {
        return EMPTY;
    }
    // The margin covers the rounding of positions to f32 (relative error
    // 2^-24 ≈ 6e-8 of the mesh size) many times over, and keeps a flat mesh
    // seen head-on from having an empty range.
    let reach = nearest.abs().max(farthest.abs());
    let pad = (1e-3 * (farthest - nearest) + 1e-5 * reach).max(1e-30);
    if perspective {
        let far = farthest + pad;
        if far <= 0.0 {
            // Everything is behind the eye and is clipped anyway.
            return EMPTY;
        }
        // A camera inside the model: keep the ratio far / near bounded.
        ((nearest - pad).max(far * 1e-6), far)
    } else {
        (nearest - pad, farthest + pad)
    }
}

/// The distance along the view direction below which a perspective picture
/// of what lies in `bounds` ([`depth_bounds`]) clips; `None` for an
/// orthographic one.
pub(crate) fn near_distance(view: &View, bounds: &Bbox3) -> Option<f64> {
    if !matches!(view.projection, Projection::Perspective { .. }) {
        return None;
    }
    Some(depth_range(view.eye, view.forward, bounds, true).0)
}

/// A corner of a clipped triangle: the vertex `lo` itself (`hi == lo`,
/// `t == 0`) or the point `t` of the way from vertex `lo` to vertex `hi`,
/// the ends in the order of their positions ([`ends`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Corner {
    pub lo: u32,
    pub hi: u32,
    pub t: f64,
}

/// What clipping does to one triangle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Clipped {
    /// Nothing in front of the near plane, or a corner that is not a
    /// number (the renderer leaves such a triangle out as before).
    Whole,
    /// All of it behind the near plane.
    Gone,
    /// The part in front: a polygon of three or four corners, in the
    /// triangle's winding.
    Part([Corner; 4], usize),
}

/// The order of an edge's ends: by their positions' bits (x, then y, then
/// z), then by index, so that the edge is walked the same way by every
/// triangle that has it, whatever indices its points have there.
fn first_is_lower(a: (u32, Vec3), b: (u32, Vec3)) -> bool {
    let bits = |p: Vec3| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()];
    (bits(a.1), a.0) <= (bits(b.1), b.0)
}

/// Clips the triangle with vertices `ids` at `points`, at distances
/// `dists` along the view, against `dist ≥ near`.
pub(crate) fn clip_triangle(
    ids: [u32; 3],
    points: [Vec3; 3],
    dists: [f64; 3],
    near: f64,
) -> Clipped {
    if dists.iter().any(|d| d.is_nan()) || near.is_nan() {
        return Clipped::Whole;
    }
    let inside = dists.map(|d| d >= near);
    match inside.iter().filter(|&&i| i).count() {
        3 => return Clipped::Whole,
        0 => return Clipped::Gone,
        _ => {}
    }
    let mut out = [Corner {
        lo: 0,
        hi: 0,
        t: 0.0,
    }; 4];
    let mut n = 0;
    for k in 0..3 {
        let next = (k + 1) % 3;
        let (Some(&a), Some(&b), Some(&pa), Some(&pb), Some(&da), Some(&db), Some(&ia), Some(&ib)) = (
            ids.get(k),
            ids.get(next),
            points.get(k),
            points.get(next),
            dists.get(k),
            dists.get(next),
            inside.get(k),
            inside.get(next),
        ) else {
            return Clipped::Gone;
        };
        if ia && let Some(slot) = out.get_mut(n) {
            *slot = Corner {
                lo: a,
                hi: a,
                t: 0.0,
            };
            n += 1;
        }
        if ia != ib {
            // The ends in the order of their positions: every triangle of
            // the edge computes the same point.
            let ((lo, dlo), (hi, dhi)) = if first_is_lower((a, pa), (b, pb)) {
                ((a, da), (b, db))
            } else {
                ((b, db), (a, da))
            };
            let span = dhi - dlo;
            let t = if span != 0.0 && span.is_finite() {
                ((near - dlo) / span).clamp(0.0, 1.0)
            } else {
                0.5
            };
            if let Some(slot) = out.get_mut(n) {
                *slot = Corner { lo, hi, t };
                n += 1;
            }
        }
    }
    if n < 3 {
        return Clipped::Gone;
    }
    Clipped::Part(out, n)
}

/// The triangles of the part of the triangle at `corners` in front of
/// `near` (along `forward` from `eye`), in its winding: the triangle
/// itself, none, or one or two; with their count. The same points as
/// [`clip_mesh`] makes, without a mesh: for looking up one position.
pub(crate) fn clip_corners(
    corners: [Vec3; 3],
    eye: Vec3,
    forward: Vec3,
    near: f64,
) -> ([[Vec3; 3]; 2], usize) {
    let dists = corners.map(|p| (p - eye).dot(forward));
    match clip_triangle([0, 1, 2], corners, dists, near) {
        Clipped::Whole => ([corners, corners], 1),
        Clipped::Gone => ([corners, corners], 0),
        Clipped::Part(cs, n) => {
            let at = |k: u32| corners.get(k as usize).copied().unwrap_or(Vec3::ZERO);
            let point = |c: &Corner| {
                let (p0, p1) = (at(c.lo), at(c.hi));
                if c.lo == c.hi {
                    p0
                } else {
                    p0 + (p1 - p0) * c.t
                }
            };
            let p: Vec<Vec3> = cs.iter().take(n).map(point).collect();
            let get = |k: usize| p.get(k).copied().unwrap_or(Vec3::ZERO);
            let mut out = [[get(0), get(1), get(2)]; 2];
            if n == 4
                && let Some(second) = out.get_mut(1)
            {
                *second = [get(0), get(2), get(3)];
            }
            (out, if n == 4 { 2 } else { 1 })
        }
    }
}

/// A mesh with every triangle clipped to the near plane (see the module
/// docs), the values per vertex of the analysis with it, and for each of
/// its triangles the one of the mesh it comes from.
pub(crate) struct ClippedMesh<'a> {
    pub mesh: Cow<'a, Mesh>,
    pub values: Cow<'a, [f32]>,
    /// `None` when nothing was clipped: the triangles are the mesh's own.
    origin: Option<Vec<usize>>,
}

impl ClippedMesh<'_> {
    /// The triangle of the original mesh triangle `t` comes from.
    pub fn origin(&self, t: usize) -> usize {
        match &self.origin {
            Some(o) => o.get(t).copied().unwrap_or(t),
            None => t,
        }
    }

    /// Whether nothing was clipped (the mesh is the one given, borrowed).
    #[cfg(test)]
    fn is_unchanged(&self) -> bool {
        self.origin.is_none() && matches!(self.mesh, Cow::Borrowed(_))
    }
}

/// `mesh` (and `values`, one per vertex or none) clipped to the near
/// plane of `view` for what lies in `bounds` ([`depth_bounds`]): the mesh
/// itself, borrowed, when no vertex is in front of it. A new vertex is
/// made at most once per edge (three per triangle at most), so the
/// clipped mesh is at most a small multiple of the mesh.
pub(crate) fn clip_mesh<'a>(
    mesh: &'a Mesh,
    values: &'a [f32],
    view: &View,
    bounds: &Bbox3,
) -> Result<ClippedMesh<'a>, RenderError> {
    let unchanged = || ClippedMesh {
        mesh: Cow::Borrowed(mesh),
        values: Cow::Borrowed(values),
        origin: None,
    };
    let Some(near) = near_distance(view, bounds) else {
        return Ok(unchanged());
    };
    let dists: Vec<f64> = mesh
        .positions
        .iter()
        .map(|&p| (p - view.eye).dot(view.forward))
        .collect();
    if !dists.iter().any(|d| *d < near) {
        return Ok(unchanged());
    }
    let with_values = values.len() == mesh.positions.len();
    let mut out = Mesh {
        positions: mesh.positions.clone(),
        normals: mesh.normals.clone(),
        triangles: Vec::with_capacity(mesh.triangles.len()),
        face_ids: Vec::with_capacity(mesh.face_ids.len()),
    };
    let mut new_values: Vec<f32> = values.to_vec();
    let mut origin = Vec::with_capacity(mesh.triangles.len());
    let mut made: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    for (t, (tri, &face)) in mesh.triangles.iter().zip(&mesh.face_ids).enumerate() {
        let dist = |k: u32| {
            dists
                .get(k as usize)
                .copied()
                .ok_or(RenderError::InvalidMesh { triangle: t })
        };
        let at = |v: &[Vec3], k: u32| {
            v.get(k as usize)
                .copied()
                .ok_or(RenderError::InvalidMesh { triangle: t })
        };
        let [ia, ib, ic] = *tri;
        let points = [
            at(&mesh.positions, ia)?,
            at(&mesh.positions, ib)?,
            at(&mesh.positions, ic)?,
        ];
        let (corners, n) =
            match clip_triangle(*tri, points, [dist(ia)?, dist(ib)?, dist(ic)?], near) {
                Clipped::Whole => {
                    out.triangles.push(*tri);
                    out.face_ids.push(face);
                    origin.push(t);
                    continue;
                }
                Clipped::Gone => continue,
                Clipped::Part(corners, n) => (corners, n),
            };
        let mut index = [0u32; 4];
        for (slot, c) in index.iter_mut().zip(corners.iter()).take(n) {
            if c.lo == c.hi {
                *slot = c.lo;
                continue;
            }
            if let Some(&k) = made.get(&(c.lo, c.hi)) {
                *slot = k;
                continue;
            }
            let (p0, p1) = (at(&mesh.positions, c.lo)?, at(&mesh.positions, c.hi)?);
            let (n0, n1) = (at(&mesh.normals, c.lo)?, at(&mesh.normals, c.hi)?);
            let k = u32::try_from(out.positions.len())
                .map_err(|_| RenderError::InvalidMesh { triangle: t })?;
            out.positions.push(p0 + (p1 - p0) * c.t);
            out.normals.push(n0 + (n1 - n0) * c.t);
            if with_values {
                let v = |k: u32| {
                    new_values
                        .get(k as usize)
                        .map_or(f64::NAN, |&v| f64::from(v))
                };
                let (v0, v1) = (v(c.lo), v(c.hi));
                new_values.push((v0 + (v1 - v0) * c.t) as f32);
            }
            made.insert((c.lo, c.hi), k);
            *slot = k;
        }
        // A fan from the first corner keeps the winding.
        for j in 1..n.saturating_sub(1) {
            if let (Some(&a), Some(&b), Some(&c)) = (index.first(), index.get(j), index.get(j + 1))
            {
                out.triangles.push([a, b, c]);
                out.face_ids.push(face);
                origin.push(t);
            }
        }
    }
    Ok(ClippedMesh {
        mesh: Cow::Owned(out),
        values: if with_values {
            Cow::Owned(new_values)
        } else {
            Cow::Borrowed(values)
        },
        origin: Some(origin),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Camera, section_bounds};

    /// Points on the view direction +x from the origin: `dist` is `x`.
    fn along(d: [f64; 3]) -> [Vec3; 3] {
        [
            Vec3::new(d[0], 0.0, 0.0),
            Vec3::new(d[1], 1.0, 0.0),
            Vec3::new(d[2], 0.0, 1.0),
        ]
    }

    fn clip(ids: [u32; 3], d: [f64; 3], near: f64) -> Clipped {
        clip_triangle(ids, along(d), d, near)
    }

    #[test]
    fn a_triangle_is_whole_gone_or_cut() {
        assert_eq!(clip([0, 1, 2], [2.0, 3.0, 4.0], 1.0), Clipped::Whole);
        assert_eq!(clip([0, 1, 2], [0.0, -3.0, 0.5], 1.0), Clipped::Gone);
        assert_eq!(clip([0, 1, 2], [f64::NAN, 3.0, 4.0], 1.0), Clipped::Whole);
        // One corner in front: a triangle of it and two points on its edges.
        let Clipped::Part(c, n) = clip([0, 1, 2], [3.0, 0.0, 0.0], 1.0) else {
            panic!("not cut");
        };
        assert_eq!(n, 3);
        assert_eq!(
            c[0],
            Corner {
                lo: 0,
                hi: 0,
                t: 0.0
            }
        );
        // The ends by their positions' bits: (0, 1, 0) before (3, 0, 0).
        assert_eq!(
            c[1],
            Corner {
                lo: 1,
                hi: 0,
                t: 1.0 / 3.0
            }
        );
        // Two in front: a quadrilateral.
        let Clipped::Part(_, n) = clip([0, 1, 2], [3.0, 3.0, 0.0], 1.0) else {
            panic!("not cut");
        };
        assert_eq!(n, 4);
    }

    /// Two faces meeting along a sharp edge have vertices of their own at
    /// the same places, and walk the edge in opposite directions: the point
    /// where the near plane cuts it is the same, bit for bit, for both.
    #[test]
    fn a_sharp_edge_with_vertices_of_its_own_is_cut_at_one_point() {
        let (p, q) = (Vec3::new(3.1, 0.2, -0.7), Vec3::new(-0.4, 0.9, 0.3));
        let (r, s) = (Vec3::new(2.0, 1.7, 0.4), Vec3::new(1.5, -1.2, 0.8));
        let (eye, forward) = (Vec3::ZERO, Vec3::X);
        let near = 0.77;
        // Face one: p → q, indices 4 and 9; face two: q → p, indices 12
        // and 3 (the same places, other vertices).
        let one = clip_corners([p, q, r], eye, forward, near);
        let two = clip_corners([q, p, s], eye, forward, near);
        let on_edge = |(parts, n): ([[Vec3; 3]; 2], usize)| -> Vec<[u64; 3]> {
            parts
                .iter()
                .take(n)
                .flatten()
                .filter(|v| (v.x - near).abs() < 1e-12 && **v != r && **v != s)
                .filter(|v| {
                    // On the line p q.
                    let t = (v.x - p.x) / (q.x - p.x);
                    (p + (q - p) * t - **v).length() < 1e-12
                })
                .map(|v| [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()])
                .collect()
        };
        let (a, b) = (on_edge(one), on_edge(two));
        assert!(!a.is_empty() && !b.is_empty(), "{a:?} {b:?}");
        assert_eq!(a[0], b[0]);
        // And through the mesh: two triangles with their own vertices.
        let mesh = Mesh {
            positions: vec![p, q, r, q, p, s],
            normals: vec![Vec3::Z; 6],
            triangles: vec![[0, 1, 2], [3, 4, 5]],
            face_ids: vec![1, 2],
        };
        let camera = Camera {
            eye: Vec3::new(-0.2, 0.3, 0.1),
            target: Vec3::new(5.0, 0.3, 0.1),
            up: Vec3::Z,
            projection: Projection::Perspective { fov_y_deg: 60.0 },
        };
        let view = View::new(&camera, 64, 48).unwrap();
        let bounds = depth_bounds(&section_bounds(&mesh), false);
        let near = near_distance(&view, &bounds).unwrap();
        let clipped = clip_mesh(&mesh, &[], &view, &bounds).unwrap();
        assert!(!clipped.is_unchanged());
        let cut: Vec<[u64; 3]> = clipped
            .mesh
            .positions
            .iter()
            .skip(6)
            .filter(|v| ((**v - view.eye).dot(view.forward) - near).abs() < 1e-9)
            .map(|v| [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()])
            .collect();
        // The edge p q is cut once per face, at the same bits.
        let mut unique = cut.clone();
        unique.sort_unstable();
        unique.dedup();
        assert!(cut.len() > unique.len(), "{cut:?}");
    }

    fn quad() -> Mesh {
        Mesh {
            positions: vec![
                Vec3::new(-1.0, -1.0, 0.0),
                Vec3::new(1.0, -1.0, 0.0),
                Vec3::new(1.0, 1.0, 0.0),
                Vec3::new(-1.0, 1.0, 0.0),
            ],
            normals: vec![Vec3::Z; 4],
            triangles: vec![[0, 1, 2], [0, 2, 3]],
            face_ids: vec![0, 0],
        }
    }

    /// A camera outside the box (further than its margin) clips nothing:
    /// the mesh is the one given, borrowed.
    #[test]
    fn a_camera_outside_the_box_borrows_the_mesh() {
        let mesh = quad();
        let camera = Camera {
            eye: Vec3::new(0.5, -3.0, 2.0),
            target: Vec3::ZERO,
            up: Vec3::Z,
            projection: Projection::Perspective { fov_y_deg: 40.0 },
        };
        let view = View::new(&camera, 64, 48).unwrap();
        let bounds = depth_bounds(&section_bounds(&mesh), false);
        let clipped = clip_mesh(&mesh, &[], &view, &bounds).unwrap();
        assert!(clipped.is_unchanged());
        assert_eq!(clipped.origin(1), 1);
    }

    /// The near plane comes from the drawn triangles' box: a vertex no
    /// triangle uses, even an infinite one, does not move it (the GPU's
    /// box is the same), and the cap's margin does when a section has a
    /// cap. Before, `mesh.bbox()` took every position, an infinite one
    /// made the box infinite, and nothing was clipped at all.
    #[test]
    fn the_near_plane_is_the_drawn_triangles_and_the_caps() {
        let mut mesh = quad();
        let clean = section_bounds(&mesh);
        mesh.positions.push(Vec3::new(f64::INFINITY, 0.0, 0.0));
        mesh.normals.push(Vec3::Z);
        mesh.positions.push(Vec3::new(1e9, 1e9, 1e9));
        mesh.normals.push(Vec3::Z);
        assert_eq!(section_bounds(&mesh), clean);
        let camera = Camera {
            eye: Vec3::new(0.0, -0.5, 0.2),
            target: Vec3::new(0.0, 1.0, 0.0),
            up: Vec3::Z,
            projection: Projection::Perspective { fov_y_deg: 60.0 },
        };
        let view = View::new(&camera, 64, 48).unwrap();
        let bounds = depth_bounds(&section_bounds(&mesh), false);
        let near = near_distance(&view, &bounds).unwrap();
        assert!(near > 0.0 && near < 1e-4, "{near}");
        let clipped = clip_mesh(&mesh, &[], &view, &bounds).unwrap();
        assert!(!clipped.is_unchanged(), "the camera inside the box clips");
        // With a cap, the box grows by the margin.
        let capped = depth_bounds(&clean, true);
        let (lo, hi) = capped.corners().unwrap();
        let margin = clean.diagonal() * crate::SECTION_MARGIN;
        assert!((hi.x - 1.0 - margin).abs() < 1e-15 && (lo.x + 1.0 + margin).abs() < 1e-15);
    }
}
