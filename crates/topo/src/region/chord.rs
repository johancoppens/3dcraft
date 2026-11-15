//! Verify before refusing (since 2026-11-15; `docs/design/topology.md` ›
//! *What lies between an edge and its chord*).
//!
//! The walk of the loop rules ([`super::follow`]) reads each step of an
//! edge at its two ends and its middle and takes the chord of the step for
//! the edge; how far the edge lies from that chord is measured at the
//! step's middle, never bounded. So the walk can see a fault that is not
//! there (a wavy side whose steps end, and are measured, where it crosses
//! its middle line reads as straight, and its chords cross the other side),
//! and can miss one that is. Here a face the walk is about to refuse is
//! walked again, on a plane, a cylinder, a cone or a sphere, in stretches
//! each of which is shown to keep to a bound, and the fault is claimed only
//! where it holds with those bounds; a face the walk passes is never walked
//! here, so what this adds is only ever a pass where the walk refused.
//!
//! **What is shown of a stretch.** A stretch of an edge, from the
//! parameter `s` to `e`, its image in the chart running from `A` to `B`:
//!
//! - (1) the image runs one way: its direction keeps within a fan
//!   narrower than a half turn (the legs of points whose convex hull holds
//!   the curve and whose legs' sums with weights that are not negative hold
//!   its direction: the cut Bézier pieces of a spline, the three points of a
//!   rational quadratic for an arc of a conic, the two ends of a line; on a
//!   plane each leg read along the plane, about an axis each leg's box of
//!   directions over every place the stretch can be, [`super::AboutMap`],
//!   widened by what rounding can have moved the leg). The image and the
//!   chord `AB` are then each crossed once by every line square to the
//!   fan's middle, so together they wind about no point more than once,
//!   and only about points between them.
//! - (2) the image lies within `bound` of the chord `AB`: on a plane, the
//!   farthest of those points from the chord (the plane's chart is linear,
//!   so the image lies in their hull, and a hull lies as far from a segment
//!   as its farthest corner); about an axis, from the fan alone: a curve
//!   from `A` to `B` whose direction keeps within the cone of the fan's two
//!   ends `d1`, `d2` lies in the parallelogram `(A + cone) ∩ (B − cone)`,
//!   whose other two corners lie `α β |d1 × d2| / |AB|` from the chord
//!   (`B − A = α d1 + β d2`, both not negative: else the stretch is not
//!   shown). Each bound has what rounding can have moved the points added.
//!
//! **What is claimed of a loop.** The polyline of the stretches' starts,
//! each segment with its margin (its stretch's bound and the gap from the
//! stretch's end to the next one's start, where two edges meet at a vertex
//! within its tolerance): the loop's true boundary lies within a margin of
//! each segment, and with each segment winds about no point outside that
//! margin. So (a) its area is the polyline's within `Σ 2 m (L + m) + π m²`
//! over the segments (length `L`, margin `m`), and a loop's sense is
//! claimed only where its area is farther from nothing than that; (b) a
//! point farther from every segment than its margin and ten tolerances is
//! wound by the loop as by the polyline; (c) two segments that cross
//! properly, each one's ends on either side of the other's line by more
//! than both margins and ten tolerances, are crossed by the loop's true
//! boundary: each boundary stretch lies in the band of its segment's
//! margin, and runs from one side of the other band to its other side,
//! so within the parallelogram where the two bands meet one joins one pair
//! of its opposite sides and the other the other pair, and two such paths
//! meet. The region rule's faults (a sense, a hole outside, a crossing) and
//! the rule of simple loops' third (a point wound neither 0 times nor once
//! its own way) are claimed again with these, and the walk's message is
//! kept where the same kind of fault is shown.
//!
//! **What is not shown, and passes.** Where a stretch is not shown, or the
//! bounds are too wide to show the fault, stretches are halved, those with
//! the widest margins first, until the fault is shown, or no two segments
//! that do not meet at a point of their loop come within their margins and
//! every sense and hole is told (nothing can be shown by halving further:
//! two stretches that meet at a point are not held to each other, so a
//! fault within two neighbouring stretches is not looked for), or every
//! margin is within ten tolerances, or [`super::MOST_PROOF_WORK`] units, [`MOST_ROUNDS`]
//! rounds or [`MOST_STRETCHES`] stretches of an edge are spent: then the
//! fault is not shown and the face passes. Not asked here at all, the walk's
//! fault kept as it is: a face on a torus (whose chart is not shown,
//! [`Rise::Unshown`]) or on a spline surface (whose chart's map is not
//! known), and every other rule (an edge round its circle twice, a loop
//! round a period too often, two stretches on each other).

use super::{
    AboutMap, Fan, Held, Lay, MOST_PAIRS, ONE_WAY, Pieces, QUARTER_TURN, Ring, Rise, Sweep, Wedge,
    hull_rounding, one_way, pairs_of_steps, shift_holes, signed_area, spend, to_segment, winding,
};
use crate::{Body, Coedge, EdgeId, Face};
use craft3d_geom::{Curve3, Surface};
use craft3d_math::{Portable, Tolerance, Vec3};
use std::collections::BTreeMap;
use std::f64::consts::PI;

/// Rounds of halving at the most.
const MOST_ROUNDS: usize = 24;
/// Stretches of one edge at the most.
const MOST_STRETCHES: usize = 1 << 13;
/// What placing a point counts for against [`super::MOST_PROOF_WORK`]: one
/// evaluation of the curve and its place on the surface, half a step of
/// the walk.
const POINT_UNITS: usize = 16;
/// How far a fan's two ends are turned outward (radians) before the
/// parallelogram is made, so that a cross product rounded on a direction
/// of the fan cannot leave it outside.
const WIDEN: f64 = 1e-9;
/// Units in the last place of a chart point's largest coordinate by which
/// it may be off: the curve's point and its place on the surface.
const PLACE_ROUNDING: f64 = 64.0 * f64::EPSILON;

/// The surfaces whose chart takes a direction by a map that is known
/// ([`Lay`]): a plane, a cylinder, a cone, a sphere.
fn asked(surface: &Surface) -> bool {
    matches!(
        surface,
        Surface::Plane(_) | Surface::Cylinder(_) | Surface::Cone(_) | Surface::Sphere(_)
    )
}

/// Where a stretch stands.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Bound {
    /// Not looked at yet.
    Unknown,
    /// Shown: the image lies within this of its chord, and runs one way.
    Shown(f64),
    /// Not shown at this length: to be halved.
    Not,
}

/// A stretch of an edge, in the order its loop runs.
#[derive(Clone, Copy, Debug)]
struct Stretch {
    /// The coedge's place in its loop.
    coedge: usize,
    /// The edge's parameters at the stretch's start and its end (`e < s`
    /// where the coedge runs against its edge).
    s: f64,
    e: f64,
    /// Their places in the chart, as the surface's projection gives them
    /// (not moved by periods).
    start: [f64; 2],
    end: [f64; 2],
    bound: Bound,
}

/// A loop as the polyline of its stretches' starts, with each segment's
/// margin ([`Walked::rings`]).
type Margined = (Ring, Vec<f64>);

/// A face's loops walked in stretches.
struct Walked<'a> {
    body: &'a Body,
    surface: &'a Surface,
    lay: Lay,
    periods: [Option<f64>; 2],
    near: f64,
    /// The shortest of `∂u`, `∂v` at the points placed.
    scale: f64,
    loops: Vec<(Vec<Coedge>, Vec<Stretch>)>,
    pieces: BTreeMap<EdgeId, Pieces<'a>>,
    work: usize,
}

/// The place of `p` on `surface`, as the walk's chart takes it
/// ([`super::Chart::at`], an analytic surface): `None` off the surface by
/// more than `near`, at a sphere's pole or a cone's apex, or for numbers
/// that are none; and the shortest of `∂u`, `∂v` there.
fn place(surface: &Surface, near: f64, p: Vec3) -> Option<([f64; 2], f64)> {
    let (u, v) = surface.project(p).ok()?;
    let (at, du, dv) = surface.derivatives(u, v);
    let off = at.distance(p);
    if off.is_nan() || off > near {
        return None;
    }
    match surface {
        Surface::Sphere(s) if s.radius() * v.pcos() <= near => return None,
        Surface::Cone(c) if surface.eval(0.0, c.apex_v()).distance(p) <= near => return None,
        _ => {}
    }
    (u.is_finite() && v.is_finite()).then(|| ([u, v], du.length().min(dv.length())))
}

impl<'a> Walked<'a> {
    /// The loops of `face` in stretches: the halves of the walk's steps
    /// (a line in one on a plane, a conic's steps of at most π/8, a
    /// spline's shown steps), each end placed. `None` where a loop cannot
    /// be walked so (a point off the surface or at a pole, the work spent).
    fn new(body: &'a Body, face: &'a Face, tol: &Tolerance) -> Option<Walked<'a>> {
        let surface = &face.surface;
        let near = 10.0 * body.working_tolerance(tol.linear);
        let mut walked = Walked {
            body,
            surface,
            lay: Lay::of(surface),
            periods: [surface.period_u(), surface.period_v()],
            near,
            scale: f64::INFINITY,
            loops: Vec::with_capacity(face.loops.len()),
            pieces: BTreeMap::new(),
            work: 0,
        };
        let planar = matches!(surface, Surface::Plane(_));
        for id in &face.loops {
            let lp = body.loop_(*id)?;
            let mut stretches = Vec::new();
            for (c, coedge) in lp.coedges.iter().enumerate() {
                let edge = body.edge(coedge.edge)?;
                let (t0, t1) = edge.range;
                if !(t0.is_finite() && t1.is_finite() && t1 > t0) {
                    return None;
                }
                // Few stretches to begin with, halved where a stretch is
                // not shown or its bound is too wide: a line in one on a
                // plane, else in two; an arc in quarter turns at the most
                // (a rational quadratic holds it); a spline in two.
                let count = match &edge.curve {
                    Curve3::Line(_) if planar => 1,
                    Curve3::Line(_) | Curve3::Nurbs(_) => 2,
                    Curve3::Circle(_) | Curve3::Ellipse(_) => {
                        let n = ((t1 - t0) / QUARTER_TURN).ceil();
                        if !n.is_finite() {
                            return None;
                        }
                        (n.max(1.0) as usize).min(MOST_STRETCHES)
                    }
                };
                let at = |k: usize| {
                    let f = k as f64 / count as f64;
                    if coedge.reversed {
                        t1 - (t1 - t0) * f
                    } else {
                        t0 + (t1 - t0) * f
                    }
                };
                let mut before = walked.place_at(&edge.curve, at(0))?;
                for k in 0..count {
                    let next = walked.place_at(&edge.curve, at(k + 1))?;
                    stretches.push(Stretch {
                        coedge: c,
                        s: at(k),
                        e: at(k + 1),
                        start: before,
                        end: next,
                        bound: Bound::Unknown,
                    });
                    before = next;
                }
            }
            if stretches.is_empty() {
                return None;
            }
            walked.loops.push((lp.coedges.clone(), stretches));
        }
        Some(walked)
    }

    /// The edge's point at `t` placed (and counted).
    fn place_at(&mut self, curve: &Curve3, t: f64) -> Option<[f64; 2]> {
        spend(&mut self.work, POINT_UNITS)?;
        let (uv, scale) = place(self.surface, self.near, curve.eval(t))?;
        if scale.is_finite() {
            self.scale = self.scale.min(scale);
        }
        Some(uv)
    }

    /// Ten tolerances in parameters at the surface's smallest scale over
    /// the points placed.
    fn reach(&self) -> f64 {
        self.near / self.scale.max(f64::MIN_POSITIVE)
    }

    /// Shows the bound of every stretch not looked at, and halves every
    /// one not shown, until all are shown. `None` where one cannot be
    /// halved further or the work is spent.
    fn show_all(&mut self) -> Option<()> {
        for _ in 0..=MOST_STRETCHES.ilog2() + 1 {
            let mut halve = Vec::new();
            for l in 0..self.loops.len() {
                let count = self.loops.get(l).map_or(0, |(_, s)| s.len());
                for k in 0..count {
                    let stretch = *self.loops.get(l)?.1.get(k)?;
                    let bound = match stretch.bound {
                        Bound::Unknown => self.bound_of(l, k)?,
                        other => other,
                    };
                    if let Some(st) = self.loops.get_mut(l).and_then(|(_, s)| s.get_mut(k)) {
                        st.bound = bound;
                    }
                    if bound == Bound::Not {
                        halve.push((l, k));
                    }
                }
            }
            if halve.is_empty() {
                return Some(());
            }
            self.halve(&halve)?;
        }
        None
    }

    /// Halves the stretches `which` (loop, place), each at its middle.
    /// `None` past the stretches of an edge or the work.
    fn halve(&mut self, which: &[(usize, usize)]) -> Option<()> {
        let body = self.body;
        for l in 0..self.loops.len() {
            let mut marked: Vec<usize> = which
                .iter()
                .filter(|(m, _)| *m == l)
                .map(|(_, k)| *k)
                .collect();
            if marked.is_empty() {
                continue;
            }
            marked.sort_unstable();
            let (coedges, old) = self.loops.get_mut(l)?;
            let coedges = coedges.clone();
            let old = std::mem::take(old);
            let mut of_coedge: BTreeMap<usize, usize> = BTreeMap::new();
            for st in &old {
                *of_coedge.entry(st.coedge).or_insert(0) += 1;
            }
            let mut out = Vec::with_capacity(old.len() + marked.len());
            let mut next = marked.iter().peekable();
            for (k, stretch) in old.into_iter().enumerate() {
                if next.peek() != Some(&&k) {
                    out.push(stretch);
                    continue;
                }
                next.next();
                let count = of_coedge.entry(stretch.coedge).or_insert(0);
                if *count >= MOST_STRETCHES {
                    return None;
                }
                *count += 1;
                let edge = body.edge(coedges.get(stretch.coedge)?.edge)?;
                let m = 0.5 * (stretch.s + stretch.e);
                let middle = self.place_at(&edge.curve, m)?;
                out.push(Stretch {
                    e: m,
                    end: middle,
                    bound: Bound::Unknown,
                    ..stretch
                });
                out.push(Stretch {
                    s: m,
                    start: middle,
                    bound: Bound::Unknown,
                    ..stretch
                });
            }
            self.loops.get_mut(l)?.1 = out;
        }
        Some(())
    }

    /// The bound of a stretch ([`Bound::Shown`]) or that it is not shown
    /// ([`Bound::Not`]); `None` where the work is spent.
    fn bound_of(&mut self, l: usize, k: usize) -> Option<Bound> {
        let body = self.body;
        let (coedges, stretches) = self.loops.get(l)?;
        let stretch = *stretches.get(k)?;
        let edge_id = coedges.get(stretch.coedge)?.edge;
        let edge = body.edge(edge_id)?;
        let (lo, hi) = (stretch.s.min(stretch.e), stretch.s.max(stretch.e));
        // The chart's points at the lower and the higher parameter.
        let (a, b) = if stretch.s <= stretch.e {
            (stretch.start, stretch.end)
        } else {
            (stretch.end, stretch.start)
        };
        let mut chord = [b[0] - a[0], b[1] - a[1]];
        for (d, period) in self.periods.iter().enumerate() {
            if let (Some(p), Some(x)) = (period, chord.get_mut(d)) {
                *x -= (*x / p).round() * p;
            }
        }
        let mut held = Held::default();
        match &edge.curve {
            Curve3::Line(_) => {
                let ends = [edge.curve.eval(lo), edge.curve.eval(hi)];
                held.take(&ends, rounding_of(&ends));
                self.hull_bound(&held, a, b, chord)
            }
            Curve3::Circle(_) | Curve3::Ellipse(_) => {
                let (frame, ra, rb) = match &edge.curve {
                    Curve3::Circle(c) => (*c.frame(), c.radius(), c.radius()),
                    Curve3::Ellipse(e) => {
                        let (x, y) = e.semi_axes();
                        (*e.frame(), x, y)
                    }
                    Curve3::Line(_) | Curve3::Nurbs(_) => return Some(Bound::Not),
                };
                let half = 0.5 * (hi - lo);
                // A rational quadratic holds an arc of up to a half turn;
                // the walk's arcs are a sixteenth of one.
                if !(half > 0.0 && half <= 0.25 * PI) {
                    return Some(Bound::Not);
                }
                let (sm, cm) = (0.5 * (lo + hi)).psin_cos();
                let lift = 1.0 / half.pcos();
                let points = [
                    edge.curve.eval(lo),
                    frame.point(ra * cm * lift, rb * sm * lift, 0.0),
                    edge.curve.eval(hi),
                ];
                held.take(&points, rounding_of(&points));
                self.hull_bound(&held, a, b, chord)
            }
            Curve3::Nurbs(n) => {
                let Some(rounding) = hull_rounding(n) else {
                    return Some(Bound::Not);
                };
                let clear = super::HULL_CLEAR * rounding;
                // Over many spans, the control points of the spans the
                // stretch meets, as they are: their hull holds the curve
                // there and their legs its direction, and they cost no
                // piece made (they reach a degree's spans past the stretch
                // either way, little over many spans); not shown so, the
                // stretch is halved. Over fewer, the Bézier pieces cut to it.
                let (d0, d1) = n.domain();
                let (knots, ctrl, degree) = (n.knots(), n.control_points(), n.degree());
                let span = |t: f64| {
                    knots
                        .partition_point(|k| *k <= t)
                        .saturating_sub(1)
                        .clamp(degree, ctrl.len().saturating_sub(1))
                };
                if lo >= d0 && hi <= d1 && span(hi).saturating_sub(span(lo)) > super::MANY_SPANS {
                    let first = span(lo).saturating_sub(degree);
                    let Some(points) = ctrl.get(first..=span(hi)) else {
                        return Some(Bound::Not);
                    };
                    held.take(points, clear);
                    return self.hull_bound(&held, a, b, chord);
                }
                // Then the Bézier pieces cut to the stretch.
                let pieces = self.pieces.entry(edge_id).or_insert_with(|| Pieces::of(n));
                let taken = pieces.each_piece((lo, hi), &mut self.work, |points, growth| {
                    held.take(points, clear * growth);
                    true
                })?;
                if !taken {
                    return Some(Bound::Not);
                }
                self.hull_bound(&held, a, b, chord)
            }
        }
    }

    /// The bound shown by points `held` whose hull holds the stretch and
    /// whose legs hold its direction, the stretch's chart points at its
    /// lower and higher parameter `a`, `b` and the chord between them (`u`
    /// moved by whole periods to the nearest).
    fn hull_bound(
        &mut self,
        held: &Held,
        a: [f64; 2],
        b: [f64; 2],
        chord: [f64; 2],
    ) -> Option<Bound> {
        let widest_clear = held.runs.iter().map(|r| r.1).fold(0.0_f64, f64::max);
        let size = a
            .iter()
            .chain(b.iter())
            .fold(0.0_f64, |m, x| m.max(x.abs()));
        let rounded = PLACE_ROUNDING * size;
        match self.lay {
            Lay::Flat { .. } => {
                let (one, legs) = one_way(&self.lay, &held.points, &held.runs, None);
                spend(&mut self.work, legs)?;
                if !one {
                    return Some(Bound::Not);
                }
                // The plane's chart is the plane's frame: the image of the
                // hull is the hull of the points' images.
                let mut far = 0.0_f64;
                let mut starts = 0_usize;
                for &(end, clear) in &held.runs {
                    for p in held.points.get(starts..end).unwrap_or(&[]) {
                        let Some((q, _)) = place(self.surface, f64::INFINITY, *p) else {
                            return Some(Bound::Not);
                        };
                        far = far.max(to_segment(q, a, b) + clear);
                    }
                    starts = end;
                }
                spend(&mut self.work, held.points.len())?;
                let bound = far + 2.0 * widest_clear + rounded;
                Some(if bound.is_finite() {
                    Bound::Shown(bound)
                } else {
                    Bound::Not
                })
            }
            Lay::About {
                rise: Rise::Unshown,
                ..
            }
            | Lay::Space => Some(Bound::Not),
            Lay::About { origin, z, rise } => {
                let mut wedge = Wedge::about((origin, z), QUARTER_TURN);
                let mut starts = 0_usize;
                for &(end, clear) in &held.runs {
                    for p in held.points.get(starts..end).unwrap_or(&[]) {
                        if !wedge.add(*p, clear) {
                            return Some(Bound::Not);
                        }
                    }
                    starts = end;
                }
                spend(&mut self.work, held.points.len())?;
                let Some(reach) = wedge.reach() else {
                    return Some(Bound::Not);
                };
                let Some(map) = AboutMap::new(z, rise, &reach) else {
                    return Some(Bound::Not);
                };
                let (rate_u, rate_v) = map.rate();
                let mut fan = Fan::within(ONE_WAY);
                let mut legs = 0_usize;
                let mut starts = 0_usize;
                let mut fits = true;
                'runs: for &(end, clear) in &held.runs {
                    let run = held.points.get(starts..end).unwrap_or(&[]);
                    starts = end;
                    for pair in run.windows(2) {
                        let [p, q] = pair else { continue };
                        legs += 1;
                        let w = *q - *p;
                        if w.x == 0.0 && w.y == 0.0 && w.z == 0.0 {
                            continue;
                        }
                        // Each end may be `clear` off: the leg twice that.
                        let moved = 2.0 * clear;
                        if w.length().partial_cmp(&moved) != Some(std::cmp::Ordering::Greater) {
                            fits = false;
                            break 'runs;
                        }
                        let (u, v) = map.ranges(w);
                        let (du, dv) = (moved * rate_u, moved * rate_v);
                        let (u, v) = ((u.0 - du, u.1 + du), (v.0 - dv, v.1 + dv));
                        for t in [(u.0, v.0), (u.0, v.1), (u.1, v.0), (u.1, v.1)] {
                            if !fan.add(t) {
                                fits = false;
                                break 'runs;
                            }
                        }
                    }
                }
                spend(&mut self.work, legs * super::ABOUT_UNITS)?;
                if !fits {
                    return Some(Bound::Not);
                }
                let Some((low, high)) = fan.ends else {
                    // Every leg of no length: the stretch is a point.
                    return Some(if chord == [0.0, 0.0] {
                        Bound::Shown(rounded)
                    } else {
                        Bound::Not
                    });
                };
                let (su, sv) = map.scale();
                let turn = |(x, y): (f64, f64), by: f64| {
                    let (s, c) = by.psin_cos();
                    (x * c - y * s, x * s + y * c)
                };
                let (l, h) = (turn(low, -WIDEN), turn(high, WIDEN));
                let d1 = (l.0 / su, l.1 / sv);
                let d2 = (h.0 / su, h.1 / sv);
                let Some(off) = parallelogram(d1, d2, (chord[0], chord[1])) else {
                    return Some(Bound::Not);
                };
                // A point of the curve may be `widest_clear` off in space:
                // in the chart that over the least distance from the axis
                // (`u`), and as much again or over it (`v`).
                let near = reach.off.0;
                let moved = widest_clear * (2.0 / near + 1.0);
                let bound = off + 2.0 * moved + rounded;
                Some(if bound.is_finite() {
                    Bound::Shown(bound)
                } else {
                    Bound::Not
                })
            }
        }
    }

    /// The loops as polylines of their stretches' starts, each point moved
    /// by whole periods to the one before (as the walk does), with each
    /// segment's margin: its stretch's bound and the gap from its end to the
    /// next start. `None` where a stretch is not shown; `Some(None)` where a
    /// loop goes round a period (a band, which the region rule passes).
    fn rings(&self) -> Option<Option<Vec<Margined>>> {
        let mut out = Vec::with_capacity(self.loops.len());
        for (_, stretches) in &self.loops {
            let n = stretches.len();
            let mut points = Vec::with_capacity(n);
            let mut ends = Vec::with_capacity(n);
            let mut bounds = Vec::with_capacity(n);
            let mut starts = Vec::new();
            let mut before: Option<[f64; 2]> = None;
            let mut coedge = usize::MAX;
            for st in stretches {
                let Bound::Shown(bound) = st.bound else {
                    return None;
                };
                let a = self.unwrapped(st.start, before);
                let b = self.unwrapped(st.end, Some(a));
                if st.coedge != coedge {
                    starts.push(points.len());
                    coedge = st.coedge;
                }
                points.push(a);
                ends.push(b);
                bounds.push(bound);
                before = Some(b);
            }
            let (Some(first), Some(last)) = (points.first(), ends.last()) else {
                return None;
            };
            for (d, period) in self.periods.iter().enumerate() {
                if let (Some(p), Some(x), Some(y)) = (period, first.get(d), last.get(d))
                    && (x - y).abs() > 0.5 * p
                {
                    return Some(None);
                }
            }
            let margins: Vec<f64> = (0..n)
                .map(|i| {
                    let (Some(b), Some(next), Some(bound)) =
                        (ends.get(i), points.get((i + 1) % n), bounds.get(i))
                    else {
                        return f64::INFINITY;
                    };
                    bound + (b[0] - next[0]).phypot(b[1] - next[1])
                })
                .collect();
            let mut low = [f64::INFINITY; 2];
            let mut high = [f64::NEG_INFINITY; 2];
            for p in &points {
                low = [low[0].min(p[0]), low[1].min(p[1])];
                high = [high[0].max(p[0]), high[1].max(p[1])];
            }
            let error = margins.iter().copied().fold(0.0_f64, f64::max);
            out.push((
                Ring {
                    points,
                    winds: [0.0; 2],
                    error,
                    low,
                    high,
                    starts,
                    aliased: false,
                },
                margins,
            ));
        }
        Some(Some(out))
    }

    /// `p` moved by whole periods to lie nearest `before`.
    fn unwrapped(&self, p: [f64; 2], before: Option<[f64; 2]>) -> [f64; 2] {
        let mut p = p;
        if let Some(b) = before {
            for (d, period) in self.periods.iter().enumerate() {
                if let (Some(per), Some(x), Some(y)) = (period, p.get_mut(d), b.get(d)) {
                    *x += ((y - *x) / per).round() * per;
                }
            }
        }
        p
    }
}

/// How far rounding can have put points made in closed form (a line's
/// ends, an arc's rational quadratic): 64 units in the last place of their
/// largest coordinate.
fn rounding_of(points: &[Vec3]) -> f64 {
    let size = points.iter().fold(0.0_f64, |m, p| {
        m.max(p.x.abs()).max(p.y.abs()).max(p.z.abs())
    });
    64.0 * f64::EPSILON * size
}

/// How far a curve from `(0, 0)` to `chord` whose direction keeps within
/// the cone of `d1` and `d2` (`d2` anticlockwise of `d1`, less than a half
/// turn) lies from the chord at most: the curve lies in the parallelogram
/// `cone ∩ (chord − cone)`, the hull of its four corners `0`, `α d1`,
/// `chord`, `β d2` (`chord = α d1 + β d2`), so no farther from the segment
/// than the farther of `α d1` and `β d2` (a distance from a segment is
/// largest over a hull at a corner). `None` where the chord is not in the
/// cone (`α` or `β` negative: the curve does not keep within it), or a
/// number is none. A chord of no length: the curve is a point, 0.
fn parallelogram(d1: (f64, f64), d2: (f64, f64), chord: (f64, f64)) -> Option<f64> {
    let cross = |p: (f64, f64), q: (f64, f64)| p.0 * q.1 - p.1 * q.0;
    let det = cross(d1, d2);
    if !(det > 0.0 && det.is_finite()) {
        return None;
    }
    let alpha = cross(chord, d2) / det;
    let beta = cross(d1, chord) / det;
    if !(alpha >= 0.0 && beta >= 0.0) {
        return None;
    }
    if chord == (0.0, 0.0) {
        return Some(0.0);
    }
    let (o, c) = ([0.0, 0.0], [chord.0, chord.1]);
    let off = to_segment([alpha * d1.0, alpha * d1.1], o, c).max(to_segment(
        [beta * d2.0, beta * d2.1],
        o,
        c,
    ));
    off.is_finite().then_some(off)
}

/// What the bounds show of a fault.
#[derive(Clone, Debug, PartialEq)]
enum Judged {
    /// Shown: of this kind, with this message.
    Shown(Kind, String),
    /// Nothing can be shown by halving further.
    Clear,
    /// Not shown at these lengths: the stretches to halve (loop, place);
    /// none where halving cannot tell more.
    Unsure(Vec<(usize, usize)>),
}

/// The kinds of fault of the region rule, by their messages.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Clockwise,
    LessThanHole,
    ManyOuter,
    Hole,
    CrossesItself,
    Cross,
}

/// The kind of a message of the region rule ([`super::region_followed`]).
fn kind_of(message: &str) -> Option<Kind> {
    if message.starts_with("every loop runs clockwise") {
        Some(Kind::Clockwise)
    } else if message.starts_with("its counter-clockwise loop") {
        Some(Kind::LessThanHole)
    } else if message.contains("of its loops run counter-clockwise") {
        Some(Kind::ManyOuter)
    } else if message.starts_with("its hole, loop") {
        Some(Kind::Hole)
    } else if message.ends_with("crosses itself") {
        Some(Kind::CrossesItself)
    } else if message.ends_with(" cross") {
        Some(Kind::Cross)
    } else {
        None
    }
}

/// Whether two segments come within `margin` of each other (meeting
/// included).
fn near(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2], margin: f64) -> bool {
    let orient = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| {
        (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])
    };
    let meet = orient(a, b, c) * orient(a, b, d) <= 0.0 && orient(c, d, a) * orient(c, d, b) <= 0.0;
    meet || to_segment(a, c, d) <= margin
        || to_segment(b, c, d) <= margin
        || to_segment(c, a, b) <= margin
        || to_segment(d, a, b) <= margin
}

/// The pairs of segments of `rings` (only of `only`, where given) that
/// cross by more than their margins and ten tolerances (`reach`), the
/// first found; or else the stretches of the pairs that come within those
/// (each whose margin is wider than `reach`, to be halved), and whether a
/// pair that does is no farther apart than ten tolerances with margins
/// within them (halving cannot tell it from touching).
fn crossing(rings: &[Margined], only: Option<usize>, reach: f64) -> Crossing {
    let (picked, margins): (Vec<Ring>, Vec<&Vec<f64>>) = rings
        .iter()
        .enumerate()
        .filter(|(i, _)| only.is_none_or(|o| o == *i))
        .map(|(_, (r, m))| (copy(r), m))
        .unzip();
    let widest = margins
        .iter()
        .flat_map(|m| m.iter())
        .copied()
        .fold(0.0_f64, f64::max);
    let grow = 2.0 * (2.0 * widest + reach);
    let index = |r: usize| only.unwrap_or(r);
    let mut near_ones: Vec<(usize, usize)> = Vec::new();
    let mut stuck = false;
    let margin = |r: usize, i: usize| {
        margins
            .get(r)
            .and_then(|m| m.get(i))
            .copied()
            .unwrap_or(f64::INFINITY)
    };
    let swept = pairs_of_steps(&picked, grow, &mut |(r, i), a, b, (s, j), c, d| {
        let (mi, mj) = (margin(r, i), margin(s, j));
        let m = mi + mj + reach;
        if super::cross(a, b, c, d, m) {
            return true;
        }
        if near(a, b, c, d, m) {
            // The wider of the two halved, and the other where its margin
            // is as wide as half of that.
            let before = near_ones.len();
            if mi > reach && 2.0 * mi >= mj {
                near_ones.push((index(r), i));
            }
            if mj > reach && 2.0 * mj >= mi {
                near_ones.push((index(s), j));
            }
            if near_ones.len() == before {
                stuck = true;
            }
        }
        false
    });
    near_ones.sort_unstable();
    near_ones.dedup();
    match swept {
        Sweep::Found(r, _, s, _) => Crossing::Crossed(index(r.min(s)), index(r.max(s))),
        // Not every pair looked at: nothing told.
        Sweep::Cut => Crossing::Near(Vec::new()),
        Sweep::Done if stuck => Crossing::Near(Vec::new()),
        Sweep::Done if near_ones.is_empty() => Crossing::Apart,
        Sweep::Done => Crossing::Near(near_ones),
    }
}

/// What [`crossing`] found.
#[derive(Clone, Debug, PartialEq)]
enum Crossing {
    /// Two loops (or one and itself) cross.
    Crossed(usize, usize),
    /// No two segments come within their margins.
    Apart,
    /// Some do: these stretches to halve (none: halving cannot tell).
    Near(Vec<(usize, usize)>),
}

/// The stretches of `rings` (of `only`, where given) whose margins are the
/// widest: wider than ten tolerances (`reach`) and than a quarter of the
/// widest.
fn widest_of(rings: &[Margined], only: Option<usize>, reach: f64) -> Vec<(usize, usize)> {
    let picked = |l: usize| only.is_none_or(|o| o == l);
    let widest = rings
        .iter()
        .enumerate()
        .filter(|(l, _)| picked(*l))
        .flat_map(|(_, (_, m))| m.iter())
        .copied()
        .fold(0.0_f64, f64::max);
    let past = reach.max(0.25 * widest);
    let mut out = Vec::new();
    for (l, (_, m)) in rings.iter().enumerate() {
        if !picked(l) {
            continue;
        }
        for (k, margin) in m.iter().enumerate() {
            if *margin > past {
                out.push((l, k));
            }
        }
    }
    out
}

/// The region rule's faults again, with the bounds ([`super::region_followed`]).
fn judge_region(walked: &Walked<'_>, reversed: bool) -> Option<Judged> {
    let Some(rings) = walked.rings()? else {
        // A band: the region rule passes it.
        return Some(Judged::Clear);
    };
    let reach = walked.reach();
    let sense = if reversed { -1.0 } else { 1.0 };
    let mut areas = Vec::with_capacity(rings.len());
    let mut errors = Vec::with_capacity(rings.len());
    for (l, (ring, margins)) in rings.iter().enumerate() {
        let area = signed_area(&ring.points) * sense;
        let error = area_error(&ring.points, margins);
        if !(area.is_finite() && error.is_finite() && area.abs() > error) {
            return Some(Judged::Unsure(widest_of(&rings, Some(l), reach)));
        }
        areas.push(area);
        errors.push(error);
    }
    let list = |areas: &[f64]| super::list(areas);
    let outer: Vec<usize> = (0..areas.len())
        .filter(|i| areas.get(*i).is_some_and(|a| *a > 0.0))
        .collect();
    let others = |o: usize, most: bool| {
        (0..areas.len())
            .filter(|k| *k != o)
            .filter_map(|k| {
                let (a, e) = (areas.get(k)?.abs(), *errors.get(k)?);
                Some(if most { a + e } else { a - e })
            })
            .fold(f64::NEG_INFINITY, f64::max)
    };
    match outer.as_slice() {
        [] if matches!(walked.surface, Surface::Sphere(_)) => {}
        [] => {
            return Some(Judged::Shown(
                Kind::Clockwise,
                format!(
                    "every loop runs clockwise about the outward normal (areas {}): the face would be everything outside them",
                    list(&areas)
                ),
            ));
        }
        [o] => {
            let o = *o;
            let (a, e) = (areas.get(o)?.abs(), *errors.get(o)?);
            if a - e > others(o, true) {
                // The outer loop is the largest.
            } else if a + e < others(o, false) {
                let largest = (0..areas.len()).filter(|k| *k != o).max_by(|x, y| {
                    let (Some(x), Some(y)) = (areas.get(*x), areas.get(*y)) else {
                        return std::cmp::Ordering::Equal;
                    };
                    x.abs().total_cmp(&y.abs())
                })?;
                return Some(Judged::Shown(
                    Kind::LessThanHole,
                    format!(
                        "its counter-clockwise loop {o} encloses less than its hole {largest} (areas {})",
                        list(&areas)
                    ),
                ));
            } else {
                return Some(Judged::Unsure(widest_of(&rings, None, reach)));
            }
        }
        _ => {
            return Some(Judged::Shown(
                Kind::ManyOuter,
                format!(
                    "{} of its loops run counter-clockwise about the outward normal (areas {}): a face has one outer loop",
                    outer.len(),
                    list(&areas)
                ),
            ));
        }
    }
    let mut every_point_told = true;
    let mut rings = rings;
    if let ([o], false) = (
        outer.as_slice(),
        matches!(walked.surface, Surface::Plane(_)),
    ) {
        let o = *o;
        let mut plain: Vec<Ring> = rings.iter().map(|(r, _)| copy(r)).collect();
        shift_holes(&mut plain, o, walked.periods)?;
        for (k, ring) in plain.into_iter().enumerate() {
            if let Some(slot) = rings.get_mut(k) {
                slot.0 = ring;
            }
        }
        let widest: Vec<f64> = rings.iter().map(|(r, _)| r.error).collect();
        let mut work = 0_usize;
        for (k, (ring, _)) in rings.iter().enumerate() {
            if k == o {
                continue;
            }
            for point in &ring.points {
                // The hole's point is a point of its curve: told where it
                // keeps clear of every other loop by that loop's widest
                // margin (and its own, as the walk takes it) and ten
                // tolerances.
                let mut clear = true;
                for (j, (other, _)) in rings.iter().enumerate() {
                    if j == k {
                        continue;
                    }
                    let margin = widest.get(j).copied().unwrap_or(f64::INFINITY)
                        + widest.get(k).copied().unwrap_or(f64::INFINITY)
                        + reach;
                    if other.box_distance(*point) > margin {
                        continue;
                    }
                    work = work.saturating_add(other.points.len());
                    if work > MOST_PAIRS {
                        return None;
                    }
                    if super::distance(*point, &other.points) <= margin {
                        clear = false;
                        break;
                    }
                }
                if !clear {
                    every_point_told = false;
                    continue;
                }
                work = work.saturating_add(rings.iter().map(|(r, _)| r.points.len()).sum());
                if work > MOST_PAIRS {
                    return None;
                }
                let wound: i64 = rings
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| *j != k)
                    .map(|(_, (other, _))| winding(*point, &other.points))
                    .sum::<i64>()
                    * if reversed { -1 } else { 1 };
                if wound != 1 {
                    let each: Vec<String> = rings
                        .iter()
                        .map(|(other, _)| winding(*point, &other.points).to_string())
                        .collect();
                    return Some(Judged::Shown(
                        Kind::Hole,
                        format!(
                            "its hole, loop {k}, lies outside its outer loop {o} or inside another hole (wound {wound} times at ({:.6}, {:.6}) of the {}; by each loop {}; areas {})",
                            point[0],
                            point[1],
                            walked.surface.kind(),
                            each.join(", "),
                            list(&areas),
                        ),
                    ));
                }
            }
        }
    }
    let crossed = crossing(&rings, None, reach);
    if let Crossing::Crossed(i, j) = crossed {
        return Some(Judged::Shown(
            if i == j {
                Kind::CrossesItself
            } else {
                Kind::Cross
            },
            if i == j {
                format!("its loop {i} crosses itself")
            } else {
                format!("its loops {i} and {j} cross")
            },
        ));
    }
    Some(match crossed {
        Crossing::Apart if every_point_told => Judged::Clear,
        Crossing::Apart => Judged::Unsure(widest_of(&rings, None, reach)),
        Crossing::Near(halve) => Judged::Unsure(halve),
        Crossing::Crossed(..) => Judged::Unsure(Vec::new()),
    })
}

/// A copy of a ring.
fn copy(r: &Ring) -> Ring {
    Ring {
        points: r.points.clone(),
        winds: r.winds,
        error: r.error,
        low: r.low,
        high: r.high,
        starts: r.starts.clone(),
        aliased: r.aliased,
    }
}

/// How far a loop's area can be from its polyline's: each segment's
/// stretch of the boundary, with the segment, winds about no point more
/// than once and only within its margin `m` of the segment (length `L`), a
/// region of no more than `2 m (L + m) + π m²`.
fn area_error(points: &[[f64; 2]], margins: &[f64]) -> f64 {
    let n = points.len();
    (0..n)
        .map(|i| {
            let (Some(a), Some(b), Some(m)) =
                (points.get(i), points.get((i + 1) % n), margins.get(i))
            else {
                return f64::INFINITY;
            };
            let length = (b[0] - a[0]).phypot(b[1] - a[1]);
            2.0 * m * (length + m) + PI * m * m
        })
        .sum()
}

/// The region rule's fault `original` of `face`, verified: the message to
/// refuse with, or `None` where the bounds show the loops have none of the
/// region rule's faults (they bound a region: every sense told and as it
/// must be, every point of a hole told inside the outer loop, and no two
/// segments that do not meet at a point of their loop within their
/// margins), so that the walk's fault was its reading of a step. Where the
/// bounds show a fault, the walk's message where it is of the same kind
/// ([`kind_of`]), else the fault shown; where they show neither within the
/// work, the rounds and the stretches allowed, `original` as it is: a fault
/// is only ever dropped where it is shown not to be there. On a surface
/// not [`asked`], `original`.
pub(super) fn region_shown(
    body: &Body,
    face: &Face,
    tol: &Tolerance,
    original: &str,
) -> Option<String> {
    let kept = Some(original.to_string());
    if !asked(&face.surface) {
        return kept;
    }
    let Some(mut walked) = Walked::new(body, face, tol) else {
        return kept;
    };
    for _ in 0..MOST_ROUNDS {
        if walked.show_all().is_none() {
            return kept;
        }
        let judged = judge_region(&walked, face.reversed);
        match judged {
            Some(Judged::Shown(kind, message)) => {
                return Some(if kind_of(original) == Some(kind) {
                    original.to_string()
                } else {
                    message
                });
            }
            Some(Judged::Clear) => return None,
            Some(Judged::Unsure(halve)) if !halve.is_empty() => {
                if walked.halve(&halve).is_none() {
                    return kept;
                }
            }
            Some(Judged::Unsure(_)) | None => return kept,
        }
    }
    kept
}

/// Whether the rule of simple loops' third fault, of the loop `k` of
/// `face` (a point it winds about neither 0 times nor once its own way),
/// stands: false only where the bounds show the loop has no such point (its
/// sense told, and no two of its segments that do not meet at a point of
/// it within their margins, so that it is a simple closed curve as far as
/// stretches that do not meet go); else true, the walk's finding kept. On a
/// surface not [`asked`], true.
pub(crate) fn winding_shown(body: &Body, face: &Face, tol: &Tolerance, k: usize) -> bool {
    if !asked(&face.surface) {
        return true;
    }
    let Some(mut walked) = Walked::new(body, face, tol) else {
        return true;
    };
    for _ in 0..MOST_ROUNDS {
        if walked.show_all().is_none() {
            return true;
        }
        let Some(Some(rings)) = walked.rings() else {
            return true;
        };
        let reach = walked.reach();
        let Some((ring, margins)) = rings.get(k) else {
            return true;
        };
        let area = signed_area(&ring.points);
        let halve = if area.is_finite() && area.abs() > area_error(&ring.points, margins) {
            let margin = 2.0 * ring.error + reach;
            if crate::simple::wound_within(ring, margin, crate::simple::MOST_WINDING_WORK).is_some()
            {
                return true;
            }
            match crossing(&rings, Some(k), reach) {
                Crossing::Crossed(..) => return true,
                Crossing::Apart => return false,
                Crossing::Near(halve) => halve,
            }
        } else {
            widest_of(&rings, Some(k), reach)
        };
        if halve.is_empty() || walked.halve(&halve).is_none() {
            return true;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

    use super::*;

    /// The parallelogram's corners against a curve's points: a quarter
    /// circle from (1, 0) to (0, 1) keeps its direction between (0, 1)
    /// and (-1, 0), and lies within `(√2 − 1)·…` of its chord: the bound is
    /// the corner (1, 1)'s distance from the chord, `1 / √2`, and the
    /// circle's own, `1 − 1 / √2`, is within it.
    #[test]
    fn a_parallelogram_holds_a_quarter_circle() {
        let off = parallelogram((0.0, 1.0), (-1.0, 0.0), (-1.0, 1.0)).unwrap();
        assert!((off - 0.5_f64.sqrt()).abs() < 1e-15, "{off}");
        // A cone nearly a half turn wide: the corners lie far past the
        // chord's ends, and are measured from the segment, not its line.
        let off = parallelogram((1.0, 0.01), (-1.0, 0.01), (0.0, 0.02)).unwrap();
        assert!((off - 1.0).abs() < 1e-12, "{off}");
        assert!(1.0 - 0.5_f64.sqrt() <= off);
        // A chord outside the cone: no bound.
        assert_eq!(parallelogram((0.0, 1.0), (-1.0, 0.0), (1.0, 1.0)), None);
        // A cone of no width, a cone over a half turn: none.
        assert_eq!(parallelogram((1.0, 0.0), (1.0, 0.0), (1.0, 0.0)), None);
        assert_eq!(parallelogram((1.0, 0.0), (0.0, -1.0), (1.0, -1.0)), None);
        assert_eq!(parallelogram((1.0, 0.0), (0.0, 1.0), (0.0, 0.0)), Some(0.0));
    }

    use crate::{BodyKind, Coedge};
    use craft3d_geom::{Cone, Cylinder, NurbsCurve, Plane, Sphere};
    use craft3d_math::Frame;
    use std::f64::consts::TAU;

    fn tilted(k: f64) -> Frame {
        Frame::new(
            Vec3::new(1.3, -2.1, 0.7) * (10.0 * k),
            Vec3::new(-0.3, 0.8, 0.52),
            Vec3::new(0.6, 0.1, 0.4),
        )
        .unwrap()
    }

    fn about(frame: &Frame, rho: f64, angle: f64, h: f64) -> Vec3 {
        let (s, c) = angle.psin_cos();
        frame.point(rho * c, rho * s, h)
    }

    /// A plane, a cylinder, a cone and a sphere of size `k` in `frame`,
    /// each with the map from its chart to it.
    #[allow(clippy::type_complexity)]
    fn grounds(frame: Frame, k: f64) -> Vec<(Surface, Box<dyn Fn(f64, f64) -> Vec3>)> {
        let slope = 0.4_f64.ptan();
        vec![
            (
                Surface::Plane(Plane::new(frame)),
                Box::new(move |x, y| frame.point(k * x, k * y, 0.0)),
            ),
            (
                Surface::Cylinder(Cylinder::new(frame, k).unwrap()),
                Box::new(move |x, y| about(&frame, k, x, k * y)),
            ),
            (
                Surface::Cone(Cone::new(frame, 2.0 * k, 0.4).unwrap()),
                Box::new(move |x, y| about(&frame, 2.0 * k + k * y * slope, x, k * y)),
            ),
            (
                Surface::Sphere(Sphere::new(frame, 2.0 * k).unwrap()),
                Box::new(move |x, y| {
                    let (s, c) = y.psin_cos();
                    about(&frame, 2.0 * k * c, x, 2.0 * k * s)
                }),
            ),
        ]
    }

    /// A closed curve of degree `degree` through `count + 1` points of the
    /// chart's wavy ring `(0.4 + a sin(waves θ)) (cos θ, sin θ) + (0, 0.1)`
    /// taken to the surface; with `rational`, through them with weights
    /// that swing between 0.6 and 1.4.
    fn ring_curve(
        at: &dyn Fn(f64, f64) -> Vec3,
        waves: f64,
        a: f64,
        count: usize,
        degree: usize,
        rational: bool,
    ) -> Option<NurbsCurve> {
        let params: Vec<f64> = (0..=count).map(|i| i as f64 / count as f64).collect();
        let point = |s: f64| {
            let theta = TAU * s;
            let r = 0.4 + a * (waves * theta).psin();
            let (sn, cs) = theta.psin_cos();
            at(r * cs, 0.1 + r * sn)
        };
        let weight = |s: f64| {
            if rational {
                1.0 + 0.4 * (3.0 * TAU * s).psin()
            } else {
                1.0
            }
        };
        let values: Vec<Vec<f64>> = params
            .iter()
            .map(|s| {
                let (p, w) = (point(*s), weight(*s));
                vec![p.x * w, p.y * w, p.z * w, w]
            })
            .collect();
        let (knots, ctrl) = craft3d_geom::refine::interpolate(&params, degree, &values).ok()?;
        let weights: Vec<f64> = ctrl.iter().map(|c| c[3]).collect();
        let points: Vec<Vec3> = ctrl
            .iter()
            .map(|c| Vec3::new(c[0] / c[3], c[1] / c[3], c[2] / c[3]))
            .collect();
        NurbsCurve::new(degree, knots, points, weights).ok()
    }

    /// Every stretch shown holds what it says, against 64 points of its
    /// image placed on the surface: within its bound of its chord, and
    /// running one way (its directions within a half turn). Closed wavy
    /// splines of degrees 2 to 7, through many points or as few as one
    /// span (bulging far from their chords), rational and not, on a plane,
    /// a cylinder, a cone and a sphere, turned and moved, at three scales.
    #[test]
    fn a_stretch_shown_lies_within_its_bound_and_runs_one_way() {
        let (mut shown, mut walked_faces, mut stretches) = (0, 0, 0_usize);
        for k in [1e-3, 1.0, 1e6] {
            for frame in [Frame::WORLD, tilted(k)] {
                for (surface, at) in grounds(frame, k) {
                    for (degree, count, waves, a, rational) in [
                        (3, 96, 7.0, 0.08, false),
                        (5, 64, 5.0, 0.12, true),
                        (7, 7, 3.0, 0.15, false),
                        (7, 7, 2.0, 0.2, true),
                        (2, 40, 9.0, 0.05, false),
                        (4, 12, 3.0, 0.25, false),
                    ] {
                        // A rational fit through few points can have a
                        // weight that is not positive: no such curve.
                        let Some(curve) =
                            ring_curve(at.as_ref(), waves, a, count, degree, rational)
                        else {
                            assert!(rational, "{} degree {degree}", surface.kind());
                            continue;
                        };
                        let mut body = Body::new(BodyKind::Sheet);
                        let v = body.add_vertex(curve.eval(0.0));
                        let edge = body.add_edge(Curve3::Nurbs(curve.clone()), (0.0, 1.0), v, v);
                        let lp = body.add_loop(vec![Coedge {
                            edge,
                            reversed: false,
                        }]);
                        let face = body.add_face(surface.clone(), false, vec![lp]);
                        body.add_shell(vec![face]);
                        // A loose tolerance, so that a curve through few
                        // points of the surface is placed between them too
                        // (the bounds are of the curve's image, on the
                        // surface or not).
                        let tol = Tolerance::new(1e-3 * k, 1e-9).unwrap();
                        let face = &body.faces[0];
                        let Some(mut walked) = Walked::new(&body, face, &tol) else {
                            eprintln!(
                                "not walked: {} at {k}, degree {degree} through {count}",
                                surface.kind()
                            );
                            continue;
                        };
                        let shown_all = walked.show_all();
                        // A point of a curve through few points of a curved
                        // surface may lie off it by more than the tolerance
                        // where a stretch is halved: not walked.
                        if shown_all.is_none() && walked.loops[0].1.is_empty() {
                            continue;
                        }
                        walked_faces += 1;
                        if shown_all.is_none() {
                            eprintln!(
                                "not shown: {} at {k}, degree {degree} through {count}, rational {rational}, work {}",
                                surface.kind(),
                                walked.work
                            );
                            continue;
                        }
                        shown += 1;
                        let kind = surface.kind();
                        // As walked, and halved three times over.
                        for level in 0..4 {
                            if level > 0 {
                                let every: Vec<(usize, usize)> =
                                    (0..walked.loops[0].1.len()).map(|k| (0, k)).collect();
                                if walked.halve(&every).is_none() || walked.show_all().is_none() {
                                    break;
                                }
                            }
                            let all = walked.loops[0].1.clone();
                            for st in &all {
                                let Bound::Shown(bound) = st.bound else {
                                    panic!("{kind}: a stretch not shown after show_all");
                                };
                                stretches += 1;
                                let start = st.start;
                                let end = walked.unwrapped(st.end, Some(start));
                                let mut before = start;
                                let mut far = 0.0_f64;
                                let mut turned: Vec<f64> = Vec::new();
                                for i in 0..=64 {
                                    let t = st.s + (st.e - st.s) * i as f64 / 64.0;
                                    let (u, v) = surface.project(curve.eval(t)).unwrap();
                                    let q = [u, v];
                                    let q = walked.unwrapped(q, Some(before));
                                    far = far.max(to_segment(q, start, end));
                                    let d = (q[0] - before[0], q[1] - before[1]);
                                    if i > 0 && (d.0 != 0.0 || d.1 != 0.0) {
                                        let mut angle = d.1.patan2(d.0);
                                        if let Some(first) = turned.first() {
                                            angle += ((first - angle) / TAU).round() * TAU;
                                        }
                                        turned.push(angle);
                                    }
                                    before = q;
                                }
                                let size = start[0].abs().max(start[1].abs()).max(1.0);
                                assert!(
                                    far <= bound + 1e-12 * size,
                                    "{kind} at {k}, degree {degree} through {count}: {far} off its chord, bound {bound}"
                                );
                                let low = turned.iter().copied().fold(f64::INFINITY, f64::min);
                                let high = turned.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                                assert!(
                                    turned.is_empty() || high - low < PI,
                                    "{kind} at {k}, degree {degree} through {count}: turns {}",
                                    high - low
                                );
                            }
                        }
                    }
                }
            }
        }
        // Every face walked has every stretch shown, the few-span ones
        // included.
        assert_eq!(shown, walked_faces, "of {walked_faces}");
        eprintln!("{walked_faces} faces walked, {stretches} stretches");
        assert!(walked_faces >= 80, "{walked_faces} faces walked");
        assert!(stretches > 10_000, "{stretches} stretches");
    }

    #[test]
    fn faults_are_known_by_their_messages() {
        for (message, kind) in [
            (
                "every loop runs clockwise about the outward normal (areas -1.000e0): the face would be everything outside them",
                Kind::Clockwise,
            ),
            (
                "its counter-clockwise loop 0 encloses less than its hole 1 (areas 1, -2)",
                Kind::LessThanHole,
            ),
            (
                "2 of its loops run counter-clockwise about the outward normal (areas 1, 2): a face has one outer loop",
                Kind::ManyOuter,
            ),
            (
                "its hole, loop 1, lies outside its outer loop 0 or inside another hole (wound 0 times)",
                Kind::Hole,
            ),
            ("its loop 0 crosses itself", Kind::CrossesItself),
            ("its loops 0 and 1 cross", Kind::Cross),
        ] {
            assert_eq!(kind_of(message), Some(kind), "{message}");
        }
        assert_eq!(kind_of("something else"), None);
    }
}
