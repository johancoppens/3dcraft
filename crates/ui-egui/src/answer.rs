//! What the status bar says about a command's answer: a few words a
//! person reads at a glance ("Activated strandbeest-leg", "Saved
//! bracket"), never the answer's raw JSON, which `ui.get`, the control
//! channel and MCP give in full.
//!
//! The words come from the answer's own fields, the same way for every
//! command: its `message` or `summary` when it has one; else what it is
//! about (its `name`, the file of its `path`, or the active document of a
//! listing of `documents`), after the verb of the command's id
//! (`file.activate` → "Activated"); else the verb alone, or "Done".

use serde_json::Value;

/// Most characters of a summary.
pub const SUMMARY_CHARS: usize = 160;

/// The status bar's words for `result`, the answer of the command
/// `label` (its id, such as `file.activate`).
pub fn summary(label: &str, result: &Value) -> String {
    let text = match result {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        Value::Array(items) => match items.len() {
            1 => "1 item".to_string(),
            n => format!("{n} items"),
        },
        Value::Object(map) => {
            let said = ["message", "summary"]
                .iter()
                .find_map(|key| map.get(*key).and_then(Value::as_str))
                .filter(|s| !s.trim().is_empty());
            let base = match said {
                Some(said) => said.to_string(),
                None => match (verb(label), subject(result)) {
                    (Some(verb), Some(subject)) => format!("{verb} {subject}"),
                    (None, Some(subject)) => subject,
                    (Some(verb), None) => verb.to_string(),
                    (None, None) => "Done".to_string(),
                },
            };
            format!("{base}{}", trouble(result))
        }
    };
    shorten(&text)
}

/// [`summary`] of an answer given as text (an MCP tool's content): its
/// summary when it is JSON, the text itself otherwise.
pub fn summary_of_text(label: &str, text: &str) -> String {
    match serde_json::from_str::<Value>(text) {
        Ok(value) if value.is_object() || value.is_array() => summary(label, &value),
        _ => shorten(text),
    }
}

/// Whether `text` looks like a JSON object or array: what the status bar
/// never shows.
pub fn looks_like_json(text: &str) -> bool {
    let text = text.trim_start();
    text.starts_with('{') || text.starts_with('[')
}

/// The past tense of the last word of a command id, for the verbs
/// commands are named with.
fn verb(label: &str) -> Option<&'static str> {
    let last = label.rsplit('.').next().unwrap_or(label);
    Some(match last {
        "activate" => "Activated",
        "open" => "Opened",
        "save" | "saveAs" => "Saved",
        "export" => "Exported",
        "import" => "Imported",
        "new" | "create" => "Created",
        "close" => "Closed",
        "delete" | "remove" | "discard" => "Removed",
        "rename" => "Renamed",
        "add" => "Added",
        "set" => "Set",
        "undo" => "Undone",
        "redo" => "Redone",
        _ => return None,
    })
}

/// What went wrong in an answer that is not an error (a feature stored
/// that fails, a check that found problems): its `status` when that is
/// not `ok`, its `error`, `ok: false`, and how many `failed`, `errors`,
/// `problems` and `warnings` it has (a list or a count); nothing when all
/// is well.
fn trouble(result: &Value) -> String {
    let mut out = String::new();
    let status = result
        .get("status")
        .and_then(Value::as_str)
        .filter(|s| *s != "ok");
    if let Some(status) = status {
        out.push_str(" · ");
        out.push_str(status);
    }
    if let Some(error) = result.get("error").and_then(Value::as_str) {
        out.push_str(": ");
        out.push_str(error);
    }
    let mut counted = false;
    for (key, said) in [
        ("failed", "failed"),
        ("errors", "errors"),
        ("problems", "problems"),
        ("warnings", "warnings"),
    ] {
        let n = match result.get(key) {
            Some(Value::Array(items)) => items.len() as u64,
            Some(Value::Number(n)) => n.as_u64().unwrap_or(0),
            _ => 0,
        };
        if n > 0 {
            counted |= key != "warnings";
            let said = match (n, said) {
                (1, "errors") => "error",
                (1, "problems") => "problem",
                (1, "warnings") => "warning",
                _ => said,
            };
            out.push_str(&format!(" · {n} {said}"));
        }
    }
    // Not ok, and nothing above said why.
    if result.get("ok") == Some(&Value::Bool(false)) && status.is_none() && !counted {
        out.push_str(" · not ok");
    }
    // What an export or a list left out: hidden bodies and bodies named
    // that are switched off (`hiddenLeftOut`, `switchedOffLeftOut`).
    for (key, said) in [
        ("hiddenLeftOut", "hidden"),
        ("switchedOffLeftOut", "switched off"),
    ] {
        let n = result
            .get(key)
            .and_then(|left| left.get("count"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if n > 0 {
            let bodies = if n == 1 { "body" } else { "bodies" };
            out.push_str(&format!(" · {n} {said} {bodies} left out"));
        }
    }
    out
}

/// What an answer is about: its `name`, the file of its `path`, or the
/// active document of a listing of `documents`.
fn subject(result: &Value) -> Option<String> {
    if let Some(name) = result
        .get("name")
        .and_then(Value::as_str)
        .filter(|n| !n.is_empty())
    {
        return Some(name.to_string());
    }
    if let Some(path) = result.get("path").and_then(Value::as_str) {
        return file_stem(path);
    }
    let documents = result.get("documents")?.as_array()?;
    let active = result.get("active").and_then(Value::as_u64);
    let row = documents
        .iter()
        .find(|d| d.get("active").and_then(Value::as_bool) == Some(true))
        .or_else(|| {
            let index = usize::try_from(active?).ok()?;
            documents.get(index)
        })?;
    match row.get("path").and_then(Value::as_str) {
        Some(path) => file_stem(path),
        // An example from the start screen, by its title.
        None => Some(
            row.get("title")
                .and_then(Value::as_str)
                .unwrap_or("Untitled")
                .to_string(),
        ),
    }
}

/// The name of the file at `path`, without its extension.
fn file_stem(path: &str) -> Option<String> {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let stem = match name.split_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => name,
    };
    (!stem.is_empty()).then(|| stem.to_string())
}

fn shorten(text: &str) -> String {
    let mut out: String = text.chars().take(SUMMARY_CHARS).collect();
    if text.chars().count() > SUMMARY_CHARS {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// What an export or a list left out is said (review of J-045, S3).
    #[test]
    fn what_was_left_out_is_said() {
        let result = json!({"path": "a/part.stl", "bodies": 1,
            "hiddenLeftOut": {"count": 2, "bodies": ["Gauge", "Jig"]},
            "switchedOffLeftOut": {"count": 1, "bodies": ["Lid"]}});
        assert_eq!(
            summary("file.export", &result),
            "Exported part · 2 hidden bodies left out · 1 switched off body left out"
        );
    }

    #[test]
    fn a_listing_of_documents_says_the_active_one() {
        let result = json!({"active": 1, "documents": [
            {"active": false, "document": 0, "path": "/a/strandbeest-walker.3dcraftasm"},
            {"active": true, "document": 1, "path": "/a/strandbeest-leg.3dcraftasm"},
        ]});
        assert_eq!(
            summary("file.activate", &result),
            "Activated strandbeest-leg"
        );
        let untitled = json!({"active": 0, "documents": [{"active": true, "path": null}]});
        assert_eq!(summary("file.activate", &untitled), "Activated Untitled");
        // An example from the start screen, by its title.
        let example = json!({"active": 0, "documents": [
            {"active": true, "path": null, "title": "Moka pot"},
        ]});
        assert_eq!(summary("file.activate", &example), "Activated Moka pot");
    }

    #[test]
    fn an_answer_is_said_in_words_never_as_json() {
        assert_eq!(
            summary(
                "file.save",
                &json!({"path": "C:\\d\\bracket.3dcraft", "bytes": 12})
            ),
            "Saved bracket"
        );
        assert_eq!(
            summary("feature.block", &json!({"id": "F3", "name": "Plate"})),
            "Plate"
        );
        assert_eq!(
            summary("view.trace", &json!({"points": [[0, 0, 0]]})),
            "Done"
        );
        assert_eq!(summary("file.close", &json!({"closed": 1})), "Closed");
        assert_eq!(
            summary("x.y", &json!({"message": "3 holes found"})),
            "3 holes found"
        );
        assert_eq!(
            summary(
                "feature.sphere",
                &json!({"name": "F1", "status": "failed", "error": "the radius is not positive"})
            ),
            "F1 · failed: the radius is not positive"
        );
        assert_eq!(
            summary(
                "variable.set",
                &json!({"name": "R", "failed": [{"feature": "F2"}]})
            ),
            "Set R · 1 failed"
        );
        assert_eq!(summary("x.y", &json!([1, 2])), "2 items");
    }

    /// Answers that report trouble without being errors say so, never
    /// "Done" alone.
    #[test]
    fn trouble_in_an_answer_is_said() {
        // document.check
        assert_eq!(
            summary(
                "document.check",
                &json!({"ok": false, "problems": [{"what": "a"}], "warnings": []})
            ),
            "Done · 1 problem"
        );
        assert_eq!(
            summary(
                "document.check",
                &json!({"ok": true, "problems": [], "warnings": []})
            ),
            "Done"
        );
        // The assertions' summary: counts.
        assert_eq!(
            summary(
                "assert.check",
                &json!({"ok": false, "passed": 3, "failed": 2, "errors": 1})
            ),
            "Done · 2 failed · 1 error"
        );
        assert_eq!(
            summary(
                "assert.check",
                &json!({"ok": true, "passed": 3, "failed": 0, "errors": 0})
            ),
            "Done"
        );
        // file.open and recovery: warnings.
        assert_eq!(
            summary(
                "file.open",
                &json!({"path": "/w/leg.3dcraft", "warnings": ["a", "b"]})
            ),
            "Opened leg · 2 warnings"
        );
        // Not ok with nothing counted.
        assert_eq!(
            summary("motion.check", &json!({"ok": false})),
            "Done · not ok"
        );
        // A status that is not ok already says it.
        assert_eq!(
            summary(
                "x.y",
                &json!({"name": "F1", "status": "failed", "ok": false})
            ),
            "F1 · failed"
        );
        assert_eq!(summary("x.y", &json!(4.5)), "4.5");
        assert_eq!(summary("x.y", &Value::Null), "");
        for result in [
            json!({"a": {"b": [1, 2, {"c": 3}]}}),
            json!({"active": 5, "documents": []}),
            json!({"name": "", "path": ""}),
        ] {
            let said = summary("file.activate", &result);
            assert!(!looks_like_json(&said), "{said}");
        }
        assert_eq!(
            summary_of_text(
                "file.activate",
                r#"{"active":0,"documents":[{"active":true,"path":"/x/leg.3dcraft"}]}"#
            ),
            "Activated leg"
        );
        assert_eq!(summary_of_text("x", "plain words"), "plain words");
        let long = "w".repeat(400);
        assert_eq!(
            summary("x", &json!(long)).chars().count(),
            SUMMARY_CHARS + 1
        );
    }
}
