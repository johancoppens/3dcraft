//! What a body closed again around new surfaces is held to beyond the
//! checks of each edge and each flat or round face (`super`): faces that
//! reached or passed something that is not theirs
//! (`docs/design/face-operations.md` › *A face moved up to or past what
//! ends its neighbours*).
//!
//! 1. **Two edges of one face that meet where they did not** ([`crossed`]).
//!    A face that moves past the face that ends a neighbour of it leaves
//!    that neighbour with loops that run across each other: the rim of a
//!    rod's end cap moved past the block the rod stands out of crosses the
//!    edges in which the rod's wall ends on the block. The flat faces whose
//!    edges are lines and circles were held to that by their outlines
//!    (`super::outline_is_simple`); this holds every face next to an edge
//!    made again, on any surface and with any curves, in space: an edge
//!    made again and another edge of the same face that come within half
//!    the tolerance of each other away from a vertex they share, where the
//!    edges they were did not. Only the edges whose hulls reach the edges
//!    made again are followed, and their steps are swept along one axis,
//!    so that a step is held only against the steps whose boxes reach it.
//! 2. **A vertex a moved face went through** ([`swept`]). A vertex that is
//!    not on a face that moved, and lies on the other side of that face's
//!    surface than it did (or on it), with the face over it: beside the
//!    edges that end in such a vertex the body is asked whether its faces
//!    are still skin, as beside the edges made again (`super::buried`).
//!    The question decides, the vertex only says where to ask: a vertex
//!    under an overhang the face grew into is passed by its surface, and
//!    is fine. A largest move that fits is said only where the vertex is
//!    the first thing the face can have met ([`first_met`]).
//! 3. **An edge made again that goes through a face not its own**
//!    ([`pierced`]). The rim of a pocket's floor moved down a pocket that
//!    widens runs out through the sides of the block: where an edge made
//!    again crosses the surface of a face that shares no vertex with it,
//!    on that face, the edge is cut into stretches, and the same question
//!    is asked beside the middle of each (the middle of the whole edge,
//!    which is where it was asked before, can lie on the right side).
//!
//!    2 and 3 ask how far a point is from a face's surface, of every
//!    vertex and every step of an edge near it, and so only of faces
//!    where that is a closed form ([`analytic`]): a freeform face that
//!    moves through a vertex, and an edge that runs out through a freeform
//!    face, are not looked for.
//! 4. **A body turned inside out** ([`inside_out`]). A shell that enclosed
//!    a volume and is measured to enclose a negative one. Measured only
//!    where a face that does not touch a moved face lies in the box the
//!    moved face went through ([`Swept::near`]): the measure costs what
//!    `document.measure` costs.
//!
//! **Cost.** 1 to 3 are counted against the operation's work bound
//! (`craft3d_geom::allowance`: the evaluations of freeform curves count
//! themselves, each pair looked at closely is charged besides), and have
//! no cap of their own: a check that cannot end within the bound refuses
//! the operation as needing too much work ([`Spent`]), which says so, and
//! never stops without a word. The measure of 4 is not counted (it has
//! its own bound per face, and is taken only where a far face lay in the
//! way). Measured on a plate whose top has 240 wavy spline edges, its top
//! offset: `docs/design/face-operations.md` › *What the checks cost*.
//!
//! What these do not find: a moved face that passes a face far from it
//! where no vertex lies in its way, no edge made again goes through it
//! and the volume stays positive (the bottom of a block moved up into a
//! groove of its top, a dimple, the point of a drilled hole). That needs
//! the faces' intersection. The mesh the regeneration looks for
//! self-intersections on finds it only from some depth on: a few
//! thousandths of a millimetre into a smooth face it is still built
//! wrong, also through the commands (owed; pinned as known in
//! `crates/kernel/tests/hunt_offset.rs`).

use super::distance;
use craft3d_geom::{Curve3, Surface};
use craft3d_math::{Bbox3, Tolerance, Vec3};
use craft3d_topo::{Body, Edge, EdgeId, FaceId, VertexId};
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::PI;

/// Steps a circle or an ellipse is followed in at most (a whole turn), and
/// a spline per span of its knots and in all.
const CONIC_STEPS: usize = 16;
const SPAN_STEPS: usize = 4;
const MOST_STEPS: usize = 256;
/// Steps of the search for where two edges are nearest, and of a point's
/// nearest point on an edge.
const NEAREST_STEPS: usize = 24;
/// Places between a vertex two edges share and a place where they meet at
/// which they are asked whether they part.
const BETWEEN: usize = 8;
/// How near two edges of one face come where they meet, as a share of the
/// tolerance: edges a tolerance apart bound a face that is thin, which is
/// a face (`craft3d_topo`'s rule of simple loops), and crossing edges are
/// no distance apart; half of it tells the two apart whatever rounding
/// does to an edge that stops exactly a tolerance short.
const MEET: f64 = 0.5;

/// An edge followed in steps: its parameters and points, how far the
/// curve may be from the chords, and the box of the points.
struct Path {
    params: Vec<f64>,
    points: Vec<Vec3>,
    slack: f64,
    lo: Vec3,
    hi: Vec3,
}

/// The knots of a spline strictly inside `range`, each once.
fn spans(curve: &Curve3, range: (f64, f64)) -> usize {
    let Curve3::Nurbs(spline) = curve else {
        return 1;
    };
    let mut count = 1_usize;
    let mut last = f64::NAN;
    for &knot in spline.knots() {
        if knot > range.0 && knot < range.1 && knot != last {
            count = count.saturating_add(1);
            last = knot;
        }
    }
    count
}

fn path(edge: &Edge) -> Option<Path> {
    let (a, b) = edge.range;
    if !(a.is_finite() && b.is_finite() && b > a) {
        return None;
    }
    let steps = match &edge.curve {
        Curve3::Line(_) => 1,
        Curve3::Circle(_) | Curve3::Ellipse(_) => {
            let turn = ((b - a) / (PI / 8.0)).ceil();
            if turn.is_finite() && turn >= 1.0 {
                (turn as usize).min(CONIC_STEPS)
            } else {
                1
            }
        }
        Curve3::Nurbs(_) => spans(&edge.curve, edge.range)
            .saturating_mul(SPAN_STEPS)
            .clamp(8, MOST_STEPS),
    };
    let mut params = Vec::with_capacity(steps + 1);
    let mut points = Vec::with_capacity(steps + 1);
    for k in 0..=steps {
        let t = if k == steps {
            b
        } else {
            a + (b - a) * k as f64 / steps as f64
        };
        let p = edge.curve.eval(t);
        if !p.is_finite() {
            return None;
        }
        params.push(t);
        points.push(p);
    }
    // How far the curve is from a chord, at its middle and twice that for
    // what lies between.
    let mut slack = 0.0_f64;
    if !matches!(edge.curve, Curve3::Line(_)) {
        for (ts, ps) in params.windows(2).zip(points.windows(2)) {
            let ([t0, t1], [p0, p1]) = (ts, ps) else {
                continue;
            };
            let off = edge.curve.eval(0.5 * (t0 + t1)).distance(p0.lerp(*p1, 0.5));
            if !off.is_finite() {
                return None;
            }
            slack = slack.max(2.0 * off);
        }
    }
    let first = *points.first()?;
    let (lo, hi) = points
        .iter()
        .fold((first, first), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
    Some(Path {
        params,
        points,
        slack,
        lo,
        hi,
    })
}

/// Whether the boxes of two paths come within `reach` of each other.
fn boxes_near(a: &Path, b: &Path, reach: f64) -> bool {
    a.lo.x <= b.hi.x + reach
        && b.lo.x <= a.hi.x + reach
        && a.lo.y <= b.hi.y + reach
        && b.lo.y <= a.hi.y + reach
        && a.lo.z <= b.hi.z + reach
        && b.lo.z <= a.hi.z + reach
}

/// Where the segments `p0 p1` and `q0 q1` are nearest: the share along
/// each, and how far apart they are there.
fn segments(p0: Vec3, p1: Vec3, q0: Vec3, q1: Vec3) -> (f64, f64, f64) {
    let (d1, d2, r) = (p1 - p0, q1 - q0, p0 - q0);
    let (a, e, f) = (d1.dot(d1), d2.dot(d2), d2.dot(r));
    let (s, t);
    if a <= 0.0 && e <= 0.0 {
        (s, t) = (0.0, 0.0);
    } else if a <= 0.0 {
        s = 0.0;
        t = (f / e).clamp(0.0, 1.0);
    } else {
        let c = d1.dot(r);
        if e <= 0.0 {
            t = 0.0;
            s = (-c / a).clamp(0.0, 1.0);
        } else {
            let b = d1.dot(d2);
            let denominator = a * e - b * b;
            let mut along = if denominator > 0.0 {
                ((b * f - c * e) / denominator).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let mut across = (b * along + f) / e;
            if across < 0.0 {
                across = 0.0;
                along = (-c / a).clamp(0.0, 1.0);
            } else if across > 1.0 {
                across = 1.0;
                along = ((b - c) / a).clamp(0.0, 1.0);
            }
            (s, t) = (along, across);
        }
    }
    let gap = (p0 + d1 * s).distance(q0 + d2 * t);
    (s, t, gap)
}

/// The parameters at which the edges `a` and `b` are nearest each other,
/// from `(s, t)`, and how far apart they are there: Gauss-Newton steps on
/// the vector between them, held to the edges' ranges.
fn nearest_pair(a: &Edge, b: &Edge, mut s: f64, mut t: f64, tol: f64) -> (f64, f64, f64) {
    // The nearest seen: a step where the curves run nearly side by side
    // can overshoot.
    let mut best = (s, t, f64::INFINITY);
    for _ in 0..NEAREST_STEPS {
        let (pa, da) = a.curve.derivatives(s);
        let (pb, db) = b.curve.derivatives(t);
        let r = pa - pb;
        let apart = r.length();
        if apart < best.2 {
            best = (s, t, apart);
        }
        let (aa, ab, bb) = (da.dot(da), da.dot(db), db.dot(db));
        let (ga, gb) = (da.dot(r), -db.dot(r));
        let det = aa * bb - ab * ab;
        let (ds, dt) = if det.is_finite() && det > 1e-6 * aa * bb {
            (-(bb * ga + ab * gb) / det, -(ab * ga + aa * gb) / det)
        } else {
            // Side by side there: each to the other's nearest point, half
            // way.
            (
                if aa > 0.0 { -0.5 * ga / aa } else { 0.0 },
                if bb > 0.0 { -0.5 * gb / bb } else { 0.0 },
            )
        };
        if !(ds.is_finite() && dt.is_finite()) {
            break;
        }
        let (was_s, was_t) = (s, t);
        s = (s + ds).clamp(a.range.0, a.range.1);
        t = (t + dt).clamp(b.range.0, b.range.1);
        let moved = (s - was_s).abs() * aa.sqrt() + (t - was_t).abs() * bb.sqrt();
        if moved <= 1e-3 * tol {
            break;
        }
    }
    let apart = a.curve.eval(s).distance(b.curve.eval(t));
    if apart < best.2 { (s, t, apart) } else { best }
}

/// How far `point` is from the edge `b`, followed as `path`.
fn gap(point: Vec3, b: &Edge, path: &Path) -> f64 {
    let mut best = (f64::INFINITY, b.range.0);
    for (ts, ps) in path.params.windows(2).zip(path.points.windows(2)) {
        let ([t0, t1], [p0, p1]) = (ts, ps) else {
            continue;
        };
        let (_, share, off) = segments(point, point, *p0, *p1);
        if off < best.0 {
            best = (off, t0 + (t1 - t0) * share);
        }
    }
    let mut t = best.1;
    for _ in 0..NEAREST_STEPS {
        let (p, d) = b.curve.derivatives(t);
        let dd = d.dot(d);
        if dd <= 0.0 || !dd.is_finite() {
            break;
        }
        let step = (point - p).dot(d) / dd;
        if !step.is_finite() {
            break;
        }
        let was = t;
        t = (t + step).clamp(b.range.0, b.range.1);
        if (t - was).abs() * dd.sqrt() <= 1e-12 * (1.0 + best.0) {
            break;
        }
    }
    let found = b.curve.eval(t).distance(point);
    if found.is_finite() {
        found.min(best.0)
    } else {
        best.0
    }
}

/// Whether the edges `a` and `b` come within `tol` (the share [`MEET`] of
/// the tolerance) of each other at a place that is not a vertex they
/// share: one between which and each shared end of `a` the two part by
/// more than `tol`.
fn meet(a: &Edge, pa: &Path, b: &Edge, pb: &Path, tol: f64) -> bool {
    let reach = pa.slack + pb.slack + tol;
    if !boxes_near(pa, pb, reach) {
        return false;
    }
    (0..pa.points.len().saturating_sub(1))
        .any(|i| (0..pb.points.len().saturating_sub(1)).any(|j| look(a, pa, i, b, pb, j, tol)))
}

/// [`meet`] for one step of each edge (`i` of `a`, `j` of `b`): whether the
/// two come within `tol` of each other at a place found from where those
/// steps are nearest, that is not a vertex the edges share.
fn look(a: &Edge, pa: &Path, i: usize, b: &Edge, pb: &Path, j: usize, tol: f64) -> bool {
    let (Some([s0, s1]), Some([p0, p1]), Some([t0, t1]), Some([q0, q1])) = (
        pa.params.get(i..i.saturating_add(2)),
        pa.points.get(i..i.saturating_add(2)),
        pb.params.get(j..j.saturating_add(2)),
        pb.points.get(j..j.saturating_add(2)),
    ) else {
        return false;
    };
    let (along, across, off) = segments(*p0, *p1, *q0, *q1);
    if off.is_nan() || off > pa.slack + pb.slack + tol {
        return false;
    }
    let (s, _, apart) = nearest_pair(a, b, s0 + (s1 - s0) * along, t0 + (t1 - t0) * across, tol);
    if apart.is_nan() || apart > tol {
        return false;
    }
    // The vertex's own: the edges never part between it and here.
    let own = [(a.start, a.range.0), (a.end, a.range.1)]
        .into_iter()
        .filter(|(vertex, _)| *vertex == b.start || *vertex == b.end)
        .any(|(_, end)| {
            !(1..=BETWEEN).any(|k| {
                let at = end + (s - end) * k as f64 / (BETWEEN + 1) as f64;
                gap(a.curve.eval(at), b, pb) > tol
            })
        });
    !own
}

/// The work bound of the operation was reached while a check ran
/// ([`craft3d_geom::allowance`]): the operation is refused as needing too
/// much work, which says so; a check never stops without a word.
pub(super) struct Spent;

/// One step of an edge of a face, for the sweep of [`crossed`]: the edge,
/// the step's place along its path, and the step's box grown by how far
/// the curve may be from its chord and half of how near two edges meet.
#[derive(Clone, Copy)]
struct Step {
    edge: EdgeId,
    at: usize,
    remade: bool,
    lo: Vec3,
    hi: Vec3,
}

/// A box that holds the edge, from points that take no evaluation of its
/// curve along the way: a line's ends, a circle's or an ellipse's centre
/// and largest radius, a spline's control points (its hull; the whole
/// curve's, also where the edge is a part of it). `None` for an edge whose
/// box is not finite.
fn hull(edge: &Edge) -> Option<(Vec3, Vec3)> {
    let (lo, hi) = match &edge.curve {
        Curve3::Line(_) => {
            let (a, b) = (edge.curve.eval(edge.range.0), edge.curve.eval(edge.range.1));
            (a.min(b), a.max(b))
        }
        Curve3::Circle(circle) => {
            let (centre, r) = (circle.frame().origin(), circle.radius());
            (centre - Vec3::new(r, r, r), centre + Vec3::new(r, r, r))
        }
        Curve3::Ellipse(ellipse) => {
            let (a, b) = ellipse.semi_axes();
            let (centre, r) = (ellipse.frame().origin(), a.max(b));
            (centre - Vec3::new(r, r, r), centre + Vec3::new(r, r, r))
        }
        Curve3::Nurbs(spline) => {
            let mut points = spline.control_points().iter();
            let first = *points.next()?;
            points.fold((first, first), |(lo, hi), p| (lo.min(*p), hi.max(*p)))
        }
    };
    (lo.is_finite() && hi.is_finite()).then_some((lo, hi))
}

fn part(v: Vec3, axis: usize) -> f64 {
    match axis {
        0 => v.x,
        1 => v.y,
        _ => v.z,
    }
}

/// The first edge of `remade` that comes within half the tolerance
/// ([`MEET`]) of another edge of a face they both bound in `made`, away
/// from a vertex they share, where the two edges they were in `body` did
/// not (an edge that is
/// new in `made` met nothing before): the face, the edge made again and
/// the other edge.
pub(super) fn crossed(
    body: &Body,
    made: &Body,
    uses: &[Vec<FaceId>],
    remade: &BTreeSet<EdgeId>,
    tolerance: &Tolerance,
) -> Result<Option<(FaceId, EdgeId, EdgeId)>, Spent> {
    let tol = MEET * tolerance.linear;
    let mut paths: BTreeMap<EdgeId, Option<Path>> = BTreeMap::new();
    // Pairs that met before the operation too: not looked at again.
    let mut old: BTreeSet<(EdgeId, EdgeId)> = BTreeSet::new();
    for face in super::faces_with(uses, remade) {
        let edges: BTreeSet<EdgeId> = made
            .face(face)
            .into_iter()
            .flat_map(|f| &f.loops)
            .filter_map(|l| made.loop_(*l))
            .flat_map(|l| l.coedges.iter().map(|c| c.edge))
            .collect();
        // The box round the edges made again, from their hulls: an edge
        // whose own hull does not reach it is not followed at all (a side
        // moved on a plate whose top has hundreds of slots follows the
        // top's one edge that moved and the few beside it).
        let mut around: Option<(Vec3, Vec3)> = None;
        for id in edges.iter().filter(|id| remade.contains(id)) {
            let Some((lo, hi)) = made.edge(*id).and_then(hull) else {
                continue;
            };
            around = Some(match around {
                Some((all_lo, all_hi)) => (all_lo.min(lo), all_hi.max(hi)),
                None => (lo, hi),
            });
        }
        let Some((around_lo, around_hi)) = around else {
            continue;
        };
        let reaches = |id: &EdgeId| {
            remade.contains(id)
                || made.edge(*id).and_then(hull).is_none_or(|(lo, hi)| {
                    (0..3).all(|k| {
                        part(lo, k) <= part(around_hi, k) + tol
                            && part(around_lo, k) <= part(hi, k) + tol
                    })
                })
        };
        // Every step of every such edge of the face, boxed.
        let mut steps: Vec<Step> = Vec::new();
        for id in edges.iter().filter(|id| reaches(id)) {
            if craft3d_geom::allowance::exhausted() {
                return Err(Spent);
            }
            let followed = paths
                .entry(*id)
                .or_insert_with(|| made.edge(*id).and_then(path));
            let Some(followed) = followed else {
                continue;
            };
            let grow = followed.slack + 0.5 * tol;
            let grow = Vec3::new(grow, grow, grow);
            let made_again = remade.contains(id);
            for (at, ends) in followed.points.windows(2).enumerate() {
                let [p0, p1] = ends else {
                    continue;
                };
                steps.push(Step {
                    edge: *id,
                    at,
                    remade: made_again,
                    lo: p0.min(*p1) - grow,
                    hi: p0.max(*p1) + grow,
                });
            }
        }
        // Swept along the axis the steps spread furthest along: a step is
        // held against the steps whose boxes reach it, and only a step of
        // an edge made again against a step of another edge.
        let Some(first) = steps.first().copied() else {
            continue;
        };
        let (lo, hi) = steps.iter().fold((first.lo, first.hi), |(lo, hi), s| {
            (lo.min(s.lo), hi.max(s.hi))
        });
        let axis = (0..3)
            .max_by(|a, b| (part(hi, *a) - part(lo, *a)).total_cmp(&(part(hi, *b) - part(lo, *b))))
            .unwrap_or(0);
        let others = [(axis + 1) % 3, (axis + 2) % 3];
        steps.sort_by(|a, b| {
            part(a.lo, axis)
                .total_cmp(&part(b.lo, axis))
                .then(a.edge.cmp(&b.edge))
                .then(a.at.cmp(&b.at))
        });
        let mut reaching: Vec<Step> = Vec::new();
        for step in &steps {
            reaching.retain(|other| part(other.hi, axis) >= part(step.lo, axis));
            for other in &reaching {
                if other.edge == step.edge
                    || !(step.remade || other.remade)
                    || others.iter().any(|k| {
                        part(other.hi, *k) < part(step.lo, *k)
                            || part(step.hi, *k) < part(other.lo, *k)
                    })
                {
                    continue;
                }
                // The edge made again first (of two, the lower number).
                let (a, b) = if step.remade && (!other.remade || step.edge < other.edge) {
                    (step, other)
                } else {
                    (other, step)
                };
                if old.contains(&(a.edge, b.edge)) {
                    continue;
                }
                // Counted, also for lines and circles, whose evaluations
                // are not: a face of many near edges ends at the bound.
                craft3d_geom::allowance::charge(1);
                if craft3d_geom::allowance::exhausted() {
                    return Err(Spent);
                }
                let (Some(ea), Some(eb), Some(Some(pa)), Some(Some(pb))) = (
                    made.edge(a.edge),
                    made.edge(b.edge),
                    paths.get(&a.edge),
                    paths.get(&b.edge),
                ) else {
                    continue;
                };
                if !look(ea, pa, a.at, eb, pb, b.at, tol) {
                    continue;
                }
                // Where they were: edges that met already are not this
                // operation's doing.
                let before = match (body.edge(a.edge), body.edge(b.edge)) {
                    (Some(ea), Some(eb)) => match (path(ea), path(eb)) {
                        (Some(pa), Some(pb)) => meet(ea, &pa, eb, &pb, tol),
                        // Not followed: it cannot be told, and says nothing.
                        _ => true,
                    },
                    _ => false,
                };
                if !before {
                    return Ok(Some((face, a.edge, b.edge)));
                }
                old.insert((a.edge, b.edge));
            }
            reaching.push(*step);
        }
    }
    Ok(None)
}

/// What the faces that moved went through ([`swept`]).
pub(super) struct Swept {
    /// The edges that end in a vertex a moved face passed with the face
    /// over it, each with that face.
    pub edges: BTreeMap<EdgeId, FaceId>,
    /// Whether a face that shares no vertex with a moved face lies in the
    /// box that face went through.
    pub near: bool,
    /// Whether the vertices passed are the first of the body each moved
    /// face can have met on its way, so that the largest move that fits,
    /// found by trying smaller ones, is where the face first touches
    /// something ([`first_met`]). Where they may not be, a smaller move
    /// can pass every check and still run into a smooth face no vertex
    /// stands for, and no limit is said.
    pub first: bool,
}

/// Whether two boxes share a point.
fn boxes_meet(a: &Bbox3, b: &Bbox3) -> bool {
    match (a.corners(), b.corners()) {
        (Some((alo, ahi)), Some((blo, bhi))) => {
            alo.x <= bhi.x
                && blo.x <= ahi.x
                && alo.y <= bhi.y
                && blo.y <= ahi.y
                && alo.z <= bhi.z
                && blo.z <= ahi.z
        }
        _ => false,
    }
}

/// Where the faces of a body lie: the box of each (none for a face that
/// cannot be boxed), made once for [`swept`] and [`pierced`]. The
/// vertices of a face are looked up only for a face whose box is in the
/// way of something: on a plate of two thousand holes, sets of them for
/// every face cost more than the checks.
pub(super) struct Layout {
    boxes: Vec<Option<Bbox3>>,
}

impl Layout {
    pub(super) fn of(made: &Body) -> Self {
        Layout {
            boxes: (0..made.faces.len())
                .map(|i| made.face_bbox(FaceId(super::index_u32(i))).ok())
                .collect(),
        }
    }
}

/// Halvings of the search for where an edge crosses a surface.
const CUT_STEPS: usize = 48;

/// Whether the distance from a point to `surface` is a closed form (a
/// plane, a cylinder, a cone, a sphere, a torus). [`swept`] and [`pierced`]
/// ask it of every vertex and every step of an edge near a face, which a
/// freeform face answers by a search each time: on a plate with 240 wavy
/// slots, `pierced` alone, asked of the slots' freeform walls too, took
/// longer than the whole offset of the plate's top had (the review of
/// 2026-11-14). So a freeform face is not asked: an edge that runs out
/// through one, and a vertex one moves through, are left to the mesh the
/// regeneration looks at, as before these checks.
fn analytic(surface: &Surface) -> bool {
    match surface {
        Surface::Plane(_)
        | Surface::Cylinder(_)
        | Surface::Cone(_)
        | Surface::Sphere(_)
        | Surface::Torus(_) => true,
        Surface::Bezier(_) | Surface::Nurbs(_) => false,
    }
}

/// The edges of `remade` that cross the surface of a face of `made` that
/// shares no vertex with them and lies on a surface with a closed form
/// ([`analytic`]), on that face (or where that cannot be told): per edge,
/// the middles of the stretches the crossings cut it into, and the first
/// face it goes through (the module documentation). Found from the edge
/// followed in steps: a stretch that goes in and out again between two
/// steps is not seen. Every pair of an edge and a face near it is counted
/// against the operation's work bound; reaching it is [`Spent`].
pub(super) fn pierced(
    made: &Body,
    layout: &Layout,
    uses: &[Vec<FaceId>],
    remade: &BTreeSet<EdgeId>,
    tolerance: &Tolerance,
) -> Result<BTreeMap<EdgeId, (Vec<f64>, FaceId)>, Spent> {
    let tol = tolerance.linear;
    let mut found = BTreeMap::new();
    for id in remade {
        let Some(edge) = made.edge(*id) else {
            continue;
        };
        let Some(followed) = path(edge) else {
            continue;
        };
        let reach = followed.slack + tol;
        let grow = Vec3::new(reach, reach, reach);
        let around = Bbox3::from_corners(followed.lo - grow, followed.hi + grow);
        let own = uses.get(id.0 as usize).map_or(&[][..], Vec::as_slice);
        let mut cuts: Vec<f64> = Vec::new();
        let mut through = None;
        for (index, bounds) in layout.boxes.iter().enumerate() {
            let face = FaceId(super::index_u32(index));
            if !bounds.as_ref().is_some_and(|b| boxes_meet(b, &around)) || own.contains(&face) {
                continue;
            }
            let Some(data) = made.face(face).filter(|f| analytic(&f.surface)) else {
                continue;
            };
            let vertices = super::face_vertices(made, face);
            if vertices.contains(&edge.start) || vertices.contains(&edge.end) {
                continue;
            }
            craft3d_geom::allowance::charge(1);
            if craft3d_geom::allowance::exhausted() {
                return Err(Spent);
            }
            let off = |t: f64| distance(&data.surface, edge.curve.eval(t)).map(|(d, _)| d);
            let sides: Vec<Option<f64>> = followed.params.iter().map(|t| off(*t)).collect();
            // Where the edge crosses the surface between two parameters on
            // either side of it, by halving.
            let crossing = |from: f64, to: f64, below: bool| {
                let (mut lo, mut hi) = (from, to);
                for _ in 0..CUT_STEPS {
                    let middle = 0.5 * (lo + hi);
                    match off(middle) {
                        Some(d) if (d < 0.0) == below => lo = middle,
                        Some(_) => hi = middle,
                        None => break,
                    }
                }
                0.5 * (lo + hi)
            };
            let mut places: Vec<f64> = Vec::new();
            for (ts, ds) in followed.params.windows(2).zip(sides.windows(2)) {
                let ([t0, t1], [Some(d0), Some(d1)]) = (ts, ds) else {
                    continue;
                };
                if (*d0 < 0.0 && *d1 > 0.0) || (*d0 > 0.0 && *d1 < 0.0) {
                    places.push(crossing(*t0, *t1, *d0 < 0.0));
                } else if followed.slack > 0.0 && *d0 != 0.0 && d0.abs().min(d1.abs()) <= reach {
                    // Both steps on one side and near the surface: a bend
                    // between them may dip through it and come back. Where
                    // it comes nearest (or goes farthest through), by
                    // thirds.
                    let side = d0.signum();
                    let depth = |t: f64| off(t).map(|d| side * d);
                    let (mut lo, mut hi) = (*t0, *t1);
                    for _ in 0..CUT_STEPS {
                        let (a, b) = (lo + (hi - lo) / 3.0, hi - (hi - lo) / 3.0);
                        match (depth(a), depth(b)) {
                            (Some(da), Some(db)) if da < db => hi = b,
                            (Some(_), Some(_)) => lo = a,
                            _ => break,
                        }
                    }
                    let dip = 0.5 * (lo + hi);
                    if depth(dip).is_some_and(|d| d < 0.0) {
                        places.push(crossing(*t0, dip, *d0 < 0.0));
                        places.push(crossing(dip, *t1, *d0 > 0.0));
                    }
                }
            }
            for at in places {
                let on = craft3d_topo::face_holds(made, data, tolerance, &[edge.curve.eval(at)])
                    .and_then(|held| held.first().copied())
                    .unwrap_or(true);
                if on {
                    cuts.push(at);
                    through.get_or_insert(face);
                }
            }
        }
        let Some(through) = through else {
            continue;
        };
        cuts.sort_by(f64::total_cmp);
        let mut ends = vec![edge.range.0];
        ends.extend(cuts);
        ends.push(edge.range.1);
        let middles: Vec<f64> = ends
            .windows(2)
            .filter_map(|w| match w {
                [a, b] if b > a => Some(0.5 * (a + b)),
                _ => None,
            })
            .collect();
        found.insert(*id, (middles, through));
    }
    Ok(found)
}

/// The vertices of `body` the faces of `surfaces` passed on their way to
/// where they are in `made`, as the edges that end in them, and whether a
/// face far from a moved one lies in its way (the module documentation).
/// A vertex both bodies have is looked at; one that is new in `made` was
/// nowhere before.
pub(super) fn swept(
    body: &Body,
    made: &Body,
    layout: &Layout,
    surfaces: &BTreeMap<FaceId, Surface>,
    tolerance: &Tolerance,
) -> Result<Swept, Spent> {
    let tol = tolerance.linear;
    let mut found = Swept {
        edges: BTreeMap::new(),
        near: false,
        first: true,
    };
    let boxes = &layout.boxes;
    for face in surfaces.keys() {
        let (Some(was), Some(is)) = (body.face(*face), made.face(*face)) else {
            continue;
        };
        let (Ok(from), Some(Some(to))) = (body.face_bbox(*face), boxes.get(face.0 as usize)) else {
            continue;
        };
        let (Some((flo, fhi)), Some((tlo, thi))) = (from.corners(), to.corners()) else {
            continue;
        };
        let way = Bbox3::from_corners(flo.min(tlo), fhi.max(thi));
        let own = super::face_vertices(made, *face);
        if !found.near {
            found.near = boxes.iter().enumerate().any(|(i, b)| {
                i != face.0 as usize
                    && b.as_ref().is_some_and(|b| boxes_meet(b, &way))
                    && super::face_vertices(made, FaceId(super::index_u32(i))).is_disjoint(&own)
            });
        }
        // Where its vertices went is asked of a face on a plane, a
        // cylinder, a cone, a sphere or a torus only ([`analytic`]).
        if !(analytic(&was.surface) && analytic(&is.surface)) {
            continue;
        }
        // The nearest of the vertices passed, as it lay before the face
        // (its distance, and which side of the face that was).
        let mut nearest: Option<(f64, f64)> = None;
        for (index, (old, new)) in body.vertices.iter().zip(&made.vertices).enumerate() {
            let vertex = VertexId(super::index_u32(index));
            if !(way.contains(old.point, tol) || way.contains(new.point, tol))
                || own.contains(&vertex)
            {
                continue;
            }
            craft3d_geom::allowance::charge(1);
            if craft3d_geom::allowance::exhausted() {
                return Err(Spent);
            }
            let (Some((d0, _)), Some((d1, out))) = (
                distance(&was.surface, old.point),
                distance(&is.surface, new.point),
            ) else {
                continue;
            };
            let passed = (d0 > tol && d1 <= tol) || (d0 < -tol && d1 >= -tol);
            if !passed {
                continue;
            }
            // Its foot on the face as it is now: on the face, or not to
            // be told (the question beside its edges decides either way).
            let foot = new.point - out * d1;
            let over = craft3d_topo::face_holds(made, is, tolerance, &[foot])
                .and_then(|held| held.first().copied())
                .unwrap_or(true);
            if !over {
                continue;
            }
            // The edges that end in it.
            for (at, edge) in made.edges.iter().enumerate() {
                if edge.start == vertex || edge.end == vertex {
                    found
                        .edges
                        .entry(EdgeId(super::index_u32(at)))
                        .or_insert(*face);
                }
            }
            if nearest.is_none_or(|(least, _)| d0.abs() < least) {
                nearest = Some((d0.abs(), d0.signum()));
            }
        }
        if let Some((least, side)) = nearest
            && !first_met(made, layout, *face, &was.surface, &way, least, side, tol)
        {
            found.first = false;
        }
    }
    Ok(found)
}

/// Whether nothing of the body lay nearer before the flat face `face`
/// (on `was` before it moved, its way the box `way`) than a vertex at
/// `least` on the side `side` of it: so that the face, moved toward that
/// vertex, meets nothing sooner.
///
/// Told only where it can be told from vertices: every face that shares
/// no vertex with `face` and whose box lies in its way has to be a plane
/// parallel to it or a cylinder along its normal (each is nearest the face
/// at its vertices), and none of their vertices may lie nearer than
/// `least`. Any other face in the way (a cone, whose point has no vertex:
/// a drilled hole's bottom; a ball, a ring, a freeform face, a face at a
/// slant) can be met between its vertices, where nothing looks: `false`,
/// and so is a face that moved and is no plane.
#[allow(clippy::too_many_arguments)]
fn first_met(
    made: &Body,
    layout: &Layout,
    face: FaceId,
    was: &Surface,
    way: &Bbox3,
    least: f64,
    side: f64,
    tol: f64,
) -> bool {
    let Surface::Plane(plane) = was else {
        return false;
    };
    let normal = plane.frame().z();
    let own = super::face_vertices(made, face);
    for (index, bounds) in layout.boxes.iter().enumerate() {
        if index == face.0 as usize || !bounds.as_ref().is_some_and(|b| boxes_meet(b, way)) {
            continue;
        }
        let vertices = super::face_vertices(made, FaceId(super::index_u32(index)));
        if !vertices.is_disjoint(&own) {
            continue;
        }
        let along = match made.faces.get(index).map(|f| &f.surface) {
            Some(Surface::Plane(other)) => other.frame().z(),
            Some(Surface::Cylinder(other)) => other.frame().z(),
            _ => return false,
        };
        if along.cross(normal).length() > super::SQUARE {
            return false;
        }
        for vertex in &vertices {
            let Some(point) = made.vertex(*vertex).map(|v| v.point) else {
                return false;
            };
            match distance(was, point) {
                Some((d, _)) if side * d >= least - tol => {}
                _ => return false,
            }
        }
    }
    true
}

/// Whether a shell of `made` encloses a negative volume where the same
/// shell of `body` enclosed a positive one, or the other way round, each
/// beyond what its measure can be off by (`crate::lumps::outer_shells`):
/// the body turned inside out. A shell that cannot be measured, or whose
/// volume cannot be told from none, says nothing. Not counted against the
/// operation's work bound: what an operation is allowed must not depend
/// on whether its result is measured.
pub(super) fn inside_out(body: &Body, made: &Body, tolerance: &Tolerance) -> bool {
    let signs = |of: &Body| {
        let volumes = craft3d_geom::allowance::uncounted(|| craft3d_topo::shell_volumes(of));
        crate::lumps::outer_shells(of, &volumes, tolerance).ok()
    };
    let Some(after) = signs(made) else {
        return false;
    };
    if !after.contains(&Some(false)) {
        return false;
    }
    let Some(before) = signs(body) else {
        return false;
    };
    before
        .iter()
        .zip(&after)
        .any(|(was, is)| matches!((was, is), (Some(a), Some(b)) if a != b))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;
    use craft3d_geom::{Circle, Line};
    use craft3d_math::Frame;

    fn line(a: Vec3, b: Vec3, start: u32, end: u32) -> Edge {
        Edge {
            curve: Curve3::Line(Line::through(a, b).unwrap()),
            range: (0.0, a.distance(b)),
            start: VertexId(start),
            end: VertexId(end),
        }
    }

    fn arc(centre: Vec3, radius: f64, range: (f64, f64), start: u32, end: u32) -> Edge {
        Edge {
            curve: Curve3::Circle(Circle::new(Frame::WORLD.at(centre).unwrap(), radius).unwrap()),
            range,
            start: VertexId(start),
            end: VertexId(end),
        }
    }

    fn meets(a: &Edge, b: &Edge, tol: f64) -> bool {
        meet(a, &path(a).unwrap(), b, &path(b).unwrap(), tol)
    }

    /// A circle that crosses a line, touches it, and stays a tolerance and
    /// a bit clear of it, at three scales: met, met, not met; either way
    /// round.
    #[test]
    fn a_circle_meets_a_line_it_crosses_or_touches_and_not_one_it_clears() {
        for k in [1e-3, 1.0, 1e6] {
            let tol = 1e-6 * k;
            let circle = arc(Vec3::new(0.0, 0.0, 0.0), 7.0 * k, (0.0, 2.0 * PI), 0, 0);
            let at = |y: f64| {
                line(
                    Vec3::new(-20.0 * k, y, 0.0),
                    Vec3::new(20.0 * k, y, 0.0),
                    1,
                    2,
                )
            };
            for (y, met) in [
                (3.0 * k, true),
                (7.0 * k, true),
                (7.0 * k + 0.5 * tol, true),
                (7.0 * k + 2.0 * tol, false),
                (9.0 * k, false),
            ] {
                assert_eq!(meets(&circle, &at(y), tol), met, "scale {k}, line at {y}");
                assert_eq!(meets(&at(y), &circle, tol), met, "scale {k}, line at {y}");
            }
        }
    }

    /// Two edges that share a vertex and leave it tangent to each other
    /// (a line and the arc of a round that runs on from it) do not meet:
    /// they are within the tolerance of each other for a stretch from the
    /// vertex on, and never part before it. The same arc turned back
    /// across the line meets it.
    #[test]
    fn edges_tangent_at_a_vertex_they_share_do_not_meet_there() {
        for k in [1e-3, 1.0, 1e6] {
            let tol = 1e-6 * k;
            let along = line(
                Vec3::new(-10.0 * k, 0.0, 0.0),
                Vec3::new(0.0, 0.0, 0.0),
                0,
                1,
            );
            // The arc about (0, 10) from its lowest point, where it is
            // tangent to the line, a quarter turn up.
            let round = arc(
                Vec3::new(0.0, 10.0 * k, 0.0),
                10.0 * k,
                (1.5 * PI, 2.0 * PI),
                1,
                2,
            );
            assert!(!meets(&along, &round, tol), "scale {k}");
            assert!(!meets(&round, &along, tol), "scale {k}");
            // Three quarters of the circle about (0, -5), from its highest
            // point: a line on from the vertex touches it there only.
            let back = arc(
                Vec3::new(0.0, -5.0 * k, 0.0),
                5.0 * k,
                (0.5 * PI, 2.0 * PI),
                1,
                2,
            );
            let longer = line(
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(10.0 * k, 0.0, 0.0),
                1,
                3,
            );
            assert!(!meets(&longer, &back, tol), "scale {k}: only at the vertex");
            let lower = line(
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(10.0 * k, -10.0 * k, 0.0),
                1,
                3,
            );
            assert!(meets(&lower, &back, tol), "scale {k}: across the arc");
            assert!(meets(&back, &lower, tol), "scale {k}: across the arc");
        }
    }

    /// Lines that cross, lines that lie on each other, and lines that
    /// pass a tolerance and a bit apart.
    #[test]
    fn lines_meet_where_they_cross_or_lie_on_each_other() {
        let tol = 1e-6;
        let a = line(Vec3::new(0.0, 0.0, 0.0), Vec3::new(10.0, 0.0, 0.0), 0, 1);
        let across = line(Vec3::new(5.0, -1.0, 0.0), Vec3::new(5.0, 1.0, 0.0), 2, 3);
        let over = line(Vec3::new(5.0, -1.0, 3e-6), Vec3::new(5.0, 1.0, 3e-6), 2, 3);
        let on = line(Vec3::new(2.0, 0.0, 0.0), Vec3::new(8.0, 0.0, 0.0), 2, 3);
        let next = line(Vec3::new(10.0, 0.0, 0.0), Vec3::new(10.0, 5.0, 0.0), 1, 2);
        assert!(meets(&a, &across, tol));
        assert!(!meets(&a, &over, tol));
        assert!(meets(&a, &on, tol));
        assert!(!meets(&a, &next, tol), "a corner is a vertex they share");
    }
}
