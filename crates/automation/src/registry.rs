//! What both protocols offer on top of `Session::execute`: the command list
//! and batches. One implementation, so `engine.commands` and `command_list`,
//! `batch` and `command_batch` cannot drift apart.

use crate::MAX_BATCH_STEPS;
use crate::fields::Fields;
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// The registry as agents read it: every command with its params notation
/// and whether it can run right now.
///
/// `filter` is a case-insensitive substring of the id or the label; `id`
/// picks one command by its exact id. `brief` (2026-11-05: the whole list
/// was 289 KB, the feature commands alone 90 KB, too much for an agent to
/// start with) gives each command as `{id, params, summary, enabled}`
/// with only the first sentence of its summary ([`first_sentence`]); the
/// full entry, with its label, whole summary and why it is disabled, is
/// one `id` away.
pub(crate) fn command_list(session: &Session, query: &ListQuery) -> Value {
    let needle = query.filter.map(str::to_lowercase);
    let commands: Vec<Value> = Session::commands()
        .iter()
        .filter(|spec| query.id.is_none_or(|id| spec.id == id))
        .filter(|spec| {
            needle.as_deref().is_none_or(|needle| {
                spec.id.to_lowercase().contains(needle)
                    || spec.label.to_lowercase().contains(needle)
            })
        })
        .filter_map(|spec| {
            let disabled = session.disabled_reason(spec);
            (disabled.is_none() || !query.enabled_only).then(|| {
                if query.brief {
                    json!({
                        "id": spec.id,
                        "params": spec.params,
                        "summary": first_sentence(spec.summary),
                        "enabled": disabled.is_none(),
                    })
                } else {
                    json!({
                        "id": spec.id,
                        "label": spec.label,
                        "params": spec.params,
                        "summary": spec.summary,
                        "needsDocument": spec.needs_document,
                        "enabled": disabled.is_none(),
                        "disabledReason": disabled,
                    })
                }
            })
        })
        .collect();
    json!({ "commands": commands })
}

/// What [`command_list`] lists.
#[derive(Default)]
pub(crate) struct ListQuery<'a> {
    pub filter: Option<&'a str>,
    pub id: Option<&'a str>,
    pub enabled_only: bool,
    pub brief: bool,
}

/// Longest summary a brief list gives, in characters.
const BRIEF_SUMMARY_CHARS: usize = 200;

/// The first sentence of `summary`: up to the first full stop that ends
/// a sentence (followed by a space and a capital, or the end; not inside
/// brackets, not "e.g."), at most [`BRIEF_SUMMARY_CHARS`] characters (cut
/// at a word, with an ellipsis).
pub(crate) fn first_sentence(summary: &str) -> String {
    let chars: Vec<char> = summary.chars().collect();
    let mut depth = 0_i32;
    let mut end = chars.len();
    for (i, c) in chars.iter().enumerate() {
        match c {
            '(' | '[' | '{' => depth = depth.saturating_add(1),
            ')' | ']' | '}' => depth = depth.saturating_sub(1).max(0),
            '.' if depth == 0 => {
                let next = chars.get(i.saturating_add(1));
                let after = chars.get(i.saturating_add(2));
                let ends = match next {
                    None => true,
                    Some(' ') => after.is_some_and(|c| c.is_uppercase()),
                    _ => false,
                };
                let abbreviation = i >= 3
                    && chars
                        .get(i.saturating_sub(3)..i)
                        .is_some_and(|w| w.iter().collect::<String>() == "e.g");
                if ends && !abbreviation {
                    end = i.saturating_add(1);
                    break;
                }
            }
            _ => {}
        }
    }
    let sentence: String = chars.get(..end).unwrap_or_default().iter().collect();
    if sentence.chars().count() <= BRIEF_SUMMARY_CHARS {
        return sentence;
    }
    let cut: String = sentence.chars().take(BRIEF_SUMMARY_CHARS).collect();
    let at_word = cut
        .rfind(' ')
        .map_or(cut.as_str(), |i| cut.get(..i).unwrap_or(&cut));
    format!("{}…", at_word.trim_end_matches([',', ';', ':']))
}

/// One step of a batch: a command and its params (an object, or null).
pub(crate) struct Step<'a> {
    pub command: &'a str,
    pub params: &'a Value,
    /// Why the agent runs it, in its own words (the window's agent log).
    pub intent: Option<&'a str>,
}

/// Reads the `steps` of a batch. `owner` and `noun` are for messages;
/// `id_key` is what this protocol calls the command id in a step.
///
/// Every step is checked before any runs, so a malformed step 7 does not
/// leave steps 0 to 6 applied.
pub(crate) fn parse_steps<'a>(
    owner: &str,
    noun: &str,
    fields: &Fields<'a>,
    id_key: &str,
) -> Result<Vec<Step<'a>>, String> {
    let items = fields
        .array("steps")?
        .ok_or_else(|| fields.missing("steps"))?;
    if items.len() > MAX_BATCH_STEPS {
        return Err(format!(
            "{owner}: `steps` has {} steps, and a batch takes at most {MAX_BATCH_STEPS}; split it into several batches",
            items.len()
        ));
    }
    let mut steps = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let step_owner = format!("{owner}: steps[{index}]");
        if !item.is_object() {
            return Err(format!(
                "{step_owner} must be an object {{\"{id_key}\": string, \"params\": object}}, got {}",
                crate::fields::describe(item)
            ));
        }
        let step = Fields::new(&step_owner, noun, Some(item), &[id_key, "params", "intent"])?;
        steps.push(Step {
            command: step.required_str(id_key)?,
            params: step.object("params")?,
            intent: step.str("intent")?,
        });
    }
    Ok(steps)
}

/// The outcome of a batch, in the shape both protocols return.
pub(crate) struct BatchOutcome {
    pub failed: usize,
    pub report: Value,
}

/// Runs the steps in order. Not a transaction: steps that ran stay applied
/// when a later one fails. With `stop_on_error` the first failure ends the
/// batch, and `results` is shorter than `steps`.
///
/// A step fails when its command returns an error, and also (2026-11-05)
/// when its command succeeded but reports a feature that could not be
/// built ([`failing_features`]): the feature it added or edited, or one its
/// edit made fail. Such a step was applied, and stays so (the feature is
/// in the history as the command left it): its entry says `applied: true`
/// and keeps the command's `result`. Before, it counted as completed and a
/// batch with `stop_on_error` went on building on a broken feature.
///
/// `terse` is for long batches, where the results of every step would be
/// a lot to send and to read: the report then holds the failures only,
/// each with the number of its step, and the result of the last step that
/// succeeded. It also holds `featuresFailed`, when there are any: the
/// features that fail at the end of the batch and that a step of it
/// reported failing (a feature it added or edited that could not be
/// built, or one its edit broke, listed under that step's `failed`), each
/// with the last such step. A failure a later step repaired is not in
/// it; neither is a feature that failed before the batch and that no step
/// touched, nor a failed assertion (`assert.list` and `document.check` say
/// those). A step that changes the active document (`file.new`,
/// `file.open`, `file.activate`, `file.close`, `recovery.open`) starts the
/// collection again.
///
/// Every step's entry (in `results`, or in `errors` and `last` when
/// terse) has `ms`, the wall-clock milliseconds it took, and the report
/// has `ms` for the whole batch: the only fields that differ from run to
/// run (null where the host has no clock, the browser).
pub(crate) fn run_batch(
    session: &mut Session,
    steps: &[Step],
    stop_on_error: bool,
    terse: bool,
    intent: Option<&str>,
) -> BatchOutcome {
    let whole = Stopwatch::start();
    let mut results = Vec::with_capacity(if terse { 0 } else { steps.len() });
    let mut errors = Vec::new();
    // Feature id -> the last step that reported it failing.
    let mut reported: BTreeMap<String, usize> = BTreeMap::new();
    let mut last = Value::Null;
    let (mut completed, mut failed) = (0_usize, 0_usize);
    for (index, step) in steps.iter().enumerate() {
        let clock = Stopwatch::start();
        // A step's own intent, else the batch's.
        let outcome = session.with_intent(step.intent.or(intent), |session| {
            session.execute(step.command, step.params)
        });
        let ms = clock.ms();
        let stop = match outcome {
            Ok(result) => {
                if SWITCHES_DOCUMENT.contains(&step.command) {
                    reported.clear();
                }
                for id in reported_failing(&result) {
                    reported.insert(id, index);
                }
                let broken = failing_features(step.command, &result);
                if broken.is_empty() {
                    completed = completed.saturating_add(1);
                    if terse {
                        last = json!({ "step": index, "result": result, "ms": ms });
                    } else {
                        results.push(json!({ "ok": true, "result": result, "ms": ms }));
                    }
                    false
                } else {
                    // A feature that could not be built: the command did
                    // what was asked, but the model is broken from here.
                    failed = failed.saturating_add(1);
                    let error = format!(
                        "{}: {}; the step was applied and stays so (the feature is in the history as it is now: fix it with feature.edit, or take the step back with edit.undo)",
                        step.command,
                        broken.join("; ")
                    );
                    if terse {
                        errors.push(
                            json!({ "step": index, "error": error, "applied": true, "ms": ms }),
                        );
                    } else {
                        results.push(json!({
                            "ok": false,
                            "error": error,
                            "applied": true,
                            "result": result,
                            "ms": ms,
                        }));
                    }
                    stop_on_error
                }
            }
            Err(error) => {
                failed = failed.saturating_add(1);
                if terse {
                    errors.push(json!({ "step": index, "error": error.message(), "ms": ms }));
                } else {
                    results.push(json!({ "ok": false, "error": error.message(), "ms": ms }));
                }
                stop_on_error
            }
        };
        if stop {
            break;
        }
    }
    let report = if terse {
        let mut report =
            json!({ "completed": completed, "failed": failed, "errors": errors, "last": last });
        // What still fails at the end, of what the steps reported.
        let features_failed: Vec<Value> = if reported.is_empty() {
            Vec::new()
        } else {
            session
                .failed_features()
                .into_iter()
                .filter_map(|mut entry| {
                    let id = entry.get("id").and_then(Value::as_str)?.to_string();
                    let step = *reported.get(&id)?;
                    if let Some(map) = entry.as_object_mut() {
                        map.insert("step".into(), step.into());
                    }
                    Some(entry)
                })
                .collect()
        };
        if let (false, Some(map)) = (features_failed.is_empty(), report.as_object_mut()) {
            map.insert("featuresFailed".into(), features_failed.into());
        }
        report
    } else {
        json!({ "completed": completed, "failed": failed, "results": results })
    };
    let mut report = report;
    if let Some(map) = report.as_object_mut() {
        map.insert("ms".into(), whole.ms());
    }
    BatchOutcome { failed, report }
}

/// Wall-clock time for the `ms` of a reply, where the host has a clock:
/// the browser has none in `std` (`Instant::now` panics there), so it
/// says null.
pub(crate) struct Stopwatch {
    #[cfg(not(target_arch = "wasm32"))]
    started: std::time::Instant,
}

impl Stopwatch {
    pub(crate) fn start() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            // native clock: the field is cfg(not(target_arch = "wasm32")).
            started: std::time::Instant::now(),
        }
    }

    /// The milliseconds since [`Stopwatch::start`], to a tenth; null
    /// without a clock.
    pub(crate) fn ms(&self) -> Value {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let ms = self.started.elapsed().as_secs_f64() * 1000.0;
            json!((ms * 10.0).round() / 10.0)
        }
        #[cfg(target_arch = "wasm32")]
        {
            Value::Null
        }
    }
}

/// What makes a step whose command succeeded a failed step: each feature
/// its result reports failing ([`reported_failing`]), said as a sentence
/// ("the feature F3 `Fillet1` could not be built: why"). Empty when none
/// does.
/// Renaming a feature that failed before is no failure of the rename.
fn failing_features(command: &str, result: &Value) -> Vec<String> {
    if command == "feature.rename" {
        return Vec::new();
    }
    let mut said = Vec::new();
    collect_failing(result, &mut said);
    said
}

fn collect_failing(result: &Value, said: &mut Vec<String>) {
    let text = |value: &Value, key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let name = |value: &Value, key: &str| {
        let name = text(value, key);
        if name.is_empty() {
            String::new()
        } else {
            format!(" `{name}`")
        }
    };
    if result.get("status").and_then(Value::as_str) == Some("failed")
        && let Some(id) = result.get("id").and_then(Value::as_str)
        && is_feature_id(id)
    {
        let cause = result
            .get("causedBy")
            .and_then(Value::as_str)
            .map(|c| format!(" (because {c} failed)"))
            .unwrap_or_default();
        said.push(format!(
            "the feature {id}{} could not be built{cause}: {}",
            name(result, "name"),
            text(result, "error")
        ));
    }
    if let Some(sweep) = result.get("sweep") {
        collect_failing(sweep, said);
    }
    for entry in result
        .get("failed")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(id) = entry.get("id").and_then(Value::as_str)
            && is_feature_id(id)
        {
            said.push(format!(
                "it made the feature {id}{} fail: {}",
                name(entry, "feature"),
                text(entry, "error")
            ));
        }
    }
}

/// Whether `id` is a feature's (`F<n>`; an assertion's or an instance's
/// is not).
fn is_feature_id(id: &str) -> bool {
    id.strip_prefix('F')
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Commands after which the active document may be another one.
const SWITCHES_DOCUMENT: &[&str] = &[
    "file.new",
    "file.open",
    "file.activate",
    "file.close",
    "recovery.open",
];

/// The ids of the features a step's result reports failing: the feature
/// the result is about when its status is `failed` (only a feature's
/// result has an id `F<n>`; an assertion's has none), the sweep an
/// extrusion with `cut` made, and the features under `failed`.
fn reported_failing(result: &Value) -> Vec<String> {
    let mut ids = Vec::new();
    if result.get("status").and_then(Value::as_str) == Some("failed")
        && let Some(id) = result.get("id").and_then(Value::as_str)
        && is_feature_id(id)
    {
        ids.push(id.to_string());
    }
    if let Some(sweep) = result.get("sweep") {
        ids.extend(reported_failing(sweep));
    }
    for entry in result
        .get("failed")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(id) = entry.get("id").and_then(Value::as_str)
            && is_feature_id(id)
        {
            ids.push(id.to_string());
        }
    }
    ids
}

/// What a caught panic said, for the error that replaces it.
pub(crate) fn panic_detail(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|text| (*text).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "no details".to_string())
}
