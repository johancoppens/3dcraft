//! `craft3d-web`: 3DCraft in the browser, the same engine and egui shell
//! as the desktop app compiled to WebAssembly (AGENTS.md: no webview, no
//! JavaScript framework; wasm-bindgen's generated glue and a small
//! `index.html` load it).
//!
//! It is a viewer, as the desktop app is: open a `.3dcraft` document
//! (File, Open…, or drop it on the page), look at it with the viewport,
//! the view cube, the feature tree and the selection; save it, export it
//! (STEP, STL, 3MF, OBJ with its material library, DXF, SVG) and import
//! STEP. Files the person saves or exports are browser downloads. There
//! is no control channel and no MCP in the browser.
//!
//! - [`flags`]: `?webgl`, `?cpu`, `?open=` in the page's address.
//! - [`prefs`]: preferences in `localStorage`, which may be missing.
//! - [`host`]: the session (files in memory) and the shell, and the File
//!   menu's actions: everything but the browser, tested natively.
//! - `page` (wasm32 only): eframe's web runner, the file chooser,
//!   downloads, storage, the browser's warning before unsaved work is
//!   closed, and the graphics device's errors.
//!
//! The page runs on one thread: no `SharedArrayBuffer`, so no
//! cross-origin isolation headers. The viewport is drawn by the GPU
//! renderer on the page's own device (WebGPU, else WebGL2; `?webgl`
//! forces WebGL2), with the CPU reference renderer standing by on the same
//! thread. Crash recovery is off in the browser: a document lives in the
//! page until it is saved, and the browser asks before a page with unsaved
//! changes is closed. Building the site: `cargo xtask package --formats
//! web` (docs/development.md, Web build).

pub mod flags;
pub mod host;
#[cfg(target_arch = "wasm32")]
mod page;
pub mod prefs;
