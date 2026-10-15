//! Coarse first, then sharp: the display scene of the active document is
//! built on a thread of its own while the window shows the quick first
//! picture, and while the session goes on running commands on the engine
//! thread (docs/design/engine.md).
//!
//! The engine thread builds the first picture as part of a task, as it
//! built the display scene before; this module takes it from there. A
//! refinement holds the document state and its regeneration, both shared,
//! so it needs no session: a command that arrives meanwhile is not held up
//! behind it (the two share the processor, nothing more). Its result is
//! handed to the session when the session is home, and kept for the state
//! it was built for.
//!
//! One refinement is wanted at a time, the active document's. One that is
//! not wanted any more (an edit, another tab) is cancelled: it stops
//! before its next body or attempt, and a new one starts at once beside
//! it, up to [`MAX_DRAINING`] cancelled ones still running.

use crate::control::Wake;
use craft3d_engine::{Refined, Refinement, SceneQuality, Session};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};

/// Most cancelled refinements still running before a new one waits for
/// them: each holds a share of the processor until its body is done.
const MAX_DRAINING: usize = 2;

/// A refinement on its thread.
struct Running {
    id: u64,
    job: Arc<Refinement>,
    cancel: Arc<AtomicBool>,
}

/// The refinements of the window's session.
pub struct Refiner {
    wake: Wake,
    /// The active document's, while it runs.
    current: Option<Running>,
    /// Cancelled ones that have not finished yet.
    draining: Vec<Running>,
    next_id: u64,
    done: Receiver<(u64, Refined)>,
    send: Sender<(u64, Refined)>,
    /// How long each refinement waits before it starts: a slow model, for
    /// the tests.
    #[cfg(test)]
    pub(crate) delay: std::time::Duration,
}

impl Refiner {
    /// `wake` is called when a refinement is done.
    pub fn new(wake: Wake) -> Self {
        let (send, done) = mpsc::channel();
        Self {
            wake,
            current: None,
            draining: Vec::new(),
            next_id: 0,
            done,
            send,
            #[cfg(test)]
            delay: std::time::Duration::ZERO,
        }
    }

    /// Hands what is done to `session`, cancels what it no longer wants
    /// and starts what it wants now: [`Poll::Changed`] when the active
    /// document's scene is sharper now, [`Poll::CannotStart`] when no
    /// thread could be started (the caller then builds the display scene
    /// elsewhere). For a session that is home, never one that is away.
    pub fn poll(&mut self, session: &mut Session, quality: SceneQuality) -> Poll {
        let mut changed = false;
        while let Ok((id, refined)) = self.done.try_recv() {
            if self.current.as_ref().is_some_and(|r| r.id == id) {
                self.current = None;
            }
            self.draining.retain(|r| r.id != id);
            changed |= session.accept_refined(refined);
        }
        if let Some(current) = &self.current
            && !current.job.wanted(session)
        {
            current.cancel.store(true, Ordering::Relaxed);
            if let Some(current) = self.current.take() {
                self.draining.push(current);
            }
        }
        if self.current.is_none()
            && self.draining.len() < MAX_DRAINING
            && let Some(job) = session.refinement(quality)
            && !self.start(job)
        {
            return Poll::CannotStart;
        }
        if changed { Poll::Changed } else { Poll::Same }
    }

    /// True while the active document's display scene is being built
    /// here, or is wanted and waits for a cancelled one to finish.
    pub fn refining(&self, session: &mut Session, quality: SceneQuality) -> bool {
        self.current.is_some() || session.refinement(quality).is_some()
    }

    fn start(&mut self, job: Refinement) -> bool {
        let (id, job, cancel) = (
            self.next_id,
            Arc::new(job),
            Arc::new(AtomicBool::new(false)),
        );
        self.next_id = self.next_id.wrapping_add(1);
        let (for_thread, cancel_there, send, wake) = (
            Arc::clone(&job),
            Arc::clone(&cancel),
            self.send.clone(),
            Arc::clone(&self.wake),
        );
        #[cfg(test)]
        let delay = self.delay;
        let spawned = std::thread::Builder::new()
            .name("craft3d-refine".to_string())
            .spawn(move || {
                #[cfg(test)]
                std::thread::sleep(delay);
                // `run` catches a panic of its own; a window that is gone
                // takes no result.
                let refined = for_thread.run(&cancel_there);
                if send.send((id, refined)).is_ok() {
                    wake();
                }
            });
        if spawned.is_err() {
            return false;
        }
        self.current = Some(Running { id, job, cancel });
        true
    }
}

/// What [`Refiner::poll`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Poll {
    /// The active document's scene is sharper now.
    Changed,
    Same,
    /// No thread could be started for a refinement.
    CannotStart,
}

impl Drop for Refiner {
    /// Cancels whatever still runs; its threads end at their next body.
    fn drop(&mut self) {
        for running in self.current.iter().chain(&self.draining) {
            running.cancel.store(true, Ordering::Relaxed);
        }
    }
}
