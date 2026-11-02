//! Tolerant entities: a tolerance of its own per vertex and per edge, for
//! geometry that is not exact to the document's tolerance (a STEP file
//! whose edges lie off their faces, an approximation that cannot reach
//! the document's tolerance). The developer's decision of 2026-10-18
//! (`docs/decisions.md`, *Tolerant modelling*; `docs/design/topology.md`,
//! Method › *Tolerant entities*).
//!
//! **The table.** [`Tolerances`] is a sparse, ordered table on the body:
//! an entity that is not in it has the document's tolerance (every body
//! 3DCraft builds itself has an empty table, and is then exactly what it
//! was before the table existed, bit for bit). A tolerance in it is a
//! *ball* for a vertex (the curves of its edges end within it) and a
//! *tube* for an edge (the surfaces of its faces pass within it of every
//! point of its curve). An entity's tolerance is never tighter than the
//! document's: the one that holds is the larger of the two
//! ([`Body::vertex_tolerance_at`], [`Body::edge_tolerance_at`]). The
//! invariant, as other modellers keep it: a vertex is at least as loose as
//! every edge that ends at it ([`Body::set_edge_tolerance`] raises the
//! vertices; [`crate::check`] reports a vertex left tighter).
//!
//! **The cap.** No entity may be looser than a cap, document data
//! (`doc::Tolerance::loosest`, by default [`DEFAULT_LOOSEST`] times the
//! linear tolerance): [`crate::check_capped`] reports an entity above it,
//! and an operation on such a body refuses rather than let a tolerance
//! grow unseen.
//!
//! **Measuring.** [`measure_edge`] proves how far an edge's curve is from
//! the surfaces of its faces over the whole edge
//! (`craft3d_geom::certify::curve_on_surface`, not samples), and
//! [`measure_vertex`] how far the ends of its edges are from a vertex;
//! [`record_edge`] and [`record_vertex`] put what was measured in the
//! table, so that sewing and import can set it.

use crate::{Body, EdgeId, VertexId};
use craft3d_geom::certify::{CurveBound, Within, curve_on_surface};
use craft3d_geom::{Curve3, Surface};
use craft3d_math::{Bits, Tolerance, Vec3};
use std::collections::BTreeMap;
use std::fmt;

pub use craft3d_math::DEFAULT_LOOSEST;

/// The most pieces a proof that an edge lies within its tolerance of a
/// surface may take (the measure of 2026-10-18 proved every coedge of the
/// benchmark's models but circles on tori and some splines within this at
/// the document's tolerance; a loose edge is proven at its own, looser).
pub const PROOF_PIECES: usize = 1 << 16;

/// A measured distance is recorded this much larger. Not for the rounding
/// of an isometry (about 1e-15 of the size), but because what is checked
/// later is a *proof*, not the distance: `certify::curve_on_surface`
/// bounds each piece of the curve by its ends' distances plus a
/// derivative term that shrinks with the piece, and halves pieces until
/// that bound is within the target. A target equal to the true distance
/// can never be met (the term never reaches zero); one an eighth above it
/// is met once the pieces are short enough for the term to be an eighth
/// of the distance, i.e. after a few halvings more than the measure took,
/// wherever the body is moved. An eighth keeps the recorded tolerance
/// close to the gap (1.125 times it; other modellers record the gap plus
/// rounding) at a bounded cost in pieces.
pub const RECORD_MARGIN: f64 = 1.125;

/// The tolerances of a body's loose vertices and edges; an entity not in
/// it has the document's. Ordered, so that what is built from it does
/// not depend on the order it was filled in.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Tolerances {
    vertices: BTreeMap<VertexId, f64>,
    edges: BTreeMap<EdgeId, f64>,
}

impl Tolerances {
    /// No entity has a tolerance of its own: the body is exact to the
    /// document's tolerance.
    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty() && self.edges.is_empty()
    }

    /// The vertices with a tolerance of their own, in order.
    pub fn vertices(&self) -> impl Iterator<Item = (VertexId, f64)> + '_ {
        self.vertices.iter().map(|(v, t)| (*v, *t))
    }

    /// The edges with a tolerance of their own, in order.
    pub fn edges(&self) -> impl Iterator<Item = (EdgeId, f64)> + '_ {
        self.edges.iter().map(|(e, t)| (*e, *t))
    }

    /// The largest tolerance in the table, `None` when it is empty.
    pub fn largest(&self) -> Option<f64> {
        self.vertices
            .values()
            .chain(self.edges.values())
            .copied()
            .reduce(f64::max)
    }

    /// Adds `other`'s entries with their ids mapped (what `merge` does to
    /// a body's ids).
    pub(crate) fn extend_shifted<E>(
        &mut self,
        other: &Tolerances,
        vertex: impl Fn(VertexId) -> Result<VertexId, E>,
        edge: impl Fn(EdgeId) -> Result<EdgeId, E>,
    ) -> Result<(), E> {
        for (v, t) in &other.vertices {
            self.vertices.insert(vertex(*v)?, *t);
        }
        for (e, t) in &other.edges {
            self.edges.insert(edge(*e)?, *t);
        }
        Ok(())
    }
}

impl Bits for Tolerances {
    /// Nothing for an empty table, so that an exact body is written out
    /// as it was before the table existed.
    fn push_bits(&self, out: &mut Vec<u64>) {
        if self.is_empty() {
            return;
        }
        // A word no count of shells is, then both lists.
        out.push(u64::MAX);
        self.vertices.len().push_bits(out);
        for (v, t) in &self.vertices {
            v.0.push_bits(out);
            t.push_bits(out);
        }
        self.edges.len().push_bits(out);
        for (e, t) in &self.edges {
            e.0.push_bits(out);
            t.push_bits(out);
        }
    }
}

/// A tolerance that cannot be an entity's, or an entity that does not
/// exist.
#[derive(Clone, Debug, PartialEq)]
pub enum ToleranceError {
    /// Not positive and finite.
    NotPositive {
        value: f64,
    },
    NoVertex {
        vertex: VertexId,
    },
    NoEdge {
        edge: EdgeId,
    },
    /// The edge could not be proven within any tolerance up to `cap` of
    /// the surface of one of its faces: `at` is the curve's parameter
    /// where the proof stopped.
    EdgeNotMeasured {
        edge: EdgeId,
        at: f64,
        cap: f64,
    },
    /// What was measured is above the cap.
    AboveCap {
        measured: f64,
        cap: f64,
    },
}

impl fmt::Display for ToleranceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ToleranceError::NotPositive { value } => {
                write!(f, "a tolerance must be positive and finite, got {value:e}")
            }
            ToleranceError::NoVertex { vertex } => {
                write!(f, "vertex {} does not exist", vertex.0)
            }
            ToleranceError::NoEdge { edge } => write!(f, "edge {} does not exist", edge.0),
            ToleranceError::EdgeNotMeasured { edge, at, cap } => write!(
                f,
                "edge {} could not be proven within {cap:e} of its faces' surfaces (the proof stopped at parameter {at})",
                edge.0
            ),
            ToleranceError::AboveCap { measured, cap } => write!(
                f,
                "a measured tolerance of {measured:e} is above the cap of {cap:e} the document allows any entity"
            ),
        }
    }
}

impl std::error::Error for ToleranceError {}

fn positive(t: f64) -> Result<f64, ToleranceError> {
    if t.is_finite() && t > 0.0 {
        Ok(t)
    } else {
        Err(ToleranceError::NotPositive { value: t })
    }
}

impl Body {
    /// The vertex's own tolerance, `None` for the document's.
    pub fn vertex_tolerance(&self, vertex: VertexId) -> Option<f64> {
        self.tolerances.vertices.get(&vertex).copied()
    }

    /// The edge's own tolerance, `None` for the document's.
    pub fn edge_tolerance(&self, edge: EdgeId) -> Option<f64> {
        self.tolerances.edges.get(&edge).copied()
    }

    /// The tolerance that holds for the vertex in a document whose linear
    /// tolerance is `linear`: its own or the document's, the larger.
    pub fn vertex_tolerance_at(&self, vertex: VertexId, linear: f64) -> f64 {
        self.vertex_tolerance(vertex)
            .map_or(linear, |t| t.max(linear))
    }

    /// The tolerance that holds for the edge in a document whose linear
    /// tolerance is `linear`: its own or the document's, the larger.
    pub fn edge_tolerance_at(&self, edge: EdgeId, linear: f64) -> f64 {
        self.edge_tolerance(edge).map_or(linear, |t| t.max(linear))
    }

    /// Gives the vertex a tolerance of its own (replacing any it had). A
    /// vertex left tighter than an edge that ends at it is a defect
    /// ([`crate::Defect::VertexTighterThanEdge`]).
    pub fn set_vertex_tolerance(
        &mut self,
        vertex: VertexId,
        tolerance: f64,
    ) -> Result<(), ToleranceError> {
        let tolerance = positive(tolerance)?;
        if self.vertex(vertex).is_none() {
            return Err(ToleranceError::NoVertex { vertex });
        }
        self.tolerances.vertices.insert(vertex, tolerance);
        Ok(())
    }

    /// Gives the edge a tolerance of its own (replacing any it had), and
    /// raises its vertices to at least it, which keeps the invariant.
    pub fn set_edge_tolerance(
        &mut self,
        edge: EdgeId,
        tolerance: f64,
    ) -> Result<(), ToleranceError> {
        let tolerance = positive(tolerance)?;
        let Some(e) = self.edge(edge) else {
            return Err(ToleranceError::NoEdge { edge });
        };
        let ends = [e.start, e.end];
        self.tolerances.edges.insert(edge, tolerance);
        for v in ends {
            if self.vertex(v).is_some() && self.vertex_tolerance(v).is_none_or(|t| t < tolerance) {
                self.tolerances.vertices.insert(v, tolerance);
            }
        }
        Ok(())
    }

    /// The vertex has the document's tolerance again.
    pub fn clear_vertex_tolerance(&mut self, vertex: VertexId) {
        self.tolerances.vertices.remove(&vertex);
    }

    /// The edge has the document's tolerance again (its vertices keep
    /// theirs).
    pub fn clear_edge_tolerance(&mut self, edge: EdgeId) {
        self.tolerances.edges.remove(&edge);
    }

    /// Whether any entity has a tolerance of its own.
    pub fn is_tolerant(&self) -> bool {
        !self.tolerances.is_empty()
    }

    /// The tolerance the kernel works at on this body in a document whose
    /// linear tolerance is `linear`: the largest of the entities' and the
    /// document's.
    pub fn working_tolerance(&self, linear: f64) -> f64 {
        self.tolerances.largest().map_or(linear, |t| t.max(linear))
    }
}

/// How far the edge's curve is from the surface of each face that uses
/// it, proven over the whole edge: the least bound proven, in steps of
/// two from the farthest of five places along it (at least the document's
/// tolerance), up to `cap`. `Ok(None)` for an edge no face uses.
pub fn measure_edge(
    body: &Body,
    edge: EdgeId,
    tolerance: &Tolerance,
    cap: f64,
) -> Result<Option<f64>, ToleranceError> {
    let e = body.edge(edge).ok_or(ToleranceError::NoEdge { edge })?;
    let mut worst: Option<f64> = None;
    for face in faces_of(body, edge) {
        let bound = measure_curve(&e.curve, e.range, &face.surface, tolerance, cap)
            .map_err(|at| ToleranceError::EdgeNotMeasured { edge, at, cap })?;
        worst = Some(worst.map_or(bound, |w: f64| w.max(bound)));
    }
    Ok(worst)
}

/// How far `curve` over `range` is from `surface`, proven over the whole
/// range: the least bound proven, in steps of two from twice the farthest
/// of five places along it (at least the document's tolerance), up to
/// `cap`; `Err` with the parameter where the last proof stopped when none
/// within the cap is found.
pub fn measure_curve(
    curve: &Curve3,
    range: (f64, f64),
    surface: &Surface,
    tolerance: &Tolerance,
    cap: f64,
) -> Result<f64, f64> {
    let bounded = CurveBound::new(curve);
    let (sampled, start) = walk(curve, range, surface, cap);
    let mut target = (2.0 * sampled).max(tolerance.linear);
    let mut stopped = 0.5 * (range.0 + range.1);
    while target.is_finite() && target <= 2.0 * cap {
        match curve_on_surface(
            &bounded,
            range,
            surface,
            start,
            &[],
            Within {
                target,
                most: PROOF_PIECES,
            },
        ) {
            Ok(proven) => {
                let bound = proven.bound.max(sampled);
                // A proof at a target above the cap may bound a gap
                // within it by more than the cap (the bound lies
                // anywhere up to the target): once more at the cap,
                // which a gap within it can meet (2026-11-01, ABC).
                if bound > cap && target > cap && sampled < cap {
                    let tighter = Within {
                        target: cap,
                        most: PROOF_PIECES,
                    };
                    if let Ok(p) = curve_on_surface(&bounded, range, surface, start, &[], tighter) {
                        return Ok(p.bound.max(sampled));
                    }
                }
                return Ok(bound);
            }
            Err(not) => stopped = not.at,
        }
        target *= 2.0;
    }
    Err(stopped)
}

/// Places per quarter of a curve where its feet on a surface are walked
/// from one to the next: at least eight, two per span of a spline curve,
/// at most 256.
fn walk_steps(curve: &Curve3) -> u32 {
    let spans = match curve {
        Curve3::Nurbs(n) => {
            let mut k = n.knots().to_vec();
            k.dedup();
            k.len()
        }
        _ => 0,
    };
    u32::try_from(spans / 2).unwrap_or(u32::MAX).clamp(8, 256)
}

/// The most values per direction of the grid [`nearest_foot`] looks at.
const SEED_GRID: usize = 129;

/// The values of a grid over a spline's parameter range: every distinct
/// knot within the domain and the middle of every span, at most
/// [`SEED_GRID`] (every so many of them when there are more).
fn seed_values(knots: &[f64], (lo, hi): (f64, f64)) -> Vec<f64> {
    let mut distinct: Vec<f64> = knots
        .iter()
        .copied()
        .filter(|k| *k >= lo && *k <= hi && k.is_finite())
        .collect();
    distinct.dedup();
    let mut all = Vec::with_capacity(2 * distinct.len());
    for pair in distinct.windows(2) {
        if let [a, b] = pair {
            all.push(*a);
            all.push(0.5 * (a + b));
        }
    }
    if let Some(last) = distinct.last() {
        all.push(*last);
    }
    if all.len() <= SEED_GRID {
        return all;
    }
    let step = all.len().div_ceil(SEED_GRID);
    all.into_iter().step_by(step.max(1)).collect()
}

/// The foot of `p` on `surface`, the nearest found: the surface's own
/// projection, and for a spline surface the walks (`project_from`) from
/// the four nearest points of a grid at its knots and the middles of its
/// spans. The projection searches a grid of fixed size, which on a spline
/// of many spans (a rational surface of 96 spans round, ABC 2026-11-01)
/// may land in another dip of the distance, a millimetre or more away.
/// `None` when nothing is found.
pub fn nearest_foot(surface: &Surface, p: Vec3) -> Option<(f64, f64)> {
    let off = |f: (f64, f64)| {
        let d = surface.eval(f.0, f.1).distance(p);
        if d.is_finite() { d } else { f64::INFINITY }
    };
    let mut best = surface.project(p).ok().map(|f| (off(f), f));
    let (us, vs) = match surface {
        Surface::Nurbs(n) => {
            let (du, dv) = surface.domain();
            (seed_values(n.knots_u(), du), seed_values(n.knots_v(), dv))
        }
        _ => return best.map(|(_, f)| f),
    };
    let mut near: Vec<(f64, (f64, f64))> = Vec::with_capacity(us.len() * vs.len());
    for &u in &us {
        for &v in &vs {
            near.push((off((u, v)), (u, v)));
        }
    }
    near.sort_by(|a, b| {
        a.0.total_cmp(&b.0)
            .then(a.1.0.total_cmp(&b.1.0))
            .then(a.1.1.total_cmp(&b.1.1))
    });
    for &(_, seed) in near.iter().take(4) {
        if let Ok(f) = surface.project_from(p, seed) {
            let d = off(f);
            if best.is_none_or(|(b, _)| d < b) {
                best = Some((d, f));
            }
        }
    }
    best.map(|(_, f)| f)
}

/// The farthest of five places along `curve` over `range` from
/// `surface`, and where the proof should start (`None`: the proof
/// projects the start itself, and steps off a seam or pole from there).
/// When the surface's own projection puts each of the five within `cap`,
/// that is all (as before 2026-11-01). Otherwise a projection may have
/// landed in another dip of the distance, far from the curve, which made
/// the first target larger than the cap so that the edge was never tried
/// (140 ABC models on rational surfaces of 60 to 128 spans): each of the
/// five feet is then the nearest of [`nearest_foot`] and the foot walked
/// on (`project_from`) from the place before, through places in between,
/// and the start is the nearest of its own projection, [`nearest_foot`]
/// and the foot walked back from the end.
fn walk(
    curve: &Curve3,
    range: (f64, f64),
    surface: &Surface,
    cap: f64,
) -> (f64, Option<(f64, f64)>) {
    let (t0, t1) = range;
    let off = |p: Vec3, f: (f64, f64)| {
        let d = surface.eval(f.0, f.1).distance(p);
        if d.is_finite() { d } else { f64::INFINITY }
    };
    let five: Vec<f64> = (0..=4)
        .map(|k| {
            let p = curve.eval(t0 + (t1 - t0) * f64::from(k) / 4.0);
            surface.project(p).ok().map_or(f64::INFINITY, |f| off(p, f))
        })
        .collect();
    let sampled = five.iter().copied().fold(0.0, f64::max);
    if sampled <= cap {
        return (sampled, None);
    }
    let quarter = walk_steps(curve);
    let all = 4 * quarter;
    let at = |k: u32| curve.eval(t0 + (t1 - t0) * f64::from(k) / f64::from(all));
    let nearer = |p: Vec3, a: Option<(f64, f64)>, b: Option<(f64, f64)>| match (a, b) {
        (Some(a), Some(b)) => Some(if off(p, b) < off(p, a) { b } else { a }),
        (a, b) => a.or(b),
    };
    let mut sampled = 0.0_f64;
    let mut last: Option<(f64, f64)> = None;
    let mut first: Option<(f64, f64)> = None;
    for k in 0..=all {
        let p = at(k);
        let walked = last.and_then(|l| surface.project_from(p, l).ok());
        if k % quarter == 0 {
            let foot = nearer(p, nearest_foot(surface, p), walked);
            sampled = sampled.max(foot.map_or(f64::INFINITY, |f| off(p, f)));
            last = foot;
            if k == 0 {
                first = foot;
            }
        } else {
            last = walked.or(last);
        }
    }
    // Back from the end's foot to the start, a place at a time.
    let mut back = last;
    for k in (0..all).rev() {
        back = back.and_then(|b| surface.project_from(at(k), b).ok());
    }
    let p = at(0);
    let own = surface.project(p).ok();
    let candidate = nearer(p, first, back);
    let start = match (own, candidate) {
        (Some(o), Some(c)) if off(p, c) < off(p, o) => Some(c),
        (None, c) => c,
        _ => None,
    };
    (sampled, start)
}

/// How far the ends of the edges at the vertex are from it, and at least
/// the tolerance of each of those edges; `None` for a vertex no edge
/// ends at.
pub fn measure_vertex(body: &Body, vertex: VertexId) -> Result<Option<f64>, ToleranceError> {
    let point = body
        .vertex(vertex)
        .ok_or(ToleranceError::NoVertex { vertex })?
        .point;
    let mut worst: Option<f64> = None;
    for (i, e) in body.edges.iter().enumerate() {
        let Ok(i) = u32::try_from(i) else {
            break;
        };
        let own = body.edge_tolerance(EdgeId(i)).unwrap_or(0.0);
        for (v, t) in [(e.start, e.range.0), (e.end, e.range.1)] {
            if v == vertex {
                let d = e.curve.eval(t).distance(point);
                let d = if d.is_finite() { d } else { f64::INFINITY };
                worst = Some(worst.unwrap_or(0.0).max(d).max(own));
            }
        }
    }
    Ok(worst)
}

/// Measures the edge ([`measure_edge`]) and records what it measured, a
/// little larger ([`RECORD_MARGIN`]), as its tolerance when that is
/// looser than the document's (its vertices raised to it); an edge within
/// the document's tolerance is left as it is. Returns the tolerance
/// recorded. Refused above `cap`.
pub fn record_edge(
    body: &mut Body,
    edge: EdgeId,
    tolerance: &Tolerance,
    cap: f64,
) -> Result<Option<f64>, ToleranceError> {
    let Some(measured) = measure_edge(body, edge, tolerance, cap)? else {
        return Ok(None);
    };
    record(measured, tolerance, cap, |t| {
        body.set_edge_tolerance(edge, t)
    })
}

/// Measures the vertex ([`measure_vertex`]) and records it as
/// [`record_edge`] does.
pub fn record_vertex(
    body: &mut Body,
    vertex: VertexId,
    tolerance: &Tolerance,
    cap: f64,
) -> Result<Option<f64>, ToleranceError> {
    let Some(measured) = measure_vertex(body, vertex)? else {
        return Ok(None);
    };
    record(measured, tolerance, cap, |t| {
        body.set_vertex_tolerance(vertex, t)
    })
}

/// A vertex and the tolerance recorded for it, `None` when it lies
/// within the document's.
pub type Recorded = (VertexId, Option<f64>);

/// [`record_vertex`] for each of `vertices` (in order), with one pass
/// over the edges rather than one per vertex: what each records, or the
/// first refusal with the vertex it is for.
pub fn record_vertices(
    body: &mut Body,
    vertices: &[VertexId],
    tolerance: &Tolerance,
    cap: f64,
) -> Result<Vec<Recorded>, (VertexId, ToleranceError)> {
    let mut worst: BTreeMap<VertexId, f64> = BTreeMap::new();
    for v in vertices {
        if body.vertex(*v).is_none() {
            return Err((*v, ToleranceError::NoVertex { vertex: *v }));
        }
    }
    let wanted: std::collections::BTreeSet<VertexId> = vertices.iter().copied().collect();
    for (i, e) in body.edges.iter().enumerate() {
        let Ok(i) = u32::try_from(i) else {
            break;
        };
        let own = body.edge_tolerance(EdgeId(i)).unwrap_or(0.0);
        for (v, t) in [(e.start, e.range.0), (e.end, e.range.1)] {
            if !wanted.contains(&v) {
                continue;
            }
            let Some(point) = body.vertex(v).map(|x| x.point) else {
                continue;
            };
            let d = e.curve.eval(t).distance(point);
            let d = if d.is_finite() { d } else { f64::INFINITY };
            let slot = worst.entry(v).or_insert(0.0);
            *slot = slot.max(d).max(own);
        }
    }
    let mut out = Vec::with_capacity(vertices.len());
    for v in vertices {
        let recorded = match worst.get(v) {
            None => None,
            Some(&measured) => record(measured, tolerance, cap, |t| {
                body.set_vertex_tolerance(*v, t)
            })
            .map_err(|e| (*v, e))?,
        };
        out.push((*v, recorded));
    }
    Ok(out)
}

fn record(
    measured: f64,
    tolerance: &Tolerance,
    cap: f64,
    set: impl FnOnce(f64) -> Result<(), ToleranceError>,
) -> Result<Option<f64>, ToleranceError> {
    if measured <= tolerance.linear {
        return Ok(None);
    }
    // The cap is on the gap: a gap within it is recorded at most at the
    // cap, the margin never pushes it over (since slice 2, 2026-10-22).
    if measured.is_nan() || measured > cap {
        return Err(ToleranceError::AboveCap {
            measured: measured * RECORD_MARGIN,
            cap,
        });
    }
    let recorded = (measured * RECORD_MARGIN).min(cap).max(tolerance.linear);
    set(recorded)?;
    Ok(Some(recorded))
}

/// The faces whose loops use the edge, in face order, each once.
fn faces_of(body: &Body, edge: EdgeId) -> Vec<&crate::Face> {
    body.faces
        .iter()
        .filter(|face| {
            face.loops.iter().any(|l| {
                body.loop_(*l)
                    .is_some_and(|l| l.coedges.iter().any(|c| c.edge == edge))
            })
        })
        .collect()
}

/// The cap in a document whose linear tolerance is `linear` and that sets
/// none of its own.
pub fn default_loosest(linear: f64) -> f64 {
    DEFAULT_LOOSEST * linear
}
