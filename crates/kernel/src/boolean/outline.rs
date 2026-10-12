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
use craft3d_geom::{Curve3, Ellipse, Surface};
use craft3d_math::quadrature::Rule;
use craft3d_math::{Bbox3, Frame, Vec3};
use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, TAU};

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
    if let Curve3::Ellipse(ellipse) = curve {
        return nearest_on_ellipse(ellipse, range, p);
    }
    // On a line or a circle the distance grows with the parameter's
    // distance from the nearest point of the whole curve, so clamping
    // that into the range gives the nearest point of the piece.
    let t = lift(curve, range, curve.project(p).ok()?);
    let t = t.max(range.0).min(range.1);
    let d = curve.eval(t).distance(p);
    d.is_finite().then_some((t, d))
}

/// [`nearest`] for a piece of an ellipse, where the nearest point of the
/// whole curve says little about the nearest point of a piece, and where a
/// long ellipse has several near points close together around its tips.
///
/// Branch and bound: a stretch of the curve of length `l` between two
/// points at distances `d0` and `d1` comes no closer than
/// `min(d0, d1) − l / 2`. Stretches that cannot beat the best point found
/// are dropped, the others halved; Newton's method polishes the result.
fn nearest_on_ellipse(ellipse: &Ellipse, range: (f64, f64), p: Vec3) -> Option<(f64, f64)> {
    let (a, b) = ellipse.semi_axes();
    let (x, y, z) = ellipse.frame().local(p);
    if !(x.is_finite() && y.is_finite() && z.is_finite()) {
        return None;
    }
    let distance = |t: f64| {
        let (s, c) = t.sin_cos();
        (a * c - x).hypot(b * s - y)
    };
    let speed = |t: f64| {
        let (s, c) = t.sin_cos();
        (a * s).hypot(b * c)
    };
    // The speed is monotone between multiples of a quarter turn.
    let fastest = |t0: f64, t1: f64| {
        if (t0 / FRAC_PI_2).floor() != (t1 / FRAC_PI_2).floor() {
            a.max(b)
        } else {
            speed(t0).max(speed(t1))
        }
    };
    let (t0, t1) = range;
    let (d0, d1) = (distance(t0), distance(t1));
    let mut best = if d0 <= d1 { (d0, t0) } else { (d1, t1) };
    let mut stack = vec![(t0, d0, t1, d1)];
    let mut steps = 0_usize;
    while let Some((ta, da, tb, db)) = stack.pop() {
        steps += 1;
        let length = fastest(ta, tb) * (tb - ta);
        // Not worth looking at: cannot be closer, or as short as rounding.
        if steps > 20_000
            || da.min(db) - 0.5 * length >= best.0
            || length <= (1e-3 * best.0).max(1e-7 * a.max(b))
        {
            continue;
        }
        let tm = 0.5 * (ta + tb);
        if !(tm > ta && tm < tb) {
            continue;
        }
        let dm = distance(tm);
        if dm < best.0 {
            best = (dm, tm);
        }
        stack.push((ta, da, tm, dm));
        stack.push((tm, dm, tb, db));
    }
    let mut t = best.1;
    for _ in 0..40 {
        let (s, c) = t.sin_cos();
        let (dx, dy) = (a * c - x, b * s - y);
        // Half the derivative of the squared distance, and its slope.
        let g = -dx * a * s + dy * b * c;
        let slope = a * a * s * s + b * b * c * c - dx * a * c - dy * b * s;
        if !(slope.is_finite() && slope > 0.0) {
            break;
        }
        let next = (t - g / slope).max(t0).min(t1);
        let moved = (next - t).abs();
        t = next;
        if moved <= 1e-15 {
            break;
        }
    }
    let d = distance(t);
    let (d, t) = if d <= best.0 { (d, t) } else { best };
    let distance = d.hypot(z);
    distance.is_finite().then_some((t, distance))
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

/// An angle as far as can be from the ends of all pieces of the loops: the
/// middle of the widest gap between them.
pub(super) fn clear_angle(surface: &Surface, loops: &[&[Seg]]) -> f64 {
    let Surface::Cylinder(cylinder) = surface else {
        return 0.0;
    };
    let mut angles: Vec<f64> = loops
        .iter()
        .flat_map(|l| l.iter())
        .flat_map(|seg| [seg.a, seg.b])
        .map(|p| angle_height(cylinder.frame(), p).0)
        .collect();
    angles.sort_by(f64::total_cmp);
    let (Some(&first), Some(&last)) = (angles.first(), angles.last()) else {
        return 0.0;
    };
    // The gap across the seam first, then those between neighbours.
    let mut best = (first + TAU - last, wrap(last + 0.5 * (first + TAU - last)));
    for (a, b) in angles.iter().zip(angles.iter().skip(1)) {
        if b - a > best.0 {
            best = (b - a, a + 0.5 * (b - a));
        }
    }
    best.1
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
    let clear = clearance(loops, p);
    if clear <= tol {
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
            // The point may be in line with a ruling of the boundary (it
            // often is: on the continuation of an edge). The two ends of
            // a ruling do not have the same angle to the last bit, so
            // which pieces span a ruling through them is not reliable.
            // Everything within `clear` of the point is on the same side
            // as the point: ask on either side of it, halfway there.
            let nudge = (0.5 * clear / cylinder.radius()).min(0.5);
            let mut sides = [false, false];
            for (side, u) in sides.iter_mut().zip([wrap(u - nudge), wrap(u + nudge)]) {
                let mut above = 0_usize;
                for seg in loops.iter().flatten() {
                    if crossing(seg, frame, cylinder.radius(), u)?.is_some_and(|at| at > v) {
                        above += 1;
                    }
                }
                *side = above % 2 == 1;
            }
            match sides {
                [true, true] => Ok(Side::In),
                [false, false] => Ok(Side::Out),
                _ => Err(TOO_CLOSE),
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_geom::{Circle, Cylinder, Line, Plane};

    fn line(a: Vec3, b: Vec3, reversed: bool) -> Seg {
        Seg {
            curve: Curve3::Line(Line::through(a, b).unwrap()),
            range: (0.0, a.distance(b)),
            a,
            b,
            closed: false,
            reversed,
        }
    }

    fn arc(frame: Frame, radius: f64, range: (f64, f64), reversed: bool) -> Seg {
        let curve = Curve3::Circle(Circle::new(frame, radius).unwrap());
        Seg {
            a: curve.eval(range.0),
            b: curve.eval(range.1),
            closed: range.1 - range.0 >= TAU,
            curve,
            range,
            reversed,
        }
    }

    /// A 4 by 2 rectangle in the plane of `frame` with a half-disc bulging
    /// out of its right side and a round hole.
    fn plate(frame: Frame) -> (Surface, Vec<Vec<Seg>>) {
        let p = |x: f64, y: f64| frame.point(x, y, 0.0);
        let bulge = frame.at(p(4.0, 1.0)).unwrap();
        let hole = frame.at(p(1.0, 1.0)).unwrap();
        let outer = vec![
            line(p(0.0, 0.0), p(4.0, 0.0), false),
            arc(bulge, 1.0, (-FRAC_PI_2, FRAC_PI_2), false),
            line(p(4.0, 2.0), p(0.0, 2.0), false),
            line(p(0.0, 2.0), p(0.0, 0.0), false),
        ];
        let inner = vec![arc(hole, 0.5, (0.0, TAU), true)];
        (Surface::Plane(Plane::new(frame)), vec![outer, inner])
    }

    fn tilted() -> Frame {
        Frame::new(
            Vec3::new(3.0, -1.0, 2.0),
            Vec3::new(0.2, -0.5, 1.0),
            Vec3::new(1.0, 0.3, 0.0),
        )
        .unwrap()
    }

    #[test]
    fn inside_on_and_outside_of_a_planar_outline() {
        for frame in [Frame::WORLD, tilted()] {
            let (surface, loops) = plate(frame);
            let side = |x: f64, y: f64| side_of(&surface, &loops, frame.point(x, y, 0.0), 1e-6);
            for (x, y) in [(2.5, 1.0), (4.9, 1.0), (0.1, 0.1), (1.0, 1.6), (4.5, 1.8)] {
                assert_eq!(side(x, y), Ok(Side::In), "({x}, {y})");
            }
            // In the hole, beyond the edges, beside the bulge, and level
            // with vertices (which a ray would have to step around).
            for (x, y) in [
                (1.0, 1.0),
                (1.2, 0.8),
                (-0.5, 1.0),
                (5.5, 1.0),
                (4.8, 1.9),
                (6.0, 0.0),
                (-1.0, 2.0),
                (2.0, -3.0),
            ] {
                assert_eq!(side(x, y), Ok(Side::Out), "({x}, {y})");
            }
            for (x, y) in [(2.0, 0.0), (5.0, 1.0), (1.5, 1.0), (0.0, 0.0), (0.0, 2.0)] {
                assert_eq!(side(x, y), Ok(Side::On), "({x}, {y})");
            }
            // A hair inside the boundary is inside, at any distance scale.
            assert_eq!(side(2.0, 3e-6), Ok(Side::In));
            assert_eq!(side(4.999_997, 1.0), Ok(Side::In));
            assert_eq!(side(1.500_003, 1.0), Ok(Side::In));
            assert_eq!(side(1.499_997, 1.0), Ok(Side::Out));
            // Counter-clockwise outside, clockwise hole; exact areas.
            let area = signed_area(&surface, false, &loops[0]).unwrap();
            assert!((area - (8.0 + FRAC_PI_2)).abs() < 1e-12, "{area}");
            let hole = signed_area(&surface, false, &loops[1]).unwrap();
            assert!((hole + FRAC_PI_4).abs() < 1e-12, "{hole}");
            assert!(signed_area(&surface, true, &loops[0]).unwrap() < 0.0);
            let outline = Outline {
                surface: surface.clone(),
                reversed: false,
                loops: loops.clone(),
            };
            let inside = interior_point(&outline, 1e-6, 4e-6).unwrap();
            assert_eq!(side_of(&surface, &loops, inside, 1e-6), Ok(Side::In));
            assert!(clearance(&loops, inside) >= 4e-6);
        }
    }

    /// A patch of a cylinder between two rulings and two circles, with a
    /// step in its upper rim: the rulings of the step are in line with
    /// points further up and down the cylinder.
    fn patch(frame: Frame) -> (Surface, Vec<Vec<Seg>>) {
        let r = 2.0;
        let at = |u: f64, v: f64| frame.point(r * u.cos(), r * u.sin(), v);
        let ring = |v: f64| frame.at(frame.point(0.0, 0.0, v)).unwrap();
        let outer = vec![
            arc(ring(0.0), r, (0.5, 2.5), false),
            line(at(2.5, 0.0), at(2.5, 2.0), false),
            arc(ring(2.0), r, (1.5, 2.5), true),
            line(at(1.5, 1.0), at(1.5, 2.0), true),
            arc(ring(1.0), r, (0.5, 1.5), true),
            line(at(0.5, 0.0), at(0.5, 1.0), true),
        ];
        (
            Surface::Cylinder(Cylinder::new(frame, r).unwrap()),
            vec![outer],
        )
    }

    #[test]
    fn inside_on_and_outside_of_a_cylindrical_outline() {
        for frame in [Frame::WORLD, tilted()] {
            let (surface, loops) = patch(frame);
            let r = 2.0;
            let side = |u: f64, v: f64| {
                let p = frame.point(r * u.cos(), r * u.sin(), v);
                side_of(&surface, &loops, p, 1e-6)
            };
            for (u, v) in [
                (1.0, 0.5),
                (2.0, 1.5),
                (2.0, 0.5),
                (1.5, 0.5),
                (2.5 - 1e-5, 1.0),
            ] {
                assert_eq!(side(u, v), Ok(Side::In), "({u}, {v})");
            }
            // Beside, above and below it; on the other side of the
            // cylinder; and in line with each of the three rulings.
            for (u, v) in [
                (1.0, 1.5),
                (0.2, 0.5),
                (3.0, 0.5),
                (4.5, 0.5),
                (1.0, -0.5),
                (2.0, 2.5),
                (1.5, 2.5),
                (1.5, -1.0),
                (0.5, 1.5),
                (0.5, -1.0),
                (2.5, 3.0),
                (2.5, -0.5),
            ] {
                assert_eq!(side(u, v), Ok(Side::Out), "({u}, {v})");
            }
            for (u, v) in [(1.0, 0.0), (1.5, 1.5), (2.5, 1.0), (2.0, 2.0), (0.5, 0.0)] {
                assert_eq!(side(u, v), Ok(Side::On), "({u}, {v})");
            }
            assert_eq!(turns(&surface, &loops[0]), Ok(0));
            // In (angle, height) times the radius: 2 x 1 and 1 x 1, by r.
            let area = signed_area(&surface, false, &loops[0]).unwrap();
            assert!((area - 6.0).abs() < 1e-12, "{area}");
            assert!(signed_area(&surface, true, &loops[0]).unwrap() < 0.0);
            let outline = Outline {
                surface: surface.clone(),
                reversed: false,
                loops: loops.clone(),
            };
            let inside = interior_point(&outline, 1e-6, 4e-6).unwrap();
            assert_eq!(side_of(&surface, &loops, inside, 1e-6), Ok(Side::In));

            // The whole wall between two circles: each rim goes around
            // once, the lower one with the wall above it.
            let ring = |v: f64| frame.at(frame.point(0.0, 0.0, v)).unwrap();
            let wall = vec![
                vec![arc(ring(0.0), r, (0.0, TAU), false)],
                vec![arc(ring(3.0), r, (0.0, TAU), true)],
            ];
            assert_eq!(turns(&surface, &wall[0]), Ok(1));
            assert_eq!(turns(&surface, &wall[1]), Ok(-1));
            for u in [0.0_f64, 1.0, 3.0, 6.0] {
                let p = |v: f64| frame.point(r * u.cos(), r * u.sin(), v);
                assert_eq!(side_of(&surface, &wall, p(1.0), 1e-6), Ok(Side::In));
                assert_eq!(side_of(&surface, &wall, p(-1.0), 1e-6), Ok(Side::Out));
                assert_eq!(side_of(&surface, &wall, p(4.0), 1e-6), Ok(Side::Out));
                assert_eq!(side_of(&surface, &wall, p(3.0), 1e-6), Ok(Side::On));
            }
            let top = |loop_: &[Seg]| top_crossing(&surface, loop_, 2.0).unwrap().unwrap();
            assert!((top(&wall[1]) - 3.0).abs() < 1e-12);
            assert!((top(&loops[0]) - 2.0).abs() < 1e-12);
            assert_eq!(top_crossing(&surface, &loops[0], 4.0), Ok(None));
        }
    }

    #[test]
    fn a_stretch_of_angles_has_its_start_and_not_its_end() {
        // Up to, not including: two stretches that share an end never
        // both have it. Also across the seam at a full turn.
        assert!(within(1.0, 1.0, 2.0) && within(1.5, 1.0, 2.0));
        assert!(!within(2.0, 1.0, 2.0) && !within(0.5, 1.0, 2.0) && !within(2.5, 1.0, 2.0));
        assert!(within(6.0, 5.0, 1.0) && within(0.5, 5.0, 1.0) && within(5.0, 5.0, 1.0));
        assert!(!within(1.0, 5.0, 1.0) && !within(3.0, 5.0, 1.0));
        assert!(!within(1.0, 1.0, 1.0));
    }

    #[test]
    fn the_nearest_point_of_a_long_ellipse_arc_is_the_nearest() {
        // 40 to 1: around its tips the curve doubles back on itself.
        let ellipse = Ellipse::new(tilted(), 20.0, 0.5).unwrap();
        let curve = Curve3::Ellipse(ellipse);
        let mut seed = 7_u64;
        let mut unit = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        for _ in 0..200 {
            let t0 = unit() * TAU;
            let range = (t0, t0 + 0.05 + unit() * 3.0);
            let p = tilted().point(
                (unit() - 0.5) * 44.0,
                (unit() - 0.5) * 2.0,
                (unit() - 0.5) * 0.5,
            );
            let (t, d) = nearest(&curve, range, p).unwrap();
            assert!(t >= range.0 && t <= range.1);
            assert!((curve.eval(t).distance(p) - d).abs() < 1e-12);
            let mut brute = f64::INFINITY;
            for k in 0..=20_000 {
                let s = range.0 + (range.1 - range.0) * f64::from(k) / 20_000.0;
                brute = brute.min(curve.eval(s).distance(p));
            }
            assert!(d <= brute + 1e-12, "{d} is not the nearest: {brute}");
            assert!(d >= brute - 1e-2, "{d} is closer than any sample: {brute}");
        }
        // And on a circle or a line the range is respected too.
        let circle = Curve3::Circle(Circle::new(Frame::WORLD, 1.0).unwrap());
        let (t, d) = nearest(&circle, (0.0, 1.0), Vec3::new(-2.0, 0.1, 0.0)).unwrap();
        assert_eq!(t, 1.0);
        assert!((d - circle.eval(1.0).distance(Vec3::new(-2.0, 0.1, 0.0))).abs() < 1e-15);
        assert_eq!(
            locate(&circle, (5.0, 7.0), Vec3::new(1.0, 0.0, 0.0), 1e-6),
            Some(TAU)
        );
        assert_eq!(
            locate(&circle, (0.5, 1.0), Vec3::new(1.0, 0.0, 0.0), 1e-6),
            None
        );
    }
}
