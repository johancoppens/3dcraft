//! The player's bake hint (docs/design/motion.md › *The player*): while a
//! motion plays, the window weighs what its moments cost, and when they
//! are heavy (the median over the recent ones at or above
//! [`HEAVY_ON_MS`]) the player offers "This motion is heavy — bake
//! first?" with a button that runs `motion.bake` and a progress bar while
//! it bakes. Hysteresis keeps the hint from flickering: once heavy, a
//! motion is light again only when the median has fallen below
//! [`HEAVY_OFF_MS`]. The hint goes away when the motion is baked or
//! dismissed (remembered per document for the session). A motion that is
//! not heavy but whose moments are shown provisionally (an assembly whose
//! configurations follow `t`, built in the background as it plays) is
//! baked by itself, once, as the player did before the hint, with the
//! same progress bar.
//!
//! A moment's cost is the time its `view.motion` took in the window's
//! thread (the solve and the placement, or a part's regeneration), on
//! a clock the browser has too (`web_time`'s there), not the frame around it:
//! a heavy picture is not a heavy moment, and baking would not help it
//! (the lighter drawing while it plays does). Nothing is weighed while the
//! window is recorded or captured, or for a moment asked for by the slider
//! or an agent rather than the playing player.

use super::Shell;
use crate::kit::{Kit, controls::Button, status::BannerKind};
use serde_json::{Value, json};
use std::collections::{BTreeSet, VecDeque};
// The browser has no clock in `std` (`std::time::Instant::now` panics
// there): `web_time`'s, the page's.
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

/// The moments weighed: the median over the last so many.
pub(crate) const HEAVY_MOMENTS: usize = 15;

/// Fewest moments weighed before the player judges.
pub(crate) const HEAVY_MIN_MOMENTS: usize = 8;

/// A motion whose moments cost this much (the median), in milliseconds,
/// or more is heavy.
pub(crate) const HEAVY_ON_MS: f64 = 100.0;

/// A heavy motion is light again once the median is below this, in
/// milliseconds: the gap between the two keeps the hint from flickering.
pub(crate) const HEAVY_OFF_MS: f64 = 70.0;

/// The share of the moments weighed shown provisionally (configurations
/// still built in the background) at or above which a motion that is not
/// heavy is baked by itself.
const PROVISIONAL_BAKE: f64 = 0.5;

/// A moment that took longer than this, in milliseconds, is not weighed:
/// a machine that stood still for a while (suspended, swapping) says
/// nothing about the motion.
const COST_CAP_MS: f64 = 2000.0;

/// The frames the player bakes at: 30 a second over a play at speed 1
/// (`MOTION_SECONDS`), as the player rounds `t` to them.
pub(crate) const BAKE_FRAMES: u64 = 120;

/// Configurations (an assembly) or moments (a part) baked per call where
/// the bake runs in the window's thread, between its frames (a part, the
/// browser): one, so a frame takes about a moment's time.
const BAKE_PIECE: u64 = 1;

/// What the recent moments of a motion cost, and whether it is heavy.
#[derive(Clone, Debug, Default)]
pub(crate) struct Heavy {
    /// The cost of each moment weighed, in milliseconds, oldest first, and
    /// whether it was shown provisionally.
    moments: VecDeque<(f64, bool)>,
    /// The judgement, held between the thresholds.
    heavy: bool,
}

impl Heavy {
    /// Weighs a moment that cost `ms` milliseconds, shown provisionally
    /// or not. A cost that is not a number is not weighed.
    pub(crate) fn note(&mut self, ms: f64, provisional: bool) {
        if !ms.is_finite() || !(0.0..=COST_CAP_MS).contains(&ms) {
            return;
        }
        self.moments.push_back((ms, provisional));
        while self.moments.len() > HEAVY_MOMENTS {
            self.moments.pop_front();
        }
        self.judge();
    }

    /// The median cost of the moments weighed, in milliseconds, once
    /// there are enough to judge.
    pub(crate) fn median(&self) -> Option<f64> {
        if self.moments.len() < HEAVY_MIN_MOMENTS {
            return None;
        }
        let mut costs: Vec<f64> = self.moments.iter().map(|(ms, _)| *ms).collect();
        costs.sort_by(f64::total_cmp);
        let half = costs.len() / 2;
        let upper = costs.get(half).copied()?;
        if costs.len() % 2 == 1 {
            Some(upper)
        } else {
            let lower = costs.get(half.checked_sub(1)?).copied()?;
            Some((lower + upper) * 0.5)
        }
    }

    /// The share of the moments weighed that were shown provisionally.
    pub(crate) fn provisional(&self) -> f64 {
        if self.moments.is_empty() {
            return 0.0;
        }
        let provisional = self.moments.iter().filter(|(_, p)| *p).count();
        provisional as f64 / self.moments.len() as f64
    }

    fn judge(&mut self) {
        let Some(median) = self.median() else {
            return;
        };
        if self.heavy {
            if median < HEAVY_OFF_MS {
                self.heavy = false;
            }
        } else if median >= HEAVY_ON_MS {
            self.heavy = true;
        }
    }

    /// Whether a motion that is not heavy is to be baked by itself: enough
    /// moments weighed, and at least half of them shown provisionally.
    pub(crate) fn wants_bake(&self) -> bool {
        self.median().is_some() && !self.heavy && self.provisional() >= PROVISIONAL_BAKE
    }

    /// Whether the motion is heavy, as last judged.
    pub(crate) fn heavy(&self) -> bool {
        self.heavy
    }

    /// Forgets every moment: another document, an edit, a bake done.
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
}

/// A bake the hint asked for, while it runs.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Baking {
    /// Baked in pieces in the window's thread (a part, the browser): the
    /// next piece is asked for when the last answered not done. Else the
    /// bake runs in the background builder, and what it has left is the
    /// motion's `building`.
    pieces: bool,
    /// A piece (or the bake handed to the builder) waits for its answer.
    asked: bool,
    /// The answer to the first call came: what is left is known.
    started: bool,
    /// What is left: the builder's `building`, or the pieces' `remaining`.
    left: usize,
    /// The most left seen since it started: the progress bar's whole.
    whole: usize,
    /// Pieces in a row that left as much as before ([`STALLED_PIECES`]).
    stalled: u32,
    /// The most done shown so far: the bar never goes back when the
    /// builder finds more to do (it asks for the frames' configurations
    /// a few at a time).
    shown: f64,
    /// Started by itself (a light motion shown provisionally), not by the
    /// Bake button.
    auto: bool,
    /// Stopped by a stall before the bake was done ([`STALLED_PIECES`]).
    cut_short: bool,
}

impl Baking {
    /// How far it is, from 0 to 1, as last seen ([`Self::see`]).
    pub(crate) fn progress(&self) -> f64 {
        self.shown
    }

    /// Takes in the answer of a call of `motion.bake`: whether another
    /// piece is to be asked for.
    pub(crate) fn answered(&mut self, answer: &Value) -> bool {
        self.asked = false;
        self.started = true;
        let count = |key: &str| {
            answer
                .get(key)
                .and_then(Value::as_u64)
                .map_or(0, |n| usize::try_from(n).unwrap_or(usize::MAX))
        };
        if answer.get("background").and_then(Value::as_bool) == Some(true) {
            self.pieces = false;
            self.see(count("building"));
            false
        } else {
            let before = self.pieces.then_some(self.left);
            self.pieces = true;
            let left = count("remaining");
            // A piece that built a configuration (an assembly) or a
            // moment (a part) made progress even when the steps left did
            // not go down (a step of `t` may need several).
            let built = answer
                .pointer("/configurations/built")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                .saturating_add(answer.get("moments").and_then(Value::as_u64).unwrap_or(0));
            self.stalled = match before {
                Some(before) if left >= before && built == 0 => self.stalled.saturating_add(1),
                _ => 0,
            };
            self.see(left);
            let more = answer.get("done").and_then(Value::as_bool) == Some(false);
            if more && self.stalled >= STALLED_PIECES {
                // Nothing moves: what was made stays, and the hint ends;
                // the motion is not baked (offered again).
                self.left = 0;
                self.cut_short = true;
                return false;
            }
            more
        }
    }

    /// What is left now.
    pub(crate) fn see(&mut self, left: usize) {
        self.left = left;
        self.whole = self.whole.max(left);
        if self.whole > 0 {
            let done = self.whole.saturating_sub(left) as f64 / self.whole as f64;
            self.shown = self.shown.max(done.clamp(0.0, 1.0));
        }
    }

    /// Whether it is done: answered, and nothing left (in pieces: the
    /// last piece said done; in the background: the builder has nothing
    /// left).
    pub(crate) fn done(&self) -> bool {
        self.started && !self.asked && self.left == 0
    }
}

/// Pieces in a row that leave as much to do as before, after which the
/// hint stops asking for more (the bake cannot go on: its budget is full).
const STALLED_PIECES: u32 = 3;

/// What the player's hint shows.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Hint {
    /// Nothing.
    None,
    /// "This motion is heavy — bake first?", with Bake and Not now.
    Offer,
    /// The bake it asked for runs: how far it is.
    Baking(f64),
}

/// What the hint keeps between frames.
#[derive(Clone, Debug, Default)]
pub(crate) struct BakeHint {
    /// What the recent moments cost.
    pub(crate) heavy: Heavy,
    /// The player asked for a moment that is not shown yet: its cost is
    /// weighed when it is.
    asked: bool,
    /// What the `view.motion` of the moment asked for took, in
    /// milliseconds, once it ran.
    made: Option<f64>,
    /// When ([`MomentClock::now`]) the moment asked for was handed to be
    /// made elsewhere (the page's worker): it is weighed by the time until
    /// it came, not by the command that asked for it.
    elsewhere_since: Option<f64>,
    /// The clock moments are timed by ([`MomentClock`]).
    pub(crate) clock: MomentClock,
    /// The bake asked for from the hint, while it runs.
    pub(crate) baking: Option<Baking>,
    /// The documents (by id) whose hint was dismissed in this session.
    dismissed: BTreeSet<u64>,
    /// The documents (by id) whose bake from the hint started and did not
    /// finish (cut short by a stall, an error or another tab): their
    /// motion's `baked` says frames although not all are made, so it is
    /// offered (or baked by itself) again.
    partial: BTreeSet<u64>,
    /// The motion was baked by itself once (a light one shown
    /// provisionally): not again until an edit or another document.
    auto_baked: bool,
    /// The answers of `motion.bake` asked for by the hint, not taken in
    /// yet: `Err` with why it failed.
    pub(crate) answers: Vec<Result<Value, String>>,
    /// Where the Bake button was drawn in this frame, in points.
    bake_hit: Option<egui::Rect>,
}

impl BakeHint {
    /// Another document is shown, or the document was edited: what was
    /// weighed and baked is about the motion that was (an edit lets go of
    /// a bake); what was dismissed stays.
    pub(crate) fn forget(&mut self) {
        self.heavy.reset();
        self.not_weighed();
        self.baking = None;
        self.answers.clear();
        self.auto_baked = false;
    }

    /// The playing player asked for a moment: its `view.motion` is timed.
    pub(crate) fn asked(&mut self) {
        self.asked = true;
        self.made = None;
        self.elsewhere_since = None;
    }

    /// The `view.motion` of the moment asked for handed it to be made
    /// elsewhere at `now` ([`MomentClock::now`]).
    pub(crate) fn made_elsewhere_from(&mut self, now: f64) {
        if self.asked {
            self.elsewhere_since = Some(now);
        }
    }

    /// The moment made elsewhere came at `now`: what it took is its cost.
    pub(crate) fn came(&mut self, now: f64) {
        if let Some(since) = self.elsewhere_since.take() {
            self.made(now - since);
        }
    }

    /// A moment made elsewhere is on its way.
    pub(crate) fn coming(&self) -> bool {
        self.elsewhere_since.is_some()
    }

    /// Whether the `view.motion` about to run is the playing player's,
    /// to be timed.
    pub(crate) fn timing(&self) -> bool {
        self.asked && self.made.is_none()
    }

    /// The `view.motion` of the moment asked for ran in `ms` milliseconds.
    pub(crate) fn made(&mut self, ms: f64) {
        if self.asked {
            self.made = Some(ms);
        }
    }

    /// The moment asked for is shown, provisionally or not: its cost is
    /// weighed (once its `view.motion` ran).
    pub(crate) fn shown(&mut self, provisional: bool) {
        if let (true, Some(ms)) = (self.asked, self.made) {
            self.heavy.note(ms, provisional);
            self.not_weighed();
        }
    }

    /// Stops weighing the moment asked for (playing stopped, the window
    /// is recorded): its time says nothing of its cost.
    pub(crate) fn not_weighed(&mut self) {
        self.asked = false;
        self.made = None;
        self.elsewhere_since = None;
    }
}

/// The clock a moment's `view.motion` is timed by, in milliseconds:
/// `web_time`'s (the browser has it), or one a test sets
/// ([`Shell::set_moment_clock`]).
#[derive(Clone, Default)]
pub(crate) struct MomentClock(Option<std::sync::Arc<dyn Fn() -> f64 + Send + Sync>>);

impl std::fmt::Debug for MomentClock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_some() {
            "MomentClock(set)"
        } else {
            "MomentClock"
        })
    }
}

impl MomentClock {
    /// Milliseconds since a moment of its own (the first time asked),
    /// for spans across frames.
    pub(crate) fn now(&self) -> f64 {
        match &self.0 {
            Some(clock) => clock(),
            None => {
                static EPOCH: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
                EPOCH.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.0
            }
        }
    }

    /// Runs `work` and says how long it took, in milliseconds.
    pub(crate) fn time<R>(&self, work: impl FnOnce() -> R) -> (R, f64) {
        match &self.0 {
            Some(clock) => {
                let start = clock();
                let out = work();
                (out, clock() - start)
            }
            None => {
                // native clock: `Instant` is std's only off the browser
                // (the `use` above).
                let start = Instant::now();
                let out = work();
                (out, start.elapsed().as_secs_f64() * 1000.0)
            }
        }
    }
}

impl Shell {
    /// What the player's hint shows for the motion of the active document.
    pub(crate) fn bake_hint(&self) -> Hint {
        if !self.timeline.has_timeline || !self.playing_motion() {
            return Hint::None;
        }
        let hint = &self.player.hint;
        if let Some(baking) = &hint.baking {
            return Hint::Baking(baking.progress());
        }
        if self.bake_dismissed() || self.motion_baked() || !hint.heavy.heavy() {
            return Hint::None;
        }
        Hint::Offer
    }

    /// Whether the hint was dismissed for the active document.
    fn bake_dismissed(&self) -> bool {
        self.viewing()
            .is_some_and(|id| self.player.hint.dismissed.contains(&id))
    }

    /// Whether the motion shown is baked: a bake's frames, and nothing
    /// left for the builder.
    fn motion_baked(&self) -> bool {
        let partial = self
            .viewing()
            .is_some_and(|id| self.player.hint.partial.contains(&id));
        !partial
            && self
                .timeline
                .motion
                .as_ref()
                .is_some_and(|m| m.baked.is_some() && m.building == 0)
    }

    /// Times moments by `clock` (milliseconds) instead of the wall's, or
    /// by the wall's again with `None`: for tests, which make a moment as
    /// slow or as quick as they need.
    pub fn set_moment_clock(
        &mut self,
        clock: Option<std::sync::Arc<dyn Fn() -> f64 + Send + Sync>>,
    ) {
        self.player.hint.clock = MomentClock(clock);
    }

    /// While it plays: a motion that is not heavy but shown provisionally
    /// (configurations built in the background as it plays) is baked by
    /// itself, once, as the player did before the hint (2026-11-01); a
    /// heavy one is offered instead, and a dismissed one is left alone.
    pub(crate) fn auto_bake(&mut self) {
        let hint = &self.player.hint;
        if hint.auto_baked
            || hint.baking.is_some()
            || !hint.heavy.wants_bake()
            || self.bake_dismissed()
            || self.motion_baked()
        {
            return;
        }
        self.player.hint.auto_baked = true;
        self.bake_from_hint();
        if let Some(baking) = self.player.hint.baking.as_mut() {
            baking.auto = true;
        }
    }

    /// Bakes the motion of the active document, as the hint's button does:
    /// `motion.bake` at the frames the player plays, in the background
    /// builder where there is one, else in pieces between frames.
    pub(crate) fn bake_from_hint(&mut self) {
        if self.player.hint.baking.is_some() {
            return;
        }
        self.player.hint.baking = Some(Baking {
            asked: true,
            ..Baking::default()
        });
        // Not baked until it is done: cut short, it is offered again.
        if let Some(id) = self.viewing() {
            self.player.hint.partial.insert(id);
        }
        self.queue_bake_piece();
    }

    fn queue_bake_piece(&mut self) {
        self.timeline.queue(
            "motion.bake",
            json!({"frames": BAKE_FRAMES, "background": true, "limit": BAKE_PIECE}),
        );
    }

    /// The hint's Not now: no hint for this document again in this
    /// session.
    pub(crate) fn dismiss_bake_hint(&mut self) {
        if let Some(id) = self.viewing() {
            self.player.hint.dismissed.insert(id);
        }
    }

    /// Once a frame: takes in the answers of the bake asked for, asks for
    /// its next piece, follows the builder, and ends it when done. True
    /// while it runs, so the window comes back for it.
    pub(crate) fn follow_bake(&mut self) -> bool {
        self.player.hint.bake_hit = None;
        let answers = std::mem::take(&mut self.player.hint.answers);
        let mut next_piece = false;
        for answer in answers {
            match (answer, self.player.hint.baking.as_mut()) {
                (Ok(value), Some(baking)) => next_piece |= baking.answered(&value),
                // Said in the timeline's problem already: the hint offers
                // the bake again.
                (Err(_), _) => self.player.hint.baking = None,
                (Ok(_), None) => {}
            }
        }
        // What the builder has left, by steps of about equal work.
        let building = self.timeline.motion.as_ref().map(|m| m.work_left);
        let Some(baking) = self.player.hint.baking.as_mut() else {
            return false;
        };
        if !baking.pieces && baking.started {
            baking.see(building.unwrap_or(0));
        }
        if next_piece {
            baking.asked = true;
            self.queue_bake_piece();
            return true;
        }
        if baking.done() {
            let finished = !baking.cut_short;
            self.player.hint.baking = None;
            self.player.hint.heavy.reset();
            if finished && let Some(id) = self.viewing() {
                self.player.hint.partial.remove(&id);
            }
            return false;
        }
        true
    }

    /// The hint under the motion's caption: the offer with its buttons,
    /// or the bake's progress.
    // Review of 2026-11-02, N4: the hint's place and look are restyled with
    // the player in the UI overhaul's phase 2b; kept small until then.
    pub(crate) fn bake_hint_ui(&mut self, ui: &mut egui::Ui) {
        let kit = Kit::panel(&self.tokens);
        match self.bake_hint() {
            Hint::None => {}
            Hint::Offer => {
                let ms = self.player.hint.heavy.median().unwrap_or(0.0);
                let question = "This motion is heavy — bake first?";
                let reason = format!(
                    "Each moment takes about {:.0} ms to make. Baking makes the moments \
                     of a play ahead; then it plays smoothly.",
                    ms.round()
                );
                // The title does not wrap: in a narrow panel the question
                // opens the body instead, so the banner never widens it.
                let t = &kit.t;
                let around =
                    2.0 * (t.inset + 2.0) + t.info_bar_width + t.nav_icon_size + t.nav_icon_gap;
                let wide = kit
                    .galley(ui, question, kit.medium_font(ui), kit.skin.text)
                    .size()
                    .x;
                let (title, body) = if wide + around <= ui.available_width() {
                    (Some(question), reason)
                } else {
                    (None, format!("{question} {reason}"))
                };
                let mut bake = false;
                let mut dismiss = false;
                let mut hit = None;
                kit.banner(ui, BannerKind::Attention, title, &body, |kit, ui| {
                    let button = kit.button(ui, Button::primary("Bake")).on_hover_text(
                        "Make the moments of a play ahead (motion.bake), in the \
                             background: it goes on playing meanwhile",
                    );
                    hit = Some(button.rect);
                    bake = button.clicked();
                    dismiss = kit
                        .button(ui, Button::quiet("Not now"))
                        .on_hover_text("Do not offer it again for this document")
                        .clicked();
                });
                self.player.hint.bake_hit = hit;
                if bake {
                    self.bake_from_hint();
                    ui.ctx().request_repaint();
                } else if dismiss {
                    self.dismiss_bake_hint();
                }
            }
            Hint::Baking(progress) => {
                let text = format!("Baking the motion… {:.0}%", progress * 100.0);
                let width = ui.available_width();
                kit.progress(ui, &text, progress as f32, width)
                    .on_hover_text("motion.bake runs; the motion plays smoothly once it is done");
            }
        }
    }

    /// Where the hint's Bake button was drawn in the last frame, in
    /// points: for a host or a test that presses it.
    pub fn bake_hint_button(&self) -> Option<egui::Rect> {
        self.player.hint.bake_hit
    }

    /// The hint as `ui.get` gives it under `timeline.heavy`.
    pub(crate) fn heavy_json(&self) -> Value {
        let hint = &self.player.hint;
        let shown = self.bake_hint();
        json!({
            "msPerMoment": hint.heavy.median().map(|ms| (ms * 10.0).round() / 10.0),
            "moments": hint.heavy.moments.len(),
            "provisional": (hint.heavy.provisional() * 100.0).round() / 100.0,
            "heavy": hint.heavy.heavy(),
            "suggestBake": shown == Hint::Offer,
            "baking": match (shown, &hint.baking) {
                (Hint::Baking(progress), Some(baking)) => json!({
                    "progress": (progress * 1000.0).round() / 1000.0,
                    "auto": baking.auto,
                }),
                _ => Value::Null,
            },
            "dismissed": self.viewing().is_some_and(|id| hint.dismissed.contains(&id)),
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{BAKE_FRAMES, Baking, HEAVY_MIN_MOMENTS, HEAVY_MOMENTS, Heavy, Hint};
    use crate::state::Playback;
    use serde_json::json;

    /// A crank whose block goes round a circle, played in a shell whose
    /// frames are `dt` seconds of egui's clock apart, and whose moments
    /// take `dt` too on the clock the hint times them by (injected:
    /// [`crate::Shell::set_moment_clock`]), unless it is the wall's.
    struct Played {
        shell: crate::Shell,
        session: craft3d_engine::Session,
        ctx: egui::Context,
        time: f64,
        /// What the injected clock moves by each time it is read, in ms.
        step: std::sync::Arc<std::sync::Mutex<f64>>,
    }

    impl Played {
        fn new() -> Self {
            let mut played = Self::wall();
            let step = std::sync::Arc::clone(&played.step);
            let now = std::sync::Mutex::new(0.0_f64);
            played
                .shell
                .set_moment_clock(Some(std::sync::Arc::new(move || {
                    let step = *step.lock().unwrap();
                    let mut at = now.lock().unwrap();
                    *at += step;
                    *at
                })));
            played
        }

        /// Moments timed by the wall's clock.
        fn wall() -> Self {
            let mut session = craft3d_engine::Session::new(craft3d_engine::Workspace::none());
            for (id, params) in [
                ("file.new", json!({})),
                ("variable.set", json!({"name": "t", "expr": "0"})),
                (
                    "feature.block",
                    json!({"name": "Crank", "size": ["4 mm", "4 mm", "4 mm"],
                           "position": ["15 mm * cos(360 deg * t)", "15 mm * sin(360 deg * t)", "0 mm"]}),
                ),
            ] {
                session.execute(id, &params).unwrap();
            }
            let mut played = Self {
                shell: crate::Shell::new(),
                session,
                ctx: egui::Context::default(),
                time: 0.0,
                step: std::sync::Arc::new(std::sync::Mutex::new(0.0)),
            };
            played.frame(0.0);
            played.shell.state.timeline = Playback {
                playing: true,
                motion: true,
                looping: true,
                ..Playback::default()
            };
            played
        }

        fn frame(&mut self, dt: f64) {
            self.frame_with(dt, Vec::new());
        }

        fn frame_with(&mut self, dt: f64, events: Vec<egui::Event>) {
            self.time += dt;
            *self.step.lock().unwrap() = dt * 1000.0;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 700.0),
                )),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let Self { shell, session, .. } = self;
            let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
            output.textures_delta.clear();
        }

        fn frames(&mut self, n: usize, dt: f64) {
            for _ in 0..n {
                self.frame(dt);
            }
        }

        fn heavy(&self) -> serde_json::Value {
            self.shell.timeline_json()["heavy"].clone()
        }
    }

    #[test]
    fn the_hint_shows_for_slow_moments_not_for_quick_ones() {
        // Moments 30 ms apart: light, no hint.
        let mut quick = Played::new();
        quick.frames(40, 0.03);
        let heavy = quick.heavy();
        assert!(
            (heavy["msPerMoment"].as_f64().unwrap() - 30.0).abs() < 1.0,
            "{heavy}"
        );
        assert_eq!(heavy["suggestBake"], false, "{heavy}");
        assert_eq!(quick.shell.bake_hint(), Hint::None);

        // Moments 150 ms apart: heavy once enough are weighed.
        let mut slow = Played::new();
        slow.frames(3, 0.15);
        assert_eq!(slow.heavy()["suggestBake"], false, "too few weighed yet");
        slow.frames(30, 0.15);
        let heavy = slow.heavy();
        assert!(
            (heavy["msPerMoment"].as_f64().unwrap() - 150.0).abs() < 1.0,
            "{heavy}"
        );
        assert_eq!(heavy["suggestBake"], true, "{heavy}");
        assert_eq!(slow.shell.bake_hint(), Hint::Offer);
        // Paused, it stays offered.
        slow.shell.stop_playing();
        slow.frames(3, 0.15);
        assert_eq!(slow.shell.bake_hint(), Hint::Offer);

        // Frames of a recording or a capture are not weighed: their clock
        // is not the wall's.
        let mut recorded = Played::new();
        recorded.shell.set_frame_exact(true);
        recorded.frames(30, 0.5);
        assert_eq!(recorded.heavy()["moments"], 0);
        assert_eq!(recorded.heavy()["suggestBake"], false);
    }

    #[test]
    fn the_bake_button_runs_motion_bake_and_the_hint_goes_when_baked() {
        let mut p = Played::new();
        p.frames(30, 0.15);
        assert_eq!(p.shell.bake_hint(), Hint::Offer);
        assert!(p.session.motion().unwrap().baked.is_none());
        // A click on the Bake button the hint shows.
        let at = p
            .shell
            .bake_hint_button()
            .expect("the Bake button drawn")
            .center();
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        p.frame_with(0.03, vec![egui::Event::PointerMoved(at), button(true)]);
        p.frame_with(0.03, vec![button(false)]);
        // It runs motion.bake: the hint follows the bake it asked for.
        assert!(
            p.shell.player.hint.baking.is_some(),
            "the click ran no bake"
        );
        assert_eq!(p.session.motion().unwrap().baked, Some(BAKE_FRAMES));
        p.frame(0.03);
        // A part bakes in pieces between frames: the progress grows.
        let first = match p.shell.bake_hint() {
            Hint::Baking(progress) => progress,
            other => panic!("not baking: {other:?}"),
        };
        p.frames(40, 0.03);
        let later = match p.shell.bake_hint() {
            Hint::Baking(progress) => progress,
            other => panic!("not baking: {other:?}"),
        };
        assert!(later > first, "{first} -> {later}");
        assert!(p.heavy()["baking"]["progress"].as_f64().unwrap() > 0.0);
        for _ in 0..400 {
            if p.shell.bake_hint() == Hint::None {
                break;
            }
            p.frame(0.03);
        }
        assert_eq!(p.shell.bake_hint(), Hint::None);
        assert_eq!(p.session.motion().unwrap().baked, Some(BAKE_FRAMES));
        assert_eq!(p.heavy()["suggestBake"], false);
        assert_eq!(p.heavy()["baking"], serde_json::Value::Null);
        // Baked, slow frames do not bring the offer back.
        p.frames(30, 0.15);
        assert_eq!(p.shell.bake_hint(), Hint::None);
    }

    #[test]
    fn a_dismissed_hint_stays_away_for_that_document() {
        let mut p = Played::new();
        p.frames(30, 0.15);
        assert_eq!(p.shell.bake_hint(), Hint::Offer);
        p.shell.dismiss_bake_hint();
        assert_eq!(p.shell.bake_hint(), Hint::None);
        assert_eq!(p.heavy()["dismissed"], true);
        // An edit lets go of what was weighed, not of the dismissal.
        p.session
            .execute("variable.set", &json!({"name": "t", "expr": "0.5"}))
            .unwrap();
        p.frame(0.15);
        p.shell.state.timeline.playing = true;
        p.frames(30, 0.15);
        assert!(p.shell.player.hint.heavy.heavy());
        assert_eq!(p.shell.bake_hint(), Hint::None);
        // Another document is offered it.
        p.session.execute("file.new", &json!({})).unwrap();
        for (id, params) in [
            ("variable.set", json!({"name": "t", "expr": "0"})),
            (
                "feature.block",
                json!({"name": "B", "size": ["2 mm", "2 mm", "10 mm * t + 1 mm"]}),
            ),
        ] {
            p.session.execute(id, &params).unwrap();
        }
        p.frame(0.15);
        p.shell.state.timeline.playing = true;
        p.shell.state.timeline.motion = true;
        p.frames(30, 0.15);
        assert_eq!(p.shell.bake_hint(), Hint::Offer);
    }

    #[test]
    fn slow_frames_of_quick_moments_are_not_heavy() {
        // Frames 150 ms apart (a heavy picture), the moments timed by the
        // wall's clock: the crank's view.motion takes far less.
        let mut p = Played::wall();
        p.frames(30, 0.15);
        let heavy = p.heavy();
        assert!(heavy["moments"].as_u64().unwrap() >= 8, "{heavy}");
        assert!(heavy["msPerMoment"].as_f64().unwrap() < 100.0, "{heavy}");
        assert_eq!(heavy["suggestBake"], false, "{heavy}");
        assert_eq!(p.shell.bake_hint(), Hint::None);
    }

    #[test]
    fn a_bake_cut_short_by_another_tab_is_offered_again() {
        let mut p = Played::new();
        p.frames(30, 0.15);
        assert_eq!(p.shell.bake_hint(), Hint::Offer);
        p.shell.bake_from_hint();
        p.frames(5, 0.03);
        assert!(matches!(p.shell.bake_hint(), Hint::Baking(_)));
        // Some frames are made: the motion says it was baked.
        assert_eq!(p.session.motion().unwrap().baked, Some(BAKE_FRAMES));
        // Another tab, and back: the bake is let go of unfinished.
        p.session.execute("file.new", &json!({})).unwrap();
        p.frame(0.03);
        p.session
            .execute("file.activate", &json!({"document": 0}))
            .unwrap();
        p.frame(0.03);
        p.shell.state.timeline.playing = true;
        p.shell.state.timeline.motion = true;
        p.frames(30, 0.15);
        assert_eq!(p.shell.bake_hint(), Hint::Offer, "{}", p.heavy());
    }

    #[test]
    fn a_pause_drops_the_moment_asked_for() {
        let mut p = Played::new();
        p.frames(3, 0.03);
        p.shell.state.timeline.playing = false;
        p.frames(3, 0.5);
        p.shell.state.timeline.playing = true;
        p.frames(2, 0.03);
        // No moment weighed at the paused frames' 500 ms.
        let heavy = p.heavy();
        assert!(
            p.shell
                .player
                .hint
                .heavy
                .moments
                .iter()
                .all(|(ms, _)| *ms < 100.0),
            "{heavy}"
        );
        // A cost beyond the cap is not weighed at all.
        let mut h = Heavy::default();
        h.note(super::COST_CAP_MS + 1.0, false);
        assert!(h.moments.is_empty());
    }

    fn weighed(costs: &[f64]) -> Heavy {
        let mut h = Heavy::default();
        for ms in costs {
            h.note(*ms, false);
        }
        h
    }

    #[test]
    fn heavy_above_the_threshold_not_below_and_only_once_there_are_enough() {
        // Below: never heavy.
        assert!(!weighed(&[90.0; 20]).heavy());
        // Above, but too few moments to judge.
        let few = weighed(&[150.0; HEAVY_MIN_MOMENTS - 1]);
        assert!(!few.heavy());
        assert_eq!(few.median(), None);
        // Enough: heavy.
        let many = weighed(&[150.0; HEAVY_MIN_MOMENTS]);
        assert!(many.heavy());
        assert_eq!(many.median(), Some(150.0));
    }

    #[test]
    fn one_spike_is_not_heavy_the_median_is() {
        let mut costs = vec![40.0; HEAVY_MOMENTS - 1];
        costs.push(1500.0);
        let h = weighed(&costs);
        assert!(!h.heavy());
        assert_eq!(h.median(), Some(40.0));
        // A cost that is not a number is not weighed.
        let mut h = weighed(&[40.0; 10]);
        h.note(f64::NAN, false);
        h.note(-3.0, false);
        assert_eq!(h.moments.len(), 10);
    }

    #[test]
    fn hysteresis_keeps_it_from_flickering() {
        // Costs that wander around the threshold: once heavy it stays so
        // until the median is well below it.
        let mut h = weighed(&[120.0; HEAVY_MOMENTS]);
        assert!(h.heavy());
        let mut flips = 0;
        let mut was = h.heavy();
        for k in 0..200 {
            let ms = if k % 2 == 0 { 85.0 } else { 105.0 };
            h.note(ms, false);
            if h.heavy() != was {
                flips += 1;
                was = h.heavy();
            }
        }
        assert_eq!(flips, 0, "it flickered");
        assert!(h.heavy());
        // Light again only below the lower threshold.
        for _ in 0..HEAVY_MOMENTS {
            h.note(50.0, false);
        }
        assert!(!h.heavy());
        // And from light, the same wandering does not make it heavy.
        for k in 0..200 {
            h.note(if k % 2 == 0 { 85.0 } else { 95.0 }, false);
            assert!(!h.heavy(), "heavy at {k}");
        }
    }

    #[test]
    fn quick_moments_shown_provisionally_are_baked_by_themselves_not_offered() {
        let mut h = Heavy::default();
        for _ in 0..HEAVY_MIN_MOMENTS - 1 {
            h.note(40.0, true);
        }
        assert!(!h.wants_bake(), "too few weighed yet");
        h.note(40.0, true);
        assert!(!h.heavy());
        assert!(h.wants_bake());
        assert_eq!(h.provisional(), 1.0);
        // Exact again: nothing to bake.
        for _ in 0..HEAVY_MOMENTS {
            h.note(40.0, false);
        }
        assert!(!h.wants_bake());
        // Heavy and provisional: offered, not baked by itself.
        for _ in 0..HEAVY_MOMENTS {
            h.note(150.0, true);
        }
        assert!(h.heavy());
        assert!(!h.wants_bake());
    }

    #[test]
    fn progress_follows_what_is_left() {
        // In the background: what the builder has left.
        let mut b = Baking {
            asked: true,
            ..Baking::default()
        };
        assert_eq!(b.progress(), 0.0);
        assert!(!b.done());
        let again =
            b.answered(&json!({"frames": 120, "background": true, "queued": 121, "building": 40}));
        assert!(!again);
        assert_eq!(b.progress(), 0.0);
        b.see(30);
        assert_eq!(b.progress(), 0.25);
        b.see(10);
        assert_eq!(b.progress(), 0.75);
        assert!(!b.done());
        // The builder found more to do: the bar does not go back.
        b.see(30);
        assert_eq!(b.progress(), 0.75);
        b.see(5);
        assert_eq!(b.progress(), 0.875);
        b.see(0);
        assert_eq!(b.progress(), 1.0);
        assert!(b.done());
        // In pieces: what the last piece said is left, and another piece
        // until done.
        let mut p = Baking {
            asked: true,
            ..Baking::default()
        };
        assert!(p.answered(&json!({"frames": 120, "done": false, "remaining": 120})));
        assert!(p.answered(&json!({"frames": 120, "done": false, "remaining": 60})));
        assert_eq!(p.progress(), 0.5);
        assert!(!p.answered(&json!({"frames": 120, "done": true, "remaining": 0})));
        assert!(p.done());
        // A piece that built something made progress, though the steps
        // left stayed.
        let mut slow = Baking::default();
        let building = json!({"frames": 120, "done": false, "remaining": 50,
                              "configurations": {"built": 1}});
        assert!((0..10).all(|_| slow.answered(&building)));
        // Pieces that build nothing and leave as much: it stops asking.
        let mut stuck = Baking::default();
        let answer = json!({"frames": 120, "done": false, "remaining": 50});
        let asked: Vec<bool> = (0..10).map(|_| stuck.answered(&answer)).collect();
        assert_eq!(asked.iter().filter(|more| **more).count(), 3, "{asked:?}");
        assert!(stuck.done());
    }
}
