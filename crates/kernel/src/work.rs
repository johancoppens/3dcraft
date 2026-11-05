//! Counts of the work the kernel did, for the performance benchmark of the
//! gate (`crates/engine/tests/benchmark.rs`, docs/testing.md): a count,
//! not a clock, so a slowdown shows the same on a machine under load.
//!
//! The counts are process-wide and only ever grow: a reader takes them
//! before and after the work it measures. They are added to once per
//! operation or per face pair (relaxed atomics, a few nanoseconds each),
//! never in an inner loop, and sum the same whatever the threads, so a
//! deterministic computation gives the same counts on every run.

use std::sync::atomic::{AtomicU64, Ordering};

static BOOLEANS: AtomicU64 = AtomicU64::new(0);
static FACE_PAIRS: AtomicU64 = AtomicU64::new(0);
static FACE_CANDIDATES: AtomicU64 = AtomicU64::new(0);
static EDGE_CANDIDATES: AtomicU64 = AtomicU64::new(0);
static PATCH_TREES: AtomicU64 = AtomicU64::new(0);
static BLEND_SECTIONS: AtomicU64 = AtomicU64::new(0);
static HELPER_PANICS: AtomicU64 = AtomicU64::new(0);

/// Helper threads of the kernel that panicked (a boolean section's
/// helpers, `boolean/section.rs` › `Ahead`): the work goes on without
/// them, so a bug in one would go unseen but for this count, which the
/// fuzzer and the health report can read. Zero in a process without such
/// a bug.
pub fn helper_panics() -> u64 {
    HELPER_PANICS.load(Ordering::Relaxed)
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn helper_panicked() {
    HELPER_PANICS.fetch_add(1, Ordering::Relaxed);
}

/// The kernel's work so far ([`work`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KernelWork {
    /// Calls of [`crate::boolean::boolean_with`].
    pub booleans: u64,
    /// Pairs of faces, one of each operand, whose surfaces a boolean
    /// intersected (their boxes and hulls met).
    pub face_pairs: u64,
    /// Faces a boolean's broad phase (`boolean/facetree.rs`) handed on to
    /// an exact test of their boxes: per face of A the faces of B its box
    /// may meet, per point or ray of the classification the faces it may
    /// reach. Testing every face against every face, the count grows with
    /// the product of the face counts; with the tree, about with their sum.
    pub face_candidates: u64,
    /// Edges of a face a boolean asked where they meet the other face's
    /// surface: those near where the two faces are, of every pair of faces
    /// whose surfaces meet (`boolean/section.rs` › `edge_cuts`).
    pub edge_candidates: u64,
    /// Patch trees of freeform surfaces built (not found on the shelf).
    pub patch_trees: u64,
    /// Cross-sections of rolling-ball and chamfer blends solved.
    pub blend_sections: u64,
}

/// What the kernel has done since the process started.
pub fn work() -> KernelWork {
    KernelWork {
        booleans: BOOLEANS.load(Ordering::Relaxed),
        face_pairs: FACE_PAIRS.load(Ordering::Relaxed),
        face_candidates: FACE_CANDIDATES.load(Ordering::Relaxed),
        edge_candidates: EDGE_CANDIDATES.load(Ordering::Relaxed),
        patch_trees: PATCH_TREES.load(Ordering::Relaxed),
        blend_sections: BLEND_SECTIONS.load(Ordering::Relaxed),
    }
}

pub(crate) fn boolean() {
    BOOLEANS.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn face_pairs(count: u64) {
    FACE_PAIRS.fetch_add(count, Ordering::Relaxed);
}

/// A count of the faces (or edges) a loop of the boolean tested by box,
/// added to its counter once, when the loop is left (by any way out): each
/// pass of the loop counts one, whatever the loop iterates over, so a loop
/// that went back to testing every face would show it.
pub(crate) struct Tally {
    count: u64,
    counter: &'static AtomicU64,
}

impl Tally {
    /// For [`KernelWork::face_candidates`].
    pub(crate) fn faces() -> Tally {
        Tally {
            count: 0,
            counter: &FACE_CANDIDATES,
        }
    }

    /// For [`KernelWork::edge_candidates`].
    pub(crate) fn edges() -> Tally {
        Tally {
            count: 0,
            counter: &EDGE_CANDIDATES,
        }
    }

    pub(crate) fn one(&mut self) {
        self.count = self.count.saturating_add(1);
    }
}

impl Drop for Tally {
    fn drop(&mut self) {
        self.counter.fetch_add(self.count, Ordering::Relaxed);
    }
}

pub(crate) fn patch_tree() {
    PATCH_TREES.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn blend_section() {
    BLEND_SECTIONS.fetch_add(1, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use crate::boolean::{BooleanOp, boolean};
    use crate::primitives;
    use craft3d_math::{Frame, Tolerance, Vec3};

    /// A boolean of two overlapping blocks is counted, with the pairs of
    /// faces it intersected. (Other tests of the crate run at the same
    /// time and add to the counts too: only growth is certain.)
    #[test]
    fn a_boolean_and_its_face_pairs_are_counted() {
        let a = primitives::block(&Frame::WORLD, 2.0, 2.0, 2.0).unwrap();
        let at = Frame::WORLD.at(Vec3::new(1.0, 1.0, 1.0)).unwrap();
        let b = primitives::block(&at, 2.0, 2.0, 2.0).unwrap();
        let before = super::work();
        boolean(&a, &b, BooleanOp::Union, &Tolerance::default()).unwrap();
        let after = super::work();
        assert!(after.booleans > before.booleans);
        assert!(after.face_pairs > before.face_pairs);
    }
}
