//! The control listener: JSON lines over TCP on 127.0.0.1, behind a token
//! (docs/control-protocol.md).
//!
//! A connection's first request must be `auth` with the token; nothing is
//! dispatched before it succeeds, and a failed attempt closes the
//! connection. After that every line goes, unparsed, through a channel to
//! the thread that owns the session (the UI thread), and the reply comes
//! back the same way: there is no second copy of the document, and no
//! command runs on a network thread.
//!
//! Input is hostile. A malformed, oversized or non-UTF-8 line gets an error
//! reply; a peer that goes away mid-line only ends its own connection.

use crate::host::Request;
use crate::token;
use craft3d_automation::{Line, MAX_REQUEST_BYTES, read_line, rpc};
use serde_json::{Value, json};
use std::io::{BufReader, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

/// Tells the thread that owns the session that a request is waiting.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone)]
pub struct ControlConfig {
    /// 0 picks a free port.
    pub port: u16,
    pub token: String,
    /// Connections served at once; one more is told so and closed.
    pub max_connections: usize,
    /// A connection that sends nothing for this long is closed, and a
    /// reply that cannot be written within it is given up.
    pub idle_timeout: Duration,
    /// How long a request may wait for the session's thread to answer.
    pub reply_timeout: Duration,
}

impl ControlConfig {
    /// The limits of docs/control-protocol.md.
    pub fn new(port: u16, token: String) -> Self {
        Self {
            port,
            token,
            max_connections: 16,
            idle_timeout: Duration::from_secs(30),
            reply_timeout: Duration::from_secs(120),
        }
    }
}

struct Shared {
    config: ControlConfig,
    requests: Mutex<Sender<Request>>,
    wake: Wake,
    connections: AtomicUsize,
    stopping: AtomicBool,
}

/// A running listener. Dropping it stops accepting connections.
pub struct ControlServer {
    address: SocketAddr,
    shared: Arc<Shared>,
    accepting: Option<JoinHandle<()>>,
}

/// One slot of the connection limit, given back when dropped.
struct Slot<'a>(&'a AtomicUsize);

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn send_line(stream: &mut TcpStream, reply: &str) -> std::io::Result<()> {
    stream.write_all(reply.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()
}

/// The reply to a connection's first request, and whether it let it in.
fn authenticate(line: &str, expected: &str) -> (bool, String) {
    let refuse = |id: &Value, why: &str| (false, rpc::error_reply(id, why));
    let first = r#"the first request must be {"id": ..., "method": "auth", "params": {"token": "<the control token>"}}"#;
    let Ok(Value::Object(request)) = serde_json::from_str::<Value>(line) else {
        return refuse(&Value::Null, &format!("not authenticated: {first}"));
    };
    let id = request.get("id").unwrap_or(&Value::Null);
    if request.get("method").and_then(Value::as_str) != Some("auth") {
        return refuse(id, &format!("not authenticated: {first}"));
    }
    let given = request
        .get("params")
        .and_then(|params| params.get("token"))
        .and_then(Value::as_str);
    match given {
        Some(given) if token::matches(expected, given) => {
            (true, rpc::ok_reply(id, &json!({ "authenticated": true })))
        }
        Some(_) => refuse(id, "auth: wrong token"),
        None => refuse(id, &format!("auth: missing `token`; {first}")),
    }
}

/// Hands a line to the session's thread and waits for its reply.
fn dispatch(shared: &Shared, line: String) -> Result<Option<String>, String> {
    let (reply, answer) = mpsc::channel();
    let sent = shared
        .requests
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .send(Request { line, reply });
    if sent.is_err() {
        return Err("the application is closing".to_string());
    }
    (shared.wake)();
    match answer.recv_timeout(shared.config.reply_timeout) {
        Ok(reply) => Ok(reply),
        Err(RecvTimeoutError::Timeout) => Err(format!(
            "the application did not answer within {} s; the request may still run",
            shared.config.reply_timeout.as_secs()
        )),
        Err(RecvTimeoutError::Disconnected) => Err("the application is closing".to_string()),
    }
}

/// Serves one connection until its peer goes away, misbehaves at the
/// socket level or fails to authenticate.
fn serve(stream: TcpStream, shared: &Shared) {
    let timeout = Some(shared.config.idle_timeout);
    if stream.set_read_timeout(timeout).is_err() || stream.set_write_timeout(timeout).is_err() {
        return;
    }
    let Ok(mut output) = stream.try_clone() else {
        return;
    };
    let mut input = BufReader::new(stream);
    let mut authenticated = false;
    loop {
        if shared.stopping.load(Ordering::SeqCst) {
            break;
        }
        let reply = match read_line(&mut input, MAX_REQUEST_BYTES) {
            // The end of the input, a timeout or a reset: all the same.
            Ok(Line::End) | Err(_) => break,
            Ok(Line::TooLong) => Some(rpc::too_long_reply()),
            Ok(Line::Bytes(bytes)) => match std::str::from_utf8(&bytes) {
                Err(_) => Some(rpc::not_utf8_reply()),
                Ok(line) if line.trim().is_empty() => continue,
                Ok(line) if !authenticated => {
                    let (accepted, reply) = authenticate(line, &shared.config.token);
                    if !accepted {
                        let _ = send_line(&mut output, &reply);
                        break;
                    }
                    authenticated = true;
                    Some(reply)
                }
                Ok(line) => match dispatch(shared, line.to_string()) {
                    Ok(reply) => reply,
                    Err(why) => Some(rpc::error_reply(&Value::Null, &why)),
                },
            },
        };
        // Before authentication nothing but `auth` is answered at length:
        // an over-long or garbled first line ends the connection too.
        if !authenticated {
            if let Some(reply) = reply {
                let _ = send_line(&mut output, &reply);
            }
            break;
        }
        if let Some(reply) = reply
            && send_line(&mut output, &reply).is_err()
        {
            break;
        }
    }
    let _ = output.shutdown(Shutdown::Both);
}

fn accept(listener: &TcpListener, shared: &Arc<Shared>) {
    for stream in listener.incoming() {
        if shared.stopping.load(Ordering::SeqCst) {
            break;
        }
        let Ok(mut stream) = stream else {
            // Out of descriptors, or the peer was gone already: not a
            // reason to stop listening, but not one to spin either.
            std::thread::sleep(Duration::from_millis(20));
            continue;
        };
        let limit = shared.config.max_connections;
        if shared.connections.fetch_add(1, Ordering::SeqCst) >= limit {
            shared.connections.fetch_sub(1, Ordering::SeqCst);
            let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
            let _ = send_line(
                &mut stream,
                &rpc::error_reply(
                    &Value::Null,
                    &format!("too many connections ({limit} at most); close one and try again"),
                ),
            );
            continue;
        }
        let for_thread = Arc::clone(shared);
        let spawned = std::thread::Builder::new()
            .name("craft3d-control".to_string())
            .spawn(move || {
                let slot = Slot(&for_thread.connections);
                serve(stream, &for_thread);
                drop(slot);
            });
        if spawned.is_err() {
            // The thread never ran, so its slot was never given back.
            shared.connections.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

impl ControlServer {
    /// Binds 127.0.0.1 and starts accepting. Requests arrive on `requests`,
    /// each followed by a call of `wake`.
    pub fn start(
        config: ControlConfig,
        requests: Sender<Request>,
        wake: Wake,
    ) -> Result<Self, String> {
        if config.token.is_empty() {
            return Err("the control channel needs a token".to_string());
        }
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, config.port)).map_err(|e| {
            format!(
                "cannot listen on 127.0.0.1:{} for the control channel: {e}",
                config.port
            )
        })?;
        let address = listener
            .local_addr()
            .map_err(|e| format!("cannot read the control channel's address: {e}"))?;
        let shared = Arc::new(Shared {
            config,
            requests: Mutex::new(requests),
            wake,
            connections: AtomicUsize::new(0),
            stopping: AtomicBool::new(false),
        });
        let for_thread = Arc::clone(&shared);
        let accepting = std::thread::Builder::new()
            .name("craft3d-control-accept".to_string())
            .spawn(move || accept(&listener, &for_thread))
            .map_err(|e| format!("cannot start the control channel's thread: {e}"))?;
        Ok(Self {
            address,
            shared,
            accepting: Some(accepting),
        })
    }

    /// Where it listens: 127.0.0.1 and the port that was bound.
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// Connections being served right now.
    pub fn connections(&self) -> usize {
        self.shared.connections.load(Ordering::SeqCst)
    }
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        self.shared.stopping.store(true, Ordering::SeqCst);
        // The accept loop is blocked in `accept`: a connection wakes it.
        let _ = TcpStream::connect_timeout(&self.address, Duration::from_secs(1));
        if let Some(thread) = self.accepting.take() {
            let _ = thread.join();
        }
    }
}
