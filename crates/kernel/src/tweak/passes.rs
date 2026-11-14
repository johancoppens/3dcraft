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
//!    edges they were did not.
//! 2. **A vertex a moved face went through** ([`swept`]). A vertex that is
//!    not on a face that moved, and lies on the other side of that face's
//!    surface than it did (or on it), with the face over it: beside the
//!    edges that end in such a vertex the body is asked whether its faces
//!    are still skin, as beside the edges made again (`super::buried`).
//!    The question decides, the vertex only says where to ask: a vertex
//!    under an overhang the face grew into is passed by its surface, and
//!    is fine.
//! 3. **An edge made again that goes through a face not its own**
//!    ([`pierced`]). The rim of a pocket's floor moved down a pocket that
//!    widens runs out through the sides of the block: where an edge made
//!    again crosses the surface of a face that shares no vertex with it,
//!    on that face, the edge is cut into stretches, and the same question
//!    is asked beside the middle of each (the middle of the whole edge,
//!    which is where it was asked before, can lie on the right side).
//! 4. **A body turned inside out** ([`inside_out`]). A shell that enclosed
//!    a volume and is measured to enclose a negative one. Measured only
//!    where a face that does not touch a moved face lies in the box the
//!    moved face went through ([`Swept::near`]): the measure costs what
//!    `document.measure` costs.
//!
//! What these do not find: a moved face that passes a face far from it
//! where no vertex lies in its way, no edge made again goes through it
//! and the volume stays positive (the bottom of a block moved up into a
//! groove of its top). That needs the faces' intersection, and is left,
//! as before, to the mesh the regeneration looks for self-intersections
//! on.

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
/// Most places looked at closely for one pair of edges, and pairs of edges
/// looked at in one operation; past them the check says nothing.
const MOST_LOOKS: usize = 64;
const MOST_PAIRS: usize = 1 << 20;
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
    let shared: Vec<f64> = [(a.start, a.range.0), (a.end, a.range.1)]
        .into_iter()
        .filter(|(vertex, _)| *vertex == b.start || *vertex == b.end)
        .map(|(_, at)| at)
        .collect();
    let mut looks = 0_usize;
    for (ts, ps) in pa.params.windows(2).zip(pa.points.windows(2)) {
        let ([s0, s1], [p0, p1]) = (ts, ps) else {
            continue;
        };
        for (us, qs) in pb.params.windows(2).zip(pb.points.windows(2)) {
            let ([t0, t1], [q0, q1]) = (us, qs) else {
                continue;
            };
            let (along, across, off) = segments(*p0, *p1, *q0, *q1);
            if off.is_nan() || off > reach {
                continue;
            }
            looks += 1;
            if looks > MOST_LOOKS {
                return false;
            }
            let (s, _, apart) =
                nearest_pair(a, b, s0 + (s1 - s0) * along, t0 + (t1 - t0) * across, tol);
            if apart.is_nan() || apart > tol {
                continue;
            }
            // The vertex's own: the edges never part between it and here.
            let own = shared.iter().any(|end| {
                !(1..=BETWEEN).any(|k| {
                    let at = end + (s - end) * k as f64 / (BETWEEN + 1) as f64;
                    gap(a.curve.eval(at), b, pb) > tol
                })
            });
            if !own {
                return true;
            }
        }
    }
    false
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
) -> Option<(FaceId, EdgeId, EdgeId)> {
    let tol = MEET * tolerance.linear;
    let mut paths: BTreeMap<EdgeId, Option<Path>> = BTreeMap::new();
    let mut seen: BTreeSet<(EdgeId, EdgeId)> = BTreeSet::new();
    let mut pairs = 0_usize;
    for face in super::faces_with(uses, remade) {
        let edges: BTreeSet<EdgeId> = made
            .face(face)
            .into_iter()
            .flat_map(|f| &f.loops)
            .filter_map(|l| made.loop_(*l))
            .flat_map(|l| l.coedges.iter().map(|c| c.edge))
            .collect();
        for first in edges.iter().filter(|e| remade.contains(e)) {
            for second in edges.iter().filter(|e| *e != first) {
                let key = (*first.min(second), *first.max(second));
                if !seen.insert(key) {
                    continue;
                }
                pairs += 1;
                if pairs > MOST_PAIRS || craft3d_geom::allowance::exhausted() {
                    return None;
                }
                let (Some(a), Some(b)) = (made.edge(*first), made.edge(*second)) else {
                    continue;
                };
                for id in [first, second] {
                    paths
                        .entry(*id)
                        .or_insert_with(|| made.edge(*id).and_then(path));
                }
                let (Some(Some(pa)), Some(Some(pb))) = (paths.get(first), paths.get(second)) else {
                    continue;
                };
                if !meet(a, pa, b, pb, tol) {
                    continue;
                }
                // Where they were: edges that met already are not this
                // operation's doing.
                let before = match (body.edge(*first), body.edge(*second)) {
                    (Some(a), Some(b)) => match (path(a), path(b)) {
                        (Some(pa), Some(pb)) => meet(a, &pa, b, &pb, tol),
                        // Not followed: it cannot be told, and says nothing.
                        _ => true,
                    },
                    _ => false,
                };
                if !before {
                    return Some((face, *first, *second));
                }
            }
        }
    }
    None
}

/// What the faces that moved went through ([`swept`]).
pub(super) struct Swept {
    /// The edges that end in a vertex a moved face passed with the face
    /// over it, each with that face.
    pub edges: BTreeMap<EdgeId, FaceId>,
    /// Whether a face that shares no vertex with a moved face lies in the
    /// box that face went through.
    pub near: bool,
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

/// Where the faces of a body lie: the vertices of each and its box (none
/// for a face that cannot be boxed), made once for [`swept`] and
/// [`pierced`].
pub(super) struct Layout {
    vertices: Vec<BTreeSet<VertexId>>,
    boxes: Vec<Option<Bbox3>>,
}

impl Layout {
    pub(super) fn of(made: &Body) -> Self {
        Layout {
            vertices: (0..made.faces.len())
                .map(|i| super::face_vertices(made, FaceId(super::index_u32(i))))
                .collect(),
            boxes: (0..made.faces.len())
                .map(|i| made.face_bbox(FaceId(super::index_u32(i))).ok())
                .collect(),
        }
    }
}

/// Halvings of the search for where an edge crosses a surface.
const CUT_STEPS: usize = 48;
/// Most pairs of an edge and a face near it ([`pierced`]), and of a moved
/// face and a vertex in its way ([`swept`]), that are looked at closely in
/// one operation; past them the check says nothing more. These checks are
/// not counted against the operation's work bound (`super::resurface_with`),
/// so each is held by a count of its own.
const MOST_NEAR: usize = 1 << 13;

/// The edges of `remade` that cross the surface of a face of `made` that
/// shares no vertex with them, on that face (or where that cannot be
/// told): per edge, the middles of the stretches the crossings cut it
/// into, and the first face it goes through (the module documentation).
/// Found from the edge followed in steps: a stretch that goes in and out
/// again between two steps is not seen.
pub(super) fn pierced(
    made: &Body,
    layout: &Layout,
    uses: &[Vec<FaceId>],
    remade: &BTreeSet<EdgeId>,
    tolerance: &Tolerance,
) -> BTreeMap<EdgeId, (Vec<f64>, FaceId)> {
    let tol = tolerance.linear;
    let mut found = BTreeMap::new();
    let mut looked = 0_usize;
    for id in remade {
        if looked > MOST_NEAR {
            break;
        }
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
        for (index, (bounds, vertices)) in layout.boxes.iter().zip(&layout.vertices).enumerate() {
            let face = FaceId(super::index_u32(index));
            if !bounds.as_ref().is_some_and(|b| boxes_meet(b, &around))
                || own.contains(&face)
                || vertices.contains(&edge.start)
                || vertices.contains(&edge.end)
            {
                continue;
            }
            let Some(data) = made.face(face) else {
                continue;
            };
            looked += 1;
            if looked > MOST_NEAR {
                break;
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
    found
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
) -> Swept {
    let tol = tolerance.linear;
    let mut found = Swept {
        edges: BTreeMap::new(),
        near: false,
    };
    let (vertices, boxes) = (&layout.vertices, &layout.boxes);
    let mut looked = 0_usize;
    // The edges at every vertex.
    let mut at: Vec<Vec<EdgeId>> = vec![Vec::new(); made.vertices.len()];
    for (index, edge) in made.edges.iter().enumerate() {
        for vertex in [edge.start, edge.end] {
            if let Some(list) = at.get_mut(vertex.0 as usize) {
                list.push(EdgeId(super::index_u32(index)));
            }
        }
    }
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
        let Some(own) = vertices.get(face.0 as usize) else {
            continue;
        };
        if !found.near {
            found.near = boxes.iter().zip(vertices).enumerate().any(|(i, (b, v))| {
                i != face.0 as usize
                    && b.as_ref().is_some_and(|b| boxes_meet(b, &way))
                    && v.is_disjoint(own)
            });
        }
        for (index, (old, new)) in body.vertices.iter().zip(&made.vertices).enumerate() {
            let vertex = VertexId(super::index_u32(index));
            if !(way.contains(old.point, tol) || way.contains(new.point, tol))
                || own.contains(&vertex)
            {
                continue;
            }
            looked += 1;
            if looked > MOST_NEAR {
                return found;
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
            for edge in at.get(index).into_iter().flatten() {
                found.edges.entry(*edge).or_insert(*face);
            }
        }
    }
    found
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
