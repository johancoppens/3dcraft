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
use crate::state::Playback;
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
        } else if self.timeline.steps > 0 {
            self.state.timeline.playing = true;
        }
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
        let tick = self
            .player
            .tick(play, now, self.timeline.shown_step(), self.timeline.steps);
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

    /// The player's row of buttons under the timeline's slider.
    pub(crate) fn player_controls(&mut self, ui: &mut egui::Ui) {
        let tokens = self.tokens;
        let steps = self.timeline.steps;
        let shown = self.timeline.shown_step().clamp(1, steps.max(1));
        let play = self.state.timeline;
        let at_end = self.at_end();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
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
            if icon_button(ui, Icon::StepForward, "Step forward", false, shown < steps, &tokens)
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
            ui.add_space(4.0);
            let speed = speed_text(play.speed);
            if ui
                .add(egui::Button::new(speed).min_size(vec2(
                    0.0,
                    button_side(ui),
                )))
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
        });
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
    use super::{Player, Tick};
    use crate::state::Playback;
    use serde_json::json;

    fn playing() -> Playback {
        Playback {
            playing: true,
            ..Playback::default()
        }
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
