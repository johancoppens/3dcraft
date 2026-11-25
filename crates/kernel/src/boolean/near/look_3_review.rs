//! The review of the look's round three (2026-11-25; the workshop's
//! `plan/near-look-3/review/`): the bound of two edges ([`edges_apart`])
//! against splines of many spans, rational and clamped or not, against the
//! ends of an ellipse's major axis, arcs of half a turn and more, short
//! edges and centres far from the origin, and against a spline edge whose
//! range runs past its curve's domain (which `NurbsCurve::eval` continues
//! and `craft3d_topo`'s regions read as continued); the escape's cost and
//! its answer to hostile input.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::primitives;
use craft3d_geom::{Circle, Ellipse, Line};
use craft3d_math::Frame;
use craft3d_topo::VertexId;

fn edge(curve: Curve3, range: (f64, f64)) -> Edge {
    Edge {
        curve,
        range,
        start: VertexId(0),
        end: VertexId(0),
    }
}

/// A number in [0, 1) (SplitMix64).
fn random(state: &mut u64) -> f64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 11) as f64 / (1_u64 << 53) as f64
}

fn random_unit(state: &mut u64) -> Vec3 {
    loop {
        let v = Vec3::new(
            2.0 * random(state) - 1.0,
            2.0 * random(state) - 1.0,
            2.0 * random(state) - 1.0,
        );
        if v.length() > 0.2
            && let Some(u) = v.normalized()
        {
            return u;
        }
    }
}

fn random_frame(state: &mut u64, origin: Vec3) -> Frame {
    loop {
        if let Some(f) = Frame::new(origin, random_unit(state), random_unit(state)) {
            return f;
        }
    }
}

/// A spline of degree 1 to 5 with 0 to 7 inner knots, some repeated up to
/// the degree, clamped or not (a uniform knot vector, whose domain starts
/// inside it), its domain anywhere from -3 to 7, polynomial or rational
/// with weights from 1e-3 to 1e3, its control points within `k` of
/// `centre`; and a range inside its domain, now and then from or to a
/// knot or the whole domain.
fn random_spline(state: &mut u64, centre: Vec3, k: f64) -> Edge {
    let p = 1 + (random(state) * 5.0) as usize;
    let n = p + 1 + (random(state) * 8.0) as usize;
    let start = 10.0 * random(state) - 3.0;
    let length = 0.5 + 3.5 * random(state);
    let knots: Vec<f64> = if random(state) < 0.3 {
        let step = length / (n + p) as f64;
        (0..n + p + 1).map(|i| start + step * i as f64).collect()
    } else {
        let mut inner: Vec<f64> = Vec::new();
        while inner.len() < n - p - 1 {
            let u = start + length * (0.05 + 0.9 * random(state));
            let times = if random(state) < 0.3 {
                1 + (random(state) * p as f64) as usize
            } else {
                1
            };
            for _ in 0..times.min(p) {
                if inner.len() < n - p - 1 {
                    inner.push(u);
                }
            }
        }
        inner.sort_by(f64::total_cmp);
        let mut out: Vec<f64> = std::iter::repeat_n(start, p + 1).collect();
        out.extend(inner);
        out.extend(std::iter::repeat_n(start + length, p + 1));
        out
    };
    let points: Vec<Vec3> = (0..n)
        .map(|_| centre + random_unit(state) * (k * random(state)))
        .collect();
    let weights: Vec<f64> = if random(state) < 0.3 {
        vec![1.0; n]
    } else {
        (0..n)
            .map(|_| 10.0_f64.ppowf(6.0 * random(state) - 3.0))
            .collect()
    };
    let curve = NurbsCurve::new(p, knots.clone(), points, weights).unwrap();
    let (d0, d1) = curve.domain();
    let inside: Vec<f64> = knots
        .iter()
        .copied()
        .filter(|&u| u > d0 && u < d1)
        .collect();
    let pick = |state: &mut u64| {
        if !inside.is_empty() && random(state) < 0.3 {
            inside[((random(state) * inside.len() as f64) as usize).min(inside.len() - 1)]
        } else {
            d0 + (d1 - d0) * random(state)
        }
    };
    let range = if random(state) < 0.2 {
        (d0, d1)
    } else {
        let (a, b) = (pick(state), pick(state));
        if a < b {
            (a, b)
        } else if b < a {
            (b, a)
        } else {
            (d0, d1)
        }
    };
    edge(Curve3::Nurbs(curve), range)
}

/// An edge of a kind (0 a line, 1 an arc of a circle, 2 an arc of an
/// ellipse as eccentric as 1000 to 1, 3 a spline of [`random_spline`]) that
/// passes `gap` from `at` (a line, an arc at its middle: an ellipse's at
/// an end of its major axis, its arc centred there; a spline anywhere
/// near), of a size about `k`; arcs from a millionth of a radian to a whole
/// turn, half a turn exactly now and then; lines from a millionth of `k`.
fn random_edge(kind: usize, state: &mut u64, at: Vec3, gap: f64, k: f64) -> Edge {
    let off = at + random_unit(state) * gap;
    let sweep = match (random(state) * 6.0) as usize {
        0 => 1e-6,
        1 => PI,
        2 => PI * (1.0 + 1e-12),
        3 => 2.0 * PI,
        _ => 0.05 + (2.0 * PI - 0.05) * random(state),
    };
    match kind {
        0 => {
            let length = if random(state) < 0.2 {
                1e-6 * k
            } else {
                k * (0.01 + 2.0 * random(state))
            };
            let line = Line::new(off, random_unit(state)).unwrap();
            let shift = length * (random(state) - 0.5);
            edge(
                Curve3::Line(line),
                (shift - 0.5 * length, shift + 0.5 * length),
            )
        }
        1 => {
            let radius = k * (0.05 + 2.0 * random(state));
            let frame = random_frame(state, Vec3::ZERO);
            let frame = Frame::new(off - frame.x() * radius, frame.z(), frame.x()).unwrap();
            edge(
                Curve3::Circle(Circle::new(frame, radius).unwrap()),
                (-0.5 * sweep, 0.5 * sweep),
            )
        }
        2 => {
            let a = k * (0.05 + 2.0 * random(state));
            let b = a * [1.0, 0.3, 0.1, 1e-3][(random(state) * 4.0) as usize % 4];
            let frame = random_frame(state, Vec3::ZERO);
            let frame = Frame::new(off - frame.x() * a, frame.z(), frame.x()).unwrap();
            let at_start = random(state) < 0.5;
            let middle = if at_start { 0.0 } else { PI };
            let frame = if at_start {
                frame
            } else {
                Frame::new(off + frame.x() * a, frame.z(), frame.x()).unwrap()
            };
            edge(
                Curve3::Ellipse(Ellipse::new(frame, a, b).unwrap()),
                (middle - 0.5 * sweep, middle + 0.5 * sweep),
            )
        }
        _ => random_spline(state, off, k),
    }
}

/// The least distance between two edges by dense sampling: 161 points
/// along each, each with its nearest point on the other (a bound from
/// above, between points as evaluated: a `near` above it is above the
/// distance of two points of the edges, however coarse the sampling).
fn sampled(e1: &Edge, e2: &Edge) -> f64 {
    let one_way = |a: &Edge, b: &Edge| {
        (0..=160)
            .map(|i| {
                let t = a.range.0 + (a.range.1 - a.range.0) * f64::from(i) / 160.0;
                let p = a.curve.eval(t);
                outline::nearest(&b.curve, b.range, p).map_or(f64::INFINITY, |(_, d)| d)
            })
            .fold(f64::INFINITY, f64::min)
    };
    one_way(e1, e2).min(one_way(e2, e1))
}

/// A point of an edge at random.
fn somewhere(e: &Edge, state: &mut u64) -> Vec3 {
    e.curve
        .eval(e.range.0 + (e.range.1 - e.range.0) * random(state))
}

/// Whether [`edges_apart`] says the two are apart at `near`, either way
/// round.
fn apart(e1: &Edge, e2: &Edge, near: f64) -> bool {
    let work = Work::new();
    edges_apart(e1, e2, near, &work) || edges_apart(e2, e1, near, &work)
}

/// The bound of two edges never passes over a pair that comes nearer than
/// its `near`: splines of many spans (degree 1 to 5, inner knots repeated,
/// clamped and uniform, rational with weights from 1e-3 to 1e3, ranges
/// from and to knots) against every kind, and every kind against every
/// kind, arcs of ellipses as eccentric as 1000 to 1 centred on an end of
/// their major axis, arcs of a millionth of a radian, of half a turn and a
/// hair more, and of a whole turn, lines a millionth of the size; the
/// second edge passing from a millionth to twice the size from a point of
/// the first; at 1e-3, 1 and 1e6, centred at the origin and a million
/// sizes away. At every `near` above the least distance dense sampling
/// finds (a bound from above of the least there is), never apart. (Run
/// with six pairs per pair of kinds and 401 samples, 576 pairs, in Check
/// 45659572290: none apart, 117 proven apart at half their least; two per
/// pair of kinds and 161 samples here, for the gate's time.)
#[test]
fn the_bound_of_two_edges_holds_for_rational_splines_of_many_spans_and_eccentric_arcs() {
    let mut state = 20_261_125_u64;
    let (mut cases, mut proven) = (0usize, 0usize);
    for k in [1e-3, 1.0, 1e6] {
        for far in [0.0, 1e6] {
            let centre = Vec3::new(3.0, -2.0, 5.0) * (k * (1.0 + far));
            for one in 0..4 {
                for two in 0..4 {
                    for _ in 0..2 {
                        let e1 = if one == 3 {
                            random_spline(&mut state, centre, k)
                        } else {
                            random_edge(one, &mut state, centre, 0.0, k)
                        };
                        let at = somewhere(&e1, &mut state);
                        let gap = k * 10.0_f64.ppowf(-6.0 + 6.3 * random(&mut state));
                        let e2 = random_edge(two, &mut state, at, gap, k);
                        let least = sampled(&e1, &e2);
                        assert!(least.is_finite(), "kinds {one} and {two} at {k}");
                        for fraction in [1.0 + 1e-9, 1.001, 1.5] {
                            let near = fraction * least + 1e-9 * k;
                            assert!(
                                !apart(&e1, &e2, near),
                                "kinds {one} and {two} at {k}, {far} away: apart at {near:e}, sampled {least:e}: {e1:?} {e2:?}"
                            );
                        }
                        cases += 1;
                        proven += usize::from(apart(&e1, &e2, 0.5 * least));
                    }
                }
            }
        }
    }
    println!(
        "look_3_review: edges_apart proved {proven} of {cases} pairs apart at half their least"
    );
}

/// An arc of an ellipse centred on an end of its major axis bulges from
/// its chord by exactly `a (1 − cos(φ/2)) = 2a sin²(φ/4)`, the reach the
/// bound gives it: the bound is tight there, and must still never pass
/// over a short line or a small circle that passes the end of the axis
/// outside the bulge, a third and nine tenths of `near` away. Ellipses
/// from round to 1000 to 1, arcs of a millionth of a radian to a whole
/// turn, at 1e-3, 1 and 1e6, turned in space and a million sizes away.
#[test]
fn the_bound_of_two_edges_holds_at_the_end_of_an_ellipse_s_major_axis() {
    let mut state = 11_u64;
    for k in [1e-3, 1.0, 1e6] {
        for far in [0.0, 1e6] {
            let origin = Vec3::new(-1.0, 4.0, 2.0) * (k * (1.0 + far));
            for ratio in [1.0, 0.1, 1e-3] {
                for sweep in [1e-6, 0.1, 1.0, PI - 1e-9, PI, PI + 1e-9, 4.0, 2.0 * PI] {
                    let frame = random_frame(&mut state, origin);
                    let (a, b) = (k, k * ratio);
                    let ellipse = edge(
                        Curve3::Ellipse(Ellipse::new(frame, a, b).unwrap()),
                        (-0.5 * sweep, 0.5 * sweep),
                    );
                    let end = frame.point(a, 0.0, 0.0);
                    let near = 1e-6 * k;
                    for share in [0.3, 0.9] {
                        let at = end + frame.x() * (share * near);
                        let line = edge(
                            Curve3::Line(Line::new(at, frame.y()).unwrap()),
                            (-1e-3 * k, 1e-3 * k),
                        );
                        let circle_frame =
                            Frame::new(at + frame.x() * (0.01 * k), frame.z(), frame.x()).unwrap();
                        let circle = edge(
                            Curve3::Circle(Circle::new(circle_frame, 0.01 * k).unwrap()),
                            (PI - 0.5, PI + 0.5),
                        );
                        for other in [&line, &circle] {
                            assert!(
                                !apart(&ellipse, other, near),
                                "an arc of {sweep} rad of an ellipse {a} by {b} at {k}, {far} away, passed {share} of near off the end of its axis: proven apart"
                            );
                        }
                    }
                }
            }
        }
    }
}

/// A cubic Bézier piece over [0, 1] whose edge's range runs from -1 to 1,
/// as `NurbsCurve::eval` continues it and `craft3d_topo`'s regions read
/// it (`region.rs` › `each_piece`: "the first and the last piece of the
/// curve continued past its domain as far as the stretch goes"): past its
/// domain the curve leaves the convex hull of its control points (2.85
/// from the chord at most, the curve 3.11 at t ≈ −0.26). A short line
/// 0.1 outside the curve there is 0.1 from the edge; at a `near` of 0.2
/// the bound of two edges must not say apart. The search the bound spares
/// (`least_along` with the nearest points of the line, as `edge_edge` runs
/// it) finds the pair under 0.2.
///
/// Fails on `check/near-look-3` ba61a439 (Check 45659572290: "proven apart
/// at 0.2 from a line 0.1 from it"), the review's blocker: `holding_points`
/// takes the first and the last span's control points for a range past the
/// domain, and `speed_bound` does not hold there either. Passes once
/// [`edges_apart`] proves nothing for a spline whose range leaves its
/// domain (or bounds such a piece as `craft3d_topo`'s regions do, with
/// `craft3d_geom::piece_points_over`).
#[test]
fn a_spline_edge_whose_range_runs_past_its_domain_is_not_proven_apart_from_what_passes_it() {
    let points = vec![
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(1.0, 1.0, 0.0),
        Vec3::new(2.0, -1.0, 0.0),
        Vec3::new(3.0, 0.0, 0.0),
    ];
    let knots = vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0];
    let curve = Curve3::Nurbs(NurbsCurve::polynomial(3, knots, points).unwrap());
    let spline = edge(curve, (-1.0, 1.0));
    // Where the curve is farthest from its chord, and the way out.
    let (a, b) = (spline.curve.eval(-1.0), spline.curve.eval(1.0));
    let chord = (b - a).normalized().unwrap();
    let off = |t: f64| {
        let d = spline.curve.eval(t) - a;
        (d - chord * d.dot(chord)).length()
    };
    let t = golden(-1.0, 0.0, |t| -off(t));
    let p = spline.curve.eval(t);
    let out = {
        let d = p - a;
        (d - chord * d.dot(chord)).normalized().unwrap()
    };
    let at = p + out * 0.1;
    let line = edge(Curve3::Line(Line::new(at, chord).unwrap()), (-0.05, 0.05));
    let near = 0.2;
    let least = sampled(&spline, &line);
    let distance =
        |q: Vec3| outline::nearest(&line.curve, line.range, q).map_or(f64::INFINITY, |(_, d)| d);
    let searched = least_along(&spline.curve, spline.range, near, &distance)
        .into_iter()
        .map(|s| distance(spline.curve.eval(s)))
        .fold(f64::INFINITY, f64::min);
    println!(
        "look_3_review: the spline past its domain: {:.4} from its chord at t = {t:.4}, the line {least:.4} from it, the search finds {searched:.4}",
        off(t)
    );
    assert!(least < near && searched < near, "{least} {searched}");
    assert!(
        !apart(&spline, &line, near),
        "a spline edge past its domain proven apart at {near} from a line {least} from it"
    );
}

/// The escape from a settled pair, on the review's ring slid on a rod one
/// tolerance off its axis (a cylinder and a torus): from the rod's +y,
/// where the projections settle 3.5 tolerances from the ring, it reaches
/// the least, 1.5 tolerances at −y; it costs at most some 1 250 units of
/// work (the curvature's 8 pairs, a line search's 34, a descent's 8 steps
/// of two line searches); and from a point or a reach that is not finite,
/// nothing, zero or negative, or huge, it answers within that work too,
/// with nothing where the reach or the point is not finite.
#[test]
fn the_escape_reaches_the_least_within_its_work_and_answers_hostile_input() {
    let tol = 0.01;
    let t = tol;
    let rod = primitives::cylinder(
        &Frame::new(Vec3::new(-5.0, 0.0, 0.0), Vec3::X, Vec3::Y).unwrap(),
        1.0,
        10.0,
    )
    .unwrap();
    let inside_r = 1.0 + 1.5 * t + t;
    let ring = primitives::torus(
        &Frame::new(Vec3::new(0.0, t, 0.0), Vec3::X, Vec3::Z).unwrap(),
        2.0,
        2.0 - inside_r,
    )
    .unwrap();
    let wall = rod
        .faces
        .iter()
        .position(|f| matches!(f.surface, Surface::Cylinder(_)))
        .unwrap();
    let o1 = section::face_outline(&rod, wall, tol).unwrap();
    let o2 = section::face_outline(&ring, 0, tol).unwrap();
    let held = |body: &Body, f: usize| body.held_face_bbox(FaceId(f as u32)).unwrap();
    let reach = 0.25 * held(&rod, wall).diagonal().min(held(&ring, 0).diagonal());
    let fits = |p: Vec3, q: Vec3| inside(&o1, p, tol) && inside(&o2, q, tol);
    let work = Work::new();
    let found = escape(&o1, &o2, Vec3::new(0.0, 1.0, 0.0), reach, tol, &work, fits);
    let used = work.done();
    let (p, q) = found.expect("the escape finds a way down");
    println!(
        "look_3_review: the escape from the rod's +y: {:.4} tolerances at {p:?}, {used} units",
        p.distance(q) / tol
    );
    assert!(p.distance(q) < 1.6 * tol && p.y < 0.0, "{p:?} {q:?}");
    assert!(used <= 1300, "{used} units");
    let points = [
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::new(f64::NAN, 0.0, 0.0),
        Vec3::new(f64::INFINITY, 0.0, 0.0),
        Vec3::new(1e300, -1e300, 1e300),
        Vec3::new(0.0, 0.0, 0.0),
    ];
    let reaches = [reach, 0.0, -1.0, f64::NAN, f64::INFINITY, 1e-300, 1e300];
    for &start in &points {
        for &r in &reaches {
            let work = Work::new();
            let answer = escape(&o1, &o2, start, r, tol, &work, fits);
            assert!(
                work.done() <= 1300,
                "from {start:?} with reach {r}: {} units",
                work.done()
            );
            if !(start.is_finite() && r.is_finite() && r > 0.0) {
                assert!(
                    answer.is_none(),
                    "from {start:?} with reach {r}: {answer:?}"
                );
            }
            if let Some((p, q)) = answer {
                assert!(
                    p.is_finite() && q.is_finite(),
                    "from {start:?} with reach {r}"
                );
            }
        }
    }
}
