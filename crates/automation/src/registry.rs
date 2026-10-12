//! What both protocols offer on top of `Session::execute`: the command list
//! and batches. One implementation, so `engine.commands` and `command_list`,
//! `batch` and `command_batch` cannot drift apart.

use crate::MAX_BATCH_STEPS;
use crate::fields::Fields;
use craft3d_engine::Session;
use serde_json::{Value, json};

/// The registry as agents read it: every command with its params notation
/// and whether it can run right now.
///
/// `filter` is a case-insensitive substring of the id or the label.
pub(crate) fn command_list(session: &Session, filter: Option<&str>, enabled_only: bool) -> Value {
    let needle = filter.map(str::to_lowercase);
    let commands: Vec<Value> = Session::commands()
        .iter()
        .filter(|spec| {
            needle.as_deref().is_none_or(|needle| {
                spec.id.to_lowercase().contains(needle)
                    || spec.label.to_lowercase().contains(needle)
            })
        })
        .filter_map(|spec| {
            let disabled = session.disabled_reason(spec);
            (disabled.is_none() || !enabled_only).then(|| {
                json!({
                    "id": spec.id,
                    "label": spec.label,
                    "params": spec.params,
                    "summary": spec.summary,
                    "needsDocument": spec.needs_document,
                    "enabled": disabled.is_none(),
                    "disabledReason": disabled,
                })
            })
        })
        .collect();
    json!({ "commands": commands })
}

/// One step of a batch: a command and its params (an object, or null).
pub(crate) struct Step<'a> {
    pub command: &'a str,
    pub params: &'a Value,
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
        let step = Fields::new(&step_owner, noun, Some(item), &[id_key, "params"])?;
        steps.push(Step {
            command: step.required_str(id_key)?,
            params: step.object("params")?,
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
/// `terse` is for long batches, where the results of every step would be
/// a lot to send and to read: the report then holds the failures only,
/// each with the number of its step, and the result of the last step that
/// succeeded.
pub(crate) fn run_batch(
    session: &mut Session,
    steps: &[Step],
    stop_on_error: bool,
    terse: bool,
) -> BatchOutcome {
    let mut results = Vec::with_capacity(if terse { 0 } else { steps.len() });
    let mut errors = Vec::new();
    let mut last = Value::Null;
    let (mut completed, mut failed) = (0_usize, 0_usize);
    for (index, step) in steps.iter().enumerate() {
        match session.execute(step.command, step.params) {
            Ok(result) => {
                completed = completed.saturating_add(1);
                if terse {
                    last = json!({ "step": index, "result": result });
                } else {
                    results.push(json!({ "ok": true, "result": result }));
                }
            }
            Err(error) => {
                failed = failed.saturating_add(1);
                if terse {
                    errors.push(json!({ "step": index, "error": error.message() }));
                } else {
                    results.push(json!({ "ok": false, "error": error.message() }));
                }
                if stop_on_error {
                    break;
                }
            }
        }
    }
    let report = if terse {
        json!({ "completed": completed, "failed": failed, "errors": errors, "last": last })
    } else {
        json!({ "completed": completed, "failed": failed, "results": results })
    };
    BatchOutcome { failed, report }
}

/// What a caught panic said, for the error that replaces it.
pub(crate) fn panic_detail(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|text| (*text).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "no details".to_string())
}
