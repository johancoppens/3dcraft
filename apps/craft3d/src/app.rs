//! The window: eframe around the shell, with the control channel's
//! requests answered on the UI thread between frames.

use crate::args::Options;
use crate::control::{ControlConfig, ControlServer, Wake};
use crate::host::{Host, Request};
use crate::token;
use craft3d_engine::{Access, Session, Workspace};
use eframe::egui;
use serde_json::json;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, OnceLock};

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

struct Viewer {
    host: Host,
    requests: Receiver<Request>,
    /// Kept so the listener lives as long as the window.
    _server: Option<ControlServer>,
}

impl eframe::App for Viewer {
    /// Runs before every frame, and also while the window is hidden: a
    /// request is answered whether or not anyone is looking.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.host.pump(&self.requests) > 0 {
            // Show what the requests changed, and look for more.
            ctx.request_repaint();
        }
        if self.host.shell.quit_requested() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let Host { session, shell } = &mut self.host;
        shell.ui(ui, session);
    }
}

/// Opens the window and runs until it is closed. `announce` is told where
/// the control channel listens, once it does.
pub fn run(options: &Options, announce: &dyn Fn(&str)) -> Result<(), String> {
    let session = session(options)?;
    // The listener starts before the window, so a port that is taken is an
    // error on the command line, not a window without a control channel.
    let context: Arc<OnceLock<egui::Context>> = Arc::new(OnceLock::new());
    let for_wake = Arc::clone(&context);
    let wake: Wake = Arc::new(move || {
        if let Some(ctx) = for_wake.get() {
            ctx.request_repaint();
        }
    });
    let (server, requests) = control(options, wake)?;
    if let (Some(server), Some(token_file)) = (&server, &options.token_file) {
        announce(&format!(
            "control channel on {} (token in {})",
            server.address(),
            token_file.display()
        ));
    }
    let viewer = Viewer {
        host: Host::new(session),
        requests,
        _server: server,
    };
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(TITLE)
            .with_app_id("craft3d")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([480.0, 320.0]),
        ..Default::default()
    };
    eframe::run_native(
        TITLE,
        native,
        Box::new(move |creation| {
            // Requests that arrived before the window existed are waiting
            // in the channel; the first frame's `logic` answers them.
            let _ = context.set(creation.egui_ctx.clone());
            creation.egui_ctx.request_repaint();
            Ok(Box::new(viewer))
        }),
    )
    .map_err(|e| format!("the window could not be opened: {e}"))
}
