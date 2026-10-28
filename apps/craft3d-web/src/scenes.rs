//! The page's regeneration and meshing in a Web Worker: the document the
//! page shows is regenerated and meshed by an instance of the page's own
//! module in a worker of its own (`packaging/web/worker.js`), so the
//! page's thread never builds a feature or meshes a body to show it, and
//! keeps drawing, turning the camera and taking edits while that takes
//! minutes (docs/design/engine.md › *Regeneration in a Web Worker*).
//!
//! [`Scenes`] decides, once per frame, what goes to the worker and what
//! comes back, as [`crate::checks::Checks`] does for the checks, through
//! the same [`Transport`]. The session hands out one job at a time
//! (`Session::scene_job`): the state shown and the quality, written out
//! (`Session::scene_request`) and posted. The worker answers part by
//! part (the quick first picture, then the display scene), each handed
//! to the session as it comes (`Session::scene_answered`), so the page
//! shows the first picture while the display scene is built; then that
//! the answer is complete (`Session::scene_done`). A worker that is done
//! is kept, with what it regenerated: the next state rebuilds only what
//! the edit changed. A job replaced while the worker works on it (an
//! edit, an undo, another tab) is cancelled by ending the worker, which
//! the next job starts again from nothing: a late answer never lands on
//! a newer state (the session takes parts only for the job that waits).
//!
//! A worker that cannot be had at all (no worker file, a browser or a
//! policy that refuses it) before it ever answered sends the work home:
//! the page regenerates and meshes a piece per frame, as it did before.
//! One whose module aborts while building (a panic: a bug) leaves the
//! scenes of that state errors that say so, and the next state goes to
//! a new worker: never built on the page instead, where the same bug
//! would end the page. One that takes longer than [`DEADLINE_MS`] over a
//! job is ended, and that state's scenes say so.

use crate::checks::{Reply, Transport};
use craft3d_engine::{SceneQuality, Session};

/// What [`Scenes::frame_at`] did, for the page's log and its tests.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    /// The job posted this frame.
    pub posted: Option<u64>,
    /// The parts taken this frame, by job (parts of replaced jobs are
    /// dropped and not counted).
    pub parts: Vec<u64>,
    /// The job cancelled this frame (replaced while it was built).
    pub cancelled: Option<u64>,
    /// The work was sent home this frame, and why.
    pub home: Option<String>,
    /// The job given up this frame: the worker took longer than
    /// [`DEADLINE_MS`] on it.
    pub timed_out: Option<u64>,
}

/// How long the worker may take over one state before it is ended and
/// that state's scenes are errors that say so, in milliseconds: half an
/// hour, far beyond what any model regenerates in, so only a worker that
/// hangs reaches it.
pub const DEADLINE_MS: f64 = 30.0 * 60.0 * 1000.0;

/// What the scenes of a state given up at the deadline say.
pub const TOO_LONG: &str = "regenerating it beside the page took longer than 30 minutes and was stopped; change a parameter or open the document again to regenerate it again";

pub struct Scenes<T: Transport> {
    transport: T,
    /// The job the worker is building.
    busy: Option<u64>,
    /// When it was posted ([`Scenes::frame_at`]'s clock, ms).
    posted_at: f64,
    /// The worker answered at least once: it can be had on this page.
    answered_once: bool,
    /// Regenerated on the page again (the worker could not be had).
    home: bool,
}

impl<T: Transport> Scenes<T> {
    /// Has `session` regenerate its documents elsewhere, through
    /// `transport` (in effect while its checks are measured elsewhere
    /// too: `Session::regen_elsewhere`).
    pub fn new(transport: T, session: &mut Session) -> Self {
        session.set_regen_elsewhere(true);
        Self {
            transport,
            busy: None,
            posted_at: 0.0,
            answered_once: false,
            home: false,
        }
    }

    /// True once the work was sent home: the page regenerates.
    pub fn home(&self) -> bool {
        self.home
    }

    /// The job the worker is building.
    pub fn busy(&self) -> Option<u64> {
        self.busy
    }

    /// Takes what came back (`replies`), gives up a job the worker has
    /// had longer than [`DEADLINE_MS`] at `now` (ms, any clock that does
    /// not go back), cancels a job that was replaced, and posts the job
    /// of the scene shown at `quality` that waits. Called once per frame,
    /// before the page decides what to draw.
    pub fn frame_at(
        &mut self,
        session: &mut Session,
        quality: SceneQuality,
        replies: Vec<Reply>,
        now: f64,
    ) -> Frame {
        let mut done = Frame::default();
        if self.home {
            return done;
        }
        for reply in replies {
            match reply {
                Reply::Part { token, bytes } => {
                    self.answered_once = true;
                    if session.scene_answered(token, Ok(&bytes)) {
                        done.parts.push(token);
                    }
                }
                Reply::Done { token } => {
                    self.answered_once = true;
                    if self.busy == Some(token) {
                        self.busy = None;
                    }
                    session.scene_done(token);
                }
                // The module aborted while building: a new worker for the
                // next state.
                Reply::Answer { token, answer } => {
                    if self.busy == Some(token) {
                        self.busy = None;
                    }
                    self.transport.stop();
                    let why = answer.err().unwrap_or_else(|| {
                        "the worker answered as the checking worker does".to_string()
                    });
                    if session.scene_answered(token, Err(&why)) {
                        done.parts.push(token);
                    }
                }
                Reply::Failed(why) => {
                    self.transport.stop();
                    let busy = self.busy.take();
                    if !self.answered_once {
                        // No worker on this page: regenerated here, as before.
                        self.go_home(session, &why, &mut done);
                        return done;
                    }
                    if let Some(token) = busy
                        && session.scene_answered(token, Err(&why))
                    {
                        done.parts.push(token);
                    }
                }
            }
        }
        if let Some(busy) = self.busy
            && now - self.posted_at > DEADLINE_MS
        {
            self.transport.stop();
            self.busy = None;
            done.timed_out = Some(busy);
            session.scene_failed(busy, TOO_LONG);
        }
        let wanted = session.scene_job(quality);
        if let Some(busy) = self.busy
            && wanted != Some(busy)
        {
            // Superseded: what it builds is for a state no longer shown.
            self.transport.stop();
            self.busy = None;
            done.cancelled = Some(busy);
        }
        if self.busy.is_none()
            && let Some(token) = wanted
        {
            match session.scene_request(token) {
                Ok(request) => match self.transport.post(token, &request.document, &request.work) {
                    Ok(()) => {
                        self.busy = Some(token);
                        self.posted_at = now;
                        done.posted = Some(token);
                    }
                    Err(why) => {
                        self.transport.stop();
                        if self.answered_once {
                            session.scene_answered(token, Err(&why));
                        } else {
                            self.go_home(session, &why, &mut done);
                        }
                    }
                },
                // The document cannot be written out (it breaks its own
                // rules, or is larger than a file may be): its scenes say
                // why, as its checks do.
                Err(why) => {
                    session.scene_failed(
                        token,
                        &format!("the document cannot be handed to the regenerating worker: {why}"),
                    );
                }
            }
        }
        done
    }

    fn go_home(&mut self, session: &mut Session, why: &str, done: &mut Frame) {
        self.home = true;
        self.busy = None;
        session.set_regen_elsewhere(false);
        done.home = Some(why.to_string());
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;
    use craft3d_engine::{Workspace, scene_begin, scene_bytes, scene_next};
    use serde_json::json;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// The jobs posted to the fake worker: token, document, work.
    type Posted = Rc<RefCell<Vec<(u64, Vec<u8>, String)>>>;

    /// A worker in the test: answers what it is posted with
    /// `scene_begin` and `scene_next` when told to, or fails as told.
    #[derive(Clone, Default)]
    struct Fake {
        posted: Posted,
        stops: Rc<RefCell<usize>>,
        refuse: Option<String>,
    }

    impl Transport for Fake {
        fn post(&mut self, token: u64, document: &[u8], work: &str) -> Result<(), String> {
            if let Some(why) = &self.refuse {
                return Err(why.clone());
            }
            self.posted
                .borrow_mut()
                .push((token, document.to_vec(), work.into()));
            Ok(())
        }
        fn stop(&mut self) {
            *self.stops.borrow_mut() += 1;
        }
    }

    impl Fake {
        /// The answers to everything posted so far, parts and ends.
        fn answers(&self) -> Vec<Reply> {
            let mut replies = Vec::new();
            for (token, document, work) in self.posted.borrow_mut().drain(..) {
                scene_begin(&document, &work);
                while let Some(bytes) = scene_next() {
                    replies.push(Reply::Part { token, bytes });
                }
                replies.push(Reply::Done { token });
            }
            replies
        }
    }

    fn quality() -> SceneQuality {
        SceneQuality::display()
    }

    fn session() -> Session {
        let mut s = Session::new(Workspace::memory());
        s.set_stepwise(true);
        s.set_checks_elsewhere(true);
        for (id, params) in [
            ("file.new", json!({"units": "mm"})),
            ("variable.set", json!({"name": "w", "expr": "10 mm"})),
            (
                "feature.block",
                json!({"size": ["w", "w", "w"], "name": "B"}),
            ),
            ("variable.set", json!({"name": "w", "expr": "11 mm"})),
        ] {
            s.execute(id, &params).unwrap();
        }
        s
    }

    /// The scene a session that builds it itself shows for `w`.
    fn wanted(w: f64) -> Vec<u8> {
        let mut s = Session::new(Workspace::memory());
        for (id, params) in [
            ("file.new", json!({"units": "mm"})),
            ("variable.set", json!({"name": "w", "expr": "10 mm"})),
            (
                "feature.block",
                json!({"size": ["w", "w", "w"], "name": "B"}),
            ),
            (
                "variable.set",
                json!({"name": "w", "expr": format!("{w} mm")}),
            ),
        ] {
            s.execute(id, &params).unwrap();
        }
        scene_bytes(&s.scene_with(quality()).unwrap().unwrap())
    }

    fn shown(s: &mut Session) -> Vec<u8> {
        assert!(s.scene_ready(quality()));
        scene_bytes(&s.scene_with(quality()).unwrap().unwrap())
    }

    #[test]
    fn scenes_go_to_the_worker_and_come_back() {
        let mut s = session();
        let fake = Fake::default();
        let mut scenes = Scenes::new(fake.clone(), &mut s);
        assert!(s.regen_elsewhere());
        let token = scenes
            .frame_at(&mut s, quality(), Vec::new(), 0.0)
            .posted
            .unwrap();
        // Waiting: nothing posted twice, nothing built here.
        assert_eq!(
            scenes.frame_at(&mut s, quality(), Vec::new(), 1.0).posted,
            None
        );
        assert!(!s.scene_ready(quality()));
        let back = scenes.frame_at(&mut s, quality(), fake.answers(), 2.0);
        assert_eq!(back.parts, vec![token, token]);
        assert_eq!(scenes.busy(), None);
        assert_eq!(shown(&mut s), wanted(11.0));
        assert!(!s.regenerated_here());
        // Done: the worker is kept for the next state.
        assert_eq!(*fake.stops.borrow(), 0);
        s.execute("variable.set", &json!({"name": "w", "expr": "12 mm"}))
            .unwrap();
        let next = scenes
            .frame_at(&mut s, quality(), Vec::new(), 3.0)
            .posted
            .unwrap();
        scenes.frame_at(&mut s, quality(), fake.answers(), 4.0);
        assert_ne!(next, token);
        assert_eq!(*fake.stops.borrow(), 0);
        assert_eq!(shown(&mut s), wanted(12.0));
    }

    #[test]
    fn an_edit_cancels_the_job_the_worker_builds() {
        let mut s = session();
        let fake = Fake::default();
        let mut scenes = Scenes::new(fake.clone(), &mut s);
        let old = scenes
            .frame_at(&mut s, quality(), Vec::new(), 0.0)
            .posted
            .unwrap();
        let late = fake.answers();
        s.execute("variable.set", &json!({"name": "w", "expr": "13 mm"}))
            .unwrap();
        // The old job is no longer wanted: cancelled, the new one posted.
        let frame = scenes.frame_at(&mut s, quality(), Vec::new(), 1.0);
        assert_eq!(frame.cancelled, Some(old));
        assert_eq!(*fake.stops.borrow(), 1);
        let new = frame.posted.unwrap();
        assert_ne!(new, old);
        // The old answer arrives late: dropped, nothing shown of it.
        assert!(
            scenes
                .frame_at(&mut s, quality(), late, 2.0)
                .parts
                .is_empty()
        );
        assert!(!s.scene_ready(quality()));
        assert_eq!(scenes.busy(), Some(new));
        scenes.frame_at(&mut s, quality(), fake.answers(), 3.0);
        assert_eq!(shown(&mut s), wanted(13.0));
    }

    #[test]
    fn a_worker_that_hangs_is_ended_at_the_deadline() {
        let mut s = session();
        let fake = Fake::default();
        let mut scenes = Scenes::new(fake.clone(), &mut s);
        let token = scenes
            .frame_at(&mut s, quality(), Vec::new(), 1000.0)
            .posted
            .unwrap();
        let frame = scenes.frame_at(&mut s, quality(), Vec::new(), 1000.0 + DEADLINE_MS);
        assert_eq!(frame.timed_out, None);
        let frame = scenes.frame_at(&mut s, quality(), Vec::new(), 1001.0 + DEADLINE_MS);
        assert_eq!(frame.timed_out, Some(token));
        assert_eq!(*fake.stops.borrow(), 1);
        let error = s.scene_with(quality()).unwrap_err();
        assert_eq!(error.message(), format!("not regenerated: {TOO_LONG}"));
        // Not posted again for that state; the next state goes on.
        assert_eq!(frame.posted, None);
        assert!(
            scenes
                .frame_at(&mut s, quality(), fake.answers(), 2e7)
                .parts
                .is_empty()
        );
        s.execute("variable.set", &json!({"name": "w", "expr": "15 mm"}))
            .unwrap();
        assert!(
            scenes
                .frame_at(&mut s, quality(), Vec::new(), 2e7)
                .posted
                .is_some()
        );
        scenes.frame_at(&mut s, quality(), fake.answers(), 2e7);
        assert_eq!(shown(&mut s), wanted(15.0));
    }

    #[test]
    fn a_worker_that_stopped_while_building_leaves_an_error() {
        let mut s = session();
        let fake = Fake::default();
        let mut scenes = Scenes::new(fake.clone(), &mut s);
        // It answered once before: it can be had here.
        scenes.frame_at(&mut s, quality(), Vec::new(), 0.0);
        scenes.frame_at(&mut s, quality(), fake.answers(), 1.0);
        s.execute("variable.set", &json!({"name": "w", "expr": "14 mm"}))
            .unwrap();
        let token = scenes
            .frame_at(&mut s, quality(), Vec::new(), 2.0)
            .posted
            .unwrap();
        let frame = scenes.frame_at(
            &mut s,
            quality(),
            vec![Reply::Answer {
                token,
                answer: Err("RuntimeError: unreachable".into()),
            }],
            3.0,
        );
        assert_eq!(frame.parts, vec![token]);
        assert_eq!(*fake.stops.borrow(), 1);
        let error = s.scene_with(quality()).unwrap_err();
        assert!(error.message().contains("unreachable"), "{error}");
        assert!(!scenes.home() && !s.regenerated_here());
        // The worker failing outright after it answered: the same.
        s.execute("variable.set", &json!({"name": "w", "expr": "16 mm"}))
            .unwrap();
        scenes.frame_at(&mut s, quality(), Vec::new(), 4.0);
        let frame = scenes.frame_at(&mut s, quality(), vec![Reply::Failed("oom".into())], 5.0);
        assert!(frame.home.is_none());
        assert!(s.scene_with(quality()).is_err());
    }

    #[test]
    fn a_page_without_a_worker_regenerates_itself() {
        for refuse in [true, false] {
            let mut s = session();
            let fake = Fake {
                refuse: refuse.then(|| "SecurityError".to_string()),
                ..Fake::default()
            };
            let mut scenes = Scenes::new(fake.clone(), &mut s);
            let frame = if refuse {
                scenes.frame_at(&mut s, quality(), Vec::new(), 0.0)
            } else {
                scenes.frame_at(&mut s, quality(), Vec::new(), 0.0);
                scenes.frame_at(&mut s, quality(), vec![Reply::Failed("404".into())], 1.0)
            };
            assert!(frame.home.is_some(), "{frame:?}");
            assert!(scenes.home() && !s.regen_elsewhere());
            // Built on the page, a piece per frame, as before.
            while s.prepare_some(quality(), || false).is_some() {}
            assert_eq!(shown(&mut s), wanted(11.0));
            assert_eq!(
                scenes.frame_at(&mut s, quality(), Vec::new(), 2.0),
                Frame::default()
            );
        }
    }
}
