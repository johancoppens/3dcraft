//! Helices, and profiles swept along them.
//!
//! The oracles are closed forms. A helix of radius r and pitch p is
//! `turns · √((2πr)² + p²)` long, every point of it is r from the axis,
//! and one turn takes it one pitch along the axis. A profile carried by
//! the screw motion of a helix sweeps, per radian, the volume of its
//! area times the speed of the motion across its plane; that speed is
//! affine in the point, so its mean over the profile is its value at the
//! centre of area `c`: `V = θ · A · n·v(c)`, with `v(c) = ±a × (c − o) +
//! (p / 2π) a`. For a profile in a plane through the axis this is
//! Pappus, `A · 2π x̄ · turns`; for a profile square to the helix with
//! its centre on it, `A` times the length of the helix, exactly and not
//! only for a thin wire (a wire is not a circle carried by an arbitrary
//! frame, but by this motion the formula holds whatever its size, as
//! long as the sweep does not pass through itself).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{circle, rectangle, scale_curves};
use craft3d_geom::Surface;
use craft3d_kernel::KernelError;
use craft3d_kernel::extrude::Role;
use craft3d_kernel::helix::{Helix, RunOut, circle_error, sweep_helix};
use craft3d_kernel::profile::{Profile, SketchCurve, detect_profiles};
use craft3d_kernel::sweep::Swept;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_tess::{SelfCheckParams, TessParams, body_self_intersections, tessellate_body};
use craft3d_topo::{Body, check, mass_properties};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

/// The document's tolerance at scale `k`: a millionth of a unit, and a
/// millionth of the model at large scales.
fn tol(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k.max(1.0), 1e-9).unwrap()
}

fn profile(curves: &[SketchCurve], k: f64) -> Profile {
    let profiles = detect_profiles(curves, &tol(k)).unwrap();
    assert_eq!(profiles.len(), 1);
    profiles[0].clone()
}

/// The plane through the z axis facing −y: the sketch's x is the world's
/// x, its y the world's z.
fn axial() -> Frame {
    Frame::new(Vec3::ZERO, -Vec3::Y, Vec3::X).unwrap()
}

fn helix(radius: f64, pitch: f64, turns: f64, left: bool) -> Helix {
    Helix::new(Vec3::ZERO, Vec3::Z, Vec3::X, radius, pitch, turns, left).unwrap()
}

#[track_caller]
fn close(found: f64, expected: f64, relative: f64) {
    assert!(
        (found - expected).abs() <= relative * expected.abs(),
        "found {found}, expected {expected} (off by {:e})",
        (found - expected).abs() / expected.abs()
    );
}

fn mesh_volume(body: &Body, chord: f64) -> f64 {
    let mesh = tessellate_body(
        body,
        &TessParams {
            chordal_tolerance: chord,
            ..TessParams::default()
        },
    )
    .unwrap();
    let welded = mesh.weld().unwrap();
    assert!(welded.is_closed(), "the mesh is not watertight at {chord}");
    welded.volume()
}

/// Valid, outside out, one name per face, not passing through itself,
/// watertight as a mesh whose volume tends to the measured one. Returns
/// the volume.
fn sound(swept: &Swept, size: f64, tolerance: &Tolerance) -> f64 {
    let body = &swept.body;
    assert_eq!(check(body, tolerance), vec![]);
    assert_eq!(body.faces.len(), swept.faces.len());
    let mut names: Vec<_> = swept.faces.iter().collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), swept.faces.len(), "two faces with one origin");
    let mass = mass_properties(body).unwrap();
    assert!(mass.volume > 0.0, "inside out: {}", mass.volume);
    let found = body_self_intersections(body, &SelfCheckParams::new(tolerance.linear)).unwrap();
    assert!(found.is_clean(), "{found}");
    for fine in [1e-1, 1e-2] {
        let now = (mesh_volume(body, fine * size) - mass.volume).abs() / mass.volume;
        assert!(now < 4.0 * fine, "mesh volume off by {now} at {fine}");
    }
    assert!(swept.deviation <= tolerance.linear / 2.0, "{}", swept.deviation);
    mass.volume
}

fn refusal(result: Result<Swept, KernelError>) -> String {
    match result {
        Ok(_) => panic!("should have been refused"),
        Err(error) => error.to_string(),
    }
}

/// The length of a curve by Gauss–Legendre quadrature, five points per
/// one of `pieces` pieces of its domain.
fn curve_length(curve: &craft3d_geom::NurbsCurve, pieces: usize) -> f64 {
    const NODES: [(f64, f64); 5] = [
        (0.0, 0.568_888_888_888_888_9),
        (-0.538_469_310_105_683, 0.478_628_670_499_366_5),
        (0.538_469_310_105_683, 0.478_628_670_499_366_5),
        (-0.906_179_845_938_664, 0.236_926_885_056_189_1),
        (0.906_179_845_938_664, 0.236_926_885_056_189_1),
    ];
    let (t0, t1) = curve.domain();
    let step = (t1 - t0) / pieces as f64;
    let mut total = 0.0;
    for i in 0..pieces {
        let middle = t0 + step * (i as f64 + 0.5);
        for (x, w) in NODES {
            total += w * curve.derivatives(middle + x * step / 2.0).1.length() * step / 2.0;
        }
    }
    total
}

#[test]
fn a_helix_is_as_long_as_the_closed_form_and_keeps_its_radius_and_pitch() {
    for k in SCALES {
        for left in [false, true] {
            let (r, p, turns) = (5.0 * k, 1.5 * k, 3.25);
            let h = helix(r, p, turns, left);
            close(h.length(), turns * (TAU * r).hypot(p), 1e-15);
            close(h.height(), p * turns, 1e-15);
            // A turn takes it one pitch along the axis, back over its
            // start; a quarter turn to +y for a right-handed one.
            assert!(h.point(TAU).distance(h.point(0.0) + Vec3::Z * p) < 1e-14 * r.max(p));
            let quarter = h.point(FRAC_PI_2);
            let side = if left { -1.0 } else { 1.0 };
            assert!(quarter.distance(Vec3::new(0.0, side * r, p / 4.0)) < 1e-14 * r);
            // The curve: within a quarter of the tolerance, ending on the
            // helix to the bit, every point r from the axis.
            let tolerance = tol(k).linear;
            let made = h.curve(tolerance).unwrap();
            assert!(made.deviation <= tolerance / 4.0, "{}", made.deviation);
            let curve = &made.curve;
            let (t0, t1) = curve.domain();
            assert_eq!(curve.eval(t0), h.point(0.0));
            assert_eq!(curve.eval(t1), h.point(h.angle()));
            for i in 0..=1000 {
                let at = curve.eval(t0 + (t1 - t0) * f64::from(i) / 1000.0);
                let rho = at.x.hypot(at.y);
                assert!((rho - r).abs() <= tolerance / 4.0, "{rho} at {i}");
            }
            close(curve_length(curve, 400), h.length(), 1e-6);
        }
    }
}

#[test]
fn the_plans_follow_the_unit_circle_as_closely_as_their_degree_allows() {
    // The error of an interpolating polynomial of degree n on an arc of
    // angle s is about 2 (s/4)^(n+1) / (n+1)!: each plan is finer than
    // the one before.
    let errors: Vec<f64> = [(4, 5), (4, 7), (4, 9), (8, 9), (16, 9)]
        .iter()
        .map(|&(per_turn, degree)| circle_error(per_turn, degree).unwrap())
        .collect();
    assert!(errors.windows(2).all(|w| w[1] < w[0]), "{errors:?}");
    assert!(errors[0] < 1e-4 && errors[0] > 1e-7, "{errors:?}");
    assert!(errors[4] < 1e-14, "{errors:?}");
}

#[test]
fn bad_helices_are_refused() {
    let at = |radius: f64, pitch: f64, turns: f64, axis: Vec3| {
        Helix::new(Vec3::ZERO, axis, Vec3::X, radius, pitch, turns, false)
    };
    assert!(at(0.0, 1.0, 1.0, Vec3::Z).is_err());
    assert!(at(-1.0, 1.0, 1.0, Vec3::Z).is_err());
    assert!(at(1.0, 0.0, 1.0, Vec3::Z).is_err());
    assert!(at(1.0, f64::NAN, 1.0, Vec3::Z).is_err());
    assert!(at(1.0, 1.0, 0.0, Vec3::Z).is_err());
    assert!(at(1.0, 1.0, 1e9, Vec3::Z).is_err());
    assert!(at(1.0, 1.0, 1.0, Vec3::ZERO).is_err());
    // The start direction along the axis has nothing square to it left.
    assert!(at(1.0, 1.0, 1.0, Vec3::X).is_err());
    assert!(helix(1.0, 1.0, 1.0, false).curve(0.0).is_err());
    assert!(helix(1.0, 1.0, 1.0, false).curve(f64::NAN).is_err());
}

/// A coil spring: a wire of radius `a` square to the helix at its start,
/// centred on it.
fn spring(k: f64, left: bool) -> (Swept, Helix, f64) {
    let (r, p, turns, a) = (10.0 * k, 4.0 * k, 2.5, 1.0 * k);
    let h = helix(r, p, turns, left);
    let start = h.point(0.0);
    let along = h.velocity(start).normalized().unwrap();
    let plane = Frame::new(start, along, Vec3::X).unwrap();
    let wire = profile(&[circle(1, [0.0, 0.0], a)], k);
    let swept = sweep_helix(&wire, &plane, &h, None, &tol(k)).unwrap();
    (swept, h, a)
}

#[test]
fn a_coil_spring_is_the_area_of_its_wire_times_the_length_of_its_helix() {
    for k in SCALES {
        for left in [false, true] {
            let (swept, h, a) = spring(k, left);
            close(swept.length, h.length(), 1e-15);
            let volume = sound(&swept, a, &tol(k));
            close(volume, PI * a * a * h.length(), 2e-6);
            // Four quarters of the wire per half turn, five half turns,
            // and two ends.
            assert_eq!(swept.body.faces.len(), 4 * 5 + 2);
            let pieces: Vec<u32> = swept
                .faces
                .iter()
                .filter(|o| o.role == Role::Side)
                .map(|o| o.piece)
                .collect();
            assert_eq!(pieces, (0..20).collect::<Vec<u32>>());
        }
    }
}

/// A square groove: the rectangle from `inner` to `outer` from the axis,
/// `height` high from z = 0, in the plane through the axis.
fn groove(k: f64, inner: f64, outer: f64, height: f64) -> Profile {
    profile(
        &scale_curves(&rectangle(1, 1, (inner, outer), (0.0, height)), k),
        k,
    )
}

#[test]
fn a_profile_through_the_axis_sweeps_by_pappus_and_its_lines_along_the_axis_are_cylinders() {
    for k in SCALES {
        let (inner, outer, height, pitch, turns) = (4.0, 5.0, 1.0, 1.5, 3.0);
        let h = helix(inner * k, pitch * k, turns, false);
        let swept = sweep_helix(
            &groove(k, inner, outer, height),
            &axial(),
            &h,
            None,
            &tol(k),
        )
        .unwrap();
        let volume = sound(&swept, height * k, &tol(k));
        let area = (outer - inner) * height * k * k;
        let mean = 0.5 * (inner + outer) * k;
        close(volume, area * TAU * mean * turns, 2e-6);
        // Per half turn the two lines along the axis are cylinders, the
        // two square to it helicoids.
        let kinds: Vec<&str> = swept
            .body
            .faces
            .iter()
            .zip(&swept.faces)
            .filter(|(_, o)| o.role == Role::Side)
            .map(|(f, _)| f.surface.kind())
            .collect();
        assert_eq!(kinds.len(), 4 * 6);
        assert_eq!(kinds.iter().filter(|s| **s == "cylinder").count(), 12);
        for face in &swept.body.faces {
            if let Surface::Cylinder(c) = &face.surface {
                let radius = c.radius() / k;
                assert!(
                    (radius - inner).abs() < 1e-12 || (radius - outer).abs() < 1e-12,
                    "{radius}"
                );
            }
        }
    }
}

#[test]
fn a_profile_square_to_the_axis_sweeps_its_area_times_the_advance() {
    // A flat plate square to the axis rises one pitch per turn: what it
    // sweeps per radian is its area times the advance, p / 2π. (A steep
    // helix: with a shallow one the plate's lines run almost the way the
    // motion goes, and sweep faces thinner than a mesh at a hundredth of
    // the plate can follow.)
    let (inner, outer, width, pitch, turns) = (4.0, 6.0, 1.0, 20.0, 1.5);
    let h = helix(inner, pitch, turns, false);
    let flat = Frame::new(Vec3::ZERO, Vec3::Z, Vec3::X).unwrap();
    let plate = profile(
        &rectangle(1, 1, (inner, outer), (-width / 2.0, width / 2.0)),
        1.0,
    );
    let swept = sweep_helix(&plate, &flat, &h, None, &tol(1.0)).unwrap();
    let volume = sound(&swept, 1.0, &tol(1.0));
    close(
        volume,
        h.angle() * (outer - inner) * width * pitch / TAU,
        2e-6,
    );
}

#[test]
fn a_left_handed_sweep_is_the_mirror_image_of_a_right_handed_one() {
    let right = spring(1.0, false).0;
    let left = spring(1.0, true).0;
    let (a, b) = (
        mass_properties(&right.body).unwrap(),
        mass_properties(&left.body).unwrap(),
    );
    close(a.volume, b.volume, 1e-9);
    let (ca, cb) = (a.centroid.unwrap(), b.centroid.unwrap());
    assert!((ca.x - cb.x).abs() < 1e-6 && (ca.y + cb.y).abs() < 1e-6 && (ca.z - cb.z).abs() < 1e-6);
}

#[test]
fn a_run_out_lifts_the_groove_out_and_adds_what_it_sweeps() {
    let k = 1.0;
    let (inner, outer, height, pitch, turns) = (4.0, 5.0, 1.0, 1.5, 2.0);
    let h = helix(inner, pitch, turns, false);
    let plain = sweep_helix(
        &groove(k, inner, outer, height),
        &axial(),
        &h,
        None,
        &tol(k),
    )
    .unwrap();
    let run = RunOut {
        angle: FRAC_PI_2,
        rise: 1.0,
    };
    let lifted = sweep_helix(
        &groove(k, inner, outer, height),
        &axial(),
        &h,
        Some(run),
        &tol(k),
    )
    .unwrap();
    let (v0, v1) = (
        sound(&plain, height, &tol(k)),
        sound(&lifted, height, &tol(k)),
    );
    // The run-out sweeps at least what a quarter turn at the start's
    // radius would, and at most what one at the radius it rises to.
    let area = (outer - inner) * height;
    let mean = 0.5 * (inner + outer);
    assert!(v1 - v0 > area * FRAC_PI_2 * mean, "{v0} {v1}");
    assert!(v1 - v0 < area * FRAC_PI_2 * (mean + run.rise), "{v0} {v1}");
    // Its end is a quarter turn on: the plane through the axis that
    // started facing −y faces ∓x.
    let end = lifted
        .body
        .faces
        .iter()
        .zip(&lifted.faces)
        .find(|(_, o)| o.role == Role::EndCap)
        .map(|(f, _)| f)
        .unwrap();
    let Surface::Plane(plane) = &end.surface else {
        panic!("the end is flat")
    };
    let n = plane.frame().z();
    assert!(n.dot(Vec3::X).abs() > 1.0 - 1e-12, "{n:?}");
    assert!(lifted.length > h.length());
}

#[test]
fn profiles_that_cannot_be_carried_along_a_helix_are_refused_with_the_reason() {
    let k = 1.0;
    let h = helix(4.0, 1.5, 2.0, false);
    // Across the axis.
    let across = refusal(sweep_helix(
        &groove(k, -1.0, 5.0, 1.0),
        &axial(),
        &helix(5.0, 1.5, 2.0, false),
        None,
        &tol(k),
    ));
    assert!(across.contains("axis"), "{across}");
    // On the other side of the axis from the start.
    let behind = refusal(sweep_helix(
        &groove(k, -6.0, -4.0, 1.0),
        &axial(),
        &h,
        None,
        &tol(k),
    ));
    assert!(behind.contains("round the axis"), "{behind}");
    // Longer along the axis than the pitch.
    let long = refusal(sweep_helix(
        &groove(k, 4.0, 5.0, 1.6),
        &axial(),
        &h,
        None,
        &tol(k),
    ));
    assert!(long.contains("pitch"), "{long}");
    // Not where the helix starts.
    let elsewhere = Frame::new(Vec3::new(0.0, 0.5, 0.0), -Vec3::Y, Vec3::X).unwrap();
    let away = refusal(sweep_helix(
        &groove(k, 4.0, 5.0, 1.0),
        &elsewhere,
        &h,
        None,
        &tol(k),
    ));
    assert!(away.contains("starts"), "{away}");
    // In a plane the motion runs along: tangent to the cylinder the
    // helix lies on, the profile straddling the start.
    let tangent = Frame::new(Vec3::new(4.0, 0.0, 0.0), Vec3::X, Vec3::Y).unwrap();
    let fold = refusal(sweep_helix(
        &profile(&rectangle(1, 1, (-1.0, 1.0), (0.0, 1.0)), k),
        &tangent,
        &h,
        None,
        &tol(k),
    ));
    assert!(fold.contains("fold"), "{fold}");
    // A run-out that is no angle.
    assert!(
        sweep_helix(
            &groove(k, 4.0, 5.0, 1.0),
            &axial(),
            &h,
            Some(RunOut {
                angle: -1.0,
                rise: 1.0
            }),
            &tol(k),
        )
        .is_err()
    );
}

/// The groove of an ISO metric thread of pitch `p` on a shank of radius
/// `major`, in the plane through the axis, centred on z = 0: the basic
/// profile's space between two teeth (flanks at 30° to the radial
/// direction, the root flat P/4 wide at the minor radius), carried on
/// past the major radius by H/16 so that it cuts through the shank's
/// surface. Ids 1 to 4: root, lower flank, outside, upper flank.
fn iso_groove(major: f64, p: f64) -> Vec<SketchCurve> {
    let h = 3.0_f64.sqrt() / 2.0 * p;
    let root = major - 5.0 * h / 8.0;
    let out = major + h / 16.0;
    let half = 7.0 * p / 16.0 + (h / 16.0) * (PI / 6.0).tan();
    common::polygon(
        1,
        1,
        &[
            [root, p / 8.0],
            [root, -p / 8.0],
            [out, -half],
            [out, half],
        ],
    )
}

/// A shank of radius 5 from z = 0 to 20 with a 1 × 1 chamfer at z = 0,
/// revolved about the z axis.
fn shank() -> Body {
    let outline = profile(
        &common::polygon(
            11,
            1,
            &[[0.0, 0.0], [4.0, 0.0], [5.0, 1.0], [5.0, 20.0], [0.0, 20.0]],
        ),
        1.0,
    );
    craft3d_kernel::revolve::revolve(
        &[outline],
        &axial(),
        [0.0, 0.0],
        [0.0, 1.0],
        0.0,
        TAU,
        &tol(1.0),
    )
    .unwrap()
    .body
}

#[test]
fn an_iso_groove_subtracted_from_a_chamfered_shank_cuts_a_thread() {
    let (major, p, length) = (5.0, 1.5, 12.0);
    let h_basic = 3.0_f64.sqrt() / 2.0 * p;
    let below = Frame::new(Vec3::new(0.0, 0.0, -p), -Vec3::Y, Vec3::X).unwrap();
    let minor = major - 5.0 * h_basic / 8.0;
    let h = Helix::new(
        Vec3::new(0.0, 0.0, -p),
        Vec3::Z,
        Vec3::X,
        minor,
        p,
        (length + p) / p,
        false,
    )
    .unwrap();
    let started = std::time::Instant::now();
    let groove = sweep_helix(
        &profile(&iso_groove(major, p), 1.0),
        &below,
        &h,
        Some(RunOut {
            angle: FRAC_PI_2,
            rise: 5.0 * h_basic / 8.0 + h_basic / 8.0,
        }),
        &tol(1.0),
    )
    .unwrap();
    eprintln!(
        "groove: {} faces in {:?}, deviation {}",
        groove.body.faces.len(),
        started.elapsed(),
        groove.deviation
    );
    let started = std::time::Instant::now();
    let cut = craft3d_kernel::boolean::boolean(
        &shank(),
        &groove.body,
        craft3d_kernel::boolean::BooleanOp::Subtract,
        &tol(1.0),
    )
    .unwrap();
    eprintln!(
        "thread: {} faces in {:?}",
        cut.body.faces.len(),
        started.elapsed()
    );
    assert_eq!(check(&cut.body, &tol(1.0)), vec![]);
    let before = mass_properties(&shank()).unwrap().volume;
    let after = mass_properties(&cut.body).unwrap().volume;
    eprintln!("removed {}", before - after);
    assert!(after < before && after > 0.8 * before, "{before} {after}");
}
