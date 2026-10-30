//! A calm status bar (docs/design/engine.md › *Saying what is being
//! done*): the work indicator shows only for work that has run a while
//! and then stays a while, and the texts that change while work or a
//! motion runs change at most a few times a second, an error at once.
//!
//! Both are functions of the time the frames give (`egui`'s input time),
//! so a headless test drives them frame by frame.

/// When work that is going on shows, and what it says.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct WorkIndicator {
    /// When the work now going on was first seen.
    since: Option<f64>,
    /// When the indicator was last shown, while it shows.
    shown_at: Option<f64>,
    /// What it says (held like the other texts).
    text: Held,
}

/// What [`WorkIndicator::frame`] decided: the words to show, if any, and
/// when to look again (seconds from now) for a change that is due
/// without anything else drawing a frame.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Shown {
    pub text: Option<String>,
    pub again_in: Option<f64>,
}

/// A time that means something: finite, else 0.
fn clean(seconds: f64) -> f64 {
    if seconds.is_finite() {
        seconds.max(0.0)
    } else {
        0.0
    }
}

impl WorkIndicator {
    /// One frame at `now`, with what is being done (`None`: nothing). The
    /// indicator shows once the work has run `after` seconds and stays at
    /// least `hold` seconds after it was shown; its words change at most
    /// once every `every` seconds.
    pub(crate) fn frame(
        &mut self,
        now: f64,
        busy: Option<&str>,
        after: f64,
        hold: f64,
        every: f64,
    ) -> Shown {
        let (after, hold) = (clean(after), clean(hold));
        match busy {
            Some(text) => {
                let since = *self.since.get_or_insert(now);
                let waited = now - since;
                if self.shown_at.is_none() && waited < after {
                    return Shown {
                        text: None,
                        again_in: Some((after - waited).max(0.0)),
                    };
                }
                self.shown_at = Some(now);
                let (text, again) = self.text.frame(now, text, false, every);
                Shown {
                    text: Some(text.to_string()),
                    again_in: again,
                }
            }
            None => {
                self.since = None;
                let Some(at) = self.shown_at else {
                    return Shown {
                        text: None,
                        again_in: None,
                    };
                };
                let left = hold - (now - at);
                if left > 0.0 {
                    Shown {
                        text: Some(self.text.shown().to_string()),
                        again_in: Some(left),
                    }
                } else {
                    self.shown_at = None;
                    self.text = Held::default();
                    Shown {
                        text: None,
                        again_in: None,
                    }
                }
            }
        }
    }

    /// Whether the indicator showed in the last frame.
    #[cfg(test)]
    pub(crate) fn showing(&self) -> bool {
        self.shown_at.is_some()
    }
}

/// A text of the status bar that changes at most once every so often:
/// the one shown, and when it was shown.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Held {
    shown: String,
    at: Option<f64>,
}

impl Held {
    /// The text to show at `now` when `wanted` is asked for: `wanted` when
    /// it is what is shown, when `urgent` (an error), the first time, or
    /// when what is shown has been there `every` seconds; else what is
    /// shown, and in how many seconds `wanted` may take its place.
    pub(crate) fn frame(
        &mut self,
        now: f64,
        wanted: &str,
        urgent: bool,
        every: f64,
    ) -> (&str, Option<f64>) {
        if self.shown == wanted && self.at.is_some() {
            return (&self.shown, None);
        }
        let left = self.at.map_or(0.0, |at| clean(every) - (now - at));
        if urgent || left <= 0.0 {
            wanted.clone_into(&mut self.shown);
            self.at = Some(now);
            return (&self.shown, None);
        }
        (&self.shown, Some(left))
    }

    pub(crate) fn shown(&self) -> &str {
        &self.shown
    }
}

/// The width an item of the status bar keeps while only its numbers
/// change: the widest it was for text of the same shape (its digits
/// counted as one), so that numbers counting up and down never make the
/// items after it jump.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Steady {
    shape: String,
    width: f32,
}

/// `text` with every run of digits as one `0`: `"CPU 800×600 · 12 ms"`
/// and `"CPU 801×600 · 9 ms"` have the same shape.
pub(crate) fn shape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_digits = false;
    for c in text.chars() {
        if c.is_ascii_digit() {
            if !in_digits {
                out.push('0');
            }
            in_digits = true;
        } else {
            out.push(c);
            in_digits = false;
        }
    }
    out
}

impl Steady {
    /// The width for `text`, `measured` wide: at least as wide as it was
    /// for text of its shape; a text of another shape starts anew.
    pub(crate) fn width(&mut self, text: &str, measured: f32) -> f32 {
        let measured = if measured.is_finite() {
            measured.max(0.0)
        } else {
            0.0
        };
        let shape = shape(text);
        if shape != self.shape {
            self.shape = shape;
            self.width = measured;
        } else {
            self.width = self.width.max(measured);
        }
        self.width
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AFTER: f64 = 0.35;
    const HOLD: f64 = 0.3;
    const EVERY: f64 = 0.25;

    #[test]
    fn short_work_never_shows_long_work_shows_late_and_stays() {
        let mut w = WorkIndicator::default();
        // Motion frames: 50 ms of work, 50 ms of none, 10 times a second.
        let mut now = 0.0;
        for _ in 0..50 {
            assert_eq!(
                w.frame(now, Some("view.motion…"), AFTER, HOLD, EVERY).text,
                None
            );
            now += 0.05;
            assert_eq!(w.frame(now, None, AFTER, HOLD, EVERY).text, None);
            now += 0.05;
        }
        // Long work: not before 0.35 s, then shown.
        let start = now;
        while now - start < AFTER - 1e-9 {
            let shown = w.frame(now, Some("Regenerating… 1/9"), AFTER, HOLD, EVERY);
            assert_eq!(shown.text, None);
            assert!(shown.again_in.is_some());
            now += 0.01;
        }
        // Just past the wait (`start + AFTER - start` may round below it).
        now = start + AFTER + 1e-9;
        assert_eq!(
            w.frame(now, Some("Regenerating… 1/9"), AFTER, HOLD, EVERY)
                .text
                .as_deref(),
            Some("Regenerating… 1/9")
        );
        // Done: stays 0.3 s with its last words, then goes.
        let done = now;
        assert_eq!(
            w.frame(done + 0.1, None, AFTER, HOLD, EVERY)
                .text
                .as_deref(),
            Some("Regenerating… 1/9")
        );
        assert_eq!(
            w.frame(done + 0.29, None, AFTER, HOLD, EVERY)
                .text
                .as_deref(),
            Some("Regenerating… 1/9")
        );
        assert_eq!(w.frame(done + 0.31, None, AFTER, HOLD, EVERY).text, None);
        assert!(!w.showing());
    }

    #[test]
    fn held_text_changes_at_most_every_so_often_an_error_at_once() {
        let mut h = Held::default();
        let mut changes = 0;
        let mut last = String::new();
        let mut now = 0.0;
        for k in 0..100 {
            let (text, _) = h.frame(now, &format!("t = {k}"), false, EVERY);
            if text != last {
                changes += 1;
                last = text.to_string();
            }
            now += 0.02;
        }
        // 2 s: the first, then one per 0.25 s at most.
        assert!(changes <= 9, "{changes}");
        let (text, _) = h.frame(now, "failed", true, EVERY);
        assert_eq!(text, "failed");
    }

    #[test]
    fn a_steady_width_only_grows_for_the_same_shape() {
        let mut s = Steady::default();
        assert_eq!(s.width("12 ms", 40.0), 40.0);
        assert_eq!(s.width("9 ms", 32.0), 40.0);
        assert_eq!(s.width("123 ms", 48.0), 48.0);
        assert_eq!(s.width("no picture", 60.0), 60.0);
        assert_eq!(s.width("t = 0.420", f32::NAN), 0.0);
        assert_eq!(shape("CPU 800×600 · 12 ms"), "CPU 0×0 · 0 ms");
    }
}
