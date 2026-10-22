//! The mesh of a body against its faces' surfaces, point by point, with
//! loose entities in mind (tolerant modelling, slice 2, 2026-10-22;
//! `docs/design/tessellator.md`, *On its surface, within whose
//! tolerance*).
//!
//! A point of a face's mesh inside the face is on the face's surface to
//! rounding. A point on the face's boundary is a point of an edge's curve
//! (or a vertex), and a loose edge's curve lies up to its own tolerance
//! off the face's surface (its tube), a loose vertex up to its own: such
//! a point is held to that entity's tolerance, not the document's and not
//! rounding. Which entity a point is on is found from the point: a point
//! further than rounding from the surface must lie within rounding of an
//! edge of its face (or of one of their vertices), and is then allowed the
//! largest tolerance among those it lies on.

use crate::Mesh;
use craft3d_math::Vec3;
use craft3d_topo::{Body, EdgeId, FaceId};

/// A point of a face's mesh further from the face's surface than it may
/// be.
#[derive(Clone, Debug, PartialEq)]
pub struct OffSurface {
    pub face: FaceId,
    /// Index into `Mesh::positions`.
    pub position: usize,
    pub distance: f64,
    /// What it may be: rounding, or the tolerance of the edge or vertex
    /// it lies on.
    pub allowed: f64,
}

/// The points of `mesh` (a mesh of `body`, `Mesh::face_ids` naming its
/// faces) further from their face's surface than allowed: `rounding` for
/// a point inside a face; for a point within `rounding` of an edge of the
/// face, or of a vertex of such an edge, the entity's tolerance in a
/// document whose linear tolerance is `linear` (its own or the
/// document's), plus `rounding`. Each position is looked at once, for the
/// first triangle that uses it, in triangle order.
pub fn off_surfaces(body: &Body, mesh: &Mesh, linear: f64, rounding: f64) -> Vec<OffSurface> {
    let mut seen = vec![false; mesh.positions.len()];
    let mut out = Vec::new();
    let mut edges_of: Vec<Option<Vec<EdgeId>>> = vec![None; body.faces.len()];
    for (t, tri) in mesh.triangles.iter().enumerate() {
        let Some(&f) = mesh.face_ids.get(t) else {
            continue;
        };
        let face_id = FaceId(f);
        let Some(face) = body.face(face_id) else {
            continue;
        };
        for &k in tri {
            let k = k as usize;
            match seen.get_mut(k) {
                Some(s) if !*s => *s = true,
                _ => continue,
            }
            let Some(&p) = mesh.positions.get(k) else {
                continue;
            };
            let distance = face
                .surface
                .project(p)
                .ok()
                .map(|(u, v)| face.surface.eval(u, v).distance(p))
                .filter(|d| d.is_finite())
                .unwrap_or(f64::INFINITY);
            if distance <= rounding {
                continue;
            }
            let edges = edges_of
                .get_mut(f as usize)
                .map(|slot| {
                    slot.get_or_insert_with(|| face_edges(body, face_id))
                        .clone()
                })
                .unwrap_or_default();
            let allowed =
                on_boundary(body, &edges, p, linear, rounding).map_or(rounding, |t| t + rounding);
            if distance > allowed {
                out.push(OffSurface {
                    face: face_id,
                    position: k,
                    distance,
                    allowed,
                });
            }
        }
    }
    out
}

/// The edges a face's loops use, each once, in order.
fn face_edges(body: &Body, face: FaceId) -> Vec<EdgeId> {
    let mut edges: Vec<EdgeId> = body
        .face(face)
        .map(|f| {
            f.loops
                .iter()
                .filter_map(|l| body.loop_(*l))
                .flat_map(|l| l.coedges.iter().map(|c| c.edge))
                .collect()
        })
        .unwrap_or_default();
    edges.sort();
    edges.dedup();
    edges
}

/// The largest tolerance of an edge among `edges`, or of a vertex of one,
/// that `p` lies within `rounding` of; `None` when it lies on none.
fn on_boundary(body: &Body, edges: &[EdgeId], p: Vec3, linear: f64, rounding: f64) -> Option<f64> {
    let mut best: Option<f64> = None;
    let mut take = |t: f64| best = Some(best.map_or(t, |b: f64| b.max(t)));
    for &e in edges {
        let Some(edge) = body.edge(e) else {
            continue;
        };
        for v in [edge.start, edge.end] {
            if body
                .vertex(v)
                .is_some_and(|x| x.point.distance(p) <= rounding)
            {
                take(body.vertex_tolerance_at(v, linear));
            }
        }
        let (t0, t1) = edge.range;
        let on = edge.curve.project(p).ok().is_some_and(|t| {
            let at = |t: f64| edge.curve.eval(t).distance(p);
            let mut d = at(t0).min(at(t1));
            if let Some(period) = edge.curve.period()
                && period > 0.0
            {
                let turns = ((t0 - t) / period).ceil();
                let t = if turns.is_finite() && turns.abs() < 1e6 {
                    t + turns * period
                } else {
                    t
                };
                if (t0..=t1).contains(&t) {
                    d = d.min(at(t));
                }
            } else if (t0..=t1).contains(&t) {
                d = d.min(at(t));
            }
            d <= rounding
        });
        if on {
            take(body.edge_tolerance_at(e, linear));
        }
    }
    best
}
