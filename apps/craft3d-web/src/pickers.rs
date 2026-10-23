//! The system's file dialogs, through the File System Access API
//! (`showOpenFilePicker`, `showSaveFilePicker`: Chrome, Edge,
//! Chromebooks), and writing to the file a dialog chose.
//!
//! The two pickers are called by name on `window` (web-sys only has them
//! behind `--cfg=web_sys_unstable_apis`, which would have to be set for
//! every build); the handles they return and the streams that write
//! through them are web-sys's stable bindings.
//!
//! A browser only shows a picker with transient user activation: the
//! page calls one in the frame that handles the click or key press that
//! asked for it (eframe hands input to the app in the animation frame
//! right after the event, well within the activation the browser keeps
//! for some seconds). A picker that is refused anyway (no activation
//! left, an iframe of another site, a browser without the API) is
//! [`Picked::Unavailable`], and the page falls back to a download or the
//! file input.

use crate::host::PickerType;
use js_sys::{Array, Function, Object, Promise, Reflect};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{FileSystemFileHandle, FileSystemWritableFileStream};

/// What a picker gave.
pub enum Picked {
    /// The file the person chose.
    Chosen(FileSystemFileHandle),
    /// The person closed the dialog: nothing to do, nothing to report.
    Cancelled,
    /// The browser did not show the dialog, and why.
    Unavailable(String),
}

/// True when the browser has both pickers.
pub fn available() -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    ["showOpenFilePicker", "showSaveFilePicker"]
        .iter()
        .all(|name| Reflect::get(&window, &JsValue::from_str(name)).is_ok_and(|f| f.is_function()))
}

/// The `name` and `message` of a thrown value (a `DOMException`, an
/// `Error`, or anything else).
pub fn describe(error: &JsValue) -> (String, String) {
    let field = |key: &str| {
        Reflect::get(error, &JsValue::from_str(key))
            .ok()
            .and_then(|v| v.as_string())
            .unwrap_or_default()
    };
    let message = error.as_string().unwrap_or_else(|| field("message"));
    (field("name"), message)
}

fn set(target: &Object, key: &str, value: &JsValue) -> Result<(), String> {
    Reflect::set(target, &JsValue::from_str(key), value)
        .map(|_| ())
        .map_err(|e| describe(&e).1)
}

/// The options of a picker: the kinds of file (`types`, the first one
/// proposed) and, for a save dialog, the name proposed.
fn options(types: &[PickerType], suggested: Option<&str>) -> Result<Object, String> {
    let options = Object::new();
    let kinds = Array::new();
    for kind in types {
        let accept = Object::new();
        let extensions = Array::new();
        for extension in &kind.extensions {
            extensions.push(&JsValue::from_str(&format!(".{extension}")));
        }
        set(&accept, kind.mime, &extensions)?;
        let entry = Object::new();
        set(&entry, "description", &JsValue::from_str(kind.description))?;
        set(&entry, "accept", &accept)?;
        kinds.push(&entry);
    }
    set(&options, "types", &kinds)?;
    if let Some(name) = suggested {
        set(&options, "suggestedName", &JsValue::from_str(name))?;
        // A save dialog offers only the file's own kind, no "All files":
        // the bytes are of one format.
        set(&options, "excludeAcceptAllOption", &JsValue::TRUE)?;
    }
    Ok(options)
}

/// Calls `window[method](options)` and waits for the dialog.
async fn show(method: &str, options: Object) -> Result<JsValue, Picked> {
    let unavailable = |e: &JsValue| {
        let (name, message) = describe(e);
        if name == "AbortError" {
            Picked::Cancelled
        } else {
            Picked::Unavailable(format!("{name}: {message}"))
        }
    };
    let window = web_sys::window().ok_or_else(|| Picked::Unavailable("no window".into()))?;
    let function: Function = Reflect::get(&window, &JsValue::from_str(method))
        .map_err(|e| unavailable(&e))?
        .dyn_into()
        .map_err(|_| Picked::Unavailable(format!("this browser has no {method}")))?;
    let promise: Promise = function
        .call1(&window, &options)
        .map_err(|e| unavailable(&e))?
        .dyn_into()
        .map_err(|_| Picked::Unavailable(format!("{method} gave no promise")))?;
    JsFuture::from(promise).await.map_err(|e| unavailable(&e))
}

fn handle(value: JsValue) -> Picked {
    match value.dyn_into::<FileSystemFileHandle>() {
        Ok(handle) => Picked::Chosen(handle),
        Err(_) => Picked::Unavailable("the dialog gave no file".into()),
    }
}

/// The save dialog: a file to write, proposed as `suggested`.
pub async fn save(types: &[PickerType], suggested: &str) -> Picked {
    let options = match options(types, Some(suggested)) {
        Ok(options) => options,
        Err(why) => return Picked::Unavailable(why),
    };
    match show("showSaveFilePicker", options).await {
        Ok(value) => handle(value),
        Err(picked) => picked,
    }
}

/// The open dialog: one file to read.
pub async fn open(types: &[PickerType]) -> Picked {
    let options = match options(types, None) {
        Ok(options) => options,
        Err(why) => return Picked::Unavailable(why),
    };
    match show("showOpenFilePicker", options).await {
        Ok(value) => match value.dyn_into::<Array>() {
            Ok(files) => handle(files.get(0)),
            Err(_) => Picked::Unavailable("the dialog gave no file".into()),
        },
        Err(picked) => picked,
    }
}

/// The file behind `handle`, to read.
pub async fn file(handle: &FileSystemFileHandle) -> Result<web_sys::File, String> {
    let value = JsFuture::from(handle.get_file())
        .await
        .map_err(|e| describe(&e).1)?;
    value
        .dyn_into()
        .map_err(|_| "the browser gave no file".to_string())
}

/// Removes the file of `handle` when it is empty: a save dialog makes the
/// file it names, and an export that then fails would leave it behind.
/// A file with anything in it is kept. Where the browser cannot remove
/// it (`remove` is newer than the dialogs), it stays.
pub async fn remove_if_empty(handle: &FileSystemFileHandle) {
    let Ok(file) = self::file(handle).await else {
        return;
    };
    if file.size() > 0.0 {
        return;
    }
    let Ok(remove) = Reflect::get(handle, &JsValue::from_str("remove")) else {
        return;
    };
    let Ok(remove) = remove.dyn_into::<Function>() else {
        return;
    };
    if let Ok(value) = remove.call0(handle)
        && let Ok(promise) = value.dyn_into::<Promise>()
    {
        let _ = JsFuture::from(promise).await;
    }
}

/// Writes `bytes` to the file of `handle`, in place of what it held. The
/// browser may ask the person first whether the page may write there (a
/// file chosen to open); a refusal is an error.
pub async fn write(handle: &FileSystemFileHandle, bytes: &[u8]) -> Result<(), String> {
    let cannot = |e: JsValue| {
        let (name, message) = describe(&e);
        if name == "NotAllowedError" {
            "the browser was not allowed to write there".to_string()
        } else {
            format!("{name}: {message}")
        }
    };
    let stream: FileSystemWritableFileStream = JsFuture::from(handle.create_writable())
        .await
        .map_err(cannot)?
        .dyn_into()
        .map_err(|_| "the browser gave no stream to write".to_string())?;
    // A copy in JavaScript's memory, not a view into the module's: the
    // write is asynchronous, and the module's memory may grow meanwhile.
    let copy = js_sys::Uint8Array::from(bytes);
    let written = match stream.write_with_buffer_source(&copy) {
        Ok(promise) => JsFuture::from(promise).await.map(|_| ()),
        Err(e) => Err(e),
    };
    if let Err(e) = written {
        // What was written is dropped; the file keeps what it held.
        let _ = JsFuture::from(stream.abort()).await;
        return Err(cannot(e));
    }
    JsFuture::from(stream.close())
        .await
        .map(|_| ())
        .map_err(cannot)
}
