//! The JSON-lines protocol of the headless server (`craft3d-cli serve`).
//!
//! One request per line, one reply per line with the same `id`:
//!
//! ```text
//! {"id": 1, "method": "engine.execute", "params": {"command": "file.new"}}
//! {"id": 1, "ok": true, "result": {"document": 0, "units": "mm"}}
//! {"id": 2, "method": "engine.execute", "params": {"command": "feature.blok"}}
//! {"id": 2, "ok": false, "error": "unknown command `feature.blok`; did you mean `feature.block`?"}
//! ```
//!
//! Every request gets exactly one reply, whatever was wrong with it; when
//! the id could not be read the reply has `"id": null`. Blank lines are
//! skipped. Errors are sentences for an agent to act on.

use crate::fields::{Fields, describe, shorten};
use crate::lines::{self, Protocol, Rejected};
use crate::registry::{self, panic_detail};
use crate::{MAX_BATCH_STEPS, MAX_REPLY_BYTES, MAX_REQUEST_BYTES};
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::io::{self, BufRead, Write};

/// A method of the protocol, as `methods` lists it.
pub struct Method {
    pub name: &'static str,
    /// The parameters, in the notation of the command registry.
    pub params: &'static str,
    pub summary: &'static str,
}

/// Every method, in a fixed order.
pub const METHODS: &[Method] = &[
    Method {
        name: "engine.execute",
        params: r#"{"command": string, "params": object = {}, "intent": string}"#,
        summary: "Runs a command by id and returns its result. intent (optional, at most 300 characters, one line): why, in the agent's own words (\"round the base edges for handling\"); the window shows it in its agent log.",
    },
    Method {
        name: "engine.commands",
        params: r#"{"filter": string}"#,
        summary: "Lists the commands with their params, and whether each can run now. filter is a case-insensitive substring of the id or label.",
    },
    Method {
        name: "batch",
        params: r#"{"steps": [{"command": string, "params": object = {}}], "stopOnError": boolean = true, "terse": boolean = false}"#,
        summary: "Runs up to 256 commands in order and returns {completed, failed, results: [{ok, result | error}]}; with terse, {completed, failed, errors: [{step, error}], last: {step, result}}. Not a transaction: earlier steps stay applied.",
    },
    Method {
        name: "methods",
        params: "{}",
        summary: "Lists the methods of this protocol.",
    },
];

/// Handles one request line. Returns the reply line (without a newline), or
/// `None` for a blank line. Never panics, whatever the line holds.
pub fn handle_line(session: &mut Session, line: &str) -> Option<String> {
    handle_line_capped(session, line, MAX_REPLY_BYTES)
}

/// What a host's own method returned: `None` when the method is not one of
/// the host's, so the request is an unknown method.
pub type ExtensionResult = Option<Result<Value, String>>;

/// Methods a host adds to the protocol: the desktop app's `ui.*`, which act
/// on its window rather than on the session.
pub struct Extension<'a> {
    /// Listed by `methods` after the protocol's own.
    pub methods: &'a [Method],
    /// Gets every method the protocol does not know, with its params.
    pub handle: &'a mut dyn FnMut(&mut Session, &str, Option<&Value>) -> ExtensionResult,
}

/// [`handle_line`] for a host with methods of its own: the same envelope,
/// the same checks and limits, the same guard against a panic.
pub fn handle_line_with(
    session: &mut Session,
    line: &str,
    extension: &mut Extension<'_>,
) -> Option<String> {
    handle_line_extended(session, line, MAX_REPLY_BYTES, Some(extension))
}

/// The reply to a request that failed, as one line.
pub fn error_reply(id: &Value, error: &str) -> String {
    failure(id, error)
}

/// The reply to a request that succeeded, as one line.
pub fn ok_reply(id: &Value, result: &Value) -> String {
    serialize(&json!({ "id": id, "ok": true, "result": result }))
}

/// The reply to a line longer than [`MAX_REQUEST_BYTES`].
pub fn too_long_reply() -> String {
    failure(&Value::Null, &Rejected::TooLong.message())
}

/// The reply to a line that is not UTF-8.
pub fn not_utf8_reply() -> String {
    failure(&Value::Null, &Rejected::NotUtf8.message())
}

/// Serves requests from `input` until it ends, writing each reply to
/// `output` and flushing it before the next request is read. Returns early
/// only when reading or writing fails.
pub fn serve(session: &mut Session, input: impl BufRead, output: impl Write) -> io::Result<()> {
    lines::serve(
        session,
        input,
        output,
        &Protocol {
            handle: handle_line,
            reject: |why: Rejected| failure(&Value::Null, &why.message()),
        },
    )
}

/// [`handle_line`] with the reply cap as a parameter, so tests can reach it
/// with a small one.
fn handle_line_capped(session: &mut Session, line: &str, max_reply: usize) -> Option<String> {
    handle_line_extended(session, line, max_reply, None)
}

fn handle_line_extended(
    session: &mut Session,
    line: &str,
    max_reply: usize,
    mut extension: Option<&mut Extension<'_>>,
) -> Option<String> {
    if line.trim().is_empty() {
        return None;
    }
    if line.len() > MAX_REQUEST_BYTES {
        return Some(failure(&Value::Null, &lines::too_long_message()));
    }
    let request: Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(e) => {
            return Some(failure(
                &Value::Null,
                &format!("the request is not valid JSON: {e}"),
            ));
        }
    };
    let Value::Object(request) = request else {
        return Some(failure(
            &Value::Null,
            &format!(
                "a request must be a JSON object {{\"id\", \"method\", \"params\"}}, got {}",
                describe(&request)
            ),
        ));
    };
    let id = request.get("id").unwrap_or(&Value::Null);
    if let Some(unknown) = request
        .keys()
        .find(|key| !matches!(key.as_str(), "id" | "method" | "params"))
    {
        return Some(failure(
            id,
            &format!(
                "unknown request field `{}`; a request has id, method and params",
                shorten(unknown)
            ),
        ));
    }
    let method = match request.get("method") {
        Some(Value::String(method)) => method.as_str(),
        Some(other) => {
            return Some(failure(
                id,
                &format!("`method` must be a string, got {}", describe(other)),
            ));
        }
        None => {
            return Some(failure(
                id,
                "the request has no `method`; call `methods` for the list",
            ));
        }
    };
    let params = request.get("params");

    // Last-resort guard (AGENTS.md, never crash). `Session::execute` already
    // catches a panic inside a command; this one covers this module.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        dispatch(session, method, params, extension.as_deref_mut())
    }))
    .unwrap_or_else(|payload| {
        Err(format!(
            "{}: internal error (a bug in 3DCraft, please report it): {}",
            shorten(method),
            panic_detail(payload.as_ref())
        ))
    });

    let reply = match outcome {
        Ok(result) => serialize(&json!({ "id": id, "ok": true, "result": result })),
        Err(error) => return Some(failure(id, &error)),
    };
    if reply.len() > max_reply {
        return Some(failure(
            id,
            &format!(
                "{}: the reply is {} bytes, over the limit of {max_reply} bytes; ask for less at a time (a filter, fewer batch steps, a smaller image)",
                shorten(method),
                reply.len()
            ),
        ));
    }
    Some(reply)
}

fn dispatch(
    session: &mut Session,
    method: &str,
    params: Option<&Value>,
    extension: Option<&mut Extension<'_>>,
) -> Result<Value, String> {
    let extra: &[Method] = extension.as_ref().map_or(&[], |e| e.methods);
    match method {
        "engine.execute" => {
            let p = Fields::new(
                method,
                "parameter",
                params,
                &["command", "params", "intent"],
            )?;
            let command = p.required_str("command")?;
            let command_params = p.object("params")?;
            session.set_intent(p.str("intent")?);
            let outcome = session
                .execute(command, command_params)
                .map_err(|e| e.message().to_string());
            session.set_intent(None);
            outcome
        }
        "engine.commands" => {
            let p = Fields::new(method, "parameter", params, &["filter"])?;
            Ok(registry::command_list(session, p.str("filter")?, false))
        }
        "batch" => {
            let p = Fields::new(
                method,
                "parameter",
                params,
                &["steps", "stopOnError", "terse", "intent"],
            )?;
            let stop_on_error = p.bool("stopOnError")?.unwrap_or(true);
            let terse = p.bool("terse")?.unwrap_or(false);
            let intent = p.str("intent")?;
            let steps = registry::parse_steps(method, "parameter", &p, "command")?;
            Ok(registry::run_batch(session, &steps, stop_on_error, terse, intent).report)
        }
        "methods" => {
            Fields::new(method, "parameter", params, &[])?;
            let methods: Vec<Value> = METHODS
                .iter()
                .chain(extra)
                .map(|m| json!({ "name": m.name, "params": m.params, "summary": m.summary }))
                .collect();
            Ok(json!({
                "methods": methods,
                "limits": {
                    "requestBytes": MAX_REQUEST_BYTES,
                    "replyBytes": MAX_REPLY_BYTES,
                    "batchSteps": MAX_BATCH_STEPS,
                },
            }))
        }
        other => {
            if let Some(extension) = extension
                && let Some(outcome) = (extension.handle)(session, other, params)
            {
                return outcome;
            }
            let names: Vec<&str> = METHODS.iter().chain(extra).map(|m| m.name).collect();
            Err(format!(
                "unknown method `{}`; the methods are: {}",
                shorten(other),
                names.join(", ")
            ))
        }
    }
}

fn failure(id: &Value, error: &str) -> String {
    serialize(&json!({ "id": id, "ok": false, "error": error }))
}

/// One line of JSON. Serialising a `Value` cannot fail (its keys are strings
/// and its numbers finite), but the fallback keeps that an error reply
/// rather than a panic if it ever does.
fn serialize(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| {
        r#"{"id":null,"ok":false,"error":"internal error: the reply could not be written as JSON"}"#
            .to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_engine::Workspace;

    fn parse(reply: Option<String>) -> Value {
        serde_json::from_str(&reply.expect("a reply")).expect("the reply is JSON")
    }

    /// The reply cap, reached with a small cap rather than an 8 MiB reply.
    #[test]
    fn a_reply_over_the_cap_becomes_an_error_and_the_session_keeps_serving() {
        let mut session = Session::new(Workspace::none());
        let request = r#"{"id": 7, "method": "engine.commands"}"#;

        // The registry does not fit in 200 bytes.
        let reply = parse(handle_line_capped(&mut session, request, 200));
        assert_eq!(reply["id"], 7);
        assert_eq!(reply["ok"], false);
        let error = reply["error"].as_str().unwrap();
        assert!(
            error.contains("over the limit of 200 bytes") && error.contains("engine.commands"),
            "{error}"
        );

        // A reply that fits the same cap is delivered as it is.
        let small =
            r#"{"id": 8, "method": "engine.commands", "params": {"filter": "no such command"}}"#;
        let reply = parse(handle_line_capped(&mut session, small, 200));
        assert_eq!(
            reply,
            json!({"id": 8, "ok": true, "result": {"commands": []}})
        );

        // And the uncapped handler returns the registry.
        let reply = parse(handle_line(&mut session, request));
        assert_eq!(
            reply["result"]["commands"].as_array().unwrap().len(),
            Session::commands().len()
        );
    }

    /// The cap is on the bytes of the reply line, exactly.
    #[test]
    fn the_reply_cap_is_exact() {
        let mut session = Session::new(Workspace::none());
        let request = r#"{"id": 1, "method": "engine.execute", "params": {"command": "file.new"}}"#;
        let full = handle_line(&mut session, request).unwrap();
        let mut session = Session::new(Workspace::none());
        let at_cap = handle_line_capped(&mut session, request, full.len()).unwrap();
        assert_eq!(at_cap, full);
        let mut session = Session::new(Workspace::none());
        let under = parse(handle_line_capped(&mut session, request, full.len() - 1));
        assert_eq!(under["ok"], false);
    }
}
