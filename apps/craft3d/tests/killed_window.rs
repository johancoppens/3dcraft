//! Roadmap R1 for the application itself: **a killed app recovers the
//! document on restart.** The real `craft3d` binary, under a compositor
//! that never shows its window: a model is built over the control
//! channel and autosaved, the process is killed with SIGKILL, and the
//! next start on the same recovery directory shows the document in
//! `ui.get`, opens it with `recovery.open` and measures the same volume.
//! A window that is closed on purpose (`ui.quit`) leaves nothing behind.
//!
//! Needs `mutter` (run headless, on a D-Bus session of its own) and a
//! graphics adapter; skipped, saying so, where there is no such thing.
//! (The same sentence without a window: `craft3d-cli`'s
//! `tests/recovery.rs`.)

#![cfg(target_os = "linux")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const PATIENCE: Duration = Duration::from_secs(20);

fn on_path(tool: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(tool).is_file()))
}

/// A headless compositor in a session of its own, ended with everything
/// it started when dropped.
struct Compositor {
    leader: Child,
    display: String,
}

impl Compositor {
    fn start(runtime: &Path) -> Option<Self> {
        let display = format!("craft3d-test-{}", std::process::id());
        let leader = Command::new("setsid")
            .args(["dbus-run-session", "--", "mutter", "--headless", "--no-x11"])
            .args(["--wayland-display", &display])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let compositor = Self { leader, display };
        let socket = runtime.join(&compositor.display);
        let start = Instant::now();
        while !socket.exists() {
            if start.elapsed() > PATIENCE {
                return None;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Some(compositor)
    }
}

impl Drop for Compositor {
    fn drop(&mut self) {
        let group = format!("-{}", self.leader.id());
        let _ = Command::new("kill").args(["-TERM", "--", &group]).status();
        let _ = self.leader.wait();
    }
}

/// The application and a connection to its control channel. Killed when
/// dropped, so no test leaves one running.
struct App {
    child: Child,
    stream: TcpStream,
    reader: BufReader<TcpStream>,
    /// What it said on standard error before the window existed.
    announced: Vec<String>,
    next_id: u64,
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl App {
    fn start(display: &str, dir: &Path) -> Self {
        let token_file = dir.join("token");
        let mut child = Command::new(env!("CARGO_BIN_EXE_craft3d"))
            .args(["--renderer", "cpu", "--control", "0"])
            .arg("--control-token-file")
            .arg(&token_file)
            .arg("--recovery-dir")
            .arg(dir.join("recovery"))
            .args(["--autosave-interval", "0.05"])
            .env("WAYLAND_DISPLAY", display)
            .env_remove("DISPLAY")
            // Never the developer's list of recent files.
            .env_remove("CRAFT3D_RECENT_FILE")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut errors = BufReader::new(child.stderr.take().unwrap());
        // The control channel's address, what can be recovered if
        // anything, then the renderer: the window exists.
        let mut announced = Vec::new();
        loop {
            let mut line = String::new();
            assert!(errors.read_line(&mut line).unwrap() > 0, "{announced:?}");
            let last = line.contains("viewport renderer");
            announced.push(line);
            if last {
                break;
            }
        }
        let address = announced[0]
            .split_whitespace()
            .find(|word| word.starts_with("127.0.0.1:"))
            .unwrap_or_else(|| panic!("no address in {announced:?}"))
            .to_string();
        let token = std::fs::read_to_string(&token_file).unwrap();
        let stream = TcpStream::connect(&address).unwrap();
        stream.set_read_timeout(Some(PATIENCE)).unwrap();
        let reader = BufReader::new(stream.try_clone().unwrap());
        let mut app = Self {
            child,
            stream,
            reader,
            announced,
            next_id: 1,
        };
        app.call("auth", json!({"token": token.trim()}));
        app
    }

    fn call(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({"id": id, "method": method, "params": params});
        self.stream
            .write_all(format!("{request}\n").as_bytes())
            .unwrap();
        let mut line = String::new();
        self.reader
            .read_line(&mut line)
            .unwrap_or_else(|e| panic!("no reply to {method}: {e}"));
        let reply: Value = serde_json::from_str(&line).unwrap_or_else(|e| panic!("{e}: {line}"));
        assert_eq!(reply["ok"], true, "{method}: {reply}");
        reply["result"].clone()
    }

    fn run(&mut self, command: &str, params: Value) -> Value {
        self.call(
            "engine.execute",
            json!({"command": command, "params": params}),
        )
    }

    fn kill(mut self) {
        self.child.kill().unwrap();
        let status = self.child.wait().unwrap();
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(status.signal(), Some(9), "{status}");
    }

    fn quit(mut self) {
        assert_eq!(self.call("ui.quit", json!({}))["quitting"], true);
        let asked = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "{status}");
                return;
            }
            assert!(asked.elapsed() < PATIENCE, "still running after ui.quit");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

fn names(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir.join("recovery")) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

#[test]
fn a_killed_app_recovers_the_document_on_restart() {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    let Some(runtime) = runtime.filter(|_| {
        ["setsid", "dbus-run-session", "mutter", "kill"]
            .iter()
            .all(|tool| on_path(tool))
    }) else {
        println!("SKIPPED: needs XDG_RUNTIME_DIR, setsid, dbus-run-session and mutter");
        return;
    };
    let Some(compositor) = Compositor::start(&runtime) else {
        println!("SKIPPED: the headless compositor did not start");
        return;
    };
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("killed-window");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let mut first = App::start(&compositor.display, &dir);
    assert_eq!(first.announced.len(), 2, "{:?}", first.announced);
    first.run("file.new", json!({"units": "mm"}));
    first.run(
        "feature.block",
        json!({"name": "Plate", "size": [40, 20, 5]}),
    );
    first.run("feature.cylinder", json!({"radius": 2, "height": 10}));
    let before = first.run("document.measure", json!({}));
    assert!((before["bodies"][0]["volume"].as_f64().unwrap() - 4000.0).abs() < 1e-9);
    // The window autosaves by itself, and says so in `ui.get`.
    let waited = Instant::now();
    loop {
        let recovery = first.call("ui.get", json!({}))["recovery"].clone();
        assert_eq!(recovery["problem"], Value::Null, "{recovery}");
        if recovery["writes"].as_u64().unwrap() >= 1 && recovery["pending"] == 0 {
            assert!(recovery["autosaved"].as_u64().unwrap() > 1_700_000_000);
            break;
        }
        assert!(waited.elapsed() < PATIENCE, "not autosaved: {recovery}");
        std::thread::sleep(Duration::from_millis(20));
    }
    first.kill();
    assert_eq!(names(&dir).len(), 3, "{:?}", names(&dir));

    // The next start says what it found, opens nothing by itself, and
    // shows it where the window's recovery bar reads it.
    let mut second = App::start(&compositor.display, &dir);
    assert!(
        second.announced.len() == 3 && second.announced[1].contains("can be recovered"),
        "{:?}",
        second.announced
    );
    let got = second.call("ui.get", json!({}));
    assert_eq!(got["document"], Value::Null);
    assert_eq!(got["state"]["panels"]["recovery"], true);
    let documents = got["recovery"]["documents"].as_array().unwrap().clone();
    assert_eq!(documents.len(), 1, "{got}");
    assert_eq!(documents[0]["features"], 2);
    assert_eq!(documents[0]["openable"], true);
    let listed = second.run("recovery.list", json!({}));
    assert_eq!(listed["documents"], json!(documents));

    let opened = second.run("recovery.open", json!({"id": documents[0]["id"]}));
    assert_eq!(opened["recovered"], true);
    assert_eq!(second.run("document.measure", json!({})), before);
    let got = second.call("ui.get", json!({}));
    assert_eq!(got["document"]["dirty"], true);
    assert_eq!(got["scene"]["bodies"], 2);
    assert_eq!(got["recovery"]["documents"], json!([]));

    // Closed on purpose, although it was not saved: nothing is left.
    second.quit();
    assert_eq!(names(&dir), Vec::<String>::new());
    let third = App::start(&compositor.display, &dir);
    assert_eq!(third.announced.len(), 2, "{:?}", third.announced);
    third.quit();
}
