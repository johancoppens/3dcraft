//! A trace (`view.trace`) in the window itself, as a person sees it: the
//! application under a headless compositor with a virtual monitor, so the
//! window is shown and draws its frames, and `ui.screenshot {window:
//! true}` takes the window's own picture (panels, controls and the
//! viewport as the GPU drew it on the window's device). Until 2026-10-30
//! the window drew no trace at all, in either look, while the offscreen
//! `ui.screenshot` showed it.
//!
//! The pictures are kept in `target/evidence/window/` for whoever looks:
//! `window-trace-plain.png` and `window-trace-studio.png`.
//!
//! Needs `mutter` with `--virtual-monitor` (run headless on a D-Bus
//! session of its own) and a graphics adapter; skipped, saying so, where
//! there is no such thing.

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

const PATIENCE: Duration = Duration::from_secs(60);

fn on_path(tool: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(tool).is_file()))
}

/// A headless compositor with one virtual monitor, in a session of its
/// own, ended with everything it started when dropped.
struct Compositor {
    leader: Child,
    display: String,
}

impl Compositor {
    fn start(runtime: &Path) -> Option<Self> {
        let display = format!("craft3d-traces-{}", std::process::id());
        let leader = Command::new("setsid")
            .args(["dbus-run-session", "--", "mutter", "--headless", "--no-x11"])
            .args(["--virtual-monitor", "1600x1000"])
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
            if start.elapsed() > Duration::from_secs(20) {
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

/// The application, killed when dropped.
struct App(Child);

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Client {
    stream: TcpStream,
    reader: BufReader<TcpStream>,
    id: u64,
}

impl Client {
    fn call(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        let line = json!({"id": self.id, "method": method, "params": params}).to_string();
        self.stream
            .write_all(format!("{line}\n").as_bytes())
            .unwrap();
        let mut reply = String::new();
        self.reader
            .read_line(&mut reply)
            .unwrap_or_else(|e| panic!("no reply to {method}: {e}"));
        let reply: Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(reply["ok"], true, "{method} {params}: {reply}");
        reply["result"].clone()
    }

    fn execute(&mut self, command: &str, params: Value) -> Value {
        self.call(
            "engine.execute",
            json!({"command": command, "params": params}),
        )
    }
}

/// Pixels near `ink`.
fn count(rgb: &[u8], ink: [u8; 3], within: i32) -> usize {
    rgb.chunks(3)
        .filter(|p| (0..3).all(|k| (i32::from(p[k]) - i32::from(ink[k])).abs() <= within))
        .count()
}

fn read_png(path: &Path) -> Vec<u8> {
    let file = std::fs::File::open(path).unwrap();
    let mut reader = png::Decoder::new(BufReader::new(file)).read_info().unwrap();
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).unwrap();
    let bytes = buffer.get(..info.buffer_size()).unwrap();
    match info.color_type {
        png::ColorType::Rgba => bytes.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect(),
        _ => bytes.to_vec(),
    }
}

#[test]
fn the_window_draws_a_trace_in_both_looks() {
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
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/window");
    std::fs::create_dir_all(&dir).unwrap();
    let token_file = dir.join("token-traces");
    let _ = std::fs::remove_file(&token_file);
    let mut child = Command::new(env!("CARGO_BIN_EXE_craft3d"))
        .args(["--renderer", "auto", "--control", "0"])
        .arg("--control-token-file")
        .arg(&token_file)
        .arg("--recovery-dir")
        .arg("none")
        .arg("--automation-write-root")
        .arg(&dir)
        .env("WAYLAND_DISPLAY", &compositor.display)
        .env_remove("DISPLAY")
        .env_remove("CRAFT3D_RECENT_FILE")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut errors = BufReader::new(child.stderr.take().unwrap());
    let _app = App(child);
    let mut line = String::new();
    errors.read_line(&mut line).unwrap();
    let address = line
        .split_whitespace()
        .find(|word| word.starts_with("127.0.0.1:"))
        .unwrap_or_else(|| panic!("no address in `{line}`"))
        .to_string();
    line.clear();
    errors.read_line(&mut line).unwrap();
    assert!(line.contains("viewport renderer"), "{line}");
    // The rest of what it says is not read: drained so it never blocks.
    std::thread::spawn(move || {
        let mut sink = String::new();
        while errors.read_line(&mut sink).is_ok_and(|n| n > 0) {
            sink.clear();
        }
    });
    let token = std::fs::read_to_string(&token_file).unwrap();
    let stream = TcpStream::connect(&address).unwrap();
    stream.set_read_timeout(Some(PATIENCE)).unwrap();
    let reader = BufReader::new(stream.try_clone().unwrap());
    let mut client = Client {
        stream,
        reader,
        id: 0,
    };
    client.call("auth", json!({"token": token.trim()}));

    // A crank whose block goes round a circle of 15 mm, traced at a top
    // corner, shown at t = 0.3.
    client.execute("file.new", json!({}));
    client.execute("variable.set", json!({"name": "t", "expr": "0"}));
    client.execute(
        "feature.block",
        json!({"name": "Crank", "size": ["4 mm", "4 mm", "4 mm"],
               "position": ["15 mm * cos(360 deg * t) - 2 mm", "15 mm * sin(360 deg * t) - 2 mm", "0 mm"]}),
    );
    let edges = client.execute("document.edges", json!({"feature": "Crank"}));
    let top = edges["vertices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["point"][2].as_f64() == Some(4.0))
        .unwrap()["name"]
        .as_str()
        .unwrap()
        .to_string();
    client.execute("view.trace", json!({"point": top, "samples": 65}));
    client.execute("view.motion", json!({"t": 0.3}));
    client.call("ui.set", json!({"camera": {"yaw": -90.0, "pitch": 89.0}}));
    client.call("ui.fit", json!({}));
    let got = client.call("ui.get", json!({}));
    assert_eq!(got["timeline"]["traces"], 1, "{}", got["timeline"]);
    let ink = {
        let tokens = craft3d_ui_egui::Tokens::default();
        tokens.trace
    };
    for (look, least) in [("plain", 300), ("studio", 200)] {
        client.call("ui.set", json!({"look": {"mode": look, "accumulate": 1}}));
        let name = format!("window-trace-{look}.png");
        let shot = client.call("ui.screenshot", json!({"window": true, "path": name}));
        assert_eq!(shot["window"], true, "{shot}");
        let path = dir.join(&name);
        println!("evidence: {}", path.display());
        let rgb = read_png(&path);
        let inked = count(&rgb, ink, 40);
        println!("{look}: {inked} pixels of the trace");
        assert!(
            inked >= least,
            "{look}: {inked} pixels of the trace in the window ({})",
            path.display()
        );
    }
}
