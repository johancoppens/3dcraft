//! The page's workers in the browser: module Web Workers running
//! `worker.js` (`packaging/web`, named in the page's
//! `<meta name="craft3d-worker">`), which loads this same module and
//! answers each job: a job of checks with [`craft3d_check`], a job of
//! scenes with [`craft3d_scene_begin`] and [`craft3d_scene_step`], a
//! piece at a time, looking at its messages between pieces (and running
//! a command where the bodies are with [`craft3d_command`]): a newer job
//! of scenes replaces the one it builds, `{kind: "cancel"}` leaves it,
//! and what it regenerated stays for the next.
//! [`WebWorker`] is the page's [`Transport`] for
//! [`crate::checks::Checks`] and [`crate::scenes::Scenes`], one worker
//! each.
//!
//! Messages: the page posts `{token, kind, document, work, module}`
//! (`kind` is `check` or `scene`, the bytes of the document's `.3dcraft`
//! file as a `Uint8Array`, the work as a string, and the page's compiled
//! `WebAssembly.Module`, so the worker compiles nothing); the worker
//! answers a check with `{token, reply}`, a scene with a `{token, part}`
//! per part (a `Uint8Array`, its buffer handed over, not copied) and
//! `{token, done}`; `{token, fatal}` when the work threw (a panic aborts
//! the module: a bug), or `{token, unavailable}` when the module could
//! not be started there. An error that escapes the worker (it could not
//! be loaded) arrives as its `error` event. Every reply is put in its
//! worker's queue the next frame takes, and asks for that frame.

use crate::checks::{Reply, Transport};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

/// The name of the meta element that names the worker's script.
const META: &str = "craft3d-worker";

/// What the worker answers a job with: `craft3d_engine::measure_checks`.
#[wasm_bindgen]
pub fn craft3d_check(document: &[u8], work: &str) -> String {
    craft3d_engine::measure_checks(document, work)
}

/// What the regenerating worker answers a job of scenes with, first:
/// `craft3d_engine::scene_begin`.
#[wasm_bindgen]
pub fn craft3d_scene_begin(document: &[u8], work: &str) {
    craft3d_engine::scene_begin(document, work);
}

/// A piece of that answer (`craft3d_engine::scene_step`): the work goes
/// on for about `budget_ms` milliseconds (at least a feature or a body),
/// so that the worker looks at its messages between pieces. A part when
/// one is there (never empty), an empty array when only work was done,
/// `undefined` once the answer is complete.
#[wasm_bindgen]
pub fn craft3d_scene_step(budget_ms: f64) -> Option<Vec<u8>> {
    let until = js_sys::Date::now() + budget_ms;
    match craft3d_engine::scene_step(move || js_sys::Date::now() >= until) {
        craft3d_engine::SceneStep::Part(part) => Some(part),
        craft3d_engine::SceneStep::Working => Some(Vec::new()),
        craft3d_engine::SceneStep::Done => None,
    }
}

/// What the regenerating worker answers a command with, run where the
/// bodies are (`craft3d_engine::run_command`): `{answer, files}`, the
/// files as `[name, Uint8Array]` pairs.
#[wasm_bindgen]
pub fn craft3d_command(document: &[u8], work: &str) -> JsValue {
    let answered = craft3d_engine::run_command(document, work);
    let files = js_sys::Array::new();
    for (name, bytes) in &answered.files {
        let pair = js_sys::Array::new();
        pair.push(&JsValue::from_str(name));
        pair.push(&js_sys::Uint8Array::from(&bytes[..]));
        files.push(&pair);
    }
    let reply = js_sys::Object::new();
    let _ = js_sys::Reflect::set(
        &reply,
        &JsValue::from_str("answer"),
        &JsValue::from_str(&answered.answer),
    );
    let _ = js_sys::Reflect::set(&reply, &JsValue::from_str("files"), &files);
    reply.into()
}

/// The worker's script, as the page names it; `None` on a page that
/// names none (the checks are then measured on the page).
pub fn script() -> Option<String> {
    let document = web_sys::window()?.document()?;
    let meta = document
        .query_selector(&format!("meta[name=\"{META}\"]"))
        .ok()??;
    meta.get_attribute("content").filter(|s| !s.is_empty())
}

pub type Replies = Rc<RefCell<Vec<Reply>>>;

/// A running worker and its handlers, kept alive with it.
struct Running {
    worker: web_sys::Worker,
    _message: Closure<dyn FnMut(web_sys::MessageEvent)>,
    _error: Closure<dyn FnMut(web_sys::Event)>,
}

pub struct WebWorker {
    script: String,
    /// What it is posted: `check` or `scene` (`worker.js`).
    kind: &'static str,
    /// What it is called in messages: "the checking worker".
    name: &'static str,
    replies: Replies,
    ctx: egui::Context,
    running: Option<Running>,
}

impl WebWorker {
    /// A worker for jobs of `kind` (`check`, `scene`), `name` in what
    /// the page says of it.
    pub fn new(
        script: String,
        kind: &'static str,
        name: &'static str,
        replies: Replies,
        ctx: egui::Context,
    ) -> Self {
        Self {
            script,
            kind,
            name,
            replies,
            ctx,
            running: None,
        }
    }

    fn start(&mut self) -> Result<&web_sys::Worker, String> {
        if self.running.is_none() {
            let options = web_sys::WorkerOptions::new();
            options.set_type(web_sys::WorkerType::Module);
            let worker = web_sys::Worker::new_with_options(&self.script, &options)
                .map_err(|e| format!("{} cannot be started: {}", self.name, text(&e)))?;
            let (replies, ctx, name) = (Rc::clone(&self.replies), self.ctx.clone(), self.name);
            let message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(
                move |event: web_sys::MessageEvent| {
                    replies.borrow_mut().push(reply_of(&event.data(), name));
                    ctx.request_repaint();
                },
            );
            let (replies, ctx, name) = (Rc::clone(&self.replies), self.ctx.clone(), self.name);
            // A script that cannot be loaded (a 404, a failed import, a
            // `Content-Security-Policy` that refuses it) fires a plain
            // `Event`, an error that escapes the worker an `ErrorEvent`:
            // both are taken, the message read only where there is one.
            let error = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
                // Not the page's error: the page goes on.
                event.prevent_default();
                let why = event
                    .dyn_ref::<web_sys::ErrorEvent>()
                    .map(web_sys::ErrorEvent::message)
                    .filter(|m| !m.is_empty())
                    .unwrap_or_else(|| format!("{name} could not be loaded"));
                replies.borrow_mut().push(Reply::Failed(why));
                ctx.request_repaint();
            });
            worker.set_onmessage(Some(message.as_ref().unchecked_ref()));
            worker.set_onerror(Some(error.as_ref().unchecked_ref()));
            self.running = Some(Running {
                worker,
                _message: message,
                _error: error,
            });
        }
        self.running
            .as_ref()
            .map(|r| &r.worker)
            .ok_or_else(|| format!("{} cannot be started", self.name))
    }
}

impl WebWorker {
    fn post_kind(
        &mut self,
        kind: &str,
        token: u64,
        document: &[u8],
        work: &str,
    ) -> Result<(), String> {
        let message = js_sys::Object::new();
        for (key, value) in [
            ("token", JsValue::from_str(&token.to_string())),
            ("kind", JsValue::from_str(kind)),
            ("document", js_sys::Uint8Array::from(document).into()),
            ("work", JsValue::from_str(work)),
            ("module", wasm_bindgen::module()),
        ] {
            js_sys::Reflect::set(&message, &JsValue::from_str(key), &value)
                .map_err(|e| text(&e))?;
        }
        let name = self.name;
        self.start()?
            .post_message(&message)
            .map_err(|e| format!("the job cannot be posted to {name}: {}", text(&e)))
    }
}

impl Transport for WebWorker {
    fn post(&mut self, token: u64, document: &[u8], work: &str) -> Result<(), String> {
        self.post_kind(self.kind, token, document, work)
    }

    fn post_command(&mut self, token: u64, document: &[u8], work: &str) -> Result<(), String> {
        self.post_kind("command", token, document, work)
    }

    fn cancel(&mut self) {
        let Some(running) = &self.running else {
            return;
        };
        let message = js_sys::Object::new();
        let posted = js_sys::Reflect::set(
            &message,
            &JsValue::from_str("kind"),
            &JsValue::from_str("cancel"),
        )
        .and_then(|_| running.worker.post_message(&message));
        if posted.is_err() {
            self.stop();
        }
    }

    fn stop(&mut self) {
        if let Some(running) = self.running.take() {
            running.worker.set_onmessage(None);
            running.worker.set_onerror(None);
            running.worker.terminate();
        }
    }
}

/// A message of the worker as a [`Reply`]; one that does not fit is the
/// worker failing.
fn reply_of(data: &JsValue, name: &str) -> Reply {
    let get = |key: &str| js_sys::Reflect::get(data, &JsValue::from_str(key)).ok();
    let field = |key: &str| get(key).and_then(|v| v.as_string());
    let Some(token) = field("token").and_then(|t| t.parse::<u64>().ok()) else {
        return Reply::Failed(format!("{name} answered without a job"));
    };
    if let Some(part) = get("part").and_then(|v| v.dyn_into::<js_sys::Uint8Array>().ok()) {
        return Reply::Part {
            token,
            bytes: part.to_vec(),
        };
    }
    if get("done").is_some_and(|v| v.is_truthy()) {
        return Reply::Done { token };
    }
    if let Some(answer) = field("command") {
        let mut files = Vec::new();
        if let Some(list) = get("files").and_then(|v| v.dyn_into::<js_sys::Array>().ok()) {
            for pair in list.iter() {
                let name = js_sys::Reflect::get(&pair, &JsValue::from_f64(0.0))
                    .ok()
                    .and_then(|v| v.as_string());
                let bytes = js_sys::Reflect::get(&pair, &JsValue::from_f64(1.0))
                    .ok()
                    .and_then(|v| v.dyn_into::<js_sys::Uint8Array>().ok());
                if let (Some(name), Some(bytes)) = (name, bytes) {
                    files.push((name, bytes.to_vec()));
                }
            }
        }
        return Reply::Command {
            token,
            answer: Ok(craft3d_engine::CommandAnswer { answer, files }),
        };
    }
    if let Some(why) = field("unavailable") {
        return Reply::Failed(format!("the module cannot be started in the worker: {why}"));
    }
    match (field("reply"), field("fatal")) {
        (Some(reply), _) => Reply::Answer {
            token,
            answer: Ok(reply),
        },
        (None, Some(fatal)) => Reply::Answer {
            token,
            answer: Err(fatal),
        },
        (None, None) => Reply::Answer {
            token,
            answer: Err(format!("{name} answered nothing")),
        },
    }
}

fn text(value: &JsValue) -> String {
    value
        .as_string()
        .or_else(|| {
            value
                .dyn_ref::<js_sys::Error>()
                .map(|e| String::from(e.message()))
        })
        .unwrap_or_else(|| format!("{value:?}"))
}
