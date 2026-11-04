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
//! at the first with `loop`; an edit of the document, another tab, End
//! stops it, and a failed scrub too. With orbit
//! the camera of the tab turns slowly about the vertical while it plays
//! (at most 30 frames a second, sharp unless a frame turns it more than a
//! degree), and waits while a turn, a flight or a drag has the camera.
//!
//! While the view is scrubbed or playing, the viewport shows the step's
//! caption, the one `view.replay` writes ([`craft3d_engine::step_caption`]).

use super::Shell;
use super::icons::{self, Icon};
use crate::kit::{ChipKind, Glyph, Kit, Segment};
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
/// view only: it leaves the rollback marker where it is, for no bare key
/// edits the document (the roll-back bar's *Roll forward to end* does).
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
    /// Where the motion's clock is while it plays: `t` before it is
    /// rounded to the frames of a bake ([`rounded_to_frames`]), so a play
    /// at frame rate goes on at its speed whatever the rounding.
    clock: Option<f64>,
    /// The bake hint: what the moments cost, the bake it asked for, the
    /// documents it was dismissed for (`bake_hint.rs`). Kept when playing
    /// stops: the hint stays while the motion is paused.
    pub(crate) hint: super::bake_hint::BakeHint,
}

/// The studio look drawn while a motion plays (`timeline.playQuality`
/// auto, [`Shell::playing_light`]): one frame instead of the frames a
/// still picture accumulates, no depth of field (whose blur needs them),
/// and none of the screen-space passes that cost the most per frame on a
/// laptop's GPU (ambient occlusion, the reflections in glossy faces, the
/// contact shadows and the key's shadow on the reflections: together about
/// two thirds of a light frame of the router; docs/design/motion.md ›
/// *Findings*, 2026-11-02). The light, the key's shadows, the ground and
/// its reflection, bloom and the tone mapping stay, so a moving part keeps
/// its shading and the shadow that grounds it; once the motion stops the
/// full look is drawn again and settles.
pub(crate) fn light_look(look: craft3d_render::studio::Look) -> craft3d_render::studio::Look {
    craft3d_render::studio::Look {
        accumulate: 1,
        dof: None,
        ao: false,
        ssr: false,
        contact_shadows: false,
        specular_shadow: false,
        ..look
    }
}

/// `t` rounded to the nearest frame `k / frames` of a bake: a baked
/// motion played so asks for nothing that is not made ahead
/// (docs/design/motion.md › *The player*). Unchanged without a bake, or
/// when `t` is not a number.
pub(crate) fn rounded_to_frames(t: f64, frames: Option<u64>) -> f64 {
    match frames {
        Some(n) if n > 0 && t.is_finite() => {
            let n = n as f64;
            ((t.clamp(0.0, 1.0) * n).round() / n).clamp(0.0, 1.0)
        }
        _ => t,
    }
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
        self.clock = None;
        self.hint.not_weighed();
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

    /// The view at the end of the history, without touching the document
    /// (the End key): stops playing.
    pub(crate) fn view_end(&mut self) {
        self.stop_playing();
        self.timeline.queue("view.scrub", json!({"feature": null}));
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
        } else if self.playing_motion() || self.timeline.steps > 0 {
            self.state.timeline.playing = true;
        } else {
            // Never a silent play that does nothing.
            self.timeline.problem = Some(
                "Nothing to play: the document has no features and no motion timeline (a variable t)"
                    .to_string(),
            );
        }
    }

    /// Whether the player plays the motion: asked for, and the document
    /// has a timeline.
    ///
    /// The mode the player plays in follows the document while the
    /// person's choice is kept: a document without a motion timeline plays
    /// its history whatever was chosen (a `t` broken for a moment, or
    /// another document after an assembly, does not lose the choice), and
    /// one with a timeline and no history plays its motion.
    pub(crate) fn playing_motion(&self) -> bool {
        self.timeline.has_timeline && (self.state.timeline.motion || self.timeline.steps == 0)
    }

    /// Shows the moment `t` of the motion (clamped to 0 to 1).
    pub(crate) fn go_to_t(&mut self, t: f64) {
        let t = if t.is_finite() {
            t.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let collisions = self.state.timeline.collisions;
        // At once with what is built, the rest built in the background:
        // the window never waits for a configuration
        // (docs/design/motion.md › *Shapes in the background*). Recorded
        // frame by frame or captured, each moment is the exact one, so the
        // frames do not depend on how far the builder got.
        let shapes = if self.exact_pictures() {
            "exact"
        } else {
            "ready"
        };
        self.timeline.queue(
            "view.motion",
            json!({"t": t, "collisions": collisions, "shapes": shapes}),
        );
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
        // The moment asked for is shown: what it cost is weighed for the
        // bake hint (`bake_hint.rs`), unless the window is recorded or
        // captured, whose clock is not the wall's. A moment built as it
        // plays (shown provisionally while its configurations are built)
        // is baked by itself once it is known not to be heavy; a heavy
        // one the hint offers.
        if self.exact_pictures() || self.recording() {
            self.player.hint.not_weighed();
        } else {
            let provisional = self
                .timeline
                .motion
                .as_ref()
                .is_some_and(|m| !m.exact || !m.stale.is_empty());
            self.player.hint.shown(provisional);
            self.auto_bake();
        }
        let Some(shown) = self.timeline.motion_t() else {
            self.player.moved = Some(now);
            self.player.clock = Some(0.0);
            self.go_to_t(0.0);
            self.ask_weighed();
            ctx.request_repaint();
            return;
        };
        let frames = self.timeline.motion.as_ref().and_then(|m| m.baked);
        // The clock goes on from where it was, unless something else moved
        // the moment shown (the slider, an agent).
        let t = match self.player.clock {
            Some(clock) if rounded_to_frames(clock, frames) == shown => clock,
            _ => shown,
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
                self.player.clock = Some(next);
                let asked = rounded_to_frames(next, frames);
                if asked != shown {
                    self.go_to_t(asked);
                    self.ask_weighed();
                }
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(1.0 / ORBIT_FPS));
            }
            MotionStep::Stop => self.stop_playing(),
        }
    }

    /// The playing player asked for a moment: its `view.motion` is timed
    /// for the bake hint, unless the window is recorded or captured (its
    /// moments are made exactly, not as the play makes them).
    fn ask_weighed(&mut self) {
        if self.exact_pictures() || self.recording() {
            self.player.hint.not_weighed();
        } else {
            self.player.hint.asked();
        }
    }

    /// The motion's own row over the player: History or Motion, the
    /// slider over `t`, back and forth, collisions, and what is shown.
    /// What the motion adds under the player's row: Back and forth and
    /// Collisions as chips, the colliding pairs, and the bake hint.
    pub(crate) fn motion_extras(&mut self, ui: &mut egui::Ui) {
        let tokens = self.tokens;
        let kit = Kit::panel(&tokens);
        let shown = self.timeline.motion_t().unwrap_or(0.0);
        let collisions_on = self.state.timeline.collisions;
        ui.horizontal_wrapped(|ui| {
            let kind = |on: bool| if on { ChipKind::On } else { ChipKind::Plain };
            if kit
                .chip(ui, "Back and forth", kind(self.state.timeline.bounce))
                .on_hover_text("Play from 0 to 1 and back again")
                .clicked()
            {
                self.state.timeline.bounce = !self.state.timeline.bounce;
            }
            if kit
                .chip(ui, "Collisions", kind(collisions_on))
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
        if pairs > 0 {
            ui.label(
                egui::RichText::new(format!("{pairs} colliding"))
                    .font(kit.font())
                    .color(color(tokens.failed)),
            );
        }
        // This motion is heavy — bake first? (`bake_hint.rs`)
        self.bake_hint_ui(ui);
    }

    /// History or Motion, for a document that has both (board s8).
    pub(crate) fn player_mode(&mut self, ui: &mut egui::Ui) {
        if !self.timeline.has_timeline || self.timeline.steps == 0 {
            return;
        }
        let kit = Kit::panel(&self.tokens);
        let motion = self.state.timeline.motion;
        let mut chosen = usize::from(motion);
        kit.segmented(
            ui,
            &mut chosen,
            &[Segment::Text("History"), Segment::Text("Motion")],
        )
        .response
        .on_hover_text(
            "History plays the steps of the design; Motion runs the timeline t from 0 to 1, \
                 driving the formulas that use it. The document does not change.",
        );
        if chosen == 1 && !motion {
            self.stop_playing();
            self.state.timeline.motion = true;
            let t = self.timeline.motion_t().unwrap_or(0.0);
            self.go_to_t(t);
        } else if chosen == 0 && motion {
            self.stop_playing();
            self.state.timeline.motion = false;
            self.timeline.queue("view.motion", json!({"t": null}));
        }
    }

    /// The motion's timeline: a slider over t.
    pub(crate) fn motion_slider(&mut self, ui: &mut egui::Ui) {
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
        self.player_shown = true;
    }

    /// After a sync with the session: an edit of the document stops
    /// playing, and the step the view shows has its scene once no scrub
    /// waits to be run.
    pub(crate) fn sync_player(&mut self, session: &Session) {
        let revision = session.document_revision();
        if self.player.revision.is_some_and(|seen| seen != revision) {
            self.stop_playing();
            // An edit lets go of the bake, and the motion is another.
            self.player.hint.forget();
            self.timeline.drop_pending("motion.bake");
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
        self.player.hint.forget();
    }

    /// One frame of the player: the next step when it is time, the orbit,
    /// and the frame to come.
    pub(crate) fn advance_player(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        // The bake the hint asked for: its next piece, its progress.
        if self.follow_bake() {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(1.0 / ORBIT_FPS));
        }
        let play = self.state.timeline;
        let tick = if self.playing_motion() && play.playing {
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
            let mut text = t_text(motion.t);
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

    /// The player's row under the timeline (board s8, decisions 4 and 5):
    /// step back, play or pause (the round amber button), step forward,
    /// where it is (`3 / 10`, `t 0.35`), then at the right speed, loop and
    /// orbit. No first and last: those are the timeline's ends (Home and
    /// End). What a narrow panel cannot hold is left out
    /// ([`player_parts`]): orbit under 220 points, and with touch sizes
    /// under 380 points everything after the position.
    pub(crate) fn player_controls(&mut self, ui: &mut egui::Ui) {
        let kit = Kit::panel(&self.tokens);
        self.player_row(ui, &kit);
    }

    /// The row on `kit`'s ground: the panel's, or the plate of the phone's
    /// strip over the viewport.
    fn player_row(&mut self, ui: &mut egui::Ui, kit: &Kit) {
        let parts = player_parts(ui.available_width(), kit.touch());
        let motion = self.playing_motion_mode();
        let mut hits = Vec::new();
        // Out of reach at the ends of the timeline.
        let (at_start, at_end) = if motion {
            let t = self.timeline.motion_t().unwrap_or(0.0);
            (t <= 0.0, t >= 1.0)
        } else {
            let steps = self.timeline.steps;
            let shown = self.timeline.shown_step().clamp(1, steps.max(1));
            (shown <= 1, shown >= steps)
        };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = PLAYER_GAP;
            let back = kit
                .icon_button_enabled(
                    ui,
                    Glyph::Nav(Icon::StepBack),
                    false,
                    "Step back",
                    !at_start,
                )
                .on_hover_text(if motion {
                    "Back a little (←)"
                } else {
                    "Step back (←); Home goes to the first step"
                });
            hits.push(("back", back.rect));
            if back.clicked() {
                self.step_by(-1);
            }
            let play = self.play_button(ui, kit);
            hits.push(("play", play.rect));
            let forward = kit
                .icon_button_enabled(
                    ui,
                    Glyph::Nav(Icon::StepForward),
                    false,
                    "Step forward",
                    !at_end,
                )
                .on_hover_text(if motion {
                    "Forward a little (→)"
                } else {
                    "Step forward (→); End goes to the end"
                });
            hits.push(("forward", forward.rect));
            if forward.clicked() {
                self.step_by(1);
            }
            ui.add_space(PLAYER_GROUP_GAP);
            let position = self.player_position();
            let label = ui.add(
                egui::Label::new(
                    egui::RichText::new(position)
                        .font(kit.font())
                        .color(color(kit.skin.text_dim)),
                )
                .truncate(),
            );
            hits.push(("position", label.rect));
            if parts.settings {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = PLAYER_GAP;
                    let play = self.state.timeline;
                    if parts.orbit {
                        let orbit = kit
                            .icon_button(ui, Glyph::Nav(Icon::Orbit), play.orbit, "Orbit")
                            .on_hover_text("Orbit: turn the camera slowly while it plays");
                        hits.push(("orbit", orbit.rect));
                        if orbit.clicked() {
                            self.state.timeline.orbit = !play.orbit;
                        }
                    }
                    let looping = kit
                        .icon_button(ui, Glyph::Nav(Icon::Loop), play.looping, "Loop")
                        .on_hover_text("Loop: start again at the beginning after the end");
                    hits.push(("loop", looping.rect));
                    if looping.clicked() {
                        self.state.timeline.looping = !play.looping;
                    }
                    let speed = kit
                        .chip(ui, &speed_text(play.speed), ChipKind::Plain)
                        .on_hover_text("Playback speed: 0.5×, 1× or 2×");
                    hits.push(("speed", speed.rect));
                    if speed.clicked() {
                        self.state.timeline.speed = play.next_speed();
                    }
                });
            }
        });
        self.player_hits = hits;
        self.player_shown = true;
    }

    /// The round amber button: play, or pause while it plays.
    fn play_button(&mut self, ui: &mut egui::Ui, kit: &Kit) -> egui::Response {
        let t = kit.t;
        let playing = self.state.timeline.playing;
        let side = t.control_height;
        let hit = kit.hit(side);
        let (outer, response) = ui.allocate_exact_size(vec2(hit, hit), Sense::click());
        let centre = outer.center();
        let fill = if response.hovered() {
            crate::theme::blend(t.knob, 24, t.accent)
        } else {
            t.accent
        };
        ui.painter().circle_filled(centre, side * 0.5, color(fill));
        let icon = if playing { Icon::Pause } else { Icon::Play };
        let mark = side * 0.55;
        icons::paint(
            ui.painter(),
            icon,
            egui::Rect::from_center_size(centre, vec2(mark, mark)),
            color(t.knob),
            &t,
        );
        let name = if playing { "Pause" } else { "Play" };
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name));
        let response = response.on_hover_text(if playing {
            "Pause (Space)"
        } else if self.playing_motion_mode() {
            "Play the motion (Space)"
        } else {
            "Play the history step by step (Space). Each step waits until its model is built."
        });
        if response.clicked() {
            if self.timeline.steps == 0 && self.timeline.has_timeline && !self.state.timeline.motion
            {
                // An assembly: its motion is what plays.
                self.state.timeline.motion = true;
                let t = self.timeline.motion_t().unwrap_or(0.0);
                self.go_to_t(t);
            }
            self.toggle_playing();
        }
        response
    }

    /// True while the player plays (or would play) the motion: chosen, or
    /// an assembly, which has no history.
    pub(crate) fn playing_motion_mode(&self) -> bool {
        self.state.timeline.motion || (self.timeline.steps == 0 && self.timeline.has_timeline)
    }

    /// One step back (`-1`) or forward: a step of the history, or a
    /// twentieth of the motion's timeline.
    fn step_by(&mut self, by: i8) {
        if self.playing_motion_mode() {
            let t = self.timeline.motion_t().unwrap_or(0.0);
            let to = (t + f64::from(by) * MOTION_STEP).clamp(0.0, 1.0);
            self.stop_playing();
            self.go_to_t(to);
        } else {
            let steps = self.timeline.steps;
            let shown = self.timeline.shown_step().clamp(1, steps.max(1));
            let to = if by < 0 {
                shown.saturating_sub(1).max(1)
            } else {
                shown.saturating_add(1).min(steps.max(1))
            };
            if to != shown {
                self.go_to_step(to);
            }
        }
    }

    /// Where the player is: `3 / 10` in the history, `t 0.35` in the
    /// motion.
    fn player_position(&self) -> String {
        if self.playing_motion_mode() {
            let shown = self.timeline.motion_t().unwrap_or(0.0);
            t_text(self.shown_t().unwrap_or(shown))
        } else {
            let steps = self.timeline.steps;
            let shown = self.timeline.shown_step().clamp(1, steps.max(1));
            format!("{shown} / {steps}")
        }
    }

    /// Where the player's row drew its parts in the last frame: `back`,
    /// `play`, `forward`, `position`, `speed`, `loop`, `orbit` (those
    /// shown).
    pub fn player_hits(&self) -> &[(&'static str, egui::Rect)] {
        &self.player_hits
    }

    /// The `t` of the motion's pose that is drawn: the one `t` the player,
    /// the caption, the status bar and `ui.get` say.
    pub(crate) fn shown_t(&self) -> Option<f64> {
        self.timeline
            .motion
            .as_ref()
            .map(|m| m.t)
            .filter(|t| t.is_finite())
    }

    /// The width the compact player of the narrow layout needs, in points:
    /// the three buttons and the label of its step or `t`.
    pub(crate) fn compact_player_width(&self, side: f32, label: f32) -> f32 {
        3.0 * side + 2.0 * PLAYER_GAP + PLAYER_GROUP_GAP + label
    }

    /// The compact player of the narrow layout (a phone), in `rect` on its
    /// plate over the viewport while the drawer, which holds the whole
    /// player, is shut: a motion's round play button, its slider and its
    /// `t`; a history's three buttons and its step (board s8, the 44 px
    /// strip).
    pub(crate) fn compact_player(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        let tokens = self.tokens;
        let kit = Kit::plate(&tokens);
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = PLAYER_GAP;
                ui.style_mut().visuals.override_text_color = Some(color(tokens.plate_text));
                if self.playing_motion_mode() {
                    let play = self.play_button(ui, &kit);
                    let shown = self.timeline.motion_t().unwrap_or(0.0);
                    let label = t_text(self.shown_t().unwrap_or(shown));
                    let room =
                        (ui.available_width() - tokens.compact_player_label - PLAYER_GROUP_GAP)
                            .max(40.0);
                    ui.spacing_mut().slider_width = room;
                    let mut t = shown;
                    let slider = ui.add(
                        egui::Slider::new(&mut t, 0.0..=1.0)
                            .show_value(false)
                            .trailing_fill(true),
                    );
                    if t != shown {
                        self.go_to_t(t);
                        ui.ctx().request_repaint();
                    }
                    ui.add_space(PLAYER_GROUP_GAP);
                    let position = ui.label(label);
                    self.player_hits = vec![
                        ("play", play.rect),
                        ("slider", slider.rect),
                        ("position", position.rect),
                    ];
                } else {
                    self.player_row(ui, &kit);
                }
            },
        );
        self.player_shown = true;
    }
}

/// How far the step buttons move a motion's `t`.
const MOTION_STEP: f64 = 0.05;

/// The narrowest panel that holds the whole row; under it orbit is left
/// out (board s8).
pub(crate) const PLAYER_FULL_WIDTH: f32 = 220.0;

/// With touch sizes, the narrowest width that holds more than the three
/// buttons and the position (decision 4).
pub(crate) const PLAYER_TOUCH_FULL_WIDTH: f32 = 380.0;

/// Which parts of the player's row a width holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PlayerParts {
    /// Speed and loop (and orbit when `orbit`).
    pub settings: bool,
    pub orbit: bool,
}

/// The parts of the row for `width` points, with touch sizes or not.
pub(crate) fn player_parts(width: f32, touch: bool) -> PlayerParts {
    let roomy = width
        .partial_cmp(&PLAYER_TOUCH_FULL_WIDTH)
        .is_some_and(|o| o.is_ge());
    if touch && !roomy {
        PlayerParts {
            settings: false,
            orbit: false,
        }
    } else {
        PlayerParts {
            settings: true,
            orbit: width >= PLAYER_FULL_WIDTH,
        }
    }
}

/// The space between two of the player's buttons, in points.
const PLAYER_GAP: f32 = 2.0;

/// The extra space between the buttons and the position.
const PLAYER_GROUP_GAP: f32 = 6.0;

/// `0.5×`, `1×`, `2×`.
pub(crate) fn speed_text(speed: f64) -> String {
    format!("{speed}×")
}

/// The side of the player's buttons for touch targets `row` points
/// high: the kit's control, at least the target.
pub(crate) fn button_side_for(row: f32, tokens: &Tokens) -> f32 {
    tokens.control_height.max(row)
}

/// How every place that shows a motion's `t` writes it (the player, the
/// caption, the status bar's answer to `view.motion`): one source,
/// `Shell::shown_t`, rounded the same way.
pub(crate) fn t_text(t: f64) -> String {
    format!("t = {t:.3}")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::{PLAYER_FULL_WIDTH, PLAYER_TOUCH_FULL_WIDTH, Player, Tick, player_parts};
    use crate::state::Playback;
    use serde_json::json;

    fn playing() -> Playback {
        Playback {
            playing: true,
            ..Playback::default()
        }
    }

    /// A baked motion is asked for at its frames only, and a clock that
    /// moves less than a frame a tick still gets there: the clock runs
    /// unrounded, only what is asked for is rounded.
    #[test]
    fn a_baked_motion_plays_its_frames_at_its_speed() {
        use super::{MotionStep, motion_step, rounded_to_frames};
        assert_eq!(rounded_to_frames(0.26, None), 0.26);
        assert_eq!(rounded_to_frames(0.26, Some(4)), 0.25);
        assert_eq!(rounded_to_frames(0.38, Some(4)), 0.5);
        assert_eq!(rounded_to_frames(1.7, Some(120)), 1.0);
        assert_eq!(rounded_to_frames(-0.2, Some(120)), 0.0);
        assert!(rounded_to_frames(f64::NAN, Some(120)).is_nan());
        assert_eq!(rounded_to_frames(0.3, Some(0)), 0.3);
        // Every value asked for is a frame k / 120.
        let play = playing();
        let (mut clock, mut asked) = (0.0, Vec::new());
        // 100 ticks of 10 ms: a third of a frame each at speed 1.
        for _ in 0..100 {
            let MotionStep::Go(next, false) = motion_step(clock, false, 0.01, play) else {
                panic!("plays on");
            };
            clock = next;
            let t = rounded_to_frames(clock, Some(120));
            assert_eq!((t * 120.0).round() / 120.0, t);
            if asked.last() != Some(&t) {
                asked.push(t);
            }
        }
        // A second at 4 s a play: a quarter of the way, frame by frame.
        assert!((clock - 0.25).abs() < 1e-9, "{clock}");
        assert_eq!(asked.len(), 31);
        assert_eq!(asked.last(), Some(&0.25));
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

    /// One row always (board s8): orbit goes under 220 points; with touch
    /// sizes under 380 only the three buttons and the position stay.
    #[test]
    fn the_row_leaves_out_what_a_narrow_panel_cannot_hold() {
        let all = player_parts(PLAYER_FULL_WIDTH, false);
        assert!(all.settings && all.orbit);
        let narrow = player_parts(PLAYER_FULL_WIDTH - 1.0, false);
        assert!(narrow.settings && !narrow.orbit);
        let phone = player_parts(PLAYER_TOUCH_FULL_WIDTH - 1.0, true);
        assert!(!phone.settings && !phone.orbit);
        let tablet = player_parts(PLAYER_TOUCH_FULL_WIDTH, true);
        assert!(tablet.settings && tablet.orbit);
        for width in [f32::NAN, f32::NEG_INFINITY, -1.0] {
            let parts = player_parts(width, true);
            assert!(!parts.settings, "{width}");
        }
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
    fn the_mode_follows_the_document_and_a_play_that_cannot_run_says_why() {
        // A document with a motion timeline, its player in Motion.
        let mut p = Played::new(Playback::default());
        p.session
            .execute("variable.set", &json!({"name": "t", "expr": "0"}))
            .unwrap();
        p.frame();
        p.shell.state.timeline.motion = true;
        p.frame();
        assert!(p.shell.playing_motion());
        // Then a part without one (another file opened in its place, as a
        // turbofan after an assembly): the player plays its history.
        for (id, params) in [
            ("file.new", json!({})),
            ("feature.block", json!({"size": [10, 10, 10], "name": "A"})),
            ("feature.sphere", json!({"radius": 3, "name": "B"})),
        ] {
            p.session.execute(id, &params).unwrap();
        }
        p.frame();
        assert!(!p.shell.playing_motion(), "still in Motion");
        // The person's choice is kept for a document that has a timeline.
        assert!(p.shell.state.timeline.motion);
        p.shell.toggle_playing();
        assert!(p.shell.state.timeline.playing);
        assert!(p.until(3.0, |p| p.shown() == 1), "the history did not play");
        // Nothing at all to play: said, not silently ignored.
        p.shell.stop_playing();
        p.session.execute("file.new", &json!({})).unwrap();
        p.frame();
        p.shell.toggle_playing();
        assert!(!p.shell.state.timeline.playing);
        assert!(p.shell.timeline_json()["problem"].is_string());
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
