//! Edges of network patches held in curvature (G2, `Condition::Curvature`)
//! and the proven `continuity` measure, at three scales, checked against
//! closed forms, not against the patch's own report: a dome held to a
//! sphere along the equator takes the sphere's normal curvature 1/R across
//! it, to the sphere itself and to a network octant of it (a spline face);
//! a blend between two parallel cylinders takes 1/R₁ and 1/R₂ across its
//! two edges; G1-only and G2 pairs are told apart, a G2 patch disturbed
//! by a curvature step ten times the threshold is not G2 (its position and
//! tangent planes untouched), one a hundredth of it is; refusals.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_geom::curvature::second_derivatives;
use craft3d_geom::network::{
    Condition, Network, NetworkError, Order, Patch, arc, continuity, gordon,
};
use craft3d_geom::{Cylinder, NurbsCurve, NurbsSurface, Plane, Sphere, Surface, Torus};
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
    v(a.cos(), a.sin(), 0.0)
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
        points[i * nv + 2] = points[i * nv + 2] + v(0.0, delta, 0.0);
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
        // A small step: well within the tolerance in position.
        let probe = 1e-9 * scale;
        let moved = continuity(
            &Surface::Nurbs(disturbed(&patch.surface, probe)),
            &a,
            &edge,
            tol,
            ang,
        )
        .unwrap();
        let per = moved.curvature.found / probe;
        assert!(per > 0.0, "{moved:?}");
        let threshold = moved.curvature.threshold;
        // Ten times the threshold: G1, its position and angle untouched.
        let big = continuity(
            &Surface::Nurbs(disturbed(&patch.surface, 10.0 * threshold / per)),
            &a,
            &edge,
            tol,
            ang,
        )
        .unwrap();
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
    let err = gordon(&net(&us, &vs, both, scale)).unwrap_err();
    assert!(
        matches!(&err, NetworkError::Refused(w) if w.contains("both edges")),
        "{err}"
    );
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
