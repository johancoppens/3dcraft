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

use crate::assert_cmds::{AssertionState, AssertionStatus, Checked, check_one};
use crate::verify_cmds::{Ctx, subject};
use crate::{CommandError, Session};
use craft3d_doc::{Assertion, Document};
use craft3d_regen::Regen;
use craft3d_topo::Body;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
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

/// The assertions being measured on another thread.
struct Job {
    work: Vec<(Key, Assertion)>,
    results: Mutex<Vec<Option<Checked>>>,
    done: AtomicBool,
}

/// What the checking of one open document keeps between calls.
#[derive(Clone, Default)]
pub(crate) struct Checks {
    /// The keys of the assertions of one revision, in order.
    keys: Option<(u64, Vec<Key>)>,
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

/// What `assertion` depends on.
fn key_of(doc: &Document, regen: &Regen, assertion: &Assertion) -> Key {
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

/// Measures `work` against `doc` and `regen` into `job`.
fn run(job: &Job, doc: &Document, regen: &Regen) {
    for (n, (_, assertion)) in job.work.iter().enumerate() {
        let checked = check_one(doc, regen, assertion);
        let mut results = job.results.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(slot) = results.get_mut(n) {
            *slot = Some(checked);
        }
    }
    job.done.store(true, Ordering::Release);
}

/// Starts `job` on a thread of its own (inline where there are none, or
/// when a thread cannot be had).
fn start(job: Arc<Job>, doc: Arc<Document>, regen: Arc<Regen>) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let for_thread = Arc::clone(&job);
        let (d, r) = (Arc::clone(&doc), Arc::clone(&regen));
        let spawned = std::thread::Builder::new()
            .name("craft3d-assertions".into())
            .spawn(move || run(&for_thread, &d, &r));
        if spawned.is_ok() {
            return;
        }
    }
    run(&job, &doc, &regen);
}

impl Session {
    /// The assertions of the active document checked against it as it is
    /// now, waiting as `wait` says; those not measured yet are pending.
    pub(crate) fn assertion_checks(&mut self, wait: Wait) -> Result<Vec<Checked>, CommandError> {
        let open = self.document_mut()?;
        let doc = Arc::clone(open.history.current());
        if doc.assertions.is_empty() {
            return Ok(Vec::new());
        }
        let revision = open.history.revision();
        let regen = open.regen();
        let checks = &mut open.assertions;
        if checks.keys.as_ref().is_none_or(|(at, _)| *at != revision) {
            let keys = doc
                .assertions
                .iter()
                .map(|a| key_of(&doc, &regen, a))
                .collect();
            checks.keys = Some((revision, keys));
        }
        // Taken when first needed: a browser has no clock to ask, and
        // there every job runs to its end before this is reached.
        #[cfg(not(target_arch = "wasm32"))]
        let mut deadline: Option<std::time::Instant> = None;
        loop {
            // What a finished job measured.
            if let Some(job) = checks.job.take() {
                if job.done.load(Ordering::Acquire) {
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
            let keys = checks
                .keys
                .as_ref()
                .map(|(_, k)| k.as_slice())
                .unwrap_or(&[]);
            let mut out = Vec::with_capacity(keys.len());
            let mut missing = Vec::new();
            for (assertion, key) in doc.assertions.iter().zip(keys) {
                match checks.memo.get(&key.text) {
                    // The answer is the assertion's, whatever its name was
                    // when it was measured.
                    Some((checked, _)) => {
                        let mut c = checked.clone();
                        c.state.name.clone_from(&assertion.name);
                        out.push(c);
                    }
                    None => {
                        out.push(pending(assertion));
                        missing.push((key.clone(), assertion.clone()));
                    }
                }
            }
            if missing.is_empty() {
                // What other states measured stays for an undo, within a
                // bound: past it, all but what now depends on goes.
                if checks.memo.len() > MEMO.max(4 * keys.len()) {
                    let wanted: std::collections::BTreeSet<&String> =
                        keys.iter().map(|k| &k.text).collect();
                    checks.memo.retain(|k, _| wanted.contains(k));
                }
                return Ok(out);
            }
            if checks.job.is_none() {
                let job = Arc::new(Job {
                    results: Mutex::new(vec![None; missing.len()]),
                    work: missing,
                    done: AtomicBool::new(false),
                });
                checks.job = Some(Arc::clone(&job));
                start(job, Arc::clone(&doc), Arc::clone(&regen));
                continue;
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
                return Ok(out);
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

    /// Whether some assertion of the active document is still being
    /// measured: a viewer asks again soon.
    pub fn assertions_pending(&mut self) -> bool {
        self.assertion_states()
            .iter()
            .any(|a| a.status == AssertionStatus::Pending)
    }
}
