//! The window: eframe around the shell, on an event loop of our own, with
//! the control channel's requests answered on the UI thread whenever that
//! loop is woken for them, whether or not a frame is drawn.

use crate::args::Options;
use crate::control::{ControlConfig, ControlServer, Wake};
use crate::files::{Picker, SystemDialogs};
use crate::host::{Host, Request};
use crate::token;
use craft3d_engine::{Access, RecoveryOptions, Session, Workspace};
use craft3d_ui_egui::{Shell, window_title};
use eframe::UserEvent;
use eframe::egui;
use serde_json::json;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::WindowId;

/// The window's title: the product name, as users read it.
pub const TITLE: &str = "3DCraft";

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
pub struct Desk {
    pub host: Host,
    requests: Receiver<Request>,
    /// The open and save dialogs.
    picker: Picker,
}

/// What one turn of serving did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Turn {
    /// Something changed that a frame should show, or more may be waiting.
    pub changed: bool,
    /// The application was asked to stop (`ui.quit`).
    pub quit: bool,
}

impl Desk {
    pub fn new(host: Host, requests: Receiver<Request>, picker: Picker) -> Self {
        Self {
            host,
            requests,
            picker,
        }
    }

    /// Answers the requests that are waiting, then does what the File menu
    /// asked for in the last frame and what a dialog answered since.
    /// Needs no frame and no window.
    pub fn turn(&mut self) -> Turn {
        let answered = self.host.pump(&self.requests) > 0;
        let filed = self.host.pump_files(&mut self.picker);
        Turn {
            changed: answered || filed,
            quit: self.host.shell.quit_requested(),
        }
    }
}

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
        let title = window_title(desk.host.session.document_status().as_ref());
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
        let Host { session, shell } = &mut desk.host;
        shell.ui(ui, session);
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
    let desk = Rc::new(RefCell::new(Desk::new(host, requests, picker)));
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(&title)
            .with_app_id("craft3d")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([480.0, 320.0]),
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
    // The window was closed, or could not be made: either way the
    // application ends on purpose, and leaves no recovery files. (A crash
    // or a kill never gets here, and leaves them.)
    if let Ok(mut desk) = driver.desk.try_borrow_mut() {
        desk.host.session.end_recovery();
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
