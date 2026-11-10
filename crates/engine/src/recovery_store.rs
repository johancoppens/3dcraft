//! The recovery directory on disk: the files of this launch, the thread
//! that writes them, and what other launches left behind. The design is
//! in `recovery.rs`.
//!
//! Two locks. `disk` serialises everything that creates, renames or
//! deletes files of this launch; `state` guards what is pending. Whoever
//! needs both takes `disk` first. A command never waits for `state`
//! longer than a few instructions; it waits for `disk` only when it
//! removes a recovery file while that very store is writing one.

use crate::recovery::{
    Clock, FileStamp, Recoverable, RecoveryNotify, RecoveryOptions, RecoveryOverview,
};
use crate::workspace::write_atomic;
use craft3d_doc::Document;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

/// The version of the record's layout.
const RECORD_VERSION: u64 = 1;
/// A record larger than this is not read.
const MAX_RECORD_BYTES: u64 = 64 * 1024;
/// Most directory entries a scan looks at, and most documents it lists.
const MAX_DIR_ENTRIES: usize = 4096;
const MAX_LISTED: usize = 256;
/// Longest name and path taken from a record, in characters.
const MAX_NAME_CHARS: usize = 255;
const MAX_PATH_CHARS: usize = 4096;
const KEY_LEN: usize = 16;
const DOCUMENT_EXTENSION: &str = "3dcraft";

/// A key no other launch has: 16 hexadecimal digits from the standard
/// library's per-process random hasher, the process id and the time.
fn new_key() -> String {
    use std::hash::{BuildHasher, Hasher};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u32(std::process::id());
    hasher.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    if let Ok(now) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        hasher.write_u128(now.as_nanos());
    }
    format!("{:016x}", hasher.finish())
}

fn is_key(text: &str) -> bool {
    text.len() == KEY_LEN && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// `<key>-<n>` as `recovery.open` takes it.
fn parse_id(id: &str) -> Option<(&str, u64)> {
    let (key, number) = id.split_once('-')?;
    let plain =
        !number.is_empty() && number.len() <= 20 && number.bytes().all(|b| b.is_ascii_digit());
    (is_key(key) && plain)
        .then(|| number.parse::<u64>().ok())
        .flatten()
        .map(|number| (key, number))
}

/// What a file in the recovery directory is, by its name.
#[derive(Debug, PartialEq, Eq)]
enum Part {
    Lock,
    Document(u64),
    Record(u64),
    /// A temporary file of a write that did not finish.
    Temp,
}

fn parse_name(name: &str) -> Option<(&str, Part)> {
    if let Some(hidden) = name.strip_prefix('.') {
        let key = hidden.get(..KEY_LEN)?;
        return (is_key(key) && hidden.ends_with(".tmp")).then_some((key, Part::Temp));
    }
    let (stem, extension) = name.rsplit_once('.')?;
    if extension == "lock" {
        return is_key(stem).then_some((stem, Part::Lock));
    }
    let (key, number) = parse_id(stem)?;
    match extension {
        DOCUMENT_EXTENSION => Some((key, Part::Document(number))),
        "json" => Some((key, Part::Record(number))),
        _ => None,
    }
}

fn guard<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// An open document as the store needs to see it.
pub(crate) struct DocView<'a> {
    pub id: u64,
    pub revision: u64,
    /// It has unsaved changes worth a recovery file.
    pub worth: bool,
    pub document: &'a Arc<Document>,
    pub path: Option<&'a str>,
    /// The name it is shown under while it has no file
    /// (`OpenDocument::title`).
    pub title: Option<&'a str>,
    /// The document's own file when it was opened or last saved.
    pub original: Option<FileStamp>,
    pub recovered: bool,
}

/// A snapshot to write, with what its record says.
struct Job {
    revision: u64,
    document: Arc<Document>,
    path: Option<String>,
    title: Option<String>,
    original: Option<FileStamp>,
    recovered: bool,
}

/// What the store knows about one open document with unsaved changes.
struct Entry {
    /// The revision last handed over (written or pending).
    noted: Option<u64>,
    /// The start of the interval: the last write, or the first change
    /// that was not written.
    since: Duration,
    pending: Option<Job>,
    /// Write as soon as possible, whatever the interval.
    urgent: bool,
    /// When its recovery file was last written (seconds since the Unix
    /// epoch); `None` before the first.
    written: Option<u64>,
}

#[derive(Default)]
struct State {
    entries: BTreeMap<u64, Entry>,
    /// The document whose snapshot is being written right now.
    writing: Option<u64>,
    stop: bool,
    last_autosave: Option<u64>,
    last_write_ms: Option<f64>,
    writes: u64,
    problem: Option<String>,
    found: Vec<Recoverable>,
    running: usize,
}

/// What a record says.
#[derive(Default)]
struct Record {
    name: Option<String>,
    path: Option<String>,
    /// Since 2026-11-10, optional: a record written before has none.
    title: Option<String>,
    original: Option<FileStamp>,
    revision: Option<u64>,
    time: Option<u64>,
    pid: Option<u32>,
}

/// A document taken over from a dead launch.
pub(crate) struct Adopted {
    pub document: Document,
    pub name: String,
    pub path: Option<String>,
    /// The name it was shown under without a file (`OpenDocument::title`).
    pub title: Option<String>,
    pub original: Option<FileStamp>,
    /// Why the recovery file could not be made this launch's own; the
    /// document is open all the same.
    pub warning: Option<String>,
    /// What was changed on reading the document
    /// (`craft3d_format::load_noted`).
    pub read: Vec<String>,
}

/// The files of one launch, as a directory listing shows them.
#[derive(Default)]
struct Launch {
    /// Per document number: has a document file, has a record.
    documents: BTreeMap<u64, (bool, bool)>,
    temps: Vec<PathBuf>,
}

struct Shared {
    dir: PathBuf,
    key: String,
    interval: Duration,
    clock: Arc<dyn Clock>,
    notify: Option<RecoveryNotify>,
    state: Mutex<State>,
    wake: Condvar,
    /// The lock on this launch's lock file, once it is held.
    disk: Mutex<Option<File>>,
}

pub(crate) struct Store {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

/// The name a record gives its document: its file's, else the title it
/// was shown under, else `untitled`.
fn display_name(path: Option<&str>, title: Option<&str>) -> String {
    let name = path
        .and_then(|path| path.rsplit(['/', '\\']).find(|part| !part.is_empty()))
        .or(title)
        .unwrap_or("untitled");
    name.chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME_CHARS)
        .collect()
}

fn stamp_json(stamp: Option<FileStamp>) -> Value {
    stamp.map_or(Value::Null, |stamp| {
        json!({
            "size": stamp.size,
            "mtime": stamp.modified.map(|(seconds, nanos)| json!([seconds, nanos])),
        })
    })
}

fn stamp_from(value: Option<&Value>) -> Option<FileStamp> {
    let value = value?;
    let size = value.get("size")?.as_u64()?;
    let modified = value.get("mtime").and_then(|mtime| {
        let seconds = mtime.get(0)?.as_u64()?;
        let nanos = u32::try_from(mtime.get(1)?.as_u64()?).ok()?;
        Some((seconds, nanos))
    });
    Some(FileStamp { size, modified })
}

fn read_capped(path: &Path, max: u64) -> Result<Vec<u8>, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(max.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > max {
        return Err(format!("it is larger than {max} bytes"));
    }
    Ok(bytes)
}

fn read_record(path: &Path) -> Result<Record, String> {
    let bytes = read_capped(path, MAX_RECORD_BYTES)
        .map_err(|e| format!("its record cannot be read ({e})"))?;
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| "its record is damaged".to_string())?;
    let Some(version) = value.get("recovery_version").and_then(Value::as_u64) else {
        return Err("its record is damaged".to_string());
    };
    if version > RECORD_VERSION {
        return Err("its record was written by a newer 3DCraft".to_string());
    }
    let text = |key: &str, max: usize| {
        value
            .get(key)
            .and_then(Value::as_str)
            .map(|s| {
                s.chars()
                    .filter(|c| !c.is_control())
                    .take(max)
                    .collect::<String>()
            })
            .filter(|s| !s.is_empty())
    };
    Ok(Record {
        name: text("name", MAX_NAME_CHARS),
        path: text("path", MAX_PATH_CHARS),
        title: text("title", MAX_NAME_CHARS),
        original: stamp_from(value.get("original")),
        revision: value.get("revision").and_then(Value::as_u64),
        time: value.get("time").and_then(Value::as_u64),
        pid: value
            .get("pid")
            .and_then(Value::as_u64)
            .and_then(|pid| u32::try_from(pid).ok()),
    })
}

/// Whether the launch with this key is gone.
enum Liveness {
    /// Its process holds the lock.
    Alive,
    /// Nobody holds the lock. The file is our hold on it while we act;
    /// `None` when there is no lock file.
    Dead(Option<File>),
}

impl Shared {
    fn document_path(&self, key: &str, number: u64) -> PathBuf {
        self.dir
            .join(format!("{key}-{number}.{DOCUMENT_EXTENSION}"))
    }

    fn record_path(&self, key: &str, number: u64) -> PathBuf {
        self.dir.join(format!("{key}-{number}.json"))
    }

    fn lock_path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.lock"))
    }

    fn changed(&self) {
        if let Some(notify) = &self.notify {
            notify();
        }
    }

    /// The directory exists and this launch holds its lock. A file system
    /// that cannot lock is no reason not to save: the files are written
    /// without one (and another launch cannot tell this one is alive).
    fn ensure_ready(&self, lock: &mut Option<File>) -> Result<(), String> {
        fs::create_dir_all(&self.dir).map_err(|e| {
            format!(
                "cannot create the recovery directory `{}`: {e}",
                self.dir.display()
            )
        })?;
        let path = self.lock_path(&self.key);
        if lock.is_some() && path.exists() {
            return Ok(());
        }
        *lock = None;
        // A scan of another launch may hold the lock for an instant, and
        // remove the file when it found no documents next to it: wait for
        // it, and make the file again when it went.
        for _ in 0..4 {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&path)
                .map_err(|e| {
                    format!(
                        "cannot write in the recovery directory `{}`: {e}",
                        self.dir.display()
                    )
                })?;
            if file.lock().is_err() {
                return Ok(());
            }
            if path.exists() {
                *lock = Some(file);
                return Ok(());
            }
        }
        Ok(())
    }

    fn liveness(&self, key: &str) -> Liveness {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(self.lock_path(key));
        match file {
            Ok(file) => match file.try_lock() {
                Ok(()) => Liveness::Dead(Some(file)),
                Err(TryLockError::WouldBlock) => Liveness::Alive,
                // Locks do not work here: nothing says it is alive.
                Err(TryLockError::Error(_)) => Liveness::Dead(None),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Liveness::Dead(None),
            // A lock file we may not open is somebody else's.
            Err(_) => Liveness::Alive,
        }
    }

    fn record_json(&self, number: u64, job: &Job, features: usize, time: u64) -> Value {
        json!({
            "recovery_version": RECORD_VERSION,
            "launch": self.key,
            "pid": std::process::id(),
            "document": number,
            "name": display_name(job.path.as_deref(), job.title.as_deref()),
            "path": job.path,
            "title": job.title,
            "original": stamp_json(job.original),
            "revision": job.revision,
            "features": features,
            "time": time,
            "recovered": job.recovered,
        })
    }

    fn write_record(&self, number: u64, job: &Job, time: u64) -> Result<(), String> {
        let record = self.record_json(number, job, job.document.features.len(), time);
        let mut text = serde_json::to_vec_pretty(&record).map_err(|e| e.to_string())?;
        text.push(b'\n');
        write_atomic(&self.record_path(&self.key, number), &text)
            .map_err(|e| e.message().to_string())
    }

    /// Writes one snapshot. `Ok(false)` when the document no longer wants
    /// a recovery file by the time the disk is ours.
    fn write_job(&self, number: u64, job: &Job) -> Result<bool, String> {
        // The slow part that needs no disk first, outside every lock.
        let bytes = craft3d_format::to_bytes(&job.document).map_err(|e| {
            format!(
                "cannot autosave {}: {e}",
                display_name(job.path.as_deref(), job.title.as_deref())
            )
        })?;
        let mut lock = guard(&self.disk);
        if !guard(&self.state).entries.contains_key(&number) {
            return Ok(false);
        }
        self.ensure_ready(&mut lock)?;
        // The document first: a record without its document is worth
        // nothing, a document without its record can still be opened.
        write_atomic(&self.document_path(&self.key, number), &bytes)
            .map_err(|e| format!("autosave failed: {}", e.message()))?;
        self.write_record(number, job, self.clock.epoch_seconds())
            .map_err(|e| format!("autosave failed: {e}"))?;
        Ok(true)
    }

    /// When the entry's snapshot is to be written, if it has one.
    fn due(&self, entry: &Entry) -> Option<Duration> {
        entry.pending.as_ref()?;
        Some(if entry.urgent {
            Duration::ZERO
        } else {
            entry.since.saturating_add(self.interval)
        })
    }

    fn next_due(&self, state: &State) -> Option<(Duration, u64)> {
        state
            .entries
            .iter()
            .filter_map(|(number, entry)| Some((self.due(entry)?, *number)))
            .min()
    }

    /// Writes the snapshot that is due first, if one is due. Returns
    /// whether there was one.
    fn step(&self) -> bool {
        let (number, job) = {
            let mut state = guard(&self.state);
            let now = self.clock.monotonic();
            let Some((due, number)) = self.next_due(&state) else {
                return false;
            };
            if due > now {
                return false;
            }
            let Some(entry) = state.entries.get_mut(&number) else {
                return false;
            };
            entry.urgent = false;
            let Some(job) = entry.pending.take() else {
                return false;
            };
            state.writing = Some(number);
            (number, job)
        };
        let started = std::time::Instant::now();
        let outcome = self.write_job(number, &job);
        let took = started.elapsed().as_secs_f64() * 1000.0;
        {
            let mut state = guard(&self.state);
            state.writing = None;
            let now = self.clock.monotonic();
            match outcome {
                Ok(true) => {
                    let at = self.clock.epoch_seconds();
                    if let Some(entry) = state.entries.get_mut(&number) {
                        entry.since = now;
                        entry.written = Some(at);
                    }
                    state.last_autosave = Some(at);
                    state.last_write_ms = Some(took);
                    state.writes = state.writes.saturating_add(1);
                    state.problem = None;
                }
                Ok(false) => {}
                Err(problem) => {
                    // Tried again an interval from now, with this
                    // snapshot unless a newer one came.
                    if let Some(entry) = state.entries.get_mut(&number) {
                        entry.since = now;
                        if entry.pending.is_none() {
                            entry.pending = Some(job);
                        }
                    }
                    state.problem = Some(problem);
                }
            }
        }
        self.changed();
        true
    }

    /// The autosave thread: sleeps until a snapshot is due, writes it.
    fn work(&self) {
        loop {
            {
                let mut state = guard(&self.state);
                loop {
                    if state.stop {
                        return;
                    }
                    let now = self.clock.monotonic();
                    state = match self.next_due(&state) {
                        Some((due, _)) if due <= now => break,
                        Some((due, _)) => {
                            self.wake
                                .wait_timeout(state, due.saturating_sub(now))
                                .unwrap_or_else(PoisonError::into_inner)
                                .0
                        }
                        None => self
                            .wake
                            .wait(state)
                            .unwrap_or_else(PoisonError::into_inner),
                    };
                }
            }
            // A panic in the writer must not end autosave for good.
            let stepped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.step()));
            if stepped.is_err() {
                let now = self.clock.monotonic();
                let mut state = guard(&self.state);
                // Its snapshot went with the panic: the next command's
                // look at the documents hands it over again, and it is
                // tried an interval from now.
                if let Some(number) = state.writing.take()
                    && let Some(entry) = state.entries.get_mut(&number)
                {
                    entry.noted = None;
                    entry.since = now;
                }
                state.problem = Some(craft3d_math::bug_text!("autosave failed").to_string());
                drop(state);
                self.changed();
            }
        }
    }

    /// The files in the directory, per launch other than this one.
    fn launches(&self) -> BTreeMap<String, Launch> {
        let mut launches: BTreeMap<String, Launch> = BTreeMap::new();
        let Ok(listing) = fs::read_dir(&self.dir) else {
            // No directory: nothing was ever saved here.
            return launches;
        };
        for entry in listing.take(MAX_DIR_ENTRIES).flatten() {
            let name = entry.file_name();
            let Some((key, part)) = name.to_str().and_then(parse_name) else {
                continue;
            };
            if key == self.key {
                continue;
            }
            let launch = launches.entry(key.to_string()).or_default();
            match part {
                Part::Lock => {}
                Part::Document(number) => launch.documents.entry(number).or_default().0 = true,
                Part::Record(number) => launch.documents.entry(number).or_default().1 = true,
                Part::Temp => launch.temps.push(entry.path()),
            }
        }
        launches
    }

    /// Removes what a dead launch has no use for: unfinished temporary
    /// files and, when it has no documents left, its lock file.
    fn tidy(&self, key: &str, launch: &Launch, documents_left: bool) {
        for temp in &launch.temps {
            let _ = fs::remove_file(temp);
        }
        if !documents_left {
            let _ = fs::remove_file(self.lock_path(key));
        }
    }

    fn inspect(&self, key: &str, number: u64, has_document: bool, has_record: bool) -> Recoverable {
        let path = self.document_path(key, number);
        let record = if has_record {
            read_record(&self.record_path(key, number))
        } else {
            Err("its record is missing".to_string())
        };
        let loaded = if has_document {
            craft3d_format::load(&path).map_err(|e| e.to_string())
        } else {
            Err("its document file is missing".to_string())
        };
        let problem = match (&loaded, &record) {
            (Err(why), _) => Some(format!("it cannot be opened: {why}")),
            (Ok(_), Err(why)) => Some(format!("{why}: its name and original path are unknown")),
            (Ok(_), Ok(_)) => None,
        };
        let record = record.unwrap_or_default();
        Recoverable {
            id: format!("{key}-{number}"),
            name: record
                .name
                .unwrap_or_else(|| display_name(record.path.as_deref(), record.title.as_deref())),
            path: record.path,
            time: record.time,
            size: fs::metadata(&path).ok().map(|m| m.len()),
            revision: record.revision,
            features: loaded.as_ref().ok().map(|document| document.features.len()),
            pid: record.pid,
            openable: loaded.is_ok(),
            problem,
        }
    }

    fn scan(&self) -> (Vec<Recoverable>, usize) {
        let mut found = Vec::new();
        let mut running = 0_usize;
        for (key, launch) in self.launches() {
            // Held while its files are looked at, so no two scans tidy
            // the same launch at once.
            let _held = match self.liveness(&key) {
                Liveness::Alive => {
                    running = running.saturating_add(1);
                    continue;
                }
                Liveness::Dead(held) => held,
            };
            self.tidy(&key, &launch, !launch.documents.is_empty());
            for (number, (has_document, has_record)) in &launch.documents {
                if found.len() >= MAX_LISTED {
                    break;
                }
                found.push(self.inspect(&key, *number, *has_document, *has_record));
            }
        }
        (found, running)
    }
}

impl Store {
    pub(crate) fn new(options: RecoveryOptions) -> Self {
        let shared = Arc::new(Shared {
            dir: options.directory,
            key: new_key(),
            interval: options.interval,
            clock: options.clock,
            notify: options.notify,
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
            disk: Mutex::new(None),
        });
        let worker = if options.background {
            let for_worker = Arc::clone(&shared);
            // Without a thread the snapshots are written by the commands
            // themselves: slower, never lost.
            std::thread::Builder::new()
                .name("craft3d-autosave".to_string())
                .spawn(move || for_worker.work())
                .ok()
        } else {
            None
        };
        Self { shared, worker }
    }

    /// Notes which documents changed, removes the recovery files of the
    /// ones that no longer need one and, without a thread, writes what
    /// is due.
    pub(crate) fn sync(&self, views: &[DocView<'_>], urgent: bool) {
        let shared = &self.shared;
        let now = shared.clock.monotonic();
        let mut gone = Vec::new();
        {
            let mut state = guard(&shared.state);
            for view in views.iter().filter(|view| view.worth) {
                let entry = state.entries.entry(view.id).or_insert(Entry {
                    noted: None,
                    since: now,
                    pending: None,
                    urgent: false,
                    written: None,
                });
                if entry.noted != Some(view.revision) {
                    entry.noted = Some(view.revision);
                    entry.pending = Some(Job {
                        revision: view.revision,
                        document: Arc::clone(view.document),
                        path: view.path.map(str::to_string),
                        title: view.title.map(str::to_string),
                        original: view.original,
                        recovered: view.recovered,
                    });
                }
                if urgent && entry.pending.is_some() {
                    entry.urgent = true;
                }
            }
            for number in state.entries.keys() {
                if !views.iter().any(|view| view.id == *number && view.worth) {
                    gone.push(*number);
                }
            }
        }
        if !gone.is_empty() {
            self.remove(&gone);
        }
        if self.worker.is_some() {
            shared.wake.notify_all();
        } else {
            while shared.step() {}
        }
    }

    /// Forgets these documents and removes their recovery files, after a
    /// write that is in progress.
    fn remove(&self, numbers: &[u64]) {
        let shared = &self.shared;
        let _disk = guard(&shared.disk);
        {
            let mut state = guard(&shared.state);
            for number in numbers {
                state.entries.remove(number);
            }
        }
        for number in numbers {
            // Not there when nothing was written yet.
            let _ = fs::remove_file(shared.document_path(&shared.key, *number));
            let _ = fs::remove_file(shared.record_path(&shared.key, *number));
        }
    }

    pub(crate) fn overview(&self) -> RecoveryOverview {
        let shared = &self.shared;
        let state = guard(&shared.state);
        RecoveryOverview {
            enabled: true,
            directory: Some(shared.dir.display().to_string()),
            interval_seconds: shared.interval.as_secs_f64(),
            last_autosave: state.last_autosave,
            autosaved: state
                .entries
                .iter()
                .filter_map(|(number, entry)| Some((*number, entry.written?)))
                .collect(),
            last_write_ms: state.last_write_ms,
            writes: state.writes,
            problem: state.problem.clone(),
            // A snapshot that is being written is not on disk yet.
            pending: state
                .entries
                .iter()
                .filter(|(number, entry)| {
                    entry.pending.is_some() || state.writing == Some(**number)
                })
                .count(),
            found: state.found.clone(),
            running: state.running,
        }
    }

    /// Looks in the directory for what dead launches left behind, and
    /// remembers it for [`Self::overview`].
    pub(crate) fn scan(&self) -> Vec<Recoverable> {
        let (found, running) = self.shared.scan();
        let mut state = guard(&self.shared.state);
        state.found.clone_from(&found);
        state.running = running;
        found
    }

    /// Loads the document `id` of a dead launch and makes its recovery
    /// file this launch's own, as document `number`.
    pub(crate) fn adopt(&self, id: &str, number: u64) -> Result<Adopted, String> {
        let shared = &self.shared;
        let missing = || {
            format!(
                "there is no recoverable document `{}`; recovery.list shows what there is",
                id.chars().take(40).collect::<String>()
            )
        };
        let (key, theirs) = parse_id(id).ok_or_else(missing)?;
        if key == shared.key {
            return Err(missing());
        }
        let mut lock = guard(&shared.disk);
        let _held = match shared.liveness(key) {
            Liveness::Alive => {
                return Err(format!("`{id}` belongs to a 3DCraft that is still running"));
            }
            Liveness::Dead(held) => held,
        };
        let from = shared.document_path(key, theirs);
        if !from.is_file() {
            return Err(missing());
        }
        let (document, read) = craft3d_format::load_noted(&from)
            .map_err(|e| format!("`{id}` cannot be opened: {e}"))?;
        let their_record = shared.record_path(key, theirs);
        let record = read_record(&their_record).unwrap_or_default();
        let job = Job {
            revision: 0,
            document: Arc::new(document.clone()),
            path: record.path.clone(),
            title: record.title.clone(),
            original: record.original,
            recovered: true,
        };
        let time = record.time.unwrap_or_else(|| shared.clock.epoch_seconds());
        let adopted = shared
            .ensure_ready(&mut lock)
            .and_then(|()| {
                fs::rename(&from, shared.document_path(&shared.key, number))
                    .map_err(|e| e.to_string())
            })
            .map(|()| {
                // From here the file is ours; a record that cannot be
                // written is rewritten by the next autosave.
                let _ = shared.write_record(number, &job, time);
                let _ = fs::remove_file(&their_record);
                guard(&shared.state).entries.insert(
                    number,
                    Entry {
                        noted: Some(0),
                        since: shared.clock.monotonic(),
                        pending: None,
                        urgent: false,
                        // Its file, taken over, as it was written.
                        written: Some(time),
                    },
                );
            });
        Ok(Adopted {
            document,
            name: record
                .name
                .unwrap_or_else(|| display_name(record.path.as_deref(), record.title.as_deref())),
            path: record.path,
            title: record.title,
            original: record.original,
            warning: adopted.err().map(|why| {
                format!("its recovery file could not be taken over ({why}) and stays where it is")
            }),
            read,
        })
    }

    /// Removes one recoverable document (`Some(id)`) or all of them.
    /// Returns how many were removed. Never touches a running launch's.
    /// One that cannot be removed is an error when it was asked for by
    /// id; of all, it is left and stays listed.
    pub(crate) fn discard(&self, id: Option<&str>) -> Result<usize, String> {
        let shared = &self.shared;
        let wanted = match id {
            Some(id) => {
                let shown: String = id.chars().take(40).collect();
                Some(parse_id(id).ok_or_else(|| {
                    format!("there is no recoverable document `{shown}`; recovery.list shows what there is")
                })?)
            }
            None => None,
        };
        let mut discarded = 0_usize;
        let mut matched = false;
        for (key, launch) in shared.launches() {
            if wanted.is_some_and(|(wanted_key, _)| wanted_key != key) {
                continue;
            }
            let _held = match shared.liveness(&key) {
                Liveness::Alive => {
                    if wanted.is_some() {
                        return Err(format!(
                            "`{key}` is a 3DCraft that is still running; its documents are not for discarding"
                        ));
                    }
                    continue;
                }
                Liveness::Dead(held) => held,
            };
            let mut left = 0_usize;
            for number in launch.documents.keys() {
                if wanted.is_some_and(|(_, wanted_number)| wanted_number != *number) {
                    left = left.saturating_add(1);
                    continue;
                }
                matched = true;
                let document = fs::remove_file(shared.document_path(&key, *number));
                let record = fs::remove_file(shared.record_path(&key, *number));
                let gone = |outcome: &std::io::Result<()>| match outcome {
                    Ok(()) => true,
                    Err(error) => error.kind() == std::io::ErrorKind::NotFound,
                };
                if gone(&document) && gone(&record) {
                    discarded = discarded.saturating_add(1);
                } else if wanted.is_some() {
                    let why = document.err().or(record.err()).map(|e| e.to_string());
                    return Err(format!(
                        "`{key}-{number}` could not be removed: {}",
                        why.unwrap_or_default()
                    ));
                } else {
                    // One of all: the others are still discarded, and
                    // this one stays listed.
                    left = left.saturating_add(1);
                }
            }
            shared.tidy(&key, &launch, left > 0);
        }
        if let (Some(_), false) = (wanted, matched) {
            let shown: String = id.unwrap_or_default().chars().take(40).collect();
            return Err(format!(
                "there is no recoverable document `{shown}`; recovery.list shows what there is"
            ));
        }
        Ok(discarded)
    }

    fn stop_worker(&mut self) {
        guard(&self.shared.state).stop = true;
        self.shared.wake.notify_all();
        if let Some(worker) = self.worker.take() {
            // A worker that panicked has nothing left to finish.
            let _ = worker.join();
        }
    }

    /// The clean end: nothing is being written any more, and this
    /// launch's files are removed.
    pub(crate) fn end(mut self) {
        self.stop_worker();
        let shared = &self.shared;
        let mut lock = guard(&shared.disk);
        guard(&shared.state).entries.clear();
        if let Ok(listing) = fs::read_dir(&shared.dir) {
            let hidden = format!(".{}", shared.key);
            for entry in listing.take(MAX_DIR_ENTRIES).flatten() {
                let name = entry.file_name();
                let ours = name
                    .to_str()
                    .is_some_and(|name| name.starts_with(&shared.key) || name.starts_with(&hidden));
                if ours {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
        *lock = None;
    }
}

impl Drop for Store {
    /// Stops the thread and leaves the files: what a crash leaves.
    fn drop(&mut self) {
        self.stop_worker();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_in_the_directory_are_told_apart() {
        let key = "0123456789abcdef";
        assert_eq!(parse_name("0123456789abcdef.lock"), Some((key, Part::Lock)));
        assert_eq!(
            parse_name("0123456789abcdef-12.3dcraft"),
            Some((key, Part::Document(12)))
        );
        assert_eq!(
            parse_name("0123456789abcdef-0.json"),
            Some((key, Part::Record(0)))
        );
        assert_eq!(
            parse_name(".0123456789abcdef-3.3dcraft.4242.tmp"),
            Some((key, Part::Temp))
        );
        for other in [
            "",
            ".",
            "notes.txt",
            "0123456789ABCDEF.lock",
            "0123456789abcde.lock",
            "0123456789abcdef-.json",
            "0123456789abcdef--1.json",
            "0123456789abcdef-1.txt",
            "0123456789abcdef-99999999999999999999999.json",
            "../0123456789abcdef-1.json",
            ".é123456789abcdef.tmp",
        ] {
            assert_eq!(parse_name(other), None, "{other}");
        }
        assert_eq!(parse_id("0123456789abcdef-7"), Some((key, 7)));
        assert_eq!(parse_id("0123456789abcdef-+7"), None);
        assert_eq!(parse_id("../../etc/passwd-1"), None);
    }

    /// A snapshot that is being written counts as pending: whoever waits
    /// for "everything is on disk" must not be told so in mid-write.
    #[test]
    fn a_write_in_progress_is_still_pending() {
        let clock = Arc::new(crate::recovery::ManualClock::new(0));
        let store = Store::new(RecoveryOptions::manual("unused-recovery-directory", clock));
        let document = Arc::new(Document::default());
        store.sync(
            &[DocView {
                id: 7,
                revision: 1,
                worth: true,
                document: &document,
                path: None,
                title: None,
                original: None,
                recovered: false,
            }],
            false,
        );
        assert_eq!(store.overview().pending, 1);
        // What `step` does when it starts to write.
        {
            let mut state = guard(&store.shared.state);
            state.entries.get_mut(&7).unwrap().pending = None;
            state.writing = Some(7);
        }
        assert_eq!(store.overview().pending, 1);
        guard(&store.shared.state).writing = None;
        assert_eq!(store.overview().pending, 0);
    }

    #[test]
    fn keys_are_keys_and_differ() {
        let (a, b) = (new_key(), new_key());
        assert!(is_key(&a) && is_key(&b), "{a} {b}");
        assert_ne!(a, b);
    }

    #[test]
    fn a_record_keeps_what_a_stamp_says() {
        let stamp = Some(FileStamp {
            size: 1234,
            modified: Some((1_700_000_000, 987_654_321)),
        });
        assert_eq!(stamp_from(Some(&stamp_json(stamp))), stamp);
        assert_eq!(stamp_from(Some(&stamp_json(None))), None);
        assert_eq!(stamp_from(Some(&json!({"size": "big"}))), None);
    }

    /// A record names its document by its file, else by the title it was
    /// shown under, else `untitled`; a record written before titles were
    /// (2026-11-10) has none, and still reads.
    #[test]
    fn a_record_names_its_document_and_reads_without_a_title() {
        assert_eq!(
            display_name(Some("/a/gear.3dcraft"), Some("Gear")),
            "gear.3dcraft"
        );
        assert_eq!(display_name(None, Some("Moka pot")), "Moka pot");
        assert_eq!(display_name(None, None), "untitled");
        let dir = std::env::temp_dir().join(format!("craft3d-record-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let old = dir.join("old.json");
        std::fs::write(
            &old,
            br#"{"recovery_version": 1, "name": "untitled", "path": null, "revision": 3, "time": 1800000000, "pid": 1}"#,
        )
        .unwrap();
        let record = read_record(&old).unwrap();
        assert_eq!(
            (record.name.as_deref(), record.title),
            (Some("untitled"), None)
        );
        assert_eq!(record.revision, Some(3));
        let new = dir.join("new.json");
        std::fs::write(
            &new,
            br#"{"recovery_version": 1, "name": "Moka pot", "path": null, "title": "Moka pot\u0007"}"#,
        )
        .unwrap();
        assert_eq!(
            read_record(&new).unwrap().title.as_deref(),
            Some("Moka pot")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
