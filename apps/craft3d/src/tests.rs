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
            // A reset is a closed connection too.
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => None,
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
            "mcp.message",
            "ui.get",
            "ui.set",
            "ui.fit",
            "ui.screenshot",
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
    let body = |x: usize, y: usize| {
        let i = (y * 300 + x) * 3;
        let p = [rgb[i], rgb[i + 1], rgb[i + 2]];
        // Shaded grey: not an axis, not the background, and not the dark
        // line along the plate's edges, which is drawn on its outline and
        // so reaches half its width beyond it.
        p != background && p[0].abs_diff(p[2]) < 40 && p[0] > 80
    };
    let columns = (0..300).filter(|&x| (0..200).any(|y| body(x, y))).count();
    let rows = (0..200).filter(|&y| (0..300).any(|x| body(x, y))).count();
    assert!((columns as f64 - 285.7).abs() <= 2.0, "{columns} columns");
    assert!((rows as f64 - 190.5).abs() <= 2.0, "{rows} rows");
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

#[test]
fn the_control_option_starts_a_listener_with_a_fresh_token_file() {
    let dir = workdir("option");
    let wake: Wake = Arc::new(|| {});
    let (server, _incoming) = app::control(&Options::default(), Arc::clone(&wake)).unwrap();
    assert!(server.is_none(), "no --control, no listener");

    let token_file = dir.join("control.token");
    let options = Options {
        control: Some(0),
        token_file: Some(token_file.clone()),
        ..Options::default()
    };
    let (server, incoming) = app::control(&options, Arc::clone(&wake)).unwrap();
    let server = server.unwrap();
    let token = std::fs::read_to_string(&token_file).unwrap();
    assert_eq!(token.trim().len(), 64);
    // A client that read the file gets in; the request reaches the channel.
    let mut client = Client::connect(server.address());
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
        control: Some(server.address().port()),
        token_file: Some(token_file.clone()),
        ..Options::default()
    };
    let error = app::control(&second, Arc::clone(&wake)).err().unwrap();
    assert!(error.contains("cannot listen"), "{error}");
    // A token file that holds something else stops the start.
    std::fs::write(&token_file, "not a token").unwrap();
    let error = app::control(&options, wake).err().unwrap();
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
    assert!(message.contains("refused the control token"), "{message}");
    // Nothing reached the session's thread.
    assert_eq!(rig.dispatched.load(Ordering::SeqCst), 0);

    // The window is gone: an error that says how to start it, for every
    // request, while notifications stay unanswered.
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
    for (reply, id) in replies.iter().zip([json!("a"), json!(9)]) {
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
