//! The long work behind showing a document, a piece at a time, for a
//! host with one thread (the page in a browser): regenerating, the quick
//! first picture, the display mesh and the assertions, each stopped
//! between features, bodies or assertions when the host's frame is used
//! up, and taken up again in the next frame where it stopped. What is
//! built is what [`Session::scene_with`] and the assertions build in one
//! go (docs/design/engine.md › *A piece at a time*).

use crate::{SceneQuality, Session};
use craft3d_regen::progress::Progress;
use craft3d_regen::{Need, Status};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

impl Session {
    /// Says that the host does the long work a piece at a time with
    /// [`Self::prepare_some`]: assertions are then measured by it (or
    /// all at once when a command waits for them), never on a thread of
    /// their own; and the display mesh is built by it
    /// ([`Self::set_refine_elsewhere`]).
    pub fn set_stepwise(&mut self, stepwise: bool) {
        self.stepwise = stepwise;
        self.set_refine_elsewhere(stepwise);
    }

    /// True when the host does the long work a piece at a time
    /// ([`Self::set_stepwise`]) and the active document's current state
    /// is not regenerated yet: an edit then leaves its regeneration to
    /// [`Self::prepare_some`], which the host does between its frames,
    /// instead of doing all of it in the command (which held the page's
    /// one thread for as long as the whole regeneration took). The edit's
    /// result then says `deferred: true` and leaves out what only the
    /// regeneration knows (the features that fail now, the assertions'
    /// summary); the window shows both once they are there.
    pub(crate) fn regeneration_deferred(&self) -> bool {
        self.stepwise
            && self.documents.get(self.active).is_some_and(|open| {
                open.regen
                    .as_ref()
                    .is_none_or(|(at, _)| *at != open.history.revision())
            })
    }

    /// Does a piece of what showing the active document at `quality`
    /// still needs, in this order: its regeneration, the quick first
    /// picture, the display scene (for [`SceneQuality::Display`]) and its
    /// assertions. `stop` is asked after each feature built, each body
    /// meshed and each assertion measured (at least one is done per
    /// call); a host passes one that says when its frame is used up.
    ///
    /// `None` when nothing is left. Otherwise what is being done, for
    /// the host to show: `regenerating` with the features built so far of
    /// all and the next one's name, `meshing` (the first picture, which
    /// is not there yet), `refining` (it is there, the display mesh is
    /// not), `checking` with the assertions measured so far of all.
    ///
    /// Never panics: a panic in the work (a bug) is caught, kept as the
    /// error of the active document's scenes (which a viewer then shows)
    /// and nothing more is done for that state of the document, so the
    /// host does not ask again every frame.
    pub fn prepare_some(
        &mut self,
        quality: SceneQuality,
        stop: impl Fn() -> bool + Clone + 'static,
    ) -> Option<Progress> {
        let state = self
            .documents
            .get(self.active)
            .map(|open| (open.id, open.history.revision()));
        if state.is_some() && self.prepare_broken == state {
            return None;
        }
        let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.prepare_unguarded(quality, stop)
        }));
        match done {
            Ok(progress) => progress,
            Err(payload) => {
                self.prepare_broken = state;
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    self.fail_scenes(quality, payload.as_ref());
                }));
                None
            }
        }
    }

    fn prepare_unguarded(
        &mut self,
        quality: SceneQuality,
        stop: impl Fn() -> bool + Clone + 'static,
    ) -> Option<Progress> {
        #[cfg(test)]
        if tests::PANIC.with(std::cell::Cell::take) {
            panic!("a bug in the work, for the test");
        }
        // Regenerated and meshed elsewhere (`regen_elsewhere.rs`): the
        // host hands the scenes' jobs over, and the checks go with them.
        if self.regen_elsewhere() {
            return None;
        }
        if let Some(progress) = self.regenerate_some(stop.clone()) {
            return Some(progress);
        }
        let revision = self.scene_revision();
        let (stop_past, seen) = self.past_reached(revision, "meshing", stop.clone());
        let preview = self.preview_some(stop_past);
        if !preview {
            self.reached = Some((revision, "meshing", seen.get()));
            return Some(phase("meshing"));
        }
        let (stop_past, seen) = self.past_reached(revision, "refining", stop.clone());
        if self.refine_some(quality, stop_past) {
            self.reached = Some((revision, "refining", seen.get()));
            return Some(phase("refining"));
        }
        let (done, of) = self.assertions_some(stop)?;
        Some(Progress {
            phase: "checking",
            feature: None,
            name: None,
            step: done,
            steps: of,
        })
    }

    /// `stop`, asked only past the bodies the last call that stopped in
    /// `phase` of this scene revision got to, so that each call meshes at
    /// least one body more (the bodies meshed before come from the store
    /// of meshes, reported again on the way); and how many were reported.
    fn past_reached(
        &self,
        revision: u64,
        phase: &'static str,
        stop: impl Fn() -> bool + 'static,
    ) -> (impl Fn() -> bool + 'static, Rc<Cell<usize>>) {
        let reached = match self.reached {
            Some((at, was, n)) if at == revision && was == phase => n,
            _ => 0,
        };
        let seen = Rc::new(Cell::new(0_usize));
        let count = Rc::clone(&seen);
        let past = move || {
            let n = count.get().saturating_add(1);
            count.set(n);
            n > reached && stop()
        };
        (past, seen)
    }

    /// The regeneration of the active document, a piece of it: `None`
    /// once it is there. A document scrubbed to an earlier state shows
    /// that state, which is regenerated in one go.
    fn regenerate_some(&mut self, stop: impl Fn() -> bool + 'static) -> Option<Progress> {
        let open = self.documents.get_mut(self.active)?;
        let revision = open.history.revision();
        if open.regen.as_ref().is_some_and(|(at, _)| *at == revision) {
            return None;
        }
        let doc = Arc::clone(open.history.current());
        open.work.regenerated();
        let cache = &mut open.cache;
        let (regen, stopped) = craft3d_regen::progress::with_stop(stop, || {
            craft3d_regen::regenerate_cached_needing(&doc, cache, Need::Bodies)
        });
        if !stopped {
            open.regen = Some((revision, Arc::new(regen)));
            return None;
        }
        // Built so far: up to the first feature it stopped before (one the
        // rollback marker leaves out does not count as stopped at).
        let done = regen
            .features
            .iter()
            .zip(&doc.features)
            .take_while(|(r, _)| r.status != Status::RolledBack)
            .count();
        let next = doc.features.get(done);
        Some(Progress {
            phase: "regenerating",
            feature: next.map(|f| f.id.to_string()),
            name: next.map(|f| f.name.clone()),
            step: done,
            steps: doc.features.len(),
        })
    }

    /// The display scene of the active document, built here a piece at a
    /// time when it is built elsewhere than where it is asked for
    /// ([`Self::set_refine_elsewhere`]): true while it is not done.
    fn refine_some(&mut self, quality: SceneQuality, stop: impl Fn() -> bool + 'static) -> bool {
        if !self.scene.refine_elsewhere {
            return false;
        }
        let Some(job) = self.refinement(quality) else {
            return false;
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let for_sink = Arc::clone(&cancel);
        let refined = craft3d_regen::progress::with_sink(
            move |_| {
                if stop() {
                    for_sink.store(true, Ordering::Relaxed);
                }
            },
            || job.run(&cancel),
        );
        if refined.cancelled() {
            return true;
        }
        self.accept_refined(refined);
        false
    }
}

fn phase(phase: &'static str) -> Progress {
    Progress {
        phase,
        feature: None,
        name: None,
        step: 0,
        steps: 0,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;
    use crate::Workspace;
    use serde_json::json;

    thread_local! {
        /// The next piece of work panics (once).
        pub(super) static PANIC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    /// A panic in the work is caught: the host is told nothing is left
    /// (it stops asking), the scenes say what went wrong, and an edit
    /// makes the work go on again.
    #[test]
    fn a_panic_in_the_work_stops_it_for_that_state_and_shows_the_error() {
        let mut session = Session::new(Workspace::none());
        session.set_stepwise(true);
        for (id, params) in [
            ("file.new", json!({"units": "mm"})),
            ("feature.block", json!({"size": [10, 10, 10]})),
        ] {
            session.execute(id, &params).unwrap();
        }
        PANIC.with(|p| p.set(true));
        let quality = SceneQuality::display();
        assert_eq!(session.prepare_some(quality, || true), None);
        // Not again for this state, and not looping.
        assert_eq!(session.prepare_some(quality, || true), None);
        assert!(session.scene_ready(quality));
        let shown = session.scene_with(quality);
        let error = shown
            .err()
            .map(|e| e.message().to_string())
            .unwrap_or_default();
        assert!(error.contains("internal error"), "{error}");
        // An edit is another state: the work goes on, and ends.
        session
            .execute(
                "feature.block",
                &json!({"size": [5, 5, 5], "position": [20, 0, 0]}),
            )
            .unwrap();
        let mut calls = 0;
        while session.prepare_some(quality, || false).is_some() {
            calls += 1;
            assert!(calls < 10);
        }
        let scene = session.scene_with(quality).unwrap().unwrap();
        assert_eq!(scene.bodies.len(), 2);
    }
}
