//! The nets of a tip (`craft3d_geom::tip`) as the faces of a body.
//!
//! Nothing in the kernel builds a tip yet: the bodies here are put
//! together by hand, a flat base on the rim's plane under the tip's
//! faces, with the rim curves and the rails as their edges. What is
//! asked is what the kernel makes of such faces: the validity check, the
//! exact measure against closed forms (the half ball; two thirds of base
//! times length over any prism with quarter ellipses, eleven fifteenths
//! with parabolas) and a watertight mesh at two tolerances, at three
//! scales and in two frames.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_geom::pole::Border;
use craft3d_geom::tip::{Tip, TipForm, TipRim, TipSpec, tip_nets};
use craft3d_geom::{Curve3, NurbsCurve, NurbsSurface, Plane, Surface};
use craft3d_math::quadrature::Rule;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_tess::{TessParams, tessellate_body};
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId, check, mass_properties};
use std::f64::consts::{FRAC_1_SQRT_2, PI, SQRT_2};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];
/// The fullness of quarter ellipses (the conic's weight is √½) and of
/// parabolas (weight 1).
const ROUND: f64 = SQRT_2 - 1.0;
const PARABOLA: f64 = 0.5;

/// One Bézier piece of a section in its plane: degree, control points,
/// weights.
type Piece = (usize, Vec<[f64; 2]>, Vec<f64>);

fn frames(k: f64) -> [Frame; 2] {
    [
        Frame::WORLD,
        Frame::new(
            Vec3::new(3.0 * k, -2.0 * k, 5.0 * k),
            Vec3::new(1.0, 2.0, 2.0),
            Vec3::new(1.0, 0.0, 0.0),
        )
        .unwrap(),
    ]
}

fn circle(r: f64) -> Vec<Piece> {
    let quarters = [
        [(1.0, 0.0), (1.0, 1.0), (0.0, 1.0)],
        [(0.0, 1.0), (-1.0, 1.0), (-1.0, 0.0)],
        [(-1.0, 0.0), (-1.0, -1.0), (0.0, -1.0)],
        [(0.0, -1.0), (1.0, -1.0), (1.0, 0.0)],
    ];
    quarters
        .iter()
        .map(|q| {
            (
                2,
                q.iter().map(|&(x, y)| [r * x, r * y]).collect(),
                vec![1.0, FRAC_1_SQRT_2, 1.0],
            )
        })
        .collect()
}

/// The sections with the area each encloses, in closed form: a parabola
/// over its chord holds two thirds of chord times height, the cubic
/// `(a, 0), (a, b), (−a, b), (−a, 0)` three fifths of `2 a b`.
fn sections() -> Vec<(&'static str, Vec<Piece>, f64, [f64; 2])> {
    let cubic = |a: f64, b: f64| (3, vec![[a, 0.0], [a, b], [-a, b], [-a, 0.0]], vec![1.0; 4]);
    let parabola = |a: f64, b: f64| (2, vec![[a, 0.0], [0.0, 2.0 * b], [-a, 0.0]], vec![1.0; 3]);
    let line = |p: [f64; 2], q: [f64; 2]| (1, vec![p, q], vec![1.0; 2]);
    vec![
        // Two cubic arcs, tangent where they meet.
        (
            "oval",
            vec![cubic(6.0, 1.0), cubic(-6.0, -1.0)],
            14.4,
            [0.3, 0.1],
        ),
        // Two parabolas with two sharp corners.
        (
            "lens",
            vec![parabola(6.0, 1.0), parabola(-6.0, -1.0)],
            16.0,
            [0.3, 0.1],
        ),
        // A cambered deck over a U, sharp at the sheer.
        (
            "hull",
            vec![parabola(8.0, 0.96), cubic(-8.0, -28.0)],
            279.04,
            [0.5, -9.0],
        ),
        (
            "square",
            vec![
                line([1.0, -1.0], [1.0, 1.0]),
                line([1.0, 1.0], [-1.0, 1.0]),
                line([-1.0, 1.0], [-1.0, -1.0]),
                line([-1.0, -1.0], [1.0, -1.0]),
            ],
            4.0,
            [0.2, -0.1],
        ),
        ("circle", circle(5.0), 25.0 * PI, [0.0, 0.0]),
    ]
}

fn one_span(degree: usize) -> Vec<f64> {
    let mut knots = vec![0.0; degree + 1];
    knots.extend(std::iter::repeat_n(1.0, degree + 1));
    knots
}

/// The side faces behind the section's curves in the xy plane of
/// `frame`: ruled to each from the row `back` gives for its control
/// points.
fn sides(
    frame: &Frame,
    k: f64,
    section: &[Piece],
    back: impl Fn(Vec3) -> Vec3,
) -> Vec<NurbsSurface> {
    section
        .iter()
        .map(|(degree, points, weights)| {
            let mut net = Vec::new();
            let mut net_weights = Vec::new();
            for (p, &w) in points.iter().zip(weights) {
                let c = frame.point(p[0] * k, p[1] * k, 0.0);
                net.extend([back(c), c]);
                net_weights.extend([w, w]);
            }
            NurbsSurface::new(
                *degree,
                1,
                one_span(*degree),
                one_span(1),
                points.len(),
                2,
                net,
                net_weights,
            )
            .unwrap()
        })
        .collect()
}

fn tip_over(sides: &[NurbsSurface], at: Vec3, normal: Vec3, fullness: f64, k: f64) -> Tip {
    let rims: Vec<TipRim> = sides
        .iter()
        .map(|s| TipRim::of_border(s, Border::VEnd).unwrap())
        .collect();
    tip_nets(
        &rims,
        &TipSpec {
            tip: at,
            normal,
            fullness,
            tolerance: 1e-9 * k,
            angular: 1e-9,
        },
    )
    .unwrap()
}

fn forward(edge: EdgeId) -> Coedge {
    Coedge {
        edge,
        reversed: false,
    }
}

fn backward(edge: EdgeId) -> Coedge {
    Coedge {
        edge,
        reversed: true,
    }
}

/// The solid between the tip's faces and the plane of `frame` its rim
/// lies in: one face per rim curve, each bounded by its rim curve and
/// two rails, and the flat base.
fn pod(tip: &Tip, frame: &Frame) -> Body {
    let mut body = Body::new(BodyKind::Solid);
    let count = tip.faces.len();
    let corners: Vec<_> = tip
        .rims
        .iter()
        .map(|rim| body.add_vertex(rim.control_points()[0]))
        .collect();
    let top = body.add_vertex(tip.tip);
    let rims: Vec<EdgeId> = (0..count)
        .map(|k| {
            let rim = &tip.rims[k];
            body.add_edge(
                Curve3::Nurbs(rim.clone()),
                rim.domain(),
                corners[k],
                corners[(k + 1) % count],
            )
        })
        .collect();
    let rails: Vec<EdgeId> = (0..count)
        .map(|k| {
            body.add_edge(
                Curve3::Nurbs(tip.rails[k].clone()),
                (0.0, 1.0),
                corners[k],
                top,
            )
        })
        .collect();
    let mut faces = Vec::new();
    for k in 0..count {
        let next = (k + 1) % count;
        // Round the face counter-clockwise seen from outside: along the
        // rim, up the next rail, down its own; the other way round where
        // the faces' own normals point inward.
        let coedges = if tip.counter_clockwise {
            vec![forward(rims[k]), forward(rails[next]), backward(rails[k])]
        } else {
            vec![forward(rails[k]), backward(rails[next]), backward(rims[k])]
        };
        let outline = body.add_loop(coedges);
        faces.push(body.add_face(
            Surface::Nurbs(tip.faces[k].clone()),
            !tip.counter_clockwise,
            vec![outline],
        ));
    }
    let down = frame.reversed_at(frame.origin()).unwrap();
    let coedges = if tip.counter_clockwise {
        (0..count).rev().map(|k| backward(rims[k])).collect()
    } else {
        (0..count).map(|k| forward(rims[k])).collect()
    };
    let outline = body.add_loop(coedges);
    faces.push(body.add_face(Surface::Plane(Plane::new(down)), false, vec![outline]));
    body.add_shell(faces);
    body
}

/// Valid, measured, and meshed watertight at two tolerances: the body's
/// volume and area.
#[track_caller]
fn sound(body: &Body, k: f64, size: f64, what: &str) -> (f64, f64) {
    let tolerance = Tolerance::new(1e-7 * k, 1e-9).unwrap();
    assert_eq!(check(body, &tolerance), [], "{what}");
    let exact = mass_properties(body).unwrap_or_else(|e| panic!("{what}: {e}"));
    for chord in [2e-2, 2e-3] {
        let chord = chord * size * k;
        let params = TessParams {
            chordal_tolerance: chord,
            ..TessParams::default()
        };
        let mesh = tessellate_body(body, &params).unwrap_or_else(|e| panic!("{what}: {e}"));
        let welded = mesh.weld().unwrap_or_else(|e| panic!("{what}: {e:?}"));
        assert!(welded.is_closed(), "{what}: the mesh is not watertight");
        assert!(
            (welded.volume() - exact.volume).abs() <= exact.area * chord,
            "{what}: mesh volume {} against {}",
            welded.volume(),
            exact.volume
        );
    }
    (exact.volume, exact.area)
}

#[track_caller]
fn assert_near(got: f64, want: f64, relative: f64, what: &str) {
    assert!(
        (got - want).abs() <= relative * want.abs(),
        "{what}: got {got:e}, want {want:e} (off by {:e})",
        ((got - want) / want).abs()
    );
}

#[test]
fn a_tip_over_a_prism_is_a_valid_body_of_the_closed_form_volume() {
    let mut worst = 0.0_f64;
    for k in SCALES {
        for frame in frames(k) {
            for (name, section, area, foot) in sections() {
                let prism = sides(&frame, k, &section, |c| c - frame.z() * (2.0 * k));
                let size = if name == "hull" { 30.0 } else { 10.0 };
                let length = 0.3 * size;
                let at = frame.point(foot[0] * k, foot[1] * k, length * k);
                for (fullness, share) in [(ROUND, 2.0 / 3.0), (PARABOLA, 11.0 / 15.0)] {
                    let what = format!("{name} at {k}, fullness {fullness}");
                    let tip = tip_over(&prism, at, frame.z(), fullness, k);
                    assert_eq!(tip.form, TipForm::Level, "{what}");
                    let body = pod(&tip, &frame);
                    let (volume, _) = sound(&body, k, size, &what);
                    let want = share * area * length * k * k * k;
                    assert_near(volume, want, 1e-9, &what);
                    worst = worst.max(((volume - want) / want).abs());
                }
            }
        }
    }
    println!("volumes over prisms by the exact measure: within {worst:e} of the closed forms");
}

#[test]
fn a_tip_over_a_rod_is_the_half_ball_from_four_curves_and_from_one() {
    for k in SCALES {
        for frame in frames(k) {
            let r = 5.0 * k;
            let at = frame.point(0.0, 0.0, r);
            let quarters = tip_over(
                &sides(&frame, k, &circle(5.0), |c| c - frame.z() * (2.0 * k)),
                at,
                frame.z(),
                ROUND,
                k,
            );
            // One closed curve for the whole rim: cut in two.
            let whole = NurbsCurve::circle(&frame, r).unwrap();
            let rim = TipRim {
                cross: whole
                    .weights()
                    .iter()
                    .map(|&w| {
                        let z = frame.z() * w;
                        [z.x, z.y, z.z, 0.0]
                    })
                    .collect(),
                curve: whole,
            };
            let halves = tip_nets(
                &[rim],
                &TipSpec {
                    tip: at,
                    normal: frame.z(),
                    fullness: ROUND,
                    tolerance: 1e-9 * k,
                    angular: 1e-9,
                },
            )
            .unwrap();
            for (tip, faces) in [(quarters, 4), (halves, 2)] {
                assert_eq!(tip.faces.len(), faces);
                let what = format!("half ball of {faces} faces at {k}");
                let (volume, area) = sound(&pod(&tip, &frame), k, 10.0, &what);
                assert_near(volume, 2.0 * PI * r * r * r / 3.0, 1e-10, &what);
                assert_near(area, 3.0 * PI * r * r, 1e-10, &what);
            }
        }
    }
}

/// The volume under the tip's faces down to the plane of `frame`, by the
/// divergence theorem and Gauss–Legendre on every face: this file's own
/// integral, to hold the kernel's measure against where no closed form
/// is known.
fn volume_under(tip: &Tip, frame: &Frame) -> f64 {
    let rule = Rule::new(40);
    let mut sum = 0.0;
    for face in &tip.faces {
        let ((u0, u1), _) = face.domain();
        for (u, wu) in rule.points(u0, u1) {
            for (v, wv) in rule.points(0.0, 1.0) {
                let (p, du, dv) = face.derivatives(u, v);
                sum += wu * wv * (p - frame.origin()).dot(du.cross(dv)) / 3.0;
            }
        }
    }
    if tip.counter_clockwise { sum } else { -sum }
}

#[test]
fn a_tip_over_sides_that_are_not_level_is_a_valid_body_too() {
    // The row before the rim lies in a plane that leans to the end: the
    // four-row net. And over a cone, the three-row net with a shoulder
    // smaller than the rim.
    for k in SCALES {
        for frame in frames(k) {
            for (name, section, _, foot) in sections() {
                let size = if name == "hull" { 30.0 } else { 10.0 };
                let reach = 0.5 * size * k;
                let leaning = sides(&frame, k, &section, |c| {
                    let (x, y, _) = frame.local(c);
                    let back = 2.0 * k * (1.0 + 0.3 * x / reach + 0.15 * y / reach);
                    frame.point(0.95 * x, 0.95 * y, -back)
                });
                let apex = frame.point(foot[0] * k, foot[1] * k, 1.5 * size * k);
                let cone = sides(&frame, k, &section, |c| apex + (c - apex) * 1.5);
                let at = frame.point(foot[0] * k, foot[1] * k, 0.3 * size * k);
                for (sides, form) in [(leaning, TipForm::General), (cone, TipForm::Level)] {
                    let what = format!("{name} at {k}, {form:?}");
                    let tip = tip_over(&sides, at, frame.z(), ROUND, k);
                    assert_eq!(tip.form, form, "{what}");
                    let (volume, _) = sound(&pod(&tip, &frame), k, size, &what);
                    assert_near(volume, volume_under(&tip, &frame), 1e-9, &what);
                }
            }
        }
    }
}

#[test]
fn a_rim_that_runs_the_other_way_gives_the_same_body() {
    let frame = frames(1.0)[1];
    let (_, section, area, foot) = sections().remove(2);
    let prism = sides(&frame, 1.0, &section, |c| c - frame.z() * 2.0);
    let rims: Vec<TipRim> = prism
        .iter()
        .rev()
        .map(|s| {
            TipRim::of_border(s, Border::VEnd)
                .unwrap()
                .reversed()
                .unwrap()
        })
        .collect();
    let tip = tip_nets(
        &rims,
        &TipSpec {
            tip: frame.point(foot[0], foot[1], 9.0),
            normal: frame.z(),
            fullness: ROUND,
            tolerance: 1e-9,
            angular: 1e-9,
        },
    )
    .unwrap();
    assert!(!tip.counter_clockwise);
    let (volume, _) = sound(&pod(&tip, &frame), 1.0, 30.0, "hull, clockwise");
    assert_near(volume, 2.0 / 3.0 * area * 9.0, 1e-9, "hull, clockwise");
}
