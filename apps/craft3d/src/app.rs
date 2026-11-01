//! The window: eframe around the shell, on an event loop of our own, with
//! the control channel's requests answered on the UI thread whenever that
//! loop is woken for them, whether or not a frame is drawn.

use crate::args::Options;
use crate::control::{ControlConfig, ControlServer, Wake};
use crate::files::{self, Choice, Picker, SystemDialogs};
use crate::host::{Host, Request, Route, answer_on_engine, report, route};
use crate::recording::{
    ADVANCE_SETTLE, ADVANCE_SLICE, ADVANCE_TICK, Advance, FADE_PASSES, PICTURE_PASSES, Rec,
    SETTLE_PASSES, SETTLE_TIME,
};
use crate::refine::{Poll, Refiner};
use crate::tabs;
use crate::token;
use crate::worker::{Finished, Outcome, Run, Task, Worker};
use craft3d_automation::rpc;
use craft3d_engine::{Access, RecoveryOptions, Session, Workspace};
use craft3d_ui_egui::files::file_stem;
use craft3d_ui_egui::{DocAction, FileAction, ParamAsk, Shell, window_title};
use eframe::UserEvent;
use eframe::egui;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::WindowId;

/// How long a closing application waits for work still running.
const SETTLE: Duration = Duration::from_secs(3);

/// The window's title: the product name, as users read it.
pub const TITLE: &str = "3DCraft";

/// The reverse-DNS identifier of 3DCraft (docs/decisions.md, 2026-10-22):
/// the window's app id, the desktop file's name, the macOS bundle id.
pub const APP_ID: &str = "io.github.johancoppens.craft3d";

/// The window's icon (X11 and Windows show it; Wayland and macOS take the
/// installed one), decoded from the PNG the installers carry. `None` when
/// it cannot be read: the window then has the toolkit's icon.
pub fn window_icon() -> Option<egui::IconData> {
    let bytes: &[u8] = include_bytes!("../../../packaging/icons/craft3d-64.png");
    let mut reader = png::Decoder::new(bytes).read_info().ok()?;
    let mut rgba = vec![0; reader.output_buffer_size()];
    let frame = reader.next_frame(&mut rgba).ok()?;
    if frame.color_type != png::ColorType::Rgba || frame.bit_depth != png::BitDepth::Eight {
        return None;
    }
    rgba.truncate(frame.buffer_size());
    Some(egui::IconData {
        rgba,
        width: frame.width,
        height: frame.height,
    })
}

/// The window as it opens: titled, with the app id and icon of the
/// installers, of the size asked for, and decorated by the system. The
/// title bar and borders are the platform's: the window manager's on X11,
/// Windows and macOS, and on Wayland winit's own (sctk-adwaita), because
/// GNOME draws none for a window (docs/development.md, Window). Nothing
/// here makes the window transparent or undecorated.
pub fn viewport(options: &Options, title: &str) -> egui::ViewportBuilder {
    let viewport = egui::ViewportBuilder::default()
        .with_title(title)
        // The desktop file's name (packaging/linux), so a Wayland
        // compositor finds the entry and the icon.
        .with_app_id(APP_ID)
        .with_decorations(true)
        .with_transparent(false)
        .with_resizable(true)
        .with_inner_size(
            options
                .window_size
                .map_or([1280.0, 800.0], |[w, h]| [w as f32, h as f32]),
        )
        .with_min_inner_size([480.0, 320.0]);
    match window_icon() {
        Some(icon) => viewport.with_icon(icon),
        None => viewport,
    }
}

/// Turns a root flag's value into an access rule. A missing directory whose
/// parent exists is made (the start screen's suggested work folder,
/// `Documents/3DCraft`); it is made absolute with its symbolic links
/// resolved: the engine compares paths as text, so the root has to be the
/// one true spelling.
pub(crate) fn root(flag: &str, dir: Option<&str>) -> Result<Access, String> {
    let Some(dir) = dir else {
        return Ok(Access::None);
    };
    let wanted = std::path::Path::new(dir);
    if !wanted.exists()
        && wanted
            .parent()
            .is_some_and(|parent| parent.as_os_str().is_empty() || parent.is_dir())
    {
        std::fs::create_dir(wanted)
            .map_err(|e| format!("`{flag} {dir}`: cannot make this directory: {e}"))?;
    }
    let path = craft3d_engine::canonical(wanted)
        .map_err(|e| format!("`{flag} {dir}`: cannot use this directory: {e}"))?;
    if !path.is_dir() {
        return Err(format!("`{flag} {dir}`: not a directory"));
    }
    Ok(Access::Root(path))
}

/// The session the window starts with: the file the user named is open,
/// and from then on files are reachable only beneath the automation roots.
pub fn session(options: &Options) -> Result<Session, String> {
    let workspace = Workspace {
        read: root("--automation-read-root", options.read_root.as_deref())?,
        write: root("--automation-write-root", options.write_root.as_deref())?,
    };
    // The user's own file: read with the user's own authority, like a file
    // picked in a dialog. Nothing else is read this way.
    let mut session = Session::new(Workspace {
        read: Access::Unrestricted,
        write: Access::None,
    });
    if let Some(file) = &options.file {
        session
            .execute("file.open", &json!({ "path": file }))
            .map_err(|e| e.message().to_string())?;
    }
    session.set_workspace(workspace);
    Ok(session)
}

/// How the window's session autosaves, when the options name a recovery
/// directory: there, at the interval asked for, on the engine's own
/// thread, waking the window when something was written or went wrong so
/// the status bar shows it. The directory is made absolute, so it does
/// not move with the working directory; it need not exist yet.
pub fn recovery(options: &Options, wake: Wake) -> Result<Option<RecoveryOptions>, String> {
    let Some(dir) = &options.recovery_dir else {
        return Ok(None);
    };
    let dir = std::path::absolute(dir).map_err(|e| {
        format!(
            "`--recovery-dir {}`: cannot use this directory: {e}",
            dir.display()
        )
    })?;
    let mut recovery = RecoveryOptions::new(dir).with_notify(wake);
    if let Some(interval) = options.autosave_interval {
        recovery = recovery.with_interval(interval);
    }
    Ok(Some(recovery))
}

/// The control listener the options ask for, if any, with the channel its
/// requests arrive on.
pub fn control(
    options: &Options,
    wake: Wake,
) -> Result<(Option<ControlServer>, Receiver<Request>), String> {
    let (requests, incoming) = mpsc::channel();
    let Some(port) = options.control else {
        return Ok((None, incoming));
    };
    let Some(token_file) = &options.token_file else {
        return Err("`--control` needs `--control-token-file FILE`".to_string());
    };
    let token = token::load_or_create(token_file)?;
    let server = ControlServer::start(ControlConfig::new(port, token), requests, wake)?;
    Ok((Some(server), incoming))
}

/// What the window works on, and what reaches it from outside a frame:
/// the control channel's requests and the answers of the file dialogs.
///
/// It is served in two places, by the frames and by the event loop when
/// it is woken ([`Driver`]), because a frame is not something to count
/// on: a Wayland compositor sends a window it does not show (another
/// workspace, minimised, fully covered, a locked screen) no frame
/// callbacks, winit then delivers no redraw, and work that waits for a
/// frame waits until the window is looked at again.
///
/// With an engine thread ([`Self::with_engine_thread`]) commands run
/// there: the session is handed over with the request and comes back with
/// the reply, and meanwhile the window draws what it showed before and
/// what is being done ([`Shell::ui_away`]). Requests stay in the order
/// they came in; only `ui.get` and `ui.quit` are answered at once while
/// work runs. Without one (tests, or a machine that cannot start a thread)
/// every request is answered on the thread that serves the desk, as it
/// comes.
pub struct Desk {
    pub host: Host,
    requests: Receiver<Request>,
    /// The open and save dialogs.
    picker: Picker,
    /// Requests that came in and wait for their turn.
    queue: VecDeque<Request>,
    worker: Option<Worker>,
    /// Builds the display scene beside the engine thread while the window
    /// shows the quick first picture (with the engine thread only).
    refiner: Option<Refiner>,
    /// What the engine thread is doing for the desk, while it has the
    /// session.
    waiting: Option<Waiting>,
    /// How long a turn waits for a task it just started before it lets the
    /// window draw: a quick command is answered in the same turn, a slow
    /// one does not hold up a frame.
    patience: Duration,
    /// A picture of the window asked for (`ui.screenshot {window: true}`),
    /// while it waits for its frame; the requests after it wait with it.
    capture: Option<Capture>,
    /// How long a capture waits for a frame ([`CAPTURE_DEADLINE`]).
    capture_deadline: Duration,
    /// Wakes the thread that serves the desk, frame or no frame: what a
    /// capture's deadline rings ([`Self::with_wake`]).
    wake: Option<Wake>,
    /// The timer that rings `wake` at a capture's deadline: one thread
    /// for the window's life, handed each deadline as it comes.
    deadlines: Option<mpsc::Sender<Instant>>,
    /// The window's frames painted offscreen: recordings, and pictures of
    /// a window that is not on screen (`recording.rs`).
    rec: Rec,
}

/// A request for a picture of the window: it is answered once egui hands
/// over a frame it drew after the request came in, and the session is
/// back on this thread (the path is written through it); or with an error
/// when no frame came within [`CAPTURE_DEADLINE`].
struct Capture {
    request: Request,
    path: Option<String>,
    /// Frames drawn since it came in: the frame is asked for after a few,
    /// so that what came before it (a `ui.set`) is drawn and has settled.
    frames: u32,
    /// Whether the last frame found the viewport still averaging the
    /// frames of its still picture (then `frames` does not count it).
    settling: bool,
    /// The frame count when the picture was last asked for: egui drops a
    /// request on a surface that timed out, is outdated, occluded, lost or
    /// recreated, so it is asked again after [`CAPTURE_RETRY_FRAMES`].
    asked: Option<u32>,
    /// When it came in.
    since: Instant,
    /// The picture, once egui handed it over, waiting for the session.
    image: Option<Arc<egui::ColorImage>>,
}

/// Frames a picture of the window waits before it is asked for, counted
/// once the viewport has averaged the frames of its still picture.
const CAPTURE_FRAMES: u32 = 3;
/// Frames drawn after asking without a picture before it is asked again.
const CAPTURE_RETRY_FRAMES: u32 = 30;
/// How long a picture of the window waits for a frame before it is
/// answered with an error.
const CAPTURE_DEADLINE: Duration = Duration::from_secs(8);

/// How long after a capture's deadline its timer wakes the desk.
const DEADLINE_GRACE: Duration = Duration::from_millis(20);

/// The error of a picture of the window that got no frame in time.
const NO_FRAME: &str = "ui.screenshot: the window drew no frame in time; is it minimised, hidden or on another workspace?";
/// The error of a picture of the window whose viewport was still averaging
/// the frames of its still picture at the deadline.
const NOT_SETTLED: &str = "ui.screenshot: the viewport was still averaging the frames of its picture when the time was up; ask again, or set fewer frames (ui.set look.accumulate)";

/// Who waits for the task on the engine thread.
enum Waiting {
    /// A request: its reply goes back when the task is done.
    Request(Request),
    /// A file chosen in a dialog: the outcome goes to the status bar.
    File(&'static str),
    /// An export the save dialog named: the outcome goes to the status
    /// bar, and once it is written what its dialog chose is kept.
    Export(Box<files::ExportDone>),
    /// A command the shell asked for (the Parameters tab's `param.set`):
    /// the outcome goes back to the shell and to the status bar.
    Command(&'static str),
    /// `ui.param`: its command's outcome goes back to the shell, then the
    /// reply, made once the window has synced, to the request.
    Param(Request, Box<ParamAsk>),
    /// Only the scene the window shows.
    Scene,
}

/// How long a turn waits for a task it started, by default: well under a
/// frame at 60 Hz.
pub const PATIENCE: Duration = Duration::from_millis(12);

/// What one turn of serving did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Turn {
    /// Something changed that a frame should show, or more may be waiting.
    pub changed: bool,
    /// The application was asked to stop (`ui.quit`).
    pub quit: bool,
}

const PANICKED: &str = "internal error (a bug in 3DCraft, please report it); the document was kept";

impl Desk {
    pub fn new(host: Host, requests: Receiver<Request>, picker: Picker) -> Self {
        Self {
            host,
            requests,
            picker,
            queue: VecDeque::new(),
            worker: None,
            refiner: None,
            waiting: None,
            patience: PATIENCE,
            capture: None,
            capture_deadline: CAPTURE_DEADLINE,
            wake: None,
            deadlines: None,
            rec: Rec::default(),
        }
    }

    /// Paints the window's frames offscreen from now on, on the device and
    /// with the renderer of `state` (the window's own): `ui.record`,
    /// `ui.advance`, and `ui.screenshot {window: true}` without a frame
    /// of the compositor. Without one those wait for the window's frames.
    pub fn attach_recorder(&mut self, state: Option<&eframe::egui_wgpu::RenderState>) {
        self.rec.recorder = state.cloned().map(craft3d_ui_egui::record::Recorder::new);
    }

    /// The window's size in points before it has drawn a frame.
    pub fn set_window_points(&mut self, size: [f32; 2]) {
        self.rec.input = crate::recording::window_input(size);
    }

    /// Wakes the desk with `wake` when a picture of the window reaches
    /// its deadline: a window that draws no frames (hidden, minimised, on
    /// another workspace) sends no event that would serve it, and the
    /// request would wait past its deadline for the next one. Set by
    /// [`Self::with_engine_thread`] too.
    #[must_use]
    pub fn with_wake(mut self, wake: Wake) -> Self {
        self.wake = Some(wake);
        self
    }

    /// Runs commands on an engine thread of their own from now on, which
    /// calls `wake` when there is something new to show. When the thread
    /// cannot be started they run where the desk is served, as before.
    #[must_use]
    pub fn with_engine_thread(mut self, wake: Wake) -> Self {
        self.wake = Some(Arc::clone(&wake));
        self.worker = Worker::start(Arc::clone(&wake)).ok();
        if self.worker.is_some() {
            // Coarse first, then sharp: the engine thread builds the quick
            // first picture, the refiner the display scene.
            self.refiner = Some(Refiner::new(wake));
            self.host.session.set_refine_elsewhere(true);
        }
        // The window's thread never waits for a bake: it is made in the
        // background, cancellable (docs/design/motion.md).
        self.host.session.set_bake_in_background(true);
        self
    }

    /// Makes every refinement wait `delay` before it starts: a slow
    /// model, for the tests.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn with_slow_refinement(mut self, delay: Duration) -> Self {
        if let Some(refiner) = &mut self.refiner {
            refiner.delay = delay;
        }
        self
    }

    /// How long a picture of the window waits for a frame.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn with_capture_deadline(mut self, deadline: Duration) -> Self {
        self.capture_deadline = deadline;
        self
    }

    /// How long a turn waits for a task it started ([`PATIENCE`]).
    #[must_use]
    pub fn with_patience(mut self, patience: Duration) -> Self {
        self.patience = patience;
        self
    }

    /// True while the session is on the engine thread: draw with
    /// [`Shell::ui_away`] and take the title from what the shell shows.
    pub fn away(&self) -> bool {
        self.waiting.is_some()
    }

    /// Answers the requests that are waiting, then does what the File menu
    /// asked for in the last frame and what a dialog answered since.
    /// Needs no frame and no window.
    pub fn turn(&mut self) -> Turn {
        if self.worker.is_none() {
            let mut answered = self.settle_capture();
            while let Ok(request) = self.requests.try_recv() {
                self.queue.push_back(request);
            }
            let mut served = 0;
            while served < MAX_PER_TURN {
                if self.capture.is_some() || self.rec.advance.is_some() {
                    self.answer_record_stop_now();
                    self.answer_anytime_now();
                    break;
                }
                let Some(request) = self.queue.pop_front() else {
                    break;
                };
                served += 1;
                answered = true;
                if let Some(request) = self
                    .intercept_record(request)
                    .and_then(|request| self.intercept_capture(request))
                {
                    self.host.answer(request);
                }
            }
            let filed = self.host.pump_files(&mut self.picker);
            let closing = self.capture.is_none() && self.advance_quit();
            return Turn {
                changed: answered || filed || closing,
                quit: self.host.shell.quit_requested(),
            };
        }
        let mut changed = self.collect();
        if self.settle_capture() {
            changed = true;
        }
        let mut served = 0;
        let mut scenes = 0;
        loop {
            if self.waiting.is_some() {
                let finished = self.worker.as_ref().and_then(Worker::try_finished);
                let Some(finished) = finished else {
                    // What came in with the request that started the
                    // work and may be answered meanwhile, is.
                    self.answer_anytime();
                    // Progress to show.
                    changed = true;
                    break;
                };
                self.complete(finished);
                changed = true;
                continue;
            }
            if self.refine() {
                changed = true;
            }
            if served >= MAX_PER_TURN || self.host.shell.quit_requested() {
                break;
            }
            // The scene the window shows is built on the engine thread,
            // never by a frame or a request on this one.
            if scenes == 0 && !self.scene_ready() {
                scenes += 1;
                // The document's name: a file named on the command line is
                // opened here, its scene being its first.
                let subject = self
                    .host
                    .session
                    .document_status()
                    .and_then(|status| status.path)
                    .and_then(|path| file_stem(&path));
                self.start_about(
                    Waiting::Scene,
                    "meshing".to_string(),
                    subject,
                    Box::new(|_| Outcome::Built),
                );
                continue;
            }
            // A screenshot shows the display scene, not the first picture:
            // it waits for the refinement, and what came after it waits
            // with it, in order.
            if self.queue.front().is_some_and(|r| is_screenshot(&r.line)) && self.refining() {
                break;
            }
            // A picture of the window waits for its frame, and what came
            // after it waits with it; so do the frames of a `ui.advance`.
            if self.capture.is_some() || self.rec.advance.is_some() {
                // `ui.record {stop: true}` does not wait for an advance.
                self.answer_record_stop_now();
                // `ui.get` and `ui.quit` do not wait for it, nor does what
                // a dialog of the File menu answered.
                self.answer_anytime_now();
                if self.pump_files() {
                    changed = true;
                }
                break;
            }
            if let Some(request) = self.queue.pop_front() {
                served += 1;
                changed = true;
                let Some(request) = self
                    .intercept_record(request)
                    .and_then(|request| self.intercept_capture(request))
                else {
                    continue;
                };
                // `ui.param` is settled here and regenerates on the
                // engine thread, as the form's commits do.
                if is_method(&request.line, "ui.param") {
                    self.ask_param(request);
                    continue;
                }
                match route(&request.line) {
                    Route::Engine => {
                        let label = request_label(&request.line);
                        let subject = request_subject(&request.line);
                        let line = request.line.clone();
                        self.start_about(
                            Waiting::Request(request),
                            label,
                            subject,
                            Box::new(move |session| {
                                Outcome::Reply(answer_on_engine(session, &line))
                            }),
                        );
                    }
                    Route::Window | Route::Anytime => self.host.answer(request),
                }
                continue;
            }
            if self.pump_files() {
                changed = true;
                continue;
            }
            // Nothing else to do: the next step of closing the window.
            if self.capture.is_none() && self.advance_quit() {
                changed = true;
                continue;
            }
            break;
        }
        self.show_work();
        Turn {
            changed,
            quit: self.host.shell.quit_requested(),
        }
    }

    /// Takes the requests that came in: queued in order, but for `ui.get`
    /// and `ui.quit` while the engine thread has the session, which are
    /// answered at once from what the window shows.
    fn collect(&mut self) -> bool {
        let mut any = false;
        while let Ok(request) = self.requests.try_recv() {
            any = true;
            if self.away() && route(&request.line) == Route::Anytime {
                self.show_work();
                let reply = self.answer_away(&request.line);
                let _ = request.reply.send(reply);
            } else {
                self.queue.push_back(request);
            }
        }
        any
    }

    /// Answers the queued `ui.get` and `ui.quit` while the session is
    /// away; the rest keeps its place.
    fn answer_anytime(&mut self) {
        if !self.queue.iter().any(|r| route(&r.line) == Route::Anytime) {
            return;
        }
        self.show_work();
        let queued = std::mem::take(&mut self.queue);
        for request in queued {
            if route(&request.line) == Route::Anytime {
                let reply = self.answer_away(&request.line);
                let _ = request.reply.send(reply);
            } else {
                self.queue.push_back(request);
            }
        }
    }

    /// `ui.get` or `ui.quit` while the session is away. Never panics.
    fn answer_away(&mut self, line: &str) -> Option<String> {
        let shell = &mut self.host.shell;
        let answered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let request: Value = serde_json::from_str(line).unwrap_or(Value::Null);
            let id = request.get("id").cloned().unwrap_or(Value::Null);
            let params = request.get("params");
            let result = match request.get("method").and_then(Value::as_str) {
                Some("ui.get") => shell.ui_get_away(params),
                _ => {
                    shell.request_quit();
                    Ok(json!({ "quitting": true }))
                }
            };
            match result {
                Ok(result) => rpc::ok_reply(&id, &result),
                Err(error) => rpc::error_reply(&id, &error),
            }
        }));
        Some(answered.unwrap_or_else(|_| rpc::error_reply(&Value::Null, PANICKED)))
    }

    /// Hands the refiner's results to the session and starts what it
    /// wants next; the session is home. True when the active document's
    /// scene changed. Where no thread can be started for a refinement,
    /// the display scene is built on the engine thread from then on.
    fn refine(&mut self) -> bool {
        let quality = self.host.shell.scene_quality();
        let Some(refiner) = &mut self.refiner else {
            return false;
        };
        match refiner.poll(&mut self.host.session, quality) {
            Poll::Changed => true,
            Poll::Same => false,
            Poll::CannotStart => {
                self.refiner = None;
                self.host.session.set_refine_elsewhere(false);
                true
            }
        }
    }

    /// True while the active document's display scene is being refined.
    fn refining(&mut self) -> bool {
        let quality = self.host.shell.scene_quality();
        self.refiner
            .as_ref()
            .is_some_and(|refiner| refiner.refining(&mut self.host.session, quality))
    }

    /// True when the scene the window shows is built (or could not be,
    /// and that is known), so nothing on this thread builds it.
    fn scene_ready(&mut self) -> bool {
        let quality = self.host.shell.scene_quality();
        self.host.session.scene_ready(quality)
    }

    /// Hands the session to the engine thread with a task, and waits a
    /// moment for it ([`Self::with_patience`]).
    fn start(&mut self, waiting: Waiting, label: String, run: Run) {
        self.start_about(waiting, label, None, run);
    }

    /// [`Self::start`] for work about `subject` (the name of the file
    /// being opened), which the status bar names.
    fn start_about(&mut self, waiting: Waiting, label: String, subject: Option<String>, run: Run) {
        let task = Task {
            label,
            subject,
            run,
            quality: self.host.shell.scene_quality(),
        };
        let Some(worker) = &self.worker else {
            let outcome = Worker::run_here(&mut self.host.session, task);
            self.finish(waiting, outcome);
            return;
        };
        let session = std::mem::replace(&mut self.host.session, Session::new(Workspace::none()));
        match worker.submit(session, task) {
            Ok(()) => {
                self.waiting = Some(waiting);
                if let Some(finished) = worker.wait_finished(self.patience) {
                    self.complete(finished);
                }
            }
            // The engine thread is gone: here, then.
            Err(back) => {
                let (session, task) = *back;
                self.host.session = session;
                self.worker = None;
                let outcome = Worker::run_here(&mut self.host.session, task);
                self.finish(waiting, outcome);
            }
        }
    }

    /// The session is back: answer whoever waited.
    fn complete(&mut self, finished: Finished) {
        self.host.session = finished.session;
        if let Some(waiting) = self.waiting.take() {
            self.finish(waiting, finished.outcome);
        }
        self.show_work();
    }

    fn finish(&mut self, waiting: Waiting, outcome: Outcome) {
        let host = &mut self.host;
        let mut param_reply = None;
        let (reply_to, reply) = match waiting {
            Waiting::Request(request) => {
                let reply = match outcome {
                    Outcome::Reply(reply) => reply,
                    Outcome::Panicked | Outcome::Ran(_) | Outcome::Built => {
                        Some(rpc::error_reply(&Value::Null, PANICKED))
                    }
                };
                if let Some(reply) = &reply {
                    // The status bar's line: not worth a panic.
                    let line = request.line.as_str();
                    if let Ok(Some(report)) = std::panic::catch_unwind(|| report(line, reply)) {
                        host.shell.last_command = Some(report);
                    }
                    // `ui.quit {save: "save"}` saved everything.
                    if crate::host::quits(line, reply) {
                        host.shell.request_quit_saved();
                    }
                }
                (Some(request.reply), reply)
            }
            Waiting::File(label) => {
                let outcome = match outcome {
                    Outcome::Ran(outcome) => outcome,
                    Outcome::Panicked | Outcome::Reply(_) | Outcome::Built => {
                        Err(PANICKED.to_string())
                    }
                };
                host.report(label, &outcome);
                (None, None)
            }
            Waiting::Export(done) => {
                let outcome = match outcome {
                    Outcome::Ran(outcome) => outcome,
                    Outcome::Panicked | Outcome::Reply(_) | Outcome::Built => {
                        Err(PANICKED.to_string())
                    }
                };
                host.report("file.export", &outcome);
                if outcome.is_ok() {
                    host.export_written(&done);
                }
                (None, None)
            }
            Waiting::Command(label) => {
                let outcome = match outcome {
                    Outcome::Ran(outcome) => outcome,
                    Outcome::Panicked | Outcome::Reply(_) | Outcome::Built => {
                        Err(PANICKED.to_string())
                    }
                };
                host.report(label, &outcome);
                host.shell.command_done(outcome);
                (None, None)
            }
            Waiting::Param(request, ask) => {
                let outcome = match outcome {
                    Outcome::Ran(outcome) => outcome,
                    Outcome::Panicked | Outcome::Reply(_) | Outcome::Built => {
                        Err(PANICKED.to_string())
                    }
                };
                host.report("ui.param", &outcome);
                host.shell.command_done(outcome.clone());
                param_reply = Some((request, ask, outcome));
                (None, None)
            }
            Waiting::Scene => (None, None),
        };
        // The camera follows the model, the tree its features: from the
        // scene the engine thread built. Guarded like a request.
        let synced = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            host.shell.sync(&mut host.session);
        }));
        if synced.is_err() {
            host.autosave_after_a_panic();
        }
        if let Some(reply_to) = reply_to {
            // A requester that has gone away is not an error.
            let _ = reply_to.send(reply);
        }
        // `ui.param` says what the form shows now that the window synced.
        if let Some((request, ask, outcome)) = param_reply {
            let id = request_id(&request.line);
            let reply = rpc::ok_reply(&id, &host.shell.ui_param_answer(&ask, &outcome));
            let _ = request.reply.send(Some(reply));
        }
    }

    /// `ui.param`: settled and joined to the form's waiting commit here
    /// (it reads `param.list`), then run on the engine thread; the reply
    /// comes once it is done. What it refuses is answered at once.
    fn ask_param(&mut self, request: Request) {
        let parsed: Value = serde_json::from_str(&request.line).unwrap_or(Value::Null);
        let id = parsed.get("id").cloned().unwrap_or(Value::Null);
        let asked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.host
                .shell
                .ui_param_ask(&mut self.host.session, parsed.get("params"))
        }))
        .unwrap_or_else(|_| Err(PANICKED.to_string()));
        match asked {
            Err(error) => {
                let _ = request.reply.send(Some(rpc::error_reply(&id, &error)));
            }
            Ok(ask) => {
                let command = ask.command.clone();
                self.start(
                    Waiting::Param(request, Box::new(ask)),
                    "param.set".to_string(),
                    Box::new(move |session| {
                        Outcome::Ran(
                            session
                                .execute(command.command, &command.params)
                                .map_err(|e| e.message().to_string()),
                        )
                    }),
                );
            }
        }
    }

    /// What the menu asked for and what a dialog answered: a file chosen
    /// to open or to save as is opened or saved on the engine thread.
    fn pump_files(&mut self) -> bool {
        if let Some(choice) = self.picker.poll() {
            match choice {
                Ok(Choice::Open(path)) => self.start_about(
                    Waiting::File("file.open"),
                    "file.open".to_string(),
                    file_stem(&path.to_string_lossy()),
                    Box::new(move |session| {
                        Outcome::Ran(files::run_as_user(session, "file.open", &path, false, None))
                    }),
                ),
                Ok(Choice::Save(path)) => {
                    // Save in the question before closing: closed once
                    // it is saved.
                    let close_after = std::mem::take(&mut self.host.close_after_save);
                    self.start(
                        Waiting::File("file.save"),
                        "file.save".to_string(),
                        Box::new(move |session| {
                            let confirmed =
                                Some((files::CONFIRMED.0, Value::Bool(files::CONFIRMED.1)));
                            let saved =
                                files::run_as_user(session, "file.save", &path, true, confirmed);
                            Outcome::Ran(tabs::close_when_saved(session, saved, close_after))
                        }),
                    );
                }
                Ok(Choice::Import(path)) => self.start(
                    Waiting::File("file.import"),
                    "file.import".to_string(),
                    Box::new(move |session| Outcome::Ran(files::import_as_user(session, &path))),
                ),
                Ok(Choice::ExportJig(path)) => self.start(
                    Waiting::File("jig.export"),
                    "jig.export".to_string(),
                    Box::new(move |session| {
                        Outcome::Ran(files::run_as_user(session, "jig.export", &path, true, None))
                    }),
                ),
                // Written at once: the save dialog named the file.
                Ok(Choice::Export(job)) => {
                    if let Some((path, params, done)) = self.host.export_target(*job) {
                        self.start_export(path, params, done);
                    }
                }
                other => self.host.file_choice(other),
            }
            return true;
        }
        // The export dialog's Export…: the save dialog asks where.
        if let Some(request) = self.host.shell.take_export_answer() {
            self.host.ask_where_to_export(request, &mut self.picker);
            return true;
        }
        if let Some(action) = self.host.shell.take_file_action() {
            // A model of a Jig in use is made on the engine thread, as
            // every command that builds a document's scene.
            if action == FileAction::OpenForEditing {
                self.start(
                    Waiting::File("jig.edit"),
                    "jig.edit".to_string(),
                    Box::new(|session| {
                        Outcome::Ran(
                            session
                                .execute("jig.edit", &json!({}))
                                .map_err(|e| e.message().to_string()),
                        )
                    }),
                );
                return true;
            }
            self.host.file_action(action, &mut self.picker);
            return true;
        }
        if let Some(action) = self.host.shell.take_doc_action() {
            self.doc_action(action);
            return true;
        }
        // Undo and Redo of the Edit menu and Ctrl+Z, on the engine thread
        // like every command that rebuilds the scene. Reported as a file
        // action is: not a commit of the Parameters form, which judges
        // only its own (an undo back to a failing state is no value of
        // the form's that broke the model).
        if let Some(request) = self.host.shell.take_edit_action() {
            let command = request.action.command();
            self.start(
                Waiting::File(command),
                command.to_string(),
                Box::new(move |session| Outcome::Ran(craft3d_ui_egui::run_edit(session, request))),
            );
            return true;
        }
        if let Some(path) = self.host.shell.take_open_recent() {
            let path = std::path::PathBuf::from(path);
            self.start_about(
                Waiting::File("file.open"),
                "file.open".to_string(),
                file_stem(&path.to_string_lossy()),
                Box::new(move |session| {
                    Outcome::Ran(files::run_as_user(session, "file.open", &path, false, None))
                }),
            );
            return true;
        }
        if let Some(asked) = self.host.shell.take_command() {
            let label = asked.command;
            self.start(
                Waiting::Command(label),
                label.to_string(),
                Box::new(move |session| {
                    Outcome::Ran(
                        session
                            .execute(asked.command, &asked.params)
                            .map_err(|e| e.message().to_string()),
                    )
                }),
            );
            return true;
        }
        if let Some(example) = self.host.shell.take_example() {
            self.start(
                Waiting::File("example"),
                format!("example {}", example.title),
                Box::new(move |session| {
                    Outcome::Ran(craft3d_ui_egui::shell::run_example(session, example))
                }),
            );
            return true;
        }
        self.host.pump_places()
    }

    /// Exports to the file the person chose, with the parameters made when
    /// the export dialog was answered, on the engine thread.
    fn start_export(
        &mut self,
        path: std::path::PathBuf,
        params: files::ExportParams,
        done: files::ExportDone,
    ) {
        self.start(
            Waiting::Export(Box::new(done)),
            "file.export".to_string(),
            Box::new(move |session| {
                Outcome::Ran(files::run_as_user_with(
                    session,
                    "file.export",
                    &path,
                    true,
                    params,
                ))
            }),
        );
    }

    /// What a tab asked for, on the engine thread like every command, so
    /// the window keeps drawing while the scene of a heavy document is
    /// built. Save and close for a document without a file activates it
    /// there and opens the save dialog; it is closed once it is saved.
    fn doc_action(&mut self, action: DocAction) {
        let label = tabs::label(&action);
        if tabs::needs_a_name(&self.host.session, &action) {
            let DocAction::SaveAndClose(document) = action else {
                return;
            };
            self.host.ask_name_then_close(&mut self.picker, document);
            self.start(
                Waiting::File("file.activate"),
                "file.activate".to_string(),
                Box::new(move |session| {
                    Outcome::Ran(tabs::perform(session, &DocAction::Activate(document)))
                }),
            );
            return;
        }
        self.start(
            Waiting::File(label),
            label.to_string(),
            Box::new(move |session| Outcome::Ran(tabs::perform(session, &action))),
        );
    }

    /// Tells the shell what the engine thread is doing.
    fn show_work(&mut self) {
        let work = if self.away() {
            self.worker.as_ref().and_then(Worker::work)
        } else {
            None
        };
        self.host.shell.set_work(work);
    }

    /// The next step of closing the window ([`Shell::advance_quit`]),
    /// once the session is here and nothing the last step asked for is
    /// pending: a document action, the save dialog of Save for a
    /// document without a file, or the save of the file chosen there.
    fn advance_quit(&mut self) -> bool {
        if self.away() || self.host.close_after_save || !self.host.shell.quitting() {
            return false;
        }
        self.host.shell.advance_quit()
    }

    /// The window's close button: true when the window may close now;
    /// otherwise it stays and asks about the documents with unsaved
    /// changes first ([`Shell::request_window_close`]).
    pub fn window_close(&mut self) -> bool {
        // While a command runs the tabs may not show what it leaves
        // unsaved: decided once it is done.
        if self.away() {
            return self.host.shell.request_window_close_when_idle();
        }
        self.host.shell.request_window_close()
    }

    /// Whether the autosaves of the documents may go as the application
    /// ends on purpose: nothing unsaved is left that was not dropped on
    /// purpose. A document that was changed while the window closed (by
    /// work that finished after the questions) keeps its autosave, as
    /// after a crash.
    pub fn may_end_recovery(&self) -> bool {
        self.host.shell.quit_discards()
            || !self
                .host
                .session
                .document_statuses()
                .iter()
                .any(|status| status.close_asks)
    }

    /// Waits at most `timeout` for the work in progress, so the session is
    /// back: for an application that is closing. False when it did not
    /// come back in time.
    pub fn settle(&mut self, timeout: Duration) -> bool {
        if !self.away() {
            return true;
        }
        let finished = self
            .worker
            .as_ref()
            .and_then(|worker| worker.wait_finished(timeout));
        match finished {
            Some(finished) => {
                self.complete(finished);
                true
            }
            None => false,
        }
    }
}

/// The `id` of a request, `null` when it has none.
fn request_id(line: &str) -> Value {
    serde_json::from_str::<Value>(line)
        .ok()
        .and_then(|request| request.get("id").cloned())
        .unwrap_or(Value::Null)
}

impl Desk {
    /// Before a frame: asks egui for a picture of the frame once a
    /// capture has waited its frames.
    fn ask_capture(&mut self, ctx: &egui::Context) {
        // Still averaging, or the look on its way (the environment being
        // prepared, a popover fading in): not yet.
        let now = ctx.input(|i| i.time);
        let converging = self.host.shell.converging() || self.host.shell.look_settling(ctx, now);
        self.ask_capture_when(ctx, converging);
    }

    /// [`Self::ask_capture`] with whether the viewport is still averaging
    /// its frames: then the frames are not counted, so the picture is the
    /// settled still and not a timing-dependent part of the way there; the
    /// capture's deadline still bounds the wait.
    fn ask_capture_when(&mut self, ctx: &egui::Context, converging: bool) {
        let Some(capture) = self.capture.as_mut() else {
            return;
        };
        if capture.image.is_some() {
            return;
        }
        capture.settling = converging && capture.asked.is_none();
        if capture.settling {
            ctx.request_repaint();
            return;
        }
        capture.frames = capture.frames.saturating_add(1);
        let ask = match capture.asked {
            None => capture.frames >= CAPTURE_FRAMES,
            Some(at) => capture.frames.saturating_sub(at) >= CAPTURE_RETRY_FRAMES,
        };
        if ask {
            capture.asked = Some(capture.frames);
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        ctx.request_repaint();
    }

    /// A frame egui drew, as asked for: kept for the capture, which is
    /// answered in the next turn the session is here.
    fn captured(&mut self, image: Arc<egui::ColorImage>) {
        if let Some(capture) = self.capture.as_mut().filter(|c| c.image.is_none()) {
            capture.image = Some(image);
        }
    }

    /// Answers the capture: with its picture once there is one and the
    /// session is here, with an error once its deadline has passed.
    /// Returns whether it was answered.
    fn settle_capture(&mut self) -> bool {
        let Some(capture) = self.capture.as_ref() else {
            return false;
        };
        let ready = capture.image.is_some() && !self.away();
        let late = capture.since.elapsed() >= self.capture_deadline;
        if !(ready || late) {
            return false;
        }
        let Some(capture) = self.capture.take() else {
            return false;
        };
        let id = request_id(&capture.request.line);
        let reply = match (&capture.image, ready) {
            (Some(image), true) => match self.host.shell.window_picture(
                &mut self.host.session,
                image,
                capture.path.as_deref(),
            ) {
                Ok(result) => rpc::ok_reply(&id, &result),
                Err(error) => rpc::error_reply(&id, &error),
            },
            _ if capture.settling => rpc::error_reply(&id, NOT_SETTLED),
            _ => rpc::error_reply(&id, NO_FRAME),
        };
        let _ = capture.request.reply.send(Some(reply));
        true
    }

    /// Takes `request` when it asks for a picture of the window: it becomes
    /// the capture, or is answered with why its parameters are wrong.
    /// Gives any other request back. Only with the session here.
    fn intercept_capture(&mut self, request: Request) -> Option<Request> {
        match Shell::window_picture_request(&self.host.session, &request.line) {
            Some(Ok(path)) => {
                self.capture = Some(Capture {
                    request,
                    path,
                    frames: 0,
                    settling: false,
                    asked: None,
                    since: Instant::now(),
                    image: None,
                });
                self.ring_at_deadline();
                None
            }
            Some(Err(error)) => {
                let id = request_id(&request.line);
                let _ = request.reply.send(Some(rpc::error_reply(&id, &error)));
                None
            }
            None => Some(request),
        }
    }

    /// Wakes the desk once the capture's deadline has passed, from a
    /// thread of its own: the deadline is kept without frames
    /// ([`Self::with_wake`]). Without a wake, or when no thread can be
    /// started, the deadline is kept at the next turn as before.
    fn ring_at_deadline(&mut self) {
        // A little after the deadline, so the turn it wakes finds it
        // passed.
        let Some(at) =
            Instant::now().checked_add(self.capture_deadline.saturating_add(DEADLINE_GRACE))
        else {
            return;
        };
        if let Some(timer) = &self.deadlines
            && timer.send(at).is_ok()
        {
            return;
        }
        // No timer yet, or it ended: one for the deadlines from now on.
        let Some(wake) = self.wake.clone() else {
            return;
        };
        let (deadlines, incoming) = mpsc::channel::<Instant>();
        let started = std::thread::Builder::new()
            .name("craft3d-capture-deadline".to_string())
            .spawn(move || {
                // Captures come one at a time, so the deadlines arrive in
                // order; the thread ends with the desk.
                while let Ok(at) = incoming.recv() {
                    let now = Instant::now();
                    if at > now {
                        std::thread::sleep(at - now);
                    }
                    wake();
                }
            });
        if started.is_ok() && deadlines.send(at).is_ok() {
            self.deadlines = Some(deadlines);
        }
    }

    /// Answers the queued `ui.get` and `ui.quit` now, from the session
    /// here, while the rest waits for the capture.
    fn answer_anytime_now(&mut self) {
        for request in std::mem::take(&mut self.queue) {
            if route(&request.line) == Route::Anytime {
                self.host.answer(request);
            } else {
                self.queue.push_back(request);
            }
        }
    }
}

impl Desk {
    /// Takes `request` when it is `ui.record` or `ui.advance`: answered
    /// here, or (an advance) kept until its frames are drawn. Gives any
    /// other request back. Only with the session here.
    fn intercept_record(&mut self, request: Request) -> Option<Request> {
        let Ok(parsed) = serde_json::from_str::<Value>(&request.line) else {
            return Some(request);
        };
        let id = parsed.get("id").cloned().unwrap_or(Value::Null);
        // Through MCP's `control_call` too, answered in its envelope.
        let (method, params, mcp) = match crate::host::control_call_of(&parsed) {
            Some((method, params, line)) => (method, params, Some(line)),
            None => (
                parsed
                    .get("method")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                parsed.get("params").cloned(),
                None,
            ),
        };
        let result = match method.as_str() {
            "ui.record" => self.ui_record(params.as_ref()),
            "ui.advance" => match self.ui_advance(params.as_ref()) {
                Ok((left, dt, timeout)) => {
                    self.rec.advance = Some(Advance {
                        request,
                        mcp,
                        left,
                        dt,
                        drawn: 0,
                        unsettled: 0,
                        waiting: None,
                        began: Instant::now(),
                        timeout: Duration::from_secs_f64(timeout),
                        timing: crate::recording::Timing::default(),
                    });
                    // Woken now and then while it waits for the window to
                    // settle, so its deadline is kept without other events.
                    self.rec.start_ticker(ADVANCE_TICK, self.wake.clone());
                    if let Some(wake) = &self.wake {
                        wake();
                    }
                    return None;
                }
                Err(error) => Err(error),
            },
            _ => return Some(request),
        };
        let reply = self.record_reply(&id, mcp.as_deref(), result);
        let _ = request.reply.send(Some(reply));
        None
    }

    /// The reply line to a `ui.record` or `ui.advance` with `id`, wrapped
    /// as `control_call`'s when it came in the MCP message `mcp`.
    fn record_reply(
        &mut self,
        id: &Value,
        mcp: Option<&str>,
        result: Result<Value, String>,
    ) -> String {
        match mcp {
            Some(line) => {
                let wrapped = crate::host::control_call_reply(&mut self.host.session, line, result);
                rpc::ok_reply(id, &wrapped)
            }
            None => match result {
                Ok(result) => rpc::ok_reply(id, &result),
                Err(error) => rpc::error_reply(id, &error),
            },
        }
    }

    fn ui_record(&mut self, params: Option<&Value>) -> Result<Value, String> {
        use craft3d_ui_egui::record::{RecordAsk, record_params};
        let ask = record_params(params)?;
        // A real-time recording that failed between requests says so.
        if let Some(error) = self.rec.failure.take() {
            return Err(error);
        }
        if self.rec.recorder.is_none() {
            return Err(
                "ui.record: this window has no wgpu device to paint its frames on".to_string(),
            );
        }
        let now = self.rec.now();
        let rec = &mut self.rec;
        let Some(recorder) = rec.recorder.as_mut() else {
            return Err("ui.record: the recorder is gone".to_string());
        };
        match ask {
            RecordAsk::Status => Ok(recorder.status()),
            RecordAsk::Stop => {
                self.host.shell.set_frame_exact(false);
                let stopped = recorder.stop();
                rec.stop_ticker();
                rec.next_frame = None;
                rec.events.clear();
                rec.mirror = None;
                stopped
            }
            RecordAsk::Start(options) => {
                let shown = self.host.shell.pixels_per_point();
                let pixels_per_point = if shown.is_finite() && shown > 0.0 {
                    shown
                } else {
                    1.0
                };
                let mut input = rec.input.clone();
                input.time = Some(now);
                let viewport = self.host.shell.viewport_rect();
                let real_time = !options.step;
                let fps = options.fps;
                let status = recorder.start(
                    &self.host.session,
                    options,
                    &input,
                    pixels_per_point,
                    viewport,
                )?;
                self.host.shell.set_frame_exact(!real_time);
                if real_time {
                    rec.next_frame = Some(Instant::now());
                    rec.start_ticker(Duration::from_secs_f64(1.0 / fps), self.wake.clone());
                }
                Ok(status)
            }
        }
    }

    fn ui_advance(&mut self, params: Option<&Value>) -> Result<(u64, f64, f64), String> {
        let recording = self
            .rec
            .recorder
            .as_ref()
            .and_then(|r| r.recording.as_ref())
            .filter(|r| r.options.step)
            .ok_or_else(|| {
                "ui.advance: no frame-exact recording runs; start one with ui.record {path, step: true}"
                    .to_string()
            })?;
        craft3d_ui_egui::record::advance_params(params, recording.options.fps)
    }

    /// One pass of the shell at `time` with `events`, painted offscreen,
    /// read back when `read`. The shell is drawn as the window draws it:
    /// with the session when it is here, as it is away otherwise.
    fn offscreen_pass(
        &mut self,
        ctx: &egui::Context,
        time: f64,
        events: Vec<egui::Event>,
        read: bool,
    ) -> Result<Option<craft3d_ui_egui::record::Frame>, String> {
        let started = Instant::now();
        let away = self.away();
        let mut input = self.rec.input.clone();
        input.time = Some(time);
        input.events = events;
        // egui's animations (a panel fading in) step by the clock's step
        // when no repaint was asked for in the last pass, by the time
        // between passes otherwise: on the recording's clock either way.
        input.predicted_dt = self.rec.pass_dt;
        let recorder = self
            .rec
            .recorder
            .as_mut()
            .ok_or("this window has no wgpu device to paint its frames on")?;
        let Host { session, shell, .. } = &mut self.host;
        let painted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            recorder.pass(ctx, input, read, |ui| {
                if away {
                    shell.ui_away(ui);
                } else {
                    shell.ui(ui, session);
                }
            })
        }));
        let painted = painted.unwrap_or_else(|_| Err(PANICKED.to_string()));
        if let Some(advance) = self.rec.advance.as_mut() {
            let timing = &mut advance.timing;
            timing.passes = timing.passes.saturating_add(1);
            let took = started.elapsed().as_secs_f64();
            if read {
                timing.read_passes += took;
            } else {
                timing.other_passes += took;
            }
        }
        painted
    }

    /// A frame at `time`, read back after `passes` passes, once the
    /// viewport's picture is done: further passes at the same moment while
    /// the CPU renderer makes its sharp image, or while the viewport
    /// averages its still for up to `average` frames in all, within one
    /// deadline ([`SETTLE_TIME`]). Only the last pass is read back. True
    /// with the frame when it settled.
    fn settled_frame(
        &mut self,
        ctx: &egui::Context,
        time: f64,
        events: Vec<egui::Event>,
        passes: u32,
        average: u32,
    ) -> Result<(craft3d_ui_egui::record::Frame, bool), String> {
        let began = Instant::now();
        let passes = passes.max(1);
        let mut events = Some(events);
        let mut frame = None;
        for pass in 1..=passes {
            frame =
                self.offscreen_pass(ctx, time, events.take().unwrap_or_default(), pass == passes)?;
        }
        let mut averaged = 1u32;
        let needs = |desk: &Desk, averaged: u32| {
            let sharp = desk.host.shell.sharp_pending();
            (
                sharp,
                sharp || (desk.host.shell.converging() && averaged < average),
            )
        };
        let (_, mut more) = needs(self, averaged);
        if more {
            // Passes not read back until the last.
            loop {
                let left = SETTLE_TIME.saturating_sub(began.elapsed());
                if left.is_zero() || averaged >= SETTLE_PASSES {
                    break;
                }
                if self.host.shell.sharp_pending() {
                    self.host.shell.wait_for_sharp(left);
                }
                self.offscreen_pass(ctx, time, Vec::new(), false)?;
                averaged = averaged.saturating_add(1);
                more = needs(self, averaged).1;
                if !more {
                    break;
                }
            }
            frame = self.offscreen_pass(ctx, time, Vec::new(), true)?;
        }
        let settled = !self.host.shell.sharp_pending() && !more;
        let frame = frame.ok_or_else(|| "no frame was painted".to_string())?;
        Ok((frame, settled))
    }

    /// Whether the window has settled for the next frame of a frame-exact
    /// recording: no command running, the scene built and refined.
    fn settled(&mut self) -> bool {
        !self.away() && self.capture.is_none() && self.scene_ready() && !self.refining()
    }

    /// What the window paints offscreen, where the event loop is woken
    /// (frame or no frame): a picture of the window asked for, the next
    /// frame of a real-time recording when it is due, the frames of a
    /// `ui.advance` while the window is settled. True when something was
    /// drawn or answered.
    pub fn drive(&mut self, ctx: &egui::Context) -> bool {
        if self.rec.recorder.is_none() {
            return false;
        }
        let mut changed = false;
        // A picture of the window, painted offscreen: it needs no frame
        // of the compositor. It shows the display scene, as one taken
        // from a frame does.
        if self.capture.as_ref().is_some_and(|c| c.image.is_none())
            && !self.away()
            && !self.refining()
        {
            let time = self.rec.now();
            self.rec.pass_dt = 1.0 / 60.0;
            // The picture shows the look asked for: its environment is
            // prepared now rather than the plain look drawn meanwhile (a
            // look that cannot be prepared is drawn plain, as on screen).
            let _ = self.host.shell.prepare_look();
            let picture = self
                .settled_frame(ctx, time, Vec::new(), PICTURE_PASSES, SETTLE_PASSES)
                .and_then(|(frame, settled)| self.faded_in(ctx, time, frame, settled));
            match picture {
                Ok((frame, _)) => {
                    if let Some(capture) = self.capture.as_mut() {
                        capture.image = Some(Arc::new(frame.color_image()));
                    }
                }
                Err(error) => {
                    if let Some(capture) = self.capture.take() {
                        let id = request_id(&capture.request.line);
                        let reply = rpc::error_reply(&id, &format!("ui.screenshot: {error}"));
                        let _ = capture.request.reply.send(Some(reply));
                    }
                }
            }
            self.settle_capture();
            changed = true;
        }
        if self.real_time_frame(ctx) {
            changed = true;
        }
        if self.rec.advance.is_some() && self.advance_frames(ctx) {
            changed = true;
        }
        changed
    }

    /// `frame`, painted at `time`, or a later one once nothing moves into
    /// place any more ([`Shell::animating`]: a popover fading in, the
    /// drawer sliding): the passes of a picture are all at one moment, so
    /// one opened by the request before it would be caught half drawn. Further passes step the clock by a pass each, at most
    /// [`FADE_PASSES`].
    fn faded_in(
        &mut self,
        ctx: &egui::Context,
        time: f64,
        frame: craft3d_ui_egui::record::Frame,
        settled: bool,
    ) -> Result<(craft3d_ui_egui::record::Frame, bool), String> {
        let mut frame = frame;
        let mut at = time;
        for _ in 0..FADE_PASSES {
            if !self.host.shell.animating(ctx, at) {
                break;
            }
            at += f64::from(self.rec.pass_dt);
            if let Some(next) = self.offscreen_pass(ctx, at, Vec::new(), true)? {
                frame = next;
            }
        }
        Ok((frame, settled))
    }

    /// The next frame of a real-time recording, when it is due.
    fn real_time_frame(&mut self, ctx: &egui::Context) -> bool {
        let Some(due) = self.rec.next_frame else {
            return false;
        };
        let now = Instant::now();
        if now < due {
            return false;
        }
        let Some(fps) = self
            .rec
            .recorder
            .as_ref()
            .and_then(|r| r.recording.as_ref())
            .filter(|r| !r.options.step)
            .map(|r| r.options.fps)
        else {
            self.rec.next_frame = None;
            return false;
        };
        let period = Duration::from_secs_f64(1.0 / fps);
        let events = std::mem::take(&mut self.rec.events);
        let time = self.rec.now();
        self.rec.pass_dt = (1.0 / fps) as f32;
        let drawn = self
            .offscreen_pass(ctx, time, events, true)
            .and_then(|frame| frame.ok_or_else(|| "no frame was painted".to_string()))
            .and_then(|frame| {
                let recorder = self.rec.recorder.as_mut().ok_or("the recorder is gone")?;
                recorder.push(&frame)?;
                if let Some(recording) = recorder.recording.as_mut() {
                    recording.time = time;
                }
                Ok(frame)
            });
        match drawn {
            Ok(frame) => self.rec.mirror = Some(frame.color_image()),
            Err(error) => {
                // Ended, and said at the next `ui.record`.
                self.rec.stop_ticker();
                self.rec.next_frame = None;
                self.rec.mirror = None;
                if let Some(recorder) = self.rec.recorder.as_mut() {
                    let _ = recorder.stop();
                }
                self.rec.failure = Some(format!("ui.record: the recording stopped: {error}"));
                return true;
            }
        }
        // Frames that could not be drawn in time are skipped, not drawn
        // late in a burst.
        let mut next = due.checked_add(period).unwrap_or(now);
        if next <= now {
            let behind = now.saturating_duration_since(next).as_secs_f64() / period.as_secs_f64();
            if let Some(recording) = self
                .rec
                .recorder
                .as_mut()
                .and_then(|r| r.recording.as_mut())
            {
                recording.skipped = recording.skipped.saturating_add(behind as u64 + 1);
            }
            next = now.checked_add(period).unwrap_or(now);
        }
        self.rec.next_frame = Some(next);
        true
    }

    /// Draws the frames of the `ui.advance` that waits, each once the
    /// window has settled, for at most [`ADVANCE_SLICE`]; answers it when
    /// the last is drawn. True when a frame was drawn or it was answered.
    fn advance_frames(&mut self, ctx: &egui::Context) -> bool {
        let began = Instant::now();
        let mut changed = false;
        loop {
            let Some(advance) = self.rec.advance.as_ref() else {
                return changed;
            };
            // Done, or out of time: the frames drawn so far, and how many
            // are left for the client to ask for again.
            let timed_out = advance.began.elapsed() >= advance.timeout;
            if advance.left == 0 || timed_out {
                let status = self.rec.recorder.as_ref().map(|r| r.status());
                let result = json!({
                    "frames": advance.drawn,
                    "left": advance.left,
                    "complete": advance.left == 0,
                    "unsettled": advance.unsettled,
                    "recorded": status.as_ref().and_then(|s| s.get("frames")).cloned(),
                    "time": status.as_ref().and_then(|s| s.get("time")).cloned(),
                    "seconds": advance.began.elapsed().as_secs_f64(),
                    "timing": advance.timing.per_frame(advance.drawn),
                });
                if let Some(advance) = self.rec.advance.take() {
                    self.rec.stop_ticker();
                    let id = request_id(&advance.request.line);
                    let reply = self.record_reply(&id, advance.mcp.as_deref(), Ok(result));
                    let _ = advance.request.reply.send(Some(reply));
                }
                return true;
            }
            let dt = advance.dt;
            if began.elapsed() >= ADVANCE_SLICE {
                // The rest in the next turn: the window and the channel
                // get theirs meanwhile.
                if let Some(wake) = &self.wake {
                    wake();
                }
                return changed;
            }
            // What the last frame asked for (a step of the player, a
            // command of the shell) is started, and its scene built,
            // before the next frame is drawn; the engine thread wakes the
            // desk when it is done.
            self.turn();
            if !self.settled() {
                let waited = self
                    .rec
                    .advance
                    .as_mut()
                    .map(|a| a.waiting.get_or_insert_with(Instant::now).elapsed());
                if waited.is_some_and(|w| w >= ADVANCE_SETTLE) {
                    let drawn = self.rec.advance.as_ref().map_or(0, |a| a.drawn);
                    self.fail_advance(format!(
                        "ui.advance: the window did not settle within {} s (a command or a scene still running); {drawn} frames were drawn",
                        ADVANCE_SETTLE.as_secs()
                    ));
                    return true;
                }
                return changed;
            }
            if let Some(advance) = self.rec.advance.as_mut()
                && let Some(since) = advance.waiting.take()
            {
                advance.timing.waiting += since.elapsed().as_secs_f64();
            }
            let Some(recording) = self
                .rec
                .recorder
                .as_ref()
                .and_then(|r| r.recording.as_ref())
            else {
                self.fail_advance("ui.advance: the recording has ended".to_string());
                return true;
            };
            let time = recording.time + dt;
            let average = recording.options.accumulate;
            self.rec.pass_dt = dt as f32;
            let drawn = self
                .settled_frame(ctx, time, Vec::new(), 1, average)
                .and_then(|(frame, settled)| {
                    if !settled && let Some(advance) = self.rec.advance.as_mut() {
                        advance.unsettled = advance.unsettled.saturating_add(1);
                    }
                    let pushed = Instant::now();
                    let recorder = self.rec.recorder.as_mut().ok_or("the recorder is gone")?;
                    recorder.push(&frame)?;
                    if let Some(advance) = self.rec.advance.as_mut() {
                        advance.timing.push += pushed.elapsed().as_secs_f64();
                    }
                    let recorder = self.rec.recorder.as_mut().ok_or("the recorder is gone")?;
                    if let Some(recording) = recorder.recording.as_mut() {
                        recording.time = time;
                    }
                    Ok(frame)
                });
            changed = true;
            match drawn {
                Ok(frame) => {
                    self.rec.mirror = Some(frame.color_image());
                    if let Some(advance) = self.rec.advance.as_mut() {
                        advance.left = advance.left.saturating_sub(1);
                        advance.drawn = advance.drawn.saturating_add(1);
                    }
                }
                Err(error) => {
                    self.fail_advance(format!("ui.advance: {error}"));
                    return true;
                }
            }
        }
    }

    /// Answers a queued `ui.record {stop: true}` now, while an advance
    /// draws its frames: the advance ends with an error that says how far
    /// it got, then the recording stops. The rest keeps its place.
    fn answer_record_stop_now(&mut self) {
        if self.rec.advance.is_none() {
            return;
        }
        let at = self.queue.iter().position(|r| is_record_stop(&r.line));
        let Some(request) = at.and_then(|at| self.queue.remove(at)) else {
            return;
        };
        let drawn = self.rec.advance.as_ref().map_or(0, |a| a.drawn);
        self.fail_advance(format!(
            "ui.advance: stopped by ui.record {{stop: true}} after {drawn} frames"
        ));
        if let Some(request) = self.intercept_record(request) {
            self.queue.push_front(request);
        }
    }

    fn fail_advance(&mut self, error: String) {
        if let Some(advance) = self.rec.advance.take() {
            self.rec.stop_ticker();
            let id = request_id(&advance.request.line);
            let reply = self.record_reply(&id, advance.mcp.as_deref(), Err(error));
            let _ = advance.request.reply.send(Some(reply));
        }
    }

    /// Whether the window shows the recording's frames instead of drawing
    /// the shell itself: while a recording runs.
    pub fn mirrored(&self) -> bool {
        self.rec.mirrored()
    }
}

/// Whether `line` is `ui.record {stop: true}`, plain or through MCP's
/// `control_call`.
fn is_record_stop(line: &str) -> bool {
    let Ok(request) = serde_json::from_str::<Value>(line) else {
        return false;
    };
    let (method, params) = match crate::host::control_call_of(&request) {
        Some((method, params, _)) => (method, params),
        None => (
            request
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            request.get("params").cloned(),
        ),
    };
    method == "ui.record" && params.as_ref().and_then(|p| p.get("stop")) == Some(&Value::Bool(true))
}

/// True for a `ui.screenshot` request.
/// Whether `line` is a request of `method`.
fn is_method(line: &str, method: &str) -> bool {
    serde_json::from_str::<Value>(line)
        .ok()
        .and_then(|request| {
            request
                .get("method")
                .and_then(Value::as_str)
                .map(|m| m == method)
        })
        .unwrap_or(false)
}

fn is_screenshot(line: &str) -> bool {
    serde_json::from_str::<Value>(line)
        .ok()
        .and_then(|request| {
            request
                .get("method")
                .and_then(Value::as_str)
                .map(|m| m == "ui.screenshot")
        })
        .unwrap_or(false)
}

/// What a request is about, for the status bar: the file `file.open`
/// opens.
fn request_subject(line: &str) -> Option<String> {
    let request: Value = serde_json::from_str(line).ok()?;
    let params = request.get("params")?;
    if request.get("method")?.as_str()? != "engine.execute"
        || params.get("command")?.as_str()? != "file.open"
    {
        return None;
    }
    file_stem(params.get("params")?.get("path")?.as_str()?)
}

/// What the window says it is doing for a request: the command it runs,
/// `batch of N`, or the method.
fn request_label(line: &str) -> String {
    let request: Value = serde_json::from_str(line).unwrap_or(Value::Null);
    let params = request.get("params");
    let label = match request.get("method").and_then(Value::as_str) {
        Some("engine.execute") => params
            .and_then(|p| p.get("command"))
            .and_then(Value::as_str)
            .map(str::to_string),
        Some("batch") => params
            .and_then(|p| p.get("steps"))
            .and_then(Value::as_array)
            .map(|steps| format!("batch of {}", steps.len())),
        Some("mcp.message") => Some("MCP tool call".to_string()),
        Some(method) => Some(method.to_string()),
        None => None,
    };
    label
        .map(|label| label.chars().take(80).collect())
        .unwrap_or_else(|| "request".to_string())
}

/// Most requests a turn takes from the queue, so a flood of quick ones
/// cannot keep the window from drawing.
const MAX_PER_TURN: usize = 64;

struct Viewer {
    desk: Rc<RefCell<Desk>>,
    /// The last frame recorded, shown while a recording draws the shell.
    mirror: Option<egui::TextureHandle>,
    /// The title the window has now.
    title: String,
    /// Kept so the listener lives as long as the window.
    _server: Option<ControlServer>,
}

impl eframe::App for Viewer {
    /// Runs before every frame. The requests are served by the event loop
    /// too ([`Driver`]), which does not need a frame.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Borrowed elsewhere only if a frame were drawn from within a
        // turn, which nothing does; then that turn does the work.
        let Ok(mut desk) = self.desk.try_borrow_mut() else {
            return;
        };
        if desk.turn().changed {
            // Show what changed, and look for more.
            ctx.request_repaint();
        }
        // The window's close button (or the window manager): the window
        // stays and asks first when a document has unsaved changes.
        if ctx.input(|i| i.viewport().close_requested()) && !desk.window_close() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.request_repaint();
        }
        desk.ask_capture(ctx);
        let title = if desk.away() {
            window_title(desk.host.shell.shown_status())
        } else {
            window_title(desk.host.session.document_status().as_ref())
        };
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
        // `ui.quit` ends the event loop where it is woken (`Driver`).
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let Ok(mut desk) = self.desk.try_borrow_mut() else {
            return;
        };
        let shot = ui.input(|i| {
            i.raw.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(Arc::clone(image)),
                _ => None,
            })
        });
        if let Some(image) = shot {
            desk.captured(image);
        }
        // What the passes painted offscreen run with: this frame's size,
        // pixels per point, modifiers and (while recording in real time)
        // events.
        ui.input(|i| desk.rec.window_frame(&i.raw));
        if desk.mirrored() {
            // The recording draws the shell; the window shows its frames.
            if let Some(image) = desk.rec.mirror.take() {
                match &mut self.mirror {
                    Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
                    None => {
                        self.mirror = Some(ui.ctx().load_texture(
                            "craft3d-recording",
                            image,
                            egui::TextureOptions::LINEAR,
                        ));
                    }
                }
            }
            let rect = ui.max_rect();
            ui.painter().rect_filled(rect, 0.0, egui::Color32::BLACK);
            if let Some(texture) = &self.mirror {
                let size = texture.size_vec2() / ui.ctx().pixels_per_point();
                let scale = (rect.width() / size.x).min(rect.height() / size.y).min(1.0);
                let shown = egui::Rect::from_min_size(rect.min, size * scale);
                ui.painter().image(
                    texture.id(),
                    shown,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }
            return;
        }
        self.mirror = None;
        if desk.away() {
            desk.host.shell.ui_away(ui);
        } else {
            let Host { session, shell, .. } = &mut desk.host;
            shell.ui(ui, session);
        }
    }
}

/// The event loop's handler: eframe's own, with the desk served whenever
/// the loop is woken for it, frame or no frame.
struct Driver<'a> {
    eframe: eframe::EframeWinitApplication<'a>,
    desk: Rc<RefCell<Desk>>,
    /// Set once the window's shell exists: nothing is answered before.
    context: Arc<OnceLock<egui::Context>>,
}

impl Driver<'_> {
    fn serve(&mut self, event_loop: &ActiveEventLoop) {
        let Some(ctx) = self.context.get() else {
            return;
        };
        // Borrowed already only inside a frame, which serves the desk
        // itself.
        let Ok(mut desk) = self.desk.try_borrow_mut() else {
            return;
        };
        let mut turn = desk.turn();
        // What is painted offscreen needs no frame: a recording, a picture
        // of a window that is not on screen. What it drew may have asked
        // for more (a step of the player): served again, a few times.
        for _ in 0..8 {
            if !desk.drive(ctx) {
                break;
            }
            let again = desk.turn();
            turn.changed = true;
            turn.quit |= again.quit;
        }
        drop(desk);
        if turn.changed {
            ctx.request_repaint();
        }
        if turn.quit {
            event_loop.exit();
        }
    }
}

impl ApplicationHandler<UserEvent> for Driver<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.eframe.resumed(event_loop);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        self.eframe.window_event(event_loop, id, event);
        // A frame may have answered `ui.quit`.
        self.serve(event_loop);
    }

    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        self.eframe.new_events(event_loop, cause);
    }

    /// Where a wake arrives ([`waker`]): the requests are answered here,
    /// whether or not the compositor lets the window draw.
    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        self.serve(event_loop);
        self.eframe.user_event(event_loop, event);
    }

    fn device_event(&mut self, event_loop: &ActiveEventLoop, id: DeviceId, event: DeviceEvent) {
        self.eframe.device_event(event_loop, id, event);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.eframe.about_to_wait(event_loop);
    }

    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        self.eframe.suspended(event_loop);
    }

    fn exiting(&mut self, event_loop: &ActiveEventLoop) {
        self.eframe.exiting(event_loop);
    }

    fn memory_warning(&mut self, event_loop: &ActiveEventLoop) {
        self.eframe.memory_warning(event_loop);
    }
}

/// What wakes the thread that owns the session: an event of the event
/// loop itself, which arrives whether or not a frame is drawn, and which
/// eframe takes as a request to repaint. `send` posts one event; `pass`
/// is egui's count of passes, 0 before there is a window.
///
/// Not `egui::Context::request_repaint`: that reaches the application
/// only through a frame, and asks for nothing at all while an earlier
/// request for a frame is still waiting.
pub fn waker(
    send: impl Fn(UserEvent) + Send + Sync + 'static,
    pass: impl Fn() -> u64 + Send + Sync + 'static,
) -> Wake {
    Arc::new(move || {
        send(UserEvent::RequestRepaint {
            viewport_id: egui::ViewportId::ROOT,
            when: Instant::now(),
            cumulative_pass_nr: pass(),
        });
    })
}

/// Opens the window and runs until it is closed. `announce` is told where
/// the control channel listens, once it does.
pub fn run(options: &Options, announce: &dyn Fn(&str)) -> Result<(), String> {
    let mut session = session(options)?;
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .map_err(|e| window_error(&eframe::Error::WinitEventLoop(e)))?;
    let context: Arc<OnceLock<egui::Context>> = Arc::new(OnceLock::new());
    let for_pass = Arc::clone(&context);
    // The proxy behind a lock: whether it may be shared between threads
    // as it is differs per platform.
    let proxy = Mutex::new(event_loop.create_proxy());
    let wake = waker(
        move |event| {
            // A loop that has ended has nobody to wake.
            let _ = proxy
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .send_event(event);
        },
        move || {
            for_pass
                .get()
                .map_or(0, |ctx| ctx.cumulative_pass_nr_for(egui::ViewportId::ROOT))
        },
    );
    // The listener starts before the window, so a port that is taken is an
    // error on the command line, not a window without a control channel.
    let (server, requests) = control(options, Arc::clone(&wake))?;
    if let (Some(server), Some(token_file)) = (&server, &options.token_file) {
        announce(&format!(
            "control channel on {} (token in {})",
            server.address(),
            token_file.display()
        ));
    }
    // Autosave starts with the window, and looks for what an earlier
    // launch that crashed left behind. Nothing is recovered by itself:
    // the window shows what was found, and the person or an agent decides
    // (`recovery.list`, `recovery.open`, `recovery.discard`).
    if let Some(recovery) = recovery(options, Arc::clone(&wake))? {
        session.enable_recovery(recovery);
        let overview = session.recovery_overview();
        if !overview.found.is_empty() {
            announce(&format!(
                "{} unsaved document(s) of an earlier session can be recovered (in {})",
                overview.found.len(),
                overview.directory.as_deref().unwrap_or_default()
            ));
        }
    }
    let mut host = Host::new(session);
    host.recent_file.clone_from(&options.recent_file);
    // What the start screen and the Help menu show of this window.
    let recent = options
        .recent_file
        .as_deref()
        .map(crate::places::read_recent)
        .unwrap_or_default();
    host.recent_baseline = recent.clone();
    let env = |name: &str| std::env::var(name).ok();
    let data_folder = crate::places::data_folder(options, &env);
    let agent = crate::places::agent_setup(
        server.as_ref().map(|server| server.address().port()),
        options.token_file.as_deref(),
        &env,
    );
    let title = window_title(host.session.document_status().as_ref());
    let picker = Picker::new(Arc::new(SystemDialogs), Arc::clone(&wake));
    let mut desk = Desk::new(host, requests, picker).with_engine_thread(Arc::clone(&wake));
    if let Some([w, h]) = options.window_size {
        desk.set_window_points([w as f32, h as f32]);
    }
    let desk = Rc::new(RefCell::new(desk));
    let native = eframe::NativeOptions {
        viewport: viewport(options, &title),
        ..Default::default()
    };
    let renderer = options.renderer;
    let failure = failures();
    let (for_viewer, for_context) = (Rc::clone(&desk), Arc::clone(&context));
    let eframe = eframe::create_native(
        TITLE,
        native,
        Box::new(move |creation| {
            // The viewport is drawn by the GPU on the window's own wgpu
            // device when the machine has one, into the texture that is
            // shown; the CPU reference renderer is the fallback, and what
            // `--renderer cpu` asks for outright. The window never waits
            // for the CPU renderer: its sharp images are made on the
            // shell's own thread.
            let mut shell = Shell::threaded_with_gpu(renderer, creation.wgpu_render_state.as_ref());
            shell.set_recent_files(recent);
            shell.set_data_folder(data_folder);
            shell.set_agent_setup(agent);
            announce(&format!(
                "viewport renderer: {}",
                shell.renderer_report().describe()
            ));
            if let Ok(mut desk) = for_viewer.try_borrow_mut() {
                desk.host.shell = shell;
            }
            if let Ok(mut desk) = for_viewer.try_borrow_mut() {
                desk.attach_recorder(creation.wgpu_render_state.as_ref());
            }
            let viewer = Viewer {
                mirror: None,
                desk: for_viewer,
                title,
                _server: server,
            };
            // Requests that arrived before the window existed are waiting
            // in the channel; they are answered from here on.
            let _ = for_context.set(creation.egui_ctx.clone());
            wake();
            Ok(Box::new(viewer))
        }),
        &event_loop,
    );
    let mut driver = Driver {
        eframe,
        desk,
        context: Arc::clone(&context),
    };
    let ran = event_loop.run_app(&mut driver);
    // The window goes at once, also when work is still running.
    let desk = Rc::clone(&driver.desk);
    drop(driver);
    // The window was closed, or could not be made: either way the
    // application ends on purpose, and leaves no recovery files. (A crash
    // or a kill never gets here, and leaves them.) Work that is still
    // running gets a moment to bring the session back; work that takes
    // longer is left, and so are the recovery files of the documents with
    // unsaved changes, as after a crash: nothing is lost.
    if let Ok(mut desk) = desk.try_borrow_mut()
        && desk.settle(SETTLE)
    {
        if desk.may_end_recovery() {
            desk.host.session.end_recovery();
        } else {
            // Kept for the next start, written now rather than after the
            // interval.
            desk.host.session.autosave_now();
        }
    }
    ran.map_err(|e| window_error(&eframe::Error::WinitEventLoop(e)))?;
    // eframe ends the loop when the window cannot be made, and keeps the
    // reason to itself: it is only logged.
    if context.get().is_some() {
        return Ok(());
    }
    Err(match failure.take() {
        Some(logged) => logged_window_error(&logged),
        None => "the window could not be opened (no reason was given)".to_string(),
    })
}

/// The last error eframe logged. eframe reports a window it cannot make
/// (no graphics adapter, no display) by logging it and ending the event
/// loop; with an event loop of our own there is no other way to the
/// reason.
pub struct Failures(Mutex<Option<String>>);

impl Failures {
    fn take(&self) -> Option<String> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).take()
    }
}

impl log::Log for Failures {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Error
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) && record.target().starts_with("eframe") {
            let text = record.args().to_string();
            *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(text);
        }
    }

    fn flush(&self) {}
}

/// Starts listening to what eframe logs as errors. When another logger is
/// installed already, nothing is caught and a window that cannot be made
/// is reported without its reason.
fn failures() -> &'static Failures {
    static FAILURES: Failures = Failures(Mutex::new(None));
    if log::set_logger(&FAILURES).is_ok() {
        log::set_max_level(log::LevelFilter::Error);
    }
    &FAILURES
}

/// How eframe words the error that ends its event loop
/// (`eframe::native::run`, `handle_event_result`).
const LOGGED_PREFIX: &str = "Exiting because of error: ";

/// [`window_error`] for an error that only reached us as the line eframe
/// logged: the text of an `eframe::Error` after [`LOGGED_PREFIX`].
pub fn logged_window_error(logged: &str) -> String {
    let text = logged.strip_prefix(LOGGED_PREFIX).unwrap_or(logged);
    match text.strip_prefix(WGPU_PREFIX) {
        Some(detail) => no_adapter_help(detail),
        None => format!("the window could not be opened: {text}"),
    }
}

/// How `eframe::Error::Wgpu` starts when displayed; pinned by a test.
const WGPU_PREFIX: &str = "WGPU error: ";

/// Why the window could not be opened, as a sentence for the terminal.
/// A machine without a graphics adapter gets what to install.
pub fn window_error(error: &eframe::Error) -> String {
    match error {
        eframe::Error::Wgpu(detail) => no_adapter_help(&detail.to_string()),
        other => format!("the window could not be opened: {other}"),
    }
}

/// What to do when no graphics adapter can present the window.
///
/// The window is drawn by eframe on wgpu, which needs Vulkan, Metal,
/// DirectX 12 or OpenGL from the system: there is no way to show an
/// eframe window without one of them. The model itself is rendered on
/// the CPU, so a software implementation of one of them is enough.
pub fn no_adapter_help(detail: &str) -> String {
    format!(
        "the window could not be opened: no graphics adapter can show it ({detail}).\n\
         3DCraft renders the model on the CPU, but its window is drawn through Vulkan, Metal, \
         DirectX 12 or OpenGL. On a machine without a GPU, install a software implementation \
         and start again:\n  \
         Linux: Mesa's lavapipe (Fedora `mesa-vulkan-drivers`, Debian and Ubuntu \
         `mesa-vulkan-drivers`, Arch `vulkan-swrast`), or llvmpipe for OpenGL \
         (`mesa-dri-drivers`, `libgl1-mesa-dri`)\n  \
         Windows: it comes with the system (WARP); update the graphics driver\n\
         WGPU_BACKEND=vulkan|gl|dx12|metal picks one when several are there.\n\
         Everything but the window works without any of this: `craft3d-cli render`, `run`, \
         `serve` and `mcp` need no graphics adapter."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_engine::Workspace;
    use craft3d_math::Portable;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn desk() -> (Desk, mpsc::Sender<Request>) {
        let (requests, incoming) = mpsc::channel();
        let picker = Picker::new(Arc::new(SystemDialogs), Arc::new(|| {}));
        let host = Host::new(Session::new(Workspace::none()));
        (Desk::new(host, incoming, picker), requests)
    }

    /// `ui.record` and `ui.advance` are the desk's: painted offscreen on
    /// the window's device, with no frame of the compositor and no
    /// window at all (a covered or minimised window is the same), plain
    /// and through MCP's `control_call`; `ui.screenshot {window: true}`
    /// too. Without a device both are refused, and so is an advance
    /// without a frame-exact recording.
    #[test]
    fn a_desk_records_and_takes_pictures_without_a_window() {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/tmp/craft3d-desk-record");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir = craft3d_engine::canonical(&dir).unwrap();
        let (requests, incoming) = mpsc::channel();
        let picker = Picker::new(Arc::new(SystemDialogs), Arc::new(|| {}));
        let host = Host::new(Session::new(Workspace {
            read: Access::Root(dir.clone()),
            write: Access::Root(dir.clone()),
        }));
        let mut desk = Desk::new(host, incoming, picker);
        desk.set_window_points([320.0, 240.0]);
        let ctx = egui::Context::default();
        let serve = |desk: &mut Desk, answer: &Receiver<Option<String>>| -> Value {
            for _ in 0..2000 {
                desk.turn();
                desk.drive(&ctx);
                if let Ok(reply) = answer.try_recv() {
                    return serde_json::from_str(&reply.unwrap()).unwrap();
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            panic!("no reply");
        };
        let mcp = |id: u64, method: &str, params: Value| {
            let line = json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
                "params": {"name": "control_call", "arguments": {"method": method, "params": params}}});
            json!({"id": id, "method": "mcp.message", "params": {"line": line.to_string()}})
                .to_string()
        };
        let start =
            r#"{"id":1,"method":"ui.record","params":{"path":"frames","step":true,"fps":10}}"#;
        // No device yet.
        let reply = serve(&mut desk, &ask(&requests, start));
        assert_eq!(reply["ok"], false, "{reply}");
        assert!(
            reply["error"].as_str().unwrap().contains("wgpu device"),
            "{reply}"
        );
        let reply = serve(
            &mut desk,
            &ask(&requests, &mcp(2, "ui.record", json!({"path": "x"}))),
        );
        let inner: Value =
            serde_json::from_str(reply["result"]["reply"].as_str().unwrap()).unwrap();
        assert_eq!(inner["result"]["isError"], true, "{inner}");
        let state = match craft3d_ui_egui::direct::headless_render_state() {
            Ok(state) => state,
            Err(e) => {
                println!("SKIPPED the rest: no graphics adapter: {e}");
                return;
            }
        };
        desk.attach_recorder(Some(&state));
        for line in [
            r#"{"id":3,"method":"engine.execute","params":{"command":"file.new","params":{}}}"#,
            r#"{"id":4,"method":"engine.execute","params":{"command":"feature.block","params":{"size":["10 mm","10 mm","10 mm"]}}}"#,
        ] {
            assert_eq!(serve(&mut desk, &ask(&requests, line))["ok"], true);
        }
        // Laid out once, as a window is (its textures applied to the
        // renderer, as the window's frames apply theirs).
        desk.offscreen_pass(&ctx, 0.0, Vec::new(), false).unwrap();
        let advance = r#"{"id":5,"method":"ui.advance","params":{"frames":1}}"#;
        let reply = serve(&mut desk, &ask(&requests, advance));
        assert_eq!(reply["ok"], false, "{reply}");
        let reply = serve(&mut desk, &ask(&requests, start));
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(reply["result"]["width"], 320);
        assert!(desk.mirrored());
        let reply = serve(
            &mut desk,
            &ask(
                &requests,
                r#"{"id":6,"method":"ui.advance","params":{"frames":4,"dt":0.05}}"#,
            ),
        );
        assert_eq!(reply["result"]["frames"], 4, "{reply}");
        let reply = serve(
            &mut desk,
            &ask(&requests, &mcp(7, "ui.advance", json!({"frames": 2}))),
        );
        let inner: Value =
            serde_json::from_str(reply["result"]["reply"].as_str().expect("an MCP reply")).unwrap();
        assert_eq!(inner["result"]["isError"], false, "{inner}");
        let reply = serve(
            &mut desk,
            &ask(&requests, r#"{"id":8,"method":"ui.record","params":{}}"#),
        );
        assert_eq!(reply["result"]["frames"], 6, "{reply}");
        let reply = serve(
            &mut desk,
            &ask(
                &requests,
                r#"{"id":9,"method":"ui.record","params":{"stop":true}}"#,
            ),
        );
        assert_eq!(reply["result"]["frames"], 6, "{reply}");
        assert!(!desk.mirrored());
        let files = std::fs::read_dir(dir.join("frames")).unwrap().count();
        assert_eq!(files, 6);
        // A long advance is stopped by a stop: neither waits for the other.
        let again = r#"{"id":11,"method":"ui.record","params":{"path":"again","step":true}}"#;
        assert_eq!(serve(&mut desk, &ask(&requests, again))["ok"], true);
        let long = ask(
            &requests,
            r#"{"id":12,"method":"ui.advance","params":{"frames":100000}}"#,
        );
        let stop = ask(
            &requests,
            r#"{"id":13,"method":"ui.record","params":{"stop":true}}"#,
        );
        let stopped = serve(&mut desk, &stop);
        assert_eq!(stopped["ok"], true, "{stopped}");
        let advanced = serve(&mut desk, &long);
        assert_eq!(advanced["ok"], false, "{advanced}");
        assert!(
            advanced["error"].as_str().unwrap().contains("stopped"),
            "{advanced}"
        );
        assert!(!desk.mirrored());
        // A picture of the window, no frame drawn.
        let reply = serve(
            &mut desk,
            &ask(
                &requests,
                r#"{"id":10,"method":"ui.screenshot","params":{"window":true,"path":"shot.png"}}"#,
            ),
        );
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(reply["result"]["width"], 320);
        assert!(dir.join("shot.png").is_file());
    }

    /// Frame-exact frames in the studio look, whose viewport averages 16
    /// frames of its still: by default a frame is one pass, read back
    /// once; `accumulate: 16` waits for the average (more passes, only the
    /// last read back). Prints where the time goes.
    #[test]
    fn a_frame_exact_frame_is_one_pass_unless_it_waits_for_the_average() {
        let Ok(state) = craft3d_ui_egui::direct::headless_render_state() else {
            println!("SKIPPED: no graphics adapter");
            return;
        };
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/tmp/craft3d-desk-record-speed");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir = craft3d_engine::canonical(&dir).unwrap();
        let (requests, incoming) = mpsc::channel();
        let picker = Picker::new(Arc::new(SystemDialogs), Arc::new(|| {}));
        let mut host = Host::new(Session::new(Workspace {
            read: Access::Root(dir.clone()),
            write: Access::Root(dir.clone()),
        }));
        host.shell.attach_gpu(&state).unwrap();
        let mut desk = Desk::new(host, incoming, picker);
        desk.set_window_points([1600.0, 1000.0]);
        desk.attach_recorder(Some(&state));
        let ctx = egui::Context::default();
        let serve = |desk: &mut Desk, line: &str| -> Value {
            let answer = ask(&requests, line);
            for _ in 0..20000 {
                desk.turn();
                desk.drive(&ctx);
                if let Ok(reply) = answer.try_recv() {
                    return serde_json::from_str(&reply.unwrap()).unwrap();
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            panic!("no reply to {line}");
        };
        for line in [
            r#"{"id":1,"method":"engine.execute","params":{"command":"file.new","params":{}}}"#,
            r#"{"id":2,"method":"engine.execute","params":{"command":"feature.block","params":{"size":["40 mm","30 mm","20 mm"]}}}"#,
            r#"{"id":3,"method":"engine.execute","params":{"command":"feature.sphere","params":{"radius":"12 mm","position":["50 mm","0 mm","0 mm"]}}}"#,
            r#"{"id":4,"method":"ui.set","params":{"look":{"mode":"studio"}}}"#,
        ] {
            assert_eq!(serve(&mut desk, line)["ok"], true, "{line}");
        }
        desk.offscreen_pass(&ctx, 0.0, Vec::new(), false).unwrap();
        for (accumulate, path, view) in [(1, "one", "top"), (16, "sixteen", "front")] {
            // Another view: the viewport starts its average again.
            let view = format!(r#"{{"id":4,"method":"ui.set","params":{{"view":"{view}"}}}}"#);
            assert_eq!(serve(&mut desk, &view)["ok"], true);
            desk.offscreen_pass(&ctx, 0.0, Vec::new(), false).unwrap();
            let averages = desk.host.shell.converging();
            let start = format!(
                r#"{{"id":5,"method":"ui.record","params":{{"path":"{path}","step":true,"accumulate":{accumulate}}}}}"#
            );
            assert_eq!(serve(&mut desk, &start)["ok"], true);
            let began = Instant::now();
            let reply = serve(
                &mut desk,
                r#"{"id":6,"method":"ui.advance","params":{"frames":6}}"#,
            );
            let took = began.elapsed().as_secs_f64() / 6.0;
            let result = &reply["result"];
            println!(
                "accumulate {accumulate}: {:.0} ms a frame at 1600 x 1000, {}",
                took * 1e3,
                result["timing"]
            );
            assert_eq!(result["frames"], 6, "{reply}");
            assert_eq!(result["complete"], true);
            let passes = result["timing"]["passesPerFrame"].as_f64().unwrap();
            if accumulate == 1 {
                assert_eq!(passes, 1.0, "{reply}");
            } else if averages {
                assert!(passes > 1.0, "{reply}");
            } else {
                println!("the viewport does not average its still here (no direct GPU path)");
            }
            let stop = r#"{"id":7,"method":"ui.record","params":{"stop":true}}"#;
            assert_eq!(serve(&mut desk, stop)["ok"], true);
        }
        // Out of time: the frames drawn so far, the rest left to ask for.
        let start = r#"{"id":8,"method":"ui.record","params":{"path":"late","step":true}}"#;
        assert_eq!(serve(&mut desk, start)["ok"], true);
        let reply = serve(
            &mut desk,
            r#"{"id":9,"method":"ui.advance","params":{"frames":100000,"timeout":0.5}}"#,
        );
        assert_eq!(reply["result"]["complete"], false, "{reply}");
        assert!(reply["result"]["left"].as_u64().unwrap() > 0);
        assert_eq!(
            serve(
                &mut desk,
                r#"{"id":10,"method":"ui.record","params":{"stop":true}}"#
            )["ok"],
            true
        );
    }

    /// The status bar names the file a request opens, without its folder
    /// and extension; other requests are about nothing in particular.
    #[test]
    fn a_file_opened_is_named_by_its_stem() {
        let open = |path: &str| {
            json!({"id": 1, "method": "engine.execute",
                   "params": {"command": "file.open", "params": {"path": path}}})
            .to_string()
        };
        assert_eq!(
            request_subject(&open("/jigs/welding-table.3dcraftjig")).as_deref(),
            Some("welding-table")
        );
        assert_eq!(
            request_subject(&open("C:\\models\\bracket.3dcraft")).as_deref(),
            Some("bracket")
        );
        assert_eq!(request_subject(&open("")), None);
        assert_eq!(request_subject("not json"), None);
        let other = json!({"id": 1, "method": "engine.execute",
                           "params": {"command": "file.new", "params": {}}});
        assert_eq!(request_subject(&other.to_string()), None);
        assert_eq!(file_stem("plate.step").as_deref(), Some("plate.step"));
    }

    fn ask(requests: &mpsc::Sender<Request>, line: &str) -> Receiver<Option<String>> {
        let (reply, answer) = mpsc::channel();
        let line = line.to_string();
        requests.send(Request { line, reply }).unwrap();
        answer
    }

    /// The bug: a window its compositor does not show gets no frames, and
    /// requests were answered, and `ui.quit` was looked at, only in a
    /// frame. A turn needs neither a frame nor an egui context.
    #[test]
    fn a_turn_answers_requests_and_sees_quit_without_a_frame() {
        let (mut desk, requests) = desk();
        let idle = desk.turn();
        assert_eq!(
            idle,
            Turn {
                changed: false,
                quit: false
            }
        );
        let made = ask(
            &requests,
            r#"{"id":1,"method":"engine.execute","params":{"command":"file.new","params":{"units":"mm"}}}"#,
        );
        let turn = desk.turn();
        assert!(turn.changed && !turn.quit, "{turn:?}");
        let reply = made.try_recv().unwrap().unwrap();
        assert!(reply.contains(r#""ok":true"#), "{reply}");
        // The command ran on the desk's own session.
        assert!(desk.host.session.document_status().is_some());

        let quit = ask(&requests, r#"{"id":2,"method":"ui.quit"}"#);
        let turn = desk.turn();
        assert!(turn.changed && turn.quit, "{turn:?}");
        let reply = quit.try_recv().unwrap().unwrap();
        assert!(reply.contains(r#""quitting":true"#), "{reply}");
        // It stays asked for: whichever turn the event loop looks at.
        assert!(desk.turn().quit);
    }

    /// Turns until `answer` has its reply, or a few seconds have passed.
    fn turn_until(desk: &mut Desk, answer: &Receiver<Option<String>>) -> Option<String> {
        for _ in 0..500 {
            desk.turn();
            if let Ok(reply) = answer.try_recv() {
                return reply;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        None
    }

    /// `ui.screenshot {window: true}` is answered with the frame egui hands
    /// over after it asked for one; meanwhile the requests after it wait,
    /// but `ui.get` does not, and parameters it does not take are refused
    /// at once.
    #[test]
    fn a_window_picture_waits_for_its_frame_and_holds_what_comes_after() {
        let (requests, incoming) = mpsc::channel();
        let picker = Picker::new(Arc::new(SystemDialogs), Arc::new(|| {}));
        let host = Host::new(Session::new(Workspace::none()));
        let mut desk = Desk::new(host, incoming, picker).with_engine_thread(Arc::new(|| {}));
        let wrong = ask(
            &requests,
            r#"{"id":1,"method":"ui.screenshot","params":{"window":true,"width":30}}"#,
        );
        let reply = turn_until(&mut desk, &wrong).unwrap();
        assert!(
            reply.contains(r#""ok":false"#) && reply.contains("width"),
            "{reply}"
        );
        let shot = ask(
            &requests,
            r#"{"id":2,"method":"ui.screenshot","params":{"window":true}}"#,
        );
        let after = ask(
            &requests,
            r#"{"id":3,"method":"ui.notice","params":{"text":"next"}}"#,
        );
        let get = ask(&requests, r#"{"id":4,"method":"ui.get"}"#);
        assert!(turn_until(&mut desk, &get).is_some());
        assert!(shot.try_recv().is_err() && after.try_recv().is_err());
        // Asked for after a few frames, once.
        let ctx = egui::Context::default();
        for _ in 0..CAPTURE_FRAMES {
            desk.ask_capture(&ctx);
        }
        assert!(desk.capture.as_ref().is_some_and(|c| c.asked.is_some()));
        let image = egui::ColorImage::new([5, 2], vec![egui::Color32::from_rgb(1, 2, 3); 10]);
        desk.captured(Arc::new(image));
        let reply = turn_until(&mut desk, &shot).unwrap();
        assert!(
            reply.contains(r#""ok":true"#) && reply.contains(r#""width":5"#),
            "{reply}"
        );
        let reply = turn_until(&mut desk, &after).unwrap();
        assert!(reply.contains(r#""ok":true"#), "{reply}");
    }

    /// A picture that arrives while the session is on the engine thread
    /// waits for it: the path is written through the real session, never
    /// the stand-in the desk holds meanwhile.
    #[test]
    fn a_window_picture_is_answered_only_with_the_session_here() {
        let (requests, incoming) = mpsc::channel();
        let picker = Picker::new(Arc::new(SystemDialogs), Arc::new(|| {}));
        let host = Host::new(Session::new(Workspace::none()));
        let mut desk = Desk::new(host, incoming, picker).with_engine_thread(Arc::new(|| {}));
        let shot = ask(
            &requests,
            r#"{"id":1,"method":"ui.screenshot","params":{"window":true}}"#,
        );
        let get = ask(&requests, r#"{"id":2,"method":"ui.get"}"#);
        assert!(turn_until(&mut desk, &get).is_some());
        assert!(desk.capture.is_some());
        // Work on the engine thread, and the picture comes meanwhile.
        desk.start(
            Waiting::Scene,
            "slow".to_string(),
            Box::new(|_| {
                std::thread::sleep(Duration::from_millis(200));
                Outcome::Built
            }),
        );
        assert!(desk.away());
        let image = egui::ColorImage::new([2, 2], vec![egui::Color32::from_rgb(1, 2, 3); 4]);
        desk.captured(Arc::new(image));
        desk.turn();
        if desk.away() {
            assert!(shot.try_recv().is_err(), "answered while away");
        }
        let reply = turn_until(&mut desk, &shot).unwrap();
        assert!(reply.contains(r#""ok":true"#), "{reply}");
        assert!(!desk.away());
    }

    /// A picture of the window waits while the viewport averages the
    /// frames of its still picture (the studio look's accumulation): it is
    /// asked for only after the frames that follow, so it is the settled
    /// still; and one that never settles is answered at its deadline.
    #[test]
    fn a_window_picture_waits_until_the_viewport_has_settled() {
        let (requests, incoming) = mpsc::channel();
        let picker = Picker::new(Arc::new(SystemDialogs), Arc::new(|| {}));
        let host = Host::new(Session::new(Workspace::none()));
        let mut desk =
            Desk::new(host, incoming, picker).with_capture_deadline(Duration::from_millis(300));
        let shot = ask(
            &requests,
            r#"{"id":1,"method":"ui.screenshot","params":{"window":true}}"#,
        );
        desk.turn();
        assert!(desk.capture.is_some());
        let ctx = egui::Context::default();
        // Sixteen frames of accumulation and more: not asked for.
        for _ in 0..40 {
            desk.ask_capture_when(&ctx, true);
        }
        assert!(desk.capture.as_ref().is_some_and(|c| c.asked.is_none()));
        // Settled: asked for after its frames.
        for _ in 1..CAPTURE_FRAMES {
            desk.ask_capture_when(&ctx, false);
        }
        assert!(desk.capture.as_ref().is_some_and(|c| c.asked.is_none()));
        desk.ask_capture_when(&ctx, false);
        assert!(desk.capture.as_ref().is_some_and(|c| c.asked.is_some()));
        let image = egui::ColorImage::new([5, 2], vec![egui::Color32::from_rgb(1, 2, 3); 10]);
        desk.captured(Arc::new(image));
        let reply = turn_until(&mut desk, &shot).unwrap();
        assert!(reply.contains(r#""ok":true"#), "{reply}");
        // One that never settles ends at its deadline.
        let shot = ask(
            &requests,
            r#"{"id":2,"method":"ui.screenshot","params":{"window":true}}"#,
        );
        desk.turn();
        for _ in 0..40 {
            desk.ask_capture_when(&ctx, true);
        }
        assert!(desk.capture.as_ref().is_some_and(|c| c.asked.is_none()));
        let reply = turn_until(&mut desk, &shot).unwrap();
        assert!(
            reply.contains(r#""ok":false"#) && reply.contains("still averaging"),
            "{reply}"
        );
    }

    /// A picture of the window whose frame never comes (egui dropped it:
    /// a surface that timed out or was occluded) is asked for again after
    /// some frames, and answered with an error once its deadline passed;
    /// then what waited behind it is answered. Likewise without an engine
    /// thread, where it is recognised as one too.
    #[test]
    fn a_window_picture_without_a_frame_ends_in_an_error_and_the_queue_drains() {
        for engine in [true, false] {
            let (requests, incoming) = mpsc::channel();
            let picker = Picker::new(Arc::new(SystemDialogs), Arc::new(|| {}));
            let host = Host::new(Session::new(Workspace::none()));
            let mut desk =
                Desk::new(host, incoming, picker).with_capture_deadline(Duration::from_millis(300));
            if engine {
                desk = desk.with_engine_thread(Arc::new(|| {}));
            }
            let shot = ask(
                &requests,
                r#"{"id":1,"method":"ui.screenshot","params":{"window":true}}"#,
            );
            let after = ask(
                &requests,
                r#"{"id":2,"method":"ui.notice","params":{"text":"x"}}"#,
            );
            desk.turn();
            assert!(desk.capture.is_some(), "engine {engine}");
            // Frames come, the picture is asked for, and again later.
            let ctx = egui::Context::default();
            for _ in 0..CAPTURE_FRAMES {
                desk.ask_capture(&ctx);
            }
            let first = desk.capture.as_ref().and_then(|c| c.asked);
            for _ in 0..CAPTURE_RETRY_FRAMES {
                desk.ask_capture(&ctx);
            }
            let again = desk.capture.as_ref().and_then(|c| c.asked);
            assert!(first.is_some() && again > first, "{first:?} {again:?}");
            assert!(after.try_recv().is_err());
            let reply = turn_until(&mut desk, &shot).unwrap();
            assert!(
                reply.contains(r#""ok":false"#) && reply.contains("drew no frame"),
                "{reply}"
            );
            let reply = turn_until(&mut desk, &after).unwrap();
            assert!(reply.contains(r#""ok":true"#), "{reply}");
            assert!(desk.capture.is_none());
        }
    }

    /// A picture of a window that draws no frame at all (hidden, on
    /// another workspace: no frame and no event comes) is still answered
    /// at its deadline: a timer wakes the desk, and the turn that wake
    /// brings answers it with the documented error (R1 B8: agents saw it
    /// hang past its 8 s).
    #[test]
    fn a_window_picture_is_answered_at_its_deadline_without_any_frame() {
        for engine in [true, false] {
            let (requests, incoming) = mpsc::channel();
            let picker = Picker::new(Arc::new(SystemDialogs), Arc::new(|| {}));
            let host = Host::new(Session::new(Workspace::none()));
            let (rang, rings) = mpsc::channel();
            let rang = Mutex::new(rang);
            let wake: Wake = Arc::new(move || {
                let _ = rang.lock().unwrap().send(());
            });
            let mut desk =
                Desk::new(host, incoming, picker).with_capture_deadline(Duration::from_millis(200));
            desk = if engine {
                desk.with_engine_thread(wake)
            } else {
                desk.with_wake(wake)
            };
            // Twice: the second deadline is rung by the same timer.
            for id in 1..=2 {
                let shot = ask(
                    &requests,
                    &format!(
                        r#"{{"id":{id},"method":"ui.screenshot","params":{{"window":true}}}}"#
                    ),
                );
                let started = Instant::now();
                desk.turn();
                assert!(desk.capture.is_some(), "engine {engine}");
                // No frame, no request: only the deadline's timer wakes it.
                loop {
                    rings
                        .recv_timeout(Duration::from_secs(5))
                        .expect("the deadline wakes the desk");
                    if started.elapsed() >= Duration::from_millis(200) {
                        break;
                    }
                }
                desk.turn();
                let reply = shot.try_recv().unwrap().unwrap();
                assert!(
                    reply.contains(r#""ok":false"#) && reply.contains("drew no frame"),
                    "{reply}"
                );
                assert!(started.elapsed() < Duration::from_secs(3));
                assert!(desk.capture.is_none());
            }
            assert!(desk.deadlines.is_some(), "one timer thread");
        }
    }

    /// Every wake posts an event of its own to the event loop, also when
    /// nothing was drawn since the last one (egui's `request_repaint`
    /// posts none while an earlier request is still waiting for a frame).
    #[test]
    fn every_wake_posts_an_event_to_the_event_loop() {
        let posted = Arc::new(Mutex::new(Vec::new()));
        let for_send = Arc::clone(&posted);
        let pass = Arc::new(AtomicU64::new(0));
        let for_pass = Arc::clone(&pass);
        let wake = waker(
            move |event| for_send.lock().unwrap().push(event),
            move || for_pass.load(Ordering::SeqCst),
        );
        wake();
        wake();
        pass.store(7, Ordering::SeqCst);
        wake();
        let posted = posted.lock().unwrap();
        let passes: Vec<u64> = posted
            .iter()
            .map(|event| match event {
                UserEvent::RequestRepaint {
                    viewport_id,
                    cumulative_pass_nr,
                    ..
                } => {
                    assert_eq!(*viewport_id, egui::ViewportId::ROOT);
                    *cumulative_pass_nr
                }
            })
            .collect();
        // The pass egui is at, so eframe does not drop it as outdated.
        assert_eq!(passes, [0, 0, 7]);
    }

    #[test]
    fn an_error_eframe_only_logged_gets_the_same_help() {
        let detail = eframe::egui_wgpu::WgpuError::NoSurfaceFormatsAvailable.to_string();
        // As eframe words it, from the error's own text: this pins
        // `WGPU_PREFIX` to the eframe in use.
        let error = eframe::Error::Wgpu(eframe::egui_wgpu::WgpuError::NoSurfaceFormatsAvailable);
        let logged = format!("{LOGGED_PREFIX}{error}");
        assert_eq!(logged_window_error(&logged), window_error(&error));
        assert_eq!(logged_window_error(&logged), no_adapter_help(&detail));
        // Anything else is passed on as it is.
        let other = logged_window_error("Exiting because of error: the display went away");
        assert_eq!(
            other,
            "the window could not be opened: the display went away"
        );
        assert_eq!(
            logged_window_error("no prefix"),
            "the window could not be opened: no prefix"
        );
    }

    #[test]
    fn only_errors_of_eframe_are_kept_and_taking_one_empties_it() {
        use log::Log as _;
        let failures = Failures(Mutex::new(None));
        let record = |level, target: &'static str| {
            failures.log(
                &log::Record::builder()
                    .level(level)
                    .target(target)
                    .args(format_args!("what happened"))
                    .build(),
            );
        };
        record(log::Level::Warn, "eframe::native::run");
        record(log::Level::Error, "wgpu_hal::vulkan");
        assert_eq!(failures.take(), None);
        record(log::Level::Error, "eframe::native::run");
        assert_eq!(failures.take().as_deref(), Some("what happened"));
        assert_eq!(failures.take(), None);
    }

    #[test]
    fn a_missing_graphics_adapter_says_what_to_install_and_what_still_works() {
        // What eframe reports when wgpu finds nothing to draw with.
        let error = eframe::Error::Wgpu(eframe::egui_wgpu::WgpuError::NoSurfaceFormatsAvailable);
        let detail = eframe::egui_wgpu::WgpuError::NoSurfaceFormatsAvailable.to_string();
        let message = window_error(&error);
        assert_eq!(message, no_adapter_help(&detail));
        assert!(message.starts_with("the window could not be opened: no graphics adapter"));
        assert!(message.contains(&detail), "the cause is kept: {message}");
        for needed in [
            "lavapipe",
            "mesa-vulkan-drivers",
            "llvmpipe",
            "WGPU_BACKEND",
            "craft3d-cli render",
            "mcp",
        ] {
            assert!(message.contains(needed), "`{needed}` is missing: {message}");
        }
        // Lines a terminal shows whole: none longer than a wide terminal
        // would wrap twice.
        assert!(message.lines().count() >= 4);
        // Any other failure is passed on as it is, without that advice.
        let other = eframe::Error::AppCreation("the shell fell over".into());
        let message = window_error(&other);
        assert!(message.contains("the shell fell over"), "{message}");
        assert!(!message.contains("lavapipe"));
    }

    /// A frame of the window, then a turn of the desk, as the event loop
    /// has them.
    fn draw(desk: &mut Desk, ctx: &egui::Context, time: &mut f64, events: Vec<egui::Event>) {
        *time += 1.0 / 60.0;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 760.0),
            )),
            time: Some(*time),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            if desk.away() {
                desk.host.shell.ui_away(ui);
            } else {
                let Host { session, shell, .. } = &mut desk.host;
                shell.ui(ui, session);
            }
        });
        output.textures_delta.clear();
        desk.turn();
    }

    /// `ui.param` is settled on the window's thread and regenerates on the
    /// engine thread; the reply comes once the window has synced, with the
    /// form's state. What it refuses is answered at once.
    #[test]
    fn ui_param_regenerates_on_the_engine_thread() {
        let (requests, incoming) = mpsc::channel::<Request>();
        let picker = Picker::new(Arc::new(SystemDialogs), Arc::new(|| {}));
        let mut session = Session::new(Workspace::none());
        for (id, params) in [
            ("file.new", json!({"units": "mm"})),
            ("variable.set", json!({"name": "w", "expr": "40 mm"})),
            ("feature.block", json!({"size": ["w", "20 mm", "5 mm"]})),
            (
                "variable.publish",
                json!({"name": "w", "min": "10 mm", "max": "90 mm"}),
            ),
        ] {
            session.execute(id, &params).unwrap();
        }
        let mut desk =
            Desk::new(Host::new(session), incoming, picker).with_engine_thread(Arc::new(|| {}));
        let answer = ask(
            &requests,
            r#"{"id":7,"method":"ui.param","params":{"name":"w","value":500}}"#,
        );
        let reply: Value = serde_json::from_str(&turn_until(&mut desk, &answer).unwrap()).unwrap();
        assert_eq!(reply["id"], 7, "{reply}");
        let result = &reply["result"];
        assert_eq!(result["sent"], 90.0, "{reply}");
        assert_eq!(result["note"], "clamped to 90 mm");
        assert_eq!(result["result"]["changed"], true, "{reply}");
        assert_eq!(result["parameters"]["busy"], false, "{reply}");
        assert!(!desk.away());
        let list = desk.host.session.execute("param.list", &json!({})).unwrap();
        assert_eq!(list["parameters"][0]["value"], 90.0);
        let answer = ask(
            &requests,
            r#"{"id":8,"method":"ui.param","params":{"name":"nope","value":1}}"#,
        );
        let reply: Value = serde_json::from_str(&turn_until(&mut desk, &answer).unwrap()).unwrap();
        assert!(
            reply["error"]
                .to_string()
                .contains("not a public parameter"),
            "{reply}"
        );
    }

    /// Open for editing makes the model of a Jig on the engine thread,
    /// as opening a file does.
    #[test]
    fn open_for_editing_runs_on_the_engine_thread() {
        let (_requests, incoming) = mpsc::channel::<Request>();
        let picker = Picker::new(Arc::new(SystemDialogs), Arc::new(|| {}));
        let mut session = Session::new(Workspace::memory());
        for (id, params) in [
            ("file.new", json!({"units": "mm"})),
            ("variable.set", json!({"name": "w", "expr": "40 mm"})),
            ("feature.block", json!({"size": ["w", "20 mm", "5 mm"]})),
            ("variable.publish", json!({"name": "w"})),
            ("jig.export", json!({"path": "p.3dcraftjig"})),
            ("file.open", json!({"path": "p.3dcraftjig"})),
        ] {
            session.execute(id, &params).unwrap();
        }
        let mut desk =
            Desk::new(Host::new(session), incoming, picker).with_engine_thread(Arc::new(|| {}));
        desk.host
            .shell
            .request_file_action(FileAction::OpenForEditing);
        for _ in 0..500 {
            desk.turn();
            if !desk.away() && desk.host.session.document_statuses().len() == 3 {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let status = desk.host.session.document_status().unwrap();
        assert!(!status.jig_in_use && status.path.is_none(), "{status:?}");
        let report = desk.host.shell.last_command.clone().unwrap();
        assert_eq!(report.label, "jig.edit");
        assert!(report.ok, "{}", report.text);
    }

    /// Undo and Redo of the Edit menu run on the engine thread and are not
    /// a commit of the Parameters form: an undo back to a state in which a
    /// feature fails is not judged as a value that broke the model (no
    /// trouble, no Back to last good).
    #[test]
    fn undo_on_the_engine_thread_is_no_verdict_of_the_parameter_form() {
        let (requests, incoming) = mpsc::channel::<Request>();
        drop(requests);
        let picker = Picker::new(Arc::new(SystemDialogs), Arc::new(|| {}));
        let mut session = Session::new(Workspace::none());
        for (id, params) in [
            ("file.new", json!({"units": "mm"})),
            ("variable.set", json!({"name": "w", "expr": "40 mm"})),
            ("feature.block", json!({"size": ["w", "20 mm", "5 mm"]})),
            ("variable.publish", json!({"name": "w"})),
            // A width of nothing: the block fails.
            ("variable.set", json!({"name": "w", "expr": "0 mm"})),
            ("variable.set", json!({"name": "w", "expr": "40 mm"})),
        ] {
            session.execute(id, &params).unwrap();
        }
        let id = session.document_status().unwrap().id;
        let mut desk =
            Desk::new(Host::new(session), incoming, picker).with_engine_thread(Arc::new(|| {}));
        let ctx = egui::Context::default();
        let mut time = 0.0;
        for _ in 0..6 {
            draw(&mut desk, &ctx, &mut time, Vec::new());
        }
        desk.host
            .shell
            .request_edit_action(craft3d_ui_egui::EditAction::Undo, id);
        let mut ran = false;
        for _ in 0..500 {
            draw(&mut desk, &ctx, &mut time, Vec::new());
            ran |= desk.away();
            if ran && !desk.away() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        for _ in 0..4 {
            draw(&mut desk, &ctx, &mut time, Vec::new());
        }
        let report = desk.host.shell.last_command.clone().unwrap();
        assert_eq!(report.label, "edit.undo");
        assert!(report.ok, "{}", report.text);
        assert!(
            !desk.host.session.failed_features().is_empty(),
            "back to the failing state"
        );
        let Host { session, shell, .. } = &mut desk.host;
        let state = shell
            .handle_method(session, "ui.get", None)
            .unwrap()
            .unwrap();
        assert_eq!(
            state["parameters"]["trouble"],
            Value::Null,
            "{}",
            state["parameters"]
        );
    }

    /// The Parameters tab's commit runs on the engine thread, as every
    /// command, and comes back to the form: Reset all, clicked in the
    /// window, sets a Jig's parameter back and the status bar says so.
    #[test]
    fn the_parameter_form_commits_on_the_engine_thread() {
        let (requests, incoming) = mpsc::channel::<Request>();
        drop(requests);
        let picker = Picker::new(Arc::new(SystemDialogs), Arc::new(|| {}));
        let mut session = Session::new(Workspace::none());
        for (id, params) in [
            ("file.new", json!({"units": "mm"})),
            ("variable.set", json!({"name": "w", "expr": "40 mm"})),
            ("feature.block", json!({"size": ["w", "20 mm", "5 mm"]})),
            (
                "variable.publish",
                json!({"name": "w", "min": "10 mm", "max": "90 mm"}),
            ),
            ("param.set", json!({"values": {"w": 70}})),
        ] {
            session.execute(id, &params).unwrap();
        }
        let host = Host::new(session);
        let mut desk = Desk::new(host, incoming, picker).with_engine_thread(Arc::new(|| {}));
        let ctx = egui::Context::default();
        let mut time = 0.0;
        for _ in 0..6 {
            draw(&mut desk, &ctx, &mut time, Vec::new());
        }
        let reset = desk
            .host
            .shell
            .parameter_hits()
            .iter()
            .find(|(name, _)| name == "Reset all")
            .map(|(_, rect)| rect.center())
            .unwrap();
        for pressed in [true, false] {
            draw(
                &mut desk,
                &ctx,
                &mut time,
                vec![egui::Event::PointerButton {
                    pos: reset,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        for _ in 0..500 {
            draw(&mut desk, &ctx, &mut time, Vec::new());
            if !desk.away() && !desk.host.shell.parameters_busy() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let list = desk.host.session.execute("param.list", &json!({})).unwrap();
        assert_eq!(list["parameters"][0]["value"], 40.0, "{list}");
        let report = desk.host.shell.last_command.clone().unwrap();
        assert_eq!(report.label, "param.set");
        assert!(report.ok, "{}", report.text);
    }

    /// The window's bug with an assembly (2026-10-30): with the engine
    /// thread and the refiner, as the window runs, a four-bar made by
    /// commands over the control channel is drawn where `joint.set` puts
    /// its crank, at three angles, and `ui.fit` frames it.
    #[test]
    fn the_window_follows_joint_set_on_an_assembly() {
        let dir = std::env::temp_dir().join(format!("craft3d-asm-window-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("demo/asm")).unwrap();
        let dir = craft3d_engine::canonical(&dir).unwrap();
        let (requests, incoming) = mpsc::channel::<Request>();
        let picker = Picker::new(Arc::new(SystemDialogs), Arc::new(|| {}));
        // The window's own session: the user's authority narrowed to the
        // automation roots (`--automation-read-root`).
        let options = crate::args::Options {
            read_root: Some(dir.to_string_lossy().to_string()),
            write_root: Some(dir.to_string_lossy().to_string()),
            ..crate::args::Options::default()
        };
        let session = super::session(&options).unwrap();
        let mut desk =
            Desk::new(Host::new(session), incoming, picker).with_engine_thread(Arc::new(|| {}));
        let mut id = 0;
        let mut execute = |desk: &mut Desk, command: &str, params: Value| -> Value {
            id += 1;
            let line = json!({"id": id, "method": "engine.execute",
                              "params": {"command": command, "params": params}});
            let answer = ask(&requests, &line.to_string());
            let reply: Value = serde_json::from_str(&turn_until(desk, &answer).unwrap()).unwrap();
            assert!(reply.get("error").is_none(), "{command}: {reply}");
            reply
        };
        execute(&mut desk, "file.new", json!({}));
        execute(
            &mut desk,
            "variable.set",
            json!({"name": "len", "expr": "24 mm"}),
        );
        execute(
            &mut desk,
            "variable.publish",
            json!({"name": "len", "min": "10 mm", "max": "500 mm"}),
        );
        execute(
            &mut desk,
            "feature.block",
            json!({"name": "Bar", "size": ["len", 4, 2]}),
        );
        execute(
            &mut desk,
            "datum.axis",
            json!({"name": "End1", "through": [[2, 2, 0], [2, 2, 1]]}),
        );
        execute(
            &mut desk,
            "datum.axis",
            json!({"name": "End2", "through": [["len - 2 mm", 2, 0], ["len - 2 mm", 2, 1]]}),
        );
        execute(
            &mut desk,
            "file.save",
            json!({"path": "demo/asm/bar.3dcraft"}),
        );
        execute(&mut desk, "assembly.new", json!({}));
        // One source named by its absolute path, as an agent may.
        let absolute = dir
            .join("demo/asm/bar.3dcraft")
            .to_string_lossy()
            .to_string();
        for (name, len, source) in [
            ("Ground", "64 mm", "demo/asm/bar.3dcraft"),
            ("Crank", "24 mm", absolute.as_str()),
            ("Coupler", "64 mm", "demo/asm/bar.3dcraft"),
            ("Rocker", "24 mm", "demo/asm/bar.3dcraft"),
        ] {
            execute(
                &mut desk,
                "instance.add",
                json!({"source": source, "name": name, "body": "Bar",
                       "params": {"len": len}}),
            );
        }
        for (name, a, b) in [
            ("Base", "world/z", "Ground/End1"),
            ("Drive", "Ground/End1", "Crank/End1"),
            ("Elbow", "Crank/End2", "Coupler/End1"),
            ("Knee", "Coupler/End2", "Rocker/End2"),
            ("Hip", "Ground/End2", "Rocker/End1"),
        ] {
            let kind = if name == "Base" { "fixed" } else { "revolute" };
            execute(
                &mut desk,
                "mate.add",
                json!({"kind": kind, "a": a, "b": b, "name": name, "flip": true}),
            );
        }
        let mut seen = Vec::new();
        for angle in [60.0_f64, 150.0, 240.0] {
            execute(
                &mut desk,
                "joint.set",
                json!({"mate": "Drive", "value": format!("{angle} deg")}),
            );
            // Until the work is done and the display scene is refined.
            for _ in 0..500 {
                desk.turn();
                if !desk.away() && !desk.refining() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            desk.host.shell.sync(&mut desk.host.session);
            let scene = desk.host.shell.shown_scene_arc().expect("a scene shown");
            let crank = scene
                .bodies
                .iter()
                .find(|b| b.name == "Crank")
                .unwrap_or_else(|| {
                    panic!(
                        "{angle}: no Crank among {:?} (preview {})",
                        scene.bodies.iter().map(|b| &b.name).collect::<Vec<_>>(),
                        scene.preview
                    )
                });
            let (lo, hi) = crank.mesh.bbox().corners().expect("a box");
            let mid = [(lo.x + hi.x) / 2.0, (lo.y + hi.y) / 2.0];
            let (c, n) = (angle.to_radians().pcos(), angle.to_radians().psin());
            assert!(
                (mid[0] - 10.0 * c).abs() < 3.0 && (mid[1] - 10.0 * n).abs() < 3.0,
                "{angle} deg: the crank is drawn at {mid:?}"
            );
            seen.push(mid);
        }
        let answer = ask(&requests, r#"{"id":900,"method":"ui.fit"}"#);
        let reply = turn_until(&mut desk, &answer).unwrap();
        assert!(!reply.contains("error"), "{reply}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
