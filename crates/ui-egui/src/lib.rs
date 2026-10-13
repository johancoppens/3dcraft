//! The egui shell of 3DCraft: a viewport, a read-only feature tree and a
//! status bar over one engine [`Session`](craft3d_engine::Session).
//!
//! Thin by design (AGENTS.md rules 1 and 9): the shell owns only view state
//! ([`state::UiState`]: camera, panels, selection, caption), draws what the
//! engine's scene holds, and never edits a document. It is a library with
//! no window of its own, so `apps/craft3d` hosts it in eframe and tests run
//! it with `egui::Context::run_ui` and synthetic input.
//!
//! - [`state`]: the serde state the control channel reads and drives.
//! - [`theme`]: the design tokens every colour and radius comes from.
//! - [`camera`]: orbit, pan, zoom, fit, and projecting like the renderer.
//! - [`backend`]: which renderer shades the bodies when the window does
//!   not lend the shell its device: the wgpu renderer on a device of its
//!   own, read back, when there is an adapter, else the CPU reference
//!   renderer, which is also the fallback when the GPU fails.
//! - [`direct`]: the viewport drawn by the GPU on the window's own device
//!   into the texture egui shows, multisampled, with the lines over it
//!   drawn and depth-tested on the GPU: what the window uses.
//! - [`viewport`]: the rendered image and the lines drawn over it.
//! - [`frames`]: which image is on screen, and the thread that makes the
//!   sharp one.
//! - [`shell`]: the panels and the mouse.
//! - [`control`]: the `ui.*` methods of the control protocol.
//!
//! `ui.screenshot` always renders on the CPU: it is the oracle the GPU
//! viewport is checked against (docs/architecture.md §7).

pub mod backend;
pub mod camera;
pub mod control;
#[cfg(not(target_arch = "wasm32"))]
pub mod direct;
pub mod frames;
pub mod shell;
pub mod state;
pub mod theme;
pub mod viewport;

pub use backend::{Backend, BodyRenderer, RendererChoice, RendererReport};
pub use control::{UI_METHODS, UiMethod};
pub use shell::{
    CommandReport, FileAction, RecoveryAction, RenderStats, Shell, TreeRow, autosave_text,
    window_title,
};
pub use state::{CameraState, Panels, ProjectionKind, UiState};
pub use theme::Tokens;
