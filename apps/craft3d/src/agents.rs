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
use crate::consent::{Origin, Protected, Wanted};
use crate::control::{ControlConfig, ControlServer, Wake};
use crate::host::Request;
use crate::token;
use craft3d_automation::discovery::{self, Preferences, WindowEntry};
use craft3d_engine::{Access, Workspace};
use craft3d_ui_egui::{AgentClient, AgentPrompt, AgentRoot, AgentsStatus, PromptAnswer};
use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::Instant;

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
    /// What is never offered to an agent (`consent.rs`): the home folder,
    /// keys and settings, the system; the state folder is added to it
    /// whatever is given here.
    pub protected: Protected,
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
            protected: Protected::from_env(env, options.state_dir.as_deref()),
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
    /// Folders allowed for this run (This session only).
    session_folders: Vec<PathBuf>,
    /// The questions waiting for the person, oldest first.
    prompts: Vec<Pending>,
    next_prompt: u64,
    /// The clients asked about their working folder this run.
    asked: BTreeSet<String>,
    /// What is never offered.
    protected: Protected,
}

/// Most questions waiting for the person at once, and per client.
pub const MAX_PROMPTS: usize = 32;
pub const MAX_PROMPTS_PER_CLIENT: usize = 4;
/// How long a connect question stays when nobody answers it.
pub const CONNECT_WAIT: std::time::Duration = std::time::Duration::from_secs(600);
/// Most clients remembered as asked about (then the list starts again).
const MAX_ASKED: usize = 1024;

/// A question waiting for the person.
#[derive(Clone, Debug)]
pub struct Pending {
    pub prompt: AgentPrompt,
    /// When it was asked: a command waits [`crate::consent::PROMPT_WAIT`].
    pub asked: Instant,
    /// The name the asking client authenticated with (a connect
    /// question's): Refuse disconnects it by that name.
    pub key: Option<String>,
}

/// What the person's answer means for the request that waited.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Run it (its folder allowed from now on).
    Run,
    /// Run it with this one file allowed, in this direction, for it alone.
    Once { file: PathBuf, write: bool },
    /// Refuse it, with this reason.
    Refuse(String),
}

impl Agents {
    pub fn new(config: AgentsConfig, requests: Sender<Request>, wake: Wake) -> Self {
        let preferences = config
            .state_dir
            .as_deref()
            .map(discovery::read_preferences)
            .unwrap_or_default();
        let current = config.base_workspace();
        // The state folder (the token, the remembered choices) is never
        // offered, whatever the configuration says.
        let mut protected = config.protected.clone();
        if let Some(state) = &config.state_dir {
            protected.state =
                Some(craft3d_engine::canonical(state).unwrap_or_else(|_| state.clone()));
        }
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
            session_folders: Vec::new(),
            prompts: Vec::new(),
            next_prompt: 1,
            asked: BTreeSet::new(),
            protected,
        }
    }

    /// True while the person's switch lets agents in: only then is the
    /// person asked (a window `--control` opened for a script is not).
    pub fn asks(&self) -> bool {
        self.server.is_some() && self.switched
    }

    /// The session's roots as they are now.
    pub fn workspace(&self) -> &Workspace {
        &self.current
    }

    /// Whether `folder` lies beneath one of the agents' roots now.
    fn reachable(&self, folder: &Path) -> bool {
        let workspace = &self.current;
        workspace
            .read
            .roots()
            .iter()
            .chain(workspace.write.roots().iter())
            .any(|root| {
                folder.starts_with(root)
                    || craft3d_engine::canonical(root).is_ok_and(|real| folder.starts_with(real))
            })
    }

    /// Asks the person about the working folders of the clients that
    /// connected, once per client and run: one that is reachable already,
    /// protected, or missing is not asked about.
    pub fn ask_about_clients(&mut self) {
        if !self.asks() {
            return;
        }
        let clients = self
            .server
            .as_ref()
            .map(ControlServer::clients)
            .unwrap_or_default();
        for client in clients {
            let (Some(key), Some(cwd)) = (client.key, client.cwd) else {
                continue;
            };
            // Asked under the name the person knows (its MCP client's,
            // from `initialize`), or once it has done something without
            // giving one: not in the moment between its `auth` and its
            // first message.
            if !client.named && client.command.is_none() {
                continue;
            }
            if self.asked.contains(&key) || self.prompts.len() >= MAX_PROMPTS {
                continue;
            }
            if self.asked.len() >= MAX_ASKED {
                self.asked.clear();
            }
            self.asked.insert(key.clone());
            if cwd.to_string_lossy().chars().any(crate::consent::deceptive) {
                continue;
            }
            let Ok(folder) = craft3d_engine::canonical(&cwd) else {
                continue;
            };
            if !folder.is_dir()
                || self.reachable(&folder)
                || self.protected.reason(&folder).is_some()
            {
                continue;
            }
            let text = folder.to_string_lossy().into_owned();
            self.push_prompt_from(
                Some(key.clone()),
                AgentPrompt {
                    id: 0,
                    kind: "connect".to_string(),
                    client: client.name,
                    path: text.clone(),
                    folder: text,
                    file_name: String::new(),
                    command: None,
                },
            );
        }
    }

    fn push_prompt(&mut self, prompt: AgentPrompt) -> u64 {
        self.push_prompt_from(None, prompt)
    }

    /// A question asked of the person, from the client that authenticated
    /// as `key` (when known).
    fn push_prompt_from(&mut self, key: Option<String>, mut prompt: AgentPrompt) -> u64 {
        let id = self.next_prompt;
        self.next_prompt = self.next_prompt.saturating_add(1);
        prompt.id = id;
        self.prompts.push(Pending {
            prompt,
            asked: Instant::now(),
            key,
        });
        id
    }

    /// What to do with a request of `origin` that names `wanted` paths:
    /// `Ok(None)` runs it (nothing outside the roots, or nothing to ask
    /// about: the command then says what is wrong itself), `Ok(Some(id))`
    /// holds it for prompt `id`, `Err` refuses it at once (a protected
    /// place).
    pub fn screen(&mut self, origin: &Origin, wanted: &[Wanted]) -> Result<Option<u64>, String> {
        if !self.asks() {
            return Ok(None);
        }
        // Bounded: a flood of requests is refused, not queued for the
        // person.
        let theirs = self
            .prompts
            .iter()
            .filter(|p| p.prompt.client == origin.name)
            .count();
        if self.prompts.len() >= MAX_PROMPTS || theirs >= MAX_PROMPTS_PER_CLIENT {
            return Err(format!(
                "requests already wait for the person to answer in the 3DCraft window ({theirs} of yours, at most {MAX_PROMPTS_PER_CLIENT}); wait for those first"
            ));
        }
        for want in wanted {
            let resolved = if want.write {
                self.current.resolve_write(&want.path)
            } else {
                self.current.resolve_read(&want.path)
            };
            if resolved.is_ok() {
                continue;
            }
            // A name that could read as something else in the dialog (a
            // line break, a direction override) is never asked about.
            if want.path.chars().any(crate::consent::deceptive) {
                return Err(format!(
                    "{}: the path holds a control, direction or invisible character; 3DCraft does not ask the person about such a path (rename it)",
                    want.command
                ));
            }
            let Some(folder) = crate::consent::folder_of(want) else {
                continue;
            };
            if let Some(why) = self.protected.reason(&folder) {
                return Err(format!(
                    "{}: 3DCraft never gives an agent `{}`: {why}",
                    want.command, want.path
                ));
            }
            let file_name = Path::new(&want.path)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let id = self.push_prompt(AgentPrompt {
                id: 0,
                kind: if want.write { "write" } else { "read" }.to_string(),
                client: origin.name.clone(),
                path: want.path.clone(),
                folder: folder.to_string_lossy().into_owned(),
                file_name,
                command: Some(want.command.clone()),
            });
            return Ok(Some(id));
        }
        Ok(None)
    }

    /// The person answered prompt `id`; what it means for the request
    /// that waited on it (none for a connect question).
    pub fn answer(&mut self, id: u64, answer: PromptAnswer) -> Option<Verdict> {
        let index = self.prompts.iter().position(|p| p.prompt.id == id)?;
        let pending = self.prompts.remove(index);
        let key = pending.key;
        let prompt = pending.prompt;
        let folder = PathBuf::from(&prompt.folder);
        let connect = prompt.kind == "connect";
        match answer {
            PromptAnswer::Folder => {
                if !self.preferences.folders.contains(&folder) {
                    self.preferences.folders.push(folder);
                    self.remember();
                }
            }
            PromptAnswer::Session => {
                if !self.session_folders.contains(&folder) {
                    self.session_folders.push(folder);
                }
            }
            // Once: the file for that request only, given to it alone by
            // the desk (`Verdict::Once`); nothing here changes.
            PromptAnswer::Once | PromptAnswer::Deny => {}
            // Refuse at connect: that agent is disconnected and refused
            // for the rest of the run (decision of 2026-11-04); its
            // other questions go with it.
            PromptAnswer::Refuse => {
                if connect && let Some(key) = &key {
                    self.disconnect(key);
                }
            }
        }
        self.recompute();
        if connect {
            return None;
        }
        Some(match answer {
            PromptAnswer::Deny | PromptAnswer::Refuse => Verdict::Refuse(denied(&prompt)),
            PromptAnswer::Once => Verdict::Once {
                file: PathBuf::from(&prompt.path),
                write: prompt.kind == "write",
            },
            _ => Verdict::Run,
        })
    }

    /// The prompts asked before `now - wait`: taken off, with the reason
    /// for their requests. A connect question simply goes.
    pub fn expire(&mut self, now: Instant) -> Vec<(u64, String)> {
        let mut gone = Vec::new();
        self.prompts.retain(|pending| {
            let waited = now.saturating_duration_since(pending.asked);
            if pending.prompt.kind == "connect" {
                // A connect question nobody answered goes quietly.
                return waited < CONNECT_WAIT;
            }
            let late = waited >= crate::consent::PROMPT_WAIT;
            if late {
                gone.push((pending.prompt.id, denied(&pending.prompt)));
            }
            !late
        });
        gone
    }

    /// When the oldest command prompt times out, if any.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.prompts
            .iter()
            .filter(|p| p.prompt.kind != "connect")
            .map(|p| p.asked + crate::consent::PROMPT_WAIT)
            .min()
    }

    /// The person took a folder back (Remove).
    pub fn revoke(&mut self, folder: &Path) {
        let before = self.preferences.folders.len();
        self.preferences.folders.retain(|f| f != folder);
        if self.preferences.folders.len() != before {
            self.remember();
        }
        self.session_folders.retain(|f| f != folder);
        self.recompute();
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
            .and_then(|folder| {
                let real = usable_folder(&folder)?;
                match self.protected.reason(&real) {
                    Some(why) => Err(format!("3DCraft never gives agents this folder: {why}")),
                    None => Ok(folder),
                }
            });
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
                        let usable = usable_folder(&folder).and_then(|real| {
                            match self.protected.reason(&real) {
                                Some(why) => {
                                    Err(format!("3DCraft never gives agents this folder: {why}"))
                                }
                                None => Ok(real),
                            }
                        });
                        match usable {
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
            for (folders, from) in [
                (self.preferences.folders.clone(), "allowed"),
                (self.session_folders.clone(), "session"),
            ] {
                for folder in folders {
                    if !folder.is_dir()
                        || roots.iter().any(|r| Path::new(&r.path) == folder)
                        || self.protected.reason(&folder).is_some()
                    {
                        continue;
                    }
                    roots.push(entry(&folder, from, true, true));
                    read.push(folder.clone());
                    write.push(folder);
                }
            }
            if self.preferences.document_folder
                && let Some(document) = &self.document
                && self.protected.reason(document).is_none()
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

    /// Closes the connection of the agent that authenticated as `key` and
    /// refuses it for the rest of the run (its ✕ in the agent panel, or
    /// Refuse when it connects); the questions it still had wait no
    /// longer. Whether it was known.
    pub fn disconnect(&mut self, key: &str) -> bool {
        self.prompts.retain(|p| p.key.as_deref() != Some(key));
        self.server
            .as_ref()
            .is_some_and(|server| server.disconnect(key))
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

    /// Something went wrong around the switch that the person should see
    /// (the folder picker could not be shown).
    pub fn note_error(&mut self, error: String) {
        self.error = Some(error);
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
                    cwd: client.cwd.map(|cwd| cwd.to_string_lossy().into_owned()),
                    key: client.key,
                })
                .collect(),
            work_folder: self
                .granted
                .as_ref()
                .map(|folder| folder.to_string_lossy().into_owned()),
            command_line: self.config.command_line.is_some(),
            document_folder: self.preferences.document_folder,
            roots: self.roots.clone(),
            prompts: self.prompts.iter().map(|p| p.prompt.clone()).collect(),
            error: self.error.clone(),
            home: self.config.home.clone(),
        }
    }
}

/// What a refused or unanswered request is told.
fn denied(prompt: &AgentPrompt) -> String {
    format!(
        "{}: the person has not given access to `{}`; ask them to confirm in the 3DCraft window (or to allow its folder in the agents panel)",
        prompt.command.as_deref().unwrap_or("file access"),
        prompt.path
    )
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
            protected: Protected::default(),
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
        let mut config = config(&state.join("craft"));
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
        let mut config = config(&state.join("craft"));
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
        assert!(!discovery::read_preferences(&state.join("craft")).document_folder);
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
        let (mut agents, _pump) = agents(config(&state.join("craft")));
        agents.allow(true);
        let mine = state.join("mine").join("agents");
        agents.choose_folder(&mine).unwrap();
        assert!(mine.is_dir(), "made when missing");
        assert_eq!(
            discovery::read_preferences(&state.join("craft")).folder,
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
        let (mut again, _pump) = super::tests::agents(config(&state.join("craft")));
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
        let mut config = config(&state.join("craft"));
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
        let mut config = config(&state.join("craft"));
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

    // ------------------------------------------------------------ consent

    /// A desk (no engine thread: requests are answered in its turns) whose
    /// switch is on, with the chosen folder in `state/Documents/3DCraft`.
    fn consent_desk(state: &Path) -> crate::app::Desk {
        use crate::app::Desk;
        use crate::files::{Picker, SystemDialogs};
        // 3DCraft's own folder beside the test's folders, not above them:
        // it is protected.
        let mut config = config(&state.join("craft"));
        config.work_folder = Some(state.join("Documents").join("3DCraft"));
        let (requests, incoming) = mpsc::channel();
        config.protected = crate::consent::Protected {
            home: Some(state.join("home")),
            ..crate::consent::Protected::default()
        };
        let agents = Agents::new(config, requests, Arc::new(|| {}));
        let picker = Picker::new(Arc::new(SystemDialogs), Arc::new(|| {}));
        let mut desk = Desk::new(Host::new(Session::new(Workspace::none())), incoming, picker)
            .with_agents(agents);
        desk.host.shell.request_agents(true);
        desk.turn();
        desk
    }

    /// A bridge that works in `cwd` and finds the desk's window.
    fn bridge_in(state: &Path, cwd: &Path) -> Bridge {
        let mut config = BridgeConfig::discover(state.join("craft"), None);
        config.cwd = Some(craft3d_engine::canonical(cwd).unwrap());
        // A name of its own per bridge, short enough to be one (80
        // characters at most).
        static BRIDGES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = BRIDGES.fetch_add(1, Ordering::Relaxed);
        config.client = Some(format!("MCP bridge (test {n})"));
        Bridge::new(config)
    }

    /// Sends `message` through `bridge` while the desk takes its turns;
    /// `until` is turned in between (to answer a prompt), and returns true
    /// once it did what it waited for.
    fn relay_turning(
        desk: &mut crate::app::Desk,
        bridge: Bridge,
        message: Value,
        mut until: impl FnMut(&mut crate::app::Desk) -> bool,
    ) -> (Bridge, Value) {
        let thread = std::thread::spawn(move || {
            let mut bridge = bridge;
            let reply = bridge::relay(&mut bridge, &message.to_string());
            (bridge, reply)
        });
        let start = Instant::now();
        let mut done = false;
        while !thread.is_finished() {
            assert!(start.elapsed() < Duration::from_secs(60), "no reply");
            desk.turn();
            if !done {
                done = until(desk);
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let (bridge, reply) = thread.join().unwrap();
        (bridge, serde_json::from_str(&reply.unwrap()).unwrap())
    }

    fn initialize() -> Value {
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                       "clientInfo": {"name": "Claude Code", "version": "2"}}})
    }

    fn run(id: u64, command: &str, params: Value) -> Value {
        json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
               "params": {"name": "command_run", "arguments": {"id": command, "params": params}}})
    }

    fn prompts(desk: &crate::app::Desk) -> Vec<AgentPrompt> {
        desk.host.shell.agents_status().prompts.clone()
    }

    /// Answers the first prompt of `kind` as the person's click does.
    fn click(desk: &mut crate::app::Desk, kind: &str, answer: PromptAnswer) -> bool {
        let Some(prompt) = prompts(desk).into_iter().find(|p| p.kind == kind) else {
            return false;
        };
        desk.host.shell.answer_agent_prompt(prompt.id, answer);
        true
    }

    fn tool_text(reply: &Value) -> String {
        reply["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    }

    /// The developer's scenario: Claude Code opened in a project folder
    /// connects; the person allows that folder; the agent makes a new
    /// document with a 10 mm cube and saves it there.
    #[test]
    fn an_agent_saves_a_new_cube_in_the_folder_it_works_in_once_allowed() {
        let state = workdir("consent-scenario");
        let project = state.join("home").join("projects").join("bracket");
        std::fs::create_dir_all(&project).unwrap();
        let project = craft3d_engine::canonical(&project).unwrap();
        let mut desk = consent_desk(&state);
        // Connecting: the bridge sends the folder; the person is asked.
        let (bridge, reply) =
            relay_turning(&mut desk, bridge_in(&state, &project), initialize(), |_| {
                true
            });
        assert_eq!(reply["result"]["serverInfo"]["name"], "craft3d", "{reply}");
        desk.turn();
        let status = desk.host.shell.agents_status().clone();
        assert_eq!(
            status.clients[0].cwd.as_deref(),
            project.to_str(),
            "the cwd came along"
        );
        let asked = prompts(&desk);
        assert_eq!(asked.len(), 1, "{asked:?}");
        assert_eq!(
            (
                asked[0].kind.as_str(),
                asked[0].client.as_str(),
                asked[0].path.as_str()
            ),
            ("connect", "Claude Code", project.to_str().unwrap())
        );
        assert!(click(&mut desk, "connect", PromptAnswer::Folder));
        desk.turn();
        assert!(prompts(&desk).is_empty());
        let roots = desk.host.shell.agents_status().roots.clone();
        assert!(
            roots
                .iter()
                .any(|r| r.from == "allowed" && r.path == project.to_str().unwrap()),
            "{roots:?}"
        );
        assert_eq!(
            discovery::read_preferences(&state.join("craft")).folders,
            std::slice::from_ref(&project),
            "remembered"
        );
        // New document, a 10 mm cube, saved in the project folder.
        let cube = project.join("cube.3dcraft");
        let mut bridge = bridge;
        for (id, command, params) in [
            (2, "file.new", json!({"units": "mm"})),
            (
                3,
                "feature.block",
                json!({"name": "Cube", "size": [10, 10, 10]}),
            ),
            (4, "file.save", json!({"path": cube.to_str().unwrap()})),
        ] {
            let (back, reply) =
                relay_turning(&mut desk, bridge, run(id, command, params), |_| true);
            bridge = back;
            assert_eq!(reply["result"]["isError"], false, "{command}: {reply}");
        }
        assert!(cube.is_file(), "saved in the client's folder");
        // The window's agent log has it, under the client's own name, with
        // the intent the agent gave.
        let intent = json!({"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {
            "name": "command_run", "arguments": {"id": "feature.cylinder",
                "params": {"name": "Peg", "position": [5, 5, 10], "radius": 1, "height": 4},
                "intent": "a peg to locate the lid"}}});
        let (back, reply) = relay_turning(&mut desk, bridge, intent, |_| true);
        bridge = back;
        assert_eq!(reply["result"]["isError"], false, "{reply}");
        let got = desk
            .host
            .shell
            .handle_method(&mut desk.host.session, "ui.get", None)
            .unwrap()
            .unwrap();
        let log = &got["agentLog"];
        assert_eq!(log["entries"][0]["client"], "Claude Code", "{log}");
        assert_eq!(
            log["entries"][0]["intent"], "a peg to locate the lid",
            "{log}"
        );
        assert_eq!(log["entries"][0]["created"], json!(["F2"]));
        assert_eq!(log["entries"][1]["command"], "file.save");
        assert_eq!(log["features"][0]["madeBy"], "Claude Code");
        let _ = bridge;
        let mut check = Session::new(Workspace::unrestricted());
        check
            .execute("file.open", &json!({"path": cube.to_str().unwrap()}))
            .unwrap();
        let volume = check.execute("document.measure", &json!({})).unwrap()["volume"]
            .as_f64()
            .unwrap();
        assert!((volume - 1000.0).abs() < 1e-6, "{volume}");
        assert!(prompts(&desk).is_empty(), "no prompt for a folder allowed");
        let _ = std::fs::remove_dir_all(&state);
    }

    /// The connect question: This session only (not remembered) and No.
    #[test]
    fn the_connect_question_for_this_session_or_not_at_all() {
        let state = workdir("consent-connect");
        let (a, b) = (state.join("work").join("a"), state.join("work").join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let mut desk = consent_desk(&state);
        let _ = relay_turning(&mut desk, bridge_in(&state, &a), initialize(), |_| true);
        desk.turn();
        assert!(click(&mut desk, "connect", PromptAnswer::Session));
        desk.turn();
        let a = craft3d_engine::canonical(&a).unwrap();
        assert!(
            desk.host
                .shell
                .agents_status()
                .roots
                .iter()
                .any(|r| r.from == "session" && Path::new(&r.path) == a)
        );
        assert!(
            discovery::read_preferences(&state.join("craft"))
                .folders
                .is_empty(),
            "not remembered"
        );
        // Asked once per client: another message asks nothing.
        let mut second = bridge_in(&state, &b);
        let (back, _) = relay_turning(&mut desk, second, initialize(), |_| true);
        second = back;
        desk.turn();
        assert!(click(&mut desk, "connect", PromptAnswer::Deny));
        desk.turn();
        let _ = relay_turning(&mut desk, second, initialize(), |_| true);
        desk.turn();
        assert!(prompts(&desk).is_empty(), "not asked again");
        let b = craft3d_engine::canonical(&b).unwrap();
        assert!(
            !desk
                .host
                .shell
                .agents_status()
                .roots
                .iter()
                .any(|r| Path::new(&r.path) == b)
        );
        // Remove takes a session folder back.
        desk.host.shell.request_agents_revoke(a.to_str().unwrap());
        desk.turn();
        assert!(
            !desk
                .host
                .shell
                .agents_status()
                .roots
                .iter()
                .any(|r| Path::new(&r.path) == a)
        );
        let _ = std::fs::remove_dir_all(&state);
    }

    /// The connect question's two refusals (decision of 2026-11-04):
    /// "Only the work folder" keeps the agent connected with the work
    /// folder alone; "Refuse" disconnects that agent and refuses it for
    /// the rest of the run, while another agent keeps working.
    #[test]
    fn refuse_at_connect_disconnects_that_agent_only_the_work_folder_keeps_it() {
        let state = workdir("consent-refuse");
        let (a, b) = (state.join("work").join("a"), state.join("work").join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let mut desk = consent_desk(&state);
        // A: only the work folder. It stays connected and works.
        let (mut first, _) =
            relay_turning(&mut desk, bridge_in(&state, &a), initialize(), |_| true);
        desk.turn();
        assert!(click(&mut desk, "connect", PromptAnswer::Deny));
        desk.turn();
        let a_canon = craft3d_engine::canonical(&a).unwrap();
        let status = desk.host.shell.agents_status().clone();
        assert!(
            !status.roots.iter().any(|r| Path::new(&r.path) == a_canon),
            "its folder is not given: {status:?}"
        );
        assert!(
            status.roots.iter().any(|r| r.from == "chosen"),
            "the work folder is: {status:?}"
        );
        let (back, reply) = relay_turning(
            &mut desk,
            first,
            run(2, "file.new", json!({"units": "mm"})),
            |_| true,
        );
        first = back;
        assert_eq!(
            reply["result"]["isError"], false,
            "still connected: {reply}"
        );
        // B: Refuse. Its connection is closed and it is refused from then
        // on, by its name.
        let (second, _) = relay_turning(&mut desk, bridge_in(&state, &b), initialize(), |_| true);
        desk.turn();
        assert!(click(&mut desk, "connect", PromptAnswer::Refuse));
        desk.turn();
        let (_, refused) = relay_turning(&mut desk, second, initialize(), |_| true);
        let message = refused["error"]["message"].as_str().unwrap_or_default();
        assert!(message.contains("disconnected"), "{refused}");
        let b_canon = craft3d_engine::canonical(&b).unwrap();
        assert!(
            !desk
                .host
                .shell
                .agents_status()
                .roots
                .iter()
                .any(|r| Path::new(&r.path) == b_canon)
        );
        // A goes on working.
        let (_, reply) = relay_turning(
            &mut desk,
            first,
            run(3, "feature.block", json!({"size": [5, 5, 5]})),
            |_| true,
        );
        assert_eq!(reply["result"]["isError"], false, "{reply}");
        let _ = std::fs::remove_dir_all(&state);
    }

    /// The ✕ of one agent in the agent panel disconnects that one and
    /// refuses it for the rest of the run; the other stays.
    #[test]
    fn one_agent_is_disconnected_by_its_cross_and_the_other_stays() {
        let state = workdir("consent-cross");
        let work = state.join("Documents").join("3DCraft");
        std::fs::create_dir_all(&work).unwrap();
        let mut desk = consent_desk(&state);
        let named = |name: &str| {
            let mut config = BridgeConfig::discover(state.join("craft"), None);
            config.cwd = Some(craft3d_engine::canonical(&work).unwrap());
            config.client = Some(name.to_string());
            Bridge::new(config)
        };
        // Both work in the work folder: nothing is asked.
        let (gone, _) = relay_turning(
            &mut desk,
            named("MCP bridge (cross a)"),
            initialize(),
            |_| true,
        );
        let (stays, _) = relay_turning(
            &mut desk,
            named("MCP bridge (cross b)"),
            initialize(),
            |_| true,
        );
        desk.turn();
        let clients = desk.host.shell.agents_status().clients.clone();
        assert_eq!(clients.len(), 2, "{clients:?}");
        assert!(
            clients
                .iter()
                .any(|c| c.key.as_deref() == Some("MCP bridge (cross a)")),
            "{clients:?}"
        );
        desk.host
            .shell
            .request_agent_disconnect("MCP bridge (cross a)");
        desk.turn();
        let left = desk.host.shell.agents_status().clients.clone();
        assert_eq!(left.len(), 1, "{left:?}");
        assert_eq!(left[0].key.as_deref(), Some("MCP bridge (cross b)"));
        // The one sent away is refused; the other works.
        let (_, refused) = relay_turning(&mut desk, gone, initialize(), |_| true);
        let message = refused["error"]["message"].as_str().unwrap_or_default();
        assert!(message.contains("disconnected"), "{refused}");
        let (_, reply) = relay_turning(&mut desk, stays, run(2, "file.new", json!({})), |_| true);
        assert_eq!(reply["result"]["isError"], false, "{reply}");
        let _ = std::fs::remove_dir_all(&state);
    }

    /// A path outside every root waits for the person: Allow once runs
    /// that command only, Allow this folder from then on, Deny and a
    /// timeout refuse with what to do; the window answers other requests
    /// meanwhile.
    #[test]
    fn a_path_outside_every_root_waits_for_the_person() {
        let state = workdir("consent-path");
        let outside = state.join("elsewhere");
        std::fs::create_dir_all(&outside).unwrap();
        let outside = craft3d_engine::canonical(&outside).unwrap();
        let mut desk = consent_desk(&state);
        // Works in the chosen folder: nothing to ask about at connect.
        let chosen = state.join("Documents").join("3DCraft");
        let (mut bridge, _) =
            relay_turning(&mut desk, bridge_in(&state, &chosen), initialize(), |_| {
                true
            });
        desk.turn();
        assert!(prompts(&desk).is_empty());
        let (back, _) = relay_turning(&mut desk, bridge, run(2, "file.new", json!({})), |_| true);
        bridge = back;
        let (back, _) = relay_turning(
            &mut desk,
            bridge,
            run(20, "feature.block", json!({"size": [10, 10, 10]})),
            |_| true,
        );
        bridge = back;
        // Exports: a save would make `elsewhere` the open document's
        // folder, a root of its own.
        let save = |id: u64, name: &str| {
            run(
                id,
                "file.export",
                json!({"path": outside.join(name).to_str().unwrap()}),
            )
        };
        // Allow once: this save only. Meanwhile the window answers.
        let (back, reply) = relay_turning(&mut desk, bridge, save(3, "once.stl"), |desk| {
            let asked = prompts(desk);
            if asked.is_empty() {
                return false;
            }
            assert_eq!(asked[0].kind, "write");
            assert_eq!(
                asked[0].path,
                outside.join("once.stl").to_str().unwrap(),
                "the full path"
            );
            assert_eq!(asked[0].command.as_deref(), Some("file.export"));
            click(desk, "write", PromptAnswer::Once)
        });
        bridge = back;
        assert_eq!(reply["result"]["isError"], false, "{reply}");
        assert!(outside.join("once.stl").is_file());
        desk.turn();
        assert!(
            !desk
                .host
                .shell
                .agents_status()
                .roots
                .iter()
                .any(|r| r.from == "once"),
            "taken back"
        );
        // Deny: a tool error that says what to do.
        let (back, reply) = relay_turning(&mut desk, bridge, save(4, "denied.stl"), |desk| {
            click(desk, "write", PromptAnswer::Deny)
        });
        bridge = back;
        assert_eq!(reply["result"]["isError"], true, "{reply}");
        let text = tool_text(&reply);
        assert!(
            text.contains("has not given access") && text.contains("3DCraft window"),
            "{text}"
        );
        assert!(!outside.join("denied.stl").exists());
        // Nobody answers: refused after the wait (an injected clock).
        let (back, reply) = relay_turning(&mut desk, bridge, save(5, "late.stl"), |desk| {
            if prompts(desk).is_empty() {
                return false;
            }
            desk.expire_prompts(
                Instant::now() + crate::consent::PROMPT_WAIT + Duration::from_secs(1),
            );
            true
        });
        bridge = back;
        assert!(
            tool_text(&reply).contains("has not given access"),
            "{reply}"
        );
        // Allow this folder: from then on without asking.
        let (back, reply) = relay_turning(&mut desk, bridge, save(6, "folder.stl"), |desk| {
            click(desk, "write", PromptAnswer::Folder)
        });
        bridge = back;
        assert_eq!(reply["result"]["isError"], false, "{reply}");
        let (_, reply) = relay_turning(&mut desk, bridge, save(7, "again.stl"), |desk| {
            assert!(prompts(desk).is_empty(), "asked again");
            false
        });
        assert_eq!(reply["result"]["isError"], false, "{reply}");
        assert!(outside.join("again.stl").is_file());
        let _ = std::fs::remove_dir_all(&state);
    }

    /// Protected places are refused at once, never asked about; a path
    /// through a link or with `..` is not offered either.
    #[test]
    fn protected_places_and_escapes_are_refused_without_asking() {
        let state = workdir("consent-protected");
        let home = state.join("home");
        std::fs::create_dir_all(home.join(".ssh")).unwrap();
        std::fs::create_dir_all(state.join("outside")).unwrap();
        let mut desk = consent_desk(&state);
        let chosen = state.join("Documents").join("3DCraft");
        let (mut bridge, _) =
            relay_turning(&mut desk, bridge_in(&state, &chosen), initialize(), |_| {
                true
            });
        let (back, _) = relay_turning(&mut desk, bridge, run(2, "file.new", json!({})), |_| true);
        bridge = back;
        let home = craft3d_engine::canonical(&home).unwrap();
        let state_real = craft3d_engine::canonical(&state).unwrap();
        let mut bad: Vec<String> = vec![
            home.join("x.3dcraft").to_str().unwrap().to_string(),
            home.join(".ssh")
                .join("x.3dcraft")
                .to_str()
                .unwrap()
                .to_string(),
            state_real
                .join("windows-x.3dcraft")
                .to_str()
                .unwrap()
                .to_string(),
        ];
        #[cfg(unix)]
        {
            let chosen = craft3d_engine::canonical(&chosen).unwrap();
            std::os::unix::fs::symlink(state_real.join("outside"), chosen.join("out")).unwrap();
            bad.push(
                chosen
                    .join("out")
                    .join("x.3dcraft")
                    .to_str()
                    .unwrap()
                    .to_string(),
            );
            bad.push(format!("{}/../outside/x.3dcraft", chosen.display()));
        }
        for (index, path) in bad.iter().enumerate() {
            let id = 10 + index as u64;
            let (back, reply) = relay_turning(
                &mut desk,
                bridge,
                run(id, "file.save", json!({"path": path})),
                |desk| {
                    assert!(prompts(desk).is_empty(), "{path}: asked");
                    false
                },
            );
            bridge = back;
            assert_eq!(reply["result"]["isError"], true, "{path}: {reply}");
        }
        assert!(!home.join("x.3dcraft").exists());
        let _ = std::fs::remove_dir_all(&state);
    }

    /// An agent cannot answer its own prompt: no ui.set or other method
    /// of the control channel or of MCP reaches it.
    #[test]
    fn an_agent_cannot_approve_its_own_request() {
        let state = workdir("consent-self");
        let outside = state.join("elsewhere");
        std::fs::create_dir_all(&outside).unwrap();
        let outside = craft3d_engine::canonical(&outside).unwrap();
        let mut desk = consent_desk(&state);
        let chosen = state.join("Documents").join("3DCraft");
        let (mut bridge, _) =
            relay_turning(&mut desk, bridge_in(&state, &chosen), initialize(), |_| {
                true
            });
        let (back, _) = relay_turning(&mut desk, bridge, run(2, "file.new", json!({})), |_| true);
        bridge = back;
        let target = outside.join("self.3dcraft");
        let mut tried = false;
        let control = |id: u64, method: &str, params: Value| {
            json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
                   "params": {"name": "control_call", "arguments": {"method": method, "params": params}}})
        };
        let (_, reply) = relay_turning(
            &mut desk,
            bridge,
            run(3, "file.save", json!({"path": target.to_str().unwrap()})),
            |desk| {
                let asked = prompts(desk);
                if asked.is_empty() || tried {
                    return false;
                }
                tried = true;
                let id = asked[0].id;
                // A second agent connection tries every way it has.
                let mut other = bridge_in(&state, &chosen);
                for (n, (method, params)) in [
                    ("ui.set", json!({"agents": {"prompts": []}})),
                    ("ui.set", json!({"prompts": [{"id": id, "answer": "folder"}]})),
                    ("ui.set", json!({"agents": {"allowed": true, "roots": [{"path": outside.to_str().unwrap()}]}})),
                    ("ui.set", json!({"dialogs": {"agents": true, "prompt": id}})),
                    ("ui.answer", json!({"id": id, "answer": "folder"})),
                ]
                .into_iter()
                .enumerate()
                {
                    let message = control(100 + n as u64, method, params);
                    let thread = std::thread::spawn({
                        let mut moved = std::mem::replace(&mut other, bridge_in(&state, &chosen));
                        move || bridge::relay(&mut moved, &message.to_string())
                    });
                    while !thread.is_finished() {
                        desk.turn();
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    let _ = thread.join();
                    desk.turn();
                }
                // Still asked, still unanswered; then the person says no.
                let still = prompts(desk);
                assert_eq!(still.len(), 1, "{still:?}");
                assert_eq!(still[0].id, id);
                assert!(
                    !desk
                        .host
                        .shell
                        .agents_status()
                        .roots
                        .iter()
                        .any(|r| Path::new(&r.path) == outside)
                );
                desk.host.shell.answer_agent_prompt(id, PromptAnswer::Deny);
                true
            },
        );
        assert!(tried);
        assert_eq!(reply["result"]["isError"], true, "{reply}");
        assert!(!target.exists());
        let _ = std::fs::remove_dir_all(&state);
    }

    /// The questions waiting for the person are bounded: one more request
    /// is refused at once, with what to do.
    #[test]
    fn a_flood_of_requests_is_refused_not_queued() {
        let state = workdir("consent-flood");
        let outside = state.join("elsewhere");
        std::fs::create_dir_all(&outside).unwrap();
        let mut config = config(&state.join("craft"));
        config.work_folder = Some(state.join("chosen"));
        let (mut agents, _pump) = agents(config);
        agents.allow(true);
        let origin = |n: usize| Origin {
            key: format!("k{n}"),
            name: format!("Agent {n}"),
            cwd: None,
            grant: None,
        };
        let want = |n: usize| Wanted {
            command: "file.export".to_string(),
            path: outside
                .join(format!("{n}.stl"))
                .to_str()
                .unwrap()
                .to_string(),
            write: true,
            folder: false,
        };
        // One client: at most MAX_PROMPTS_PER_CLIENT.
        for n in 0..MAX_PROMPTS_PER_CLIENT {
            assert!(
                matches!(agents.screen(&origin(0), &[want(n)]), Ok(Some(_))),
                "{n}"
            );
        }
        let error = agents.screen(&origin(0), &[want(50)]).unwrap_err();
        assert!(error.contains("wait for the person"), "{error}");
        // All clients: at most MAX_PROMPTS.
        let mut asked = MAX_PROMPTS_PER_CLIENT;
        let mut client = 1;
        while asked < MAX_PROMPTS {
            assert!(matches!(
                agents.screen(&origin(client), &[want(100 + asked)]),
                Ok(Some(_))
            ));
            asked += 1;
            if asked.is_multiple_of(MAX_PROMPTS_PER_CLIENT) {
                client += 1;
            }
        }
        let error = agents.screen(&origin(99), &[want(999)]).unwrap_err();
        assert!(error.contains("wait for the person"), "{error}");
        assert_eq!(agents.status().prompts.len(), MAX_PROMPTS);
        let _ = std::fs::remove_dir_all(&state);
    }

    /// M1 (security review, 2026-11-02): the window built by `app::control`
    /// protects the home folder, its keys and settings and its own state
    /// folder, from the environment.
    #[test]
    fn the_window_protects_home_keys_and_its_own_folder() {
        let root = workdir("protected-wired");
        let home = root.join("home");
        std::fs::create_dir_all(home.join(".ssh")).unwrap();
        std::fs::create_dir_all(home.join("projects")).unwrap();
        let state = root.join("state");
        std::fs::create_dir_all(&state).unwrap();
        let options = crate::args::Options {
            state_dir: Some(state.clone()),
            agent_port: Some(0),
            ..crate::args::Options::default()
        };
        let text = home.to_str().unwrap().to_string();
        let env = move |name: &str| (name == "HOME" || name == "USERPROFILE").then(|| text.clone());
        let (mut agents, _incoming) = crate::app::control(&options, Arc::new(|| {}), &env).unwrap();
        agents.allow(true);
        let origin = Origin {
            key: "k".to_string(),
            name: "Claude Code".to_string(),
            cwd: None,
            grant: None,
        };
        let want = |path: PathBuf| Wanted {
            command: "file.export".to_string(),
            path: path.to_str().unwrap().to_string(),
            write: true,
            folder: false,
        };
        for refused in [
            home.join(".ssh").join("authorized_keys"),
            home.join(".bashrc"),
            state.join("x.stl"),
        ] {
            let error = agents
                .screen(&origin, &[want(refused.clone())])
                .unwrap_err();
            assert!(error.contains("never gives"), "{refused:?}: {error}");
        }
        assert!(matches!(
            agents.screen(&origin, &[want(home.join("projects").join("x.stl"))]),
            Ok(Some(_))
        ));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// M5: a path that could read as something else in the dialog is
    /// refused, never asked about.
    #[test]
    fn a_path_with_a_line_break_or_a_direction_mark_is_refused() {
        let state = workdir("deceptive");
        let outside = state.join("elsewhere");
        std::fs::create_dir_all(&outside).unwrap();
        let mut config = config(&state.join("craft"));
        config.work_folder = Some(state.join("chosen"));
        let (mut agents, _pump) = agents(config);
        agents.allow(true);
        let origin = Origin {
            key: "k".to_string(),
            name: "Claude Code".to_string(),
            cwd: None,
            grant: None,
        };
        for name in [
            "a\nAllow this: /home/me.stl",
            "safe\u{202E}lts.exe",
            "x\u{200B}.stl",
        ] {
            let want = Wanted {
                command: "file.export".to_string(),
                path: outside.join(name).to_str().unwrap().to_string(),
                write: true,
                folder: false,
            };
            let error = agents.screen(&origin, &[want]).unwrap_err();
            assert!(error.contains("does not ask"), "{name:?}: {error}");
        }
        assert!(agents.status().prompts.is_empty());
        let _ = std::fs::remove_dir_all(&state);
    }

    /// M2: the open document's folder is a root only for a file the person
    /// opened themselves and made active; one the agent opened, or made
    /// active, gives nothing.
    #[test]
    fn an_agent_cannot_steer_the_document_folder() {
        let state = workdir("doc-steer");
        let chosen = state.join("Documents").join("3DCraft");
        let (mut desk, one, two) = desk_with_two_documents(&state, &chosen);
        let has = |desk: &crate::app::Desk, dir: &Path| {
            desk.host
                .shell
                .agents_status()
                .roots
                .iter()
                .any(|r| r.from == "document" && Path::new(&r.path) == dir)
        };
        assert!(has(&desk, &two), "the person's own document");
        // An agent makes the other document active: no document folder.
        desk.host.session.set_actor(Some("Claude Code"));
        desk.host
            .session
            .execute("file.activate", &json!({"document": 0}))
            .unwrap();
        desk.host.session.set_actor(None);
        desk.turn();
        assert!(
            !has(&desk, &one) && !has(&desk, &two),
            "made active by an agent"
        );
        // The person clicks the tab: it is theirs again.
        desk.host
            .session
            .execute("file.activate", &json!({"document": 0}))
            .unwrap();
        desk.turn();
        assert!(has(&desk, &one));
        // An agent opens a file itself: nothing.
        let third = state.join("three");
        std::fs::create_dir_all(&third).unwrap();
        let copy = third.join("part.3dcraft");
        std::fs::copy(one.join("part.3dcraft"), &copy).unwrap();
        desk.host.session.set_workspace(Workspace::unrestricted());
        desk.host.session.set_actor(Some("Claude Code"));
        desk.host
            .session
            .execute("file.open", &json!({"path": copy.to_str().unwrap()}))
            .unwrap();
        desk.host.session.set_actor(None);
        desk.turn();
        let third = craft3d_engine::canonical(&third).unwrap();
        assert!(!has(&desk, &third), "opened by an agent");
        // Clicking its tab later does not make it the person's either.
        desk.host
            .session
            .execute("file.activate", &json!({"document": 0}))
            .unwrap();
        desk.host
            .session
            .execute("file.activate", &json!({"document": 2}))
            .unwrap();
        desk.turn();
        assert!(!has(&desk, &third), "an agent's file stays the agent's");
        let _ = std::fs::remove_dir_all(&state);
    }
}
