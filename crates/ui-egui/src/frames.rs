//! Which image the viewport shows, and when a new one is made.
//!
//! The sharp image of the CPU renderer takes 50 to 200 ms: too long for
//! the thread that draws the window. So the window shows a cheap preview
//! at once, has the sharp image made on a worker thread, and swaps it in
//! when it arrives, provided it is still the image of what is on screen.
//!
//! Two parts:
//!
//! - [`Scheduler`] decides. It is a state machine without threads or
//!   clocks: it is told what the viewport should show (a key: document
//!   revision, camera, size, …) and which results arrived, and answers
//!   what to render. An image is never shown for another key than the one
//!   wanted now, however late or out of order results arrive.
//! - [`Worker`] is the thread. It renders the newest job it was given and
//!   skips the ones that were overtaken while it was busy. A panic in a
//!   render is that job's error, not the end of the thread.
//!
//! A host without threads (a browser) uses the scheduler in
//! [`Mode::Inline`]: the sharp image is then rendered where it is asked
//! for, and there are no previews at rest.

/// How finished an image is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    /// Small and unfiltered: quick enough to make every frame.
    Preview,
    /// Full size, supersampled.
    Sharp,
}

/// Where sharp images are rendered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Where they are asked for: the caller renders the sharp image
    /// before going on, so a preview is only needed while moving.
    Inline,
    /// Elsewhere: a preview is shown until the sharp image arrives.
    Background,
}

/// Names one request for a sharp image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ticket(u64);

/// What to render now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    /// Render a preview of the key and show it, then call
    /// [`Scheduler::preview_shown`].
    pub preview: bool,
    /// Have the sharp image of the key rendered, and hand the outcome to
    /// [`Scheduler::deliver`] or [`Scheduler::fail`] with this ticket.
    pub sharp: Option<Ticket>,
}

/// Decides which image is made and which result may be shown. `K` is
/// everything an image depends on.
#[derive(Clone, Debug)]
pub struct Scheduler<K> {
    mode: Mode,
    /// What the viewport should show, as of the last [`Self::plan`].
    wanted: Option<K>,
    /// What it shows.
    shown: Option<(K, Level)>,
    /// The sharp image being made.
    pending: Option<(Ticket, K)>,
    /// A key whose sharp image could not be made: not asked for again
    /// until something changes.
    failed: Option<K>,
    next: u64,
}

impl<K: Clone + PartialEq> Scheduler<K> {
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            wanted: None,
            shown: None,
            pending: None,
            failed: None,
            next: 0,
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Changing the mode forgets the image being made: its result would
    /// arrive on a path that is no longer listened to.
    pub fn set_mode(&mut self, mode: Mode) {
        if self.mode != mode {
            self.mode = mode;
            self.pending = None;
        }
    }

    /// The key and level of the image on screen.
    pub fn shown(&self) -> Option<(&K, Level)> {
        self.shown.as_ref().map(|(key, level)| (key, *level))
    }

    /// The ticket of the sharp image being made.
    pub fn pending(&self) -> Option<Ticket> {
        self.pending.as_ref().map(|(ticket, _)| *ticket)
    }

    /// True when the image on screen is the sharp one of what is wanted.
    pub fn is_sharp(&self) -> bool {
        match (&self.shown, &self.wanted) {
            (Some((shown, Level::Sharp)), Some(wanted)) => shown == wanted,
            _ => false,
        }
    }

    /// The viewport should show `key`; `moving` while the camera is being
    /// moved (no sharp image is started then). Returns what to render.
    pub fn plan(&mut self, key: &K, moving: bool) -> Plan {
        self.wanted = Some(key.clone());
        let level = match &self.shown {
            Some((shown, level)) if shown == key => Some(*level),
            _ => None,
        };
        let wants_sharp =
            !moving && level != Some(Level::Sharp) && self.failed.as_ref() != Some(key);
        let sharp = if !wants_sharp {
            None
        } else if self
            .pending
            .as_ref()
            .is_some_and(|(_, pending)| pending == key)
        {
            // Already being made.
            None
        } else {
            let ticket = Ticket(self.next);
            self.next = self.next.wrapping_add(1);
            // An image still being made for another key is abandoned: its
            // ticket no longer matches.
            self.pending = Some((ticket, key.clone()));
            Some(ticket)
        };
        // Something of the key must be on screen now. Inline, the sharp
        // image is that something.
        let preview = level.is_none() && !(self.mode == Mode::Inline && sharp.is_some());
        Plan { preview, sharp }
    }

    /// The caller rendered and showed a preview of `key`.
    pub fn preview_shown(&mut self, key: &K) {
        self.shown = Some((key.clone(), Level::Preview));
    }

    /// The sharp image of `ticket` arrived. True when it is to be shown:
    /// it is the image asked for last, and of what is wanted now. Anything
    /// else is stale and must be dropped.
    pub fn deliver(&mut self, ticket: Ticket) -> bool {
        let Some((_, key)) = self.pending.take_if(|(pending, _)| *pending == ticket) else {
            return false;
        };
        if self.wanted.as_ref() != Some(&key) {
            // Overtaken: the next plan asks for the image of the new key.
            return false;
        }
        self.shown = Some((key, Level::Sharp));
        true
    }

    /// The sharp image of `ticket` could not be made. True when that is
    /// news about what is wanted now (the caller reports the reason); the
    /// key is then not asked for again until another one was wanted.
    pub fn fail(&mut self, ticket: Ticket) -> bool {
        let Some((_, key)) = self.pending.take_if(|(pending, _)| *pending == ticket) else {
            return false;
        };
        if self.wanted.as_ref() != Some(&key) {
            return false;
        }
        self.failed = Some(key);
        true
    }

    /// Nothing is on screen any more (the texture was lost): everything is
    /// made again.
    pub fn forget(&mut self) {
        self.shown = None;
        self.pending = None;
        self.failed = None;
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use worker::{Done, Worker};

#[cfg(not(target_arch = "wasm32"))]
mod worker {
    use super::Ticket;
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::thread::JoinHandle;
    use std::time::Duration;

    /// The outcome of one job.
    #[derive(Debug)]
    pub struct Done<R> {
        pub ticket: Ticket,
        /// The image, or why there is none (also when the render panicked).
        pub result: Result<R, String>,
    }

    /// A thread that renders sharp images. `J` is a job, `R` its image.
    ///
    /// Jobs are done one at a time. A job that was overtaken by a newer
    /// one before the thread got to it is skipped: its image would be
    /// stale. Dropping the worker ends the thread after the job it is
    /// busy with.
    pub struct Worker<J, R> {
        jobs: Option<Sender<(Ticket, J)>>,
        done: Receiver<Done<R>>,
        thread: Option<JoinHandle<()>>,
    }

    fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
        payload
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "no details".to_string())
    }

    impl<J: Send + 'static, R: Send + 'static> Worker<J, R> {
        /// Starts the thread. `render` makes the image of a job; `ready`
        /// is called after each result is in the queue (to ask for a
        /// repaint). An error when the thread cannot be started.
        pub fn start(
            render: impl Fn(&J) -> Result<R, String> + Send + 'static,
            ready: impl Fn(&J) + Send + 'static,
        ) -> Result<Self, String> {
            let (jobs, incoming) = mpsc::channel::<(Ticket, J)>();
            let (outgoing, done) = mpsc::channel::<Done<R>>();
            let thread = std::thread::Builder::new()
                .name("craft3d-render".to_string())
                .spawn(move || {
                    while let Ok(mut next) = incoming.recv() {
                        // Only the newest request is worth rendering.
                        while let Ok(newer) = incoming.try_recv() {
                            next = newer;
                        }
                        let (ticket, job) = next;
                        // A panic in the renderer must not end the thread,
                        // let alone the window (AGENTS.md, never crash).
                        let result =
                            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| render(&job)))
                                .unwrap_or_else(|payload| {
                                    Err(format!(
                                        "internal error (a bug in 3DCraft, please report it): {}",
                                        panic_text(payload.as_ref())
                                    ))
                                });
                        if outgoing.send(Done { ticket, result }).is_err() {
                            // Nobody is waiting any more.
                            break;
                        }
                        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            ready(&job);
                        }));
                    }
                })
                .map_err(|e| format!("the render thread could not be started: {e}"))?;
            Ok(Self {
                jobs: Some(jobs),
                done,
                thread: Some(thread),
            })
        }

        /// Hands a job to the thread. When the thread is gone the job
        /// comes back as the error; the caller then renders by itself.
        pub fn submit(&self, ticket: Ticket, job: J) -> Result<(), J> {
            match &self.jobs {
                Some(jobs) => jobs.send((ticket, job)).map_err(|refused| refused.0.1),
                None => Err(job),
            }
        }

        /// A result that is ready, without waiting.
        pub fn poll(&self) -> Option<Done<R>> {
            self.done.try_recv().ok()
        }

        /// The next result, waiting up to `timeout` for it. `None` when
        /// none came in time or the thread is gone.
        pub fn wait(&self, timeout: Duration) -> Option<Done<R>> {
            self.done.recv_timeout(timeout).ok()
        }

        /// False once the thread has ended, which it only does when it
        /// could not go on.
        pub fn alive(&self) -> bool {
            self.thread.as_ref().is_some_and(|t| !t.is_finished())
        }
    }

    impl<J, R> Drop for Worker<J, R> {
        fn drop(&mut self) {
            // Closing the queue ends the thread's loop.
            self.jobs = None;
            if let Some(thread) = self.thread.take() {
                // A thread that ended badly is nothing to act on here.
                let _: Result<(), _> = thread.join();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (revision, camera)
    type Key = (u64, i32);

    fn background() -> Scheduler<Key> {
        Scheduler::new(Mode::Background)
    }

    #[test]
    fn at_rest_a_preview_is_shown_at_once_and_the_sharp_image_follows() {
        let mut s = background();
        let key = (1, 0);
        let plan = s.plan(&key, false);
        assert!(plan.preview);
        let ticket = plan.sharp.unwrap();
        s.preview_shown(&key);
        assert_eq!(s.shown(), Some((&key, Level::Preview)));
        assert!(!s.is_sharp());
        // While it is being made nothing more is asked for.
        for _ in 0..3 {
            assert_eq!(
                s.plan(&key, false),
                Plan {
                    preview: false,
                    sharp: None
                }
            );
        }
        assert_eq!(s.pending(), Some(ticket));
        assert!(s.deliver(ticket));
        assert_eq!(s.shown(), Some((&key, Level::Sharp)));
        assert!(s.is_sharp());
        assert_eq!(s.pending(), None);
        // And nothing after it arrived.
        assert_eq!(
            s.plan(&key, false),
            Plan {
                preview: false,
                sharp: None
            }
        );
        // The same result a second time is not shown again.
        assert!(!s.deliver(ticket));
    }

    #[test]
    fn a_result_for_an_old_camera_is_never_shown() {
        let mut s = background();
        let old = (1, 0);
        let first = s.plan(&old, false).sharp.unwrap();
        s.preview_shown(&old);
        // The camera moves on before the image arrives.
        let new = (1, 5);
        let plan = s.plan(&new, false);
        assert!(plan.preview, "the new camera needs its own preview");
        let second = plan.sharp.unwrap();
        assert_ne!(first, second);
        s.preview_shown(&new);
        // The old image arrives late: dropped, the preview of the new
        // camera stays.
        assert!(!s.deliver(first));
        assert_eq!(s.shown(), Some((&new, Level::Preview)));
        assert_eq!(s.pending(), Some(second), "still waiting for the new one");
        assert!(s.deliver(second));
        assert_eq!(s.shown(), Some((&new, Level::Sharp)));
    }

    #[test]
    fn a_result_for_an_old_revision_is_never_shown() {
        let mut s = background();
        let old = (1, 0);
        let first = s.plan(&old, false).sharp.unwrap();
        s.preview_shown(&old);
        assert!(s.deliver(first));
        // The document changes; the image on screen is of the old one.
        let new = (2, 0);
        let plan = s.plan(&new, false);
        assert!(plan.preview && plan.sharp.is_some());
        assert!(
            !s.is_sharp(),
            "the sharp image on screen is not of this key"
        );
        s.preview_shown(&new);
        // Results out of order: a ticket that was never the newest.
        assert!(!s.deliver(first));
        assert_eq!(s.shown(), Some((&new, Level::Preview)));
    }

    #[test]
    fn a_result_that_arrives_while_the_camera_moved_away_is_dropped_and_asked_again() {
        let mut s = background();
        let rest = (1, 0);
        let ticket = s.plan(&rest, false).sharp.unwrap();
        s.preview_shown(&rest);
        // A drag begins: previews only, no new sharp image.
        for camera in 1..=3 {
            let key = (1, camera);
            assert_eq!(
                s.plan(&key, true),
                Plan {
                    preview: true,
                    sharp: None
                }
            );
            s.preview_shown(&key);
        }
        // The image of where the drag began arrives mid-drag.
        assert!(!s.deliver(ticket));
        assert_eq!(s.shown(), Some((&(1, 3), Level::Preview)));
        assert_eq!(s.pending(), None);
        // The drag ends where it is: the preview is of this camera
        // already, the sharp image is asked for.
        let plan = s.plan(&(1, 3), false);
        assert!(!plan.preview);
        let ticket = plan.sharp.unwrap();
        assert!(s.deliver(ticket));
        assert!(s.is_sharp());
    }

    #[test]
    fn coming_back_to_the_key_being_made_accepts_its_result() {
        let mut s = background();
        let a = (1, 0);
        let ticket = s.plan(&a, false).sharp.unwrap();
        s.preview_shown(&a);
        // Away while moving (nothing new is started) and back.
        assert!(s.plan(&(1, 1), true).preview);
        s.preview_shown(&(1, 1));
        let plan = s.plan(&a, true);
        assert_eq!(
            plan,
            Plan {
                preview: true,
                sharp: None
            }
        );
        s.preview_shown(&a);
        assert!(s.deliver(ticket), "it is the image of what is wanted now");
        assert_eq!(s.shown(), Some((&a, Level::Sharp)));
        // Holding still while "moving" keeps the sharp image.
        assert_eq!(
            s.plan(&a, true),
            Plan {
                preview: false,
                sharp: None
            }
        );
    }

    #[test]
    fn inline_there_is_no_preview_at_rest_and_one_while_moving() {
        let mut s: Scheduler<Key> = Scheduler::new(Mode::Inline);
        let key = (1, 0);
        let plan = s.plan(&key, false);
        assert!(!plan.preview);
        assert!(s.deliver(plan.sharp.unwrap()));
        assert!(s.is_sharp());
        let moved = (1, 1);
        assert_eq!(
            s.plan(&moved, true),
            Plan {
                preview: true,
                sharp: None
            }
        );
        s.preview_shown(&moved);
        let plan = s.plan(&moved, false);
        assert!(!plan.preview);
        assert!(s.deliver(plan.sharp.unwrap()));
    }

    #[test]
    fn a_failed_image_is_not_asked_for_every_frame() {
        let mut s = background();
        let key = (1, 0);
        let ticket = s.plan(&key, false).sharp.unwrap();
        s.preview_shown(&key);
        assert!(s.fail(ticket));
        for _ in 0..3 {
            assert_eq!(
                s.plan(&key, false),
                Plan {
                    preview: false,
                    sharp: None
                }
            );
        }
        // Something changed: asked again.
        let next = (2, 0);
        assert!(s.plan(&next, false).sharp.is_some());
        // A failure of an overtaken job is not news.
        let mut s = background();
        let old = s.plan(&(1, 0), false).sharp.unwrap();
        let new = s.plan(&(1, 1), false).sharp.unwrap();
        assert!(!s.fail(old));
        assert_eq!(s.pending(), Some(new));
    }

    #[test]
    fn forgetting_makes_everything_again() {
        let mut s = background();
        let key = (1, 0);
        let ticket = s.plan(&key, false).sharp.unwrap();
        s.preview_shown(&key);
        assert!(s.deliver(ticket));
        s.forget();
        assert_eq!(s.shown(), None);
        let plan = s.plan(&key, false);
        assert!(plan.preview && plan.sharp.is_some());
    }

    #[cfg(not(target_arch = "wasm32"))]
    mod worker {
        use super::super::*;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::mpsc;
        use std::sync::{Arc, Mutex, PoisonError};
        use std::time::Duration;

        const LONG: Duration = Duration::from_secs(60);

        fn tickets(count: usize) -> Vec<Ticket> {
            let mut s: Scheduler<usize> = Scheduler::new(Mode::Background);
            (0..count)
                .map(|k| s.plan(&k, false).sharp.unwrap())
                .collect()
        }

        #[test]
        fn the_worker_renders_a_job_and_says_so() {
            let ready = Arc::new(AtomicUsize::new(0));
            let counted = Arc::clone(&ready);
            let worker: Worker<u32, u32> = Worker::start(
                |job| Ok(job * 2),
                move |_| {
                    counted.fetch_add(1, Ordering::SeqCst);
                },
            )
            .unwrap();
            let t = tickets(1);
            assert!(worker.poll().is_none());
            assert!(worker.submit(t[0], 21).is_ok());
            let done = worker.wait(LONG).unwrap();
            assert_eq!((done.ticket, done.result), (t[0], Ok(42)));
            assert!(worker.alive());
            drop(worker);
            assert_eq!(ready.load(Ordering::SeqCst), 1);
        }

        /// The render of each job waits for a go-ahead, so the test
        /// decides what is in the queue when: no sleeps.
        #[test]
        fn jobs_overtaken_in_the_queue_are_skipped() {
            let (go, gate) = mpsc::channel::<()>();
            let gate = Mutex::new(gate);
            let (started, starts) = mpsc::channel::<u32>();
            let started = Mutex::new(started);
            let worker: Worker<u32, u32> = Worker::start(
                move |job| {
                    let _ = started
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .send(*job);
                    let _ = gate.lock().unwrap_or_else(PoisonError::into_inner).recv();
                    Ok(*job)
                },
                |_| {},
            )
            .unwrap();
            let t = tickets(4);
            assert!(worker.submit(t[0], 0).is_ok());
            // The thread is busy with job 0 from here on.
            assert_eq!(starts.recv_timeout(LONG), Ok(0));
            for (k, ticket) in t.iter().enumerate().skip(1) {
                assert!(worker.submit(*ticket, k as u32).is_ok());
            }
            go.send(()).unwrap();
            let done = worker.wait(LONG).unwrap();
            assert_eq!((done.ticket, done.result), (t[0], Ok(0)));
            // Of the three that queued up, only the newest is rendered.
            assert_eq!(starts.recv_timeout(LONG), Ok(3));
            go.send(()).unwrap();
            let done = worker.wait(LONG).unwrap();
            assert_eq!((done.ticket, done.result), (t[3], Ok(3)));
            assert!(worker.poll().is_none());
            // The scheduler would have shown only the last: the tickets
            // of the skipped jobs never come back, and that of job 0 is
            // stale.
            drop(go);
            drop(worker);
            assert!(starts.try_recv().is_err(), "jobs 1 and 2 never started");
        }

        #[test]
        fn a_panicking_render_is_an_error_and_the_next_job_still_runs() {
            let worker: Worker<u32, u32> = Worker::start(
                |job| {
                    assert!(*job != 13, "unlucky job");
                    if *job == 7 {
                        return Err("seven cannot be rendered".to_string());
                    }
                    Ok(*job)
                },
                |job| assert!(*job != 1, "a panic while waking is swallowed too"),
            )
            .unwrap();
            let t = tickets(4);
            assert!(worker.submit(t[0], 13).is_ok());
            let done = worker.wait(LONG).unwrap();
            assert_eq!(done.ticket, t[0]);
            let error = done.result.unwrap_err();
            assert!(
                error.contains("internal error") && error.contains("unlucky job"),
                "{error}"
            );
            assert!(worker.submit(t[1], 7).is_ok());
            let done = worker.wait(LONG).unwrap();
            assert_eq!(done.result, Err("seven cannot be rendered".to_string()));
            assert!(worker.submit(t[2], 1).is_ok());
            assert_eq!(worker.wait(LONG).unwrap().result, Ok(1));
            assert!(worker.submit(t[3], 2).is_ok());
            assert_eq!(worker.wait(LONG).unwrap().result, Ok(2));
            assert!(worker.alive());
        }

        #[test]
        fn waiting_for_nothing_times_out_and_dropping_joins_a_busy_thread() {
            let finished = Arc::new(AtomicUsize::new(0));
            let counted = Arc::clone(&finished);
            let (go, gate) = mpsc::channel::<()>();
            let gate = Mutex::new(gate);
            let worker: Worker<u32, u32> = Worker::start(
                move |job| {
                    let _ = gate.lock().unwrap_or_else(PoisonError::into_inner).recv();
                    counted.fetch_add(1, Ordering::SeqCst);
                    Ok(*job)
                },
                |_| {},
            )
            .unwrap();
            assert!(worker.wait(Duration::from_millis(1)).is_none());
            let t = tickets(1);
            assert!(worker.submit(t[0], 5).is_ok());
            // Dropping the gate's sender lets the render go on; dropping
            // the worker then waits for the thread to end.
            drop(go);
            drop(worker);
            assert_eq!(finished.load(Ordering::SeqCst), 1);
        }
    }
}
