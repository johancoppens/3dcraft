//! Embed's door for a module loaded in the page (experimental): how the
//! page reaches the core ([`crate::embed`]). The connection is a
//! `MessagePort`, so the same page code can later talk to 3DCraft in an
//! iframe, a worker or on a server without knowing which: the page calls
//! [`craft3d_connect`] and gets one end of a `MessageChannel`, 3DCraft
//! holds the other.
//!
//! What the page posts: a request (the envelope `{id, method, params}`)
//! as an object or as its JSON text; [`crate::embed::GIVE`]'s file as
//! `bytes` beside it (an `ArrayBuffer` or a typed array), taken out
//! before the rest is read as the envelope. What it gets: each reply and
//! each event as an object (the JSON of the protocol, parsed), a reply
//! with the files its request wrote under `files: [{name, bytes}]`
//! (`Uint8Array`s, their buffers handed over, not copied).
//!
//! Messages wait in an inbox for the page's next frame (`page.rs` takes
//! them, the core answers them, the door posts what comes out). What a
//! message can make the door keep is capped ([`MAX_INBOX`],
//! [`MAX_INBOX_BYTES`], the protocol's line limit and
//! [`crate::embed::MAX_FILE_BYTES`]): past it the message is answered with
//! an error at once. Nothing here panics: a message that does not read is
//! an error reply, a port that is gone is no error.

use crate::embed::{self, Message, Out};
use craft3d_automation::MAX_REQUEST_BYTES;
use craft3d_automation::rpc;
use serde_json::Value;
use std::cell::RefCell;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

/// Most messages waiting for the next frame.
const MAX_INBOX: usize = 1024;

/// Most bytes they may hold together (their lines and files).
const MAX_INBOX_BYTES: usize = embed::MAX_WAITING_BYTES;

#[derive(Default)]
struct Door {
    /// 3DCraft's end of the channel, once the page connected.
    port: Option<web_sys::MessagePort>,
    _listener: Option<Closure<dyn FnMut(web_sys::MessageEvent)>>,
    inbox: Vec<Message>,
    inbox_bytes: usize,
    /// The page's frames: a message asks for one.
    ctx: Option<egui::Context>,
    /// Why 3DCraft could not start, said to a page that connects later.
    failed: Option<String>,
}

thread_local! {
    static DOOR: RefCell<Door> = RefCell::new(Door::default());
}

/// Connects the page to 3DCraft (Embed, experimental): returns the page's
/// end of a `MessageChannel`, whose other end 3DCraft holds. The page
/// posts requests on it and hears replies and events on it, starting with
/// `ready` once the window is there (or `failed`). One port per page: a
/// second call is an error.
#[wasm_bindgen]
pub fn craft3d_connect() -> Result<web_sys::MessagePort, JsValue> {
    DOOR.with(|door| {
        let mut door = door
            .try_borrow_mut()
            .map_err(|_| JsValue::from_str("3DCraft is busy; connect again"))?;
        if door.port.is_some() {
            return Err(JsValue::from_str(
                "3DCraft is connected already: a page has one port",
            ));
        }
        let channel = web_sys::MessageChannel::new()?;
        let ours = channel.port2();
        let listener = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(
            move |event: web_sys::MessageEvent| arrived(&event.data()),
        );
        ours.set_onmessage(Some(listener.as_ref().unchecked_ref()));
        if let Some(why) = &door.failed {
            post_line(&ours, &embed::failed_event(why), Vec::new());
        }
        door.port = Some(ours);
        door._listener = Some(listener);
        if let Some(ctx) = &door.ctx {
            ctx.request_repaint();
        }
        Ok(channel.port1())
    })
}

/// The page's frames, asked for when a message comes.
pub fn set_context(ctx: egui::Context) {
    DOOR.with(|door| {
        if let Ok(mut door) = door.try_borrow_mut() {
            door.ctx = Some(ctx);
        }
    });
}

/// True once the page connected.
pub fn connected() -> bool {
    DOOR.with(|door| door.try_borrow().is_ok_and(|door| door.port.is_some()))
}

/// 3DCraft could not start in the page: said now to a page that listens,
/// and to one that connects later; what waits in the inbox, and every
/// request from now on, is answered with that error (no frame will come
/// to answer it).
pub fn failed(why: &str) {
    let waiting = DOOR.with(|door| {
        let Ok(mut door) = door.try_borrow_mut() else {
            return Vec::new();
        };
        if let Some(port) = &door.port {
            post_line(port, &embed::failed_event(why), Vec::new());
        }
        door.failed = Some(why.to_string());
        door.inbox_bytes = 0;
        std::mem::take(&mut door.inbox)
    });
    for message in waiting {
        refuse(&id_in(&message.line), &not_running(why));
    }
}

/// What a request gets while 3DCraft is not running in the page.
fn not_running(why: &str) -> String {
    format!("3DCraft is not running in this page: {why}")
}

/// The `id` of a request line, where it reads.
fn id_in(line: &str) -> Value {
    serde_json::from_str::<Value>(line)
        .ok()
        .and_then(|v| v.get("id").cloned())
        .unwrap_or(Value::Null)
}

/// The messages that came since the last call, in order.
pub fn take() -> Vec<Message> {
    DOOR.with(|door| {
        door.try_borrow_mut()
            .map(|mut door| {
                door.inbox_bytes = 0;
                std::mem::take(&mut door.inbox)
            })
            .unwrap_or_default()
    })
}

/// Posts the replies and events to the page, in order.
pub fn post(outs: Vec<Out>) {
    if outs.is_empty() {
        return;
    }
    DOOR.with(|door| {
        let Ok(door) = door.try_borrow() else {
            return;
        };
        let Some(port) = &door.port else {
            return;
        };
        for out in outs {
            match out {
                Out::Reply { line, files } => post_line(port, &line, files),
                Out::Event(line) => post_line(port, &line, Vec::new()),
            }
        }
    });
}

/// A message of the page, into the inbox; or its error reply at once.
fn arrived(data: &JsValue) {
    let message = match read(data) {
        Ok(message) => message,
        Err((id, why)) => {
            refuse(&id, &why);
            return;
        }
    };
    let size = message
        .line
        .len()
        .saturating_add(message.bytes.as_ref().map_or(0, Vec::len));
    // 3DCraft did not start: the request is answered with why, at once.
    let failed = DOOR.with(|door| door.try_borrow().ok().and_then(|door| door.failed.clone()));
    if let Some(why) = failed {
        refuse(&id_in(&message.line), &not_running(&why));
        return;
    }
    let kept = DOOR.with(move |door| {
        let Ok(mut door) = door.try_borrow_mut() else {
            return Err(message);
        };
        if door.inbox.len() >= MAX_INBOX || door.inbox_bytes.saturating_add(size) > MAX_INBOX_BYTES
        {
            return Err(message);
        }
        door.inbox_bytes = door.inbox_bytes.saturating_add(size);
        door.inbox.push(message);
        if let Some(ctx) = &door.ctx {
            ctx.request_repaint();
        }
        Ok(())
    });
    if let Err(message) = kept {
        let id = id_in(&message.line);
        refuse(
            &id,
            &format!(
                "too many messages wait for 3DCraft's next frame (at most {MAX_INBOX}, {MAX_INBOX_BYTES} bytes); send it again once the replies have come"
            ),
        );
    }
}

fn refuse(id: &Value, why: &str) {
    let line = rpc::error_reply(id, why);
    DOOR.with(|door| {
        if let Ok(door) = door.try_borrow()
            && let Some(port) = &door.port
        {
            post_line(port, &line, Vec::new());
        }
    });
}

/// A message of the page as the core takes it: the envelope's JSON text,
/// and the file beside it. An error (with the request's id where it has
/// one) when it does not read.
fn read(data: &JsValue) -> Result<Message, (Value, String)> {
    if let Some(text) = data.dyn_ref::<js_sys::JsString>() {
        if text.length() as usize > MAX_REQUEST_BYTES {
            return Err((Value::Null, too_long()));
        }
        return Ok(Message::line(String::from(text)));
    }
    let id = id_of(data);
    let field = |key: &str| js_sys::Reflect::get(data, &JsValue::from_str(key)).ok();
    let given = field("bytes").filter(|b| !b.is_undefined() && !b.is_null());
    let bytes = match &given {
        Some(value) => Some(bytes_of(value).map_err(|why| (id.clone(), why))?),
        None => None,
    };
    // The envelope without the file.
    let request = match (&given, data.dyn_ref::<js_sys::Object>()) {
        (Some(_), Some(object)) => {
            let copy = js_sys::Object::assign(&js_sys::Object::new(), object);
            let _ = js_sys::Reflect::delete_property(&copy, &JsValue::from_str("bytes"));
            copy.into()
        }
        _ => data.clone(),
    };
    // A message that cannot be written as JSON (a cycle, a BigInt) throws,
    // which is caught; `undefined` gives no text at all.
    let unread = || {
        (
            id.clone(),
            "the message cannot be read as JSON; post the request {id, method, params} as an object or as its JSON text".to_string(),
        )
    };
    let text: JsValue = js_sys::JSON::stringify(&request)
        .map(JsValue::from)
        .map_err(|_| unread())?;
    let Some(text) = text.dyn_ref::<js_sys::JsString>() else {
        return Err(unread());
    };
    if text.length() as usize > MAX_REQUEST_BYTES {
        return Err((id, too_long()));
    }
    Ok(Message {
        line: String::from(text),
        bytes,
    })
}

fn too_long() -> String {
    format!("the request is longer than {MAX_REQUEST_BYTES} bytes; send less at a time")
}

/// The request's `id`, when it is a number or a string.
fn id_of(data: &JsValue) -> Value {
    let Ok(id) = js_sys::Reflect::get(data, &JsValue::from_str("id")) else {
        return Value::Null;
    };
    if let Some(text) = id.as_string() {
        return Value::String(text.chars().take(200).collect());
    }
    match id.as_f64() {
        Some(n) if n.is_finite() && n.fract() == 0.0 && n.abs() < 9.0e15 => {
            #[allow(clippy::cast_possible_truncation)]
            let whole = n as i64;
            Value::from(whole)
        }
        Some(n) => serde_json::Number::from_f64(n).map_or(Value::Null, Value::Number),
        None => Value::Null,
    }
}

/// The file's bytes: an `ArrayBuffer`, or a view of one (a `Uint8Array`
/// or another typed array, a `DataView`), at most
/// [`embed::MAX_FILE_BYTES`].
fn bytes_of(value: &JsValue) -> Result<Vec<u8>, String> {
    let too_large = |length: f64| {
        format!(
            "{}: the file is {length} bytes; at most {} are taken",
            embed::GIVE,
            embed::MAX_FILE_BYTES
        )
    };
    if let Some(buffer) = value.dyn_ref::<js_sys::ArrayBuffer>() {
        let length = buffer.byte_length();
        if length as usize > embed::MAX_FILE_BYTES {
            return Err(too_large(f64::from(length)));
        }
        return Ok(js_sys::Uint8Array::new(buffer).to_vec());
    }
    if js_sys::ArrayBuffer::is_view(value) {
        let number = |key: &str| {
            js_sys::Reflect::get(value, &JsValue::from_str(key))
                .ok()
                .and_then(|v| v.as_f64())
                .filter(|n| n.is_finite() && *n >= 0.0 && *n <= f64::from(u32::MAX))
        };
        let buffer = js_sys::Reflect::get(value, &JsValue::from_str("buffer"))
            .ok()
            .and_then(|b| b.dyn_into::<js_sys::ArrayBuffer>().ok());
        if let (Some(buffer), Some(offset), Some(length)) =
            (buffer, number("byteOffset"), number("byteLength"))
        {
            if length > embed::MAX_FILE_BYTES as f64 {
                return Err(too_large(length));
            }
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let (offset, length) = (offset as u32, length as u32);
            return Ok(js_sys::Uint8Array::new_with_byte_offset_and_length(
                &buffer, offset, length,
            )
            .to_vec());
        }
    }
    Err("`bytes` must be the file's bytes: an ArrayBuffer or a typed array (a Uint8Array); read a Blob or a File with its arrayBuffer() first".to_string())
}

/// Posts one reply or event as an object, with its files as
/// `Uint8Array`s whose buffers are handed over.
fn post_line(port: &web_sys::MessagePort, line: &str, files: Vec<(String, Vec<u8>)>) {
    let value = js_sys::JSON::parse(line).unwrap_or_else(|_| JsValue::from_str(line));
    let transfer = js_sys::Array::new();
    if !files.is_empty() && value.is_object() {
        let list = js_sys::Array::new();
        for (name, bytes) in files {
            let array = js_sys::Uint8Array::from(bytes.as_slice());
            let file = js_sys::Object::new();
            let _ =
                js_sys::Reflect::set(&file, &JsValue::from_str("name"), &JsValue::from_str(&name));
            let _ = js_sys::Reflect::set(&file, &JsValue::from_str("bytes"), &array);
            list.push(&file);
            transfer.push(&array.buffer());
        }
        let _ = js_sys::Reflect::set(&value, &JsValue::from_str("files"), &list);
    }
    // A port the page closed takes nothing; that is all.
    let _ = port.post_message_with_transferable(&value, &transfer);
}
