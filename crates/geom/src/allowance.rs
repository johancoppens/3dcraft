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
//!
//! **Caches.** A cache whose hit skips counted work would make the count
//! depend on what ran before. A cache that speeds up a counted search
//! keeps what the miss cost ([`measured`]) and charges it on a hit (the
//! projections a surface remembers, the meshes of faces kept); one whose
//! making is not counted ([`uncounted`]) is charged by its size once per
//! operation ([`once`]); the memos of a patch tree are kept per operation
//! under a bound ([`scope`]). Held by the tests that count the same
//! operation on fresh bodies cold, warm and after others.
//!
//! **What is bounded.** The face operations' search (`craft3d_kernel::
//! offset_face`) and the self-check (`craft3d_tess`). Not booleans,
//! blends, sweeps, lofts, delete face or pattern joins: they set no bound,
//! and what polls for one does nothing outside it.

use std::cell::Cell;

/// Multiplications of arithmetic on nets and polynomials charged as one
/// unit (about the time of one freeform evaluation).
pub const ARITHMETIC_PER_UNIT: u64 = 256;

thread_local! {
    /// Units this thread has done since it started.
    static SPENT: Cell<u64> = const { Cell::new(0) };
    /// The count at which the innermost bound on this thread ends.
    static LIMIT: Cell<u64> = const { Cell::new(u64::MAX) };
    /// How many [`uncounted`] regions this thread is in: none counts.
    static FROZEN: Cell<u32> = const { Cell::new(0) };
    /// The outermost bound in force on this thread, by a number no other
    /// bound of this thread had ([`scope`]).
    static SCOPE: Cell<Option<u64>> = const { Cell::new(None) };
    /// The number the next outermost bound gets.
    static SCOPES: Cell<u64> = const { Cell::new(0) };
    /// What [`once`] charged under the outermost bound in force: its
    /// number and the things charged.
    // Asked whether it holds a thing, never walked: its order is no part
    // of any result.
    static ONCE: std::cell::RefCell<(u64, std::collections::HashSet<u64>)> =
        std::cell::RefCell::new((0, std::collections::HashSet::new()));
}

/// Charges `units` for the thing numbered `thing` (a tree of a surface, a
/// numeric intersection, a body checked: what a cache keeps) the first
/// time the operation under the outermost bound in force asks for it, and
/// not again within that operation: what making it once takes, whether it
/// was made then or found made. Outside a bound, every time.
pub fn once(thing: u64, units: u64) {
    let Some(scope) = scope() else {
        charge(units);
        return;
    };
    let first = ONCE
        .try_with(|once| {
            let Ok(mut once) = once.try_borrow_mut() else {
                return false;
            };
            if once.0 != scope {
                *once = (scope, std::collections::HashSet::new());
            }
            once.1.insert(thing)
        })
        .unwrap_or(false);
    if first {
        charge(units);
    }
}

/// FNV-1a of `text`: the same number for the same text on every run, for
/// [`once`].
pub fn number_of(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The outermost bound in force on this thread, if any, as a number no
/// other bound on this thread had: what an operation under a bound
/// remembers for itself (answers a cache would share with every operation)
/// is kept under it, so that whether a question is answered from memory
/// depends on what the operation asked before, not on what ran before it.
pub fn scope() -> Option<u64> {
    SCOPE.try_with(Cell::get).ok().flatten()
}

/// Adds `units` to this thread's work.
#[inline]
pub fn charge(units: u64) {
    if FROZEN.try_with(Cell::get).unwrap_or(0) > 0 {
        return;
    }
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

/// Runs `work` without counting what it does on this thread: what a
/// cache makes or finds.
///
/// **Caches are not counted; each question to one is, the same whether it
/// is answered from the cache or not.** Whatever a cache keeps (a tree of
/// a surface, a numeric intersection, an answer it remembers, a check a
/// body passed) is made inside this, and the question charges a nominal
/// amount that is a function of what was asked or of the answer (the
/// nodes of the tree, the points of the intersection) instead. An
/// operation then counts the same whether what it asks for was made
/// before, by it, by an earlier operation, on another thread or for
/// another document, so its count, and what a bound decides about it, is
/// a function of its inputs alone. (Charging what the making cost on every
/// hit would be pure too, but it counts a remembered answer asked for a
/// thousand times a thousand times: a legitimate offset of the rotor
/// counted 2.5 billion units.) Inside, [`exhausted`] sees the count as it
/// was when the region began, so a cut it makes does not depend on whether
/// the cache was hit either.
pub fn uncounted<R>(work: impl FnOnce() -> R) -> R {
    struct Thaw;
    impl Drop for Thaw {
        fn drop(&mut self) {
            let _ = FROZEN.try_with(|frozen| frozen.set(frozen.get().saturating_sub(1)));
        }
    }
    let _ = FROZEN.try_with(|frozen| frozen.set(frozen.get().saturating_add(1)));
    let _thaw = Thaw;
    work()
}

/// Whether this thread counts its work now (not in an [`uncounted`]
/// region). A cache that keeps what a miss cost with what it keeps keeps
/// nothing made while not counting (its cost would read nothing) or while
/// the bound in force was reached (a search it cut short).
pub fn counting() -> bool {
    FROZEN.try_with(Cell::get).unwrap_or(0) == 0 && !exhausted()
}

/// Runs `work` and gives back what it returned with the units it charged
/// on this thread (what it handed to other threads included, once they
/// were joined).
pub fn measured<R>(work: impl FnOnce() -> R) -> (R, u64) {
    let before = spent();
    let made = work();
    (made, spent().saturating_sub(before))
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
    struct Unscope(bool);
    impl Drop for Unscope {
        fn drop(&mut self) {
            if self.0 {
                let _ = SCOPE.try_with(|scope| scope.set(None));
            }
        }
    }
    let earlier = LIMIT.try_with(Cell::get).unwrap_or(u64::MAX);
    let ends = spent().saturating_add(units).min(earlier);
    let _ = LIMIT.try_with(|limit| limit.set(ends));
    let _restore = Restore(earlier);
    let outermost = scope().is_none();
    if outermost {
        let number = SCOPES
            .try_with(|n| {
                let next = n.get().wrapping_add(1);
                n.set(next);
                next
            })
            .unwrap_or(0);
        let _ = SCOPE.try_with(|scope| scope.set(Some(number)));
    }
    let _unscope = Unscope(outermost);
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
