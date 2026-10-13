//! Roadmap R1, the acceptance sentence: **a killed app recovers the
//! document on restart.** The real thing, once: a `craft3d-cli serve`
//! process builds a model, autosaves it, is killed with SIGKILL (no clean
//! end, no destructor, no flush), and a new process on the same recovery
//! directory lists the document, opens it and measures the same volume.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

fn workdir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("cli-recovery-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A running `craft3d-cli serve`.
struct Server {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl Server {
    fn start(dir: &Path, extra: &[&str]) -> Self {
        let work = dir.join("work");
        std::fs::create_dir_all(&work).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_craft3d-cli"))
            .arg("serve")
            .arg("--automation-read-root")
            .arg(&work)
            .arg("--automation-write-root")
            .arg(&work)
            .arg("--recovery-dir")
            .arg(dir.join("recovery"))
            .args(extra)
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("the binary starts");
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            stdout,
            next_id: 1,
        }
    }

    fn ask(&mut self, command: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({
            "id": id,
            "method": "engine.execute",
            "params": {"command": command, "params": params},
        });
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{request}").unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        let reply: Value = serde_json::from_str(&line).unwrap_or_else(|e| panic!("{e}: {line}"));
        assert_eq!(reply["id"], id, "{reply}");
        if reply["ok"] == true {
            Ok(reply["result"].clone())
        } else {
            Err(reply["error"].as_str().unwrap().to_string())
        }
    }

    fn run(&mut self, command: &str, params: Value) -> Value {
        self.ask(command, params.clone())
            .unwrap_or_else(|e| panic!("{command} {params}: {e}"))
    }

    /// Waits until everything that changed is in a recovery file.
    fn wait_for_autosave(&mut self) -> Value {
        let started = Instant::now();
        loop {
            let listed = self.run("recovery.list", json!({}));
            assert_eq!(listed["problem"], Value::Null, "{listed}");
            if listed["writes"].as_u64().unwrap() >= 1 && listed["pending"] == 0 {
                return listed;
            }
            assert!(
                started.elapsed() < Duration::from_secs(30),
                "nothing was autosaved: {listed}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// SIGKILL on Unix, TerminateProcess on Windows: the process gets no
    /// chance to do anything.
    fn kill(mut self) {
        self.child.kill().unwrap();
        let status = self.child.wait().unwrap();
        assert!(!status.success(), "{status}");
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            assert_eq!(status.signal(), Some(9), "{status}");
        }
    }

    /// Closes stdin: the server's clean end.
    fn end(mut self) {
        drop(self.stdin.take());
        let status = self.child.wait().unwrap();
        assert!(status.success(), "{status}");
    }
}

impl Drop for Server {
    /// No test leaves a server behind, also when it fails half-way.
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn build(server: &mut Server) {
    server.run("file.new", json!({}));
    server.run("variable.set", json!({"name": "width", "expr": "40 mm"}));
    server.run(
        "feature.block",
        json!({"name": "Plate", "size": ["width", 20, 5]}),
    );
    server.run("feature.cylinder", json!({"radius": 2, "height": 10}));
    server.run("sketch.create", json!({"name": "Profile"}));
    server.run(
        "sketch.polyline",
        json!({"sketch": "Profile", "closed": true, "points": [[0, 0], [9, 1], [10, 6], [1, 5]]}),
    );
    server.run(
        "feature.extrude",
        json!({"sketch": "Profile", "distance": "width / 8", "name": "Body"}),
    );
}

/// The document without its undo history, which no file holds.
fn inspected(server: &mut Server) -> Value {
    let mut all = server.run("document.inspect", json!({}));
    all.as_object_mut().unwrap().remove("history").unwrap();
    all
}

fn volumes(inspected: &Value) -> Vec<f64> {
    inspected["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|body| body["volume"].as_f64().unwrap())
        .collect()
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
    let dir = workdir("killed");
    let mut first = Server::start(&dir, &["--autosave-interval", "0.05"]);
    // Nothing to recover, and autosave is on.
    let listed = first.run("recovery.list", json!({}));
    assert_eq!(listed["enabled"], true);
    assert_eq!(listed["interval"], 0.05);
    assert_eq!(listed["documents"], json!([]));
    build(&mut first);
    let before = inspected(&mut first);
    let volumes_before = volumes(&before);
    assert_eq!(volumes_before.len(), 3, "{before}");
    // The plate is 40 x 20 x 5; the others are bodies too.
    assert!(
        (volumes_before[0] - 4000.0).abs() < 1e-9,
        "{volumes_before:?}"
    );
    assert!(
        volumes_before.iter().all(|v| *v > 1.0),
        "{volumes_before:?}"
    );
    first.wait_for_autosave();
    first.kill();
    // What a killed process leaves: its lock file, the document, its record.
    assert_eq!(names(&dir).len(), 3, "{:?}", names(&dir));

    let mut second = Server::start(&dir, &[]);
    let listed = second.run("recovery.list", json!({}));
    let documents = listed["documents"].as_array().unwrap();
    assert_eq!(documents.len(), 1, "{listed}");
    assert_eq!(documents[0]["openable"], true);
    assert_eq!(documents[0]["features"], 4);
    assert_eq!(documents[0]["name"], "untitled");
    // Nothing is opened by itself.
    assert!(second.ask("document.inspect", json!({})).is_err());

    let opened = second.run("recovery.open", json!({"id": documents[0]["id"]}));
    assert_eq!(opened["recovered"], true);
    let after = inspected(&mut second);
    // The same volumes to the last bit, and the same everything else.
    assert_eq!(volumes(&after), volumes_before);
    assert_eq!(after, before);

    // Saved: the recovery file is gone; a clean end leaves nothing.
    second.run("file.save", json!({"path": "part.3dcraft"}));
    assert_eq!(names(&dir).len(), 1, "{:?}", names(&dir));
    second.end();
    assert_eq!(names(&dir), Vec::<String>::new());
    assert!(dir.join("work").join("part.3dcraft").exists());
}

#[test]
fn a_server_that_ends_cleanly_leaves_nothing_to_recover() {
    let dir = workdir("clean");
    let mut first = Server::start(&dir, &["--autosave-interval=0.05"]);
    build(&mut first);
    first.wait_for_autosave();
    assert_eq!(names(&dir).len(), 3);
    first.end();
    assert_eq!(names(&dir), Vec::<String>::new());
    let mut second = Server::start(&dir, &[]);
    let listed = second.run("recovery.list", json!({}));
    assert_eq!(listed["documents"], json!([]));
    second.end();
}

#[test]
fn a_running_server_is_not_recovered_from_under_it() {
    let dir = workdir("running");
    let mut first = Server::start(&dir, &["--autosave-interval", "0.05"]);
    build(&mut first);
    first.wait_for_autosave();
    let mut second = Server::start(&dir, &[]);
    let listed = second.run("recovery.list", json!({}));
    assert_eq!(listed["documents"], json!([]), "{listed}");
    assert_eq!(listed["running"], 1);
    let discarded = second.run("recovery.discard", json!({"all": true}));
    assert_eq!(discarded["discarded"], 0);
    assert_eq!(names(&dir).len(), 3);
    // Killed: now it is offered.
    first.kill();
    let listed = second.run("recovery.list", json!({}));
    assert_eq!(listed["documents"].as_array().unwrap().len(), 1, "{listed}");
    assert_eq!(listed["running"], 0);
    second.end();
}

#[test]
fn the_recovery_flags_are_checked() {
    let run = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_craft3d-cli"))
            .args(args)
            .current_dir(env!("CARGO_TARGET_TMPDIR"))
            .stdin(Stdio::null())
            .output()
            .unwrap();
        (
            output.status.code().unwrap(),
            String::from_utf8(output.stdout).unwrap(),
            String::from_utf8(output.stderr).unwrap(),
        )
    };
    for (args, words) in [
        (&["serve", "--recovery-dir"][..], "needs a value"),
        (&["serve", "--recovery-dir", ""], "needs a directory"),
        (
            &["serve", "--recovery-dir", "a", "--recovery-dir", "b"],
            "given twice",
        ),
        (
            &["serve", "--autosave-interval", "5"],
            "needs `--recovery-dir DIR`",
        ),
        (
            &[
                "serve",
                "--recovery-dir",
                "none",
                "--autosave-interval",
                "5",
            ],
            "needs `--recovery-dir DIR`",
        ),
        (
            &["mcp", "--recovery-dir", "r", "--autosave-interval", "soon"],
            "not a number of seconds",
        ),
        (
            &["mcp", "--recovery-dir", "r", "--autosave-interval", "0"],
            "not a number of seconds",
        ),
        (
            &[
                "mcp",
                "--bridge",
                "7",
                "--control-token-file",
                "missing.token",
                "--recovery-dir",
                "r",
            ],
            "give `--recovery-dir` to `craft3d`",
        ),
    ] {
        let (code, _, stderr) = run(args);
        assert_eq!(code, 2, "{args:?}: {stderr}");
        assert!(stderr.contains(words), "{args:?}: {stderr}");
        assert!(!stderr.contains("panicked"), "{stderr}");
    }
    // `none` is the default said aloud; the help names both flags.
    let (code, stdout, stderr) = run(&["serve", "--recovery-dir", "none"]);
    assert_eq!((code, stdout.as_str(), stderr.as_str()), (0, "", ""));
    for subcommand in ["serve", "mcp"] {
        let (_, help, _) = run(&[subcommand, "--help"]);
        assert!(
            help.contains("--recovery-dir DIR|none")
                && help.contains("--autosave-interval SECONDS"),
            "{help}"
        );
    }
}
