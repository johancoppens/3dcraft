//! Seeded fuzzes of Embed's core (`crate::embed`): the messages a page can
//! send. Written by the review of 2026-11-14 (`check/review-embed-core`,
//! 2af51ab1) and adopted here, with what it counted made assertions: each
//! export's reply carries its own file and no other reply carries one
//! (B1), what `document.inspect` says of the undo history is the page's
//! own session's (B2), and no read is lost to an edit sent behind it.
//!
//! - `garbage`: random messages (not JSON, odd envelopes, commands with
//!   random params, files that are not files), frames, a worker that holds
//!   its answers, cancels: nothing panics, every message that is not blank
//!   gets exactly one reply, the queue stays within its cap, no file stays
//!   in memory.
//! - `order`: random edits and reads of one document sent faster than they
//!   are answered, some made on the page and some in the worker: every one
//!   answered once, and the document after them is the one a plain
//!   `Session` gives for the same commands in the same order (what
//!   `craft3d-cli` runs).
//! - `cancel`: the same with the worker holding its answers and the
//!   person's Cancel in between: every request still gets exactly one
//!   reply, and the document is the one a plain `Session` gives for the
//!   edits that were answered ok, in order.
//!
//! - `slider`: written by the review of Embed's round 2 (2026-11-14,
//!   `check/review-embed-core-r2`): a slider with a price behind it, 600
//!   changes of a variable with a measure behind each and the worker held
//!   now and then. Each answered once, each measure on its own state, and
//!   (R2-2 of that review) nothing built on the page's own thread
//!   (`Session::work_here`), though more reads wait than the worker takes.
//!
//! `EMBED_FUZZ_SEEDS` sets how many seeds each runs (the defaults keep the
//! three under a minute in a debug build).

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]

use crate::checks::{Reply, Transport};
use crate::embed::{Embed, GIVE, MAX_WAITING, Message, Out};
use crate::host::WebHost;
use crate::scenes::Scenes;
use craft3d_engine::{Session, Workspace, scene_begin, scene_next};
use craft3d_ui_egui::shell::Shell;
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

/// xorshift64*: seeded, the same on every platform.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn chance(&mut self, of: usize) -> bool {
        self.below(of) == 0
    }
    fn pick<'a, T>(&mut self, from: &'a [T]) -> &'a T {
        &from[self.below(from.len())]
    }
}

enum Job {
    Scene(u64, Vec<u8>, String),
    Command(u64, Vec<u8>, String),
}

#[derive(Clone, Default)]
struct Beside {
    jobs: Rc<RefCell<Vec<Job>>>,
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
                    scene_begin(&document, &work);
                    while let Some(bytes) = scene_next() {
                        replies.push(Reply::Part { token, bytes });
                    }
                    replies.push(Reply::Done { token });
                }
                Job::Command(token, document, work) => {
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

struct Page {
    host: WebHost,
    embed: Embed,
    scenes: Option<Scenes<Beside>>,
    worker: Beside,
    clock: Rc<Cell<f64>>,
    out: Vec<Out>,
    most_waiting: usize,
}

impl Page {
    fn new(worker: bool) -> Self {
        let mut host = WebHost::new(Shell::new());
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
            most_waiting: 0,
        }
    }

    fn send(&mut self, message: Message) {
        self.embed.receive(&mut self.host, message);
        self.most_waiting = self.most_waiting.max(self.embed.waiting());
        self.out.extend(self.embed.take_out());
    }

    /// One frame, as `page.rs` has it: the worker's answers, the host's
    /// work, a restart the host asked for, the core's.
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
        while let Some(token) = self.host.session.checks_job() {
            let request = self.host.session.checks_request(token).unwrap();
            let reply = craft3d_engine::measure_checks(&request.document, &request.work);
            self.host.session.checks_answered(token, Ok(&reply));
        }
        self.embed.frame(&mut self.host);
        self.out.extend(self.embed.take_out());
        // `page.rs`'s `ui`: a restart the host asked for (a Cancel).
        if self.host.take_worker_restart()
            && let Some(scenes) = &mut self.scenes
        {
            let answered = scenes.restart(&mut self.host.session);
            self.host.answered(answered);
        }
        self.most_waiting = self.most_waiting.max(self.embed.waiting());
    }

    /// Frames until nothing waits any more (the worker answering; a pair
    /// of an edit made there and a read takes two frames of this worker,
    /// and as many requests wait as the door takes in a frame).
    fn settle(&mut self) {
        self.worker.held.set(false);
        for _ in 0..4000 {
            self.frame();
            if self.embed.waiting() == 0
                && !self.host.session.commands_waiting()
                && !self.host.page_answers_waiting()
            {
                break;
            }
        }
        self.frame();
        self.frame();
    }

    /// The replies so far (no events), as JSON, with their files.
    fn take_replies(&mut self) -> Vec<(Value, usize)> {
        self.out
            .drain(..)
            .filter_map(|out| match out {
                Out::Reply { line, files } => Some((
                    serde_json::from_str(&line).unwrap_or(Value::Null),
                    files.len(),
                )),
                Out::Event(_) => None,
            })
            .collect()
    }
}

fn execute(id: impl Into<Value>, command: &str, params: Value) -> String {
    json!({"id": id.into(), "method": "engine.execute", "params": {"command": command, "params": params}})
        .to_string()
}

fn block_document() -> Vec<u8> {
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
    ] {
        session.execute(id, &params).unwrap();
    }
    session.take_file("block.3dcraft").unwrap()
}

fn random_text(rng: &mut Rng) -> String {
    let pool = [
        "",
        " ",
        "a",
        "w",
        "10 mm",
        "1e400",
        "-0",
        "NaN",
        "inf",
        "∞",
        "\u{0}",
        "\"",
        "{",
        "}",
        "[",
        "../..",
        "/etc/passwd",
        "x.3dcraft",
        "x.step",
        "x.stl",
        "Block",
        "F1",
        "F1:top",
        "width",
        "nonsense +",
        "1/0 mm",
        "sqrt(-1)",
        "😀",
        "\u{202e}",
        "a\nb",
    ];
    let mut text = String::new();
    for _ in 0..rng.below(4) {
        text.push_str(rng.pick(&pool));
    }
    if rng.chance(40) {
        text = text.repeat(1 + rng.below(3000));
    }
    text
}

fn random_value(rng: &mut Rng, depth: usize) -> Value {
    match rng.below(if depth > 3 { 6 } else { 9 }) {
        0 => Value::Null,
        1 => json!(rng.chance(2)),
        2 => json!(rng.next() as i64),
        3 => json!((rng.next() as f64) / 1e9 - 4e9),
        4 => json!(random_text(rng)),
        5 => json!(*rng.pick(&[0.0, -1.0, 1e308, -1e308, 1e-320, 4.2e9, 0.5])),
        6 => Value::Array(
            (0..rng.below(5))
                .map(|_| random_value(rng, depth + 1))
                .collect(),
        ),
        _ => {
            let keys = [
                "name",
                "expr",
                "size",
                "path",
                "values",
                "width",
                "command",
                "params",
                "steps",
                "presentation",
                "camera",
                "show",
                "id",
                "method",
                "bytes",
                "of",
                "measure",
                "eq",
                "tol",
                "document",
                "discard",
                "x",
            ];
            let mut map = serde_json::Map::new();
            for _ in 0..rng.below(5) {
                map.insert((*rng.pick(&keys)).to_string(), random_value(rng, depth + 1));
            }
            Value::Object(map)
        }
    }
}

const COMMANDS: &[&str] = &[
    "file.new",
    "file.close",
    "file.open",
    "file.save",
    "file.export",
    "file.import",
    "file.list",
    "variable.set",
    "variable.delete",
    "variable.publish",
    "param.set",
    "param.list",
    "feature.block",
    "feature.fillet",
    "feature.pattern",
    "feature.delete",
    "document.measure",
    "document.inspect",
    "document.check",
    "assert.add",
    "assert.list",
    "assert.delete",
    "edit.undo",
    "edit.redo",
    "history.rollback",
    "select.set",
    "select.get",
    "view.render",
    "jig.export",
    "jig.edit",
    "nonsense.command",
];

/// Params that are sometimes right for the command and sometimes not.
fn params_for(rng: &mut Rng, command: &str) -> Value {
    if rng.chance(3) {
        return random_value(rng, 1);
    }
    let n = 1 + rng.below(40);
    match command {
        "file.new" => json!({"units": *rng.pick(&["mm", "m", "in", "furlong"])}),
        "file.close" => json!({"discard": rng.chance(2)}),
        "file.save" | "file.export" | "view.render" | "jig.export" | "file.open"
        | "file.import" => {
            json!({"path": *rng.pick(&["a.3dcraft", "a.stl", "a.step", "a.3mf", "a.png", "../a.stl", "/a.stl", "a", "", "d/e.obj", "a.3dcraftjig", "a.dxf", "a.svg"])})
        }
        "variable.set" => {
            json!({"name": *rng.pick(&["width", "h", "t", "x y", ""]), "expr": *rng.pick(&["12 mm", "width * 2", "h + 1 mm", "3", "nonsense +", "1/0", "t", "-5 mm", "1e300 mm"])})
        }
        "variable.delete" | "variable.publish" => {
            json!({"name": *rng.pick(&["width", "h", "t", "nope"])})
        }
        "param.set" => {
            json!({"values": {"width": format!("{n} mm")}, "merge": rng.chance(2), "clamp": rng.chance(2), "wait": rng.chance(6)})
        }
        "feature.block" => {
            json!({"name": format!("B{n}"), "size": [n, *rng.pick(&[1.0, 0.0, -3.0, 1e9, 2.5]), "width"]})
        }
        "feature.fillet" => {
            json!({"edges": [[*rng.pick(&["F1:top", "F2:side", "x"]), "F1:front"]], "radius": *rng.pick(&[1.0, 500.0, 0.0, -1.0])})
        }
        "feature.pattern" => {
            json!({"source": *rng.pick(&["Block", "F1", "nope"]), "count": *rng.pick(&[2, 3, 0, 2000]), "step": [n, 0, 0]})
        }
        "feature.delete" => json!({"feature": *rng.pick(&["Block", "F1", "F2", "nope"])}),
        "history.rollback" => {
            json!({"feature": rng.pick(&[json!("F1"), json!(null), json!("nope")]).clone()})
        }
        "assert.add" => {
            json!({"name": format!("A{}", rng.below(4)), "measure": "volume", "of": *rng.pick(&["Block", "F1", "nope"]),
                   "eq": format!("{n} * (1 mm)^3"), "tol": "1e-6 * (1 mm)^3"})
        }
        "assert.delete" => json!({"name": format!("A{}", rng.below(4))}),
        "select.set" => {
            json!({"names": [*rng.pick(&["F1:top", "F1", "x"])], "mode": *rng.pick(&["replace", "add", "clear", "nope"])})
        }
        _ => json!({}),
    }
}

fn random_message(rng: &mut Rng, n: usize, document: &[u8]) -> Message {
    let id = match rng.below(12) {
        0 => Value::Null,
        1 => json!({"a": n}),
        2 => json!([n]),
        3 => json!(n as f64 + 0.5),
        4 => json!(format!("s{n}")),
        _ => json!(n),
    };
    match rng.below(20) {
        0 => Message::line(random_text(rng)),
        1 => Message::line(random_value(rng, 0).to_string()),
        2 => Message::line(format!("{{\"id\": {id}, \"method\": ")),
        3 => {
            let method = *rng.pick(&[
                "methods",
                "engine.commands",
                "ui.get",
                "ui.set",
                "nope",
                "ui.quit",
                "auth",
                "",
            ]);
            Message::line(
                json!({"id": id, "method": method, "params": random_value(rng, 1)}).to_string(),
            )
        }
        4 => {
            let patch = match rng.below(5) {
                0 => json!({"presentation": *rng.pick(&["full", "view", "form", "none"])}),
                1 => json!({"view": *rng.pick(&["iso", "top", "nope"])}),
                2 => json!({"show": {"sketches": rng.chance(2)}}),
                3 => json!({"camera": {"distance": *rng.pick(&[0.0, -1.0, 1e300, 5.0])}}),
                _ => random_value(rng, 1),
            };
            Message::line(json!({"id": id, "method": "ui.set", "params": patch}).to_string())
        }
        5 => Message::line(json!({"id": id, "method": "ui.get"}).to_string()),
        6 => {
            let steps: Vec<Value> = (0..rng.below(6))
                .map(|_| {
                    let command = *rng.pick(COMMANDS);
                    json!({"command": command, "params": params_for(rng, command)})
                })
                .collect();
            Message::line(json!({"id": id, "method": "batch", "params": {"steps": steps, "stopOnError": rng.chance(2)}}).to_string())
        }
        7 => {
            // A file: the real document, a cut one, junk, none.
            let name = *rng.pick(&[
                "block.3dcraft",
                "b.3dcraftjig",
                "c.step",
                "d.FCStd",
                "e.exe",
                "../f.3dcraft",
                "",
            ]);
            let bytes = match rng.below(5) {
                0 => Some(document.to_vec()),
                1 => Some(document[..rng.below(document.len())].to_vec()),
                2 => Some((0..rng.below(4000)).map(|_| rng.next() as u8).collect()),
                3 => Some(
                    b"ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;\n"
                        .to_vec(),
                ),
                _ => None,
            };
            Message {
                line: json!({"id": id, "method": GIVE, "params": {"name": name}}).to_string(),
                bytes,
            }
        }
        8 => Message {
            // Bytes with a request that takes none.
            line: execute(id, "file.list", json!({})),
            bytes: Some(vec![1, 2, 3]),
        },
        _ => {
            let command = *rng.pick(COMMANDS);
            Message::line(execute(id, command, params_for(rng, command)))
        }
    }
}

/// Any message, any time: no panic, one reply each, the caps kept.
#[test]
fn garbage_gets_one_reply_each_and_nothing_panics() {
    let document = block_document();
    let seeds: u64 = std::env::var("EMBED_FUZZ_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(40);
    let mut total = 0usize;
    let mut refused = 0usize;
    for seed in 0..seeds {
        let mut rng = Rng::new(seed + 1);
        let mut page = Page::new(seed % 2 == 0);
        // Half of them start on a document.
        if seed % 4 < 2 {
            page.send(Message {
                line: json!({"id": "start", "method": GIVE, "params": {"name": "block.3dcraft"}})
                    .to_string(),
                bytes: Some(document.clone()),
            });
            page.frame();
            page.frame();
            page.take_replies();
        }
        let mut sent = 0usize;
        let count = 120;
        for n in 0..count {
            let message = random_message(&mut rng, n, &document);
            if !message.line.trim().is_empty() {
                sent += 1;
            }
            page.send(message);
            match rng.below(12) {
                0..=3 => page.frame(),
                4 => page.worker.held.set(true),
                5 => page.worker.held.set(false),
                6 => {
                    page.host.cancel_edits();
                }
                _ => {}
            }
        }
        page.settle();
        let replies = page.take_replies();
        for (reply, _) in &replies {
            let ok = reply.get("ok").and_then(Value::as_bool);
            assert!(ok.is_some(), "seed {seed}: a reply without ok: {reply}");
            if ok == Some(false) {
                refused += 1;
                let error = reply.get("error").and_then(Value::as_str).unwrap_or("");
                assert!(
                    !error.is_empty(),
                    "seed {seed}: an error without text: {reply}"
                );
                assert!(
                    !error.contains("a bug in 3DCraft") && !error.contains("panicked"),
                    "seed {seed}: {error}"
                );
            }
        }
        assert_eq!(
            replies.len(),
            sent,
            "seed {seed}: {sent} messages, {} replies",
            replies.len()
        );
        assert!(page.most_waiting <= MAX_WAITING, "seed {seed}");
        assert_eq!(page.embed.waiting(), 0, "seed {seed}");
        assert!(
            page.host.session.file_names().is_empty(),
            "seed {seed}: files left in memory: {:?}",
            page.host.session.file_names()
        );
        total += sent;
    }
    eprintln!("garbage: {seeds} seeds, {total} messages, {refused} refused, none panicked");
}

/// The edits and reads of the order fuzz: `(command, params, edits)`.
fn order_command(rng: &mut Rng, n: usize) -> (&'static str, Value, bool) {
    match rng.below(16) {
        0..=3 => (
            "variable.set",
            json!({"name": *rng.pick(&["w", "d", "h"]), "expr": format!("{} mm", 5 + rng.below(40))}),
            true,
        ),
        4 => (
            "variable.set",
            json!({"name": format!("v{}", rng.below(3)), "expr": format!("{} mm", n)}),
            true,
        ),
        5 => (
            "feature.block",
            json!({"name": format!("B{n}"), "size": [1 + rng.below(5), 2, 3], "position": [100 * (n + 1), 0, 0]}),
            true,
        ),
        6 | 7 => (
            "assert.add",
            json!({"name": format!("A{n}"), "measure": "volume", "of": "Block",
                   "eq": format!("{} * (1 mm)^3", 1000 + n), "tol": "1e-6 * (1 mm)^3"}),
            true,
        ),
        8 => ("edit.undo", json!({}), true),
        9 => ("edit.redo", json!({}), true),
        10 => (
            "variable.set",
            json!({"name": "w", "expr": "nonsense +"}),
            true,
        ),
        11 | 12 => ("document.measure", json!({}), false),
        13 => ("file.export", json!({"path": format!("o{n}.stl")}), false),
        14 => ("assert.list", json!({}), false),
        _ => ("file.list", json!({}), false),
    }
}

const START: &[(&str, &str)] = &[
    ("file.new", r#"{"units": "mm"}"#),
    ("variable.set", r#"{"name": "w", "expr": "10 mm"}"#),
    ("variable.set", r#"{"name": "d", "expr": "20 mm"}"#),
    ("variable.set", r#"{"name": "h", "expr": "30 mm"}"#),
    (
        "feature.block",
        r#"{"name": "Block", "size": ["w", "d", "h"]}"#,
    ),
];

/// What of a document must be the same: its variables, its features, its
/// assertions by name and what they expect, its history and its volume.
fn fingerprint(inspect: &Value, measure: &Result<Value, String>) -> Value {
    let names = |key: &str, fields: &[&str]| -> Value {
        inspect
            .get(key)
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .map(|item| {
                        let mut map = serde_json::Map::new();
                        for field in fields {
                            map.insert(
                                (*field).to_string(),
                                item.get(*field).cloned().unwrap_or(Value::Null),
                            );
                        }
                        Value::Object(map)
                    })
                    .collect::<Vec<Value>>()
            })
            .into()
    };
    json!({
        "variables": names("variables", &["name", "expr", "value"]),
        "features": names("features", &["id", "name", "kind", "status"]),
        "assertions": names("assertions", &["name"]),
        "units": inspect.get("units").cloned(),
        "volume": match measure {
            Ok(m) => m.get("volume").and_then(Value::as_f64).map(|v| (v * 1e6).round() / 1e6).into(),
            Err(e) => Value::String(e.clone()),
        },
    })
}

fn order_run(seed: u64, cancels: bool) -> (usize, usize, usize, usize, usize) {
    let mut rng = Rng::new(seed + 1000);
    let mut page = Page::new(true);
    let mut sent: Vec<(&'static str, Value, bool)> = Vec::new();
    for (command, params) in START {
        sent.push((*command, serde_json::from_str(params).unwrap(), true));
    }
    for n in 0..60 {
        sent.push(order_command(&mut rng, n));
    }
    for (n, (command, params, _)) in sent.iter().enumerate() {
        page.send(Message::line(execute(n, command, params.clone())));
        match rng.below(if cancels { 10 } else { 7 }) {
            0 | 1 => page.frame(),
            2 => {
                page.frame();
                page.frame();
            }
            3 => page.worker.held.set(true),
            4 => page.worker.held.set(false),
            7 => {
                // The person's Cancel, as the small window has it.
                page.host.cancel_edits();
            }
            _ => {}
        }
    }
    page.settle();
    // Exactly one reply for each, by its id.
    let mut by_id: BTreeMap<usize, Vec<(Value, usize)>> = BTreeMap::new();
    for (reply, files) in page.take_replies() {
        let id = reply["id"].as_u64().unwrap_or(u64::MAX) as usize;
        by_id.entry(id).or_default().push((reply, files));
    }
    for (n, (command, _, _)) in sent.iter().enumerate() {
        let got = by_id.get(&n).map_or(0, Vec::len);
        assert_eq!(
            got, 1,
            "seed {seed}: request {n} ({command}) got {got} replies"
        );
    }
    assert_eq!(
        by_id.len(),
        sent.len(),
        "seed {seed}: a reply under an id never sent"
    );
    // The same commands on a plain session, in the same order; with
    // cancels, only the edits that were answered ok.
    let mut plain = Session::new(Workspace::memory());
    let (mut lost_reads, mut cancelled, mut differ, mut misfiled) =
        (0usize, 0usize, 0usize, 0usize);
    // B1: no reply but an export's carries a file.
    for (m, got) in &by_id {
        assert!(
            got[0].1 == 0 || sent[*m].0 == "file.export",
            "seed {seed}: request {m} ({}) carries {} file(s): {}",
            sent[*m].0,
            got[0].1,
            got[0].0
        );
    }
    for (n, (command, params, edits)) in sent.iter().enumerate() {
        let (reply, files) = &by_id[&n][0];
        let ok = reply["ok"] == true;
        let error = reply["error"].as_str().unwrap_or("");
        assert!(
            !error.contains("a bug in 3DCraft") && !error.contains("panicked"),
            "seed {seed}: request {n} ({command}): {error}"
        );
        if !*edits {
            // A read: its answer, or that the document changed before it came.
            let here = plain.execute(command, params);
            let _ = plain.take_file(params["path"].as_str().unwrap_or(""));
            if ok {
                if *command == "document.measure" {
                    let a = reply["result"]["volume"].as_f64();
                    let b = here.as_ref().ok().and_then(|v| v["volume"].as_f64());
                    assert!(
                        matches!((a, b), (Some(a), Some(b)) if (a - b).abs() <= 1e-6 * b.abs().max(1.0)),
                        "seed {seed}: request {n}: the measure is {a:?}, a plain session's {b:?}"
                    );
                }
                // B1: an export's reply carries its own file, one.
                if *command == "file.export" && *files != 1 {
                    misfiled += 1;
                }
            } else if !cancels && error.contains("changed before its answer came") {
                // A read keeps the state it was sent for.
                panic!(
                    "seed {seed}: request {n} ({command}) was lost to an edit behind it: {error}"
                );
            } else if error.contains("cancelled")
                || error.contains("was ended when an edit was cancelled")
            {
                lost_reads += 1;
            } else {
                assert!(
                    here.is_err(),
                    "seed {seed}: request {n} ({command}) failed on the page ({error}) and not on a plain session"
                );
            }
            continue;
        }
        let was_cancelled = error.contains("cancelled");
        if was_cancelled {
            assert!(
                cancels,
                "seed {seed}: request {n} cancelled without a Cancel: {error}"
            );
            cancelled += 1;
            continue;
        }
        let here = plain.execute(command, params);
        if here.is_ok() != ok {
            panic!(
                "seed {seed}: request {n} ({command} {params}): the page said {reply}, a plain session {here:?}"
            );
        }
    }
    let inspect = execute("i", "document.inspect", json!({"measures": false}));
    let measure = execute("m", "document.measure", json!({}));
    page.send(Message::line(inspect));
    page.send(Message::line(measure));
    page.settle();
    let after = page.take_replies();
    let find = |id: &str| {
        after
            .iter()
            .find(|(r, _)| r["id"] == id)
            .map(|(r, _)| r.clone())
            .unwrap_or_else(|| panic!("seed {seed}: no reply to {id}"))
    };
    let (page_inspect, page_measure) = (find("i"), find("m"));
    let page_measure = if page_measure["ok"] == true {
        Ok(page_measure["result"].clone())
    } else {
        Err("no measure".to_string())
    };
    let plain_inspect = plain
        .execute("document.inspect", &json!({"measures": false}))
        .unwrap();
    let plain_measure = plain
        .execute("document.measure", &json!({}))
        .map_err(|_| "no measure".to_string());
    assert_eq!(
        fingerprint(&page_inspect["result"], &page_measure),
        fingerprint(&plain_inspect, &plain_measure),
        "seed {seed}: the document after the page's commands is not the plain session's"
    );
    // What `document.inspect` says of the undo history, and what the
    // page's own session holds (its status says the step Undo takes).
    // B2: what `document.inspect` says of the undo history is the page's
    // own session's, which is the plain session's.
    let said = page_inspect["result"]["history"].clone();
    let plain_said = plain_inspect["history"].clone();
    if said != plain_said {
        differ += 1;
    }
    assert_eq!(
        said, plain_said,
        "seed {seed}: document.inspect through the page says another history than the session's"
    );
    assert_eq!(
        misfiled, 0,
        "seed {seed}: {misfiled} exports answered ok without their file"
    );
    // Undo is the page's own all the same: the step it would take is the
    // plain session's.
    assert_eq!(
        page.host.session.document_status().and_then(|s| s.undo),
        plain.document_status().and_then(|s| s.undo),
        "seed {seed}: the step Undo takes"
    );
    (sent.len(), lost_reads, cancelled, differ, misfiled)
}

/// Sent faster than answered, on the page and in the worker: applied once
/// each, in order, and the document is the plain session's.
#[test]
fn edits_sent_faster_than_answered_leave_the_plain_sessions_document() {
    let seeds: u64 = std::env::var("EMBED_FUZZ_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(25);
    let (mut requests, mut lost, mut histories, mut exports) = (0, 0, 0, 0);
    for seed in 0..seeds {
        let (sent, lost_reads, _, differ, misfiled) = order_run(seed, false);
        exports += misfiled;
        requests += sent;
        lost += lost_reads;
        histories += differ;
    }
    assert_eq!((histories, exports, lost), (0, 0, 0));
    eprintln!(
        "order: {seeds} seeds, {requests} requests, each answered once, each read on its own state, each export with its file"
    );
}

/// The same with the person's Cancel and a worker that holds its answers.
#[test]
fn a_cancel_in_the_middle_still_answers_every_request_once() {
    let seeds: u64 = std::env::var("EMBED_FUZZ_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(25);
    let (mut requests, mut lost, mut cancelled, mut exports) = (0, 0, 0, 0);
    for seed in 0..seeds {
        let (sent, lost_reads, cancels, _, misfiled) = order_run(seed, true);
        exports += misfiled;
        requests += sent;
        lost += lost_reads;
        cancelled += cancels;
    }
    assert_eq!(exports, 0);
    eprintln!(
        "cancel: {seeds} seeds, {requests} requests, each answered once; {cancelled} edits cancelled, {lost} reads ended by a cancel"
    );
}

/// A slider with a price behind it: 600 changes of one variable, a
/// measure sent behind each, with frames in between as a browser has them
/// (and none for a while, as when the worker is busy). Every request is
/// answered once; each measure answered ok has the volume of the state it
/// was sent for, and one that is not is refused because too many requests
/// wait, saying so; and the page's own thread built nothing, though more
/// reads waited than the session hands to the worker (R2-2 of the review
/// of round 2: past that cap the next read had its bodies built on the
/// page, which held it seconds).
#[test]
fn a_slider_of_600_changes_with_a_measure_each_is_answered_on_each_state() {
    let mut page = Page::new(true);
    for (n, (command, params)) in START.iter().enumerate() {
        page.send(Message::line(execute(
            format!("s{n}"),
            command,
            serde_json::from_str(params).unwrap(),
        )));
    }
    page.settle();
    page.take_replies();
    let work = page.host.session.work_here();
    let mut rng = Rng::new(7);
    let mut full = 0usize;
    let mut widths = Vec::new();
    for n in 0..600usize {
        let width = 5 + (n % 97);
        widths.push(width);
        page.send(Message::line(execute(
            format!("p{n}"),
            "variable.set",
            json!({"name": "w", "expr": format!("{width} mm")}),
        )));
        page.send(Message::line(execute(
            format!("m{n}"),
            "document.measure",
            json!({}),
        )));
        match rng.below(6) {
            0 | 1 => page.frame(),
            2 => page.worker.held.set(true),
            3 => page.worker.held.set(false),
            _ => {}
        }
        full += usize::from(page.host.session.commands_full());
        assert_eq!(
            page.host.session.work_here(),
            work,
            "after request {n}: built on the page's own thread"
        );
    }
    page.settle();
    assert_eq!(
        page.host.session.work_here(),
        work,
        "built on the page's own thread"
    );
    let mut answers: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for (reply, _) in page.take_replies() {
        answers
            .entry(reply["id"].as_str().unwrap_or("?").to_string())
            .or_default()
            .push(reply);
    }
    let missing: Vec<String> = (0..600)
        .flat_map(|n| [format!("p{n}"), format!("m{n}")])
        .filter(|id| !answers.contains_key(id))
        .collect();
    assert!(
        missing.is_empty(),
        "{} not answered (first {:?}); {} waiting, commands waiting {}, page answers waiting {}",
        missing.len(),
        missing.iter().take(6).collect::<Vec<_>>(),
        page.embed.waiting(),
        page.host.session.commands_waiting(),
        page.host.page_answers_waiting()
    );
    let too_many = |reply: &Value| {
        reply["error"]
            .as_str()
            .unwrap_or("")
            .contains(crate::embed::TOO_MANY_WAIT)
    };
    let (mut right, mut refused, mut wrong, mut other) = (0usize, 0usize, 0usize, 0usize);
    let mut first_wrong = String::new();
    // The width the document has: the last edit that was made.
    let mut current = 10usize;
    for (n, width) in widths.iter().enumerate() {
        let edit = &answers[&format!("p{n}")];
        assert_eq!(edit.len(), 1, "p{n}");
        if edit[0]["ok"] == true {
            current = *width;
        } else if too_many(&edit[0]) {
            refused += 1;
        } else {
            other += 1;
            if first_wrong.is_empty() {
                first_wrong = format!("p{n}: {}", edit[0]);
            }
        }
        let measure = &answers[&format!("m{n}")];
        assert_eq!(measure.len(), 1, "m{n}: {measure:?}");
        if measure[0]["ok"] == true {
            let volume = measure[0]["result"]["volume"].as_f64().unwrap_or(-1.0);
            let expected = (current as f64) * 20.0 * 30.0;
            if (volume - expected).abs() <= 1e-6 * expected {
                right += 1;
            } else {
                wrong += 1;
                if first_wrong.is_empty() {
                    first_wrong =
                        format!("m{n}: width {current}, volume {volume}, expected {expected}");
                }
            }
        } else if too_many(&measure[0]) {
            refused += 1;
        } else {
            other += 1;
            if first_wrong.is_empty() {
                first_wrong = format!("m{n}: {}", measure[0]);
            }
        }
    }
    eprintln!(
        "slider: 600 changes with a measure each: {right} measures right, {wrong} wrong, {refused} requests refused because too many waited, {other} other errors; the worker's cap reached after {full} of 600 pairs; most waiting {}; nothing built on the page",
        page.most_waiting
    );
    assert_eq!((wrong, other), (0, 0), "{first_wrong}");
    assert!(right > 0);
    assert!(page.most_waiting <= MAX_WAITING);
}
