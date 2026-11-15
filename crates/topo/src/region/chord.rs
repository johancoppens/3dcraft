//! Verify before refusing, on a plane (since 2026-11-15;
//! `docs/design/topology.md` › *What lies between an edge and its chord*,
//! which has the theorem below with its proof in full).
//!
//! The walk of the loop rules ([`super::follow`]) reads each step of an
//! edge at its two ends and its middle and takes the chord of the step for
//! the edge; how far the edge lies from that chord is measured at the
//! step's middle, never bounded. So the walk can see a fault that is not
//! there (a wavy side whose steps end, and are measured, where it crosses
//! its middle line reads as straight, and its chords cross the other side).
//! Here a face on a plane that the walk is about to refuse is walked again
//! in stretches, and the walk's fault is dropped only where the whole face
//! is *cleared*: where the conditions C0 to C6 below, each computed, all
//! hold. A face the walk passes is never walked here, and a face on any
//! other surface is not walked again at all ([`asked`]): its fault stands
//! as the walk found it.
//!
//! **The setting.** The chart of a plane is `(u, v) = ((p − o)·x, (p −
//! o)·y)`, `o`, `x`, `y` its frame's origin and axes: an affine map, with
//! no period and no pole, one to one on the plane, and an isometry of it
//! where `x` and `y` are unit vectors square to each other, which
//! [`flat`] checks to 1e-9: lengths in the chart are the model's, and the
//! *reach* `R` is ten of the body's working tolerances in both. An edge's
//! *image* is its curve over its range taken through the chart, also where
//! the curve lies off the plane. A *stretch* is a part of one coedge's
//! edge between two parameters; the stretches of a loop, in the order the
//! loop runs, take every coedge's whole range ([`Walked::new`],
//! [`Walked::halve`]). A stretch `i` runs from `Aᵢ` to `Bᵢ` (both placed,
//! [`Walked::place_at`]); the *gap* `gᵢ = |Bᵢ − Aᵢ₊₁|` is nothing inside
//! an edge and what two edges' ends miss each other by at a vertex; the
//! loop's *curve* `Γ` is its stretches' images in order, each joined to the
//! next by the straight segment across its gap; its *polyline* `Π` is `A₀
//! A₁ … Aₙ₋₁`, its segment `Sᵢ = Aᵢ Aᵢ₊₁`; the *piece* `πᵢ` is the image
//! of stretch `i` with the segment across its gap.
//!
//! **What is shown of a stretch** ([`Walked::bound_of`],
//! [`Walked::hull_bound`]). Points in runs are made for it (a line's two
//! ends; the three points of the rational quadratic of an arc of a circle
//! or an ellipse of at most a quarter turn; the control points of a
//! spline's Bézier pieces cut to the stretch, a run each, or over more than
//! [`super::MANY_SPANS`] spans the control points of the spans it meets,
//! one run) such that (H) the curve over each run's part of the stretch
//! lies within the run's *clear* (what rounding can have moved the points,
//! [`super::hull_rounding`] times [`super::HULL_CLEAR`], [`rounding_of`])
//! of the convex hull of the run's points, and (D) the curve's direction
//! at each of its points is a sum, with weights that are not negative, of
//! the legs from each point of its run to the next (a spline's with
//! positive weights by the total positivity of its basis,
//! [`super::one_way`]); a spline is held to being one curve over the
//! stretch, its two ends included: each of its pieces starting where the
//! one before ends ([`Walked::hull_bound`]) and, where the spans' control
//! points are read, no knot of theirs inside the domain repeated more often
//! than the degree ([`jumps`]).
//! The chart being affine, the image lies in the hulls of the points'
//! images and its direction is such a sum of the legs' images. Then:
//!
//! - (S1) *the bound* `bᵢ`: the farthest of the points' images from the
//!   chord `Aᵢ Bᵢ`, with the clears and the rounding of the two placed
//!   ends added: the image lies within `bᵢ` of its chord (a hull lies no
//!   farther from a segment than its farthest corner).
//! - (S2) *the aim*: a cone `Kᵢ` narrower than a half turn that holds every
//!   leg's image, each widened by the angle rounding can have turned it
//!   (`asin(2 clear / |leg|)`, a leg shorter than ten clears giving none):
//!   every move along the stretch, from one of its points to a later one,
//!   is a direction of `Kᵢ`. So the stretch is a simple arc (its points
//!   move strictly forward along the cone's middle), and its image and its
//!   chord are each crossed at most once by every line square to that
//!   middle.
//!
//! **The lemmas** (`mᵢ = bᵢ + gᵢ` the *margin* of segment `i`; `N(S, m)`
//! the points within `m` of a segment `S`, a convex set):
//!
//! - (L1) `πᵢ ⊂ N(Sᵢ, mᵢ)`: the image lies within `bᵢ` of `Aᵢ Bᵢ`, every
//!   point of which lies within `gᵢ` of `Sᵢ`, and the segment across the
//!   gap lies within `gᵢ` of `Aᵢ₊₁`.
//! - (L2) a point farther than `mᵢ` from every `Sᵢ` of a loop is wound by
//!   `Γ` as by `Π`: `Γ − Π` is the sum of the closed curves `πᵢ − Sᵢ`,
//!   each inside the convex `N(Sᵢ, mᵢ)`, which winds about no point
//!   outside it.
//! - (L3) the area `Γ` bounds (the integral of its winding number) is
//!   `Π`'s within `Σ 2 mᵢ (Lᵢ + mᵢ) + π mᵢ²`, `Lᵢ` the length of `Sᵢ`
//!   ([`area_error`]): `πᵢ − Sᵢ` is the image closed by its chord, which by
//!   (S2) winds at most once about any point and only inside `N(Aᵢ Bᵢ,
//!   bᵢ)`, an area of `2 bᵢ |Aᵢ Bᵢ| + π bᵢ²` at the most, and the triangle
//!   `Aᵢ Bᵢ Aᵢ₊₁`, of at most `gᵢ |Aᵢ Bᵢ| / 2`, with `|Aᵢ Bᵢ| ≤ Lᵢ + gᵢ`.
//! - (L4) two segments farther apart than `mᵢ + mⱼ + R` hold pieces
//!   farther apart than `R` (L1).
//! - (L5) two stretches of which the second starts where the first ends
//!   but for the gap `g` (and slacks), whose cones both fit in a fan
//!   narrower than a half turn by `2 asin(2 g / R)`, meet only within `R /
//!   2` of the first's end, and only at that point where `g` is nothing
//!   ([`neighbours_apart`], with its proof).
//!
//! **Cleared** ([`judge`]) is all of:
//!
//! - (C0) every stretch is shown with an aim ([`Walked::rings`] gives no
//!   loops otherwise; [`neighbours`] takes a stretch without an aim for
//!   not apart);
//! - (C1) every loop has three stretches or more (so two stretches that
//!   follow each other share one joint only);
//! - (C2) every two segments that do not follow each other in one loop,
//!   of the same loop or of two, are farther apart than both margins and
//!   `R` ([`crossing`]: every such pair whose boxes come within that is
//!   looked at, [`super::pairs_of_steps`]; one not looked at, past
//!   [`super::MOST_PAIRS`], clears nothing);
//! - (C3) every two stretches that follow each other in a loop are apart
//!   by (L5), every gap under `R / 4` ([`neighbours`]);
//! - (C4) every loop's polyline bounds more area than its error (L3);
//! - (C5) exactly one loop runs counter-clockwise about the face's normal,
//!   and its area less its error is more than any other's with its error;
//! - (C6) a point `A₀` of every other loop is wound once by that loop's
//!   polyline and by no other hole's.
//!
//! **Theorem.** If a face on a plane is cleared, then (i) every stretch is
//! a simple arc (S2); (ii) two pieces that do not follow each other in one
//! loop, and any two pieces of two loops, lie more than `R` apart (C2,
//! L4); (iii) two stretches that follow each other meet only within `R /
//! 2` of their joint, and only at the joint where it has no gap (C3, L5):
//! so no two loops meet, and each loop's curve is a closed curve that
//! meets itself nowhere but, at the most, within `R / 2` of a vertex where
//! two edges' ends miss each other (by less than `R / 4`); with every
//! edge's end on the next one's start it is a simple closed curve; (iv)
//! each loop's curve bounds an area of the sign of its polyline's (C4,
//! L3), so exactly one runs counter-clockwise about the face's normal,
//! bounding more than any other, and the others run clockwise (C5); (v)
//! every other loop lies inside that one and inside no other hole (C6:
//! `A₀` lies farther than `mᵢ + R` from every segment of every other loop
//! by C2, so each winds about it as its polyline does, L2, and a loop that
//! meets no other lies wholly on the side of it its point `A₀` is on). So
//! the loops bound a region, and none of the faults the walk can claim on
//! a plane holds (a sense, an outer loop smaller than a hole, a crossing,
//! a point wound other than once): which is why such a fault is dropped.
//!
//! **Where nothing is dropped.** Every step above that cannot be taken
//! leaves the walk's fault standing: a point not placed, a stretch not
//! shown or with no aim once it cannot be halved further, a spline that
//! jumps, a pair of segments within their margins and `R`
//! whose margins are within `R` already, two lines whose cones do not
//! fit, a gap of `R / 4` or more, an area not told, a frame that is not
//! orthonormal, a tolerance that is no number, [`MOST_ROUNDS`] rounds,
//! [`MOST_STRETCHES`] stretches of an edge, and the work of a face
//! ([`super::MOST_PROOF_WORK`], every point placed, leg read, pair of
//! segments and winding counted, [`super::spend`]: running out returns
//! `None` all the way up, which [`Verifier::region_shown`] and
//! [`Verifier::winding_shown`] read as "the fault stands"). Where the
//! bounds show a fault instead (a sense, a hole outside, a crossing by the
//! band lemma of [`crossing`]) the face is refused with the walk's message
//! where it is of the same kind, else with the fault shown.
//!
//! **The two faults found in this family, and why a plane has neither.**
//! (1) Two stretches that meet at a point were never held to each other:
//! here every pair of two different stretches is of exactly one of two
//! kinds, following each other in one loop (C3) or not (C2), and both are
//! asked of every pair (C1 makes the kinds exclusive). (2) Pieces a period
//! apart in the chart lie on one place of a closed surface: a plane's
//! chart has no period and is one to one, so two points of the plane are
//! one where, and only where, their chart points are.

use super::{
    Fan, Held, Lay, ONE_WAY, Pieces, QUARTER_TURN, Ring, Sweep, hull_rounding, one_way,
    pairs_of_steps, signed_area, spend, to_segment, winding,
};
use crate::{Body, Coedge, EdgeId, Face};
use craft3d_geom::{Curve3, NurbsCurve, Surface};
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
/// How far a cone's two ends are turned outward (radians) past what
/// rounding can have turned its legs, so that a cross product rounded on a
/// direction of the cone cannot leave it outside.
const WIDEN: f64 = 1e-9;
/// Pairs looked at between two charges of their work, so that the work of
/// a face is passed by no more than a charge.
const CHARGE: usize = 64;
/// Units in the last place of a chart point's largest coordinate by which
/// it may be off: the curve's point and its place on the surface.
const PLACE_ROUNDING: f64 = 64.0 * f64::EPSILON;
/// How far a plane's frame may be from unit axes square to each other for
/// its chart to be taken for an isometry ([`flat`]).
const FRAME_ROUNDING: f64 = 1e-9;

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

/// The surfaces a face is walked again on: a plane, whose chart is affine,
/// one to one, and has no period and no pole. On every other surface the
/// walk's fault stands as it is.
fn asked(surface: &Surface) -> bool {
    matches!(surface, Surface::Plane(_))
}

/// The axes of a plane's chart, where they are unit vectors square to each
/// other to [`FRAME_ROUNDING`]: the chart is then an isometry of the plane
/// to that, and a length in it is the model's. `None` for any other
/// surface or frame.
fn flat(surface: &Surface) -> Option<(Vec3, Vec3)> {
    let Surface::Plane(plane) = surface else {
        return None;
    };
    let (x, y) = (plane.frame().x(), plane.frame().y());
    let unit = |v: Vec3| (v.length_squared() - 1.0).abs() <= FRAME_ROUNDING;
    (unit(x) && unit(y) && x.dot(y).abs() <= FRAME_ROUNDING).then_some((x, y))
}

/// Whether a spline jumps over the knots `from..=to` (indices, cut to the
/// knots there are): some knot value inside its domain stands there more
/// often than its degree. At such a knot the curve jumps from one control
/// point to the next, and it is not one curve: no hull holds what lies
/// between, since nothing does.
fn jumps(curve: &NurbsCurve, from: usize, to: usize) -> bool {
    let (d0, d1) = curve.domain();
    let degree = curve.degree();
    let knots = curve.knots();
    let to = to.min(knots.len().saturating_sub(1));
    let mut run = 0_usize;
    let mut before = f64::NAN;
    for k in knots.get(from..=to).unwrap_or(&[]) {
        if *k == before {
            run += 1;
        } else {
            run = 1;
            before = *k;
        }
        if run > degree && *k > d0 && *k < d1 {
            return true;
        }
    }
    false
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
    /// Shown: the image lies within this of its chord (S1 of the module),
    /// and where it heads, in the order of the edge's parameter ([`Aim`],
    /// S2). No aim: the stretch is a point, or a leg is too short against
    /// what rounding can have moved its ends to give its direction within
    /// a fifth of a radian (a leg of no length among other points not all
    /// one among them); a face with such a stretch is not cleared.
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
    /// Their places in the chart.
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
    /// The plane's chart ([`Lay::Flat`], its axes checked by [`flat`]).
    lay: Lay,
    near: f64,
    /// Ten working tolerances in the chart, which are ten in the model
    /// ([`flat`]).
    reach: f64,
    loops: Vec<(Vec<Coedge>, Vec<Stretch>)>,
    pieces: BTreeMap<EdgeId, Pieces<'a>>,
    /// What rounding can have moved a spline edge's hull points
    /// ([`hull_rounding`]), found once an edge.
    roundings: BTreeMap<EdgeId, Option<f64>>,
    work: usize,
}

/// The place of `p` in the chart of the plane `surface`: `None` off the
/// plane by more than `near`, or for numbers that are none.
fn place(surface: &Surface, near: f64, p: Vec3) -> Option<[f64; 2]> {
    let (u, v) = surface.project(p).ok()?;
    let off = surface.eval(u, v).distance(p);
    if off.is_nan() || off > near {
        return None;
    }
    (u.is_finite() && v.is_finite()).then_some([u, v])
}

impl<'a> Walked<'a> {
    /// The loops of `face`, on a plane, in stretches: a line in one, an
    /// arc of a circle or an ellipse in quarter turns at the most, a
    /// spline in two, each end placed. `None` where the face cannot be
    /// walked so: not on a plane whose frame is orthonormal ([`flat`]), a
    /// tolerance that is no positive number, a range that is none, a point
    /// off the plane, the work spent.
    fn new(body: &'a Body, face: &'a Face, tol: &Tolerance) -> Option<Walked<'a>> {
        let surface = &face.surface;
        let (x, y) = flat(surface)?;
        let near = 10.0 * body.working_tolerance(tol.linear);
        if !(near.is_finite() && near > 0.0) {
            return None;
        }
        let mut walked = Walked {
            body,
            surface,
            lay: Lay::Flat { x, y },
            near,
            reach: near,
            loops: Vec::with_capacity(face.loops.len()),
            pieces: BTreeMap::new(),
            roundings: BTreeMap::new(),
            work: 0,
        };
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
                // not shown or its bound is too wide: a line in one; an arc
                // in quarter turns at the most (a rational quadratic holds
                // it); a spline in two.
                let count = match &edge.curve {
                    Curve3::Line(_) => 1,
                    Curve3::Nurbs(_) => 2,
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
        place(self.surface, self.near, curve.eval(t))
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
    /// ([`Bound::Not`]); `None` where the work is spent. The points made
    /// for it are those of the module's (H) and (D).
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
        let mut held = Held::default();
        match &edge.curve {
            Curve3::Line(_) => {
                let ends = [edge.curve.eval(lo), edge.curve.eval(hi)];
                held.take(&ends, rounding_of(&ends));
                self.hull_bound(&held, a, b)
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
                // A rational quadratic with positive weights holds an arc
                // of less than a half turn; taken up to a quarter.
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
                self.hull_bound(&held, a, b)
            }
            Curve3::Nurbs(n) => {
                let rounding = match self.roundings.get(&edge_id) {
                    Some(r) => *r,
                    None => {
                        // A look at each control point and weight, once an
                        // edge, not counted (as in the walk, which looks
                        // once a step).
                        let r = hull_rounding(n);
                        self.roundings.insert(edge_id, r);
                        r
                    }
                };
                let Some(rounding) = rounding else {
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
                    // One curve over those spans: no knot of theirs, the
                    // stretch's two ends included, at which it jumps (the
                    // knots are as many as the points looked at, and are
                    // counted with them).
                    if jumps(n, first, span(hi).saturating_add(degree + 1)) {
                        return Some(Bound::Not);
                    }
                    held.take(points, clear);
                    return self.hull_bound(&held, a, b);
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
                self.hull_bound(&held, a, b)
            }
        }
    }

    /// The bound and the aim shown by points `held` whose hulls hold the
    /// stretch run by run and whose legs hold its direction ((H) and (D) of
    /// the module), `a` and `b` the stretch's chart points at its lower and
    /// its higher parameter.
    fn hull_bound(&mut self, held: &Held, a: [f64; 2], b: [f64; 2]) -> Option<Bound> {
        let Lay::Flat { x, y } = self.lay else {
            return Some(Bound::Not);
        };
        // One curve: each run starts where the one before ends, to their
        // clears (the Bézier pieces of a spline at a knot; a spline that
        // jumps there is not shown).
        let mut starts = 0_usize;
        let mut before: Option<(Vec3, f64)> = None;
        for &(end, clear) in &held.runs {
            let run = held.points.get(starts..end).unwrap_or(&[]);
            starts = end;
            let (Some(first), Some(last)) = (run.first(), run.last()) else {
                continue;
            };
            if let Some((point, cleared)) = before
                && point.distance(*first).partial_cmp(&(clear + cleared))
                    != Some(std::cmp::Ordering::Less)
                && point != *first
            {
                return Some(Bound::Not);
            }
            before = Some((*last, clear));
        }
        let (one, legs) = one_way(&self.lay, &held.points, &held.runs, None);
        spend(&mut self.work, legs)?;
        if !one {
            return Some(Bound::Not);
        }
        let widest_clear = held.runs.iter().map(|r| r.1).fold(0.0_f64, f64::max);
        let size = a
            .iter()
            .chain(b.iter())
            .fold(0.0_f64, |m, x| m.max(x.abs()));
        let rounded = PLACE_ROUNDING * size;
        // (S1). The plane's chart is affine: the image of a hull is the
        // hull of the points' images, and a hull lies no farther from the
        // chord than its farthest corner.
        let mut far = 0.0_f64;
        let mut starts = 0_usize;
        for &(end, clear) in &held.runs {
            for p in held.points.get(starts..end).unwrap_or(&[]) {
                let Some(q) = place(self.surface, f64::INFINITY, *p) else {
                    return Some(Bound::Not);
                };
                far = far.max(to_segment(q, a, b) + clear);
            }
            starts = end;
        }
        spend(&mut self.work, held.points.len())?;
        let bound = far + 2.0 * widest_clear + rounded;
        // (S2). The cone of the legs' directions along the plane (the
        // chart's), each widened by the angle by which rounding can have
        // turned it: its ends may each be `clear` off, the leg twice that,
        // a turn of at most `asin(2 clear / |leg|)`.
        let mut fan = Fan::within(ONE_WAY);
        let mut widest = 0.0_f64;
        let mut slack = 0.0_f64;
        let mut starts = 0_usize;
        let mut fits = true;
        for &(end, clear) in &held.runs {
            let run = held.points.get(starts..end).unwrap_or(&[]);
            starts = end;
            // Points all one (a piece of no width, where a stretch ends on
            // a knot): the curve there lies within `clear` of that point,
            // and moves by twice it at the most.
            if run.windows(2).all(|w| matches!(w, [p, q] if p == q)) {
                slack += 2.0 * clear;
                continue;
            }
            for pair in run.windows(2) {
                let [p, q] = pair else { continue };
                let w = *q - *p;
                let t = (w.dot(x), w.dot(y));
                let size = t.0 * t.0 + t.1 * t.1;
                if size == 0.0 {
                    // A leg of no length among others is none where the
                    // points are exact; else it may point anywhere.
                    if clear > 0.0 {
                        fits = false;
                    }
                    continue;
                }
                // The square of twice `clear` over the leg's length:
                // under a twenty-fifth, a fifth of a radian.
                let share = 4.0 * clear * clear / size;
                if share.partial_cmp(&0.04) != Some(std::cmp::Ordering::Less) || !fan.add(t) {
                    fits = false;
                }
                widest = widest.max(share);
            }
        }
        let turned = widest.sqrt().min(1.0).pasin();
        let aim = match (fits, fan.ends) {
            (true, Some((low, high))) if slack.is_finite() => Some(Aim {
                cone: widened((low, high), turned + WIDEN),
                slack,
            }),
            _ => None,
        };
        Some(if bound.is_finite() {
            Bound::Shown(bound, aim)
        } else {
            Bound::Not
        })
    }

    /// The loops as polylines of their stretches' starts, with each
    /// segment's margin, cone, gap and kind ([`Margined`]). `None` where a
    /// stretch is not shown.
    fn rings(&self) -> Option<Vec<Margined>> {
        let mut out = Vec::with_capacity(self.loops.len());
        for (coedges, stretches) in &self.loops {
            let n = stretches.len();
            let mut points = Vec::with_capacity(n);
            let mut ends = Vec::with_capacity(n);
            let mut bounds = Vec::with_capacity(n);
            let mut cones = Vec::with_capacity(n);
            let mut straight = Vec::with_capacity(n);
            let mut starts = Vec::new();
            let mut coedge = usize::MAX;
            for st in stretches {
                let Bound::Shown(bound, cone) = st.bound else {
                    return None;
                };
                if st.coedge != coedge {
                    starts.push(points.len());
                    coedge = st.coedge;
                }
                points.push(st.start);
                ends.push(st.end);
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
            }
            if points.is_empty() {
                return None;
            }
            // The gap from each stretch's end to the next one's start, the
            // last one's to the first's.
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
        Some(out)
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

/// What the bounds show of a face.
#[derive(Clone, Debug, PartialEq)]
enum Judged {
    /// A fault is shown: of this kind, with this message.
    Shown(Kind, String),
    /// Cleared: C0 to C6 of the module hold.
    Clear,
    /// Neither at these lengths: the stretches to halve (loop, place);
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
/// included). Two segments that do not cross properly, each one's ends
/// strictly on either side of the other's line, are as far apart as the
/// nearest of their four ends is from the other segment (nothing where they
/// touch or lie over each other: then an end of one lies on the other). A
/// number that is none counts as near.
fn near(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2], margin: f64) -> bool {
    let orient = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| {
        (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])
    };
    let cross = orient(a, b, c) * orient(a, b, d) < 0.0 && orient(c, d, a) * orient(c, d, b) < 0.0;
    let within = |d: f64| d.partial_cmp(&margin) != Some(std::cmp::Ordering::Greater);
    cross
        || within(to_segment(a, c, d))
        || within(to_segment(b, c, d))
        || within(to_segment(c, a, b))
        || within(to_segment(d, a, b))
}

/// What [`crossing`] found of the pairs of segments that do not follow each
/// other in one loop.
#[derive(Clone, Debug, PartialEq)]
enum Crossing {
    /// Two loops (or one and itself) cross.
    Crossed(usize, usize),
    /// Every such pair was looked at, and none comes within its margins
    /// and the reach (C2 of the module).
    Apart,
    /// Some do: these stretches to halve (none: halving cannot tell, or not
    /// every pair was looked at).
    Near(Vec<(usize, usize)>),
}

/// The pairs of segments of `rings` that do not follow each other in one
/// loop: the first that cross by more than their margins and ten tolerances
/// (`reach`): each one's ends lie on either side of the other's line by
/// more than that, so each piece of the boundary, within its margin of its
/// segment (L1), runs from one side of the other's band to its other side,
/// and within the parallelogram where the two bands meet the two join
/// opposite pairs of its sides, and meet; or else the stretches of the
/// pairs that come within those (the wider of each, and the other where its
/// margin is half as wide or more, each wider than `reach`), none where a
/// pair that does has both margins within `reach` (halving cannot tell it
/// from touching). Each pair looked at counts half a unit of `work`; `None`
/// where the work is spent.
fn crossing(rings: &[Margined], reach: f64, work: &mut usize) -> Option<Crossing> {
    let plain: Vec<Ring> = rings.iter().map(|m| copy(&m.ring)).collect();
    let widest = rings
        .iter()
        .flat_map(|m| m.margins.iter())
        .copied()
        .fold(0.0_f64, f64::max);
    // Past twice the widest margin and the reach no pair is within its
    // own margins and the reach; twice that, for the sweep's bands.
    let grow = 2.0 * (2.0 * widest + reach);
    if !grow.is_finite() {
        return Some(Crossing::Near(Vec::new()));
    }
    let mut near_ones: Vec<(usize, usize)> = Vec::new();
    let mut stuck = false;
    let mut visits = 0_usize;
    let margin = |r: usize, i: usize| {
        rings
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
                near_ones.push((r, i));
            }
            if mj > reach && 2.0 * mj >= mi {
                near_ones.push((s, j));
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
        Sweep::Found(r, _, s, _) => Crossing::Crossed(r.min(s), r.max(s)),
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
/// within half of ten tolerances (`reach`) of the first's end ((L5) of the
/// module): where the cones of their aims ([`Aim`]), in the order the loop
/// runs, both fit in a fan narrower than a half turn by `θ = 2 asin(2 g /
/// reach)`, `g` the gap and both slacks. *Proof*: then the fan's middle `n`
/// has `n·d ≥ sin(θ/2) |d|` for every direction `d` of either cone; a point
/// `x` of the first stretch is `E − a − e`, `E` its end, `a` in its cone
/// and `|e|` within its slack, and one of the second `P + b + f` alike, `P`
/// its start; where they meet, `n·(E − P) − n·(e + f) = n·a + n·b ≥
/// sin(θ/2) (|a| + |b|)`, so `|a| + |b| ≤ g / sin(θ/2) = reach / 2`, and
/// `x` lies within half the reach (and the slack) of `E`. With no gap and
/// no slack the fan is still narrower than a half turn (by 1e-9), `|a| +
/// |b| ≤ 0`, and they meet only at the point they share. Not shown
/// (false): a stretch with no aim, `g` of a quarter of the reach or more,
/// cones that do not fit, a number that is none.
fn neighbours_apart(a: Option<Aim>, b: Option<Aim>, gap: f64, reach: f64) -> bool {
    let (Some(a), Some(b)) = (a, b) else {
        return false;
    };
    let g = gap + a.slack + b.slack;
    if !(g >= 0.0 && reach > 0.0 && reach.is_finite() && g < 0.25 * reach) {
        return false;
    }
    let theta = 2.0 * (2.0 * g / reach).pasin();
    let mut fan = Fan::within(PI - theta.max(WIDEN));
    let (ca, cb) = (a.cone, b.cone);
    [ca.0, ca.1, cb.0, cb.1].into_iter().all(|t| fan.add(t))
}

/// The neighbouring stretches of `rings` not shown apart
/// ([`neighbours_apart`]: C3 of the module), the two of each pair that are
/// not both lines (a line's cone does not narrow when it is halved); and
/// whether some pair not shown apart is two lines (halving cannot part
/// them). Each pair counts a quarter unit of `work`; `None` where the work
/// is spent.
fn neighbours(
    rings: &[Margined],
    reach: f64,
    work: &mut usize,
) -> Option<(Vec<(usize, usize)>, bool)> {
    let mut halve = Vec::new();
    let mut stuck = false;
    let mut pairs = 0_usize;
    for (l, m) in rings.iter().enumerate() {
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

/// The stretches of `rings` (of the loop `only`, where given) whose
/// margins are the widest: wider than ten tolerances (`reach`) and than a
/// quarter of the widest.
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

/// What the bounds show of the face walked ([`Judged`]): a fault, that it
/// is cleared (C0 to C6 of the module, each at its place below), or
/// neither, with the stretches to halve. `None` where a stretch is not
/// shown or the work is spent.
fn judge(walked: &mut Walked<'_>, reversed: bool) -> Option<Judged> {
    // C0: every stretch shown (and, with C3 below, with an aim).
    let rings = walked.rings()?;
    let segments: usize = rings.iter().map(|m| m.margins.len()).sum();
    spend(&mut walked.work, segments.div_ceil(4))?;
    let reach = walked.reach;
    let crossed = crossing(&rings, reach, &mut walked.work)?;
    let (mut halve, stuck) = neighbours(&rings, reach, &mut walked.work)?;
    // C2 and C3: every pair of pieces apart.
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
    // C4: every loop's area told ((L3) of the module).
    let sense = if reversed { -1.0 } else { 1.0 };
    let mut areas = Vec::with_capacity(rings.len());
    let mut errors = Vec::with_capacity(rings.len());
    for (l, m) in rings.iter().enumerate() {
        let area = signed_area(&m.ring.points) * sense;
        let error = area_error(&m.ring.points, &m.margins);
        if !(area.is_finite() && error.is_finite() && area.abs() > error) {
            return Some(Judged::Unsure(widest_of(&rings, Some(l), reach)));
        }
        areas.push(area);
        errors.push(error);
    }
    // C5: one loop counter-clockwise, bounding more than any other.
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
    let o = match outer.as_slice() {
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
                // The outer loop bounds more than any hole, for every area
                // within the errors.
                o
            } else if a + e < others(o, false) {
                let largest = (0..areas.len()).filter(|k| *k != o).max_by(|x, y| {
                    let (Some(x), Some(y)) = (areas.get(*x), areas.get(*y)) else {
                        return std::cmp::Ordering::Equal;
                    };
                    x.abs().total_cmp(&y.abs())
                })?;
                // Shown: some hole bounds more than the outer loop, for
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
    };
    if let Crossing::Crossed(i, j) = crossed {
        // Shown: two pieces of the boundary cross ([`crossing`]).
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
    if !apart {
        // Some pair of pieces is not shown apart: its stretches, or none
        // where halving cannot part it.
        return Some(Judged::Unsure(halve));
    }
    // C6: every hole inside the outer loop and inside no other hole. Every
    // pair of pieces being apart (C2), a point of a hole's polyline lies
    // farther than its margin and the reach from every segment of every
    // other loop, so each winds about it as its polyline does (L2); a
    // polyline winds about no point outside its box.
    for (k, m) in rings.iter().enumerate() {
        if k == o {
            continue;
        }
        let Some(point) = m.ring.points.first().copied() else {
            return Some(Judged::Unsure(Vec::new()));
        };
        let mut each = Vec::with_capacity(rings.len());
        for (j, other) in rings.iter().enumerate() {
            each.push(if j == k || other.ring.box_distance(point) > 0.0 {
                0
            } else {
                spend(&mut walked.work, other.ring.points.len().div_ceil(2))?;
                winding(point, &other.ring.points)
            });
        }
        let wound = each.iter().sum::<i64>() * if reversed { -1 } else { 1 };
        if wound != 1 {
            // Shown: the hole lies outside the outer loop or inside
            // another hole.
            return Some(Judged::Shown(
                Kind::Hole,
                format!(
                    "its hole, loop {k}, lies outside its outer loop {o} or inside another hole (wound {wound} times at ({:.6}, {:.6}) of the {}; by each loop {}; areas {})",
                    point[0],
                    point[1],
                    walked.surface.kind(),
                    each.iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", "),
                    list(&areas),
                ),
            ));
        }
    }
    // C1: three stretches a loop or more (an area that is told has them;
    // asked outright, since C2 and C3 between them take every pair of
    // stretches only then).
    if rings.iter().any(|m| m.ring.points.len() < 3) {
        return Some(Judged::Unsure(Vec::new()));
    }
    // Cleared: C0 to C6 hold, and with them the module's theorem.
    Some(Judged::Clear)
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

/// How far a loop's area can be from its polyline's ((L3) of the module):
/// each segment's piece of the boundary, with the segment, winds about no
/// point more than once and only within its margin `m` of the segment
/// (length `L`), a region of no more than `2 m (L + m) + π m²`.
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
/// verified, its verdict kept for every other fault of the face: one
/// [`super::MOST_PROOF_WORK`] of work and [`MOST_ROUNDS`] rounds of halving
/// for the whole face, every point placed, leg read, pair of segments and
/// winding counted (in [`super::chord_proofs`], apart from the walk's own;
/// a face whose second walk passes the bound is counted there once). The
/// verdict is of the whole face: a fault of any loop is dropped only where
/// every loop is cleared ([`judge`]). Where any step of it cannot be taken
/// (a point not placed, a stretch not shown within the stretches of an
/// edge, the work or the rounds spent), the walk's faults stand.
pub(super) struct Verifier<'a> {
    body: &'a Body,
    face: &'a Face,
    tol: &'a Tolerance,
    /// The verdict, once made: `Some(None)` where a step could not be
    /// taken.
    verdict: Option<Option<Judged>>,
}

impl<'a> Verifier<'a> {
    pub(super) fn new(body: &'a Body, face: &'a Face, tol: &'a Tolerance) -> Self {
        Verifier {
            body,
            face,
            tol,
            verdict: None,
        }
    }

    /// Rounds of [`judge`], halving what it asks between them, until it
    /// says shown, cleared, or that halving cannot tell more; `None` where
    /// a step cannot be taken.
    fn settle(&self) -> Option<Judged> {
        let _counted = Verifying::begin();
        let mut walked = Walked::new(self.body, self.face, self.tol)?;
        for _ in 0..MOST_ROUNDS {
            walked.show_all()?;
            match judge(&mut walked, self.face.reversed)? {
                Judged::Unsure(halve) if !halve.is_empty() => walked.halve(&halve)?,
                other => return Some(other),
            }
        }
        None
    }

    /// The face's verdict, made once: `None` where a step could not be
    /// taken, or on a surface not [`asked`] (no second walk at all).
    fn verdict(&mut self) -> Option<&Judged> {
        if !asked(&self.face.surface) {
            return None;
        }
        if self.verdict.is_none() {
            self.verdict = Some(self.settle());
        }
        self.verdict.as_ref().and_then(Option::as_ref)
    }

    /// The region rule's fault `original` of the face, verified: the
    /// message to refuse with, or `None` where the face is cleared
    /// ([`judge`]: the module's theorem holds of it), so that the walk's
    /// fault was its reading of a step. Where the bounds show a fault, the
    /// walk's message where it is of the same kind ([`kind_of`]), else the
    /// fault shown. Where they show neither (halving cannot part two
    /// stretches, the work, the rounds or the stretches of an edge are
    /// spent, a point is not placed) the walk's fault stands: a fault is
    /// dropped only where the face is cleared. On a surface not [`asked`],
    /// `original`.
    pub(super) fn region_shown(&mut self, original: &str) -> Option<String> {
        match self.verdict() {
            Some(Judged::Shown(kind, message)) => Some(if kind_of(original) == Some(*kind) {
                original.to_string()
            } else {
                message.clone()
            }),
            Some(Judged::Clear) => None,
            Some(Judged::Unsure(_)) | None => Some(original.to_string()),
        }
    }

    /// Whether a fault the rule of simple loops' third rule found in a
    /// loop of the face (a point it winds about neither 0 times nor once
    /// its own way) stands: false only where the whole face is cleared
    /// ([`judge`]), every loop of it a simple closed curve of the sense a
    /// face's must have, no two meeting; else true, the walk's finding
    /// kept. (Not where that loop alone is one: the walk can have misread
    /// its sense, or another loop.) On a surface not [`asked`], true.
    pub(crate) fn winding_shown(&mut self) -> bool {
        !matches!(self.verdict(), Some(Judged::Clear))
    }
}

#[cfg(test)]
mod tests;
