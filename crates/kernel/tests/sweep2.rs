//! Sweeps along two rails: a profile placed at every station so that two
//! of its points follow two curves.
//!
//! The oracles are closed forms, worked out here from the rails and the
//! profile as drawn, not from the sweep: parallel lines carry a circle into
//! a cylinder; converging lines carry it into a cone; two concentric
//! quarter arcs carry it into a quarter of a torus (Pappus); a rectangle
//! between diverging lines makes a prismatoid whose volume Simpson's rule
//! gives exactly; scaled only along the chord, a circle becomes ellipses
//! whose areas are linear in the place. Every point of the faces' meshes
//! is held to the true surface within the deviation the sweep proves.
//! Each at 1e-3, 1 and 1e6, with the tolerance a millionth of the size.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{arc, circle, line, rectangle};
use craft3d_kernel::KernelError;
use craft3d_kernel::extrude::Role;
use craft3d_kernel::profile::{Curve2, Profile, SketchCurve, Spline2, detect_profiles};
use craft3d_kernel::sweep::Swept;
use craft3d_kernel::sweep2::{Scaling, sweep_two_rails};
use craft3d_math::{Frame, Portable, Tolerance, Vec3};
use craft3d_tess::{SelfCheckParams, TessParams, body_self_intersections, tessellate_body};
use craft3d_topo::{check, mass_properties};
use std::f64::consts::{FRAC_PI_2, PI};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn tol(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

/// The plane the profiles are drawn on: through the world's origin,
/// facing −y, the sketch's x along the world's x and its y along z.
fn front() -> Frame {
    Frame::new(Vec3::ZERO, -Vec3::Y, Vec3::X).unwrap()
}

fn profile(curves: &[SketchCurve], k: f64) -> Profile {
    let profiles = detect_profiles(curves, &tol(k)).unwrap();
    assert_eq!(profiles.len(), 1);
    profiles[0].clone()
}

fn sweep(
    section: &[SketchCurve],
    a: &[SketchCurve],
    b: &[SketchCurve],
    scaling: Scaling,
    k: f64,
) -> Result<Swept, KernelError> {
    sweep_two_rails(
        &profile(section, k),
        &front(),
        a,
        &Frame::WORLD,
        b,
        &Frame::WORLD,
        scaling,
        &tol(k),
    )
}

/// Valid, outside out, not through itself, a watertight mesh; the
/// volume and area measured, and the mesh's points.
fn sound(swept: &Swept, k: f64) -> (f64, f64, Vec<Vec3>) {
    let body = &swept.body;
    assert_eq!(check(body, &tol(k)), vec![]);
    assert_eq!(body.faces.len(), swept.faces.len());
    let mut names: Vec<_> = swept.faces.iter().collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), swept.faces.len(), "two faces with one origin");
    let mass = mass_properties(body).unwrap();
    assert!(mass.volume > 0.0, "inside out: {}", mass.volume);
    let found = body_self_intersections(body, &SelfCheckParams::new(tol(k).linear)).unwrap();
    assert!(found.is_clean(), "{found}");
    let mesh = tessellate_body(
        body,
        &TessParams {
            chordal_tolerance: 1e-3 * k,
            ..TessParams::default()
        },
    )
    .unwrap();
    let welded = mesh.weld().unwrap();
    assert!(welded.is_closed(), "the mesh is not watertight");
    (mass.volume, mass.area, mesh.positions.clone())
}

#[track_caller]
fn within(found: f64, expected: f64, allowed: f64) {
    assert!(
        (found - expected).abs() <= allowed,
        "found {found}, expected {expected} (off by {:e}, {:e} allowed)",
        (found - expected).abs(),
        allowed
    );
}

fn refusal(result: Result<Swept, KernelError>) -> String {
    match result {
        Ok(_) => panic!("should have been refused"),
        Err(error) => error.to_string(),
    }
}

fn kinds(swept: &Swept, role: Role) -> Vec<&'static str> {
    swept
        .body
        .faces
        .iter()
        .zip(&swept.faces)
        .filter(|(_, origin)| origin.role == role)
        .map(|(face, _)| face.surface.kind())
        .collect()
}

#[test]
fn a_circle_between_parallel_lines_is_an_exact_cylinder() {
    for k in SCALES {
        let (r, l) = (2.0 * k, 30.0 * k);
        let pipe = sweep(
            &[circle(1, [r, 0.0], r)],
            &[line(10, [0.0, 0.0], [0.0, l])],
            &[line(20, [2.0 * r, 0.0], [2.0 * r, l])],
            Scaling::Uniform,
            k,
        )
        .unwrap();
        assert_eq!(pipe.deviation, 0.0);
        assert_eq!(
            kinds(&pipe, Role::Side),
            ["cylinder", "cylinder", "cylinder", "cylinder"]
        );
        let (volume, area, _) = sound(&pipe, k);
        within(volume, PI * r * r * l, 1e-10 * PI * r * r * l);
        within(
            area,
            2.0 * PI * r * l + 2.0 * PI * r * r,
            1e-10 * 2.0 * PI * r * l,
        );
        // Named after the curve of the profile and the two rails: one
        // name per quarter of the circle, numbered.
        let sides: Vec<_> = pipe
            .faces
            .iter()
            .filter(|o| o.role == Role::Side)
            .map(|o| (o.sources.clone(), o.piece))
            .collect();
        assert_eq!(
            sides,
            [
                (vec![1, 10, 20], 0),
                (vec![1, 10, 20], 1),
                (vec![1, 10, 20], 2),
                (vec![1, 10, 20], 3)
            ]
        );
    }
}

/// The distance of `p` from the cone about the line `x = c`, `z = 0`
/// along y whose radius is `r0 + (r1 − r0) y / l`, measured square to
/// the axis (an upper bound of the distance to the cone).
fn off_cone(p: Vec3, c: f64, r0: f64, r1: f64, l: f64) -> f64 {
    let radius = r0 + (r1 - r0) * p.y / l;
    ((p.x - c).phypot(p.z) - radius).abs()
}

#[test]
fn a_circle_between_converging_lines_is_a_cone_within_the_proven_bound() {
    for k in SCALES {
        let (r0, r1, l) = (3.0 * k, 1.5 * k, 20.0 * k);
        let c = r0;
        let cone = sweep(
            &[circle(1, [c, 0.0], r0)],
            &[line(10, [0.0, 0.0], [r0 - r1, l])],
            &[line(20, [2.0 * r0, 0.0], [r0 + r1, l])],
            Scaling::Uniform,
            k,
        )
        .unwrap();
        let t = tol(k).linear;
        assert!(cone.deviation > 0.0 && cone.deviation <= t / 2.0);
        let (volume, area, points) = sound(&cone, k);
        let slant = (r0 - r1).phypot(l);
        let lateral = PI * (r0 + r1) * slant;
        within(area, lateral + PI * (r0 * r0 + r1 * r1), 1e-6 * area);
        let exact = PI * l * (r0 * r0 + r0 * r1 + r1 * r1) / 3.0;
        within(volume, exact, area * t);
        // Every point of the mesh (on the faces' surfaces) is within the
        // proven bound of the cone, or on a cap.
        let mut worst: f64 = 0.0;
        for p in points {
            if p.y.abs() <= t || (p.y - l).abs() <= t {
                continue;
            }
            worst = worst.max(off_cone(p, c, r0, r1, l));
        }
        assert!(
            worst <= cone.deviation * 1.001 + 1e-12 * k,
            "a point is {worst} off the cone, more than the bound {}",
            cone.deviation
        );
    }
}

#[test]
fn a_circle_between_concentric_arcs_is_a_quarter_torus_by_pappus() {
    for k in SCALES {
        let (r1, r2) = (8.0 * k, 12.0 * k);
        let (centre, minor) = (0.5 * (r1 + r2), 0.5 * (r2 - r1));
        let ring = sweep(
            &[circle(1, [centre, 0.0], minor)],
            &[arc(10, [0.0, 0.0], [r1, 0.0], [0.0, r1])],
            &[arc(20, [0.0, 0.0], [r2, 0.0], [0.0, r2])],
            Scaling::Uniform,
            k,
        )
        .unwrap();
        let t = tol(k).linear;
        assert!(ring.deviation <= t / 2.0);
        let (volume, area, points) = sound(&ring, k);
        let exact = PI * minor * minor * centre * FRAC_PI_2;
        within(volume, exact, area * t);
        within(
            area,
            2.0 * PI * minor * centre * FRAC_PI_2 + 2.0 * PI * minor * minor,
            area * 1e-6,
        );
        let mut worst: f64 = 0.0;
        for p in points {
            let rho = p.x.phypot(p.y);
            let off = ((rho - centre).phypot(p.z) - minor).abs();
            if p.x > t && p.y > t {
                worst = worst.max(off);
            }
        }
        assert!(
            worst <= ring.deviation * 1.001 + 1e-12 * k,
            "{worst} > {}",
            ring.deviation
        );
    }
}

#[test]
fn a_rectangle_between_diverging_lines_is_a_prismatoid() {
    for k in SCALES {
        let (w0, w1, h, l) = (4.0 * k, 10.0 * k, 3.0 * k, 25.0 * k);
        let section = rectangle(1, 1, (0.0, w0), (0.0, h));
        let a = [line(10, [0.0, 0.0], [0.0, l])];
        let b = [line(20, [w0, 0.0], [w1, l])];
        let t = tol(k).linear;
        // Uniform: the height grows with the width, w(y) h(y) = h w(y)² / w0.
        let grown = sweep(&section, &a, &b, Scaling::Uniform, k).unwrap();
        let (volume, area, _) = sound(&grown, k);
        let exact = h / w0 * l * (w0 * w0 + w0 * w1 + w1 * w1) / 3.0;
        within(volume, exact, area * t);
        // Width only: the height stays.
        let wide = sweep(&section, &a, &b, Scaling::Width, k).unwrap();
        let (volume, area, _) = sound(&wide, k);
        within(volume, h * l * (w0 + w1) / 2.0, area * t);
        // Every face lies in a plane.
        assert!(
            kinds(&wide, Role::Side).iter().all(|kind| *kind == "plane"),
            "{:?}",
            kinds(&wide, Role::Side)
        );
    }
}

#[test]
fn a_circle_scaled_only_across_becomes_ellipses() {
    for k in SCALES {
        let (w0, w1, l) = (4.0 * k, 7.0 * k, 18.0 * k);
        let flat = sweep(
            &[circle(1, [w0 / 2.0, 0.0], w0 / 2.0)],
            &[line(10, [0.0, 0.0], [0.0, l])],
            &[line(20, [w0, 0.0], [w1, l])],
            Scaling::Width,
            k,
        )
        .unwrap();
        let (volume, area, _) = sound(&flat, k);
        // Ellipses of half-axes w(y)/2 and w0/2.
        let exact = PI * w0 / 4.0 * l * (w0 + w1) / 2.0;
        within(volume, exact, area * tol(k).linear);
    }
}

fn knots(degree: usize, count: usize) -> Vec<f64> {
    let spans = count - degree;
    let mut knots = vec![0.0; degree + 1];
    knots.extend((1..spans).map(|i| i as f64 / spans as f64));
    knots.extend(vec![1.0; degree + 1]);
    knots
}

/// A cubic spline on evenly spaced points of the line from `from` to
/// `to`: the line itself, its parameter in proportion to its length.
fn straight_spline(id: u64, from: [f64; 2], to: [f64; 2]) -> SketchCurve {
    let control: Vec<[f64; 2]> = (0..4)
        .map(|i| {
            let t = f64::from(i) / 3.0;
            [
                from[0] + (to[0] - from[0]) * t,
                from[1] + (to[1] - from[1]) * t,
            ]
        })
        .collect();
    SketchCurve {
        id,
        curve: Curve2::Spline(Spline2 {
            degree: 3,
            knots: knots(3, 4),
            control,
        }),
    }
}

#[test]
fn a_spline_rail_is_followed_by_its_parameter() {
    for k in SCALES {
        let (w0, w1, h, l) = (4.0 * k, 10.0 * k, 3.0 * k, 25.0 * k);
        let swept = sweep(
            &rectangle(1, 1, (0.0, w0), (0.0, h)),
            &[line(10, [0.0, 0.0], [0.0, l])],
            &[straight_spline(20, [w0, 0.0], [w1, l])],
            Scaling::Width,
            k,
        )
        .unwrap();
        let (volume, area, _) = sound(&swept, k);
        within(volume, h * l * (w0 + w1) / 2.0, area * tol(k).linear);
    }
}

#[test]
fn rails_of_two_curves_each_meet_at_their_joints_and_names_follow_the_pieces() {
    for k in SCALES {
        // Straight, then a quarter turn to the left, for both: the joints
        // are at different fractions of the rails' lengths, so there are
        // three legs.
        let (r1, r2, l) = (8.0 * k, 12.0 * k, 10.0 * k);
        let a = [
            line(10, [r1, -l], [r1, 0.0]),
            arc(11, [0.0, 0.0], [r1, 0.0], [0.0, r1]),
        ];
        let b = [
            line(20, [r2, -l], [r2, 0.0]),
            arc(21, [0.0, 0.0], [r2, 0.0], [0.0, r2]),
        ];
        let minor = 0.5 * (r2 - r1);
        let section = [circle(1, [0.5 * (r1 + r2), 0.0], minor)];
        let profile_plane = Frame::new(Vec3::new(0.0, -l, 0.0), -Vec3::Y, Vec3::X).unwrap();
        let swept = sweep_two_rails(
            &profile(&section, k),
            &profile_plane,
            &a,
            &Frame::WORLD,
            &b,
            &Frame::WORLD,
            Scaling::Uniform,
            &tol(k),
        )
        .unwrap();
        let (volume, area, _) = sound(&swept, k);
        assert!(volume > 0.0 && area > 0.0);
        let mut legs: Vec<Vec<u64>> = swept
            .faces
            .iter()
            .filter(|o| o.role == Role::Side)
            .map(|o| o.sources.clone())
            .collect();
        legs.dedup();
        // Rail B turns first: at 10 / (10 + 6π) of its length, rail A at
        // 10 / (10 + 4π).
        assert_eq!(legs, [vec![1, 10, 20], vec![1, 10, 21], vec![1, 11, 21]]);
    }
}

#[test]
fn rails_that_cross_are_refused_with_the_place() {
    for k in SCALES {
        let why = refusal(sweep(
            &[circle(1, [1.0 * k, 0.0], 1.0 * k)],
            &[line(10, [0.0, 0.0], [0.0, 10.0 * k])],
            &[line(20, [2.0 * k, 0.0], [-2.0 * k, 10.0 * k])],
            Scaling::Uniform,
            k,
        ));
        assert!(why.contains("meet or cross"), "{why}");
        assert!(why.contains("e10") && why.contains("e20"), "{why}");
    }
}

#[test]
fn a_rail_off_the_section_is_refused() {
    for k in SCALES {
        let why = refusal(sweep(
            &[circle(1, [1.0 * k, 0.0], 1.0 * k)],
            &[line(10, [0.0, 0.0], [0.0, 10.0 * k])],
            &[line(20, [3.0 * k, 0.0], [3.0 * k, 10.0 * k])],
            Scaling::Uniform,
            k,
        ));
        assert!(why.contains("outline of the section"), "{why}");
        assert!(why.contains("e20"), "{why}");
    }
}

#[test]
fn rails_that_start_at_one_point_are_refused() {
    for k in SCALES {
        let why = refusal(sweep(
            &[circle(1, [1.0 * k, 0.0], 1.0 * k)],
            &[line(10, [0.0, 0.0], [0.0, 10.0 * k])],
            &[line(20, [0.0, 0.0], [2.0 * k, 10.0 * k])],
            Scaling::Uniform,
            k,
        ));
        assert!(why.contains("start at one point"), "{why}");
    }
}

#[test]
fn a_section_off_the_plane_of_the_chord_and_up_is_refused() {
    for k in SCALES {
        // Drawn on a plane through both starts that leans 30° from the
        // vertical.
        let (s, c) = (PI / 6.0).psin_cos();
        let leaning = Frame::new(Vec3::ZERO, Vec3::new(0.0, -c, s), Vec3::X).unwrap();
        let why = refusal(sweep_two_rails(
            &profile(&[circle(1, [1.0 * k, 0.0], 1.0 * k)], k),
            &leaning,
            &[line(10, [0.0, 0.0], [0.0, 10.0 * k])],
            &Frame::WORLD,
            &[line(20, [2.0 * k, 0.0], [2.0 * k, 10.0 * k])],
            &Frame::WORLD,
            Scaling::Uniform,
            &tol(k),
        ));
        assert!(why.contains("up direction"), "{why}");
    }
}

#[test]
fn a_rail_along_the_section_folds_and_is_refused() {
    for k in SCALES {
        let why = refusal(sweep(
            &[circle(1, [1.0 * k, 0.0], 1.0 * k)],
            &[line(10, [0.0, 0.0], [0.0, 10.0 * k])],
            &[line(20, [2.0 * k, 0.0], [12.0 * k, 0.0])],
            Scaling::Uniform,
            k,
        ));
        assert!(why.contains("fold"), "{why}");
    }
}

#[test]
fn a_closed_rail_is_refused() {
    let why = refusal(sweep(
        &[circle(1, [1.0, 0.0], 1.0)],
        &[line(10, [0.0, 0.0], [0.0, 10.0])],
        &[circle(20, [5.0, 0.0], 3.0)],
        Scaling::Uniform,
        1.0,
    ));
    assert!(!why.is_empty());
}

#[test]
fn the_same_input_gives_the_same_body_to_the_bit() {
    let make = || {
        sweep(
            &[circle(1, [8.5, 0.0], 1.5)],
            &[arc(10, [0.0, 0.0], [7.0, 0.0], [0.0, 7.0])],
            &[line(20, [10.0, 0.0], [3.0, 12.0])],
            Scaling::Uniform,
            1.0,
        )
        .unwrap()
    };
    let (one, two) = (make(), make());
    assert_eq!(format!("{:?}", one.body), format!("{:?}", two.body));
    assert_eq!(one.deviation.to_bits(), two.deviation.to_bits());
}

/// The point at `t` of the Bézier curve on `points` (de Casteljau), the
/// test's own.
fn casteljau(points: &[[f64; 2]], t: f64) -> [f64; 2] {
    let mut d = points.to_vec();
    for r in 1..d.len() {
        for i in 0..d.len() - r {
            d[i] = [
                d[i][0] + (d[i + 1][0] - d[i][0]) * t,
                d[i][1] + (d[i + 1][1] - d[i][1]) * t,
            ];
        }
    }
    d[0]
}

/// The distance from `p` to the Bézier curve on `points`: the nearest of
/// 4000 samples, then narrowed by golden sections round it.
fn to_bezier(points: &[[f64; 2]], p: [f64; 2]) -> f64 {
    let dist = |t: f64| {
        let q = casteljau(points, t);
        (q[0] - p[0]).phypot(q[1] - p[1])
    };
    let n = 4000;
    let best = (0..=n)
        .map(|i| f64::from(i) / f64::from(n))
        .min_by(|a, b| dist(*a).total_cmp(&dist(*b)))
        .unwrap();
    let (mut lo, mut hi) = (
        (best - 1.0 / f64::from(n)).max(0.0),
        (best + 1.0 / f64::from(n)).min(1.0),
    );
    for _ in 0..100 {
        let (a, b) = (lo + (hi - lo) * 0.382, lo + (hi - lo) * 0.618);
        if dist(a) < dist(b) {
            hi = b;
        } else {
            lo = a;
        }
    }
    dist(0.5 * (lo + hi)).min(dist(best))
}

#[test]
fn a_spline_section_along_curved_spline_rails_is_within_the_proven_bound() {
    for k in SCALES {
        // Rail B is rail A moved across by d: every station is the
        // section moved along rail A, square to y. Rail A's y runs
        // linearly from 0 to 30 (its control points' y are even), so a
        // slice y = c is the section moved to A(c / 30): by Cavalieri the
        // volume is the section's area times 30, and a point of a face
        // lies within the bound of the section moved to its own y (and
        // within 1.3 of it in the slice: x moves 0.3 per unit of y at
        // most along A).
        let d = 6.0 * k;
        let rail: Vec<[f64; 2]> = [[0.0, 0.0], [3.0, 10.0], [-3.0, 20.0], [0.0, 30.0]]
            .iter()
            .map(|p| [p[0] * k, p[1] * k])
            .collect();
        let moved: Vec<[f64; 2]> = rail.iter().map(|p| [p[0] + d, p[1]]).collect();
        let cubic = |id: u64, control: &[[f64; 2]]| SketchCurve {
            id,
            curve: Curve2::Spline(Spline2 {
                degree: 3,
                knots: knots(3, 4),
                control: control.to_vec(),
            }),
        };
        let arch: Vec<[f64; 2]> = vec![[d, 0.0], [d, 4.5 * k], [0.0, 4.5 * k], [0.0, 0.0]];
        let section = [line(1, [0.0, 0.0], [d, 0.0]), cubic(2, &arch)];
        let swept = sweep(
            &section,
            &[cubic(10, &rail)],
            &[cubic(20, &moved)],
            Scaling::Uniform,
            k,
        )
        .unwrap();
        let t = tol(k).linear;
        assert!(swept.deviation > 0.0 && swept.deviation <= t / 2.0);
        let (volume, area, points) = sound(&swept, k);
        // The area under the arch by Gauss–Legendre on its own
        // polynomial: ∫ z (−dx/dt) dt, exact for this degree.
        let gauss = craft3d_math::quadrature::gauss_legendre(8);
        let mut under = 0.0;
        for (x, w) in gauss {
            let s = 0.5 * (x + 1.0);
            let h = 1e-6;
            let (a, b) = (casteljau(&arch, s - h), casteljau(&arch, s + h));
            let dx = (b[0] - a[0]) / (2.0 * h);
            under += 0.5 * w * casteljau(&arch, s)[1] * -dx;
        }
        within(volume, under * 30.0 * k, area * t + 1e-8 * volume);
        let mut worst: f64 = 0.0;
        for p in points {
            if p.y <= t || p.y >= 30.0 * k - t {
                continue;
            }
            let at = casteljau(&rail, p.y / (30.0 * k));
            let local = [p.x - at[0], p.z];
            let bottom = if local[0] < 0.0 {
                local[0].phypot(local[1])
            } else if local[0] > d {
                (local[0] - d).phypot(local[1])
            } else {
                local[1].abs()
            };
            worst = worst.max(bottom.min(to_bezier(&arch, local)));
        }
        assert!(worst > 0.0);
        assert!(
            worst <= 1.3 * swept.deviation + 1e-12 * k,
            "a point is {worst} off the section, more than the bound {}",
            swept.deviation
        );
    }
}

#[test]
fn rails_in_different_planes_tilt_the_section() {
    for k in SCALES {
        // Rail A along y on the xy plane, rail B on the plane x = d,
        // rising by h over the same run: the chord at height y runs from
        // (0, y, 0) to (d, y, h y / l), and the circle on it has half its
        // length as radius: V = π/4 (d² l + h² l / 3).
        let (d, h, l) = (4.0 * k, 3.0 * k, 20.0 * k);
        let side = Frame::new(Vec3::new(d, 0.0, 0.0), Vec3::X, Vec3::Y).unwrap();
        let swept = sweep_two_rails(
            &profile(&[circle(1, [d / 2.0, 0.0], d / 2.0)], k),
            &front(),
            &[line(10, [0.0, 0.0], [0.0, l])],
            &Frame::WORLD,
            &[line(20, [0.0, 0.0], [l, h])],
            &side,
            Scaling::Uniform,
            &tol(k),
        )
        .unwrap();
        let (volume, area, _) = sound(&swept, k);
        within(
            volume,
            PI / 4.0 * (d * d * l + h * h * l / 3.0),
            area * tol(k).linear,
        );
    }
}

#[test]
fn a_fold_in_the_middle_of_a_leg_is_refused() {
    for k in SCALES {
        // Rail B is half a circle: half way along it runs straight across,
        // along the section's own plane.
        let (d, r) = (4.0 * k, 10.0 * k);
        let why = refusal(sweep(
            &[circle(1, [d / 2.0, 0.0], d / 2.0)],
            &[line(10, [0.0, 0.0], [0.0, 40.0 * k])],
            &[arc(20, [d + r, 0.0], [d + 2.0 * r, 0.0], [d, 0.0])],
            Scaling::Uniform,
            k,
        ));
        assert!(why.contains("fold"), "{why}");
        assert!(!why.contains("(0, 0, 0)"), "{why}");
    }
}

#[test]
fn a_corner_added_to_one_rail_renames_only_the_legs_after_it() {
    let polyline = |id: u64, points: &[[f64; 2]]| SketchCurve {
        id,
        curve: Curve2::Spline(Spline2 {
            degree: 1,
            knots: knots(1, points.len()),
            control: points.to_vec(),
        }),
    };
    let sides = |swept: &Swept| -> Vec<(Vec<u64>, u32)> {
        swept
            .faces
            .iter()
            .filter(|o| o.role == Role::Side)
            .map(|o| (o.sources.clone(), o.piece))
            .collect()
    };
    let section = [circle(1, [2.0, 0.0], 2.0)];
    let a = [polyline(10, &[[0.0, 0.0], [0.0, 10.0], [-2.0, 20.0]])];
    let straight = sweep(
        &section,
        &a,
        &[line(20, [4.0, 0.0], [4.0, 20.0])],
        Scaling::Uniform,
        1.0,
    )
    .unwrap();
    let bent = sweep(
        &section,
        &a,
        &[polyline(20, &[[4.0, 0.0], [4.0, 15.0], [5.0, 20.0]])],
        Scaling::Uniform,
        1.0,
    )
    .unwrap();
    let (before, after) = (sides(&straight), sides(&bent));
    // The first piece of rail A with the first of rail B: the same four
    // quarters, numbered 0 to 3, before and after.
    assert_eq!(before[..4], after[..4]);
    assert!(before[..4].iter().all(|(s, n)| *s == [1, 10, 20] && *n < 4));
    // Rail A's second piece with rail B's first: 4096 on in both.
    let second: Vec<u32> = before[4..].iter().map(|(_, n)| *n).collect();
    assert_eq!(second, [4096, 4097, 4098, 4099]);
    assert_eq!(
        after[4..8].iter().map(|(_, n)| *n).collect::<Vec<_>>(),
        second
    );
    // And the leg on rail B's second piece is new.
    let last: Vec<u32> = after[8..].iter().map(|(_, n)| *n).collect();
    assert_eq!(last, [4100, 4101, 4102, 4103]);
}

#[test]
fn a_tolerance_too_fine_for_the_size_is_refused() {
    let why = refusal(sweep_two_rails(
        &profile(&[circle(1, [2.0e6, 0.0], 2.0e6)], 1e6),
        &front(),
        &[line(10, [0.0, 0.0], [0.0, 3.0e7])],
        &Frame::WORLD,
        &[line(20, [4.0e6, 0.0], [3.0e6, 3.0e7])],
        &Frame::WORLD,
        Scaling::Uniform,
        &Tolerance::new(1e-9, 1e-9).unwrap(),
    ));
    assert!(why.contains("too fine"), "{why}");
}
