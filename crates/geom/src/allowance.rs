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
//! **What is bounded.** On the count: the face operations' search
//! (`craft3d_kernel::offset_face`), the plate's tries, a STEP body read
//! and the self-check (`craft3d_tess`). On the ledger (below): what enters
//! `craft3d_kernel`'s budget of held trees (booleans, interference, the
//! classification of points). Not blends, sweeps, lofts, delete face or
//! pattern joins: they set no bound, and what polls for one does nothing
//! outside it.
//!
//! **The ledger** ([`asked`]). Beside the count, each thread keeps what its
//! operations asked for: every unit charged, and what the count leaves out
//! ([`replay`]): the work a cache answers from memory where finding it is
//! counted only when the cache is missed (the memos of a patch tree,
//! shared by every operation and thread), and arithmetic the count does
//! not price (the normal net of a rational patch). So the ledger of an
//! operation is a function of what it asks alone, the same whatever was
//! kept before, by whom, and on however many threads, also outside a bound
//! (where the count is not: `craft3d_kernel`'s section meets pairs ahead
//! on helper threads that fill those memos). A bound on the ledger
//! ([`asked_within`]) opens no scope: the operation's caches, its helpers
//! and its count stay what they are without it, and only [`exhausted`]
//! says when it is reached. The operations of `craft3d_kernel` that enter
//! through its budget of held trees (the boolean, interference, and the
//! classification of points) are held to it.

use std::cell::Cell;

/// Multiplications of arithmetic on nets and polynomials charged as one
/// unit (about the time of one freeform evaluation).
pub const ARITHMETIC_PER_UNIT: u64 = 256;

thread_local! {
    /// Units this thread has done since it started.
    static SPENT: Cell<u64> = const { Cell::new(0) };
    /// The count at which the innermost bound on this thread ends.
    static LIMIT: Cell<u64> = const { Cell::new(u64::MAX) };
    /// The ledger of this thread since it started ([`asked`]).
    static ASKED: Cell<u64> = const { Cell::new(0) };
    /// The ledger at which the innermost bound on it ends
    /// ([`asked_within`]).
    static ASKED_LIMIT: Cell<u64> = const { Cell::new(u64::MAX) };
    /// How many regions this thread is in where the ledger does not grow
    /// ([`unasked`]).
    static UNASKED: Cell<u32> = const { Cell::new(0) };
    /// What calls off the work this thread does for someone else
    /// ([`stopping_when`]).
    static STOP: std::cell::RefCell<Option<std::sync::Arc<std::sync::atomic::AtomicBool>>> =
        const { std::cell::RefCell::new(None) };
    /// What the operation under the outermost bound on the ledger asked
    /// for already ([`asked_first_time`]); `None` outside one.
    // Asked whether it holds a thing, never walked: its order is no part
    // of any result.
    static ASKED_ONCE: std::cell::RefCell<Option<AskedOnce>> =
        const { std::cell::RefCell::new(None) };
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
    /// What to call when the outermost bound ends ([`at_scope_end`]).
    static SCOPE_ENDS: std::cell::RefCell<Vec<fn()>> = const { std::cell::RefCell::new(Vec::new()) };
    static ONCE: std::cell::RefCell<(u64, std::collections::HashSet<u64>)> =
        std::cell::RefCell::new((0, std::collections::HashSet::new()));
}

/// Charges `units` for the thing numbered `thing` (a tree of a surface, a
/// numeric intersection, a body checked: what a cache keeps) the first
/// time the operation under the outermost bound in force asks for it, and
/// not again within that operation: what making it once takes, whether it
/// was made then or found made. Outside a bound, every time.
pub fn once(thing: u64, units: u64) {
    if first_time(thing) {
        charge(units);
    }
}

/// [`once`], and in the ledger what making the thing asked beyond its
/// count (`asked`, at least `units`: [`costs_of`]) at the operation's first
/// question for it ([`asked_first_time`]).
pub fn once_asked(thing: u64, units: u64, asked: u64) {
    let first = first_time(thing);
    if asked_first_time(thing) {
        if first {
            charge(units);
        } else {
            replay(units);
        }
        replay(asked.saturating_sub(units));
    } else if first {
        unasked(|| charge(units));
    }
}

/// Whether the operation under the outermost bound in force asks for the
/// thing numbered `thing` for the first time, which it is then marked as
/// having done ([`once`]). Outside a bound, always. Inside an uncounted
/// region nothing is charged, so nothing is marked either: the first
/// counted question for it still pays.
pub fn first_time(thing: u64) -> bool {
    if FROZEN.try_with(Cell::get).unwrap_or(0) > 0 {
        return true;
    }
    let Some(scope) = scope() else {
        return true;
    };
    ONCE.try_with(|once| {
        let Ok(mut once) = once.try_borrow_mut() else {
            return false;
        };
        if once.0 != scope {
            *once = (scope, std::collections::HashSet::new());
        }
        once.1.insert(thing)
    })
    .unwrap_or(false)
}

/// Has `end` called when the outermost bound in force on this thread
/// ends (once, however often it is asked for): for what an operation
/// keeps for itself in a thread-local of another crate (the memos of
/// `craft3d_kernel`'s patch trees), let go with it.
pub fn at_scope_end(end: fn()) {
    if scope().is_none() {
        return;
    }
    let _ = SCOPE_ENDS.try_with(|ends| {
        if let Ok(mut ends) = ends.try_borrow_mut()
            && !ends.iter().any(|e| std::ptr::fn_addr_eq(*e, end))
        {
            ends.push(end);
        }
    });
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

/// Adds `units` to this thread's work (and to its ledger, [`asked`]).
#[inline]
pub fn charge(units: u64) {
    if FROZEN.try_with(Cell::get).unwrap_or(0) > 0 {
        return;
    }
    let _ = SPENT.try_with(|spent| spent.set(spent.get().saturating_add(units)));
    if UNASKED.try_with(Cell::get).unwrap_or(0) == 0 {
        let _ = ASKED.try_with(|asked| asked.set(asked.get().saturating_add(units)));
    }
}

/// Adds `units` to this thread's ledger alone ([`asked`]): what a cache
/// answered from memory, or arithmetic the count does not price. Not
/// inside an [`uncounted`] region.
#[inline]
pub fn replay(units: u64) {
    if FROZEN.try_with(Cell::get).unwrap_or(0) > 0 || UNASKED.try_with(Cell::get).unwrap_or(0) > 0 {
        return;
    }
    let _ = ASKED.try_with(|asked| asked.set(asked.get().saturating_add(units)));
}

/// Runs `work` with the ledger left as it is ([`asked`]; the count is
/// charged as ever): for a question the operation under the bound on the
/// ledger asked already and paid for in the ledger then, worked out again
/// because the cache that held its answer let go of it.
pub fn unasked<R>(work: impl FnOnce() -> R) -> R {
    struct Thaw;
    impl Drop for Thaw {
        fn drop(&mut self) {
            let _ = UNASKED.try_with(|u| u.set(u.get().saturating_sub(1)));
        }
    }
    let _ = UNASKED.try_with(|u| u.set(u.get().saturating_add(1)));
    let _thaw = Thaw;
    work()
}

/// What the operation under the outermost bound on the ledger asked for
/// already, by group ([`asked_first_time_in`]).
type AskedOnce = std::collections::HashMap<u64, Things, Mixed>;

/// The things of one group the operation asked for ([`AskedOnce`]).
type Things = std::collections::HashSet<u64, Mixed>;

/// Hashes a number that is a hash already (a thing's or a group's number,
/// FNV-1a of what it stands for) as itself: asked about once per question
/// to a memo, on the hot path of a boolean, where the default hasher's
/// rounds cost more than the question answered from memory.
type Mixed = std::hash::BuildHasherDefault<AsIs>;

/// The hasher of [`Mixed`]: the number itself (a `u64` only).
#[derive(Default)]
struct AsIs(u64);

impl std::hash::Hasher for AsIs {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
    }

    fn write_u64(&mut self, n: u64) {
        self.0 = n;
    }
}

/// Whether the operation under the outermost bound on the ledger
/// ([`asked_within`]) asks for the thing numbered `thing` (a question to a
/// cache, a tree) for the first time, which it is then marked as having
/// done; outside such a bound, always ([`asked_first_time_in`]).
pub fn asked_first_time(thing: u64) -> bool {
    asked_first_time_in(0, thing, usize::MAX)
}

/// [`asked_first_time`] among the things of `group`, of which the
/// operation remembers at most `most`: past that it forgets all of the
/// group's and starts again. What a cache answers from memory is put in
/// the ledger at an operation's first question for it, as a memo of the
/// operation's own of `most` answers would answer it (the cache's own
/// size: a memo of a patch tree forgets all its answers when full), and
/// a question asked again while that memo would still hold it is free,
/// whoever answers it and whatever the shared cache kept. So the ledger
/// is a function of the questions the operation asks, in their order, as
/// the count is under a bound (`scope`); and work done again where a
/// memo of that size would have forgotten is in it. Inside an
/// [`uncounted`] region nothing is marked.
pub fn asked_first_time_in(group: u64, thing: u64, most: usize) -> bool {
    if FROZEN.try_with(Cell::get).unwrap_or(0) > 0 {
        return true;
    }
    ASKED_ONCE
        .try_with(|once| match once.try_borrow_mut() {
            Ok(mut once) => once.as_mut().is_none_or(|groups| {
                let set = groups.entry(group).or_default();
                if set.len() >= most && !set.contains(&thing) {
                    set.clear();
                }
                set.insert(thing)
            }),
            Err(_) => true,
        })
        .unwrap_or(true)
}

/// This thread's ledger so far (only ever grows): every unit charged and
/// every unit replayed ([`replay`]).
pub fn asked() -> u64 {
    ASKED.try_with(Cell::get).unwrap_or(0)
}

/// Whether the innermost bound on this thread's ledger has been reached
/// ([`asked_within`]).
pub fn asked_reached() -> bool {
    asked() >= ASKED_LIMIT.try_with(Cell::get).unwrap_or(u64::MAX)
}

/// Runs `work` with this thread's ledger bounded to `units` more (or the
/// bound already in force on it, when that ends sooner), and puts back the
/// bound there was before, also when `work` panics. It opens no scope
/// ([`scope`]): what the work counts, keeps and hands to other threads is
/// what it would be without it; only [`exhausted`] (and [`pass`]) say when
/// it is reached, as they do for the count's bound.
pub fn asked_within<R>(units: u64, work: impl FnOnce() -> R) -> R {
    struct Restore(u64);
    impl Drop for Restore {
        fn drop(&mut self) {
            let _ = ASKED_LIMIT.try_with(|limit| limit.set(self.0));
        }
    }
    struct Forget(bool);
    impl Drop for Forget {
        fn drop(&mut self) {
            if self.0 {
                let _ = ASKED_ONCE.try_with(|once| {
                    if let Ok(mut once) = once.try_borrow_mut() {
                        *once = None;
                    }
                });
            }
        }
    }
    let earlier = ASKED_LIMIT.try_with(Cell::get).unwrap_or(u64::MAX);
    let ends = asked().saturating_add(units).min(earlier);
    let _ = ASKED_LIMIT.try_with(|limit| limit.set(ends));
    let _restore = Restore(earlier);
    let outermost = ASKED_ONCE
        .try_with(|once| match once.try_borrow_mut() {
            Ok(mut once) if once.is_none() => {
                *once = Some(AskedOnce::default());
                true
            }
            _ => false,
        })
        .unwrap_or(false);
    let _forget = Forget(outermost);
    work()
}

/// The units this thread has done so far (only ever grows).
pub fn spent() -> u64 {
    SPENT.try_with(Cell::get).unwrap_or(0)
}

/// True when this thread's innermost bound has been reached, on the count
/// or on the ledger ([`asked_within`]).
pub fn exhausted() -> bool {
    let limit = LIMIT.try_with(Cell::get).unwrap_or(u64::MAX);
    spent() >= limit || asked_reached() || stopped()
}

/// Whether the work this thread does for someone else was called off
/// ([`stopping_when`]).
fn stopped() -> bool {
    STOP.try_with(|stop| {
        stop.try_borrow()
            .ok()
            .and_then(|s| {
                s.as_ref()
                    .map(|f| f.load(std::sync::atomic::Ordering::Acquire))
            })
            .unwrap_or(false)
    })
    .unwrap_or(false)
}

/// Runs `work` as work this thread does for someone else that `stop`
/// calls off: once it is set, [`exhausted`] says so here, so the loops
/// that ask for a bound stop as they would at one, and the caches keep
/// nothing found after it ([`counting`]). For helper threads whose work
/// only fills caches (`craft3d_kernel`'s section meets pairs ahead on
/// them): what they would have found is found by whoever asks for it
/// next, the same, so calling them off changes no answer.
pub fn stopping_when<R>(
    stop: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    work: impl FnOnce() -> R,
) -> R {
    struct Restore(Option<std::sync::Arc<std::sync::atomic::AtomicBool>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let earlier = self.0.take();
            let _ = STOP.try_with(|stop| {
                if let Ok(mut stop) = stop.try_borrow_mut() {
                    *stop = earlier;
                }
            });
        }
    }
    let earlier = STOP
        .try_with(|s| {
            s.try_borrow_mut()
                .ok()
                .and_then(|mut s| s.replace(std::sync::Arc::clone(stop)))
        })
        .ok()
        .flatten();
    let _restore = Restore(earlier);
    work()
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
/// hit, for a cache whose answer an operation asks for again and again,
/// counts it every time: tried, it made a legitimate offset of the rotor
/// count 2.5 billion units, so that is done only where an answer is asked
/// for about once per operation, or charged once per operation with
/// [`once`].) Inside, [`exhausted`] sees the count as it
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

/// Runs `work` and gives back what it returned with the work it took on
/// this thread, measured also inside an [`uncounted`] region (where it is
/// then not charged): for what a cache makes and keeps with its cost, so
/// that the cost kept does not depend on where it was first made.
pub fn cost_of<R>(work: impl FnOnce() -> R) -> (R, u64) {
    let (made, units, _) = costs_of(work);
    (made, units)
}

/// [`cost_of`], with what it asked of the ledger ([`asked`]) besides: for
/// a cache that keeps both and charges both on a hit ([`once_asked`]).
///
/// What the work takes is measured as if counted and asked, through every
/// region that suspends either: an [`uncounted`] one (neither grows) and
/// an [`unasked`] one (the ledger does not grow). There are no others: the
/// count and the ledger grow only through [`charge`] and [`replay`], which
/// those two regions alone hold back. What the region held back stays
/// held back: the count and the ledger are put back where it would not
/// have let them grow. So what a cache keeps with what it made is the same
/// wherever it was made: a memo's answer searched again inside an
/// [`unasked`] region (an operation's own question asked again) keeps what
/// the search asked, not nothing, and a later operation that asks it first
/// pays that (until 2026-11-16 it kept 0: the ledger of the same question
/// was 238 cold and 0 after such a search, the review's).
///
/// While it measures through such a region the ledger's bound
/// ([`asked_within`]) stands where it stood: reached inside if it was
/// reached on entry, not reached otherwise. The ledger grows there only to
/// be put back, so a loop inside that asks for the bound sees what the
/// region would show it, not a bound crossed by work that is not asked
/// (and a search done again only because a shared cache let go of its
/// answer is not cut short where the cold one was not).
pub fn costs_of<R>(work: impl FnOnce() -> R) -> (R, u64, u64) {
    struct Restore(u32, u32, Option<u64>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let _ = FROZEN.try_with(|f| f.set(self.0));
            let _ = UNASKED.try_with(|u| u.set(self.1));
            if let Some(limit) = self.2 {
                let _ = ASKED_LIMIT.try_with(|l| l.set(limit));
            }
        }
    }
    let frozen = FROZEN.try_with(Cell::get).unwrap_or(0);
    let unasked_here = UNASKED.try_with(Cell::get).unwrap_or(0);
    let (before, asked_before) = (spent(), asked());
    let held = (frozen > 0 || unasked_here > 0).then(|| {
        let limit = ASKED_LIMIT.try_with(Cell::get).unwrap_or(u64::MAX);
        let inside = if asked_before >= limit { 0 } else { u64::MAX };
        let _ = ASKED_LIMIT.try_with(|l| l.set(inside));
        limit
    });
    let _restore = Restore(frozen, unasked_here, held);
    let _ = FROZEN.try_with(|f| f.set(0));
    let _ = UNASKED.try_with(|u| u.set(0));
    let made = work();
    let units = spent().saturating_sub(before);
    let asked_units = asked().saturating_sub(asked_before);
    if frozen > 0 {
        // Not counted where it ran.
        let _ = SPENT.try_with(|spent| spent.set(before));
    }
    if frozen > 0 || unasked_here > 0 {
        // Not asked where it ran.
        let _ = ASKED.try_with(|asked| asked.set(asked_before));
    }
    (made, units, asked_units)
}

/// Whether this thread counts its work now (not in an [`uncounted`]
/// region). A cache that keeps what a miss cost with what it keeps keeps
/// nothing made while not counting (its cost would read nothing) or while
/// the bound in force was reached (a search it cut short).
pub fn counting() -> bool {
    FROZEN.try_with(Cell::get).unwrap_or(0) == 0 && !exhausted()
}

/// Runs `work` and gives back what it returned with what it added to this
/// thread's ledger ([`asked`]).
pub fn measured_asked<R>(work: impl FnOnce() -> R) -> (R, u64) {
    let before = asked();
    let made = work();
    (made, asked().saturating_sub(before))
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
                // What the operation kept for itself goes with it.
                let _ = ONCE.try_with(|once| {
                    if let Ok(mut once) = once.try_borrow_mut() {
                        *once = (0, std::collections::HashSet::new());
                    }
                });
                let ends = SCOPE_ENDS
                    .try_with(|ends| ends.try_borrow_mut().map(|mut e| std::mem::take(&mut *e)))
                    .ok()
                    .and_then(Result::ok)
                    .unwrap_or_default();
                for end in ends {
                    end();
                }
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
    fn the_ledger_holds_what_is_charged_and_replayed_and_its_bound_opens_no_scope() {
        let (spent0, asked0) = (spent(), asked());
        charge(3);
        replay(4);
        assert_eq!((spent() - spent0, asked() - asked0), (3, 7));
        // Not inside an uncounted region, nor where the ledger is left as
        // it is.
        uncounted(|| {
            charge(5);
            replay(5);
        });
        unasked(|| {
            charge(2);
            replay(9);
        });
        assert_eq!((spent() - spent0, asked() - asked0), (5, 7));
        // A bound on the ledger: no scope, reached by the ledger alone.
        asked_within(10, || {
            assert!(scope().is_none());
            assert!(!exhausted());
            replay(9);
            assert!(!exhausted());
            assert!(pass(), "a pass reaches it");
            assert!(asked_reached());
        });
        assert!(!exhausted() && !asked_reached());
        // What it took inside an uncounted region is measured and not kept.
        let (_, units, asked_units) = uncounted(|| {
            costs_of(|| {
                charge(4);
                replay(6);
            })
        });
        assert_eq!((units, asked_units), (4, 10));
        assert_eq!((spent() - spent0, asked() - asked0), (6, 17));
        // Inside a region that leaves the ledger alone: measured the same,
        // the count charged, the ledger not.
        let (_, units, asked_units) = unasked(|| {
            costs_of(|| {
                charge(4);
                replay(6);
            })
        });
        assert_eq!((units, asked_units), (4, 10));
        assert_eq!((spent() - spent0, asked() - asked0), (10, 17));
        // Both, nested either way: the same measure, nothing kept.
        for nested in [0, 1] {
            let measure = || {
                costs_of(|| {
                    charge(4);
                    replay(6);
                })
            };
            let (_, units, asked_units) = if nested == 0 {
                uncounted(|| unasked(measure))
            } else {
                unasked(|| uncounted(measure))
            };
            assert_eq!((units, asked_units), (4, 10));
        }
        assert_eq!((spent() - spent0, asked() - asked0), (10, 17));
        // The regions come back as they were.
        unasked(|| {
            let _ = costs_of(|| ());
            replay(5);
        });
        assert_eq!(asked() - asked0, 17);
        // Measured through a region, the ledger's bound stands where it
        // stood: not crossed by what is put back, reached if it was.
        asked_within(3, || {
            unasked(|| {
                let (crossed, _, asked_units) = costs_of(|| {
                    replay(10);
                    asked_reached()
                });
                assert!(!crossed && asked_units == 10);
            });
            assert!(!asked_reached());
            replay(3);
            assert!(asked_reached());
            let (still, ..) = uncounted(|| costs_of(asked_reached));
            assert!(still && asked_reached());
        });
    }

    #[test]
    fn an_operation_asks_for_a_thing_for_the_first_time_once() {
        assert!(
            asked_first_time(7) && asked_first_time(7),
            "outside a bound"
        );
        asked_within(u64::MAX, || {
            assert!(asked_first_time(7));
            assert!(!asked_first_time(7));
            // Not marked inside an uncounted region.
            assert!(uncounted(|| asked_first_time(8)));
            asked_within(5, || assert!(!asked_first_time(7), "nested: the outermost"));
            assert!(asked_first_time(8));
            // `once_asked`: the ledger at the first question only.
            let before = asked();
            once_asked(9, 2, 5);
            once_asked(9, 2, 5);
            assert_eq!(asked() - before, 5);
        });
        asked_within(u64::MAX, || assert!(asked_first_time(7), "a new operation"));
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

    #[test]
    fn work_called_off_is_exhausted_from_then_on_and_keeps_nothing() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        let stop = Arc::new(AtomicBool::new(false));
        assert!(!exhausted());
        stopping_when(&stop, || {
            assert!(!exhausted() && counting() && !pass());
            // Set from another thread, seen here at the next question.
            std::thread::scope(|s| {
                let _ = s.spawn(|| stop.store(true, Ordering::Release)).join();
            });
            assert!(exhausted() && !counting() && pass());
            // Nested: the inner one's flag, then the outer one's again.
            let other = Arc::new(AtomicBool::new(false));
            stopping_when(&other, || assert!(!exhausted()));
            assert!(exhausted());
        });
        // Not called off outside it, also when its work panicked.
        assert!(!exhausted() && counting());
        let fresh = Arc::new(AtomicBool::new(true));
        let caught =
            std::panic::catch_unwind(|| stopping_when(&fresh, || -> u8 { panic!("on purpose") }));
        assert!(caught.is_err());
        assert!(!exhausted());
    }
}
