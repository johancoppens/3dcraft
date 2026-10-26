//! What a long piece of work is doing right now, for whoever shows it: a
//! window that keeps drawing while its engine thread regenerates a
//! document says which feature is being built.
//!
//! A report goes to the sink installed on the current thread
//! ([`with_sink`]), and nowhere when none is: regeneration does not know
//! who listens (a window, a test, nobody) and does not wait for them. The
//! sink is per thread, so two threads that regenerate at once do not see
//! each other's steps, and nothing a sink does changes what is built.

use std::cell::RefCell;

/// One step of the work, as it starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Progress {
    /// What is being done: `regenerating`, or a phase another layer
    /// reports through [`report`] (`meshing`).
    pub phase: &'static str,
    /// The feature being built (`F7`), when the step is one.
    pub feature: Option<String>,
    /// That feature's name, as the tree shows it.
    pub name: Option<String>,
    /// Which step this is, counted from 1, and how many there are at most
    /// (the features of the document; reused ones are skipped, so the
    /// count can jump).
    pub step: usize,
    pub steps: usize,
}

type Sink = Box<dyn FnMut(&Progress)>;

thread_local! {
    static SINK: RefCell<Option<Sink>> = const { RefCell::new(None) };
}

/// Runs `work` with `sink` receiving the steps reported on this thread,
/// and puts back whatever sink was there before, also when `work` panics
/// (the guard is dropped while unwinding).
pub fn with_sink<R>(sink: impl FnMut(&Progress) + 'static, work: impl FnOnce() -> R) -> R {
    struct Restore(Option<Sink>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let earlier = self.0.take();
            // During thread teardown the slot may be gone: nothing to do.
            let _ = SINK.try_with(|slot| {
                if let Ok(mut slot) = slot.try_borrow_mut() {
                    *slot = earlier;
                }
            });
        }
    }
    let earlier = SINK
        .try_with(|slot| {
            slot.try_borrow_mut()
                .ok()
                .and_then(|mut slot| slot.replace(Box::new(sink)))
        })
        .ok()
        .flatten();
    let _restore = Restore(earlier);
    work()
}

/// Tells this thread's sink, if any, that a step starts. A sink that
/// reports from inside itself is not called again (the report is
/// dropped), so a report never recurses.
pub fn report(progress: &Progress) {
    let _ = SINK.try_with(|slot| {
        if let Ok(mut slot) = slot.try_borrow_mut()
            && let Some(sink) = slot.as_mut()
        {
            sink(progress);
        }
    });
}

type Stop = Box<dyn FnMut() -> bool>;

thread_local! {
    static STOP: RefCell<Option<Stop>> = const { RefCell::new(None) };
    static STOPPED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Runs `work` with the regeneration in it allowed to stop part way, for
/// a host with one thread that has to draw between pieces of the work
/// (the page in a browser): once a feature was built, `stop` is asked
/// before each next one, and when it says so the features from there on
/// are not built (rolled back in the result, as behind the rollback
/// marker) and nothing is remembered of them, so the next regeneration
/// goes on where this one stopped and reuses what it built. The second
/// value is true when it stopped: such a result is not the document's,
/// only a measure of how far the work got.
///
/// Only the outermost regeneration in `work` may stop (one it starts
/// within, for a part of a feature, runs to its end), and only one: the
/// stop is used once. What is built is the same, stopped or not.
pub fn with_stop<R>(stop: impl FnMut() -> bool + 'static, work: impl FnOnce() -> R) -> (R, bool) {
    struct Restore(Option<Stop>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let earlier = self.0.take();
            let _ = STOP.try_with(|slot| {
                if let Ok(mut slot) = slot.try_borrow_mut() {
                    *slot = earlier;
                }
            });
        }
    }
    let earlier = STOP
        .try_with(|slot| {
            slot.try_borrow_mut()
                .ok()
                .and_then(|mut slot| slot.replace(Box::new(stop)))
        })
        .ok()
        .flatten();
    let _ = STOPPED.try_with(|s| s.set(false));
    let restore = Restore(earlier);
    let out = work();
    drop(restore);
    let stopped = STOPPED.try_with(|s| s.replace(false)).unwrap_or(false);
    (out, stopped)
}

/// The stop of [`with_stop`], taken by the regeneration that may use it
/// (none for one started within it).
pub(crate) fn take_stop() -> Option<Stop> {
    STOP.try_with(|slot| slot.try_borrow_mut().ok().and_then(|mut s| s.take()))
        .ok()
        .flatten()
}

/// Says that a regeneration stopped (`with_stop`'s answer).
pub(crate) fn note_stopped() {
    let _ = STOPPED.try_with(|s| s.set(true));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    #[test]
    fn a_stop_is_taken_once_and_reports_whether_it_was_used() {
        let ((), stopped) = with_stop(
            || true,
            || {
                assert!(take_stop().is_some());
                assert!(take_stop().is_none(), "used once");
            },
        );
        assert!(!stopped);
        let ((), stopped) = with_stop(|| true, note_stopped);
        assert!(stopped);
        assert!(take_stop().is_none(), "gone after the work");
        let ((), stopped) = with_stop(|| true, || {});
        assert!(!stopped, "not carried over");
    }

    fn step(n: usize) -> Progress {
        Progress {
            phase: "regenerating",
            feature: Some(format!("F{n}")),
            name: None,
            step: n,
            steps: 3,
        }
    }

    #[test]
    fn steps_go_to_the_sink_of_this_thread_only_while_it_is_installed() {
        let seen = Rc::new(RefCell::new(Vec::new()));
        report(&step(0));
        let for_sink = Rc::clone(&seen);
        let answer = with_sink(
            move |p| for_sink.borrow_mut().push(p.step),
            || {
                report(&step(1));
                // Another thread has no sink.
                std::thread::spawn(|| report(&step(9))).join().unwrap();
                report(&step(2));
                42
            },
        );
        report(&step(3));
        assert_eq!(answer, 42);
        assert_eq!(*seen.borrow(), [1, 2]);
    }

    #[test]
    fn the_earlier_sink_comes_back_also_after_a_panic() {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let outer = Rc::clone(&seen);
        with_sink(
            move |p| outer.borrow_mut().push(p.step),
            || {
                let inner = std::panic::catch_unwind(|| {
                    with_sink(|_| {}, || panic!("in the work"));
                });
                assert!(inner.is_err());
                report(&step(5));
            },
        );
        assert_eq!(*seen.borrow(), [5]);
    }
}
