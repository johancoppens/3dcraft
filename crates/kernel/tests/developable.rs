//! Developable connections and unfolds against geometry with a known
//! answer (docs/design/developable.md › *What is checked*): rails on one
//! cone whose rulings must pass through its apex, a curve and a
//! differently parametrized translate whose rulings must be parallel to
//! the shift, so that the rulings are not matched by parameter (s ≠ t);
//! the coplanarity condition at every ruling, no curvature on the fitted
//! face, fans where the geometry puts them; unfolds keeping lengths and
//! areas, cylinders and cone frustums in closed form, two bands sharing a
//! circle, a seam; refusals. At three scales.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::refine::{interpolate, piece, reparametrized};
use craft3d_geom::{Circle, Cone, Curve3, Cylinder, Line, NurbsCurve, Surface};
use craft3d_kernel::developable::{Connected, Piece, Rail, RailPiece, connect};
use craft3d_kernel::unfold::develop;
use craft3d_kernel::{KernelError, primitives};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, FaceId};
use std::f64::consts::TAU;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn tol(scale: f64) -> Tolerance {
    Tolerance::new(1e-6 * scale, 1e-9).unwrap()
}

fn rail(curve: Curve3, range: (f64, f64)) -> Rail {
    Rail {
        pieces: vec![RailPiece {
            curve,
            range,
            reversed: false,
            source: 1,
        }],
    }
}

/// The cubic spline through `points` at uniform parameters.
fn spline(points: &[Vec3]) -> Rail {
    let n = points.len();
    let params: Vec<f64> = (0..n).map(|k| k as f64 / (n - 1) as f64).collect();
    let values: Vec<Vec<f64>> = points.iter().map(|p| vec![p.x, p.y, p.z]).collect();
    let (knots, ctrl) = interpolate(&params, 3, &values).unwrap();
    let ctrl: Vec<Vec3> = ctrl.iter().map(|c| Vec3::new(c[0], c[1], c[2])).collect();
    let curve = NurbsCurve::polynomial(3, knots, ctrl).unwrap();
    let range = curve.domain();
    rail(Curve3::Nurbs(curve), range)
}

fn arc(center: Vec3, radius: f64, from: f64, to: f64) -> Rail {
    let frame = Frame::new(center, Vec3::Z, Vec3::X).unwrap();
    rail(
        Curve3::Circle(Circle::new(frame, radius).unwrap()),
        (from, to),
    )
}

/// A rail's point and unit tangent at its parameter `s` (piece 0).
fn at(r: &Rail, s: f64) -> (Vec3, Vec3) {
    let piece = &r.pieces[0];
    let (a, b) = piece.range;
    let (p, d) = piece.curve.derivatives(a + s * (b - a));
    (p, d.normalized().unwrap())
}

/// The coplanarity condition det[Â', B̂', D̂] at a ruling.
fn condition(a: &Rail, b: &Rail, (s, t): (f64, f64)) -> f64 {
    let ((pa, ua), (pb, ub)) = (at(a, s), at(b, t));
    ua.dot(ub.cross((pb - pa).normalized().unwrap()))
}

fn check_rulings(made: &Connected, a: &Rail, b: &Rail) {
    for &ruling in &made.rulings {
        let f = condition(a, b, ruling);
        assert!(f.abs() < 1e-8, "condition {f} at {ruling:?}");
    }
    assert!(
        made.rulings
            .windows(2)
            .all(|w| w[1].0 >= w[0].0 && w[1].1 >= w[0].1),
        "rulings are monotone"
    );
}

/// Rails on one cone: a wavy spline and an arc of its base, the spline
/// sampled unevenly in angle (a parameter the arc's does not match).
/// Every ruling passes through the apex.
fn cone_rails(k: f64) -> (Rail, Rail, Vec3) {
    let (r, h) = (2.0 * k, 3.0 * k);
    let apex = Vec3::new(0.0, 0.0, h);
    // A circle of the cone at 0.4 of the way down from the apex, as a
    // rational spline, the piece from about 0.3 to 1.9 rad, its parameter
    // run through a cubic: exactly on the cone, matched to the base arc
    // by no parameter.
    let z = h * 0.6;
    let frame = Frame::new(Vec3::new(0.0, 0.0, z), Vec3::Z, Vec3::X).unwrap();
    let circle = NurbsCurve::circle(&frame, 0.4 * r).unwrap();
    let (c0, c1) = circle.domain();
    let span = c1 - c0;
    let piece = piece(&circle, c0 + 0.3 / TAU * span, c0 + 1.9 / TAU * span).unwrap();
    let curve = reparametrized(&piece, 0.5, 1.6).unwrap();
    let range = curve.domain();
    (
        rail(Curve3::Nurbs(curve), range),
        arc(Vec3::ZERO, r, 0.2, 2.0),
        apex,
    )
}

#[test]
fn rails_on_a_cone_are_joined_through_its_apex_with_a_fan_where_the_rail_starts_later() {
    for k in SCALES {
        let (a, b, apex) = cone_rails(k);
        let made = connect(&a, &b, &tol(k)).unwrap();
        check_rulings(&made, &a, &b);
        // Not matched by parameter.
        assert!(made.rulings.iter().any(|(s, t)| (s - t).abs() > 0.05));
        for &(s, t) in &made.rulings {
            let ((pa, _), (pb, _)) = (at(&a, s), at(&b, t));
            let d = (pb - pa).normalized().unwrap();
            let off = (apex - pa) - d * (apex - pa).dot(d);
            assert!(
                off.length() <= 1e-6 * k,
                "a ruling misses the apex by {}",
                off.length()
            );
        }
        assert!(made.curvature * k * k < 1e-4, "K {}", made.curvature);
        // The spline starts at about 0.3 rad, the arc at 0.2, and ends at
        // 2.0 past the spline's 1.9: fans from the spline's ends, to the
        // arc's points at the spline's ends' angles.
        assert_eq!(
            made.faces,
            vec![Piece::StartFan, Piece::Rulings, Piece::EndFan]
        );
        let (first, last) = (made.rulings[0], *made.rulings.last().unwrap());
        assert_eq!(first.0, 0.0);
        assert_eq!(last.0, 1.0);
        let angle_of = |p: Vec3| p.y.atan2(p.x);
        let angle = |t: f64| angle_of(at(&b, t).0);
        let (start, end) = (angle_of(at(&a, 0.0).0), angle_of(at(&a, 1.0).0));
        assert!(
            (angle(first.1) - start).abs() < 1e-6,
            "first ruling at {}",
            angle(first.1)
        );
        assert!(
            (angle(last.1) - end).abs() < 1e-6,
            "last ruling at {}",
            angle(last.1)
        );
        // Laid flat as one piece: lengths and area kept.
        let faces: Vec<FaceId> = (0..made.body.faces.len() as u32).map(FaceId).collect();
        let dev = develop(&made.body, &faces, &tol(k)).unwrap();
        let e = dev.error();
        assert!(e.area < 1e-6 && e.length < 1e-6, "{e:?}");
        assert!(e.map <= 0.5e-6 * k, "{e:?}");
    }
}

#[test]
fn a_curve_and_a_differently_drawn_translate_are_joined_by_parallel_rulings() {
    for k in SCALES {
        let shift = Vec3::new(0.3, 0.2, 2.0) * k;
        let wave = |x: f64| Vec3::new(x * k, (1.3 * x).sin() * k, 0.2 * x * x * k);
        let n = 40;
        let low: Vec<Vec3> = (0..=n).map(|j| wave(4.0 * j as f64 / n as f64)).collect();
        let a = spline(&low);
        // Its exact translate, the parameter run through a cubic.
        let Curve3::Nurbs(curve) = &a.pieces[0].curve else {
            panic!("a spline")
        };
        let moved: Vec<Vec3> = curve.control_points().iter().map(|p| *p + shift).collect();
        let moved = NurbsCurve::new(
            curve.degree(),
            curve.knots().to_vec(),
            moved,
            curve.weights().to_vec(),
        )
        .unwrap();
        let moved = reparametrized(&moved, 0.5, 1.6).unwrap();
        let range = moved.domain();
        let b = rail(Curve3::Nurbs(moved), range);
        let made = connect(&a, &b, &tol(k)).unwrap();
        check_rulings(&made, &a, &b);
        assert_eq!(made.faces, vec![Piece::Rulings]);
        assert!(made.rulings.iter().any(|(s, t)| (s - t).abs() > 0.05));
        let along = shift.normalized().unwrap();
        for &(s, t) in &made.rulings {
            let d = (at(&b, t).0 - at(&a, s).0).normalized().unwrap();
            assert!(d.cross(along).length() < 1e-6, "a ruling is off the shift");
        }
        assert!(made.curvature * k * k < 1e-4, "K {}", made.curvature);
        let dev = develop(&made.body, &[FaceId(0)], &tol(k)).unwrap();
        let e = dev.error();
        assert!(e.area < 1e-6 && e.length < 1e-6, "{e:?}");
    }
}

#[test]
fn coaxial_arcs_give_a_cone_and_rails_drawn_the_other_way_are_turned() {
    for k in SCALES {
        let a = arc(Vec3::ZERO, 2.0 * k, 0.2, 2.0);
        let b = arc(Vec3::new(0.0, 0.0, 1.5 * k), 1.0 * k, 0.2, 2.0);
        let made = connect(&a, &b, &tol(k)).unwrap();
        check_rulings(&made, &a, &b);
        assert_eq!(made.faces, vec![Piece::Rulings]);
        assert!(!made.reversed_b);
        assert!(made.deviation <= 0.5e-6 * k, "deviation {}", made.deviation);
        assert!(made.curvature * k * k < 1e-6, "K {}", made.curvature);
        for &(s, t) in &made.rulings {
            assert!((s - t).abs() < 1e-7, "a coaxial ruling at ({s}, {t})");
        }
        let turned = connect(&a, &b.reversed(), &tol(k)).unwrap();
        assert_eq!(turned.faces, vec![Piece::Rulings]);
        assert!(turned.reversed_b);
        assert_eq!(turned.rulings.len(), made.rulings.len());
    }
}

#[test]
fn lines_in_one_plane_give_the_plane_and_skew_lines_nothing() {
    let line = |a: Vec3, b: Vec3| {
        rail(
            Curve3::Line(Line::through(a, b).unwrap()),
            (0.0, a.distance(b)),
        )
    };
    let a = line(Vec3::ZERO, Vec3::new(4.0, 0.0, 0.0));
    let b = line(Vec3::new(0.0, 2.0, 0.0), Vec3::new(3.0, 2.0, 0.0));
    let made = connect(&a, &b, &tol(1.0)).unwrap();
    assert!(made.planar);
    assert!(matches!(made.body.faces[0].surface, Surface::Plane(_)));
    let area = craft3d_topo::mass_properties(&made.body).unwrap().area;
    assert!((area - 7.0).abs() < 1e-12, "{area}");
    let skew = line(Vec3::new(0.0, 2.0, 1.0), Vec3::new(0.0, 2.0, 5.0));
    assert!(matches!(
        connect(&a, &skew, &tol(1.0)),
        Err(KernelError::Refused { .. })
    ));
}

/// Bands of a cylinder or cone between circles at the heights `z`, in
/// one sheet, each sharing its circles with the next.
fn bands(surface: &dyn Fn(f64) -> f64, z: &[f64], cone: Option<f64>) -> Body {
    let mut body = Body::new(BodyKind::Sheet);
    let mut circles = Vec::new();
    for &h in z {
        let r = surface(h);
        let frame = Frame::new(Vec3::new(0.0, 0.0, h), Vec3::Z, Vec3::X).unwrap();
        let v = body.add_vertex(Vec3::new(r, 0.0, h));
        circles.push(body.add_edge(
            Curve3::Circle(Circle::new(frame, r).unwrap()),
            (0.0, TAU),
            v,
            v,
        ));
    }
    let r0 = surface(0.0);
    let face_surface = match cone {
        Some(half) => Surface::Cone(Cone::new(Frame::WORLD, r0, half).unwrap()),
        None => Surface::Cylinder(Cylinder::new(Frame::WORLD, r0).unwrap()),
    };
    let mut faces = Vec::new();
    for w in circles.windows(2) {
        let lo = body.add_loop(vec![Coedge {
            edge: w[0],
            reversed: false,
        }]);
        let hi = body.add_loop(vec![Coedge {
            edge: w[1],
            reversed: true,
        }]);
        faces.push(body.add_face(face_surface.clone(), false, vec![lo, hi]));
    }
    body.add_shell(faces);
    body
}

#[test]
fn cylinders_and_cone_frustums_unroll_to_their_closed_forms() {
    for k in SCALES {
        // A cylinder of radius r and height h: the rectangle 2πr × h.
        let (r, h) = (2.0 * k, 3.0 * k);
        let can = primitives::cylinder(&Frame::WORLD, r, h).unwrap();
        let dev = develop(&can, &[FaceId(0)], &tol(k)).unwrap();
        let flat = dev.flat_body(&Frame::WORLD, &tol(k)).unwrap();
        let area = craft3d_topo::mass_properties(&flat.body).unwrap().area;
        assert!((area - TAU * r * h).abs() <= 1e-6 * TAU * r * h, "{area}");
        // A frustum from radius R to r over h: the annular sector of the
        // slant radii ρ₂ = R s / (R − r), ρ₁ = ρ₂ − s, angle 2πR / ρ₂.
        let (big, small, height) = (3.0 * k, 1.0 * k, 4.0 * k);
        let half = -((big - small) / height).atan();
        let radius = |z: f64| big + z * half.tan();
        let body = bands(&radius, &[0.0, height], Some(half));
        let dev = develop(&body, &[FaceId(0)], &tol(k)).unwrap();
        let slant = (big - small).hypot(height);
        let rho2 = big / (big - small) * slant;
        let rho1 = rho2 - slant;
        let expected = 0.5 * (TAU * big / rho2) * (rho2 * rho2 - rho1 * rho1);
        let flat = dev.flat_body(&Frame::WORLD, &tol(k)).unwrap();
        let area = craft3d_topo::mass_properties(&flat.body).unwrap().area;
        assert!(
            (area - expected).abs() <= 1e-6 * expected,
            "{area} against {expected}"
        );
        for c in dev.outline() {
            assert!((c.length_flat - c.length_3d).abs() <= 1e-6 * c.length_3d.max(k));
        }
        // A point to the pattern and back.
        let p = body.faces[0].surface.eval(1.0, 1.5 * k);
        let (_, back) = dev.to_3d(dev.to_flat(FaceId(0), p).unwrap()).unwrap();
        assert!(back.distance(p) <= 1e-6 * k, "{back:?} against {p:?}");
    }
}

#[test]
fn two_bands_sharing_a_circle_unroll_as_one_piece() {
    for k in SCALES {
        let r = 2.0 * k;
        let body = bands(&|_| r, &[0.0, 1.0 * k, 3.0 * k], None);
        let dev = develop(&body, &[FaceId(0), FaceId(1)], &tol(k)).unwrap();
        let flat = dev.flat_body(&Frame::WORLD, &tol(k)).unwrap();
        let area = craft3d_topo::mass_properties(&flat.body).unwrap().area;
        let expected = TAU * r * 3.0 * k;
        assert!(
            (area - expected).abs() <= 1e-6 * expected,
            "{area} against {expected}"
        );
        assert_eq!(flat.body.faces.len(), 1);
    }
}

#[test]
fn a_cylinder_face_with_a_seam_is_cut_along_it() {
    let (r, h) = (2.0, 3.0);
    let mut body = Body::new(BodyKind::Sheet);
    let (v0, v1) = (
        body.add_vertex(Vec3::new(r, 0.0, 0.0)),
        body.add_vertex(Vec3::new(r, 0.0, h)),
    );
    let low = Frame::WORLD;
    let high = Frame::new(Vec3::new(0.0, 0.0, h), Vec3::Z, Vec3::X).unwrap();
    let c0 = body.add_edge(
        Curve3::Circle(Circle::new(low, r).unwrap()),
        (0.0, TAU),
        v0,
        v0,
    );
    let c1 = body.add_edge(
        Curve3::Circle(Circle::new(high, r).unwrap()),
        (0.0, TAU),
        v1,
        v1,
    );
    let seam = body.add_edge(
        Curve3::Line(Line::through(Vec3::new(r, 0.0, 0.0), Vec3::new(r, 0.0, h)).unwrap()),
        (0.0, h),
        v0,
        v1,
    );
    let coedge = |edge, reversed| Coedge { edge, reversed };
    let lp = body.add_loop(vec![
        coedge(c0, false),
        coedge(seam, false),
        coedge(c1, true),
        coedge(seam, true),
    ]);
    let face = body.add_face(
        Surface::Cylinder(Cylinder::new(Frame::WORLD, r).unwrap()),
        false,
        vec![lp],
    );
    body.add_shell(vec![face]);
    let dev = develop(&body, &[face], &tol(1.0)).unwrap();
    let flat = dev.flat_body(&Frame::WORLD, &tol(1.0)).unwrap();
    let area = craft3d_topo::mass_properties(&flat.body).unwrap().area;
    assert!((area - TAU * r * h).abs() <= 1e-6 * TAU * r * h, "{area}");
    assert_eq!(
        dev.outline().len(),
        4,
        "both sides of the seam bound the pattern"
    );
}

#[test]
fn a_sphere_is_refused_with_its_curvature() {
    let ball = primitives::sphere(&Frame::WORLD, 2.0).unwrap();
    let error = develop(&ball, &[FaceId(0)], &tol(1.0)).unwrap_err();
    assert!(error.to_string().contains("not developable"), "{error}");
}

/// A generalised cylinder whose cross-section inflects (the sea kayak's
/// panel, 2026-10-27): `A(u) = k (100 x, 0, 20 x³)` with `x = 2u − 1`, an
/// inflection at `u = ½`, and `B(u) = A(u) + w(u) n` with `w = k (40 +
/// 20 x²)` and `n` fixed, both exact cubics, `B` reparametrized when
/// `skew`. The developable between them is the cylinder of the rulings
/// along `n`, whose area is `∫ w |A' × n| du`; at the inflection the
/// condition's zero set crosses another branch (its gradient vanishes on
/// the ruling there), which used to be refused as rulings that cross.
fn inflected_cylinder(k: f64, skew: bool) -> (Rail, Rail, Vec3, f64) {
    let n = Vec3::new(0.3, 1.0, 0.8).normalized().unwrap();
    // Bézier control values of x, x³ and w over u in [0, 1].
    let x = [-1.0, -1.0 / 3.0, 1.0 / 3.0, 1.0];
    let x3 = [-1.0, 1.0, -1.0, 1.0];
    let w = [60.0, 40.0 - 20.0 / 3.0, 40.0 - 20.0 / 3.0, 60.0];
    let knots = vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0];
    let a_ctrl: Vec<Vec3> = (0..4)
        .map(|i| Vec3::new(100.0 * x[i], 0.0, 20.0 * x3[i]) * k)
        .collect();
    let b_ctrl: Vec<Vec3> = (0..4).map(|i| a_ctrl[i] + n * (w[i] * k)).collect();
    let a = NurbsCurve::polynomial(3, knots.clone(), a_ctrl).unwrap();
    let b = NurbsCurve::polynomial(3, knots, b_ctrl).unwrap();
    let b = if skew {
        reparametrized(&b, 0.5, 1.6).unwrap()
    } else {
        b
    };
    let (ra, rb) = (a.domain(), b.domain());
    // ∫ w |A' × n| du by Simpson's rule (the integrand is smooth).
    let steps = 4000;
    let integrand = |u: f64| {
        let x = 2.0 * u - 1.0;
        let da = Vec3::new(200.0, 0.0, 120.0 * x * x) * k;
        (40.0 + 20.0 * x * x) * k * da.cross(n).length()
    };
    let h = 1.0 / steps as f64;
    let mut area = integrand(0.0) + integrand(1.0);
    for j in 1..steps {
        area += integrand(j as f64 * h) * if j % 2 == 1 { 4.0 } else { 2.0 };
    }
    area *= h / 3.0;
    (
        rail(Curve3::Nurbs(a), ra),
        rail(Curve3::Nurbs(b), rb),
        n,
        area,
    )
}

#[test]
fn a_generalised_cylinder_whose_section_inflects_is_joined_through_the_inflection() {
    for skew in [false, true] {
        for k in SCALES {
            let (a, b, n, area) = inflected_cylinder(k, skew);
            let made = connect(&a, &b, &tol(k)).unwrap_or_else(|e| panic!("{k} {skew}: {e}"));
            assert_eq!(made.faces, vec![Piece::Rulings]);
            assert!(
                made.rulings
                    .windows(2)
                    .all(|w| w[1].0 >= w[0].0 && w[1].1 >= w[0].1),
                "rulings are monotone"
            );
            // Through the inflection: rulings on both sides of it.
            assert!(made.rulings.iter().any(|(s, _)| *s < 0.45));
            assert!(made.rulings.iter().any(|(s, _)| *s > 0.55));
            for &(s, t) in &made.rulings {
                let d = (at(&b, t).0 - at(&a, s).0).normalized().unwrap();
                assert!(
                    d.cross(n).length() < 1e-6,
                    "{k} {skew}: a ruling at ({s}, {t}) is off the cylinder's direction by {}",
                    d.cross(n).length()
                );
            }
            assert!(made.curvature * k * k < 1e-6, "K {}", made.curvature);
            let measured = craft3d_topo::mass_properties(&made.body).unwrap().area;
            assert!(
                (measured - area).abs() <= 1e-6 * area,
                "{k} {skew}: area {measured}, expected {area}"
            );
            let dev = develop(&made.body, &[FaceId(0)], &tol(k)).unwrap();
            let e = dev.error();
            assert!(e.area < 1e-6 && e.length < 1e-6, "{e:?}");
        }
    }
}

/// The sea kayak's first finding as it was drawn (2026-10-27): `A(x) = k
/// (100 x, 0, 20 x³)` inflects at `x = 0`, and `B(x) = k (100 x, 20 + 10
/// x², 20 (x/1.1)³ + (40 + 20 (x/1.1)²) sin 60°)` (a profile scaled by 1.1
/// against its plan) does not inflect with it, both exact cubics. The two
/// branches of the condition that cross at an inflection pass each other
/// here, so the rulings from the start turn back near `x = 0`: the sheet
/// is closed there by a fan with its apex on A, from the ruling where they
/// turn to the next ruling that goes on forward. Held to what makes it a
/// smooth developable whatever the fan: the condition at every ruling, no
/// curvature, the faces tangent across every edge they share, laid flat
/// as one piece with lengths and area kept; at three scales.
fn passing_branches(k: f64) -> (Rail, Rail) {
    let x = [-1.0, -1.0 / 3.0, 1.0 / 3.0, 1.0];
    let x2 = [1.0, -1.0 / 3.0, -1.0 / 3.0, 1.0];
    let x3 = [-1.0, 1.0, -1.0, 1.0];
    let sin60 = 60f64.to_radians().sin();
    let (c3, c2, c0) = (20.0 / 1.331, 20.0 * sin60 / 1.21, 40.0 * sin60);
    let knots = vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0];
    let a: Vec<Vec3> = (0..4)
        .map(|i| Vec3::new(100.0 * x[i], 0.0, 20.0 * x3[i]) * k)
        .collect();
    let b: Vec<Vec3> = (0..4)
        .map(|i| {
            Vec3::new(
                100.0 * x[i],
                20.0 + 10.0 * x2[i],
                c3 * x3[i] + c0 + c2 * x2[i],
            ) * k
        })
        .collect();
    let a = NurbsCurve::polynomial(3, knots.clone(), a).unwrap();
    let b = NurbsCurve::polynomial(3, knots, b).unwrap();
    let (ra, rb) = (a.domain(), b.domain());
    (rail(Curve3::Nurbs(a), ra), rail(Curve3::Nurbs(b), rb))
}

#[test]
fn branches_that_pass_each_other_at_an_inflection_are_bridged_by_a_fan() {
    for k in SCALES {
        let (a, b) = passing_branches(k);
        let made = connect(&a, &b, &tol(k)).unwrap_or_else(|e| panic!("{k}: {e}"));
        assert!(
            made.faces.iter().any(|p| matches!(p, Piece::Inner(_))),
            "{k}: a fan inside the sheet: {:?}",
            made.faces
        );
        check_rulings(&made, &a, &b);
        assert!(made.curvature * k * k < 1e-6, "K {}", made.curvature);
        // Tangent across every edge two faces share, at its middle.
        let body = &made.body;
        let uses = craft3d_topo::edge_uses(body);
        let mut shared = 0;
        for (edge, faces) in uses.iter().enumerate() {
            let [first, second] = faces.as_slice() else {
                continue;
            };
            shared += 1;
            let e = &body.edges[edge];
            let mid = e.curve.eval(0.5 * (e.range.0 + e.range.1));
            let normal = |use_: &craft3d_topo::EdgeUse| {
                let face = body.face(use_.face).unwrap();
                let (u, v) = face.surface.project(mid).unwrap();
                let n = face.surface.normal(u, v).unwrap();
                if face.reversed { -n } else { n }
            };
            let (n0, n1) = (normal(first), normal(second));
            assert!(
                n0.cross(n1).length() < 1e-6 && n0.dot(n1) > 0.0,
                "{k}: faces {:?} and {:?} meet at an angle of {} rad",
                first.face,
                second.face,
                n0.cross(n1).length()
            );
        }
        assert!(shared >= 2, "{k}: {shared} shared edges");
        let faces: Vec<FaceId> = (0..made.body.faces.len() as u32).map(FaceId).collect();
        let dev = develop(&made.body, &faces, &tol(k)).unwrap();
        let e = dev.error();
        assert!(e.area < 1e-6 && e.length < 1e-6, "{e:?}");
    }
}
