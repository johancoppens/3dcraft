//! Settling the nearest points of two faces that run nearly parallel
//! (docs/design/verification.md › *Settling on the exact faces*):
//! projecting onto each face in turn closes in on them at a ratio near 1
//! per step there, and stopped short after all its steps; with the jumps
//! ahead of [`settle`] the closed form is reached. A plane over a round
//! of radius `R`, `g` above it: from a point a fifth of `R` aside,
//! projecting in turn moves it by `R / (R + g)` per step (0.99 here), so
//! 400 steps leave it 0.018 of the way, `6e-4 g` too far; the closed form
//! is `g`. At three scales.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::{Part, distance, settle};
use craft3d_geom::Surface;
use craft3d_kernel::boolean::probe::Probe;
use craft3d_kernel::primitives::{block, cylinder};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::Body;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn tol(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

/// A round of radius 100 along Y (axis through the origin, 10 long) and
/// a block whose bottom face lies 1 above it, scaled by `k`; and the
/// index of the round's side and of the block's bottom.
fn plane_over_round(k: f64) -> (Body, Body, usize, usize) {
    let axis = Frame::new(Vec3::ZERO, Vec3::Y, Vec3::X).unwrap();
    let round = cylinder(&axis, 100.0 * k, 10.0 * k).unwrap();
    let corner = Frame::new(Vec3::new(-50.0 * k, 0.0, 101.0 * k), Vec3::Z, Vec3::X).unwrap();
    let slab = block(&corner, 100.0 * k, 10.0 * k, 10.0 * k).unwrap();
    let side = round
        .faces
        .iter()
        .position(|f| matches!(f.surface, Surface::Cylinder(_)))
        .unwrap();
    let bottom = slab
        .faces
        .iter()
        .position(|f| match &f.surface {
            Surface::Plane(p) => {
                let o = p.frame.origin();
                p.frame.z().z.abs() > 0.5 && (o.z - 101.0 * k).abs() < 1e-9 * k
            }
            _ => false,
        })
        .unwrap();
    (round, slab, side, bottom)
}

#[test]
fn nearly_parallel_faces_settle_on_the_closed_form() {
    for k in SCALES {
        let (round, slab, side, bottom) = plane_over_round(k);
        let (pa, pb) = (
            Probe::new(&slab, &tol(k)).unwrap(),
            Probe::new(&round, &tol(k)).unwrap(),
        );
        let start = Vec3::new(20.0 * k, 5.0 * k, 101.0 * k);
        let (p, q) = settle(&pa, bottom, &pb, side, start, start).unwrap();
        let d = p.distance(q);
        assert!((d - k).abs() <= 1e-9 * k, "at {k}: {d} for {k}");
        // On the faces: the plane's height, the round's radius.
        assert!((p.z - 101.0 * k).abs() <= 1e-9 * k, "at {k}: {p:?}");
        let radius = (q.x * q.x + q.z * q.z).sqrt();
        assert!((radius - 100.0 * k).abs() <= 1e-9 * k, "at {k}: {q:?}");

        // The whole distance, from the meshes on.
        let (a, b) = (
            Part::new(&slab, None, &tol(k)).unwrap(),
            Part::new(&round, None, &tol(k)).unwrap(),
        );
        let near = distance(&[a], &[b]).unwrap();
        assert!(
            (near.distance - k).abs() <= 1e-9 * k,
            "at {k}: {}",
            near.distance
        );
        assert!(near.lower <= near.distance, "at {k}: {near:?}");
    }
}
