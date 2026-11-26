//! The review of `check/spline-box-domain` (a93e8aea, 2026-11-26): the
//! speed, turn and bend bounds of a spline ([`speed_bound`],
//! [`turn_bound`], [`bend_bound`]) and the boolean's box of a piece of one
//! (`outline.rs` › `piece_box`) now take the range.
//!
//! Within the domain they are those of `d835d891`, bit for bit: that
//! version's spline arms are kept here word for word as the reference (a
//! line, a circle and an ellipse are untouched), on random splines of
//! every kind (degrees 1 to 12, 16, 24 and 32; weights all one to six
//! decades either side of one, and one control point a million times
//! heavier than the rest; clamped and unclamped ends, spans a millionth
//! of the others' width; control points at 1e-3 to 1e6, at the origin and
//! far from it), over ranges within the domain of every kind.
//!
//! Past the domain they hold over the range: no point of the curve
//! outside the box of the piece, no speed above the speed bound (to a
//! millionth: the bounds carried on are not padded for rounding, and
//! `nurbs_continued_review.rs` measured the speed 2.4e-8 of the bound above
//! it). The turn bound past the domain is the larger of the one within it
//! and `continued_derivative_bounds`' second, which that file holds to the
//! curve (run 4 of the review, Check 45763699075, also measured the mean
//! second derivative across the domain, which a knot of the degree's
//! multiplicity, where the first derivative jumps, does not bound: taken
//! out).

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use super::{bend_bound, speed_bound, turn_bound};
use crate::boolean::outline::piece_box;
use craft3d_geom::{Curve3, Cylinder, NurbsCurve, Plane, Sphere, Surface, Torus};
use craft3d_math::{Bbox3, Frame, Portable, Vec3};

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

/// A random spline and its size; `None` for knots that leave no domain.
fn random_curve(rng: &mut Rng) -> Option<(NurbsCurve, f64)> {
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
        .map(|c| (c, k))
}

/// `speed_bound` of a spline on `d835d891`, word for word.
fn reference_speed(c: &NurbsCurve) -> f64 {
    // The derivative of a B-spline is a B-spline over the scaled
    // differences of its control points; for a rational curve the
    // differences of the weighted points, seen from a point of the
    // curve, are bounded through the size of the control polygon.
    let degree = c.degree();
    let (knots, points, weights) = (c.knots(), c.control_points(), c.weights());
    let size = Bbox3::from_points(points.iter().copied()).diagonal();
    let (mut lightest, mut heaviest) = (f64::INFINITY, 0.0_f64);
    for &w in weights {
        lightest = lightest.min(w);
        heaviest = heaviest.max(w);
    }
    let mut bound = 0.0_f64;
    for (i, (pair, weight)) in points.windows(2).zip(weights.windows(2)).enumerate() {
        let (Some(&lo), Some(&hi)) = (knots.get(i + 1), knots.get(i + degree + 1)) else {
            continue;
        };
        let (&[a, b], &[wa, wb]) = (pair, weight) else {
            continue;
        };
        if hi > lo {
            let reach = heaviest * a.distance(b) + (wb - wa).abs() * size;
            bound = bound.max(degree as f64 * reach / (hi - lo));
        }
    }
    bound / lightest
}

/// `turn_bound` of a spline on `d835d891`, word for word.
fn reference_turn(c: &NurbsCurve) -> Option<f64> {
    if c.weights().iter().any(|&w| (w - 1.0).abs() > 1e-12) {
        return c.derivative_bounds().map(|(_, second)| second);
    }
    let (degree, knots, points) = (c.degree(), c.knots(), c.control_points());
    if degree < 2 {
        return Some(0.0);
    }
    let first: Vec<Vec3> = points
        .windows(2)
        .enumerate()
        .map(|(i, pair)| {
            let (Some(&lo), Some(&hi), &[a, b]) =
                (knots.get(i + 1), knots.get(i + degree + 1), pair)
            else {
                return Vec3::ZERO;
            };
            if hi > lo {
                (b - a) * (degree as f64 / (hi - lo))
            } else {
                Vec3::ZERO
            }
        })
        .collect();
    let mut bound = 0.0_f64;
    for (i, pair) in first.windows(2).enumerate() {
        let (Some(&lo), Some(&hi), &[a, b]) = (knots.get(i + 2), knots.get(i + degree + 1), pair)
        else {
            continue;
        };
        if hi > lo {
            bound = bound.max((b - a).length() * ((degree - 1) as f64) / (hi - lo));
        }
    }
    bound.is_finite().then_some(bound)
}

/// `bend_bound` on `d835d891`, word for word, of a spline (with the two
/// above).
fn reference_bend(c: &NurbsCurve, surface: &Surface) -> Option<(f64, f64)> {
    let (curvature, zone) = match surface {
        Surface::Plane(_) => (0.0, f64::INFINITY),
        Surface::Cylinder(s) => (2.0 / s.radius(), 0.25 * s.radius()),
        Surface::Sphere(s) => (2.0 / s.radius(), 0.25 * s.radius()),
        Surface::Torus(s) => {
            let least = s.radii().1.min(s.radii().0 - s.radii().1);
            (2.0 / least, 0.25 * least)
        }
        Surface::Cone(_) | Surface::Bezier(_) | Surface::Nurbs(_) => return None,
    };
    let speed = reference_speed(c);
    let turn = reference_turn(c)?;
    let bound = speed * speed * curvature + turn;
    (bound.is_finite() && zone > 0.0).then_some((bound, zone))
}

/// `piece_box` of a spline on `d835d891`: the ends and the control points.
fn reference_box(c: &NurbsCurve, a: Vec3, b: Vec3) -> Bbox3 {
    let mut bx = Bbox3::from_points([a, b]);
    for &p in c.control_points() {
        bx.add(p);
    }
    bx
}

/// The bits of a box's corners.
fn corners(bx: &Bbox3) -> Option<[u64; 6]> {
    bx.corners()
        .map(|(lo, hi)| [lo.x, lo.y, lo.z, hi.x, hi.y, hi.z].map(f64::to_bits))
}

/// Surfaces of every kind `bend_bound` reads, about the curve's size.
fn surfaces(k: f64) -> [Surface; 4] {
    let f = Frame::new(
        Vec3::new(1.0, -2.0, 0.5) * k,
        Vec3::new(0.3, -0.4, 1.0),
        Vec3::new(1.0, 0.2, 0.0),
    )
    .unwrap();
    [
        Surface::Plane(Plane::new(f)),
        Surface::Cylinder(Cylinder::new(f, 2.0 * k).unwrap()),
        Surface::Sphere(Sphere::new(f, 3.0 * k).unwrap()),
        Surface::Torus(Torus::new(f, 4.0 * k, k).unwrap()),
    ]
}

#[test]
fn within_the_domain_the_bounds_and_the_box_of_a_spline_are_those_of_d835d891_bit_for_bit() {
    let mut rng = Rng(0xb0dd_ed00_2026_1126);
    let (mut curves, mut compared) = (0_usize, 0_usize);
    while curves < 800 {
        let Some((c, k)) = random_curve(&mut rng) else {
            continue;
        };
        curves += 1;
        let curve = Curve3::Nurbs(c.clone());
        let (d0, d1) = c.domain();
        let l = d1 - d0;
        let mid = 0.5 * (d0 + d1);
        let speed = reference_speed(&c).to_bits();
        let turn = reference_turn(&c).map(f64::to_bits);
        let bends: Vec<Option<[u64; 2]>> = surfaces(k)
            .iter()
            .map(|s| reference_bend(&c, s).map(|(b, z)| [b.to_bits(), z.to_bits()]))
            .collect();
        for range in [
            (d0, d1),
            (d1, d0),
            (d0, mid),
            (mid, d1),
            (d0 + 0.3 * l, d0 + 0.31 * l),
            (mid, mid),
            (d0, d0),
            (d1, d1),
            (f64::NAN, f64::NAN),
        ] {
            compared += 1;
            assert_eq!(
                speed_bound(&curve, range).to_bits(),
                speed,
                "speed {range:?}: {c:?}"
            );
            assert_eq!(
                turn_bound(&curve, range).map(f64::to_bits),
                turn,
                "turn {range:?}: {c:?}"
            );
            for (surface, bend) in surfaces(k).iter().zip(&bends) {
                assert_eq!(
                    bend_bound(&curve, range, surface).map(|(b, z)| [b.to_bits(), z.to_bits()]),
                    *bend,
                    "bend {range:?}: {c:?}"
                );
            }
            if range.0.is_finite() {
                let (a, b) = (c.eval(range.0), c.eval(range.1));
                assert_eq!(
                    corners(&piece_box(&curve, range, a, b)),
                    corners(&reference_box(&c, a, b)),
                    "box {range:?}: {c:?}"
                );
            }
        }
    }
    println!("{curves} splines, {compared} ranges within the domain: as on d835d891");
}

#[test]
fn past_the_domain_the_bounds_and_the_box_of_a_spline_hold_it() {
    let mut rng = Rng(0x0a57_d0a1_2026_1126);
    let (mut curves, mut held, mut unbounded, mut no_turn) = (0_usize, 0_usize, 0_usize, 0_usize);
    while curves < 800 {
        let Some((c, k)) = random_curve(&mut rng) else {
            continue;
        };
        curves += 1;
        let curve = Curve3::Nurbs(c.clone());
        let (d0, d1) = c.domain();
        let l = d1 - d0;
        let mid = 0.5 * (d0 + d1);
        let s = [1e-9, 0.01, 0.3, 1.0, 3.0][rng.below(5)];
        for range in [
            (d0 - s * l, mid),
            (mid, d1 + s * l),
            (d0 - s * l, d1 + s * l),
            (d1 + 0.5 * s * l, d1 + s * l),
        ] {
            // An edge nothing bounds there is refused by topo's check.
            if !c.runs_past_domain(range) || c.continued_hull(range).is_none() {
                continue;
            }
            let (a, b) = (c.eval(range.0), c.eval(range.1));
            let bx = piece_box(&curve, range, a, b);
            let speed = speed_bound(&curve, range);
            let turn = turn_bound(&curve, range);
            held += 1;
            unbounded += usize::from(!speed.is_finite());
            no_turn += usize::from(turn.is_none());
            for i in 0..=256 {
                let t =
                    (range.0 + (range.1 - range.0) * f64::from(i) / 256.0).clamp(range.0, range.1);
                let p = c.eval(t);
                let size = p.x.abs().max(p.y.abs()).max(p.z.abs());
                // Within the domain the box is unpadded, as on d835d891.
                let margin = if (d0..=d1).contains(&t) {
                    1e-12 * size.max(k)
                } else {
                    0.0
                };
                assert!(
                    bx.contains(p, margin),
                    "{range:?} at {t}: {p:?} outside {bx:?}; {c:?}"
                );
                let fast = c.derivatives(t).1.length();
                assert!(
                    fast <= speed * (1.0 + 1e-6),
                    "{range:?} at {t}: speed {fast} > {speed}; {c:?}"
                );
            }
        }
    }
    println!(
        "{curves} splines, {held} ranges past the domain held ({unbounded} with no finite speed bound, {no_turn} with no turn bound)"
    );
    assert!(held > 500, "{held}");
}
