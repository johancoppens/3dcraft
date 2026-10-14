//! Delete face: faces of a solid taken out, and the body closed again
//! where they were by the faces round them.
//!
//! Two ways of closing it are built, and a set of faces that neither fits
//! is refused with the faces and the reason (docs/architecture.md §5.3 ›
//! *Face operations as built*). The faces given fall into groups that
//! hang together by edges; each group is closed on its own, one after the
//! other.
//!
//! **(a) Holes filled.** A group whose every edge to the rest of the body
//! lies in a hole of a face round it, each such hole bounded by the group
//! alone: a hole's wall (and its floor), a boss's wall and top, a pocket's
//! walls and floor. Taking the group out and those holes out of their
//! faces leaves a closed body: the faces round it go on across where the
//! holes were, on the surfaces they had. That is purely topological; what
//! is checked is that every such loop is a hole (its face lies outside
//! it: it turns against the face's outer loops, measured in the face's
//! own parameters, and does not go round the axis of a round face), that
//! the result passes the checker, and that it encloses a volume. A group
//! that is a whole shell with nothing round it is a cavity, filled.
//!
//! **(b) A strip collapsed.** One face that runs between two faces along
//! its two sides and ends against a face at each end (a chamfer or a
//! fillet along one edge: a loop of four edges), or that runs all the
//! way round between two faces (the round of a rod's rim: two loops of
//! one edge each). The face is taken out, its two sides become one edge
//! between the two faces beside it, and the two ends of each end edge one
//! vertex; then the faces beside it are continued until they meet
//! ([`crate::tweak::resurface_at`]: the merged vertices are found where
//! the surfaces round them meet, the edges ending in them are made again
//! on their faces' surfaces, and the result is held against what a body
//! is). Of the two ways to pair the four edges of a strip, the sides are
//! the two edges along which the face is tangent to its neighbours when
//! there are such (a fillet), else the pair that works, the longer first.
//! The face at an end may not be tangent to a face beside the strip:
//! that is another blend going on there (a chain, or a corner where
//! blends meet), whose own face would have to be continued, which is not
//! built.
//!
//! **Never a wrong solid.** Every refusal names the faces (and the edge)
//! in the numbers of the body given. What a local check cannot see, a
//! face continued until it passes through a part of the body far away, is
//! left to the caller's check on the whole body, as for the other face
//! operations.

use crate::KernelError;
use crate::offset_face::refuse;
use crate::tweak::{self, TweakError, edge_faces, loop_points, unwrapped};
use craft3d_geom::Surface;
use craft3d_math::{Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId, FaceId, LoopId, VertexId};
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::PI;

const OPERATION: &str = "delete face";

/// Most faces one deletion takes.
pub const MAX_FACES: usize = 4096;

pub(crate) const NOT_HEALABLE: &str = "the faces round it cannot close the body: it is neither bounded only by whole holes of the faces round it (a hole, a boss, a pocket) nor a strip between two faces that can be continued until they meet (a chamfer or a fillet along one edge)";
const PART_OF_LOOP: &str = "a face round it shares only part of one of its loops with the faces deleted: the body cannot be closed by filling holes, and faces that meet are collapsed only one at a time";
const NOT_A_HOLE: &str = "a loop the faces deleted leave in a face round them is not a hole of that face (it is its outline, or goes round its axis): the face would have no boundary there";
const CANNOT_TELL: &str = "whether a loop the faces deleted leave in a face round them is a hole of it cannot be told (a face with a pole inside the loop)";
const NO_VOLUME: &str = "what is left encloses no volume: the faces are the outside of the body";
const TANGENT_END: &str = "the face at an end of the strip is tangent to it or to a face beside it: another blend goes on there (a chain, or a corner where blends meet), which would have to be continued too; that is not built";
const NO_COLLAPSE: &str = "the faces beside it, continued, do not meet where it was in a line or a circle (they are parallel, or meet in another curve): it cannot be collapsed into an edge between them";
const SIDES: &str = "the strip's sides do not run between two different faces that stay (a seam, or a face deleted too)";
const MERGED_FACE: &str = "the faces beside the strip already meet along another edge at its end, or an end face is one of them: collapsing it is not built";

/// A body with faces deleted and where each of its entities came from:
/// `faces[i]` is the face of the body given that `FaceId(i)` of `body`
/// is, and so on; `None` for an edge or a vertex the deletion made (the
/// edge a strip collapsed into, the vertex its two ends became).
#[derive(Clone, Debug, PartialEq)]
pub struct Deleted {
    pub body: Body,
    pub faces: Vec<FaceId>,
    pub edges: Vec<Option<EdgeId>>,
    pub vertices: Vec<Option<VertexId>>,
}

fn index_u32(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}

/// The body being worked on, with where its entities came from in the
/// body given.
struct Work {
    body: Body,
    faces: Vec<FaceId>,
    edges: Vec<Option<EdgeId>>,
    vertices: Vec<Option<VertexId>>,
}

impl Work {
    fn of(body: &Body) -> Self {
        Work {
            faces: (0..body.faces.len())
                .map(|i| FaceId(index_u32(i)))
                .collect(),
            edges: (0..body.edges.len())
                .map(|i| Some(EdgeId(index_u32(i))))
                .collect(),
            vertices: (0..body.vertices.len())
                .map(|i| Some(VertexId(index_u32(i))))
                .collect(),
            body: body.clone(),
        }
    }

    /// The face of the body given that `face` is.
    fn original_face(&self, face: FaceId) -> FaceId {
        self.faces.get(face.0 as usize).copied().unwrap_or(face)
    }

    /// The face of this body that is `original` of the body given.
    fn current_face(&self, original: FaceId) -> Option<FaceId> {
        self.faces
            .iter()
            .position(|face| *face == original)
            .map(|i| FaceId(index_u32(i)))
    }

    /// A refusal in the numbers of the body given.
    fn refused(&self, faces: &[FaceId], edge: Option<EdgeId>, reason: &'static str) -> KernelError {
        let mut named: Vec<FaceId> = faces.iter().map(|f| self.original_face(*f)).collect();
        named.dedup();
        let edge = edge.and_then(|e| self.edges.get(e.0 as usize).copied().flatten());
        refuse(OPERATION, named, edge, reason)
    }

    fn tweak_error(&self, error: TweakError) -> KernelError {
        match error {
            TweakError::Kernel(error) => error,
            TweakError::Unsupported {
                faces,
                edge,
                reason,
            }
            | TweakError::Vanishes {
                faces,
                edge,
                reason,
            } => {
                let reason = if reason == tweak::NO_MEETING || reason == tweak::OFF_SURFACE {
                    NO_COLLAPSE
                } else {
                    reason
                };
                self.refused(&faces, edge, reason)
            }
        }
    }
}

/// What a change of topology does to a body.
#[derive(Default)]
struct Change {
    /// Faces taken out.
    gone: BTreeSet<FaceId>,
    /// Loops given new coedges; `None` takes the loop out of its face.
    loops: BTreeMap<LoopId, Option<Vec<Coedge>>>,
    /// Vertices that become another one.
    merged: BTreeMap<VertexId, VertexId>,
    /// Edges that are new where they lie: they come from nothing.
    renewed: BTreeSet<EdgeId>,
}

/// `work` changed as `change` says, with every edge and vertex nothing
/// uses any more left out and the rest numbered again in their order;
/// with what each vertex that stays is now.
fn rebuilt(work: &Work, change: &Change) -> (Work, BTreeMap<VertexId, VertexId>) {
    let body = &work.body;
    let merged = |v: VertexId| change.merged.get(&v).copied().unwrap_or(v);
    // The faces that stay, with their loops.
    let mut faces: Vec<(FaceId, Vec<Vec<Coedge>>)> = Vec::new();
    for (index, face) in body.faces.iter().enumerate() {
        let id = FaceId(index_u32(index));
        if change.gone.contains(&id) {
            continue;
        }
        let mut loops = Vec::new();
        for loop_ in &face.loops {
            match change.loops.get(loop_) {
                Some(None) => {}
                Some(Some(coedges)) => loops.push(coedges.clone()),
                None => loops.push(
                    body.loop_(*loop_)
                        .map(|l| l.coedges.clone())
                        .unwrap_or_default(),
                ),
            }
        }
        faces.push((id, loops));
    }
    let used_edges: BTreeSet<EdgeId> = faces
        .iter()
        .flat_map(|(_, loops)| loops.iter().flatten().map(|c| c.edge))
        .collect();
    let used_vertices: BTreeSet<VertexId> = used_edges
        .iter()
        .filter_map(|e| body.edge(*e))
        .flat_map(|e| [merged(e.start), merged(e.end)])
        .collect();
    let targets: BTreeSet<VertexId> = change.merged.values().copied().collect();

    let mut made = Body::new(body.kind);
    let mut vertex_map: BTreeMap<VertexId, VertexId> = BTreeMap::new();
    let mut vertices = Vec::new();
    for vertex in &used_vertices {
        let Some(point) = body.vertex(*vertex).map(|v| v.point) else {
            continue;
        };
        vertex_map.insert(*vertex, made.add_vertex(point));
        vertices.push(if targets.contains(vertex) {
            None
        } else {
            work.vertices.get(vertex.0 as usize).copied().flatten()
        });
    }
    let mut edge_map: BTreeMap<EdgeId, EdgeId> = BTreeMap::new();
    let mut edges = Vec::new();
    for edge_id in &used_edges {
        let Some(edge) = body.edge(*edge_id) else {
            continue;
        };
        let (Some(start), Some(end)) = (
            vertex_map.get(&merged(edge.start)),
            vertex_map.get(&merged(edge.end)),
        ) else {
            continue;
        };
        edge_map.insert(
            *edge_id,
            made.add_edge(edge.curve.clone(), edge.range, *start, *end),
        );
        edges.push(if change.renewed.contains(edge_id) {
            None
        } else {
            work.edges.get(edge_id.0 as usize).copied().flatten()
        });
    }
    let mut face_map: BTreeMap<FaceId, FaceId> = BTreeMap::new();
    let mut origins = Vec::new();
    for (id, loops) in faces {
        let Some(face) = body.face(id) else {
            continue;
        };
        let loops: Vec<LoopId> = loops
            .into_iter()
            .map(|coedges| {
                made.add_loop(
                    coedges
                        .into_iter()
                        .filter_map(|c| {
                            edge_map.get(&c.edge).map(|edge| Coedge {
                                edge: *edge,
                                reversed: c.reversed,
                            })
                        })
                        .collect(),
                )
            })
            .collect();
        face_map.insert(
            id,
            made.add_face(face.surface.clone(), face.reversed, loops),
        );
        origins.push(work.original_face(id));
    }
    for shell in &body.shells {
        let faces: Vec<FaceId> = shell
            .faces
            .iter()
            .filter_map(|f| face_map.get(f).copied())
            .collect();
        if !faces.is_empty() {
            made.add_shell(faces);
        }
    }
    (
        Work {
            body: made,
            faces: origins,
            edges,
            vertices,
        },
        vertex_map,
    )
}

/// The unit outward normal of `face` at the point of it nearest `at`.
fn outward(body: &Body, face: FaceId, at: Vec3) -> Option<Vec3> {
    let face = body.face(face)?;
    let (u, v) = face.surface.project(at).ok()?;
    let normal = face.surface.normal(u, v)?;
    Some(if face.reversed { -normal } else { normal })
}

/// Normals nearer to each other than this sine are one direction.
const TANGENT: f64 = 1e-6;

/// Whether faces `a` and `b` are tangent at `at` (normals along one
/// line). `None` where a face has no normal there.
fn tangent_at(body: &Body, a: FaceId, b: FaceId, at: Vec3) -> Option<bool> {
    let (na, nb) = (outward(body, a, at)?, outward(body, b, at)?);
    Some(na.cross(nb).length() <= TANGENT)
}

/// Whether the loop `loop_` of `face` is a hole of it: the face lies
/// outside it. In the face's own parameters (its normal is ∂u × ∂v) an
/// outer loop turns counter-clockwise seen from outside and a hole
/// clockwise; a loop that goes round a periodic direction (the axis of a
/// cylinder) is neither. `None` when it cannot be told.
fn is_hole(body: &Body, face: FaceId, loop_: LoopId) -> Option<bool> {
    let face = body.face(face)?;
    let surface = &face.surface;
    let points = loop_points(body, loop_);
    if points.len() < 3 {
        return None;
    }
    let mut chart: Vec<(f64, f64)> = Vec::with_capacity(points.len());
    for point in &points {
        let (mut u, mut v) = surface.project(*point).ok()?;
        if let Some(&(pu, pv)) = chart.last() {
            if surface.period_u().is_some() {
                u = unwrapped(u, pu);
            }
            if surface.period_v().is_some() {
                v = unwrapped(v, pv);
            }
        }
        if !(u.is_finite() && v.is_finite()) {
            return None;
        }
        chart.push((u, v));
    }
    let (first, last) = (chart.first()?, chart.last()?);
    // Round a periodic direction: the loop does not come back where it
    // started in the chart.
    if (last.0 - first.0).abs() > PI || (last.1 - first.1).abs() > PI {
        return Some(false);
    }
    // A pole inside the loop (a sphere) makes the chart useless.
    if matches!(surface, Surface::Sphere(_)) {
        let near_pole = chart.iter().any(|(_, v)| (v.abs() - 0.5 * PI).abs() < 1e-9);
        if near_pole {
            return None;
        }
    }
    let mut twice = 0.0;
    for (index, (u0, v0)) in chart.iter().enumerate() {
        let Some((u1, v1)) = chart.get((index + 1) % chart.len()) else {
            continue;
        };
        twice += u0 * v1 - u1 * v0;
    }
    if !twice.is_finite() || twice == 0.0 {
        return None;
    }
    let turning = if face.reversed { -twice } else { twice };
    Some(turning < 0.0)
}

/// The groups of `faces` that hang together by edges, each ascending, in
/// the order of their first face.
fn groups(body: &Body, faces: &BTreeSet<FaceId>) -> Vec<BTreeSet<FaceId>> {
    let uses = edge_faces(body);
    let mut left = faces.clone();
    let mut found = Vec::new();
    while let Some(first) = left.pop_first() {
        let mut group = BTreeSet::from([first]);
        let mut todo = vec![first];
        while let Some(face) = todo.pop() {
            for coedge in body
                .face(face)
                .into_iter()
                .flat_map(|f| &f.loops)
                .filter_map(|l| body.loop_(*l))
                .flat_map(|l| &l.coedges)
            {
                for other in uses.get(coedge.edge.0 as usize).into_iter().flatten() {
                    if left.remove(other) {
                        group.insert(*other);
                        todo.push(*other);
                    }
                }
            }
        }
        found.push(group);
    }
    found
}

/// How a group can be closed, or why not.
enum Plan {
    Holes(Change),
    NotHoles(&'static str, Vec<FaceId>),
}

/// (a): the loops of the faces round `group` it is bounded by, when they
/// are all holes bounded by it alone.
fn plan_holes(body: &Body, group: &BTreeSet<FaceId>) -> Plan {
    let uses = edge_faces(body);
    let mut change = Change {
        gone: group.clone(),
        ..Change::default()
    };
    for (index, face) in body.faces.iter().enumerate() {
        let id = FaceId(index_u32(index));
        if group.contains(&id) {
            continue;
        }
        for loop_ in &face.loops {
            let Some(coedges) = body.loop_(*loop_).map(|l| &l.coedges) else {
                continue;
            };
            let touching = coedges
                .iter()
                .filter(|c| {
                    uses.get(c.edge.0 as usize)
                        .is_some_and(|users| users.iter().any(|f| group.contains(f)))
                })
                .count();
            if touching == 0 {
                continue;
            }
            if touching != coedges.len() {
                return Plan::NotHoles(PART_OF_LOOP, vec![id]);
            }
            match is_hole(body, id, *loop_) {
                Some(true) => {
                    change.loops.insert(*loop_, None);
                }
                Some(false) => return Plan::NotHoles(NOT_A_HOLE, vec![id]),
                None => return Plan::NotHoles(CANNOT_TELL, vec![id]),
            }
        }
    }
    Plan::Holes(change)
}

fn volume_of(body: &Body) -> Option<f64> {
    craft3d_topo::mass_properties(body)
        .ok()
        .map(|m| m.volume)
        .filter(|v| v.is_finite())
}

/// The checks every result of a step gets.
fn held(work: &Work, faces: &[FaceId], tolerance: &Tolerance) -> Result<(), KernelError> {
    let defects = craft3d_topo::check(&work.body, tolerance);
    if !defects.is_empty() {
        return Err(KernelError::InvalidBody {
            operation: OPERATION,
            defects: defects.into_iter().take(crate::MAX_REPORTED).collect(),
        });
    }
    match volume_of(&work.body) {
        Some(volume) if volume > 0.0 => Ok(()),
        _ => Err(refuse(OPERATION, faces.to_vec(), None, NO_VOLUME)),
    }
}

/// One way to collapse a strip: its sides (the coedges of the strip's
/// loops along the faces that stay beside it) and its ends.
struct Pairing {
    sides: [Coedge; 2],
    ends: Option<[Coedge; 2]>,
}

/// The one face other than `face` that uses `edge`.
fn other_face(uses: &[Vec<FaceId>], edge: EdgeId, face: FaceId) -> Option<FaceId> {
    let users = uses.get(edge.0 as usize)?;
    match users.as_slice() {
        [a, b] if *a == face && *b != face => Some(*b),
        [a, b] if *b == face && *a != face => Some(*a),
        _ => None,
    }
}

fn length_of(body: &Body, edge: EdgeId) -> f64 {
    let Some(edge) = body.edge(edge) else {
        return 0.0;
    };
    let points: Vec<Vec3> = (0..=8)
        .map(|i| {
            let s = f64::from(i) / 8.0;
            edge.curve
                .eval(edge.range.0 + (edge.range.1 - edge.range.0) * s)
        })
        .collect();
    points
        .windows(2)
        .map(|w| match w {
            [a, b] => a.distance(*b),
            _ => 0.0,
        })
        .sum()
}

fn middle_of(body: &Body, edge: EdgeId) -> Option<Vec3> {
    let edge = body.edge(edge)?;
    Some(edge.curve.eval(0.5 * (edge.range.0 + edge.range.1)))
}

/// The ways to pair the edges of `face` as a strip, in the order to try
/// them; empty when the face is no strip.
fn pairings(body: &Body, face: FaceId) -> Vec<Pairing> {
    let Some(f) = body.face(face) else {
        return Vec::new();
    };
    let loops: Vec<&[Coedge]> = f
        .loops
        .iter()
        .filter_map(|l| body.loop_(*l))
        .map(|l| l.coedges.as_slice())
        .collect();
    match loops.as_slice() {
        [[a], [b]] => vec![Pairing {
            sides: [*a, *b],
            ends: None,
        }],
        [[c0, c1, c2, c3]] => {
            let uses = edge_faces(body);
            let tangent = |c: &Coedge| {
                let (Some(other), Some(at)) =
                    (other_face(&uses, c.edge, face), middle_of(body, c.edge))
                else {
                    return false;
                };
                tangent_at(body, face, other, at) == Some(true)
            };
            let first = Pairing {
                sides: [*c0, *c2],
                ends: Some([*c1, *c3]),
            };
            let second = Pairing {
                sides: [*c1, *c3],
                ends: Some([*c2, *c0]),
            };
            let along = |p: &Pairing| tangent(&p.sides[0]) && tangent(&p.sides[1]);
            match (along(&first), along(&second)) {
                (true, false) => vec![first],
                (false, true) => vec![second],
                _ => {
                    let long = |p: &Pairing| {
                        length_of(body, p.sides[0].edge) + length_of(body, p.sides[1].edge)
                    };
                    if long(&second) > long(&first) {
                        vec![second, first]
                    } else {
                        vec![first, second]
                    }
                }
            }
        }
        _ => Vec::new(),
    }
}

/// The vertex a coedge runs from and the one it runs to, in its loop's
/// direction.
fn ends_of(body: &Body, coedge: Coedge) -> Option<(VertexId, VertexId)> {
    body.coedge_ends(coedge)
}

/// The loop of `face` with the coedge of `edge` in it, and where.
fn find_use(body: &Body, face: FaceId, edge: EdgeId) -> Option<(LoopId, usize)> {
    body.face(face)?.loops.iter().find_map(|l| {
        body.loop_(*l)?
            .coedges
            .iter()
            .position(|c| c.edge == edge)
            .map(|at| (*l, at))
    })
}

/// (b): `face` collapsed as `pairing` says, the faces beside it
/// continued until they meet.
fn collapse(
    work: &Work,
    face: FaceId,
    pairing: &Pairing,
    tolerance: &Tolerance,
) -> Result<Work, KernelError> {
    let body = &work.body;
    let uses = edge_faces(body);
    let [s1, s2] = pairing.sides;
    let (Some(n1), Some(n2)) = (
        other_face(&uses, s1.edge, face),
        other_face(&uses, s2.edge, face),
    ) else {
        return Err(work.refused(&[face], None, SIDES));
    };
    if n1 == n2 {
        return Err(work.refused(&[face, n1], None, SIDES));
    }
    let mut change = Change {
        gone: BTreeSet::from([face]),
        ..Change::default()
    };
    // The side along n2 becomes the side along n1, used the other way.
    let (Some((l1, at1)), Some((l2, at2))) =
        (find_use(body, n1, s1.edge), find_use(body, n2, s2.edge))
    else {
        return Err(work.refused(&[face], None, SIDES));
    };
    let Some(r1) = body
        .loop_(l1)
        .and_then(|l| l.coedges.get(at1))
        .map(|c| c.reversed)
    else {
        return Err(work.refused(&[face], None, SIDES));
    };
    let mut beside = body
        .loop_(l2)
        .map(|l| l.coedges.clone())
        .unwrap_or_default();
    if let Some(slot) = beside.get_mut(at2) {
        *slot = Coedge {
            edge: s1.edge,
            reversed: !r1,
        };
    }
    change.loops.insert(l2, Some(beside));
    change.renewed.insert(s1.edge);
    let mut also: Vec<VertexId> = Vec::new();
    let mut end_faces: Vec<(FaceId, VertexId, VertexId)> = Vec::new();
    match pairing.ends {
        None => {
            // A ring: one vertex on each side.
            let (Some((a, _)), Some((b, _))) = (ends_of(body, s1), ends_of(body, s2)) else {
                return Err(work.refused(&[face], None, SIDES));
            };
            change.merged.insert(b, a);
            also.push(a);
        }
        Some(ends) => {
            for end in ends {
                let Some(e) = other_face(&uses, end.edge, face) else {
                    return Err(work.refused(&[face], Some(end.edge), SIDES));
                };
                if e == n1 || e == n2 {
                    return Err(work.refused(&[face, e], Some(end.edge), MERGED_FACE));
                }
                // In the strip's loop the end runs from the side before
                // it to the side after it; its two vertices become one.
                let Some((from, to)) = ends_of(body, end) else {
                    return Err(work.refused(&[face], Some(end.edge), SIDES));
                };
                if from == to {
                    return Err(work.refused(&[face], Some(end.edge), SIDES));
                }
                // Tangent to the strip along the end: a blend that goes
                // on there (the next of a chain, the ball in a corner).
                if let Some(at) = middle_of(body, end.edge)
                    && tangent_at(body, e, face, at) == Some(true)
                {
                    return Err(work.refused(&[face, e], Some(end.edge), TANGENT_END));
                }
                end_faces.push((e, from, to));
                change.merged.insert(to, from);
                also.push(from);
                let Some((loop_, _)) = find_use(body, e, end.edge) else {
                    return Err(work.refused(&[face], Some(end.edge), SIDES));
                };
                let mut coedges = change
                    .loops
                    .get(&loop_)
                    .cloned()
                    .flatten()
                    .or_else(|| body.loop_(loop_).map(|l| l.coedges.clone()))
                    .unwrap_or_default();
                // Taken out by edge, not by place: an end face may be
                // the face at both ends.
                let before = coedges.len();
                coedges.retain(|c| c.edge != end.edge);
                if coedges.len() + 1 != before || coedges.is_empty() {
                    return Err(work.refused(&[face, e], Some(end.edge), MERGED_FACE));
                }
                change.loops.insert(loop_, Some(coedges));
            }
        }
    }
    // An end face tangent to a face beside the strip is another blend
    // going on there.
    for (e, from, to) in &end_faces {
        for (side, at) in [(n1, *from), (n2, *to), (n1, *to), (n2, *from)] {
            let Some(point) = body.vertex(at).map(|v| v.point) else {
                continue;
            };
            // Only where the side face is at that vertex.
            let on_side = body
                .face(side)
                .and_then(|f| {
                    let (u, v) = f.surface.project(point).ok()?;
                    Some(f.surface.eval(u, v).distance(point) <= tolerance.linear)
                })
                .unwrap_or(false);
            if on_side && tangent_at(body, *e, side, point) == Some(true) {
                return Err(work.refused(&[face, *e, side], None, TANGENT_END));
            }
        }
    }
    let (changed, now) = rebuilt(work, &change);
    let mut found: BTreeSet<VertexId> = BTreeSet::new();
    for vertex in &also {
        let Some(vertex) = now.get(vertex) else {
            return Err(work.refused(&[face], None, SIDES));
        };
        found.insert(*vertex);
    }
    let made = tweak::resurface_at(
        &changed.body,
        &BTreeMap::new(),
        &found,
        OPERATION,
        tolerance,
    )
    .map_err(|error| changed.tweak_error(error))?;
    Ok(Work {
        body: made,
        ..changed
    })
}

/// `body` without the faces `faces`, closed again by the faces round
/// them (see the module documentation).
///
/// **Errors.** [`KernelError::FaceOp`] for faces that are not there,
/// given twice, or whose removal no face round them closes, with the
/// faces (and the edge) and the reason; [`KernelError::InvalidBody`] when
/// the body given, or (a bug, or input at the edge of tolerance) the
/// result, fails the checker.
pub fn delete_faces(
    body: &Body,
    faces: &[FaceId],
    tolerance: &Tolerance,
) -> Result<Deleted, KernelError> {
    let tol = tolerance.linear;
    if !(tol.is_finite() && tol > 0.0 && tolerance.angular.is_finite() && tolerance.angular > 0.0) {
        return Err(KernelError::InvalidSize {
            name: "tolerance",
            value: tol,
        });
    }
    if body.kind != BodyKind::Solid {
        return Err(refuse(
            OPERATION,
            Vec::new(),
            None,
            "only the faces of a solid can be deleted",
        ));
    }
    if faces.is_empty() {
        return Err(refuse(
            OPERATION,
            Vec::new(),
            None,
            "there is no face to delete",
        ));
    }
    if faces.len() > MAX_FACES {
        return Err(refuse(
            OPERATION,
            Vec::new(),
            None,
            "more faces than one operation takes",
        ));
    }
    let mut asked: BTreeSet<FaceId> = BTreeSet::new();
    for face in faces {
        if body.face(*face).is_none() {
            return Err(refuse(
                OPERATION,
                vec![*face],
                None,
                "there is no such face in the body",
            ));
        }
        if !asked.insert(*face) {
            return Err(refuse(
                OPERATION,
                vec![*face],
                None,
                "the face is given twice",
            ));
        }
    }
    if asked.len() == body.faces.len() {
        return Err(refuse(OPERATION, Vec::new(), None, NO_VOLUME));
    }
    let defects = craft3d_topo::check(body, tolerance);
    if !defects.is_empty() {
        return Err(KernelError::InvalidBody {
            operation: OPERATION,
            defects: defects.into_iter().take(crate::MAX_REPORTED).collect(),
        });
    }
    let mut work = Work::of(body);
    for group in groups(body, &asked) {
        // In the numbers of the body as it is now.
        let current: BTreeSet<FaceId> = group
            .iter()
            .filter_map(|face| work.current_face(*face))
            .collect();
        let named: Vec<FaceId> = group.iter().copied().collect();
        if current.len() != group.len() {
            return Err(refuse(OPERATION, named, None, NOT_HEALABLE));
        }
        let why = match plan_holes(&work.body, &current) {
            Plan::Holes(change) => {
                let (next, _) = rebuilt(&work, &change);
                held(&next, &named, tolerance)?;
                work = next;
                continue;
            }
            Plan::NotHoles(reason, faces) => (reason, faces),
        };
        let [face] = named.as_slice() else {
            // Several faces that meet: only the holes they leave can be
            // filled.
            return Err(work.refused(&why.1, None, why.0));
        };
        let Some(face) = work.current_face(*face) else {
            return Err(refuse(OPERATION, named, None, NOT_HEALABLE));
        };
        let tries = pairings(&work.body, face);
        if tries.is_empty() {
            return Err(refuse(OPERATION, named, None, NOT_HEALABLE));
        }
        let mut first_error = None;
        let mut done = None;
        for pairing in &tries {
            match collapse(&work, face, pairing, tolerance)
                .and_then(|next| held(&next, &named, tolerance).map(|()| next))
            {
                Ok(next) => {
                    done = Some(next);
                    break;
                }
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        match (done, first_error) {
            (Some(next), _) => work = next,
            (None, Some(error)) => return Err(error),
            (None, None) => return Err(refuse(OPERATION, named, None, NOT_HEALABLE)),
        }
    }
    Ok(Deleted {
        body: work.body,
        faces: work.faces,
        edges: work.edges,
        vertices: work.vertices,
    })
}
