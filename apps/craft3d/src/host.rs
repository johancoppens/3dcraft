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
}

fn methods() -> &'static [Method] {
    static ALL: std::sync::OnceLock<Vec<Method>> = std::sync::OnceLock::new();
    ALL.get_or_init(|| {
        let mut all = vec![Method {
            name: "auth",
            params: r#"{"token": string}"#,
            summary: "Authenticates the connection; must be its first request.",
        }];
        all.extend(UI_METHODS.iter().map(|m| Method {
            name: m.name,
            params: m.params,
            summary: m.summary,
        }));
        all
    })
}

fn shorten(text: &str) -> String {
    let mut out: String = text.chars().take(REPORT_CHARS).collect();
    if text.chars().count() > REPORT_CHARS {
        out.push('…');
    }
    out
}

/// What the status bar says about a request, when it ran a command.
fn report(line: &str, reply: &str) -> Option<CommandReport> {
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
        _ => return None,
    };
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

impl Host {
    pub fn new(session: Session) -> Self {
        Self {
            session,
            shell: Shell::new(),
        }
    }

    /// Answers one request line: the reply line, or `None` for a blank
    /// line. Never panics: a panic anywhere in a handler becomes an error
    /// reply, and the documents stay as they were.
    pub fn handle_line(&mut self, line: &str) -> Option<String> {
        // The protocol guards its dispatch already; this guard covers
        // everything around it: the status bar's report and bringing the
        // view up to date (AGENTS.md, last-resort guard).
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.answer_line(line)))
            .unwrap_or_else(|_| {
                Some(rpc::error_reply(
                    &Value::Null,
                    "internal error (a bug in 3DCraft, please report it); the document was kept",
                ))
            })
    }

    fn answer_line(&mut self, line: &str) -> Option<String> {
        let Host { session, shell } = self;
        let mut handle = |session: &mut Session, method: &str, params: Option<&Value>| {
            match method {
                // The connection authenticated before anything got here.
                "auth" => Some(Ok(serde_json::json!({ "authenticated": true }))),
                #[cfg(test)]
                "debug.panic" => crate::tests::fall_over(),
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
