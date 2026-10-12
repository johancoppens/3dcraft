//! Preparing the operands and finding where their faces meet: the curves
//! in which two surfaces meet, cut down to the stretches that are within
//! both faces.

use super::hits::{curve_hits, signed_distance};
use super::outline::{Hull, Outline, Seg, Side, boxes_meet, locate, nearest, surface_distance};
use super::{
    AMBIGUOUS, LOST, Meeting, NEAR, NEARLY_COINCIDENT, Operand, Stop, Vertices, face_id, geometry,
};
use crate::{KernelError, MAX_REPORTED};
use craft3d_geom::Surface;
use craft3d_geom::intersect::CurveIntersection;
use craft3d_geom::intersect::{MeetingCurve, SurfaceMeeting, meet_surfaces};
use craft3d_math::{Bbox3, Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind, EdgeId, FaceId, VertexId, check};
use std::collections::{BTreeMap, BTreeSet};

/// One operand, prepared.
pub(super) struct Solid<'a> {
    pub operand: Operand,
    pub body: &'a Body,
    /// Per face.
    pub outlines: Vec<Outline>,
    pub boxes: Vec<Bbox3>,
    pub hulls: Vec<Hull>,
    /// The merged vertex of each of the body's vertices.
    pub vertices: Vec<usize>,
}

impl Solid<'_> {
    pub fn name(&self, face: usize) -> (Operand, FaceId) {
        (self.operand, face_id(face))
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

    /// A box around the whole body.
    fn bbox(&self) -> Bbox3 {
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
}

#[derive(Default)]
pub(super) struct Sections {
    pub stretches: Vec<Stretch>,
    /// Per face of A: the faces of B on the same surface, and whether the
    /// surfaces' normals agree. And the other way round.
    pub coincident_a: BTreeMap<usize, Vec<(usize, bool)>>,
    pub coincident_b: BTreeMap<usize, Vec<(usize, bool)>>,
    /// Per edge: the vertices to split it at.
    pub splits_a: BTreeMap<EdgeId, BTreeSet<usize>>,
    pub splits_b: BTreeMap<EdgeId, BTreeSet<usize>>,
}

pub(super) fn prepare<'a>(
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
    let tol = tolerance.linear;
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
        let outline = Outline::new(face.surface.clone(), face.reversed, loops, 4.0 * tol)
            .map_err(|reason| Stop::of(reason).at(&named))?;
        let hull = outline.hull(tol);
        boxes.push(hull.bbox());
        hulls.push(hull);
        outlines.push(outline);
    }
    Ok(Solid {
        operand,
        body,
        outlines,
        boxes,
        hulls,
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
    /// Where the curve itself starts or ends, rather than where an edge
    /// meets a surface.
    end: bool,
}

/// Adds to `cuts` where the edges of `face` of `own` meet `surface`.
fn edge_cuts(
    own: &Solid,
    face: usize,
    surface: &Surface,
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
                for hit in hits {
                    if locate(&edge.curve, edge.range, hit.point, tol).is_none() {
                        continue;
                    }
                    // A touch is a cut like any other: the stretches on
                    // either side of it are judged on their own.
                    cut(hit.point, true);
                }
            }
        }
    }
    Ok(())
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
    let (Some(oa), Some(ob), Some(box_a), Some(box_b)) = (
        a.outlines.get(fa),
        b.outlines.get(fb),
        a.boxes.get(fa),
        b.boxes.get(fb),
    ) else {
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
    // An end of the curve next to a place where an edge meets a surface
    // is that place.
    // (Not the seam of a closed spline: no stretch may run across that.)
    if !closed {
        let places: Vec<Vec3> = along
            .iter()
            .filter(|cut| !cut.end)
            .map(|cut| cut.point)
            .collect();
        along.retain(|cut| {
            !cut.end
                || !places
                    .iter()
                    .any(|p| p.distance(cut.point) < vertices.guard)
        });
    }
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
        // Outside a face's box is outside the face.
        if !(box_a.contains(middle, 2.0 * tol) && box_b.contains(middle, 2.0 * tol)) {
            continue;
        }
        let mut inside = [false, false];
        let mut keep = true;
        for (outline, inside) in [oa, ob].into_iter().zip(inside.iter_mut()) {
            let side = |p: Vec3| outline.side(p, tol).map_err(|r| Stop::of(r).at(&named));
            let here = side(middle)?;
            match here {
                Side::Out => {
                    keep = false;
                    break;
                }
                Side::In => *inside = true,
                Side::On => {}
            }
            if [side(at(0.25))?, side(at(0.75))?]
                .iter()
                .any(|&q| q != here)
            {
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

/// How far apart two faces on surfaces of one kind are where they are over
/// each other: the greatest distance from a place on the boundary of
/// either (or, without a boundary, on it) that is in or on the other face
/// to the other's surface. `None` when no such place was found.
fn gap(a: &Outline, b: &Outline, tol: f64) -> Result<Option<f64>, Stop> {
    let mut widest: Option<f64> = None;
    for (own, other) in [(a, b), (b, a)] {
        let mut places: Vec<Vec3> = Vec::new();
        for seg in own.loops.iter().flatten() {
            places.extend([seg.a, seg.at(0.25), seg.at(0.5), seg.at(0.75), seg.b]);
        }
        if own.loops.is_empty() {
            places = own.interior_points(tol, 4.0 * tol, 4).map_err(Stop::of)?;
        }
        for p in places {
            let Some(distance) = signed_distance(&other.surface, p).map(f64::abs) else {
                continue;
            };
            // Further than this they are plainly apart here.
            if distance >= 2.0 * tol {
                continue;
            }
            let (u, v) = other.surface.project(p).map_err(|e| geometry(e, &[]))?;
            let foot = other.surface.eval(u, v);
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
fn clear_of(surface: &Surface, hull: &Hull, margin: f64) -> bool {
    if let Surface::Plane(plane) = surface {
        // Flat: how far the hull reaches along the normal says it exactly.
        let normal = plane.frame().z();
        let (low, high) = hull.span(normal);
        let level = plane.frame().origin().dot(normal);
        return low > high || low - level > margin || level - high > margin;
    }
    let Some((lo, hi)) = hull.bbox().corners() else {
        return true;
    };
    // The distance changes by no more than the distance moved.
    signed_distance(surface, lo.lerp(hi, 0.5))
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

pub(super) fn section(
    a: &Solid,
    b: &Solid,
    vertices: &mut Vertices,
    tolerance: &Tolerance,
) -> Result<Sections, Stop> {
    let tol = tolerance.linear;
    let mut sections = Sections::default();
    // Only where both bodies are can their faces meet.
    // (With room to spare, so that where a curve is cut off is not where
    // anything else is.)
    let (box_a, box_b) = (a.bbox(), b.bbox());
    let Some(snug) = overlap(&box_a, &box_b, 2.0 * tol) else {
        return Ok(sections);
    };
    let room = 16.0 * tol + 0.013 * snug.diagonal();
    let Some(within) = overlap(&box_a, &box_b, room) else {
        return Ok(sections);
    };
    for (fa, (oa, box_a)) in a.outlines.iter().zip(&a.boxes).enumerate() {
        for (fb, (ob, box_b)) in b.outlines.iter().zip(&b.boxes).enumerate() {
            if !boxes_meet(box_a, box_b, 2.0 * tol) {
                continue;
            }
            let (Some(hull_a), Some(hull_b)) = (a.hulls.get(fa), b.hulls.get(fb)) else {
                return Err(Stop::of(LOST));
            };
            if clear_of(&oa.surface, hull_b, 2.0 * tol) || clear_of(&ob.surface, hull_a, 2.0 * tol)
            {
                continue;
            }
            let named = [a.name(fa), b.name(fb)];
            let met = meet_surfaces(&oa.surface, &ob.surface, &within, tolerance)
                .map_err(|e| geometry(e, &named))?;
            // Faces on one surface, or on two that never cross: the same
            // surface when they are within half the tolerance of each
            // other where one is over the other, apart when twice the
            // tolerance or more lies between them, and in between neither
            // (the result would be glued together or left apart by a
            // margin nobody asked for).
            let alike = std::mem::discriminant(&oa.surface) == std::mem::discriminant(&ob.surface);
            let unsure = match &met {
                SurfaceMeeting::Coincident { .. } => gap(oa, ob, tol)
                    .map_err(|s| s.at(&named))?
                    .is_some_and(|g| g > 0.5 * tol),
                SurfaceMeeting::None if alike => {
                    gap(oa, ob, tol).map_err(|s| s.at(&named))?.is_some()
                }
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
                }
                // Points where the surfaces only touch divide nothing.
                SurfaceMeeting::Meets(meeting) => {
                    if meeting.curves.is_empty() {
                        continue;
                    }
                    let mut found = Vec::new();
                    edge_cuts(a, fa, &ob.surface, &named, tolerance, &mut found)?;
                    edge_cuts(b, fb, &oa.surface, &named, tolerance, &mut found)?;
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
    Ok(sections)
}
