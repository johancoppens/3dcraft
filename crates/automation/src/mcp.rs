//! The Model Context Protocol over stdio (`craft3d-cli mcp`): JSON-RPC 2.0,
//! one message per line.
//!
//! Implemented from the public MCP specification: `initialize`, `ping`,
//! `tools/list` and `tools/call`, with notifications accepted and never
//! answered. Five tools cover the whole command registry:
//!
//! | Tool | Does |
//! |---|---|
//! | `command_list` | the registry, with params notation and enablement |
//! | `command_run` | runs one command |
//! | `command_batch` | runs several in order |
//! | `doc_inspect` | `document.inspect`, or `{"document": null}` with nothing open |
//! | `doc_render_preview` | a PNG of the model as an image block |
//!
//! Errors come in the two kinds MCP distinguishes. A request the server
//! cannot act on (unknown method or tool, an unknown or ill-typed argument)
//! is a JSON-RPC error, raised before anything runs. A command that ran and
//! failed is a normal result with `isError: true`, so the model reads the
//! message and corrects itself.

use crate::fields::{Fields, describe, shorten};
use crate::lines::{self, Protocol, Rejected};
use crate::registry::{self, panic_detail};
use crate::{MAX_BATCH_STEPS, MAX_REPLY_BYTES, MAX_REQUEST_BYTES};
use craft3d_engine::Session;
use serde_json::{Map, Value, json};
use std::io::{self, BufRead, Write};

/// The MCP revisions this server speaks, latest first. `initialize` echoes
/// the client's revision when it is one of these, and offers the first
/// otherwise.
pub const PROTOCOL_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// The name the server gives itself in `initialize`.
pub const SERVER_NAME: &str = "craft3d";

/// Default and largest longest side of a preview, in pixels.
pub const PREVIEW_DEFAULT_SIDE: u64 = 1024;
pub const PREVIEW_MAX_SIDE: u64 = 2048;
/// The engine's renderer does not go below this.
const PREVIEW_MIN_SIDE: u64 = 16;
/// Largest PNG a preview returns.
const PREVIEW_MAX_PNG_BYTES: usize = 5 << 20;

const INSTRUCTIONS: &str = "3DCraft is a parametric CAD modeller. Every action is a command with a stable id and JSON params. \
Start with the tool command_list to see the commands and their params, then run them with command_run \
(first file.new or file.open, then variable.set and feature.* to model). \
Check your work with doc_inspect (volumes, areas, feature status) and doc_render_preview (a picture). \
Lengths are expressions such as \"40 mm\" or \"width / 2\". File paths resolve beneath the roots the server was started with.";

// JSON-RPC 2.0 error codes.
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const INTERNAL_ERROR: i64 = -32603;

/// A JSON-RPC error: the request was not acted on.
struct RpcError {
    code: i64,
    message: String,
}

impl RpcError {
    fn invalid_params(message: impl Into<String>) -> Self {
        Self {
            code: INVALID_PARAMS,
            message: message.into(),
        }
    }
}

/// Handles one message. Returns the reply line (without a newline), or
/// `None` when the message needs no reply: a notification, a response, or a
/// blank line. Never panics, whatever the line holds.
pub fn handle_line(session: &mut Session, line: &str) -> Option<String> {
    handle_line_capped(session, line, MAX_REPLY_BYTES)
}

/// Serves messages from `input` until it ends, writing each reply to
/// `output` and flushing it before the next message is read. Returns early
/// only when reading or writing fails.
pub fn serve(session: &mut Session, input: impl BufRead, output: impl Write) -> io::Result<()> {
    lines::serve(
        session,
        input,
        output,
        &Protocol {
            handle: handle_line,
            reject: |why: Rejected| {
                let code = match why {
                    // Bytes that are not text cannot be parsed as JSON.
                    Rejected::NotUtf8 => PARSE_ERROR,
                    Rejected::TooLong => INVALID_REQUEST,
                };
                error_reply(&Value::Null, code, &why.message())
            },
        },
    )
}

/// [`handle_line`] with the reply cap as a parameter, so tests can reach it
/// with a small one.
fn handle_line_capped(session: &mut Session, line: &str, max_reply: usize) -> Option<String> {
    if line.trim().is_empty() {
        return None;
    }
    if line.len() > MAX_REQUEST_BYTES {
        return Some(error_reply(
            &Value::Null,
            INVALID_REQUEST,
            &lines::too_long_message(),
        ));
    }
    let message: Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(e) => {
            return Some(error_reply(
                &Value::Null,
                PARSE_ERROR,
                &format!("the message is not valid JSON: {e}"),
            ));
        }
    };
    let Value::Object(message) = message else {
        let why = if message.is_array() {
            // Batches existed in one revision of MCP only (2025-03-26).
            "JSON-RPC batches are not supported; send one message per line".to_string()
        } else {
            format!(
                "a message must be a JSON-RPC 2.0 object, got {}",
                describe(&message)
            )
        };
        return Some(error_reply(&Value::Null, INVALID_REQUEST, &why));
    };

    // A request has an id (a string or a number); a notification has none.
    let id = match message.get("id") {
        None => None,
        Some(id @ (Value::String(_) | Value::Number(_))) => Some(id),
        Some(other) => {
            return Some(error_reply(
                &Value::Null,
                INVALID_REQUEST,
                &format!("`id` must be a string or a number, got {}", describe(other)),
            ));
        }
    };
    let reply_id = id.unwrap_or(&Value::Null);
    if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Some(error_reply(
            reply_id,
            INVALID_REQUEST,
            "`jsonrpc` must be \"2.0\"",
        ));
    }
    let method = match message.get("method") {
        Some(Value::String(method)) => method.as_str(),
        // A response to a request of ours. This server sends no requests,
        // so there is nothing to match it with; answering a response with
        // an error would only confuse the peer.
        None if id.is_some()
            && (message.contains_key("result") || message.contains_key("error")) =>
        {
            return None;
        }
        Some(other) => {
            return Some(error_reply(
                reply_id,
                INVALID_REQUEST,
                &format!("`method` must be a string, got {}", describe(other)),
            ));
        }
        None => {
            return Some(error_reply(
                reply_id,
                INVALID_REQUEST,
                "the message has no `method`",
            ));
        }
    };
    // Notifications are never answered, not even with an error (JSON-RPC
    // 2.0 §4.1), and none of them changes anything here:
    // `notifications/initialized` and `notifications/cancelled` need no
    // action from a server that runs every request to completion.
    let id = id?;

    // Last-resort guard (AGENTS.md, never crash). `Session::execute` already
    // catches a panic inside a command; this one covers this module.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        dispatch(session, method, message.get("params"))
    }))
    .unwrap_or_else(|payload| {
        Err(RpcError {
            code: INTERNAL_ERROR,
            message: format!(
                "{}: internal error (a bug in 3DCraft, please report it): {}",
                shorten(method),
                panic_detail(payload.as_ref())
            ),
        })
    });

    let reply = match outcome {
        Ok(result) => serialize(&json!({ "jsonrpc": "2.0", "id": id, "result": result })),
        Err(error) => return Some(error_reply(id, error.code, &error.message)),
    };
    if reply.len() > max_reply {
        return Some(error_reply(
            id,
            INTERNAL_ERROR,
            &format!(
                "{}: the reply is {} bytes, over the limit of {max_reply} bytes; ask for less at a time (a filter, fewer batch steps, a smaller max_side)",
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
) -> Result<Value, RpcError> {
    // JSON-RPC also allows positional params (an array); MCP never uses them.
    let params = match params {
        None | Some(Value::Null) => None,
        Some(Value::Object(map)) => Some(map),
        Some(other) => {
            return Err(RpcError::invalid_params(format!(
                "{}: `params` must be an object, got {}",
                shorten(method),
                describe(other)
            )));
        }
    };
    match method {
        "initialize" => initialize(params),
        "ping" => Ok(json!({})),
        // The list is one page: a `cursor` has nothing to continue.
        "tools/list" => Ok(json!({ "tools": tools() })),
        "tools/call" => call_tool(session, params),
        other => Err(RpcError {
            code: METHOD_NOT_FOUND,
            message: format!(
                "unknown method `{}`; this server implements initialize, ping, tools/list and tools/call",
                shorten(other)
            ),
        }),
    }
}

fn initialize(params: Option<&Map<String, Value>>) -> Result<Value, RpcError> {
    let latest = PROTOCOL_VERSIONS.first().copied().unwrap_or("2025-06-18");
    let version = match params.and_then(|p| p.get("protocolVersion")) {
        // The specification requires it; a client that leaves it out gets
        // our latest, as one with an unknown revision does.
        None | Some(Value::Null) => latest,
        Some(Value::String(requested)) => PROTOCOL_VERSIONS
            .iter()
            .copied()
            .find(|supported| supported == requested)
            .unwrap_or(latest),
        Some(other) => {
            return Err(RpcError::invalid_params(format!(
                "initialize: `protocolVersion` must be a string, got {}",
                describe(other)
            )));
        }
    };
    Ok(json!({
        "protocolVersion": version,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION") },
        "instructions": INSTRUCTIONS,
    }))
}

// ------------------------------------------------------------------ tools

/// The tool definitions `tools/list` returns. Every schema refuses unknown
/// arguments, and `call_tool` enforces the same before running anything.
fn tools() -> Value {
    json!([
        {
            "name": "command_list",
            "description": "Lists 3DCraft's commands: id, label, params notation, summary, and whether each can run now (enabled, disabledReason). Call this first to learn what command_run accepts.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "filter": {
                        "type": "string",
                        "description": "Case-insensitive substring of a command id or label, e.g. \"feature\"."
                    },
                    "enabled_only": {
                        "type": "boolean",
                        "description": "Only commands that can run in the current state. Default false."
                    }
                },
                "additionalProperties": false
            }
        },
        {
            "name": "command_run",
            "description": "Runs one command by id and returns its JSON result. Start with file.new or file.open. A failed command returns isError with a message that names the command and the parameter.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "A command id from command_list, e.g. \"feature.block\"."
                    },
                    "params": {
                        "type": "object",
                        "description": "The command's parameters, as its params notation in command_list describes. Omit for none."
                    }
                },
                "required": ["id"],
                "additionalProperties": false
            }
        },
        {
            "name": "command_batch",
            "description": "Runs up to 256 commands in order and returns {completed, failed, results: [{ok, result | error}]}. Not atomic: steps that ran stay applied when a later one fails; recover with edit.undo once per applied step.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "steps": {
                        "type": "array",
                        "maxItems": MAX_BATCH_STEPS,
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "string", "description": "A command id." },
                                "params": { "type": "object", "description": "The command's parameters." }
                            },
                            "required": ["id"],
                            "additionalProperties": false
                        }
                    },
                    "stop_on_error": {
                        "type": "boolean",
                        "description": "Stop at the first failing step. Default true."
                    }
                },
                "required": ["steps"],
                "additionalProperties": false
            }
        },
        {
            "name": "doc_inspect",
            "description": "Returns the active document: units, variables with values, features with status, bodies with face/edge/vertex counts, volume, area and centroid, and the undo history. Returns {\"document\": null} when no document is open.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }
        },
        {
            "name": "doc_render_preview",
            "description": "Renders the active document's bodies to a PNG image (4:3, CPU renderer) and returns it with the coverage, triangle count and body list.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "view": {
                        "type": "string",
                        "enum": ["iso", "front", "back", "left", "right", "top", "bottom"],
                        "description": "Default iso."
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["shaded", "ids"],
                        "description": "shaded (default), or ids: one flat colour per face."
                    },
                    "max_side": {
                        "type": "integer",
                        "minimum": PREVIEW_MIN_SIDE,
                        "maximum": PREVIEW_MAX_SIDE,
                        "description": "Longest side of the image in pixels. Default 1024."
                    }
                },
                "additionalProperties": false
            }
        }
    ])
}

/// How a tool call ended when it did not succeed.
enum ToolFailure {
    /// The arguments were wrong; nothing ran. A JSON-RPC error.
    Invalid(String),
    /// The command ran and failed. A result with `isError: true`.
    Failed(String),
}

/// Argument errors: `Fields` reports them as strings.
impl From<String> for ToolFailure {
    fn from(message: String) -> Self {
        ToolFailure::Invalid(message)
    }
}

/// What a tool returns: content blocks, and whether they describe a failure.
struct ToolOutput {
    content: Vec<Value>,
    is_error: bool,
}

impl ToolOutput {
    /// A JSON value as one text block.
    fn json(value: &Value) -> Self {
        Self {
            content: vec![text_block(serialize(value))],
            is_error: false,
        }
    }
}

fn text_block(text: String) -> Value {
    json!({ "type": "text", "text": text })
}

fn call_tool(
    session: &mut Session,
    params: Option<&Map<String, Value>>,
) -> Result<Value, RpcError> {
    let missing_name = || RpcError::invalid_params("tools/call: missing `name`");
    let name = match params.ok_or_else(missing_name)?.get("name") {
        Some(Value::String(name)) => name.as_str(),
        Some(other) => {
            return Err(RpcError::invalid_params(format!(
                "tools/call: `name` must be a string, got {}",
                describe(other)
            )));
        }
        None => return Err(missing_name()),
    };
    // Other keys of `params` (`_meta` with a progress token, say) are the
    // protocol's, not ours, and are ignored; only `arguments` is checked.
    let arguments = params.and_then(|p| p.get("arguments"));
    let outcome = match name {
        "command_list" => tool_command_list(session, arguments),
        "command_run" => tool_command_run(session, arguments),
        "command_batch" => tool_command_batch(session, arguments),
        "doc_inspect" => tool_doc_inspect(session, arguments),
        "doc_render_preview" => tool_doc_render_preview(session, arguments),
        other => {
            return Err(RpcError::invalid_params(format!(
                "unknown tool `{}`; the tools are: command_list, command_run, command_batch, doc_inspect, doc_render_preview",
                shorten(other)
            )));
        }
    };
    let output = match outcome {
        Ok(output) => output,
        Err(ToolFailure::Invalid(message)) => return Err(RpcError::invalid_params(message)),
        Err(ToolFailure::Failed(message)) => ToolOutput {
            content: vec![text_block(message)],
            is_error: true,
        },
    };
    Ok(json!({ "content": output.content, "isError": output.is_error }))
}

fn tool_command_list(
    session: &mut Session,
    arguments: Option<&Value>,
) -> Result<ToolOutput, ToolFailure> {
    let a = Fields::new(
        "command_list",
        "argument",
        arguments,
        &["filter", "enabled_only"],
    )?;
    let enabled_only = a.bool("enabled_only")?.unwrap_or(false);
    Ok(ToolOutput::json(&registry::command_list(
        session,
        a.str("filter")?,
        enabled_only,
    )))
}

fn tool_command_run(
    session: &mut Session,
    arguments: Option<&Value>,
) -> Result<ToolOutput, ToolFailure> {
    let a = Fields::new("command_run", "argument", arguments, &["id", "params"])?;
    let id = a.required_str("id")?;
    let params = a.object("params")?;
    match session.execute(id, params) {
        Ok(result) => Ok(ToolOutput::json(&result)),
        Err(error) => Err(ToolFailure::Failed(error.message().to_string())),
    }
}

fn tool_command_batch(
    session: &mut Session,
    arguments: Option<&Value>,
) -> Result<ToolOutput, ToolFailure> {
    let a = Fields::new(
        "command_batch",
        "argument",
        arguments,
        &["steps", "stop_on_error"],
    )?;
    let stop_on_error = a.bool("stop_on_error")?.unwrap_or(true);
    let steps = registry::parse_steps("command_batch", "argument", &a, "id")?;
    let outcome = registry::run_batch(session, &steps, stop_on_error);
    // The report is the content either way: it says which steps ran. The
    // flag makes sure a model does not read a failed batch as done.
    Ok(ToolOutput {
        is_error: outcome.failed > 0,
        ..ToolOutput::json(&outcome.report)
    })
}

fn tool_doc_inspect(
    session: &mut Session,
    arguments: Option<&Value>,
) -> Result<ToolOutput, ToolFailure> {
    Fields::new("doc_inspect", "argument", arguments, &[])?;
    if !session.has_document() {
        // Not an error: "nothing is open" is a state an agent asks about.
        return Ok(ToolOutput::json(&json!({ "document": null })));
    }
    match session.execute("document.inspect", &Value::Null) {
        Ok(result) => Ok(ToolOutput::json(&result)),
        Err(error) => Err(ToolFailure::Failed(error.message().to_string())),
    }
}

fn tool_doc_render_preview(
    session: &mut Session,
    arguments: Option<&Value>,
) -> Result<ToolOutput, ToolFailure> {
    let a = Fields::new(
        "doc_render_preview",
        "argument",
        arguments,
        &["view", "mode", "max_side"],
    )?;
    let side = a
        .integer("max_side", PREVIEW_MIN_SIDE, PREVIEW_MAX_SIDE)?
        .unwrap_or(PREVIEW_DEFAULT_SIDE);
    // 4:3, the longer side being `max_side`.
    let height = (side.saturating_mul(3) / 4).max(PREVIEW_MIN_SIDE);
    let mut params = Map::new();
    params.insert("width".into(), side.into());
    params.insert("height".into(), height.into());
    // Which views and modes exist is the command's business: it names the
    // valid ones in its error.
    for key in ["view", "mode"] {
        if let Some(value) = a.str(key)? {
            params.insert(key.into(), value.into());
        }
    }
    // Without `path` the command returns the PNG as base64.
    let mut result = session
        .execute("view.render", &Value::Object(params))
        .map_err(|error| ToolFailure::Failed(error.message().to_string()))?;
    let image = result
        .as_object_mut()
        .and_then(|map| {
            map.remove("mime");
            map.remove("base64")
        })
        .and_then(|data| match data {
            Value::String(data) => Some(data),
            _ => None,
        })
        .ok_or_else(|| {
            ToolFailure::Failed("doc_render_preview: view.render returned no image".into())
        })?;
    // Four base64 characters carry three bytes.
    let png_bytes = image.len() / 4 * 3;
    if png_bytes > PREVIEW_MAX_PNG_BYTES {
        return Err(ToolFailure::Failed(format!(
            "doc_render_preview: the PNG is about {png_bytes} bytes, over the limit of {PREVIEW_MAX_PNG_BYTES}; use a smaller max_side"
        )));
    }
    Ok(ToolOutput {
        content: vec![
            json!({ "type": "image", "data": image, "mimeType": "image/png" }),
            text_block(serialize(&result)),
        ],
        is_error: false,
    })
}

// ---------------------------------------------------------------- replies

fn error_reply(id: &Value, code: i64, message: &str) -> String {
    serialize(&json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message },
    }))
}

/// One line of JSON. Serialising a `Value` cannot fail (its keys are strings
/// and its numbers finite), but the fallback keeps that an error reply
/// rather than a panic if it ever does.
fn serialize(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| {
        r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32603,"message":"internal error: the reply could not be written as JSON"}}"#
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
        let list = r#"{"jsonrpc": "2.0", "id": "big", "method": "tools/list"}"#;

        let reply = parse(handle_line_capped(&mut session, list, 300));
        assert_eq!(reply["id"], "big");
        assert_eq!(reply["error"]["code"], INTERNAL_ERROR);
        let message = reply["error"]["message"].as_str().unwrap();
        assert!(
            message.contains("over the limit of 300 bytes") && message.contains("tools/list"),
            "{message}"
        );
        assert!(reply.get("result").is_none());

        // A reply that fits the same cap is delivered.
        let ping = r#"{"jsonrpc": "2.0", "id": 2, "method": "ping"}"#;
        let reply = parse(handle_line_capped(&mut session, ping, 300));
        assert_eq!(reply, json!({"jsonrpc": "2.0", "id": 2, "result": {}}));

        // And the uncapped handler returns the tools.
        let reply = parse(handle_line(&mut session, list));
        assert_eq!(reply["result"]["tools"].as_array().unwrap().len(), 5);
    }

    #[test]
    fn the_latest_protocol_version_is_listed_first() {
        // ISO dates sort as text.
        let mut sorted = PROTOCOL_VERSIONS.to_vec();
        sorted.sort_unstable();
        sorted.reverse();
        assert_eq!(sorted, PROTOCOL_VERSIONS);
    }
}
