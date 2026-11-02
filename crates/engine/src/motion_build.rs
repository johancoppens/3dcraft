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

/// What a run on a copy of the motion's state hands in: grid points
/// solved, the configurations it built (as a placement builds them) and
/// what each instance asks for at the frames it looked at.
#[derive(Default)]
pub(crate) struct Worked {
    pub steps: Vec<GridStep>,
    pub built: Vec<PartDone>,
    pub seen: Vec<(f64, Vec<(String, String)>)>,
    /// A trace's points worked out (`view.trace` in the background): the
    /// trace, or why it could not be made.
    pub trace: Option<Result<crate::motion::Trace, String>>,
    /// A run that goes on: the job for the rest, queued again behind what
    /// waits (a trace a chunk of samples at a time).
    pub next: Option<GridJob>,
    /// What the run's panic said, when it panicked.
    pub panicked: Option<String>,
    /// The run stopped short: it panicked, or a moment of it could not be
    /// made. A grid run that failed is not started again for this sweep
    /// (it would fail again, and the session would ask for it every
    /// poll); the moments it was for are made where they are asked for.
    pub failed: bool,
}

/// A run on a copy of the motion's state: the grid solved a stretch
/// further (`Sweep::queue_grid`), or frames of a bake built
/// (`Sweep::queue_baked`). It is given a test of whether it was let go
/// of (a new sweep, the builder dropped), which it asks between its grid
/// points and frames, and then hands in what it has.
pub(crate) type GridJob = Box<dyn FnOnce(Cancel) -> Worked + Send>;

/// Whether a run was let go of (a new epoch, the builder dropped): asked
/// between its grid points and frames, and between the features of each
/// configuration it builds (`AsmState::cancel`).
pub(crate) type Cancel = Arc<dyn Fn() -> bool + Send + Sync>;

/// Which run a run is.
#[derive(Clone, PartialEq, Eq)]
enum Run {
    /// Grid points solved (`Sweep::queue_grid`).
    Grid,
    /// Frames of a bake (`Sweep::queue_baked`).
    Frames,
    /// A trace's points (`view.trace` in the background): its name and
    /// the serial it was asked for with.
    Trace(String, u64),
}

/// A trace worked out in the background, as it is handed in.
pub(crate) struct TraceDone {
    pub name: String,
    pub serial: u64,
    pub result: Result<crate::motion::Trace, String>,
}

/// What a worker is given.
enum Job {
    Part(Box<PartJob>),
    Grid(GridJob, Run),
}

/// What a worker gives back.
enum Done {
    Part(PartDone),
    Grid(Box<Worked>, Run),
}

/// Most configurations queued or being built at once. Each job holds a
/// copy of its source document and of the regeneration cache of its
/// source (whose bodies are shared), so this bounds the queue's memory
/// (docs/design/motion.md › *Shapes in the background*, the budget); the
/// configurations not queued are asked for again by the next moment that
/// lacks them, and a bake's frames are built by runs, not queued here.
pub(crate) const MAX_QUEUED: usize = 256;

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
    /// A grid run of this epoch failed ([`Worked::failed`]), not taken yet.
    grid_failed: bool,
    /// Traces being worked out, and those done and not taken yet.
    traces_busy: usize,
    traces_done: Vec<TraceDone>,
    /// Configurations built and not taken yet: whatever call collects
    /// them keeps them here until [`Self::take`].
    parts_done: Vec<PartDone>,
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
        self.grid_failed = false;
        self.parts_done.clear();
        self.traces_busy = 0;
        self.traces_done.clear();
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
        self.grid_busy = self.push(vec![Job::Grid(job, Run::Grid)], true);
        self.grid_busy
    }

    /// Builds frames of a bake in the background, behind what is asked
    /// for at once: false without a thread.
    pub(crate) fn queue_frames(&mut self, job: GridJob) -> bool {
        if !Self::available() {
            return false;
        }
        let queued = self.push(vec![Job::Grid(job, Run::Frames)], false);
        if queued {
            self.frames_busy = self.frames_busy.saturating_add(1);
        }
        queued
    }

    /// Works out a trace's points in the background (`view.trace` in a
    /// window): false without a thread.
    pub(crate) fn queue_trace(&mut self, job: GridJob, name: String, serial: u64) -> bool {
        if !Self::available() {
            return false;
        }
        let queued = self.push(vec![Job::Grid(job, Run::Trace(name, serial))], false);
        if queued {
            self.traces_busy = self.traces_busy.saturating_add(1);
        }
        queued
    }

    /// Traces being worked out.
    pub(crate) fn traces_busy(&self) -> usize {
        self.traces_busy
    }

    /// The traces worked out since the last call.
    pub(crate) fn take_traces(&mut self) -> Vec<TraceDone> {
        self.collect();
        std::mem::take(&mut self.traces_done)
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

    /// True once when a grid run failed since the last call.
    pub(crate) fn take_grid_failed(&mut self) -> bool {
        std::mem::take(&mut self.grid_failed)
    }

    /// The grid points solved since the last call, in order.
    pub(crate) fn take_grid(&mut self) -> Vec<GridStep> {
        self.collect();
        std::mem::take(&mut self.grid_done)
    }

    /// Takes in what the workers finished for this epoch, each kind kept
    /// until it is taken (a configuration collected by [`Self::take_grid`]
    /// is not lost: the review of 2026-11-02 found one built twice).
    fn collect(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(running) = &self.running else {
                return;
            };
            let parts = &mut self.parts_done;
            for done in running.finished(self.epoch) {
                match done {
                    Done::Part(d) => {
                        if self.in_flight.remove(&d.key) {
                            parts.push(d);
                        }
                    }
                    Done::Grid(worked, run) => {
                        let worked = *worked;
                        match run {
                            Run::Grid => {
                                self.grid_busy = false;
                                self.grid_failed |= worked.failed;
                            }
                            Run::Frames => {
                                self.frames_busy = self.frames_busy.saturating_sub(1);
                            }
                            Run::Trace(name, serial) => {
                                if let Some(next) = worked.next {
                                    // The rest, behind what waits now.
                                    running.push(
                                        self.epoch,
                                        vec![Job::Grid(next, Run::Trace(name, serial))],
                                        false,
                                    );
                                    continue;
                                }
                                self.traces_busy = self.traces_busy.saturating_sub(1);
                                let result = worked.trace.unwrap_or_else(|| {
                                    Err(match &worked.panicked {
                                        Some(why) => {
                                            format!("the trace could not be worked out: {why}")
                                        }
                                        None => "the trace could not be worked out".to_string(),
                                    })
                                });
                                self.traces_done.push(TraceDone {
                                    name,
                                    serial,
                                    result,
                                });
                            }
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
        }
    }

    /// The configurations built since the last call for this epoch, sorted
    /// by key.
    pub(crate) fn take(&mut self) -> Vec<PartDone> {
        self.collect();
        let mut done = std::mem::take(&mut self.parts_done);
        done.sort_by(|a, b| a.key.cmp(&b.key));
        done
    }

    /// The keys queued or being built.
    pub(crate) fn in_flight(&self) -> &BTreeSet<String> {
        &self.in_flight
    }

    /// Regeneration caches the builder's workers hold now (tests of
    /// their bound).
    #[cfg(test)]
    pub(crate) fn caches_held(&self) -> usize {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.running
                .as_ref()
                .map_or(0, native::Running::caches_held)
        }
        #[cfg(target_arch = "wasm32")]
        {
            0
        }
    }

    /// How many configurations are queued or being built.
    pub(crate) fn pending(&self) -> usize {
        self.in_flight.len()
    }
}

/// Most sources a worker keeps a regeneration cache of: the caches hold
/// the bodies of every feature of a configuration, so a worker that went
/// through many sources does not keep them all (part of the builder's
/// budget, docs/design/motion.md).
const MAX_CACHES: usize = 4;

/// Most bytes of bodies a worker's caches hold together, estimated as the
/// configuration each was last used for ([`crate::asm::regen_bytes`]): a
/// cache over it is not kept (the next configuration of its source is
/// regenerated from the session's cache, or from nothing). With three
/// workers at most 3 × 384 MiB; the caches are let go of as soon as the
/// motion they were for is (a new epoch), also by an idle worker.
#[cfg(not(target_arch = "wasm32"))]
const CACHE_BYTES: usize = 384 << 20;
#[cfg(target_arch = "wasm32")]
const CACHE_BYTES: usize = 64 << 20;

/// A worker's regeneration caches by source with their estimated bytes,
/// the one used last at the back.
#[derive(Default)]
struct Caches(Vec<(String, RegenCache, usize)>);

impl Caches {
    fn take(&mut self, source: &str) -> Option<RegenCache> {
        let at = self.0.iter().position(|(s, _, _)| s == source)?;
        Some(self.0.remove(at).1)
    }

    fn put(&mut self, source: String, cache: RegenCache, bytes: usize) {
        self.0.retain(|(s, _, _)| *s != source);
        self.0.push((source, cache, bytes));
        let over = self.0.len().saturating_sub(MAX_CACHES);
        self.0.drain(..over);
        while self.bytes() > CACHE_BYTES && !self.0.is_empty() {
            self.0.remove(0);
        }
    }

    fn bytes(&self) -> usize {
        self.0
            .iter()
            .fold(0usize, |sum, (_, _, bytes)| sum.saturating_add(*bytes))
    }

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn clear(&mut self) {
        self.0.clear();
    }

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn len(&self) -> usize {
        self.0.len()
    }
}

/// Builds one part's configuration as `asm` builds it. `stop` is asked
/// between its features (`craft3d_regen::progress::with_stop`): when it
/// says so the build ends there and gives nothing, so a configuration cut
/// short is never kept as a result (`None`).
fn build(
    job: PartJob,
    caches: &mut Caches,
    stop: impl FnMut() -> bool + 'static,
) -> Option<PartDone> {
    let PartJob {
        key,
        doc,
        source,
        cache,
        slim,
    } = job;
    let mut cache = source
        .as_ref()
        .and_then(|s| caches.take(s))
        .or(cache)
        .unwrap_or_default();
    let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        craft3d_regen::progress::with_stop(stop, || {
            craft3d_regen::regenerate_cached_needing(&doc, &mut cache, craft3d_regen::Need::All)
        })
    }));
    let built = match ran {
        Ok((_, true)) => return None,
        Ok((regen, false)) => {
            let bytes = crate::asm::regen_bytes(&regen);
            if let Some(source) = source {
                caches.put(source, cache, bytes);
            }
            Ok(Arc::new(Built::Part {
                doc: Arc::new(doc),
                regen: Arc::new(if slim {
                    crate::asm::slimmed(regen)
                } else {
                    regen
                }),
            }))
        }
        // The cache may hold half of what the panic interrupted: not kept.
        Err(_) => Err("its regeneration stopped".to_string()),
    };
    Some(PartDone {
        key,
        built,
        slim,
        solved: false,
    })
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::{Caches, Done, Job, build};
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

    /// Most threads the builder starts. Each works only while it holds a
    /// thread of the shared pool (`craft3d_math::pool::lease_for_worker`)
    /// and takes no more from it (`work_alone`): the builder's threads
    /// count against the pool's limit with everyone else's, and a
    /// regeneration a build runs (a pattern's unions) gets no helpers.
    const MAX_WORKERS: usize = 3;

    /// How long a worker waits before it asks the pool again for a thread
    /// when none was free.
    const NO_THREAD_WAIT: std::time::Duration = std::time::Duration::from_millis(25);

    struct Queue {
        jobs: VecDeque<(u64, Job)>,
    }

    struct Shared {
        queue: Mutex<Queue>,
        ready: Condvar,
        epoch: AtomicU64,
        /// The builder was dropped: workers end, runs stop at their next
        /// point.
        stop: AtomicBool,
        /// Regeneration caches the workers hold now, together.
        cached: AtomicUsize,
    }

    fn lock(shared: &Shared) -> MutexGuard<'_, Queue> {
        shared.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The threads and the way to and from them. Dropped, the threads are
    /// told to stop and are not waited for (detached, so dropping a
    /// session never waits for a regeneration): a run stops at its next
    /// grid point or frame, a configuration's build ends with the build,
    /// and then the thread ends, its results let go of.
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
                }),
                ready: Condvar::new(),
                epoch: AtomicU64::new(epoch),
                stop: AtomicBool::new(false),
                cached: AtomicUsize::new(0),
            });
            let (send, done) = mpsc::channel();
            // A quarter of the machine, at most three: the window's
            // thread, the engine thread, the refiner and the GPU driver
            // keep the rest, so a play stays smooth while the builder
            // works.
            let workers = (craft3d_math::available_threads() / 4).clamp(1, MAX_WORKERS);
            let mut started = 0;
            for n in 0..workers {
                let (shared, send) = (Arc::clone(&shared), send.clone());
                let spawned = std::thread::Builder::new()
                    .name(format!("craft3d-motion-{n}"))
                    .spawn(move || work(shared, &send));
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

        /// Regeneration caches the workers hold now.
        #[cfg(test)]
        pub(super) fn caches_held(&self) -> usize {
            self.shared.cached.load(Ordering::Acquire)
        }

        pub(super) fn reset(&self, epoch: u64) {
            self.shared.epoch.store(epoch, Ordering::Release);
            lock(&self.shared).jobs.clear();
            // Idle workers wake to let go of their caches.
            self.shared.ready.notify_all();
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
            self.shared.stop.store(true, Ordering::Release);
            lock(&self.shared).jobs.clear();
            self.shared.ready.notify_all();
        }
    }

    /// A worker: takes the next job of the current epoch and builds it
    /// while it holds a thread of the shared pool, keeping the
    /// regeneration caches of the last configurations it built
    /// ([`Caches`], let go of when the epoch changes).
    fn work(shared: Arc<Shared>, send: &Sender<(u64, Done)>) {
        craft3d_math::pool::work_alone();
        let mut caches = Caches::default();
        let mut cached_for = shared.epoch.load(Ordering::Acquire);
        loop {
            let (epoch, job, _thread) = {
                let mut queue = lock(&shared);
                loop {
                    if shared.stop.load(Ordering::Acquire) {
                        return;
                    }
                    // The motion they were for was let go of: so are the
                    // caches, also by a worker with nothing to do.
                    let now = shared.epoch.load(Ordering::Acquire);
                    if cached_for != now {
                        cached_for = now;
                        if !caches.is_empty() {
                            drop(queue);
                            held(&shared, &caches, 0);
                            caches.clear();
                            queue = lock(&shared);
                            continue;
                        }
                    }
                    if queue.jobs.is_empty() {
                        queue = shared
                            .ready
                            .wait(queue)
                            .unwrap_or_else(PoisonError::into_inner);
                        continue;
                    }
                    let thread = craft3d_math::pool::lease_for_worker();
                    if thread.threads() == 0 {
                        // Every thread of the pool works for someone:
                        // asked again in a moment.
                        queue = shared
                            .ready
                            .wait_timeout(queue, NO_THREAD_WAIT)
                            .map(|(queue, _)| queue)
                            .unwrap_or_else(|e| e.into_inner().0);
                        continue;
                    }
                    if let Some((epoch, job)) = queue.jobs.pop_front() {
                        break (epoch, job, thread);
                    }
                }
            };
            if epoch != cached_for {
                continue;
            }
            let let_go: super::Cancel = {
                let shared = Arc::clone(&shared);
                Arc::new(move || {
                    shared.stop.load(Ordering::Acquire)
                        || shared.epoch.load(Ordering::Acquire) != epoch
                })
            };
            let before = caches.len();
            let done = match job {
                Job::Part(job) => match build(*job, &mut caches, move || let_go()) {
                    Some(done) => Done::Part(done),
                    // Cut short: let go of, as its epoch is.
                    None => {
                        held_change(&shared, before, caches.len());
                        continue;
                    }
                },
                Job::Grid(job, run) => Done::Grid(
                    Box::new(
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job(let_go)))
                            .unwrap_or_else(|payload| super::Worked {
                                failed: true,
                                panicked: Some(
                                    payload
                                        .downcast_ref::<&str>()
                                        .map(|s| (*s).to_string())
                                        .or_else(|| payload.downcast_ref::<String>().cloned())
                                        .unwrap_or_else(|| "it panicked".to_string()),
                                ),
                                ..super::Worked::default()
                            }),
                    ),
                    run,
                ),
            };
            held_change(&shared, before, caches.len());
            if send.send((epoch, done)).is_err() {
                return;
            }
        }
    }

    /// Counts a worker's caches going from `before` to `after`.
    fn held_change(shared: &Shared, before: usize, after: usize) {
        if after > before {
            shared
                .cached
                .fetch_add(after.saturating_sub(before), Ordering::AcqRel);
        } else {
            shared
                .cached
                .fetch_sub(before.saturating_sub(after), Ordering::AcqRel);
        }
    }

    /// Counts `caches` let go of (to `after` held).
    fn held(shared: &Shared, caches: &Caches, after: usize) {
        held_change(shared, caches.len(), after);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use std::time::{Duration, Instant};

    fn job(n: usize) -> PartJob {
        PartJob {
            key: format!("job-{n:05}"),
            doc: Document::default(),
            source: None,
            cache: None,
            slim: false,
        }
    }

    /// Waits until `builder` has taken in `n` configurations.
    fn take_all(builder: &mut Builder, n: usize) -> Vec<PartDone> {
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut done = Vec::new();
        while done.len() < n {
            done.extend(builder.take());
            assert!(Instant::now() < deadline, "{} of {n} built", done.len());
            std::thread::sleep(Duration::from_millis(2));
        }
        done
    }

    /// The queue holds at most [`MAX_QUEUED`] configurations (each with a
    /// copy of its document), and the builder's threads work only while
    /// they hold a thread of the shared pool (2026-11-02).
    #[test]
    fn the_queue_is_bounded_and_workers_take_threads_from_the_pool() {
        if !Builder::available() {
            return;
        }
        // Every thread of the pool taken (others' leases come back and
        // are taken too): the builder's workers get none and build nothing.
        let mut held = Vec::new();
        let mut have = 0;
        let deadline = Instant::now() + Duration::from_secs(60);
        while have < craft3d_math::pool::SHARED_THREADS {
            let lease = craft3d_math::pool::lease(craft3d_math::pool::SHARED_THREADS);
            have += lease.threads();
            held.push(lease);
            assert!(Instant::now() < deadline, "the pool never came back");
            std::thread::sleep(Duration::from_millis(1));
        }
        let mut builder = Builder::default();
        let queued = builder.queue((0..1000).map(job).collect(), false);
        assert_eq!(queued, MAX_QUEUED);
        assert_eq!(builder.pending(), MAX_QUEUED);
        // Asked again: nothing more fits.
        assert_eq!(builder.queue(vec![job(5000)], true), 0);
        std::thread::sleep(Duration::from_millis(150));
        assert!(
            builder.take().is_empty(),
            "built without a thread of the pool"
        );
        drop(held);
        let done = take_all(&mut builder, MAX_QUEUED);
        assert_eq!(done.len(), MAX_QUEUED);
        assert_eq!(builder.pending(), 0);
    }

    /// A new epoch lets go of what was queued and of what a run was doing:
    /// the run is told between its points and its results are not taken.
    #[test]
    fn a_reset_stops_a_run_between_its_points_and_drops_its_results() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        if !Builder::available() {
            return;
        }
        let mut builder = Builder::default();
        let points = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&points);
        assert!(builder.queue_grid(Box::new(move |let_go: Cancel| {
            let mut steps = 0;
            while !let_go() && steps < 10_000 {
                counted.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(1));
                steps += 1;
            }
            Worked::default()
        })));
        let deadline = Instant::now() + Duration::from_secs(60);
        while points.load(Ordering::SeqCst) == 0 {
            assert!(Instant::now() < deadline, "the run never started");
            std::thread::sleep(Duration::from_millis(1));
        }
        builder.reset();
        assert!(!builder.grid_busy());
        // The run stops at its next point, far short of its 10 000.
        std::thread::sleep(Duration::from_millis(100));
        let stopped = points.load(Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(points.load(Ordering::SeqCst), stopped);
        assert!(stopped < 5_000, "{stopped}");
        assert!(builder.take_grid().is_empty());
        assert!(!builder.take_grid_failed());
    }

    /// A document of two blocks: a regeneration that can stop between
    /// its features.
    fn two_blocks() -> Document {
        let mut s = crate::Session::new(crate::Workspace::none());
        for (id, params) in [
            ("file.new", serde_json::json!({})),
            (
                "feature.block",
                serde_json::json!({"name": "A", "size": [1, 1, 1]}),
            ),
            (
                "feature.block",
                serde_json::json!({"name": "B", "size": [1, 1, 1], "position": [3, 0, 0]}),
            ),
        ] {
            s.execute(id, &params).unwrap();
        }
        (**s.document().unwrap().history.current()).clone()
    }

    /// A configuration's build asks between features whether it was let
    /// go of, and one cut short gives nothing (never a result); its cache
    /// is not kept. One built to its end is kept with its cache.
    #[test]
    fn a_part_build_stops_between_features_and_keeps_nothing() {
        let doc = two_blocks();
        let job = |doc: &Document| PartJob {
            key: "k".to_string(),
            doc: doc.clone(),
            source: Some("src".to_string()),
            cache: None,
            slim: false,
        };
        let mut caches = Caches::default();
        assert!(build(job(&doc), &mut caches, || true).is_none());
        assert!(caches.is_empty());
        let done = build(job(&doc), &mut caches, || false).unwrap();
        assert!(done.built.is_ok());
        assert_eq!(caches.len(), 1);
    }

    /// The caches are bounded by bytes as well as by count.
    #[test]
    fn caches_are_bounded_by_bytes() {
        let mut caches = Caches::default();
        caches.put("a".into(), RegenCache::default(), CACHE_BYTES / 2);
        caches.put("b".into(), RegenCache::default(), CACHE_BYTES / 2);
        assert_eq!(caches.len(), 2);
        caches.put("c".into(), RegenCache::default(), CACHE_BYTES / 2);
        assert_eq!(caches.len(), 2, "the oldest went");
        assert!(caches.take("a").is_none());
        caches.put("huge".into(), RegenCache::default(), CACHE_BYTES + 1);
        assert!(caches.is_empty(), "one over the budget is not kept");
    }

    /// A reset wakes idle workers, which let go of their caches.
    #[test]
    fn a_reset_lets_idle_workers_drop_their_caches() {
        if !Builder::available() {
            return;
        }
        let mut builder = Builder::default();
        let job = PartJob {
            key: "k".to_string(),
            doc: two_blocks(),
            source: Some("src".to_string()),
            cache: None,
            slim: false,
        };
        assert_eq!(builder.queue(vec![job], true), 1);
        take_all(&mut builder, 1);
        let deadline = Instant::now() + Duration::from_secs(60);
        while builder.caches_held() == 0 {
            assert!(Instant::now() < deadline, "no cache was kept");
            std::thread::sleep(Duration::from_millis(1));
        }
        builder.reset();
        while builder.caches_held() > 0 {
            assert!(
                Instant::now() < deadline,
                "the caches were kept after a reset"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// A configuration collected while the grid's results are taken is
    /// kept for the next [`Builder::take`], not lost (the review of
    /// 2026-11-02: it was built a second time).
    #[test]
    fn a_build_collected_with_the_grid_is_not_lost() {
        if !Builder::available() {
            return;
        }
        let mut builder = Builder::default();
        assert_eq!(builder.queue(vec![job(1)], true), 1);
        let deadline = Instant::now() + Duration::from_secs(60);
        while builder.pending() > 0 {
            assert!(builder.take_grid().is_empty());
            assert!(Instant::now() < deadline, "never built");
            std::thread::sleep(Duration::from_millis(1));
        }
        let done = builder.take();
        assert_eq!(done.len(), 1);
        assert_eq!(done.first().map(|d| d.key.as_str()), Some("job-00001"));
    }
}
