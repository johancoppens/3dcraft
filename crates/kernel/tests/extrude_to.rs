//! Extrusion up to a face: closed-form volumes and areas for a profile
//! that ends on a plane, a cylinder, a sphere, a torus and a freeform
//! wall, at three scales and in a tilted frame; the names of what is
//! made; and what is refused.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;

use common::{circle, frames, line, rectangle};
use craft3d_geom::Surface;
use craft3d_kernel::KernelError;
use craft3d_kernel::extrude::{OpResult, Role, extrude};
use craft3d_kernel::extrude_to::{Target, extrude_up_to, profile_box};
use craft3d_kernel::primitives;
use craft3d_kernel::profile::{Curve2, Profile, SketchCurve, Spline2, detect_profiles};
use craft3d_math::{Frame, Tolerance};
use craft3d_topo::{Body, FaceId, check, mass_properties};
use std::f64::consts::PI;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

fn profiles(curves: &[SketchCurve], k: f64) -> Vec<Profile> {
    detect_profiles(curves, &tolerance(k)).unwrap()
}

/// A frame placed in `base`: origin and axes given in `base`'s own
/// coordinates.
fn within(base: &Frame, origin: [f64; 3], z: [f64; 3], x: [f64; 3]) -> Frame {
    let v = |a: [f64; 3]| base.vector(a[0], a[1], a[2]);
    Frame::new(base.point(origin[0], origin[1], origin[2]), v(z), v(x)).unwrap()
}

fn face_on(body: &Body, pick: impl Fn(&Surface) -> bool) -> Vec<FaceId> {
    let found: Vec<FaceId> = body
        .faces
        .iter()
        .enumerate()
        .filter(|(_, f)| pick(&f.surface))
        .map(|(i, _)| FaceId(i as u32))
        .collect();
    assert!(!found.is_empty(), "no such face");
    found
}

fn measured(made: &OpResult, k: f64) -> (f64, f64) {
    assert_eq!(check(&made.body, &tolerance(k)), vec![], "valid body");
    let mass = mass_properties(&made.body).unwrap();
    (mass.volume, mass.area)
}

fn assert_close(actual: f64, expected: f64, relative: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= relative * expected.abs(),
        "{what}: {actual}, expected {expected} (off by {:e})",
        (actual - expected) / expected
    );
}

fn roles(made: &OpResult) -> Vec<(Role, Vec<u64>)> {
    let mut all: Vec<(Role, Vec<u64>)> = made
        .origins
        .faces
        .iter()
        .map(|o| (o.role, o.sources.clone()))
        .collect();
    all.sort();
    all
}

/// A round profile up to an inclined plane: the volume is that of a
/// cylinder of the mean height, the end an ellipse of area πa²/cos θ, the
/// wall 2πa times the mean height.
#[test]
fn a_round_profile_up_to_an_inclined_plane_has_the_mean_height() {
    for k in SCALES {
        for base in frames(k) {
            let (a, h) = (2.0 * k, 7.0 * k);
            let tilt = 0.4_f64;
            // A lying cylinder whose floor is the plane: through (0, 0, h),
            // tilted about y.
            let floor = within(
                &base,
                [0.0, 0.0, h],
                [tilt.psin(), 0.0, tilt.pcos()],
                [0.0, 1.0, 0.0],
            );
            let target = primitives::cylinder(&floor, 20.0 * k, 3.0 * k).unwrap();
            // The floor is the plane face through the frame's origin.
            let faces = face_on(
                &target,
                |s| matches!(s, Surface::Plane(p) if p.frame().origin().distance(floor.origin()) < 1e-9 * k),
            );
            let round = profiles(&[circle(5, [0.0, 0.0], a)], k);
            let made = extrude_up_to(
                &round,
                &base,
                false,
                &Target {
                    body: &target,
                    faces: &faces,
                },
                &tolerance(k),
            )
            .unwrap();
            let (volume, area) = measured(&made, k);
            assert_close(volume, PI * a * a * h, 1e-9, "volume");
            assert_close(
                area,
                PI * a * a + 2.0 * PI * a * h + PI * a * a / tilt.pcos(),
                1e-9,
                "area",
            );
            assert_eq!(
                roles(&made),
                vec![
                    (Role::StartCap, vec![5]),
                    (Role::EndCap, vec![5]),
                    (Role::Side, vec![5]),
                ]
            );
            // The end is a plane, the start the sketch plane.
            let end = made
                .origins
                .faces
                .iter()
                .position(|o| o.role == Role::EndCap)
                .unwrap();
            assert!(matches!(made.body.faces[end].surface, Surface::Plane(_)));
            // One round edge at each end and their vertices, named as a
            // plain extrusion names them.
            let mut edges: Vec<Role> = made.origins.edges.iter().map(|o| o.role).collect();
            edges.sort();
            assert_eq!(edges, vec![Role::StartEdge, Role::EndEdge]);
            assert!(
                made.origins
                    .vertices
                    .iter()
                    .all(|o| o.sources == vec![5, 5])
            );
        }
    }
}

/// The same against the normal: the plane lies below the sketch plane.
#[test]
fn reversed_goes_against_the_normal() {
    let k = 1.0;
    let base = Frame::WORLD;
    let block = primitives::block(
        &within(
            &base,
            [-10.0, -10.0, -9.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 0.0],
        ),
        20.0,
        20.0,
        4.0,
    )
    .unwrap();
    // The top of the block, at z = -5.
    let top = face_on(
        &block,
        |s| matches!(s, Surface::Plane(p) if (p.frame().origin().z + 5.0).abs() < 1e-9),
    );
    let target = Target {
        body: &block,
        faces: &top,
    };
    let square = profiles(&rectangle(1, 1, (-1.0, 2.0), (0.0, 2.0)), k);
    let made = extrude_up_to(&square, &base, true, &target, &tolerance(k)).unwrap();
    let (volume, area) = measured(&made, k);
    assert_close(volume, 3.0 * 2.0 * 5.0, 1e-12, "volume");
    assert_close(area, 2.0 * 6.0 + 10.0 * 5.0, 1e-12, "area");
    let bbox = made.body.bbox().unwrap().corners().unwrap();
    assert!((bbox.0.z + 5.0).abs() < 1e-6 && bbox.1.z.abs() < 1e-6);
    // The start cap is the one in the sketch plane.
    let start = made
        .origins
        .faces
        .iter()
        .position(|o| o.role == Role::StartCap)
        .unwrap();
    let Surface::Plane(plane) = &made.body.faces[start].surface else {
        panic!("a flat cap")
    };
    assert!(plane.frame().origin().z.abs() < 1e-9);
    // Along the normal the block is behind the sketch plane.
    let error = extrude_up_to(&square, &base, false, &target, &tolerance(k)).unwrap_err();
    assert!(
        error.to_string().contains("behind the sketch plane"),
        "{error}"
    );
}

/// A rectangle `2a` wide across a lying cylinder of radius `r` whose axis
/// is at height `h`, `l` long along it: under the arch
/// `l (2 a h − a √(r² − a²) − r² asin(a / r))`.
#[test]
fn a_rectangle_up_to_a_cylinder_ends_on_its_arch() {
    for k in SCALES {
        for base in frames(k) {
            let (a, l, r, h) = (1.5 * k, 4.0 * k, 2.5 * k, 6.0 * k);
            // Lying along the sketch's y, long enough to cover the profile.
            let lying = within(&base, [0.0, -3.0 * k, h], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]);
            let target = primitives::cylinder(&lying, r, 10.0 * k).unwrap();
            let wall = face_on(&target, |s| matches!(s, Surface::Cylinder(_)));
            let rect = profiles(&rectangle(1, 1, (-a, a), (0.0, l)), k);
            let made = extrude_up_to(
                &rect,
                &base,
                false,
                &Target {
                    body: &target,
                    faces: &wall,
                },
                &tolerance(k),
            )
            .unwrap();
            let (volume, area) = measured(&made, k);
            let drop = (r * r - a * a).sqrt();
            let segment = a * drop + r * r * (a / r).pasin();
            let front = 2.0 * a * h - segment;
            assert_close(volume, l * front, 1e-9, "volume");
            assert_close(
                area,
                2.0 * a * l + 2.0 * front + 2.0 * l * (h - drop) + 2.0 * r * (a / r).pasin() * l,
                1e-9,
                "area",
            );
            assert_eq!(
                roles(&made),
                vec![
                    (Role::StartCap, vec![1, 2, 3, 4]),
                    (Role::EndCap, vec![1, 2, 3, 4]),
                    (Role::Side, vec![1]),
                    (Role::Side, vec![2]),
                    (Role::Side, vec![3]),
                    (Role::Side, vec![4]),
                ]
            );
            // Four edges at each end, four upright, and their corners.
            let count = |role: Role| made.origins.edges.iter().filter(|o| o.role == role).count();
            assert_eq!(
                (
                    count(Role::StartEdge),
                    count(Role::EndEdge),
                    count(Role::LateralEdge)
                ),
                (4, 4, 4)
            );
            let corners = |role: Role| {
                made.origins
                    .vertices
                    .iter()
                    .filter(|o| o.role == role)
                    .count()
            };
            assert_eq!(
                (corners(Role::StartVertex), corners(Role::EndVertex)),
                (4, 4)
            );
        }
    }
}

/// A round profile of radius `a` on the axis of a ball of radius `r`,
/// centre at height `h`: from outside it ends on the near side,
/// `π a² h − (2π/3)(r³ − (r² − a²)^{3/2})`; from a sketch plane inside the
/// ball on the far side, with the sign of the cap the other way.
#[test]
fn a_round_profile_up_to_a_sphere_from_outside_and_from_inside() {
    for k in SCALES {
        for base in frames(k) {
            let (a, r) = (1.5 * k, 4.0 * k);
            let drop = (r * r - a * a).sqrt();
            let cap_volume = 2.0 * PI / 3.0 * (r.ppowi(3) - drop.ppowi(3));
            let cap_area = 2.0 * PI * r * (r - drop);
            for (h, inside) in [(9.0 * k, false), (1.0 * k, true)] {
                let centre = within(&base, [0.0, 0.0, h], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]);
                let ball = primitives::sphere(&centre, r).unwrap();
                let round = profiles(&[circle(3, [0.0, 0.0], a)], k);
                let made = extrude_up_to(
                    &round,
                    &base,
                    false,
                    &Target {
                        body: &ball,
                        faces: &[FaceId(0)],
                    },
                    &tolerance(k),
                )
                .unwrap();
                let (volume, area) = measured(&made, k);
                let (expected, height) = if inside {
                    (PI * a * a * h + cap_volume, h + drop)
                } else {
                    (PI * a * a * h - cap_volume, h - drop)
                };
                assert_close(volume, expected, 1e-9, "volume");
                assert_close(
                    area,
                    PI * a * a + 2.0 * PI * a * height + cap_area,
                    1e-9,
                    "area",
                );
                let end = made
                    .origins
                    .faces
                    .iter()
                    .position(|o| o.role == Role::EndCap)
                    .unwrap();
                assert!(matches!(made.body.faces[end].surface, Surface::Sphere(_)));
            }
        }
    }
}

/// A round profile down onto the top of a ring: it ends on the tube. The
/// volume is checked against the same solid cut from a plain extrusion by
/// counting, which does not go through the boolean's result: by Pappus the
/// part of the profile's column inside the tube is not a closed form here,
/// so the check is that the body is valid, lies between the two flat
/// bounds, and ends on the torus.
#[test]
fn a_round_profile_up_to_a_torus_ends_on_the_tube() {
    for k in SCALES {
        let base = Frame::WORLD;
        let (major, minor, a, h) = (5.0 * k, 1.5 * k, 0.5 * k, 6.0 * k);
        let ring = primitives::torus(&base, major, minor).unwrap();
        // Above the tube's crest, coming down.
        let above = within(&base, [major, 0.0, h], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]);
        let round = profiles(&[circle(2, [0.0, 0.0], a)], k);
        let made = extrude_up_to(
            &round,
            &above,
            true,
            &Target {
                body: &ring,
                faces: &[FaceId(0)],
            },
            &tolerance(k),
        )
        .unwrap();
        let (volume, _) = measured(&made, k);
        // The crest is at `minor`; the lowest the end gets is where the
        // profile's rim meets the tube.
        let low = (minor * minor - a * a).sqrt();
        assert!(volume > PI * a * a * (h - minor) && volume < PI * a * a * (h - low));
        let end = made
            .origins
            .faces
            .iter()
            .position(|o| o.role == Role::EndCap)
            .unwrap();
        assert!(matches!(made.body.faces[end].surface, Surface::Torus(_)));
    }
}

fn knots(degree: usize, count: usize) -> Vec<f64> {
    let spans = count - degree;
    let mut knots = vec![0.0; degree + 1];
    knots.extend((1..spans).map(|i| i as f64 / spans as f64));
    knots.extend(vec![1.0; degree + 1]);
    knots
}

/// The value at `u` of the spline function with these coefficients, by de
/// Boor's algorithm.
fn de_boor(degree: usize, knots: &[f64], coefficients: &[f64], u: f64) -> f64 {
    let n = coefficients.len();
    let span = (degree..n).rev().find(|&s| knots[s] <= u).unwrap_or(degree);
    let mut d: Vec<f64> = coefficients[span - degree..=span].to_vec();
    for r in 1..=degree {
        for j in (r..=degree).rev() {
            let i = span - degree + j;
            let alpha = (u - knots[i]) / (knots[i + degree + 1 - r] - knots[i]);
            d[j] = d[j - 1] + (d[j] - d[j - 1]) * alpha;
        }
    }
    d[degree]
}

/// A block with a wavy underside: above the spline `y = f(x)` for `x` from
/// -6 to 6, up to y = 12, from z = 0 to z = 5 (all times `k`), in `base`.
/// The spline's control points lie at its Greville abscissae in x, so x
/// runs evenly with the parameter and the spline is the graph of a spline
/// function with the given heights as coefficients.
fn wavy_block(base: &Frame, degree: usize, heights: &[f64], k: f64) -> (Body, Vec<FaceId>) {
    let knots = knots(degree, heights.len());
    let control: Vec<[f64; 2]> = heights
        .iter()
        .enumerate()
        .map(|(i, y)| {
            let greville: f64 = knots[i + 1..=i + degree].iter().sum::<f64>() / degree as f64;
            [(-6.0 + 12.0 * greville) * k, y * k]
        })
        .collect();
    let (first, last) = (control[0], control[control.len() - 1]);
    let curves = vec![
        SketchCurve {
            id: 1,
            curve: Curve2::Spline(Spline2 {
                degree,
                knots,
                control,
            }),
        },
        line(2, last, [6.0 * k, 12.0 * k]),
        line(3, [6.0 * k, 12.0 * k], [-6.0 * k, 12.0 * k]),
        line(4, [-6.0 * k, 12.0 * k], first),
    ];
    let made = extrude(&profiles(&curves, k), base, 0.0, 5.0 * k, &tolerance(k)).unwrap();
    let wall = made
        .origins
        .faces
        .iter()
        .position(|o| o.role == Role::Side && o.sources == vec![1])
        .unwrap();
    assert!(matches!(made.body.faces[wall].surface, Surface::Nurbs(_)));
    (made.body, vec![FaceId(wall as u32)])
}

/// The plane y = 0 of `base`, looking along +y: sketch x is the base's x,
/// sketch y is the base's -z.
fn front_plane(base: &Frame) -> Frame {
    within(base, [0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0])
}

/// A rectangle up to the underside of a block whose underside is a
/// parabola (a degree-2 B-spline wall, one span): the volume is the
/// rectangle's height times the area under the parabola, the end its arc
/// length times the height; both closed forms.
#[test]
fn a_rectangle_up_to_a_parabolic_wall_has_the_area_under_the_parabola() {
    for k in SCALES {
        for base in frames(k) {
            // y(x) = 4 + 8 t (1 - t) with t = (x + 6) / 12.
            let (target, wall) = wavy_block(&base, 2, &[4.0, 8.0, 4.0], k);
            let (x0, x1, z0, z1) = (-2.0, 3.0, 1.0, 4.0);
            let rect = profiles(&rectangle(1, 1, (x0 * k, x1 * k), (-z1 * k, -z0 * k)), k);
            let made = extrude_up_to(
                &rect,
                &front_plane(&base),
                false,
                &Target {
                    body: &target,
                    faces: &wall,
                },
                &tolerance(k),
            )
            .unwrap();
            let (volume, area) = measured(&made, k);
            let t = |x: f64| (x + 6.0) / 12.0;
            let y = |x: f64| 4.0 + 8.0 * t(x) * (1.0 - t(x));
            let primitive =
                |x: f64| 12.0 * (4.0 * t(x) + 4.0 * t(x).ppowi(2) - 8.0 / 3.0 * t(x).ppowi(3));
            let under = primitive(x1) - primitive(x0);
            // The slope is s = (2 - 4t) / 3, and dx = -9 ds.
            let slope = |x: f64| (2.0 - 4.0 * t(x)) / 3.0;
            let arc = |s: f64| (s * (1.0 + s * s).sqrt() + s.pasinh()) / 2.0;
            let length = 9.0 * (arc(slope(x0)) - arc(slope(x1)));
            let height = z1 - z0;
            assert_close(volume, height * under * k.ppowi(3), 1e-7, "volume");
            assert_close(
                area,
                ((x1 - x0) * height + height * length + 2.0 * under + height * (y(x0) + y(x1)))
                    * k
                    * k,
                1e-7,
                "area",
            );
            let end = made
                .origins
                .faces
                .iter()
                .position(|o| o.role == Role::EndCap)
                .unwrap();
            assert!(matches!(made.body.faces[end].surface, Surface::Nurbs(_)));
            assert_eq!(made.origins.faces.len(), 6);
        }
    }
}

/// The same on a cubic wall of two spans, with a round profile: the volume
/// is the integral of the wall's height over the disc, by Simpson's rule
/// on the spline function evaluated by de Boor's algorithm.
#[test]
fn a_round_profile_up_to_a_cubic_wall_follows_it() {
    for k in SCALES {
        let base = Frame::WORLD;
        let heights = [4.0, 6.5, 3.0, 6.0, 4.5];
        let (target, wall) = wavy_block(&base, 3, &heights, k);
        let (cx, cz, a) = (0.5, 2.5, 1.5);
        let round = profiles(&[circle(9, [cx * k, -cz * k], a * k)], k);
        let made = extrude_up_to(
            &round,
            &front_plane(&base),
            false,
            &Target {
                body: &target,
                faces: &wall,
            },
            &tolerance(k),
        )
        .unwrap();
        let (volume, _) = measured(&made, k);
        // ∫ y(x) 2 √(a² − (x − cx)²) dx with x = cx + a sin φ, which takes
        // the square root out of the integrand.
        let knots = knots(3, heights.len());
        let y = |x: f64| de_boor(3, &knots, &heights, (x + 6.0) / 12.0);
        let steps = 20_000;
        let f = |i: usize| {
            let phi = -PI / 2.0 + PI * i as f64 / steps as f64;
            y(cx + a * phi.psin()) * 2.0 * a * a * phi.pcos().ppowi(2)
        };
        let mut sum = f(0) + f(steps);
        for i in 1..steps {
            sum += f(i) * if i % 2 == 1 { 4.0 } else { 2.0 };
        }
        let expected = sum * (PI / steps as f64) / 3.0;
        assert_close(volume, expected * k.ppowi(3), 1e-6, "volume");
    }
}

/// Two profiles, one with a hole, up to one inclined plane: a shell each,
/// each with its own end cap, and the volume of each is its area times
/// the height of the plane over its centroid.
#[test]
fn every_profile_gets_its_own_end() {
    let k = 1.0;
    let base = Frame::WORLD;
    // Its underside is the plane z = 4 - 0.1 x.
    let roof = primitives::block(
        &within(&base, [-20.0, -20.0, 6.0], [0.1, 0.0, 1.0], [1.0, 0.0, 0.0]),
        60.0,
        60.0,
        2.0,
    )
    .unwrap();
    let under = face_on(
        &roof,
        |s| matches!(s, Surface::Plane(p) if p.frame().z().z < -0.9),
    );
    let mut curves = rectangle(1, 1, (0.0, 4.0), (0.0, 4.0));
    curves.push(circle(5, [2.0, 2.0], 1.0));
    curves.push(circle(6, [8.0, 2.0], 1.0));
    let both = profiles(&curves, k);
    assert_eq!(both.len(), 2);
    let made = extrude_up_to(
        &both,
        &base,
        false,
        &Target {
            body: &roof,
            faces: &under,
        },
        &tolerance(k),
    )
    .unwrap();
    assert_eq!(made.body.shells.len(), 2);
    let (volume, _) = measured(&made, k);
    assert_close(
        volume,
        (16.0 - PI) * (4.0 - 0.2) + PI * (4.0 - 0.8),
        1e-9,
        "volume",
    );
    assert_eq!(
        roles(&made),
        vec![
            (Role::StartCap, vec![1, 2, 3, 4]),
            (Role::StartCap, vec![6]),
            (Role::EndCap, vec![1, 2, 3, 4]),
            (Role::EndCap, vec![6]),
            (Role::Side, vec![1]),
            (Role::Side, vec![2]),
            (Role::Side, vec![3]),
            (Role::Side, vec![4]),
            (Role::Side, vec![5]),
            (Role::Side, vec![6]),
        ]
    );
}

#[test]
fn what_cannot_end_is_refused_with_its_reason() {
    let k = 1.0;
    let base = Frame::WORLD;
    let tol = tolerance(k);
    let square = profiles(&rectangle(1, 1, (-1.0, 1.0), (-1.0, 1.0)), k);
    let block = primitives::block(
        &within(&base, [-5.0, -5.0, 3.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
        10.0,
        10.0,
        2.0,
    )
    .unwrap();
    let refused =
        |profiles: &[Profile], body: &Body, faces: &[FaceId], reversed: bool| match extrude_up_to(
            profiles,
            &base,
            reversed,
            &Target { body, faces },
            &tol,
        ) {
            Err(error) => error.to_string(),
            Ok(_) => panic!("should be refused"),
        };
    // A side of the block: parallel to the direction.
    let side = face_on(
        &block,
        |s| matches!(s, Surface::Plane(p) if p.frame().z().x > 0.9),
    );
    let reason = refused(&square, &block, &side, false);
    assert!(reason.contains("parallel to the direction"), "{reason}");
    // The underside, but going down.
    let under = face_on(
        &block,
        |s| matches!(s, Surface::Plane(p) if p.frame().z().z < -0.9),
    );
    let reason = refused(&square, &block, &under, true);
    assert!(reason.contains("behind the sketch plane"), "{reason}");
    // A profile far beside the block: the plane goes on, the face does not.
    let far = profiles(&rectangle(1, 1, (40.0, 42.0), (0.0, 2.0)), k);
    let reason = refused(&far, &block, &under, false);
    assert!(reason.contains("misses the face"), "{reason}");
    // ...but one that hangs over the block's edge ends on the plane.
    let over = profiles(&rectangle(1, 1, (3.0, 8.0), (0.0, 2.0)), k);
    let made = extrude_up_to(
        &over,
        &base,
        false,
        &Target {
            body: &block,
            faces: &under,
        },
        &tol,
    )
    .unwrap();
    assert_close(measured(&made, k).0, 5.0 * 2.0 * 3.0, 1e-12, "overhang");

    // A ball the profile is too wide for: part of it passes beside.
    let ball = primitives::sphere(
        &within(&base, [0.0, 0.0, 6.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
        1.2,
    )
    .unwrap();
    let reason = refused(&square, &ball, &[FaceId(0)], false);
    assert!(reason.contains("never ends"), "{reason}");
    // A ball that dips through the sketch plane inside the profile.
    let dipping = primitives::sphere(
        &within(&base, [0.0, 0.0, 0.3], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
        0.5,
    )
    .unwrap();
    let reason = refused(&square, &dipping, &[FaceId(0)], false);
    assert!(
        reason.contains("never ends") || reason.contains("passes through the sketch plane"),
        "{reason}"
    );
    // A cylinder along the direction.
    let upright = primitives::cylinder(
        &within(&base, [0.0, 0.0, 2.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
        3.0,
        4.0,
    )
    .unwrap();
    let wall = face_on(&upright, |s| matches!(s, Surface::Cylinder(_)));
    let reason = refused(&square, &upright, &wall, false);
    assert!(reason.contains("parallel to the direction"), "{reason}");
    // A cone: not built yet.
    let cone = primitives::cone(
        &within(&base, [0.0, 0.0, 9.0], [0.0, 0.0, -1.0], [1.0, 0.0, 0.0]),
        4.0,
        4.0,
    )
    .unwrap();
    let side = face_on(&cone, |s| matches!(s, Surface::Cone(_)));
    let reason = refused(&square, &cone, &side, false);
    assert!(reason.contains("a cone face"), "{reason}");
    // A freeform wall the profile reaches beyond.
    let (wavy, wall) = wavy_block(&base, 2, &[4.0, 8.0, 4.0], k);
    let wide = profiles(&rectangle(1, 1, (-2.0, 3.0), (-9.0, -1.0)), k);
    let error = extrude_up_to(
        &wide,
        &front_plane(&base),
        false,
        &Target {
            body: &wavy,
            faces: &wall,
        },
        &tol,
    )
    .unwrap_err();
    assert!(error.to_string().contains("never ends"), "{error}");
    // A face that is not there, no profiles, a bad tolerance.
    assert!(matches!(
        extrude_up_to(
            &square,
            &base,
            false,
            &Target {
                body: &block,
                faces: &[FaceId(99)]
            },
            &tol
        ),
        Err(KernelError::NoEnd { .. })
    ));
    assert!(matches!(
        extrude_up_to(
            &[],
            &base,
            false,
            &Target {
                body: &block,
                faces: &under
            },
            &tol
        ),
        Err(KernelError::NoProfile)
    ));
    assert!(profile_box(&[]).is_none());
    let nan = Tolerance {
        linear: f64::NAN,
        angular: 1e-9,
    };
    assert!(
        extrude_up_to(
            &square,
            &base,
            false,
            &Target {
                body: &block,
                faces: &under
            },
            &nan
        )
        .is_err()
    );
}

/// `∫ f` over the disc of radius `a` about `(cx, 0)` in x, for an `f` of x
/// alone: with x = cx + a sin φ the chord's square root goes.
fn over_disc(a: f64, cx: f64, f: impl Fn(f64) -> f64) -> f64 {
    let steps = 20_000;
    let g = |i: usize| {
        let phi = -PI / 2.0 + PI * i as f64 / steps as f64;
        f(cx + a * phi.psin()) * 2.0 * a * a * phi.pcos().ppowi(2)
    };
    let mut sum = g(0) + g(steps);
    for i in 1..steps {
        sum += g(i) * if i % 2 == 1 { 4.0 } else { 2.0 };
    }
    sum * (PI / steps as f64) / 3.0
}

/// A round profile against a lying cylinder: the two cylinders meet in a
/// curve without a closed form (a spline within the tolerance). From
/// outside the profile stops under the arch, from a sketch plane through
/// the cylinder's axis it fills the bore up to the wall: the height over
/// the disc is `h ∓ √(r² − x²)`, integrated by Simpson's rule.
#[test]
fn a_round_profile_up_to_a_cylinder_from_outside_and_from_inside() {
    for k in SCALES {
        let base = Frame::WORLD;
        let (a, r) = (1.5 * k, 2.5 * k);
        for (h, cx, inside) in [(6.0 * k, 0.4 * k, false), (0.0, 0.4 * k, true)] {
            let lying = within(&base, [0.0, -8.0 * k, h], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]);
            let target = primitives::cylinder(&lying, r, 16.0 * k).unwrap();
            let wall = face_on(&target, |s| matches!(s, Surface::Cylinder(_)));
            let round = profiles(&[circle(4, [cx, 0.0], a)], k);
            let made = extrude_up_to(
                &round,
                &base,
                false,
                &Target {
                    body: &target,
                    faces: &wall,
                },
                &tolerance(k),
            )
            .unwrap();
            let (volume, _) = measured(&made, k);
            let arch = over_disc(a, cx, |x| (r * r - x * x).sqrt());
            let expected = if inside { arch } else { PI * a * a * h - arch };
            assert_close(volume, expected, 1e-6, "volume");
        }
    }
}

/// A profile with arcs, a spline and a hole up to an inclined plane: the
/// volume is the profile's area times the plane's height over its
/// centroid, whatever the outline.
#[test]
fn any_outline_up_to_an_inclined_plane_has_its_area_times_the_height_at_its_centroid() {
    for k in SCALES {
        let base = frames(k).remove(1);
        let roof = primitives::block(
            &within(
                &base,
                [-30.0 * k, -30.0 * k, 9.0 * k],
                [0.2, -0.1, 1.0],
                [1.0, 0.0, 0.0],
            ),
            80.0 * k,
            80.0 * k,
            2.0 * k,
        )
        .unwrap();
        // The underside passes through the block's corner; its normal
        // looks back at the sketch plane.
        let under = face_on(
            &roof,
            |s| matches!(s, Surface::Plane(p) if p.frame().z().dot(base.z()) < -0.9),
        );
        let p = |x: f64, y: f64| [x * k, y * k];
        let curves = vec![
            line(1, p(0.0, 0.0), p(6.0, 0.0)),
            common::arc(2, p(6.0, 2.0), p(6.0, 0.0), p(6.0, 4.0)),
            SketchCurve {
                id: 3,
                curve: Curve2::Spline(Spline2 {
                    degree: 3,
                    knots: knots(3, 5),
                    control: vec![
                        p(6.0, 4.0),
                        p(4.5, 5.5),
                        p(3.0, 3.0),
                        p(1.5, 5.5),
                        p(0.0, 4.0),
                    ],
                }),
            },
            line(4, p(0.0, 4.0), p(0.0, 0.0)),
            circle(5, p(2.0, 1.5), 0.8 * k),
        ];
        let outline = profiles(&curves, k);
        let (area, centroid) = (outline[0].area(), outline[0].centroid().unwrap());
        let made = extrude_up_to(
            &outline,
            &base,
            false,
            &Target {
                body: &roof,
                faces: &under,
            },
            &tolerance(k),
        )
        .unwrap();
        let (volume, _) = measured(&made, k);
        // z over (x, y): the plane through the corner with normal n.
        let n = [0.2, -0.1, 1.0];
        let height =
            9.0 * k - (n[0] * (centroid[0] + 30.0 * k) + n[1] * (centroid[1] + 30.0 * k)) / n[2];
        // To 1e-8, not to rounding: the edge in which the spline wall
        // meets the plane is a curve fitted within the tolerance (1e-6 of
        // the unit), so the faces close only to that and the volume
        // depends, by a part of that share, on the point it is taken
        // about. Measured: 1.7e-9 at most over the three scales, taken
        // about the middle of the body (under 1e-9 about the origin).
        assert_close(volume, area * height, 1e-8, "volume");
        assert_eq!(made.origins.faces.len(), 7);
    }
}

/// Where a wall of the profile is tangent to the surface (a profile as
/// wide as the cylinder it runs into) the boolean decides: an error, or a
/// body, and then the right one. The closed forms: the rectangle of
/// `a_rectangle_up_to_a_cylinder_ends_on_its_arch` with `a = r`; two
/// equal cylinders at right angles, of whose common part (16 r³ / 3) the
/// half below the axis is missing, `π r² h − 8 r³ / 3`; and a rectangle
/// from `x = -1` to the tangent wall at `x = r = 2`.
#[test]
fn walls_tangent_to_the_surface_give_the_right_body_or_an_error() {
    let k = 1.0;
    let base = Frame::WORLD;
    let (r, h) = (2.0_f64, 6.0);
    let lying = within(&base, [0.0, -8.0, h], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]);
    let target = primitives::cylinder(&lying, r, 16.0).unwrap();
    let wall = face_on(&target, |s| matches!(s, Surface::Cylinder(_)));
    let under = |x: f64| x / 2.0 * (r * r - x * x).sqrt() + r * r / 2.0 * (x / r).pasin();
    for (curves, expected) in [
        (
            rectangle(1, 1, (-r, r), (0.0, 3.0)),
            3.0 * (2.0 * r * h - PI * r * r / 2.0),
        ),
        (
            vec![circle(1, [0.0, 0.0], r)],
            PI * r * r * h - 8.0 * r.ppowi(3) / 3.0,
        ),
        (
            rectangle(1, 1, (-1.0, r), (0.0, 3.0)),
            3.0 * (3.0 * h - (under(r) - under(-1.0))),
        ),
    ] {
        let outline = profiles(&curves, k);
        match extrude_up_to(
            &outline,
            &base,
            false,
            &Target {
                body: &target,
                faces: &wall,
            },
            &tolerance(k),
        ) {
            Ok(made) => assert_close(measured(&made, k).0, expected, 1e-6, "tangent walls"),
            Err(error) => println!("tangent walls: {error}"),
        }
    }
}
