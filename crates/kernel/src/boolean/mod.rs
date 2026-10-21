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
mod contact;
mod cut;
mod facetree;
mod freeform;
mod hits;
mod loose;
mod merge;
mod outline;
mod regions;
mod section;
mod sew;
mod verify;

pub use contact::{ContactKind, Interference, MAX_CONTACT_POINTS, interference};
pub use freeform::{release_unused, shelf_size};
pub(crate) use hits::speed_bound;
pub use verify::{Verification, VerifyOptions, Violation, VolumeRule, Volumes, verify};

use crate::{KernelError, MAX_REPORTED};
use craft3d_geom::{Curve3, GeomError};
use craft3d_math::{Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind, EdgeId, FaceId, VertexId};
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
#[derive(Clone, PartialEq)]
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
    /// Every tolerance the boolean gave an entity of the result that the
    /// entity did not have in its operand (operands with loose entities
    /// only; empty for two exact operands): what it measured where the
    /// boolean, working at the operands' largest tolerance, left an entity
    /// off the document's.
    pub raised: Vec<Raised>,
}

/// As derived, but `raised` only when it holds something: the result of
/// two exact operands prints as it did before tolerant entities existed
/// (the boolean corpus's digest is of this text).
impl std::fmt::Debug for BooleanResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let BooleanResult {
            body,
            faces,
            merged,
            edges,
            vertices,
            raised,
        } = self;
        let mut out = f.debug_struct("BooleanResult");
        out.field("body", body)
            .field("faces", faces)
            .field("merged", merged)
            .field("edges", edges)
            .field("vertices", vertices);
        if !raised.is_empty() {
            out.field("raised", raised);
        }
        out.finish()
    }
}

/// An entity of a boolean's result given a tolerance it did not have
/// ([`BooleanResult::raised`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Raised {
    pub entity: RaisedEntity,
    /// Its tolerance in the result.
    pub tolerance: f64,
    /// Where it came from: one the boolean made, or one kept from an
    /// operand (loose there, or exact there and next to what the boolean
    /// changed).
    pub origin: Origin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RaisedEntity {
    Vertex(VertexId),
    Edge(EdgeId),
}

/// Where an entity of a boolean's result came from, as tolerant modelling
/// tells it apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// Made by the boolean (an intersection edge, a new vertex).
    New,
    /// Kept from an operand in which it had a tolerance of its own.
    Loose,
    /// Kept from an operand in which it was exact.
    Exact,
}

/// How a boolean is carried out.
#[derive(Clone, Copy, Debug, PartialEq)]
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
    /// The cap on any entity's own tolerance (the document's,
    /// `doc::Tolerance::loosest`); `None` for
    /// `craft3d_topo::DEFAULT_LOOSEST` times the linear tolerance. A
    /// boolean on operands with loose entities works at their largest
    /// tolerance, and refuses above this.
    pub loosest: Option<f64>,
}

impl Default for BooleanOptions {
    fn default() -> Self {
        BooleanOptions {
            merge: true,
            trust_operands: false,
            refuse_zero_thickness: true,
            loosest: None,
        }
    }
}

/// What the two operands of a boolean are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// Two solids.
    Solids,
    /// A sheet (A) trimmed by a solid (B): the result is a sheet.
    Trim,
    /// A solid (A) cut by a sheet (B): the result is a solid.
    Cut,
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
/// A union with a sheet.
pub(crate) const SHEET_UNION: Reason = "the union of a sheet with a body: a sheet can be subtracted from or intersected with a solid (to cut it), or trimmed by one, not joined";
/// A question about inside and outside, asked of a sheet.
pub(crate) const NOT_SOLID: Reason =
    "a body that is a sheet, which has no inside: points and interference are told against solids";
/// Two sheets.
const TWO_SHEETS: Reason =
    "a boolean of two sheets: a sheet is trimmed by a solid or cuts one, not another sheet (yet)";
const NO_RAY: Reason = "could not tell inside from outside: every ray tried grazes the other body";
pub(crate) const LOST: Reason = "the boolean lost track of an entity; this is a bug";
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

/// `craft3d_topo::check` of `body`, remembering the bodies that pass
/// ([`craft3d_topo::check_remembered`]): in a row of booleans each result
/// is the first operand of the next, and is checked as a result, again by
/// regeneration, and again as an operand.
pub(crate) fn checked(body: &Body, tolerance: &Tolerance) -> Vec<craft3d_topo::Defect> {
    craft3d_topo::check_remembered(body, tolerance)
}

fn face_id(index: usize) -> FaceId {
    FaceId(u32::try_from(index).unwrap_or(u32::MAX))
}

/// What is known about a merged vertex.
#[derive(Clone, Copy, Debug, Default)]
struct Meeting {
    kept: Option<(Operand, VertexId)>,
    /// Which operands have a vertex here: 1 for A, 2 for B.
    operands: u8,
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
    // What the operands are: two solids, a sheet trimmed by a solid
    // (the result a sheet), or a solid cut by a sheet (a solid).
    let mode = match (a.kind, b.kind) {
        (BodyKind::Solid, BodyKind::Solid) => Mode::Solids,
        (BodyKind::Sheet, BodyKind::Sheet) => return Err(Stop::of(TWO_SHEETS)),
        _ if op == BooleanOp::Union => return Err(Stop::of(SHEET_UNION)),
        (BodyKind::Sheet, BodyKind::Solid) => Mode::Trim,
        (BodyKind::Solid, BodyKind::Sheet) => Mode::Cut,
    };
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
    let table = match mode {
        Mode::Solids => classify::solids,
        Mode::Trim | Mode::Cut => classify::sheet,
    };
    // The regions of the sheet of a cut, kept apart to tell the solid's
    // sides by.
    let mut cap = Vec::new();
    if mode != Mode::Cut {
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
            table,
            tolerance,
            &mut kept,
        )?;
    }
    if mode != Mode::Trim {
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
            table,
            tolerance,
            if mode == Mode::Cut {
                &mut cap
            } else {
                &mut kept
            },
        )?;
    }
    if mode == Mode::Cut {
        cut::select(
            &a,
            &b,
            &cap,
            &pieces_a,
            &interior_a,
            &sections.coincident_a,
            &arcs,
            &vertices,
            op,
            tolerance,
            &mut kept,
        )?;
        kept.append(&mut cap);
    }
    if kept.is_empty() {
        return Err(Stop::Error(KernelError::EmptyResult {
            operation: OPERATION,
        }));
    }
    let mut work = sew::sew(kept, &arcs, &vertices)?;
    if options.merge {
        merge::merge(&mut work, &sections.coincident_a, a.body, b.body, tol)?;
    }
    let kind = if mode == Mode::Trim {
        BodyKind::Sheet
    } else {
        BodyKind::Solid
    };
    let mut result = sew::finish(work, kind)?;
    pieces_of_splines(&mut result.body);
    let mut defects = checked(&result.body, tolerance);
    if !defects.is_empty() {
        defects.truncate(MAX_REPORTED);
        return Err(Stop::Error(KernelError::InvalidBody {
            operation: OPERATION,
            defects,
        }));
    }
    // What every exact boolean keeps, at a few per cent of the boolean
    // (verify.rs; the volumes and the classified points are left to the
    // tests and to `document.check`, or asked for with
    // CRAFT3D_BOOLEAN_CHECK=full): a result that breaks it is refused.
    let verified = verify::verify(
        a.body,
        b.body,
        op,
        &result.body,
        tolerance,
        &verify::checked_by_every_boolean(),
    )
    .map_err(Stop::Error)?;
    if let Some(first) = verified.violations.first() {
        return Err(Stop::Error(KernelError::WrongResult {
            operation: OPERATION,
            violation: first.clone(),
            count: verified.violations.len(),
        }));
    }
    if options.refuse_zero_thickness {
        // Where the operands meet: the vertices both are at, those the
        // section made, the points where faces only touch, and a point
        // inside every stretch of curve along which faces meet.
        let mut meetings: Vec<Vec3> = vertices
            .points
            .iter()
            .zip(&vertices.meetings)
            .filter(|(_, m)| {
                let a = m.operands & 1 != 0 || m.a_edge.is_some() || m.a_face.is_some();
                let b = m.operands & 2 != 0 || m.b_edge.is_some() || m.b_face.is_some();
                a && b
            })
            .map(|(p, _)| *p)
            .collect();
        meetings.extend_from_slice(&sections.touches);
        // Where an edge of one meets the other's surface: also a contact
        // in a point that leaves no stretch (a rim touching a ball from
        // outside).
        meetings.extend_from_slice(&sections.edge_meets);
        meetings.extend(on_the_other(&a, &b, tol));
        meetings.extend(on_the_other(&b, &a, tol));
        // Where an operand already touches itself (a shell meeting itself
        // at a vertex or along an edge, or two of its shells): the parts
        // a cut makes may touch there although the operands never met.
        for operand in [a.body, b.body] {
            if let Ok(found) =
                craft3d_topo::touches(operand, 2.0 * tol, 1024, craft3d_topo::TouchScope::All)
            {
                meetings.extend(found.iter().map(|t| t.at));
            }
        }
        meetings.extend(
            sections
                .stretches
                .iter()
                .map(|s| s.curve.eval(0.5 * (s.range.0 + s.range.1))),
        );
        zero_thickness(&result, tolerance, &distinct(meetings))?;
        // A sheet that comes back to itself in a point is refused; a
        // solid's shell that does so is left to the rules above and to
        // regeneration (a wall that thins to nothing: accepted, decided
        // 2026-10-18).
        if result.body.kind == BodyKind::Sheet {
            touching_itself(&result, tol)?;
        }
    }
    Ok(result)
}

/// The places in their first order, each once: a place found by several
/// of the ways [`zero_thickness`] is given its places (a vertex of the
/// section that is also where an operand touches itself, the same touch
/// from both operands) gets the same answer each time, so it is looked at
/// once. A union of many shelled balls had 39 671 places, 2282 of them
/// distinct (fuzz seed 20757788, 2026-10-21).
fn distinct(places: Vec<Vec3>) -> Vec<Vec3> {
    let mut seen = std::collections::BTreeSet::new();
    places
        .into_iter()
        .filter(|p| seen.insert([p.x.to_bits(), p.y.to_bits(), p.z.to_bits()]))
        .collect()
}

/// Refuses a result whose parts (shells) touch each other, which is
/// geometry of zero thickness (the developer's decision of 2026-10-15).
///
/// Parts can only touch where the operands meet, or where an operand
/// already touches itself, so only those places are looked at
/// (`meetings`: the vertices both operands are at, the vertices the
/// section made, the points where two faces only touch, the points where
/// an edge of one meets the other's surface, a point inside
/// every stretch of curve along which faces meet, and the places where a
/// vertex of an operand lies on a vertex or an edge of that operand
/// not its own, `craft3d_topo::touches`). Faces of one operand that touch
/// with no vertex there are not seen here (owed); regeneration's
/// `body_touches_itself` also looks at every result for the vertex and
/// edge forms. A place is a
/// touch when it is within twice the tolerance of a face of one part and
/// of a face of another, not outside either face's boundary, and the
/// nearest points of the two faces are within twice the tolerance of each
/// other: the parts are no further apart there than the tolerance can
/// tell, the rule of `craft3d_topo::touches` for vertices and edges.
/// Every place costs a look at the faces whose boxes hold it; an outline
/// is made only for a face that may be at a place without being joined
/// there to the other face looked at. Two walls of one shell that touch
/// (parts that hang together elsewhere as well) are the second half,
/// [`within_a_shell`].
fn zero_thickness(
    result: &BooleanResult,
    tolerance: &Tolerance,
    meetings: &[Vec3],
) -> Result<(), Stop> {
    let body = &result.body;
    if meetings.is_empty() {
        return Ok(());
    }
    let tol = tolerance.linear;
    let near = 2.0 * tol;
    let mut face_shell = vec![usize::MAX; body.faces.len()];
    for (s, shell) in body.shells.iter().enumerate() {
        for face in &shell.faces {
            if let Some(slot) = face_shell.get_mut(face.0 as usize) {
                *slot = s;
            }
        }
    }
    // The faces' boxes, grown by the distance (their size sets how far
    // `within_a_shell` looks about a place).
    let grown = |b: craft3d_math::Bbox3| {
        b.corners().map(|(lo, hi)| {
            let grow = Vec3::new(near, near, near);
            (lo - grow, hi + grow)
        })
    };
    let boxes: Vec<Option<(Vec3, Vec3)>> = (0..body.faces.len())
        .map(|f| body.face_bbox(face_id(f)).ok().and_then(grown))
        .collect();
    // Which faces may be at a place: those whose box holds it and the box
    // of what they hold too (a piece of a ball near where it is, not the
    // whole ball: `Body::held_face_bbox`), grown by the distance and the
    // tolerance (a foot on a face's boundary may lie that far outside
    // it), in a grid of cells about as wide as a typical face, so that a
    // place finds the faces near it without looking at all of them. A
    // union of many balls cut into pieces had every one of its 1100 faces'
    // boxes hold every place (fuzz seed 20757788, 2026-10-21).
    let reach = near + tol;
    let held: Vec<Option<(Vec3, Vec3)>> = (0..body.faces.len())
        .map(|f| {
            let whole = boxes.get(f).copied().flatten()?;
            let Some((lo, hi)) = body
                .held_face_bbox(face_id(f))
                .ok()
                .and_then(|b| b.corners())
            else {
                return Some(whole);
            };
            let grow = Vec3::new(reach, reach, reach);
            let (lo, hi) = ((lo - grow).max(whole.0), (hi + grow).min(whole.1));
            (lo.x <= hi.x && lo.y <= hi.y && lo.z <= hi.z).then_some((lo, hi))
        })
        .collect();
    let grid = FaceGrid::new(&held);
    let mut outlines: BTreeMap<usize, Option<outline::Outline>> = BTreeMap::new();
    // Per place, the faces at it with their nearest points, as the first
    // half found them, for the second half to start from.
    let mut at_places: Vec<Option<Vec<(usize, Vec3)>>> = vec![None; meetings.len()];
    for (index, &p) in meetings.iter().enumerate() {
        if !p.is_finite() || body.shells.len() < 2 {
            continue;
        }
        let mut at = Vec::new();
        // Per shell, the faces the place is on and the nearest point of
        // each.
        let mut on: BTreeMap<usize, Vec<(usize, Vec3)>> = BTreeMap::new();
        for f in grid.near(p) {
            let shell = face_shell.get(f).copied().unwrap_or(usize::MAX);
            let inside = held.get(f).copied().flatten().is_some_and(|(lo, hi)| {
                lo.x <= p.x
                    && p.x <= hi.x
                    && lo.y <= p.y
                    && p.y <= hi.y
                    && lo.z <= p.z
                    && p.z <= hi.z
            });
            if shell == usize::MAX || !inside {
                continue;
            }
            let outline = outlines
                .entry(f)
                .or_insert_with(|| section::face_outline(body, f, tol).ok());
            let Some(outline) = outline.as_ref() else {
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
                on.entry(shell).or_default().push((f, foot));
                at.push((f, foot));
            }
        }
        if let Some(slot) = at_places.get_mut(index) {
            *slot = Some(at);
        }
        if on.len() < 2 {
            continue;
        }
        let parts: Vec<&Vec<(usize, Vec3)>> = on.values().collect();
        for (i, here) in parts.iter().enumerate() {
            for there in parts.iter().skip(i + 1) {
                let touching = here.iter().find_map(|&(f, a)| {
                    there
                        .iter()
                        .find(|&&(_, b)| a.distance(b) <= near)
                        .map(|&(g, _)| (f, g))
                });
                let Some((f, g)) = touching else {
                    continue;
                };
                // Name the pair by two different faces of the operands
                // where the parts have them (pieces of one face share a
                // source).
                let source = |f: usize| result.faces.get(f).map(|s| (s.operand, s.face));
                let pair = here
                    .iter()
                    .filter_map(|&(f, _)| source(f))
                    .flat_map(|a| {
                        there
                            .iter()
                            .filter_map(|&(g, _)| source(g))
                            .map(move |b| (a, b))
                    })
                    .find(|(a, b)| a != b)
                    .or_else(|| Some((source(f)?, source(g)?)));
                let Some((a, b)) = pair else {
                    return Err(Stop::of(LOST));
                };
                return Err(Stop::Error(KernelError::ZeroThickness {
                    operation: OPERATION,
                    at: p,
                    faces: [a, b],
                }));
            }
        }
    }
    let faces = Faces {
        boxes: &boxes,
        held: &held,
        grid: &grid,
        shell: &face_shell,
    };
    within_a_shell(result, tol, meetings, &faces, &mut outlines, &at_places)
}

/// What [`zero_thickness`] knows of the result's faces, for
/// [`within_a_shell`]: per face its box and the box of what it holds
/// (grown by the distance looked within), the grid of the latter, and the
/// shell it is in.
#[derive(Clone, Copy)]
struct Faces<'a> {
    boxes: &'a [Option<(Vec3, Vec3)>],
    held: &'a [Option<(Vec3, Vec3)>],
    grid: &'a FaceGrid,
    shell: &'a [usize],
}

/// Refuses a result one of whose shells touches itself at a vertex: a
/// vertex within twice the tolerance of another vertex, or of an edge it
/// is not an end of, of the same shell (`craft3d_topo::touches`, the rule
/// regeneration holds every body to). [`zero_thickness`] finds two faces
/// at a place; this finds a face, or the boundary of a sheet, that comes
/// back to itself in a point where the sewing gave each side its own
/// vertex (a hole tangent to a cut-out, the figure of eight a plane
/// leaves of a ring it touches).
fn touching_itself(result: &BooleanResult, tol: f64) -> Result<(), Stop> {
    let body = &result.body;
    let Ok(found) = craft3d_topo::touches(body, 2.0 * tol, 1, craft3d_topo::TouchScope::All) else {
        return Ok(());
    };
    let Some(touch) = found.into_iter().find(|t| !t.between_shells()) else {
        return Ok(());
    };
    // A face at the vertex and one at what it touches, by their sources.
    let face_at = |test: &dyn Fn(&craft3d_topo::Coedge) -> bool| {
        body.faces.iter().enumerate().find_map(|(f, face)| {
            face.loops
                .iter()
                .filter_map(|l| body.loop_(*l))
                .flat_map(|l| &l.coedges)
                .any(test)
                .then(|| result.faces.get(f).map(|s| (s.operand, s.face)))
                .flatten()
        })
    };
    let at_vertex = |v: u32| {
        move |c: &craft3d_topo::Coedge| {
            body.edge(c.edge)
                .is_some_and(|e| e.start.0 == v || e.end.0 == v)
        }
    };
    let first = face_at(&at_vertex(touch.vertex));
    let second = match touch.touches {
        craft3d_topo::Touched::Vertex(v) => face_at(&at_vertex(v)),
        craft3d_topo::Touched::Edge(e) => face_at(&|c: &craft3d_topo::Coedge| c.edge.0 == e),
    };
    let (Some(a), Some(b)) = (first, second) else {
        return Err(Stop::of(LOST));
    };
    Err(Stop::Error(KernelError::ZeroThickness {
        operation: OPERATION,
        at: touch.at,
        faces: [a, b],
    }))
}

/// Whether face `g` (its outline, at `b`) stays on the outer side of
/// face `f`'s surface near the place where they touch: no point of `g`
/// within `reach` of `b` (eight directions about `b` in `g`'s tangent
/// plane, onto `g`'s surface and inside `g`, whose foot on `f`'s surface
/// lies inside `f`) is more than `near` inside `f`'s material, by `f`'s
/// outward normal there.
///
/// Two walls of one shell that touch with opposite outward normals are
/// one of two things. Their materials lie on their far sides and the
/// space between them closes up (a ball resting on a plate's top, both
/// parts of the result; an edge of one part along a face of the other):
/// two parts of the solid touch, and each wall stays on the outer side of
/// the other. Or the material lies between them and thins to nothing (a
/// hole whose wall touches the plate's side along a line): each wall
/// bends into the other's material away from the place. The first is
/// zero thickness as refused since 2026-10-15; the second is a wall that
/// thins out to no thickness, which the boolean has always made and its
/// tests pin (whether it is refused too is the developer's to settle).
/// The two differ only to second order at the place itself, hence the
/// look a little way off. Where no point of `g` is found to look at, the
/// walls are taken to be apart, so that the refusal errs on the side of
/// no zero-thickness result.
fn walls_apart(f: &outline::Outline, b: Vec3, g: &outline::Outline, near: f64, reach: f64) -> bool {
    use outline::{Sheet, onto, outward_normal};
    if !(reach.is_finite() && reach > near) {
        return true;
    }
    let Some(normal) = outward_normal(Sheet::bare(&g.surface), g.reversed, b) else {
        return true;
    };
    let Some(frame) = craft3d_math::Frame::from_z(b, normal) else {
        return true;
    };
    for k in 0..8 {
        let angle = f64::from(k) * std::f64::consts::FRAC_PI_4;
        let off = b + (frame.x() * angle.cos() + frame.y() * angle.sin()) * reach;
        let Some(y) = onto(Sheet::bare(&g.surface), off) else {
            continue;
        };
        if y.distance(b) < 0.25 * reach || !matches!(g.side(y, near * 0.5), Ok(outline::Side::In)) {
            continue;
        }
        let (Some(q), Some(outward)) = (
            onto(Sheet::bare(&f.surface), y),
            outward_normal(Sheet::bare(&f.surface), f.reversed, y),
        ) else {
            continue;
        };
        // Only where `f` itself is there: beyond its boundary its surface
        // says nothing about its material (a bar's edge along a plate,
        // whose side faces lie under the plate's surface past their own
        // edges).
        if !matches!(f.side(q, near * 0.5), Ok(outline::Side::In)) {
            continue;
        }
        if (y - q).dot(outward) < -near {
            return false;
        }
    }
    true
}

/// The second half of [`zero_thickness`]: a shell that touches itself.
/// Two faces of one shell that are both at a place (within twice the
/// tolerance, not outside either boundary, their nearest points within
/// twice the tolerance of each other) touch there unless they are joined
/// there, by a vertex of both or an edge of both that passes within four
/// times the tolerance of the place. In a manifold every point of a
/// shell is inside one face, on one edge (two faces) or at one vertex
/// (the faces around it), so two faces at one place with nothing of
/// theirs in common there are two walls of the shell that meet: a
/// result of zero thickness along a line or in a point, whether the
/// parts that touch are separate shells or hang together elsewhere (the
/// fuzzer's seed 5279, 2026-10-17: an edge of one operand lying along a
/// face of the other, the union one shell through the volume both have
/// in common). The margin for "joined" is twice that for "at", which
/// covers faces that meet at an edge at 29° or more; at a sharper angle
/// they stay within the tolerance of each other further from the edge,
/// and the margin of a shared edge grows as 1 / sin(θ/2) (`wedged`), so
/// that a wedge or a groove is never taken for two walls that touch.
fn within_a_shell(
    result: &BooleanResult,
    tol: f64,
    meetings: &[Vec3],
    faces: &Faces<'_>,
    outlines: &mut BTreeMap<usize, Option<outline::Outline>>,
    at_places: &[Option<Vec<(usize, Vec3)>>],
) -> Result<(), Stop> {
    let Faces {
        boxes,
        held,
        grid,
        shell: face_shell,
    } = *faces;
    let body = &result.body;
    let near = 2.0 * tol;
    let reach = 2.0 * near;
    // Per face, its vertices and edges, each once, ascending.
    let mut parts: BTreeMap<usize, (Vec<u32>, Vec<u32>)> = BTreeMap::new();
    let parts_of = |f: usize| -> (Vec<u32>, Vec<u32>) {
        let mut vertices = Vec::new();
        let mut edges = Vec::new();
        let coedges = body
            .face(face_id(f))
            .into_iter()
            .flat_map(|face| &face.loops)
            .filter_map(|l| body.loop_(*l))
            .flat_map(|l| &l.coedges);
        for coedge in coedges {
            edges.push(coedge.edge.0);
            if let Some(edge) = body.edge(coedge.edge) {
                vertices.extend([edge.start.0, edge.end.0]);
            }
        }
        for list in [&mut vertices, &mut edges] {
            list.sort_unstable();
            list.dedup();
        }
        (vertices, edges)
    };
    // The ids both ascending lists hold, merged in one pass.
    let common = |a: &[u32], b: &[u32]| -> Vec<u32> {
        let (mut i, mut j, mut out) = (0, 0, Vec::new());
        while let (Some(&x), Some(&y)) = (a.get(i), b.get(j)) {
            match x.cmp(&y) {
                std::cmp::Ordering::Less => i += 1,
                std::cmp::Ordering::Greater => j += 1,
                std::cmp::Ordering::Equal => {
                    out.push(x);
                    i += 1;
                    j += 1;
                }
            }
        }
        out
    };
    // The point of edge `e` nearest `p`, inside its range; `Err` where the
    // projection fails.
    let on_edge = |e: u32, p: Vec3| -> Result<Option<Vec3>, ()> {
        let Some(edge) = body.edge(craft3d_topo::EdgeId(e)) else {
            return Ok(None);
        };
        let t = edge.curve.project(p).map_err(|_| ())?;
        let (t0, t1) = edge.range;
        let t = match edge.curve.period() {
            Some(period) if period > 0.0 => t - ((t - t0) / period).floor() * period,
            _ => t,
        };
        Ok((t >= t0 && t <= t1).then(|| edge.curve.eval(t)))
    };
    let joined = |vertices: &[u32], edges: &[u32], p: Vec3| -> bool {
        let by_vertex = vertices.iter().any(|&v| {
            body.vertices
                .get(v as usize)
                .is_some_and(|vertex| vertex.point.distance(p) <= reach)
        });
        by_vertex
            || edges.iter().any(|&e| {
                let in_box = body
                    .edge_bbox(craft3d_topo::EdgeId(e))
                    .is_ok_and(|b| b.contains(p, reach));
                in_box
                    && match on_edge(e, p) {
                        // Not known to be apart: joined, never a refusal
                        // made up.
                        Err(()) => true,
                        Ok(q) => q.is_some_and(|q| q.distance(p) <= reach),
                    }
            })
    };
    // Faces that meet at a shared edge at an angle θ (between the planes
    // they leave it in) stay within `near` of each other for
    // `near / (2 sin(θ/2))` from it: past `reach` below about 29°. So a
    // shared edge joins them at the place also when the place is within
    // `near / sin(θ/2)` of it (twice that, for margin), θ read from the
    // outward normals at the edge's nearest point (sin(θ/2) =
    // sqrt((1 + n_f · n_g) / 2) for faces that leave the edge on one side
    // of each other), up to `most`: one sliver of a wedge or a groove, not
    // two walls.
    //
    // Beyond `most` (a tenth of the smaller face, which keeps the reading
    // local) a shared edge still joins the faces at the place when the
    // gap between them there, `gap`, is at least half of what two planes
    // leaving the edge at θ have at that distance (`2 d sin(θ/2)`): they
    // do not come back towards each other, the gap is the wedge's (a
    // crack between faces at 1.19°, cut short by a third face 55 µm from
    // a corner of a face that is itself that small: the Garabit pier's
    // diagonal, 2026-10-21).
    let wedged = |f: usize, g: usize, edges: &[u32], p: Vec3, most: f64, gap: f64| -> bool {
        let most = most.max(reach);
        let faces = body.face(face_id(f)).zip(body.face(face_id(g)));
        edges.iter().any(|&e| {
            // An edge further than `most` joins nothing here by its box,
            // only by the wedge's gap below.
            let within = body
                .edge_bbox(craft3d_topo::EdgeId(e))
                .is_ok_and(|b| b.contains(p, most));
            // The point of the edge nearest the place: its foot when that
            // is inside the edge's range, otherwise the nearer end (the
            // distance to the edge as a set of points: a wedge whose edge
            // a third face cuts short at a vertex is still a wedge about
            // that vertex, the Garabit pier's diagonal, 2026-10-21).
            let nearest = match on_edge(e, p) {
                Ok(Some(q)) => Ok(Some(q)),
                Ok(None) => Ok(body.edge(craft3d_topo::EdgeId(e)).and_then(|edge| {
                    let ends = [edge.start, edge.end]
                        .map(|v| body.vertices.get(v.0 as usize).map(|v| v.point));
                    match ends {
                        [Some(a), Some(b)] => {
                            Some(if a.distance(p) <= b.distance(p) { a } else { b })
                        }
                        [Some(a), None] | [None, Some(a)] => Some(a),
                        [None, None] => None,
                    }
                })),
                Err(()) => Err(()),
            };
            match nearest {
                // Its nearest point not known: joined only as close as
                // its box allows.
                Err(()) => within,
                Ok(None) => false,
                Ok(Some(q)) => {
                    let angle = faces.and_then(|(sf, sg)| Some((sf.outward(q)?, sg.outward(q)?)));
                    let Some((nf, ng)) = angle else {
                        // No normal there (an apex, a pole): the widest
                        // margin, and no further.
                        return within && q.distance(p) <= most;
                    };
                    let half = (0.5 * (1.0 + nf.dot(ng))).max(0.0).sqrt();
                    let margin = if half > 0.0 {
                        near / half
                    } else {
                        f64::INFINITY
                    };
                    let d = q.distance(p);
                    (within && d <= margin.max(reach).min(most)) || (d <= margin && gap >= d * half)
                }
            }
        })
    };

    for (index, &p) in meetings.iter().enumerate() {
        if !p.is_finite() {
            continue;
        }
        let candidates: Vec<usize> = grid
            .near(p)
            .into_iter()
            .filter(|&f| {
                face_shell.get(f).is_some_and(|&s| s != usize::MAX)
                    && held.get(f).copied().flatten().is_some_and(|(lo, hi)| {
                        lo.x <= p.x
                            && p.x <= hi.x
                            && lo.y <= p.y
                            && p.y <= hi.y
                            && lo.z <= p.z
                            && p.z <= hi.z
                    })
            })
            .collect();
        if candidates.len() < 2 {
            continue;
        }
        for &f in &candidates {
            parts.entry(f).or_insert_with(|| parts_of(f));
        }
        // The nearest point of each candidate when the place is on it,
        // made when a pair needs it, or found by the first half for every
        // candidate (the same test on the same faces). A pair one of whose
        // faces is not at the place goes on to the next whatever else is
        // true of it, so then only the faces at the place are paired: the
        // same pairs in the same order, without the square of every face
        // whose box holds the place (a union of many balls, each face's
        // box its whole sphere: 1100 candidates a place).
        let mut feet: BTreeMap<usize, Option<Vec3>> = BTreeMap::new();
        let paired: Vec<usize> = match at_places.get(index).and_then(Option::as_ref) {
            Some(at) => {
                for &f in &candidates {
                    let found = at.iter().find(|(g, _)| *g == f).map(|&(_, foot)| foot);
                    feet.insert(f, found);
                }
                candidates
                    .iter()
                    .copied()
                    .filter(|f| feet.get(f).copied().flatten().is_some())
                    .collect()
            }
            None => candidates.clone(),
        };
        let mut foot =
            |f: usize, outlines: &mut BTreeMap<usize, Option<outline::Outline>>| -> Option<Vec3> {
                *feet.entry(f).or_insert_with(|| {
                    let outline = outlines
                        .entry(f)
                        .or_insert_with(|| section::face_outline(body, f, tol).ok())
                        .as_ref()?;
                    let (u, v) = outline.surface.project(p).ok()?;
                    let foot = outline.surface.eval(u, v);
                    (foot.distance(p) <= near
                        && matches!(
                            outline.side(foot, tol),
                            Ok(outline::Side::In | outline::Side::On)
                        ))
                    .then_some(foot)
                })
            };
        for (i, &f) in paired.iter().enumerate() {
            for &g in paired.iter().skip(i + 1) {
                if face_shell.get(f) != face_shell.get(g) {
                    // Between shells: the first half's.
                    continue;
                }
                let (Some((fv, fe)), Some((gv, ge))) = (parts.get(&f), parts.get(&g)) else {
                    return Err(Stop::of(LOST));
                };
                let edges = common(fe, ge);
                if joined(&common(fv, gv), &edges, p) {
                    continue;
                }
                let (Some(a), Some(b)) = (foot(f, outlines), foot(g, outlines)) else {
                    continue;
                };
                let size = |f: usize| {
                    boxes
                        .get(f)
                        .copied()
                        .flatten()
                        .map_or(0.0, |(lo, hi)| lo.distance(hi))
                };
                let reach = 0.1 * size(f).min(size(g));
                if a.distance(b) > near || wedged(f, g, &edges, p, reach, a.distance(b)) {
                    continue;
                }
                let (Some(Some(of)), Some(Some(og))) = (outlines.get(&f), outlines.get(&g)) else {
                    return Err(Stop::of(LOST));
                };
                if !(walls_apart(of, b, og, near, reach) && walls_apart(og, a, of, near, reach)) {
                    // Material between them, thinning to nothing: not
                    // two parts that touch (see `walls_apart`).
                    continue;
                }
                // Through a third face at the place: one of the two is
                // joined there to a face `h` that makes a wedge with the
                // other (the end face of a girder whose bottom edge is
                // the place, near the leg's face only as the girder's
                // bottom is, which leaves the leg's face at 1.19°: the
                // Garabit pier's diagonal, 2026-10-21). The place is one
                // point; that it is near the other face is the wedge's
                // doing, whichever face of `h`'s edge it is read on.
                let mut through = false;
                for &h in &paired {
                    if through {
                        break;
                    }
                    if h == f || h == g || face_shell.get(h) != face_shell.get(f) {
                        continue;
                    }
                    let Some(c) = foot(h, outlines) else {
                        continue;
                    };
                    let Some((hv, he)) = parts.get(&h) else {
                        return Err(Stop::of(LOST));
                    };
                    for ((x, xe, foot_x), (yv, ye)) in
                        [((f, fe, a), (gv, ge)), ((g, ge, b), (fv, fe))]
                    {
                        if joined(&common(yv, hv), &common(ye, he), p) {
                            let edges = common(xe, he);
                            let reach = 0.1 * size(x).min(size(h));
                            if wedged(x, h, &edges, p, reach, foot_x.distance(c)) {
                                through = true;
                                break;
                            }
                        }
                    }
                }
                if through {
                    continue;
                }
                let source = |f: usize| result.faces.get(f).map(|s| (s.operand, s.face));
                let (Some(a), Some(mut b)) = (source(f), source(g)) else {
                    return Err(Stop::of(LOST));
                };
                if a.0 == b.0 {
                    // Both of one operand (two pieces of one face, say):
                    // name a face of the other operand at the place where
                    // there is one, else another face of this one.
                    let mut other_face = None;
                    for &h in &candidates {
                        let Some(o) = source(h).filter(|o| *o != a) else {
                            continue;
                        };
                        if foot(h, outlines).is_none() {
                            continue;
                        }
                        if o.0 != a.0 {
                            other_face = Some(o);
                            break;
                        }
                        if b == a && other_face.is_none() {
                            other_face = Some(o);
                        }
                    }
                    if let Some(o) = other_face {
                        b = o;
                    }
                }
                return Err(Stop::Error(KernelError::ZeroThickness {
                    operation: OPERATION,
                    at: p,
                    faces: [a, b],
                }));
            }
        }
    }
    Ok(())
}

/// The vertices of `x` that lie on a face of `y`: within twice the
/// tolerance of its surface and not outside its boundary (a corner of
/// one body standing on a face of the other, which the section, finding
/// no curve, leaves alone). Only vertices in `y`'s box are looked at,
/// against the faces whose boxes hold them.
fn on_the_other(x: &Solid, y: &Solid, tol: f64) -> Vec<Vec3> {
    let near = 2.0 * tol;
    let all = y.bbox();
    let boxes: Vec<Option<(Vec3, Vec3)>> = y
        .boxes
        .iter()
        .map(|b| {
            b.corners().map(|(lo, hi)| {
                let grow = Vec3::new(near, near, near);
                (lo - grow, hi + grow)
            })
        })
        .collect();
    let mut grid: Option<FaceGrid> = None;
    let mut found = Vec::new();
    for vertex in &x.body.vertices {
        let p = vertex.point;
        if !all.contains(p, near) {
            continue;
        }
        let grid = grid.get_or_insert_with(|| FaceGrid::new(&boxes));
        let on = grid.near(p).into_iter().any(|f| {
            boxes.get(f).copied().flatten().is_some_and(|(lo, hi)| {
                lo.x <= p.x
                    && p.x <= hi.x
                    && lo.y <= p.y
                    && p.y <= hi.y
                    && lo.z <= p.z
                    && p.z <= hi.z
            }) && y.outline(f).is_ok_and(|outline| {
                outline.surface.project(p).is_ok_and(|(u, v)| {
                    let foot = outline.surface.eval(u, v);
                    foot.distance(p) <= near
                        && matches!(
                            outline.side(foot, tol),
                            Ok(outline::Side::In | outline::Side::On)
                        )
                })
            })
        });
        if on {
            found.push(p);
        }
    }
    found
}

/// Where two shells of a solid touch: `at` is a point of shell
/// `faces[0]`'s face that lies on face `faces[1]` of another shell, both
/// as indices into the body's faces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PartsTouch {
    pub at: Vec3,
    pub faces: [FaceId; 2],
}

/// The first place, if any, where two shells (parts) of the solid `body`
/// touch: a vertex of one shell, the middle of one of its edges, or a
/// point of one of its faces without loops, that lies on a face of
/// another shell (within twice the tolerance of its surface and not
/// outside its boundary). That is geometry of zero thickness, refused by
/// the developer's decision of 2026-10-15, the rule the boolean holds its
/// results to (`zero_thickness`) applied to a body without operands: an
/// operation that builds a body by other means (a blend's steps, which
/// leave the refusal to the blended body; a shell, an offset) is held to
/// it by regeneration. `craft3d_topo::touches` finds a vertex on a vertex
/// or an edge of another shell; this finds one on a face (the cavity of
/// a hollow block whose outer edge was chamfered by twice the wall: the
/// chamfer's face passes through the cavity's edge). Faces that touch
/// with no vertex or middle of an edge there (a ball in a cavity resting
/// on its floor) are not seen (owed, as for the boolean).
///
/// Only places in another shell's box are looked at, against the faces
/// whose boxes hold them; outlines are made for those faces only. In face
/// order, so the answer does not depend on threads or runs.
///
/// **Errors.** [`KernelError::InvalidSize`] for a tolerance that is not
/// positive and finite.
pub fn parts_touch(body: &Body, tolerance: &Tolerance) -> Result<Option<PartsTouch>, KernelError> {
    let tol = tolerance.linear;
    if !(tol.is_finite() && tol > 0.0) {
        return Err(KernelError::InvalidSize {
            name: "tolerance",
            value: tol,
        });
    }
    if body.kind != BodyKind::Solid || body.shells.len() < 2 {
        return Ok(None);
    }
    let near = 2.0 * tol;
    let grow = Vec3::new(near, near, near);
    let mut face_shell = vec![usize::MAX; body.faces.len()];
    for (s, shell) in body.shells.iter().enumerate() {
        for face in &shell.faces {
            if let Some(slot) = face_shell.get_mut(face.0 as usize) {
                *slot = s;
            }
        }
    }
    let boxes: Vec<Option<(Vec3, Vec3)>> = (0..body.faces.len())
        .map(|f| {
            body.face_bbox(face_id(f))
                .ok()
                .and_then(|b| b.corners())
                .map(|(lo, hi)| (lo - grow, hi + grow))
        })
        .collect();
    // Each shell's box, the faces' boxes together.
    let mut shell_boxes: Vec<Option<(Vec3, Vec3)>> = vec![None; body.shells.len()];
    for (f, b) in boxes.iter().enumerate() {
        let (Some((lo, hi)), Some(slot)) =
            (b, face_shell.get(f).and_then(|&s| shell_boxes.get_mut(s)))
        else {
            continue;
        };
        *slot = Some(match *slot {
            None => (*lo, *hi),
            Some((a, b)) => (
                Vec3::new(a.x.min(lo.x), a.y.min(lo.y), a.z.min(lo.z)),
                Vec3::new(b.x.max(hi.x), b.y.max(hi.y), b.z.max(hi.z)),
            ),
        });
    }
    let held = |b: Option<(Vec3, Vec3)>, p: Vec3| {
        b.is_some_and(|(lo, hi)| {
            lo.x <= p.x && p.x <= hi.x && lo.y <= p.y && p.y <= hi.y && lo.z <= p.z && p.z <= hi.z
        })
    };
    let grid = FaceGrid::new(&boxes);
    let mut outlines: BTreeMap<usize, Option<outline::Outline>> = BTreeMap::new();
    let mut seen_vertex = vec![false; body.vertices.len()];
    let mut seen_edge = vec![false; body.edges.len()];
    for (f, face) in body.faces.iter().enumerate() {
        let own = face_shell.get(f).copied().unwrap_or(usize::MAX);
        if own == usize::MAX {
            continue;
        }
        // The places of this face not looked at yet.
        let mut places: Vec<Vec3> = Vec::new();
        if face.loops.is_empty() {
            places.push(face.surface.eval(0.0, 0.0));
        }
        for coedge in face
            .loops
            .iter()
            .filter_map(|l| body.loop_(*l))
            .flat_map(|l| &l.coedges)
        {
            let Some(edge) = body.edge(coedge.edge) else {
                continue;
            };
            if let Some(seen) = seen_edge.get_mut(coedge.edge.0 as usize)
                && !*seen
            {
                *seen = true;
                let (t0, t1) = edge.range;
                places.push(edge.curve.eval(0.5 * (t0 + t1)));
            }
            for v in [edge.start, edge.end] {
                if let (Some(seen), Some(vertex)) =
                    (seen_vertex.get_mut(v.0 as usize), body.vertex(v))
                    && !*seen
                {
                    *seen = true;
                    places.push(vertex.point);
                }
            }
        }
        for p in places {
            if !p.is_finite() {
                continue;
            }
            let elsewhere = shell_boxes
                .iter()
                .enumerate()
                .any(|(t, b)| t != own && held(*b, p));
            if !elsewhere {
                continue;
            }
            for g in grid.near(p) {
                let shell = face_shell.get(g).copied().unwrap_or(usize::MAX);
                if shell == own || shell == usize::MAX || !held(boxes.get(g).copied().flatten(), p)
                {
                    continue;
                }
                let outline = outlines
                    .entry(g)
                    .or_insert_with(|| section::face_outline(body, g, tol).ok());
                let Some(outline) = outline.as_ref() else {
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
                    return Ok(Some(PartsTouch {
                        at: p,
                        faces: [face_id(f), face_id(g)],
                    }));
                }
            }
        }
    }
    Ok(None)
}

/// Boxes in a grid of cells, about as many cells as boxes, each box in
/// the cells it overlaps (at most [`FaceGrid::MOST_CELLS_PER_BOX`] of
/// them; a larger box goes in a list of its own that every query sees).
struct FaceGrid {
    low: Vec3,
    cell: f64,
    size: [usize; 3],
    cells: BTreeMap<usize, Vec<usize>>,
    large: Vec<usize>,
}

impl FaceGrid {
    const MOST_CELLS_PER_BOX: usize = 64;

    fn new(boxes: &[Option<(Vec3, Vec3)>]) -> FaceGrid {
        let mut lo = Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
        let mut hi = Vec3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        for (a, b) in boxes.iter().flatten() {
            lo = Vec3::new(lo.x.min(a.x), lo.y.min(a.y), lo.z.min(a.z));
            hi = Vec3::new(hi.x.max(b.x), hi.y.max(b.y), hi.z.max(b.z));
        }
        let extent = hi - lo;
        let count = boxes.len().max(1) as f64;
        let largest = extent.x.max(extent.y).max(extent.z);
        // Cells about as many as boxes.
        let cell = (extent.x.max(largest * 1e-3)
            * extent.y.max(largest * 1e-3)
            * extent.z.max(largest * 1e-3)
            / count)
            .cbrt();
        let mut grid = FaceGrid {
            low: lo,
            cell,
            size: [1, 1, 1],
            cells: BTreeMap::new(),
            large: Vec::new(),
        };
        if !(cell.is_finite() && cell > 0.0 && largest.is_finite()) {
            grid.large = (0..boxes.len()).collect();
            return grid;
        }
        let along = |e: f64| ((e / cell).floor() as usize).saturating_add(1).min(1 << 20);
        grid.size = [along(extent.x), along(extent.y), along(extent.z)];
        for (f, b) in boxes.iter().enumerate() {
            let Some((a, b)) = b else {
                grid.large.push(f);
                continue;
            };
            let (i0, i1) = (grid.index(*a), grid.index(*b));
            let span = i0
                .iter()
                .zip(&i1)
                .map(|(a, b)| b.saturating_sub(*a).saturating_add(1))
                .fold(1_usize, usize::saturating_mul);
            if span > Self::MOST_CELLS_PER_BOX {
                grid.large.push(f);
                continue;
            }
            for x in i0[0]..=i1[0] {
                for y in i0[1]..=i1[1] {
                    for z in i0[2]..=i1[2] {
                        grid.cells.entry(grid.key([x, y, z])).or_default().push(f);
                    }
                }
            }
        }
        grid
    }

    fn index(&self, p: Vec3) -> [usize; 3] {
        let at = |x: f64, low: f64, size: usize| {
            let i = ((x - low) / self.cell).floor();
            if i.is_finite() && i > 0.0 {
                (i as usize).min(size.saturating_sub(1))
            } else {
                0
            }
        };
        [
            at(p.x, self.low.x, self.size[0]),
            at(p.y, self.low.y, self.size[1]),
            at(p.z, self.low.z, self.size[2]),
        ]
    }

    fn key(&self, [x, y, z]: [usize; 3]) -> usize {
        x.saturating_add(
            self.size[0].saturating_mul(y.saturating_add(self.size[1].saturating_mul(z))),
        )
    }

    /// The boxes that may hold `p`, each once, ascending.
    fn near(&self, p: Vec3) -> Vec<usize> {
        let mut found: Vec<usize> = self.large.clone();
        if let Some(cell) = self.cells.get(&self.key(self.index(p))) {
            found.extend_from_slice(cell);
        }
        found.sort_unstable();
        found.dedup();
        found
    }
}

/// Every edge on a part of a spline curve gets that part as its curve
/// (exactly, by knot insertion: `geom::refine::piece`), so that what an
/// edge's curve is and where its control points lie is the edge and no
/// more. A stretch of a fitted branch, or a piece of an operand's edge
/// split by the section, kept the whole curve with a range: the hull of
/// every face it bounds, which counts a spline's control points, then
/// reached as far as the whole curve (the turbofan's casing kept a hole's
/// whole loop with each piece of it, and its faces' boxes grew by the
/// loop). Done when the body is made, after `merge`, which joins two edges
/// of one curve by its identity. (Where the piece cannot be made, the
/// edge keeps its curve and range, as before.)
fn pieces_of_splines(body: &mut Body) {
    for edge in &mut body.edges {
        let Curve3::Nurbs(curve) = &edge.curve else {
            continue;
        };
        let (start, end) = curve.domain();
        let rounding = 64.0 * f64::EPSILON * start.abs().max(end.abs()).max(end - start);
        // A cut within rounding of a knot (or an end) is at that knot: cut
        // a rounding away from it, the piece would begin with a span of no
        // length, and its derivative there would vanish.
        let snap = |t: f64| {
            curve
                .knots()
                .iter()
                .copied()
                .find(|k| (k - t).abs() <= rounding)
                .unwrap_or(t)
        };
        let (from, to) = (snap(edge.range.0), snap(edge.range.1));
        // Already the whole curve.
        if from <= start && to >= end {
            continue;
        }
        if let Ok(piece) = craft3d_geom::refine::piece(curve, from, to) {
            let domain = piece.domain();
            edge.curve = Curve3::Nurbs(piece);
            edge.range = domain;
        }
    }
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
    crate::work::boolean();
    if a.is_tolerant() || b.is_tolerant() {
        // Loose entities: at the operands' largest tolerance, the result's
        // entities carrying what they were built at (`loose`).
        let cap = loose::cap(tolerance, options);
        let working = loose::working_tolerance(a, b, tolerance.linear, cap)?;
        // Each operand held to the document's tolerance and its own
        // table first: a gap no entity records is a defect, not something
        // the working tolerance may hide.
        if !options.trust_operands {
            loose::operands_valid(a, b, tolerance, cap)?;
        }
        let at = Tolerance {
            linear: working,
            ..*tolerance
        };
        let inner = BooleanOptions {
            trust_operands: true,
            ..*options
        };
        let mut result = exact(a, b, op, &at, &inner)?;
        loose::carry(&mut result, a, b, tolerance, cap, working)?;
        return Ok(result);
    }
    exact(a, b, op, tolerance, options)
}

/// [`boolean_with`] of bodies held to one tolerance.
fn exact(
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
    if body.kind != BodyKind::Solid {
        return Err(unsupported(Stop::of(NOT_SOLID)));
    }
    // The vertices are not merged with anything here.
    let mut vertices = Vertices::new(0.0, 0.0);
    let solid =
        section::prepare(body, Operand::A, &mut vertices, tolerance, false).map_err(unsupported)?;
    let mut found = Vec::with_capacity(points.len());
    for &point in points {
        // A point can be dear to classify among freeform faces: the
        // operation's work bound, when one is in force, is asked per point.
        if craft3d_geom::allowance::pass() {
            return Err(KernelError::Unsupported {
                operation: "point classification",
                reason: "the work bound of the operation it is part of was reached",
                faces: Vec::new(),
            });
        }
        found.push(match classify::inside(&solid, point, tolerance) {
            Ok(inside) => Some(inside),
            Err(Stop::Unsupported { .. }) => None,
            Err(stop) => return Err(unsupported(stop)),
        });
    }
    Ok(found)
}

/// Where a point lies against a solid ([`classify_points`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Inside,
    Outside,
    /// Within twice the tolerance of the solid's boundary.
    OnBoundary,
    /// No ray could tell.
    Unknown,
}

/// [`contains_points`] that tells a point on the boundary from one no ray
/// could place.
pub fn classify_points(
    body: &Body,
    points: &[Vec3],
    tolerance: &Tolerance,
) -> Result<Vec<Place>, KernelError> {
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
    if body.kind != BodyKind::Solid {
        return Err(unsupported(Stop::of(NOT_SOLID)));
    }
    let mut vertices = Vertices::new(0.0, 0.0);
    let solid =
        section::prepare(body, Operand::A, &mut vertices, tolerance, false).map_err(unsupported)?;
    let mut found = Vec::with_capacity(points.len());
    for &point in points {
        found.push(match classify::inside(&solid, point, tolerance) {
            Ok(true) => Place::Inside,
            Ok(false) => Place::Outside,
            Err(Stop::Unsupported { reason, .. }) if reason == ON_A_FACE => Place::OnBoundary,
            Err(Stop::Unsupported { .. }) => Place::Unknown,
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
        let defects = craft3d_topo::check(&bad, &tolerance);
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
