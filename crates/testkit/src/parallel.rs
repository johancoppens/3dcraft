//! Running independent test cases on every core.
//!
//! The long random tests (`panic_hunt`, `boolean_random`, `meet_random`)
//! are thousands of cases that do not depend on each other. Run one after
//! the other they took most of the gate's time on one core while the
//! others idled; [`map_parallel`] spreads them over the machine and keeps
//! the results in the order of the inputs, so the tables and digests a
//! test prints are the same whatever the thread count.

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

/// Runs `work` on every item, on as many threads as the machine has, and
/// returns the results in the order of the items.
///
/// A panic in one item stops the others from starting new items and is
/// raised again on the calling thread once every thread is done, with its
/// original payload: an assertion message reads as it would have on one
/// thread. Items must not depend on each other or on the order they run
/// in; a seeded random test gives each item its own seed.
pub fn map_parallel<T, R, F>(items: Vec<T>, work: F) -> Vec<R>
where
    T: Send,
    R: Send,
    F: Fn(T) -> R + Sync,
{
    let count = items.len();
    let threads = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(count)
        .max(1);
    let queue: Mutex<VecDeque<(usize, T)>> = Mutex::new(items.into_iter().enumerate().collect());
    let done: Mutex<Vec<(usize, R)>> = Mutex::new(Vec::with_capacity(count));
    let failed = AtomicBool::new(false);
    let failure: Mutex<Option<Box<dyn std::any::Any + Send>>> = Mutex::new(None);
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                while !failed.load(Ordering::Acquire) {
                    let next = queue
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .pop_front();
                    let Some((index, item)) = next else { break };
                    match catch_unwind(AssertUnwindSafe(|| work(item))) {
                        Ok(result) => done
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .push((index, result)),
                        Err(payload) => {
                            let mut first = failure.lock().unwrap_or_else(PoisonError::into_inner);
                            if first.is_none() {
                                *first = Some(payload);
                            }
                            failed.store(true, Ordering::Release);
                        }
                    }
                }
            });
        }
    });
    if let Some(payload) = failure
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
    {
        resume_unwind(payload);
    }
    let mut done = done.into_inner().unwrap_or_else(PoisonError::into_inner);
    done.sort_by_key(|(index, _)| *index);
    done.into_iter().map(|(_, result)| result).collect()
}

#[cfg(test)]
mod tests {
    use super::map_parallel;

    #[test]
    fn results_come_back_in_the_order_of_the_items() {
        let squares = map_parallel((0..1000_u64).collect(), |i| i * i);
        assert_eq!(squares.len(), 1000);
        assert!(
            squares
                .iter()
                .enumerate()
                .all(|(i, s)| *s == (i * i) as u64)
        );
    }

    #[test]
    fn an_empty_run_is_fine() {
        let none: Vec<u8> = map_parallel(Vec::<u8>::new(), |x| x);
        assert!(none.is_empty());
    }

    #[test]
    fn a_panic_in_one_item_fails_the_whole_run_with_its_message() {
        let outcome = std::panic::catch_unwind(|| {
            map_parallel((0..200_u32).collect(), |i| {
                assert!(i != 77, "item {i} is the broken one");
                i
            })
        });
        let payload = outcome.expect_err("one item panicked");
        let message = payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default();
        assert!(message.contains("item 77 is the broken one"), "{message}");
    }
}
