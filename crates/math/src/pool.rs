//! One limit, for the whole process, on the extra threads started for
//! heavy kernel work: the engine's pairs of an overlap or an interference
//! (`interference_cmds::in_order`, which also builds an assembly's
//! configurations and bakes a motion), its assertions measured side by
//! side (`assert_check`), and the regeneration's unions of a pattern
//! (`craft3d_regen` › `derive::in_order`). Kept here, at the bottom of the
//! layers, so every crate that starts such threads takes them from the
//! same pool (an independent review of 2026-11-01 found the pattern's
//! unions starting their own threads on top of the engine's). Each such thread may run
//! a freeform boolean with its own budget of trees (`craft3d_kernel` ›
//! `freeform::HELD_BYTES`, 2 GiB), and the work nests (an assertion job
//! measures overlaps, each with its pairs): without one limit the threads
//! multiplied (2 assertions × 8 pairs). A caller always works itself and
//! takes extra threads only as far as the pool has them, never waiting
//! for one, so nothing can deadlock and the answers do not depend on how
//! many it got (each task's answer goes to its own place).

use std::sync::atomic::{AtomicUsize, Ordering};

/// Extra threads for heavy work, at most, in the whole process.
pub const SHARED_THREADS: usize = 7;

static FREE: AtomicUsize = AtomicUsize::new(SHARED_THREADS);

/// Threads taken from the pool, given back when dropped.
pub struct Lease(usize);

impl Lease {
    /// How many extra threads the holder may start.
    pub fn threads(&self) -> usize {
        self.0
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        FREE.fetch_add(self.0, Ordering::AcqRel);
    }
}

/// Up to `wanted` extra threads, as many as are free now (none is fine).
pub fn lease(wanted: usize) -> Lease {
    let mut free = FREE.load(Ordering::Acquire);
    loop {
        let take = wanted.min(free);
        if take == 0 {
            return Lease(0);
        }
        match FREE.compare_exchange_weak(free, free - take, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return Lease(take),
            Err(now) => free = now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Leases never hand out more than the pool holds, nested or not,
    /// and give everything back.
    #[test]
    fn leases_stay_within_the_pool_and_come_back() {
        // Other tests of this crate may hold some: count from what is
        // free now, alone in this thread's view.
        let a = lease(5);
        let b = lease(5);
        assert!(a.threads() + b.threads() <= SHARED_THREADS);
        let c = lease(SHARED_THREADS);
        assert!(a.threads() + b.threads() + c.threads() <= SHARED_THREADS);
        drop((a, b, c));
        assert_eq!(lease(0).threads(), 0);
    }
}
