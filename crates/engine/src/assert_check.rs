//! Checking assertions without holding anyone up (docs/design/
//! verification.md › *Checking*).
//!
//! Each assertion is measured once per *what it depends on*: its own text
//! and bounds, the variables' values, the units and tolerance, and the
//! bodies (by their `Arc`, which regeneration keeps for a feature it did
//! not rebuild) and faces its subjects name. An edit that leaves those
//! alone reuses the last answer; the others are measured on a thread of
//! their own, one job at a time, while the caller gets "pending" for them
//! or waits as long as it chooses (the window never waits; an edit's
//! result waits [`EDIT_WAIT`]; `assert.list` and `document.check` wait
//! for every answer). The answers do not depend on which thread measured
//! them or when.
//!
//! The document's outputs that are measured (`variable.measure`) are
//! measured the same way, as assertions without a bound, and first: an
//! output or an assertion goes into a job once every output it uses is
//! computed, so a chain (the waterline, then the centre of buoyancy at
//! it, then the metacentric height from both) takes one job per link,
//! each measured once per what it depends on (`output_cmds.rs`).

use crate::assert_cmds::{AssertionState, AssertionStatus, Checked, check_one};
use crate::measure_pieces::PairAnswer;
use crate::output_cmds::{OutputValue, Plan, Resolved};
use crate::verify_cmds::{Ctx, subject};
use crate::{CommandError, Session};
use craft3d_doc::{Assertion, Document};
use craft3d_regen::Regen;
use craft3d_topo::Body;
use serde_json::{Map, Value};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

/// How many answers are kept beyond those of the current assertions.
const MEMO: usize = 256;

/// How long the result of an edit waits for the assertions it touched.
pub(crate) const EDIT_WAIT: Duration = Duration::from_secs(2);

/// How long a caller waits.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Wait {
    /// Not at all: what is not measured yet is pending.
    No,
    /// At most this long.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    For(Duration),
    /// Until every assertion is measured.
    All,
}

/// What an assertion depends on, and the bodies that key holds alive (so
/// that their addresses in it cannot be taken by other bodies).
#[derive(Clone)]
struct Key {
    text: String,
    _held: Vec<Arc<Body>>,
}

/// How many threads measure the assertions of one job at most (the extra
/// ones from the process's pool, `crate::pool`). Two: a
/// Jig's assertions are mostly quick with one or two that take long (the
/// gear reducer's flank clearances, 10 and 16 s), which then run side by
/// side (the reducer's 22 took 29 s one after the other, 15 s so). More
/// would not shorten that, and each measure may use threads of its own
/// (an overlap's pairs, up to eight), each with the kernel's budget for
/// its trees.
#[cfg(not(target_arch = "wasm32"))]
const MAX_THREADS: usize = 2;

/// The assertions being measured on other threads (or a piece at a time
/// by a host with one thread: [`Session::assertions_some`]), against the
/// document state and regeneration they were asked for.
struct Job {
    work: Vec<(Key, Assertion)>,
    results: Mutex<Vec<Option<Checked>>>,
    done: AtomicBool,
    /// The next assertion to take.
    next: AtomicUsize,
    /// The pairs of bodies the next assertion measured so far, taken a
    /// piece at a time ([`measure_some`]): which assertion, and its pairs.
    pieces: Mutex<Option<(usize, Vec<PairAnswer>)>>,
    doc: Arc<Document>,
    regen: Arc<Regen>,
    /// The outputs as far as they were computed when the job was made:
    /// every one its work uses.
    outputs: Arc<Resolved>,
}

/// The keys of one revision: of its measured outputs and of its
/// assertions, by index, each once every output it uses is computed.
#[derive(Clone, Default)]
struct Stage {
    outputs: Vec<Option<Key>>,
    assertions: Vec<Option<Key>>,
    /// The outputs once all are computed (none pending).
    resolved: Option<Arc<Resolved>>,
    /// How the outputs use each other.
    plan: Option<Arc<Plan>>,
}

impl Stage {
    fn keys(&self) -> impl Iterator<Item = &Key> {
        self.outputs.iter().chain(&self.assertions).flatten()
    }
}

/// What the checking of one open document keeps between calls.
#[derive(Clone, Default)]
pub(crate) struct Checks {
    /// The keys of one revision.
    keys: Option<(u64, Stage)>,
    /// Answers by key.
    memo: BTreeMap<String, (Checked, Vec<Arc<Body>>)>,
    job: Option<Arc<Job>>,
    /// How many assertions were measured, in all.
    measured: usize,
}

/// The bodies a subject names, written into the key with their addresses.
fn subjects_into(ctx: &Ctx<'_>, value: &Value, out: &mut String, held: &mut Vec<Arc<Body>>) {
    match subject(ctx, value, &["body", "face", "feature"]) {
        Err(e) => out.push_str(&format!("!{e}")),
        Ok(found) => {
            for f in found {
                out.push_str(&format!("{}:{:?}:{}", f.feature, f.indices, f.name));
                if let Some(body) = ctx.regen.feature(f.feature).and_then(|o| o.own_body()) {
                    out.push_str(&format!("@{:p};", Arc::as_ptr(body)));
                    held.push(Arc::clone(body));
                }
            }
        }
    }
}

/// What `assertion` (or an output's measure) depends on, with the
/// outputs it uses as they are computed.
fn key_of(doc: &Document, regen: &Regen, assertion: &Assertion, outputs: &Resolved) -> Key {
    let mut text = format!(
        "{}|{:?}|{:?}|{:?}|",
        assertion.measure, assertion.expect, doc.units, doc.tolerance
    );
    // The variables its bounds and its measure's own expressions name.
    let mut names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for e in assertion.expect.exprs() {
        names.extend(
            craft3d_expr::names_in(e.source())
                .into_iter()
                .map(str::to_string),
        );
    }
    if let Ok(value) = serde_json::from_str::<Value>(&assertion.measure) {
        let mut stack = vec![(&value, 0_usize)];
        while let Some((v, depth)) = stack.pop() {
            match v {
                Value::String(t) => {
                    names.extend(craft3d_expr::names_in(t).into_iter().map(str::to_string));
                }
                Value::Array(items) if depth < 64 => {
                    stack.extend(items.iter().map(|i| (i, depth + 1)))
                }
                Value::Object(map) if depth < 64 => {
                    stack.extend(map.values().map(|i| (i, depth + 1)))
                }
                _ => {}
            }
        }
    }
    for v in regen.variables.iter().filter(|v| names.contains(&v.name)) {
        text.push_str(&format!("{}={:?};", v.name, v.value));
    }
    // A table its expressions look up in directly (`belt[2].pitch`): a
    // table edit changes no variable it names.
    for table in doc.tables.iter().filter(|t| names.contains(&t.name)) {
        text.push_str(&format!("{}[]={table:?};", table.name));
    }
    text.push_str(&outputs.key_text(&names));
    let mut held = Vec::new();
    let Ok(ctx) = Ctx::new(doc, regen) else {
        return Key { text, _held: held };
    };
    let params: Map<String, Value> = serde_json::from_str(&assertion.measure).unwrap_or_default();
    for key in ["of", "from", "to"] {
        text.push('|');
        match params.get(key) {
            Some(v) if !v.is_null() => subjects_into(&ctx, v, &mut text, &mut held),
            // Every body.
            _ if key == "of" => {
                for (id, body) in regen.bodies() {
                    text.push_str(&format!("{id}@{:p};", Arc::as_ptr(body)));
                    held.push(Arc::clone(body));
                }
            }
            _ => {}
        }
    }
    // A mass follows the materials, which change no body, and whether the
    // feature each is given to is off (a material of a feature that is off
    // weighs the body it passes on only where that body has none: a
    // fillet left with no edges passes on its source's very body, so
    // switching it changes the material and no body).
    if matches!(
        params.get("measure").and_then(Value::as_str),
        Some("mass" | "centerOfMass")
    ) {
        text.push('|');
        let off: std::collections::BTreeSet<craft3d_doc::FeatureId> = regen
            .features
            .iter()
            .filter(|r| r.is_off())
            .map(|r| r.id)
            .collect();
        let values = doc
            .bodies
            .iter()
            .any(|b| {
                b.material
                    .as_ref()
                    .is_some_and(|m| m.density_expr.is_some())
            })
            .then(|| crate::appearance::evaluation(doc))
            .flatten();
        for given in &doc.bodies {
            if let Some(material) = &given.material {
                let state = if off.contains(&given.feature) {
                    "off"
                } else {
                    "on"
                };
                // A density that follows the variables: its value now.
                let density = material.density_with(values.as_ref());
                text.push_str(&format!(
                    "{}={material:?}:{state}={density:?};",
                    given.feature
                ));
            }
        }
    }
    // A datum plane moves without a body changing.
    if let Some(Value::String(name)) = params.get("plane")
        && let Some(f) = doc.feature_by_name(name).map(|f| f.id).or_else(|| {
            name.strip_prefix('F')
                .and_then(|d| d.parse().ok())
                .map(craft3d_doc::FeatureId)
        })
    {
        text.push_str(&format!(
            "|{:?}",
            regen.feature(f).and_then(|o| o.datum.as_ref())
        ));
    }
    Key { text, _held: held }
}

/// A placeholder for an assertion still being measured.
fn pending(assertion: &Assertion) -> Checked {
    let params: Map<String, Value> = serde_json::from_str(&assertion.measure).unwrap_or_default();
    Checked {
        state: AssertionState {
            name: assertion.name.clone(),
            measure: params
                .get("measure")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            status: AssertionStatus::Pending,
            measured: None,
            expected: String::new(),
            message: "being measured".into(),
        },
        stored: assertion.measure.clone(),
        value: Value::Null,
        unit: None,
        detail: Map::new(),
    }
}

/// Measures the assertions of `job` that nobody took yet, one after the
/// other, until `stop` says so after one or none are left. Several
/// threads may run this on one job: each assertion is taken by one of
/// them and its answer goes to its own place, so the answers do not
/// depend on who measured them. The job is done once every answer is in.
fn measure(job: &Job, mut stop: impl FnMut() -> bool) {
    loop {
        let n = job.next.fetch_add(1, Ordering::Relaxed);
        let Some((_, assertion)) = job.work.get(n) else {
            return;
        };
        let checked = check_one(&job.doc, &job.regen, assertion, &job.outputs);
        let mut results = job.results.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(slot) = results.get_mut(n) {
            *slot = Some(checked);
        }
        if results.iter().all(Option::is_some) {
            job.done.store(true, Ordering::Release);
            return;
        }
        drop(results);
        if stop() {
            return;
        }
    }
}

/// For a host with one thread ([`Session::assertions_some`]): measures
/// the assertions of `job` from the next one on, until `stop` says so.
/// One over many pairs of bodies (a clearance, an overlap) is measured a
/// pair at a time (`measure_pieces`): `stop` is asked between its pairs
/// too, and the pairs measured so far wait in the job for the next call,
/// which goes on from there. At least one pair, or one assertion, is
/// measured per call. The answers are those [`measure`] gives.
fn measure_some(job: &Job, stop: impl FnMut() -> bool + 'static) {
    let stop = Rc::new(RefCell::new(stop));
    loop {
        let n = job.next.load(Ordering::Acquire);
        let Some((_, assertion)) = job.work.get(n) else {
            return;
        };
        let earlier = match job
            .pieces
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            Some((at, answers)) if at == n => answers,
            _ => Vec::new(),
        };
        let asked = Rc::clone(&stop);
        let taken = crate::measure_pieces::in_pieces(
            earlier,
            Box::new(move || (asked.borrow_mut())()),
            || check_one(&job.doc, &job.regen, assertion, &job.outputs),
        );
        if taken.stopped {
            *job.pieces.lock().unwrap_or_else(PoisonError::into_inner) = Some((n, taken.answers));
            return;
        }
        job.next.store(n.saturating_add(1), Ordering::Release);
        let mut results = job.results.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(slot) = results.get_mut(n) {
            *slot = Some(taken.out);
        }
        if results.iter().all(Option::is_some) {
            job.done.store(true, Ordering::Release);
            return;
        }
        drop(results);
        if (stop.borrow_mut())() {
            return;
        }
    }
}

/// Measures every assertion of `job`, several at once where there are
/// threads ([`MAX_THREADS`] at most), and then says it is done. One that
/// no thread answered (a thread that could not start takes none; one that
/// stopped leaves its own) is an error, never pending for ever.
fn run(job: &Job) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let threads = craft3d_math::available_threads()
            .min(MAX_THREADS)
            .min(job.work.len());
        // From the process's one pool of threads for heavy work: what the
        // measures start of their own comes from it too.
        let lease = crate::pool::lease(threads.saturating_sub(1));
        std::thread::scope(|scope| {
            let workers: Vec<_> = (0..lease.threads())
                .filter_map(|_| {
                    std::thread::Builder::new()
                        .name("craft3d-assertions".into())
                        .spawn_scoped(scope, || measure(job, || false))
                        .ok()
                })
                .collect();
            measure(job, || false);
            for worker in workers {
                let _ = worker.join();
            }
        });
    }
    measure(job, || false);
    let mut results = job.results.lock().unwrap_or_else(PoisonError::into_inner);
    for (slot, (_, assertion)) in results.iter_mut().zip(&job.work) {
        if slot.is_none() {
            let mut failed = pending(assertion);
            failed.state.status = AssertionStatus::Error;
            failed.state.message =
                "internal error (a bug in 3DCraft, please report it): not measured".into();
            *slot = Some(failed);
        }
    }
    drop(results);
    job.done.store(true, Ordering::Release);
}

/// Starts `job` on a thread of its own (inline where there are none, or
/// when a thread cannot be had); not at all for a host that measures a
/// piece at a time ([`Session::assertions_some`]).
fn start(job: Arc<Job>, stepwise: bool) {
    if stepwise {
        return;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let for_thread = Arc::clone(&job);
        let spawned = std::thread::Builder::new()
            .name("craft3d-assertions".into())
            .spawn(move || run(&for_thread));
        if spawned.is_ok() {
            return;
        }
    }
    run(&job);
}

impl Session {
    /// The assertions of the active document checked against it as it is
    /// now, waiting as `wait` says; those not measured yet are pending.
    pub(crate) fn assertion_checks(&mut self, wait: Wait) -> Result<Vec<Checked>, CommandError> {
        Ok(self.checks_and_outputs(wait)?.0)
    }

    /// [`Session::assertion_checks`], with the document's outputs as far
    /// as they are computed: measured outputs and assertions are measured
    /// in stages, each once every output it uses is known (or unknown).
    pub(crate) fn checks_and_outputs(
        &mut self,
        wait: Wait,
    ) -> Result<(Vec<Checked>, Arc<Resolved>), CommandError> {
        let stepwise = self.stepwise;
        let open = self.document_mut()?;
        let doc = Arc::clone(open.history.current());
        if doc.assertions.is_empty() && doc.outputs.is_empty() {
            return Ok((Vec::new(), Arc::new(Resolved::default())));
        }
        let revision = open.history.revision();
        let regen = open.regen();
        let checks = &mut open.assertions;
        if checks.keys.as_ref().is_none_or(|(at, _)| *at != revision) {
            let stage = Stage {
                outputs: vec![None; doc.outputs.len()],
                assertions: vec![None; doc.assertions.len()],
                resolved: None,
                plan: (!doc.outputs.is_empty()).then(|| Arc::new(Plan::of(&doc))),
            };
            checks.keys = Some((revision, stage));
        }
        // Taken when first needed: a browser has no clock to ask, and
        // there every job runs to its end before this is reached.
        #[cfg(not(target_arch = "wasm32"))]
        let mut deadline: Option<std::time::Instant> = None;
        loop {
            // What a finished job measured. A host that measures a piece
            // at a time stops a job of another state of the document (an
            // edit came since) at once, keeping what it measured: on one
            // thread, finishing it would hold up the answers wanted now.
            if let Some(job) = checks.job.take() {
                let current = |key: &Key| {
                    checks
                        .keys
                        .as_ref()
                        .is_some_and(|(_, stage)| stage.keys().any(|k| k.text == key.text))
                };
                let stale = stepwise && job.work.iter().any(|(key, _)| !current(key));
                if job.done.load(Ordering::Acquire) || stale {
                    let results = job.results.lock().unwrap_or_else(PoisonError::into_inner);
                    for ((key, _), result) in job.work.iter().zip(results.iter()) {
                        if let Some(checked) = result {
                            checks.measured = checks.measured.saturating_add(1);
                            checks
                                .memo
                                .insert(key.text.clone(), (checked.clone(), key._held.clone()));
                        }
                    }
                } else {
                    checks.job = Some(job);
                }
            }
            // What the job in progress has measured so far, by key: shown,
            // though kept only once the job is done.
            let partial: BTreeMap<String, Checked> = checks
                .job
                .as_ref()
                .map(|job| {
                    let results = job.results.lock().unwrap_or_else(PoisonError::into_inner);
                    job.work
                        .iter()
                        .zip(results.iter())
                        .filter_map(|((key, _), r)| Some((key.text.clone(), r.clone()?)))
                        .collect()
                })
                .unwrap_or_default();
            let Some((_, stage)) = checks.keys.as_mut() else {
                return Ok((Vec::new(), Arc::new(Resolved::default())));
            };
            let memo = &checks.memo;
            let answer = |key: &Key| {
                memo.get(&key.text)
                    .map(|(c, _)| (c.clone(), true))
                    .or_else(|| partial.get(&key.text).map(|c| (c.clone(), false)))
            };
            let mut missing = Vec::new();
            // The outputs, stage by stage.
            let resolved = match (&stage.resolved, &stage.plan) {
                (Some(resolved), _) => Arc::clone(resolved),
                (None, None) => Arc::new(Resolved::default()),
                (None, Some(plan)) => {
                    let plan = Arc::clone(plan);
                    let keys = &mut stage.outputs;
                    let resolved = crate::output_cmds::resolve(&doc, &plan, |i, output, known| {
                        let Some(pseudo) = crate::output_cmds::as_assertion(output) else {
                            return OutputValue::Unknown("internal error: not a measure".into());
                        };
                        let Some(slot) = keys.get_mut(i) else {
                            return OutputValue::Unknown("internal error: no key".into());
                        };
                        let key = slot.get_or_insert_with(|| key_of(&doc, &regen, &pseudo, known));
                        match answer(key) {
                            Some((checked, kept)) => {
                                if !kept {
                                    missing.push((key.clone(), pseudo));
                                }
                                crate::output_cmds::from_checked(&doc, &checked)
                            }
                            None => {
                                missing.push((key.clone(), pseudo));
                                OutputValue::Pending
                            }
                        }
                    });
                    let resolved = Arc::new(resolved);
                    // Kept once every output is computed and remembered.
                    let all_kept = stage
                        .outputs
                        .iter()
                        .flatten()
                        .all(|k| memo.contains_key(&k.text));
                    if !resolved.any_pending() && all_kept {
                        stage.resolved = Some(Arc::clone(&resolved));
                    }
                    resolved
                }
            };
            let mut out = Vec::with_capacity(doc.assertions.len());
            for (j, assertion) in doc.assertions.iter().enumerate() {
                let key = match stage.assertions.get_mut(j) {
                    Some(Some(key)) => Some(key.clone()),
                    Some(slot) => {
                        let ready = doc.outputs.is_empty()
                            || crate::output_cmds::assertion_uses(&doc, assertion)
                                .iter()
                                .all(|n| !matches!(resolved.get(n), Some(OutputValue::Pending)));
                        ready.then(|| {
                            slot.insert(key_of(&doc, &regen, assertion, &resolved))
                                .clone()
                        })
                    }
                    None => None,
                };
                let Some(key) = key else {
                    // Waiting for an output it uses, which is measured
                    // first.
                    let mut c = pending(assertion);
                    c.state.message = "waiting for the outputs it uses".into();
                    out.push(c);
                    continue;
                };
                match answer(&key) {
                    // The answer is the assertion's, whatever its name was
                    // when it was measured.
                    Some((mut c, kept)) => {
                        c.state.name.clone_from(&assertion.name);
                        out.push(c);
                        if !kept {
                            missing.push((key, assertion.clone()));
                        }
                    }
                    None => {
                        let mut c = pending(assertion);
                        c.state.name.clone_from(&assertion.name);
                        out.push(c);
                        missing.push((key, assertion.clone()));
                    }
                }
            }
            // Everything measured and kept: nothing waits (an output or an
            // assertion waits only for an output that is missing).
            if missing.is_empty() {
                // What other states measured stays for an undo, within a
                // bound: past it, all but what now depends on goes.
                let wanted: std::collections::BTreeSet<String> =
                    stage.keys().map(|k| k.text.clone()).collect();
                if checks.memo.len() > MEMO.max(4 * wanted.len()) {
                    checks.memo.retain(|k, _| wanted.contains(k));
                }
                return Ok((out, resolved));
            }
            if checks.job.is_none() {
                let job = Arc::new(Job {
                    results: Mutex::new(vec![None; missing.len()]),
                    work: missing,
                    done: AtomicBool::new(false),
                    next: AtomicUsize::new(0),
                    pieces: Mutex::new(None),
                    doc: Arc::clone(&doc),
                    regen: Arc::clone(&regen),
                    outputs: Arc::clone(&resolved),
                });
                checks.job = Some(Arc::clone(&job));
                start(job, stepwise);
                continue;
            }
            // A host that measures a piece at a time: what a caller waits
            // for to the end is measured here and now; one that waits a
            // while (an edit's result) gets what is measured, the rest
            // pending, as one that does not wait.
            if stepwise {
                match (&checks.job, wait) {
                    (Some(job), Wait::All) => {
                        run(job);
                        continue;
                    }
                    _ => return Ok((out, resolved)),
                }
            }
            let more = match wait {
                Wait::No => false,
                Wait::All => true,
                #[cfg(not(target_arch = "wasm32"))]
                Wait::For(d) => {
                    let end = *deadline.get_or_insert_with(|| std::time::Instant::now() + d);
                    std::time::Instant::now() < end
                }
                #[cfg(target_arch = "wasm32")]
                Wait::For(_) => false,
            };
            if !more {
                return Ok((out, resolved));
            }
            #[cfg(not(target_arch = "wasm32"))]
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// The assertions of the active document and how each stands, in the
    /// document's order, without waiting (those being measured are
    /// pending); empty without a document or assertions.
    pub fn assertion_states(&mut self) -> Vec<AssertionState> {
        self.assertion_checks(Wait::No)
            .map(|c| c.into_iter().map(|c| c.state).collect())
            .unwrap_or_default()
    }

    /// How many assertions of the active document were measured since it
    /// was opened: an edit that leaves an assertion's subjects alone does
    /// not measure it again (for tests).
    pub fn assertions_measured(&self) -> Option<usize> {
        Some(self.documents.get(self.active)?.assertions.measured)
    }

    /// For a host that measures a piece at a time ([`Session::prepare_some`]):
    /// measures assertions of the active document that are not measured
    /// yet, one after the other until `stop` says so after one, and one
    /// over many pairs of bodies (a clearance, an overlap) a pair at a
    /// time, so that no call takes as long as all its pairs. `None`
    /// when every one is measured (or there are none); otherwise how many
    /// are measured of how many.
    pub(crate) fn assertions_some(
        &mut self,
        stop: impl FnMut() -> bool + 'static,
    ) -> Option<(usize, usize)> {
        let checked = self.checks_and_outputs(Wait::No).ok()?;
        let job = self.document_mut().ok()?.assertions.job.clone();
        if let Some(job) = &job
            && !job.done.load(Ordering::Acquire)
        {
            measure_some(job, stop);
        }
        let (checked, outputs) = if job.is_some() {
            self.checks_and_outputs(Wait::No).ok()?
        } else {
            checked
        };
        // The outputs count as what they are measured with.
        let doc = Arc::clone(self.document().ok()?.history.current());
        let total = checked.len().saturating_add(doc.outputs.len());
        let pending = checked
            .iter()
            .filter(|c| c.state.status == AssertionStatus::Pending)
            .count()
            .saturating_add(
                doc.outputs
                    .iter()
                    .filter(|o| matches!(outputs.get(&o.name), Some(OutputValue::Pending)))
                    .count(),
            );
        (pending > 0).then(|| (total.saturating_sub(pending), total))
    }

    /// Whether some assertion or output of the active document is still
    /// being measured: a viewer asks again soon.
    pub fn assertions_pending(&mut self) -> bool {
        self.checks_and_outputs(Wait::No)
            .is_ok_and(|(checked, outputs)| {
                outputs.any_pending()
                    || checked
                        .iter()
                        .any(|c| c.state.status == AssertionStatus::Pending)
            })
    }
}
