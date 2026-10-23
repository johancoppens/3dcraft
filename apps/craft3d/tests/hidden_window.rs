//! The window under a compositor that never shows it: the control channel
//! must answer, and `ui.quit` must end the process, without a single frame.
//!
//! A Wayland compositor sends no frame callbacks to a window it does not
//! show (another workspace, minimised, covered, a locked screen), and
//! winit then delivers no redraw. Until 2026-10-13 requests were answered
//! in a frame only: such a window answered nothing until the request timed
//! out after 120 s, and a `ui.quit` it had answered was never acted on.
//!
//! Needs `mutter` (run headless, without a monitor, on a D-Bus session of
//! its own) and a graphics adapter; skipped, saying so, where there is no
//! such thing.

#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

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
        // `setsid`: the process is the leader of a group of its own, so
        // the D-Bus daemon and the compositor can be ended together.
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

/// The application, killed when dropped.
struct App(Child);

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn call(stream: &mut TcpStream, reader: &mut BufReader<TcpStream>, request: &str) -> String {
    let asked = Instant::now();
    stream.write_all(format!("{request}\n").as_bytes()).unwrap();
    let mut reply = String::new();
    reader
        .read_line(&mut reply)
        .unwrap_or_else(|e| panic!("no reply to {request} after {:?}: {e}", asked.elapsed()));
    // Not the whole line: the first one holds the token.
    let shown: String = request.chars().take(36).collect();
    println!("{:>6.1} ms  {shown}…", asked.elapsed().as_secs_f64() * 1e3);
    reply
}

fn quits_unseen(renderer: &str, display: &str, dir: &Path) {
    let token_file = dir.join(format!("token-{renderer}"));
    let _ = std::fs::remove_file(&token_file);
    let mut child = Command::new(env!("CARGO_BIN_EXE_craft3d"))
        .args(["--renderer", renderer, "--control", "0"])
        .arg("--control-token-file")
        .arg(&token_file)
        // Not the developer's own recovery directory.
        .arg("--recovery-dir")
        .arg(dir.join(format!("recovery-{renderer}")))
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
    let mut app = App(child);
    // `craft3d: control channel on 127.0.0.1:PORT (token in FILE)`.
    let mut line = String::new();
    errors.read_line(&mut line).unwrap();
    let address = line
        .split_whitespace()
        .find(|word| word.starts_with("127.0.0.1:"))
        .unwrap_or_else(|| panic!("no address in `{line}`"))
        .to_string();
    // The window exists once the renderer is announced.
    line.clear();
    errors.read_line(&mut line).unwrap();
    assert!(line.contains("viewport renderer"), "{line}");
    let token = std::fs::read_to_string(&token_file).unwrap();

    let mut stream = TcpStream::connect(&address).unwrap();
    stream.set_read_timeout(Some(PATIENCE)).unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let auth = format!(
        r#"{{"id":0,"method":"auth","params":{{"token":"{}"}}}}"#,
        token.trim()
    );
    assert!(call(&mut stream, &mut reader, &auth).contains(r#""ok":true"#));
    for request in [
        r#"{"id":1,"method":"engine.execute","params":{"command":"file.new","params":{"units":"mm"}}}"#,
        r#"{"id":2,"method":"engine.execute","params":{"command":"feature.block","params":{"name":"P","size":["10 mm","10 mm","10 mm"]}}}"#,
        r#"{"id":3,"method":"ui.get"}"#,
        r#"{"id":4,"method":"ui.screenshot","params":{"width":64,"height":48}}"#,
    ] {
        let reply = call(&mut stream, &mut reader, request);
        assert!(reply.contains(r#""ok":true"#), "{request}: {reply}");
    }
    // A picture of the window itself needs a frame, which never comes: it
    // is answered with its error at its deadline (8 s), not left waiting
    // for the control channel's 120 s (R1 B8).
    let asked = Instant::now();
    let reply = call(
        &mut stream,
        &mut reader,
        r#"{"id":6,"method":"ui.screenshot","params":{"window":true}}"#,
    );
    assert!(
        reply.contains(r#""ok":false"#) && reply.contains("drew no frame"),
        "{reply}"
    );
    assert!(
        asked.elapsed() < Duration::from_secs(12),
        "answered after {:?}",
        asked.elapsed()
    );
    let reply = call(&mut stream, &mut reader, r#"{"id":5,"method":"ui.quit"}"#);
    assert!(reply.contains(r#""quitting":true"#), "{reply}");
    let asked = Instant::now();
    loop {
        if let Some(status) = app.0.try_wait().unwrap() {
            println!(
                "--renderer {renderer}: gone {:.0} ms after ui.quit ({status})",
                asked.elapsed().as_secs_f64() * 1e3
            );
            assert!(status.success(), "{status}");
            break;
        }
        assert!(
            asked.elapsed() < PATIENCE,
            "--renderer {renderer}: still running {PATIENCE:?} after ui.quit was answered"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_window_that_is_never_shown_answers_and_quits() {
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
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("hidden-window");
    std::fs::create_dir_all(&dir).unwrap();
    for renderer in ["cpu", "auto"] {
        quits_unseen(renderer, &compositor.display, &dir);
    }
}
