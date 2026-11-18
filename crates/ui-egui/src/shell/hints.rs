//! The status bar says what a click will do (the UX round of 2026-11-18,
//! V6): after the selection's words, dim, one hint for the state the
//! window is in ([`hint_for`]): what a click does with nothing selected,
//! that Shift adds a second thing, that **N** writes a note, that Esc
//! clears, what the section's arrow and a note's pin take. A refusal that
//! has words of its own (a key that cannot work, a plane that was lost, a
//! check that was kept) puts them there for a while instead
//! ([`Shell::hint`], [`Shell::hint_until_next`]): the other parts of the
//! window call these. `ui.get` says the hint the bar shows under `hint`.

use super::{CommandReport, Shell};

/// How long a hint given by [`Shell::hint`] stays, in seconds, where the
/// caller has no reason for another time: a refusal's.
pub const HINT_SECONDS: f32 = 4.0;
/// A status bar narrower than this, in points, shows the selection's
/// words alone, without the hint.
pub const HINT_LEAST_BAR: f32 = 800.0;
/// The longest a hint given until the next action
/// ([`Shell::hint_until_next`]) stays while nothing is done, in seconds.
pub const UNTIL_NEXT_MOST_SECONDS: f32 = 15.0;

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
/// drawn, or until the next action.
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
    /// Until the next action of the person's or an agent's, at most
    /// [`UNTIL_NEXT_MOST_SECONDS`] (the review of the UX round,
    /// 2026-11-18: the window's own clicks and the side panel's commands
    /// report no command, so a hint that waited for one stayed through
    /// any number of clicks): the actions counted and the command
    /// reported when it was first drawn.
    Next {
        anchor: Option<(u64, Option<CommandReport>)>,
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

    /// `text` until the next action after the frame it is first drawn (a
    /// pointer's button or a key pressed in the window, a command
    /// reported), at most [`UNTIL_NEXT_MOST_SECONDS`].
    pub fn until_next(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            end: End::Next { anchor: None },
            since: None,
        }
    }

    /// Whether it is still shown at `now`, the last command reported being
    /// `last`, and how long until it ends by its time: [`Self::live_among`]
    /// where no action is counted.
    pub fn live(&mut self, now: f64, last: Option<&CommandReport>) -> (bool, Option<f64>) {
        self.live_among(now, last, 0)
    }

    /// Whether it is still shown at `now`, the last command reported being
    /// `last` and the actions counted so far `acts`; and how long until it
    /// ends by its time.
    pub fn live_among(
        &mut self,
        now: f64,
        last: Option<&CommandReport>,
        acts: u64,
    ) -> (bool, Option<f64>) {
        let since = *self.since.get_or_insert(now);
        let seconds = match &self.end {
            End::Seconds(seconds) => *seconds,
            End::Next { .. } => UNTIL_NEXT_MOST_SECONDS,
        };
        let left = f64::from(seconds) - (now - since);
        let quiet = match &mut self.end {
            End::Seconds(_) => true,
            End::Next { anchor } => {
                let (seen_acts, seen_last) = anchor.get_or_insert_with(|| (acts, last.cloned()));
                *seen_acts == acts && seen_last.as_ref() == last
            }
        };
        let live = quiet && left > 0.0;
        (live, live.then_some(left))
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
    /// The actions seen in the window so far: a pointer's button or a key
    /// pressed ([`Shell::count_acts`]).
    acts: u64,
}

/// How many of `events` are an action of the person's: a pointer's button
/// or a key pressed (a key held down and repeating counts once).
pub fn acts_in(events: &[egui::Event]) -> u64 {
    let acts = events
        .iter()
        .filter(|event| {
            matches!(
                event,
                egui::Event::PointerButton { pressed: true, .. }
                    | egui::Event::Key {
                        pressed: true,
                        repeat: false,
                        ..
                    }
            )
        })
        .count();
    u64::try_from(acts).unwrap_or(u64::MAX)
}

impl Shell {
    /// Shows `text` as the status bar's hint for `seconds` from the frame
    /// it is first drawn, in place of the state's hint: a refusal's words
    /// where the person acted ([`HINT_SECONDS`] for one).
    pub fn hint(&mut self, text: impl Into<String>, seconds: f32) {
        self.hints.given = Some(Timed::seconds(text, seconds));
    }

    /// Shows `text` as the status bar's hint until the next action after
    /// the frame it is first drawn (a click, a key, an agent's command), at
    /// most [`UNTIL_NEXT_MOST_SECONDS`].
    pub fn hint_until_next(&mut self, text: impl Into<String>) {
        self.hints.given = Some(Timed::until_next(text));
    }

    /// The hint the status bar showed in the last frame: `ui.get` › `hint`.
    pub fn status_hint(&self) -> Option<&str> {
        self.hints.shown.as_deref()
    }

    /// Counts the actions in this frame's input: called before anything
    /// is drawn, so a hint given in answer to one is not ended by it.
    pub(super) fn count_acts(&mut self, ctx: &egui::Context) {
        let acts = ctx.input(|i| acts_in(&i.events));
        self.hints.acts = self.hints.acts.saturating_add(acts);
    }

    /// What the bar shows of the hint this frame (`None` where it shows
    /// none, as a bar too narrow for it): what `ui.get` says.
    pub(super) fn hint_shown(&mut self, text: Option<String>) {
        self.hints.shown = text;
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
    /// state's), and how long until a given one ends. The bar says what it
    /// showed of it with [`Self::hint_shown`].
    pub(super) fn hint_now(
        &mut self,
        now: f64,
        pointer: Option<egui::Pos2>,
    ) -> (Option<String>, Option<f64>) {
        let last = self.last_command.clone();
        let acts = self.hints.acts;
        let mut wait = None;
        match self
            .hints
            .given
            .as_mut()
            .map(|given| given.live_among(now, last.as_ref(), acts))
        {
            Some((true, left)) => wait = left,
            Some((false, _)) => self.hints.given = None,
            None => {}
        }
        let text = match &self.hints.given {
            Some(given) => Some(given.text.clone()),
            None => hint_for(&self.hint_state(pointer)).map(str::to_string),
        };
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
    fn a_hint_until_the_next_action_ends_at_a_click_a_key_a_command_or_its_time() {
        let report = |label: &str| CommandReport {
            label: label.to_string(),
            ok: true,
            text: String::new(),
        };
        let before = report("select.pick");
        // Anchored where it is first drawn: what was done before stays.
        let mut kept = Timed::until_next("Kept");
        assert_eq!(
            kept.live_among(0.0, Some(&before), 3),
            (true, Some(f64::from(UNTIL_NEXT_MOST_SECONDS)))
        );
        assert!(
            kept.live_among(5.0, Some(&before), 3).0,
            "nothing done since"
        );
        assert!(
            !kept.live_among(5.1, Some(&before), 4).0,
            "a click or a key"
        );
        let mut lost = Timed::until_next("Lost");
        assert!(lost.live_among(0.0, None, 0).0);
        assert!(
            !lost.live_among(1.0, Some(&before), 0).0,
            "an agent's command"
        );
        let mut idle = Timed::until_next("Idle");
        assert!(idle.live_among(10.0, None, 0).0);
        assert!(idle.live_among(24.9, None, 0).0);
        assert!(!idle.live_among(25.1, None, 0).0, "its time is up");
    }

    #[test]
    fn a_pointer_or_a_key_pressed_is_an_action_its_release_and_a_repeat_not() {
        let key = |pressed: bool, repeat: bool| egui::Event::Key {
            key: egui::Key::Z,
            physical_key: None,
            pressed,
            repeat,
            modifiers: egui::Modifiers::CTRL,
        };
        let button = |pressed: bool| egui::Event::PointerButton {
            pos: egui::Pos2::ZERO,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        assert_eq!(acts_in(&[button(true), key(true, false)]), 2);
        assert_eq!(
            acts_in(&[
                button(false),
                key(false, false),
                key(true, true),
                egui::Event::PointerMoved(egui::Pos2::ZERO),
            ]),
            0
        );
    }
}
