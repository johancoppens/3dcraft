//! Near-plane clipping for the CPU renderer's perspective pictures
//! (docs/design/rendering.md › *Near-plane clipping*).
//!
//! A triangle with a corner closer to the eye than the near plane is cut
//! along that plane (Sutherland–Hodgman against `dist ≥ near`, `dist` the
//! distance along the view direction, which is the clip-space `w`), so
//! that what lies in front of the plane is drawn as the GPU draws it,
//! instead of the whole triangle being left out. The near plane is the
//! GPU's (`craft3d_gpu`'s `depth_range`): the nearest corner of the
//! mesh's box less a margin, and at least a millionth of the farthest, so
//! that a camera outside the box clips nothing and one inside it clips at
//! the same distance on both renderers.
//!
//! A new corner is the point `t` of the way along a triangle's edge, and
//! every attribute there is the same interpolation of the edge's ends:
//! along a straight edge in space the perspective-correct interpolation of
//! an attribute is the linear one, so the normal, the point and the value
//! of the analysis are exact. Its ends are taken in the order of their
//! vertex indices, so the two triangles that share an edge get the very
//! same new vertex and stay watertight; the clipped mesh is built in the
//! order of the triangles, with a `BTreeMap` for the shared edges: the
//! same mesh gives the same clipped mesh, bit for bit.

use crate::{Projection, RenderError, View};
use craft3d_math::Vec3;
use craft3d_tess::Mesh;
use std::borrow::Cow;
use std::collections::BTreeMap;

/// The distance along the view direction below which a perspective picture
/// clips (see the module docs); `None` for an orthographic one or a mesh
/// without a box (nothing to clip). Infinite when the whole mesh is behind
/// the eye.
pub(crate) fn near_distance(view: &View, mesh: &Mesh) -> Option<f64> {
    if !matches!(view.projection, Projection::Perspective { .. }) {
        return None;
    }
    let (lo, hi) = mesh.bbox().corners()?;
    let (mut nearest, mut farthest) = (f64::INFINITY, f64::NEG_INFINITY);
    for x in [lo.x, hi.x] {
        for y in [lo.y, hi.y] {
            for z in [lo.z, hi.z] {
                let dist = (Vec3::new(x, y, z) - view.eye).dot(view.forward);
                nearest = nearest.min(dist);
                farthest = farthest.max(dist);
            }
        }
    }
    if !(nearest.is_finite() && farthest.is_finite()) {
        return None;
    }
    let reach = nearest.abs().max(farthest.abs());
    let pad = (1e-3 * (farthest - nearest) + 1e-5 * reach).max(1e-30);
    let far = farthest + pad;
    if far <= 0.0 {
        return Some(f64::INFINITY);
    }
    Some((nearest - pad).max(far * 1e-6))
}

/// A corner of a clipped triangle: the vertex `lo` itself (`hi == lo`,
/// `t == 0`) or the point `t` of the way from vertex `lo` to vertex `hi`
/// (`lo < hi`).
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

/// Clips the triangle with vertices `ids` at distances `dists` against
/// `dist ≥ near`.
pub(crate) fn clip_triangle(ids: [u32; 3], dists: [f64; 3], near: f64) -> Clipped {
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
        let (Some(&a), Some(&b), Some(&da), Some(&db), Some(&ia), Some(&ib)) = (
            ids.get(k),
            ids.get(next),
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
            // The ends in the order of their indices: both triangles of
            // the edge compute the same point.
            let ((lo, dlo), (hi, dhi)) = if a <= b {
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
}

/// At most this many vertices are added (two per triangle at most; a mesh
/// is bounded long before).
const MAX_NEW_VERTICES: usize = 1 << 26;

/// `mesh` (and `values`, one per vertex or none) clipped to the near
/// plane of `view`: the mesh itself, borrowed, when no vertex is in front
/// of it.
pub(crate) fn clip_mesh<'a>(
    mesh: &'a Mesh,
    values: &'a [f32],
    view: &View,
) -> Result<ClippedMesh<'a>, RenderError> {
    let unchanged = || ClippedMesh {
        mesh: Cow::Borrowed(mesh),
        values: Cow::Borrowed(values),
        origin: None,
    };
    let Some(near) = near_distance(view, mesh) else {
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
        let [ia, ib, ic] = *tri;
        let corners = match clip_triangle(*tri, [dist(ia)?, dist(ib)?, dist(ic)?], near) {
            Clipped::Whole => {
                out.triangles.push(*tri);
                out.face_ids.push(face);
                origin.push(t);
                continue;
            }
            Clipped::Gone => continue,
            Clipped::Part(corners, n) => (corners, n),
        };
        let (corners, n) = corners;
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
            if made.len() >= MAX_NEW_VERTICES {
                return Err(RenderError::InvalidMesh { triangle: t });
            }
            let at = |v: &[Vec3], k: u32| {
                v.get(k as usize)
                    .copied()
                    .ok_or(RenderError::InvalidMesh { triangle: t })
            };
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

    #[test]
    fn a_triangle_is_whole_gone_or_cut() {
        assert_eq!(
            clip_triangle([0, 1, 2], [2.0, 3.0, 4.0], 1.0),
            Clipped::Whole
        );
        assert_eq!(
            clip_triangle([0, 1, 2], [0.0, -3.0, 0.5], 1.0),
            Clipped::Gone
        );
        assert_eq!(
            clip_triangle([0, 1, 2], [f64::NAN, 3.0, 4.0], 1.0),
            Clipped::Whole
        );
        // One corner in front: a triangle of it and two points on its edges.
        let Clipped::Part(c, n) = clip_triangle([0, 1, 2], [3.0, 0.0, 0.0], 1.0) else {
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
        assert_eq!(
            c[1],
            Corner {
                lo: 0,
                hi: 1,
                t: 2.0 / 3.0
            }
        );
        assert_eq!(
            c[2],
            Corner {
                lo: 0,
                hi: 2,
                t: 2.0 / 3.0
            }
        );
        // Two in front: a quadrilateral.
        let Clipped::Part(_, n) = clip_triangle([0, 1, 2], [3.0, 3.0, 0.0], 1.0) else {
            panic!("not cut");
        };
        assert_eq!(n, 4);
        // The shared edge's point is the same whichever way it is walked.
        let Clipped::Part(c1, _) = clip_triangle([4, 9, 2], [3.0, 0.5, 0.2], 1.0) else {
            panic!("not cut");
        };
        let Clipped::Part(c2, _) = clip_triangle([9, 4, 7], [0.5, 3.0, 0.1], 1.0) else {
            panic!("not cut");
        };
        let edge = |c: &[Corner; 4]| c.iter().copied().find(|x| (x.lo, x.hi) == (4, 9));
        assert_eq!(edge(&c1), edge(&c2));
        assert!(edge(&c1).is_some());
    }
}
