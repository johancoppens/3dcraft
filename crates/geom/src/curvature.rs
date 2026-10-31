//! Second derivatives and curvature of any surface.
//!
//! The Gaussian curvature `K = (LN − M²)/(EG − F²)` and the mean
//! curvature `H = (EN − 2FM + GL) / (2(EG − F²))`, from the first
//! fundamental form `E = S_u·S_u`, `F = S_u·S_v`, `G = S_v·S_v` and the
//! second `L = S_uu·n`, `M = S_uv·n`, `N = S_vv·n` (do Carmo,
//! *Differential Geometry of Curves and Surfaces*, §3-3).
//!
//! The second derivatives are exact for every kind: closed forms for the
//! analytic surfaces; for Bézier and NURBS surfaces the derivatives of
//! the B-spline basis functions to order two (Piegl & Tiller, *The NURBS
//! Book*, algorithm A2.3) over the homogeneous control net and the
//! quotient rule for a rational surface (ibid. §4.5). See
//! docs/design/developable.md.

use crate::{Curve3, NurbsCurve, NurbsSurface, Surface};
use craft3d_math::Portable;
use craft3d_math::Vec3;

/// Highest degree the basis derivatives are worked out for here: the
/// geometry's own cap (`crate::MAX_DEGREE`).
const ROOM: usize = crate::MAX_DEGREE + 1;

/// `(S_uu, S_uv, S_vv)` at `(u, v)`. Zero vectors where a surface has no
/// such derivative (a plane; a spline of degree one in a direction).
pub fn second_derivatives(surface: &Surface, u: f64, v: f64) -> (Vec3, Vec3, Vec3) {
    let (su, cu) = u.psin_cos();
    match surface {
        Surface::Plane(_) => (Vec3::ZERO, Vec3::ZERO, Vec3::ZERO),
        Surface::Cylinder(c) => {
            let r = c.radius();
            (
                c.frame().vector(-r * cu, -r * su, 0.0),
                Vec3::ZERO,
                Vec3::ZERO,
            )
        }
        Surface::Cone(c) => {
            let (rho, k) = (
                c.radius() + v * c.half_angle().ptan(),
                c.half_angle().ptan(),
            );
            (
                c.frame().vector(-rho * cu, -rho * su, 0.0),
                c.frame().vector(-k * su, k * cu, 0.0),
                Vec3::ZERO,
            )
        }
        Surface::Sphere(s) => {
            let r = s.radius();
            let (sv, cv) = v.psin_cos();
            (
                s.frame().vector(-r * cv * cu, -r * cv * su, 0.0),
                s.frame().vector(r * sv * su, -r * sv * cu, 0.0),
                s.frame().vector(-r * cv * cu, -r * cv * su, -r * sv),
            )
        }
        Surface::Torus(t) => {
            let (major, minor) = t.radii();
            let (sv, cv) = v.psin_cos();
            let ring = major + minor * cv;
            (
                t.frame().vector(-ring * cu, -ring * su, 0.0),
                t.frame().vector(minor * sv * su, -minor * sv * cu, 0.0),
                t.frame()
                    .vector(-minor * cv * cu, -minor * cv * su, -minor * sv),
            )
        }
        Surface::Bezier(b) => match NurbsSurface::from_bezier(b) {
            Ok(s) => nurbs_second(&s, u, v),
            Err(_) => (Vec3::ZERO, Vec3::ZERO, Vec3::ZERO),
        },
        Surface::Nurbs(s) => nurbs_second(s, u, v),
    }
}

/// The curvatures at `(u, v)`: `(K, H)`, Gaussian and mean, the mean
/// with the sign of the surface's own normal `S_u × S_v` (positive where
/// the surface bends towards it). `None` where the surface has no normal
/// (a cone's apex, a collapsed edge of a spline) or a value is not finite.
pub fn curvatures(surface: &Surface, u: f64, v: f64) -> Option<(f64, f64)> {
    let (_, du, dv) = surface.derivatives(u, v);
    let cross = du.cross(dv);
    let det = cross.length_squared();
    let scale = du.length_squared() * dv.length_squared();
    // `EG − F²` is |S_u × S_v|²: no normal when it is lost in the
    // rounding of `E G`.
    if !(det.is_finite() && scale.is_finite()) || det <= 1e-24 * scale || det == 0.0 {
        return None;
    }
    let n = cross * (1.0 / det.sqrt());
    let (duu, duv, dvv) = second_derivatives(surface, u, v);
    let (e, f, g) = (du.dot(du), du.dot(dv), dv.dot(dv));
    let (l, m, nn) = (duu.dot(n), duv.dot(n), dvv.dot(n));
    let k = (l * nn - m * m) / det;
    let h = (e * nn - 2.0 * f * m + g * l) / (2.0 * det);
    (k.is_finite() && h.is_finite()).then_some((k, h))
}

/// `C''(t)`, the exact second derivative of a curve: closed forms for
/// lines, circles and ellipses; for a NURBS curve the basis derivatives
/// to order two (A2.3) over the homogeneous control points and the
/// quotient rule `C'' = (A'' − 2 W' C' − W'' C) / W`. Zero where the curve
/// has none (a line; a spline of degree one); `None` for a spline whose
/// degree is beyond [`MAX_CURVATURE_DEGREE`] or whose value is not finite.
pub fn curve_second_derivative(curve: &Curve3, t: f64) -> Option<Vec3> {
    Some(match curve {
        Curve3::Line(_) => Vec3::ZERO,
        Curve3::Circle(c) => {
            let (s, co) = t.psin_cos();
            let r = c.radius();
            c.frame().vector(-r * co, -r * s, 0.0)
        }
        Curve3::Ellipse(e) => {
            let (s, co) = t.psin_cos();
            let (a, b) = e.semi_axes();
            e.frame().vector(-a * co, -b * s, 0.0)
        }
        Curve3::Nurbs(c) => nurbs_curve_second(c, t)?,
    })
}

/// The highest degree whose second derivatives are worked out here.
pub const MAX_CURVATURE_DEGREE: usize = ROOM - 1;

/// Whether [`second_derivatives`] and [`curve_second_derivative`] are
/// exact for this surface: false for a spline of a degree beyond
/// [`MAX_CURVATURE_DEGREE`], whose second derivatives come back zero.
pub fn surface_curvature_supported(surface: &Surface) -> bool {
    match surface {
        Surface::Nurbs(s) => {
            let (pu, pv) = s.degrees();
            pu <= MAX_CURVATURE_DEGREE && pv <= MAX_CURVATURE_DEGREE
        }
        Surface::Bezier(b) => NurbsSurface::from_bezier(b).is_ok_and(|s| {
            let (pu, pv) = s.degrees();
            pu <= MAX_CURVATURE_DEGREE && pv <= MAX_CURVATURE_DEGREE
        }),
        _ => true,
    }
}

/// The curvature vector `κ N = (C' × C'') × C' / |C'|⁴` of a curve at
/// `t` (pointing to the centre of curvature, of length the curvature),
/// with the unit tangent. `None` where the curve has no tangent (a
/// vanishing derivative) or a value is not finite.
pub fn curve_curvature(curve: &Curve3, t: f64) -> Option<(Vec3, Vec3)> {
    let (_, d1) = curve.derivatives(t);
    let d2 = curve_second_derivative(curve, t)?;
    let speed2 = d1.length_squared();
    if !(speed2.is_finite() && speed2 > 0.0) {
        return None;
    }
    let k = d1.cross(d2).cross(d1) * (1.0 / (speed2 * speed2));
    let tangent = d1.normalized()?;
    k.is_finite().then_some((k, tangent))
}

fn nurbs_curve_second(c: &NurbsCurve, t: f64) -> Option<Vec3> {
    let p = c.degree();
    let (points, weights, knots) = (c.control_points(), c.weights(), c.knots());
    let n = points.len();
    if p >= ROOM || n == 0 {
        return None;
    }
    let span = span_of(knots, p, n, t);
    let b = basis_derivatives(knots, p, span, t);
    let (mut a, mut w) = ([Vec3::ZERO; 3], [0.0_f64; 3]);
    for i in 0..=p {
        let Some(index) = (span + i).checked_sub(p) else {
            continue;
        };
        let (Some(&q), Some(&weight)) = (points.get(index), weights.get(index)) else {
            continue;
        };
        for (k, (ak, wk)) in a.iter_mut().zip(w.iter_mut()).enumerate() {
            let nb = b.get(k).and_then(|r| r.get(i)).copied().unwrap_or(0.0) * weight;
            *ak += q * nb;
            *wk += nb;
        }
    }
    let [a0, a1, a2] = a;
    let [w0, w1, w2] = w;
    if !(w0.is_finite() && w0 > 0.0) {
        return None;
    }
    let c0 = a0 * (1.0 / w0);
    let c1 = (a1 - c0 * w1) * (1.0 / w0);
    let c2 = (a2 - c1 * (2.0 * w1) - c0 * w2) * (1.0 / w0);
    c2.is_finite().then_some(c2)
}

/// The Gaussian curvature at `(u, v)`; see [`curvatures`].
pub fn gaussian_curvature(surface: &Surface, u: f64, v: f64) -> Option<f64> {
    curvatures(surface, u, v).map(|(k, _)| k)
}

/// The knot span of `t` (`knots[span] <= t < knots[span + 1]`, the last
/// span for the end of the domain): The NURBS Book, A2.1.
fn span_of(knots: &[f64], degree: usize, count: usize, t: f64) -> usize {
    let (low_end, high_end) = (degree, count.saturating_sub(1).max(degree));
    let at = |i: usize| knots.get(i).copied().unwrap_or(f64::NAN);
    if t >= at(high_end + 1) {
        return high_end;
    }
    if t <= at(low_end) {
        return low_end;
    }
    let (mut low, mut high) = (low_end, high_end + 1);
    // Bisection over at most 2^16 control points: bounded by 64 steps.
    for _ in 0..64 {
        if high - low <= 1 {
            break;
        }
        let mid = low + (high - low) / 2;
        if t < at(mid) {
            high = mid;
        } else {
            low = mid;
        }
    }
    low
}

/// The basis functions of `degree` that are not zero at `t` (those of the
/// control points `span − degree ..= span`) and their first and second
/// derivatives: `out[k][j]` is the `k`-th derivative of `N_{span−degree+j}`.
/// The NURBS Book, A2.3, with `n = 2`.
fn basis_derivatives(knots: &[f64], degree: usize, span: usize, t: f64) -> [[f64; ROOM]; 3] {
    let p = degree.min(ROOM - 1);
    let k = |i: usize| knots.get(i).copied().unwrap_or(0.0);
    let mut ndu = [[0.0_f64; ROOM]; ROOM];
    let mut left = [0.0_f64; ROOM];
    let mut right = [0.0_f64; ROOM];
    if let Some(row) = ndu.get_mut(0)
        && let Some(cell) = row.get_mut(0)
    {
        *cell = 1.0;
    }
    let get = |a: &[[f64; ROOM]; ROOM], r: usize, c: usize| {
        a.get(r).and_then(|row| row.get(c)).copied().unwrap_or(0.0)
    };
    let set = |a: &mut [[f64; ROOM]; ROOM], r: usize, c: usize, value: f64| {
        if let Some(cell) = a.get_mut(r).and_then(|row| row.get_mut(c)) {
            *cell = value;
        }
    };
    for j in 1..=p {
        if let Some(l) = left.get_mut(j) {
            *l = t - k((span + 1).saturating_sub(j));
        }
        if let Some(r) = right.get_mut(j) {
            *r = k(span + j) - t;
        }
        let mut saved = 0.0;
        for r in 0..j {
            let right_r = right.get(r + 1).copied().unwrap_or(0.0);
            let left_jr = left.get(j - r).copied().unwrap_or(0.0);
            // Lower triangle: the knot differences.
            let diff = right_r + left_jr;
            set(&mut ndu, j, r, diff);
            let temp = if diff != 0.0 {
                get(&ndu, r, j - 1) / diff
            } else {
                0.0
            };
            // Upper triangle: the basis functions.
            set(&mut ndu, r, j, saved + right_r * temp);
            saved = left_jr * temp;
        }
        set(&mut ndu, j, j, saved);
    }
    let mut out = [[0.0_f64; ROOM]; 3];
    for j in 0..=p {
        if let Some(cell) = out.get_mut(0).and_then(|row| row.get_mut(j)) {
            *cell = get(&ndu, j, p);
        }
    }
    let mut a = [[0.0_f64; ROOM]; 2];
    let geta = |a: &[[f64; ROOM]; 2], r: usize, c: usize| {
        a.get(r).and_then(|row| row.get(c)).copied().unwrap_or(0.0)
    };
    let seta = |a: &mut [[f64; ROOM]; 2], r: usize, c: usize, value: f64| {
        if let Some(cell) = a.get_mut(r).and_then(|row| row.get_mut(c)) {
            *cell = value;
        }
    };
    for r in 0..=p {
        let (mut s1, mut s2) = (0_usize, 1_usize);
        seta(&mut a, 0, 0, 1.0);
        for order in 1..=2_usize.min(p) {
            let mut d = 0.0;
            let pk = p - order;
            if r >= order {
                let denominator = get(&ndu, pk + 1, r - order);
                let value = if denominator != 0.0 {
                    geta(&a, s1, 0) / denominator
                } else {
                    0.0
                };
                seta(&mut a, s2, 0, value);
                d = value * get(&ndu, r - order, pk);
            }
            let j1 = if r + 1 >= order { 1 } else { order - r };
            let j2 = if r <= pk + 1 { order - 1 } else { p - r };
            for j in j1..=j2 {
                let denominator = get(&ndu, pk + 1, r + j - order);
                let value = if denominator != 0.0 {
                    (geta(&a, s1, j) - geta(&a, s1, j - 1)) / denominator
                } else {
                    0.0
                };
                seta(&mut a, s2, j, value);
                d += value * get(&ndu, r + j - order, pk);
            }
            if r <= pk {
                let denominator = get(&ndu, pk + 1, r);
                let value = if denominator != 0.0 {
                    -geta(&a, s1, order - 1) / denominator
                } else {
                    0.0
                };
                seta(&mut a, s2, order, value);
                d += value * get(&ndu, r, pk);
            }
            if let Some(cell) = out.get_mut(order).and_then(|row| row.get_mut(r)) {
                *cell = d;
            }
            std::mem::swap(&mut s1, &mut s2);
        }
    }
    // Multiply by p!/(p−k)!.
    let mut factor = p as f64;
    for order in 1..=2_usize.min(p) {
        for j in 0..=p {
            if let Some(cell) = out.get_mut(order).and_then(|row| row.get_mut(j)) {
                *cell *= factor;
            }
        }
        factor *= (p - order) as f64;
    }
    out
}

/// `(S_uu, S_uv, S_vv)` of a NURBS surface: the derivatives of the
/// homogeneous surface `A = Σ N_i N_j w_ij P_ij`, `w = Σ N_i N_j w_ij`,
/// then the quotient rule.
fn nurbs_second(s: &NurbsSurface, u: f64, v: f64) -> (Vec3, Vec3, Vec3) {
    let zero = (Vec3::ZERO, Vec3::ZERO, Vec3::ZERO);
    let ((pu, pv), (nu, nv)) = (s.degrees(), s.counts());
    if pu >= ROOM || pv >= ROOM || nu == 0 || nv == 0 {
        return zero;
    }
    let (ku, kv) = (s.knots_u(), s.knots_v());
    let (span_u, span_v) = (span_of(ku, pu, nu, u), span_of(kv, pv, nv, v));
    let bu = basis_derivatives(ku, pu, span_u, u);
    let bv = basis_derivatives(kv, pv, span_v, v);
    let (points, weights) = (s.control_points(), s.weights());
    // a[k][l]: ∂^(k+l) A / ∂u^k ∂v^l, w likewise.
    let mut a = [[Vec3::ZERO; 3]; 3];
    let mut w = [[0.0_f64; 3]; 3];
    for i in 0..=pu {
        let Some(row) = (span_u + i).checked_sub(pu) else {
            continue;
        };
        for j in 0..=pv {
            let Some(col) = (span_v + j).checked_sub(pv) else {
                continue;
            };
            let index = row * nv + col;
            let (Some(&p), Some(&weight)) = (points.get(index), weights.get(index)) else {
                continue;
            };
            for (k, a_row) in a.iter_mut().enumerate() {
                for (l, cell) in a_row.iter_mut().enumerate() {
                    if k + l > 2 {
                        continue;
                    }
                    let n = bu.get(k).and_then(|r| r.get(i)).copied().unwrap_or(0.0)
                        * bv.get(l).and_then(|r| r.get(j)).copied().unwrap_or(0.0)
                        * weight;
                    *cell += p * n;
                    if let Some(wc) = w.get_mut(k).and_then(|r| r.get_mut(l)) {
                        *wc += n;
                    }
                }
            }
        }
    }
    let [[a00, a01, a02], [a10, a11, _], [a20, _, _]] = a;
    let [[w00, w01, w02], [w10, w11, _], [w20, _, _]] = w;
    if !(w00.is_finite() && w00 > 0.0) {
        return zero;
    }
    let inv = 1.0 / w00;
    let p = a00 * inv;
    let su = (a10 - p * w10) * inv;
    let sv = (a01 - p * w01) * inv;
    let suu = (a20 - su * (2.0 * w10) - p * w20) * inv;
    let suv = (a11 - su * w01 - sv * w10 - p * w11) * inv;
    let svv = (a02 - sv * (2.0 * w01) - p * w02) * inv;
    (suu, suv, svv)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BezierPatch, Cone, Cylinder, Plane, Sphere, Torus};
    use craft3d_math::Frame;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol * (1.0 + b.abs())
    }

    /// The second derivatives against differences of the exact first
    /// derivatives, at a few points.
    fn agrees_with_differences(s: &Surface, points: &[(f64, f64)], scale: f64) {
        let h = 1e-5;
        for &(u, v) in points {
            let (_, du_p, dv_p) = s.derivatives(u + h, v);
            let (_, du_m, _) = s.derivatives(u - h, v);
            let (_, _, dv_q) = s.derivatives(u, v + h);
            let (_, _, dv_r) = s.derivatives(u, v - h);
            let (_, _, dv_m) = s.derivatives(u - h, v);
            let (suu, suv, svv) = second_derivatives(s, u, v);
            let fd_uu = (du_p - du_m) * (0.5 / h);
            let fd_uv = (dv_p - dv_m) * (0.5 / h);
            let fd_vv = (dv_q - dv_r) * (0.5 / h);
            for (exact, fd, what) in [(suu, fd_uu, "uu"), (suv, fd_uv, "uv"), (svv, fd_vv, "vv")] {
                assert!(
                    exact.distance(fd) <= 1e-5 * scale,
                    "{} {what} at ({u}, {v}): {exact:?} against {fd:?}",
                    s.kind()
                );
            }
        }
    }

    #[test]
    fn a_sphere_has_the_curvature_one_over_its_radius_squared_at_three_scales() {
        for r in [1e-3, 1.0, 1e6] {
            let s = Surface::Sphere(Sphere::new(Frame::WORLD, r).unwrap());
            for (u, v) in [(0.3, 0.2), (2.0, -1.1), (5.0, 1.4)] {
                let (k, h) = curvatures(&s, u, v).unwrap();
                assert!(close(k, 1.0 / (r * r), 1e-12), "K {k} at r {r}");
                assert!(close(h.abs(), 1.0 / r, 1e-12), "H {h} at r {r}");
            }
        }
    }

    #[test]
    fn a_torus_has_its_closed_form_curvature_at_three_scales() {
        for scale in [1e-3, 1.0, 1e6] {
            let (major, minor) = (3.0 * scale, 1.0 * scale);
            let s = Surface::Torus(Torus::new(Frame::WORLD, major, minor).unwrap());
            for (u, v) in [(0.3, 0.2), (2.0, 2.5), (5.0, 4.0), (1.0, 3.0)] {
                let k = gaussian_curvature(&s, u, v).unwrap();
                let expected = v.pcos() / (minor * (major + minor * v.pcos()));
                assert!(
                    (k - expected).abs() <= 1e-12 / (scale * scale),
                    "K {k} against {expected}"
                );
            }
        }
    }

    #[test]
    fn planes_cylinders_and_cones_have_no_gaussian_curvature() {
        for scale in [1e-3, 1.0, 1e6] {
            let frame = Frame::WORLD;
            let surfaces = [
                Surface::Plane(Plane::new(frame)),
                Surface::Cylinder(Cylinder::new(frame, 2.0 * scale).unwrap()),
                Surface::Cone(Cone::new(frame, 2.0 * scale, 0.4).unwrap()),
            ];
            for s in &surfaces {
                for (u, v) in [(0.3, 0.2 * scale), (2.0, -0.5 * scale), (5.0, 1.0 * scale)] {
                    let k = gaussian_curvature(s, u, v).unwrap();
                    assert!(k.abs() <= 1e-14 / (scale * scale), "{} K {k}", s.kind());
                }
            }
            // The cylinder's mean curvature is half its curvature across.
            let (_, h) = curvatures(&surfaces[1], 0.7, 0.0).unwrap();
            assert!(close(h.abs(), 0.5 / (2.0 * scale), 1e-12));
        }
    }

    #[test]
    fn the_second_derivatives_of_analytic_surfaces_agree_with_differences() {
        let frame = Frame::WORLD;
        let points = [(0.3, 0.2), (2.0, 0.7), (4.0, -0.4)];
        for s in [
            Surface::Cylinder(Cylinder::new(frame, 2.0).unwrap()),
            Surface::Cone(Cone::new(frame, 2.0, 0.4).unwrap()),
            Surface::Sphere(Sphere::new(frame, 2.0).unwrap()),
            Surface::Torus(Torus::new(frame, 3.0, 1.0).unwrap()),
        ] {
            agrees_with_differences(&s, &points, 3.0);
        }
    }

    #[test]
    fn the_second_derivatives_of_splines_agree_with_differences() {
        // A bicubic Bézier patch, as a patch and as a rational NURBS
        // surface with uneven weights and inner knots.
        let mut net = [[Vec3::ZERO; 4]; 4];
        for (i, row) in net.iter_mut().enumerate() {
            for (j, p) in row.iter_mut().enumerate() {
                let (x, y) = (i as f64, j as f64);
                *p = Vec3::new(x, y, (x * 0.7).psin() + 0.3 * x * y - 0.2 * y * y);
            }
        }
        let patch = BezierPatch::bicubic(net).unwrap();
        let points = [(0.2, 0.3), (0.5, 0.5), (0.9, 0.1)];
        agrees_with_differences(&Surface::Bezier(patch), &points, 10.0);

        let (nu, nv) = (6, 5);
        let mut ctrl = Vec::new();
        let mut weights = Vec::new();
        for i in 0..nu {
            for j in 0..nv {
                let (x, y) = (i as f64, j as f64);
                ctrl.push(Vec3::new(
                    x,
                    y + 0.1 * x * x,
                    (x * 0.9).pcos() * (y * 0.5).psin(),
                ));
                weights.push(1.0 + 0.3 * ((i + 2 * j) % 3) as f64);
            }
        }
        let ku = vec![0.0, 0.0, 0.0, 0.0, 0.4, 0.7, 1.0, 1.0, 1.0, 1.0];
        let kv = vec![0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0, 1.0];
        let s = NurbsSurface::new(3, 2, ku, kv, nu, nv, ctrl, weights).unwrap();
        let points = [(0.2, 0.3), (0.55, 0.6), (0.9, 0.1), (0.45, 0.45)];
        agrees_with_differences(&Surface::Nurbs(s), &points, 30.0);
    }

    #[test]
    fn a_ruled_spline_between_a_line_and_its_translate_has_no_curvature() {
        // Degree 3 along u, 1 along v: a cylinder over a cubic.
        let profile = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 2.0, 0.0),
            Vec3::new(3.0, -1.0, 0.0),
            Vec3::new(4.0, 1.0, 0.0),
        ];
        let shift = Vec3::new(0.0, 0.5, 3.0);
        let mut ctrl = Vec::new();
        for p in profile {
            ctrl.push(p);
            ctrl.push(p + shift);
        }
        let s = NurbsSurface::new(
            3,
            1,
            vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
            vec![0.0, 0.0, 1.0, 1.0],
            4,
            2,
            ctrl,
            vec![1.0; 8],
        )
        .unwrap();
        let s = Surface::Nurbs(s);
        for (u, v) in [(0.1, 0.2), (0.5, 0.9), (0.8, 0.5)] {
            let k = gaussian_curvature(&s, u, v).unwrap();
            assert!(k.abs() < 1e-12, "K {k}");
        }
    }

    #[test]
    fn no_curvature_where_there_is_no_normal() {
        let cone = Surface::Cone(Cone::new(Frame::WORLD, 1.0, -0.5).unwrap());
        let apex = cone.domain().1.1;
        assert_eq!(gaussian_curvature(&cone, 0.3, apex), None);
    }

    /// A circle's curvature is 1/r towards its centre; a NURBS circle
    /// (rational quadratic) agrees with the closed form, and a polynomial
    /// spline's second derivative with differences of its first.
    #[test]
    fn curve_curvature_of_circles_and_splines() {
        let frame =
            Frame::new(Vec3::new(1.0, 2.0, 3.0), Vec3::new(0.2, 0.1, 1.0), Vec3::X).unwrap();
        let circle = Curve3::Circle(crate::Circle::new(frame, 2.5).unwrap());
        let nurbs = Curve3::Nurbs(NurbsCurve::circle(&frame, 2.5).unwrap());
        for curve in [&circle, &nurbs] {
            let (lo, hi) = curve.domain();
            for k in 0..9 {
                let t = lo + (hi - lo) * (f64::from(k) + 0.3) / 9.0;
                let (kn, _) = curve_curvature(curve, t).unwrap();
                assert!(close(kn.length(), 0.4, 1e-12), "{}", kn.length());
                let to_centre = (frame.origin() - curve.eval(t)).normalized().unwrap();
                assert!(kn.normalized().unwrap().dot(to_centre) > 1.0 - 1e-12);
            }
        }
        let spline = Curve3::Nurbs(
            NurbsCurve::polynomial(
                3,
                vec![0.0, 0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0, 1.0],
                vec![
                    Vec3::new(0.0, 0.0, 0.0),
                    Vec3::new(1.0, 2.0, 0.0),
                    Vec3::new(2.0, -1.0, 1.0),
                    Vec3::new(3.0, 0.5, 0.0),
                    Vec3::new(4.0, 0.0, 2.0),
                ],
            )
            .unwrap(),
        );
        for t in [0.1, 0.3, 0.7, 0.9] {
            let h = 1e-5;
            let fd = (spline.derivatives(t + h).1 - spline.derivatives(t - h).1) * (0.5 / h);
            let exact = curve_second_derivative(&spline, t).unwrap();
            assert!((fd - exact).length() <= 1e-6 * (1.0 + exact.length()));
        }
    }
}
