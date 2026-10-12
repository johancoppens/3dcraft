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
//! - [`viewport`]: the CPU-rendered image and the lines drawn over it.
//! - [`frames`]: which image is on screen, and the thread that makes the
//!   sharp one.
//! - [`shell`]: the panels and the mouse.
//! - [`control`]: the `ui.*` methods of the control protocol.
//!
//! The viewport shows the image of the CPU reference renderer for now; the
//! wgpu renderer is a later slice.

pub mod camera;
pub mod control;
pub mod frames;
pub mod shell;
pub mod state;
pub mod theme;
pub mod viewport;

pub use control::{UI_METHODS, UiMethod};
pub use shell::{CommandReport, FileAction, RenderStats, Shell, TreeRow, window_title};
pub use state::{CameraState, Panels, ProjectionKind, UiState};
pub use theme::Tokens;
