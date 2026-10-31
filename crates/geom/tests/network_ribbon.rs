//! Edges held to a spline face along a curve on it that is no parameter
//! line (the ribbon, docs/design/surfacing.md › *Holds and the measure
//! along any curve on a spline face*), and `continuity` along such a
//! curve, at three scales:
//! - a cylinder written as a rational spline drum, cut by a slanted
//!   ellipse (its curve in the drum's parameters projected, the edge
//!   composed exactly), and a band held in curvature (G2) to it: proven G2
//!   and its normal curvature across the seam the cylinder's closed form
//!   `cos²(φ) / R` at every sample;
//! - a sphere-like bicubic drum cut along a slanted curve in its
//!   parameters, a band held G2: proven G2, its curvature across the
//!   drum's own at the same point and direction;
//! - the same seams lifted: the edge moved off the drum by twice the
//!   tolerance is not G0, the second row turned by ten times the angular
//!   tolerance is G0 only, the third row moved by ten times the curvature
//!   threshold is G1 only, by a hundredth of it still G2;
//! - a projected curve (not a composition) held tangent and in
//!   curvature: built, and its seam proven against the drum.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_geom::curvature::second_derivatives;
use craft3d_geom::network::{Condition, Network, Order, Patch, arc, continuity, gordon, iso_curve};
use craft3d_geom::trim::{compose, uv_through};
use craft3d_geom::{NurbsCurve, NurbsSurface, Surface};
use craft3d_math::Portable;
use craft3d_math::{Frame, Vec3};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn v(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3::new(x, y, z)
}

fn bezier(points: [Vec3; 4]) -> NurbsCurve {
    NurbsCurve::polynomial(
        3,
        vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
        points.to_vec(),
    )
    .unwrap()
}

/// The curve `c` swept along z from `z0` to `z1`: a ruled spline of
/// degree 1 across, exactly the cylinder over `c`.
fn swept(c: &NurbsCurve, z0: f64, z1: f64) -> NurbsSurface {
    let n = c.control_points().len();
    let mut points = Vec::with_capacity(2 * n);
    let mut weights = Vec::with_capacity(2 * n);
    for (p, w) in c.control_points().iter().zip(c.weights()) {
        points.push(*p + v(0.0, 0.0, z0));
        points.push(*p + v(0.0, 0.0, z1));
        weights.push(*w);
        weights.push(*w);
    }
    NurbsSurface::new(
        c.degree(),
        1,
        c.knots().to_vec(),
        vec![0.0, 0.0, 1.0, 1.0],
        n,
        2,
        points,
        weights,
    )
    .unwrap()
}

/// The drum: a third of the cylinder of radius `r` about z (from −60°),
/// `z` from −r to 2r, a rational spline.
fn cylinder_drum(r: f64) -> NurbsSurface {
    let circle = arc(&Frame::WORLD, r, -60f64.to_radians(), 120f64.to_radians()).unwrap();
    swept(&circle, -r, 2.0 * r)
}

/// The curve on `drum` through the parameters of `points` (closest
/// points), composed exactly: its parameter-space cubic and the edge.
fn on_drum(drum: &NurbsSurface, points: &[Vec3]) -> (NurbsCurve, NurbsCurve) {
    let wrapped = Surface::Nurbs(drum.clone());
    let n = points.len() - 1;
    let params: Vec<f64> = (0..=n).map(|i| i as f64 / n as f64).collect();
    let uv: Vec<(f64, f64)> = points
        .iter()
        .map(|p| wrapped.project(*p).unwrap())
        .collect();
    let c = uv_through(&params, &uv).unwrap();
    let edge = compose(drum, &c).unwrap();
    (c, edge)
}

/// The slanted ellipse on the cylinder: the plane `z = 0.6 x + 0.2 r`
/// met with it, from −45° to 45°.
fn ellipse_points(r: f64) -> Vec<Vec3> {
    (0..=12)
        .map(|i| {
            let a = (-45.0 + 7.5 * f64::from(i)).to_radians();
            let (x, y) = (r * a.pcos(), r * a.psin());
            v(x, y, 0.6 * x + 0.2 * r)
        })
        .collect()
}

/// Every control point of `c` mapped by `f` (exact for an affine `f`).
fn mapped(c: &NurbsCurve, f: impl Fn(Vec3) -> Vec3) -> NurbsCurve {
    NurbsCurve::new(
        c.degree(),
        c.knots().to_vec(),
        c.control_points().iter().map(|p| f(*p)).collect(),
        c.weights().to_vec(),
    )
    .unwrap()
}

/// A band from `edge` up and in to a copy of it moved by `lift` and
/// drawn towards the z axis by `inward`, crossed by four cubics.
fn band(edge: &NurbsCurve, lift: f64, inward: f64) -> (Vec<NurbsCurve>, Vec<NurbsCurve>) {
    let top = mapped(edge, |p| v(p.x * inward, p.y * inward, p.z + lift));
    let (a, b) = edge.domain();
    let vs: Vec<NurbsCurve> = [0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0]
        .iter()
        .map(|f| {
            let t = a + (b - a) * f;
            let (p, q) = (edge.eval(t), top.eval(t));
            bezier([
                p,
                p + v(0.0, 0.0, 0.35 * lift),
                q + v(p.x - q.x, p.y - q.y, 0.0) * 0.4,
                q,
            ])
        })
        .collect();
    (vec![edge.clone(), top], vs)
}

fn net<'a>(
    u: &'a [NurbsCurve],
    vv: &'a [NurbsCurve],
    first: Condition<'a>,
    tol: f64,
) -> Network<'a> {
    Network {
        u,
        v: vv,
        edges: [first, Condition::Free, Condition::Free, Condition::Free],
        tolerance: tol,
        angular: 1e-9,
    }
}

/// The normal curvature of `s` across v = 0 in the direction it leaves
/// the edge, the point, the unit normal and that direction, at `u`.
fn across(s: &NurbsSurface, u: f64) -> (Vec3, f64, Vec3, Vec3) {
    let wrapped = Surface::Nurbs(s.clone());
    let (p, du, dv) = wrapped.derivatives(u, 0.0);
    let (_, _, dvv) = second_derivatives(&wrapped, u, 0.0);
    let n = du.cross(dv).normalized().unwrap();
    (p, n.dot(dvv) / dv.dot(dv), n, dv.normalized().unwrap())
}

fn samples() -> impl Iterator<Item = f64> {
    (0..=24).map(|k| f64::from(k) / 24.0)
}

fn cylinder_patch(scale: f64) -> (NurbsSurface, Patch) {
    let r = 2.0 * scale;
    let drum = cylinder_drum(r);
    let (_, edge) = on_drum(&drum, &ellipse_points(r));
    let (us, vs) = band(&edge, 0.8 * r, 0.7);
    let face = Surface::Nurbs(drum.clone());
    let patch = gordon(&net(
        &us,
        &vs,
        Condition::Curvature {
            surface: &face,
            weight: 1.0,
        },
        1e-6 * scale,
    ))
    .unwrap();
    (drum, patch)
}

#[test]
fn a_band_held_in_curvature_along_a_slanted_ellipse_on_a_cylinder_drum_takes_its_curvature() {
    for scale in SCALES {
        let r = 2.0 * scale;
        let (drum, patch) = cylinder_patch(scale);
        let seam = patch.seams[0].expect("the seam is measured");
        assert_eq!(seam.order, Order::G2, "scale {scale}: {seam:?}");
        for u in samples() {
            let (p, k, n, d) = across(&patch.surface, u);
            let radial = v(p.x, p.y, 0.0);
            assert!(
                (radial.length() - r).abs() <= 1e-9 * scale,
                "off the cylinder by {:e} (scale {scale})",
                radial.length() - r
            );
            let outward = radial.normalized().unwrap();
            assert!(n.cross(outward).length() <= 1e-8, "tangent plane");
            // The cylinder bends away from its outward normal by cos²φ / R
            // in a direction at φ from its circles.
            let around = v(0.0, 0.0, 1.0).cross(outward);
            let closed = -(d.dot(around)).powi(2) / r;
            let k_out = k * n.dot(outward).signum();
            assert!(
                (k_out - closed).abs() <= 1e-7 / r,
                "curvature across {k_out:e}, the cylinder's {closed:e} (scale {scale}, u {u})"
            );
        }
        // Measured both ways round: the drum first or the band first.
        let edge = patch.boundary[0].clone().unwrap();
        let (tol, ang) = (1e-6 * scale, 1e-9);
        let sheet = Surface::Nurbs(patch.surface.clone());
        let face = Surface::Nurbs(drum.clone());
        let ab = continuity(&sheet, &face, &edge, tol, ang).unwrap();
        let ba = continuity(&face, &sheet, &edge, tol, ang).unwrap();
        assert_eq!(ab.order, Order::G2, "{ab:?}");
        assert_eq!(ba.order, Order::G2, "{ba:?}");
        // Held only tangent, the band is G1 and not G2.
        let (_, ellipse) = on_drum(&drum, &ellipse_points(r));
        let (us, vs) = band(&ellipse, 0.8 * r, 0.7);
        let g1 = gordon(&net(
            &us,
            &vs,
            Condition::Tangent {
                surface: &face,
                weight: 1.0,
            },
            tol,
        ))
        .unwrap();
        let tangency = g1.tangency[0].unwrap();
        assert!(tangency <= ang, "{tangency:e}");
    }
}

/// The patch with row `row` of its control points across v = 0 moved by
/// `delta` along the drum's normal at the closest point.
fn lifted(s: &NurbsSurface, drum: &NurbsSurface, row: usize, delta: f64) -> NurbsSurface {
    let face = Surface::Nurbs(drum.clone());
    let (pu, pv) = s.degrees();
    let (nu, nv) = s.counts();
    let mut points = s.control_points().to_vec();
    for i in 0..nu {
        let edge_point = s.control_points()[i * nv];
        let (a, b) = face.project(edge_point).unwrap();
        let n = face.normal(a, b).unwrap();
        points[i * nv + row] += n * delta;
    }
    NurbsSurface::new(
        pu,
        pv,
        s.knots_u().to_vec(),
        s.knots_v().to_vec(),
        nu,
        nv,
        points,
        s.weights().to_vec(),
    )
    .unwrap()
}

#[test]
fn seams_lifted_just_above_a_threshold_are_refused_that_order() {
    for scale in SCALES {
        let (tol, ang) = (1e-6 * scale, 1e-9);
        let (drum, patch) = cylinder_patch(scale);
        let face = Surface::Nurbs(drum.clone());
        // Each lifted patch is measured along its own edge (its line
        // v = 0), against the drum.
        let measure = |s: NurbsSurface| {
            let edge = iso_curve(&s, true, 0.0).unwrap();
            continuity(&Surface::Nurbs(s), &face, &edge, tol, ang)
        };
        let exact = measure(patch.surface.clone()).unwrap();
        assert_eq!(exact.order, Order::G2);
        // Position: the edge row moved off by twice the tolerance.
        // No longer a curve of the drum, the lifted edge is not taken for
        // one: the seam is not proven, and the measure says so.
        let off = measure(lifted(&patch.surface, &drum, 0, 2.0 * tol));
        let Err(e) = off else {
            panic!("a seam lifted off the drum was measured: {off:?}")
        };
        assert!(e.to_string().contains("not proven"), "{e}");
        // The angle: the second row moved so that the planes open by about
        // ten times the angular tolerance (the probe gives the rate).
        let probe = 1e-4 * scale;
        let opened = measure(lifted(&patch.surface, &drum, 1, probe)).unwrap();
        let per = opened.angle.found / probe;
        assert!(per > 0.0, "{opened:?}");
        let tilted = measure(lifted(&patch.surface, &drum, 1, 10.0 * ang / per)).unwrap();
        assert_eq!(tilted.order, Order::G0, "{tilted:?}");
        assert!(tilted.angle.beyond(), "{tilted:?}");
        // Curvature: the third row by ten times the threshold (G1) and by
        // a hundredth of it (still G2).
        let bent = measure(lifted(&patch.surface, &drum, 2, probe)).unwrap();
        let per = bent.curvature.found / probe;
        assert!(per > 0.0, "{bent:?}");
        let threshold = bent.curvature.threshold;
        let big = measure(lifted(&patch.surface, &drum, 2, 10.0 * threshold / per)).unwrap();
        assert_eq!(big.order, Order::G1, "{big:?}");
        assert!(big.curvature.beyond(), "{big:?}");
        let small = measure(lifted(&patch.surface, &drum, 2, 0.01 * threshold / per)).unwrap();
        assert_eq!(small.order, Order::G2, "scale {scale}: {small:?}");
    }
}

/// A sphere-like bicubic drum: a 6 × 6 net on the sphere of radius `r`
/// about `(0, 0, −0.4 r)`, over x, y in [−0.6 r, 0.6 r].
fn sphere_drum(r: f64) -> NurbsSurface {
    let n = 6;
    let mut points = Vec::with_capacity(n * n);
    for i in 0..n {
        for j in 0..n {
            let x = -0.6 * r + 1.2 * r * i as f64 / (n - 1) as f64;
            let y = -0.6 * r + 1.2 * r * j as f64 / (n - 1) as f64;
            let z = (r * r - x * x - y * y).sqrt() - 0.4 * r;
            points.push(v(x, y, z));
        }
    }
    let knots = vec![0.0, 0.0, 0.0, 0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0, 1.0, 1.0, 1.0];
    NurbsSurface::new(3, 3, knots.clone(), knots, n, n, points, vec![1.0; n * n]).unwrap()
}

#[test]
fn a_band_held_in_curvature_along_a_slanted_curve_on_a_sphere_like_drum_takes_its_curvature() {
    for scale in SCALES {
        let r = 1.5 * scale;
        let drum = sphere_drum(r);
        let face = Surface::Nurbs(drum.clone());
        // A curve in the drum's parameters, slanted and bowed: no
        // parameter line, across both knot lines.
        let c = uv_through(&[0.0, 0.5, 1.0], &[(0.15, 0.25), (0.5, 0.42), (0.85, 0.7)]).unwrap();
        let edge = compose(&drum, &c).unwrap();
        let top = mapped(&edge, |p| v(0.6 * p.x, 0.6 * p.y + 0.3 * r, p.z + 0.5 * r));
        let (a, b) = edge.domain();
        let vs: Vec<NurbsCurve> = [0.0, 0.5, 1.0]
            .iter()
            .map(|f| {
                let t = a + (b - a) * f;
                let (p, q) = (edge.eval(t), top.eval(t));
                bezier([p, p + (q - p) * 0.33, p + (q - p) * 0.66, q])
            })
            .collect();
        let us = vec![edge.clone(), top];
        let tol = 1e-6 * scale;
        let patch = gordon(&net(
            &us,
            &vs,
            Condition::Curvature {
                surface: &face,
                weight: 1.0,
            },
            tol,
        ))
        .unwrap();
        let seam = patch.seams[0].expect("measured");
        assert_eq!(seam.order, Order::G2, "scale {scale}: {seam:?}");
        // Against the drum's own curvature at the same point, in the
        // direction the band leaves it.
        for u in samples() {
            let (p, k, n, d) = across(&patch.surface, u);
            let (s, t) = face.project(p).unwrap();
            let q = face.eval(s, t);
            assert!(q.distance(p) <= 1e-9 * scale, "off the drum");
            let (_, fu, fv) = face.derivatives(s, t);
            let (fuu, fuv, fvv) = second_derivatives(&face, s, t);
            let m = fu.cross(fv).normalized().unwrap();
            // d in the drum's tangent plane: d = α fu + β fv.
            let (e, f, g) = (fu.dot(fu), fu.dot(fv), fv.dot(fv));
            let (x, y) = (d.dot(fu), d.dot(fv));
            let det = e * g - f * f;
            let (al, be) = ((x * g - y * f) / det, (y * e - x * f) / det);
            let second = m.dot(fuu) * al * al + 2.0 * m.dot(fuv) * al * be + m.dot(fvv) * be * be;
            let k_drum = second * m.dot(n).signum();
            assert!(
                (k - k_drum).abs() <= 1e-7 / r,
                "curvature across {k:e}, the drum's {k_drum:e} (scale {scale}, u {u})"
            );
        }
    }
}

#[test]
fn an_edge_only_within_the_tolerance_of_the_drum_is_projected_held_and_proven() {
    for scale in SCALES {
        let r = 2.0 * scale;
        let (tol, ang) = (1e-6 * scale, 1e-9);
        let drum = cylinder_drum(r);
        let face = Surface::Nurbs(drum.clone());
        // The ellipse's edge pushed out from the axis by a third of the
        // tolerance: no longer a curve of the drum (not recovered), within
        // the tolerance of it. The hold projects it (sampled) and takes
        // the projection's curve on the drum as the edge.
        let (_, exact) = on_drum(&drum, &ellipse_points(r));
        let edge = mapped(&exact, |p| {
            let radial = v(p.x, p.y, 0.0).normalized().unwrap();
            p + radial * (tol / 3.0)
        });
        let (us, vs) = band(&edge, 0.8 * r, 0.7);
        let patch = gordon(&net(
            &us,
            &vs,
            Condition::Tangent {
                surface: &face,
                weight: 1.0,
            },
            tol,
        ))
        .unwrap();
        assert!(patch.tangency[0].unwrap() <= ang);
        assert!(patch.mismatch <= tol, "{:e}", patch.mismatch);
        // The patch's edge is the projection's curve composed on the
        // drum: recovered from it, and the seam proven tangent.
        let boundary = patch.boundary[0].clone().unwrap();
        let c = continuity(
            &Surface::Nurbs(patch.surface.clone()),
            &face,
            &boundary,
            tol,
            ang,
        )
        .unwrap();
        assert!(c.position.within() && c.angle.within(), "{c:?}");
        // The pushed edge itself is no curve of the drum: the measure
        // does not take a sampled projection for one, and says so.
        let not = continuity(
            &Surface::Nurbs(patch.surface.clone()),
            &face,
            &edge,
            tol,
            ang,
        );
        let Err(e) = not else {
            panic!("measured along a curve that is not the drum's: {not:?}")
        };
        assert!(e.to_string().contains("not proven"), "{e}");
    }
}
