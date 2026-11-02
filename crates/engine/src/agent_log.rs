//! What agents did to a document (2026-11-02): one entry per command an
//! agent ran, with who, when, what it made or changed, how it went and the
//! intent the agent gave, newest last. Which agent made and last changed
//! each feature is written into the document itself (`Feature::made_by`,
//! saved since format 63).
//!
//! Session state, per open document, like the selection: not saved in the
//! `.3dcraft` file and not undone. A host says who is acting
//! ([`Session::set_actor`]) and why ([`Session::set_intent`]) around the
//! commands it runs for an agent; commands it runs for the person are not
//! logged. The coming Agent tab reads it through `ui.get` › `agentLog`, an
//! agent through `agent.log`.

use crate::{CommandError, CommandSpec, Session};
use serde_json::{Value, json};
use std::collections::{BTreeMap, VecDeque};

/// Most entries kept per document; the oldest go first.
pub const MAX_ENTRIES: usize = 500;
/// Most characters of an intent; a longer one is cut with `…`.
pub const MAX_INTENT_CHARS: usize = 300;
/// Most characters of a result's message.
const MAX_MESSAGE_CHARS: usize = 160;

/// Who acts, and why.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Actor {
    pub client: String,
    pub intent: Option<String>,
}

/// One command an agent ran.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub client: String,
    /// Milliseconds since the Unix epoch (0 where there is no clock).
    pub time_ms: u64,
    pub command: String,
    pub label: String,
    pub ok: bool,
    /// One line: what it answered, or why it failed.
    pub message: String,
    /// The features it made, and those it changed.
    pub created: Vec<String>,
    pub changed: Vec<String>,
    pub intent: Option<String>,
}

impl Entry {
    fn json(&self) -> Value {
        json!({
            "client": self.client,
            "timeMs": self.time_ms,
            "command": self.command,
            "label": self.label,
            "ok": self.ok,
            "message": self.message,
            "created": self.created,
            "changed": self.changed,
            "intent": self.intent,
        })
    }
}

/// A document's log.
#[derive(Clone, Debug, Default)]
pub(crate) struct AgentLog {
    entries: VecDeque<Entry>,
}

impl AgentLog {
    fn push(&mut self, entry: Entry) {
        if self.entries.len() >= MAX_ENTRIES {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }
}

/// `text` as an intent: control characters (line breaks too) made spaces,
/// runs of spaces made one, trimmed, at most [`MAX_INTENT_CHARS`]
/// characters; `None` when nothing is left.
pub fn clean_intent(text: &str) -> Option<String> {
    let mut out = String::new();
    let mut space = false;
    for c in text.chars() {
        // No direction override or invisible character: it could make the
        // intent read otherwise where it is shown.
        if invisible(c) {
            continue;
        }
        let c = if c.is_control() || c.is_whitespace() {
            ' '
        } else {
            c
        };
        if c == ' ' {
            if !space && !out.is_empty() {
                out.push(' ');
            }
            space = true;
        } else {
            out.push(c);
            space = false;
        }
    }
    let out = out.trim_end().to_string();
    if out.is_empty() {
        return None;
    }
    if out.chars().count() > MAX_INTENT_CHARS {
        let cut: String = out.chars().take(MAX_INTENT_CHARS - 1).collect();
        return Some(format!("{}…", cut.trim_end()));
    }
    Some(out)
}

/// A direction mark or override, or an invisible format character: left
/// out of what is shown, so it cannot make a text read otherwise.
fn invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
            | '\u{E0001}'
            | '\u{E0020}'..='\u{E007F}'
    )
}

/// One line of at most [`MAX_MESSAGE_CHARS`] characters.
fn one_line(text: &str) -> String {
    let flat: String = text
        .chars()
        .filter(|c| !invisible(*c))
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let flat = flat.trim();
    if flat.chars().count() > MAX_MESSAGE_CHARS {
        let cut: String = flat.chars().take(MAX_MESSAGE_CHARS - 1).collect();
        format!("{cut}…")
    } else {
        flat.to_string()
    }
}

/// What a result says in one line: its `name`, `message` or `summary`
/// when it has one, else nothing.
fn result_line(result: &Value) -> String {
    ["summary", "message", "name", "path"]
        .iter()
        .find_map(|key| result.get(*key).and_then(Value::as_str))
        .map(one_line)
        .unwrap_or_default()
}

fn now_ms() -> u64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
    }
    #[cfg(target_arch = "wasm32")]
    {
        0
    }
}

/// Commands that only list what is there: not worth an entry, and the
/// window itself runs some of them after every request (`param.list` for
/// its Parameters tab), which are no agent's doing. Checks an agent runs
/// (`document.check`, `assert.check`) are logged.
const NOT_LOGGED: [&str; 10] = [
    "agent.log",
    "document.inspect",
    "select.get",
    "file.list",
    "session.report",
    "param.list",
    "assert.list",
    "variable.list",
    "set.list",
    "recovery.list",
];

/// The active document's id and the document as it is: a shared handle,
/// nothing copied.
fn snapshot(session: &Session) -> Option<(u64, std::sync::Arc<craft3d_doc::Document>)> {
    let open = session.document().ok()?;
    Some((open.id, std::sync::Arc::clone(open.history.current())))
}

impl Session {
    /// Who runs the commands from now on: an agent's name (the one its MCP
    /// client gave), or `None` for the person. Only an agent's commands
    /// are logged.
    pub fn set_actor(&mut self, client: Option<&str>) {
        self.actor = client.map(|client| Actor {
            client: one_line(client),
            intent: None,
        });
    }

    /// Why the agent runs the next commands, in its own words (cleaned,
    /// [`clean_intent`]); `None` clears it.
    pub fn set_intent(&mut self, intent: Option<&str>) {
        if let Some(actor) = &mut self.actor {
            actor.intent = intent.and_then(clean_intent);
        }
    }

    /// Runs `run` (one command) and logs it for the acting agent.
    pub(crate) fn logged(
        &mut self,
        id: &str,
        run: impl FnOnce(&mut Self) -> Result<Value, CommandError>,
    ) -> Result<Value, CommandError> {
        let Some(actor) = self.actor.clone() else {
            return run(self);
        };
        if NOT_LOGGED.contains(&id) {
            return run(self);
        }
        let before = snapshot(self);
        let outcome = run(self);
        let label = Self::command(id).map_or_else(
            || id.to_string(),
            |spec: &CommandSpec| spec.label.to_string(),
        );
        let (ok, message) = match &outcome {
            Ok(result) => (true, result_line(result)),
            Err(e) => (false, one_line(e.message())),
        };
        let after = snapshot(self);
        let (mut created, mut changed) = (Vec::new(), Vec::new());
        if let (Some((was, old)), Some((is, new))) = (&before, &after)
            && was == is
            && !std::sync::Arc::ptr_eq(old, new)
        {
            let old: BTreeMap<_, _> = old.features.iter().map(|f| (f.id, f)).collect();
            for feature in &new.features {
                match old.get(&feature.id) {
                    None => created.push(feature.id.to_string()),
                    Some(previous) if *previous != feature => changed.push(feature.id.to_string()),
                    Some(_) => {}
                }
            }
        }
        let entry = Entry {
            client: actor.client,
            time_ms: now_ms(),
            command: id.to_string(),
            label,
            ok,
            message,
            created,
            changed,
            intent: actor.intent,
        };
        if let Ok(open) = self.document_mut() {
            // Who made or changed a feature goes into the document itself
            // (saved since format 63), as no undo step of its own: the
            // document makes the same bodies either way.
            if !entry.created.is_empty() || !entry.changed.is_empty() {
                let (created, changed, client) = (&entry.created, &entry.changed, &entry.client);
                open.history.annotate(|doc| {
                    for feature in &mut doc.features {
                        let id = feature.id.to_string();
                        if created.contains(&id) {
                            feature.made_by = Some(client.clone());
                        } else if changed.contains(&id) {
                            feature.last_changed_by = Some(client.clone());
                        }
                    }
                });
            }
            open.agent_log.push(entry);
        }
        outcome
    }

    /// The active document's log, newest first, at most `limit` entries,
    /// and the features agents made or changed.
    pub fn agent_log_json(&self, limit: usize) -> Value {
        let Ok(open) = self.document() else {
            return json!({ "document": null, "entries": [], "features": [] });
        };
        let log = &open.agent_log;
        let entries: Vec<Value> = log
            .entries
            .iter()
            .rev()
            .take(limit)
            .map(Entry::json)
            .collect();
        let features: Vec<Value> = open
            .history
            .current()
            .features
            .iter()
            .filter(|feature| feature.made_by.is_some() || feature.last_changed_by.is_some())
            .map(|feature| {
                json!({"id": feature.id.to_string(), "name": feature.name,
                       "madeBy": feature.made_by, "changedBy": feature.last_changed_by})
            })
            .collect();
        json!({
            "document": self.active,
            "total": log.entries.len(),
            "entries": entries,
            "features": features,
        })
    }
}

fn agent_log(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p = crate::params::Params::new("agent.log", params, &["limit"])?;
    let limit = p.integer("limit", 1, MAX_ENTRIES as u64)?.unwrap_or(50);
    session.document()?;
    Ok(session.agent_log_json(usize::try_from(limit).unwrap_or(MAX_ENTRIES)))
}

pub(crate) const SPECS: &[CommandSpec] = &[CommandSpec {
    id: "agent.log",
    label: "Agent log",
    params: r#"{"limit": int = 50}"#,
    summary: "What agents did to the active document, newest first (at most limit, 1 to 500 of the last 500): entries {client, timeMs, command, label, ok, message, created, changed, intent} and features [{id, name, madeBy, changedBy}]: who made and who last changed each feature. Session state, not saved; the person's own commands are not in it.",
    needs_document: true,
    names: &[],
    run: agent_log,
}];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Workspace;

    #[test]
    fn an_intent_is_cleaned_and_capped() {
        assert_eq!(
            clean_intent("  round the base\n edges\tfor handling  ").as_deref(),
            Some("round the base edges for handling")
        );
        assert_eq!(
            clean_intent("a\u{0}b\u{1b}[31mc").as_deref(),
            Some("a b [31mc")
        );
        assert_eq!(
            clean_intent("safe\u{202E}txt.exe\u{200B}").as_deref(),
            Some("safetxt.exe"),
            "no direction override, no invisible character"
        );
        assert_eq!(clean_intent(" \n\t "), None);
        assert_eq!(clean_intent(""), None);
        let long = "x".repeat(1000);
        let cut = clean_intent(&long).unwrap();
        assert_eq!(cut.chars().count(), MAX_INTENT_CHARS);
        assert!(cut.ends_with('…'));
        assert_eq!(
            clean_intent("ø".repeat(400).as_str())
                .unwrap()
                .chars()
                .count(),
            MAX_INTENT_CHARS
        );
    }

    #[test]
    fn an_agents_commands_are_logged_with_what_they_made_and_why() {
        let mut session = Session::new(Workspace::none());
        // The person's own commands: not logged.
        session
            .execute("file.new", &json!({"units": "mm"}))
            .unwrap();
        session
            .execute(
                "feature.block",
                &json!({"name": "Base", "size": [40, 30, 8]}),
            )
            .unwrap();
        assert_eq!(session.agent_log_json(10)["total"], 0);
        session.set_actor(Some("Claude Code"));
        session.set_intent(Some("a post for the handle"));
        session
            .execute(
                "feature.cylinder",
                &json!({"name": "Post", "position": [10, 10, 8], "radius": 3, "height": 20}),
            )
            .unwrap();
        session.set_intent(None);
        session
            .execute(
                "feature.edit",
                &json!({"feature": "Base", "size": [50, 30, 8]}),
            )
            .unwrap();
        assert!(
            session
                .execute("feature.fillet", &json!({"edges": "nope"}))
                .is_err()
        );
        let log = session.agent_log_json(10);
        assert_eq!(log["total"], 3);
        let entries = log["entries"].as_array().unwrap();
        // Newest first.
        assert_eq!(entries[0]["command"], "feature.fillet");
        assert_eq!(entries[0]["ok"], false);
        assert!(!entries[0]["message"].as_str().unwrap().is_empty());
        assert_eq!(entries[1]["command"], "feature.edit");
        assert_eq!(entries[1]["changed"], json!(["F1"]));
        assert_eq!(entries[1]["intent"], Value::Null);
        assert_eq!(entries[2]["command"], "feature.cylinder");
        assert_eq!(entries[2]["created"], json!(["F2"]));
        assert_eq!(entries[2]["intent"], "a post for the handle");
        assert_eq!(entries[2]["client"], "Claude Code");
        assert!(entries[2]["label"].as_str().is_some_and(|l| !l.is_empty()));
        // Provenance: the agent made the post and changed the base.
        assert_eq!(
            log["features"],
            json!([
                {"id": "F1", "name": "Base", "madeBy": null, "changedBy": "Claude Code"},
                {"id": "F2", "name": "Post", "madeBy": "Claude Code", "changedBy": null}
            ])
        );
        // The command, for an agent.
        let read = session.execute("agent.log", &json!({"limit": 1})).unwrap();
        assert_eq!(read["entries"].as_array().unwrap().len(), 1);
        assert!(session.execute("agent.log", &json!({"limit": 0})).is_err());
        assert!(
            session
                .execute("agent.log", &json!({"limit": 501}))
                .is_err()
        );
        assert!(
            session
                .execute("agent.log", &json!({"limit": "x"}))
                .is_err()
        );
        assert_eq!(
            session.agent_log_json(10)["total"],
            3,
            "agent.log is not logged"
        );
        // Listings (the window runs param.list after each request) are
        // not an agent's doing.
        session.execute("param.list", &json!({})).unwrap();
        assert_eq!(session.agent_log_json(10)["total"], 3);
        // The person again: not logged.
        session.set_actor(None);
        session
            .execute("feature.block", &json!({"size": [1, 1, 1]}))
            .unwrap();
        assert_eq!(session.agent_log_json(10)["total"], 3);
    }

    #[test]
    fn the_log_keeps_the_last_entries_only() {
        let mut session = Session::new(Workspace::none());
        session.execute("file.new", &json!({})).unwrap();
        session.set_actor(Some("bot"));
        for n in 0..(MAX_ENTRIES + 20) {
            session
                .execute(
                    "variable.set",
                    &json!({"name": "v", "expr": format!("{n} mm")}),
                )
                .unwrap();
        }
        let log = session.agent_log_json(MAX_ENTRIES);
        assert_eq!(log["total"], MAX_ENTRIES);
        assert_eq!(log["entries"].as_array().unwrap().len(), MAX_ENTRIES);
        // Per document: a new one starts empty.
        session.execute("file.new", &json!({})).unwrap();
        assert_eq!(
            session.agent_log_json(10)["total"],
            1,
            "file.new itself, in the new document"
        );
    }

    /// Who opened the active document (security review, 2026-11-02): only
    /// a file the person opened or saved, and made active, is theirs.
    #[test]
    fn the_person_document_file_is_the_persons_only() {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/tmp/craft3d-person-file");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = |name: &str| dir.join(name).to_str().unwrap().to_string();
        let mut session = Session::new(Workspace::unrestricted());
        session.execute("file.new", &json!({})).unwrap();
        assert_eq!(session.person_document_file(), None, "a new document");
        session
            .execute("file.save", &json!({"path": path("mine.3dcraft")}))
            .unwrap();
        let mine = session.person_document_file().unwrap();
        assert!(mine.ends_with("mine.3dcraft"), "{mine}");
        // An agent saves it elsewhere: not the person's file any more.
        session.set_actor(Some("bot"));
        session
            .execute("file.save", &json!({"path": path("moved.3dcraft")}))
            .unwrap();
        session.set_actor(None);
        assert_eq!(session.person_document_file(), None);
        // The person saves it again: theirs.
        session
            .execute("file.save", &json!({"path": path("mine.3dcraft")}))
            .unwrap();
        assert!(session.person_document_file().is_some());
        // An agent opens another file: neither it nor (while active) any.
        session.set_actor(Some("bot"));
        session
            .execute("file.open", &json!({"path": path("moved.3dcraft")}))
            .unwrap();
        assert_eq!(session.person_document_file(), None);
        // An agent makes the person's document active: still none.
        session
            .execute("file.activate", &json!({"document": 0}))
            .unwrap();
        session.set_actor(None);
        assert_eq!(
            session.person_document_file(),
            None,
            "made active by an agent"
        );
        // The person activates it: theirs again.
        session
            .execute("file.activate", &json!({"document": 0}))
            .unwrap();
        assert!(session.person_document_file().is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
