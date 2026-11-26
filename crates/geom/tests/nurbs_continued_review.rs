//! The review of `check/spline-box-domain` (a93e8aea, 2026-11-26): a spline
//! carried on past its domain, bounded by `NurbsCurve::continued_hull`,
//! `continued_pieces` and `continued_derivative_bounds`, and read by
//! `hull_points`, on splines the builder's test does not reach: degrees 1
//! to 12, 16, 24 and 32; weights all one, within a decade of one, three
//! and six decades either side of it, and one control point a million
//! times heavier than the rest; clamped and unclamped ends, spans a
//! millionth of the others' width; parameters scaled by 1e-3 to 1e3 and
//! shifted; control points at 1e-3 to 1e6, at the origin and far from it;
//! ranges a unit in the last place to ten times the domain's length past
//! either end, past both, and wholly before or after it.
//!
//! Held, for every point `eval` gives as it is computed (no margin past
//! the domain but the test's own rounding of a dot product):
//! - the box of the control points and `continued_hull` (topo's box of an
//!   edge before its own padding) holds every point of the range;
//! - `hull_points` holds every point along 32 directions (its support
//!   function: the convex hull, not only its box);
//! - past the domain, a stretch of `continued_pieces` whose parameters
//!   hold the point, padded as `continued_hull` pads it, holds it (and
//!   the largest share of a pad the rounding took is printed);
//! - past the domain, the speed and the bend stay within
//!   `continued_derivative_bounds`, to a millionth and a thousandth (the
//!   bounds are not padded for rounding; how far above them the speed
//!   comes is printed);
//! - a range a unit in the last place, or a millionth of a millionth of
//!   the domain, past it has a hull where the weights lie within a decade
//!   of one (topo's check refuses an edge without one,
//!   `Defect::PastDomain`), but past the end of a domain that ends on a
//!   span of no width (counted, as such ranges of weights decades apart
//!   given none are);
//! - a polynomial spline has a hull wherever neither its points nor the
//!   hull's pad overflow, but past the end of a domain that ends on a span
//!   of no width;
//! - within the domain nothing is added, and `hull_points` is that of
//!   `d835d891`, bit for bit (kept here word for word as the reference).
//!
//! Printed, not asserted: how often a rational range has no hull though
//! its weight stays above a millionth of its heaviest past the domain (the
//! halving gave up on a curve that is bounded there; topo's check refuses
//! such an edge), and how often `continued_pieces` gives pieces where
//! `continued_hull` gives none (its pad overflows millions of span widths
//! past, where the cut's points do not).
//!
//! Run 1 of the review (Check 45761771208, e26dfd3d) measured: 8027 ranges
//! past the domain held, no point outside, the rounding taking at most a
//! tenth of a stretch's pad; the speed `derivatives` gives at the far end
//! of a stretch 2.4e-8 of the bound above `continued_derivative_bounds`
//! (degree 9, weights three decades apart, 1e4 from the origin).
//!
//! The last test is a blocker of the branch: `continued_pieces` of a range
//! that starts at or past the domain's end reaches back to the start of
//! the last span, outside the range (`outline_domain_review.rs` shows what
//! the search by pieces answers then).

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_geom::{NurbsCurve, hull_points_in};
use craft3d_math::{Bbox3, Portable, Vec3};
use std::collections::BTreeMap;

/// Numbers in [0, 1) by splitmix64.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        (z >> 11) as f64 / (1_u64 << 53) as f64
    }

    fn below(&mut self, n: usize) -> usize {
        ((self.next() * n as f64) as usize).min(n.saturating_sub(1))
    }
}

/// The kinds of weights `random_curve` gives.
const WEIGHTS: [&str; 5] = [
    "polynomial",
    "within a decade",
    "three decades",
    "six decades",
    "one heavy",
];

/// A random spline and the kind of its weights (an index of `WEIGHTS`);
/// `None` for knots that leave no domain.
fn random_curve(rng: &mut Rng) -> Option<(NurbsCurve, usize)> {
    const DEGREES: [usize; 15] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 16, 24, 32];
    let degree = DEGREES[rng.below(DEGREES.len())];
    let count = degree + 1 + rng.below(8);
    let wanted = count + degree + 1;
    let scale = [1e-3, 1.0, 1e3][rng.below(3)];
    let mut at = [0.0, 1e3, -5.0][rng.below(3)] * scale;
    let mut knots = Vec::with_capacity(wanted);
    if rng.next() < 0.5 {
        knots.extend(std::iter::repeat_n(at, degree + 1));
    }
    while knots.len() < wanted {
        let step = match rng.below(4) {
            0 => 1.0,
            1 => 0.001 + rng.next(),
            2 => 1e-6 * (1.0 + rng.next()),
            _ => 0.1 + 3.0 * rng.next(),
        };
        at += step * scale;
        let times = (1 + rng.below(degree)).min(wanted - knots.len());
        knots.extend(std::iter::repeat_n(at, times));
    }
    if rng.next() < 0.5 {
        // Clamped at the end: the last degree + 1 knots equal.
        let last = knots[wanted - 1];
        for k in &mut knots[wanted - degree - 1..] {
            *k = last;
        }
    }
    let kind = rng.below(WEIGHTS.len());
    let mut weights: Vec<f64> = (0..count)
        .map(|_| match kind {
            1 => 0.3 + 2.7 * rng.next(),
            2 => 10.0_f64.ppowf(-3.0 + 6.0 * rng.next()),
            3 => 10.0_f64.ppowf(-6.0 + 12.0 * rng.next()),
            _ => 1.0,
        })
        .collect();
    if kind == 4 {
        let heavy = rng.below(count);
        weights[heavy] = 1e6;
    }
    let k = [1e-3, 1.0, 1e3, 1e6][rng.below(4)];
    let shift = if rng.next() < 0.5 {
        Vec3::ZERO
    } else {
        Vec3::new(3.0, -7.0, 11.0) * (1e3 * k)
    };
    let ctrl = (0..count)
        .map(|_| Vec3::new(rng.next() - 0.5, rng.next() - 0.5, rng.next() - 0.5) * k + shift)
        .collect();
    NurbsCurve::new(degree, knots, ctrl, weights)
        .ok()
        .map(|c| (c, kind))
}

/// `NurbsCurve::hull_points` as it was on `d835d891`, word for word but
/// for `self`: the reference within the domain.
fn reference_hull_points(c: &NurbsCurve, range: (f64, f64)) -> Option<Vec<Vec3>> {
    if !(range.0.is_finite() && range.1.is_finite()) {
        return None;
    }
    let (r0, r1) = (range.0.min(range.1), range.0.max(range.1));
    let (d0, d1) = c.domain();
    if r0 <= d0 && r1 >= d1 {
        let positive = c.weights().iter().all(|w| w.is_finite() && *w > 0.0);
        return positive.then(|| c.control_points().to_vec());
    }
    if r0 < d0 || r1 > d1 {
        return None;
    }
    hull_points_in(&c.bezier_pieces(), (r0, r1))
}

/// The bits of some points, to compare them exactly.
fn bits(points: Option<&[Vec3]>) -> Option<Vec<[u64; 3]>> {
    points.map(|ps| {
        ps.iter()
            .map(|p| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()])
            .collect()
    })
}

/// The spline's weight as a function, carried on as `eval` carries the
/// curve (the same knots, so the same span): a polynomial spline whose
/// control points' x are the weights.
fn weight_curve(c: &NurbsCurve) -> NurbsCurve {
    let points = c.weights().iter().map(|&w| Vec3::new(w, 0.0, 0.0));
    NurbsCurve::polynomial(c.degree(), c.knots().to_vec(), points.collect()).unwrap()
}

/// The largest coordinate of `p`, in size.
fn largest_of(p: Vec3) -> f64 {
    p.x.abs().max(p.y.abs()).max(p.z.abs())
}

/// How far `p` lies outside `bx`, along the axis where it lies farthest.
fn excess(bx: &Bbox3, p: Vec3) -> f64 {
    let Some((lo, hi)) = bx.corners() else {
        return f64::INFINITY;
    };
    let over = |a: f64, low: f64, high: f64| (low - a).max(a - high).max(0.0);
    over(p.x, lo.x, hi.x)
        .max(over(p.y, lo.y, hi.y))
        .max(over(p.z, lo.z, hi.z))
}

/// The 26 directions to the corners, edges and faces of a cube, and six
/// at random.
fn directions(rng: &mut Rng) -> Vec<Vec3> {
    let mut out = Vec::new();
    for x in [-1.0, 0.0, 1.0] {
        for y in [-1.0, 0.0, 1.0] {
            for z in [-1.0, 0.0, 1.0] {
                let d = Vec3::new(x, y, z);
                if d != Vec3::ZERO {
                    out.push(d);
                }
            }
        }
    }
    for _ in 0..6 {
        out.push(Vec3::new(
            rng.next() - 0.5,
            rng.next() - 0.5,
            rng.next() - 0.5,
        ));
    }
    out
}

/// The parameters a range is sampled at: evenly, its ends, and the
/// domain's ends and just outside them where the range holds them.
fn samples((r0, r1): (f64, f64), (d0, d1): (f64, f64)) -> Vec<f64> {
    let mut ts: Vec<f64> = (0..=192)
        .map(|i| (r0 + (r1 - r0) * f64::from(i) / 192.0).clamp(r0, r1))
        .collect();
    ts.extend([r0, r1]);
    let l = d1 - d0;
    for t in [
        d0,
        d1,
        d0.next_down(),
        d1.next_up(),
        d0 - 1e-9 * l,
        d1 + 1e-9 * l,
    ] {
        if (r0..=r1).contains(&t) {
            ts.push(t);
        }
    }
    ts
}

/// One stretch of an end piece carried on: its parameters, the box of its
/// control points, that box padded as `continued_hull` pads it, and the
/// pad.
struct Stretch {
    over: (f64, f64),
    bare: Bbox3,
    padded: Bbox3,
    pad: f64,
}

/// What the review found.
#[derive(Default)]
struct Log {
    /// Per kind of failure, how many and the first few.
    failures: BTreeMap<&'static str, (usize, Vec<String>)>,
    /// Per kind of outcome, how many.
    outcomes: BTreeMap<&'static str, usize>,
    /// Ranges past the domain with a hull, and with derivative bounds.
    held: usize,
    bounded: usize,
    /// The largest share of a stretch's pad the rounding took, and where.
    most_pad: (f64, String),
    /// How far, at most, the speed past the domain came above
    /// `continued_derivative_bounds` (relative), and where.
    most_speed: (f64, String),
    /// The most a rational weight stayed above nothing past the domain,
    /// relative to the heaviest, on a range given no hull, and where.
    most_refused: (f64, String),
}

impl Log {
    fn fail(&mut self, what: &'static str, message: String) {
        let entry = self.failures.entry(what).or_default();
        entry.0 += 1;
        if entry.1.len() < 6 {
            entry.1.push(message);
        }
    }

    fn count(&mut self, outcome: &'static str) {
        *self.outcomes.entry(outcome).or_default() += 1;
    }
}

/// A short account of a spline, for a failure's message.
fn describe(c: &NurbsCurve, kind: usize) -> String {
    format!(
        "degree {} weights {} domain {:?} knots {:?} weights {:?} first point {:?}",
        c.degree(),
        WEIGHTS[kind],
        c.domain(),
        c.knots(),
        c.weights(),
        c.control_points().first()
    )
}

#[test]
fn a_spline_carried_on_past_its_domain_is_held_for_every_degree_weight_and_reach() {
    let mut rng = Rng(0x5b1e_d0aa_2026_1126);
    let dirs = directions(&mut rng);
    let mut log = Log::default();
    let (mut curves, mut skipped) = (0_usize, 0_usize);
    while curves < 2400 {
        let Some((curve, kind)) = random_curve(&mut rng) else {
            skipped += 1;
            continue;
        };
        curves += 1;
        review(&curve, kind, &dirs, &mut rng, &mut log);
    }
    println!(
        "{curves} splines ({skipped} knot vectors left no domain); {} ranges past the domain held, {} with derivative bounds",
        log.held, log.bounded
    );
    for (outcome, n) in &log.outcomes {
        println!("  {outcome}: {n}");
    }
    println!(
        "  the largest share of a stretch's pad the rounding took: {:e} ({})",
        log.most_pad.0, log.most_pad.1
    );
    println!(
        "  the most a rational weight stayed above nothing, of its heaviest, past the domain on a range given no hull: {:e} ({})",
        log.most_refused.0, log.most_refused.1
    );
    println!(
        "  the most the speed past the domain came above continued_derivative_bounds: {:e} of it ({})",
        log.most_speed.0, log.most_speed.1
    );
    for (what, (n, examples)) in &log.failures {
        println!("FAILED {what}: {n}");
        for example in examples {
            println!("    {example}");
        }
    }
    assert!(
        log.failures.is_empty(),
        "{} kinds of failure (printed above)",
        log.failures.len()
    );
    assert!(log.held > 2000, "{} held", log.held);
}

/// Every check of one spline.
fn review(curve: &NurbsCurve, kind: usize, dirs: &[Vec3], rng: &mut Rng, log: &mut Log) {
    let (d0, d1) = curve.domain();
    let l = d1 - d0;
    let mid = 0.5 * (d0 + d1);
    let knots = curve.knots();
    let count = curve.control_points().len();
    // `eval` carries on the span `count - 1` past the end.
    let empty_end = knots[count] <= knots[count - 1];
    let what = describe(curve, kind);
    // Within the domain: nothing carried on, and the hull as it was.
    for range in [
        (d0, d1),
        (d1, d0),
        (d0, mid),
        (mid, d1),
        (d0 + 0.3 * l, d0 + 0.31 * l),
        (mid, mid),
        (d1, d1),
        (d0, d0),
    ] {
        if curve.runs_past_domain(range) {
            log.fail(
                "a range within the domain read as past it",
                format!("{range:?}: {what}"),
            );
            continue;
        }
        let nothing = curve.continued_hull(range).is_some_and(|h| h.is_empty())
            && curve.continued_pieces(range).is_some_and(|p| p.is_empty())
            && curve.continued_derivative_bounds(range) == Some((0.0, 0.0));
        if !nothing {
            log.fail(
                "a range within the domain given a part carried on",
                format!("{range:?}: {what}"),
            );
        }
        let now = curve.hull_points(range);
        let then = reference_hull_points(curve, range);
        if bits(now.as_deref()) != bits(then.as_deref()) {
            log.fail(
                "hull_points within the domain not d835d891's, bit for bit",
                format!("{range:?}: {now:?} against {then:?}: {what}"),
            );
        }
        log.count("ranges within the domain, as before");
    }
    // A rounding past the domain.
    for range in [
        (d0.next_down(), d1),
        (d0, d1.next_up()),
        (d0.next_down(), d1.next_up()),
        (d0 - 1e-12 * l, d1 + 1e-12 * l),
    ] {
        if !curve.runs_past_domain(range) {
            continue;
        }
        if curve.continued_hull(range).is_some() {
            log.count("a rounding past the domain: held");
        } else if range.1 > d1 && empty_end {
            log.count("a rounding past the end of a domain that ends on an empty span: no hull");
        } else if kind >= 2 {
            // Weights decades apart: the weight carried on may fall to
            // nothing within a rounding of the domain.
            log.count("a rounding past the domain, weights decades apart: no hull");
        } else {
            log.fail(
                "a range a rounding past the domain has no hull",
                format!("{range:?}: {what}"),
            );
        }
    }
    // Past it, by a millionth of a millionth to ten times the domain.
    let s = [1e-12, 1e-6, 0.01, 0.3, 1.0, 3.0, 10.0][rng.below(7)];
    let (before, after) = (d0 - s * l, d1 + s * l);
    for range in [
        (before, mid),
        (mid, after),
        (before, after),
        (before, d0 - 0.5 * s * l),
        (d1 + 0.5 * s * l, after),
        (after, before),
    ] {
        if curve.runs_past_domain(range) {
            past(curve, kind, range, dirs, empty_end, log);
        }
    }
}

/// The checks of one range past the domain.
fn past(
    curve: &NurbsCurve,
    kind: usize,
    range: (f64, f64),
    dirs: &[Vec3],
    empty_end: bool,
    log: &mut Log,
) {
    let (r0, r1) = (range.0.min(range.1), range.0.max(range.1));
    let (d0, d1) = curve.domain();
    let ts = samples((r0, r1), (d0, d1));
    let what = || format!("{range:?}: {}", describe(curve, kind));
    let carried = curve.continued_hull(range);
    let pieces = curve.continued_pieces(range);
    let hull = curve.hull_points(range);
    if carried.is_none() && pieces.is_some() && hull.is_none() {
        // The pad overflows where the cut's points do not (millions of
        // span widths past): no hull, and so a refusal by topo's check,
        // but pieces for a search (counted; `continued_pieces` says it is
        // none where `continued_hull` is).
        log.count("no hull but pieces: the pad overflows (continued_pieces says none where continued_hull is)");
        if kind == 0 {
            // A polynomial's points there reach 1e169 and more (run 3 of
            // the review, Check 45763696546): refused, never wrong.
            log.count("no hull: a polynomial whose pad overflows");
        } else {
            unknown(curve, kind, (r0, r1), empty_end, log);
        }
        return;
    }
    if carried.is_some() != pieces.is_some() || carried.is_some() != hull.is_some() {
        log.fail(
            "continued_hull, continued_pieces and hull_points disagree on a hull",
            format!(
                "{} {} {}: {}",
                carried.is_some(),
                pieces.is_some(),
                hull.is_some(),
                what()
            ),
        );
        return;
    }
    let (Some(carried), Some(pieces), Some(hull)) = (carried, pieces, hull) else {
        unknown(curve, kind, (r0, r1), empty_end, log);
        return;
    };
    log.held += 1;
    // Topo's box of the edge, before its own padding.
    let mut boxed = Bbox3::from_points(curve.control_points().iter().copied());
    for &p in &carried {
        boxed.add(p);
    }
    // The hull's support along each direction.
    let support: Vec<f64> = dirs
        .iter()
        .map(|&d| hull.iter().fold(f64::NEG_INFINITY, |m, &q| m.max(d.dot(q))))
        .collect();
    let largest = hull.iter().fold(0.0_f64, |m, &q| m.max(largest_of(q)));
    // The stretches, eight padded corners for each control point.
    let per = 8 * (curve.degree() + 1);
    if carried.len() != pieces.len() * per {
        log.fail(
            "continued_hull is not eight corners for each point of continued_pieces",
            what(),
        );
        return;
    }
    let stretches: Vec<Stretch> = pieces
        .iter()
        .enumerate()
        .map(|(s, (over, net))| {
            let corners = &carried[s * per..(s + 1) * per];
            Stretch {
                over: *over,
                bare: Bbox3::from_points(net.iter().map(|&(q, _)| q)),
                padded: Bbox3::from_points(corners.iter().copied()),
                pad: 0.5 * (corners[7].x - corners[0].x),
            }
        })
        .collect();
    let bounds = curve.continued_derivative_bounds(range);
    if bounds.is_some() {
        log.bounded += 1;
    }
    let h = 1e-3 * (r1 - r0);
    for &t in &ts {
        let p = curve.eval(t);
        let outside = !(d0..=d1).contains(&t);
        let size = largest.max(largest_of(p));
        if !outside {
            // Within the domain the hull is the control points' and that of
            // pieces cut within it, unpadded, as on `d835d891`: held to
            // 1e-12 of the size, as `hull_points.rs` holds it, and a miss
            // counted, not failed (it is not this branch's).
            let along = |d: Vec3| d.dot(p);
            let misses = !boxed.contains(p, 1e-12 * size)
                || dirs.iter().zip(&support).any(|(&d, &most)| {
                    let reach = d.x.abs() + d.y.abs() + d.z.abs();
                    along(d).is_nan() || along(d) > most + 1e-12 * size * reach
                });
            if misses {
                log.count("a point within the domain outside by more than 1e-12 of the size (as on d835d891)");
            }
            continue;
        }
        if !boxed.contains(p, 0.0) {
            log.fail(
                "the box of the control points and continued_hull misses a point",
                format!("at {t}: {p:?} outside {boxed:?}; {}", what()),
            );
        }
        // Past the domain the hull is padded: no margin but the rounding
        // of the dot product.
        let margin = 4.0 * f64::EPSILON * size;
        for (&d, &most) in dirs.iter().zip(&support) {
            let along = d.dot(p);
            let reach = d.x.abs() + d.y.abs() + d.z.abs();
            if along.is_nan() || along > most + margin * reach {
                log.fail(
                    "hull_points misses a point",
                    format!(
                        "at {t}: {p:?} along {d:?} {along} beyond {most}; {}",
                        what()
                    ),
                );
                break;
            }
        }
        // A stretch that covers the point holds it, padded.
        let (mut covered, mut held, mut share) = (false, false, f64::INFINITY);
        for stretch in &stretches {
            if (stretch.over.0..=stretch.over.1).contains(&t) {
                covered = true;
                held |= stretch.padded.contains(p, 0.0);
                share = share.min(excess(&stretch.bare, p) / stretch.pad);
            }
        }
        if !covered {
            log.fail(
                "no stretch covers a parameter past the domain",
                format!("at {t}; {}", what()),
            );
        } else if !held {
            log.fail(
                "no stretch that covers a point past the domain holds it, padded",
                format!("at {t}: {p:?}, {share} of the pad; {}", what()),
            );
        }
        if share.is_finite() && share > log.most_pad.0 {
            log.most_pad = (share, format!("at {t}; {}", what()));
        }
        let Some((first, second)) = bounds else {
            continue;
        };
        // The bounds are not padded for rounding: how far above them the
        // speed `derivatives` gives comes is measured, and more than a
        // millionth fails.
        let speed = curve.derivatives(t).1.length();
        let over = speed / first - 1.0;
        if speed.is_nan() || over > 1e-6 {
            log.fail(
                "the speed past the domain is above continued_derivative_bounds",
                format!("at {t}: {speed} > {first}; {}", what()),
            );
        }
        if over > log.most_speed.0 {
            log.most_speed = (over, format!("at {t}; {}", what()));
        }
        // The bend, as the mean of the second derivative over a short
        // stretch past the domain within the range, over the step taken
        // (a step of a few units in the last place of `t` is not measured).
        let (ta, tb) = (t - h, t + h);
        let step = tb - ta;
        if step > 1e4 * f64::EPSILON * t.abs() && ta >= r0 && tb <= r1 && (tb < d0 || ta > d1) {
            let bend = (curve.derivatives(tb).1 - curve.derivatives(ta).1).length() / step;
            if bend.is_nan() || bend > second * (1.0 + 1e-3) + 1e-6 * first / step {
                log.fail(
                    "the bend past the domain is above continued_derivative_bounds",
                    format!("at {t}: {bend} > {second}; {}", what()),
                );
            }
        }
    }
}

/// Why a range past the domain has no hull.
fn unknown(curve: &NurbsCurve, kind: usize, (r0, r1): (f64, f64), empty_end: bool, log: &mut Log) {
    let (d0, d1) = curve.domain();
    if r1 > d1 && empty_end {
        log.count("no hull: past the end of a domain that ends on an empty span");
        return;
    }
    let ts: Vec<f64> = (0..=2000)
        .map(|i| (r0 + (r1 - r0) * f64::from(i) / 2000.0).clamp(r0, r1))
        .filter(|t| !(d0..=d1).contains(t))
        .collect();
    if kind == 0 {
        // A polynomial loses its hull only where its points overflow.
        let most = ts
            .iter()
            .fold(0.0_f64, |m, &t| m.max(largest_of(curve.eval(t))));
        if most.is_finite() && most < 1e200 {
            log.fail(
                "a polynomial spline past its domain has no hull",
                format!(
                    "points up to {most:e} over [{r0}, {r1}]: {}",
                    describe(curve, kind)
                ),
            );
        } else {
            log.count("no hull: a polynomial carried on to where it overflows");
        }
        return;
    }
    let weight = weight_curve(curve);
    let heaviest = curve.weights().iter().fold(0.0_f64, |m, &w| m.max(w));
    let lightest = ts
        .iter()
        .fold(f64::INFINITY, |m, &t| m.min(weight.eval(t).x))
        / heaviest;
    if lightest > 1e-6 {
        log.count(
            "no hull: rational, its weight above a millionth of the heaviest past the domain",
        );
        if lightest > log.most_refused.0 {
            log.most_refused = (lightest, format!("[{r0}, {r1}]: {}", describe(curve, kind)));
        }
    } else {
        log.count(
            "no hull: rational, its weight a millionth of the heaviest or less past the domain",
        );
    }
}

/// A rational quadratic whose weight `1 + 18 t - 18 t^2` falls to nothing
/// at t = 1.0528 and t = -0.0528: carried on through there it runs off to
/// infinity, no hull and no bound is claimed (the halving stops, at most
/// six deep), also over ranges so long that cutting the piece overflows;
/// short of there it is held, every point in the box.
#[test]
fn a_rational_spline_carried_on_to_where_its_weight_vanishes_has_no_hull_and_short_of_it_is_held() {
    for k in [1e-3, 1.0, 1e6] {
        let c = NurbsCurve::new(
            2,
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            vec![
                Vec3::new(k, 0.0, 0.0),
                Vec3::new(k, k, 0.0),
                Vec3::new(0.0, k, 0.0),
            ],
            vec![1.0, 10.0, 1.0],
        )
        .unwrap();
        for range in [
            (0.0, 2.0),
            (0.5, 1.06),
            (-1.0, 0.5),
            (-1e3, 1e3),
            (0.0, 1e300),
            (-f64::MAX, f64::MAX),
        ] {
            assert!(c.runs_past_domain(range), "{k} {range:?}");
            assert!(c.continued_hull(range).is_none(), "{k} {range:?}");
            assert!(c.continued_pieces(range).is_none(), "{k} {range:?}");
            assert!(
                c.continued_derivative_bounds(range).is_none(),
                "{k} {range:?}"
            );
            assert!(c.hull_points(range).is_none(), "{k} {range:?}");
        }
        for range in [(0.0, 1.04), (-0.05, 1.0), (-0.05, 1.04)] {
            let carried = c.continued_hull(range).unwrap();
            let mut boxed = Bbox3::from_points(c.control_points().iter().copied());
            for p in carried {
                boxed.add(p);
            }
            for i in 0..=4000 {
                let t = range.0 + (range.1 - range.0) * f64::from(i) / 4000.0;
                let p = c.eval(t);
                // Within the domain the control points' box, unpadded.
                let margin = if (0.0..=1.0).contains(&t) {
                    4.0 * f64::EPSILON * k
                } else {
                    0.0
                };
                assert!(boxed.contains(p, margin), "{k} {range:?} at {t}: {p:?}");
            }
        }
        // How near the root a hull is still had (printed).
        let mut reach = 1.0;
        for i in 0..=1000 {
            let t = 1.0 + 0.0528 * f64::from(i) / 1000.0;
            if c.continued_hull((0.0, t)).is_some() {
                reach = t;
            }
        }
        println!("{k}: held up to {reach} of the root at 1.05277");
    }
}

/// `continued_pieces` gives the end pieces "over the parts of `range` that
/// lie past" the domain, what the boolean's search by pieces reads
/// (`outline.rs` › `nearest_by_pieces`). For a range that starts at or
/// past the domain's end, the piece is cut from the start of the last
/// span instead (`continued`: `from` is the range's start only while it
/// lies before the domain's end), so the search looks at parameters that
/// are not the edge's and can answer one (a blocker of the branch; the
/// hull and the bounds are only larger for it).
#[test]
fn the_pieces_carried_on_lie_within_a_range_that_starts_at_or_past_the_domains_end() {
    for k in [1e-3, 1.0, 1e6] {
        let cubic = NurbsCurve::polynomial(
            3,
            vec![0.0, 0.0, 0.0, 0.0, 0.2, 0.5, 0.6, 1.0, 1.0, 1.0, 1.0],
            vec![
                Vec3::new(0.0, 0.0, 0.0) * k,
                Vec3::new(1.0, 2.0, -1.0) * k,
                Vec3::new(2.0, -1.0, 0.5) * k,
                Vec3::new(3.0, 0.5, 2.0) * k,
                Vec3::new(4.0, 1.0, -0.5) * k,
                Vec3::new(5.0, -2.0, 0.0) * k,
                Vec3::new(6.0, 0.0, 1.0) * k,
            ],
        )
        .unwrap();
        let quarter = NurbsCurve::new(
            2,
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            vec![
                Vec3::new(k, 0.0, 0.0),
                Vec3::new(k, k, 0.0),
                Vec3::new(0.0, k, 0.0),
            ],
            vec![1.0, 0.5_f64.sqrt(), 1.0],
        )
        .unwrap();
        for (curve, range) in [
            (&cubic, (1.2, 1.5)),
            (&cubic, (1.0, 1.4)),
            (&quarter, (4.0, 9.0)),
            (&quarter, (1.0, 3.0)),
        ] {
            let pieces = curve.continued_pieces(range).unwrap();
            assert!(!pieces.is_empty(), "{k} {range:?}");
            for (over, _) in &pieces {
                assert!(
                    over.0 >= range.0 && over.1 <= range.1,
                    "{k} {range:?}: a piece carried on over {over:?}"
                );
            }
        }
    }
}
