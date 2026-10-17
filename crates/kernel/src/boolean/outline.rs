//! A face, or a piece of one, as the booleans see it: a surface and the
//! loops of curve pieces that bound it, with the questions asked of it: is
//! this point of the surface inside, where is a point safely inside, what
//! box holds it.
//!
//! Inside or outside is a winding number in a chart of the surface
//! ([`super::chart`]), for every analytic surface kind and every curve
//! kind alike. A face without loops is its whole surface (a sphere, a
//! torus).

use super::Reason;
use super::chart::{About, Chart, TOO_CLOSE};
use super::freeform::{PatchTree, is_freeform};
use super::hits::speed_bound;
use craft3d_geom::{Curve3, Ellipse, Surface};
use craft3d_math::{Bbox3, Frame, Vec3};
use std::f64::consts::{FRAC_PI_2, FRAC_PI_8, PI, TAU};
use std::sync::Arc;

/// A piece of a curve in a loop.
#[derive(Clone, Debug)]
pub(super) struct Seg {
    pub curve: Curve3,
    pub range: (f64, f64),
    /// The vertex positions at `range.0` and `range.1`.
    pub a: Vec3,
    pub b: Vec3,
    /// The loop runs against the curve.
    pub reversed: bool,
}

impl Seg {
    /// The point a fraction `k` of the way along the piece's range.
    pub fn at(&self, k: f64) -> Vec3 {
        self.curve
            .eval(self.range.0 + (self.range.1 - self.range.0) * k)
    }

    /// The direction the loop runs in at the middle of the piece.
    pub fn travel(&self) -> (Vec3, Vec3) {
        self.travel_at(0.5)
    }

    /// The point a fraction `k` of the way along the piece's range, and
    /// the direction the loop runs in there.
    pub fn travel_at(&self, k: f64) -> (Vec3, Vec3) {
        let (point, tangent) = self
            .curve
            .derivatives(self.range.0 + (self.range.1 - self.range.0) * k);
        (point, if self.reversed { -tangent } else { tangent })
    }

    pub fn sign(&self) -> f64 {
        if self.reversed { -1.0 } else { 1.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Side {
    In,
    /// Within the tolerance of the boundary.
    On,
    Out,
}

const SLIVER: Reason =
    "a piece of a face too thin to find a point inside it (at the edge of tolerance)";
/// Where along a boundary piece [`Outline::interior_points`] steps into
/// the face, the part of the piece's width the first step starts from (it
/// is shrunk before use), and how many points are taken there. Every
/// piece is tried at one place before any at the next.
const STATIONS: [(f64, f64, usize); 3] = [(0.5, 1.0, 2), (0.2764, 0.618, 1), (0.7236, 0.382, 1)];
const UNBOUNDED: Reason = "a face without a boundary on a surface that is not closed";

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
    match curve {
        Curve3::Ellipse(ellipse) => return nearest_on_ellipse(ellipse, range, p),
        Curve3::Nurbs(_) => return nearest_on_spline(curve, range, p),
        Curve3::Line(_) | Curve3::Circle(_) => {}
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

/// [`nearest`] for a piece of a spline: branch and bound as for an ellipse,
/// with the curve's speed bound, then Newton's method.
fn nearest_on_spline(curve: &Curve3, range: (f64, f64), p: Vec3) -> Option<(f64, f64)> {
    let speed = speed_bound(curve);
    let (t0, t1) = range;
    if !(speed.is_finite() && t0.is_finite() && t1.is_finite() && p.is_finite()) {
        return None;
    }
    let distance = |t: f64| curve.eval(t).distance(p);
    let (d0, d1) = (distance(t0), distance(t1));
    let mut best = if d0 <= d1 { (d0, t0) } else { (d1, t1) };
    let shortest = 1e-9 * speed * (t1 - t0);
    let mut stack = vec![(t0, d0, t1, d1)];
    let mut steps = 0_usize;
    while let Some((ta, da, tb, db)) = stack.pop() {
        steps += 1;
        let length = speed * (tb - ta);
        if steps > 20_000
            || da.min(db) - 0.5 * length >= best.0
            || length <= (1e-3 * best.0).max(shortest)
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
    // Newton on (C - p) · C', its slope by a central difference of C'.
    let h = (t1 - t0) * 1e-6;
    let mut t = best.1;
    for _ in 0..40 {
        let (c, d1) = curve.derivatives(t);
        let g = (c - p).dot(d1);
        let d2 = (curve.derivatives(t + h).1 - curve.derivatives(t - h).1) * (0.5 / h);
        let slope = d1.length_squared() + (c - p).dot(d2);
        if !(slope.is_finite() && slope > 0.0) {
            break;
        }
        let next = (t - g / slope).max(t0).min(t1);
        let moved = (next - t).abs();
        t = next;
        if moved <= 1e-15 * (t1 - t0) {
            break;
        }
    }
    let d = distance(t);
    let (d, t) = if d <= best.0 { (d, t) } else { best };
    d.is_finite().then_some((t, d))
}

/// The parameter of `p` on the curve piece, when it is within `tol` of it.
pub(super) fn locate(curve: &Curve3, range: (f64, f64), p: Vec3, tol: f64) -> Option<f64> {
    nearest(curve, range, p)
        .filter(|&(_, d)| d <= tol)
        .map(|(t, _)| t)
}

/// A surface with, when it is freeform, its index: what the helpers below
/// and the hits take. An analytic surface needs none; a freeform surface
/// without one is refused where the index is needed (the hits, the chart)
/// and projected the slow way elsewhere.
#[derive(Clone, Copy)]
pub(super) struct Sheet<'a> {
    pub surface: &'a Surface,
    pub tree: Option<&'a Arc<PatchTree>>,
}

impl<'a> Sheet<'a> {
    pub fn bare(surface: &'a Surface) -> Self {
        Sheet {
            surface,
            tree: None,
        }
    }

    /// The parameters of the point of the surface nearest to `p`.
    fn project(self, p: Vec3) -> Option<(f64, f64)> {
        match self.tree {
            Some(tree) => tree.nearest(p).map(|n| (n.u, n.v)),
            None => self.surface.project(p).ok(),
        }
    }
}

/// The distance from `p` to the surface; infinite when it has none.
pub(super) fn surface_distance(sheet: Sheet, p: Vec3) -> f64 {
    if let Some(tree) = sheet.tree {
        return tree.nearest(p).map_or(f64::INFINITY, |n| n.distance);
    }
    match sheet.surface.project(p) {
        Ok((u, v)) => {
            let d = sheet.surface.eval(u, v).distance(p);
            if d.is_finite() { d } else { f64::INFINITY }
        }
        Err(_) => f64::INFINITY,
    }
}

/// The point of the surface nearest to `p`.
pub(super) fn onto(sheet: Sheet, p: Vec3) -> Option<Vec3> {
    let (u, v) = sheet.project(p)?;
    let q = sheet.surface.eval(u, v);
    q.is_finite().then_some(q)
}

/// The outward normal of a face on the surface at (the projection of) `p`.
///
/// At the apex of a cone, where the surface has no normal, the direction
/// along the axis on the outward side: curves that leave the apex are told
/// apart by their directions seen along the axis.
pub(super) fn outward_normal(sheet: Sheet, reversed: bool, p: Vec3) -> Option<Vec3> {
    let surface = sheet.surface;
    let (u, v) = sheet.project(p)?;
    // At a pole of a freeform surface (a border that is one point), where
    // its formula has no normal: the direction it closes around, which is
    // its normal there when it closes up smoothly and stands in for the
    // axis as at an apex when it comes to a point.
    if let Some(axis) = super::freeform::pole_axis(surface, sheet.tree.map(|t| &**t), u, v) {
        return axis.map(|axis| if reversed { -axis } else { axis });
    }
    let normal = match (surface.normal(u, v), surface) {
        (Some(normal), _) => normal,
        (None, Surface::Cone(cone)) => {
            // The normal is (cos u, sin u, -tan α) scaled: it leans
            // against the axis where the cone widens along it.
            if cone.half_angle() > 0.0 {
                -cone.frame().z()
            } else {
                cone.frame().z()
            }
        }
        (None, _) => return None,
    };
    Some(if reversed { -normal } else { normal })
}

/// The smallest distance from `p` to the loops; infinite without loops.
pub(super) fn clearance(loops: &[Vec<Seg>], p: Vec3) -> f64 {
    let mut least = f64::INFINITY;
    for seg in loops.iter().flatten() {
        let d = nearest(&seg.curve, seg.range, p).map_or(0.0, |(_, d)| d);
        least = least.min(d);
    }
    least
}

/// A box around every piece of some loops, arranged so that the pieces
/// near a point are found without looking at all of them: what keeps a
/// face with a thousand holes from costing a million measurements.
///
/// The boxes are sorted by where they begin along x. The few that are much
/// longer along x than the rest (the sides of a plate, against its holes)
/// are kept apart and always looked at, so that the others are found in a
/// stretch of the sorted row no longer than the search is wide.
#[derive(Clone, Debug, Default)]
pub(super) struct Boxes {
    /// Where the box begins along x, the box, and the piece: its loop and
    /// its place in the loop.
    row: Vec<(f64, Bbox3, usize, usize)>,
    /// The greatest length along x of a box in the row.
    width: f64,
    long: Vec<(Bbox3, usize, usize)>,
    /// Per loop, a box around all of it.
    around: Vec<Bbox3>,
}

/// A box around a piece of `curve` from `a` to `b`: the ends, a piece of
/// a circle or an ellipse counted as the whole of it, and a spline's
/// control points (which its curve stays within).
pub(super) fn piece_box(curve: &Curve3, a: Vec3, b: Vec3) -> Bbox3 {
    let mut bx = Bbox3::from_points([a, b]);
    // A piece of a circle or an ellipse counts as the whole of it.
    let conic = match &curve {
        Curve3::Line(_) => None,
        Curve3::Circle(c) => {
            let f = c.frame();
            Some((f.origin(), f.x() * c.radius(), f.y() * c.radius()))
        }
        Curve3::Ellipse(e) => {
            let (f, (major, minor)) = (e.frame(), e.semi_axes());
            Some((f.origin(), f.x() * major, f.y() * minor))
        }
        Curve3::Nurbs(c) => {
            for &p in c.control_points() {
                bx.add(p);
            }
            None
        }
    };
    if let Some((center, x, y)) = conic {
        let reach = Vec3::new(x.x.hypot(y.x), x.y.hypot(y.y), x.z.hypot(y.z));
        bx.add(center - reach);
        bx.add(center + reach);
    }
    bx
}

pub(super) fn boxes(loops: &[Vec<Seg>]) -> Boxes {
    let mut all = Vec::new();
    let mut around = vec![Bbox3::EMPTY; loops.len()];
    for (l, loop_) in loops.iter().enumerate() {
        for (k, seg) in loop_.iter().enumerate() {
            let b = piece_box(&seg.curve, seg.a, seg.b);
            if let Some((lo, hi)) = b.corners() {
                if let Some(whole) = around.get_mut(l) {
                    whole.add(lo);
                    whole.add(hi);
                }
                all.push((lo.x, hi.x - lo.x, b, l, k));
            }
        }
    }
    // Long: more than four times the middle length.
    let mut lengths: Vec<f64> = all.iter().map(|entry| entry.1).collect();
    lengths.sort_by(f64::total_cmp);
    let limit = 4.0 * lengths.get(lengths.len() / 2).copied().unwrap_or(0.0);
    let mut found = Boxes {
        around,
        ..Boxes::default()
    };
    for (begin, length, b, l, k) in all {
        if length > limit || !begin.is_finite() || !length.is_finite() {
            found.long.push((b, l, k));
        } else {
            found.width = found.width.max(length);
            found.row.push((begin, b, l, k));
        }
    }
    found
        .row
        .sort_by(|p, q| p.0.total_cmp(&q.0).then(p.2.cmp(&q.2)).then(p.3.cmp(&q.3)));
    found
}

impl Boxes {
    /// True when `p` is within `margin` of the box around loop `l`, or
    /// that box is not known: outside it, a loop in a plane does not go
    /// around the point.
    pub fn around(&self, l: usize, p: Vec3, margin: f64) -> bool {
        self.around.get(l).is_none_or(|b| b.contains(p, margin))
    }

    /// The pieces whose box is within `r` of `p`, each as its loop and
    /// its place in the loop: every piece that is itself within `r` of
    /// `p` is among them.
    pub fn near(&self, p: Vec3, r: f64) -> Vec<(usize, usize)> {
        let room = r * (1.0 + 1e-9) + 1e-300;
        let (from, to) = if p.is_finite() && room.is_finite() {
            (
                self.row
                    .partition_point(|entry| entry.0 < p.x - room - self.width),
                self.row.partition_point(|entry| entry.0 <= p.x + room),
            )
        } else {
            (0, self.row.len())
        };
        let row = self
            .row
            .get(from..to)
            .into_iter()
            .flatten()
            .map(|(_, b, l, k)| (b, *l, *k));
        row.chain(self.long.iter().map(|(b, l, k)| (b, *l, *k)))
            .filter(|(b, ..)| {
                b.corners().is_some_and(|(lo, hi)| {
                    // (Not finite: not ruled out.)
                    {
                        let away = (lo - p).max(p - hi).max(Vec3::ZERO).length();
                        away <= room || away.is_nan()
                    }
                })
            })
            .map(|(_, l, k)| (l, k))
            .collect()
    }

    /// True when a piece of the loops is nearer to `p` than `r` (or as
    /// near, when `or_as_near`): what `clearance(loops, p) < r` says,
    /// measuring only the pieces whose box is that near.
    pub fn nearer(&self, loops: &[Vec<Seg>], p: Vec3, r: f64, or_as_near: bool) -> bool {
        if !(p.is_finite() && r.is_finite()) {
            let least = clearance(loops, p);
            return least < r || (or_as_near && least == r);
        }
        // With room for the rounding of a box's own distance.
        let room = r * (1.0 + 1e-9) + 1e-300;
        let from = self
            .row
            .partition_point(|entry| entry.0 < p.x - room - self.width);
        let to = self.row.partition_point(|entry| entry.0 <= p.x + room);
        let near = self
            .row
            .get(from..to)
            .into_iter()
            .flatten()
            .map(|(_, b, l, k)| (b, l, k));
        for (b, &l, &k) in near.chain(self.long.iter().map(|(b, l, k)| (b, l, k))) {
            let Some((lo, hi)) = b.corners() else {
                continue;
            };
            if (lo - p).max(p - hi).max(Vec3::ZERO).length() > room {
                continue;
            }
            let Some(seg) = loops.get(l).and_then(|loop_| loop_.get(k)) else {
                continue;
            };
            let d = nearest(&seg.curve, seg.range, p).map_or(0.0, |(_, d)| d);
            if d < r || (or_as_near && d == r) {
                return true;
            }
        }
        false
    }
}

/// How the searches for a point inside a face step in from its boundary:
/// by what each step shrinks, how many steps at most, and how far from the
/// boundary the point must be. Steps that halve find a face of ordinary
/// width at once. A strip ten tolerances wide has room for a point only
/// between four and six tolerances in, a window halving steps hit by
/// chance, so where they find nothing the steps shrink by a twelfth of an
/// octave. And the end of a sliver ten tolerances thick is a triangle
/// twenty wide and ten high, in which no point is further than 4.14
/// tolerances from every side: the last pass settles for three quarters
/// of the margin.
fn passes(margin: f64) -> [(f64, usize, f64); 3] {
    let fine = 0.5_f64.powf(1.0 / 12.0);
    [
        (0.5, 64, margin),
        (fine, 768, margin),
        (fine, 768, 0.75 * margin),
    ]
}

/// A point of the surface just to the left of one of the pieces `of`, seen
/// from outside: on the side of it that the face it bounds is on, and with
/// nothing of `all` between it and that piece. At least `margin` away from
/// everything.
pub(super) fn left_of(
    sheet: Sheet,
    reversed: bool,
    all: &[Vec<Seg>],
    all_boxes: &Boxes,
    of: &[Seg],
    margin: f64,
) -> Result<Vec3, Reason> {
    // Steps that halve find a face of ordinary width at once. In a strip
    // ten tolerances wide the candidate must be more than the margin (four
    // tolerances) from its piece and less than half the strip (five), a
    // window halving steps hit only by chance: where they find nothing the
    // steps shrink by a twelfth of an octave instead.
    for (shrink, most, margin) in passes(margin) {
        for seg in of {
            let (point, travel) = seg.travel();
            let Some(outward) = outward_normal(sheet, reversed, point) else {
                continue;
            };
            // Seen from outside the face lies to the left of its boundary.
            let Some(inward) = outward.cross(travel).normalized() else {
                continue;
            };
            let mut step = seg.at(0.25).distance(seg.at(0.75));
            for _ in 0..most {
                step *= shrink;
                if !(step.is_finite() && step > margin) {
                    break;
                }
                let Some(candidate) = onto(sheet, point + inward * step) else {
                    continue;
                };
                // The nearest thing to it must be this piece, away from its
                // ends: then nothing lies between the two, and the candidate
                // is right next to the piece there. On which side, the
                // direction from that nearest point says.
                let Some((t, reach)) = nearest(&seg.curve, seg.range, candidate) else {
                    continue;
                };
                let edge = 1e-6 * (seg.range.1 - seg.range.0);
                if !(reach > margin && t > seg.range.0 + edge && t < seg.range.1 - edge)
                    || all_boxes.nearer(all, candidate, reach * (1.0 - 1e-9), false)
                {
                    continue;
                }
                let (foot, tangent) = seg.curve.derivatives(t);
                let travel = if seg.reversed { -tangent } else { tangent };
                let left = outward_normal(sheet, reversed, foot).map(|n| n.cross(travel));
                if left.is_some_and(|left| left.dot(candidate - foot) > 0.0) {
                    return Ok(candidate);
                }
            }
        }
    }
    Err(SLIVER)
}

#[derive(Clone, Debug)]
pub(super) struct Outline {
    pub surface: Surface,
    /// The outward normal is opposite to the surface's.
    pub reversed: bool,
    pub loops: Vec<Vec<Seg>>,
    /// The [`boxes`] of the loops.
    boxes: Boxes,
    chart: Chart,
    /// The face contains the place its chart leaves out (the centre of
    /// projection of a sphere, the cut of a torus). Its loops then wind
    /// about the points outside it, and not about those inside.
    infinite: bool,
    /// For a freeform surface: its index, shared with the pieces of the
    /// face.
    tree: Option<Arc<PatchTree>>,
}

impl Outline {
    /// `margin` is the least distance from the boundary at which a point is
    /// looked for to tell which side of its loops a face on a sphere or a
    /// torus is on.
    pub fn new(
        surface: Surface,
        reversed: bool,
        loops: Vec<Vec<Seg>>,
        margin: f64,
    ) -> Result<Outline, Reason> {
        let tree = if is_freeform(&surface) {
            Some(PatchTree::shared(&surface)?)
        } else {
            None
        };
        Self::build(surface, tree, reversed, loops, margin)
    }

    /// A piece of this face: the same surface, with its index shared, and
    /// other loops.
    pub fn piece(&self, loops: Vec<Vec<Seg>>, margin: f64) -> Result<Outline, Reason> {
        Self::build(
            self.surface.clone(),
            self.tree.clone(),
            self.reversed,
            loops,
            margin,
        )
    }

    fn build(
        surface: Surface,
        tree: Option<Arc<PatchTree>>,
        reversed: bool,
        loops: Vec<Vec<Seg>>,
        margin: f64,
    ) -> Result<Outline, Reason> {
        let sheet = Sheet {
            surface: &surface,
            tree: tree.as_ref(),
        };
        let chart = Chart::new(sheet, &loops)?;
        if loops.is_empty() && !chart.is_closed() {
            return Err(UNBOUNDED);
        }
        let boxes = boxes(&loops);
        let infinite = if chart.is_closed() && !loops.is_empty() {
            // The loops go around a point just inside the face once, or
            // not at all when the face is the outside of them.
            let mut found = None;
            for loop_ in &loops {
                if let Ok(witness) = left_of(sheet, reversed, &loops, &boxes, loop_, margin) {
                    found = Some(witness);
                    break;
                }
            }
            let witness = found.ok_or(SLIVER)?;
            match chart.winding(loops.iter().map(Vec::as_slice), About::Point(witness))? {
                0 => true,
                1 | -1 => false,
                _ => return Err(TOO_CLOSE),
            }
        } else {
            false
        };
        Ok(Outline {
            surface,
            reversed,
            loops,
            boxes,
            chart,
            infinite,
            tree,
        })
    }

    /// The surface with its index.
    pub fn sheet(&self) -> Sheet<'_> {
        Sheet {
            surface: &self.surface,
            tree: self.tree.as_ref(),
        }
    }

    /// The pieces of the boundary that may be within `r` of `p`: see
    /// [`Boxes::near`].
    pub fn pieces_near(&self, p: Vec3, r: f64) -> Vec<(usize, usize)> {
        self.boxes.near(p, r)
    }

    /// Where a point of the surface is with respect to the loops.
    pub fn side(&self, p: Vec3, tol: f64) -> Result<Side, Reason> {
        if self.boxes.nearer(&self.loops, p, tol, true) {
            return Ok(Side::On);
        }
        if self.loops.is_empty() {
            return Ok(Side::In);
        }
        // In a plane a loop does not go around a point outside the box
        // around it: on a plate with a thousand holes that leaves the
        // outside and at most a hole or two to follow.
        let flat = matches!(self.surface, Surface::Plane(_));
        let winding = self.chart.winding(
            self.loops
                .iter()
                .enumerate()
                .filter(|(l, _)| !flat || self.boxes.around(*l, p, tol))
                .map(|(_, loop_)| loop_.as_slice()),
            About::Point(p),
        )?;
        Ok(if (winding == 0) == self.infinite {
            Side::In
        } else {
            Side::Out
        })
    }

    /// Up to `want` points of the face well inside the loops: at least
    /// `margin` from all of them. Tried from the middle of each boundary
    /// piece, then from two other places of each, stepping into the face by ever smaller amounts; on a face
    /// without loops, fixed places of the surface.
    pub fn interior_points(&self, tol: f64, margin: f64, want: usize) -> Result<Vec<Vec3>, Reason> {
        let mut found = Vec::new();
        if self.loops.is_empty() {
            // A freeform surface is evaluated at those shares of its
            // domain (it is no surface beyond it), the others at those
            // parameters.
            let ((u0, u1), (v0, v1)) = self.surface.domain();
            for (u, v) in [(0.7, 0.3), (2.9, -0.8), (4.6, 1.0), (5.5, -0.2)] {
                let p = if self.tree.is_some() {
                    let (s, t) = (u / TAU, (v + PI) / TAU);
                    self.surface.eval(u0 + s * (u1 - u0), v0 + t * (v1 - v0))
                } else {
                    self.surface.eval(u, v)
                };
                if p.is_finite() && found.len() < want {
                    found.push(p);
                }
            }
            return if found.is_empty() {
                Err(SLIVER)
            } else {
                Ok(found)
            };
        }
        let mut failure = None;
        // Steps that halve find a face of ordinary width at once; a strip
        // only a little wider than twice the margin (a sliver ten
        // tolerances thick, with room for a point between four and six
        // tolerances in) is hit by them only by chance, so where they find
        // nothing the steps shrink by a twelfth of an octave instead.
        for (shrink, most, margin) in passes(margin) {
            if !found.is_empty() {
                break;
            }
            self.points_inwards(tol, margin, want, shrink, most, &mut found, &mut failure);
        }
        if found.is_empty() {
            Err(failure.unwrap_or(SLIVER))
        } else {
            Ok(found)
        }
    }

    /// The search of [`Outline::interior_points`] with steps into the face
    /// that shrink by `shrink` each time, at most `most` of them per
    /// boundary piece.
    #[allow(clippy::too_many_arguments)]
    fn points_inwards(
        &self,
        tol: f64,
        margin: f64,
        want: usize,
        shrink: f64,
        most: usize,
        found: &mut Vec<Vec3>,
        failure: &mut Option<Reason>,
    ) {
        // First from the middle of each piece, two at different depths:
        // where the first is of no use (it is just where the other body
        // touches), the next is somewhere else. Then, while more are
        // wanted, from other places along each piece and from other
        // depths: an edge of the other body through the middle of a disc
        // lies on every step in from the middle of its one piece, and
        // half the width in from anywhere on its rim is that middle.
        for (station, depth, take) in STATIONS {
            for seg in self.loops.iter().flatten() {
                if found.len() >= want {
                    return;
                }
                let (point, travel) = seg.travel_at(station);
                let Some(outward) = outward_normal(self.sheet(), self.reversed, point) else {
                    continue;
                };
                // Seen from outside the face lies to the left of its
                // boundary.
                let Some(inward) = outward.cross(travel).normalized() else {
                    continue;
                };
                let mut step = depth * seg.at(0.25).distance(seg.at(0.75));
                let mut taken = 0;
                for _ in 0..most {
                    step *= shrink;
                    if !(step.is_finite() && step > margin) {
                        break;
                    }
                    let Some(candidate) = onto(self.sheet(), point + inward * step) else {
                        continue;
                    };
                    if self.boxes.nearer(&self.loops, candidate, margin, false) {
                        continue;
                    }
                    match self.side(candidate, tol) {
                        Ok(Side::In) => {
                            found.push(candidate);
                            taken += 1;
                            if taken == take || found.len() >= want {
                                break;
                            }
                        }
                        Ok(_) => {}
                        Err(reason) => *failure = failure.or(Some(reason)),
                    }
                }
            }
        }
    }

    /// Points, circles and balls that the face lies within the convex
    /// hull of ([`hull_of`]).
    #[cfg(test)]
    pub fn hull(&self, tol: f64) -> Hull {
        let reaches =
            |tip: Vec3| -> Result<bool, ()> { Ok(!matches!(self.side(tip, tol), Ok(Side::Out))) };
        hull_of(&self.surface, &self.loops, reaches).unwrap_or_default()
    }
}

/// Points, circles and balls that a face on `surface` bounded by `loops`
/// lies within the convex hull of. Conservative: an arc counts as its
/// whole circle or ellipse, a face on a cylinder or a cone as the whole
/// stretch of the surface between the ends of its boundary along the axis
/// (to the apex when the face reaches it), a face on a torus as the tube
/// around the stretch of its central circle that the face turns through
/// about the axis (or, when that is the whole turn or cannot be told,
/// the ball around the whole torus), a face on a sphere as the ball.
/// `reaches(p)` says whether the face holds the point `p` of its surface
/// (or comes to it): asked of a cone at its apex, and of a torus once per
/// stretch of turn that its boundary leaves out.
pub(super) fn hull_of<E>(
    surface: &Surface,
    loops: &[Vec<Seg>],
    mut reaches: impl FnMut(Vec3) -> Result<bool, E>,
) -> Result<Hull, E> {
    let mut hull = Hull::default();
    // The points each piece lies within the hull of (an arc as the polygon
    // of its pieces of at most a quarter turn and the corners where their
    // end tangents meet), for the turn of a face on a torus.
    let mut pieces: Vec<Vec<Vec3>> = Vec::new();
    for seg in loops.iter().flatten() {
        hull.points.push(seg.a);
        hull.points.push(seg.b);
        let mut points = vec![seg.a, seg.b];
        let conic = match &seg.curve {
            Curve3::Line(_) => None,
            Curve3::Circle(c) => {
                let f = c.frame();
                Some((f.origin(), f.x() * c.radius(), f.y() * c.radius()))
            }
            Curve3::Ellipse(e) => {
                let f = e.frame();
                let (a, b) = e.semi_axes();
                Some((f.origin(), f.x() * a, f.y() * b))
            }
            Curve3::Nurbs(c) => {
                hull.points.extend_from_slice(c.control_points());
                points.extend_from_slice(c.control_points());
                None
            }
        };
        if let Some((center, x, y)) = conic {
            hull.conics.push((center, x, y));
            if !arc_polygon(center, x, y, seg.range, FRAC_PI_2, &mut points) {
                points.extend(square(center, x, y));
            }
        }
        pieces.push(points);
    }
    let (frame, radius, slope, apex) = match surface {
        Surface::Plane(_) => return Ok(hull),
        // A freeform surface lies in the hull of its control points.
        Surface::Bezier(b) => {
            hull.points.extend_from_slice(b.control_points());
            return Ok(hull);
        }
        Surface::Nurbs(s) => {
            hull.points.extend_from_slice(s.control_points());
            return Ok(hull);
        }
        Surface::Sphere(s) => {
            hull.balls.push((s.frame().origin(), s.radius()));
            return Ok(hull);
        }
        Surface::Torus(s) => {
            let frame = *s.frame();
            let (major, minor) = s.radii();
            let whole = (frame.origin(), major + minor);
            // (A face whose outline cannot be made, or whose turn cannot
            // be told, keeps the ball: it is as true as before, only
            // larger.)
            let turns = torus_turns(&frame, major, minor, &pieces, &mut reaches)
                .ok()
                .flatten();
            let Some(turns) = turns else {
                hull.balls.push(whole);
                return Ok(hull);
            };
            // The tube around each stretch of the central circle: within
            // the balls of the tube's radius about the polygon the stretch
            // lies within (pieces of at most an eighth of a turn, whose
            // corners stand out by under 2 % of the radius), and within the
            // ball around the whole torus as well, which bounds every span
            // where the polygon reaches further. The face, its boundary
            // included, lies in both.
            let mut tube = Hull {
                bound: Some(whole),
                ..Hull::default()
            };
            for (from, to) in turns {
                let mut spine = Vec::new();
                let (x, y) = (frame.x() * major, frame.y() * major);
                if !arc_polygon(frame.origin(), x, y, (from, to), FRAC_PI_8, &mut spine) {
                    hull.balls.push(whole);
                    return Ok(hull);
                }
                tube.balls.extend(spine.into_iter().map(|p| (p, minor)));
            }
            return Ok(tube);
        }
        Surface::Cylinder(s) => (*s.frame(), s.radius(), 0.0, None),
        Surface::Cone(s) => (
            *s.frame(),
            s.radius(),
            s.half_angle().tan(),
            Some(s.apex_v()),
        ),
    };
    // The stretch of the axis that the boundary spans.
    let (mut low, mut high) = hull.span(frame.z());
    let base = frame.origin().dot(frame.z());
    (low, high) = (low - base, high - base);
    if let Some(apex) = apex {
        // A face that may contain the apex reaches it.
        let tip = frame.point(0.0, 0.0, apex);
        if reaches(tip)? {
            low = low.min(apex);
            high = high.max(apex);
        }
        // Nothing of a cone lies beyond its apex.
        if slope > 0.0 {
            low = low.max(apex);
        } else {
            high = high.min(apex);
        }
    }
    for along in [low, high] {
        if along.is_finite() {
            // The circle of the surface at that height.
            let reach = (radius + along * slope).max(0.0);
            hull.conics.push((
                frame.point(0.0, 0.0, along),
                frame.x() * reach,
                frame.y() * reach,
            ));
        }
    }
    Ok(hull)
}

/// Adds to `points` the polygon that the arc `center + x cos t + y sin t`
/// for `t` over `range` lies within: its ends and, cut into pieces of at
/// most `piece` (at most a quarter turn), the ends of the pieces and the corners where the
/// tangents at the ends of each piece meet (`1 / cos(half the piece)` out
/// along its middle direction). `false`, and nothing added, for an arc of
/// a whole turn or more or a range that is not finite.
fn arc_polygon(
    center: Vec3,
    x: Vec3,
    y: Vec3,
    range: (f64, f64),
    piece: f64,
    points: &mut Vec<Vec3>,
) -> bool {
    let (from, to) = (range.0.min(range.1), range.0.max(range.1));
    let sweep = to - from;
    if !(sweep.is_finite() && sweep < TAU) {
        return false;
    }
    let at = |t: f64, out: f64| {
        let (s, c) = t.sin_cos();
        center + (x * c + y * s) * out
    };
    // At most `piece` (itself at most a quarter turn) each, and at least
    // one piece.
    let piece = piece.clamp(1e-3, FRAC_PI_2);
    let count = (sweep / piece).ceil().clamp(1.0, (TAU / piece).ceil());
    let step = sweep / count;
    let out = 1.0 / (0.5 * step).cos();
    let mut t = from;
    points.push(at(from, 1.0));
    for _ in 0..(count as u32) {
        points.push(at(t + 0.5 * step, out));
        t += step;
        points.push(at(t, 1.0));
    }
    true
}

/// The corners of the parallelogram around the whole conic
/// `center + x cos t + y sin t`.
fn square(center: Vec3, x: Vec3, y: Vec3) -> Vec<Vec3> {
    vec![
        center + x + y,
        center + x - y,
        center - x + y,
        center - x - y,
    ]
}

/// The stretches of turn about the axis of a torus (as angles of its
/// frame, each from the first to the second) that a face on it bounded
/// by `pieces` lies within, or `None` when that is the whole turn or
/// cannot be told. Each piece of the boundary lies within the hull of its
/// points, and when those points stay within less than half a turn of
/// each other about the axis, so does the piece: the turns the boundary
/// takes are the union of those stretches. Between them the face has no
/// boundary, so a whole circle of the tube at a turn there is all in the
/// face or all out of it, and one point of it says which (`reaches`).
fn torus_turns<E>(
    frame: &Frame,
    major: f64,
    minor: f64,
    pieces: &[Vec<Vec3>],
    reaches: &mut impl FnMut(Vec3) -> Result<bool, E>,
) -> Result<Option<Vec<(f64, f64)>>, E> {
    // Each piece's stretch of turn, as (start, length) with the length
    // below half a turn.
    let mut stretches: Vec<(f64, f64)> = Vec::new();
    for points in pieces {
        let angles: Option<Vec<f64>> = points
            .iter()
            .map(|&p| {
                let (x, y, _) = frame.local(p);
                // (A point on the axis is of every turn.)
                (x.hypot(y) > 0.0).then(|| y.atan2(x))
            })
            .collect();
        let Some(mut angles) = angles.filter(|a| !a.is_empty()) else {
            return Ok(None);
        };
        angles.sort_by(f64::total_cmp);
        // The largest gap between neighbours (round the circle) is what
        // the piece leaves out; it must be more than half a turn.
        let mut widest = (0.0, 0_usize);
        for (k, pair) in angles.windows(2).enumerate() {
            if let [a, b] = pair
                && b - a > widest.0
            {
                widest = (b - a, k + 1);
            }
        }
        let (Some(&first), Some(&last)) = (angles.first(), angles.last()) else {
            return Ok(None);
        };
        let around = first + TAU - last;
        let (start, length) = if around >= widest.0 {
            (first, last - first)
        } else {
            let Some(&start) = angles.get(widest.1) else {
                return Ok(None);
            };
            (start, TAU - widest.0)
        };
        if length.is_nan() || length >= PI {
            return Ok(None);
        }
        stretches.push((start.rem_euclid(TAU), length));
    }
    if stretches.is_empty() {
        return Ok(None);
    }
    let boundary = union_round(&stretches);
    if boundary.is_empty() {
        return Ok(None);
    }
    // The gaps between them, from the end of one to the start of the
    // next (the last round to the first): left out of the face unless it
    // holds the tube's circle in the middle of the gap.
    let mut covered = stretches;
    for (k, &(_, end)) in boundary.iter().enumerate() {
        let Some(&(next, _)) = boundary.get((k + 1) % boundary.len()) else {
            return Ok(None);
        };
        let next = if next <= end { next + TAU } else { next };
        let (s, c) = (0.5 * (end + next)).sin_cos();
        if reaches(frame.point((major + minor) * c, (major + minor) * s, 0.0))? {
            covered.push((end.rem_euclid(TAU), next - end));
        }
    }
    let turns = union_round(&covered);
    if turns.is_empty() {
        return Ok(None);
    }
    Ok(Some(turns))
}

/// The union of stretches of turn `(start, length)` (start in `[0, 2π)`,
/// length below a whole turn), as `(from, to)` with `from` in `[0, 2π)`
/// and `to` past it, in order round the circle, one that runs past a
/// whole turn joined to the first; empty when they cover the whole turn.
fn union_round(stretches: &[(f64, f64)]) -> Vec<(f64, f64)> {
    // Stretches that meet to rounding are one: the ends of a gap and of
    // the stretches beside it are the same angles, taken round by a turn.
    let rounding = 8.0 * f64::EPSILON * TAU;
    // On [0, 2π], those that run past it cut in two.
    let mut flat: Vec<(f64, f64)> = Vec::new();
    for &(start, length) in stretches {
        let end = start + length;
        if end > TAU {
            flat.push((start, TAU));
            flat.push((0.0, end - TAU));
        } else {
            flat.push((start, end));
        }
    }
    flat.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for (a, b) in flat {
        match merged.last_mut() {
            Some(last) if a <= last.1 + rounding => last.1 = last.1.max(b),
            _ => merged.push((a, b)),
        }
    }
    if let [(a, b)] = merged.as_slice()
        && *a <= rounding
        && *b >= TAU - rounding
    {
        return Vec::new();
    }
    // One that ends at a whole turn goes on in one that starts at none.
    if merged.len() > 1
        && merged.first().is_some_and(|first| first.0 <= rounding)
        && merged.last().is_some_and(|last| last.1 >= TAU - rounding)
        && let (Some(first), Some(last)) = (merged.first().copied(), merged.pop())
        && let Some(head) = merged.first_mut()
    {
        *head = (last.0, first.1 + TAU);
    }
    merged
}

/// What a face lies within the convex hull of.
#[derive(Clone, Debug, Default)]
pub(super) struct Hull {
    points: Vec<Vec3>,
    /// `center + x cos t + y sin t`, whole.
    conics: Vec<(Vec3, Vec3, Vec3)>,
    /// Centre and radius.
    balls: Vec<(Vec3, f64)>,
    /// A ball the face lies within as well: every span is cut to it.
    bound: Option<(Vec3, f64)>,
}

impl Hull {
    /// The least and greatest of `p · direction` over the hull; an empty
    /// stretch (least above greatest) for an empty hull.
    pub fn span(&self, direction: Vec3) -> (f64, f64) {
        let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
        let mut add = |middle: f64, reach: f64| {
            low = low.min(middle - reach);
            high = high.max(middle + reach);
        };
        for &p in &self.points {
            add(p.dot(direction), 0.0);
        }
        for &(center, x, y) in &self.conics {
            add(
                center.dot(direction),
                x.dot(direction).hypot(y.dot(direction)),
            );
        }
        for &(center, radius) in &self.balls {
            add(center.dot(direction), radius * direction.length());
        }
        if let Some((center, radius)) = self.bound {
            let (middle, reach) = (center.dot(direction), radius * direction.length());
            low = low.max(middle - reach);
            high = high.min(middle + reach);
        }
        (low, high)
    }

    pub fn bbox(&self) -> Bbox3 {
        let mut b = Bbox3::EMPTY;
        let (x, y, z) = (self.span(Vec3::X), self.span(Vec3::Y), self.span(Vec3::Z));
        if x.0 <= x.1 {
            b.add(Vec3::new(x.0, y.0, z.0));
            b.add(Vec3::new(x.1, y.1, z.1));
        }
        b
    }
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

/// The stretch of the infinite line through `origin` along `direction`
/// that is within `margin` of the box, as parameters along it.
pub(super) fn line_in_box(
    origin: Vec3,
    direction: Vec3,
    b: &Bbox3,
    margin: f64,
) -> Option<(f64, f64)> {
    let (lo, hi) = b.corners()?;
    let (mut enter, mut leave) = (f64::NEG_INFINITY, f64::INFINITY);
    for (o, d, l, h) in [
        (origin.x, direction.x, lo.x, hi.x),
        (origin.y, direction.y, lo.y, hi.y),
        (origin.z, direction.z, lo.z, hi.z),
    ] {
        let (l, h) = (l - margin, h + margin);
        if d == 0.0 {
            if o < l || o > h {
                return None;
            }
        } else {
            let (t0, t1) = ((l - o) / d, (h - o) / d);
            enter = enter.max(t0.min(t1));
            leave = leave.min(t0.max(t1));
        }
    }
    (enter <= leave).then_some((enter, leave))
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_geom::{Circle, Cone, Cylinder, Line, NurbsCurve, Plane, Sphere, Torus};
    use craft3d_math::Frame;
    use std::f64::consts::{PI, TAU};

    fn line(a: Vec3, b: Vec3, reversed: bool) -> Seg {
        Seg {
            curve: Curve3::Line(Line::through(a, b).unwrap()),
            range: (0.0, a.distance(b)),
            a,
            b,
            reversed,
        }
    }

    fn arc(frame: Frame, radius: f64, range: (f64, f64), reversed: bool) -> Seg {
        let curve = Curve3::Circle(Circle::new(frame, radius).unwrap());
        Seg {
            a: curve.eval(range.0),
            b: curve.eval(range.1),
            curve,
            range,
            reversed,
        }
    }

    fn outline(surface: Surface, reversed: bool, loops: Vec<Vec<Seg>>) -> Outline {
        Outline::new(surface, reversed, loops, 4e-6).unwrap()
    }

    /// A 4 by 2 rectangle in the plane of `frame` with a half-disc bulging
    /// out of its right side and a round hole.
    fn plate(frame: Frame) -> Outline {
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
        outline(Surface::Plane(Plane::new(frame)), false, vec![outer, inner])
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
            let face = plate(frame);
            let side = |x: f64, y: f64| face.side(frame.point(x, y, 0.0), 1e-6);
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
            let inside = face.interior_points(1e-6, 4e-6, 3).unwrap();
            assert_eq!(inside.len(), 3);
            for p in inside {
                assert_eq!(face.side(p, 1e-6), Ok(Side::In));
                assert!(clearance(&face.loops, p) >= 4e-6);
            }
            // Just left of the hole's rim is in the plate, not in the hole.
            let beside = left_of(
                face.sheet(),
                false,
                &face.loops,
                &boxes(&face.loops),
                &face.loops[1],
                4e-6,
            )
            .unwrap();
            assert_eq!(face.side(beside, 1e-6), Ok(Side::In));
            let (x, y, _) = frame.local(beside);
            assert!((x - 1.0).hypot(y - 1.0) > 0.5);
        }
    }

    /// A patch of a cylinder between two rulings and two circles, with a
    /// step in its upper rim: the rulings of the step are in line with
    /// points further up and down the cylinder.
    fn patch(frame: Frame) -> Outline {
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
        outline(
            Surface::Cylinder(Cylinder::new(frame, r).unwrap()),
            false,
            vec![outer],
        )
    }

    #[test]
    fn inside_on_and_outside_of_a_cylindrical_outline() {
        for frame in [Frame::WORLD, tilted()] {
            let face = patch(frame);
            let r = 2.0;
            let at = |u: f64, v: f64| frame.point(r * u.cos(), r * u.sin(), v);
            let side = |u: f64, v: f64| face.side(at(u, v), 1e-6);
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
            // cylinder; in line with each of the three rulings; and far
            // along the axis, where an exponential would overflow.
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
                (1.0, 1e5),
                (1.0, -1e5),
            ] {
                assert_eq!(side(u, v), Ok(Side::Out), "({u}, {v})");
            }
            for (u, v) in [(1.0, 0.0), (1.5, 1.5), (2.5, 1.0), (2.0, 2.0), (0.5, 0.0)] {
                assert_eq!(side(u, v), Ok(Side::On), "({u}, {v})");
            }
            let inside = face.interior_points(1e-6, 4e-6, 1).unwrap();
            assert_eq!(face.side(inside[0], 1e-6), Ok(Side::In));

            // The whole wall between two circles: each rim goes around
            // once, the lower one with the wall above it.
            let ring = |v: f64| frame.at(frame.point(0.0, 0.0, v)).unwrap();
            for reversed in [false, true] {
                // Seen from inside (a hole's wall) the rims run the other
                // way round.
                let wall = outline(
                    Surface::Cylinder(Cylinder::new(frame, r).unwrap()),
                    reversed,
                    vec![
                        vec![arc(ring(0.0), r, (0.0, TAU), reversed)],
                        vec![arc(ring(3.0), r, (0.0, TAU), !reversed)],
                    ],
                );
                for u in [0.0_f64, 1.0, 3.0, 6.0] {
                    assert_eq!(wall.side(at(u, 1.0), 1e-6), Ok(Side::In));
                    assert_eq!(wall.side(at(u, 2.999), 1e-6), Ok(Side::In));
                    assert_eq!(wall.side(at(u, -1.0), 1e-6), Ok(Side::Out));
                    assert_eq!(wall.side(at(u, 4.0), 1e-6), Ok(Side::Out));
                    assert_eq!(wall.side(at(u, 3000.0), 1e-6), Ok(Side::Out));
                    assert_eq!(wall.side(at(u, 3.0), 1e-6), Ok(Side::On));
                }
            }
        }
    }

    #[test]
    fn inside_and_outside_on_a_cone_through_its_apex() {
        for (frame, k) in [(Frame::WORLD, 1.0), (tilted(), 1e-3), (tilted(), 1e6)] {
            let frame = frame.at(frame.origin() * k).unwrap();
            // Radius 2 at the base, apex at height 4.
            let slope: f64 = -0.5;
            let cone = Surface::Cone(Cone::new(frame, 2.0 * k, slope.atan()).unwrap());
            let at = |u: f64, v: f64| {
                let r = (2.0 + v * slope) * k;
                frame.point(r * u.cos(), r * u.sin(), v * k)
            };
            let tol = 1e-6 * k;
            // The whole side: one rim, the apex inside.
            let side = outline(
                cone.clone(),
                false,
                vec![vec![arc(frame, 2.0 * k, (0.0, TAU), false)]],
            );
            for (u, v) in [(0.3, 0.5), (4.0, 3.9), (2.0, 4.0), (5.9, 1e-4)] {
                assert_eq!(side.side(at(u, v), tol), Ok(Side::In), "({u}, {v})");
            }
            assert_eq!(side.side(at(1.0, 0.0), tol), Ok(Side::On));
            assert_eq!(side.side(at(1.0, -0.5), tol), Ok(Side::Out));
            assert!(side.hull(tol).bbox().contains(at(0.0, 4.0), 0.0));
            // A frustum between heights 1 and 2: the apex is outside.
            let ring = |v: f64| frame.at(frame.point(0.0, 0.0, v * k)).unwrap();
            let frustum = outline(
                cone.clone(),
                false,
                vec![
                    vec![arc(ring(1.0), 1.5 * k, (0.0, TAU), false)],
                    vec![arc(ring(2.0), k, (0.0, TAU), true)],
                ],
            );
            for (u, v, expected) in [
                (0.3, 1.5, Side::In),
                (3.3, 1.01, Side::In),
                (0.3, 0.5, Side::Out),
                (0.3, 2.5, Side::Out),
                (0.3, 4.0, Side::Out),
                (2.0, 2.0, Side::On),
            ] {
                assert_eq!(frustum.side(at(u, v), tol), Ok(expected), "({u}, {v})");
            }
            assert!(!frustum.hull(tol).bbox().contains(at(0.0, 4.0), 1e-9 * k));
            assert!(frustum.hull(tol).bbox().contains(at(1.0, 1.5), 1e-9 * k));
            // Half of it, cut through the axis: two rulings that meet at
            // the apex, where the boundary turns a corner.
            let half = outline(
                cone,
                false,
                vec![vec![
                    arc(frame, 2.0 * k, (0.0, PI), false),
                    line(at(PI, 0.0), at(PI, 4.0), false),
                    line(at(0.0, 0.0), at(0.0, 4.0), true),
                ]],
            );
            for (u, v, expected) in [
                (1.0, 1.0, Side::In),
                (3.0, 3.99, Side::In),
                (4.0, 1.0, Side::Out),
                (6.2, 3.99, Side::Out),
                (0.0, 2.0, Side::On),
                (0.0, 4.0, Side::On),
            ] {
                assert_eq!(half.side(at(u, v), tol), Ok(expected), "({u}, {v})");
            }
            let inside = half.interior_points(tol, 4.0 * tol, 3).unwrap();
            assert!(!inside.is_empty());
        }
    }

    #[test]
    fn just_beyond_the_rim_of_a_cone_is_outside() {
        // The surface goes on past the rim, widening slowly: a point of it
        // a few tolerances beyond the rim is hardly further from the axis
        // than the rim is.
        let frame = tilted();
        let slope: f64 = -0.278;
        let cone = Surface::Cone(Cone::new(frame, 0.625, slope.atan()).unwrap());
        let side = outline(
            cone,
            false,
            vec![vec![arc(frame, 0.625, (0.0, TAU), false)]],
        );
        for u in [0.0, 9.3e-6, 1.0, 3.0, TAU - 1e-5] {
            for (v, expected) in [
                (-2.9e-6, Side::Out),
                (-1e-3, Side::Out),
                (2.9e-6, Side::In),
                (1e-3, Side::In),
                (0.5e-6, Side::On),
            ] {
                let r = 0.625 + v * slope;
                let p = frame.point(r * u.cos(), r * u.sin(), v);
                assert_eq!(side.side(p, 1e-6), Ok(expected), "({u}, {v})");
            }
        }
    }

    #[test]
    fn caps_bands_and_their_outsides_on_a_sphere() {
        for (frame, k) in [(Frame::WORLD, 1.0), (tilted(), 1e-3), (tilted(), 1e6)] {
            let frame = frame.at(frame.origin() * k).unwrap();
            let r = 2.0 * k;
            let tol = 1e-6 * k;
            let sphere = Surface::Sphere(Sphere::new(frame, r).unwrap());
            let at = |u: f64, v: f64| sphere.eval(u, v);
            // The circle of latitude at height h, counter-clockwise about
            // the axis.
            let parallel = |h: f64, reversed: bool| {
                let ring = frame.at(frame.point(0.0, 0.0, h * r)).unwrap();
                arc(ring, r * (1.0 - h * h).sqrt(), (0.0, TAU), reversed)
            };
            let latitude = |h: f64| h.asin();
            let build = |loops: Vec<Vec<Seg>>| {
                Outline::new(sphere.clone(), false, loops, 4.0 * tol).unwrap()
            };
            // Above the circle at 0.5; below it; between two circles; and
            // the two caps beyond them as one would-be face each.
            let top = build(vec![vec![parallel(0.5, false)]]);
            let bottom = build(vec![vec![parallel(0.5, true)]]);
            let band = build(vec![vec![parallel(-0.3, false)], vec![parallel(0.5, true)]]);
            for u in [0.0, 1.0, 2.5, 4.0, 6.0] {
                for h in [-1.0, -0.9, -0.31, -0.29, 0.0, 0.49, 0.51, 0.9, 1.0_f64] {
                    let p = at(u, latitude(h));
                    let expect = |inside: bool| Ok(if inside { Side::In } else { Side::Out });
                    assert_eq!(top.side(p, tol), expect(h > 0.5), "top at {h}");
                    assert_eq!(bottom.side(p, tol), expect(h < 0.5), "bottom at {h}");
                    assert_eq!(
                        band.side(p, tol),
                        expect(h > -0.3 && h < 0.5),
                        "band at {h}"
                    );
                }
                assert_eq!(top.side(at(u, latitude(0.5)), tol), Ok(Side::On));
            }
            // A circle through both poles (a plane through the axis): the
            // half towards +y.
            let upright = Frame::new(frame.origin(), frame.y(), frame.x()).unwrap();
            let half = build(vec![vec![arc(upright, r, (0.0, TAU), false)]]);
            for (u, v, inside) in [
                (1.0, 0.3, true),
                (2.0, -1.2, true),
                (4.0, 0.3, false),
                (5.0, 1.5, false),
            ] {
                let expected = if inside { Side::In } else { Side::Out };
                assert_eq!(half.side(at(u, v), tol), Ok(expected), "({u}, {v})");
            }
            assert_eq!(half.side(at(0.0, FRAC_PI_2), tol), Ok(Side::On));
            // The whole sphere.
            let whole = build(Vec::new());
            assert_eq!(whole.side(at(1.0, 1.0), tol), Ok(Side::In));
            assert_eq!(whole.interior_points(tol, 4.0 * tol, 2).unwrap().len(), 2);
            for face in [&top, &bottom, &band, &half] {
                for p in face.interior_points(tol, 4.0 * tol, 2).unwrap() {
                    assert_eq!(face.side(p, tol), Ok(Side::In));
                }
            }
        }
    }

    #[test]
    fn faces_on_a_torus_around_the_tube_around_the_axis_and_neither() {
        for (frame, k) in [(Frame::WORLD, 1.0), (tilted(), 1e-3), (tilted(), 1e6)] {
            let frame = frame.at(frame.origin() * k).unwrap();
            let (major, minor) = (4.0 * k, k);
            let tol = 1e-6 * k;
            let torus = Surface::Torus(Torus::new(frame, major, minor).unwrap());
            let at = |u: f64, v: f64| torus.eval(u, v);
            let build = |loops: Vec<Vec<Seg>>| {
                Outline::new(torus.clone(), false, loops, 4.0 * tol).unwrap()
            };
            // The circle around the tube at angle u about the axis, running
            // with v; and the circle about the axis at angle v around the
            // tube, running with u.
            let meridian = |u: f64, reversed: bool| {
                let center = frame.point(major * u.cos(), major * u.sin(), 0.0);
                let out = frame.vector(u.cos(), u.sin(), 0.0);
                let around = Frame::new(center, out.cross(frame.z()), out).unwrap();
                arc(around, minor, (0.0, TAU), reversed)
            };
            let parallel = |v: f64, reversed: bool| {
                let ring = frame.at(frame.point(0.0, 0.0, minor * v.sin())).unwrap();
                arc(ring, major + minor * v.cos(), (0.0, TAU), reversed)
            };
            // The face is to the left of its boundary seen from outside:
            // beyond a meridian running with v lies lower u.
            let elbow = build(vec![vec![meridian(1.0, true)], vec![meridian(2.5, false)]]);
            let rest = build(vec![vec![meridian(1.0, false)], vec![meridian(2.5, true)]]);
            // Beyond a parallel running with u lies higher v.
            let outer = build(vec![vec![parallel(-1.0, false)], vec![parallel(1.0, true)]]);
            let inner = build(vec![vec![parallel(-1.0, true)], vec![parallel(1.0, false)]]);
            let wrap = |a: f64| a.rem_euclid(TAU);
            for i in 0..12 {
                for j in 0..12 {
                    let (u, v) = (0.11 + f64::from(i) * 0.52, 0.07 + f64::from(j) * 0.52);
                    let p = at(u, v);
                    let expect = |inside: bool| Ok(if inside { Side::In } else { Side::Out });
                    let between = wrap(u) > 1.0 && wrap(u) < 2.5;
                    assert_eq!(elbow.side(p, tol), expect(between), "elbow ({u}, {v})");
                    assert_eq!(rest.side(p, tol), expect(!between), "rest ({u}, {v})");
                    let outside = wrap(v) < 1.0 || wrap(v) > TAU - 1.0;
                    assert_eq!(outer.side(p, tol), expect(outside), "outer ({u}, {v})");
                    assert_eq!(inner.side(p, tol), expect(!outside), "inner ({u}, {v})");
                }
            }
            for face in [&elbow, &rest, &outer, &inner] {
                for p in face.interior_points(tol, 4.0 * tol, 2).unwrap() {
                    assert_eq!(face.side(p, tol), Ok(Side::In));
                }
            }
            assert_eq!(elbow.side(at(1.0, 2.0), tol), Ok(Side::On));
            assert_eq!(build(Vec::new()).side(at(1.0, 2.0), tol), Ok(Side::In));
        }
    }

    #[test]
    fn a_spline_boundary_is_a_boundary_like_any_other() {
        let frame = tilted();
        let rim = Curve3::Nurbs(NurbsCurve::circle(&frame, 2.0).unwrap());
        let seg = Seg {
            a: rim.eval(0.0),
            b: rim.eval(1.0),
            curve: rim,
            range: (0.0, 1.0),
            reversed: false,
        };
        let disc = outline(Surface::Plane(Plane::new(frame)), false, vec![vec![seg]]);
        assert_eq!(disc.side(frame.point(1.9, 0.0, 0.0), 1e-6), Ok(Side::In));
        assert_eq!(disc.side(frame.point(-1.4, 1.4, 0.0), 1e-6), Ok(Side::In));
        assert_eq!(disc.side(frame.point(1.5, 1.5, 0.0), 1e-6), Ok(Side::Out));
        assert_eq!(disc.side(frame.point(0.0, -2.0, 0.0), 1e-6), Ok(Side::On));
        // The nearest point of a spline piece is the nearest.
        let curve = &disc.loops[0][0].curve;
        let mut seed = 3_u64;
        let mut unit = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        for _ in 0..100 {
            let t0 = unit() * 0.7;
            let range = (t0, t0 + 0.05 + unit() * 0.25);
            let p = frame.point((unit() - 0.5) * 6.0, (unit() - 0.5) * 6.0, unit() - 0.5);
            let (t, d) = nearest(curve, range, p).unwrap();
            assert!(t >= range.0 && t <= range.1);
            let mut brute = f64::INFINITY;
            for k in 0..=5000 {
                let s = range.0 + (range.1 - range.0) * f64::from(k) / 5000.0;
                brute = brute.min(curve.eval(s).distance(p));
            }
            assert!(d <= brute + 1e-12, "{d} is not the nearest: {brute}");
            assert!(d >= brute - 1e-2);
        }
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

    /// The union of stretches of turn: apart, overlapping, across the
    /// start of the turn (joined there), the whole turn (empty), none.
    #[test]
    fn stretches_of_turn_are_united_round_the_circle() {
        let close = |a: &[(f64, f64)], b: &[(f64, f64)]| {
            a.len() == b.len()
                && a.iter()
                    .zip(b)
                    .all(|(p, q)| (p.0 - q.0).abs() < 1e-12 && (p.1 - q.1).abs() < 1e-12)
        };
        let got = union_round(&[(1.0, 0.5), (0.2, 0.3)]);
        assert!(close(&got, &[(0.2, 0.5), (1.0, 1.5)]), "{got:?}");
        let got = union_round(&[(1.0, 0.5), (1.2, 1.0)]);
        assert!(close(&got, &[(1.0, 2.2)]), "{got:?}");
        // Across the start: one stretch from 6.0 to 0.5 past a whole turn.
        let got = union_round(&[(6.0, 0.6), (0.1, 0.4)]);
        assert!(close(&got, &[(6.0, TAU + 0.5)]), "{got:?}");
        let got = union_round(&[(6.0, 0.6), (3.0, 0.4)]);
        assert!(close(&got, &[(6.0, 6.6), (3.0, 3.4)]), "{got:?}");
        assert!(union_round(&[(0.0, 3.2), (3.1, 3.2)]).is_empty());
        assert!(union_round(&[(5.0, 3.0), (1.5, 3.6)]).is_empty());
        assert!(union_round(&[]).is_empty());
    }

    /// The turns of a face on a torus from the turns of its boundary's
    /// pieces and whether it holds a circle of the tube in each gap.
    #[test]
    fn the_turn_of_a_face_on_a_torus() {
        let frame = Frame::WORLD;
        let (major, minor) = (3.0, 1.0);
        let at = |a: f64| frame.point(major * a.cos(), major * a.sin(), 0.5);
        let piece = |a: f64, b: f64| vec![at(a), at(0.5 * (a + b)), at(b)];
        let pieces = vec![piece(0.1, 0.3), piece(1.0, 1.2)];
        let turn_of = |p: Vec3| {
            let (x, y, _) = frame.local(p);
            y.atan2(x).rem_euclid(TAU)
        };
        // The face holds the circles between the pieces the short way.
        let mut short = |p: Vec3| -> Result<bool, ()> {
            let a = turn_of(p);
            Ok(a > 0.3 && a < 1.0)
        };
        let turns = torus_turns(&frame, major, minor, &pieces, &mut short)
            .unwrap()
            .unwrap();
        assert_eq!(turns.len(), 1, "{turns:?}");
        assert!(
            (turns[0].0 - 0.1).abs() < 1e-12 && (turns[0].1 - 1.2).abs() < 1e-12,
            "{turns:?}"
        );
        // The long way round.
        let mut long = |p: Vec3| -> Result<bool, ()> {
            let a = turn_of(p);
            Ok(!(a > 0.3 && a < 1.0))
        };
        let turns = torus_turns(&frame, major, minor, &pieces, &mut long)
            .unwrap()
            .unwrap();
        assert_eq!(turns.len(), 1, "{turns:?}");
        assert!((turns[0].0 - 1.0).abs() < 1e-12, "{turns:?}");
        assert!((turns[0].1 - (TAU + 0.3)).abs() < 1e-12, "{turns:?}");
        // Neither gap: two faces' worth, each piece's own stretch.
        let mut neither = |_: Vec3| -> Result<bool, ()> { Ok(false) };
        let turns = torus_turns(&frame, major, minor, &pieces, &mut neither)
            .unwrap()
            .unwrap();
        assert_eq!(turns.len(), 2, "{turns:?}");
        // Both: the whole turn, which is no stretch.
        let mut both = |_: Vec3| -> Result<bool, ()> { Ok(true) };
        assert!(
            torus_turns(&frame, major, minor, &pieces, &mut both)
                .unwrap()
                .is_none()
        );
        // A piece through the axis, or reaching half a turn: not told.
        let through = vec![vec![frame.point(0.0, 0.0, 0.5), at(0.2)]];
        assert!(
            torus_turns(&frame, major, minor, &through, &mut both)
                .unwrap()
                .is_none()
        );
        let half = vec![vec![at(0.0), at(1.6), at(3.2)]];
        assert!(
            torus_turns(&frame, major, minor, &half, &mut both)
                .unwrap()
                .is_none()
        );
        // An error asking: passed on (the caller keeps the ball).
        let mut fails = |_: Vec3| -> Result<bool, ()> { Err(()) };
        assert!(torus_turns(&frame, major, minor, &pieces, &mut fails).is_err());
    }

    /// Every point of an arc of a circle or an ellipse lies within the
    /// polygon `arc_polygon` gives, for pieces of a quarter and of an
    /// eighth of a turn: 400 seeded arcs, 200 points each, tested in the
    /// arc's own plane, where the polygon is convex.
    #[test]
    fn an_arc_lies_within_its_polygon() {
        let mut state = 0x5eed_a5c5_u64;
        let mut unit = || {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            (z ^ (z >> 31)) as f64 / u64::MAX as f64
        };
        for case in 0..400 {
            let k = [1e-3, 1.0, 1e6][case % 3];
            let centre = Vec3::new(unit() - 0.5, unit() - 0.5, unit() - 0.5) * (10.0 * k);
            let normal = Vec3::new(unit() - 0.5, unit() - 0.5, unit() - 0.5 + 1e-3);
            let frame = Frame::from_z(centre, normal).unwrap();
            let (a, b) = ((0.1 + unit()) * k, (0.1 + unit()) * k);
            let (x, y) = (frame.x() * a, frame.y() * b);
            let from = TAU * (unit() - 0.5) * 2.0;
            let sweep = TAU * unit() * 0.999;
            for piece in [FRAC_PI_2, FRAC_PI_8] {
                let mut polygon = Vec::new();
                assert!(arc_polygon(
                    centre,
                    x,
                    y,
                    (from, from + sweep),
                    piece,
                    &mut polygon
                ));
                // In the arc's coordinates: the unit circle.
                let flat = |p: Vec3| {
                    let d = p - centre;
                    (d.dot(x) / (a * a), d.dot(y) / (b * b))
                };
                let corners: Vec<(f64, f64)> = polygon.iter().map(|&p| flat(p)).collect();
                for i in 0..=200 {
                    let t = from + sweep * f64::from(i) / 200.0;
                    let q = flat(centre + x * t.cos() + y * t.sin());
                    let n = corners.len();
                    for j in 0..n {
                        let (p0, p1) = (corners[j], corners[(j + 1) % n]);
                        let cross = (p1.0 - p0.0) * (q.1 - p0.1) - (p1.1 - p0.1) * (q.0 - p0.0);
                        assert!(cross >= -1e-9, "case {case}: {q:?} outside at side {j}");
                    }
                }
            }
        }
        // A whole turn or more is no arc: the caller keeps the conic.
        let mut none = Vec::new();
        assert!(!arc_polygon(
            Vec3::ZERO,
            Vec3::X,
            Vec3::Y,
            (0.0, TAU),
            FRAC_PI_2,
            &mut none
        ));
        assert!(none.is_empty());
    }
}
