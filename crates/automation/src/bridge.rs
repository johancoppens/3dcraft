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
    /// How long the window may take to answer the challenge, the first
    /// line of every connection. A window answers it at once on the
    /// connection's own thread, whatever its engine is doing; a program
    /// that accepts and never answers would otherwise hold the bridge for
    /// the whole reply timeout.
    pub challenge_timeout: Duration,
    /// How long one message may take in the app (a large regeneration,
    /// or the person's answer to a prompt: the window waits two minutes
    /// for that, so this is longer).
    pub reply_timeout: Duration,
    /// The folder the MCP client works in (this process's working folder,
    /// as the client starts the bridge there), absolute with its links
    /// resolved; sent in `auth`, so the window can ask the person whether
    /// the agent may read and write files there.
    pub cwd: Option<PathBuf>,
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
            challenge_timeout: Duration::from_secs(5),
            reply_timeout: Duration::from_secs(330),
            cwd: std::env::current_dir()
                .ok()
                .and_then(|dir| std::fs::canonicalize(dir).ok())
                .map(craft3d_engine::plain),
        }
    }
}

/// The window a message goes to: where, and the token.
#[derive(Clone, Debug)]
struct Target {
    address: SocketAddr,
    token: String,
    /// The window's process, when discovery named it.
    pid: Option<u32>,
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
                pid: None,
            }),
            Window::Discover { folder, port } => {
                if let Some(chosen) = &self.chosen {
                    return Ok(chosen.clone());
                }
                let live = crate::discovery::live_entries(folder, None);
                // A port with no entry is not a window that allows agents:
                // the token is offered to no program but a discovered one.
                let entry = crate::discovery::choose(&live, *port)?;
                let token = crate::discovery::read_token(&entry.token_file)?;
                let target = Target {
                    address: SocketAddr::from((Ipv4Addr::LOCALHOST, entry.port)),
                    token,
                    pid: Some(entry.pid),
                };
                self.chosen = Some(target.clone());
                Ok(target)
            }
        }
    }

    /// Passes one MCP message on; a window that went away is forgotten
    /// and, when discovered, looked for again once.
    fn exchange(&mut self, line: &str) -> Result<Option<String>, Failure> {
        // No window found to talk to is a window not reachable too.
        let target = self.target().map_err(Failure::Unreachable)?;
        let result = match exchange(&self.config, &target, line) {
            Err(Failure::Unreachable(why)) => {
                let discovering = matches!(self.config.window, Window::Discover { .. });
                if !discovering || self.chosen.take().is_none() {
                    return Err(Failure::Unreachable(why));
                }
                let again = self.target().map_err(Failure::Unreachable)?;
                exchange(&self.config, &again, line)
            }
            other => other,
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
                pid: None,
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
    /// No window answered at the address: nothing listens there, or the
    /// program there dropped the connection or stayed silent before it
    /// proved it is the window (`before_proof`).
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

fn send(stream: &mut TcpStream, request: &Value) -> io::Result<()> {
    let mut line = serialize(request);
    line.push('\n');
    stream.write_all(line.as_bytes())?;
    stream.flush()
}

/// What a send's failure reads as once the window has answered.
fn send_text(e: &io::Error) -> String {
    format!("could not send to the 3DCraft window: {e}")
}

/// Why no reply was read.
#[derive(Debug)]
enum Received {
    /// The connection failed or was closed before a whole reply came.
    Lost(io::Error),
    /// A reply came that is not one of the control channel's.
    Bad(String),
}

impl Received {
    /// What it reads as once the window has answered before.
    fn text(self) -> String {
        match self {
            Self::Lost(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                "the 3DCraft window closed the connection without answering".into()
            }
            Self::Lost(e) => format!("no answer from the 3DCraft window: {e}"),
            Self::Bad(text) => text,
        }
    }
}

/// One reply of the control channel, as `Ok(result)` or `Err(error text)`.
fn receive(reader: &mut BufReader<TcpStream>) -> Result<Result<Value, String>, Received> {
    // The app caps its replies; a little room for the envelope around one.
    let cap = MAX_REPLY_BYTES.saturating_add(4096);
    let bytes = match read_line(reader, cap) {
        Ok(Line::Bytes(bytes)) => bytes,
        Ok(Line::End) => {
            return Err(Received::Lost(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the connection was closed without an answer",
            )));
        }
        Ok(Line::TooLong) => {
            return Err(Received::Bad(
                "the 3DCraft window's answer is too large".into(),
            ));
        }
        Err(e) => return Err(Received::Lost(e)),
    };
    let reply: Value = serde_json::from_slice(&bytes)
        .map_err(|_| Received::Bad("the 3DCraft window's answer is not JSON".to_string()))?;
    match reply.get("ok").and_then(Value::as_bool) {
        Some(true) => Ok(Ok(reply.get("result").cloned().unwrap_or(Value::Null))),
        Some(false) => Ok(Err(reply
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("it gave no reason")
            .to_string())),
        None => Err(Received::Bad(
            "the 3DCraft window's answer has no `ok` field".into(),
        )),
    }
}

/// The message for a window that cannot be reached at `at`, with the
/// operating system's reason and what to do about it.
fn not_reachable(at: SocketAddr, fixed: bool, why: &io::Error) -> String {
    if fixed {
        format!(
            "the 3DCraft window is not reachable at {at} ({why}); start it with `craft3d --control {} --control-token-file <the same token file>` and try again",
            at.port()
        )
    } else {
        format!(
            "the 3DCraft window is not reachable at {at} ({why}); open 3DCraft and switch on Allow AI agents (on the start screen or in the Help menu), then try again"
        )
    }
}

/// Whether a failure of this kind means the connection went away: the
/// peer refused it, reset or dropped it, or never answered.
fn lost(kind: io::ErrorKind) -> bool {
    use io::ErrorKind::*;
    matches!(
        kind,
        ConnectionRefused
            | ConnectionReset
            | ConnectionAborted
            | NotConnected
            | BrokenPipe
            | UnexpectedEof
            | TimedOut
            | WouldBlock
    )
}

/// A failure before the program at `at` proved it is the window: when the
/// connection went away, no window answered there, which to the agent is
/// the same as nothing listening (2026-11-04: a port taken and let go by
/// another process reset the connection, and the agent was told only
/// "connection reset").
fn before_proof(
    at: SocketAddr,
    fixed: bool,
    e: io::Error,
    text: impl FnOnce(io::Error) -> String,
) -> Failure {
    if lost(e.kind()) {
        Failure::Unreachable(not_reachable(at, fixed, &e))
    } else {
        Failure::Other(text(e))
    }
}

/// The window's refusal of a connection over its limit
/// (`apps/craft3d/src/control.rs`, `accept`), sent instead of the
/// challenge's answer: told as such, not as a program that is not the
/// window. Not proven to come from the window, so nothing is sent after it.
fn too_many(at: SocketAddr, reason: &str) -> Option<Failure> {
    reason.starts_with("too many connections").then(|| {
        Failure::Other(format!(
            "the 3DCraft window at {at} refused the connection: {reason} (other agents or scripts hold them; a silent one is closed after 30 s)"
        ))
    })
}

/// A duration in words, for messages.
fn seconds(d: Duration) -> String {
    let ms = d.as_millis();
    if ms.is_multiple_of(1000) {
        format!("{} s", ms / 1000)
    } else {
        format!("{ms} ms")
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
    let mut stream = TcpStream::connect_timeout(&at, config.connect_timeout)
        .map_err(|e| Failure::Unreachable(not_reachable(at, fixed, &e)))?;
    let timed = |e: io::Error| {
        Failure::Other(format!(
            "could not set up the connection to the 3DCraft window: {e}"
        ))
    };
    // The challenge's answer under its own short timeout, the rest under
    // the caller's.
    let challenge_timeout = config.challenge_timeout.min(timeout);
    stream
        .set_read_timeout(Some(challenge_timeout))
        .map_err(timed)?;
    stream.set_write_timeout(Some(timeout)).map_err(timed)?;
    let _ = stream.set_nodelay(true);
    let mut reader = BufReader::new(stream.try_clone().map_err(timed)?);
    // The window proves it holds the token first; the token itself never
    // crosses the socket (`proof.rs`).
    let ours = crate::proof::nonce().map_err(Failure::Other)?;
    send(
        &mut stream,
        &json!({ "id": 0, "method": "challenge", "params": { "nonce": ours } }),
    )
    .map_err(|e| before_proof(at, fixed, e, |e| send_text(&e)))?;
    let not_the_window = |why: &str| {
        Failure::Other(format!(
            "the program at {at} is not the 3DCraft window that holds this token ({why}); nothing was sent to it"
        ))
    };
    let answer = receive(&mut reader)
        .map_err(|why| match why {
            Received::Lost(e) => {
                let e = if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) {
                    io::Error::new(
                        io::ErrorKind::TimedOut,
                        format!(
                            "no answer to the challenge within {}",
                            seconds(challenge_timeout)
                        ),
                    )
                } else {
                    e
                };
                before_proof(at, fixed, e, |e| Received::Lost(e).text())
            }
            bad => Failure::Other(bad.text()),
        })?
        .map_err(|reason| match too_many(at, &reason) {
            Some(failure) => failure,
            None => not_the_window(&reason),
        })?;
    stream.set_read_timeout(Some(timeout)).map_err(timed)?;
    let theirs = answer
        .get("nonce")
        .and_then(Value::as_str)
        .filter(|nonce| crate::proof::is_nonce(nonce))
        .ok_or_else(|| not_the_window("it gave no nonce"))?
        .to_ascii_lowercase();
    let pid = answer
        .get("pid")
        .and_then(Value::as_u64)
        .and_then(|pid| u32::try_from(pid).ok())
        .ok_or_else(|| not_the_window("it gave no process id"))?;
    if target.pid.is_some_and(|wanted| wanted != pid) {
        return Err(not_the_window(
            "another process than the one that announced the window",
        ));
    }
    let shown = answer
        .get("proof")
        .and_then(Value::as_str)
        .unwrap_or_default();
    // The port this bridge connected to: a proof the window made for its
    // own port does not pass here when relayed from another one.
    let expected = crate::proof::proof(
        &target.token,
        crate::proof::WINDOW,
        at.port(),
        pid,
        &ours,
        &theirs,
    );
    if !crate::proof::same(shown, &expected) {
        let hint = if fixed {
            "; give the bridge the token file the window was started with"
        } else {
            ""
        };
        return Err(not_the_window(&format!("its proof does not match{hint}")));
    }
    let mut auth = json!({
        "proof": crate::proof::proof(&target.token, crate::proof::CLIENT, at.port(), pid, &theirs, &ours)
    });
    if let (Some(client), Value::Object(params)) = (&config.client, &mut auth) {
        params.insert("client".to_string(), Value::String(client.clone()));
    }
    if let (Some(cwd), Value::Object(params)) =
        (config.cwd.as_ref().and_then(|c| c.to_str()), &mut auth)
    {
        params.insert("cwd".to_string(), Value::String(cwd.to_string()));
    }
    send(
        &mut stream,
        &json!({ "id": 1, "method": "auth", "params": auth }),
    )
    .map_err(|e| Failure::Other(send_text(&e)))?;
    if let Err(reason) = receive(&mut reader).map_err(|why| Failure::Other(why.text()))? {
        return Err(Failure::Other(format!(
            "the 3DCraft window at {at} refused this agent ({reason})"
        )));
    }
    send(&mut stream, request).map_err(|e| Failure::Other(send_text(&e)))?;
    receive(&mut reader).map_err(|why| Failure::Other(why.text()))
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
        // What does not need the window is answered here when it is not
        // there: `initialize`, `ping` and `tools/list` (the tools are the
        // same whatever the window holds). A client that starts the
        // bridge before the window then keeps the server instead of
        // marking it failed; its tool calls say how to start the window.
        Err(Failure::Unreachable(reason)) => {
            offline(line).or_else(|| answer_failure(line, &reason))
        }
        Err(other) => answer_failure(line, &other.text()),
    }
}

/// The reply to `line` when it could not be passed on, for `reason`.
fn answer_failure(line: &str, reason: &str) -> Option<String> {
    // Only a request (it has an id) is owed an answer; the app would have
    // said the same about anything else.
    match serde_json::from_str::<Value>(line) {
        Ok(Value::Object(message)) => match message.get("id") {
            Some(id) if !id.is_null() && message.contains_key("method") => {
                Some(rpc_error(id, BRIDGE_ERROR, reason))
            }
            _ => None,
        },
        Ok(_) => Some(rpc_error(&Value::Null, INVALID_REQUEST, reason)),
        Err(_) => Some(rpc_error(&Value::Null, PARSE_ERROR, reason)),
    }
}

/// The methods answered without the window ([`offline`]).
const OFFLINE: [&str; 3] = ["initialize", "ping", "tools/list"];

/// The answer to `line` without a window, when it is one of [`OFFLINE`]:
/// the same MCP code the window runs, on an empty session, with no
/// window methods to list for `control_call`. `None` for anything else.
fn offline(line: &str) -> Option<String> {
    let message: Value = serde_json::from_str(line).ok()?;
    let method = message.get("method")?.as_str()?;
    if !OFFLINE.contains(&method) {
        return None;
    }
    let mut session = craft3d_engine::Session::new(craft3d_engine::Workspace::none());
    let mut handle = |_: &mut craft3d_engine::Session, method: &str, _: Option<&Value>| {
        Some(Err(format!("{method}: the 3DCraft window is not running")))
    };
    let mut extension = crate::rpc::Extension {
        methods: &[],
        handle: &mut handle,
    };
    crate::mcp::handle_line_with(&mut session, line, &mut extension)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn at() -> SocketAddr {
        SocketAddr::from((Ipv4Addr::LOCALHOST, 47_611))
    }

    /// Every way a connection goes away before the window has answered
    /// tells the agent the window is not reachable and how to start it,
    /// with the operating system's reason kept (2026-11-04).
    #[test]
    fn a_connection_that_goes_away_before_the_proof_is_a_window_not_reachable() {
        use io::ErrorKind::*;
        for kind in [
            ConnectionRefused,
            ConnectionReset,
            ConnectionAborted,
            NotConnected,
            BrokenPipe,
            UnexpectedEof,
            TimedOut,
            WouldBlock,
        ] {
            for fixed in [true, false] {
                let e = io::Error::new(kind, format!("reason {kind:?}"));
                let Failure::Unreachable(text) = before_proof(at(), fixed, e, |_| String::new())
                else {
                    panic!("{kind:?} is not read as a window not reachable");
                };
                assert!(text.contains("not reachable at 127.0.0.1:47611"), "{text}");
                assert!(text.contains(&format!("(reason {kind:?})")), "{text}");
                let how = if fixed {
                    "start it with `craft3d --control 47611 "
                } else {
                    "switch on Allow AI agents"
                };
                assert!(text.contains(how), "{text}");
            }
        }
        // The other kinds are not about the connection: told as they are.
        for kind in [PermissionDenied, InvalidData, OutOfMemory, Other] {
            let e = io::Error::new(kind, "why");
            let failure = before_proof(at(), true, e, |e| format!("as is: {e}"));
            assert!(
                matches!(&failure, Failure::Other(text) if text == "as is: why"),
                "{kind:?}"
            );
        }
    }

    /// A program that accepts the connection and never answers the
    /// challenge holds the bridge for the challenge's timeout, not for the
    /// reply timeout of a whole message, and reads as no window there
    /// (2026-11-04: before, 330 s, twice when discovering).
    #[test]
    fn a_silent_program_is_given_up_on_after_the_challenge_timeout() {
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let (done, wait) = std::sync::mpsc::channel::<()>();
        // Accepts, reads nothing, says nothing, until the test is over.
        let holder = std::thread::spawn(move || {
            let held = listener.accept();
            let _ = wait.recv();
            drop(held);
        });
        let mut config = BridgeConfig::new(address, "token".into());
        config.challenge_timeout = Duration::from_millis(200);
        config.reply_timeout = Duration::from_secs(600);
        let target = Target {
            address,
            token: "token".into(),
            pid: None,
        };
        let started = std::time::Instant::now();
        let outcome = call(
            &config,
            &target,
            &json!({"id": 1, "method": "x"}),
            config.reply_timeout,
        );
        let took = started.elapsed();
        let _ = done.send(());
        let _ = holder.join();
        let Err(Failure::Unreachable(text)) = outcome else {
            panic!("a silent program is not a window not reachable");
        };
        assert!(
            text.contains("(no answer to the challenge within 200 ms)"),
            "{text}"
        );
        assert!(text.contains("start it with `craft3d --control"), "{text}");
        // Far below the reply timeout; generous against a loaded machine.
        assert!(took < Duration::from_secs(120), "{took:?}");
    }

    /// Without a window, `initialize`, `ping` and `tools/list` are answered
    /// by the bridge (an MCP client that starts it before the window keeps
    /// the server instead of marking it failed, the end-to-end user test of
    /// 2026-11-04), and a tool call gets the error that says how to start
    /// the window. A notification gets nothing.
    #[test]
    fn without_a_window_the_bridge_still_introduces_itself() {
        // A port nothing listens on: bound, then let go.
        let address = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .local_addr()
            .unwrap();
        let mut bridge = Bridge::new(BridgeConfig::new(address, "token".into()));
        let ask = |bridge: &mut Bridge, line: Value| -> Value {
            serde_json::from_str(&relay(bridge, &line.to_string()).unwrap()).unwrap()
        };
        let init = ask(
            &mut bridge,
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
                   "params": {"protocolVersion": "2025-06-18"}}),
        );
        assert_eq!(init["result"]["protocolVersion"], "2025-06-18", "{init}");
        assert_eq!(
            init["result"]["serverInfo"]["name"],
            crate::mcp::SERVER_NAME
        );
        assert!(
            relay(
                &mut bridge,
                r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#
            )
            .is_none()
        );
        let pong = ask(
            &mut bridge,
            json!({"jsonrpc": "2.0", "id": 2, "method": "ping"}),
        );
        assert_eq!(pong["result"], json!({}), "{pong}");
        let list = ask(
            &mut bridge,
            json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list"}),
        );
        let names: Vec<&str> = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        for name in crate::mcp::tool_names() {
            assert!(names.contains(&name.as_str()), "{name} missing: {names:?}");
        }
        assert!(names.contains(&"control_call"), "{names:?}");
        let call = ask(
            &mut bridge,
            json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call",
                   "params": {"name": "doc_inspect", "arguments": {}}}),
        );
        assert_eq!(call["error"]["code"], BRIDGE_ERROR, "{call}");
        let message = call["error"]["message"].as_str().unwrap();
        assert!(
            message.contains("not reachable") && message.contains("start it with"),
            "{message}"
        );
    }

    /// The window's refusal of a connection over its limit is told as such.
    #[test]
    fn too_many_connections_is_told_as_such() {
        let reason = "too many connections (16 at most); close one and try again";
        let Some(Failure::Other(text)) = too_many(at(), reason) else {
            panic!("the refusal is not recognised");
        };
        assert_eq!(
            text,
            format!(
                "the 3DCraft window at 127.0.0.1:47611 refused the connection: {reason} (other agents or scripts hold them; a silent one is closed after 30 s)"
            )
        );
        assert!(too_many(at(), "auth: a proof needs a challenge first").is_none());
        assert_eq!(seconds(Duration::from_secs(5)), "5 s");
        assert_eq!(seconds(Duration::from_millis(250)), "250 ms");
    }

    /// Once the window has answered, a dropped connection is told as such:
    /// the message may have reached it, so it is not "start the window".
    #[test]
    fn a_connection_lost_after_the_proof_keeps_its_own_words() {
        let closed = Received::Lost(io::Error::new(io::ErrorKind::UnexpectedEof, "eof")).text();
        assert_eq!(
            closed,
            "the 3DCraft window closed the connection without answering"
        );
        let reset = Received::Lost(io::Error::new(io::ErrorKind::ConnectionReset, "reset")).text();
        assert_eq!(reset, "no answer from the 3DCraft window: reset");
    }
}
