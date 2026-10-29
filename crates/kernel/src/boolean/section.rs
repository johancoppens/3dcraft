//! Preparing the operands and finding where their faces meet: the curves
//! in which two surfaces meet, cut down to the stretches that are within
//! both faces.

use super::coincide::{Coincidence, PARTLY, coincide};
use super::cover::{Cover, Face as CoverFace, clear_of_surface};
use super::facetree::FaceTree;
use super::freeform::is_freeform;
use super::hits::{curve_hits, signed_distance};
use super::outline::{
    Hull, Outline, Seg, Sheet, Side, boxes_meet, hull_of, locate, nearest, onto, outward_normal,
    piece_box, surface_distance,
};
use super::{
    AMBIGUOUS, LOST, Meeting, NEAR, NEARLY_COINCIDENT, NEARLY_TOUCHING, Operand, Reason, Stop,
    Vertices, face_id, geometry,
};
use crate::{KernelError, MAX_REPORTED};
use craft3d_geom::certify::{CurveBound, Within, curve_on_surface};
use craft3d_geom::intersect::CurveIntersection;
use craft3d_geom::intersect::{MeetingCurve, SurfaceMeeting, meet_surfaces};
use craft3d_geom::{Curve3, GeomError, Surface};
use craft3d_math::{Bbox3, Tolerance, Vec3};
use craft3d_topo::{Body, EdgeId, FaceId, VertexId};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

/// The outline of a face, made the first time it is asked for. Making
/// the outline of a freeform face indexes its surface (a tree of up to
/// 2^15 pieces, tens of megabytes), and of a body with thousands of faces
/// most are nowhere near the other operand: their boxes and hulls, which
/// need no outline, say so. An outline is a function of the face alone,
/// so when it is made does not change it.
pub(super) struct Lazy {
    surface: Surface,
    reversed: bool,
    loops: Vec<Vec<Seg>>,
    margin: f64,
    made: OnceLock<Result<Outline, Reason>>,
}

impl Lazy {
    fn get(&self) -> Result<&Outline, Reason> {
        self.made
            .get_or_init(|| {
                Outline::new(
                    self.surface.clone(),
                    self.reversed,
                    self.loops.clone(),
                    self.margin,
                )
            })
            .as_ref()
            .map_err(|reason| *reason)
    }
}

/// One operand, prepared.
pub(super) struct Solid<'a> {
    pub operand: Operand,
    pub body: &'a Body,
    /// Per face: its outline, made when needed ([`Solid::outline`]).
    outlines: Vec<Lazy>,
    pub boxes: Vec<Bbox3>,
    /// Per face: the box of [`Hull::near_bbox`], each edge held by the
    /// part of its curve it is: for a pair whose surfaces cannot be met
    /// in the box of `boxes` (`meet_in`).
    pub near: Vec<Bbox3>,
    pub hulls: Vec<Hull>,
    /// The merged vertex of each of the body's vertices.
    pub vertices: Vec<usize>,
    /// The tree of `boxes`, made the first time it is asked for.
    tree: OnceLock<FaceTree>,
    /// Per face: its edges, each once and ascending, and the tree of their
    /// boxes ([`piece_box`]), made the first time they are asked for.
    edge_trees: Vec<OnceLock<(Vec<EdgeId>, FaceTree)>>,
    /// Per face: the boxes that cover it ([`Cover`]), made as they are
    /// asked for; `None` for a face its box stands for.
    covers: Vec<OnceLock<Option<Cover>>>,
}

impl Solid<'_> {
    pub fn name(&self, face: usize) -> (Operand, FaceId) {
        (self.operand, face_id(face))
    }

    /// How many faces the solid has.
    pub fn faces(&self) -> usize {
        self.outlines.len()
    }

    /// The outline of a face, or why it has none (with the face named).
    pub fn outline(&self, face: usize) -> Result<&Outline, Stop> {
        self.outlines
            .get(face)
            .ok_or(Stop::of(LOST))?
            .get()
            .map_err(|reason| Stop::of(reason).at(&[self.name(face)]))
    }

    /// The edges around a face, each once.
    pub fn edges_of(&self, face: usize) -> Vec<EdgeId> {
        let mut edges = BTreeSet::new();
        if let Some(f) = self.body.faces.get(face) {
            for loop_ in f.loops.iter().filter_map(|&l| self.body.loop_(l)) {
                edges.extend(loop_.coedges.iter().map(|c| c.edge));
            }
        }
        edges.into_iter().collect()
    }

    pub fn ends(&self, edge: EdgeId) -> Option<(usize, usize)> {
        let e = self.body.edge(edge)?;
        Some((
            *self.vertices.get(e.start.0 as usize)?,
            *self.vertices.get(e.end.0 as usize)?,
        ))
    }

    /// The faces whose boxes may pass `test`, ascending: all that pass it
    /// and some that do not, for the caller to test as it would test every
    /// face. `test` must pass for a box whenever it passes for a box inside
    /// it ([`FaceTree`]).
    pub fn candidates(&self, test: impl Fn(&Bbox3) -> bool) -> Vec<usize> {
        self.tree
            .get_or_init(|| FaceTree::new(&self.boxes))
            .candidates(test)
    }

    /// The edges around a face whose boxes pass `test` (and those whose
    /// boxes are not finite), each once and ascending. `test` must pass for
    /// a box whenever it passes for a box inside it.
    pub fn edges_near(&self, face: usize, test: impl Fn(&Bbox3) -> bool) -> Vec<EdgeId> {
        let Some(cell) = self.edge_trees.get(face) else {
            return Vec::new();
        };
        let (edges, tree) = cell.get_or_init(|| {
            let edges = self.edges_of(face);
            let boxes: Vec<Bbox3> = edges.iter().map(|&e| self.edge_box(e)).collect();
            let tree = FaceTree::new(&boxes);
            (edges, tree)
        });
        tree.candidates(test)
            .into_iter()
            .filter_map(|i| edges.get(i).copied())
            .collect()
    }

    /// The boxes that cover a face at the linear tolerance `tol`, or
    /// `None` when its box stands for it ([`Cover`]).
    pub fn cover(&self, face: usize, tol: f64) -> Option<(&Cover, CoverFace<'_>)> {
        #[cfg(test)]
        if tests::NO_COVERS.with(std::cell::Cell::get) {
            return None;
        }
        let of = CoverFace {
            surface: &self.body.faces.get(face)?.surface,
            bbox: self.boxes.get(face)?,
            outline: self.outline(face).ok(),
            tol,
        };
        let cover = self
            .covers
            .get(face)?
            .get_or_init(|| Cover::new(of))
            .as_ref()?;
        Some((cover, of))
    }

    /// False only when `p` is proven further than four times the tolerance
    /// `tol` from face `face` by its cover: then it is outside the face
    /// (`Side::Out`, and not within the tolerance of its boundary), and
    /// the face's outline need not be asked.
    pub fn may_hold(&self, face: usize, p: Vec3, tol: f64) -> bool {
        self.cover(face, tol)
            .is_none_or(|(cover, of)| cover.holds(of, p, 4.0 * tol))
    }

    /// A box around an edge; empty for an edge the body does not have.
    pub fn edge_box(&self, id: EdgeId) -> Bbox3 {
        let Some(edge) = self.body.edge(id) else {
            return Bbox3::EMPTY;
        };
        match (self.body.vertex(edge.start), self.body.vertex(edge.end)) {
            (Some(a), Some(b)) => piece_box(&edge.curve, a.point, b.point),
            _ => Bbox3::EMPTY,
        }
    }

    /// A box around the whole body.
    pub fn bbox(&self) -> Bbox3 {
        if let Ok(b) = self.body.bbox() {
            return b;
        }
        let mut all = Bbox3::EMPTY;
        for (lo, hi) in self.boxes.iter().filter_map(Bbox3::corners) {
            all.add(lo);
            all.add(hi);
        }
        all
    }
}

/// A stretch of an intersection curve within both faces.
pub(super) struct Stretch {
    pub curve: craft3d_geom::Curve3,
    pub range: (f64, f64),
    pub start: usize,
    pub end: usize,
    pub fa: usize,
    pub fb: usize,
    /// Strictly inside the face, rather than along its boundary.
    pub inside_a: bool,
    pub inside_b: bool,
    /// The surfaces cross along it, rather than touch without crossing.
    pub crossing: bool,
}

#[derive(Default)]
pub(super) struct Sections {
    pub stretches: Vec<Stretch>,
    /// Per face of A: the faces of B on the same surface, and whether the
    /// surfaces' normals agree. And the other way round.
    pub coincident_a: BTreeMap<usize, Vec<(usize, bool)>>,
    pub coincident_b: BTreeMap<usize, Vec<(usize, bool)>>,
    /// Points where two faces' surfaces only touch (a ball on a plane):
    /// they divide nothing, but are where the bodies meet.
    pub touches: Vec<Vec3>,
    /// Points where an edge of one face meets the surface of the other
    /// of a pair whose surfaces meet in a curve: where the bodies meet,
    /// also where no stretch of the curve is left between two of them (a
    /// circular edge of one body that touches the other's face in a point
    /// from outside it).
    pub edge_meets: Vec<Vec3>,
    /// Per edge: the vertices to split it at.
    pub splits_a: BTreeMap<EdgeId, BTreeSet<usize>>,
    pub splits_b: BTreeMap<EdgeId, BTreeSet<usize>>,
}

/// The outline of face `index` of `body`, as [`prepare`] makes it.
pub(super) fn face_outline(body: &Body, index: usize, tol: f64) -> Result<Outline, Stop> {
    let face = body.faces.get(index).ok_or(Stop::of(LOST))?;
    let mut loops = Vec::with_capacity(face.loops.len());
    for loop_ in face.loops.iter().filter_map(|&l| body.loop_(l)) {
        let mut segs = Vec::with_capacity(loop_.coedges.len());
        for coedge in &loop_.coedges {
            let edge = body.edge(coedge.edge).ok_or(Stop::of(LOST))?;
            let (Some(a), Some(b)) = (body.vertex(edge.start), body.vertex(edge.end)) else {
                return Err(Stop::of(LOST));
            };
            segs.push(Seg {
                curve: edge.curve.clone(),
                range: edge.range,
                a: a.point,
                b: b.point,
                reversed: coedge.reversed,
            });
        }
        loops.push(segs);
    }
    Outline::new(face.surface.clone(), face.reversed, loops, 4.0 * tol).map_err(Stop::of)
}

pub(super) fn prepare<'a>(
    body: &'a Body,
    operand: Operand,
    vertices: &mut Vertices,
    tolerance: &Tolerance,
    trusted: bool,
) -> Result<Solid<'a>, Stop> {
    let mut defects = if trusted {
        Vec::new()
    } else {
        super::checked(body, tolerance)
    };
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
    let tol = tolerance.linear;
    let mut merged = Vec::with_capacity(body.vertices.len());
    for (index, vertex) in body.vertices.iter().enumerate() {
        let at = vertices.find_or_add(vertex.point)?;
        let meeting = vertices.meetings.get_mut(at).ok_or(Stop::of(LOST))?;
        if meeting.kept.is_none() {
            meeting.kept = Some((operand, VertexId(u32::try_from(index).unwrap_or(u32::MAX))));
        }
        meeting.operands |= match operand {
            Operand::A => 1,
            Operand::B => 2,
        };
        merged.push(at);
    }
    let mut outlines = Vec::with_capacity(body.faces.len());
    let mut boxes = Vec::with_capacity(body.faces.len());
    let mut near = Vec::with_capacity(body.faces.len());
    let mut hulls = Vec::with_capacity(body.faces.len());
    for (index, face) in body.faces.iter().enumerate() {
        let named = [(operand, face_id(index))];
        let mut loops = Vec::with_capacity(face.loops.len());
        for loop_ in face.loops.iter().filter_map(|&l| body.loop_(l)) {
            let mut segs = Vec::with_capacity(loop_.coedges.len());
            for coedge in &loop_.coedges {
                let edge = body.edge(coedge.edge).ok_or(Stop::of(LOST))?;
                let (Some(a), Some(b)) = (body.vertex(edge.start), body.vertex(edge.end)) else {
                    return Err(Stop::of(LOST));
                };
                segs.push(Seg {
                    curve: edge.curve.clone(),
                    range: edge.range,
                    a: a.point,
                    b: b.point,
                    reversed: coedge.reversed,
                });
            }
            loops.push(segs);
        }
        let lazy = Lazy {
            surface: face.surface.clone(),
            reversed: face.reversed,
            loops,
            margin: 4.0 * tol,
            made: OnceLock::new(),
        };
        // The hull asks the outline of a cone (whether the face reaches
        // the apex) and of a torus (whether the face holds a circle of the
        // tube between the turns its boundary takes); those outlines are
        // cheap, and a torus whose outline fails keeps the whole-torus ball.
        let hull = hull_of(&lazy.surface, &lazy.loops, |tip| {
            lazy.get()
                .map_err(|reason| Stop::of(reason).at(&named))
                .map(|outline| !matches!(outline.side(tip, tol), Ok(Side::Out)))
        })?;
        boxes.push(hull.bbox());
        near.push(hull.near_bbox());
        hulls.push(hull);
        outlines.push(lazy);
    }
    Ok(Solid {
        operand,
        body,
        outlines,
        boxes,
        near,
        hulls,
        vertices: merged,
        tree: OnceLock::new(),
        edge_trees: (0..body.faces.len()).map(|_| OnceLock::new()).collect(),
        covers: (0..body.faces.len()).map(|_| OnceLock::new()).collect(),
    })
}

/// What two surfaces do along a curve in which they meet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Passage {
    /// They pass through each other: on either side of the curve one is
    /// twice the tolerance or more beyond the other, on opposite sides.
    Crossing,
    /// One dips through the other and comes back: next to the curve lies
    /// a lens between the two surfaces that is nowhere as thick as twice
    /// the tolerance. The number is its greatest thickness found.
    Lens(f64),
    /// They stay on one side of each other: a touch along the curve.
    Touch,
}

/// How far `surface` goes through `other` beside the point `p` of a curve
/// in which they meet, whose direction there is `tangent`.
///
/// The surface is walked away from the curve at right angles to it, both
/// ways, until it is twice the tolerance from the other surface. Where the
/// two cross plainly that happens on opposite sides of the other surface.
/// Where it happens on the same side both ways, the surface has gone
/// through the other and come back on the way: what lies between is a
/// lens thinner than twice the tolerance, and its thickness is measured.
///
/// The steps grow with the distance walked (a quarter of it, two
/// tolerances at least), so a lens is seen as long as it is not narrower
/// than a few tolerances; a walk that does not get twice the tolerance
/// away within its limits proves nothing, and the curve counts as a
/// crossing, as does everything with a freeform surface.
pub(super) fn passage(
    surface: Sheet,
    other: Sheet,
    p: Vec3,
    tangent: Vec3,
    reach: f64,
    tol: f64,
) -> Passage {
    const STEPS: usize = 160;
    // A freeform surface is taken as crossing: its index answers the walk
    // below at a price, and a lens between it and another surface is a
    // case the corpus has yet to show.
    if is_freeform(surface.surface) || is_freeform(other.surface) {
        return Passage::Crossing;
    }
    let Some(across) =
        outward_normal(surface, false, p).and_then(|normal| normal.cross(tangent).normalized())
    else {
        return Passage::Crossing;
    };
    let distance = |s: f64| {
        onto(surface, p + across * s)
            .and_then(|q| signed_distance(other, q))
            .filter(|d| d.is_finite())
    };
    // Per direction: where the walk got away (how far and on which side),
    // and the samples on the way.
    let mut exits = [None, None];
    let mut walks: [Vec<(f64, f64)>; 2] = [Vec::new(), Vec::new()];
    for ((exit, walk), sign) in exits.iter_mut().zip(walks.iter_mut()).zip([1.0, -1.0]) {
        let mut s = 0.0;
        for _ in 0..STEPS {
            s += (2.0 * tol).max(0.25 * s);
            if s > reach {
                break;
            }
            let Some(d) = distance(sign * s) else {
                break;
            };
            if d.abs() >= 2.0 * tol {
                *exit = Some((s, d > 0.0));
                break;
            }
            walk.push((s, d));
        }
    }
    let [Some((s0, side0)), Some((s1, side1))] = exits else {
        return Passage::Crossing;
    };
    if side0 != side1 {
        return Passage::Crossing;
    }
    // The lens is on the side where the surface was on the other side of
    // `other` first: from the curve to where it comes back.
    let mut thickest = 0.0_f64;
    let mut found = false;
    for ((walk, end), sign) in walks.iter().zip([s0, s1]).zip([1.0, -1.0]) {
        let beyond = |d: f64| d != 0.0 && (d > 0.0) != side0;
        let Some(last) = walk.iter().rev().find(|(_, d)| beyond(*d)).map(|w| w.0) else {
            continue;
        };
        found = true;
        // Up to the next sample, which is on the way out.
        let back = walk.iter().map(|w| w.0).find(|&s| s > last).unwrap_or(end);
        for k in 1..64 {
            let s = back * f64::from(k) / 64.0;
            if let Some(d) = distance(sign * s)
                && beyond(d)
            {
                thickest = thickest.max(d.abs());
            }
        }
    }
    if found {
        Passage::Lens(thickest)
    } else {
        Passage::Touch
    }
}

/// A place where an intersection curve may enter or leave a face.
#[derive(Clone, Copy, Debug)]
struct Cut {
    t: f64,
    point: Vec3,
    a_edge: Option<EdgeId>,
    b_edge: Option<EdgeId>,
    /// Where the curve itself starts or ends, rather than where an edge
    /// meets a surface.
    end: bool,
}

/// Whether two places, each found where an edge meets a surface, are the
/// one place where an edge of A meets an edge of B. Such a place is found
/// twice: as A's edge crossing the surface of a face of B, and as B's
/// edge crossing the surface of a face of A. Each edge is only within the
/// tolerance of the curve it stands for, so where the edges run onto the
/// surfaces at a shallow angle the two places may be further apart than
/// points that are merged. When each lies within the tolerance of both
/// edges, they are that one meeting.
fn one_meeting(
    (a, b): (&Solid, &Solid),
    (p, p_edges): (Vec3, (Option<EdgeId>, Option<EdgeId>)),
    (q, q_edges): (Vec3, (Option<EdgeId>, Option<EdgeId>)),
    tol: f64,
) -> bool {
    let on = |solid: &Solid, edge: Option<EdgeId>, at: Vec3| {
        edge.and_then(|e| solid.body.edge(e))
            .is_some_and(|e| locate(&e.curve, e.range, at, tol).is_some())
    };
    // p found on an edge of A and q on an edge of B, or the other way
    // round, and each within the tolerance of the other's edge too.
    (on(a, p_edges.0, q) && on(b, q_edges.1, p)) || (on(a, q_edges.0, p) && on(b, p_edges.1, q))
}

/// How far, in tolerances, an edge's box may be from where the boxes of
/// two faces overlap and still be asked where it meets the other face's
/// surface ([`edge_cuts`]). A kept stretch has its ends within twice the
/// tolerance of both boxes, a cut is within the tolerance of its edge, and
/// cuts within the tolerance of one another are merged: four tolerances
/// would do; the rest is room.
const EDGE_REACH: f64 = 16.0;

/// The most pieces a proof that an edge lies on a surface may take
/// (`lying_edges`).
const LYING_PIECES: usize = 1 << 12;

/// Adds to `cuts` where the edges of `face` of `own` meet `surface`, of
/// the edges whose boxes come within [`EDGE_REACH`] tolerances of `near`,
/// the overlap of the two faces' boxes.
///
/// A cut further away bounds only stretches that [`clip`] leaves out
/// without looking at them: a stretch is kept only when its middle and its
/// ends are within both faces' boxes, and between two cuts that are, the
/// curve does not leave both faces without crossing an edge of one of them
/// where it is still within the other, which is a cut near the overlap
/// too. So the stretches kept, their ends, and the vertices made of them
/// are those of asking every edge; what is not asked is where an edge far
/// from the other face meets its surface. Of a face with a thousand edges
/// the few near the other face are asked, not all of them for every face
/// it meets.
#[allow(clippy::too_many_arguments)]
fn edge_cuts(
    own: &Solid,
    face: usize,
    surface: Sheet,
    near: &Bbox3,
    other: Option<(&Cover, CoverFace)>,
    named: &[(Operand, FaceId)],
    tolerance: &Tolerance,
    cuts: &mut Vec<Cut>,
) -> Result<(), Stop> {
    let tol = tolerance.linear;
    let reach = EDGE_REACH * tol;
    let mut asked = crate::work::Tally::edges();
    for id in own.edges_near(face, |b| boxes_meet(b, near, reach)) {
        asked.one();
        // An edge further from the other face than its cover meets the
        // other surface only where no stretch can be kept: a stretch
        // between the cuts that are left crosses no boundary of the other
        // face, and holds a point outside it, so all of it is outside.
        if other.is_some_and(|(cover, of)| !cover.reaches(of, &own.edge_box(id), None, APART * tol))
        {
            continue;
        }
        // Nor does an edge whose box is further than that from the other
        // surface (then it does not meet it at all).
        if clear_of_surface(surface.surface, &own.edge_box(id), APART * tol) {
            continue;
        }
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
                    end: false,
                },
                Operand::B => Cut {
                    t: 0.0,
                    point,
                    a_edge: None,
                    b_edge: edge,
                    end: false,
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
        // The edge's own curve with the other body's surface: a tolerant
        // edge is taken as it is, not as the exact curve it stands for.
        match curve_hits(&edge.curve, edge.range, surface, tolerance)
            .map_err(|e| geometry(e, named))?
        {
            CurveIntersection::OnSurface => {
                // The whole curve does, but this piece of it did not pass
                // the test above.
                return Err(Stop::of(AMBIGUOUS).at(named));
            }
            CurveIntersection::Hits(hits) => {
                // Where along the edge each hit is (`corner` keeps a hit
                // between its neighbours).
                let ats: Vec<f64> = hits
                    .iter()
                    .filter_map(|hit| locate(&edge.curve, edge.range, hit.point, tol))
                    .collect();
                for hit in hits {
                    let Some(at) = locate(&edge.curve, edge.range, hit.point, tol) else {
                        continue;
                    };
                    // An edge that leaves the surface tangentially at one
                    // of its ends (where a fillet's face meets a face it
                    // is tangent to) stays within the tolerance of it for
                    // a while, and an approximated edge may cross it
                    // again on the way: the edge meets the surface at its
                    // end, not there.
                    let hugs = |end: f64| {
                        (end - at).abs() > 0.0
                            && [0.25, 0.5, 0.75].into_iter().all(|k| {
                                surface_distance(surface, edge.curve.eval(end + (at - end) * k))
                                    <= tol
                            })
                    };
                    // (Only an approximated edge does: an exact one that
                    // lies on the surface up to the hit, as a ruling of a
                    // cone up to its apex, meets it there.)
                    let approximated = matches!(edge.curve, Curve3::Nurbs(_));
                    if approximated && surface_distance(surface, a.point) <= tol && hugs(t0) {
                        cut(a.point, true);
                        continue;
                    }
                    if approximated && surface_distance(surface, b.point) <= tol && hugs(t1) {
                        cut(b.point, true);
                        continue;
                    }
                    // A hit next to an end of the edge that lies on the
                    // surface is that end: where the edge meets the
                    // surface at a shallow angle (an ellipse or a circle
                    // that runs onto a face it is nearly tangent to) the
                    // hit is found only to a fraction of the tolerance,
                    // and the end, which the other edges there share, is
                    // exact. Two such hits of edges that meet there would
                    // otherwise be two places closer together than the
                    // tolerance can tell apart.
                    let end = [a.point, b.point].into_iter().find(|end| {
                        hit.point.distance(*end) <= 2.0 * tol
                            && surface_distance(surface, *end) <= tol
                    });
                    // A spline's hit at a shallow angle is taken to where
                    // the surfaces meet (`corner`).
                    let point = match end {
                        Some(end) => end,
                        None if approximated => {
                            // Halfway to the hits on either side, so that
                            // no two hits become one place or swap.
                            let before = ats
                                .iter()
                                .copied()
                                .filter(|&t| t < at)
                                .reduce(f64::max)
                                .map_or(t0, |t| 0.5 * (t + at));
                            let after = ats
                                .iter()
                                .copied()
                                .filter(|&t| t > at)
                                .reduce(f64::min)
                                .map_or(t1, |t| 0.5 * (t + at));
                            super::corner::refine(
                                own,
                                id,
                                surface,
                                (hit.point, at, hit.tangent),
                                (before, after),
                                tol,
                            )
                            .map_err(|r| Stop::of(r).at(named))?
                            .unwrap_or(hit.point)
                        }
                        None => hit.point,
                    };
                    // A touch is a cut like any other: the stretches on
                    // either side of it are judged on their own.
                    cut(point, true);
                }
            }
        }
    }
    Ok(())
}

/// The cuts `along` a curve over `(first, last)`, in order and with its
/// ends (`end`) among them, with each end next to a place where an edge
/// meets a surface taken to be that place; `guard` is the distance within
/// which two points may be one.
///
/// A curve that begins and ends in one such place (a loop through it)
/// has that place at each end, but a place is found on the curve once,
/// at whichever end is nearer: the other end is the place too, and is
/// kept as it (a figure eight of a torus and the plane it touches,
/// through the pole of a patch, lost the lobe back into the face).
///
/// The seam of a closed spline is where its trace happened to start, no
/// place of its own; it stays a cut (no stretch may run across it), but
/// next to a place that the curve reaches from the seam without leaving
/// it (on either side of the seam) it is that place, as an end is: else
/// the place and the seam, a tolerance or two apart, are two points
/// closer together than the tolerance can tell apart (the sea kayak's
/// stitch holes at the Jig's tolerance of 0.01 mm, repros 21 to 24: a
/// hole's wall meets the plate beside it in a closed curve along the
/// hole's rim, whose trace started one or two tolerances from the rim's
/// own vertex).
fn ends_at_places(
    along: Vec<Cut>,
    curve: &Curve3,
    (first, last): (f64, f64),
    closed: bool,
    guard: f64,
) -> Vec<Cut> {
    let places: Vec<Cut> = along.iter().filter(|cut| !cut.end).copied().collect();
    let mut kept = Vec::with_capacity(along.len());
    for cut in along {
        if !cut.end {
            kept.push(cut);
            continue;
        }
        let near: Vec<&Cut> = places
            .iter()
            .filter(|p| p.point.distance(cut.point) < guard)
            .collect();
        if closed {
            // The nearest place the curve reaches from the seam without
            // leaving it, from its start or back from its end.
            let stays =
                |from: f64, to: f64| curve.eval(0.5 * (from + to)).distance(cut.point) < guard;
            let reached = near
                .iter()
                .filter(|p| stays(first, p.t) || stays(p.t, last))
                .min_by(|p, q| {
                    p.point
                        .distance(cut.point)
                        .total_cmp(&q.point.distance(cut.point))
                        .then(p.t.total_cmp(&q.t))
                });
            kept.push(match reached {
                Some(&&place) => Cut {
                    t: cut.t,
                    end: false,
                    ..place
                },
                None => cut,
            });
            continue;
        }
        let Some(&&place) = near.first() else {
            kept.push(cut);
            continue;
        };
        // Found at this end: the curve stays at the place between the
        // two.
        let here = near
            .iter()
            .any(|p| curve.eval(0.5 * (p.t + cut.t)).distance(cut.point) < guard);
        if !here {
            kept.push(Cut {
                t: cut.t,
                end: false,
                ..place
            });
        }
    }
    kept
}

/// The stretches of `meeting`, a curve in which the surfaces of face `fa`
/// of A and `fb` of B meet, that are within both faces. `found` are the
/// places where an edge of either face meets the other's surface.
///
/// Where the surfaces touch along the curve without crossing, a stretch is
/// imprinted like any other. Along the boundary of either face it is an
/// edge that was there already (a smooth edge of one body lying on a face
/// of the other). Inside both it divides each face only if the result
/// needs it to: a piece that ends inside a face, or has the same piece of
/// the face on both sides, is dropped when the regions are made, and what
/// it divides needlessly is merged again at the end.
#[allow(clippy::too_many_arguments)]
fn clip(
    meeting: &MeetingCurve,
    found: &[Cut],
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
    let (oa, ob) = (a.outline(fa)?, b.outline(fb)?);
    let (Some(box_a), Some(box_b)) = (a.boxes.get(fa), b.boxes.get(fb)) else {
        return Err(Stop::of(LOST));
    };
    let curve = &meeting.curve;
    let (first, last) = meeting.range;
    if !(first.is_finite() && last.is_finite() && last > first) {
        return Err(Stop::of(LOST).at(&named));
    }
    // A closed curve is gone around; one that closes on itself without
    // being periodic (a spline) cannot be followed across its seam, and
    // is cut there.
    let closed = meeting.closed;
    let periodic = closed && curve.period().is_some();
    let period = curve.period().filter(|_| periodic).unwrap_or(last - first);
    let plain = |t: f64| Cut {
        t,
        point: curve.eval(t),
        a_edge: None,
        b_edge: None,
        end: true,
    };

    // The cuts on this curve (the surfaces may meet in another one too),
    // in order along it, each place once.
    let mut along: Vec<Cut> = Vec::new();
    for &(mut cut) in found {
        let t = if periodic {
            match curve.project(cut.point) {
                Ok(t) if curve.eval(t).distance(cut.point) <= tol => t,
                _ => continue,
            }
        } else {
            match nearest(curve, (first, last), cut.point) {
                Some((t, d)) if d <= tol => t,
                _ => continue,
            }
        };
        cut.t = t;
        along.push(cut);
    }
    if !periodic {
        // Where the curve itself starts and ends: an apex, the joint
        // between two pieces of one curve, the seam of a closed spline,
        // or simply the edge of the region looked at.
        along.push(plain(first));
        if !closed {
            along.push(plain(last));
        }
    }
    along.sort_by(|p, q| p.t.total_cmp(&q.t));
    let along = ends_at_places(along, curve, (first, last), closed, vertices.guard);
    let mut cuts: Vec<Cut> = Vec::new();
    for cut in along {
        let same = |other: &Cut| -> Result<bool, Stop> {
            let d = other.point.distance(cut.point);
            if d > vertices.merge && d < vertices.guard {
                if one_meeting(
                    (a, b),
                    (cut.point, (cut.a_edge, cut.b_edge)),
                    (other.point, (other.a_edge, other.b_edge)),
                    tol,
                ) {
                    return Ok(true);
                }
                return Err(Stop::of(NEAR).at(&named));
            }
            Ok(d <= vertices.merge)
        };
        // Two cuts in one place are one cut when the curve stays there
        // between them. A curve that leaves and comes back (a loop
        // through the apex of a cone that lies on a sphere begins and
        // ends there) has two cuts in that place, with all of itself
        // between them.
        let is_last = match cuts.last() {
            Some(last) => {
                same(last)?
                    && curve.eval(0.5 * (last.t + cut.t)).distance(cut.point) < vertices.guard
            }
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
        match cuts.as_slice() {
            [] => {
                let seam = plain(first);
                spans.push((seam, seam, first, first + period));
            }
            [first_cut, ..] => {
                let after = cuts.iter().skip(1).chain(std::iter::once(first_cut));
                for (from, to) in cuts.iter().zip(after) {
                    let end = if to.t > from.t { to.t } else { to.t + period };
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
        let middle = at(0.5);
        // Outside a face's box is outside the face. Between its ends a
        // stretch crosses the boundary of neither face (where it would,
        // an edge meets the other surface, and that is a cut), so it is
        // out of a face wherever any point of it is: its middle, or an
        // end beyond a face's box (where the box the surfaces were asked
        // in cut the curve, that end is clear of one of the two boxes).
        let clear = |p: Vec3| !(box_a.contains(p, 2.0 * tol) && box_b.contains(p, 2.0 * tol));
        if clear(middle) || clear(from.point) || clear(to.point) {
            continue;
        }
        // Out of either face at the middle is no part of the imprint,
        // whatever the stretch does along the boundary of the other: a
        // curve that leaves a face's edge tangentially (where a fillet's
        // face meets the other side of a sharp edge) stays within the
        // tolerance of that edge for a while beyond the other face too.
        // (Further from either face than its cover is outside it.)
        if !(a.may_hold(fa, middle, tol) && b.may_hold(fb, middle, tol)) {
            continue;
        }
        let mut heres = [Side::On; 2];
        for (outline, here) in [oa, ob].into_iter().zip(heres.iter_mut()) {
            *here = outline
                .side(middle, tol)
                .map_err(|r| Stop::of(r).at(&named))?;
            if *here == Side::Out {
                break;
            }
        }
        if heres.contains(&Side::Out) {
            continue;
        }
        let mut inside = [false, false];
        let mut keep = true;
        for ((outline, inside), here) in [oa, ob].into_iter().zip(inside.iter_mut()).zip(heres) {
            let side = |p: Vec3| outline.side(p, tol).map_err(|r| Stop::of(r).at(&named));
            if here == Side::Out {
                keep = false;
                break;
            }
            let quarters = [side(at(0.25))?, side(at(0.75))?];
            let mixed = quarters.iter().any(|&q| q != here);
            if quarters.contains(&Side::Out) || (mixed && meeting.tangent) {
                // A curve along which the surfaces only touch divides
                // nothing: where it runs over a face's boundary without
                // a cut there (it is within the tolerance of the surface
                // well past where it leaves it), it is left out. A
                // crossing that leaves the face in that state is refused.
                if meeting.tangent {
                    keep = false;
                    break;
                }
                return Err(Stop::of(AMBIGUOUS).at(&named));
            }
            // Between its cuts a stretch crosses no edge of the face, so
            // it is either along the boundary (within the tolerance of
            // it everywhere) or inside the face. A place within the
            // tolerance of the boundary says nothing by itself; one
            // clear of it says inside: a chord that cuts a corner of the
            // face a few tolerances from its vertex is inside, and
            // passes within the tolerance of both edges on the way. But a
            // cut can be lost in that band (an edge fitted within the
            // tolerance of its curve meets the surface at a shallow
            // angle more than a tolerance from the meeting curve; a hit
            // is taken to the edge's end), so a stretch that is inside
            // somewhere must be inside by the sign of its offset from
            // the nearest edge wherever it is in the band, at seven
            // places; one that leaves the face there is refused.
            if here == Side::In || quarters.contains(&Side::In) {
                if mixed {
                    for k in 1..8 {
                        let p = at(f64::from(k) / 8.0);
                        match side(p)? {
                            Side::In => {}
                            Side::On if outline.inside_by_sign(p, tol) == Some(true) => {}
                            _ => return Err(Stop::of(AMBIGUOUS).at(&named)),
                        }
                    }
                }
                *inside = true;
            }
        }
        if !keep {
            continue;
        }
        // Faces that go through each other by less than half the
        // tolerance touch, and nothing is imprinted; by twice the
        // tolerance or more they cross, exactly as given; in between they
        // do neither. (The counterpart, for surfaces of different kinds,
        // of the rule for faces on one surface.)
        let mut crossing = !meeting.tangent;
        if crossing {
            let reach = box_a.diagonal().max(box_b.diagonal());
            let mut seen = [Passage::Touch; 3];
            for (k, seen) in [0.25, 0.5, 0.75].into_iter().zip(seen.iter_mut()) {
                let (p, tangent) = curve.derivatives(t0 + (t1 - t0) * k);
                *seen = passage(oa.sheet(), ob.sheet(), p, tangent, reach, tol);
            }
            if seen.contains(&Passage::Crossing) {
                // As given.
            } else if seen.iter().all(|&p| p == Passage::Touch) {
                // A touch along the curve that geometry did not call one:
                // imprinted like any other touch.
                crossing = false;
            } else {
                let thickest = seen.iter().fold(0.0_f64, |most, p| match p {
                    Passage::Lens(depth) => most.max(*depth),
                    _ => most,
                });
                if thickest <= 0.5 * tol {
                    continue;
                }
                return Err(Stop::of(NEARLY_TOUCHING).at(&named));
            }
        } else {
            // A curve geometry calls a touch may be two crossings closer
            // together than it tells apart (two cylinders on parallel
            // axes, one inside the other, whose walls cross a
            // hundred-thousandth of a radian apart): the same rule.
            let reach = box_a.diagonal().max(box_b.diagonal());
            let (p, tangent) = curve.derivatives(t0 + (t1 - t0) * 0.5);
            if let Passage::Lens(depth) = passage(oa.sheet(), ob.sheet(), p, tangent, reach, tol)
                && depth > 0.5 * tol
            {
                return Err(Stop::of(NEARLY_TOUCHING).at(&named));
            }
        }
        // Where an edge of one body meets an edge of the other, the place
        // is found twice: as the first edge's crossing of the other face's
        // surface, and as the second's crossing of the first face's. Each
        // edge is only within the tolerance of the curve it stands for,
        // so where they run onto a surface at a shallow angle the two
        // places may be further apart than points that are merged. Each
        // is within the tolerance of both edges: it is one vertex.
        let edge_meets_edge = |cut: Cut| {
            move |q: Vec3, known: &Meeting| {
                one_meeting(
                    (a, b),
                    (cut.point, (cut.a_edge, cut.b_edge)),
                    (q, (known.a_edge, known.b_edge)),
                    tol,
                )
            }
        };
        let start = vertices
            .find_or_add_with(from.point, edge_meets_edge(from))
            .map_err(|s| s.at(&named))?;
        let end = vertices
            .find_or_add_with(to.point, edge_meets_edge(to))
            .map_err(|s| s.at(&named))?;
        // A stretch from a vertex back to itself is a loop when it goes
        // somewhere on the way: the whole of a closed curve, or a curve
        // that begins and ends in one place.
        let whole = closed && cuts.len() <= 1;
        let looped = middle.distance(from.point) >= vertices.guard;
        if start == end && !(whole || looped) {
            return Err(Stop::of(NEAR).at(&named));
        }
        for (vertex, cut) in [(start, &from), (end, &to)] {
            let meeting: &mut Meeting = vertices.meetings.get_mut(vertex).ok_or(Stop::of(LOST))?;
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
                // (Only the edges near the vertex are looked at: a face may
                // have a thousand.)
                let outline = solid.outline(face)?;
                for (l, k) in outline.pieces_near(point, tol) {
                    let id = solid
                        .body
                        .faces
                        .get(face)
                        .and_then(|f| f.loops.iter().filter_map(|&id| solid.body.loop_(id)).nth(l))
                        .and_then(|loop_| loop_.coedges.get(k))
                        .map(|coedge| coedge.edge)
                        .ok_or(Stop::of(LOST))?;
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
            crossing,
        });
    }
    Ok(())
}

/// How far apart two faces on surfaces of one kind are where they are over
/// each other: the greatest distance from a place on the boundary of
/// either (or, without a boundary, on it) that is in or on the other face
/// to the other's surface. `None` when no such place was found.
fn gap(a: (&Outline, &Bbox3), b: (&Outline, &Bbox3), tol: f64) -> Result<Option<f64>, Stop> {
    let mut widest: Option<f64> = None;
    for ((own, _), (other, other_box)) in [(a, b), (b, a)] {
        let mut places: Vec<Vec3> = Vec::new();
        for seg in own.loops.iter().flatten() {
            places.extend([seg.a, seg.at(0.25), seg.at(0.5), seg.at(0.75), seg.b]);
        }
        if own.loops.is_empty() {
            places = own.interior_points(tol, 4.0 * tol, 4).map_err(Stop::of)?;
        }
        for p in places {
            // Outside the other face's box it is not over that face.
            if !other_box.contains(p, 2.0 * tol) {
                continue;
            }
            let Some(distance) = signed_distance(other.sheet(), p).map(f64::abs) else {
                continue;
            };
            // Further than this they are plainly apart here.
            if distance >= 2.0 * tol {
                continue;
            }
            let foot = onto(other.sheet(), p).ok_or(Stop::of(LOST))?;
            if other.side(foot, tol).map_err(Stop::of)? != Side::Out {
                widest = Some(widest.map_or(distance, |w: f64| w.max(distance)));
            }
        }
    }
    Ok(widest)
}

/// True when everything in the hull is on one side of the surface, more
/// than `margin` away from it: a face in the hull cannot meet a face on
/// the surface.
fn clear_of(sheet: Sheet, hull: &Hull, margin: f64) -> bool {
    if let Surface::Plane(plane) = sheet.surface {
        // Flat: how far the hull reaches along the normal says it exactly.
        let normal = plane.frame().z();
        let (low, high) = hull.span(normal);
        let level = plane.frame().origin().dot(normal);
        return low > high || low - level > margin || level - high > margin;
    }
    let Some((lo, hi)) = hull.bbox().corners() else {
        return true;
    };
    // The distance changes by no more than the distance moved. (A freeform
    // surface answers with the bound below its distance, which is what a
    // proof of clearance needs.)
    let middle = lo.lerp(hi, 0.5);
    if let Some(tree) = sheet.tree {
        let needed = 0.5 * lo.distance(hi) + margin;
        return tree.lower_bound(middle, 0.5 * margin, 1.001 * needed) > needed;
    }
    signed_distance(sheet, middle)
        .is_some_and(|middle| middle.abs() > 0.5 * lo.distance(hi) + margin)
}

/// The part of space both boxes cover, grown by `margin`.
fn overlap(a: &Bbox3, b: &Bbox3, margin: f64) -> Option<Bbox3> {
    let ((alo, ahi), (blo, bhi)) = (a.corners()?, b.corners()?);
    let (lo, hi) = (alo.max(blo), ahi.min(bhi));
    let pad = Vec3::new(margin, margin, margin);
    let (lo, hi) = (lo - pad, hi + pad);
    (lo.x <= hi.x && lo.y <= hi.y && lo.z <= hi.z).then(|| Bbox3::from_points([lo, hi]))
}

/// Meets two faces' surfaces (`meet`) in `within`, the box both faces
/// share grown by `room`, the room of the larger face. The room is taken
/// from the larger face, so next to a large face (a terrain) the box
/// reaches far beyond the smaller one, where the surfaces may meet in
/// curves that cannot be traced (a plane at the height of a lake the
/// fitted surface ripples about, 20 m from the face) and that no face
/// holds. When the meeting fails so (`NoConvergence`, `Unsupported`), it is
/// asked again in the box both faces share grown by the smaller face's
/// room, which still holds everything of both faces; what that cannot
/// trace either is refused with the first reason, and any other failure
/// as it is (*A plane beside a rippled surface*, docs/design/booleans.md).
fn in_room<T>(
    box_a: &Bbox3,
    box_b: &Bbox3,
    within: &Bbox3,
    room: f64,
    tol: f64,
    meet: impl Fn(&Bbox3) -> Result<T, GeomError>,
) -> Result<T, GeomError> {
    match meet(within) {
        Ok(met) => Ok(met),
        Err(first @ (GeomError::NoConvergence(_) | GeomError::Unsupported(_))) => {
            let near = 16.0 * tol + 0.013 * box_a.diagonal().min(box_b.diagonal());
            match overlap(box_a, box_b, near) {
                Some(closer) if near < room => meet(&closer).map_err(|_| first),
                _ => Err(first),
            }
        }
        Err(other) => Err(other),
    }
}

/// `in_room`, and where the surfaces cannot be met in the boxes' room,
/// the same in the room of the faces' nearer boxes (`Hull::near_bbox`)
/// when that is smaller: the boxes hold the whole circles and control
/// polygons of the curves the faces' edges are pieces of, and a stretch
/// of a circle far larger than the face (a nearly straight arc) makes
/// the box of a face millimetres across hundreds of metres wide, at
/// which size the tolerance is finer than rounding allows. Where the
/// nearer boxes do not come within twice the tolerance of each other the
/// faces do not meet. (Asked only where the first refuses, so that what
/// was met before is met the same.)
fn meet_in(
    (box_a, box_b): (&Bbox3, &Bbox3),
    near: (Option<&Bbox3>, Option<&Bbox3>),
    (within, room): (&Bbox3, f64),
    tol: f64,
    meet: impl Fn(&Bbox3) -> Result<SurfaceMeeting, GeomError>,
) -> Result<SurfaceMeeting, GeomError> {
    let first = match in_room(box_a, box_b, within, room, tol, &meet) {
        Ok(met) => return Ok(met),
        Err(error) => error,
    };
    let (Some(near_a), Some(near_b)) = near else {
        return Err(first);
    };
    if !boxes_meet(near_a, near_b, 2.0 * tol) {
        return Ok(SurfaceMeeting::None);
    }
    let near_room = 16.0 * tol + 0.013 * near_a.diagonal().max(near_b.diagonal());
    match overlap(near_a, near_b, near_room) {
        Some(closer) if closer.diagonal() < within.diagonal() => {
            in_room(near_a, near_b, &closer, near_room, tol, &meet).map_err(|_| first)
        }
        _ => Err(first),
    }
}

/// How far apart, in tolerances, two faces' covers must be for the pair
/// to be passed over: beyond every distance a rule of the section reads
/// (two tolerances for faces over each other or touching, the outline's
/// margin of four for a point on a face's boundary), with room to spare.
const APART: f64 = 16.0;

pub(super) fn section(
    a: &Solid,
    b: &Solid,
    vertices: &mut Vertices,
    tolerance: &Tolerance,
) -> Result<Sections, Stop> {
    let tol = tolerance.linear;
    let mut sections = Sections::default();
    // Only where both bodies are can their faces meet.
    if overlap(&a.bbox(), &b.bbox(), 2.0 * tol).is_none() {
        return Ok(sections);
    }
    let mut tested = crate::work::Tally::faces();
    // Pairs of faces whose surfaces only touch (points, no curve), in the
    // order they were met.
    let mut touching: Vec<(usize, usize)> = Vec::new();
    // Pairs of faces on one surface: a boundary edge of either face that
    // lies on the other is imprinted on it from the edge itself, once
    // every pair has been met (`lying_edges`).
    let mut coincident: Vec<(usize, usize)> = Vec::new();
    for (fa, box_a) in a.boxes.iter().enumerate() {
        // The faces of B whose boxes meet this one's, in the order of B's
        // faces: the pairs, and their order, of testing every face of B.
        for fb in b.candidates(|box_b| boxes_meet(box_a, box_b, 2.0 * tol)) {
            tested.one();
            let Some(box_b) = b.boxes.get(fb) else {
                return Err(Stop::of(LOST));
            };
            // Only where both faces are can they meet: their surfaces are
            // intersected there and nowhere else, so that what the surfaces
            // do far from the faces (cross, touch, run tangent) does not
            // depend on what else either body holds. (With room to spare,
            // so that where a curve is cut off is not where anything of
            // either face is.)
            if !boxes_meet(box_a, box_b, 2.0 * tol) {
                continue;
            }
            let room = 16.0 * tol + 0.013 * box_a.diagonal().max(box_b.diagonal());
            let Some(within) = overlap(box_a, box_b, room) else {
                continue;
            };
            let (Some(hull_a), Some(hull_b)) = (a.hulls.get(fa), b.hulls.get(fb)) else {
                return Err(Stop::of(LOST));
            };
            let (oa, ob) = (a.outline(fa)?, b.outline(fb)?);
            if clear_of(oa.sheet(), hull_b, 2.0 * tol) || clear_of(ob.sheet(), hull_a, 2.0 * tol) {
                continue;
            }
            // Faces whose covers are further apart than this cannot meet
            // (`Cover`), nor come near enough for any rule of the
            // tolerance to read them together: nothing of the pair is
            // asked for.
            let apart = APART * tol;
            let meet = match (a.cover(fa, tol), b.cover(fb, tol)) {
                (Some((ca, of_a)), Some((cb, of_b))) => ca.meets(of_a, cb, of_b, apart),
                (Some((ca, of_a)), None) => ca.reaches(of_a, box_b, Some(&ob.surface), apart),
                (None, Some((cb, of_b))) => cb.reaches(of_b, box_a, Some(&oa.surface), apart),
                (None, None) => true,
            };
            if !meet {
                continue;
            }
            let named = [a.name(fa), b.name(fb)];
            crate::work::face_pairs(1);
            #[cfg(test)]
            tests::PAIRS_MET.with(|n| n.set(n.get() + 1));
            let near = (a.near.get(fa), b.near.get(fb));
            let met = match meet_in((box_a, box_b), near, (&within, room), tol, |room| {
                meet_surfaces(&oa.surface, &ob.surface, room, tolerance)
            }) {
                Ok(met) => met,
                // Surfaces that cannot be traced may be faces that lie on
                // one another where one is over the other, though the
                // surfaces part beyond them (`coincide`).
                Err(error @ (GeomError::NoConvergence(_) | GeomError::Unsupported(_))) => {
                    match coincide(a, fa, b, fb, tol).map_err(|s| s.at(&named))? {
                        Coincidence::Faces(same_normal) => {
                            SurfaceMeeting::Coincident { same_normal }
                        }
                        Coincidence::Partly => return Err(Stop::of(PARTLY).at(&named)),
                        Coincidence::Unknown => return Err(geometry(error, &named)),
                    }
                }
                Err(error) => return Err(geometry(error, &named)),
            };
            // Faces on one surface, or on two that never cross: the same
            // surface when they are within half the tolerance of each
            // other where one is over the other, apart when twice the
            // tolerance or more lies between them, and in between neither
            // (the result would be glued together or left apart by a
            // margin nobody asked for).
            let alike = std::mem::discriminant(&oa.surface) == std::mem::discriminant(&ob.surface);
            let unsure = match &met {
                SurfaceMeeting::Coincident { .. } => gap((oa, box_a), (ob, box_b), tol)
                    .map_err(|s| s.at(&named))?
                    .is_some_and(|g| g > 0.5 * tol),
                // (Within half the tolerance at such a place they touch
                // there, as a vertex of one body on a face of the other
                // does: a contact, which the classification handles.)
                SurfaceMeeting::None if alike => gap((oa, box_a), (ob, box_b), tol)
                    .map_err(|s| s.at(&named))?
                    .is_some_and(|g| g > 0.5 * tol),
                _ => false,
            };
            if unsure {
                return Err(Stop::of(NEARLY_COINCIDENT).at(&named));
            }
            match met {
                SurfaceMeeting::None => {}
                SurfaceMeeting::Coincident { same_normal } => {
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
                    coincident.push((fa, fb));
                }
                // Points where the surfaces only touch divide nothing.
                SurfaceMeeting::Meets(meeting) => {
                    // A point where geometry says the surfaces touch is
                    // one where they are within the tolerance of each
                    // other. Where one face is through the other there
                    // by more than half the tolerance (its nearest point
                    // lies inside the other body), the faces neither
                    // touch nor safely cross.
                    for &p in &meeting.points {
                        sections.touches.push(p);
                        let (Some(qa), Some(qb)) = (onto(oa.sheet(), p), onto(ob.sheet(), p))
                        else {
                            continue;
                        };
                        let through = outward_normal(oa.sheet(), oa.reversed, qa)
                            .is_some_and(|outward| (qb - qa).dot(outward) < 0.0);
                        if qa.distance(qb) > 0.5 * tol
                            && through
                            && oa.side(qa, tol).map_err(|r| Stop::of(r).at(&named))? != Side::Out
                            && ob.side(qb, tol).map_err(|r| Stop::of(r).at(&named))? != Side::Out
                        {
                            return Err(Stop::of(NEARLY_TOUCHING).at(&named));
                        }
                    }
                    if meeting.curves.is_empty() {
                        // Touch points and no curve: the surfaces may touch
                        // all along an edge of either face, which is then
                        // imprinted on the other face from the edge itself
                        // (once every pair has been met: `lying_edges`).
                        touching.push((fa, fb));
                        continue;
                    }
                    let mut found = Vec::new();
                    // (Some: the boxes meet within twice the tolerance.)
                    let near = overlap(box_a, box_b, 2.0 * tol).unwrap_or(within);
                    let (ca, cb) = (a.cover(fa, tol), b.cover(fb, tol));
                    edge_cuts(a, fa, ob.sheet(), &near, cb, &named, tolerance, &mut found)?;
                    edge_cuts(b, fb, oa.sheet(), &near, ca, &named, tolerance, &mut found)?;
                    sections
                        .edge_meets
                        .extend(found.iter().map(|cut| cut.point));
                    for curve in &meeting.curves {
                        clip(
                            curve,
                            &found,
                            a,
                            fa,
                            b,
                            fb,
                            vertices,
                            tolerance,
                            &mut sections,
                        )?;
                    }
                }
            }
        }
    }
    for (fa, fb) in touching {
        lying_edges(
            a,
            fa,
            b,
            fb,
            Lying::Near,
            vertices,
            tolerance,
            &mut sections,
        )?;
    }
    // The boundary edges of both bodies, once per boolean: two solids
    // have none, and then there is nothing to imprint.
    if !coincident.is_empty() {
        let (boundary_a, boundary_b) = (boundary_edges(a.body), boundary_edges(b.body));
        if !(boundary_a.is_empty() && boundary_b.is_empty()) {
            for (fa, fb) in coincident {
                lying_edges(
                    a,
                    fa,
                    b,
                    fb,
                    Lying::Boundary(&boundary_a, &boundary_b),
                    vertices,
                    tolerance,
                    &mut sections,
                )?;
            }
        }
    }
    Ok(sections)
}

/// Which edges of a pair of faces [`lying_edges`] looks at.
#[derive(Clone, Copy)]
enum Lying<'e> {
    /// Every edge of either face near the other face.
    Near,
    /// The edges of either face that only one face of its body uses (the
    /// boundary of a sheet: these of A, these of B), which no second face
    /// of their own imprints.
    Boundary(&'e BTreeSet<EdgeId>, &'e BTreeSet<EdgeId>),
}

/// The edges of `body` that one face uses, once.
fn boundary_edges(body: &Body) -> BTreeSet<EdgeId> {
    let mut uses: BTreeMap<EdgeId, usize> = BTreeMap::new();
    for face in &body.faces {
        for loop_ in face.loops.iter().filter_map(|&l| body.loop_(l)) {
            for coedge in &loop_.coedges {
                *uses.entry(coedge.edge).or_insert(0) += 1;
            }
        }
    }
    uses.into_iter()
        .filter(|&(_, n)| n == 1)
        .map(|(edge, _)| edge)
        .collect()
}

/// The most pieces [`edge_crossings`] halves an edge into, per edge of
/// the other face.
const CROSSING_PIECES: usize = 1 << 16;
/// How long, in tolerances, a piece of an edge must be (its chord) to be
/// taken to lie along another edge when it is within the tolerance of it
/// at nine places ([`edge_crossings`]): two curves that cross at an angle
/// θ are within the tolerance of each other over 2 / sin θ tolerances,
/// so only crossings flatter than about two degrees, within the
/// tolerance of each other over that length, read as a shared stretch.
const ALONG: f64 = 64.0;
/// Where an edge crosses another on one surface, when the pieces it takes
/// to tell are more than [`CROSSING_PIECES`].
const CROSSING_UNRESOLVED: Reason = "where an edge of one body crosses an edge of the other, on a surface both lie on, could not be told within the work allowed (the edges run close to each other, or bend more than can be bounded, over a long stretch)";

/// The distance between the segments from `a.0` to `a.1` and from `b.0`
/// to `b.1` (either may be a point): the closest points by the clamped
/// solution of the two lines' normal equations, each clamp tried from
/// both segments' side, and the least of that and the four end-to-segment
/// distances (which covers parallel and degenerate segments). NaN in, NaN
/// out.
fn segments_apart(a: (Vec3, Vec3), b: (Vec3, Vec3)) -> f64 {
    let to_segment = |p: Vec3, (s0, s1): (Vec3, Vec3)| {
        let d = s1 - s0;
        let len2 = d.dot(d);
        let t = if len2 > 0.0 {
            ((p - s0).dot(d) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        p.distance(s0 + d * t)
    };
    let mut best = to_segment(a.0, b)
        .min(to_segment(a.1, b))
        .min(to_segment(b.0, a))
        .min(to_segment(b.1, a));
    let (u, v, w) = (a.1 - a.0, b.1 - b.0, a.0 - b.0);
    let (uu, uv, vv, uw, vw) = (u.dot(u), u.dot(v), v.dot(v), u.dot(w), v.dot(w));
    let det = uu * vv - uv * uv;
    if det > 0.0 {
        let s = ((uv * vw - vv * uw) / det).clamp(0.0, 1.0);
        let t = ((uu * vw - uv * uw) / det).clamp(0.0, 1.0);
        best = best.min((a.0 + u * s).distance(b.0 + v * t));
    }
    if [a.0, a.1, b.0, b.1].iter().any(|p| !p.is_finite()) {
        return f64::NAN;
    }
    best
}

/// Adds to `cuts` where edge `id` of `own`, which lies on the surface of
/// face `face` of `other`, crosses an edge of that face: where it enters
/// or leaves the face. Both curves lie on one surface, so neither meets
/// the other's surface there and [`edge_cuts`] cannot see the place; it
/// is found between the two curves, for curves and surfaces of any kind.
///
/// Branch and bound over pairs of a piece of the edge and a piece of the
/// other edge, with two lower bounds on how near they come. The chords:
/// each piece lies within `turn · h² / 2` of its chord (`turn` bounds the
/// curve's second derivative, `hits::turn_bound`; h is half the piece's
/// parameter range), so the distance between the two chords less both
/// strays bounds the distance between the pieces from below, and pieces
/// side by side at any distance beyond the tolerance are dropped as soon
/// as their chords hug them (at once for two lines). And the middle:
/// the distance from the middle of the edge's piece to the other piece
/// (`nearest`) changes along the piece by no more than the edge's speed
/// bound times the change of parameter, so a middle further than the
/// tolerance plus that bound times the half width drops the pair. The
/// longer piece of a pair is halved, down to pieces of the edge a
/// quarter of the tolerance long. (For lines, circles and ellipses the
/// bounds are proofs; for a rational spline the speed and bend bounds
/// come from its Bézier nets and are estimates, so a crossing of a
/// heavily weighted spline edge could in principle be missed.) A piece
/// within the tolerance of the other edge at nine
/// places along it lies along that edge (the checker's measure of an edge
/// lying on something, sampled): the two share a stretch there, which
/// their vertices bound, and it is no crossing. Pieces left that follow
/// one another are one place where the edges meet: a vertex of either
/// edge within twice the tolerance of both edges there (next to a shared
/// stretch the run reaches from the vertex where it ends into it, all of
/// it as near the other edge), as a hit near an end is that end in
/// [`edge_cuts`]; else the point of the run nearest the other edge.
fn edge_crossings(
    own: &Solid,
    id: EdgeId,
    other: &Solid,
    face: usize,
    named: &[(Operand, FaceId)],
    tol: f64,
    cuts: &mut Vec<Cut>,
) -> Result<(), Stop> {
    let edge = own.body.edge(id).ok_or(Stop::of(LOST))?;
    let (Some(start), Some(end)) = (own.body.vertex(edge.start), own.body.vertex(edge.end)) else {
        return Err(Stop::of(LOST));
    };
    let (t0, t1) = edge.range;
    let speed = super::hits::speed_bound(&edge.curve);
    if !(speed.is_finite() && speed > 0.0 && t1 > t0) {
        return Err(Stop::of(LOST).at(named));
    }
    let edge_box = own.edge_box(id);
    for crossed in other.edges_near(face, |bx| boxes_meet(bx, &edge_box, 2.0 * tol)) {
        let piece = other.body.edge(crossed).ok_or(Stop::of(LOST))?;
        let (Some(p0), Some(p1)) = (other.body.vertex(piece.start), other.body.vertex(piece.end))
        else {
            return Err(Stop::of(LOST));
        };
        let distance = |t: f64| -> Result<f64, Stop> {
            nearest(&piece.curve, piece.range, edge.curve.eval(t))
                .map(|(_, d)| d)
                .ok_or(Stop::of(LOST).at(named))
        };
        // How far a piece of either curve strays from its chord: the
        // point at a parameter is within turn · h² / 2 of the point the
        // same share of the way along the chord (h the half width of the
        // piece's parameter range). `None` without a bound: then only
        // the edge is halved, and its pieces are told by the distance
        // from their middle alone.
        let bends = super::hits::turn_bound(&edge.curve)
            .zip(super::hits::turn_bound(&piece.curve))
            .filter(|(a, b)| a.is_finite() && b.is_finite());
        let other_speed = super::hits::speed_bound(&piece.curve);
        let (e0, e1) = piece.range;
        // Pairs of a piece of the edge and a piece of the other edge.
        let mut open: Vec<(f64, f64, f64, f64)> = vec![(t0, t1, e0, e1)];
        let mut kept: Vec<(f64, f64, f64, f64)> = Vec::new();
        let mut pieces = 0_usize;
        while let Some((lo, hi, elo, ehi)) = open.pop() {
            pieces += 1;
            if pieces > CROSSING_PIECES {
                return Err(Stop::of(CROSSING_UNRESOLVED).at(named));
            }
            let (middle, half) = (0.5 * (lo + hi), 0.5 * (hi - lo));
            let (other_middle, other_half) = (0.5 * (elo + ehi), 0.5 * (ehi - elo));
            let reach = speed * half;
            // Apart by more than the tolerance where the chords of the
            // two pieces, less how far each piece strays from its chord,
            // are: two pieces running side by side at any distance
            // beyond the tolerance are told apart at once.
            if let Some((bend, other_bend)) = bends {
                let stray = 0.5 * (bend * half * half + other_bend * other_half * other_half);
                let gap = segments_apart(
                    (edge.curve.eval(lo), edge.curve.eval(hi)),
                    (piece.curve.eval(elo), piece.curve.eval(ehi)),
                );
                if gap - stray > tol {
                    continue;
                }
            }
            let d = nearest(&piece.curve, (elo, ehi), edge.curve.eval(middle))
                .map(|(_, d)| d)
                .ok_or(Stop::of(LOST).at(named))?;
            // (A NaN is dropped too.)
            if d.is_nan() || d > tol + reach {
                continue;
            }
            if reach <= 0.25 * tol || !(hi > middle && middle > lo) {
                kept.push((lo, hi, middle, distance(middle)?));
                continue;
            }
            // (Only a piece much longer than the band in which a crossing
            // is within the tolerance of the other edge: a shorter one
            // around a crossing is within it at all nine places.)
            let mut along = edge.curve.eval(lo).distance(edge.curve.eval(hi)) >= ALONG * tol;
            for k in 0..=8 {
                if !along {
                    break;
                }
                let here = distance(lo + (hi - lo) * f64::from(k) / 8.0)?;
                if here.is_nan() || here > tol {
                    along = false;
                    break;
                }
            }
            if along {
                continue;
            }
            // The longer piece is halved (the other edge's only where
            // both bend by a known amount); depth first, the lower half
            // first.
            if bends.is_some()
                && other_speed * other_half > reach
                && ehi > other_middle
                && other_middle > elo
            {
                open.push((lo, hi, other_middle, ehi));
                open.push((lo, hi, elo, other_middle));
            } else {
                open.push((middle, hi, elo, ehi));
                open.push((lo, middle, elo, ehi));
            }
        }
        // A piece of the edge may be kept with more than one piece of the
        // other edge: once, in order along the edge (the pieces are
        // halves of halves of the edge's range, so the same piece has the
        // same ends).
        kept.sort_by(|p, q| p.0.total_cmp(&q.0));
        kept.dedup_by(|p, q| p.0 == q.0);
        // Runs of pieces that follow one another, each one place: the
        // range of the run, and the parameter and distance of its point
        // nearest the other edge.
        let mut runs: Vec<(f64, f64, f64, f64)> = Vec::new();
        for (lo, hi, t, d) in kept {
            match runs.last_mut() {
                Some(run) if run.1 == lo => {
                    run.1 = hi;
                    if d < run.3 {
                        (run.2, run.3) = (t, d);
                    }
                }
                _ => runs.push((lo, hi, t, d)),
            }
        }
        for (lo, hi, t, d) in runs {
            if d > tol {
                continue;
            }
            // A vertex of either edge in the run, on both edges, is the
            // place: next to a stretch the two share, the run reaches
            // from the vertex where the stretch ends into it, every point
            // of it as near the other edge as the place.
            let at = edge.curve.eval(t);
            let on_both = |v: Vec3| {
                locate(&edge.curve, (lo, hi), v, 2.0 * tol).is_some()
                    && locate(&piece.curve, piece.range, v, 2.0 * tol).is_some()
            };
            let point = [start.point, end.point, p0.point, p1.point]
                .into_iter()
                .filter(|&v| on_both(v))
                .min_by(|v, w| v.distance(at).total_cmp(&w.distance(at)))
                .unwrap_or(at);
            let (a_edge, b_edge) = match own.operand {
                Operand::A => (Some(id), Some(crossed)),
                Operand::B => (Some(crossed), Some(id)),
            };
            cuts.push(Cut {
                t: 0.0,
                point,
                a_edge,
                b_edge,
                end: false,
            });
        }
    }
    Ok(())
}

/// Where the surfaces of face `fa` of A and `fb` of B only touch (geometry
/// gave points and no curve), the edges of either face that lie on the
/// other face's surface, imprinted on the other face from their own
/// curves; and where the faces lie on one surface (`Lying::Boundary`),
/// the boundary edges of either face (used by one face of their body: a
/// sheet's rim), which have no second face to imprint them, as a solid's
/// edges have, so that the face of the other body is divided where the
/// sheet on it ends, from where the rim enters the face to where it
/// leaves it ([`edge_crossings`]).
///
/// Two surfaces tangent along a curve with no closed form (the face of a
/// fillet, exact or approximated, against the faces it rounds; a rational
/// cylinder lying on a plane) meet in a curve along which the distance
/// between them has a valley of minima: the numeric intersection settles
/// it as touches, a disc of samples on one side around each. Where that
/// curve is the boundary of either face (the border between a fillet and
/// the strip of a tool that lies on the face it rounds), the face of the
/// other body has to be divided along it, as along any edge of one body
/// lying on a face of the other, and nothing else gives that curve: the
/// faces on either side of the edge either touch the other face too or
/// lie on its surface. So here every edge of either face near the other
/// face that comes within the tolerance of its surface is cut where it
/// meets that surface's face boundary (`edge_cuts`, as for any pair), and
/// each piece that lies on the surface at five places is a stretch of
/// touch along that edge, judged against both faces by [`clip`] like any
/// other curve in which they meet. A piece already imprinted on the same
/// face by another pair of faces (its middle on such a stretch) is left
/// out. An edge of neither face divides nothing where surfaces only
/// touch: on both sides of such a touch the face is on the same side of
/// the other body.
#[allow(clippy::too_many_arguments)]
fn lying_edges(
    a: &Solid,
    fa: usize,
    b: &Solid,
    fb: usize,
    which: Lying,
    vertices: &mut Vertices,
    tolerance: &Tolerance,
    sections: &mut Sections,
) -> Result<(), Stop> {
    let tol = tolerance.linear;
    let named = [a.name(fa), b.name(fb)];
    let (oa, ob) = (a.outline(fa)?, b.outline(fb)?);
    let (Some(box_a), Some(box_b)) = (a.boxes.get(fa), b.boxes.get(fb)) else {
        return Err(Stop::of(LOST));
    };
    let Some(near) = overlap(box_a, box_b, 2.0 * tol) else {
        return Ok(());
    };
    let reach = EDGE_REACH * tol;
    let (boundary_a, boundary_b) = match which {
        Lying::Near => (None, None),
        Lying::Boundary(of_a, of_b) => (Some(of_a), Some(of_b)),
    };
    // The edges of each face that come within the tolerance of the other
    // face's surface somewhere (nine places along them).
    let close = |own: &Solid,
                 face: usize,
                 other: Sheet,
                 boundary: Option<&BTreeSet<EdgeId>>|
     -> Result<Vec<EdgeId>, Stop> {
        let mut found = Vec::new();
        for id in own.edges_near(face, |bx| boxes_meet(bx, &near, reach)) {
            if boundary.is_some_and(|b| !b.contains(&id)) {
                continue;
            }
            let edge = own.body.edge(id).ok_or(Stop::of(LOST))?;
            let (t0, t1) = edge.range;
            if (0..=8).any(|k| {
                let t = t0 + (t1 - t0) * f64::from(k) / 8.0;
                surface_distance(other, edge.curve.eval(t)) <= tol
            }) {
                found.push(id);
            }
        }
        Ok(found)
    };
    let (of_a, of_b) = (
        close(a, fa, ob.sheet(), boundary_a)?,
        close(b, fb, oa.sheet(), boundary_b)?,
    );
    if of_a.is_empty() && of_b.is_empty() {
        return Ok(());
    }
    let mut found = Vec::new();
    edge_cuts(
        a,
        fa,
        ob.sheet(),
        &near,
        None,
        &named,
        tolerance,
        &mut found,
    )?;
    edge_cuts(
        b,
        fb,
        oa.sheet(),
        &near,
        None,
        &named,
        tolerance,
        &mut found,
    )?;
    // A boundary edge lies on the other face's surface, and so do that
    // face's own edges: no edge meets the other's surface where the two
    // cross, so where the boundary edge enters or leaves the other face
    // is found between the curves themselves.
    if matches!(which, Lying::Boundary(..)) {
        for &id in &of_a {
            edge_crossings(a, id, b, fb, &named, tol, &mut found)?;
        }
        for &id in &of_b {
            edge_crossings(b, id, a, fa, &named, tol, &mut found)?;
        }
    }
    for (own, edges, other, operand) in [
        (a, &of_a, ob.sheet(), Operand::A),
        (b, &of_b, oa.sheet(), Operand::B),
    ] {
        for &id in edges {
            let edge = own.body.edge(id).ok_or(Stop::of(LOST))?;
            let (t0, t1) = edge.range;
            // The edge cut where it meets the other surface's boundary.
            let mut at: Vec<f64> = vec![t0, t1];
            for cut in &found {
                let mine = match operand {
                    Operand::A => cut.a_edge,
                    Operand::B => cut.b_edge,
                };
                if mine == Some(id)
                    && let Some(t) = locate(&edge.curve, edge.range, cut.point, tol)
                {
                    at.push(t);
                }
            }
            at.sort_by(f64::total_cmp);
            at.dedup();
            // On the surface by proof (`geom::certify`: the distance from
            // the curve to the surface bounded over every piece, pieces
            // halved where the bound is over the tolerance, at most
            // `LYING_PIECES`); five places first, to turn away the many
            // edges that are not, cheaply. A proof not reached is no
            // imprint: samples alone took a circle 5e4 tolerances off a
            // surface between them for one lying on it.
            let shape = CurveBound::new(&edge.curve);
            let lies = |from: f64, to: f64| {
                let near = (0..=4).all(|k| {
                    let t = from + (to - from) * f64::from(k) / 4.0;
                    surface_distance(other, edge.curve.eval(t)) <= tol
                });
                if !near {
                    return false;
                }
                let Some(start) = other.project(edge.curve.eval(from)) else {
                    return false;
                };
                let within = Within {
                    target: tol,
                    most: LYING_PIECES,
                };
                curve_on_surface(&shape, (from, to), other.surface, Some(start), &[], within)
                    .is_ok()
            };
            // Consecutive pieces that lie on the surface, as one.
            let mut pieces: Vec<(f64, f64)> = Vec::new();
            for (&from, &to) in at.iter().zip(at.iter().skip(1)) {
                if to > from && lies(from, to) {
                    match pieces.last_mut() {
                        Some(last) if last.1 == from => last.1 = to,
                        _ => pieces.push((from, to)),
                    }
                }
            }
            // The stretches already on the face the edge lies on: a piece
            // is cut where one of them ends on it, and what lies along
            // one of them (its middle on it) is left out.
            let on_face = |s: &&Stretch| match operand {
                Operand::A => s.fb == fb,
                Operand::B => s.fa == fa,
            };
            let mut uncovered: Vec<(f64, f64)> = Vec::new();
            for (from, to) in pieces {
                let mut cuts = vec![from, to];
                for stretch in sections.stretches.iter().filter(on_face) {
                    for vertex in [stretch.start, stretch.end] {
                        if let Some(&p) = vertices.points.get(vertex)
                            && let Some(t) = locate(&edge.curve, (from, to), p, tol)
                            && t > from
                            && t < to
                        {
                            cuts.push(t);
                        }
                    }
                }
                cuts.sort_by(f64::total_cmp);
                cuts.dedup();
                for (&lo, &hi) in cuts.iter().zip(cuts.iter().skip(1)) {
                    let middle = edge.curve.eval(0.5 * (lo + hi));
                    let imprinted = sections
                        .stretches
                        .iter()
                        .filter(on_face)
                        .any(|s| locate(&s.curve, s.range, middle, tol).is_some());
                    if hi > lo && !imprinted {
                        match uncovered.last_mut() {
                            Some(last) if last.1 == lo => last.1 = hi,
                            _ => uncovered.push((lo, hi)),
                        }
                    }
                }
            }
            for (from, to) in uncovered {
                let whole = from <= t0 && to >= t1;
                let meeting = MeetingCurve {
                    curve: edge.curve.clone(),
                    range: (from, to),
                    closed: whole && edge.start == edge.end,
                    tangent: true,
                    exact: true,
                    deviation: 0.0,
                };
                clip(
                    &meeting, &found, a, fa, b, fb, vertices, tolerance, sections,
                )?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
pub(super) mod tests {
    thread_local! {
        /// Booleans on this thread work without covers (`cover.rs`), to
        /// hold them to the results with covers bit for bit.
        pub(in crate::boolean) static NO_COVERS: std::cell::Cell<bool> =
            const { std::cell::Cell::new(false) };
        /// The pairs of faces whose surfaces booleans on this thread
        /// intersected.
        pub(in crate::boolean) static PAIRS_MET: std::cell::Cell<usize> =
            const { std::cell::Cell::new(0) };
    }

    use super::*;
    use craft3d_geom::{Cylinder, Plane, Sphere, Torus};

    /// A closed spline (a cubic through eight points round a circle of
    /// radius `r`, clamped where it starts and ends, as a trace of a
    /// closed meeting curve is) and the cut at parameter `t` of it, an
    /// edge's crossing (`a_edge` 7).
    fn loop_and_place(r: f64, t: f64) -> (Curve3, (f64, f64), Cut) {
        let mut ctrl: Vec<Vec3> = (0..8)
            .map(|i| {
                let a = std::f64::consts::TAU * f64::from(i) / 8.0;
                Vec3::new(r * a.cos(), r * a.sin(), 0.0)
            })
            .collect();
        ctrl.push(ctrl[0]);
        let mut knots = vec![0.0; 4];
        knots.extend((1..6).map(|i| f64::from(i) / 6.0));
        knots.extend([1.0; 4]);
        let curve = Curve3::Nurbs(craft3d_geom::NurbsCurve::polynomial(3, knots, ctrl).unwrap());
        let place = Cut {
            t,
            point: curve.eval(t),
            a_edge: Some(EdgeId(7)),
            b_edge: None,
            end: false,
        };
        (curve, (0.0, 1.0), place)
    }

    /// The seam of a closed curve a tolerance or two from a place where an
    /// edge meets a surface, on either side of it, is that place
    /// (`ends_at_places`; the sea kayak's stitch holes, repros 21 to 24,
    /// refused "two points closer together than the tolerance can tell
    /// apart" before); one further than the guard, or one the curve comes
    /// back to only round the loop, is not; an open curve's end is as it
    /// was. At three scales.
    #[test]
    fn the_seam_of_a_closed_curve_next_to_a_place_is_that_place() {
        for k in [1e-3, 1.0, 1e6] {
            let (tol, r) = (1e-2 * k, 10.0 * k);
            let guard = 2.0 * tol;
            // The parameter `dt` from the seam is about `dt` times the
            // speed (about 2πr) away.
            let speed = std::f64::consts::TAU * r;
            for (from_seam, taken) in [(1.5, true), (0.8, true), (2.5, false)] {
                let dt = from_seam * tol / speed;
                for t in [dt, 1.0 - dt] {
                    let (curve, range, place) = loop_and_place(r, t);
                    let seam = Cut {
                        t: 0.0,
                        point: curve.eval(0.0),
                        a_edge: None,
                        b_edge: None,
                        end: true,
                    };
                    let d = seam.point.distance(place.point);
                    assert!(
                        (d - from_seam * tol).abs() < 0.1 * tol,
                        "{d} for {from_seam} tolerances at {k}"
                    );
                    let along = vec![seam, place];
                    let found = ends_at_places(along, &curve, range, true, guard);
                    assert_eq!(found.len(), 2, "at {k}");
                    let first = found[0];
                    assert_eq!(first.t, 0.0, "the seam stays where it is, at {k}");
                    if taken {
                        assert!(!first.end, "at {k}, {from_seam} from the seam at {t}");
                        assert_eq!(first.point, place.point, "at {k}");
                        assert_eq!(first.a_edge, Some(EdgeId(7)), "at {k}");
                    } else {
                        assert!(first.end, "at {k}, {from_seam} from the seam at {t}");
                        assert_eq!(first.point, seam.point, "at {k}");
                    }
                }
            }
            // A place halfway round the loop that the seam is near in
            // space only (the loop pinched there) is not taken: a loop
            // whose control points come back past the seam.
            let (curve, range, _) = loop_and_place(r, 0.5);
            let far = Cut {
                t: 0.5,
                point: curve.eval(0.0) + Vec3::new(0.0, tol, 0.0),
                a_edge: Some(EdgeId(7)),
                b_edge: None,
                end: false,
            };
            let seam = Cut {
                t: 0.0,
                point: curve.eval(0.0),
                a_edge: None,
                b_edge: None,
                end: true,
            };
            let found = ends_at_places(vec![seam, far], &curve, range, true, guard);
            assert!(found[0].end, "at {k}: {:?}", found[0]);
            // An open curve's end next to a place it stays at is dropped
            // for the place, as before.
            let (curve, _, place) = loop_and_place(r, 1.5 * tol / speed);
            let start = Cut {
                t: 0.0,
                point: curve.eval(0.0),
                a_edge: None,
                b_edge: None,
                end: true,
            };
            let end = Cut {
                t: 0.5,
                point: curve.eval(0.5),
                a_edge: None,
                b_edge: None,
                end: true,
            };
            let found = ends_at_places(vec![start, place, end], &curve, (0.0, 0.5), false, guard);
            assert_eq!(found.len(), 2, "at {k}: {found:?}");
            assert_eq!(found[0].point, place.point, "at {k}");
        }
    }

    /// The re-ask with the smaller face's room (`in_room`, the Garabit
    /// valley's finding 5), held without a terrain: a meeting that cannot
    /// be traced wherever its box reaches a "lake" beside a small face is
    /// asked again in the small face's room, which holds both faces where
    /// they overlap and leaves the lake out; at three scales.
    #[test]
    fn a_meeting_that_fails_in_the_larger_faces_room_is_asked_in_the_smaller_ones() {
        use std::cell::RefCell;
        for k in [1e-3, 1.0, 1e3] {
            let tol = 1e-6 * k;
            let at = |lo: [f64; 3], hi: [f64; 3]| {
                Bbox3::from_points([
                    Vec3::new(lo[0] * k, lo[1] * k, lo[2] * k),
                    Vec3::new(hi[0] * k, hi[1] * k, hi[2] * k),
                ])
            };
            // A terrain face 1 km across and a pit's bottom, 30 × 20 m.
            let terrain = at([0.0, -500.0, 700.0], [1000.0, 500.0, 800.0]);
            let pit = at([400.0, 112.0, 735.0], [430.0, 132.0, 735.0]);
            // A lake 12 m beside the pit: within the terrain's room (about
            // 18 m), not within the pit's (about half a metre).
            let lake = Vec3::new(415.0 * k, 100.0 * k, 735.0 * k);
            let room = 16.0 * tol + 0.013 * terrain.diagonal().max(pit.diagonal());
            let within = overlap(&terrain, &pit, room).unwrap();
            assert!(within.contains(lake, 0.0), "the lake is in the larger room");
            let asked = RefCell::new(Vec::new());
            let traced = |b: &Bbox3| {
                asked.borrow_mut().push(*b);
                if b.contains(lake, 0.0) {
                    Err(GeomError::NoConvergence(
                        "an intersection curve needs more points than the cap allows",
                    ))
                } else {
                    Ok(*b)
                }
            };
            let used = in_room(&terrain, &pit, &within, room, tol, traced).unwrap();
            assert_eq!(asked.borrow().len(), 2, "asked again once at {k}");
            assert!(!used.contains(lake, 0.0), "at {k}");
            // Still everything of both faces where they overlap.
            let both = overlap(&terrain, &pit, 0.0).unwrap();
            let (lo, hi) = both.corners().unwrap();
            assert!(used.contains(lo, 0.0) && used.contains(hi, 0.0), "at {k}");
            // What cannot be traced in the smaller room either is refused
            // with the first reason.
            let never =
                |_: &Bbox3| -> Result<(), GeomError> { Err(GeomError::Unsupported("first")) };
            assert!(matches!(
                in_room(&terrain, &pit, &within, room, tol, never),
                Err(GeomError::Unsupported("first"))
            ));
            // Another failure is not asked again.
            let count = RefCell::new(0);
            let invalid = |_: &Bbox3| -> Result<(), GeomError> {
                *count.borrow_mut() += 1;
                Err(GeomError::InvalidParameter("other"))
            };
            assert!(matches!(
                in_room(&terrain, &pit, &within, room, tol, invalid),
                Err(GeomError::InvalidParameter("other"))
            ));
            assert_eq!(*count.borrow(), 1);
            // Faces of one size have no smaller room: asked once.
            let other = at([400.0, 112.0, 735.0], [1400.0, 1112.0, 835.0]);
            let room = 16.0 * tol + 0.013 * terrain.diagonal().max(other.diagonal());
            let within = overlap(&terrain, &other, room).unwrap();
            let count = RefCell::new(0);
            let fails = |_: &Bbox3| -> Result<(), GeomError> {
                *count.borrow_mut() += 1;
                Err(GeomError::NoConvergence("first"))
            };
            assert!(in_room(&terrain, &other, &within, room, tol, fails).is_err());
            assert_eq!(*count.borrow(), 1, "at {k}");
        }
    }
    use craft3d_math::Frame;

    /// A ball of radius 2 that dips `depth` through the plane z = 0: they
    /// meet in a circle of radius √(depth (4 − depth)), and the cap below
    /// the plane is `depth` thick.
    fn dip(k: f64, depth: f64, swap: bool) -> Passage {
        let tol = 1e-6 * k;
        let plane = Surface::Plane(Plane::new(Frame::WORLD));
        let r = 2.0 * k;
        let centre = Vec3::new(0.3 * k, -0.2 * k, r - depth);
        let ball = Surface::Sphere(Sphere::new(Frame::WORLD.at(centre).unwrap(), r).unwrap());
        let rim = (depth * (2.0 * r - depth)).sqrt();
        let p = Vec3::new(centre.x + rim * 0.6, centre.y + rim * 0.8, 0.0);
        let tangent = Vec3::new(-0.8, 0.6, 0.0);
        if swap {
            passage(
                Sheet::bare(&ball),
                Sheet::bare(&plane),
                p,
                tangent,
                10.0 * k,
                tol,
            )
        } else {
            passage(
                Sheet::bare(&plane),
                Sheet::bare(&ball),
                p,
                tangent,
                10.0 * k,
                tol,
            )
        }
    }

    #[test]
    fn a_dip_thinner_than_twice_the_tolerance_is_a_lens_of_that_thickness() {
        for k in [1e-3, 1.0, 1e6] {
            let tol = 1e-6 * k;
            for swap in [false, true] {
                // Well through: a crossing, whichever surface is walked.
                for depth in [2.5 * tol, 10.0 * tol, 1e-3 * k, 0.5 * k, 2.0 * k] {
                    assert_eq!(dip(k, depth, swap), Passage::Crossing, "{depth} at {k}");
                }
                // Barely through: a lens, measured to a few percent (the
                // cap is as thick as the dip is deep).
                for parts in [0.05, 0.1, 0.4, 0.6, 1.0, 1.7] {
                    let depth = parts * tol;
                    match dip(k, depth, swap) {
                        Passage::Lens(found) => assert!(
                            (found - depth).abs() <= 0.03 * depth,
                            "{found} for a dip of {depth} at scale {k}"
                        ),
                        other => panic!("{other:?} for a dip of {parts} tolerances at {k}"),
                    }
                }
            }
        }
    }

    #[test]
    fn a_touch_is_a_touch_and_a_crossing_at_any_angle_a_crossing() {
        for k in [1e-3, 1.0, 1e6] {
            let tol = 1e-6 * k;
            // A cylinder lying on a plane touches it along a line.
            let plane = Surface::Plane(Plane::new(Frame::WORLD));
            let lying = Frame::new(Vec3::new(0.0, 0.0, k), Vec3::X, Vec3::Y).unwrap();
            let cylinder = Surface::Cylinder(Cylinder::new(lying, k).unwrap());
            let on = Vec3::new(0.4 * k, 0.0, 0.0);
            for (a, b) in [(&plane, &cylinder), (&cylinder, &plane)] {
                assert_eq!(
                    passage(Sheet::bare(a), Sheet::bare(b), on, Vec3::X, 10.0 * k, tol),
                    Passage::Touch
                );
            }
            // A plane through the top of a torus at a slant: the tube
            // bends away from its tangent plane as x^2 / 2k, so the two
            // enclose a lens slant^2 k / 2 thick. At a slant of 0.1 that
            // is five thousand tolerances, a crossing.
            let torus = Surface::Torus(Torus::new(Frame::WORLD, 4.0 * k, k).unwrap());
            let top = Vec3::new(4.0 * k, 0.0, k);
            for slant in [0.1, 1.0_f64] {
                let normal = Vec3::new(slant.sin(), 0.0, slant.cos());
                let slanted = Surface::Plane(Plane::new(Frame::from_z(top, normal).unwrap()));
                // The curve leaves `top` along y, by symmetry.
                assert_eq!(
                    passage(
                        Sheet::bare(&torus),
                        Sheet::bare(&slanted),
                        top,
                        Vec3::Y,
                        10.0 * k,
                        tol
                    ),
                    Passage::Crossing,
                    "slant {slant} at {k}"
                );
                assert_eq!(
                    passage(
                        Sheet::bare(&slanted),
                        Sheet::bare(&torus),
                        top,
                        Vec3::Y,
                        10.0 * k,
                        tol
                    ),
                    Passage::Crossing
                );
            }
            // At a slant of 0.0006 the lens is 0.18 of a tolerance thick.
            let slant = 6e-4_f64;
            let normal = Vec3::new(slant.sin(), 0.0, slant.cos());
            let slanted = Surface::Plane(Plane::new(Frame::from_z(top, normal).unwrap()));
            for (a, b) in [(&torus, &slanted), (&slanted, &torus)] {
                match passage(Sheet::bare(a), Sheet::bare(b), top, Vec3::Y, 10.0 * k, tol) {
                    Passage::Lens(found) => assert!(
                        (found - 0.18 * tol).abs() <= 0.01 * tol,
                        "{found} at scale {k}"
                    ),
                    other => panic!("{other:?} at scale {k}"),
                }
            }
        }
    }
}
