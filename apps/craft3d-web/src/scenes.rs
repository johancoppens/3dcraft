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
//! the edit changed. The worker builds a piece at a time (a feature, a
//! body) and looks at its messages between pieces, so a job replaced
//! while it works on it (an edit, an undo, another tab) is left for the
//! one posted next, or for none (`Transport::cancel`), without ending
//! the worker: what it built stays, and a slider dragged over a heavy
//! model rebuilds only what each value changes. A late answer never
//! lands on a newer state (the session takes parts only for the job
//! that waits).
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
use craft3d_engine::{Answered, SceneQuality, Session};

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
    /// The commands run where the bodies are that were answered this
    /// frame (`Session::command_answered`), for the host to report.
    pub commands: Vec<Answered>,
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
    /// Since when ([`Scenes::frame_at`]'s clock, ms) commands posted to
    /// the worker wait for their answers: one that hangs is given up at
    /// [`DEADLINE_MS`], as a scene is.
    commands_since: Option<f64>,
}

/// What a command waiting for the worker says when the worker was ended
/// because an edit was cancelled.
pub const RESTARTED: &str =
    "the worker beside the page was ended when an edit was cancelled; run it again";

/// What a command run where the bodies are says when the worker took
/// longer than [`DEADLINE_MS`] over it.
pub const COMMAND_TOO_LONG: &str =
    "running it beside the page took longer than 30 minutes and was stopped; run it again";

impl<T: Transport> Scenes<T> {
    /// Has `session` regenerate its documents elsewhere, through
    /// `transport` (in effect while its checks are measured elsewhere
    /// too: `Session::regen_elsewhere`).
    pub fn new(transport: T, session: &mut Session) -> Self {
        session.set_regen_elsewhere(true);
        // The commands that need the bodies go to the same worker, which
        // has them (`commands_elsewhere.rs` in the engine).
        session.set_commands_elsewhere(true);
        Self {
            transport,
            busy: None,
            posted_at: 0.0,
            answered_once: false,
            home: false,
            commands_since: None,
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
                Reply::Command { token, answer } => {
                    self.answered_once = true;
                    // It answers: the next answer is waited for from now.
                    if self.commands_since.is_some() {
                        self.commands_since = Some(now);
                    }
                    done.commands
                        .extend(session.command_answered(token, answer));
                }
                // The module aborted while building: a new worker for the
                // next state.
                Reply::Answer { token, answer } => {
                    // Whichever job it was on, the worker is gone: the job
                    // it was given since goes to the next one.
                    self.busy = None;
                    self.transport.stop();
                    let why = answer.err().unwrap_or_else(|| {
                        "the worker answered as the checking worker does".to_string()
                    });
                    done.commands.extend(session.commands_failed(&format!(
                        "internal error (a bug in 3DCraft, please report it): the worker running it beside the page stopped: {why}"
                    )));
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
                    done.commands.extend(session.commands_failed(&format!(
                        "the worker running it beside the page stopped: {why}"
                    )));
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
            done.commands.extend(session.commands_failed(TOO_LONG));
        }
        // A command the worker never answers (it hangs): given up, its
        // job and every other waiting one errors that say so, and the
        // worker ended (the next job starts a new one).
        if !session.commands_waiting() {
            self.commands_since = None;
        } else if let Some(since) = self.commands_since
            && now - since > DEADLINE_MS
        {
            self.transport.stop();
            self.commands_since = None;
            if let Some(busy) = self.busy.take() {
                done.timed_out = Some(busy);
                session.scene_failed(busy, TOO_LONG);
            }
            done.commands
                .extend(session.commands_failed(COMMAND_TOO_LONG));
        }
        let wanted = session.scene_job(quality);
        if let Some(busy) = self.busy
            && wanted != Some(busy)
        {
            // Superseded: what it builds is for a state no longer shown.
            // The worker is not ended: it looks at its messages between
            // features and bodies, leaves the old job for the one posted
            // next (or for none), and keeps what it built, so the next
            // state rebuilds only what the edit changed. Parts of the old
            // job that were on their way are dropped by their number.
            self.busy = None;
            done.cancelled = Some(busy);
            if wanted.is_none() {
                self.transport.cancel();
            }
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
        // The commands that need the bodies, to the worker that has them.
        for token in session.command_jobs() {
            let posted = session.command_request(token).and_then(|request| {
                self.transport
                    .post_command(token, &request.document, &request.work)
            });
            match posted {
                Ok(()) => {
                    self.commands_since.get_or_insert(now);
                }
                Err(why) => {
                    done.commands
                        .extend(session.command_answered(token, Err(why)));
                }
            }
        }
        done
    }

    /// Ends the worker (an edit the person cancelled is the one it works
    /// on, and it may never answer): what was posted to it is answered
    /// with errors that say so, and the next job starts a new worker (the
    /// scene it was building is posted again to that one).
    pub fn restart(&mut self, session: &mut Session) -> Vec<Answered> {
        self.transport.stop();
        self.busy = None;
        self.commands_since = None;
        session.commands_dropped(RESTARTED)
    }

    fn go_home(&mut self, session: &mut Session, why: &str, done: &mut Frame) {
        self.home = true;
        self.busy = None;
        session.set_regen_elsewhere(false);
        session.set_commands_elsewhere(false);
        done.home = Some(why.to_string());
        done.commands.extend(session.commands_failed(&format!(
            "it was to run beside the page, where the worker cannot be had ({why}); run it again"
        )));
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
        cancels: Rc<RefCell<usize>>,
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
        fn cancel(&mut self) {
            *self.cancels.borrow_mut() += 1;
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

    /// `w` set as the Parameters form sets it: the regeneration is left
    /// to the host.
    fn set(s: &mut Session, w: f64) {
        s.execute("param.set", &json!({"values": {"w": format!("{w} mm")}}))
            .unwrap();
    }

    /// A cube of side `w`, a public parameter, set to `w` last.
    fn cube(s: &mut Session, w: f64) {
        for (id, params) in [
            ("file.new", json!({"units": "mm"})),
            ("variable.set", json!({"name": "w", "expr": "10 mm"})),
            (
                "variable.publish",
                json!({"name": "w", "min": "1 mm", "max": "100 mm"}),
            ),
            (
                "feature.block",
                json!({"size": ["w", "w", "w"], "name": "B"}),
            ),
        ] {
            s.execute(id, &params).unwrap();
        }
        set(s, w);
    }

    fn session() -> Session {
        let mut s = Session::new(Workspace::memory());
        s.set_stepwise(true);
        s.set_checks_elsewhere(true);
        cube(&mut s, 11.0);
        s
    }

    /// The scene a session that builds it itself shows for `w`.
    fn wanted(w: f64) -> Vec<u8> {
        let mut s = Session::new(Workspace::memory());
        cube(&mut s, w);
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
        set(&mut s, 12.0);
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
        set(&mut s, 13.0);
        // The old job is no longer wanted: cancelled, the new one posted.
        let frame = scenes.frame_at(&mut s, quality(), Vec::new(), 1.0);
        assert_eq!(frame.cancelled, Some(old));
        // The worker is kept, with what it built: the new job replaces
        // the old in it.
        assert_eq!(*fake.stops.borrow(), 0);
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
        set(&mut s, 15.0);
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
        set(&mut s, 14.0);
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
        set(&mut s, 16.0);
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

    /// Another quality asked for (a finer display, the window's own)
    /// while the worker builds: the job is cancelled and the one at the
    /// new quality posted; its answer is the scene at that quality.
    #[test]
    fn another_quality_cancels_the_job_the_worker_builds() {
        let mut s = session();
        let fake = Fake::default();
        let mut scenes = Scenes::new(fake.clone(), &mut s);
        let old = scenes
            .frame_at(&mut s, quality(), Vec::new(), 0.0)
            .posted
            .unwrap();
        let late = fake.answers();
        let finer = SceneQuality::Display {
            relative: 1e-4,
            max_triangles: 1_000_000,
            max_angle: None,
        };
        let frame = scenes.frame_at(&mut s, finer, Vec::new(), 1.0);
        assert_eq!(frame.cancelled, Some(old));
        assert_eq!(*fake.stops.borrow(), 0);
        let new = frame.posted.unwrap();
        assert!(scenes.frame_at(&mut s, finer, late, 2.0).parts.is_empty());
        scenes.frame_at(&mut s, finer, fake.answers(), 3.0);
        assert_eq!(scenes.busy(), None);
        assert!(s.scene_ready(finer));
        let mut here = Session::new(Workspace::memory());
        cube(&mut here, 11.0);
        assert_eq!(
            scene_bytes(&s.scene_with(finer).unwrap().unwrap()),
            scene_bytes(&here.scene_with(finer).unwrap().unwrap()),
            "job {new}"
        );
    }

    /// Another tab made active while the worker builds the first: the job
    /// is cancelled, the other document's posted; back on the first tab,
    /// its state is built again, never shown from a late answer.
    #[test]
    fn another_tab_cancels_the_job_the_worker_builds() {
        let mut s = session();
        cube(&mut s, 17.0);
        let fake = Fake::default();
        let mut scenes = Scenes::new(fake.clone(), &mut s);
        let second = scenes
            .frame_at(&mut s, quality(), Vec::new(), 0.0)
            .posted
            .unwrap();
        let late = fake.answers();
        s.execute("file.activate", &json!({"document": 0})).unwrap();
        let frame = scenes.frame_at(&mut s, quality(), Vec::new(), 1.0);
        assert_eq!(frame.cancelled, Some(second));
        assert_eq!(*fake.stops.borrow(), 0);
        let first = frame.posted.unwrap();
        assert_ne!(first, second);
        assert!(
            scenes
                .frame_at(&mut s, quality(), late, 2.0)
                .parts
                .is_empty()
        );
        assert!(!s.scene_ready(quality()));
        scenes.frame_at(&mut s, quality(), fake.answers(), 3.0);
        assert_eq!(shown(&mut s), wanted(11.0));
        s.execute("file.activate", &json!({"document": 1})).unwrap();
        assert!(!s.scene_ready(quality()));
        scenes.frame_at(&mut s, quality(), Vec::new(), 4.0);
        scenes.frame_at(&mut s, quality(), fake.answers(), 5.0);
        assert_eq!(shown(&mut s), wanted(17.0));
        assert!(!s.regenerated_here());
    }

    /// An undo back to a state whose scenes are there while the worker
    /// builds another: nothing is posted, and the worker is told to leave
    /// the job (not ended).
    #[test]
    fn a_job_no_longer_wanted_is_left_by_the_worker_it_keeps() {
        let mut s = session();
        let fake = Fake::default();
        let mut scenes = Scenes::new(fake.clone(), &mut s);
        scenes.frame_at(&mut s, quality(), Vec::new(), 0.0);
        scenes.frame_at(&mut s, quality(), fake.answers(), 1.0);
        set(&mut s, 12.0);
        let busy = scenes
            .frame_at(&mut s, quality(), Vec::new(), 2.0)
            .posted
            .unwrap();
        s.execute("edit.undo", &json!({})).unwrap();
        let frame = scenes.frame_at(&mut s, quality(), Vec::new(), 3.0);
        assert_eq!(frame.cancelled, Some(busy));
        assert_eq!(frame.posted, None);
        assert_eq!((*fake.cancels.borrow(), *fake.stops.borrow()), (1, 0));
        assert_eq!(shown(&mut s), wanted(11.0));
    }

    /// A worker as `worker.js` is: it builds the job posted last a piece
    /// at a time (`stop` says yes at once: a feature or a body per
    /// piece), and takes the messages that came between pieces.
    #[derive(Clone, Default)]
    struct Stepping {
        inbox: Posted,
        current: Rc<RefCell<Option<u64>>>,
    }

    impl Transport for Stepping {
        fn post(&mut self, token: u64, document: &[u8], work: &str) -> Result<(), String> {
            self.inbox
                .borrow_mut()
                .push((token, document.to_vec(), work.into()));
            Ok(())
        }
        fn stop(&mut self) {
            craft3d_engine::scene_forget();
            self.inbox.borrow_mut().clear();
            *self.current.borrow_mut() = None;
        }
        fn cancel(&mut self) {
            *self.current.borrow_mut() = None;
        }
    }

    impl Stepping {
        /// One piece, after the messages: what it answered, and whether
        /// it worked at all.
        fn piece(&self) -> (Vec<Reply>, bool) {
            for (token, document, work) in self.inbox.borrow_mut().drain(..) {
                scene_begin(&document, &work);
                *self.current.borrow_mut() = Some(token);
            }
            let Some(token) = *self.current.borrow() else {
                return (Vec::new(), false);
            };
            let reply = match craft3d_engine::scene_step(|| true) {
                craft3d_engine::SceneStep::Part(bytes) => vec![Reply::Part { token, bytes }],
                craft3d_engine::SceneStep::Working => Vec::new(),
                craft3d_engine::SceneStep::Done => {
                    *self.current.borrow_mut() = None;
                    vec![Reply::Done { token }]
                }
            };
            (reply, true)
        }
    }

    /// `n` sketches, then a block `w` wide: `w` changes only the last
    /// feature, and there is one body to mesh.
    fn row(s: &mut Session, n: usize) {
        s.execute("file.new", &json!({"units": "mm"})).unwrap();
        s.execute("variable.set", &json!({"name": "w", "expr": "4 mm"}))
            .unwrap();
        for i in 0..n {
            s.execute("sketch.create", &json!({"name": format!("S{i}")}))
                .unwrap();
        }
        s.execute("feature.block", &json!({"size": ["w", 2, 2], "name": "B"}))
            .unwrap();
    }

    /// Pieces until the state shown has its first picture, with the page
    /// taking what came back after each.
    fn pieces_to_first_picture(
        s: &mut Session,
        scenes: &mut Scenes<Stepping>,
        worker: &Stepping,
    ) -> usize {
        let mut pieces = 0;
        for at in 0..10_000 {
            if s.scene_ready(SceneQuality::Preview) {
                return pieces;
            }
            let (replies, worked) = worker.piece();
            pieces += usize::from(worked);
            scenes.frame_at(s, quality(), replies, f64::from(at));
        }
        panic!("no first picture");
    }

    /// The worker keeps its caches when a job is replaced: a value set
    /// while it builds the one before rebuilds only what the value
    /// changes, where a worker started again (as one that was ended)
    /// regenerates every feature.
    #[test]
    fn a_superseded_job_leaves_its_caches_to_the_next() {
        let n = 24;
        let mut s = Session::new(Workspace::memory());
        s.set_stepwise(true);
        s.set_checks_elsewhere(true);
        row(&mut s, n);
        let worker = Stepping::default();
        let mut scenes = Scenes::new(worker.clone(), &mut s);
        let from_nothing = pieces_to_first_picture(&mut s, &mut scenes, &worker);
        assert!(from_nothing >= n, "{from_nothing} pieces for {n} features");
        // Values one after another, each set after a single piece of the
        // one before: every job is replaced while it is built.
        for (i, w) in [5.0, 6.0, 7.0, 8.0].into_iter().enumerate() {
            s.execute(
                "variable.set",
                &json!({"name": "w", "expr": format!("{w} mm")}),
            )
            .unwrap();
            scenes.frame_at(&mut s, quality(), Vec::new(), 100.0 + i as f64);
            let (replies, _) = worker.piece();
            scenes.frame_at(&mut s, quality(), replies, 100.5 + i as f64);
        }
        let kept = pieces_to_first_picture(&mut s, &mut scenes, &worker);
        assert!(
            kept * 4 < from_nothing,
            "{kept} pieces after a replaced job, {from_nothing} from nothing"
        );
        // The scenes are those of a session that builds them itself.
        while scenes.busy().is_some() {
            let (replies, _) = worker.piece();
            scenes.frame_at(&mut s, quality(), replies, 1e6);
        }
        let mut here = Session::new(Workspace::memory());
        row(&mut here, n);
        here.execute("variable.set", &json!({"name": "w", "expr": "8 mm"}))
            .unwrap();
        assert_eq!(
            shown(&mut s),
            scene_bytes(&here.scene_with(quality()).unwrap().unwrap())
        );
        // A worker started again rebuilds everything.
        let mut ended = worker.clone();
        ended.stop();
        s.execute("variable.set", &json!({"name": "w", "expr": "9 mm"}))
            .unwrap();
        let again = pieces_to_first_picture(&mut s, &mut scenes, &worker);
        assert!(again >= n, "{again} pieces from nothing");
    }
}
