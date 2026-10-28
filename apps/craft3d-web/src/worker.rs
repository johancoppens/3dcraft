//! The checking worker in the browser: a module Web Worker running
//! `worker.js` (`packaging/web`, named in the page's
//! `<meta name="craft3d-worker">`), which loads this same module and
//! answers each job with [`craft3d_check`]. [`WebWorker`] is the page's
//! [`Transport`] for [`crate::checks::Checks`].
//!
//! Messages: the page posts `{token, document, work, module}` (strings,
//! and the page's compiled `WebAssembly.Module`, so the worker compiles
//! nothing); the worker answers `{token, reply}`, `{token, fatal}` when
//! measuring threw (a panic aborts the module: a bug), or `{token,
//! unavailable}` when the module could not be started there. An error that
//! escapes the worker (it could not be loaded) arrives as its `error`
//! event. Every reply is put in a queue the next frame takes, and asks for
//! that frame.

use crate::checks::{Reply, Transport};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

/// The name of the meta element that names the worker's script.
const META: &str = "craft3d-worker";

/// What the worker answers a job with: `craft3d_engine::measure_checks`.
#[wasm_bindgen]
pub fn craft3d_check(document: &str, work: &str) -> String {
    craft3d_engine::measure_checks(document, work)
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
    _error: Closure<dyn FnMut(web_sys::ErrorEvent)>,
}

pub struct WebWorker {
    script: String,
    replies: Replies,
    ctx: egui::Context,
    running: Option<Running>,
}

impl WebWorker {
    pub fn new(script: String, replies: Replies, ctx: egui::Context) -> Self {
        Self {
            script,
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
                .map_err(|e| format!("the checking worker cannot be started: {}", text(&e)))?;
            let (replies, ctx) = (Rc::clone(&self.replies), self.ctx.clone());
            let message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(
                move |event: web_sys::MessageEvent| {
                    replies.borrow_mut().push(reply_of(&event.data()));
                    ctx.request_repaint();
                },
            );
            let (replies, ctx) = (Rc::clone(&self.replies), self.ctx.clone());
            let error = Closure::<dyn FnMut(web_sys::ErrorEvent)>::new(
                move |event: web_sys::ErrorEvent| {
                    // Not the page's error: the page goes on.
                    event.prevent_default();
                    let why = event.message();
                    let why = if why.is_empty() {
                        "the checking worker could not be loaded".to_string()
                    } else {
                        why
                    };
                    replies.borrow_mut().push(Reply::Failed(why));
                    ctx.request_repaint();
                },
            );
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
            .ok_or_else(|| "the checking worker cannot be started".to_string())
    }
}

impl Transport for WebWorker {
    fn post(&mut self, token: u64, document: &str, work: &str) -> Result<(), String> {
        let message = js_sys::Object::new();
        for (key, value) in [
            ("token", JsValue::from_str(&token.to_string())),
            ("document", JsValue::from_str(document)),
            ("work", JsValue::from_str(work)),
            ("module", wasm_bindgen::module()),
        ] {
            js_sys::Reflect::set(&message, &JsValue::from_str(key), &value)
                .map_err(|e| text(&e))?;
        }
        self.start()?.post_message(&message).map_err(|e| {
            format!(
                "the job cannot be posted to the checking worker: {}",
                text(&e)
            )
        })
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
fn reply_of(data: &JsValue) -> Reply {
    let field = |key: &str| {
        js_sys::Reflect::get(data, &JsValue::from_str(key))
            .ok()
            .and_then(|v| v.as_string())
    };
    let Some(token) = field("token").and_then(|t| t.parse::<u64>().ok()) else {
        return Reply::Failed("the checking worker answered without a job".into());
    };
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
            answer: Err("the checking worker answered nothing".into()),
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
