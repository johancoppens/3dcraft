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
