//! Embed's core natively (`crate::embed`): a page's messages answered by
//! the page's own [`WebHost`], with and without its regenerating worker
//! (a [`Transport`] that answers with what `worker.js` runs, a frame at a
//! time, as `beside_tests.rs` has it). What it checks: commands answered
//! under their ids, errors as errors, the events and their order, files in
//! and out as bytes, a command run in the worker answered when its answer
//! comes, the requests behind an edit made there kept in order, the
//! presentation set by a message, and that no message, however hostile,
//! makes it panic or keep more than its caps.

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use crate::checks::{Reply, Transport};
use crate::embed::{Embed, GIVE, MAX_FILE_BYTES, MAX_WAITING, Message, Out, PROTOCOL};
use crate::host::WebHost;
use crate::scenes::Scenes;
use craft3d_automation::MAX_REQUEST_BYTES;
use craft3d_engine::{Session, Workspace, scene_begin, scene_next};
use craft3d_ui_egui::shell::Shell;
use craft3d_ui_egui::state::Presentation;
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

enum Job {
    Scene(u64, Vec<u8>, String),
    Command(u64, Vec<u8>, String),
}

/// The files of a reply, by name.
type Files = Vec<(String, Vec<u8>)>;

/// The regenerating worker: keeps what it is posted and answers all of it
/// when the frame asks.
#[derive(Clone, Default)]
struct Beside {
    jobs: Rc<RefCell<Vec<Job>>>,
    /// How many scenes it built, and how many commands it ran.
    scenes: Rc<Cell<usize>>,
    commands: Rc<Cell<usize>>,
    /// It answers nothing while this is set (a long regeneration).
    held: Rc<Cell<bool>>,
}

impl Transport for Beside {
    fn post(&mut self, token: u64, document: &[u8], work: &str) -> Result<(), String> {
        self.jobs
            .borrow_mut()
            .push(Job::Scene(token, document.to_vec(), work.into()));
        Ok(())
    }
    fn post_command(&mut self, token: u64, document: &[u8], work: &str) -> Result<(), String> {
        self.jobs
            .borrow_mut()
            .push(Job::Command(token, document.to_vec(), work.into()));
        Ok(())
    }
    fn stop(&mut self) {
        self.jobs.borrow_mut().clear();
    }
    fn cancel(&mut self) {}
}

impl Beside {
    fn answers(&self) -> Vec<Reply> {
        if self.held.get() {
            return Vec::new();
        }
        let mut replies = Vec::new();
        for job in self.jobs.borrow_mut().drain(..) {
            match job {
                Job::Scene(token, document, work) => {
                    self.scenes.set(self.scenes.get() + 1);
                    scene_begin(&document, &work);
                    while let Some(bytes) = scene_next() {
                        replies.push(Reply::Part { token, bytes });
                    }
                    replies.push(Reply::Done { token });
                }
                Job::Command(token, document, work) => {
                    self.commands.set(self.commands.get() + 1);
                    replies.push(Reply::Command {
                        token,
                        answer: Ok(crate::jobs::run(&document, &work)),
                    });
                }
            }
        }
        replies
    }
}

/// The page that embeds 3DCraft, as `page.rs` drives it: its host, the
/// core, and (or not) the regenerating worker; a clock in milliseconds.
struct Page {
    host: WebHost,
    embed: Embed,
    scenes: Option<Scenes<Beside>>,
    worker: Beside,
    clock: Rc<Cell<f64>>,
    out: Vec<Out>,
}

impl Page {
    /// Without a worker: everything is done on the page, at once.
    fn alone() -> Self {
        Self::new(false)
    }

    /// With the regenerating worker, a piece per frame, as in the browser.
    fn with_worker() -> Self {
        Self::new(true)
    }

    fn new(worker: bool) -> Self {
        let mut host = WebHost::new(Shell::new());
        let beside = Beside::default();
        let scenes = worker.then(|| {
            host.set_stepwise(true);
            host.session.set_checks_elsewhere(true);
            Scenes::new(beside.clone(), &mut host.session)
        });
        let clock = Rc::new(Cell::new(0.0));
        let time = Rc::clone(&clock);
        let embed = Embed::new(true, Box::new(move || time.get()));
        Self {
            host,
            embed,
            scenes,
            worker: beside,
            clock,
            out: Vec::new(),
        }
    }

    fn send(&mut self, line: impl Into<String>) {
        self.embed.receive(&mut self.host, Message::line(line));
        self.collect();
    }

    fn send_file(&mut self, line: &str, bytes: Vec<u8>) {
        self.embed.receive(
            &mut self.host,
            Message {
                line: line.to_string(),
                bytes: Some(bytes),
            },
        );
        self.collect();
    }

    fn collect(&mut self) {
        self.out.extend(self.embed.take_out());
    }

    /// One frame: the worker's answers, the host's work, the core's.
    fn frame(&mut self) {
        self.clock.set(self.clock.get() + 16.0);
        if let Some(scenes) = &mut self.scenes {
            let quality = self.host.shell.scene_quality();
            let replies = self.worker.answers();
            let done = scenes.frame_at(&mut self.host.session, quality, replies, self.clock.get());
            if !done.commands.is_empty() {
                self.host.answered(done.commands);
            }
        }
        self.host.pump();
        self.host.prepare(|| false);
        // Checks measured elsewhere are answered at once.
        while let Some(token) = self.host.session.checks_job() {
            let request = self.host.session.checks_request(token).unwrap();
            let reply = craft3d_engine::measure_checks(&request.document, &request.work);
            self.host.session.checks_answered(token, Ok(&reply));
        }
        self.embed.frame(&mut self.host);
        self.collect();
    }

    fn frames(&mut self, n: usize) {
        for _ in 0..n {
            self.frame();
        }
    }

    /// The replies and events since the last call, as JSON, with the
    /// files of each reply.
    fn take(&mut self) -> Vec<(Value, Files)> {
        self.out
            .drain(..)
            .map(|out| {
                let json = out.json();
                let files = match out {
                    Out::Reply { files, .. } => files,
                    Out::Event(_) => Vec::new(),
                };
                (json, files)
            })
            .collect()
    }

    fn replies(&mut self) -> Vec<Value> {
        self.take().into_iter().map(|(json, _)| json).collect()
    }

    /// One request answered at once: its reply.
    fn ask(&mut self, line: &str) -> Value {
        self.send(line);
        let mut got = self.replies();
        got.retain(|r| r.get("event").is_none());
        assert_eq!(got.len(), 1, "{line}: {got:?}");
        got.remove(0)
    }

    /// One request answered now or once the worker answered it: its
    /// reply (the events meanwhile are dropped).
    fn answer(&mut self, line: &str) -> Value {
        let id = serde_json::from_str::<Value>(line).unwrap()["id"].clone();
        self.send(line);
        for _ in 0..20 {
            if let Some(reply) = self
                .replies()
                .into_iter()
                .find(|r| r.get("id") == Some(&id))
            {
                return reply;
            }
            self.frame();
        }
        panic!("{line}: no reply");
    }
}

fn execute(id: impl Into<Value>, command: &str, params: Value) -> String {
    json!({"id": id.into(), "method": "engine.execute", "params": {"command": command, "params": params}})
        .to_string()
}

/// A block of 10 × 20 × 30 mm, as a `.3dcraft` file and as a STEP file.
fn block_files() -> (Vec<u8>, Vec<u8>) {
    let mut session = Session::new(Workspace::memory());
    for (id, params) in [
        ("file.new", json!({"units": "mm"})),
        ("variable.set", json!({"name": "width", "expr": "10 mm"})),
        (
            "variable.publish",
            json!({"name": "width", "min": "5 mm", "max": "100 mm"}),
        ),
        (
            "feature.block",
            json!({"name": "Block", "size": ["width", 20, 30]}),
        ),
        ("file.save", json!({"path": "block.3dcraft"})),
        ("file.export", json!({"path": "block.step"})),
    ] {
        session.execute(id, &params).unwrap();
    }
    (
        session.take_file("block.3dcraft").unwrap(),
        session.take_file("block.step").unwrap(),
    )
}

/// An assertion on the block's volume: an edit that measures, so it is
/// made where the bodies are (`assert.add`).
fn assertion(name: &str) -> Value {
    json!({"name": name, "measure": "volume", "of": "Block",
           "eq": "6000 * (1 mm)^3", "tol": "1e-6 * (1 mm)^3"})
}

fn volume(reply: &Value) -> f64 {
    reply["result"]["volume"]
        .as_f64()
        .unwrap_or_else(|| panic!("{reply}"))
}

#[test]
fn a_page_hears_ready_and_its_commands_are_answered_under_their_ids() {
    let mut page = Page::alone();
    page.embed.greet(&page.host);
    page.embed.greet(&page.host);
    page.collect();
    let said = page.replies();
    assert_eq!(said.len(), 1, "ready once: {said:?}");
    assert_eq!(said[0]["event"], "ready");
    assert_eq!(said[0]["protocol"], PROTOCOL);
    assert_eq!(said[0]["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(said[0]["presentation"], "full");
    assert_eq!(said[0]["embedded"], true);

    let made = page.ask(&execute(1, "file.new", json!({"units": "mm"})));
    assert_eq!(made["id"], 1);
    assert_eq!(made["ok"], true, "{made}");
    assert!(made["ms"].is_number(), "timed by the page's clock: {made}");
    let block = page.ask(&execute(
        "b",
        "feature.block",
        json!({"name": "Block", "size": [10, 20, 30]}),
    ));
    assert_eq!(block["id"], "b");
    assert_eq!(block["ok"], true, "{block}");
    let measured = page.ask(&execute(3, "document.measure", json!({})));
    assert_eq!(measured["id"], 3);
    assert!((volume(&measured) - 6000.0).abs() < 1e-6, "{measured}");
    // An error is an error, under its id, with what to do.
    let wrong = page.ask(&execute(4, "feature.blok", json!({})));
    assert_eq!(wrong["id"], 4);
    assert_eq!(wrong["ok"], false);
    assert!(
        wrong["error"].as_str().unwrap().contains("feature.block"),
        "{wrong}"
    );
    // The window says what the page did, as it says an agent's commands.
    let said = page.host.shell.last_command.clone().unwrap();
    assert_eq!(said.label, "feature.blok");
    assert!(!said.ok);
    // The methods: the protocol's, the file, the window's state.
    let methods = page.ask(r#"{"id": 5, "method": "methods"}"#);
    let names: Vec<&str> = methods["result"]["methods"]
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
            GIVE,
            "ui.get",
            "ui.set"
        ]
    );
    // A batch, the protocol's own.
    let batch = page.ask(
        &json!({"id": 6, "method": "batch", "params": {"steps": [
            {"command": "variable.set", "params": {"name": "t", "expr": "2 mm"}},
            {"command": "document.measure"}
        ]}})
        .to_string(),
    );
    assert_eq!(batch["result"]["completed"], 2, "{batch}");
    // The document was rebuilt after the changes: said once per state.
    page.frame();
    let events = page.replies();
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0]["event"], "regenerated");
    assert_eq!(events[0]["document"], 0);
    let revision = events[0]["revision"].as_u64().unwrap();
    assert_eq!(Some(revision), page.host.session.document_revision());
    page.frames(3);
    assert!(page.replies().is_empty(), "nothing changed, nothing said");
}

#[test]
fn hostile_messages_are_errors_and_nothing_panics() {
    let mut page = Page::alone();
    page.ask(&execute(1, "file.new", json!({})));
    let long = format!(
        r#"{{"id": 9, "method": "methods", "params": {{"x": "{}"}}}}"#,
        "a".repeat(MAX_REQUEST_BYTES)
    );
    for (line, id) in [
        ("not json", Value::Null),
        ("[1, 2]", Value::Null),
        ("5", Value::Null),
        ("null", Value::Null),
        (r#"{"id": 1}"#, json!(1)),
        (r#"{"id": 2, "method": 5}"#, json!(2)),
        (r#"{"id": 3, "method": "ui.quit"}"#, json!(3)),
        (
            r#"{"id": 4, "method": "engine.execute", "params": "x"}"#,
            json!(4),
        ),
        (
            r#"{"id": 5, "method": "engine.execute", "params": {"command": 7}}"#,
            json!(5),
        ),
        (
            r#"{"id": 6, "method": "engine.execute", "params": {"command": "file.new"}, "x": 1}"#,
            json!(6),
        ),
        (
            r#"{"id": 7, "method": "embed.give", "params": {"name": "a.3dcraft"}}"#,
            json!(7),
        ),
        (r#"{"id": 8, "method": "embed.give"}"#, json!(8)),
        (
            r#"{"id": [1], "method": "ui.set", "params": {"presentation": "none"}}"#,
            json!([1]),
        ),
        (r#"{"id": 10, "method": "ui.set", "params": 3}"#, json!(10)),
        (&long, Value::Null),
    ] {
        let reply = page.ask(line);
        assert_eq!(reply["ok"], false, "{line}: {reply}");
        assert_eq!(reply["id"], id, "{line}: {reply}");
        assert!(
            reply["error"].as_str().is_some_and(|e| !e.is_empty()),
            "{reply}"
        );
    }
    // A blank line gets no reply, as on every transport.
    page.send("   ");
    assert!(page.replies().is_empty());
    // The file's own errors.
    for (params, bytes, says) in [
        (
            json!({"name": "a.3dcraft"}),
            Some(b"not a document".to_vec()),
            "file.open",
        ),
        (
            json!({"name": "a.exe"}),
            Some(vec![1, 2, 3]),
            "not a 3DCraft document",
        ),
        (json!({"name": 7}), Some(vec![1]), "must be a string"),
        (json!({}), Some(vec![1]), "missing"),
        (
            json!({"name": "a.step", "path": "x"}),
            Some(vec![1]),
            "unknown parameter",
        ),
        (
            json!({"name": "../../../../etc/passwd.step"}),
            Some(b"ISO-10303-21;".to_vec()),
            "file.import",
        ),
        (
            json!({"name": "x".repeat(5000) + ".3dcraft"}),
            Some(vec![1]),
            "longer",
        ),
    ] {
        let line = json!({"id": "f", "method": GIVE, "params": params}).to_string();
        page.send_file(&line, bytes.unwrap());
        let got = page.replies();
        assert_eq!(got.len(), 1, "{line}: {got:?}");
        assert_eq!(got[0]["ok"], false, "{line}: {}", got[0]);
        assert_eq!(got[0]["id"], "f");
        assert!(
            got[0]["error"].as_str().unwrap().contains(says),
            "{says}: {}",
            got[0]
        );
    }
    // Bytes with a request that takes none, and a file over the cap.
    page.send_file(&execute("x", "document.measure", json!({})), vec![1, 2]);
    let got = page.replies();
    assert!(
        got[0]["error"].as_str().unwrap().contains("bytes"),
        "{got:?}"
    );
    page.send_file(
        &json!({"id": "big", "method": GIVE, "params": {"name": "a.step"}}).to_string(),
        vec![0; MAX_FILE_BYTES + 1],
    );
    let got = page.replies();
    assert_eq!(got[0]["id"], "big");
    assert!(
        got[0]["error"].as_str().unwrap().contains("at most"),
        "{got:?}"
    );
    // Nothing of it stays in memory, and the page goes on.
    assert!(page.host.session.file_names().is_empty());
    let after = page.ask(&execute(11, "feature.block", json!({"size": [1, 2, 3]})));
    assert_eq!(after["ok"], true, "{after}");
}

#[test]
fn files_go_in_and_come_out_as_bytes() {
    let (document, step) = block_files();
    let mut page = Page::alone();
    page.send_file(
        &json!({"id": "g", "method": GIVE, "params": {"name": "block.3dcraft"}}).to_string(),
        document,
    );
    let got = page.replies();
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0]["id"], "g");
    assert_eq!(got[0]["ok"], true, "{}", got[0]);
    let status = page.host.session.document_status().unwrap();
    assert_eq!(status.path.as_deref(), Some("block.3dcraft"));
    assert!(
        page.host.session.file_names().is_empty(),
        "the file given is not kept in memory"
    );
    // An export comes back with its reply, as bytes, not as a download.
    page.send(execute("e", "file.export", json!({"path": "block.stl"})));
    let mut got = page.take();
    got.retain(|(json, _)| json.get("event").is_none());
    let (reply, files) = got.remove(0);
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(files.len(), 1, "{reply}");
    assert_eq!(files[0].0, "block.stl");
    assert!(
        files[0].1.len() > 84,
        "a binary STL: header, count, triangles"
    );
    // Saved, the document comes back as a `.3dcraft` file that opens.
    page.send(execute("s", "file.save", json!({"path": "copy.3dcraft"})));
    let (reply, files) = page.take().remove(0);
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].0, "copy.3dcraft");
    let mut check = Session::new(Workspace::memory());
    check.put_file("copy.3dcraft", files[0].1.clone()).unwrap();
    check
        .execute("file.open", &json!({"path": "copy.3dcraft"}))
        .unwrap();
    let measured = check.execute("document.measure", &json!({})).unwrap();
    assert!((measured["volume"].as_f64().unwrap() - 6000.0).abs() < 1e-6);
    assert!(page.host.session.file_names().is_empty());
    // A STEP file given is imported into a new document of its own.
    let mut page = Page::alone();
    page.send_file(
        &json!({"id": "s", "method": GIVE, "params": {"name": "block.step"}}).to_string(),
        step,
    );
    let got = page.replies();
    assert_eq!(got[0]["ok"], true, "{}", got[0]);
    let measured = page.ask(&execute(2, "document.measure", json!({})));
    assert!((volume(&measured) - 6000.0).abs() < 1e-3, "{measured}");
}

#[test]
fn a_command_run_in_the_worker_is_answered_when_its_answer_comes() {
    let (document, _) = block_files();
    let mut page = Page::with_worker();
    page.embed.greet(&page.host);
    page.send_file(
        &json!({"id": "g", "method": GIVE, "params": {"name": "block.3dcraft"}}).to_string(),
        document,
    );
    let got = page.replies();
    assert_eq!(got[0]["event"], "ready");
    assert_eq!(got[1]["id"], "g");
    assert_eq!(got[1]["ok"], true, "{}", got[1]);
    // Not shown yet: the worker builds it. Then it is, and said once.
    assert!(page.host.away());
    page.frames(2);
    let events = page.replies();
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0]["event"], "regenerated");
    let work = page.host.session.work_here();
    // A measure needs the bodies: run in the worker, answered when it
    // answers, under its id, and nothing regenerated on the page.
    page.send(execute(7, "document.measure", json!({})));
    assert!(page.replies().is_empty(), "no reply before the answer");
    // Posted in the next frame, answered in the one after.
    page.frame();
    assert!(page.replies().is_empty(), "no reply before the answer");
    page.frame();
    let got = page.replies();
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0]["id"], 7);
    assert!((volume(&got[0]) - 6000.0).abs() < 1e-6, "{}", got[0]);
    assert!(got[0]["ms"].as_f64().unwrap() >= 16.0, "{}", got[0]);
    assert_eq!(
        page.host.session.work_here(),
        work,
        "the page built nothing"
    );
    assert!(page.worker.commands.get() >= 1);
    // An export run there comes back with its files.
    page.send(execute("e", "file.export", json!({"path": "block.3mf"})));
    page.frames(2);
    let mut got = page.take();
    got.retain(|(json, _)| json.get("event").is_none());
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].0["id"], "e");
    assert_eq!(got[0].0["ok"], true, "{}", got[0].0);
    assert_eq!(got[0].1.len(), 1);
    assert_eq!(got[0].1[0].0, "block.3mf");
    assert!(got[0].1[0].1.starts_with(b"PK"), "a 3MF is a ZIP");
}

#[test]
fn requests_behind_an_edit_made_in_the_worker_wait_and_run_in_order() {
    let (document, _) = block_files();
    let mut page = Page::with_worker();
    page.send_file(
        &json!({"id": "g", "method": GIVE, "params": {"name": "block.3dcraft"}}).to_string(),
        document,
    );
    page.frames(2);
    page.replies();
    // An assertion is an edit that measures: made where the bodies are.
    page.send(execute("a", "assert.add", assertion("the block")));
    assert!(page.host.session.edits_waiting());
    page.send(execute("r", "file.list", json!({})));
    page.send(r#"{"id": "u", "method": "ui.get"}"#);
    assert_eq!(page.embed.waiting(), 2);
    assert!(page.replies().is_empty());
    page.frames(3);
    let got = page.replies();
    let order: Vec<Value> = got
        .iter()
        .map(|r| r.get("id").cloned().unwrap_or_else(|| r["event"].clone()))
        .collect();
    assert_eq!(
        order,
        [json!("a"), json!("r"), json!("u"), json!("regenerated")],
        "{got:?}"
    );
    assert_eq!(got[0]["ok"], true, "{}", got[0]);
    assert_eq!(got[0]["result"]["status"], "passed", "{}", got[0]);
    assert_eq!(got[1]["ok"], true);
    assert_eq!(got[2]["ok"], true);
    // A flood behind an edit is capped: one more is refused, at once.
    page.send(execute("a2", "assert.add", assertion("again")));
    for n in 0..MAX_WAITING {
        page.send(execute(n, "file.list", json!({})));
    }
    assert_eq!(page.embed.waiting(), MAX_WAITING);
    page.send(execute("over", "file.list", json!({})));
    let got = page.replies();
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0]["id"], "over");
    assert!(
        got[0]["error"].as_str().unwrap().contains("too many"),
        "{}",
        got[0]
    );
    page.frames(3);
    let got = page.replies();
    assert_eq!(
        got.iter().filter(|r| r.get("id").is_some()).count(),
        MAX_WAITING + 1
    );
    assert_eq!(page.embed.waiting(), 0);
}

#[test]
fn an_edit_of_the_page_cancelled_in_the_window_is_answered_as_cancelled() {
    let (document, _) = block_files();
    let mut page = Page::with_worker();
    page.send_file(
        &json!({"id": "g", "method": GIVE, "params": {"name": "block.3dcraft"}}).to_string(),
        document,
    );
    page.frames(2);
    page.replies();
    page.worker.held.set(true);
    page.send(execute("a", "assert.add", assertion("slow")));
    page.frame();
    assert!(page.replies().is_empty());
    // Cancel, in the small window that says the edit waits.
    page.host.cancel_edits();
    page.frame();
    let got = page.replies();
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0]["id"], "a");
    assert_eq!(got[0]["ok"], false);
    assert!(
        got[0]["error"].as_str().unwrap().contains("cancelled"),
        "{}",
        got[0]
    );
}

#[test]
fn the_presentation_is_said_set_by_a_message_and_waits_for_the_window() {
    let mut page = Page::alone();
    page.host.shell.state.presentation = Presentation::View;
    page.embed.greet(&page.host);
    page.collect();
    assert_eq!(page.replies()[0]["presentation"], "view");
    let set = page.ask(r#"{"id": 1, "method": "ui.set", "params": {"presentation": "form"}}"#);
    assert_eq!(set["ok"], true, "{set}");
    assert_eq!(page.host.shell.state.presentation, Presentation::Form);
    let got = page.ask(r#"{"id": 2, "method": "ui.get"}"#);
    assert_eq!(got["result"]["state"]["presentation"], "form");
    let refused = page.ask(r#"{"id": 3, "method": "ui.set", "params": {"presentation": "none"}}"#);
    assert_eq!(refused["ok"], false);
    assert_eq!(page.host.shell.state.presentation, Presentation::Form);
    // While the window does not show the document yet (the worker builds
    // it), a ui.set waits; ui.get says what the window shows.
    let (document, _) = block_files();
    let mut page = Page::with_worker();
    page.send_file(
        &json!({"id": "g", "method": GIVE, "params": {"name": "block.3dcraft"}}).to_string(),
        document,
    );
    page.replies();
    assert!(page.host.away());
    page.send(r#"{"id": "s", "method": "ui.set", "params": {"presentation": "view"}}"#);
    assert_eq!(page.embed.waiting(), 1);
    page.frames(3);
    let got = page.replies();
    let set = got.iter().find(|r| r["id"] == "s").unwrap();
    assert_eq!(set["ok"], true, "{set}");
    assert_eq!(page.host.shell.state.presentation, Presentation::View);
}

/// Sixty changes of one parameter, as a slider dragged for a second sends
/// them, two frames apart (the browser's measure is in the dev log of
/// 2026-11-14): `param.set` is made on the page at once (its regeneration
/// comes after, `deferred: true` without a job), so each is answered in
/// order as it comes; the worker is given the newest state each frame,
/// the one it was building replaced, so the states between are not all
/// built; the last state is the one shown, and said regenerated once.
#[test]
fn sixty_changes_of_a_slider_are_answered_in_order_and_the_newest_is_shown() {
    let (document, _) = block_files();
    let mut page = Page::with_worker();
    page.send_file(
        &json!({"id": "g", "method": GIVE, "params": {"name": "block.3dcraft"}}).to_string(),
        document,
    );
    page.frames(2);
    page.replies();
    let scenes = page.worker.scenes.get();
    let mut got = Vec::new();
    for n in 0..60 {
        let width = 10 + n;
        page.send(execute(
            n,
            "param.set",
            json!({"values": {"width": format!("{width} mm")}, "merge": n > 0, "clamp": true}),
        ));
        got.extend(page.replies());
        if n % 2 == 1 {
            page.frame();
            got.extend(page.replies());
        }
    }
    page.frames(3);
    got.extend(page.replies());
    let ids: Vec<i64> = got.iter().filter_map(|r| r["id"].as_i64()).collect();
    assert_eq!(ids, (0..60).collect::<Vec<_>>(), "in order, each once");
    assert!(
        got.iter()
            .filter(|r| r.get("id").is_some())
            .all(|r| r["ok"] == true),
        "{got:?}"
    );
    let regenerated: Vec<&Value> = got.iter().filter(|r| r["event"] == "regenerated").collect();
    let last = regenerated.last().unwrap();
    assert_eq!(
        last["revision"].as_u64(),
        page.host.session.document_revision()
    );
    let built = page.worker.scenes.get() - scenes;
    assert!(built <= 31, "at most one state a frame: {built}");
    // One undo step for the drag (merge).
    assert_eq!(
        page.host.session.document_status().unwrap().undo.as_deref(),
        Some("Set width")
    );
    let measured = page.answer(&execute("m", "document.measure", json!({})));
    assert!(
        (volume(&measured) - 69.0 * 600.0).abs() < 1e-6,
        "{measured}"
    );
    eprintln!(
        "sixty param.set two frames apart: {} replies, {} regenerated, {built} scenes built",
        ids.len(),
        regenerated.len()
    );
}
