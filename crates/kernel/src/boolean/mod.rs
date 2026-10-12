//! Booleans: the union, difference and intersection of two solids.
//!
//! Supported so far: solids whose faces are planes and cylinders and whose
//! edges are lines, circles and ellipses, where no two cylinders with
//! different axis directions cross and nothing touches tangentially.
//! Coincident faces (two faces in one plane, or on one cylinder) are
//! handled. Everything else is [`KernelError::Unsupported`] with the faces
//! and the reason; a result is never guessed and never returned without
//! passing `craft3d_topo::check`.
//!
//! How it works (the classical imprint, classify, select and sew):
//!
//! 1. **Section.** For every face of A and face of B whose boxes meet, the
//!    curves in which their surfaces cross are computed in closed form
//!    (`craft3d_geom::intersect`). Each curve is cut where an edge of either
//!    face meets the other face's surface; a stretch between two cuts is
//!    kept when its middle is in or on both faces. Its ends become
//!    vertices: points closer than half the linear tolerance are one
//!    vertex, points between that and twice the tolerance are refused.
//! 2. **Imprint.** Every edge of A and B is split at the vertices found on
//!    it, and the pieces are registered as *arcs*; so are the stretches of
//!    step 1. Two arcs between the same vertices along the same curve are
//!    one arc, which is how an edge of B lying on a face of A, or along an
//!    edge of A, becomes shared.
//! 3. **Regions.** A face with arcs across it falls into regions
//!    ([`regions`]).
//! 4. **Classify.** One point well inside each region decides for the whole
//!    region: on a face of the other solid with a coincident surface (with
//!    the same or the opposite outward normal), or else inside or outside
//!    the other solid by casting a ray and counting crossings.
//! 5. **Select.** Union keeps what is outside the other solid, intersection
//!    what is inside, difference the outside of A and the inside of B, the
//!    latter turned inside out. Of two coincident regions at most A's is
//!    kept (see [`boolean`]).
//! 6. **Sew.** The kept regions become the faces of the result; arcs become
//!    its edges. They already share arcs, so nothing is matched by
//!    position here. Faces connected through edges form a shell.

mod outline;
mod regions;

use crate::{KernelError, MAX_REPORTED};
use craft3d_geom::intersect::{
    CurveIntersection, SurfaceIntersection, intersect_curve_surface, intersect_surfaces,
};
use craft3d_geom::{Curve3, GeomError, Line, Surface};
use craft3d_math::{Bbox3, Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId, FaceId, VertexId, check};
use outline::{
    Outline, Seg, Side, bbox, boxes_meet, interior_point, line_meets_box, locate, side_of,
    surface_distance,
};
use regions::Half;
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::TAU;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BooleanOp {
    Union,
    /// A minus B.
    Subtract,
    Intersect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Operand {
    A,
    B,
}

/// Where a face of the result came from: it is a face, or a piece of a
/// face, of one operand. Several faces of the result may have the same
/// source, when a face was cut into pieces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FaceSource {
    pub operand: Operand,
    pub face: FaceId,
}

/// Where an edge of the result came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EdgeSource {
    /// An edge, or a piece of an edge, of one operand. Where edges of both
    /// operands coincide it is A's.
    Kept { operand: Operand, edge: EdgeId },
    /// New: where face `a` of A crosses face `b` of B. Two faces can cross
    /// in several edges.
    Intersection { a: FaceId, b: FaceId },
}

/// An edge or a face of an operand.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Entity {
    Edge(EdgeId),
    Face(FaceId),
}

/// Where a vertex of the result came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VertexSource {
    /// A vertex of one operand. Where vertices of both coincide it is A's.
    Kept { operand: Operand, vertex: VertexId },
    /// New: where `a` of A meets `b` of B. Each is the edge the vertex lies
    /// on, or else a face it lies on: an edge through a face, two edges
    /// that cross, or two faces for the one vertex of a closed
    /// intersection curve. `index` counts the vertices with the same `a`
    /// and `b`, in the order of the result's vertices.
    New { a: Entity, b: Entity, index: u32 },
}

/// The result of a boolean and where each of its entities came from:
/// `faces[i]` belongs to `FaceId(i)` of `body`, and so on.
#[derive(Clone, Debug, PartialEq)]
pub struct BooleanResult {
    pub body: Body,
    pub faces: Vec<FaceSource>,
    pub edges: Vec<EdgeSource>,
    pub vertices: Vec<VertexSource>,
}

/// Why something is refused, as a fixed text: tests count refusals by it.
pub(crate) type Reason = &'static str;

const OPERATION: &str = "boolean";
pub(crate) const NEAR: Reason =
    "two points closer together than the tolerance can tell apart (at the edge of tolerance)";
const AMBIGUOUS: Reason =
    "a curve that is partly on and partly off a face boundary (at the edge of tolerance)";
const INCONSISTENT: Reason = "an imprinted curve that does not fit the edges it should coincide with (at the edge of tolerance)";
const TANGENT_SURFACES: Reason = "two faces touch along a curve without crossing (tangent contact)";
const TANGENT_EDGE: Reason = "an edge touches a face without crossing it (tangent contact)";
const NON_MANIFOLD: Reason =
    "the result would have an edge shared by more than two faces (the bodies touch along an edge)";
const ON_A_FACE: Reason = "a piece of a face lies within tolerance of a face it is not coincident with (at the edge of tolerance)";
const NO_RAY: Reason = "could not tell inside from outside: every ray tried grazes the other body";
const LOST: Reason = "the boolean lost track of an entity; this is a bug";

enum Stop {
    Unsupported {
        reason: Reason,
        faces: Vec<(Operand, FaceId)>,
    },
    Error(KernelError),
}

impl Stop {
    fn of(reason: Reason) -> Stop {
        Stop::Unsupported {
            reason,
            faces: Vec::new(),
        }
    }

    /// The same stop, naming `faces` when it names none yet.
    fn at(self, named: &[(Operand, FaceId)]) -> Stop {
        match self {
            Stop::Unsupported { reason, faces } if faces.is_empty() => Stop::Unsupported {
                reason,
                faces: named.to_vec(),
            },
            other => other,
        }
    }
}

fn geometry(error: GeomError, faces: &[(Operand, FaceId)]) -> Stop {
    match error {
        GeomError::Unsupported(reason) => Stop::Unsupported {
            reason,
            faces: faces.to_vec(),
        },
        other => Stop::Error(KernelError::Geometry(other)),
    }
}

fn face_id(index: usize) -> FaceId {
    FaceId(u32::try_from(index).unwrap_or(u32::MAX))
}

/// One operand, prepared.
struct Solid<'a> {
    operand: Operand,
    body: &'a Body,
    /// Per face.
    outlines: Vec<Outline>,
    boxes: Vec<Bbox3>,
    /// The merged vertex of each of the body's vertices.
    vertices: Vec<usize>,
}

impl Solid<'_> {
    fn name(&self, face: usize) -> (Operand, FaceId) {
        (self.operand, face_id(face))
    }

    /// The edges around a face, each once.
    fn edges_of(&self, face: usize) -> Vec<EdgeId> {
        let mut edges = BTreeSet::new();
        if let Some(f) = self.body.faces.get(face) {
            for loop_ in f.loops.iter().filter_map(|&l| self.body.loop_(l)) {
                edges.extend(loop_.coedges.iter().map(|c| c.edge));
            }
        }
        edges.into_iter().collect()
    }

    fn ends(&self, edge: EdgeId) -> Option<(usize, usize)> {
        let e = self.body.edge(edge)?;
        Some((
            *self.vertices.get(e.start.0 as usize)?,
            *self.vertices.get(e.end.0 as usize)?,
        ))
    }
}

/// What is known about a merged vertex.
#[derive(Clone, Copy, Debug, Default)]
struct Meeting {
    kept: Option<(Operand, VertexId)>,
    a_edge: Option<EdgeId>,
    b_edge: Option<EdgeId>,
    a_face: Option<FaceId>,
    b_face: Option<FaceId>,
}

/// Points, merged within half the tolerance.
struct Vertices {
    points: Vec<Vec3>,
    meetings: Vec<Meeting>,
    merge: f64,
    guard: f64,
}

impl Vertices {
    fn find_or_add(&mut self, p: Vec3) -> Result<usize, Stop> {
        let mut nearest: Option<(f64, usize)> = None;
        for (index, q) in self.points.iter().enumerate() {
            let d = p.distance(*q);
            if d < self.guard && nearest.is_none_or(|(least, _)| d < least) {
                nearest = Some((d, index));
            }
        }
        match nearest {
            Some((d, index)) if d <= self.merge => Ok(index),
            Some(_) => Err(Stop::of(NEAR)),
            None => {
                if !p.is_finite() {
                    return Err(Stop::of(LOST));
                }
                self.points.push(p);
                self.meetings.push(Meeting::default());
                Ok(self.points.len() - 1)
            }
        }
    }
}

/// A piece of a curve between two merged vertices: an edge of the result
/// when a kept region uses it.
#[derive(Clone, Debug)]
pub(crate) struct Arc {
    pub curve: Curve3,
    pub range: (f64, f64),
    pub start: usize,
    pub end: usize,
    pub source: EdgeSource,
}

#[derive(Default)]
struct Arcs {
    arcs: Vec<Arc>,
    by_ends: BTreeMap<(usize, usize), Vec<usize>>,
}

impl Arcs {
    /// The arc along `curve` over `range` between the two vertices: an
    /// existing one when there is one in the same place (and then whether
    /// it runs the other way), otherwise a new one.
    fn find_or_add(&mut self, new: Arc, tol: f64) -> (usize, bool) {
        let key = (new.start.min(new.end), new.start.max(new.end));
        let middle = 0.5 * (new.range.0 + new.range.1);
        let (point, tangent) = new.curve.derivatives(middle);
        for &index in self.by_ends.get(&key).into_iter().flatten() {
            let Some(old) = self.arcs.get(index) else {
                continue;
            };
            let Some(t) = locate(&old.curve, old.range, point, tol) else {
                continue;
            };
            let flipped = if new.start == new.end {
                old.curve.derivatives(t).1.dot(tangent) < 0.0
            } else {
                old.start != new.start
            };
            return (index, flipped);
        }
        self.arcs.push(new);
        let index = self.arcs.len() - 1;
        self.by_ends.entry(key).or_default().push(index);
        (index, false)
    }
}

/// A stretch of an intersection curve within both faces.
struct Stretch {
    curve: Curve3,
    range: (f64, f64),
    start: usize,
    end: usize,
    fa: usize,
    fb: usize,
    /// Strictly inside the face, rather than along its boundary.
    inside_a: bool,
    inside_b: bool,
}

#[derive(Default)]
struct Sections {
    stretches: Vec<Stretch>,
    /// Per face of A: the faces of B on the same surface, and whether the
    /// surfaces' normals agree. And the other way round.
    coincident_a: BTreeMap<usize, Vec<(usize, bool)>>,
    coincident_b: BTreeMap<usize, Vec<(usize, bool)>>,
    /// Per edge: the vertices to split it at.
    splits_a: BTreeMap<EdgeId, BTreeSet<usize>>,
    splits_b: BTreeMap<EdgeId, BTreeSet<usize>>,
}

fn prepare<'a>(
    body: &'a Body,
    operand: Operand,
    vertices: &mut Vertices,
    tolerance: &Tolerance,
) -> Result<Solid<'a>, Stop> {
    if body.kind != BodyKind::Solid {
        return Err(Stop::of("an operand that is a sheet; booleans need solids"));
    }
    let mut defects = check(body, tolerance);
    if !defects.is_empty() {
        defects.truncate(MAX_REPORTED);
        return Err(Stop::Error(KernelError::InvalidBody {
            operation: match operand {
                Operand::A => "the first operand of a boolean",
                Operand::B => "the second operand of a boolean",
            },
            defects,
        }));
    }
    let mut merged = Vec::with_capacity(body.vertices.len());
    for (index, vertex) in body.vertices.iter().enumerate() {
        let at = vertices.find_or_add(vertex.point)?;
        let meeting = vertices.meetings.get_mut(at).ok_or(Stop::of(LOST))?;
        if meeting.kept.is_none() {
            meeting.kept = Some((operand, VertexId(u32::try_from(index).unwrap_or(u32::MAX))));
        }
        merged.push(at);
    }
    let mut outlines = Vec::with_capacity(body.faces.len());
    let mut boxes = Vec::with_capacity(body.faces.len());
    for (index, face) in body.faces.iter().enumerate() {
        let named = [(operand, face_id(index))];
        if !matches!(face.surface, Surface::Plane(_) | Surface::Cylinder(_)) {
            return Err(Stop::of(outline::NOT_PLANE_OR_CYLINDER).at(&named));
        }
        let mut loops = Vec::with_capacity(face.loops.len());
        for loop_ in face.loops.iter().filter_map(|&l| body.loop_(l)) {
            let mut segs = Vec::with_capacity(loop_.coedges.len());
            for coedge in &loop_.coedges {
                let edge = body.edge(coedge.edge).ok_or(Stop::of(LOST))?;
                if matches!(edge.curve, Curve3::Nurbs(_)) {
                    return Err(Stop::of(
                        "a NURBS edge; booleans handle lines, circles and ellipses so far",
                    )
                    .at(&named));
                }
                let (Some(a), Some(b)) = (body.vertex(edge.start), body.vertex(edge.end)) else {
                    return Err(Stop::of(LOST));
                };
                segs.push(Seg {
                    curve: edge.curve.clone(),
                    range: edge.range,
                    a: a.point,
                    b: b.point,
                    closed: edge.start == edge.end,
                    reversed: coedge.reversed,
                });
            }
            loops.push(segs);
        }
        if loops.is_empty() {
            return Err(Stop::of("a face without a boundary").at(&named));
        }
        boxes.push(bbox(&loops));
        outlines.push(Outline {
            surface: face.surface.clone(),
            reversed: face.reversed,
            loops,
        });
    }
    Ok(Solid {
        operand,
        body,
        outlines,
        boxes,
        vertices: merged,
    })
}

/// A place where an intersection curve may enter or leave a face.
#[derive(Clone, Copy)]
struct Cut {
    t: f64,
    point: Vec3,
    a_edge: Option<EdgeId>,
    b_edge: Option<EdgeId>,
}

/// Adds to `cuts` where the edges of `face` of `own` meet `surface`, the
/// surface of `other_face`.
fn edge_cuts(
    own: &Solid,
    face: usize,
    surface: &Surface,
    other: &Outline,
    named: &[(Operand, FaceId)],
    tolerance: &Tolerance,
    cuts: &mut Vec<Cut>,
) -> Result<(), Stop> {
    let tol = tolerance.linear;
    for id in own.edges_of(face) {
        let edge = own.body.edge(id).ok_or(Stop::of(LOST))?;
        let (Some(a), Some(b)) = (own.body.vertex(edge.start), own.body.vertex(edge.end)) else {
            return Err(Stop::of(LOST));
        };
        let mut cut = |point: Vec3, on_edge: bool| {
            let edge = on_edge.then_some(id);
            cuts.push(match own.operand {
                Operand::A => Cut {
                    t: 0.0,
                    point,
                    a_edge: edge,
                    b_edge: None,
                },
                Operand::B => Cut {
                    t: 0.0,
                    point,
                    a_edge: None,
                    b_edge: edge,
                },
            });
        };
        // An edge that lies on the surface, by the checker's own measure.
        let (t0, t1) = edge.range;
        let lies_on = (0..=4).all(|k| {
            let t = t0 + (t1 - t0) * f64::from(k) / 4.0;
            surface_distance(surface, edge.curve.eval(t)) <= tol
        });
        if lies_on {
            cut(a.point, false);
            cut(b.point, false);
            continue;
        }
        match intersect_curve_surface(&edge.curve, surface, tolerance)
            .map_err(|e| geometry(e, named))?
        {
            CurveIntersection::OnSurface => {
                // The whole curve does, but this piece of it did not pass
                // the test above.
                return Err(Stop::of(AMBIGUOUS).at(named));
            }
            CurveIntersection::Hits(hits) => {
                for hit in hits {
                    if locate(&edge.curve, edge.range, hit.point, tol).is_none() {
                        continue;
                    }
                    if hit.tangent {
                        let side = side_of(&other.surface, &other.loops, hit.point, tol)
                            .map_err(|r| Stop::of(r).at(named))?;
                        if side != Side::Out {
                            return Err(Stop::of(TANGENT_EDGE).at(named));
                        }
                        continue;
                    }
                    cut(hit.point, true);
                }
            }
        }
    }
    Ok(())
}

/// The stretches of `curve`, an intersection curve of the surfaces of face
/// `fa` of A and `fb` of B, that are within both faces.
#[allow(clippy::too_many_arguments)]
fn clip(
    curve: &Curve3,
    a: &Solid,
    fa: usize,
    b: &Solid,
    fb: usize,
    vertices: &mut Vertices,
    tolerance: &Tolerance,
    sections: &mut Sections,
) -> Result<(), Stop> {
    let tol = tolerance.linear;
    let named = [a.name(fa), b.name(fb)];
    let (Some(oa), Some(ob)) = (a.outlines.get(fa), b.outlines.get(fb)) else {
        return Err(Stop::of(LOST));
    };
    let mut found = Vec::new();
    edge_cuts(a, fa, &ob.surface, ob, &named, tolerance, &mut found)?;
    edge_cuts(b, fb, &oa.surface, oa, &named, tolerance, &mut found)?;

    // Those on this curve (the surfaces may cross in another one too), in
    // order along it, each place once.
    let closed = curve.period().is_some();
    let mut along: Vec<Cut> = Vec::new();
    for mut cut in found {
        let Ok(t) = curve.project(cut.point) else {
            continue;
        };
        if curve.eval(t).distance(cut.point) <= tol {
            cut.t = t;
            along.push(cut);
        }
    }
    along.sort_by(|p, q| p.t.total_cmp(&q.t));
    let mut cuts: Vec<Cut> = Vec::new();
    for cut in along {
        let same = |other: &Cut| -> Result<bool, Stop> {
            let d = other.point.distance(cut.point);
            if d > vertices.merge && d < vertices.guard {
                return Err(Stop::of(NEAR).at(&named));
            }
            Ok(d <= vertices.merge)
        };
        let is_last = match cuts.last() {
            Some(last) => same(last)?,
            None => false,
        };
        let is_first = match cuts.first() {
            Some(first) if closed && !is_last => same(first)?,
            _ => false,
        };
        let merged = if is_last {
            cuts.last_mut()
        } else if is_first {
            cuts.first_mut()
        } else {
            None
        };
        match merged {
            Some(into) => {
                into.a_edge = into.a_edge.or(cut.a_edge);
                into.b_edge = into.b_edge.or(cut.b_edge);
            }
            None => cuts.push(cut),
        }
    }

    // The stretches between the cuts.
    let mut spans: Vec<(Cut, Cut, f64, f64)> = Vec::new();
    if closed {
        let seam = Cut {
            t: 0.0,
            point: curve.eval(0.0),
            a_edge: None,
            b_edge: None,
        };
        match cuts.as_slice() {
            [] => spans.push((seam, seam, 0.0, TAU)),
            [first, ..] => {
                let after = cuts.iter().skip(1).chain(std::iter::once(first));
                for (from, to) in cuts.iter().zip(after) {
                    let end = if to.t > from.t { to.t } else { to.t + TAU };
                    spans.push((*from, *to, from.t, end));
                }
            }
        }
    } else {
        for (from, to) in cuts.iter().zip(cuts.iter().skip(1)) {
            spans.push((*from, *to, from.t, to.t));
        }
    }

    for (from, to, t0, t1) in spans {
        let rising = t1 > t0;
        if !rising {
            return Err(Stop::of(NEAR).at(&named));
        }
        // In, on or out per face, by the middle; a stretch along a
        // boundary must be on it at its quarter points as well.
        let at = |k: f64| curve.eval(t0 + (t1 - t0) * k);
        let mut inside = [false, false];
        let mut keep = true;
        for (outline, inside) in [oa, ob].into_iter().zip(inside.iter_mut()) {
            let side = |p: Vec3| {
                side_of(&outline.surface, &outline.loops, p, tol)
                    .map_err(|r| Stop::of(r).at(&named))
            };
            let middle = side(at(0.5))?;
            let quarters = [side(at(0.25))?, side(at(0.75))?];
            match middle {
                Side::Out => keep = false,
                Side::In => *inside = true,
                Side::On => {}
            }
            if middle != Side::Out && quarters.iter().any(|&q| q != middle) {
                return Err(Stop::of(AMBIGUOUS).at(&named));
            }
        }
        if !keep {
            continue;
        }
        let start = vertices.find_or_add(from.point).map_err(|s| s.at(&named))?;
        let end = vertices.find_or_add(to.point).map_err(|s| s.at(&named))?;
        let whole = closed && cuts.len() <= 1;
        if start == end && !whole {
            return Err(Stop::of(NEAR).at(&named));
        }
        for (vertex, cut) in [(start, &from), (end, &to)] {
            let meeting = vertices.meetings.get_mut(vertex).ok_or(Stop::of(LOST))?;
            meeting.a_edge = meeting.a_edge.or(cut.a_edge);
            meeting.b_edge = meeting.b_edge.or(cut.b_edge);
            meeting.a_face = meeting.a_face.or(Some(face_id(fa)));
            meeting.b_face = meeting.b_face.or(Some(face_id(fb)));
            // The edges of both faces this vertex is on, away from their
            // ends, are split there.
            let Some(&point) = vertices.points.get(vertex) else {
                return Err(Stop::of(LOST));
            };
            for (solid, face, splits) in [
                (a, fa, &mut sections.splits_a),
                (b, fb, &mut sections.splits_b),
            ] {
                for id in solid.edges_of(face) {
                    let (Some(edge), Some(ends)) = (solid.body.edge(id), solid.ends(id)) else {
                        return Err(Stop::of(LOST));
                    };
                    if vertex != ends.0
                        && vertex != ends.1
                        && locate(&edge.curve, edge.range, point, tol).is_some()
                    {
                        splits.entry(id).or_default().insert(vertex);
                    }
                }
            }
        }
        sections.stretches.push(Stretch {
            curve: curve.clone(),
            range: (t0, t1),
            start,
            end,
            fa,
            fb,
            inside_a: inside.first().copied().unwrap_or(false),
            inside_b: inside.get(1).copied().unwrap_or(false),
        });
    }
    Ok(())
}

fn section(
    a: &Solid,
    b: &Solid,
    vertices: &mut Vertices,
    tolerance: &Tolerance,
) -> Result<Sections, Stop> {
    let tol = tolerance.linear;
    let mut sections = Sections::default();
    for (fa, (oa, box_a)) in a.outlines.iter().zip(&a.boxes).enumerate() {
        for (fb, (ob, box_b)) in b.outlines.iter().zip(&b.boxes).enumerate() {
            if !boxes_meet(box_a, box_b, 2.0 * tol) {
                continue;
            }
            let named = [a.name(fa), b.name(fb)];
            match intersect_surfaces(&oa.surface, &ob.surface, tolerance)
                .map_err(|e| geometry(e, &named))?
            {
                SurfaceIntersection::None => {}
                SurfaceIntersection::Coincident { same_normal } => {
                    sections
                        .coincident_a
                        .entry(fa)
                        .or_default()
                        .push((fb, same_normal));
                    sections
                        .coincident_b
                        .entry(fb)
                        .or_default()
                        .push((fa, same_normal));
                }
                SurfaceIntersection::Tangent(curves) => {
                    // Refused when the line of contact may reach both
                    // faces.
                    for curve in curves {
                        let reaches = match &curve {
                            Curve3::Line(l) => [box_a, box_b]
                                .iter()
                                .all(|b| line_meets_box(l.origin(), l.direction(), b, 2.0 * tol)),
                            _ => true,
                        };
                        if reaches {
                            return Err(Stop::of(TANGENT_SURFACES).at(&named));
                        }
                    }
                }
                SurfaceIntersection::Curves(curves) => {
                    for curve in curves {
                        clip(&curve, a, fa, b, fb, vertices, tolerance, &mut sections)?;
                    }
                }
            }
        }
    }
    Ok(sections)
}

/// Splits every edge of `solid` at its vertices and registers the pieces.
/// Per edge: its pieces, in the edge's direction.
fn split_edges(
    solid: &Solid,
    splits: &BTreeMap<EdgeId, BTreeSet<usize>>,
    vertices: &Vertices,
    arcs: &mut Arcs,
    tol: f64,
) -> Result<Vec<Vec<Half>>, Stop> {
    let mut pieces = Vec::with_capacity(solid.body.edges.len());
    for (index, edge) in solid.body.edges.iter().enumerate() {
        let id = EdgeId(u32::try_from(index).unwrap_or(u32::MAX));
        let (start, end) = solid.ends(id).ok_or(Stop::of(LOST))?;
        let mut stops = Vec::new();
        for &vertex in splits.get(&id).into_iter().flatten() {
            let point = *vertices.points.get(vertex).ok_or(Stop::of(LOST))?;
            let t = locate(&edge.curve, edge.range, point, tol).ok_or(Stop::of(INCONSISTENT))?;
            stops.push((t, vertex));
        }
        stops.sort_by(|p, q| p.0.total_cmp(&q.0));
        stops.insert(0, (edge.range.0, start));
        stops.push((edge.range.1, end));
        let mut halves = Vec::with_capacity(stops.len());
        for (from, to) in stops.iter().zip(stops.iter().skip(1)) {
            let rising = to.0 > from.0;
            if !rising {
                return Err(Stop::of(NEAR));
            }
            halves.push(arcs.find_or_add(
                Arc {
                    curve: edge.curve.clone(),
                    range: (from.0, to.0),
                    start: from.1,
                    end: to.1,
                    source: EdgeSource::Kept {
                        operand: solid.operand,
                        edge: id,
                    },
                },
                tol,
            ));
        }
        pieces.push(halves);
    }
    Ok(pieces)
}

/// The boundary of a face in arcs: per loop, the uses of the pieces of its
/// edges in the loop's direction.
fn boundary(solid: &Solid, face: usize, pieces: &[Vec<Half>]) -> Result<Vec<Vec<Half>>, Stop> {
    let f = solid.body.faces.get(face).ok_or(Stop::of(LOST))?;
    let mut loops = Vec::with_capacity(f.loops.len());
    for loop_ in f.loops.iter().filter_map(|&l| solid.body.loop_(l)) {
        let mut halves = Vec::new();
        for &Coedge { edge, reversed } in &loop_.coedges {
            let of_edge = pieces.get(edge.0 as usize).ok_or(Stop::of(LOST))?;
            if reversed {
                halves.extend(of_edge.iter().rev().map(|&(arc, flipped)| (arc, !flipped)));
            } else {
                halves.extend(of_edge.iter().copied());
            }
        }
        loops.push(halves);
    }
    Ok(loops)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    In,
    Out,
    /// On a face of the other solid whose outward normal is the same.
    OnSame,
    /// On a face of the other solid that faces the other way.
    OnOpposite,
}

/// Directions for the rays of [`inside`]: none along an axis or a diagonal.
const RAYS: [(f64, f64, f64); 12] = [
    (0.577, 0.211, 0.789),
    (-0.343, 0.871, 0.351),
    (0.129, -0.497, 0.858),
    (-0.811, -0.263, 0.523),
    (0.657, 0.731, -0.183),
    (0.279, -0.913, -0.297),
    (-0.593, 0.437, -0.677),
    (0.913, -0.151, 0.379),
    (-0.117, 0.337, -0.934),
    (0.453, 0.619, 0.641),
    (-0.767, -0.587, -0.259),
    (0.211, -0.143, -0.967),
];

/// True when `p`, which is not on its boundary, is inside `solid`: a ray
/// from it crosses the boundary an odd number of times. A ray that meets
/// an edge, a vertex, or a face tangentially is abandoned for the next.
fn inside(solid: &Solid, p: Vec3, tolerance: &Tolerance) -> Result<bool, Stop> {
    let tol = tolerance.linear;
    'rays: for (x, y, z) in RAYS {
        let Ok(line) = Line::new(p, Vec3::new(x, y, z)) else {
            continue;
        };
        let ray = Curve3::Line(line);
        let mut crossings = 0_usize;
        for (face, (outline, box_)) in solid.outlines.iter().zip(&solid.boxes).enumerate() {
            if !line_meets_box(line.origin(), line.direction(), box_, 2.0 * tol) {
                continue;
            }
            let named = [solid.name(face)];
            let side = |q: Vec3| {
                side_of(&outline.surface, &outline.loops, q, tol)
                    .map_err(|r| Stop::of(r).at(&named))
            };
            let hits = match intersect_curve_surface(&ray, &outline.surface, tolerance)
                .map_err(|e| geometry(e, &named))?
            {
                CurveIntersection::OnSurface => continue 'rays,
                CurveIntersection::Hits(hits) => hits,
            };
            for hit in hits {
                if hit.t.abs() <= tol {
                    // The point is on this surface: fine when the face is
                    // elsewhere on it.
                    if side(p)? != Side::Out {
                        return Err(Stop::of(ON_A_FACE).at(&named));
                    }
                    continue;
                }
                if hit.t < 0.0 || !box_.contains(hit.point, 2.0 * tol) {
                    continue;
                }
                match side(hit.point)? {
                    Side::Out => {}
                    Side::On => continue 'rays,
                    Side::In if hit.tangent => continue 'rays,
                    Side::In => crossings += 1,
                }
            }
        }
        return Ok(crossings % 2 == 1);
    }
    Err(Stop::of(NO_RAY))
}

/// Where a region of a face of `own` is with respect to `other`.
fn classify(
    region: &Outline,
    other: &Solid,
    coincident: &[(usize, bool)],
    tolerance: &Tolerance,
) -> Result<Class, Stop> {
    let tol = tolerance.linear;
    let p = interior_point(region, tol, 4.0 * tol).map_err(Stop::of)?;
    for &(face, same_normal) in coincident {
        let outline = other.outlines.get(face).ok_or(Stop::of(LOST))?;
        match side_of(&outline.surface, &outline.loops, p, tol)
            .map_err(|r| Stop::of(r).at(&[other.name(face)]))?
        {
            Side::Out => {}
            Side::On => return Err(Stop::of(ON_A_FACE).at(&[other.name(face)])),
            Side::In => {
                let same = same_normal == (region.reversed == outline.reversed);
                return Ok(if same {
                    Class::OnSame
                } else {
                    Class::OnOpposite
                });
            }
        }
    }
    Ok(if inside(other, p, tolerance)? {
        Class::In
    } else {
        Class::Out
    })
}

/// A region of a face of an operand that becomes a face of the result.
struct Kept {
    source: FaceSource,
    surface: Surface,
    reversed: bool,
    loops: Vec<Vec<Half>>,
}

/// The regions of every face of `own`, classified against `other`, and of
/// those the ones the operation keeps.
#[allow(clippy::too_many_arguments)]
fn select(
    own: &Solid,
    other: &Solid,
    pieces: &[Vec<Half>],
    interior: &BTreeMap<usize, BTreeSet<usize>>,
    coincident: &BTreeMap<usize, Vec<(usize, bool)>>,
    arcs: &Arcs,
    vertices: &Vertices,
    op: BooleanOp,
    tolerance: &Tolerance,
    kept: &mut Vec<Kept>,
) -> Result<(), Stop> {
    let tol = tolerance.linear;
    let none = BTreeSet::new();
    for (face, outline) in own.outlines.iter().enumerate() {
        let named = [own.name(face)];
        let loops = boundary(own, face, pieces)?;
        let across = interior.get(&face).unwrap_or(&none);
        let whole = across.is_empty()
            && own.edges_of(face).iter().all(|edge| {
                pieces
                    .get(edge.0 as usize)
                    .is_some_and(|halves| halves.len() == 1)
            });
        let regions = if whole {
            vec![loops]
        } else {
            regions::build(
                &outline.surface,
                outline.reversed,
                &loops,
                across,
                &arcs.arcs,
                &vertices.points,
                tol,
            )
            .map_err(|r| Stop::of(r).at(&named))?
        };
        for loops in regions {
            let mut segs = Vec::with_capacity(loops.len());
            for loop_ in &loops {
                segs.push(
                    regions::segs(loop_, &arcs.arcs, &vertices.points)
                        .map_err(|r| Stop::of(r).at(&named))?,
                );
            }
            let region = Outline {
                surface: outline.surface.clone(),
                reversed: outline.reversed,
                loops: segs,
            };
            let coincident = coincident.get(&face).map_or(&[][..], Vec::as_slice);
            let class =
                classify(&region, other, coincident, tolerance).map_err(|s| s.at(&named))?;
            // Of two coincident regions with the same normal A's is the
            // one kept; with opposite normals the solids lie on either
            // side and only A minus B has a face there, A's.
            let (keep, flip) = match (op, own.operand, class) {
                (BooleanOp::Union, _, Class::Out) => (true, false),
                (BooleanOp::Union, Operand::A, Class::OnSame) => (true, false),
                (BooleanOp::Intersect, _, Class::In) => (true, false),
                (BooleanOp::Intersect, Operand::A, Class::OnSame) => (true, false),
                (BooleanOp::Subtract, Operand::A, Class::Out | Class::OnOpposite) => (true, false),
                (BooleanOp::Subtract, Operand::B, Class::In) => (true, true),
                _ => (false, false),
            };
            if !keep {
                continue;
            }
            let loops = if flip {
                loops
                    .into_iter()
                    .map(|l| l.into_iter().rev().map(|(arc, r)| (arc, !r)).collect())
                    .collect()
            } else {
                loops
            };
            kept.push(Kept {
                source: FaceSource {
                    operand: own.operand,
                    face: face_id(face),
                },
                surface: outline.surface.clone(),
                reversed: outline.reversed != flip,
                loops,
            });
        }
    }
    Ok(())
}

/// The root of `i` in a union-find forest, halving paths on the way.
fn root(parents: &mut [usize], mut i: usize) -> usize {
    for _ in 0..parents.len() {
        let Some(&parent) = parents.get(i) else {
            break;
        };
        if parent == i {
            break;
        }
        let grand = parents.get(parent).copied().unwrap_or(parent);
        if let Some(slot) = parents.get_mut(i) {
            *slot = grand;
        }
        i = parent;
    }
    i
}

fn sew(kept: Vec<Kept>, arcs: &Arcs, vertices: &Vertices) -> Result<BooleanResult, Stop> {
    let mut body = Body::new(BodyKind::Solid);
    let mut result_vertices: BTreeMap<usize, VertexId> = BTreeMap::new();
    let mut result_edges: BTreeMap<usize, EdgeId> = BTreeMap::new();
    let mut faces = Vec::with_capacity(kept.len());
    let mut edges = Vec::new();
    let mut vertex_sources = Vec::new();
    let mut counts: BTreeMap<(Entity, Entity), u32> = BTreeMap::new();
    // Per arc: how often it is used, and the first face that used it.
    let mut uses: BTreeMap<usize, (usize, usize)> = BTreeMap::new();
    let mut parents: Vec<usize> = (0..kept.len()).collect();

    for (index, region) in kept.into_iter().enumerate() {
        let mut loops = Vec::with_capacity(region.loops.len());
        for loop_ in region.loops {
            let mut coedges = Vec::with_capacity(loop_.len());
            for (arc_index, reversed) in loop_ {
                let arc = arcs.arcs.get(arc_index).ok_or(Stop::of(LOST))?;
                let mut vertex = |at: usize| -> Result<VertexId, Stop> {
                    if let Some(&id) = result_vertices.get(&at) {
                        return Ok(id);
                    }
                    let (Some(&point), Some(meeting)) =
                        (vertices.points.get(at), vertices.meetings.get(at))
                    else {
                        return Err(Stop::of(LOST));
                    };
                    let source = match meeting.kept {
                        Some((operand, vertex)) => VertexSource::Kept { operand, vertex },
                        None => {
                            let a = meeting
                                .a_edge
                                .map(Entity::Edge)
                                .or(meeting.a_face.map(Entity::Face));
                            let b = meeting
                                .b_edge
                                .map(Entity::Edge)
                                .or(meeting.b_face.map(Entity::Face));
                            let (Some(a), Some(b)) = (a, b) else {
                                return Err(Stop::of(LOST));
                            };
                            let count = counts.entry((a, b)).or_insert(0);
                            let index = *count;
                            *count = count.saturating_add(1);
                            VertexSource::New { a, b, index }
                        }
                    };
                    let id = body.add_vertex(point);
                    vertex_sources.push(source);
                    result_vertices.insert(at, id);
                    Ok(id)
                };
                let edge = match result_edges.get(&arc_index) {
                    Some(&edge) => edge,
                    None => {
                        let (start, end) = (vertex(arc.start)?, vertex(arc.end)?);
                        let edge = body.add_edge(arc.curve.clone(), arc.range, start, end);
                        edges.push(arc.source);
                        result_edges.insert(arc_index, edge);
                        edge
                    }
                };
                coedges.push(Coedge { edge, reversed });
                let entry = uses.entry(arc_index).or_insert((0, index));
                entry.0 += 1;
                if entry.0 > 2 {
                    return Err(Stop::of(NON_MANIFOLD));
                }
                let (first, here) = (root(&mut parents, entry.1), root(&mut parents, index));
                if let Some(slot) = parents.get_mut(first.max(here)) {
                    *slot = first.min(here);
                }
            }
            loops.push(body.add_loop(coedges));
        }
        body.add_face(region.surface, region.reversed, loops);
        faces.push(region.source);
    }

    // Shells: faces connected through edges, in the order of their first
    // face.
    let mut shells: BTreeMap<usize, Vec<FaceId>> = BTreeMap::new();
    for index in 0..faces.len() {
        let shell = root(&mut parents, index);
        shells.entry(shell).or_default().push(face_id(index));
    }
    for (_, shell) in shells {
        body.add_shell(shell);
    }
    Ok(BooleanResult {
        body,
        faces,
        edges,
        vertices: vertex_sources,
    })
}

fn run(a: &Body, b: &Body, op: BooleanOp, tolerance: &Tolerance) -> Result<BooleanResult, Stop> {
    let tol = tolerance.linear;
    if !(tol.is_finite() && tol > 0.0) {
        return Err(Stop::Error(KernelError::InvalidSize {
            name: "tolerance",
            value: tol,
        }));
    }
    if !(tolerance.angular.is_finite() && tolerance.angular > 0.0) {
        return Err(Stop::Error(KernelError::InvalidSize {
            name: "angular tolerance",
            value: tolerance.angular,
        }));
    }
    let mut vertices = Vertices {
        points: Vec::new(),
        meetings: Vec::new(),
        merge: 0.5 * tol,
        guard: 2.0 * tol,
    };
    let a = prepare(a, Operand::A, &mut vertices, tolerance)?;
    let b = prepare(b, Operand::B, &mut vertices, tolerance)?;
    let sections = section(&a, &b, &mut vertices, tolerance)?;

    let mut arcs = Arcs::default();
    let pieces_a = split_edges(&a, &sections.splits_a, &vertices, &mut arcs, tol)?;
    let pieces_b = split_edges(&b, &sections.splits_b, &vertices, &mut arcs, tol)?;

    // The arcs along each face's boundary, to tell a stretch across a face
    // from one along its edge.
    let rims = |solid: &Solid, pieces: &[Vec<Half>]| -> Vec<BTreeSet<usize>> {
        (0..solid.outlines.len())
            .map(|face| {
                solid
                    .edges_of(face)
                    .iter()
                    .filter_map(|edge| pieces.get(edge.0 as usize))
                    .flatten()
                    .map(|&(arc, _)| arc)
                    .collect()
            })
            .collect()
    };
    let (rims_a, rims_b) = (rims(&a, &pieces_a), rims(&b, &pieces_b));
    let mut interior_a: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    let mut interior_b: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for stretch in &sections.stretches {
        let named = [a.name(stretch.fa), b.name(stretch.fb)];
        let (arc, _) = arcs.find_or_add(
            Arc {
                curve: stretch.curve.clone(),
                range: stretch.range,
                start: stretch.start,
                end: stretch.end,
                source: EdgeSource::Intersection {
                    a: face_id(stretch.fa),
                    b: face_id(stretch.fb),
                },
            },
            tol,
        );
        for (face, inside, rims, interior) in [
            (stretch.fa, stretch.inside_a, &rims_a, &mut interior_a),
            (stretch.fb, stretch.inside_b, &rims_b, &mut interior_b),
        ] {
            let on_rim = rims.get(face).is_some_and(|rim| rim.contains(&arc));
            if on_rim == inside {
                return Err(Stop::of(INCONSISTENT).at(&named));
            }
            if inside {
                interior.entry(face).or_default().insert(arc);
            }
        }
    }

    let mut kept = Vec::new();
    select(
        &a,
        &b,
        &pieces_a,
        &interior_a,
        &sections.coincident_a,
        &arcs,
        &vertices,
        op,
        tolerance,
        &mut kept,
    )?;
    select(
        &b,
        &a,
        &pieces_b,
        &interior_b,
        &sections.coincident_b,
        &arcs,
        &vertices,
        op,
        tolerance,
        &mut kept,
    )?;
    if kept.is_empty() {
        return Err(Stop::Error(KernelError::EmptyResult {
            operation: OPERATION,
        }));
    }
    let result = sew(kept, &arcs, &vertices)?;
    let mut defects = check(&result.body, tolerance);
    if !defects.is_empty() {
        defects.truncate(MAX_REPORTED);
        return Err(Stop::Error(KernelError::InvalidBody {
            operation: OPERATION,
            defects,
        }));
    }
    Ok(result)
}

/// The union, difference (`a` minus `b`) or intersection of two solids.
///
/// The result has passed `craft3d_topo::check` with `tolerance`. Its faces
/// come first from A, in the order of A's faces, then from B; edges and
/// vertices are numbered in the order the faces use them. Faces connected
/// through edges form a shell, so a union of two bodies apart, a difference
/// that falls into two parts, and a difference that leaves a cavity all
/// have several shells. The same input gives the same result, bit for bit.
///
/// **Coincident faces.** Where a face of A and a face of B lie on the same
/// surface with the same outward normal, the union and the intersection
/// keep A's piece (the face's source is A's face) and drop B's; the
/// difference has no face there. Where they face each other, only the
/// difference has a face there, A's. Faces are never merged: where a face
/// of A continues into a coplanar face of B the result has two faces with
/// an edge between them, and an imprint that separates two kept pieces of
/// one face stays as an edge.
///
/// **Errors.** [`KernelError::EmptyResult`] when nothing is left;
/// [`KernelError::Unsupported`] for geometry or contact that is not
/// handled (see the module documentation), with the faces involved;
/// [`KernelError::InvalidBody`] when an operand, or (a bug, or input at the
/// edge of tolerance) the result fails the checker.
pub fn boolean(
    a: &Body,
    b: &Body,
    op: BooleanOp,
    tolerance: &Tolerance,
) -> Result<BooleanResult, KernelError> {
    run(a, b, op, tolerance).map_err(|stop| match stop {
        Stop::Error(error) => error,
        Stop::Unsupported { reason, mut faces } => {
            faces.truncate(MAX_REPORTED);
            KernelError::Unsupported {
                operation: OPERATION,
                reason,
                faces,
            }
        }
    })
}
