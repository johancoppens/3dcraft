//! `common::checked::Overlapped`, which compares each command of an example
//! script with a regeneration from nothing on threads of its own: seen to
//! fail, with the comparisons given by the test (`compare_after`) instead
//! of a session's, so that one can be made to differ, to panic, or to end
//! in a chosen order.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::Overlapped;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The text a panic carries.
fn said(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_string()))
        .unwrap_or_default()
}

/// What `checked::run` said when the regeneration after a command differed.
fn as_run_said_it(after: &str) -> String {
    let payload = catch_unwind(|| assert_ne!(Some(false), Some(false), "{after}")).unwrap_err();
    said(payload.as_ref())
}

fn after(k: usize) -> String {
    format!("after step.{k} {{\"k\":{k}}}")
}

/// Commands 2 and 3 differ; their comparisons end in the reverse order of
/// the commands (the last first), so 3's verdict is there before 2's. The
/// test fails with `run`'s message for command 2, and names no later one.
#[test]
fn the_first_command_that_differs_fails_the_test_whatever_ends_first() {
    let ended = Arc::new(Mutex::new(Vec::new()));
    let mut checks = Overlapped::new(5);
    let unreported = checks.unreported();
    // Comparison k waits until k + 1's has ended; the last one's is let go
    // below.
    let (go, wait): (Vec<_>, Vec<_>) = (0..5).map(|_| mpsc::channel::<()>()).unzip();
    for (k, wait_for) in wait.into_iter().enumerate() {
        let let_earlier_go = k.checked_sub(1).map(|j| go[j].clone());
        let ended = Arc::clone(&ended);
        checks.compare_after(after(k), move || {
            wait_for.recv().unwrap();
            ended.lock().unwrap().push(k);
            if let Some(earlier) = let_earlier_go {
                earlier.send(()).unwrap();
            }
            k != 2 && k != 3
        });
    }
    go[4].send(()).unwrap();
    let outcome = catch_unwind(AssertUnwindSafe(move || checks.finish()));
    let failure = said(outcome.unwrap_err().as_ref());
    assert_eq!(failure, as_run_said_it(&after(2)));
    assert!(!failure.contains("step.3"), "{failure}");
    assert_eq!(*ended.lock().unwrap(), vec![4, 3, 2, 1, 0]);
    assert!(unreported.lock().unwrap().is_empty());
}

/// A script whose last command differs and that ends without `wait`
/// (it returned early) still fails, with `run`'s message for that command.
#[test]
fn a_difference_after_the_last_command_is_not_lost_when_the_test_ends_early() {
    let outcome = catch_unwind(|| {
        let mut checks = Overlapped::new(3);
        checks.compare_after(after(0), || true);
        checks.compare_after(after(1), || true);
        checks.compare_after(after(2), || {
            std::thread::sleep(Duration::from_millis(50));
            false
        });
        // No wait: the checks are dropped here.
    });
    assert_eq!(
        said(outcome.unwrap_err().as_ref()),
        as_run_said_it(&after(2))
    );
}

/// A test that fails for its own reason while a comparison that differs is
/// under way fails with its own message, and the comparison is reported
/// beside it, not dropped.
#[test]
fn a_test_failing_for_its_own_reason_keeps_its_message_and_reports_the_comparison() {
    let unreported;
    let outcome = {
        let mut checks = Overlapped::new(3);
        unreported = checks.unreported();
        checks.compare_after(after(0), || true);
        checks.compare_after(after(1), || {
            std::thread::sleep(Duration::from_millis(50));
            false
        });
        catch_unwind(AssertUnwindSafe(move || {
            let _checks = checks;
            panic!("the test's own assertion");
        }))
    };
    assert_eq!(
        said(outcome.unwrap_err().as_ref()),
        "the test's own assertion"
    );
    assert_eq!(
        *unreported.lock().unwrap(),
        vec![format!(
            "the regeneration differed from one from nothing {}",
            after(1)
        )]
    );
}

/// A comparison that panics fails the test, naming its command and saying
/// why.
#[test]
fn a_comparison_that_panics_fails_the_test_naming_its_command() {
    let outcome = catch_unwind(|| {
        let mut checks = Overlapped::new(3);
        checks.compare_after(after(0), || true);
        checks.compare_after(after(1), || panic!("no regeneration"));
        checks.compare_after(after(2), || true);
        checks.finish();
    });
    let failure = said(outcome.unwrap_err().as_ref());
    assert!(failure.contains(&after(1)), "{failure}");
    assert!(failure.contains("no regeneration"), "{failure}");
    assert!(!failure.contains("step.2"), "{failure}");
}

/// With three under way the fourth command waits for the first comparison
/// to end before its own starts: none is skipped, at most three run at
/// once, and every command's comparison ran (counted by the comparisons
/// themselves, and by the helper's own count in `finish`).
#[test]
fn a_full_queue_makes_the_next_command_wait_and_skips_none() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let running = Arc::new(Mutex::new((0usize, 0usize)));
    let mut checks = Overlapped::new(3);
    for k in 0..10 {
        let (log_k, running_k) = (Arc::clone(&log), Arc::clone(&running));
        checks.compare_after(after(k), move || {
            {
                let mut r = running_k.lock().unwrap();
                r.0 += 1;
                r.1 = r.1.max(r.0);
            }
            log_k.lock().unwrap().push(format!("start {k}"));
            std::thread::sleep(Duration::from_millis(20));
            log_k.lock().unwrap().push(format!("end {k}"));
            running_k.lock().unwrap().0 -= 1;
            true
        });
        log.lock().unwrap().push(format!("queued {k}"));
    }
    checks.finish();
    let log = log.lock().unwrap().clone();
    let at = |event: String| log.iter().position(|e| *e == event).unwrap();
    for k in 3..10 {
        assert!(
            at(format!("end {}", k - 3)) < at(format!("queued {k}")),
            "{log:?}"
        );
    }
    assert_eq!(log.iter().filter(|e| e.starts_with("end ")).count(), 10);
    assert!(running.lock().unwrap().1 <= 3, "{log:?}");
}

/// Command 4 fails (panics) while the comparison of command 2, which
/// differs, is still under way: the test fails with `run`'s message for
/// command 2, as `checked::run` would have before command 4 ran. With no
/// difference under way, command 4's own failure goes on.
#[test]
fn a_failing_command_does_not_hide_an_earlier_difference() {
    let outcome = catch_unwind(|| {
        let mut checks = Overlapped::new(3);
        checks.compare_after(after(1), || true);
        checks.compare_after(after(2), || {
            std::thread::sleep(Duration::from_millis(50));
            false
        });
        checks.compare_after(after(3), || true);
        checks.command(|| panic!("command 4 failed"));
    });
    let failure = said(outcome.unwrap_err().as_ref());
    assert_eq!(failure, as_run_said_it(&after(2)));
    let outcome = catch_unwind(|| {
        let mut checks = Overlapped::new(3);
        checks.compare_after(after(1), || true);
        checks.compare_after(after(2), || {
            std::thread::sleep(Duration::from_millis(50));
            true
        });
        checks.command(|| panic!("command 3 failed"));
    });
    assert_eq!(said(outcome.unwrap_err().as_ref()), "command 3 failed");
    let mut checks = Overlapped::new(3);
    assert_eq!(checks.command(|| 7), 7);
    checks.finish();
}
