//! A fillet built in place is refused where its ball is shown to take in a
//! face beside its edge between the edge and the ball's foot on that face.
//!
//! **The rule.** A fillet rounds a convex corner off and fills a concave
//! one: in each plane across the edge it changes the region bounded by the
//! two faces' traces from the edge to the ball's feet and the ball's arc
//! between the feet (the short arc, on the edge's side of the centre). The
//! ball of the radius in that plane is the circle of the radius about its
//! centre (the centre lies in the plane). A trace starts at the edge,
//! outside the ball, and ends at its foot, on it: where a point of it lies
//! inside the ball by more than the tolerance between the rays from the
//! centre through the two feet (behind the arc), it has crossed the arc
//! (unless it reached in round a foot, outside the ball, which a face of a
//! solid would have to wrap round the other face's foot to do), and the
//! fillet face runs through that face into the material on the far side
//! of it, which must stay as it is: the region the fillet changes crosses
//! itself, and no fillet of that radius rolls there. What the construction
//! builds then adds material at a convex edge or takes it at a concave
//! one: a groove's rim with a bead beside it reaching into the ball
//! (property cases 585 and 716 of `blend_monotone`, built so by 0.1.0). A
//! face that reaches into the ball only beyond its foot, outside the rays,
//! crosses no arc, and the fillet is built (as a bead past a chamfer's line
//! beyond its foot is: [`super`]).
//!
//! **Where it is looked for.** In the planes of the chamfer's search
//! ([`super::SECTIONS`] + 1, evenly in the edge's parameter, both ends
//! included), each ball found as the construction finds its own (from the
//! one in the middle, which is the construction's seed, each from the one
//! before, in steps no longer than the construction's, so as to stay with
//! its family of balls, or the seed moved where a motion carries the edge
//! and both faces into themselves, as the construction's are; where that
//! does not follow, afresh in that plane, and where that fails too the
//! plane is passed over), and only where both feet lie on
//! their faces and at a point inside the face (within its loops, not on its
//! boundary), as the chamfer's.
//! - where the face's kind gives its trace in the plane as a rational curve
//!   (the chamfer's: a cylinder, a spline extruded along a direction, a
//!   cone, a sphere, a torus in a plane through its axis) and the foot lies
//!   on it, the curve between the edge and the foot is cut into pieces (at
//!   its knots first, then halved by de Casteljau, each piece with its own
//!   control points), each held in the convex hull of its control points,
//!   halved until every hull
//!   keeps clear of what lies behind the arc (all of it beyond the line of
//!   one ray, or all of it at least the radius less the tolerance from the
//!   centre) or a point of the curve behind the arc and inside the face is
//!   found (refused), within [`super::PIECES`] pieces; undecided there, not
//!   refused. A piece not clear one of whose three points looked at lies
//!   behind the arc but not inside the face (and none does) is taken to lie
//!   past the face and is not halved further. A plane (a line, tangent to
//!   the ball at the foot) keeps clear.
//! - other faces, or a foot off the plane (where the faces change along the
//!   edge, the ball touches them a little off the plane across it): the
//!   trace at [`super::SAMPLES`] points, each a part of the straight
//!   distance from the edge's point to the foot, followed from the foot
//!   back towards the edge; between them nothing is looked at.
//!
//! Nothing here asks the body where a point lies until a point behind the
//! arc is found, and no ball is counted as the construction's work: a
//! fillet not shown wrong costs its planes' balls and traces (measured,
//! release, on a loaded laptop: about 1.3 ms beside a slanted cylinder, 1.6
//! to 3 ms beside the groove's spline wall, 0.01 ms between two planes, of
//! fillets that take 6 to 15 ms in all).
//!
//! **What it is not.** A search, not a proof, and a fillet not shown wrong
//! is built, as before. It can miss what the chamfer's search can (a
//! stretch shorter than a sixty-fourth of the edge's parameter, between two
//! planes; a trace held by its points between them; a trace left
//! undecided; a plane passed over; a piece taken to lie past the face; a
//! plane where a foot leaves its face), and: a face beyond its foot that
//! comes back across the arc into what the fillet changes; a ball the
//! search finds on another branch than the construction (a chain followed
//! from its first edge, where each edge is seeded here on its own); faces
//! other than the edge's two. The closed forms (lines and circles across
//! the edge) need no search: a line or a circle tangent to a circle meets
//! it nowhere else.

use super::{DEPTH, FOLLOW, Held, PIECES, SAMPLES, SECTIONS, Trace, trace_of};
use crate::blend::law::{Along, RadiusLaw};
use crate::blend::rolling::{
    Across, Item, Section, Shape, Side, Solver, Spine, Steps, edge_length, exact, flat_seed,
    refuse, resized, scale_of,
};
use crate::blend::{BlendError, Reason};
use crate::boolean::probe::{OnFace, Probe};
use craft3d_geom::{NurbsCurve, Surface};
use craft3d_math::Portable;
use craft3d_math::{Tolerance, Vec3};
use craft3d_topo::Body;
use std::cell::OnceCell;

/// `Ok` unless the fillet of `item` (built in place) of `radius` is shown,
/// in one of the planes across its edge looked at, to have a face's trace
/// between the edge and its foot behind the ball's arc; then refused as
/// passing through that face (see the module documentation).
pub(super) fn shown_wrong(
    body: &Body,
    item: &Item,
    radius: f64,
    law: Option<RadiusLaw>,
    tolerance: &Tolerance,
) -> Result<(), BlendError> {
    let edge = item.request.edge;
    let tol = tolerance.linear;
    let Some(data) = body.edge(edge) else {
        return Ok(());
    };
    // A radius that varies along the edge (`radius` its largest): each
    // plane's ball has the law's radius there. A law that is not one is
    // the construction's to refuse.
    let along = match law {
        Some(law) => match Along::new(law, edge, &data.curve, data.range, tol) {
            Ok(along) => Some(along),
            Err(_) => return Ok(()),
        },
        None => None,
    };
    let [face_a, face_b] = item.faces;
    let (Ok(side_a), Ok(side_b)) = (Side::of(body, face_a), Side::of(body, face_b)) else {
        return Ok(());
    };
    let shape = Shape::Ball {
        radius,
        convex: item.convex,
    };
    let solver = Solver {
        spine: Spine {
            curve: &data.curve,
            range: data.range,
            closed: data.start == data.end,
            joints: [None, None],
            turn: 0.0,
        },
        sides: [side_a, side_b],
        shape,
        scale: scale_of(data, radius, tol),
        moved: None,
        settle: 0.0,
        law: along.as_ref(),
    };
    // The construction's seed, settled without being counted as the
    // construction's work (`Solver::section` counts it).
    let Some(seed) = afresh(&solver, 0.5, item) else {
        return Ok(());
    };
    // Where a point of a face's surface lies against the face: asked of
    // the body only where a point behind the arc is found.
    let probe: OnceCell<Option<Probe<'_>>> = OnceCell::new();
    let place = |k: usize, p: Vec3| {
        let face = item.faces.get(k)?;
        probe
            .get_or_init(|| Probe::new(body, tolerance).ok())
            .as_ref()?
            .on_face(face.0 as usize, p)
            .ok()
    };
    let half = SECTIONS / 2;
    let step = 1.0 / f64::from(SECTIONS);
    // The construction follows its balls in steps of at most this: so is
    // each plane reached from the one before, so as to stay with the
    // construction's balls where another family lies near.
    let inside = Steps::of(radius, edge_length(data)).inside;
    // Where a motion carries the edge and both faces into themselves the
    // construction's balls are the seed moved, and so are these.
    let moved = match solver.law {
        Some(_) => None,
        None => exact::motion(&solver.spine, solver.sides)
            .and_then(|m| exact::Moved::new(m, &solver.spine, seed, 0.5)),
    };
    // From the middle out to each end, each plane from the one before.
    for way in [1.0, -1.0] {
        let mut at = (0.5, seed);
        for j in 0..=half {
            let s = 0.5 + way * f64::from(j) * step;
            let found = if j == 0 {
                Some(seed)
            } else {
                moved
                    .and_then(|m| m.at(s, solver.sides))
                    .or_else(|| march(&solver, at, s, inside))
                    .or_else(|| afresh(&solver, s, item))
            };
            // Passed over: no ball found in this plane.
            let Some(section) = found else {
                continue;
            };
            at = (s, section);
            // The ball's radius in this plane.
            let Some(radius) = solver.size_at(s) else {
                continue;
            };
            if let Some(face) = wrong_in(&solver, s, &section, radius, tol, &place) {
                let face = item.faces.get(face).copied();
                return Err(refuse(edge, face, Reason::Through));
            }
        }
    }
    Ok(())
}

/// The ball in the plane at `s` from the flat estimate, as the
/// construction's first section is found.
fn afresh(solver: &Solver<'_>, s: f64, item: &Item) -> Option<Section> {
    let seed = flat_seed(solver, s, item.into, item.request.edge).ok()?;
    ball_at(solver, s, solver.spine.at(s)?, &seed)
}

/// The ball in the plane `across` (the one at `s`), started from `near`.
fn ball_at(solver: &Solver<'_>, s: f64, across: Across, near: &Section) -> Option<Section> {
    match solver.shape {
        Shape::Ball { convex, .. } => {
            let radius = solver.size_at(s)?;
            // As the construction starts a ball of a law: from its
            // neighbour resized.
            let near = match solver.law {
                Some(_) => resized(near, across, radius)?,
                None => *near,
            };
            solver.ball(across, &near, radius, convex)
        }
        Shape::Cut { .. } => None,
    }
}

/// The ball at `to`, from `near` (the one at `from`), in even steps no
/// longer than `step` (the construction's), each followed as the
/// construction follows its own ([`follow`]).
fn march(solver: &Solver<'_>, (from, near): (f64, Section), to: f64, step: f64) -> Option<Section> {
    let span = (to - from).abs();
    let mut count = 1_u32;
    while f64::from(count) * step < span && count < MOST_STEPS {
        count += 1;
    }
    let mut at = (from, near);
    for i in 1..=count {
        let s = if i == count {
            to
        } else {
            from + (to - from) * f64::from(i) / f64::from(count)
        };
        let next = follow(solver, at.0, &at.1, s, FOLLOW)?;
        at = (s, next);
    }
    Some(at.1)
}

/// Most steps from one plane looked at to the next.
const MOST_STEPS: u32 = 64;

/// The ball at `to`, from `near` (the one at `from`), as the construction
/// follows its own: directly, or through balls between, halving the step
/// up to `depth` times.
fn follow(solver: &Solver<'_>, from: f64, near: &Section, to: f64, depth: u32) -> Option<Section> {
    if let Some(found) = ball_at(solver, to, solver.spine.at(to)?, near) {
        return Some(found);
    }
    if depth == 0 {
        return None;
    }
    let middle = 0.5 * (from + to);
    let half = follow(solver, from, near, middle, depth - 1)?;
    follow(solver, middle, &half, to, depth - 1)
}

/// The ball's circle in the plane across the edge and what lies behind its
/// arc: inside the circle by more than the tolerance, strictly between the
/// rays from the centre through the two feet (put into the plane).
struct Arc {
    centre: Vec3,
    /// The radius less the tolerance.
    inner: f64,
    /// From the centre towards each foot, in the plane.
    rays: [Vec3; 2],
    /// Square to the plane, turned so that the first ray turns into the
    /// second about it by less than half a turn.
    normal: Vec3,
}

impl Arc {
    /// The arc of a ball centred at `centre` (in the plane through `e`
    /// square to `normal`) touching the faces at `feet`; `None` where the
    /// feet put into the plane lie on one line through the centre (an arc
    /// of no turn or of half a turn: nothing to hold against).
    fn new(
        centre: Vec3,
        radius: f64,
        feet: [Vec3; 2],
        e: Vec3,
        normal: Vec3,
        tol: f64,
    ) -> Option<Arc> {
        let flat = |p: Vec3| p - normal * normal.dot(p - e);
        let rays = [flat(feet[0]) - centre, flat(feet[1]) - centre];
        let [a, b] = rays;
        let turn = a.cross(b).dot(normal);
        if !(turn.abs() > 1e-9 * a.length() * b.length()) {
            return None;
        }
        Some(Arc {
            centre,
            inner: radius - tol,
            rays,
            normal: if turn > 0.0 { normal } else { -normal },
        })
    }

    /// How far `p` lies on the first ray's side of the line of the first
    /// ray (positive towards the second), and on the second's of the line
    /// of the second.
    fn sides(&self, p: Vec3) -> [f64; 2] {
        let q = p - self.centre;
        let [a, b] = self.rays;
        [a.cross(q).dot(self.normal), q.cross(b).dot(self.normal)]
    }

    /// Behind the arc: inside the circle by more than the tolerance,
    /// strictly between the rays.
    fn behind(&self, p: Vec3) -> bool {
        let [s, t] = self.sides(p);
        (p - self.centre).length() < self.inner && s > 0.0 && t > 0.0
    }

    /// Whether the convex hull of `hull` (points in the plane) keeps clear
    /// of what lies behind the arc: all of it on the far side of the line
    /// of one ray, or all of it at least the radius less the tolerance
    /// from the centre.
    fn clear_of(&self, hull: &[Vec3]) -> bool {
        hull.iter().all(|p| self.sides(*p)[0] <= 0.0)
            || hull.iter().all(|p| self.sides(*p)[1] <= 0.0)
            || self.reach(hull) >= self.inner
    }

    /// The distance from the centre to the convex hull of `points` (in the
    /// plane): none where the centre lies within it (the points spread
    /// round it by half a turn or more), else the least distance to a
    /// segment between two of them (the hull's nearest point lies on one of
    /// its edges, each such a segment, and every such segment lies in it).
    fn reach(&self, points: &[Vec3]) -> f64 {
        let [a, _] = self.rays;
        let Some(x) = a.normalized() else {
            return 0.0;
        };
        let y = self.normal.cross(x);
        let mut angles = Vec::with_capacity(points.len());
        for p in points {
            let q = *p - self.centre;
            let (u, v) = (q.dot(x), q.dot(y));
            if !(u.phypot(v) > 0.0) {
                return 0.0;
            }
            angles.push(v.patan2(u));
        }
        angles.sort_by(f64::total_cmp);
        let (Some(first), Some(last)) = (angles.first(), angles.last()) else {
            return 0.0;
        };
        // The widest gap between the directions to the points, round the
        // turn: less than half a turn, and the centre is within the hull.
        let mut widest = first + std::f64::consts::TAU - last;
        for w in angles.windows(2) {
            if let [p, q] = w {
                widest = widest.max(q - p);
            }
        }
        if !(widest > std::f64::consts::PI) {
            return 0.0;
        }
        let mut least = f64::INFINITY;
        for (i, p) in points.iter().enumerate() {
            for q in points.iter().skip(i) {
                least = least.min(to_segment(*p - self.centre, *q - self.centre));
            }
        }
        least
    }
}

/// The distance from the origin to the segment from `p` to `q`.
fn to_segment(p: Vec3, q: Vec3) -> f64 {
    let d = q - p;
    let length = d.dot(d);
    let s = if length > 0.0 {
        (-p.dot(d) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p + d * s).length()
}

/// The side whose face's trace between the edge and its foot is shown
/// behind the arc of the ball of `section` (at `s`), at a point inside
/// that face (`place`, of the side and a point), if one is; none where a
/// foot lies off its face.
fn wrong_in(
    solver: &Solver<'_>,
    s: f64,
    section: &Section,
    radius: f64,
    tol: f64,
    place: &dyn Fn(usize, Vec3) -> Option<OnFace>,
) -> Option<usize> {
    let across = solver.spine.at(s)?;
    let (e, normal) = (across.point, across.normal);
    let centre = section.centre?;
    let arc = Arc::new(centre, radius, section.feet, e, normal, tol)?;
    for (k, (side, &foot)) in solver.sides.iter().zip(&section.feet).enumerate() {
        let inside = |p: Vec3| place(k, p) == Some(OnFace::In);
        // A foot off the plane by more than the tolerance is on no curve in
        // it (the trace's ends are held to within the tolerance): held by
        // points without building the curve. (A plane's foot lies in the
        // plane: the edge lies in the face.)
        let off = normal.dot(foot - e).abs() > tol;
        let traced = if off && !matches!(side.surface, Surface::Plane(_)) {
            None
        } else {
            trace_of(side.surface, e, normal, foot, tol)
        };
        let held = match traced {
            // A line through the foot is tangent to the ball there.
            Some(Trace::Straight) => Held::Clear,
            Some(Trace::Curve(curve, from, to)) => held_by_hull(&curve, (from, to), &arc, &inside),
            None => held_by_points(solver, across, section, k, &arc, &inside),
        };
        if held == Held::Wrong {
            // A foot off its face: the construction does not roll there
            // (it carries the border onto the next face, or the plane has
            // left the face): nothing is shown.
            let off_face = section
                .feet
                .iter()
                .enumerate()
                .any(|(j, foot)| place(j, *foot) == Some(OnFace::Out));
            return (!off_face).then_some(k);
        }
    }
    None
}

/// The trace held by the convex hulls of the control points of its pieces
/// against what lies behind `arc` (see the module documentation): wrong at
/// a point behind it that lies inside the face (`inside`). On a closed
/// curve the piece from the edge's point to the foot is the shorter way
/// round, as the chamfer's.
fn held_by_hull(
    curve: &NurbsCurve,
    (from, to): (f64, f64),
    arc: &Arc,
    inside: &dyn Fn(Vec3) -> bool,
) -> Held {
    let (d0, d1) = curve.domain();
    let (start, end) = (curve.eval(d0), curve.eval(d1));
    let closed = start.distance(end) <= 1e-12 * (1.0 + start.length());
    let (lo, hi) = (from.min(to), from.max(to));
    // On a closed curve, the way round whose middle lies nearer the
    // middle of the two ends.
    let (a, b) = (curve.eval(from), curve.eval(to));
    let mid = (a + b) * 0.5;
    let short = curve.eval(0.5 * (lo + hi)).distance(mid);
    let round = {
        let t = 0.5 * (hi + d1 + lo - d0);
        let t = if t > d1 { t - (d1 - d0) } else { t };
        curve.eval(t).distance(mid)
    };
    let ranges = if closed && round < short {
        vec![(d0, lo), (hi, d1)]
    } else {
        vec![(lo, hi)]
    };
    // Each range cut at the curve's knots into rational Bézier pieces, each
    // with its own control points (homogeneous), halved by de Casteljau:
    // the curve is split into its spans once, not for every piece.
    let spans = curve.bezier_pieces();
    let mut stack: Vec<(f64, f64, u32, Row)> = Vec::new();
    for (lo, hi) in ranges {
        for ((s0, s1), points) in &spans {
            let (a, b) = (lo.max(*s0), hi.min(*s1));
            if !(b > a && s1 > s0) {
                continue;
            }
            let row: Row = points.iter().map(|&(p, w)| (p * w, w)).collect();
            let row = split(&row, (a - s0) / (s1 - s0)).1;
            let row = split(&row, (b - a) / (s1 - a)).0;
            stack.push((a, b, 0, row));
        }
    }
    let mut pieces = 0_usize;
    let mut undecided = false;
    while let Some((a, b, depth, row)) = stack.pop() {
        pieces += 1;
        if pieces > PIECES {
            return Held::Undecided;
        }
        let (left, right) = split(&row, 0.5);
        let ends = [row.first(), left.last(), row.last()];
        // Points behind the arc, asked whether they lie inside the face;
        // one that does shows it.
        let mut past_face = false;
        for p in ends.iter().filter_map(|h| h.and_then(|&h| point(h))) {
            if !arc.behind(p) {
                continue;
            }
            if inside(p) {
                return Held::Wrong;
            }
            past_face = true;
        }
        let hull: Option<Vec<Vec3>> = row.iter().map(|&h| point(h)).collect();
        if let Some(hull) = hull
            && arc.clear_of(&hull)
        {
            continue;
        }
        // Not clear, behind the arc only off the face: taken to lie past
        // the face.
        if past_face {
            continue;
        }
        let middle = 0.5 * (a + b);
        if depth >= DEPTH || !(middle > a && middle < b) {
            undecided = true;
            continue;
        }
        stack.push((middle, b, depth + 1, right));
        stack.push((a, middle, depth + 1, left));
    }
    if undecided {
        Held::Undecided
    } else {
        Held::Clear
    }
}

/// The control points of a rational Bézier piece, homogeneous: each point
/// times its weight, and the weight.
type Row = Vec<(Vec3, f64)>;

/// A homogeneous control point as a point: `None` where its weight is not
/// positive (the convex hull does not hold the curve there).
fn point((p, w): (Vec3, f64)) -> Option<Vec3> {
    (w.is_finite() && w > 0.0).then(|| p * (1.0 / w))
}

/// The piece `row` cut at `u` (0 to 1, in its own parameter) by de
/// Casteljau on the homogeneous points: the part before and the part
/// after, each the same degree.
fn split(row: &[(Vec3, f64)], u: f64) -> (Row, Row) {
    let mut work: Row = row.to_vec();
    let mut before = Vec::with_capacity(row.len());
    let mut after = Vec::with_capacity(row.len());
    while let (Some(&first), Some(&last)) = (work.first(), work.last()) {
        before.push(first);
        after.push(last);
        work = work
            .windows(2)
            .filter_map(|w| match w {
                [(p, a), (q, b)] => Some((*p * (1.0 - u) + *q * u, a * (1.0 - u) + b * u)),
                _ => None,
            })
            .collect();
    }
    after.reverse();
    (before, after)
}

/// The trace of the face on side `k` held by [`SAMPLES`] of its points
/// against what lies behind `arc`: each where the construction's chamfer
/// solver finds a point of the face in the plane (straight from the
/// edge's point) at a part of the distance to the foot, followed from the
/// one before, from the foot back towards the edge. Wrong at a point
/// behind the arc inside the face (`inside`).
fn held_by_points(
    solver: &Solver<'_>,
    across: Across,
    section: &Section,
    k: usize,
    arc: &Arc,
    inside: &dyn Fn(Vec3) -> bool,
) -> Held {
    let reach = section.feet.map(|foot| foot.distance(across.point));
    let mut near = *section;
    for i in (1..SAMPLES).rev() {
        let part = f64::from(i) / f64::from(SAMPLES);
        let Some(found) = solver.cut(across, &near, reach.map(|d| d * part)) else {
            return Held::Undecided;
        };
        if let Some(p) = found.feet.get(k)
            && arc.behind(*p)
            && inside(*p)
        {
            return Held::Wrong;
        }
        near = found;
    }
    Held::Clear
}
