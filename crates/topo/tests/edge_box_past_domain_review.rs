//! The review of `check/spline-box-domain` (a93e8aea, 2026-11-26): topo's
//! box of a spline edge, and the check's `Defect::PastDomain`, on random
//! splines of every kind (degrees 1 to 12, 16, 24 and 32; weights all one
//! to six decades either side of one, and one control point a million
//! times heavier than the rest; clamped and unclamped ends; control points
//! at 1e-3 to 1e6, at the origin and far from it), as they are and turned
//! and moved far off:
//! - within its domain an edge's box, and the box of a body of that edge,
//!   is that of `d835d891`, bit for bit: the box of its vertices and its
//!   control points, padded by 64 units in the last place of its largest
//!   coordinate (that version's `edge_bbox`, `edge_extent` and
//!   `Extent::padded`, kept here as the reference);
//! - past it, the edge's box holds every point `eval` gives over its
//!   range, with no margin past the domain; or, where the edge has no box,
//!   the check refuses it as past its domain, and only then;
//! - an edge whose range lies a unit in the last place, or a millionth of
//!   a millionth of its domain, past its domain is not refused as past it
//!   where its weights lie within a decade of one, but past the end of a
//!   domain that ends on a knot span of no width (counted, as the refusals
//!   of weights decades apart are).

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_geom::{Curve3, NurbsCurve};
use craft3d_math::{Bbox3, Portable, Tolerance, Transform, Vec3};
use craft3d_topo::{Body, BodyKind, Defect, EdgeId, check};

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

/// A random spline, its size and the kind of its weights (0 all one, 1
/// within a decade of one, 2 and 3 three and six decades either side of
/// it, 4 one control point a million times heavier); `None` for knots
/// that leave no domain.
fn random_curve(rng: &mut Rng) -> Option<(NurbsCurve, f64, usize)> {
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
    let kind = rng.below(5);
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
        .map(|c| (c, k, kind))
}

/// A body of the one edge `curve` over `range`, from and to its points
/// there, placed by `place`; `None` where those points are not finite (a
/// rational spline carried on to where its weight vanishes) or the curve
/// does not survive the placing.
fn edge_alone(curve: &NurbsCurve, range: (f64, f64), place: &Transform) -> Option<Body> {
    let (pa, pb) = (curve.eval(range.0), curve.eval(range.1));
    if !(pa.is_finite() && pb.is_finite()) {
        return None;
    }
    let mut body = Body::new(BodyKind::Sheet);
    let a = body.add_vertex(pa);
    let b = body.add_vertex(pb);
    body.add_edge(Curve3::Nurbs(curve.clone()), range, a, b);
    body.transformed(place).ok()
}

/// `Body::edge_bbox` of a spline edge on `d835d891`: its vertices, then its
/// control points (`edge_extent`), per axis the least and the greatest,
/// padded by 64 units in the last place of the largest coordinate
/// (`Extent::padded`), as that version computes it.
fn reference_box(body: &Body) -> Bbox3 {
    let edge = &body.edges[0];
    let Curve3::Nurbs(curve) = &edge.curve else {
        panic!("not a spline");
    };
    let (mut min, mut max) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    let mut finite = true;
    let ends = [
        body.vertex(edge.start).unwrap().point,
        body.vertex(edge.end).unwrap().point,
    ];
    for p in ends
        .into_iter()
        .chain(curve.control_points().iter().copied())
    {
        for (axis, value) in [p.x, p.y, p.z].into_iter().enumerate() {
            if value.is_finite() {
                min[axis] = min[axis].min(value);
                max[axis] = max[axis].max(value);
            } else {
                finite = false;
            }
        }
    }
    assert!(finite);
    let (lo, hi) = (
        Vec3::new(min[0], min[1], min[2]),
        Vec3::new(max[0], max[1], max[2]),
    );
    let largest = min.iter().chain(&max).fold(0.0_f64, |a, b| a.max(b.abs()));
    let pad = 64.0 * f64::EPSILON * largest;
    let pad = Vec3::new(pad, pad, pad);
    let (lo_padded, hi_padded) = (lo - pad, hi + pad);
    if lo_padded.is_finite() && hi_padded.is_finite() {
        Bbox3::from_points([lo_padded, hi_padded])
    } else {
        Bbox3::from_points([lo, hi])
    }
}

/// The bits of a box's corners.
fn corners(bx: &Bbox3) -> Option<[u64; 6]> {
    bx.corners()
        .map(|(lo, hi)| [lo.x, lo.y, lo.z, hi.x, hi.y, hi.z].map(f64::to_bits))
}

#[test]
fn a_spline_edge_is_boxed_as_before_within_its_domain_and_whole_past_it() {
    let mut rng = Rng(0x7090_b0c5_2026_1126);
    let (mut curves, mut within, mut held) = (0_usize, 0_usize, 0_usize);
    let (mut refused, mut unplaced) = (0_usize, 0_usize);
    let mut failures: Vec<String> = Vec::new();
    while curves < 900 {
        let Some((curve, k, _)) = random_curve(&mut rng) else {
            continue;
        };
        curves += 1;
        let tol = Tolerance::new(1e-6 * k, 1e-9).unwrap();
        let places = [
            Transform::translation(Vec3::ZERO),
            Transform::rotation(Vec3::new(0.3, -0.5, 0.8), 0.7)
                .unwrap()
                .then(&Transform::translation(
                    Vec3::new(7.0, -3.0, 2.0) * (1e3 * k),
                )),
        ];
        let (d0, d1) = curve.domain();
        let l = d1 - d0;
        let mid = 0.5 * (d0 + d1);
        for place in &places {
            // Within the domain, as before, bit for bit.
            for range in [
                (d0, d1),
                (d0 + 0.3 * l, d0 + 0.31 * l),
                (mid, d1),
                (d0, d0 + 1e-9 * l),
            ] {
                let body = edge_alone(&curve, range, place).unwrap();
                let reference = corners(&reference_box(&body));
                let edge = body.edge_bbox(EdgeId(0)).map(|b| corners(&b));
                let whole = body.bbox().map(|b| corners(&b));
                if edge.as_ref().ok() != Some(&reference) || whole.as_ref().ok() != Some(&reference)
                {
                    failures.push(format!(
                        "within the domain {range:?}: {edge:?} and {whole:?} against {reference:?}; {curve:?}"
                    ));
                }
                within += 1;
            }
            // Past it: boxed whole, or refused by the check, never both or
            // neither.
            let s = [1e-6, 0.3, 1.0, 10.0][rng.below(4)];
            for range in [
                (d0 - s * l, mid),
                (mid, d1 + s * l),
                (d0 - s * l, d1 + s * l),
                (d1 + 0.5 * s * l, d1 + s * l),
            ] {
                let Some(body) = edge_alone(&curve, range, place) else {
                    unplaced += 1;
                    continue;
                };
                let past = check(&body, &tol).contains(&Defect::PastDomain { edge: EdgeId(0) });
                let Ok(boxed) = body.edge_bbox(EdgeId(0)) else {
                    refused += 1;
                    if !past {
                        failures.push(format!(
                            "no box but not refused as past the domain {range:?}; {curve:?}"
                        ));
                    }
                    continue;
                };
                if past {
                    failures.push(format!(
                        "a box but refused as past the domain {range:?}; {curve:?}"
                    ));
                }
                held += 1;
                let edge = &body.edges[0];
                for i in 0..=400 {
                    let t = (range.0 + (range.1 - range.0) * f64::from(i) / 400.0)
                        .clamp(range.0, range.1);
                    let p = edge.curve.eval(t);
                    // Within the domain the box is the control points', as
                    // on d835d891 (padded by topo for rounding).
                    let margin = if (d0..=d1).contains(&t) {
                        1e-12 * k.max(p.x.abs().max(p.y.abs()).max(p.z.abs()))
                    } else {
                        0.0
                    };
                    if !boxed.contains(p, margin) {
                        failures.push(format!(
                            "past the domain {range:?} at {t}: {p:?} outside {boxed:?}; {curve:?}"
                        ));
                        break;
                    }
                }
            }
        }
    }
    println!(
        "{curves} splines: {within} edges within the domain, {held} past it boxed, {refused} refused, {unplaced} not finite at their ends"
    );
    for failure in failures.iter().take(12) {
        println!("FAILED {failure}");
    }
    assert!(failures.is_empty(), "{} failures", failures.len());
}

#[test]
fn an_edge_a_rounding_past_its_domain_is_not_refused_but_past_an_empty_end_span() {
    let mut rng = Rng(0x0e1f_ba5e_2026_1126);
    let (mut curves, mut taken, mut empty_end, mut extreme) = (0_usize, 0_usize, 0_usize, 0_usize);
    let mut failures: Vec<String> = Vec::new();
    while curves < 1500 {
        let Some((curve, k, kind)) = random_curve(&mut rng) else {
            continue;
        };
        curves += 1;
        let tol = Tolerance::new(1e-6 * k, 1e-9).unwrap();
        let (d0, d1) = curve.domain();
        let l = d1 - d0;
        let knots = curve.knots();
        let count = curve.control_points().len();
        let empty = knots[count] <= knots[count - 1];
        for range in [
            (d0.next_down(), d1),
            (d0, d1.next_up()),
            (d0.next_down(), d1.next_up()),
            (d0 - 1e-12 * l, d1 + 1e-12 * l),
        ] {
            let body = edge_alone(&curve, range, &Transform::translation(Vec3::ZERO)).unwrap();
            let past = check(&body, &tol).contains(&Defect::PastDomain { edge: EdgeId(0) });
            if !past {
                taken += 1;
            } else if range.1 > d1 && empty {
                empty_end += 1;
            } else if kind >= 2 {
                // Weights decades apart: the weight carried on may fall to
                // nothing within a rounding of the domain (counted).
                extreme += 1;
            } else {
                failures.push(format!("refused as past the domain {range:?}; {curve:?}"));
            }
        }
    }
    println!(
        "{curves} splines: {taken} edges a rounding past the domain taken, {empty_end} refused past an empty end span, {extreme} with weights decades apart refused"
    );
    for failure in failures.iter().take(12) {
        println!("FAILED {failure}");
    }
    assert!(failures.is_empty(), "{} failures", failures.len());
}
