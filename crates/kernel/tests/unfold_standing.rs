//! The unfold of a strip whose rail stands still at a corner and whose
//! knots crowd towards it, as the strip beside a fan at a fold does (its
//! rail stands still where the zero set turned, and the walk's rulings,
//! the fitted face's knots, crowd there): docs/design/developable.md ›
//! *Distances in the layout* (span by span). Held by a strip of a cone of
//! half-angle 45° about its apex `O`, `S(u, v) = O + t(u, v) g(x(u))` with
//! `g(x) = (x, (x² − 1)/2, (x² + 1)/2)` (the cone's section by the plane
//! `z = y + 1`, a parabola) and `t = (1 − v) q(u) + v b`, where the lower
//! row's `q` is a C¹ piecewise quadratic on spans that halve towards `u =
//! 1` (its second derivative jumping between 1 and 3 at every knot) with
//! `q(1) = q'(1) = 0`: the lower row runs into the apex and stands still
//! there. The face is a B-spline of degree 4 by 1, exactly on the cone, so
//! laid flat it is the polar region `ρ = t |g|`, `ψ = φ(x)/√2` (`φ =
//! atan2((x² − 1)/2, x)`, `|g| = (x² + 1)/√2`), and its area, its edges'
//! lengths and the flat distance between any two of its points are known in
//! closed form. At three scales, far from the origin.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::{Curve3, Line, NurbsCurve, NurbsSurface, Surface};
use craft3d_kernel::unfold::develop;
use craft3d_math::Portable;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, FaceId};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn tol(scale: f64) -> Tolerance {
    Tolerance::new(1e-6 * scale, 1e-9).unwrap()
}

/// The strip's constants: `x` from `X0` over `L`, the upper row `b`
/// along the cone's rays, and how many times the spans halve towards
/// `u = 1` (the last one `2⁻¹²` of the face, as the fold strip's spans of
/// 4.5e-4 at its standing corner).
const X0: f64 = -0.5;
const L: f64 = 1.5;
const B: f64 = 2.0;
const HALVINGS: i32 = 12;

/// The cone's apex, away from the origin.
fn apex(k: f64) -> Vec3 {
    Vec3::new(3.0, -2.0, 5.0) * k
}

fn g(x: f64) -> Vec3 {
    Vec3::new(x, 0.5 * (x * x - 1.0), 0.5 * (x * x + 1.0))
}

/// The knots of the lower row: 0, then `1 − 2⁻ⁿ` for n = 1 to
/// [`HALVINGS`], then 1.
fn breaks() -> Vec<f64> {
    let mut out = vec![0.0];
    for n in 1..=HALVINGS {
        out.push(1.0 - 0.5f64.ppowi(n));
    }
    out.push(1.0);
    out
}

/// `q` and `q'` at `u`: integrated back from `q(1) = q'(1) = 0` with `q''`
/// 1 on the last span, 3 on the one before, and so on.
fn lower(u: f64) -> (f64, f64) {
    let knots = breaks();
    let (mut q, mut dq) = (0.0, 0.0);
    let spans = knots.len() - 1;
    for s in (0..spans).rev() {
        let (lo, hi) = (knots[s], knots[s + 1]);
        let a = if (spans - 1 - s).is_multiple_of(2) {
            1.0
        } else {
            3.0
        };
        let to = u.max(lo);
        let h = to - hi;
        q += dq * h + 0.5 * a * h * h;
        dq += a * h;
        if u >= lo {
            break;
        }
    }
    (q, dq)
}

/// The point of the strip at `(u, v)`.
fn point(k: f64, u: f64, v: f64) -> Vec3 {
    let t = (1.0 - v) * lower(u).0 + v * B;
    apex(k) + g(X0 + L * u) * (t * k)
}

/// The point's polar coordinates in the cone's development about its apex.
fn polar(u: f64, v: f64) -> (f64, f64) {
    let x = X0 + L * u;
    let t = (1.0 - v) * lower(u).0 + v * B;
    let rho = t * (x * x + 1.0) / 2f64.sqrt();
    let psi = (0.5 * (x * x - 1.0)).patan2(x) / 2f64.sqrt();
    (rho, psi)
}

/// The flat distance between two points of the strip, times `k`.
fn flat_distance(k: f64, (u1, v1): (f64, f64), (u2, v2): (f64, f64)) -> f64 {
    let (r1, p1) = polar(u1, v1);
    let (r2, p2) = polar(u2, v2);
    (r1 * r1 + r2 * r2 - 2.0 * r1 * r2 * (p1 - p2).pcos())
        .max(0.0)
        .sqrt()
        * k
}

/// The strip's area, `∫ ½ (b² − q²) (x² + 1)/√2 L du`, times `k²`: on
/// each span a polynomial of degree 6 in `u`, so Gauss–Legendre at four
/// points per span gives it exactly (up to rounding).
fn closed_area(k: f64) -> f64 {
    let nodes = [
        (-0.861_136_311_594_052_6, 0.347_854_845_137_453_8),
        (-0.339_981_043_584_856_3, 0.652_145_154_862_546_1),
        (0.339_981_043_584_856_3, 0.652_145_154_862_546_1),
        (0.861_136_311_594_052_6, 0.347_854_845_137_453_8),
    ];
    let mut sum = 0.0;
    for w in breaks().windows(2) {
        let (lo, hi) = (w[0], w[1]);
        for (z, weight) in nodes {
            let u = lo + 0.5 * (z + 1.0) * (hi - lo);
            let x = X0 + L * u;
            let q = lower(u).0;
            let f = 0.5 * (B * B - q * q) * (x * x + 1.0) / 2f64.sqrt() * L;
            sum += 0.5 * (hi - lo) * weight * f;
        }
    }
    sum * k * k
}

/// The length of the upper row, `b ∫ |g'| dx` with `|g'| = √(1 + 2x²)`,
/// times `k`.
fn closed_upper_length(k: f64) -> f64 {
    let primitive = |x: f64| {
        let r2 = 2f64.sqrt();
        0.5 * x * (1.0 + 2.0 * x * x).sqrt() + (r2 * x).pasinh() / (2.0 * r2)
    };
    B * (primitive(X0 + L) - primitive(X0)) * k
}

/// The Bernstein coefficients, at degree 4, of the product of a scalar
/// and a point quadratic given by theirs at degree 2.
fn product(q: [f64; 3], p: [Vec3; 3]) -> [Vec3; 5] {
    let c2 = [1.0, 2.0, 1.0];
    let c4 = [1.0, 4.0, 6.0, 4.0, 1.0];
    let mut out = [Vec3::ZERO; 5];
    for (m, slot) in out.iter_mut().enumerate() {
        for i in 0..3 {
            if m >= i && m - i < 3 {
                let j = m - i;
                *slot += p[j] * (c2[i] * c2[j] / c4[m] * q[i]);
            }
        }
    }
    out
}

/// The strip as a one-face sheet, its upper row lifted off the cone by
/// `twist u (1 − u)` (in units of the strip's size) across the cone (0
/// for the developable strip).
fn strip(k: f64, twist: f64) -> Body {
    let o = apex(k);
    let lift = Vec3::new(0.0, -1.0, 1.0) * (twist * k);
    let knots_at = breaks();
    let (mut lower_ctrl, mut upper_ctrl) = (vec![], vec![]);
    let mut knots = vec![0.0; 5];
    for (s, w) in knots_at.windows(2).enumerate() {
        let (lo, hi) = (w[0], w[1]);
        let d = hi - lo;
        // `g`, `q` and the lift `u (1 − u)` on the span, at degree 2.
        let (x0, x1) = (X0 + L * lo, X0 + L * hi);
        let gs = [g(x0), g(x0) + Vec3::new(1.0, x0, x0) * (0.5 * L * d), g(x1)];
        let (q0, dq0) = lower(lo);
        let qs = [q0, q0 + 0.5 * d * dq0, lower(hi).0];
        let f = |u: f64| u * (1.0 - u);
        let fs = [f(lo), f(lo) + 0.5 * d * (1.0 - 2.0 * lo), f(hi)];
        let low = product(qs, gs);
        let up = product([1.0; 3], gs);
        let lifted = product(fs, [lift; 3]);
        for m in 0..5 {
            if s > 0 && m == 0 {
                continue;
            }
            lower_ctrl.push(o + low[m] * k);
            upper_ctrl.push(o + up[m] * (B * k) + lifted[m]);
        }
        let end = if s + 2 == knots_at.len() { 5 } else { 4 };
        knots.extend(std::iter::repeat_n(hi, end));
    }
    let count = lower_ctrl.len();
    let mut ctrl = Vec::new();
    for (p, q) in lower_ctrl.iter().zip(&upper_ctrl) {
        ctrl.push(*p);
        ctrl.push(*q);
    }
    let surface = NurbsSurface::new(
        4,
        1,
        knots.clone(),
        vec![0.0, 0.0, 1.0, 1.0],
        count,
        2,
        ctrl,
        vec![1.0; 2 * count],
    )
    .unwrap();
    let a = NurbsCurve::polynomial(4, knots.clone(), lower_ctrl.clone()).unwrap();
    let b = NurbsCurve::polynomial(4, knots, upper_ctrl.clone()).unwrap();
    let mut body = Body::new(BodyKind::Sheet);
    let (a0, a1) = (lower_ctrl[0], lower_ctrl[count - 1]);
    let (b0, b1) = (upper_ctrl[0], upper_ctrl[count - 1]);
    let (va0, va1, vb1, vb0) = (
        body.add_vertex(a0),
        body.add_vertex(a1),
        body.add_vertex(b1),
        body.add_vertex(b0),
    );
    let ea = body.add_edge(Curve3::Nurbs(a), (0.0, 1.0), va0, va1);
    let end = body.add_edge(
        Curve3::Line(Line::through(a1, b1).unwrap()),
        (0.0, a1.distance(b1)),
        va1,
        vb1,
    );
    let eb = body.add_edge(Curve3::Nurbs(b), (0.0, 1.0), vb0, vb1);
    let start = body.add_edge(
        Curve3::Line(Line::through(a0, b0).unwrap()),
        (0.0, a0.distance(b0)),
        va0,
        vb0,
    );
    let lp = body.add_loop(vec![
        Coedge {
            edge: ea,
            reversed: false,
        },
        Coedge {
            edge: end,
            reversed: false,
        },
        Coedge {
            edge: eb,
            reversed: true,
        },
        Coedge {
            edge: start,
            reversed: true,
        },
    ]);
    let face = body.add_face(Surface::Nurbs(surface), false, vec![lp]);
    body.add_shell(vec![face]);
    body
}

/// The strip is laid flat: its area and every edge's length are those of
/// the cone's development in closed form within 1e-6 relative, the flat
/// distance between pairs of points across the face (along its rays,
/// across them, and diagonally, some next to the standing corner) is the
/// polar one within 1e-6 of the strip's size, and points mapped there and
/// back return. Refused before 2026-11-02 in three ways, each fixed and
/// each failing this test alone when undone: the layout's images
/// integrated across knots ("does not lie flat … do not converge", or
/// "the grid … cannot be laid flat near" the apex), their rates at a
/// piece's end taken at the knot, in the next span (7.7 tolerances between
/// the layouts), and the apex corner's boundary sample given its
/// neighbour's parameter ("the flat image of an edge … departs from the
/// pattern's map by 5.4e-7" at 1e-3).
#[test]
fn a_cone_strip_whose_row_stands_still_at_the_apex_is_laid_flat() {
    for k in SCALES {
        let body = strip(k, 0.0);
        let surface = &body.faces[0].surface;
        for (u, v) in [(0.0, 0.0), (0.3, 0.7), (1.0, 1.0), (0.99, 0.01)] {
            assert!(
                surface.eval(u, v).distance(point(k, u, v)) <= 1e-12 * k * 10.0,
                "{k}: the patch is the cone at ({u}, {v})"
            );
        }
        let dev = develop(&body, &[FaceId(0)], &tol(k)).unwrap_or_else(|e| panic!("{k}: {e}"));
        let e = dev.error();
        eprintln!("{k}: {e:?}");
        assert!(e.map <= 0.5e-6 * k, "{k}: {e:?}");
        assert!(e.length <= 1e-6 && e.area <= 1e-6, "{k}: {e:?}");
        // The pattern's area, measured on the flat sheet, against the
        // closed form.
        let flat = dev.flat_body(&Frame::WORLD, &tol(k)).unwrap();
        let area: f64 = craft3d_topo::face_areas(&flat.body)
            .iter()
            .map(|a| *a.as_ref().unwrap())
            .sum();
        let want = closed_area(k);
        assert!(
            (area - want).abs() <= 1e-6 * want,
            "{k}: the pattern's area {area}, the development's {want}"
        );
        // Every edge keeps its length; the rays and the upper row against
        // their closed forms.
        let rays = [
            (B - lower(0.0).0) * (X0 * X0 + 1.0) / 2f64.sqrt() * k,
            B * ((X0 + L) * (X0 + L) + 1.0) / 2f64.sqrt() * k,
        ];
        let upper = closed_upper_length(k);
        let mut matched = 0;
        for c in dev.outline() {
            assert!(
                (c.length_flat - c.length_3d).abs() <= 1e-6 * c.length_3d,
                "{k}: an edge {} long on the part, {} flat",
                c.length_3d,
                c.length_flat
            );
            for want in rays.iter().chain([&upper]) {
                if (c.length_3d - want).abs() <= 1e-6 * want {
                    assert!((c.length_flat - want).abs() <= 1e-6 * want, "{k}");
                    matched += 1;
                }
            }
        }
        assert_eq!(matched, 3, "{k}: the two rays and the upper row");
        // Distances in the pattern: along rays, across them (one pair on
        // the standing row's side, a ten-thousandth from the apex), and
        // diagonally.
        let to_flat = |(u, v): (f64, f64)| dev.to_flat(FaceId(0), point(k, u, v)).unwrap();
        let pairs = [
            ((0.2, 0.0), (0.2, 1.0)),
            ((0.9999, 0.0), (0.9999, 1.0)),
            ((0.0, 0.5), (1.0, 0.5)),
            ((0.98, 0.001), (0.9999, 0.001)),
            ((0.5, 0.0), (1.0, 0.0)),
            ((0.1, 0.9), (0.95, 0.05)),
            ((0.0, 0.0), (1.0, 1.0)),
        ];
        for (p, q) in pairs {
            let (fp, fq) = (to_flat(p), to_flat(q));
            let d = (fp[0] - fq[0]).phypot(fp[1] - fq[1]);
            let want = flat_distance(k, p, q);
            assert!(
                (d - want).abs() <= 1e-6 * k,
                "{k}: {p:?} to {q:?} is {d} flat, {want} in the development"
            );
        }
        // There and back.
        for p in [(0.2, 0.3), (0.5, 0.9), (0.97, 0.02), (0.8, 0.5)] {
            let x = point(k, p.0, p.1);
            let (face, y) = dev.to_3d(to_flat(p)).unwrap();
            assert_eq!(face, FaceId(0));
            assert!(y.distance(x) <= 1e-6 * k, "{k}: {x:?} came back as {y:?}");
        }
    }
}

/// The same strip with its upper row lifted off the cone by a hundredth
/// of its size (a ruled face that is not developable, its corner still
/// standing and its knots still crowded) is refused, with its curvature,
/// at three scales.
#[test]
fn a_twisted_strip_whose_row_stands_still_is_refused() {
    for k in SCALES {
        let body = strip(k, 1e-2);
        let refused = develop(&body, &[FaceId(0)], &tol(k))
            .map(|d| d.error())
            .expect_err("a twisted strip is not developable");
        let why = refused.to_string();
        assert!(
            why.contains("is not developable: its Gaussian curvature reaches"),
            "{k}: {why}"
        );
    }
}

/// A flat rectangular sheet `w` by `h` as a spline of degree 1 by 1 whose
/// u parameter has `spans` spans, its control points along u at
/// `spacing(i)` (from 0 to 1): a plane, its parametrisation as given.
fn plane_sheet(k: f64, spans: usize, spacing: impl Fn(usize) -> f64) -> Body {
    let (w, h) = (100.0 * k, 30.0 * k);
    let o = Vec3::new(5.0, -3.0, 2.0) * k;
    let mut knots = vec![0.0, 0.0];
    for i in 1..spans {
        knots.push(i as f64 / spans as f64);
    }
    knots.extend([1.0, 1.0]);
    let (mut lower, mut upper) = (Vec::new(), Vec::new());
    for i in 0..=spans {
        let x = spacing(i) * w;
        lower.push(o + Vec3::new(x, 0.0, 0.0));
        upper.push(o + Vec3::new(x, h, 0.0));
    }
    let mut ctrl = Vec::new();
    for (p, q) in lower.iter().zip(&upper) {
        ctrl.push(*p);
        ctrl.push(*q);
    }
    let surface = NurbsSurface::new(
        1,
        1,
        knots.clone(),
        vec![0.0, 0.0, 1.0, 1.0],
        spans + 1,
        2,
        ctrl,
        vec![1.0; 2 * (spans + 1)],
    )
    .unwrap();
    let a = NurbsCurve::polynomial(1, knots.clone(), lower.clone()).unwrap();
    let b = NurbsCurve::polynomial(1, knots, upper.clone()).unwrap();
    let mut body = Body::new(BodyKind::Sheet);
    let (a0, a1, b0, b1) = (lower[0], lower[spans], upper[0], upper[spans]);
    let (va0, va1, vb1, vb0) = (
        body.add_vertex(a0),
        body.add_vertex(a1),
        body.add_vertex(b1),
        body.add_vertex(b0),
    );
    let ea = body.add_edge(Curve3::Nurbs(a), (0.0, 1.0), va0, va1);
    let end = body.add_edge(
        Curve3::Line(Line::through(a1, b1).unwrap()),
        (0.0, h),
        va1,
        vb1,
    );
    let eb = body.add_edge(Curve3::Nurbs(b), (0.0, 1.0), vb0, vb1);
    let start = body.add_edge(
        Curve3::Line(Line::through(a0, b0).unwrap()),
        (0.0, h),
        va0,
        vb0,
    );
    let coedge = |edge, reversed| Coedge { edge, reversed };
    let lp = body.add_loop(vec![
        coedge(ea, false),
        coedge(end, false),
        coedge(eb, true),
        coedge(start, true),
    ]);
    let face = body.add_face(Surface::Nurbs(surface), false, vec![lp]);
    body.add_shell(vec![face]);
    body
}

/// A face whose spline has 65 535 knot spans (the most a spline holds) (review of 2026-11-02: the
/// images are integrated span by span, so a face of very many spans made
/// the unfold's work grow with them) is refused at once, before it is
/// measured or sampled, with how many it has; at three scales.
#[test]
fn a_face_of_very_many_knot_spans_is_refused_at_once() {
    for k in SCALES {
        let spans = (1 << 16) - 1;
        let body = plane_sheet(k, spans, |i| i as f64 / spans as f64);
        let started = std::time::Instant::now();
        let refused = develop(&body, &[FaceId(0)], &tol(k))
            .map(|d| d.error())
            .expect_err("too many spans");
        let took = started.elapsed();
        let why = refused.to_string();
        assert!(why.contains("65535 knot spans"), "{k}: {why}");
        assert!(took.as_secs_f64() < 5.0, "{k}: refused after {took:?}");
    }
}

/// A plane whose parametrisation kinks at an inner knot (degree 1, its
/// control points closer on one side: the derivative along u jumps) is
/// refused by name, as the images are joined across knots in the
/// direction they arrive in; the same plane evenly parametrised is laid
/// flat to its rectangle. At three scales.
#[test]
fn a_face_whose_parametrisation_kinks_at_a_knot_is_refused() {
    for k in SCALES {
        let kinked = plane_sheet(k, 2, |i| [0.0, 0.3, 1.0][i]);
        let refused = develop(&kinked, &[FaceId(0)], &tol(k))
            .map(|d| d.error())
            .expect_err("a kinked parametrisation");
        let why = refused.to_string();
        assert!(
            why.contains("is not smooth in its parameters across the knot u = 0.5"),
            "{k}: {why}"
        );
        let even = plane_sheet(k, 2, |i| [0.0, 0.5, 1.0][i]);
        let dev = develop(&even, &[FaceId(0)], &tol(k)).unwrap_or_else(|e| panic!("{k}: {e}"));
        let flat = dev.flat_body(&Frame::WORLD, &tol(k)).unwrap();
        let area: f64 = craft3d_topo::face_areas(&flat.body)
            .iter()
            .map(|a| *a.as_ref().unwrap())
            .sum();
        let want = 3000.0 * k * k;
        assert!((area - want).abs() <= 1e-9 * want, "{k}: {area}");
    }
}

/// A fan of the same cone, its apex at `o`: `o + v b g(x(u))`, its apex
/// row (v = 0) one point, of size `k`.
fn fan(k: f64, o: Vec3) -> Body {
    let g0 = g(X0);
    let g1 = g0 + Vec3::new(1.0, X0, X0) * (0.5 * L);
    let g2 = g(X0 + L);
    let far = [o + g0 * (B * k), o + g1 * (B * k), o + g2 * (B * k)];
    let knots = vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
    let mut ctrl = Vec::new();
    for q in far {
        ctrl.push(o);
        ctrl.push(q);
    }
    let surface = NurbsSurface::new(
        2,
        1,
        knots.clone(),
        vec![0.0, 0.0, 1.0, 1.0],
        3,
        2,
        ctrl,
        vec![1.0; 6],
    )
    .unwrap();
    let row = NurbsCurve::polynomial(2, knots, far.to_vec()).unwrap();
    let mut body = Body::new(BodyKind::Sheet);
    let (va, v0, v1) = (
        body.add_vertex(o),
        body.add_vertex(far[0]),
        body.add_vertex(far[2]),
    );
    let ray = |body: &mut Body, q: Vec3, end| {
        body.add_edge(
            Curve3::Line(Line::through(o, q).unwrap()),
            (0.0, o.distance(q)),
            va,
            end,
        )
    };
    let (r0, r1) = (ray(&mut body, far[0], v0), ray(&mut body, far[2], v1));
    let e = body.add_edge(Curve3::Nurbs(row), (0.0, 1.0), v0, v1);
    let coedge = |edge, reversed| Coedge { edge, reversed };
    let lp = body.add_loop(vec![coedge(r1, false), coedge(e, true), coedge(r0, true)]);
    let face = body.add_face(Surface::Nurbs(surface), false, vec![lp]);
    body.add_shell(vec![face]);
    body
}

/// A small fan far from the origin (review of 2026-11-02): a part of size
/// 1e-3 whose apex lies 1e3 and 1e4 away (the three-scale tests scale
/// about the origin). Laid flat it is the polar sector of the cone's
/// development: its rays keep their lengths and open by the closed form
/// `Δψ`, a ray's middle lies halfway along its image, every edge keeps
/// its length. The boundary's sample at the apex takes its neighbour's
/// parameter within the rounding of its coordinates as well as a
/// millionth of the way to that neighbour; that floor only decides
/// beyond a ratio of about 1e8 between the distance and the size, where
/// the apex is not told as collapsed (`collapsed_at`, refused at 1e5
/// here: "a boundary that does not close"), so this pins the fan, not
/// the floor.
#[test]
fn a_small_fan_far_from_the_origin_is_laid_flat() {
    let k = 1e-3;
    for o in [Vec3::new(1e3, -7e2, 4e2), Vec3::new(1e4, -7e3, 4e3)] {
        let body = fan(k, o);
        let tolerance = Tolerance::new(1e-6 * k, 1e-9).unwrap();
        let dev = develop(&body, &[FaceId(0)], &tolerance).unwrap_or_else(|e| panic!("{o:?}: {e}"));
        let e = dev.error();
        assert!(e.map <= 0.5e-6 * k && e.length <= 1e-6, "{o:?}: {e:?}");
        let surface = &body.faces[0].surface;
        let flat = |u: f64, v: f64| dev.to_flat(FaceId(0), surface.eval(u, v)).unwrap();
        let apex = flat(0.5, 0.0);
        let (first, last) = (flat(0.0, 1.0), flat(1.0, 1.0));
        let (a, b) = (
            [first[0] - apex[0], first[1] - apex[1]],
            [last[0] - apex[0], last[1] - apex[1]],
        );
        let opened = (a[0] * b[1] - a[1] * b[0])
            .abs()
            .patan2(a[0] * b[0] + a[1] * b[1]);
        let want = (polar(1.0, 1.0).1 - polar(0.0, 1.0).1).abs();
        assert!(
            (opened - want).abs() <= 1e-6,
            "{o:?}: opens by {opened}, the cone's development {want}"
        );
        for (u, ray) in [(0.0, a), (1.0, b)] {
            let length = B * ((X0 + L * u).ppowi(2) + 1.0) / 2f64.sqrt() * k;
            let got = ray[0].phypot(ray[1]);
            assert!(
                (got - length).abs() <= 1e-6 * k,
                "{o:?}: ray {u}: {got} for {length}"
            );
        }
        let (m, end) = (flat(0.4, 0.5), flat(0.4, 1.0));
        let half = [0.5 * (apex[0] + end[0]), 0.5 * (apex[1] + end[1])];
        assert!(
            (m[0] - half[0]).phypot(m[1] - half[1]) <= 1e-6 * k,
            "{o:?}: a ray's middle off its image's"
        );
    }
}

/// The parameters of a degree-1 spline's control points with `crowd`
/// inner knots, all within the first thousandth, and its knot vector.
fn crowded(crowd: usize) -> (Vec<f64>, Vec<f64>) {
    let mut at: Vec<f64> = vec![0.0];
    for i in 1..=crowd {
        at.push(1e-3 * i as f64 / (crowd + 1) as f64);
    }
    at.push(1.0);
    let mut knots = vec![0.0];
    knots.extend(at.iter().copied());
    knots.push(1.0);
    (at, knots)
}

/// A flat `w` by `h` sheet as a spline of degree 1 by 1 whose inner knots
/// crowd into the first thousandth of each parameter, `in_u` and `in_v`
/// of them (its control points at the knots, so the parametrisation is
/// linear and smooth across them).
fn crowded_plane(k: f64, in_u: usize, in_v: usize) -> Body {
    let (w, h) = (100.0 * k, 30.0 * k);
    let o = Vec3::new(5.0, -3.0, 2.0) * k;
    let ((at_u, knots_u), (at_v, knots_v)) = (crowded(in_u), crowded(in_v));
    let (nu, nv) = (at_u.len(), at_v.len());
    let mut ctrl = Vec::with_capacity(nu * nv);
    for x in &at_u {
        for y in &at_v {
            ctrl.push(o + Vec3::new(x * w, y * h, 0.0));
        }
    }
    let surface = NurbsSurface::new(
        1,
        1,
        knots_u.clone(),
        knots_v.clone(),
        nu,
        nv,
        ctrl,
        vec![1.0; nu * nv],
    )
    .unwrap();
    let row = |y: f64| -> Vec<Vec3> {
        at_u.iter()
            .map(|x| o + Vec3::new(x * w, y * h, 0.0))
            .collect()
    };
    let column = |x: f64| -> Vec<Vec3> {
        at_v.iter()
            .map(|y| o + Vec3::new(x * w, y * h, 0.0))
            .collect()
    };
    let along_u = |points: Vec<Vec3>| {
        Curve3::Nurbs(NurbsCurve::polynomial(1, knots_u.clone(), points).unwrap())
    };
    let along_v = |points: Vec<Vec3>| {
        Curve3::Nurbs(NurbsCurve::polynomial(1, knots_v.clone(), points).unwrap())
    };
    let mut body = Body::new(BodyKind::Sheet);
    let corner = |x: f64, y: f64| o + Vec3::new(x * w, y * h, 0.0);
    let (v00, v10, v11, v01) = (
        body.add_vertex(corner(0.0, 0.0)),
        body.add_vertex(corner(1.0, 0.0)),
        body.add_vertex(corner(1.0, 1.0)),
        body.add_vertex(corner(0.0, 1.0)),
    );
    let bottom = body.add_edge(along_u(row(0.0)), (0.0, 1.0), v00, v10);
    let right = body.add_edge(along_v(column(1.0)), (0.0, 1.0), v10, v11);
    let top = body.add_edge(along_u(row(1.0)), (0.0, 1.0), v01, v11);
    let left = body.add_edge(along_v(column(0.0)), (0.0, 1.0), v00, v01);
    let coedge = |edge, reversed| Coedge { edge, reversed };
    let lp = body.add_loop(vec![
        coedge(bottom, false),
        coedge(right, false),
        coedge(top, true),
        coedge(left, true),
    ]);
    let face = body.add_face(Surface::Nurbs(surface), false, vec![lp]);
    body.add_shell(vec![face]);
    body
}

/// A face whose knots crowd in u and in v into one corner cell of the
/// grid (review of 2026-11-02: an image from a cell's corner to a point
/// inside it crosses the knots of both, here 1000 in u and 40 in v, more
/// than the 1024 an image was allowed, and its NaN went through the
/// checks as a number) is laid flat to its rectangle, every number of its
/// development finite.
#[test]
fn knots_crowded_in_u_and_v_into_one_cell_are_laid_flat() {
    for k in SCALES {
        let body = crowded_plane(k, 1000, 40);
        let dev = develop(&body, &[FaceId(0)], &tol(k)).unwrap_or_else(|e| panic!("{k}: {e}"));
        let e = dev.error();
        assert!(
            e.map.is_finite() && e.map <= 0.5e-6 * k && e.length <= 1e-9 && e.area <= 1e-9,
            "{k}: {e:?}"
        );
        let flat = dev.flat_body(&Frame::WORLD, &tol(k)).unwrap();
        let area: f64 = craft3d_topo::face_areas(&flat.body)
            .iter()
            .map(|a| *a.as_ref().unwrap())
            .sum();
        let want = 3000.0 * k * k;
        assert!((area - want).abs() <= 1e-9 * want, "{k}: {area}");
        // A point inside the crowded cell maps there and back.
        let p = body.faces[0].surface.eval(3e-4, 7e-4);
        let (_, q) = dev.to_3d(dev.to_flat(FaceId(0), p).unwrap()).unwrap();
        assert!(q.distance(p) <= 1e-6 * k, "{k}: {p:?} came back as {q:?}");
    }
}
