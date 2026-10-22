//! The window: eframe around the shell, on an event loop of our own, with
//! the control channel's requests answered on the UI thread whenever that
//! loop is woken for them, whether or not a frame is drawn.

use crate::args::Options;
use crate::control::{ControlConfig, ControlServer, Wake};
use crate::files::{self, Choice, Picker, SystemDialogs};
use crate::host::{Host, Request, Route, answer_on_engine, report, route};
use crate::refine::{Poll, Refiner};
use crate::tabs;
use crate::token;
use crate::worker::{Finished, Outcome, Run, Task, Worker};
use craft3d_automation::rpc;
use craft3d_engine::{Access, RecoveryOptions, Session, Workspace};
use craft3d_ui_egui::{DocAction, Shell, window_title};
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

/// Turns a root flag's value into an access rule. The directory must exist,
/// and is made absolute with its symbolic links resolved: the engine
/// compares paths as text, so the root has to be the one true spelling.
fn root(flag: &str, dir: Option<&str>) -> Result<Access, String> {
    let Some(dir) = dir else {
        return Ok(Access::None);
    };
    let path = std::fs::canonicalize(dir)
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
    /// The frame count when the picture was last asked for: egui drops a
    /// request on a surface that timed out, is outdated, occluded, lost or
    /// recreated, so it is asked again after [`CAPTURE_RETRY_FRAMES`].
    asked: Option<u32>,
    /// When it came in.
    since: Instant,
    /// The picture, once egui handed it over, waiting for the session.
    image: Option<Arc<egui::ColorImage>>,
}

/// Frames a picture of the window waits before it is asked for.
const CAPTURE_FRAMES: u32 = 3;
/// Frames drawn after asking without a picture before it is asked again.
const CAPTURE_RETRY_FRAMES: u32 = 30;
/// How long a picture of the window waits for a frame before it is
/// answered with an error.
const CAPTURE_DEADLINE: Duration = Duration::from_secs(8);

/// The error of a picture of the window that got no frame in time.
const NO_FRAME: &str = "ui.screenshot: the window drew no frame in time; is it minimised, hidden or on another workspace?";

/// Who waits for the task on the engine thread.
enum Waiting {
    /// A request: its reply goes back when the task is done.
    Request(Request),
    /// A file chosen in a dialog: the outcome goes to the status bar.
    File(&'static str),
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
        }
    }

    /// Runs commands on an engine thread of their own from now on, which
    /// calls `wake` when there is something new to show. When the thread
    /// cannot be started they run where the desk is served, as before.
    #[must_use]
    pub fn with_engine_thread(mut self, wake: Wake) -> Self {
        self.worker = Worker::start(Arc::clone(&wake)).ok();
        if self.worker.is_some() {
            // Coarse first, then sharp: the engine thread builds the quick
            // first picture, the refiner the display scene.
            self.refiner = Some(Refiner::new(wake));
            self.host.session.set_refine_elsewhere(true);
        }
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
                if self.capture.is_some() {
                    self.answer_anytime_now();
                    break;
                }
                let Some(request) = self.queue.pop_front() else {
                    break;
                };
                served += 1;
                answered = true;
                if let Some(request) = self.intercept_capture(request) {
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
                self.start(
                    Waiting::Scene,
                    "meshing".to_string(),
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
            // after it waits with it.
            if self.capture.is_some() {
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
                let Some(request) = self.intercept_capture(request) else {
                    continue;
                };
                match route(&request.line) {
                    Route::Engine => {
                        let label = request_label(&request.line);
                        let line = request.line.clone();
                        self.start(
                            Waiting::Request(request),
                            label,
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
        let task = Task {
            label,
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
    }

    /// What the menu asked for and what a dialog answered: a file chosen
    /// to open or to save as is opened or saved on the engine thread.
    fn pump_files(&mut self) -> bool {
        if let Some(choice) = self.picker.poll() {
            match choice {
                Ok(Choice::Open(path)) => self.start(
                    Waiting::File("file.open"),
                    "file.open".to_string(),
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
                Ok(Choice::Export(path)) => {
                    self.host.note_export(&path);
                    self.start(
                        Waiting::File("file.export"),
                        "file.export".to_string(),
                        Box::new(move |session| {
                            Outcome::Ran(files::export_as_user(session, &path))
                        }),
                    );
                }
                other => self.host.file_choice(other),
            }
            return true;
        }
        if let Some(action) = self.host.shell.take_file_action() {
            self.host.file_action(action, &mut self.picker);
            return true;
        }
        if let Some(action) = self.host.shell.take_doc_action() {
            self.doc_action(action);
            return true;
        }
        false
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
            self.host.ask_name_then_close(&mut self.picker);
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
        let Some(capture) = self.capture.as_mut() else {
            return;
        };
        if capture.image.is_some() {
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
                    asked: None,
                    since: Instant::now(),
                    image: None,
                });
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

/// True for a `ui.screenshot` request.
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
        let turn = desk.turn();
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
    let host = Host::new(session);
    let title = window_title(host.session.document_status().as_ref());
    let picker = Picker::new(Arc::new(SystemDialogs), Arc::clone(&wake));
    let desk = Rc::new(RefCell::new(
        Desk::new(host, requests, picker).with_engine_thread(Arc::clone(&wake)),
    ));
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(&title)
            // The desktop file's name (packaging/linux), so a Wayland
            // compositor finds the entry and the icon.
            .with_app_id(APP_ID)
            .with_inner_size(
                options
                    .window_size
                    .map_or([1280.0, 800.0], |[w, h]| [w as f32, h as f32]),
            )
            .with_min_inner_size([480.0, 320.0]),
        ..Default::default()
    };
    let native = match window_icon() {
        Some(icon) => eframe::NativeOptions {
            viewport: native.viewport.with_icon(icon),
            ..native
        },
        None => native,
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
            let shell = Shell::threaded_with_gpu(renderer, creation.wgpu_render_state.as_ref());
            announce(&format!(
                "viewport renderer: {}",
                shell.renderer_report().describe()
            ));
            if let Ok(mut desk) = for_viewer.try_borrow_mut() {
                desk.host.shell = shell;
            }
            let viewer = Viewer {
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
    use std::sync::atomic::{AtomicU64, Ordering};

    fn desk() -> (Desk, mpsc::Sender<Request>) {
        let (requests, incoming) = mpsc::channel();
        let picker = Picker::new(Arc::new(SystemDialogs), Arc::new(|| {}));
        let host = Host::new(Session::new(Workspace::none()));
        (Desk::new(host, incoming, picker), requests)
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
}
