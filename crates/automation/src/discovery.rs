//! How an MCP bridge finds a window that allows AI agents, with nothing
//! given on its command line (docs/control-protocol.md, "Discovery").
//!
//! Everything lives in 3DCraft's own folder in the person's state directory
//! (`3dcraft/` beside the recovery directory and the recent files):
//!
//! - `control.token`: the default control token (written by the window,
//!   mode 0600; this module only reads it);
//! - `windows/<pid>.json`: one small entry per window that listens, with
//!   its process id, its port, the path of its token file and when it was
//!   last focused. No secret: the token stays in its own file;
//! - `agents.json`: whether the person allows AI agents, remembered for
//!   the next start.
//!
//! Every file here is read as hostile input: size-capped, a bad one is
//! skipped (an entry) or read as "off" (the preference), never a panic.

use serde_json::{Value, json};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The default token file's name in the state folder.
pub const TOKEN_FILE: &str = "control.token";
/// The folder of the discovery entries in the state folder.
pub const WINDOWS_DIR: &str = "windows";
/// The preference's file in the state folder.
pub const PREFERENCE_FILE: &str = "agents.json";
/// The first port a window tries when nothing names one, and how many
/// after it (7878 to 7897).
pub const FIRST_PORT: u16 = 7878;
pub const PORT_TRIES: u16 = 20;

/// Most bytes read of an entry or the preference.
const MAX_FILE_BYTES: u64 = 4096;
/// Most entries looked at: there are never more windows than this.
const MAX_ENTRIES: usize = 256;

/// 3DCraft's folder in the person's state directory (`3dcraft` beside the
/// default recovery directory), or `None` when the environment names no
/// state directory.
pub fn default_state_folder(env: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let recovery = craft3d_engine::default_recovery_directory(env)?;
    recovery.parent().map(Path::to_path_buf)
}

/// The default token file in `folder`.
pub fn default_token_file(folder: &Path) -> PathBuf {
    folder.join(TOKEN_FILE)
}

/// One window that listens for agents, as its entry says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowEntry {
    pub pid: u32,
    pub port: u16,
    /// The token file that window uses, absolute.
    pub token_file: PathBuf,
    /// When it was last focused, in milliseconds since the Unix epoch; 0
    /// when it has not been yet.
    pub focused_at: u64,
    /// When it started listening, in the same unit.
    pub started_at: u64,
}

impl WindowEntry {
    pub fn to_json(&self) -> Value {
        json!({
            "pid": self.pid,
            "port": self.port,
            "tokenFile": self.token_file.to_string_lossy(),
            "focusedAt": self.focused_at,
            "startedAt": self.started_at,
        })
    }

    /// Reads an entry; `None` for anything that is not a whole one.
    pub fn from_json(text: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(text).ok()?;
        let pid = u32::try_from(value.get("pid")?.as_u64()?).ok()?;
        let port = u16::try_from(value.get("port")?.as_u64()?).ok()?;
        let token_file = PathBuf::from(value.get("tokenFile")?.as_str()?);
        if port == 0 || !token_file.is_absolute() {
            return None;
        }
        let number = |key: &str| value.get(key).and_then(Value::as_u64).unwrap_or(0);
        Some(Self {
            pid,
            port,
            token_file,
            focused_at: number("focusedAt"),
            started_at: number("startedAt"),
        })
    }

    /// How the error messages name it.
    pub fn describe(&self) -> String {
        format!("3DCraft (process {}) on port {}", self.pid, self.port)
    }
}

/// Milliseconds since the Unix epoch, 0 for a clock before it.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn read_capped(path: &Path) -> Option<String> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// Makes `dir` (and what is above it), private to its owner on Unix.
fn make_private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Counts the temporary files this process made.
static TEMPORARIES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Writes `text` to `path` whole: through a new temporary file of this
/// process's own beside it (mode 0600 on Unix, never through a link),
/// renamed over it.
pub fn write_atomically(path: &Path, text: &str) -> Result<(), String> {
    use std::io::Write;
    let shown = path.display();
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| format!("`{shown}` has no folder"))?;
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("cannot make the folder of `{shown}`: {e}"))?;
    let count = TEMPORARIES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(format!(".{}-{count}.tmp", std::process::id()));
    let temporary = PathBuf::from(temporary);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = options.open(&temporary).and_then(|mut file| {
        file.write_all(text.as_bytes())?;
        file.sync_all()
    });
    if let Err(e) = written {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!("cannot write `{shown}`: {e}"));
    }
    std::fs::rename(&temporary, path).map_err(|e| {
        let _ = std::fs::remove_file(&temporary);
        format!("cannot write `{shown}`: {e}")
    })
}

/// The entry file of the window with process id `pid`.
pub fn entry_path(folder: &Path, pid: u32) -> PathBuf {
    folder.join(WINDOWS_DIR).join(format!("{pid}.json"))
}

/// Writes (or rewrites) a window's entry.
pub fn write_entry(folder: &Path, entry: &WindowEntry) -> Result<(), String> {
    let dir = folder.join(WINDOWS_DIR);
    make_private_dir(&dir).map_err(|e| format!("cannot make `{}`: {e}", dir.display()))?;
    write_atomically(&entry_path(folder, entry.pid), &entry.to_json().to_string())
}

/// Removes a window's entry; one that is not there is no error.
pub fn remove_entry(folder: &Path, pid: u32) {
    let _ = std::fs::remove_file(entry_path(folder, pid));
}

/// The entries in `folder`, in the order of their process ids; files that
/// do not read as one are left out (and left alone).
pub fn read_entries(folder: &Path) -> Vec<WindowEntry> {
    let Ok(dir) = std::fs::read_dir(folder.join(WINDOWS_DIR)) else {
        return Vec::new();
    };
    let mut entries: Vec<WindowEntry> = dir
        .filter_map(Result::ok)
        .take(MAX_ENTRIES)
        .filter_map(|item| {
            let path = item.path();
            let name = path.file_name()?.to_str()?;
            let pid: u32 = name.strip_suffix(".json")?.parse().ok()?;
            let entry = WindowEntry::from_json(&read_capped(&path)?)?;
            // An entry is only believed under its own process's name.
            (entry.pid == pid).then_some(entry)
        })
        .collect();
    entries.sort_by_key(|entry| entry.pid);
    entries
}

/// Whether the process may still be running: on Linux, whether it is in
/// `/proc`; elsewhere this cannot be told without the system's calls, and
/// the port alone decides.
fn process_may_run(pid: u32) -> bool {
    if cfg!(target_os = "linux") && Path::new("/proc/self").exists() {
        return Path::new("/proc").join(pid.to_string()).exists();
    }
    true
}

/// Whether something accepts connections on the entry's port.
fn port_answers(port: u16, timeout: Duration) -> bool {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    TcpStream::connect_timeout(&address, timeout).is_ok()
}

/// Whether the window of an entry still listens.
pub fn is_live(entry: &WindowEntry) -> bool {
    process_may_run(entry.pid) && port_answers(entry.port, Duration::from_millis(500))
}

/// The entries of windows that still listen; the files of the others
/// (a window that crashed or was killed) are removed. `except` is a
/// process id never removed (the caller's own).
pub fn live_entries(folder: &Path, except: Option<u32>) -> Vec<WindowEntry> {
    read_entries(folder)
        .into_iter()
        .filter(|entry| {
            if Some(entry.pid) == except || is_live(entry) {
                true
            } else {
                remove_entry(folder, entry.pid);
                false
            }
        })
        .collect()
}

/// Which window a bridge talks to, of the `live` ones (on `port` only,
/// when one is given): the only one, else the one focused most recently.
/// None, or several where none was focused later than the others, is an
/// error that says what to do and names the windows.
pub fn choose(live: &[WindowEntry], port: Option<u16>) -> Result<WindowEntry, String> {
    let candidates: Vec<&WindowEntry> = live
        .iter()
        .filter(|entry| port.is_none_or(|port| entry.port == port))
        .collect();
    let mut sorted = candidates.clone();
    sorted.sort_by_key(|entry| std::cmp::Reverse((entry.focused_at, entry.started_at)));
    match sorted.as_slice() {
        [] => Err(match port {
            Some(port) => format!(
                "no 3DCraft window that allows AI agents listens on port {port}; open 3DCraft and switch on Allow AI agents (on the start screen or in the Help menu)"
            ),
            None => "no 3DCraft window allows AI agents right now; open 3DCraft and switch on Allow AI agents (on the start screen or in the Help menu)".to_string(),
        }),
        [only] => Ok((*only).clone()),
        [first, second, ..] if first.focused_at > second.focused_at => Ok((*first).clone()),
        several => {
            let names: Vec<String> = several.iter().map(|entry| entry.describe()).collect();
            Err(format!(
                "several 3DCraft windows allow AI agents and none was used last: {}; click into the window the agent should use and try again, or give its port (`craft3d-cli mcp --bridge PORT`)",
                names.join(", ")
            ))
        }
    }
}

/// What the person chose for agents, remembered in `agents.json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preferences {
    /// The switch Allow AI agents.
    pub allow_agents: bool,
    /// The folder agents may read and write files in, chosen with
    /// Change…; `None` for the default (`Documents/3DCraft`).
    pub folder: Option<PathBuf>,
    /// Also the open document's folder (on unless turned off).
    pub document_folder: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            allow_agents: false,
            folder: None,
            document_folder: true,
        }
    }
}

/// What is remembered in `folder`; the defaults for what is not (or not
/// readably): the switch off, the default folder, the document's folder on.
pub fn read_preferences(folder: &Path) -> Preferences {
    let Some(value) = read_capped(&folder.join(PREFERENCE_FILE))
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
    else {
        return Preferences::default();
    };
    let defaults = Preferences::default();
    Preferences {
        allow_agents: value
            .get("allowAgents")
            .and_then(Value::as_bool)
            .unwrap_or(defaults.allow_agents),
        folder: value
            .get("folder")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute()),
        document_folder: value
            .get("documentFolder")
            .and_then(Value::as_bool)
            .unwrap_or(defaults.document_folder),
    }
}

/// Remembers what the person chose.
pub fn write_preferences(folder: &Path, preferences: &Preferences) -> Result<(), String> {
    let chosen = preferences
        .folder
        .as_ref()
        .map(|path| Value::String(path.to_string_lossy().into_owned()))
        .unwrap_or(Value::Null);
    write_atomically(
        &folder.join(PREFERENCE_FILE),
        &json!({
            "allowAgents": preferences.allow_agents,
            "folder": chosen,
            "documentFolder": preferences.document_folder,
        })
        .to_string(),
    )
}

/// Whether the person allows AI agents, as remembered in `folder`; off
/// when nothing (or nothing readable) is remembered.
pub fn read_preference(folder: &Path) -> bool {
    read_preferences(folder).allow_agents
}

/// Remembers whether the person allows AI agents, keeping the rest.
pub fn write_preference(folder: &Path, allowed: bool) -> Result<(), String> {
    let mut preferences = read_preferences(folder);
    preferences.allow_agents = allowed;
    write_preferences(folder, &preferences)
}

/// Reads a control token from `path`: 64 hexadecimal characters, lower
/// case. The error names the file and what to do, never what it holds.
pub fn read_token(path: &Path) -> Result<String, String> {
    let shown = path.display();
    let text = read_capped(path).ok_or_else(|| {
        format!(
            "cannot read the control token file `{shown}`: it is missing, not a file or too large"
        )
    })?;
    let token = text.trim();
    if token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(token.to_ascii_lowercase())
    } else {
        Err(format!(
            "`{shown}` does not hold a control token (64 hexadecimal characters)"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn dir(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../target/tmp/craft3d-discovery-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn entry(pid: u32, port: u16, focused_at: u64) -> WindowEntry {
        WindowEntry {
            pid,
            port,
            token_file: std::path::absolute("t.token").unwrap(),
            focused_at,
            started_at: 1,
        }
    }

    #[test]
    fn the_state_folder_is_beside_the_recovery_directory() {
        let env = |name: &str| match name {
            "HOME" => Some("/home/me".to_string()),
            "XDG_STATE_HOME" => Some("/state".to_string()),
            "LOCALAPPDATA" => Some("C:\\state".to_string()),
            _ => None,
        };
        let folder = default_state_folder(&env).unwrap();
        assert!(folder.ends_with("3dcraft"), "{folder:?}");
        assert_eq!(
            default_token_file(&folder),
            folder.join("control.token"),
            "beside recovery/ and recent-files.json"
        );
        assert_eq!(default_state_folder(&|_| None), None);
    }

    #[test]
    fn entries_round_trip_and_bad_ones_are_skipped() {
        let folder = dir("entries");
        let ours = entry(std::process::id(), 7901, 5);
        write_entry(&folder, &ours).unwrap();
        assert_eq!(read_entries(&folder), [ours.clone()]);
        let text = std::fs::read_to_string(entry_path(&folder, ours.pid)).unwrap();
        assert!(!text.contains("token\":\""), "no token in an entry: {text}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&entry_path(&folder, ours.pid)), 0o600);
            assert_eq!(mode(&folder.join(WINDOWS_DIR)), 0o700);
        }
        let windows = folder.join(WINDOWS_DIR);
        for (name, content) in [
            ("1.json", "{"),
            ("2.json", r#"{"pid": 2, "port": 0, "tokenFile": "/t"}"#),
            ("3.json", r#"{"pid": 4, "port": 7000, "tokenFile": "/t"}"#),
            ("5.json", r#"{"pid": 5, "port": 70000, "tokenFile": "/t"}"#),
            (
                "6.json",
                r#"{"pid": 6, "port": 7000, "tokenFile": "relative"}"#,
            ),
            ("notes.txt", "hello"),
            ("x.json", r#"{"pid": 1, "port": 7000, "tokenFile": "/t"}"#),
        ] {
            std::fs::write(windows.join(name), content).unwrap();
        }
        std::fs::write(windows.join("7.json"), "x".repeat(10_000)).unwrap();
        assert_eq!(read_entries(&folder), [ours.clone()]);
        remove_entry(&folder, ours.pid);
        remove_entry(&folder, ours.pid);
        assert!(read_entries(&folder).is_empty());
        assert!(read_entries(&folder.join("nowhere")).is_empty());
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_stale_entry_is_removed_and_a_live_one_kept() {
        let folder = dir("stale");
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let live = entry(std::process::id(), port, 1);
        write_entry(&folder, &live).unwrap();
        // A window that was killed: its port no longer answers.
        let gone = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let gone_port = gone.local_addr().unwrap().port();
        drop(gone);
        // (A process id that is ours keeps it from looking dead on Linux;
        // the port decides.)
        let mut dead = entry(1, gone_port, 9);
        dead.pid = u32::MAX - 1;
        write_entry(&folder, &dead).unwrap();
        assert_eq!(read_entries(&folder).len(), 2);
        assert_eq!(live_entries(&folder, None), [live.clone()]);
        assert!(!entry_path(&folder, dead.pid).exists(), "cleaned up");
        // A caller's own entry is never removed, live or not.
        drop(listener);
        assert_eq!(live_entries(&folder, Some(live.pid)), [live.clone()]);
        assert!(live_entries(&folder, None).is_empty());
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn the_bridge_picks_the_only_or_the_last_focused_window() {
        assert!(choose(&[], None).unwrap_err().contains("Allow AI agents"));
        let a = entry(10, 7878, 100);
        let b = entry(11, 7879, 200);
        assert_eq!(choose(std::slice::from_ref(&a), None).unwrap(), a);
        assert_eq!(choose(&[a.clone(), b.clone()], None).unwrap(), b);
        assert_eq!(choose(&[a.clone(), b.clone()], Some(7878)).unwrap(), a);
        assert!(
            choose(&[a.clone(), b.clone()], Some(7000))
                .unwrap_err()
                .contains("port 7000")
        );
        // Neither focused later than the other: ambiguous, both named.
        let c = entry(12, 7880, 200);
        let error = choose(&[a, b, c], None).unwrap_err();
        assert!(error.contains("several"), "{error}");
        assert!(
            error.contains("process 11 on port 7879") && error.contains("process 12 on port 7880"),
            "{error}"
        );
    }

    #[test]
    fn the_preference_is_off_unless_remembered_on() {
        let folder = dir("preference");
        assert!(!read_preference(&folder));
        write_preference(&folder, true).unwrap();
        assert!(read_preference(&folder));
        write_preference(&folder, false).unwrap();
        assert!(!read_preference(&folder));
        for bad in ["", "{", "[true]", r#"{"allowAgents": "yes"}"#] {
            std::fs::write(folder.join(PREFERENCE_FILE), bad).unwrap();
            assert!(!read_preference(&folder), "{bad:?}");
        }
        // The folder and the document's folder are kept beside it.
        let chosen = std::path::absolute("chosen").unwrap();
        write_preferences(
            &folder,
            &Preferences {
                allow_agents: false,
                folder: Some(chosen.clone()),
                document_folder: false,
            },
        )
        .unwrap();
        write_preference(&folder, true).unwrap();
        let read = read_preferences(&folder);
        assert_eq!(
            read,
            Preferences {
                allow_agents: true,
                folder: Some(chosen),
                document_folder: false
            }
        );
        // A relative folder is not believed; the document's folder is on
        // unless said otherwise.
        std::fs::write(folder.join(PREFERENCE_FILE), r#"{"folder": "rel"}"#).unwrap();
        assert_eq!(read_preferences(&folder), Preferences::default());
        assert!(Preferences::default().document_folder);
        // A folder that cannot be made: an error, not a panic.
        let file = folder.join("file");
        std::fs::write(&file, "").unwrap();
        assert!(write_preference(&file.join("below"), true).is_err());
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_token_file_is_read_and_a_bad_one_named_without_its_content() {
        let folder = dir("token");
        let path = folder.join(TOKEN_FILE);
        assert!(read_token(&path).unwrap_err().contains("cannot read"));
        let token = "AB".repeat(32);
        std::fs::write(&path, format!("{token}\n")).unwrap();
        assert_eq!(read_token(&path).unwrap(), token.to_ascii_lowercase());
        std::fs::write(&path, "secret").unwrap();
        let error = read_token(&path).unwrap_err();
        assert!(error.contains("does not hold") && !error.contains("secret"));
        let _ = std::fs::remove_dir_all(&folder);
    }
}
