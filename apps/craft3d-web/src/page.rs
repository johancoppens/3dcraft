//! The page: eframe's web runner around the [`WebHost`], and what only
//! the browser can do: choose a file (with the system's file dialogs
//! where the browser has them, [`crate::pickers`], else with the file
//! input), read a dropped one, write a file or offer a download, keep
//! preferences, warn before unsaved work is closed.
//!
//! Every call into the browser is a `Result` or an `Option` that is
//! handled: a page without storage, a refused dialog, a failed write or
//! fetch is reported in the status bar or ignored, never a panic (a panic
//! in WebAssembly ends the page). A file dialog the person closes is not
//! reported: nothing was asked of the page.

use crate::checks::{Checks, Reply};
use crate::door;
use crate::embed::Embed;
use crate::flags::{self, Flags};
use crate::host::{
    Ask, Download, Pick, PickerType, WebHost, open_types, page_title, received_kind,
    refusal_is_about_activation,
};
use crate::pickers::{self, Picked};
use crate::prefs::{self, Prefs};
use crate::scenes::Scenes;
use crate::worker::{self, Replies, WebWorker};
use craft3d_ui_egui::{RendererChoice, Shell};
use egui_wgpu::wgpu;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::sync::{Arc, Mutex, PoisonError};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::FileSystemFileHandle;

/// The canvas and the message element of `index.html`.
pub(crate) const CANVAS_ID: &str = "craft3d_canvas";
const LOADING_ID: &str = "craft3d_loading";

/// The canvas's attribute by which a page asks for the embedded stand,
/// its value the options (Embed, experimental: `crate::embed`).
const EMBED_ATTRIBUTE: &str = "data-craft3d-embed";

/// The most bytes of a file handed to the page (a `.3dcraft` file or a
/// STEP file is read whole into memory).
const MAX_FILE: f64 = 256.0 * 1024.0 * 1024.0;

/// A file read in the background, for the next frame.
struct Arrived {
    name: String,
    bytes: Result<Vec<u8>, String>,
    wanted: Option<Ask>,
    /// The file it was read from, when the open dialog chose it: a
    /// document is saved back there.
    handle: Option<FileSystemFileHandle>,
}

type Inbox = Rc<RefCell<Vec<Arrived>>>;

/// How long a frame works on the long work (regenerating, meshing, the
/// assertions) before the page draws, in milliseconds: some 20 frames a
/// second while it lasts, each piece (a feature, a body, an assertion)
/// whole.
const FRAME_WORK_MS: f64 = 40.0;

/// How many frames the embedded stand draws after a key or a click while
/// Escape had something of the window's to do, so that what it closed is
/// seen gone ([`Page::keys_after`]).
const SETTLING_FRAMES: u8 = 3;

/// What a file dialog or a write in the background left for the next
/// frame.
enum Mail {
    /// The save dialog chose a file for `pick`.
    Picked(Pick, FileSystemFileHandle),
    /// The save dialog was not shown (why): the file is downloaded, or
    /// the dialog over the page names it first.
    NoSavePicker(Pick, String),
    /// The open dialog was not shown (why): the file input asks.
    NoOpenPicker(Ask, String),
    /// A file was written through a handle, or not.
    Written(Written),
    /// A line for the status bar.
    Note {
        label: &'static str,
        ok: bool,
        text: String,
    },
}

/// A write through a file dialog's handle, done.
struct Written {
    handle: FileSystemFileHandle,
    /// The document it is the saved file of (saved there again from now
    /// on), or `None` for an export.
    document: Option<u64>,
    /// The document is closed once this is written (and only then).
    close: bool,
    bytes: usize,
    result: Result<(), String>,
}

type Mailbox = Rc<RefCell<Vec<Mail>>>;

/// Starts the page once the module is loaded.
#[wasm_bindgen(start)]
pub fn start() {
    // The release notes the build script took from CHANGELOG.md, for
    // What's new and `session.whatsNew`, in the page and in its workers
    // (a command may run in either). The build read them before, so they
    // read here.
    let _ = craft3d_engine::set_release_notes(crate::RELEASE_NOTES);
    // The same module runs the checking worker (`worker.rs`), where there
    // is no window and no page to start.
    if web_sys::window().is_none() {
        return;
    }
    // The commit the build script embedded, for the bug report.
    craft3d_engine::set_build_commit(option_env!("CRAFT3D_GIT_COMMIT"));
    let _ = eframe::WebLogger::init(log::LevelFilter::Info);
    wasm_bindgen_futures::spawn_local(run());
}

fn window() -> Option<web_sys::Window> {
    web_sys::window()
}

fn document() -> Option<web_sys::Document> {
    window()?.document()
}

/// Milliseconds on the page's monotonic clock (`performance.now()`), which
/// never goes back as the wall clock may (a clock set back, a laptop
/// woken): what the frame's budget and the workers' deadlines are
/// measured with. The wall clock only where the page has no
/// `performance`.
fn now_ms() -> f64 {
    window()
        .and_then(|w| w.performance())
        .map_or_else(js_sys::Date::now, |p| p.now())
}

/// Puts `text` into the loading element, in place of the app.
fn say(text: &str) {
    if let Some(element) = document().and_then(|d| d.get_element_by_id(LOADING_ID)) {
        element.set_text_content(Some(text));
    }
}

fn hide_loading() {
    if let Some(element) = document().and_then(|d| d.get_element_by_id(LOADING_ID)) {
        element.remove();
    }
}

fn text_of(value: &JsValue) -> String {
    value
        .as_string()
        .or_else(|| {
            value
                .dyn_ref::<js_sys::Error>()
                .map(|e| String::from(e.message()))
        })
        .unwrap_or_else(|| format!("{value:?}"))
}

async fn run() {
    let Some(canvas) = document()
        .and_then(|d| d.get_element_by_id(CANVAS_ID))
        .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
    else {
        say("3DCraft cannot start: the page has no canvas.");
        // A page that connected hears it, and every request it sends.
        door::failed(&format!(
            "3DCraft cannot start: the page has no <canvas id=\"{CANVAS_ID}\"> when the module starts"
        ));
        return;
    };
    // The embedded stand (Embed, experimental): a page that sets
    // `data-craft3d-embed` on the canvas keeps what is page-wide: its
    // title, its focus, its keys outside 3DCraft, its question before
    // leaving, its storage and its address. The options come from the
    // attribute, in the syntax of the address's flags
    // (`presentation=view&webgl`), not from the page's address; what of
    // them is not taken is said in the console and in `ready`.
    let embed_options = canvas.get_attribute(EMBED_ATTRIBUTE);
    let embedded = embed_options.is_some();
    let warnings = embed_options
        .as_deref()
        .map(flags::embed_warnings)
        .unwrap_or_default();
    for warning in &warnings {
        log::warn!("3DCraft, {EMBED_ATTRIBUTE}: {warning}");
    }
    let search = match embed_options {
        Some(options) => options,
        None => window()
            .and_then(|w| w.location().search().ok())
            .unwrap_or_default(),
    };
    let flags = flags::parse(&search);
    let keys = Keys::default();
    let mut options = eframe::WebOptions::default();
    if flags.webgl {
        let mut setup = egui_wgpu::WgpuSetupCreateNew::without_display_handle();
        setup.instance_descriptor.backends = wgpu::Backends::GL;
        options.wgpu_options.wgpu_setup = egui_wgpu::WgpuSetup::CreateNew(setup);
    }
    let inbox: Inbox = Rc::new(RefCell::new(Vec::new()));
    let unsaved = Rc::new(Cell::new(false));
    if !embedded {
        warn_before_leaving(Rc::clone(&unsaved), true);
        if let Some(window) = window() {
            keep_file_shortcuts(&[window.into()]);
        }
    } else if flags.warn {
        warn_before_leaving(Rc::clone(&unsaved), false);
    }
    // Where the page had the focus before eframe's start takes it.
    let focused = document().and_then(|d| d.active_element());
    let runner = eframe::WebRunner::new();
    let (for_app, for_unsaved) = (Rc::clone(&inbox), Rc::clone(&unsaved));
    let open = flags.open.clone();
    let stand = Stand {
        embedded,
        warnings,
        keys: keys.clone(),
    };
    let start = runner.start(
        canvas.clone(),
        options,
        Box::new(move |creation| {
            let app = Page::new(creation, &flags, for_app, for_unsaved, stand);
            Ok(Box::new(app))
        }),
    );
    let started = if embedded {
        start_leaving_focus(start, &canvas, focused, keys).await
    } else {
        start.await
    };
    match started {
        Ok(()) => {
            hide_loading();
            if let Some(url) = open {
                fetch_into(url, inbox);
            }
        }
        Err(error) => {
            let why = text_of(&error);
            say(&format!(
                "3DCraft needs WebGPU or WebGL2, and this browser offers neither here ({why}). \
                 Try an up-to-date Chrome, Edge, Firefox or Safari, with hardware acceleration on."
            ));
            door::failed(&format!(
                "3DCraft needs WebGPU or WebGL2, and this browser offers neither here: {why}"
            ));
        }
    }
    // The runner lives as long as the page.
    std::mem::forget(runner);
}

/// What the page asked for at the start, for [`Page::new`].
struct Stand {
    /// The embedded stand (`data-craft3d-embed`).
    embedded: bool,
    /// What of its options was not taken, for `ready`.
    warnings: Vec<String>,
    keys: Keys,
}

/// What the embedded stand's own listeners on the canvas need to know of
/// the window, set at the end of each frame ([`Page::ui`]).
#[derive(Clone, Default)]
struct Keys {
    /// Escape has something of the window's to do: a field has the keys,
    /// a menu, a list, a popover or a dialog is open
    /// ([`craft3d_ui_egui::Shell::escape_has_work`]).
    busy: Rc<Cell<bool>>,
    /// The presentation is `view`: Tab has nothing to move to inside.
    view: Rc<Cell<bool>>,
    /// Frames still to draw after a key or a click while Escape had work
    /// ([`Page::keys_after`]).
    settling: Rc<Cell<u8>>,
}

/// eframe's start in the embedded stand: it makes its hidden text field
/// beside the canvas and gives it the focus before it first waits (for the
/// graphics adapter), with no option to leave it be. The focus goes back
/// to where the page had it (or nowhere) right after that first step, in
/// the same task, so no key of the page's goes astray; and Undo, Redo and
/// the File keys are kept from the browser only where 3DCraft has the
/// focus (its canvas, its text field), not on the whole page.
///
/// And a keyboard user can leave: eframe keeps Tab inside its canvas
/// whatever its options say, so 3DCraft's own listeners, called before
/// eframe's (the capture phase on the canvas), let it go to the browser
/// where Escape has nothing of the window's to do: Tab and Shift+Tab in
/// `view` move on to the page's next or previous element; Escape leaves
/// the canvas in every presentation, but first closes what is open (a
/// menu, a list, a popover, a dialog) or gives a field's keyboard back,
/// one Escape each, as in the desktop app. eframe's text field is taken
/// out of the order Tab follows. A ring round the canvas shows when 3DCraft
/// has the keys. The wheel over the canvas turns into a zoom
/// only while 3DCraft has the focus or with Ctrl (⌘) held, a pinch;
/// otherwise it scrolls the page.
async fn start_leaving_focus(
    start: impl std::future::Future<Output = Result<(), JsValue>>,
    canvas: &web_sys::HtmlCanvasElement,
    focused: Option<web_sys::Element>,
    keys: Keys,
) -> Result<(), JsValue> {
    use std::task::Poll;
    let mut start = std::pin::pin!(start);
    let first = std::future::poll_fn(|cx| Poll::Ready(start.as_mut().poll(cx))).await;
    // eframe puts its text field right after the canvas.
    let agent = canvas
        .next_element_sibling()
        .filter(|e| e.dyn_ref::<web_sys::HtmlInputElement>().is_some());
    let body = document()
        .and_then(|d| d.body())
        .map(web_sys::Element::from);
    match focused
        .filter(|e| Some(e) != body.as_ref())
        .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
    {
        Some(element) => {
            let options = web_sys::FocusOptions::new();
            options.set_prevent_scroll(true);
            let _ = element.focus_with_options(&options);
        }
        None => {
            if let Some(agent) = agent
                .as_ref()
                .and_then(|a| a.dyn_ref::<web_sys::HtmlElement>())
            {
                let _ = agent.blur();
            }
        }
    }
    if let Some(agent) = &agent {
        let _ = agent.set_attribute("tabindex", "-1");
    }
    let mut targets: Vec<web_sys::EventTarget> = vec![canvas.clone().into()];
    targets.extend(agent.clone().map(web_sys::EventTarget::from));
    keep_file_shortcuts(&targets);
    let_the_keyboard_and_the_wheel_out(canvas, agent, &targets, keys);
    match first {
        Poll::Ready(started) => started,
        Poll::Pending => start.await,
    }
}

/// The embedded stand's listeners on the canvas and eframe's text field,
/// in the capture phase so that they come before eframe's own
/// ([`start_leaving_focus`]). Kept for the page's lifetime.
fn let_the_keyboard_and_the_wheel_out(
    canvas: &web_sys::HtmlCanvasElement,
    agent: Option<web_sys::Element>,
    targets: &[web_sys::EventTarget],
    keys: Keys,
) {
    let (out, agent_out) = (canvas.clone(), agent.clone());
    let key =
        Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(move |event: web_sys::KeyboardEvent| {
            if keys.busy.get() || event.ctrl_key() || event.meta_key() || event.alt_key() {
                return;
            }
            match event.key().as_str() {
                // The browser moves the focus on; eframe does not see it.
                "Tab" if keys.view.get() => event.stop_immediate_propagation(),
                // Nothing of the window's to close: the canvas lets go of
                // the keys. The Escape is 3DCraft's still, in every browser
                // alike (eframe in Firefox would take it otherwise): a page
                // hears that 3DCraft let go by the canvas's `blur`.
                "Escape" => {
                    let _ = out.blur();
                    if let Some(agent) = agent_out
                        .as_ref()
                        .and_then(|a| a.dyn_ref::<web_sys::HtmlElement>())
                    {
                        let _ = agent.blur();
                    }
                    event.stop_immediate_propagation();
                }
                _ => {}
            }
        });
    for target in targets {
        let _ = target.add_event_listener_with_callback_and_bool(
            "keydown",
            key.as_ref().unchecked_ref(),
            true,
        );
    }
    key.forget();
    let (own, agent_own) = (canvas.clone(), agent);
    let wheel =
        Closure::<dyn FnMut(web_sys::WheelEvent)>::new(move |event: web_sys::WheelEvent| {
            let active = document().and_then(|d| d.active_element());
            let ours = active.as_ref().is_some_and(|a| {
                a == own.as_ref() || agent_own.as_ref().is_some_and(|agent| agent == a)
            });
            if !ours && !event.ctrl_key() && !event.meta_key() {
                // eframe does not see it, so it does not cancel it: the page
                // scrolls.
                event.stop_immediate_propagation();
            }
        });
    let _ = canvas.add_event_listener_with_callback_and_bool(
        "wheel",
        wheel.as_ref().unchecked_ref(),
        true,
    );
    wheel.forget();
}

/// The browser's question before the page is closed or reloaded while a
/// document has unsaved changes. Kept for the page's lifetime. The plain
/// page's own (`assign`: `window.onbeforeunload`); in the embedded stand,
/// asked for with `warn`, a listener beside the page's own handler.
fn warn_before_leaving(unsaved: Rc<Cell<bool>>, assign: bool) {
    let Some(window) = window() else {
        return;
    };
    let handler = Closure::<dyn FnMut(web_sys::BeforeUnloadEvent)>::new(
        move |event: web_sys::BeforeUnloadEvent| {
            if unsaved.get() {
                event.prevent_default();
                event.set_return_value("There are unsaved changes.");
            }
        },
    );
    if assign {
        window.set_onbeforeunload(Some(handler.as_ref().unchecked_ref()));
    } else {
        let _ = window
            .add_event_listener_with_callback("beforeunload", handler.as_ref().unchecked_ref());
    }
    handler.forget();
}

/// Keeps the File shortcuts eframe leaves to the browser for the page:
/// Ctrl+E (Export; Chrome and Firefox move the focus to the address bar's
/// search) and Ctrl+I (Import; Firefox's page information). eframe itself
/// keeps Ctrl+O and Ctrl+S; Ctrl+N and Ctrl+W the browser never gives up.
/// Undo and Redo too: Ctrl+Z, Ctrl+Shift+Z and Ctrl+Y, so the browser
/// does not act on them as well (egui still gets them, and a text field
/// in the page keeps its own undo). Which keys: [`crate::host::page_keeps_key`].
/// On `targets`: the window on the plain page, 3DCraft's canvas and text
/// field in the embedded stand. Kept for the page's lifetime.
fn keep_file_shortcuts(targets: &[web_sys::EventTarget]) {
    let handler =
        Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(move |event: web_sys::KeyboardEvent| {
            let command = event.ctrl_key() || event.meta_key();
            let key = event.key().to_ascii_lowercase();
            if crate::host::page_keeps_key(command, event.alt_key(), event.shift_key(), &key) {
                event.prevent_default();
            }
        });
    for target in targets {
        let _ =
            target.add_event_listener_with_callback("keydown", handler.as_ref().unchecked_ref());
    }
    handler.forget();
}

/// Reads a file of the same site (`?open=`) into the inbox.
fn fetch_into(url: String, inbox: Inbox) {
    wasm_bindgen_futures::spawn_local(async move {
        let name = flags::file_name(&url);
        let bytes = fetch(&url).await;
        inbox.borrow_mut().push(Arrived {
            name,
            bytes,
            wanted: None,
            handle: None,
        });
    });
}

/// The address `url` names, resolved against the page's own, when it is
/// on the page's own origin; an error otherwise. On top of the textual
/// check of `flags::same_site`: the browser's own parser decides.
fn same_origin(url: &str) -> Result<String, String> {
    let location = window().ok_or("no window")?.location();
    let page = location.href().map_err(|e| text_of(&e))?;
    let origin = location.origin().map_err(|e| text_of(&e))?;
    let resolved = web_sys::Url::new_with_base(url, &page)
        .map_err(|_| format!("`{url}` is not an address"))?;
    if resolved.origin() != origin {
        return Err(format!(
            "`{url}` is not on this site; only files of {origin} are opened"
        ));
    }
    Ok(resolved.href())
}

async fn fetch(url: &str) -> Result<Vec<u8>, String> {
    let window = window().ok_or("no window")?;
    let resolved = same_origin(url)?;
    let response = JsFuture::from(window.fetch_with_str(&resolved))
        .await
        .map_err(|e| format!("cannot fetch `{url}`: {}", text_of(&e)))?;
    let response: web_sys::Response = response
        .dyn_into()
        .map_err(|_| format!("cannot fetch `{url}`"))?;
    if !response.ok() {
        return Err(format!(
            "cannot fetch `{url}`: {} {}",
            response.status(),
            response.status_text()
        ));
    }
    let too_large = || format!("`{url}` is larger than {MAX_FILE} bytes");
    // What the server says first; then what arrives, chunk by chunk, so
    // a file that is larger than it said is cut off at the limit.
    if let Ok(Some(length)) = response.headers().get("content-length")
        && length.trim().parse::<f64>().is_ok_and(|n| n > MAX_FILE)
    {
        return Err(too_large());
    }
    let cannot = |e: JsValue| format!("cannot read `{url}`: {}", text_of(&e));
    let body = response
        .body()
        .ok_or_else(|| format!("`{url}` has no body"))?;
    let reader: web_sys::ReadableStreamDefaultReader = body
        .get_reader()
        .dyn_into()
        .map_err(|_| format!("cannot read `{url}`"))?;
    let mut bytes = Vec::new();
    loop {
        let chunk = JsFuture::from(reader.read()).await.map_err(cannot)?;
        let done = js_sys::Reflect::get(&chunk, &JsValue::from_str("done"))
            .map_err(cannot)?
            .as_bool()
            .unwrap_or(true);
        if done {
            return Ok(bytes);
        }
        let value = js_sys::Reflect::get(&chunk, &JsValue::from_str("value")).map_err(cannot)?;
        let array: js_sys::Uint8Array = value
            .dyn_into()
            .map_err(|_| format!("cannot read `{url}`"))?;
        let total = bytes.len().saturating_add(array.length() as usize);
        if total as f64 > MAX_FILE {
            let _ = reader.cancel();
            return Err(too_large());
        }
        let start = bytes.len();
        bytes.resize(total, 0);
        if let Some(slot) = bytes.get_mut(start..) {
            array.copy_to(slot);
        }
    }
}

/// Reads a file the person chose or dropped.
async fn read_file(file: web_sys::File) -> Result<Vec<u8>, String> {
    if file.size() > MAX_FILE {
        return Err(format!(
            "`{}` is {} bytes; at most {MAX_FILE} are read",
            file.name(),
            file.size()
        ));
    }
    let buffer = JsFuture::from(file.array_buffer())
        .await
        .map_err(|e| format!("cannot read `{}`: {}", file.name(), text_of(&e)))?;
    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}

/// Shows the browser's file chooser for `ask`; the file chosen arrives
/// in the inbox. Nothing arrives when the person cancels.
fn choose(ask: Ask, inbox: &Inbox, ctx: &egui::Context) -> Result<(), String> {
    let document = document().ok_or("no document")?;
    let input: web_sys::HtmlInputElement = document
        .create_element("input")
        .map_err(|e| text_of(&e))?
        .dyn_into()
        .map_err(|_| "no file input".to_string())?;
    input.set_type("file");
    input.set_accept(match ask {
        Ask::Open => ".3dcraft,.3dcraftjig",
        Ask::Import => ".step,.stp,.STEP,.STP,.FCStd,.fcstd,.brep,.brp",
    });
    let (inbox, ctx) = (Rc::clone(inbox), ctx.clone());
    let chosen = input.clone();
    let on_change = Closure::once_into_js(move || {
        let Some(file) = chosen.files().and_then(|files| files.get(0)) else {
            return;
        };
        wasm_bindgen_futures::spawn_local(async move {
            let name = file.name();
            let bytes = read_file(file).await;
            inbox.borrow_mut().push(Arrived {
                name,
                bytes,
                wanted: Some(ask),
                handle: None,
            });
            ctx.request_repaint();
        });
    });
    input.set_onchange(Some(on_change.unchecked_ref()));
    input.click();
    Ok(())
}

/// Offers `download` to the person as a file to save: a link to it,
/// clicked. The link is in the page while it is clicked (Firefox only
/// follows a link of the page).
fn offer(download: &Download) -> Result<(), String> {
    let document = document().ok_or("no document")?;
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(download.bytes.as_slice()));
    let options = web_sys::BlobPropertyBag::new();
    options.set_type("application/octet-stream");
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options)
        .map_err(|e| text_of(&e))?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(|e| text_of(&e))?;
    let anchor: web_sys::HtmlAnchorElement = document
        .create_element("a")
        .map_err(|e| text_of(&e))?
        .dyn_into()
        .map_err(|_| "no link".to_string())?;
    anchor.set_href(&url);
    anchor.set_download(&download.name);
    let body = document.body().ok_or("the page has no body")?;
    body.append_child(&anchor).map_err(|e| text_of(&e))?;
    anchor.click();
    anchor.remove();
    // The address is let go of once the browser has taken the file.
    let revoke = Closure::once_into_js(move || {
        let _ = web_sys::Url::revoke_object_url(&url);
    });
    if let Some(window) = window() {
        let _ = window
            .set_timeout_with_callback_and_timeout_and_arguments_0(revoke.unchecked_ref(), 60_000);
    }
    Ok(())
}

fn storage() -> Option<web_sys::Storage> {
    window()?.local_storage().ok().flatten()
}

/// The preferences as stored, when there are any.
fn stored_prefs() -> Option<String> {
    storage().and_then(|s| s.get_item(prefs::KEY).ok().flatten())
}

fn load_prefs(stored: Option<&str>, defaults: Prefs) -> Prefs {
    match stored {
        Some(text) => Prefs::from_json(text, defaults),
        None => defaults,
    }
}

/// True when the stored preferences can be written back as they are: the
/// page then can remember that release notes were closed.
fn prefs_writable(stored: Option<&str>) -> bool {
    match (storage(), stored) {
        (Some(storage), Some(text)) => storage.set_item(prefs::KEY, text).is_ok(),
        _ => false,
    }
}

fn store_prefs(text: &str) {
    if let Some(storage) = storage() {
        // Full or blocked storage keeps nothing; that is all.
        let _ = storage.set_item(prefs::KEY, text);
    }
}

/// What came of the font fetches that ended: (font, outcome), for the
/// next frame.
type FontNews = Rc<RefCell<Vec<(String, Result<(), String>)>>>;

struct Page {
    host: WebHost,
    inbox: Inbox,
    mail: Mailbox,
    /// The file each document (by id) was opened from or saved to through
    /// a file dialog: Save writes there.
    handles: BTreeMap<u64, FileSystemFileHandle>,
    /// The documents a write is running for.
    busy: BTreeSet<u64>,
    /// The next write of a busy document, waiting for the running one.
    queued: BTreeMap<u64, (FileSystemFileHandle, Download)>,
    ctx: egui::Context,
    unsaved: Rc<Cell<bool>>,
    /// The first error the graphics device reported, for the next frame.
    gpu_error: Arc<Mutex<Option<String>>>,
    /// The studio look's environments asked for (fetched from
    /// `environments/<name>.hdr` beside the page, not carried in the
    /// module), and what came of it: `None` while it is on its way, the
    /// error when it failed.
    environments: Rc<RefCell<BTreeMap<String, Option<String>>>>,
    /// The built-in fonts asked for (`crate::fonts`), and the failures of
    /// their fetches, for the next frame to say.
    fonts: crate::fonts::FontFetches,
    font_news: FontNews,
    /// The title the page has now.
    title: String,
    /// The title `index.html` gave the page, kept while no document is
    /// open ([`page_title`]).
    page_title: String,
    stored: String,
    /// The newest version whose release notes were seen in this browser,
    /// as the preferences keep it (`prefs::notes_at_start`).
    seen_version: Option<String>,
    /// The assertions and outputs, measured in the checking worker
    /// (`checks.rs`); `None` on a page that names no worker, which
    /// measures them itself a piece per frame.
    checks: Option<Checks<WebWorker>>,
    /// What the worker answered, for the next frame.
    replies: Replies,
    /// The document shown, regenerated and meshed in a worker of its own
    /// (`scenes.rs`); `None` on a page that names no worker, or with
    /// `?pageregen`, which regenerates a piece per frame.
    scenes: Option<Scenes<WebWorker>>,
    /// What that worker answered, for the next frame.
    scene_replies: Replies,
    /// The page asked for the embedded stand: its title and its storage
    /// are left alone.
    embedded: bool,
    /// What the embedded stand's listeners on the canvas read of the
    /// window, set each frame.
    keys: Keys,
    /// What the page that embeds 3DCraft asked through the door, and
    /// what goes back (`crate::embed`).
    embed: Embed,
}

impl Page {
    fn new(
        creation: &eframe::CreationContext<'_>,
        flags: &Flags,
        inbox: Inbox,
        unsaved: Rc<Cell<bool>>,
        stand: Stand,
    ) -> Self {
        let Stand {
            embedded,
            warnings,
            keys,
        } = stand;
        // A message through the door asks for a frame.
        door::set_context(creation.egui_ctx.clone());
        let choice = if flags.cpu {
            RendererChoice::Cpu
        } else {
            RendererChoice::Auto
        };
        let state = creation.wgpu_render_state.as_ref();
        let gpu_error = Arc::new(Mutex::new(None));
        if let Some(state) = state {
            // WebGPU reports a device's errors later, here; wgpu's own
            // handler would panic, which ends the page. The viewport then
            // goes over to the CPU renderer. This is the first moment the
            // page has the device: eframe makes it in `WebRunner::start`
            // (egui-wgpu offers no hook between making the device and
            // handing it over, and making it ourselves for
            // `WgpuSetup::Existing` would have to repeat its WebGPU and
            // WebGL2 adapter choice). In that window only eframe's own
            // painter is set up; none of our pipelines exist yet.
            let seen = Arc::clone(&gpu_error);
            state
                .device
                .on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
                    log::error!("graphics device: {error}");
                    let mut slot = seen.lock().unwrap_or_else(PoisonError::into_inner);
                    if slot.is_none() {
                        *slot = Some(error.to_string());
                    }
                }));
        }
        // On WebGL2 too every picture is the GPU's: the passes that read
        // the depth buffer (the lines' distances, see-through bodies, the
        // section's layers) sample it with the nearest filter, which
        // OpenGL ES 3.0 needs of a depth texture (`craft3d_gpu::peel`).
        let mut shell = Shell::threaded_with_gpu(choice, state);
        // The studio look drawn light while the camera moves: the full look
        // at the page's frame rate stutters on a laptop's GPU
        // (`Shell::interacting_light`); it settles once the camera rests.
        // `?fullmove` draws it in full, `?movescale=N` N times smaller per
        // side meanwhile (2 unless given).
        shell.set_light_while_moving(!flags.full_move);
        shell.set_moving_divisor(flags.move_scale.unwrap_or(2));
        log::info!("viewport renderer: {}", shell.renderer_report().describe());
        let mut host = WebHost::new(shell);
        // The long work a piece per frame: the page's one thread draws
        // between the pieces (`WebHost::prepare`).
        host.set_stepwise(true);
        // The checks off the page's thread, in a worker of their own.
        let replies: Replies = Rc::new(RefCell::new(Vec::new()));
        let checks = worker::script().map(|script| {
            log::info!("checks: in a Web Worker ({script})");
            let transport = WebWorker::new(
                script,
                "check",
                "the checking worker",
                Rc::clone(&replies),
                creation.egui_ctx.clone(),
            );
            Checks::new(transport, &mut host.session)
        });
        // The regeneration and meshing too, in another, so that a
        // regeneration of minutes never holds the page up.
        let scene_replies: Replies = Rc::new(RefCell::new(Vec::new()));
        let scenes = worker::script()
            .filter(|_| !flags.page_regen)
            .map(|script| {
                log::info!("regeneration: in a Web Worker ({script})");
                let transport = WebWorker::new(
                    script,
                    "scene",
                    "the regenerating worker",
                    Rc::clone(&scene_replies),
                    creation.egui_ctx.clone(),
                );
                Scenes::new(transport, &mut host.session)
            });
        if let Some(url) = &flags.open {
            host.fetching(&flags::file_name(url));
        }
        // The embedded stand reads and writes nothing in the page's
        // storage: it starts with the defaults and the page's options.
        let text = if embedded { None } else { stored_prefs() };
        let prefs = load_prefs(text.as_deref(), Prefs::of(&host.shell.state, None));
        prefs.apply(&mut host.shell.state);
        let stored = prefs.to_json();
        // What's new: after the page was served in a newer version, the
        // start screen offers once what changed, where the browser keeps
        // what was seen. The embedded stand offers nothing and remembers
        // nothing; its Help menu, where the whole window is shown, has
        // this version's notes as everywhere.
        let running = craft3d_engine::running_version();
        let (seen, seen_version) = match running {
            Some(running) if !embedded => {
                let writable = prefs_writable(text.as_deref());
                prefs::notes_at_start(text.as_deref(), writable, running)
            }
            _ => (craft3d_engine::changelog::Seen::Never, None),
        };
        if let (Some(notes), Some(running)) = (craft3d_engine::release_notes(), running) {
            host.shell
                .set_whats_new(craft3d_ui_egui::shell::WhatsNew::for_start(
                    notes, running, seen,
                ));
        }
        // In the browser the page decides how much of the window is drawn
        // (`ui.set`), and in the embedded stand its option says it first.
        host.shell.allow_presentations(true);
        if embedded && let Some(presentation) = flags.presentation {
            host.shell.set_presentation(presentation);
        }
        // The canvas shows nothing of its focus (eframe takes its outline
        // away): in a page, a ring shows when 3DCraft has the keys.
        host.shell.set_focus_ring(embedded);
        // The system's file dialogs where the browser has them, unless the
        // address asks for downloads (`?download`).
        host.pickers = !flags.download && pickers::available();
        log::info!(
            "files: {}",
            if host.pickers {
                "the system's file dialogs"
            } else {
                "downloads and the file input"
            }
        );
        Self {
            host,
            inbox,
            mail: Rc::new(RefCell::new(Vec::new())),
            handles: BTreeMap::new(),
            busy: BTreeSet::new(),
            queued: BTreeMap::new(),
            ctx: creation.egui_ctx.clone(),
            unsaved,
            gpu_error,
            environments: Rc::new(RefCell::new(BTreeMap::new())),
            fonts: crate::fonts::FontFetches::default(),
            font_news: Rc::new(RefCell::new(Vec::new())),
            title: String::new(),
            page_title: document().map(|d| d.title()).unwrap_or_default(),
            stored,
            seen_version,
            checks,
            replies,
            scenes,
            scene_replies,
            embedded,
            keys,
            embed: {
                let mut embed = Embed::new(embedded, Box::new(now_ms));
                embed.set_warnings(warnings);
                embed
            },
        }
    }

    /// What the embedded stand's listeners on the canvas read, as the
    /// window was just drawn: whether Escape has something of the window's
    /// to do, and whether the presentation is `view`. egui counts an area
    /// that closed in this frame as shown until the next one, and a field
    /// of a dialog that closed as having the keys until the end of the
    /// frame after; so a frame that took a key or a click while something
    /// was open asks for [`SETTLING_FRAMES`] more: the next Escape finds
    /// the window as it is (the review's second Escape after the export
    /// dialog closed did nothing, as nothing had drawn the window since).
    fn keys_after(&mut self, ctx: &egui::Context) {
        let staying = [egui::Id::new(crate::host::WORKING_ID)];
        let busy = self.host.shell.escape_has_work(ctx, &staying);
        self.keys.busy.set(busy);
        self.keys
            .view
            .set(self.host.shell.state.presentation == craft3d_ui_egui::state::Presentation::View);
        let acted = ctx.input(|i| {
            i.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::Key { .. } | egui::Event::PointerButton { .. }
                )
            })
        });
        let settling = &self.keys.settling;
        if busy && acted {
            settling.set(SETTLING_FRAMES);
        }
        if busy && settling.get() > 0 {
            settling.set(settling.get() - 1);
            ctx.request_repaint();
        } else if !busy {
            settling.set(0);
        }
    }

    /// Open or Import: the open dialog where there is one, else the file
    /// input. Called in the frame of the click or key press that asked
    /// (the browser shows a dialog only then).
    fn ask(&mut self, ask: Ask) {
        if !self.host.pickers {
            self.choose(ask);
            return;
        }
        let (inbox, mail, ctx) = (
            Rc::clone(&self.inbox),
            Rc::clone(&self.mail),
            self.ctx.clone(),
        );
        wasm_bindgen_futures::spawn_local(async move {
            match pickers::open(&open_types(ask)).await {
                Picked::Chosen(handle) => {
                    let (name, bytes) = match pickers::file(&handle).await {
                        Ok(file) => (file.name(), read_file(file).await),
                        Err(why) => (handle.name(), Err(why)),
                    };
                    inbox.borrow_mut().push(Arrived {
                        name,
                        bytes,
                        wanted: Some(ask),
                        handle: (ask == Ask::Open).then_some(handle),
                    });
                }
                Picked::Cancelled => {}
                Picked::Unavailable(why) => mail.borrow_mut().push(Mail::NoOpenPicker(ask, why)),
            }
            ctx.request_repaint();
        });
    }

    /// The file input for `ask`.
    fn choose(&mut self, ask: Ask) {
        if let Err(why) = choose(ask, &self.inbox, &self.ctx) {
            self.host.note_error(
                "file",
                format!("the file chooser could not be shown: {why}"),
            );
        }
    }

    /// Save As, or an export already made: the save dialog. Called in the
    /// frame of the click or key press that asked.
    fn pick(&mut self, pick: Pick) {
        let (mail, ctx) = (Rc::clone(&self.mail), self.ctx.clone());
        wasm_bindgen_futures::spawn_local(async move {
            let types = pick.types();
            let suggested = pick.suggested().to_string();
            match pickers::save(&types, &suggested).await {
                Picked::Chosen(handle) => mail.borrow_mut().push(Mail::Picked(pick, handle)),
                Picked::Cancelled => {}
                Picked::Unavailable(why) => mail.borrow_mut().push(Mail::NoSavePicker(pick, why)),
            }
            ctx.request_repaint();
        });
    }

    /// Writes `download` through `handle` in the background; the result
    /// comes back as [`Mail::Written`]. A document's writes go one at a
    /// time, in order: while one runs, the newest next one waits (an
    /// older waiting one is dropped, the newer holds all it held).
    fn write(&mut self, handle: FileSystemFileHandle, download: Download) {
        if let Some(id) = download.document {
            if self.busy.contains(&id) {
                self.queued.insert(id, (handle, download));
                return;
            }
            self.busy.insert(id);
        }
        let (mail, ctx) = (Rc::clone(&self.mail), self.ctx.clone());
        wasm_bindgen_futures::spawn_local(async move {
            let result = pickers::write(&handle, &download.bytes).await;
            mail.borrow_mut().push(Mail::Written(Written {
                handle,
                document: download.document,
                close: download.close,
                bytes: download.bytes.len(),
                result,
            }));
            ctx.request_repaint();
        });
    }

    /// The file that goes with an export (an OBJ's material library), after
    /// the person clicked to save it: its own save dialog, else a download.
    fn save_companion(&mut self, file: Download) {
        let (mail, ctx) = (Rc::clone(&self.mail), self.ctx.clone());
        wasm_bindgen_futures::spawn_local(async move {
            let types = [PickerType {
                description: "OBJ material library",
                mime: "model/mtl",
                extensions: vec!["mtl"],
            }];
            let note = match pickers::save(&types, &file.name).await {
                Picked::Chosen(handle) => {
                    let result = pickers::write(&handle, &file.bytes).await;
                    Mail::Written(Written {
                        handle,
                        document: None,
                        close: false,
                        bytes: file.bytes.len(),
                        result,
                    })
                }
                Picked::Cancelled => Mail::Note {
                    label: "file.export",
                    ok: true,
                    text: format!("`{}` (the OBJ's colours) was not saved", file.name),
                },
                Picked::Unavailable(_) => match offer(&file) {
                    Ok(()) => Mail::Note {
                        label: "file.export",
                        ok: true,
                        text: format!(
                            "downloaded `{}`: put it next to the OBJ (see the browser's downloads)",
                            file.name
                        ),
                    },
                    Err(why) => Mail::Note {
                        label: "file.export",
                        ok: false,
                        text: format!("`{}` could not be downloaded: {why}", file.name),
                    },
                },
            };
            mail.borrow_mut().push(note);
            ctx.request_repaint();
        });
    }

    /// Where saved and exported files go: back to the file a document was
    /// opened from or saved to (with the file dialogs), else a download.
    fn deliver(&mut self, downloads: Vec<Download>) {
        let mut offered = Vec::new();
        let mut saved = false;
        for download in downloads {
            if let Some(id) = download.document {
                let own = self
                    .handles
                    .get(&id)
                    .filter(|handle| handle.name() == download.name)
                    .cloned();
                if let Some(handle) = own {
                    self.write(handle, download);
                    continue;
                }
                if self.host.pickers && self.host.writable.contains(&id) {
                    // Its file is under another name than the document
                    // now: ask where, and until then it is not saved.
                    self.host.write_failed(id);
                    self.pick(Pick::Save {
                        document: id,
                        suggested: download.name,
                        close: download.close,
                    });
                    continue;
                }
            }
            saved |= download.document.is_some();
            match offer(&download) {
                Ok(()) => {
                    offered.push(format!("`{}`", download.name));
                    if let (true, Some(id)) = (download.close, download.document) {
                        self.host.close_document(id);
                    }
                }
                Err(why) => {
                    if let Some(id) = download.document {
                        self.host.write_failed(id);
                    }
                    self.host.note_error(
                        "download",
                        format!("`{}` could not be downloaded: {why}", download.name),
                    );
                }
            }
        }
        // Said in the status bar, so the file is looked for where it went.
        let succeeded = self.host.shell.last_command.as_ref().is_none_or(|r| r.ok);
        if !offered.is_empty() && succeeded {
            let label = if saved { "file.save" } else { "file.export" };
            self.host.note(
                label,
                true,
                format!(
                    "downloaded {}: see the browser's downloads",
                    offered.join(", ")
                ),
            );
        }
    }

    /// What the dialogs and writes in the background left.
    fn read_mail(&mut self) {
        let mail: Vec<Mail> = self.mail.borrow_mut().drain(..).collect();
        for item in mail {
            match item {
                Mail::Picked(pick, handle) => self.picked(pick, handle),
                Mail::NoSavePicker(pick, why) => {
                    log::warn!("the save dialog was not shown: {why}");
                    self.refused(&why);
                    let outcome = self.host.without_picker(pick, &why);
                    self.deliver(outcome.downloads);
                }
                Mail::NoOpenPicker(ask, why) => {
                    log::warn!("the open dialog was not shown: {why}");
                    self.refused(&why);
                    self.host.note(
                        "file",
                        false,
                        format!(
                            "the browser did not show its file dialog ({why}): its file chooser \
                             is asked instead; when none appears, try again from the File menu"
                        ),
                    );
                    self.choose(ask);
                }
                Mail::Written(written) => self.written(written),
                Mail::Note { label, ok, text } => self.host.note(label, ok, text),
            }
        }
    }

    /// A file dialog was refused for a reason other than a click that came
    /// too long ago (a cross-origin iframe, a browser that has the API but
    /// not for this page): downloads and the file input from now on.
    fn refused(&mut self, why: &str) {
        if !refusal_is_about_activation(why) {
            self.host.pickers = false;
        }
    }

    /// The save dialog chose `handle` for `pick`: the document is saved, or
    /// the export written, there. When nothing is (the status bar says
    /// why), the empty file the dialog made goes again; a file with
    /// anything in it is kept.
    fn picked(&mut self, pick: Pick, handle: FileSystemFileHandle) {
        let name = handle.name();
        let written = if pick.accepts(&name) {
            match pick {
                Pick::Save {
                    document, close, ..
                } => {
                    let downloads = self.host.save_document(document, &name, close);
                    let any = !downloads.is_empty();
                    for download in downloads {
                        self.write(handle.clone(), download);
                    }
                    any
                }
                Pick::Export { file } => {
                    self.write(handle.clone(), Download { name, ..file });
                    self.host.export_picked();
                    true
                }
            }
        } else {
            let wanted = pick
                .types()
                .iter()
                .flat_map(|t| t.extensions.iter().map(|e| format!(".{e}")))
                .collect::<Vec<_>>()
                .join(" or ");
            let label = match pick {
                Pick::Save { .. } => "file.save",
                Pick::Export { .. } => "file.export",
            };
            self.host.note(
                label,
                false,
                format!("`{name}` is not a {wanted} file: nothing was written; choose a name that ends in {wanted}"),
            );
            false
        };
        if !written {
            wasm_bindgen_futures::spawn_local(async move {
                pickers::remove_if_empty(&handle).await;
            });
        }
    }

    /// A write through a file dialog's handle is done.
    fn written(&mut self, done: Written) {
        let Written {
            handle,
            document,
            close,
            bytes,
            result,
        } = done;
        let name = handle.name();
        let label = if document.is_some() {
            "file.save"
        } else {
            "file.export"
        };
        if let Some(id) = document {
            self.busy.remove(&id);
        }
        let later = document.and_then(|id| self.queued.remove(&id));
        match result {
            Ok(()) => {
                // A later write of the same document keeps its own file
                // once it is done.
                if let (Some(id), None) = (document, &later) {
                    self.handles.insert(id, handle);
                    self.host.writable.insert(id);
                }
                let verb = if document.is_some() { "saved" } else { "wrote" };
                self.host
                    .note(label, true, format!("{verb} `{name}` ({bytes} bytes)"));
                if let (Some(id), true, None) = (document, close, &later) {
                    self.host.close_document(id);
                }
            }
            Err(why) => {
                let mut text = format!("`{name}` was not written: {why}");
                if let Some(id) = document {
                    // Still open and unsaved; the next Save asks where.
                    self.handles.remove(&id);
                    self.host.write_failed(id);
                    text.push_str("; the document is not saved");
                }
                self.host.note(label, false, text);
                // A file the save dialog made and nothing reached goes again.
                wasm_bindgen_futures::spawn_local(async move {
                    pickers::remove_if_empty(&handle).await;
                });
            }
        }
        if let Some((handle, download)) = later {
            self.write(handle, download);
        }
    }

    /// Forgets the files of documents that are closed.
    fn forget_closed(&mut self) {
        if self.handles.is_empty() && self.host.writable.is_empty() {
            return;
        }
        let open: Vec<u64> = self
            .host
            .session
            .document_statuses()
            .iter()
            .map(|s| s.id)
            .collect();
        self.handles.retain(|id, _| open.contains(id));
        self.host.writable.retain(|id| open.contains(id));
    }
}

impl Page {
    /// Fetches the studio look's environment when the look asks for one
    /// that is not here yet: once per environment; a failure is said once
    /// and the viewport stays in the plain look.
    fn fetch_environment(&mut self, ctx: &egui::Context) {
        // A failure is said once (and then kept as an empty text).
        let failed: Vec<(String, String)> = self
            .environments
            .borrow()
            .iter()
            .filter_map(|(name, why)| Some((name.clone(), why.clone().filter(|w| !w.is_empty())?)))
            .collect();
        for (name, why) in failed {
            self.host.note(
                "look",
                false,
                format!("the studio look's environment {name} is not available: {why}"),
            );
            self.environments
                .borrow_mut()
                .insert(name, Some(String::new()));
        }
        let look = &self.host.shell.state.look;
        if !look.is_studio() || craft3d_engine::look::environment_ready(&look.environment) {
            return;
        }
        let name = look.environment.clone();
        if self.environments.borrow().contains_key(&name) {
            return;
        }
        self.environments.borrow_mut().insert(name.clone(), None);
        let (environments, ctx) = (Rc::clone(&self.environments), ctx.clone());
        wasm_bindgen_futures::spawn_local(async move {
            let url = format!("environments/{name}.hdr");
            let outcome = match fetch(&url).await {
                Ok(bytes) => craft3d_engine::look::register_environment(&name, &bytes),
                Err(why) => Err(why),
            };
            if let Err(why) = outcome {
                environments.borrow_mut().insert(name, Some(why));
            }
            ctx.request_repaint();
        });
    }
}

impl Page {
    /// Fetches the built-in fonts a text asked for (once each), and says
    /// what came of the fetches that ended.
    fn fetch_fonts(&mut self, ctx: &egui::Context) {
        let news: Vec<(String, Result<(), String>)> =
            self.font_news.borrow_mut().drain(..).collect();
        for (name, outcome) in news {
            // The interface's font is no text's: when it does not come,
            // egui's own font stays, unsaid and not fetched again.
            if crate::fonts::is_interface_font(&name) {
                if let Err(why) = outcome {
                    log::warn!("the interface font {name} could not be fetched: {why}");
                }
                continue;
            }
            if outcome.is_err() {
                self.fonts.failed(&name);
            }
            self.host.font_fetched(&name, outcome);
        }
        for (name, url) in self.fonts.due() {
            let (news, ctx) = (Rc::clone(&self.font_news), ctx.clone());
            wasm_bindgen_futures::spawn_local(async move {
                let outcome = match fetch(&url).await {
                    Ok(bytes) => craft3d_engine::fonts::register(name, &bytes),
                    Err(why) => Err(why),
                };
                news.borrow_mut().push((name.to_string(), outcome));
                ctx.request_repaint();
            });
        }
    }
}

impl eframe::App for Page {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.fetch_environment(ctx);
        self.fetch_fonts(ctx);
        // Files dropped on the page are read in the background; without
        // the whole window (Embed's `view` and `form`) the page decides
        // what is opened, and a file dropped on the canvas is let be.
        let dropped = if self.host.shell.state.presentation.is_full() {
            ctx.input(|i| i.raw.dropped_files.clone())
        } else {
            Vec::new()
        };
        for file in dropped {
            let (inbox, ctx) = (Rc::clone(&self.inbox), ctx.clone());
            let name = file.path().to_string_lossy().into_owned();
            // Kind and size first: a file is read whole into memory, and
            // running out of it ends the page.
            let refused = if received_kind(&name).is_none() {
                Some(format!(
                    "`{name}` is not a 3DCraft document (.3dcraft), a Jig (.3dcraftjig), a STEP file (.step, .stp) or a FreeCAD document (.FCStd)"
                ))
            } else {
                match file.web_file().map(|f| f.size()) {
                    Some(size) if size <= MAX_FILE => None,
                    Some(size) => Some(format!(
                        "`{name}` is {size} bytes; at most {MAX_FILE} are read"
                    )),
                    None => Some(format!("`{name}` cannot be read here")),
                }
            };
            if let Some(why) = refused {
                self.inbox.borrow_mut().push(Arrived {
                    name,
                    bytes: Err(why),
                    wanted: None,
                    handle: None,
                });
                continue;
            }
            wasm_bindgen_futures::spawn_local(async move {
                let bytes = file.bytes_async().await;
                inbox.borrow_mut().push(Arrived {
                    name,
                    bytes,
                    wanted: None,
                    handle: None,
                });
                ctx.request_repaint();
            });
        }
        // A file opened or dropped while an edit made in the worker waits
        // for its answer waits too (the frame of that answer takes it).
        let arrived: Vec<Arrived> = if self.host.session.edits_waiting() {
            Vec::new()
        } else {
            self.inbox.borrow_mut().drain(..).collect()
        };
        for file in arrived {
            match file.bytes {
                Ok(bytes) => {
                    let opened = self.host.receive(&file.name, bytes, file.wanted);
                    // A document chosen in the open dialog is saved back to
                    // its file.
                    if let (Ok(_), Some(handle)) = (opened, file.handle)
                        && received_kind(&file.name) == Some(crate::host::Received::Document)
                        && let Some(status) = self.host.session.document_status()
                    {
                        self.handles.insert(status.id, handle);
                        self.host.writable.insert(status.id);
                    }
                }
                Err(why) => {
                    if self.host.fetching_name() == Some(file.name.as_str()) {
                        self.host.fetch_failed();
                    }
                    self.host.note("file.open", false, why);
                }
            }
        }
        self.read_mail();
        // What the page that embeds 3DCraft asked through the door, in
        // order (answered now, or kept behind an edit made in the worker),
        // after `ready`, which comes first.
        if door::connected() {
            self.embed.greet(&self.host);
        }
        for message in door::take() {
            self.embed.receive(&mut self.host, message);
        }
        let failed = self
            .gpu_error
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(why) = failed {
            self.host
                .shell
                .leave_gpu(format!("the graphics device reported an error: {why}"));
        }
        // What the File menu, its shortcuts and the tabs asked for, in the
        // frame of the click or key press: a browser shows a file dialog
        // (or lets a download through) only so shortly after one.
        let outcome = self.host.pump();
        if let Some(ask) = outcome.ask {
            self.ask(ask);
        }
        if let Some(pick) = outcome.pick {
            self.pick(pick);
        }
        self.deliver(outcome.downloads);
        self.forget_closed();
        // The scenes to and from their worker, before the page decides
        // what to draw: a part that came is shown in this frame.
        let mut answered = Vec::new();
        if let Some(scenes) = &mut self.scenes {
            let replies: Vec<Reply> = self.scene_replies.borrow_mut().drain(..).collect();
            let quality = self.host.shell.scene_quality();
            let done = scenes.frame_at(&mut self.host.session, quality, replies, now_ms());
            if let Some(why) = done.home {
                log::warn!("regeneration: on the page, the worker cannot be had: {why}");
            }
            if let Some(token) = done.cancelled {
                log::info!(
                    "regeneration: job {token} superseded; the worker goes on with what it built"
                );
            }
            if let Some(token) = done.timed_out {
                log::warn!("regeneration: job {token} took too long in the worker and was stopped");
            }
            if !done.parts.is_empty() || !done.commands.is_empty() {
                ctx.request_repaint();
            }
            if scenes.busy().is_some() || self.host.session.commands_waiting() {
                ctx.request_repaint_after(std::time::Duration::from_secs(30));
            }
            answered = done.commands;
        }
        // Commands run where the bodies are, answered: reported, an
        // export's files offered.
        if !answered.is_empty() {
            let outcome = self.host.answered(answered);
            if let Some(pick) = outcome.pick {
                self.pick(pick);
            }
            self.deliver(outcome.downloads);
        }
        // A piece of the long work, for as long as a frame allows: the
        // rest in the next frames, drawn in between.
        let started = now_ms();
        let busy = self
            .host
            .prepare(move || now_ms() - started > FRAME_WORK_MS);
        if busy {
            ctx.request_repaint();
        }
        // The checks to and from their worker; the worker's answer asks
        // for the frame that takes it.
        if let Some(checks) = &mut self.checks {
            let replies: Vec<Reply> = self.replies.borrow_mut().drain(..).collect();
            let done = checks.frame_at(&mut self.host.session, replies, now_ms());
            if let Some(why) = done.home {
                log::warn!("checks: measured on the page, the worker cannot be had: {why}");
            }
            if !done.answered.is_empty() {
                ctx.request_repaint();
            }
            if let Some(token) = done.timed_out {
                log::warn!("checks: job {token} took too long in the worker and was stopped");
            }
            // While the worker measures, a frame now and then to see
            // whether it has taken too long (`checks::DEADLINE_MS`).
            if checks.busy().is_some() {
                ctx.request_repaint_after(std::time::Duration::from_secs(30));
            }
        }
        // The page's door: the replies of commands run in the worker, what
        // waited, `regenerated`.
        self.embed.frame(&mut self.host);
        door::post(self.embed.take_out());
        // Requests left for the next frame (`embed::FRAME_MS`).
        if self.embed.goes_on_next_frame(&self.host) {
            ctx.request_repaint();
        }
        self.unsaved.set(self.host.has_unsaved());
        // The page's title and its storage are its own in the embedded
        // stand.
        if self.embedded {
            return;
        }
        let title = page_title(
            self.host.session.document_status().as_ref(),
            &self.page_title,
        );
        if title != self.title {
            if let Some(document) = document() {
                document.set_title(&title);
            }
            self.title = title;
        }
        // The release notes the person closed are seen from now on.
        if self.host.shell.take_whats_new_closed()
            && let Some(running) = craft3d_engine::running_version()
        {
            self.seen_version = Some(prefs::seen_after_closing(
                self.seen_version.as_deref(),
                running,
            ));
        }
        let prefs = Prefs::of(
            &self.host.shell.state,
            Some(self.host.shell.state.export.format.extension()),
        )
        .with_seen(self.seen_version.clone());
        let mut text = prefs.to_json();
        if text != self.stored {
            // Another tab may have closed the notes since this one read
            // them: the newer version of the two is the one written.
            let newest =
                prefs::newest_seen(self.seen_version.as_deref(), stored_prefs().as_deref());
            if newest != self.seen_version {
                self.seen_version = newest;
                text = prefs.with_seen(self.seen_version.clone()).to_json();
            }
        }
        if text != self.stored {
            store_prefs(&text);
            self.stored = text;
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // While the scene to show is not there yet, the window is drawn as
        // it was, with the work over it: drawing it from the session would
        // build the scene here, all at once.
        let away = self.host.away();
        let WebHost { session, shell, .. } = &mut self.host;
        if away {
            shell.ui_away(ui);
        } else {
            shell.ui(ui, session);
        }
        // The answers of the dialogs over the page, in the frame of their
        // click: a system's save dialog is shown only so shortly after.
        let outcome = self.host.dialog_ui(ui.ctx());
        if let Some(pick) = outcome.pick {
            self.pick(pick);
        }
        self.deliver(outcome.downloads);
        // An edit made in the worker that takes a while: what it is, and
        // Cancel.
        let cancelled = self.host.waiting_ui(ui.ctx());
        self.deliver(cancelled.downloads);
        if self.host.take_worker_restart()
            && let Some(scenes) = &mut self.scenes
        {
            let answered = scenes.restart(&mut self.host.session);
            let outcome = self.host.answered(answered);
            self.deliver(outcome.downloads);
        }
        if let Some(file) = self.host.companion_ui(ui.ctx()) {
            self.save_companion(file);
        }
        // A command of the page's that a Cancel or a restart answered:
        // its reply goes in the next frame.
        if self.host.page_answers_waiting() {
            ui.ctx().request_repaint();
        }
        if self.embedded {
            self.keys_after(ui.ctx());
        }
    }
}
