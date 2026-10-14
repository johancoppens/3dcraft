//! The engine thread: where the window's session does its work, so the
//! thread that draws the window never waits for a command.
//!
//! The session is handed over whole, with one task, and handed back with
//! the task's outcome once the task is done and the scene the window
//! shows has been built (regeneration and meshing are both done here).
//! While it is away the window draws what it showed last and the work in
//! progress ([`Shell::ui_away`](craft3d_ui_egui::Shell::ui_away)): the
//! document shown is never one half-way through an edit, because the
//! window has no other copy of it.
//!
//! One task at a time, in the order they were given: what the window and
//! its control channel ask for stays in order.
//!
//! What the task is doing is reported as it goes: the steps regeneration
//! and meshing report on this thread (`craft3d_regen::progress`) are kept
//! where the window reads them, and each one wakes it.
//!
//! A panic in a task is caught here, the last good state of every
//! document with unsaved changes is autosaved at once, and the session
//! comes back as the task left it (a command's edits are atomic, so that
//! is the state before it).

use crate::control::Wake;
use craft3d_engine::{SceneQuality, Session};
use craft3d_regen::progress::{self, Progress};
use craft3d_ui_egui::Work;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// What a task does with the session. Its answer is passed back as it is.
pub type Run = Box<dyn FnOnce(&mut Session) -> Outcome + Send>;

/// What a task gave back.
#[derive(Debug)]
pub enum Outcome {
    /// The reply line of a request (`None`: a line that gets none).
    Reply(Option<String>),
    /// What a command run for the window itself did.
    Ran(Result<serde_json::Value, String>),
    /// Nothing to say: the scene was built.
    Built,
    /// The task panicked (a bug): the session is as it left it.
    Panicked,
}

/// One task: what it is called in the window, and what it does.
pub struct Task {
    pub label: String,
    pub run: Run,
    /// The quality of the scene the window will show, built before the
    /// session comes back.
    pub quality: SceneQuality,
}

/// A task that is done, with the session back.
pub struct Finished {
    pub session: Session,
    pub outcome: Outcome,
}

/// The work in progress, as the window reads it.
#[derive(Default)]
struct Now {
    label: String,
    started: Option<Instant>,
    step: Option<Progress>,
}

/// The thread, and the way to and from it.
pub struct Worker {
    tasks: Option<Sender<(Session, Task)>>,
    finished: Receiver<Finished>,
    now: Arc<Mutex<Now>>,
    thread: Option<JoinHandle<()>>,
}

fn lock(now: &Mutex<Now>) -> std::sync::MutexGuard<'_, Now> {
    now.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Runs one task on the session, guarded: a panic becomes
/// [`Outcome::Panicked`] after an autosave.
fn perform(session: &mut Session, task: Task) -> Outcome {
    let Task { run, quality, .. } = task;
    let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(session)));
    let outcome = ran.unwrap_or_else(|_| {
        // Not worth a second panic.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            session.autosave_now();
        }));
        Outcome::Panicked
    });
    // The scene the window asks for next, built here and not in its frame.
    // `scene_with` guards its own work; this guard is for what it does
    // not cover.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = session.scene_with(quality);
    }));
    outcome
}

impl Worker {
    /// Starts the thread. `wake` is called whenever there is something new
    /// to show: a step, or a finished task.
    pub fn start(wake: Wake) -> Result<Self, String> {
        let (tasks, incoming) = mpsc::channel::<(Session, Task)>();
        let (done, finished) = mpsc::channel();
        let now = Arc::new(Mutex::new(Now::default()));
        let for_thread = Arc::clone(&now);
        let thread = std::thread::Builder::new()
            .name("craft3d-engine".to_string())
            .spawn(move || {
                while let Ok((mut session, task)) = incoming.recv() {
                    {
                        let mut now = lock(&for_thread);
                        now.label.clone_from(&task.label);
                        now.started = Some(Instant::now());
                        now.step = None;
                    }
                    wake();
                    let (for_sink, wake_sink) = (Arc::clone(&for_thread), Arc::clone(&wake));
                    let outcome = progress::with_sink(
                        move |step: &Progress| {
                            lock(&for_sink).step = Some(step.clone());
                            wake_sink();
                        },
                        || perform(&mut session, task),
                    );
                    *lock(&for_thread) = Now::default();
                    // A window that is gone takes no answers; the session
                    // ends here with the thread.
                    if done.send(Finished { session, outcome }).is_err() {
                        break;
                    }
                    wake();
                }
            })
            .map_err(|e| format!("the engine thread could not be started: {e}"))?;
        Ok(Self {
            tasks: Some(tasks),
            finished,
            now,
            thread: Some(thread),
        })
    }

    /// Hands the session over with a task. When the thread is gone, both
    /// come back, to be run where the caller is.
    pub fn submit(&self, session: Session, task: Task) -> Result<(), Box<(Session, Task)>> {
        let Some(tasks) = &self.tasks else {
            return Err(Box::new((session, task)));
        };
        tasks
            .send((session, task))
            .map_err(|mpsc::SendError(back)| Box::new(back))
    }

    /// The task that finished, if one did.
    pub fn try_finished(&self) -> Option<Finished> {
        self.finished.try_recv().ok()
    }

    /// Waits at most `timeout` for the task to finish.
    pub fn wait_finished(&self, timeout: Duration) -> Option<Finished> {
        self.finished.recv_timeout(timeout).ok()
    }

    /// What the thread is doing now, for the window; `None` when idle.
    pub fn work(&self) -> Option<Work> {
        let now = lock(&self.now);
        let started = now.started?;
        let seconds = started.elapsed().as_secs_f64();
        Some(match &now.step {
            Some(step) => Work {
                label: now.label.clone(),
                phase: step.phase.to_string(),
                feature: step.feature.clone(),
                name: step.name.clone(),
                step: step.step,
                steps: step.steps,
                seconds,
            },
            None => Work {
                label: now.label.clone(),
                phase: "running".to_string(),
                seconds,
                ..Work::default()
            },
        })
    }

    /// Runs a task on this thread, the way the engine thread would: where
    /// no engine thread could be started.
    pub fn run_here(session: &mut Session, task: Task) -> Outcome {
        perform(session, task)
    }
}

impl Drop for Worker {
    /// Closes the way in, so the thread ends after the task it is on, and
    /// does not wait for it: a window that closes while work runs closes
    /// at once, and the process ends with it.
    fn drop(&mut self) {
        self.tasks = None;
        drop(self.thread.take());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_engine::Workspace;
    use serde_json::json;

    fn task(label: &str, run: Run) -> Task {
        Task {
            label: label.to_string(),
            run,
            quality: SceneQuality::display(),
        }
    }

    /// A panic on the engine thread is caught there: the session comes back
    /// with its document as it was, and the thread takes the next task.
    #[test]
    fn a_panic_on_the_engine_thread_keeps_the_session_and_the_thread() {
        let worker = Worker::start(Arc::new(|| {})).unwrap();
        let mut session = Session::new(Workspace::none());
        session.execute("file.new", &json!({})).unwrap();
        session
            .execute("feature.block", &json!({"size": [1, 2, 3]}))
            .unwrap();
        let revision = session.document_revision();
        let fall_over: Run = Box::new(|_| panic!("on purpose, on the engine thread"));
        assert!(worker.submit(session, task("fall over", fall_over)).is_ok());
        let finished = worker.wait_finished(Duration::from_secs(30)).unwrap();
        assert!(matches!(finished.outcome, Outcome::Panicked));
        let mut session = finished.session;
        assert_eq!(session.document_revision(), revision);
        // The scene was built after all, on the engine thread.
        assert!(session.scene_ready(SceneQuality::display()));
        assert!(worker.work().is_none());
        let next: Run = Box::new(|session| {
            Outcome::Ran(
                session
                    .execute("document.measure", &json!({}))
                    .map_err(|e| e.to_string()),
            )
        });
        assert!(worker.submit(session, task("measure", next)).is_ok());
        let finished = worker.wait_finished(Duration::from_secs(30)).unwrap();
        let Outcome::Ran(Ok(measured)) = finished.outcome else {
            panic!("the next task ran");
        };
        assert!(
            (measured["volume"].as_f64().unwrap() - 6.0).abs() < 1e-9,
            "{measured}"
        );
    }

    /// The steps a task reports are what the window reads, while it runs.
    #[test]
    fn the_steps_of_a_task_are_readable_while_it_runs() {
        let woken = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let for_wake = Arc::clone(&woken);
        let worker = Worker::start(Arc::new(move || {
            for_wake.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }))
        .unwrap();
        let (go, wait) = mpsc::channel::<()>();
        let run: Run = Box::new(move |_| {
            progress::report(&Progress {
                phase: "regenerating",
                feature: Some("F2".to_string()),
                name: Some("J1".to_string()),
                step: 2,
                steps: 5,
            });
            let _ = wait.recv_timeout(Duration::from_secs(30));
            Outcome::Built
        });
        assert!(
            worker
                .submit(Session::new(Workspace::none()), task("file.open", run))
                .is_ok()
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        let work = loop {
            if let Some(work) = worker.work().filter(|w| w.step == 2) {
                break work;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        };
        assert_eq!(work.label, "file.open");
        assert_eq!(work.describe(), "file.open: regenerating J1 (2 of 5)");
        go.send(()).unwrap();
        assert!(worker.wait_finished(Duration::from_secs(30)).is_some());
        // Woken at the start, at the step and when done (that last one
        // just after the answer was sent).
        let deadline = Instant::now() + Duration::from_secs(10);
        while woken.load(std::sync::atomic::Ordering::SeqCst) < 3 {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}
