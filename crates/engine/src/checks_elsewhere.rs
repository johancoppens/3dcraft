//! Checking elsewhere: the assertions and outputs of a job
//! (`assert_check.rs`) measured by another instance of the engine, such
//! as the page's Web Worker, so that the host's own thread never measures
//! (docs/design/engine.md › *Checks in a Web Worker*).
//!
//! A host that says so ([`Session::set_checks_elsewhere`]) takes the job
//! of the active document that waits for answers ([`Session::checks_job`],
//! a number), has it written out ([`Session::checks_request`]: the job's
//! document as the bytes of its `.3dcraft` file, with the files it holds
//! such as an imported STEP file, and its work, the assertions and
//! the outputs known so far, as JSON) and hands that to the other side,
//! which answers with [`measure_checks`]. The answer goes back with
//! [`Session::checks_answered`]; from then on the job is done as if this
//! session had measured it, and its answers are kept and reused the same
//! way.
//!
//! The answers are the ones this session would have measured, bit for
//! bit: regeneration is a function of the document alone (and a saved
//! document regenerates to the bodies it was saved from), a check is a
//! function of the document, its regeneration, the assertion and the
//! outputs it uses (`assert_cmds::check_one`), and the numbers cross as
//! JSON written in the shortest form that reads back as the same `f64`
//! (`serde_json`'s `float_roundtrip`; a quantity's value goes as its bits).
//! What cannot be measured elsewhere (the other side stopped, or its
//! answer does not fit the job) becomes the error of every assertion and
//! output of the job, with the reason: never pending for ever, and never
//! measured here instead, since what stopped the other side would stop
//! this one too.

use crate::Session;
use crate::assert_check::{Wait, pending};
use crate::assert_cmds::{AssertionState, AssertionStatus, Checked, check_one};
use crate::output_cmds::{OutputValue, Resolved};
use craft3d_doc::{Assertion, Document};
use craft3d_expr::{Dimension, Quantity};
use craft3d_regen::RegenCache;
use serde_json::{Map, Value, json};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, PoisonError};

/// A job of checks, written out to be measured elsewhere.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckRequest {
    /// What its answer goes back with ([`Session::checks_answered`]).
    pub token: u64,
    /// The document the job measures, as the bytes of its `.3dcraft`
    /// file (with the files it holds: an imported STEP file).
    pub document: Vec<u8>,
    /// What to measure: `{assertions, outputs}`.
    pub work: String,
}

const STATUSES: [AssertionStatus; 6] = [
    AssertionStatus::Passed,
    AssertionStatus::Failed,
    AssertionStatus::NotProven,
    AssertionStatus::Pending,
    AssertionStatus::Error,
    AssertionStatus::Inactive,
];

impl Session {
    /// Has the assertions and outputs measured elsewhere from now on (by a
    /// host that does the long work a piece at a time,
    /// [`Session::set_stepwise`]): [`Session::prepare_some`] no longer
    /// measures them, and the host hands their jobs over
    /// ([`Session::checks_job`]). A command that waits for every answer
    /// (`assert.list`, `document.check`) still measures them in line.
    /// Off again, the jobs not answered yet are measured here.
    pub fn set_checks_elsewhere(&mut self, elsewhere: bool) {
        self.checks_elsewhere = elsewhere;
    }

    pub fn checks_elsewhere(&self) -> bool {
        self.checks_elsewhere
    }

    /// The job of checks of the active document that waits for answers
    /// from elsewhere, by its number (given it the first time it is
    /// asked for); `None` when there is none, when checks are not measured
    /// elsewhere, or while the document's state is still being
    /// regenerated (its checks come after). The same job keeps its number
    /// until it is answered or replaced (an edit, a stage of outputs
    /// done): a host whose number changed drops the work it started for
    /// the old one.
    pub fn checks_job(&mut self) -> Option<u64> {
        if !(self.checks_elsewhere && self.stepwise) {
            return None;
        }
        // Regenerated elsewhere: a whole state at a time.
        if self.regen_elsewhere() {
            return self.state_checks_job();
        }
        if self.regeneration_deferred() {
            return None;
        }
        // Collects what a finished job measured and makes the next one.
        self.checks_and_outputs(Wait::No).ok()?;
        let job = Arc::clone(self.documents.get(self.active)?.assertions.job.as_ref()?);
        if job.done.load(Ordering::Acquire) {
            return None;
        }
        let token = job.token.load(Ordering::Acquire);
        if token != 0 {
            return Some(token);
        }
        self.checks_token = self.checks_token.wrapping_add(1).max(1);
        job.token.store(self.checks_token, Ordering::Release);
        Some(self.checks_token)
    }

    /// The job numbered `token` ([`Session::checks_job`]), written out for
    /// [`measure_checks`]. An error when no open document has that job
    /// waiting any more, or its document cannot be written.
    pub fn checks_request(&self, token: u64) -> Result<CheckRequest, String> {
        if let Some((_, doc)) = self.state_job(token) {
            let document = craft3d_format::to_bytes(&doc)
                .map_err(|e| format!("the document cannot be written out: {e}"))?;
            return Ok(CheckRequest {
                token,
                document,
                work: json!({ "state": true }).to_string(),
            });
        }
        let job = self
            .job_with_token(token)
            .ok_or_else(|| format!("no job of checks {token} waits for answers"))?;
        let document = craft3d_format::to_bytes(&job.doc)
            .map_err(|e| format!("the document cannot be written out: {e}"))?;
        let assertions: Vec<&Assertion> = job.work.iter().map(|(_, a)| a).collect();
        let work = json!({
            "assertions": assertions,
            "outputs": outputs_json(&job.outputs),
        });
        Ok(CheckRequest {
            token,
            document,
            work: work.to_string(),
        })
    }

    /// The answer of job `token`: what [`measure_checks`] returned, or why
    /// it could not be had (the other side stopped). The job is then done:
    /// with the answers, or with every one of its assertions and outputs
    /// an error that says why. False when no open document has that job
    /// waiting any more (an edit replaced it, it was measured here): the
    /// answer is dropped.
    pub fn checks_answered(&mut self, token: u64, answer: Result<&str, &str>) -> bool {
        if let Some((at, doc)) = self.state_job(token) {
            let decoded = match answer {
                Ok(text) => decode_state(text, &doc),
                Err(why) => Err(format!("the worker measuring it stopped: {why}")),
            };
            let (checked, outputs) = decoded.unwrap_or_else(|why| {
                failed_state(
                    &doc,
                    &format!(
                        "internal error (a bug in 3DCraft, please report it): could not be measured off the page's thread: {why}"
                    ),
                )
            });
            self.state_answered(at, doc, checked, outputs);
            return true;
        }
        let Some(job) = self.job_with_token(token) else {
            return false;
        };
        let decoded = match answer {
            Ok(text) => decode_answer(text, job.work.len()),
            Err(why) => Err(format!("the worker measuring it stopped: {why}")),
        };
        let checked: Vec<Checked> = match decoded {
            Ok(checked) => checked,
            Err(why) => job
                .work
                .iter()
                .map(|(_, assertion)| {
                    let mut failed = pending(assertion);
                    failed.state.status = AssertionStatus::Error;
                    failed.state.message = format!(
                        "internal error (a bug in 3DCraft, please report it): could not be measured off the page's thread: {why}"
                    );
                    failed
                })
                .collect(),
        };
        let mut results = job.results.lock().unwrap_or_else(PoisonError::into_inner);
        for (slot, answer) in results.iter_mut().zip(checked) {
            *slot = Some(answer);
        }
        drop(results);
        job.done.store(true, Ordering::Release);
        true
    }

    /// Job `token` could not be measured, for a reason that is no bug
    /// (the worker took too long, the document cannot be written out):
    /// every one of its checks is an error that says `why` as it is. The
    /// job is then done. False when it no longer waits.
    pub fn checks_failed(&mut self, token: u64, why: &str) -> bool {
        if let Some((at, doc)) = self.state_job(token) {
            let (checked, outputs) = failed_state(&doc, &format!("not measured: {why}"));
            self.state_answered(at, doc, checked, outputs);
            return true;
        }
        let Some(job) = self.job_with_token(token) else {
            return false;
        };
        let mut results = job.results.lock().unwrap_or_else(PoisonError::into_inner);
        for (slot, (_, assertion)) in results.iter_mut().zip(&job.work) {
            let mut failed = pending(assertion);
            failed.state.status = AssertionStatus::Error;
            failed.state.message = format!("not measured: {why}");
            *slot = Some(failed);
        }
        drop(results);
        job.done.store(true, Ordering::Release);
        true
    }

    /// The job numbered `token` of any open document, while it waits.
    fn job_with_token(&self, token: u64) -> Option<Arc<crate::assert_check::Job>> {
        if token == 0 {
            return None;
        }
        self.documents.iter().find_map(|open| {
            let job = open.assertions.job.as_ref()?;
            (job.token.load(Ordering::Acquire) == token && !job.done.load(Ordering::Acquire))
                .then(|| Arc::clone(job))
        })
    }
}

thread_local! {
    /// What the regenerations of earlier requests left, on this side: the
    /// next state of a document rebuilds only what changed. The result is
    /// that of regenerating from nothing (`craft3d_regen::regenerate_cached`).
    static CACHE: RefCell<RegenCache> = RefCell::new(RegenCache::default());
}

/// Measures a job of checks written out by [`Session::checks_request`]:
/// regenerates its document and measures its work in order. Returns the
/// answer for [`Session::checks_answered`], `{"checked": [...]}`, or
/// `{"error": why}` when the request cannot be read. Never panics on any
/// input.
pub fn measure_checks(document: &[u8], work: &str) -> String {
    if serde_json::from_str::<Value>(work).is_ok_and(|w| w.get("state") == Some(&Value::Bool(true)))
    {
        return measure_state(document);
    }
    let measured = (|| -> Result<Vec<Value>, String> {
        let doc = craft3d_format::from_bytes(document)
            .map_err(|e| format!("the document cannot be read: {e}"))?;
        let work: Value =
            serde_json::from_str(work).map_err(|e| format!("the work cannot be read: {e}"))?;
        let assertions: Vec<Assertion> =
            serde_json::from_value(work.get("assertions").cloned().unwrap_or(Value::Null))
                .map_err(|e| format!("the assertions cannot be read: {e}"))?;
        let outputs = outputs_from(work.get("outputs").unwrap_or(&Value::Null))?;
        let regen = CACHE.with(|cache| match cache.try_borrow_mut() {
            Ok(mut cache) => craft3d_regen::regenerate_cached(&doc, &mut cache),
            Err(_) => craft3d_regen::regenerate(&doc),
        });
        Ok(assertions
            .iter()
            .map(|a| checked_json(&check_one(&doc, &regen, a, &outputs, None)))
            .collect())
    })();
    match measured {
        Ok(checked) => json!({ "checked": checked }).to_string(),
        Err(why) => json!({ "error": why }).to_string(),
    }
}

thread_local! {
    /// The session that measures whole states on this side: the next
    /// state of a document rebuilds only what changed, and an assertion
    /// whose subjects did not change is not measured again.
    static STATES: RefCell<Option<Session>> = const { RefCell::new(None) };
}

/// Measures every assertion and output of the document state `document`
/// (a job of checks of a whole state: the host regenerates elsewhere,
/// `regen_elsewhere.rs`). `{"checked": [...], "outputs": [...]}`, or
/// `{"error": why}`.
fn measure_state(document: &[u8]) -> String {
    let measured = (|| -> Result<Value, String> {
        let doc = craft3d_format::from_bytes(document)
            .map_err(|e| format!("the document cannot be read: {e}"))?;
        STATES.with(|states| {
            let mut states = states
                .try_borrow_mut()
                .map_err(|_| "the checks are being measured already".to_string())?;
            let session = states.get_or_insert_with(|| {
                let mut s = Session::new(crate::Workspace::none());
                // Measured in line, on this thread.
                s.set_stepwise(true);
                s
            });
            crate::regen_elsewhere::load_state(session, doc);
            let (checked, outputs) = session
                .checks_and_outputs(Wait::All)
                .map_err(|e| e.message().to_string())?;
            Ok(json!({
                "checked": checked.iter().map(checked_json).collect::<Vec<_>>(),
                "outputs": outputs_json(&outputs),
            }))
        })
    })();
    match measured {
        Ok(answer) => answer.to_string(),
        Err(why) => json!({ "error": why }).to_string(),
    }
}

/// The answer of [`measure_state`] for `doc`.
fn decode_state(text: &str, doc: &Document) -> Result<(Vec<Checked>, Resolved), String> {
    let checked = decode_answer(text, doc.assertions.len())?;
    let answer: Value =
        serde_json::from_str(text).map_err(|e| format!("its answer cannot be read: {e}"))?;
    let outputs = outputs_from(answer.get("outputs").unwrap_or(&Value::Null))?;
    Ok((checked, outputs))
}

/// Every assertion and output of `doc` an error that says `why`.
fn failed_state(doc: &Document, why: &str) -> (Vec<Checked>, Resolved) {
    let checked = doc
        .assertions
        .iter()
        .map(|assertion| {
            let mut failed = pending(assertion);
            failed.state.status = AssertionStatus::Error;
            failed.state.message = why.to_string();
            failed
        })
        .collect();
    let outputs = doc
        .outputs
        .iter()
        .map(|o| (o.name.clone(), OutputValue::Unknown(why.to_string())))
        .collect();
    (checked, Resolved::from_entries(outputs))
}

/// The outputs as far as they are known: `[[name, value]]`, a value
/// `{"known": [bits, length, angle, mass, time]}` (the bits of the `f64` in base
/// units, in hex), `{"unknown": why}` or `"pending"`.
fn outputs_json(outputs: &Resolved) -> Value {
    Value::Array(
        outputs
            .entries()
            .iter()
            .map(|(name, value)| {
                let value = match value {
                    OutputValue::Known(q) => json!({ "known": [
                        format!("{:016x}", q.value.to_bits()),
                        q.dimension.length,
                        q.dimension.angle,
                        q.dimension.mass,
                        q.dimension.time,
                    ]}),
                    OutputValue::Unknown(why) => json!({ "unknown": why }),
                    OutputValue::Pending => json!("pending"),
                };
                json!([name, value])
            })
            .collect(),
    )
}

fn outputs_from(value: &Value) -> Result<Resolved, String> {
    let bad = || "the outputs cannot be read".to_string();
    let mut values = BTreeMap::new();
    for entry in value.as_array().ok_or_else(bad)? {
        let (Some(name), Some(v)) = (entry.get(0).and_then(Value::as_str), entry.get(1)) else {
            return Err(bad());
        };
        let output = if v.as_str() == Some("pending") {
            OutputValue::Pending
        } else if let Some(why) = v.get("unknown").and_then(Value::as_str) {
            OutputValue::Unknown(why.to_string())
        } else {
            let known = v.get("known").and_then(Value::as_array).ok_or_else(bad)?;
            let bits = known
                .first()
                .and_then(Value::as_str)
                .and_then(|b| u64::from_str_radix(b, 16).ok())
                .ok_or_else(bad)?;
            let exponent = |i: usize| {
                known
                    .get(i)
                    .and_then(Value::as_i64)
                    .and_then(|e| i32::try_from(e).ok())
                    .ok_or_else(bad)
            };
            OutputValue::Known(Quantity::new(
                f64::from_bits(bits),
                Dimension::of(exponent(1)?, exponent(2)?, exponent(3)?, exponent(4)?),
            ))
        };
        values.insert(name.to_string(), output);
    }
    Ok(Resolved::from_entries(values))
}

fn checked_json(c: &Checked) -> Value {
    json!({
        "name": c.state.name,
        "measure": c.state.measure,
        "status": c.state.status.name(),
        "measured": c.state.measured,
        "expected": c.state.expected,
        "message": c.state.message,
        "stored": c.stored,
        "value": c.value,
        "unit": c.unit,
        "detail": c.detail,
    })
}

fn checked_from(value: &Value) -> Option<Checked> {
    let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_string);
    let status = text("status")?;
    let status = STATUSES.into_iter().find(|s| s.name() == status)?;
    let measured = match value.get("measured")? {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        _ => return None,
    };
    let unit = match value.get("unit")? {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        _ => return None,
    };
    let detail: Map<String, Value> = value.get("detail")?.as_object()?.clone();
    Some(Checked {
        state: AssertionState {
            name: text("name")?,
            measure: text("measure")?,
            status,
            measured,
            expected: text("expected")?,
            message: text("message")?,
        },
        stored: text("stored")?,
        value: value.get("value")?.clone(),
        unit,
        detail,
    })
}

/// The answers of [`measure_checks`] for a job of `count`.
fn decode_answer(text: &str, count: usize) -> Result<Vec<Checked>, String> {
    let answer: Value =
        serde_json::from_str(text).map_err(|e| format!("its answer cannot be read: {e}"))?;
    if let Some(why) = answer.get("error").and_then(Value::as_str) {
        return Err(why.to_string());
    }
    let checked = answer
        .get("checked")
        .and_then(Value::as_array)
        .ok_or("its answer has no checks")?;
    if checked.len() != count {
        return Err(format!(
            "its answer has {} checks for {count}",
            checked.len()
        ));
    }
    checked
        .iter()
        .map(|c| checked_from(c).ok_or_else(|| "a check in its answer cannot be read".to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantities_cross_bit_for_bit() {
        let mut values = BTreeMap::new();
        for (i, v) in [
            0.1 + 0.2,
            -0.0,
            f64::MIN_POSITIVE,
            1e300,
            f64::INFINITY,
            f64::NAN,
        ]
        .into_iter()
        .enumerate()
        {
            values.insert(
                format!("o{i}"),
                OutputValue::Known(Quantity::new(v, Dimension::new(3, -1))),
            );
        }
        values.insert("u".into(), OutputValue::Unknown("why".into()));
        values.insert("p".into(), OutputValue::Pending);
        // A stress and a mass keep their mass and time, not only length
        // and angle (the web build's worker, 2026-10-30).
        values.insert(
            "stress".into(),
            OutputValue::Known(Quantity::new(1e4, Dimension::PRESSURE)),
        );
        values.insert(
            "m".into(),
            OutputValue::Known(Quantity::new(2.5, Dimension::MASS)),
        );
        let resolved = Resolved::from_entries(values);
        let back = outputs_from(&outputs_json(&resolved)).unwrap();
        assert!(matches!(
            back.entries().get("stress"),
            Some(OutputValue::Known(q)) if q.dimension == Dimension::PRESSURE
        ));
        for (name, value) in resolved.entries() {
            let other = back.entries().get(name).unwrap();
            match (value, other) {
                (OutputValue::Known(a), OutputValue::Known(b)) => {
                    assert_eq!(a.value.to_bits(), b.value.to_bits());
                    assert_eq!(a.dimension, b.dimension);
                }
                _ => assert_eq!(value, other),
            }
        }
    }

    #[test]
    fn a_request_that_cannot_be_read_is_an_error_answer() {
        for (document, work) in [
            ("", ""),
            ("{}", "{}"),
            ("not json", "[]"),
            ("PK\u{3}\u{4}", "{\"assertions\": 3}"),
        ] {
            let answer: Value =
                serde_json::from_str(&measure_checks(document.as_bytes(), work)).unwrap();
            assert!(answer["error"].is_string(), "{answer}");
        }
        assert!(decode_answer("{\"checked\": []}", 1).is_err());
        assert!(decode_answer("{\"checked\": [{}]}", 1).is_err());
        assert!(decode_answer("[", 0).is_err());
        assert!(outputs_from(&json!([["a", {"known": ["zz", 1, 0]}]])).is_err());
        assert!(outputs_from(&json!([["a", {"known": ["0", 1e12, 0]}]])).is_err());
    }
}
