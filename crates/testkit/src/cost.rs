//! Bounds on what a computation costs that a busy machine does not break.
//!
//! A wall clock counts the time a thread waits for a core: under the gate's
//! load (seven test binaries at once beside other agents) a computation
//! that takes 4 s alone took 25 s on the wall (gate 90), and a solve of
//! 0.5 s took 13.7 s (gate 56). The bounds of the gated tests are there to
//! catch work that grew (a loop that is not bounded, a search that became
//! quadratic), so they are measured on the work done:
//!
//! - [`WorkClock`]: the CPU time of the calling thread (Linux:
//!   `/proc/thread-self/schedstat`; the wall clock elsewhere). For a bound
//!   on a fixed input ("never hangs") of work done on that thread.
//! - [`ProcessClock`]: the CPU time of every thread of the process, for
//!   work spread over threads, in a test binary that runs nothing else
//!   meanwhile.
//! - [`assert_growth_below`] / [`assert_not_quadratic`]: the same work at
//!   `n` and at `k n`, compared with each other. A slower machine or a
//!   busier one stretches both alike; a quadratic search costs `k²` times
//!   as much where a linear one costs `k` times.
//!
//! Timings proper (how fast, in release) are ignored tests that print.

use std::time::{Duration, Instant};

/// Nanoseconds the calling thread has spent on a CPU: the first field of
/// `/proc/thread-self/schedstat`; `None` where there is no such file.
pub fn thread_cpu_nanos() -> Option<u64> {
    std::fs::read_to_string("/proc/thread-self/schedstat")
        .ok()?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// A clock of the work the calling thread does: its CPU time where the
/// system reports it, the wall clock elsewhere. Work done on other threads
/// is not counted, so it measures computations that run on the thread
/// that started the clock.
#[derive(Clone, Copy, Debug)]
pub struct WorkClock {
    cpu: Option<u64>,
    wall: Instant,
}

impl WorkClock {
    /// Starts the clock.
    pub fn start() -> Self {
        Self {
            cpu: thread_cpu_nanos(),
            wall: Instant::now(),
        }
    }

    /// The work done on this thread since [`WorkClock::start`].
    pub fn elapsed(&self) -> Duration {
        match (self.cpu, thread_cpu_nanos()) {
            (Some(start), Some(now)) => Duration::from_nanos(now.saturating_sub(start)),
            _ => self.wall.elapsed(),
        }
    }
}

/// The CPU time of the whole process, every thread of it, from the
/// fields `utime` and `stime` of `/proc/self/stat` (in clock ticks of
/// 1/100 s, which is what Linux reports there on every architecture);
/// `None` where there is no such file.
pub fn process_cpu_nanos() -> Option<u64> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    // The command name (in parentheses) may hold spaces: count from after it.
    let rest = stat.get(stat.rfind(')')?.checked_add(1)?..)?;
    let mut fields = rest.split_whitespace().skip(11);
    let user: u64 = fields.next()?.parse().ok()?;
    let system: u64 = fields.next()?.parse().ok()?;
    user.checked_add(system)?.checked_mul(10_000_000)
}

/// A clock of the work of every thread of the process ([`process_cpu_nanos`];
/// the wall clock elsewhere): for a computation that spreads over threads,
/// in a test binary that does nothing else meanwhile (one test, or
/// `--test-threads=1`), since other tests' work counts too. Its grain is
/// a hundredth of a second.
#[derive(Clone, Copy, Debug)]
pub struct ProcessClock {
    cpu: Option<u64>,
    wall: Instant,
}

impl ProcessClock {
    /// Starts the clock.
    pub fn start() -> Self {
        Self {
            cpu: process_cpu_nanos(),
            wall: Instant::now(),
        }
    }

    /// The work done by the process since [`ProcessClock::start`].
    pub fn elapsed(&self) -> Duration {
        match (self.cpu, process_cpu_nanos()) {
            (Some(start), Some(now)) => Duration::from_nanos(now.saturating_sub(start)),
            _ => self.wall.elapsed(),
        }
    }
}

/// `f()` and the work it took on this thread ([`WorkClock`]).
pub fn work_of<T>(f: impl FnOnce() -> T) -> (T, Duration) {
    let clock = WorkClock::start();
    let value = f();
    (value, clock.elapsed())
}

/// The least work [`WorkClock`] measures of `runs` runs of `f`.
pub fn least_work(runs: usize, mut f: impl FnMut()) -> Duration {
    (0..runs.max(1))
        .map(|_| work_of(&mut f).1)
        .min()
        .unwrap_or(Duration::ZERO)
}

/// Work [`work_per_run`] measures at least, in one go: the thread's CPU
/// time is brought up to date only when the scheduler looks at the thread
/// (at a tick or a switch, every few milliseconds), so a run shorter than
/// that can read as no work at all (gate 95: 0 ns against 1.5 ms).
const MIN_SAMPLE: Duration = Duration::from_millis(20);
/// Runs [`work_per_run`] makes at most, however short each one is.
const MAX_REPEATS: u32 = 10_000;

/// The work of one run of `f` on this thread ([`WorkClock`]): `f` is run
/// again until the clock has counted [`MIN_SAMPLE`] (or [`MAX_REPEATS`]
/// runs, or as long on the wall), and the work is shared out over the runs.
pub fn work_per_run(mut f: impl FnMut()) -> Duration {
    let clock = WorkClock::start();
    let mut runs = 0u32;
    loop {
        f();
        runs = runs.saturating_add(1);
        let work = clock.elapsed();
        if work >= MIN_SAMPLE || runs >= MAX_REPEATS || clock.wall.elapsed() >= MIN_SAMPLE * 50 {
            return work / runs.max(1);
        }
    }
}

/// How the work of `run(small)` and `run(large)` compare.
#[derive(Clone, Copy, Debug)]
pub struct Growth {
    /// The least work at the small size.
    pub small: Duration,
    /// The least work at the large size.
    pub large: Duration,
}

impl Growth {
    /// `large / small`, with `slack` added to the small work (for inputs
    /// so small that the clock's grain and fixed costs dominate).
    pub fn ratio(&self, slack: Duration) -> f64 {
        self.large.as_secs_f64() / (self.small + slack).as_secs_f64().max(1e-9)
    }
}

/// Rounds [`assert_growth_below`] measures in before it fails.
const ROUNDS: usize = 4;
/// Runs of the small size per round.
const SMALL_RUNS: usize = 3;

/// Asserts that the work of `run(large)` is less than `limit` times that
/// of `run(small)` (plus `slack`), measured on this thread
/// ([`work_per_run`], so a run shorter than the clock's grain is repeated
/// until it can be measured). Each round runs the small size [`SMALL_RUNS`] times
/// and the large one once, and the least of each over the rounds so far
/// counts: contention only ever adds work, so the least is the closest to
/// the work itself. It passes as soon as the ratio is below the limit
/// and fails after [`ROUNDS`] rounds above it: a computation that grows
/// too fast fails every round, one that does not passes once a quiet
/// moment came. The small size runs most, so that a moment of contention
/// on it cannot make a fast growth look slow, and a failure costs only a
/// few runs of the large size.
pub fn assert_growth_below(
    what: &str,
    (small, large): (usize, usize),
    limit: f64,
    slack: Duration,
    mut run: impl FnMut(usize),
) {
    let mut least = Growth {
        small: Duration::MAX,
        large: Duration::MAX,
    };
    for _ in 0..ROUNDS {
        for _ in 0..SMALL_RUNS {
            least.small = least.small.min(work_per_run(|| run(small)));
        }
        least.large = least.large.min(work_per_run(|| run(large)));
        if least.ratio(slack) < limit {
            return;
        }
    }
    assert!(
        least.ratio(slack) < limit,
        "{what}: {small} took {:?} and {large} took {:?} of work, {:.1} times as much \
         (the bound is {limit})",
        least.small,
        least.large,
        least.ratio(slack)
    );
}

/// [`assert_growth_below`] with the limit halfway (geometrically) between
/// linear and quadratic growth: `k^1.5` for `large = k small`. For `k = 4`
/// a linear computation takes 4 times as long (a little more with a
/// logarithm), a quadratic one 16 times, and the bound is 8.
pub fn assert_not_quadratic(what: &str, sizes: (usize, usize), run: impl FnMut(usize)) {
    let k = sizes.1 as f64 / sizes.0.max(1) as f64;
    assert_growth_below(what, sizes, k.powf(1.5), Duration::ZERO, run);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn spin(n: usize) -> u64 {
        (0..n as u64).fold(0u64, |a, i| std::hint::black_box(a.wrapping_mul(31) ^ i))
    }

    #[test]
    fn the_clock_counts_work_and_not_sleep() {
        let clock = WorkClock::start();
        std::thread::sleep(Duration::from_millis(30));
        let slept = clock.elapsed();
        if thread_cpu_nanos().is_some() {
            assert!(slept < Duration::from_millis(20), "{slept:?}");
        }
    }

    #[test]
    fn the_process_clock_counts_the_work_of_other_threads() {
        if process_cpu_nanos().is_none() {
            return;
        }
        let clock = ProcessClock::start();
        let workers: Vec<_> = (0..2)
            .map(|_| std::thread::spawn(|| spin(300_000_000)))
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        assert!(
            clock.elapsed() >= Duration::from_millis(20),
            "{:?}",
            clock.elapsed()
        );
    }

    #[test]
    fn a_run_shorter_than_the_clock_grain_is_still_measured() {
        let work = work_per_run(|| {
            std::hint::black_box(spin(20_000));
        });
        assert!(work > Duration::ZERO, "{work:?}");
        assert!(work < Duration::from_millis(5), "{work:?}");
    }

    #[test]
    fn linear_work_passes_and_quadratic_work_fails() {
        assert_not_quadratic("linear", (200_000, 800_000), |n| {
            std::hint::black_box(spin(n));
        });
        let quadratic = std::panic::catch_unwind(|| {
            assert_not_quadratic("quadratic", (400, 1600), |n| {
                for _ in 0..n {
                    std::hint::black_box(spin(n));
                }
            });
        });
        assert!(quadratic.is_err());
    }
}
