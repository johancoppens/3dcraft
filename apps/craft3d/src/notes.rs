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
//! (a test's, an agent's: `--recovery-dir DIR` or `none`) has no such
//! file: it offers nothing and writes nothing. That is also true of a
//! person who starts 3DCraft with a recovery directory of their own: the
//! card never comes up by itself there, and Help › What's new has the
//! notes.
//!
//! **What tells an update from a first start** ([`EARLIER_TRACES`]). The
//! 0.1 releases kept no note; what they left in the state folder is the
//! autosaves' directory and the recent files. Only those two count, and
//! they are the two things by which a window keeps the note: so a window
//! that keeps no note cannot leave a trace, and the person's first own
//! window after an agent's or a test's is a first start. The control
//! token, the remembered switch for agents and the discovery entries are
//! no traces: a window that keeps no note writes them too (until
//! 2026-11-15 the token counted, and a person's first window after an
//! agent's showed *New in …* with nothing old). A 0.1 user who never
//! opened or saved a file and never had an autosave left neither, and
//! gets no card: the safe side.
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
/// What an earlier 3DCraft left in its state folder, and what only a
/// window that keeps the note writes there ([`seen_file_for`]): the
/// recent files and the autosaves.
pub const EARLIER_TRACES: [&str; 2] = [crate::places::RECENT_FILE, RECOVERY_DIR];

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

/// True when `options` make a window write one of the [`EARLIER_TRACES`]
/// into its state folder: its autosaves go to `recovery` there, or its
/// recent files to `recent-files.json` there, by default or by name.
pub fn leaves_traces(options: &Options) -> bool {
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
/// keeps its autosaves or its recent files there ([`leaves_traces`]),
/// else nowhere.
pub fn seen_file_for(options: &Options) -> Option<PathBuf> {
    let state = options.state_dir.as_ref()?;
    leaves_traces(options).then(|| state.join(SEEN_FILE))
}

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

    /// What a window that keeps no note can write into the state folder
    /// is no trace of an earlier 3DCraft: the control token (which counted
    /// until 2026-11-15, so a person's first window after an agent's
    /// showed the card with nothing old), the remembered switch for
    /// agents, the discovery entries.
    #[test]
    fn what_a_window_without_a_note_writes_is_no_trace() {
        for name in ["control.token", "agents.json", "windows"] {
            assert!(!EARLIER_TRACES.contains(&name), "{name}");
            let dir = workdir("no-trace");
            if name == "windows" {
                std::fs::create_dir_all(dir.join(name).join("1234")).unwrap();
            } else {
                std::fs::write(dir.join(name), "{}").unwrap();
            }
            let file = dir.join(SEEN_FILE);
            assert_eq!(read_seen(&file), Seen::Never, "{name}");
            assert_eq!(seen_at_start(Some(&file), v("0.2.0")), Seen::Never);
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

    /// The traces are exactly what only a window that keeps the note
    /// writes: however a window is started, it keeps the note when, and
    /// only when, its autosaves or its recent files go into the state
    /// folder, by default or by a path that names the same place.
    #[test]
    fn a_window_keeps_the_note_exactly_when_it_can_leave_a_trace() {
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
            assert_eq!(leaves_traces(&options), *keeps, "{extra:?}");
            assert_eq!(
                options.seen_file,
                keeps.then(|| state.join(SEEN_FILE)),
                "{extra:?}"
            );
            assert_eq!(seen_file_for(&options), options.seen_file, "{extra:?}");
            // Without the note, neither place a trace lies in is one the
            // window writes to.
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
            assert!(leaves_traces(&options));
        }
        // No state folder at all: no note, whatever else is given.
        let options = parsed(&["--recovery-dir", &elsewhere]);
        assert_eq!((options.state_dir, options.seen_file), (None, None));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The reviewer's finding, the whole way: an agent's (or a test's)
    /// window runs first in a fresh state folder and keeps no note, with
    /// its control port open, a file noted as recent, an autosave, and the
    /// person turning the agents' switch in it. It leaves its token, its
    /// discovery entry and the remembered switch, and no trace. The
    /// person's own first window after it is a first start: nothing is
    /// offered, and this version is written as seen. Before the fix the
    /// token alone made it "an earlier 3DCraft", and the card came up.
    #[test]
    fn a_persons_first_window_after_an_agents_window_is_a_first_start() {
        use craft3d_engine::{Session, Workspace};
        let running = craft3d_engine::running_version().unwrap();
        for own in ["none", "elsewhere"] {
            let dir = workdir("after-agent");
            let state = dir.join("state");
            let s = state.to_str().unwrap().to_string();
            let autosaves = dir.join("agent-recovery");
            let recovery_dir = if own == "none" {
                "none".to_string()
            } else {
                autosaves.to_str().unwrap().to_string()
            };
            let options = parsed(&[
                "--state-dir",
                &s,
                "--recovery-dir",
                &recovery_dir,
                "--control",
                "0",
            ]);
            assert_eq!(options.seen_file, None, "{own}");
            assert_eq!(
                seen_at_start(options.seen_file.as_deref(), running),
                Seen::Never
            );
            // The window's own writers, as `app::run` starts them.
            let wake: crate::control::Wake = std::sync::Arc::new(|| {});
            let env = |_: &str| None;
            let (agents, _requests) =
                crate::app::control(&options, std::sync::Arc::clone(&wake), &env).unwrap();
            assert!(agents.address().is_some(), "{own}: it listens");
            let mut session = Session::new(Workspace::memory());
            if let Some(recovery) = crate::app::recovery(&options, wake).unwrap() {
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
            // Its list of recent files changes: kept for the run only.
            host.shell.clear_recent_files();
            host.pump_places();
            // The person turns the agents' switch in that window.
            craft3d_automation::discovery::write_preference(&state, true).unwrap();
            drop(agents);
            // What it left: its token and the switch, and no trace.
            assert!(state.join("control.token").is_file(), "{own}");
            assert!(state.join("agents.json").is_file(), "{own}");
            for trace in EARLIER_TRACES {
                assert!(!state.join(trace).exists(), "{own}: it left {trace}");
            }
            assert!(!state.join(SEEN_FILE).exists(), "{own}: and no note");
            // The person's own first window.
            let person = parsed(&["--state-dir", &s]);
            let file = person.seen_file.clone().unwrap();
            assert_eq!(read_seen(&file), Seen::Never, "{own}");
            assert_eq!(seen_at_start(Some(&file), running), Seen::Never, "{own}");
            let notes = WhatsNew::for_start(&changelog(), running, Seen::Never).unwrap();
            assert!(!notes.offered, "{own}");
            assert_eq!(read_seen(&file), Seen::Version(running), "{own}");
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
