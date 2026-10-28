//! The page's checks in a Web Worker: the assertions and outputs of the
//! active document are measured by a second instance of the page's own
//! module in a worker (`packaging/web/worker.js`), so the page's thread
//! never measures one and stays free to draw while they take seconds
//! (docs/design/engine.md › *Checks in a Web Worker*).
//!
//! [`Checks`] decides, once per frame, what goes to the worker and what
//! comes back; the worker itself is behind [`Transport`] (the page's is a
//! `web_sys::Worker`, `page.rs`), so this runs in native tests. The
//! session hands out one job at a time (`Session::checks_job`): it is
//! written out (`Session::checks_request`), posted, and its answer handed
//! back (`Session::checks_answered`). A job replaced while the worker
//! works on it (an edit, another document shown) is cancelled by ending
//! the worker, which the next job starts again. An answer that says the
//! worker stopped while measuring (a bug that aborted its module) makes
//! the job's checks errors and the worker is started again. A worker that
//! cannot be had at all (no worker file, a browser or a policy that
//! refuses it) before it ever answered sends the checks home: the page
//! measures them a piece per frame, as it did before.

use craft3d_engine::Session;

/// The worker, as [`Checks`] uses it.
pub trait Transport {
    /// Starts the worker (when it is not running) and posts it a job.
    fn post(&mut self, token: u64, document: &str, work: &str) -> Result<(), String>;
    /// Ends the worker and whatever it was doing.
    fn stop(&mut self);
}

/// What came back from the worker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    /// The answer to job `token` (`measure_checks`'s text), or why the
    /// worker stopped while measuring it.
    Answer {
        token: u64,
        answer: Result<String, String>,
    },
    /// The worker itself failed (it could not be loaded, or an error
    /// escaped it): why.
    Failed(String),
}

/// What [`Checks::frame`] did, for the page's log and its tests.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    /// The job posted this frame.
    pub posted: Option<u64>,
    /// The jobs answered this frame (answers for replaced jobs are
    /// dropped and not counted).
    pub answered: Vec<u64>,
    /// The job cancelled this frame (replaced while it was measured).
    pub cancelled: Option<u64>,
    /// The checks were sent home this frame, and why.
    pub home: Option<String>,
}

pub struct Checks<T: Transport> {
    transport: T,
    /// The job the worker is measuring.
    busy: Option<u64>,
    /// The worker answered at least once: it can be had on this page.
    answered_once: bool,
    /// Measured on the page again (the worker could not be had).
    home: bool,
}

impl<T: Transport> Checks<T> {
    /// Has `session` measure its checks elsewhere, through `transport`.
    pub fn new(transport: T, session: &mut Session) -> Self {
        session.set_checks_elsewhere(true);
        Self {
            transport,
            busy: None,
            answered_once: false,
            home: false,
        }
    }

    /// True once the checks were sent home: the page measures them.
    pub fn home(&self) -> bool {
        self.home
    }

    /// The job the worker is measuring.
    pub fn busy(&self) -> Option<u64> {
        self.busy
    }

    /// Takes what came back (`replies`), cancels a job that was replaced,
    /// and posts the job that waits. Called once per frame.
    pub fn frame(&mut self, session: &mut Session, replies: Vec<Reply>) -> Frame {
        let mut done = Frame::default();
        if self.home {
            return done;
        }
        for reply in replies {
            match reply {
                Reply::Answer { token, answer } => {
                    if self.busy == Some(token) {
                        self.busy = None;
                    }
                    match &answer {
                        Ok(_) => self.answered_once = true,
                        // Its module aborted: a new worker for the next.
                        Err(_) => self.transport.stop(),
                    }
                    if session.checks_answered(token, answer.as_deref().map_err(String::as_str)) {
                        done.answered.push(token);
                    }
                }
                Reply::Failed(why) => {
                    self.transport.stop();
                    let busy = self.busy.take();
                    if !self.answered_once {
                        // No worker on this page: measured here, as before.
                        self.go_home(session, &why, &mut done);
                        return done;
                    }
                    if let Some(token) = busy
                        && session.checks_answered(token, Err(&why))
                    {
                        done.answered.push(token);
                    }
                }
            }
        }
        let wanted = session.checks_job();
        if let Some(busy) = self.busy
            && wanted != Some(busy)
        {
            self.transport.stop();
            self.busy = None;
            done.cancelled = Some(busy);
        }
        if self.busy.is_none()
            && let Some(token) = wanted
        {
            match session.checks_request(token) {
                Ok(request) => match self.transport.post(token, &request.document, &request.work) {
                    Ok(()) => {
                        self.busy = Some(token);
                        done.posted = Some(token);
                    }
                    Err(why) => {
                        self.transport.stop();
                        if self.answered_once {
                            if session.checks_answered(token, Err(&why)) {
                                done.answered.push(token);
                            }
                        } else {
                            self.go_home(session, &why, &mut done);
                        }
                    }
                },
                // The document cannot be written out: its checks say so.
                Err(why) => {
                    if session.checks_answered(token, Err(&why)) {
                        done.answered.push(token);
                    }
                }
            }
        }
        done
    }

    fn go_home(&mut self, session: &mut Session, why: &str, done: &mut Frame) {
        self.home = true;
        self.busy = None;
        session.set_checks_elsewhere(false);
        done.home = Some(why.to_string());
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;
    use craft3d_engine::{AssertionStatus, SceneQuality, Workspace, measure_checks};
    use serde_json::json;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// A worker in the test: answers what it is posted with
    /// `measure_checks` when told to, or fails as told.
    #[derive(Clone, Default)]
    struct Fake {
        posted: Rc<RefCell<Vec<(u64, String, String)>>>,
        stops: Rc<RefCell<usize>>,
        refuse: Option<String>,
    }

    impl Transport for Fake {
        fn post(&mut self, token: u64, document: &str, work: &str) -> Result<(), String> {
            if let Some(why) = &self.refuse {
                return Err(why.clone());
            }
            self.posted
                .borrow_mut()
                .push((token, document.into(), work.into()));
            Ok(())
        }
        fn stop(&mut self) {
            *self.stops.borrow_mut() += 1;
        }
    }

    impl Fake {
        /// The answers to everything posted so far.
        fn answers(&self) -> Vec<Reply> {
            self.posted
                .borrow_mut()
                .drain(..)
                .map(|(token, d, w)| Reply::Answer {
                    token,
                    answer: Ok(measure_checks(&d, &w)),
                })
                .collect()
        }
    }

    fn session() -> Session {
        let mut s = Session::new(Workspace::memory());
        s.set_stepwise(true);
        for (id, params) in [
            ("file.new", json!({"units": "mm"})),
            ("variable.set", json!({"name": "w", "expr": "10 mm"})),
            (
                "feature.block",
                json!({"size": ["w", "w", "w"], "name": "B"}),
            ),
            (
                "assert.add",
                json!({"name": "cube", "measure": "volume", "of": "B", "eq": "w^3", "tol": "1e-6 * (1 mm)^3"}),
            ),
            // An edit: its checks are the host's (`assert.add` measured
            // its own in line).
            ("variable.set", json!({"name": "w", "expr": "11 mm"})),
        ] {
            s.execute(id, &params).unwrap();
        }
        s
    }

    fn pieces(s: &mut Session) {
        while s.prepare_some(SceneQuality::display(), || false).is_some() {}
    }

    #[test]
    fn checks_go_to_the_worker_and_come_back() {
        let mut s = session();
        let fake = Fake::default();
        let mut checks = Checks::new(fake.clone(), &mut s);
        pieces(&mut s);
        let first = checks.frame(&mut s, Vec::new());
        let token = first.posted.unwrap();
        // Waiting: nothing posted twice.
        assert_eq!(checks.frame(&mut s, Vec::new()).posted, None);
        let back = checks.frame(&mut s, fake.answers());
        assert_eq!(back.answered, vec![token]);
        let states = s.assertion_states();
        assert_eq!(states[0].status, AssertionStatus::Passed, "{states:?}");
        assert_eq!(checks.busy(), None);
    }

    #[test]
    fn an_edit_cancels_the_job_the_worker_measures() {
        let mut s = session();
        let fake = Fake::default();
        let mut checks = Checks::new(fake.clone(), &mut s);
        pieces(&mut s);
        let old = checks.frame(&mut s, Vec::new()).posted.unwrap();
        let late = fake.answers();
        s.execute("variable.set", &json!({"name": "w", "expr": "12 mm"}))
            .unwrap();
        // While regenerating there is no job: the old one is cancelled.
        let frame = checks.frame(&mut s, Vec::new());
        assert_eq!(frame.cancelled, Some(old));
        assert_eq!(*fake.stops.borrow(), 1);
        pieces(&mut s);
        let new = checks.frame(&mut s, Vec::new()).posted.unwrap();
        assert_ne!(new, old);
        // The old answer arrives late: dropped.
        assert!(checks.frame(&mut s, late).answered.is_empty());
        assert_eq!(checks.frame(&mut s, fake.answers()).answered, vec![new]);
        assert_eq!(s.assertion_states()[0].status, AssertionStatus::Passed);
    }

    #[test]
    fn a_worker_that_stopped_while_measuring_leaves_an_error() {
        let mut s = session();
        let fake = Fake::default();
        let mut checks = Checks::new(fake.clone(), &mut s);
        pieces(&mut s);
        let token = checks.frame(&mut s, Vec::new()).posted.unwrap();
        let frame = checks.frame(
            &mut s,
            vec![Reply::Answer {
                token,
                answer: Err("RuntimeError: unreachable".into()),
            }],
        );
        assert_eq!(frame.answered, vec![token]);
        assert_eq!(*fake.stops.borrow(), 1);
        let states = s.assertion_states();
        assert_eq!(states[0].status, AssertionStatus::Error, "{states:?}");
        assert!(states[0].message.contains("unreachable"));
        assert!(!checks.home());
    }

    #[test]
    fn a_page_without_a_worker_measures_its_checks_itself() {
        for refuse in [true, false] {
            let mut s = session();
            let fake = Fake {
                refuse: refuse.then(|| "SecurityError".to_string()),
                ..Fake::default()
            };
            let mut checks = Checks::new(fake.clone(), &mut s);
            pieces(&mut s);
            let frame = if refuse {
                checks.frame(&mut s, Vec::new())
            } else {
                checks.frame(&mut s, Vec::new());
                checks.frame(&mut s, vec![Reply::Failed("404 worker.js".into())])
            };
            assert!(frame.home.is_some(), "{frame:?}");
            assert!(checks.home() && !s.checks_elsewhere());
            // Measured on the page, a piece per frame, as before.
            pieces(&mut s);
            assert_eq!(s.assertion_states()[0].status, AssertionStatus::Passed);
            assert_eq!(checks.frame(&mut s, Vec::new()), Frame::default());
        }
    }

    #[test]
    fn a_worker_that_fails_after_it_answered_leaves_an_error_and_starts_again() {
        let mut s = session();
        let fake = Fake::default();
        let mut checks = Checks::new(fake.clone(), &mut s);
        pieces(&mut s);
        checks.frame(&mut s, Vec::new());
        checks.frame(&mut s, fake.answers());
        s.execute("variable.set", &json!({"name": "w", "expr": "14 mm"}))
            .unwrap();
        pieces(&mut s);
        let token = checks.frame(&mut s, Vec::new()).posted.unwrap();
        let frame = checks.frame(&mut s, vec![Reply::Failed("out of memory".into())]);
        assert_eq!(frame.answered, vec![token]);
        assert!(frame.home.is_none() && !checks.home());
        assert_eq!(s.assertion_states()[0].status, AssertionStatus::Error);
        // The next state goes to a new worker.
        s.execute("variable.set", &json!({"name": "w", "expr": "13 mm"}))
            .unwrap();
        pieces(&mut s);
        assert!(checks.frame(&mut s, Vec::new()).posted.is_some());
    }
}
