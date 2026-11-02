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
/// programs that run (a file planted there would run).
const HOME_SECRETS: [&str; 23] = [
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
];

/// Folders of the system.
const SYSTEM: [&str; 18] = [
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
    "/System",
    "/Library",
    "/private",
    r"C:\Windows",
    r"C:\Program Files",
    r"C:\ProgramData",
];

/// The places never offered to an agent.
#[derive(Clone, Debug, Default)]
pub struct Protected {
    pub home: Option<PathBuf>,
    /// 3DCraft's state folder (the control token lives there).
    pub state: Option<PathBuf>,
    /// Places from the environment: `%APPDATA%`, `%LOCALAPPDATA%`,
    /// `%SystemRoot%`, `%ProgramFiles%`, `$XDG_CONFIG_HOME`.
    pub places: Vec<PathBuf>,
}

/// `path` with its links resolved where it exists, else as given.
fn real(path: &Path) -> PathBuf {
    craft3d_engine::canonical(path).unwrap_or_else(|_| path.to_path_buf())
}

impl Protected {
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
        ]
        .iter()
        .filter_map(|name| var(name))
        .collect();
        Self {
            home: home.map(|h| real(&h)),
            state: state.map(real),
            places,
        }
    }

    /// Why `folder` (absolute, its links resolved) is never offered to an
    /// agent; `None` when it may be.
    pub fn reason(&self, folder: &Path) -> Option<String> {
        let shown = folder.display();
        // A network or device path (`\\server\share`, `\\?\C:\`,
        // `\\localhost\C$`): never offered, its identity cannot be told.
        let text = folder.to_string_lossy();
        if text.starts_with("\\\\") || text.starts_with("//") {
            return Some(format!("`{shown}` is a network or device path"));
        }
        if folder.parent().is_none() || !folder.is_absolute() {
            return Some(format!(
                "`{shown}` is the root of a drive or of the file system"
            ));
        }
        // A folder that holds `place`, or lies in it.
        let clash = |place: &Path, what: &str| -> Option<String> {
            if folder.starts_with(place) {
                Some(format!("`{shown}` is in {what} (`{}`)", place.display()))
            } else if place.starts_with(folder) {
                Some(format!("`{shown}` holds {what} (`{}`)", place.display()))
            } else {
                None
            }
        };
        let mut sensitive: Vec<(PathBuf, &str)> = Vec::new();
        if let Some(home) = &self.home {
            if folder == home {
                return Some(format!("`{shown}` is your home folder itself"));
            }
            if home.starts_with(folder) {
                return Some(format!("`{shown}` holds your home folder"));
            }
            // (After the home folder itself: the state folder is in it.)
            if let Some(state) = &self.state
                && let Some(why) = clash(state, "3DCraft's own folder, where its control token is")
            {
                return Some(why);
            }
            for secret in HOME_SECRETS {
                sensitive.push((real(&home.join(secret)), "a folder of keys and settings"));
            }
        }
        if self.home.is_none()
            && let Some(state) = &self.state
            && let Some(why) = clash(state, "3DCraft's own folder, where its control token is")
        {
            return Some(why);
        }
        for place in &self.places {
            sensitive.push((real(place), "a folder of the system or of your settings"));
        }
        // System folders only for what lies outside the home folder: some
        // systems keep homes beneath one of them (`/var/home`).
        let in_home = self
            .home
            .as_ref()
            .is_some_and(|home| folder.starts_with(home));
        if !in_home {
            for system in SYSTEM {
                let path = PathBuf::from(system);
                if path.is_absolute() {
                    sensitive.push((real(&path), "a folder of the system"));
                }
            }
        }
        sensitive
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

    fn protected() -> Protected {
        Protected {
            home: Some(PathBuf::from("/home/ann")),
            state: Some(PathBuf::from("/home/ann/.local/state/3dcraft")),
            places: vec![PathBuf::from("/home/ann/.config")],
        }
    }

    #[cfg(unix)]
    #[test]
    fn secrets_the_system_and_the_home_folder_are_never_offered() {
        let p = protected();
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
            "/tmp/x",
            "/home/ann/.local/share/projects",
        ] {
            assert_eq!(p.reason(Path::new(offered)), None, "{offered}");
        }
        // A home beneath a system folder: its own folders may be offered.
        let silverblue = Protected {
            home: Some(PathBuf::from("/var/home/ann")),
            ..Protected::default()
        };
        assert_eq!(silverblue.reason(Path::new("/var/home/ann/work")), None);
        assert!(silverblue.reason(Path::new("/var/lib")).is_some());
        assert!(silverblue.reason(Path::new("/var/home")).is_some());
        // A relative path is nothing to offer.
        assert!(p.reason(Path::new("work")).is_some());
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
        let appdata = p.places[0].clone();
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
