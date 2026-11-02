//! How long the page's thread is held up by an export from the menu, an
//! edit that needs the bodies and `assert.list` on a heavy model, with
//! the regenerating worker doing them (since 2026-11-02) and with the
//! page doing them itself (what the reads of the export dialog, such an
//! edit and `assert.list` did before: regenerate the state on the page).
//!
//! The page's own code (`WebHost`, `Scenes`) runs on this thread, a frame
//! every 16 ms; the worker is a thread of its own that does what
//! `packaging/web/worker.js` does (`scene_begin`/`scene_step`, and
//! `craft3d_web::jobs::run` for a command or the page's own job), and the
//! checks are measured on a third, as in the page's checking worker. On
//! the cutaway turbofan of the benchmark (241 steps), opened from its
//! `.3dcraft` file as the page opens one:
//!
//! `cargo test --release -p craft3d-web --test page_stall -- --ignored --nocapture`
//!
//! It prints, per action, the longest frame of the page's thread and the
//! page's time in all, from the click until the answer is there; it
//! asserts only what holds whatever the machine: with the worker the page
//! regenerates and meshes nothing (`Session::work_here`), and the files
//! and documents are those the page makes itself.

#![cfg(not(target_arch = "wasm32"))]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{SceneStep, Session, Workspace, measure_checks, scene_begin, scene_step};
use craft3d_ui_egui::FileAction;
use craft3d_ui_egui::files::{ExportFormat, ExportWhat};
use craft3d_ui_egui::shell::Shell;
use craft3d_web::checks::{Reply, Transport};
use craft3d_web::host::{Outcome, WebHost};
use craft3d_web::scenes::Scenes;
use serde_json::{Value, json};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const PIECE: Duration = Duration::from_millis(30);
const FRAME: Duration = Duration::from_millis(16);

enum Message {
    Scene(u64, Vec<u8>, String),
    Command(u64, Vec<u8>, String),
    Check(u64, Vec<u8>, String),
    Cancel,
}

type Replies = Arc<Mutex<Vec<Reply>>>;

fn work(rx: &Receiver<Message>, replies: &Replies, checks: &Arc<Mutex<Vec<(u64, String)>>>) {
    let mut current: Option<u64> = None;
    loop {
        let mut messages = Vec::new();
        if current.is_none() {
            match rx.recv() {
                Ok(m) => messages.push(m),
                Err(_) => return,
            }
        }
        loop {
            match rx.try_recv() {
                Ok(m) => messages.push(m),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        for message in messages {
            match message {
                Message::Scene(token, document, work) => {
                    scene_begin(&document, &work);
                    current = Some(token);
                }
                Message::Command(token, document, work) => {
                    let answer = craft3d_web::jobs::run(&document, &work);
                    replies.lock().unwrap().push(Reply::Command {
                        token,
                        answer: Ok(answer),
                    });
                }
                Message::Check(token, document, work) => {
                    let answer = measure_checks(&document, &work);
                    checks.lock().unwrap().push((token, answer));
                }
                Message::Cancel => current = None,
            }
        }
        let Some(token) = current else {
            continue;
        };
        let until = Instant::now() + PIECE;
        let reply = match scene_step(move || Instant::now() >= until) {
            SceneStep::Part(bytes) => Some(Reply::Part { token, bytes }),
            SceneStep::Working => None,
            SceneStep::Done => {
                current = None;
                Some(Reply::Done { token })
            }
        };
        if let Some(reply) = reply {
            replies.lock().unwrap().push(reply);
        }
    }
}

struct Worker {
    tx: Sender<Message>,
}

impl Transport for Worker {
    fn post(&mut self, token: u64, document: &[u8], work: &str) -> Result<(), String> {
        self.tx
            .send(Message::Scene(token, document.to_vec(), work.into()))
            .map_err(|e| e.to_string())
    }
    fn post_command(&mut self, token: u64, document: &[u8], work: &str) -> Result<(), String> {
        self.tx
            .send(Message::Command(token, document.to_vec(), work.into()))
            .map_err(|e| e.to_string())
    }
    fn stop(&mut self) {}
    fn cancel(&mut self) {
        let _ = self.tx.send(Message::Cancel);
    }
}

/// The page with its workers (two threads: the regenerating one and the
/// checking one).
struct Page {
    host: WebHost,
    scenes: Scenes<Worker>,
    replies: Replies,
    checks: Arc<Mutex<Vec<(u64, String)>>>,
    checker: Sender<Message>,
    clock: Instant,
    /// The page thread's frames since the last [`Page::take_frames`], ms.
    frames: Vec<f64>,
}

impl Page {
    fn new() -> Self {
        let replies: Replies = Arc::default();
        let checks = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = channel();
        let (r, c) = (Arc::clone(&replies), Arc::clone(&checks));
        std::thread::spawn(move || work(&rx, &r, &c));
        let (checker, rx) = channel();
        let (r, c) = (Arc::clone(&replies), Arc::clone(&checks));
        std::thread::spawn(move || work(&rx, &r, &c));
        let mut host = WebHost::new(Shell::new());
        host.set_stepwise(true);
        host.session.set_checks_elsewhere(true);
        let scenes = Scenes::new(Worker { tx }, &mut host.session);
        Self {
            host,
            scenes,
            replies,
            checks,
            checker,
            clock: Instant::now(),
            frames: Vec::new(),
        }
    }

    /// One frame of the page's thread, timed.
    fn frame(&mut self) -> Outcome {
        let started = Instant::now();
        let mut outcome = self.host.pump();
        let taken = std::mem::take(&mut *self.replies.lock().unwrap());
        let quality = self.host.shell.scene_quality();
        let now = self.clock.elapsed().as_secs_f64() * 1000.0;
        let done = self
            .scenes
            .frame_at(&mut self.host.session, quality, taken, now);
        let answered = self.host.answered(done.commands);
        outcome.downloads.extend(answered.downloads);
        outcome.pick = outcome.pick.or(answered.pick);
        self.host.prepare(|| true);
        for (token, answer) in std::mem::take(&mut *self.checks.lock().unwrap()) {
            self.host.session.checks_answered(token, Ok(&answer));
        }
        if let Some(token) = self.host.session.checks_job()
            && let Ok(request) = self.host.session.checks_request(token)
        {
            let _ = self
                .checker
                .send(Message::Check(token, request.document, request.work));
        }
        self.frames.push(started.elapsed().as_secs_f64() * 1000.0);
        outcome
    }

    fn idle(&mut self) -> bool {
        let quality = self.host.shell.scene_quality();
        self.scenes.busy().is_none()
            && !self.host.session.commands_waiting()
            && self.host.session.scene_job(quality).is_none()
            && !self.host.export_choices_waiting()
            && self.host.session.checks_job().is_none()
    }

    /// Frames until nothing waits; what they left for the page.
    fn settle(&mut self) -> Outcome {
        let mut outcome = Outcome::default();
        let start = Instant::now();
        loop {
            let started = Instant::now();
            let more = self.frame();
            outcome.downloads.extend(more.downloads);
            outcome.pick = outcome.pick.or(more.pick);
            if self.idle() && self.replies.lock().unwrap().is_empty() {
                return outcome;
            }
            assert!(start.elapsed() < Duration::from_secs(900), "never settled");
            std::thread::sleep(FRAME.saturating_sub(started.elapsed()));
        }
    }

    /// The longest frame and the page's time in all since the last call,
    /// ms, and how many frames.
    fn take_frames(&mut self) -> (f64, f64, usize) {
        let frames = std::mem::take(&mut self.frames);
        let longest = frames.iter().copied().fold(0.0, f64::max);
        (longest, frames.iter().sum(), frames.len())
    }

    /// File › Export as `what` in `format`, through the dialog.
    fn export(&mut self, what: &ExportWhat, format: ExportFormat) -> Outcome {
        let started = Instant::now();
        self.host.file_action(FileAction::Export);
        self.frames.push(started.elapsed().as_secs_f64() * 1000.0);
        self.settle();
        self.host.shell.set_export_what(what.clone()).unwrap();
        self.host.shell.set_export_format(format).unwrap();
        self.host.shell.answer_export(true);
        let request = self.host.shell.take_export_answer().unwrap();
        let started = Instant::now();
        let mut outcome = self.host.export_request(request);
        self.frames.push(started.elapsed().as_secs_f64() * 1000.0);
        let more = self.settle();
        outcome.downloads.extend(more.downloads);
        outcome
    }

    /// `id` dispatched as the page dispatches what a person asks for.
    fn run(&mut self, id: &str, params: &Value) -> Value {
        let started = Instant::now();
        let now = self
            .host
            .session
            .execute_deferrable(id, params)
            .unwrap_or_else(|e| json!({"error": e.message()}));
        self.frames.push(started.elapsed().as_secs_f64() * 1000.0);
        self.settle();
        now
    }
}

fn turbofan() -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/engine/tests/bench_models/turbofan.steps.json");
    let steps: Vec<(String, Value)> =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut s = Session::new(Workspace::memory());
    s.execute("file.new", &json!({"units": "mm"})).unwrap();
    for (id, params) in steps {
        s.execute(&id, &params).unwrap();
    }
    s.execute("file.save", &json!({"path": "turbofan.3dcraft"}))
        .unwrap();
    s.take_file("turbofan.3dcraft").unwrap()
}

/// A body of the turbofan to measure: the first one, by name.
fn a_body(bytes: &[u8]) -> String {
    let mut s = Session::new(Workspace::memory());
    s.open_bytes("turbofan.3dcraft", bytes).unwrap();
    let found = s
        .execute("select.query", &json!({"kind": "body", "limit": 1}))
        .unwrap();
    found["items"][0]["name"].as_str().unwrap().to_string()
}

fn report(what: &str, worker: bool, page: &mut Page, wall: Duration) {
    let (longest, total, frames) = page.take_frames();
    println!(
        "{what} ({}): longest page frame {longest:.1} ms, page thread {total:.1} ms over {frames} frames, answered after {:.0} ms",
        if worker { "worker" } else { "on the page" },
        wall.as_secs_f64() * 1000.0
    );
}

#[test]
#[ignore = "timings: run in release with --ignored --nocapture"]
fn the_page_stalls_on_the_turbofan() {
    let bytes = turbofan();
    let body = a_body(&bytes);
    for worker in [false, true] {
        let mut page = Page::new();
        page.host
            .receive("turbofan.3dcraft", bytes.clone(), None)
            .unwrap();
        page.settle();
        page.take_frames();
        // Before 2026-11-02 these regenerated the state on the page: the
        // page that has the commands run here does just that.
        page.host.session.set_commands_elsewhere(worker);
        // A new state, not regenerated on the page (as after a parameter
        // set in the form): the case the reads used to regenerate for.
        page.run(
            "variable.set",
            &json!({"name": "filletM", "expr": "1.6 mm"}),
        );
        page.take_frames();
        let work = page.host.session.work_here();
        let mut files = Vec::new();
        for format in [ExportFormat::Step, ExportFormat::Stl] {
            let started = Instant::now();
            let outcome = page.export(&ExportWhat::Model, format);
            report(
                &format!("export {format:?}"),
                worker,
                &mut page,
                started.elapsed(),
            );
            assert_eq!(outcome.downloads.len(), 1, "{format:?}");
            files.push(outcome.downloads[0].bytes.clone());
        }
        let started = Instant::now();
        page.run(
            "variable.measure",
            &json!({"of": body, "name": "Vb", "measure": "volume"}),
        );
        report(
            "variable.measure (an edit)",
            worker,
            &mut page,
            started.elapsed(),
        );
        let started = Instant::now();
        page.run(
            "assert.add",
            &json!({"name": "vol", "measure": "volume", "of": body, "min": "0 mm^3"}),
        );
        report("assert.add (an edit)", worker, &mut page, started.elapsed());
        page.run(
            "variable.set",
            &json!({"name": "filletM", "expr": "1.5 mm"}),
        );
        page.take_frames();
        let started = Instant::now();
        let list = page.run("assert.list", &json!({}));
        report("assert.list", worker, &mut page, started.elapsed());
        let _ = list;
        if worker {
            assert_eq!(
                page.host.session.work_here(),
                work,
                "regenerated on the page"
            );
        }
        println!(
            "{}: document {} bytes, STEP {} bytes, STL {} bytes",
            if worker { "worker" } else { "page" },
            page.host.session.document_bytes().map_or(0, |b| b.len()),
            files[0].len(),
            files[1].len()
        );
    }
}
