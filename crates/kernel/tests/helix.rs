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
use craft3d_kernel::thread::{IsoThread, ThreadPlace};
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
    assert!(
        swept.deviation <= tolerance.linear / 2.0,
        "{}",
        swept.deviation
    );
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
    eprintln!("circle errors (4,5) (4,7) (4,9) (8,9) (16,9): {errors:?}");
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
            eprintln!(
                "spring at {k}: deviation {}, stations {}, volume off by {:e}",
                swept.deviation,
                swept.stations,
                volume / (PI * a * a * h.length()) - 1.0
            );
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

/// A plain shank of radius `r` from z = 0 to `top`, revolved about z;
/// with a `chamfer` × `chamfer` lead at z = 0 when it is above 0.
fn shank(r: f64, top: f64, chamfer: f64) -> Body {
    let mut corners = vec![[0.0, 0.0]];
    if chamfer > 0.0 {
        corners.extend([[r - chamfer, 0.0], [r, chamfer]]);
    } else {
        corners.push([r, 0.0]);
    }
    corners.extend([[r, top], [0.0, top]]);
    let outline = profile(&common::polygon(11, 1, &corners), 1.0);
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

/// The groove of `thread` cut into a shank of its major diameter from
/// z = 0, `length` long at full depth.
fn threaded(thread: &IsoThread, top: f64, length: f64, chamfer: f64) -> (Body, Body, Swept) {
    let blank = shank(thread.major / 2.0, top, chamfer);
    let place = ThreadPlace {
        start: Vec3::ZERO,
        along: Vec3::Z,
        across: Vec3::X,
        length,
        left_handed: false,
    };
    let (groove, _) = craft3d_kernel::thread::groove(thread, &place, &tol(1.0)).unwrap();
    let cut = craft3d_kernel::boolean::boolean(
        &blank,
        &groove.body,
        craft3d_kernel::boolean::BooleanOp::Subtract,
        &tol(1.0),
    )
    .unwrap();
    (blank, cut.body, groove)
}

/// What the groove removes from a shank of the major radius `big`: by
/// the flux of the screw motion through the groove's plane, `∫ dθ ∫∫ r
/// dA` over the part of the groove inside the shank. Along the thread
/// at full depth that is `A r̄ 2π L / P` (the part cut off by the end
/// of the shank is made up by the part beyond `L`, the groove being
/// symmetric about its centre); along the run-out, where the groove has
/// moved out by `ρ(φ)`, the integral is over the part of the trapezoid
/// still inside, worked out here by Simpson's rule over φ.
fn removed(thread: &IsoThread, length: f64) -> f64 {
    let (p, big) = (thread.pitch, thread.major / 2.0);
    let main = thread.groove_area() * thread.groove_centre() * TAU * length / p;
    let root = thread.minor() / 2.0;
    let slope = 2.0 / 3.0_f64.sqrt();
    // ∫ (r + ρ) w(r) dr over r from the root to `to`, w(r) = P/4 +
    // slope (r − root).
    let moment = |rho: f64, to: f64| -> f64 {
        if to <= root {
            return 0.0;
        }
        let f = |r: f64| {
            let s = r - root;
            // (root + s + ρ)(P/4 + slope s), integrated in s.
            let a = root + rho;
            a * p / 4.0 * s + (a * slope + p / 4.0) * s * s / 2.0 + slope * s * s * s / 3.0
        };
        f(to) - f(root)
    };
    let rise = thread.depth() + thread.triangle() / 8.0;
    let n = 2000;
    let h = FRAC_PI_2 / n as f64;
    let mut run = 0.0;
    for i in 0..=n {
        let phi = h * i as f64;
        let rho = rise * (phi / FRAC_PI_2).powi(2);
        let w = if i == 0 || i == n {
            1.0
        } else if i % 2 == 1 {
            4.0
        } else {
            2.0
        };
        run += w * moment(rho, big - rho);
    }
    main + run * h / 3.0
}

#[test]
fn an_iso_thread_removes_its_groove_and_leaves_the_minor_diameter() {
    // M10 × 1.5 and M10 × 1.25 (fine) on a shank 20 long, 12 of thread.
    for pitch in [1.5, 1.25] {
        let thread = IsoThread::new(10.0, pitch).unwrap();
        close(thread.minor(), 10.0 - 1.082_531_754_730_548 * pitch, 1e-12);
        let (blank, cut, groove) = threaded(&thread, 20.0, 12.0, 0.0);
        assert_eq!(check(&cut, &tol(1.0)), vec![]);
        assert!(groove.deviation <= 0.5e-6);
        let before = mass_properties(&blank).unwrap().volume;
        let after = mass_properties(&cut).unwrap().volume;
        // The groove's faces lie within half the tolerance of the true
        // ones over an area of about 10 per turn.
        let slack = 0.5e-6 * 10.0 * (12.0 / pitch + 2.0);
        let expected = removed(&thread, 12.0);
        eprintln!(
            "M10 x {pitch}: removed {} against {expected}; deviation {}",
            before - after,
            groove.deviation
        );
        assert!(
            ((before - after) - expected).abs() < slack,
            "removed {} against {expected}",
            before - after
        );
        let found = body_self_intersections(&cut, &SelfCheckParams::new(1e-6)).unwrap();
        assert!(found.is_clean(), "{found}");
        // The minor diameter, by a section through the axis of a mesh at
        // a chordal tolerance of 1e-4: the least distance from the axis
        // of the points where the mesh crosses the plane y = 0 on +x,
        // along the full thread.
        let mesh = tessellate_body(
            &cut,
            &TessParams {
                chordal_tolerance: 1e-4,
                ..TessParams::default()
            },
        )
        .unwrap();
        let mut least = f64::INFINITY;
        for t in 0..mesh.triangles.len() {
            let corners = mesh.corners(t).unwrap();
            for (a, b) in [(0, 1), (1, 2), (2, 0)] {
                let (a, b) = (corners[a], corners[b]);
                if (a.y > 0.0) == (b.y > 0.0) {
                    continue;
                }
                let at = a + (b - a) * (a.y / (a.y - b.y));
                if at.x > 0.0 && at.z > 2.0 && at.z < 10.0 {
                    least = least.min(at.x);
                }
            }
        }
        eprintln!(
            "minor radius by section {least} against {}",
            thread.minor() / 2.0
        );
        assert!(
            (least - thread.minor() / 2.0).abs() < 2e-4,
            "{least} against {}",
            thread.minor() / 2.0
        );
    }
}

#[test]
fn a_thread_runs_through_a_lead_chamfer() {
    let thread = IsoThread::new(10.0, 1.5).unwrap();
    let (blank, cut, _) = threaded(&thread, 20.0, 12.0, 1.0);
    assert_eq!(check(&cut, &tol(1.0)), vec![]);
    let (before, after) = (
        mass_properties(&blank).unwrap().volume,
        mass_properties(&cut).unwrap().volume,
    );
    // Less is removed where the chamfer took material already.
    let removed = before - after;
    let full = super_removed(&thread);
    assert!(removed > 0.8 * full && removed < full, "{removed} {full}");
    let found = body_self_intersections(&cut, &SelfCheckParams::new(1e-6)).unwrap();
    assert!(found.is_clean(), "{found}");
}

fn super_removed(thread: &IsoThread) -> f64 {
    removed(thread, 12.0)
}

#[test]
fn iso_sizes_and_pitches_are_read_from_their_designation() {
    use craft3d_kernel::thread::{coarse_pitch, nominal_diameter};
    assert_eq!(nominal_diameter("M10"), Some(10.0));
    assert_eq!(nominal_diameter(" m2.5 "), Some(2.5));
    for bad in ["", "M", "10", "M-3", "Mx", "M1e9", "M10x1.5", "M0"] {
        assert_eq!(nominal_diameter(bad), None, "{bad}");
    }
    assert_eq!(coarse_pitch(10.0), Some(1.5));
    assert_eq!(coarse_pitch(3.0), Some(0.5));
    assert_eq!(coarse_pitch(24.0), Some(3.0));
    assert_eq!(coarse_pitch(11.0), None);
    assert!(IsoThread::new(10.0, 0.0).is_err());
    assert!(IsoThread::new(10.0, 12.0).is_err());
    assert!(IsoThread::new(f64::NAN, 1.0).is_err());
}
