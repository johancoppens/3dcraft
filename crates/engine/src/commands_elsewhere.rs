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
    /// The document after each undo step an edit made, as the bytes of
    /// its `.3dcraft` file, in order (the answer's `steps` say how each
    /// was made): what the host applies to its own history.
    pub documents: Vec<Vec<u8>>,
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
    /// It edited the document: its edit is applied here now, as one undo
    /// step (or as the steps it made), as if it had run here.
    pub edited: bool,
}

/// What a job handed over may do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JobKind {
    /// A command that only reads: its result, the selection, the files.
    Read,
    /// A command that may edit the document: the edit is made there,
    /// where the bodies are, and applied here from its answer.
    Edit,
    /// A host's own job ([`Session::hand_over`]): the host's code runs
    /// there on the state (an export with its choices resolved).
    Host,
}

/// An undo step an edit made, as `edit_merging` made it: what the host
/// needs to make the same step in its own history.
#[derive(Clone, Debug)]
pub(crate) struct EditStep {
    pub label: String,
    pub key: Option<String>,
    pub merge: bool,
    pub after: Arc<Document>,
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
    /// Another command handed over before it, on the same state and
    /// selection, still waits: the worker runs this one on the selection
    /// that one left (`run_command`), as it would have here.
    follows: bool,
    /// The window's camera, when it was given for this document (an
    /// export `view: "current"` draws what the window shows).
    camera: Option<Value>,
    kind: JobKind,
    /// The document is a Jig in use (only its parameters change).
    jig_in_use: bool,
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
    /// The command being run was dispatched by a person or an agent
    /// ([`Session::execute_deferrable`]): only such a command is handed
    /// over. A host's own reads (what the export dialog offers, how many
    /// are selected) need their answer now.
    deferrable: bool,
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
            || !self.commands.deferrable
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

    /// After a command tried without the bodies. `params` as the command
    /// read them, `asked` as they were given (an edit runs there from
    /// these: its `when` and the feature names in its face names are
    /// taken there again, from the same state).
    pub(crate) fn after_trial(
        &mut self,
        snapshot: Session,
        id: &str,
        params: &Value,
        asked: &Value,
    ) -> Trial {
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
        // Another document than the active one, or another active one:
        // not an edit of the one document a job carries.
        let elsewhere_too = self.documents.len() != snapshot.documents.len()
            || self.active != snapshot.active
            || self
                .documents
                .iter()
                .zip(&snapshot.documents)
                .enumerate()
                .any(|(at, (now, was))| {
                    now.id != was.id
                        || (at != self.active && now.history.revision() != was.history.revision())
                });
        self.restore(snapshot);
        let kind = if answered_elsewhere(id) && !edited {
            JobKind::Read
        } else if edits_elsewhere(id) && !elsewhere_too && self.edits_can_cross() {
            JobKind::Edit
        } else {
            self.commands.busy = true;
            return Trial::RunHere;
        };
        let Some(open) = self.documents.get(self.active) else {
            return Trial::RunHere;
        };
        self.commands.token = self.commands.token.wrapping_add(1).max(1);
        let token = self.commands.token;
        let current = open.history.current();
        let serial = open.selection.serial();
        // Selections chain only from reads (two clicks that add); an
        // edit is run on the selection it read.
        let follows = kind == JobKind::Read
            && self.commands.jobs.iter().any(|j| {
                j.kind == JobKind::Read
                    && j.doc_id == open.id
                    && j.serial == serial
                    && Arc::ptr_eq(&j.doc, current)
            });
        let camera = self
            .window_camera
            .as_ref()
            .filter(|(id, _)| *id == open.id)
            .map(|(_, camera)| camera_wire(camera));
        let params = match kind {
            JobKind::Edit => asked.clone(),
            _ => params.clone(),
        };
        self.commands.jobs.push(CommandJob {
            token,
            doc_id: open.id,
            doc: Arc::clone(open.history.current()),
            command: id.to_string(),
            params,
            selection: open.selection.to_wire(),
            serial,
            posted: false,
            follows,
            camera,
            kind,
            jig_in_use: open.jig_in_use,
        });
        Trial::Deferred(json!({
            "deferred": true,
            "job": token,
            "summary": format!("{id} runs where the bodies are; its answer follows"),
        }))
    }

    /// An edit of the active document can be made where the bodies are
    /// and applied here: its view is not scrubbed to an earlier state
    /// (view state the other side does not have). A motion's moment is
    /// view state too; a motion shown on the page is regenerated there
    /// (`view.motion`), until its playback gets a worker of its own.
    fn edits_can_cross(&mut self) -> bool {
        self.documents
            .get_mut(self.active)
            .is_some_and(|open| open.scrubbed_to().is_none())
    }

    /// Hands the host's own job over to be run where the bodies are, on
    /// the active document as it is now with its selection (and the
    /// window's camera): `command` names it for the other side's host
    /// code (`craft3d-web`'s `jobs.rs`), which runs it with
    /// [`crate::run_on_state`]; its answer comes back as a command's
    /// does ([`Session::command_answered`]). `None` when the active
    /// document is regenerated here (or not elsewhere at all): the host
    /// does the job itself.
    pub fn hand_over(&mut self, command: &str, params: &Value) -> Option<u64> {
        if !self.commands.enabled
            || self.commands.jobs.len() >= MAX_JOBS
            || !self.regen_elsewhere()
            || self.regenerated_here()
        {
            return None;
        }
        let open = self.documents.get(self.active)?;
        self.commands.token = self.commands.token.wrapping_add(1).max(1);
        let token = self.commands.token;
        let camera = self
            .window_camera
            .as_ref()
            .filter(|(id, _)| *id == open.id)
            .map(|(_, camera)| camera_wire(camera));
        self.commands.jobs.push(CommandJob {
            token,
            doc_id: open.id,
            doc: Arc::clone(open.history.current()),
            command: command.to_string(),
            params: params.clone(),
            selection: open.selection.to_wire(),
            serial: open.selection.serial(),
            posted: false,
            follows: false,
            camera,
            kind: JobKind::Host,
            jig_in_use: open.jig_in_use,
        });
        Some(token)
    }

    /// True while an edit handed over waits for its answer: a host keeps
    /// the commands asked for after it until it is applied, so that they
    /// run on the document it leaves, in the order they were asked for.
    pub fn edits_waiting(&self) -> bool {
        self.commands
            .jobs
            .iter()
            .any(|job| job.kind == JobKind::Edit)
    }

    /// [`Session::execute`] for a command a person or an agent asked for:
    /// while the active document is regenerated elsewhere and not here,
    /// one that needs the bodies and only reads is handed over, and its
    /// result now is `{deferred: true, job}` (`commands_elsewhere.rs`).
    /// [`Session::execute`] itself always answers at once.
    pub fn execute_deferrable(&mut self, id: &str, params: &Value) -> Result<Value, CommandError> {
        let was = std::mem::replace(&mut self.commands.deferrable, true);
        let outcome = self.execute(id, params);
        self.commands.deferrable = was;
        outcome
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
            "follows": job.follows,
            "camera": job.camera,
            "jigInUse": job.jig_in_use,
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
        let revision = self
            .documents
            .iter()
            .find(|open| open.id == job.doc_id)
            .map(|open| open.history.revision());
        let result = self.take_answer(&job, answer);
        if let Err(why) = &result {
            self.record_failure(&job.command, &job.params, why);
        }
        let edited = self
            .documents
            .iter()
            .find(|open| open.id == job.doc_id)
            .map(|open| open.history.revision())
            != revision;
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
            edited,
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
            if job.kind == JobKind::Edit
                && let Some(applied) = self.apply_edit(at, &job.doc, &reply, answer.documents)
            {
                return applied.map(|value| (value, Vec::new()));
            }
            // It changes the document and its edit cannot be taken from
            // there: run here, as it always was.
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

impl Session {
    /// Applies the edit an answer brings to the document at `at`, still
    /// in the state `doc` the edit was made on: each undo step it made,
    /// as `edit_merging` would have made it here (its label, and folded
    /// into the last step as a run of the same key is), so undo and redo
    /// see one step as after any edit. `None` when the answer's steps
    /// cannot be taken (none, or a document that does not read, or a
    /// state that does not cross the file as it is here): the caller
    /// runs the command here instead.
    fn apply_edit(
        &mut self,
        at: usize,
        doc: &Arc<Document>,
        reply: &Value,
        documents: Vec<Vec<u8>>,
    ) -> Option<Result<Value, String>> {
        let steps = reply.get("steps")?.as_array()?;
        if steps.is_empty() || steps.len() != documents.len() {
            return None;
        }
        // The other side made the edit on the state read from its file:
        // taken only when that is this state, field for field (a format
        // that left something out would otherwise lose it here).
        let crossed = craft3d_format::to_bytes(doc)
            .ok()
            .and_then(|bytes| craft3d_format::from_bytes(&bytes).ok())?;
        if crossed != **doc {
            return None;
        }
        let mut made = Vec::with_capacity(steps.len());
        for (step, bytes) in steps.iter().zip(&documents) {
            let label = step.get("label")?.as_str()?.to_string();
            let key = step.get("key").and_then(Value::as_str).map(str::to_string);
            let merge = step.get("merge").and_then(Value::as_bool).unwrap_or(false);
            let after = craft3d_format::from_bytes(bytes).ok()?;
            made.push((label, key, merge, after));
        }
        let open = self.documents.get_mut(at)?;
        for (label, key, merge, after) in made {
            let applied = open
                .history
                .apply_merging(&label, key.as_deref(), merge, |current| {
                    *current = after;
                    Ok::<(), ()>(())
                });
            if applied.is_err() {
                return None;
            }
        }
        // What it left selected, checked against this state once it is
        // regenerated here.
        if let Some(wire) = reply.get("selection").filter(|s| !s.is_null())
            && let Some(selection) = Selection::from_wire(wire, None)
        {
            open.selection = selection;
        }
        self.recovery_sync(false);
        Some(match reply.get("error") {
            Some(error) => Err(error.as_str().unwrap_or("failed").to_string()),
            None => Ok(reply.get("ok").cloned().unwrap_or(Value::Null)),
        })
    }
}

/// The commands that may edit the document and are handed over when
/// they need the bodies (`assert.add`, a feature that follows a face or
/// an edge): their edit is made there and applied here from the answer
/// (`apply_edit`), in the order asked (a host keeps later commands while
/// one waits: [`Session::edits_waiting`]). The families whose commands
/// change the document and nothing else; not the texts, whose fonts are
/// files of this session the other side does not have, and not
/// `view.motion`, whose moments are regenerated where the motion is
/// played (the hook for that: a motion's states handed over as a job of
/// their own, the motion's view state kept here).
fn edits_elsewhere(id: &str) -> bool {
    const FAMILIES: [&str; 13] = [
        "feature.",
        "sketch.",
        "datum.",
        "assert.",
        "variable.",
        "param.",
        "body.",
        "face.",
        "set.",
        "table.",
        "output.",
        "fold.",
        "unfold.",
    ];
    FAMILIES.iter().any(|family| id.starts_with(family))
        && !matches!(id, "sketch.text" | "sketch.textEdit" | "sketch.textOnCurve")
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
                | "assert.list"
                | "document.check"
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
    steps: &[EditStep],
) -> String {
    let mut answer = match result {
        Ok(value) => json!({ "ok": value }),
        Err(e) => json!({ "error": e.message() }),
    };
    if let Some(map) = answer.as_object_mut() {
        map.insert("edited".into(), Value::Bool(edited));
        map.insert("selection".into(), selection.unwrap_or(Value::Null));
        if !steps.is_empty() {
            let steps: Vec<Value> = steps
                .iter()
                .map(|step| json!({"label": step.label, "key": step.key, "merge": step.merge}))
                .collect();
            map.insert("steps".into(), Value::Array(steps));
        }
    }
    answer.to_string()
}

/// A camera written out, its numbers as their bits.
fn camera_wire(camera: &craft3d_render::Camera) -> Value {
    let bits = |v: f64| format!("{:016x}", v.to_bits());
    let vec = |v: craft3d_math::Vec3| json!([bits(v.x), bits(v.y), bits(v.z)]);
    let projection = match camera.projection {
        craft3d_render::Projection::Perspective { fov_y_deg } => {
            json!({"perspective": bits(fov_y_deg)})
        }
        craft3d_render::Projection::Orthographic { half_height } => {
            json!({"orthographic": bits(half_height)})
        }
    };
    json!({"eye": vec(camera.eye), "target": vec(camera.target), "up": vec(camera.up), "projection": projection})
}

/// A camera read back from [`camera_wire`].
pub(crate) fn camera_from(value: &Value) -> Option<craft3d_render::Camera> {
    let bits = |v: &Value| {
        u64::from_str_radix(v.as_str()?, 16)
            .ok()
            .map(f64::from_bits)
    };
    let vec = |key: &str| -> Option<craft3d_math::Vec3> {
        let [x, y, z] = value.get(key)?.as_array()?.as_slice() else {
            return None;
        };
        Some(craft3d_math::Vec3::new(bits(x)?, bits(y)?, bits(z)?))
    };
    let p = value.get("projection")?;
    let projection = if let Some(f) = p.get("perspective") {
        craft3d_render::Projection::Perspective {
            fov_y_deg: bits(f)?,
        }
    } else {
        craft3d_render::Projection::Orthographic {
            half_height: bits(p.get("orthographic")?)?,
        }
    };
    Some(craft3d_render::Camera {
        eye: vec("eye")?,
        target: vec("target")?,
        up: vec("up")?,
        projection,
    })
}
