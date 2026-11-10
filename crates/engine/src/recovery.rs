//! Autosave and crash recovery: what a killed 3DCraft gives back.
//!
//! # Design
//!
//! **Where.** A session is given a *recovery directory* by its host
//! ([`Session::enable_recovery`]); without one nothing here does anything.
//! Every launch (one store, one process) picks a random 16-digit key and
//! owns three kinds of files in that directory:
//!
//! ```text
//! <key>.lock        held with an operating-system file lock while the launch lives
//! <key>-<n>.3dcraft document n of that launch, written by the normal writer
//! <key>-<n>.json    its record: what the file belongs to (below)
//! ```
//!
//! **When written.** After every command the session compares its open
//! documents with what the store holds (`Session::execute`; O(documents),
//! no disk). A document with unsaved changes hands the store an immutable
//! snapshot (an `Arc` of the document: no copy). The store writes it when
//! the interval (default 30 s) has passed since that document's last
//! write, or since its first unwritten change when it was never written:
//! a burst of commands gives one write, and at most one interval of work
//! is lost. After a caught panic the last good snapshot is written at
//! once. Serialising and writing happen on the store's own thread, never
//! on the command's; the file is written whole to a temporary name,
//! synced and renamed, so a crash in mid-write leaves the previous
//! recovery file. The `.3dcraft` bytes are exactly those of `file.save`;
//! everything that varies (time, process) is in the record.
//!
//! **When removed.** As soon as the document has no unsaved changes
//! (it was saved), when it is closed, and when the session ends cleanly
//! ([`Session::end_recovery`]). Removal happens on the command's thread,
//! behind the lock that serialises it with a write in progress: when
//! `file.save` returns, the recovery file is gone. Dropping a session
//! without ending it leaves the files, as a crash does. (An undo or redo
//! back to the very state saved counts as saved, as `file.list`'s `dirty`
//! does: `OpenDocument::dirty`.)
//!
//! **Dead or alive.** A launch is alive exactly while it holds the lock
//! on its `<key>.lock`. The operating system releases the lock when the
//! process ends, however it ends, so there is no process id to look up
//! and none to be reused. A scan tries each other launch's lock: got it,
//! or there is no lock file, means dead and its documents are listed;
//! "would block" means alive and they are left alone, by `recovery.open`
//! and `recovery.discard` as well. A lock file that may not be opened
//! counts as alive; a lock that fails for another reason as dead, since
//! then nothing says the launch lives and nothing is opened or removed
//! without being asked. Limits: on a file system without working locks
//! (some network mounts) every other launch looks dead, a running one
//! included; a lock file deleted by hand under a live launch makes it
//! look dead. The process id in the record is information for a person,
//! not a check, so a reused process id cannot mislead.
//!
//! **Recovering.** `recovery.open` loads the file, opens it as an unsaved
//! document that remembers its original path, and *adopts* the file: it
//! is renamed to this launch's key, so it is this launch's autosave of
//! that document from then on and goes away when the document is saved
//! or closed. A recovered document keeps everything a saved file keeps
//! (the whole design history, so it regenerates to the same bodies); it
//! loses the undo stack, which belongs to the session and is in no file.
//!
//! **Overwrite guard.** The record holds the size and modification time
//! the original file had when the document was opened or last saved.
//! `file.save` of a recovered document to that path refuses when the
//! file there is no longer that one, and names `"overwrite": true`.
//!
//! **Failures.** A recovery directory that cannot be written (missing,
//! read-only, full disk) is a problem in [`RecoveryOverview::problem`],
//! shown by `recovery.list`, `ui.get` and the status bar until a write
//! succeeds; it is retried once per interval and never fails a command.

use crate::Session;
#[cfg(not(target_arch = "wasm32"))]
use crate::{CommandError, OpenDocument};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

/// How long after a change a document is autosaved, by default.
pub const DEFAULT_AUTOSAVE_INTERVAL: Duration = Duration::from_secs(30);
/// The shortest and longest interval a host may ask for.
pub const MIN_AUTOSAVE_INTERVAL: Duration = Duration::from_millis(10);
pub const MAX_AUTOSAVE_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// The time, as the store sees it: a parameter, so tests move it by hand.
pub trait Clock: Send + Sync {
    /// Time since some fixed moment. Never goes back.
    fn monotonic(&self) -> Duration;
    /// Seconds since the Unix epoch, for the records.
    fn epoch_seconds(&self) -> u64;
}

/// The system's clocks.
#[cfg(not(target_arch = "wasm32"))]
pub struct SystemClock {
    start: std::time::Instant,
}

#[cfg(not(target_arch = "wasm32"))]
impl SystemClock {
    pub fn new() -> Self {
        Self {
            // native clock: `SystemClock` is cfg(not(target_arch = "wasm32")).
            start: std::time::Instant::now(),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Clock for SystemClock {
    fn monotonic(&self) -> Duration {
        self.start.elapsed()
    }

    fn epoch_seconds(&self) -> u64 {
        // native clock: `SystemClock` is cfg(not(target_arch = "wasm32")).
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    }
}

/// A clock that stands still until it is moved: for tests.
pub struct ManualClock {
    epoch: u64,
    elapsed: Mutex<Duration>,
}

impl ManualClock {
    /// A clock at `epoch` seconds since the Unix epoch.
    pub fn new(epoch: u64) -> Self {
        Self {
            epoch,
            elapsed: Mutex::new(Duration::ZERO),
        }
    }

    pub fn advance(&self, by: Duration) {
        let mut elapsed = self.elapsed.lock().unwrap_or_else(PoisonError::into_inner);
        *elapsed = elapsed.saturating_add(by);
    }
}

impl Clock for ManualClock {
    fn monotonic(&self) -> Duration {
        *self.elapsed.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn epoch_seconds(&self) -> u64 {
        self.epoch.saturating_add(self.monotonic().as_secs())
    }
}

/// Called when the autosave state changed (a write, a problem), from the
/// store's thread: a window uses it to repaint.
pub type RecoveryNotify = Arc<dyn Fn() + Send + Sync>;

/// How a session autosaves.
#[derive(Clone)]
pub struct RecoveryOptions {
    /// The recovery directory. Created when the first file is written.
    pub directory: PathBuf,
    /// How long after a change a document is written.
    pub interval: Duration,
    pub clock: Arc<dyn Clock>,
    /// Write on a thread of the store's own. Without it, what is due is
    /// written by the command that finds it due and by
    /// [`Session::autosave_poll`]: for tests with a [`ManualClock`].
    pub background: bool,
    pub notify: Option<RecoveryNotify>,
}

impl RecoveryOptions {
    /// Autosave into `directory` every [`DEFAULT_AUTOSAVE_INTERVAL`], on a
    /// background thread, by the system's clock.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
            interval: DEFAULT_AUTOSAVE_INTERVAL,
            clock: Arc::new(SystemClock::new()),
            background: true,
            notify: None,
        }
    }

    /// Autosave into `directory` by a clock the caller moves, with no
    /// thread: nothing is written but inside a call on the session.
    pub fn manual(directory: impl Into<PathBuf>, clock: Arc<dyn Clock>) -> Self {
        Self {
            directory: directory.into(),
            interval: DEFAULT_AUTOSAVE_INTERVAL,
            clock,
            background: false,
            notify: None,
        }
    }

    pub fn with_interval(mut self, interval: Duration) -> Self {
        self.interval = interval.clamp(MIN_AUTOSAVE_INTERVAL, MAX_AUTOSAVE_INTERVAL);
        self
    }

    pub fn with_notify(mut self, notify: RecoveryNotify) -> Self {
        self.notify = Some(notify);
        self
    }
}

/// An autosave interval as a flag or an environment variable gives it:
/// seconds, fractions allowed.
pub fn parse_autosave_interval(text: &str) -> Result<Duration, String> {
    let shown: String = text.chars().take(40).collect();
    let seconds = text
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|s| s.is_finite() && *s > 0.0)
        .ok_or_else(|| format!("`{shown}` is not a number of seconds above 0"))?;
    let (min, max) = (
        MIN_AUTOSAVE_INTERVAL.as_secs_f64(),
        MAX_AUTOSAVE_INTERVAL.as_secs_f64(),
    );
    if !(min..=max).contains(&seconds) {
        return Err(format!(
            "`{shown}` is out of range: from {min} to {max} seconds"
        ));
    }
    Ok(Duration::from_secs_f64(seconds))
}

/// Where the desktop app keeps its recovery files when it is not told:
/// `3dcraft/recovery` in the platform's per-user state or data directory.
/// `env` looks up an environment variable. `None` when the platform's
/// variables give no place.
///
/// - Linux and other Unix: `$XDG_STATE_HOME`, else `$HOME/.local/state`
/// - macOS: `$HOME/Library/Application Support`
/// - Windows: `%LOCALAPPDATA%`
pub fn default_recovery_directory(env: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let var = |name: &str| {
        env(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            // A relative base would move with the working directory.
            .filter(|path| path.is_absolute())
    };
    let base = if cfg!(target_os = "windows") {
        var("LOCALAPPDATA")?
    } else if cfg!(target_os = "macos") {
        var("HOME")?.join("Library").join("Application Support")
    } else {
        match var("XDG_STATE_HOME") {
            Some(state) => state,
            None => var("HOME")?.join(".local").join("state"),
        }
    };
    Some(base.join("3dcraft").join("recovery"))
}

/// What a file looked like on disk: enough to tell that it was replaced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FileStamp {
    pub size: u64,
    /// Seconds and nanoseconds since the Unix epoch, where the file
    /// system has a modification time.
    pub modified: Option<(u64, u32)>,
}

impl FileStamp {
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn of(path: &std::path::Path) -> Option<Self> {
        let metadata = std::fs::metadata(path).ok()?;
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|since| (since.as_secs(), since.subsec_nanos()));
        Some(Self {
            size: metadata.len(),
            modified,
        })
    }
}

/// What makes an open document a recovered one, until it is saved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Recovered {
    /// The original file as the crashed launch knew it; `None` when it
    /// had none or the record did not say.
    pub original: Option<FileStamp>,
}

/// A document a previous launch left behind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recoverable {
    /// What `recovery.open` and `recovery.discard` take.
    pub id: String,
    /// The file name of the original, or `untitled`.
    pub name: String,
    /// The path the document was opened from or last saved to, as that
    /// launch's request gave it.
    pub path: Option<String>,
    /// When it was autosaved, in seconds since the Unix epoch.
    pub time: Option<u64>,
    /// The size of the recovery file in bytes.
    pub size: Option<u64>,
    /// The document's revision in that launch when it was written: how
    /// many changes had been made to it since it was opened.
    pub revision: Option<u64>,
    pub features: Option<usize>,
    /// The process that wrote it. Information only.
    pub pid: Option<u32>,
    /// False when the document cannot be read at all.
    pub openable: bool,
    /// What is wrong with it: why it cannot be opened, or what is missing
    /// although it can.
    pub problem: Option<String>,
}

/// The state of autosave and recovery in a session.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct RecoveryOverview {
    /// False when the session has no recovery directory.
    pub enabled: bool,
    pub directory: Option<String>,
    pub interval_seconds: f64,
    /// When a document was last autosaved, in seconds since the Unix
    /// epoch.
    pub last_autosave: Option<u64>,
    /// How long that write took, serialising included, in milliseconds.
    pub last_write_ms: Option<f64>,
    /// How many autosave writes this session made.
    pub writes: u64,
    /// Why autosave does not work, while it does not.
    pub problem: Option<String>,
    /// Documents with changes that are not in a recovery file yet.
    pub pending: usize,
    /// What the last scan found of launches that are gone.
    pub found: Vec<Recoverable>,
    /// How many other launches are running on the same directory.
    pub running: usize,
}

/// A session's store. A clone of a session is an independent session and
/// gets none: two sessions never share one launch key.
#[derive(Default)]
pub(crate) struct RecoveryHandle {
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) store: Option<crate::recovery_store::Store>,
}

impl Clone for RecoveryHandle {
    fn clone(&self) -> Self {
        Self::default()
    }
}

/// Whether a document is worth a recovery file: it has changes that are
/// not on disk, and it is not an empty document that was never saved. An
/// example the program carries has none until it is edited
/// (`open_bytes.rs`).
#[cfg(not(target_arch = "wasm32"))]
fn worth(open: &OpenDocument) -> bool {
    let document = open.history.current();
    // A Jig in use whose parameters changed: its values are kept too.
    open.parameters_changed()
        || (open.dirty()
            && (open.path.is_some()
                || !document.features.is_empty()
                || !document.variables.is_empty()
                || document.assembly.as_ref().is_some_and(|a| !a.is_empty())))
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) const DISABLED: &str =
    "this session has no recovery directory; start it with one (`--recovery-dir DIR`)";

#[cfg(not(target_arch = "wasm32"))]
impl Session {
    /// Starts autosaving into `options.directory`, and looks there for
    /// what earlier launches left behind ([`Self::recovery_overview`]).
    /// Nothing is written until a document has unsaved changes. Replaces
    /// (and ends) a store the session already had.
    pub fn enable_recovery(&mut self, options: RecoveryOptions) {
        self.end_recovery();
        let store = crate::recovery_store::Store::new(options);
        store.scan();
        self.recovery.store = Some(store);
        self.recovery_sync(false);
    }

    /// The clean end: stops the autosave thread and removes this launch's
    /// recovery files. Call it when the session's owner quits on purpose;
    /// a session that is merely dropped leaves its files to be recovered.
    pub fn end_recovery(&mut self) {
        if let Some(store) = self.recovery.store.take() {
            store.end();
        }
    }

    /// Writes every document that has unwritten changes now instead of
    /// when its interval has passed (on the store's thread when it has
    /// one). The session does this itself after a caught panic.
    pub fn autosave_now(&mut self) {
        self.recovery_sync(true);
    }

    /// Notes what changed and, in a session without an autosave thread,
    /// writes what is due by the clock.
    pub fn autosave_poll(&mut self) {
        self.recovery_sync(false);
    }

    /// Autosave and recovery as they are now. The list of recoverable
    /// documents is the one of the last scan (`enable_recovery`, and the
    /// `recovery.*` commands): no disk is read here.
    pub fn recovery_overview(&self) -> RecoveryOverview {
        self.recovery
            .store
            .as_ref()
            .map(crate::recovery_store::Store::overview)
            .unwrap_or_default()
    }

    /// Brings the store up to date with the open documents.
    pub(crate) fn recovery_sync(&mut self, urgent: bool) {
        let Some(store) = &self.recovery.store else {
            return;
        };
        let views: Vec<crate::recovery_store::DocView<'_>> = self
            .documents
            .iter()
            .map(|open| crate::recovery_store::DocView {
                id: open.id,
                revision: open.history.revision(),
                worth: worth(open),
                document: open.history.current(),
                path: open.path.as_deref(),
                title: open.title.as_deref(),
                original: open.disk,
                recovered: open.recovered.is_some(),
            })
            .collect();
        store.sync(&views, urgent);
    }

    pub(crate) fn recovery_store(
        &self,
        command: &str,
    ) -> Result<&crate::recovery_store::Store, CommandError> {
        self.recovery
            .store
            .as_ref()
            .ok_or_else(|| CommandError::new(format!("{command}: {DISABLED}")))
    }
}

/// There are no files, so there is no recovery, in a browser.
#[cfg(target_arch = "wasm32")]
impl Session {
    pub fn enable_recovery(&mut self, _options: RecoveryOptions) {}

    pub fn end_recovery(&mut self) {}

    pub fn autosave_now(&mut self) {}

    pub fn autosave_poll(&mut self) {}

    pub fn recovery_overview(&self) -> RecoveryOverview {
        RecoveryOverview::default()
    }

    pub(crate) fn recovery_sync(&mut self, _urgent: bool) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_directory_follows_the_platform() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| (*value).to_string())
            }
        };
        assert_eq!(default_recovery_directory(&env(&[])), None);
        // A relative or empty base is no place.
        assert_eq!(
            default_recovery_directory(&env(&[
                ("HOME", "home"),
                ("XDG_STATE_HOME", ""),
                ("LOCALAPPDATA", "data")
            ])),
            None
        );
        if cfg!(all(unix, not(target_os = "macos"))) {
            assert_eq!(
                default_recovery_directory(&env(&[("HOME", "/home/ann")])),
                Some(PathBuf::from("/home/ann/.local/state/3dcraft/recovery"))
            );
            assert_eq!(
                default_recovery_directory(&env(&[
                    ("HOME", "/home/ann"),
                    ("XDG_STATE_HOME", "/var/state")
                ])),
                Some(PathBuf::from("/var/state/3dcraft/recovery"))
            );
        }
    }

    #[test]
    fn an_interval_is_seconds_within_range() {
        assert_eq!(parse_autosave_interval("30"), Ok(Duration::from_secs(30)));
        assert_eq!(
            parse_autosave_interval(" 0.25 "),
            Ok(Duration::from_millis(250))
        );
        for bad in ["", "soon", "0", "-1", "nan", "inf", "1e9", "0.0001"] {
            let error = parse_autosave_interval(bad).unwrap_err();
            assert!(error.contains(bad), "{bad}: {error}");
        }
    }

    #[test]
    fn a_manual_clock_moves_only_when_moved() {
        let clock = ManualClock::new(1_000);
        assert_eq!(clock.monotonic(), Duration::ZERO);
        clock.advance(Duration::from_millis(2_500));
        assert_eq!(clock.monotonic(), Duration::from_millis(2_500));
        assert_eq!(clock.epoch_seconds(), 1_002);
    }
}
