//! Curves side by side (2026-11-22, docs/design/booleans.md › *A step of
//! the march onto the curve beside the one it follows*).
//!
//! A step of the march is held to how far it goes, to the turn of the
//! tangent and to ending on the curve. None of that tells a step along a
//! curve from one that lands on another curve beside it that heads the
//! same way; the fit does, finding no points between two points that are
//! not neighbours on one curve, and such a pair was refused, "an
//! intersection curve could not be fitted within the tolerance". Since
//! 2026-11-22 the march is taken up again from the point before such a
//! span, in shorter steps.
//!
//! Three families of a flat surface against another, each at three
//! scales, in the world's axes and askew to them; what the crossing is,
//! is known for each without the marcher.
//!
//! - A corrugated sheet under a plane level with its crests that rises a
//!   little along them: a hairpin over each crest, side by side, all
//!   beginning and ending on the sheet's near edge. This is the family
//!   that was refused: a march down a side of a hairpin leaves the sheet
//!   in its last step, and that step is put on the edge where the
//!   crossing is found from where the step pointed, which with a step
//!   some way past the edge is the foot of another hairpin (freeform: the
//!   sheet is walked). The other case found is in `meet_shallow.rs`, the
//!   wing's skin under its spar: a step past the turn of a hairpin onto
//!   a later stretch of the same curve.
//! - A torus and a plane along its axis near the plane that touches its
//!   inner equator: two ovals whose tips face each other a short way
//!   apart where the plane is nearer the axis, one curve with a narrow
//!   waist where it is further (analytic: the plane is walked, the torus
//!   measured against); a fat torus, and a ring three thousandths as
//!   thick as it is wide, whose ovals leave their tips at 3 degrees to
//!   the line between them.
//! - A rippled ridge, a spline surface whose crest line rises and falls,
//!   cut by a plane level with the saddles between its crests, or with
//!   the crests themselves: a row of loops, tip to tip (freeform); a
//!   broad ridge, and a sharp one whose loops are a twenty-fifth as wide
//!   as they are long.
//!
//! The last two were answered before the march was taken up again, by
//! the same steps (no step of theirs was found on another curve than it
//! began on), and are here so that they stay answered. As read from the
//! sign of the distance, not traced: about a saddle of the distance
//! between the surfaces the tips of the two curves face each other, but
//! the side of the one that lies straight on from a side of the other is
//! travelled the other way (the distance has the other sign beside it),
//! so a step that lands there has turned back and is not taken, and the
//! march goes round each tip.
//!
//! The curves are held to the crossing itself: every point of a curve has
//! a change of sign of the distance between the surfaces within the
//! tolerance of it, every point of the crossing is on a curve, and there
//! is one curve to each curve of the crossing: once round the inside of a
//! closed one and round no other (a curve that joins two of them, or goes
//! round twice, is not), or by one open strand all its length.

// Test code: a failure in a helper is a failed test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
// `!(x <= y)` is deliberate: unlike `x > y` it is also true for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

mod common;

use common::*;
use craft3d_geom::intersect::{MeetingCurve, SurfaceMeeting, meet_surfaces, meet_surfaces_once};
use craft3d_geom::{Curve3, NurbsSurface, Surface};
use craft3d_math::{Bbox3, Frame, Portable, Vec3};

/// A curve that came back, at eight places to each of its own spans (it
/// has more spans where it bends).
struct Sampled {
    curve: Curve3,
    at: Vec<f64>,
    points: Vec<Vec3>,
}

fn sampled(curve: &MeetingCurve) -> Sampled {
    let Curve3::Nurbs(spline) = &curve.curve else {
        panic!("a marched curve is a spline: {curve:?}");
    };
    let (t0, t1) = curve.range;
    let mut knots = vec![t0];
    knots.extend(
        spline
            .knots()
            .iter()
            .copied()
            .filter(|k| *k > t0 && *k < t1),
    );
    knots.push(t1);
    knots.dedup();
    let mut at = Vec::new();
    for w in knots.windows(2) {
        for s in 0..8 {
            at.push(w[0] + (w[1] - w[0]) * f64::from(s) / 8.0);
        }
    }
    at.push(t1);
    let points = at.iter().map(|t| spline.eval(*t)).collect();
    Sampled {
        curve: curve.curve.clone(),
        at,
        points,
    }
}

/// The least distance from `p` to the curve between two of its
/// parameters, by golden sections.
fn least(s: &Sampled, p: Vec3, mut lo: f64, mut hi: f64) -> f64 {
    for _ in 0..60 {
        let (m1, m2) = (lo + (hi - lo) * 0.382, lo + (hi - lo) * 0.618);
        if s.curve.eval(m1).distance(p) <= s.curve.eval(m2).distance(p) {
            hi = m2;
        } else {
            lo = m1;
        }
    }
    s.curve.eval(0.5 * (lo + hi)).distance(p)
}

/// How far `p` is from the curve: the least about every one of its
/// samples that is nearer `p` than the samples before and after it. (Not
/// about the nearest sample alone: the two sides of a narrow loop are
/// nearer each other than a side's samples are, and the nearest sample
/// is then on the other side as often as not.)
fn off(s: &Sampled, p: Vec3) -> f64 {
    let last = s.at.len() - 1;
    let far: Vec<f64> = s.points.iter().map(|q| q.distance(p)).collect();
    let mut found = f64::INFINITY;
    for (i, here) in far.iter().enumerate() {
        let before = if i == 0 { f64::INFINITY } else { far[i - 1] };
        let after = if i == last { f64::INFINITY } else { far[i + 1] };
        if *here <= before && *here <= after {
            let about = least(s, p, s.at[i.saturating_sub(1)], s.at[(i + 1).min(last)]);
            found = found.min(*here).min(about);
        }
    }
    found
}

/// How many times the curve goes round `centre`, in the plane of `e1`
/// and `e2`.
fn winding(s: &Sampled, centre: Vec3, e1: Vec3, e2: Vec3) -> f64 {
    let (pi, tau) = (std::f64::consts::PI, std::f64::consts::TAU);
    let angle = |p: Vec3| (p - centre).dot(e2).patan2((p - centre).dot(e1));
    let mut total = 0.0;
    for w in s.points.windows(2) {
        let mut turned = angle(w[1]) - angle(w[0]);
        if turned > pi {
            turned -= tau;
        } else if turned < -pi {
            turned += tau;
        }
        total += turned;
    }
    total / tau
}

/// A flat surface and another whose crossing is known without the
/// marcher.
struct Pair {
    what: String,
    k: f64,
    flat: Surface,
    other: Surface,
    within: Bbox3,
    /// A point of the plane and two unit directions in it, square to
    /// each other.
    origin: Vec3,
    e1: Vec3,
    e2: Vec3,
    /// The distance from a point of the plane to the other surface,
    /// signed, by that surface's own numbers.
    gap: Box<dyn Fn(Vec3) -> f64>,
    /// Points of the crossing.
    crossing: Vec<Vec3>,
    /// The curves of the crossing.
    curves: Curves,
}

/// The curves a crossing is made of.
enum Curves {
    /// Closed curves: a point inside each.
    Loops(Vec<Vec3>),
    /// Open curves, so many: which of them a place near the crossing is
    /// by.
    Strands(usize, Box<dyn Fn(Vec3) -> usize>),
}

/// The pair's curves as they should come back: closed or open as the
/// crossing's are, every point of them within the tolerance of a change
/// of sign of the distance between the surfaces, every point of the
/// crossing on a curve, and one curve to each curve of the crossing: once
/// round the inside of a closed one and round no other, or every point of
/// it by one open strand and no other curve by that strand. `Ok` with how
/// many spans they have.
fn check(pair: &Pair) -> Result<usize, String> {
    let tolerance = tol(pair.k);
    let t = tolerance.linear;
    let meeting = match meet_surfaces(&pair.flat, &pair.other, &pair.within, &tolerance) {
        Ok(SurfaceMeeting::Meets(m)) => m,
        Ok(other) => return Err(format!("no curve: {other:?}")),
        Err(e) => return Err(format!("refused: {e}")),
    };
    if !meeting.points.is_empty() {
        return Err(format!("{} points", meeting.points.len()));
    }
    let normal = pair.e1.cross(pair.e2);
    let mut curves = Vec::new();
    for (n, curve) in meeting.curves.iter().enumerate() {
        let loops = matches!(pair.curves, Curves::Loops(_));
        if curve.closed != loops || curve.tangent || !(curve.deviation <= t) {
            return Err(format!(
                "curve {n} of {}: closed {}, tangent {}, deviation {}",
                meeting.curves.len(),
                curve.closed,
                curve.tangent,
                curve.deviation
            ));
        }
        let s = sampled(curve);
        for (i, p) in s.points.iter().enumerate() {
            let above = (*p - pair.origin).dot(normal);
            if !(above.abs() <= t) {
                return Err(format!("curve {n}: point {i} is off the plane"));
            }
            // The crossing itself within the tolerance, in the plane.
            let q = *p - normal * above;
            let near = [
                (pair.gap)(q),
                (pair.gap)(q + pair.e1 * t),
                (pair.gap)(q - pair.e1 * t),
                (pair.gap)(q + pair.e2 * t),
                (pair.gap)(q - pair.e2 * t),
            ];
            let low = near.iter().copied().fold(f64::INFINITY, f64::min);
            let high = near.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            if !(low <= 0.0 && high >= 0.0) {
                return Err(format!(
                    "curve {n}: point {i} of {} is not within the tolerance of the crossing: the surfaces are {low:e} to {high:e} apart about it",
                    s.points.len()
                ));
            }
        }
        curves.push(s);
    }
    let expected = match &pair.curves {
        Curves::Loops(insides) => insides.len(),
        Curves::Strands(count, _) => *count,
    };
    if curves.len() != expected {
        return Err(format!(
            "{} curves where the crossing is {expected}",
            curves.len()
        ));
    }
    match &pair.curves {
        Curves::Loops(insides) => {
            // Each curve of the crossing has one curve once round its
            // inside, and each curve is round one.
            let round: Vec<Vec<f64>> = insides
                .iter()
                .map(|c| {
                    curves
                        .iter()
                        .map(|s| winding(s, *c, pair.e1, pair.e2))
                        .collect()
                })
                .collect();
            let once = |w: f64| (w.abs() - 1.0).abs() < 0.01;
            let never = |w: f64| w.abs() < 0.01;
            let right = round.iter().all(|row| {
                row.iter().filter(|w| once(**w)).count() == 1
                    && row.iter().all(|w| once(*w) || never(*w))
            }) && (0..curves.len())
                .all(|n| round.iter().filter(|row| once(row[n])).count() == 1);
            if !right {
                return Err(format!(
                    "the curves go round the insides of the crossing's curves {round:?} times"
                ));
            }
        }
        Curves::Strands(count, by) => {
            // Each curve is by one strand all its length, and no two
            // curves by the same.
            let mut taken = vec![false; *count];
            for (n, s) in curves.iter().enumerate() {
                let strand = by(s.points[0]);
                if let Some(i) = s.points.iter().position(|p| by(*p) != strand) {
                    return Err(format!(
                        "curve {n} is by strand {strand} of the crossing at its start and by strand {} at point {i} of {}",
                        by(s.points[i]),
                        s.points.len()
                    ));
                }
                if strand >= *count || taken[strand] {
                    return Err(format!("two curves by strand {strand} of the crossing"));
                }
                taken[strand] = true;
            }
        }
    }
    for (n, p) in pair.crossing.iter().enumerate() {
        let gap = curves
            .iter()
            .map(|s| off(s, *p))
            .fold(f64::INFINITY, f64::min);
        if !(gap <= t) {
            return Err(format!(
                "point {n} of {} of the crossing is {:.3} tolerances from every curve",
                pair.crossing.len(),
                gap / t
            ));
        }
    }
    Ok(curves.iter().map(|s| s.at.len() / 8).sum())
}

/// Checks every pair and fails with the list of those that did not come
/// back as they should.
fn all_come_back(pairs: Vec<Pair>) {
    let clock = std::time::Instant::now();
    let (mut failed, mut spans) = (Vec::new(), 0);
    for pair in &pairs {
        match check(pair) {
            Ok(n) => spans += n,
            Err(why) => failed.push(format!("{}: {why}", pair.what)),
        }
    }
    eprintln!(
        "{} pairs in {:.1} s, {} failed; {spans} spans",
        pairs.len(),
        clock.elapsed().as_secs_f64(),
        failed.len()
    );
    assert!(
        failed.is_empty(),
        "{} of {} pairs:\n{}",
        failed.len(),
        pairs.len(),
        failed.join("\n")
    );
}

/// The frames a family is made in: the world's own and one askew to it.
fn frames(k: f64) -> [(&'static str, Frame); 2] {
    [("upright", Frame::WORLD), ("askew", tilted(k))]
}

/// A torus about the z axis of `frame` (radii 10 and `tube`, at the
/// scale `k`) and a plane along its axis, `inside` nearer the axis than
/// the plane that touches the inner equator (negative: further out).
///
/// In the plane, `y` across and `z` along the axis, the torus is
/// `(√(d² + y²) − 10)² + z² = tube²`, `d` the plane's distance from the
/// axis: about the point of the inner equator the distance to it is
/// `inside − y² / 2d + z² / 2 tube` to second order, a saddle. Nearer the
/// axis the crossing is two ovals whose tips are `√(2 d · inside)` either
/// side of that point, and whose sides leave the tips at a slope of
/// `√(tube / d)` to the line between them; further out one curve with a
/// waist there.
fn pinched(k: f64, name: &str, frame: Frame, tube: f64, inside: f64) -> Pair {
    let (major, minor) = (10.0 * k, tube * k);
    let d = major - minor - inside;
    let origin = frame.point(d, 0.0, 0.0);
    let gap = move |p: Vec3| {
        let (x, y, z) = frame.local(p);
        let from_circle = (x * x + y * y).sqrt() - major;
        (from_circle * from_circle + z * z).sqrt() - minor
    };
    // Round the tube: each oval once (both, where the plane is nearer
    // the axis than the inner equator), or the part of the one curve
    // on each side.
    let mut crossing = Vec::new();
    for i in 0..2000 {
        let (sine, cosine) = (std::f64::consts::TAU * f64::from(i) / 2000.0).psin_cos();
        let (from_axis, z) = (major + minor * cosine, minor * sine);
        if from_axis > d {
            let y = (from_axis * from_axis - d * d).sqrt();
            crossing.push(frame.point(d, y, z));
            crossing.push(frame.point(d, -y, z));
        }
    }
    let middle = (major * major - d * d).sqrt();
    let insides = if inside > 0.0 {
        vec![frame.point(d, middle, 0.0), frame.point(d, -middle, 0.0)]
    } else {
        vec![frame.point(d, middle, 0.0)]
    };
    let curves = Curves::Loops(insides);
    Pair {
        what: format!(
            "torus of tube {tube}, scale {k}, {name}, the plane {:e} tolerances inside the inner tangent plane",
            inside / tol(k).linear
        ),
        k,
        flat: plane(Frame::new(origin, frame.x(), frame.y()).unwrap()),
        other: torus(frame, major, minor),
        within: cube(
            origin,
            1.5 * ((major + minor) * (major + minor) - d * d).sqrt(),
        ),
        origin,
        e1: frame.y(),
        e2: frame.z(),
        gap: Box::new(gap),
        crossing,
        curves,
    }
}

/// The heights of the ridge's crest line at its control points: five
/// crests with four saddles between them, and its ends well below.
const CREST_LINE: [f64; 11] = [-0.6, 0.3, 0.0, 0.3, 0.0, 0.3, 0.0, 0.3, 0.0, 0.3, -0.6];
const RIDGE_LENGTH: f64 = 8.0;

/// A rippled ridge along the y axis of `frame`, at the scale `k`:
/// `z = s(y) − depth (x / half)²` over `x` in `±half` and `y` in 0..8,
/// with `s` the cubic spline of [`CREST_LINE`] over uniform knots (a
/// spline surface of degree two across and three along, `x` and `y`
/// linear in its parameters).
fn ridge(k: f64, frame: Frame, half: f64, depth: f64) -> Surface {
    let mut knots = vec![0.0; 4];
    knots.extend((1..8).map(|i| f64::from(i) / 8.0));
    knots.extend([1.0; 4]);
    let mut ctrl = Vec::new();
    for (i, x) in [-half, 0.0, half].into_iter().enumerate() {
        for (j, crest) in CREST_LINE.iter().enumerate() {
            let along = (knots[j + 1] + knots[j + 2] + knots[j + 3]) / 3.0;
            let z = if i == 1 { crest + depth } else { crest - depth };
            ctrl.push(frame.point(x * k, RIDGE_LENGTH * along * k, z * k));
        }
    }
    Surface::Nurbs(
        NurbsSurface::new(
            2,
            3,
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            knots,
            3,
            11,
            ctrl,
            vec![1.0; 33],
        )
        .unwrap(),
    )
}

/// Where the plane that cuts the ridge is.
#[derive(Clone, Copy, Debug)]
enum Level {
    /// So far above the lowest saddle of the crest line: loops whose
    /// tips face each other across it.
    AboveSaddle(f64),
    /// So far below it: the loops either side of it are one, with a
    /// waist.
    BelowSaddle(f64),
    /// So far below the lowest crest: a small loop about it.
    BelowCrest(f64),
}

/// The least (`sign` 1) or greatest (−1) of `f` between `lo` and `hi`,
/// by golden sections.
fn extreme(f: &dyn Fn(f64) -> f64, mut lo: f64, mut hi: f64, sign: f64) -> f64 {
    for _ in 0..200 {
        let (m1, m2) = (lo + (hi - lo) * 0.382, lo + (hi - lo) * 0.618);
        if sign * f(m1) <= sign * f(m2) {
            hi = m2;
        } else {
            lo = m1;
        }
    }
    f(0.5 * (lo + hi))
}

/// The ridge and a level plane across it (`level` above or below a saddle
/// or a crest of its crest line, in tolerances).
///
/// The crossing is `depth (x / half)² = s(y) − level` wherever the crest
/// line is above the plane: a closed loop over each such stretch of `y`.
/// Everything here is read from the surface's own evaluation: the crest
/// line along the middle of its parameter across, its saddles and
/// crests, the stretches, and the height at any place.
fn rippled(k: f64, name: &str, frame: Frame, (half, depth): (f64, f64), level: Level) -> Pair {
    let t = tol(k).linear;
    let surface = ridge(k, frame, half, depth);
    let crest = {
        let surface = surface.clone();
        move |v: f64| frame.local(surface.eval(0.5, v)).2
    };
    // The saddles and crests of the crest line: its interior extremes.
    let n = 8000;
    let at = |i: i32| f64::from(i) / f64::from(n);
    let (mut saddle, mut top) = (f64::INFINITY, f64::INFINITY);
    for i in 1..n {
        let (before, here, after) = (crest(at(i - 1)), crest(at(i)), crest(at(i + 1)));
        if here < before && here <= after {
            saddle = saddle.min(extreme(&crest, at(i - 1), at(i + 1), 1.0));
        }
        if here > before && here >= after {
            top = top.min(extreme(&crest, at(i - 1), at(i + 1), -1.0));
        }
    }
    assert!(saddle.is_finite() && top.is_finite());
    let (height, described) = match level {
        Level::AboveSaddle(by) => (saddle + by * t, format!("{by} tolerances above a saddle")),
        Level::BelowSaddle(by) => (saddle - by * t, format!("{by} tolerances below a saddle")),
        Level::BelowCrest(by) => (top - by * t, format!("{by} tolerances below a crest")),
    };
    // The stretches of the crest line above the plane, each from where
    // it rises through the plane to where it falls back (by halving).
    let over = |v: f64| crest(v) - height;
    let through = |mut lo: f64, mut hi: f64| {
        for _ in 0..200 {
            let mid = 0.5 * (lo + hi);
            if (over(mid) > 0.0) == (over(lo) > 0.0) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        0.5 * (lo + hi)
    };
    assert!(over(0.0) < 0.0 && over(1.0) < 0.0);
    let mut stretches: Vec<(f64, f64)> = Vec::new();
    let mut rose = None;
    for i in 1..=n {
        let (before, here) = (over(at(i - 1)), over(at(i)));
        if before <= 0.0 && here > 0.0 {
            rose = Some(through(at(i - 1), at(i)));
        } else if before > 0.0 && here <= 0.0 {
            stretches.push((rose.take().unwrap(), through(at(i), at(i - 1))));
        }
    }
    // The crossing over each stretch, closer together towards its tips,
    // and a point inside each loop: in the plane, under the highest
    // place of the crest line over the stretch (where the loop is
    // widest, not at a waist).
    let place = |x: f64, v: f64| frame.point(x, RIDGE_LENGTH * k * v, height);
    let (mut crossing, mut insides) = (Vec::new(), Vec::new());
    for (from, to) in &stretches {
        for i in 0..=400 {
            let s = f64::from(i) / 400.0;
            let v = from + (to - from) * s * s * (3.0 - 2.0 * s);
            // `z = s(y) − depth (x / half)²`, at the scale `k`.
            let x = half * k * (over(v).max(0.0) / (depth * k)).sqrt();
            crossing.push(place(x, v));
            crossing.push(place(-x, v));
        }
        let mut widest = 0.5 * (from + to);
        for i in 0..=n {
            if at(i) > *from && at(i) < *to && over(at(i)) > over(widest) {
                widest = at(i);
            }
        }
        insides.push(place(0.0, widest));
    }
    let curves = Curves::Loops(insides);
    let origin = frame.point(0.0, 0.0, height);
    let gap = {
        let surface = surface.clone();
        move |p: Vec3| {
            let (x, y, _) = frame.local(p);
            let u = (0.5 * (x / (half * k) + 1.0)).clamp(0.0, 1.0);
            let v = (y / (RIDGE_LENGTH * k)).clamp(0.0, 1.0);
            frame.local(surface.eval(u, v)).2 - height
        }
    };
    Pair {
        what: format!(
            "ridge {half} to either side and {depth} deep, scale {k}, {name}, the plane {described}"
        ),
        k,
        flat: plane(Frame::new(origin, frame.z(), frame.x()).unwrap()),
        other: surface,
        within: cube(frame.point(0.0, 0.5 * RIDGE_LENGTH * k, 0.0), 5.0 * k),
        origin,
        e1: frame.x(),
        e2: frame.y(),
        gap: Box::new(gap),
        crossing,
        curves,
    }
}

/// The heights of a corrugated sheet's profile at its control points:
/// five crests of uneven height, the troughs between them low, and its
/// ends well below. (By the spline the three crests in the middle are
/// 0.16 to 0.21 high and the troughs 0.09 to 0.11.)
const CRESTS: [f64; 11] = [-0.6, 0.30, 0.0, 0.24, 0.0, 0.32, 0.0, 0.26, 0.0, 0.31, -0.6];
/// The same with the trough between the third crest and the fourth high,
/// 0.18: a shoulder between them.
const SHOULDER: [f64; 11] = [
    -0.6, 0.30, 0.0, 0.24, 0.0, 0.32, 0.12, 0.26, 0.0, 0.31, -0.6,
];
const SHEET_WIDTH: f64 = 10.0;
const SHEET_LENGTH: f64 = 100.0;

/// A corrugated sheet and a plane level with its crests that rises a
/// little along them, at the scale `k`. The sheet: the cubic spline
/// `p(x)` of `heights` over `x` in 0..10, drawn out along `y` over
/// 0..100 to the same profile at `taper` of its height (a ruled spline
/// surface: a loft between two sections, as a wing's skin is). The plane:
/// `z = from + rise · y / 100`, above every crest at `y = 100`.
///
/// Over a crest the plane crosses the sheet in a hairpin: up one side of
/// the crest from the sheet's edge at `y = 0`, round where the plane is
/// level with the crest line, and back down the other side. Where the
/// trough between two crests is above the plane at the edge, the two
/// hairpins are one curve, which turns once more where the plane is
/// level with the trough. The curves lie side by side, their stretches
/// 1 to 2.5 apart, and turn at different heights, the crests and troughs
/// being uneven. In the sheet's own numbers the crossing is `y / 100 =
/// (p(x) − from) / (rise + (1 − taper) p(x))` wherever that is positive,
/// with `p` read from the surface's evaluation along its first section.
fn corrugated(
    k: f64,
    name: &str,
    frame: Frame,
    heights: &[f64; 11],
    taper: f64,
    (from, rise): (f64, f64),
) -> Pair {
    let mut knots = vec![0.0; 4];
    knots.extend((1..8).map(|i| f64::from(i) / 8.0));
    knots.extend([1.0; 4]);
    let mut ctrl = Vec::new();
    for (i, height) in heights.iter().enumerate() {
        let across = (knots[i + 1] + knots[i + 2] + knots[i + 3]) / 3.0;
        let x = SHEET_WIDTH * across * k;
        ctrl.push(frame.point(x, 0.0, height * k));
        ctrl.push(frame.point(x, SHEET_LENGTH * k, height * taper * k));
    }
    let surface = Surface::Nurbs(
        NurbsSurface::new(
            3,
            1,
            knots,
            vec![0.0, 0.0, 1.0, 1.0],
            11,
            2,
            ctrl,
            vec![1.0; 22],
        )
        .unwrap(),
    );
    // The profile along the first section, and how far along the sheet
    // the plane crosses it there.
    let profile = {
        let surface = surface.clone();
        move |u: f64| frame.local(surface.eval(u, 0.0)).2 / k
    };
    let along = |u: f64| (profile(u) - from) / (rise + (1.0 - taper) * profile(u));
    // The stretches of the profile above the plane's lowest line: one
    // hairpin over each, from where the profile rises through that line
    // to where it falls back (by halving).
    let through = |mut lo: f64, mut hi: f64| {
        for _ in 0..200 {
            let mid = 0.5 * (lo + hi);
            if (along(mid) > 0.0) == (along(lo) > 0.0) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        0.5 * (lo + hi)
    };
    let n = 8000;
    let at = |i: i32| f64::from(i) / f64::from(n);
    assert!(along(0.0) < 0.0 && along(1.0) < 0.0);
    let mut stretches: Vec<(f64, f64)> = Vec::new();
    let mut rose = None;
    for i in 1..=n {
        let (before, here) = (along(at(i - 1)), along(at(i)));
        assert!(here < 0.99, "the plane leaves the sheet's far end");
        if before <= 0.0 && here > 0.0 {
            rose = Some(through(at(i - 1), at(i)));
        } else if before > 0.0 && here <= 0.0 {
            stretches.push((rose.take().unwrap(), through(at(i), at(i - 1))));
        }
    }
    let slope = rise / SHEET_LENGTH;
    let place = |u: f64, v: f64| {
        let y = SHEET_LENGTH * k * v;
        frame.point(SHEET_WIDTH * k * u, y, from * k + slope * y)
    };
    let mut crossing = Vec::new();
    for (start, end) in &stretches {
        for i in 0..=600 {
            let u = start + (end - start) * f64::from(i) / 600.0;
            crossing.push(place(u, along(u).max(0.0)));
        }
    }
    // The strand a place is by: the stretch its `x` is in, or nearest.
    let by = {
        let stretches = stretches.clone();
        move |p: Vec3| {
            let u = frame.local(p).0 / (SHEET_WIDTH * k);
            let apart = |(a, b): (f64, f64)| (a - u).max(u - b).max(0.0);
            let mut nearest = 0;
            for (i, stretch) in stretches.iter().enumerate() {
                if apart(*stretch) < apart(stretches[nearest]) {
                    nearest = i;
                }
            }
            nearest
        }
    };
    let origin = frame.point(0.0, 0.0, from * k);
    let (e1, e2) = (
        frame.x(),
        frame.vector(0.0, 1.0, slope).normalized().unwrap(),
    );
    let gap = {
        let surface = surface.clone();
        move |p: Vec3| {
            let (x, y, z) = frame.local(p);
            let u = (x / (SHEET_WIDTH * k)).clamp(0.0, 1.0);
            let v = (y / (SHEET_LENGTH * k)).clamp(0.0, 1.0);
            frame.local(surface.eval(u, v)).2 - z
        }
    };
    Pair {
        what: format!(
            "corrugated sheet{} tapered to {taper}, scale {k}, {name}, the plane from {from} rising {rise}",
            if heights[6] > 0.0 {
                " with a shoulder"
            } else {
                ""
            }
        ),
        k,
        flat: plane(Frame::new(origin, e1.cross(e2), e1).unwrap()),
        other: surface,
        within: cube(
            frame.point(0.5 * SHEET_WIDTH * k, 0.5 * SHEET_LENGTH * k, 0.0),
            52.0 * k,
        ),
        origin,
        e1,
        e2,
        gap: Box::new(gap),
        crossing,
        curves: Curves::Strands(stretches.len(), Box::new(by)),
    }
}

/// How far the planes are from the saddle or the crest, in tolerances.
const OFF: [f64; 3] = [10.0, 100.0, 3000.0];

/// The tube of the fat torus and of the thin ring.
const TUBES: [f64; 2] = [2.0, 0.03];
/// The broad ridge and the sharp one: how far to either side, how deep.
const RIDGES: [(f64, f64); 2] = [(1.0, 1.0), (0.05, 0.6)];

fn pinches(tube: f64, inside: bool) -> Vec<Pair> {
    let mut pairs = Vec::new();
    for k in SCALES {
        for (name, frame) in frames(k) {
            for by in OFF {
                let by = if inside { by } else { -by };
                pairs.push(pinched(k, name, frame, tube, by * tol(k).linear));
            }
        }
    }
    pairs
}

fn corrugations(heights: &[f64; 11], planes: [(f64, f64); 2]) -> Vec<Pair> {
    let mut pairs = Vec::new();
    for k in SCALES {
        for (name, frame) in frames(k) {
            for taper in [1.0, 0.9] {
                for plane in planes {
                    pairs.push(corrugated(k, name, frame, heights, taper, plane));
                }
            }
        }
    }
    pairs
}

fn ripples(shape: (f64, f64), level: fn(f64) -> Level) -> Vec<Pair> {
    let mut pairs = Vec::new();
    for k in SCALES {
        for (name, frame) in frames(k) {
            for by in OFF {
                pairs.push(rippled(k, name, frame, shape, level(by)));
            }
        }
    }
    pairs
}

/// A plane along the axis of a fat torus, a little nearer the axis than
/// the plane that touches its inner equator: two ovals, their tips
/// facing each other 25 to 440 thousand tolerances apart, their sides at
/// 27 degrees to the line between the tips.
#[test]
fn a_plane_just_inside_the_inner_tangent_plane_of_a_torus_meets_it_in_two_ovals() {
    all_come_back(pinches(TUBES[0], true));
}

/// The same of a thin ring: two long ovals, their tips 28 to 490 thousand
/// tolerances apart, their sides at 3 degrees to the line between them.
#[test]
fn a_plane_just_inside_the_inner_tangent_plane_of_a_thin_ring_meets_it_in_two_ovals() {
    all_come_back(pinches(TUBES[1], true));
}

/// The same planes a little further from the axis than the tangent
/// plane: one curve, with a waist where the ovals' tips were.
#[test]
fn a_plane_just_outside_the_inner_tangent_plane_of_a_torus_meets_it_in_one_curve() {
    all_come_back(pinches(TUBES[0], false));
    all_come_back(pinches(TUBES[1], false));
}

/// A broad rippled ridge cut a little above the saddles between its
/// crests: a row of loops whose tips face each other across each saddle.
#[test]
fn a_plane_just_above_the_saddles_of_a_rippled_ridge_meets_it_in_a_row_of_loops() {
    all_come_back(ripples(RIDGES[0], Level::AboveSaddle));
}

/// The same of a sharp ridge: a row of long narrow loops, tip to tip.
#[test]
fn a_plane_just_above_the_saddles_of_a_sharp_ridge_meets_it_in_a_row_of_narrow_loops() {
    all_come_back(ripples(RIDGES[1], Level::AboveSaddle));
}

/// Cut a little below its lowest saddles: the loops either side of them
/// are one, with a waist.
#[test]
fn a_plane_just_below_the_saddles_of_a_rippled_ridge_meets_it_in_loops_with_waists() {
    all_come_back(ripples(RIDGES[0], Level::BelowSaddle));
    all_come_back(ripples(RIDGES[1], Level::BelowSaddle));
}

/// Cut level with its crests, a little below the lowest of them: a small
/// loop about each crest that reaches the plane.
#[test]
fn a_plane_level_with_the_crests_of_a_rippled_ridge_meets_it_in_small_loops() {
    all_come_back(ripples(RIDGES[0], Level::BelowCrest));
    all_come_back(ripples(RIDGES[1], Level::BelowCrest));
}

/// A corrugated sheet whose crests are of uneven height, under a plane
/// level with them that rises a little along them, from 0.11 at the
/// sheet's near edge: four curves side by side, each from the near edge
/// up to where the plane is level with its crest and back; three are
/// hairpins over one crest, and the fourth is over the last two crests,
/// whose trough is just above the plane at the edge: it turns once more
/// a little way in from the edge (0.6 to 2.4), where the plane is level
/// with that trough.
/// Refused until 2026-11-22, all 24 pairs, in the fit's words: a march
/// down that curve towards its turn by the edge, in steps of up to a
/// thirty-second of the box (5.6, the sheet 100 long), comes within 2 to
/// 4 of the edge, and its last step, which leaves the sheet, was put on
/// the edge at a foot of the crossing 2.4 to 7.6 across.
#[test]
fn a_plane_level_with_the_crests_of_a_corrugated_sheet_meets_it_in_hairpins_side_by_side() {
    all_come_back(corrugations(&CRESTS, [(0.11, 0.12), (0.11, 0.5)]));
}

/// The same sheet with a shoulder between two of its crests, under a
/// plane from 0.10 at the near edge: two curves, one over the first two
/// crests and one over the last three, which runs up the sheet, back to
/// where the plane is level with the shoulder, up again, back to where
/// it is level with the next trough, a little way in from the edge, and
/// up and back once more: stretches side by side that turn into one
/// another, as the wing's skin has them under its spar.
#[test]
fn a_plane_level_with_a_shoulder_of_a_corrugated_sheet_meets_it_in_stretches_side_by_side() {
    all_come_back(corrugations(&SHOULDER, [(0.10, 0.15), (0.10, 0.5)]));
}

/// Asked once (`meet_surfaces_once`), a pair is marched as it was before
/// branches were taken up again: the corrugated sheet is refused, in the
/// fit's words, and a pair that march answers (the sharp ridge) comes
/// back from `meet_surfaces` as from it, curve for curve and number for
/// number: `meet_surfaces` marches a pair a second time, taking branches
/// up, only where the first march's fit gives up.
#[test]
fn asked_once_a_pair_is_marched_as_it_was_and_again_only_where_that_fit_gives_up() {
    for k in SCALES {
        let t = tol(k);
        let sheet = corrugated(k, "upright", Frame::WORLD, &CRESTS, 0.9, (0.11, 0.12));
        let once = meet_surfaces_once(&sheet.flat, &sheet.other, &sheet.within, &t);
        assert!(
            matches!(&once, Err(e) if e.to_string().contains("could not be fitted within the tolerance")),
            "{k}: {once:?}"
        );
        let again = meet_surfaces(&sheet.flat, &sheet.other, &sheet.within, &t);
        assert!(
            matches!(again, Ok(SurfaceMeeting::Meets(_))),
            "{k}: {again:?}"
        );
        let ridge = rippled(
            k,
            "upright",
            Frame::WORLD,
            RIDGES[1],
            Level::AboveSaddle(10.0),
        );
        let once = meet_surfaces_once(&ridge.flat, &ridge.other, &ridge.within, &t).unwrap();
        let again = meet_surfaces(&ridge.flat, &ridge.other, &ridge.within, &t).unwrap();
        assert!(matches!(once, SurfaceMeeting::Meets(_)), "{k}");
        assert_eq!(once, again, "{k}");
    }
}

/// The heights of a sheet's profile at its control points, drawn: five
/// crests 0.15 to 0.36 high there, the troughs between them low (within
/// 0.05 of nothing) or, two in five, high (0.08 to 0.2: a shoulder), and
/// the ends well below, as [`CRESTS`] has them.
fn drawn_profile(rng: &mut Rng) -> [f64; 11] {
    let mut heights = [-0.6; 11];
    for (i, h) in heights.iter_mut().enumerate().take(10).skip(1) {
        *h = if i % 2 == 1 {
            rng.range(0.15, 0.36)
        } else if rng.range(0.0, 1.0) < 0.4 {
            rng.range(0.08, 0.2)
        } else {
            rng.range(-0.05, 0.05)
        };
    }
    heights
}

/// The greatest height of a profile and the heights of its troughs (the
/// least between two crests), by the sheet's own evaluation along its
/// first section.
fn profile_shape(heights: &[f64; 11]) -> (f64, Vec<f64>) {
    // (Under a plane the whole profile is below: only the sheet is read.)
    let trial = corrugated(1.0, "upright", Frame::WORLD, heights, 1.0, (0.0, 10.0));
    let height = |u: f64| trial.other.eval(u, 0.0).z;
    let at = |i: i32| height(f64::from(i) / 20_000.0);
    let top = (0..=20_000).map(at).fold(f64::NEG_INFINITY, f64::max);
    let mut troughs = Vec::new();
    for i in 1..20_000 {
        let here = at(i);
        if here < at(i - 1) && here <= at(i + 1) && here > -0.2 {
            let (lo, hi) = (f64::from(i - 1) / 20_000.0, f64::from(i + 1) / 20_000.0);
            troughs.push(extreme(&height, lo, hi, 1.0));
        }
    }
    (top, troughs)
}

/// A sheet's pair at each scale, upright at the least and the greatest
/// and askew at the middle one.
fn at_each_scale(
    what: &str,
    heights: &[f64; 11],
    taper: f64,
    plane: (f64, f64),
    pairs: &mut Vec<Pair>,
) {
    for (i, k) in SCALES.into_iter().enumerate() {
        let [upright, askew] = frames(k);
        let (name, frame) = if i == 1 { askew } else { upright };
        let mut pair = corrugated(k, name, frame, heights, taper, plane);
        pair.what = format!("{what}: {}", pair.what);
        pairs.push(pair);
    }
}

/// Sheets of drawn profiles under drawn planes (2026-11-22, the review
/// of the march taken up again): eight profiles of five crests with
/// troughs and shoulders as they fall, each tapered to 1, 0.9 or 0.8 of
/// its height under two planes from three to eight tenths of its
/// greatest height at the near edge, which leave the sheet three to nine
/// tenths of the way along it. 48 pairs, held to the crossing as the
/// sheets of [`CRESTS`] and [`SHOULDER`] are. When this was written 15 of
/// them were refused where a pair is marched once (`meet_surfaces_once`),
/// in the fit's words, and came back right with their branches taken up
/// again; the other 33 were answered by the first march.
#[test]
fn planes_over_sheets_of_drawn_profiles_meet_them_in_their_curves() {
    let mut rng = Rng(20_261_122);
    let mut pairs = Vec::new();
    for n in 0..8 {
        let heights = drawn_profile(&mut rng);
        let (top, _) = profile_shape(&heights);
        for _ in 0..2 {
            let taper = [1.0, 0.9, 0.8][(rng.next() % 3) as usize];
            let from = top * rng.range(0.3, 0.8);
            let reach = rng.range(0.3, 0.9);
            // (No lower: the tapered profile's low ends stay under it.)
            let rise = ((top - from) / reach).max(0.6 * (1.0 - taper) + 0.02);
            let what = format!("drawn profile {n}");
            at_each_scale(&what, &heights, taper, (from, rise), &mut pairs);
        }
    }
    assert_eq!(pairs.len(), 48);
    all_come_back(pairs);
}

/// Sheets of drawn profiles under a plane whose lowest line is 3, 10 and
/// 100 tolerances below and above the profile's highest trough at the
/// near edge. Below it the two hairpins either side of the trough are
/// one curve, which turns back a short way inside the edge (0.001 to
/// 0.3 of a sheet 100 long), far tighter than a step; above it they are
/// two curves whose feet on the edge are a few thousand to a few ten
/// thousand tolerances apart.
/// Five profiles (a sixth, whose highest trough is within a tenth of its
/// greatest height, is left out), six planes, 90 pairs. When this was
/// written 45 of them were refused where a pair is marched once, in the
/// fit's words, and came back right with their branches taken up again.
#[test]
fn planes_level_with_a_trough_of_a_drawn_sheet_meet_it_in_its_curves() {
    let mut rng = Rng(7_112_026);
    let mut pairs = Vec::new();
    for n in 0..6 {
        let heights = drawn_profile(&mut rng);
        let (top, troughs) = profile_shape(&heights);
        let trough = troughs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        if trough <= 0.02 || trough >= 0.9 * top {
            continue;
        }
        let taper = [1.0, 0.9][(rng.next() % 2) as usize];
        let reach = rng.range(0.4, 0.9);
        for by in [-100.0, -10.0, -3.0, 3.0, 10.0, 100.0] {
            let from = trough + by * 1e-6;
            let rise = ((top - from) / reach).max(0.6 * (1.0 - taper) + 0.02);
            let what =
                format!("drawn profile {n}, the plane {by} tolerances above its highest trough");
            at_each_scale(&what, &heights, taper, (from, rise), &mut pairs);
        }
    }
    assert_eq!(pairs.len(), 90);
    all_come_back(pairs);
}
