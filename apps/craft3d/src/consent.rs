//! Asking the person before an agent reaches a folder (2026-11-02,
//! docs/control-protocol.md, "Asking the person").
//!
//! Two questions: when a client connects, whether it may read and write
//! files in the folder it works in (its MCP client's working folder, which
//! the bridge sends in `auth`); and when a command wants a path outside
//! every root, whether it may have that one path, or its folder. Only the
//! person answers, with a click in the window.
//!
//! What is never offered, whatever is asked: the file system's root, the
//! home folder itself or a folder that holds it, the places where secrets
//! and the system live, and 3DCraft's own state folder (the control token
//! is there). [`Protected::reason`] is that list, in one place.
//!
//! This module is the pure part: the list, and the paths a request line
//! names. The desk holds the requests that wait for an answer.

use serde_json::Value;
use std::path::{Component, Path, PathBuf};

/// Who sent a request: the client's name in `auth` (the key), the name its
/// MCP client gave, and the folder it works in.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Origin {
    pub key: String,
    pub name: String,
    pub cwd: Option<PathBuf>,
    /// A file the person allowed once for this request alone, and whether
    /// for writing (`Workspace::with_file`).
    pub grant: Option<(PathBuf, bool)>,
}

/// Whether `c` is a control character, a direction mark or override, or
/// an invisible format character (Unicode's Cf, the common ones): what
/// could make a path or a name read as something else in a dialog.
pub fn deceptive(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00AD}'
                | '\u{061C}'
                | '\u{180E}'
                | '\u{200B}'..='\u{200F}'
                | '\u{2028}'..='\u{202E}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{206F}'
                | '\u{FEFF}'
                | '\u{FFF9}'..='\u{FFFB}'
                | '\u{E0001}'
                | '\u{E0020}'..='\u{E007F}'
        )
}

/// How long a command waits for the person's answer.
pub const PROMPT_WAIT: std::time::Duration = std::time::Duration::from_secs(120);

/// Folders beneath the home folder that hold secrets, settings, or
/// programs that run (a file planted there would run). `Applications`:
/// macOS's per-user programs, and on Linux the AppImages the launcher
/// keeps there; `bin`: on the `PATH` through Debian's and Ubuntu's
/// `.profile`.
const HOME_SECRETS: [&str; 25] = [
    ".ssh",
    ".gnupg",
    ".config",
    ".local/share/keyrings",
    ".local/bin",
    ".local/share/applications",
    ".aws",
    ".azure",
    ".kube",
    ".docker",
    ".password-store",
    ".mozilla",
    ".thunderbird",
    ".mozilla-thunderbird",
    ".icedove",
    ".evolution",
    ".local/share/evolution",
    ".cargo",
    ".rustup",
    ".claude",
    ".var/app",
    "snap",
    "Library",
    "Applications",
    "bin",
];

/// Folders of the system. On macOS `/etc`, `/var` and `/tmp` are links
/// into `/private` ([`Protected::reason`] follows them); `/Applications`
/// is writable by every administrator, and Homebrew's `/opt/homebrew`
/// (its `bin` on the `PATH`) by the person who installed it, as is its
/// twin on Linux, `/home/linuxbrew/.linuxbrew`. `/run/user` holds each
/// person's runtime folder (`$XDG_RUNTIME_DIR`), with the user units the
/// service manager starts.
const SYSTEM: [&str; 22] = [
    "/etc",
    "/usr",
    "/var",
    "/bin",
    "/sbin",
    "/boot",
    "/dev",
    "/proc",
    "/sys",
    "/lib",
    "/lib64",
    "/root",
    "/run/user",
    "/System",
    "/Library",
    "/private",
    "/Applications",
    "/opt/homebrew",
    "/home/linuxbrew/.linuxbrew",
    r"C:\Windows",
    r"C:\Program Files",
    r"C:\ProgramData",
];

/// Why a place is protected, as the refusal says it.
const KEYS: &str = "a folder of keys and settings";
const SETTINGS: &str = "a folder of the system or of your settings";
const OF_THE_SYSTEM: &str = "a folder of the system";
const OWN: &str = "3DCraft's own folder, where its control token is";

/// The places never offered to an agent, made once (they are looked up
/// on the file system then, not at every question: [`Protected::reason`]
/// runs on the window's thread).
#[derive(Clone, Debug)]
pub struct Protected {
    home: Option<PathBuf>,
    /// 3DCraft's state folder (the control token lives there), as given
    /// and resolved.
    state: Vec<PathBuf>,
    /// The folders of keys and settings beneath the home folder and the
    /// places from the environment, each as written and resolved.
    places: Vec<(PathBuf, &'static str)>,
    /// [`SYSTEM`], each as written and resolved: for what lies outside
    /// the home folder.
    system: Vec<(PathBuf, &'static str)>,
    /// `~/Library`, as written and resolved: what the cloud folders are
    /// let out of.
    library: Vec<PathBuf>,
    /// `~/Library/CloudStorage` (one folder per provider beneath it) and
    /// iCloud Drive (`~/Library/Mobile Documents/com~apple~CloudDocs`), as
    /// written and resolved.
    cloud_storage: Vec<PathBuf>,
    icloud_drive: Vec<PathBuf>,
}

impl Default for Protected {
    /// No home folder, no state folder, nothing from the environment: the
    /// system's folders only.
    fn default() -> Self {
        Self::new(None, None, Vec::new())
    }
}

/// `path` with its links resolved: the deepest part of it that exists,
/// resolved, with the rest appended as written (as the engine's roots
/// do, `craft3d_engine::workspace`'s `leaves_root`). `None` when that part
/// cannot be resolved: a link that leads nowhere, a drive that is not
/// there.
fn resolved(path: &Path) -> Option<PathBuf> {
    let mut probe = path.to_path_buf();
    let mut rest = Vec::new();
    while std::fs::symlink_metadata(&probe).is_err() {
        rest.push(probe.file_name()?.to_os_string());
        if !probe.pop() {
            return None;
        }
    }
    let real = craft3d_engine::canonical(&probe).ok()?;
    Some(rest.iter().rev().fold(real, |path, part| path.join(part)))
}

/// `place` as written and, where it differs, resolved, so that a folder
/// spelled either way meets its own spelling (`/etc` and `/private/etc`
/// on macOS).
fn both_spellings(place: PathBuf) -> Vec<PathBuf> {
    match resolved(&place) {
        Some(real) if real != place => vec![place, real],
        _ => vec![place],
    }
}

/// Whether `path` as written has a `.` or `..` part (`Path::components`
/// drops a `.` inside a path, so the text is read).
fn has_dot_part(path: &Path) -> bool {
    path.to_string_lossy()
        .split(|c| c == '/' || (cfg!(windows) && c == '\\'))
        .any(|part| part == "." || part == "..")
}

impl Protected {
    /// The places for a home folder, 3DCraft's state folder and the places
    /// the environment names (all absolute; a relative one is left out).
    pub fn new(home: Option<PathBuf>, state: Option<PathBuf>, environment: Vec<PathBuf>) -> Self {
        let mut places = Vec::new();
        if let Some(home) = &home {
            for secret in HOME_SECRETS {
                for spelling in both_spellings(home.join(secret)) {
                    places.push((spelling, KEYS));
                }
            }
        }
        for place in environment.into_iter().filter(|p| p.is_absolute()) {
            for spelling in both_spellings(place) {
                places.push((spelling, SETTINGS));
            }
        }
        let mut system = Vec::new();
        for place in SYSTEM.iter().map(PathBuf::from).filter(|p| p.is_absolute()) {
            for spelling in both_spellings(place) {
                system.push((spelling, OF_THE_SYSTEM));
            }
        }
        let beneath_home = |part: &str| {
            home.as_ref()
                .map(|home| both_spellings(home.join(part)))
                .unwrap_or_default()
        };
        Self {
            library: beneath_home("Library"),
            cloud_storage: beneath_home("Library/CloudStorage"),
            icloud_drive: beneath_home("Library/Mobile Documents/com~apple~CloudDocs"),
            home,
            state: state.map(both_spellings).unwrap_or_default(),
            places,
            system,
        }
    }

    /// Whether `folder` is a cloud folder of macOS, or in one: a
    /// provider's folder beneath `~/Library/CloudStorage` (Dropbox,
    /// OneDrive, Google Drive, Box; not `CloudStorage` itself, the list of
    /// them) or iCloud Drive (not the other folders of
    /// `~/Library/Mobile Documents`, which are apps' own). The developer's
    /// decision of 2026-11-10: these may be a work folder although
    /// `~/Library` is protected.
    fn in_cloud(&self, folder: &Path) -> bool {
        self.cloud_storage.iter().any(|storage| {
            folder
                .strip_prefix(storage)
                .is_ok_and(|rest| rest.components().next().is_some())
        }) || self
            .icloud_drive
            .iter()
            .any(|drive| folder.starts_with(drive))
    }

    /// The same places with 3DCraft's state folder `state`.
    pub fn set_state(&mut self, state: &Path) {
        self.state = both_spellings(state.to_path_buf());
    }

    pub fn from_env(env: &dyn Fn(&str) -> Option<String>, state: Option<&Path>) -> Self {
        let var = |name: &str| {
            env(name)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
        };
        let home = if cfg!(windows) {
            var("USERPROFILE")
        } else {
            var("HOME")
        };
        let places = [
            "APPDATA",
            "LOCALAPPDATA",
            "SystemRoot",
            "ProgramFiles",
            "ProgramFiles(x86)",
            "XDG_CONFIG_HOME",
            "XDG_RUNTIME_DIR",
        ]
        .iter()
        .filter_map(|name| var(name))
        .collect();
        let real = |path: PathBuf| resolved(&path).unwrap_or(path);
        Self::new(
            home.map(real),
            state.map(|state| real(state.to_path_buf())),
            places,
        )
    }

    /// Why `folder` (absolute) is never offered to an agent; `None` when
    /// it may be. It is judged as it is written and with its links
    /// resolved (the deepest part that exists, the rest as written), and
    /// refused when either is protected: a folder written through a link
    /// would otherwise slip past the place it leads to (on macOS `/etc` is
    /// `/private/etc`; 2026-11-10, the weekly CI). A folder written with a
    /// `.` or `..` part, or through a link that leads nowhere, is refused.
    pub fn reason(&self, folder: &Path) -> Option<String> {
        let shown = folder.display();
        if let Some(why) = self.why(folder) {
            return Some(format!("`{shown}` {why}"));
        }
        if has_dot_part(folder) {
            return Some(format!(
                "`{shown}` has a `.` or `..` part; only a plainly written folder is offered"
            ));
        }
        let Some(real) = resolved(folder) else {
            return Some(format!(
                "`{shown}` goes through a link that leads nowhere, or a drive that is not there"
            ));
        };
        if real == folder {
            return None;
        }
        self.why(&real)
            .map(|why| format!("`{shown}` leads to `{}`, which {why}", real.display()))
    }

    /// [`Protected::reason`] for `folder` as it is written: what it is or
    /// holds ("is your home folder itself"), without the folder.
    fn why(&self, folder: &Path) -> Option<String> {
        // A network or device path (`\\server\share`, `\\?\C:\`,
        // `\\localhost\C$`): never offered, its identity cannot be told.
        let text = folder.to_string_lossy();
        if text.starts_with("\\\\") || text.starts_with("//") {
            return Some("is a network or device path".to_string());
        }
        if !folder.is_absolute() {
            return Some("is not an absolute path".to_string());
        }
        if folder.parent().is_none() {
            return Some("is the root of a drive or of the file system".to_string());
        }
        // A folder that is `place`, lies in it, or holds it (the place is
        // named unless it is the folder itself).
        let clash = |place: &Path, what: &str| -> Option<String> {
            if folder == place {
                Some(format!("is {what}"))
            } else if folder.starts_with(place) {
                Some(format!("is in {what} (`{}`)", place.display()))
            } else if place.starts_with(folder) {
                Some(format!("holds {what} (`{}`)", place.display()))
            } else {
                None
            }
        };
        if let Some(home) = &self.home {
            if folder == home {
                return Some("is your home folder itself".to_string());
            }
            if home.starts_with(folder) {
                return Some("holds your home folder".to_string());
            }
        }
        // (After the home folder itself: the state folder is in it.)
        if let Some(why) = self.state.iter().find_map(|state| clash(state, OWN)) {
            return Some(why);
        }
        // A cloud folder is let out of `~/Library` alone; where a link in
        // it leads is judged again, resolved.
        let cloud = self.in_cloud(folder);
        if let Some(why) = self
            .places
            .iter()
            .filter(|(place, _)| !(cloud && self.library.contains(place)))
            .find_map(|(place, what)| clash(place, what))
        {
            return Some(why);
        }
        // System folders only for what lies outside the home folder: some
        // systems keep homes beneath one of them (`/var/home`).
        let in_home = self
            .home
            .as_ref()
            .is_some_and(|home| folder.starts_with(home));
        if in_home {
            return None;
        }
        self.system
            .iter()
            .find_map(|(place, what)| clash(place, what))
    }
}

/// A path a request names, and whether the command writes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wanted {
    pub command: String,
    pub path: String,
    pub write: bool,
    /// The path names a folder (`view.replay`'s `dir`), not a file.
    pub folder: bool,
}

/// The commands that read the file they name; the others write it.
const READS: [&str; 4] = ["file.open", "file.import", "asm.add", "asm.insert"];

fn wanted_in(command: &str, params: Option<&Value>, out: &mut Vec<Wanted>) {
    let Some(Value::Object(params)) = params else {
        return;
    };
    for (key, folder) in [("path", false), ("dir", true)] {
        if let Some(path) = params.get(key).and_then(Value::as_str) {
            out.push(Wanted {
                command: command.to_string(),
                path: path.to_string(),
                write: !READS.contains(&command),
                folder,
            });
        }
    }
}

/// The paths a request line names: those of `engine.execute`, `batch`,
/// `ui.screenshot`, and of the MCP tools `command_run`, `command_batch` and
/// `control_call` inside `mcp.message`.
pub fn wanted(line: &str) -> Vec<Wanted> {
    let mut out = Vec::new();
    let Ok(request) = serde_json::from_str::<Value>(line) else {
        return out;
    };
    let params = request.get("params");
    let steps = |steps: Option<&Value>, key: &str, out: &mut Vec<Wanted>| {
        for step in steps.and_then(Value::as_array).into_iter().flatten() {
            if let Some(command) = step.get(key).and_then(Value::as_str) {
                wanted_in(command, step.get("params"), out);
            }
        }
    };
    match request.get("method").and_then(Value::as_str) {
        Some("engine.execute") => {
            if let Some(command) = params
                .and_then(|p| p.get("command"))
                .and_then(Value::as_str)
            {
                wanted_in(command, params.and_then(|p| p.get("params")), &mut out);
            }
        }
        Some("batch") => steps(params.and_then(|p| p.get("steps")), "command", &mut out),
        Some("ui.screenshot") => wanted_in("ui.screenshot", params, &mut out),
        Some("mcp.message") => {
            let message = params
                .and_then(|p| p.get("line"))
                .and_then(Value::as_str)
                .and_then(|line| serde_json::from_str::<Value>(line).ok());
            let Some(message) = message else {
                return out;
            };
            if message.get("method").and_then(Value::as_str) != Some("tools/call") {
                return out;
            }
            let inner = message.get("params");
            let arguments = inner.and_then(|p| p.get("arguments"));
            match inner.and_then(|p| p.get("name")).and_then(Value::as_str) {
                Some("command_run") => {
                    if let Some(id) = arguments.and_then(|a| a.get("id")).and_then(Value::as_str) {
                        wanted_in(id, arguments.and_then(|a| a.get("params")), &mut out);
                    }
                }
                Some("command_batch") => {
                    steps(arguments.and_then(|a| a.get("steps")), "id", &mut out)
                }
                Some("control_call") => {
                    if let Some(method) = arguments
                        .and_then(|a| a.get("method"))
                        .and_then(Value::as_str)
                    {
                        wanted_in(method, arguments.and_then(|a| a.get("params")), &mut out);
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
    out
}

/// The folder a wanted path would be allowed by: its own folder (the
/// folder itself for a `dir`), absolute, existing, and spelled as given:
/// a path that goes through a symbolic link, or up with `..`, is never
/// offered (the command then fails with the engine's own reason).
pub fn folder_of(wanted: &Wanted) -> Option<PathBuf> {
    let path = Path::new(&wanted.path);
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return None;
    }
    let folder = if wanted.folder && path.is_dir() {
        path
    } else {
        path.parent()?
    };
    let real = craft3d_engine::canonical(folder).ok()?;
    (real == folder && real.is_dir()).then_some(real)
}

/// The reply that refuses `line` with `why`: an error of the control
/// channel, or, for an MCP message, a tool error (or a JSON-RPC error)
/// that the MCP client reads as such.
pub fn refusal(line: &str, why: &str) -> String {
    use craft3d_automation::rpc;
    let request: Value = serde_json::from_str(line).unwrap_or(Value::Null);
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let message = request
        .get("params")
        .and_then(|p| p.get("line"))
        .and_then(Value::as_str)
        .and_then(|line| serde_json::from_str::<Value>(line).ok());
    match (request.get("method").and_then(Value::as_str), message) {
        (Some("mcp.message"), Some(message)) => {
            let mcp_id = message.get("id").cloned().unwrap_or(Value::Null);
            let reply = if message.get("method").and_then(Value::as_str) == Some("tools/call") {
                serde_json::json!({"jsonrpc": "2.0", "id": mcp_id, "result": {
                    "content": [{"type": "text", "text": why}], "isError": true}})
            } else {
                serde_json::json!({"jsonrpc": "2.0", "id": mcp_id,
                    "error": {"code": -32000, "message": why}})
            };
            rpc::ok_reply(&id, &serde_json::json!({ "reply": reply.to_string() }))
        }
        _ => rpc::error_reply(&id, why),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Ann's places, made as the window makes them, from the environment:
    /// her home folder resolved where it exists (on an atomic desktop
    /// `/home` leads to `/var/home`, on macOS to
    /// `/System/Volumes/Data/home`).
    fn on_unix(home: &str) -> Protected {
        let env = |name: &str| match name {
            "HOME" => Some(home.to_string()),
            "XDG_CONFIG_HOME" => Some(format!("{home}/.config")),
            _ => None,
        };
        let state = PathBuf::from(home).join(".local/state/3dcraft");
        Protected::from_env(&env, Some(&state))
    }

    #[cfg(unix)]
    #[test]
    fn secrets_the_system_and_the_home_folder_are_never_offered() {
        let p = on_unix("/home/ann");
        for refused in [
            "/",
            "/home",
            "/home/ann",
            "/home/ann/.ssh",
            "/home/ann/.ssh/keys",
            "/home/ann/.cargo/bin",
            "/home/ann/.local/bin",
            "/home/ann/.claude",
            "/home/ann/snap/app",
            "//server/share/x",
            "/home/ann/.gnupg",
            "/home/ann/.config/app",
            "/home/ann/.local/share/keyrings",
            "/home/ann/.local",
            "/home/ann/.local/state/3dcraft",
            "/home/ann/.local/state/3dcraft/windows",
            "/home/ann/.local/state",
            "/etc",
            "/etc/ssh",
            "/usr/local/bin",
            "/var",
            "/var/lib",
            "/proc/1",
            "/home/ann/bin",
            "/run/user/1000",
            "/run/user/1000/systemd/user",
            "/home/linuxbrew/.linuxbrew/bin",
            "/home/linuxbrew",
        ] {
            let why = p.reason(Path::new(refused));
            assert!(why.is_some(), "{refused} was offered");
        }
        assert!(
            p.reason(Path::new("/home/ann"))
                .unwrap()
                .contains("home folder itself")
        );
        assert!(
            p.reason(Path::new("/home/ann/.local/state/3dcraft"))
                .unwrap()
                .contains("control token")
        );
        for offered in [
            "/home/ann/projects/brackets",
            "/home/ann/Documents/3DCraft",
            "/srv/work",
            "/home/ann/.local/share/projects",
        ] {
            assert_eq!(p.reason(Path::new(offered)), None, "{offered}");
        }
        // On macOS `/tmp` leads into `/private`, a folder of the system.
        assert_eq!(
            p.reason(Path::new("/tmp/x")).is_some(),
            cfg!(target_os = "macos")
        );
        // A home beneath a system folder: its own folders may be offered.
        let silverblue = on_unix("/var/home/ann");
        assert_eq!(silverblue.reason(Path::new("/var/home/ann/work")), None);
        assert!(silverblue.reason(Path::new("/var/lib")).is_some());
        assert!(silverblue.reason(Path::new("/var/home")).is_some());
        // A relative path is nothing to offer.
        assert!(p.reason(Path::new("work")).is_some());
    }

    /// macOS's places, on any system (the paths need not exist): the home
    /// folder beneath `/Users` with its `Library`, the programs in
    /// `/Applications` and Homebrew's, and `/private`, where `/etc`, `/var`
    /// (with the temporary folders of `/var/folders`) and `/tmp` lead.
    #[cfg(unix)]
    #[test]
    fn the_places_of_macos_are_never_offered() {
        let mac = Protected::new(
            Some(PathBuf::from("/Users/ann")),
            Some(PathBuf::from(
                "/Users/ann/Library/Application Support/3dcraft",
            )),
            Vec::new(),
        );
        for refused in [
            "/Users",
            "/Users/ann",
            "/Users/ann/Library",
            "/Users/ann/Library/Keychains",
            "/Users/ann/Library/Application Support/3dcraft",
            "/Users/ann/Applications",
            "/Users/ann/.ssh",
            "/Applications",
            "/Applications/Tool.app/Contents/MacOS",
            "/opt/homebrew/bin",
            "/Library/LaunchDaemons",
            "/System/Volumes/Data",
            "/private/etc",
            "/private/var/folders/xy/abc/T",
            "/private/tmp/x",
            "/usr/local/bin",
        ] {
            assert!(mac.reason(Path::new(refused)).is_some(), "{refused}");
        }
        for offered in [
            "/Users/ann/Documents/3DCraft",
            "/Users/ann/projects/brackets",
            "/Users/Shared/brackets",
            "/Volumes/Work/brackets",
        ] {
            assert_eq!(mac.reason(Path::new(offered)), None, "{offered}");
        }
    }

    /// The cloud folders of macOS may be a work folder although
    /// `~/Library` is protected (the developer, 2026-11-10): each
    /// provider's folder beneath `CloudStorage` and iCloud Drive, with
    /// what is in them; not the list of providers, not the other folders
    /// of `Mobile Documents` (apps' own), nothing else of `~/Library`.
    /// Paths as data: they need not exist.
    #[cfg(unix)]
    #[test]
    fn the_cloud_folders_of_macos_may_be_offered() {
        let mac = Protected::new(Some(PathBuf::from("/Users/ann")), None, Vec::new());
        for offered in [
            "/Users/ann/Library/CloudStorage/Dropbox",
            "/Users/ann/Library/CloudStorage/Dropbox/brackets",
            "/Users/ann/Library/CloudStorage/OneDrive-Personal/work",
            "/Users/ann/Library/CloudStorage/GoogleDrive-ann@example.com/My Drive/3DCraft",
            "/Users/ann/Library/CloudStorage/Box-Box/parts",
            "/Users/ann/Library/Mobile Documents/com~apple~CloudDocs",
            "/Users/ann/Library/Mobile Documents/com~apple~CloudDocs/3DCraft",
        ] {
            assert_eq!(mac.reason(Path::new(offered)), None, "{offered}");
        }
        for refused in [
            "/Users/ann/Library",
            "/Users/ann/Library/CloudStorage",
            "/Users/ann/Library/Mobile Documents",
            "/Users/ann/Library/Mobile Documents/com~apple~Pages",
            "/Users/ann/Library/Mobile Documents/com~apple~Pages/Documents",
            "/Users/ann/Library/Preferences",
            "/Users/ann/Library/Keychains",
            "/Users/ann/Library/CloudStorage/../Preferences",
        ] {
            assert!(mac.reason(Path::new(refused)).is_some(), "{refused}");
        }
    }

    /// In a cloud folder a link is judged where it leads: into a folder
    /// of keys or elsewhere in `~/Library`, refused; a link from
    /// elsewhere into a cloud folder, offered.
    #[cfg(unix)]
    #[test]
    fn a_link_in_or_into_a_cloud_folder_is_judged_where_it_leads() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/tmp/craft3d-consent-cloud-links");
        let _ = std::fs::remove_dir_all(&dir);
        let dropbox = dir.join("Library/CloudStorage/Dropbox");
        std::fs::create_dir_all(dropbox.join("work")).unwrap();
        std::fs::create_dir_all(dir.join("Library/Preferences")).unwrap();
        std::fs::create_dir_all(dir.join(".ssh")).unwrap();
        std::fs::create_dir_all(dir.join("projects")).unwrap();
        let home = craft3d_engine::canonical(&dir).unwrap();
        let dropbox = home.join("Library/CloudStorage/Dropbox");
        std::os::unix::fs::symlink(home.join(".ssh"), dropbox.join("keys")).unwrap();
        std::os::unix::fs::symlink(home.join("Library/Preferences"), dropbox.join("prefs"))
            .unwrap();
        std::os::unix::fs::symlink(&dropbox, home.join("projects/dropbox")).unwrap();
        let p = Protected::new(Some(home.clone()), None, Vec::new());
        assert_eq!(p.reason(&dropbox.join("work")), None);
        let why = p.reason(&dropbox.join("keys")).unwrap();
        assert!(why.contains("keys and settings"), "{why}");
        assert!(p.reason(&dropbox.join("prefs")).is_some());
        assert!(p.reason(&dropbox.join("keys/new")).is_some());
        assert_eq!(p.reason(&home.join("projects/dropbox")), None);
        assert_eq!(p.reason(&home.join("projects/dropbox/work")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A folder spelled through a link is judged by where the link leads:
    /// into a folder of keys, or out of the home folder into the system's.
    #[cfg(unix)]
    #[test]
    fn a_link_into_a_protected_place_is_never_offered() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/tmp/craft3d-consent-protected-links");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".ssh")).unwrap();
        std::fs::create_dir_all(dir.join("work")).unwrap();
        let home = craft3d_engine::canonical(&dir).unwrap();
        std::os::unix::fs::symlink(home.join(".ssh"), home.join("work/keys")).unwrap();
        std::os::unix::fs::symlink("/etc", home.join("work/settings")).unwrap();
        let p = Protected::new(Some(home.clone()), None, Vec::new());
        assert_eq!(p.reason(&home.join("work")), None);
        let why = p.reason(&home.join("work/keys")).unwrap();
        assert!(why.contains("keys and settings"), "{why}");
        // The folder it leads to is said once.
        assert_eq!(why.matches(".ssh").count(), 1, "{why}");
        let why = p.reason(&home.join("work/settings")).unwrap();
        assert!(why.contains("of the system"), "{why}");
        // A folder that does not exist yet: the part that does is
        // resolved, the rest taken as written.
        let why = p.reason(&home.join("work/keys/new")).unwrap();
        assert!(why.contains("keys and settings"), "{why}");
        assert!(p.reason(&home.join("work/settings/new/deeper")).is_some());
        assert_eq!(p.reason(&home.join("work/new/deeper")), None);
        // Up and back, or a `.`, as written: refused whatever it leads to.
        let why = p.reason(&home.join("work/../.ssh/new")).unwrap();
        assert!(why.contains("`..`"), "{why}");
        assert!(p.reason(&home.join("work/./new")).is_some());
        assert!(p.reason(&home.join("work/new/..")).is_some());
        // A link that leads nowhere: where it would lead cannot be told.
        std::os::unix::fs::symlink(home.join("missing"), home.join("work/dangling")).unwrap();
        assert!(p.reason(&home.join("work/dangling/new")).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_environments_places_are_protected() {
        let env = |name: &str| match name {
            "HOME" | "USERPROFILE" => Some(
                if cfg!(windows) {
                    r"C:\Users\ann"
                } else {
                    "/home/ann"
                }
                .to_string(),
            ),
            "APPDATA" => Some(
                if cfg!(windows) {
                    r"C:\Users\ann\AppData\Roaming"
                } else {
                    "/home/ann/appdata"
                }
                .to_string(),
            ),
            _ => None,
        };
        let p = Protected::from_env(&env, None);
        let appdata = PathBuf::from(env("APPDATA").unwrap());
        assert!(p.reason(&appdata.join("x")).is_some());
    }

    #[test]
    fn the_paths_a_request_names_are_found_in_every_shape() {
        let line = |v: Value| v.to_string();
        let mcp = |tool: &str, arguments: Value| {
            line(json!({"id": 1, "method": "mcp.message", "params": {"line":
                json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call",
                       "params": {"name": tool, "arguments": arguments}}).to_string()}}))
        };
        let got = wanted(&line(json!({"id": 1, "method": "engine.execute",
            "params": {"command": "file.save", "params": {"path": "/w/a.3dcraft"}}})));
        assert_eq!(
            got,
            [Wanted {
                command: "file.save".into(),
                path: "/w/a.3dcraft".into(),
                write: true,
                folder: false
            }]
        );
        let got = wanted(&mcp(
            "command_run",
            json!({"id": "file.open", "params": {"path": "/w/b.3dcraft"}}),
        ));
        assert_eq!(
            (got[0].command.as_str(), got[0].write),
            ("file.open", false)
        );
        let got = wanted(&mcp(
            "command_batch",
            json!({"steps": [{"id": "file.export", "params": {"path": "/w/c.stl"}},
                             {"id": "view.replay", "params": {"dir": "/w/frames"}}]}),
        ));
        assert_eq!(got.len(), 2);
        assert!(got[1].folder);
        let got = wanted(&mcp(
            "control_call",
            json!({"method": "ui.screenshot", "params": {"path": "/w/s.png"}}),
        ));
        assert_eq!(got[0].path, "/w/s.png");
        assert!(wanted(&line(json!({"id": 1, "method": "ui.get"}))).is_empty());
        assert!(wanted("not json").is_empty());
        assert!(
            wanted(&mcp(
                "command_run",
                json!({"id": "file.save", "params": {"path": 5}})
            ))
            .is_empty()
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_path_through_a_link_or_up_is_never_offered() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/tmp/craft3d-consent-links");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("real")).unwrap();
        let dir = craft3d_engine::canonical(&dir).unwrap();
        std::os::unix::fs::symlink(dir.join("real"), dir.join("link")).unwrap();
        let want = |path: PathBuf| Wanted {
            command: "file.save".into(),
            path: path.to_str().unwrap().to_string(),
            write: true,
            folder: false,
        };
        assert_eq!(
            folder_of(&want(dir.join("real").join("a.3dcraft"))),
            Some(dir.join("real"))
        );
        assert_eq!(folder_of(&want(dir.join("link").join("a.3dcraft"))), None);
        assert_eq!(
            folder_of(&want(dir.join("real").join("..").join("a.3dcraft"))),
            None
        );
        assert_eq!(
            folder_of(&want(dir.join("missing").join("a.3dcraft"))),
            None
        );
        assert_eq!(folder_of(&want(PathBuf::from("relative.3dcraft"))), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_refusal_is_a_tool_error_for_an_mcp_client() {
        let tools = json!({"id": 3, "method": "mcp.message", "params": {"line":
            json!({"jsonrpc": "2.0", "id": 9, "method": "tools/call", "params": {}}).to_string()}})
        .to_string();
        let reply: Value = serde_json::from_str(&refusal(&tools, "no")).unwrap();
        assert_eq!(reply["ok"], true);
        let inner: Value =
            serde_json::from_str(reply["result"]["reply"].as_str().unwrap()).unwrap();
        assert_eq!(inner["id"], 9);
        assert_eq!(inner["result"]["isError"], true);
        assert_eq!(inner["result"]["content"][0]["text"], "no");
        let plain: Value =
            serde_json::from_str(&refusal(r#"{"id": 4, "method": "engine.execute"}"#, "no"))
                .unwrap();
        assert_eq!(
            (plain["ok"].clone(), plain["id"].clone()),
            (json!(false), json!(4))
        );
    }
}
