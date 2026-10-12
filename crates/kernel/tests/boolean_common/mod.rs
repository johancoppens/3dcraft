//! Shared by the boolean tests: operands at a scale, and the checks every
//! result gets.

// Test support: a failure here is a failed test.
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::{BooleanOp, BooleanResult, boolean};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, MassProperties, check, mass_properties};

/// Model sizes the tests run at; the tolerance scales along.
pub const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

pub fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

/// The box from `lo` to `hi`, times `k`.
pub fn boxed(k: f64, lo: [f64; 3], hi: [f64; 3]) -> Body {
    let frame = Frame::WORLD
        .at(Vec3::new(lo[0] * k, lo[1] * k, lo[2] * k))
        .unwrap();
    primitives::block(
        &frame,
        (hi[0] - lo[0]) * k,
        (hi[1] - lo[1]) * k,
        (hi[2] - lo[2]) * k,
    )
    .unwrap()
}

/// An upright cylinder standing on `base`, times `k`.
pub fn rod(k: f64, base: [f64; 3], radius: f64, height: f64) -> Body {
    let frame = Frame::WORLD
        .at(Vec3::new(base[0] * k, base[1] * k, base[2] * k))
        .unwrap();
    primitives::cylinder(&frame, radius * k, height * k).unwrap()
}

pub fn mass(body: &Body) -> MassProperties {
    mass_properties(body).unwrap()
}

/// `(vertices, edges, faces, shells)`.
pub fn counts(result: &BooleanResult) -> (usize, usize, usize, usize) {
    let b = &result.body;
    (
        b.vertices.len(),
        b.edges.len(),
        b.faces.len(),
        b.shells.len(),
    )
}

/// The boolean, which must succeed, checked again here.
pub fn solve(a: &Body, b: &Body, op: BooleanOp, k: f64) -> BooleanResult {
    let tol = tolerance(k);
    let result = match boolean(a, b, op, &tol) {
        Ok(result) => result,
        Err(e) => panic!("{op:?} at scale {k}: {e}"),
    };
    assert_eq!(check(&result.body, &tol), [], "{op:?} at scale {k}");
    assert_eq!(result.faces.len(), result.body.faces.len());
    assert_eq!(result.edges.len(), result.body.edges.len());
    assert_eq!(result.vertices.len(), result.body.vertices.len());
    result
}

pub fn assert_empty(a: &Body, b: &Body, op: BooleanOp, k: f64) {
    match boolean(a, b, op, &tolerance(k)) {
        Err(KernelError::EmptyResult { .. }) => {}
        other => panic!("{op:?} at scale {k}: expected an empty result, got {other:?}"),
    }
}

#[track_caller]
pub fn assert_close(actual: f64, expected: f64, what: &str) {
    let slack = 1e-9 * expected.abs().max(actual.abs());
    assert!(
        (actual - expected).abs() <= slack,
        "{what}: {actual} is not {expected}"
    );
}

/// Volume and area, given for scale 1.
#[track_caller]
pub fn assert_mass(result: &BooleanResult, volume: f64, area: f64, k: f64) {
    let m = mass(&result.body);
    assert_close(m.volume, volume * k * k * k, "volume");
    assert_close(m.area, area * k * k, "area");
}

/// SplitMix64: small, seeded, the same on every platform.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n`.
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }

    /// Uniform in `[0, 1)`.
    pub fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform in `[lo, hi)`.
    pub fn between(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }
}
