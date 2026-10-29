//! Edges of network patches held to faces (`network::gordon`), checked
//! against the faces' closed-form normals, not against the patch's own
//! `tangency` report, at three scales: tangent (G1) along a circle of a
//! cylinder (the edge a rational arc), along a parallel of a sphere that is
//! not a great circle, along a circle of a cone, along a curve in a plane,
//! and along both kinds of parameter line of a closed spline face (one on
//! its seam, one ending there); an edge held in position on its face is
//! accepted and one off it refused; two held edges whose corner conflicts
//! (in their tangent planes, or only in their twist) are refused, and so
//! are crossing curves that leave a tangent edge to opposite sides (a patch
//! that would fold over its edge, its normal turned over between them).

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_geom::network::{Condition, Network, NetworkError, Patch, arc, gordon};
use craft3d_geom::{Cone, Cylinder, NurbsCurve, NurbsSurface, Plane, Sphere, Surface};
use craft3d_math::{Frame, Vec3};
use std::f64::consts::{FRAC_PI_2, PI};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];
const FRACTIONS: [f64; 3] = [0.0, 0.5, 1.0];

fn v(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3::new(x, y, z)
}

fn line(a: Vec3, b: Vec3) -> NurbsCurve {
    NurbsCurve::polynomial(1, vec![0.0, 0.0, 1.0, 1.0], vec![a, b]).unwrap()
}

fn spline(points: &[Vec3]) -> NurbsCurve {
    let n = points.len();
    let p = (n - 1).min(3);
    let mut knots = vec![0.0; p + 1];
    for k in 1..n - p {
        knots.push(k as f64 / (n - p) as f64);
    }
    knots.extend(vec![1.0; p + 1]);
    NurbsCurve::polynomial(p, knots, points.to_vec()).unwrap()
}

fn at_height(z: f64) -> Frame {
    Frame::new(v(0.0, 0.0, z), v(0.0, 0.0, 1.0), v(1.0, 0.0, 0.0)).unwrap()
}

fn network<'a>(
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

fn tangent_first(face: &Surface) -> [Condition<'_>; 4] {
    [
        Condition::Tangent {
            surface: face,
            weight: 1.0,
        },
        Condition::Free,
        Condition::Free,
        Condition::Free,
    ]
}

fn held_in_position(face: &Surface) -> [Condition<'_>; 4] {
    [
        Condition::Position(face),
        Condition::Free,
        Condition::Free,
        Condition::Free,
    ]
}

fn refused(result: Result<Patch, NetworkError>) -> String {
    match result {
        Err(NetworkError::Refused(why)) => why,
        Err(other) => panic!("expected a refusal, got the error {other}"),
        Ok(p) => panic!(
            "expected a refusal, got a patch (tangency {:?})",
            p.tangency
        ),
    }
}

/// Along the patch's first u curve (v = 0): the patch's normal against the
/// face's closed-form normal `normal(p)` at 129 places, one side all along,
/// and the edge on the face (`off(p)`, the closed-form distance).
fn meets_with_one_tangent_plane(
    patch: &Patch,
    normal: impl Fn(Vec3) -> Vec3,
    off: impl Fn(Vec3) -> f64,
    scale: f64,
) {
    let mut side = 0.0;
    for k in 0..=128 {
        let u = f64::from(k) / 128.0;
        let (p, du, dv) = patch.surface.derivatives(u, 0.0);
        assert!(
            off(p) <= 1e-9 * scale,
            "off the face by {:e} at {u}",
            off(p)
        );
        let n = du.cross(dv).normalized().unwrap();
        let m = normal(p).normalized().unwrap();
        let sine = n.cross(m).length();
        assert!(sine <= 1e-9, "tangent planes {sine:e} apart at u = {u}");
        let here = n.dot(m).signum();
        if k == 0 {
            side = here;
        }
        assert_eq!(here, side, "the normal turns over at u = {u}");
    }
    assert!(patch.tangency[0].unwrap() <= 1e-9);
}

#[test]
fn a_patch_tangent_along_a_circle_of_a_cylinder_has_its_normals() {
    for scale in SCALES {
        let (r, h) = (2.0 * scale, 3.0 * scale);
        let face = Surface::Cylinder(Cylinder::new(Frame::WORLD, r).unwrap());
        let us = vec![
            arc(&Frame::WORLD, r, 0.0, FRAC_PI_2).unwrap(),
            arc(&at_height(h), 1.5 * r, 0.0, FRAC_PI_2).unwrap(),
        ];
        // Each leaves the circle outwards and up: bent to leave along the
        // cylinder (straight up).
        let vs: Vec<NurbsCurve> = FRACTIONS
            .iter()
            .map(|f| {
                let (p0, p1) = (us[0].eval(*f), us[1].eval(*f));
                spline(&[p0, p0 * 1.2 + v(0.0, 0.0, 0.5 * h), p1])
            })
            .collect();
        let patch = gordon(&network(&us, &vs, tangent_first(&face), scale)).unwrap();
        meets_with_one_tangent_plane(
            &patch,
            |p| v(p.x, p.y, 0.0),
            |p| (v(p.x, p.y, 0.0).length() - r).abs(),
            scale,
        );
    }
}

#[test]
fn a_patch_tangent_along_a_parallel_of_a_sphere_has_its_normals() {
    for scale in SCALES {
        let big = 1.5 * scale;
        let face = Surface::Sphere(Sphere::new(Frame::WORLD, big).unwrap());
        // The parallel at 30 degrees: not a great circle.
        let phi = PI / 6.0;
        let (z0, rho) = (big * phi.sin(), big * phi.cos());
        let us = vec![
            arc(&at_height(z0), rho, 0.0, FRAC_PI_2).unwrap(),
            arc(&at_height(1.2 * big), 0.4 * big, 0.0, FRAC_PI_2).unwrap(),
        ];
        let vs: Vec<NurbsCurve> = FRACTIONS
            .iter()
            .map(|f| {
                let (p0, p1) = (us[0].eval(*f), us[1].eval(*f));
                let theta = p0.y.atan2(p0.x);
                // Up the meridian, and a little outwards: bent.
                let up = v(
                    -phi.sin() * theta.cos(),
                    -phi.sin() * theta.sin(),
                    phi.cos(),
                );
                let out = p0.normalized().unwrap();
                spline(&[p0, p0 + up * (0.5 * big) + out * (0.15 * big), p1])
            })
            .collect();
        let patch = gordon(&network(&us, &vs, tangent_first(&face), scale)).unwrap();
        meets_with_one_tangent_plane(&patch, |p| p, |p| (p.length() - big).abs(), scale);
    }
}

#[test]
fn a_patch_tangent_along_a_circle_of_a_cone_has_its_normals() {
    for scale in SCALES {
        let (r0, alpha, h) = (2.0 * scale, 0.3_f64, 3.0 * scale);
        let face = Surface::Cone(Cone::new(Frame::WORLD, r0, alpha).unwrap());
        let us = vec![
            arc(&Frame::WORLD, r0, 0.0, FRAC_PI_2).unwrap(),
            arc(&at_height(h), r0 + h * alpha.tan() + scale, 0.0, FRAC_PI_2).unwrap(),
        ];
        let vs: Vec<NurbsCurve> = FRACTIONS
            .iter()
            .map(|f| {
                let (p0, p1) = (us[0].eval(*f), us[1].eval(*f));
                let theta = p0.y.atan2(p0.x);
                let ruling = v(alpha.tan() * theta.cos(), alpha.tan() * theta.sin(), 1.0);
                let out = v(theta.cos(), theta.sin(), 0.0);
                spline(&[p0, p0 + ruling * (0.4 * h) + out * (0.3 * scale), p1])
            })
            .collect();
        let patch = gordon(&network(&us, &vs, tangent_first(&face), scale)).unwrap();
        // The cone's normal at angle θ: (cos θ, sin θ, −tan α); its radius
        // at height z: r0 + z tan α.
        meets_with_one_tangent_plane(
            &patch,
            |p| {
                let theta = p.y.atan2(p.x);
                v(theta.cos(), theta.sin(), -alpha.tan())
            },
            |p| (v(p.x, p.y, 0.0).length() - (r0 + p.z * alpha.tan())).abs() * alpha.cos(),
            scale,
        );
    }
}

#[test]
fn a_patch_tangent_to_a_plane_along_a_curve_in_it_has_its_normal() {
    for scale in SCALES {
        let face = Surface::Plane(Plane::new(Frame::WORLD));
        let us = vec![
            spline(&[
                v(0.0, 0.0, 0.0) * scale,
                v(1.0, 0.4, 0.0) * scale,
                v(2.0, -0.3, 0.0) * scale,
                v(3.0, 0.1, 0.0) * scale,
            ]),
            spline(&[
                v(0.0, 2.0, 1.0) * scale,
                v(1.5, 2.2, 1.2) * scale,
                v(3.0, 2.0, 0.8) * scale,
            ]),
        ];
        let vs: Vec<NurbsCurve> = FRACTIONS
            .iter()
            .map(|f| {
                let (p0, p1) = (us[0].eval(*f), us[1].eval(*f));
                spline(&[p0, p0 + v(0.2, 0.8, 0.6) * scale, p1])
            })
            .collect();
        let patch = gordon(&network(&us, &vs, tangent_first(&face), scale)).unwrap();
        meets_with_one_tangent_plane(&patch, |_| v(0.0, 0.0, 1.0), |p| p.z.abs(), scale);
    }
}

/// The whole cylinder of radius `r` and height `h` about the z axis as
/// one spline face: the rational circle (closed, its seam at angle 0)
/// times a line.
fn closed_cylinder(r: f64, h: f64) -> NurbsSurface {
    let circle = NurbsCurve::circle(&Frame::WORLD, r).unwrap();
    let mut ctrl = Vec::new();
    let mut weights = Vec::new();
    for (p, w) in circle.control_points().iter().zip(circle.weights()) {
        for z in [0.0, h] {
            ctrl.push(*p + v(0.0, 0.0, z));
            weights.push(*w);
        }
    }
    NurbsSurface::new(
        circle.degree(),
        1,
        circle.knots().to_vec(),
        vec![0.0, 0.0, 1.0, 1.0],
        circle.control_points().len(),
        2,
        ctrl,
        weights,
    )
    .unwrap()
}

#[test]
fn a_patch_tangent_to_a_closed_spline_face_along_its_seam_has_its_normals() {
    for scale in SCALES {
        let (r, h) = (2.0 * scale, 3.0 * scale);
        let face = Surface::Nurbs(closed_cylinder(r, h));
        assert!(face.eval(0.0, 0.0).distance(v(r, 0.0, 0.0)) <= 1e-12 * scale);
        // Along the line on the seam, out towards +y and +x.
        let seam = line(v(r, 0.0, 0.0), v(r, 0.0, h));
        let far = |z: f64| v(r + 1.5 * scale, 2.5 * scale, z + 0.2 * scale);
        let vs: Vec<NurbsCurve> = [0.0, 0.5 * h, h]
            .iter()
            .map(|z| spline(&[v(r, 0.0, *z), v(r + 0.6 * scale, 1.2 * scale, *z), far(*z)]))
            .collect();
        let us = vec![seam, line(far(0.0), far(h))];
        let patch = gordon(&network(&us, &vs, tangent_first(&face), scale)).unwrap();
        meets_with_one_tangent_plane(
            &patch,
            |p| v(p.x, p.y, 0.0),
            |p| (v(p.x, p.y, 0.0).length() - r).abs(),
            scale,
        );
    }
}

#[test]
fn a_patch_tangent_to_a_closed_spline_face_along_an_arc_ending_on_its_seam_has_its_normals() {
    for scale in SCALES {
        let (r, h) = (2.0 * scale, 3.0 * scale);
        let face = Surface::Nurbs(closed_cylinder(r, h));
        // The last quarter of the circle, from −y round to the seam at +x.
        let us = vec![
            arc(&Frame::WORLD, r, 1.5 * PI, FRAC_PI_2).unwrap(),
            arc(&at_height(h), 1.5 * r, 1.5 * PI, FRAC_PI_2).unwrap(),
        ];
        let vs: Vec<NurbsCurve> = FRACTIONS
            .iter()
            .map(|f| {
                let (p0, p1) = (us[0].eval(*f), us[1].eval(*f));
                spline(&[p0, p0 * 1.2 + v(0.0, 0.0, 0.5 * h), p1])
            })
            .collect();
        let patch = gordon(&network(&us, &vs, tangent_first(&face), scale)).unwrap();
        meets_with_one_tangent_plane(
            &patch,
            |p| v(p.x, p.y, 0.0),
            |p| (v(p.x, p.y, 0.0).length() - r).abs(),
            scale,
        );
    }
}

/// Quarter arcs of radius `r` at heights 0 and `h` with the lines between
/// them: the quarter cylinder's network.
fn quarter_cylinder(r: f64, h: f64) -> (Vec<NurbsCurve>, Vec<NurbsCurve>) {
    let us = vec![
        arc(&Frame::WORLD, r, 0.0, FRAC_PI_2).unwrap(),
        arc(&at_height(h), r, 0.0, FRAC_PI_2).unwrap(),
    ];
    let vs = FRACTIONS
        .iter()
        .map(|f| line(us[0].eval(*f), us[1].eval(*f)))
        .collect();
    (us, vs)
}

#[test]
fn an_edge_held_in_position_is_accepted_on_its_face_and_refused_off_it() {
    for scale in SCALES {
        let (r, h) = (2.0 * scale, 3.0 * scale);
        let (us, vs) = quarter_cylinder(r, h);
        let on = Surface::Cylinder(Cylinder::new(Frame::WORLD, r).unwrap());
        let off = Surface::Cylinder(Cylinder::new(Frame::WORLD, r * (1.0 + 1e-3)).unwrap());
        let patch = gordon(&network(&us, &vs, held_in_position(&on), scale)).unwrap();
        for k in 0..=32 {
            let p = patch.surface.eval(f64::from(k) / 32.0, 0.0);
            assert!((v(p.x, p.y, 0.0).length() - r).abs() <= 1e-12 * scale);
        }
        let why = refused(gordon(&network(&us, &vs, held_in_position(&off), scale)));
        assert!(why.contains("off its face"), "{why}");
    }
}

#[test]
fn two_held_edges_whose_corner_asks_two_tangent_planes_are_refused() {
    for scale in SCALES {
        // The first u curve along x held tangent to z = 0, the first v
        // curve held tangent to x = 0: at their corner the patch would
        // need both planes.
        let floor = Surface::Plane(Plane::new(Frame::WORLD));
        let wall = Surface::Plane(Plane::new(
            Frame::new(v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0), v(0.0, 1.0, 0.0)).unwrap(),
        ));
        let s = scale;
        let us = vec![
            line(v(0.0, 0.0, 0.0), v(s, 0.0, 0.0)),
            line(v(0.0, s, s), v(s, s, s)),
        ];
        let vs = vec![
            line(v(0.0, 0.0, 0.0), v(0.0, s, s)),
            line(v(s, 0.0, 0.0), v(s, s, s)),
        ];
        let edges = [
            Condition::Tangent {
                surface: &floor,
                weight: 1.0,
            },
            Condition::Free,
            Condition::Tangent {
                surface: &wall,
                weight: 1.0,
            },
            Condition::Free,
        ];
        refused(gordon(&network(&us, &vs, edges, scale)));
    }
}

#[test]
fn two_held_edges_whose_corner_asks_two_twists_are_refused() {
    for scale in SCALES {
        let s = scale;
        // The saddle z = x y / s as a bilinear patch: it holds the y axis
        // (its parameter line u = 1/2) with the normal (−y/s, 0, 1) there,
        // the plane z = 0 at the origin but twisting along it.
        let saddle = NurbsSurface::new(
            1,
            1,
            vec![0.0, 0.0, 1.0, 1.0],
            vec![0.0, 0.0, 1.0, 1.0],
            2,
            2,
            vec![v(-s, -s, s), v(-s, s, -s), v(s, -s, -s), v(s, s, s)],
            vec![1.0; 4],
        )
        .unwrap();
        let saddle = Surface::Nurbs(saddle);
        let floor = Surface::Plane(Plane::new(Frame::WORLD));
        let us = vec![
            line(v(0.0, 0.0, 0.0), v(s, 0.0, 0.0)),
            line(v(0.0, s, 0.0), v(s, s, 0.3 * s)),
        ];
        let vs = vec![
            line(v(0.0, 0.0, 0.0), v(0.0, s, 0.0)),
            line(v(s, 0.0, 0.0), v(s, s, 0.3 * s)),
        ];
        let edges = [
            Condition::Tangent {
                surface: &floor,
                weight: 1.0,
            },
            Condition::Free,
            Condition::Tangent {
                surface: &saddle,
                weight: 1.0,
            },
            Condition::Free,
        ];
        refused(gordon(&network(&us, &vs, edges, scale)));
    }
}

#[test]
fn curves_that_leave_a_tangent_edge_to_opposite_sides_are_refused() {
    for scale in SCALES {
        let s = scale;
        let face = Surface::Plane(Plane::new(Frame::WORLD));
        let us = vec![
            line(v(0.0, 0.0, 0.0), v(s, 0.0, 0.0)),
            line(v(0.0, s, 0.5 * s), v(s, s, 0.5 * s)),
        ];
        // The middle curve leaves towards −y, the others towards +y: held
        // tangent, the patch would fold over its edge between them, its
        // normal turned over (the old check, blind to the sign, passed it).
        let vs: Vec<NurbsCurve> = [(0.0, 1.0), (0.5, -1.0), (1.0, 1.0)]
            .iter()
            .map(|(x, way)| {
                spline(&[
                    v(*x, 0.0, 0.0) * s,
                    v(*x, 0.5 * way, 0.2) * s,
                    v(*x, 1.0, 0.5) * s,
                ])
            })
            .collect();
        let why = refused(gordon(&network(&us, &vs, tangent_first(&face), scale)));
        assert!(why.contains("opposite sides"), "{why}");
        // Without the condition the network is a patch.
        assert!(gordon(&network(&us, &vs, [Condition::Free; 4], scale)).is_ok());
    }
}
