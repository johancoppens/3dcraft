//! A device error never ends the window (docs/design/rendering.md › *A
//! device error never ends the window*). On an older Intel card drawing
//! through OpenGL, 0.1.2's window ended with exit code 101 a minute after
//! the studio look was switched on: egui's painter configured the surface
//! at a resize, wgpu waited for the GPU to come idle, OpenGL's wait gave up
//! after about two seconds while a studio frame was still drawn, and the
//! error went to the device's handler, wgpu's default, which panics.
//!
//! The first test raises a device error on a device of this machine: with
//! wgpu's default handler (the 0.1.2 window's) it panics, with the
//! window's handler it is counted. The second starts the window under a
//! headless compositor with `CRAFT3D_GPU_FAULT=configure`, which raises a
//! device error inside the resize the window hands to egui's painter once
//! the studio look was drawn, where the painter's `Surface::configure`
//! raised it on that card: the window lives, the studio look gives way to
//! the plain look, the trace says each step, and the window's picture
//! (with the bar) is kept in `target/evidence/window/device-error.png`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d::device_errors::{DeviceErrors, install, raise_fault};
use eframe::wgpu;
use std::sync::Arc;

/// A device on the first adapter of this machine, or `None`.
fn device() -> Option<wgpu::Device> {
    let adapter = craft3d_gpu::adapters().into_iter().next()?;
    let future = adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("craft3d device error test"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::downlevel_webgl2_defaults()
            .using_resolution(adapter.limits()),
        experimental_features: wgpu::ExperimentalFeatures::default(),
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::default(),
    });
    // Native wgpu's futures are ready at once.
    let mut future = std::pin::pin!(future);
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    match future.as_mut().poll(&mut context) {
        std::task::Poll::Ready(Ok((device, _queue))) => Some(device),
        _ => None,
    }
}

#[test]
fn a_device_error_panics_with_wgpus_handler_and_is_counted_with_the_windows() {
    let Some(device) = device() else {
        println!("SKIPPED: no graphics adapter");
        return;
    };
    // wgpu's default handler, which the window had until 0.1.2: a panic,
    // exit code 101 in the window.
    let default = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| raise_fault(&device)));
    let why = default.expect_err("wgpu's default handler panics on a device error");
    let text = why.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(text.contains("wgpu error"), "{text}");
    // The window's handler: counted and kept, no panic, again and again.
    let errors = Arc::new(DeviceErrors::default());
    install(&device, Arc::clone(&errors));
    for _ in 0..3 {
        raise_fault(&device);
    }
    assert_eq!(errors.count(), 3);
    assert!(errors.last().contains("Validation"), "{}", errors.last());
    assert_eq!(
        errors.counter().load(std::sync::atomic::Ordering::Relaxed),
        3
    );
}

#[cfg(target_os = "linux")]
mod window {
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

    /// A headless compositor with one virtual monitor, in a session of
    /// its own, ended with everything it started when dropped.
    struct Compositor {
        leader: Child,
        display: String,
    }

    impl Compositor {
        fn start(runtime: &Path) -> Option<Self> {
            let display = format!("craft3d-device-error-{}", std::process::id());
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
                .unwrap_or_else(|e| panic!("no reply to {method} (the window ended?): {e}"));
            let reply: Value = serde_json::from_str(&reply)
                .unwrap_or_else(|e| panic!("no reply to {method} (the window ended?): {e}"));
            assert_eq!(reply["ok"], true, "{method} {params}: {reply}");
            reply["result"].clone()
        }
    }

    #[test]
    fn the_window_lives_through_a_device_error_and_the_studio_look_gives_way() {
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
        let token_file = dir.join("token-device-error");
        let trace = dir.join("device-error-trace.txt");
        let _ = std::fs::remove_file(&token_file);
        let _ = std::fs::remove_file(&trace);
        let mut child = Command::new(env!("CARGO_BIN_EXE_craft3d"))
            .args(["--renderer", "auto", "--control", "0"])
            .arg("--control-token-file")
            .arg(&token_file)
            .arg("--recovery-dir")
            .arg("none")
            .arg("--automation-write-root")
            .arg(&dir)
            .env("WAYLAND_DISPLAY", &compositor.display)
            .env("CRAFT3D_GPU_FAULT", "configure")
            .env("CRAFT3D_GPU_TRACE", &trace)
            .env_remove("DISPLAY")
            .env_remove("CRAFT3D_RECENT_FILE")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut errors = BufReader::new(child.stderr.take().unwrap());
        let mut app = App(child);
        // The address: the first line that names one (the trace's own
        // lines on standard error come first).
        let mut line = String::new();
        let address = loop {
            line.clear();
            assert!(errors.read_line(&mut line).unwrap() > 0, "the window ended");
            if let Some(address) = line
                .split_whitespace()
                .find(|word| word.starts_with("127.0.0.1:"))
            {
                break address.to_string();
            }
        };
        // The rest of what it says is kept, read on a thread so it never
        // blocks.
        let said = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let keep = std::sync::Arc::clone(&said);
        std::thread::spawn(move || {
            let mut line = String::new();
            while errors.read_line(&mut line).is_ok_and(|n| n > 0) {
                keep.lock().unwrap().push_str(&line);
                line.clear();
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
        client.call(
            "engine.execute",
            json!({"command": "file.new", "params": {}}),
        );
        client.call(
            "engine.execute",
            json!({"command": "feature.block", "params": {"size": ["30 mm", "20 mm", "10 mm"]}}),
        );
        client.call("ui.fit", json!({}));
        client.call(
            "ui.set",
            json!({"look": {"mode": "studio", "accumulate": 1}}),
        );
        let studio = client.call(
            "ui.screenshot",
            json!({"window": true, "path": "device-error-studio.png"}),
        );
        assert_eq!(studio["window"], true, "{studio}");
        // The fault is raised once the first studio frame was presented;
        // the window's pictures keep it drawing until the look gave way.
        let start = Instant::now();
        let mut look = String::new();
        while start.elapsed() < PATIENCE {
            client.call(
                "ui.screenshot",
                json!({"window": true, "path": "device-error.png"}),
            );
            let got = client.call("ui.get", json!({}));
            look = got["state"]["look"]["mode"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            if look == "plain" {
                break;
            }
        }
        assert_eq!(look, "plain", "the studio look did not give way");
        // The bar is drawn in the frame after; then the window's picture.
        client.call(
            "ui.screenshot",
            json!({"window": true, "path": "device-error.png"}),
        );
        println!("evidence: {}", dir.join("device-error.png").display());
        // Alive, and it answers.
        assert!(app.0.try_wait().unwrap().is_none(), "the window ended");
        let again = client.call("ui.get", json!({}));
        assert_eq!(
            again["state"]["look"]["mode"], "plain",
            "{}",
            again["state"]["look"]
        );
        // The studio look asked for again gives way again.
        client.call("ui.set", json!({"look": {"mode": "studio"}}));
        client.call(
            "ui.screenshot",
            json!({"window": true, "path": "device-error-again.png"}),
        );
        let after = client.call("ui.get", json!({}));
        assert_eq!(
            after["state"]["look"]["mode"], "plain",
            "{}",
            after["state"]["look"]
        );
        // The surface is configured again once the device is idle.
        let start = Instant::now();
        let mut text = String::new();
        while start.elapsed() < PATIENCE {
            client.call("ui.get", json!({}));
            text = std::fs::read_to_string(&trace).unwrap_or_default();
            if text.contains("the surface was configured again") {
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        for wanted in [
            "CRAFT3D_GPU_FAULT: a device error is raised at a resize",
            "device error 1, taken by the window's handler",
            "the surface could not be configured",
            "the studio look gives way to the plain look for this session",
            "the first studio frame was presented",
            "the surface was configured again",
        ] {
            assert!(
                text.contains(wanted),
                "`{wanted}` not in the trace:\n{text}"
            );
        }
        assert!(app.0.try_wait().unwrap().is_none(), "the window ended");
        let stderr = said.lock().unwrap().clone();
        assert!(
            stderr.contains("device error 1, taken by the window's handler"),
            "{stderr}"
        );
        assert!(!stderr.contains("panicked"), "{stderr}");
        client.call("ui.quit", json!({"save": "discard"}));
        let start = Instant::now();
        let status = loop {
            if let Some(status) = app.0.try_wait().unwrap() {
                break status;
            }
            assert!(start.elapsed() < PATIENCE, "the window did not close");
            std::thread::sleep(Duration::from_millis(100));
        };
        assert!(status.success(), "{status:?}\n{stderr}");
    }
}
