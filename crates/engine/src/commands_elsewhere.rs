//! Commands run where the bodies are: while a host has the active
//! document regenerated elsewhere (`regen_elsewhere.rs`, the page's Web
//! Worker) and it is not regenerated here, a command that needs the
//! bodies (a click that selects, `measure.*`, `file.export`, ...) would
//! regenerate the whole state on the host's one thread. Such a command is
//! handed to the other side instead, which has the state regenerated
//! already (it built the scene the host shows), runs it there and answers
//! with its result (docs/design/engine.md › *Commands where the bodies
//! are*).
//!
//! Which commands need the bodies is not listed: a command is tried here
//! first, with every regeneration of the active document it asks for that
//! is not there answered by an empty one and noted
//! (`OpenDocument::forbid_regen`). One that asked for none is done: its
//! result stands. One that asked is undone whole (the session is put back
//! as it was before it, from a copy that shares the documents, bodies and
//! files) and handed over: [`Session::command_jobs`] and
//! [`Session::command_request`] write it out (the document state as the
//! bytes of its `.3dcraft` file, the command, its parameters and the
//! selection), [`crate::run_command`] runs it on the other side, and
//! [`Session::command_answered`] takes its answer: the result, the
//! selection it left (a click's `select.set`), the files it wrote (an
//! export). Its result now says `deferred: true` and the job's number; the
//! host reports the answer when it comes.
//!
//! The answer is the one this session would have given: the command runs
//! on the same document state with the same selection, and regeneration
//! is a function of the document alone. A command that changes the
//! document (an edit that needs the bodies, such as one that follows a
//! face) is not run there: the answer says so and it runs here, as it
//! always did, once the answer is back. An answer for a state that is no
//! longer the document's (an edit came between) is an error that says so;
//! nothing of it is taken.

use crate::selection::Selection;
use crate::{CommandError, Session};
use craft3d_doc::Document;
use serde_json::{Value, json};
use std::sync::Arc;

/// A command written out to be run where the bodies are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandRequest {
    /// What its answer goes back with ([`Session::command_answered`]).
    pub token: u64,
    /// The document state, as the bytes of its `.3dcraft` file.
    pub document: Vec<u8>,
    /// `{command, params, selection}`.
    pub work: String,
}

/// What the other side answers ([`crate::run_command`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommandAnswer {
    /// `{ok: result}` or `{error: why}`, with `edited` and `selection`.
    pub answer: String,
    /// The files the command wrote, by name.
    pub files: Vec<(String, Vec<u8>)>,
}

/// A command run elsewhere and answered, for the host to report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answered {
    pub token: u64,
    pub command: String,
    pub params: Value,
    /// What the command returned, or its error.
    pub result: Result<Value, String>,
    /// The files it wrote, now in this session's memory
    /// ([`Session::take_file`]).
    pub files: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct CommandJob {
    token: u64,
    /// The open document and its state the command was run on.
    doc_id: u64,
    doc: Arc<Document>,
    command: String,
    params: Value,
    /// The selection it read, and its serial then.
    selection: Value,
    serial: u64,
    /// Written out for the host ([`Session::command_request`]).
    posted: bool,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Commands {
    /// The host hands such commands over.
    enabled: bool,
    jobs: Vec<CommandJob>,
    token: u64,
    /// A command is being tried, or run here after all: no other is
    /// tried within it.
    busy: bool,
    /// The selection serial a command read and the serial of the
    /// selection its answer left: the next command handed over before
    /// that answer came read the same selection, and the worker ran it on
    /// what the first left (`run_command`), so its answer is taken too
    /// (two clicks that add, in quick succession).
    adopted: Option<(u64, u64)>,
}

/// Most commands waiting for their answers at once; past that a command
/// runs here.
const MAX_JOBS: usize = 64;

/// What came of trying a command without the bodies.
pub(crate) enum Trial {
    /// It needed none: its result stands.
    Kept,
    /// It needed them and is handed over: the result to give now.
    Deferred(Value),
    /// It needed them and changed the document as well: the session is
    /// as it was before it, and it runs again, here.
    RunHere,
}

impl Session {
    /// Has the commands that need the bodies run elsewhere while the
    /// active document is regenerated elsewhere and not here (a host that
    /// takes the jobs, [`Session::command_jobs`]). Off again, the jobs
    /// still waiting are errors for [`Session::commands_failed`] to say.
    pub fn set_commands_elsewhere(&mut self, elsewhere: bool) {
        self.commands.enabled = elsewhere;
    }

    /// Before a command runs: a copy of the session to go back to when the
    /// command turns out to need the bodies, with the active document
    /// told to note that. `None` when it runs here as always.
    pub(crate) fn try_without_bodies(&mut self) -> Option<Session> {
        if !self.commands.enabled
            || self.commands.busy
            || self.commands.jobs.len() >= MAX_JOBS
            || !self.regen_elsewhere()
            || self.regenerated_here()
        {
            return None;
        }
        let snapshot = self.clone();
        self.documents.get_mut(self.active)?.forbid_regen = Some(false);
        self.commands.busy = true;
        Some(snapshot)
    }

    /// After a command tried without the bodies.
    pub(crate) fn after_trial(&mut self, snapshot: Session, id: &str, params: &Value) -> Trial {
        self.commands.busy = false;
        let tried = snapshot.documents.get(snapshot.active).map(|open| open.id);
        let mut wanted = false;
        for open in &mut self.documents {
            if open.forbid_regen.take() == Some(true) && Some(open.id) == tried {
                wanted = true;
            }
        }
        if !wanted {
            return Trial::Kept;
        }
        let edited = self.documents.len() != snapshot.documents.len()
            || self.active != snapshot.active
            || self
                .documents
                .iter()
                .zip(&snapshot.documents)
                .any(|(now, was)| {
                    now.id != was.id || now.history.revision() != was.history.revision()
                });
        self.restore(snapshot);
        if edited || !answered_elsewhere(id) {
            self.commands.busy = true;
            return Trial::RunHere;
        }
        let Some(open) = self.documents.get(self.active) else {
            return Trial::RunHere;
        };
        self.commands.token = self.commands.token.wrapping_add(1).max(1);
        let token = self.commands.token;
        self.commands.jobs.push(CommandJob {
            token,
            doc_id: open.id,
            doc: Arc::clone(open.history.current()),
            command: id.to_string(),
            params: params.clone(),
            selection: open.selection.to_wire(),
            serial: open.selection.serial(),
            posted: false,
        });
        Trial::Deferred(json!({
            "deferred": true,
            "job": token,
            "summary": format!("{id} runs where the bodies are; its answer follows"),
        }))
    }

    /// A command that was run again here after its trial is done.
    pub(crate) fn ran_here(&mut self) {
        self.commands.busy = false;
    }

    /// Puts the session back as `snapshot` holds it, keeping what is not
    /// copied with a session (the recovery store).
    fn restore(&mut self, mut snapshot: Session) {
        std::mem::swap(&mut snapshot.recovery, &mut self.recovery);
        *self = snapshot;
    }

    /// The commands handed over and not written out yet, in order.
    pub fn command_jobs(&self) -> Vec<u64> {
        self.commands
            .jobs
            .iter()
            .filter(|job| !job.posted)
            .map(|job| job.token)
            .collect()
    }

    /// True while a command handed over waits for its answer.
    pub fn commands_waiting(&self) -> bool {
        !self.commands.jobs.is_empty()
    }

    /// The command numbered `token`, written out for
    /// [`crate::run_command`]; from then on it waits for its answer. An
    /// error when there is no such job (the job is then gone) or its
    /// document cannot be written out (the job is answered with that
    /// error: [`Session::command_answered`] with `Err`).
    pub fn command_request(&mut self, token: u64) -> Result<CommandRequest, String> {
        let job = self
            .commands
            .jobs
            .iter_mut()
            .find(|job| job.token == token)
            .ok_or_else(|| format!("no command {token} waits"))?;
        job.posted = true;
        let document = craft3d_format::to_bytes(&job.doc)
            .map_err(|e| format!("the document cannot be written out: {e}"))?;
        let work = json!({
            "command": job.command,
            "params": job.params,
            "selection": job.selection,
        });
        Ok(CommandRequest {
            token,
            document,
            work: work.to_string(),
        })
    }

    /// The answer to command `token`, or why it could not be run there
    /// (the worker stopped). `None` when no such command waits. The
    /// selection it left and the files it wrote are taken when the
    /// document is still in the state it was run on; a command that
    /// changed the document runs here now.
    pub fn command_answered(
        &mut self,
        token: u64,
        answer: Result<CommandAnswer, String>,
    ) -> Option<Answered> {
        let at = self.commands.jobs.iter().position(|j| j.token == token)?;
        let job = self.commands.jobs.remove(at);
        let result = self.take_answer(&job, answer);
        let (result, files) = match result {
            Ok((value, files)) => (Ok(value), files),
            Err(why) => (Err(why), Vec::new()),
        };
        Some(Answered {
            token,
            command: job.command,
            params: job.params,
            result,
            files,
        })
    }

    /// Every command that waits, answered with `why`: the worker that
    /// runs them is gone, or the host runs them itself from now on.
    pub fn commands_failed(&mut self, why: &str) -> Vec<Answered> {
        let tokens: Vec<u64> = self.commands.jobs.iter().map(|j| j.token).collect();
        tokens
            .into_iter()
            .filter_map(|token| self.command_answered(token, Err(why.to_string())))
            .collect()
    }

    fn take_answer(
        &mut self,
        job: &CommandJob,
        answer: Result<CommandAnswer, String>,
    ) -> Result<(Value, Vec<String>), String> {
        let id = &job.command;
        let answer = answer.map_err(|why| format!("{id}: {why}"))?;
        let reply: Value = serde_json::from_str(&answer.answer).map_err(|e| {
            format!("{id}: internal error (a bug in 3DCraft, please report it): its answer cannot be read: {e}")
        })?;
        let Some(at) = self.documents.iter().position(|open| {
            open.id == job.doc_id && Arc::ptr_eq(open.history.current(), &job.doc)
        }) else {
            return Err(format!(
                "{id}: the document changed before its answer came, so nothing of it was taken; run it again"
            ));
        };
        if reply.get("edited") == Some(&Value::Bool(true)) {
            // It changes the document: run here, as it always was.
            if at != self.active {
                return Err(format!("{id}: another document is shown now; run it again"));
            }
            self.commands.busy = true;
            let result = self.execute(id, &job.params);
            self.commands.busy = false;
            return result
                .map(|value| (value, Vec::new()))
                .map_err(|e| e.message().to_string());
        }
        let adopted = self.commands.adopted;
        let open = self.documents.get_mut(at).ok_or("no document")?;
        let serial = open.selection.serial();
        if (serial == job.serial || adopted == Some((job.serial, serial)))
            && let Some(wire) = reply.get("selection").filter(|s| !s.is_null())
        {
            let checked = (open.history.revision(), open.asm.generation);
            if let Some(selection) = Selection::from_wire(wire, Some(checked)) {
                let now = selection.serial();
                open.selection = selection;
                self.commands.adopted = Some((job.serial, now));
            }
        }
        if let Some(error) = reply.get("error") {
            return Err(error.as_str().unwrap_or("failed").to_string());
        }
        let mut files = Vec::new();
        for (name, bytes) in answer.files {
            self.put_file(&name, bytes)
                .map_err(|e| format!("{id}: {}", e.message()))?;
            files.push(name);
        }
        Ok((reply.get("ok").cloned().unwrap_or(Value::Null), files))
    }
}

/// The commands that are handed over when they need the bodies: those
/// that only read the document, whose effects are a result, the
/// selection and files written, which is all an answer carries. A
/// command that edits the document may need the bodies too (it follows
/// a face), and with an empty regeneration its trial can fail before it
/// edits, so the trial alone cannot tell it from one that reads: it runs
/// here, as it always did (an edit is the document's, applied in the
/// order it was asked for, never from an answer that comes later).
fn answered_elsewhere(id: &str) -> bool {
    id.starts_with("measure.")
        || id.starts_with("select.")
        || matches!(
            id,
            "file.export"
                | "document.inspect"
                | "document.edges"
                | "document.measure"
                | "document.interference"
        )
}

/// The result of a command run on the other side, as
/// [`CommandAnswer::answer`] carries it.
pub(crate) fn answer_json(
    result: Result<Value, CommandError>,
    edited: bool,
    selection: Option<Value>,
) -> String {
    let mut answer = match result {
        Ok(value) => json!({ "ok": value }),
        Err(e) => json!({ "error": e.message() }),
    };
    if let Some(map) = answer.as_object_mut() {
        map.insert("edited".into(), Value::Bool(edited));
        map.insert("selection".into(), selection.unwrap_or(Value::Null));
    }
    answer.to_string()
}
