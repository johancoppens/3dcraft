//! How the two faces of an edge turn against each other along it, and the
//! places strictly inside the edge where a fillet's section runs to
//! nothing.
//!
//! **Why.** A fillet's section across the edge is the arc of the ball
//! between its two feet. Between the tangent planes of the two faces at a
//! point of the edge, which turn against each other by the angle φ there,
//! the feet lie `r·tan(φ/2)` from the edge: the section is `2r·tan(φ/2)`
//! wide, and it is nothing where φ is nothing. Where that happens at a
//! place inside the edge, with an angle on both sides of it, the fillet
//! face's two borders run through one point which is not a vertex: a face
//! pinched to nothing there, which is not a face. Where the edge turns from
//! convex to concave the faces are tangent at the place between, and past
//! it the ball that touches both lies on the other side of the body's
//! surface. Read at the edge's middle only, as it was before 2026-11-14,
//! such an edge is a convex or a concave one, and the construction, whose
//! samples step over the place, returned a face with no width there (and
//! with its borders crossed between two such places).
//!
//! **The rule.** The sine of the turn across the edge is read along it
//! ([`super::across_at`]: negative where the edge is convex), at
//! [`PER_SPAN`] stations per span of the edge's curve and at least
//! [`LEAST_STATIONS`], evenly in its parameter. A station is *tangent*
//! where the section between the tangent planes there is no wider than
//! the linear tolerance (two points that close are one point) or the sine
//! is no more than [`SMOOTH_SINE`], below which two faces are tangent
//! across an edge wherever the kernel asks; it is *opposite* where the
//! edge turns the other way than at its middle by more than that. Then:
//!
//! - a stretch of stations that are not turned like the middle and one of
//!   which is opposite: the edge is convex on one side and concave on the
//!   other of the place where the turn passes through zero, between the
//!   stretch and the station turned like the middle beside it (found by
//!   bisection; a place on each side of the stretch that has one);
//! - a stretch of tangent stations with stations turned like the middle
//!   on both sides of it: the place where the turn is least (the faces
//!   become tangent and part again, or come too close to it for the
//!   radius);
//! - a station turned like the middle that is lower than both its
//!   neighbours, and so near the bound that the turn between the stations
//!   could reach it, is looked at between them (a golden-section search
//!   for the least turn): a dip under the bound that the stations step
//!   over is found;
//! - a stretch of tangent stations that reaches an end of the edge is the
//!   edge's end, not a place inside it (a fillet that narrows towards the
//!   end of its edge is cut off there by the face it runs into, and is
//!   built as before), and so is a place within the linear tolerance of an
//!   end vertex.
//!
//! A fillet of an edge with such a place is refused, with the place. What
//! is not seen: a dip between two stations that leaves no station lower
//! than its neighbours (narrower than an eighth of a span of the curve and
//! on a slope). Nothing else of a blend reads this: an edge without such a
//! place is built exactly as it was.

use super::{SMOOTH_SINE, Use, across_at};
use crate::FilletPinch;
use craft3d_geom::Curve3;
use craft3d_math::Vec3;
use craft3d_topo::{Body, EdgeId};

/// Stations per span of the edge's curve, and the fewest and the most
/// along one edge.
const PER_SPAN: usize = 8;
const LEAST_STATIONS: usize = 64;
const MOST_STATIONS: usize = 2048;
/// Steps of a bisection and of a golden-section search between two
/// stations: each leaves less than a part in 1e12 of the step between
/// them.
const NARROWINGS: usize = 64;

/// A place strictly inside an edge where a fillet's section runs to
/// nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Pinch {
    pub at: Vec3,
    pub why: FilletPinch,
    /// How many such places there are along the edge; `at` is the first.
    pub places: usize,
}

/// How many stations an edge is read at: [`PER_SPAN`] per span of its
/// curve over `range`, [`LEAST_STATIONS`] at least.
fn stations(curve: &Curve3, range: (f64, f64)) -> usize {
    let spans = match curve {
        Curve3::Nurbs(nurbs) => {
            let mut inner = 0_usize;
            let mut last = f64::NEG_INFINITY;
            for &knot in nurbs.knots() {
                if knot > range.0 && knot < range.1 && knot > last {
                    inner = inner.saturating_add(1);
                    last = knot;
                }
            }
            inner.saturating_add(1)
        }
        Curve3::Line(_) | Curve3::Circle(_) | Curve3::Ellipse(_) => 1,
    };
    spans
        .saturating_mul(PER_SPAN)
        .clamp(LEAST_STATIONS, MOST_STATIONS)
}

/// The edge read along itself: the sine of the turn across it at a
/// parameter of its curve, times `sign`.
struct Reader<'a> {
    body: &'a Body,
    uses: &'a [Vec<Use>],
    edge: EdgeId,
    range: (f64, f64),
    /// A closed edge: parameters past its range go round.
    closed: bool,
    sign: f64,
}

impl Reader<'_> {
    fn at(&self, t: f64) -> Option<f64> {
        let (t0, t1) = self.range;
        let t = if self.closed && t1 > t0 {
            t0 + (t - t0).rem_euclid(t1 - t0)
        } else {
            t.clamp(t0, t1)
        };
        let found = across_at(self.body, self.uses, self.edge, t).ok()??;
        let value = found.sine * self.sign;
        value.is_finite().then_some(value)
    }
}

/// Where `g` passes through zero between `above` (where it is more) and
/// `below` (where it is not), by bisection.
fn zero(g: &impl Fn(f64) -> Option<f64>, above: f64, below: f64) -> f64 {
    let (mut a, mut b) = (above, below);
    for _ in 0..NARROWINGS {
        let middle = 0.5 * (a + b);
        match g(middle) {
            Some(value) if value > 0.0 => a = middle,
            Some(_) => b = middle,
            None => break,
        }
    }
    0.5 * (a + b)
}

/// Where `g` is least between `a` and `b`, and its value there: a
/// golden-section search, which finds the bottom of the one dip between
/// three stations.
fn least(g: &impl Fn(f64) -> Option<f64>, a: f64, b: f64) -> Option<(f64, f64)> {
    // (√5 − 1) / 2.
    const RATIO: f64 = 0.618_033_988_749_894_9;
    let (mut a, mut b) = (a, b);
    let mut c = b - RATIO * (b - a);
    let mut d = a + RATIO * (b - a);
    let (mut gc, mut gd) = (g(c)?, g(d)?);
    for _ in 0..NARROWINGS {
        if gc < gd {
            b = d;
            d = c;
            gd = gc;
            c = b - RATIO * (b - a);
            gc = g(c)?;
        } else {
            a = c;
            c = d;
            gc = gd;
            d = a + RATIO * (b - a);
            gd = g(d)?;
        }
    }
    Some(if gc < gd { (c, gc) } else { (d, gd) })
}

/// The sine of the turn at which a fillet of `radius` is as wide as `tol`
/// between the tangent planes (`2r·tan(φ/2) = tol`), [`SMOOTH_SINE`] at
/// least.
fn narrow_sine(radius: f64, tol: f64) -> f64 {
    let half_tan = 0.5 * tol / radius;
    let sine = 2.0 * half_tan / (1.0 + half_tan * half_tan);
    if sine.is_finite() {
        sine.max(SMOOTH_SINE)
    } else {
        SMOOTH_SINE
    }
}

/// The places strictly inside `edge` where the section of a fillet of
/// `radius` runs to nothing (see the module documentation): the first
/// along the edge and how many there are. `convex` is what the edge is at
/// its middle. `None` for an edge without one, and for one that cannot be
/// read.
pub(super) fn pinched(
    body: &Body,
    uses: &[Vec<Use>],
    edge: EdgeId,
    convex: bool,
    radius: f64,
    tol: f64,
) -> Option<Pinch> {
    let data = body.edge(edge)?;
    let (t0, t1) = data.range;
    if !(t0.is_finite() && t1.is_finite() && t1 > t0 && radius > 0.0 && tol > 0.0) {
        return None;
    }
    let closed = data.start == data.end;
    let reader = Reader {
        body,
        uses,
        edge,
        range: data.range,
        closed,
        // Positive where the edge turns as at its middle.
        sign: if convex { -1.0 } else { 1.0 },
    };
    let g = |t: f64| reader.at(t);
    let bound = narrow_sine(radius, tol);
    let count = stations(&data.curve, data.range);
    let step = (t1 - t0) / count as f64;
    // The stations that can be read, in order. A closed edge is read from
    // the station where it turns most, once round and that station again:
    // every stretch then has a station turned like the middle on both
    // sides.
    let mut read: Vec<(f64, f64)> = Vec::with_capacity(count + 1);
    for i in 0..=count {
        if closed && i == count {
            break;
        }
        let t = if i == count { t1 } else { t0 + step * i as f64 };
        if let Some(value) = g(t) {
            read.push((t, value));
        }
    }
    if closed {
        let (first, _) = read
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.1.total_cmp(&b.1.1))?;
        read.rotate_left(first);
        let (t, value) = *read.first()?;
        for later in read.iter_mut().skip(1) {
            if later.0 < t {
                later.0 += t1 - t0;
            }
        }
        read.push((t + (t1 - t0), value));
    }
    let last = read.len().checked_sub(1)?;
    let turned = |value: f64| value > bound;
    let why = |value: f64| {
        if value <= SMOOTH_SINE {
            FilletPinch::Tangent
        } else {
            // Between the tangent planes, at the angle of this sine.
            let cosine = (1.0 - value * value).max(0.0).sqrt();
            FilletPinch::Narrow {
                width: 2.0 * radius * value / (1.0 + cosine),
                tolerance: tol,
            }
        }
    };
    // The places of a dip between `before` and `after` (where the edge is
    // turned like its middle) whose bottom is `lowest`, at `bottom`: where
    // the turn passes through zero on the way down and on the way up when
    // it goes below it, else the bottom when that is under the bound.
    let dip = |before: f64, bottom: f64, lowest: f64, after: f64| -> Vec<(f64, FilletPinch)> {
        if lowest < -SMOOTH_SINE {
            vec![
                (zero(&g, before, bottom), FilletPinch::Flips),
                (zero(&g, after, bottom), FilletPinch::Flips),
            ]
        } else if turned(lowest) {
            Vec::new()
        } else {
            vec![(bottom, why(lowest))]
        }
    };
    let mut places: Vec<(f64, FilletPinch)> = Vec::new();
    let mut i = 0;
    while i <= last {
        let (_, value) = *read.get(i)?;
        if turned(value) {
            // Lower than both neighbours, and near enough to the bound for
            // the turn between the stations to reach it: the bottom of a
            // parabola through three stations lies below the lowest of
            // them by at most an eighth of what the higher neighbour is
            // above it; eight times that is allowed for.
            if i > 0 && i < last {
                let (before, left) = *read.get(i - 1)?;
                let (after, right) = *read.get(i + 1)?;
                let drop = (left - value).max(right - value);
                if turned(left)
                    && turned(right)
                    && value <= left
                    && value <= right
                    && drop > 0.0
                    && value - drop <= bound
                    && let Some((bottom, lowest)) = least(&g, before, after)
                {
                    places.extend(dip(before, bottom, lowest, after));
                }
            }
            i += 1;
            continue;
        }
        // A stretch of stations not turned like the middle.
        let from = i;
        let mut to = i;
        while to < last && read.get(to + 1).is_some_and(|next| !turned(next.1)) {
            to += 1;
        }
        let stretch = read.get(from..=to)?;
        let opposite = |value: f64| value < -SMOOTH_SINE;
        if stretch.iter().any(|s| opposite(s.1)) {
            // Convex on one side and concave on the other: where the turn
            // passes through zero, into the stretch and out of it.
            if let (Some(before), Some(first)) = (
                from.checked_sub(1).and_then(|b| read.get(b)),
                stretch.iter().find(|s| opposite(s.1)),
            ) {
                places.push((zero(&g, before.0, first.0), FilletPinch::Flips));
            }
            if let (Some(after), Some(final_one)) =
                (read.get(to + 1), stretch.iter().rfind(|s| opposite(s.1)))
            {
                places.push((zero(&g, after.0, final_one.0), FilletPinch::Flips));
            }
        } else if from > 0 && to < last {
            // Tangent, or nearly, with an angle on both sides: where the
            // turn is least.
            let (mut bottom, mut lowest) =
                stretch.iter().copied().min_by(|a, b| a.1.total_cmp(&b.1))?;
            let (before, after) = (read.get(from - 1)?.0, read.get(to + 1)?.0);
            let around = ((bottom - step).max(before), (bottom + step).min(after));
            if let Some((t, value)) = least(&g, around.0, around.1)
                && value < lowest
            {
                (bottom, lowest) = (t, value);
            }
            places.extend(dip(before, bottom, lowest, after));
        }
        // A stretch that reaches an end of the edge is the edge's end.
        i = to + 1;
    }
    // A place within the tolerance of an end vertex is that end.
    let ends = [data.curve.eval(t0), data.curve.eval(t1)];
    let mut inside = places.into_iter().filter_map(|(t, why)| {
        let t = if closed {
            t0 + (t - t0).rem_euclid(t1 - t0)
        } else {
            t.clamp(t0, t1)
        };
        let at = data.curve.eval(t);
        let clear = closed || ends.iter().all(|end| at.distance(*end) > tol);
        (at.is_finite() && clear).then_some((at, why))
    });
    let (at, why) = inside.next()?;
    Some(Pinch {
        at,
        why,
        places: 1 + inside.count(),
    })
}

/// For an edge whose faces are tangent at its middle: whether they are so
/// only there, not all along it (at some station the sine of the turn is
/// more than [`SMOOTH_SINE`]); then the middle is a place inside the edge
/// where the faces are tangent, with whether the edge is convex at one
/// station and concave at another. `None` for an edge tangent at every
/// station (and for one that cannot be read): an edge with no corner.
pub(super) fn tangent_at_middle(body: &Body, uses: &[Vec<Use>], edge: EdgeId) -> Option<Pinch> {
    let data = body.edge(edge)?;
    let (t0, t1) = data.range;
    if !(t0.is_finite() && t1.is_finite() && t1 > t0) {
        return None;
    }
    let reader = Reader {
        body,
        uses,
        edge,
        range: data.range,
        closed: false,
        sign: 1.0,
    };
    let count = stations(&data.curve, data.range);
    let step = (t1 - t0) / count as f64;
    let (mut convex, mut concave) = (false, false);
    for i in 0..=count {
        let t = if i == count { t1 } else { t0 + step * i as f64 };
        match reader.at(t) {
            Some(sine) if sine < -SMOOTH_SINE => convex = true,
            Some(sine) if sine > SMOOTH_SINE => concave = true,
            _ => {}
        }
    }
    if !(convex || concave) {
        return None;
    }
    let at = data.curve.eval(0.5 * (t0 + t1));
    at.is_finite().then_some(Pinch {
        at,
        why: if convex && concave {
            FilletPinch::Flips
        } else {
            FilletPinch::Tangent
        },
        places: 1,
    })
}
