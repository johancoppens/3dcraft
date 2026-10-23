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

use crate::flags::{self, Flags};
use crate::host::{
    Ask, Download, Pick, PickerType, WebHost, open_types, received_kind,
    refusal_is_about_activation,
};
use crate::pickers::{self, Picked};
use crate::prefs::{self, Prefs};
use craft3d_ui_egui::{RendererChoice, Shell, window_title};
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
const CANVAS_ID: &str = "craft3d_canvas";
const LOADING_ID: &str = "craft3d_loading";

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
    let search = window()
        .and_then(|w| w.location().search().ok())
        .unwrap_or_default();
    let flags = flags::parse(&search);
    let Some(canvas) = document()
        .and_then(|d| d.get_element_by_id(CANVAS_ID))
        .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
    else {
        say("3DCraft cannot start: the page has no canvas.");
        return;
    };
    let mut options = eframe::WebOptions::default();
    if flags.webgl {
        let mut setup = egui_wgpu::WgpuSetupCreateNew::without_display_handle();
        setup.instance_descriptor.backends = wgpu::Backends::GL;
        options.wgpu_options.wgpu_setup = egui_wgpu::WgpuSetup::CreateNew(setup);
    }
    let inbox: Inbox = Rc::new(RefCell::new(Vec::new()));
    let unsaved = Rc::new(Cell::new(false));
    warn_before_leaving(Rc::clone(&unsaved));
    keep_file_shortcuts();
    let runner = eframe::WebRunner::new();
    let (for_app, for_unsaved) = (Rc::clone(&inbox), Rc::clone(&unsaved));
    let open = flags.open.clone();
    let started = runner
        .start(
            canvas,
            options,
            Box::new(move |creation| {
                let app = Page::new(creation, &flags, for_app, for_unsaved);
                Ok(Box::new(app))
            }),
        )
        .await;
    match started {
        Ok(()) => {
            hide_loading();
            if let Some(url) = open {
                fetch_into(url, inbox);
            }
        }
        Err(error) => {
            say(&format!(
                "3DCraft needs WebGPU or WebGL2, and this browser offers neither here ({}). \
                 Try an up-to-date Chrome, Edge, Firefox or Safari, with hardware acceleration on.",
                text_of(&error)
            ));
        }
    }
    // The runner lives as long as the page.
    std::mem::forget(runner);
}

/// The browser's question before the page is closed or reloaded while a
/// document has unsaved changes. Kept for the page's lifetime.
fn warn_before_leaving(unsaved: Rc<Cell<bool>>) {
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
    window.set_onbeforeunload(Some(handler.as_ref().unchecked_ref()));
    handler.forget();
}

/// Keeps the File shortcuts eframe leaves to the browser for the page:
/// Ctrl+E (Export; Chrome and Firefox move the focus to the address bar's
/// search) and Ctrl+I (Import; Firefox's page information). eframe itself
/// keeps Ctrl+O and Ctrl+S; Ctrl+N and Ctrl+W the browser never gives up.
/// Kept for the page's lifetime.
fn keep_file_shortcuts() {
    let Some(window) = window() else {
        return;
    };
    let handler =
        Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(move |event: web_sys::KeyboardEvent| {
            let command = event.ctrl_key() || event.meta_key();
            let key = event.key().to_ascii_lowercase();
            // Not with Shift: Ctrl+Shift+I and Ctrl+Shift+E stay the
            // browser's (its developer tools).
            if command && !event.alt_key() && !event.shift_key() && (key == "e" || key == "i") {
                event.prevent_default();
            }
        });
    let _ = window.add_event_listener_with_callback("keydown", handler.as_ref().unchecked_ref());
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
        Ask::Open => ".3dcraft",
        Ask::Import => ".step,.stp,.STEP,.STP",
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

fn load_prefs(defaults: Prefs) -> Prefs {
    match storage().and_then(|s| s.get_item(prefs::KEY).ok().flatten()) {
        Some(text) => Prefs::from_json(&text, defaults),
        None => defaults,
    }
}

fn store_prefs(text: &str) {
    if let Some(storage) = storage() {
        // Full or blocked storage keeps nothing; that is all.
        let _ = storage.set_item(prefs::KEY, text);
    }
}

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
    title: String,
    stored: String,
}

impl Page {
    fn new(
        creation: &eframe::CreationContext<'_>,
        flags: &Flags,
        inbox: Inbox,
        unsaved: Rc<Cell<bool>>,
    ) -> Self {
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
        let mut shell = Shell::threaded_with_gpu(choice, state);
        if state.is_some_and(|s| s.adapter.get_info().backend == wgpu::Backend::Gl) {
            // WebGL2 reads 0 from a depth texture its passes for
            // see-through bodies and the section load without a sampler
            // (OpenGL ES 3.0 takes it for incomplete; the lines' distance
            // pass samples it, `craft3d_gpu::onscreen`): those pictures are
            // the CPU renderer's there.
            shell.without_gpu_see_through();
            shell.without_gpu_section();
        }
        log::info!("viewport renderer: {}", shell.renderer_report().describe());
        let mut host = WebHost::new(shell);
        let prefs = load_prefs(Prefs::of(&host.shell.state, None));
        prefs.apply(&mut host.shell.state);
        host.export_extension.clone_from(&prefs.export_extension);
        let stored = prefs.to_json();
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
            title: String::new(),
            stored,
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

impl eframe::App for Page {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Files dropped on the page are read in the background.
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        for file in dropped {
            let (inbox, ctx) = (Rc::clone(&self.inbox), ctx.clone());
            let name = file.path().to_string_lossy().into_owned();
            // Kind and size first: a file is read whole into memory, and
            // running out of it ends the page.
            let refused = if received_kind(&name).is_none() {
                Some(format!(
                    "`{name}` is not a 3DCraft document (.3dcraft) or a STEP file (.step, .stp)"
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
        let arrived: Vec<Arrived> = self.inbox.borrow_mut().drain(..).collect();
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
                Err(why) => self.host.note("file.open", false, why),
            }
        }
        self.read_mail();
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
        let title = window_title(self.host.session.document_status().as_ref());
        if title != self.title {
            if let Some(document) = document() {
                document.set_title(&title);
            }
            self.title = title;
        }
        self.unsaved.set(self.host.has_unsaved());
        let prefs = Prefs::of(
            &self.host.shell.state,
            self.host.export_extension.as_deref(),
        );
        let text = prefs.to_json();
        if text != self.stored {
            store_prefs(&text);
            self.stored = text;
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let WebHost { session, shell, .. } = &mut self.host;
        shell.ui(ui, session);
        // The answers of the dialogs over the page, in the frame of their
        // click: a system's save dialog is shown only so shortly after.
        let outcome = self.host.dialog_ui(ui.ctx());
        if let Some(pick) = outcome.pick {
            self.pick(pick);
        }
        self.deliver(outcome.downloads);
        if let Some(file) = self.host.companion_ui(ui.ctx()) {
            self.save_companion(file);
        }
    }
}
