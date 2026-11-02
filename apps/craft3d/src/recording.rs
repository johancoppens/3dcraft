//! What the window keeps to record itself and to take pictures of itself
//! while it is not on screen (`ui.record`, `ui.advance`, `ui.screenshot
//! {window: true}`): the recorder on the window's own device
//! (`craft3d_ui_egui::record`), the input of the window's last frame to
//! run its passes with, and the clock of a frame-exact recording.
//!
//! While a recording runs, the shell is drawn only by the recorder's
//! passes, on one clock: the window's own frames show the last frame
//! recorded (a mirror), and in real time what the person does in the
//! window goes into the next pass. The passes are run where the event
//! loop is woken ([`crate::app`]'s driver), frame or no frame: a ticker
//! wakes it every `1/fps` seconds in real time, and the end of each task
//! of the engine thread while a frame-exact advance waits for the window
//! to settle.

use crate::control::Wake;
use crate::host::Request;
use craft3d_ui_egui::record::Recorder;
use eframe::egui;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Most passes a picture or a frame-exact frame runs at one moment of the
/// clock while the viewport still averages the frames of its picture
/// (`Look::accumulate`) or the CPU renderer makes its sharp image.
pub const SETTLE_PASSES: u32 = 256;
/// Passes a picture of the window runs before it is read back: what came
/// before it (a `ui.set`) is laid out and drawn as in the window's frames.
pub const PICTURE_PASSES: u32 = 3;
/// Most passes a picture of the window runs after those, a pass apart on
/// its clock, while something still moves into place (a popover fades in
/// over egui's animation time, 1/12 s, the drawer slides in a tenth of a
/// second: five and six passes at 60 a second).
pub const FADE_PASSES: u32 = 12;
/// How long one picture or frame may take to settle (its passes, and the
/// CPU renderer's sharp image) on the thread that draws the window: then
/// it is taken as it is, said to be unsettled.
pub const SETTLE_TIME: Duration = Duration::from_secs(4);
/// How long a `ui.advance` waits for the window to settle before its next
/// frame (a command or a scene still running) before it fails.
pub const ADVANCE_SETTLE: Duration = Duration::from_secs(60);
/// How often the event loop is woken while an advance waits, so that
/// deadline is kept without other events.
pub const ADVANCE_TICK: Duration = Duration::from_millis(250);
/// How long one turn of the event loop draws frame-exact frames before it
/// lets the window and the control channel have a turn.
pub const ADVANCE_SLICE: Duration = Duration::from_millis(40);

/// A `ui.advance` that is drawing its frames.
pub struct Advance {
    pub request: Request,
    /// The MCP message it came in, when it is a call of `control_call`
    /// (the reply is wrapped as that tool's).
    pub mcp: Option<String>,
    /// Frames still to draw.
    pub left: u64,
    pub dt: f64,
    pub drawn: u64,
    /// Frames drawn when their picture had not settled within
    /// [`SETTLE_TIME`].
    pub unsettled: u64,
    /// Since when it waits for the window to settle for its next frame.
    pub waiting: Option<Instant>,
    pub began: Instant,
    /// How long it draws before it answers with what it drew.
    pub timeout: Duration,
    pub timing: Timing,
}

/// Where the time of an advance went, in seconds.
#[derive(Clone, Copy, Debug, Default)]
pub struct Timing {
    /// Passes of the shell painted offscreen.
    pub passes: u64,
    /// In the passes read back (the shell, painting, reading back).
    pub read_passes: f64,
    /// In the passes not read back (settling: averaging, sharp image).
    pub other_passes: f64,
    /// Handing frames to the writer (waiting for room in its queue).
    pub push: f64,
    /// Waiting for the window to settle (commands, scenes).
    pub waiting: f64,
}

impl Timing {
    /// Per frame, in milliseconds, as `ui.advance` replies.
    pub fn per_frame(&self, frames: u64) -> serde_json::Value {
        let n = frames.max(1) as f64;
        let ms = |s: f64| (s * 1e3 / n * 10.0).round() / 10.0;
        serde_json::json!({
            "passesPerFrame": (self.passes as f64 / n * 10.0).round() / 10.0,
            "readPassMs": ms(self.read_passes),
            "settlePassMs": ms(self.other_passes),
            "pushMs": ms(self.push),
            "waitMs": ms(self.waiting),
        })
    }
}

/// The window's recording state.
pub struct Rec {
    /// On the window's device; `None` before the window exists, or when
    /// it has no wgpu device.
    pub recorder: Option<Recorder>,
    /// The input of the window's last frame, without its events: what the
    /// passes run with (its size, pixels per point, modifiers).
    pub input: egui::RawInput,
    /// egui's time of the window's last frame, and when that was.
    pub clock: Option<(f64, Instant)>,
    /// What the person did in the window since the last pass, while a
    /// real-time recording draws the shell.
    pub events: Vec<egui::Event>,
    /// The last frame recorded, for the window to show.
    pub mirror: Option<egui::ColorImage>,
    /// The `ui.advance` drawing its frames.
    pub advance: Option<Advance>,
    /// The step of the clock the next pass is made with (egui's
    /// `predicted_dt`).
    pub pass_dt: f32,
    /// When a real-time recording draws its next frame.
    pub next_frame: Option<Instant>,
    /// Why a real-time recording stopped by itself, for the next
    /// `ui.record`.
    pub failure: Option<String>,
    /// Keeps the ticker of a real-time recording going while true.
    ticker: Option<Arc<AtomicBool>>,
    started: Instant,
}

impl Default for Rec {
    fn default() -> Self {
        Self {
            recorder: None,
            input: window_input([1280.0, 800.0]),
            clock: None,
            events: Vec::new(),
            mirror: None,
            advance: None,
            pass_dt: 1.0 / 60.0,
            next_frame: None,
            failure: None,
            ticker: None,
            started: Instant::now(),
        }
    }
}

/// The input of a window of `size` points that has drawn no frame yet.
pub fn window_input(size: [f32; 2]) -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(size[0], size[1]),
        )),
        ..Default::default()
    }
}

impl Rec {
    /// egui's time now, on the clock of the window's frames.
    pub fn now(&self) -> f64 {
        match self.clock {
            Some((time, at)) => time + at.elapsed().as_secs_f64(),
            None => self.started.elapsed().as_secs_f64(),
        }
    }

    /// Takes in the input of a window frame: what the passes run with, and
    /// in a real-time recording its events, for the next pass.
    pub fn window_frame(&mut self, raw: &egui::RawInput) {
        let mut input = raw.clone();
        let events = std::mem::take(&mut input.events);
        input.hovered_files.clear();
        input.dropped_files.clear();
        if let Some(time) = input.time {
            self.clock = Some((time, Instant::now()));
        }
        self.input = input;
        let real_time = self
            .recorder
            .as_ref()
            .is_some_and(|r| r.recording.is_some() && !r.stepping());
        if real_time {
            // Bounded: a window that is not drawn into for long while the
            // person types would otherwise keep every event.
            if self.events.len() < 4096 {
                self.events.extend(events);
            }
        }
    }

    /// Whether the window shows the recording's frames instead of drawing
    /// the shell itself.
    pub fn mirrored(&self) -> bool {
        self.recorder
            .as_ref()
            .is_some_and(|r| r.recording.is_some())
    }

    /// Starts waking the event loop every `period` with `wake`, until
    /// [`Self::stop_ticker`].
    pub fn start_ticker(&mut self, period: Duration, wake: Option<Wake>) {
        self.stop_ticker();
        let Some(wake) = wake else {
            return;
        };
        let running = Arc::new(AtomicBool::new(true));
        let keep = Arc::clone(&running);
        let started = std::thread::Builder::new()
            .name("craft3d-record-ticker".to_string())
            .spawn(move || {
                while keep.load(Ordering::Relaxed) {
                    std::thread::sleep(period);
                    wake();
                }
            });
        if started.is_ok() {
            self.ticker = Some(running);
        }
    }

    pub fn stop_ticker(&mut self) {
        if let Some(running) = self.ticker.take() {
            running.store(false, Ordering::Relaxed);
        }
    }
}

impl Drop for Rec {
    fn drop(&mut self) {
        self.stop_ticker();
    }
}
