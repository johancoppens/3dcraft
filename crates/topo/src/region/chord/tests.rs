//! The steps of the plane theorem ([`super`]), each held to its statement.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use super::*;
use crate::{BodyKind, Coedge};
use craft3d_geom::{Circle, Cone, Cylinder, Line, NurbsCurve, Plane, Sphere, Torus};
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

/// The two circles of a band about the world's axis at the radius 2, the
/// heights 0 and 3, as the two loops of a face on `surface`.
fn band(surface: Surface) -> Body {
    let mut body = Body::new(BodyKind::Sheet);
    let mut loops = Vec::new();
    for (z, reversed) in [(0.0, false), (3.0, true)] {
        let circle = Circle::new(
            Frame::new(Vec3::new(0.0, 0.0, z), Vec3::Z, Vec3::X).unwrap(),
            2.0,
        )
        .unwrap();
        let v = body.add_vertex(Vec3::new(2.0, 0.0, z));
        let edge = body.add_edge(Curve3::Circle(circle), (0.0, TAU), v, v);
        loops.push(body.add_loop(vec![Coedge { edge, reversed }]));
    }
    let face = body.add_face(surface, false, loops);
    body.add_shell(vec![face]);
    body
}

/// A face on a torus or a spline surface is not walked again at all: the
/// walk's fault stands, whatever it says, and nothing is counted.
#[test]
fn a_face_on_a_torus_is_not_walked_again() {
    let body = band(Surface::Torus(Torus::new(Frame::WORLD, 5.0, 3.0).unwrap()));
    let tol = tolerance(1.0);
    let face = &body.faces[0];
    assert!(!asked(&face.surface));
    assert!(chart_of(&face.surface).is_none());
    assert!(Walked::new(&body, face, &tol).is_none());
    let before = crate::region::chord_proofs();
    let original = "its loops 0 and 1 cross";
    let mut verifier = Verifier::new(&body, face, &tol);
    assert_eq!(verifier.region_shown(original).as_deref(), Some(original));
    assert!(verifier.winding_shown());
    assert_eq!(crate::region::chord_proofs(), before, "nothing counted");
}

/// A band of a cylinder (two circles round it) is not judged by the
/// bounds: its loops' lifts do not close, and a fault of its region rule
/// stands, whatever it says.
#[test]
fn a_band_is_never_cleared() {
    let body = band(Surface::Cylinder(Cylinder::new(Frame::WORLD, 2.0).unwrap()));
    let tol = tolerance(1.0);
    let face = &body.faces[0];
    let mut walked = Walked::new(&body, face, &tol).unwrap();
    walked.show_all().unwrap();
    assert!(matches!(walked.rings(), Some((_, false))), "read as a band");
    assert_eq!(judge(&mut walked, false), Some(Judged::Unsure(Vec::new())));
    let original = "its loops 0 and 1 cross";
    let mut verifier = Verifier::new(&body, face, &tol);
    assert_eq!(verifier.region_shown(original).as_deref(), Some(original));
    assert!(verifier.winding_shown());
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

/// The loop of the chart rectangle `[u0, u1] × [v0, v1]` of `surface` (a
/// cylinder, a cone or a sphere in `frame`), added to `body`: two parallels
/// (circles about the axis) and two rulings (lines) or, on a sphere,
/// meridians (circles through the poles). Counter-clockwise in the chart;
/// clockwise where `hole`. `short` shortens the right side at its upper
/// end by that length.
fn chart_rectangle(
    body: &mut Body,
    surface: &Surface,
    frame: &Frame,
    (u0, u1, v0, v1): (f64, f64, f64, f64),
    hole: bool,
    short: f64,
) -> crate::LoopId {
    let parallel = |v: f64| {
        let (x, y, h) = frame.local(surface.eval(0.0, v));
        let at = frame.at(frame.point(0.0, 0.0, h)).unwrap();
        Curve3::Circle(Circle::new(at, x.phypot(y)).unwrap())
    };
    let side = |u: f64| -> (Curve3, (f64, f64)) {
        let (a, b) = (surface.eval(u, v0), surface.eval(u, v1));
        match surface {
            Surface::Sphere(s) => {
                let (sn, cs) = u.psin_cos();
                let out = frame.vector(cs, sn, 0.0);
                let meridian = Frame::new(frame.origin(), out.cross(frame.z()), out).unwrap();
                (
                    Curve3::Circle(Circle::new(meridian, s.radius()).unwrap()),
                    (v0, v1 - short / s.radius()),
                )
            }
            _ => (
                Curve3::Line(Line::through(a, b).unwrap()),
                (0.0, a.distance(b) - short),
            ),
        }
    };
    let corners = [
        surface.eval(u0, v0),
        surface.eval(u1, v0),
        surface.eval(u1, v1),
        surface.eval(u0, v1),
    ];
    let vs: Vec<_> = corners.iter().map(|p| body.add_vertex(*p)).collect();
    let (right, right_range) = side(u1);
    let (left, left_range) = side(u0);
    let left_range = if short > 0.0 {
        // Only the right side is shortened.
        match surface {
            Surface::Sphere(_) => (v0, v1),
            _ => (left_range.0, left_range.1 + short),
        }
    } else {
        left_range
    };
    let edges = [
        body.add_edge(parallel(v0), (u0, u1), vs[0], vs[1]),
        body.add_edge(right, right_range, vs[1], vs[2]),
        body.add_edge(parallel(v1), (u0, u1), vs[3], vs[2]),
        body.add_edge(left, left_range, vs[0], vs[3]),
    ];
    let mut coedges: Vec<Coedge> = edges
        .iter()
        .zip([false, false, true, true])
        .map(|(edge, reversed)| Coedge {
            edge: *edge,
            reversed,
        })
        .collect();
    if hole {
        coedges.reverse();
        for co in &mut coedges {
            co.reversed = !co.reversed;
        }
    }
    body.add_loop(coedges)
}

/// A chart rectangle: its angles `(u0, u1)` and the shares of the height
/// it lies between.
type Rectangle = ((f64, f64), (f64, f64));

/// A cylinder, a cone and a sphere of size `k` in `frame`, each with the
/// heights (latitudes, on the sphere) its test rectangles lie between.
fn about_grounds(frame: Frame, k: f64) -> Vec<(Surface, (f64, f64))> {
    vec![
        (
            Surface::Cylinder(Cylinder::new(frame, 1.3 * k).unwrap()),
            (0.2 * k, 1.1 * k),
        ),
        (
            Surface::Cone(Cone::new(frame, 2.0 * k, 0.4).unwrap()),
            (0.1 * k, 1.0 * k),
        ),
        (
            Surface::Sphere(Sphere::new(frame, 1.7 * k).unwrap()),
            (0.1, 0.8),
        ),
    ]
}

/// A one-face sheet on `surface` whose loops are chart rectangles: the
/// first counter-clockwise, the others clockwise (holes), each `(u0, u1)`
/// with the shares of the height it lies between.
fn rectangles(
    surface: &Surface,
    frame: &Frame,
    (low, high): (f64, f64),
    loops: &[Rectangle],
    short: f64,
) -> Body {
    let mut body = Body::new(BodyKind::Sheet);
    let ids = loops
        .iter()
        .enumerate()
        .map(|(i, ((u0, u1), (a, b)))| {
            let at = |share: f64| low + (high - low) * share;
            chart_rectangle(
                &mut body,
                surface,
                frame,
                (*u0, *u1, at(*a), at(*b)),
                i > 0,
                if i == 0 { short } else { 0.0 },
            )
        })
        .collect();
    let face = body.add_face(surface.clone(), false, ids);
    body.add_shell(vec![face]);
    body
}

/// Pieces a period apart in the chart lie on one place of the surface, and
/// are held to each other (C2′ and C6′ of the module; the second review's
/// fault): a chart rectangle of a cylinder, a cone and a sphere less than
/// once round is cleared, almost all the way round too; once round and a
/// bit, exactly once round, within half a reach of once round, twice round
/// and a half, and further than the copies reach, it is not. A hole is
/// inside its outer loop whichever of its lifts the chart gives.
#[test]
fn pieces_a_period_apart_are_held_to_each_other() {
    let whole = (0.0, 1.0);
    for k in [1e-3, 1.0, 1e6] {
        for frame in [Frame::WORLD, tilted(k)] {
            for (surface, heights) in about_grounds(frame, k) {
                let kind = surface.kind();
                let of = |loops: &[Rectangle]| {
                    verdict(&rectangles(&surface, &frame, heights, loops, 0.0), k)
                };
                let cleared = |judged: &Option<Judged>| *judged == Some(Judged::Clear);
                // The reach as an angle: ten tolerances over the least
                // distance from the axis the rectangles have.
                let least = [heights.0, heights.1]
                    .iter()
                    .map(|v| {
                        let (x, y, _) = frame.local(surface.eval(0.0, *v));
                        x.phypot(y)
                    })
                    .fold(f64::INFINITY, f64::min);
                let reach = 1e-5 * k / least;
                for (extent, clears) in [
                    (1.0, true),
                    (4.0, true),
                    (0.99 * TAU, true),
                    (TAU - 30.0 * reach, true),
                    (TAU - 0.5 * reach, false),
                    (TAU, false),
                    (TAU + 0.5 * reach, false),
                    (TAU + 0.03, false),
                    (1.3 * TAU, false),
                    (2.5 * TAU, false),
                    (3.5 * TAU, false),
                    (40.0 * TAU, false),
                ] {
                    for start in [0.3, -2.9, 5.0] {
                        let judged = of(&[((start, start + extent), whole)]);
                        assert_eq!(
                            cleared(&judged),
                            clears,
                            "{kind} at {k}: a rectangle from {start} {extent} round: {judged:?}"
                        );
                    }
                }
                // A hole in the middle, as the chart gives it and a period
                // or two away; beside the rectangle; inside another hole.
                let outer = ((0.3, 2.3), whole);
                for shift in [0.0, TAU, -2.0 * TAU] {
                    let hole = ((1.0 + shift, 1.5 + shift), (0.3, 0.6));
                    assert!(
                        cleared(&of(&[outer, hole])),
                        "{kind} at {k}: a hole {shift} on"
                    );
                    let beside = ((2.6 + shift, 3.0 + shift), (0.3, 0.6));
                    assert!(
                        matches!(of(&[outer, beside]), Some(Judged::Shown(Kind::Hole, _))),
                        "{kind} at {k}: a hole beside, {shift} on"
                    );
                    let nested = ((1.1 + shift, 1.4 + shift), (0.4, 0.5));
                    assert!(
                        matches!(
                            of(&[outer, ((1.0, 1.5), (0.3, 0.6)), nested]),
                            Some(Judged::Shown(Kind::Hole, _))
                        ),
                        "{kind} at {k}: a hole in a hole, {shift} on"
                    );
                    // A hole across the outer loop's side, seen a period on.
                    let astride = ((2.1 + shift, 2.6 + shift), (0.3, 0.6));
                    assert!(
                        !cleared(&of(&[outer, astride])),
                        "{kind} at {k}: a hole astride, {shift} on"
                    );
                }
                // A hole of an outer loop that goes almost all the way
                // round, lying across the chart's seam.
                let wide = ((0.5, 0.5 + 0.9 * TAU), whole);
                assert!(
                    !cleared(&of(&[wide, ((-0.3, 0.2), (0.3, 0.6))])),
                    "{kind} at {k}: a hole in the gap of a wide loop"
                );
                assert!(
                    cleared(&of(&[wide, ((3.0, 3.6), (0.3, 0.6))])),
                    "{kind} at {k}: a hole of a wide loop"
                );
            }
        }
    }
}

/// On a sphere every loop may run clockwise, the face being the sphere less
/// the discs they bound (C5″, C6″ of the module): cleared where the discs
/// lie apart, shown where one holds another, a period on or not.
#[test]
fn a_sphere_less_discs_is_held_to_discs_apart() {
    for k in [1e-3, 1.0, 1e6] {
        for frame in [Frame::WORLD, tilted(k)] {
            let surface = Surface::Sphere(Sphere::new(frame, 1.7 * k).unwrap());
            let heights = (0.1, 0.8);
            let discs = |loops: &[Rectangle]| {
                let mut body = Body::new(BodyKind::Sheet);
                let ids = loops
                    .iter()
                    .map(|((u0, u1), (a, b))| {
                        let at = |share: f64| heights.0 + (heights.1 - heights.0) * share;
                        chart_rectangle(
                            &mut body,
                            &surface,
                            &frame,
                            (*u0, *u1, at(*a), at(*b)),
                            true,
                            0.0,
                        )
                    })
                    .collect();
                let face = body.add_face(surface.clone(), false, ids);
                body.add_shell(vec![face]);
                verdict(&body, k)
            };
            let whole = (0.0, 1.0);
            assert_eq!(
                discs(&[((0.3, 2.3), whole)]),
                Some(Judged::Clear),
                "one disc"
            );
            assert_eq!(
                discs(&[((0.3, 1.3), whole), ((2.0, 3.0), whole)]),
                Some(Judged::Clear),
                "two apart"
            );
            for shift in [0.0, TAU, -TAU] {
                assert!(
                    matches!(
                        discs(&[
                            ((0.3, 2.3), whole),
                            ((1.0 + shift, 1.5 + shift), (0.3, 0.6))
                        ]),
                        Some(Judged::Shown(Kind::Hole, _))
                    ),
                    "one in another, {shift} on"
                );
                assert_ne!(
                    discs(&[((0.3, 2.3), whole), ((2.0 + shift, 3.0 + shift), whole)]),
                    Some(Judged::Clear),
                    "two across each other, {shift} on"
                );
            }
            // A cylinder's loops may not all run clockwise.
            let cylinder = Surface::Cylinder(Cylinder::new(frame, 1.3 * k).unwrap());
            let mut body = Body::new(BodyKind::Sheet);
            let lp = chart_rectangle(
                &mut body,
                &cylinder,
                &frame,
                (0.3, 2.3, 0.2 * k, 1.1 * k),
                true,
                0.0,
            );
            let face = body.add_face(cylinder, false, vec![lp]);
            body.add_shell(vec![face]);
            assert!(matches!(
                verdict(&body, k),
                Some(Judged::Shown(Kind::Clockwise, _))
            ));
        }
    }
}

/// What is allowed at a joint is ten tolerances on the surface whichever
/// way it runs (the second review's reading: ten tolerances in the chart of
/// a cylinder of radius a thousandth were ten thousand along its axis). A
/// rectangle of a thin cylinder, a cone and a sphere whose right side stops
/// short of its upper corner: cleared under a quarter of ten tolerances,
/// not from a quarter on.
#[test]
fn a_gap_is_held_to_tolerances_on_the_surface() {
    for k in [1e-3, 1.0, 1e6] {
        let frame = Frame::WORLD;
        let mut all = about_grounds(frame, k);
        // A cylinder a thousand times thinner than it is long.
        all.push((
            Surface::Cylinder(Cylinder::new(frame, 1e-3 * k).unwrap()),
            (0.2 * k, 1.1 * k),
        ));
        for (surface, heights) in all {
            for (share, clears) in [
                (0.0, true),
                (0.05, true),
                (0.15, true),
                (0.3, false),
                (1.0, false),
                (30.0, false),
                (2500.0, false),
            ] {
                let body = rectangles(
                    &surface,
                    &frame,
                    heights,
                    &[((0.3, 2.3), (0.0, 1.0))],
                    share * 1e-5 * k,
                );
                let judged = verdict(&body, k);
                assert_eq!(
                    judged == Some(Judged::Clear),
                    clears,
                    "{} at {k}, a gap of {share} of ten tolerances: {judged:?}",
                    surface.kind()
                );
            }
        }
    }
}

/// A loop near a pole of a sphere or the apex of a cone is never cleared: a
/// point within ten tolerances of one is not placed, and a stretch that
/// goes round the axis there is not shown.
#[test]
fn a_loop_beside_a_pole_or_the_apex_is_never_cleared() {
    let frame = Frame::WORLD;
    let k = 1.0;
    let sphere = Surface::Sphere(Sphere::new(frame, 1.7).unwrap());
    let top = std::f64::consts::FRAC_PI_2;
    for (to, clears) in [(top - 0.3, true), (top - 1e-7, false), (top, false)] {
        let judged = verdict(
            &rectangles(&sphere, &frame, (0.1, to), &[((0.3, 2.3), (0.0, 1.0))], 0.0),
            k,
        );
        assert_eq!(
            judged == Some(Judged::Clear),
            clears,
            "sphere to {to}: {judged:?}"
        );
    }
    let Surface::Cone(cone) = Surface::Cone(Cone::new(frame, 2.0, 0.4).unwrap()) else {
        panic!("a cone");
    };
    let apex = cone.apex_v();
    let surface = Surface::Cone(cone);
    // The apex lies below the cone's frame: rectangles from beside it up.
    assert!(apex < 0.0);
    for (from, clears) in [
        (apex * 0.5, true),
        (apex * (1.0 - 1e-8), false),
        (apex, false),
    ] {
        let judged = verdict(
            &rectangles(
                &surface,
                &frame,
                (from, 0.1),
                &[((0.3, 2.3), (0.0, 1.0))],
                0.0,
            ),
            k,
        );
        assert_eq!(
            judged == Some(Judged::Clear),
            clears,
            "cone from {from}: {judged:?}"
        );
    }
}

/// The parallelogram's corners against a curve's points: a quarter circle
/// from (1, 0) to (0, 1) keeps its direction between (0, 1) and (-1, 0),
/// and the bound is the corner (1, 1)'s distance from the chord, `1 / √2`,
/// which the circle's own, `1 − 1 / √2`, is within.
#[test]
fn a_parallelogram_holds_a_quarter_circle() {
    let off = parallelogram((0.0, 1.0), (-1.0, 0.0), (-1.0, 1.0)).unwrap();
    assert!((off - 0.5_f64.sqrt()).abs() < 1e-15, "{off}");
    // A cone nearly a half turn wide: the corners lie far past the
    // chord's ends, and are measured from the segment, not its line.
    let off = parallelogram((1.0, 0.01), (-1.0, 0.01), (0.0, 0.02)).unwrap();
    assert!((off - 1.0).abs() < 1e-12, "{off}");
    // A chord outside the cone: no bound.
    assert_eq!(parallelogram((0.0, 1.0), (-1.0, 0.0), (1.0, 1.0)), None);
    // A cone of no width, a cone over a half turn: none.
    assert_eq!(parallelogram((1.0, 0.0), (1.0, 0.0), (1.0, 0.0)), None);
    assert_eq!(parallelogram((1.0, 0.0), (0.0, -1.0), (1.0, -1.0)), None);
    assert_eq!(parallelogram((1.0, 0.0), (0.0, 1.0), (0.0, 0.0)), Some(0.0));
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
/// module), against 64 points of its image placed on the surface and taken
/// to the scaled chart: within its bound of its chord, every move from a
/// place to a later one in its cone, running one way; and the area the
/// sampled loop bounds is its polyline's within the error claimed. Closed
/// wavy splines of degrees 1 to 7, through many points or as few as one
/// span (bulging far from their chords), rational and not, on a plane, a
/// cylinder, a cone and a sphere, in the world and turned and moved, at
/// three scales, as walked and halved three times over (so that both the
/// spans' control points and the cut Bézier pieces are read).
#[test]
fn a_stretch_shown_lies_within_its_bound_and_runs_one_way() {
    let (mut shown, mut walked_faces, mut stretches) = (0, 0, 0_usize);
    let mut coned = 0_usize;
    for k in [1e-3, 1.0, 1e6] {
        for frame in [Frame::WORLD, tilted(k)] {
            for (surface, at) in grounds(frame, k) {
                let kind = surface.kind();
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
                    // A rational fit through few points can have a weight
                    // that is not positive: no such curve.
                    let Some(curve) = ring_curve(at.as_ref(), waves, a, count, degree, rational)
                    else {
                        assert!(rational, "{kind} degree {degree}");
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
                    // A loose tolerance, so that a curve through few points
                    // of a curved surface is placed between them too (the
                    // bounds are of the curve's image, on the surface or
                    // not).
                    let tol = Tolerance::new(1e-3 * k, 1e-9).unwrap();
                    let face = &body.faces[0];
                    let Some(mut walked) = Walked::new(&body, face, &tol) else {
                        assert!(
                            !matches!(surface, Surface::Plane(_)),
                            "not walked on a plane"
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
                            "not shown: {kind} at {k}, degree {degree} through {count}, rational {rational}, work {}",
                            walked.work
                        );
                        continue;
                    }
                    shown += 1;
                    let unit = walked.unit.unwrap();
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
                        let (rings, closed) = walked.rings().unwrap();
                        assert!(closed, "{kind}: the loop goes round no axis");
                        let ring = &rings[0];
                        let mut sampled: Vec<[f64; 2]> = Vec::new();
                        for (place, st) in all.iter().enumerate() {
                            let Bound::Shown(bound, aim, _) = st.bound else {
                                panic!("{kind}: a stretch not shown after show_all");
                            };
                            stretches += 1;
                            // The stretch's lift, from its start as the
                            // loop's polyline has it.
                            let start = ring.ring.points[place];
                            let end = walked.unwrapped(st.end, Some(start));
                            let mut before = start;
                            let mut far = 0.0_f64;
                            let mut turned: Vec<f64> = Vec::new();
                            let mut places: Vec<[f64; 2]> = Vec::new();
                            for i in 0..=64 {
                                let t = st.s + (st.e - st.s) * i as f64 / 64.0;
                                let (u, v) = surface.project(curve.eval(t)).unwrap();
                                let q = walked.unwrapped([u * unit.0, v * unit.1], Some(before));
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
                                "{kind} at {k}, degree {degree} through {count}: {far} off its chord, bound {bound}"
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
                                    "{kind}: a cone of a half turn or more"
                                );
                                for i in 0..places.len() {
                                    for j in [i + 1, i + 7, i + 23, 64] {
                                        let (Some(p), Some(q)) = (places.get(i), places.get(j))
                                        else {
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
                                            "{kind} at {k}, degree {degree} through {count}: the move {d:?} outside the cone {c0:?} to {c1:?}"
                                        );
                                    }
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
                        // (L3): the sampled loop's area against the
                        // polyline's.
                        let error = area_error(&ring.ring.points, &ring.margins);
                        let (dense, coarse) =
                            (signed_area(&sampled), signed_area(&ring.ring.points));
                        assert!(
                            (dense - coarse).abs() <= error * (1.0 + 1e-3) + 1e-12 * k * k,
                            "{kind} at {k}, degree {degree} through {count}, level {level}: the loop bounds {dense}, its polyline {coarse}, error {error}"
                        );
                    }
                }
            }
        }
    }
    // Every face walked has every stretch shown, the few-span ones
    // included.
    assert_eq!(shown, walked_faces, "of {walked_faces}");
    eprintln!("{walked_faces} faces walked, {stretches} stretches, {coned} with a cone");
    assert!(walked_faces >= 200, "{walked_faces} faces walked");
    assert!(stretches > 30_000, "{stretches} stretches");
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
    // Over the knots before the jump and after it, none.
    assert!(!jumps(&jump, 0, 2) && !jumps(&jump, 3, 5));
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
