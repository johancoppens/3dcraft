//! Blends along a chain of edges that go on smoothly into each other: the
//! cut edge of a casing that runs from a cone into the torus of a root
//! fillet and on along a flange, the rim of the cap of an extruded smooth
//! outline, the foot of a boss that runs over a fillet.
//!
//! **One spine, a face per edge.** The balls (or the chamfer's lines) of a
//! chain are one family: they are found edge after edge, each from its
//! neighbour, and every edge gets a blend face of its own, between the two
//! faces of that edge. Where two edges of the chain meet (a *joint*) the
//! two blend faces share a section as their common edge: the arc of one
//! ball (or one line), which both surfaces pass through exactly, so the
//! blend runs on from one face into the next without a gap and tangent
//! across it as the true rolling-ball surface is.
//!
//! **Joints.** Across a joint either both faces of the edges go on (a
//! vertex of only the two edges), or one face goes on and the other one
//! changes, into a face tangent to it along a smooth edge that leaves the
//! vertex (a cone that goes on in the torus of a fillet). In the second
//! case the shared section is not the one in the plane across the edge at
//! the vertex: it is the ball that touches the changing side exactly on
//! the smooth edge, where both of its faces are tangent to it (found along
//! that edge by bisection: the point of it whose ball, the radius in along
//! the common normal, is the radius from the face that goes on). The
//! planes the sections of each edge are found in turn smoothly from the
//! planes across the edge into the plane of the shared section over half
//! the edge (which planes pick the balls does not change the surface), so
//! the last section of one edge and the first of the next are that ball.
//! The smooth edge is shortened to where the blend's border reaches it;
//! the vertex moves to where the face that goes on is touched.
//!
//! **Ends.** An open chain ends at its two outer vertices as a single
//! edge does (a corner of three faces, cut where the blend leaves the
//! third face); a closed chain has none.
//!
//! **Refused** (with the reason): edges that meet without going on into
//! each other, three or more blended edges at a vertex, a joint with more
//! edges than the smooth one, an outside and an inside edge in one chain,
//! a chain with more than one size, and a chamfer whose side changes at a
//! joint.

// `!(x > 0.0)` is deliberate: unlike `x <= 0.0` it is also true for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use super::super::{
    BlendError, Reason, Size, Turn, Use, across, lost, refused, sine_at, vertex_edges,
};
use super::{
    Across, BEND, End, FOLLOW_DEPTH, Item, MAX_SAMPLES, Section, Shape, Side, Solver, Spine, Steps,
    Stop, beyond_by, border, cut_back, cut_curve, edge_length, end_at, end_cut, first_section,
    insert_cut, march_out, outward_from, refine, refuse, scale_of, tangent_or_refused,
};
use crate::chamfer::ChamferSize;
use craft3d_geom::{Curve3, NurbsCurve, NurbsSurface, Surface};
use craft3d_math::{Tolerance, Vec3};
use craft3d_topo::{Body, Coedge, Edge, EdgeId, FaceId, Loop, LoopId, Vertex, VertexId};
use std::collections::BTreeMap;

mod meet;

/// Blends the chains `which` of `chains`, which end in the `meets`, in
/// place (see [`meet`]).
pub(super) fn build_meets(
    body: &mut Body,
    chains: &[Chain],
    which: &[usize],
    meets: &[Meet],
    items: &[Item],
    tolerance: &Tolerance,
) -> Result<Vec<(FaceId, usize)>, BlendError> {
    meet::build_with(body, chains, which, meets, items, tolerance)
}

/// One edge of a chain, followed in the chain's direction.
#[derive(Clone, Copy, Debug)]
pub(super) struct Piece {
    /// The index of its request.
    pub item: usize,
    pub edge: EdgeId,
    /// The faces on its two sides; the face that goes on across a joint
    /// keeps its side.
    pub faces: [FaceId; 2],
    /// Whether the chain runs along the edge's curve.
    pub forward: bool,
    pub shape: Shape,
}

/// Where the face on one side changes at a joint.
#[derive(Clone, Copy, Debug)]
pub(super) struct Switch {
    /// The side that changes.
    pub side: usize,
    /// The smooth edge between the face before and the face after.
    pub smooth: EdgeId,
}

/// Where two edges of a chain meet.
#[derive(Clone, Copy, Debug)]
pub(super) struct Joint {
    pub vertex: VertexId,
    pub switch: Option<Switch>,
}

/// Edges that are blended as one: `joints[i]` lies between `pieces[i]`
/// and the piece after it (the first, for the last joint of a closed
/// chain).
#[derive(Clone, Debug)]
pub(super) struct Chain {
    pub pieces: Vec<Piece>,
    pub joints: Vec<Joint>,
    pub closed: bool,
    /// For an open chain, the meet (an index into the meets of
    /// [`group`]) at its start and at its end, where there is one.
    pub meets: [Option<usize>; 2],
}

/// Two blended edges that end in one vertex without going on into each
/// other, the vertex's third edge left sharp (the tip of a blade: the
/// rims of its tip face meet at its leading edge). See [`meet`].
#[derive(Clone, Copy, Debug)]
pub(super) struct Meet {
    pub vertex: VertexId,
    /// The two requests.
    pub items: [usize; 2],
    /// The face both edges lie on.
    pub shared: FaceId,
    /// The vertex's third edge, between the other faces of the two.
    pub third: EdgeId,
}

/// The shape of a request's blend, with a chamfer's distances in the
/// order of the faces given (`swapped`: the other way round).
fn shape_of(size: Size, convex: bool, swapped: bool, edge: EdgeId) -> Result<Shape, BlendError> {
    match size {
        Size::Radius(radius) => Ok(Shape::Ball { radius, convex }),
        Size::Chamfer(ChamferSize::Distances { first, second }) => Ok(Shape::Cut {
            distances: if swapped {
                [second, first]
            } else {
                [first, second]
            },
        }),
        Size::Chamfer(ChamferSize::DistanceAngle { .. }) => Err(refused(edge, Reason::FreeAngle)),
    }
}

fn same_shape(a: Shape, b: Shape) -> bool {
    let close = |x: f64, y: f64| (x - y).abs() <= 1e-12 * x.abs().max(y.abs());
    match (a, b) {
        (
            Shape::Ball {
                radius: r1,
                convex: c1,
            },
            Shape::Ball {
                radius: r2,
                convex: c2,
            },
        ) => c1 == c2 && close(r1, r2),
        (
            Shape::Cut {
                distances: [a1, a2],
            },
            Shape::Cut {
                distances: [b1, b2],
            },
        ) => close(a1, b1) && close(a2, b2),
        _ => false,
    }
}

/// The unit direction of the chain at the start (`at_end` false) or the
/// end of a piece's edge.
fn direction(data: &Edge, forward: bool, at_end: bool) -> Option<Vec3> {
    let t = if forward == at_end {
        data.range.1
    } else {
        data.range.0
    };
    let (_, d) = data.curve.derivatives(t);
    let d = d.normalized()?;
    Some(if forward { d } else { -d })
}

/// The vertex a piece starts (`at_end` false) or ends at.
fn vertex_of(data: &Edge, forward: bool, at_end: bool) -> VertexId {
    if forward == at_end {
        data.end
    } else {
        data.start
    }
}

/// The faces of an edge, from its uses.
fn faces_of(uses: &[Vec<Use>], edge: EdgeId) -> Option<[FaceId; 2]> {
    let list = uses.get(edge.0 as usize)?;
    let [a, b] = <[Use; 2]>::try_from(list.as_slice()).ok()?;
    (a.face != b.face).then_some([a.face, b.face])
}

/// What is needed to read the body around a joint.
struct Around<'a> {
    body: &'a Body,
    uses: &'a [Vec<Use>],
    at: &'a [Vec<EdgeId>],
    items: &'a [Item],
    tolerance: &'a Tolerance,
}

impl Around<'_> {
    /// The joint at `vertex` between `p` and the request `next`, followed
    /// `forward` or not; with the piece `next` becomes.
    fn joint(
        &self,
        p: &Piece,
        next: usize,
        forward: bool,
        vertex: VertexId,
    ) -> Result<(Joint, Piece), BlendError> {
        let body = self.body;
        let item = self.items.get(next).ok_or_else(lost)?;
        let edge = item.request.edge;
        let meet = || refused(edge, Reason::FreeMeet);
        let joint_refused = || refused(edge, Reason::FreeJoint);
        let (pd, qd) = (
            body.edge(p.edge).ok_or_else(lost)?,
            body.edge(edge).ok_or_else(lost)?,
        );
        let (Some(out), Some(on)) = (
            direction(pd, p.forward, true),
            direction(qd, forward, false),
        ) else {
            return Err(meet());
        };
        let cosine = out.dot(on).clamp(-1.0, 1.0);
        if !(cosine > 1.0 - 1e-9 || cosine.acos() <= self.tolerance.angular) {
            return Err(meet());
        }
        let p_convex = self.items.get(p.item).ok_or_else(lost)?.convex;
        if p_convex != item.convex {
            return Err(joint_refused());
        }
        let others: Vec<EdgeId> = self
            .at
            .get(vertex.0 as usize)
            .ok_or_else(lost)?
            .iter()
            .copied()
            .filter(|e| *e != p.edge && *e != edge)
            .collect();
        let [qa, qb] = item.faces;
        // A seam of one of the faces through the vertex (where a cylinder
        // closes) is crossed as a smooth edge between the face and itself:
        // the blend's border on that face has to meet it.
        let seam = |e: EdgeId| -> Option<usize> {
            let list = self.uses.get(e.0 as usize)?;
            let [a, b] = <[Use; 2]>::try_from(list.as_slice()).ok()?;
            if a.face != b.face {
                return None;
            }
            p.faces.iter().position(|f| *f == a.face)
        };
        if let [one] = others.as_slice()
            && let Some(side) = seam(*one)
        {
            let data = body.edge(*one).ok_or_else(lost)?;
            if !(p.faces.contains(&qa) && p.faces.contains(&qb)) || data.start == data.end {
                return Err(joint_refused());
            }
            let shape = shape_of(item.request.size, item.convex, p.faces[0] != qa, edge)?;
            if !same_shape(shape, p.shape) {
                return Err(refused(edge, Reason::FreeChainSize));
            }
            if matches!(shape, Shape::Cut { .. }) {
                return Err(joint_refused());
            }
            return Ok((
                Joint {
                    vertex,
                    switch: Some(Switch { side, smooth: *one }),
                },
                Piece {
                    item: next,
                    edge,
                    faces: p.faces,
                    forward,
                    shape,
                },
            ));
        }
        let (faces, switch) = match others.as_slice() {
            [] => {
                if !(p.faces.contains(&qa) && p.faces.contains(&qb)) {
                    return Err(joint_refused());
                }
                (p.faces, None)
            }
            [smooth] => {
                let [pa, pb] = p.faces;
                // The side that goes on.
                let keep = match (item.faces.contains(&pa), item.faces.contains(&pb)) {
                    (true, false) => 0_usize,
                    (false, true) => 1,
                    _ => return Err(joint_refused()),
                };
                let kept = *p.faces.get(keep).ok_or_else(lost)?;
                let before = *p.faces.get(1 - keep).ok_or_else(lost)?;
                let after = if qa == kept { qb } else { qa };
                let Some([sa, sb]) = faces_of(self.uses, *smooth) else {
                    return Err(joint_refused());
                };
                let between = (sa == before && sb == after) || (sa == after && sb == before);
                let data = body.edge(*smooth).ok_or_else(lost)?;
                // Tangent at the joint: the faces' normals there agree as closely
                // as the blend's own tangency across a joint is measured (ten
                // times what its faces are refined to: the angular tolerance,
                // or the angle the linear tolerance makes over the size), at the
                // vertex rather than along the edge.
                let reach = match item.request.size {
                    Size::Radius(r) => r,
                    Size::Chamfer(ChamferSize::Distances { first, second }) => first.max(second),
                    Size::Chamfer(ChamferSize::DistanceAngle { distance, .. }) => distance,
                };
                let bound = 10.0
                    * self
                        .tolerance
                        .angular
                        .max(BEND)
                        .max(self.tolerance.linear / reach.abs().max(self.tolerance.linear));
                let at = body.vertex(vertex).ok_or_else(lost)?.point;
                let smooth_here =
                    sine_at(body, self.uses, *smooth, at)?.is_some_and(|s| s <= bound);
                if !between || !smooth_here || data.start == data.end {
                    return Err(joint_refused());
                }
                let mut faces = [kept; 2];
                if let Some(slot) = faces.get_mut(1 - keep) {
                    *slot = after;
                }
                (
                    faces,
                    Some(Switch {
                        side: 1 - keep,
                        smooth: *smooth,
                    }),
                )
            }
            _ => return Err(joint_refused()),
        };
        let swapped = faces[0] != qa;
        let shape = shape_of(item.request.size, item.convex, swapped, edge)?;
        if !same_shape(shape, p.shape) {
            return Err(refused(edge, Reason::FreeChainSize));
        }
        if switch.is_some() && matches!(shape, Shape::Cut { .. }) {
            return Err(joint_refused());
        }
        Ok((
            Joint { vertex, switch },
            Piece {
                item: next,
                edge,
                faces,
                forward,
                shape,
            },
        ))
    }
}

/// The requests grouped into chains of edges that go on smoothly into
/// each other (a request on its own is a chain of one), in the order of
/// their first requests, and the meets of chains that end in one vertex
/// at an angle. Blended edges that meet in any other way are refused.
pub(super) fn group(
    body: &Body,
    uses: &[Vec<Use>],
    items: &[Item],
    tolerance: &Tolerance,
) -> Result<(Vec<Chain>, Vec<Meet>), BlendError> {
    let edge_of = |i: usize| -> Result<&Edge, BlendError> {
        let item = items.get(i).ok_or_else(lost)?;
        body.edge(item.request.edge).ok_or_else(lost)
    };
    // Per vertex, the requests that end there.
    let mut meeting: BTreeMap<VertexId, Vec<usize>> = BTreeMap::new();
    for i in 0..items.len() {
        let data = edge_of(i)?;
        meeting.entry(data.start).or_default().push(i);
        if data.end != data.start {
            meeting.entry(data.end).or_default().push(i);
        }
    }
    for list in meeting.values() {
        if let [_, _, third, ..] = list.as_slice() {
            let edge = items.get(*third).ok_or_else(lost)?.request.edge;
            return Err(refused(edge, Reason::FreeMeet));
        }
    }
    let at = vertex_edges(body);
    // Two requests at a vertex go on into each other (a joint) or meet
    // there at an angle.
    let mut meets = Vec::new();
    let mut linked: BTreeMap<VertexId, [usize; 2]> = BTreeMap::new();
    for (vertex, list) in &meeting {
        let [a, b] = list.as_slice() else { continue };
        let leaving = |i: usize| -> Result<Option<Vec3>, BlendError> {
            let data = edge_of(i)?;
            Ok(direction(data, data.start == *vertex, false))
        };
        let tangent = match (leaving(*a)?, leaving(*b)?) {
            (Some(da), Some(db)) => {
                let cosine = (-da.dot(db)).clamp(-1.0, 1.0);
                cosine > 1.0 - 1e-9 || cosine.acos() <= tolerance.angular
            }
            _ => false,
        };
        if tangent {
            linked.insert(*vertex, [*a, *b]);
        } else {
            meets.push(meet(body, uses, &at, items, *vertex, [*a, *b])?);
        }
    }
    let partner = |i: usize, vertex: VertexId| -> Option<usize> {
        match linked.get(&vertex)? {
            [a, b] if *a == i => Some(*b),
            [a, b] if *b == i => Some(*a),
            _ => None,
        }
    };
    let around = Around {
        body,
        uses,
        at: &at,
        items,
        tolerance,
    };
    let mut done = vec![false; items.len()];
    let mut chains = Vec::new();
    for first in 0..items.len() {
        if done.get(first).copied().unwrap_or(true) {
            continue;
        }
        let data = edge_of(first)?;
        if data.start == data.end {
            // A ring: on its own (another request at its vertex would have
            // two partners there, or meet it).
            if meeting.get(&data.start).is_some_and(|l| l.len() > 1) {
                let edge = items.get(first).ok_or_else(lost)?.request.edge;
                return Err(refused(edge, Reason::FreeMeet));
            }
        }
        // Back to where the chain starts, or round to `first` again.
        let (mut current, mut forward) = (first, true);
        let mut closed = false;
        for _ in 0..=items.len() {
            let here = edge_of(current)?;
            if here.start == here.end {
                break;
            }
            let back = vertex_of(here, forward, false);
            let Some(before) = partner(current, back) else {
                break;
            };
            if before == first {
                closed = true;
                break;
            }
            let there = edge_of(before)?;
            forward = there.end == back;
            current = before;
        }
        if closed {
            (current, forward) = (first, true);
        }
        // Then along it.
        let item = items.get(current).ok_or_else(lost)?;
        let mut piece = Piece {
            item: current,
            edge: item.request.edge,
            faces: item.faces,
            forward,
            shape: shape_of(item.request.size, item.convex, false, item.request.edge)?,
        };
        let start = piece;
        let mut pieces = Vec::new();
        let mut joints = Vec::new();
        for _ in 0..=items.len() {
            if let Some(slot) = done.get_mut(piece.item) {
                *slot = true;
            }
            pieces.push(piece);
            let here = edge_of(piece.item)?;
            if here.start == here.end {
                break;
            }
            let exit = vertex_of(here, piece.forward, true);
            let Some(next) = partner(piece.item, exit) else {
                break;
            };
            let there = edge_of(next)?;
            let next_forward = there.start == exit;
            let (joint, next_piece) = around.joint(&piece, next, next_forward, exit)?;
            joints.push(joint);
            if closed && next == start.item {
                // The faces have to come round as they started.
                if next_piece.faces != start.faces || next_piece.forward != start.forward {
                    return Err(refused(start.edge, Reason::FreeJoint));
                }
                break;
            }
            if done.get(next).copied().unwrap_or(true) {
                return Err(lost());
            }
            piece = next_piece;
        }
        // The meets at the ends of an open chain.
        let mut ends = [None, None];
        if !closed {
            let (Some(head), Some(tail)) = (pieces.first(), pieces.last()) else {
                return Err(lost());
            };
            let at_vertex = [
                (
                    head.item,
                    vertex_of(edge_of(head.item)?, head.forward, false),
                ),
                (
                    tail.item,
                    vertex_of(edge_of(tail.item)?, tail.forward, true),
                ),
            ];
            for (slot, (item, vertex)) in ends.iter_mut().zip(at_vertex) {
                *slot = meets
                    .iter()
                    .position(|m: &Meet| m.vertex == vertex && m.items.contains(&item));
            }
        }
        chains.push(Chain {
            pieces,
            joints,
            closed,
            meets: ends,
        });
    }
    Ok((chains, meets))
}

/// The meet of the requests `pair` at `vertex`, or the refusal: the
/// vertex has three edges and three faces, the two edges share one face,
/// all three edges turn the same way (all convex or all concave) and the
/// two are filleted with one radius.
fn meet(
    body: &Body,
    uses: &[Vec<Use>],
    at: &[Vec<EdgeId>],
    items: &[Item],
    vertex: VertexId,
    pair: [usize; 2],
) -> Result<Meet, BlendError> {
    let [a, b] = pair;
    let (ia, ib) = (
        items.get(a).ok_or_else(lost)?,
        items.get(b).ok_or_else(lost)?,
    );
    let refuse_meet = || refused(ib.request.edge, Reason::FreeMeet);
    let edges = at.get(vertex.0 as usize).ok_or_else(lost)?;
    let [x, y, z] = edges.as_slice() else {
        return Err(refuse_meet());
    };
    let third = [*x, *y, *z]
        .into_iter()
        .find(|e| *e != ia.request.edge && *e != ib.request.edge)
        .ok_or_else(refuse_meet)?;
    let shared: Vec<FaceId> = ia
        .faces
        .iter()
        .copied()
        .filter(|f| ib.faces.contains(f))
        .collect();
    let [shared] = shared.as_slice() else {
        return Err(refuse_meet());
    };
    let other = |item: &Item| item.faces.iter().copied().find(|f| f != shared);
    let (Some(fa), Some(fb)) = (other(ia), other(ib)) else {
        return Err(refuse_meet());
    };
    let Some(third_faces) = faces_of(uses, third) else {
        return Err(refuse_meet());
    };
    let turn = across(body, uses, third)?.map(|t| t.turn);
    let same = matches!(
        (ia.convex, ib.convex, turn),
        (true, true, Some(Turn::Convex)) | (false, false, Some(Turn::Concave))
    );
    let radii = match (ia.request.size, ib.request.size) {
        (Size::Radius(p), Size::Radius(q)) => (p - q).abs() <= 1e-12 * p.abs().max(q.abs()),
        _ => false,
    };
    // Two edges of one kind meeting at a third edge of the other: the
    // balls that touch the two faces of the third edge's sides are inside
    // the body there and roll over it, a patch between the two fillets.
    let over = matches!(
        (ia.convex, ib.convex, turn),
        (true, true, Some(Turn::Concave)) | (false, false, Some(Turn::Convex))
    );
    if third_faces.contains(&fa) && third_faces.contains(&fb) && over {
        return Err(refused(ib.request.edge, Reason::FreeRollOver));
    }
    if !(third_faces.contains(&fa) && third_faces.contains(&fb) && same && radii) {
        return Err(refuse_meet());
    }
    Ok(Meet {
        vertex,
        items: pair,
        shared: *shared,
        third,
    })
}

/// A section's curve as an edge's: the rational quadratic arc of a ball
/// (or the chamfer's line), from the first face to the second.
fn section_curve(section: &Section) -> Option<NurbsCurve> {
    let net = section.net()?;
    let degree = net.len().checked_sub(1)?;
    let mut knots = vec![0.0; net.len()];
    knots.extend(std::iter::repeat_n(1.0, net.len()));
    let (ctrl, weights): (Vec<Vec3>, Vec<f64>) = net.into_iter().unzip();
    NurbsCurve::new(degree, knots, ctrl, weights).ok()
}

/// The ball at a joint where the face on `side` changes: it touches the
/// smooth edge between the face before and the face after, where both are
/// tangent to it, and is the radius from the face that goes on. Returns
/// the section with the parameters of the faces before (`sides`) and of
/// the faces after (`after`), the plane it lies in (its normal along the
/// chain, `along`), and the smooth edge's parameter where it is touched.
#[allow(clippy::too_many_arguments)]
fn switch_ball(
    sides: [Side<'_>; 2],
    after: [Side<'_>; 2],
    side: usize,
    smooth: &Edge,
    vertex: VertexId,
    shape: Shape,
    along: Vec3,
    scale: f64,
) -> Option<(Section, Section, Across, f64)> {
    let Shape::Ball { radius, convex } = shape else {
        return None;
    };
    let inward = if convex { -radius } else { radius };
    let changing = *sides.get(side)?;
    let kept = *sides.get(1 - side)?;
    let (from, to) = if smooth.start == vertex {
        smooth.range
    } else if smooth.end == vertex {
        (smooth.range.1, smooth.range.0)
    } else {
        return None;
    };
    // The centre of the ball touching the smooth edge at `t`, and how far
    // it is from being the radius from the face that goes on.
    let centre = |t: f64| -> Option<(Vec3, Vec3, f64)> {
        let point = smooth.curve.eval(t);
        let start = changing.surface.project(point).ok()?;
        let (uv, _) = changing.foot(point, start)?;
        let c = point + changing.normal(uv)? * inward;
        let near = kept.surface.project(c).ok()?;
        let (d, _) = kept.signed(c, near)?;
        Some((c, point, d - inward))
    };
    let count = 64;
    let mut lo = from;
    let (_, _, mut g_lo) = centre(lo)?;
    let mut hi = None;
    for k in 1..=count {
        let t = from + (to - from) * f64::from(k) / f64::from(count);
        let (_, _, g) = centre(t)?;
        if (g > 0.0) != (g_lo > 0.0) {
            hi = Some(t);
            break;
        }
        (lo, g_lo) = (t, g);
    }
    let mut hi = hi?;
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        let (_, _, g) = centre(mid)?;
        if (g > 0.0) == (g_lo > 0.0) {
            (lo, g_lo) = (mid, g);
        } else {
            hi = mid;
        }
        if (hi - lo).abs() <= 1e-15 * (1.0 + mid.abs()) {
            break;
        }
    }
    let t = 0.5 * (lo + hi);
    let (c, touched, _) = centre(t)?;
    let start = kept.surface.project(c).ok()?;
    let (kept_uv, kept_foot) = kept.foot(c, start)?;
    // The ball has to touch the face that goes on, on the right side.
    let offset = c - kept_foot;
    let n = kept.normal(kept_uv)?;
    if !((offset.length() - radius).abs() <= 1e-9 * scale
        && offset.cross(n).length() <= 1e-7 * radius
        && offset.dot(n) * inward > 0.0)
    {
        return None;
    }
    let params_on = |pair: [Side<'_>; 2]| -> Option<[(f64, f64); 2]> {
        let mut params = [(0.0, 0.0); 2];
        for (k, s) in pair.iter().enumerate() {
            let point = if k == side { touched } else { kept_foot };
            let start = s.surface.project(point).ok()?;
            let (uv, _) = s.foot(point, start)?;
            *params.get_mut(k)? = uv;
        }
        Some(params)
    };
    let mut feet = [kept_foot; 2];
    *feet.get_mut(side)? = touched;
    let before = Section {
        feet,
        params: params_on(sides)?,
        centre: Some(c),
    };
    let later = Section {
        feet,
        params: params_on(after)?,
        centre: Some(c),
    };
    let [f0, f1] = feet;
    let normal = (f0 - c).cross(f1 - c);
    if !(normal.length() > 1e-6 * radius * radius) {
        return None;
    }
    let normal = normal.normalized()?;
    let normal = if normal.dot(along) < 0.0 {
        -normal
    } else {
        normal
    };
    Some((before, later, Across { point: c, normal }, t))
}

/// Sections from `from` to `to`, evenly, no further apart than `step`;
/// the last one is at `to` exactly. `from` is not among them.
fn march(
    solver: &Solver<'_>,
    from: (f64, Section),
    to: f64,
    step: f64,
) -> Option<Vec<(f64, Section)>> {
    let span = to - from.0;
    let mut count = 1_usize;
    while span.abs() / count as f64 > step && count < MAX_SAMPLES {
        count += 1;
    }
    let mut solved = Vec::with_capacity(count);
    let (mut at, mut last) = from;
    for k in 1..=count {
        let s = if k == count {
            to
        } else {
            from.0 + span * (k as f64 / count as f64)
        };
        last = solver.follow(at, &last, s, FOLLOW_DEPTH)?;
        at = s;
        solved.push((s, last));
    }
    Some(solved)
}

/// What is worked out for one piece.
struct PieceMade {
    surface: NurbsSurface,
    /// Per border, the range of `s` it covers.
    spans: [(f64, f64); 2],
    outward: Vec3,
}

/// What is worked out for one joint: the shared section and, where a face
/// changes, the smooth edge's parameter where the blend touches it.
struct JointMade {
    section: Section,
    touched: Option<f64>,
}

/// What is worked out for an end of an open chain.
struct EndMade {
    end: End,
    cut: NurbsCurve,
    at: [f64; 2],
}

/// How an open chain ends at one of its ends: cut by the third face
/// there, or at a meet with another blend, which trims it later
/// ([`meet`]); `range` is where along the face that trim is looked for
/// (from a few radii before the vertex to the last ball, out of the far
/// face).
enum EndKind {
    Third(EndMade),
    Meet { range: (f64, f64) },
}

struct Computed {
    pieces: Vec<PieceMade>,
    joints: Vec<JointMade>,
    ends: Option<[EndKind; 2]>,
}

/// What a meet gives the end of a chain: the vertices where the borders
/// end (per side of the face) and the edges from the first to the second
/// (each with whether it runs that way against its curve).
#[derive(Clone, Debug)]
pub(super) struct MeetEnd {
    corners: [VertexId; 2],
    edges: Vec<(EdgeId, bool)>,
}

fn sides_of<'a>(body: &'a Body, faces: [FaceId; 2]) -> Result<[Side<'a>; 2], BlendError> {
    let side = |f: FaceId| -> Result<Side<'a>, BlendError> {
        let face = body.face(f).ok_or_else(lost)?;
        Ok(Side {
            surface: &face.surface,
            reversed: face.reversed,
        })
    };
    let [a, b] = faces;
    Ok([side(a)?, side(b)?])
}

/// The two sections are one ball (or one line), to well within the
/// tolerance.
fn agree(a: &Section, b: &Section, limit: f64) -> bool {
    let feet = a
        .feet
        .iter()
        .zip(&b.feet)
        .all(|(p, q)| p.distance(*q) <= limit);
    let centres = match (a.centre, b.centre) {
        (Some(p), Some(q)) => p.distance(q) <= limit,
        (None, None) => true,
        _ => false,
    };
    feet && centres
}

/// How far apart the arcs (or lines) of two sections are, the largest
/// distance at nine points of the same parameter.
fn apart(a: &Section, b: &Section) -> Option<f64> {
    let mut most = 0.0_f64;
    for k in 0..=8 {
        let v = f64::from(k) / 8.0;
        most = most.max(a.eval(v)?.distance(b.eval(v)?));
    }
    Some(most)
}

/// Works out the blend faces of `chain` on `body` (unchanged), with the
/// sections in planes across the edges (a chamfer's, whose lines depend
/// on those planes).
#[allow(clippy::too_many_lines)]
fn compute(
    body: &Body,
    chain: &Chain,
    items: &[Item],
    tolerance: &Tolerance,
) -> Result<Computed, BlendError> {
    let tol = tolerance.linear;
    let pieces = &chain.pieces;
    let n = pieces.len();
    let first = *pieces.first().ok_or_else(lost)?;
    let edge = first.edge;
    let unsolvable = || refuse(edge, None, Reason::FreeSurface);
    let shape = first.shape;
    let size = shape.size();
    let convex = items.get(first.item).ok_or_else(lost)?.convex;
    let datas: Vec<&Edge> = pieces
        .iter()
        .map(|p| body.edge(p.edge).ok_or_else(lost))
        .collect::<Result<_, _>>()?;
    let sides: Vec<[Side<'_>; 2]> = pieces
        .iter()
        .map(|p| sides_of(body, p.faces))
        .collect::<Result<_, _>>()?;
    let scale = datas
        .iter()
        .map(|d| scale_of(d, size, tol))
        .fold(tol, f64::max);
    let after = |j: usize| (j + 1) % n;
    // The planes of the joints, and the shared sections where a face
    // changes.
    let mut planes = Vec::with_capacity(chain.joints.len());
    let mut switched: Vec<Option<(Section, Section, f64)>> = Vec::with_capacity(chain.joints.len());
    for (j, joint) in chain.joints.iter().enumerate() {
        let (p, q) = (
            pieces.get(j).ok_or_else(lost)?,
            pieces.get(after(j)).ok_or_else(lost)?,
        );
        let (pd, qd) = (
            *datas.get(j).ok_or_else(lost)?,
            *datas.get(after(j)).ok_or_else(lost)?,
        );
        let (Some(out), Some(on)) = (
            direction(pd, p.forward, true),
            direction(qd, q.forward, false),
        ) else {
            return Err(unsolvable());
        };
        let along = (out + on).normalized().ok_or_else(unsolvable)?;
        match joint.switch {
            None => {
                let point = body.vertex(joint.vertex).ok_or_else(lost)?.point;
                planes.push(Across {
                    point,
                    normal: along,
                });
                switched.push(None);
            }
            Some(switch) => {
                let smooth = body.edge(switch.smooth).ok_or_else(lost)?;
                let (before, later, plane, t) = switch_ball(
                    *sides.get(j).ok_or_else(lost)?,
                    *sides.get(after(j)).ok_or_else(lost)?,
                    switch.side,
                    smooth,
                    joint.vertex,
                    shape,
                    along,
                    scale,
                )
                .ok_or_else(|| refused(q.edge, Reason::FreeJoint))?;
                planes.push(plane);
                switched.push(Some((before, later, t)));
            }
        }
    }
    let turn = 0.5;
    let solvers: Vec<Solver<'_>> = pieces
        .iter()
        .enumerate()
        .map(|(i, p)| -> Result<Solver<'_>, BlendError> {
            let data = *datas.get(i).ok_or_else(lost)?;
            let range = if p.forward {
                data.range
            } else {
                (data.range.1, data.range.0)
            };
            let before = if i > 0 {
                planes.get(i - 1).copied()
            } else if chain.closed {
                planes.last().copied()
            } else {
                None
            };
            let behind = if i + 1 < n || chain.closed {
                planes.get(i).copied()
            } else {
                None
            };
            Ok(Solver {
                spine: Spine {
                    curve: &data.curve,
                    range,
                    closed: false,
                    joints: [before, behind],
                    turn,
                },
                sides: *sides.get(i).ok_or_else(lost)?,
                shape: p.shape,
                scale,
            })
        })
        .collect::<Result<_, _>>()?;
    let lengths: Vec<f64> = datas.iter().map(|d| edge_length(d)).collect();
    if lengths.iter().any(|l| !(*l > 0.0)) {
        return Err(unsolvable());
    }
    let steps: Vec<Steps> = lengths.iter().map(|l| Steps::of(size, *l)).collect();
    // The ends of an open chain.
    let ends = if chain.closed {
        None
    } else {
        let last = *pieces.last().ok_or_else(lost)?;
        let (fd, ld) = (
            *datas.first().ok_or_else(lost)?,
            *datas.last().ok_or_else(lost)?,
        );
        Some([
            end_at(
                body,
                first.edge,
                first.faces,
                vertex_of(fd, first.forward, false),
                tolerance.angular.max(BEND).max(tolerance.linear / size),
            )?,
            end_at(
                body,
                last.edge,
                last.faces,
                vertex_of(ld, last.forward, true),
                tolerance.angular.max(BEND).max(tolerance.linear / size),
            )?,
        ])
    };
    let third_of = |end: &End| end.side(body);
    let stop = |stop: Stop, face: Option<FaceId>| match stop {
        Stop::Unsolvable => unsolvable(),
        Stop::TooFar => refuse(edge, face, Reason::FreeEnd),
    };
    let agreement = 1e-7 * scale;
    // March along the chain from the middle of the first piece.
    let solver0 = solvers.first().ok_or_else(lost)?;
    let into = items.get(first.item).ok_or_else(lost)?.into;
    let seed = first_section(solver0, 0.5, into, edge)?;
    let mut solved: Vec<Vec<(f64, Section)>> = vec![Vec::new(); n];
    let mut joint_sections: Vec<Section> = Vec::with_capacity(chain.joints.len());
    let mut reach = [0.0_f64; 2];
    let mut from = (0.5, seed);
    for i in 0..n {
        let solver = solvers.get(i).ok_or_else(lost)?;
        let step = steps.get(i).ok_or_else(lost)?;
        let list = solved.get_mut(i).ok_or_else(lost)?;
        if i == 0 {
            list.push(from);
        }
        if i + 1 == n && !chain.closed {
            let end = ends.as_ref().ok_or_else(lost)?.get(1).ok_or_else(lost)?;
            let (more, past) = march_out(solver, from, 1.0, third_of(end)?, end.steps(*step), tol)
                .map_err(|s| stop(s, Some(end.third)))?;
            list.extend(more);
            reach[1] = past;
            break;
        }
        let mut more = march(solver, from, 1.0, step.inside).ok_or_else(unsolvable)?;
        // The section at the joint: where a face changes, the ball found
        // on the smooth edge, which the march has to have come to.
        let (_, reached) = *more.last().ok_or_else(lost)?;
        let (mine, next) = match switched.get(i).ok_or_else(lost)? {
            Some((before, later, _)) => {
                if !agree(&reached, before, agreement) {
                    return Err(refused(
                        pieces.get(after(i)).ok_or_else(lost)?.edge,
                        Reason::FreeJoint,
                    ));
                }
                (*before, *later)
            }
            None => (reached, reached),
        };
        if let Some(slot) = more.last_mut() {
            slot.1 = mine;
        }
        list.extend(more);
        joint_sections.push(mine);
        from = (0.0, next);
        if i + 1 < n
            && let Some(next_list) = solved.get_mut(i + 1)
        {
            next_list.push(from);
        }
    }
    // The first piece back from its middle: to the joint the chain closes
    // in, or past its start.
    let solver = solvers.first().ok_or_else(lost)?;
    let step = *steps.first().ok_or_else(lost)?;
    let back = if chain.closed {
        let mut more = march(solver, (0.5, seed), 0.0, step.inside).ok_or_else(unsolvable)?;
        let closing = n.checked_sub(1).ok_or_else(lost)?;
        let shared = match switched.get(closing).ok_or_else(lost)? {
            Some((_, later, _)) => *later,
            None => *joint_sections.get(closing).ok_or_else(lost)?,
        };
        let (_, reached) = *more.last().ok_or_else(lost)?;
        if !agree(&reached, &shared, agreement) {
            return Err(refused(first.edge, Reason::FreeJoint));
        }
        if let Some(slot) = more.last_mut() {
            slot.1 = shared;
        }
        more
    } else {
        let end = ends.as_ref().ok_or_else(lost)?.first().ok_or_else(lost)?;
        let (more, past) = march_out(
            solver,
            (0.5, seed),
            0.0,
            third_of(end)?,
            end.steps(step),
            tol,
        )
        .map_err(|s| stop(s, Some(end.third)))?;
        reach[0] = past;
        more
    };
    solved.first_mut().ok_or_else(lost)?.extend(back);
    // Each piece's surface.
    let limit = 0.25 * tol;
    let bend = tolerance.angular.max(BEND).max(tol / size);
    let mut made = Vec::with_capacity(n);
    for (i, list) in solved.iter_mut().enumerate() {
        list.sort_by(|a, b| a.0.total_cmp(&b.0));
        let solver = solvers.get(i).ok_or_else(lost)?;
        let mut params: Vec<f64> = list.iter().map(|x| x.0).collect();
        let mut sections: Vec<Section> = list.iter().map(|x| x.1).collect();
        let piece_edge = pieces.get(i).ok_or_else(lost)?.edge;
        let surface = refine(solver, &mut params, &mut sections, limit, bend)
            .ok_or_else(|| refuse(piece_edge, None, Reason::FreeApproximation))?;
        let outward =
            outward_from(solver, &surface, &params, &sections, convex).ok_or_else(unsolvable)?;
        made.push(PieceMade {
            surface,
            spans: [(0.0, 1.0); 2],
            outward,
        });
    }
    // The cuts at the ends of an open chain, and how far the borders go.
    let ends = match ends {
        None => None,
        Some([start, finish]) => {
            let mut done = Vec::with_capacity(2);
            for (index, end) in [start, finish].into_iter().enumerate() {
                let piece = if index == 0 { 0 } else { n - 1 };
                let surface = &made.get(piece).ok_or_else(lost)?.surface;
                let relative = size / *lengths.get(piece).ok_or_else(lost)?;
                let past = *reach.get(index).ok_or_else(lost)?;
                let (cut, at) = end_cut(
                    surface,
                    third_of(&end)?,
                    index == 1,
                    past,
                    relative,
                    limit,
                    scale,
                )
                .ok_or_else(|| {
                    refuse(
                        pieces.get(piece).map_or(edge, |p| p.edge),
                        Some(end.third),
                        Reason::FreeEnd,
                    )
                })?;
                let spans = &mut made.get_mut(piece).ok_or_else(lost)?.spans;
                for (k, s) in at.iter().enumerate() {
                    let span = spans.get_mut(k).ok_or_else(lost)?;
                    if index == 0 {
                        span.0 = *s;
                    } else {
                        span.1 = *s;
                    }
                }
                done.push(EndKind::Third(EndMade { end, cut, at }));
            }
            let [a, b] = <[EndKind; 2]>::try_from(done).map_err(|_| lost())?;
            Some([a, b])
        }
    };
    for (piece, m) in pieces.iter().zip(&made) {
        for (k, (a, b)) in m.spans.iter().enumerate() {
            if !(b > a) {
                let face = *piece.faces.get(k).ok_or_else(lost)?;
                return Err(refuse(piece.edge, Some(face), Reason::FreeTooLarge));
            }
        }
    }
    let joints = joint_sections
        .into_iter()
        .zip(&switched)
        .map(|(section, switch)| JointMade {
            section,
            touched: switch.as_ref().map(|s| s.2),
        })
        .collect();
    Ok(Computed {
        pieces: made,
        joints,
        ends,
    })
}

/// Blends `chain` of `body` in place: see the module documentation.
/// Returns each piece's blend face with the index of its request.
pub(super) fn build(
    body: &mut Body,
    chain: &Chain,
    items: &[Item],
    tolerance: &Tolerance,
) -> Result<Vec<(FaceId, usize)>, BlendError> {
    let computed = prepare(body, chain, items, &[], tolerance)?;
    finish(body, chain, computed, [None, None], tolerance)
}

/// Works out the blend faces of `chain` on `body` (not changed yet): the
/// ends at meets are left for [`meet`] to trim.
fn prepare(
    body: &Body,
    chain: &Chain,
    items: &[Item],
    meets: &[Meet],
    tolerance: &Tolerance,
) -> Result<Computed, BlendError> {
    match chain.pieces.first().ok_or_else(lost)?.shape {
        Shape::Ball { .. } => along_spine(body, chain, items, meets, tolerance),
        Shape::Cut { .. } => compute(body, chain, items, tolerance),
    }
}

/// Puts the worked-out blend faces of `chain` into `body` (with what the
/// meets give its ends) and measures them.
fn finish(
    body: &mut Body,
    chain: &Chain,
    computed: Computed,
    meet_ends: [Option<MeetEnd>; 2],
    tolerance: &Tolerance,
) -> Result<Vec<(FaceId, usize)>, BlendError> {
    let made = splice_chain(body, chain, computed, meet_ends, tolerance)?;
    measure(body, chain, &made, tolerance)
}

/// Holds the blend faces `made` of `chain`, put into `body`, against what
/// they have to be: each tangent to its faces along its borders, which
/// stay on them, and to the next across a joint.
fn measure(
    body: &Body,
    chain: &Chain,
    made: &[(FaceId, EdgeId)],
    tolerance: &Tolerance,
) -> Result<Vec<(FaceId, usize)>, BlendError> {
    let shape = chain.pieces.first().ok_or_else(lost)?.shape;
    let mut faces = Vec::with_capacity(made.len());
    for (piece, (face, second_border)) in chain.pieces.iter().zip(made) {
        tangent_or_refused(
            body,
            piece.edge,
            piece.faces,
            *face,
            *second_border,
            shape,
            tolerance,
        )?;
        faces.push((*face, piece.item));
    }
    smooth_joints(body, chain, made, shape, tolerance)?;
    Ok(faces)
}

/// Two blend faces that meet at a joint are tangent across it, measured
/// along the shared section (to ten times what the approximation was
/// refined to).
fn smooth_joints(
    body: &Body,
    chain: &Chain,
    made: &[(FaceId, EdgeId)],
    shape: Shape,
    tolerance: &Tolerance,
) -> Result<(), BlendError> {
    let n = chain.pieces.len();
    let size = shape.size();
    let limit = 10.0 * tolerance.angular.max(BEND).max(tolerance.linear / size);
    for j in 0..chain.joints.len() {
        let (Some((a, _)), Some((b, _))) = (made.get(j), made.get((j + 1) % n)) else {
            return Err(lost());
        };
        let edge = chain.pieces.get((j + 1) % n).ok_or_else(lost)?.edge;
        let normal = |face: FaceId, u: f64, v: f64| -> Option<Vec3> {
            let f = body.face(face)?;
            let Surface::Nurbs(surface) = &f.surface else {
                return None;
            };
            let (_, du, dv) = surface.derivatives(u, v);
            let n = du.cross(dv).normalized()?;
            Some(if f.reversed { -n } else { n })
        };
        for k in 1..8 {
            let v = f64::from(k) / 8.0;
            let (Some(m), Some(q)) = (normal(*a, 1.0, v), normal(*b, 0.0, v)) else {
                return Err(refuse(edge, None, Reason::NotTangent));
            };
            if !(m.cross(q).length() <= limit && m.dot(q) > 0.0) {
                return Err(refuse(edge, None, Reason::NotTangent));
            }
        }
    }
    Ok(())
}

/// Puts the blend faces of a chain into the body: per piece the edge
/// becomes its first border and a second border is new; per joint the
/// vertex moves to where the first side is touched, a new vertex where
/// the second is, the shared section is a new edge between them and a
/// smooth edge where a face changes is shortened to it; the ends of an
/// open chain are cut as a single edge's are. Returns each piece's blend
/// face and second border.
#[allow(clippy::too_many_lines)]
fn splice_chain(
    body: &mut Body,
    chain: &Chain,
    computed: Computed,
    meet_ends: [Option<MeetEnd>; 2],
    tolerance: &Tolerance,
) -> Result<Vec<(FaceId, EdgeId)>, BlendError> {
    let tol = tolerance.linear;
    let pieces = &chain.pieces;
    let n = pieces.len();
    let first = *pieces.first().ok_or_else(lost)?;
    let too_large = |face: FaceId| refuse(first.edge, Some(face), Reason::FreeTooLarge);
    let Computed {
        pieces: made,
        joints,
        ends,
    } = computed;
    let borders: Vec<[NurbsCurve; 2]> = made
        .iter()
        .map(|m| -> Option<[NurbsCurve; 2]> {
            Some([border(&m.surface, false)?, border(&m.surface, true)?])
        })
        .collect::<Option<_>>()
        .ok_or_else(lost)?;
    let new_vertex = |body: &mut Body, point: Vec3| -> Result<VertexId, BlendError> {
        let id = VertexId(u32::try_from(body.vertices.len()).map_err(|_| lost())?);
        body.vertices.push(Vertex { point });
        Ok(id)
    };
    let new_edge = |body: &mut Body, edge: Edge| -> Result<EdgeId, BlendError> {
        let id = EdgeId(u32::try_from(body.edges.len()).map_err(|_| lost())?);
        body.edges.push(edge);
        Ok(id)
    };
    // The joints: the vertex to the first side's foot, a new one at the
    // second's, the smooth edge shortened.
    let mut joint_vertices = Vec::with_capacity(joints.len());
    for (joint, jm) in chain.joints.iter().zip(&joints) {
        let [w0, w1] = jm.section.feet;
        if let Some(v) = body.vertices.get_mut(joint.vertex.0 as usize) {
            v.point = w0;
        }
        let added = new_vertex(body, w1)?;
        if let (Some(switch), Some(t)) = (joint.switch, jm.touched) {
            let face = *first.faces.get(switch.side).ok_or_else(lost)?;
            let slot = body
                .edges
                .get_mut(switch.smooth.0 as usize)
                .ok_or_else(lost)?;
            let (lo, hi) = slot.range;
            let span = hi - lo;
            let target = if switch.side == 0 {
                joint.vertex
            } else {
                added
            };
            if slot.start == joint.vertex {
                if !(t > lo && t < hi - 1e-9 * span) {
                    return Err(too_large(face));
                }
                slot.range.0 = t;
                slot.start = target;
            } else if slot.end == joint.vertex {
                if !(t < hi && t > lo + 1e-9 * span) {
                    return Err(too_large(face));
                }
                slot.range.1 = t;
                slot.end = target;
            } else {
                return Err(lost());
            }
        }
        joint_vertices.push([joint.vertex, added]);
    }
    // The ends of an open chain.
    let mut end_vertices = Vec::with_capacity(2);
    if let Some(ends) = &ends {
        for (index, kind) in ends.iter().enumerate() {
            match kind {
                EndKind::Third(em) => {
                    let piece = if index == 0 { 0 } else { n - 1 };
                    let faces = pieces.get(piece).ok_or_else(lost)?.faces;
                    let [b1, b2] = borders.get(piece).ok_or_else(lost)?;
                    let [s1, s2] = em.at;
                    let added = cut_back(
                        body,
                        &em.end,
                        faces,
                        [b1.eval(s1), b2.eval(s2)],
                        tol,
                        &too_large,
                    )?;
                    end_vertices.push([em.end.vertex, added]);
                }
                EndKind::Meet { .. } => {
                    let given = meet_ends
                        .get(index)
                        .and_then(Option::as_ref)
                        .ok_or_else(lost)?;
                    end_vertices.push(given.corners);
                }
            }
        }
    }
    let start_of = |i: usize| -> Option<[VertexId; 2]> {
        if i == 0 && !chain.closed {
            end_vertices.first().copied()
        } else {
            joint_vertices.get((i + n - 1) % n).copied()
        }
    };
    let finish_of = |i: usize| -> Option<[VertexId; 2]> {
        if i + 1 == n && !chain.closed {
            end_vertices.get(1).copied()
        } else {
            joint_vertices.get(i).copied()
        }
    };
    // The borders.
    let mut second_borders = Vec::with_capacity(n);
    for (i, (piece, m)) in pieces.iter().zip(&made).enumerate() {
        let (Some([a0, a1]), Some([z0, z1])) = (start_of(i), finish_of(i)) else {
            return Err(lost());
        };
        let [b1, b2] = borders.get(i).ok_or_else(lost)?.clone();
        let [span1, span2] = m.spans;
        *body.edges.get_mut(piece.edge.0 as usize).ok_or_else(lost)? = Edge {
            curve: Curve3::Nurbs(b1),
            range: span1,
            start: a0,
            end: z0,
        };
        second_borders.push(new_edge(
            body,
            Edge {
                curve: Curve3::Nurbs(b2),
                range: span2,
                start: a1,
                end: z1,
            },
        )?);
    }
    // The shared sections and the cuts.
    let mut joint_edges = Vec::with_capacity(joints.len());
    for (jm, [v0, v1]) in joints.iter().zip(&joint_vertices) {
        let curve = section_curve(&jm.section).ok_or_else(lost)?;
        joint_edges.push(new_edge(
            body,
            Edge {
                curve: Curve3::Nurbs(curve),
                range: (0.0, 1.0),
                start: *v0,
                end: *v1,
            },
        )?);
    }
    // Per open end, the edges from the first side's corner to the
    // second's: the cut, or what the meet gives.
    let mut cut_edges: Vec<Vec<(EdgeId, bool)>> = Vec::with_capacity(2);
    if let Some(ends) = &ends {
        for (index, (kind, [v0, v1])) in ends.iter().zip(&end_vertices).enumerate() {
            match kind {
                EndKind::Third(em) => {
                    let id = new_edge(
                        body,
                        Edge {
                            curve: Curve3::Nurbs(em.cut.clone()),
                            range: (0.0, 1.0),
                            start: *v0,
                            end: *v1,
                        },
                    )?;
                    insert_cut(body, &em.end, id, *v1)?;
                    cut_edges.push(vec![(id, false)]);
                }
                EndKind::Meet { .. } => {
                    let given = meet_ends
                        .get(index)
                        .and_then(Option::as_ref)
                        .ok_or_else(lost)?;
                    cut_edges.push(given.edges.clone());
                }
            }
        }
    }
    // The faces beside the chain: the second side uses the second border
    // where it used the edge; both run them the other way round where the
    // chain goes against the edge's curve.
    let mut first_reversed = Vec::with_capacity(n);
    for (piece, second_border) in pieces.iter().zip(&second_borders) {
        let mut found = None;
        for (k, face) in piece.faces.iter().enumerate() {
            let loops = body.face(*face).ok_or_else(lost)?.loops.clone();
            let mut done = false;
            for l in loops {
                let ring = body.loops.get_mut(l.0 as usize).ok_or_else(lost)?;
                for coedge in &mut ring.coedges {
                    if coedge.edge != piece.edge {
                        continue;
                    }
                    if k == 1 {
                        coedge.edge = *second_border;
                    }
                    if !piece.forward {
                        coedge.reversed = !coedge.reversed;
                    }
                    if k == 0 {
                        found = Some(coedge.reversed);
                    }
                    done = true;
                }
            }
            if !done {
                return Err(lost());
            }
        }
        first_reversed.push(found.ok_or_else(lost)?);
    }
    // The blend faces.
    let [own, _] = first.faces;
    let shell = body
        .shells
        .iter()
        .position(|s| s.faces.contains(&own))
        .ok_or_else(lost)?;
    let mut faces = Vec::with_capacity(n);
    for (i, (piece, m)) in pieces.iter().zip(made).enumerate() {
        // From the first side's corner to the second's, at the start and
        // at the end of the face.
        let start_edges: Vec<(EdgeId, bool)> = if i == 0 && !chain.closed {
            cut_edges.first().cloned()
        } else {
            joint_edges.get((i + n - 1) % n).map(|e| vec![(*e, false)])
        }
        .ok_or_else(lost)?;
        let finish_edges: Vec<(EdgeId, bool)> = if i + 1 == n && !chain.closed {
            cut_edges.get(1).cloned()
        } else {
            joint_edges.get(i).map(|e| vec![(*e, false)])
        }
        .ok_or_else(lost)?;
        let second_border = *second_borders.get(i).ok_or_else(lost)?;
        let reversed_first = *first_reversed.get(i).ok_or_else(lost)?;
        let coedge = |edge: EdgeId, reversed: bool| Coedge { edge, reversed };
        let forwards = |list: &[(EdgeId, bool)]| -> Vec<Coedge> {
            list.iter().map(|(e, r)| coedge(*e, *r)).collect()
        };
        let backwards = |list: &[(EdgeId, bool)]| -> Vec<Coedge> {
            list.iter().rev().map(|(e, r)| coedge(*e, !*r)).collect()
        };
        let mut coedges = Vec::with_capacity(4 + start_edges.len() + finish_edges.len());
        if reversed_first {
            // The first face runs the border backwards: this face forwards.
            coedges.push(coedge(piece.edge, false));
            coedges.extend(forwards(&finish_edges));
            coedges.push(coedge(second_border, true));
            coedges.extend(backwards(&start_edges));
        } else {
            coedges.push(coedge(piece.edge, true));
            coedges.extend(forwards(&start_edges));
            coedges.push(coedge(second_border, false));
            coedges.extend(backwards(&finish_edges));
        }
        let ring = LoopId(u32::try_from(body.loops.len()).map_err(|_| lost())?);
        body.loops.push(Loop { coedges });
        let PieceMade {
            surface,
            spans,
            outward,
        } = m;
        let [(u0, u1), _] = spans;
        let (_, du, dv) = surface.derivatives(0.5 * (u0 + u1), 0.5);
        let reversed = du.cross(dv).dot(outward) < 0.0;
        let id = FaceId(u32::try_from(body.faces.len()).map_err(|_| lost())?);
        body.faces.push(craft3d_topo::Face {
            surface: Surface::Nurbs(surface),
            reversed,
            loops: vec![ring],
        });
        body.shells.get_mut(shell).ok_or_else(lost)?.faces.push(id);
        faces.push((id, second_border));
    }
    Ok(faces)
}

/// The direction the balls' centres move in at `section`: along both
/// offset surfaces, square to both normals at the feet (`along` says which
/// way).
fn spine_tangent(sides: [Side<'_>; 2], section: &Section, along: Vec3) -> Option<Vec3> {
    let [a, b] = sides;
    let [pa, pb] = section.params;
    let t = a.normal(pa)?.cross(b.normal(pb)?).normalized()?;
    Some(if t.dot(along) < 0.0 { -t } else { t })
}

/// The plane a ball's arc lies in (square to the centres' path there),
/// its normal along `along`.
fn arc_plane(section: &Section, along: Vec3) -> Option<Across> {
    let c = section.centre?;
    let [f0, f1] = section.feet;
    let n = (f0 - c).cross(f1 - c).normalized()?;
    Some(Across {
        point: c,
        normal: if n.dot(along) < 0.0 { -n } else { n },
    })
}

/// Where a walk along the spine stops.
enum Until<'a> {
    /// Where the centres cross this plane (from behind it).
    Plane(Across),
    /// Past this plane, where a whole section lies beyond `third`:
    /// outside it (`sign` 1, at a corner of three faces or a convex meet),
    /// or inside it (−1, a concave meet, whose far face it crosses into
    /// the material).
    Out {
        end: Across,
        third: Side<'a>,
        sign: f64,
    },
}

/// What a walk found: the balls after the start, before the plane it
/// stops at (`Plane`), or up to and with the first one outside the third
/// face (`Out`).
struct Walked {
    balls: Vec<Section>,
}

/// Walks along the spine from `start` (moving along `along`): each ball in
/// the plane square to the path a step ahead, the step shrinking where the
/// ball cannot be found or the path turns by more than a quarter radian
/// and growing up to `longest` where it runs straight.
#[allow(clippy::too_many_arguments)]
fn walk(
    solver: &Solver<'_>,
    start: Section,
    along: Vec3,
    first_step: f64,
    longest: f64,
    until: &Until<'_>,
    most_out: f64,
    tol: f64,
) -> Result<Walked, Stop> {
    let Shape::Ball { radius, convex } = solver.shape else {
        return Err(Stop::Unsolvable);
    };
    let shortest = 1e-4 * first_step;
    let (mut current, mut t, mut h) = (start, along, first_step);
    let mut balls = Vec::new();
    let mut past: Option<f64> = None;
    let mut travelled = 0.0;
    // A plane is crossed where the centres go from behind it to in front
    // of it near the point it is given by (the path may pass the plane far
    // away, round a closed outline).
    let plane_of = |until: &Until<'_>| match until {
        Until::Plane(plane) => *plane,
        Until::Out { end, .. } => *end,
    };
    let near = |centre: Vec3, plane: &Across, h: f64| {
        centre.distance(plane.point) <= 4.0 * radius + 2.0 * h
    };
    let side = |centre: Vec3| {
        let plane = plane_of(until);
        plane.normal.dot(centre - plane.point)
    };
    let mut behind = side(start.centre.ok_or(Stop::Unsolvable)?) < 0.0;
    for _ in 0..8192 {
        let c = current.centre.ok_or(Stop::Unsolvable)?;
        let ahead = Across {
            point: c + t * h,
            normal: t,
        };
        let found = solver
            .ball(ahead, &current, radius, convex)
            .and_then(|next| Some((next, spine_tangent(solver.sides, &next, t)?)));
        let Some((next, turned)) = found.filter(|(_, n)| n.dot(t) > 0.25_f64.cos()) else {
            h *= 0.5;
            if h < shortest {
                return Err(Stop::Unsolvable);
            }
            continue;
        };
        let centre = next.centre.ok_or(Stop::Unsolvable)?;
        let plane = plane_of(until);
        let crossed = behind && side(centre) >= 0.0 && near(centre, &plane, h);
        behind = side(centre) < 0.0;
        match until {
            Until::Plane(_) => {
                if crossed {
                    return Ok(Walked { balls });
                }
            }
            Until::Out { third, sign, .. } => {
                if past.is_none() && crossed {
                    past = Some(travelled);
                }
                if let Some(at) = past {
                    if beyond_by(*third, &next, tol, *sign) {
                        balls.push(next);
                        return Ok(Walked { balls });
                    }
                    // No further than the face's approximation can take
                    // samples (`march_out`).
                    if travelled - at > most_out || balls.len() >= MAX_SAMPLES {
                        return Err(Stop::TooFar);
                    }
                }
            }
        }
        travelled += centre.distance(c);
        balls.push(next);
        let straight = turned.dot(t) > 0.05_f64.cos();
        current = next;
        t = turned;
        if straight {
            h = (h * 1.5).min(longest);
        }
    }
    Err(Stop::Unsolvable)
}

/// The path of the balls' centres through `balls` (in order): the cubic
/// spline through them with continuous curvature (one piece between two
/// balls, its parameter their distance along the chords from 0 to 1),
/// leaving the first and reaching the last in the direction the centres
/// move in there (square to both normals at the feet on `sides`); with
/// each ball's parameter.
fn path_through(balls: &[Section], sides: [Side<'_>; 2]) -> Option<(Curve3, Vec<f64>)> {
    let centres: Vec<Vec3> = balls.iter().map(|b| b.centre).collect::<Option<_>>()?;
    let n = centres.len();
    if n < 2 {
        return None;
    }
    let chords: Vec<f64> = centres
        .windows(2)
        .filter_map(|w| match w {
            [a, b] => Some(a.distance(*b)),
            _ => None,
        })
        .collect();
    let total: f64 = chords.iter().sum();
    if !(total > 0.0) || chords.iter().any(|c| !(*c > 0.0)) {
        return None;
    }
    let mut params = Vec::with_capacity(n);
    let mut sum = 0.0;
    params.push(0.0);
    for c in &chords {
        sum += c;
        params.push(sum / total);
    }
    let steps: Vec<f64> = chords.iter().map(|c| c / total).collect();
    let end_tangent = |i: usize, j: usize| -> Option<Vec3> {
        let (a, b) = (*centres.get(i)?, *centres.get(j)?);
        Some(spine_tangent(sides, balls.get(i)?, b - a)? * total)
    };
    let first = end_tangent(0, 1)?;
    let last = {
        let t = end_tangent(n - 1, n - 2)?;
        -t
    };
    let tangents = clamped_tangents(&centres, &steps, first, last)?;
    let mut ctrl = Vec::with_capacity(3 * (n - 1) + 1);
    for i in 0..n - 1 {
        let (p, q) = (*centres.get(i)?, *centres.get(i + 1)?);
        let (tp, tq) = (*tangents.get(i)?, *tangents.get(i + 1)?);
        let third = steps.get(i)? / 3.0;
        ctrl.push(p);
        ctrl.push(p + tp * third);
        ctrl.push(q - tq * third);
    }
    ctrl.push(*centres.last()?);
    let mut knots = vec![0.0; 4];
    for p in params.iter().skip(1).take(n - 2) {
        knots.extend([*p; 3]);
    }
    knots.extend([1.0; 4]);
    let curve = NurbsCurve::polynomial(3, knots, ctrl).ok()?;
    Some((Curve3::Nurbs(curve), params))
}

/// The derivatives at `points` (at parameter steps `steps`) of the cubic
/// spline through them with continuous curvature whose derivatives at the
/// first and the last are `first` and `last`: the tridiagonal system of a
/// clamped spline, solved by elimination.
fn clamped_tangents(points: &[Vec3], steps: &[f64], first: Vec3, last: Vec3) -> Option<Vec<Vec3>> {
    let n = points.len();
    if n < 2 || steps.len() + 1 != n {
        return None;
    }
    if n == 2 {
        return Some(vec![first, last]);
    }
    // Unknowns: the tangents at 1 .. n-2.
    let m = n - 2;
    let mut lower = vec![0.0; m];
    let mut diagonal = vec![0.0; m];
    let mut upper = vec![0.0; m];
    let mut rhs = vec![Vec3::ZERO; m];
    for k in 0..m {
        let i = k + 1;
        let (hb, ha) = (*steps.get(i - 1)?, *steps.get(i)?);
        let (pb, pi, pa) = (*points.get(i - 1)?, *points.get(i)?, *points.get(i + 1)?);
        let mut r = ((pi - pb) * (ha / hb) + (pa - pi) * (hb / ha)) * 3.0;
        *diagonal.get_mut(k)? = 2.0 * (hb + ha);
        if k == 0 {
            r = r - first * ha;
        } else {
            *lower.get_mut(k)? = ha;
        }
        if k + 1 == m {
            r = r - last * hb;
        } else {
            *upper.get_mut(k)? = hb;
        }
        *rhs.get_mut(k)? = r;
    }
    // Forward elimination, then back substitution.
    for k in 1..m {
        let w = lower.get(k)? / diagonal.get(k - 1)?;
        let (u, r) = (*upper.get(k - 1)?, *rhs.get(k - 1)?);
        *diagonal.get_mut(k)? -= w * u;
        let slot = rhs.get_mut(k)?;
        *slot = *slot - r * w;
    }
    let mut inner = vec![Vec3::ZERO; m];
    for k in (0..m).rev() {
        let next = if k + 1 < m {
            *inner.get(k + 1)?
        } else {
            Vec3::ZERO
        };
        let d = *diagonal.get(k)?;
        if !(d.abs() > 0.0) {
            return None;
        }
        *inner.get_mut(k)? = (*rhs.get(k)? - next * *upper.get(k)?) * (1.0 / d);
    }
    let mut all = Vec::with_capacity(n);
    all.push(first);
    all.extend(inner);
    all.push(last);
    all.iter().all(|t| t.is_finite()).then_some(all)
}

/// About `count` of the indices into `params` (increasing), at nearly even
/// steps of the parameter, the first and the last always.
fn pick(params: &[f64], count: usize) -> Vec<usize> {
    let n = params.len();
    if n <= count || count < 2 {
        return (0..n).collect();
    }
    let mut picked: Vec<usize> = Vec::with_capacity(count);
    let mut at = 0;
    for k in 0..count {
        let goal = k as f64 / (count - 1) as f64;
        while at + 1 < n && params.get(at + 1).is_some_and(|p| *p <= goal) {
            at += 1;
        }
        let best = if params.get(at + 1).is_some_and(|p| {
            (p - goal).abs() < (params.get(at).copied().unwrap_or(0.0) - goal).abs()
        }) {
            at + 1
        } else {
            at
        };
        if picked.last() != Some(&best) {
            picked.push(best);
        }
    }
    if picked.last() != Some(&(n - 1)) {
        picked.push(n - 1);
    }
    if picked.first() != Some(&0) {
        picked.insert(0, 0);
    }
    picked
}

/// Works out the fillet faces of `chain` on `body` (unchanged), following
/// the balls along their path: from the middle of the first edge round
/// the chain, each edge's balls from the joint before it to the joint
/// after it (a ball where a face changes is found on the smooth edge; one
/// where none does, in the plane across the edges at the vertex). The
/// sections of each face are then the balls in the planes square to a
/// cubic through the centres found, turned at the joints into the planes
/// of the joints' balls, so the faces on both sides of a joint end in the
/// same ball.
#[allow(clippy::too_many_lines)]
fn along_spine(
    body: &Body,
    chain: &Chain,
    items: &[Item],
    meets: &[Meet],
    tolerance: &Tolerance,
) -> Result<Computed, BlendError> {
    let tol = tolerance.linear;
    let pieces = &chain.pieces;
    let n = pieces.len();
    let first = *pieces.first().ok_or_else(lost)?;
    let edge = first.edge;
    let unsolvable = || refuse(edge, None, Reason::FreeSurface);
    let shape = first.shape;
    let size = shape.size();
    let convex = items.get(first.item).ok_or_else(lost)?.convex;
    let datas: Vec<&Edge> = pieces
        .iter()
        .map(|p| body.edge(p.edge).ok_or_else(lost))
        .collect::<Result<_, _>>()?;
    let sides: Vec<[Side<'_>; 2]> = pieces
        .iter()
        .map(|p| sides_of(body, p.faces))
        .collect::<Result<_, _>>()?;
    let scale = datas
        .iter()
        .map(|d| scale_of(d, size, tol))
        .fold(tol, f64::max);
    let after = |j: usize| (j + 1) % n;
    let lengths: Vec<f64> = datas.iter().map(|d| edge_length(d)).collect();
    if lengths.iter().any(|l| !(*l > 0.0)) {
        return Err(unsolvable());
    }
    // The edges' own planes: for the seed and the joints and ends.
    let edge_solvers: Vec<Solver<'_>> = pieces
        .iter()
        .enumerate()
        .map(|(i, p)| -> Result<Solver<'_>, BlendError> {
            let data = *datas.get(i).ok_or_else(lost)?;
            Ok(Solver {
                spine: Spine {
                    curve: &data.curve,
                    range: if p.forward {
                        data.range
                    } else {
                        (data.range.1, data.range.0)
                    },
                    closed: false,
                    joints: [None, None],
                    turn: 0.0,
                },
                sides: *sides.get(i).ok_or_else(lost)?,
                shape,
                scale,
            })
        })
        .collect::<Result<_, _>>()?;
    // The balls where a face changes, known before the walk.
    let mut switched: Vec<Option<(Section, Section, f64)>> = Vec::with_capacity(chain.joints.len());
    let mut stops: Vec<Across> = Vec::with_capacity(chain.joints.len());
    for (j, joint) in chain.joints.iter().enumerate() {
        let (p, q) = (
            pieces.get(j).ok_or_else(lost)?,
            pieces.get(after(j)).ok_or_else(lost)?,
        );
        let (pd, qd) = (
            *datas.get(j).ok_or_else(lost)?,
            *datas.get(after(j)).ok_or_else(lost)?,
        );
        let (Some(out), Some(on)) = (
            direction(pd, p.forward, true),
            direction(qd, q.forward, false),
        ) else {
            return Err(unsolvable());
        };
        let along = (out + on).normalized().ok_or_else(unsolvable)?;
        match joint.switch {
            None => {
                let point = body.vertex(joint.vertex).ok_or_else(lost)?.point;
                stops.push(Across {
                    point,
                    normal: along,
                });
                switched.push(None);
            }
            Some(switch) => {
                let smooth = body.edge(switch.smooth).ok_or_else(lost)?;
                let (before, later, plane, t) = switch_ball(
                    *sides.get(j).ok_or_else(lost)?,
                    *sides.get(after(j)).ok_or_else(lost)?,
                    switch.side,
                    smooth,
                    joint.vertex,
                    shape,
                    along,
                    scale,
                )
                .ok_or_else(|| refused(q.edge, Reason::FreeJoint))?;
                stops.push(plane);
                switched.push(Some((before, later, t)));
            }
        }
    }
    // The ends of an open chain.
    let ends = if chain.closed {
        None
    } else {
        let last = *pieces.last().ok_or_else(lost)?;
        let (fd, ld) = (
            *datas.first().ok_or_else(lost)?,
            *datas.last().ok_or_else(lost)?,
        );
        let [at_start, at_end] = chain.meets;
        let ending = |piece: &Piece, data: &Edge, at_end: bool, meet: Option<usize>| match meet {
            Some(index) => {
                let found = meets.get(index).ok_or_else(lost)?;
                let [x, y] =
                    faces_of(&super::super::edge_uses(body), found.third).ok_or_else(lost)?;
                let far = if piece.faces.contains(&x) { y } else { x };
                Ok(Ending::Meet { far })
            }
            None => end_at(
                body,
                piece.edge,
                piece.faces,
                vertex_of(data, piece.forward, at_end),
                tolerance.angular.max(BEND).max(tolerance.linear / size),
            )
            .map(Ending::Third),
        };
        Some([
            ending(&first, fd, false, at_start)?,
            ending(&last, ld, true, at_end)?,
        ])
    };
    let third_of = |end: &Ending| -> Result<Side<'_>, BlendError> {
        match end {
            Ending::Third(end) => end.side(body),
            Ending::Meet { .. } => {
                let face = body.face(end.third()).ok_or_else(lost)?;
                Ok(Side {
                    surface: &face.surface,
                    reversed: face.reversed,
                })
            }
        }
    };
    // Steps of up to two radii, or a 24th of the edge where that is
    // more: the walk only has to follow the path, which the steps adapt
    // to where it turns.
    let step_of = |i: usize| -> (f64, f64) {
        let length = lengths.get(i).copied().unwrap_or(size);
        let longest = (length / 24.0)
            .max(2.0 * size)
            .min(length / 3.0)
            .max(1e-3 * size);
        ((0.5 * size).min(longest), longest)
    };
    let most_out = 8.0 * size;
    // Past a meet's vertex a concave fillet leaves what is kept (the
    // empty side) into the material beyond the far face.
    let sign_of = |end: &Ending| match end {
        Ending::Meet { .. } if !convex => -1.0,
        _ => 1.0,
    };
    // Why a walk out past an end stopped: the ball could not be followed
    // (a face curves more tightly than the radius somewhere), or it did
    // not get out of the third face.
    let stopped = |stopped: Stop, face: Option<FaceId>| match stopped {
        Stop::Unsolvable => unsolvable(),
        Stop::TooFar => refuse(edge, face, Reason::FreeEnd),
    };
    // The seed, in the middle of the first edge.
    let solver0 = edge_solvers.first().ok_or_else(lost)?;
    let into = items.get(first.item).ok_or_else(lost)?.into;
    let seed = first_section(solver0, 0.5, into, edge)?;
    let forward0 = solver0.spine.at(0.5).ok_or_else(unsolvable)?.normal;
    let t0 = spine_tangent(solver0.sides, &seed, forward0).ok_or_else(unsolvable)?;
    // Per piece, its balls in order along the chain; per joint, its ball
    // and plane.
    let mut balls: Vec<Vec<Section>> = vec![Vec::new(); n];
    let mut joint_balls: Vec<(Section, Section)> = Vec::with_capacity(chain.joints.len());
    let mut joint_planes: Vec<Across> = Vec::with_capacity(chain.joints.len());
    // Per open end, the plane across the edge at its end, looking out.
    let mut end_planes: [Option<Across>; 2] = [None, None];
    let mut from = (seed, t0);
    for i in 0..n {
        let solver = edge_solvers.get(i).ok_or_else(lost)?;
        let (h, longest) = step_of(i);
        let list = balls.get_mut(i).ok_or_else(lost)?;
        list.push(from.0);
        if i + 1 == n && !chain.closed {
            let end = ends.as_ref().ok_or_else(lost)?.get(1).ok_or_else(lost)?;
            let plane = solver.spine.at(1.0).ok_or_else(unsolvable)?;
            end_planes[1] = Some(plane);
            let walked = walk(
                solver,
                from.0,
                from.1,
                h,
                longest,
                &Until::Out {
                    end: plane,
                    third: third_of(end)?,
                    sign: sign_of(end),
                },
                end.most_out(most_out),
                tol,
            )
            .map_err(|s| stopped(s, Some(end.third())))?;
            list.extend(walked.balls);
            break;
        }
        let stop = *stops.get(i).ok_or_else(lost)?;
        let walked = walk(
            solver,
            from.0,
            from.1,
            h,
            longest,
            &Until::Plane(stop),
            most_out,
            tol,
        )
        .map_err(|_| unsolvable())?;
        list.extend(walked.balls);
        let (mine, next) = match switched.get(i).ok_or_else(lost)? {
            Some((before, later, _)) => (*before, *later),
            None => {
                let last = *list.last().ok_or_else(lost)?;
                let Shape::Ball { radius, convex } = shape else {
                    return Err(lost());
                };
                let ball = solver
                    .ball(stop, &last, radius, convex)
                    .ok_or_else(unsolvable)?;
                (ball, ball)
            }
        };
        let plane = arc_plane(&mine, stop.normal).ok_or_else(unsolvable)?;
        // A ball found just before the joint would be a sliver of a step.
        if let Some(last) = list.last().and_then(|b| b.centre)
            && list.len() > 1
            && plane.normal.dot(plane.point - last) < 0.1 * h
        {
            list.pop();
        }
        list.push(mine);
        joint_balls.push((mine, next));
        joint_planes.push(plane);
        from = (next, plane.normal);
    }
    // The first piece back from the seed.
    let solver = edge_solvers.first().ok_or_else(lost)?;
    let (h, longest) = step_of(0);
    let back = if chain.closed {
        let closing = n.checked_sub(1).ok_or_else(lost)?;
        let plane = *joint_planes.get(closing).ok_or_else(lost)?;
        let behind = Across {
            point: plane.point,
            normal: -plane.normal,
        };
        let mut walked = walk(
            solver,
            seed,
            -t0,
            h,
            longest,
            &Until::Plane(behind),
            most_out,
            tol,
        )
        .map_err(|_| unsolvable())?
        .balls;
        if let Some(last) = walked.last().and_then(|b| b.centre)
            && behind.normal.dot(behind.point - last) < 0.1 * h
        {
            walked.pop();
        }
        walked.push(joint_balls.get(closing).ok_or_else(lost)?.1);
        walked
    } else {
        let end = ends.as_ref().ok_or_else(lost)?.first().ok_or_else(lost)?;
        let start = solver.spine.at(0.0).ok_or_else(unsolvable)?;
        let plane = Across {
            point: start.point,
            normal: -start.normal,
        };
        end_planes[0] = Some(plane);
        let walked = walk(
            solver,
            seed,
            -t0,
            h,
            longest,
            &Until::Out {
                end: plane,
                third: third_of(end)?,
                sign: sign_of(end),
            },
            end.most_out(most_out),
            tol,
        )
        .map_err(|s| stopped(s, Some(end.third())))?;
        walked.balls
    };
    {
        let list = balls.first_mut().ok_or_else(lost)?;
        let mut whole: Vec<Section> = back.into_iter().rev().collect();
        whole.append(list);
        *list = whole;
    }
    // The paths, and each piece's planes: square to its path, turned into
    // the joints' planes at its ends.
    let mut paths = Vec::with_capacity(n);
    let mut kept = Vec::with_capacity(n);
    for (i, list) in balls.iter().enumerate() {
        let (curve, params) =
            path_through(list, *sides.get(i).ok_or_else(lost)?).ok_or_else(unsolvable)?;
        paths.push(curve);
        kept.push((list.clone(), params));
    }
    let mut made = Vec::with_capacity(n);
    let limit = 0.25 * tol;
    let bend = tolerance.angular.max(BEND).max(tol / size);
    let mut cut_ranges: [(f64, f64); 2] = [(0.0, 0.0); 2];
    for (i, ((list, params), path)) in kept.iter().zip(&paths).enumerate() {
        let before = if i > 0 {
            joint_planes.get(i - 1).copied()
        } else if chain.closed {
            joint_planes.last().copied()
        } else {
            None
        };
        let behind = if i + 1 < n || chain.closed {
            joint_planes.get(i).copied()
        } else {
            None
        };
        let solver = Solver {
            spine: Spine {
                curve: path,
                range: (0.0, 1.0),
                closed: false,
                joints: [before, behind],
                turn: 0.5,
            },
            sides: *sides.get(i).ok_or_else(lost)?,
            shape,
            scale,
        };
        // The surface starts from some of the balls, evenly along the
        // path; it is refined from there.
        let picked = pick(params, 48);
        let count = list.len();
        let mut sections = Vec::with_capacity(picked.len());
        let mut samples = Vec::with_capacity(picked.len());
        for k in picked {
            let (Some(ball), Some(s)) = (list.get(k), params.get(k)) else {
                return Err(lost());
            };
            let fixed = (k == 0 && before.is_some()) || (k + 1 == count && behind.is_some());
            if fixed {
                // The joint's ball, as this piece's faces hold it: where a
                // face changes across an edge along which the two are
                // tangent only to within the tolerance, the ball of this
                // piece's own faces in the joint's plane is a little off
                // the shared one, and the face has to end in its own. The
                // shared arc is the edge: it has to be within the tolerance
                // of the face, which is within a quarter of it of its own
                // arc, so the two arcs may be three quarters of it apart.
                // Further off, the faces are creased at the joint by more
                // than the tolerance allows at this size: refused.
                let own = solver.follow(*s, ball, *s, 0).ok_or_else(unsolvable)?;
                if !apart(&own, ball).is_some_and(|d| d <= 0.75 * tol) {
                    let edge = pieces.get(i).ok_or_else(lost)?.edge;
                    return Err(refused(edge, Reason::FreeJoint));
                }
                sections.push(own);
            } else {
                sections.push(solver.follow(*s, ball, *s, 0).ok_or_else(unsolvable)?);
            }
            samples.push(*s);
        }
        // Where the cuts at the open ends are looked for: from a few
        // radii before the end of the edge to the last ball.
        if !chain.closed {
            let back = 4.0 * size / path_length(list);
            let [start_plane, end_plane] = end_planes;
            if let (0, Some(plane)) = (i, start_plane) {
                let at = crossing_param(list, params, &plane, false);
                cut_ranges[0] = ((at + back).min(0.5 * (at + 1.0)), 0.0);
            }
            if let (true, Some(plane)) = (i + 1 == n, end_plane) {
                let at = crossing_param(list, params, &plane, true);
                cut_ranges[1] = ((at - back).max(0.5 * at), 1.0);
            }
        }
        let mut params = samples;
        let piece_edge = pieces.get(i).ok_or_else(lost)?.edge;
        let surface = refine(&solver, &mut params, &mut sections, limit, bend)
            .ok_or_else(|| refuse(piece_edge, None, Reason::FreeApproximation))?;
        let outward =
            outward_from(&solver, &surface, &params, &sections, convex).ok_or_else(unsolvable)?;
        made.push(PieceMade {
            surface,
            spans: [(0.0, 1.0); 2],
            outward,
        });
    }
    let ends = match ends {
        None => None,
        Some([start, finish]) => {
            let mut done = Vec::with_capacity(2);
            for (index, end) in [start, finish].into_iter().enumerate() {
                let piece = if index == 0 { 0 } else { n - 1 };
                let range = *cut_ranges.get(index).ok_or_else(lost)?;
                let end = match end {
                    Ending::Meet { .. } => {
                        done.push(EndKind::Meet { range });
                        continue;
                    }
                    Ending::Third(end) => end,
                };
                let surface = &made.get(piece).ok_or_else(lost)?.surface;
                let (inner, outer) = range;
                let side = end.side(body)?;
                let (cut, at) =
                    cut_curve(surface, side, inner, outer, limit, scale).ok_or_else(|| {
                        refuse(
                            pieces.get(piece).map_or(edge, |p| p.edge),
                            Some(end.third),
                            Reason::FreeEnd,
                        )
                    })?;
                let spans = &mut made.get_mut(piece).ok_or_else(lost)?.spans;
                for (k, s) in at.iter().enumerate() {
                    let span = spans.get_mut(k).ok_or_else(lost)?;
                    if index == 0 {
                        span.0 = *s;
                    } else {
                        span.1 = *s;
                    }
                }
                done.push(EndKind::Third(EndMade { end, cut, at }));
            }
            let [a, b] = <[EndKind; 2]>::try_from(done).map_err(|_| lost())?;
            Some([a, b])
        }
    };
    for (piece, m) in pieces.iter().zip(&made) {
        for (k, (a, b)) in m.spans.iter().enumerate() {
            if !(b > a) {
                let face = *piece.faces.get(k).ok_or_else(lost)?;
                return Err(refuse(piece.edge, Some(face), Reason::FreeTooLarge));
            }
        }
    }
    let joints = joint_balls
        .into_iter()
        .zip(&switched)
        .map(|((section, _), switch)| JointMade {
            section,
            touched: switch.as_ref().map(|s| s.2),
        })
        .collect();
    Ok(Computed {
        pieces: made,
        joints,
        ends,
    })
}

/// How an open chain ends, before it is cut: at a corner of three faces,
/// or at a meet, running on past the vertex out of the far face (the
/// other blended edge's other face).
enum Ending {
    Third(End),
    Meet { far: FaceId },
}

impl Ending {
    /// The face the blend runs out of past the vertex.
    fn third(&self) -> FaceId {
        match self {
            Ending::Third(end) => end.third,
            Ending::Meet { far, .. } => *far,
        }
    }

    /// How far past this end the walk may go before giving up: longer the
    /// more nearly the edge runs along the third face (`End::steps`).
    fn most_out(&self, most_out: f64) -> f64 {
        match self {
            Ending::Third(end) => most_out / end.slant,
            Ending::Meet { .. } => most_out,
        }
    }
}

/// The length of the path through the balls' centres.
fn path_length(balls: &[Section]) -> f64 {
    balls
        .windows(2)
        .filter_map(|w| match w {
            [a, b] => Some(a.centre?.distance(b.centre?)),
            _ => None,
        })
        .sum::<f64>()
        .max(f64::MIN_POSITIVE)
}

/// The parameter of the ball where the path crosses `plane` (whose
/// normal looks out of the edge's end): the first one past it from the
/// start (`last`: the end at 1), or the last one past it from the start
/// (the end at 0).
fn crossing_param(balls: &[Section], params: &[f64], plane: &Across, last: bool) -> f64 {
    let past = |b: &Section| {
        b.centre
            .is_some_and(|c| plane.normal.dot(c - plane.point) >= 0.0)
    };
    // From the open end inwards, the balls past the end come first.
    let found = if last {
        let from_end = balls.iter().rev().position(|b| !past(b));
        from_end.map(|k| balls.len().saturating_sub(k))
    } else {
        balls
            .iter()
            .position(|b| !past(b))
            .map(|k| k.saturating_sub(1))
    };
    found
        .and_then(|k| params.get(k).copied())
        .unwrap_or(if last { 1.0 } else { 0.0 })
}
