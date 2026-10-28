//! Checks measured elsewhere (`Session::set_checks_elsewhere`, the page's
//! Web Worker): the host hands the jobs of assertions and outputs over,
//! `measure_checks` answers them from the written-out request alone, and
//! the session ends with exactly what a session that measures them itself
//! shows; without ever measuring one on its own thread.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{AssertionStatus, SceneQuality, Session, Workspace, measure_checks};
use serde_json::{Value, json};

/// Blocks at scale `s` (mm), a hole, outputs measured in two stages (the
/// second uses the first) and assertions on bodies and on the outputs;
/// then `w` set to 41 (scaled), an edit whose checks are left to the
/// host (`assert.add` measures its own assertion in line).
fn build(session: &mut Session, s: f64) {
    let mm = |v: f64| format!("{} mm", v * s);
    let steps = [
        ("file.new", json!({"units": "mm"})),
        ("variable.set", json!({"name": "w", "expr": mm(40.0)})),
        (
            "feature.block",
            json!({"size": ["w", mm(20.0), mm(5.0)], "name": "A"}),
        ),
        (
            "feature.block",
            json!({"size": ["w / 4", "w / 4", "w / 4"], "position": [mm(60.0), "0 mm", "0 mm"], "name": "B"}),
        ),
        (
            "feature.cylinder",
            json!({"radius": mm(2.0), "height": mm(20.0), "position": [mm(10.0), mm(10.0), mm(-5.0)], "name": "Pin"}),
        ),
        (
            "feature.boolean",
            json!({"op": "subtract", "target": "A", "tool": "Pin", "name": "Holed"}),
        ),
        (
            "variable.measure",
            json!({"name": "VB", "measure": "volume", "of": "B", "wait": false}),
        ),
        (
            "variable.output",
            json!({"name": "VB2", "expr": "2 * VB", "wait": false}),
        ),
        (
            "assert.add",
            json!({"name": "cube", "measure": "volume", "of": "B", "eq": "(w / 4)^3", "tol": "1e-6 * (1 mm)^3"}),
        ),
        (
            "assert.add",
            json!({"name": "apart", "measure": "overlap", "is": false}),
        ),
        (
            "assert.add",
            json!({"name": "twice", "measure": "value", "expr": "VB2", "min": "(w / 4)^3"}),
        ),
        ("variable.set", json!({"name": "w", "expr": mm(41.0)})),
    ];
    for (id, params) in steps {
        session
            .execute(id, &params)
            .unwrap_or_else(|e| panic!("{id}: {}", e.message()));
    }
}

/// Every check and output, with details, once all are measured.
fn answers(session: &mut Session) -> (Value, Value) {
    let assertions = session
        .execute("assert.list", &json!({"wait": true, "details": true}))
        .unwrap();
    let outputs = session
        .execute("output.list", &json!({"wait": true}))
        .unwrap();
    (assertions, outputs)
}

/// A session that measures elsewhere: the pieces, never `checking`.
fn elsewhere() -> Session {
    let mut session = Session::new(Workspace::none());
    session.set_stepwise(true);
    session.set_checks_elsewhere(true);
    session
}

/// Does the pieces, then hands every job over and answers it with
/// `measure_checks`, until none waits. Returns how many jobs there were.
fn settle(session: &mut Session) -> usize {
    let mut jobs = 0;
    for _ in 0..100 {
        while let Some(p) = session.prepare_some(SceneQuality::display(), || false) {
            assert_ne!(p.phase, "checking", "measured on the host's thread");
        }
        let Some(token) = session.checks_job() else {
            return jobs;
        };
        // Asked again, the same job keeps its number.
        assert_eq!(session.checks_job(), Some(token));
        let request = session.checks_request(token).unwrap();
        assert_eq!(request.token, token);
        let answer = measure_checks(&request.document, &request.work);
        assert!(session.checks_answered(token, Ok(&answer)), "{answer}");
        // Answered once: a second answer is dropped.
        assert!(!session.checks_answered(token, Ok(&answer)));
        jobs += 1;
    }
    panic!("jobs without end");
}

fn nothing_pending(session: &mut Session) -> bool {
    !session.assertions_pending()
        && session
            .assertion_states()
            .iter()
            .all(|a| a.status != AssertionStatus::Pending)
}

#[test]
fn checks_measured_elsewhere_are_those_measured_here() {
    for s in [1e-2, 1.0, 1e3] {
        let mut here = Session::new(Workspace::none());
        build(&mut here, s);
        let want = answers(&mut here);
        assert!(
            want.0["assertions"]
                .as_array()
                .unwrap()
                .iter()
                .all(|a| a["status"] == "passed"),
            "{}",
            want.0
        );

        let mut session = elsewhere();
        build(&mut session, s);
        // The outputs first, then what uses them: two stages at least.
        let jobs = settle(&mut session);
        assert!(jobs >= 2, "{jobs} jobs at scale {s}");
        // Everything is there before a command that waits is asked, so
        // that command measures nothing.
        assert!(nothing_pending(&mut session), "scale {s}");
        assert_eq!(answers(&mut session), want, "scale {s}");

        // An edit and back: only what changed is handed over again.
        for w in [40.0, 41.0] {
            session
                .execute(
                    "variable.set",
                    &json!({"name": "w", "expr": format!("{} mm", w * s)}),
                )
                .unwrap();
            settle(&mut session);
            assert!(nothing_pending(&mut session));
        }
        assert_eq!(answers(&mut session), want, "scale {s}, after edits");
    }
}

#[test]
fn an_edit_replaces_the_job_handed_out() {
    let mut session = elsewhere();
    build(&mut session, 1.0);
    while session
        .prepare_some(SceneQuality::display(), || false)
        .is_some()
    {}
    let old = session.checks_job().unwrap();
    let request = session.checks_request(old).unwrap();
    let late = measure_checks(&request.document, &request.work);
    session
        .execute("variable.set", &json!({"name": "w", "expr": "50 mm"}))
        .unwrap();
    while session
        .prepare_some(SceneQuality::display(), || false)
        .is_some()
    {}
    let new = session.checks_job().unwrap();
    assert_ne!(new, old);
    // The answer for the state before is dropped, and asking for it fails.
    assert!(!session.checks_answered(old, Ok(&late)));
    assert!(session.checks_request(old).is_err());
    settle(&mut session);
    assert!(nothing_pending(&mut session));
}

#[test]
fn a_worker_that_stopped_leaves_errors_never_pending() {
    for answer in [
        Err("RuntimeError: unreachable"),
        Ok("not json"),
        Ok("{\"checked\": []}"),
        Ok("{\"error\": \"the document cannot be read\"}"),
    ] {
        let mut session = elsewhere();
        build(&mut session, 1.0);
        while session
            .prepare_some(SceneQuality::display(), || false)
            .is_some()
        {}
        let token = session.checks_job().unwrap();
        assert!(session.checks_answered(token, answer));
        // What uses the outputs that failed is not left waiting either.
        settle(&mut session);
        let states = session.assertion_states();
        assert!(!states.is_empty());
        assert!(
            states.iter().all(|a| a.status != AssertionStatus::Pending),
            "{states:?}"
        );
        assert!(
            states.iter().any(|a| a.status == AssertionStatus::Error
                && a.message.contains("off the page's thread")),
            "{states:?}"
        );
    }
}

#[test]
fn only_a_host_that_works_a_piece_at_a_time_measures_elsewhere() {
    let mut session = Session::new(Workspace::none());
    session.set_checks_elsewhere(true);
    build(&mut session, 1.0);
    assert_eq!(session.checks_job(), None);
    assert!(session.checks_request(1).is_err());
    assert!(!session.checks_answered(1, Ok("{}")));
    let states = session
        .execute("assert.list", &json!({"wait": true}))
        .unwrap();
    assert_eq!(states["passed"], 3, "{states}");
    // Without a document, nothing either.
    let mut empty = elsewhere();
    assert_eq!(empty.checks_job(), None);
    assert!(!empty.checks_answered(0, Err("x")));
}
