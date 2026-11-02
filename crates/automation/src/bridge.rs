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
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::PathBuf;
use std::time::Duration;

/// JSON-RPC's range for errors the server defines itself.
const BRIDGE_ERROR: i64 = -32000;
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;

/// Which window the bridge talks to.
#[derive(Clone, Debug)]
pub enum Window {
    /// A window at a known address, with the token it was started with
    /// (`--bridge PORT --control-token-file FILE`).
    Fixed { address: SocketAddr, token: String },
    /// A window found through the discovery entries in a state folder
    /// (`crate::discovery`), on `port` only when one is given: chosen at
    /// the first message and kept while it answers; when it goes away the
    /// next message looks again.
    Discover { folder: PathBuf, port: Option<u16> },
}

/// Where the app listens and how long to wait for it.
#[derive(Clone, Debug)]
pub struct BridgeConfig {
    /// The app's control channel. The token travels in the clear, so this
    /// is a loopback address; the CLI refuses any other, and discovery
    /// only ever names 127.0.0.1.
    pub window: Window,
    /// How this bridge names itself to the window (`auth`'s `client`),
    /// which counts it as a connected agent until it says `bye` or has
    /// been quiet for a while.
    pub client: Option<String>,
    pub connect_timeout: Duration,
    /// How long one message may take in the app (a large regeneration).
    pub reply_timeout: Duration,
}

impl BridgeConfig {
    pub fn new(address: SocketAddr, token: String) -> Self {
        Self::with(Window::Fixed { address, token })
    }

    /// A bridge that finds its window in `folder` (`crate::discovery`).
    pub fn discover(folder: PathBuf, port: Option<u16>) -> Self {
        Self::with(Window::Discover { folder, port })
    }

    fn with(window: Window) -> Self {
        Self {
            window,
            client: Some(format!("MCP bridge (process {})", std::process::id())),
            connect_timeout: Duration::from_secs(3),
            reply_timeout: Duration::from_secs(120),
        }
    }
}

/// The window a message goes to: where, and the token.
#[derive(Clone, Debug)]
struct Target {
    address: SocketAddr,
    token: String,
}

/// The bridge while it runs: its configuration and the window it chose.
pub struct Bridge {
    config: BridgeConfig,
    chosen: Option<Target>,
    /// True once a message reached a window: `bye` is said at the end.
    reached: bool,
}

impl Bridge {
    pub fn new(config: BridgeConfig) -> Self {
        Self {
            config,
            chosen: None,
            reached: false,
        }
    }

    /// The window to talk to now: the fixed one, the one chosen before,
    /// or one found in the discovery folder.
    fn target(&mut self) -> Result<Target, String> {
        match &self.config.window {
            Window::Fixed { address, token } => Ok(Target {
                address: *address,
                token: token.clone(),
            }),
            Window::Discover { folder, port } => {
                if let Some(chosen) = &self.chosen {
                    return Ok(chosen.clone());
                }
                let live = crate::discovery::live_entries(folder, None);
                let entry = match crate::discovery::choose(&live, *port) {
                    Ok(entry) => entry,
                    // A port given, and no entry for it (a window started
                    // with a state folder of its own): the default token.
                    Err(why) => match port {
                        Some(port) => crate::discovery::WindowEntry {
                            pid: 0,
                            port: *port,
                            token_file: crate::discovery::default_token_file(folder),
                            focused_at: 0,
                            started_at: 0,
                        },
                        None => return Err(why),
                    },
                };
                let token = crate::discovery::read_token(&entry.token_file)?;
                let target = Target {
                    address: SocketAddr::from((Ipv4Addr::LOCALHOST, entry.port)),
                    token,
                };
                self.chosen = Some(target.clone());
                Ok(target)
            }
        }
    }

    /// Passes one MCP message on; a window that went away is forgotten
    /// and, when discovered, looked for again once.
    fn exchange(&mut self, line: &str) -> Result<Option<String>, String> {
        let target = self.target()?;
        let result = match exchange(&self.config, &target, line) {
            Err(Failure::Unreachable(why)) => {
                let discovering = matches!(self.config.window, Window::Discover { .. });
                if !discovering || self.chosen.take().is_none() {
                    return Err(why);
                }
                let again = self.target()?;
                exchange(&self.config, &again, line).map_err(Failure::text)
            }
            other => other.map_err(Failure::text),
        };
        if result.is_ok() {
            self.reached = true;
        }
        result
    }

    /// Tells the window this bridge is done, so it no longer counts it as
    /// connected. Best effort: a window that is gone needs no goodbye.
    pub fn bye(&mut self) {
        if !self.reached {
            return;
        }
        let Some(target) = self.chosen.clone().or_else(|| match &self.config.window {
            Window::Fixed { address, token } => Some(Target {
                address: *address,
                token: token.clone(),
            }),
            Window::Discover { .. } => None,
        }) else {
            return;
        };
        let request = json!({ "id": 1, "method": "bye" });
        let _ = call(&self.config, &target, &request, Duration::from_secs(2));
    }
}

/// Why a message did not get through.
enum Failure {
    /// Nothing listens at the address.
    Unreachable(String),
    Other(String),
}

impl Failure {
    fn text(self) -> String {
        match self {
            Self::Unreachable(text) | Self::Other(text) => text,
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

/// Connects to the window, authenticates, sends `request` and returns
/// the window's answer to it.
fn call(
    config: &BridgeConfig,
    target: &Target,
    request: &Value,
    timeout: Duration,
) -> Result<Result<Value, String>, Failure> {
    let at = target.address;
    let fixed = matches!(config.window, Window::Fixed { .. });
    let mut stream = TcpStream::connect_timeout(&at, config.connect_timeout).map_err(|e| {
        Failure::Unreachable(if fixed {
            format!(
                "the 3DCraft window is not reachable at {at} ({e}); start it with `craft3d --control {} --control-token-file <the same token file>` and try again",
                at.port()
            )
        } else {
            format!(
                "the 3DCraft window is not reachable at {at} ({e}); open 3DCraft and switch on Allow AI agents (on the start screen or in the Help menu), then try again"
            )
        })
    })?;
    let timed = |e: io::Error| {
        Failure::Other(format!(
            "could not set up the connection to the 3DCraft window: {e}"
        ))
    };
    stream.set_read_timeout(Some(timeout)).map_err(timed)?;
    stream.set_write_timeout(Some(timeout)).map_err(timed)?;
    let _ = stream.set_nodelay(true);
    let mut reader = BufReader::new(stream.try_clone().map_err(timed)?);
    let mut auth = json!({ "token": target.token });
    if let (Some(client), Value::Object(params)) = (&config.client, &mut auth) {
        params.insert("client".to_string(), Value::String(client.clone()));
    }
    send(
        &mut stream,
        &json!({ "id": 0, "method": "auth", "params": auth }),
    )
    .map_err(Failure::Other)?;
    if let Err(reason) = receive(&mut reader).map_err(Failure::Other)? {
        let hint = if fixed {
            "; give the bridge the token file the window was started with"
        } else {
            ""
        };
        return Err(Failure::Other(format!(
            "the 3DCraft window at {at} refused the control token ({reason}){hint}"
        )));
    }
    send(&mut stream, request).map_err(Failure::Other)?;
    receive(&mut reader).map_err(Failure::Other)
}

/// Passes one MCP message to the app and returns the app's answer to it
/// (`None` for a message that gets none, such as a notification).
fn exchange(config: &BridgeConfig, target: &Target, line: &str) -> Result<Option<String>, Failure> {
    let at = target.address;
    let request = json!({ "id": 1, "method": "mcp.message", "params": { "line": line } });
    // Checked before connecting: the app would refuse the line anyway.
    if serialize(&request).len() >= MAX_REQUEST_BYTES {
        return Err(Failure::Other(format!(
            "the message is too large to pass on to the 3DCraft window (the limit is {MAX_REQUEST_BYTES} bytes)"
        )));
    }
    let result = call(config, target, &request, config.reply_timeout)?.map_err(|reason| {
        Failure::Other(format!(
            "the 3DCraft window at {at} did not take the message: {reason}"
        ))
    })?;
    match result.get("reply") {
        Some(Value::String(reply)) => Ok(Some(reply.clone())),
        Some(Value::Null) | None => Ok(None),
        Some(_) => Err(Failure::Other(
            "the 3DCraft window's answer has an unexpected shape".into(),
        )),
    }
}

/// Handles one line from the MCP client: the reply line, or `None` when
/// the message needs none. Never panics, whatever the line holds, and
/// never fails: what goes wrong becomes a JSON-RPC error for the client.
pub fn relay(bridge: &mut Bridge, line: &str) -> Option<String> {
    if line.trim().is_empty() {
        return None;
    }
    match bridge.exchange(line) {
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
/// `output` and flushing it before the next message is read; at the end of
/// the input the window is told `bye`. Returns early only when reading or
/// writing fails.
pub fn serve(
    config: &BridgeConfig,
    mut input: impl BufRead,
    mut output: impl Write,
) -> io::Result<()> {
    let mut bridge = Bridge::new(config.clone());
    loop {
        let reply = match read_line(&mut input, MAX_REQUEST_BYTES)? {
            Line::End => {
                bridge.bye();
                return Ok(());
            }
            Line::TooLong => Some(rpc_error(
                &Value::Null,
                INVALID_REQUEST,
                &format!("the message is longer than {MAX_REQUEST_BYTES} bytes"),
            )),
            Line::Bytes(bytes) => match std::str::from_utf8(&bytes) {
                Ok(line) => relay(&mut bridge, line.trim_end_matches('\r')),
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
