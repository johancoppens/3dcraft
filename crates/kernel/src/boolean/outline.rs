//! A face, or a piece of one, as the booleans see it: a planar or
//! cylindrical surface and the loops of curve pieces that bound it, with
//! the questions asked of it: is this point of the surface inside, which
//! way round does a loop go, where is a point safely inside.
//!
//! Inside or outside on a plane is the winding number of the boundary about
//! the point, summed as the angle each piece turns through; it has no
//! special cases for a point level with a vertex. On a cylinder it is the
//! parity of the boundary pieces met going up the ruling through the point;
//! which pieces span that ruling is decided by comparing angles computed
//! once per vertex, so two pieces that share a vertex never both claim, or
//! both miss, a ruling through it.
//!
//! On a cylinder a circle or ellipse piece is a section of that cylinder,
//! on which the curve parameter and the cylinder's angle run at the same
//! rate (one way or the other); `craft3d_geom::intersect` makes them so and
//! tests it.

use super::Reason;
use craft3d_geom::{Curve3, Surface};
use craft3d_math::quadrature::Rule;
use craft3d_math::{Bbox3, Frame, Vec3};
use std::f64::consts::{FRAC_PI_4, TAU};

/// A piece of a curve in a loop.
#[derive(Clone, Debug)]
pub(super) struct Seg {
    pub curve: Curve3,
    pub range: (f64, f64),
    /// The vertex positions at `range.0` and `range.1`.
    pub a: Vec3,
    pub b: Vec3,
    /// Both ends are one vertex: a whole closed curve.
    pub closed: bool,
    /// The loop runs against the curve.
    pub reversed: bool,
}

impl Seg {
    pub fn middle(&self) -> f64 {
        0.5 * (self.range.0 + self.range.1)
    }

    /// The direction the loop runs in at the middle of the piece.
    pub fn travel(&self) -> (Vec3, Vec3) {
        let (point, tangent) = self.curve.derivatives(self.middle());
        (point, if self.reversed { -tangent } else { tangent })
    }

    fn sign(&self) -> f64 {
        if self.reversed { -1.0 } else { 1.0 }
    }
}

#[derive(Clone, Debug)]
pub(super) struct Outline {
    pub surface: Surface,
    /// The outward normal is opposite to the surface's.
    pub reversed: bool,
    pub loops: Vec<Vec<Seg>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Side {
    In,
    /// Within the tolerance of the boundary.
    On,
    Out,
}

pub(super) const NOT_PLANE_OR_CYLINDER: Reason =
    "a face that is not planar or cylindrical; booleans handle planes and cylinders so far";
const NOT_A_SECTION: Reason =
    "an edge of a cylindrical face that is not a ruling or a plane section of the cylinder";
const TOO_CLOSE: Reason =
    "a point too close to a face boundary to tell inside from outside (at the edge of tolerance)";

fn wrap(a: f64) -> f64 {
    let w = a.rem_euclid(TAU);
    if w >= TAU { 0.0 } else { w }
}

/// `t` moved by whole periods to where it is closest to `range`.
pub(super) fn lift(curve: &Curve3, range: (f64, f64), t: f64) -> f64 {
    let Some(period) = curve.period() else {
        return t;
    };
    let turns = ((0.5 * (range.0 + range.1) - t) / period).round();
    if turns.is_finite() {
        t + turns * period
    } else {
        t
    }
}

/// The parameter within `range` of the point of the curve piece nearest to
/// `p`, and its distance from `p`.
pub(super) fn nearest(curve: &Curve3, range: (f64, f64), p: Vec3) -> Option<(f64, f64)> {
    let t = lift(curve, range, curve.project(p).ok()?);
    let t = t.max(range.0).min(range.1);
    let d = curve.eval(t).distance(p);
    d.is_finite().then_some((t, d))
}

/// The parameter of `p` on the curve piece, when it is within `tol` of it.
pub(super) fn locate(curve: &Curve3, range: (f64, f64), p: Vec3, tol: f64) -> Option<f64> {
    nearest(curve, range, p)
        .filter(|&(_, d)| d <= tol)
        .map(|(t, _)| t)
}

/// The distance from `p` to the surface; infinite when it has none.
pub(super) fn surface_distance(surface: &Surface, p: Vec3) -> f64 {
    match surface.project(p) {
        Ok((u, v)) => {
            let d = surface.eval(u, v).distance(p);
            if d.is_finite() { d } else { f64::INFINITY }
        }
        Err(_) => f64::INFINITY,
    }
}

/// The point of the surface nearest to `p`.
fn onto(surface: &Surface, p: Vec3) -> Option<Vec3> {
    let (u, v) = surface.project(p).ok()?;
    let q = surface.eval(u, v);
    q.is_finite().then_some(q)
}

/// The surface normal at (the projection of) `p`.
pub(super) fn surface_normal(surface: &Surface, p: Vec3) -> Option<Vec3> {
    let (u, v) = surface.project(p).ok()?;
    surface.normal(u, v)
}

/// A cylinder's angle and height of `p`.
fn angle_height(frame: &Frame, p: Vec3) -> (f64, f64) {
    let (x, y, z) = frame.local(p);
    let u = if x == 0.0 && y == 0.0 {
        0.0
    } else {
        wrap(y.atan2(x))
    };
    (u, z)
}

/// True when `u` is in the stretch that runs counter-clockwise from `lo` up
/// to, not including, `hi`. Comparisons only, so that stretches which share
/// an end never overlap or leave a gap.
fn within(u: f64, lo: f64, hi: f64) -> bool {
    if lo < hi {
        lo <= u && u < hi
    } else if lo > hi {
        u >= lo || u < hi
    } else {
        false
    }
}

/// A piece of a cylindrical face's boundary, seen in the cylinder's (u, v).
struct Section {
    /// +1 when the curve parameter runs with u, −1 against.
    with_u: f64,
    /// The plane of the circle or ellipse.
    center: Vec3,
    normal: Vec3,
}

fn section(seg: &Seg, frame: &Frame) -> Result<Option<Section>, Reason> {
    let plane = match &seg.curve {
        Curve3::Line(_) => return Ok(None), // a ruling
        Curve3::Circle(c) => c.frame(),
        Curve3::Ellipse(e) => e.frame(),
        Curve3::Nurbs(_) => return Err(NOT_A_SECTION),
    };
    let (point, tangent) = seg.curve.derivatives(seg.middle());
    let (x, y, _) = frame.local(point);
    let around = frame.vector(-y, x, 0.0);
    let rate = tangent.dot(around);
    let rise = plane.z().dot(frame.z());
    if !(rate.abs() > 1e-9 * tangent.length() * around.length() && rise.abs() > 1e-9) {
        return Err(NOT_A_SECTION);
    }
    Ok(Some(Section {
        with_u: rate.signum(),
        center: plane.origin(),
        normal: plane.z(),
    }))
}

/// The height at which the piece crosses the ruling at angle `u`, when it
/// spans that ruling.
fn crossing(seg: &Seg, frame: &Frame, radius: f64, u: f64) -> Result<Option<f64>, Reason> {
    let Some(section) = section(seg, frame)? else {
        return Ok(None);
    };
    if !seg.closed {
        let (ua, ub) = (angle_height(frame, seg.a).0, angle_height(frame, seg.b).0);
        let (lo, hi) = if section.with_u > 0.0 {
            (ua, ub)
        } else {
            (ub, ua)
        };
        if !within(u, lo, hi) {
            return Ok(None);
        }
    }
    let (s, c) = u.sin_cos();
    let foot = frame.point(radius * c, radius * s, 0.0);
    let v = (section.center - foot).dot(section.normal) / frame.z().dot(section.normal);
    if v.is_finite() {
        Ok(Some(v))
    } else {
        Err(NOT_A_SECTION)
    }
}

/// The highest crossing of a loop with the ruling at angle `u`.
pub(super) fn top_crossing(
    surface: &Surface,
    loop_: &[Seg],
    u: f64,
) -> Result<Option<f64>, Reason> {
    let Surface::Cylinder(cylinder) = surface else {
        return Err(NOT_PLANE_OR_CYLINDER);
    };
    let mut top: Option<f64> = None;
    for seg in loop_ {
        if let Some(v) = crossing(seg, cylinder.frame(), cylinder.radius(), u)? {
            top = Some(top.map_or(v, |t| t.max(v)));
        }
    }
    Ok(top)
}

/// How many times a loop on a cylinder goes around the axis, in the sense
/// of the cylinder's angle.
pub(super) fn turns(surface: &Surface, loop_: &[Seg]) -> Result<i32, Reason> {
    let Surface::Cylinder(cylinder) = surface else {
        return Ok(0);
    };
    let mut total = 0.0;
    for seg in loop_ {
        if let Some(section) = section(seg, cylinder.frame())? {
            total += seg.sign() * section.with_u * (seg.range.1 - seg.range.0);
        }
    }
    let turns = (total / TAU).round();
    if turns.abs() <= 1.0 {
        Ok(turns as i32)
    } else {
        Err("a loop that winds around a cylinder more than once")
    }
}

/// The most evaluations the winding sum spends on one curve piece.
const MAX_TURN_STEPS: usize = 4096;

/// The angle the direction from `q` to a point moving along the piece (in
/// the curve's own direction) turns through, in the plane's coordinates.
fn turn(seg: &Seg, frame: &Frame, q: (f64, f64)) -> Result<f64, Reason> {
    let flat = |p: Vec3| {
        let (x, y, _) = frame.local(p);
        (x - q.0, y - q.1)
    };
    let angle = |a: (f64, f64), b: (f64, f64)| (a.0 * b.1 - a.1 * b.0).atan2(a.0 * b.0 + a.1 * b.1);
    let reach = match &seg.curve {
        Curve3::Line(_) => return Ok(angle(flat(seg.a), flat(seg.b))),
        Curve3::Circle(c) => c.radius(),
        Curve3::Ellipse(e) => e.semi_axes().0.max(e.semi_axes().1),
        Curve3::Nurbs(_) => {
            return Err("a NURBS edge; booleans handle lines, circles and ellipses");
        }
    };
    // A stretch that stays within a disc that does not contain `q` turns
    // by less than half a turn, which is what `angle` assumes. Halve the
    // stretches until they do.
    let mut total = 0.0;
    let mut stack = vec![(seg.range.0, flat(seg.a), seg.range.1, flat(seg.b))];
    let mut steps = 0_usize;
    while let Some((t0, p0, t1, p1)) = stack.pop() {
        steps += 1;
        if steps > MAX_TURN_STEPS {
            return Err(TOO_CLOSE);
        }
        let length = reach * (t1 - t0);
        if p0.0.hypot(p0.1) > length {
            total += angle(p0, p1);
        } else {
            let mid = 0.5 * (t0 + t1);
            if !(mid > t0 && mid < t1) {
                return Err(TOO_CLOSE);
            }
            let pm = flat(seg.curve.eval(mid));
            stack.push((mid, pm, t1, p1));
            stack.push((t0, p0, mid, pm));
        }
    }
    Ok(total)
}

/// The smallest distance from `p` to the loops.
pub(super) fn clearance(loops: &[Vec<Seg>], p: Vec3) -> f64 {
    let mut least = f64::INFINITY;
    for seg in loops.iter().flatten() {
        let d = nearest(&seg.curve, seg.range, p).map_or(0.0, |(_, d)| d);
        least = least.min(d);
    }
    least
}

/// Where a point of the surface is with respect to the loops.
pub(super) fn side_of(
    surface: &Surface,
    loops: &[Vec<Seg>],
    p: Vec3,
    tol: f64,
) -> Result<Side, Reason> {
    if clearance(loops, p) <= tol {
        return Ok(Side::On);
    }
    match surface {
        Surface::Plane(plane) => {
            let frame = plane.frame();
            let (x, y, _) = frame.local(p);
            let mut total = 0.0;
            for seg in loops.iter().flatten() {
                total += seg.sign() * turn(seg, frame, (x, y))?;
            }
            let winding = (total / TAU).round();
            if !winding.is_finite() {
                return Err(TOO_CLOSE);
            }
            Ok(if winding == 0.0 { Side::Out } else { Side::In })
        }
        Surface::Cylinder(cylinder) => {
            let frame = cylinder.frame();
            let (u, v) = angle_height(frame, p);
            let mut above = 0_usize;
            for seg in loops.iter().flatten() {
                if crossing(seg, frame, cylinder.radius(), u)?.is_some_and(|at| at > v) {
                    above += 1;
                }
            }
            Ok(if above % 2 == 1 { Side::In } else { Side::Out })
        }
        _ => Err(NOT_PLANE_OR_CYLINDER),
    }
}

fn panels(seg: &Seg) -> usize {
    (((seg.range.1 - seg.range.0) / FRAC_PI_4).ceil() as usize).clamp(1, 16)
}

/// The area a loop encloses, positive when it runs counter-clockwise seen
/// from outside the face. On a cylinder the area is taken in (angle,
/// height) and is meaningful for loops that do not go around the axis.
pub(super) fn signed_area(surface: &Surface, reversed: bool, loop_: &[Seg]) -> Result<f64, Reason> {
    let rule = Rule::new(8);
    let Some(origin) = loop_.first().map(|s| s.a) else {
        return Ok(0.0);
    };
    let mut area = 0.0;
    match surface {
        Surface::Plane(plane) => {
            let frame = plane.frame();
            let flat = |p: Vec3| {
                let (x, y, _) = frame.local(p);
                let (ox, oy, _) = frame.local(origin);
                (x - ox, y - oy)
            };
            for seg in loop_ {
                let part = match &seg.curve {
                    Curve3::Line(_) => {
                        let (a, b) = (flat(seg.a), flat(seg.b));
                        0.5 * (a.0 * b.1 - a.1 * b.0)
                    }
                    _ => rule.integrate(seg.range.0, seg.range.1, panels(seg), |t| {
                        let (point, tangent) = seg.curve.derivatives(t);
                        let (x, y) = flat(point);
                        let (dx, dy) = (tangent.dot(frame.x()), tangent.dot(frame.y()));
                        0.5 * (x * dy - y * dx)
                    }),
                };
                area += seg.sign() * part;
            }
        }
        Surface::Cylinder(cylinder) => {
            let frame = cylinder.frame();
            let base = angle_height(frame, origin).1;
            for seg in loop_ {
                let Some(section) = section(seg, frame)? else {
                    continue;
                };
                // −∮ v du, with du = ±dt along a section.
                let part = rule.integrate(seg.range.0, seg.range.1, panels(seg), |t| {
                    angle_height(frame, seg.curve.eval(t)).1 - base
                });
                area -= seg.sign() * section.with_u * part * cylinder.radius();
            }
        }
        _ => return Err(NOT_PLANE_OR_CYLINDER),
    }
    if !area.is_finite() {
        return Err("a loop whose area is not finite");
    }
    Ok(if reversed { -area } else { area })
}

/// A point of the face well inside the loops: at least `margin` from all of
/// them. Tried from the middle of each boundary piece, stepping into the
/// face by ever smaller amounts.
pub(super) fn interior_point(outline: &Outline, tol: f64, margin: f64) -> Result<Vec3, Reason> {
    const SLIVER: Reason =
        "a piece of a face too thin to find a point inside it (at the edge of tolerance)";
    for seg in outline.loops.iter().flatten() {
        let (point, travel) = seg.travel();
        let Some(normal) = surface_normal(&outline.surface, point) else {
            continue;
        };
        let outward = if outline.reversed { -normal } else { normal };
        // Seen from outside the face lies to the left of its boundary.
        let Some(inward) = outward.cross(travel).normalized() else {
            continue;
        };
        let quarter = |k: f64| {
            seg.curve
                .eval(seg.range.0 + (seg.range.1 - seg.range.0) * k)
        };
        let mut step = quarter(0.25).distance(quarter(0.75));
        for _ in 0..64 {
            step *= 0.5;
            if !(step.is_finite() && step > margin) {
                break;
            }
            let Some(candidate) = onto(&outline.surface, point + inward * step) else {
                continue;
            };
            if clearance(&outline.loops, candidate) < margin {
                continue;
            }
            if side_of(&outline.surface, &outline.loops, candidate, tol)? == Side::In {
                return Ok(candidate);
            }
        }
    }
    Err(SLIVER)
}

/// A box around everything the loops reach. An arc counts as its whole
/// circle or ellipse, which also covers the bulge of a cylindrical face.
pub(super) fn bbox(loops: &[Vec<Seg>]) -> Bbox3 {
    let mut b = Bbox3::EMPTY;
    for seg in loops.iter().flatten() {
        b.add(seg.a);
        b.add(seg.b);
        let (center, reach) = match &seg.curve {
            Curve3::Line(_) | Curve3::Nurbs(_) => continue,
            Curve3::Circle(c) => (c.frame().origin(), c.radius()),
            Curve3::Ellipse(e) => (e.frame().origin(), e.semi_axes().0.max(e.semi_axes().1)),
        };
        let reach = Vec3::new(reach, reach, reach);
        b.add(center - reach);
        b.add(center + reach);
    }
    b
}

/// True when two boxes come within `margin` of each other.
pub(super) fn boxes_meet(a: &Bbox3, b: &Bbox3, margin: f64) -> bool {
    let (Some((alo, ahi)), Some((blo, bhi))) = (a.corners(), b.corners()) else {
        return false;
    };
    alo.x <= bhi.x + margin
        && blo.x <= ahi.x + margin
        && alo.y <= bhi.y + margin
        && blo.y <= ahi.y + margin
        && alo.z <= bhi.z + margin
        && blo.z <= ahi.z + margin
}

/// True when the infinite line through `origin` along `direction` passes
/// within `margin` of the box.
pub(super) fn line_meets_box(origin: Vec3, direction: Vec3, b: &Bbox3, margin: f64) -> bool {
    let Some((lo, hi)) = b.corners() else {
        return false;
    };
    let (mut enter, mut leave) = (f64::NEG_INFINITY, f64::INFINITY);
    for (o, d, l, h) in [
        (origin.x, direction.x, lo.x, hi.x),
        (origin.y, direction.y, lo.y, hi.y),
        (origin.z, direction.z, lo.z, hi.z),
    ] {
        let (l, h) = (l - margin, h + margin);
        if d == 0.0 {
            if o < l || o > h {
                return false;
            }
        } else {
            let (t0, t1) = ((l - o) / d, (h - o) / d);
            enter = enter.max(t0.min(t1));
            leave = leave.min(t0.max(t1));
        }
    }
    enter <= leave
}
