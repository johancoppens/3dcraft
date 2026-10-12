//! The MCP bridge: an MCP server on stdin and stdout that has no session of
//! its own. Every message is passed, unchanged, to a running desktop app
//! over its control channel, which answers it on the document its window
//! shows (`mcp.message`, docs/control-protocol.md). An agent that speaks
//! only MCP then models in the window the developer is looking at.
//!
//! The bridge keeps no connection open: each message is one short
//! connection (authenticate, ask, read the answer). The app closes idle
//! connections after seconds, an agent thinks for minutes, and on the
//! loopback interface a connection costs nothing worth saving.
//!
//! When the app cannot be reached the agent is told so in a JSON-RPC error
//! that says how to start it; the bridge itself keeps running, so starting
//! the app is enough to carry on.

use crate::lines::{Line, read_line};
use crate::{MAX_REPLY_BYTES, MAX_REQUEST_BYTES};
use serde_json::{Value, json};
use std::io::{self, BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

/// JSON-RPC's range for errors the server defines itself.
const BRIDGE_ERROR: i64 = -32000;
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;

/// Where the app listens and how long to wait for it.
#[derive(Clone, Debug)]
pub struct BridgeConfig {
    /// The app's control channel. The token travels in the clear, so this
    /// should be a loopback address; the CLI refuses any other.
    pub address: SocketAddr,
    pub token: String,
    pub connect_timeout: Duration,
    /// How long one message may take in the app (a large regeneration).
    pub reply_timeout: Duration,
}

impl BridgeConfig {
    pub fn new(address: SocketAddr, token: String) -> Self {
        Self {
            address,
            token,
            connect_timeout: Duration::from_secs(3),
            reply_timeout: Duration::from_secs(120),
        }
    }
}

fn serialize(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| {
        r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32603,"message":"the reply could not be written as JSON"}}"#
            .to_string()
    })
}

fn rpc_error(id: &Value, code: i64, message: &str) -> String {
    serialize(&json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message },
    }))
}

fn send(stream: &mut TcpStream, request: &Value) -> Result<(), String> {
    let mut line = serialize(request);
    line.push('\n');
    stream
        .write_all(line.as_bytes())
        .and_then(|()| stream.flush())
        .map_err(|e| format!("could not send to the 3DCraft window: {e}"))
}

/// One reply of the control channel, as `Ok(result)` or `Err(error text)`.
fn receive(reader: &mut BufReader<TcpStream>) -> Result<Result<Value, String>, String> {
    // The app caps its replies; a little room for the envelope around one.
    let cap = MAX_REPLY_BYTES.saturating_add(4096);
    let bytes = match read_line(reader, cap) {
        Ok(Line::Bytes(bytes)) => bytes,
        Ok(Line::End) => {
            return Err("the 3DCraft window closed the connection without answering".into());
        }
        Ok(Line::TooLong) => return Err("the 3DCraft window's answer is too large".into()),
        Err(e) => return Err(format!("no answer from the 3DCraft window: {e}")),
    };
    let reply: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "the 3DCraft window's answer is not JSON".to_string())?;
    match reply.get("ok").and_then(Value::as_bool) {
        Some(true) => Ok(Ok(reply.get("result").cloned().unwrap_or(Value::Null))),
        Some(false) => Ok(Err(reply
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("it gave no reason")
            .to_string())),
        None => Err("the 3DCraft window's answer has no `ok` field".into()),
    }
}

/// Passes one MCP message to the app and returns the app's answer to it
/// (`None` for a message that gets none, such as a notification).
fn exchange(config: &BridgeConfig, line: &str) -> Result<Option<String>, String> {
    let at = config.address;
    let request = json!({ "id": 1, "method": "mcp.message", "params": { "line": line } });
    // Checked before connecting: the app would refuse the line anyway.
    if serialize(&request).len() >= MAX_REQUEST_BYTES {
        return Err(format!(
            "the message is too large to pass on to the 3DCraft window (the limit is {MAX_REQUEST_BYTES} bytes)"
        ));
    }
    let mut stream = TcpStream::connect_timeout(&at, config.connect_timeout).map_err(|e| {
        format!(
            "the 3DCraft window is not reachable at {at} ({e}); start it with `craft3d --control {} --control-token-file <the same token file>` and try again",
            at.port()
        )
    })?;
    let timed =
        |e: io::Error| format!("could not set up the connection to the 3DCraft window: {e}");
    stream
        .set_read_timeout(Some(config.reply_timeout))
        .map_err(timed)?;
    stream
        .set_write_timeout(Some(config.reply_timeout))
        .map_err(timed)?;
    let _ = stream.set_nodelay(true);
    let mut reader = BufReader::new(stream.try_clone().map_err(timed)?);

    send(
        &mut stream,
        &json!({ "id": 0, "method": "auth", "params": { "token": config.token } }),
    )?;
    if let Err(reason) = receive(&mut reader)? {
        return Err(format!(
            "the 3DCraft window at {at} refused the control token ({reason}); give the bridge the token file the window was started with"
        ));
    }
    send(&mut stream, &request)?;
    let result = receive(&mut reader)?.map_err(|reason| {
        format!("the 3DCraft window at {at} did not take the message: {reason}")
    })?;
    match result.get("reply") {
        Some(Value::String(reply)) => Ok(Some(reply.clone())),
        Some(Value::Null) | None => Ok(None),
        Some(_) => Err("the 3DCraft window's answer has an unexpected shape".into()),
    }
}

/// Handles one line from the MCP client: the reply line, or `None` when
/// the message needs none. Never panics, whatever the line holds, and
/// never fails: what goes wrong becomes a JSON-RPC error for the client.
pub fn relay(config: &BridgeConfig, line: &str) -> Option<String> {
    if line.trim().is_empty() {
        return None;
    }
    match exchange(config, line) {
        Ok(reply) => reply,
        Err(reason) => {
            // Only a request (it has an id) is owed an answer; the app
            // would have said the same about anything else.
            match serde_json::from_str::<Value>(line) {
                Ok(Value::Object(message)) => match message.get("id") {
                    Some(id) if !id.is_null() && message.contains_key("method") => {
                        Some(rpc_error(id, BRIDGE_ERROR, &reason))
                    }
                    _ => None,
                },
                Ok(_) => Some(rpc_error(&Value::Null, INVALID_REQUEST, &reason)),
                Err(_) => Some(rpc_error(&Value::Null, PARSE_ERROR, &reason)),
            }
        }
    }
}

/// Serves MCP messages from `input` until it ends, writing each reply to
/// `output` and flushing it before the next message is read. Returns early
/// only when reading or writing fails.
pub fn serve(
    config: &BridgeConfig,
    mut input: impl BufRead,
    mut output: impl Write,
) -> io::Result<()> {
    loop {
        let reply = match read_line(&mut input, MAX_REQUEST_BYTES)? {
            Line::End => return Ok(()),
            Line::TooLong => Some(rpc_error(
                &Value::Null,
                INVALID_REQUEST,
                &format!("the message is longer than {MAX_REQUEST_BYTES} bytes"),
            )),
            Line::Bytes(bytes) => match std::str::from_utf8(&bytes) {
                Ok(line) => relay(config, line.trim_end_matches('\r')),
                Err(_) => Some(rpc_error(
                    &Value::Null,
                    PARSE_ERROR,
                    "the message is not UTF-8 text",
                )),
            },
        };
        if let Some(reply) = reply {
            output.write_all(reply.as_bytes())?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
}
