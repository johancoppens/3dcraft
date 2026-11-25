//! The review of the near look's round three (2026-11-25; the workshop's
//! `plan/near-look-3/review/`; docs/design/booleans.md › *Near approaches
//! inside an edge or a face* › *Round three*).
//!
//! - The box of a whole torus face (F3) holds the surface for frames made
//!   from a hint that is not square to the axis (whose x and y then carry
//!   rounding along the axis), leaning from none to 0.4 rad off each world
//!   axis, at three scales and far from the origin: every point of the
//!   surface, evaluated, lies in `Body::bbox` and `Body::held_bbox`.
//! - The escape (F2) and the flat ring (F1) refuse no body just outside
//!   the band: a ring slid on a rod off its axis 2.2 tolerances from it at
//!   the nearest, and a ring leaning from none to 8e-10 rad 2.2 tolerances
//!   over a plate, are united as two parts and their interference is
//!   clear, while at 1.5 tolerances they are refused and touch; at 1e-3, 1
//!   and 1e3, turned in space.
//! - The look gives the same answers, to the bit, run on eight threads at
//!   once as run alone.
//!
//! Run with `--nocapture` for the answers.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::Surface;
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::{BooleanOp, BooleanResult, Interference, boolean, interference};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Portable, Tolerance, Vec3};
use craft3d_topo::Body;
use std::f64::consts::PI;

/// What a boolean said, in a word.
fn said(r: &Result<BooleanResult, KernelError>) -> String {
    match r {
        Ok(r) => format!("built ({} shells)", r.body.shells.len()),
        Err(KernelError::ZeroThickness { .. }) => "zero thickness".to_string(),
        Err(KernelError::Unsupported { reason, .. }) => format!("refused: {reason}"),
        Err(e) => format!("error: {e}"),
    }
}

/// A refusal of the band: too near to tell apart, or zero thickness.
fn refused(r: &Result<BooleanResult, KernelError>) -> bool {
    matches!(
        r,
        Err(KernelError::Unsupported { .. } | KernelError::ZeroThickness { .. })
    )
}

/// The interference in a word.
fn contact(found: &Result<Interference, KernelError>) -> &'static str {
    match found {
        Ok(Interference::Clear) => "clear",
        Ok(Interference::Touch { .. }) => "touch",
        Ok(Interference::Overlap(_)) => "overlap",
        Err(_) => "unknown",
    }
}

/// A frame turned in space, at `origin`.
fn turned_at(origin: Vec3) -> Frame {
    Frame::new(origin, Vec3::new(0.3, -0.5, 0.81), Vec3::new(1.0, 0.2, 0.0)).unwrap()
}

/// A whole torus face boxed whole, its frame made from a hint that is not
/// square to its axis: the axis leans `lean` from a world axis towards the
/// next, the hint is that axis and the next mixed with the third, so the
/// frame's x is made by taking most of the hint away (and keeps rounding
/// along the axis that a hint square to it would not leave). The surface
/// evaluated over a grid of 256 by 64 angles, and about each of its
/// furthest points along each world axis (found from the frame, a
/// thousandth of a radian round either way), lies in the body's box and in
/// its held box, with no margin. Leans 0, 1e-15 to 1e-2 and 0.4 rad, at
/// 1e-3, 1 and 1e6, centred at the origin, near it and a hundred thousand
/// sizes away.
#[test]
fn a_torus_on_a_frame_made_from_a_slanted_hint_holds_its_surface_in_its_box() {
    let axes = [Vec3::X, Vec3::Y, Vec3::Z];
    let mut checked = 0usize;
    for k in [1e-3, 1.0, 1e6] {
        let (major, minor) = (30000.0 * k, 10.0 * k);
        for centre in [
            Vec3::ZERO,
            Vec3::new(3.0, -7.0, 11.0) * k,
            Vec3::new(1e5, -2e5, 5e4) * (30000.0 * k),
        ] {
            for lean in [0.0, 1e-15, 1e-12, 1e-10, 8e-9, 1e-8, 1e-6, 1e-4, 1e-2, 0.4] {
                let (s, c) = lean.psin_cos();
                for (i, &along) in axes.iter().enumerate() {
                    let (toward, third) = (axes[(i + 1) % 3], axes[(i + 2) % 3]);
                    let axis = along * c + toward * s;
                    for hint in [
                        along * 0.7 + third,
                        along * 2.0 + toward * 0.5 + third * 0.3,
                    ] {
                        let Some(frame) = Frame::new(centre, axis, hint) else {
                            continue;
                        };
                        let body = primitives::torus(&frame, major, minor).unwrap();
                        let surface = &body.faces[0].surface;
                        let Surface::Torus(torus) = surface else {
                            panic!("a torus");
                        };
                        let mut points: Vec<Vec3> = Vec::new();
                        for a in 0..256 {
                            for b in 0..64 {
                                let u = 2.0 * PI * f64::from(a) / 256.0;
                                let v = 2.0 * PI * f64::from(b) / 64.0;
                                points.push(surface.eval(u, v));
                            }
                        }
                        let f = torus.frame();
                        for e in axes {
                            let (ex, ey, ez) = (e.dot(f.x()), e.dot(f.y()), e.dot(f.z()));
                            let u = ey.patan2(ex);
                            let v = ez.patan2(ex.phypot(ey));
                            for (u, v) in [(u, v), (u + PI, -v)] {
                                for du in [-1e-3, -1e-6, 0.0, 1e-6, 1e-3] {
                                    for dv in [-1e-3, -1e-6, 0.0, 1e-6, 1e-3] {
                                        points.push(surface.eval(u + du, v + dv));
                                    }
                                }
                            }
                        }
                        let what = format!(
                            "a torus at {k}, centre {centre:?}, leaning {lean} from axis {i}, hint {hint:?}"
                        );
                        for (name, bbox) in [
                            ("bbox", body.bbox().unwrap()),
                            ("held_bbox", body.held_bbox().unwrap()),
                        ] {
                            for p in &points {
                                assert!(
                                    bbox.contains(*p, 0.0),
                                    "{what}, {name}: {bbox:?} misses the surface's point {p:?}"
                                );
                            }
                        }
                        checked += 1;
                    }
                }
            }
        }
    }
    println!("look_3_review: {checked} tori boxed whole");
}

/// A rod of radius `k` along the frame's x from −5k to 5k, and a ring
/// about it (axis along the frame's x, major radius 2k) centred one
/// tolerance from the rod's axis towards the frame's +y, its inside `k +
/// (gap + 1) t` from the ring's axis: `gap` tolerances from the rod at
/// −y, `gap + 2` at +y. The rod first (A).
fn ring_on_rod(f: &Frame, k: f64, gap: f64, tol: &Tolerance) -> (Body, Body) {
    let t = tol.linear;
    let rod = primitives::cylinder(
        &Frame::new(f.point(-5.0 * k, 0.0, 0.0), f.x(), f.y()).unwrap(),
        k,
        10.0 * k,
    )
    .unwrap();
    let inside = k + (gap + 1.0) * t;
    let ring = primitives::torus(
        &Frame::new(f.point(0.0, t, 0.0), f.x(), f.z()).unwrap(),
        2.0 * k,
        2.0 * k - inside,
    )
    .unwrap();
    (rod, ring)
}

/// The near-approach tests' turned frame, at the world's origin.
fn turned() -> Frame {
    turned_at(Vec3::ZERO)
}

/// The dimple review's ring of radius 3000 and tube 10 leaning `lean` rad
/// from the turned frame's z towards its −y, over a plate whose top lies
/// `gap` tolerances under the ring's lowest point (see
/// `boolean_near_dimple_review.rs` › `ring_over_plate`). The ring first.
fn ring_over_plate(lean: f64, gap: f64, tol: &Tolerance) -> (Body, Body) {
    let f = turned();
    let (major, minor) = (3000.0, 10.0);
    let (s, c) = lean.psin_cos();
    let ring = primitives::torus(
        &Frame::new(Vec3::ZERO, f.vector(0.0, -s, c), f.x()).unwrap(),
        major,
        minor,
    )
    .unwrap();
    let top = -(minor + major * s) - gap * tol.linear;
    let plate = primitives::block(
        &f.at(f.point(-3500.0, -3200.0, top - 10.0)).unwrap(),
        6400.0,
        6400.0,
        10.0,
    )
    .unwrap();
    (ring, plate)
}

/// Whether a union and an interference answer as two parts apart (built,
/// two shells, clear) or as a touch (refused, a touch or unknown).
fn answers(a: &Body, b: &Body, tol: &Tolerance) -> (bool, bool, String) {
    let union = boolean(a, b, BooleanOp::Union, tol);
    let between = interference(a, b, tol);
    let apart =
        union.as_ref().is_ok_and(|r| r.body.shells.len() == 2) && contact(&between) == "clear";
    let touch = refused(&union) && matches!(contact(&between), "touch" | "unknown");
    let words = format!(
        "the union {}, the interference {}",
        said(&union),
        contact(&between)
    );
    (apart, touch, words)
}

/// A ring slid on a rod one tolerance off its axis (a cylinder and a
/// torus, which the escape is for): 2.2 tolerances from the rod at the
/// nearest, it is united as two parts and clear; 1.5 tolerances, refused
/// and a touch. At 1e-3, 1 and 1e3, in the world's frame and turned.
#[test]
fn a_ring_slid_on_a_rod_off_its_axis_just_outside_the_band_is_still_two_parts() {
    let mut wrong = Vec::new();
    for k in [1e-3, 1.0, 1e3] {
        let tol = Tolerance::new(0.01 * k, 1e-9).unwrap();
        for (name, f) in [
            ("world", Frame::WORLD),
            ("turned", turned_at(Vec3::new(13.0, -7.0, 3.0) * k)),
        ] {
            for gap in [1.5, 2.2] {
                let (rod, ring) = ring_on_rod(&f, k, gap, &tol);
                let (apart, touch, words) = answers(&rod, &ring, &tol);
                println!(
                    "look_3_review: a ring on a rod at {k}, {name}, {gap} tolerances: {words}"
                );
                let right = if gap < 2.0 { touch } else { apart };
                if !right {
                    wrong.push(format!("at {k}, {name}, {gap} tolerances: {words}"));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// The dimple review's ring over a plate, leaning from none to 8e-10 rad,
/// either side of the 1e-12 under which `from_plane` takes it as flat:
/// 2.2 tolerances under its lowest point, united as two parts and clear;
/// 1.5 tolerances, refused and a touch.
#[test]
fn a_ring_leaning_either_side_of_flat_over_a_plate_is_refused_in_the_band_only() {
    let tol = Tolerance::default();
    let mut wrong = Vec::new();
    for lean in [0.0, 5e-13, 2e-12, 1e-10, 8e-10] {
        for gap in [1.5, 2.2] {
            let (ring, plate) = ring_over_plate(lean, gap, &tol);
            let (apart, touch, words) = answers(&ring, &plate, &tol);
            println!(
                "look_3_review: a ring leaning {lean} rad, {gap} tolerances over a plate: {words}"
            );
            let right = if gap < 2.0 { touch } else { apart };
            if !right {
                wrong.push(format!("leaning {lean}, {gap} tolerances: {words}"));
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// The answers of a boolean and an interference, to the bit: the result
/// itself, or the error's words.
fn bits(a: &Body, b: &Body, tol: &Tolerance) -> (Result<BooleanResult, String>, String) {
    let union = boolean(a, b, BooleanOp::Union, tol).map_err(|e| format!("{e:?}"));
    let between = format!("{:?}", interference(a, b, tol));
    (union, between)
}

/// The look and what it reads give the same answers, to the bit, on eight
/// threads at once as alone: the ring on a rod at 1.5 and 2.2 tolerances,
/// the ring leaning 8e-10 rad over a plate at 1.5 and 2.2.
#[test]
fn the_look_answers_the_same_bits_on_eight_threads_at_once() {
    let tol = Tolerance::new(0.01, 1e-9).unwrap();
    let fine = Tolerance::default();
    let cases: Vec<(Body, Body, Tolerance)> = [1.5, 2.2]
        .into_iter()
        .flat_map(|gap| {
            let (rod, ring) = ring_on_rod(&turned(), 1.0, gap, &tol);
            let (ring2, plate) = ring_over_plate(8e-10, gap, &fine);
            [(rod, ring, tol), (ring2, plate, fine)]
        })
        .collect();
    let alone: Vec<_> = cases.iter().map(|(a, b, t)| bits(a, b, t)).collect();
    let together: Vec<Vec<_>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    cases
                        .iter()
                        .map(|(a, b, t)| bits(a, b, t))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    for (n, run) in together.iter().enumerate() {
        for (i, (one, other)) in alone.iter().zip(run).enumerate() {
            assert!(one == other, "case {i} on thread {n}: not the same bits");
        }
    }
}
