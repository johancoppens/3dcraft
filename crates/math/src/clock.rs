//! MEASUREMENT ONLY (grid-scale, 2026-11-17; removed before the work is
//! handed in): wall-clock time per named stage, summed over every time a
//! stage runs, read by the ignored timing tests. Off unless switched on;
//! nothing on wasm32.

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

/// What the clocks summed since the last call (stage, seconds, runs),
/// in the order the stages first ran.
pub fn take() -> Vec<(&'static str, f64, u64)> {
    std::mem::take(&mut *TOTALS.lock().unwrap_or_else(PoisonError::into_inner))
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
            let seconds = start.elapsed().as_secs_f64();
            let mut totals = TOTALS.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(entry) = totals.iter_mut().find(|(l, _, _)| *l == self.label) {
                entry.1 += seconds;
                entry.2 += 1;
            } else {
                totals.push((self.label, seconds, 1));
            }
        }
        let _ = self.label;
    }
}
