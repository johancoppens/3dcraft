//! Configurations of a motion built in the background
//! (docs/design/motion.md › *Shapes in the background*): while the window
//! plays a motion whose instances take parameters of `t`, the moment shown
//! is answered at once with the configurations that are kept
//! (`view.motion {shapes: "ready"}`), and the ones it lacks are built here,
//! on threads of their own, and taken in by the session at the start of the
//! next motion command or poll ([`crate::Session::poll_motion`]).
//!
//! A job is a part's configured document (with a copy of the regeneration
//! cache of the last configuration built of its source, which is `Send`);
//! its result is the configuration [`crate::asm`] would have built itself,
//! body for body (`craft3d_regen::regenerate_cached_needing` with
//! `Need::All` is a regeneration from nothing): only when it is built
//! differs, never what. Results are taken in sorted by their keys.
//!
//! In the browser (one thread) there is no builder: [`Builder::available`]
//! is false and every moment is made where it is asked for, as before.

// In the browser nothing is built in the background: the jobs are made
// and never run.
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

use crate::asm::Built;
use craft3d_doc::Document;
use craft3d_regen::RegenCache;
use std::collections::BTreeSet;
use std::sync::Arc;

/// A part's configuration to build.
pub(crate) struct PartJob {
    /// Its key among the configurations ([`crate::asm::config_key`] with
    /// the unit).
    pub key: String,
    /// The source with its parameters set.
    pub doc: Document,
    /// The source's hash and unit, for a part given parameters: the
    /// regeneration cache of its last configuration is kept under it.
    pub source: Option<String>,
    /// That cache as the session has it now (a start for a worker that has
    /// none of its own for the source).
    pub cache: Option<RegenCache>,
    /// Kept slim (`asm::slimmed`).
    pub slim: bool,
}

/// A configuration built.
pub(crate) struct PartDone {
    pub key: String,
    pub built: Result<Arc<Built>, String>,
    pub slim: bool,
    /// Used by a grid point's solve (and only that, so far): let go of
    /// once its grid point is solved, unless a frame asks for it.
    pub solved: bool,
}

/// A grid point of an assembly's motion solved in the background: its
/// index, the state reached and where the motion stopped before it.
pub(crate) struct GridStep {
    pub k: i64,
    pub carried: crate::asm::Carried,
    pub stuck: Option<crate::motion::Stuck>,
}

/// Solves grid points one after the other on a copy of the motion's
/// state (`Sweep::grid_job`).
/// What a run on a copy of the motion's state hands in: grid points
/// solved, the configurations it built (as a placement builds them) and
/// what each instance asks for at the frames it looked at.
#[derive(Default)]
pub(crate) struct Worked {
    pub steps: Vec<GridStep>,
    pub built: Vec<PartDone>,
    pub seen: Vec<(f64, Vec<(String, String)>)>,
}

/// A run on a copy of the motion's state: the grid solved a stretch
/// further (`Sweep::queue_grid`), or frames of a bake built
/// (`Sweep::queue_baked`).
pub(crate) type GridJob = Box<dyn FnOnce() -> Worked + Send>;

/// What a worker is given.
enum Job {
    Part(Box<PartJob>),
    /// A run, and whether it is the grid's.
    Grid(GridJob, bool),
}

/// What a worker gives back.
enum Done {
    Part(PartDone),
    Grid(Worked, bool),
}

/// Most jobs waiting at once: a bake of a thousand frames through the
/// grid asks for about two thousand configurations of one instance.
const MAX_QUEUED: usize = 16_384;

/// The background builder of one open document's motion. A clone has
/// none: background work stays with the session that asked for it.
#[derive(Default)]
pub(crate) struct Builder {
    /// Counts the sweeps: results of an earlier one are let go of.
    epoch: u64,
    /// Keys queued or being built for this epoch.
    in_flight: BTreeSet<String>,
    /// A run of grid points is being solved.
    grid_busy: bool,
    /// Runs of frames being built.
    frames_busy: usize,
    /// Grid points solved and not taken yet.
    grid_done: Vec<GridStep>,
    /// What runs saw the instances ask for, not taken yet.
    seen_done: Vec<(f64, Vec<(String, String)>)>,
    #[cfg(not(target_arch = "wasm32"))]
    running: Option<native::Running>,
}

impl Clone for Builder {
    fn clone(&self) -> Self {
        Builder::default()
    }
}

impl Builder {
    /// True where configurations can be built in the background: natively
    /// with a thread to spare.
    pub(crate) fn available() -> bool {
        #[cfg(target_arch = "wasm32")]
        {
            false
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            craft3d_math::available_threads() >= 2
        }
    }

    /// A new sweep: what is queued is dropped, and what is being built
    /// is let go of when it comes.
    pub(crate) fn reset(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.in_flight.clear();
        self.grid_busy = false;
        self.frames_busy = 0;
        self.grid_done.clear();
        self.seen_done.clear();
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(running) = &self.running {
            running.reset(self.epoch);
        }
    }

    /// Queues `jobs` (those not queued already), at the front when
    /// `urgent` (in the order given), else at the back. Returns how many
    /// were queued. Nothing where [`Self::available`] is false.
    pub(crate) fn queue(&mut self, jobs: Vec<PartJob>, urgent: bool) -> usize {
        if !Self::available() {
            return 0;
        }
        let fresh: Vec<PartJob> = jobs
            .into_iter()
            .filter(|job| !self.in_flight.contains(&job.key))
            .take(MAX_QUEUED.saturating_sub(self.in_flight.len()))
            .collect();
        if fresh.is_empty() {
            return 0;
        }
        for job in &fresh {
            self.in_flight.insert(job.key.clone());
        }
        let n = fresh.len();
        let jobs = fresh
            .into_iter()
            .map(|job| Job::Part(Box::new(job)))
            .collect();
        if !self.push(jobs, urgent) {
            // No thread: nothing is built here; the moments are made
            // where they are asked for.
            self.in_flight.clear();
            return 0;
        }
        n
    }

    /// Solves a run of grid points in the background (one run at a time):
    /// false when one runs already or there is no thread.
    pub(crate) fn queue_grid(&mut self, job: GridJob) -> bool {
        if !Self::available() || self.grid_busy {
            return false;
        }
        self.grid_busy = self.push(vec![Job::Grid(job, true)], true);
        self.grid_busy
    }

    /// Builds frames of a bake in the background, behind what is asked
    /// for at once: false without a thread.
    pub(crate) fn queue_frames(&mut self, job: GridJob) -> bool {
        if !Self::available() {
            return false;
        }
        let queued = self.push(vec![Job::Grid(job, false)], false);
        if queued {
            self.frames_busy = self.frames_busy.saturating_add(1);
        }
        queued
    }

    /// Runs of frames being built.
    pub(crate) fn frames_busy(&self) -> usize {
        self.frames_busy
    }

    /// What runs saw the instances ask for at their frames, since the
    /// last call.
    pub(crate) fn take_seen(&mut self) -> Vec<(f64, Vec<(String, String)>)> {
        std::mem::take(&mut self.seen_done)
    }

    /// True while a run of grid points is being solved.
    pub(crate) fn grid_busy(&self) -> bool {
        self.grid_busy
    }

    fn push(&mut self, jobs: Vec<Job>, urgent: bool) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let epoch = self.epoch;
            if self.running.is_none() {
                match native::Running::start(epoch) {
                    Ok(running) => self.running = Some(running),
                    Err(_) => return false,
                }
            }
            if let Some(running) = &self.running {
                running.push(epoch, jobs, urgent);
                return true;
            }
            false
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (jobs, urgent);
            false
        }
    }

    /// The grid points solved since the last call, in order.
    pub(crate) fn take_grid(&mut self) -> Vec<GridStep> {
        self.collect();
        std::mem::take(&mut self.grid_done)
    }

    /// Takes in what the workers finished for this epoch.
    fn collect(&mut self) -> Vec<PartDone> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(running) = &self.running else {
                return Vec::new();
            };
            let mut parts = Vec::new();
            for done in running.finished(self.epoch) {
                match done {
                    Done::Part(d) => {
                        if self.in_flight.remove(&d.key) {
                            parts.push(d);
                        }
                    }
                    Done::Grid(worked, grid) => {
                        if grid {
                            self.grid_busy = false;
                        } else {
                            self.frames_busy = self.frames_busy.saturating_sub(1);
                        }
                        self.grid_done.extend(worked.steps);
                        self.seen_done.extend(worked.seen);
                        parts.extend(
                            worked
                                .built
                                .into_iter()
                                .filter(|d| !self.in_flight.contains(&d.key)),
                        );
                    }
                }
            }
            parts
        }
        #[cfg(target_arch = "wasm32")]
        {
            Vec::new()
        }
    }

    /// The configurations built since the last call for this epoch, sorted
    /// by key.
    pub(crate) fn take(&mut self) -> Vec<PartDone> {
        let mut done = self.collect();
        done.sort_by(|a, b| a.key.cmp(&b.key));
        done
    }

    /// The keys queued or being built.
    pub(crate) fn in_flight(&self) -> &BTreeSet<String> {
        &self.in_flight
    }

    /// How many configurations are queued or being built.
    pub(crate) fn pending(&self) -> usize {
        self.in_flight.len()
    }
}

/// Builds one part's configuration as `asm` builds it.
fn build(job: PartJob, caches: &mut std::collections::BTreeMap<String, RegenCache>) -> PartDone {
    let PartJob {
        key,
        doc,
        source,
        cache,
        slim,
    } = job;
    let mut cache = source
        .as_ref()
        .and_then(|s| caches.remove(s))
        .or(cache)
        .unwrap_or_default();
    let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        craft3d_regen::regenerate_cached_needing(&doc, &mut cache, craft3d_regen::Need::All)
    }));
    let built = match ran {
        Ok(regen) => Ok(Arc::new(Built::Part {
            doc: Arc::new(doc),
            regen: Arc::new(if slim {
                crate::asm::slimmed(regen)
            } else {
                regen
            }),
        })),
        Err(_) => Err("its regeneration stopped".to_string()),
    };
    if let Some(source) = source {
        caches.insert(source, cache);
    }
    PartDone {
        key,
        built,
        slim,
        solved: false,
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::{Done, Job, build};
    use craft3d_regen::RegenCache;
    use std::collections::{BTreeMap, VecDeque};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

    /// Most threads the builder starts (each build may take more from the
    /// shared pool, `pool.rs`).
    const MAX_WORKERS: usize = 3;

    struct Queue {
        jobs: VecDeque<(u64, Job)>,
        stop: bool,
    }

    struct Shared {
        queue: Mutex<Queue>,
        ready: Condvar,
        epoch: AtomicU64,
    }

    fn lock(shared: &Shared) -> MutexGuard<'_, Queue> {
        shared.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The threads and the way to and from them. Dropped, the threads end
    /// after the job they are on (they are not waited for).
    pub(super) struct Running {
        shared: Arc<Shared>,
        /// Behind a lock so that a session stays `Sync`.
        done: Mutex<Receiver<(u64, Done)>>,
    }

    impl Running {
        pub(super) fn start(epoch: u64) -> Result<Self, String> {
            let shared = Arc::new(Shared {
                queue: Mutex::new(Queue {
                    jobs: VecDeque::new(),
                    stop: false,
                }),
                ready: Condvar::new(),
                epoch: AtomicU64::new(epoch),
            });
            let (send, done) = mpsc::channel();
            // Half the machine at most: the window's thread, the engine
            // thread, the refiner and the GPU driver keep the rest, so a
            // play stays smooth while the builder works.
            let workers = (craft3d_math::available_threads() / 4).clamp(1, MAX_WORKERS);
            let mut started = 0;
            for n in 0..workers {
                let (shared, send) = (Arc::clone(&shared), send.clone());
                let spawned = std::thread::Builder::new()
                    .name(format!("craft3d-motion-{n}"))
                    .spawn(move || work(&shared, &send));
                if spawned.is_ok() {
                    started += 1;
                }
            }
            if started == 0 {
                return Err("no thread could be started".to_string());
            }
            Ok(Self {
                shared,
                done: Mutex::new(done),
            })
        }

        pub(super) fn reset(&self, epoch: u64) {
            self.shared.epoch.store(epoch, Ordering::Release);
            lock(&self.shared).jobs.clear();
        }

        pub(super) fn push(&self, epoch: u64, jobs: Vec<Job>, urgent: bool) {
            let mut queue = lock(&self.shared);
            if urgent {
                for job in jobs.into_iter().rev() {
                    queue.jobs.push_front((epoch, job));
                }
            } else {
                queue.jobs.extend(jobs.into_iter().map(|job| (epoch, job)));
            }
            drop(queue);
            self.shared.ready.notify_all();
        }

        pub(super) fn finished(&self, epoch: u64) -> Vec<Done> {
            self.done
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .try_iter()
                .filter(|(e, _)| *e == epoch)
                .map(|(_, done)| done)
                .collect()
        }
    }

    impl Drop for Running {
        fn drop(&mut self) {
            lock(&self.shared).stop = true;
            self.shared.ready.notify_all();
        }
    }

    /// A worker: takes the next job of the current epoch and builds it,
    /// keeping the regeneration cache of the last configuration of each
    /// source it built (let go of when the epoch changes).
    fn work(shared: &Shared, send: &Sender<(u64, Done)>) {
        craft3d_math::pool::work_alone();
        let mut caches: BTreeMap<String, RegenCache> = BTreeMap::new();
        let mut cached_for = shared.epoch.load(Ordering::Acquire);
        loop {
            let (epoch, job) = {
                let mut queue = lock(shared);
                loop {
                    if queue.stop {
                        return;
                    }
                    if let Some(next) = queue.jobs.pop_front() {
                        break next;
                    }
                    queue = shared
                        .ready
                        .wait(queue)
                        .unwrap_or_else(PoisonError::into_inner);
                }
            };
            let now = shared.epoch.load(Ordering::Acquire);
            if epoch != now {
                continue;
            }
            if cached_for != now {
                caches.clear();
                cached_for = now;
            }
            let done = match job {
                Job::Part(job) => Done::Part(build(*job, &mut caches)),
                Job::Grid(job, grid) => Done::Grid(
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)).unwrap_or_default(),
                    grid,
                ),
            };
            if send.send((epoch, done)).is_err() {
                return;
            }
        }
    }
}
