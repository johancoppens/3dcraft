//! A chamfer built in place is refused where it is shown to pass through
//! a face beside its edge.
//!
//! **The rule.** A chamfer cuts a convex corner off and fills a concave
//! one: in each plane across the edge it changes the region bounded by
//! the two faces' traces from the edge to their feet and the segment
//! between the feet. That region is a chamfer's where neither face runs
//! into it: it is not where a trace crosses the segment between the feet
//! (a pipe chamfered far along it, the overhanging nose of an ogee, a cone
//! near its narrow end: the face between the edge and its foot lies on
//! the far side of the chamfer face, in what the chamfer should leave), or
//! where the face beyond its foot runs back into the region between the
//! feet (it comes to its foot from past the segment's line and goes on to
//! the edge's side: the face that stays lies on the chamfer's side of the
//! chamfer face), and what the construction builds then takes material
//! from the face it should only fill against (or adds to one it should
//! only cut). A trace that goes past the line only beyond the segment's
//! ends (a bead on the face, past the foot along the line's extension)
//! meets neither, and the chamfer is right, also where the face beyond the
//! foot only grazes the region, within the tolerance (the chamfer line
//! near its tangent to the face, down the bead's far slope).
//!
//! **Where it is looked for.** In [`SECTIONS`] + 1 planes across the edge,
//! evenly in its parameter from one end to the other (both ends
//! included), each found as the construction finds its own (from the one
//! in the middle, each from the one before; where that does not follow,
//! afresh in that plane, and where that fails too the plane is passed
//! over): a stretch of the edge over which the line runs through a face,
//! if it is at least `1 / SECTIONS` of the edge's parameter long, holds
//! one of them. In each of those planes the trace of each face's surface
//! from the edge to the foot is held against the segment, the tolerance
//! allowed: a point of it more than the tolerance past the segment's line,
//! between the feet, or, where it comes to the foot from past the line, a
//! point of the face beyond the foot more than the tolerance into the
//! region between the feet, shows the chamfer wrong, but only at a point
//! inside the face (within its loops, not on its
//! boundary), and only where both feet lie on their faces: the surface
//! goes on past the face (near an end of the edge, where the face beside
//! it meets the edge's plane obliquely, a plane across the edge leaves the
//! face before the foot; where the construction carries a cut line across
//! a smooth edge onto the next face, the foot here is on the first face's
//! surface continued, not where the construction puts it), and there it
//! stands down.
//! - where the face's kind gives the trace as a rational curve: a plane
//!   (a line), a cylinder or a spline extruded along a direction (its
//!   circle or profile carried along that direction into the plane: an
//!   affine image), a cone (its circle carried from the apex into the
//!   plane: a projective one), a sphere (a circle), a torus in a plane
//!   through its axis (a circle of its tube): the curve between the edge
//!   and the foot is cut into pieces each held in the convex hull of its
//!   control points, halved until every hull keeps clear of what lies
//!   past the segment between the feet (clear) or a point of the curve
//!   inside the face is shown there (refused), within [`PIECES`] pieces;
//!   undecided there, not refused. A piece not clear none of whose three
//!   points looked at lies inside the face is taken to lie past it and is
//!   not halved further, which is not shown. Where the trace comes to the
//!   foot from past the line (a point of it just before the foot), the
//!   curve beyond the foot is followed at points closer and closer to it
//!   and then in steps of 1/256 of the trace, as far again, while it lies
//!   on the edge's side between the segment's ends.
//! - other faces (a torus in other planes, other spline faces, a cone
//!   whose circle carried into the plane is no curve the kernel has: the
//!   plane cuts the cone in a hyperbola or a parabola): the trace at
//!   [`SAMPLES`] points, each the distance along it the construction
//!   measures (straight, in the plane), and beyond the foot as far again
//!   in the same steps; between them nothing is looked at.
//!
//! **What it is not.** It is a search, not a proof, and a chamfer not
//! shown wrong is built, as before. What it can miss: a crossing shorter
//! than `1 / SECTIONS` of the edge's parameter, between two of the planes;
//! a crossing between the points of a trace held by its points; a trace
//! the halving leaves undecided; a plane passed over because no section
//! was found in it; a crossing inside a piece whose points looked at all
//! lie past the face; a plane where a foot leaves its face (a cut line
//! carried across a smooth edge). Fillets are held in the same planes by
//! [`ball`]. Where every section is the same (a straight edge along faces
//! a translation along it carries into themselves, a circle between faces
//! turned about its axis), one plane stands for all and nothing lies
//! between.

mod ball;

use super::{Item, Section, Shape, Side, Solver, Spine, first_section, refuse, scale_of};
use crate::blend::{BlendError, Reason, Size};
use crate::boolean::probe::{OnFace, Probe};
use crate::chamfer::ChamferSize;
use craft3d_geom::{Curve3, NurbsCurve, NurbsSurface, Surface};
use craft3d_math::Portable;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::Body;
use std::cell::OnceCell;

/// How many steps of the edge's parameter apart the planes it is held in
/// are: a stretch of the edge a sixty-fourth of its parameter long is
/// met by one of them.
const SECTIONS: u32 = 64;

/// Most pieces a trace is cut into in one plane.
const PIECES: usize = 4096;

/// Most halvings of one piece.
const DEPTH: u32 = 48;

/// Points along a trace that is held by its points.
const SAMPLES: u32 = 32;

/// Most halvings of a step from one plane to the next where the section
/// does not follow at once (the construction's own bound).
const FOLLOW: u32 = 8;

/// What a trace in one plane was found to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Held {
    /// On the edge's side all along.
    Clear,
    /// A point of it on the other side.
    Wrong,
    /// Neither shown within the pieces allowed.
    Undecided,
}

/// `Ok` unless the chamfer of `item` (built in place) is shown, in one of
/// the planes across its edge looked at, to have a face's trace on the
/// other side of its line from the edge; then refused as passing through
/// that face (see the module documentation). A fillet is held by
/// [`ball::shown_wrong`].
pub(super) fn shown_wrong(
    body: &Body,
    item: &Item,
    tolerance: &Tolerance,
) -> Result<(), BlendError> {
    match item.request.size {
        Size::Radius(radius) => return ball::shown_wrong(body, item, radius, None, tolerance),
        Size::Law(law) => {
            return ball::shown_wrong(body, item, law.largest(), Some(law), tolerance);
        }
        Size::Chamfer(_) => {}
    }
    let Size::Chamfer(ChamferSize::Distances { first, second }) = item.request.size else {
        return Ok(());
    };
    let edge = item.request.edge;
    let tol = tolerance.linear;
    let Some(data) = body.edge(edge) else {
        return Ok(());
    };
    let [face_a, face_b] = item.faces;
    let (Ok(side_a), Ok(side_b)) = (Side::of(body, face_a), Side::of(body, face_b)) else {
        return Ok(());
    };
    let sides = [side_a, side_b];
    let shape = Shape::Cut {
        distances: [first, second],
    };
    let solver = Solver {
        spine: Spine {
            curve: &data.curve,
            range: data.range,
            closed: data.start == data.end,
            joints: [None, None],
            turn: 0.0,
        },
        sides,
        shape,
        scale: scale_of(data, shape.size(), tol),
        moved: None,
        settle: 0.0,
        law: None,
    };
    // A section the construction cannot find leaves nothing to look at
    // here; it says so itself.
    let Ok(seed) = first_section(&solver, 0.5, item.into, edge) else {
        return Ok(());
    };
    // Where a point of a face's surface lies against the face: asked of
    // the body only where a point on the wrong side is found.
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
    // From the middle out to each end, each plane from the one before.
    for way in [1.0, -1.0] {
        let mut at = (0.5, seed);
        for j in 0..=half {
            let s = 0.5 + way * f64::from(j) * step;
            let found = if j == 0 {
                Some(seed)
            } else {
                follow(&solver, at.0, &at.1, s, FOLLOW, [first, second])
                    .or_else(|| first_section(&solver, s, item.into, edge).ok())
            };
            // Passed over: no section found in this plane.
            let Some(section) = found else {
                continue;
            };
            at = (s, section);
            if let Some(face) = wrong_in(&solver, s, &section, [first, second], tol, &place) {
                let face = item.faces.get(face).copied();
                return Err(refuse(edge, face, Reason::Through));
            }
        }
    }
    Ok(())
}

/// The section at `to`, from `near` (the one at `from`), as the
/// construction follows its own: directly, or through sections between,
/// halving the step up to `depth` times.
fn follow(
    solver: &Solver<'_>,
    from: f64,
    near: &Section,
    to: f64,
    depth: u32,
    distances: [f64; 2],
) -> Option<Section> {
    let across = solver.spine.at(to)?;
    if let Some(found) = solver.cut(across, near, distances) {
        return Some(found);
    }
    if depth == 0 {
        return None;
    }
    let middle = 0.5 * (from + to);
    let half = follow(solver, from, near, middle, depth - 1, distances)?;
    follow(solver, middle, &half, to, depth - 1, distances)
}

/// The segment between a section's feet in its plane, and the side of it
/// the edge's point lies on.
struct Segment {
    /// The first foot.
    from: Vec3,
    /// Along the segment, from the first foot to the second.
    along: Vec3,
    /// Square to it in the plane, towards the edge's point.
    towards: Vec3,
    length: f64,
    /// How far past the line a point may lie: the tolerance.
    floor: f64,
}

impl Segment {
    /// How far `p` lies on the edge's side of the line: negative past it.
    fn clearance(&self, p: Vec3) -> f64 {
        self.towards.dot(p - self.from)
    }

    /// Where `p` lies along the line, from the first foot.
    fn at(&self, p: Vec3) -> f64 {
        self.along.dot(p - self.from)
    }

    /// Past the line by more than the tolerance, between the feet: on the
    /// far side of the segment itself.
    fn behind(&self, p: Vec3) -> bool {
        let at = self.at(p);
        self.clearance(p) < self.floor && at > 0.0 && at < self.length
    }

    /// Whether every point of a convex hull keeps clear of what lies
    /// behind the segment: all of it on the edge's side within the
    /// tolerance, or all of it beyond one end.
    fn clear_of(&self, hull: &[Vec3]) -> bool {
        hull.iter().all(|p| self.clearance(*p) >= self.floor)
            || hull.iter().all(|p| self.at(*p) <= 0.0)
            || hull.iter().all(|p| self.at(*p) >= self.length)
    }
}

/// The side whose face's trace is shown to cross the segment between the
/// feet of `section` (at `s`), or to come to its foot from past the
/// segment's line, at a point inside that face (`place`, of the side and
/// a point), if one is; none where a foot lies off its face.
fn wrong_in(
    solver: &Solver<'_>,
    s: f64,
    section: &Section,
    distances: [f64; 2],
    tol: f64,
    place: &dyn Fn(usize, Vec3) -> Option<OnFace>,
) -> Option<usize> {
    let across = solver.spine.at(s)?;
    let (e, normal) = (across.point, across.normal);
    let [g1, g2] = section.feet;
    let along = (g2 - g1).normalized()?;
    let off = e - g1;
    // The line through the edge's point: nothing to hold against.
    let towards = (off - along * off.dot(along)).normalized()?;
    let segment = Segment {
        from: g1,
        along,
        towards,
        length: g1.distance(g2),
        floor: -tol,
    };
    for (k, (side, &foot)) in solver.sides.iter().zip(&section.feet).enumerate() {
        let inside = |p: Vec3| place(k, p) == Some(OnFace::In);
        let held = match trace_of(side.surface, e, normal, foot, tol) {
            Some(Trace::Straight) => Held::Clear,
            Some(Trace::Curve(curve, from, to)) => {
                held_by_hull(&curve, (from, to), &segment, &inside)
            }
            None => held_by_points(solver, across, section, distances, k, &segment, &inside),
        };
        if held == Held::Wrong {
            // A foot off its face: the construction does not cut there
            // (it carries the cut line onto the next face, or the plane
            // has left the face): nothing is shown.
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

/// A face's trace in one plane, from the edge's point to its foot.
enum Trace {
    /// A straight piece.
    Straight,
    /// A piece of a rational curve, between two of its parameters.
    Curve(NurbsCurve, f64, f64),
}

/// The trace of the face on `surface` in the plane through `e` square to
/// `normal`, from `e` to `foot`, as a rational curve where the face's kind
/// gives one (see the module documentation). `None` where it does not.
fn trace_of(surface: &Surface, e: Vec3, normal: Vec3, foot: Vec3, tol: f64) -> Option<Trace> {
    match surface {
        Surface::Plane(_) => Some(Trace::Straight),
        Surface::Cylinder(c) => {
            let circle = NurbsCurve::circle(c.frame(), c.radius()).ok()?;
            carried(&circle, c.frame().z(), e, normal, foot, tol)
        }
        Surface::Bezier(_) | Surface::Nurbs(_) => {
            let (profile, d) = extrusion(surface, tol)?;
            carried(&profile, d, e, normal, foot, tol)
        }
        Surface::Cone(c) => {
            let frame = c.frame();
            let apex = frame.point(0.0, 0.0, c.apex_v());
            let k = normal.dot(e - apex);
            // A plane through the apex meets the cone in its lines.
            if k.abs() <= tol {
                return Some(Trace::Straight);
            }
            // The cone's circle at the edge's point's level, carried
            // from the apex into the plane.
            let (_, v) = c_level(surface, e)?;
            let r = c.radius() + v * c.half_angle().ptan();
            if !(r.abs() > tol) {
                return None;
            }
            let level = Frame::new(frame.point(0.0, 0.0, v), frame.z(), frame.x())?;
            let circle = NurbsCurve::circle(&level, r.abs()).ok()?;
            let mut points = Vec::with_capacity(circle.control_points().len());
            let mut weights = Vec::with_capacity(points.capacity());
            for (p, w) in circle.control_points().iter().zip(circle.weights()) {
                let height = normal.dot(*p - apex);
                if !(height.abs() > 0.0) {
                    return None;
                }
                points.push(apex + (*p - apex) * (k / height));
                weights.push(w * height / k);
            }
            let curve =
                NurbsCurve::new(circle.degree(), circle.knots().to_vec(), points, weights).ok()?;
            ends_on(curve, e, foot, tol)
        }
        Surface::Sphere(s) => {
            let c = s.frame().origin();
            let centre = c - normal * (c - e).dot(normal);
            let radius = (e - centre).length();
            circle_in_plane(centre, radius, normal, e, foot, tol)
        }
        Surface::Torus(t) => {
            // In a plane through its axis: a circle of its tube.
            let (major, minor) = t.radii();
            let (o, axis) = (t.frame().origin(), t.frame().z());
            let through = axis.dot(normal).abs() <= 1e-9 && (o - e).dot(normal).abs() <= tol;
            if !through {
                return None;
            }
            let d = e - o;
            let radial = (d - axis * d.dot(axis)).normalized()?;
            circle_in_plane(o + radial * major, minor, normal, e, foot, tol)
        }
    }
}

/// The parameters of `p` on the cone `surface`.
fn c_level(surface: &Surface, p: Vec3) -> Option<(f64, f64)> {
    surface.project(p).ok()
}

/// The circle about `centre` of `radius` in the plane square to `normal`,
/// as a rational curve, between `e` and `foot`.
fn circle_in_plane(
    centre: Vec3,
    radius: f64,
    normal: Vec3,
    e: Vec3,
    foot: Vec3,
    tol: f64,
) -> Option<Trace> {
    if !(radius > tol) {
        return None;
    }
    let x = (e - centre).normalized()?;
    let frame = Frame::new(centre, normal, x)?;
    let curve = NurbsCurve::circle(&frame, radius).ok()?;
    ends_on(curve, e, foot, tol)
}

/// `profile` (a curve on a surface made of its lines along `d`) carried
/// along `d` into the plane through `e` square to `normal`: the surface's
/// trace in that plane (an affine image: the same weights). A plane along
/// `d` meets the surface in its lines.
fn carried(
    profile: &NurbsCurve,
    d: Vec3,
    e: Vec3,
    normal: Vec3,
    foot: Vec3,
    tol: f64,
) -> Option<Trace> {
    let across = normal.dot(d);
    if across.abs() <= 1e-9 * d.length() {
        return Some(Trace::Straight);
    }
    let moved: Vec<Vec3> = profile
        .control_points()
        .iter()
        .map(|p| *p + d * (normal.dot(e - *p) / across))
        .collect();
    let curve = NurbsCurve::new(
        profile.degree(),
        profile.knots().to_vec(),
        moved,
        profile.weights().to_vec(),
    )
    .ok()?;
    ends_on(curve, e, foot, tol)
}

/// `curve` between the parameters of `e` and `foot` on it, both within
/// the tolerance of it (as they are on the face): the shorter way round a
/// closed curve.
fn ends_on(curve: NurbsCurve, e: Vec3, foot: Vec3, tol: f64) -> Option<Trace> {
    let as_curve = Curve3::Nurbs(curve.clone());
    let from = as_curve.project(e).ok()?;
    let to = as_curve.project(foot).ok()?;
    let on = |t: f64, p: Vec3| curve.eval(t).distance(p) <= tol;
    if !(on(from, e) && on(to, foot)) {
        return None;
    }
    Some(Trace::Curve(curve, from, to))
}

/// The trace held by the convex hull of the control points of its pieces
/// against `segment` (see the module documentation): wrong at a point
/// behind the segment that lies inside the face (`inside`), or where it
/// comes to its foot (the curve's `to`) from past the segment's line and
/// the curve beyond the foot runs into the region between the feet. On
/// a closed curve the piece from the edge's point to the foot is the
/// shorter way round in the parameter; pieces across the seam are held on
/// each side of it.
fn held_by_hull(
    curve: &NurbsCurve,
    (from, to): (f64, f64),
    segment: &Segment,
    inside: &dyn Fn(Vec3) -> bool,
) -> Held {
    let (d0, d1) = curve.domain();
    let (start, end) = (curve.eval(d0), curve.eval(d1));
    let closed = start.distance(end) <= 1e-12 * (1.0 + start.length());
    let (lo, hi) = (from.min(to), from.max(to));
    // On a closed curve, the way round whose middle lies nearer the
    // middle of the two ends (a circle carried from a cone's apex runs out
    // to infinity the other way).
    let (a, b) = (curve.eval(from), curve.eval(to));
    let mid = (a + b) * 0.5;
    let short = curve.eval(0.5 * (lo + hi)).distance(mid);
    let round = {
        let t = 0.5 * (hi + d1 + lo - d0);
        let t = if t > d1 { t - (d1 - d0) } else { t };
        curve.eval(t).distance(mid)
    };
    let wraps = closed && round < short;
    let ranges = if wraps {
        vec![(d0, lo), (hi, d1)]
    } else {
        vec![(lo, hi)]
    };
    // The parameter a part `u` of the way from the edge's point to the
    // foot.
    let period = d1 - d0;
    let at = |u: f64| {
        if wraps {
            let way = if from < to { -1.0 } else { 1.0 };
            let t = from + way * u * (period - (hi - lo));
            if t < d0 {
                t + period
            } else if t > d1 {
                t - period
            } else {
                t
            }
        } else {
            from + u * (to - from)
        }
    };
    // Past the foot, as far again; an open curve ends with its domain.
    let along = |u: f64| {
        let t = at(u);
        (wraps || (t >= d0 && t <= d1)).then(|| curve.eval(t))
    };
    if runs_in_beyond(&along, segment, inside) {
        return Held::Wrong;
    }
    let mut stack: Vec<(f64, f64, u32)> = ranges.into_iter().map(|(a, b)| (a, b, 0)).collect();
    let mut pieces = 0_usize;
    let mut undecided = false;
    while let Some((a, b, depth)) = stack.pop() {
        pieces += 1;
        if pieces > PIECES {
            return Held::Undecided;
        }
        let points = [a, 0.5 * (a + b), b].map(|t| curve.eval(t));
        // A point of the curve behind the segment, inside the face, shows
        // it.
        if points.iter().any(|p| segment.behind(*p) && inside(*p)) {
            return Held::Wrong;
        }
        if let Some(hull) = curve.hull_points((a, b))
            && segment.clear_of(&hull)
        {
            continue;
        }
        // Not clear, and none of its points inside the face: taken to
        // lie past the face.
        if !points.iter().any(|p| inside(*p)) {
            continue;
        }
        let middle = 0.5 * (a + b);
        if depth >= DEPTH || !(middle > a && middle < b) {
            undecided = true;
            continue;
        }
        stack.push((middle, b, depth + 1));
        stack.push((a, middle, depth + 1));
    }
    if undecided {
        Held::Undecided
    } else {
        Held::Clear
    }
}

/// Whether the face beyond the foot runs into the region between the
/// feet. `point` of a part of the way along the trace from the edge's
/// point (0) to the foot (1), and on past it (to 2; `None` where the face's
/// curve ends). Where the trace comes to its foot from past the segment's
/// line (its point just before the foot on the far side), the curve beyond
/// the foot is followed at parts closer and closer to the foot and then in
/// steps of 1/256, while it lies on the edge's side between the segment's
/// ends: deeper than the tolerance at a point inside the face, it runs in.
/// For a circle or a conic the stretch before the foot and the face beyond
/// it make one lens; for a trace that bends both ways they do not (the
/// stretch before can lie far past the line along its extension, a bead,
/// while the face beyond only grazes the region), and what decides is the
/// face beyond.
fn runs_in_beyond(
    point: &dyn Fn(f64) -> Option<Vec3>,
    segment: &Segment,
    inside: &dyn Fn(Vec3) -> bool,
) -> bool {
    const CLOSE: i32 = 24;
    const STEPS: u32 = 256;
    let Some(before) = point(1.0 - 0.5_f64.ppowi(CLOSE)) else {
        return false;
    };
    if !(segment.clearance(before) < 0.0) {
        return false;
    }
    let near = (8..=CLOSE).rev().map(|k| 1.0 + 0.5_f64.ppowi(k));
    let far = (2..=STEPS).map(|i| 1.0 + f64::from(i) / f64::from(STEPS));
    let mut deepest: Option<(f64, Vec3)> = None;
    for u in near.chain(far) {
        let Some(p) = point(u) else {
            break;
        };
        let (c, at) = (segment.clearance(p), segment.at(p));
        if !(c > 0.0 && at > 0.0 && at < segment.length) {
            break;
        }
        if deepest.is_none_or(|(d, _)| c > d) {
            deepest = Some((c, p));
        }
    }
    deepest.is_some_and(|(c, p)| c > -segment.floor && inside(p))
}

/// The trace of the face on side `k` held by [`SAMPLES`] of its points
/// against `segment`: each where the construction finds a foot (straight
/// in the plane) at a part of the distance, followed from the one before,
/// from the foot back towards the edge, and beyond the foot as far again.
/// Wrong at a point behind the segment inside the face (`inside`), or
/// where the point next to the foot lies past the segment's line and the
/// face beyond the foot runs into the region between the feet
/// ([`runs_in_beyond`]).
fn held_by_points(
    solver: &Solver<'_>,
    across: super::Across,
    section: &Section,
    distances: [f64; 2],
    k: usize,
    segment: &Segment,
    inside: &dyn Fn(Vec3) -> bool,
) -> Held {
    let mut near = *section;
    let mut before = None;
    for i in (1..SAMPLES).rev() {
        let part = f64::from(i) / f64::from(SAMPLES);
        let Some(found) = solver.cut(across, &near, distances.map(|d| d * part)) else {
            return Held::Undecided;
        };
        if let Some(p) = found.feet.get(k) {
            if before.is_none() {
                before = Some(*p);
            }
            if segment.behind(*p) && inside(*p) {
                return Held::Wrong;
            }
        }
        near = found;
    }
    // Beyond the foot, only where the trace comes to it from past the line.
    if !before.is_some_and(|p| segment.clearance(p) < 0.0) {
        return Held::Clear;
    }
    let mut near = *section;
    let mut deepest: Option<(f64, Vec3)> = None;
    for i in 1..=SAMPLES {
        let part = 1.0 + f64::from(i) / f64::from(SAMPLES);
        let Some(found) = solver.cut(across, &near, distances.map(|d| d * part)) else {
            break;
        };
        let Some(p) = found.feet.get(k).copied() else {
            break;
        };
        let (c, at) = (segment.clearance(p), segment.at(p));
        if !(c > 0.0 && at > 0.0 && at < segment.length) {
            break;
        }
        if deepest.is_none_or(|(d, _)| c > d) {
            deepest = Some((c, p));
        }
        near = found;
    }
    if deepest.is_some_and(|(c, p)| c > -segment.floor && inside(p)) {
        Held::Wrong
    } else {
        Held::Clear
    }
}

/// A spline surface that is an extrusion: one direction of degree one
/// with two rows of control points, each point of the second row the
/// first one's moved by one vector, with the same weight. The profile (the
/// first row's curve) and that vector; `None` otherwise.
fn extrusion(surface: &Surface, tol: f64) -> Option<(NurbsCurve, Vec3)> {
    let owned;
    let nurbs = match surface {
        Surface::Nurbs(n) => n,
        Surface::Bezier(patch) => {
            owned = NurbsSurface::from_bezier(patch).ok()?;
            &owned
        }
        _ => return None,
    };
    let (du, dv) = nurbs.degrees();
    let (nu, nv) = nurbs.counts();
    let (ctrl, weights) = (nurbs.control_points(), nurbs.weights());
    // Which direction is linear; the profile runs along the other.
    let linear_v = if dv == 1 && nv == 2 {
        true
    } else if du == 1 && nu == 2 {
        false
    } else {
        return None;
    };
    let (count, degree, knots) = if linear_v {
        (nu, du, nurbs.knots_u().to_vec())
    } else {
        (nv, dv, nurbs.knots_v().to_vec())
    };
    // The index of the profile's point `i` in the linear direction's row.
    let index = |i: usize, row: usize| {
        if linear_v {
            i.checked_mul(nv)?.checked_add(row)
        } else {
            row.checked_mul(nv)?.checked_add(i)
        }
    };
    let mut profile = Vec::with_capacity(count);
    let mut profile_weights = Vec::with_capacity(count);
    let mut shift: Option<Vec3> = None;
    for i in 0..count {
        let (first, second) = (index(i, 0)?, index(i, 1)?);
        let (p, q) = (ctrl.get(first)?, ctrl.get(second)?);
        let (w0, w1) = (weights.get(first)?, weights.get(second)?);
        if !((w0 - w1).abs() <= 1e-12 * w0.abs()) {
            return None;
        }
        let d = *q - *p;
        match shift {
            None => shift = Some(d),
            Some(s) => {
                if (d - s).length() > 1e-3 * tol {
                    return None;
                }
            }
        }
        profile.push(*p);
        profile_weights.push(*w0);
    }
    let d = shift?;
    if !(d.length() > tol) {
        return None;
    }
    Some((
        NurbsCurve::new(degree, knots, profile, profile_weights).ok()?,
        d,
    ))
}
