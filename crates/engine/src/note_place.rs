//! Where a note's point goes on a body (`note_cmds.rs`): the point of a
//! face, an edge, a vertex or a whole body nearest a given point, and a
//! point in the middle of one, from the exact geometry. On a solid a
//! face's point is the foot on its surface when the boolean kernel's
//! probe says it lies within the face's loops (`verify::nearest_on_face`),
//! else the nearest point of its edges, so every face of a valid solid
//! takes a note whatever a mesh of it would do; an edge's from its curve,
//! a vertex is its point. A sheet, which the probe does not take (it has
//! no inside), is placed on a mesh of it.

use crate::verify;
use craft3d_kernel::boolean::probe::{OnFace, Probe};
use craft3d_math::{Bbox3, Vec3};
use craft3d_topo::{Body, FaceId};
use std::cell::OnceCell;

/// What on a body: every face of it, some faces (the pieces of one name,
/// or what one feature made), some edges, some vertices.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Entity {
    Body,
    Faces(Vec<usize>),
    Edges(Vec<usize>),
    Vertices(Vec<usize>),
}

/// A sheet's mesh: within this share of the body's size, at most this
/// many triangles.
const MESH_SHARE: f64 = 1e-3;
const MESH_TRIANGLES: usize = 200_000;
/// Points along each edge of a face whose boundary gives its middle.
const BOUNDARY_SAMPLES: u32 = 16;

/// A body made ready to place points on.
pub(crate) struct Geometry<'a> {
    body: &'a Body,
    probe: Option<Probe<'a>>,
    /// For a sheet: its mesh, made when first asked for.
    mesh: OnceCell<Option<craft3d_tess::Mesh>>,
    linear: f64,
    size: f64,
}

impl<'a> Geometry<'a> {
    pub(crate) fn new(body: &'a Body, linear: f64, angular: f64) -> Self {
        let probe = craft3d_math::Tolerance::new(linear, angular)
            .and_then(|tolerance| Probe::new(body, &tolerance).ok());
        let size = body
            .bbox()
            .ok()
            .map(|b| b.diagonal())
            .filter(|d| d.is_finite() && *d > 0.0)
            .unwrap_or(1.0);
        Geometry {
            body,
            probe,
            mesh: OnceCell::new(),
            linear,
            size,
        }
    }

    /// The diagonal of the body's box.
    pub(crate) fn size(&self) -> f64 {
        self.size
    }

    /// How far a mesh of a sheet may be off its surface (0 for a solid,
    /// which is placed on its exact geometry).
    pub(crate) fn slack(&self) -> f64 {
        if self.probe.is_some() {
            0.0
        } else {
            (self.size * MESH_SHARE).max(10.0 * self.linear)
        }
    }

    /// The point of `what` nearest `p`; `None` when it has none.
    pub(crate) fn nearest(&self, what: &Entity, p: Vec3) -> Option<Vec3> {
        if !p.is_finite() {
            return None;
        }
        match what {
            Entity::Body => {
                self.nearest_on_faces(&(0..self.body.faces.len()).collect::<Vec<_>>(), p)
            }
            Entity::Faces(faces) => self.nearest_on_faces(faces, p),
            Entity::Edges(edges) => edges
                .iter()
                .filter_map(|e| self.body.edges.get(*e))
                .filter_map(|edge| verify::nearest_on_edge(edge, p).map(|(q, _)| q))
                .filter(|q| q.is_finite())
                .min_by(|a, b| a.distance(p).total_cmp(&b.distance(p))),
            Entity::Vertices(vertices) => vertices
                .iter()
                .filter_map(|v| self.body.vertices.get(*v))
                .map(|v| v.point)
                .filter(|q| q.is_finite())
                .min_by(|a, b| a.distance(p).total_cmp(&b.distance(p))),
        }
    }

    /// A point in the middle of `what`: of a face, its foot nearest the
    /// centre of its boundary that lies within it; of an edge, its curve
    /// at the middle of its range; a vertex itself; of a body, its point
    /// nearest the middle of its box. Of several, the first.
    pub(crate) fn middle(&self, what: &Entity) -> Option<Vec3> {
        match what {
            Entity::Body => {
                let centre = self
                    .body
                    .bbox()
                    .ok()
                    .and_then(|b| b.corners())
                    .map(|(lo, hi)| (lo + hi) * 0.5)?;
                self.nearest(&Entity::Body, centre)
            }
            Entity::Faces(faces) => faces.iter().find_map(|f| self.face_middle(*f)),
            Entity::Edges(edges) => edges.iter().find_map(|e| self.edge_middle(*e)),
            Entity::Vertices(vertices) => vertices
                .iter()
                .find_map(|v| self.body.vertices.get(*v))
                .map(|v| v.point)
                .filter(|q| q.is_finite()),
        }
    }

    fn face_box(&self, face: usize) -> Option<Bbox3> {
        u32::try_from(face)
            .ok()
            .and_then(|id| self.body.face_bbox(FaceId(id)).ok())
            .filter(|b| !b.is_empty())
    }

    /// The nearest point of `faces`, looked for nearest box first.
    fn nearest_on_faces(&self, faces: &[usize], p: Vec3) -> Option<Vec3> {
        let mut order: Vec<(f64, usize)> = faces
            .iter()
            .filter(|f| self.body.faces.get(**f).is_some())
            .map(|f| {
                let near = self
                    .face_box(*f)
                    .map_or(0.0, |b| verify::box_distance(&b, p));
                (near, *f)
            })
            .collect();
        order.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let mut best: Option<(f64, Vec3)> = None;
        for (near, face) in order {
            if best.is_some_and(|(d, _)| near > d) {
                break;
            }
            if let Some(q) = self.nearest_on_face(face, p) {
                let d = q.distance(p);
                if best.is_none_or(|(b, _)| d < b) {
                    best = Some((d, q));
                }
            }
        }
        best.map(|(_, q)| q)
    }

    fn nearest_on_face(&self, face: usize, p: Vec3) -> Option<Vec3> {
        match &self.probe {
            Some(probe) => verify::nearest_on_face(probe, face, p)
                .ok()
                .filter(|q| q.is_finite()),
            None => {
                let tris = self.triangles(Some(face));
                nearest_on_triangles(&tris, p).map(|(q, _)| q)
            }
        }
    }

    /// The edges of `face`, once each.
    fn face_edges(&self, face: usize) -> Vec<usize> {
        let mut out: Vec<usize> = self
            .body
            .faces
            .get(face)
            .into_iter()
            .flat_map(|f| f.loops.iter())
            .filter_map(|l| self.body.loops.get(l.0 as usize))
            .flat_map(|l| l.coedges.iter())
            .map(|c| c.edge.0 as usize)
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// A point of `face` in its middle ([`Geometry::middle`]).
    fn face_middle(&self, face: usize) -> Option<Vec3> {
        let Some(probe) = &self.probe else {
            return middle_of_triangles(&self.triangles(Some(face)));
        };
        let f = self.body.faces.get(face)?;
        let edges = self.face_edges(face);
        // The centre of the boundary: its points weighted by the length
        // they stand for.
        let mut total = 0.0;
        let mut sum = Vec3::new(0.0, 0.0, 0.0);
        let mut middles = Vec::new();
        for e in &edges {
            let Some(edge) = self.body.edges.get(*e) else {
                continue;
            };
            let points: Vec<Vec3> = verify::edge_points(edge, BOUNDARY_SAMPLES).collect();
            for pair in points.windows(2) {
                if let [a, b] = pair {
                    let length = a.distance(*b);
                    if length.is_finite() && length > 0.0 {
                        total += length;
                        sum += (*a + *b) * (0.5 * length);
                    }
                }
            }
            if let Some(m) = self.edge_middle(*e) {
                middles.push(m);
            }
        }
        let centre = if total > 0.0 {
            sum * (1.0 / total)
        } else {
            // A face without edges (a whole sphere): the middle of its
            // parameters.
            let ((u0, u1), (v0, v1)) = f.surface.domain();
            let mid = |a: f64, b: f64| {
                if a.is_finite() && b.is_finite() {
                    0.5 * (a + b)
                } else {
                    0.0
                }
            };
            f.surface.eval(mid(u0, u1), mid(v0, v1))
        };
        // The foot of the centre, else of points between it and the
        // middles of the edges, that lies inside the face.
        let mut tried = vec![centre];
        for share in [0.5, 0.25, 0.75] {
            tried.extend(middles.iter().map(|m| centre + (*m - centre) * share));
        }
        for q in tried {
            if !q.is_finite() {
                continue;
            }
            let Ok((u, v)) = f.surface.project(q) else {
                continue;
            };
            let foot = f.surface.eval(u, v);
            if foot.is_finite() && probe.on_face(face, foot).ok() == Some(OnFace::In) {
                return Some(foot);
            }
        }
        self.nearest_on_face(face, centre)
    }

    fn edge_middle(&self, index: usize) -> Option<Vec3> {
        let edge = self.body.edges.get(index)?;
        let (t0, t1) = edge.range;
        let mid = 0.5 * (t0 + t1);
        if !mid.is_finite() {
            return None;
        }
        Some(edge.curve.eval(mid)).filter(|p| p.is_finite())
    }

    /// The triangles of `face` (every face when `None`) of a sheet's
    /// mesh.
    fn triangles(&self, face: Option<usize>) -> Vec<[Vec3; 3]> {
        let mesh = self.mesh.get_or_init(|| {
            let params = craft3d_tess::TessParams {
                chordal_tolerance: self.slack().max(f64::MIN_POSITIVE),
                max_triangles: Some(MESH_TRIANGLES),
                max_angle: Some(15f64.to_radians()),
                ..craft3d_tess::TessParams::default()
            };
            craft3d_tess::tessellate_within_budget(self.body, &params)
                .ok()
                .map(|made| made.mesh)
        });
        let Some(mesh) = mesh else {
            return Vec::new();
        };
        mesh.face_ids
            .iter()
            .enumerate()
            .filter(|(_, f)| face.is_none_or(|face| usize::try_from(**f).ok() == Some(face)))
            .filter_map(|(t, _)| mesh.corners(t))
            .collect()
    }
}

/// The point of `tris` nearest `p`, and its distance; `None` for none.
pub(crate) fn nearest_on_triangles(tris: &[[Vec3; 3]], p: Vec3) -> Option<(Vec3, f64)> {
    tris.iter()
        .map(|[a, b, c]| {
            let q = crate::analysis::closest_on_triangle(p, *a, *b, *c);
            (q, q.distance(p))
        })
        .filter(|(_, d)| !d.is_nan())
        .min_by(|x, y| x.1.total_cmp(&y.1))
}

/// The point of the triangles nearest their centre of area.
pub(crate) fn middle_of_triangles(tris: &[[Vec3; 3]]) -> Option<Vec3> {
    let mut total = 0.0;
    let mut sum = Vec3::new(0.0, 0.0, 0.0);
    for [a, b, c] in tris {
        let area = (*b - *a).cross(*c - *a).length() * 0.5;
        if area.is_finite() && area > 0.0 {
            total += area;
            sum += (*a + *b + *c) * (area / 3.0);
        }
    }
    let centre = if total > 0.0 {
        sum * (1.0 / total)
    } else {
        let [a, b, c] = tris.first()?;
        (*a + *b + *c) * (1.0 / 3.0)
    };
    nearest_on_triangles(tris, centre).map(|(q, _)| q)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    #[test]
    fn triangles_give_their_nearest_points_and_middles() {
        let tri = |a: [f64; 3], b: [f64; 3], c: [f64; 3]| {
            [
                Vec3::new(a[0], a[1], a[2]),
                Vec3::new(b[0], b[1], b[2]),
                Vec3::new(c[0], c[1], c[2]),
            ]
        };
        // A square ring of two triangles each side: its middle lies on it.
        let ring = [
            tri([0.0, 0.0, 0.0], [3.0, 0.0, 0.0], [3.0, 1.0, 0.0]),
            tri([0.0, 0.0, 0.0], [3.0, 1.0, 0.0], [0.0, 1.0, 0.0]),
            tri([0.0, 2.0, 0.0], [3.0, 2.0, 0.0], [3.0, 3.0, 0.0]),
            tri([0.0, 2.0, 0.0], [3.0, 3.0, 0.0], [0.0, 3.0, 0.0]),
        ];
        let middle = middle_of_triangles(&ring).unwrap();
        let (_, d) = nearest_on_triangles(&ring, middle).unwrap();
        assert!(d < 1e-12, "{middle:?}");
        assert!(middle_of_triangles(&[]).is_none());
        let far = nearest_on_triangles(&ring, Vec3::new(1e300, 0.0, 0.0)).unwrap();
        assert!(far.1.is_infinite() || far.1 > 1e299);
    }
}
