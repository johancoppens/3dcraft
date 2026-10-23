//! What the window keeps and opens outside a document: the list of recent
//! files on disk, and the pages and folders the Help menu opens with the
//! platform's own opener (the browser, the file manager).
//!
//! The recent files live in `recent-files.json` next to the default
//! recovery directory, in the person's state directory (`--recent-file`
//! names another file). The file is read as hostile input (a corrupt one
//! is an empty list, never an error) and written whole through a
//! temporary file of this process's own, so a crash leaves the old list or
//! the new one. Before writing, what another window added since is read
//! back and kept.

use crate::args::Options;
use crate::host::Host;
use craft3d_ui_egui::shell::{AgentSetup, External, RecentFiles};
use std::path::{Path, PathBuf};

/// The name of the file the recent files are kept in.
pub const RECENT_FILE: &str = "recent-files.json";
/// Most bytes of it read: a list of ten paths is far smaller.
const MAX_RECENT_BYTES: u64 = 64 * 1024;

/// Where the recent files are kept when nothing says otherwise: beside
/// the recovery directory (`3dcraft/recent-files.json` in the state
/// directory, for the default one). `None` for a directory with nothing
/// above it.
pub fn recent_file_beside(recovery: &Path) -> Option<PathBuf> {
    let parent = recovery.parent().filter(|p| !p.as_os_str().is_empty())?;
    Some(parent.join(RECENT_FILE))
}

/// The recent files kept at `path`; an empty list when there is no such
/// file or it does not read.
pub fn read_recent(path: &Path) -> RecentFiles {
    let fits = std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() <= MAX_RECENT_BYTES);
    if !fits {
        return RecentFiles::default();
    }
    std::fs::read_to_string(path)
        .map(|text| RecentFiles::from_json(&text))
        .unwrap_or_default()
}

/// `ours`, with what another window added to the file since this one
/// last read or wrote it (`baseline`): those come after ours.
pub fn merge_recent(
    ours: &RecentFiles,
    baseline: &RecentFiles,
    on_disk: &RecentFiles,
) -> RecentFiles {
    let added = on_disk
        .paths()
        .iter()
        .filter(|path| !baseline.paths().contains(path) && !ours.paths().contains(path));
    let order: Vec<&String> = ours.paths().iter().chain(added).collect();
    let mut merged = RecentFiles::default();
    // Noted oldest first: each goes in front.
    for path in order
        .into_iter()
        .take(craft3d_ui_egui::shell::MAX_RECENT)
        .rev()
    {
        merged.note(path);
    }
    merged
}

/// Counts the temporary files this process made.
static TEMPORARIES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Stores `ours` at `path`, merged with what another window wrote there
/// since `baseline` ([`merge_recent`]); returns what was written. Through
/// a temporary file of this process's own beside it (made new, so it never
/// follows a link someone left there), renamed over the file.
pub fn store_recent(
    path: &Path,
    ours: &RecentFiles,
    baseline: &RecentFiles,
) -> Result<RecentFiles, String> {
    use std::io::Write;
    let shown = path.display();
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot make the folder of `{shown}`: {e}"))?;
    }
    let merged = merge_recent(ours, baseline, &read_recent(path));
    let count = TEMPORARIES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(format!(".{}-{count}.tmp", std::process::id()));
    let temporary = PathBuf::from(temporary);
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .and_then(|mut file| file.write_all(merged.to_json().as_bytes()));
    if let Err(e) = written {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!("cannot write `{shown}`: {e}"));
    }
    std::fs::rename(&temporary, path).map_err(|e| {
        let _ = std::fs::remove_file(&temporary);
        format!("cannot write `{shown}`: {e}")
    })?;
    Ok(merged)
}

/// The folder Help, Open data folder shows: 3DCraft's own folder in the
/// state directory (the parent of the default recovery directory), when
/// the window keeps its autosaves or its recent files there; `None`
/// otherwise, so a folder given on the command line is never opened or
/// made by the menu.
pub fn data_folder(options: &Options, env: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let recovery = craft3d_engine::default_recovery_directory(env)?;
    let own = recovery.parent()?.to_path_buf();
    let used = options.recovery_dir.as_deref() == Some(recovery.as_path())
        || options.recent_file.as_deref() == Some(own.join(RECENT_FILE).as_path());
    used.then_some(own)
}

/// Where a token file would go when the window does not listen yet, for
/// the start screen's configuration: absolute, in the user's
/// configuration folder (`$XDG_CONFIG_HOME` or `~/.config` spelled out,
/// `%APPDATA%\3DCraft`, `~/Library/Application Support/3DCraft`).
pub fn suggested_token_file(env: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    let var = |name: &str| {
        env(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
    };
    let file = if cfg!(windows) {
        var("APPDATA")?.join("3DCraft").join("control.token")
    } else if cfg!(target_os = "macos") {
        var("HOME")?
            .join("Library")
            .join("Application Support")
            .join("3DCraft")
            .join("control.token")
    } else {
        var("XDG_CONFIG_HOME")
            .or_else(|| var("HOME").map(|home| home.join(".config")))?
            .join("craft3d-control.token")
    };
    file.to_str().map(str::to_string)
}

/// How an agent reaches this window: `craft3d-cli` beside this program
/// when it is there (as the installers put it), else by its name on the
/// `PATH`; the port the window listens on and its token file, absolute.
pub fn agent_setup(
    port: Option<u16>,
    token_file: Option<&Path>,
    env: &dyn Fn(&str) -> Option<String>,
) -> AgentSetup {
    let name = if cfg!(windows) {
        "craft3d-cli.exe"
    } else {
        "craft3d-cli"
    };
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(name)))
        .filter(|cli| cli.is_file())
        .and_then(|cli| cli.to_str().map(str::to_string));
    let token_file = token_file
        .and_then(|file| std::path::absolute(file).ok())
        .and_then(|file| file.to_str().map(str::to_string));
    AgentSetup {
        cli: beside.unwrap_or_else(|| "craft3d-cli".to_string()),
        port,
        token_file,
        suggested_token: suggested_token_file(env),
        windows: cfg!(windows),
    }
}

/// The program that opens `target` on this platform, and its arguments.
pub fn opener(target: &External) -> (&'static str, Vec<String>) {
    let what = match target {
        External::Url(url) => url.clone(),
        External::Folder(folder) => folder.display().to_string(),
    };
    if cfg!(target_os = "macos") {
        ("open", vec![what])
    } else if cfg!(windows) {
        ("explorer", vec![what])
    } else {
        ("xdg-open", vec![what])
    }
}

/// Opens `target` with the platform's opener, without waiting for it. A
/// folder that does not exist yet is made first.
pub fn open_external(target: &External) -> Result<(), String> {
    if let External::Url(url) = target
        && !(url.starts_with("https://") || url.starts_with("http://"))
    {
        return Err(format!("`{url}` is not a web address"));
    }
    let absolute;
    let target = match target {
        External::Folder(folder) => {
            std::fs::create_dir_all(folder)
                .map_err(|e| format!("cannot make `{}`: {e}", folder.display()))?;
            absolute = External::Folder(
                std::path::absolute(folder)
                    .map_err(|e| format!("cannot open `{}`: {e}", folder.display()))?,
            );
            &absolute
        }
        url => url,
    };
    let (program, args) = opener(target);
    let mut command = std::process::Command::new(program);
    // Explorer reads its own command line and takes a comma for a
    // separator, unless the path is in quotes: given as it is, quoted
    // (a Windows path cannot hold a `"`).
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        for arg in &args {
            command.raw_arg(format!("\"{}\"", arg.replace('"', "")));
        }
    }
    #[cfg(not(windows))]
    command.args(&args);
    let mut child = command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot start `{program}`: {e}"))?;
    // Reaped when it ends, so no process is left behind as a zombie.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

impl Host {
    /// Opens what the Help menu asked to open, and stores the recent
    /// files when they changed. True when something was done.
    pub(crate) fn pump_places(&mut self) -> bool {
        let mut did = false;
        if let Some(target) = self.shell.take_external() {
            let label = match &target {
                External::Url(_) => "open link",
                External::Folder(_) => "open folder",
            };
            let outcome = open_external(&target).map(|()| serde_json::json!({"opened": true}));
            self.report(label, &outcome);
            did = true;
        }
        if self.shell.take_recent_changed().is_some()
            && let Some(file) = self.recent_file.clone()
        {
            let ours = self.shell.recent_list().clone();
            match store_recent(&file, &ours, &self.recent_baseline) {
                Ok(merged) => {
                    if merged != ours {
                        self.shell.set_recent_files(merged.clone());
                    }
                    self.recent_baseline = merged;
                }
                Err(e) => {
                    self.report_notice("recent files", &e);
                    did = true;
                }
            }
        }
        did
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workdir(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../target/tmp/craft3d-places-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_recent_files_go_beside_the_recovery_directory() {
        let env = |name: &str| match name {
            "HOME" => Some("/home/me".to_string()),
            "XDG_STATE_HOME" | "LOCALAPPDATA" => Some("/state".to_string()),
            _ => None,
        };
        let recovery = craft3d_engine::default_recovery_directory(&env).unwrap();
        let file = recent_file_beside(&recovery).unwrap();
        assert_eq!(file.parent(), recovery.parent());
        assert_eq!(file.file_name().unwrap(), RECENT_FILE);
        assert_eq!(recent_file_beside(Path::new("rec")), None);
    }

    fn list(paths: &[&str]) -> RecentFiles {
        let mut list = RecentFiles::default();
        for path in paths.iter().rev() {
            list.note(path);
        }
        list
    }

    #[test]
    fn a_list_is_written_whole_and_a_bad_one_reads_as_empty() {
        let dir = workdir("rw");
        let file = dir.join("deeper").join(RECENT_FILE);
        assert_eq!(read_recent(&file), RecentFiles::default(), "no file yet");
        let ours = list(&["/w/a.3dcraft"]);
        let written = store_recent(&file, &ours, &RecentFiles::default()).unwrap();
        assert_eq!(written, ours);
        assert_eq!(read_recent(&file), ours);
        // No temporary file is left behind.
        let left: Vec<_> = std::fs::read_dir(dir.join("deeper")).unwrap().collect();
        assert_eq!(left.len(), 1);
        for bad in ["", "{{{{", "[1, 2]", "\u{feff}garbage"] {
            std::fs::write(&file, bad).unwrap();
            assert_eq!(read_recent(&file), RecentFiles::default(), "{bad:?}");
        }
        std::fs::write(&file, vec![b'x'; 70 * 1024]).unwrap();
        assert_eq!(read_recent(&file), RecentFiles::default(), "too long");
        // A directory where the file should be.
        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir_all(&file).unwrap();
        assert_eq!(read_recent(&file), RecentFiles::default());
        assert!(store_recent(&file, &ours, &RecentFiles::default()).is_err());
    }

    #[test]
    fn what_another_window_added_is_kept_after_ours() {
        let dir = workdir("merge");
        let file = dir.join(RECENT_FILE);
        // Both windows started from [a]; the other one added b and wrote.
        let baseline = list(&["/w/a.3dcraft"]);
        store_recent(&file, &list(&["/w/b.3dcraft", "/w/a.3dcraft"]), &baseline).unwrap();
        // This one added c: c first, then a, then the other window's b.
        let ours = list(&["/w/c.3dcraft", "/w/a.3dcraft"]);
        let merged = store_recent(&file, &ours, &baseline).unwrap();
        assert_eq!(
            merged.paths(),
            ["/w/c.3dcraft", "/w/a.3dcraft", "/w/b.3dcraft"]
        );
        assert_eq!(read_recent(&file), merged);
        // What this window cleared stays cleared when nothing new came.
        let cleared = store_recent(&file, &RecentFiles::default(), &merged).unwrap();
        assert_eq!(cleared, RecentFiles::default());
        // At most ten in all.
        let many: Vec<String> = (0..10).map(|i| format!("/w/{i}.3dcraft")).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        let theirs = list(&["/w/x.3dcraft"]);
        assert_eq!(
            merge_recent(&list(&many), &RecentFiles::default(), &theirs)
                .paths()
                .len(),
            10
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_link_where_the_temporary_file_would_go_is_not_followed() {
        let dir = workdir("link");
        let file = dir.join(RECENT_FILE);
        let target = dir.join("victim.txt");
        std::fs::write(&target, "keep").unwrap();
        // A link at every name the next temporary file could have.
        let next = TEMPORARIES.load(std::sync::atomic::Ordering::Relaxed);
        for count in next..next + 64 {
            let name = format!("{RECENT_FILE}.{}-{count}.tmp", std::process::id());
            std::os::unix::fs::symlink(&target, dir.join(name)).unwrap();
        }
        let ours = list(&["/w/a.3dcraft"]);
        // Refused or written beside the link, never through it.
        let _ = store_recent(&file, &ours, &RecentFiles::default());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep");
    }

    #[test]
    fn the_data_folder_is_the_apps_own_folder_only() {
        let env = |name: &str| match name {
            "HOME" => Some("/home/me".to_string()),
            "XDG_STATE_HOME" | "LOCALAPPDATA" => Some("/state".to_string()),
            _ => None,
        };
        let recovery = craft3d_engine::default_recovery_directory(&env).unwrap();
        let own = recovery.parent().unwrap().to_path_buf();
        let mut options = Options::default();
        assert_eq!(data_folder(&options, &env), None);
        options.recovery_dir = Some(PathBuf::from("/tmp/mine/recovery"));
        options.recent_file = Some(PathBuf::from("/tmp/mine/list.json"));
        assert_eq!(
            data_folder(&options, &env),
            None,
            "given folders: not offered"
        );
        options.recovery_dir = Some(recovery.clone());
        assert_eq!(data_folder(&options, &env), Some(own.clone()));
        options.recovery_dir = None;
        options.recent_file = Some(own.join(RECENT_FILE));
        assert_eq!(data_folder(&options, &env), Some(own));
        assert_eq!(data_folder(&options, &|_| None), None);
    }

    #[test]
    fn links_and_folders_go_to_the_platforms_opener() {
        let (program, args) = opener(&External::Url("https://example.org".to_string()));
        assert!(["open", "explorer", "xdg-open"].contains(&program));
        assert_eq!(args, ["https://example.org"]);
        let (_, args) = opener(&External::Folder(PathBuf::from("/s/a,b")));
        assert_eq!(args, ["/s/a,b"], "one argument, comma and all");
        // Only web addresses are handed to the opener as links.
        assert!(open_external(&External::Url("file:///etc/passwd".to_string())).is_err());
        assert!(open_external(&External::Url("--help".to_string())).is_err());
    }

    #[test]
    fn the_agent_setup_names_this_windows_port_and_an_absolute_token_file() {
        let env = |name: &str| match name {
            "HOME" => Some("/home/me".to_string()),
            "APPDATA" => Some(r"C:\Users\me\AppData\Roaming".to_string()),
            _ => None,
        };
        let setup = agent_setup(Some(7901), Some(Path::new("tok/control.token")), &env);
        assert!(setup.listening());
        assert!(Path::new(setup.token_file.as_deref().unwrap()).is_absolute());
        assert!(setup.cli.contains("craft3d-cli"));
        let idle = agent_setup(None, None, &env);
        assert!(!idle.listening());
        // A token file to suggest, spelled out: never `~`.
        let suggested = idle.suggested_token.clone().unwrap();
        assert!(!suggested.contains('~'), "{suggested}");
        assert!(!idle.mcp_config().contains('~'));
        assert_eq!(suggested_token_file(&|_| None), None);
        assert_eq!(
            suggested_token_file(&|n| (n == "HOME").then(|| "relative".to_string())),
            None
        );
    }
}
