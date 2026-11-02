//! Edges of network patches held in curvature (G2, `Condition::Curvature`)
//! and the proven `continuity` measure, at three scales, checked against
//! closed forms, not against the patch's own report: a dome held to a
//! sphere along the equator takes the sphere's normal curvature 1/R across
//! it, to the sphere itself and to a network octant of it (a spline face);
//! a blend between two parallel cylinders takes 1/R₁ and 1/R₂ across its
//! two edges; G1-only and G2 pairs are told apart, a G2 patch disturbed
//! by a curvature step ten times the threshold is not G2 (its position and
//! tangent planes untouched, the step found by the test's own evaluation),
//! one a hundredth of it is; a sheet twisted within the angular tolerance
//! is not G2; a dome a million units from the origin; refusals.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_geom::curvature::second_derivatives;
use craft3d_geom::network::{
    Condition, Network, NetworkError, Order, Patch, arc, continuity, gordon,
};
use craft3d_geom::{Cylinder, NurbsCurve, NurbsSurface, Plane, Sphere, Surface, Torus};
use craft3d_math::Portable;
use craft3d_math::{Frame, Vec3};
use std::f64::consts::FRAC_PI_2;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn v(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3::new(x, y, z)
}

fn line(a: Vec3, b: Vec3) -> NurbsCurve {
    NurbsCurve::polynomial(1, vec![0.0, 0.0, 1.0, 1.0], vec![a, b]).unwrap()
}

fn bezier(points: [Vec3; 4]) -> NurbsCurve {
    NurbsCurve::polynomial(
        3,
        vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
        points.to_vec(),
    )
    .unwrap()
}

fn net<'a>(
    u: &'a [NurbsCurve],
    vv: &'a [NurbsCurve],
    edges: [Condition<'a>; 4],
    scale: f64,
) -> Network<'a> {
    Network {
        u,
        v: vv,
        edges,
        tolerance: 1e-6 * scale,
        angular: 1e-9,
    }
}

fn first(c: Condition<'_>) -> [Condition<'_>; 4] {
    [c, Condition::Free, Condition::Free, Condition::Free]
}

/// The direction at longitude `f` of a right angle (0 to 1).
fn longitude(f: f64) -> Vec3 {
    let a = FRAC_PI_2 * f;
    v(a.pcos(), a.psin(), 0.0)
}

/// A dome over a quarter of the equator of the sphere of radius `r` about
/// the origin: it leaves the equator upwards and ends on an arc at 0.8 r,
/// of radius 0.7 r: not the sphere.
fn dome(r: f64) -> (Vec<NurbsCurve>, Vec<NurbsCurve>) {
    let top = Frame::new(v(0.0, 0.0, 0.8 * r), v(0.0, 0.0, 1.0), v(1.0, 0.0, 0.0)).unwrap();
    let us = vec![
        arc(&Frame::WORLD, r, 0.0, FRAC_PI_2).unwrap(),
        arc(&top, 0.7 * r, 0.0, FRAC_PI_2).unwrap(),
    ];
    let vs = [0.0, 0.5, 1.0]
        .iter()
        .map(|f| {
            let d = longitude(*f);
            bezier([
                d * r,
                d * r + v(0.0, 0.0, 0.3 * r),
                d * (0.85 * r) + v(0.0, 0.0, 0.65 * r),
                d * (0.7 * r) + v(0.0, 0.0, 0.8 * r),
            ])
        })
        .collect();
    (us, vs)
}

/// The lower octant of the sphere: the equator, the parallel at −45° and
/// the south pole, crossed by meridians (the network reproduces it).
fn lower_octant(r: f64) -> Patch {
    let vs: Vec<NurbsCurve> = [0.0, 0.5, 1.0]
        .iter()
        .map(|f| {
            let q = longitude(*f);
            // From the equator down: x along q, y along z, swept back.
            let frame = Frame::new(v(0.0, 0.0, 0.0), v(q.y, -q.x, 0.0), q).unwrap();
            arc(&frame, r, 0.0, -FRAC_PI_2).unwrap()
        })
        .collect();
    let middle = vs[0].eval(0.5);
    let parallel = Frame::new(v(0.0, 0.0, middle.z), v(0.0, 0.0, 1.0), v(1.0, 0.0, 0.0)).unwrap();
    let pole = v(0.0, 0.0, -r);
    let us = vec![
        arc(&Frame::WORLD, r, 0.0, FRAC_PI_2).unwrap(),
        arc(&parallel, middle.x, 0.0, FRAC_PI_2).unwrap(),
        line(pole, pole),
    ];
    let free = [Condition::Free; 4];
    gordon(&Network {
        u: &us,
        v: &vs,
        edges: free,
        tolerance: 1e-6 * r,
        angular: 1e-9,
    })
    .unwrap()
}

/// The normal curvature of `s` across its edge v = v0 in the direction it
/// leaves it, and the unit normal, at `u`.
fn across(s: &NurbsSurface, u: f64, v0: f64) -> (Vec3, f64, Vec3) {
    let wrapped = Surface::Nurbs(s.clone());
    let (p, du, dv) = wrapped.derivatives(u, v0);
    let (_, _, dvv) = second_derivatives(&wrapped, u, v0);
    let n = du.cross(dv).normalized().unwrap();
    (p, n.dot(dvv) / dv.dot(dv), n)
}

fn samples() -> impl Iterator<Item = f64> {
    (0..=16).map(|k| f64::from(k) / 16.0)
}

#[test]
fn a_dome_held_in_curvature_to_a_sphere_takes_its_curvature_across_the_equator() {
    for scale in SCALES {
        let r = 1.5 * scale;
        let sphere = Surface::Sphere(Sphere::new(Frame::WORLD, r).unwrap());
        let (us, vs) = dome(r);
        let held = Condition::Curvature {
            surface: &sphere,
            weight: 1.0,
        };
        let patch = gordon(&net(&us, &vs, first(held), scale)).unwrap();
        let seam = patch.seams[0].expect("the seam is measured");
        assert_eq!(seam.order, Order::G2, "{seam:?}");
        for u in samples() {
            let (p, k, n) = across(&patch.surface, u, 0.0);
            assert!((p.length() - r).abs() <= 1e-9 * scale, "off the sphere");
            let outward = p.normalized().unwrap();
            assert!(n.cross(outward).length() <= 1e-9, "tangent plane");
            // The sphere bends away from its outward normal by 1/r in
            // every direction.
            let k_out = k * n.dot(outward).signum();
            assert!(
                (k_out + 1.0 / r).abs() <= 1e-8 / r,
                "curvature across {k_out:e}, the sphere's {:e} (scale {scale})",
                -1.0 / r
            );
        }
        // Held only tangent, the same dome is G1 and not G2: the measure
        // tells them apart.
        let tangent = Condition::Tangent {
            surface: &sphere,
            weight: 1.0,
        };
        let g1 = gordon(&net(&us, &vs, first(tangent), scale)).unwrap();
        let edge = g1.boundary[0].clone().unwrap();
        let c = continuity(
            &Surface::Nurbs(g1.surface.clone()),
            &sphere,
            &edge,
            1e-6 * scale,
            1e-9,
        )
        .unwrap();
        assert_eq!(c.order, Order::G1, "{c:?}");
        assert!(c.curvature.found > c.curvature.threshold, "{c:?}");
        assert!(c.angle.within() && c.position.within(), "{c:?}");
    }
}

#[test]
fn a_dome_held_in_curvature_to_a_network_octant_of_the_sphere_is_g2_to_it() {
    for scale in SCALES {
        let r = 1.5 * scale;
        let octant = lower_octant(r);
        let face = Surface::Nurbs(octant.surface.clone());
        let (us, vs) = dome(r);
        let held = Condition::Curvature {
            surface: &face,
            weight: 1.0,
        };
        let patch = gordon(&net(&us, &vs, first(held), scale)).unwrap();
        let seam = patch.seams[0].expect("the seam is measured");
        assert_eq!(seam.order, Order::G2, "{seam:?}");
        for u in samples() {
            let (p, k, n) = across(&patch.surface, u, 0.0);
            let outward = p.normalized().unwrap();
            let k_out = k * n.dot(outward).signum();
            assert!(
                (k_out + 1.0 / r).abs() <= 1e-8 / r,
                "curvature across {k_out:e}, the sphere's {:e} (scale {scale})",
                -1.0 / r
            );
        }
        // The two patches measured the other way round agree.
        let edge = patch.boundary[0].clone().unwrap();
        let back = continuity(
            &face,
            &Surface::Nurbs(patch.surface.clone()),
            &edge,
            1e-6 * scale,
            1e-9,
        )
        .unwrap();
        assert_eq!(back.order, Order::G2, "{back:?}");
    }
}

/// A blend between the top rulings of two parallel cylinders about z: the
/// first of radius `r1` about (−d, 0), the second of `r2` about (d, 0).
fn blend(scale: f64) -> (Surface, Surface, Vec<NurbsCurve>, Vec<NurbsCurve>, f64, f64) {
    let (d, r1, r2, h) = (2.0 * scale, 1.0 * scale, 0.6 * scale, 1.5 * scale);
    let frame = |x: f64| Frame::new(v(x, 0.0, 0.0), v(0.0, 0.0, 1.0), v(1.0, 0.0, 0.0)).unwrap();
    let a = Surface::Cylinder(Cylinder::new(frame(-d), r1).unwrap());
    let b = Surface::Cylinder(Cylinder::new(frame(d), r2).unwrap());
    let us = vec![
        line(v(-d, r1, 0.0), v(-d, r1, h)),
        line(v(d, r2, 0.0), v(d, r2, h)),
    ];
    let vs = [0.0, 0.5, 1.0]
        .iter()
        .map(|f| {
            let z = h * f;
            bezier([
                v(-d, r1, z),
                v(-d + 0.8 * scale, r1, z),
                v(d - 0.8 * scale, r2, z),
                v(d, r2, z),
            ])
        })
        .collect();
    (a, b, us, vs, r1, r2)
}

#[test]
fn a_blend_held_in_curvature_to_two_cylinders_takes_their_curvatures() {
    for scale in SCALES {
        let (a, b, us, vs, r1, r2) = blend(scale);
        let edges = [
            Condition::Curvature {
                surface: &a,
                weight: 1.0,
            },
            Condition::Curvature {
                surface: &b,
                weight: 1.0,
            },
            Condition::Free,
            Condition::Free,
        ];
        let patch = gordon(&net(&us, &vs, edges, scale)).unwrap();
        for e in 0..2 {
            let seam = patch.seams[e].expect("measured");
            assert_eq!(seam.order, Order::G2, "edge {e}: {seam:?}");
        }
        // Across each ruling the section bends down (towards the axis
        // below it) by the cylinder's 1/R.
        for (v0, r) in [(0.0, r1), (1.0, r2)] {
            for u in samples() {
                let (_, k, n) = across(&patch.surface, u, v0);
                assert!(n.cross(v(0.0, 1.0, 0.0)).length() <= 1e-9, "tangent plane");
                let k_up = k * n.y.signum();
                assert!(
                    (k_up + 1.0 / r).abs() <= 1e-8 / r,
                    "curvature across {k_up:e}, the cylinder's {:e} (scale {scale})",
                    -1.0 / r
                );
            }
        }
        // Held tangent only, the blend is G1 at both rulings.
        let tangent = [
            Condition::Tangent {
                surface: &a,
                weight: 1.0,
            },
            Condition::Tangent {
                surface: &b,
                weight: 1.0,
            },
            Condition::Free,
            Condition::Free,
        ];
        let g1 = gordon(&net(&us, &vs, tangent, scale)).unwrap();
        let edge = g1.boundary[0].clone().unwrap();
        let c = continuity(
            &Surface::Nurbs(g1.surface.clone()),
            &a,
            &edge,
            1e-6 * scale,
            1e-9,
        )
        .unwrap();
        assert_eq!(c.order, Order::G1, "{c:?}");
    }
}

/// The surface with its third row of control points across v = 0 moved
/// by `delta` along +y: the edge and its tangent planes stay, its
/// curvature across changes.
fn disturbed(s: &NurbsSurface, delta: f64) -> NurbsSurface {
    let (pu, pv) = s.degrees();
    let (nu, nv) = s.counts();
    let mut points = s.control_points().to_vec();
    for i in 0..nu {
        points[i * nv + 2] += v(0.0, delta, 0.0);
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

/// The blend's normal curvature across v = 0 at `u`, signed towards +y
/// (the side its rulings' faces face).
fn across_y(s: &NurbsSurface, u: f64) -> f64 {
    let (_, k, n) = across(s, u, 0.0);
    k * n.y.signum()
}

/// The largest change in curvature across v = 0 from `s` to `t`, at the
/// samples, by the test's own evaluation.
fn step(s: &NurbsSurface, t: &NurbsSurface) -> f64 {
    samples()
        .map(|u| (across_y(t, u) - across_y(s, u)).abs())
        .fold(0.0, f64::max)
}

#[test]
fn nearly_g2_is_not_g2() {
    for scale in SCALES {
        let (a, b, us, vs, _, _) = blend(scale);
        let edges = [
            Condition::Curvature {
                surface: &a,
                weight: 1.0,
            },
            Condition::Curvature {
                surface: &b,
                weight: 1.0,
            },
            Condition::Free,
            Condition::Free,
        ];
        let patch = gordon(&net(&us, &vs, edges, scale)).unwrap();
        let edge = patch.boundary[0].clone().unwrap();
        let (tol, ang) = (1e-6 * scale, 1e-9);
        let exact =
            continuity(&Surface::Nurbs(patch.surface.clone()), &a, &edge, tol, ang).unwrap();
        assert_eq!(exact.order, Order::G2);
        // The disturbance moves S_vv along y only and leaves the normal
        // (y) and S_v, so the curvature across changes by a step linear in
        // it and the same all along the edge: found here independently,
        // with a probe large enough that rounding does not blur it.
        let probe = 1e-3 * scale;
        let moved_surface = disturbed(&patch.surface, probe);
        let dk = step(&patch.surface, &moved_surface);
        for u in samples() {
            let here = (across_y(&moved_surface, u) - across_y(&patch.surface, u)).abs();
            assert!((here - dk).abs() <= 1e-9 * dk, "{here:e} vs {dk:e}");
        }
        // The measure's gap is that step (the undisturbed seam's own gap
        // is far below it).
        let moved = continuity(&Surface::Nurbs(moved_surface), &a, &edge, tol, ang).unwrap();
        assert!(
            (moved.curvature.found - dk).abs() <= 1e-6 * dk,
            "measured {:e}, the step {dk:e} (scale {scale})",
            moved.curvature.found
        );
        assert!(moved.curvature.bound >= dk * (1.0 - 1e-9), "{moved:?}");
        let per = dk / probe;
        let threshold = (ang / moved.length).min(2.0 * tol / (moved.length * moved.length));
        assert_eq!(threshold, moved.curvature.threshold);
        // Ten times the threshold: G1, its position and angle untouched.
        let big_surface = disturbed(&patch.surface, 10.0 * threshold / per);
        let big_step = step(&patch.surface, &big_surface);
        assert!(
            (big_step - 10.0 * threshold).abs() <= 1e-3 * big_step,
            "{big_step:e}"
        );
        let big = continuity(&Surface::Nurbs(big_surface), &a, &edge, tol, ang).unwrap();
        assert_eq!(big.order, Order::G1, "{big:?}");
        assert!(big.curvature.beyond(), "{big:?}");
        assert!(big.position.bound <= 1e-12 * scale, "{big:?}");
        // A hundredth of it: still G2.
        let small = continuity(
            &Surface::Nurbs(disturbed(&patch.surface, 0.01 * threshold / per)),
            &a,
            &edge,
            tol,
            ang,
        )
        .unwrap();
        assert_eq!(small.order, Order::G2, "{small:?}");
    }
}

/// A sheet along the edge y = z = 0 of the plane z = 0, `l` long and wide,
/// whose cross tangent turns about the edge by φ(u) ≈ `amplitude` ·
/// sin(8πu): flat across (S_vv = 0) and never steeper than `amplitude`,
/// but twisted, n·S_uv ≈ l φ′(u).
fn twisted(l: f64, amplitude: f64) -> NurbsSurface {
    let spans = 64;
    let mut knots = vec![0.0; 4];
    knots.extend((1..spans).map(|k| f64::from(k) / f64::from(spans)));
    knots.extend([1.0; 4]);
    let nu = spans as usize + 3;
    let mut points = Vec::new();
    for i in 0..nu {
        // Greville abscissae: the sheet runs along x at unit speed.
        let g = (knots[i + 1] + knots[i + 2] + knots[i + 3]) / 3.0;
        let phi = amplitude * (8.0 * std::f64::consts::PI * g).psin();
        for vj in [0.0, 0.5, 1.0] {
            points.push(v(g * l, vj * l, vj * l * phi));
        }
    }
    NurbsSurface::new(
        3,
        2,
        knots,
        vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        nu,
        3,
        points,
        vec![1.0; nu * 3],
    )
    .unwrap()
}

#[test]
fn a_twist_within_the_angular_tolerance_is_not_g2() {
    for scale in SCALES {
        let (tol, ang) = (1e-6 * scale, 1e-9);
        let s = twisted(scale, 0.5 * ang);
        let plane = Surface::Plane(Plane::new(Frame::WORLD));
        let edge = line(v(0.0, 0.0, 0.0), v(scale, 0.0, 0.0));
        let c = continuity(&Surface::Nurbs(s.clone()), &plane, &edge, tol, ang).unwrap();
        // Independently: the angle to the plane, the curvature across and
        // the twist, n·S_uv / (|S_u| |S_v|), along the edge.
        let wrapped = Surface::Nurbs(s);
        let (mut angle, mut bend, mut twist) = (0.0_f64, 0.0_f64, 0.0_f64);
        for k in 0..=1024 {
            let u = f64::from(k) / 1024.0;
            let (_, du, dv) = wrapped.derivatives(u, 0.0);
            let (_, duv, dvv) = second_derivatives(&wrapped, u, 0.0);
            let n = du.cross(dv).normalized().unwrap();
            angle = angle.max(n.cross(v(0.0, 0.0, 1.0)).length().pasin());
            bend = bend.max(n.dot(dvv).abs() / dv.dot(dv));
            twist = twist.max(n.dot(duv).abs() / (du.length() * dv.length()));
        }
        let threshold = (ang / scale).min(2.0 * tol / (scale * scale));
        assert!(
            angle <= ang && bend <= 1e-3 * threshold,
            "{angle:e} {bend:e}"
        );
        assert!(twist >= 10.0 * threshold, "{twist:e} vs {threshold:e}");
        // G1 by the measure, and not G2.
        assert!(c.position.within() && c.angle.within(), "{c:?}");
        assert_eq!(c.order, Order::G1, "scale {scale}: {c:?}");
        // By its twist: beyond the threshold, its bound covering what the
        // test found; flat across, so the curvature across is G2's.
        assert!(c.twist.beyond(), "{c:?}");
        assert!(c.twist.bound >= twist * (1.0 - 1e-9), "{c:?} vs {twist:e}");
        assert!(c.curvature.within(), "{c:?}");
    }
}

/// The dome over a quarter of the equator of the sphere of radius `r`
/// about `centre`.
fn dome_at(r: f64, centre: Vec3) -> (Vec<NurbsCurve>, Vec<NurbsCurve>) {
    let base = Frame::new(centre, v(0.0, 0.0, 1.0), v(1.0, 0.0, 0.0)).unwrap();
    let top = Frame::new(
        centre + v(0.0, 0.0, 0.8 * r),
        v(0.0, 0.0, 1.0),
        v(1.0, 0.0, 0.0),
    )
    .unwrap();
    let us = vec![
        arc(&base, r, 0.0, FRAC_PI_2).unwrap(),
        arc(&top, 0.7 * r, 0.0, FRAC_PI_2).unwrap(),
    ];
    let vs = [0.0, 0.5, 1.0]
        .iter()
        .map(|f| {
            let d = longitude(*f);
            bezier([
                centre + d * r,
                centre + d * r + v(0.0, 0.0, 0.3 * r),
                centre + d * (0.85 * r) + v(0.0, 0.0, 0.65 * r),
                centre + d * (0.7 * r) + v(0.0, 0.0, 0.8 * r),
            ])
        })
        .collect();
    (us, vs)
}

/// The dome held in curvature to its sphere a million units from the
/// origin, at every scale: G2, and the patch's curvature across, on an
/// exact copy of it moved back to the origin, is the sphere's within what
/// G2 promises.
#[test]
fn a_dome_held_in_curvature_far_from_the_origin_is_g2() {
    let centre = v(1.0e6, -1.0e6, 1.0e6);
    for scale in SCALES {
        let r = 1.5 * scale;
        // What coordinates of the centre's size can carry: their rounding,
        // with room for the construction's arithmetic.
        let carried = 4096.0 * f64::EPSILON * centre.length();
        let tol = (1e-6 * scale).max(carried);
        let ang = 1e-9_f64.max(carried / scale);
        let sphere = Surface::Sphere(
            Sphere::new(
                Frame::new(centre, v(0.0, 0.0, 1.0), v(1.0, 0.0, 0.0)).unwrap(),
                r,
            )
            .unwrap(),
        );
        let (us, vs) = dome_at(r, centre);
        let held = Condition::Curvature {
            surface: &sphere,
            weight: 1.0,
        };
        let patch = gordon(&Network {
            u: &us,
            v: &vs,
            edges: first(held),
            tolerance: tol,
            angular: ang,
        })
        .unwrap();
        let seam = patch.seams[0].expect("the seam is measured");
        assert_eq!(seam.order, Order::G2, "scale {scale}: {seam:?}");
        // The copy at the origin: differences of nearby doubles are exact,
        // so it is the same surface (but at the largest scale, where the
        // patch reaches the origin and the copy is within a rounding).
        let s = &patch.surface;
        let (pu, pv) = s.degrees();
        let (nu, nv) = s.counts();
        let copy = NurbsSurface::new(
            pu,
            pv,
            s.knots_u().to_vec(),
            s.knots_v().to_vec(),
            nu,
            nv,
            s.control_points().iter().map(|p| *p - centre).collect(),
            s.weights().to_vec(),
        )
        .unwrap();
        for u in samples() {
            let (p, k, n) = across(&copy, u, 0.0);
            let outward = p.normalized().unwrap();
            let k_out = k * n.dot(outward).signum();
            assert!(
                (k_out + 1.0 / r).abs() <= seam.curvature.threshold + 1e-8 / r,
                "curvature across {k_out:e}, the sphere's {:e} (scale {scale})",
                -1.0 / r
            );
        }
    }
}

#[test]
fn refusals_name_their_reasons() {
    let scale = 1.0;
    let (a, _, us, vs, _, _) = blend(scale);
    // A torus.
    let torus = Surface::Torus(Torus::new(Frame::WORLD, 3.0, 1.0).unwrap());
    let held = Condition::Curvature {
        surface: &torus,
        weight: 1.0,
    };
    let err = gordon(&net(&us, &vs, first(held), scale)).unwrap_err();
    assert!(
        matches!(&err, NetworkError::Refused(w) if w.contains("torus")),
        "{err}"
    );
    // Two analytic faces are not compared.
    let plane = Surface::Plane(Plane::new(Frame::WORLD));
    let err = continuity(&plane, &a, &us[0], 1e-6, 1e-9).unwrap_err();
    assert!(
        matches!(&err, NetworkError::Refused(w) if w.contains("analytic")),
        "{err}"
    );
    // An edge that is not a parameter line of the spline face.
    let free = [Condition::Free; 4];
    let patch = gordon(&net(&us, &vs, free, scale)).unwrap();
    let s = Surface::Nurbs(patch.surface.clone());
    let diagonal = line(patch.surface.eval(0.0, 0.0), patch.surface.eval(1.0, 1.0));
    let err = continuity(&s, &a, &diagonal, 1e-6, 1e-9).unwrap_err();
    assert!(
        matches!(&err, NetworkError::Refused(w) if w.contains("parameter line")),
        "{err}"
    );
    // Both edges of one family held in curvature to spline faces.
    let face = Surface::Nurbs(patch.surface.clone());
    let both = [
        Condition::Curvature {
            surface: &face,
            weight: 1.0,
        },
        Condition::Curvature {
            surface: &face,
            weight: 1.0,
        },
        Condition::Free,
        Condition::Free,
    ];
    // Built since 2026-11-01 (the modern mouse's top, G2 to both its
    // flanks; docs/design/surfacing.md): the crossings of the two edges
    // agree, and both seams are proven G2 against the face they lie on.
    let held = gordon(&net(&us, &vs, both, scale)).unwrap();
    for seam in [held.seams[0], held.seams[1]] {
        let seam = seam.expect("both seams are measured");
        assert_eq!(seam.order, Order::G2, "{seam:?}");
    }
    // Bad tolerances.
    assert!(continuity(&s, &a, &us[0], 0.0, 1e-9).is_err());
    assert!(continuity(&s, &a, &us[0], 1e-6, f64::NAN).is_err());
}

#[test]
fn a_patch_measured_against_itself_along_an_inner_line_is_g2() {
    // Any spline face is G2 to itself along one of its parameter lines.
    let (_, _, us, vs, _, _) = blend(1.0);
    let patch = gordon(&net(&us, &vs, [Condition::Free; 4], 1.0)).unwrap();
    let s = Surface::Nurbs(patch.surface.clone());
    let iso = line(patch.surface.eval(0.0, 0.5), patch.surface.eval(1.0, 0.5));
    let c = continuity(&s, &s, &iso, 1e-6, 1e-9).unwrap();
    assert_eq!(c.order, Order::G2, "{c:?}");
}

/// A sheet flat in the plane z = 0 along the edge y = z = 0, `l` long and
/// wide, of 64 spans along it and degree 2 across, whose third row of
/// control points across is lifted by `delta` in z: along the edge its
/// curvature across is `2 delta / l²` and its tangent plane the plane's.
fn lifted(l: f64, delta: f64) -> NurbsSurface {
    let spans = 64;
    let mut knots = vec![0.0; 4];
    knots.extend((1..spans).map(|k| f64::from(k) / f64::from(spans)));
    knots.extend([1.0; 4]);
    let nu = spans as usize + 3;
    let mut points = Vec::new();
    for i in 0..nu {
        let g = (knots[i + 1] + knots[i + 2] + knots[i + 3]) / 3.0;
        points.push(v(g * l, 0.0, 0.0));
        points.push(v(g * l, 0.5 * l, 0.0));
        points.push(v(g * l, l, delta));
    }
    NurbsSurface::new(
        3,
        2,
        knots,
        vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        nu,
        3,
        points,
        vec![1.0; nu * 3],
    )
    .unwrap()
}

/// The tighter rounding allowance (per coordinate, from the rows that
/// make a row; 2026-10-31) still refuses a seam that is not G2: a long
/// seam of many spans along a plane, its second row across lifted so that
/// the curvature across is three thresholds off, is G1 by its curvature,
/// the bound covering the closed form; lifted to a tenth of a threshold
/// it is proven G2.
#[test]
fn a_plane_seam_with_its_second_row_lifted_is_g1_by_its_curvature() {
    for scale in SCALES {
        let (tol, ang) = (1e-6 * scale, 1e-9);
        let plane = Surface::Plane(Plane::new(Frame::WORLD));
        let edge = line(v(0.0, 0.0, 0.0), v(scale, 0.0, 0.0));
        let threshold = (ang / scale).min(2.0 * tol / (scale * scale));
        for (times, order) in [(3.0, Order::G1), (0.1, Order::G2)] {
            // Curvature across 2 delta / l² = times · threshold.
            let delta = 0.5 * times * threshold * scale * scale;
            let s = lifted(scale, delta);
            let c = continuity(&Surface::Nurbs(s), &plane, &edge, tol, ang).unwrap();
            assert_eq!(c.order, order, "scale {scale}, {times} thresholds: {c:?}");
            let exact = times * threshold;
            assert!(
                c.curvature.bound >= exact * (1.0 - 1e-6),
                "scale {scale}: the bound {:e} is below the curvature {exact:e}",
                c.curvature.bound
            );
            if order == Order::G1 {
                assert!(c.curvature.beyond(), "{c:?}");
                assert!(c.angle.within() && c.twist.within(), "{c:?}");
            }
        }
    }
}

/// Two sheets meeting along a long straight seam of `l`, of degree 5 and
/// 48 knot spans along it, the spans alternately long and ten times
/// shorter (as a Gordon surface's knots crowd where its curves are
/// re-parametrised, the tank's side: spans of 0.022 among 0.2) and the
/// control points along it spaced unevenly, so that the speed along the
/// seam changes and the second derivatives along it are large; across,
/// one quadratic span. `b` leaves the seam towards +y, `a` towards −y as
/// the same polynomial continued (its rows the blossoms `b₀`, `2b₀ − b₁`,
/// `4b₀ − 4b₁ + b₂`), so the two are one surface, G2 exactly; then `a`'s
/// third row is lifted by `delta` along the seam's unit normal, which
/// changes its normal curvature across by `2 delta / |S_v|²` and nothing
/// of its position and tangent plane along the seam.
///
/// `rational`: the weights change along the seam (the same for every row
/// across), which leaves every cross-section as it is (`S = x(u) +
/// Σⱼ Bⱼ(v) rⱼ`, the rational part only in `x(u)`), so the closed form
/// holds, while the differences along have unequal weights.
fn long_seam(l: f64, delta: f64, rational: bool) -> (NurbsSurface, NurbsSurface, NurbsCurve, f64) {
    let (p, spans) = (5, 48);
    let mut lengths = Vec::new();
    for k in 0..spans {
        lengths.push(if k % 2 == 0 { 1.0 } else { 0.1 });
    }
    let total: f64 = lengths.iter().sum();
    let mut knots = vec![0.0; p + 1];
    let mut run = 0.0;
    for d in lengths.iter().take(spans - 1) {
        run += d;
        knots.push(run / total);
    }
    knots.extend([1.0; 6]);
    let nu = spans + p;
    let (w, h) = (0.25 * l, 0.1 * l);
    // Along: the Greville abscissae bent, so the speed is not even.
    let xs: Vec<f64> = (0..nu)
        .map(|i| {
            let g = knots[i + 1..=i + p].iter().sum::<f64>() / p as f64;
            l * (g + 0.15 * g * (1.0 - g) * (3.0 * g).psin())
        })
        .collect();
    let rows_b = [v(0.0, 0.0, 0.0), v(0.0, 0.5 * w, h), v(0.0, w, 0.0)];
    let rows_a = [
        rows_b[0],
        rows_b[0] * 2.0 - rows_b[1],
        rows_b[0] * 4.0 - rows_b[1] * 4.0 + rows_b[2],
    ];
    // The seam's normal: along x, S_v of `a` there is 2 (a₁ − a₀).
    let n = v(1.0, 0.0, 0.0)
        .cross(rows_a[1] - rows_a[0])
        .normalized()
        .unwrap();
    // Smooth in the parameter (of the Greville abscissae), so the weight's
    // derivatives along are of the curve's own size.
    let greville: Vec<f64> = (0..nu)
        .map(|i| knots[i + 1..=i + p].iter().sum::<f64>() / p as f64)
        .collect();
    let weight = |k: usize| {
        if rational {
            1.0 + 0.4 * (std::f64::consts::PI * greville[k]).psin()
        } else {
            1.0
        }
    };
    let sheet = |rows: [Vec3; 3], lift: Vec3| {
        let mut points = Vec::new();
        let mut weights = Vec::new();
        for (k, x) in xs.iter().enumerate() {
            for (j, r) in rows.iter().enumerate() {
                let lifted = if j == 2 { lift } else { v(0.0, 0.0, 0.0) };
                points.push(v(*x, 0.0, 0.0) + *r + lifted);
                weights.push(weight(k));
            }
        }
        NurbsSurface::new(
            p,
            2,
            knots.clone(),
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            nu,
            3,
            points,
            weights,
        )
        .unwrap()
    };
    let b = sheet(rows_b, v(0.0, 0.0, 0.0));
    let a = sheet(rows_a, n * delta);
    let edge = craft3d_geom::network::iso_curve(&b, true, 0.0).unwrap();
    let sv = (rows_a[1] - rows_a[0]) * 2.0;
    (a, b, edge, sv.length().ppowi(2))
}

/// The rows formed from the stored net by differences (2026-10-31, the
/// tank's long seams) bound a long seam of many uneven spans at the
/// default tolerances, and still refuse one that is not G2: lifted to
/// 1.5 curvature thresholds it is G1 by its curvature (seen beyond the
/// threshold, the bound covering the closed form), lifted to half a
/// threshold it is proven G2; polynomial and rational (weights changing
/// along the seam). Before, the rows along were the cut row
/// differentiated, which multiplied the positions' rounding by `2p / Δ`
/// each time, and the G2 seam stayed unproven at every scale.
#[test]
fn a_long_seam_of_many_spans_lifted_just_past_the_threshold_is_g1() {
    for (scale, rational) in SCALES.iter().flat_map(|s| [(*s, false), (*s, true)]) {
        let (tol, ang) = (1e-6 * scale, 1e-9);
        for (times, order) in [(1.5, Order::G1), (0.5, Order::G2)] {
            let (_, _, probe, sv2) = long_seam(scale, 0.0, rational);
            let length = probe
                .control_points()
                .windows(2)
                .map(|w| w[0].distance(w[1]))
                .sum::<f64>();
            let threshold = (ang / length).min(2.0 * tol / (length * length));
            let delta = 0.5 * times * threshold * sv2;
            let (a, b, edge, _) = long_seam(scale, delta, rational);
            let c = continuity(&Surface::Nurbs(a), &Surface::Nurbs(b), &edge, tol, ang).unwrap();
            assert_eq!(
                c.order, order,
                "scale {scale}, rational {rational}, {times} thresholds: {c:?}"
            );
            let exact = times * threshold;
            assert!(
                c.curvature.bound >= exact * (1.0 - 1e-6),
                "scale {scale}: the bound {:e} is below the curvature {exact:e}",
                c.curvature.bound
            );
            if order == Order::G1 {
                assert!(c.curvature.beyond(), "scale {scale}: {c:?}");
                assert!(
                    c.angle.within() && c.twist.within() && c.along.within(),
                    "{c:?}"
                );
            }
        }
    }
}

/// Two distinct knots of a face within rounding of each other along the
/// edge are one break for the measure; the piece after them would reach
/// past its knot span, so the measure refuses the seam rather than leave
/// the sliver between the knots unmeasured (2026-11-01, review of
/// r1-g2long).
#[test]
fn knots_within_rounding_of_each_other_are_refused() {
    for scale in SCALES {
        let knots = vec![0.0, 0.0, 0.0, 0.0, 0.5, 0.5 + 2e-13, 1.0, 1.0, 1.0, 1.0];
        let rows_b = [v(0.0, 0.0, 0.0), v(0.0, 0.25, 0.1), v(0.0, 0.5, 0.0)];
        let rows_a = [
            rows_b[0],
            rows_b[0] * 2.0 - rows_b[1],
            rows_b[0] * 4.0 - rows_b[1] * 4.0 + rows_b[2],
        ];
        let sheet = |rows: [Vec3; 3]| {
            let mut points = Vec::new();
            for k in 0..6 {
                for r in rows {
                    points.push((v(f64::from(k) / 5.0, 0.0, 0.0) + r) * scale);
                }
            }
            NurbsSurface::new(
                3,
                2,
                knots.clone(),
                vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                6,
                3,
                points,
                vec![1.0; 18],
            )
            .unwrap()
        };
        let (a, b) = (sheet(rows_a), sheet(rows_b));
        let edge = craft3d_geom::network::iso_curve(&b, true, 0.0).unwrap();
        let tol = 1e-6 * scale;
        match continuity(&Surface::Nurbs(a), &Surface::Nurbs(b), &edge, tol, 1e-9) {
            Err(NetworkError::Refused(why)) => {
                assert!(why.contains("two distinct knots"), "scale {scale}: {why}")
            }
            other => panic!("scale {scale}: {other:?}"),
        }
    }
}

/// Against a quadric the measure compares with the level surface through
/// the patch's point, not with the face itself: a quarter cylinder of
/// radius `R + δ` along a cylinder of radius `R` (a position gap `δ`
/// within the tolerance) is that level surface exactly, while its
/// curvature across differs from the face's by `δ / (R (R + δ))`, 500
/// curvature thresholds here. The bound must cover that difference
/// (2026-11-01: `shift_terms`, `H · H B / (G (G − H B))`), so the seam is
/// not G2; before, it was "proven" G2.
#[test]
fn a_cylinder_one_tolerance_out_is_not_g2_to_the_cylinder() {
    for scale in SCALES {
        let (r, h) = (scale, scale);
        let delta = 0.5e-6 * scale;
        let rp = r + delta;
        let s = std::f64::consts::FRAC_1_SQRT_2;
        let mut points = Vec::new();
        let mut weights = Vec::new();
        for z in [0.0, h] {
            for (p, w) in [
                (v(rp, 0.0, z), 1.0),
                (v(rp, rp, z), s),
                (v(0.0, rp, z), 1.0),
            ] {
                points.push(p);
                weights.push(w);
            }
        }
        let patch = NurbsSurface::new(
            1,
            2,
            vec![0.0, 0.0, 1.0, 1.0],
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            2,
            3,
            points,
            weights,
        )
        .unwrap();
        let edge = craft3d_geom::network::iso_curve(&patch, true, 0.0).unwrap();
        let frame = Frame::new(v(0.0, 0.0, 0.0), v(0.0, 0.0, 1.0), v(1.0, 0.0, 0.0)).unwrap();
        let cylinder = Surface::Cylinder(Cylinder::new(frame, r).unwrap());
        let (tol, ang) = (1e-6 * scale, 1e-9);
        let c = continuity(&Surface::Nurbs(patch), &cylinder, &edge, tol, ang).unwrap();
        let mismatch = delta / (r * rp);
        assert!(
            c.position.within() && c.angle.within(),
            "scale {scale}: {c:?}"
        );
        assert_eq!(c.order, Order::G1, "scale {scale}: {c:?}");
        assert!(
            c.curvature.bound >= mismatch,
            "scale {scale}: the bound {:e} is below the curvature difference {mismatch:e}",
            c.curvature.bound
        );
    }
}

/// A flat bicubic patch over x in [0, 1] and y in [y0, y0 + 0.5] (z = 0),
/// its control points along x at `xs` (uniform: x is its parameter; else
/// the same line run at another pace).
fn flat(xs: [f64; 4], y0: f64) -> NurbsSurface {
    let mut ctrl = Vec::new();
    for x in xs {
        for j in 0..4 {
            ctrl.push(v(x, y0 + 0.5 * f64::from(j) / 3.0, 0.0));
        }
    }
    let knots = vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0];
    NurbsSurface::new(3, 3, knots.clone(), knots, 4, 4, ctrl, vec![1.0; 16]).unwrap()
}

/// Both edges of one family held in curvature to spline faces whose lines
/// the crossing curves cross at different parameters: one line can give
/// the family its crossing parameters, not two, so it is refused, naming
/// how far apart they are.
#[test]
fn two_curvature_edges_whose_lines_are_crossed_at_different_parameters_are_refused() {
    let third = [0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0];
    // The face below y = 0 runs along x evenly; the one above y = 1 at
    // another pace (its points along x crowded at the start).
    let below = Surface::Nurbs(flat(third, -0.5));
    let above = Surface::Nurbs(flat([0.0, 0.05, 0.3, 1.0], 1.0));
    let us = vec![
        bezier([
            v(0.0, 0.0, 0.0),
            v(1.0 / 3.0, 0.0, 0.0),
            v(2.0 / 3.0, 0.0, 0.0),
            v(1.0, 0.0, 0.0),
        ]),
        bezier([
            v(0.0, 1.0, 0.0),
            v(1.0 / 3.0, 1.0, 0.0),
            v(2.0 / 3.0, 1.0, 0.0),
            v(1.0, 1.0, 0.0),
        ]),
    ];
    let vs: Vec<NurbsCurve> = [0.0, 0.5, 1.0]
        .iter()
        .map(|&x| {
            bezier([
                v(x, 0.0, 0.0),
                v(x, 1.0 / 3.0, 0.1),
                v(x, 2.0 / 3.0, 0.1),
                v(x, 1.0, 0.0),
            ])
        })
        .collect();
    let both = [
        Condition::Curvature {
            surface: &below,
            weight: 1.0,
        },
        Condition::Curvature {
            surface: &above,
            weight: 1.0,
        },
        Condition::Free,
        Condition::Free,
    ];
    let err = gordon(&net(&us, &vs, both, 1.0)).unwrap_err();
    assert!(
        matches!(&err, NetworkError::Refused(w) if w.contains("different parameters")),
        "{err}"
    );
    // Held to the evenly run face on both sides, the same network is built.
    let even = Surface::Nurbs(flat(third, 1.0));
    let agreeing = [
        Condition::Curvature {
            surface: &below,
            weight: 1.0,
        },
        Condition::Curvature {
            surface: &even,
            weight: 1.0,
        },
        Condition::Free,
        Condition::Free,
    ];
    assert!(gordon(&net(&us, &vs, agreeing, 1.0)).is_ok());
}

/// `b` of a knotted seam: degree `p` along the edge (the x axis, `l`
/// long) with the single knots `inner`, the profile `z = c N_j(u)` (the
/// basis function nearest the middle, scaled so that its largest
/// curvature along the edge is `peak`), swept along +y; its control points
/// along moved off the Greville abscissae by `±shift` in turn, so that it
/// runs along the seam at another pace than `a` and the points compared
/// lie up to about `shift` apart along it, across the breaks too. `a` is
/// the flat sheet on the −y side, of the same degree and knots, on the
/// Greville abscissae. The seam is G1 (the profile's slope stays far
/// within the angular tolerance) and its gap along is `b`'s curvature
/// along: returned is its largest value, measured on `b` itself.
fn knotted(
    l: f64,
    p: usize,
    inner: &[f64],
    peak: f64,
    shift: f64,
) -> (NurbsSurface, NurbsSurface, f64) {
    let mut knots = vec![0.0; p + 1];
    knots.extend_from_slice(inner);
    knots.extend(vec![1.0; p + 1]);
    let nu = knots.len() - p - 1;
    let greville: Vec<f64> = (0..nu)
        .map(|i| knots[i + 1..=i + p].iter().sum::<f64>() / p as f64)
        .collect();
    // The basis function whose support is centred nearest the middle.
    let j = (0..nu)
        .min_by(|&x, &y| {
            let centre = |i: usize| 0.5 * (knots[i] + knots[i + p + 1]) - 0.5;
            centre(x).abs().total_cmp(&centre(y).abs())
        })
        .unwrap();
    let sheet = |side: f64, moved: f64, z: &dyn Fn(usize) -> f64| {
        let mut points = Vec::new();
        for (i, g) in greville.iter().enumerate() {
            // The ends stay: the two edges' ends meet.
            let off = if i == 0 || i + 1 == nu {
                0.0
            } else if i % 2 == 0 {
                moved
            } else {
                -moved
            };
            for k in 0..3 {
                points.push(v(g * l + off, side * 0.05 * l * f64::from(k), z(i)));
            }
        }
        NurbsSurface::new(
            p,
            2,
            knots.clone(),
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            nu,
            3,
            points,
            vec![1.0; nu * 3],
        )
        .unwrap()
    };
    // The largest normal curvature along the edge, `n·S_uu / |S_u|²`,
    // sampled finely and at the knots from both sides.
    let most = |s: &NurbsSurface| {
        let surface = Surface::Nurbs(s.clone());
        let mut at: Vec<f64> = (0..=4000).map(|k| f64::from(k) / 4000.0).collect();
        for k in inner {
            at.extend([k - 1e-12, *k + 1e-12]);
        }
        let mut most = 0.0_f64;
        for u in at {
            let (_, su, sv) = s.derivatives(u, 0.0);
            let n = su.cross(sv).normalized().unwrap();
            let (suu, _, _) = second_derivatives(&surface, u, 0.0);
            most = most.max(n.dot(suu).abs() / su.dot(su));
        }
        most
    };
    let unit = most(&sheet(1.0, shift, &|i| if i == j { l } else { 0.0 }));
    let c = peak / unit;
    let b = sheet(1.0, shift, &|i| if i == j { c * l } else { 0.0 });
    let a = sheet(-1.0, 0.0, &|_| 0.0);
    let measured = most(&b);
    (a, b, measured)
}

/// The breaks of a knotted `b` (`Between`): a seam whose curvature along
/// peaks just past the threshold among `b`'s single knots (at a knot for
/// degree 3, between them for degree 4), with `b` run along the seam at
/// another pace than `a` (the points compared up to a third of the
/// tolerance apart, so the shift reaches across the breaks), is not
/// proven G2 but found beyond it, whether `b`'s rows are C¹ through its
/// knots (degree 4: the expansion along `b` goes on into the neighbour,
/// `Between::Smooth`, and no break is rough) or not (degree 3: its second
/// derivatives jump there, `Between::Rough`); at half the threshold it is
/// proven G2. Three scales.
#[test]
fn a_curvature_peak_among_the_knots_of_b_just_past_the_threshold_is_not_g2() {
    let inner = [0.2, 0.4, 0.45, 0.5, 0.55, 0.6, 0.8];
    for scale in SCALES {
        let (tol, ang) = (1e-6 * scale, 1e-9);
        let threshold = (ang / scale).min(2.0 * tol / (scale * scale));
        let edge = line(v(0.0, 0.0, 0.0), v(scale, 0.0, 0.0));
        for p in [3, 4] {
            for (times, order) in [(1.2, Order::G1), (0.5, Order::G2)] {
                let (a, b, peak) = knotted(scale, p, &inner, times * threshold, tol / 3.0);
                let c =
                    continuity(&Surface::Nurbs(a), &Surface::Nurbs(b), &edge, tol, ang).unwrap();
                let what = format!("scale {scale}, degree {p}, {times} thresholds: {c:?}");
                assert_eq!(c.order, order, "{what}");
                assert!(!c.exhausted, "{what}");
                assert!(
                    c.along.bound >= peak * (1.0 - 1e-9),
                    "{what}: peak {peak:e}"
                );
                if p == 4 {
                    assert!(c.rough_breaks == 0 && c.smooth_breaks > 0, "{what}");
                } else {
                    assert!(c.rough_breaks > 0, "{what}");
                }
                if order == Order::G1 {
                    assert!(c.along.beyond() && c.angle.within(), "{what}");
                }
            }
        }
    }
}
