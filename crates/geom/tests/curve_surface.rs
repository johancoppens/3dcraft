//! `intersect_curve_surface` for every kind of curve against every kind of
//! surface: closed forms against known positions, everything against a
//! brute-force search for sign changes of an implicit function written
//! here.

// Test code: a failure in a helper is a failed test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::needless_range_loop
)]

use craft3d_math::Portable;
mod common;

use common::*;
use craft3d_geom::intersect::{CurveHit, CurveIntersection, intersect_curve_surface};
use craft3d_geom::{
    BezierPatch, Circle, Curve3, Ellipse, GeomError, Line, NurbsCurve, NurbsSurface, Surface,
};
use craft3d_math::{Frame, Tolerance, Vec3};
use std::collections::BTreeMap;
use std::time::Instant;

/// A function with the surface as its zero set, changing sign across it.
/// `None` past the edge of a freeform patch.
fn implicit(surface: &Surface, p: Vec3) -> Option<f64> {
    match surface {
        Surface::Plane(s) => Some(s.frame().local(p).2),
        Surface::Cylinder(s) => {
            let (x, y, _) = s.frame().local(p);
            Some(x.phypot(y) - s.radius())
        }
        Surface::Cone(s) => {
            let (x, y, z) = s.frame().local(p);
            Some(x.phypot(y) - (s.radius() + z * s.half_angle().ptan()))
        }
        Surface::Sphere(s) => Some(p.distance(s.frame().origin()) - s.radius()),
        Surface::Torus(s) => {
            let (x, y, z) = s.frame().local(p);
            Some((x.phypot(y) - s.radii().0).phypot(z) - s.radii().1)
        }
        Surface::Bezier(_) | Surface::Nurbs(_) => {
            let ((u0, u1), (v0, v1)) = surface.domain();
            let (u, v) = surface.project(p).ok()?;
            let edge = |x: f64, a: f64, b: f64| x - a <= 1e-9 * (b - a) || b - x <= 1e-9 * (b - a);
            if edge(u, u0, u1) || edge(v, v0, v1) {
                return None;
            }
            Some((p - surface.eval(u, v)).dot(surface.normal(u, v)?))
        }
    }
}

fn range_of(curve: &Curve3, s: f64) -> (f64, f64) {
    match curve {
        Curve3::Line(_) => (-40.0 * s, 40.0 * s),
        _ => curve.domain(),
    }
}

/// Crossings of the curve with the surface by sampling and bisection:
/// `(parameter, sine of the crossing angle)`.
fn brute(curve: &Curve3, surface: &Surface, s: f64) -> Vec<(f64, f64)> {
    let (t0, t1) = range_of(curve, s);
    let n = if matches!(surface, Surface::Bezier(_) | Surface::Nurbs(_)) {
        1500
    } else {
        40_000
    };
    let value = |t: f64| implicit(surface, curve.eval(t));
    let mut roots = Vec::new();
    let mut previous = value(t0);
    for k in 0..n {
        let (a, b) = (
            t0 + (t1 - t0) * f64::from(k) / f64::from(n),
            t0 + (t1 - t0) * f64::from(k + 1) / f64::from(n),
        );
        let next = value(b);
        if let (Some(fa), Some(fb)) = (previous, next)
            && (fa >= 0.0) != (fb >= 0.0)
        {
            let (mut lo, mut hi) = (a, b);
            for _ in 0..60 {
                let mid = 0.5 * (lo + hi);
                match value(mid) {
                    Some(f) if (f >= 0.0) == (fa >= 0.0) => lo = mid,
                    _ => hi = mid,
                }
            }
            let t = 0.5 * (lo + hi);
            let (p, d) = curve.derivatives(t);
            // A jump of the foot point between sheets is not a crossing.
            if distance(surface, p) <= 1e-6 * s
                && let Some(d) = d.normalized()
            {
                // No normal at a cone's apex: any crossing there counts.
                roots.push((t, normal_at(surface, p).map_or(1.0, |n| n.dot(d).abs())));
            }
        }
        previous = next;
    }
    roots
}

fn hits_of(curve: &Curve3, surface: &Surface, t: &Tolerance) -> Vec<CurveHit> {
    match intersect_curve_surface(curve, surface, t).unwrap() {
        CurveIntersection::Hits(h) => h,
        CurveIntersection::OnSurface => panic!("{} on {}", curve.kind(), surface.kind()),
    }
}

/// The hits are on the curve and the surface, in order, and are the
/// crossings a brute-force search finds.
fn check(curve: &Curve3, surface: &Surface, s: f64, label: &str) -> Vec<CurveHit> {
    let t = tol(s);
    let found = hits_of(curve, surface, &t);
    assert_eq!(
        found,
        hits_of(curve, surface, &t),
        "{label}: not deterministic"
    );
    for pair in found.windows(2) {
        assert!(pair[0].t < pair[1].t, "{label}: not ascending");
    }
    for h in &found {
        assert!(
            curve.eval(h.t).distance(h.point) <= 1e-12 * s.max(1.0),
            "{label}"
        );
        let off = distance(surface, h.point);
        let bound = if h.tangent {
            2.0 * t.linear
        } else {
            1e-3 * t.linear
        };
        assert!(off <= bound, "{label}: a hit {off} off the surface");
    }
    let roots = brute(curve, surface, s);
    for (root, sine) in &roots {
        let p = curve.eval(*root);
        let near = found
            .iter()
            .map(|h| (h.point.distance(p), h.tangent))
            .fold((f64::INFINITY, false), |a, b| if b.0 < a.0 { b } else { a });
        // A grazing crossing may be reported as one touch nearby.
        let bound = if near.1 {
            4.0 * (t.linear * s).sqrt()
        } else {
            1e-7 * s / sine.max(1e-3)
        };
        assert!(
            near.0 <= bound,
            "{label}: the crossing at {root} (sine {sine}) is {} from any of {} hits",
            near.0,
            found.len()
        );
    }
    for h in found.iter().filter(|h| !h.tangent) {
        let (t0, t1) = range_of(curve, s);
        if h.t < t0 || h.t > t1 {
            continue;
        }
        let near = roots
            .iter()
            .map(|(root, _)| curve.eval(*root).distance(h.point))
            .fold(f64::INFINITY, f64::min);
        assert!(
            near <= 1e-5 * s,
            "{label}: a hit at {} that is no crossing",
            h.t
        );
    }
    found
}

fn bumpy(s: f64) -> BezierPatch {
    let mut net = [[Vec3::ZERO; 4]; 4];
    for (i, row) in net.iter_mut().enumerate() {
        for (j, p) in row.iter_mut().enumerate() {
            let (x, y) = (i as f64, j as f64 * 1.5);
            *p = Vec3::new(x, y, ((i * 7 + j * 3) % 5) as f64 * 0.3 - 0.2 * x) * s;
        }
    }
    BezierPatch::bicubic(net).unwrap()
}

fn rational(s: f64) -> NurbsSurface {
    let knots = vec![0.0, 0.0, 0.0, 0.0, 0.4, 1.0, 1.0, 1.0, 1.0];
    let (mut ctrl, mut weights) = (Vec::new(), Vec::new());
    for i in 0..5 {
        for j in 0..5 {
            let (x, y) = (i as f64 * 0.8, j as f64 * 1.1);
            ctrl.push(Vec3::new(x, y, ((i * 3 + j * 2) % 4) as f64 * 0.35 - 0.1 * y) * s);
            weights.push(1.0 + 0.5 * ((i + 2 * j) % 3) as f64);
        }
    }
    NurbsSurface::new(3, 3, knots.clone(), knots, 5, 5, ctrl, weights).unwrap()
}

fn line(a: Vec3, direction: Vec3) -> Curve3 {
    Curve3::Line(Line::new(a, direction).unwrap())
}

fn count(curve: &Curve3, surface: &Surface, s: f64, hits: usize, tangent: bool, label: &str) {
    let found = check(curve, surface, s, label);
    assert_eq!(found.len(), hits, "{label}: {found:?}");
    assert!(
        found.iter().all(|h| h.tangent == tangent),
        "{label}: {found:?}"
    );
}

fn on_surface(curve: &Curve3, surface: &Surface, s: f64) -> bool {
    intersect_curve_surface(curve, surface, &tol(s)).unwrap() == CurveIntersection::OnSurface
}

#[test]
fn lines_meet_spheres_cones_and_tori() {
    for s in SCALES {
        let f = tilted(s);
        let ball = sphere(f, 3.0 * s);
        let through = line(f.point(-9.0 * s, 0.5 * s, s), f.vector(1.0, 0.1, -0.05));
        count(&through, &ball, s, 2, false, "line/sphere");
        count(
            &line(f.point(0.0, 3.0 * s, 0.0), f.x()),
            &ball,
            s,
            1,
            true,
            "tangent line/sphere",
        );
        count(
            &line(f.point(0.0, 3.1 * s, 0.0), f.x()),
            &ball,
            s,
            0,
            false,
            "line misses sphere",
        );
        // Chord length against Pythagoras.
        let found = check(&line(f.point(-9.0 * s, 0.0, s), f.x()), &ball, s, "chord");
        assert!((found[1].t - found[0].t - 2.0 * 8.0_f64.sqrt() * s).abs() <= 1e-9 * s);

        let alpha: f64 = 0.4;
        let k = alpha.ptan();
        let c = cone(f, 1.5 * s, alpha);
        let apex = f.point(0.0, 0.0, -1.5 * s / k);
        count(
            &line(f.point(-9.0 * s, 0.3 * s, 2.0 * s), f.x()),
            &c,
            s,
            2,
            false,
            "line across cone",
        );
        // Steeper than the rulings from inside: the other root is on the
        // nappe the surface does not have.
        count(
            &line(f.point(0.2 * s, 0.0, 0.0), f.vector(0.1, 0.0, 1.0)),
            &c,
            s,
            1,
            false,
            "line up the cone",
        );
        count(
            &line(f.point(0.0, 0.0, -9.0 * s), f.x()),
            &c,
            s,
            0,
            false,
            "line behind the apex",
        );
        let ruling = line(apex, f.vector(alpha.psin(), 0.0, alpha.pcos()));
        assert!(on_surface(&ruling, &c, s), "a ruling is on the cone");
        // Parallel to a ruling: inside the cone it meets the opposite
        // ruling once (x = -z tan α at t = 1 / (2 sin α)); outside, the
        // one point it shares with the double cone is on the other nappe.
        let inside = line(apex - f.x() * s, f.vector(alpha.psin(), 0.0, alpha.pcos()));
        let found = check(&inside, &c, s, "line parallel to a ruling, inside");
        assert!(found.len() == 1 && !found[0].tangent, "{found:?}");
        assert!((found[0].t - s / (2.0 * alpha.psin())).abs() <= 1e-9 * s);
        let outside = line(apex + f.x() * s, f.vector(alpha.psin(), 0.0, alpha.pcos()));
        count(
            &outside,
            &c,
            s,
            0,
            false,
            "line parallel to a ruling, outside",
        );
        // Tangent: in the plane that touches the cone along the ruling at
        // u = 0, across that ruling.
        let on_ruling = apex + f.vector(alpha.psin(), 0.0, alpha.pcos()) * (4.0 * s);
        count(
            &line(on_ruling, f.y()),
            &c,
            s,
            1,
            true,
            "line tangent to cone",
        );
        // Through the apex: flatter than the rulings it only touches
        // there; along the axis it goes from outside to inside.
        count(
            &line(apex, f.vector(1.0, 0.2, 0.1)),
            &c,
            s,
            1,
            true,
            "line across the apex",
        );
        count(&line(apex, f.z()), &c, s, 1, false, "the axis");

        let ring = torus(f, 4.0 * s, s);
        count(
            &line(f.point(-9.0 * s, 0.2 * s, 0.3 * s), f.x()),
            &ring,
            s,
            4,
            false,
            "line through torus",
        );
        count(
            &line(f.point(4.0 * s, 0.0, -9.0 * s), f.z()),
            &ring,
            s,
            2,
            false,
            "line through tube",
        );
        count(
            &line(f.point(0.0, 5.0 * s, 0.0), f.x()),
            &ring,
            s,
            1,
            true,
            "line tangent to torus",
        );
        count(
            &line(f.point(0.0, 0.0, -9.0 * s), f.z()),
            &ring,
            s,
            0,
            false,
            "line through the hole",
        );
        // Dipping in by less than the tolerance is one touch, by more two
        // crossings: the rule of the closed forms.
        let dip = |depth: f64| line(f.point(0.0, 5.0 * s - depth * tol(s).linear, 0.0), f.x());
        count(&dip(0.3), &ring, s, 1, true, "line grazing the torus");
        count(
            &dip(30.0),
            &ring,
            s,
            2,
            false,
            "line just through the torus",
        );
        count(
            &line(f.point(0.0, 9.0 * s, 0.0), f.x()),
            &ring,
            s,
            0,
            false,
            "line beside torus",
        );
    }
}

/// A hit a line should have: its parameter, whether it is a touch, and
/// the sine of the angle at which the line leaves the surface there (a
/// hit within the tolerance of the surface may lie the tolerance over it
/// away along the line).
type Want = (f64, bool, f64);

/// Where a line in the plane through a cone's axis meets the cone. In the
/// frame's coordinates x and v (along the axis), with y = 0, the line is
/// `(x0, v0) + t (sin φ, cos φ)` and the cone `x = ±r(v)`,
/// `r(v) = r0 + v tan α`, its rulings at u = 0 and u = π, each only above
/// the apex (the cone has one nappe). The line meets the first at
/// `t = (r(v0) − x0) cos α / sin(φ − α)` and the second at
/// `t = −(r(v0) + x0) cos α / sin(φ + α)`, leaving them at angles whose
/// sines are `|sin(φ ∓ α)|`; it meets neither where it is parallel to it.
fn in_axis_plane(r0: f64, alpha: f64, (x0, v0): (f64, f64), phi: f64) -> Vec<Want> {
    let (cos, tan) = (alpha.pcos(), alpha.ptan());
    let cp = phi.pcos();
    let apex = -r0 / tan;
    let r = r0 + v0 * tan;
    let mut hits = Vec::new();
    for (numerator, sine) in [
        (r - x0, (phi - alpha).psin()),
        (-(r + x0), (phi + alpha).psin()),
    ] {
        if sine != 0.0 {
            let t = numerator * cos / sine;
            if v0 + t * cp > apex {
                hits.push((t, false, sine.abs()));
            }
        }
    }
    hits.sort_by(|a, b| a.0.total_cmp(&b.0));
    hits
}

/// Lines against cones of half-angle 1e-6, 1e-5, 1e-4 and 1e-3 rad (and
/// 0.4 beside them), radius `s` at the origin of a tilted frame, at three
/// scales, against hits known by construction: lines in the plane through
/// the axis parallel to a ruling (and within 1e-10, 1e-7 and 1e-5 rad of
/// it) through a point of the opposite ruling, lines crossing the axis
/// steeper and flatter than the rulings and within 1e-7 of them, lines
/// through the apex (the axis; steeper, flatter and within 1e-7 of a
/// ruling; a ruling and one within 1e-10 of it, which are on the
/// surface), lines tangent to the cone at an angle β to the ruling there
/// (π/2, 0.3, 1e-3: one touch where the tangent plane holds the line),
/// and chords between two points of the cone. Each line's origin is
/// moved back along it. A hit found must be one wanted, within the
/// tolerance over the sine of its angle along the line, of the same kind,
/// and within the tolerance of the cone; a touch within the stretch where
/// the line stays within the tolerance of the cone. (The constructions are
/// the roots of the quadratic solved exactly in rational arithmetic, to 80
/// digits, for half-angles 1e-6 and 1e-3 at scale 1.) Of the 405 lines,
/// 158 were wrong before 2026-11-14: every line within 4.5e-5 rad of a
/// narrow cone's axis was taken as parallel to a ruling, the coefficients
/// taken about a far apex cancelled, and a line through the apex was a
/// touch or a crossing by where its origin lay (docs/design/geometry.md ›
/// *A line and a cone*).
#[test]
fn lines_meet_narrow_cones_where_their_construction_says() {
    let mut wrong: Vec<String> = Vec::new();
    for s in SCALES {
        let f = tilted(s);
        let t = tol(s);
        for alpha in [1e-6_f64, 1e-5, 1e-4, 1e-3, 0.4] {
            let c = cone(f, s, alpha);
            let tan = alpha.ptan();
            let apex = -s / tan;
            // A line in the axis plane through (x, v) at φ from the axis,
            // its origin moved `back` along it.
            let planar = |(x, v): (f64, f64), phi: f64, back: f64| {
                let (sp, cp) = phi.psin_cos();
                let from = (x - back * sp, v - back * cp);
                (from, f.point(from.0, 0.0, from.1), f.vector(sp, 0.0, cp))
            };
            let mut cases: Vec<(String, Vec3, Vec3, Option<Vec<Want>>)> = Vec::new();
            // Parallel to the ruling at u = 0 (and nearly), through the
            // point of the ruling at u = π at v = 0: that point, and where
            // the line is not parallel within the angular tolerance, the
            // ruling at u = 0 far away.
            for delta in [0.0, 1e-10, -1e-10, 1e-7, -1e-7, 1e-5, -1e-5] {
                let (from, o, d) = planar((-s, 0.0), alpha + delta, 2.0 * s);
                let mut want = in_axis_plane(s, alpha, from, alpha + delta);
                if delta.abs() <= t.angular {
                    want.retain(|h| (h.0 - 2.0 * s).abs() <= s);
                }
                cases.push((
                    format!("parallel to a ruling, {delta:e} off"),
                    o,
                    d,
                    Some(want),
                ));
            }
            // Crossing the axis at v = 0, steeper and flatter than the
            // rulings, within 1e-7 of them, and across.
            for phi in [
                0.5 * alpha,
                2.0 * alpha,
                alpha + 1e-7,
                alpha - 1e-7,
                0.3 + alpha,
                1.2,
            ] {
                let (from, o, d) = planar((0.0, 0.0), phi, 3.0 * s);
                let want = in_axis_plane(s, alpha, from, phi);
                cases.push((format!("crossing the axis at {phi:e}"), o, d, Some(want)));
            }
            // Through the apex: steeper than the rulings a crossing there,
            // flatter a touch, along a ruling (or within the angular
            // tolerance of one) on the surface.
            for phi in [
                0.0,
                0.5 * alpha,
                2.0 * alpha,
                alpha + 1e-7,
                alpha - 1e-7,
                1.2,
            ] {
                let (_, o, d) = planar((0.0, apex), phi, 3.0 * s);
                let off = (phi - alpha).psin().abs();
                let want = vec![(3.0 * s, phi > alpha, off)];
                cases.push((format!("through the apex at {phi:e}"), o, d, Some(want)));
            }
            for delta in [0.0, 1e-10] {
                let (_, o, d) = planar((0.0, apex), alpha + delta, 3.0 * s);
                cases.push((format!("a ruling, {delta:e} off"), o, d, None));
            }
            // Tangent at the point of the ruling at u = 0 at v = 0, at β to
            // the ruling in the tangent plane: one touch there, the line
            // within the tolerance of the cone `√(2 tol r / (sin²β cos α))`
            // either way (its normal curvature `sin²β cos α / r`).
            for beta in [std::f64::consts::FRAC_PI_2, 0.3, 1e-3] {
                let (sb, cb) = beta.psin_cos();
                let d = f.vector(cb * alpha.psin(), sb, cb * alpha.pcos());
                let o = f.point(s, 0.0, 0.0) - d * (2.0 * s);
                let stretch = (2.0 * t.linear * s / (sb * sb * alpha.pcos())).sqrt();
                let want = vec![(2.0 * s, true, t.linear / stretch)];
                cases.push((
                    format!("tangent at {beta:e} to the ruling"),
                    o,
                    d,
                    Some(want),
                ));
            }
            // Chords between two points of the cone.
            for (u2, v2) in [(0.5, 0.25 * s / alpha), (2.0, s), (3.0, -0.5 * s)] {
                let (p1, p2) = (c.eval(0.0, 0.0), c.eval(u2, v2));
                let Some(d) = (p2 - p1).normalized() else {
                    continue;
                };
                let sine = |u: f64, v: f64| c.normal(u, v).map_or(1.0, |n| n.dot(d).abs());
                let want = vec![
                    (s, false, sine(0.0, 0.0)),
                    (s + p1.distance(p2), false, sine(u2, v2)),
                ];
                cases.push((
                    format!("a chord to ({u2}, {v2:e})"),
                    p1 - d * s,
                    d,
                    Some(want),
                ));
            }
            for (label, o, d, want) in cases {
                let label = format!("scale {s:e}, α {alpha:e}: {label}");
                let curve = line(o, d);
                let found = intersect_curve_surface(&curve, &c, &t);
                match (found, want) {
                    (Ok(CurveIntersection::OnSurface), None) => {}
                    (Ok(CurveIntersection::Hits(found)), Some(want)) => {
                        let fits = |h: &CurveHit, w: &Want| {
                            h.tangent == w.1
                                && (h.t - w.0).abs() * w.2 <= t.linear
                                && curve.eval(h.t).distance(h.point) <= 1e-12 * s.max(h.t.abs())
                                && distance(&c, h.point) <= t.linear
                        };
                        let ok = found.len() == want.len()
                            && found.iter().zip(&want).all(|(h, w)| fits(h, w));
                        if !ok {
                            let got: Vec<_> = found
                                .iter()
                                .map(|h| (h.t, h.tangent, distance(&c, h.point)))
                                .collect();
                            wrong.push(format!("{label}: got {got:?} (t, touch, off), want {want:?} (t, touch, sine)"));
                        }
                    }
                    (Ok(other), want) => {
                        wrong.push(format!("{label}: got {other:?}, want {want:?}"));
                    }
                    (Err(e), _) => wrong.push(format!("{label}: {e}")),
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

#[test]
fn circles_and_ellipses_meet_curved_surfaces() {
    for s in SCALES {
        let f = tilted(s);
        let t = tol(s);
        let ball = sphere(f, 3.0 * s);
        let leaning = Frame::new(
            f.point(2.0 * s, 0.0, 0.5 * s),
            f.vector(0.3, 1.0, 0.2),
            f.x(),
        )
        .unwrap();
        let circle = Curve3::Circle(Circle::new(leaning, 2.0 * s).unwrap());
        count(&circle, &ball, s, 2, false, "circle/sphere");
        // A parallel of the sphere, and a circle touching it from outside.
        let parallel = Curve3::Circle(
            Circle::new(f.at(f.point(0.0, 0.0, s)).unwrap(), 8.0_f64.sqrt() * s).unwrap(),
        );
        assert!(on_surface(&parallel, &ball, s));
        let touching = Curve3::Circle(
            Circle::new(
                Frame::new(f.point(5.0 * s, 0.0, 0.0), f.y(), f.x()).unwrap(),
                2.0 * s,
            )
            .unwrap(),
        );
        let found = hits_of(&touching, &ball, &t);
        assert!(found.len() == 1 && found[0].tangent);
        assert!(found[0].point.distance(f.point(3.0 * s, 0.0, 0.0)) <= 2.0 * (t.linear * s).sqrt());
        let far =
            Curve3::Circle(Circle::new(f.at(f.point(0.0, 0.0, 9.0 * s)).unwrap(), s).unwrap());
        count(&far, &ball, s, 0, false, "circle misses sphere");

        let ellipse = Curve3::Ellipse(Ellipse::new(leaning, 3.0 * s, 1.2 * s).unwrap());
        check(&ellipse, &ball, s, "ellipse/sphere");
        let c = cone(f, 1.5 * s, 0.4);
        let ring = torus(f, 4.0 * s, s);
        let crossing = cylinder(Frame::new(f.origin(), f.x(), f.z()).unwrap(), s);
        let mut total = 0;
        for (name, surface) in [("cone", &c), ("torus", &ring), ("cylinder", &crossing)] {
            total += check(&circle, surface, s, &format!("circle/{name}")).len();
            total += check(&ellipse, surface, s, &format!("ellipse/{name}")).len();
        }
        assert!(total >= 8, "only {total} hits");

        // Circles about the axis: on the surface or not at all.
        let about = |z: f64, r: f64| {
            Curve3::Circle(Circle::new(f.at(f.point(0.0, 0.0, z * s)).unwrap(), r * s).unwrap())
        };
        assert!(on_surface(&about(2.0, 1.5 + 2.0 * 0.4_f64.ptan()), &c, s));
        count(
            &about(2.0, 1.0),
            &c,
            s,
            0,
            false,
            "coaxial circle inside cone",
        );
        assert!(on_surface(&about(1.0, 4.0), &ring, s));
        assert!(on_surface(&about(0.0, 5.0), &ring, s));
        count(
            &about(0.5, 4.0),
            &ring,
            s,
            0,
            false,
            "coaxial circle inside tube",
        );
        // A Villarceau circle lies on the torus.
        let lean = 0.25_f64.pasin();
        let villarceau = Curve3::Circle(
            Circle::new(
                Frame::new(
                    f.point(0.0, s, 0.0),
                    f.vector(-lean.psin(), 0.0, lean.pcos()),
                    f.y(),
                )
                .unwrap(),
                4.0 * s,
            )
            .unwrap(),
        );
        assert!(on_surface(&villarceau, &ring, s));
    }
}

#[test]
fn every_curve_meets_freeform_surfaces_and_nurbs_curves_meet_everything() {
    for s in SCALES {
        let f = tilted(s);
        let patch = Surface::Bezier(bumpy(s));
        let nurbs = Surface::Nurbs(rational(s));
        let mut total = 0;
        for (name, surface) in [("bezier", &patch), ("nurbs", &nurbs)] {
            let down = line(Vec3::new(1.3, 2.1, 5.0) * s, Vec3::new(0.05, -0.02, -1.0));
            count(
                &down,
                surface,
                s,
                1,
                false,
                &format!("line down through {name}"),
            );
            let level = line(Vec3::new(-2.0, 1.9, 0.33) * s, Vec3::new(1.0, 0.07, 0.01));
            total += check(&level, surface, s, &format!("level line/{name}")).len();
            let hoop = Frame::new(
                Vec3::new(1.5, 2.0, 0.2) * s,
                Vec3::new(0.1, 1.0, 0.1),
                Vec3::X,
            )
            .unwrap();
            let circle = Curve3::Circle(Circle::new(hoop, 1.1 * s).unwrap());
            total += check(&circle, surface, s, &format!("circle/{name}")).len();
            let ellipse = Curve3::Ellipse(Ellipse::new(hoop, 1.4 * s, 0.7 * s).unwrap());
            total += check(&ellipse, surface, s, &format!("ellipse/{name}")).len();
            count(
                &line(Vec3::new(0.0, 0.0, 9.0) * s, Vec3::X),
                surface,
                s,
                0,
                false,
                "line above",
            );
        }
        assert!(total >= 8, "only {total} hits on the patches");

        // A cubic with weights and a double knot, and the rational circle.
        let wiggle = Curve3::Nurbs(
            NurbsCurve::new(
                3,
                vec![0.0, 0.0, 0.0, 0.0, 0.3, 0.6, 0.6, 1.0, 1.0, 1.0, 1.0],
                [
                    (-3.0, -2.0, -1.0),
                    (-1.0, 3.0, 1.5),
                    (0.5, -3.0, 2.0),
                    (2.0, 2.5, -1.5),
                    (3.0, 4.0, 0.0),
                    (4.5, 1.0, 2.5),
                    (6.0, -1.0, 1.0),
                ]
                .iter()
                .map(|&(x, y, z)| f.point(x * s, y * s, z * s))
                .collect(),
                vec![1.0, 2.0, 0.5, 1.0, 3.0, 1.0, 0.7],
            )
            .unwrap(),
        );
        let round = Curve3::Nurbs(
            NurbsCurve::circle(
                &Frame::new(
                    f.point(2.0 * s, 0.0, 0.5 * s),
                    f.vector(0.3, 1.0, 0.2),
                    f.x(),
                )
                .unwrap(),
                2.0 * s,
            )
            .unwrap(),
        );
        let analytic = [
            plane(f),
            cylinder(f, 2.0 * s),
            cone(f, 1.5 * s, 0.4),
            sphere(f, 3.0 * s),
            torus(f, 4.0 * s, s),
        ];
        let mut total = 0;
        for surface in &analytic {
            total += check(
                &wiggle,
                surface,
                s,
                &format!("nurbs curve/{}", surface.kind()),
            )
            .len();
            total += check(
                &round,
                surface,
                s,
                &format!("nurbs circle/{}", surface.kind()),
            )
            .len();
        }
        assert!(total >= 12, "only {total} hits of the NURBS curves");
        // The rational circle on the cylinder and the sphere it is a
        // section of.
        let section = Curve3::Nurbs(
            NurbsCurve::circle(&f.at(f.point(0.0, 0.0, s)).unwrap(), 2.0 * s).unwrap(),
        );
        assert!(on_surface(&section, &cylinder(f, 2.0 * s), s));
        assert!(on_surface(&section, &sphere(f, 5.0_f64.sqrt() * s), s));
        assert!(on_surface(
            &section,
            &plane(f.at(f.point(s, s, s)).unwrap()),
            s
        ));
    }
    // NURBS curves against the patches, at one scale (slow to check).
    let patch = Surface::Bezier(bumpy(1.0));
    let across = Curve3::Nurbs(
        NurbsCurve::polynomial(
            3,
            vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
            vec![
                Vec3::new(-0.5, 0.5, -1.0),
                Vec3::new(1.0, 4.0, 2.5),
                Vec3::new(2.0, 0.0, -2.0),
                Vec3::new(3.5, 4.0, 1.5),
            ],
        )
        .unwrap(),
    );
    assert!(!check(&across, &patch, 1.0, "nurbs curve/bezier").is_empty());
    assert!(
        !check(
            &across,
            &Surface::Nurbs(rational(1.0)),
            1.0,
            "nurbs curve/nurbs"
        )
        .is_empty()
    );
}

fn random_curve(kind: usize, rng: &mut Rng) -> Curve3 {
    match kind {
        0 => line(rng.vec(2.0), rng.frame(1.0).z()),
        1 => Curve3::Circle(Circle::new(rng.frame(2.0), rng.range(1.0, 3.5)).unwrap()),
        2 => Curve3::Ellipse(
            Ellipse::new(rng.frame(2.0), rng.range(2.0, 4.0), rng.range(0.6, 2.0)).unwrap(),
        ),
        _ => {
            let ctrl: Vec<Vec3> = (0..6).map(|_| rng.vec(4.0)).collect();
            let weights = (0..6).map(|_| rng.range(0.6, 1.8)).collect();
            Curve3::Nurbs(
                NurbsCurve::new(
                    3,
                    vec![0.0, 0.0, 0.0, 0.0, 0.35, 0.7, 1.0, 1.0, 1.0, 1.0],
                    ctrl,
                    weights,
                )
                .unwrap(),
            )
        }
    }
}

fn random_surface(kind: usize, rng: &mut Rng) -> Surface {
    match kind {
        0 => plane(rng.frame(1.5)),
        1 => cylinder(rng.frame(2.0), rng.range(0.5, 2.5)),
        2 => {
            let side = if rng.next().is_multiple_of(2) {
                1.0
            } else {
                -1.0
            };
            cone(
                rng.frame(2.0),
                rng.range(0.0, 1.5),
                side * rng.range(0.2, 0.9),
            )
        }
        3 => sphere(rng.frame(2.0), rng.range(1.0, 3.0)),
        4 => {
            let major = rng.range(1.5, 3.0);
            torus(rng.frame(2.0), major, rng.range(0.3, 0.7 * major))
        }
        kind => {
            let f = rng.frame(1.5);
            let base = if kind == 5 {
                Surface::Bezier(bumpy(1.0))
            } else {
                Surface::Nurbs(rational(1.0))
            };
            let place = craft3d_math::Transform::translation(f.origin() - Vec3::new(1.5, 2.0, 0.0));
            base.transformed(&place).unwrap()
        }
    }
}

#[test]
fn random_curves_against_random_surfaces_of_every_kind() {
    const CURVES: [&str; 4] = ["line", "circle", "ellipse", "nurbs"];
    const SURFACES: [&str; 7] = [
        "plane", "cylinder", "cone", "sphere", "torus", "bezier", "nurbs",
    ];
    let t = tol(1.0);
    let mut rows = Vec::new();
    let (mut refused_total, mut total) = (0, 0);
    for i in 0..CURVES.len() {
        for j in 0..SURFACES.len() {
            let cases = if j >= 5 { 4 } else { 10 };
            let mut rng = Rng(0xC0FFEE + 100 * i as u64 + j as u64);
            let (mut solved, mut hits, mut slowest) = (0, 0, 0.0_f64);
            let mut refused: BTreeMap<String, usize> = BTreeMap::new();
            for case in 0..cases {
                let (mut curve, surface) = (random_curve(i, &mut rng), random_surface(j, &mut rng));
                if j >= 5 {
                    // A patch is a small target: move the curve so that it
                    // passes near a point of it.
                    let target =
                        surface.eval(rng.range(0.3, 0.7), rng.range(0.3, 0.7)) + rng.vec(0.1);
                    let from = curve.eval(if i == 0 { 0.0 } else { 0.5 });
                    curve = curve
                        .transformed(&craft3d_math::Transform::translation(target - from))
                        .unwrap();
                }
                let label = format!("{} / {} case {case}", CURVES[i], SURFACES[j]);
                let clock = Instant::now();
                let found = intersect_curve_surface(&curve, &surface, &t);
                slowest = slowest.max(clock.elapsed().as_secs_f64() * 1e3);
                total += 1;
                match found {
                    Err(e) => *refused.entry(e.to_string()).or_default() += 1,
                    Ok(CurveIntersection::OnSurface) => panic!("{label}: on the surface"),
                    Ok(CurveIntersection::Hits(_)) => {
                        hits += check(&curve, &surface, 1.0, &label).len();
                        solved += 1;
                    }
                }
            }
            refused_total += refused.values().sum::<usize>();
            rows.push((
                format!("{} / {}", CURVES[i], SURFACES[j]),
                solved,
                hits,
                refused,
                slowest,
            ));
        }
    }
    eprintln!("pair                 solved  hits  refused  slowest ms");
    for (pair, solved, hits, refused, slowest) in &rows {
        eprintln!(
            "{pair:<20}{solved:>7}{hits:>6}{:>9}{slowest:>12.2}",
            refused.values().sum::<usize>()
        );
        for (reason, count) in refused {
            eprintln!("    {count} x {reason}");
        }
    }
    eprintln!("{refused_total} of {total} cases refused");
    assert!(refused_total * 10 <= total);
    assert!(rows.iter().all(|row| row.2 > 0), "a pair never met");
}

#[test]
fn hostile_input_is_an_error_or_a_plain_answer() {
    let f = tilted(1.0);
    let ring = torus(f, 4.0, 1.0);
    let circle =
        Curve3::Circle(Circle::new(Frame::new(f.origin(), f.x(), f.z()).unwrap(), 4.5).unwrap());
    for bad in [f64::NAN, 0.0, -1.0, f64::INFINITY] {
        let t = Tolerance {
            linear: bad,
            angular: 1e-9,
        };
        assert!(intersect_curve_surface(&circle, &ring, &t).is_err());
    }
    // A line in the plane of a flat patch: on it for a stretch, off it
    // beyond its edge. Neither a hit list nor "on the surface" is true.
    let flat = Surface::Bezier(
        BezierPatch::new(
            1,
            1,
            vec![Vec3::ZERO, Vec3::Y, Vec3::X, Vec3::new(1.0, 1.0, 0.0)],
        )
        .unwrap(),
    );
    let within = line(Vec3::new(-3.0, 0.4, 0.0), Vec3::X);
    assert!(matches!(
        intersect_curve_surface(&within, &flat, &tol(1.0)),
        Err(GeomError::NoConvergence(_))
    ));
    // A patch collapsed to a point.
    let dot = Surface::Bezier(BezierPatch::new(1, 1, vec![Vec3::new(0.3, 0.2, 0.1); 4]).unwrap());
    assert!(!matches!(
        intersect_curve_surface(&circle, &dot, &tol(1.0)),
        Ok(CurveIntersection::OnSurface)
    ));
}

/// A crossing exactly at a closed curve's seam (its parameter 0, which is
/// also its parameter 2π) is a hit like any other. The two evaluations of
/// that one point differ in the last bit, and with them the sign of the
/// distance: read on its own each side of the seam then saw no change of
/// sign, and the hit was lost (found as an invalid boolean of two slots
/// whose round ends cross at right angles).
#[test]
fn a_crossing_at_the_seam_of_a_circle_is_found() {
    for s in [1e-3, 1.0, 1e6] {
        let t = Tolerance::new(1e-6 * s, 1e-9).unwrap();
        // The cylinder about z through (0.5, 0.5), radius 0.5; a circle of
        // radius 0.75 in the plane y = 0.5 about (1, 0.5, 1), whose
        // parameter 0 is at the top, (1, 0.5, 1.75): there and at the
        // bottom it passes through the cylinder's ruling x = 1.
        let cylinder = Surface::Cylinder(
            craft3d_geom::Cylinder::new(
                Frame::WORLD.at(Vec3::new(0.5, 0.5, -0.5) * s).unwrap(),
                0.5 * s,
            )
            .unwrap(),
        );
        let frame = Frame::new(Vec3::new(1.0, 0.5, 1.0) * s, Vec3::Y, Vec3::Z).unwrap();
        assert_eq!(frame.x(), Vec3::Z);
        let circle = Curve3::Circle(Circle::new(frame, 0.75 * s).unwrap());
        let hits = hits_of(&circle, &cylinder, &t);
        let near = |p: Vec3| {
            hits.iter()
                .filter(|h| h.point.distance(p) < 1e-9 * s)
                .count()
        };
        assert_eq!(hits.len(), 2, "at scale {s}: {hits:?}");
        assert_eq!(near(Vec3::new(1.0, 0.5, 1.75) * s), 1, "{hits:?}");
        assert_eq!(near(Vec3::new(1.0, 0.5, 0.25) * s), 1, "{hits:?}");
        assert!(hits.iter().all(|h| !h.tangent));
        // The same with the seam turned to every multiple of a sixteenth
        // of a turn, where the first intervals of the search meet.
        for k in 0..16 {
            let angle = std::f64::consts::TAU * f64::from(k) / 16.0;
            let x = Vec3::Z * angle.pcos() + Vec3::X * angle.psin();
            let frame = Frame::new(Vec3::new(1.0, 0.5, 1.0) * s, Vec3::Y, x).unwrap();
            let circle = Curve3::Circle(Circle::new(frame, 0.75 * s).unwrap());
            let hits = hits_of(&circle, &cylinder, &t);
            assert_eq!(
                hits.len(),
                2,
                "seam turned by {k}/16 at scale {s}: {hits:?}"
            );
        }
    }
}
