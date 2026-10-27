//! A measure over many pairs of bodies (a clearance, an overlap), taken
//! a pair at a time by a host with one thread (the page in a browser,
//! [`crate::Session::prepare_some`]): one pair per piece of work at
//! least, the pairs measured so far kept between the pieces, and the
//! answer, once every pair is in, the one measuring all of them in one
//! go gives, bit for bit (each pair is measured by the same code from the
//! same bodies, and the answers are merged in the pairs' order).
//!
//! The measure itself does not know whether it is taken in pieces: it
//! asks [`pair`] for each pair in its order, which measures the pair
//! there and then outside [`in_pieces`], and inside it hands back a pair
//! measured in an earlier piece, measures the next one, or says the piece
//! is used up ([`STOPPED`]: the measure gives up, its answer is thrown
//! away, and the next piece takes it up again with the pairs so far).

use crate::verify::Nearest;
use std::cell::RefCell;

/// What one pair of bodies of a measure came to.
#[derive(Clone, Debug)]
pub(crate) enum PairAnswer {
    /// A clearance: the nearest places and the pair's status (`clear`,
    /// `touch` or `overlap`), or why it cannot be told.
    Clearance(Result<(Nearest, &'static str), String>),
    /// An overlap: whether the pair has a volume in common and how much
    /// (`overlap`'s answer per pair), `None` for a pair that is not one.
    Overlap(Option<Result<Option<Option<f64>>, String>>),
}

/// The error of a measure that stopped between two pairs; never shown,
/// as its answer is thrown away.
pub(crate) const STOPPED: &str = "stopped between two pairs of bodies (taken up in the next piece)";

struct Pieces {
    /// The pairs measured so far, in the order the measure asks for them.
    answers: Vec<PairAnswer>,
    /// The next pair the measure asks for.
    next: usize,
    /// Pairs measured in this piece.
    fresh: usize,
    stop: Box<dyn FnMut() -> bool>,
    stopped: bool,
}

thread_local! {
    static PIECES: RefCell<Option<Pieces>> = const { RefCell::new(None) };
}

/// What [`in_pieces`] did.
pub(crate) struct Taken<R> {
    /// What the work returned: its answer unless it `stopped`.
    pub out: R,
    /// Every pair measured so far, to be handed to the next piece.
    pub answers: Vec<PairAnswer>,
    /// True when it stopped between two pairs: `out` is not the answer.
    pub stopped: bool,
}

/// Puts back what was there before, also when the work panics (a bug):
/// a measure after it on this thread is not taken in pieces by mistake.
struct Restore(Option<Option<Pieces>>);

impl Drop for Restore {
    fn drop(&mut self) {
        if let Some(earlier) = self.0.take() {
            let _ = PIECES.try_with(|p| p.replace(earlier));
        }
    }
}

/// Runs `work` (one assertion's measure) a piece of it: the pairs in
/// `answers` (from the pieces before) are not measured again, at least
/// one more is, and `stop` is asked before each one after it.
pub(crate) fn in_pieces<R>(
    answers: Vec<PairAnswer>,
    stop: Box<dyn FnMut() -> bool>,
    work: impl FnOnce() -> R,
) -> Taken<R> {
    let mine = Pieces {
        answers,
        next: 0,
        fresh: 0,
        stop,
        stopped: false,
    };
    let mut restore = Restore(Some(PIECES.with(|p| p.replace(Some(mine)))));
    let out = work();
    let earlier = restore.0.take().unwrap_or_default();
    let mine = PIECES.with(|p| p.replace(earlier));
    let (answers, stopped) = mine.map_or((Vec::new(), false), |m| (m.answers, m.stopped));
    Taken {
        out,
        answers,
        stopped,
    }
}

/// True while a measure is taken in pieces ([`in_pieces`]): its pairs
/// are then asked for one after the other, never on threads of their
/// own.
pub(crate) fn active() -> bool {
    PIECES.with(|p| p.borrow().is_some())
}

/// True when the next pair asked for was measured in an earlier piece:
/// the measure was begun before, so what it checks of all its bodies
/// before its first pair (their parts) was checked then.
pub(crate) fn resumed() -> bool {
    PIECES.with(|p| {
        p.borrow()
            .as_ref()
            .is_some_and(|p| p.answers.len() > p.next)
    })
}

enum Step {
    Now,
    Known(PairAnswer),
    Fresh(usize),
    Stop,
}

/// The next pair of the measure: measured by `measure` now, or, taken in
/// pieces, the answer of an earlier piece; `Err(STOPPED)` when the piece
/// is used up before it (never before the first pair of a piece).
pub(crate) fn pair(measure: impl FnOnce() -> PairAnswer) -> Result<PairAnswer, String> {
    let step = PIECES.with(|p| {
        let mut held = p.borrow_mut();
        let Some(pieces) = held.as_mut() else {
            return Step::Now;
        };
        if pieces.stopped {
            return Step::Stop;
        }
        let n = pieces.next;
        pieces.next = n.saturating_add(1);
        if let Some(known) = pieces.answers.get(n) {
            return Step::Known(known.clone());
        }
        if pieces.fresh > 0 && (pieces.stop)() {
            pieces.stopped = true;
            return Step::Stop;
        }
        Step::Fresh(n)
    });
    match step {
        Step::Now => Ok(measure()),
        Step::Known(known) => Ok(known),
        Step::Stop => Err(STOPPED.into()),
        Step::Fresh(n) => {
            let answer = measure();
            PIECES.with(|p| {
                if let Some(pieces) = p.borrow_mut().as_mut() {
                    if pieces.answers.len() == n {
                        pieces.answers.push(answer.clone());
                    }
                    pieces.fresh = pieces.fresh.saturating_add(1);
                }
            });
            Ok(answer)
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;

    fn overlap(v: f64) -> PairAnswer {
        PairAnswer::Overlap(Some(Ok(Some(Some(v)))))
    }

    fn value(a: &PairAnswer) -> f64 {
        match a {
            PairAnswer::Overlap(Some(Ok(Some(Some(v))))) => *v,
            _ => f64::NAN,
        }
    }

    /// Five pairs, a stop that always says stop: one pair per piece,
    /// each measured once, and the last piece gives the answer.
    #[test]
    fn a_stop_that_always_stops_takes_one_pair_per_piece() {
        let measured = std::rc::Rc::new(std::cell::Cell::new(0));
        let work = |m: std::rc::Rc<std::cell::Cell<i32>>| {
            move || -> Result<f64, String> {
                let mut sum = 0.0;
                for i in 0..5 {
                    let m = std::rc::Rc::clone(&m);
                    sum += value(&pair(move || {
                        m.set(m.get() + 1);
                        overlap(f64::from(i))
                    })?);
                }
                Ok(sum)
            }
        };
        let mut answers = Vec::new();
        let mut pieces = 0;
        let out = loop {
            pieces += 1;
            let taken = in_pieces(answers, Box::new(|| true), work(measured.clone()));
            answers = taken.answers;
            if !taken.stopped {
                break taken.out;
            }
            assert_eq!(taken.out, Err(STOPPED.to_string()));
            assert!(pieces < 10);
        };
        assert_eq!(out, Ok(10.0));
        assert_eq!(pieces, 5);
        assert_eq!(measured.get(), 5);
        assert!(!active());
        // Outside, every pair is measured there and then.
        assert_eq!(work(measured.clone())(), Ok(10.0));
        assert_eq!(measured.get(), 10);
    }

    /// A panic in the work leaves the thread as it was.
    #[test]
    fn a_panic_in_the_work_leaves_no_pieces_behind() {
        let caught = std::panic::catch_unwind(|| {
            in_pieces(Vec::new(), Box::new(|| false), || panic!("a bug"))
        });
        assert!(caught.is_err());
        assert!(!active());
    }
}
