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
//! - [`fonts`]: the interface's fonts, Inter and JetBrains Mono, swapped
//!   in when they are here (egui's own until then).
//! - [`files`]: what the hosts' file actions share (proposed names, what
//!   `file.export` is given for the selection), on disk or in memory.
//! - [`frames`]: which image is on screen, and the thread that makes the
//!   sharp one.
//! - [`view_cube`]: the view cube's geometry.
//! - [`kit`]: the component kit, every control painted from the tokens
//!   on a panel or a plate, with its line icons.
//! - [`shell`]: the panels, the tabs of the open documents and the mouse.
//! - [`control`]: the `ui.*` methods of the control protocol.
//! - [`encoder`]: which video encoder a recording asks ffmpeg for.
//! - [`record`]: the window's own frames painted offscreen and read back:
//!   `ui.record`, `ui.advance`, and `ui.screenshot {window: true}` of a
//!   window that is not on screen.
//!
//! `ui.screenshot` always renders on the CPU: it is the oracle the GPU
//! viewport is checked against (docs/design/rendering.md).

pub mod answer;
pub mod backend;
pub mod camera;
pub mod control;
pub mod direct;
#[cfg(not(target_arch = "wasm32"))]
pub mod encoder;
pub mod files;
pub mod fonts;
pub mod frames;
pub mod kit;
#[cfg(not(target_arch = "wasm32"))]
pub mod record;
pub mod shell;
pub mod state;
pub mod theme;
pub mod view_cube;
pub mod viewport;

pub use backend::{Backend, BodyRenderer, RendererChoice, RendererReport};
pub use control::{QuitMode, UI_METHODS, UiMethod, quit_mode};
pub use shell::{
    CloseAnswer, CommandReport, DocAction, EditAction, EditRequest, FLY_SECONDS, FileAction,
    MIN_FLY_CONTEXT, ParamAsk, RecoveryAction, RenderStats, STATUS_YIELD, Shell, ShellCommand,
    StatusFit, StatusSlot, TabPart, TreeRow, ZOOM_TOOLTIP, autosave_text, doc_tabs, fit_status,
    run_edit, window_title,
};
pub use state::{
    AgentClient, AgentPrompt, AgentRoot, AgentsStatus, CameraState, Closing, DocTab, Panels,
    ParametersView, PlayQuality, Playback, ProjectionKind, PromptAnswer, Show, SidePanelView,
    SideTab, TabView, UiState, Work,
};
pub use theme::Tokens;
