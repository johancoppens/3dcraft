//! `craft3d-web`: 3DCraft in the browser, the same engine and egui shell
//! as the desktop app compiled to WebAssembly (AGENTS.md: no webview, no
//! JavaScript framework; wasm-bindgen's generated glue and a small
//! `index.html` load it).
//!
//! It is a viewer, as the desktop app is: open a `.3dcraft` document
//! (File, Open…, or drop it on the page), look at it with the viewport,
//! the view cube, the feature tree and the selection; save it, export it
//! (STEP, STL, 3MF, OBJ with its material library, DXF, SVG) and import
//! STEP. Where the browser has the File System Access API (Chrome, Edge,
//! Chromebooks) Open, Save As, Import and Export show the system's file
//! dialogs and Save writes back to the document's file; elsewhere (or
//! with `?download`) saved and exported files are browser downloads and
//! files are chosen with the page's file input. There is no control
//! channel and no MCP in the browser.
//!
//! Embed (experimental, 2026-11-14): a page that loads the module itself
//! and asks for the embedded stand (`data-craft3d-embed` on the canvas)
//! keeps its title, focus, keys, storage and address, and drives 3DCraft
//! through the control channel's envelope on a `MessagePort`
//! (`craft3d_connect`): commands, the window's state (`full`, `view` or
//! `form`), events, files in and out as bytes (packaging/web/HOSTING.md ›
//! *Embedding*; docs/control-protocol.md › *The page as a transport*).
//!
//! - [`flags`]: `?webgl`, `?cpu`, `?download`, `?open=` in the page's
//!   address (or, in the embedded stand, the canvas's attribute, with
//!   `presentation=` and `warn`).
//! - [`embed`]: what a page that embeds 3DCraft asks and hears, without a
//!   browser in it: tested natively; `door` (wasm32 only) is the port.
//! - [`prefs`]: preferences in `localStorage`, which may be missing.
//! - [`host`]: the session (files in memory) and the shell, and the File
//!   menu's actions: everything but the browser, tested natively.
//! - `page` (wasm32 only): eframe's web runner, the file chooser,
//!   downloads, storage, the browser's warning before unsaved work is
//!   closed, and the graphics device's errors.
//! - `pickers` (wasm32 only): the system's file dialogs and writing to
//!   the file one chose.
//! - [`checks`]: the assertions and outputs measured in a Web Worker;
//!   [`scenes`]: the document regenerated and meshed in another, which
//!   also runs what needs its bodies ([`jobs`]: a command, an edit, an
//!   export from the menu); `worker` (wasm32 only) is such a worker and
//!   its messages.
//!
//! The page runs on its own thread plus two Web Workers, one for the
//! checks and one for the regeneration and meshing, which share no memory
//! (messages only): no `SharedArrayBuffer`, so no cross-origin isolation
//! headers. The viewport is drawn by the GPU
//! renderer on the page's own device (WebGPU, else WebGL2; `?webgl`
//! forces WebGL2), with the CPU reference renderer standing by on the same
//! thread. Crash recovery is off in the browser: a document lives in the
//! page until it is saved, and the browser asks before a page with unsaved
//! changes is closed. Building the site: `cargo xtask package --formats
//! web` (docs/development.md, Web build).

/// The release notes this build carries: the workspace's `CHANGELOG.md`
/// as the build script took it (`build.rs`, `craft3d-changelog`). Nothing
/// is fetched for them.
pub const RELEASE_NOTES: &str = include_str!(concat!(env!("OUT_DIR"), "/release-notes.md"));

#[cfg(test)]
mod beside_tests;
pub mod checks;
#[cfg(target_arch = "wasm32")]
mod door;
pub mod embed;
#[cfg(test)]
mod embed_fuzz;
#[cfg(test)]
mod embed_tests;
pub mod flags;
pub mod fonts;
pub mod host;
pub mod jobs;
#[cfg(target_arch = "wasm32")]
mod page;
#[cfg(target_arch = "wasm32")]
mod pickers;
pub mod prefs;
pub mod scenes;
pub mod seen;
#[cfg(target_arch = "wasm32")]
mod worker;
