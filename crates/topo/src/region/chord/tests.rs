//! The steps of the plane theorem ([`super`]), each held to its statement.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use super::*;
use crate::{BodyKind, Coedge};
use craft3d_geom::{Cylinder, Line, NurbsCurve, Plane};
use craft3d_math::Frame;
use std::f64::consts::TAU;

fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

fn tilted(k: f64) -> Frame {
    Frame::new(
        Vec3::new(1.3, -2.1, 0.7) * (10.0 * k),
        Vec3::new(-0.3, 0.8, 0.52),
        Vec3::new(0.6, 0.1, 0.4),
    )
    .unwrap()
}

/// A one-face sheet on the plane of `frame` whose loops are the polygons
/// `loops` (chart points times `k`), each side a line; `short` shortens the
/// first side of the first loop by that much at its end, so that it misses
/// the next side's start.
fn polygons(frame: Frame, k: f64, loops: &[&[(f64, f64)]], reversed: bool, short: f64) -> Body {
    let mut body = Body::new(BodyKind::Sheet);
    let mut ids = Vec::new();
    for (l, points) in loops.iter().enumerate() {
        let at = |i: usize| {
            let (x, y) = points[i % points.len()];
            frame.point(k * x, k * y, 0.0)
        };
        let vertices: Vec<_> = (0..points.len()).map(|i| body.add_vertex(at(i))).collect();
        let coedges = (0..points.len())
            .map(|i| {
                let (a, b) = (at(i), at(i + 1));
                let cut = if l == 0 && i == 0 { short } else { 0.0 };
                let edge = body.add_edge(
                    Curve3::Line(Line::through(a, b).unwrap()),
                    (0.0, a.distance(b) - cut),
                    vertices[i],
                    vertices[(i + 1) % points.len()],
                );
                Coedge {
                    edge,
                    reversed: false,
                }
            })
            .collect();
        ids.push(body.add_loop(coedges));
    }
    let face = body.add_face(Surface::Plane(Plane::new(frame)), reversed, ids);
    body.add_shell(vec![face]);
    body
}

/// The second walk's verdict of a body's only face.
fn verdict(body: &Body, k: f64) -> Option<Judged> {
    let tol = tolerance(k);
    Verifier::new(body, &body.faces[0], &tol).settle()
}

const SQUARE: &[(f64, f64)] = &[(0.0, 0.0), (4.0, 0.0), (4.0, 3.0), (0.0, 3.0)];
/// A hole of the square, running clockwise.
const INSIDE: &[(f64, f64)] = &[(1.0, 1.0), (1.0, 2.0), (2.0, 2.0), (2.0, 1.0)];
/// The same beside the square.
const OUTSIDE: &[(f64, f64)] = &[(5.0, 1.0), (5.0, 2.0), (6.0, 2.0), (6.0, 1.0)];
/// A hole inside [`INSIDE`].
const NESTED: &[(f64, f64)] = &[(1.2, 1.2), (1.2, 1.8), (1.8, 1.8), (1.8, 1.2)];

/// Neighbouring stretches are shown apart only where their cones fit
/// in a fan narrower than a half turn by what the gap and the slacks
/// ask: a corner and a tangent join are; a hairpin, a stretch with no
/// aim, a gap or slack of a quarter of the reach are not. And the claim
/// itself, sampled: two rays from either end of a gap, each in its
/// cone, meet (where they do) within half the reach of the first's end.
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
    // A gap or slack of a quarter of the reach or more, no aim, a gap or a
    // reach that is no number: never apart.
    assert!(!neighbours_apart(east, east, 0.25 * reach, reach));
    assert!(!neighbours_apart(
        east,
        aim(0.0, 0.0, 0.25 * reach),
        0.0,
        reach
    ));
    assert!(!neighbours_apart(None, east, 0.0, reach));
    assert!(!neighbours_apart(east, east, f64::NAN, reach));
    assert!(!neighbours_apart(east, east, 0.0, f64::NAN));
    assert!(!neighbours_apart(east, east, 0.0, f64::INFINITY));
    assert!(!neighbours_apart(east, east, 0.0, 0.0));
    // Sampled: the first stretch runs into `E` along a direction of its
    // cone, the second out of `P = E + gap` along one of its own; where
    // two such straight stretches cross, they do within half the reach of
    // E.
    let (mut checked, mut met) = (0, 0);
    for i in 0..40 {
        let angle_a = -3.0 + 0.15 * f64::from(i);
        for j in 0..40 {
            let angle_b = -3.0 + 0.15 * f64::from(j);
            for g in [0.0, 0.05, 0.2, 0.249] {
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
                            x.0.phypot(x.1) <= 0.5 * reach * (1.0 + 1e-9),
                            "angles {angle_a}, {angle_b}, gap {gap}: they meet {} from E",
                            x.0.phypot(x.1)
                        );
                        assert!(
                            gap > 0.0 || x.0.phypot(x.1) <= 1e-12 * reach,
                            "angles {angle_a}, {angle_b}, no gap: they meet {} from E",
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

/// (C2)'s test of two segments: near where they cross, touch, lie over each
/// other or come within the margin; apart otherwise, two on one line among
/// them (two holes in a row); a number that is none counts as near.
#[test]
fn segments_are_near_where_they_meet_or_come_within_the_margin() {
    let m = 1e-5;
    let seg = |x0: f64, y0: f64, x1: f64, y1: f64| ([x0, y0], [x1, y1]);
    let is_near = |(a, b): ([f64; 2], [f64; 2]), (c, d): ([f64; 2], [f64; 2]), margin: f64| {
        let one = near(a, b, c, d, margin);
        // Either way round, and either way along each.
        for (p, q, r, s) in [(c, d, a, b), (b, a, c, d), (a, b, d, c), (d, c, b, a)] {
            assert_eq!(near(p, q, r, s, margin), one, "{a:?} {b:?} {c:?} {d:?}");
        }
        one
    };
    // Crossing, touching at an end, a T, over each other, one inside the
    // other, the same segment.
    assert!(is_near(seg(0.0, 0.0, 2.0, 2.0), seg(0.0, 2.0, 2.0, 0.0), m));
    assert!(is_near(seg(0.0, 0.0, 1.0, 0.0), seg(1.0, 0.0, 1.0, 1.0), m));
    assert!(is_near(seg(0.0, 0.0, 2.0, 0.0), seg(1.0, 0.0, 1.0, 1.0), m));
    assert!(is_near(seg(0.0, 0.0, 2.0, 0.0), seg(1.0, 0.0, 3.0, 0.0), m));
    assert!(is_near(seg(0.0, 0.0, 3.0, 0.0), seg(1.0, 0.0, 2.0, 0.0), m));
    assert!(is_near(seg(0.0, 0.0, 3.0, 1.0), seg(0.0, 0.0, 3.0, 1.0), m));
    // Within the margin, and just past it: beside each other, end to end
    // on one line, an end beside the other's middle.
    assert!(is_near(
        seg(0.0, 0.0, 1.0, 0.0),
        seg(0.0, 0.9 * m, 1.0, 0.9 * m),
        m
    ));
    assert!(!is_near(
        seg(0.0, 0.0, 1.0, 0.0),
        seg(0.0, 1.1 * m, 1.0, 1.1 * m),
        m
    ));
    assert!(is_near(
        seg(0.0, 0.0, 1.0, 0.0),
        seg(1.0 + 0.9 * m, 0.0, 2.0, 0.0),
        m
    ));
    assert!(!is_near(
        seg(0.0, 0.0, 1.0, 0.0),
        seg(1.0 + 1.1 * m, 0.0, 2.0, 0.0),
        m
    ));
    assert!(is_near(
        seg(0.0, 0.0, 2.0, 0.0),
        seg(1.0, 0.9 * m, 1.0, 1.0),
        m
    ));
    assert!(!is_near(
        seg(0.0, 0.0, 2.0, 0.0),
        seg(1.0, 1.1 * m, 1.0, 1.0),
        m
    ));
    // Two on one line, far apart; two that would cross were they longer.
    assert!(!is_near(
        seg(0.0, 0.0, 1.0, 0.0),
        seg(3.0, 0.0, 4.0, 0.0),
        m
    ));
    assert!(!is_near(
        seg(0.0, 0.0, 1.0, 1.0),
        seg(3.0, 3.0, 5.0, 5.0),
        m
    ));
    assert!(!is_near(
        seg(0.0, 0.0, 1.0, 0.0),
        seg(2.0, -1.0, 2.0, 1.0),
        m
    ));
    assert!(!is_near(
        seg(0.0, 0.0, 1.0, 0.0),
        seg(0.5, 1.0, 0.5, 2.0),
        m
    ));
    // Segments of no length.
    assert!(is_near(seg(1.0, 0.0, 1.0, 0.0), seg(0.0, 0.0, 2.0, 0.0), m));
    assert!(!is_near(
        seg(1.0, 1.0, 1.0, 1.0),
        seg(0.0, 0.0, 2.0, 0.0),
        m
    ));
    assert!(is_near(seg(1.0, 1.0, 1.0, 1.0), seg(1.0, 1.0, 1.0, 1.0), m));
    // Numbers that are none: near.
    assert!(near([f64::NAN, 0.0], [1.0, 0.0], [3.0, 0.0], [4.0, 0.0], m));
    assert!(near(
        [0.0, 0.0],
        [1.0, 0.0],
        [3.0, 0.0],
        [4.0, 0.0],
        f64::NAN
    ));
    assert!(near(
        [0.0, 0.0],
        [1.0, 0.0],
        [3.0, 0.0],
        [f64::INFINITY, 0.0],
        m
    ));
    // Sampled against the distance between points of the two: never apart
    // where two of their points are within the margin.
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let mut next = || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((state >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0
    };
    let (mut apart, mut close) = (0, 0);
    for _ in 0..20_000 {
        let (a, b) = ([next(), next()], [next(), next()]);
        let (c, d) = ([next(), next()], [next(), next()]);
        let margin = 0.2 * next().abs();
        let mut least = f64::INFINITY;
        for i in 0..=24 {
            for j in 0..=24 {
                let (s, t) = (f64::from(i) / 24.0, f64::from(j) / 24.0);
                let p = [a[0] + s * (b[0] - a[0]), a[1] + s * (b[1] - a[1])];
                let q = [c[0] + t * (d[0] - c[0]), c[1] + t * (d[1] - c[1])];
                least = least.min((p[0] - q[0]).phypot(p[1] - q[1]));
            }
        }
        if near(a, b, c, d, margin) {
            close += 1;
            // Then the sampled distance is within the margin and a step of
            // the samples.
            assert!(
                least <= margin + 0.25,
                "{a:?} {b:?} {c:?} {d:?}: {least} past {margin}"
            );
        } else {
            apart += 1;
            assert!(
                least > margin,
                "{a:?} {b:?} {c:?} {d:?}: {least} within {margin}"
            );
        }
    }
    assert!(
        apart > 2_000 && close > 2_000,
        "{apart} apart, {close} near"
    );
}

/// A face that is not on a plane is not walked again at all: the walk's
/// fault stands, whatever it says, and nothing is counted. A band of a
/// cylinder between two circles.
#[test]
fn a_face_off_a_plane_is_not_walked_again() {
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
    let tol = tolerance(1.0);
    let face = &body.faces[0];
    assert!(!asked(&face.surface));
    assert!(flat(&face.surface).is_none());
    assert!(Walked::new(&body, face, &tol).is_none());
    let before = crate::region::chord_proofs();
    let original = "its loops 0 and 1 cross";
    let mut verifier = Verifier::new(&body, face, &tol);
    assert_eq!(verifier.region_shown(original).as_deref(), Some(original));
    assert!(verifier.winding_shown());
    assert_eq!(crate::region::chord_proofs(), before, "nothing counted");
}

/// A closed curve of degree `degree` through `count + 1` points of the
/// chart's wavy ring `(0.4 + a sin(waves θ)) (cos θ, sin θ) + (0, 0.1)`
/// taken to the plane; with `rational`, through them with weights
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

/// Every stretch shown holds what it says ((S1), (S2) and (L3) of the
/// module), against 64 points of its image placed on the plane: within its
/// bound of its chord, every secant in its cone, running one way; and the
/// area the sampled loop bounds is its polyline's within the error claimed.
/// Closed wavy splines of degrees 1 to 7, through many points or as few as
/// one span (bulging far from their chords), rational and not, on a plane
/// in the world and turned and moved, at three scales, as walked and halved
/// three times over (so that both the spans' control points and the cut
/// Bézier pieces are read).
#[test]
fn a_stretch_shown_lies_within_its_bound_and_runs_one_way() {
    let (mut shown, mut walked_faces, mut stretches) = (0, 0, 0_usize);
    let mut coned = 0_usize;
    for k in [1e-3, 1.0, 1e6] {
        for frame in [Frame::WORLD, tilted(k)] {
            let surface = Surface::Plane(Plane::new(frame));
            let at = move |x: f64, y: f64| frame.point(k * x, k * y, 0.0);
            for (degree, count, waves, a, rational) in [
                (3, 96, 7.0, 0.08, false),
                (5, 64, 5.0, 0.12, true),
                (7, 7, 3.0, 0.15, false),
                (7, 7, 2.0, 0.2, true),
                (2, 40, 9.0, 0.05, false),
                (4, 12, 3.0, 0.25, false),
                (3, 200, 11.0, 0.05, false),
                (3, 24, 4.0, 0.2, true),
                (6, 30, 6.0, 0.1, false),
                (2, 12, 2.0, 0.3, true),
                (5, 16, 3.0, 0.3, false),
                (1, 60, 5.0, 0.1, false),
                (1, 9, 2.0, 0.2, false),
            ] {
                // A rational fit through few points can have a weight that
                // is not positive: no such curve.
                let Some(curve) = ring_curve(&at, waves, a, count, degree, rational) else {
                    assert!(rational, "degree {degree}");
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
                let tol = tolerance(k);
                let face = &body.faces[0];
                let mut walked = Walked::new(&body, face, &tol).unwrap();
                walked_faces += 1;
                if walked.show_all().is_none() {
                    eprintln!(
                        "not shown: at {k}, degree {degree} through {count}, rational {rational}, work {}",
                        walked.work
                    );
                    continue;
                }
                shown += 1;
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
                    let mut sampled: Vec<[f64; 2]> = Vec::new();
                    for st in &all {
                        let Bound::Shown(bound, aim) = st.bound else {
                            panic!("a stretch not shown after show_all");
                        };
                        stretches += 1;
                        let (start, end) = (st.start, st.end);
                        let mut before = start;
                        let mut far = 0.0_f64;
                        let mut turned: Vec<f64> = Vec::new();
                        let mut places: Vec<[f64; 2]> = Vec::new();
                        for i in 0..=64 {
                            let t = st.s + (st.e - st.s) * i as f64 / 64.0;
                            let (u, v) = surface.project(curve.eval(t)).unwrap();
                            let q = [u, v];
                            far = far.max(to_segment(q, start, end));
                            let d = (q[0] - before[0], q[1] - before[1]);
                            if i > 0 && (d.0 != 0.0 || d.1 != 0.0) {
                                let mut angle = d.1.patan2(d.0);
                                if let Some(first) = turned.first() {
                                    angle += ((first - angle) / TAU).round() * TAU;
                                }
                                turned.push(angle);
                            }
                            places.push(q);
                            before = q;
                        }
                        sampled.extend(places.iter().take(64).copied());
                        let size = start[0].abs().max(start[1].abs()).max(k);
                        assert!(
                            far <= bound + 1e-12 * size,
                            "at {k}, degree {degree} through {count}: {far} off its chord, bound {bound}"
                        );
                        // Every move from a point to a later one lies in
                        // the cone (to the rounding of its ends).
                        if let Some(Aim {
                            cone: (c0, c1),
                            slack,
                        }) = aim
                        {
                            coned += 1;
                            let unit = |c: (f64, f64)| {
                                let n = c.0.phypot(c.1);
                                (c.0 / n, c.1 / n)
                            };
                            let (c0, c1) = (unit(c0), unit(c1));
                            assert!(
                                c0.0 * c1.1 - c0.1 * c1.0 > -1e-12
                                    && (c0.0 * c1.1 - c0.1 * c1.0 > 1e-12
                                        || c0.0 * c1.0 + c0.1 * c1.1 > 0.0),
                                "a cone of a half turn or more"
                            );
                            for i in 0..places.len() {
                                for j in [i + 1, i + 7, i + 23, 64] {
                                    let (Some(p), Some(q)) = (places.get(i), places.get(j)) else {
                                        continue;
                                    };
                                    if j <= i {
                                        continue;
                                    }
                                    let d = (q[0] - p[0], q[1] - p[1]);
                                    let slack = 1e-9 * d.0.phypot(d.1) + 1e-12 * size + slack;
                                    assert!(
                                        c0.0 * d.1 - c0.1 * d.0 >= -slack
                                            && d.0 * c1.1 - d.1 * c1.0 >= -slack,
                                        "at {k}, degree {degree} through {count}: the move {d:?} outside the cone {c0:?} to {c1:?}"
                                    );
                                }
                            }
                        }
                        let low = turned.iter().copied().fold(f64::INFINITY, f64::min);
                        let high = turned.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                        assert!(
                            turned.is_empty() || high - low < PI,
                            "at {k}, degree {degree} through {count}: turns {}",
                            high - low
                        );
                    }
                    // (L3): the sampled loop's area against the polyline's.
                    let rings = walked.rings().unwrap();
                    let ring = &rings[0];
                    let error = area_error(&ring.ring.points, &ring.margins);
                    let (dense, coarse) = (signed_area(&sampled), signed_area(&ring.ring.points));
                    assert!(
                        (dense - coarse).abs() <= error * (1.0 + 1e-3) + 1e-12 * k * k,
                        "at {k}, degree {degree} through {count}, level {level}: the loop bounds {dense}, its polyline {coarse}, error {error}"
                    );
                }
            }
        }
    }
    // Every face walked has every stretch shown, the few-span ones
    // included.
    assert_eq!(shown, walked_faces, "of {walked_faces}");
    eprintln!("{walked_faces} faces walked, {stretches} stretches, {coned} with a cone");
    assert!(walked_faces >= 60, "{walked_faces} faces walked");
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

/// A spline with a knot inside its domain repeated more often than its
/// degree jumps there: it is not one curve, no stretch that holds the jump
/// or ends on it is shown, and a face with such an edge is never cleared
/// (so its fault stands). A knot repeated as often as the degree is a
/// corner, and at the ends of the domain a knot may be repeated once more.
#[test]
fn a_spline_that_jumps_is_not_one_curve() {
    let p = |x: f64, y: f64| Vec3::new(x, y, 0.0);
    let all = |curve: &NurbsCurve| jumps(curve, 0, usize::MAX);
    let corner = NurbsCurve::polynomial(
        1,
        vec![0.0, 0.0, 0.5, 1.0, 1.0],
        vec![p(0.0, 0.0), p(4.0, 0.0), p(4.0, 3.0)],
    )
    .unwrap();
    assert!(!all(&corner));
    let jump = NurbsCurve::polynomial(
        1,
        vec![0.0, 0.0, 0.5, 0.5, 1.0, 1.0],
        vec![p(0.0, 0.0), p(3.9, 0.0), p(4.0, 0.1), p(4.0, 3.0)],
    )
    .unwrap();
    assert!(all(&jump));
    // Over the knots before the jump and after it, none; a knot of the
    // jump's value is looked at with every knot of that value, so a window
    // that holds one of the two sees the jump (round 4: the count was of
    // the window's own knots, and a window that cut the run saw none).
    assert!(!jumps(&jump, 0, 1) && !jumps(&jump, 4, 5));
    assert!(jumps(&jump, 0, 2) && jumps(&jump, 3, 5));
    let cubic = NurbsCurve::polynomial(
        3,
        vec![0.0, 0.0, 0.0, 0.0, 0.5, 0.5, 0.5, 1.0, 1.0, 1.0, 1.0],
        (0..7).map(|i| p(f64::from(i), f64::from(i % 2))).collect(),
    )
    .unwrap();
    assert!(!all(&cubic));
    let cubic_jump = NurbsCurve::polynomial(
        3,
        vec![0.0, 0.0, 0.0, 0.0, 0.5, 0.5, 0.5, 0.5, 1.0, 1.0, 1.0, 1.0],
        (0..8).map(|i| p(f64::from(i), f64::from(i % 2))).collect(),
    )
    .unwrap();
    assert!(all(&cubic_jump));
    // A triangle whose first two sides are one spline: with the corner it
    // is cleared; with the jump (a tenth of a unit open) no stretch that
    // ends on the jump is shown, and there is no verdict. The same through
    // many spans, where the spans' control points are read: the two sides
    // as a spline of degree 1 through 41 points a side.
    let many = |jump: bool| {
        let mut knots = vec![0.0];
        let mut ctrl = Vec::new();
        for i in 0..=40 {
            ctrl.push(p(0.1 * f64::from(i), 0.0));
            knots.push(f64::from(i) / 80.0);
        }
        for i in 0..=40 {
            if i > 0 || jump {
                let share = f64::from(i) / 40.0;
                ctrl.push(p(4.0, if jump { 0.1 + 2.9 * share } else { 3.0 * share }));
                knots.push(0.5 + f64::from(i) / 80.0);
            }
        }
        knots.push(1.0);
        NurbsCurve::polynomial(1, knots, ctrl).unwrap()
    };
    assert!(!all(&many(false)) && all(&many(true)));
    for (curve, clears) in [
        (corner, true),
        (jump, false),
        (many(false), true),
        (many(true), false),
    ] {
        let mut body = Body::new(BodyKind::Sheet);
        let (a, b) = (body.add_vertex(p(0.0, 0.0)), body.add_vertex(p(4.0, 3.0)));
        let sides = body.add_edge(Curve3::Nurbs(curve), (0.0, 1.0), a, b);
        let back = body.add_edge(
            Curve3::Line(Line::through(p(4.0, 3.0), p(0.0, 0.0)).unwrap()),
            (0.0, 5.0),
            b,
            a,
        );
        let lp = body.add_loop(vec![
            Coedge {
                edge: sides,
                reversed: false,
            },
            Coedge {
                edge: back,
                reversed: false,
            },
        ]);
        let face = body.add_face(Surface::Plane(Plane::new(Frame::WORLD)), false, vec![lp]);
        body.add_shell(vec![face]);
        assert_eq!(
            verdict(&body, 1.0),
            clears.then_some(Judged::Clear),
            "the verdict"
        );
    }
}

/// A plate (three lines) whose upper side, from `(4, 3)` to `(0, 3)`, is a
/// rational spline of degree `degree` in two Bézier halves that meet at
/// the parameter 0.3, its weights `ratio` apart: one curve with a corner
/// there (`jump` nothing: the knot stands `degree` times) or one that stops
/// there and starts again `jump` further on along the side or, with
/// `sideways`, into the plate (the knot stands once more). In the frame
/// given, the chart times `k`, the plate moved by `off` within its plane.
fn plate_with_a_spline_side(
    (frame, off): (Frame, (f64, f64)),
    k: f64,
    degree: usize,
    ratio: f64,
    jump: f64,
    sideways: bool,
    short: f64,
) -> Body {
    let at = |x: f64, y: f64| frame.point(k * (x + off.0), k * (y + off.1), 0.0);
    let middle = 4.0 - 4.0 * 0.3;
    let (dx, dy) = if sideways { (0.0, -jump) } else { (-jump, 0.0) };
    let mut ctrl = Vec::new();
    let mut weights = Vec::new();
    let heavy = ratio.sqrt();
    for i in 0..=degree {
        let t = i as f64 / degree as f64;
        ctrl.push(at(4.0 + (middle - 4.0) * t, 3.0));
        weights.push(if i == 0 || i == degree {
            1.0
        } else if i % 2 == 1 {
            heavy
        } else {
            1.0 / heavy
        });
    }
    let from = if jump == 0.0 { 1 } else { 0 };
    for i in from..=degree {
        let t = i as f64 / degree as f64;
        let (x0, y0) = (middle + dx / k, 3.0 + dy / k);
        ctrl.push(at(x0 + (0.0 - x0) * t, y0 + (3.0 - y0) * t));
        weights.push(if i == 0 || i == degree {
            1.0
        } else if i % 2 == 1 {
            1.0 / heavy
        } else {
            heavy
        });
    }
    let mut knots = vec![0.0; degree + 1];
    knots.extend(std::iter::repeat_n(
        0.3,
        if jump == 0.0 { degree } else { degree + 1 },
    ));
    knots.extend(std::iter::repeat_n(1.0, degree + 1));
    let side = NurbsCurve::new(degree, knots, ctrl, weights).unwrap();
    let corners = [at(0.0, 0.0), at(4.0, 0.0), at(4.0, 3.0), at(0.0, 3.0)];
    let mut body = Body::new(BodyKind::Sheet);
    let vs: Vec<_> = corners.iter().map(|p| body.add_vertex(*p)).collect();
    let line = |body: &mut Body, i: usize, j: usize, cut: f64| {
        body.add_edge(
            Curve3::Line(Line::through(corners[i], corners[j]).unwrap()),
            (0.0, corners[i].distance(corners[j]) - cut),
            vs[i],
            vs[j],
        )
    };
    let edges = [
        line(&mut body, 0, 1, 0.0),
        line(&mut body, 1, 2, short),
        body.add_edge(Curve3::Nurbs(side), (0.0, 1.0), vs[2], vs[3]),
        line(&mut body, 3, 0, 0.0),
    ];
    let lp = body.add_loop(
        edges
            .iter()
            .map(|edge| Coedge {
                edge: *edge,
                reversed: false,
            })
            .collect(),
    );
    let face = body.add_face(Surface::Plane(Plane::new(frame)), false, vec![lp]);
    body.add_shell(vec![face]);
    body
}

/// Allowances far from the origin (the review of the plane's theorem,
/// 2026-11-15: an edge that jumped by less than its hull's rounding
/// allowance, which grows with the coordinates, the degree and the weights,
/// was cleared; `main` refused such a face). Plates with a rational spline
/// side of degrees 2 to 9, weights up to a thousand apart, turned and a
/// thousand to a hundred million sizes off: with a side that jumps, by 3
/// tolerances to a thousand, along itself or sideways, never cleared (the
/// knots say it jumps, whatever the rounding); with a vertex open by 0.3 of
/// the reach, never; right, cleared near the origin, and not at all from
/// where the rounding of a point passes a share of the reach: far off, not
/// shown is the answer.
#[test]
fn allowances_far_from_the_origin_are_held_under_the_reach() {
    let k = 1.0;
    let tolerances = 1e-6 * k;
    let mut cleared = std::collections::BTreeMap::new();
    let turned = |origin: Vec3| {
        Frame::new(origin, Vec3::new(-0.3, 0.8, 0.52), Vec3::new(0.6, 0.1, 0.4)).unwrap()
    };
    for far in [0.0, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8] {
        // Far off with its plane's frame (the chart's coordinates stay
        // small), and far off within its plane (they do not).
        for (place, frame, off) in [
            (
                "frame",
                if far == 0.0 {
                    Frame::WORLD
                } else {
                    turned(Vec3::new(1.3, -2.1, 0.7) * (k * far))
                },
                (0.0, 0.0),
            ),
            ("chart", turned(Vec3::ZERO), (1.3 * far, -2.1 * far)),
        ] {
            for degree in [2_usize, 3, 5, 9] {
                for ratio in [1.0, 10.0, 50.0, 1000.0] {
                    let plate = |jump: f64, sideways: bool, short: f64| {
                        plate_with_a_spline_side(
                            (frame, off),
                            k,
                            degree,
                            ratio,
                            jump * tolerances,
                            sideways,
                            short * tolerances,
                        )
                    };
                    // A side that jumps is never cleared.
                    for jump in [3.0, 15.0, 60.0, 1000.0] {
                        for sideways in [false, true] {
                            assert_ne!(
                                verdict(&plate(jump, sideways, 0.0), k),
                                Some(Judged::Clear),
                                "{far:e} sizes off by its {place}, degree {degree}, weights {ratio} apart: a jump of {jump} tolerances, sideways {sideways}"
                            );
                        }
                    }
                    // A vertex open by 0.3 of the reach is never cleared.
                    assert_ne!(
                        verdict(&plate(0.0, false, 3.0), k),
                        Some(Judged::Clear),
                        "{far:e} sizes off by its {place}, degree {degree}, weights {ratio} apart: a vertex open by 3 tolerances"
                    );
                    // The right plate: cleared or not, counted.
                    let judged = verdict(&plate(0.0, false, 0.0), k);
                    assert!(
                        !matches!(judged, Some(Judged::Shown(..))),
                        "{far:e} sizes off by its {place}, degree {degree}, weights {ratio} apart: a right plate shown wrong: {judged:?}"
                    );
                    let slot = cleared.entry(format!("{place} {far:e}")).or_insert((0, 0));
                    slot.1 += 1;
                    slot.0 += usize::from(judged == Some(Judged::Clear));
                }
            }
        }
    }
    println!("right plates cleared, by how many sizes off: {cleared:?}");
    for place in ["frame", "chart"] {
        assert_eq!(
            cleared[&format!("{place} 0e0")],
            (16, 16),
            "at the origin every right plate"
        );
        assert_eq!(
            cleared[&format!("{place} 1e3")],
            (16, 16),
            "a thousand sizes off every one"
        );
        assert_eq!(
            cleared[&format!("{place} 1e8")].0,
            0,
            "a hundred million sizes off none"
        );
    }
}

/// The review's smallest case of an edge that jumps within the rounding
/// its hull is allowed: a rational quadratic of six points on a line, its
/// weights 50 apart, that stops at 0.3 and starts again 15 tolerances on,
/// the upper side of a plate turned and a million sizes off. Not cleared;
/// nor is any stretch of that side shown that holds the jump or ends on it.
#[test]
fn a_jump_within_the_rounding_of_its_hull_is_not_cleared() {
    let frame = Frame::new(
        Vec3::new(1.3, -2.1, 0.7) * 1e6,
        Vec3::new(-0.3, 0.8, 0.52),
        Vec3::new(0.6, 0.1, 0.4),
    )
    .unwrap();
    let body = plate_with_a_spline_side((frame, (0.0, 0.0)), 1.0, 2, 50.0, 15e-6, false, 0.0);
    let Curve3::Nurbs(side) = &body.edges[2].curve else {
        panic!("a spline side");
    };
    assert!(jumps(side, 0, usize::MAX));
    assert_eq!(
        verdict(&body, 1.0),
        None,
        "no verdict: the side is never shown"
    );
    let tol = tolerance(1.0);
    let mut walked = Walked::new(&body, &body.faces[0], &tol).unwrap();
    assert!(walked.show_all().is_none());
    // The same side as one curve, with a corner where it jumped: never
    // shown wrong; a million sizes off the rounding of its points (18
    // tolerances) is past a quarter of the reach, and it is not cleared
    // either; a thousand sizes off it is, and the one that jumps is not.
    let whole = plate_with_a_spline_side((frame, (0.0, 0.0)), 1.0, 2, 50.0, 0.0, false, 0.0);
    assert_eq!(verdict(&whole, 1.0), None);
    let nearer = Frame::new(
        Vec3::new(1.3, -2.1, 0.7) * 1e3,
        Vec3::new(-0.3, 0.8, 0.52),
        Vec3::new(0.6, 0.1, 0.4),
    )
    .unwrap();
    let whole = plate_with_a_spline_side((nearer, (0.0, 0.0)), 1.0, 2, 50.0, 0.0, false, 0.0);
    assert_eq!(verdict(&whole, 1.0), Some(Judged::Clear));
    let jumping = plate_with_a_spline_side((nearer, (0.0, 0.0)), 1.0, 2, 50.0, 15e-6, false, 0.0);
    assert_eq!(verdict(&jumping, 1.0), None);
}

/// A knot that stands more often than the degree is a jump only where
/// the curve starts again at another point than it ran up to: the plate's
/// spline side with its middle knot standing once more than the degree
/// and its middle point twice, and with that knot standing twice more and
/// a stray point between that the curve never touches, is one curve
/// (read either side of the knot it is at one place) and is cleared, at
/// every degree; the same with the second of those points a unit in the
/// last place off is not.
#[test]
fn a_knot_past_the_degree_between_one_point_and_itself_is_no_jump() {
    let nearer = Frame::new(
        Vec3::new(1.3, -2.1, 0.7) * 1e3,
        Vec3::new(-0.3, 0.8, 0.52),
        Vec3::new(0.6, 0.1, 0.4),
    )
    .unwrap();
    for frame in [Frame::WORLD, nearer] {
        for degree in [2_usize, 3, 5, 9] {
            for ratio in [1.0, 50.0] {
                let body = plate_with_a_spline_side(
                    (frame, (0.0, 0.0)),
                    1.0,
                    degree,
                    ratio,
                    0.0,
                    false,
                    0.0,
                );
                assert_eq!(verdict(&body, 1.0), Some(Judged::Clear));
                let Curve3::Nurbs(side) = body.edges[2].curve.clone() else {
                    panic!("a spline side");
                };
                let middle = side.control_points()[degree];
                for extra in [1_usize, 2] {
                    for off in [false, true] {
                        // The middle knot `extra` times more; the middle
                        // point again after `extra - 1` stray points.
                        let mut knots = side.knots().to_vec();
                        let mut ctrl = side.control_points().to_vec();
                        let mut weights = side.weights().to_vec();
                        for _ in 0..extra {
                            knots.insert(degree + 1, 0.3);
                        }
                        let again = if off {
                            Vec3::new(middle.x, middle.y, f64::from_bits(middle.z.to_bits() + 1))
                        } else {
                            middle
                        };
                        ctrl.insert(degree + 1, again);
                        weights.insert(degree + 1, 1.0);
                        for _ in 1..extra {
                            ctrl.insert(degree + 1, middle + Vec3::new(0.3, 0.2, 0.1));
                            weights.insert(degree + 1, 1.0);
                        }
                        let curve = NurbsCurve::new(degree, knots, ctrl, weights).unwrap();
                        let said = format!(
                            "degree {degree}, weights {ratio} apart, the knot {extra} more, off {off}"
                        );
                        assert_eq!(jumps(&curve, 0, usize::MAX), off, "{said}");
                        if !off {
                            let gap = curve.eval(0.3).distance(curve.eval(0.3 - 1e-13));
                            assert!(
                                gap < 1e-9,
                                "{said}: one place either side of the knot, {gap:e}"
                            );
                        }
                        let mut body = body.clone();
                        body.edges[2].curve = Curve3::Nurbs(curve);
                        let judged = verdict(&body, 1.0);
                        if off {
                            assert_eq!(judged, None, "{said}: never shown");
                        } else if extra == 1 {
                            assert_eq!(judged, Some(Judged::Clear), "{said}");
                        } else {
                            assert!(
                                !matches!(judged, Some(Judged::Shown(..))),
                                "{said}: {judged:?}"
                            );
                        }
                    }
                }
            }
        }
    }
}

/// C5 and C6 of the module on polygons, in the world and turned, at three
/// scales: a square is cleared, with a hole inside it too; a hole beside
/// the square, or inside another hole, is shown; so is a loop that runs
/// clockwise alone, two that run counter-clockwise, and a hole larger than
/// its outer loop.
#[test]
fn senses_and_holes_are_held_of_the_whole_face() {
    let shown = |judged: Option<Judged>| match judged {
        Some(Judged::Shown(kind, _)) => Some(kind),
        _ => None,
    };
    let turned_over =
        |points: &[(f64, f64)]| -> Vec<(f64, f64)> { points.iter().rev().copied().collect() };
    for k in [1e-3, 1.0, 1e6] {
        for frame in [Frame::WORLD, tilted(k)] {
            let of = |loops: &[&[(f64, f64)]], reversed: bool| {
                verdict(&polygons(frame, k, loops, reversed, 0.0), k)
            };
            assert_eq!(of(&[SQUARE], false), Some(Judged::Clear));
            assert_eq!(of(&[SQUARE, INSIDE], false), Some(Judged::Clear));
            // The hole first, the outer loop second.
            assert_eq!(of(&[INSIDE, SQUARE], false), Some(Judged::Clear));
            assert_eq!(shown(of(&[SQUARE, OUTSIDE], false)), Some(Kind::Hole));
            assert_eq!(
                shown(of(&[SQUARE, INSIDE, NESTED], false)),
                Some(Kind::Hole)
            );
            // The face turned over, its loops kept: every sense is wrong.
            assert_eq!(shown(of(&[SQUARE], true)), Some(Kind::Clockwise));
            assert_eq!(shown(of(&[SQUARE, INSIDE], true)), Some(Kind::LessThanHole));
            // Loops and face turned over: right again.
            let (square, inside) = (turned_over(SQUARE), turned_over(INSIDE));
            assert_eq!(
                of(&[square.as_slice(), inside.as_slice()], true),
                Some(Judged::Clear)
            );
            assert_eq!(
                shown(of(&[SQUARE, inside.as_slice()], false)),
                Some(Kind::ManyOuter)
            );
            assert_eq!(
                shown(of(&[square.as_slice()], false)),
                Some(Kind::Clockwise)
            );
            // A fault of the rule of simple loops is dropped only where the
            // whole face is cleared.
            for (loops, stands) in [
                (vec![SQUARE], false),
                (vec![SQUARE, INSIDE], false),
                (vec![SQUARE, OUTSIDE], true),
                (vec![square.as_slice()], true),
            ] {
                let body = polygons(frame, k, &loops, false, 0.0);
                let tol = tolerance(k);
                let mut verifier = Verifier::new(&body, &body.faces[0], &tol);
                assert_eq!(verifier.winding_shown(), stands, "{loops:?}");
            }
        }
    }
}

/// C3's gap: two sides whose ends miss each other at a vertex by less than
/// a quarter of the reach (ten tolerances) are shown apart there, and the
/// square is cleared; by a quarter or more nothing is shown, and the walk's
/// fault would stand.
#[test]
fn a_gap_at_a_vertex_is_held_to_a_quarter_of_the_reach() {
    for k in [1e-3, 1.0, 1e6] {
        let reach = 10.0 * 1e-6 * k;
        for (share, cleared) in [
            (0.0, true),
            (0.05, true),
            (0.2, true),
            (0.24, true),
            (0.26, false),
            (0.5, false),
            (3.0, false),
        ] {
            let body = polygons(Frame::WORLD, k, &[SQUARE], false, share * reach);
            let judged = verdict(&body, k);
            assert_eq!(
                judged == Some(Judged::Clear),
                cleared,
                "at {k}, a gap of {share} of the reach: {judged:?}"
            );
            assert!(
                !matches!(judged, Some(Judged::Shown(..))),
                "at {k}, a gap of {share} of the reach: {judged:?}"
            );
        }
    }
}

/// What can never be cleared: a loop of two stretches (there and back
/// along one line: C1 and C4), sides folded back on each other (a spur: C3),
/// a side over another (C2), a corner within the reach of another side (C2).
#[test]
fn degenerate_loops_are_never_cleared() {
    for k in [1e-3, 1.0, 1e6] {
        let r = 1e-5;
        let cases: [&[(f64, f64)]; 5] = [
            // There and back.
            &[(0.0, 0.0), (4.0, 0.0)],
            // A spur: out along a line and back over it.
            &[
                (0.0, 0.0),
                (4.0, 0.0),
                (4.0, 3.0),
                (4.0, 5.0),
                (4.0, 3.0),
                (0.0, 3.0),
            ],
            // A bow tie.
            &[(0.0, 0.0), (4.0, 3.0), (4.0, 0.0), (0.0, 3.0)],
            // A corner half a reach from the opposite side.
            &[
                (0.0, 0.0),
                (4.0, 0.0),
                (4.0, 3.0),
                (2.0, 0.5 * r),
                (0.0, 3.0),
            ],
            // A corner on the opposite side.
            &[(0.0, 0.0), (4.0, 0.0), (4.0, 3.0), (2.0, 0.0), (0.0, 3.0)],
        ];
        for (i, points) in cases.iter().enumerate() {
            let judged = verdict(&polygons(Frame::WORLD, k, &[points], false, 0.0), k);
            assert_ne!(judged, Some(Judged::Clear), "at {k}, case {i}");
        }
        // The same corner two reaches from the side: cleared.
        let clear: &[(f64, f64)] = &[
            (0.0, 0.0),
            (4.0, 0.0),
            (4.0, 3.0),
            (2.0, 2.0 * r),
            (0.0, 3.0),
        ];
        assert_eq!(
            verdict(&polygons(Frame::WORLD, k, &[clear], false, 0.0), k),
            Some(Judged::Clear),
            "at {k}"
        );
    }
}

/// Running out of work leaves the fault standing: the verdict of a face
/// whose work is spent before it is judged is none, which
/// [`Verifier::region_shown`] and [`Verifier::winding_shown`] read as the
/// walk's fault.
#[test]
fn work_spent_leaves_the_fault_standing() {
    let body = polygons(Frame::WORLD, 1.0, &[SQUARE, INSIDE], false, 0.0);
    let tol = tolerance(1.0);
    let face = &body.faces[0];
    let mut walked = Walked::new(&body, face, &tol).unwrap();
    walked.show_all().unwrap();
    assert_eq!(judge(&mut walked, false), Some(Judged::Clear));
    // The same face with its work all but spent: every step that counts
    // says none.
    walked.work = crate::region::MOST_PROOF_WORK;
    assert_eq!(judge(&mut walked, false), None);
    walked.work = crate::region::MOST_PROOF_WORK + 1;
    assert_eq!(walked.place_at(&body.edges[0].curve, 0.0), None);
    assert_eq!(judge(&mut walked, false), None);
    let mut verifier = Verifier::new(&body, face, &tol);
    verifier.verdict = Some(None);
    assert_eq!(
        verifier
            .region_shown("its loop 0 crosses itself")
            .as_deref(),
        Some("its loop 0 crosses itself")
    );
    assert!(verifier.winding_shown());
}
