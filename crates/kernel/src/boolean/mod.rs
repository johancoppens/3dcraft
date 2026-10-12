//! Booleans: the union, difference and intersection of two solids.
//!
//! Supported: solids whose faces are planes, cylinders, cones, spheres and
//! tori (a whole sphere or torus without any edge included) and whose
//! edges are lines, circles, ellipses and splines, meeting in curves that
//! `meet_surfaces` can give. Coincident faces, faces that touch without
//! crossing, and bodies that touch along an edge or at a vertex are
//! handled (see [`boolean`]). What is left is
//! [`KernelError::Unsupported`] with the faces and the reason: freeform
//! faces, pairs of surfaces whose intersection is not available, and input
//! at the edge of the tolerance. A result is never guessed and never
//! returned without passing `craft3d_topo::check`.
//!
//! How it works (the classical imprint, classify, select and sew):
//!
//! 1. **Section** ([`section`]). For every face of A and face of B whose
//!    boxes meet, the curves in which their surfaces meet are asked for
//!    ([`meet`]). Each curve is cut where an edge of either face meets the
//!    other face's surface ([`hits`]); a stretch between two cuts is kept
//!    when its middle is in or on both faces. Its ends become vertices:
//!    points closer than half the linear tolerance are one vertex, points
//!    between that and twice the tolerance are refused.
//! 2. **Imprint.** Every edge of A and B is split at the vertices found on
//!    it, and the pieces are registered as *arcs*; so are the stretches of
//!    step 1. Two arcs between the same vertices along the same curve are
//!    one arc, which is how an edge of B lying on a face of A, or along an
//!    edge of A, becomes shared.
//! 3. **Regions.** A face with arcs across it falls into regions
//!    ([`regions`]).
//! 4. **Classify** ([`classify`]). One point well inside each region
//!    decides for the whole region: on a face of the other solid with a
//!    coincident surface (with the same or the opposite outward normal),
//!    or else inside or outside the other solid by casting a ray and
//!    counting crossings. Inside or outside a face is a winding number in
//!    a chart of its surface ([`chart`]), whatever the surface.
//! 5. **Select.** Union keeps what is outside the other solid, intersection
//!    what is inside, difference the outside of A and the inside of B, the
//!    latter turned inside out. Of two coincident regions at most A's is
//!    kept.
//! 6. **Sew** ([`sew`]). The kept regions become the faces of the result;
//!    arcs become its edges. They already share arcs, so nothing is matched
//!    by position here. Where more than two faces meet along an arc, or
//!    several fans of faces at a vertex, the arc or vertex is duplicated
//!    so that the result is a manifold whose parts touch.
//! 7. **Merge** ([`merge`]). Faces that the operation split, or that
//!    continue each other across the two operands, become one face again;
//!    edges and vertices that only the imprint needed go.

mod chart;
mod classify;
mod hits;
mod merge;
mod outline;
mod regions;
mod section;
mod sew;

use crate::{KernelError, MAX_REPORTED};
use craft3d_geom::{Curve3, GeomError};
use craft3d_math::{Tolerance, Vec3};
use craft3d_topo::{Body, EdgeId, FaceId, VertexId, check};
use outline::locate;
use regions::Half;
use section::Solid;
use std::collections::{BTreeMap, BTreeSet};

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
/// source, when a face was cut into pieces that are apart.
///
/// A face that was merged from pieces of several faces (where a face of A
/// continues into a face of B on the same surface) has the lowest-numbered
/// of A's faces among them as its source, or else the lowest-numbered of
/// B's; its surface is that face's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FaceSource {
    pub operand: Operand,
    pub face: FaceId,
}

/// Where an edge of the result came from. An edge that was merged from
/// several (two collinear edges whose common vertex nothing else needs)
/// has the least of their sources in this type's order: an edge of A
/// before an edge of B before an intersection.
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
    /// Where parts of the result only touch at it, each part has its own
    /// vertex there, with the same source.
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

/// How a boolean is carried out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BooleanOptions {
    /// Merge the faces, edges and vertices that the operation split
    /// needlessly (see [`boolean`]). Without it every imprint stays; the
    /// result is the same solid with more entities.
    pub merge: bool,
}

impl Default for BooleanOptions {
    fn default() -> Self {
        BooleanOptions { merge: true }
    }
}

/// Why something is refused, as a fixed text: tests count refusals by it.
pub(crate) type Reason = &'static str;

const OPERATION: &str = "boolean";
pub(crate) const NEAR: Reason =
    "two points closer together than the tolerance can tell apart (at the edge of tolerance)";
const NEARLY_COINCIDENT: Reason = "two faces between half and twice the tolerance apart: neither the same surface nor safely two (at the edge of tolerance)";
const AMBIGUOUS: Reason =
    "a curve that is partly on and partly off a face boundary (at the edge of tolerance)";
const INCONSISTENT: Reason = "an imprinted curve that does not fit the edges it should coincide with (at the edge of tolerance)";
const NON_MANIFOLD: Reason = "the result would have an edge shared by faces that do not pair up around it (at the edge of tolerance)";
const TANGENT_EDGE: Reason = "more than two faces meet along an edge and two of them are tangent there (tangent contact along an edge)";
const ON_A_FACE: Reason = "a piece of a face lies within tolerance of a face it is not coincident with (at the edge of tolerance)";
const NO_RAY: Reason = "could not tell inside from outside: every ray tried grazes the other body";
const LOST: Reason = "the boolean lost track of an entity; this is a bug";

#[derive(Debug)]
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
        // What geometry cannot give, or could not find: refused, with its
        // reason.
        GeomError::Unsupported(reason) | GeomError::NoConvergence(reason) => Stop::Unsupported {
            reason,
            faces: faces.to_vec(),
        },
        other => Stop::Error(KernelError::Geometry(other)),
    }
}

fn face_id(index: usize) -> FaceId {
    FaceId(u32::try_from(index).unwrap_or(u32::MAX))
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

fn run(
    a: &Body,
    b: &Body,
    op: BooleanOp,
    tolerance: &Tolerance,
    options: &BooleanOptions,
) -> Result<BooleanResult, Stop> {
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
    let a = section::prepare(a, Operand::A, &mut vertices, tolerance)?;
    let b = section::prepare(b, Operand::B, &mut vertices, tolerance)?;
    let sections = section::section(&a, &b, &mut vertices, tolerance)?;

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
    classify::select(
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
    classify::select(
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
    let mut work = sew::sew(kept, &arcs, &vertices)?;
    if options.merge {
        merge::merge(&mut work, &sections.coincident_a, a.body, b.body, tol)?;
    }
    let result = sew::finish(work)?;
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
/// difference has a face there, A's.
///
/// **Contact without overlap.** Bodies that only touch (a face against a
/// face along a line or at a point, as a cylinder lying on a plate or a
/// ball standing on it; an edge against a face or an edge; a vertex
/// against anything) have no volume in common. Their union is both bodies
/// in one body, as shells that share no vertex and no edge, however they
/// touch; their intersection is [`KernelError::EmptyResult`]; A minus B is
/// A. In the same way a result whose parts would hang together only
/// through an edge or a vertex (a difference that leaves two blocks
/// joined along an edge) has that edge or vertex once per part: every
/// edge of a result is shared by exactly two faces.
///
/// **Merging.** Faces of the result that lie on one surface, face the same
/// way, share an edge, and are pieces of one face of an operand or pieces
/// of faces of A and of B that lie on that surface, are one face: the
/// union of two boxes that are flush is a box with six faces, and an
/// imprint that does not separate what is kept from what is not leaves no
/// trace. A vertex left with two edges along one curve, which the result
/// does not need, is removed and its edges joined, unless the operand it
/// came from had it like that. See [`FaceSource`] and [`EdgeSource`] for
/// the source of what was merged. [`boolean_with`] can leave this step
/// out.
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
    boolean_with(a, b, op, tolerance, &BooleanOptions::default())
}

/// [`boolean`] with options.
pub fn boolean_with(
    a: &Body,
    b: &Body,
    op: BooleanOp,
    tolerance: &Tolerance,
    options: &BooleanOptions,
) -> Result<BooleanResult, KernelError> {
    run(a, b, op, tolerance, options).map_err(|stop| match stop {
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

/// Whether each of `points` is inside the solid `body`: `Some(true)`
/// inside, `Some(false)` outside, `None` for a point within twice the
/// tolerance of its boundary or one for which no ray could tell.
///
/// This is the classification the booleans use (a ray from the point,
/// counting the faces it crosses), applied to any solid they accept as an
/// operand. It does not use `craft3d_topo::mass_properties`, which makes a
/// count of sampled points a measure of a body's volume that is
/// independent of it.
pub fn contains_points(
    body: &Body,
    points: &[Vec3],
    tolerance: &Tolerance,
) -> Result<Vec<Option<bool>>, KernelError> {
    let unsupported = |stop| match stop {
        Stop::Error(error) => error,
        Stop::Unsupported { reason, mut faces } => {
            faces.truncate(MAX_REPORTED);
            KernelError::Unsupported {
                operation: "point classification",
                reason,
                faces,
            }
        }
    };
    let tol = tolerance.linear;
    if !(tol.is_finite() && tol > 0.0 && tolerance.angular.is_finite() && tolerance.angular > 0.0) {
        return Err(KernelError::InvalidSize {
            name: "tolerance",
            value: tol,
        });
    }
    // The vertices are not merged with anything here.
    let mut vertices = Vertices {
        points: Vec::new(),
        meetings: Vec::new(),
        merge: 0.0,
        guard: 0.0,
    };
    let solid =
        section::prepare(body, Operand::A, &mut vertices, tolerance).map_err(unsupported)?;
    let mut found = Vec::with_capacity(points.len());
    for &point in points {
        found.push(match classify::inside(&solid, point, tolerance) {
            Ok(inside) => Some(inside),
            Err(Stop::Unsupported { .. }) => None,
            Err(stop) => return Err(unsupported(stop)),
        });
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn points_within_half_the_tolerance_are_one_vertex() {
        let mut vertices = Vertices {
            points: Vec::new(),
            meetings: Vec::new(),
            merge: 0.5e-6,
            guard: 2e-6,
        };
        let mut add = |x: f64| vertices.find_or_add(Vec3::new(x, 0.0, 0.0)).ok();
        assert_eq!(add(0.0), Some(0));
        assert_eq!(add(1.0), Some(1));
        assert_eq!(add(0.4e-6), Some(0));
        assert_eq!(add(1.0 - 0.4e-6), Some(1));
        // Neither the same place nor safely another.
        assert_eq!(add(1e-6), None);
        assert_eq!(add(1.0 + 1.9e-6), None);
        assert_eq!(add(2.1e-6), Some(2));
        assert_eq!(add(f64::NAN), None);
    }
}
