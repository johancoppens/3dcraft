//! The history player: the timeline played like a video.
//!
//! The controls sit under the timeline's slider (to the first step, a
//! step back, play or pause, a step forward, to the end, the speed, loop
//! and orbit), and the keys work like a video player's while the pointer
//! is over the viewport or the feature panel and no text field has the
//! keyboard and no dialog is open: Space plays or pauses, the arrows
//! step, Home goes to the first step and End shows the end. The keys only
//! change the view: the "To end" button also removes the rollback marker,
//! an edit, which no bare key makes (decided 2026-10-23).
//!
//! Playing is view state ([`Playback`] in the UI state, so `ui.set` starts
//! it for the person watching): every step is a `view.scrub`, asked for
//! the way the slider asks. The next step is asked for only once the
//! scene of the step shown is there (a sync with the session found no
//! scrub waiting: whatever step the view settled on, the player's or one
//! an agent or a flight asked for, it goes on from there) and the step has
//! been drawn for [`Playback::dwell`], counted from the frame after the
//! one that drew it, so a heavy model never skips a step and a slow frame
//! does not eat the dwell. Time is egui's input time, which the browser
//! has too. At the last step it stops, or starts again
//! at the first with `loop`; an edit of the document, another tab, "To
//! end" or "Roll back here" stops it, and a failed scrub too. With orbit
//! the camera of the tab turns slowly about the vertical while it plays
//! (at most 30 frames a second, sharp unless a frame turns it more than a
//! degree), and waits while a turn, a flight or a drag has the camera.
//!
//! While the view is scrubbed or playing, the viewport shows the step's
//! caption, the one `view.replay` writes ([`craft3d_engine::step_caption`]).

use super::Shell;
use super::icons::{self, Icon};
use crate::state::{MOTION_SECONDS, Playback};
use crate::theme::{Tokens, color};
use craft3d_engine::Session;
use egui::{Sense, vec2};
use serde_json::{Value, json};

/// How fast the camera turns while it plays with orbit, at speed 1, in
/// degrees a second: once round in half a minute.
pub(crate) const ORBIT_DEGREES_PER_SECOND: f64 = 12.0;

/// The frames a second the orbit asks for.
const ORBIT_FPS: f64 = 30.0;

/// A turn per frame above this many degrees is drawn as a moving camera
/// (coarser, `MOVING_DIVISOR`); a slower orbit is drawn sharp.
const ORBIT_MOVING_DEGREES: f64 = 1.0;

/// Space: play or pause.
pub(crate) const PLAY_KEY: egui::Key = egui::Key::Space;
/// Left arrow: a step back.
pub(crate) const BACK_KEY: egui::Key = egui::Key::ArrowLeft;
/// Right arrow: a step forward.
pub(crate) const FORWARD_KEY: egui::Key = egui::Key::ArrowRight;
/// Home: the first step.
pub(crate) const START_KEY: egui::Key = egui::Key::Home;
/// End: the view at the end of the history (`view.scrub` with null). The
/// view only: unlike the "To end" button it leaves the rollback marker
/// where it is, for no bare key edits the document.
pub(crate) const END_KEY: egui::Key = egui::Key::End;

/// What the player keeps between frames besides its settings.
#[derive(Default)]
pub(crate) struct Player {
    /// Playing at the last tick: a start is the change from not playing.
    was_playing: bool,
    /// The step asked for whose scene is not shown yet.
    waiting: Option<usize>,
    /// When the step shown had been drawn, in egui's input time.
    since: Option<f64>,
    /// The step asked for was reached in this frame's sync: the frame that
    /// draws it is still being made, so the dwell starts at the next one
    /// (whose input time is after this frame, however long it took).
    reached: bool,
    /// The input time of the last frame that turned the camera.
    turned: Option<f64>,
    /// The document's revision at the last sync: another one is an edit.
    revision: Option<Option<u64>>,
    /// The input time of the last moment of the motion asked for.
    moved: Option<f64>,
    /// The motion runs from 1 back to 0 (back and forth).
    backwards: bool,
}

/// What the motion does next ([`motion_step`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum MotionStep {
    /// Shows this `t`, going backwards when the flag says so.
    Go(f64, bool),
    /// At an end without loop: stops.
    Stop,
}

/// The step of the motion after `seconds` at `t`: `t` runs from 0 to 1 in
/// [`MOTION_SECONDS`] / `speed`. At 1 it starts again at 0 with loop, or
/// turns back with bounce (and at 0 again turns forward with loop); at an
/// end it would pass otherwise it stops there.
pub(crate) fn motion_step(t: f64, backwards: bool, seconds: f64, play: Playback) -> MotionStep {
    let delta = seconds.max(0.0) * play.speed / MOTION_SECONDS;
    let t = t.clamp(0.0, 1.0);
    if !backwards {
        if t >= 1.0 {
            return match (play.bounce, play.looping) {
                (true, _) => MotionStep::Go((1.0 - delta).max(0.0), true),
                (false, true) => MotionStep::Go(delta.min(1.0), false),
                (false, false) => MotionStep::Stop,
            };
        }
        MotionStep::Go((t + delta).min(1.0), false)
    } else {
        if t <= 0.0 {
            return if play.looping {
                MotionStep::Go(delta.min(1.0), false)
            } else {
                MotionStep::Stop
            };
        }
        MotionStep::Go((t - delta).max(0.0), true)
    }
}

/// What the player does at a tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Tick {
    /// Not playing.
    Idle,
    /// Waits for the scene of the step asked for (`None`), or so many
    /// seconds more on the step shown.
    Wait(Option<f64>),
    /// Asks for this step.
    Go(usize),
    /// At the end without loop, or nothing to play: stops.
    Stop,
}

impl Player {
    /// Forgets where it was: not playing.
    pub(crate) fn reset(&mut self) {
        self.was_playing = false;
        self.waiting = None;
        self.since = None;
        self.reached = false;
        self.turned = None;
        self.moved = None;
        self.backwards = false;
    }

    /// No scrub waits to be run and the view's scene is built: the step
    /// asked for is reached. Whichever step the view settled on counts (an
    /// agent's `view.scrub` or a flight's may have landed instead of the
    /// player's): the player goes on from there, a whole dwell later.
    pub(crate) fn settled(&mut self) {
        if self.waiting.take().is_some() {
            self.since = None;
            self.reached = true;
        }
    }

    /// A step was asked for by hand while playing: the player waits for
    /// it and shows it for a whole dwell before it goes on.
    pub(crate) fn went_to(&mut self, step: usize) {
        self.waiting = Some(step);
        self.since = None;
        self.reached = false;
    }

    /// One tick at input time `now`, with the view at step `shown` of
    /// `steps`.
    pub(crate) fn tick(&mut self, play: Playback, now: f64, shown: usize, steps: usize) -> Tick {
        if !play.playing {
            self.reset();
            return Tick::Idle;
        }
        if steps == 0 {
            return Tick::Stop;
        }
        let starting = !self.was_playing;
        self.was_playing = true;
        if self.waiting.is_some() {
            return Tick::Wait(None);
        }
        // Play at the last step: from the first again, as a video does.
        if starting && shown >= steps {
            self.went_to(1);
            return Tick::Go(1);
        }
        if std::mem::take(&mut self.reached) {
            // This frame draws the step: the dwell starts at the next.
            return Tick::Wait(Some(0.0));
        }
        let since = *self.since.get_or_insert(now);
        let left = since + play.dwell() - now;
        if left > 0.0 {
            return Tick::Wait(Some(left));
        }
        let next = if shown >= steps {
            if !play.looping {
                return Tick::Stop;
            }
            1
        } else {
            shown.saturating_add(1)
        };
        self.went_to(next);
        Tick::Go(next)
    }

    /// The camera's turn for this frame while it plays with orbit, in
    /// degrees: by the time since the last frame, at most a quarter of a
    /// second's worth (a window that was not drawn for a while does not
    /// jump).
    pub(crate) fn orbit(&mut self, play: Playback, now: f64) -> Option<f64> {
        if !(play.playing && play.orbit) {
            self.turned = None;
            return None;
        }
        let last = self.turned.replace(now).unwrap_or(now);
        let seconds = (now - last).clamp(0.0, 0.25);
        Some(ORBIT_DEGREES_PER_SECOND * play.speed * seconds)
    }
}

impl Shell {
    /// The parameters of a `view.scrub` to `step`: the end (the real
    /// marker, else the last feature) is the document as it is.
    pub(crate) fn scrub_params(&self, step: usize) -> Value {
        let end = self.timeline.rollback.unwrap_or(self.timeline.steps);
        if step == end {
            json!({"feature": null})
        } else {
            json!({"step": step})
        }
    }

    /// Shows step `step` (clamped to the history), as the slider does;
    /// while playing, the player goes on from there.
    pub(crate) fn go_to_step(&mut self, step: usize) {
        let steps = self.timeline.steps;
        if steps == 0 {
            return;
        }
        let step = step.clamp(1, steps);
        let params = self.scrub_params(step);
        self.timeline.queue("view.scrub", params);
        if self.state.timeline.playing {
            self.player.went_to(step);
        }
    }

    /// "To end": the whole history, without the rollback marker; stops
    /// playing.
    pub(crate) fn go_to_end_and_unroll(&mut self) {
        self.stop_playing();
        // A Jig in use has no marker to take away: it changes only through
        // its parameters (the command would be refused).
        if self.timeline.rollback.is_some() && !self.jig_in_use() {
            self.timeline
                .queue("history.rollback", json!({"feature": null}));
        }
        self.timeline.queue("view.scrub", json!({"feature": null}));
    }

    /// The view at the end of the history, without touching the document
    /// (the End key): stops playing.
    pub(crate) fn view_end(&mut self) {
        self.stop_playing();
        self.timeline.queue("view.scrub", json!({"feature": null}));
    }

    /// Whether "To end" has anything to do.
    pub(crate) fn at_end(&self) -> bool {
        self.timeline.position.is_none() && self.timeline.rollback.is_none()
    }

    /// Whether the active document has a step to play.
    pub(crate) fn has_steps(&self) -> bool {
        self.timeline.steps > 0
    }

    pub(crate) fn stop_playing(&mut self) {
        self.state.timeline.playing = false;
        self.player.reset();
    }

    pub(crate) fn toggle_playing(&mut self) {
        if self.state.timeline.playing {
            self.stop_playing();
        } else if self.playing_motion() || (!self.state.timeline.motion && self.timeline.steps > 0)
        {
            self.state.timeline.playing = true;
        }
    }

    /// Whether the player plays the motion: asked for, and the document
    /// has a timeline.
    pub(crate) fn playing_motion(&self) -> bool {
        self.state.timeline.motion && self.timeline.has_timeline
    }

    /// Shows the moment `t` of the motion (clamped to 0 to 1).
    pub(crate) fn go_to_t(&mut self, t: f64) {
        let t = if t.is_finite() {
            t.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let collisions = self.state.timeline.collisions;
        self.timeline
            .queue("view.motion", json!({"t": t, "collisions": collisions}));
    }

    /// One frame of the motion while it plays: the next moment once the
    /// one asked for is shown, by the time since the last.
    fn advance_motion(&mut self, ctx: &egui::Context, now: f64) {
        let play = self.state.timeline;
        if !self.timeline.has_timeline {
            self.stop_playing();
            return;
        }
        if self.timeline.motion_waiting() {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(1.0 / ORBIT_FPS));
            return;
        }
        let Some(t) = self.timeline.motion_t() else {
            self.player.moved = Some(now);
            self.go_to_t(0.0);
            ctx.request_repaint();
            return;
        };
        let starting = self.player.moved.is_none();
        let last = self.player.moved.replace(now).unwrap_or(now);
        // A window that was not drawn for a while does not jump.
        let seconds = (now - last).clamp(0.0, 0.25);
        // Play at the end: from the start again, as a video does.
        let step = if starting && t >= 1.0 && !play.bounce {
            MotionStep::Go(0.0, false)
        } else {
            motion_step(t, self.player.backwards, seconds, play)
        };
        match step {
            MotionStep::Go(next, backwards) => {
                self.player.backwards = backwards;
                if next != t {
                    self.go_to_t(next);
                }
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(1.0 / ORBIT_FPS));
            }
            MotionStep::Stop => self.stop_playing(),
        }
    }

    /// The motion's own row over the player: History or Motion, the
    /// slider over `t`, back and forth, collisions, and what is shown.
    pub(crate) fn motion_ui(&mut self, ui: &mut egui::Ui) {
        if !self.timeline.has_timeline {
            return;
        }
        let tokens = self.tokens;
        let motion = self.state.timeline.motion;
        ui.horizontal(|ui| {
            if ui
                .selectable_label(!motion, "History")
                .on_hover_text("The player plays the history, step by step")
                .clicked()
                && motion
            {
                self.stop_playing();
                self.state.timeline.motion = false;
                self.timeline.queue("view.motion", json!({"t": null}));
            }
            if ui
                .selectable_label(motion, "Motion")
                .on_hover_text(
                    "The player plays the motion: the timeline t runs from 0 to 1, \
                     driving the formulas that use it. The document does not change.",
                )
                .clicked()
                && !motion
            {
                self.stop_playing();
                self.state.timeline.motion = true;
                let t = self.timeline.motion_t().unwrap_or(0.0);
                self.go_to_t(t);
            }
        });
        if !motion {
            return;
        }
        let shown = self.timeline.motion_t().unwrap_or(0.0);
        let mut t = shown;
        ui.horizontal(|ui| {
            ui.spacing_mut().slider_width = (ui.available_width() - 8.0).max(40.0);
            ui.add(
                egui::Slider::new(&mut t, 0.0..=1.0)
                    .show_value(false)
                    .trailing_fill(true),
            )
            .on_hover_text("Drag through the motion. The document does not change.");
        });
        if t != shown {
            self.go_to_t(t);
            ui.ctx().request_repaint();
        }
        let collisions_on = self.state.timeline.collisions;
        ui.horizontal(|ui| {
            if ui
                .selectable_label(self.state.timeline.bounce, "Back and forth")
                .on_hover_text("Play from 0 to 1 and back again")
                .clicked()
            {
                self.state.timeline.bounce = !self.state.timeline.bounce;
            }
            if ui
                .selectable_label(collisions_on, "Collisions")
                .on_hover_text("Draw the bodies that overlap at this t in red")
                .clicked()
            {
                self.state.timeline.collisions = !collisions_on;
                self.go_to_t(shown);
            }
        });
        let pairs = self
            .timeline
            .motion
            .as_ref()
            .map_or(0, |m| m.collisions.len());
        let text = if pairs > 0 {
            format!("t = {shown:.3}  ·  {pairs} colliding")
        } else {
            format!("t = {shown:.3}")
        };
        let ink = if pairs > 0 {
            tokens.failed
        } else {
            tokens.attention
        };
        ui.label(egui::RichText::new(text).color(color(ink)));
    }

    /// After a sync with the session: an edit of the document stops
    /// playing, and the step the view shows has its scene once no scrub
    /// waits to be run.
    pub(crate) fn sync_player(&mut self, session: &Session) {
        let revision = session.document_revision();
        if self.player.revision.is_some_and(|seen| seen != revision) {
            self.stop_playing();
        }
        self.player.revision = Some(revision);
        if !self.timeline.scrub_waiting() {
            self.player.settled();
        }
    }

    /// Another tab is shown: playing stops (the step shown is the other
    /// document's).
    pub(crate) fn player_switched(&mut self) {
        self.stop_playing();
        self.player.revision = None;
    }

    /// One frame of the player: the next step when it is time, the orbit,
    /// and the frame to come.
    pub(crate) fn advance_player(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let play = self.state.timeline;
        let tick = if play.motion && play.playing {
            self.advance_motion(ctx, now);
            Tick::Idle
        } else {
            self.player
                .tick(play, now, self.timeline.shown_step(), self.timeline.steps)
        };
        match tick {
            Tick::Idle | Tick::Wait(None) => {}
            Tick::Wait(Some(seconds)) => {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(
                    seconds.clamp(0.0, 60.0),
                ));
            }
            Tick::Go(step) => {
                let params = self.scrub_params(step);
                self.timeline.queue("view.scrub", params);
                ctx.request_repaint();
            }
            Tick::Stop => self.stop_playing(),
        }
        self.orbiting = false;
        // A turn or a flight of the camera, or a drag, has the camera to
        // itself: the orbit waits (and does not jump afterwards).
        let busy = self.turn.is_some() || self.drag_from.is_some() || self.cube_drag_from.is_some();
        let degrees = self.player.orbit(self.state.timeline, now);
        if let Some(degrees) = degrees {
            if !busy {
                let camera = &mut self.state.camera;
                camera.yaw = (camera.yaw + degrees + 180.0).rem_euclid(360.0) - 180.0;
                // A slow turn is drawn at full quality; only a large step
                // per frame is drawn as a moving camera.
                self.orbiting = degrees > ORBIT_MOVING_DEGREES;
            }
            // At most ORBIT_FPS frames a second, not as fast as egui can.
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(1.0 / ORBIT_FPS));
        }
    }

    /// The player's keys, while the pointer is over the viewport or the
    /// feature panel and no text field has the keyboard.
    pub(crate) fn player_keys(&mut self, ctx: &egui::Context) {
        // Not under a dialog: the closing question or a Help window.
        let modal =
            self.closing.is_some() || self.state.dialogs.about || self.state.dialogs.shortcuts;
        if self.timeline.steps == 0 || modal || ctx.egui_wants_keyboard_input() {
            return;
        }
        let over = ctx.pointer_hover_pos().is_some_and(|at| {
            self.rect.is_some_and(|r| r.contains(at))
                || self.tree_panel.is_some_and(|r| r.contains(at))
        });
        if !over {
            return;
        }
        let plain = |key| ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, key));
        let shown = self.timeline.shown_step();
        if plain(PLAY_KEY) {
            self.toggle_playing();
        } else if plain(BACK_KEY) {
            self.go_to_step(shown.saturating_sub(1).max(1));
        } else if plain(FORWARD_KEY) {
            self.go_to_step(shown.saturating_add(1));
        } else if plain(START_KEY) {
            self.go_to_step(1);
        } else if plain(END_KEY) {
            self.view_end();
        } else {
            return;
        }
        ctx.request_repaint();
    }

    /// The caption over the viewport while the view is scrubbed or
    /// playing: `Step 3/10 · Plate · block`.
    pub(crate) fn player_caption(&self) -> Option<String> {
        if let Some(motion) = &self.timeline.motion {
            let mut text = format!("t = {:.3}", motion.t);
            if !motion.collisions.is_empty() {
                let pairs: Vec<String> = motion
                    .collisions
                    .iter()
                    .map(|(a, b)| format!("{a} and {b}"))
                    .collect();
                text.push_str(&format!(" · collision: {}", pairs.join(", ")));
            }
            return Some(text);
        }
        let steps = self.timeline.steps;
        if steps == 0 || (self.timeline.position.is_none() && !self.state.timeline.playing) {
            return None;
        }
        let step = self.timeline.shown_step().clamp(1, steps);
        let row = self.rows.get(step.saturating_sub(1))?;
        Some(craft3d_engine::step_caption(
            step, steps, &row.name, row.kind,
        ))
    }

    /// The player's buttons under the timeline's slider: the five that
    /// move through the history, then speed, loop and orbit. One row
    /// where the panel is wide enough, else two rows, each centred
    /// ([`player_rows`]), so no button is left alone on a row of its own.
    pub(crate) fn player_controls(&mut self, ui: &mut egui::Ui) {
        let rows = player_rows(ui.available_width(), button_side(ui));
        let row = |ui: &mut egui::Ui, pad: f32, add: &mut dyn FnMut(&mut egui::Ui)| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = PLAYER_GAP;
                if pad > 0.0 {
                    ui.add_space(pad);
                }
                add(ui);
            });
        };
        match rows {
            PlayerRows::One => row(ui, 0.0, &mut |ui| {
                self.transport_buttons(ui);
                ui.add_space(PLAYER_GROUP_GAP);
                self.setting_buttons(ui);
            }),
            PlayerRows::Two {
                transport,
                settings,
            } => {
                row(ui, transport, &mut |ui| self.transport_buttons(ui));
                row(ui, settings, &mut |ui| self.setting_buttons(ui));
            }
        }
    }

    /// First step, step back, play or pause, step forward, to end.
    fn transport_buttons(&mut self, ui: &mut egui::Ui) {
        let tokens = self.tokens;
        let steps = self.timeline.steps;
        let shown = self.timeline.shown_step().clamp(1, steps.max(1));
        let play = self.state.timeline;
        let at_end = self.at_end();
        if icon_button(ui, Icon::Start, "First step", false, shown > 1, &tokens)
            .on_hover_text("First step (Home)")
            .clicked()
        {
            self.go_to_step(1);
        }
        if icon_button(ui, Icon::StepBack, "Step back", false, shown > 1, &tokens)
            .on_hover_text("Step back (←)")
            .clicked()
        {
            self.go_to_step(shown.saturating_sub(1));
        }
        let (icon, tip) = if play.playing {
            (Icon::Pause, "Pause (Space)")
        } else {
            (
                Icon::Play,
                "Play the history step by step (Space). Each step waits until its model is built.",
            )
        };
        let name = if play.playing { "Pause" } else { "Play" };
        if icon_button(ui, icon, name, false, true, &tokens)
            .on_hover_text(tip)
            .clicked()
        {
            self.toggle_playing();
        }
        if icon_button(
            ui,
            Icon::StepForward,
            "Step forward",
            false,
            shown < steps,
            &tokens,
        )
        .on_hover_text("Step forward (→)")
        .clicked()
        {
            self.go_to_step(shown.saturating_add(1));
        }
        if icon_button(ui, Icon::End, "To end", false, !at_end, &tokens)
            .on_hover_text(
                "To end: show the whole history again and remove the rollback marker \
                 (an edit of the document, undone with Undo). The End key shows the end \
                 in the view only and leaves the marker.",
            )
            .clicked()
        {
            self.go_to_end_and_unroll();
        }
    }

    /// Speed, loop and orbit.
    fn setting_buttons(&mut self, ui: &mut egui::Ui) {
        let tokens = self.tokens;
        let play = self.state.timeline;
        let speed = speed_text(play.speed);
        if ui
            .add(egui::Button::new(speed).min_size(vec2(SPEED_WIDTH, button_side(ui))))
            .on_hover_text("Playback speed: 0.5×, 1× or 2×")
            .clicked()
        {
            self.state.timeline.speed = play.next_speed();
        }
        if icon_button(ui, Icon::Loop, "Loop", play.looping, true, &tokens)
            .on_hover_text("Loop: start again at the first step after the last")
            .clicked()
        {
            self.state.timeline.looping = !play.looping;
        }
        if icon_button(ui, Icon::Orbit, "Orbit", play.orbit, true, &tokens)
            .on_hover_text("Orbit: turn the camera slowly while it plays")
            .clicked()
        {
            self.state.timeline.orbit = !play.orbit;
        }
    }
}

/// The space between two of the player's buttons, in points.
const PLAYER_GAP: f32 = 2.0;
/// The extra space between the five that move and the settings.
const PLAYER_GROUP_GAP: f32 = 4.0;
/// The speed button's width, the same for 0.5×, 1× and 2× (wide enough
/// for each), so the rows are measured before they are drawn.
const SPEED_WIDTH: f32 = 34.0;

/// How the player's buttons are laid out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum PlayerRows {
    /// All eight in one row.
    One,
    /// The five that move on the first row, speed, loop and orbit on the
    /// second, each row moved right by this much to be centred.
    Two { transport: f32, settings: f32 },
}

/// The widths of the two groups of buttons for icon buttons of `side`
/// points: the five that move, and speed, loop and orbit.
pub(crate) fn player_group_widths(side: f32) -> (f32, f32) {
    let icon = side + 4.0;
    (
        5.0 * icon + 4.0 * PLAYER_GAP,
        SPEED_WIDTH + 2.0 * icon + 2.0 * PLAYER_GAP,
    )
}

/// One row when `available` points hold all eight buttons, else two,
/// centred.
pub(crate) fn player_rows(available: f32, side: f32) -> PlayerRows {
    let (transport, settings) = player_group_widths(side);
    if available >= transport + PLAYER_GAP + PLAYER_GROUP_GAP + settings {
        PlayerRows::One
    } else {
        let centred = |width: f32| ((available - width) * 0.5).max(0.0);
        PlayerRows::Two {
            transport: centred(transport),
            settings: centred(settings),
        }
    }
}

/// `0.5×`, `1×`, `2×`.
pub(crate) fn speed_text(speed: f64) -> String {
    format!("{speed}×")
}

/// The height of the player's buttons, in points.
fn button_side(ui: &egui::Ui) -> f32 {
    (ui.spacing().interact_size.y + 6.0).max(22.0)
}

/// A small button with a painted icon, called `name` for screen readers
/// and tests; `on` marks a toggle that is on (Loop, Orbit: they say
/// whether they are selected).
fn icon_button(
    ui: &mut egui::Ui,
    icon: Icon,
    name: &str,
    on: bool,
    enabled: bool,
    tokens: &Tokens,
) -> egui::Response {
    let side = button_side(ui);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(side + 4.0, side), sense);
    let painter = ui.painter();
    let round = egui::CornerRadius::same(tokens.radius);
    if on {
        painter.rect_filled(rect, round, color(tokens.selection_fill));
    } else if enabled && response.hovered() {
        painter.rect_filled(rect, round, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    let ink = if !enabled {
        color(tokens.inactive)
    } else if response.hovered() || on {
        color(tokens.text)
    } else {
        color(tokens.text_dim)
    };
    icons::paint(painter, icon, rect.shrink(side * 0.17), ink, tokens);
    let toggle = matches!(icon, Icon::Loop | Icon::Orbit);
    response.widget_info(|| {
        if toggle {
            egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, on, name)
        } else {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, name)
        }
    });
    response
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::{Player, PlayerRows, Tick, player_group_widths, player_rows};
    use crate::state::Playback;
    use serde_json::json;

    fn playing() -> Playback {
        Playback {
            playing: true,
            ..Playback::default()
        }
    }

    #[test]
    fn the_motion_runs_on_loops_turns_back_and_stops() {
        use super::{MotionStep, motion_step};
        let play = playing();
        // Speed 1: 4 s from 0 to 1, so a second moves t by a quarter.
        assert_eq!(
            motion_step(0.0, false, 1.0, play),
            MotionStep::Go(0.25, false)
        );
        assert_eq!(
            motion_step(0.9, false, 1.0, play),
            MotionStep::Go(1.0, false)
        );
        assert_eq!(motion_step(1.0, false, 1.0, play), MotionStep::Stop);
        let looping = Playback {
            looping: true,
            ..play
        };
        assert_eq!(
            motion_step(1.0, false, 1.0, looping),
            MotionStep::Go(0.25, false)
        );
        let bounce = Playback {
            bounce: true,
            speed: 2.0,
            ..play
        };
        assert_eq!(
            motion_step(1.0, false, 1.0, bounce),
            MotionStep::Go(0.5, true)
        );
        assert_eq!(
            motion_step(0.5, true, 1.0, bounce),
            MotionStep::Go(0.0, true)
        );
        assert_eq!(motion_step(0.0, true, 1.0, bounce), MotionStep::Stop);
        let both = Playback {
            looping: true,
            ..bounce
        };
        assert_eq!(
            motion_step(0.0, true, 0.5, both),
            MotionStep::Go(0.25, false)
        );
        // Time does not run backwards, and t stays within 0 and 1.
        assert_eq!(
            motion_step(0.5, false, -3.0, play),
            MotionStep::Go(0.5, false)
        );
        assert_eq!(motion_step(7.0, false, 0.0, play), MotionStep::Stop);
    }

    #[test]
    fn it_waits_for_the_scene_and_the_dwell_then_goes_on() {
        let mut p = Player::default();
        let play = playing();
        // Started at step 1 of 3: shown for a dwell first.
        assert!(matches!(p.tick(play, 10.0, 1, 3), Tick::Wait(Some(_))));
        assert!(matches!(p.tick(play, 10.5, 1, 3), Tick::Wait(Some(_))));
        assert_eq!(p.tick(play, 10.81, 1, 3), Tick::Go(2));
        // The scene of step 2 is not there: it waits, however long.
        assert_eq!(p.tick(play, 11.0, 1, 3), Tick::Wait(None));
        assert_eq!(p.tick(play, 99.0, 2, 3), Tick::Wait(None));
        // Built: the frame that draws it does not count (it may have been
        // a slow one); the dwell runs from the frame after it.
        p.settled();
        assert_eq!(p.tick(play, 100.0, 2, 3), Tick::Wait(Some(0.0)));
        assert!(matches!(p.tick(play, 100.5, 2, 3), Tick::Wait(Some(_))));
        assert!(matches!(p.tick(play, 101.2, 2, 3), Tick::Wait(Some(_))));
        assert_eq!(p.tick(play, 101.31, 2, 3), Tick::Go(3));
        p.settled();
        assert_eq!(p.tick(play, 102.0, 3, 3), Tick::Wait(Some(0.0)));
        assert!(matches!(p.tick(play, 102.1, 3, 3), Tick::Wait(Some(_))));
        // At the end without loop: it stops.
        assert_eq!(p.tick(play, 103.0, 3, 3), Tick::Stop);
        // Settling with nothing asked for changes nothing.
        p.settled();
        assert_eq!(p.tick(play, 104.0, 3, 3), Tick::Stop);
    }

    #[test]
    fn the_buttons_take_one_row_or_two_centred_never_one_alone() {
        let side = 24.0;
        let (transport, settings) = player_group_widths(side);
        assert!(settings < transport);
        // Wide: one row.
        assert_eq!(player_rows(400.0, side), PlayerRows::One);
        assert_eq!(
            player_rows(transport + settings + 6.0, side),
            PlayerRows::One
        );
        // From just too narrow for one row down to the panel's least
        // width: two rows, both inside the width and centred.
        let least = super::super::TREE_PANEL_MIN - 2.0 * 10.0;
        assert!(least >= transport, "{least} < {transport}");
        let mut width = transport + settings + 5.9;
        while width >= least {
            match player_rows(width, side) {
                PlayerRows::Two {
                    transport: a,
                    settings: b,
                } => {
                    assert!(a >= 0.0 && a + transport <= width + 1e-3, "{width}");
                    assert!(b >= 0.0 && b + settings <= width + 1e-3, "{width}");
                    assert!((a + transport * 0.5 - width * 0.5).abs() < 1e-3);
                    assert!((b + settings * 0.5 - width * 0.5).abs() < 1e-3);
                }
                PlayerRows::One => panic!("one row at {width}"),
            }
            width -= 1.0;
        }
        // Narrower than a row (never, with the panel's least width): no
        // negative padding.
        assert_eq!(
            player_rows(50.0, side),
            PlayerRows::Two {
                transport: 0.0,
                settings: 0.0
            }
        );
    }

    #[test]
    fn speed_loop_and_a_start_at_the_end() {
        let mut p = Player::default();
        let fast = Playback {
            speed: 2.0,
            looping: true,
            ..playing()
        };
        // Play at the end: from the first step again, at once.
        assert_eq!(p.tick(fast, 0.0, 3, 3), Tick::Go(1));
        p.settled();
        p.tick(fast, 1.0, 1, 3);
        assert!(matches!(p.tick(fast, 1.0, 1, 3), Tick::Wait(Some(_))));
        // Twice as fast: 0.4 s.
        assert_eq!(p.tick(fast, 1.41, 1, 3), Tick::Go(2));
        p.settled();
        p.tick(fast, 2.0, 2, 3);
        p.tick(fast, 2.0, 2, 3);
        assert_eq!(p.tick(fast, 2.41, 2, 3), Tick::Go(3));
        p.settled();
        p.tick(fast, 3.0, 3, 3);
        p.tick(fast, 3.0, 3, 3);
        // Loop: after the last, the first.
        assert_eq!(p.tick(fast, 3.41, 3, 3), Tick::Go(1));
        // A pause forgets; nothing to play stops.
        let paused = Playback {
            playing: false,
            ..fast
        };
        assert_eq!(p.tick(paused, 4.0, 1, 3), Tick::Idle);
        assert_eq!(p.tick(fast, 4.0, 1, 0), Tick::Stop);
        // Half speed: 1.6 s.
        let mut p = Player::default();
        let slow = Playback {
            speed: 0.5,
            ..playing()
        };
        p.tick(slow, 0.0, 1, 3);
        assert!(matches!(p.tick(slow, 1.5, 1, 3), Tick::Wait(Some(_))));
        assert_eq!(p.tick(slow, 1.61, 1, 3), Tick::Go(2));
    }

    #[test]
    fn the_orbit_turns_by_the_time_between_frames() {
        let mut p = Player::default();
        let play = Playback {
            orbit: true,
            ..playing()
        };
        let near = |got: Option<f64>, want: f64| (got.unwrap() - want).abs() < 1e-9;
        assert!(near(p.orbit(play, 1.0), 0.0));
        // A long pause between frames counts as a quarter of a second.
        assert!(near(
            p.orbit(play, 1.5),
            super::ORBIT_DEGREES_PER_SECOND * 0.25
        ));
        assert!(near(
            p.orbit(play, 1.6),
            super::ORBIT_DEGREES_PER_SECOND * 0.1
        ));
        assert_eq!(p.orbit(Playback::default(), 2.0), None);
    }

    /// A box, a ball and a rod in a shell, played from the end with loop.
    struct Played {
        shell: crate::Shell,
        session: craft3d_engine::Session,
        ctx: egui::Context,
        time: f64,
    }

    impl Played {
        fn new(play: Playback) -> Self {
            let mut session = craft3d_engine::Session::new(craft3d_engine::Workspace::none());
            for (id, params) in [
                ("file.new", json!({})),
                (
                    "feature.block",
                    json!({"size": [10, 10, 10], "name": "Box"}),
                ),
                ("feature.sphere", json!({"radius": 3, "name": "Ball"})),
                (
                    "feature.cylinder",
                    json!({"radius": 1, "height": 30, "name": "Rod"}),
                ),
            ] {
                session.execute(id, &params).unwrap();
            }
            let mut played = Self {
                shell: crate::Shell::new(),
                session,
                ctx: egui::Context::default(),
                time: 0.0,
            };
            played.frame();
            played.shell.state.timeline = play;
            played
        }

        fn frame(&mut self) {
            self.time += 0.05;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 700.0),
                )),
                time: Some(self.time),
                ..Default::default()
            };
            let Self { shell, session, .. } = self;
            let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
            output.textures_delta.clear();
        }

        fn shown(&self) -> usize {
            self.shell.timeline.shown_step()
        }

        /// Frames until `done`, at most `seconds`; true when it came.
        fn until(&mut self, seconds: f64, done: impl Fn(&Self) -> bool) -> bool {
            let end = self.time + seconds;
            while self.time < end {
                self.frame();
                if done(self) {
                    return true;
                }
            }
            false
        }
    }

    fn looping() -> Playback {
        Playback {
            looping: true,
            ..playing()
        }
    }

    #[test]
    fn another_scrub_landing_first_does_not_hang_the_player() {
        // An agent's view.scrub runs after the player's, before its scene
        // was seen: the player goes on from where the view settled.
        let mut p = Played::new(looping());
        assert!(p.until(2.0, |p| p.shown() == 1));
        p.shell.player.went_to(2);
        p.session
            .execute("view.scrub", &json!({"step": 3}))
            .unwrap();
        assert!(p.until(0.2, |p| p.shown() == 3));
        assert!(p.until(3.0, |p| p.shown() == 1), "the player hung");
        assert!(p.shell.state.timeline.playing);

        // A flight's scrub (to the end, to show its feature) replaces the
        // player's while it waits to be run.
        let mut p = Played::new(looping());
        assert!(p.until(2.0, |p| p.shown() == 1));
        p.shell.timeline.queue("view.scrub", json!({"step": 2}));
        p.shell.player.went_to(2);
        p.shell
            .request_fly(super::super::FlyTo::Feature("Rod".to_string()));
        assert!(p.until(1.0, |p| p.shell.timeline.position.is_none()));
        assert!(p.until(3.0, |p| p.shown() == 1), "the player hung");
        assert!(p.shell.state.timeline.playing);
    }

    #[test]
    fn a_view_cube_turn_while_orbiting_reaches_its_target() {
        let mut p = Played::new(Playback {
            orbit: true,
            ..looping()
        });
        p.frame();
        p.frame();
        let from = p.shell.state.camera;
        let mut to = from;
        to.set_view("top").unwrap();
        p.shell.turn = Some(super::super::Turn {
            from,
            to,
            started: p.time,
            last: from,
            seconds: 0.25,
        });
        assert!(p.until(1.0, |p| p.shell.turn.is_none()));
        assert_eq!(p.shell.state.camera, to, "the turn was cut short");
        // Then the orbit goes on.
        p.frame();
        p.frame();
        assert_ne!(p.shell.state.camera.yaw, to.yaw);
    }
}
