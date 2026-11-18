//! The status bar says what a click will do (the UX round of 2026-11-18,
//! V6): after the selection's words, dim, one hint for the state the
//! window is in ([`hint_for`]): what a click does with nothing selected,
//! that Shift adds a second thing, that **N** writes a note, that Esc
//! clears, what the section's arrow and a note's pin take. A refusal that
//! has words of its own (a key that cannot work, a plane that was lost, a
//! check that was kept) puts them there for a while instead
//! ([`Shell::hint`], [`Shell::hint_until_command`]): the other parts of
//! the window call these. `ui.get` says the hint shown under `hint`.

use super::{CommandReport, Shell};

/// How long a hint given by [`Shell::hint`] stays, in seconds, where the
/// caller has no reason for another time: a refusal's.
pub const HINT_SECONDS: f32 = 4.0;
/// A status bar narrower than this, in points, shows the selection's
/// words alone, without the hint.
pub const HINT_LEAST_BAR: f32 = 800.0;

/// The state the bar's hint is chosen by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HintState {
    /// The document shown has bodies to click on.
    pub bodies: bool,
    /// How many things are selected.
    pub selected: usize,
    /// **N** can write a note here (a part, not an assembly or a Jig in
    /// use).
    pub notes: bool,
    /// The pointer rests on the section's arrow, or on a note's pin.
    pub on_arrow: bool,
    pub on_pin: bool,
}

/// The hint for `state`; `None` where there is nothing to say (no bodies,
/// nothing selected).
pub fn hint_for(state: &HintState) -> Option<&'static str> {
    if state.on_arrow {
        return Some("Drag to move the cut · double-click keeps the other half · Alt: no snapping");
    }
    if state.on_pin {
        return Some("Click to read the note");
    }
    match state.selected {
        0 if state.bodies => Some("Click a face, an edge or a hole to see its size"),
        0 => None,
        1 if state.notes => Some("Shift-click another to measure between · N writes a note"),
        1 => Some("Shift-click another to measure between"),
        2 => Some("Esc clears"),
        _ => Some("a measure takes one or two things"),
    }
}

/// A text shown for a while: for `seconds` from the frame it is first
/// drawn, or until the next command.
#[derive(Clone, Debug, PartialEq)]
pub struct Timed {
    pub text: String,
    end: End,
    /// When it was first drawn (egui's time); `None` before.
    since: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
enum End {
    Seconds(f32),
    /// Until the next command: the label of the command it waits for
    /// first (the one that made it, still queued), then the report it was
    /// shown with (`Some(None)`: no command yet).
    Command {
        after: Option<String>,
        anchor: Option<Option<CommandReport>>,
    },
}

impl Timed {
    /// `text` for `seconds` (at least a frame's worth).
    pub fn seconds(text: impl Into<String>, seconds: f32) -> Self {
        let seconds = if seconds.is_finite() {
            seconds.max(0.05)
        } else {
            HINT_SECONDS
        };
        Self {
            text: text.into(),
            end: End::Seconds(seconds),
            since: None,
        }
    }

    /// `text` until the next command after the one labelled `after` (that
    /// one still to come), or after the last one.
    pub fn until_command(text: impl Into<String>, after: Option<&str>) -> Self {
        Self {
            text: text.into(),
            end: End::Command {
                after: after.map(str::to_string),
                anchor: None,
            },
            since: None,
        }
    }

    /// Whether it is still shown at `now`, the last command being
    /// `last`; and how long until it ends, where that is a time.
    pub fn live(&mut self, now: f64, last: Option<&CommandReport>) -> (bool, Option<f64>) {
        let since = *self.since.get_or_insert(now);
        match &mut self.end {
            End::Seconds(seconds) => {
                let left = f64::from(*seconds) - (now - since);
                (left > 0.0, (left > 0.0).then_some(left))
            }
            End::Command { after, anchor } => {
                let ran = after
                    .as_deref()
                    .map(|label| last.is_some_and(|r| r.label == label));
                match ran {
                    Some(true) => {
                        *after = None;
                        *anchor = Some(last.cloned());
                        (true, None)
                    }
                    Some(false) => (true, None),
                    None => match anchor {
                        None => {
                            *anchor = Some(last.cloned());
                            (true, None)
                        }
                        Some(seen) => (seen.as_ref() == last, None),
                    },
                }
            }
        }
    }
}

/// What the status bar's hint holds between frames.
#[derive(Clone, Debug, Default)]
pub struct Hints {
    /// A hint given by [`Shell::hint`]: shown in place of the state's
    /// until it ends.
    given: Option<Timed>,
    /// What the bar showed in the last frame.
    shown: Option<String>,
}

impl Shell {
    /// Shows `text` as the status bar's hint for `seconds` from the frame
    /// it is first drawn, in place of the state's hint: a refusal's words
    /// where the person acted ([`HINT_SECONDS`] for one).
    pub fn hint(&mut self, text: impl Into<String>, seconds: f32) {
        self.hints.given = Some(Timed::seconds(text, seconds));
    }

    /// Shows `text` as the status bar's hint until the next command: the
    /// one after the command labelled `after` (queued, not yet run), or
    /// after the last one.
    pub fn hint_until_command(&mut self, text: impl Into<String>, after: Option<&str>) {
        self.hints.given = Some(Timed::until_command(text, after));
    }

    /// The hint the status bar showed in the last frame: `ui.get` › `hint`.
    pub fn status_hint(&self) -> Option<&str> {
        self.hints.shown.as_deref()
    }

    /// The state the hint is chosen by, at the pointer `pointer`.
    pub(super) fn hint_state(&self, pointer: Option<egui::Pos2>) -> HintState {
        let on_arrow = pointer.is_some_and(|p| {
            self.section_handle().is_some_and(|place| {
                place.part_at(p) == Some(super::section_handle::HandlePart::Arrow)
            })
        });
        let reach = self.tokens.note_radius + 3.0;
        let on_pin = pointer.is_some_and(|p| {
            self.notes
                .markers
                .iter()
                .any(|(_, centre)| centre.distance(p) <= reach)
        });
        HintState {
            bodies: self
                .shown_scene
                .as_ref()
                .is_some_and(|scene| !scene.bodies.is_empty()),
            selected: self.select.view.items.len(),
            notes: !self.is_assembly_shown() && !self.jig_in_use(),
            on_arrow,
            on_pin,
        }
    }

    /// The hint to show now (a given one while it lasts, else the
    /// state's), and how long until a given one ends; kept for `ui.get`.
    pub(super) fn hint_now(
        &mut self,
        now: f64,
        pointer: Option<egui::Pos2>,
    ) -> (Option<String>, Option<f64>) {
        let last = self.last_command.clone();
        let mut wait = None;
        match self
            .hints
            .given
            .as_mut()
            .map(|given| given.live(now, last.as_ref()))
        {
            Some((true, left)) => wait = left,
            Some((false, _)) => self.hints.given = None,
            None => {}
        }
        let text = match &self.hints.given {
            Some(given) => Some(given.text.clone()),
            None => hint_for(&self.hint_state(pointer)).map(str::to_string),
        };
        self.hints.shown.clone_from(&text);
        (text, wait)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hint_for_every_state() {
        let state = |selected: usize| HintState {
            bodies: true,
            selected,
            notes: true,
            ..HintState::default()
        };
        assert_eq!(
            hint_for(&state(0)),
            Some("Click a face, an edge or a hole to see its size")
        );
        assert_eq!(
            hint_for(&HintState::default()),
            None,
            "nothing to click on: nothing to say"
        );
        assert_eq!(
            hint_for(&state(1)),
            Some("Shift-click another to measure between · N writes a note")
        );
        assert_eq!(
            hint_for(&HintState {
                notes: false,
                ..state(1)
            }),
            Some("Shift-click another to measure between")
        );
        assert_eq!(hint_for(&state(2)), Some("Esc clears"));
        assert_eq!(
            hint_for(&state(3)),
            Some("a measure takes one or two things")
        );
        assert_eq!(
            hint_for(&HintState {
                on_arrow: true,
                ..state(2)
            }),
            Some("Drag to move the cut · double-click keeps the other half · Alt: no snapping")
        );
        assert_eq!(
            hint_for(&HintState {
                on_pin: true,
                ..state(0)
            }),
            Some("Click to read the note")
        );
    }

    #[test]
    fn a_timed_hint_counts_from_the_frame_it_is_first_drawn() {
        let mut timed = Timed::seconds("Select one thing", 4.0);
        assert_eq!(timed.live(100.0, None), (true, Some(4.0)));
        assert!(timed.live(103.5, None).0);
        assert!(!timed.live(104.5, None).0);
    }

    #[test]
    fn a_hint_until_the_next_command_waits_for_its_own_first() {
        let report = |label: &str| CommandReport {
            label: label.to_string(),
            ok: true,
            text: String::new(),
        };
        let mut timed = Timed::until_command("Kept", Some("assert.add"));
        let before = report("select.pick");
        assert!(
            timed.live(0.0, Some(&before)).0,
            "its command still to come"
        );
        let kept = report("assert.add");
        assert!(timed.live(0.1, Some(&kept)).0, "its command ran");
        assert!(timed.live(5.0, Some(&kept)).0, "no command since");
        assert!(!timed.live(5.1, Some(&report("select.clear"))).0);
        // Without a command to wait for: until the one after the last.
        let mut lost = Timed::until_command("Lost", None);
        assert!(lost.live(0.0, None).0);
        assert!(lost.live(1.0, None).0);
        assert!(!lost.live(2.0, Some(&before)).0);
    }
}
