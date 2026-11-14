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
//!   answer does. Besides, the window's state as data: `ui.get` and
//!   `ui.set` (the presentation is a field of it).
//! - **hear an event**: `ready` once the window is there, `regenerated`
//!   each time the document shown has been rebuilt after a change.
//! - **give a file**: [`GIVE`], with the file's bytes beside the request,
//!   opens a document or imports a STEP file as a file dropped on the page
//!   does.
//! - **take a file**: what a request writes (`file.export`, `file.save`)
//!   comes back with its reply as bytes, instead of a download.
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
use std::collections::VecDeque;

/// The version of what a page sees (the methods, the events and their
/// fields): it goes up when a page written for the one before could
/// break. Experimental: no promise yet that it stays.
pub const PROTOCOL: u64 = 1;

/// The method that gives 3DCraft a file: `{name}` with the file's bytes
/// beside the request.
pub const GIVE: &str = "embed.give";

/// Most requests that wait at once (behind an edit made in the worker,
/// or for the window): one more is answered with an error.
pub const MAX_WAITING: usize = 256;

/// Most bytes the requests that wait may hold together (their lines and
/// their files).
pub const MAX_WAITING_BYTES: usize = 320 << 20;

/// The most bytes of a file a page gives (as of a file the person
/// chooses or drops).
pub const MAX_FILE_BYTES: usize = 256 << 20;

/// The longest file name taken, in bytes (what is kept of it in memory is
/// shorter still: `crate::host::memory_name`).
const MAX_NAME_BYTES: usize = 1024;

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
    }
}

/// The methods a page has besides the protocol's own: [`GIVE`], and the
/// window's state (`ui.get`, `ui.set`), as `methods` lists them.
fn methods() -> &'static [Method] {
    static ALL: std::sync::OnceLock<Vec<Method>> = std::sync::OnceLock::new();
    ALL.get_or_init(|| {
        let mut all = vec![Method {
            name: GIVE,
            params: r#"{"name": string} (the file's bytes beside the request, as `bytes`)"#,
            summary: "Gives 3DCraft a file, as a file dropped on the page: a .3dcraft, .3dcraftjig or .3dcraftasm document opens in a tab of its own and is shown, a STEP (.step, .stp), FreeCAD (.FCStd) or BREP file is imported into the active document (a new one when none is open). name says which by its extension; at most 256 MiB. Returns what file.open or file.import returned. Experimental.",
        }];
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
            waiting: VecDeque::new(),
            waiting_bytes: 0,
            deferred: Vec::new(),
            out: Vec::new(),
            built: None,
            greeted: false,
        }
    }

    /// What goes to the page now, in order.
    pub fn take_out(&mut self) -> Vec<Out> {
        std::mem::take(&mut self.out)
    }

    /// How many requests wait (behind an edit, or for the window).
    pub fn waiting(&self) -> usize {
        self.waiting.len()
    }

    /// Says `ready` once: the window is there and a page listens.
    pub fn greet(&mut self, host: &WebHost) {
        if self.greeted {
            return;
        }
        self.greeted = true;
        self.out.push(Out::Event(ready_event(
            host.shell.state.presentation.name(),
            self.embedded,
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

    /// The replies of the commands run in the worker that were answered.
    fn answers(&mut self, host: &mut WebHost) {
        for answer in host.take_page_answers() {
            let Some(at) = self.deferred.iter().position(|d| d.token == answer.token) else {
                continue;
            };
            let job = self.deferred.remove(at);
            let ms = self.ms_since(job.started);
            let files = host.page_did(&answer.command, &answer.result);
            let reply = match &answer.result {
                Ok(result) => json!({ "id": job.id, "ok": true, "result": result, "ms": ms }),
                Err(error) => json!({ "id": job.id, "ok": false, "error": error, "ms": ms }),
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

    /// Dispatches one request, through the protocol's own dispatch.
    fn handle(&mut self, host: &mut WebHost, message: Message) {
        let Message { line, bytes } = message;
        let peek = peek(&line);
        let giving = peek.method.as_deref() == Some(GIVE);
        if bytes.is_some() && !giving {
            self.refuse(
                &peek.id,
                &format!("`bytes` came with a request that does not take a file; only {GIVE} does"),
            );
            return;
        }
        let started = (self.clock)();
        let away = host.away();
        let mut file = bytes;
        let mut given: Option<&'static str> = None;
        let reply = {
            let WebHost { session, shell, .. } = host;
            let mut handle =
                |session: &mut Session, method: &str, params: Option<&Value>| -> ExtensionResult {
                    match method {
                        GIVE => Some(give(session, params, file.take(), &mut given)),
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
        let Some(reply) = reply else {
            return;
        };
        match peek.method.as_deref() {
            Some("engine.execute") => self.executed(host, &peek, &reply, started),
            Some("batch") => {
                let label = format!("batch of {}", peek.steps.unwrap_or(0));
                let outcome = outcome_of(&reply);
                let files = host.page_did(&label, &outcome);
                self.reply_line_with(reply, files);
            }
            Some(GIVE) => match given {
                Some(label) => {
                    let outcome = outcome_of(&reply);
                    let files = host.page_did(label, &outcome);
                    self.reply_line_with(reply, files);
                }
                None => self.reply_line(reply),
            },
            _ => self.reply_line(reply),
        }
    }

    /// The reply of `engine.execute`: kept until its answer comes when its
    /// command runs in the worker; else given now, timed by the page's
    /// clock (the protocol's own clock is the native one), with what the
    /// command wrote.
    fn executed(&mut self, host: &mut WebHost, peek: &Peek, reply: &str, started: f64) {
        let Ok(mut value) = serde_json::from_str::<Value>(reply) else {
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
        if let Some(map) = value.as_object_mut() {
            map.insert("ms".to_string(), ms);
        }
        // Said and shown as a command of the control channel is: only
        // when the request named one (a refused envelope ran nothing).
        let files = match &peek.command {
            Some(command) => host.page_did(command, &outcome_of(reply)),
            None => Vec::new(),
        };
        self.reply(&peek.id, &value, files);
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

/// The `ready` event: the protocol's version, 3DCraft's, the presentation
/// and the stand.
pub fn ready_event(presentation: &str, embedded: bool) -> String {
    json!({
        "event": "ready",
        "protocol": PROTOCOL,
        "version": env!("CARGO_PKG_VERSION"),
        "presentation": presentation,
        "embedded": embedded,
    })
    .to_string()
}

/// The `failed` event: 3DCraft could not start in the page (no WebGPU or
/// WebGL2), and why. No `ready` comes.
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

/// [`GIVE`]: the file opened or imported as a dropped one is
/// (`crate::host::receive_into`); `given` says which, once it ran. The
/// file is not kept in memory after (it is not a file the request wrote).
fn give(
    session: &mut Session,
    params: Option<&Value>,
    bytes: Option<Vec<u8>>,
    given: &mut Option<&'static str>,
) -> Result<Value, String> {
    let name = match params {
        Some(Value::Object(map)) => {
            if let Some(other) = map.keys().find(|k| k.as_str() != "name") {
                let other: String = other.chars().take(40).collect();
                return Err(format!(
                    "{GIVE}: unknown parameter `{other}`; it takes name"
                ));
            }
            match map.get("name") {
                Some(Value::String(name)) => name.as_str(),
                Some(_) => return Err(format!("{GIVE}: `name` must be a string")),
                None => {
                    return Err(format!(
                        "{GIVE}: `name` is missing: the file's name, which says by its extension what it is"
                    ));
                }
            }
        }
        _ => {
            return Err(format!(
                "{GIVE}: its params are {{\"name\": string}}, with the file's bytes beside the request"
            ));
        }
    };
    if name.len() > MAX_NAME_BYTES {
        return Err(format!(
            "{GIVE}: the name is longer than {MAX_NAME_BYTES} bytes"
        ));
    }
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
