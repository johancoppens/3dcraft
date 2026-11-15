//! Curves that pass near a cone's apex or a sphere's or a revolved
//! spline's pole: proven when they lie on the surface, never when they
//! are off it (`SurfaceBound::curve_within`'s careful second proof).
//!
//! Every curve is exact (a conic as the rational quadratic B-spline a
//! STEP writer gives, a circle, an ellipse) or off by a known amount, and
//! its distance from the surface is held to the closed form here: the
//! distance from a double cone in the half-plane through its axis, from
//! a sphere by its centre.

use super::*;
use crate::{Circle, Cone, Ellipse, NurbsSurface, Sphere};
use craft3d_math::Frame;
use std::f64::consts::PI;

/// A frame at the origin, or turned and a thousand sizes from it.
fn placed(k: f64, turned: bool) -> Frame {
    if turned {
        Frame::new(
            Vec3::new(1.0, -2.0, 3.0) * (1e3 * k),
            Vec3::new(0.3, -0.5, 0.8),
            Vec3::new(1.0, 0.2, 0.1),
        )
        .unwrap()
    } else {
        Frame::WORLD
    }
}

/// A cone of radius `k` at `v = 0` and half-angle `alpha` on `frame`, and
/// the frame moved to its apex.
fn cone(frame: Frame, k: f64, alpha: f64) -> (Surface, Frame) {
    let c = Cone::new(frame, k, alpha).unwrap();
    let apex = frame.at(frame.point(0.0, 0.0, c.apex_v())).unwrap();
    (Surface::Cone(c), apex)
}

/// The distance from the double cone of half-angle `alpha` with its apex
/// at `apex`'s origin and its axis along `apex`'s z.
fn cone_distance(apex: &Frame, alpha: f64, p: Vec3) -> f64 {
    let (x, y, z) = apex.local(p);
    let r = x.phypot(y);
    let (s, c) = alpha.psin_cos();
    let ray = |wr: f64, wz: f64| {
        if r * wr + z * wz >= 0.0 {
            (r * wz - z * wr).abs()
        } else {
            r.phypot(z)
        }
    };
    ray(s, c).min(ray(s, -c))
}

/// Rational quadratic Bézier pieces joined into one B-spline.
fn joined(pieces: &[([Vec3; 3], f64)]) -> NurbsCurve {
    let mut points = Vec::new();
    let mut weights = Vec::new();
    let mut knots = vec![0.0; 3];
    for (i, (p, w)) in pieces.iter().enumerate() {
        if i == 0 {
            points.push(p[0]);
            weights.push(1.0);
        }
        points.extend([p[1], p[2]]);
        weights.extend([*w, 1.0]);
        let m = if i + 1 == pieces.len() { 3 } else { 2 };
        knots.extend(std::iter::repeat_n((i + 1) as f64, m));
    }
    NurbsCurve::new(2, knots, points, weights).unwrap()
}

/// The hyperbola where the plane `y = d sin α` (apex-local) cuts the
/// cone, its vertex `d` from the apex, over `u ∈ [u0, u1]` of
/// `(δ sinh u, δ, δ cosh u / tan α)`, in `n` equal rational spans (the
/// middle point `C + E(m) / cosh h`, weight `cosh h`).
fn hyperbola(apex: &Frame, alpha: f64, d: f64, (u0, u1): (f64, f64), n: usize) -> NurbsCurve {
    let delta = d * alpha.psin();
    let a = delta / alpha.ptan();
    let at = |u: f64| apex.point(delta * u.psinh(), delta, a * u.pcosh());
    let step = (u1 - u0) / n as f64;
    let pieces: Vec<_> = (0..n)
        .map(|i| {
            let lo = u0 + step * i as f64;
            let hi = if i + 1 == n { u1 } else { lo + step };
            let (m, h) = (0.5 * (lo + hi), 0.5 * (hi - lo));
            let w = h.pcosh();
            let middle = apex.point(delta * m.psinh() / w, delta, a * m.pcosh() / w);
            ([at(lo), middle, at(hi)], w)
        })
        .collect();
    joined(&pieces)
}

/// The parabola where a plane parallel to a ruling cuts the cone, its
/// vertex `d` from the apex: `(q (t² − 1) / 2, q t, q (t² + 1) / (2 tan α))`
/// with `q = 2 d sin α`, over `[t0, t1]` in `n` equal spans.
fn parabola(apex: &Frame, alpha: f64, d: f64, (t0, t1): (f64, f64), n: usize) -> NurbsCurve {
    let q = 2.0 * d * alpha.psin();
    let ta = alpha.ptan();
    let polar = |a: f64, b: f64| {
        apex.point(
            q * (a * b - 1.0) / 2.0,
            q * 0.5 * (a + b),
            q * (a * b + 1.0) / (2.0 * ta),
        )
    };
    let step = (t1 - t0) / n as f64;
    let pieces: Vec<_> = (0..n)
        .map(|i| {
            let lo = t0 + step * i as f64;
            let hi = if i + 1 == n { t1 } else { lo + step };
            ([polar(lo, lo), polar(lo, hi), polar(hi, hi)], 1.0)
        })
        .collect();
    joined(&pieces)
}

/// The ellipse where the plane `z = z0 + m x` (apex-local, `m tan α = e`)
/// cuts the cone, its near vertex `d` from the apex.
fn ellipse(apex: &Frame, alpha: f64, d: f64, e: f64) -> Ellipse {
    let ta = alpha.ptan();
    let near = d * alpha.psin() * (1.0 + e);
    let m = e / ta;
    let centre_x = near * e / (1.0 - e * e);
    let centre = apex.point(centre_x, 0.0, near / ta + m * centre_x);
    let frame = Frame::new(centre, apex.vector(-m, 0.0, 1.0), apex.vector(1.0, 0.0, m)).unwrap();
    let semi = near / (1.0 - e * e) * (1.0 + m * m).sqrt();
    Ellipse::new(frame, semi, near / (1.0 - e * e).sqrt()).unwrap()
}

fn domain_of(curve: &Curve3) -> (f64, f64) {
    match curve {
        Curve3::Nurbs(n) => n.domain(),
        _ => (0.0, 2.0 * PI),
    }
}

/// The farthest of 4001 places of the curve from the surface, by the
/// closed form `distance`.
fn gap(curve: &Curve3, range: (f64, f64), distance: &dyn Fn(Vec3) -> f64) -> f64 {
    (0..=4000)
        .map(|i| {
            let t = range.0 + (range.1 - range.0) * f64::from(i) / 4000.0;
            distance(curve.eval(t))
        })
        .fold(0.0, f64::max)
}

/// The proof as [`curve_on_surface`] makes it, and whether the first
/// proof (feet followed by Gauss-Newton steps alone) proved it too.
fn proved(
    curve: &Curve3,
    range: (f64, f64),
    surface: &Surface,
    target: f64,
) -> (Result<Proven, NotProven>, bool) {
    let cb = CurveBound::new(curve);
    let sb = SurfaceBound::new(surface).unwrap();
    let start = surface.project(curve.eval(range.0)).unwrap();
    let start = sb.off_singular(&cb, range, start, Borders::Any);
    let first = sb
        .prove(&cb, range, start, &[], target, 1 << 16, Feet::Followed)
        .is_ok();
    let within = Within {
        target,
        most: 1 << 16,
    };
    (
        curve_on_surface(&cb, range, surface, None, &[], within),
        first,
    )
}

/// The conics, circles and ellipses of a cone's sections that pass by
/// its apex.
fn cone_curves(apex: &Frame, alpha: f64, d: f64, size: f64) -> Vec<(String, Curve3)> {
    let u1 = (size / d).pasinh().max(0.5);
    let t1 = (size / d - 1.0).max(0.25).sqrt();
    let mut out = Vec::new();
    for n in [1, 2] {
        out.push((
            format!("hyperbola in {n}"),
            Curve3::Nurbs(hyperbola(apex, alpha, d, (-u1, u1), n)),
        ));
        out.push((
            format!("hyperbola from its vertex in {n}"),
            Curve3::Nurbs(hyperbola(apex, alpha, d, (0.0, u1), n)),
        ));
        out.push((
            format!("parabola in {n}"),
            Curve3::Nurbs(parabola(apex, alpha, d, (-t1, t1), n)),
        ));
        out.push((
            format!("parabola across its vertex in {n}"),
            Curve3::Nurbs(parabola(apex, alpha, d, (-0.3 * t1, t1), n)),
        ));
    }
    out.push((
        "ellipse".to_string(),
        Curve3::Ellipse(ellipse(apex, alpha, d, 0.9)),
    ));
    out.push((
        "circle".to_string(),
        Curve3::Circle(
            Circle::new(
                apex.at(apex.point(0.0, 0.0, d * alpha.pcos())).unwrap(),
                d * alpha.psin(),
            )
            .unwrap(),
        ),
    ));
    out
}

/// Exact sections of cones of 10° to 89° half-angle passing a tenth to a
/// hundred-thousandth of their size from the apex, as one or two long
/// rational spans: proven within a millionth of the size at the scales
/// 1e-3, 1 and 1e6, in the world and turned a thousand sizes from it,
/// the bound above every place's closed-form distance. Most were not
/// proven by the first proof (counted: the test is of the second).
#[test]
fn sections_of_a_cone_that_pass_by_its_apex_are_proven() {
    let (mut cases, mut first_failed) = (0, 0);
    for k in [1e-3, 1.0, 1e6] {
        let target = 1e-6 * k;
        for turned in [false, true] {
            for degrees in [10.0_f64, 60.0, 81.5, 89.0] {
                let alpha = degrees.to_radians();
                let (surface, apex) = cone(placed(k, turned), k, alpha);
                let size = 2.0 * k;
                for share in [1e-1, 1e-3, 1e-5] {
                    for (name, curve) in cone_curves(&apex, alpha, share * size, size) {
                        let range = domain_of(&curve);
                        let what = format!("{name}, k {k}, turned {turned}, {degrees}°, {share}");
                        // One span of a hyperbola from a hundred-thousandth
                        // of the size has a weight of 1e5: a thousand sizes
                        // from the origin the rounding allowed for its
                        // evaluation is above the target by itself.
                        let rounding = ROUNDING_ULPS * f64::EPSILON * CurveBound::new(&curve).size;
                        if rounding > 0.5 * target {
                            assert!(turned && name == "hyperbola in 1", "{what}");
                            continue;
                        }
                        let (proof, first) = proved(&curve, range, &surface, target);
                        let proof = proof.unwrap_or_else(|n| panic!("{what}: not proven at {n:?}"));
                        let seen = gap(&curve, range, &|p| cone_distance(&apex, alpha, p));
                        assert!(
                            proof.bound <= target && proof.bound >= seen,
                            "{what}: {proof:?}, seen {seen:e}"
                        );
                        cases += 1;
                        first_failed += usize::from(!first);
                    }
                }
            }
        }
    }
    assert!(first_failed * 4 > cases, "{first_failed} of {cases}");
}

/// The same hyperbola and parabola lifted off the cone along its normal
/// by 1.1, 2 and 10 times the target under a bump about the vertex
/// (cubic Hermite pieces through the lifted points), outward and inward,
/// a tenth to a hundred-thousandth of the size from the apex: never
/// proven within the target; lifted by half the target, proven, the
/// bound above the closed-form distance.
#[test]
fn sections_lifted_off_a_cone_by_its_apex_are_never_proven_on_it() {
    let hermite = |f: &dyn Fn(f64) -> Vec3, (t0, t1): (f64, f64), n: usize| {
        let step = (t1 - t0) / n as f64;
        let e = step * 1e-4;
        let tangent = |t: f64| (f(t + e) - f(t - e)) * (1.0 / (2.0 * e));
        let mut points = vec![f(t0)];
        let mut knots = vec![0.0; 4];
        for i in 0..n {
            let (lo, hi) = (t0 + step * i as f64, t0 + step * (i + 1) as f64);
            points.extend([
                f(lo) + tangent(lo) * (step / 3.0),
                f(hi) - tangent(hi) * (step / 3.0),
                f(hi),
            ]);
            let m = if i + 1 == n { 4 } else { 3 };
            knots.extend(std::iter::repeat_n((i + 1) as f64, m));
        }
        NurbsCurve::polynomial(3, knots, points).unwrap()
    };
    let mut proven_off = Vec::new();
    for k in [1e-3, 1.0, 1e6] {
        let target = 1e-6 * k;
        for turned in [false, true] {
            for degrees in [10.0_f64, 81.5] {
                let alpha = degrees.to_radians();
                let (s, c) = alpha.psin_cos();
                let (surface, apex) = cone(placed(k, turned), k, alpha);
                let normal = |p: Vec3| {
                    let (x, y, _) = apex.local(p);
                    let r = x.phypot(y);
                    apex.vector(c * x / r, c * y / r, -s)
                };
                let size = 2.0 * k;
                for share in [1e-1, 1e-3, 1e-5] {
                    let d = share * size;
                    let u1 = (size / d).pasinh();
                    let t1 = (size / d - 1.0).sqrt();
                    for times in [0.5, 1.1, 2.0, 10.0] {
                        for side in [1.0, -1.0] {
                            let lift =
                                |u: f64| side * times * target * (-(u / 0.3) * (u / 0.3)).pexp();
                            let delta = d * s;
                            let hyp = |u: f64| {
                                let p = apex.point(
                                    delta * u.psinh(),
                                    delta,
                                    delta * u.pcosh() / alpha.ptan(),
                                );
                                p + normal(p) * lift(u)
                            };
                            let q = 2.0 * d * s;
                            let par = |t: f64| {
                                let p = apex.point(
                                    q * (t * t - 1.0) / 2.0,
                                    q * t,
                                    q * (t * t + 1.0) / (2.0 * alpha.ptan()),
                                );
                                p + normal(p) * lift(t)
                            };
                            for (name, curve) in [
                                ("hyperbola", hermite(&hyp, (-u1, u1), 256)),
                                ("parabola", hermite(&par, (-t1, t1), 256)),
                            ] {
                                let curve = Curve3::Nurbs(curve);
                                let range = domain_of(&curve);
                                let what = format!(
                                    "{name} {times} x {side}, k {k}, turned {turned}, {degrees}°, {share}"
                                );
                                let seen = gap(&curve, range, &|p| cone_distance(&apex, alpha, p));
                                match proved(&curve, range, &surface, target).0 {
                                    Ok(p) if times > 1.0 || p.bound < seen => {
                                        proven_off.push(format!("{what}: {p:?}, seen {seen:e}"));
                                    }
                                    Err(n) if times < 1.0 => panic!("{what}: not proven at {n:?}"),
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(proven_off.is_empty(), "{proven_off:#?}");
}

/// The upper half of a sphere of radius `r` about `frame`'s origin as a
/// rational spline turned about its z (its first row of control points
/// one point: a pole), its seam at the azimuth 0.
fn dome(frame: &Frame, r: f64) -> Surface {
    let s = std::f64::consts::FRAC_1_SQRT_2;
    let ring = [
        (1.0, 0.0, 1.0),
        (1.0, 1.0, s),
        (0.0, 1.0, 1.0),
        (-1.0, 1.0, s),
        (-1.0, 0.0, 1.0),
        (-1.0, -1.0, s),
        (0.0, -1.0, 1.0),
        (1.0, -1.0, s),
        (1.0, 0.0, 1.0),
    ];
    let arc = [(0.0, r, 1.0), (r, r, s), (r, 0.0, 1.0)];
    let mut points = Vec::new();
    let mut weights = Vec::new();
    for (x, y, w) in ring {
        for (rho, z, own) in arc {
            points.push(frame.point(rho * x, rho * y, z));
            weights.push(w * own);
        }
    }
    let knots = vec![
        0.0, 0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0, 1.0,
    ];
    Surface::Nurbs(
        NurbsSurface::new(
            2,
            2,
            knots,
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            9,
            3,
            points,
            weights,
        )
        .unwrap(),
    )
}

/// Arcs that pass a hundredth to a millionth of the radius from a pole,
/// on a sphere and on the same half sphere as a revolved rational spline
/// (away from its seam): a great circle over the pole (on the spline, its
/// arc over the half), the section by a plane parallel to the axis, and
/// that section as one and as two rational spans; proven within a
/// millionth of the size at three scales, in the world and turned a
/// thousand sizes from it, the bound above every place's closed-form
/// distance. The first proof proved none of them.
#[test]
fn arcs_that_pass_by_a_pole_are_proven() {
    let (mut cases, mut first_failed) = (0, 0);
    for k in [1e-3, 1.0, 1e6] {
        let target = 1e-6 * k;
        let r = 2.0 * k;
        for turned in [false, true] {
            let frame = placed(k, turned);
            let sphere = Surface::Sphere(Sphere::new(frame, r).unwrap());
            let spline = dome(&frame, r);
            let distance = |p: Vec3| (p.distance(frame.origin()) - r).abs();
            for share in [1e-2, 1e-4, 1e-6] {
                // Over the pole at the azimuth π: clear of the spline's seam.
                let delta = share;
                let top = frame.vector(-delta.psin(), 0.0, delta.pcos());
                let normal = frame.vector(delta.pcos(), 0.0, delta.psin());
                let great = Frame::new(frame.origin(), normal, top).unwrap();
                let off = r * delta.psin();
                let cut = Frame::new(frame.point(-off, 0.0, 0.0), frame.x(), frame.z()).unwrap();
                let small = (r * r - off * off).sqrt();
                let at = |t: f64| cut.point(small * t.pcos(), small * t.psin(), 0.0);
                let span = |lo: f64, hi: f64| {
                    let (m, h) = (0.5 * (lo + hi), 0.5 * (hi - lo));
                    let w = h.pcos();
                    let middle = cut.point(small * m.pcos() / w, small * m.psin() / w, 0.0);
                    ([at(lo), middle, at(hi)], w)
                };
                let half = 0.45 * PI;
                let curves: Vec<(&str, Curve3, (f64, f64))> = vec![
                    (
                        "great circle",
                        Curve3::Circle(Circle::new(great, r).unwrap()),
                        (-half, half),
                    ),
                    (
                        "section",
                        Curve3::Circle(Circle::new(cut, small).unwrap()),
                        (-half, half),
                    ),
                    (
                        "section in 1",
                        Curve3::Nurbs(joined(&[span(-half, half)])),
                        (0.0, 1.0),
                    ),
                    (
                        "section in 2",
                        Curve3::Nurbs(joined(&[span(-half, 0.0), span(0.0, half)])),
                        (0.0, 2.0),
                    ),
                ];
                for (surface_name, surface) in [("sphere", &sphere), ("spline", &spline)] {
                    for (name, curve, range) in &curves {
                        let what = format!(
                            "{name} on the {surface_name}, k {k}, turned {turned}, {share}"
                        );
                        let (proof, first) = proved(curve, *range, surface, target);
                        let proof = proof.unwrap_or_else(|n| panic!("{what}: not proven at {n:?}"));
                        let seen = gap(curve, *range, &distance);
                        assert!(
                            proof.bound <= target && proof.bound >= seen,
                            "{what}: {proof:?}, seen {seen:e}"
                        );
                        cases += 1;
                        first_failed += usize::from(!first);
                    }
                }
            }
        }
    }
    assert!(first_failed * 2 > cases, "{first_failed} of {cases}");
}

/// What the first proof proves is the answer, bit for bit: the fitted
/// curves of the other tests on every kind of surface, at three scales.
#[test]
fn a_curve_the_first_proof_proves_is_proven_as_it_was() {
    for k in [1e-3, 1.0, 1e6] {
        for (surface, path) in super::tests::surfaces(k) {
            let (curve, breaks) = super::tests::curve_on(&surface, path, 12, 0.0, k);
            let cb = CurveBound::nurbs(&curve);
            let sb = SurfaceBound::new(&surface).unwrap();
            let range = curve.domain();
            let start = surface.project(curve.eval(range.0)).unwrap();
            for target in [1e-6 * k, 1e-9 * k] {
                let first = sb.prove(&cb, range, start, &breaks, target, 1 << 12, Feet::Followed);
                let whole = sb.curve_within(&cb, range, start, &breaks, target, 1 << 12);
                if let Ok(p) = first {
                    assert_eq!(whole, Ok(p), "{surface:?} at {target:e}");
                }
            }
        }
    }
}
