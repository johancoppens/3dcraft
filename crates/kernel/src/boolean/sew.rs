//! From kept regions to a body.
//!
//! The regions already share arcs, so sewing is bookkeeping, with two
//! exceptions that keep the result a manifold:
//!
//! - **More than two faces along an arc** (the parts of the result touch
//!   along an edge). Around the arc faces and material alternate: seen
//!   along the arc, a face that runs against it has the solid
//!   counter-clockwise of it, a face that runs with it clockwise. Each
//!   face that runs against the arc is paired with the next face
//!   counter-clockwise, and each pair gets its own copy of the edge.
//! - **Several fans of faces at a vertex** (the parts touch at a point).
//!   Going from an edge end across a face corner to the next edge end
//!   reaches the faces of one fan; each fan gets its own copy of the
//!   vertex.
//!
//! The copies are in the same place and share nothing.

use super::classify::Kept;
use super::outline::{Seg, clearance, onto, outward_normal};
use super::{
    Arcs, BooleanResult, EdgeSource, Entity, FaceSource, LOST, NON_MANIFOLD, Operand, Stop,
    TANGENT_EDGE, VertexSource, Vertices, face_id,
};
use craft3d_geom::{Curve3, Surface};
use craft3d_math::{Frame, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId, FaceId, VertexId};
use std::collections::BTreeMap;
use std::f64::consts::TAU;

/// Faces closer together than this (radians) around an edge cannot be put
/// in order reliably.
const ANGLE_GUARD: f64 = 1e-6;

/// Where a vertex in the making came from; a new one gets its number when
/// the body is finished.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Origin {
    Kept { operand: Operand, vertex: VertexId },
    New { a: Entity, b: Entity },
}

#[derive(Clone, Debug)]
pub(super) struct WorkVertex {
    pub point: Vec3,
    pub origin: Origin,
}

#[derive(Clone, Debug)]
pub(super) struct WorkEdge {
    pub curve: Curve3,
    pub range: (f64, f64),
    pub start: usize,
    pub end: usize,
    pub source: EdgeSource,
}

#[derive(Clone, Debug)]
pub(super) struct WorkFace {
    pub surface: Surface,
    pub reversed: bool,
    /// Uses of edges: the edge and whether it is run backwards.
    pub loops: Vec<Vec<(usize, bool)>>,
    pub source: FaceSource,
    /// Every face of the operands that this face has a piece of, the
    /// source among them, in order.
    pub contributors: Vec<FaceSource>,
}

/// A body in the making. Removed edges and faces leave a gap, so that
/// indices stay put; vertices that nothing uses are left out at the end.
pub(super) struct Work {
    pub vertices: Vec<WorkVertex>,
    pub edges: Vec<Option<WorkEdge>>,
    pub faces: Vec<Option<WorkFace>>,
}

/// The root of `i` in a union-find forest, halving paths on the way.
pub(super) fn root(parents: &mut [usize], mut i: usize) -> usize {
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

pub(super) fn unite(parents: &mut [usize], a: usize, b: usize) {
    let (x, y) = (root(parents, a), root(parents, b));
    if let Some(slot) = parents.get_mut(x.max(y)) {
        *slot = x.min(y);
    }
}

/// One use of an arc by a kept region.
#[derive(Clone, Copy)]
struct Use {
    face: usize,
    loop_: usize,
    at: usize,
    reversed: bool,
}

/// Which copy of an arc each of its uses gets, when more than two faces
/// meet along it: per use, in the order given, the number of its pair.
fn pair_up(
    arc: &super::Arc,
    uses: &[Use],
    kept: &[Kept],
    named: &dyn Fn(usize) -> Vec<(Operand, FaceId)>,
) -> Result<Vec<usize>, Stop> {
    let all: Vec<usize> = uses.iter().map(|u| u.face).collect();
    let stop =
        |reason| Stop::of(reason).at(&all.iter().flat_map(|&f| named(f)).collect::<Vec<_>>());
    let against = uses.iter().filter(|u| u.reversed).count();
    if 2 * against != uses.len() {
        return Err(stop(NON_MANIFOLD));
    }
    let middle = 0.5 * (arc.range.0 + arc.range.1);
    let (point, tangent) = arc.curve.derivatives(middle);
    let frame = Frame::from_z(point, tangent).ok_or(Stop::of(LOST))?;
    // Per use the direction in which its face leaves the arc, as an angle
    // about the arc.
    let mut around = Vec::with_capacity(uses.len());
    for (index, u) in uses.iter().enumerate() {
        let face = kept.get(u.face).ok_or(Stop::of(LOST))?;
        let outward = outward_normal(&face.surface, face.reversed, point).ok_or(Stop::of(LOST))?;
        let travel = if u.reversed { -tangent } else { tangent };
        let into = outward.cross(travel);
        let angle = into.dot(frame.y()).atan2(into.dot(frame.x()));
        if !angle.is_finite() {
            return Err(Stop::of(LOST));
        }
        around.push((angle, index));
    }
    around.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let count = around.len();
    // Faces that leave the arc in the same direction (they are tangent
    // along it) are told apart a little way off it: by where each face is
    // at one small distance from the arc.
    let tied = |around: &[(f64, usize)], place: usize| {
        let (Some(here), Some(next)) = (around.get(place), around.get((place + 1) % count)) else {
            return false;
        };
        let gap = (next.0 - here.0).rem_euclid(TAU);
        gap < ANGLE_GUARD || gap > TAU - ANGLE_GUARD
    };
    if (0..count).any(|place| tied(&around, place)) {
        let reach = 0.02
            * arc
                .curve
                .eval(arc.range.0 + 0.25 * (arc.range.1 - arc.range.0))
                .distance(
                    arc.curve
                        .eval(arc.range.0 + 0.75 * (arc.range.1 - arc.range.0)),
                );
        if !(reach.is_finite() && reach > 0.0) {
            return Err(stop(TANGENT_EDGE));
        }
        let start = (0..count)
            .find(|&place| !tied(&around, (place + count - 1) % count))
            .unwrap_or(0);
        around.rotate_left(start);
        let mut from = 0;
        while from < count {
            let mut to = from;
            while to + 1 < count && tied(&around, to) {
                to += 1;
            }
            if to > from {
                let run = around.get_mut(from..=to).ok_or(Stop::of(LOST))?;
                let base = run.first().map_or(0.0, |first| first.0);
                for entry in run.iter_mut() {
                    let u = uses.get(entry.1).ok_or(Stop::of(LOST))?;
                    let face = kept.get(u.face).ok_or(Stop::of(LOST))?;
                    let outward = outward_normal(&face.surface, face.reversed, point)
                        .ok_or(Stop::of(LOST))?;
                    let travel = if u.reversed { -tangent } else { tangent };
                    let into = outward.cross(travel);
                    let off =
                        onto(&face.surface, point + into * reach).ok_or(Stop::of(LOST))? - point;
                    let angle = off.dot(frame.y()).atan2(off.dot(frame.x()));
                    // Close to where it was, on the same turn.
                    entry.0 = base + (angle - base + TAU / 2.0).rem_euclid(TAU) - TAU / 2.0;
                }
                run.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
                for pair in run.windows(2) {
                    if let [a, b] = pair
                        && b.0 - a.0 < ANGLE_GUARD
                    {
                        return Err(stop(TANGENT_EDGE));
                    }
                }
            }
            from = to + 1;
        }
    }
    let mut pairs = vec![usize::MAX; uses.len()];
    let mut next_pair = 0;
    for (place, &(_, index)) in around.iter().enumerate() {
        let &(_, next) = around.get((place + 1) % count).ok_or(Stop::of(LOST))?;
        let (Some(here), Some(there)) = (uses.get(index), uses.get(next)) else {
            return Err(Stop::of(LOST));
        };
        if !here.reversed {
            continue;
        }
        // The solid lies counter-clockwise of a face that runs against
        // the arc, up to a face that runs with it.
        if there.reversed {
            return Err(stop(NON_MANIFOLD));
        }
        for slot in [index, next] {
            *pairs.get_mut(slot).ok_or(Stop::of(LOST))? = next_pair;
        }
        next_pair += 1;
    }
    if pairs.contains(&usize::MAX) {
        return Err(stop(NON_MANIFOLD));
    }
    Ok(pairs)
}

pub(super) fn sew(kept: Vec<Kept>, arcs: &Arcs, vertices: &Vertices) -> Result<Work, Stop> {
    // Per arc its uses, in the order of the faces.
    let mut uses: BTreeMap<usize, Vec<Use>> = BTreeMap::new();
    for (face, region) in kept.iter().enumerate() {
        for (loop_, halves) in region.loops.iter().enumerate() {
            for (at, &(arc, reversed)) in halves.iter().enumerate() {
                uses.entry(arc).or_default().push(Use {
                    face,
                    loop_,
                    at,
                    reversed,
                });
            }
        }
    }

    // Edges: one per arc, or one per pair of faces around it. Per use of
    // an arc, the edge it gets.
    let named = |face: usize| -> Vec<(Operand, FaceId)> {
        kept.get(face)
            .map(|k| (k.source.operand, k.source.face))
            .into_iter()
            .collect()
    };
    let mut edge_arcs: Vec<usize> = Vec::new();
    let mut edge_of: BTreeMap<(usize, usize, usize), usize> = BTreeMap::new();
    for (&arc_index, arc_uses) in &uses {
        let pairs = if arc_uses.len() > 2 {
            let arc = arcs.arcs.get(arc_index).ok_or(Stop::of(LOST))?;
            pair_up(arc, arc_uses, &kept, &named)?
        } else {
            // A use too few shows up in the checker, as it always did.
            vec![0; arc_uses.len()]
        };
        let first = edge_arcs.len();
        let copies = pairs.iter().copied().max().map_or(0, |most| most + 1);
        edge_arcs.extend(std::iter::repeat_n(arc_index, copies));
        for (u, pair) in arc_uses.iter().zip(pairs) {
            edge_of.insert((u.face, u.loop_, u.at), first + pair);
        }
    }

    // Fans: edge ends are numbered 2 × edge, plus 1 for the end.
    let mut faces = Vec::with_capacity(kept.len());
    let mut fans: Vec<usize> = (0..2 * edge_arcs.len()).collect();
    let end_of =
        |edge: usize, reversed: bool, leaving: bool| 2 * edge + usize::from(reversed == leaving);
    for (face, region) in kept.into_iter().enumerate() {
        let mut loops = Vec::with_capacity(region.loops.len());
        for (loop_, halves) in region.loops.iter().enumerate() {
            let mut coedges = Vec::with_capacity(halves.len());
            for (at, &(_, reversed)) in halves.iter().enumerate() {
                let &edge = edge_of.get(&(face, loop_, at)).ok_or(Stop::of(LOST))?;
                coedges.push((edge, reversed));
            }
            let after = coedges.iter().cycle().skip(1);
            for (arriving, leaving) in coedges.iter().zip(after) {
                unite(
                    &mut fans,
                    end_of(arriving.0, arriving.1, false),
                    end_of(leaving.0, leaving.1, true),
                );
            }
            loops.push(coedges);
        }
        faces.push(Some(WorkFace {
            surface: region.surface,
            reversed: region.reversed,
            loops,
            source: region.source,
            contributors: vec![region.source],
        }));
    }

    // Vertices: one per place and fan.
    let mut work_vertices: Vec<WorkVertex> = Vec::new();
    let mut vertex_of: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    let mut edges = Vec::with_capacity(edge_arcs.len());
    for (edge, &arc_index) in edge_arcs.iter().enumerate() {
        let arc = arcs.arcs.get(arc_index).ok_or(Stop::of(LOST))?;
        let mut vertex = |at: usize, end: usize| -> Result<usize, Stop> {
            let fan = root(&mut fans, 2 * edge + end);
            if let Some(&id) = vertex_of.get(&(at, fan)) {
                return Ok(id);
            }
            let (Some(&point), Some(meeting)) =
                (vertices.points.get(at), vertices.meetings.get(at))
            else {
                return Err(Stop::of(LOST));
            };
            let origin = match meeting.kept {
                Some((operand, vertex)) => Origin::Kept { operand, vertex },
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
                    Origin::New { a, b }
                }
            };
            work_vertices.push(WorkVertex { point, origin });
            vertex_of.insert((at, fan), work_vertices.len() - 1);
            Ok(work_vertices.len() - 1)
        };
        let (start, end) = (vertex(arc.start, 0)?, vertex(arc.end, 1)?);
        edges.push(Some(WorkEdge {
            curve: arc.curve.clone(),
            range: arc.range,
            start,
            end,
            source: arc.source,
        }));
    }
    Ok(Work {
        vertices: work_vertices,
        edges,
        faces,
    })
}

/// Gives every bounded face on a sphere a frame that suits it.
///
/// A sphere is the same surface whichever way its poles point, but what
/// reads a face through the surface's parameters (mass properties, the
/// tessellator) is at its best when the poles are well away from the
/// face's boundary. A face bounded by one whole circle gets the circle's
/// axis as its sphere's axis, so that the boundary is a circle of
/// latitude; any other face the axis, among its old frame's axes and
/// their diagonals, whose two poles are furthest from its boundary.
fn reframe_spheres(work: &mut Work) {
    for face in work.faces.iter_mut().flatten() {
        let Surface::Sphere(sphere) = &face.surface else {
            continue;
        };
        let (old, radius) = (*sphere.frame(), sphere.radius());
        let center = old.origin();
        let edge = |index: usize| work.edges.get(index).and_then(Option::as_ref);
        let around_circle = match face.loops.as_slice() {
            [single] => match single.as_slice() {
                [(index, _)] => match edge(*index).map(|e| (&e.curve, e.start == e.end)) {
                    Some((Curve3::Circle(circle), true)) => circle.frame().at(center),
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        };
        let frame = around_circle.or_else(|| {
            let mut loops = Vec::with_capacity(face.loops.len());
            for loop_ in &face.loops {
                let mut segs = Vec::with_capacity(loop_.len());
                for &(index, reversed) in loop_ {
                    let made = edge(index)?;
                    segs.push(Seg {
                        curve: made.curve.clone(),
                        range: made.range,
                        a: work.vertices.get(made.start)?.point,
                        b: work.vertices.get(made.end)?.point,
                        reversed,
                    });
                }
                loops.push(segs);
            }
            if loops.is_empty() {
                return None;
            }
            let diagonal = 1.0 / 3.0_f64.sqrt();
            let mut best: Option<(f64, Vec3)> = None;
            for axis in [
                old.z(),
                old.x(),
                old.y(),
                old.vector(1.0, 1.0, 1.0) * diagonal,
                old.vector(-1.0, 1.0, 1.0) * diagonal,
                old.vector(1.0, -1.0, 1.0) * diagonal,
                old.vector(1.0, 1.0, -1.0) * diagonal,
            ] {
                let clear = clearance(&loops, center + axis * radius)
                    .min(clearance(&loops, center - axis * radius));
                if best.is_none_or(|(most, _)| clear > most) {
                    best = Some((clear, axis));
                }
            }
            let (_, axis) = best?;
            if axis == old.z() {
                return None;
            }
            Frame::from_z(center, axis)
        });
        if let Some(frame) = frame
            && let Ok(sphere) = craft3d_geom::Sphere::new(frame, radius)
        {
            face.surface = Surface::Sphere(sphere);
        }
    }
}

/// The body, with faces in their order and edges and vertices numbered in
/// the order the faces use them, and where everything came from.
pub(super) fn finish(mut work: Work) -> Result<BooleanResult, Stop> {
    reframe_spheres(&mut work);
    let mut body = Body::new(BodyKind::Solid);
    let mut vertex_ids: BTreeMap<usize, VertexId> = BTreeMap::new();
    let mut edge_ids: BTreeMap<usize, EdgeId> = BTreeMap::new();
    let mut faces = Vec::new();
    let mut merged = Vec::new();
    let mut edges = Vec::new();
    let mut vertices = Vec::new();
    let mut counts: BTreeMap<(Entity, Entity), u32> = BTreeMap::new();
    // Per edge the first face that used it; faces joined through edges.
    let mut first_use: BTreeMap<usize, usize> = BTreeMap::new();
    let live: Vec<WorkFace> = work.faces.into_iter().flatten().collect();
    let mut parents: Vec<usize> = (0..live.len()).collect();

    for (index, face) in live.into_iter().enumerate() {
        let mut loops = Vec::with_capacity(face.loops.len());
        for loop_ in face.loops {
            let mut coedges = Vec::with_capacity(loop_.len());
            for (edge_index, reversed) in loop_ {
                let edge = match edge_ids.get(&edge_index) {
                    Some(&edge) => edge,
                    None => {
                        let made = work
                            .edges
                            .get(edge_index)
                            .and_then(Option::as_ref)
                            .ok_or(Stop::of(LOST))?;
                        let mut vertex = |at: usize| -> Result<VertexId, Stop> {
                            if let Some(&id) = vertex_ids.get(&at) {
                                return Ok(id);
                            }
                            let made = work.vertices.get(at).ok_or(Stop::of(LOST))?;
                            let source = match made.origin {
                                Origin::Kept { operand, vertex } => {
                                    VertexSource::Kept { operand, vertex }
                                }
                                Origin::New { a, b } => {
                                    let count = counts.entry((a, b)).or_insert(0);
                                    let index = *count;
                                    *count = count.saturating_add(1);
                                    VertexSource::New { a, b, index }
                                }
                            };
                            let id = body.add_vertex(made.point);
                            vertices.push(source);
                            vertex_ids.insert(at, id);
                            Ok(id)
                        };
                        let (start, end) = (vertex(made.start)?, vertex(made.end)?);
                        let edge = body.add_edge(made.curve.clone(), made.range, start, end);
                        edges.push(made.source);
                        edge_ids.insert(edge_index, edge);
                        edge
                    }
                };
                coedges.push(Coedge { edge, reversed });
                let first = *first_use.entry(edge_index).or_insert(index);
                unite(&mut parents, first, index);
            }
            loops.push(body.add_loop(coedges));
        }
        body.add_face(face.surface, face.reversed, loops);
        faces.push(face.source);
        merged.push(face.contributors);
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
        merged,
        edges,
        vertices,
    })
}
