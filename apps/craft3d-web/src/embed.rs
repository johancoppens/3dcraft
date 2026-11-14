//! Embed (experimental): the page the web build sits in drives 3DCraft
//! through the same envelope as the control channel
//! (docs/control-protocol.md › *The page as a transport*). This is the
//! core of it, without a browser in it, as [`crate::host`] is: one
//! function takes a message and gives the reply, and events come out of
//! it; the page's door (`door.rs`, a `MessagePort`) only carries them.
//!
//! Four verbs for a page, no more:
//!
//! - **run a command**: a request of the protocol (`engine.execute`,
//!   `batch`, `engine.commands`, `methods`), answered under its `id` by
//!   `craft3d_automation::rpc`, the dispatch of every other transport. A
//!   command is run as the menu's are (`Session::execute_deferrable`): one
//!   that needs bodies the page does not have runs in the worker "where
//!   the bodies are" (`crate::scenes`), and its reply comes when its
//!   answer does; a read is answered on the state it was sent for, also
//!   when an edit came behind it. What would hold the page's own thread
//!   is refused instead: a `batch` step, or a command sent with
//!   `wait: true`, that needs the bodies. Besides, the window's state as
//!   data: `ui.get` and `ui.set` (the presentation is a field of it).
//! - **hear an event**: `ready` once the window is there, `regenerated`
//!   each time the document shown has been rebuilt after a change. A
//!   reply to a request that ran a command says the state it left
//!   (`state`: the document, its revision, and whether that state was
//!   said `regenerated` already), so a page knows whether to wait.
//! - **give a file**: [`GIVE`], with the file's bytes beside the request,
//!   opens a document or imports a STEP file as a file dropped on the page
//!   does.
//! - **take a file**: what a request writes (`file.export`, `file.save`)
//!   comes back with its reply as bytes, instead of a download: exactly
//!   the files its own command wrote; [`TAKE`] gives the document's own
//!   file and leaves the document as it was.
//!
//! Requests are answered in the order they came, except a command run in
//! the worker, whose reply comes with its answer (a page matches replies
//! by `id`); while an edit made in the worker waits for its answer, the
//! requests after it wait too and run on the document it leaves, in
//! order, as the menu's do. Input is hostile: no message makes it panic,
//! and what a message can make it keep is capped ([`MAX_WAITING`],
//! [`MAX_WAITING_BYTES`], [`MAX_FILE_BYTES`]).

use crate::host::{WebHost, memory_name, receive_into, received_kind};
use craft3d_automation::rpc::{self, Extension, ExtensionResult, Method};
use craft3d_automation::{MAX_REPLY_BYTES, MAX_REQUEST_BYTES};
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::collections::{BTreeSet, VecDeque};

/// The version of what a page sees (the methods, the events and their
/// fields): it goes up when a page written for the one before could
/// break. Experimental: no promise yet that it stays.
pub const PROTOCOL: u64 = 1;

/// The method that gives 3DCraft a file: `{name}` with the file's bytes
/// beside the request.
pub const GIVE: &str = "embed.give";

/// The method that takes the document's own file out, the document left
/// as it was: `{name?}`.
pub const TAKE: &str = "embed.take";

/// Most requests that wait at once (behind an edit made in the worker,
/// or for the window): one more is answered with an error.
pub const MAX_WAITING: usize = 256;

/// Most bytes the requests that wait may hold together (their lines and
/// their files).
pub const MAX_WAITING_BYTES: usize = 320 << 20;

/// The most bytes of a file a page gives (as of a file the person
/// chooses or drops). A STEP file is read up to `file.import`'s own
/// limit, which is lower.
pub const MAX_FILE_BYTES: usize = 256 << 20;

/// The longest file name taken, in bytes (what is kept of it in memory is
/// shorter still: `crate::host::memory_name`).
const MAX_NAME_BYTES: usize = 1024;

/// What a `batch` step that needs the bodies is refused with: a batch
/// runs to its end on the page's own thread.
pub const BATCH_REFUSED: &str = "a page's batch runs to its end on the page's own thread, which it would hold while the bodies are built: send this command on its own with engine.execute, which runs it beside the page";

/// What a command sent with `wait: true` that needs the bodies is refused
/// with: it would measure every assertion on the page's own thread.
pub const WAIT_REFUSED: &str = "`wait: true` would measure every assertion on the page's own thread and hold the page meanwhile: send it with engine.execute without `wait`, and read the assertions (assert.list) once `regenerated` came";

/// A message from the page: one request, as the JSON text of the
/// envelope, and the bytes that came beside it ([`GIVE`]'s file).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Message {
    pub line: String,
    pub bytes: Option<Vec<u8>>,
}

impl Message {
    /// A request without a file.
    pub fn line(line: impl Into<String>) -> Self {
        Self {
            line: line.into(),
            bytes: None,
        }
    }

    fn size(&self) -> usize {
        self.line
            .len()
            .saturating_add(self.bytes.as_ref().map_or(0, Vec::len))
    }
}

/// What goes to the page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Out {
    /// The reply to a request (one JSON object as text, `{id, ok, result |
    /// error}`), with the files the request wrote, by name.
    Reply {
        line: String,
        files: Vec<(String, Vec<u8>)>,
    },
    /// An event, one JSON object as text with its name under `event`.
    Event(String),
}

impl Out {
    /// The reply or the event as JSON (for tests and the door).
    pub fn json(&self) -> Value {
        let line = match self {
            Out::Reply { line, .. } | Out::Event(line) => line,
        };
        serde_json::from_str(line).unwrap_or(Value::Null)
    }
}

/// A request whose command runs in the worker: its reply comes with the
/// answer.
struct Deferred {
    token: u64,
    id: Value,
    started: f64,
}

/// What can be read of a request before it is dispatched: its `id` and
/// `method`, when it is a JSON object (the dispatch says what is wrong
/// with one that is not).
#[derive(Default)]
struct Peek {
    id: Value,
    method: Option<String>,
    /// `params.command` of `engine.execute`; the steps of a batch.
    command: Option<String>,
    steps: Option<usize>,
    /// `params.params.wait` of `engine.execute` is `true`.
    waits: bool,
}

fn peek(line: &str) -> Peek {
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(line) else {
        return Peek::default();
    };
    let params = map.get("params");
    Peek {
        id: map.get("id").cloned().unwrap_or(Value::Null),
        method: map
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_string),
        command: params
            .and_then(|p| p.get("command"))
            .and_then(Value::as_str)
            .map(str::to_string),
        steps: params
            .and_then(|p| p.get("steps"))
            .and_then(Value::as_array)
            .map(Vec::len),
        waits: params
            .and_then(|p| p.get("params"))
            .and_then(|p| p.get("wait"))
            == Some(&Value::Bool(true)),
    }
}

/// The methods a page has besides the protocol's own: [`GIVE`], [`TAKE`],
/// and the window's state (`ui.get`, `ui.set`), as `methods` lists them.
fn methods() -> &'static [Method] {
    static ALL: std::sync::OnceLock<Vec<Method>> = std::sync::OnceLock::new();
    ALL.get_or_init(|| {
        let mut all = vec![
            Method {
                name: GIVE,
                params: r#"{"name": string} (the file's bytes beside the request, as `bytes`)"#,
                summary: "Gives 3DCraft a file, as a file dropped on the page: a .3dcraft, .3dcraftjig or .3dcraftasm document opens in a tab of its own and is shown, a STEP (.step, .stp), FreeCAD (.FCStd) or BREP file is imported into the active document (a new one when none is open). name says which by its extension; at most 256 MiB (a STEP file: what file.import reads). Returns what file.open or file.import returned. Experimental.",
            },
            Method {
                name: TAKE,
                params: r#"{"name": string = the document's own name}"#,
                summary: "Takes the active document's own file out, as bytes with the reply (files), and leaves the document as it was: its name, its file and its unsaved mark do not change (file.save {path} names the document after the file and marks it saved). Experimental.",
            },
        ];
        all.extend(
            craft3d_ui_egui::UI_METHODS
                .iter()
                .filter(|m| matches!(m.name, "ui.get" | "ui.set"))
                .map(|m| Method {
                    name: m.name,
                    params: m.params,
                    summary: m.summary,
                }),
        );
        all
    })
}

/// The core: what a page asked for and what goes back to it.
pub struct Embed {
    /// The page's clock, in milliseconds (`performance.now()` in the
    /// browser): what a reply's `ms` is measured with.
    clock: Box<dyn Fn() -> f64>,
    /// The page asked for the embedded stand (`data-craft3d-embed`).
    embedded: bool,
    /// What the page's options said that was not understood, for `ready`.
    warnings: Vec<String>,
    /// Requests that wait, in order: behind an edit made in the worker,
    /// or (`ui.set`) for the window to show the document.
    waiting: VecDeque<Message>,
    waiting_bytes: usize,
    /// Requests whose command runs in the worker.
    deferred: Vec<Deferred>,
    out: Vec<Out>,
    /// The document and its revision last said `regenerated`.
    built: Option<(u64, u64)>,
    /// `ready` was said.
    greeted: bool,
}

impl Embed {
    /// A core that measures with `clock` (milliseconds, any clock that
    /// does not go back).
    pub fn new(embedded: bool, clock: Box<dyn Fn() -> f64>) -> Self {
        Self {
            clock,
            embedded,
            warnings: Vec::new(),
            waiting: VecDeque::new(),
            waiting_bytes: 0,
            deferred: Vec::new(),
            out: Vec::new(),
            built: None,
            greeted: false,
        }
    }

    /// What the page's options said that 3DCraft did not take, said in
    /// `ready` (`crate::flags::embed_warnings`).
    pub fn set_warnings(&mut self, warnings: Vec<String>) {
        self.warnings = warnings;
    }

    /// What goes to the page now, in order.
    pub fn take_out(&mut self) -> Vec<Out> {
        std::mem::take(&mut self.out)
    }

    /// How many requests wait (behind an edit, or for the window).
    pub fn waiting(&self) -> usize {
        self.waiting.len()
    }

    /// Says `ready` once: the window is there and a page listens. Called
    /// before the page's messages are taken, so that it comes first.
    pub fn greet(&mut self, host: &WebHost) {
        if self.greeted {
            return;
        }
        self.greeted = true;
        self.out.push(Out::Event(ready_event(
            host.shell.state.presentation.name(),
            self.embedded,
            &self.warnings,
        )));
    }

    /// A message from the page: answered now, or kept until it can be
    /// (behind an edit made in the worker; a `ui.set` until the window
    /// shows the document).
    pub fn receive(&mut self, host: &mut WebHost, message: Message) {
        if message.line.len() > MAX_REQUEST_BYTES {
            self.reply_line(rpc::too_long_reply());
            return;
        }
        let peek = peek(&message.line);
        if let Some(bytes) = &message.bytes
            && bytes.len() > MAX_FILE_BYTES
        {
            self.refuse(
                &peek.id,
                &format!(
                    "{GIVE}: the file is {} bytes; at most {MAX_FILE_BYTES} are taken",
                    bytes.len()
                ),
            );
            return;
        }
        if !self.waiting.is_empty() || must_wait(host, &peek) {
            let size = message.size();
            if self.waiting.len() >= MAX_WAITING
                || self.waiting_bytes.saturating_add(size) > MAX_WAITING_BYTES
            {
                self.refuse(
                    &peek.id,
                    &format!(
                        "too many requests wait for an edit made beside the page (at most {MAX_WAITING}, {MAX_WAITING_BYTES} bytes); send it again once the replies have come"
                    ),
                );
                return;
            }
            self.waiting_bytes = self.waiting_bytes.saturating_add(size);
            self.waiting.push_back(message);
            return;
        }
        self.handle(host, message);
    }

    /// Once per frame, after the worker's answers were taken
    /// (`WebHost::answered`): the replies of commands run in the worker,
    /// the requests that waited and can run now, and `regenerated` when
    /// the document shown has been rebuilt.
    pub fn frame(&mut self, host: &mut WebHost) {
        self.answers(host);
        while let Some(front) = self.waiting.front() {
            if must_wait(host, &peek(&front.line)) {
                break;
            }
            let Some(message) = self.waiting.pop_front() else {
                break;
            };
            self.waiting_bytes = self.waiting_bytes.saturating_sub(message.size());
            self.handle(host, message);
            // A command it ran may have been answered at once (no
            // worker), and the next may wait for its edit.
            self.answers(host);
        }
        self.regenerated(host);
    }

    /// The replies of the commands run in the worker that were answered,
    /// each with the files its own command wrote.
    fn answers(&mut self, host: &mut WebHost) {
        for (answer, files) in host.take_page_answers() {
            let Some(at) = self.deferred.iter().position(|d| d.token == answer.token) else {
                continue;
            };
            let job = self.deferred.remove(at);
            let ms = self.ms_since(job.started);
            host.page_did(&answer.command, &answer.result);
            let state = self.state_of(host);
            let reply = match &answer.result {
                Ok(result) => {
                    json!({ "id": job.id, "ok": true, "result": result, "ms": ms, "state": state })
                }
                Err(error) => {
                    json!({ "id": job.id, "ok": false, "error": error, "ms": ms, "state": state })
                }
            };
            self.reply(&job.id, &reply, files);
        }
    }

    /// `regenerated`, once per state of the document shown: when its
    /// scene is there (the bodies rebuilt and meshed, by the worker or on
    /// the page), so a measure of it is read from what it shows.
    fn regenerated(&mut self, host: &mut WebHost) {
        let (Some(status), Some(revision)) = (
            host.session.document_status(),
            host.session.document_revision(),
        ) else {
            self.built = None;
            return;
        };
        let key = (status.id, revision);
        if self.built == Some(key) || host.session.edits_waiting() {
            return;
        }
        let quality = host.shell.scene_quality();
        if !host.session.scene_ready(quality) {
            return;
        }
        self.built = Some(key);
        self.out.push(Out::Event(
            json!({
                "event": "regenerated",
                "document": status.index,
                "revision": revision,
            })
            .to_string(),
        ));
    }

    /// The state a reply leaves: the document shown, its revision, and
    /// whether it was said `regenerated` already (if not, a `regenerated`
    /// for this revision, or a later one of the same document, follows).
    /// Null while no document is open.
    fn state_of(&self, host: &WebHost) -> Value {
        match (
            host.session.document_status(),
            host.session.document_revision(),
        ) {
            (Some(status), Some(revision)) => json!({
                "document": status.index,
                "revision": revision,
                "regenerated": self.built == Some((status.id, revision)),
            }),
            _ => Value::Null,
        }
    }

    /// Dispatches one request, through the protocol's own dispatch.
    fn handle(&mut self, host: &mut WebHost, message: Message) {
        let Message { line, bytes } = message;
        let peek = peek(&line);
        let method = peek.method.as_deref();
        if bytes.is_some() && method != Some(GIVE) {
            self.refuse(
                &peek.id,
                &format!("`bytes` came with a request that does not take a file; only {GIVE} does"),
            );
            return;
        }
        let started = (self.clock)();
        let away = host.away();
        // What is in memory before: a command's files are those it adds.
        let before: BTreeSet<String> = host.session.file_names().into_iter().collect();
        // What would hold the page's own thread is refused, and a read
        // keeps the state it was sent for.
        let refuse = match method {
            Some("batch") => Some(BATCH_REFUSED),
            Some("engine.execute") if peek.waits => Some(WAIT_REFUSED),
            _ => None,
        };
        host.session.set_refuse_bodies(refuse);
        host.session.set_reads_on_their_state(true);
        let mut file = bytes;
        let mut given: Option<&'static str> = None;
        let mut taken: Option<(String, Vec<u8>)> = None;
        let reply = {
            let WebHost { session, shell, .. } = &mut *host;
            let mut handle =
                |session: &mut Session, method: &str, params: Option<&Value>| -> ExtensionResult {
                    match method {
                        GIVE => Some(give(session, params, file.take(), &mut given)),
                        TAKE => Some(take(session, params).map(|(result, file)| {
                            taken = Some(file);
                            result
                        })),
                        // The window as it shows the document while its scene
                        // is built: never built here for an answer.
                        "ui.get" if away => Some(shell.ui_get_away(params)),
                        "ui.get" | "ui.set" => shell.handle_method(session, method, params),
                        _ => None,
                    }
                };
            let mut extension = Extension {
                methods: methods(),
                handle: &mut handle,
            };
            rpc::handle_line_deferrable(session, &line, &mut extension)
        };
        host.session.set_refuse_bodies(None);
        host.session.set_reads_on_their_state(false);
        let Some(reply) = reply else {
            return;
        };
        let written: Vec<String> = host
            .session
            .file_names()
            .into_iter()
            .filter(|name| !before.contains(name))
            .collect();
        match method {
            Some("engine.execute") => self.executed(host, &peek, &reply, started, &written),
            Some("batch") => {
                let label = format!("batch of {}", peek.steps.unwrap_or(0));
                host.page_did(&label, &outcome_of(&reply));
                let files = host.take_files(&written);
                self.reply_with_state(host, &peek.id, &reply, None, files);
            }
            Some(GIVE) => match given {
                Some(label) => {
                    host.page_did(label, &outcome_of(&reply));
                    let files = host.take_files(&written);
                    self.reply_with_state(host, &peek.id, &reply, None, files);
                }
                None => self.reply_line(reply),
            },
            Some(TAKE) => {
                let files = taken.into_iter().collect();
                self.reply_line_with(reply, files);
            }
            _ => self.reply_line(reply),
        }
    }

    /// The reply of `engine.execute`: kept until its answer comes when its
    /// command runs in the worker; else given now, timed by the page's
    /// clock (the protocol's own clock is the native one), with the files
    /// the command wrote and the state it left.
    fn executed(
        &mut self,
        host: &mut WebHost,
        peek: &Peek,
        reply: &str,
        started: f64,
        written: &[String],
    ) {
        let Ok(value) = serde_json::from_str::<Value>(reply) else {
            self.reply_line(reply.to_string());
            return;
        };
        if let Some(token) = value.get("result").and_then(deferred) {
            host.page_job(token);
            self.deferred.push(Deferred {
                token,
                id: peek.id.clone(),
                started,
            });
            return;
        }
        let ms = self.ms_since(started);
        // Said and shown as a command of the control channel is: only
        // when the request named one (a refused envelope ran nothing).
        if let Some(command) = &peek.command {
            host.page_did(command, &outcome_of(reply));
        }
        let files = host.take_files(written);
        self.reply_with_state(host, &peek.id, reply, Some(ms), files);
    }

    /// A reply line with the state it left (and its time), as a page
    /// reads it after a request that ran a command.
    fn reply_with_state(
        &mut self,
        host: &WebHost,
        id: &Value,
        reply: &str,
        ms: Option<Value>,
        files: Vec<(String, Vec<u8>)>,
    ) {
        let Ok(mut value) = serde_json::from_str::<Value>(reply) else {
            self.reply_line_with(reply.to_string(), files);
            return;
        };
        let state = self.state_of(host);
        if let Some(map) = value.as_object_mut() {
            if let Some(ms) = ms {
                map.insert("ms".to_string(), ms);
            }
            map.insert("state".to_string(), state);
        }
        self.reply(id, &value, files);
    }

    fn ms_since(&self, started: f64) -> Value {
        let ms = (self.clock)() - started;
        if ms.is_finite() && ms >= 0.0 {
            json!((ms * 10.0).round() / 10.0)
        } else {
            Value::Null
        }
    }

    /// A reply as the protocol writes it, capped as the protocol caps
    /// its replies.
    fn reply(&mut self, id: &Value, value: &Value, files: Vec<(String, Vec<u8>)>) {
        let line = serde_json::to_string(value).unwrap_or_else(|_| {
            rpc::error_reply(
                id,
                "the reply could not be written as JSON (a bug in 3DCraft)",
            )
        });
        if line.len() > MAX_REPLY_BYTES {
            let error = format!(
                "the reply is {} bytes, over the limit of {MAX_REPLY_BYTES} bytes; ask for less at a time",
                line.len()
            );
            self.reply_line_with(rpc::error_reply(id, &error), files);
            return;
        }
        self.reply_line_with(line, files);
    }

    fn reply_line(&mut self, line: String) {
        self.reply_line_with(line, Vec::new());
    }

    fn reply_line_with(&mut self, line: String, files: Vec<(String, Vec<u8>)>) {
        self.out.push(Out::Reply { line, files });
    }

    fn refuse(&mut self, id: &Value, error: &str) {
        self.reply_line(rpc::error_reply(id, error));
    }
}

/// The `ready` event: the protocol's version, 3DCraft's, the
/// presentation, the stand, and what of the page's options was not taken.
pub fn ready_event(presentation: &str, embedded: bool, warnings: &[String]) -> String {
    json!({
        "event": "ready",
        "protocol": PROTOCOL,
        "version": env!("CARGO_PKG_VERSION"),
        "presentation": presentation,
        "embedded": embedded,
        "warnings": warnings,
    })
    .to_string()
}

/// The `failed` event: 3DCraft could not start in the page (no canvas, no
/// WebGPU or WebGL2), and why. No `ready` comes, and every request is
/// answered with that error.
pub fn failed_event(why: &str) -> String {
    json!({ "event": "failed", "error": why }).to_string()
}

/// A request waits: behind an edit made in the worker (it runs on the
/// document that edit leaves, as the menu's do), or a `ui.set` while the
/// window does not show the document yet (setting it there would build
/// the scene on the page, all at once).
fn must_wait(host: &WebHost, peek: &Peek) -> bool {
    host.session.edits_waiting() || (peek.method.as_deref() == Some("ui.set") && host.away())
}

/// The job a command was handed over under (`{deferred: true, job}`).
fn deferred(result: &Value) -> Option<u64> {
    (result.get("deferred") == Some(&Value::Bool(true)))
        .then(|| result.get("job").and_then(Value::as_u64))
        .flatten()
}

/// A reply line as the outcome the status bar says.
fn outcome_of(reply: &str) -> Result<Value, String> {
    let value: Value = serde_json::from_str(reply).map_err(|e| e.to_string())?;
    match value.get("ok") {
        Some(Value::Bool(true)) => Ok(value.get("result").cloned().unwrap_or(Value::Null)),
        _ => Err(value
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("failed")
            .to_string()),
    }
}

/// The `name` of a page's method, when its params are `{name}` alone (or
/// nothing, when `required` is false).
fn name_param<'a>(
    method: &str,
    params: Option<&'a Value>,
    required: bool,
) -> Result<Option<&'a str>, String> {
    let map = match params {
        Some(Value::Object(map)) => map,
        None | Some(Value::Null) if !required => return Ok(None),
        _ => {
            return Err(format!(
                "{method}: its params are {{\"name\": string}}{}",
                if method == GIVE {
                    ", with the file's bytes beside the request"
                } else {
                    ""
                }
            ));
        }
    };
    if let Some(other) = map.keys().find(|k| k.as_str() != "name") {
        let other: String = other.chars().take(40).collect();
        return Err(format!(
            "{method}: unknown parameter `{other}`; it takes name"
        ));
    }
    let name = match map.get("name") {
        Some(Value::String(name)) => name.as_str(),
        Some(_) => return Err(format!("{method}: `name` must be a string")),
        None if required => {
            return Err(format!(
                "{method}: `name` is missing: the file's name, which says by its extension what it is"
            ));
        }
        None => return Ok(None),
    };
    if name.len() > MAX_NAME_BYTES {
        return Err(format!(
            "{method}: the name is longer than {MAX_NAME_BYTES} bytes"
        ));
    }
    Ok(Some(name))
}

/// [`GIVE`]: the file opened or imported as a dropped one is
/// (`crate::host::receive_into`); `given` says which, once it ran. The
/// file is not kept in memory after (it is not a file the request wrote).
fn give(
    session: &mut Session,
    params: Option<&Value>,
    bytes: Option<Vec<u8>>,
    given: &mut Option<&'static str>,
) -> Result<Value, String> {
    let Some(name) = name_param(GIVE, params, true)? else {
        return Err(format!("{GIVE}: `name` is missing"));
    };
    let Some(bytes) = bytes else {
        return Err(format!(
            "{GIVE}: no file came with the request; send its bytes beside it, as `bytes`"
        ));
    };
    let Some(kind) = received_kind(name) else {
        return Err(format!(
            "{GIVE}: `{name}` is not a 3DCraft document (.3dcraft), a Jig (.3dcraftjig), an assembly (.3dcraftasm), a STEP file (.step, .stp) or a FreeCAD document (.FCStd)"
        ));
    };
    *given = Some(match kind {
        crate::host::Received::Document => "file.open",
        crate::host::Received::Step => "file.import",
    });
    let outcome = receive_into(session, name, bytes, None);
    if let Some(name) = memory_name(name) {
        let _ = session.take_file(&name);
    }
    outcome
}

/// [`TAKE`]: the active document's own file (its `.3dcraft` bytes, or an
/// assembly's), under the name asked for or its own; the document, its
/// name and its unsaved mark stay as they were.
fn take(
    session: &mut Session,
    params: Option<&Value>,
) -> Result<(Value, (String, Vec<u8>)), String> {
    let asked = name_param(TAKE, params, false)?;
    let Some(status) = session.document_status() else {
        return Err(format!(
            "{TAKE}: no document is open; give one ({GIVE}) or make one (file.new)"
        ));
    };
    let Some(bytes) = session.document_bytes() else {
        return Err(format!("{TAKE}: the document cannot be written as a file"));
    };
    let name = match asked {
        Some(name) => {
            memory_name(name).ok_or_else(|| format!("{TAKE}: `{name}` is not a file name"))?
        }
        None => craft3d_ui_egui::files::suggested_save_name(&status),
    };
    let result = json!({ "name": name, "bytes": bytes.len(), "document": status.index });
    Ok((result, (name, bytes)))
}
