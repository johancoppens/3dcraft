//! MEASUREMENT ONLY (boolean-spans, 2026-11-17; removed before the work is
//! handed in): wall-clock time per named stage, summed over every time a
//! stage runs, and counts, read by the ignored timing tests. Off unless
//! switched on; nothing on wasm32.

use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::atomic::{AtomicBool, Ordering};

static ON: AtomicBool = AtomicBool::new(false);
static TOTALS: Mutex<Vec<(&'static str, f64, u64)>> = Mutex::new(Vec::new());

/// Switches the clocks on or off, and empties what they summed.
pub fn enable(on: bool) {
    ON.store(on, Ordering::Relaxed);
    TOTALS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clear();
}

/// Whether the clocks are on.
pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

/// What the clocks summed since the last call (stage, seconds, runs),
/// in the order the stages first ran.
pub fn take() -> Vec<(&'static str, f64, u64)> {
    std::mem::take(&mut *TOTALS.lock().unwrap_or_else(PoisonError::into_inner))
}

/// What the clocks summed so far, left in place.
pub fn peek() -> Vec<(&'static str, f64, u64)> {
    TOTALS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

fn add(label: &'static str, seconds: f64, runs: u64) {
    let mut totals = TOTALS.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(entry) = totals.iter_mut().find(|(l, _, _)| *l == label) {
        entry.1 += seconds;
        entry.2 = entry.2.saturating_add(runs);
    } else {
        totals.push((label, seconds, runs));
    }
}

/// Adds `n` to the count of `label` (no time).
pub fn count(label: &'static str, n: u64) {
    if ON.load(Ordering::Relaxed) {
        add(label, 0.0, n);
    }
}

/// A running clock: its time is added to its stage when it is dropped.
pub struct Clock {
    label: &'static str,
    #[cfg(not(target_arch = "wasm32"))]
    start: Option<std::time::Instant>,
}

/// A clock for `label`, running until it is dropped.
pub fn clock(label: &'static str) -> Clock {
    Clock {
        label,
        #[cfg(not(target_arch = "wasm32"))]
        start: ON.load(Ordering::Relaxed).then(std::time::Instant::now),
    }
}

impl Drop for Clock {
    fn drop(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(start) = self.start {
            add(self.label, start.elapsed().as_secs_f64(), 1);
        }
        let _ = self.label;
    }
}
