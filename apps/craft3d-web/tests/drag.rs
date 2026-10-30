//! A slider dragged over a heavy model on the page: how long until the
//! window shows a new state, with the regenerating worker as it is (it
//! builds a piece at a time, looks at its messages between pieces and
//! keeps its caches when a job is replaced) and as it was before
//! 2026-10-30 (a replaced job ended the worker, and the next state was
//! regenerated from nothing).
//!
//! The page's own code (`Scenes` over a session that regenerates
//! elsewhere) runs on this thread, a frame every 16 ms; the worker is a
//! thread of its own that does what `packaging/web/worker.js` does with
//! the module's `scene_begin` and `scene_step`. Measured in release, on
//! the cutaway turbofan of the benchmark (241 steps):
//!
//! `cargo test --release -p craft3d-web --test drag -- --ignored --nocapture`
//!
//! It prints the times; it asserts only what holds whatever the machine:
//! every state ends up shown, and the last is the one a session that
//! builds it itself shows.

#![cfg(not(target_arch = "wasm32"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft3d_engine::{SceneQuality, SceneStep, Session, Workspace, scene_begin, scene_step};
use craft3d_web::checks::{Reply, Transport};
use craft3d_web::scenes::Scenes;
use serde_json::{Value, json};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long a piece works before the worker looks at its messages
/// (`worker.js`, PIECE_MS).
const PIECE: Duration = Duration::from_millis(30);
/// A frame of the page.
const FRAME: Duration = Duration::from_millis(16);

enum Message {
    Scene(u64, Vec<u8>, String),
    Cancel,
}

type Replies = Arc<Mutex<Vec<Reply>>>;

/// The worker: a thread with its own module state (the builder is per
/// thread), as a Web Worker has its own instance.
struct Worker {
    tx: Sender<Message>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// It has no job (answered the last, or was told to leave it).
    idle: Arc<Mutex<bool>>,
}

fn spawn(replies: &Replies) -> Worker {
    let (tx, rx) = channel();
    let idle = Arc::new(Mutex::new(true));
    let (replies, flag) = (Arc::clone(replies), Arc::clone(&idle));
    let thread = std::thread::spawn(move || work(&rx, &replies, &flag));
    Worker {
        tx,
        thread: Some(thread),
        idle,
    }
}

fn work(rx: &Receiver<Message>, replies: &Replies, idle: &Mutex<bool>) {
    let mut current: Option<u64> = None;
    loop {
        // Idle: wait for a message; working: take those that came.
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
                // Ended (`terminate`): its state goes with the thread.
                Err(TryRecvError::Disconnected) => return,
            }
        }
        for message in messages {
            match message {
                Message::Scene(token, document, work) => {
                    scene_begin(&document, &work);
                    current = Some(token);
                }
                Message::Cancel => current = None,
            }
        }
        let Some(token) = current else {
            *idle.lock().unwrap() = true;
            continue;
        };
        let until = Instant::now() + PIECE;
        let reply = match scene_step(move || Instant::now() >= until) {
            SceneStep::Part(bytes) => Some(Reply::Part { token, bytes }),
            SceneStep::Working => None,
            SceneStep::Done => {
                current = None;
                *idle.lock().unwrap() = true;
                Some(Reply::Done { token })
            }
        };
        if let Some(reply) = reply {
            replies.lock().unwrap().push(reply);
        }
    }
}

/// The page's transport. `before`: a job posted while the worker still
/// builds another ends it and starts a new one, as the page did before
/// 2026-10-30.
struct Transported {
    worker: Option<Worker>,
    replies: Replies,
    before: bool,
}

impl Transport for Transported {
    fn post(&mut self, token: u64, document: &[u8], work: &str) -> Result<(), String> {
        let busy = self
            .worker
            .as_ref()
            .is_some_and(|w| !*w.idle.lock().unwrap());
        if self.before && busy {
            self.stop();
        }
        let worker = self.worker.get_or_insert_with(|| spawn(&self.replies));
        *worker.idle.lock().unwrap() = false;
        worker
            .tx
            .send(Message::Scene(token, document.to_vec(), work.to_string()))
            .map_err(|e| e.to_string())
    }
    fn stop(&mut self) {
        // Ended: it sees that at its next look at its messages (within a
        // piece) and is waited for, so that no two workers run at once
        // in this process (they would share what the kernel keeps per
        // process, which two Web Workers never do).
        if let Some(mut worker) = self.worker.take() {
            let thread = worker.thread.take();
            drop(worker);
            if let Some(thread) = thread {
                let _ = thread.join();
            }
        }
    }
    fn cancel(&mut self) {
        if self.before {
            self.stop();
            return;
        }
        if let Some(worker) = &self.worker {
            let _ = worker.tx.send(Message::Cancel);
        }
    }
}

impl Drop for Transported {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The turbofan's steps, after a new document.
fn turbofan() -> Vec<(String, Value)> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/engine/tests/bench_models/turbofan.steps.json");
    let steps: Vec<(String, Value)> =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    std::iter::once(("file.new".to_string(), json!({"units": "mm"})))
        .chain(steps)
        .collect()
}

fn quality() -> SceneQuality {
    SceneQuality::display()
}

/// What one drag showed, in ms from its first value.
struct Drag {
    /// The first new state shown (its first picture).
    first: Option<f64>,
    /// How many of the values were shown while the drag went on.
    during: usize,
    /// The last value's display scene, from the last value.
    last: f64,
}

fn drag(name: &str, values: &[String], interval: Duration, before: bool) -> Drag {
    let replies: Replies = Arc::default();
    let transport = Transported {
        worker: None,
        replies: Arc::clone(&replies),
        before,
    };
    let mut page = Session::new(Workspace::memory());
    page.set_stepwise(true);
    page.set_checks_elsewhere(true);
    let mut scenes = Scenes::new(transport, &mut page);
    for (id, params) in turbofan() {
        page.execute(&id, &params).unwrap();
    }
    let clock = Instant::now();
    let ms = |at: Instant| at.duration_since(clock).as_secs_f64() * 1000.0;
    // The model opened and shown first.
    while !page.scene_ready(quality()) {
        let taken = std::mem::take(&mut *replies.lock().unwrap());
        scenes.frame_at(&mut page, quality(), taken, ms(Instant::now()));
        std::thread::sleep(FRAME);
        assert!(clock.elapsed() < Duration::from_secs(600), "never shown");
    }
    let start = Instant::now();
    let mut next = 0;
    let mut first = None;
    let mut during = 0;
    let mut shown_since_set = true;
    loop {
        let now = Instant::now();
        if next < values.len() && now >= start + interval * u32::try_from(next).unwrap() {
            let value = values.get(next).unwrap();
            page.execute("variable.set", &json!({"name": name, "expr": value}))
                .unwrap();
            next += 1;
            shown_since_set = false;
        }
        let taken = std::mem::take(&mut *replies.lock().unwrap());
        scenes.frame_at(&mut page, quality(), taken, ms(now));
        if !shown_since_set && page.scene_ready(SceneQuality::Preview) {
            shown_since_set = true;
            let at = now.duration_since(start).as_secs_f64() * 1000.0;
            first.get_or_insert(at);
            if next < values.len() {
                during += 1;
            }
        }
        if next == values.len() && page.scene_ready(quality()) {
            let last = start + interval * u32::try_from(values.len() - 1).unwrap();
            let done = Drag {
                first,
                during,
                last: now.duration_since(last).as_secs_f64() * 1000.0,
            };
            // The last value is shown as a session that builds it itself
            // shows it after the same values, one after another (each
            // built from what the one before left, as the desktop's
            // engine thread does); and whether that is what a session
            // that builds the last value from nothing shows.
            let shown = craft3d_engine::scene_bytes(&page.scene_with(quality()).unwrap().unwrap());
            let mut stepped = Session::new(Workspace::memory());
            for (id, params) in turbofan() {
                stepped.execute(&id, &params).unwrap();
            }
            stepped.scene_with(quality()).unwrap();
            for value in values {
                stepped
                    .execute("variable.set", &json!({"name": name, "expr": value}))
                    .unwrap();
                stepped.scene_with(quality()).unwrap();
            }
            let mut fresh = Session::new(Workspace::memory());
            for (id, params) in turbofan() {
                fresh.execute(&id, &params).unwrap();
            }
            fresh
                .execute(
                    "variable.set",
                    &json!({"name": name, "expr": values.last().unwrap()}),
                )
                .unwrap();
            let stepped =
                craft3d_engine::scene_bytes(&stepped.scene_with(quality()).unwrap().unwrap());
            let fresh = craft3d_engine::scene_bytes(&fresh.scene_with(quality()).unwrap().unwrap());
            println!(
                "drag {name}: the page's last scene {} the stepped session's, {} the fresh session's; stepped {} fresh",
                if shown == stepped {
                    "equals"
                } else {
                    "DIFFERS from"
                },
                if shown == fresh {
                    "equals"
                } else {
                    "DIFFERS from"
                },
                if stepped == fresh {
                    "equals"
                } else {
                    "DIFFERS from"
                },
            );
            assert!(
                shown == stepped || shown == fresh,
                "{name}: the page's scene is neither"
            );
            return done;
        }
        assert!(start.elapsed() < Duration::from_secs(900), "never done");
        std::thread::sleep(FRAME.saturating_sub(now.elapsed()));
    }
}

#[test]
#[ignore = "timings: run in release with --ignored --nocapture"]
fn a_slider_dragged_over_the_turbofan() {
    let interval = Duration::from_millis(250);
    for (name, unit, from, step) in [
        ("scallopR", "mm", 2.5, 0.1),
        ("filletM", "mm", 1.5, 0.05),
        ("boltHeadH", "mm", 1.8, 0.05),
    ] {
        let values: Vec<String> = (1..=10)
            .map(|i| format!("{} {unit}", from + step * f64::from(i)))
            .collect();
        for before in [true, false] {
            let d = drag(name, &values, interval, before);
            println!(
                "drag {name} x{} every {} ms, {}: first new state shown at {}, {} of {} values shown during the drag, last value's display scene {:.0} ms after it was set",
                values.len(),
                interval.as_millis(),
                if before {
                    "before (ended on supersede)"
                } else {
                    "after (kept, stoppable)"
                },
                d.first
                    .map_or("never".to_string(), |t| format!("{t:.0} ms")),
                d.during,
                values.len(),
                d.last,
            );
        }
    }
}

/// Where a difference between the scene the worker built and a session's
/// comes from: the same state built in one go, a piece at a time with a
/// stop that says yes at once, and in pieces that never stop.
#[test]
#[ignore = "diagnosis: run in release with --ignored --nocapture"]
fn the_turbofan_built_in_pieces_is_the_one_built_in_one_go() {
    let value = "2 mm";
    let scene = |stepwise: Option<bool>| {
        let mut s = Session::new(Workspace::memory());
        for (id, params) in turbofan() {
            s.execute(&id, &params).unwrap();
        }
        s.execute("variable.set", &json!({"name": "filletM", "expr": value}))
            .unwrap();
        if let Some(stop) = stepwise {
            s.set_stepwise(true);
            s.set_checks_elsewhere(true);
            while s.prepare_some(quality(), move || stop).is_some() {}
        }
        craft3d_engine::scene_bytes(&s.scene_with(quality()).unwrap().unwrap())
    };
    let one_go = scene(None);
    let pieces = scene(Some(true));
    let unstopped = scene(Some(false));
    // Through the worker's own path, from nothing.
    let mut page = Session::new(Workspace::memory());
    for (id, params) in turbofan() {
        page.execute(&id, &params).unwrap();
    }
    page.execute("variable.set", &json!({"name": "filletM", "expr": value}))
        .unwrap();
    let request = {
        page.set_stepwise(true);
        page.set_checks_elsewhere(true);
        page.set_regen_elsewhere(true);
        let token = page.scene_job(quality()).unwrap();
        page.scene_request(token).unwrap()
    };
    craft3d_engine::scene_forget();
    scene_begin(&request.document, &request.work);
    let mut parts = Vec::new();
    while let Some(part) = craft3d_engine::scene_next() {
        parts.push(part);
    }
    for part in &parts {
        page.scene_answered(request.token, Ok(part));
    }
    let worker = craft3d_engine::scene_bytes(&page.scene_with(quality()).unwrap().unwrap());
    println!(
        "filletM {value}: pieces {} one go, unstopped {} one go, worker {} one go",
        if pieces == one_go {
            "equals"
        } else {
            "DIFFERS from"
        },
        if unstopped == one_go {
            "equals"
        } else {
            "DIFFERS from"
        },
        if worker == one_go {
            "equals"
        } else {
            "DIFFERS from"
        },
    );
}

/// A state replaced after `n` pieces of its build, then the next built
/// to the end in the same worker: the same scene as a session that
/// builds the next in one go?
#[test]
#[ignore = "diagnosis: run in release with --ignored --nocapture"]
fn a_build_replaced_part_way_leaves_the_next_as_it_is_built_in_one_go() {
    let request = |value: &str| {
        let mut page = Session::new(Workspace::memory());
        for (id, params) in turbofan() {
            page.execute(&id, &params).unwrap();
        }
        page.execute("variable.set", &json!({"name": "scallopR", "expr": value}))
            .unwrap();
        page.set_stepwise(true);
        page.set_checks_elsewhere(true);
        page.set_regen_elsewhere(true);
        let token = page.scene_job(quality()).unwrap();
        (page.scene_request(token).unwrap(), page)
    };
    let (first, _) = request("2.6 mm");
    let (second, _) = request("2.7 mm");
    let mut fresh = Session::new(Workspace::memory());
    for (id, params) in turbofan() {
        fresh.execute(&id, &params).unwrap();
    }
    fresh
        .execute(
            "variable.set",
            &json!({"name": "scallopR", "expr": "2.7 mm"}),
        )
        .unwrap();
    let want = craft3d_engine::scene_bytes(&fresh.scene_with(quality()).unwrap().unwrap());
    let quick = |s: &Session| {
        let mut s = s.clone();
        craft3d_engine::scene_bytes(&s.scene_with(SceneQuality::Preview).unwrap().unwrap())
    };
    let want_preview = quick(&fresh);
    // The whole first state first (as when the page opened it), then
    // the first value replaced part way, then the second.
    let (opened, _) = request("2.5 mm");
    for n in [0_usize, 1, 3, 10, 30, 60, 100, 1000] {
        craft3d_engine::scene_forget();
        scene_begin(&opened.document, &opened.work);
        while craft3d_engine::scene_next().is_some() {}
        scene_begin(&first.document, &first.work);
        for _ in 0..n {
            if scene_step(|| true) == SceneStep::Done {
                break;
            }
        }
        scene_begin(&second.document, &second.work);
        let mut parts = Vec::new();
        while let Some(part) = craft3d_engine::scene_next() {
            parts.push(part);
        }
        let (_, mut page) = request("2.7 mm");
        let token = page.scene_job(quality()).unwrap();
        for part in &parts {
            page.scene_answered(token, Ok(part));
        }
        let got = craft3d_engine::scene_bytes(&page.scene_with(quality()).unwrap().unwrap());
        let got_preview =
            craft3d_engine::scene_bytes(&page.scene_with(SceneQuality::Preview).unwrap().unwrap());
        println!(
            "replaced after {n} pieces: display {}, preview {}",
            if got == want { "equal" } else { "DIFFERS" },
            if got_preview == want_preview {
                "equal"
            } else {
                "DIFFERS"
            },
        );
    }
}
