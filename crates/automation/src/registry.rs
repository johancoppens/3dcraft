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
/// brackets, not "e.g."), at most [`BRIEF_SUMMARY_CHARS`] characters, the ellipsis
/// included (cut at a word).
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
    // Room for the ellipsis within the limit.
    let cut: String = sentence
        .chars()
        .take(BRIEF_SUMMARY_CHARS.saturating_sub(1))
        .collect();
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
    /// Steps whose command returned an error.
    pub failed: usize,
    /// Features that fail at the end and that a step of the batch made
    /// fail (`featuresFailed`).
    pub broken: usize,
    pub report: Value,
}

impl BatchOutcome {
    /// Whether the batch did not do what was asked: a command failed, or
    /// it leaves a feature it broke failing.
    pub(crate) fn is_error(&self) -> bool {
        self.failed > 0 || self.broken > 0
    }
}

/// How a batch ends ([`run_batch`]).
pub(crate) struct BatchStops {
    /// At the first command that returns an error.
    pub on_error: bool,
    /// Also at the first step that makes a feature fail.
    pub on_break: bool,
}

/// Runs the steps in order. Not a transaction: steps that ran stay applied
/// when a later one fails. With `stops.on_error` the first command that
/// returns an error ends the batch, and `results` is shorter than `steps`.
///
/// Features that fail are judged at the end (2026-11-05): a step that
/// makes a feature fail is no failed step, since a later step may repair
/// it (a variable set too far, then the variable it is measured against).
/// Each step's entry lists under `broke` the features it made fail
/// (failing after it and not before it, so a query never breaks anything
/// and touching a feature that failed already is no break), each with
/// `repairedBy`, the step after which it no longer failed, when one did.
/// The report's `featuresFailed` lists the features that fail at the end
/// and that a step made fail, or that a step edited (`feature.edit`, a
/// `sketch.*` command on it) and left failing, a repair that did not work
/// even when it failed before the batch; each with the last such step,
/// `reason`, its kind and what fixes it. When there are any the batch did
/// not do what was asked ([`BatchOutcome::is_error`]). With
/// `stops.on_break` the batch also ends at the first step that makes a
/// feature fail. A step after which another document is active (by its
/// identity, `Session::active_document_id`: `file.new`, `file.open`,
/// `instance.open`, `jig.edit`, …) starts the judging again, on that
/// document.
///
/// The features that fail are read after each step that changed the
/// document (`Session::failed_features`, which regenerates what the step
/// left to regenerate; a step that changed nothing costs nothing), and
/// once before the first step.
///
/// `terse` is for long batches: the report then holds the command errors
/// only, each with the number of its step, the result of the last step
/// that succeeded, and `featuresFailed`.
///
/// Every step's entry (in `results`, or in `errors` and `last` when
/// terse) has `ms`, the wall-clock milliseconds it took, and the report
/// has `ms` for the whole batch: the only fields that differ from run to
/// run (null where the host has no clock, the browser).
pub(crate) fn run_batch(
    session: &mut Session,
    steps: &[Step],
    stops: &BatchStops,
    terse: bool,
    intent: Option<&str>,
) -> BatchOutcome {
    let whole = Stopwatch::start();
    let mut results: Vec<Value> = Vec::with_capacity(if terse { 0 } else { steps.len() });
    let mut errors = Vec::new();
    // Feature id -> the last step that made it fail.
    let mut broken_by: BTreeMap<String, usize> = BTreeMap::new();
    // Each break: (step, feature id, its entry), for `repairedBy`.
    let mut breaks: Vec<(usize, String, Value)> = Vec::new();
    let mut last = Value::Null;
    let (mut completed, mut failed) = (0_usize, 0_usize);
    // Feature id -> the last step that edited it (feature.edit, sketch.*)
    // and left it failing: a repair that did not work.
    let mut edited: BTreeMap<String, usize> = BTreeMap::new();
    let mut before = failing_now(session);
    let mut revision = session.document_revision();
    let mut document = session.active_document_id();
    for (index, step) in steps.iter().enumerate() {
        let clock = Stopwatch::start();
        // A step's own intent, else the batch's.
        let outcome = session.with_intent(step.intent.or(intent), |session| {
            session.execute(step.command, step.params)
        });
        let ms = clock.ms();
        let now_revision = session.document_revision();
        // Another document is active (file.new, file.open, instance.open,
        // jig.edit, …: whichever command did it).
        let now_document = session.active_document_id();
        let switched = now_document != document;
        let mut broke: Vec<Value> = Vec::new();
        if switched || now_revision != revision {
            let after = failing_now(session);
            if switched {
                broken_by.clear();
                edited.clear();
            } else {
                let ids = |list: &[Value]| -> Vec<String> {
                    list.iter()
                        .filter_map(|e| e.get("id").and_then(Value::as_str))
                        .map(str::to_string)
                        .collect()
                };
                let (was, is) = (ids(&before), ids(&after));
                for id in was.iter().filter(|id| !is.contains(id)) {
                    edited.remove(id);
                    if broken_by.remove(id).is_some() {
                        for (_, broken, entry) in breaks.iter_mut().rev() {
                            if broken == id {
                                if let Some(map) = entry.as_object_mut()
                                    && !map.contains_key("repairedBy")
                                {
                                    map.insert("repairedBy".into(), index.into());
                                }
                                break;
                            }
                        }
                    }
                }
                for entry in after.iter() {
                    let Some(id) = entry.get("id").and_then(Value::as_str) else {
                        continue;
                    };
                    if was.iter().any(|w| w == id) {
                        continue;
                    }
                    broken_by.insert(id.to_string(), index);
                    let mut noted = entry.clone();
                    if let Some(map) = noted.as_object_mut() {
                        map.remove("type");
                    }
                    breaks.push((index, id.to_string(), noted.clone()));
                    broke.push(noted);
                }
                // An edit of a feature that fails after it: whether it
                // failed before or not, the step did not leave it built.
                if let Some(target) = edited_target(step) {
                    let hit = after.iter().find_map(|entry| {
                        let id = entry.get("id").and_then(Value::as_str)?;
                        let name = entry.get("feature").and_then(Value::as_str);
                        (id == target || name == Some(target)).then(|| id.to_string())
                    });
                    if let Some(id) = hit {
                        edited.insert(id, index);
                    }
                }
            }
            before = after;
            revision = now_revision;
            document = now_document;
        }
        let stop = match outcome {
            Ok(result) => {
                completed = completed.saturating_add(1);
                if terse {
                    last = json!({ "step": index, "result": result, "ms": ms });
                } else {
                    results.push(json!({ "ok": true, "result": result, "ms": ms }));
                }
                stops.on_break && !broke.is_empty()
            }
            Err(error) => {
                failed = failed.saturating_add(1);
                if terse {
                    errors.push(json!({ "step": index, "error": error.message(), "ms": ms }));
                } else {
                    results.push(json!({ "ok": false, "error": error.message(), "ms": ms }));
                }
                stops.on_error
            }
        };
        if !terse && !broke.is_empty() {
            // Filled in below, with `repairedBy` where a later step did.
            if let Some(map) = results.last_mut().and_then(Value::as_object_mut) {
                map.insert("broke".into(), Value::Array(Vec::new()));
            }
        }
        if stop {
            break;
        }
    }
    // The breaks, with what repaired them, into their steps' entries.
    if !terse {
        for (step, _, entry) in &breaks {
            let list = results
                .get_mut(*step)
                .and_then(|r| r.get_mut("broke"))
                .and_then(Value::as_array_mut);
            if let Some(list) = list {
                list.push(entry.clone());
            }
        }
    }
    // What still fails at the end, of what the steps broke or edited.
    let features_failed: Vec<Value> = before
        .into_iter()
        .filter_map(|mut entry| {
            let id = entry.get("id").and_then(Value::as_str)?.to_string();
            let (step, reason) = match (broken_by.get(&id), edited.get(&id)) {
                (Some(&b), Some(&e)) if e > b => {
                    (e, format!("still fails after step {e}, which edited it"))
                }
                (Some(&b), _) => (b, format!("broken by step {b}")),
                (None, Some(&e)) => (e, format!("still fails after step {e}, which edited it")),
                (None, None) => return None,
            };
            let fix = fix_advice(&entry);
            if let Some(map) = entry.as_object_mut() {
                map.insert("step".into(), step.into());
                map.insert("reason".into(), reason.into());
                map.insert("fix".into(), fix.into());
            }
            Some(entry)
        })
        .collect();
    let broken = features_failed.len();
    let mut report = if terse {
        json!({ "completed": completed, "failed": failed, "errors": errors, "last": last })
    } else {
        json!({ "completed": completed, "failed": failed, "results": results })
    };
    if let Some(map) = report.as_object_mut() {
        if !features_failed.is_empty() {
            map.insert("featuresFailed".into(), features_failed.into());
        }
        map.insert("ms".into(), whole.ms());
    }
    BatchOutcome {
        failed,
        broken,
        report,
    }
}

/// What fixes a feature that fails at the end of a batch (an entry of
/// `featuresFailed`): the feature it fails because of first; a sketch
/// with the sketch commands, anything else with feature.edit. Undoing is
/// said for what it is: the last step applied, once per step, never the
/// step that broke it alone.
fn fix_advice(entry: &Value) -> String {
    let own = match entry.get("type").and_then(Value::as_str) {
        Some("sketch") => {
            "fix it with sketch.edit (its dimensions and constraints) or sketch.delete"
        }
        _ => "fix it with feature.edit (feature.delete removes it)",
    };
    let first = entry
        .get("causedBy")
        .and_then(Value::as_str)
        .map(|cause| format!("it fails because {cause} does: fix {cause} first; else "))
        .unwrap_or_default();
    format!("{first}{own}; edit.undo takes back the last step applied, once per step")
}

/// The features of the active document that fail now (none without one).
fn failing_now(session: &mut Session) -> Vec<Value> {
    if session.has_document() {
        session.failed_features()
    } else {
        Vec::new()
    }
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

/// The feature a step edits explicitly, as its params name it (id or
/// name): `feature` of `feature.edit`, `sketch` of a `sketch.*` command.
/// Moving, switching, suppressing or renaming a feature is no edit of it.
fn edited_target<'a>(step: &Step<'a>) -> Option<&'a str> {
    let key = if step.command == "feature.edit" {
        "feature"
    } else if step.command.starts_with("sketch.") {
        "sketch"
    } else {
        return None;
    };
    step.params.get(key).and_then(Value::as_str)
}

/// What a caught panic said, for the error that replaces it.
pub(crate) fn panic_detail(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|text| (*text).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "no details".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Review of 2026-11-05: the advice said feature.edit for a sketch and
    /// "take the step back with edit.undo", which undoes the last step,
    /// not the one that broke it.
    #[test]
    fn the_fix_is_said_for_the_kind_of_feature() {
        let sketch = fix_advice(&json!({"id": "F3", "type": "sketch"}));
        assert!(sketch.starts_with("fix it with sketch.edit"), "{sketch}");
        assert!(!sketch.contains("feature.edit"), "{sketch}");
        let fillet = fix_advice(&json!({"id": "F2", "type": "fillet"}));
        assert!(fillet.starts_with("fix it with feature.edit"), "{fillet}");
        assert!(
            fillet.contains("the last step applied, once per step"),
            "{fillet}"
        );
        let downstream = fix_advice(&json!({"id": "F4", "type": "extrude", "causedBy": "F3"}));
        assert!(
            downstream.starts_with("it fails because F3 does: fix F3 first"),
            "{downstream}"
        );
    }

    #[test]
    fn a_brief_summary_is_its_first_sentence_within_the_limit() {
        assert_eq!(
            first_sentence("Saves it (e.g. as a.b). Then more."),
            "Saves it (e.g. as a.b)."
        );
        assert_eq!(first_sentence("Uses e.g. this. Next."), "Uses e.g. this.");
        let long = format!("{}.", "word ".repeat(100));
        let cut = first_sentence(&long);
        assert!(cut.ends_with('…'), "{cut}");
        assert!(cut.chars().count() <= BRIEF_SUMMARY_CHARS, "{cut}");
    }
}
