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
use crate::rpc::Extension;
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
Start with the tool command_list with brief: true to see the commands and their params (then with id for the full entry of a command), \
and run them with command_run or command_batch (first file.new or file.open, then variable.set and feature.* to model). \
Check your work with doc_inspect (volumes, areas, feature status) and doc_render_preview (a picture). \
Lengths are expressions such as \"40 mm\" or \"width / 2\". File paths resolve beneath the folders agents may use: \
the roots craft3d-cli serve or mcp was started with, or, through a 3DCraft window, the work folder the person gave agents there \
(Help, Connect an AI agent).";

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
    handle_line_capped(session, line, MAX_REPLY_BYTES, None)
}

/// [`handle_line`] for a host with a window: the same protocol and tools,
/// plus the tool `control_call`, which runs one of the host's own methods
/// (the desktop app's `ui.*`: what the viewport shows, what was clicked in
/// it, a face to highlight). `extension` is what the host gives the JSON
/// protocol (`rpc::handle_line_with`): its methods are listed in the
/// tool's description and its handler answers them.
pub fn handle_line_with(
    session: &mut Session,
    line: &str,
    extension: &mut Extension<'_>,
) -> Option<String> {
    handle_line_capped(session, line, MAX_REPLY_BYTES, Some(extension))
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
fn handle_line_capped(
    session: &mut Session,
    line: &str,
    max_reply: usize,
    mut extension: Option<&mut Extension<'_>>,
) -> Option<String> {
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
        dispatch(
            session,
            method,
            message.get("params"),
            extension.as_deref_mut(),
        )
    }))
    .unwrap_or_else(|payload| {
        Err(RpcError {
            code: INTERNAL_ERROR,
            message: format!(
                "{}: {}: {}: {}",
                shorten(method),
                craft3d_math::BUG,
                craft3d_engine::PANICKED,
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
    extension: Option<&mut Extension<'_>>,
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
        "tools/list" => {
            let mut tools = tools();
            if let (Some(extension), Value::Array(list)) = (&extension, &mut tools) {
                list.push(control_call_tool(extension));
            }
            Ok(json!({ "tools": tools }))
        }
        "tools/call" => call_tool(session, params, extension),
        // The selection, readable at any time (decision B1 of 2026-10-21).
        "resources/list" => Ok(json!({ "resources": [selection_resource()] })),
        "resources/templates/list" => Ok(json!({ "resourceTemplates": [] })),
        "resources/read" => read_resource(session, params),
        other => Err(RpcError {
            code: METHOD_NOT_FOUND,
            message: format!(
                "unknown method `{}`; this server implements initialize, ping, tools/list, tools/call, resources/list, resources/templates/list and resources/read",
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
        "capabilities": { "tools": {}, "resources": {} },
        "serverInfo": { "name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION") },
        "instructions": INSTRUCTIONS,
    }))
}

// -------------------------------------------------------------- resources

/// The URI of the selection as a resource.
const SELECTION_URI: &str = "craft3d://selection";

fn selection_resource() -> Value {
    json!({
        "uri": SELECTION_URI,
        "name": "selection",
        "title": "What is selected",
        "description": "What is selected in the active document (in the window when this server is bridged to one): items {kind, name, feature} with their context (surface and measure, the drivers that set it and the edit to make, the pattern instance), count, summary, what dropped out and what is hidden for now, as the command select.get gives it. The tool selection_get gives the same.",
        "mimeType": "application/json",
    })
}

/// The selection as the command `select.get` gives it; `{"document":
/// null}` without a document.
fn selection_json(session: &mut Session) -> Result<Value, String> {
    if !session.has_document() {
        return Ok(json!({ "document": null }));
    }
    session
        .execute("select.get", &Value::Null)
        .map_err(|e| e.message().to_string())
}

fn read_resource(
    session: &mut Session,
    params: Option<&Map<String, Value>>,
) -> Result<Value, RpcError> {
    let uri = match params.and_then(|p| p.get("uri")) {
        Some(Value::String(uri)) => uri.as_str(),
        _ => {
            return Err(RpcError::invalid_params(
                "resources/read: `uri` must be a string; resources/list lists them",
            ));
        }
    };
    if uri != SELECTION_URI {
        return Err(RpcError::invalid_params(format!(
            "resources/read: unknown resource `{}`; the resources are: {SELECTION_URI}",
            shorten(uri)
        )));
    }
    let selection = selection_json(session).map_err(|message| RpcError {
        code: INTERNAL_ERROR,
        message,
    })?;
    Ok(json!({
        "contents": [{ "uri": SELECTION_URI, "mimeType": "application/json", "text": serialize(&selection) }],
    }))
}

// ------------------------------------------------------------------ tools

/// The tool a server that belongs to a window offers besides
/// [`tool_names`]: that window's own methods (`ui.get`, `ui.pick`,
/// `ui.screenshot`, ...). `craft3d-cli mcp --bridge` passes on to such a
/// server.
pub const WINDOW_TOOL: &str = "control_call";

/// The names of the tools every server offers, in the order `tools/list`
/// gives them; one that belongs to a window offers [`WINDOW_TOOL`]
/// besides. From the same definitions as `tools/list`, so that what a
/// help text lists cannot drift from what is served.
pub fn tool_names() -> Vec<String> {
    match tools() {
        Value::Array(list) => list
            .iter()
            .filter_map(|tool| tool.get("name").and_then(Value::as_str))
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

/// The tool definitions `tools/list` returns. Every schema refuses unknown
/// arguments, and `call_tool` enforces the same before running anything.
fn tools() -> Value {
    json!([
        {
            "name": "command_list",
            "description": "Lists 3DCraft's commands: id, label, params notation, summary, and whether each can run now (enabled, disabledReason). Call this first to learn what command_run accepts, with brief: true (each command's id, params and the first sentence of its summary: the whole list in a fraction of the size), then with id for the full entry of a command you use.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "brief": {
                        "type": "boolean",
                        "description": "Only id, params, the first sentence of the summary and enabled per command. Default false. Start with this."
                    },
                    "id": {
                        "type": "string",
                        "description": "One command by its exact id, e.g. \"feature.fillet\": its full entry."
                    },
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
            "description": "Runs one command by id and returns its JSON result (its wall-clock milliseconds in the tool result's _meta.ms; a command_batch step has them in its entry). Start with file.new or file.open. A failed command returns isError with a message that names the command and the parameter.",
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
                    },
                    "intent": {
                        "type": "string",
                        "maxLength": 300,
                        "description": "Why you run it, in a short sentence of your own (\"round the base edges for handling\"). Optional but asked for: the person watching sees it beside the command in the window's agent log."
                    }
                },
                "required": ["id"],
                "additionalProperties": false
            }
        },
        {
            "name": "command_batch",
            "description": "Runs up to 256 commands in order and returns {completed, failed, results: [{ok, result | error, broke?, ms}], featuresFailed?, ms}. failed counts the commands that returned an error; stop_on_error (default true) stops at the first. A command that succeeds but makes a feature fail is no failed step, since a later step may repair it: its entry lists the features under broke ({feature, id, error, causedBy?, repairedBy?: the step after which it no longer failed}); a query, or touching a feature that failed already, breaks nothing. The batch is judged at the end: featuresFailed lists the features that fail in the final state and that a step made fail ({step, feature, id, type, error, causedBy?, fix}), and the batch is an error (isError) when there are any. A feature a step edited (feature.edit, sketch.* on it) and left failing is listed too, even when it failed before. The judging starts again on the new document after a step that makes another document active (file.new, file.open, file.activate, instance.open, jig.edit, ...). stop_on_break: true also stops at the first step that makes a feature fail. Not atomic: steps that ran stay applied; edit.undo takes back the last applied step, once per step. terse reports {completed, failed, errors: [{step, error, ms}], last: {step, result, ms}, featuresFailed?, ms}. ms is the wall-clock milliseconds each step and the whole batch took: the only fields that differ from run to run (null where there is no clock).",
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
                                "params": { "type": "object", "description": "The command's parameters." },
                                "intent": { "type": "string", "maxLength": 300, "description": "Why this step, if it differs from the batch's intent." }
                            },
                            "required": ["id"],
                            "additionalProperties": false
                        }
                    },
                    "stop_on_error": {
                        "type": "boolean",
                        "description": "Stop at the first command that returns an error. Default true."
                    },
                    "stop_on_break": {
                        "type": "boolean",
                        "description": "Also stop at the first step that makes a feature fail. Default false: judged at the end (featuresFailed)."
                    },
                    "terse": {
                        "type": "boolean",
                        "description": "Report only the failures and the last result. Default false."
                    },
                    "intent": {
                        "type": "string",
                        "maxLength": 300,
                        "description": "Why you run these steps, in a short sentence of your own. Optional but asked for: the person watching sees it in the window's agent log."
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
            "name": "selection_get",
            "description": "Returns what is selected in the active document: what the developer clicked in the window (when this server is bridged to one) and what agents selected (select.set, select.query with then), as items {kind, name, feature} with their context (body, origin, surface or curve with its measure, drivers: the parameter or sketch constraint that sets it with the edit to make, copyOf: the pattern instance), count, summary (\"3 bodies, 2 faces\"), dropped (what no longer exists) and hidden (what a suppressed or rolled-back feature hides for now). Returns {\"document\": null} when no document is open. The same as the resource craft3d://selection.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }
        },
        {
            "name": "doc_render_preview",
            "description": "Renders the active document's bodies to a PNG image (4:3) as view.render does with its defaults, on the renderer it chooses (the GPU when the session has one, as craft3d-cli mcp does, else the CPU renderer, as in a window), and returns it with the coverage, triangle count, body list and which renderer drew. For a detail: projection orthographic with a box or a zoom.",
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
                    "projection": {
                        "type": "string",
                        "enum": ["perspective", "orthographic"],
                        "description": "Default perspective. orthographic shows lengths across the view to scale: use it to check a detail or a top view."
                    },
                    "zoom": {
                        "type": "number",
                        "description": "Magnifies about the centre of the picture. Default 1."
                    },
                    "box": {
                        "type": "array",
                        "description": "Two opposite corners [[x, y, z], [x, y, z]] in document units: frame this box instead of the whole model."
                    },
                    "tolerance": {
                        "type": "number",
                        "description": "Chordal tolerance of the mesh in document units. Default: between a quarter and half a pixel of the picture where it shows the model nearest, never finer than the floor (2e-4 of the diagonal of the box around the bodies), at most 12 degrees per chord, within a total budget of a million triangles (so a close-up box or a larger picture is finer). Give one to fix it."
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

/// The definition of `control_call` for a host with these methods.
fn control_call_tool(extension: &Extension<'_>) -> Value {
    let mut description = String::from(
        "Runs one method of the 3DCraft window this server belongs to: what the developer sees, not the document. Use it to read the view (ui.get: the camera, state.picked, the face, the edge or the vertex the developer clicked last, and state.highlight, state.highlightEdge and state.highlightVertex), to ask what the viewport shows at a pixel (ui.pick), to point at a face, an edge or a vertex for the developer (ui.highlight), to set the selection the developer sees (ui.select; ui.get has it under selection, the tool selection_get too) and to take a picture of the viewport (ui.screenshot). The methods:",
    );
    for method in extension.methods {
        description.push_str(&format!(
            "\n{} {}: {}",
            method.name, method.params, method.summary
        ));
    }
    if extension.methods.is_empty() {
        description.push_str(
            "\n(listed here when the window runs; it was not running when this list was made: ui.get, ui.pick, ui.highlight, ui.select and ui.screenshot are the usual ones, docs/control-protocol.md has them all)",
        );
    }
    json!({
        "name": WINDOW_TOOL,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": {
                "method": {
                    "type": "string",
                    "description": "The method, e.g. \"ui.pick\"."
                },
                "params": {
                    "type": "object",
                    "description": "The method's parameters."
                }
            },
            "required": ["method"],
            "additionalProperties": false
        }
    })
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
    extension: Option<&mut Extension<'_>>,
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
    let clock = registry::Stopwatch::start();
    let outcome = match name {
        "command_list" => tool_command_list(session, arguments),
        "command_run" => tool_command_run(session, arguments),
        "command_batch" => tool_command_batch(session, arguments),
        "doc_inspect" => tool_doc_inspect(session, arguments),
        "doc_render_preview" => tool_doc_render_preview(session, arguments),
        "selection_get" => tool_selection_get(session, arguments),
        "control_call" => match extension {
            Some(extension) => tool_control_call(session, arguments, extension),
            None => {
                return Err(RpcError::invalid_params(
                    "the tool `control_call` reaches a 3DCraft window, and this server has none: switch on Allow AI agents in 3DCraft (on the start screen or in the Help menu) and start this server as `craft3d-cli mcp --bridge`",
                ));
            }
        },
        other => {
            return Err(RpcError::invalid_params(format!(
                "unknown tool `{}`; the tools are: command_list, command_run, command_batch, doc_inspect, selection_get, doc_render_preview{}",
                shorten(other),
                if extension.is_some() {
                    ", control_call"
                } else {
                    ""
                }
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
    let mut reply = json!({ "content": output.content, "isError": output.is_error });
    // One command's wall-clock time, beside its result rather than in it
    // (the result stays what the command returns); a batch has it per
    // step in its report.
    if let ("command_run", Some(map)) = (name, reply.as_object_mut()) {
        map.insert("_meta".into(), json!({ "ms": clock.ms() }));
    }
    Ok(reply)
}

fn tool_command_list(
    session: &mut Session,
    arguments: Option<&Value>,
) -> Result<ToolOutput, ToolFailure> {
    let a = Fields::new(
        "command_list",
        "argument",
        arguments,
        &["filter", "id", "enabled_only", "brief"],
    )?;
    let query = registry::ListQuery {
        filter: a.str("filter")?,
        id: a.str("id")?,
        enabled_only: a.bool("enabled_only")?.unwrap_or(false),
        brief: a.bool("brief")?.unwrap_or(false),
    };
    Ok(ToolOutput::json(&registry::command_list(session, &query)))
}

fn tool_command_run(
    session: &mut Session,
    arguments: Option<&Value>,
) -> Result<ToolOutput, ToolFailure> {
    let a = Fields::new(
        "command_run",
        "argument",
        arguments,
        &["id", "params", "intent"],
    )?;
    let id = a.required_str("id")?;
    let params = a.object("params")?;
    let outcome = session.with_intent(a.str("intent")?, |session| session.execute(id, params));
    match outcome {
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
        &["steps", "stop_on_error", "stop_on_break", "terse", "intent"],
    )?;
    let stops = registry::BatchStops {
        on_error: a.bool("stop_on_error")?.unwrap_or(true),
        on_break: a.bool("stop_on_break")?.unwrap_or(false),
    };
    let terse = a.bool("terse")?.unwrap_or(false);
    let intent = a.str("intent")?;
    let steps = registry::parse_steps("command_batch", "argument", &a, "id")?;
    let outcome = registry::run_batch(session, &steps, &stops, terse, intent);
    // The report is the content either way: it says which steps ran. The
    // flag makes sure a model does not read a failed batch as done.
    Ok(ToolOutput {
        is_error: outcome.is_error(),
        ..ToolOutput::json(&outcome.report)
    })
}

fn tool_control_call(
    session: &mut Session,
    arguments: Option<&Value>,
    extension: &mut Extension<'_>,
) -> Result<ToolOutput, ToolFailure> {
    let a = Fields::new("control_call", "argument", arguments, &["method", "params"])?;
    let method = a.required_str("method")?;
    // Only what the host listed: nothing else of the host is reachable
    // from here, whatever its handler would answer.
    if !extension.methods.iter().any(|m| m.name == method) {
        let names: Vec<&str> = extension.methods.iter().map(|m| m.name).collect();
        return Err(ToolFailure::Invalid(format!(
            "control_call: unknown method `{}`; the methods are: {}",
            shorten(method),
            names.join(", ")
        )));
    }
    let params = a.object("params")?;
    let params = (!params.is_null()).then_some(params);
    let mut result = match (extension.handle)(session, method, params) {
        Some(Ok(result)) => result,
        Some(Err(error)) => return Err(ToolFailure::Failed(error)),
        None => {
            return Err(ToolFailure::Failed(format!(
                "control_call: the window does not answer `{}`",
                shorten(method)
            )));
        }
    };
    // A picture comes back as a picture (ui.screenshot without a path).
    let image = match result.as_object_mut() {
        Some(map) if map.get("mime").and_then(Value::as_str) == Some("image/png") => {
            match map.remove("base64") {
                Some(Value::String(data)) => {
                    map.remove("mime");
                    Some(data)
                }
                Some(other) => {
                    map.insert("base64".into(), other);
                    None
                }
                None => None,
            }
        }
        _ => None,
    };
    let mut content = Vec::new();
    if let Some(image) = image {
        let png_bytes = image.len() / 4 * 3;
        if png_bytes > PREVIEW_MAX_PNG_BYTES {
            return Err(ToolFailure::Failed(format!(
                "control_call: the PNG is about {png_bytes} bytes, over the limit of {PREVIEW_MAX_PNG_BYTES}; ask for a smaller width and height, or give a path"
            )));
        }
        content.push(json!({ "type": "image", "data": image, "mimeType": "image/png" }));
    }
    content.push(text_block(serialize(&result)));
    Ok(ToolOutput {
        content,
        is_error: false,
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

fn tool_selection_get(
    session: &mut Session,
    arguments: Option<&Value>,
) -> Result<ToolOutput, ToolFailure> {
    Fields::new("selection_get", "argument", arguments, &[])?;
    selection_json(session)
        .map(|selection| ToolOutput::json(&selection))
        .map_err(ToolFailure::Failed)
}

fn tool_doc_render_preview(
    session: &mut Session,
    arguments: Option<&Value>,
) -> Result<ToolOutput, ToolFailure> {
    let a = Fields::new(
        "doc_render_preview",
        "argument",
        arguments,
        &[
            "view",
            "mode",
            "max_side",
            "projection",
            "zoom",
            "box",
            "tolerance",
        ],
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
    for key in ["view", "mode", "projection"] {
        if let Some(value) = a.str(key)? {
            params.insert(key.into(), value.into());
        }
    }
    // Passed on as they are: the command checks them and names what is
    // wrong.
    for key in ["zoom", "box", "tolerance"] {
        if let Some(value) = arguments.and_then(|a| a.get(key)).filter(|v| !v.is_null()) {
            params.insert(key.into(), value.clone());
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

/// The reply when a reply cannot be written as JSON: a bug, said in its
/// one wording.
const UNWRITTEN: &str = concat!(
    r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32603,"message":""#,
    craft3d_math::bug_text!("the reply could not be written as JSON"),
    r#""}}"#
);

/// One line of JSON. Serialising a `Value` cannot fail (its keys are strings
/// and its numbers finite), but the fallback keeps that an error reply
/// rather than a panic if it ever does.
fn serialize(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| UNWRITTEN.to_string())
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

        let reply = parse(handle_line_capped(&mut session, list, 300, None));
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
        let reply = parse(handle_line_capped(&mut session, ping, 300, None));
        assert_eq!(reply, json!({"jsonrpc": "2.0", "id": 2, "result": {}}));

        // And the uncapped handler returns the tools.
        let reply = parse(handle_line(&mut session, list));
        assert_eq!(reply["result"]["tools"].as_array().unwrap().len(), 6);
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

#[cfg(test)]
mod unwritten_tests {
    #![allow(clippy::unwrap_used)]

    /// The fallback reply is JSON, and says a bug in the one wording.
    #[test]
    fn the_reply_that_could_not_be_written_is_json_naming_a_bug() {
        let v: serde_json::Value = serde_json::from_str(super::UNWRITTEN).unwrap();
        assert!(v.to_string().contains(craft3d_math::BUG), "{v}");
    }
}
