//! "Allow AI agents": the control listener turned on and off while the
//! window runs, and what goes with it (docs/control-protocol.md,
//! "Connecting an agent").
//!
//! Off by default. On, the window listens on 127.0.0.1, on port 7878 or
//! the first free one after it, behind the control token in the state
//! folder's `control.token` (made on first use, mode 0600); it writes a
//! discovery entry there with its port, so `craft3d-cli mcp --bridge`
//! finds it with nothing on its command line (a window given a token file
//! of its own writes none: it is reached with that file). Off, the
//! listener is gone, every connection closed and the entry removed. The
//! person's choice is remembered in the state folder for the next start.
//! `--control PORT` turns it on from the start, for scripts; the switch
//! still turns it off.
//!
//! The roots (2026-11-02): an agent the switch lets in may read and write
//! files in the folder the person chose (Change…, remembered; by default
//! `Documents/3DCraft`, made when missing), or in the roots the command
//! line gave instead, which win; and, unless the person turned it off, in
//! the folder of the open document, following the active tab. Off takes
//! them back. `--control` alone keeps the command line's roots.
//!
//! The token is the guard: without it, any web page in a browser could
//! post a JSON line to the port. It is never shown or copied; the bridge
//! reads it from the file.

use crate::args::Options;
use crate::control::{ControlConfig, ControlServer, Wake};
use crate::host::Request;
use crate::token;
use craft3d_automation::discovery::{self, Preferences, WindowEntry};
use craft3d_engine::{Access, Workspace};
use craft3d_ui_egui::{AgentClient, AgentRoot, AgentsStatus};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

/// What the switch works with.
#[derive(Clone, Debug, Default)]
pub struct AgentsConfig {
    /// 3DCraft's folder in the state directory: the default token file,
    /// the discovery entries, the remembered choices. `None` when the
    /// environment names none: the switch then needs a token file given.
    pub state_dir: Option<PathBuf>,
    /// The token file given on the command line.
    pub token_file: Option<PathBuf>,
    /// `--control PORT`: listen from the start, on this port only.
    pub forced_port: Option<u16>,
    /// The first port the switch tries, and how many.
    pub first_port: u16,
    pub tries: u16,
    /// The folder agents get when the person chose none
    /// (`Documents/3DCraft`).
    pub work_folder: Option<PathBuf>,
    /// The roots the command line gave (read, write), which win over the
    /// chosen folder; `None` when it gave none.
    pub command_line: Option<(Access, Access)>,
    /// The person's home folder, to show paths beneath it with `~`.
    pub home: Option<String>,
    /// The process id the discovery entry goes under.
    pub pid: u32,
}

impl AgentsConfig {
    /// The configuration the command line and the environment give. The
    /// roots of the command line were checked when the session was made.
    pub fn from_options(options: &Options, env: &dyn Fn(&str) -> Option<String>) -> Self {
        let given = options.read_root.is_some() || options.write_root.is_some();
        let command_line = given.then(|| {
            let access =
                |flag: &str, dir: Option<&str>| crate::app::root(flag, dir).unwrap_or(Access::None);
            (
                access("--automation-read-root", options.read_root.as_deref()),
                access("--automation-write-root", options.write_root.as_deref()),
            )
        });
        let home = if cfg!(windows) {
            env("USERPROFILE")
        } else {
            env("HOME")
        };
        Self {
            state_dir: options.state_dir.clone(),
            token_file: options.token_file.clone(),
            forced_port: options.control,
            first_port: options.agent_port.unwrap_or(discovery::FIRST_PORT),
            tries: if options.agent_port == Some(0) {
                1
            } else {
                discovery::PORT_TRIES
            },
            work_folder: crate::places::suggested_work_folder(env).map(PathBuf::from),
            command_line,
            home: home.filter(|home| !home.is_empty()),
            pid: std::process::id(),
        }
    }

    /// The session's roots when the switch gives nothing: the command
    /// line's.
    fn base_workspace(&self) -> Workspace {
        match &self.command_line {
            Some((read, write)) => Workspace {
                read: read.clone(),
                write: write.clone(),
            },
            None => Workspace::none(),
        }
    }
}

/// The switch and the listener behind it.
pub struct Agents {
    config: AgentsConfig,
    requests: Sender<Request>,
    wake: Wake,
    server: Option<ControlServer>,
    /// The token file in use while listening, absolute.
    token_file: Option<PathBuf>,
    /// Why it could not be turned on, or what went wrong around it.
    error: Option<String>,
    /// What the person chose, as remembered.
    preferences: Preferences,
    /// The switch (not `--control`) lets agents in: they get the roots.
    switched: bool,
    /// The open document's folder, with its links resolved, and its path
    /// as last given.
    document: Option<PathBuf>,
    document_path: Option<PathBuf>,
    /// The chosen folder agents have, while they have it.
    granted: Option<PathBuf>,
    /// The roots agents have, and where each came from.
    roots: Vec<AgentRoot>,
    /// The session's roots as last made here.
    current: Workspace,
    /// The session's roots, changed, until the desk applies them (when the
    /// session is not on the engine thread).
    workspace: Option<Workspace>,
    started_at: u64,
    focused_at: u64,
}

impl Agents {
    pub fn new(config: AgentsConfig, requests: Sender<Request>, wake: Wake) -> Self {
        let preferences = config
            .state_dir
            .as_deref()
            .map(discovery::read_preferences)
            .unwrap_or_default();
        let current = config.base_workspace();
        Self {
            config,
            requests,
            wake,
            server: None,
            token_file: None,
            error: None,
            preferences,
            switched: false,
            document: None,
            document_path: None,
            granted: None,
            roots: Vec::new(),
            current,
            workspace: None,
            started_at: discovery::now_ms(),
            focused_at: 0,
        }
    }

    /// What the window starts with: listening when `--control` asks for it
    /// (and an error when it cannot: a script wants to know), else when
    /// the person left the switch on (an error then is shown, not fatal).
    pub fn start(&mut self) -> Result<(), String> {
        if self.config.forced_port.is_some() {
            return self.listen(false);
        }
        if self.preferences.allow_agents
            && let Err(e) = self.listen(true)
        {
            self.error = Some(e);
        }
        Ok(())
    }

    /// The person turned the switch: on or off, remembered for the next
    /// start. What goes wrong is kept for the status, never a panic.
    pub fn allow(&mut self, on: bool) {
        self.error = None;
        if on {
            if self.server.is_none()
                && let Err(e) = self.listen(true)
            {
                self.error = Some(e);
                return;
            }
            if !self.switched {
                self.switched = true;
                self.recompute();
            }
        } else {
            self.stop();
        }
        self.preferences.allow_agents = on;
        self.remember();
    }

    fn remember(&mut self) {
        if let Some(folder) = &self.config.state_dir
            && let Err(e) = discovery::write_preferences(folder, &self.preferences)
        {
            self.error = Some(format!("The choice could not be remembered: {e}"));
        }
    }

    /// The person chose another folder for the agents' files (Change…):
    /// made when missing, remembered. Refused, with the reason, when the
    /// command line gave the roots or the folder cannot be used; the
    /// folder before stays then.
    pub fn choose_folder(&mut self, folder: &Path) -> Result<(), String> {
        let refused = |e: String| e;
        if self.config.command_line.is_some() {
            return Err(refused(
                "the command line gave the folders agents may use (--automation-read-root, --automation-write-root)"
                    .to_string(),
            ));
        }
        let checked = std::path::absolute(folder)
            .map_err(|e| format!("`{}`: cannot use this folder: {e}", folder.display()))
            .and_then(|folder| usable_folder(&folder).map(|_| folder));
        let folder = match checked {
            Ok(folder) => folder,
            Err(e) => {
                self.error = Some(format!("That folder cannot be used: {e}"));
                return Err(e);
            }
        };
        self.preferences.folder = Some(folder);
        self.error = None;
        self.remember();
        self.recompute();
        Ok(())
    }

    /// Whether agents also get the open document's folder; remembered.
    pub fn set_document_folder(&mut self, on: bool) {
        self.preferences.document_folder = on;
        self.remember();
        self.recompute();
    }

    /// The open document's file changed (another tab, a save as, none):
    /// its folder is a root while the switch lets agents in.
    pub fn set_document(&mut self, path: Option<&Path>) {
        // Asked every turn: the file system is looked at only when the
        // path changes.
        if path == self.document_path.as_deref() {
            return;
        }
        self.document_path = path.map(Path::to_path_buf);
        let folder = path
            .filter(|path| path.is_absolute())
            .and_then(Path::parent)
            .and_then(|folder| craft3d_engine::canonical(folder).ok())
            .filter(|folder| folder.is_dir());
        if folder != self.document {
            self.document = folder;
            self.recompute();
        }
    }

    /// The folder chosen, or the default one.
    fn chosen_folder(&self) -> Option<PathBuf> {
        self.preferences
            .folder
            .clone()
            .or_else(|| self.config.work_folder.clone())
    }

    /// The session's roots from what is chosen now; the change, if any,
    /// waits for the desk.
    fn recompute(&mut self) {
        let mut roots: Vec<AgentRoot> = Vec::new();
        let mut workspace = self.config.base_workspace();
        self.granted = None;
        let entry = |path: &Path, from: &str, read: bool, write: bool| AgentRoot {
            path: path.to_string_lossy().into_owned(),
            from: from.to_string(),
            read,
            write,
        };
        if self.server.is_some() && self.switched {
            let mut read: Vec<PathBuf> = Vec::new();
            let mut write: Vec<PathBuf> = Vec::new();
            match &self.config.command_line {
                Some((given_read, given_write)) => {
                    let (r, w) = (given_read.roots(), given_write.roots());
                    for path in r.iter().chain(w.iter()) {
                        if !roots.iter().any(|root| Path::new(&root.path) == *path) {
                            roots.push(entry(
                                path,
                                "command line",
                                r.contains(path),
                                w.contains(path),
                            ));
                        }
                    }
                    read.extend(r.iter().map(|p| p.to_path_buf()));
                    write.extend(w.iter().map(|p| p.to_path_buf()));
                }
                None => {
                    if let Some(folder) = self.chosen_folder() {
                        match usable_folder(&folder) {
                            Ok(real) => {
                                roots.push(entry(&real, "chosen", true, true));
                                read.push(real.clone());
                                write.push(real.clone());
                                self.granted = Some(folder);
                            }
                            Err(e) => {
                                self.error =
                                    Some(format!("Agents may read and write no files: {e}"));
                            }
                        }
                    }
                }
            }
            if self.preferences.document_folder
                && let Some(document) = &self.document
            {
                roots.push(entry(document, "document", true, true));
                read.push(document.clone());
                write.push(document.clone());
            }
            workspace = Workspace {
                read: Access::from_roots(read),
                write: Access::from_roots(write),
            };
        } else if let Some((given_read, given_write)) = &self.config.command_line {
            let (r, w) = (given_read.roots(), given_write.roots());
            for path in r.iter().chain(w.iter()) {
                if !roots.iter().any(|root| Path::new(&root.path) == *path) {
                    roots.push(entry(
                        path,
                        "command line",
                        r.contains(path),
                        w.contains(path),
                    ));
                }
            }
        }
        self.roots = roots;
        if workspace != self.current {
            self.current = workspace.clone();
            self.workspace = Some(workspace);
        }
    }

    /// The token file to use: the one given, else the state folder's.
    fn token_path(&self) -> Result<PathBuf, String> {
        let path = match (&self.config.token_file, &self.config.state_dir) {
            (Some(file), _) => file.clone(),
            (None, Some(folder)) => discovery::default_token_file(folder),
            (None, None) => {
                return Err(
                    "there is no state folder to keep the control token in (HOME is not set); start 3DCraft with `--control-token-file FILE`"
                        .to_string(),
                );
            }
        };
        std::path::absolute(&path)
            .map_err(|e| format!("cannot use the token file `{}`: {e}", path.display()))
    }

    /// Starts listening; `switched` when the switch (not `--control`) asked,
    /// which gives agents their roots.
    fn listen(&mut self, switched: bool) -> Result<(), String> {
        let token_file = self.token_path()?;
        let token = token::load_or_create(&token_file)?;
        let (port, tries) = match self.config.forced_port {
            Some(port) => (port, 1),
            None => (self.config.first_port, self.config.tries),
        };
        let mut config = ControlConfig::new(port, token);
        config.tries = tries;
        let server = ControlServer::start(
            config,
            self.requests.clone(),
            std::sync::Arc::clone(&self.wake),
        )?;
        let port = server.address().port();
        self.server = Some(server);
        self.error = None;
        self.token_file = Some(token_file.clone());
        self.switched = switched;
        // A folder that cannot be used is said (`error`); agents still
        // connect.
        self.recompute();
        // Found by the bridge only with the state folder's own token: a
        // window given a token file of its own (a script's, a test's) is
        // reached with that file and its port.
        if let Some(folder) = self.discoverable() {
            // Windows that crashed or were killed leave entries behind.
            let _ = discovery::live_entries(&folder, Some(self.config.pid));
            if let Err(e) = discovery::write_entry(&folder, &self.entry(port, token_file)) {
                self.error = Some(format!(
                    "Agents can connect on port {port} (`craft3d-cli mcp --bridge {port}`), but the bridge cannot find this window by itself: {e}"
                ));
            }
        }
        Ok(())
    }

    /// The state folder, when this window writes a discovery entry there.
    fn discoverable(&self) -> Option<PathBuf> {
        self.config
            .state_dir
            .clone()
            .filter(|_| self.config.token_file.is_none())
    }

    fn entry(&self, port: u16, token_file: PathBuf) -> WindowEntry {
        WindowEntry {
            pid: self.config.pid,
            port,
            token_file,
            focused_at: self.focused_at,
            started_at: self.started_at,
        }
    }

    /// Stops listening: every connection closed, the entry removed, the
    /// roots the switch gave taken back.
    fn stop(&mut self) {
        if self.server.take().is_none() {
            return;
        }
        self.switched = false;
        if let Some(folder) = self.discoverable() {
            discovery::remove_entry(&folder, self.config.pid);
        }
        self.recompute();
    }

    /// Closes every agent's connection and refuses those agents for the
    /// rest of the run; how many were connected.
    pub fn disconnect_all(&mut self) -> usize {
        self.server
            .as_ref()
            .map_or(0, ControlServer::disconnect_all)
    }

    /// The window got the focus: the bridge prefers the window focused
    /// last.
    pub fn focused(&mut self) {
        self.focused_at = discovery::now_ms();
        if let (Some(server), Some(folder), Some(token_file)) =
            (&self.server, self.discoverable(), self.token_file.clone())
        {
            let entry = self.entry(server.address().port(), token_file);
            let _ = discovery::write_entry(&folder, &entry);
        }
    }

    /// The session's roots as the switch changed them, once.
    pub fn take_workspace(&mut self) -> Option<Workspace> {
        self.workspace.take()
    }

    /// Where it listens, while it does.
    pub fn address(&self) -> Option<SocketAddr> {
        self.server.as_ref().map(ControlServer::address)
    }

    /// The token file in use, while listening.
    pub fn token_file(&self) -> Option<&Path> {
        self.token_file.as_deref().filter(|_| self.server.is_some())
    }

    /// The folder Change… starts in.
    pub fn folder(&self) -> Option<PathBuf> {
        self.chosen_folder()
    }

    /// How things stand, for the shell and `ui.get`.
    pub fn status(&self) -> AgentsStatus {
        let clients = self
            .server
            .as_ref()
            .map(ControlServer::clients)
            .unwrap_or_default();
        AgentsStatus {
            available: true,
            allowed: self.server.is_some(),
            forced: self.config.forced_port.is_some(),
            port: self.address().map(|a| a.port()),
            connected: clients.len(),
            clients: clients
                .into_iter()
                .map(|client| AgentClient {
                    name: client.name,
                    since_ms: client.since_ms,
                    last_command: client.command,
                    last_ms: client.command_ms,
                })
                .collect(),
            work_folder: self
                .granted
                .as_ref()
                .map(|folder| folder.to_string_lossy().into_owned()),
            command_line: self.config.command_line.is_some(),
            document_folder: self.preferences.document_folder,
            roots: self.roots.clone(),
            error: self.error.clone(),
            home: self.config.home.clone(),
        }
    }
}

/// `folder` made when missing and checked to be a folder; its spelling
/// with links resolved (the session compares roots that way).
fn usable_folder(folder: &Path) -> Result<PathBuf, String> {
    let shown = folder.display();
    std::fs::create_dir_all(folder)
        .map_err(|e| format!("`{shown}`: cannot make this folder: {e}"))?;
    let real = craft3d_engine::canonical(folder)
        .map_err(|e| format!("`{shown}`: cannot use this folder: {e}"))?;
    if !real.is_dir() {
        return Err(format!("`{shown}` is not a folder"));
    }
    Ok(real)
}

impl Drop for Agents {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::Host;
    use craft3d_automation::bridge::{self, Bridge, BridgeConfig};
    use craft3d_engine::{Access, Session};
    use serde_json::{Value, json};
    use std::io::{BufRead, BufReader, Write};
    use std::net::{Ipv4Addr, TcpListener, TcpStream};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};

    fn workdir(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../target/tmp/craft3d-agents-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Canonical: the session's roots are compared as text.
        craft3d_engine::canonical(&dir).unwrap()
    }

    /// A port that was free a moment ago (never 7878: the developer's
    /// window may listen there).
    fn free_port() -> u16 {
        TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    fn config(state: &Path) -> AgentsConfig {
        AgentsConfig {
            state_dir: Some(state.to_path_buf()),
            token_file: None,
            forced_port: None,
            first_port: free_port(),
            tries: 3,
            work_folder: None,
            command_line: None,
            home: None,
            pid: std::process::id(),
        }
    }

    /// The thread that stands in for the UI thread: answers what comes in.
    struct Pump {
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
    }

    impl Pump {
        fn start(incoming: Receiver<Request>) -> Self {
            let stop = Arc::new(AtomicBool::new(false));
            let for_thread = Arc::clone(&stop);
            let thread = std::thread::spawn(move || {
                let mut host = Host::new(Session::new(Workspace::none()));
                loop {
                    match incoming.recv_timeout(Duration::from_millis(5)) {
                        Ok(request) => host.answer(request),
                        Err(RecvTimeoutError::Timeout) => {
                            if for_thread.load(Ordering::SeqCst) {
                                break;
                            }
                        }
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
            });
            Self {
                stop,
                thread: Some(thread),
            }
        }
    }

    impl Drop for Pump {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    fn agents(config: AgentsConfig) -> (Agents, Pump) {
        let (requests, incoming) = mpsc::channel();
        let agents = Agents::new(config, requests, Arc::new(|| {}));
        (agents, Pump::start(incoming))
    }

    fn eventually(what: &str, mut done: impl FnMut() -> bool) {
        let start = Instant::now();
        while !done() {
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "timed out: {what}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Authenticates on `port` with `token`; the reply.
    fn auth(port: u16, token: &str) -> (Value, BufReader<TcpStream>, TcpStream) {
        let stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .unwrap();
        let mut writer = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);
        writeln!(
            writer,
            "{}",
            json!({"id": 1, "method": "auth", "params": {"token": token}})
        )
        .unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        (serde_json::from_str(&line).unwrap(), reader, writer)
    }

    fn closed(reader: &mut BufReader<TcpStream>) -> bool {
        let mut line = String::new();
        matches!(reader.read_line(&mut line), Ok(0) | Err(_))
    }

    #[test]
    fn the_switch_turns_the_listener_on_off_and_on_in_one_process() {
        let state = workdir("switch");
        // A window that was killed left its entry behind.
        let stale = WindowEntry {
            pid: u32::MAX - 7,
            port: free_port(),
            token_file: state.join("control.token"),
            focused_at: 0,
            started_at: 0,
        };
        discovery::write_entry(&state, &stale).unwrap();
        let start = config(&state);
        let first = start.first_port;
        let (mut agents, _pump) = agents(start);
        assert!(agents.address().is_none(), "off by default");
        assert!(!agents.status().allowed);

        agents.allow(true);
        let status = agents.status();
        assert!(status.allowed && status.available, "{status:?}");
        assert_eq!(status.error, None);
        let port = agents.address().unwrap().port();
        assert_eq!(port, first, "the first free port of the range");
        // The default token file: made, private, and nowhere else.
        let token_file = state.join(discovery::TOKEN_FILE);
        let token = discovery::read_token(&token_file).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&token_file).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // The entry the bridge finds, and the stale one gone.
        let entries = discovery::read_entries(&state);
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert_eq!(
            (entries[0].pid, entries[0].port),
            (std::process::id(), port)
        );
        assert_eq!(entries[0].token_file, token_file);
        assert!(discovery::read_preference(&state), "remembered on");

        let (reply, mut reader, _writer) = auth(port, &token);
        assert_eq!(reply["ok"], true, "{reply}");
        eventually("the connection is counted", || {
            agents.status().connected == 1
        });

        // Off: the connection is closed, the port released, the entry gone.
        agents.allow(false);
        assert!(agents.address().is_none());
        assert!(closed(&mut reader), "the open connection was closed");
        assert!(
            TcpStream::connect_timeout(
                &SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
                Duration::from_secs(1)
            )
            .is_err(),
            "nothing listens any more"
        );
        assert!(discovery::read_entries(&state).is_empty());
        assert!(!discovery::read_preference(&state), "remembered off");
        assert_eq!(agents.status().connected, 0);

        // On again: the same port is bound again, with the same token.
        agents.allow(true);
        assert_eq!(agents.address().unwrap().port(), port);
        assert_eq!(discovery::read_token(&token_file).unwrap(), token);
        let (reply, _, _) = auth(port, &token);
        assert_eq!(reply["ok"], true, "{reply}");
        // Turning on what is on changes nothing.
        agents.allow(true);
        assert_eq!(agents.address().unwrap().port(), port);
        // Dropped with the window: the entry goes with it.
        drop(agents);
        assert!(discovery::read_entries(&state).is_empty());
        // A token file of its own (a script's window): no entry.
        let mut own = config(&state);
        own.token_file = Some(state.join("own.token"));
        let (mut agents, _pump) = super::tests::agents(own);
        agents.allow(true);
        assert!(agents.address().is_some());
        assert!(discovery::read_entries(&state).is_empty());
        assert!(state.join("own.token").is_file());
        let _ = std::fs::remove_dir_all(&state);
    }

    #[test]
    fn the_remembered_choice_is_honoured_at_the_next_start() {
        let state = workdir("remembered");
        let (mut agents, _pump) = agents(config(&state));
        agents.start().unwrap();
        assert!(agents.address().is_none(), "nothing remembered: off");
        drop(agents);
        discovery::write_preference(&state, true).unwrap();
        let (mut agents, _pump) = agents_again(&state);
        agents.start().unwrap();
        assert!(agents.address().is_some(), "remembered on");
        drop(agents);
        discovery::write_preference(&state, false).unwrap();
        let (mut agents, _pump) = agents_again(&state);
        agents.start().unwrap();
        assert!(agents.address().is_none(), "remembered off");
        // `--control` turns it on whatever was remembered, on its port.
        let mut forced = config(&state);
        let port = free_port();
        forced.forced_port = Some(port);
        let (mut agents, _pump) = super::tests::agents(forced);
        agents.start().unwrap();
        assert_eq!(agents.address().unwrap().port(), port);
        assert!(agents.status().forced);
        let _ = std::fs::remove_dir_all(&state);
    }

    fn agents_again(state: &Path) -> (Agents, Pump) {
        agents(config(state))
    }

    /// What goes wrong is an error in the status (or from `start` for
    /// `--control`), never a panic.
    #[test]
    fn errors_are_said_not_panicked() {
        let state = workdir("errors");
        // A token file that holds something else.
        std::fs::write(state.join(discovery::TOKEN_FILE), "not a token").unwrap();
        let (mut agents, _pump) = agents(config(&state));
        agents.allow(true);
        let error = agents.status().error.unwrap();
        assert!(error.contains("does not hold a control token"), "{error}");
        assert!(agents.address().is_none() && !agents.status().allowed);
        // `--control` with it: an error from `start`.
        let mut forced = config(&state);
        forced.forced_port = Some(0);
        let (mut agents, _pump) = super::tests::agents(forced);
        assert!(agents.start().is_err());

        // A state folder that cannot be one (a file stands there).
        let blocked = state.join("blocked");
        std::fs::write(&blocked, "").unwrap();
        let (mut agents, _pump) = super::tests::agents(config(&blocked));
        agents.allow(true);
        let error = agents.status().error.unwrap();
        assert!(error.contains("control token file"), "{error}");
        assert!(agents.address().is_none());

        // No state folder and no token file.
        let mut nowhere = config(&state);
        nowhere.state_dir = None;
        let (mut agents, _pump) = super::tests::agents(nowhere);
        agents.allow(true);
        assert!(
            agents
                .status()
                .error
                .unwrap()
                .contains("--control-token-file")
        );

        // Every port of the range taken.
        let fresh = workdir("errors-ports");
        let held = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let first = held.local_addr().unwrap().port();
        // The next one, held too when it is free (taken by someone else
        // is just as taken).
        let _next = first
            .checked_add(1)
            .and_then(|next| TcpListener::bind((Ipv4Addr::LOCALHOST, next)).ok());
        let mut full = config(&fresh);
        full.first_port = first;
        full.tries = 2;
        let (mut agents, _pump) = super::tests::agents(full);
        agents.allow(true);
        let error = agents.status().error.unwrap();
        assert!(error.contains("every port from"), "{error}");
        assert!(agents.address().is_none());
        // A port at the top of the range does not wrap around.
        let mut top = config(&fresh);
        top.first_port = u16::MAX;
        top.tries = 5;
        let (mut agents, _pump) = super::tests::agents(top);
        agents.allow(true);
        let _ = agents.status();
        let _ = std::fs::remove_dir_all(&state);
        let _ = std::fs::remove_dir_all(&fresh);
    }

    /// The bridge with nothing given finds the window and its token, the
    /// window counts it while it works and lets it go when it says bye;
    /// "Disconnect all" sends it away for the rest of the run.
    #[test]
    fn the_bridge_finds_the_window_and_is_counted_until_it_says_bye() {
        let state = workdir("bridge");
        let (mut agents, _pump) = agents(config(&state));
        agents.allow(true);
        let initialize = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                       "clientInfo": {"name": "test", "version": "0"}}})
        .to_string();
        let mut bridge = Bridge::new(BridgeConfig::discover(state.clone(), None));
        let reply: Value =
            serde_json::from_str(&bridge::relay(&mut bridge, &initialize).unwrap()).unwrap();
        assert_eq!(reply["result"]["protocolVersion"], "2025-06-18", "{reply}");
        let status = agents.status();
        assert_eq!(status.connected, 1, "{status:?}");
        // Named by the MCP client's own `initialize`.
        assert_eq!(status.clients[0].name, "test", "{status:?}");
        bridge.bye();
        eventually("bye is heard", || agents.status().connected == 0);

        // Through `serve`, as `craft3d-cli mcp --bridge` runs it: the bye
        // comes at the end of the input.
        let mut output = Vec::new();
        bridge::serve(
            &BridgeConfig::discover(state.clone(), Some(agents.address().unwrap().port())),
            format!("{initialize}\n").as_bytes(),
            &mut output,
        )
        .unwrap();
        assert!(
            String::from_utf8(output)
                .unwrap()
                .contains("protocolVersion")
        );
        eventually("served and gone", || agents.status().connected == 0);

        // Disconnect all: the agent is refused from then on.
        let mut bridge = Bridge::new(BridgeConfig::discover(state.clone(), None));
        assert!(bridge::relay(&mut bridge, &initialize).is_some());
        assert_eq!(agents.status().connected, 1);
        assert_eq!(agents.disconnect_all(), 1);
        eventually("all disconnected", || agents.status().connected == 0);
        let refused: Value =
            serde_json::from_str(&bridge::relay(&mut bridge, &initialize).unwrap()).unwrap();
        let message = refused["error"]["message"].as_str().unwrap();
        assert!(message.contains("disconnected"), "{message}");
        // A new agent (another process) gets in.
        let mut other = BridgeConfig::discover(state.clone(), None);
        other.client = Some("MCP bridge (process 1)".to_string());
        let mut other = Bridge::new(other);
        let reply: Value =
            serde_json::from_str(&bridge::relay(&mut other, &initialize).unwrap()).unwrap();
        assert!(reply.get("result").is_some(), "{reply}");

        // Switched off: the bridge says how to turn it on.
        agents.allow(false);
        let mut late = Bridge::new(BridgeConfig::discover(state.clone(), None));
        let reply: Value =
            serde_json::from_str(&bridge::relay(&mut late, &initialize).unwrap()).unwrap();
        let message = reply["error"]["message"].as_str().unwrap();
        assert!(message.contains("Allow AI agents"), "{message}");
        let _ = std::fs::remove_dir_all(&state);
    }

    /// The switch on the desk: the shell asks, the desk turns the listener
    /// on, gives agents the work folder as their roots and the shell (and
    /// `ui.get`) the status; off takes it all back.
    #[test]
    fn the_shells_switch_reaches_the_listener_and_the_roots() {
        use crate::app::Desk;
        use crate::files::{Picker, SystemDialogs};
        let state = workdir("desk");
        let work = state.join("Documents").join("3DCraft");
        let mut config = config(&state);
        config.work_folder = Some(work.clone());
        let (requests, incoming) = mpsc::channel();
        let agents = Agents::new(config, requests, Arc::new(|| {}));
        let picker = Picker::new(Arc::new(SystemDialogs), Arc::new(|| {}));
        let host = Host::new(Session::new(Workspace::none()));
        let mut desk = Desk::new(host, incoming, picker).with_agents(agents);
        desk.turn();
        assert!(desk.host.shell.agents_status().available);
        assert!(!desk.host.shell.agents_status().allowed);

        desk.host.shell.request_agents(true);
        assert!(desk.turn().changed);
        let status = desk.host.shell.agents_status().clone();
        assert!(status.allowed, "{status:?}");
        assert_eq!(status.work_folder.as_deref(), work.to_str());
        assert!(work.is_dir(), "the folder is made");
        let root = craft3d_engine::canonical(&work).unwrap();
        assert_eq!(
            *desk.host.session.workspace(),
            Workspace {
                read: Access::Root(root.clone()),
                write: Access::Root(root),
            }
        );
        // `ui.get` reports it, read-only.
        let got = desk
            .host
            .shell
            .handle_method(&mut desk.host.session, "ui.get", None)
            .unwrap()
            .unwrap();
        assert_eq!(got["agents"]["allowed"], true, "{}", got["agents"]);
        assert_eq!(got["agents"]["port"], status.port.unwrap());
        assert!(
            desk.host
                .shell
                .handle_method(
                    &mut desk.host.session,
                    "ui.set",
                    Some(&json!({"agents": {"allowed": false}}))
                )
                .unwrap()
                .is_err(),
            "not settable over the channel"
        );

        desk.host.shell.request_agents(false);
        desk.turn();
        assert!(!desk.host.shell.agents_status().allowed);
        assert_eq!(*desk.host.session.workspace(), Workspace::none());
        assert!(desk.agents().unwrap().address().is_none());
        let _ = std::fs::remove_dir_all(&state);
    }

    /// A desk whose switch is on, with the agents' chosen folder `chosen`
    /// and two documents saved in folders of their own.
    fn desk_with_two_documents(
        state: &Path,
        chosen: &Path,
    ) -> (crate::app::Desk, PathBuf, PathBuf) {
        use crate::app::Desk;
        use crate::files::{Picker, SystemDialogs};
        let (one, two) = (state.join("one"), state.join("two"));
        std::fs::create_dir_all(&one).unwrap();
        std::fs::create_dir_all(&two).unwrap();
        let mut config = config(state);
        config.work_folder = Some(chosen.to_path_buf());
        let (requests, incoming) = mpsc::channel();
        let agents = Agents::new(config, requests, Arc::new(|| {}));
        let picker = Picker::new(Arc::new(SystemDialogs), Arc::new(|| {}));
        let mut session = Session::new(Workspace::unrestricted());
        for dir in [&one, &two] {
            session.execute("file.new", &json!({})).unwrap();
            session
                .execute("feature.block", &json!({"size": [1, 2, 3]}))
                .unwrap();
            let path = dir.join("part.3dcraft");
            session
                .execute("file.save", &json!({"path": path.to_str().unwrap()}))
                .unwrap();
        }
        session.set_workspace(Workspace::none());
        let mut desk = Desk::new(Host::new(session), incoming, picker).with_agents(agents);
        desk.host.shell.request_agents(true);
        desk.turn();
        (
            desk,
            craft3d_engine::canonical(&one).unwrap(),
            craft3d_engine::canonical(&two).unwrap(),
        )
    }

    fn roots_from(desk: &crate::app::Desk) -> Vec<(String, String)> {
        desk.host
            .shell
            .agents_status()
            .roots
            .iter()
            .map(|root| (root.from.clone(), root.path.clone()))
            .collect()
    }

    /// The open document's folder is a root while the switch is on: it
    /// follows the active tab, goes with the check box, and goes when the
    /// switch is turned off.
    #[test]
    fn the_document_folder_follows_the_active_tab_and_the_switch() {
        let state = workdir("document-folder");
        let chosen = state.join("Documents").join("3DCraft");
        let (mut desk, one, two) = desk_with_two_documents(&state, &chosen);
        let chosen = craft3d_engine::canonical(&chosen).unwrap();
        let text = |p: &Path| p.to_str().unwrap().to_string();
        // The second document is active.
        assert_eq!(
            roots_from(&desk),
            [
                ("chosen".to_string(), text(&chosen)),
                ("document".to_string(), text(&two))
            ]
        );
        let workspace = desk.host.session.workspace().clone();
        assert_eq!(
            workspace.write,
            Access::Roots(vec![chosen.clone(), two.clone()])
        );
        // Relative paths go to the chosen folder; the document's folder by
        // its absolute path; the other document's folder is not a root.
        assert_eq!(
            workspace.resolve_write("x.stl").unwrap(),
            chosen.join("x.stl")
        );
        assert!(workspace.resolve_write(&text(&two.join("x.stl"))).is_ok());
        assert!(workspace.resolve_write(&text(&one.join("x.stl"))).is_err());
        // Another tab: its folder instead.
        desk.host
            .session
            .execute("file.activate", &json!({"document": 0}))
            .unwrap();
        desk.turn();
        assert_eq!(roots_from(&desk)[1], ("document".to_string(), text(&one)));
        assert!(
            desk.host
                .session
                .workspace()
                .resolve_write(&text(&one.join("x.stl")))
                .is_ok()
        );
        // A new document has no folder: nothing added.
        desk.host.session.execute("file.new", &json!({})).unwrap();
        desk.turn();
        assert_eq!(roots_from(&desk).len(), 1);
        assert_eq!(
            desk.host.session.workspace().write,
            Access::Root(chosen.clone())
        );
        // The check box off: no document folder, and it is remembered.
        desk.host
            .session
            .execute("file.activate", &json!({"document": 1}))
            .unwrap();
        desk.host.shell.request_agents_document_folder(false);
        desk.turn();
        assert_eq!(roots_from(&desk).len(), 1);
        assert!(!discovery::read_preferences(&state).document_folder);
        desk.host.shell.request_agents_document_folder(true);
        desk.turn();
        assert_eq!(roots_from(&desk).len(), 2);
        // The switch off: every root goes.
        desk.host.shell.request_agents(false);
        desk.turn();
        assert!(roots_from(&desk).is_empty());
        assert_eq!(*desk.host.session.workspace(), Workspace::none());
        let _ = std::fs::remove_dir_all(&state);
    }

    /// The chosen folder is remembered and used at the next start; one
    /// that cannot be made is refused with the reason, and the folder
    /// before stays.
    #[test]
    fn the_chosen_folder_is_remembered_and_a_bad_one_refused() {
        let state = workdir("chosen-folder");
        let (mut agents, _pump) = agents(config(&state));
        agents.allow(true);
        let mine = state.join("mine").join("agents");
        agents.choose_folder(&mine).unwrap();
        assert!(mine.is_dir(), "made when missing");
        assert_eq!(
            discovery::read_preferences(&state).folder,
            Some(mine.clone())
        );
        let status = agents.status();
        assert_eq!(status.work_folder.as_deref(), mine.to_str());
        assert_eq!(status.roots[0].from, "chosen");
        // A folder beneath a file cannot be made: an error, kept as it was.
        let file = state.join("a-file");
        std::fs::write(&file, "x").unwrap();
        let error = agents.choose_folder(&file.join("below")).unwrap_err();
        assert!(error.contains("cannot make this folder"), "{error}");
        assert!(agents.status().error.unwrap().contains("cannot be used"));
        assert_eq!(agents.status().work_folder.as_deref(), mine.to_str());
        // A file is not a folder.
        assert!(agents.choose_folder(&file).is_err());
        drop(agents);
        // The next start: the remembered folder.
        let (mut again, _pump) = super::tests::agents(config(&state));
        again.start().unwrap();
        assert!(again.address().is_some(), "remembered on");
        assert_eq!(again.status().work_folder.as_deref(), mine.to_str());
        // A default folder that cannot be made: said, agents still connect.
        let mut blocked = config(&workdir("chosen-folder-blocked"));
        blocked.work_folder = Some(file.join("Documents"));
        let (mut agents, _pump) = super::tests::agents(blocked);
        agents.allow(true);
        assert!(agents.address().is_some());
        let error = agents.status().error.unwrap();
        assert!(error.contains("no files"), "{error}");
        assert!(agents.status().roots.is_empty());
        let _ = std::fs::remove_dir_all(&state);
    }

    /// The command line's roots win: Change… is refused and they are the
    /// roots, with the document's folder after them.
    #[test]
    fn command_line_roots_win_over_the_chosen_folder() {
        let state = workdir("command-line");
        let given = state.join("given");
        std::fs::create_dir_all(&given).unwrap();
        let given = craft3d_engine::canonical(&given).unwrap();
        let mut config = config(&state);
        config.work_folder = Some(state.join("unused"));
        config.command_line = Some((Access::Root(given.clone()), Access::Root(given.clone())));
        let (mut agents, _pump) = agents(config);
        agents.allow(true);
        let status = agents.status();
        assert!(status.command_line);
        assert_eq!(status.roots.len(), 1);
        assert_eq!(status.roots[0].from, "command line");
        assert!(status.roots[0].read && status.roots[0].write);
        assert_eq!(status.work_folder, None);
        assert!(!state.join("unused").exists(), "the default is not made");
        assert!(agents.choose_folder(&state.join("other")).is_err());
        let document = state.join("doc");
        std::fs::create_dir_all(&document).unwrap();
        agents.set_document(Some(&document.join("a.3dcraft")));
        let workspace = agents.take_workspace().unwrap();
        assert_eq!(workspace.read.roots()[0], given.as_path());
        assert_eq!(workspace.read.roots().len(), 2);
        // Off: the command line's roots again, nothing more.
        agents.allow(false);
        let workspace = agents.take_workspace().unwrap();
        assert_eq!(workspace.write, Access::Root(given.clone()));
        let _ = std::fs::remove_dir_all(&state);
    }

    /// A path outside every root is refused, and so is a link beneath a
    /// root that points out of it.
    #[cfg(unix)]
    #[test]
    fn the_agents_roots_hold_paths_in() {
        let state = workdir("containment");
        let chosen = state.join("chosen");
        let outside = state.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        let mut config = config(&state);
        config.work_folder = Some(chosen.clone());
        let (mut agents, _pump) = agents(config);
        agents.allow(true);
        let workspace = agents.take_workspace().unwrap();
        std::os::unix::fs::symlink(&outside, chosen.join("out")).unwrap();
        for bad in [
            "../outside/x.stl".to_string(),
            "out/x.stl".to_string(),
            outside.join("x.stl").to_str().unwrap().to_string(),
            chosen
                .join("out")
                .join("x.stl")
                .to_str()
                .unwrap()
                .to_string(),
        ] {
            assert!(workspace.resolve_write(&bad).is_err(), "{bad}");
            assert!(workspace.resolve_read(&bad).is_err(), "{bad}");
        }
        assert!(workspace.resolve_write("in/x.stl").is_ok());
        let _ = std::fs::remove_dir_all(&state);
    }
}
