//! The page: eframe's web runner around the [`WebHost`], and what only
//! the browser can do: choose a file, read a dropped one, offer a
//! download, keep preferences, warn before unsaved work is closed.
//!
//! Every call into the browser is a `Result` or an `Option` that is
//! handled: a page without storage, a blocked download or a failed fetch
//! is reported in the status bar or ignored, never a panic (a panic in
//! WebAssembly ends the page).

use crate::flags::{self, Flags};
use crate::host::{Ask, Download, WebHost};
use crate::prefs::{self, Prefs};
use craft3d_ui_egui::{RendererChoice, Shell, window_title};
use egui_wgpu::wgpu;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Mutex, PoisonError};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

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
}

type Inbox = Rc<RefCell<Vec<Arrived>>>;

/// Starts the page once the module is loaded.
#[wasm_bindgen(start)]
pub fn start() {
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

/// Reads a file of the same site (`?open=`) into the inbox.
fn fetch_into(url: String, inbox: Inbox) {
    wasm_bindgen_futures::spawn_local(async move {
        let name = flags::file_name(&url);
        let bytes = fetch(&url).await;
        inbox.borrow_mut().push(Arrived {
            name,
            bytes,
            wanted: None,
        });
    });
}

async fn fetch(url: &str) -> Result<Vec<u8>, String> {
    let window = window().ok_or("no window")?;
    let response = JsFuture::from(window.fetch_with_str(url))
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
    let buffer = response
        .array_buffer()
        .map_err(|e| format!("cannot read `{url}`: {}", text_of(&e)))?;
    let buffer = JsFuture::from(buffer)
        .await
        .map_err(|e| format!("cannot read `{url}`: {}", text_of(&e)))?;
    let array = js_sys::Uint8Array::new(&buffer);
    if f64::from(array.length()) > MAX_FILE {
        return Err(format!("`{url}` is larger than {MAX_FILE} bytes"));
    }
    Ok(array.to_vec())
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
            });
            ctx.request_repaint();
        });
    });
    input.set_onchange(Some(on_change.unchecked_ref()));
    input.click();
    Ok(())
}

/// Offers `download` to the person as a file to save.
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
    anchor.click();
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
            // goes over to the CPU renderer.
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
        Self {
            host,
            inbox,
            unsaved,
            gpu_error,
            title: String::new(),
            stored,
        }
    }

    fn offer_all(&mut self, downloads: &[Download]) {
        for download in downloads {
            if let Err(why) = offer(download) {
                self.host.shell.last_command = Some(craft3d_ui_egui::CommandReport {
                    label: "download".to_string(),
                    ok: false,
                    text: format!("`{}` could not be offered: {why}", download.name),
                });
            }
        }
    }
}

impl eframe::App for Page {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Files dropped on the page are read in the background.
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        for file in dropped {
            let (inbox, ctx) = (Rc::clone(&self.inbox), ctx.clone());
            wasm_bindgen_futures::spawn_local(async move {
                let name = file.path().to_string_lossy().into_owned();
                let bytes = file.bytes_async().await;
                inbox.borrow_mut().push(Arrived {
                    name,
                    bytes,
                    wanted: None,
                });
                ctx.request_repaint();
            });
        }
        let arrived: Vec<Arrived> = self.inbox.borrow_mut().drain(..).collect();
        for file in arrived {
            match file.bytes {
                Ok(bytes) => {
                    let _ = self.host.receive(&file.name, bytes, file.wanted);
                }
                Err(why) => {
                    self.host.shell.last_command = Some(craft3d_ui_egui::CommandReport {
                        label: "file.open".to_string(),
                        ok: false,
                        text: why,
                    });
                }
            }
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
        let outcome = self.host.pump();
        if let Some(ask) = outcome.ask
            && let Err(why) = choose(ask, &self.inbox, ctx)
        {
            self.host.shell.last_command = Some(craft3d_ui_egui::CommandReport {
                label: "file".to_string(),
                ok: false,
                text: format!("the file chooser could not be shown: {why}"),
            });
        }
        self.offer_all(&outcome.downloads);
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
        let downloads = self.host.dialog_ui(ui.ctx());
        self.offer_all(&downloads);
    }
}
