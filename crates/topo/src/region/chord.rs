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
//! each of which is shown to keep to a bound, and the fault is dropped only
//! where the bounds show it is not there; a face the walk passes is never
//! walked here, so what this adds is only ever a pass where the walk
//! refused, and only one shown.
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
//! meet; (d) two segments that do not meet at a point of their loop and lie
//! farther apart than both margins and ten tolerances hold pieces of the
//! boundary that do not meet; (e) two neighbouring stretches whose cones
//! fit together in a fan narrower than a half turn (by an angle the gap
//! between them asks) meet only within ten tolerances of their shared point
//! ([`neighbours_apart`]). The region rule's faults (a sense, a hole
//! outside, a crossing) and the rule of simple loops' third (a point wound
//! neither 0 times nor once its own way) are claimed again with (a) to (c),
//! and the walk's message is kept where the same kind of fault is shown.
//!
//! **When a fault is dropped, and when it stands.** A fault is dropped only
//! where the bounds show it is not there: every sense told and as a face's
//! must be, every hole point told (but where the rule does not ask it: on a
//! plane, and on a sphere whose loops all run clockwise), no loop a band,
//! and every two pieces of the boundary apart by (d) or (e). Then each loop
//! is a simple closed curve that goes round no period (but for touches
//! within ten tolerances of a vertex, which the rule passes too) and no two
//! loops meet, so neither the faults the region rule asks nor those of the
//! rule of simple loops' second, third and fourth rules within one loop can
//! hold. Where that is not shown, stretches are halved (the widest margins,
//! the stretches of the pairs not shown apart) and the loops judged again;
//! where it is still not shown when halving cannot part a pair (two lines
//! whose cones do not fit, a pair within ten tolerances, a stretch with no
//! aim), a loop reads as a band, a point cannot be placed, or the work of a
//! face ([`super::MOST_PROOF_WORK`] for the whole of its second walk, every
//! read of its loops, pair of segments and point of a hole counted),
//! [`MOST_ROUNDS`] rounds or [`MOST_STRETCHES`] stretches of an edge are
//! spent, the walk's fault stands. Not looked at here, the walk's finding
//! kept as it is: a face on a torus (whose chart is not shown,
//! [`Rise::Unshown`]) or on a spline surface (whose chart's map is not
//! known), and the faults of the other rules (an edge round its circle
//! twice, a loop round a period too often, two stretches on each other).

use super::{
    AboutMap, Fan, Held, Lay, ONE_WAY, Pieces, QUARTER_TURN, Ring, Rise, Sweep, Wedge,
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
/// Pairs looked at between two charges of their work, so that the work of
/// a face is passed by no more than a charge.
const CHARGE: usize = 64;
/// Units in the last place of a chart point's largest coordinate by which
/// it may be off: the curve's point and its place on the surface.
const PLACE_ROUNDING: f64 = 64.0 * f64::EPSILON;

/// While it lives, this thread's work of showing goes to
/// [`super::chord_proofs`], not to [`super::step_proofs`]: the walk again
/// is counted apart from the walk.
struct Verifying(bool);

impl Verifying {
    fn begin() -> Verifying {
        Verifying(super::VERIFYING.with(|v| v.replace(true)))
    }
}

impl Drop for Verifying {
    fn drop(&mut self) {
        let before = self.0;
        super::VERIFYING.with(|v| v.set(before));
    }
}

/// The surfaces whose chart takes a direction by a map that is known
/// ([`Lay`]): a plane, a cylinder, a cone, a sphere.
fn asked(surface: &Surface) -> bool {
    matches!(
        surface,
        Surface::Plane(_) | Surface::Cylinder(_) | Surface::Cone(_) | Surface::Sphere(_)
    )
}

/// A cone of directions in the chart: its two ends, the second
/// anticlockwise of the first, less than a half turn apart.
type Cone = ((f64, f64), (f64, f64));

/// Where a stretch heads in the chart: every move along it, from one of
/// its points to a later one, is a direction of `cone` but for `slack`
/// (the moves over runs of control points that rounding has made one
/// point, each within twice their `clear` of nothing in space).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Aim {
    cone: Cone,
    slack: f64,
}

/// Where a stretch stands.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Bound {
    /// Not looked at yet.
    Unknown,
    /// Shown: the image lies within this of its chord, and where it heads,
    /// in the order of the edge's parameter ([`Aim`]). No aim: the stretch
    /// is a point, its legs do not keep within [`ONE_WAY`] on a plane, or a
    /// leg is too short against what rounding can have moved its ends to
    /// give its direction within a fifth of a radian (a leg of no length
    /// among other points not all one among them).
    Shown(f64, Option<Aim>),
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

/// A loop as the polyline of its stretches' starts ([`Walked::rings`]),
/// with, for each segment (from a stretch's start to the next one's), its
/// margin (the stretch's bound and the gap from the stretch's end to the
/// next start), the cone of the stretch's directions in the order the loop
/// runs, the gap at its end, and whether its edge is a line.
struct Margined {
    ring: Ring,
    margins: Vec<f64>,
    cones: Vec<Option<Aim>>,
    gaps: Vec<f64>,
    straight: Vec<bool>,
}

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
            marked.dedup();
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
                // The cone of the legs' directions along the plane (the
                // chart's), each widened by the angle by which rounding can
                // have turned it: its ends may each be `clear` off, the leg
                // twice that, a turn of at most `asin(2 clear / |leg|)`.
                let Lay::Flat { x, y } = self.lay else {
                    return Some(Bound::Not);
                };
                let mut fan = Fan::within(ONE_WAY);
                let mut turned = 0.0_f64;
                let mut slack = 0.0_f64;
                let mut starts = 0_usize;
                let mut fits = true;
                for &(end, clear) in &held.runs {
                    let run = held.points.get(starts..end).unwrap_or(&[]);
                    starts = end;
                    // Points all one (a piece of no width, where a stretch
                    // ends on a knot): the curve there lies within `clear`
                    // of that point, and moves by twice it at the most.
                    if run.windows(2).all(|w| matches!(w, [p, q] if p == q)) {
                        slack += 2.0 * clear;
                        continue;
                    }
                    for pair in run.windows(2) {
                        let [p, q] = pair else { continue };
                        let w = *q - *p;
                        let t = (w.dot(x), w.dot(y));
                        let length = t.0.phypot(t.1);
                        if length == 0.0 {
                            // A leg of no length among others is none where
                            // the points are exact; else it may point
                            // anywhere.
                            if clear > 0.0 {
                                fits = false;
                            }
                            continue;
                        }
                        let share = 2.0 * clear / length;
                        if share.partial_cmp(&0.2) != Some(std::cmp::Ordering::Less) || !fan.add(t)
                        {
                            fits = false;
                        }
                        turned = turned.max(share.min(1.0).pasin());
                    }
                }
                let cone = match (fits, fan.ends) {
                    (true, Some((low, high))) => Some(Aim {
                        cone: widened((low, high), turned + WIDEN),
                        slack,
                    }),
                    _ => None,
                };
                Some(if bound.is_finite() {
                    Bound::Shown(bound, cone)
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
                let mut aimless = false;
                let mut slack = 0.0_f64;
                'runs: for &(end, clear) in &held.runs {
                    let run = held.points.get(starts..end).unwrap_or(&[]);
                    starts = end;
                    // As on a plane: points all one move by twice their
                    // clear in space at the most (taken to the chart
                    // below).
                    if run.windows(2).all(|w| matches!(w, [p, q] if p == q)) {
                        slack += 2.0 * clear;
                        continue;
                    }
                    for pair in run.windows(2) {
                        let [p, q] = pair else { continue };
                        legs += 1;
                        let w = *q - *p;
                        if w.x == 0.0 && w.y == 0.0 && w.z == 0.0 {
                            // As on a plane: a leg of no length among
                            // others rounded may point anywhere, which the
                            // bound does not ask but the cone does.
                            aimless |= clear > 0.0;
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
                        Bound::Shown(rounded, None)
                    } else {
                        Bound::Not
                    });
                };
                let (su, sv) = map.scale();
                let (l, h) = widened((low, high), WIDEN);
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
                // The cone of the chart's directions: the fan's, its
                // corners widened by what rounding can have moved a leg
                // (above), taken to the chart by the reach's scales.
                let aim = Aim {
                    cone: (d1, d2),
                    slack: slack * (2.0 / near + 1.0),
                };
                Some(if bound.is_finite() {
                    Bound::Shown(bound, (!aimless && aim.slack.is_finite()).then_some(aim))
                } else {
                    Bound::Not
                })
            }
        }
    }

    /// The loops as polylines of their stretches' starts, each point moved
    /// by whole periods to the one before (as the walk does), with each
    /// segment's margin, cone, gap and kind ([`Margined`]). `None` where a
    /// stretch is not shown; `Some(None)` where a loop goes round a period.
    fn rings(&self) -> Option<Option<Vec<Margined>>> {
        let mut out = Vec::with_capacity(self.loops.len());
        for (coedges, stretches) in &self.loops {
            let n = stretches.len();
            let mut points = Vec::with_capacity(n);
            let mut ends = Vec::with_capacity(n);
            let mut bounds = Vec::with_capacity(n);
            let mut cones = Vec::with_capacity(n);
            let mut straight = Vec::with_capacity(n);
            let mut starts = Vec::new();
            let mut before: Option<[f64; 2]> = None;
            let mut coedge = usize::MAX;
            for st in stretches {
                let Bound::Shown(bound, cone) = st.bound else {
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
                // In the order the loop runs: against the edge's parameter
                // the directions are the cone's turned a half turn.
                cones.push(cone.map(|aim| {
                    let ((x0, y0), (x1, y1)) = aim.cone;
                    if st.s <= st.e {
                        aim
                    } else {
                        Aim {
                            cone: ((-x0, -y0), (-x1, -y1)),
                            ..aim
                        }
                    }
                }));
                let edge = coedges.get(st.coedge).and_then(|c| self.body.edge(c.edge));
                straight.push(edge.is_some_and(|e| matches!(e.curve, Curve3::Line(_))));
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
            let gaps: Vec<f64> = (0..n)
                .map(|i| {
                    let (Some(b), Some(next)) = (ends.get(i), points.get((i + 1) % n)) else {
                        return f64::INFINITY;
                    };
                    (b[0] - next[0]).phypot(b[1] - next[1])
                })
                .collect();
            let margins: Vec<f64> = (0..n)
                .map(|i| match (bounds.get(i), gaps.get(i)) {
                    (Some(bound), Some(gap)) => bound + gap,
                    _ => f64::INFINITY,
                })
                .collect();
            let mut low = [f64::INFINITY; 2];
            let mut high = [f64::NEG_INFINITY; 2];
            for p in &points {
                low = [low[0].min(p[0]), low[1].min(p[1])];
                high = [high[0].max(p[0]), high[1].max(p[1])];
            }
            let error = margins.iter().copied().fold(0.0_f64, f64::max);
            out.push(Margined {
                ring: Ring {
                    points,
                    winds: [0.0; 2],
                    error,
                    low,
                    high,
                    starts,
                    aliased: false,
                },
                margins,
                cones,
                gaps,
                straight,
            });
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

/// A cone's two ends turned outward by `by` (radians): the low end
/// clockwise, the high one anticlockwise.
fn widened((low, high): Cone, by: f64) -> Cone {
    let turn = |(x, y): (f64, f64), by: f64| {
        let (s, c) = by.psin_cos();
        (x * c - y * s, x * s + y * c)
    };
    (turn(low, -by), turn(high, by))
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
    /// The rule of simple loops' third: a point wound wrongly.
    Wound,
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

/// What [`crossing`] found of the pairs of segments that do not meet at a
/// point of their loop.
#[derive(Clone, Debug, PartialEq)]
enum Crossing {
    /// Two loops (or one and itself) cross.
    Crossed(usize, usize),
    /// Every such pair was looked at, and none comes within its margins.
    Apart,
    /// Some do: these stretches to halve (none: halving cannot tell, or not
    /// every pair was looked at).
    Near(Vec<(usize, usize)>),
}

/// The pairs of segments of `rings` (only of `only`, where given) that do
/// not meet at a point of their loop: the first that cross by more than
/// their margins and ten tolerances (`reach`), shown by the band lemma of
/// the module; or else the stretches of the pairs that come within those
/// (the wider of each, and the other where its margin is half as wide or
/// more, each wider than `reach`), none where a pair that does has both
/// margins within `reach` (halving cannot tell it from touching). Each pair
/// looked at counts half a unit of `work`; `None` where the work is spent.
fn crossing(
    rings: &[Margined],
    only: Option<usize>,
    reach: f64,
    work: &mut usize,
) -> Option<Crossing> {
    let picked: Vec<&Margined> = rings
        .iter()
        .enumerate()
        .filter(|(i, _)| only.is_none_or(|o| o == *i))
        .map(|(_, m)| m)
        .collect();
    let plain: Vec<Ring> = picked.iter().map(|m| copy(&m.ring)).collect();
    let widest = picked
        .iter()
        .flat_map(|m| m.margins.iter())
        .copied()
        .fold(0.0_f64, f64::max);
    let grow = 2.0 * (2.0 * widest + reach);
    let index = |r: usize| only.unwrap_or(r);
    let mut near_ones: Vec<(usize, usize)> = Vec::new();
    let mut stuck = false;
    let mut visits = 0_usize;
    let margin = |r: usize, i: usize| {
        picked
            .get(r)
            .and_then(|m| m.margins.get(i))
            .copied()
            .unwrap_or(f64::INFINITY)
    };
    let mut spent = false;
    let swept = pairs_of_steps(&plain, grow, true, &mut |(r, i), a, b, (s, j), c, d| {
        // Counted as it goes, so that the work is not passed by more than
        // a charge.
        visits += 1;
        if visits.is_multiple_of(CHARGE) && spend(work, CHARGE / 2).is_none() {
            spent = true;
            return true;
        }
        let (mi, mj) = (margin(r, i), margin(s, j));
        let m = mi + mj + reach;
        if super::cross(a, b, c, d, m) {
            return true;
        }
        if near(a, b, c, d, m) {
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
    if spent {
        return None;
    }
    spend(work, (visits % CHARGE).div_ceil(2))?;
    near_ones.sort_unstable();
    near_ones.dedup();
    Some(match swept {
        Sweep::Found(r, _, s, _) => Crossing::Crossed(index(r.min(s)), index(r.max(s))),
        // Not every pair looked at (past `MOST_PAIRS`): nothing told.
        Sweep::Cut => Crossing::Near(Vec::new()),
        Sweep::Done if stuck => Crossing::Near(Vec::new()),
        Sweep::Done if near_ones.is_empty() => Crossing::Apart,
        Sweep::Done => Crossing::Near(near_ones),
    })
}

/// Whether two neighbouring stretches, the first `a` ending where the
/// second `b` starts but for a gap `gap` (two edges meeting at a vertex
/// within its tolerance; nothing inside an edge), are shown to meet only
/// within ten tolerances (`reach`) of that point: where the cones of their
/// aims ([`Aim`]), in the order the loop runs, both fit in a fan narrower
/// than a half turn by `θ = 2 asin(2 g / reach)`, `g` the gap and both
/// slacks. *Proof*: then the fan's middle `n` has `n·d ≥ sin(θ/2) |d|` for
/// every direction `d` of either cone; a point `x` of the first stretch is
/// `E − a − e`, `E` its end, `a` in its cone and `|e|` within its slack,
/// and one of the second `P + b + f` alike, `P` its start; where they meet,
/// `n·(E − P) − n·(e + f) = n·a + n·b ≥ sin(θ/2) (|a| + |b|)`, so `|a| + |b|
/// ≤ g / sin(θ/2) = reach / 2`, and `x` lies within half the reach and the
/// slack, `g` being under a quarter of the reach, so within the reach, of
/// `E`. With no gap and no slack they meet only at the point they share.
/// Not shown (false): a stretch with no aim, `g` of a quarter of the reach
/// or more, cones that do not fit.
fn neighbours_apart(a: Option<Aim>, b: Option<Aim>, gap: f64, reach: f64) -> bool {
    let (Some(a), Some(b)) = (a, b) else {
        return false;
    };
    let g = gap + a.slack + b.slack;
    if !(g >= 0.0 && reach > 0.0 && g < 0.25 * reach) {
        return false;
    }
    let theta = 2.0 * (2.0 * g / reach).pasin();
    let mut fan = Fan::within(PI - theta.max(WIDEN));
    let (ca, cb) = (a.cone, b.cone);
    [ca.0, ca.1, cb.0, cb.1].into_iter().all(|t| fan.add(t))
}

/// The neighbouring stretches of `rings` (of `only`, where given) not shown
/// apart ([`neighbours_apart`]), the two of each pair that are not both
/// lines (a line's cone does not narrow when it is halved); and whether
/// some pair not shown apart is two lines (halving cannot part them). Each
/// pair counts a quarter unit of `work`; `None` where the work is spent.
fn neighbours(
    rings: &[Margined],
    only: Option<usize>,
    reach: f64,
    work: &mut usize,
) -> Option<(Vec<(usize, usize)>, bool)> {
    let mut halve = Vec::new();
    let mut stuck = false;
    let mut pairs = 0_usize;
    for (l, m) in rings.iter().enumerate() {
        if only.is_some_and(|o| o != l) {
            continue;
        }
        let n = m.cones.len();
        for i in 0..n {
            pairs += 1;
            if pairs.is_multiple_of(CHARGE) {
                spend(work, CHARGE / 4)?;
            }
            let j = (i + 1) % n;
            let (Some(a), Some(b), Some(gap)) = (m.cones.get(i), m.cones.get(j), m.gaps.get(i))
            else {
                stuck = true;
                continue;
            };
            if neighbours_apart(*a, *b, *gap, reach) {
                continue;
            }
            let line = |k: usize| m.straight.get(k).copied().unwrap_or(false);
            if line(i) && line(j) {
                stuck = true;
                continue;
            }
            if !line(i) {
                halve.push((l, i));
            }
            if !line(j) {
                halve.push((l, j));
            }
        }
    }
    spend(work, (pairs % CHARGE).div_ceil(4))?;
    halve.sort_unstable();
    halve.dedup();
    Some((halve, stuck))
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
        .flat_map(|(_, m)| m.margins.iter())
        .copied()
        .fold(0.0_f64, f64::max);
    let past = reach.max(0.25 * widest);
    let mut out = Vec::new();
    for (l, m) in rings.iter().enumerate() {
        if !picked(l) {
            continue;
        }
        for (k, margin) in m.margins.iter().enumerate() {
            if *margin > past {
                out.push((l, k));
            }
        }
    }
    out
}

/// What the bounds show of the loops of `walked` (of the loop `only`,
/// where given) before their senses and holes are judged: every loop's
/// polyline with its margins ([`Walked::rings`]), and its pairs of pieces
/// ([`crossing`], [`neighbours`]). The rest is in [`judge_region`] and
/// [`judge_simple`].
enum Parts {
    /// The loops; what is shown of their pairs that do not meet at a point
    /// of their loop; the stretches to halve (none: halving cannot part
    /// some pair, or there is none to halve); and whether every pair,
    /// neighbouring or not, is shown apart.
    Read(Vec<Margined>, Crossing, Vec<(usize, usize)>, bool),
    /// A loop goes round a period in the chart: not judged here, and so
    /// never clear (the walk's fault stands).
    Band,
}

/// [`Parts`] of `walked`; `None` where a stretch is not shown or the work is
/// spent.
fn parts(walked: &mut Walked<'_>, only: Option<usize>) -> Option<Parts> {
    let Some(rings) = walked.rings()? else {
        return Some(Parts::Band);
    };
    let segments: usize = rings.iter().map(|m| m.margins.len()).sum();
    spend(&mut walked.work, segments.div_ceil(4))?;
    let reach = walked.reach();
    let crossed = crossing(&rings, only, reach, &mut walked.work)?;
    let (mut halve, stuck) = neighbours(&rings, only, reach, &mut walked.work)?;
    let apart = matches!(crossed, Crossing::Apart) && halve.is_empty() && !stuck;
    if let Crossing::Near(more) = &crossed {
        halve.extend(more.iter().copied());
    }
    if stuck || matches!(&crossed, Crossing::Near(h) if h.is_empty()) {
        // Some pair cannot be parted by halving: nothing more can be shown
        // apart.
        halve.clear();
    }
    halve.sort_unstable();
    halve.dedup();
    Some(Parts::Read(rings, crossed, halve, apart))
}

/// The region rule's faults again, with the bounds ([`super::region_followed`]).
fn judge_region(walked: &mut Walked<'_>, reversed: bool) -> Option<Judged> {
    let Parts::Read(mut rings, crossed, halve_pairs, apart) = parts(walked, None)? else {
        return Some(Judged::Unsure(Vec::new()));
    };
    let reach = walked.reach();
    let sense = if reversed { -1.0 } else { 1.0 };
    let mut areas = Vec::with_capacity(rings.len());
    let mut errors = Vec::with_capacity(rings.len());
    for (l, m) in rings.iter().enumerate() {
        // (a) of the module: the loop's area is its polyline's within this.
        let area = signed_area(&m.ring.points) * sense;
        let error = area_error(&m.ring.points, &m.margins);
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
        // Shown: every sense is told, and a sphere less some caps is a
        // region.
        [] if matches!(walked.surface, Surface::Sphere(_)) => {}
        // Shown: every sense is told (above), and none runs
        // counter-clockwise.
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
                // Shown: the outer loop encloses more than any hole, for
                // every area within the errors.
            } else if a + e < others(o, false) {
                let largest = (0..areas.len()).filter(|k| *k != o).max_by(|x, y| {
                    let (Some(x), Some(y)) = (areas.get(*x), areas.get(*y)) else {
                        return std::cmp::Ordering::Equal;
                    };
                    x.abs().total_cmp(&y.abs())
                })?;
                // Shown: some hole encloses more than the outer loop, for
                // every area within the errors.
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
        // Shown: every sense is told, and two run counter-clockwise.
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
    if let ([o], false) = (
        outer.as_slice(),
        matches!(walked.surface, Surface::Plane(_)),
    ) {
        let o = *o;
        let mut plain: Vec<Ring> = rings.iter().map(|m| copy(&m.ring)).collect();
        shift_holes(&mut plain, o, walked.periods)?;
        for (k, ring) in plain.into_iter().enumerate() {
            if let Some(slot) = rings.get_mut(k) {
                slot.ring = ring;
            }
        }
        let widest: Vec<f64> = rings.iter().map(|m| m.ring.error).collect();
        for (k, m) in rings.iter().enumerate() {
            if k == o {
                continue;
            }
            for point in &m.ring.points {
                // (b) of the module: a point of the hole's own curve, clear
                // of every other loop by its widest margin and ten
                // tolerances, is wound by each as by its polyline.
                let mut clear = true;
                for (j, other) in rings.iter().enumerate() {
                    if j == k {
                        continue;
                    }
                    let margin = widest.get(j).copied().unwrap_or(f64::INFINITY)
                        + widest.get(k).copied().unwrap_or(f64::INFINITY)
                        + reach;
                    if other.ring.box_distance(*point) > margin {
                        continue;
                    }
                    spend(&mut walked.work, other.ring.points.len().div_ceil(2))?;
                    if super::distance(*point, &other.ring.points) <= margin {
                        clear = false;
                        break;
                    }
                }
                if !clear {
                    every_point_told = false;
                    continue;
                }
                let around: usize = rings.iter().map(|m| m.ring.points.len()).sum();
                spend(&mut walked.work, around.div_ceil(2))?;
                let wound: i64 = rings
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| *j != k)
                    .map(|(_, other)| winding(*point, &other.ring.points))
                    .sum::<i64>()
                    * if reversed { -1 } else { 1 };
                if wound != 1 {
                    let each: Vec<String> = rings
                        .iter()
                        .map(|other| winding(*point, &other.ring.points).to_string())
                        .collect();
                    // Shown: a point of the hole is wound other than once.
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
    if let Crossing::Crossed(i, j) = crossed {
        // Shown: (c) of the module, two pieces of the boundary cross.
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
    if apart && every_point_told {
        // Clear, shown: (a) every loop's sense is told and the senses are a
        // face's (one outer loop enclosing more than any hole, for every
        // area within the errors; or, on a sphere, every loop clockwise);
        // (b) with an outer loop, on every surface but a plane, every point
        // of every hole is told wound once by the others (where the rule does
        // not ask where a hole lies, on a plane or on a sphere whose loops
        // all run clockwise, nor is it shown here); (c) no two pieces of the
        // boundary meet beyond ten tolerances of a vertex: pieces whose
        // segments do not meet at a point of their loop lie within their
        // margins of segments farther apart than both margins and ten
        // tolerances ((d) of the module), and neighbouring pieces are shown
        // apart by their cones ((e), [`neighbours_apart`]); and no loop
        // reads as a band ([`parts`]). So each loop is a simple closed curve
        // that goes round no period (but for touches within ten tolerances
        // of its vertices, which the rule passes too), no two loops meet,
        // and none of the faults the region rule asks holds.
        return Some(Judged::Clear);
    }
    // Not shown either way: the stretches of the pairs not shown apart,
    // or, where those are all told and a hole's point is not, the widest.
    Some(Judged::Unsure(if halve_pairs.is_empty() && apart {
        widest_of(&rings, None, reach)
    } else {
        halve_pairs
    }))
}

/// The rule of simple loops' third rule of the loop `k` again, with the
/// bounds: shown (a point clear of the loop by its widest margin, twice, and
/// ten tolerances, wound by its polyline neither 0 times nor once its own
/// way, its sense told: (a) and (b) of the module), clear (its sense told,
/// and every pair of its pieces apart: a simple closed curve, which winds
/// once about every point inside it and 0 times outside), or neither.
fn judge_simple(walked: &mut Walked<'_>, k: usize) -> Option<Judged> {
    let Parts::Read(rings, crossed, halve, apart) = parts(walked, Some(k))? else {
        return Some(Judged::Unsure(Vec::new()));
    };
    let reach = walked.reach();
    let Some(m) = rings.get(k) else {
        return Some(Judged::Unsure(Vec::new()));
    };
    let area = signed_area(&m.ring.points);
    if !(area.is_finite() && area.abs() > area_error(&m.ring.points, &m.margins)) {
        return Some(Judged::Unsure(widest_of(&rings, Some(k), reach)));
    }
    let margin = 2.0 * m.ring.error + reach;
    spend(&mut walked.work, m.ring.points.len())?;
    if let Some((at, wound)) =
        crate::simple::wound_within(&m.ring, margin, crate::simple::MOST_WINDING_WORK)
    {
        return Some(Judged::Shown(
            Kind::Wound,
            format!(
                "it winds {wound} times about the point ({:.6}, {:.6})",
                at[0], at[1]
            ),
        ));
    }
    if let Crossing::Crossed(..) = crossed {
        // Shown: two pieces of the loop cross, so it is no simple closed
        // curve; rule 3 itself is not shown, and the walk's finding stands.
        return Some(Judged::Shown(Kind::CrossesItself, String::new()));
    }
    if apart {
        // Clear, shown: the loop's sense is told and every pair of its
        // pieces is apart (as in [`judge_region`]), so it is a simple
        // closed curve, which winds about no point other than once its own
        // way or not at all.
        return Some(Judged::Clear);
    }
    Some(Judged::Unsure(halve))
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

/// The second walk of one face, made when its first fault is to be
/// verified and kept for every other fault of the face: one
/// [`super::MOST_PROOF_WORK`] of work and [`MOST_ROUNDS`] rounds of halving
/// for the whole face, every read of its loops, pair of segments and point
/// of a hole counted (in [`super::chord_proofs`], apart from the walk's
/// own; a face whose second walk passes the bound is counted there once).
/// Where any step of it cannot be taken (a point not placed, a stretch not
/// shown within the stretches of an edge, the work or the rounds spent),
/// the walk's faults stand, for this one and every later fault of the face.
pub(super) struct Verifier<'a> {
    body: &'a Body,
    face: &'a Face,
    tol: &'a Tolerance,
    walked: Option<Walked<'a>>,
    built: bool,
    rounds: usize,
}

impl<'a> Verifier<'a> {
    pub(super) fn new(body: &'a Body, face: &'a Face, tol: &'a Tolerance) -> Self {
        Verifier {
            body,
            face,
            tol,
            walked: None,
            built: false,
            rounds: 0,
        }
    }

    /// Rounds of `judge`, halving what it asks between them, until it says
    /// shown, clear, or that halving cannot tell more; `None` where a step
    /// cannot be taken (and nothing more is asked of this face).
    fn settle(&mut self, judge: &dyn Fn(&mut Walked<'a>) -> Option<Judged>) -> Option<Judged> {
        let _counted = Verifying::begin();
        if !self.built {
            self.built = true;
            self.walked = Walked::new(self.body, self.face, self.tol);
        }
        let mut walked = self.walked.take()?;
        while self.rounds < MOST_ROUNDS {
            self.rounds += 1;
            walked.show_all()?;
            let judged = judge(&mut walked);
            match judged? {
                Judged::Unsure(halve) if !halve.is_empty() => walked.halve(&halve)?,
                other => {
                    self.walked = Some(walked);
                    return Some(other);
                }
            }
        }
        None
    }

    /// The region rule's fault `original` of the face, verified: the
    /// message to refuse with, or `None` where the bounds show the loops
    /// bound a region ([`judge_region`]'s clear: a statement shown, step by
    /// step), so that the walk's fault was its reading of a step. Where the
    /// bounds show a fault, the walk's message where it is of the same kind
    /// ([`kind_of`]), else the fault shown. Where they show neither (halving
    /// cannot part two stretches, a loop reads as a band, the work, the
    /// rounds or the stretches of an edge are spent, a point is not placed)
    /// the walk's fault stands: a fault is dropped only where it is shown
    /// not to be there. On a surface not [`asked`], `original`.
    pub(super) fn region_shown(&mut self, original: &str) -> Option<String> {
        let kept = Some(original.to_string());
        if !asked(&self.face.surface) {
            return kept;
        }
        let reversed = self.face.reversed;
        match self.settle(&|w| judge_region(w, reversed)) {
            Some(Judged::Shown(kind, message)) => Some(if kind_of(original) == Some(kind) {
                original.to_string()
            } else {
                message
            }),
            Some(Judged::Clear) => None,
            Some(Judged::Unsure(_)) | None => kept,
        }
    }

    /// Whether the rule of simple loops' third fault of the loop `k` (a
    /// point it winds about neither 0 times nor once its own way) stands:
    /// false only where the bounds show the loop is a simple closed curve
    /// ([`judge_simple`]'s clear), else true, the walk's finding kept. On a
    /// surface not [`asked`], true.
    pub(crate) fn winding_shown(&mut self, k: usize) -> bool {
        if !asked(&self.face.surface) {
            return true;
        }
        !matches!(self.settle(&|w| judge_simple(w, k)), Some(Judged::Clear))
    }
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

    /// Neighbouring stretches are shown apart only where their cones fit
    /// in a fan narrower than a half turn by what the gap and the slacks
    /// ask: a corner and a tangent join are; a hairpin, a stretch with no
    /// aim, a gap or slack of a quarter of the reach are not. And the claim
    /// itself, sampled: two rays from either end of a gap, each in its
    /// cone, meet (where they do) within the reach of the first's end.
    #[test]
    fn neighbours_are_apart_only_where_their_cones_fit() {
        let aim = |a: f64, b: f64, slack: f64| {
            let (sa, ca) = a.psin_cos();
            let (sb, cb) = b.psin_cos();
            Some(Aim {
                cone: ((ca, sa), (cb, sb)),
                slack,
            })
        };
        let reach = 1e-5;
        let deg = PI / 180.0;
        let east = aim(-deg, deg, 0.0);
        // A corner, a tangent join, a sharp corner: apart with no gap.
        assert!(neighbours_apart(
            east,
            aim(89.0 * deg, 91.0 * deg, 0.0),
            0.0,
            reach
        ));
        assert!(neighbours_apart(east, east, 0.0, reach));
        assert!(neighbours_apart(
            east,
            aim(170.0 * deg, 178.0 * deg, 0.0),
            0.0,
            reach
        ));
        // A hairpin: the second heads back over the first.
        assert!(!neighbours_apart(
            east,
            aim(179.5 * deg, 181.0 * deg, 0.0),
            0.0,
            reach
        ));
        // With a gap of an eighth of the reach the fan must be narrower by
        // 2 asin(1/4), 29 degrees: a corner fits, a sharp one does not.
        let gap = reach / 8.0;
        assert!(neighbours_apart(
            east,
            aim(89.0 * deg, 91.0 * deg, 0.0),
            gap,
            reach
        ));
        assert!(!neighbours_apart(
            east,
            aim(150.0 * deg, 160.0 * deg, 0.0),
            gap,
            reach
        ));
        // A slack counts as the gap does.
        assert!(!neighbours_apart(
            east,
            aim(150.0 * deg, 160.0 * deg, gap),
            0.0,
            reach
        ));
        // A gap or slack of a quarter of the reach or more, no aim, a gap
        // not a number: never apart.
        assert!(!neighbours_apart(east, east, 0.25 * reach, reach));
        assert!(!neighbours_apart(
            east,
            aim(0.0, 0.0, 0.25 * reach),
            0.0,
            reach
        ));
        assert!(!neighbours_apart(None, east, 0.0, reach));
        assert!(!neighbours_apart(east, east, f64::NAN, reach));
        // Sampled: the first stretch runs into `E` along a direction of its
        // cone, the second out of `P = E + gap` along one of its own; where
        // two such straight stretches cross, they do within the reach of E.
        let (mut checked, mut met) = (0, 0);
        for i in 0..40 {
            let angle_a = -3.0 + 0.15 * f64::from(i);
            for j in 0..40 {
                let angle_b = -3.0 + 0.15 * f64::from(j);
                for g in [0.0, 0.05, 0.2] {
                    for turn in 0..8 {
                        let gap_dir = f64::from(turn) * PI / 4.0;
                        let (sg, cg) = gap_dir.psin_cos();
                        let gap = g * reach;
                        let a = aim(angle_a, angle_a, 0.0);
                        let b = aim(angle_b, angle_b, 0.0);
                        if !neighbours_apart(a, b, gap, reach) {
                            continue;
                        }
                        checked += 1;
                        // E at the origin, P at the gap; the first comes in
                        // from E − 1·da, the second goes to P + 1·db.
                        let (sa, ca) = angle_a.psin_cos();
                        let (sb, cb) = angle_b.psin_cos();
                        let (p, q) = ((-ca, -sa), (0.0, 0.0));
                        let (r, t) = ((gap * cg, gap * sg), (gap * cg + cb, gap * sg + sb));
                        // Where the segments p q and r t meet, if they do.
                        let d1 = (q.0 - p.0, q.1 - p.1);
                        let d2 = (t.0 - r.0, t.1 - r.1);
                        let den = d1.0 * d2.1 - d1.1 * d2.0;
                        if den.abs() < 1e-12 {
                            continue;
                        }
                        let w = (r.0 - p.0, r.1 - p.1);
                        let u = (w.0 * d2.1 - w.1 * d2.0) / den;
                        let v = (w.0 * d1.1 - w.1 * d1.0) / den;
                        if (0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v) {
                            let x = (p.0 + u * d1.0, p.1 + u * d1.1);
                            met += usize::from(gap > 0.0);
                            assert!(
                                x.0.phypot(x.1) <= reach * (1.0 + 1e-9),
                                "angles {angle_a}, {angle_b}, gap {gap}: they meet {} from E",
                                x.0.phypot(x.1)
                            );
                        }
                    }
                }
            }
        }
        assert!(
            checked > 1000 && met > 10,
            "{checked} pairs apart, {met} meeting"
        );
    }

    /// A band of a cylinder (two circles round it) is not judged by the
    /// bounds: a fault of its region rule stands, whatever it says.
    #[test]
    fn a_band_is_never_cleared() {
        let frame = Frame::WORLD;
        let cylinder = Surface::Cylinder(Cylinder::new(frame, 2.0).unwrap());
        let mut body = Body::new(BodyKind::Sheet);
        let mut loops = Vec::new();
        for (z, reversed) in [(0.0, false), (3.0, true)] {
            let circle = craft3d_geom::Circle::new(
                Frame::new(Vec3::new(0.0, 0.0, z), Vec3::Z, Vec3::X).unwrap(),
                2.0,
            )
            .unwrap();
            let v = body.add_vertex(Vec3::new(2.0, 0.0, z));
            let edge = body.add_edge(Curve3::Circle(circle), (0.0, TAU), v, v);
            loops.push(body.add_loop(vec![Coedge { edge, reversed }]));
        }
        let face = body.add_face(cylinder, false, loops);
        body.add_shell(vec![face]);
        let tol = Tolerance::new(1e-6, 1e-9).unwrap();
        let face = &body.faces[0];
        let mut walked = Walked::new(&body, face, &tol).unwrap();
        walked.show_all().unwrap();
        assert!(matches!(walked.rings(), Some(None)), "read as a band");
        let original = "its loops 0 and 1 cross";
        let mut verifier = Verifier::new(&body, face, &tol);
        assert_eq!(verifier.region_shown(original).as_deref(), Some(original));
        assert!(verifier.winding_shown(0));
    }

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
        let mut coned = 0_usize;
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
                                let Bound::Shown(bound, _) = st.bound else {
                                    panic!("{kind}: a stretch not shown after show_all");
                                };
                                stretches += 1;
                                let start = st.start;
                                let end = walked.unwrapped(st.end, Some(start));
                                let mut before = start;
                                let mut far = 0.0_f64;
                                let mut turned: Vec<f64> = Vec::new();
                                let mut secants: Vec<(f64, f64)> = Vec::new();
                                for i in 0..=64 {
                                    let t = st.s + (st.e - st.s) * i as f64 / 64.0;
                                    let (u, v) = surface.project(curve.eval(t)).unwrap();
                                    let q = [u, v];
                                    let q = walked.unwrapped(q, Some(before));
                                    far = far.max(to_segment(q, start, end));
                                    let d = (q[0] - before[0], q[1] - before[1]);
                                    if i > 0 && (d.0 != 0.0 || d.1 != 0.0) {
                                        secants.push(d);
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
                                // Every secant, an average of the curve's
                                // directions over its stretch, lies in the
                                // cone (to the rounding of its ends).
                                if let Bound::Shown(
                                    _,
                                    Some(Aim {
                                        cone: (c0, c1),
                                        slack,
                                    }),
                                ) = st.bound
                                {
                                    coned += 1;
                                    let unit = |c: (f64, f64)| {
                                        let n = c.0.phypot(c.1);
                                        (c.0 / n, c.1 / n)
                                    };
                                    let (c0, c1) = (unit(c0), unit(c1));
                                    assert!(
                                        c0.0 * c1.1 - c0.1 * c1.0 > -1e-12
                                            || c0.0 * c1.0 + c0.1 * c1.1 > 0.0,
                                        "{kind}: a cone of a half turn or more"
                                    );
                                    for d in &secants {
                                        let slack = 1e-9 * d.0.phypot(d.1) + 1e-12 * size + slack;
                                        assert!(
                                            c0.0 * d.1 - c0.1 * d.0 >= -slack
                                                && d.0 * c1.1 - d.1 * c1.0 >= -slack,
                                            "{kind} at {k}, degree {degree} through {count}: secant {d:?} outside the cone {c0:?} to {c1:?}"
                                        );
                                    }
                                }
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
        eprintln!("{walked_faces} faces walked, {stretches} stretches, {coned} with a cone");
        assert!(walked_faces >= 80, "{walked_faces} faces walked");
        assert!(stretches > 10_000, "{stretches} stretches");
        assert!(
            coned * 10 > stretches * 9,
            "{coned} of {stretches} with a cone"
        );
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
