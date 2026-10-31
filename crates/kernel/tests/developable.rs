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
use craft3d_geom::{Circle, Cone, Curve3, Cylinder, Line, NurbsCurve, Sphere, Surface};
use craft3d_kernel::developable::{Connected, Mark, Piece, Rail, RailPiece, connect};
use craft3d_kernel::unfold::{DEVELOPABLE_SHARE, develop, face_curvature};
use craft3d_kernel::{KernelError, primitives};
use craft3d_math::Portable;
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
        assert_flat_within_tolerance(&made, k);
        // The spline starts at about 0.3 rad, the arc at 0.2, and ends at
        // 2.0 past the spline's 1.9: fans from the spline's ends, to the
        // arc's points at the spline's ends' angles.
        assert_eq!(
            made.faces,
            vec![Piece::StartFan, Piece::Rulings, Piece::EndFan]
        );
        // Built the given way, it stays the given way (its shape and its
        // faces' names), whatever the other way would give.
        assert!(!made.reversed_b);
        let (first, last) = (made.rulings[0], *made.rulings.last().unwrap());
        assert_eq!(first.0, 0.0);
        assert_eq!(last.0, 1.0);
        let angle_of = |p: Vec3| p.y.patan2(p.x);
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
        let wave = |x: f64| Vec3::new(x * k, (1.3 * x).psin() * k, 0.2 * x * x * k);
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
        assert_flat_within_tolerance(&made, k);
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
        assert_flat_within_tolerance(&made, k);
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
        let half = -((big - small) / height).patan();
        let radius = |z: f64| big + z * half.ptan();
        let body = bands(&radius, &[0.0, height], Some(half));
        let dev = develop(&body, &[FaceId(0)], &tol(k)).unwrap();
        let slant = (big - small).phypot(height);
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

/// A thin band of a sphere of radius `r` about its equator, between the
/// latitudes `±half` (radians), as one sheet face.
fn sphere_band(r: f64, half: f64) -> Body {
    let mut body = Body::new(BodyKind::Sheet);
    let mut circles = Vec::new();
    for lat in [-half, half] {
        let h = r * lat.psin();
        let frame = Frame::new(Vec3::new(0.0, 0.0, h), Vec3::Z, Vec3::X).unwrap();
        let rho = r * lat.pcos();
        let v = body.add_vertex(Vec3::new(rho, 0.0, h));
        circles.push(body.add_edge(
            Curve3::Circle(Circle::new(frame, rho).unwrap()),
            (0.0, TAU),
            v,
            v,
        ));
    }
    let lo = body.add_loop(vec![Coedge {
        edge: circles[0],
        reversed: false,
    }]);
    let hi = body.add_loop(vec![Coedge {
        edge: circles[1],
        reversed: true,
    }]);
    let surface = Surface::Sphere(Sphere::new(Frame::WORLD, r).unwrap());
    let face = body.add_face(surface, false, vec![lo, hi]);
    body.add_shell(vec![face]);
    body
}

/// The stretch gate is an estimate, not a bound (review of 2026-10-27):
/// a band of a sphere of radius `R` about its equator, `w = 0.0006 R`
/// wide and `2πR` long, gets about 0.3 of the tolerance from it and
/// passes. It is still not developable: its rims are circles of
/// geodesic curvature `tan(w / 2R) / R`, which a band of constant width
/// laid flat round a straight middle line cannot give them, and the
/// later checks refuse it (the rims developed along themselves against
/// the map, or the layouts that do not converge). At three scales.
#[test]
fn a_thin_sphere_band_that_passes_the_stretch_gate_is_still_refused() {
    for k in SCALES {
        let (r, half) = (k, 0.0003);
        let band = sphere_band(r, half);
        let measured = craft3d_kernel::unfold::face_curvature(&band, FaceId(0)).unwrap();
        assert!(
            measured.stretch <= 0.5 * tol(k).linear,
            "{k}: the gate itself refuses it ({}), which this test is not about",
            measured.stretch
        );
        let error = develop(&band, &[FaceId(0)], &tol(k))
            .err()
            .unwrap_or_else(|| panic!("{k}: a sphere band laid flat"));
        let why = error.to_string();
        assert!(
            why.contains("departs from the pattern's map")
                || why.contains("does not lie flat")
                || why.contains("in the pattern and"),
            "{k}: {why}"
        );
    }
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
            assert_flat_within_tolerance(&made, k);
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
    passing(k, 1.1, 0.0, 1.0)
}

/// [`passing_branches`] with B's profile scaled by `sigma` against its
/// plan, A bent out of its plane by `alpha x²` (`A(x) = k (100 x, alpha
/// x², 20 x³)`), and both rails run over the first `cut` of their range
/// (from `x = −1`).
fn passing(k: f64, sigma: f64, alpha: f64, cut: f64) -> (Rail, Rail) {
    let x = [-1.0, -1.0 / 3.0, 1.0 / 3.0, 1.0];
    let x2 = [1.0, -1.0 / 3.0, -1.0 / 3.0, 1.0];
    let x3 = [-1.0, 1.0, -1.0, 1.0];
    let sin60 = 60f64.to_radians().psin();
    let (c3, c2, c0) = (
        20.0 / sigma.ppowi(3),
        20.0 * sin60 / sigma.ppowi(2),
        40.0 * sin60,
    );
    let knots = vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0];
    let a: Vec<Vec3> = (0..4)
        .map(|i| Vec3::new(100.0 * x[i], alpha * x2[i], 20.0 * x3[i]) * k)
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
    let part = |(lo, hi): (f64, f64)| (lo, lo + cut * (hi - lo));
    let (ra, rb) = (part(a.domain()), part(b.domain()));
    (rail(Curve3::Nurbs(a), ra), rail(Curve3::Nurbs(b), rb))
}

/// Faces that share an edge meet tangent there (at its middle, within
/// `1e-6` rad); how many edges they share.
fn assert_tangent_across_shared_edges(made: &Connected, k: f64) -> usize {
    let body = &made.body;
    let mut shared = 0;
    for (edge, faces) in craft3d_topo::edge_uses(body).iter().enumerate() {
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
    shared
}

/// Branches of the condition that pass each other closer than a step of
/// the walk (review of 2026-10-29: the step control at a saddle was pinned
/// only by the kayak's rails). [`passing_branches`] with B's profile
/// scaled by 1.002: the two arcs at A's inflection pass about 0.0012 of
/// the square apart, under the walk's largest step of 1/128, while the
/// model's value at their saddle, about 2e-6 over a ruling of 40 (80
/// tolerances), is far more than `cross` steps over. The rails run over
/// the first 80 % of their range, so the fold lies in the sheet's second
/// half. The walk keeps to its branch, meets the fold and closes it with
/// the fan from A's inflection (the origin): every ruling on the zero
/// set, the faces tangent across their edges and flat, at three scales.
/// With the saddle guards of the walk switched off (`saddle_ahead` and
/// `split_between`) the corrector lands on the other arc and the
/// connection is refused, "the rulings between two found ones cannot be
/// followed" (measured on the Strix, 2026-10-29); with `saddle_ahead`
/// alone off, `split_between` still catches the step.
#[test]
fn branches_that_pass_closer_than_a_step_are_followed_into_the_fold() {
    for k in SCALES {
        let (a, b) = passing(k, 1.002, 0.0, 0.8);
        let made = connect(&a, &b, &tol(k)).unwrap_or_else(|e| panic!("{k}: {e}"));
        assert_eq!(
            made.faces,
            vec![
                Piece::StartFan,
                Piece::Rulings,
                Piece::Fan(Mark::FoldFromEnd(0)),
                Piece::Last,
                Piece::EndFan,
            ],
            "{k}"
        );
        check_rulings(&made, &a, &b);
        assert_flat_within_tolerance(&made, k);
        assert!(made.deviation <= 0.5e-6 * k, "{k}: {}", made.deviation);
        assert!(
            made.unresolved <= 1e-2 * 1e-6 * k,
            "{k}: {}",
            made.unresolved
        );
        assert!(assert_tangent_across_shared_edges(&made, k) >= 4, "{k}");
        let surface = &made.body.faces[2].surface;
        for u in [0.0, 0.3, 0.7, 1.0] {
            let apex = surface.eval(u, 0.0);
            assert!(apex.length() <= 1e-6 * k, "{k}: the fan's apex at {apex:?}");
        }
    }
}

/// A fan across a fold that lands on the other branch where that branch
/// already turns back, closed by a second fan right after it (review of
/// 2026-10-29: the chain of two fans was pinned only by the kayak's
/// stern). [`passing_branches`] with B's profile scaled by 1.01 and A bent
/// out of its plane by `2 x²`, over the first 80 % of their range: the two
/// arcs pass each other left and right, and the turning point of the arc
/// beyond lies a little higher along B than the fold of the walk's own, so
/// the fan across that fold (apex on B) lands on the arc beyond while it
/// still turns back in A, and a fan along (apex on A, from the first fan's
/// end) closes that. The two fans share their ruling, both on the zero
/// set, so the sheet is smooth across it; every ruling on the zero set,
/// the faces flat and tangent across every edge, its unfold refused at
/// the strip beside a fan (the owed layout of a standing corner); at
/// three scales.
#[test]
fn a_fan_that_lands_where_the_other_branch_turns_back_is_followed_by_a_second() {
    for k in SCALES {
        let (a, b) = passing(k, 1.01, 2.0, 0.8);
        let made = connect(&a, &b, &tol(k)).unwrap_or_else(|e| panic!("{k}: {e}"));
        assert_eq!(
            made.faces,
            vec![
                Piece::StartFan,
                Piece::Rulings,
                Piece::Fan(Mark::FoldFromEnd(1)),
                Piece::Fan(Mark::FoldFromEnd(0)),
                Piece::Last,
                Piece::EndFan,
            ],
            "{k}"
        );
        check_rulings(&made, &a, &b);
        assert_flat_within_tolerance(&made, k);
        assert!(made.deviation <= 0.5e-6 * k, "{k}: {}", made.deviation);
        assert!(
            made.unresolved <= 1e-2 * 1e-6 * k,
            "{k}: {}",
            made.unresolved
        );
        assert!(assert_tangent_across_shared_edges(&made, k) >= 5, "{k}");
        // The first fan's apex on B (its row at v = 1 stands), the second's
        // on A (v = 0), and the first's last ruling the second's first.
        let (first, second) = (&made.body.faces[2].surface, &made.body.faces[3].surface);
        let close = |p: Vec3, q: Vec3| p.distance(q) <= 1e-6 * k;
        assert!(close(first.eval(0.0, 1.0), first.eval(1.0, 1.0)), "{k}");
        assert!(!close(first.eval(0.0, 0.0), first.eval(1.0, 0.0)), "{k}");
        assert!(close(second.eval(0.0, 0.0), second.eval(1.0, 0.0)), "{k}");
        assert!(!close(second.eval(0.0, 1.0), second.eval(1.0, 1.0)), "{k}");
        for v in [0.0, 0.5, 1.0] {
            assert!(close(first.eval(1.0, v), second.eval(0.0, v)), "{k}: {v}");
        }
        // The unfold is refused at the strip beside a fold's fan (face 1
        // before the first fan, face 4 after the second), the owed layout
        // of a face whose parametrisation stands still at a corner
        // (docs/design/developable.md, Open), never a wrong pattern. Since
        // the rows leave their end rulings along the rails (2026-10-29)
        // the row of B stands still where the zero set turned, as the true
        // developable's does, at every scale; before, the fit's end
        // tangents were off by its error, which let it unfold at 1e-3 and
        // 1. Pinned as a refusal at every scale (final review of
        // 2026-10-29): the unfold of the standing-corner layout, when it
        // lands, flips this deliberately to a laid-flat piece whose area
        // and every edge's length are kept within 1e-6 (as
        // `a_fan_is_laid_flat_about_its_apex` checks).
        let faces: Vec<FaceId> = (0..made.body.faces.len() as u32).map(FaceId).collect();
        let refused = develop(&made.body, &faces, &tol(k))
            .map(|dev| dev.error())
            .expect_err("the strip beside a fold's fan is refused (owed layout)");
        let why = refused.to_string();
        assert!(
            why.contains("converge") && (why.contains("face 1 ") || why.contains("face 4 ")),
            "{k}: {refused}"
        );
    }
}

/// A fold at the middle of the sheet keeps its name under edits that do
/// not move it (review of 2026-10-29: a fold was counted from the start
/// below the sheet's middle and from the end at or above it, so on rails
/// whose fold lies at the middle the rounding of an edit elsewhere turned
/// its name, rule 4). [`passing`] with B's profile scaled by 1.01 over the
/// whole range, whose fold the walk meets at A's inflection, the middle
/// of A: the faces and their names are the same at three scales (each a
/// different rounding of the same rails) and with A bent out of its
/// plane by a millionth (an edit away from the fold).
#[test]
fn a_fold_at_the_middle_of_the_sheet_keeps_its_name() {
    let mut seen: Vec<(f64, f64, Vec<Piece>)> = Vec::new();
    for k in SCALES {
        for alpha in [0.0, 1e-6] {
            let (a, b) = passing(k, 1.01, alpha, 1.0);
            let made = connect(&a, &b, &tol(k)).unwrap_or_else(|e| panic!("{k} {alpha}: {e}"));
            eprintln!("{k} {alpha}: faces {:?}", made.faces);
            check_rulings(&made, &a, &b);
            seen.push((k, alpha, made.faces));
        }
    }
    let (_, _, first) = &seen[0];
    assert!(
        first
            .iter()
            .any(|p| matches!(p, Piece::Fan(Mark::FoldFromStart(_) | Mark::FoldFromEnd(_)))),
        "a fold: {first:?}"
    );
    for (k, alpha, faces) in &seen {
        assert_eq!(faces, first, "{k} {alpha}");
    }
}

#[test]
fn branches_that_pass_each_other_at_an_inflection_are_bridged_by_a_fan() {
    for k in SCALES {
        let (a, b) = passing_branches(k);
        let made = connect(&a, &b, &tol(k)).unwrap_or_else(|e| panic!("{k}: {e}"));
        assert!(
            made.faces.iter().any(|p| matches!(p, Piece::Fan(_))),
            "{k}: a fan inside the sheet: {:?}",
            made.faces
        );
        check_rulings(&made, &a, &b);
        assert_flat_within_tolerance(&made, k);
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
        // The fan's apex is where the branches pass: here the inflection
        // of A at the origin.
        let fan = made
            .faces
            .iter()
            .position(|p| matches!(p, Piece::Fan(_)))
            .unwrap();
        let surface = &made.body.faces[fan].surface;
        for u in [0.0, 0.3, 0.7, 1.0] {
            let apex = surface.eval(u, 0.0);
            assert!(apex.length() <= 1e-6 * k, "{k}: the fan's apex at {apex:?}");
        }
        // The faces beside the fan have a corner where B stands still
        // (the ruling at the fold): measured since 2026-10-27 (topo::mass
        // grades the boundary's panels at such a corner), where they were
        // refused, "cannot be measured". The fan is a cone with its apex at
        // the origin: its area is `½ ∫ |B × dB|` over its stretch of B,
        // summed here over 1 << 14 chords of the fan's own far row.
        let areas = craft3d_topo::face_areas(&made.body);
        for (i, a) in areas.iter().enumerate() {
            assert!(a.is_ok(), "{k}: face {i}: {a:?}");
        }
        const CHORDS: usize = 1 << 14;
        let row: Vec<Vec3> = (0..=CHORDS)
            .map(|i| surface.eval(i as f64 / CHORDS as f64, 1.0))
            .collect();
        let (mut area, mut angle) = (0.0, 0.0);
        for w in row.windows(2) {
            area += 0.5 * w[0].cross(w[1]).length();
            angle += w[0]
                .normalized()
                .unwrap()
                .cross(w[1].normalized().unwrap())
                .length()
                .pasin();
        }
        let measured = *areas[fan].as_ref().unwrap();
        assert!(
            (measured - area).abs() <= 1e-6 * area,
            "{k}: the fan's area {measured} for {area}"
        );
        // Laid flat, the fan opens by the length of its stretch's image on
        // the unit sphere, `∫ |d(B / |B|)|` (an isometry keeps the angle at
        // the apex). Owed (docs/design/developable.md, Open): the layout of
        // a strip beside the fan does not converge at its standing corner;
        // until then the unfold is refused with that reason, never a wrong
        // pattern.
        let faces: Vec<FaceId> = (0..made.body.faces.len() as u32).map(FaceId).collect();
        let dev = match develop(&made.body, &faces, &tol(k)) {
            Ok(dev) => dev,
            Err(e) => {
                assert!(e.to_string().contains("converge"), "{k}: {e}");
                continue;
            }
        };
        let e = dev.error();
        assert!(e.area < 1e-6 && e.length < 1e-6, "{k}: {e:?}");
        let flat = |p: Vec3| dev.to_flat(FaceId(fan as u32), p).unwrap();
        let (apex, first, last) = (
            flat(surface.eval(0.5, 0.0)),
            flat(row[0]),
            flat(row[CHORDS]),
        );
        let (a, b) = (
            [first[0] - apex[0], first[1] - apex[1]],
            [last[0] - apex[0], last[1] - apex[1]],
        );
        let opened = (a[0] * b[1] - a[1] * b[0])
            .abs()
            .patan2(a[0] * b[0] + a[1] * b[1]);
        assert!(
            (opened - angle).abs() <= 1e-6,
            "{k}: the fan opens by {opened} in the pattern, {angle} on the part"
        );
    }
}

/// A plate on a developable whose rulings are of very different lengths
/// at its two ends (the sea kayak's panel, 2026-10-27: the plate was
/// refused, "the joint reaches past the faces' surfaces continued", as the
/// surface was continued across its rulings by as far as the longest one
/// moves, and so reached a corner by a short one by a nineteenth of that).
/// `A(x) = k (100 x, 0, 20 x²)`, `B = A + w n` with `w = k (10 + 180
/// ((x + 1)/2)²)`, `n` fixed: a generalised cylinder over the profile `C`
/// (A seen along `n`). With square rims (swept by the normal) the plate
/// of thickness `T` behind the face is `T Area ∓ T²/2 ∫ κ dA` exactly,
/// `Area = ∫ w |C'| dx` and `∫ κ dA = ∫ w |C' × C''| / |C'|² dx`, minus
/// when the plate lies towards the profile's centres of curvature.
#[test]
fn a_plate_on_a_developable_with_short_and_long_rulings_has_its_closed_form_volume() {
    use craft3d_kernel::plate::{Side, plate};
    use std::collections::BTreeMap;
    for k in SCALES {
        let n = Vec3::new(0.3, 1.0, 0.8).normalized().unwrap();
        // Bézier control values over u in [0, 1], x = 2u − 1.
        let x = [-1.0, -1.0 / 3.0, 1.0 / 3.0, 1.0];
        let x2 = [1.0, -1.0 / 3.0, -1.0 / 3.0, 1.0];
        // ((x + 1)/2)² = u²: control values 0, 0, 1/3, 1.
        let u2 = [0.0, 0.0, 1.0 / 3.0, 1.0];
        let knots = vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0];
        let a_ctrl: Vec<Vec3> = (0..4)
            .map(|i| Vec3::new(100.0 * x[i], 0.0, 20.0 * x2[i]) * k)
            .collect();
        let b_ctrl: Vec<Vec3> = (0..4)
            .map(|i| a_ctrl[i] + n * ((10.0 + 180.0 * u2[i]) * k))
            .collect();
        let a = NurbsCurve::polynomial(3, knots.clone(), a_ctrl).unwrap();
        let b = NurbsCurve::polynomial(3, knots, b_ctrl).unwrap();
        let (ra, rb) = (a.domain(), b.domain());
        let (a, b) = (rail(Curve3::Nurbs(a), ra), rail(Curve3::Nurbs(b), rb));
        let made = connect(&a, &b, &tol(k)).unwrap_or_else(|e| panic!("{k}: {e}"));
        assert_eq!(made.faces, vec![Piece::Rulings]);
        let t = 4.0 * k;
        let plated = plate(
            &made.body,
            &[FaceId(0)],
            t,
            Side::In,
            &BTreeMap::new(),
            &tol(k),
        )
        .unwrap_or_else(|e| panic!("plate at scale {k}: {e}"));
        assert_eq!(craft3d_topo::check(&plated.body, &tol(k)), []);
        // The closed form, by Simpson's rule over x in [−1, 1].
        let flat = |v: Vec3| v - n * v.dot(n);
        let parts = |x: f64| {
            let w = (10.0 + 180.0 * ((x + 1.0) / 2.0).ppowi(2)) * k;
            let c1 = flat(Vec3::new(100.0, 0.0, 40.0 * x) * k);
            let c2 = flat(Vec3::new(0.0, 0.0, 40.0) * k);
            (
                w * c1.length(),
                w * c1.cross(c2).length() / c1.length_squared(),
            )
        };
        let steps = 4000;
        let h = 2.0 / steps as f64;
        let (mut area, mut bend) = (0.0, 0.0);
        for j in 0..=steps {
            let weight = if j == 0 || j == steps {
                1.0
            } else if j % 2 == 1 {
                4.0
            } else {
                2.0
            };
            let (da, db) = parts(-1.0 + j as f64 * h);
            area += weight * da;
            bend += weight * db;
        }
        area *= h / 3.0;
        bend *= h / 3.0;
        // Which way the plate lies: behind the outward normal at x = 0,
        // where the profile's centre of curvature is towards +z seen
        // across n.
        let face = made.body.face(FaceId(0)).unwrap();
        let p0 = Vec3::ZERO + n * (0.5 * 10.0 * k);
        let (u, v) = face.surface.project(p0).unwrap();
        let outward = face.surface.normal(u, v).unwrap() * if face.reversed { -1.0 } else { 1.0 };
        let centre = flat(Vec3::Z).normalized().unwrap();
        let towards = -outward.dot(centre) > 0.0;
        let expected = t * area + if towards { -1.0 } else { 1.0 } * 0.5 * t * t * bend;
        let volume = craft3d_topo::mass_properties(&plated.body)
            .unwrap()
            .volume
            .abs();
        assert!(
            (volume - expected).abs() <= 1e-6 * expected,
            "{k}: volume {volume}, expected {expected} (off by {:e})",
            (volume - expected) / expected
        );
    }
}

/// How far a ruling strays from a developable: the turn of the tangent
/// plane from its end on A to its end on B, times its length (computed
/// here from the planes themselves, not from the condition).
fn twist(a: &Rail, b: &Rail, (s, t): (f64, f64)) -> f64 {
    let ((pa, ua), (pb, ub)) = (at(a, s), at(b, t));
    let d = pb - pa;
    let (na, nb) = (
        ua.cross(d).normalized().unwrap(),
        ub.cross(d).normalized().unwrap(),
    );
    let turn = na.cross(nb).length().patan2(na.dot(nb).abs());
    turn * d.length()
}

/// The sea kayak's panels (2026-10-27): rails whose ends line up with a
/// ruling within the tolerance (edges of a boolean, fitted within it) got
/// fans at their ends, faces with edges of 1e-5 mm, which a perturbation
/// within the tolerance made or took away, and with them the names of the
/// sheet's faces and edges. Two coaxial arcs of one radius (a cylinder
/// strip, 72 times longer than wide, as a kayak's panel), the second
/// longer at both ends by `δ` along its circle, so that the rulings from
/// the first's ends miss the second's by `δ`: the strip that runs on to
/// the corners strays from a developable by about `δ h / R` there. A fan
/// within half the tolerance of that strip is not made; one beyond it is.
#[test]
fn a_fan_within_the_tolerance_of_the_strip_beside_it_is_not_made() {
    for k in SCALES {
        let t = tol(k).linear;
        // (δ, h / R, whether the ends are fans)
        let cases = [
            (0.0, 1.0 / 40.0, false),
            (0.1 * t, 1.0 / 40.0, false),
            (0.1 * t, 1.0, false),
            (10.0 * t, 1.0 / 40.0, false),
            // The same δ where the strip would turn by 10 tolerances.
            (10.0 * t, 1.0, true),
            // A millimetre at the kayak's tolerance.
            (1e6 * t, 1.0 / 40.0, true),
        ];
        for (delta, ratio, fans) in cases {
            let r = 2.0 * k;
            let h = ratio * r;
            let phi = delta / r;
            let a = arc(Vec3::ZERO, r, 0.2, 2.0);
            let b = arc(Vec3::new(0.0, 0.0, h), r, 0.2 - phi, 2.0 + phi);
            let what = format!("scale {k}, δ {delta}, h/R {ratio}");
            let made = connect(&a, &b, &tol(k)).unwrap_or_else(|e| panic!("{what}: {e}"));
            if fans {
                assert_eq!(
                    made.faces,
                    vec![Piece::StartFan, Piece::Rulings, Piece::EndFan],
                    "{what}"
                );
            } else {
                assert_eq!(made.faces, vec![Piece::Rulings], "{what}");
                assert_eq!(made.body.faces.len(), 1, "{what}");
                // The strip runs from corner to corner.
                assert_eq!(made.rulings.first(), Some(&(0.0, 0.0)), "{what}");
                assert_eq!(made.rulings.last(), Some(&(1.0, 1.0)), "{what}");
            }
            // Every ruling of the strip within half the tolerance of a
            // developable's, the corners' included, and the faces too.
            for &ruling in &made.rulings {
                let off = twist(&a, &b, ruling);
                assert!(off <= 0.5 * t, "{what}: a ruling {ruling:?} off by {off}");
            }
            assert!(made.deviation <= 0.5 * t, "{what}: {}", made.deviation);
            let faces: Vec<FaceId> = (0..made.body.faces.len() as u32).map(FaceId).collect();
            let dev =
                develop(&made.body, &faces, &tol(k)).unwrap_or_else(|e| panic!("{what}: {e}"));
            let e = dev.error();
            assert!(e.area < 1e-6 && e.length < 1e-6, "{what}: {e:?}");
        }
    }
}

/// A plate on a fan alone (rails on one cone: the fan at the start has
/// its apex at a vertex): the face has no normal at the apex, so the
/// plate is refused there by name, at once, whatever the reach past the
/// vertices (bounded: `plate.rs` › `the_reach_past_vertices_is_bounded_beside_an_apex`).
#[test]
fn a_plate_on_a_fan_to_its_apex_is_refused_by_name() {
    use craft3d_kernel::plate::{Side, plate};
    use std::collections::BTreeMap;
    for k in SCALES {
        let (a, b, _) = cone_rails(k);
        let made = connect(&a, &b, &tol(k)).unwrap();
        assert_eq!(made.faces[0], Piece::StartFan);
        let error = plate(
            &made.body,
            &[FaceId(0)],
            0.02 * k,
            Side::In,
            &BTreeMap::new(),
            &tol(k),
        )
        .err()
        .unwrap_or_else(|| panic!("{k}: a plate to an apex"));
        assert!(
            error.to_string().contains("drawn into a point"),
            "{k}: {error}"
        );
    }
}

/// The sea kayak's repro 14 (2026-10-27): a plate on a long developable
/// panel (782 control points along it) whose rulings shrink towards a
/// tip was refused, "the offset of the freeform face could not be
/// approximated": the panel's surface is continued across its rulings by
/// a box of parameters large enough for the short ruling at the tip, so
/// in the middle it reached ten rulings' lengths past the chines, and
/// the offset's fit over all of that box ran out of samples. A cone strip
/// 3000 long (at scale 1) between the generators' points at the distance
/// `r0` from the apex and `r0 + w(θ)`, `w` tapering from 190 to 3 along
/// it, as splines through 800 points: plated in the middle by `t`, its
/// volume is `A t` (Steiner's formula with `K = 0`: the mean curvature's
/// term cancels between the two sides), with the area of the developed
/// sector `A = sin α ∫ (2 r0 w + w²)/2 dθ` in closed form.
#[test]
fn a_plate_on_a_long_developable_that_tapers_to_a_tip_has_its_closed_form_volume() {
    use craft3d_kernel::plate::{Side, plate};
    use std::collections::BTreeMap;
    let alpha: f64 = 30f64.to_radians();
    let (s, c) = (alpha.psin(), alpha.pcos());
    let sweep = 2.0;
    let count = 800;
    for k in SCALES {
        let (r0, w0, w1) = (3000.0 * k, 190.0 * k, 3.0 * k);
        let g = |th: f64| Vec3::new(s * th.pcos(), s * th.psin(), c);
        let w = |th: f64| w0 + (w1 - w0) * th / sweep;
        let at = |i: usize| sweep * i as f64 / (count - 1) as f64;
        let a: Vec<Vec3> = (0..count).map(|i| g(at(i)) * r0).collect();
        let b: Vec<Vec3> = (0..count).map(|i| g(at(i)) * (r0 + w(at(i)))).collect();
        let made =
            connect(&spline(&a), &spline(&b), &tol(k)).unwrap_or_else(|e| panic!("{k}: {e}"));
        assert_eq!(made.faces, vec![Piece::Rulings], "{k}");
        let t = 4.0 * k;
        let started = std::time::Instant::now();
        let plated = plate(
            &made.body,
            &[FaceId(0)],
            t,
            Side::Mid,
            &BTreeMap::new(),
            &tol(k),
        )
        .unwrap_or_else(|e| panic!("plate at scale {k}: {e}"));
        let took = started.elapsed();
        assert_eq!(craft3d_topo::check(&plated.body, &tol(k)), []);
        // Every vertex of the plate on a skin, at ±t/2 from the exact cone
        // (signed distance `ρ cos α − z sin α` for a point near it), and
        // every point of an edge that runs along a skin too: within the
        // tolerance of the exact offset, not only of the fitted skins.
        let off = |p: Vec3| p.x.phypot(p.y) * c - p.z * s;
        let (half, limit) = (0.5 * t, tol(k).linear);
        let mut worst: f64 = 0.0;
        for edge in &plated.body.edges {
            let (t0, t1) = edge.range;
            let ends = [edge.curve.eval(t0), edge.curve.eval(t1)];
            for p in ends {
                worst = worst.max((off(p).abs() - half).abs());
            }
            if off(ends[0]) * off(ends[1]) > 0.0 {
                for i in 1..32 {
                    let p = edge.curve.eval(t0 + (t1 - t0) * f64::from(i) / 32.0);
                    worst = worst.max((off(p).abs() - half).abs());
                }
            }
        }
        assert!(
            worst <= limit,
            "{k}: a skin's point {worst:e} off the exact offset"
        );
        let area = 0.5
            * s
            * (2.0 * r0 * sweep * (w0 + w1) / 2.0 + sweep * (w0 * w0 + w0 * w1 + w1 * w1) / 3.0);
        let expected = area * t;
        let volume = craft3d_topo::mass_properties(&plated.body)
            .unwrap()
            .volume
            .abs();
        assert!(
            (volume - expected).abs() <= 1e-6 * expected,
            "{k}: volume {volume}, expected {expected} (off by {:e})",
            (volume - expected) / expected
        );
        eprintln!(
            "scale {k}: plated in {took:?}, volume off by {:e}",
            (volume - expected) / expected
        );
    }
}

/// Every face of the sheet is developable within the tolerance by the
/// unfold's own estimate (docs/design/developable.md › *Developable within
/// tolerance*): the stretch of laying it flat from its middle line, from
/// its largest |K| and its width, at most the share of the tolerance the
/// unfold's layout must converge to. A bound derived from the tolerance
/// (2026-10-28, review: `K k² < 1e-6` allowed a stretch of a few
/// millimetres on a 5 m panel).
fn assert_flat_within_tolerance(made: &Connected, k: f64) {
    let limit = DEVELOPABLE_SHARE * tol(k).linear;
    for f in 0..made.body.faces.len() {
        let c = face_curvature(&made.body, FaceId(f as u32)).unwrap();
        assert!(
            c.stretch <= limit,
            "{k}: face {f} ({:?}) stretches by {} laid flat (|K| {} at {:?}), more than {limit}",
            made.faces.get(f),
            c.stretch,
            c.largest,
            c.at
        );
    }
}

/// What the rounding of the stored control points' coordinates resolves
/// of the faces' normals at their end rulings, times the ruling's length:
/// per face end, `ε |P| L / |ΔS|`, `|P|` the largest of the control points
/// there, `ΔS` the first control leg at the middle of the ruling (the
/// rows' legs averaged); the largest over the faces' ends. What
/// [`Connected::unresolved`] measures is of this size: the normal there is
/// the cross product of that leg and the ruling.
fn end_rounding_estimate(made: &Connected) -> f64 {
    let mut worst: f64 = 0.0;
    for face in &made.body.faces {
        let Surface::Nurbs(s) = &face.surface else {
            continue;
        };
        let (n, across) = s.counts();
        if across != 2 || n < 2 {
            continue;
        }
        let c = s.control_points();
        for (end, next) in [(0, 1), (n - 1, n - 2)] {
            let (p_lo, p_hi, q_lo, q_hi) =
                (c[2 * end], c[2 * end + 1], c[2 * next], c[2 * next + 1]);
            let leg = ((q_lo - p_lo) + (q_hi - p_hi)) * 0.5;
            let size = [p_lo, p_hi, q_lo, q_hi]
                .iter()
                .map(|p| p.length())
                .fold(0.0, f64::max);
            let estimate = f64::EPSILON * size * p_lo.distance(p_hi) / leg.length();
            if estimate.is_finite() {
                worst = worst.max(estimate);
            }
        }
    }
    worst
}

/// The sea kayak's sheer panel (`DevTop4S`, 2026-10-28): the developable
/// between its chine and its sheer, edges of a boolean of the hull, cubic
/// Hermite splines of 522 and 532 control points (every inner knot double)
/// 5 m long (`developable_rails/sea_kayak_sheer.json`, in millimetres).
/// Besides the developable's own branch the condition has, near each end,
/// a branch of rulings across it (from near one end of A to farther along
/// B and back), and the two pass each other where they meet: 1.7e-4 of
/// the rails (0.85 mm) apart at the bow, the model's value at the saddle
/// 3e-8 over a ruling of 21 mm (a twist of 6.8e-7 mm, more than half the
/// tolerance), and 1e-7 over 24 mm at the stern (2.4e-6 mm). The walk
/// stepped from one branch to the other in one step and no ruling could be
/// put between the two it found: refused, "the rulings between two found
/// ones cannot be followed". Now the walk keeps its step short of such a
/// saddle, follows its branch into the fold where it turns, and closes the
/// fold with a fan (Method 4), as every fold: at each end a fan, so the
/// sheet is six faces named by place (the fan at the bow the first fold
/// from the start; at the stern the fan across the fold lands on the other
/// branch short of that branch's own fold, which a second fan closes, the
/// first fold from the end).
///
/// Every face is an exact developable: the condition at every ruling
/// found, the rulings never cross, each face flat within the tolerance by
/// the unfold's estimate, the fit within half the tolerance, and the faces
/// tangent across every edge they share; at three scales (the curves
/// scaled about the origin, at 1 the kayak's own). Laid flat as one piece
/// it is refused at the strip beside the bow's fan (the owed layout of a
/// face whose parametrisation stands still at a corner). The same panel between
/// the rails cut short of both folds (3 % to 97 % of each) is one face,
/// laid flat keeping every edge's length (it was the oracle against which
/// the stepped stretches of develop-r1 failed; review of 2026-10-28).
#[test]
fn rails_whose_condition_has_branches_that_pass_each_other_are_closed_by_fans() {
    let text = std::fs::read_to_string(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/developable_rails/sea_kayak_sheer.json"),
    )
    .unwrap();
    let data: serde_json::Value = serde_json::from_str(&text).unwrap();
    let rail_of = |name: &str, k: f64, part: (f64, f64)| -> Rail {
        let d = &data[name];
        let floats = |v: &serde_json::Value| -> Vec<f64> {
            v.as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_f64().unwrap())
                .collect()
        };
        let ctrl: Vec<Vec3> = d["ctrl"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                let p = floats(p);
                Vec3::new(p[0], p[1], p[2]) * k
            })
            .collect();
        let curve = NurbsCurve::new(
            d["degree"].as_u64().unwrap() as usize,
            floats(&d["knots"]),
            ctrl,
            floats(&d["weights"]),
        )
        .unwrap();
        let range = floats(&d["range"]);
        let w = range[1] - range[0];
        rail(
            Curve3::Nurbs(curve),
            (range[0] + part.0 * w, range[0] + part.1 * w),
        )
    };
    for k in SCALES {
        let (a, b) = (rail_of("A", k, (0.0, 1.0)), rail_of("B", k, (0.0, 1.0)));
        let made = connect(&a, &b, &tol(k)).unwrap_or_else(|e| panic!("{k}: {e}"));
        let estimate = end_rounding_estimate(&made);
        eprintln!(
            "{k}: faces {:?}, {} rulings, deviation {:e}, unresolved {:e} ({:e} tolerances; the coordinates' rounding at the ends {:e}), K {:e}",
            made.faces,
            made.rulings.len(),
            made.deviation,
            made.unresolved,
            made.unresolved / tol(k).linear,
            estimate / tol(k).linear,
            made.curvature
        );
        // What is left unresolved at the faces' ends is what the rounding
        // of their control points explains (review of 2026-10-29: it was
        // an allowance that grew as the end intervals were refined).
        assert!(
            made.unresolved <= 16.0 * estimate,
            "{k}: unresolved {:e}, the rounding {estimate:e}",
            made.unresolved
        );
        assert!(!made.reversed_b, "{k}");
        // The end rulings are the corners' (the fans there lie within the
        // tolerance of the strip run on to them, so the strip runs on to
        // the corners as in `a_fan_within_the_tolerance_of_the_strip_beside_it_is_not_made`):
        // a corner ruling is fixed by the rails' ends, not a root of the
        // condition, so it is held to the corners exactly and to the twist
        // the fit holds every ruling to (computed from the tangent planes,
        // not from the condition).
        let last = made.rulings.len() - 1;
        assert_eq!(made.rulings.first(), Some(&(0.0, 0.0)), "{k}");
        assert_eq!(made.rulings.last(), Some(&(1.0, 1.0)), "{k}");
        for (i, ruling) in made.rulings.iter().enumerate() {
            if i == 0 || i == last {
                let d = twist(&a, &b, *ruling);
                assert!(
                    d <= 0.5e-6 * k,
                    "{k}: twist {d} at the end ruling {ruling:?}"
                );
                continue;
            }
            let f = condition(&a, &b, *ruling);
            assert!(f.abs() < 1e-8, "{k}: condition {f} at {ruling:?}");
        }
        assert!(
            made.rulings
                .windows(2)
                .all(|w| w[1].0 >= w[0].0 && w[1].1 >= w[0].1),
            "{k}: rulings are monotone"
        );
        assert_flat_within_tolerance(&made, k);
        assert!(made.deviation <= 0.5e-6 * k, "{k}: {}", made.deviation);
        // Tangent across every edge two faces share, at its middle: the
        // fans meet the rulings beside them in their tangent planes.
        let body = &made.body;
        let mut shared = 0;
        for (edge, faces) in craft3d_topo::edge_uses(body).iter().enumerate() {
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
            // Each face's plane along the shared ruling within half the
            // tolerance over the ruling's length of the developable's (the
            // fit's end rule), so the two within the tolerance over it.
            let length = e.curve.eval(e.range.0).distance(e.curve.eval(e.range.1));
            let limit = 1e-6 * k / length;
            // Where a face is so narrow across its rulings that the
            // rounding of its points' coordinates turns its normal by more
            // than a tenth of that (`ε |p| / width`: the stern's second fan
            // is about 14 tolerances wide, 2.4e9 tolerances from the
            // origin), the angle is not resolved and not compared.
            let width = |use_: &craft3d_topo::EdgeUse| {
                let face = body.face(use_.face).unwrap();
                let row = |v: f64| {
                    face.surface
                        .eval(0.0, v)
                        .distance(face.surface.eval(1.0, v))
                };
                row(0.0).max(row(1.0))
            };
            let rounding = f64::EPSILON * mid.length() / width(first).min(width(second));
            if rounding > 0.1 * limit {
                continue;
            }
            assert!(
                n0.cross(n1).length() <= limit && n0.dot(n1) > 0.0,
                "{k}: faces {:?} and {:?} meet at an angle of {} rad",
                first.face,
                second.face,
                n0.cross(n1).length()
            );
        }
        assert_eq!(shared, 5, "{k}: {shared} shared edges");
        // The rails' ends line up with a ruling within the tolerance here
        // (no end fans); a fold near each end.
        assert_eq!(
            made.faces,
            vec![
                Piece::Rulings,
                Piece::Fan(Mark::FoldFromStart(0)),
                Piece::After(Mark::FoldFromStart(0)),
                Piece::Fan(Mark::FoldFromEnd(1)),
                Piece::Fan(Mark::FoldFromEnd(0)),
                Piece::Last,
            ],
            "{k}"
        );
        // Laid flat as one piece it is refused, at the strip beside the
        // bow's fan (face 2): the strip starts on the ruling where the
        // walk's branch turned, where one of its rows stands still, and the
        // layout of a face whose parametrisation stands still at a corner
        // is the owed one (docs/design/developable.md, Open; the same
        // refusal as for the strip of
        // `branches_that_pass_each_other_at_an_inflection_are_bridged_by_a_fan`).
        // A regression test of the refusal; what pins the panel is the
        // geometry above and the same panel short of the folds, laid flat
        // below.
        assert_eq!(made.faces[2], Piece::After(Mark::FoldFromStart(0)), "{k}");
        let faces: Vec<FaceId> = (0..made.body.faces.len() as u32).map(FaceId).collect();
        match develop(&made.body, &faces, &tol(k)) {
            Ok(dev) => panic!(
                "{k}: the sheer panel laid flat, {:?}: update this test",
                dev.error()
            ),
            Err(e) => {
                let e = e.to_string();
                assert!(
                    e.contains("face 2 ") && e.contains("stands still"),
                    "{k}: {e}"
                );
            }
        }
        // Cut short of the folds: one face, laid flat, every edge keeps
        // its length.
        let part = (0.03, 0.97);
        let (a, b) = (rail_of("A", k, part), rail_of("B", k, part));
        let made = connect(&a, &b, &tol(k)).unwrap_or_else(|e| panic!("{k}: short: {e}"));
        assert!(
            made.faces.iter().all(|p| !matches!(p, Piece::Fan(_))),
            "{k}: {:?}",
            made.faces
        );
        let faces: Vec<FaceId> = (0..made.body.faces.len() as u32).map(FaceId).collect();
        let dev =
            develop(&made.body, &faces, &tol(k)).unwrap_or_else(|e| panic!("{k}: short: {e}"));
        let e = dev.error();
        eprintln!("{k}: short of the folds, unfolded, {e:?}");
        assert!(e.map <= 1e-6 * k, "{k}: {e:?}");
        for c in dev.outline() {
            assert!(
                (c.length_flat - c.length_3d).abs() <= 1e-6 * k,
                "{k}: an edge {} long on the part, {} flat",
                c.length_3d,
                c.length_flat
            );
        }
    }
}

/// The fan at a fold, laid flat alone (2026-10-28): its layout from the
/// grid's rows converged only at first order towards its apex and ran out
/// of grid (r1-kayakplates, 2026-10-27). Laid about its apex, each line
/// from the apex in the direction its ruling has on the unit sphere's arc,
/// it is exact: it opens by `∫ |d(B / |B|)|` over its stretch of B (the
/// apex at the origin), every edge keeps its length, and a point mapped
/// there and back returns; at three scales.
#[test]
fn a_fan_is_laid_flat_about_its_apex() {
    for k in SCALES {
        let (a, b) = passing_branches(k);
        let made = connect(&a, &b, &tol(k)).unwrap_or_else(|e| panic!("{k}: {e}"));
        let fan = made
            .faces
            .iter()
            .position(|p| matches!(p, Piece::Fan(_)))
            .unwrap();
        let surface = &made.body.faces[fan].surface;
        const CHORDS: usize = 1 << 14;
        let row: Vec<Vec3> = (0..=CHORDS)
            .map(|i| surface.eval(i as f64 / CHORDS as f64, 1.0))
            .collect();
        let angle: f64 = row
            .windows(2)
            .map(|w| {
                w[0].normalized()
                    .unwrap()
                    .cross(w[1].normalized().unwrap())
                    .length()
                    .pasin()
            })
            .sum();
        let dev = develop(&made.body, &[FaceId(fan as u32)], &tol(k))
            .unwrap_or_else(|e| panic!("{k}: {e}"));
        let e = dev.error();
        eprintln!("{k}: the fan laid flat, {e:?}");
        assert!(e.map <= 0.5e-6 * k, "{k}: {e:?}");
        for c in dev.outline() {
            assert!(
                (c.length_flat - c.length_3d).abs() <= 1e-6 * k,
                "{k}: an edge {} long on the part, {} flat",
                c.length_3d,
                c.length_flat
            );
        }
        let flat = |p: Vec3| dev.to_flat(FaceId(fan as u32), p).unwrap();
        let (apex, first, last) = (
            flat(surface.eval(0.5, 0.0)),
            flat(row[0]),
            flat(row[CHORDS]),
        );
        let (u, w) = (
            [first[0] - apex[0], first[1] - apex[1]],
            [last[0] - apex[0], last[1] - apex[1]],
        );
        let opened = (u[0] * w[1] - u[1] * w[0])
            .abs()
            .patan2(u[0] * w[0] + u[1] * w[1]);
        assert!(
            (opened - angle).abs() <= 1e-6,
            "{k}: the fan opens by {opened} in the pattern, {angle} on the part"
        );
        // There and back.
        for (uu, vv) in [(0.2, 0.3), (0.5, 0.9), (0.8, 0.05)] {
            let p = surface.eval(uu, vv);
            let (face, q) = dev.to_3d(flat(p)).unwrap();
            assert_eq!(face, FaceId(fan as u32));
            assert!(q.distance(p) <= 1e-6 * k, "{k}: {p:?} came back as {q:?}");
        }
        // The rays from the apex are straight: the midpoint of a ruling
        // lies halfway along its image.
        let (m, end) = (flat(surface.eval(0.4, 0.5)), flat(surface.eval(0.4, 1.0)));
        let half = [0.5 * (apex[0] + end[0]), 0.5 * (apex[1] + end[1])];
        assert!(
            (m[0] - half[0]).phypot(m[1] - half[1]) <= 1e-6 * k,
            "{k}: a ruling's middle off its image's"
        );
    }
}

/// A rail of several pieces end to end.
fn chain(pieces: Vec<(Curve3, (f64, f64))>) -> Rail {
    Rail {
        pieces: pieces
            .into_iter()
            .enumerate()
            .map(|(i, (curve, range))| RailPiece {
                curve,
                range,
                reversed: false,
                source: i as u64 + 1,
            })
            .collect(),
    }
}

/// A chain's point and unit tangent at its parameter `s` (piece `⌊s⌋`
/// over `[i, i + 1]`), the piece before a joint when `before`.
fn chain_at(r: &Rail, s: f64, before: bool) -> (Vec3, Vec3) {
    let n = r.pieces.len();
    let i = if before && s.fract() == 0.0 && s >= 1.0 {
        s as usize - 1
    } else {
        (s.floor() as usize).min(n - 1)
    };
    let piece = &r.pieces[i];
    let (a, b) = piece.range;
    let (p, d) = piece.curve.derivatives(a + (s - i as f64) * (b - a));
    (p, d.normalized().unwrap())
}

/// The condition at a ruling of chains: the smaller of its two sides at
/// a joint (a ruling at a corner satisfies the condition of the face on
/// its side).
fn chain_condition(a: &Rail, b: &Rail, (s, t): (f64, f64)) -> f64 {
    let mut best = f64::INFINITY;
    for before_a in [false, true] {
        for before_b in [false, true] {
            let ((pa, ua), (pb, ub)) = (chain_at(a, s, before_a), chain_at(b, t, before_b));
            let f = ua.dot(ub.cross((pb - pa).normalized().unwrap()));
            best = best.min(f.abs());
        }
    }
    best
}

/// Faces that share an edge meet tangent there, at its middle (or, with
/// `crease`, the number of edges where they do not).
fn creases(body: &Body) -> usize {
    let uses = craft3d_topo::edge_uses(body);
    let mut count = 0;
    for (edge, faces) in uses.iter().enumerate() {
        let [first, second] = faces.as_slice() else {
            continue;
        };
        let e = &body.edges[edge];
        let mid = e.curve.eval(0.5 * (e.range.0 + e.range.1));
        let normal = |use_: &craft3d_topo::EdgeUse| {
            let face = body.face(use_.face).unwrap();
            let (u, v) = face.surface.project(mid).unwrap();
            let n = face.surface.normal(u, v).unwrap();
            if face.reversed { -n } else { n }
        };
        let (n0, n1) = (normal(first), normal(second));
        if !(n0.cross(n1).length() < 1e-6 && n0.dot(n1) > 0.0) {
            eprintln!(
                "faces {:?} and {:?} meet at {} rad (dot {})",
                first.face,
                second.face,
                n0.cross(n1).length(),
                n0.dot(n1)
            );
            count += 1;
        }
    }
    count
}

/// Rails with a corner (2026-10-28). A cone between two coaxial arcs, the
/// upper turning by a corner into an arc of another circle in its plane:
/// up to the corner the rulings pass through the cone's apex; at the
/// corner the condition jumps (the rail's tangent does), and the sheet
/// turns it by a fan from the corner across the lower arc to where the
/// rulings after the corner start (the oblique cone between the lower arc
/// and the other circle), so it is smooth across both of the fan's edges
/// while its rail has the corner. Then the rest of the lower arc is a fan
/// from the upper rail's end. Held to the condition at every ruling (on
/// its side of the corner), the fan's apex at the corner, faces tangent
/// where they meet, and the sheet laid flat as one piece keeping every
/// edge's length and its area; at three scales.
#[test]
fn a_rail_with_a_corner_is_turned_by_a_fan_from_the_corner() {
    for k in SCALES {
        let (r_low, r_up, h) = (2.0 * k, 1.0 * k, 1.0 * k);
        let frame = |c: Vec3, x: Vec3| Frame::new(c, Vec3::Z, x).unwrap();
        let circle =
            |c: Vec3, x: Vec3, r: f64| Curve3::Circle(Circle::new(frame(c, x), r).unwrap());
        let a = chain(vec![(circle(Vec3::ZERO, Vec3::X, r_low), (0.2, 2.0))]);
        // The upper rail: an arc of radius 1 at height 1 from 0.2 to 1 rad,
        // then from its end an arc of radius 3 that leaves the first's
        // tangent turned by 0.4 rad, for 0.5 rad.
        let corner_at = 1.0_f64;
        let p = Vec3::new(r_up * corner_at.pcos(), r_up * corner_at.psin(), h);
        let tangent = Vec3::new(-corner_at.psin(), corner_at.pcos(), 0.0);
        let (c, s) = (0.4_f64.pcos(), 0.4_f64.psin());
        let d = Vec3::new(
            c * tangent.x - s * tangent.y,
            s * tangent.x + c * tangent.y,
            0.0,
        );
        let left = Vec3::Z.cross(d);
        let rho = 3.0 * k;
        let b = chain(vec![
            (
                circle(Vec3::new(0.0, 0.0, h), Vec3::X, r_up),
                (0.2, corner_at),
            ),
            (circle(p + left * rho, -left, rho), (0.0, 0.5)),
        ]);
        let made = connect(&a, &b, &tol(k)).unwrap_or_else(|e| panic!("{k}: {e}"));
        eprintln!("{k}: faces {:?}", made.faces);
        assert_eq!(
            made.faces,
            // The fan from the corner of B before its second piece, and
            // the run after it, which reaches the end of A (2026-10-28:
            // named by place).
            vec![
                Piece::Rulings,
                Piece::Fan(Mark::Corner { rail: 1, source: 2 }),
                Piece::Last,
                Piece::EndFan
            ],
            "{k}"
        );
        for &ruling in &made.rulings {
            let f = chain_condition(&a, &b, ruling);
            assert!(f < 1e-8, "{k}: condition {f} at {ruling:?}");
        }
        assert!(
            made.rulings
                .windows(2)
                .all(|w| w[1].0 >= w[0].0 && w[1].1 >= w[0].1),
            "{k}: rulings are monotone"
        );
        assert_flat_within_tolerance(&made, k);
        // Up to the corner, through the cone's apex at height 2.
        let apex = Vec3::new(0.0, 0.0, 2.0 * k);
        for &(s, t) in made.rulings.iter().filter(|(_, t)| *t < 1.0) {
            let ((pa, _), (pb, _)) = (chain_at(&a, s, false), chain_at(&b, t, false));
            let dir = (pb - pa).normalized().unwrap();
            let off = (apex - pa) - dir * (apex - pa).dot(dir);
            assert!(
                off.length() <= 1e-6 * k,
                "{k}: misses the apex by {}",
                off.length()
            );
        }
        // The fan's apex is the corner.
        let fan = &made.body.faces[1].surface;
        for u in [0.0, 0.5, 1.0] {
            let at = fan
                .eval(u, 0.0)
                .distance(p)
                .min(fan.eval(u, 1.0).distance(p));
            assert!(at <= 1e-6 * k, "{k}: the fan's apex {at} from the corner");
        }
        assert_eq!(creases(&made.body), 0, "{k}: the sheet is smooth");
        // Laid flat as one piece.
        let faces: Vec<FaceId> = (0..made.body.faces.len() as u32).map(FaceId).collect();
        let dev = develop(&made.body, &faces, &tol(k)).unwrap_or_else(|e| panic!("{k}: {e}"));
        let e = dev.error();
        eprintln!("{k}: laid flat, {e:?}");
        assert!(e.area < 1e-6 && e.length < 1e-6, "{k}: {e:?}");
        assert!(e.map <= 0.5e-6 * k, "{k}: {e:?}");
        for c in dev.outline() {
            assert!(
                (c.length_flat - c.length_3d).abs() <= 1e-6 * k,
                "{k}: an edge {} long on the part, {} flat",
                c.length_3d,
                c.length_flat
            );
        }
    }
}

/// Both rails with a corner on one ruling (2026-10-28): a generalised
/// cylinder over a profile of a quarter circle and an arc of radius `ρ`
/// that leaves it turned by 45 degrees, the second rail the first moved
/// square to the profile's plane. The rulings are parallel to the move; at
/// the corner both rails turn and the sheet is creased along that ruling:
/// two faces that part there. Laid flat, the rectangle
/// `w × (π r / 2 + 0.6 ρ)`; at three scales.
#[test]
fn rails_with_a_corner_on_one_ruling_crease_the_sheet_there() {
    for k in SCALES {
        let (r, rho, w) = (1.0 * k, 1.5 * k, 0.6 * k);
        let l = 0.6 * rho;
        let profile = |z: f64| {
            let frame = Frame::new(Vec3::new(0.0, 0.0, z), Vec3::Z, Vec3::X).unwrap();
            let end = Vec3::new(0.0, r, z);
            let dir = Vec3::new(-1.0, 1.0, 0.0).normalized().unwrap();
            let left = Vec3::Z.cross(dir);
            let turn = Frame::new(end + left * rho, Vec3::Z, -left).unwrap();
            chain(vec![
                (
                    Curve3::Circle(Circle::new(frame, r).unwrap()),
                    (0.0, std::f64::consts::FRAC_PI_2),
                ),
                (Curve3::Circle(Circle::new(turn, rho).unwrap()), (0.0, 0.6)),
            ])
        };
        let (a, b) = (profile(0.0), profile(w));
        let made = connect(&a, &b, &tol(k)).unwrap_or_else(|e| panic!("{k}: {e}"));
        assert_eq!(made.faces, vec![Piece::Rulings, Piece::Last], "{k}");
        for &(s, t) in &made.rulings {
            let ((pa, _), (pb, _)) = (chain_at(&a, s, false), chain_at(&b, t, false));
            let d = pb - pa;
            assert!(
                (d - Vec3::new(0.0, 0.0, w)).length() <= 1e-6 * k,
                "{k}: a ruling {d:?}"
            );
        }
        assert_eq!(creases(&made.body), 1, "{k}: one crease");
        let faces: Vec<FaceId> = (0..made.body.faces.len() as u32).map(FaceId).collect();
        let dev = develop(&made.body, &faces, &tol(k)).unwrap_or_else(|e| panic!("{k}: {e}"));
        let e = dev.error();
        assert!(e.area < 1e-6 && e.length < 1e-6, "{k}: {e:?}");
        let area: f64 = craft3d_topo::face_areas(&made.body)
            .iter()
            .map(|a| *a.as_ref().unwrap())
            .sum();
        let expected = w * (std::f64::consts::FRAC_PI_2 * r + l);
        assert!(
            (area - expected).abs() <= 1e-6 * expected,
            "{k}: area {area} for {expected}"
        );
    }
}

/// Refusals with the reason (2026-10-28): a rail that turns back on itself
/// (a cusp), and rails that close on themselves.
#[test]
fn a_rail_that_turns_back_is_refused() {
    let k = 1.0;
    let a = chain(vec![
        (
            Curve3::Line(Line::new(Vec3::ZERO, Vec3::X).unwrap()),
            (0.0, 1.0),
        ),
        (
            Curve3::Line(Line::new(Vec3::X, -Vec3::X).unwrap()),
            (0.0, 0.5),
        ),
    ]);
    let b = chain(vec![(
        Curve3::Line(Line::new(Vec3::new(0.0, 0.0, 1.0), Vec3::X).unwrap()),
        (0.0, 1.0),
    )]);
    let e = connect(&a, &b, &tol(k)).unwrap_err();
    assert!(e.to_string().contains("cusp"), "{e}");
}

/// A full circle of radius `r` about `(cx, cy, z)` as a closed rail.
fn loop_rail(cx: f64, cy: f64, z: f64, r: f64) -> Rail {
    let frame = Frame::new(Vec3::new(cx, cy, z), Vec3::Z, Vec3::X).unwrap();
    chain(vec![(
        Curve3::Circle(Circle::new(frame, r).unwrap()),
        (0.0, TAU),
    )])
}

/// Rails that close on themselves: a developable tube (2026-10-28). Two
/// coaxial circles give the frustum of a cone, its rulings through the
/// apex, one face whose seam (the ruling from the first circle's start)
/// it uses on both sides; laid flat, a ring's sector with the frustum's
/// area `π (R₁ + R₂) √(h² + (R₁ − R₂)²)` and the circles' lengths. Two
/// parallel circles whose centres are not over one another give an
/// oblique cone: every ruling through the circles' centre of similitude.
/// At three scales.
#[test]
fn closed_rails_give_a_developable_tube() {
    for k in SCALES {
        let (r1, r2, h) = (2.0 * k, 1.0 * k, 1.5 * k);
        let (a, b) = (loop_rail(0.0, 0.0, 0.0, r1), loop_rail(0.0, 0.0, h, r2));
        let made = connect(&a, &b, &tol(k)).unwrap_or_else(|e| panic!("{k}: {e}"));
        eprintln!(
            "{k}: tube faces {:?}, {} rulings",
            made.faces,
            made.rulings.len()
        );
        assert_eq!(made.faces, vec![Piece::Rulings], "{k}");
        check_rulings(&made, &a, &b);
        let apex = Vec3::new(0.0, 0.0, h * r1 / (r1 - r2));
        for &(s, t) in &made.rulings {
            let ((pa, _), (pb, _)) = (chain_at(&a, s, false), chain_at(&b, t, false));
            let d = (pb - pa).normalized().unwrap();
            let off = (apex - pa) - d * (apex - pa).dot(d);
            assert!(
                off.length() <= 1e-6 * k,
                "{k}: misses the apex by {}",
                off.length()
            );
        }
        // Closed: every edge used twice but the two circles; the seam used
        // twice by the one face.
        let uses = craft3d_topo::edge_uses(&made.body);
        let once = uses.iter().filter(|u| u.len() == 1).count();
        let twice = uses
            .iter()
            .filter(|u| u.len() == 2 && u[0].face == u[1].face)
            .count();
        assert_eq!((once, twice), (2, 1), "{k}: {uses:?}");
        let area: f64 = craft3d_topo::face_areas(&made.body)
            .iter()
            .map(|a| *a.as_ref().unwrap())
            .sum();
        let expected = std::f64::consts::PI * (r1 + r2) * (h * h + (r1 - r2) * (r1 - r2)).sqrt();
        assert!(
            (area - expected).abs() <= 1e-6 * expected,
            "{k}: area {area} for {expected}"
        );
        let dev = develop(&made.body, &[FaceId(0)], &tol(k)).unwrap_or_else(|e| panic!("{k}: {e}"));
        let e = dev.error();
        assert!(e.area < 1e-6 && e.length < 1e-6, "{k}: {e:?}");
        let mut circles: Vec<f64> = dev
            .outline()
            .iter()
            .map(|c| c.length_flat)
            .filter(|l| *l > 2.0 * k)
            .collect();
        circles.sort_by(f64::total_cmp);
        assert_eq!(circles.len(), 2, "{k}: {circles:?}");
        assert!(
            (circles[0] - TAU * r2).abs() <= 1e-6 * k,
            "{k}: {circles:?}"
        );
        assert!(
            (circles[1] - TAU * r1).abs() <= 1e-6 * k,
            "{k}: {circles:?}"
        );

        // Oblique.
        let (a, b) = (loop_rail(0.0, 0.0, 0.0, r1), loop_rail(0.4 * k, 0.0, h, r2));
        let made = connect(&a, &b, &tol(k)).unwrap_or_else(|e| panic!("{k}: oblique: {e}"));
        check_rulings(&made, &a, &b);
        let centre = Vec3::new(r1 * 0.4 * k / (r1 - r2), 0.0, h * r1 / (r1 - r2));
        for &(s, t) in &made.rulings {
            let ((pa, _), (pb, _)) = (chain_at(&a, s, false), chain_at(&b, t, false));
            let d = (pb - pa).normalized().unwrap();
            let off = (centre - pa) - d * (centre - pa).dot(d);
            assert!(
                off.length() <= 1e-6 * k,
                "{k}: oblique: misses the centre by {}",
                off.length()
            );
        }
    }
}

/// A closed rail with an open one is refused with the reason.
/// Two closed rails in one plane: the flat ring between two loops one
/// inside the other, drawn the same way round, has the area between them
/// (`π (R² − r²)` for concentric circles, closed form). Two identical
/// loops, loops side by side, loops that cross and loops drawn opposite
/// ways round have no ring: refused as given with the reason (review of
/// 2026-10-28: the plane was taken without these checks); a loop drawn
/// the other way is then turned, as every rail that only that way gives
/// a developable (`connect`), and reported so. At three scales.
#[test]
fn closed_rails_in_one_plane_give_a_ring_only_when_nested_and_run_alike() {
    for k in SCALES {
        let (r_out, r_in) = (2.0 * k, 1.0 * k);
        let ring = TAU / 2.0 * (r_out * r_out - r_in * r_in);
        let (a, b) = (
            loop_rail(0.0, 0.0, 0.0, r_out),
            loop_rail(0.3 * k, 0.0, 0.0, r_in),
        );
        let made = connect(&a, &b, &tol(k)).unwrap_or_else(|e| panic!("{k}: {e}"));
        assert!(made.planar && !made.reversed_b, "{k}");
        let area = craft3d_topo::mass_properties(&made.body).unwrap().area;
        assert!((area - ring).abs() <= 1e-9 * ring, "{k}: {area} for {ring}");
        // The inner one first: the same ring.
        let made = connect(&b, &a, &tol(k)).unwrap_or_else(|e| panic!("{k}: {e}"));
        let area = craft3d_topo::mass_properties(&made.body).unwrap().area;
        assert!((area - ring).abs() <= 1e-9 * ring, "{k}: {area} for {ring}");
        // Identical.
        let e = connect(&a, &a.clone(), &tol(k)).unwrap_err().to_string();
        assert!(e.contains("are the same"), "{k}: {e}");
        // Side by side.
        let apart = loop_rail(5.0 * k, 0.0, 0.0, r_in);
        let e = connect(&a, &apart, &tol(k)).unwrap_err().to_string();
        assert!(e.contains("neither encloses the other"), "{k}: {e}");
        // Crossing.
        let crossing = loop_rail(2.0 * k, 0.0, 0.0, r_in);
        let e = connect(&a, &crossing, &tol(k)).unwrap_err().to_string();
        assert!(e.contains("touch or cross"), "{k}: {e}");
        // Drawn opposite ways round: turned, the same ring.
        let opposite = b.reversed();
        let made = connect(&a, &opposite, &tol(k)).unwrap_or_else(|e| panic!("{k}: {e}"));
        assert!(made.planar && made.reversed_b, "{k}");
        let area = craft3d_topo::mass_properties(&made.body).unwrap().area;
        assert!((area - ring).abs() <= 1e-9 * ring, "{k}: {area} for {ring}");
    }
}

/// Two identical closed rails that do not lie in one plane (two half
/// circles at right angles, end to end) have no tube between them:
/// refused, not a sheet of rulings of no length. At three scales.
#[test]
fn identical_closed_rails_are_refused() {
    for k in SCALES {
        let r = 2.0 * k;
        let flat = Frame::new(Vec3::ZERO, Vec3::Z, Vec3::X).unwrap();
        let upright = Frame::new(Vec3::ZERO, Vec3::Y, -Vec3::X).unwrap();
        let a = chain(vec![
            (
                Curve3::Circle(Circle::new(flat, r).unwrap()),
                (0.0, TAU / 2.0),
            ),
            (
                Curve3::Circle(Circle::new(upright, r).unwrap()),
                (0.0, TAU / 2.0),
            ),
        ]);
        let e = connect(&a, &a.clone(), &tol(k)).unwrap_err().to_string();
        assert!(e.contains("are the same"), "{k}: {e}");
    }
}

#[test]
fn a_closed_and_an_open_rail_are_refused() {
    let a = loop_rail(0.0, 0.0, 0.0, 1.0);
    let b = arc(Vec3::new(0.0, 0.0, 1.0), 1.0, 0.0, 3.0);
    let e = connect(&a, &b, &tol(1.0)).unwrap_err();
    assert!(e.to_string().contains("closed"), "{e}");
}

/// A closed loop with corners at `(±2 s k, 0, z k)`: the upper half of the
/// circle of radius `2 s k` about the axis, then the lower arc of the
/// circle about `(0, s k)` through the same two points. It starts at a
/// corner, `(2 s k, 0)`.
fn lens(k: f64, z: f64, s: f64) -> Rail {
    let r5 = 5f64.sqrt();
    let c1 = Frame::new(Vec3::new(0.0, 0.0, z * k), Vec3::Z, Vec3::X).unwrap();
    let c2 = Frame::new(Vec3::new(0.0, s * k, z * k), Vec3::Z, Vec3::X).unwrap();
    let (t0, t1) = ((-1.0f64).patan2(-2.0) + TAU, (-1.0f64).patan2(2.0) + TAU);
    chain(vec![
        (
            Curve3::Circle(Circle::new(c1, 2.0 * s * k).unwrap()),
            (0.0, std::f64::consts::PI),
        ),
        (
            Curve3::Circle(Circle::new(c2, r5 * s * k).unwrap()),
            (t0, t1),
        ),
    ])
}

/// A tube whose first loop closes at a corner (review of 2026-10-29: it
/// was refused, "start the curve where it is smooth", though a loop's
/// start is where its curves were found to close): the seam is started in
/// the middle of its first piece instead. Two lenses, the second half the
/// size and `k` higher, centred on the axis: a cone with its apex at
/// `(0, 0, 2k)`, every ruling through it, creased on the rulings through
/// the corners, laid flat keeping every edge's length; at three scales.
#[test]
fn a_closed_rail_that_closes_at_a_corner_is_started_where_it_is_smooth() {
    for k in SCALES {
        let (a, b) = (lens(k, 0.0, 1.0), lens(k, 1.0, 0.5));
        let made = connect(&a, &b, &tol(k)).unwrap_or_else(|e| panic!("{k}: {e}"));
        let apex = Vec3::new(0.0, 0.0, 2.0 * k);
        for f in &made.body.faces {
            for u in [0.0, 0.25, 0.5, 0.75, 1.0] {
                let (pa, pb) = (f.surface.eval(u, 0.0), f.surface.eval(u, 1.0));
                let d = (pb - pa).normalized().unwrap();
                let off = (apex - pa) - d * (apex - pa).dot(d);
                assert!(
                    off.length() <= 1e-6 * k,
                    "{k}: a ruling misses the apex by {}",
                    off.length()
                );
            }
        }
        assert!(made.faces.len() >= 2, "{k}: {:?}", made.faces);
        let faces: Vec<FaceId> = (0..made.body.faces.len() as u32).map(FaceId).collect();
        let dev = develop(&made.body, &faces, &tol(k)).unwrap_or_else(|e| panic!("{k}: {e}"));
        for c in dev.outline() {
            assert!(
                (c.length_flat - c.length_3d).abs() <= 1e-6 * k,
                "{k}: an edge {} long on the part, {} flat",
                c.length_3d,
                c.length_flat
            );
        }
    }
}

/// Two closed rails refused both ways round are refused with both reasons
/// (review of 2026-10-29: only the way given was told). A circle and a
/// smaller one tilted out of its plane: as given the fit does not reach
/// the tolerance, the other way round the rulings do not move along the
/// curves; at three scales.
#[test]
fn closed_rails_refused_both_ways_round_tell_both_reasons() {
    for k in SCALES {
        let a = loop_rail(0.0, 0.0, 0.0, k);
        let frame = Frame::new(
            Vec3::new(0.0, 0.0, 0.3 * k),
            Vec3::new(1.0, 0.0, 0.2).normalized().unwrap(),
            Vec3::Y,
        )
        .unwrap();
        let b = chain(vec![(
            Curve3::Circle(Circle::new(frame, 0.5 * k).unwrap()),
            (0.0, TAU),
        )]);
        let e = connect(&a, &b, &tol(k)).unwrap_err().to_string();
        assert!(
            e.contains("; with the second curve run the other way round: "),
            "{k}: {e}"
        );
    }
}
