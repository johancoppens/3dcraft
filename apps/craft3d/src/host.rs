//! The window's session and shell, and how a request from the control
//! channel reaches them: as a line through a channel, answered on the
//! thread that owns the document.
//!
//! The protocol is the one of `craft3d-cli serve` (`craft3d-automation`),
//! extended with the shell's `ui.*` methods. There is one `Session`: what a
//! request changes is what the window shows.

use craft3d_automation::rpc::{self, Extension, Method};
use craft3d_engine::Session;
use craft3d_ui_egui::{CommandReport, Shell, UI_METHODS};
use serde_json::Value;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};

/// One request line, and where its reply goes. The reply is `None` for a
/// line that gets none (a blank one).
pub struct Request {
    pub line: String,
    pub reply: Sender<Option<String>>,
}

/// Longest result or error shown in the status bar, in characters.
const REPORT_CHARS: usize = 160;
/// Replies larger than this are not parsed for the status bar.
const REPORT_PARSE_BYTES: usize = 64 * 1024;
/// Most requests answered between two frames, so a flood of requests
/// cannot keep the window from repainting.
const MAX_PER_PUMP: usize = 64;

/// Everything the application owns: the documents and the view on them.
pub struct Host {
    pub session: Session,
    pub shell: Shell,
    /// The save dialog that is open was asked for by Save in the question
    /// before closing a document without a file: the document is closed
    /// once it is saved there.
    pub(crate) close_after_save: bool,
}

fn methods() -> &'static [Method] {
    static ALL: std::sync::OnceLock<Vec<Method>> = std::sync::OnceLock::new();
    ALL.get_or_init(|| {
        let mut all = vec![Method {
            name: "auth",
            params: r#"{"token": string}"#,
            summary: "Authenticates the connection; must be its first request.",
        }];
        all.push(Method {
            name: "mcp.message",
            params: r#"{"line": string}"#,
            summary: "Answers one Model Context Protocol message (a JSON-RPC line) on this window's document and returns {\"reply\": line or null}. What `craft3d-cli mcp --bridge` sends.",
        });
        all.extend(UI_METHODS.iter().map(|m| Method {
            name: m.name,
            params: m.params,
            summary: m.summary,
        }));
        all
    })
}

/// The window's methods an MCP client reaches through the tool
/// `control_call`: the `ui.*` ones, without `ui.quit` (closing the window
/// the developer is working in is not for a model to decide).
fn mcp_methods() -> &'static [Method] {
    static ALL: std::sync::OnceLock<Vec<Method>> = std::sync::OnceLock::new();
    ALL.get_or_init(|| {
        UI_METHODS
            .iter()
            .filter(|m| m.name != "ui.quit")
            .map(|m| Method {
                name: m.name,
                params: m.params,
                summary: m.summary,
            })
            .collect()
    })
}

/// One MCP message, answered on the window's own session and, for the
/// tool `control_call`, by the window's shell.
fn mcp_message(
    session: &mut Session,
    shell: &mut Shell,
    params: Option<&Value>,
) -> Result<Value, String> {
    let line = match params {
        Some(Value::Object(map)) if map.len() == 1 => map.get("line").and_then(Value::as_str),
        _ => None,
    };
    let Some(line) = line else {
        return Err(
            "mcp.message: its one parameter is `line`, a string holding one MCP message".into(),
        );
    };
    let mut handle = |session: &mut Session, method: &str, params: Option<&Value>| {
        shell.handle_method(session, method, params)
    };
    let mut extension = Extension {
        methods: mcp_methods(),
        handle: &mut handle,
    };
    let reply = craft3d_automation::mcp::handle_line_with(session, line, &mut extension);
    Ok(serde_json::json!({ "reply": reply }))
}

/// What an MCP message did, for the status bar: the command a tool call
/// ran, or the tool's name.
fn mcp_label(request: &Value) -> Option<String> {
    let message: Value =
        serde_json::from_str(request.get("params")?.get("line")?.as_str()?).ok()?;
    if message.get("method")?.as_str()? != "tools/call" {
        return None;
    }
    let call = message.get("params")?;
    let tool = call.get("name")?.as_str()?;
    let command = call
        .get("arguments")
        .and_then(|a| a.get("id"))
        .and_then(Value::as_str);
    let method = call
        .get("arguments")
        .and_then(|a| a.get("method"))
        .and_then(Value::as_str);
    Some(match (tool, command, method) {
        ("command_run", Some(command), _) => command.to_string(),
        ("control_call", _, Some(method)) => method.to_string(),
        _ => tool.to_string(),
    })
}

/// Whether the tool call in an `mcp.message` reply worked, and its text.
fn mcp_outcome(reply: &Value) -> Option<(bool, String)> {
    let inner: Value = serde_json::from_str(reply.get("result")?.get("reply")?.as_str()?).ok()?;
    if let Some(error) = inner.get("error") {
        return Some((false, error.get("message")?.as_str()?.to_string()));
    }
    let result = inner.get("result")?;
    let failed = result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let text = result
        .get("content")
        .and_then(Value::as_array)
        .and_then(|blocks| blocks.iter().find_map(|b| b.get("text")?.as_str()))
        .unwrap_or_default();
    Some((!failed, text.to_string()))
}

fn shorten(text: &str) -> String {
    let mut out: String = text.chars().take(REPORT_CHARS).collect();
    if text.chars().count() > REPORT_CHARS {
        out.push('…');
    }
    out
}

/// What the status bar says about a request, when it ran a command.
pub(crate) fn report(line: &str, reply: &str) -> Option<CommandReport> {
    let request: Value = serde_json::from_str(line).ok()?;
    let label = match request.get("method")?.as_str()? {
        "engine.execute" => request.get("params")?.get("command")?.as_str()?.to_string(),
        "batch" => {
            let steps = request
                .get("params")
                .and_then(|p| p.get("steps"))
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            format!("batch of {steps}")
        }
        "mcp.message" => mcp_label(&request)?,
        _ => return None,
    };
    let over_mcp = request.get("method").and_then(Value::as_str) == Some("mcp.message");
    let label = shorten(&label);
    // A test's way to a panic outside the protocol's own guard.
    #[cfg(test)]
    if label == "debug.panic-in-report" {
        crate::tests::fall_over();
    }
    if reply.len() > REPORT_PARSE_BYTES {
        return Some(CommandReport {
            label,
            ok: true,
            text: format!("({} bytes)", reply.len()),
        });
    }
    let reply: Value = serde_json::from_str(reply).ok()?;
    if over_mcp {
        let (ok, text) = mcp_outcome(&reply)?;
        return Some(CommandReport {
            label,
            ok,
            text: shorten(&text),
        });
    }
    let ok = reply.get("ok")?.as_bool()?;
    let text = if ok {
        reply
            .get("result")
            .map(Value::to_string)
            .unwrap_or_default()
    } else {
        reply.get("error")?.as_str()?.to_string()
    };
    Some(CommandReport {
        label,
        ok,
        text: shorten(&text),
    })
}

/// Where a request is answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    /// On the thread that draws the window, at once: what needs the shell
    /// (the `ui.*` methods, MCP's `control_call`) and what costs nothing
    /// (`auth`, `methods`, `engine.commands`, a line that is not a
    /// request at all).
    Window,
    /// On the engine thread, with the session: what runs commands
    /// (`engine.execute`, `batch`, MCP tool calls but `control_call`).
    Engine,
    /// At once even while the engine thread works, from what the window
    /// shows: `ui.get` and `ui.quit`.
    Anytime,
}

/// Where `line` is answered ([`Route`]).
pub fn route(line: &str) -> Route {
    let Ok(request) = serde_json::from_str::<Value>(line) else {
        return Route::Window;
    };
    match request.get("method").and_then(Value::as_str) {
        Some("engine.execute" | "batch") => Route::Engine,
        #[cfg(test)]
        Some("debug.slow") => Route::Engine,
        Some("ui.get" | "ui.quit") => Route::Anytime,
        Some("mcp.message") => {
            let message = request
                .get("params")
                .and_then(|p| p.get("line"))
                .and_then(Value::as_str)
                .and_then(|line| serde_json::from_str::<Value>(line).ok());
            let tool = message.as_ref().and_then(|m| {
                (m.get("method")?.as_str()? == "tools/call")
                    .then(|| m.get("params")?.get("name")?.as_str())
                    .flatten()
            });
            match tool {
                Some("control_call") | None => Route::Window,
                Some(_) => Route::Engine,
            }
        }
        _ => Route::Window,
    }
}

/// Answers a request that runs commands, with the session alone: on the
/// engine thread, where the shell is not. The methods the protocol lists
/// are the window's, so `methods` says the same wherever it is answered;
/// one that needs the shell is answered by the window and never sent
/// here ([`route`]), and is refused if it were.
pub fn answer_on_engine(session: &mut Session, line: &str) -> Option<String> {
    let not_here = |method: &str| {
        Some(Err(format!(
            "{method} is answered by the window, not while a command runs; send it on its own"
        )))
    };
    let mut handle = |session: &mut Session, method: &str, params: Option<&Value>| match method {
        "auth" => Some(Ok(serde_json::json!({ "authenticated": true }))),
        // A command that takes its time, reporting steps as regeneration
        // does: for the tests of a window that keeps drawing meanwhile.
        #[cfg(test)]
        "debug.slow" => Some(Ok(crate::tests::slow(params))),
        "mcp.message" => {
            let line = match params {
                Some(Value::Object(map)) if map.len() == 1 => {
                    map.get("line").and_then(Value::as_str)
                }
                _ => None,
            };
            let Some(line) = line else {
                return Some(Err(
                    "mcp.message: its one parameter is `line`, a string holding one MCP message"
                        .into(),
                ));
            };
            let mut inner = |_: &mut Session, method: &str, _: Option<&Value>| not_here(method);
            let mut extension = Extension {
                methods: mcp_methods(),
                handle: &mut inner,
            };
            let reply = craft3d_automation::mcp::handle_line_with(session, line, &mut extension);
            Some(Ok(serde_json::json!({ "reply": reply })))
        }
        _ if UI_METHODS.iter().any(|m| m.name == method) => not_here(method),
        _ => None,
    };
    let mut extension = Extension {
        methods: methods(),
        handle: &mut handle,
    };
    rpc::handle_line_with(session, line, &mut extension)
}

impl Host {
    pub fn new(session: Session) -> Self {
        Self {
            session,
            shell: Shell::new(),
            close_after_save: false,
        }
    }

    /// Answers one request line: the reply line, or `None` for a blank
    /// line. Never panics: a panic anywhere in a handler becomes an error
    /// reply, and the documents stay as they were.
    pub fn handle_line(&mut self, line: &str) -> Option<String> {
        // The protocol guards its dispatch already; this guard covers
        // everything around it: the status bar's report and bringing the
        // view up to date (AGENTS.md, last-resort guard).
        let answered =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.answer_line(line)));
        answered.unwrap_or_else(|_| {
            self.autosave_after_a_panic();
            Some(rpc::error_reply(
                &Value::Null,
                "internal error (a bug in 3DCraft, please report it); the document was kept",
            ))
        })
    }

    /// After a caught panic nobody knows what else is wrong: the last
    /// good state of every document with unsaved changes is autosaved at
    /// once, not when its interval has passed. (A panic inside a command
    /// is caught by the engine, which does the same.)
    pub(crate) fn autosave_after_a_panic(&mut self) {
        let session = &mut self.session;
        // Not worth a second panic.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            session.autosave_now();
        }));
    }

    fn answer_line(&mut self, line: &str) -> Option<String> {
        let Host { session, shell, .. } = self;
        let mut handle = |session: &mut Session, method: &str, params: Option<&Value>| {
            match method {
                // The connection authenticated before anything got here.
                "auth" => Some(Ok(serde_json::json!({ "authenticated": true }))),
                #[cfg(test)]
                "debug.panic" => crate::tests::fall_over(),
                "mcp.message" => Some(mcp_message(session, shell, params)),
                _ => shell.handle_method(session, method, params),
            }
        };
        let mut extension = Extension {
            methods: methods(),
            handle: &mut handle,
        };
        let reply = rpc::handle_line_with(session, line, &mut extension);
        if let Some(reply) = &reply
            && let Some(report) = report(line, reply)
        {
            self.shell.last_command = Some(report);
        }
        // The camera follows the model whether or not a frame is drawn
        // before the next request (a screenshot right after an edit).
        self.shell.sync(&mut self.session);
        reply
    }

    /// Answers a request and sends the reply back. A requester that has
    /// gone away is not an error.
    pub fn answer(&mut self, request: Request) {
        let reply = self.handle_line(&request.line);
        let _ = request.reply.send(reply);
    }

    /// Answers the requests that are waiting, between two frames. Returns
    /// how many there were.
    pub fn pump(&mut self, requests: &Receiver<Request>) -> usize {
        let mut answered = 0;
        while answered < MAX_PER_PUMP {
            match requests.try_recv() {
                Ok(request) => {
                    self.answer(request);
                    answered += 1;
                }
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
        answered
    }
}
