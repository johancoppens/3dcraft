//! The offset of closed profiles in their plane: every loop moved a
//! distance square to itself, outer loops out and holes in for a positive
//! distance (the material grows), the other way for a negative one.
//!
//! Exact for lines, arcs and circles: the offset of a line is a line, of
//! an arc an arc about the same centre with the radius `r ± d`. Splines
//! are refused ("not supported yet").
//!
//! How it works (docs/design/sketch-offset.md):
//! 1. Every loop runs with its material on its left (outer loops
//!    counter-clockwise, holes clockwise), so the offset of every curve
//!    lies `d` to its right. A curve whose offset radius is no longer
//!    above the tolerance is left out.
//! 2. At a corner where the two offset curves part (a convex corner of
//!    the material for a positive distance, a concave one for a negative
//!    one), the gap is closed: by an arc of radius `|d|` about the corner
//!    (`Round`, the true offset: every point at distance `|d|` from the
//!    outline), or by carrying both neighbours on along their tangents to
//!    where they meet (`Sharp`, a mitre: a line is lengthened, an arc gets
//!    a straight piece). A corner next to a curve that was left out is
//!    round. Where they overlap nothing is added; a gap below the
//!    tolerance is a smooth joint.
//! 3. This raw offset is cut at every point where two of its curves meet
//!    (exact line and circle intersections); curves that follow each
//!    other at a smooth joint or a closed gap only meet there and are not
//!    intersected. The pairs whose boxes overlap come from a tree of the
//!    boxes ([`BoxTree`]) and are judged in the order of a sweep in x
//!    over every pair (`offset_reference.rs` keeps that sweep as the
//!    oracle), so the cuts are the same, in the same order.
//! 4. A piece is kept when its middle is at distance `|d|` from the
//!    outline (at least `|d| − tol/4`; a piece nearer to it lies inside the
//!    offset region and is not its boundary), and, for `Sharp`, not inside
//!    the wedge a mitre adds at another corner (the curves and wedges
//!    that may be that near come from trees of their boxes, grown by
//!    more than the distance and the rounding of the tests). The boundary
//!    of the offset
//!    region is a subset of the raw offset (the closest point of the
//!    outline to a boundary point is inside a curve, whose offset passes
//!    there, or a corner, whose arc does), so the kept pieces are exactly
//!    that boundary.
//! 5. The kept pieces go through [`detect_profiles`], which joins them
//!    into loops, refuses any that touch or cross, and orients them; every
//!    piece must then run the way it was made (material on its left), or
//!    the outline is refused rather than handed on.
//!
//! There is no mitre limit: a sharp corner of 1° puts its tip far out,
//! which is the true mitre. Inward past half the diagonal of the
//! outline's box nothing can be left; outward the size must stay within
//! what the tolerance holds (`tol / (1024 ε)`): both are refused before
//! anything is computed, with the largest distance that fits.
//!
//! A loop of the input that leaves no piece (a hole that closes, a region
//! that vanishes) is refused with the largest distance at which every loop
//! is still there, found by bisection. A result with another number of
//! loops (a region split in two, two regions joined) is returned and says
//! so, with that distance.

use crate::KernelError;
use crate::MAX_REPORTED;
use crate::boxtree::{Box2, BoxTree};
use crate::profile::{Curve2, Profile, Segment, Shape, SketchCurve, detect_profiles};
use craft3d_math::Tolerance;
use std::f64::consts::{PI, TAU};

type P2 = [f64; 2];

fn sub(a: P2, b: P2) -> P2 {
    [a[0] - b[0], a[1] - b[1]]
}

fn add(a: P2, b: P2) -> P2 {
    [a[0] + b[0], a[1] + b[1]]
}

fn scaled(a: P2, s: f64) -> P2 {
    [a[0] * s, a[1] * s]
}

fn dot(a: P2, b: P2) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

fn cross(a: P2, b: P2) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

fn length(a: P2) -> f64 {
    a[0].hypot(a[1])
}

fn distance(a: P2, b: P2) -> f64 {
    length(sub(a, b))
}

fn angle_of(v: P2) -> f64 {
    v[1].atan2(v[0])
}

/// An angle wrapped to `[0, 2π)`.
fn wrap(a: f64) -> f64 {
    let w = a.rem_euclid(TAU);
    if w >= TAU { 0.0 } else { w }
}

fn polar(c: P2, r: f64, angle: f64) -> P2 {
    let (s, co) = angle.sin_cos();
    [c[0] + r * co, c[1] + r * s]
}

/// The right-hand normal of a direction.
fn right(t: P2) -> P2 {
    [t[1], -t[0]]
}

/// How the gap at a corner is closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corners {
    Round,
    Sharp,
}

/// What a curve of an offset outline is the offset of.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum OffsetSource {
    /// The offset of this curve of the input.
    Curve(u64),
    /// The joint at the corner where the first curve runs into the second
    /// (in the direction the loop runs): the arc of a round corner, or the
    /// straight mitre where neither neighbour is a line.
    Corner(u64, u64),
    /// The straight piece a sharp corner carries the offset of an arc on
    /// by, along its tangent, to the mitre with the other curve: the
    /// offset of the first curve, towards the second.
    Extension(u64, u64),
}

/// Where a curve of an offset outline came from: its source, and, when
/// the source left more than one piece (a region split, regions joined),
/// what tells the pieces apart: the loop each closes, by the curves of the
/// input whose offsets it holds (ascending; an order along a loop would
/// change with the distance, these do not). Pieces of one source in loops
/// of the same curves are not told apart: they have the same origin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OffsetOrigin {
    pub source: OffsetSource,
    pub within: Option<Vec<u64>>,
}

/// An offset outline.
#[derive(Clone, Debug, PartialEq)]
pub struct OffsetOutline {
    /// The curves, with the ids 0, 1, 2, … in order.
    pub curves: Vec<SketchCurve>,
    /// Where each curve came from, by id.
    pub origins: Vec<OffsetOrigin>,
    /// The profiles of the curves.
    pub profiles: Vec<Profile>,
    /// How many loops the input had, and the outline has.
    pub loops_before: usize,
    pub loops_after: usize,
    /// How many regions (outer loops with their holes) the input had, and
    /// the outline has.
    pub regions_before: usize,
    pub regions_after: usize,
    /// When the number of loops or of regions changed: the largest
    /// distance at which neither had.
    pub whole_up_to: Option<f64>,
}

impl OffsetSource {
    /// The curves of the input it names.
    pub fn curves(self) -> Vec<u64> {
        match self {
            OffsetSource::Curve(c) => vec![c],
            OffsetSource::Corner(a, b) | OffsetSource::Extension(a, b) => vec![a, b],
        }
    }
}

/// The curves of the input whose offsets a loop of the outline holds,
/// ascending without repeats; the curves of its joints only when it holds
/// no offset of a curve (a loop of corner arcs alone).
pub fn loop_curves<'a>(origins: impl Iterator<Item = &'a OffsetOrigin> + Clone) -> Vec<u64> {
    let mut out: Vec<u64> = origins
        .clone()
        .filter_map(|o| match o.source {
            OffsetSource::Curve(c) => Some(c),
            _ => None,
        })
        .collect();
    if out.is_empty() {
        out = origins.flat_map(|o| o.source.curves()).collect();
    }
    out.sort_unstable();
    out.dedup();
    out
}

impl OffsetOutline {
    /// True when the outline has as many loops and regions as the input.
    pub fn is_whole(&self) -> bool {
        self.loops_after == self.loops_before && self.regions_after == self.regions_before
    }
}

/// A curve of the raw offset, in the direction it runs (material on its
/// left).
#[derive(Clone, Debug)]
enum Geo {
    Line {
        a: P2,
        b: P2,
    },
    /// From `start` to `end` about `c`, turning through `sweep` (positive
    /// counter-clockwise); a whole circle has `closed` and `|sweep| = 2π`.
    Arc {
        c: P2,
        r: f64,
        a0: f64,
        sweep: f64,
        start: P2,
        end: P2,
        closed: bool,
    },
}

impl Geo {
    fn start(&self) -> P2 {
        match self {
            Geo::Line { a, .. } => *a,
            Geo::Arc { start, .. } => *start,
        }
    }

    fn end(&self) -> P2 {
        match self {
            Geo::Line { b, .. } => *b,
            Geo::Arc { end, .. } => *end,
        }
    }

    fn closed(&self) -> bool {
        matches!(self, Geo::Arc { closed: true, .. })
    }

    fn length(&self) -> f64 {
        match self {
            Geo::Line { a, b } => distance(*a, *b),
            Geo::Arc { r, sweep, .. } => r * sweep.abs(),
        }
    }

    fn point_at(&self, t: f64) -> P2 {
        match self {
            Geo::Line { a, b } => add(*a, scaled(sub(*b, *a), t)),
            Geo::Arc {
                c, r, a0, sweep, ..
            } => polar(*c, *r, a0 + t * sweep),
        }
    }

    /// Where `p` (a point on or within `tol` of the curve) lies along it,
    /// in `[0, 1]`; `None` when it is not on the curve. On a closed curve
    /// 0 and 1 are the same place.
    fn param(&self, p: P2, tol: f64) -> Option<f64> {
        match self {
            Geo::Line { a, b } => {
                let d = sub(*b, *a);
                let l2 = dot(d, d);
                if l2.is_nan() || l2 <= 0.0 {
                    return None;
                }
                let t = dot(sub(p, *a), d) / l2;
                let l = l2.sqrt();
                if t * l < -tol || (t - 1.0) * l > tol || cross(d, sub(p, *a)).abs() / l > tol {
                    return None;
                }
                Some(t.clamp(0.0, 1.0))
            }
            Geo::Arc {
                c,
                r,
                a0,
                sweep,
                closed,
                ..
            } => {
                let span = sweep.abs();
                if span.is_nan() || span <= 0.0 || (distance(p, *c) - r).abs() > tol {
                    return None;
                }
                let along = if *sweep > 0.0 {
                    wrap(angle_of(sub(p, *c)) - a0)
                } else {
                    wrap(a0 - angle_of(sub(p, *c)))
                };
                if *closed || along <= span {
                    return Some((along / span).clamp(0.0, 1.0));
                }
                // Past the end: near it, or near the start from below.
                if (along - span) * r <= tol {
                    Some(1.0)
                } else if (TAU - along) * r <= tol {
                    Some(0.0)
                } else {
                    None
                }
            }
        }
    }

    /// `[min x, min y, max x, max y]`, generously: an arc by its circle.
    fn bbox(&self) -> [f64; 4] {
        match self {
            Geo::Line { a, b } => [
                a[0].min(b[0]),
                a[1].min(b[1]),
                a[0].max(b[0]),
                a[1].max(b[1]),
            ],
            Geo::Arc { c, r, .. } => [c[0] - r, c[1] - r, c[0] + r, c[1] + r],
        }
    }

    /// The piece from `ta` to `tb` (`ta < tb`), with the given ends.
    fn piece(&self, ta: f64, tb: f64, pa: P2, pb: P2) -> Geo {
        match self {
            Geo::Line { .. } => Geo::Line { a: pa, b: pb },
            Geo::Arc {
                c, r, a0, sweep, ..
            } => Geo::Arc {
                c: *c,
                r: *r,
                a0: a0 + ta * sweep,
                sweep: (tb - ta) * sweep,
                start: pa,
                end: pb,
                closed: false,
            },
        }
    }

    /// The curve as the kernel takes it, and whether a loop must run
    /// against its own direction.
    fn curve(&self) -> (Curve2, bool) {
        match self {
            Geo::Line { a, b } => (Curve2::Line { start: *a, end: *b }, false),
            Geo::Arc {
                c,
                r,
                sweep,
                start,
                end,
                closed,
                ..
            } => {
                if *closed {
                    (
                        Curve2::Circle {
                            center: *c,
                            radius: *r,
                        },
                        *sweep < 0.0,
                    )
                } else if *sweep > 0.0 {
                    (
                        Curve2::Arc {
                            center: *c,
                            start: *start,
                            end: *end,
                        },
                        false,
                    )
                } else {
                    (
                        Curve2::Arc {
                            center: *c,
                            start: *end,
                            end: *start,
                        },
                        true,
                    )
                }
            }
        }
    }
}

/// A curve of the input, for distances to it.
enum Bound {
    Line {
        a: P2,
        b: P2,
    },
    /// Counter-clockwise from `begin` through `sweep`.
    Arc {
        c: P2,
        r: f64,
        begin: f64,
        sweep: f64,
    },
}

impl Bound {
    /// A box that holds the curve: an arc by its circle.
    fn bbox(&self) -> Box2 {
        match self {
            Bound::Line { a, b } => [
                a[0].min(b[0]),
                a[1].min(b[1]),
                a[0].max(b[0]),
                a[1].max(b[1]),
            ],
            Bound::Arc { c, r, .. } => [c[0] - r, c[1] - r, c[0] + r, c[1] + r],
        }
    }

    fn distance(&self, p: P2) -> f64 {
        match self {
            Bound::Line { a, b } => {
                let d = sub(*b, *a);
                let l2 = dot(d, d);
                let t = if l2 > 0.0 {
                    (dot(sub(p, *a), d) / l2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                distance(p, add(*a, scaled(d, t)))
            }
            Bound::Arc { c, r, begin, sweep } => {
                let v = sub(p, *c);
                if *sweep >= TAU || wrap(angle_of(v) - begin) <= *sweep {
                    (length(v) - r).abs()
                } else {
                    distance(p, polar(*c, *r, *begin))
                        .min(distance(p, polar(*c, *r, begin + sweep)))
                }
            }
        }
    }
}

/// A piece of the raw offset before it is cut.
struct Candidate {
    geo: Geo,
    source: OffsetSource,
    /// The loop of the input it belongs to.
    lp: usize,
    /// Where it is cut: the place along it and the point.
    cuts: Vec<(f64, P2)>,
}

/// The wedge a mitre adds at a corner: the corner, the ends of the two
/// offset curves and the tip, a convex quadrilateral.
struct Wedge {
    corner: [P2; 4],
    /// The sine of the angle the corner turns through (at the tip the
    /// wedge's angle is its supplement, with the same sine).
    turn_sine: f64,
    /// The two curves of the input that meet at the corner.
    curves: [u64; 2],
}

impl Wedge {
    /// True when `p` is inside by more than `margin` from every side.
    fn contains(&self, p: P2, margin: f64) -> bool {
        let q = self.corner;
        let sides = || q.iter().zip(q.iter().cycle().skip(1));
        // The orientation of the quadrilateral.
        let area: f64 = sides().map(|(a, b)| cross(*a, *b)).sum::<f64>();
        let sign = if area >= 0.0 { 1.0 } else { -1.0 };
        sides().all(|(&a, &b)| {
            let edge = sub(b, a);
            let l = length(edge);
            l > 0.0 && sign * cross(edge, sub(p, a)) / l > margin
        })
    }
}

/// The raw offset: its curves, the wedges of its mitres, and the pairs of
/// curves that only meet where they join.
#[derive(Default)]
struct Raw {
    candidates: Vec<Candidate>,
    wedges: Vec<Wedge>,
    adjacent: Vec<(usize, usize)>,
}

/// The direction the loop runs at `t` (0 the start, 1 the end) of a
/// segment of the input, and the point there.
fn tangent(segment: &Segment, at_end: bool) -> Option<(P2, P2)> {
    let p = if at_end { segment.end } else { segment.start };
    let t = match &segment.shape {
        Shape::Line => {
            let d = sub(segment.end, segment.start);
            let l = length(d);
            if l.is_nan() || l <= 0.0 {
                return None;
            }
            scaled(d, 1.0 / l)
        }
        Shape::Arc { center, .. } | Shape::Circle { center, .. } => {
            let u = sub(p, *center);
            let l = length(u);
            if l.is_nan() || l <= 0.0 {
                return None;
            }
            let u = scaled(u, 1.0 / l);
            if segment.reversed {
                [u[1], -u[0]]
            } else {
                [-u[1], u[0]]
            }
        }
        Shape::Spline(_) => return None,
    };
    Some((p, t))
}

/// The raw offset of one loop: the offset of each segment, and the joints
/// at its corners. `wedges` gets the mitre wedges, `adjacent` the pairs of
/// candidates that only meet where they join.
fn raw_loop(
    segments: &[Segment],
    lp: usize,
    d: f64,
    corners: Corners,
    tol: f64,
    raw: &mut Raw,
) -> Result<(), KernelError> {
    let Raw {
        candidates: out,
        wedges,
        adjacent,
    } = raw;
    let n = segments.len();
    // The offset of each segment (its index in `out`), or `None` when it
    // was left out.
    let mut own: Vec<Option<usize>> = Vec::with_capacity(n);
    for segment in segments {
        let unsupported = || KernelError::OffsetProfile {
            reason: "of a spline is not supported yet",
            curves: vec![segment.source],
            largest: None,
        };
        let geo = match &segment.shape {
            Shape::Spline(_) => return Err(unsupported()),
            Shape::Line => {
                let (_, t) = tangent(segment, false).ok_or_else(unsupported)?;
                let shift = scaled(right(t), d);
                Some(Geo::Line {
                    a: add(segment.start, shift),
                    b: add(segment.end, shift),
                })
            }
            Shape::Arc { center, radius } | Shape::Circle { center, radius } => {
                let ccw = !segment.reversed;
                let r = radius + if ccw { d } else { -d };
                if r > tol {
                    let closed = matches!(segment.shape, Shape::Circle { .. });
                    let (begin, span) = segment.angles().unwrap_or((0.0, TAU));
                    let (a0, sweep) = if ccw {
                        (begin, span)
                    } else {
                        (angle_of(sub(segment.start, *center)), -span)
                    };
                    let offset_end = |at_end: bool| {
                        tangent(segment, at_end).map(|(p, t)| add(p, scaled(right(t), d)))
                    };
                    let (start, end) = if closed {
                        let p = polar(*center, r, a0);
                        (p, p)
                    } else {
                        (
                            offset_end(false).ok_or_else(unsupported)?,
                            offset_end(true).ok_or_else(unsupported)?,
                        )
                    };
                    Some(Geo::Arc {
                        c: *center,
                        r,
                        a0,
                        sweep,
                        start,
                        end,
                        closed,
                    })
                } else {
                    None
                }
            }
        };
        own.push(geo.map(|geo| {
            out.push(Candidate {
                geo,
                source: OffsetSource::Curve(segment.source),
                lp,
                cuts: Vec::new(),
            });
            out.len() - 1
        }));
    }
    if n < 2 {
        return Ok(());
    }
    for i in 0..n {
        let j = (i + 1) % n;
        let (Some(a), Some(b)) = (segments.get(i), segments.get(j)) else {
            continue;
        };
        let (Some((v, t1)), Some((_, t2))) = (tangent(a, true), tangent(b, false)) else {
            continue;
        };
        let p1 = add(v, scaled(right(t1), d));
        let p2 = add(v, scaled(right(t2), d));
        let (ci, cj) = (own.get(i).copied().flatten(), own.get(j).copied().flatten());
        if distance(p1, p2) <= tol {
            // A smooth joint: the two offsets meet there only.
            if let (Some(ci), Some(cj)) = (ci, cj) {
                adjacent.push((ci, cj));
            }
            continue;
        }
        let turn = cross(t1, t2).atan2(dot(t1, t2));
        if turn * d <= 0.0 {
            // They overlap: cutting trims them.
            continue;
        }
        let source = OffsetSource::Corner(a.source, b.source);
        let sharp = corners == Corners::Sharp && ci.is_some() && cj.is_some();
        if !sharp {
            out.push(Candidate {
                geo: Geo::Arc {
                    c: v,
                    r: d.abs(),
                    a0: angle_of(sub(p1, v)),
                    sweep: turn,
                    start: p1,
                    end: p2,
                    closed: false,
                },
                source,
                lp,
                cuts: Vec::new(),
            });
            let k = out.len() - 1;
            adjacent.extend(ci.map(|c| (c, k)));
            adjacent.extend(cj.map(|c| (k, c)));
            continue;
        }
        let denominator = cross(t1, t2);
        let s = cross(sub(p2, p1), t2) / denominator;
        let m = add(p1, scaled(t1, s));
        if !(s.is_finite() && m.iter().all(|x| x.is_finite()) && s > 0.0) {
            return Err(KernelError::OffsetProfile {
                reason: "has a corner that turns back on itself, which has no mitre (corners: round has)",
                curves: vec![a.source, b.source],
                largest: None,
            });
        }
        wedges.push(Wedge {
            corner: [v, p1, m, p2],
            turn_sine: denominator.abs(),
            curves: [a.source, b.source],
        });
        // The neighbours carried on to the tip: a line is lengthened, an
        // arc gets a straight piece.
        let mut chain: Vec<usize> = Vec::new();
        let mut first = ci;
        if let Some(c) = ci.and_then(|c| out.get_mut(c))
            && let Geo::Line { b, .. } = &mut c.geo
        {
            *b = m;
        } else {
            out.push(Candidate {
                geo: Geo::Line { a: p1, b: m },
                source: OffsetSource::Extension(a.source, b.source),
                lp,
                cuts: Vec::new(),
            });
            chain.push(out.len() - 1);
        }
        let mut last = cj;
        if let Some(c) = cj.and_then(|c| out.get_mut(c))
            && let Geo::Line { a, .. } = &mut c.geo
        {
            *a = m;
        } else {
            out.push(Candidate {
                geo: Geo::Line { a: m, b: p2 },
                source: OffsetSource::Extension(b.source, a.source),
                lp,
                cuts: Vec::new(),
            });
            chain.push(out.len() - 1);
        }
        for k in chain {
            adjacent.extend(first.map(|f| (f, k)));
            first = Some(k);
        }
        if let (Some(f), Some(l)) = (first, last.take()) {
            adjacent.push((f, l));
        }
    }
    Ok(())
}

/// The points where two curves of the raw offset meet.
fn meet(a: &Geo, b: &Geo) -> Vec<P2> {
    match (a, b) {
        (Geo::Line { a: p, b: q }, Geo::Line { a: r, b: s }) => {
            let (d, e) = (sub(*q, *p), sub(*s, *r));
            let den = cross(d, e);
            if den == 0.0 || !den.is_finite() {
                return Vec::new();
            }
            let t = cross(sub(*r, *p), e) / den;
            vec![add(*p, scaled(d, t))]
        }
        (Geo::Line { a: p, b: q }, Geo::Arc { c, r, .. })
        | (Geo::Arc { c, r, .. }, Geo::Line { a: p, b: q }) => line_circle(*p, sub(*q, *p), *c, *r),
        (Geo::Arc { c: c1, r: r1, .. }, Geo::Arc { c: c2, r: r2, .. }) => {
            circle_circle(*c1, *r1, *c2, *r2)
        }
    }
}

/// How far below zero, relative to the size of its terms, a discriminant
/// may be and still be a tangency: 64 units in the last place, the rounding
/// of the few products and sums it is made of, with room to spare.
const TANGENT_ROUNDING: f64 = 64.0 * f64::EPSILON;

/// The points where the line `p + s d` meets the circle.
fn line_circle(p: P2, d: P2, c: P2, r: f64) -> Vec<P2> {
    let f = sub(p, c);
    let a = dot(d, d);
    let k = cross(f, d);
    let disc = a * r * r - k * k;
    if a.is_nan() || a <= 0.0 || disc.is_nan() {
        return Vec::new();
    }
    let b = dot(f, d);
    if disc <= 0.0 {
        // Tangent within rounding, the one point: `disc` is a difference of
        // two products of size `a r²`, so rounding leaves it within a few
        // units in the last place of that, whatever the scale. A line that
        // misses the circle by more is apart (and `param` checks every
        // point against both curves besides).
        return if disc >= -TANGENT_ROUNDING * a * r * r {
            vec![add(p, scaled(d, -b / a))]
        } else {
            Vec::new()
        };
    }
    let root = disc.sqrt();
    vec![
        add(p, scaled(d, (-b - root) / a)),
        add(p, scaled(d, (-b + root) / a)),
    ]
}

/// The points where two circles meet.
fn circle_circle(c1: P2, r1: f64, c2: P2, r2: f64) -> Vec<P2> {
    let e = sub(c2, c1);
    let d = length(e);
    if d.is_nan() || d <= 0.0 {
        return Vec::new();
    }
    let k = ((d - r1) * (d + r1) + r2 * r2) / (2.0 * d);
    let h2 = (k - (d - r1)) * (r1 + d - k);
    if h2.is_nan() {
        return Vec::new();
    }
    let u = scaled(e, 1.0 / d);
    let foot = add(c1, scaled(u, d - k));
    if h2 <= 0.0 {
        // Touching within rounding, or apart: the one point if touching.
        let gap = (d - (r1 + r2)).abs().min((d - (r1 - r2).abs()).abs());
        return if gap <= TANGENT_ROUNDING * (r1 + r2 + d) {
            vec![foot]
        } else {
            Vec::new()
        };
    }
    let h = h2.sqrt();
    let side = [-u[1] * h, u[0] * h];
    vec![add(foot, side), sub(foot, side)]
}

fn boxes_overlap(a: [f64; 4], b: [f64; 4], tol: f64) -> bool {
    a[0] <= b[2] + tol && b[0] <= a[2] + tol && a[1] <= b[3] + tol && b[1] <= a[3] + tol
}

/// The input's curves, for distances, and the curve each is.
fn outline_bounds(profiles: &[Profile]) -> (Vec<Bound>, Vec<u64>) {
    let mut out = Vec::new();
    let mut ids = Vec::new();
    for lp in profiles
        .iter()
        .flat_map(|p| std::iter::once(&p.outer).chain(&p.holes))
    {
        for s in &lp.segments {
            ids.push(s.source);
            out.push(match &s.shape {
                Shape::Arc { center, radius } | Shape::Circle { center, radius } => {
                    let (begin, sweep) = s.angles().unwrap_or((0.0, TAU));
                    Bound::Arc {
                        c: *center,
                        r: *radius,
                        begin,
                        sweep: if matches!(s.shape, Shape::Circle { .. }) {
                            TAU
                        } else {
                            sweep
                        },
                    }
                }
                Shape::Line | Shape::Spline(_) => Bound::Line {
                    a: s.start,
                    b: s.end,
                },
            });
        }
    }
    (out, ids)
}

/// The most pieces an offset outline may have: what profile detection
/// takes ([`crate::profile::MAX_CURVES`]), which the pieces go through.
/// Until 2026-10-17 it was [`OLD_MAX_PIECES`], when the cut and keep steps
/// compared pairs swept in x.
pub const MAX_PIECES: usize = crate::profile::MAX_CURVES;

/// The most curves the raw offset may have: a round corner adds an arc to
/// a curve, a sharp one up to two straight pieces, so an input that
/// profile detection takes stays below it.
pub const MAX_RAW: usize = 4 * MAX_PIECES;

/// The cap on pieces before the broad phase (until 2026-10-17): the work
/// bounds are set from it, so that nothing it let through is refused.
pub(crate) const OLD_MAX_PIECES: usize = 4096;

/// The cap on the raw offset's curves before the broad phase.
pub(crate) const OLD_MAX_RAW: usize = 4 * OLD_MAX_PIECES;

/// The most pairs of curves of the raw offset whose boxes overlap that the
/// cut step looks at, counted in both directions and each with itself: as
/// many as [`OLD_MAX_RAW`] curves can have, so nothing accepted under that
/// cap is refused for it.
pub(crate) const MAX_PAIRS: usize = OLD_MAX_RAW * OLD_MAX_RAW;

/// The most points where curves of the raw offset cross that the cut step
/// keeps: each is kept twice, once on each curve, and makes a piece of
/// each, so this bounds the memory and the work of the keep step (reached
/// in 3.5 s in a release build, at a peak of 388 MB for the whole test,
/// whose input alone takes about 200 MB: `offset_scale.rs` ›
/// `a_bound_ends_the_search_for_a_distance`; 2²⁴ took 1.2 GB per try). A
/// patterned sheet has a few per curve: one of [`MAX_PIECES`] pieces about
/// a million.
pub(crate) const MAX_CUT_POINTS: usize = 1 << 22;

/// The most curves of the input and mitre wedges near the middles of the
/// pieces (within the boxes searched) that the keep step looks at, for all
/// pieces together: the bound on its work (each costs a box test and at
/// most one distance or wedge test, tens of nanoseconds, so seconds at the
/// bound).
pub(crate) const MAX_NEAR: usize = 1 << 28;

/// The most curves the names of pieces may list in all (a piece of a curve
/// that left several is named by the curves of the loop it closes): as
/// many as an outline of [`OLD_MAX_PIECES`] pieces can have, so nothing
/// accepted under that cap is refused for it.
pub(crate) const MAX_NAMED: usize = OLD_MAX_PIECES * OLD_MAX_PIECES;

/// The caps and the bounds on the work, as one value so that tests can
/// meet each with a small input.
#[derive(Clone, Copy, Debug)]
struct Bounds {
    pieces: usize,
    raw: usize,
    pairs: usize,
    cut_points: usize,
    near: usize,
    named: usize,
}

const BOUNDS: Bounds = Bounds {
    pieces: MAX_PIECES,
    raw: MAX_RAW,
    pairs: MAX_PAIRS,
    cut_points: MAX_CUT_POINTS,
    near: MAX_NEAR,
    named: MAX_NAMED,
};

/// How the pairs that may meet are found.
#[derive(Clone, Copy, Debug)]
enum Search {
    /// From trees of boxes ([`BoxTree`]), within the bounds.
    Indexed(Bounds),
    /// By sweeps in x and by testing every curve and wedge, as before
    /// 2026-10-17: the reference the indexed search must agree with. Its
    /// caps are [`BOUNDS`]'.
    #[cfg(test)]
    AllPairs,
}

impl Search {
    fn bounds(self) -> Bounds {
        match self {
            Search::Indexed(bounds) => bounds,
            #[cfg(test)]
            Search::AllPairs => BOUNDS,
        }
    }
}

#[cfg(test)]
#[path = "offset_reference.rs"]
mod reference;

/// The error when a bound on the work was crossed while judging the
/// candidates `about` (the one judged first, then those found near it),
/// named by the curves of the input they are offsets of.
fn crowded(what: &'static str, limit: usize, about: impl IntoIterator<Item = u64>) -> KernelError {
    let mut curves: Vec<u64> = Vec::new();
    for id in about {
        if curves.len() >= MAX_REPORTED {
            break;
        }
        if !curves.contains(&id) {
            curves.push(id);
        }
    }
    curves.sort_unstable();
    KernelError::CrowdedOffset {
        what,
        curves,
        limit,
    }
}

/// The input curves a list of candidates are offsets of.
fn sources<'a>(
    raw: &'a [Candidate],
    which: impl IntoIterator<Item = usize> + 'a,
) -> impl Iterator<Item = u64> + 'a {
    which
        .into_iter()
        .filter_map(|k| raw.get(k))
        .flat_map(|c| c.source.curves())
}

/// The largest size of a coordinate in the boxes.
fn size_of(boxes: &[Box2]) -> f64 {
    boxes.iter().flatten().fold(0.0_f64, |m, x| m.max(x.abs()))
}

/// Step 3: every candidate is cut where it meets another.
///
/// Two candidates are intersected when their boxes overlap within `tol`
/// and they are not adjacent, in the order of a sweep in x: the candidates
/// by the left side of their box (then index), and for each the ones after
/// it. The pairs come from a [`BoxTree`] of the boxes, whose test is the
/// sweep's own overlap test (it passes for a box whenever it passes for a
/// box inside it), and the candidates found after a candidate are sorted
/// by their place in the sweep, so every cut is recorded in the order the
/// sweep records it.
fn cut(
    raw: &mut [Candidate],
    boxes: &[Box2],
    adjacent: &[(usize, usize)],
    tol: f64,
    bounds: Bounds,
) -> Result<(), KernelError> {
    let left = |i: usize| boxes.get(i).map_or(f64::NAN, |b| b[0]);
    let mut order: Vec<usize> = (0..raw.len()).collect();
    order.sort_by(|&i, &j| left(i).total_cmp(&left(j)).then(i.cmp(&j)));
    let mut position = vec![0_usize; raw.len()];
    for (at, &i) in order.iter().enumerate() {
        if let Some(slot) = position.get_mut(i) {
            *slot = at;
        }
    }
    let tree = BoxTree::new(boxes.to_vec());
    let mut pairs = bounds.pairs;
    let mut points = 0_usize;
    let (mut found, mut later, mut met) = (Vec::new(), Vec::new(), Vec::new());
    for &i in &order {
        let (Some(bi), Some(&at)) = (boxes.get(i), position.get(i)) else {
            continue;
        };
        if !tree.search(|b| boxes_overlap(*bi, *b, tol), &mut found, &mut pairs) {
            return Err(crowded(
                "pairs of curves near one another",
                bounds.pairs,
                sources(raw, std::iter::once(i).chain(found.iter().copied())),
            ));
        }
        later.clear();
        for &j in &found {
            if let Some(&place) = position.get(j)
                && place > at
            {
                later.push((place, j));
            }
        }
        later.sort_unstable();
        for &(_, j) in &later {
            let pair = if i < j { (i, j) } else { (j, i) };
            if adjacent.binary_search(&pair).is_ok() {
                continue;
            }
            met.clear();
            let (Some(ci), Some(cj)) = (raw.get(i), raw.get(j)) else {
                continue;
            };
            for p in meet(&ci.geo, &cj.geo) {
                if !p.iter().all(|x| x.is_finite()) {
                    continue;
                }
                if let (Some(ti), Some(tj)) = (ci.geo.param(p, tol), cj.geo.param(p, tol)) {
                    met.push((ti, tj, p));
                }
            }
            for &(ti, tj, p) in &met {
                points += 1;
                if points > bounds.cut_points {
                    return Err(crowded(
                        "points where its curves cross",
                        bounds.cut_points,
                        sources(raw, [i, j]),
                    ));
                }
                if let Some(c) = raw.get_mut(i) {
                    c.cuts.push((ti, p));
                }
                if let Some(c) = raw.get_mut(j) {
                    c.cuts.push((tj, p));
                }
            }
        }
    }
    Ok(())
}

/// The trees of step 4: of the input's curves, each box grown so that it
/// holds every point a curve's distance test can put within `keep_from`
/// of it, and of the mitre wedges, each box grown so that it holds every
/// point the wedge's test can find inside it.
struct Near {
    curves: BoxTree,
    wedges: BoxTree,
    left: usize,
    limit: usize,
}

/// Step 4's test: a piece is kept when its middle is at least `keep_from`
/// from every curve of the input and not inside a mitre wedge by more than
/// `margin`.
struct Judge<'a> {
    outline: &'a [Bound],
    /// The curve of the input each of `outline` is.
    ids: &'a [u64],
    wedges: &'a [Wedge],
    keep_from: f64,
    margin: f64,
    /// `None` for the reference, which tests every curve and wedge.
    near: Option<Near>,
}

/// How far, relative to the largest coordinate, the rounding of the
/// distance and wedge tests may move what they find: a few products,
/// differences and a square root of coordinates and their differences,
/// each within half a unit in the last place, with ample room (64 units).
const TEST_ROUNDING: f64 = 64.0 * f64::EPSILON;

impl<'a> Judge<'a> {
    /// `size` is the largest coordinate of the curves of the raw offset
    /// and of the input, which the middles of pieces lie on and near.
    fn new(
        search: Search,
        outline: &'a [Bound],
        ids: &'a [u64],
        wedges: &'a [Wedge],
        keep_from: f64,
        margin: f64,
        size: f64,
    ) -> Judge<'a> {
        let near = match search {
            Search::Indexed(bounds) => {
                // The tests are exact up to `rounding`. A point a curve's
                // test puts within `keep_from` of it is within
                // `keep_from + rounding` of a point of the curve (the
                // nearest point of a line, or of a circle, lies in its
                // box), so in its box grown by that; twice `keep_from`
                // leaves room for the rounding of the distance itself.
                let rounding = TEST_ROUNDING * size;
                let grow = |b: Box2, by: f64| [b[0] - by, b[1] - by, b[2] + by, b[3] + by];
                let curves = outline
                    .iter()
                    .map(|b| grow(b.bbox(), 2.0 * keep_from + rounding))
                    .collect();
                // A point the wedge's test finds inside is within
                // `rounding` of the inside of each side (the test is exact
                // up to that, and asks for more than `margin` > 0), so
                // within the wedge grown by the rounding over the sine of
                // half its sharpest angle. The two at the offset curves
                // are right angles; the corner's and the tip's add up to
                // half a turn and have the sine of the turn, which is at
                // most twice the sine of half of either; four times leaves
                // room for the rounding of the sine. A sine of 0 (or NaN)
                // grows the box to everything: the wedge is then tested
                // with every point, as the reference does.
                let wedge_boxes = wedges
                    .iter()
                    .map(|w| {
                        let mut b = [
                            f64::INFINITY,
                            f64::INFINITY,
                            f64::NEG_INFINITY,
                            f64::NEG_INFINITY,
                        ];
                        for q in &w.corner {
                            b = [
                                b[0].min(q[0]),
                                b[1].min(q[1]),
                                b[2].max(q[0]),
                                b[3].max(q[1]),
                            ];
                        }
                        let sharp = if w.turn_sine > 0.0 {
                            4.0 * rounding / w.turn_sine.min(1.0)
                        } else {
                            f64::INFINITY
                        };
                        grow(b, margin + rounding + sharp)
                    })
                    .collect();
                Some(Near {
                    curves: BoxTree::new(curves),
                    wedges: BoxTree::new(wedge_boxes),
                    left: bounds.near,
                    limit: bounds.near,
                })
            }
            #[cfg(test)]
            Search::AllPairs => None,
        };
        Judge {
            outline,
            ids,
            wedges,
            keep_from,
            margin,
            near,
        }
    }

    /// True when the piece whose middle is `p` is kept; `about` is the
    /// candidate it is a piece of, for the error.
    fn keeps(&mut self, p: P2, about: OffsetSource) -> Result<bool, KernelError> {
        let (outline, keep_from) = (self.outline, self.keep_from);
        let (wedges, margin) = (self.wedges, self.margin);
        let Some(near) = &mut self.near else {
            return Ok(!(outline.iter().any(|b| b.distance(p) < keep_from)
                || wedges.iter().any(|w| w.contains(p, margin))));
        };
        let holds = |b: &Box2| b[0] <= p[0] && p[0] <= b[2] && b[1] <= p[1] && p[1] <= b[3];
        let limit = near.limit;
        let crowded_about = |last: Option<Vec<u64>>| {
            crowded(
                "curves near the middles of its pieces",
                limit,
                about.curves().into_iter().chain(last.into_iter().flatten()),
            )
        };
        let mut last = None;
        let near_curve = |i: usize| outline.get(i).is_some_and(|b| b.distance(p) < keep_from);
        match near
            .curves
            .any(holds, near_curve, &mut near.left, &mut last)
        {
            None => {
                return Err(crowded_about(
                    last.and_then(|i| self.ids.get(i)).map(|&id| vec![id]),
                ));
            }
            Some(true) => return Ok(false),
            Some(false) => {}
        }
        last = None;
        let in_wedge = |i: usize| wedges.get(i).is_some_and(|w| w.contains(p, margin));
        match near.wedges.any(holds, in_wedge, &mut near.left, &mut last) {
            None => Err(crowded_about(
                last.and_then(|i| wedges.get(i)).map(|w| w.curves.to_vec()),
            )),
            Some(found) => Ok(!found),
        }
    }
}

/// One try at distance `d`: the outline, or why there is none.
fn attempt(
    profiles: &[Profile],
    d: f64,
    corners: Corners,
    tolerance: &Tolerance,
    search: Search,
) -> Result<OffsetOutline, KernelError> {
    let bounds = search.bounds();
    let tol = tolerance.linear;
    let loops: Vec<&crate::profile::Loop2> = profiles
        .iter()
        .flat_map(|p| std::iter::once(&p.outer).chain(&p.holes))
        .collect();
    let loops_before = loops.len();
    let mut made = Raw::default();
    for (lp, l) in loops.iter().enumerate() {
        raw_loop(&l.segments, lp, d, corners, tol, &mut made)?;
        if made.candidates.len() > bounds.raw {
            return Err(KernelError::TooManyOffsetCurves {
                count: made.candidates.len(),
                limit: bounds.raw,
            });
        }
    }
    let Raw {
        candidates: mut raw,
        wedges,
        mut adjacent,
    } = made;
    adjacent.iter_mut().for_each(|p| {
        if p.0 > p.1 {
            *p = (p.1, p.0);
        }
    });
    adjacent.sort_unstable();
    adjacent.dedup();

    // 3. Cut at every meeting, in the order of a sweep by the boxes' left
    // sides.
    let boxes: Vec<[f64; 4]> = raw.iter().map(|c| c.geo.bbox()).collect();
    match search {
        Search::Indexed(bounds) => cut(&mut raw, &boxes, &adjacent, tol, bounds)?,
        #[cfg(test)]
        Search::AllPairs => reference::cut(&mut raw, &boxes, &adjacent, tol),
    }

    // 4. The pieces, kept by their distance from the outline.
    let (outline, outline_ids) = outline_bounds(profiles);
    let reach = d.abs();
    let keep_from = reach - tol / 4.0;
    let size = size_of(&boxes).max(size_of(
        &outline.iter().map(Bound::bbox).collect::<Vec<_>>(),
    ));
    let mut judge = Judge::new(
        search,
        &outline,
        &outline_ids,
        &wedges,
        keep_from,
        tol / 4.0,
        size,
    );
    let mut pieces: Vec<(Geo, OffsetSource, usize)> = Vec::new();
    for candidate in &mut raw {
        let geo = &candidate.geo;
        let total = geo.length();
        let mut cuts: Vec<(f64, P2)> = std::mem::take(&mut candidate.cuts);
        cuts.sort_by(|a, b| a.0.total_cmp(&b.0));
        // Cuts within the tolerance of an end or of each other are one.
        let mut kept: Vec<(f64, P2)> = Vec::new();
        let closed = geo.closed();
        for (t, p) in cuts {
            let from_start = t * total;
            let to_end = (1.0 - t) * total;
            if !closed && (from_start <= tol || to_end <= tol) {
                continue;
            }
            if kept.last().is_some_and(|(s, _)| (t - s) * total <= tol) {
                continue;
            }
            kept.push((t, p));
        }
        let mut stretches: Vec<(f64, f64, P2, P2)> = Vec::new();
        if closed {
            if let (Some(&(t_first, p_first)), Some(&(t_last, _))) = (kept.first(), kept.last()) {
                // The wrap from the last cut to the first is one cut when
                // they are within the tolerance across the seam.
                if kept.len() >= 2 && (t_first + 1.0 - t_last) * total <= tol {
                    kept.pop();
                }
                if kept.len() >= 2 {
                    for (a, b) in kept.iter().zip(kept.iter().skip(1)) {
                        stretches.push((a.0, b.0, a.1, b.1));
                    }
                    if let Some(&(t_last, p_last)) = kept.last() {
                        stretches.push((t_last, t_first + 1.0, p_last, p_first));
                    }
                }
            }
        } else {
            let mut from = (0.0, geo.start());
            for &(t, p) in &kept {
                stretches.push((from.0, t, from.1, p));
                from = (t, p);
            }
            stretches.push((from.0, 1.0, from.1, geo.end()));
        }
        if stretches.is_empty() {
            let middle = geo.point_at(0.5);
            if judge.keeps(middle, candidate.source)? {
                pieces.push((geo.clone(), candidate.source, candidate.lp));
            }
            continue;
        }
        for (ta, tb, pa, pb) in stretches {
            let middle = geo.point_at((ta + tb) / 2.0);
            if !judge.keeps(middle, candidate.source)? {
                continue;
            }
            pieces.push((geo.piece(ta, tb, pa, pb), candidate.source, candidate.lp));
        }
    }

    // A loop of the input that left nothing has vanished.
    let mut alive = vec![false; loops_before];
    for (_, _, lp) in &pieces {
        if let Some(a) = alive.get_mut(*lp) {
            *a = true;
        }
    }
    if let Some(gone) = alive.iter().position(|a| !a) {
        let mut curves: Vec<u64> = loops
            .get(gone)
            .map(|l| l.segments.iter().map(|s| s.source).collect())
            .unwrap_or_default();
        curves.sort_unstable();
        curves.dedup();
        curves.truncate(MAX_REPORTED);
        let hole = profiles.iter().any(|p| {
            p.holes
                .iter()
                .any(|h| h.segments.first().map(|s| s.source) == curves.first().copied())
        });
        return Err(KernelError::OffsetProfile {
            reason: if hole {
                "closes a hole: no part of it is left"
            } else {
                "leaves nothing of a region"
            },
            curves,
            largest: None,
        });
    }

    // 5. Loops, oriented; each piece must run the way it was made.
    let mut curves = Vec::with_capacity(pieces.len());
    let mut origins = Vec::with_capacity(pieces.len());
    let mut against = Vec::with_capacity(pieces.len());
    for (id, (geo, source, _)) in pieces.iter().enumerate() {
        let (curve, reversed) = geo.curve();
        curves.push(SketchCurve {
            id: id as u64,
            curve,
        });
        origins.push(OffsetOrigin {
            source: *source,
            within: None,
        });
        against.push(reversed);
    }
    let sources_of = |ids: &[u64]| -> Vec<u64> {
        let mut out: Vec<u64> = ids
            .iter()
            .filter_map(|id| usize::try_from(*id).ok().and_then(|i| origins.get(i)))
            .flat_map(|o| o.source.curves())
            .collect();
        out.sort_unstable();
        out.dedup();
        out.truncate(MAX_REPORTED);
        out
    };
    if curves.len() > bounds.pieces {
        return Err(KernelError::TooManyOffsetCurves {
            count: curves.len(),
            limit: bounds.pieces,
        });
    }
    let found = detect_profiles(&curves, tolerance).map_err(|error| {
        let (reason, ids) = match &error {
            KernelError::BranchingProfile { curves } => (
                "touches itself: more than two of its curves meet in one point",
                curves.clone(),
            ),
            KernelError::OpenProfile { curves } => (
                "does not close: a curve of it meets nothing (a joint below the tolerance, or an outline that touches itself)",
                curves.clone(),
            ),
            KernelError::CrossingCurves { curves } => {
                ("touches or crosses itself", curves.to_vec())
            }
            KernelError::DegenerateCurve { id, .. } => (
                "has a piece shorter than the tolerance, where its loops nearly touch",
                vec![*id],
            ),
            KernelError::ZeroAreaLoop { curves } => {
                ("has a loop with no area", curves.clone())
            }
            // Its own reason (a cap, the tolerance), not one of the
            // outline: passed on as it is.
            _ => return error,
        };
        KernelError::OffsetProfile {
            reason,
            curves: sources_of(&ids),
            largest: None,
        }
    })?;
    for lp in found
        .iter()
        .flat_map(|p| std::iter::once(&p.outer).chain(&p.holes))
    {
        for s in &lp.segments {
            let expected = usize::try_from(s.source)
                .ok()
                .and_then(|i| against.get(i))
                .copied();
            if expected != Some(s.reversed) {
                return Err(KernelError::OffsetProfile {
                    reason: "came out the wrong way round where its loops meet, and is refused rather than handed on",
                    curves: sources_of(&[s.source]),
                    largest: None,
                });
            }
        }
    }
    // A source that left more than one piece: each piece is told apart by
    // the loop it closes, named by the input's curves in that loop.
    let mut count: std::collections::BTreeMap<OffsetSource, usize> = Default::default();
    for origin in &origins {
        *count.entry(origin.source).or_insert(0) += 1;
    }
    let mut named = 0_usize;
    for lp in found
        .iter()
        .flat_map(|p| std::iter::once(&p.outer).chain(&p.holes))
    {
        let ids: Vec<usize> = lp
            .segments
            .iter()
            .filter_map(|s| usize::try_from(s.source).ok())
            .collect();
        let within = loop_curves(ids.iter().filter_map(|&i| origins.get(i)));
        for i in ids {
            if let Some(origin) = origins.get_mut(i)
                && count.get(&origin.source).is_some_and(|&n| n > 1)
            {
                // Each such piece lists the loop's curves: bounded in all.
                named = named.saturating_add(within.len());
                if named > bounds.named {
                    return Err(crowded(
                        "curves listed in the names of its pieces (pieces of one curve in long loops)",
                        bounds.named,
                        origin
                            .source
                            .curves()
                            .into_iter()
                            .chain(within.iter().copied()),
                    ));
                }
                origin.within = Some(within.clone());
            }
        }
    }
    let loops_after = found.iter().map(|p| 1 + p.holes.len()).sum();
    Ok(OffsetOutline {
        curves,
        origins,
        regions_before: profiles.len(),
        regions_after: found.len(),
        profiles: found,
        loops_before,
        loops_after,
        whole_up_to: None,
    })
}

/// The outline unchanged: the offset of a distance within the tolerance.
fn unchanged(profiles: &[Profile]) -> Result<OffsetOutline, KernelError> {
    let mut curves = Vec::new();
    let mut origins = Vec::new();
    for lp in profiles
        .iter()
        .flat_map(|p| std::iter::once(&p.outer).chain(&p.holes))
    {
        for s in &lp.segments {
            let curve = match &s.shape {
                Shape::Line => Curve2::Line {
                    start: s.start,
                    end: s.end,
                },
                Shape::Arc { center, .. } => {
                    let (start, end) = if s.reversed {
                        (s.end, s.start)
                    } else {
                        (s.start, s.end)
                    };
                    Curve2::Arc {
                        center: *center,
                        start,
                        end,
                    }
                }
                Shape::Circle { center, radius } => Curve2::Circle {
                    center: *center,
                    radius: *radius,
                },
                Shape::Spline(_) => {
                    return Err(KernelError::OffsetProfile {
                        reason: "of a spline is not supported yet",
                        curves: vec![s.source],
                        largest: None,
                    });
                }
            };
            curves.push(SketchCurve {
                id: curves.len() as u64,
                curve,
            });
            origins.push(OffsetOrigin {
                source: OffsetSource::Curve(s.source),
                within: None,
            });
        }
    }
    let loops = profiles.iter().map(|p| 1 + p.holes.len()).sum();
    Ok(OffsetOutline {
        curves,
        origins,
        profiles: profiles.to_vec(),
        loops_before: loops,
        loops_after: loops,
        regions_before: profiles.len(),
        regions_after: profiles.len(),
        whole_up_to: None,
    })
}

/// The largest distance between 0 and `d` (same sign) at which the
/// offset has every loop and region the input has, and no more: by bisection, to a
/// thousandth of the tolerance.
///
/// A try that fails for a reason that is not the outline's (a cap on the
/// work, which a smaller distance does not lift) ends the search with that
/// error, and the callers report no distance rather than a wrong one.
fn largest_whole(
    profiles: &[Profile],
    d: f64,
    corners: Corners,
    tolerance: &Tolerance,
    search: Search,
) -> Result<f64, KernelError> {
    let whole = |t: f64| {
        if t.abs() <= tolerance.linear {
            return Ok(true);
        }
        match attempt(profiles, t, corners, tolerance, search) {
            Ok(o) => Ok(o.is_whole()),
            Err(
                e @ (KernelError::TooManyOffsetCurves { .. }
                | KernelError::CrowdedOffset { .. }
                | KernelError::TooManyCurves { .. }
                | KernelError::CrowdedCurves { .. }),
            ) => Err(e),
            Err(_) => Ok(false),
        }
    };
    let (mut lo, mut hi) = (0.0_f64, d.abs());
    let sign = d.signum();
    for _ in 0..200 {
        if hi - lo <= tolerance.linear * 1e-3 {
            break;
        }
        let mid = 0.5 * (lo + hi);
        if whole(sign * mid)? {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Ok(sign * lo)
}

/// The closed profiles offset by `d`: positive grows the material.
pub fn offset_profiles(
    profiles: &[Profile],
    d: f64,
    corners: Corners,
    tolerance: &Tolerance,
) -> Result<OffsetOutline, KernelError> {
    offset_with(profiles, d, corners, tolerance, Search::Indexed(BOUNDS))
}

fn offset_with(
    profiles: &[Profile],
    d: f64,
    corners: Corners,
    tolerance: &Tolerance,
    search: Search,
) -> Result<OffsetOutline, KernelError> {
    let tol = tolerance.linear;
    if !(tol.is_finite() && tol > 0.0) {
        return Err(KernelError::InvalidSize {
            name: "tolerance",
            value: tol,
        });
    }
    if !d.is_finite() || d.abs() > 1e100 {
        return Err(KernelError::InvalidSize {
            name: "distance",
            value: d,
        });
    }
    if profiles.is_empty() {
        return Err(KernelError::NoProfile);
    }
    if d.abs() <= tol {
        return unchanged(profiles);
    }
    // How far the outline reaches: half the diagonal of its box. Inward,
    // no point of a region lies further than that from its outline, so a
    // larger distance leaves nothing, whatever the shape.
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for b in profiles.iter().map(|p| p.outer.bbox()) {
        lo = [lo[0].min(b[0]), lo[1].min(b[1])];
        hi = [hi[0].max(b[2]), hi[1].max(b[3])];
    }
    let reach = 0.5 * (hi[0] - lo[0]).hypot(hi[1] - lo[1]);
    if !reach.is_finite() {
        return Err(KernelError::InvalidSize {
            name: "outline",
            value: reach,
        });
    }
    // Outward the outline grows to `reach + d` about its middle; the
    // arithmetic has to hold that to within the tolerance, and does while
    // the size stays below the tolerance over 1024 units in the last
    // place (a few products and a square root lose that much at most).
    let precise = tol / (1024.0 * f64::EPSILON);
    if d > 0.0 && reach + d > precise {
        return Err(KernelError::OffsetProfile {
            reason: "is too large for the document's tolerance to be held to it",
            curves: Vec::new(),
            largest: Some((precise - reach).max(0.0)),
        });
    }
    if d < -reach {
        let mut curves: Vec<u64> = profiles
            .iter()
            .flat_map(|p| p.outer.segments.iter().map(|s| s.source))
            .collect();
        curves.sort_unstable();
        curves.dedup();
        curves.truncate(MAX_REPORTED);
        return Err(KernelError::OffsetProfile {
            reason: "leaves nothing of a region",
            curves,
            largest: largest_whole(profiles, -reach, corners, tolerance, search).ok(),
        });
    }
    match attempt(profiles, d, corners, tolerance, search) {
        Ok(mut outline) => {
            if !outline.is_whole() {
                outline.whole_up_to = largest_whole(profiles, d, corners, tolerance, search).ok();
            }
            Ok(outline)
        }
        Err(KernelError::OffsetProfile {
            reason,
            curves,
            largest: None,
        }) if !reason.contains("not supported") && !reason.contains("turns back") => {
            Err(KernelError::OffsetProfile {
                reason,
                curves,
                largest: largest_whole(profiles, d, corners, tolerance, search).ok(),
            })
        }
        Err(other) => Err(other),
    }
}

/// The area a round offset of a convex polygon of area `a` and perimeter
/// `p` has: `a + p d + π d²` (Steiner). For tests and documentation.
pub fn steiner_area(a: f64, p: f64, d: f64) -> f64 {
    a + p * d + PI * d * d
}
