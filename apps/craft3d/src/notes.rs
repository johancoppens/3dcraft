//! What's new, on the desktop: which version's release notes the person
//! has seen, kept where the window's other preferences are (the state
//! folder, beside `recent-files.json` and `agents.json`), and what the
//! window therefore shows at this start (the card *New in …* on the start
//! screen after an update; Help › What's new always).
//!
//! `whats-new.json` holds `{"seen": "0.2.0"}`: the newest version whose
//! notes were closed here. `{"seen": null}` stands for an earlier 3DCraft
//! that kept no such note (the 0.1 releases), found by what it left in the
//! state folder. No file and nothing of an earlier 3DCraft is a first
//! start: nothing is old there, so nothing is offered, and this version is
//! written as seen at once.
//!
//! The file is read as hostile input (a corrupt one is a first start,
//! never an error) and written whole through a temporary file of this
//! process's own. **The window offers nothing it cannot remember
//! closing:** before the card is offered the file is written once as it
//! stands; where that fails (a folder that cannot be written), nothing is
//! offered, rather than the same card at every start.
//!
//! A window that keeps its autosaves anywhere but the default place (a
//! test's, an agent's: `--recovery-dir DIR` or `none`) has no such file
//! (`args.rs`): it offers nothing and writes nothing, as it keeps no
//! recent files.

use crate::host::Host;
use craft3d_engine::changelog::{Seen, Version};
use craft3d_ui_egui::shell::WhatsNew;
use std::path::{Path, PathBuf};

/// The name of the file in the state folder.
pub const SEEN_FILE: &str = "whats-new.json";
/// Most bytes of it read.
const MAX_SEEN_BYTES: u64 = 4096;
/// What an earlier 3DCraft left in its state folder: the recent files,
/// the remembered switch for agents, the control token, the autosaves.
const EARLIER_TRACES: [&str; 4] = [
    crate::places::RECENT_FILE,
    "agents.json",
    "control.token",
    "recovery",
];

/// What the file at `path` says of the notes seen; without a file, what
/// its folder says: [`Seen::Earlier`] when an earlier 3DCraft left
/// something there, else [`Seen::Never`]. A file that does not read is a
/// first start.
pub fn read_seen(path: &Path) -> Seen {
    let Ok(metadata) = std::fs::metadata(path) else {
        let earlier = path
            .parent()
            .is_some_and(|folder| EARLIER_TRACES.iter().any(|name| folder.join(name).exists()));
        return if earlier { Seen::Earlier } else { Seen::Never };
    };
    if !metadata.is_file() || metadata.len() > MAX_SEEN_BYTES {
        return Seen::Never;
    }
    let value = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok());
    match value.as_ref().and_then(|value| value.get("seen")) {
        Some(serde_json::Value::Null) => Seen::Earlier,
        Some(serde_json::Value::String(version)) => {
            Version::parse(version).map_or(Seen::Never, Seen::Version)
        }
        _ => Seen::Never,
    }
}

/// Counts the temporary files this process made.
static TEMPORARIES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Writes `seen` to `path` (`None`: an earlier 3DCraft, which version is
/// not known), through a temporary file of this process's own beside it,
/// renamed over the file.
pub fn store_seen(path: &Path, seen: Option<Version>) -> Result<(), String> {
    use std::io::Write;
    let shown = path.display();
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot make the folder of `{shown}`: {e}"))?;
    }
    let text = serde_json::json!({ "seen": seen.map(|version| version.to_string()) }).to_string();
    let count = TEMPORARIES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(format!(".{}-{count}.tmp", std::process::id()));
    let temporary = PathBuf::from(temporary);
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .and_then(|mut file| file.write_all(text.as_bytes()));
    if let Err(e) = written {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!("cannot write `{shown}`: {e}"));
    }
    std::fs::rename(&temporary, path).map_err(|e| {
        let _ = std::fs::remove_file(&temporary);
        format!("cannot write `{shown}`: {e}")
    })
}

/// What is known of the notes seen as a program of version `running`
/// starts, with the file at `path` (`None`: the window keeps none). Also
/// makes sure the answer can be remembered: a first start writes
/// `running` as seen, and before anything is offered the file is written
/// as it stands; where no file is kept or it cannot be written, the answer
/// is [`Seen::Never`], so nothing is offered.
///
/// Call it before anything else writes into the state folder: a first
/// start is told from an update by what an earlier 3DCraft left there.
pub fn seen_at_start(path: Option<&Path>, running: Version) -> Seen {
    let Some(path) = path else {
        return Seen::Never;
    };
    let seen = read_seen(path);
    let kept = match seen {
        Seen::Never => store_seen(path, Some(running)),
        Seen::Earlier => store_seen(path, None),
        // Something to offer: written again, to know it can be.
        Seen::Version(version) if version < running => store_seen(path, Some(version)),
        // The same version, or a newer one (a downgrade): the newest seen
        // stays what it is.
        Seen::Version(_) => Ok(()),
    };
    if kept.is_err() { Seen::Never } else { seen }
}

/// The notes the window shows at this start: what is new since `seen`,
/// offered on the start screen, or this version's own for the Help menu.
/// `None` when the program carries no notes.
pub fn at_start(seen: Seen) -> Option<WhatsNew> {
    let notes = craft3d_engine::release_notes()?;
    let running = craft3d_engine::running_version()?;
    WhatsNew::for_start(notes, running, seen)
}

impl Host {
    /// Remembers this version's notes as seen once the person closed
    /// them. True when something was written or could not be.
    pub(crate) fn pump_notes(&mut self) -> bool {
        if !self.shell.take_whats_new_closed() {
            return false;
        }
        let (Some(path), Some(running)) =
            (self.seen_file.clone(), craft3d_engine::running_version())
        else {
            return false;
        };
        // A newer version seen here before (a downgrade) stays.
        let newest = match read_seen(&path) {
            Seen::Version(version) if version > running => version,
            _ => running,
        };
        if let Err(e) = store_seen(&path, Some(newest)) {
            self.report_notice("what's new", &e);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workdir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "craft3d-notes-{name}-{}-{}",
            std::process::id(),
            TEMPORARIES.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    #[test]
    fn a_first_start_is_seen_never_and_writes_this_version() {
        let dir = workdir("first");
        let file = dir.join("state").join(SEEN_FILE);
        assert_eq!(read_seen(&file), Seen::Never);
        assert_eq!(seen_at_start(Some(&file), v("0.2.0")), Seen::Never);
        // The next start: the same version, seen.
        assert_eq!(read_seen(&file), Seen::Version(v("0.2.0")));
        assert_eq!(
            seen_at_start(Some(&file), v("0.2.0")),
            Seen::Version(v("0.2.0"))
        );
        // And the one after an update: the version before.
        assert_eq!(
            seen_at_start(Some(&file), v("0.3.0")),
            Seen::Version(v("0.2.0"))
        );
        assert_eq!(read_seen(&file), Seen::Version(v("0.2.0")), "until closed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn what_an_earlier_3dcraft_left_behind_tells_an_update_from_a_first_start() {
        for trace in EARLIER_TRACES {
            let dir = workdir("earlier");
            if trace == "recovery" {
                std::fs::create_dir_all(dir.join(trace)).unwrap();
            } else {
                std::fs::write(dir.join(trace), "{}").unwrap();
            }
            let file = dir.join(SEEN_FILE);
            assert_eq!(read_seen(&file), Seen::Earlier, "{trace}");
            assert_eq!(seen_at_start(Some(&file), v("0.2.0")), Seen::Earlier);
            // Written as it stands: still an earlier one until closed.
            assert_eq!(std::fs::read_to_string(&file).unwrap(), r#"{"seen":null}"#);
            assert_eq!(seen_at_start(Some(&file), v("0.2.0")), Seen::Earlier);
            let _ = std::fs::remove_dir_all(&dir);
        }
        // Another program's file there is not 3DCraft's.
        let dir = workdir("other");
        std::fs::write(dir.join("notes.txt"), "x").unwrap();
        assert_eq!(read_seen(&dir.join(SEEN_FILE)), Seen::Never);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_downgrade_keeps_the_newest_version_seen() {
        let dir = workdir("downgrade");
        let file = dir.join(SEEN_FILE);
        store_seen(&file, Some(v("0.3.0"))).unwrap();
        assert_eq!(
            seen_at_start(Some(&file), v("0.2.0")),
            Seen::Version(v("0.3.0"))
        );
        assert_eq!(read_seen(&file), Seen::Version(v("0.3.0")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_that_does_not_read_is_a_first_start_and_never_an_error() {
        let dir = workdir("junk");
        let file = dir.join(SEEN_FILE);
        let long = format!("{{\"seen\": \"{}\"}}", "1".repeat(10_000));
        for junk in [
            "",
            "null",
            "[]",
            "{}",
            "{{{{",
            "{\"seen\": 7}",
            "{\"seen\": \"soon\"}",
            "{\"seen\": \"1.2\"}",
            "{\"seen\": [\"0.1.0\"]}",
            "\u{0}\u{1}",
            long.as_str(),
        ] {
            std::fs::write(&file, junk).unwrap();
            assert_eq!(read_seen(&file), Seen::Never, "{junk:?}");
        }
        // A folder of that name, too.
        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
        assert_eq!(read_seen(&file), Seen::Never);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn without_a_file_or_where_it_cannot_be_written_nothing_is_offered() {
        // No file kept (a test's or an agent's window).
        assert_eq!(seen_at_start(None, v("0.2.0")), Seen::Never);
        // A place that cannot be written: the folder is a file.
        let dir = workdir("blocked");
        let blocker = dir.join("state");
        std::fs::write(&blocker, "x").unwrap();
        let file = blocker.join(SEEN_FILE);
        assert!(store_seen(&file, Some(v("0.2.0"))).is_err());
        assert_eq!(seen_at_start(Some(&file), v("0.2.0")), Seen::Never);
        // An update whose file is read but cannot be written again.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let locked = dir.join("locked");
            std::fs::create_dir_all(&locked).unwrap();
            let file = locked.join(SEEN_FILE);
            store_seen(&file, Some(v("0.1.1"))).unwrap();
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
            // Root writes anywhere: the check holds only where the folder
            // really refuses.
            let refused = std::fs::write(locked.join("probe"), "x").is_err();
            if refused {
                assert_eq!(read_seen(&file), Seen::Version(v("0.1.1")));
                assert_eq!(seen_at_start(Some(&file), v("0.2.0")), Seen::Never);
            }
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn what_is_stored_reads_back() {
        let dir = workdir("round");
        let file = dir.join(SEEN_FILE);
        store_seen(&file, Some(v("1.20.300"))).unwrap();
        assert_eq!(read_seen(&file), Seen::Version(v("1.20.300")));
        store_seen(&file, None).unwrap();
        assert_eq!(read_seen(&file), Seen::Earlier);
        // No temporary file is left beside it.
        let left: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(left, [std::ffi::OsString::from(SEEN_FILE)]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
