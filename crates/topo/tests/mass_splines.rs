//! Mass properties of bodies whose edges are splines.
//!
//! A spline is smooth only between its knots, so a quadrature panel that
//! straddles one integrates a function with a kink in it and is off by
//! far more than rounding. The curves here are exact (conics as rational
//! quadratics in several pieces, polynomial splines whose integral is
//! known piece by piece), so every body has a closed form, stated at its
//! builder, and the knots are placed where a uniform division of the edge
//! does not fall on them.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::{Circle, Cone, Curve3, Cylinder, Line, NurbsCurve, Plane, Sphere, Surface};
use craft3d_math::Portable;
use craft3d_math::quadrature::Rule;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{
    Body, BodyKind, Coedge, EdgeId, FaceId, MassError, VertexId, check, mass_properties,
};
use std::f64::consts::{PI, SQRT_2, TAU};

fn fwd(edge: EdgeId) -> Coedge {
    Coedge {
        edge,
        reversed: false,
    }
}

fn back(edge: EdgeId) -> Coedge {
    Coedge {
        edge,
        reversed: true,
    }
}

fn flat(body: &mut Body, origin: Vec3, normal: Vec3, coedges: Vec<Coedge>) -> FaceId {
    let loop_ = body.add_loop(coedges);
    let frame = Frame::from_z(origin, normal).unwrap();
    body.add_face(Surface::Plane(Plane::new(frame)), false, vec![loop_])
}

fn curved(body: &mut Body, surface: Surface, loops: Vec<Vec<Coedge>>) -> FaceId {
    let loops = loops.into_iter().map(|l| body.add_loop(l)).collect();
    body.add_face(surface, false, loops)
}

fn line(body: &mut Body, ends: (VertexId, VertexId)) -> EdgeId {
    let (from, to) = (
        body.vertex(ends.0).unwrap().point,
        body.vertex(ends.1).unwrap().point,
    );
    body.add_edge(
        Curve3::Line(Line::through(from, to).unwrap()),
        (0.0, from.distance(to)),
        ends.0,
        ends.1,
    )
}

/// The whole ellipse with semi-axes `a` along the frame's x and `b` along
/// its y as a rational quadratic spline in `pieces` equal arcs, counter-
/// clockwise from the x axis over the parameters 0 to 1.
///
/// An arc of a circle of half-angle h is the rational quadratic Bézier
/// curve through its ends with the meeting point of the end tangents, at
/// 1 / cos h from the centre, as middle control point with weight cos h
/// (*The NURBS Book* §7.3); an ellipse is a circle stretched, which a
/// spline follows by stretching its control points. The pieces join at
/// double knots at k / pieces, and the middle of piece k is at
/// (k + ½) / pieces by symmetry.
fn spline_ellipse(frame: &Frame, a: f64, b: f64, pieces: usize) -> Curve3 {
    let half = PI / pieces as f64;
    let mut ctrl = Vec::new();
    let mut weights = Vec::new();
    let mut knots = vec![0.0, 0.0, 0.0];
    for k in 0..pieces {
        let at = 2.0 * half * k as f64;
        ctrl.push(frame.point(a * at.pcos(), b * at.psin(), 0.0));
        weights.push(1.0);
        let between = at + half;
        ctrl.push(frame.point(
            a * between.pcos() / half.pcos(),
            b * between.psin() / half.pcos(),
            0.0,
        ));
        weights.push(half.pcos());
        if k > 0 {
            let knot = k as f64 / pieces as f64;
            knots.extend([knot, knot]);
        }
    }
    ctrl.push(frame.point(a, 0.0, 0.0));
    weights.push(1.0);
    knots.extend([1.0, 1.0, 1.0]);
    Curve3::Nurbs(NurbsCurve::new(2, knots, ctrl, weights).unwrap())
}

/// A whole circle about the frame's axis at height `z`, as a spline in
/// `pieces` arcs, as an edge with one vertex.
fn spline_ring(body: &mut Body, frame: &Frame, z: f64, radius: f64, pieces: usize) -> EdgeId {
    let placed = frame.at(frame.point(0.0, 0.0, z)).unwrap();
    let vertex = body.add_vertex(placed.point(radius, 0.0, 0.0));
    body.add_edge(
        spline_ellipse(&placed, radius, radius, pieces),
        (0.0, 1.0),
        vertex,
        vertex,
    )
}

struct Solid {
    name: String,
    body: Body,
    volume: f64,
    area: f64,
}

/// A cylinder whose two rims are splines. V = πr²h, A = 2πr (r + h).
fn cylinder(frame: &Frame, radius: f64, height: f64, pieces: usize) -> Solid {
    let mut body = Body::new(BodyKind::Solid);
    let bottom = spline_ring(&mut body, frame, 0.0, radius, pieces);
    let top = spline_ring(&mut body, frame, height, radius, pieces + 2);
    let wall = curved(
        &mut body,
        Surface::Cylinder(Cylinder::new(*frame, radius).unwrap()),
        vec![vec![fwd(bottom)], vec![back(top)]],
    );
    let floor = flat(&mut body, frame.origin(), -frame.z(), vec![back(bottom)]);
    let lid = flat(
        &mut body,
        frame.point(0.0, 0.0, height),
        frame.z(),
        vec![fwd(top)],
    );
    body.add_shell(vec![wall, floor, lid]);
    Solid {
        name: format!("cylinder, rims in {pieces} and {} pieces", pieces + 2),
        body,
        volume: PI * radius * radius * height,
        area: TAU * radius * (radius + height),
    }
}

/// A cone frustum whose rims are splines: V = πh (r₁² + r₁r₂ + r₂²) / 3,
/// the wall π (r₁ + r₂) s with s the slant height.
fn frustum(frame: &Frame, r1: f64, r2: f64, height: f64, pieces: usize) -> Solid {
    let mut body = Body::new(BodyKind::Solid);
    let bottom = spline_ring(&mut body, frame, 0.0, r1, pieces);
    let top = spline_ring(&mut body, frame, height, r2, pieces + 1);
    let cone = Cone::new(*frame, r1, ((r2 - r1) / height).patan()).unwrap();
    let wall = curved(
        &mut body,
        Surface::Cone(cone),
        vec![vec![fwd(bottom)], vec![back(top)]],
    );
    let floor = flat(&mut body, frame.origin(), -frame.z(), vec![back(bottom)]);
    let lid = flat(
        &mut body,
        frame.point(0.0, 0.0, height),
        frame.z(),
        vec![fwd(top)],
    );
    body.add_shell(vec![wall, floor, lid]);
    Solid {
        name: format!("frustum, rims in {pieces} and {} pieces", pieces + 1),
        body,
        volume: PI * height * (r1 * r1 + r1 * r2 + r2 * r2) / 3.0,
        area: PI * (r1 + r2) * height.phypot(r2 - r1) + PI * (r1 * r1 + r2 * r2),
    }
}

/// A cap of a ball whose rim is a spline. For a cap of height h:
/// V = πh²(3r − h)/3, curved area 2πrh.
fn cap(frame: &Frame, radius: f64, level: f64, pieces: usize) -> Solid {
    let mut body = Body::new(BodyKind::Solid);
    let rim_radius = (radius * radius - level * level).sqrt();
    let rim = spline_ring(&mut body, frame, level, rim_radius, pieces);
    // The sphere's own frame is turned away from the cap's axis.
    let turned = Frame::new(
        frame.origin(),
        frame.vector(0.3, -0.5, 0.8),
        frame.vector(1.0, 0.1, 0.0),
    )
    .unwrap();
    let dome = curved(
        &mut body,
        Surface::Sphere(Sphere::new(turned, radius).unwrap()),
        vec![vec![fwd(rim)]],
    );
    let disc = flat(
        &mut body,
        frame.point(0.0, 0.0, level),
        -frame.z(),
        vec![back(rim)],
    );
    body.add_shell(vec![dome, disc]);
    let h = radius - level;
    Solid {
        name: format!("cap at {level}, rim in {pieces} pieces"),
        body,
        volume: PI * h * h * (3.0 * radius - h) / 3.0,
        area: TAU * radius * h + PI * rim_radius * rim_radius,
    }
}

/// What two cylinders of the same radius share when their axes cross at
/// right angles (the Steinmetz solid): V = 16r³/3, A = 16r².
///
/// The cylinders, about the frame's x and y axes, meet in two ellipses in
/// the planes x = ±y, with semi-axes r along z and √2 r across. Each is a
/// spline in three pieces starting at the bottom, so that the top is in
/// the middle of the second piece: the two edges it is cut into each have
/// a double knot a third of the way round, well inside them.
fn steinmetz(frame: &Frame, r: f64) -> Solid {
    let mut body = Body::new(BodyKind::Solid);
    let centre = frame.origin();
    let e1 = frame.vector(1.0, 1.0, 0.0) * (1.0 / SQRT_2);
    let e2 = frame.vector(1.0, -1.0, 0.0) * (1.0 / SQRT_2);
    let low = body.add_vertex(centre - frame.z() * r);
    let high = body.add_vertex(centre + frame.z() * r);
    // x along −z and y along e1 (or e2): up the + side, down the − side.
    let first = spline_ellipse(
        &Frame::new(centre, e2, -frame.z()).unwrap(),
        r,
        SQRT_2 * r,
        3,
    );
    let second = spline_ellipse(
        &Frame::new(centre, -e1, -frame.z()).unwrap(),
        r,
        SQRT_2 * r,
        3,
    );
    let e1a = body.add_edge(first.clone(), (0.0, 0.5), low, high);
    let e1b = body.add_edge(first, (0.5, 1.0), high, low);
    let e2a = body.add_edge(second.clone(), (0.0, 0.5), low, high);
    let e2b = body.add_edge(second, (0.5, 1.0), high, low);
    let about = |axis: Vec3, hint: Vec3| {
        Surface::Cylinder(Cylinder::new(Frame::new(centre, axis, hint).unwrap(), r).unwrap())
    };
    // The cylinder about y is the solid's skin where |y| ≤ |x|, the one
    // about x where |x| ≤ |y|. Each face is walked up one edge and down
    // the other with the face to the left, seen from outside.
    let faces = vec![
        curved(
            &mut body,
            about(frame.y(), frame.z()),
            vec![vec![fwd(e1a), back(e2a)]],
        ),
        curved(
            &mut body,
            about(frame.y(), frame.x()),
            vec![vec![back(e1b), fwd(e2b)]],
        ),
        curved(
            &mut body,
            about(frame.x(), frame.z()),
            vec![vec![back(e2b), back(e1a)]],
        ),
        curved(
            &mut body,
            about(frame.x(), frame.y()),
            vec![vec![fwd(e2a), fwd(e1b)]],
        ),
    ];
    body.add_shell(faces);
    Solid {
        name: "what two crossing cylinders share".to_owned(),
        body,
        volume: 16.0 * r.ppowi(3) / 3.0,
        area: 16.0 * r * r,
    }
}

/// The area between the hyperbola Z²/a² − y²/b² = 1 and its chord at
/// Z = c: (b/a) (c √(c² − a²) − a² acosh(c/a)).
fn hyperbolic_segment(a: f64, b: f64, c: f64) -> f64 {
    b / a * (c * (c * c - a * a).sqrt() - a * a * (c / a).pacosh())
}

/// A cone (base `r` on the frame's xy plane, apex at `h`) cut by the
/// plane x = `d`, parallel to its axis: the piece beyond the plane
/// (`small`) or the rest.
///
/// The cut is a hyperbola: with Z measured down from the apex,
/// Z²/(dh/r)² − y²/d² = 1, out to the chord at Z = h where y = ±√(r² − d²).
/// It is one rational quadratic: the end tangents meet on the axis of the
/// hyperbola at Z = a²/c, and the weight of that control point is c/a
/// (the curve passes through its vertex), here r/d.
///
/// The cut face is the hyperbolic segment. The base is cut along a chord
/// into circle segments, r² (φ − sin φ cos φ) with cos φ = d/r beyond the
/// plane. Every piece of the cone's wall is s/r times as large as what it
/// covers of the base (s the slant height). The volume beyond the plane
/// is the integral of the hyperbolic segments of the planes x = d to r.
fn cut_cone(frame: &Frame, r: f64, h: f64, d: f64, small: bool) -> Solid {
    let mut body = Body::new(BodyKind::Solid);
    let yc = (r * r - d * d).sqrt();
    let phi = (d / r).pacos();
    let p0 = body.add_vertex(frame.point(d, -yc, 0.0));
    let p2 = body.add_vertex(frame.point(d, yc, 0.0));
    let hyperbola = NurbsCurve::new(
        2,
        vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        vec![
            frame.point(d, -yc, 0.0),
            frame.point(d, 0.0, h * (1.0 - d * d / (r * r))),
            frame.point(d, yc, 0.0),
        ],
        vec![1.0, r / d, 1.0],
    )
    .unwrap();
    let cut = body.add_edge(Curve3::Nurbs(hyperbola), (0.0, 1.0), p0, p2);
    let chord = line(&mut body, (p0, p2));
    let circle = Curve3::Circle(Circle::new(*frame, r).unwrap());
    let cone = Surface::Cone(Cone::new(*frame, r, -(r / h).patan()).unwrap());
    let on_plane = frame.point(d, 0.0, 0.0);
    let faces = if small {
        let arc = body.add_edge(circle, (-phi, phi), p0, p2);
        vec![
            curved(&mut body, cone, vec![vec![fwd(arc), back(cut)]]),
            flat(&mut body, on_plane, -frame.x(), vec![fwd(cut), back(chord)]),
            flat(
                &mut body,
                frame.origin(),
                -frame.z(),
                vec![fwd(chord), back(arc)],
            ),
        ]
    } else {
        let arc = body.add_edge(circle, (phi, TAU - phi), p2, p0);
        vec![
            curved(&mut body, cone, vec![vec![fwd(arc), fwd(cut)]]),
            flat(&mut body, on_plane, frame.x(), vec![back(cut), fwd(chord)]),
            flat(
                &mut body,
                frame.origin(),
                -frame.z(),
                vec![back(chord), back(arc)],
            ),
        ]
    };
    body.add_shell(faces);

    let segment = hyperbolic_segment(d * h / r, d, h);
    let base_beyond = r * r * (phi - phi.psin() * phi.pcos());
    let slant = r.phypot(h);
    // ∫ S(x) dx from d to r, with x = r − (r − d) t² so that the square
    // root in S at x = r is no trouble to the rule.
    let rule = Rule::new(16);
    let beyond = rule.integrate(0.0, 1.0, 32, |t| {
        let x = r - (r - d) * t * t;
        if x >= r {
            return 0.0;
        }
        hyperbolic_segment(x * h / r, x, h) * 2.0 * (r - d) * t
    });
    let (volume, base) = if small {
        (beyond, base_beyond)
    } else {
        (PI * r * r * h / 3.0 - beyond, PI * r * r - base_beyond)
    };
    Solid {
        name: format!("cone cut along its axis at {d}, small piece: {small}"),
        body,
        volume,
        area: base * slant / r + base + segment,
    }
}

fn frames(k: f64) -> [Frame; 2] {
    [
        Frame::WORLD,
        Frame::new(
            Vec3::new(1.0, -2.0, 0.5) * k,
            Vec3::new(0.3, -0.4, 1.0),
            Vec3::new(1.0, 0.2, 0.0),
        )
        .unwrap(),
    ]
}

#[test]
fn solids_with_spline_edges_match_their_closed_forms() {
    let mut worst = (0.0_f64, 0.0_f64);
    let mut count = 0;
    for k in [1e-3, 1.0, 1e6] {
        let tolerance = Tolerance::new(1e-6 * k, 1e-9).unwrap();
        for frame in frames(k) {
            let mut all = Vec::new();
            for pieces in [3, 4, 5, 7] {
                all.push(cylinder(&frame, 1.5 * k, 2.0 * k, pieces));
                all.push(frustum(&frame, 2.0 * k, 0.8 * k, 1.5 * k, pieces));
                all.push(cap(&frame, 2.0 * k, 0.7 * k, pieces));
                all.push(cap(&frame, 2.0 * k, -1.2 * k, pieces));
            }
            all.push(steinmetz(&frame, 1.5 * k));
            for d in [0.3, 1.0, 1.7] {
                for small in [true, false] {
                    all.push(cut_cone(&frame, 2.0 * k, 3.0 * k, d * k, small));
                }
            }
            for solid in all {
                let what = format!("{} at scale {k}", solid.name);
                assert_eq!(check(&solid.body, &tolerance), vec![], "{what}");
                let mass = mass_properties(&solid.body).unwrap_or_else(|e| panic!("{what}: {e}"));
                // 1e-11: the curves are exact and the sums converge; a
                // few digits go to the tilted frames.
                let volume = (mass.volume - solid.volume).abs() / solid.volume;
                let area = (mass.area - solid.area).abs() / solid.area;
                assert!(
                    volume <= 1e-11,
                    "{what}: volume {} vs {} ({volume:e})",
                    mass.volume,
                    solid.volume
                );
                assert!(
                    area <= 1e-11,
                    "{what}: area {} vs {} ({area:e})",
                    mass.area,
                    solid.area
                );
                worst = (worst.0.max(volume), worst.1.max(area));
                count += 1;
            }
        }
    }
    println!(
        "{count} solids; worst relative error: volume {:e}, area {:e}",
        worst.0, worst.1
    );
}

/// The two pieces of a cut cone are the cone: V = πr²h/3.
#[test]
fn the_pieces_of_a_cut_cone_add_up() {
    let frame = frames(1.0)[1];
    for d in [0.3, 1.0, 1.7] {
        let volume: f64 = [true, false]
            .into_iter()
            .map(|small| {
                mass_properties(&cut_cone(&frame, 2.0, 3.0, d, small).body)
                    .unwrap()
                    .volume
            })
            .sum();
        let whole = PI * 4.0;
        assert!(
            (volume - whole).abs() <= 1e-11 * whole,
            "{volume} vs {whole}"
        );
    }
}

/// SplitMix64: seeded, no dependency.
struct Random(u64);

impl Random {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// A closed spline in the plane of `frame`, around its origin, with
/// interior knots of the multiplicities given (in turn, as many as fit),
/// at uneven places; rational when `rational`.
fn loop_spline(
    frame: &Frame,
    degree: usize,
    points: usize,
    multiplicities: &[usize],
    rational: bool,
    random: &mut Random,
    k: f64,
) -> NurbsCurve {
    // Control points around the origin at uneven distances, the last on
    // the first.
    let mut ctrl = Vec::new();
    let mut weights = Vec::new();
    for i in 0..points - 1 {
        let angle = TAU * (i as f64 + 0.3 * random.next()) / (points - 1) as f64;
        let radius = k * (1.0 + 0.5 * random.next());
        ctrl.push(frame.point(radius * angle.pcos(), radius * angle.psin(), 0.0));
        weights.push(if rational {
            0.4 + 2.0 * random.next()
        } else {
            1.0
        });
    }
    ctrl.push(ctrl[0]);
    weights.push(weights[0]);
    // Clamped, with `points − degree − 1` interior knots.
    let mut knots = vec![0.0; degree + 1];
    let mut left = points - degree - 1;
    let mut at = 0.0;
    let mut turn = 0;
    while left > 0 {
        let times = multiplicities[turn % multiplicities.len()]
            .min(degree)
            .min(left);
        at += 0.3 + random.next();
        knots.extend(std::iter::repeat_n(at, times));
        left -= times;
        turn += 1;
    }
    at += 0.3 + random.next();
    knots.extend(std::iter::repeat_n(at, degree + 1));
    NurbsCurve::new(degree, knots, ctrl, weights).unwrap()
}

/// The area inside a closed curve in the plane of `frame`, by Green's
/// theorem piece by piece: ½ ∮ (x dy − y dx) with the rule applied between
/// each two knots, where the integrand is a polynomial of degree 2p − 1
/// (exact with 16 nodes for p ≤ 16) or, for a rational spline, a smooth
/// rational function, for which each stretch is divided in 16.
fn area_inside(frame: &Frame, curve: &NurbsCurve) -> f64 {
    let rule = Rule::new(16);
    let mut stops: Vec<f64> = curve.knots().to_vec();
    stops.dedup();
    let mut area = 0.0;
    for pair in stops.windows(2) {
        area += rule.integrate(pair[0], pair[1], 16, |t| {
            let (point, tangent) = curve.derivatives(t);
            let (x, y, _) = frame.local(point);
            let (dx, dy) = (tangent.dot(frame.x()), tangent.dot(frame.y()));
            0.5 * (x * dy - y * dx)
        });
    }
    area
}

/// A flat face bounded by one closed spline, for every degree, with knots
/// of every multiplicity up to the degree (a kink at full multiplicity),
/// polynomial and rational.
#[test]
fn a_face_inside_a_spline_has_the_area_inside_the_spline() {
    let mut random = Random(0x3DC8_AF71);
    let mut worst = 0.0_f64;
    let mut count = 0;
    for k in [1e-3, 1.0, 1e6] {
        for frame in frames(k) {
            for degree in 1..=5_usize {
                for rational in [false, true] {
                    let mut patterns: Vec<Vec<usize>> = (1..=degree).map(|m| vec![m]).collect();
                    patterns.push((1..=degree).collect());
                    for multiplicities in patterns {
                        for points in [degree + 4, 2 * degree + 5, 23] {
                            let curve = loop_spline(
                                &frame,
                                degree,
                                points,
                                &multiplicities,
                                rational,
                                &mut random,
                                k,
                            );
                            let expected = area_inside(&frame, &curve);
                            assert!(expected > 0.0, "the loop runs counter-clockwise");
                            let (t0, t1) = curve.domain();
                            let mut body = Body::new(BodyKind::Sheet);
                            let vertex = body.add_vertex(curve.eval(t0));
                            let edge =
                                body.add_edge(Curve3::Nurbs(curve), (t0, t1), vertex, vertex);
                            let face = flat(&mut body, frame.origin(), frame.z(), vec![fwd(edge)]);
                            body.add_shell(vec![face]);
                            let what = format!(
                                "degree {degree}, {points} points, multiplicities \
                                 {multiplicities:?}, rational: {rational}, scale {k}"
                            );
                            let area = mass_properties(&body)
                                .unwrap_or_else(|e| panic!("{what}: {e}"))
                                .area;
                            let off = (area - expected).abs() / expected;
                            assert!(off <= 1e-11, "{what}: {area} vs {expected} ({off:e})");
                            worst = worst.max(off);
                            count += 1;
                        }
                    }
                }
            }
        }
    }
    println!("{count} faces; worst relative error {worst:e}");
}

/// A cylinder whose wall is a NURBS surface that closes on itself around
/// v without `mass`'s closure test seeing it: the last column of its net
/// is moved off the first by `apart` of the radius (a surface read from a
/// file closes to its writer's precision, not to the nine digits the test
/// asks), less than the tolerance, so the body is valid. Its rims go
/// around v, over the seam. Found in review (2026-10-17): the plan read
/// how far a loop goes around from where the samples are, which telescopes
/// to nothing where no closure is known, and the wall was measured as a
/// face its loops do not bound. Measured right, or refused, never wrong.
#[test]
fn a_wall_that_closes_unseen_is_measured_right_or_refused() {
    for apart in [0.0, 3e-7] {
        let frame = Frame::WORLD;
        let (radius, height) = (1.0, 2.0);
        let ring = spline_ellipse(&frame, radius, radius, 4);
        let Curve3::Nurbs(ring_spline) = &ring else {
            panic!("the ring is a spline")
        };
        let mut ctrl = Vec::new();
        let mut weights = Vec::new();
        let count = ring_spline.control_points().len();
        for z in [0.0, height] {
            for (j, (&p, &w)) in ring_spline
                .control_points()
                .iter()
                .zip(ring_spline.weights())
                .enumerate()
            {
                let p = if j + 1 == count {
                    p + Vec3::X * (apart * radius)
                } else {
                    p
                };
                ctrl.push(p + Vec3::Z * z);
                weights.push(w);
            }
        }
        let wall_surface = craft3d_geom::NurbsSurface::new(
            1,
            2,
            vec![0.0, 0.0, 1.0, 1.0],
            ring_spline.knots().to_vec(),
            2,
            count,
            ctrl,
            weights,
        )
        .unwrap();
        let mut body = Body::new(BodyKind::Solid);
        let bottom = spline_ring(&mut body, &frame, 0.0, radius, 4);
        let top = spline_ring(&mut body, &frame, height, radius, 4);
        let wall = curved(
            &mut body,
            Surface::Nurbs(wall_surface),
            vec![vec![fwd(bottom)], vec![back(top)]],
        );
        let floor = flat(&mut body, frame.origin(), -frame.z(), vec![back(bottom)]);
        let lid = flat(
            &mut body,
            frame.point(0.0, 0.0, height),
            frame.z(),
            vec![fwd(top)],
        );
        body.add_shell(vec![wall, floor, lid]);
        // Which way the wall faces: out, as the cylinder's.
        let ((u0, u1), (v0, v1)) = body.faces[wall.0 as usize].surface.domain();
        let (u, v) = (0.5 * (u0 + u1), 0.37 * (v0 + v1));
        let s = &body.faces[wall.0 as usize].surface;
        let out = s.eval(u, v) - Vec3::Z * s.eval(u, v).z;
        if s.normal(u, v).unwrap().dot(out) < 0.0 {
            body.faces[wall.0 as usize].reversed = true;
        }
        assert_eq!(check(&body, &Tolerance::default()), [], "{apart}");
        // The bound's integration plans the same way, to the bit.
        assert_eq!(
            craft3d_topo::mass_bound(&body).map(|(m, _)| m),
            mass_properties(&body),
            "{apart}"
        );
        match mass_properties(&body) {
            Ok(m) => {
                let volume = PI * radius * radius * height;
                assert!(
                    (m.volume - volume).abs() <= 1e-6 * volume,
                    "{apart}: {} for {volume}",
                    m.volume
                );
            }
            Err(e) => assert!(apart > 0.0, "{e:?}"),
        }
    }
}

/// A ring around a diamond as a bilinear spline surface, `u` across the
/// ring (radius 1 + u, or as `last` says for the last column) and `v`
/// around it in four spans, its last column (v = 4) back on the first
/// direction: the borders v = 0 and v = 4 are two curves near each other,
/// which the closure test does not take for a seam.
fn diamond_ring(last: impl Fn(f64) -> Vec3) -> craft3d_geom::NurbsSurface {
    let directions = [Vec3::X, Vec3::Y, -Vec3::X, -Vec3::Y];
    let mut ctrl = Vec::new();
    for u in [0.0, 1.0] {
        for d in directions {
            ctrl.push(d * (1.0 + u));
        }
        ctrl.push(last(u));
    }
    craft3d_geom::NurbsSurface::new(
        1,
        1,
        vec![0.0, 0.0, 1.0, 1.0],
        vec![0.0, 0.0, 1.0, 2.0, 3.0, 4.0, 4.0],
        2,
        5,
        ctrl,
        vec![1.0; 10],
    )
    .unwrap()
}

/// The image of the straight segment of parameters from `a` to `b`, both
/// in the span of v starting at `span`, on a bilinear surface: a quadratic
/// whose middle control point is the polar form at the ends, exactly on
/// the surface.
fn segment_on(s: &craft3d_geom::NurbsSurface, a: (f64, f64), b: (f64, f64)) -> Curve3 {
    let middle = (s.eval(a.0, b.1) + s.eval(b.0, a.1)) * 0.5;
    Curve3::Nurbs(
        NurbsCurve::new(
            2,
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            vec![s.eval(a.0, a.1), middle, s.eval(b.0, b.1)],
            vec![1.0; 3],
        )
        .unwrap(),
    )
}

/// The area of the face over a quadrilateral of parameters (corners in
/// order, all in one span of v) by Gauss–Legendre on a grid of it.
fn area_over(s: &craft3d_geom::NurbsSurface, quad: [(f64, f64); 4]) -> f64 {
    let rule = Rule::new(16);
    let cells = 32;
    let at = |x: f64, y: f64| {
        let lerp =
            |p: (f64, f64), q: (f64, f64), t: f64| (p.0 + (q.0 - p.0) * t, p.1 + (q.1 - p.1) * t);
        let (bottom, top) = (lerp(quad[0], quad[1], x), lerp(quad[3], quad[2], x));
        lerp(bottom, top, y)
    };
    let mut area = 0.0;
    for ci in 0..cells {
        for cj in 0..cells {
            let (x0, y0) = (ci as f64 / cells as f64, cj as f64 / cells as f64);
            let h = 1.0 / cells as f64;
            for (x, wx) in rule.points(x0, x0 + h) {
                for (y, wy) in rule.points(y0, y0 + h) {
                    // The map from the unit square to the parameters is
                    // bilinear: its derivatives in closed form.
                    let p = at(x, y);
                    let j = (
                        (1.0 - y) * (quad[1].0 - quad[0].0) + y * (quad[2].0 - quad[3].0),
                        (1.0 - y) * (quad[1].1 - quad[0].1) + y * (quad[2].1 - quad[3].1),
                        at(x, 1.0).0 - at(x, 0.0).0,
                        at(x, 1.0).1 - at(x, 0.0).1,
                    );
                    let (_, su, sv) = s.derivatives(p.0, p.1);
                    let du = su * j.0 + sv * j.1;
                    let dv = su * j.2 + sv * j.3;
                    area += du.cross(dv).length() * wx * wy;
                }
            }
        }
    }
    area
}

/// An hourglass on [`diamond_ring`]: one lobe over the parameters next to
/// v = 0, one next to v = 4, meeting in a neck `2 neck` wide in u around
/// `(at0, 0)` and `(at4, 4)`, which are the same point. The face is
/// measured right, or refused; never measured short by the strip between
/// the lobes that its loop does not close around in the parameters.
fn hourglass(s: craft3d_geom::NurbsSurface, (at0, at4): (f64, f64), neck: f64) -> (Body, f64) {
    let mut body = Body::new(BodyKind::Sheet);
    let lobe0 = [
        (at0 - neck, 0.0),
        (0.2, 0.4),
        (0.86, 0.4),
        (at0 + neck, 0.0),
    ];
    let lobe4 = [
        (at4 + neck, 4.0),
        (0.86, 3.6),
        (0.2, 3.6),
        (at4 - neck, 4.0),
    ];
    let n1 = body.add_vertex(s.eval(lobe0[0].0, lobe0[0].1));
    let a1 = body.add_vertex(s.eval(0.2, 0.4));
    let a2 = body.add_vertex(s.eval(0.86, 0.4));
    let n2 = body.add_vertex(s.eval(lobe0[3].0, lobe0[3].1));
    let b2 = body.add_vertex(s.eval(0.86, 3.6));
    let b1 = body.add_vertex(s.eval(0.2, 3.6));
    let edge = |body: &mut Body, a: (f64, f64), b: (f64, f64), from, to| {
        body.add_edge(segment_on(&s, a, b), (0.0, 1.0), from, to)
    };
    let edges = [
        edge(&mut body, lobe0[0], lobe0[1], n1, a1),
        edge(&mut body, lobe0[1], lobe0[2], a1, a2),
        edge(&mut body, lobe0[2], lobe0[3], a2, n2),
        edge(&mut body, lobe4[0], lobe4[1], n2, b2),
        edge(&mut body, lobe4[1], lobe4[2], b2, b1),
        edge(&mut body, lobe4[2], lobe4[3], b1, n1),
    ];
    let area = area_over(&s, [lobe0[0], lobe0[3], lobe0[2], lobe0[1]])
        + area_over(&s, [lobe4[3], lobe4[0], lobe4[1], lobe4[2]]);
    let face = curved(
        &mut body,
        Surface::Nurbs(s),
        vec![edges.iter().map(|&e| fwd(e)).collect()],
    );
    // The loop runs clockwise in the parameters: the face's normal is
    // the surface's turned over, so that the loop bounds it.
    body.faces[face.0 as usize].reversed = true;
    body.add_shell(vec![face]);
    (body, area)
}

#[track_caller]
fn right_or_refused(body: &Body, area: f64, what: &str) {
    assert_eq!(check(body, &Tolerance::default()), [], "{what}");
    match mass_properties(body) {
        Ok(m) => assert!(
            (m.area - area).abs() <= 1e-7 * area,
            "{what}: area {} for {area} (off by {:e})",
            m.area,
            (m.area - area).abs() / area
        ),
        Err(MassError::Unsupported { .. }) => {}
        Err(e) => panic!("{what}: {e:?}"),
    }
}

/// Review of 2026-10-17: the two borders of the ring cross each other at
/// u = 0.53 (the last column lifted by `0.5 (u − 0.53)`), and the neck of
/// the hourglass is on the crossing. The borders were compared only at
/// equal parameters, where the samples jump and at seventeen places, all
/// too far from the crossing: the face came out short by 2.9e-5 (refused
/// since, the borders not proven apart).
#[test]
fn an_hourglass_over_where_two_borders_cross_is_measured_right_or_refused() {
    let s = diamond_ring(|u| Vec3::new(1.0 + u, 0.0, 0.5 * (u - 0.53)));
    let (body, area) = hourglass(s, (0.53, 0.53), 1e-6);
    right_or_refused(&body, area, "crossing borders");
}

/// The borders meet as point sets along a stretch, but at other
/// parameters (the last column drawn over x from 1.2 to 1.82 where the
/// first runs from 1 to 2): a seam over part of the border that a
/// comparison at equal parameters cannot see. The hourglass's neck is on
/// it, at x = 1.53. Measured 3.4e-5 short before; refused since.
#[test]
fn an_hourglass_over_a_seam_drawn_at_other_parameters_is_measured_right_or_refused() {
    let s = diamond_ring(|u| Vec3::new(1.2 + 0.62 * u, 0.0, 0.0));
    let (body, area) = hourglass(s, (0.53, 0.33 / 0.62), 1e-6);
    right_or_refused(&body, area, "a seam at other parameters");
}

/// The unit square `k` wide drawn as a spline surface whose row v = 1
/// moves at `ε` of its speed at v = 0 at the corner (1, 1): `x = u + v
/// ((1 − ε) u − (1 − ε) u²)`, `y = v`, quadratic in u and linear in v, so
/// the row v = 1 runs `x = (2 − ε) u − (1 − ε) u²` with speed `ε` at u = 1
/// (for `ε` = 0 it stands still there: the corner of the strip beside a
/// developable's fan at a fold, docs/design/developable.md). Its straight
/// sides are its borders and its area is `k²` for every `ε`.
fn standing_square(k: f64, eps: f64) -> Body {
    let rows = [[0.0, 0.5, 1.0], [0.0, 0.5 * (2.0 - eps), 1.0]];
    let mut ctrl = Vec::new();
    for i in 0..3 {
        for (j, row) in rows.iter().enumerate() {
            ctrl.push(Vec3::new(row[i], j as f64, 0.0) * k);
        }
    }
    let surface = craft3d_geom::NurbsSurface::new(
        2,
        1,
        vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        vec![0.0, 0.0, 1.0, 1.0],
        3,
        2,
        ctrl,
        vec![1.0; 6],
    )
    .unwrap();
    let mut body = Body::new(BodyKind::Sheet);
    let corners = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]
        .map(|(x, y)| body.add_vertex(Vec3::new(x, y, 0.0) * k));
    let edges: Vec<EdgeId> = (0..4)
        .map(|i| line(&mut body, (corners[i], corners[(i + 1) % 4])))
        .collect();
    let face = curved(
        &mut body,
        Surface::Nurbs(surface),
        vec![edges.iter().map(|&e| fwd(e)).collect()],
    );
    body.add_shell(vec![face]);
    body
}

/// The plane square whose parametrisation stands still at a corner, or
/// all but (2026-10-27, owed from r1-kayakgaps2): refused before ("does
/// not settle"), as the boundary integral weighed the rates of u, which
/// along the side u = 1 through that corner are the rounding of the feet
/// over the small speed. Measured since, against the rates of v; to
/// 1e-9 at three scales, the speed at the corner from nothing to a tenth.
#[test]
fn a_square_whose_parametrisation_stands_still_at_a_corner_is_measured() {
    for k in [1e-3, 1.0, 1e6] {
        for eps in [0.0, 1e-6, 1e-3, 1e-1, 1.0] {
            let body = standing_square(k, eps);
            assert_eq!(check(&body, &Tolerance::default()), [], "{k} {eps}");
            let area = mass_properties(&body)
                .unwrap_or_else(|e| panic!("{k} {eps}: {e}"))
                .area;
            assert!(
                (area - k * k).abs() <= 1e-9 * k * k,
                "{k} {eps}: area {area} (off by {:e})",
                (area - k * k) / (k * k)
            );
        }
    }
}

/// A long panel as a ruled spline surface (the sea kayak's, 2026-10-27:
/// hundreds of spans along it): a profile in the xz-plane through 782
/// points, extruded along y. Its area is the profile's length times the
/// height, the length by Gauss–Legendre span by span. What the measure
/// costs is printed (review of r1-kayakgaps2: the test for corners where
/// the parametrisation stands still ran global projections at every end
/// of every piece).
#[test]
fn a_long_spline_panel_is_measured() {
    let count = 782;
    for k in [1e-3, 1.0, 1e6] {
        let (length, height) = (5000.0 * k, 190.0 * k);
        let profile: Vec<Vec3> = (0..count)
            .map(|i| {
                let s = i as f64 / (count - 1) as f64;
                // Unevenly spaced along x, a gentle curve in z.
                let x = length * (s + 0.01 * (7.0 * s).psin());
                Vec3::new(x, 0.0, 120.0 * k * (std::f64::consts::PI * s).psin())
            })
            .collect();
        let spans = count - 3;
        let mut knots = vec![0.0; 4];
        knots.extend((1..spans).map(|i| i as f64 / spans as f64));
        knots.extend([1.0; 4]);
        let mut ctrl = Vec::new();
        for p in &profile {
            ctrl.push(*p);
            ctrl.push(*p + Vec3::Y * height);
        }
        let surface = craft3d_geom::NurbsSurface::new(
            3,
            1,
            knots.clone(),
            vec![0.0, 0.0, 1.0, 1.0],
            count,
            2,
            ctrl,
            vec![1.0; 2 * count],
        )
        .unwrap();
        let row = |lift: f64| {
            NurbsCurve::new(
                3,
                knots.clone(),
                profile.iter().map(|p| *p + Vec3::Y * lift).collect(),
                vec![1.0; count],
            )
            .unwrap()
        };
        let (bottom, top) = (row(0.0), row(height));
        let mut body = Body::new(BodyKind::Sheet);
        let ends = |c: &NurbsCurve| (c.eval(0.0), c.eval(1.0));
        let ((a, b), (d, c)) = (ends(&bottom), ends(&top));
        let [va, vb, vc, vd] = [a, b, c, d].map(|p| body.add_vertex(p));
        let e0 = body.add_edge(Curve3::Nurbs(bottom.clone()), (0.0, 1.0), va, vb);
        let e1 = line(&mut body, (vb, vc));
        let e2 = body.add_edge(Curve3::Nurbs(top), (0.0, 1.0), vd, vc);
        let e3 = line(&mut body, (vd, va));
        let face = curved(
            &mut body,
            Surface::Nurbs(surface.clone()),
            vec![vec![fwd(e0), fwd(e1), back(e2), fwd(e3)]],
        );
        body.add_shell(vec![face]);
        assert_eq!(check(&body, &Tolerance::default()), [], "{k}");
        let rule = Rule::new(16);
        let along: f64 = (0..spans)
            .map(|i| {
                let (s0, s1) = (i as f64 / spans as f64, (i + 1) as f64 / spans as f64);
                rule.points(s0, s1)
                    .map(|(s, w)| w * bottom.derivatives(s).1.length())
                    .sum::<f64>()
            })
            .sum();
        let expected = along * height;
        let started = std::time::Instant::now();
        let measured = mass_properties(&body).unwrap_or_else(|e| panic!("{k}: {e}"));
        let took = started.elapsed();
        // What 32 searches over the whole surface cost here (four pieces,
        // eight feet each: what the corners' test took before).
        let searched = Surface::Nurbs(surface.clone());
        let started = std::time::Instant::now();
        for i in 0..32 {
            let p = surface.eval(f64::from(i) / 31.0, 0.37) + Vec3::Z * (1e-9 * k);
            searched.project(p).unwrap();
        }
        let searches = started.elapsed();
        eprintln!("scale {k}: measured in {took:?}; 32 projections {searches:?}");
        assert!(
            (measured.area - expected).abs() <= 1e-9 * expected,
            "{k}: area {} for {expected} (off by {:e})",
            measured.area,
            (measured.area - expected) / expected
        );
    }
}
