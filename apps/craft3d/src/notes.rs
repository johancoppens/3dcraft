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
//! **Who keeps the note** ([`seen_file_for`]). A window keeps it exactly
//! when it keeps its autosaves or its recent files in the state folder:
//! the person's own window. One that keeps both elsewhere or not at all
//! (a test's, an agent's: `--recovery-dir DIR` or `none`) offers nothing.
//! That is also true of a person who starts 3DCraft with a recovery
//! directory of their own: the card never comes up by itself there, and
//! Help › What's new has the notes.
//!
//! **What tells an update from a first start** ([`EARLIER_TRACES`]). The
//! 0.1 releases kept no note; what they left in the state folder is the
//! recent files, the remembered switch for agents (`agents.json`), the
//! control token and the autosaves' directory. Each counts: a 0.1 user
//! who only let agents open and check files left the switch and the token
//! alone (an agent's file is not noted as recent, and an autosave comes
//! only 30 s after an unsaved change), and is one whose proofs may have
//! changed. A 0.1 user who left nothing gets no card: the safe side.
//!
//! **A window that keeps no note on a fresh folder** ([`note_a_fresh_folder`]).
//! Such a window still writes its token, the switch and its discovery
//! entry into the state folder. So that these are never taken for a 0.1
//! user's, it writes this version as seen there first, before anything
//! else, where the folder holds neither a note nor a trace; a folder that
//! holds either is not written (the person's own folder keeps what it has).
//! The person's first own window after it is a first start. (Until
//! 2026-11-15 the token alone made it "an update"; the first fix took the
//! token and the switch out of the traces, which lost the 0.1 user above.)
//!
//! **Two windows at once on a fresh folder.** A window that keeps no note
//! writes the note before its token, each through a rename. A window that
//! keeps the note reads the note, then looks for traces, and where it
//! finds one reads the note once more ([`read_seen`]): a trace it sees
//! that such a window wrote comes after that window's note, so the second
//! reading finds the note. Two windows that keep no note write the same
//! version, the later rename over the earlier. A window that keeps the
//! note and finds nothing writes this version, which is what the other
//! writes too.
//!
//! At a start the note is read and, before anything is offered, written
//! back as it was read. A close in another window between the two (a few
//! microseconds) is undone by that; the card then comes up once more.

use crate::args::Options;
use crate::host::Host;
use craft3d_engine::changelog::{Seen, Version};
use craft3d_ui_egui::shell::WhatsNew;
use std::path::{Component, Path, PathBuf};

/// The name of the file in the state folder.
pub const SEEN_FILE: &str = "whats-new.json";
/// The name of the autosaves' directory in the state folder.
pub const RECOVERY_DIR: &str = "recovery";
/// Most bytes of it read.
const MAX_SEEN_BYTES: u64 = 4096;
/// What an earlier 3DCraft left in its state folder: the recent files,
/// the remembered switch for agents, the control token, the autosaves.
pub const EARLIER_TRACES: [&str; 4] = [
    crate::places::RECENT_FILE,
    "agents.json",
    "control.token",
    RECOVERY_DIR,
];

/// `path` as one place, however it is written: absolute, without `.` and
/// `..`, and through links where the file, or the folder it would be made
/// in, is there.
fn place(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut plain = PathBuf::new();
    for part in absolute.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                plain.pop();
            }
            other => plain.push(other),
        }
    }
    if let Ok(real) = plain.canonicalize() {
        return real;
    }
    match (plain.parent().map(Path::canonicalize), plain.file_name()) {
        (Some(Ok(parent)), Some(name)) => parent.join(name),
        _ => plain,
    }
}

/// True when `options` make a window keep its autosaves (`recovery`) or
/// its recent files (`recent-files.json`) in its state folder, by default
/// or by name: the person's own window.
pub fn keeps_its_files_here(options: &Options) -> bool {
    let Some(state) = &options.state_dir else {
        return false;
    };
    let is = |given: Option<&Path>, name: &str| {
        given.is_some_and(|given| place(given) == place(&state.join(name)))
    };
    is(options.recovery_dir.as_deref(), RECOVERY_DIR)
        || is(options.recent_file.as_deref(), crate::places::RECENT_FILE)
}

/// Where a window started with `options` keeps the newest version whose
/// release notes were seen: `whats-new.json` in its state folder when it
/// keeps its autosaves or its recent files there ([`keeps_its_files_here`]),
/// else nowhere.
pub fn seen_file_for(options: &Options) -> Option<PathBuf> {
    let state = options.state_dir.as_ref()?;
    keeps_its_files_here(options).then(|| state.join(SEEN_FILE))
}

/// True when `folder` holds one of the [`EARLIER_TRACES`].
fn has_traces(folder: &Path) -> bool {
    EARLIER_TRACES
        .iter()
        .any(|name| std::fs::symlink_metadata(folder.join(name)).is_ok())
}

/// What the file at `path` says of the notes seen; without a file, what
/// its folder says: [`Seen::Earlier`] when an earlier 3DCraft left
/// something there, else [`Seen::Never`]. A file that does not read is a
/// first start. Where a trace is found the file is looked for once more:
/// a window that keeps no note writes the note before its token, so a
/// token that came while the folder was looked at comes with its note.
pub fn read_seen(path: &Path) -> Seen {
    if let Some(seen) = read_note(path) {
        return seen;
    }
    if !path.parent().is_some_and(has_traces) {
        return Seen::Never;
    }
    read_note(path).unwrap_or(Seen::Earlier)
}

/// What the file at `path` says, or `None` where there is no file.
fn read_note(path: &Path) -> Option<Seen> {
    let Ok(metadata) = std::fs::metadata(path) else {
        return None;
    };
    Some(read_note_there(path, &metadata))
}

/// What the file at `path`, which is there, says.
fn read_note_there(path: &Path, metadata: &std::fs::Metadata) -> Seen {
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

/// For a window that keeps no note, started on `state`: where that folder
/// holds neither a note nor one of the [`EARLIER_TRACES`], writes
/// `running` there as seen, so that what the window writes next (its
/// token, the switch for agents, its discovery entry) is never taken for
/// an earlier 3DCraft's. A folder that holds a note or a trace is not
/// written. True when the note was written.
///
/// Call it before the window writes anything into the state folder.
pub fn note_a_fresh_folder(state: &Path, running: Version) -> bool {
    let file = state.join(SEEN_FILE);
    if std::fs::symlink_metadata(&file).is_ok() || has_traces(state) {
        return false;
    }
    store_seen(&file, Some(running)).is_ok()
}

/// What a window started with `options` knows of the notes seen as it
/// starts: [`seen_at_start`] for a window that keeps the note; for one
/// that keeps none, [`Seen::Never`], after [`note_a_fresh_folder`]. The
/// first thing the window does with its state folder.
pub fn seen_at_window_start(options: &Options, running: Version) -> Seen {
    match (&options.seen_file, &options.state_dir) {
        (Some(file), _) => seen_at_start(Some(file), running),
        (None, Some(state)) => {
            note_a_fresh_folder(state, running);
            Seen::Never
        }
        (None, None) => Seen::Never,
    }
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

    /// Whether the card is offered at a start that knows `seen`.
    fn offered(seen: Seen) -> bool {
        let running = craft3d_engine::running_version().unwrap();
        WhatsNew::for_start(&changelog(), running, seen).is_some_and(|notes| notes.offered)
    }

    /// Every file and folder under `dir`, with what a file holds and when
    /// it was changed: to see that nothing there was touched.
    fn snapshot(dir: &Path) -> Vec<(PathBuf, Option<Vec<u8>>, std::time::SystemTime)> {
        let mut all = Vec::new();
        let mut open = vec![dir.to_path_buf()];
        while let Some(at) = open.pop() {
            for entry in std::fs::read_dir(&at).unwrap() {
                let path = entry.unwrap().path();
                let meta = std::fs::symlink_metadata(&path).unwrap();
                let bytes = meta.is_file().then(|| std::fs::read(&path).unwrap());
                if meta.is_dir() {
                    open.push(path.clone());
                }
                all.push((path, bytes, meta.modified().unwrap()));
            }
        }
        all.sort();
        all
    }

    /// Makes `name` in `dir` as a 0.1 release left it.
    fn leave(dir: &Path, name: &str) {
        match name {
            "recovery" | "windows" => {
                std::fs::create_dir_all(dir.join(name)).unwrap();
                std::fs::write(dir.join(name).join("1234.json"), "{}").unwrap();
            }
            "agents.json" => std::fs::write(dir.join(name), r#"{"allowAgents": true}"#).unwrap(),
            "control.token" => {
                // Private, as a window writes it.
                let path = dir.join(name);
                std::fs::write(&path, "0".repeat(64)).unwrap();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                        .unwrap();
                }
            }
            _ => std::fs::write(dir.join(name), r#"{"recent": []}"#).unwrap(),
        }
    }

    /// Each thing a 0.1 release left in the person's state folder, alone,
    /// gives the card once: offered at the first start, and not again once
    /// it was closed. The discovery entries alone are no trace.
    #[test]
    fn each_trace_of_0_1_alone_gives_the_card_once() {
        let running = craft3d_engine::running_version().unwrap();
        for trace in EARLIER_TRACES.into_iter().chain(["windows"]) {
            let dir = workdir("alone");
            leave(&dir, trace);
            let s = dir.to_str().unwrap().to_string();
            let person = parsed(&["--state-dir", &s]);
            let seen = seen_at_window_start(&person, running);
            let is_trace = trace != "windows";
            assert_eq!(seen == Seen::Earlier, is_trace, "{trace}: {seen:?}");
            assert_eq!(offered(seen), is_trace, "{trace}");
            // The person closes it (as `pump_notes` writes it).
            store_seen(person.seen_file.as_deref().unwrap(), Some(running)).unwrap();
            let again = seen_at_window_start(&person, running);
            assert_eq!(again, Seen::Version(running), "{trace}");
            assert!(!offered(again), "{trace}: once");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// The finding of round 2's review (review-whats-new-r2), turned to
    /// what is right: a 0.1 user who only let agents open and check files
    /// left the switch and the token alone (0.1 notes no agent's file as
    /// recent, and an autosave waits 30 s after an unsaved change), and
    /// gets the card. Round 2 counted neither, and gave none.
    #[test]
    fn a_01_user_who_only_let_agents_check_files_gets_the_card() {
        let dir = workdir("agent-only");
        leave(&dir, "agents.json");
        leave(&dir, "control.token");
        let file = dir.join(SEEN_FILE);
        assert_eq!(read_seen(&file), Seen::Earlier);
        assert_eq!(seen_at_start(Some(&file), v("0.2.0")), Seen::Earlier);
        assert!(offered(Seen::Earlier));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A window that keeps no note writes one only into a folder that holds
    /// neither a note nor a trace. A folder shaped as the developer's own
    /// (the switch, the token, recent files, autosaves, discovery entries),
    /// with a note or without, with a note that does not read, or with one
    /// trace alone, is left exactly as it was, byte for byte and time for
    /// time; a fresh folder, or one with only discovery entries, gets this
    /// version as seen.
    #[test]
    fn a_window_without_a_note_never_writes_a_folder_with_a_note_or_a_trace() {
        let running = craft3d_engine::running_version().unwrap();
        let notes: [Option<&str>; 5] = [
            None,
            Some(r#"{"seen":"0.1.2"}"#),
            Some(r#"{"seen":null}"#),
            Some("not a note"),
            Some(""),
        ];
        for note in notes {
            let dir = workdir("persons");
            for trace in EARLIER_TRACES.into_iter().chain(["windows"]) {
                leave(&dir, trace);
            }
            if let Some(note) = note {
                std::fs::write(dir.join(SEEN_FILE), note).unwrap();
            }
            let before = snapshot(&dir);
            assert!(!note_a_fresh_folder(&dir, running), "{note:?}");
            let s = dir.to_str().unwrap().to_string();
            for extra in [
                &["--recovery-dir", "none"][..],
                &["--recovery-dir", "none", "--control", "0"],
            ] {
                let mut list = vec!["--state-dir", s.as_str()];
                list.extend(extra);
                assert_eq!(seen_at_window_start(&parsed(&list), running), Seen::Never);
            }
            assert_eq!(snapshot(&dir), before, "{note:?}");
            let _ = std::fs::remove_dir_all(&dir);
        }
        for trace in EARLIER_TRACES {
            let dir = workdir("one-trace");
            leave(&dir, trace);
            let before = snapshot(&dir);
            assert!(!note_a_fresh_folder(&dir, running), "{trace}");
            assert_eq!(snapshot(&dir), before, "{trace}");
            let _ = std::fs::remove_dir_all(&dir);
        }
        // Fresh: no folder yet, an empty one, one with discovery entries.
        for fresh in ["missing", "empty", "windows"] {
            let dir = workdir("fresh");
            let state = dir.join("state");
            if fresh != "missing" {
                std::fs::create_dir_all(&state).unwrap();
            }
            if fresh == "windows" {
                leave(&state, "windows");
            }
            assert!(note_a_fresh_folder(&state, running), "{fresh}");
            assert_eq!(read_seen(&state.join(SEEN_FILE)), Seen::Version(running));
            // Written once: the next window finds a note and leaves it.
            assert!(!note_a_fresh_folder(&state, running), "{fresh}");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    fn parsed(list: &[&str]) -> Options {
        let none = |_: &str| None;
        match crate::args::parse(list.iter().map(|s| (*s).to_string()), &none) {
            Ok(crate::args::Parsed::Run(options)) => options,
            other => panic!("{list:?}: {other:?}"),
        }
    }

    /// However a window is started, it keeps the note when, and only when,
    /// its autosaves or its recent files go into the state folder, by
    /// default or by a path that names the same place.
    #[test]
    fn a_window_keeps_the_note_exactly_when_it_keeps_its_files_in_the_state_folder() {
        let dir = workdir("who");
        let state = dir.join("state");
        std::fs::create_dir_all(&state).unwrap();
        let s = state.to_str().unwrap().to_string();
        let join = |name: &str| state.join(name).to_str().unwrap().to_string();
        let elsewhere = dir.join("elsewhere").to_str().unwrap().to_string();
        // (the options after `--state-dir STATE`, keeps the note)
        let around = format!("{s}/../state/recovery");
        let dotted = format!("{s}/./recent-files.json");
        let cases: Vec<(Vec<&str>, bool)> = vec![
            (vec![], true),
            (vec!["--control", "0"], true),
            (vec!["--recovery-dir", "none"], false),
            (vec!["--recovery-dir", elsewhere.as_str()], false),
            (vec!["--recovery-dir", "none", "--control", "0"], false),
            (
                vec!["--recovery-dir", "none", "--recent-file", "none"],
                false,
            ),
            (
                vec![
                    "--recovery-dir",
                    elsewhere.as_str(),
                    "--recent-file",
                    elsewhere.as_str(),
                ],
                false,
            ),
            (vec!["--recent-file", "none"], true),
            // Named, the same places as the defaults.
            (vec!["--recovery-dir", around.as_str()], true),
            (
                vec!["--recovery-dir", "none", "--recent-file", dotted.as_str()],
                true,
            ),
        ];
        let recovery = join(RECOVERY_DIR);
        let recent = join(crate::places::RECENT_FILE);
        for (extra, keeps) in &cases {
            let mut list = vec!["--state-dir", s.as_str()];
            list.extend(extra.iter().copied());
            let options = parsed(&list);
            assert_eq!(keeps_its_files_here(&options), *keeps, "{extra:?}");
            assert_eq!(
                options.seen_file,
                keeps.then(|| state.join(SEEN_FILE)),
                "{extra:?}"
            );
            assert_eq!(seen_file_for(&options), options.seen_file, "{extra:?}");
            // Without the note, it keeps neither its autosaves nor its
            // recent files in the state folder.
            if !keeps {
                for (given, trace) in [
                    (options.recovery_dir.as_deref(), &recovery),
                    (options.recent_file.as_deref(), &recent),
                ] {
                    assert!(
                        given.is_none_or(|given| place(given) != place(Path::new(trace))),
                        "{extra:?}: {given:?}"
                    );
                }
            }
        }
        // Through a link to the state folder, too.
        #[cfg(unix)]
        {
            let link = dir.join("link");
            std::os::unix::fs::symlink(&state, &link).unwrap();
            let through = link.join(RECOVERY_DIR).to_str().unwrap().to_string();
            let options = parsed(&["--state-dir", &s, "--recovery-dir", &through]);
            assert!(keeps_its_files_here(&options));
        }
        // No state folder at all: no note, whatever else is given.
        let options = parsed(&["--recovery-dir", &elsewhere]);
        assert_eq!((options.state_dir, options.seen_file), (None, None));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// How a window that keeps no note runs on `state`, as `app::run`
    /// starts it: the note first, then its control port (`--control`), its
    /// autosaves and recent files, a document modelled and autosaved, the
    /// person turning the agents' switch in it; `crash` ends it without
    /// stopping its listener.
    fn run_a_window_without_a_note(options: &Options, crash: bool) {
        use craft3d_engine::{Session, Workspace};
        let running = craft3d_engine::running_version().unwrap();
        assert_eq!(options.seen_file, None);
        assert_eq!(seen_at_window_start(options, running), Seen::Never);
        let wake: crate::control::Wake = std::sync::Arc::new(|| {});
        let env = |_: &str| None;
        let (agents, _requests) =
            crate::app::control(options, std::sync::Arc::clone(&wake), &env).unwrap();
        let mut session = Session::new(Workspace::memory());
        if let Some(recovery) = crate::app::recovery(options, wake).unwrap() {
            session.enable_recovery(recovery);
        }
        session.execute("file.new", &serde_json::json!({})).unwrap();
        session
            .execute("feature.block", &serde_json::json!({"size": [1, 2, 3]}))
            .unwrap();
        session.autosave_now();
        let mut host = Host::new(session);
        host.recent_file.clone_from(&options.recent_file);
        host.seen_file.clone_from(&options.seen_file);
        host.shell.clear_recent_files();
        host.pump_places();
        if let Some(state) = &options.state_dir {
            craft3d_automation::discovery::write_preference(state, true).unwrap();
        }
        if crash {
            std::mem::forget(agents);
        } else {
            drop(agents);
        }
    }

    /// A person's first own window after one that keeps no note ran first
    /// on a fresh state folder: an agent's (its control port open, with
    /// autosaves of its own or none), a test's (no port), one that crashed,
    /// one whose autosaves go inside the state folder's `recovery` (the
    /// reviewer's contrived hole, turned), and the CLI's bridge looking for
    /// a window there. Each left its token, the switch or its autosaves
    /// only after the note, or nothing at all; the person's first window
    /// offers nothing. (Until 2026-11-15 the token alone gave the card.)
    #[test]
    fn a_persons_first_window_after_an_agents_a_tests_a_crashed_one_or_the_cli_is_a_first_start() {
        let running = craft3d_engine::running_version().unwrap();
        for case in [
            "agent",
            "agent-own-autosaves",
            "test",
            "crashed",
            "inside",
            "cli",
        ] {
            let dir = workdir("after");
            let state = dir.join("state");
            let s = state.to_str().unwrap().to_string();
            let own = dir.join("agent-recovery").to_str().unwrap().to_string();
            let inside = state
                .join(RECOVERY_DIR)
                .join("agent")
                .to_str()
                .unwrap()
                .to_string();
            match case {
                "agent" => run_a_window_without_a_note(
                    &parsed(&[
                        "--state-dir",
                        &s,
                        "--recovery-dir",
                        "none",
                        "--control",
                        "0",
                    ]),
                    false,
                ),
                "agent-own-autosaves" => run_a_window_without_a_note(
                    &parsed(&["--state-dir", &s, "--recovery-dir", &own, "--control", "0"]),
                    false,
                ),
                "test" => run_a_window_without_a_note(
                    &parsed(&["--state-dir", &s, "--recovery-dir", "none"]),
                    false,
                ),
                "crashed" => run_a_window_without_a_note(
                    &parsed(&["--state-dir", &s, "--recovery-dir", &own, "--control", "0"]),
                    true,
                ),
                "inside" => {
                    run_a_window_without_a_note(
                        &parsed(&["--state-dir", &s, "--recovery-dir", &inside]),
                        false,
                    );
                    assert!(
                        state.join(RECOVERY_DIR).is_dir(),
                        "its autosave made the trace"
                    );
                }
                _ => {
                    // `craft3d-cli mcp --bridge --state-dir STATE`: it reads
                    // the folder and writes nothing there.
                    let initialize = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
                        "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                                   "clientInfo": {"name": "test", "version": "1"}}});
                    let call = serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
                        "params": {"name": "command_run", "arguments": {"id": "session.whatsNew", "params": {}}}});
                    std::fs::create_dir_all(&state).unwrap();
                    let mut output = Vec::new();
                    craft3d_automation::bridge::serve(
                        &craft3d_automation::bridge::BridgeConfig::discover(state.clone(), None),
                        format!("{initialize}\n{call}\n").as_bytes(),
                        &mut output,
                    )
                    .unwrap();
                    assert!(!output.is_empty());
                    assert_eq!(snapshot(&state), Vec::new(), "the CLI wrote nothing");
                }
            }
            // The person's own first window.
            let person = parsed(&["--state-dir", &s]);
            let seen = seen_at_window_start(&person, running);
            assert!(
                matches!(seen, Seen::Never) || seen == Seen::Version(running),
                "{case}: {seen:?}"
            );
            assert!(!offered(seen), "{case}");
            assert_eq!(
                read_seen(person.seen_file.as_deref().unwrap()),
                Seen::Version(running),
                "{case}"
            );
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// A 0.1 user who only let agents check files left the switch and the
    /// token; then a 0.2 agent's window ran first there, then the person's
    /// own: the agent's window does not note a folder with traces, so the
    /// person gets the card.
    #[test]
    fn a_01_folder_then_an_agents_window_then_the_persons_gives_the_card() {
        let running = craft3d_engine::running_version().unwrap();
        let dir = workdir("01-then-agent");
        leave(&dir, "agents.json");
        leave(&dir, "control.token");
        let s = dir.to_str().unwrap().to_string();
        run_a_window_without_a_note(
            &parsed(&[
                "--state-dir",
                &s,
                "--recovery-dir",
                "none",
                "--control",
                "0",
            ]),
            false,
        );
        assert!(
            !dir.join(SEEN_FILE).exists(),
            "no note in a folder with traces"
        );
        let person = parsed(&["--state-dir", &s]);
        let seen = seen_at_window_start(&person, running);
        assert_eq!(seen, Seen::Earlier);
        assert!(offered(seen));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Two windows starting at once on a fresh state folder, many times
    /// over: one that keeps no note (it notes the folder, then writes its
    /// token) beside the person's (it reads the note, looks for traces,
    /// and reads the note again where it finds one); and two that keep no
    /// note. The person's window never takes the other's token for an
    /// earlier 3DCraft, and the folder ends with this version as seen.
    #[test]
    fn two_windows_starting_at_once_on_a_fresh_folder() {
        let running = craft3d_engine::running_version().unwrap();
        for round in 0..200 {
            let dir = workdir("at-once");
            let state = dir.join("state");
            std::fs::create_dir_all(&state).unwrap();
            let file = state.join(SEEN_FILE);
            let two_without_a_note = round % 2 == 1;
            let gate = std::sync::Barrier::new(2);
            let (seen, _) = std::thread::scope(|scope| {
                let agent = scope.spawn(|| {
                    gate.wait();
                    note_a_fresh_folder(&state, running);
                    std::fs::write(state.join("control.token"), "0".repeat(64)).unwrap();
                });
                let other = scope.spawn(|| {
                    gate.wait();
                    if two_without_a_note {
                        note_a_fresh_folder(&state, running);
                        std::fs::write(state.join("agents.json"), "{}").unwrap();
                        Seen::Never
                    } else {
                        seen_at_start(Some(&file), running)
                    }
                });
                (other.join().unwrap(), agent.join().unwrap())
            });
            assert_ne!(seen, Seen::Earlier, "round {round}");
            assert_eq!(read_seen(&file), Seen::Version(running), "round {round}");
            let left: Vec<_> = std::fs::read_dir(&state)
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .filter(|name| name.to_string_lossy().ends_with(".tmp"))
                .collect();
            assert!(left.is_empty(), "round {round}: {left:?}");
            let _ = std::fs::remove_dir_all(&dir);
        }
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

    /// A changelog for a program of this version: one release before it.
    fn changelog() -> craft3d_engine::changelog::Changelog {
        let running = craft3d_engine::VERSION;
        let text = format!(
            "## [{running}] - 2026-11-14\n\n### Results that changed\n\n- **A measured volume could be wrong.** It is right now.\n\n### Added\n\n- **Something new.**\n\n## [0.0.1] - 2026-01-01\n\n- The first release.\n"
        );
        craft3d_engine::changelog::Changelog::parse(&text).unwrap()
    }

    /// The window on its start screen: frames of the host's shell, and a
    /// click at a point.
    struct Frames {
        ctx: eframe::egui::Context,
        time: f64,
    }

    impl Frames {
        fn new() -> Self {
            Self {
                ctx: eframe::egui::Context::default(),
                time: 0.0,
            }
        }

        fn draw(&mut self, host: &mut Host, events: Vec<eframe::egui::Event>) {
            use eframe::egui;
            self.time += 1.0 / 60.0;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100.0, 760.0),
                )),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let Host { session, shell, .. } = host;
            let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
            output.textures_delta.clear();
        }

        fn click(&mut self, host: &mut Host, at: eframe::egui::Pos2) {
            use eframe::egui::{Event, Modifiers, PointerButton};
            self.draw(host, vec![Event::PointerMoved(at)]);
            for pressed in [true, false] {
                self.draw(
                    host,
                    vec![Event::PointerButton {
                        pos: at,
                        button: PointerButton::Primary,
                        pressed,
                        modifiers: Modifiers::NONE,
                    }],
                );
            }
            self.draw(host, Vec::new());
        }
    }

    /// The whole way round on the desktop: after an update the window
    /// offers the card; closed through `ui.set` (an agent's doing) nothing
    /// is remembered; closed by the person with its ×, this version is
    /// written as seen, and the next start offers nothing.
    #[test]
    fn the_person_closing_the_card_is_remembered_and_the_next_start_offers_nothing() {
        let dir = workdir("closing");
        let file = dir.join(SEEN_FILE);
        let running = craft3d_engine::running_version().unwrap();
        store_seen(&file, Some(v("0.0.1"))).unwrap();
        let start = |file: &Path| {
            let seen = seen_at_start(Some(file), running);
            WhatsNew::for_start(&changelog(), running, seen).unwrap()
        };
        let notes = start(&file);
        assert!(notes.offered);
        let mut host = Host::new(craft3d_engine::Session::new(
            craft3d_engine::Workspace::none(),
        ));
        host.seen_file = Some(file.clone());
        host.shell.set_whats_new(Some(notes));
        let mut frames = Frames::new();
        for _ in 0..4 {
            frames.draw(&mut host, Vec::new());
        }
        assert_eq!(host.shell.whats_new_shown(), Some("card"));
        // An agent closes it: the person has not seen it for that.
        host.shell.state.whats_new.card = false;
        frames.draw(&mut host, Vec::new());
        host.pump_places();
        assert_eq!(read_seen(&file), Seen::Version(v("0.0.1")));
        // The person closes it.
        host.shell.state.whats_new.card = true;
        for _ in 0..3 {
            frames.draw(&mut host, Vec::new());
        }
        let close = host
            .shell
            .whats_new_hits()
            .iter()
            .find(|(name, _)| name == "close")
            .map(|(_, rect)| *rect)
            .unwrap();
        frames.click(&mut host, close.center());
        assert_eq!(host.shell.whats_new_shown(), None);
        assert!(host.pump_places(), "something was written");
        assert_eq!(read_seen(&file), Seen::Version(running));
        assert!(!host.pump_places(), "once");
        // The next start: the same version, nothing offered; Help has it.
        let again = start(&file);
        assert!(!again.offered);
        // A newer version seen here before (a downgrade) is kept when the
        // notes are closed from Help.
        store_seen(&file, Some(v("999.0.0"))).unwrap();
        host.shell.state.whats_new.card = true;
        for _ in 0..3 {
            frames.draw(&mut host, Vec::new());
        }
        let close = host
            .shell
            .whats_new_hits()
            .iter()
            .find(|(name, _)| name == "close")
            .map(|(_, rect)| *rect)
            .unwrap();
        frames.click(&mut host, close.center());
        host.pump_places();
        assert_eq!(read_seen(&file), Seen::Version(v("999.0.0")));
        // A window that keeps no note writes none.
        let other = dir.join("other").join(SEEN_FILE);
        host.seen_file = None;
        host.shell.state.whats_new.card = true;
        for _ in 0..3 {
            frames.draw(&mut host, Vec::new());
        }
        let close = host
            .shell
            .whats_new_hits()
            .iter()
            .find(|(name, _)| name == "close")
            .map(|(_, rect)| *rect)
            .unwrap();
        frames.click(&mut host, close.center());
        assert!(!host.pump_places());
        assert!(!other.exists());
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
