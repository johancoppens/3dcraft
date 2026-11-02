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
//! reply; a peer that goes away mid-line only ends its own connection. A
//! first line that is an HTTP request (a web page in a browser posting to
//! localhost) closes the connection without a word: no line after it, not
//! even a JSON body that holds the right token, is read.
//!
//! The listener knows who is connected, for the window's "agents
//! connected": every authenticated connection, and every client that named
//! itself in `auth` (`client`, the MCP bridge's process) until it says
//! `bye` or has been quiet for [`ControlConfig::active_for`] (the bridge
//! opens one short connection per message). "Disconnect all" closes every
//! connection and refuses the clients it knew for the rest of the run.

use crate::host::Request;
use crate::token;
use craft3d_automation::{Line, MAX_REQUEST_BYTES, read_line, rpc};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufReader, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Tells the thread that owns the session that a request is waiting.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone)]
pub struct ControlConfig {
    /// 0 picks a free port.
    pub port: u16,
    /// How many ports to try from `port` on, the first free one taken (1:
    /// that port or an error).
    pub tries: u16,
    pub token: String,
    /// Connections served at once; one more is told so and closed.
    pub max_connections: usize,
    /// A connection that sends nothing for this long is closed, and a
    /// reply that cannot be written within it is given up.
    pub idle_timeout: Duration,
    /// How long a request may wait for the session's thread to answer.
    pub reply_timeout: Duration,
    /// How long a client that named itself counts as connected after its
    /// last request, without a connection open.
    pub active_for: Duration,
}

impl ControlConfig {
    /// The limits of docs/control-protocol.md.
    pub fn new(port: u16, token: String) -> Self {
        Self {
            port,
            tries: 1,
            token,
            max_connections: 16,
            idle_timeout: Duration::from_secs(30),
            reply_timeout: Duration::from_secs(120),
            active_for: Duration::from_secs(300),
        }
    }
}

struct Shared {
    config: ControlConfig,
    requests: Mutex<Sender<Request>>,
    wake: Wake,
    connections: AtomicUsize,
    stopping: AtomicBool,
    clients: Mutex<Clients>,
}

/// One connection being served.
struct Open {
    /// A handle on its socket, to close it from outside its thread.
    stream: TcpStream,
    authenticated: bool,
    /// The name its client gave in `auth`.
    client: Option<String>,
}

/// Who is connected, and who was sent away.
#[derive(Default)]
struct Clients {
    next: u64,
    open: BTreeMap<u64, Open>,
    /// Clients that named themselves, with their last request.
    seen: BTreeMap<String, Instant>,
    /// Clients disconnected by the person: refused for the rest of the run.
    dismissed: BTreeSet<String>,
}

impl Shared {
    fn clients(&self) -> std::sync::MutexGuard<'_, Clients> {
        self.clients.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The name a client gives itself in `auth`: printable, at most 80
/// characters; anything else is no name.
fn client_name(params: Option<&Value>) -> Option<String> {
    let name = params?.get("client")?.as_str()?.trim();
    let fits =
        !name.is_empty() && name.chars().count() <= 80 && name.chars().all(|c| !c.is_control());
    fits.then(|| name.to_string())
}

/// True for a line that starts like an HTTP request: a browser's, sent to
/// the port by a web page.
fn is_http(line: &str) -> bool {
    const METHODS: [&str; 9] = [
        "GET ", "POST ", "PUT ", "HEAD ", "OPTIONS ", "DELETE ", "PATCH ", "CONNECT ", "TRACE ",
    ];
    let start = line.trim_start();
    METHODS.iter().any(|method| {
        start
            .get(..method.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(method))
    })
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
    // One write for the line and its end: two would have the second wait
    // for the peer's delayed acknowledgement of the first (about 40 ms per
    // reply on Linux).
    let mut line = Vec::with_capacity(reply.len().saturating_add(1));
    line.extend_from_slice(reply.as_bytes());
    line.push(b'\n');
    stream.write_all(&line)?;
    stream.flush()
}

/// The reply to a connection's first request, whether it let it in, and
/// the name the client gave.
fn authenticate(line: &str, expected: &str) -> (bool, String, Option<String>) {
    let refuse = |id: &Value, why: &str| (false, rpc::error_reply(id, why), None);
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
        Some(given) if token::matches(expected, given) => (
            true,
            rpc::ok_reply(id, &json!({ "authenticated": true })),
            client_name(request.get("params")),
        ),
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

/// Lets a connection in that authenticated as `client`, unless the
/// person sent that client away. The reply to send when it may not.
fn admit(shared: &Shared, id: u64, client: Option<&String>) -> Result<(), String> {
    let mut clients = shared.clients();
    if let Some(name) = client {
        if clients.dismissed.contains(name) {
            return Err("auth: this agent was disconnected in the 3DCraft window; restart it to connect again (in Claude Code: /mcp, then reconnect)".to_string());
        }
        clients.seen.insert(name.clone(), Instant::now());
    }
    if let Some(open) = clients.open.get_mut(&id) {
        open.authenticated = true;
        open.client = client.cloned();
    }
    Ok(())
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
    let Ok(handle) = stream.try_clone() else {
        return;
    };
    let id = {
        let mut clients = shared.clients();
        let id = clients.next;
        clients.next = clients.next.wrapping_add(1);
        clients.open.insert(
            id,
            Open {
                stream: handle,
                authenticated: false,
                client: None,
            },
        );
        id
    };
    // A connection that closes while the window counts it is a change the
    // window shows.
    let gone = Gone { shared, id };
    let mut input = BufReader::new(stream);
    let mut authenticated = false;
    let mut client: Option<String> = None;
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
                // A browser's request: nothing of it is read further.
                Ok(line) if !authenticated && is_http(line) => break,
                Ok(line) if !authenticated => {
                    let (accepted, reply, name) = authenticate(line, &shared.config.token);
                    if !accepted {
                        let _ = send_line(&mut output, &reply);
                        break;
                    }
                    if let Err(why) = admit(shared, id, name.as_ref()) {
                        let _ = send_line(&mut output, &rpc::error_reply(&Value::Null, &why));
                        break;
                    }
                    authenticated = true;
                    client = name;
                    // Not woken for: the request that follows wakes the
                    // window, which then counts this client.
                    Some(reply)
                }
                Ok(line) if is_bye(line) => {
                    if let Some(name) = &client {
                        shared.clients().seen.remove(name);
                    }
                    client = None;
                    if let Some(open) = shared.clients().open.get_mut(&id) {
                        open.client = None;
                    }
                    (shared.wake)();
                    Some(rpc::ok_reply(&request_id(line), &json!({ "bye": true })))
                }
                Ok(line) => {
                    if let Some(name) = &client {
                        shared.clients().seen.insert(name.clone(), Instant::now());
                    }
                    match dispatch(shared, line.to_string()) {
                        Ok(reply) => reply,
                        Err(why) => Some(rpc::error_reply(&Value::Null, &why)),
                    }
                }
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
    drop(gone);
}

/// Takes a connection off the list when its thread ends.
struct Gone<'a> {
    shared: &'a Shared,
    id: u64,
}

impl Drop for Gone<'_> {
    fn drop(&mut self) {
        let was = self.shared.clients().open.remove(&self.id);
        if was.is_some_and(|open| open.authenticated) {
            (self.shared.wake)();
        }
    }
}

/// True for a `bye` request: the client is done.
fn is_bye(line: &str) -> bool {
    serde_json::from_str::<Value>(line)
        .ok()
        .and_then(|request| {
            request
                .get("method")
                .and_then(Value::as_str)
                .map(|m| m == "bye")
        })
        .unwrap_or(false)
}

/// The `id` of a request line, `null` when it has none.
fn request_id(line: &str) -> Value {
    serde_json::from_str::<Value>(line)
        .ok()
        .and_then(|request| request.get("id").cloned())
        .unwrap_or(Value::Null)
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
        let listener = bind(config.port, config.tries)?;
        let address = listener
            .local_addr()
            .map_err(|e| format!("cannot read the control channel's address: {e}"))?;
        let shared = Arc::new(Shared {
            config,
            requests: Mutex::new(requests),
            wake,
            connections: AtomicUsize::new(0),
            stopping: AtomicBool::new(false),
            clients: Mutex::new(Clients::default()),
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

    /// The clients connected now, by name, in order: those that named
    /// themselves and were heard from within `active_for` or have a
    /// connection open, then one "control connection" per authenticated
    /// connection without a name.
    pub fn clients(&self) -> Vec<String> {
        let clients = self.shared.clients();
        let now = Instant::now();
        let active = self.shared.config.active_for;
        let mut named: BTreeSet<String> = clients
            .seen
            .iter()
            .filter(|(_, last)| now.saturating_duration_since(**last) <= active)
            .map(|(name, _)| name.clone())
            .collect();
        let mut anonymous = 0_usize;
        for open in clients.open.values().filter(|open| open.authenticated) {
            match &open.client {
                Some(name) => {
                    named.insert(name.clone());
                }
                None => anonymous += 1,
            }
        }
        let mut list: Vec<String> = named.into_iter().collect();
        list.extend(std::iter::repeat_n(
            "control connection".to_string(),
            anonymous,
        ));
        list
    }

    /// Closes every connection and refuses the clients that named
    /// themselves for the rest of the run; returns how many clients were
    /// connected.
    pub fn disconnect_all(&self) -> usize {
        let count = self.clients().len();
        let mut clients = self.shared.clients();
        let names: Vec<String> = clients
            .seen
            .keys()
            .cloned()
            .chain(clients.open.values().filter_map(|open| open.client.clone()))
            .collect();
        clients.dismissed.extend(names);
        clients.seen.clear();
        for open in clients.open.values() {
            let _ = open.stream.shutdown(Shutdown::Both);
        }
        drop(clients);
        (self.shared.wake)();
        count
    }
}

/// Binds 127.0.0.1 on `first`, else on the ports after it, `tries` in all.
fn bind(first: u16, tries: u16) -> Result<TcpListener, String> {
    if first == 0 || tries <= 1 {
        return TcpListener::bind((Ipv4Addr::LOCALHOST, first)).map_err(|e| {
            format!("cannot listen on 127.0.0.1:{first} for the control channel: {e}")
        });
    }
    let mut last = first;
    for offset in 0..tries {
        let Some(port) = first.checked_add(offset) else {
            break;
        };
        last = port;
        if let Ok(listener) = TcpListener::bind((Ipv4Addr::LOCALHOST, port)) {
            return Ok(listener);
        }
    }
    Err(format!(
        "cannot listen for AI agents: every port from 127.0.0.1:{first} to {last} is taken (other 3DCraft windows, or other programs); close one, or start with `--control PORT`"
    ))
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        self.shared.stopping.store(true, Ordering::SeqCst);
        // The connections being served end now, not at their idle timeout.
        for open in self.shared.clients().open.values() {
            let _ = open.stream.shutdown(Shutdown::Both);
        }
        // The accept loop is blocked in `accept`: a connection wakes it.
        let _ = TcpStream::connect_timeout(&self.address, Duration::from_secs(1));
        if let Some(thread) = self.accepting.take() {
            let _ = thread.join();
        }
    }
}
