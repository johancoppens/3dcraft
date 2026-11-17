//! Where a note's point goes on a body (`note_cmds.rs`): the point of a
//! face, an edge, a vertex or a whole body nearest a given point, and a
//! point in the middle of one, from the exact geometry. On a solid a
//! face's point is the foot on its surface when the boolean kernel's
//! probe says it lies within the face's loops (`verify::nearest_on_face`),
//! else the nearest point of its edges, so every face of a valid solid
//! takes a note whatever a mesh of it would do; an edge's from its curve,
//! a vertex is its point. A sheet, which the probe does not take (it has
//! no inside), is placed by its surfaces too: a face's point is the foot
//! on its surface where that lies within the face's loops, else the
//! nearest point of its edges. Whether a foot is within the loops is
//! told in the surface's parameters: the loops drawn there from points
//! along their edges, the foot inside when an odd number of them goes
//! around it (a hole of a trimmed sheet is not on it). A loop that does
//! not close in the parameters (round a seam, through a pole) leaves the
//! face's box to tell. No mesh is made (a grid sheet of 33 000 spans took
//! 6.8 s to mesh for its first note).

use crate::verify;
use craft3d_kernel::boolean::probe::{OnFace, Probe};
use craft3d_math::{Bbox3, Vec3};
use craft3d_topo::{Body, FaceId};
use std::cell::RefCell;
use std::collections::BTreeMap;

/// What on a body: every face of it, some faces (the pieces of one name,
/// or what one feature made), some edges, some vertices.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Entity {
    Body,
    Faces(Vec<usize>),
    Edges(Vec<usize>),
    Vertices(Vec<usize>),
}

/// Points along each edge of a face whose boundary gives its middle.
const BOUNDARY_SAMPLES: u32 = 16;

/// Points along each edge of a sheet's face whose loops are drawn in its
/// surface's parameters.
const LOOP_SAMPLES: u32 = 32;

/// A face's loops in its surface's parameters: each a closed run of
/// (u, v), the last point joined to the first.
type Rings = Vec<Vec<(f64, f64)>>;

/// A body made ready to place points on.
pub(crate) struct Geometry<'a> {
    body: &'a Body,
    probe: Option<Probe<'a>>,
    linear: f64,
    size: f64,
    /// The loops of a sheet's faces in their parameters, by face, made
    /// when first asked for; `None` where they do not close there.
    rings: RefCell<BTreeMap<usize, Option<Rings>>>,
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
            linear,
            size,
            rings: RefCell::new(BTreeMap::new()),
        }
    }

    /// The diagonal of the body's box.
    pub(crate) fn size(&self) -> f64 {
        self.size
    }

    /// How far a point placed may be off the exact geometry: none, every
    /// point is placed on it (the document's tolerance aside).
    pub(crate) fn slack(&self) -> f64 {
        self.linear
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
            None => self.nearest_on_sheet_face(face, p),
        }
    }

    /// The point of a sheet's face nearest `p`: the foot on its surface
    /// where it lies within the face's loops, else the nearest point of
    /// its edges, whichever is nearer.
    fn nearest_on_sheet_face(&self, face: usize, p: Vec3) -> Option<Vec3> {
        let f = self.body.faces.get(face)?;
        let foot = f.surface.project(p).ok().and_then(|(u, v)| {
            let q = f.surface.eval(u, v);
            (q.is_finite() && self.within_sheet_face(face, (u, v), q)).then_some(q)
        });
        let edge = self
            .face_edges(face)
            .iter()
            .filter_map(|e| self.body.edges.get(*e))
            .filter_map(|edge| verify::nearest_on_edge(edge, p).map(|(q, _)| q))
            .filter(|q| q.is_finite())
            .min_by(|a, b| a.distance(p).total_cmp(&b.distance(p)));
        [foot, edge]
            .into_iter()
            .flatten()
            .min_by(|a, b| a.distance(p).total_cmp(&b.distance(p)))
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
            let inside = match &self.probe {
                Some(probe) => probe.on_face(face, foot).ok() == Some(OnFace::In),
                // A sheet: within its face's loops.
                None => self.within_sheet_face(face, (u, v), foot),
            };
            if foot.is_finite() && inside {
                return Some(foot);
            }
        }
        self.nearest_on_face(face, centre)
    }

    /// Whether `q`, the point of a sheet's face's surface at `uv`, lies
    /// on the face: within its loops where they can be drawn in the
    /// surface's parameters, else within the face's box.
    fn within_sheet_face(&self, face: usize, uv: (f64, f64), q: Vec3) -> bool {
        match self.sheet_inside(face, uv) {
            Some(inside) => inside,
            None => self
                .face_box(face)
                .is_none_or(|b| verify::box_distance(&b, q) <= self.linear.max(1e-9)),
        }
    }

    /// Whether the point at `uv` of a face's surface lies within the
    /// face's loops: an odd number of them goes around it in the
    /// parameters (each with the point turned by whole periods to its
    /// middle). `None` when the loops cannot be drawn there.
    fn sheet_inside(&self, face: usize, uv: (f64, f64)) -> Option<bool> {
        let f = self.body.faces.get(face)?;
        let (pu, pv) = (f.surface.period_u(), f.surface.period_v());
        let mut known = self.rings.borrow_mut();
        let rings = known
            .entry(face)
            .or_insert_with(|| self.rings_of(face))
            .as_ref()?;
        if rings.is_empty() {
            // A face without loops is all of its surface.
            return Some(true);
        }
        // Inside after an odd number of loops that go around it.
        let mut inside = false;
        for ring in rings {
            let (mut lo, mut hi) = (
                (f64::INFINITY, f64::INFINITY),
                (f64::NEG_INFINITY, f64::NEG_INFINITY),
            );
            for (u, v) in ring {
                lo = (lo.0.min(*u), lo.1.min(*v));
                hi = (hi.0.max(*u), hi.1.max(*v));
            }
            let q = (
                turned(uv.0, 0.5 * (lo.0 + hi.0), pu),
                turned(uv.1, 0.5 * (lo.1 + hi.1), pv),
            );
            if winding(ring, q) != 0 {
                inside = !inside;
            }
        }
        Some(inside)
    }

    /// The loops of `face` in its surface's parameters, from points along
    /// their edges, each point found from the one before it (a few
    /// evaluations of a freeform surface each) and turned by whole periods
    /// to be next to it. `None` when a point does not project, or a loop
    /// does not close in the parameters (round a seam, through a pole).
    fn rings_of(&self, face: usize) -> Option<Rings> {
        let f = self.body.faces.get(face)?;
        let (pu, pv) = (f.surface.period_u(), f.surface.period_v());
        let mut rings = Vec::with_capacity(f.loops.len());
        for l in &f.loops {
            let found = self.body.loops.get(l.0 as usize)?;
            let mut ring: Vec<(f64, f64)> = Vec::new();
            for coedge in &found.coedges {
                let edge = self.body.edges.get(coedge.edge.0 as usize)?;
                let mut points: Vec<Vec3> = verify::edge_points(edge, LOOP_SAMPLES).collect();
                if coedge.reversed {
                    points.reverse();
                }
                for p in points {
                    let at = match ring.last() {
                        None => f.surface.project(p).ok()?,
                        Some(&(u0, v0)) => {
                            let (u, v) = f.surface.project_from(p, (u0, v0)).ok()?;
                            (turned(u, u0, pu), turned(v, v0, pv))
                        }
                    };
                    if !(at.0.is_finite() && at.1.is_finite()) {
                        return None;
                    }
                    ring.push(at);
                }
            }
            let (Some(first), Some(last)) = (ring.first(), ring.last()) else {
                continue;
            };
            let open = |a: f64, b: f64, period: Option<f64>| {
                period.is_some_and(|p| (a - b).abs() > 0.5 * p)
            };
            if open(first.0, last.0, pu) || open(first.1, last.1, pv) {
                return None;
            }
            rings.push(ring);
        }
        Some(rings)
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
}

/// `x` turned by whole periods to lie within half a period of `to`; `x`
/// itself where the parameter has no period.
fn turned(x: f64, to: f64, period: Option<f64>) -> f64 {
    match period {
        Some(p) if p.is_finite() && p > 0.0 => {
            let turns = ((to - x) / p).round();
            if turns.is_finite() { x + p * turns } else { x }
        }
        _ => x,
    }
}

/// How many times `ring`, its last point joined to its first, goes around
/// `q` (counter-clockwise counted up, clockwise down).
fn winding(ring: &[(f64, f64)], q: (f64, f64)) -> i64 {
    let mut count: i64 = 0;
    for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
        let cross = (b.0 - a.0) * (q.1 - a.1) - (q.0 - a.0) * (b.1 - a.1);
        if a.1 <= q.1 {
            if b.1 > q.1 && cross > 0.0 {
                count = count.saturating_add(1);
            }
        } else if b.1 <= q.1 && cross < 0.0 {
            count = count.saturating_sub(1);
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ring_winds_about_its_inside_and_not_its_outside() {
        let square = [(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0)];
        assert_eq!(winding(&square, (1.0, 1.0)), 1);
        assert_eq!(winding(&square, (3.0, 1.0)), 0);
        let mut backwards = square;
        backwards.reverse();
        assert_eq!(winding(&backwards, (1.0, 1.0)), -1);
        assert_eq!(winding(&[], (1.0, 1.0)), 0);
    }

    #[test]
    fn a_parameter_is_turned_next_to_another() {
        let tau = std::f64::consts::TAU;
        assert!((turned(0.1, 6.2, Some(tau)) - (0.1 + tau)).abs() < 1e-12);
        assert!((turned(6.2, 0.1, Some(tau)) - (6.2 - tau)).abs() < 1e-12);
        assert!((turned(0.1, 6.2, None) - 0.1).abs() < 1e-15);
        assert!(turned(f64::NAN, 1.0, Some(tau)).is_nan());
    }
}
