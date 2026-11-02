//! One limit, for the whole process, on the extra threads started for
//! heavy kernel work: the engine's pairs of an overlap or an interference
//! (`interference_cmds::in_order`, which also builds an assembly's
//! configurations and bakes a motion), its assertions measured side by
//! side (`assert_check`), and the regeneration's unions of a pattern
//! (`craft3d_regen` › `derive::in_order`), and the threads of a motion's
//! background builder (`craft3d_engine` › `motion_build.rs`), each of
//! which works only while it holds one ([`lease_for_worker`]). Kept here, at the bottom of the
//! layers, so the threads that may each run a freeform boolean come from
//! the same pool (the tessellator's and the mass integrator's lighter
//! threads keep bounds of their own) (an independent review of 2026-11-01 found the pattern's
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
pub struct Lease {
    threads: usize,
    /// Taken by a thread doing background work ([`background`]).
    background: bool,
}

impl Lease {
    /// How many extra threads the holder may start.
    pub fn threads(&self) -> usize {
        self.threads
    }

    /// Called by each thread started with the lease (and harmless on the
    /// holder's own): a helper of background work is background work too
    /// ([`mark_background`]).
    pub fn helper_starts(&self) {
        if self.background {
            mark_background();
        }
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        FREE.fetch_add(self.threads, Ordering::AcqRel);
    }
}

std::thread_local! {
    /// This thread takes no extra threads ([`work_alone`]).
    static ALONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// From now on this thread takes no threads from the pool: background
/// work (a motion's builder, `motion_build.rs`) leaves them to the work
/// someone waits for, which would otherwise find none free and run on
/// one thread.
pub fn work_alone() {
    ALONE.with(|alone| alone.set(true));
}

std::thread_local! {
    /// This thread does background work ([`mark_background`]).
    static BACKGROUND: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// From now on this thread does work no one is waiting on at this moment
/// (the assertions measured behind a command): it may still take threads
/// from the pool, but the kernel lets it hold less of the process's
/// freeform trees (`craft3d_kernel` › `freeform::HELD_TOGETHER`), so the
/// work someone waits for is not held up by it. A thread that works
/// alone ([`work_alone`]) is background work too.
pub fn mark_background() {
    BACKGROUND.with(|b| b.set(true));
}

/// True on a thread that called [`mark_background`] or [`work_alone`].
pub fn background() -> bool {
    BACKGROUND.with(std::cell::Cell::get) || ALONE.with(std::cell::Cell::get)
}

/// One thread of the pool for a background worker itself (a motion's
/// builder, `craft3d_engine` › `motion_build.rs`), when one is free: the
/// worker holds it while it works, so the threads of the work someone
/// waits for and the background's stay within [`SHARED_THREADS`] together.
/// Unlike [`lease`] it is taken also on a thread that works alone.
pub fn lease_for_worker() -> Lease {
    take(1)
}

/// Up to `wanted` extra threads, as many as are free now (none is fine).
pub fn lease(wanted: usize) -> Lease {
    if ALONE.with(std::cell::Cell::get) {
        return Lease {
            threads: 0,
            background: background(),
        };
    }
    take(wanted)
}

fn take(wanted: usize) -> Lease {
    let mut free = FREE.load(Ordering::Acquire);
    loop {
        let take = wanted.min(free);
        if take == 0 {
            return Lease {
                threads: 0,
                background: background(),
            };
        }
        match FREE.compare_exchange_weak(free, free - take, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => {
                return Lease {
                    threads: take,
                    background: background(),
                };
            }
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
        // Everything came back: nothing else in this crate leases.
        let all = lease(SHARED_THREADS);
        assert_eq!(all.threads(), SHARED_THREADS);
        assert_eq!(lease(1).threads(), 0, "none left while all are out");
        drop(all);
        assert_eq!(lease(SHARED_THREADS).threads(), SHARED_THREADS);
    }
}
