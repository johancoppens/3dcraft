//! Booleans: the union, difference and intersection of two solids.
//!
//! Supported: solids whose faces are planes, cylinders, cones, spheres and
//! tori (a whole sphere or torus without any edge included) and whose
//! edges are lines, circles, ellipses and splines, meeting in curves that
//! `meet_surfaces` can give. Coincident faces, faces that touch without
//! crossing, and bodies that touch along an edge or at a vertex are
//! handled (see [`boolean`]; a result whose parts touch is refused as zero
//! thickness). What is left is
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
mod freeform;
mod hits;
mod merge;
mod outline;
mod regions;
mod section;
mod sew;

use crate::{KernelError, MAX_REPORTED};
use craft3d_geom::{Curve3, GeomError};
use craft3d_math::{Bits, Tolerance, Vec3};
use craft3d_topo::{Body, EdgeId, FaceId, VertexId, check};
use outline::locate;
use regions::Half;
use section::Solid;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, PoisonError};

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
    /// Per face, every face of the operands it has a piece of: one for a
    /// face that is a piece of one face, several for a face merged from
    /// pieces of several (the faces of A first, each list in order and
    /// without repeats). `faces[i]` is `merged[i][0]`.
    pub merged: Vec<Vec<FaceSource>>,
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
    /// Skip the validity check of the operands. Only for bodies that a
    /// kernel operation returned and that nothing has changed since: such
    /// a body passed the check when it was made, and checking it again on
    /// every one of a long row of booleans costs more than the booleans.
    /// With an operand that would not pass, the result is an error or a
    /// body that fails the check at the end, never a panic.
    pub trust_operands: bool,
    /// Refuse a result whose parts touch each other along an edge or in a
    /// point ([`KernelError::ZeroThickness`]). Only an operation that
    /// goes on with such a body as a step of its own, not as its result,
    /// leaves this out.
    pub refuse_zero_thickness: bool,
}

impl Default for BooleanOptions {
    fn default() -> Self {
        BooleanOptions {
            merge: true,
            trust_operands: false,
            refuse_zero_thickness: true,
        }
    }
}

/// Why something is refused, as a fixed text: tests count refusals by it.
pub(crate) type Reason = &'static str;

const OPERATION: &str = "boolean";
pub(crate) const NEAR: Reason =
    "two points closer together than the tolerance can tell apart (at the edge of tolerance)";
const NEARLY_COINCIDENT: Reason = "two faces between half and twice the tolerance apart: neither the same surface nor safely two (at the edge of tolerance)";
const NEARLY_TOUCHING: Reason = "two faces that go through each other by between half and twice the tolerance: neither touching nor safely crossing (at the edge of tolerance)";
const AMBIGUOUS: Reason =
    "a curve that is partly on and partly off a face boundary (at the edge of tolerance)";
const INCONSISTENT: Reason = "an imprinted curve that does not fit the edges it should coincide with (at the edge of tolerance)";
const NON_MANIFOLD: Reason = "the result would have an edge shared by faces that do not pair up around it (at the edge of tolerance)";
const TANGENT_EDGE: Reason = "more than two faces meet along an edge and two of them are tangent there (tangent contact along an edge)";
const ON_A_FACE: Reason = "a piece of a face lies within tolerance of a face it is not coincident with (at the edge of tolerance)";
const NO_RAY: Reason = "could not tell inside from outside: every ray tried grazes the other body";
const LOST: Reason = "the boolean lost track of an entity; this is a bug";
/// Two faces pass through each other along a curve, and the curve does not
/// divide one of them: a piece of it is missing (a place where an edge
/// meets a surface was not found, or a curve was cut short).
const IMPRINT_LOST: Reason =
    "imprint lost: a curve in which two faces cross does not divide them; a piece of it is missing";

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

/// Bodies that passed `craft3d_topo::check` lately, each written out in
/// full with the tolerance it was checked with, every bit of every number
/// (`craft3d_math::Bits`: two are the same only when every number of them
/// is, bit for bit), the most recent last.
static PASSED: Mutex<Vec<Vec<u64>>> = Mutex::new(Vec::new());
/// How many are remembered, and how many words all of them are at most.
const PASSED_BODIES: usize = 8;
const PASSED_WORDS: usize = 1 << 22;

/// `craft3d_topo::check` of `body`, remembering the bodies that pass. In a
/// row of booleans each result is the first operand of the next, and is
/// checked as a result and again as an operand; written out, a body is
/// compared at a small part of what checking it costs. Written out as its
/// bits, not as text: formatting every number of a large body cost a
/// quarter of a long row of cuts.
pub(crate) fn checked(body: &Body, tolerance: &Tolerance) -> Vec<craft3d_topo::Defect> {
    let mut key = craft3d_math::bits_of(tolerance);
    body.push_bits(&mut key);
    {
        let mut passed = PASSED.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(at) = passed.iter().position(|k| *k == key) {
            let entry = passed.remove(at);
            passed.push(entry);
            return Vec::new();
        }
    }
    let defects = check(body, tolerance);
    if defects.is_empty() && key.len() <= PASSED_WORDS {
        let mut passed = PASSED.lock().unwrap_or_else(PoisonError::into_inner);
        if !passed.contains(&key) {
            passed.push(key);
        }
        while passed.len() > PASSED_BODIES
            || passed.iter().map(Vec::len).sum::<usize>() > PASSED_WORDS
        {
            let _forgotten = passed.remove(0);
        }
    }
    defects
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
    /// The points by the cell of a grid they are in, so that a point's
    /// neighbours are found without going through all of them. A cell is
    /// wider than the guard: what is within the guard of a point is in its
    /// cell or one next to it.
    cells: BTreeMap<(i64, i64, i64), Vec<usize>>,
}

impl Vertices {
    fn new(merge: f64, guard: f64) -> Vertices {
        Vertices {
            points: Vec::new(),
            meetings: Vec::new(),
            merge,
            guard,
            cells: BTreeMap::new(),
        }
    }

    fn cell(&self, p: Vec3) -> Option<(i64, i64, i64)> {
        // Much wider than the guard, so that coordinates stay well within
        // an integer at any model size.
        let width = 1024.0 * self.guard;
        let index = |x: f64| {
            let cell = (x / width).floor();
            (cell.abs() < 4e18).then_some(cell as i64)
        };
        (width > 0.0)
            .then(|| Some((index(p.x)?, index(p.y)?, index(p.z)?)))
            .flatten()
    }

    fn find_or_add(&mut self, p: Vec3) -> Result<usize, Stop> {
        self.find_or_add_with(p, |_, _| false)
    }

    /// [`Vertices::find_or_add`], where a point further than the merging
    /// distance from the nearest one, but within the guard, is that point
    /// too when `same(q, meeting)` says so of it (it is the same meeting of
    /// the same entities, found by another way).
    fn find_or_add_with(
        &mut self,
        p: Vec3,
        same: impl Fn(Vec3, &Meeting) -> bool,
    ) -> Result<usize, Stop> {
        let mut nearest: Option<(f64, usize)> = None;
        let home = self.cell(p);
        let mut consider = |index: usize, q: Vec3| {
            let d = p.distance(q);
            if d < self.guard
                && nearest.is_none_or(|(least, at)| d < least || (d == least && index < at))
            {
                nearest = Some((d, index));
            }
        };
        match home {
            Some((x, y, z)) => {
                for dx in -1..=1 {
                    for dy in -1..=1 {
                        for dz in -1..=1 {
                            let key = (
                                x.saturating_add(dx),
                                y.saturating_add(dy),
                                z.saturating_add(dz),
                            );
                            for &index in self.cells.get(&key).into_iter().flatten() {
                                if let Some(&q) = self.points.get(index) {
                                    consider(index, q);
                                }
                            }
                        }
                    }
                }
            }
            // No grid (a degenerate tolerance or a coordinate beyond it):
            // every point is looked at.
            None => {
                for (index, &q) in self.points.iter().enumerate() {
                    consider(index, q);
                }
            }
        }
        match nearest {
            Some((d, index)) if d <= self.merge => Ok(index),
            Some((_, index)) => {
                let known = self.points.get(index).zip(self.meetings.get(index));
                if known.is_some_and(|(&q, meeting)| same(q, meeting)) {
                    Ok(index)
                } else {
                    Err(Stop::of(NEAR))
                }
            }
            None => {
                if !p.is_finite() {
                    return Err(Stop::of(LOST));
                }
                self.points.push(p);
                self.meetings.push(Meeting::default());
                let index = self.points.len() - 1;
                if let Some(key) = home {
                    self.cells.entry(key).or_default().push(index);
                }
                Ok(index)
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
    let mut vertices = Vertices::new(0.5 * tol, 2.0 * tol);
    let a = section::prepare(
        a,
        Operand::A,
        &mut vertices,
        tolerance,
        options.trust_operands,
    )?;
    let b = section::prepare(
        b,
        Operand::B,
        &mut vertices,
        tolerance,
        options.trust_operands,
    )?;
    let sections = section::section(&a, &b, &mut vertices, tolerance)?;

    let mut arcs = Arcs::default();
    let pieces_a = split_edges(&a, &sections.splits_a, &vertices, &mut arcs, tol)?;
    let pieces_b = split_edges(&b, &sections.splits_b, &vertices, &mut arcs, tol)?;

    // The arcs along each face's boundary, to tell a stretch across a face
    // from one along its edge.
    let rims = |solid: &Solid, pieces: &[Vec<Half>]| -> Vec<BTreeSet<usize>> {
        (0..solid.faces())
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
    // Per face the arcs along which the other body's face passes through
    // it, strictly inside both: each of them must divide the face.
    let mut crossing_a: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    let mut crossing_b: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
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
        if stretch.crossing && stretch.inside_a && stretch.inside_b {
            crossing_a.entry(stretch.fa).or_default().insert(arc);
            crossing_b.entry(stretch.fb).or_default().insert(arc);
        }
    }

    let mut kept = Vec::new();
    classify::select(
        &a,
        &b,
        &pieces_a,
        &interior_a,
        &crossing_a,
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
        &crossing_b,
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
    let mut defects = checked(&result.body, tolerance);
    if !defects.is_empty() {
        defects.truncate(MAX_REPORTED);
        return Err(Stop::Error(KernelError::InvalidBody {
            operation: OPERATION,
            defects,
        }));
    }
    if options.refuse_zero_thickness && result.body.shells.len() > 1 {
        // Where the operands meet: their vertices, the section's, the
        // points where faces only touch, and a point inside every
        // stretch of curve along which faces meet.
        let mut meetings = vertices.points.clone();
        meetings.extend_from_slice(&sections.touches);
        meetings.extend(
            sections
                .stretches
                .iter()
                .map(|s| s.curve.eval(0.5 * (s.range.0 + s.range.1))),
        );
        zero_thickness(&result, tolerance, &meetings)?;
    }
    Ok(result)
}

/// Refuses a result whose parts (shells) touch each other, which is
/// geometry of zero thickness (the developer's decision of 2026-10-15).
/// Two ways of finding it: a vertex of one part within twice the
/// tolerance of a vertex or an edge of another (`craft3d_topo::touches`:
/// two blocks sharing an edge or a corner); and a point where the
/// operands meet (`meetings`: every vertex of either operand and every
/// vertex the section made, and a point inside every stretch of curve in
/// which their faces meet or touch) that lies on faces of two parts,
/// within twice the tolerance of each face's surface and not outside its
/// boundary (a cylinder lying on a block, a cone's tip standing on it, a
/// ring on it). Parts can only touch where the operands meet, so this
/// sees every contact the section saw.
fn zero_thickness(
    result: &BooleanResult,
    tolerance: &Tolerance,
    meetings: &[Vec3],
) -> Result<(), Stop> {
    let body = &result.body;
    if body.shells.len() < 2 {
        return Ok(());
    }
    let tol = tolerance.linear;
    let near = 2.0 * tol;
    let found = craft3d_topo::touches(body, near, 1, craft3d_topo::TouchScope::BetweenShells)
        .map_err(|error| {
            Stop::Error(KernelError::InvalidBody {
                operation: OPERATION,
                defects: vec![craft3d_topo::Defect::NonFinite {
                    what: error.to_string(),
                }],
            })
        })?;
    let uses = |face: &craft3d_topo::Face, wanted: &dyn Fn(&craft3d_topo::Edge, u32) -> bool| {
        face.loops
            .iter()
            .filter_map(|l| body.loop_(*l))
            .flat_map(|l| &l.coedges)
            .any(|c| body.edge(c.edge).is_some_and(|e| wanted(e, c.edge.0)))
    };
    let at_vertex = |v: u32| move |e: &craft3d_topo::Edge, _: u32| e.start.0 == v || e.end.0 == v;
    let faces_where = |wanted: &dyn Fn(&craft3d_topo::Face) -> bool| -> Vec<usize> {
        (0..body.faces.len())
            .filter(|&f| body.faces.get(f).is_some_and(wanted))
            .collect()
    };
    // Where, and the faces of the result there on each of the two parts.
    let (at, here, there): (Vec3, Vec<usize>, Vec<usize>) = match found.first() {
        Some(touch) => (
            touch.at,
            faces_where(&|f| uses(f, &at_vertex(touch.vertex))),
            faces_where(&|f| match touch.touches {
                craft3d_topo::Touched::Vertex(v) => uses(f, &at_vertex(v)),
                craft3d_topo::Touched::Edge(edge) => uses(f, &|_, id| id == edge),
            }),
        ),
        None => match on_two_parts(body, tolerance, meetings)? {
            Some(found) => found,
            None => return Ok(()),
        },
    };
    // The sources of the faces at each, the first pair of two different
    // faces of the operands (pieces of one face have one source), else
    // the first two.
    let source = |f: usize| result.faces.get(f).map(|s| (s.operand, s.face));
    let here: Vec<(Operand, FaceId)> = here.into_iter().filter_map(source).collect();
    let there: Vec<(Operand, FaceId)> = there.into_iter().filter_map(source).collect();
    let pair = here
        .iter()
        .flat_map(|a| there.iter().map(move |b| (*a, *b)))
        .find(|(a, b)| a != b)
        .or_else(|| Some((*here.first()?, *there.first()?)));
    let Some((a, b)) = pair else {
        return Err(Stop::of(LOST));
    };
    Err(Stop::Error(KernelError::ZeroThickness {
        operation: OPERATION,
        at,
        faces: [a, b],
    }))
}

/// The first of `points` that lies on faces of two shells of `body`:
/// within twice the tolerance of a face's surface and not outside its
/// boundary. With the faces it lies on in the first of those shells and
/// in the second. A body whose faces cannot be outlined is not looked at
/// this way.
#[allow(clippy::type_complexity)]
fn on_two_parts(
    body: &Body,
    tolerance: &Tolerance,
    points: &[Vec3],
) -> Result<Option<(Vec3, Vec<usize>, Vec<usize>)>, Stop> {
    let tol = tolerance.linear;
    let near = 2.0 * tol;
    let mut pool = Vertices::new(0.5 * tol, 2.0 * tol);
    let Ok(solid) = section::prepare(body, Operand::A, &mut pool, tolerance, true) else {
        return Ok(None);
    };
    let mut face_shell = vec![usize::MAX; body.faces.len()];
    for (s, shell) in body.shells.iter().enumerate() {
        for face in &shell.faces {
            if let Some(slot) = face_shell.get_mut(face.0 as usize) {
                *slot = s;
            }
        }
    }
    // The faces by the low x of their boxes, to find those near a point
    // without going through all of them.
    let mut order: Vec<(f64, f64, usize)> = solid
        .boxes
        .iter()
        .enumerate()
        .filter_map(|(f, b)| b.corners().map(|(lo, hi)| (lo.x, hi.x, f)))
        .collect();
    order.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.2.cmp(&b.2)));
    let widest = order
        .iter()
        .map(|(lo, hi, _)| hi - lo)
        .fold(0.0_f64, f64::max);
    for &p in points {
        if !p.is_finite() {
            continue;
        }
        let from = order.partition_point(|&(lo, _, _)| lo < p.x - near - widest);
        let mut on: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for &(lo, _, f) in order.iter().skip(from) {
            if lo > p.x + near {
                break;
            }
            let shell = face_shell.get(f).copied().unwrap_or(usize::MAX);
            if shell == usize::MAX || !solid.boxes.get(f).is_some_and(|b| b.contains(p, near)) {
                continue;
            }
            let Ok(outline) = solid.outline(f) else {
                continue;
            };
            let Ok((u, v)) = outline.surface.project(p) else {
                continue;
            };
            let foot = outline.surface.eval(u, v);
            if foot.distance(p) <= near
                && matches!(
                    outline.side(foot, tol),
                    Ok(outline::Side::In | outline::Side::On)
                )
            {
                on.entry(shell).or_default().push(f);
            }
        }
        let mut shells = on.into_values();
        if let (Some(mut here), Some(mut there)) = (shells.next(), shells.next()) {
            here.sort_unstable();
            there.sort_unstable();
            return Ok(Some((p, here, there)));
        }
    }
    Ok(None)
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
/// against anything) have no volume in common. Their intersection is
/// [`KernelError::EmptyResult`]; A minus B is A. Their union would be one
/// body whose two parts touch, and so would a result whose parts hang
/// together only through an edge, a line or a point (a difference that
/// leaves two blocks joined along an edge): geometry of zero thickness,
/// which is refused ([`KernelError::ZeroThickness`], since 2026-10-15,
/// the developer's decision, as other modellers refuse it), with where
/// and the faces there. [`BooleanOptions::refuse_zero_thickness`] off,
/// such a result is both parts in one body, as shells that share no
/// vertex and no edge, however they touch: every edge of a result is
/// shared by exactly two faces.
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
/// [`KernelError::ZeroThickness`] for parts of the result that touch;
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
    let mut vertices = Vertices::new(0.0, 0.0);
    let solid =
        section::prepare(body, Operand::A, &mut vertices, tolerance, false).map_err(unsupported)?;
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
    fn a_body_that_passed_the_check_passes_again_and_one_that_did_not_fails_again() {
        let tolerance = Tolerance::default();
        let frame = craft3d_math::Frame::WORLD;
        let good = crate::primitives::block(&frame, 2.0, 3.0, 4.0).unwrap();
        let mut bad = good.clone();
        bad.vertices[0].point = Vec3::new(-1.0, 0.0, 0.0);
        let defects = check(&bad, &tolerance);
        assert!(!defects.is_empty());
        for _ in 0..2 {
            assert!(checked(&good, &tolerance).is_empty());
            assert_eq!(checked(&bad, &tolerance), defects);
        }
        // Checked with another tolerance it is checked again: a vertex
        // 1e-3 off its edges passes with a tolerance of 1e-2 only.
        let mut near = good.clone();
        near.vertices[0].point = Vec3::new(1e-3, 0.0, 0.0);
        let loose = Tolerance {
            linear: 1e-2,
            ..tolerance
        };
        assert!(checked(&near, &loose).is_empty());
        assert!(!checked(&near, &tolerance).is_empty());
    }

    #[test]
    fn points_within_half_the_tolerance_are_one_vertex() {
        let mut vertices = Vertices::new(0.5e-6, 2e-6);
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
