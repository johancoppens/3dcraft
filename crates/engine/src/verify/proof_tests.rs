//! The proven distance below the commands (docs/design/verification.md ›
//! *The proven distance*): what the mesh proves of a pair of faces all
//! of whose triangles were left out, proofs cut short by the caps on the
//! work (still no higher than the closed form, and the work held to the
//! caps), and a rational spline face (the exact rational sphere, which has
//! no normal cone) proven to its closed-form distance at three scales.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::{Part, distance, mesh_proves};
use crate::verify_bound::{CAPS, SPENT};
use craft3d_geom::support::{SurfaceNets, surface_piece};
use craft3d_geom::{NurbsSurface, Surface};
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::primitives::{block, cylinder, sphere};
use craft3d_math::Portable;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::Body;
use std::f64::consts::{FRAC_PI_2, FRAC_PI_6};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn tol(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

fn at(origin: Vec3, z: Vec3) -> Frame {
    let x = if z.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    let x = (x - z * x.dot(z)).normalized().unwrap();
    Frame::new(origin, z, x).unwrap()
}

/// (found, proven) of the least distance between two bodies.
fn measure(a: &Body, b: &Body, k: f64) -> (f64, f64) {
    let (pa, pb) = (
        Part::new(a, None, &tol(k)).unwrap(),
        Part::new(b, None, &tol(k)).unwrap(),
    );
    let near = distance(&[pa], &[pb]).unwrap();
    (near.distance, near.lower)
}

/// The per-pair formula of what the mesh proves: a pair of faces all of
/// whose pairs of triangles were left out (further than `best + slack`)
/// proves `best + slack` less what each face can be off by, and a pair
/// compared is held to the same.
#[test]
fn a_pair_of_faces_whose_triangles_were_all_left_out_still_counts() {
    let (best, slack, err_a, err_b) = (1.0, 0.25, 0.125, 0.0625);
    let want = best + slack - err_a - err_b;
    assert_eq!(mesh_proves(None, best, slack, err_a, err_b), want);
    // Its own least further than that: held to it.
    assert_eq!(mesh_proves(Some(5.0), best, slack, err_a, err_b), want);
    // Nearer: its own least.
    assert_eq!(
        mesh_proves(Some(0.5), best, slack, err_a, err_b),
        0.5 - err_a - err_b
    );
    assert!(mesh_proves(None, best, slack, err_a, err_b).is_finite());
}

/// Two discs of radius 100, 1 apart (flat faces bounded by circles, which
/// the mesh alone proves only to its chord), a ball over a block and the
/// rational band (whose proof takes the most work): with the work cut to
/// a few pairs of pieces, to half of what the whole proof took, or to no
/// pairs of strata, the proof is short but never above the closed form,
/// and the work stays within the caps.
#[test]
fn a_proof_cut_short_by_its_caps_is_still_no_higher_than_the_closed_form() {
    let mut cut = 0;
    for k in SCALES {
        let y = Vec3::Y;
        let disc_a = cylinder(&at(Vec3::ZERO, y), 100.0 * k, 6.0 * k).unwrap();
        let disc_b = cylinder(&at(Vec3::new(0.0, -7.0 * k, 0.0), y), 100.0 * k, 6.0 * k).unwrap();
        let base = block(&Frame::WORLD, 20.0 * k, 20.0 * k, 10.0 * k).unwrap();
        let ball = sphere(
            &at(Vec3::new(10.0 * k, 10.0 * k, 18.0 * k), Vec3::Z),
            5.0 * k,
        )
        .unwrap();
        let (band, near_band, _) = rational_band(k);
        for (what, a, b, want) in [
            ("discs", &disc_a, &disc_b, k),
            ("ball over a block", &ball, &base, 3.0 * k),
            ("rational band", &band, &near_band, k),
        ] {
            CAPS.with(|caps| caps.set(None));
            let (found, full) = measure(a, b, k);
            let (full_work, _) = SPENT.with(std::cell::Cell::get);
            eprintln!("{what} at {k}: proven {full} of {want} in {full_work} pairs of pieces");
            assert!(
                (found - want).abs() <= 1e-9 * want,
                "{what} at {k}: {found}"
            );
            assert!(full <= want * (1.0 + 1e-9), "{what} at {k}: {full}");
            assert!(want - full <= 1e-6 * want, "{what} at {k}: only {full}");
            for (nodes, pairs) in [
                (0, usize::MAX),
                (1, usize::MAX),
                (40, usize::MAX),
                (full_work / 2, usize::MAX),
                (usize::MAX, 0),
                (usize::MAX, 1),
            ] {
                CAPS.with(|caps| caps.set(Some((nodes, pairs))));
                let (found, proven) = measure(a, b, k);
                let (work, made) = SPENT.with(std::cell::Cell::get);
                CAPS.with(|caps| caps.set(None));
                let label = format!("{what} at {k}, caps ({nodes}, {pairs})");
                eprintln!(
                    "{label}: proven {proven} of {want}, {work} pairs of pieces, {made} of strata"
                );
                if nodes < full_work {
                    cut += 1;
                }
                assert!(
                    (found - want).abs() <= 1e-9 * want,
                    "{label}: found {found}"
                );
                assert!(proven.is_finite() && proven >= 0.0, "{label}: {proven}");
                assert!(
                    proven <= want * (1.0 + 1e-9),
                    "{label}: proven {proven} above {want}"
                );
                // Each pair of pieces looked at past the cap is one of the
                // halves of the last one cut.
                assert!(
                    work <= nodes.saturating_add(4),
                    "{label}: {work} pairs of pieces"
                );
                assert!(made <= pairs, "{label}: {made} pairs of strata");
            }
        }
    }
    // The caps did cut proofs short (else this tested nothing).
    assert!(cut >= 3, "only {cut} proofs were cut short");
}

/// The arc of the circle of `radius` from the angle `from` to `to` (less
/// than half a turn) as the rational quadratic it is exactly (*The
/// NURBS Book* §7.3): its ends with weight 1, the meeting point of their
/// tangents with weight `cos(d / 2)`. (reach, height) points and weights.
fn arc(from: f64, to: f64, radius: f64) -> (Vec<(f64, f64)>, Vec<f64>) {
    let middle = 0.5 * (from + to);
    let w = (0.5 * (to - from)).pcos();
    let on = |a: f64, scale: f64| (radius * a.pcos() * scale, radius * a.psin() * scale);
    (
        vec![on(from, 1.0), on(middle, 1.0 / w), on(to, 1.0)],
        vec![1.0, w, 1.0],
    )
}

/// The piece of the sphere of `radius` about `frame`'s origin between the
/// longitudes `lon` and the latitudes `lat` (each less than half a turn
/// wide), as the rational biquadratic patch it is exactly: the product of
/// two such arcs (§8.5).
fn rational_sphere(frame: &Frame, radius: f64, lon: (f64, f64), lat: (f64, f64)) -> NurbsSurface {
    let (around, wu) = arc(lon.0, lon.1, 1.0);
    let (profile, wv) = arc(lat.0, lat.1, radius);
    let mut ctrl = Vec::new();
    let mut weights = Vec::new();
    for (&(x, y), &a) in around.iter().zip(&wu) {
        for (&(reach, height), &b) in profile.iter().zip(&wv) {
            ctrl.push(frame.point(reach * x, reach * y, height));
            weights.push(a * b);
        }
    }
    let knots = vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
    NurbsSurface::new(2, 2, knots.clone(), knots, 3, 3, ctrl, weights).unwrap()
}

/// A band of a sphere of radius `10 k` (a quarter turn around, 30° either
/// side of its equator: a sphere cut by a block) with its round face put on
/// the exact rational patch that is that piece of the sphere, a little
/// wider than the face, and a ball of radius `2 k` out along the diagonal
/// of the quarter, `k` from the band: (band, ball, the band's round face).
fn rational_band(k: f64) -> (Body, Body, usize) {
    let r = 10.0 * k;
    let ball = sphere(&Frame::WORLD, r).unwrap();
    let cutter = block(
        &Frame::new(Vec3::new(0.0, 0.0, -0.5 * r), Vec3::Z, Vec3::X).unwrap(),
        2.0 * r,
        2.0 * r,
        r,
    )
    .unwrap();
    let mut band = boolean(&ball, &cutter, BooleanOp::Intersect, &tol(k))
        .unwrap()
        .body;
    let index = band
        .faces
        .iter()
        .position(|f| matches!(f.surface, Surface::Sphere(_)))
        .unwrap();
    let Surface::Sphere(round) = band.faces[index].surface.clone() else {
        panic!("no round face")
    };
    let pad = 0.05;
    let patch = Surface::Nurbs(rational_sphere(
        round.frame(),
        round.radius(),
        (-pad, FRAC_PI_2 + pad),
        (-FRAC_PI_6 - pad, FRAC_PI_6 + pad),
    ));
    // The patch is that piece of the sphere, and the face keeps its
    // outside (the patch's normal may run the other way).
    let ((u0, u1), (v0, v1)) = patch.domain();
    let (u, v) = (0.5 * (u0 + u1), 0.5 * (v0 + v1));
    let middle = patch.eval(u, v);
    assert!((middle.length() - r).abs() <= 1e-12 * r, "{middle:?}");
    let (su, sv) = band.faces[index].surface.project(middle).unwrap();
    let same = patch
        .normal(u, v)
        .unwrap()
        .dot(band.faces[index].surface.normal(su, sv).unwrap())
        > 0.0;
    let reversed = band.faces[index].reversed;
    band.faces[index].surface = patch;
    band.faces[index].reversed = reversed == same;
    let (gap, rho) = (k, 2.0 * k);
    let d = Vec3::new(1.0, 1.0, 0.0).normalized().unwrap();
    let other = sphere(&at(d * (r + gap + rho), Vec3::Z), rho).unwrap();
    (band, other, index)
}

/// The rational band: its round face has no normal cone (an analytic
/// sphere's piece has one), and the distance is still found and proven
/// to `k`, no higher than it, at three scales.
#[test]
fn a_rational_sphere_face_is_proven_to_its_distance_without_a_normal_cone() {
    for k in SCALES {
        let (band, other, index) = rational_band(k);
        let patch = &band.faces[index].surface;
        let Surface::Nurbs(nurbs) = patch else {
            panic!("not rational")
        };
        assert!(nurbs.weights().iter().any(|w| (w - 1.0).abs() > 0.1));
        let nets = SurfaceNets::of(patch).unwrap();
        let piece = surface_piece(patch, &nets, (0.4, 0.6), (0.4, 0.6)).unwrap();
        assert!(piece.cone().is_none(), "a rational piece with a cone");
        let analytic = Surface::Sphere(craft3d_geom::Sphere::new(Frame::WORLD, 10.0 * k).unwrap());
        let piece = surface_piece(
            &analytic,
            &SurfaceNets::of(&analytic).unwrap(),
            (0.7, 0.8),
            (-0.05, 0.05),
        )
        .unwrap();
        assert!(piece.cone().is_some(), "an analytic piece without a cone");
        let gap = k;
        let (found, proven) = measure(&band, &other, k);
        eprintln!("rational band at {k}: found {found}, proven {proven} of {gap}");
        assert!((found - gap).abs() <= 1e-9 * gap, "at {k}: found {found}");
        assert!(
            proven <= gap * (1.0 + 1e-9),
            "at {k}: proven {proven} above {gap}"
        );
        assert!(gap - proven <= 1e-6 * gap, "at {k}: proven only {proven}");
    }
}
