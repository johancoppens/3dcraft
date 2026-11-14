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
use crate::embed::{
    BATCH_REFUSED, Embed, GIVE, MAX_FILE_BYTES, MAX_WAITING, Message, Out, PROTOCOL, TAKE,
    WAIT_REFUSED,
};
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
    /// What the window's own exports downloaded (`WebHost::answered`).
    downloads: Vec<crate::host::Download>,
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
        // As `page.rs` has it: in the browser the page decides how much
        // of the window is drawn.
        host.shell.allow_presentations(true);
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
            downloads: Vec::new(),
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
                let outcome = self.host.answered(done.commands);
                self.downloads.extend(outcome.downloads);
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
    assert_eq!(said[0]["warnings"], json!([]));
    // What the page's options said that was not taken is in `ready`.
    let mut warned = Page::alone();
    warned
        .embed
        .set_warnings(vec!["`webgel` is not an option".into()]);
    warned.embed.greet(&warned.host);
    warned.collect();
    assert_eq!(
        warned.replies()[0]["warnings"],
        json!(["`webgel` is not an option"])
    );

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
            TAKE,
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

/// The block opened through the page with its worker, shown and said
/// regenerated; nothing left to read.
fn opened_block() -> Page {
    let (document, _) = block_files();
    let mut page = Page::with_worker();
    page.send_file(
        &json!({"id": "g", "method": GIVE, "params": {"name": "block.3dcraft"}}).to_string(),
        document,
    );
    for _ in 0..10 {
        page.frame();
        if !page.host.away() && !page.host.session.commands_waiting() {
            break;
        }
    }
    page.frames(2);
    page.take();
    page
}

fn names(files: &Files) -> Vec<&str> {
    files.iter().map(|(name, _)| name.as_str()).collect()
}

/// B1 of the review of 2026-11-14: a read and an export sent together and
/// answered in the same frame gave the export's file to the read's reply.
/// Each reply carries exactly the files its own command wrote; the
/// window's own export made at the same time keeps its file too; and
/// what a command on the page writes goes with its reply only.
#[test]
fn each_reply_carries_the_files_its_own_command_wrote_and_no_others() {
    let mut page = opened_block();
    page.send(execute(1, "document.measure", json!({})));
    page.send(execute(2, "file.export", json!({"path": "box.stl"})));
    // The window's own export, from its menu, made in the worker as well.
    assert!(page.host.export_as("mine.stl").is_empty());
    page.frames(3);
    let got = page.take();
    let reply = |id: i64| got.iter().find(|(r, _)| r["id"] == id).unwrap();
    assert_eq!(reply(1).0["ok"], true, "{}", reply(1).0);
    assert!(
        reply(1).1.is_empty(),
        "the read's reply: {:?}",
        names(&reply(1).1)
    );
    assert_eq!(reply(2).0["ok"], true, "{}", reply(2).0);
    assert_eq!(names(&reply(2).1), ["box.stl"]);
    let downloaded: Vec<&str> = page.downloads.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(downloaded, ["mine.stl"], "the window's own export");
    // On the page, at once: a save and a list sent together.
    page.send(execute(3, "file.save", json!({"path": "copy.3dcraft"})));
    page.send(execute(4, "file.list", json!({})));
    let got = page.take();
    let reply = |id: i64| got.iter().find(|(r, _)| r["id"] == id).unwrap();
    assert_eq!(names(&reply(3).1), ["copy.3dcraft"]);
    assert!(reply(4).1.is_empty());
    assert!(page.host.session.file_names().is_empty());
}

/// B2 of the review of 2026-11-14: `document.inspect` answered by the
/// worker said the worker's copy's history (`["state"]`) and no path.
/// What a read says of the document is the page's own session's: its
/// place, its file, its unsaved mark, its undo history; and no other read
/// run in the worker says anything of them that the session would not.
#[test]
fn what_a_read_says_of_the_document_is_the_pages_own_session() {
    let (document, _) = block_files();
    let mut page = Page::with_worker();
    // Another document first, so that the block is the second.
    page.send(execute("n", "file.new", json!({"units": "mm"})));
    page.send_file(
        &json!({"id": "g", "method": GIVE, "params": {"name": "block.3dcraft"}}).to_string(),
        document,
    );
    page.frames(3);
    for (id, command, params) in [
        ("w1", "param.set", json!({"values": {"width": "12 mm"}})),
        ("w2", "param.set", json!({"values": {"width": "14 mm"}})),
        ("s", "file.save", json!({"path": "kept.3dcraft"})),
        ("w3", "param.set", json!({"values": {"width": "16 mm"}})),
        ("u", "edit.undo", json!({})),
    ] {
        let reply = page.answer(&execute(id, command, params));
        assert_eq!(reply["ok"], true, "{id}: {reply}");
    }
    page.frames(3);
    let asked = page.worker.commands.get();
    let reads = [
        ("document.inspect", json!({"measures": false})),
        ("document.measure", json!({})),
        ("document.check", json!({})),
        ("document.edges", json!({"feature": "Block"})),
        ("assert.list", json!({})),
        ("select.get", json!({})),
        ("select.query", json!({"kind": "face"})),
        ("document.interference", json!({})),
    ];
    let mut there = Vec::new();
    for (n, (command, params)) in reads.iter().enumerate() {
        let reply = page.answer(&execute(n as i64, command, params.clone()));
        assert_eq!(reply["ok"], true, "{command}: {reply}");
        there.push(reply["result"].clone());
    }
    assert!(
        page.worker.commands.get() > asked,
        "document.inspect at least was run in the worker"
    );
    // The same reads on the page's own session (here, after: they
    // regenerate it).
    let fields = [
        "document", "path", "dirty", "history", "title", "file", "jigInUse",
    ];
    for ((command, params), there) in reads.iter().zip(&there) {
        let here = page.host.session.execute(command, params).unwrap();
        for field in fields {
            assert_eq!(
                there.get(field),
                here.get(field),
                "{command}: `{field}` through the page is not the session's"
            );
        }
    }
    let inspect = &there[0];
    assert_eq!(inspect["document"], 1);
    assert_eq!(inspect["path"], "kept.3dcraft");
    assert_eq!(inspect["dirty"], false, "undone back to what was saved");
    let undo = inspect["history"]["undo"].as_array().unwrap();
    assert!(
        undo.len() >= 2 && undo.iter().all(|l| l != "state"),
        "{inspect}"
    );
    assert_eq!(inspect["history"]["redo"], json!(["Set width"]));
}

/// The developer's decision of 2026-11-14 (5): what would hold the page's
/// own thread is refused for a page, naming engine.execute: a batch step
/// that needs the bodies, a command sent with `wait: true` that needs
/// them. A batch of what needs none runs; nothing is regenerated on the
/// page.
#[test]
fn a_batch_step_or_a_wait_that_needs_the_bodies_is_refused_for_a_page() {
    let mut page = opened_block();
    let added = page.answer(&execute("a", "assert.add", assertion("the block")));
    assert_eq!(added["ok"], true, "{added}");
    page.frames(3);
    // A state whose bodies the page does not have (an edit made on it): a
    // command on it needs them built.
    let set = page.answer(&execute(
        "p",
        "param.set",
        json!({"values": {"width": "11 mm"}}),
    ));
    assert_eq!(set["ok"], true, "{set}");
    assert!(!page.host.session.regenerated_here());
    let work = page.host.session.work_here();
    let batch = page.answer(
        &json!({"id": "b", "method": "batch", "params": {"steps": [
            {"command": "file.list"},
            {"command": "document.measure"},
        ]}})
        .to_string(),
    );
    assert_eq!(batch["ok"], true, "{batch}");
    assert_eq!(batch["result"]["completed"], 1, "{batch}");
    assert_eq!(batch["result"]["failed"], 1, "{batch}");
    let error = batch["result"]["results"][1]["error"].as_str().unwrap();
    assert!(
        error.contains("engine.execute") && error.contains(BATCH_REFUSED),
        "{error}"
    );
    let fine = page.answer(
        &json!({"id": "c", "method": "batch", "params": {"steps": [
            {"command": "file.list"},
            {"command": "param.list"},
        ]}})
        .to_string(),
    );
    assert_eq!(fine["result"]["completed"], 2, "{fine}");
    let waited = page.answer(&execute(
        "w",
        "param.set",
        json!({"values": {"width": "12 mm"}, "wait": true}),
    ));
    assert_eq!(waited["ok"], false, "{waited}");
    let error = waited["error"].as_str().unwrap();
    assert!(
        error.contains("engine.execute") && error.contains(WAIT_REFUSED),
        "{error}"
    );
    let set = page.answer(&execute(
        "s",
        "param.set",
        json!({"values": {"width": "12 mm"}}),
    ));
    assert_eq!(set["ok"], true, "{set}");
    assert_eq!(
        page.host.session.work_here(),
        work,
        "the page built nothing"
    );
}

/// The review of 2026-11-14: `regenerated` never came after an edit that
/// changes nothing, and a reply carried no revision. A reply that ran a
/// command says the state it left; `regenerated` follows for it (or a
/// later one) exactly when that state was not said already.
#[test]
fn a_reply_says_the_state_it_left_and_whether_regenerated_follows() {
    let mut page = opened_block();
    let wait_for = |page: &mut Page, document: i64, revision: u64| -> bool {
        for _ in 0..10 {
            page.frame();
            if page.replies().iter().any(|e| {
                e["event"] == "regenerated"
                    && e["document"] == document
                    && e["revision"].as_u64().is_some_and(|r| r >= revision)
            }) {
                return true;
            }
        }
        false
    };
    // An edit that changes the document.
    let set = page.ask(&execute(
        1,
        "param.set",
        json!({"values": {"width": "20 mm"}}),
    ));
    let state = set["state"].clone();
    assert_eq!(state["document"], 0, "{set}");
    assert_eq!(state["regenerated"], false, "{set}");
    let revision = state["revision"].as_u64().unwrap();
    assert!(
        wait_for(&mut page, 0, revision),
        "regenerated for {revision}"
    );
    // The same value again: nothing changes, and the reply says no event
    // follows for it.
    let same = page.ask(&execute(
        2,
        "param.set",
        json!({"values": {"width": "20 mm"}}),
    ));
    assert_eq!(same["ok"], true, "{same}");
    if same["state"]["revision"].as_u64() == Some(revision) {
        assert_eq!(same["state"]["regenerated"], true, "{same}");
        page.frames(3);
        assert!(!page.replies().iter().any(|e| e["event"] == "regenerated"));
    } else {
        let again = same["state"]["revision"].as_u64().unwrap();
        assert!(wait_for(&mut page, 0, again));
    }
    // A read: the state as it is, said already.
    let read = page.ask(&execute(3, "file.list", json!({})));
    assert_eq!(read["state"]["regenerated"], true, "{read}");
    // A new document: its state, and the event for it.
    let made = page.ask(&execute(4, "file.new", json!({"units": "mm"})));
    assert_eq!(made["state"]["document"], 1, "{made}");
    let fresh = made["state"]["revision"].as_u64().unwrap();
    if made["state"]["regenerated"] == false {
        assert!(wait_for(&mut page, 1, fresh), "regenerated after file.new");
    }
    // An undo back to a state said before: said again.
    page.ask(&execute(5, "file.activate", json!({"document": 0})));
    page.frames(3);
    page.replies();
    let undone = page.ask(&execute(6, "edit.undo", json!({})));
    assert_eq!(undone["state"]["regenerated"], false, "{undone}");
    let after = undone["state"]["revision"].as_u64().unwrap();
    assert!(wait_for(&mut page, 0, after), "regenerated after the undo");
    // Without a document: no state.
    page.ask(&execute(7, "file.close", json!({"discard": true})));
    let closed = page.ask(&execute(8, "file.close", json!({"discard": true})));
    assert_eq!(closed["state"], Value::Null, "{closed}");
}

/// The review of 2026-11-14: a read that needs the bodies was lost when
/// an edit made on the page came behind it (a price that follows a
/// slider: 0 of 60 measures). Each read is answered on the state it was
/// sent for.
#[test]
fn a_read_is_answered_on_the_state_it_was_sent_for() {
    let mut page = opened_block();
    for n in 0..6 {
        page.send(execute(
            format!("p{n}"),
            "param.set",
            json!({"values": {"width": format!("{} mm", 10 + n)}, "merge": n > 0}),
        ));
        page.send(execute(format!("m{n}"), "document.measure", json!({})));
    }
    let mut got = Vec::new();
    for _ in 0..20 {
        page.frame();
        got.extend(page.replies());
    }
    for n in 0..6 {
        let measured = got.iter().find(|r| r["id"] == format!("m{n}")).unwrap();
        assert_eq!(measured["ok"], true, "{measured}");
        let want = f64::from(10 + n) * 600.0;
        assert!((volume(measured) - want).abs() < 1e-6, "m{n}: {measured}");
    }
    assert!(page.host.session.file_names().is_empty());
}

/// `embed.take`: the document's own file, and the document as it was
/// (its name, its file, its unsaved mark), where `file.save {path}` names
/// the document after the file and marks it saved.
#[test]
fn take_gives_the_documents_file_and_leaves_the_document_as_it_was() {
    let (document, _) = block_files();
    let mut page = Page::alone();
    let none = page.ask(&json!({"id": 0, "method": TAKE}).to_string());
    assert_eq!(none["ok"], false, "{none}");
    page.send_file(
        &json!({"id": "g", "method": GIVE, "params": {"name": "block.3dcraft"}}).to_string(),
        document,
    );
    page.take();
    page.ask(&execute(
        1,
        "param.set",
        json!({"values": {"width": "30 mm"}}),
    ));
    let before = page.host.session.document_status().unwrap();
    assert!(before.dirty);
    page.send(json!({"id": 2, "method": TAKE}).to_string());
    let (reply, files) = page.take().remove(0);
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(names(&files), ["block.3dcraft"]);
    assert_eq!(reply["result"]["bytes"], files[0].1.len());
    let after = page.host.session.document_status().unwrap();
    assert_eq!(after.path, before.path);
    assert!(after.dirty, "still unsaved");
    let mut check = Session::new(Workspace::memory());
    check.put_file("x.3dcraft", files[0].1.clone()).unwrap();
    check
        .execute("file.open", &json!({"path": "x.3dcraft"}))
        .unwrap();
    let measured = check.execute("document.measure", &json!({})).unwrap();
    assert!((measured["volume"].as_f64().unwrap() - 18000.0).abs() < 1e-6);
    page.send(json!({"id": 3, "method": TAKE, "params": {"name": "../mine.3dcraft"}}).to_string());
    let (reply, files) = page.take().remove(0);
    assert_eq!(reply["result"]["name"], "mine.3dcraft", "{reply}");
    assert_eq!(names(&files), ["mine.3dcraft"]);
    let bad = page.ask(&json!({"id": 4, "method": TAKE, "params": {"path": "x"}}).to_string());
    assert!(
        bad["error"].as_str().unwrap().contains("unknown parameter"),
        "{bad}"
    );
    assert!(page.host.session.file_names().is_empty());
}
