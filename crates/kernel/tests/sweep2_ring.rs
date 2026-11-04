//! Sweeps along two rails that close into rings, and with the up
//! direction given.
//!
//! The oracles are closed forms worked out here from the rails, the
//! profile and the up direction as drawn, not from the sweep:
//!
//! - a circle carried round two concentric circles is a torus (Pappus:
//!   `2π R · π r²`, area `2π R · 2π r`), however the rings are cut into
//!   curves, and every point of a mesh of its faces lies within the
//!   proven deviation of the torus;
//! - the same with the up direction leaning by `α` from the rings' axis:
//!   the section stands in the plane of the chord and the up direction,
//!   so it leans back and forth round the ring. The volume is
//!   `∫∫∫ det(∂P/∂θ, U, B)`; with the section's centroid on the chord the
//!   terms in `y` integrate to zero and what is left is
//!   `A R_c ∫ cos α / √(1 − sin² α cos² θ) dθ = A R_c cos α · 4 K(sin α)`,
//!   `K` the complete elliptic integral of the first kind, computed here
//!   by the arithmetic–geometric mean (`K(k) = π / (2 AGM(1, √(1 − k²)))`);
//! - between parallel straight rails with the up direction leaning by `α`
//!   towards them, an oblique cylinder: `π r² L cos α`.
//!
//! Each at 1e-3, 1 and 1e6, with the tolerance a millionth of the size.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{arc, circle, line};
use craft3d_kernel::KernelError;
use craft3d_kernel::extrude::Role;
use craft3d_kernel::profile::{Profile, SketchCurve, detect_profiles};
use craft3d_kernel::sweep::Swept;
use craft3d_kernel::sweep2::{Scaling, sweep_two_rails, sweep_two_rails_up};
use craft3d_math::{Frame, Portable, Tolerance, Vec3};
use craft3d_tess::{SelfCheckParams, TessParams, body_self_intersections, tessellate_body};
use craft3d_topo::{check, mass_properties};
use std::f64::consts::PI;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn tol(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

/// Through the world's origin, facing −y: the sketch's x along the
/// world's x and its y along z.
fn front() -> Frame {
    Frame::new(Vec3::ZERO, -Vec3::Y, Vec3::X).unwrap()
}

fn profile(curves: &[SketchCurve], k: f64) -> Profile {
    let profiles = detect_profiles(curves, &tol(k)).unwrap();
    assert_eq!(profiles.len(), 1);
    profiles[0].clone()
}

/// Valid, outside out, not through itself, a watertight mesh, faces
/// with distinct origins; the volume, the area and the mesh's points.
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

/// The point at `degrees` on the circle of radius `r` about the origin.
fn on(r: f64, degrees: f64) -> [f64; 2] {
    let (s, c) = (degrees * PI / 180.0).psin_cos();
    [r * c, r * s]
}

/// A ring of radius `r` about the origin drawn as two arcs that meet at
/// `at` and `at + 180` degrees.
fn halves(first: u64, r: f64, at: f64) -> Vec<SketchCurve> {
    vec![
        arc(first, [0.0, 0.0], on(r, at), on(r, at + 180.0)),
        arc(first + 1, [0.0, 0.0], on(r, at + 180.0), on(r, at + 360.0)),
    ]
}

fn ring(
    a: &[SketchCurve],
    b: &[SketchCurve],
    up: Option<Vec3>,
    k: f64,
    r1: f64,
    r2: f64,
) -> Result<Swept, KernelError> {
    let (centre, minor) = (0.5 * (r1 + r2), 0.5 * (r2 - r1));
    sweep_two_rails_up(
        &profile(&[circle(1, [centre, 0.0], minor)], k),
        &front(),
        a,
        &Frame::WORLD,
        b,
        &Frame::WORLD,
        Scaling::Uniform,
        up,
        &tol(k),
    )
}

/// Every point within `allowed` of the torus about z with radii `centre`
/// and `minor`.
fn on_torus(points: &[Vec3], centre: f64, minor: f64) -> f64 {
    points
        .iter()
        .map(|p| ((p.x.phypot(p.y) - centre).phypot(p.z) - minor).abs())
        .fold(0.0, f64::max)
}

#[test]
fn a_circle_round_two_concentric_circles_is_a_torus_by_pappus() {
    for k in SCALES {
        let (r1, r2) = (8.0 * k, 12.0 * k);
        let (centre, minor) = (0.5 * (r1 + r2), 0.5 * (r2 - r1));
        let torus = ring(
            &[circle(10, [0.0, 0.0], r1)],
            &[circle(20, [0.0, 0.0], r2)],
            None,
            k,
            r1,
            r2,
        )
        .unwrap();
        assert!(torus.closed);
        let t = tol(k).linear;
        assert!(torus.deviation <= t / 2.0, "{}", torus.deviation);
        // No caps: two half turns of each circle, four quarters of the
        // section each.
        assert!(torus.faces.iter().all(|o| o.role == Role::Side));
        assert_eq!(torus.faces.len(), 8);
        assert_eq!(torus.body.shells.len(), 1);
        let (volume, area, points) = sound(&torus, k);
        within(volume, 2.0 * PI * centre * PI * minor * minor, area * t);
        within(area, 4.0 * PI * PI * centre * minor, area * 1e-6);
        let worst = on_torus(&points, centre, minor);
        assert!(
            worst <= torus.deviation * 1.001 + 1e-12 * k,
            "{worst} > {}",
            torus.deviation
        );
    }
}

#[test]
fn rings_cut_at_other_places_give_the_same_torus_and_start_at_a_joint() {
    for k in SCALES {
        let (r1, r2) = (8.0 * k, 12.0 * k);
        let (centre, minor) = (0.5 * (r1 + r2), 0.5 * (r2 - r1));
        // The section is at 0 degrees, inside one arc of each ring; the
        // rings' joints are at 60 and 240 degrees on one and 100 and 280
        // on the other. The rails are travelled the way the section's
        // normal (−y) points, clockwise seen from above, so the first
        // joint after the section is at 280 degrees, on the second ring:
        // the body starts there, and the first ring is cut in two.
        let torus = ring(
            &halves(10, r1, 60.0),
            &halves(20, r2, 100.0),
            None,
            k,
            r1,
            r2,
        )
        .unwrap();
        let t = tol(k).linear;
        assert!(torus.closed);
        assert!(torus.deviation <= t / 2.0);
        // Legs: 280–240, 240–100, 100–60, 60–(−80) degrees.
        assert_eq!(torus.faces.len(), 16);
        let (volume, area, points) = sound(&torus, k);
        within(volume, 2.0 * PI * centre * PI * minor * minor, area * t);
        within(area, 4.0 * PI * PI * centre * minor, area * 1e-6);
        let worst = on_torus(&points, centre, minor);
        assert!(worst <= torus.deviation * 1.001 + 1e-12 * k, "{worst}");
        // The first ring's cut arc runs both first and last, beside
        // different arcs of the second ring.
        let legs: Vec<(u64, u64)> = torus
            .faces
            .iter()
            .map(|o| (o.sources[1], o.sources[2]))
            .collect();
        assert_eq!(legs.first(), Some(&(11, 20)));
        assert_eq!(legs.last(), Some(&(11, 21)));
    }
}

#[test]
fn a_ring_with_the_up_direction_leaning_has_the_elliptic_integral_volume() {
    for k in SCALES {
        let (r1, r2) = (8.0 * k, 12.0 * k);
        let (centre, minor) = (0.5 * (r1 + r2), 0.5 * (r2 - r1));
        let alpha: f64 = 0.5;
        let (s, c) = alpha.psin_cos();
        let leaning = ring(
            &[circle(10, [0.0, 0.0], r1)],
            &[circle(20, [0.0, 0.0], r2)],
            Some(Vec3::new(s, 0.0, c) * 3.0),
            k,
            r1,
            r2,
        )
        .unwrap();
        let t = tol(k).linear;
        assert!(leaning.closed);
        assert!(leaning.deviation <= t / 2.0);
        let (volume, area, _) = sound(&leaning, k);
        // K(sin α) = π / (2 AGM(1, cos α)).
        let (mut a, mut g) = (1.0_f64, c);
        for _ in 0..40 {
            (a, g) = (0.5 * (a + g), (a * g).sqrt());
        }
        let k_of = PI / (2.0 * a);
        let exact = PI * minor * minor * centre * c * 4.0 * k_of;
        within(volume, exact, area * t);
        // Not the torus: the lean takes volume away.
        assert!(exact < 2.0 * PI * centre * PI * minor * minor * 0.99);
    }
}

#[test]
fn parallel_rails_with_the_up_direction_leaning_make_an_oblique_cylinder() {
    for k in SCALES {
        let (r, l) = (2.0 * k, 30.0 * k);
        let alpha: f64 = 0.4;
        let (s, c) = alpha.psin_cos();
        let up = Vec3::new(0.0, s, c);
        // The section is drawn in the plane of the chord (x) and up.
        let plane = Frame::new(Vec3::ZERO, Vec3::X.cross(up), Vec3::X).unwrap();
        let section = profile(&[circle(1, [r, 0.0], r)], k);
        let rails = (
            [line(10, [0.0, 0.0], [0.0, l])],
            [line(20, [2.0 * r, 0.0], [2.0 * r, l])],
        );
        let pipe = sweep_two_rails_up(
            &section,
            &plane,
            &rails.0,
            &Frame::WORLD,
            &rails.1,
            &Frame::WORLD,
            Scaling::Uniform,
            Some(up),
            &tol(k),
        )
        .unwrap();
        let t = tol(k).linear;
        assert!(!pipe.closed);
        assert!(pipe.deviation <= t / 2.0);
        let (volume, area, _) = sound(&pipe, k);
        within(volume, PI * r * r * l * c, area * t);
        // Without the up direction the section is not in the plane the
        // rails' sketch normal asks for.
        let why = refusal(sweep_two_rails(
            &section,
            &plane,
            &rails.0,
            &Frame::WORLD,
            &rails.1,
            &Frame::WORLD,
            Scaling::Uniform,
            &tol(k),
        ));
        assert!(why.contains("normal of the first rail's sketch"), "{why}");
    }
}

#[test]
fn an_up_direction_along_the_rails_or_the_chord_or_of_no_length_is_refused() {
    let r = 2.0;
    let rails = (
        [line(10, [0.0, 0.0], [0.0, 30.0])],
        [line(20, [2.0 * r, 0.0], [2.0 * r, 30.0])],
    );
    let attempt = |plane: Frame, up: Vec3| {
        sweep_two_rails_up(
            &profile(&[circle(1, [r, 0.0], r)], 1.0),
            &plane,
            &rails.0,
            &Frame::WORLD,
            &rails.1,
            &Frame::WORLD,
            Scaling::Uniform,
            Some(up),
            &tol(1.0),
        )
    };
    // Along the rails: the section lies in their course.
    let flat = Frame::new(Vec3::ZERO, Vec3::Z, Vec3::X).unwrap();
    let why = refusal(attempt(flat, Vec3::Y));
    assert!(why.contains("fold"), "{why}");
    assert!(why.contains("up direction"), "{why}");
    // Along the chord.
    let why = refusal(attempt(front(), Vec3::X));
    assert!(why.contains("up direction"), "{why}");
    // No direction.
    let why = refusal(attempt(front(), Vec3::ZERO));
    assert!(why.contains("no direction"), "{why}");
    let why = refusal(attempt(front(), Vec3::new(f64::NAN, 0.0, 1.0)));
    assert!(why.contains("no direction"), "{why}");
}

#[test]
fn a_ring_and_an_open_rail_or_a_ring_off_the_section_are_refused() {
    let (r1, r2) = (8.0, 12.0);
    let why = refusal(ring(
        &[circle(10, [0.0, 0.0], r1)],
        &[arc(20, [0.0, 0.0], [r2, 0.0], [0.0, r2])],
        None,
        1.0,
        r1,
        r2,
    ));
    assert!(why.contains("both close"), "{why}");
    // A ring that misses the section's outline.
    let why = refusal(ring(
        &[circle(10, [0.0, 0.0], r1)],
        &[circle(20, [0.0, 0.0], r2 + 1.0)],
        None,
        1.0,
        r1,
        r2,
    ));
    assert!(why.contains("outline"), "{why}");
}

#[test]
fn a_ring_is_the_same_body_to_the_bit_every_time() {
    let make = || {
        ring(
            &halves(10, 8.0, 60.0),
            &halves(20, 12.0, 100.0),
            None,
            1.0,
            8.0,
            12.0,
        )
        .unwrap()
    };
    let (one, two) = (make(), make());
    assert_eq!(format!("{:?}", one.body), format!("{:?}", two.body));
    assert_eq!(one.deviation.to_bits(), two.deviation.to_bits());
}
