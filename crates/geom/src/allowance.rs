//! A bound on the work of one operation, counted, not clocked, so that an
//! operation that would run for minutes (a search that tries again and
//! again, each try dear) ends in a refusal instead, at the same point on
//! every machine and every run.
//!
//! **What is counted.** Work units, charged on the thread that does the
//! work: one per evaluation of a freeform surface or curve (a point, or a
//! point and its derivatives: half a microsecond or so), one per split of
//! a Bézier patch, and for arithmetic on control nets and polynomials (the
//! normal net of a patch, the forms of the offset's regularity proof) as
//! many as the multiplications it does over [`ARITHMETIC_PER_UNIT`].
//! Evaluations are what almost all freeform work is made of, so the count
//! follows the time within a small factor whatever the operation (one to
//! three million units a second in release, measured on the fuzz findings
//! and the dearest tests, 2026-10-17); a charge is a thread-local add, a
//! nanosecond. Analytic surfaces are not counted: what is dear about them
//! is bounded by their own caps.
//!
//! **What a bound's end means.** A loop that stops returns an error; it
//! never says "nothing found". A check written to let a body through when
//! it cannot tell (a point it cannot classify) may say nothing when its
//! work was cut short, so an operation that holds such checks to a bound
//! does not believe a try during which the bound was reached, whatever it
//! gave (`craft3d_kernel::offset_face`).
//!
//! **What is bounded.** Nothing, until an operation asks for a bound with
//! [`within`]: for as long as the closure runs, the work this thread does
//! may grow by at most that many units (a bound inside another ends at the
//! sooner of the two). The work does not stop by itself: the loops of the
//! operation that can run long ask [`exhausted`] at each pass, and, when
//! it is, stop and refuse, saying so. A loop that asks charges a unit for
//! the pass ([`pass`]), so even a loop whose passes evaluate nothing ends.
//!
//! **Determinism.** The count is the work of this thread. Work handed to
//! other threads is not counted while it runs there and nothing there asks
//! for the bound; a pool that runs work on several threads adds what each
//! did to the thread that waits for it when they are joined ([`charge`]
//! with the [`spent`] of each), so the total after the pool is the same
//! whatever the number of threads and however the work was shared out,
//! and an operation's outcome under a bound does not depend on them.

use std::cell::Cell;

/// Multiplications of arithmetic on nets and polynomials charged as one
/// unit (about the time of one freeform evaluation).
pub const ARITHMETIC_PER_UNIT: u64 = 256;

thread_local! {
    /// Units this thread has done since it started.
    static SPENT: Cell<u64> = const { Cell::new(0) };
    /// The count at which the innermost bound on this thread ends.
    static LIMIT: Cell<u64> = const { Cell::new(u64::MAX) };
}

/// Adds `units` to this thread's work.
#[inline]
pub fn charge(units: u64) {
    let _ = SPENT.try_with(|spent| spent.set(spent.get().saturating_add(units)));
}

/// The units this thread has done so far (only ever grows).
pub fn spent() -> u64 {
    SPENT.try_with(Cell::get).unwrap_or(0)
}

/// True when this thread's innermost bound has been reached.
pub fn exhausted() -> bool {
    let limit = LIMIT.try_with(Cell::get).unwrap_or(u64::MAX);
    spent() >= limit
}

/// One pass of a loop that asks for the bound: charges a unit for it and
/// says whether the bound has been reached.
pub fn pass() -> bool {
    charge(1);
    exhausted()
}

/// Runs `work` with this thread's work bounded to `units` more (or the
/// bound already in force, when that ends sooner), and puts back the bound
/// there was before, also when `work` panics.
pub fn within<R>(units: u64, work: impl FnOnce() -> R) -> R {
    struct Restore(u64);
    impl Drop for Restore {
        fn drop(&mut self) {
            let _ = LIMIT.try_with(|limit| limit.set(self.0));
        }
    }
    let earlier = LIMIT.try_with(Cell::get).unwrap_or(u64::MAX);
    let ends = spent().saturating_add(units).min(earlier);
    let _ = LIMIT.try_with(|limit| limit.set(ends));
    let _restore = Restore(earlier);
    work()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bound_ends_after_its_units_and_the_outer_one_comes_back() {
        assert!(!exhausted());
        within(10, || {
            charge(9);
            assert!(!exhausted());
            within(100, || {
                // The outer bound ends sooner.
                assert!(pass());
            });
            assert!(exhausted());
        });
        assert!(!exhausted());
    }

    #[test]
    fn work_on_another_thread_is_not_counted_until_it_is_added() {
        let before = spent();
        let theirs = std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    let at = spent();
                    charge(5);
                    spent() - at
                })
                .join()
                .unwrap_or(0)
        });
        assert_eq!(spent(), before);
        charge(theirs);
        assert_eq!(spent(), before + 5);
    }
}
