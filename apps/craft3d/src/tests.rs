//! The control channel end to end, without a window: a real listener on a
//! free port, a real TCP client, and a thread that owns the session and
//! answers through the very channel the window pumps.

use crate::app;
use crate::args::Options;
use crate::control::{ControlConfig, ControlServer, Wake};
use crate::host::{Host, Request};
use craft3d_engine::{Access, Session, Workspace};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// `debug.slow {"ms": n, "steps": k}`: a command that takes `ms`
/// milliseconds in `k` steps and reports each as regeneration reports a
/// feature, answered on the engine thread like any command.
pub(crate) fn slow(params: Option<&Value>) -> Value {
    let number = |key: &str, default: u64| {
        params
            .and_then(|p| p.get(key))
            .and_then(Value::as_u64)
            .unwrap_or(default)
    };
    let (ms, steps) = (number("ms", 500), number("steps", 5).max(1));
    for step in 1..=steps {
        craft3d_regen::progress::report(&craft3d_regen::progress::Progress {
            phase: "regenerating",
            feature: Some(format!("F{step}")),
            name: Some(format!("Slow{step}")),
            step: step as usize,
            steps: steps as usize,
        });
        std::thread::sleep(Duration::from_millis(ms / steps));
    }
    json!({ "slept": ms })
}

/// The panic a test injects into a handler (`debug.panic`).
pub(crate) fn fall_over() -> ! {
    panic!("a handler fell over, on purpose")
}

const TOKEN: &str = "5ca1ab1e5ca1ab1e5ca1ab1e5ca1ab1e5ca1ab1e5ca1ab1e5ca1ab1e5ca1ab1e";

fn workdir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../target/tmp/craft3d-control-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A listener and the thread that stands in for the UI thread.
struct Rig {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    pump: Option<JoinHandle<Host>>,
    server: Option<ControlServer>,
    /// How many requests reached the session's thread.
    dispatched: Arc<AtomicUsize>,
    /// How many times the listener asked for a repaint.
    woken: Arc<AtomicUsize>,
}

impl Rig {
    fn start(workspace: Workspace) -> Self {
        Self::with(workspace, |_| {})
    }

    fn with(workspace: Workspace, tweak: impl FnOnce(&mut ControlConfig)) -> Self {
        let mut config = ControlConfig::new(0, TOKEN.to_string());
        tweak(&mut config);
        let (requests, incoming) = mpsc::channel::<Request>();
        let woken = Arc::new(AtomicUsize::new(0));
        let for_wake = Arc::clone(&woken);
        let wake: Wake = Arc::new(move || {
            for_wake.fetch_add(1, Ordering::SeqCst);
        });
        let server = ControlServer::start(config, requests, wake).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let dispatched = Arc::new(AtomicUsize::new(0));
        let (for_stop, for_count) = (Arc::clone(&stop), Arc::clone(&dispatched));
        let pump = std::thread::spawn(move || {
            let mut host = Host::new(Session::new(workspace));
            loop {
                match incoming.recv_timeout(Duration::from_millis(5)) {
                    Ok(request) => {
                        for_count.fetch_add(1, Ordering::SeqCst);
                        host.answer(request);
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        if for_stop.load(Ordering::SeqCst) {
                            break;
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
            host
        });
        Self {
            address: server.address(),
            stop,
            pump: Some(pump),
            server: Some(server),
            dispatched,
            woken,
        }
    }

    /// A connection that has not said anything yet.
    fn connect(&self) -> Client {
        Client::connect(self.address)
    }

    /// A connection that has authenticated.
    fn client(&self) -> Client {
        let mut client = self.connect();
        let reply = client.call("auth", json!({"token": TOKEN}));
        assert_eq!(reply["result"], json!({"authenticated": true}), "{reply}");
        client
    }

    fn connections(&self) -> usize {
        self.server.as_ref().unwrap().connections()
    }

    /// Stops everything and hands back what the session's thread owned.
    fn finish(mut self) -> Host {
        self.stop.store(true, Ordering::SeqCst);
        let host = self.pump.take().unwrap().join().unwrap();
        self.server.take();
        host
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(pump) = self.pump.take() {
            let _ = pump.join();
        }
    }
}

struct Client {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    next_id: u64,
}

impl Client {
    fn connect(address: SocketAddr) -> Self {
        let stream = TcpStream::connect(address).unwrap();
        // A reply that never comes fails the test instead of hanging it.
        stream
            .set_read_timeout(Some(Duration::from_secs(60)))
            .unwrap();
        Self {
            reader: BufReader::new(stream.try_clone().unwrap()),
            writer: stream,
            next_id: 1,
        }
    }

    fn send(&mut self, bytes: &[u8]) {
        self.writer.write_all(bytes).unwrap();
        self.writer.flush().unwrap();
    }

    /// The next reply line, or `None` when the server closed the
    /// connection.
    fn read(&mut self) -> Option<Value> {
        let mut line = String::new();
        match self.reader.read_line(&mut line) {
            Ok(0) => None,
            Ok(_) => Some(serde_json::from_str(&line).expect("a reply is one line of JSON")),
            // A reset is a closed connection too, and so is an abort
            // (what Windows reports, os error 10053, when the server
            // closed the connection with the client's bytes unread).
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
                ) =>
            {
                None
            }
            Err(e) => panic!("no reply: {e}"),
        }
    }

    fn line(&mut self, line: &str) -> Value {
        self.send(format!("{line}\n").as_bytes());
        self.read().expect("a reply")
    }

    /// Sends a request and returns the whole reply, checking its id.
    fn call(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let reply = self.line(&json!({"id": id, "method": method, "params": params}).to_string());
        assert_eq!(reply["id"], id, "{reply}");
        reply
    }

    /// The result of a request that must succeed.
    fn ok(&mut self, method: &str, params: Value) -> Value {
        let reply = self.call(method, params.clone());
        assert_eq!(reply["ok"], true, "{method} {params}: {reply}");
        reply["result"].clone()
    }

    fn execute(&mut self, command: &str, params: Value) -> Value {
        self.ok(
            "engine.execute",
            json!({"command": command, "params": params}),
        )
    }

    /// The error of a request that must fail.
    fn error(&mut self, method: &str, params: Value) -> String {
        let reply = self.call(method, params.clone());
        assert_eq!(reply["ok"], false, "{method} {params}: {reply}");
        reply["error"].as_str().unwrap().to_string()
    }
}

/// The demo part of docs/development.md, step by step.
fn build_plate_and_post(client: &mut Client) {
    client.execute("file.new", json!({"units": "mm"}));
    client.execute("variable.set", json!({"name": "width", "expr": "60 mm"}));
    client.execute(
        "feature.block",
        json!({"name": "Plate", "size": ["width", "width * 2 / 3", "8 mm"]}),
    );
    client.execute(
        "feature.cylinder",
        json!({"name": "Post", "position": ["width / 4", "width / 3", "8 mm"],
               "radius": "width / 10", "height": "30 mm"}),
    );
}

fn unbase64(text: &str) -> Vec<u8> {
    let value = |c: u8| match c {
        b'A'..=b'Z' => u32::from(c - b'A'),
        b'a'..=b'z' => u32::from(c - b'a') + 26,
        b'0'..=b'9' => u32::from(c - b'0') + 52,
        b'+' => 62,
        b'/' => 63,
        other => panic!("not base64: {other}"),
    };
    let mut out = Vec::new();
    for chunk in text.as_bytes().chunks(4) {
        let data: Vec<u8> = chunk.iter().copied().filter(|&c| c != b'=').collect();
        let mut group = 0_u32;
        for (i, &c) in data.iter().enumerate() {
            group |= value(c) << (18 - 6 * i);
        }
        out.extend(group.to_be_bytes()[1..].iter().take(data.len() - 1));
    }
    out
}

fn decode(png_bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    let mut reader = png::Decoder::new(png_bytes).read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut pixels).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgb);
    pixels.truncate(info.buffer_size());
    (info.width, info.height, pixels)
}

/// Waits until `done` holds, for a bounded time.
fn eventually(what: &str, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "timed out: {what}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn nothing_is_dispatched_before_auth_succeeds() {
    let rig = Rig::start(Workspace::none());
    let wrong_token = TOKEN.replace('5', "6");
    let refused = [
        // A command before `auth`.
        json!({"id": 1, "method": "engine.execute", "params": {"command": "file.new"}}).to_string(),
        json!({"id": 1, "method": "ui.quit"}).to_string(),
        json!({"id": 1, "method": "methods"}).to_string(),
        // Wrong, missing and malformed tokens.
        json!({"id": 1, "method": "auth", "params": {"token": wrong_token}}).to_string(),
        json!({"id": 1, "method": "auth", "params": {"token": ""}}).to_string(),
        json!({"id": 1, "method": "auth", "params": {"token": &TOKEN[..63]}}).to_string(),
        json!({"id": 1, "method": "auth", "params": {"token": format!("{TOKEN}00")}}).to_string(),
        json!({"id": 1, "method": "auth", "params": {"token": 7}}).to_string(),
        json!({"id": 1, "method": "auth", "params": {"token": [TOKEN]}}).to_string(),
        json!({"id": 1, "method": "auth", "params": TOKEN}).to_string(),
        json!({"id": 1, "method": "auth"}).to_string(),
        json!({"id": 1, "method": "AUTH", "params": {"token": TOKEN}}).to_string(),
        json!([{"method": "auth", "params": {"token": TOKEN}}]).to_string(),
        "not json at all".to_string(),
        format!("\"{TOKEN}\""),
    ];
    for line in &refused {
        let mut client = rig.connect();
        let reply = client.line(line);
        assert_eq!(reply["ok"], false, "{line}: {reply}");
        let error = reply["error"].as_str().unwrap();
        assert!(
            error.contains("not authenticated") || error.starts_with("auth:"),
            "{line}: {error}"
        );
        assert!(!error.contains(TOKEN), "the token is never echoed");
        // One attempt per connection: it is closed, and a second request
        // on it goes nowhere.
        let _ = client.writer.write_all(
            b"{\"id\":2,\"method\":\"engine.execute\",\"params\":{\"command\":\"file.new\"}}\n",
        );
        assert_eq!(client.read(), None, "{line}: the connection stays open");
    }
    assert_eq!(rig.dispatched.load(Ordering::SeqCst), 0);
    assert_eq!(rig.woken.load(Ordering::SeqCst), 0);

    // The right token lets the connection in, with the request's id.
    let mut client = rig.connect();
    let reply = client
        .line(&json!({"id": "hello", "method": "auth", "params": {"token": TOKEN}}).to_string());
    assert_eq!(
        reply,
        json!({"id": "hello", "ok": true, "result": {"authenticated": true}})
    );
    assert_eq!(
        rig.dispatched.load(Ordering::SeqCst),
        0,
        "auth is not dispatched"
    );
    // And nothing the refused connections sent took effect.
    let state = client.ok("ui.get", json!({}));
    assert_eq!(state["document"], Value::Null);
    assert_eq!(rig.dispatched.load(Ordering::SeqCst), 1);
    assert_eq!(
        rig.woken.load(Ordering::SeqCst),
        1,
        "the window is woken per request"
    );
    // Authenticating again on an open connection is harmless.
    assert_eq!(
        client.ok("auth", json!({"token": "whatever"})),
        json!({"authenticated": true})
    );
    let host = rig.finish();
    assert!(!host.session.has_document());
    assert!(!host.shell.quit_requested());
}

/// A web page in a browser can post to the port (a "simple" request needs
/// no preflight). Its first line is the HTTP request line, and the
/// connection is closed on it, unanswered: the body, here the right token
/// and a command, is never read, even when it is in the same packet.
#[test]
fn an_http_request_is_closed_before_its_body_is_read() {
    let rig = Rig::start(Workspace::none());
    let auth = json!({"id": 1, "method": "auth", "params": {"token": TOKEN}}).to_string();
    let command = json!({"id": 2, "method": "engine.execute",
        "params": {"command": "file.new"}})
    .to_string();
    let body = format!("{auth}\n{command}\n");
    for head in [
        "POST / HTTP/1.1",
        "GET /?x HTTP/1.1",
        "post /mcp HTTP/1.0",
        "OPTIONS * HTTP/1.1",
        "PUT /x HTTP/1.1",
    ] {
        let request = format!(
            "{head}\r\nHost: 127.0.0.1\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let mut client = rig.connect();
        client.send(request.as_bytes());
        assert_eq!(client.read(), None, "{head}: closed without a reply");
    }
    // The body alone, first, is a client like any other: the token works.
    let mut client = rig.connect();
    client.send(body.as_bytes());
    assert_eq!(client.read().unwrap()["ok"], true);
    assert_eq!(client.read().unwrap()["ok"], true);
    // Of the HTTP requests nothing reached the session.
    assert_eq!(rig.dispatched.load(Ordering::SeqCst), 1);
    let host = rig.finish();
    assert!(
        host.session.has_document(),
        "only the plain client's file.new ran"
    );
}

/// The challenge (2026-11-02): the window proves it holds the token, the
/// client proves it too, and the token never crosses the socket.
#[test]
fn the_window_and_the_client_prove_the_token_to_each_other() {
    use craft3d_automation::proof;
    let rig = Rig::start(Workspace::none());
    let ours = "1".repeat(64);
    let mut client = rig.connect();
    let answer = client.ok("challenge", json!({"nonce": ours}));
    let theirs = answer["nonce"].as_str().unwrap().to_string();
    assert!(proof::is_nonce(&theirs));
    let (port, pid) = (rig.address.port(), std::process::id());
    assert_eq!(answer["pid"], pid);
    assert_eq!(
        answer["proof"],
        proof::proof(TOKEN, proof::WINDOW, port, pid, &ours, &theirs),
        "the window's proof, for its own port and process"
    );
    let reply = client.call(
        "auth",
        json!({"proof": proof::proof(TOKEN, proof::CLIENT, port, pid, &theirs, &ours), "client": "test"}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(client.ok("ui.get", json!({}))["document"], Value::Null);
    // A wrong proof, a proof without a challenge, a bad nonce, two
    // challenges: refused and closed, nothing dispatched.
    let before = rig.dispatched.load(Ordering::SeqCst);
    let mut wrong = rig.connect();
    let answer = wrong.ok("challenge", json!({"nonce": ours}));
    let theirs = answer["nonce"].as_str().unwrap().to_string();
    let reply = wrong.call(
        "auth",
        json!({"proof": proof::proof(&"0".repeat(64), proof::CLIENT, port, pid, &theirs, &ours)}),
    );
    assert_eq!(reply["ok"], false);
    assert_eq!(wrong.read(), None);
    let mut early = rig.connect();
    let reply = early.call("auth", json!({"proof": "00".repeat(32)}));
    assert!(
        reply["error"].as_str().unwrap().contains("challenge first"),
        "{reply}"
    );
    let mut bad = rig.connect();
    assert_eq!(bad.call("challenge", json!({"nonce": "12"}))["ok"], false);
    assert_eq!(bad.read(), None);
    let mut twice = rig.connect();
    twice.ok("challenge", json!({"nonce": ours}));
    assert_eq!(twice.call("challenge", json!({"nonce": ours}))["ok"], false);
    assert_eq!(rig.dispatched.load(Ordering::SeqCst), before);
}

/// A program on another port that relays the bridge's challenge to the
/// real window (re-review, 2026-11-02): the window's proof is for its own
/// port, so the bridge, which checks it with the port it connected to,
/// sends nothing further.
#[test]
fn a_relayed_proof_does_not_pass() {
    use craft3d_automation::bridge::{self, BridgeConfig};
    let rig = Rig::start(Workspace::none());
    let window = rig.address;
    let relay = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let relay_address = relay.local_addr().unwrap();
    // The relay copies every byte both ways, and keeps what it saw.
    let seen = Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let for_relay = Arc::clone(&seen);
    std::thread::spawn(move || {
        let Ok((client, _)) = relay.accept() else {
            return;
        };
        let Ok(server) = TcpStream::connect(window) else {
            return;
        };
        let (mut client_in, mut server_out) =
            (client.try_clone().unwrap(), server.try_clone().unwrap());
        let copy_up = std::thread::spawn(move || {
            let mut buffer = [0_u8; 4096];
            while let Ok(n) = std::io::Read::read(&mut client_in, &mut buffer) {
                if n == 0 {
                    break;
                }
                for_relay.lock().unwrap().extend_from_slice(&buffer[..n]);
                if server_out.write_all(&buffer[..n]).is_err() {
                    break;
                }
            }
            let _ = server_out.shutdown(std::net::Shutdown::Both);
        });
        let (mut server_in, mut client_out) = (server, client);
        let mut buffer = [0_u8; 4096];
        while let Ok(n) = std::io::Read::read(&mut server_in, &mut buffer) {
            if n == 0 || client_out.write_all(&buffer[..n]).is_err() {
                break;
            }
        }
        let _ = client_out.shutdown(std::net::Shutdown::Both);
        let _ = copy_up.join();
    });
    let message = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
    let mut output = Vec::new();
    bridge::serve(
        &BridgeConfig::new(relay_address, TOKEN.to_string()),
        format!("{message}\n").as_bytes(),
        &mut output,
    )
    .unwrap();
    let reply: Value = serde_json::from_slice(&output).unwrap();
    let error = reply["error"]["message"].as_str().unwrap();
    assert!(error.contains("nothing was sent"), "{error}");
    let seen = String::from_utf8(seen.lock().unwrap().clone()).unwrap();
    assert!(
        !seen.contains("\"auth\""),
        "no auth went through the relay: {seen}"
    );
    assert!(!seen.contains(TOKEN));
    assert_eq!(rig.dispatched.load(Ordering::SeqCst), 0);
}

#[test]
fn a_script_builds_a_part_and_measures_the_closed_form_volume() {
    let rig = Rig::start(Workspace::none());
    let mut client = rig.client();
    build_plate_and_post(&mut client);
    let measured = client.execute("document.measure", json!({}));
    // A 60 × 40 × 8 plate and a post of radius 6 and height 30.
    let plate = 60.0 * 40.0 * 8.0;
    let post = std::f64::consts::PI * 6.0 * 6.0 * 30.0;
    let volume = measured["volume"].as_f64().unwrap();
    assert!(
        (volume - (plate + post)).abs() <= 1e-9 * (plate + post),
        "volume {volume}, expected {}",
        plate + post
    );
    assert_eq!(measured["volumeUnit"], "mm^3");
    println!(
        "measured volume {volume} mm^3, closed form {}",
        plate + post
    );

    // The window's view of the same document: two bodies, framed.
    let state = client.ok("ui.get", json!({}));
    assert_eq!(state["scene"]["bodies"], 2);
    assert_eq!(state["features"].as_array().unwrap().len(), 2);
    assert_eq!(
        state["state"]["camera"]["target"],
        json!([30.0, 20.0, 19.0])
    );
    assert_eq!(state["lastCommand"]["label"], "document.measure");
    assert_eq!(state["lastCommand"]["ok"], true);

    // A batch is the protocol's own method and works here too; a failing
    // command is an error reply written for an agent.
    let batch = client.ok(
        "batch",
        json!({"steps": [
            {"command": "variable.set", "params": {"name": "width", "expr": "80 mm"}},
            {"command": "document.measure"},
        ]}),
    );
    assert_eq!(batch["completed"], 2);
    let wider = batch["results"][1]["result"]["volume"].as_f64().unwrap();
    let expected = 80.0 * (160.0 / 3.0) * 8.0 + std::f64::consts::PI * 8.0 * 8.0 * 30.0;
    assert!(
        (wider - expected).abs() <= 1e-9 * expected,
        "{wider} vs {expected}"
    );
    let error = client.error("engine.execute", json!({"command": "feature.blok"}));
    assert!(error.contains("did you mean `feature.block`"), "{error}");

    // The methods of the protocol and of the window, in one list.
    let methods = client.ok("methods", json!({}));
    let names: Vec<&str> = methods["methods"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "engine.execute",
            "engine.commands",
            "batch",
            "methods",
            "auth",
            "bye",
            "mcp.message",
            "ui.get",
            "ui.set",
            "ui.pick",
            "ui.highlight",
            "ui.select",
            "ui.fit",
            "ui.screenshot",
            "ui.record",
            "ui.advance",
            "ui.param",
            "ui.notice",
            "ui.quit"
        ]
    );
    let error = client.error("ui.nope", json!({}));
    assert!(
        error.contains("unknown method") && error.contains("ui.screenshot"),
        "{error}"
    );

    let host = rig.finish();
    // The status bar shows the last command and how it ended.
    let report = host.shell.last_command.clone().unwrap();
    assert_eq!(report.label, "feature.blok");
    assert!(!report.ok && report.text.contains("did you mean"));
    assert_eq!(host.shell.tree_rows().len(), 2);
}

#[test]
fn ui_screenshot_over_tcp_is_a_png_of_the_requested_size_that_is_not_blank() {
    let dir = workdir("screenshot");
    let rig = Rig::start(Workspace {
        read: Access::None,
        write: Access::Root(dir.clone()),
    });
    let mut client = rig.client();
    build_plate_and_post(&mut client);
    client.ok("ui.notice", json!({"text": "Plate and post"}));
    let shot = client.ok("ui.screenshot", json!({"width": 400, "height": 300}));
    assert_eq!(shot["mime"], "image/png");
    let png = unbase64(shot["base64"].as_str().unwrap());
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    let (width, height, rgb) = decode(&png);
    assert_eq!((width, height), (400, 300));
    // Not blank: the part covers more than a tenth of the picture (the
    // floor), and the decoded pixels agree with the coverage reported.
    let coverage = shot["coverage"].as_f64().unwrap();
    assert!(coverage > 0.10, "coverage {coverage}");
    let background = [rgb[0], rgb[1], rgb[2]];
    let drawn = rgb
        .as_chunks::<3>()
        .0
        .iter()
        .filter(|p| **p != background)
        .count() as f64
        / (400.0 * 300.0);
    assert!(
        drawn >= coverage - 0.01,
        "{drawn} drawn, coverage {coverage}"
    );
    println!("screenshot 400x300: coverage {coverage:.3}, drawn {drawn:.3}");

    // To a file under the write root, and nowhere else.
    let shot = client.ok(
        "ui.screenshot",
        json!({"path": "shot.png", "width": 320, "height": 240}),
    );
    assert_eq!(shot["path"], "shot.png");
    let (width, height, _) = decode(&std::fs::read(dir.join("shot.png")).unwrap());
    assert_eq!((width, height), (320, 240));
    let error = client.error("ui.screenshot", json!({"path": "../outside.png"}));
    assert!(error.starts_with("ui.screenshot:"), "{error}");
    assert!(!dir.parent().unwrap().join("outside.png").exists());

    // The camera is the one `ui.set` placed: a view from the top shows the
    // plate as a 60 × 40 rectangle, wider than high.
    client.ok("ui.set", json!({"camera": {"projection": "orthographic"}}));
    client.ok("ui.fit", json!({"view": "top"}));
    // The window's viewport is 3:2 (960 × 640 before any window lays it
    // out) and so is the plate, so it is framed on both sides at once,
    // with the 5 % margin. In a 300 × 200 picture of the same shape the
    // plate is 300 / 1.05 = 285.7 pixels wide and 200 / 1.05 = 190.5 high.
    let shot = client.ok("ui.screenshot", json!({"width": 300, "height": 200}));
    let (_, _, rgb) = decode(&unbase64(shot["base64"].as_str().unwrap()));
    let background = [rgb[0], rgb[1], rgb[2]];
    // Seen from the top the plate is one flat face of one shade: the
    // commonest colour that is not the background. A pixel of the plate is
    // of that shade, not an axis, not the background, and not the line
    // along the plate's edges, which is drawn on its outline and so
    // reaches half its width beyond it (over the light background of
    // 2026-10-21 that half is a light grey, so the shade is what tells).
    let mut counts = std::collections::BTreeMap::new();
    for p in rgb.chunks(3).filter(|p| *p != background) {
        *counts.entry([p[0], p[1], p[2]]).or_insert(0_usize) += 1;
    }
    let face = counts
        .iter()
        .max_by_key(|(_, n)| **n)
        .map(|(p, _)| *p)
        .unwrap();
    let body = |x: usize, y: usize| {
        let i = (y * 300 + x) * 3;
        let p = [rgb[i], rgb[i + 1], rgb[i + 2]];
        p.iter().zip(face).all(|(a, b)| a.abs_diff(b) <= 8)
    };
    let columns = (0..300).filter(|&x| (0..200).any(|y| body(x, y))).count();
    let rows = (0..200).filter(|&y| (0..300).any(|x| body(x, y))).count();
    // Less the inner half of the edge line on either side, which is not
    // the face's shade: half of `edge_width` (one point) each side.
    assert!((columns as f64 - 284.7).abs() <= 2.0, "{columns} columns");
    assert!((rows as f64 - 189.5).abs() <= 2.0, "{rows} rows");
    drop(client);
    rig.finish();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn hostile_lines_and_dropped_connections_leave_the_server_running() {
    let rig = Rig::start(Workspace::none());
    let mut client = rig.client();
    client.execute("file.new", json!({}));
    client.execute("feature.sphere", json!({"radius": 3}));

    // Malformed lines: an error reply each, and the connection stays.
    for bad in [
        "not json",
        "{",
        "[1, 2, 3]",
        "42",
        "\"engine.execute\"",
        r#"{"id": 1}"#,
        r#"{"id": 1, "method": 7}"#,
        r#"{"id": 1, "method": "engine.execute", "params": 7}"#,
        r#"{"id": 1, "method": "engine.execute", "extra": 1}"#,
        r#"{"id": 1, "method": "ui.set", "params": {"camera": {"yaw": "x"}}}"#,
        r#"{"id": 1, "method": "ui.screenshot", "params": {"width": 1e99}}"#,
    ] {
        let reply = client.line(bad);
        assert_eq!(reply["ok"], false, "{bad}: {reply}");
        assert!(
            !reply["error"].as_str().unwrap().contains("internal error"),
            "{bad}"
        );
    }
    // Blank lines get no reply at all; the next request gets its own.
    client.send(b"\n   \n\r\n");
    assert_eq!(
        client.ok("ui.notice", json!({"text": "still here"}))["notice"],
        "still here"
    );

    // A line that is not UTF-8.
    client.send(b"{\"id\": 1, \"method\": \"\xff\xfe\"}\n");
    let reply = client.read().unwrap();
    assert!(
        reply["error"].as_str().unwrap().contains("UTF-8"),
        "{reply}"
    );

    // A line over the limit of 1 MiB: refused without being kept, and the
    // request after it is served.
    let mut huge = vec![b'x'; (1 << 20) + 4096];
    huge.push(b'\n');
    client.send(&huge);
    let reply = client.read().unwrap();
    assert_eq!(reply["ok"], false);
    assert!(
        reply["error"]
            .as_str()
            .unwrap()
            .contains("longer than the limit"),
        "{reply}"
    );
    assert_eq!(
        client.execute("document.measure", json!({}))["bodies"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // Connections that go away at every awkward moment.
    drop(rig.connect());
    let mut half = rig.connect();
    half.send(b"{\"id\": 1, \"method\": \"au");
    drop(half);
    let mut mid_request = rig.client();
    mid_request.send(b"{\"id\": 9, \"method\": \"engine.execute\", \"params\": {\"comm");
    drop(mid_request);
    // One that sends a request and does not wait for the reply.
    let mut impatient = rig.client();
    impatient.send(
        b"{\"id\": 9, \"method\": \"engine.execute\", \"params\": {\"command\": \"feature.sphere\", \"params\": {\"radius\": 1}}}\n",
    );
    drop(impatient);
    // One that floods without a newline and leaves.
    let mut flood = rig.client();
    flood.send(&vec![b'y'; 3 << 20]);
    drop(flood);

    // The server is still there, for this connection and for new ones, and
    // gave back every slot.
    eventually("the impatient request ran", || {
        client.execute("document.measure", json!({}))["bodies"]
            .as_array()
            .unwrap()
            .len()
            == 2
    });
    let mut fresh = rig.client();
    assert_eq!(fresh.ok("ui.get", json!({}))["scene"]["bodies"], 2);
    drop(fresh);
    eventually("the dropped connections were noticed", || {
        rig.connections() == 1
    });
    drop(client);
    eventually("every connection is closed", || rig.connections() == 0);
    assert!(rig.finish().session.has_document());
}

#[test]
fn a_panicking_handler_is_reported_and_the_next_request_works() {
    let rig = Rig::start(Workspace::none());
    let mut client = rig.client();
    build_plate_and_post(&mut client);
    let before = client.execute("document.measure", json!({}));

    let reply = client.call("debug.panic", json!({}));
    assert_eq!(reply["ok"], false, "{reply}");
    let error = reply["error"].as_str().unwrap();
    assert!(
        error.contains("internal error") && error.contains("a handler fell over, on purpose"),
        "{error}"
    );
    // The same connection keeps working, on the same document.
    assert_eq!(client.execute("document.measure", json!({})), before);
    client.execute("feature.sphere", json!({"name": "After", "radius": 2}));
    // Several in a row, and from another connection.
    for _ in 0..3 {
        assert_eq!(client.call("debug.panic", json!({}))["ok"], false);
    }
    let mut other = rig.client();
    assert_eq!(other.call("debug.panic", Value::Null)["ok"], false);
    assert_eq!(other.ok("ui.get", json!({}))["scene"]["bodies"], 3);

    // A panic outside the protocol's own guard (here: while the status
    // bar's report is written, after the command ran) is caught by the
    // host's. The reply cannot name the request any more, but it comes.
    let reply = other.line(
        r#"{"id": 77, "method": "engine.execute", "params": {"command": "debug.panic-in-report"}}"#,
    );
    assert_eq!(reply["ok"], false, "{reply}");
    assert_eq!(reply["id"], Value::Null);
    let error = reply["error"].as_str().unwrap();
    assert!(
        error.contains("internal error") && error.contains("document was kept"),
        "{error}"
    );
    assert_eq!(other.ok("ui.get", json!({}))["scene"]["bodies"], 3);
    assert_eq!(
        client.execute("document.measure", json!({}))["bodies"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    let host = rig.finish();
    assert_eq!(host.shell.tree_rows().len(), 3, "the document was kept");
}

#[test]
fn two_connections_drive_one_document() {
    let rig = Rig::start(Workspace::none());
    let (mut a, mut b) = (rig.client(), rig.client());
    a.execute("file.new", json!({}));
    b.execute("feature.block", json!({"name": "FromB", "size": [1, 2, 3]}));
    a.execute("feature.sphere", json!({"name": "FromA", "radius": 1}));
    let seen: Vec<String> = b.ok("ui.get", json!({}))["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(seen, ["FromB", "FromA"]);
    // View state is shared as well: there is one window.
    a.ok("ui.notice", json!({"text": "from a"}));
    assert_eq!(b.ok("ui.get", json!({}))["state"]["notice"], "from a");
    assert_eq!(rig.connections(), 2);
}

#[test]
fn one_connection_over_the_limit_is_told_and_closed() {
    let rig = Rig::with(Workspace::none(), |config| config.max_connections = 3);
    let mut held: Vec<Client> = (0..3).map(|_| rig.client()).collect();
    eventually("three connections are counted", || rig.connections() == 3);
    let mut extra = rig.connect();
    let reply = extra.read().expect("the refusal is sent unasked");
    assert_eq!(reply["ok"], false);
    let error = reply["error"].as_str().unwrap();
    assert!(
        error.contains("too many connections (3 at most)"),
        "{error}"
    );
    assert_eq!(extra.read(), None, "and the connection is closed");
    // The ones that were in keep working.
    for client in &mut held {
        client.ok("ui.get", json!({}));
    }
    // A slot that is given back can be taken again.
    held.pop();
    eventually("a slot is free", || rig.connections() == 2);
    let mut again = rig.client();
    again.ok("ui.get", json!({}));
    assert_eq!(ControlConfig::new(1, String::new()).max_connections, 16);
}

/// A client that speaks first, as the bridge does with its challenge, still
/// reads the refusal of a connection over the limit, and then the end of
/// the connection: the window reads what it sent before closing, so the
/// close is not a reset that could lose the line (2026-11-04).
#[test]
fn a_refused_connection_that_spoke_first_still_reads_why() {
    let rig = Rig::with(Workspace::none(), |config| config.max_connections = 0);
    let mut extra = rig.connect();
    let challenge = json!({"id": 0, "method": "challenge", "params": {"nonce": "0".repeat(32)}});
    // More than one segment's worth, all unread by the window.
    extra.send(format!("{challenge}\n{}\n", " ".repeat(8000)).as_bytes());
    let reply = extra.read().expect("the refusal is read");
    let error = reply["error"].as_str().unwrap();
    assert!(
        error.contains("too many connections (0 at most)"),
        "{error}"
    );
    let mut line = String::new();
    assert_eq!(
        extra.reader.read_line(&mut line).map_err(|e| e.kind()),
        Ok(0),
        "the connection ends cleanly, not with a reset"
    );
}

/// Refused connections are closed beside the listener (review of
/// 2026-11-04): sixteen that never close, each read for up to a quarter of
/// a second, do not hold up a client that comes once a slot is free (in
/// the accept loop they took four seconds before it was taken).
#[test]
fn refusals_do_not_hold_up_the_next_connection() {
    let rig = Rig::with(Workspace::none(), |config| config.max_connections = 1);
    let held = rig.client();
    eventually("one connection is counted", || rig.connections() == 1);
    let mut refused: Vec<Client> = (0..16).map(|_| rig.connect()).collect();
    for client in &mut refused {
        client.send(b"{\"id\":0,\"method\":\"ping\"}\n");
    }
    drop(held);
    eventually("the slot is free", || rig.connections() == 0);
    let started = Instant::now();
    let mut next = rig.client();
    next.ok("ui.get", json!({}));
    let took = started.elapsed();
    assert!(took < Duration::from_secs(2), "{took:?}");
    drop(refused);
}

/// The bridge tells the window's refusal of a connection over its limit as
/// such, not as a window that is not reachable or not the window
/// (2026-11-04).
#[test]
fn the_bridge_tells_a_window_with_too_many_connections() {
    use craft3d_automation::bridge::{self, BridgeConfig};
    let rig = Rig::with(Workspace::none(), |config| config.max_connections = 0);
    let message = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
    let mut output = Vec::new();
    bridge::serve(
        &BridgeConfig::new(rig.address, TOKEN.to_string()),
        format!("{message}\n").as_bytes(),
        &mut output,
    )
    .unwrap();
    let reply: Value = serde_json::from_slice(&output).unwrap();
    let error = reply["error"]["message"].as_str().unwrap();
    assert!(
        error.contains(&format!(
            "the 3DCraft window at {} refused the connection: too many connections (0 at most)",
            rig.address
        )),
        "{error}"
    );
    assert!(!error.contains("not reachable"), "{error}");
    assert!(!error.contains("is not the 3DCraft window"), "{error}");
}

#[test]
fn an_idle_connection_is_closed_and_a_busy_one_is_not() {
    let rig = Rig::with(Workspace::none(), |config| {
        config.idle_timeout = Duration::from_millis(400);
    });
    let mut busy = rig.client();
    let mut idle = rig.client();
    let silent = rig.connect();
    for _ in 0..8 {
        std::thread::sleep(Duration::from_millis(100));
        busy.ok("ui.get", json!({}));
    }
    assert_eq!(idle.read(), None, "the idle connection was closed");
    eventually("only the busy connection is left", || {
        rig.connections() == 1
    });
    drop(silent);
    busy.ok("ui.get", json!({}));
    let default = ControlConfig::new(1, String::new());
    assert_eq!(default.idle_timeout, Duration::from_secs(30));
}

#[test]
fn a_request_nobody_answers_times_out_with_an_error() {
    // No thread pumps this channel: the window is not answering.
    let (requests, incoming) = mpsc::channel::<Request>();
    let mut config = ControlConfig::new(0, TOKEN.to_string());
    config.reply_timeout = Duration::from_millis(300);
    let wake: Wake = Arc::new(|| {});
    let server = ControlServer::start(config, requests, wake).unwrap();
    let mut client = Client::connect(server.address());
    client.call("auth", json!({"token": TOKEN}));
    let reply = client.line(r#"{"id": 1, "method": "ui.get"}"#);
    assert_eq!(reply["ok"], false);
    assert!(
        reply["error"].as_str().unwrap().contains("did not answer"),
        "{reply}"
    );
    // The request did arrive, unparsed, for the session's thread.
    let waiting = incoming.try_recv().unwrap();
    assert_eq!(waiting.line, r#"{"id": 1, "method": "ui.get"}"#);
    // Once the application is gone, a request says so at once.
    drop(waiting);
    drop(incoming);
    let reply = client.line(r#"{"id": 2, "method": "ui.get"}"#);
    assert!(
        reply["error"].as_str().unwrap().contains("closing"),
        "{reply}"
    );
}

#[test]
fn ui_quit_reaches_the_shell_and_the_listener_stops_with_the_server() {
    let rig = Rig::start(Workspace::none());
    let address = rig.address;
    let mut client = rig.client();
    assert_eq!(client.ok("ui.quit", json!({})), json!({"quitting": true}));
    drop(client);
    let host = rig.finish();
    assert!(host.shell.quit_requested());
    // The listener is gone with the server: nothing accepts any more.
    let refused = TcpStream::connect_timeout(&address, Duration::from_secs(2));
    assert!(refused.is_err(), "the port is still open");
}

#[test]
fn the_listener_binds_loopback_only_and_refuses_a_taken_port_or_no_token() {
    let (requests, _incoming) = mpsc::channel::<Request>();
    let wake: Wake = Arc::new(|| {});
    let server = ControlServer::start(
        ControlConfig::new(0, TOKEN.to_string()),
        requests.clone(),
        Arc::clone(&wake),
    )
    .unwrap();
    assert!(server.address().ip().is_loopback());
    assert_ne!(server.address().port(), 0);
    let taken = ControlServer::start(
        ControlConfig::new(server.address().port(), TOKEN.to_string()),
        requests.clone(),
        Arc::clone(&wake),
    )
    .err()
    .unwrap();
    assert!(taken.contains("cannot listen on 127.0.0.1"), "{taken}");
    let no_token = ControlServer::start(ControlConfig::new(0, String::new()), requests, wake)
        .err()
        .unwrap();
    assert!(no_token.contains("token"), "{no_token}");
}

/// A panic the engine's own guard does not see (here: while the status
/// bar's report is written, after the command ran) is caught by the
/// host's, and then the last good state is autosaved at once: whatever
/// else is wrong, the work so far is on disk.
#[test]
fn after_a_caught_panic_the_document_is_autosaved_at_once_and_can_be_recovered() {
    let dir = workdir("panic-autosave");
    let recovery = dir.join("recovery");
    let host_on = |recovery: &std::path::Path| {
        let mut session = Session::new(Workspace::none());
        // A clock that stands still: nothing is ever due by itself.
        session.enable_recovery(craft3d_engine::RecoveryOptions::manual(
            recovery,
            Arc::new(craft3d_engine::ManualClock::new(1_800_000_000)),
        ));
        Host::new(session)
    };
    let ask = |host: &mut Host, command: &str, params: Value| -> Value {
        let line = json!({"id": 1, "method": "engine.execute",
                          "params": {"command": command, "params": params}})
        .to_string();
        serde_json::from_str(&host.handle_line(&line).unwrap()).unwrap()
    };
    let mut host = host_on(&recovery);
    for (command, params) in [
        ("file.new", json!({})),
        ("feature.block", json!({"size": [10, 20, 30]})),
        ("feature.sphere", json!({"radius": 4})),
    ] {
        assert_eq!(ask(&mut host, command, params)["ok"], true);
    }
    let before = ask(&mut host, "document.measure", json!({}));
    assert!(!recovery.exists(), "nothing was due");

    let reply = ask(&mut host, "debug.panic-in-report", json!({}));
    assert_eq!(reply["ok"], false, "{reply}");
    assert!(
        reply["error"]
            .as_str()
            .unwrap()
            .contains("document was kept"),
        "{reply}"
    );
    let overview = host.session.recovery_overview();
    assert_eq!((overview.writes, overview.pending), (1, 0), "{overview:?}");
    assert_eq!(overview.problem, None);
    // The session goes on.
    assert_eq!(ask(&mut host, "document.measure", json!({})), before);
    // And if it had not: what the next start finds is that document.
    drop(host);
    let mut next = host_on(&recovery);
    let listed = ask(&mut next, "recovery.list", json!({}));
    let id = listed["result"]["documents"][0]["id"].clone();
    assert_eq!(listed["result"]["documents"][0]["features"], 2, "{listed}");
    assert_eq!(
        ask(&mut next, "recovery.open", json!({"id": id}))["ok"],
        true
    );
    assert_eq!(ask(&mut next, "document.measure", json!({})), before);
    // The window shows it as an unsaved, recovered document.
    let got = ask(&mut next, "file.list", json!({}));
    assert_eq!(got["result"]["documents"][0]["recovered"], true);
    let _ = std::fs::remove_dir_all(&dir);
}

/// What `app::run` does with the options: the session autosaves into the
/// directory they name, on the engine's thread, and the window is woken
/// when something was written; a clean end leaves nothing behind.
#[test]
fn the_recovery_options_start_autosave_that_wakes_the_window() {
    let dir = workdir("recovery-options");
    let woken = Arc::new(AtomicUsize::new(0));
    let for_wake = Arc::clone(&woken);
    let wake: Wake = Arc::new(move || {
        for_wake.fetch_add(1, Ordering::SeqCst);
    });
    // No directory: no autosave. That is what `Options::default()` is,
    // so no test of the app writes where the person's documents go.
    assert!(
        app::recovery(&Options::default(), Arc::clone(&wake))
            .unwrap()
            .is_none()
    );
    let options = Options {
        recovery_dir: Some(dir.join("recovery")),
        autosave_interval: Some(std::time::Duration::from_millis(20)),
        ..Options::default()
    };
    let mut session = app::session(&options).unwrap();
    assert!(!session.recovery_overview().enabled);
    session.enable_recovery(app::recovery(&options, Arc::clone(&wake)).unwrap().unwrap());
    let overview = session.recovery_overview();
    assert!(overview.enabled);
    assert_eq!(overview.interval_seconds, 0.02);
    // Made absolute as `app::recovery` makes it (on Windows that also
    // takes out the `..` of the test's directory).
    let absolute = std::path::absolute(dir.join("recovery")).unwrap();
    assert_eq!(overview.directory.as_deref(), absolute.to_str());
    session.execute("file.new", &json!({})).unwrap();
    session
        .execute("feature.block", &json!({"size": [1, 2, 3]}))
        .unwrap();
    let waited = std::time::Instant::now();
    while session.recovery_overview().writes == 0 {
        assert!(waited.elapsed() < std::time::Duration::from_secs(20));
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(woken.load(Ordering::SeqCst) >= 1, "the status bar is told");
    assert_eq!(std::fs::read_dir(dir.join("recovery")).unwrap().count(), 3);
    // The window closes: nothing is left to recover.
    session.end_recovery();
    assert_eq!(std::fs::read_dir(dir.join("recovery")).unwrap().count(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_window_pumps_the_same_channel_between_frames() {
    // What `Viewer::logic` does, without the window.
    let (requests, incoming) = mpsc::channel::<Request>();
    let mut host = Host::new(Session::new(Workspace::none()));
    assert_eq!(host.pump(&incoming), 0);
    let mut replies = Vec::new();
    for line in [
        r#"{"id": 1, "method": "engine.execute", "params": {"command": "file.new"}}"#,
        "",
        r#"{"id": 2, "method": "engine.execute", "params": {"command": "feature.sphere", "params": {"radius": 2}}}"#,
    ] {
        let (reply, answer) = mpsc::channel();
        requests
            .send(Request {
                line: line.to_string(),
                reply,
                origin: None,
            })
            .unwrap();
        replies.push(answer);
    }
    assert_eq!(host.pump(&incoming), 3);
    let first: Value = serde_json::from_str(&replies[0].recv().unwrap().unwrap()).unwrap();
    assert_eq!(first["ok"], true);
    assert_eq!(
        replies[1].recv().unwrap(),
        None,
        "a blank line gets no reply"
    );
    assert!(replies[2].recv().unwrap().unwrap().contains("\"ok\":true"));
    assert_eq!(host.shell.tree_rows().len(), 1);
    // A requester that gave up is not an error.
    let (reply, answer) = mpsc::channel();
    drop(answer);
    requests
        .send(Request {
            line: r#"{"id": 3, "method": "ui.get"}"#.to_string(),
            reply,
            origin: None,
        })
        .unwrap();
    assert_eq!(host.pump(&incoming), 1);
    // A flood is answered a bounded number at a time, so frames still
    // get drawn in between.
    for _ in 0..200 {
        let (reply, _) = mpsc::channel();
        requests
            .send(Request {
                line: r#"{"id": 4, "method": "methods"}"#.to_string(),
                reply,
                origin: None,
            })
            .unwrap();
    }
    let first_round = host.pump(&incoming);
    assert!(first_round > 0 && first_round < 200, "{first_round}");
    let mut rest = first_round;
    while rest < 200 {
        let round = host.pump(&incoming);
        assert!(round > 0);
        rest += round;
    }
    assert_eq!(rest, 200);
    // The sender going away is not an error either.
    drop(requests);
    assert_eq!(host.pump(&incoming), 0);
}

#[test]
fn the_file_argument_is_opened_and_automation_gets_only_its_roots() {
    let dir = workdir("session");
    let (shared, private) = (dir.join("shared"), dir.join("private"));
    std::fs::create_dir_all(&shared).unwrap();
    std::fs::create_dir_all(&private).unwrap();
    // A file outside the roots, as a user would name it on the command
    // line.
    let mut author = Session::new(Workspace::unrestricted());
    author.execute("file.new", &json!({})).unwrap();
    author
        .execute("feature.sphere", &json!({"name": "Ball", "radius": 5}))
        .unwrap();
    let file = private.join("ball.3dcraft");
    author
        .execute("file.save", &json!({"path": file.to_str().unwrap()}))
        .unwrap();

    let options = Options {
        file: Some(file.to_str().unwrap().to_string()),
        read_root: Some(shared.to_str().unwrap().to_string()),
        write_root: Some(shared.to_str().unwrap().to_string()),
        ..Options::default()
    };
    let mut session = app::session(&options).unwrap();
    assert!(session.has_document());
    assert_eq!(
        session.execute("document.inspect", &json!({})).unwrap()["features"][0]["name"],
        "Ball"
    );
    // From here on only the roots are reachable: the same file cannot be
    // opened again, nor written back, by a request.
    let again = session.execute("file.open", &json!({"path": file.to_str().unwrap()}));
    assert!(again.is_err());
    assert!(session.execute("file.save", &json!({})).is_err());
    session
        .execute("file.save", &json!({"path": "copy.3dcraft"}))
        .unwrap();
    assert!(shared.join("copy.3dcraft").exists());

    // No roots: no file access at all for requests.
    let bare = app::session(&Options::default()).unwrap();
    assert!(!bare.has_document());
    assert_eq!(*bare.workspace(), Workspace::none());
    // A file that is not there, or not a document, is an error to print.
    for bad in ["no-such-file.3dcraft", shared.to_str().unwrap()] {
        let options = Options {
            file: Some(bad.to_string()),
            ..Options::default()
        };
        assert!(app::session(&options).is_err(), "{bad}");
    }
    // A root that is not a directory.
    for (read_root, write_root) in [
        (Some("no/such/dir".to_string()), None),
        (None, Some(file.to_str().unwrap().to_string())),
    ] {
        let options = Options {
            read_root,
            write_root,
            ..Options::default()
        };
        let error = app::session(&options).err().unwrap();
        assert!(error.contains("--automation-"), "{error}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The file named on the command line by a path relative to the working
/// directory, inside the write root: a request to save it without a path
/// writes that file (2026-10-15: `craft3d target/work/x.3dcraft
/// --automation-write-root target/work` saved to
/// `target/work/target/work/x.3dcraft`, which did not exist), and the
/// listing (where the tabs take their paths from) shows where it is.
#[test]
fn a_file_named_by_a_relative_path_is_saved_back_to_itself() {
    let dir = workdir("relative");
    std::fs::create_dir_all(dir.join("work")).unwrap();
    // Tests run in the package's directory, as `workdir` assumes too.
    let relative = "../../target/tmp/craft3d-control-relative/work/part.3dcraft";
    let mut author = Session::new(Workspace::unrestricted());
    author.execute("file.new", &json!({})).unwrap();
    author
        .execute("feature.sphere", &json!({"name": "Ball", "radius": 5}))
        .unwrap();
    author
        .execute("file.save", &json!({"path": relative}))
        .unwrap();
    let file = craft3d_engine::canonical(&dir.join("work/part.3dcraft")).unwrap();

    let options = Options {
        file: Some(relative.to_string()),
        read_root: Some(dir.to_str().unwrap().to_string()),
        write_root: Some(dir.to_str().unwrap().to_string()),
        ..Options::default()
    };
    let mut session = app::session(&options).unwrap();
    let listed = session.execute("file.list", &json!({})).unwrap();
    assert_eq!(listed["documents"][0]["path"], file.to_str().unwrap());
    session
        .execute("feature.sphere", &json!({"name": "Ball 2", "radius": 2}))
        .unwrap();
    let saved = session.execute("file.save", &json!({})).unwrap();
    assert_eq!(saved["path"], file.to_str().unwrap());
    let mut check = Session::new(Workspace::unrestricted());
    let reopened = check
        .execute("file.open", &json!({"path": file.to_str().unwrap()}))
        .unwrap();
    assert_eq!(reopened["features"], 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_control_option_starts_a_listener_with_a_fresh_token_file() {
    let dir = workdir("option");
    let wake: Wake = Arc::new(|| {});
    let none = |_: &str| None;
    let (agents, _incoming) = app::control(&Options::default(), Arc::clone(&wake), &none).unwrap();
    assert!(agents.address().is_none(), "no --control, no listener");

    let token_file = dir.join("control.token");
    let options = Options {
        control: Some(0),
        token_file: Some(token_file.clone()),
        ..Options::default()
    };
    let (agents, incoming) = app::control(&options, Arc::clone(&wake), &none).unwrap();
    let address = agents.address().unwrap();
    let token = std::fs::read_to_string(&token_file).unwrap();
    assert_eq!(token.trim().len(), 64);
    // A client that read the file gets in; the request reaches the channel.
    let mut client = Client::connect(address);
    let reply = client.call("auth", json!({"token": token.trim()}));
    assert_eq!(reply["ok"], true, "{reply}");
    client.send(b"{\"id\": 5, \"method\": \"ui.get\"}\n");
    let request = incoming.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(request.line.contains("ui.get"));
    request
        .reply
        .send(Some("{\"id\":5,\"ok\":true,\"result\":{}}".to_string()))
        .unwrap();
    assert_eq!(client.read().unwrap()["id"], 5);

    // The port is taken now: starting a second app on it is an error.
    let second = Options {
        control: Some(address.port()),
        token_file: Some(token_file.clone()),
        ..Options::default()
    };
    let error = app::control(&second, Arc::clone(&wake), &none)
        .err()
        .unwrap();
    assert!(error.contains("cannot listen"), "{error}");
    // A token file that holds something else stops the start.
    std::fs::write(&token_file, "not a token").unwrap();
    let error = app::control(&options, wake, &none).err().unwrap();
    assert!(error.contains("does not hold a control token"), "{error}");
    let _ = std::fs::remove_dir_all(&dir);
}

// ------------------------------------------------------------ MCP bridge

/// Runs MCP messages through the bridge against the rig's listener, as
/// `craft3d-cli mcp --bridge` does, and returns the replies it wrote.
fn bridged(address: SocketAddr, token: &str, messages: &[Value]) -> Vec<Value> {
    use craft3d_automation::bridge::{self, BridgeConfig};
    let input: String = messages.iter().map(|m| format!("{m}\n")).collect();
    let mut output = Vec::new();
    bridge::serve(
        &BridgeConfig::new(address, token.to_string()),
        input.as_bytes(),
        &mut output,
    )
    .unwrap();
    String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).expect("a reply is one line of JSON"))
        .collect()
}

fn tool_call(id: u64, tool: &str, arguments: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
           "params": {"name": tool, "arguments": arguments}})
}

/// The JSON a tool returned as its text block.
fn tool_json(reply: &Value) -> Value {
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    let text = reply["result"]["content"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|block| block["text"].as_str())
        .unwrap_or_else(|| panic!("no text in {reply}"));
    serde_json::from_str(text).unwrap()
}

#[test]
fn an_mcp_client_models_in_the_windows_own_document_through_the_bridge() {
    let rig = Rig::start(Workspace::none());
    let replies = bridged(
        rig.address,
        TOKEN,
        &[
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
                   "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                              "clientInfo": {"name": "test", "version": "0"}}}),
            // A notification: no reply, and no line on the output.
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
            tool_call(
                3,
                "command_run",
                json!({"id": "file.new", "params": {"units": "mm"}}),
            ),
            tool_call(
                4,
                "command_run",
                json!({"id": "feature.block", "params": {"name": "Plate", "size": [60, 40, 8]}}),
            ),
            tool_call(
                5,
                "command_run",
                json!({"id": "feature.blok", "params": {}}),
            ),
            tool_call(6, "doc_inspect", json!({})),
            tool_call(7, "doc_render_preview", json!({"max_side": 64})),
        ],
    );
    assert_eq!(
        replies.iter().map(|r| r["id"].clone()).collect::<Vec<_>>(),
        [1, 2, 3, 4, 5, 6, 7].map(Value::from),
        "one reply per request, in order, none for the notification"
    );
    assert_eq!(replies[0]["result"]["protocolVersion"], "2025-06-18");
    let tools: Vec<&str> = replies[1]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(
        tools.contains(&"command_run") && tools.contains(&"doc_render_preview"),
        "{tools:?}"
    );
    assert_eq!(tool_json(&replies[3])["name"], "Plate");
    // A command that fails is a tool result marked as an error, with the
    // engine's own message.
    assert_eq!(replies[4]["result"]["isError"], true, "{}", replies[4]);
    assert!(
        replies[4]["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("did you mean `feature.block`"),
        "{}",
        replies[4]
    );
    // 60 x 40 x 8, to the rounding of the integration over the faces.
    let close = |volume: &Value| (volume.as_f64().unwrap() - 19200.0).abs() <= 1e-9 * 19200.0;
    assert!(close(&tool_json(&replies[5])["bodies"][0]["volume"]));
    assert_eq!(replies[6]["result"]["content"][0]["type"], "image");

    // The same document is the one the control channel sees ...
    let mut client = rig.client();
    let measured = client.execute("document.measure", json!({}));
    assert!(close(&measured["volume"]), "{measured}");
    // ... and the one the window owns.
    let mut host = rig.finish();
    let inspected = host
        .session
        .execute("document.inspect", &Value::Null)
        .unwrap();
    assert_eq!(inspected["features"][0]["name"], "Plate");
}

/// Pointing over both channels (roadmap M8.5): what is at a pixel of the
/// viewport, what was clicked, a face to highlight, for a client of the
/// control channel and for an MCP client through the bridge.
#[test]
fn an_agent_points_at_a_face_over_the_control_channel_and_over_mcp() {
    let rig = Rig::start(Workspace::none());
    let mut client = rig.client();
    build_plate_and_post(&mut client);
    client.ok("ui.fit", json!({"view": "top"}));
    client.ok("ui.set", json!({"camera": {"projection": "orthographic"}}));
    let got = client.ok("ui.get", json!({}));
    let (width, height) = (
        got["viewport"]["width"].as_f64().unwrap(),
        got["viewport"]["height"].as_f64().unwrap(),
    );
    assert_eq!(got["state"]["picked"], Value::Null);
    assert_eq!(got["state"]["highlight"], Value::Null);
    // Seen from the top, framed about the model's centre: the middle of
    // the viewport is the top of the plate, at z = 8.
    let middle = json!({"x": width / 2.0, "y": height / 2.0});
    let picked = client.ok("ui.pick", middle.clone());
    assert_eq!(picked["hit"], true, "{picked}");
    assert_eq!(picked["name"], "Plate");
    assert!(
        (picked["point"][2].as_f64().unwrap() - 8.0).abs() < 1e-9,
        "{picked}"
    );
    let inspect = client.execute("document.inspect", json!({}));
    let names = inspect["bodies"][0]["faceNames"].as_array().unwrap();
    let face = picked["face"].clone();
    assert_eq!(names[picked["faceIndex"].as_u64().unwrap() as usize], face);
    // The command gives the same answer for a camera of its own.
    let own = client.execute(
        "view.pick",
        json!({"x": 100, "y": 100, "width": 200, "height": 200,
               "camera": {"eye": [30, 20, 500], "target": [30, 20, 0], "up": [0, 1, 0],
                          "projection": "orthographic", "height": 100}}),
    );
    assert_eq!(own["face"], face);

    // The same question through the MCP bridge, then a click and a
    // highlight from there.
    let replies = bridged(
        rig.address,
        TOKEN,
        &[
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
            tool_call(
                2,
                "control_call",
                json!({"method": "ui.pick", "params": middle}),
            ),
            tool_call(
                3,
                "control_call",
                json!({"method": "ui.pick",
                       "params": {"x": width / 2.0, "y": height / 2.0, "select": true}}),
            ),
            tool_call(4, "control_call", json!({"method": "ui.get"})),
            tool_call(
                5,
                "control_call",
                json!({"method": "ui.highlight", "params": {"face": null}}),
            ),
            tool_call(
                6,
                "control_call",
                json!({"method": "ui.highlight", "params": {"face": face, "feature": "Plate"}}),
            ),
            tool_call(
                7,
                "control_call",
                json!({"method": "ui.screenshot", "params": {"width": 160, "height": 120}}),
            ),
            tool_call(
                8,
                "control_call",
                json!({"method": "ui.highlight", "params": {"face": "F9:face(0)"}}),
            ),
            tool_call(9, "control_call", json!({"method": "ui.quit"})),
            tool_call(
                10,
                "control_call",
                json!({"method": "mcp.message", "params": {}}),
            ),
            tool_call(11, "control_call", json!({"params": {}})),
            tool_call(
                12,
                "control_call",
                json!({"method": "ui.get", "params": [1]}),
            ),
            tool_call(
                13,
                "command_run",
                json!({"id": "view.pick", "params": {"x": 5, "y": 5, "width": 64, "height": 64}}),
            ),
        ],
    );
    assert_eq!(replies.len(), 13);
    let tools = replies[0]["result"]["tools"].as_array().unwrap();
    let control = tools.iter().find(|t| t["name"] == "control_call").unwrap();
    let description = control["description"].as_str().unwrap();
    for method in ["ui.get", "ui.pick", "ui.highlight", "ui.screenshot"] {
        assert!(description.contains(method), "{method} is not described");
    }
    assert!(!description.contains("ui.quit"));
    assert_eq!(tool_json(&replies[1]), picked);
    let clicked = tool_json(&replies[2]);
    assert_eq!(clicked["picked"]["face"], face);
    assert_eq!(clicked["picked"]["serial"], 1);
    let state = tool_json(&replies[3]);
    assert_eq!(state["state"]["picked"]["face"], face);
    assert_eq!(state["state"]["highlight"]["face"], face);
    assert_eq!(tool_json(&replies[4])["highlight"], Value::Null);
    let shown = tool_json(&replies[5]);
    assert_eq!(shown["highlight"], json!({"feature": "F1", "face": face}));
    assert_eq!(shown["faces"], 1);
    // The picture is a picture, and the plate's top in it is highlighted:
    // the middle pixel has the hue of the token, not the body's grey.
    let blocks = replies[6]["result"]["content"].as_array().unwrap();
    assert_eq!(blocks[0]["type"], "image");
    let (w, h, rgb) = decode(&unbase64(blocks[0]["data"].as_str().unwrap()));
    assert_eq!((w, h), (160, 120));
    let at = ((60 * 160 + 80) * 3) as usize;
    let (r, g, b) = (
        i32::from(rgb[at]),
        i32::from(rgb[at + 1]),
        i32::from(rgb[at + 2]),
    );
    // The highlight's copper (Mission, 2026-10-21; it was cyan): red over
    // green over blue, saturated, where the body is a warm grey.
    assert!(
        r > g + 30 && g > b + 30,
        "({r}, {g}, {b}) is not the highlight"
    );
    assert!(tool_json(&replies[6]).get("base64").is_none());
    // A face nobody has: the tool's result says so, as an error.
    assert_eq!(replies[7]["result"]["isError"], true, "{}", replies[7]);
    assert!(
        replies[7]["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("no body has a face"),
        "{}",
        replies[7]
    );
    // What is not offered is not reachable, and wrong arguments are
    // refused before anything runs.
    for (reply, part) in [
        (&replies[8], "unknown method `ui.quit`"),
        (&replies[9], "unknown method `mcp.message`"),
        (&replies[10], "method"),
        (&replies[11], "params"),
    ] {
        assert_eq!(reply["error"]["code"], -32602, "{reply}");
        assert!(
            reply["error"]["message"].as_str().unwrap().contains(part),
            "{reply}"
        );
    }
    // The command is there over MCP too, for a client without a window.
    assert_eq!(tool_json(&replies[12])["hit"], false);

    // The control channel sees what MCP did, and the window is still open.
    let got = client.ok("ui.get", json!({}));
    assert_eq!(got["state"]["highlight"]["face"], face);
    let host = rig.finish();
    assert!(!host.shell.quit_requested());
    assert_eq!(host.shell.state.picked.as_ref().unwrap().serial, 1);
}

#[test]
fn the_status_bar_reports_a_command_that_came_over_mcp() {
    let rig = Rig::start(Workspace::none());
    bridged(
        rig.address,
        TOKEN,
        &[
            tool_call(1, "command_run", json!({"id": "file.new", "params": {}})),
            tool_call(
                2,
                "command_run",
                json!({"id": "feature.sphere", "params": {"radius": -1}}),
            ),
        ],
    );
    let host = rig.finish();
    let last = host.shell.last_command.as_ref().unwrap();
    assert_eq!(last.label, "feature.sphere");
    // The sphere is stored and failed: the tool call itself worked.
    assert!(last.ok && last.text.contains("failed"), "{}", last.text);
}

#[test]
fn the_bridge_tells_the_agent_when_the_window_cannot_be_used() {
    let rig = Rig::start(Workspace::none());
    let request = json!({"jsonrpc": "2.0", "id": "a", "method": "tools/list"});
    // The wrong token: refused by the window, reported to the agent.
    let wrong = "0".repeat(64);
    let replies = bridged(rig.address, &wrong, std::slice::from_ref(&request));
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0]["id"], "a");
    let message = replies[0]["error"]["message"].as_str().unwrap();
    // Since 2026-11-02 the window proves it holds the token first: with
    // another token its proof does not match, and nothing is sent.
    assert!(
        message.contains("holds this token") && message.contains("nothing was sent"),
        "{message}"
    );
    // Nothing reached the session's thread.
    assert_eq!(rig.dispatched.load(Ordering::SeqCst), 0);

    // The window is gone: `tools/list` is answered by the bridge itself
    // (since 2026-11-04, so an MCP client keeps the server), a tool call
    // gets an error that says how to start the window, and notifications
    // stay unanswered.
    let address = rig.address;
    drop(rig.finish());
    let replies = bridged(
        address,
        TOKEN,
        &[
            request.clone(),
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            json!({"jsonrpc": "2.0", "id": 9, "method": "tools/call", "params": {}}),
        ],
    );
    assert_eq!(replies.len(), 2, "{replies:?}");
    assert_eq!(replies[0]["id"], "a");
    assert!(
        replies[0]["result"]["tools"]
            .as_array()
            .is_some_and(|tools| !tools.is_empty()),
        "{}",
        replies[0]
    );
    for (reply, id) in replies.iter().skip(1).zip([json!(9)]) {
        assert_eq!(reply["id"], id);
        assert_eq!(reply["error"]["code"], -32000);
        let message = reply["error"]["message"].as_str().unwrap();
        assert!(
            message.contains("not reachable")
                && message.contains(&format!("craft3d --control {}", address.port())),
            "{message}"
        );
    }
}

#[test]
fn mcp_message_takes_one_line_and_hostile_lines_do_no_harm() {
    let rig = Rig::start(Workspace::none());
    let mut client = rig.client();
    for params in [
        json!({}),
        json!({"line": 7}),
        json!({"line": "{}", "extra": 1}),
        json!([]),
    ] {
        let message = client.error("mcp.message", params);
        assert!(message.contains("`line`"), "{message}");
    }
    // Lines that are not MCP are answered by the MCP handler itself, as
    // JSON-RPC errors inside a successful mcp.message.
    for line in [
        "",
        "   ",
        "not json",
        "[]",
        "7",
        "{\"jsonrpc\":\"2.0\",\"id\":1}",
    ] {
        let result = client.ok("mcp.message", json!({"line": line}));
        match &result["reply"] {
            Value::Null => assert!(line.trim().is_empty(), "`{line}` got no reply"),
            Value::String(reply) => {
                let reply: Value = serde_json::from_str(reply).unwrap();
                assert!(reply["error"]["code"].is_i64(), "`{line}`: {reply}");
            }
            other => panic!("`{line}`: {other}"),
        }
    }
    // Through the bridge: garbage in, one JSON-RPC error out per line, and
    // the bridge reads on.
    let mut input = b"not json\n\xff\xfe\n".to_vec();
    input.extend(format!("{}\n", "x".repeat(2 << 20)).as_bytes());
    input.extend(b"{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"ping\"}\n");
    let mut output = Vec::new();
    craft3d_automation::bridge::serve(
        &craft3d_automation::bridge::BridgeConfig::new(rig.address, TOKEN.to_string()),
        input.as_slice(),
        &mut output,
    )
    .unwrap();
    let replies: Vec<Value> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(replies.len(), 4, "{replies:?}");
    for reply in &replies[..3] {
        assert!(reply["error"]["code"].is_i64(), "{reply}");
    }
    assert_eq!(replies[3], json!({"jsonrpc": "2.0", "id": 5, "result": {}}));
}

/// The window icon is the installers' PNG, decoded to RGBA (packaging/icons).
#[test]
fn the_window_icon_decodes() {
    let icon = app::window_icon().unwrap();
    assert_eq!((icon.width, icon.height), (64, 64));
    assert_eq!(icon.rgba.len(), 64 * 64 * 4);
    // Opaque in the middle, transparent in the corner (a rounded square).
    assert_eq!(icon.rgba[(32 * 64 + 32) * 4 + 3], 255);
    assert_eq!(icon.rgba[3], 0);
}

/// The window opens decorated by the system, opaque and resizable, with
/// the title, app id and icon of the installers.
#[test]
fn the_window_has_system_decorations() {
    let viewport = app::viewport(&Options::default(), "untitled – 3DCraft");
    assert_eq!(viewport.decorations, Some(true));
    assert_eq!(viewport.transparent, Some(false));
    assert_eq!(viewport.resizable, Some(true));
    assert_ne!(viewport.fullsize_content_view, Some(true));
    assert_ne!(viewport.titlebar_shown, Some(false));
    assert_ne!(viewport.title_shown, Some(false));
    assert_eq!(viewport.title.as_deref(), Some("untitled – 3DCraft"));
    assert_eq!(viewport.app_id.as_deref(), Some(app::APP_ID));
    assert!(viewport.icon.is_some());
}

// The window with an engine thread: commands run there, and the thread
// that draws keeps turning.

mod engine_thread {
    use super::*;
    use crate::app::Desk;
    use crate::files::{Picker, SystemDialogs};

    /// A desk with an engine thread, served on a thread of its own that
    /// turns it like frames do (every few milliseconds), counting the
    /// turns and how long the longest one took.
    struct Window {
        address: SocketAddr,
        stop: Arc<AtomicBool>,
        turns: Arc<AtomicUsize>,
        longest: Arc<std::sync::Mutex<Duration>>,
        quit: Arc<AtomicBool>,
        frames: Option<JoinHandle<()>>,
        _server: ControlServer,
    }

    impl Window {
        fn open() -> Self {
            Self::open_with(Duration::ZERO)
        }

        /// A window whose refinements each take `refining` longer.
        fn open_with(refining: Duration) -> Self {
            let (requests, incoming) = mpsc::channel::<Request>();
            let wake: Wake = Arc::new(|| {});
            let server = ControlServer::start(
                ControlConfig::new(0, TOKEN.to_string()),
                requests,
                Arc::clone(&wake),
            )
            .unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let turns = Arc::new(AtomicUsize::new(0));
            let longest = Arc::new(std::sync::Mutex::new(Duration::ZERO));
            let quit = Arc::new(AtomicBool::new(false));
            let (for_stop, for_turns, for_longest, for_quit) = (
                Arc::clone(&stop),
                Arc::clone(&turns),
                Arc::clone(&longest),
                Arc::clone(&quit),
            );
            let frames = std::thread::spawn(move || {
                let picker = Picker::new(Arc::new(SystemDialogs), Arc::new(|| {}));
                let host = Host::new(Session::new(Workspace::none()));
                let mut desk = Desk::new(host, incoming, picker)
                    .with_engine_thread(wake)
                    .with_slow_refinement(refining);
                while !for_stop.load(Ordering::SeqCst) {
                    let start = Instant::now();
                    let turn = desk.turn();
                    let took = start.elapsed();
                    let mut longest = for_longest.lock().unwrap();
                    *longest = (*longest).max(took);
                    drop(longest);
                    for_turns.fetch_add(1, Ordering::SeqCst);
                    if turn.quit {
                        for_quit.store(true, Ordering::SeqCst);
                        // Closing: the work still running gets its moment.
                        assert!(desk.settle(Duration::from_secs(10)));
                        assert!(!desk.away());
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(4));
                }
            });
            Self {
                address: server.address(),
                stop,
                turns,
                longest,
                quit,
                frames: Some(frames),
                _server: server,
            }
        }

        fn client(&self) -> Client {
            let mut client = Client::connect(self.address);
            let reply = client.call("auth", json!({"token": TOKEN}));
            assert_eq!(reply["result"], json!({"authenticated": true}), "{reply}");
            client
        }
    }

    impl Drop for Window {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(frames) = self.frames.take() {
                let _ = frames.join();
            }
        }
    }

    /// Waits until `ready` holds, for at most ten seconds.
    fn until(mut ready: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ready() {
            assert!(Instant::now() < deadline, "timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// The developer's report: opening a large document froze the window
    /// for seconds. A command that takes its time runs on the engine
    /// thread; the turns go on meanwhile, each one short, and `ui.get`
    /// says what is being done.
    #[test]
    fn a_long_command_does_not_hold_up_the_window_and_says_what_it_does() {
        let window = Window::open();
        let mut agent = window.client();
        agent.execute("file.new", json!({}));
        let address = window.address;
        let slow = std::thread::spawn(move || {
            let mut client = Client::connect(address);
            client.call("auth", json!({"token": TOKEN}));
            let started = Instant::now();
            let result = client.ok("debug.slow", json!({"ms": 1200, "steps": 6}));
            (result, started.elapsed())
        });
        // Wait until the work shows, then look at it while it runs.
        let mut seen = Value::Null;
        until(|| {
            let state = agent.ok("ui.get", json!({}));
            let busy = state["work"]["phase"] == "regenerating";
            if busy {
                seen = state;
            }
            busy
        });
        let turns_before = window.turns.load(Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(300));
        let turns_during = window.turns.load(Ordering::SeqCst) - turns_before;
        let work = &seen["work"];
        assert_eq!(work["label"], "debug.slow", "{seen}");
        assert!(work["step"].as_u64().unwrap() >= 1, "{seen}");
        assert_eq!(work["steps"], 6, "{seen}");
        assert!(work["name"].as_str().unwrap().starts_with("Slow"), "{seen}");
        // The document shown is the one before the command: whole.
        assert_eq!(seen["document"]["open"], 1, "{seen}");
        let (result, took) = slow.join().unwrap();
        assert_eq!(result["slept"], 1200);
        assert!(took >= Duration::from_millis(1100), "{took:?}");
        // Turns came all along, and none of them waited for the command
        // (only a moment, `PATIENCE`). A window held up by the command
        // turns at most once in the 300 ms; a free one every few
        // milliseconds, fewer on a slow machine (18 on a hosted macOS
        // runner of 3 virtual cores, 2026-10-30), so five tells them apart.
        assert!(turns_during >= 5, "{turns_during} turns in 300 ms");
        let longest = *window.longest.lock().unwrap();
        assert!(
            longest < Duration::from_millis(250),
            "a turn took {longest:?}"
        );
        // Idle again.
        let state = agent.ok("ui.get", json!({}));
        assert_eq!(state["work"], Value::Null, "{state}");
        assert_eq!(state["lastCommand"]["label"], "file.new", "{state}");
    }

    /// Commands stay in the order they were sent, whichever connection
    /// sent them: a second waits for the first. Only `ui.get` (and
    /// `ui.quit`) is answered while work runs.
    #[test]
    fn replies_keep_the_order_of_the_requests_across_connections() {
        let window = Window::open();
        let order = Arc::new(std::sync::Mutex::new(Vec::new()));
        let start = std::time::Instant::now();
        let spawn = |name: &'static str, delay: u64, method: &'static str, params: Value| {
            let (address, order) = (window.address, Arc::clone(&order));
            std::thread::spawn(move || {
                let mut client = Client::connect(address);
                client.call("auth", json!({"token": TOKEN}));
                std::thread::sleep(Duration::from_millis(delay));
                let reply = client.call(method, params);
                order.lock().unwrap().push((name, start.elapsed()));
                reply
            })
        };
        let first = spawn("slow", 0, "debug.slow", json!({"ms": 900, "steps": 3}));
        let second = spawn(
            "batch",
            200,
            "batch",
            json!({"steps": [
                {"command": "file.new"},
                {"command": "feature.block", "params": {"size": [1, 2, 3]}},
            ]}),
        );
        let third = spawn("ui.fit", 300, "ui.fit", json!({}));
        let look = spawn("ui.get", 400, "ui.get", json!({}));
        let looked = look.join().unwrap();
        assert_eq!(looked["result"]["work"]["label"], "debug.slow", "{looked}");
        // Before the file.new that was sent before it ran: no document.
        assert_eq!(looked["result"]["document"], Value::Null, "{looked}");
        assert_eq!(first.join().unwrap()["ok"], true);
        assert_eq!(second.join().unwrap()["ok"], true);
        // Framing waited for the block the batch sent before it made:
        // there was nothing to frame when it was sent.
        let fit = third.join().unwrap();
        assert_eq!(fit["ok"], true, "{fit}");
        // `ui.get` is answered while the slow command runs, so first. The
        // batch, sent 200 ms in, waited for the slow command: its reply
        // came no earlier than the slow command's 900 ms. (Which client
        // thread wakes first once both replies are sent is up to the
        // scheduler, so the slow and the batch replies are not compared
        // by the order the threads note them in.)
        let order = order.lock().unwrap();
        assert_eq!(order.first().map(|o| o.0), Some("ui.get"), "{order:?}");
        let batch = order.iter().find(|o| o.0 == "batch").map(|o| o.1);
        assert!(
            batch.is_some_and(|at| at >= Duration::from_millis(900)),
            "{order:?}"
        );
    }

    /// `ui.quit` while work runs: answered at once, the loop stops, and
    /// the session comes back before the application ends.
    #[test]
    fn quitting_while_work_runs_is_answered_at_once_and_waits_for_the_session() {
        let window = Window::open();
        let address = window.address;
        let slow = std::thread::spawn(move || {
            let mut client = Client::connect(address);
            client.call("auth", json!({"token": TOKEN}));
            client.call("debug.slow", json!({"ms": 800, "steps": 4}))
        });
        let mut agent = window.client();
        until(|| agent.ok("ui.get", json!({}))["work"] != Value::Null);
        let asked = Instant::now();
        let quit = agent.ok("ui.quit", json!({}));
        assert_eq!(quit["quitting"], true);
        assert!(asked.elapsed() < Duration::from_millis(400));
        until(|| window.quit.load(Ordering::SeqCst));
        // The work that was running finished and was answered.
        assert_eq!(slow.join().unwrap()["ok"], true);
    }

    /// A slow command over MCP runs on the engine thread too; listing the
    /// tools does not need to.
    /// Coarse first, then sharp: after an edit the window shows the quick
    /// first picture at once and says it is refining; a command that
    /// comes meanwhile is answered without waiting for the refinement; a
    /// screenshot waits for it and shows the display scene.
    #[test]
    fn the_first_picture_is_refined_while_commands_go_on() {
        let window = Window::open_with(Duration::from_millis(1500));
        let mut agent = window.client();
        agent.execute("file.new", json!({}));
        agent.execute("feature.sphere", json!({"radius": 20}));
        let state = agent.ok("ui.get", json!({}));
        let first = state["scene"].clone();
        assert_eq!(first["refining"], true, "{state}");
        // Answered while the refinement runs.
        let started = Instant::now();
        let measured = agent.execute("document.measure", json!({}));
        assert!(
            started.elapsed() < Duration::from_millis(1000),
            "{:?}",
            started.elapsed()
        );
        assert!(measured["area"].as_f64().unwrap() > 5000.0, "{measured}");
        let state = agent.ok("ui.get", json!({}));
        assert_eq!(state["scene"]["refining"], true, "{state}");
        // The screenshot waits for the display scene.
        let shot = agent.ok("ui.screenshot", json!({"width": 64, "height": 48}));
        assert!(shot["base64"].is_string(), "{shot}");
        let state = agent.ok("ui.get", json!({}));
        let sharp = &state["scene"];
        assert_eq!(shot["triangles"], sharp["triangles"], "{state}");
        assert_eq!(sharp["refining"], false, "{state}");
        assert_eq!(sharp["revision"], first["revision"], "{state}");
        assert!(
            sharp["tolerance"].as_f64().unwrap() < first["tolerance"].as_f64().unwrap() / 4.0,
            "{first} then {sharp}"
        );
        assert!(sharp["triangles"].as_u64().unwrap() > first["triangles"].as_u64().unwrap());
    }

    /// Switching back to a document whose scenes were built shows them
    /// at once: no first picture, nothing to refine.
    #[test]
    fn switching_tabs_shows_the_display_scene_at_once() {
        let window = Window::open();
        let mut agent = window.client();
        for radius in [5, 6, 7, 8] {
            agent.execute("file.new", json!({}));
            agent.execute("feature.sphere", json!({ "radius": radius }));
            until(|| agent.ok("ui.get", json!({}))["scene"]["refining"] == false);
        }
        for document in [0, 2, 1, 3, 0] {
            agent.execute("file.activate", json!({ "document": document }));
            let state = agent.ok("ui.get", json!({}));
            assert_eq!(state["scene"]["refining"], false, "{state}");
            assert_eq!(state["work"], Value::Null, "{state}");
        }
    }

    /// The coordinator's demo (2026-11-04): a script that made a new
    /// document with `file.new` over one connection and built in it over
    /// the next had its commands land in another document ("no feature
    /// named B0"). The document `file.new` and `file.open` make is the one
    /// the next request works on, whichever connection sends it.
    #[test]
    fn a_new_document_is_the_one_the_next_request_works_on() {
        let window = Window::open();
        let named = |client: &str| {
            let mut c = Client::connect(window.address);
            let reply = c.call("auth", json!({"token": TOKEN, "client": client}));
            assert_eq!(reply["ok"], true, "{reply}");
            c
        };
        for round in 0..4_u64 {
            let made = {
                let mut agent = named("demo");
                agent.execute("file.new", json!({"units": "mm"}))
            };
            assert_eq!(made["document"], round, "{made}");
            let mut agent = named("demo");
            agent.execute(
                "feature.block",
                json!({"name": "B0", "size": ["10 mm", "10 mm", "10 mm"]}),
            );
            let list = agent.execute("file.list", json!({}));
            assert_eq!(list["active"], round, "{list}");
            for (index, row) in list["documents"].as_array().unwrap().iter().enumerate() {
                assert_eq!(row["features"], 1, "document {index}: {list}");
            }
        }
    }

    #[test]
    fn mcp_tool_calls_go_to_the_engine_thread_and_the_rest_stays() {
        use crate::host::{Route, route};
        let mcp = |message: Value| {
            json!({"id": 1, "method": "mcp.message", "params": {"line": message.to_string()}})
                .to_string()
        };
        let call = |name: &str| {
            mcp(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
                "params": {"name": name, "arguments": {}}}))
        };
        assert_eq!(route(&call("command_run")), Route::Engine);
        assert_eq!(route(&call("control_call")), Route::Window);
        assert_eq!(
            route(&mcp(
                json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list"})
            )),
            Route::Window
        );
        let execute = r#"{"id":1,"method":"engine.execute","params":{"command":"file.new"}}"#;
        assert_eq!(route(execute), Route::Engine);
        assert_eq!(
            route(r#"{"id":1,"method":"batch","params":{"steps":[]}}"#),
            Route::Engine
        );
        assert_eq!(route(r#"{"id":1,"method":"ui.get"}"#), Route::Anytime);
        assert_eq!(route(r#"{"id":1,"method":"ui.screenshot"}"#), Route::Window);
        assert_eq!(route("not json"), Route::Window);
    }
}

/// A root named on the command line that does not exist yet is made when
/// its parent exists (the start screen's suggested `Documents/3DCraft`),
/// and is the made folder's one true spelling; one whose parent is missing
/// is still an error, as is one that is a file.
#[test]
fn a_missing_root_is_made_when_its_parent_exists() {
    let base = std::env::temp_dir().join(format!("craft3d-root-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let wanted = base.join("3DCraft");
    let access = app::root("--automation-write-root", wanted.to_str()).unwrap();
    assert!(wanted.is_dir());
    assert_eq!(
        access,
        Access::Root(craft3d_engine::canonical(&wanted).unwrap())
    );
    // Again: it is there now and is taken as it is.
    assert!(app::root("--automation-read-root", wanted.to_str()).is_ok());
    let deep = base.join("no/such/3DCraft");
    assert!(app::root("--automation-read-root", deep.to_str()).is_err());
    assert!(!deep.exists());
    let file = base.join("file");
    std::fs::write(&file, b"x").unwrap();
    assert!(app::root("--automation-read-root", file.to_str()).is_err());
    let _ = std::fs::remove_dir_all(&base);
}
