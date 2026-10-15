//! Counts of the work regeneration did, for the performance benchmark of
//! the gate (`crates/engine/tests/benchmark.rs`, docs/testing.md): counts,
//! not a clock, so a slowdown shows the same on a machine under load.
//!
//! Process-wide and only ever growing: a reader takes them before and
//! after the work it measures. Added to once per regeneration and once
//! per sketch solve.

use std::sync::atomic::{AtomicU64, Ordering};

static REGENERATIONS: AtomicU64 = AtomicU64::new(0);
static REBUILT: AtomicU64 = AtomicU64::new(0);
static REUSED: AtomicU64 = AtomicU64::new(0);
static CONTEXT_ENTRIES: AtomicU64 = AtomicU64::new(0);
static SKETCH_SOLVES: AtomicU64 = AtomicU64::new(0);

/// Regeneration's work so far ([`work`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RegenWork {
    /// Regenerations run through the cache ([`crate::RegenCache`]).
    pub regenerations: u64,
    /// Features built afresh.
    pub features_rebuilt: u64,
    /// Features whose result the cache gave back.
    pub features_reused: u64,
    /// Entries of what features are made from put together and compared
    /// ([`crate::RegenCache::last_run_work`], summed).
    pub context_entries: u64,
    /// Sketches solved by the constraint solver (not found in the cache).
    pub sketch_solves: u64,
}

/// What regeneration has done since the process started.
pub fn work() -> RegenWork {
    RegenWork {
        regenerations: REGENERATIONS.load(Ordering::Relaxed),
        features_rebuilt: REBUILT.load(Ordering::Relaxed),
        features_reused: REUSED.load(Ordering::Relaxed),
        context_entries: CONTEXT_ENTRIES.load(Ordering::Relaxed),
        sketch_solves: SKETCH_SOLVES.load(Ordering::Relaxed),
    }
}

fn wide(count: usize) -> u64 {
    u64::try_from(count).unwrap_or(u64::MAX)
}

pub(crate) fn regeneration(rebuilt: usize, reused: usize, entries: usize) {
    REGENERATIONS.fetch_add(1, Ordering::Relaxed);
    REBUILT.fetch_add(wide(rebuilt), Ordering::Relaxed);
    REUSED.fetch_add(wide(reused), Ordering::Relaxed);
    CONTEXT_ENTRIES.fetch_add(wide(entries), Ordering::Relaxed);
}

pub(crate) fn sketch_solve() {
    SKETCH_SOLVES.fetch_add(1, Ordering::Relaxed);
}
