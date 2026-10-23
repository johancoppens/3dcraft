//! What the window keeps and opens outside a document: the list of recent
//! files on disk, and the pages and folders the Help menu opens with the
//! platform's own opener (the browser, the file manager).
//!
//! The recent files live in `recent-files.json` next to the recovery
//! directory, in the person's state directory (`--recent-file`). The file
//! is read as hostile input (a corrupt one is an empty list, never an
//! error) and written whole through a temporary file, so a crash leaves
//! the old list or the new one.

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

/// Writes `text` to `path` whole: to a temporary file beside it first,
/// then renamed over it.
pub fn write_recent(path: &Path, text: &str) -> Result<(), String> {
    let shown = path.display();
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot make the folder of `{shown}`: {e}"))?;
    }
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    std::fs::write(&temporary, text).map_err(|e| format!("cannot write `{shown}`: {e}"))?;
    std::fs::rename(&temporary, path).map_err(|e| {
        let _ = std::fs::remove_file(&temporary);
        format!("cannot write `{shown}`: {e}")
    })
}

/// The folder Help, Open data folder shows: the one the recent files are
/// kept in, else the recovery directory; `None` when the window keeps
/// nothing on disk.
pub fn data_folder(options: &Options) -> Option<PathBuf> {
    options
        .recent_file
        .as_deref()
        .and_then(Path::parent)
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .or_else(|| options.recovery_dir.clone())
}

/// How an agent reaches this window: `craft3d-cli` beside this program
/// when it is there (as the installers put it), else by its name on the
/// `PATH`; the port the window listens on and its token file, absolute.
pub fn agent_setup(port: Option<u16>, token_file: Option<&Path>) -> AgentSetup {
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
    let mut child = std::process::Command::new(program)
        .args(&args)
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
        if let Some(text) = self.shell.take_recent_changed()
            && let Some(file) = self.recent_file.clone()
            && let Err(e) = write_recent(&file, &text)
        {
            self.report("recent files", &Err(e));
            did = true;
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

    #[test]
    fn a_list_is_written_whole_and_a_bad_one_reads_as_empty() {
        let dir = workdir("rw");
        let file = dir.join("deeper").join(RECENT_FILE);
        assert_eq!(read_recent(&file), RecentFiles::default(), "no file yet");
        let mut list = RecentFiles::default();
        list.note("/w/a.3dcraft");
        write_recent(&file, &list.to_json()).unwrap();
        assert_eq!(read_recent(&file), list);
        assert!(!dir.join("deeper").join("recent-files.json.tmp").exists());
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
        assert!(write_recent(&file, "{}").is_err());
    }

    #[test]
    fn the_data_folder_is_where_the_window_keeps_things() {
        let mut options = Options::default();
        assert_eq!(data_folder(&options), None);
        options.recovery_dir = Some(PathBuf::from("/s/3dcraft/recovery"));
        assert_eq!(
            data_folder(&options),
            Some(PathBuf::from("/s/3dcraft/recovery"))
        );
        options.recent_file = Some(PathBuf::from("/s/3dcraft/recent-files.json"));
        assert_eq!(data_folder(&options), Some(PathBuf::from("/s/3dcraft")));
    }

    #[test]
    fn links_and_folders_go_to_the_platforms_opener() {
        let (program, args) = opener(&External::Url("https://example.org".to_string()));
        assert!(["open", "explorer", "xdg-open"].contains(&program));
        assert_eq!(args, ["https://example.org"]);
        let (_, args) = opener(&External::Folder(PathBuf::from("/s/3dcraft")));
        assert_eq!(args.len(), 1);
        // Only web addresses are handed to the opener as links.
        assert!(open_external(&External::Url("file:///etc/passwd".to_string())).is_err());
        assert!(open_external(&External::Url("--help".to_string())).is_err());
    }

    #[test]
    fn the_agent_setup_names_this_windows_port_and_an_absolute_token_file() {
        let setup = agent_setup(Some(7901), Some(Path::new("tok/control.token")));
        assert!(setup.listening());
        assert!(Path::new(setup.token_file.as_deref().unwrap()).is_absolute());
        assert!(setup.cli.contains("craft3d-cli"));
        assert!(!agent_setup(None, None).listening());
    }
}
