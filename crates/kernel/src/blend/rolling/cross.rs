//! A border carried across a smooth edge of its face.
//!
//! **What it is.** The border of a blend on one side lies on the face of
//! that side, as long as the ball (or a chamfer's cut) touches that face.
//! Where the face goes on smoothly across one of its edges (tangent to
//! the face beyond it: the tangent line of an earlier round, the edge
//! between the two halves of a revolved spline) the ball goes on touching
//! the face beyond: the surface it rolls against is the two faces, one
//! smooth sheet, and the border crosses the edge into the face beyond.
//! Across an edge that is not smooth the ball would roll over a crease:
//! that is still refused (a border may not come near another edge of its
//! face).
//!
//! **Which edges are crossed, and where.** A smooth edge of the side's
//! face that the blend takes part of has to enter what the blend takes
//! somewhere; it does so either through the border (a point of the edge
//! where the ball that touches the face there touches the other face too)
//! or at a vertex on the boundary with a third face at an end of the
//! edge. The method looks for the second kind: at each end, the boundary
//! between the side's face and the third face is followed from the blended
//! edge's vertex (across vertices that only split it) to the first vertex
//! where another edge leaves it; when that boundary lies inside what the
//! blend takes all along, and the edge is tangent there and leads to
//! another face whose boundary with the third face goes on there, it is
//! a candidate. A point counts only ahead of the edge (on the side the
//! face is on: behind it the same surfaces go on, and a ball there may
//! reach the other surface too). Along it, a point is inside what the blend takes when
//! the ball touching the side's face there (its centre the radius in
//! along the face's normal) is nearer the other face than the radius (a
//! chamfer: when it is nearer the edge, in the plane across the edge, than
//! the distance). The candidate's vertex inside, and a change from inside
//! to outside once along it: the border crosses it there, at the point
//! found by bisection, where the ball touches both faces and the plane
//! across the edge through its centre gives the place along the blend
//! (the *switch*). Inside all along: it runs from one end to the other
//! and the face before it lies wholly in what the blend takes (it goes;
//! the border is on the face beyond all along). Anything else (the edge
//! comes back in, ends inside, is not tangent where it is crossed) is
//! refused as a border running over an edge of its face.
//!
//! **The blend.** On each side of a switch the sections are found against
//! the face that side touches, and the two pieces are refined each on
//! their own faces; both pass through the section at the switch exactly,
//! which is the ball touching both faces there, and are joined into one
//! surface across it (continuous, tangent within what the pieces are
//! refined to). The faces at the ends are followed across the crossed
//! edge's vertex (the end's walk takes the edge before it whole). A
//! border that cannot be carried (no ball against the faces beyond) is
//! refused as running off its face, as before (`blend_one`).
//!
//! **The faces after.** The border is cut at each switch into pieces,
//! each an edge on the face it lies on, with a new vertex at the switch;
//! the crossed edge is shortened to it (the piece inside goes), or taken
//! whole when it lay inside all along. The loops of the faces it touches
//! are linked again from their edges (the border's pieces in place of the
//! border); a face left with no edges is gone (named nowhere after), one
//! whose loops would come out different in number is refused.
//!
//! **Seams.** No operation of the kernel makes a face that meets itself
//! along an edge: a periodic face is bounded by two loops (a cylinder's
//! wall), a revolved spline is made in two halves that meet in a smooth
//! edge, and the circles where a periodic face is cut are split at its
//! seam by a vertex of two edges. The vertex is walked across at the ends
//! (`walk_on`), the smooth edge is crossed here; an edge between a face
//! and itself is refused.

// `!(x > 0.0)` is deliberate: unlike `x <= 0.0` it is also true for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use super::super::{BlendError, Reason, Use, edge_uses, lost, sine_at};
use super::{End, Section, Shape, Side, bend, edges_at, faces_of_edge, refuse};
use craft3d_geom::NurbsSurface;
use craft3d_math::{Tolerance, Vec3};
use craft3d_topo::{Body, Coedge, Edge, EdgeId, FaceId, VertexId};

/// Samples along a crossed edge in which the change from inside to
/// outside is looked for, and the bisection steps that find it.
const SAMPLES: usize = 64;
const BISECTIONS: usize = 80;

/// Where a border crosses a smooth edge of its side's face.
#[derive(Clone, Copy, Debug)]
pub(super) struct Crossing {
    pub side: usize,
    /// The smooth edge, between `before` (the side's face) and `after`.
    pub smooth: EdgeId,
    pub before: FaceId,
    pub after: FaceId,
    /// The end of the blended edge beyond which the face `after` lies:
    /// past the switch towards it the border is on `after`.
    pub end: usize,
    /// The vertex of `smooth` on the boundary with the third face there.
    pub inside: VertexId,
    /// Where the border crosses; `None` when the smooth edge lies inside
    /// what the blend takes all along (from one end to the other).
    pub at: Option<Switch>,
}

/// The point at which a border crosses a smooth edge.
#[derive(Clone, Copy, Debug)]
pub(super) struct Switch {
    /// The place along the blend (the spine's `s`) of the section there.
    pub s: f64,
    /// The smooth edge's parameter there, and the point.
    pub t: f64,
    pub point: Vec3,
}

/// The first vertex along the boundary between `face` and `third`,
/// followed from `vertex` along `first`, at which another edge leaves it
/// (vertices of two edges of that boundary only are passed): the vertex,
/// the edge that leaves it, and the edges of the boundary up to it.
fn branch(
    body: &Body,
    first: EdgeId,
    vertex: VertexId,
    [face, third]: [FaceId; 2],
) -> Option<(VertexId, EdgeId, Vec<EdgeId>)> {
    let sorted = |a: FaceId, b: FaceId| if a <= b { [a, b] } else { [b, a] };
    let pair = |e: EdgeId| -> Option<[FaceId; 2]> {
        match faces_of_edge(body, e).as_slice() {
            [a, b] => Some(sorted(*a, *b)),
            _ => None,
        }
    };
    let want = sorted(face, third);
    let (mut current, mut near) = (first, vertex);
    let mut path = Vec::new();
    for _ in 0..body.edges.len() {
        path.push(current);
        let data = body.edge(current)?;
        let far = if data.start == near {
            data.end
        } else if data.end == near {
            data.start
        } else {
            return None;
        };
        if far == near || far == vertex {
            return None;
        }
        let rest = edges_at(body, far, current);
        match rest.as_slice() {
            [next] if pair(*next) == Some(want) => {
                current = *next;
                near = far;
            }
            [a, b] => {
                // One leaves into the face, the other goes on between the
                // face beyond it and the third face.
                for (leaves, goes) in [(*a, *b), (*b, *a)] {
                    let Some(lp) = pair(leaves) else { continue };
                    if !lp.contains(&face) || lp.contains(&third) {
                        continue;
                    }
                    let beyond = if lp[0] == face { lp[1] } else { lp[0] };
                    if beyond != face && pair(goes) == Some(sorted(beyond, third)) {
                        return Some((far, leaves, path));
                    }
                }
                return None;
            }
            _ => return None,
        }
    }
    None
}

/// Whether every edge of `path` (from the blended edge's vertex along
/// the boundary with the third face) lies inside what the blend takes on
/// side `k`, at its far end and at points between (the points of the
/// face's surface whose balls reach the other face: further on, the same
/// faces may meet again, as both root edges of a boss do).
fn inside_along(body: &Body, blend: &Blend<'_>, k: usize, path: &[EdgeId]) -> bool {
    const POINTS: u32 = 16;
    path.iter().all(|e| {
        body.edge(*e).is_some_and(|data| {
            (0..=POINTS).all(|j| {
                let t =
                    data.range.0 + (data.range.1 - data.range.0) * f64::from(j) / f64::from(POINTS);
                blend.inside(k, data.curve.eval(t)).is_some_and(|g| g < 0.0)
            })
        })
    })
}

/// What [`find`] needs of the blend.
pub(super) struct Blend<'a> {
    pub edge: EdgeId,
    pub data: &'a Edge,
    pub faces: [FaceId; 2],
    pub sides: [Side<'a>; 2],
    pub shape: Shape,
    pub ends: &'a [End; 2],
    /// The directions into the two faces, square to the edge, at the
    /// middle of the edge.
    pub into: [Vec3; 2],
    pub tol: f64,
}

impl Blend<'_> {
    /// Whether `q`, a point of the face of side `k`, lies on the side of
    /// the edge the face is on (the strip a blend takes is there; behind
    /// the edge the same faces' surfaces go on, and a ball there may reach
    /// the other surface too): in the plane across the edge through `q`,
    /// along the direction into the face (the face's normal across the
    /// edge's tangent, turned the way `into` is at the middle).
    fn ahead(&self, k: usize, q: Vec3) -> Option<bool> {
        let side = self.sides.get(k)?;
        let across = |t: f64| -> Option<Vec3> {
            let (e, d) = self.data.curve.derivatives(t);
            let uv = side.surface.project(e).ok()?;
            side.normal(uv)?.cross(d).normalized()
        };
        let (t0, t1) = self.data.range;
        let middle = across(0.5 * (t0 + t1))?;
        let sign = if middle.dot(*self.into.get(k)?) < 0.0 {
            -1.0
        } else {
            1.0
        };
        let t = self.data.curve.project(q).ok()?;
        let e = self.data.curve.eval(t);
        Some((q - e).dot(across(t)? * sign) >= -self.tol)
    }

    /// How far inside (negative) or outside (positive) what the blend
    /// takes the point `q` of the face of side `k` is, relative to the
    /// size; `None` where it cannot be told. A point behind the edge is
    /// outside.
    fn inside(&self, k: usize, q: Vec3) -> Option<f64> {
        if !self.ahead(k, q)? {
            return Some(f64::INFINITY);
        }
        let side = self.sides.get(k)?;
        match self.shape {
            Shape::Ball { radius, convex } => {
                let inward = if convex { -radius } else { radius };
                let uv = side.surface.project(q).ok()?;
                let n = side.normal(uv)?;
                let centre = q + n * inward;
                let other = self.sides.get(1 - k)?;
                let start = other.surface.project(centre).ok()?;
                let (d, _) = other.signed(centre, start)?;
                let g = d / inward - 1.0;
                g.is_finite().then_some(g)
            }
            Shape::Cut { distances } => {
                let d = *distances.get(k)?;
                let t = self.data.curve.project(q).ok()?;
                let e = self.data.curve.eval(t);
                let g = q.distance(e) / d - 1.0;
                g.is_finite().then_some(g)
            }
        }
    }

    /// The place along the blend (the spine's `s`) of the section whose
    /// foot on side `k` is `q`.
    fn place(&self, k: usize, q: Vec3) -> Option<f64> {
        let on = match self.shape {
            Shape::Ball { radius, convex } => {
                let side = self.sides.get(k)?;
                let inward = if convex { -radius } else { radius };
                let uv = side.surface.project(q).ok()?;
                q + side.normal(uv)? * inward
            }
            Shape::Cut { .. } => q,
        };
        let t = self.data.curve.project(on).ok()?;
        let (t0, t1) = self.data.range;
        let s = (t - t0) / (t1 - t0);
        s.is_finite().then_some(s)
    }
}

/// The smooth edges the borders of `blend` cross (see the module
/// documentation), at most one from each end on each side.
pub(super) fn find(
    body: &Body,
    blend: &Blend<'_>,
    tolerance: &Tolerance,
) -> Result<Vec<Crossing>, BlendError> {
    let edge = blend.edge;
    let runs_over = |face: FaceId| refuse(edge, Some(face), Reason::FreeRunsOver);
    let tol = tolerance.linear;
    let size = blend.shape.size();
    // Tangent where it is crossed as closely as a blend's own tangency
    // across a joint is measured: ten times what its faces are refined to.
    let bound = 10.0 * bend(tolerance, size);
    // Which faces use which edge: only once a candidate is found.
    let uses: std::cell::OnceCell<Vec<Vec<Use>>> = std::cell::OnceCell::new();
    let mut found: Vec<Crossing> = Vec::new();
    for k in 0..2 {
        let face = *blend.faces.get(k).ok_or_else(lost)?;
        for (j, end) in blend.ends.iter().enumerate() {
            let first = *end.others.get(k).ok_or_else(lost)?;
            let Some((inside, smooth, path)) = branch(body, first, end.vertex, [face, end.third])
            else {
                continue;
            };
            let at = body.vertex(inside).ok_or_else(lost)?.point;
            // The boundary from the edge's vertex up to there inside what
            // the blend takes all along (the corner lies past it), or not
            // a crossing.
            if !inside_along(body, blend, k, &path) {
                continue;
            }
            // Another face, tangent to this one where the edge leaves the
            // boundary; otherwise not a crossing (the ball would roll over
            // a crease, or the face meet itself: refused as before, by the
            // end that cannot be cut).
            let after = match faces_of_edge(body, smooth).as_slice() {
                [a, b] if *a == face && *b != face => *b,
                [a, b] if *b == face && *a != face => *a,
                _ => continue,
            };
            let tangent = |p: Vec3| -> Result<bool, BlendError> {
                let uses = uses.get_or_init(|| edge_uses(body));
                Ok(sine_at(body, uses, smooth, p)?.is_some_and(|s| s <= bound))
            };
            if !tangent(at)? {
                continue;
            }
            // The same edge from the other end: it lies inside all along.
            if let Some(seen) = found.iter_mut().find(|c| c.side == k && c.smooth == smooth) {
                if seen.at.is_some() {
                    return Err(runs_over(face));
                }
                continue;
            }
            let data = body.edge(smooth).ok_or_else(lost)?;
            let (lo, hi) = data.range;
            // From the vertex inside towards the other end.
            let from_start = data.start == inside;
            let param = |x: f64| {
                if from_start {
                    lo + (hi - lo) * x
                } else {
                    hi - (hi - lo) * x
                }
            };
            let g = |x: f64| blend.inside(k, data.curve.eval(param(x)));
            let mut change = None;
            let mut previous = 0.0_f64;
            for i in 1..=SAMPLES {
                let x = i as f64 / SAMPLES as f64;
                let value = g(x).ok_or_else(|| runs_over(face))?;
                match change {
                    None if !(value < 0.0) => change = Some((previous, x)),
                    // Back inside: the edge runs in and out of what the
                    // blend takes.
                    Some(_) if value < 0.0 => return Err(runs_over(face)),
                    _ => {}
                }
                previous = x;
            }
            let Some((mut a, mut b)) = change else {
                // Inside all along: from one end to the other (checked
                // when the edge is found from the other end).
                found.push(Crossing {
                    side: k,
                    smooth,
                    before: face,
                    after,
                    end: j,
                    inside,
                    at: None,
                });
                continue;
            };
            for _ in 0..BISECTIONS {
                let m = 0.5 * (a + b);
                if !(m > a && m < b) {
                    break;
                }
                let value = g(m).ok_or_else(|| runs_over(face))?;
                if value < 0.0 {
                    a = m;
                } else {
                    b = m;
                }
            }
            let x = 0.5 * (a + b);
            let t = param(x);
            let point = data.curve.eval(t);
            let ends = [data.start, data.end].map(|v| body.vertex(v).map(|v| v.point));
            let near_end = ends
                .iter()
                .any(|p| p.is_none_or(|p| p.distance(point) <= 10.0 * tol));
            if near_end || !tangent(point)? {
                return Err(runs_over(face));
            }
            let s = blend.place(k, point).ok_or_else(|| runs_over(face))?;
            found.push(Crossing {
                side: k,
                smooth,
                before: face,
                after,
                end: j,
                inside,
                at: Some(Switch { s, t, point }),
            });
        }
    }
    // An edge inside all along has to have been found from both ends.
    for c in &found {
        if c.at.is_none() {
            let data = body.edge(c.smooth).ok_or_else(lost)?;
            let both = blend.ends.iter().all(|end| {
                let first = end.others.get(c.side).copied();
                first
                    .and_then(|f| branch(body, f, end.vertex, [c.before, end.third]))
                    .is_some_and(|(v, g, _)| g == c.smooth && (v == data.start || v == data.end))
            });
            if !both || data.start == data.end {
                return Err(runs_over(c.before));
            }
        }
    }
    // Per side, the faces the switches give have to follow each other.
    for k in 0..2 {
        let mine: Vec<&Crossing> = found.iter().filter(|c| c.side == k).collect();
        if mine.len() > 1 && mine.iter().any(|c| c.at.is_none()) {
            return Err(runs_over(*blend.faces.get(k).ok_or_else(lost)?));
        }
        if let [a, b] = mine.as_slice()
            && let (Some(x), Some(y)) = (a.at, b.at)
        {
            // One towards each end, the one towards the start first.
            let (low, high) = if a.end == 0 { (x, y) } else { (y, x) };
            if a.end == b.end || !(low.s < high.s) {
                return Err(runs_over(a.before));
            }
        }
    }
    Ok(found)
}

/// The face of side `k` at `s` along the blend.
pub(super) fn face_at(faces: [FaceId; 2], crossings: &[Crossing], k: usize, s: f64) -> FaceId {
    let mut face = faces.get(k).copied().unwrap_or(FaceId(u32::MAX));
    for c in crossings.iter().filter(|c| c.side == k) {
        match c.at {
            None => face = c.after,
            Some(at) => {
                if (c.end == 1 && s > at.s) || (c.end == 0 && s < at.s) {
                    face = c.after;
                }
            }
        }
    }
    face
}

/// The places along the blend where a side changes face, in order.
pub(super) fn switches(crossings: &[Crossing]) -> Vec<f64> {
    let mut out: Vec<f64> = crossings.iter().filter_map(|c| c.at.map(|a| a.s)).collect();
    out.sort_by(f64::total_cmp);
    out
}

/// The surfaces `parts` (open, cubic along `u`, each beginning where the
/// one before it ends and through the same section there) as one surface:
/// the knots of each after the first, past a triple knot at the joint.
pub(super) fn join(parts: &[NurbsSurface]) -> Option<NurbsSurface> {
    let first = parts.first()?;
    let (degree, degree_v) = first.degrees();
    let (_, count_v) = first.counts();
    let knots_v = first.knots_v().to_vec();
    let mut knots_u: Vec<f64> = Vec::new();
    let mut ctrl = Vec::new();
    let mut weights = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        if part.degrees() != (degree, degree_v)
            || part.counts().1 != count_v
            || part.knots_v() != knots_v.as_slice()
        {
            return None;
        }
        let (count_u, _) = part.counts();
        let knots = part.knots_u();
        let last = i + 1 == parts.len();
        // The first part keeps its start; every part its end only when
        // it is the last (a joint keeps `degree` copies).
        let from = if i == 0 { 0 } else { degree + 1 };
        let to = if last {
            knots.len()
        } else {
            knots.len().checked_sub(1)?
        };
        if let (Some(previous), Some(start)) = (knots_u.last(), knots.get(degree))
            && i > 0
            && previous != start
        {
            return None;
        }
        knots_u.extend_from_slice(knots.get(from..to)?);
        let skip = if i == 0 { 0 } else { count_v };
        let rows = count_u.checked_mul(count_v)?;
        ctrl.extend_from_slice(part.control_points().get(skip..rows)?);
        weights.extend_from_slice(part.weights().get(skip..rows)?);
    }
    let count_u = ctrl.len() / count_v.max(1);
    NurbsSurface::new(
        degree, degree_v, knots_u, knots_v, count_u, count_v, ctrl, weights,
    )
    .ok()
}

/// After the blend is spliced in: the borders (`borders`, per side, as
/// [`super::splice`] made them, each on its side's face) cut at the
/// switches into pieces on the faces they lie on, the crossed edges
/// shortened to the switches or taken whole, and the loops of the faces
/// touched linked again. The pieces of both borders, each with its face.
pub(super) fn carry(
    body: &mut Body,
    edge: EdgeId,
    blend_face: FaceId,
    borders: [EdgeId; 2],
    faces: [FaceId; 2],
    crossings: &[Crossing],
    tol: f64,
) -> Result<Vec<(EdgeId, FaceId)>, BlendError> {
    let mut pieces = Vec::new();
    for k in 0..2 {
        let border = *borders.get(k).ok_or_else(lost)?;
        let face = *faces.get(k).ok_or_else(lost)?;
        let runs_over = || refuse(edge, Some(face), Reason::FreeRunsOver);
        let mine: Vec<Crossing> = crossings.iter().copied().filter(|c| c.side == k).collect();
        if mine.is_empty() {
            pieces.push((border, face));
            continue;
        }
        // Which way the face's loop runs along the border.
        let flag = coedge_in(body, face, border).ok_or_else(lost)?.reversed;
        let data = body.edge(border).ok_or_else(lost)?.clone();
        let mut cuts: Vec<(f64, Crossing)> = mine
            .iter()
            .filter_map(|c| c.at.map(|a| (a.s, *c)))
            .collect();
        cuts.sort_by(|a, b| a.0.total_cmp(&b.0));
        let (lo, hi) = data.range;
        let mut mine_pieces: Vec<(EdgeId, FaceId)> = Vec::new();
        let (mut from, mut start) = (lo, data.start);
        let mut switched: Vec<(VertexId, EdgeId)> = Vec::new();
        for (s, c) in &cuts {
            let Some(at) = c.at else { continue };
            if !(*s > from && *s < hi) {
                return Err(runs_over());
            }
            let point = data.curve.eval(*s);
            if point.distance(at.point) > tol {
                return Err(runs_over());
            }
            // The crossed edge ends at the switch; the vertex inside has
            // nothing else left (the end's walk took the edge before it).
            if others_at(body, c.inside, c.smooth) {
                return Err(runs_over());
            }
            let x = super::add_vertex(body, point)?;
            super::shorten(body, c.smooth, c.inside, x, at.t)?;
            switched.push((x, c.smooth));
            let id = if mine_pieces.is_empty() {
                border
            } else {
                EdgeId(u32::try_from(body.edges.len()).map_err(|_| lost())?)
            };
            let piece = Edge {
                curve: data.curve.clone(),
                range: (from, *s),
                start,
                end: x,
            };
            put(body, id, piece)?;
            mine_pieces.push((id, face_at(faces, crossings, k, 0.5 * (from + s))));
            from = *s;
            start = x;
        }
        let id = if mine_pieces.is_empty() {
            border
        } else {
            EdgeId(u32::try_from(body.edges.len()).map_err(|_| lost())?)
        };
        put(
            body,
            id,
            Edge {
                curve: data.curve.clone(),
                range: (from, hi),
                start,
                end: data.end,
            },
        )?;
        mine_pieces.push((id, face_at(faces, crossings, k, 0.5 * (from + hi))));
        // Each piece and the edge it was cut against meet only at the
        // switch: proven apart away from it (`stays_on` leaves out every
        // edge that shares a vertex with the border, and the edge was
        // sampled, so a re-entry between two samples would go unseen).
        for (x, smooth) in &switched {
            for (piece, _) in &mine_pieces {
                let touches = body
                    .edge(*piece)
                    .is_some_and(|e| e.start == *x || e.end == *x);
                if touches && !apart_beyond(body, *piece, *smooth, *x, tol).unwrap_or(false) {
                    return Err(runs_over());
                }
            }
        }
        // An edge inside all along goes, with the face before it if that
        // has nothing left.
        for c in mine.iter().filter(|c| c.at.is_none()) {
            let Some(data) = body.edge(c.smooth) else {
                return Err(lost());
            };
            let (a, b) = (data.start, data.end);
            if others_at(body, a, c.smooth) || others_at(body, b, c.smooth) {
                return Err(runs_over());
            }
            super::take_whole(body, c.smooth);
        }
        // The blend face runs along the pieces where it ran along the
        // border.
        let own = coedge_in(body, blend_face, border).ok_or_else(lost)?;
        let mut along: Vec<Coedge> = mine_pieces
            .iter()
            .map(|(e, _)| Coedge {
                edge: *e,
                reversed: own.reversed,
            })
            .collect();
        if own.reversed {
            along.reverse();
        }
        replace(body, blend_face, border, &along)?;
        // The faces the border lay and lies on, linked again.
        remove_coedge(body, face, border);
        let mut touched: Vec<FaceId> = vec![face];
        for c in &mine {
            if !touched.contains(&c.after) {
                touched.push(c.after);
            }
        }
        for f in touched {
            let extra: Vec<Coedge> = mine_pieces
                .iter()
                .filter(|(_, on)| *on == f)
                .map(|(e, _)| Coedge {
                    edge: *e,
                    reversed: flag,
                })
                .collect();
            relink(body, f, &extra).map_err(|()| runs_over())?;
        }
        pieces.extend(mine_pieces);
    }
    Ok(pieces)
}

/// How close to the switch two curves that cross there may come to each
/// other apart from it: what `stays_on` holds borders to.
const APART: f64 = 10.0;

/// Whether the edges `a` and `b`, which cross at their common vertex
/// `at`, come within [`APART`] tolerances of each other nowhere else:
/// proven (`geom::certify::curves_apart`) on their ranges less the
/// stretch within `rho` of the vertex. Two curves that cross at the angle
/// θ are, to first order, `s · sin θ` apart at the distance `s` from the
/// crossing, so `rho = 2 · APART · tol / sin θ` keeps out only the stretch
/// where they are that close by crossing (twice: room for their bending);
/// a second meeting inside it would need either curve to turn by θ
/// within `rho`. `None` (not shown apart) where the crossing is tangent,
/// or `rho` takes more than half of either edge.
fn apart_beyond(body: &Body, a: EdgeId, b: EdgeId, at: VertexId, tol: f64) -> Option<bool> {
    use craft3d_geom::certify::{Apart, CurveBound, curves_apart};
    let x = body.vertex(at)?.point;
    let (ea, eb) = (body.edge(a)?, body.edge(b)?);
    // The unit tangent of each at the vertex, and which end it is.
    let leaving = |e: &Edge| -> Option<(Vec3, bool)> {
        let at_start = e.start == at;
        let t = if at_start { e.range.0 } else { e.range.1 };
        let (_, d) = e.curve.derivatives(t);
        Some((d.normalized()?, at_start))
    };
    let ((ta, sa), (tb, sb)) = (leaving(ea)?, leaving(eb)?);
    let sine = ta.cross(tb).length();
    if !(sine > 0.0) {
        return None;
    }
    let rho = 2.0 * APART * tol / sine;
    // The range of `e` less the stretch from its end at the vertex out to
    // where it first lies `rho` from it (64 steps, then bisection).
    let trimmed = |e: &Edge, at_start: bool| -> Option<(f64, f64)> {
        let (lo, hi) = e.range;
        let param = |f: f64| {
            if at_start {
                lo + (hi - lo) * f
            } else {
                hi - (hi - lo) * f
            }
        };
        let far = |f: f64| e.curve.eval(param(f)).distance(x) >= rho;
        let first = (1..=64).map(|i| f64::from(i) / 64.0).find(|f| far(*f))?;
        let (mut inner, mut outer) = (first - 1.0 / 64.0, first);
        for _ in 0..60 {
            let m = 0.5 * (inner + outer);
            if far(m) {
                outer = m;
            } else {
                inner = m;
            }
        }
        if outer > 0.5 {
            return None;
        }
        let cut = param(outer);
        Some(if at_start { (cut, hi) } else { (lo, cut) })
    };
    let (ra, rb) = (trimmed(ea, sa)?, trimmed(eb, sb)?);
    let (ca, cb) = (CurveBound::new(&ea.curve), CurveBound::new(&eb.curve));
    Some(curves_apart(&ca, ra, &cb, rb, APART * tol, super::PROOF_PIECES) == Apart::Yes)
}

/// Whether an edge other than `edge` still ends at `vertex` (one taken
/// whole ends nowhere).
fn others_at(body: &Body, vertex: VertexId, edge: EdgeId) -> bool {
    !edges_at(body, vertex, edge).is_empty()
}

/// Sets edge `id` to `data`, adding it when it is the next new one.
fn put(body: &mut Body, id: EdgeId, data: Edge) -> Result<(), BlendError> {
    let index = id.0 as usize;
    if index == body.edges.len() {
        body.edges.push(data);
    } else {
        *body.edges.get_mut(index).ok_or_else(lost)? = data;
    }
    Ok(())
}

/// The coedge of `edge` in the loops of `face`.
fn coedge_in(body: &Body, face: FaceId, edge: EdgeId) -> Option<Coedge> {
    body.face(face)?
        .loops
        .iter()
        .filter_map(|l| body.loop_(*l))
        .flat_map(|l| l.coedges.iter())
        .find(|c| c.edge == edge)
        .copied()
}

/// Puts `along` in place of the coedge of `edge` in the loops of `face`.
fn replace(
    body: &mut Body,
    face: FaceId,
    edge: EdgeId,
    along: &[Coedge],
) -> Result<(), BlendError> {
    let loops = body.face(face).ok_or_else(lost)?.loops.clone();
    for l in loops {
        let ring = body.loops.get_mut(l.0 as usize).ok_or_else(lost)?;
        if let Some(at) = ring.coedges.iter().position(|c| c.edge == edge) {
            ring.coedges.splice(at..=at, along.iter().copied());
            return Ok(());
        }
    }
    Err(lost())
}

fn remove_coedge(body: &mut Body, face: FaceId, edge: EdgeId) {
    let loops = body.face(face).map(|f| f.loops.clone()).unwrap_or_default();
    for l in loops {
        if let Some(ring) = body.loops.get_mut(l.0 as usize) {
            ring.coedges.retain(|c| c.edge != edge);
        }
    }
}

/// Links the loops of `face` again from the coedges they hold (those of
/// edges taken whole left out) and `extra`: from the end of each coedge
/// the one that begins there. A face with none left goes (out of its
/// shell, no loops); `Err` where a vertex has two ways on or a loop does
/// not close, or the loops come out different in number.
fn relink(body: &mut Body, face: FaceId, extra: &[Coedge]) -> Result<(), ()> {
    let loops = body.face(face).ok_or(())?.loops.clone();
    let mut all: Vec<Coedge> = Vec::new();
    for l in &loops {
        let ring = body.loop_(*l).ok_or(())?;
        for c in &ring.coedges {
            let data = body.edge(c.edge).ok_or(())?;
            if data.start.0 != u32::MAX
                && !all
                    .iter()
                    .any(|x| x.edge == c.edge && x.reversed == c.reversed)
            {
                all.push(*c);
            }
        }
    }
    for c in extra {
        if !all
            .iter()
            .any(|x| x.edge == c.edge && x.reversed == c.reversed)
        {
            all.push(*c);
        }
    }
    if all.is_empty() {
        for shell in &mut body.shells {
            shell.faces.retain(|f| *f != face);
        }
        for l in &loops {
            if let Some(ring) = body.loops.get_mut(l.0 as usize) {
                ring.coedges.clear();
            }
        }
        if let Some(f) = body.faces.get_mut(face.0 as usize) {
            f.loops.clear();
        }
        return Ok(());
    }
    let ends = |c: &Coedge| -> Option<(VertexId, VertexId)> {
        let data = body.edge(c.edge)?;
        Some(if c.reversed {
            (data.end, data.start)
        } else {
            (data.start, data.end)
        })
    };
    let mut used = vec![false; all.len()];
    let mut rings: Vec<Vec<Coedge>> = Vec::new();
    while let Some(first) = used.iter().position(|u| !u) {
        let mut ring = Vec::new();
        let mut at = first;
        let (begin, _) = ends(all.get(first).ok_or(())?).ok_or(())?;
        for _ in 0..=all.len() {
            *used.get_mut(at).ok_or(())? = true;
            let here = *all.get(at).ok_or(())?;
            ring.push(here);
            let (_, end) = ends(&here).ok_or(())?;
            if end == begin {
                break;
            }
            let next: Vec<usize> = (0..all.len())
                .filter(|i| {
                    !used.get(*i).copied().unwrap_or(true)
                        && all.get(*i).and_then(ends).is_some_and(|(s, _)| s == end)
                })
                .collect();
            let [n] = next.as_slice() else {
                return Err(());
            };
            at = *n;
        }
        let closed = ring.last().and_then(ends).is_some_and(|(_, e)| e == begin);
        if !closed {
            return Err(());
        }
        rings.push(ring);
    }
    if rings.len() != loops.len() {
        return Err(());
    }
    // Each ring into the loop it shares most with, in the loops' order.
    let mut order: Vec<Option<Vec<Coedge>>> = vec![None; loops.len()];
    for ring in rings {
        let best = loops
            .iter()
            .enumerate()
            .filter(|(i, _)| order.get(*i).is_some_and(Option::is_none))
            .max_by_key(|(_, l)| {
                body.loop_(**l).map_or(0, |old| {
                    ring.iter()
                        .filter(|c| old.coedges.iter().any(|o| o.edge == c.edge))
                        .count()
                })
            })
            .map(|(i, _)| i)
            .ok_or(())?;
        *order.get_mut(best).ok_or(())? = Some(ring);
    }
    for (l, ring) in loops.iter().zip(order) {
        let ring = ring.ok_or(())?;
        body.loops.get_mut(l.0 as usize).ok_or(())?.coedges = ring;
    }
    Ok(())
}

/// Whether the section `section` at a switch holds for `sides` too: its
/// feet on their surfaces, within a quarter of the tolerance; with its
/// parameters there.
pub(super) fn moved_onto(section: &Section, sides: [Side<'_>; 2], tol: f64) -> Option<Section> {
    let mut out = *section;
    for k in 0..2 {
        let side = sides.get(k)?;
        let foot = *section.feet.get(k)?;
        let start = side.surface.project(foot).ok()?;
        let (uv, on) = side.foot(foot, start)?;
        if on.distance(foot) > 0.25 * tol {
            return None;
        }
        *out.params.get_mut(k)? = uv;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use craft3d_geom::{Curve3, Line};
    use craft3d_topo::{BodyKind, Vertex};

    /// A line along x from the origin and a curve that leaves the origin
    /// at about 27° and comes back down to `gap` above the line at x = 0.7
    /// (a grazing dip past the crossing), as two edges of one vertex.
    fn dipping(gap: f64) -> Body {
        let params: Vec<f64> = (0..=40).map(|i| f64::from(i) / 40.0).collect();
        let points: Vec<Vec3> = params
            .iter()
            .map(|t| {
                let y = 0.5 * t * (t - 0.7).powi(2) / 0.49 + gap * t / 0.7;
                Vec3::new(*t, y, 0.0)
            })
            .collect();
        let curve = crate::shared::curve_through(&params, &points).unwrap();
        let mut body = Body::new(BodyKind::Sheet);
        for p in [
            Vec3::ZERO,
            *points.last().unwrap(),
            Vec3::new(1.2, 0.0, 0.0),
        ] {
            body.vertices.push(Vertex { point: p });
        }
        body.edges.push(Edge {
            curve: Curve3::Nurbs(curve),
            range: (0.0, 1.0),
            start: VertexId(0),
            end: VertexId(1),
        });
        body.edges.push(Edge {
            curve: Curve3::Line(Line::new(Vec3::ZERO, Vec3::X).unwrap()),
            range: (0.0, 1.2),
            start: VertexId(0),
            end: VertexId(2),
        });
        body
    }

    /// Two edges that cross at their shared vertex are shown apart only
    /// when they do not come back near each other elsewhere: a dip to
    /// within ten tolerances is not apart, one well clear of it is; edges
    /// tangent at the vertex are not shown apart.
    #[test]
    fn edges_crossing_at_a_switch_are_proven_apart_away_from_it() {
        let tol = 1e-6;
        for (gap, apart) in [(2e-6, false), (8e-6, false), (1e-4, true), (1e-2, true)] {
            let body = dipping(gap);
            let found = apart_beyond(&body, EdgeId(0), EdgeId(1), VertexId(0), tol);
            assert_eq!(found, Some(apart), "a dip to {gap}");
        }
        let mut body = dipping(1e-2);
        let along = body.edges[0].curve.derivatives(0.0).1;
        body.edges[1].curve = Curve3::Line(Line::new(Vec3::ZERO, along).unwrap());
        assert_eq!(
            apart_beyond(&body, EdgeId(0), EdgeId(1), VertexId(0), tol),
            None
        );
    }
}
