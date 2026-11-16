//! Which graphics interface the window starts with, and what it learnt
//! from the starts before (docs/design/rendering.md, *A graphics interface
//! that kills the start*).
//!
//! A graphics driver can end the process while the window starts (an
//! access violation inside Intel's Vulkan driver on Windows, 2026-11-16):
//! nothing can catch that, so the window keeps a journal in its state
//! folder ([`JOURNAL_FILE`]). Before the device is made on an adapter it
//! records the attempt ("attempting", synced to disk); once
//! [`FRAMES_FOR_OK`] frames are drawn it records "ok"; on an orderly exit
//! before that, "closing". At the next start an adapter's interface whose
//! last record is still "attempting" is taken as having ended the start
//! and is skipped for the next in the order ([`choose`]), and the window
//! says so (a line on standard error and a bar under the menu). A record
//! counts for its app version, adapter, driver and interface only, and
//! for [`FRESH_FOR`]: drivers get updated. When every interface in the
//! order is marked, the first is tried again: the window never refuses to
//! start. `--reset-graphics` forgets the journal.
//!
//! The order is data ([`rule`]): on Windows Vulkan, then OpenGL, then
//! DirectX 12 (whose default compiler, FXC, takes minutes over the studio
//! look's shaders), or DirectX 12 before OpenGL when a `dxcompiler.dll`
//! lies beside the program; elsewhere wgpu's own choice. An order the
//! person gives (`CRAFT3D_BACKEND_ORDER`, or `WGPU_BACKEND`) is followed as
//! it is and never skipped; the journal still records it.

use craft3d_gpu::diag::{self, Candidate};
use eframe::wgpu;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// The journal's file in the state folder.
pub const JOURNAL_FILE: &str = "graphics-start.json";
/// The frames drawn after which a start is "ok".
pub const FRAMES_FOR_OK: u64 = 3;
/// How long a record counts, in seconds: seven days.
pub const FRESH_FOR: u64 = 7 * 24 * 60 * 60;
/// The most of a journal file read; a larger one is ignored and written
/// anew.
const MOST_BYTES: u64 = 256 * 1024;
/// The most records kept, the newest.
const MOST_RECORDS: usize = 64;
/// The most characters kept of a text field.
const MOST_CHARS: usize = 200;
/// The journal's format.
const VERSION: u64 = 1;

/// The name of an interface as the window says it.
pub fn backend_name(backend: wgpu::Backend) -> &'static str {
    match backend {
        wgpu::Backend::Vulkan => "Vulkan",
        wgpu::Backend::Dx12 => "DirectX 12",
        wgpu::Backend::Gl => "OpenGL",
        wgpu::Backend::Metal => "Metal",
        wgpu::Backend::BrowserWebGpu => "WebGPU",
        wgpu::Backend::Noop => "no interface",
    }
}

fn backend_of(name: &str) -> Option<wgpu::Backend> {
    [
        wgpu::Backend::Vulkan,
        wgpu::Backend::Dx12,
        wgpu::Backend::Gl,
        wgpu::Backend::Metal,
        wgpu::Backend::BrowserWebGpu,
    ]
    .into_iter()
    .find(|b| b.to_str() == name)
}

fn clipped(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(MOST_CHARS)
        .collect()
}

/// One start's interface on one adapter: what a record is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attempt {
    pub app: String,
    pub backend: wgpu::Backend,
    pub adapter: String,
    pub vendor: u32,
    pub device: u32,
    pub driver: String,
    pub driver_info: String,
}

impl Attempt {
    pub fn of(app: &str, info: &wgpu::AdapterInfo) -> Attempt {
        Attempt {
            app: clipped(app),
            backend: info.backend,
            adapter: clipped(&info.name),
            vendor: info.vendor,
            device: info.device,
            driver: clipped(&info.driver),
            driver_info: clipped(&info.driver_info),
        }
    }

    /// The same app version, adapter, driver and interface (the adapter's
    /// name aside: the ids say which it is).
    fn same(&self, other: &Attempt) -> bool {
        self.app == other.app
            && self.backend == other.backend
            && self.vendor == other.vendor
            && self.device == other.device
            && self.driver == other.driver
            && self.driver_info == other.driver_info
    }

    /// The adapter and its driver, as the window says them.
    pub fn adapter_text(&self) -> String {
        let driver = [self.driver.trim(), self.driver_info.trim()]
            .into_iter()
            .filter(|d| !d.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        if driver.is_empty() {
            self.adapter.clone()
        } else {
            format!("{}, driver {driver}", self.adapter)
        }
    }
}

/// How a start went, as far as the journal knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The device was being made, or the first frames drawn: a start that
    /// ends here was ended by the driver (or the power).
    Attempting,
    /// The first frames were drawn.
    Ok,
    /// The window was closed on purpose before that.
    Closing,
}

impl Outcome {
    fn name(self) -> &'static str {
        match self {
            Outcome::Attempting => "attempting",
            Outcome::Ok => "ok",
            Outcome::Closing => "closing",
        }
    }

    fn of(name: &str) -> Option<Outcome> {
        match name {
            "attempting" => Some(Outcome::Attempting),
            "ok" => Some(Outcome::Ok),
            "closing" => Some(Outcome::Closing),
            _ => None,
        }
    }
}

/// The last record of an attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub attempt: Attempt,
    pub outcome: Outcome,
    /// Seconds since 1970.
    pub time: u64,
    /// The window's notice about it was dismissed.
    pub dismissed: bool,
}

/// What the window knows of its earlier starts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Journal {
    /// The last record of each attempt, the newest last.
    pub records: Vec<Record>,
}

impl Journal {
    /// The journal in `text`; `None` for one that cannot be read (not
    /// JSON, another format, too large): it is then written anew. Records
    /// that cannot be read are left out.
    pub fn parse(text: &str) -> Option<Journal> {
        if text.len() as u64 > MOST_BYTES {
            return None;
        }
        let value: Value = serde_json::from_str(text).ok()?;
        if value.get("version")?.as_u64()? != VERSION {
            return None;
        }
        let records = value
            .get("records")?
            .as_array()?
            .iter()
            .filter_map(record_of)
            .collect::<Vec<_>>();
        let start = records.len().saturating_sub(MOST_RECORDS);
        Some(Journal {
            records: records.into_iter().skip(start).collect(),
        })
    }

    /// The journal as its file holds it.
    pub fn to_text(&self) -> String {
        let records: Vec<Value> = self
            .records
            .iter()
            .map(|r| {
                json!({
                    "app": r.attempt.app,
                    "backend": r.attempt.backend.to_str(),
                    "adapter": r.attempt.adapter,
                    "vendor": r.attempt.vendor,
                    "device": r.attempt.device,
                    "driver": r.attempt.driver,
                    "driverInfo": r.attempt.driver_info,
                    "outcome": r.outcome.name(),
                    "time": r.time,
                    "dismissed": r.dismissed,
                })
            })
            .collect();
        serde_json::to_string_pretty(&json!({ "version": VERSION, "records": records }))
            .unwrap_or_default()
    }

    fn find(&self, attempt: &Attempt) -> Option<usize> {
        self.records.iter().position(|r| r.attempt.same(attempt))
    }

    /// Records `outcome` for `attempt` at `now`; a notice dismissed for it
    /// stays dismissed while it ends the start the same way.
    pub fn set(&mut self, attempt: &Attempt, outcome: Outcome, now: u64) {
        let dismissed = self.find(attempt).is_some_and(|at| {
            let old = self.records.remove(at);
            old.dismissed && old.outcome == outcome
        });
        self.records.push(Record {
            attempt: attempt.clone(),
            outcome,
            time: now,
            dismissed,
        });
        let start = self.records.len().saturating_sub(MOST_RECORDS);
        self.records.drain(..start);
    }

    /// The record that says `attempt` ended an earlier start: its last one,
    /// still "attempting", written within [`FRESH_FOR`] of `now`.
    pub fn killed(&self, attempt: &Attempt, now: u64) -> Option<&Record> {
        let record = self.records.get(self.find(attempt)?)?;
        let fresh = record.time.saturating_add(FRESH_FOR) >= now
            && record.time <= now.saturating_add(24 * 60 * 60);
        (record.outcome == Outcome::Attempting && fresh).then_some(record)
    }

    /// Forgets what is known of `attempt`: it is tried again.
    pub fn forget(&mut self, attempt: &Attempt) {
        self.records.retain(|r| !r.attempt.same(attempt));
    }

    /// The notice about `attempt` was dismissed.
    pub fn dismiss(&mut self, attempt: &Attempt) {
        if let Some(record) = self.find(attempt).and_then(|at| self.records.get_mut(at)) {
            record.dismissed = true;
        }
    }
}

fn record_of(value: &Value) -> Option<Record> {
    let text = |name: &str| value.get(name).and_then(Value::as_str).map(clipped);
    let number = |name: &str| {
        value
            .get(name)
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
    };
    Some(Record {
        attempt: Attempt {
            app: text("app")?,
            backend: backend_of(value.get("backend")?.as_str()?)?,
            adapter: text("adapter")?,
            vendor: number("vendor")?,
            device: number("device")?,
            driver: text("driver")?,
            driver_info: text("driverInfo")?,
        },
        outcome: Outcome::of(value.get("outcome")?.as_str()?)?,
        time: value.get("time")?.as_u64()?,
        dismissed: value
            .get("dismissed")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

/// The journal at `path`: empty when there is none or it cannot be read
/// (it is written anew at the next record).
pub fn read(path: &Path) -> Journal {
    let Ok(meta) = std::fs::metadata(path) else {
        return Journal::default();
    };
    if meta.len() > MOST_BYTES {
        return Journal::default();
    }
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| Journal::parse(&text))
        .unwrap_or_default()
}

/// Writes `journal` to `path` at once and for good: into a file beside it,
/// synced to disk, then renamed over it, so a start that ends a moment
/// later leaves the record and never half a file.
pub fn write(path: &Path, journal: &Journal) -> Result<(), String> {
    use std::io::Write as _;
    let folder = path
        .parent()
        .ok_or_else(|| format!("{} has no folder", path.display()))?;
    std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let temporary = folder.join(format!("{JOURNAL_FILE}.{}.tmp", std::process::id()));
    let written = std::fs::File::create(&temporary).and_then(|mut file| {
        file.write_all(journal.to_text().as_bytes())?;
        file.sync_all()
    });
    if let Err(e) = written.and_then(|()| std::fs::rename(&temporary, path)) {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!("{}: {e}", path.display()));
    }
    Ok(())
}

/// Seconds since 1970, now.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// How the interfaces are ordered, and whether the journal may skip one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    /// The interfaces in order; `None` for wgpu's own choice (every
    /// adapter the instance has, by its kind: discrete first).
    pub order: Option<Vec<wgpu::Backend>>,
    /// The order is the window's own: the journal may skip in it. An order
    /// the person gives is followed as it is.
    pub skip: bool,
    /// Why this order, for the trace.
    pub why: String,
}

/// The rule the window starts by: `CRAFT3D_BACKEND_ORDER` (`order`) or
/// `WGPU_BACKEND` (`wgpu_backend`) as the person gives them; else on
/// Windows Vulkan, OpenGL, DirectX 12 (DirectX 12 before OpenGL with a
/// `dxcompiler.dll` beside the program: `dxc`); elsewhere wgpu's own
/// choice. A bad `CRAFT3D_BACKEND_ORDER` is said and ignored.
pub fn rule(order: Option<&str>, wgpu_backend: Option<&str>, windows: bool, dxc: bool) -> Rule {
    match diag::backend_order(order, wgpu_backend) {
        Ok(Some(order)) => {
            return Rule {
                why: format!("{}={}", diag::BACKEND_ORDER_VAR, diag::backend_list(&order)),
                order: Some(order),
                skip: false,
            };
        }
        Ok(None) => {}
        Err(why) => {
            diag::say(&why);
            diag::note("adapter", &why);
        }
    }
    if wgpu_backend.is_some() {
        return Rule {
            order: None,
            skip: false,
            why: "WGPU_BACKEND chooses the interfaces".to_string(),
        };
    }
    if windows {
        use wgpu::Backend::{Dx12, Gl, Vulkan};
        return if dxc {
            Rule {
                order: Some(vec![Vulkan, Dx12, Gl]),
                skip: true,
                why: "Windows, with dxcompiler.dll beside the program".to_string(),
            }
        } else {
            Rule {
                order: Some(vec![Vulkan, Gl, Dx12]),
                skip: true,
                why: "Windows: DirectX 12 last, its compiler (FXC) is slow".to_string(),
            }
        };
    }
    Rule {
        order: None,
        skip: true,
        why: "wgpu's own choice".to_string(),
    }
}

/// An adapter the window may start on.
#[derive(Clone, Debug, PartialEq)]
pub struct Offered {
    pub candidate: Candidate,
    pub attempt: Attempt,
}

/// What [`choose`] decided.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    /// Into the adapters offered.
    pub index: usize,
    /// The records of the adapters passed over because they ended an
    /// earlier start, in order.
    pub skipped: Vec<Record>,
    /// Every adapter in the order had ended an earlier start: the first is
    /// tried again.
    pub retried: bool,
}

/// The adapter of `offered` the window starts on under `rule`, with what
/// `journal` says at `now`: in the rule's order, within an interface by
/// wgpu's own preference under `power`, only adapters that can present;
/// where the rule allows, past those that ended an earlier start; the first
/// again when all did.
pub fn choose(
    offered: &[Offered],
    rule: &Rule,
    journal: &Journal,
    power: wgpu::PowerPreference,
    now: u64,
) -> Result<Choice, String> {
    let presenting: Vec<(usize, &Offered)> = offered
        .iter()
        .enumerate()
        .filter(|(_, o)| o.candidate.presents)
        .collect();
    // wgpu's own preference, a stable sort: the enumeration's order within
    // one kind (`wgpu-core`, `Instance::request_adapter`).
    fn by_preference(
        mut list: Vec<(usize, &Offered)>,
        power: wgpu::PowerPreference,
    ) -> Vec<(usize, &Offered)> {
        if power != wgpu::PowerPreference::None {
            list.sort_by_key(|(_, o)| diag::preference(o.candidate.device_type, power));
        }
        list
    }
    let ordered: Vec<(usize, &Offered)> = match &rule.order {
        None => by_preference(presenting, power),
        Some(order) => order
            .iter()
            .flat_map(|backend| {
                by_preference(
                    presenting
                        .iter()
                        .filter(|(_, o)| o.candidate.backend == *backend)
                        .copied()
                        .collect(),
                    power,
                )
            })
            .collect(),
    };
    let Some(&(first, _)) = ordered.first() else {
        let found = offered
            .iter()
            .map(|o| {
                format!(
                    "`{}` ({}{})",
                    o.candidate.name,
                    o.candidate.backend.to_str(),
                    if o.candidate.presents {
                        ""
                    } else {
                        ", cannot present"
                    }
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "no adapter can present to the window in the order ({}; found: {})",
            rule.why,
            if found.is_empty() { "none" } else { &found }
        ));
    };
    if !rule.skip {
        return Ok(Choice {
            index: first,
            skipped: Vec::new(),
            retried: false,
        });
    }
    let mut skipped = Vec::new();
    for (index, o) in &ordered {
        match journal.killed(&o.attempt, now) {
            Some(record) => skipped.push(record.clone()),
            None => {
                return Ok(Choice {
                    index: *index,
                    skipped,
                    retried: false,
                });
            }
        }
    }
    Ok(Choice {
        index: first,
        skipped,
        retried: true,
    })
}

/// What the window says when it passed over an interface: one line, the
/// adapter that it passed over and why; `None` when it passed over none.
pub fn notice_text(chosen: &Attempt, choice: &Choice) -> Option<String> {
    if choice.retried {
        let tried = choice
            .skipped
            .iter()
            .map(|r| backend_name(r.attempt.backend))
            .collect::<Vec<_>>()
            .join(", ");
        return Some(format!(
            "Every graphics interface ended an earlier start on this computer ({tried}); \
             3DCraft tries {} again.",
            backend_name(chosen.backend)
        ));
    }
    let first = choice.skipped.first()?;
    Some(format!(
        "3DCraft draws through {} on this computer: starting with {} failed here ({}).",
        backend_name(chosen.backend),
        backend_name(first.attempt.backend),
        first.attempt.adapter_text()
    ))
}

/// Whether the window keeps the journal and passes over an interface that
/// ended a start: on Windows only in 0.1.x, where it was needed and tried
/// (2026-11-16); on Linux and macOS the window chooses as it always did
/// (docs/design/rendering.md › *Owed*: Linux's Vulkan to OpenGL for 0.2).
/// A test may turn it on elsewhere with [`TEST_SWITCH`], in a build with
/// debug assertions only: never in a package.
pub fn self_healing_enabled() -> bool {
    cfg!(windows) || (cfg!(debug_assertions) && test_switch_on(std::env::var(TEST_SWITCH).ok()))
}

/// `CRAFT3D_GRAPHICS_JOURNAL=on` turns the journal on outside Windows in a
/// build with debug assertions (a developer's, a test's), so a start that
/// passes over an interface can be tried on Linux; release builds ignore it.
pub const TEST_SWITCH: &str = "CRAFT3D_GRAPHICS_JOURNAL";

fn test_switch_on(value: Option<String>) -> bool {
    value.is_some_and(|v| v.trim() == "on")
}

/// The journal's path in the state folder `state`.
pub fn journal_path(state: &Path) -> PathBuf {
    state.join(JOURNAL_FILE)
}

/// What the window started on, as the adapter's choice left it.
#[derive(Clone, Debug, PartialEq)]
pub struct Chosen {
    pub attempt: Attempt,
    pub choice: Choice,
}

/// The window's start on its graphics interface, from the choice of the
/// adapter (eframe calls [`Start::selector`]'s function) to its first
/// frames ([`Start::frame`]) or its orderly end before them
/// ([`Start::closing`]), and the notice about it.
pub struct Start {
    /// The journal's file; `None` without a state folder (nothing is
    /// remembered then).
    journal: Option<PathBuf>,
    app: String,
    chosen: std::sync::Arc<std::sync::Mutex<Option<Chosen>>>,
    frames: u64,
    /// "ok" or "closing" was recorded, or there is nothing to record.
    settled: bool,
}

impl Start {
    /// A start that keeps its journal in the state folder `state` (none
    /// without one), of the app version `app`; with `reset`, the journal
    /// is forgotten first (`--reset-graphics`).
    pub fn new(state: Option<&Path>, app: &str, reset: bool) -> Start {
        Start::with_journal(state, app, reset, self_healing_enabled())
    }

    /// [`Start::new`], with or without the journal (`healing`): without it
    /// nothing is read, written or said, and no frame is asked for.
    pub fn with_journal(state: Option<&Path>, app: &str, reset: bool, healing: bool) -> Start {
        let journal = state.filter(|_| healing).map(journal_path);
        if reset && !healing {
            diag::say(
                "--reset-graphics: the window keeps no graphics start journal on this system",
            );
        }
        if reset && let Some(path) = &journal {
            match std::fs::remove_file(path) {
                Ok(()) => diag::say(&format!(
                    "--reset-graphics: forgot which graphics interfaces ended a start ({})",
                    path.display()
                )),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => diag::say(&format!(
                    "--reset-graphics: {} could not be removed: {e}",
                    path.display()
                )),
            }
        }
        Start {
            settled: journal.is_none(),
            journal,
            app: app.to_string(),
            chosen: std::sync::Arc::default(),
            frames: 0,
        }
    }

    /// The journal's file, if there is one.
    pub fn journal(&self) -> Option<&Path> {
        self.journal.as_deref()
    }

    /// The function eframe chooses the window's adapter with: by `rule`
    /// and the journal ([`choose`]), recording the attempt before the
    /// device is made, and saying an interface it passed over.
    pub fn selector(
        &self,
        rule: Rule,
        power: wgpu::PowerPreference,
    ) -> eframe::egui_wgpu::NativeAdapterSelectorMethod {
        let journal = self.journal.clone();
        let app = self.app.clone();
        let chosen = std::sync::Arc::clone(&self.chosen);
        std::sync::Arc::new(
            move |adapters: &[wgpu::Adapter], surface: Option<&wgpu::Surface<'_>>| {
                let offered: Vec<Offered> = adapters
                    .iter()
                    .map(|adapter| {
                        let info = adapter.get_info();
                        let presents =
                            surface.is_none_or(|surface| adapter.is_surface_supported(surface));
                        Offered {
                            candidate: Candidate::of(&info, presents),
                            attempt: Attempt::of(&app, &info),
                        }
                    })
                    .collect();
                let now = now();
                let mut book = journal.as_deref().map(read).unwrap_or_default();
                let choice = match choose(&offered, &rule, &book, power, now) {
                    Ok(choice) => choice,
                    Err(why) => {
                        diag::say(&why);
                        diag::note("adapter", &why);
                        return Err(why);
                    }
                };
                let attempt = offered
                    .get(choice.index)
                    .map(|o| o.attempt.clone())
                    .ok_or_else(|| "the chosen adapter is not there".to_string())?;
                let said = format!(
                    "graphics: {} on `{}` ({}{})",
                    backend_name(attempt.backend),
                    attempt.adapter,
                    rule.why,
                    if choice.skipped.is_empty() {
                        String::new()
                    } else {
                        format!(
                            "; passed over, for ending an earlier start: {}",
                            choice
                                .skipped
                                .iter()
                                .map(|r| format!(
                                    "{} on `{}`",
                                    backend_name(r.attempt.backend),
                                    r.attempt.adapter
                                ))
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    }
                );
                diag::note("adapter", &said);
                if let Some(text) = notice_text(&attempt, &choice) {
                    diag::say(&text);
                }
                if let Some(path) = &journal {
                    book.set(&attempt, Outcome::Attempting, now);
                    if let Err(why) = write(path, &book) {
                        diag::say(&format!(
                            "the graphics start journal cannot be written ({why}): a graphics \
                             interface that ends the start is not passed over next time"
                        ));
                    }
                }
                let adapter = adapters
                    .get(choice.index)
                    .cloned()
                    .ok_or_else(|| "the chosen adapter is not there".to_string())?;
                *chosen
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) =
                    Some(Chosen { attempt, choice });
                Ok(adapter)
            },
        )
    }

    /// What the adapter's choice left, once made.
    pub fn chosen(&self) -> Option<Chosen> {
        self.chosen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn record(&mut self, outcome: Outcome) {
        if self.settled {
            return;
        }
        self.settled = true;
        let (Some(path), Some(chosen)) = (self.journal.clone(), self.chosen()) else {
            return;
        };
        let mut book = read(&path);
        book.set(&chosen.attempt, outcome, now());
        if let Err(why) = write(&path, &book) {
            diag::say(&format!(
                "the graphics start journal cannot be written ({why})"
            ));
        }
    }

    /// A frame is about to be drawn: once [`FRAMES_FOR_OK`] were, the
    /// start is "ok". True while frames are still wanted for that (the
    /// window then asks for the next at once, so an idle window gets
    /// there too).
    pub fn frame(&mut self) -> bool {
        self.frames = self.frames.saturating_add(1);
        if self.frames > FRAMES_FOR_OK {
            self.record(Outcome::Ok);
            false
        } else {
            !self.settled
        }
    }

    /// The window ends on purpose: before its first frames were drawn,
    /// that is "closing", no failure.
    pub fn closing(&mut self) {
        self.record(Outcome::Closing);
    }

    /// The notice for the bar under the menu, when an interface was passed
    /// over and its notice was not dismissed: the line, the details, and
    /// the label of the action that tries it again.
    pub fn notice(&self, trace: Option<&str>) -> Option<(String, String, String)> {
        let chosen = self.chosen()?;
        if chosen.choice.skipped.iter().all(|r| r.dismissed) {
            return None;
        }
        let text = notice_text(&chosen.attempt, &chosen.choice)?;
        let mut details = format!(
            "Drawing through {} on {}.",
            backend_name(chosen.attempt.backend),
            chosen.attempt.adapter_text()
        );
        for record in &chosen.choice.skipped {
            details.push_str(&format!(
                " {} on {} ended a start of 3DCraft {} and is passed over for seven days, or \
                 until the driver or 3DCraft is updated.",
                backend_name(record.attempt.backend),
                record.attempt.adapter_text(),
                record.attempt.app
            ));
        }
        if let Some(path) = &self.journal {
            details.push_str(&format!(" Journal: {}.", path.display()));
        }
        if let Some(trace) = trace {
            details.push_str(&format!(" Trace: {trace}."));
        }
        details.push_str(
            " Start with --reset-graphics to try every interface again, or set \
             CRAFT3D_BACKEND_ORDER to choose (docs/guide/troubleshooting.md).",
        );
        let retry = match chosen.choice.skipped.first() {
            Some(first) if !chosen.choice.retried => {
                format!("Try {} next time", backend_name(first.attempt.backend))
            }
            _ => String::new(),
        };
        Some((text, details, retry))
    }

    /// The notice's Dismiss: it is not shown again for these interfaces
    /// while they end the start the same way.
    pub fn dismiss(&mut self) {
        self.change_skipped(Journal::dismiss);
    }

    /// The notice's Try again: the interfaces passed over are tried at the
    /// next start.
    pub fn retry(&mut self) {
        self.change_skipped(Journal::forget);
    }

    fn change_skipped(&mut self, change: fn(&mut Journal, &Attempt)) {
        let (Some(path), Some(chosen)) = (self.journal.clone(), self.chosen()) else {
            return;
        };
        let mut book = read(&path);
        for record in &chosen.choice.skipped {
            change(&mut book, &record.attempt);
        }
        if let Err(why) = write(&path, &book) {
            diag::say(&format!(
                "the graphics start journal cannot be written ({why})"
            ));
        }
    }
}

/// Whether `map` holds a key: for tests of the file.
#[cfg(test)]
fn has(map: &serde_json::Map<String, Value>, key: &str) -> bool {
    map.contains_key(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::Backend::{Dx12, Gl, Vulkan};
    use wgpu::DeviceType::{Cpu, DiscreteGpu, IntegratedGpu, Other};

    fn offered(name: &str, backend: wgpu::Backend, kind: wgpu::DeviceType) -> Offered {
        Offered {
            candidate: Candidate {
                name: name.to_string(),
                backend,
                device_type: kind,
                presents: true,
            },
            attempt: Attempt {
                app: "0.1.2".to_string(),
                backend,
                adapter: name.to_string(),
                vendor: 0x8086,
                device: 0x591c,
                driver: "Intel Corporation".to_string(),
                driver_info: "27.20.100.8681".to_string(),
            },
        }
    }

    /// PC 2 of 2026-11-16: Intel UHD 615 on Vulkan, OpenGL and DirectX 12,
    /// and Microsoft's software adapter.
    fn pc2() -> Vec<Offered> {
        vec![
            offered("Intel(R) UHD Graphics 615", Vulkan, IntegratedGpu),
            offered("Intel(R) UHD Graphics 615", Dx12, IntegratedGpu),
            offered("Microsoft Basic Render Driver", Dx12, Cpu),
            offered("Intel(R) UHD Graphics 615", Gl, Other),
        ]
    }

    const NOW: u64 = 1_800_000_000;
    const HIGH: wgpu::PowerPreference = wgpu::PowerPreference::HighPerformance;

    fn windows() -> Rule {
        rule(None, None, true, false)
    }

    /// The journal and its skipping are Windows's alone in 0.1.x: elsewhere
    /// a start reads, writes and says nothing, and asks for no frame.
    #[test]
    fn self_healing_is_on_windows_only() {
        if std::env::var_os(TEST_SWITCH).is_none() {
            assert_eq!(self_healing_enabled(), cfg!(windows));
        }
        assert!(test_switch_on(Some(" on ".to_string())));
        for other in [
            None,
            Some(String::new()),
            Some("1".to_string()),
            Some("ON!".to_string()),
        ] {
            assert!(!test_switch_on(other));
        }
        let folder = std::env::temp_dir().join(format!(
            "craft3d-graphics-off-{}-{}",
            std::process::id(),
            now()
        ));
        let mut off = Start::with_journal(Some(&folder), "0.1.2", true, false);
        assert_eq!(off.journal(), None);
        assert!(!off.frame(), "no frame is asked for");
        off.closing();
        assert_eq!(off.notice(None), None);
        assert!(!folder.exists(), "nothing is written");
        let on = Start::with_journal(Some(&folder), "0.1.2", false, true);
        assert_eq!(on.journal(), Some(journal_path(&folder).as_path()));
        // Without a state folder there is nothing to keep it in.
        let mut none = Start::with_journal(None, "0.1.2", false, true);
        assert!(!none.frame());
    }

    /// A start that comes up: "attempting" before the device, "ok" after
    /// three frames, and a close after that changes nothing; one closed
    /// before its third frame is "closing".
    #[test]
    fn a_start_is_recorded_from_attempting_to_ok_or_closing() {
        let folder = std::env::temp_dir().join(format!(
            "craft3d-graphics-on-{}-{}",
            std::process::id(),
            now()
        ));
        let attempt = pc2()[0].attempt.clone();
        let chosen = |start: &Start| {
            *start
                .chosen
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Chosen {
                attempt: attempt.clone(),
                choice: Choice {
                    index: 0,
                    skipped: Vec::new(),
                    retried: false,
                },
            });
        };
        let path = journal_path(&folder);
        let mut start = Start::with_journal(Some(&folder), "0.1.2", false, true);
        chosen(&start);
        let mut book = Journal::default();
        book.set(&attempt, Outcome::Attempting, now());
        write(&path, &book).unwrap();
        assert!(start.frame() && start.frame() && start.frame());
        assert_eq!(read(&path).records[0].outcome, Outcome::Attempting);
        assert!(!start.frame());
        assert_eq!(read(&path).records[0].outcome, Outcome::Ok);
        start.closing();
        assert_eq!(read(&path).records[0].outcome, Outcome::Ok);
        let mut quick = Start::with_journal(Some(&folder), "0.1.2", false, true);
        chosen(&quick);
        assert!(quick.frame());
        quick.closing();
        assert_eq!(read(&path).records[0].outcome, Outcome::Closing);
        // --reset-graphics forgets it.
        let _ = Start::with_journal(Some(&folder), "0.1.2", true, true);
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn the_order_on_windows_puts_directx_12_last_unless_dxc_is_there() {
        assert_eq!(windows().order, Some(vec![Vulkan, Gl, Dx12]));
        assert!(windows().skip);
        let dxc = rule(None, None, true, true);
        assert_eq!(dxc.order, Some(vec![Vulkan, Dx12, Gl]));
        // Elsewhere wgpu's own choice, still with the journal.
        let linux = rule(None, None, false, false);
        assert_eq!(linux.order, None);
        assert!(linux.skip);
        // The person's order is followed and never skipped.
        let given = rule(Some("gl,vulkan"), None, true, false);
        assert_eq!(given.order, Some(vec![Gl, Vulkan]));
        assert!(!given.skip);
        let wgpu_backend = rule(None, Some("dx12"), true, false);
        assert_eq!(wgpu_backend.order, None);
        assert!(!wgpu_backend.skip);
        // CRAFT3D_BACKEND_ORDER gives way to WGPU_BACKEND, and a bad one
        // is ignored.
        assert!(!rule(Some("gl"), Some("vulkan"), true, false).skip);
        assert_eq!(rule(Some("dx11"), None, true, false), windows());
    }

    #[test]
    fn a_clean_journal_starts_on_the_first_in_the_order() {
        let choice = choose(&pc2(), &windows(), &Journal::default(), HIGH, NOW).unwrap();
        assert_eq!(choice.index, 0);
        assert!(choice.skipped.is_empty() && !choice.retried);
        // wgpu's own choice: the hardware first, Vulkan before the others.
        let linux = rule(None, None, false, false);
        let choice = choose(&pc2(), &linux, &Journal::default(), HIGH, NOW).unwrap();
        assert_eq!(choice.index, 0);
    }

    #[test]
    fn an_interface_that_ended_the_start_is_skipped_and_said() {
        let offered = pc2();
        let mut journal = Journal::default();
        journal.set(&offered[0].attempt, Outcome::Attempting, NOW - 60);
        let choice = choose(&offered, &windows(), &journal, HIGH, NOW).unwrap();
        assert_eq!(choice.index, 3, "OpenGL, before DirectX 12");
        assert_eq!(choice.skipped.len(), 1);
        let said = notice_text(&offered[3].attempt, &choice).unwrap();
        assert_eq!(
            said,
            "3DCraft draws through OpenGL on this computer: starting with Vulkan failed here \
             (Intel(R) UHD Graphics 615, driver Intel Corporation 27.20.100.8681)."
        );
        // With dxcompiler.dll beside the program, DirectX 12 before OpenGL.
        let dxc = rule(None, None, true, true);
        assert_eq!(
            choose(&offered, &dxc, &journal, HIGH, NOW).unwrap().index,
            1
        );
        // OpenGL ended a start too: DirectX 12, the hardware before WARP.
        journal.set(&offered[3].attempt, Outcome::Attempting, NOW - 30);
        let choice = choose(&offered, &windows(), &journal, HIGH, NOW).unwrap();
        assert_eq!(choice.index, 1);
        assert_eq!(choice.skipped.len(), 2);
    }

    #[test]
    fn ok_and_closing_are_no_failure_and_old_records_do_not_count() {
        let offered = pc2();
        for outcome in [Outcome::Ok, Outcome::Closing] {
            let mut journal = Journal::default();
            journal.set(&offered[0].attempt, Outcome::Attempting, NOW - 100);
            journal.set(&offered[0].attempt, outcome, NOW - 90);
            let choice = choose(&offered, &windows(), &journal, HIGH, NOW).unwrap();
            assert_eq!(choice.index, 0, "{outcome:?}");
        }
        // Older than seven days, from a clock far ahead, of another app
        // version or another driver: tried again.
        let mut journal = Journal::default();
        journal.set(
            &offered[0].attempt,
            Outcome::Attempting,
            NOW - FRESH_FOR - 1,
        );
        assert_eq!(
            choose(&offered, &windows(), &journal, HIGH, NOW)
                .unwrap()
                .index,
            0
        );
        let mut journal = Journal::default();
        journal.set(
            &offered[0].attempt,
            Outcome::Attempting,
            NOW + 3 * 24 * 3600,
        );
        assert_eq!(
            choose(&offered, &windows(), &journal, HIGH, NOW)
                .unwrap()
                .index,
            0
        );
        for change in [
            |a: &mut Attempt| a.app = "0.1.3".to_string(),
            |a: &mut Attempt| a.driver_info = "31.0.101.2111".to_string(),
            |a: &mut Attempt| a.device = 0x46b3,
        ] {
            let mut other = offered[0].attempt.clone();
            change(&mut other);
            let mut journal = Journal::default();
            journal.set(&other, Outcome::Attempting, NOW - 60);
            assert_eq!(
                choose(&offered, &windows(), &journal, HIGH, NOW)
                    .unwrap()
                    .index,
                0
            );
        }
    }

    #[test]
    fn when_every_interface_ended_a_start_the_first_is_tried_again() {
        let offered = pc2();
        let mut journal = Journal::default();
        for o in &offered {
            journal.set(&o.attempt, Outcome::Attempting, NOW - 60);
        }
        let choice = choose(&offered, &windows(), &journal, HIGH, NOW).unwrap();
        assert_eq!(choice.index, 0);
        assert!(choice.retried);
        assert_eq!(choice.skipped.len(), 4);
        let said = notice_text(&offered[0].attempt, &choice).unwrap();
        assert!(said.contains("tries Vulkan again"), "{said}");
    }

    #[test]
    fn an_order_the_person_gives_is_never_skipped() {
        let offered = pc2();
        let mut journal = Journal::default();
        journal.set(&offered[0].attempt, Outcome::Attempting, NOW - 60);
        let given = rule(Some("vulkan,gl"), None, true, false);
        let choice = choose(&offered, &given, &journal, HIGH, NOW).unwrap();
        assert_eq!(choice.index, 0);
        assert!(choice.skipped.is_empty());
        assert_eq!(notice_text(&offered[0].attempt, &choice), None);
    }

    #[test]
    fn adapters_that_cannot_present_are_passed_and_none_is_an_error() {
        let mut offered = pc2();
        offered[0].candidate.presents = false;
        assert_eq!(
            choose(&offered, &windows(), &Journal::default(), HIGH, NOW)
                .unwrap()
                .index,
            3
        );
        for o in &mut offered {
            o.candidate.presents = false;
        }
        let why = choose(&offered, &windows(), &Journal::default(), HIGH, NOW).unwrap_err();
        assert!(why.contains("cannot present"), "{why}");
        assert!(choose(&[], &windows(), &Journal::default(), HIGH, NOW).is_err());
        // Within one interface: discrete before integrated.
        let two = vec![
            offered_kind("integrated", IntegratedGpu),
            offered_kind("discrete", DiscreteGpu),
        ];
        let linux = rule(None, None, false, false);
        assert_eq!(
            choose(&two, &linux, &Journal::default(), HIGH, NOW)
                .unwrap()
                .index,
            1
        );
    }

    fn offered_kind(name: &str, kind: wgpu::DeviceType) -> Offered {
        let mut o = offered(name, Vulkan, kind);
        o.attempt.device = name.len() as u32;
        o
    }

    #[test]
    fn the_journal_goes_through_its_file_and_keeps_dismissals() {
        let offered = pc2();
        let mut journal = Journal::default();
        journal.set(&offered[0].attempt, Outcome::Attempting, NOW);
        journal.dismiss(&offered[0].attempt);
        journal.set(&offered[3].attempt, Outcome::Ok, NOW);
        let text = journal.to_text();
        let back = Journal::parse(&text).unwrap();
        assert_eq!(back, journal);
        assert!(back.records[0].dismissed);
        // Recorded again the same way, it stays dismissed; another way, not.
        let mut again = back.clone();
        again.set(&offered[0].attempt, Outcome::Attempting, NOW + 5);
        assert!(again.records.last().unwrap().dismissed);
        again.set(&offered[0].attempt, Outcome::Ok, NOW + 6);
        assert!(!again.records.last().unwrap().dismissed);
        // Forgotten, it is tried again.
        again.set(&offered[0].attempt, Outcome::Attempting, NOW + 7);
        again.forget(&offered[0].attempt);
        assert!(again.killed(&offered[0].attempt, NOW + 8).is_none());
        let value: Value = serde_json::from_str(&text).unwrap();
        let first = value["records"][0].as_object().unwrap();
        for key in [
            "app",
            "backend",
            "adapter",
            "vendor",
            "device",
            "driver",
            "driverInfo",
        ] {
            assert!(has(first, key), "{key}");
        }
    }

    /// Hostile files: never a panic; what cannot be read is no journal.
    #[test]
    fn a_journal_that_cannot_be_read_is_none() {
        let good = {
            let mut journal = Journal::default();
            journal.set(&pc2()[0].attempt, Outcome::Attempting, NOW);
            journal.to_text()
        };
        for cut in 0..good.len() {
            if let Some(part) = good.get(..cut) {
                let _ = Journal::parse(part);
            }
        }
        for bad in [
            "",
            "garbage",
            "[]",
            "null",
            "{}",
            r#"{"version": 2, "records": []}"#,
            r#"{"version": "1", "records": []}"#,
            r#"{"version": 1}"#,
            r#"{"version": 1, "records": {}}"#,
        ] {
            assert_eq!(Journal::parse(bad), None, "{bad}");
        }
        // Records that cannot be read are left out, the others kept.
        let mixed = r#"{"version": 1, "records": [
            {"app": "0.1.2", "backend": "vulkan", "adapter": "a", "vendor": 1, "device": 2,
             "driver": "d", "driverInfo": "i", "outcome": "attempting", "time": 5},
            {"app": "0.1.2", "backend": "dx11"},
            {"app": "0.1.2", "backend": "gl", "adapter": "a", "vendor": 99999999999, "device": 2,
             "driver": "d", "driverInfo": "i", "outcome": "ok", "time": 5},
            {"app": "0.1.2", "backend": "gl", "adapter": "a", "vendor": 1, "device": 2,
             "driver": "d", "driverInfo": "i", "outcome": "maybe", "time": 5},
            7, "x", null
        ]}"#;
        let journal = Journal::parse(mixed).unwrap();
        assert_eq!(journal.records.len(), 1);
        // A huge file is none, and so many records keep the newest.
        let huge = format!(r#"{{"version": 1, "records": [{}]}}"#, " ".repeat(300_000));
        assert_eq!(Journal::parse(&huge), None);
        let mut many = Journal::default();
        for n in 0..200_u32 {
            let mut attempt = pc2()[0].attempt.clone();
            attempt.device = n;
            many.set(&attempt, Outcome::Ok, NOW);
        }
        assert_eq!(many.records.len(), MOST_RECORDS);
        assert_eq!(many.records.last().unwrap().attempt.device, 199);
        let long = format!(
            r#"{{"version": 1, "records": [{{"app": "{}", "backend": "vulkan", "adapter": "a",
            "vendor": 1, "device": 2, "driver": "d", "driverInfo": "i\u0007",
            "outcome": "ok", "time": 5}}]}}"#,
            "9".repeat(5000)
        );
        let journal = Journal::parse(&long).unwrap();
        assert_eq!(journal.records[0].attempt.app.len(), MOST_CHARS);
        assert_eq!(journal.records[0].attempt.driver_info, "i");
    }

    #[test]
    fn the_file_is_written_whole_and_read_back() {
        let folder = std::env::temp_dir().join(format!(
            "craft3d-graphics-start-{}-{}",
            std::process::id(),
            now()
        ));
        let path = journal_path(&folder.join("state"));
        assert_eq!(read(&path), Journal::default());
        let mut journal = Journal::default();
        journal.set(&pc2()[0].attempt, Outcome::Attempting, NOW);
        write(&path, &journal).unwrap();
        assert_eq!(read(&path), journal);
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(read(&path), Journal::default());
        std::fs::write(&path, vec![b'x'; 300_000]).unwrap();
        assert_eq!(read(&path), Journal::default());
        // No temporary file is left beside it.
        write(&path, &journal).unwrap();
        let names: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from(JOURNAL_FILE)]);
        // A folder that cannot be made is an error, never a panic.
        let blocked = folder.join("file");
        std::fs::write(&blocked, "").unwrap();
        assert!(write(&journal_path(&blocked), &journal).is_err());
        let _ = std::fs::remove_dir_all(&folder);
    }
}
